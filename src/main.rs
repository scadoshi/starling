//! Starling: Microsoft Teams chats in the terminal.

mod api;
mod app;
mod auth;
mod config;
mod event_loop;
mod message;
mod ui;

use std::collections::HashMap;
use std::io::{self, Stdout};

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::api::Chat;
use crate::app::{App, ChatOrder};
use crate::event_loop::EventLoop;

#[tokio::main]
async fn main() -> Result<()> {
    let (mut app, order) = startup().await?;
    let mut tui = Tui::enter()?;
    let result = EventLoop::new(order).run(&mut tui.terminal, &mut app);
    drop(tui);
    result
}

/// Signs in and loads the chat list, printing progress before the TUI starts.
#[expect(
    clippy::print_stdout,
    reason = "startup progress prints before the TUI takes the terminal"
)]
async fn startup() -> Result<(App, ChatOrder)> {
    println!("Starling\n");

    let access_token = auth::fetch_access_token().await.context("Sign-in failed")?;
    let me = api::fetch_me(&access_token).await;
    match &me {
        Ok(user) => println!("Signed in as {}", message::sanitize(&user.display_name)),
        Err(e) => println!("Signed in, but could not fetch your profile: {e:#}"),
    }

    println!("Fetching chats...");
    let mut chats = api::fetch_chats(&access_token)
        .await
        .context("Failed to fetch chats")?;

    println!("Loading the newest message of {} chats...", chats.len());
    let times = last_message_times(&access_token, &chats).await;
    app::sort_by_activity(&mut chats, &times);

    let order = ChatOrder::new(&chats, times);
    let mut app = App::new();
    app.status = format!("Loaded {} chats", chats.len());
    app.set_chats(chats);
    app.current_user_name = me.ok().map(|user| user.display_name);
    if let Some(mode) = config::load_config().and_then(|c| c.notification_mode) {
        app.notification_mode = mode;
    }
    Ok((app, order))
}

/// The time of each chat's newest message, fetched for all chats at once. Chats that fail or are empty are left out.
async fn last_message_times(access_token: &str, chats: &[Chat]) -> HashMap<String, String> {
    let tasks: Vec<_> = chats
        .iter()
        .map(|chat| {
            let token = access_token.to_owned();
            let chat_id = chat.id.clone();
            tokio::spawn(async move {
                let messages = api::fetch_messages(&token, &chat_id).await.ok()?;
                let time = app::newest(&messages)?.created_date_time.clone();
                Some((chat_id, time))
            })
        })
        .collect();

    let mut times = HashMap::new();
    for task in tasks {
        if let Ok(Some((chat_id, time))) = task.await {
            times.insert(chat_id, time);
        }
    }
    times
}

/// The terminal in raw mode on the alternate screen, put back as it was on drop.
struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout))?,
        })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        disable_raw_mode().ok();
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen).ok();
        self.terminal.show_cursor().ok();
    }
}
