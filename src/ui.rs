//! Draws whichever screen is up: the client ID prompt, sign-in, loading, or the chats, in the current theme.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Clear, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::api::Message;
use crate::app::{App, Screen, SignInCode, ThemePicker};
use crate::message::{self, Segment};
use crate::theme::{self, Palette, Row};

/// The message view shows at most this many of the newest messages.
const MAX_MESSAGES: usize = 100;

/// Message text wraps at this share of the view width, in percent, so both sides of the conversation stay readable.
const TEXT_WIDTH_PERCENT: usize = 90;

/// A border on each side.
const BORDERS: u16 = 2;

/// The setup panels are at most this wide, in cells.
const PANEL_WIDTH: u16 = 76;

/// A border and a cell of padding on each side of a panel.
const PANEL_FRAME: u16 = 4;

/// The theme picker is at most this wide and this tall, in cells.
const PICKER_WIDTH: u16 = 44;
const PICKER_HEIGHT: u16 = 30;

/// Marks the selected row, so the selection does not rest on color alone.
const SELECTED: &str = "> ";
const UNSELECTED: &str = "  ";

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App) {
    let palette = app.theme.palette();
    f.render_widget(Block::new().style(palette.base), f.area());
    match &app.screen {
        Screen::ClientId { input, error } => {
            draw_client_id(f, &palette, input, error.as_deref());
        }
        Screen::SignIn { code } => draw_sign_in(f, &palette, code.as_ref()),
        Screen::Loading { error } => draw_loading(f, &palette, error.as_deref()),
        Screen::Chats => {
            draw_chats(f, &palette, app);
            if let Some(picker) = app.picker {
                draw_theme_picker(f, &palette, app, picker);
            }
        }
    }
}

// == setup screens ==

fn draw_client_id(f: &mut Frame<'_>, palette: &Palette, input: &str, error: Option<&str>) {
    let width = panel_text_width(f.area());
    let intro = Paragraph::new(vec![
        Line::from("Starling signs in through your own Azure app registration."),
        Line::from(vec![
            "Paste its Application (client) ID. ".into(),
            "AZURE_SETUP.md".bold(),
            " has the steps.".into(),
        ]),
    ])
    .wrap(Wrap { trim: true });
    let error = error.map(|e| {
        Paragraph::new(Line::from(message::sanitize(e)).style(palette.error))
            .wrap(Wrap { trim: true })
    });
    let hints = hints(palette, &[("Enter", "sign in"), ("Esc", "quit")]);

    let intro_height = line_count(&intro, width);
    let error_height = error.as_ref().map_or(0, |e| line_count(e, width));
    let inner = panel(f, palette, sum(&[intro_height, 1, 3, error_height, 1, 1]));
    let [intro_area, _, input_area, error_area, _, hints_area] = Layout::vertical([
        Constraint::Length(intro_height),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(error_height),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    f.render_widget(intro, intro_area);
    f.render_widget(
        Paragraph::new(input).block(
            Block::bordered()
                .title(" Client ID ")
                .border_style(palette.me),
        ),
        input_area,
    );
    f.set_cursor_position(input_cursor(input, input_area));
    if let Some(error) = error {
        f.render_widget(error, error_area);
    }
    f.render_widget(hints, hints_area);
}

fn draw_sign_in(f: &mut Frame<'_>, palette: &Palette, code: Option<&SignInCode>) {
    let body = match code {
        None => vec![Line::from("Signing in...")],
        Some(code) => vec![
            Line::from("On any device, open"),
            Line::from(message::sanitize(&code.verification_uri))
                .style(palette.accent)
                .bold()
                .underlined(),
            Line::default(),
            Line::from("and enter the code"),
            Line::from(message::sanitize(&code.user_code))
                .style(palette.me)
                .bold(),
            Line::default(),
            Line::from("Waiting for you to sign in...").style(palette.muted),
        ],
    };
    draw_message_panel(f, palette, body, &[("q", "quit")]);
}

fn draw_loading(f: &mut Frame<'_>, palette: &Palette, error: Option<&str>) {
    match error {
        None => draw_message_panel(
            f,
            palette,
            vec![Line::from("Loading your chats...")],
            &[("q", "quit")],
        ),
        Some(error) => draw_message_panel(
            f,
            palette,
            vec![
                Line::from("Could not load your chats.")
                    .style(palette.error)
                    .bold(),
                Line::default(),
                Line::from(message::sanitize(error)),
            ],
            &[("r", "retry"), ("q", "quit")],
        ),
    }
}

/// A panel of text with a row of key hints under it.
fn draw_message_panel(
    f: &mut Frame<'_>,
    palette: &Palette,
    body: Vec<Line<'_>>,
    keys: &[(&str, &str)],
) {
    let width = panel_text_width(f.area());
    let body = Paragraph::new(body).wrap(Wrap { trim: true });
    let body_height = line_count(&body, width);
    let inner = panel(f, palette, sum(&[body_height, 1, 1]));
    let [body_area, _, hints_area] = Layout::vertical([
        Constraint::Length(body_height),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    f.render_widget(body, body_area);
    f.render_widget(hints(palette, keys), hints_area);
}

/// Clears and frames a centered panel `height` lines tall inside, and returns the area inside the frame.
fn panel(f: &mut Frame<'_>, palette: &Palette, height: u16) -> Rect {
    let area = f.area().centered(
        Constraint::Max(PANEL_WIDTH),
        Constraint::Length(height.saturating_add(BORDERS)),
    );
    let block = Block::bordered()
        .title(" Starling ".bold())
        .border_style(palette.border)
        .style(palette.base)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);
    inner
}

fn panel_text_width(area: Rect) -> u16 {
    area.width.min(PANEL_WIDTH).saturating_sub(PANEL_FRAME)
}

fn line_count(paragraph: &Paragraph<'_>, width: u16) -> u16 {
    u16::try_from(paragraph.line_count(width)).unwrap_or(u16::MAX)
}

fn sum(heights: &[u16]) -> u16 {
    heights.iter().fold(0, |total, h| total.saturating_add(*h))
}

/// Key hints: each key in bold, its action muted.
fn hints(palette: &Palette, keys: &[(&str, &str)]) -> Line<'static> {
    let spans: Vec<Span<'static>> = keys
        .iter()
        .flat_map(|&(key, action)| {
            [
                key.to_owned().bold(),
                Span::styled(format!(" {action}  "), palette.muted),
            ]
        })
        .collect();
    Line::from(spans)
}

// == chat screen ==

fn draw_chats(f: &mut Frame<'_>, palette: &Palette, app: &mut App) {
    let [content, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)])
        .margin(1)
        .areas(f.area());
    let [chats, right] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Fill(1)]).areas(content);

    if app.input_mode {
        let [messages, input] =
            Layout::vertical([Constraint::Fill(1), Constraint::Min(3)]).areas(right);
        draw_messages(f, palette, app, messages);
        draw_input(f, palette, app, input);
    } else {
        draw_messages(f, palette, app, right);
    }
    draw_chat_list(f, palette, app, chats);
    draw_status(f, palette, app, status);
}

fn draw_chat_list(f: &mut Frame<'_>, palette: &Palette, app: &App, area: Rect) {
    let items: Vec<ListItem<'_>> = app
        .chats
        .iter()
        .enumerate()
        .map(|(i, chat)| {
            let name = chat.display_name.as_str();
            let line = if i == app.selected_index {
                Line::from(vec![SELECTED.into(), name.into()])
                    .style(palette.accent)
                    .bold()
            } else {
                Line::from(vec![UNSELECTED.into(), name.into()])
            };
            ListItem::new(line)
        })
        .collect();

    let block = Block::bordered()
        .title(" Chats ")
        .border_style(palette.border);
    f.render_widget(List::new(items).block(block), area);
}

fn draw_messages(f: &mut Frame<'_>, palette: &Palette, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(BORDERS);
    let lines = if app.loading_messages {
        vec![Line::from("Loading messages...").style(palette.muted)]
    } else if app.messages.is_empty() {
        vec![Line::from("No messages yet.").style(palette.muted)]
    } else {
        message_lines(
            &app.messages,
            app.current_user_name.as_deref(),
            usize::from(inner_width),
            palette,
        )
    };

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = line_count(&paragraph, inner_width);
    let viewport = area.height.saturating_sub(BORDERS);
    app.max_scroll = total.saturating_sub(viewport);
    app.scroll_offset = if app.snap_to_bottom {
        app.max_scroll
    } else {
        app.scroll_offset.min(app.max_scroll)
    };

    let paragraph = paragraph
        .block(
            Block::bordered()
                .title(" Messages ")
                .border_style(palette.border),
        )
        .scroll((app.scroll_offset, 0));
    f.render_widget(paragraph, area);
}

/// The conversation as lines, oldest at the top, with the user's own messages on the right.
///
/// A header with the sender and time starts each run of messages from one sender, and a new hour starts a new run.
fn message_lines(
    messages: &[Message],
    me: Option<&str>,
    width: usize,
    palette: &Palette,
) -> Vec<Line<'static>> {
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
            lines.push(header(palette, sender, &when, is_me));
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
                    lines.push(align(attachment_line(palette, "🖼️", &label), is_me));
                }
                Segment::Attachment(id) => {
                    let attachment = msg.attachment(&id);
                    let name = attachment
                        .and_then(|a| a.name.as_deref())
                        .map_or_else(|| "Attachment".to_owned(), message::sanitize);
                    let content_type = attachment.and_then(|a| a.content_type.as_deref());
                    let icon = message::attachment_icon(&name, content_type);
                    lines.push(align(attachment_line(palette, icon, &name), is_me));
                }
            }
        }
    }
    lines
}

fn header(palette: &Palette, sender: &str, when: &str, is_me: bool) -> Line<'static> {
    if is_me {
        Line::from(format!("{when} Me"))
            .style(palette.me)
            .bold()
            .right_aligned()
    } else {
        let sender = message::sanitize(sender);
        Line::from(format!("{sender} {when}"))
            .style(palette.accent)
            .bold()
    }
}

fn attachment_line(palette: &Palette, icon: &str, name: &str) -> Line<'static> {
    Line::from(format!("{icon} {name}")).style(palette.attachment)
}

fn align(line: Line<'static>, is_me: bool) -> Line<'static> {
    if is_me { line.right_aligned() } else { line }
}

fn draw_input(f: &mut Frame<'_>, palette: &Palette, app: &App, area: Rect) {
    let input = Paragraph::new(app.input_buffer.as_str())
        .block(
            Block::bordered()
                .title(" Message ")
                .border_style(palette.me),
        )
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

fn draw_status(f: &mut Frame<'_>, palette: &Palette, app: &App, area: Rect) {
    let flashing = app
        .visual_bell_until
        .is_some_and(|until| std::time::Instant::now() < until);
    let style = if flashing {
        palette.error.bold().reversed()
    } else {
        palette.me
    };

    let keys = if app.input_mode {
        hints(
            palette,
            &[
                ("Enter", "send"),
                ("Shift+Enter", "new line"),
                ("Esc", "cancel"),
            ],
        )
    } else {
        let notify = format!("notifications: {}", app.notification_mode);
        hints(
            palette,
            &[
                ("j/k", "chats"),
                ("PgUp/PgDn", "scroll"),
                ("i", "compose"),
                ("t", "theme"),
                ("n", notify.as_str()),
                ("q", "quit"),
            ],
        )
    };

    let status = Paragraph::new(message::sanitize(&app.status))
        .style(style)
        .block(
            Block::bordered()
                .title(" Status ")
                .title_bottom(keys)
                .border_style(palette.border),
        );
    f.render_widget(status, area);
}

/// The theme list over the chat screen, which is already drawn in the theme under the selection.
fn draw_theme_picker(f: &mut Frame<'_>, palette: &Palette, app: &App, picker: ThemePicker) {
    let rows = theme::rows();
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|row| match *row {
            Row::Header(label) => ListItem::new(Line::from(label).style(palette.muted).bold()),
            Row::Choice(None) => ListItem::new("Terminal"),
            Row::Choice(Some(slug)) => ListItem::new(theme::name(slug)),
        })
        .collect();

    let height = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .saturating_add(BORDERS)
        .min(PICKER_HEIGHT);
    let area = f
        .area()
        .centered(Constraint::Max(PICKER_WIDTH), Constraint::Length(height));
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(format!(" Theme: {} ", app.theme))
                .title_bottom(hints(
                    palette,
                    &[("Tab", "dark/light"), ("Enter", "keep"), ("Esc", "cancel")],
                ))
                .border_style(palette.me)
                .style(palette.base),
        )
        .highlight_style(palette.accent.bold())
        .highlight_symbol(SELECTED)
        .highlight_spacing(HighlightSpacing::Always);

    f.render_widget(Clear, area);
    f.render_stateful_widget(
        list,
        area,
        &mut ListState::default().with_selected(Some(picker.row)),
    );
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
        let lines = text_of(&message_lines(
            &messages,
            None,
            80,
            &crate::theme::Theme::default().palette(),
        ));
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
        let lines = message_lines(
            &messages,
            Some("Me"),
            80,
            &crate::theme::Theme::default().palette(),
        );
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
        let lines = text_of(&message_lines(
            &[msg],
            None,
            80,
            &crate::theme::Theme::default().palette(),
        ));
        assert!(lines.iter().any(|l| l == "📄 plan.pdf"));
    }

    #[test]
    fn message_lines_strip_escapes_from_senders() {
        let messages = [from("Eve\u{1b}[2J", "1", "2026-01-01T10:00:00Z", "x")];
        let lines = text_of(&message_lines(
            &messages,
            None,
            80,
            &crate::theme::Theme::default().palette(),
        ));
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
        app.screen = Screen::Chats;
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
        app.screen = Screen::Chats;
        app.set_messages(vec![from("Ann", "1", "2026-01-01T10:00:00Z", "only")]);
        app.snap_to_bottom = false;
        app.scroll_offset = 500;
        screen(&mut app, 80, 20);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn the_client_id_screen_shows_the_input_and_why_it_was_refused() {
        let mut app = App::new();
        app.screen = Screen::ClientId {
            input: "abc".to_owned(),
            error: Some("That is not a client ID.".to_owned()),
        };
        let text = screen(&mut app, 100, 24);
        assert!(text.contains("Client ID"));
        assert!(text.contains("abc"));
        assert!(text.contains("That is not a client ID."));
        assert!(text.contains("AZURE_SETUP.md"));
    }

    #[test]
    fn the_sign_in_screen_shows_where_to_go_and_the_code() {
        let mut app = App::new();
        app.screen = Screen::SignIn {
            code: Some(SignInCode {
                verification_uri: "https://microsoft.com/devicelogin".to_owned(),
                user_code: "FJ9WRZ8LK".to_owned(),
            }),
        };
        let text = screen(&mut app, 100, 24);
        assert!(text.contains("https://microsoft.com/devicelogin"));
        assert!(text.contains("FJ9WRZ8LK"));
    }

    #[test]
    fn a_failed_load_says_why_and_offers_a_retry() {
        let mut app = App::new();
        app.screen = Screen::Loading {
            error: Some("GET /me/chats failed: 403".to_owned()),
        };
        let text = screen(&mut app, 100, 24);
        assert!(text.contains("Could not load your chats."));
        assert!(text.contains("403"));
        assert!(text.contains("retry"));
    }

    #[test]
    fn panels_fit_a_small_terminal() {
        let mut app = App::new();
        app.screen = Screen::Loading {
            error: Some("x ".repeat(400)),
        };
        screen(&mut app, 20, 6);
    }

    #[test]
    fn the_selected_chat_is_marked_without_color() {
        let mut app = App::new();
        app.screen = Screen::Chats;
        let mut named = chat("c1");
        named.display_name = "Launch crew".to_owned();
        app.set_chats(vec![named]);
        assert!(screen(&mut app, 100, 20).contains("> Launch crew"));
    }

    #[test]
    fn a_theme_paints_every_cell_including_the_setup_panels() {
        let mut app = App::new();
        app.theme = crate::theme::Theme::parse("gruvbox-dark").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let background = ratatui::style::Color::Rgb(0x28, 0x28, 0x28);
        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().all(|cell| cell.bg == background));
    }

    #[test]
    fn the_picker_lists_sections_and_names_over_the_chats() {
        let mut app = App::new();
        app.screen = Screen::Chats;
        app.open_theme_picker();
        let text = screen(&mut app, 100, 40);
        assert!(text.contains("Theme: Terminal"));
        assert!(text.contains("> Terminal"));
        assert!(text.contains("Warm & earthy"));
        assert!(text.contains("Gruvbox"));
    }
}
