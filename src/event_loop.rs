//! The main loop: sign-in, loading, then the chat screen, all on the terminal.
//!
//! Handlers never touch the network. They queue a [`Request`], and [`EventLoop::dispatch`] spawns each one as a task that sends an [`Update`] back.

use std::io::{self, Write};
use std::mem;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use notify_rust::Notification;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::MissedTickBehavior;

use crate::api::{self, Chat, Message, Startup};
use crate::app::{
    self, App, ChatOrder, Direction, NotificationMode, PickerExit, Screen, SignInCode,
};
use crate::{auth, config, message, ui};

/// How often the loop wakes without input, to redraw timed effects and count toward the message refresh.
const TICK: Duration = Duration::from_millis(100);

/// How often the chat list, and the newest message of every chat, is fetched.
const CHAT_REFRESH: Duration = Duration::from_secs(3);

/// The selected chat's messages reload every this many ticks, about three seconds.
const MESSAGE_REFRESH_TICKS: u32 = 30;

/// How long the status bar flashes for a console notification.
const BELL_FLASH: Duration = Duration::from_millis(200);

/// Work a handler asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    /// Signs in with a saved token, or with a device code when there is none.
    SignIn {
        client_id: String,
    },
    SaveClientId(String),
    /// Stores a theme by its [`crate::theme::Theme::key`].
    SaveTheme(String),
    SaveNotificationMode(NotificationMode),
    /// Fetches what the chat screen needs to open.
    Load,
    /// Fetches the chat list once, after a new message moves a chat.
    Chats,
    /// Fetches the chat list every [`CHAT_REFRESH`] for as long as the app runs.
    WatchChats,
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
    SignInCode(SignInCode),
    SignedIn,
    SignInFailed(String),
    Loaded(Startup),
    LoadFailed(String),
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
    /// The client ID being signed in with, kept to refill the field if sign-in fails.
    client_id: Option<String>,
    order: ChatOrder,
    /// Graph's latest chat list, before `order` is applied.
    latest_chats: Option<Vec<Chat>>,
    /// The chat selected at the last message refresh, so a refresh skips a chat that was only just opened.
    refreshed_chat: Option<String>,
    ticks: u32,
}

impl EventLoop {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            pending: Vec::new(),
            client_id: None,
            order: ChatOrder::default(),
            latest_chats: None,
            refreshed_chat: None,
            ticks: 0,
        }
    }

    /// Runs until the user quits, starting with sign-in, or with a prompt for the client ID when there is none.
    ///
    /// # Errors
    ///
    /// Fails when the terminal cannot be drawn to or read from.
    pub(crate) async fn run(
        mut self,
        terminal: &mut DefaultTerminal,
        app: &mut App,
        client_id: Option<String>,
    ) -> Result<()> {
        self.start(app, client_id);
        let mut events = EventStream::new();
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            self.dispatch();
            terminal.draw(|f| ui::draw(f, app))?;

            tokio::select! {
                Some(update) = self.rx.recv() => self.apply(app, update),
                _ = ticks.tick() => self.tick(app),
                event = events.next() => match event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if self.handle_key(app, key) == Flow::Quit {
                            return Ok(());
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                },
            }
        }
    }

    fn start(&mut self, app: &mut App, client_id: Option<String>) {
        match client_id {
            Some(id) => self.sign_in(app, id),
            None => {
                app.screen = Screen::ClientId {
                    input: String::new(),
                    error: None,
                };
            }
        }
    }

    fn sign_in(&mut self, app: &mut App, client_id: String) {
        app.screen = Screen::SignIn { code: None };
        self.pending.push(Request::SignIn {
            client_id: client_id.clone(),
        });
        self.client_id = Some(client_id);
    }

    fn apply(&mut self, app: &mut App, update: Update) {
        match update {
            Update::SignInCode(code) => {
                if let Screen::SignIn { code: shown } = &mut app.screen {
                    *shown = Some(code);
                }
            }
            Update::SignedIn => {
                app.screen = Screen::Loading { error: None };
                self.pending.push(Request::Load);
            }
            Update::SignInFailed(error) => {
                app.screen = Screen::ClientId {
                    input: self.client_id.clone().unwrap_or_default(),
                    error: Some(error),
                };
            }
            Update::Loaded(startup) => self.on_loaded(app, startup),
            Update::LoadFailed(error) => app.screen = Screen::Loading { error: Some(error) },
            Update::Chats(chats) => self.on_chats(app, chats),
            Update::Messages { chat_id, messages } => self.on_messages(app, &chat_id, messages),
            Update::Newest { chat_id, message } => self.on_newest(app, &chat_id, &message),
            Update::Status(status) => app.status = status,
        }
    }

    /// Opens the chat screen on the loaded chats, newest activity first, and starts watching for changes.
    fn on_loaded(&mut self, app: &mut App, startup: Startup) {
        if !matches!(app.screen, Screen::Loading { .. }) {
            return;
        }
        let Startup {
            user_name,
            mut chats,
            last_message_times,
        } = startup;

        app::sort_by_activity(&mut chats, &last_message_times);
        self.order = ChatOrder::new(&chats, last_message_times);
        app.status = format!("Loaded {} chats", chats.len());
        app.set_chats(chats);
        app.current_user_name = user_name;
        app.screen = Screen::Chats;
        self.pending.push(Request::WatchChats);
        self.open_selected(app);
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

    /// Counts a tick, and reloads the open chat every [`MESSAGE_REFRESH_TICKS`] of them.
    fn tick(&mut self, app: &App) {
        self.ticks = self.ticks.saturating_add(1);
        if self.ticks < MESSAGE_REFRESH_TICKS {
            return;
        }
        self.ticks = 0;

        let selected = app.selected_chat_id().map(str::to_owned);
        if let Some(id) = &selected
            && self.refreshed_chat.as_ref() == Some(id)
        {
            self.pending.push(Request::Messages(id.clone()));
        }
        self.refreshed_chat = selected;
    }

    fn handle_key(&mut self, app: &mut App, key: KeyEvent) -> Flow {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Flow::Quit;
        }
        match &mut app.screen {
            Screen::ClientId { input, error } => match key.code {
                KeyCode::Esc => return Flow::Quit,
                KeyCode::Enter => {
                    let id = input.trim().to_owned();
                    if auth::is_client_id(&id) {
                        self.pending.push(Request::SaveClientId(id.clone()));
                        self.sign_in(app, id);
                    } else {
                        *error = Some(
                            "That is not a client ID. It looks like 12345678-1234-1234-1234-123456789abc."
                                .to_owned(),
                        );
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Screen::SignIn { .. } => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    return Flow::Quit;
                }
            }
            Screen::Loading { error } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
                KeyCode::Char('r') if error.is_some() => {
                    *error = None;
                    self.pending.push(Request::Load);
                }
                _ => {}
            },
            Screen::Chats => return self.handle_chats_key(app, key),
        }
        Flow::Continue
    }

    fn handle_chats_key(&mut self, app: &mut App, key: KeyEvent) -> Flow {
        if app.picker.is_some() {
            self.handle_picker_key(app, key.code);
            return Flow::Continue;
        }
        let before = app.selected_chat_id().map(str::to_owned);
        match key.code {
            KeyCode::PageUp => app.scroll_up(),
            KeyCode::PageDown => app.scroll_down(),
            _ if app.input_mode => self.handle_input_key(app, key),
            code => {
                if self.handle_browse_key(app, code) == Flow::Quit {
                    return Flow::Quit;
                }
            }
        }
        if app.selected_chat_id() != before.as_deref() {
            self.open_selected(app);
        }
        Flow::Continue
    }

    /// Keys on the chat screen outside the compose box.
    fn handle_browse_key(&mut self, app: &mut App, code: KeyCode) -> Flow {
        match code {
            KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Down | KeyCode::Char('j') => app.next_chat(),
            KeyCode::Up | KeyCode::Char('k') => app.previous_chat(),
            KeyCode::Char('K') => app.scroll_up(),
            KeyCode::Char('J') => app.scroll_down(),
            KeyCode::Char('n') => {
                app.notification_mode = app.notification_mode.next();
                self.pending
                    .push(Request::SaveNotificationMode(app.notification_mode));
            }
            KeyCode::Char('t') => app.open_theme_picker(),
            KeyCode::Char('i') => {
                app.input_mode = true;
                app.input_buffer.clear();
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Keys while the theme picker is open. Moving previews a theme; Enter keeps and saves it; Esc puts back the one before.
    fn handle_picker_key(&mut self, app: &mut App, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => app.move_theme_picker(Direction::Up),
            KeyCode::Down | KeyCode::Char('j') => app.move_theme_picker(Direction::Down),
            KeyCode::Tab | KeyCode::Char('m') => app.theme.mode = app.theme.mode.toggled(),
            KeyCode::Enter => {
                app.close_theme_picker(PickerExit::Keep);
                self.pending.push(Request::SaveTheme(app.theme.key()));
            }
            KeyCode::Esc | KeyCode::Char('q' | 't') => app.close_theme_picker(PickerExit::Cancel),
            _ => {}
        }
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
                if let Some(update) = perform(&tx, request).await {
                    tx.send(update).ok();
                }
            });
        }
    }
}

/// Runs one request. Chat requests without a usable token, and failed fetches, give nothing: the next refresh tries again.
async fn perform(tx: &UnboundedSender<Update>, request: Request) -> Option<Update> {
    match request {
        Request::SignIn { client_id } => Some(sign_in(tx, &client_id).await),
        Request::SaveClientId(client_id) => save("client ID", |c| c.client_id = Some(client_id)),
        Request::SaveTheme(key) => save("theme", |c| c.theme = Some(key)),
        Request::SaveNotificationMode(mode) => {
            save("notification mode", |c| c.notification_mode = Some(mode))
        }
        Request::Load => Some(match load().await {
            Ok(startup) => Update::Loaded(startup),
            Err(e) => Update::LoadFailed(format!("{e:#}")),
        }),
        Request::Chats => api::fetch_chats(&token().await?)
            .await
            .ok()
            .map(Update::Chats),
        Request::WatchChats => {
            watch_chats(tx).await;
            None
        }
        Request::Messages(chat_id) => {
            let token = token().await?;
            let messages = api::fetch_messages(&token, &chat_id).await.ok()?;
            Some(Update::Messages { chat_id, messages })
        }
        Request::Newest(chat_id) => {
            let token = token().await?;
            let messages = api::fetch_messages(&token, &chat_id).await.ok()?;
            let message = app::newest(&messages)?.clone();
            Some(Update::Newest { chat_id, message })
        }
        Request::MarkRead(chat_id) => {
            api::mark_chat_as_read(&token().await?, &chat_id).await;
            None
        }
        Request::Send { chat_id, text } => {
            let token = token().await?;
            match api::send_message(&token, &chat_id, &text).await {
                Ok(()) => Box::pin(perform(tx, Request::Messages(chat_id))).await,
                Err(e) => Some(Update::Status(format!("{e:#}"))),
            }
        }
    }
}

/// Changes the config file, and reports a failure in the status bar.
fn save(what: &str, change: impl FnOnce(&mut config::Config)) -> Option<Update> {
    config::update(change)
        .err()
        .map(|e| Update::Status(format!("Could not save the {what}: {e:#}")))
}

async fn token() -> Option<String> {
    auth::fetch_token_silently().await.ok()
}

/// Signs in with the saved token if it still works, else with a device code the user enters on another device.
async fn sign_in(tx: &UnboundedSender<Update>, client_id: &str) -> Update {
    if auth::fetch_token_silently().await.is_ok() {
        return Update::SignedIn;
    }
    let result = async {
        let code = auth::start_device_flow(client_id).await?;
        tx.send(Update::SignInCode(SignInCode {
            verification_uri: code.verification_uri.clone(),
            user_code: code.user_code.clone(),
        }))
        .ok();
        auth::poll_for_token(client_id, &code).await
    }
    .await;
    match result {
        Ok(()) => Update::SignedIn,
        Err(e) => Update::SignInFailed(format!("{e:#}")),
    }
}

async fn load() -> Result<Startup> {
    let token = auth::fetch_token_silently().await?;
    api::fetch_startup(&token).await
}

/// Sends a fresh chat list every [`CHAT_REFRESH`], one fetch at a time, until the loop goes away.
async fn watch_chats(tx: &UnboundedSender<Update>) {
    let mut interval = tokio::time::interval(CHAT_REFRESH);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if tx.is_closed() {
            return;
        }
        if let Some(token) = token().await
            && let Ok(chats) = api::fetch_chats(&token).await
            && tx.send(Update::Chats(chats)).is_err()
        {
            return;
        }
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

    const CLIENT_ID: &str = "12345678-1234-1234-1234-123456789abc";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(event_loop: &mut EventLoop, app: &mut App, text: &str) {
        for c in text.chars() {
            event_loop.handle_key(app, key(KeyCode::Char(c)));
        }
    }

    fn setup(chat_ids: &[&str]) -> (EventLoop, App) {
        let chats: Vec<Chat> = chat_ids.iter().map(|id| chat(id)).collect();
        let mut event_loop = EventLoop::new();
        event_loop.order = ChatOrder::new(&chats, HashMap::new());
        let mut app = App::new();
        app.screen = Screen::Chats;
        app.set_chats(chats);
        (event_loop, app)
    }

    fn order_of(app: &App) -> Vec<&str> {
        app.chats.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn without_a_client_id_it_asks_for_one() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        event_loop.start(&mut app, None);
        assert_eq!(
            app.screen,
            Screen::ClientId {
                input: String::new(),
                error: None
            }
        );
        assert_eq!(event_loop.pending, []);
    }

    #[test]
    fn a_bad_client_id_is_refused_on_screen() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        event_loop.start(&mut app, None);
        type_text(&mut event_loop, &mut app, "not-an-id");
        event_loop.handle_key(&mut app, key(KeyCode::Enter));
        assert!(matches!(
            &app.screen,
            Screen::ClientId { input, error: Some(_) } if input == "not-an-id"
        ));
        assert_eq!(event_loop.pending, []);
    }

    #[test]
    fn a_good_client_id_is_saved_and_used_to_sign_in() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        event_loop.start(&mut app, None);
        type_text(&mut event_loop, &mut app, &format!(" {CLIENT_ID} "));
        event_loop.handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::SignIn { code: None });
        assert_eq!(
            event_loop.pending,
            [
                Request::SaveClientId(CLIENT_ID.to_owned()),
                Request::SignIn {
                    client_id: CLIENT_ID.to_owned()
                },
            ]
        );
    }

    #[test]
    fn sign_in_shows_the_code_then_loads() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        event_loop.start(&mut app, Some(CLIENT_ID.to_owned()));
        let code = SignInCode {
            verification_uri: "https://microsoft.com/devicelogin".to_owned(),
            user_code: "FJ9WRZ8LK".to_owned(),
        };
        event_loop.apply(&mut app, Update::SignInCode(code.clone()));
        assert_eq!(app.screen, Screen::SignIn { code: Some(code) });

        event_loop.pending.clear();
        event_loop.apply(&mut app, Update::SignedIn);
        assert_eq!(app.screen, Screen::Loading { error: None });
        assert_eq!(event_loop.pending, [Request::Load]);
    }

    #[test]
    fn a_failed_sign_in_returns_to_the_client_id_with_the_reason() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        event_loop.start(&mut app, Some(CLIENT_ID.to_owned()));
        event_loop.apply(&mut app, Update::SignInFailed("declined".to_owned()));
        assert_eq!(
            app.screen,
            Screen::ClientId {
                input: CLIENT_ID.to_owned(),
                error: Some("declined".to_owned())
            }
        );
    }

    #[test]
    fn a_failed_load_retries_on_r() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        app.screen = Screen::Loading { error: None };
        event_loop.handle_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(event_loop.pending, []);

        event_loop.apply(&mut app, Update::LoadFailed("offline".to_owned()));
        event_loop.handle_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(app.screen, Screen::Loading { error: None });
        assert_eq!(event_loop.pending, [Request::Load]);
    }

    #[test]
    fn loading_opens_the_most_active_chat_and_starts_watching() {
        let mut event_loop = EventLoop::new();
        let mut app = App::new();
        app.screen = Screen::Loading { error: None };
        event_loop.apply(
            &mut app,
            Update::Loaded(Startup {
                user_name: Some("Me".to_owned()),
                chats: vec![chat("quiet"), chat("busy")],
                last_message_times: HashMap::from([(
                    "busy".to_owned(),
                    "2026-01-01T00:00:00Z".to_owned(),
                )]),
            }),
        );
        assert_eq!(app.screen, Screen::Chats);
        assert_eq!(order_of(&app), ["busy", "quiet"]);
        assert_eq!(app.current_user_name.as_deref(), Some("Me"));
        assert_eq!(
            event_loop.pending,
            [Request::WatchChats, Request::Messages("busy".to_owned())]
        );
    }

    #[test]
    fn a_late_load_does_not_reopen_the_chat_screen() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.apply(
            &mut app,
            Update::Loaded(Startup {
                user_name: None,
                chats: Vec::new(),
                last_message_times: HashMap::new(),
            }),
        );
        assert_eq!(order_of(&app), ["a"]);
        assert_eq!(event_loop.pending, []);
    }

    #[test]
    fn ctrl_c_quits_from_any_screen() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for screen in [
            Screen::ClientId {
                input: String::new(),
                error: None,
            },
            Screen::SignIn { code: None },
            Screen::Chats,
        ] {
            let (mut event_loop, mut app) = setup(&["a"]);
            app.screen = screen;
            app.input_mode = true;
            assert_eq!(event_loop.handle_key(&mut app, ctrl_c), Flow::Quit);
        }
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
    fn shift_or_alt_enter_starts_a_new_line() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.handle_key(&mut app, key(KeyCode::Char('i')));
        event_loop.handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        event_loop.handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(app.input_buffer, "\n\n");
        assert!(app.input_mode);
        assert_eq!(event_loop.pending, []);
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
    fn n_cycles_notifications_and_queues_the_save() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.handle_key(&mut app, key(KeyCode::Char('n')));
        assert_eq!(app.notification_mode, NotificationMode::Console);
        assert_eq!(
            event_loop.pending,
            [Request::SaveNotificationMode(NotificationMode::Console)]
        );
    }

    #[test]
    fn the_theme_picker_previews_then_saves_on_enter() {
        let (mut event_loop, mut app) = setup(&["a"]);
        event_loop.handle_key(&mut app, key(KeyCode::Char('t')));
        assert!(app.picker.is_some());
        event_loop.handle_key(&mut app, key(KeyCode::Char('j')));
        event_loop.handle_key(&mut app, key(KeyCode::Tab));
        let previewed = app.theme;
        assert_eq!(
            app.selected_chat_id(),
            Some("a"),
            "j moves the picker, not the chats"
        );

        event_loop.handle_key(&mut app, key(KeyCode::Enter));
        assert!(app.picker.is_none());
        assert_eq!(app.theme, previewed);
        assert_eq!(event_loop.pending, [Request::SaveTheme(previewed.key())]);
        assert!(previewed.key().ends_with("-light"));
    }

    #[test]
    fn esc_leaves_the_theme_picker_without_saving() {
        let (mut event_loop, mut app) = setup(&["a"]);
        let before = app.theme;
        event_loop.handle_key(&mut app, key(KeyCode::Char('t')));
        event_loop.handle_key(&mut app, key(KeyCode::Down));
        event_loop.handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.theme, before);
        assert_eq!(event_loop.pending, []);
        assert_eq!(
            event_loop.handle_key(&mut app, key(KeyCode::Char('q'))),
            Flow::Quit
        );
    }

    #[test]
    fn the_open_chat_reloads_every_refresh_after_the_first() {
        let (mut event_loop, app) = setup(&["a"]);
        for _ in 0..MESSAGE_REFRESH_TICKS {
            event_loop.tick(&app);
        }
        assert_eq!(event_loop.pending, []);
        for _ in 0..MESSAGE_REFRESH_TICKS {
            event_loop.tick(&app);
        }
        assert_eq!(event_loop.pending, [Request::Messages("a".to_owned())]);
    }
}
