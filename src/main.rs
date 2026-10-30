//! Starling: Microsoft Teams chats in the terminal.

mod api;
mod app;
mod auth;
mod config;
mod event_loop;
mod message;
#[rustfmt::skip]
mod palettes;
mod theme;
mod ui;

use std::io;

use anyhow::Result;
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal};

use crate::app::App;
use crate::event_loop::EventLoop;
use crate::theme::Theme;

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();
    if let Some(config) = config::load_config() {
        if let Some(mode) = config.notification_mode {
            app.notification_mode = mode;
        }
        if let Some(theme) = config.theme.as_deref().and_then(Theme::parse) {
            app.theme = theme;
        }
    }

    let mut terminal = ratatui::try_init()?;
    let enhanced = enable_key_disambiguation();
    let result = EventLoop::new()
        .run(&mut terminal, &mut app, auth::client_id())
        .await;
    if enhanced {
        execute!(io::stdout(), PopKeyboardEnhancementFlags).ok();
    }
    ratatui::try_restore()?;
    result
}

/// Asks the terminal to report modifiers on keys like Enter, so Shift+Enter differs from Enter. Returns whether the terminal took it.
///
/// Terminals without the kitty keyboard protocol, such as Terminal.app, send Shift+Enter as Enter; Alt+Enter covers them.
fn enable_key_disambiguation() -> bool {
    terminal::supports_keyboard_enhancement().unwrap_or(false)
        && execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
}
