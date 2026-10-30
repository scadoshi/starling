//! Draws the chat list, the message view, the compose box and the status bar.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::api::Message;
use crate::app::App;
use crate::message::{self, Segment};

/// The message view shows at most this many of the newest messages.
const MAX_MESSAGES: usize = 100;

/// Message text wraps at this share of the view width, in percent, so both sides of the conversation stay readable.
const TEXT_WIDTH_PERCENT: usize = 90;

/// A border on each side.
const BORDERS: u16 = 2;

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App) {
    let [content, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)])
        .margin(1)
        .areas(f.area());
    let [chats, right] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(content);

    if app.input_mode {
        let [messages, input] =
            Layout::vertical([Constraint::Min(3), Constraint::Min(3)]).areas(right);
        draw_messages(f, app, messages);
        draw_input(f, app, input);
    } else {
        draw_messages(f, app, right);
    }
    draw_chat_list(f, app, chats);
    draw_status(f, app, status);
}

fn draw_chat_list(f: &mut Frame<'_>, app: &App, area: Rect) {
    let items: Vec<ListItem<'_>> = app
        .chats
        .iter()
        .enumerate()
        .map(|(i, chat)| {
            let name_style = if i == app.selected_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", chat.kind),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(chat.display_name.as_str(), name_style),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Teams Chats (j↓/k↑ to navigate, q to quit)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)),
    );
    f.render_widget(list, area);
}

fn draw_messages(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(BORDERS);
    let lines = if app.loading_messages {
        vec![Line::from("Loading messages...")]
    } else if app.messages.is_empty() {
        vec![Line::from("No messages yet.")]
    } else {
        message_lines(
            &app.messages,
            app.current_user_name.as_deref(),
            usize::from(inner_width),
        )
    };

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = u16::try_from(paragraph.line_count(inner_width)).unwrap_or(u16::MAX);
    let viewport = area.height.saturating_sub(BORDERS);
    app.max_scroll = total.saturating_sub(viewport);
    app.scroll_offset = if app.snap_to_bottom {
        app.max_scroll
    } else {
        app.scroll_offset.min(app.max_scroll)
    };

    let title = if app.input_mode {
        "Messages (ESC to cancel)"
    } else {
        "Messages (i to compose, PgUp(K)/PgDn(J) to scroll)"
    };
    let paragraph = paragraph
        .block(Block::default().title(title).borders(Borders::ALL))
        .scroll((app.scroll_offset, 0));
    f.render_widget(paragraph, area);
}

/// The conversation as lines, oldest at the top, with the user's own messages on the right.
///
/// A header with the sender and time starts each run of messages from one sender, and a new hour starts a new run.
fn message_lines(messages: &[Message], me: Option<&str>, width: usize) -> Vec<Line<'static>> {
    let text_width = width.saturating_mul(TEXT_WIDTH_PERCENT) / 100;
    let mut lines = Vec::new();
    let mut previous: Option<(&str, Option<String>)> = None;

    for msg in messages.iter().take(MAX_MESSAGES).rev() {
        let sender = msg.sender_name();
        let time = DateTime::parse_from_rfc3339(&msg.created_date_time)
            .ok()
            .map(|t| t.with_timezone(&Local));
        let hour = time.map(|t| t.format("%Y-%m-%d %H").to_string());
        let is_me = me == Some(sender);

        let new_run = previous.as_ref().is_none_or(|(last_sender, last_hour)| {
            *last_sender != sender || matches!((last_hour, &hour), (Some(a), Some(b)) if a != b)
        });
        if new_run {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            let when = time.map_or_else(
                || message::sanitize(&msg.created_date_time),
                |t| t.format("%b %d %H:%M").to_string(),
            );
            lines.push(header(sender, &when, is_me));
        }
        previous = Some((sender, hour));

        for segment in message::parse(msg.content()) {
            match segment {
                Segment::Text(html) => {
                    for line in message::wrap(&message::clean_text(&html), text_width) {
                        lines.push(align(Line::from(line), is_me));
                    }
                }
                Segment::Image { src, alt } => {
                    let label = message::image_label(src.as_deref(), alt.as_deref());
                    lines.push(align(attachment_line("🖼️", &label), is_me));
                }
                Segment::Attachment(id) => {
                    let attachment = msg.attachment(&id);
                    let name = attachment
                        .and_then(|a| a.name.as_deref())
                        .map_or_else(|| "Attachment".to_owned(), message::sanitize);
                    let content_type = attachment.and_then(|a| a.content_type.as_deref());
                    let icon = message::attachment_icon(&name, content_type);
                    lines.push(align(attachment_line(icon, &name), is_me));
                }
            }
        }
    }
    lines
}

fn header(sender: &str, when: &str, is_me: bool) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    if is_me {
        Line::styled(format!("{when} Me"), bold.fg(Color::Green)).right_aligned()
    } else {
        let sender = message::sanitize(sender);
        Line::styled(format!("{sender} {when}"), bold.fg(Color::Cyan))
    }
}

fn attachment_line(icon: &str, name: &str) -> Line<'static> {
    Line::styled(format!("{icon} {name}"), Style::default().fg(Color::Yellow))
}

fn align(line: Line<'static>, is_me: bool) -> Line<'static> {
    if is_me { line.right_aligned() } else { line }
}

fn draw_input(f: &mut Frame<'_>, app: &App, area: Rect) {
    let input = Paragraph::new(app.input_buffer.as_str())
        .block(
            Block::default()
                .title("Type your message (Enter to send, Alt+Enter for new line, ESC to cancel)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    f.render_widget(input, area);
    f.set_cursor_position(input_cursor(&app.input_buffer, area));
}

/// Where the cursor sits after the last character typed, kept inside the box's borders.
fn input_cursor(buffer: &str, area: Rect) -> Position {
    let last_line = buffer.rsplit('\n').next().unwrap_or_default();
    let column = u16::try_from(last_line.chars().count()).unwrap_or(u16::MAX);
    let row = u16::try_from(buffer.matches('\n').count()).unwrap_or(u16::MAX);
    let right = area.right().saturating_sub(BORDERS);
    let bottom = area.bottom().saturating_sub(BORDERS);
    Position {
        x: area.x.saturating_add(1).saturating_add(column).min(right),
        y: area.y.saturating_add(1).saturating_add(row).min(bottom),
    }
}

fn draw_status(f: &mut Frame<'_>, app: &App, area: Rect) {
    let flashing = app
        .visual_bell_until
        .is_some_and(|until| std::time::Instant::now() < until);
    let (border, text) = if flashing {
        let red = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        (red, red.bg(Color::White))
    } else {
        let green = Style::default().fg(Color::Green);
        (green, green)
    };

    let status = Paragraph::new(format!(
        "{} | Notification (n): {}",
        message::sanitize(&app.status),
        app.notification_mode
    ))
    .block(
        Block::default()
            .title("Status")
            .borders(Borders::ALL)
            .border_style(border),
    )
    .style(text);
    f.render_widget(status, area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::api::{MessageBody, MessageFrom, MessageUser};
    use crate::app::tests::{chat, message};

    fn from(sender: &str, id: &str, created: &str, body: &str) -> Message {
        Message {
            from: Some(MessageFrom {
                user: Some(MessageUser {
                    display_name: Some(sender.to_owned()),
                }),
            }),
            body: Some(MessageBody {
                content: Some(body.to_owned()),
            }),
            ..message(id, created)
        }
    }

    fn text_of(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn message_lines_group_a_run_under_one_header_oldest_first() {
        let messages = [
            from("Ann", "3", "2026-01-01T10:02:00Z", "<p>third</p>"),
            from("Ann", "2", "2026-01-01T10:01:00Z", "second"),
            from("Bo", "1", "2026-01-01T10:00:00Z", "first"),
        ];
        let lines = text_of(&message_lines(&messages, None, 80));
        assert!(lines.first().is_some_and(|l| l.starts_with("Bo ")));
        assert_eq!(lines.get(1).map(String::as_str), Some("first"));
        assert_eq!(lines.get(2).map(String::as_str), Some(""));
        assert!(lines.get(3).is_some_and(|l| l.starts_with("Ann ")));
        assert_eq!(
            lines.get(4..).map(<[String]>::to_vec),
            Some(vec!["second".to_owned(), "third".to_owned()])
        );
    }

    #[test]
    fn message_lines_put_my_messages_on_the_right() {
        let messages = [from("Me", "1", "2026-01-01T10:00:00Z", "hello")];
        let lines = message_lines(&messages, Some("Me"), 80);
        assert!(
            lines
                .iter()
                .all(|l| l.alignment == Some(ratatui::layout::Alignment::Right))
        );
        assert!(
            lines
                .first()
                .is_some_and(|l| l.to_string().ends_with(" Me"))
        );
    }

    #[test]
    fn message_lines_show_attachments_by_name() {
        let mut msg = from(
            "Ann",
            "1",
            "2026-01-01T10:00:00Z",
            r#"see <attachment id="a"></attachment>"#,
        );
        msg.attachments = Some(vec![crate::api::MessageAttachment {
            id: "a".to_owned(),
            name: Some("plan.pdf".to_owned()),
            content_type: None,
        }]);
        let lines = text_of(&message_lines(&[msg], None, 80));
        assert!(lines.iter().any(|l| l == "📄 plan.pdf"));
    }

    #[test]
    fn message_lines_strip_escapes_from_senders() {
        let messages = [from("Eve\u{1b}[2J", "1", "2026-01-01T10:00:00Z", "x")];
        let lines = text_of(&message_lines(&messages, None, 80));
        assert!(lines.iter().all(|l| !l.contains('\u{1b}')));
    }

    #[test]
    fn input_cursor_follows_the_last_line_and_stays_inside() {
        let area = Rect::new(10, 20, 30, 5);
        assert_eq!(input_cursor("", area), Position { x: 11, y: 21 });
        assert_eq!(input_cursor("ab\nhé", area), Position { x: 13, y: 22 });
        assert_eq!(
            input_cursor(&"x".repeat(100), area),
            Position { x: 38, y: 21 }
        );
        assert_eq!(
            input_cursor("\n\n\n\n\n\n", area),
            Position { x: 11, y: 23 }
        );
    }

    #[test]
    fn draw_shows_chats_and_snaps_to_the_newest_message() {
        let mut app = App::new();
        let mut named = chat("c1");
        named.display_name = "Launch crew".to_owned();
        app.set_chats(vec![named]);
        app.set_messages(
            (0..40)
                .rev()
                .map(|i| {
                    from(
                        "Ann",
                        &i.to_string(),
                        &format!("2026-01-01T10:{i:02}:00Z"),
                        &format!("line {i}"),
                    )
                })
                .collect(),
        );

        let text = screen(&mut app, 100, 20);
        assert!(text.contains("Launch crew"));
        assert!(text.contains("line 39"));
        assert!(!text.contains("line 0 "));
        assert!(app.max_scroll > 0);
        assert_eq!(app.scroll_offset, app.max_scroll);
    }

    #[test]
    fn draw_keeps_a_scrolled_view_in_range() {
        let mut app = App::new();
        app.set_messages(vec![from("Ann", "1", "2026-01-01T10:00:00Z", "only")]);
        app.snap_to_bottom = false;
        app.scroll_offset = 500;
        screen(&mut app, 80, 20);
        assert_eq!(app.scroll_offset, 0);
    }
}
