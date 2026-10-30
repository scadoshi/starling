//! Starling: Microsoft Teams chats in the terminal.

mod api;
mod app;
mod auth;
mod config;
mod event_loop;
mod message;
mod ui;

use anyhow::Result;

use crate::app::App;
use crate::event_loop::EventLoop;

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();
    if let Some(mode) = config::load_config().and_then(|c| c.notification_mode) {
        app.notification_mode = mode;
    }

    let mut terminal = ratatui::try_init()?;
    let result = EventLoop::new()
        .run(&mut terminal, &mut app, auth::client_id())
        .await;
    ratatui::try_restore()?;
    result
}
