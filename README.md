# Starling

Microsoft Teams chats in the terminal, built on [ratatui](https://ratatui.rs). Read and send chat messages without opening the Teams app.

Starling signs in with Microsoft's device-code flow against an Azure app you register yourself, and talks only to `login.microsoftonline.com` and `graph.microsoft.com`. It asks for `User.Read`, `Chat.ReadWrite` and `offline_access`, nothing else.

## Setup

1. Register an Azure app and copy its client ID. [AZURE_SETUP.md](AZURE_SETUP.md) walks through it. If your organization blocks app registrations, your IT admin has to do this step.
2. Install and run:

   ```bash
   cargo install --git https://github.com/scadoshi/starling
   starling
   ```

3. Paste the client ID when Starling asks for it. It shows a URL and a code: open the URL on any device, enter the code, and sign in. Both the client ID and the sign-in are saved, so later runs go straight to your chats.

A `CLIENT_ID` environment variable overrides the saved client ID.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k` or arrows | Next / previous chat |
| `PgDn` / `PgUp` or `J` / `K` | Scroll messages |
| `i` | Compose |
| `Enter` | Send |
| `Shift+Enter` | New line (`Alt+Enter` in terminals that send Shift+Enter as Enter, such as Terminal.app) |
| `Esc` | Cancel compose |
| `n` | Cycle new-message notifications: none, terminal bell, desktop, both |
| `t` | Pick a theme: move to preview, `Tab` for dark or light, `Enter` to keep, `Esc` to cancel |
| `O` | Sign out, then sign in again, as the same or another account |
| `c` | On the sign-in screen, change the client ID |
| `q` | Quit |
| `Ctrl+C` | Quit from any screen |

## Themes

Starling has the 50 themes from [orangui](https://github.com/scadoshi/orangui), each in dark and light, plus Terminal, which uses your terminal's own colors and is the default. The orangui themes are 24-bit color, which iTerm2, Ghostty, kitty, WezTerm and current Terminal.app show correctly; inside tmux, turn on its truecolor support.

## Files

The config folder is `~/Library/Application Support/starling/` on macOS and `~/.config/starling/` on Linux. It holds `config.json`, `token.json` and `profile.json`, all readable by your user only. Signing out with `O` deletes `token.json` and `profile.json`; the client ID stays.

## Development

Needs Rust 1.88 or newer. Formatting uses nightly rustfmt; see [context/development/commit_guidelines.md](context/development/commit_guidelines.md) for the checks CI runs.

## Credits

Starling started as a fork of [nospor/teams-tui](https://github.com/nospor/teams-tui) by nospor, who wrote the original client. MIT licensed; see [LICENSE](LICENSE).
