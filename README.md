# Starling

Microsoft Teams chats in the terminal, built on [ratatui](https://ratatui.rs). Read and send chat messages without opening the Teams app.

Starling signs in with Microsoft's device-code flow against an Azure app you register yourself, and talks only to `login.microsoftonline.com` and `graph.microsoft.com`. It asks for `User.Read`, `Chat.ReadWrite` and `offline_access`, nothing else.

## Setup

1. Register an Azure app and copy its client ID. [AZURE_SETUP.md](AZURE_SETUP.md) walks through it. If your organization blocks app registrations, your IT admin has to do this step.
2. Put the client ID in `config.json` in Starling's config folder: `~/Library/Application Support/starling/` on macOS, `~/.config/starling/` on Linux.

   ```json
   { "client_id": "your-client-id" }
   ```

   A `CLIENT_ID` environment variable, or a `.env` file in the directory you run from, works too and takes precedence.
3. Install and run:

   ```bash
   cargo install --git https://github.com/scadoshi/starling
   starling
   ```

The first run prints a URL and a code. Open the URL, enter the code, and sign in. The token is saved and refreshed after that.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k` or arrows | Next / previous chat |
| `PgDn` / `PgUp` or `J` / `K` | Scroll messages |
| `i` | Compose |
| `Enter` | Send |
| `Alt+Enter` | New line |
| `Esc` | Cancel compose |
| `n` | Cycle new-message notifications: none, terminal bell, desktop, both |
| `q` | Quit |

## Files

The config folder holds `config.json`, `token.json` and `profile.json`, all readable by your user only. Delete `token.json` to sign in again, and `profile.json` too if you switch accounts.

## Development

Needs Rust 1.88 or newer. Formatting uses nightly rustfmt; see [context/development/commit_guidelines.md](context/development/commit_guidelines.md) for the checks CI runs.

## Credits

Starling started as a fork of [nospor/teams-tui](https://github.com/nospor/teams-tui) by nospor, who wrote the original client. MIT licensed; see [LICENSE](LICENSE).
