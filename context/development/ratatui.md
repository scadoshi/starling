# Ratatui Guide

How Starling uses ratatui 0.30. Distilled from the community [ratatui-tui skill](https://github.com/blacktop/dotfiles/tree/master/ai/skills/ratatui-tui) and checked against ratatui's own docs, then cut down to what applies here. Where it disagrees with scadoshi-rust (it reaches for `color-eyre` and allows `unwrap` in places), scadoshi-rust wins.

## Shape of the app

Starling follows the Elm Architecture. `App` is the model, `event_loop.rs` is update, `ui.rs` is view.

- State lives in `App`. Handlers change it; nothing else does.
- `ui::draw` reads `App` and renders. The one write it makes is the scroll bounds, because only a render knows the wrapped height.
- Which screen is up is an enum, `app::Screen`, not a set of bools. Each screen draws in its own function and takes its own keys.
- Handlers never do I/O. They queue a `Request`; `EventLoop::dispatch` spawns it, and the result comes back as an `Update`. Tests drive handlers and assert on the queue.

## The loop

One `tokio::select!` over three sources: crossterm's `EventStream` for keys, the `Update` channel, and a tick. The tick redraws for timed effects (the bell flash) and paces the message refresh. Draw once per pass, before waiting.

Only `KeyEventKind::Press` counts; Windows also reports releases. `Ctrl+C` quits from any screen, because raw mode swallows the signal.

## Terminal setup

`ratatui::try_init()` enters raw mode and the alternate screen and installs a panic hook that restores the terminal. `ratatui::try_restore()` undoes it. No hand-written guard or hook. Mouse capture stays off so text can be selected.

Nothing prints to stdout once the terminal is initialized. A message for the user goes on screen.

## Styling

Use the `Stylize` helpers: `"Me".green().bold()`, `Style::new().cyan()`. Not `Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)`.

| Meaning | Style |
| --- | --- |
| Primary, selection, other people | `.cyan()` |
| The user, success, focus | `.green()` |
| Errors | `.red()` |
| Attachments, highlights | `.yellow()` |
| Hints, secondary text | `.dim()` |
| Key names in hints | `.bold()` |

No `Color::White`, `Color::Black` or blue: they vanish on one theme or the other. Text that needs no color takes the terminal's default. Color is never the only signal: selection also gets bold and a symbol, errors also say so in words.

## Layout

Destructure areas with `Layout::vertical([..]).areas(area)` rather than indexing `split`. `Constraint::Fill(1)` for the part that takes the rest. `Rect::centered` for a dialog.

## Text

Text from the network goes through `message::sanitize` or `message::clean_text` before it becomes a `Span`. Long text is wrapped by `message::wrap` or by `Paragraph::wrap`, and `Paragraph::line_count` (feature `unstable-rendered-line-info`) measures it for scrolling.

## Testing

Render into `ratatui::backend::TestBackend` and assert on the buffer's text. Test what a user would see, such as a name on screen or a scroll position, not the widget tree.
