//! The main loop: applies results from background requests, handles keys, and draws.
//!
//! Handlers never touch the network. They queue a [`Request`], and [`EventLoop::dispatch`] spawns each one as a task that sends an [`Update`] back.

use std::io::{self, Write};
use std::mem;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use notify_rust::Notification;
use ratatui::Terminal;
use ratatui::backend::Backend;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::api::{self, Chat, Message};
use crate::app::{self, App, ChatOrder};
use crate::{auth, config, message, ui};

/// How long a key press is waited for before the loop goes round again.
const POLL: Duration = Duration::from_millis(100);

/// How often the chat list, and the newest message of every chat, is fetched.
const CHAT_REFRESH: Duration = Duration::from_secs(3);

/// The selected chat's messages reload every this many polls, about three seconds.
const MESSAGE_REFRESH_POLLS: u32 = 30;

/// How long the status bar flashes for a console notification.
const BELL_FLASH: Duration = Duration::from_millis(200);

/// Network work a handler asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    Chats,
    Messages(String),
    /// Only the newest message, to spot new messages in chats that are not on screen.
    Newest(String),
    MarkRead(String),
    Send {
        chat_id: String,
        text: String,
    },
}

/// A result from a background request.
#[derive(Debug)]
enum Update {
    Chats(Vec<Chat>),
    Messages {
        chat_id: String,
        messages: Vec<Message>,
    },
    Newest {
        chat_id: String,
        message: Message,
    },
    Status(String),
}

#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

#[derive(Debug)]
pub(crate) struct EventLoop {
    tx: UnboundedSender<Update>,
    rx: UnboundedReceiver<Update>,
    pending: Vec<Request>,
    order: ChatOrder,
    /// Graph's latest chat list, before `order` is applied.
    latest_chats: Option<Vec<Chat>>,
    /// The chat selected at the last message refresh, so a refresh skips a chat that was only just opened.
    refreshed_chat: Option<String>,
    polls: u32,
}

impl EventLoop {
    pub(crate) fn new(order: ChatOrder) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            pending: Vec::new(),
            order,
            latest_chats: None,
            refreshed_chat: None,
            polls: 0,
        }
    }

    /// Runs until the user quits.
    ///
    /// # Errors
    ///
    /// Fails when the terminal cannot be drawn to or read from.
    pub(crate) fn run<B>(mut self, terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
    where
        B: Backend,
        B::Error: Send + Sync + 'static,
    {
        self.spawn_chat_refresh();
        self.open_selected(app);

        loop {
            while let Ok(update) = self.rx.try_recv() {
                self.apply(app, update);
            }
            self.tick(app);
            terminal.draw(|f| ui::draw(f, app))?;

            if event::poll(POLL)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && self.handle_key(app, key) == Flow::Quit
            {
                return Ok(());
            }
            self.dispatch();
        }
    }

    fn apply(&mut self, app: &mut App, update: Update) {
        match update {
            Update::Chats(chats) => self.on_chats(app, chats),
            Update::Messages { chat_id, messages } => self.on_messages(app, &chat_id, messages),
            Update::Newest { chat_id, message } => self.on_newest(app, &chat_id, &message),
            Update::Status(status) => app.status = status,
        }
    }

    /// Takes a fresh chat list: checks every other chat for a new message, and reloads the open one.
    fn on_chats(&mut self, app: &mut App, chats: Vec<Chat>) {
        let selected = app.selected_chat_id().map(str::to_owned);
        self.pending.extend(
            chats
                .iter()
                .filter(|c| Some(&c.id) != selected.as_ref())
                .map(|c| Request::Newest(c.id.clone())),
        );
        self.order.absorb(&chats);
        app.set_chats(self.order.arrange(&chats));
        self.latest_chats = Some(chats);
        if let Some(id) = selected {
            self.pending.push(Request::Messages(id));
        }
    }

    /// Takes the newest message of a chat in the background, and on a new one notifies and moves the chat to the top.
    fn on_newest(&mut self, app: &mut App, chat_id: &str, newest: &Message) {
        if !self.order.record_background(chat_id, newest) {
            return;
        }
        notify(app, newest.sender_name());
        self.order.move_to_top(chat_id);
        let chats = self.latest_chats.as_deref().unwrap_or(&app.chats);
        let arranged = self.order.arrange(chats);
        app.set_chats(arranged);
        self.pending.push(Request::Chats);
    }

    /// Takes the open chat's messages, if it is still open, and marks the chat read.
    fn on_messages(&mut self, app: &mut App, chat_id: &str, messages: Vec<Message>) {
        if app.selected_chat_id() != Some(chat_id) {
            return;
        }

        let newest_id = |list: &[Message]| app::newest(list).map(|m| m.id.clone());
        let changed = app.loading_messages
            || app.messages.len() != messages.len()
            || newest_id(&app.messages) != newest_id(&messages);
        if changed {
            if let Some(newest) = app::newest(&messages)
                && self.order.record_viewed(chat_id, newest)
            {
                self.order.move_to_top(chat_id);
            }
            app.set_messages(messages);
            app.snap_to_bottom = true;
        }
        self.pending.push(Request::MarkRead(chat_id.to_owned()));
    }

    /// Counts a pass of the loop, and reloads the open chat every [`MESSAGE_REFRESH_POLLS`] of them.
    fn tick(&mut self, app: &App) {
        self.polls = self.polls.saturating_add(1);
        if self.polls < MESSAGE_REFRESH_POLLS {
            return;
        }
        self.polls = 0;

        let selected = app.selected_chat_id().map(str::to_owned);
        if let Some(id) = &selected
            && self.refreshed_chat.as_ref() == Some(id)
        {
            self.pending.push(Request::Messages(id.clone()));
        }
        self.refreshed_chat = selected;
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) -> Flow {
        let before = app.selected_chat_id().map(str::to_owned);
        match key.code {
            KeyCode::PageUp => app.scroll_up(),
            KeyCode::PageDown => app.scroll_down(),
            _ if app.input_mode => self.handle_input_key(app, key),
            code => {
                if handle_browse_key(app, code) == Flow::Quit {
                    return Flow::Quit;
                }
            }
        }
        if app.selected_chat_id() != before.as_deref() {
            self.open_selected(app);
        }
        Flow::Continue
    }

    fn handle_input_key(&mut self, app: &mut App, key: KeyEvent) {
        let newline_modifiers = KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Esc => {
                app.input_mode = false;
                app.input_buffer.clear();
            }
            KeyCode::Enter if key.modifiers.intersects(newline_modifiers) => {
                app.input_buffer.push('\n');
            }
            KeyCode::Enter => self.send(app),
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Char(c) => app.input_buffer.push(c),
            _ => {}
        }
    }

    fn send(&mut self, app: &mut App) {
        if app.input_buffer.is_empty() {
            return;
        }
        let text = mem::take(&mut app.input_buffer);
        app.input_mode = false;
        if let Some(chat_id) = app.selected_chat_id() {
            self.pending.push(Request::Send {
                chat_id: chat_id.to_owned(),
                text,
            });
            app.snap_to_bottom = true;
        }
    }

    /// Clears the message view and loads the selected chat.
    fn open_selected(&mut self, app: &mut App) {
        let Some(id) = app.selected_chat_id().map(str::to_owned) else {
            return;
        };
        app.set_messages(Vec::new());
        app.loading_messages = true;
        app.snap_to_bottom = true;
        self.pending.push(Request::Messages(id));
    }

    fn dispatch(&mut self) {
        for request in mem::take(&mut self.pending) {
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let Ok(token) = auth::fetch_token_silently().await else {
                    return;
                };
                if let Some(update) = fetch_update(&token, request).await {
                    tx.send(update).ok();
                }
            });
        }
    }

    fn spawn_chat_refresh(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CHAT_REFRESH);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Ok(token) = auth::fetch_token_silently().await
                    && let Some(update) = fetch_update(&token, Request::Chats).await
                    && tx.send(update).is_err()
                {
                    return;
                }
            }
        });
    }
}

/// Keys outside the compose box.
fn handle_browse_key(app: &mut App, code: KeyCode) -> Flow {
    match code {
        KeyCode::Char('q') => return Flow::Quit,
        KeyCode::Down | KeyCode::Char('j') => app.next_chat(),
        KeyCode::Up | KeyCode::Char('k') => app.previous_chat(),
        KeyCode::Char('K') => app.scroll_up(),
        KeyCode::Char('J') => app.scroll_down(),
        KeyCode::Char('n') => {
            app.notification_mode = app.notification_mode.next();
            let mut config = config::load_config().unwrap_or_default();
            config.notification_mode = Some(app.notification_mode);
            if let Err(e) = config::save_config(&config) {
                app.status = format!("Could not save the notification mode: {e}");
            }
        }
        KeyCode::Char('i') => {
            app.input_mode = true;
            app.input_buffer.clear();
        }
        _ => {}
    }
    Flow::Continue
}

/// Runs one request. Failures are dropped, except a failed send, which comes back as a status.
async fn fetch_update(token: &str, request: Request) -> Option<Update> {
    match request {
        Request::Chats => api::fetch_chats(token).await.ok().map(Update::Chats),
        Request::Messages(chat_id) => {
            let messages = api::fetch_messages(token, &chat_id).await.ok()?;
            Some(Update::Messages { chat_id, messages })
        }
        Request::Newest(chat_id) => {
            let messages = api::fetch_messages(token, &chat_id).await.ok()?;
            let message = app::newest(&messages)?.clone();
            Some(Update::Newest { chat_id, message })
        }
        Request::MarkRead(chat_id) => {
            api::mark_chat_as_read(token, &chat_id).await;
            None
        }
        Request::Send { chat_id, text } => match api::send_message(token, &chat_id, &text).await {
            Ok(()) => Box::pin(fetch_update(token, Request::Messages(chat_id))).await,
            Err(e) => Some(Update::Status(format!("{e:#}"))),
        },
    }
}

/// Announces a new message the way the notification mode says.
fn notify(app: &mut App, sender: &str) {
    let mode = app.notification_mode;
    if mode.rings_bell() {
        let mut stdout = io::stdout();
        stdout.write_all(b"\x07").and_then(|()| stdout.flush()).ok();
        app.visual_bell_until = Instant::now().checked_add(BELL_FLASH);
    }
    if mode.shows_desktop() {
        let body = format!("New message from {}", message::sanitize(sender));
        tokio::task::spawn_blocking(move || {
            Notification::new()
                .summary("Starling")
                .body(&body)
                .appname("Starling")
                .show()
                .ok();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::app::tests::{chat, message};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn setup(chat_ids: &[&str]) -> (EventLoop, App) {
        let chats: Vec<Chat> = chat_ids.iter().map(|id| chat(id)).collect();
        let event_loop = EventLoop::new(ChatOrder::new(&chats, HashMap::new()));
        let mut app = App::new();
        app.set_chats(chats);
        (event_loop, app)
    }

    fn order_of(app: &App) -> Vec<&str> {
        app.chats.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn a_fresh_chat_list_checks_other_chats_and_reloads_the_open_one() {
        let (mut event_loop, mut app) = setup(&["a", "b"]);
        event_loop.on_chats(&mut app, vec![chat("a"), chat("b"), chat("c")]);
        assert_eq!(
            event_loop.pending,
            [
                Request::Newest("b".to_owned()),
                Request::Newest("c".to_owned()),
                Request::Messages("a".to_owned()),
            ]
        );
        assert_eq!(order_of(&app), ["a", "b", "c"]);
    }

    #[test]
    fn a_new_message_elsewhere_moves_that_chat_to_the_top_and_keeps_the_selection() {
        let (mut event_loop, mut app) = setup(&["a", "b", "c"]);
        let first = message("1", "2026-01-01T00:00:00Z");
        event_loop.on_newest(&mut app, "c", &first);
        assert_eq!(order_of(&app), ["a", "b", "c"]);

        event_loop.on_newest(&mut app, "c", &message("2", "2026-01-02T00:00:00Z"));
        assert_eq!(order_of(&app), ["c", "a", "b"]);
        assert_eq!(app.selected_chat_id(), Some("a"));
        assert_eq!(event_loop.pending, [Request::Chats]);
    }

    #[test]
    fn messages_for_a_chat_no_longer_open_are_dropped() {
        let (mut event_loop, mut app) = setup(&["a", "b"]);
        event_loop.on_messages(&mut app, "b", vec![message("1", "2026-01-01T00:00:00Z")]);
        assert!(app.messages.is_empty());
        assert_eq!(event_loop.pending, []);
    }

    #[test]
    fn an_empty_chat_finishes_loading() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.open_selected(&mut app);
        event_loop.on_messages(&mut app, "a", Vec::new());
        assert!(!app.loading_messages);
        assert_eq!(
            event_loop.pending,
            [
                Request::Messages("a".to_owned()),
                Request::MarkRead("a".to_owned()),
            ]
        );
    }

    #[test]
    fn moving_the_selection_opens_the_next_chat() {
        let (mut event_loop, mut app) = setup(&["a", "b"]);
        assert_eq!(
            event_loop.handle_key(&mut app, key(KeyCode::Char('j'))),
            Flow::Continue
        );
        assert_eq!(app.selected_chat_id(), Some("b"));
        assert!(app.loading_messages);
        assert_eq!(event_loop.pending, [Request::Messages("b".to_owned())]);
    }

    #[test]
    fn typing_and_enter_queue_a_send() {
        let (mut event_loop, mut app) = setup(&["a"]);
        for code in [KeyCode::Char('i'), KeyCode::Char('h'), KeyCode::Char('q')] {
            assert_eq!(event_loop.handle_key(&mut app, key(code)), Flow::Continue);
        }
        event_loop.handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        event_loop.handle_key(&mut app, key(KeyCode::Char('i')));
        event_loop.handle_key(&mut app, key(KeyCode::Backspace));
        event_loop.handle_key(&mut app, key(KeyCode::Enter));

        assert!(!app.input_mode);
        assert_eq!(app.input_buffer, "");
        assert_eq!(
            event_loop.pending,
            [Request::Send {
                chat_id: "a".to_owned(),
                text: "hq\n".to_owned(),
            }]
        );
    }

    #[test]
    fn enter_on_an_empty_box_sends_nothing() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.handle_key(&mut app, key(KeyCode::Char('i')));
        event_loop.handle_key(&mut app, key(KeyCode::Enter));
        assert!(app.input_mode);
        assert_eq!(event_loop.pending, []);
    }

    #[test]
    fn q_quits_only_outside_the_compose_box() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.handle_key(&mut app, key(KeyCode::Char('i')));
        assert_eq!(
            event_loop.handle_key(&mut app, key(KeyCode::Char('q'))),
            Flow::Continue
        );
        event_loop.handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(
            event_loop.handle_key(&mut app, key(KeyCode::Char('q'))),
            Flow::Quit
        );
    }

    #[test]
    fn the_open_chat_reloads_every_refresh_after_the_first() {
        let (mut event_loop, app) = setup(&["a"]);
        for _ in 0..MESSAGE_REFRESH_POLLS {
            event_loop.tick(&app);
        }
        assert_eq!(event_loop.pending, []);
        for _ in 0..MESSAGE_REFRESH_POLLS {
            event_loop.tick(&app);
        }
        assert_eq!(event_loop.pending, [Request::Messages("a".to_owned())]);
    }
}
