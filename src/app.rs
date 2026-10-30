//! UI state, and the order the chat list is shown in.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::api::{Chat, Message};

/// Lines one `PageUp` or `PageDown` moves the message view.
const SCROLL_STEP: u16 = 10;

/// How a new message in another chat is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NotificationMode {
    None,
    /// Terminal bell plus a flash of the status bar.
    Console,
    /// A desktop notification.
    System,
    Both,
}

impl NotificationMode {
    /// The mode after this one, in the order `n` cycles through them.
    pub(crate) fn next(self) -> Self {
        match self {
            Self::None => Self::Console,
            Self::Console => Self::System,
            Self::System => Self::Both,
            Self::Both => Self::None,
        }
    }

    pub(crate) fn rings_bell(self) -> bool {
        matches!(self, Self::Console | Self::Both)
    }

    pub(crate) fn shows_desktop(self) -> bool {
        matches!(self, Self::System | Self::Both)
    }
}

impl fmt::Display for NotificationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::None => "None",
            Self::Console => "Console",
            Self::System => "System",
            Self::Both => "Both",
        };
        f.write_str(name)
    }
}

/// Everything the UI draws from.
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) chats: Vec<Chat>,
    pub(crate) status: String,
    pub(crate) selected_index: usize,
    pub(crate) current_user_name: Option<String>,
    /// The selected chat's messages, newest first, as Graph returns them.
    pub(crate) messages: Vec<Message>,
    pub(crate) loading_messages: bool,
    pub(crate) input_mode: bool,
    pub(crate) input_buffer: String,
    pub(crate) scroll_offset: u16,
    /// Set by each draw from the rendered height of the message view.
    pub(crate) max_scroll: u16,
    /// Pins the view to the newest message on the next draw.
    pub(crate) snap_to_bottom: bool,
    pub(crate) notification_mode: NotificationMode,
    pub(crate) visual_bell_until: Option<Instant>,
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            chats: Vec::new(),
            status: "Loading...".to_owned(),
            selected_index: 0,
            current_user_name: None,
            messages: Vec::new(),
            loading_messages: false,
            input_mode: false,
            input_buffer: String::new(),
            scroll_offset: 0,
            max_scroll: 0,
            snap_to_bottom: true,
            notification_mode: NotificationMode::None,
            visual_bell_until: None,
        }
    }

    /// Replaces the chat list and keeps the same chat selected when it is still there.
    pub(crate) fn set_chats(&mut self, chats: Vec<Chat>) {
        let selected = self.selected_chat_id().map(str::to_owned);
        self.chats = chats;
        self.selected_index = selected
            .and_then(|id| self.chats.iter().position(|c| c.id == id))
            .unwrap_or_else(|| self.selected_index.min(self.chats.len().saturating_sub(1)));
    }

    pub(crate) fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.loading_messages = false;
    }

    pub(crate) fn selected_chat(&self) -> Option<&Chat> {
        self.chats.get(self.selected_index)
    }

    pub(crate) fn selected_chat_id(&self) -> Option<&str> {
        self.selected_chat().map(|c| c.id.as_str())
    }

    /// Moves the selection down one chat, wrapping to the top.
    pub(crate) fn next_chat(&mut self) {
        self.selected_index = self
            .selected_index
            .checked_add(1)
            .filter(|&i| i < self.chats.len())
            .unwrap_or(0);
    }

    /// Moves the selection up one chat, wrapping to the bottom.
    pub(crate) fn previous_chat(&mut self) {
        self.selected_index = self
            .selected_index
            .checked_sub(1)
            .unwrap_or_else(|| self.chats.len().saturating_sub(1));
    }

    pub(crate) fn scroll_up(&mut self) {
        self.snap_to_bottom = false;
        self.scroll_offset = self.scroll_offset.saturating_sub(SCROLL_STEP);
    }

    /// Scrolls down, and pins the view to the bottom again once it reaches it.
    pub(crate) fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(SCROLL_STEP);
        if self.scroll_offset >= self.max_scroll {
            self.snap_to_bottom = true;
        }
    }
}

/// The newest message in a list, whatever order the list is in.
pub(crate) fn newest(messages: &[Message]) -> Option<&Message> {
    messages
        .iter()
        .max_by(|a, b| a.created_date_time.cmp(&b.created_date_time))
}

/// Sorts chats newest activity first: chats with a known last message by that message's time, then the rest by Graph's `lastUpdatedDateTime`.
///
/// Times are RFC 3339 in UTC, so comparing them as strings orders them by time.
pub(crate) fn sort_by_activity(chats: &mut [Chat], last_message_times: &HashMap<String, String>) {
    chats.sort_by(
        |a, b| match (last_message_times.get(&a.id), last_message_times.get(&b.id)) {
            (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => match (&a.last_updated, &b.last_updated) {
                (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        },
    );
}

/// A stable chat order that moves a chat to the top only when a new message arrives in it.
///
/// Graph's own order shifts when a chat is merely read, so the list keeps this order instead and uses Graph's list only for which chats exist and their details.
#[derive(Debug, Default)]
pub(crate) struct ChatOrder {
    ids: Vec<String>,
    last_message_ids: HashMap<String, String>,
    last_message_times: HashMap<String, String>,
}

impl ChatOrder {
    /// Starts from chats already sorted, with the last message times used to sort them.
    pub(crate) fn new(chats: &[Chat], last_message_times: HashMap<String, String>) -> Self {
        Self {
            ids: chats.iter().map(|c| c.id.clone()).collect(),
            last_message_ids: HashMap::new(),
            last_message_times,
        }
    }

    pub(crate) fn contains(&self, chat_id: &str) -> bool {
        self.ids.iter().any(|id| id == chat_id)
    }

    /// Adds chats not seen before: at the top when they have a message, else at the bottom.
    pub(crate) fn absorb(&mut self, chats: &[Chat]) {
        for chat in chats {
            if self.contains(&chat.id) {
                continue;
            }
            if self.last_message_times.contains_key(&chat.id) {
                self.ids.insert(0, chat.id.clone());
            } else {
                self.ids.push(chat.id.clone());
            }
        }
    }

    /// `chats` in this order, followed by any this order does not know yet.
    pub(crate) fn arrange(&self, chats: &[Chat]) -> Vec<Chat> {
        let known = self
            .ids
            .iter()
            .filter_map(|id| chats.iter().find(|c| &c.id == id));
        let unknown = chats.iter().filter(|c| !self.contains(&c.id));
        known.chain(unknown).cloned().collect()
    }

    pub(crate) fn move_to_top(&mut self, chat_id: &str) {
        self.ids.retain(|id| id != chat_id);
        self.ids.insert(0, chat_id.to_owned());
    }

    /// Records the newest message of a chat in the background and says whether it is new.
    ///
    /// The first message seen for a chat counts as new only when the chat is not in the order yet.
    pub(crate) fn record_background(&mut self, chat_id: &str, newest: &Message) -> bool {
        let is_new = self
            .last_message_ids
            .get(chat_id)
            .map_or_else(|| !self.contains(chat_id), |old| old != &newest.id);
        self.record(chat_id, newest);
        is_new
    }

    /// Records the newest message of the chat on screen and says whether it is new.
    ///
    /// The first message seen is never new: it is the chat loading.
    pub(crate) fn record_viewed(&mut self, chat_id: &str, newest: &Message) -> bool {
        let is_new = self
            .last_message_ids
            .get(chat_id)
            .is_some_and(|old| old != &newest.id);
        self.record(chat_id, newest);
        is_new
    }

    fn record(&mut self, chat_id: &str, newest: &Message) {
        self.last_message_ids
            .insert(chat_id.to_owned(), newest.id.clone());
        self.last_message_times
            .insert(chat_id.to_owned(), newest.created_date_time.clone());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn chat(id: &str) -> Chat {
        Chat {
            id: id.to_owned(),
            topic: None,
            kind: "group".to_owned(),
            last_updated: None,
            members: Vec::new(),
            display_name: String::new(),
        }
    }

    pub(crate) fn message(id: &str, created: &str) -> Message {
        Message {
            id: id.to_owned(),
            created_date_time: created.to_owned(),
            from: None,
            body: None,
            attachments: None,
        }
    }

    fn ids(chats: &[Chat]) -> Vec<&str> {
        chats.iter().map(|c| c.id.as_str()).collect()
    }

    fn app_with(chat_ids: &[&str]) -> App {
        let mut app = App::new();
        app.set_chats(chat_ids.iter().map(|id| chat(id)).collect());
        app
    }

    #[test]
    fn notification_mode_cycles_through_all_four() {
        let mut mode = NotificationMode::None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(
            seen,
            [
                NotificationMode::Console,
                NotificationMode::System,
                NotificationMode::Both,
                NotificationMode::None,
            ]
        );
        assert!(NotificationMode::Both.rings_bell() && NotificationMode::Both.shows_desktop());
        assert!(!NotificationMode::None.rings_bell() && !NotificationMode::None.shows_desktop());
    }

    #[test]
    fn selection_wraps_both_ways() {
        let mut app = app_with(&["a", "b", "c"]);
        app.previous_chat();
        assert_eq!(app.selected_index, 2);
        app.next_chat();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn selection_on_an_empty_list_stays_at_zero() {
        let mut app = App::new();
        app.next_chat();
        app.previous_chat();
        assert_eq!(app.selected_index, 0);
        assert!(app.selected_chat().is_none());
    }

    #[test]
    fn set_chats_follows_the_selected_chat() {
        let mut app = app_with(&["a", "b", "c"]);
        app.next_chat();
        app.set_chats(vec![chat("b"), chat("c"), chat("a")]);
        assert_eq!(app.selected_chat_id(), Some("b"));
    }

    #[test]
    fn set_chats_clamps_when_the_selected_chat_is_gone() {
        let mut app = app_with(&["a", "b", "c"]);
        app.previous_chat();
        app.set_chats(vec![chat("a")]);
        assert_eq!(app.selected_chat_id(), Some("a"));
    }

    #[test]
    fn scrolling_down_to_the_end_pins_to_the_bottom() {
        let mut app = App::new();
        app.max_scroll = 15;
        app.scroll_offset = 15;
        app.scroll_up();
        assert!(!app.snap_to_bottom);
        assert_eq!(app.scroll_offset, 5);
        app.scroll_down();
        assert!(app.snap_to_bottom);
    }

    #[test]
    fn newest_ignores_list_order() {
        let messages = [
            message("old", "2026-01-01T00:00:00Z"),
            message("new", "2026-02-01T00:00:00Z"),
        ];
        assert_eq!(newest(&messages).map(|m| m.id.as_str()), Some("new"));
        assert!(newest(&[]).is_none());
    }

    #[test]
    fn sort_puts_recent_messages_first_then_recent_updates() {
        let mut chats = vec![chat("quiet"), chat("old"), chat("new"), chat("updated")];
        if let Some(updated) = chats.iter_mut().find(|c| c.id == "updated") {
            updated.last_updated = Some("2026-03-01T00:00:00Z".to_owned());
        }
        let times = HashMap::from([
            ("old".to_owned(), "2026-01-01T00:00:00Z".to_owned()),
            ("new".to_owned(), "2026-02-01T00:00:00Z".to_owned()),
        ]);
        sort_by_activity(&mut chats, &times);
        assert_eq!(ids(&chats), ["new", "old", "updated", "quiet"]);
    }

    #[test]
    fn order_adds_unseen_chats_by_whether_they_have_messages() {
        let mut order = ChatOrder::new(
            &[chat("a")],
            HashMap::from([("hot".to_owned(), "2026-01-01T00:00:00Z".to_owned())]),
        );
        order.absorb(&[chat("a"), chat("cold"), chat("hot")]);
        assert_eq!(
            ids(&order.arrange(&[chat("cold"), chat("a"), chat("hot")])),
            ["hot", "a", "cold"]
        );
    }

    #[test]
    fn arrange_keeps_unknown_chats_and_drops_missing_ones() {
        let order = ChatOrder::new(&[chat("a"), chat("gone"), chat("b")], HashMap::new());
        assert_eq!(
            ids(&order.arrange(&[chat("b"), chat("new"), chat("a")])),
            ["a", "b", "new"]
        );
    }

    #[test]
    fn move_to_top_does_not_duplicate() {
        let mut order = ChatOrder::new(&[chat("a"), chat("b")], HashMap::new());
        order.move_to_top("b");
        order.move_to_top("b");
        assert_eq!(ids(&order.arrange(&[chat("a"), chat("b")])), ["b", "a"]);
    }

    #[test]
    fn background_first_sight_is_new_only_for_unlisted_chats() {
        let mut order = ChatOrder::new(&[chat("listed")], HashMap::new());
        let m1 = message("1", "2026-01-01T00:00:00Z");
        assert!(!order.record_background("listed", &m1));
        assert!(order.record_background("unlisted", &m1));
        assert!(!order.record_background("listed", &m1));
        assert!(order.record_background("listed", &message("2", "2026-01-02T00:00:00Z")));
    }

    #[test]
    fn viewed_first_sight_is_never_new() {
        let mut order = ChatOrder::new(&[], HashMap::new());
        assert!(!order.record_viewed("x", &message("1", "2026-01-01T00:00:00Z")));
        assert!(order.record_viewed("x", &message("2", "2026-01-02T00:00:00Z")));
    }
}
