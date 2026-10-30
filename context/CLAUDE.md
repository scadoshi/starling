# CLAUDE.md

Guidance for Claude Code and any other AI assistant working in this repository.

## Project Overview

Starling is a terminal client for Microsoft Teams chats, built on [ratatui](https://ratatui.rs) and crossterm. It started as a fork of [nospor/teams-tui](https://github.com/nospor/teams-tui) (MIT, deprecated upstream) and diverges from it freely. The name is the bird: starlings fly in coordinated flocks and mimic what they hear.

- One binary crate. `main.rs` runs the event loop, `ui.rs` draws, `app.rs` holds state, `api.rs` calls Microsoft Graph, `auth.rs` runs the OAuth device-code flow, `config.rs` loads `~/.config/starling/config.json`.
- The UI stays on ratatui. Don't swap the TUI framework.

## Security Rules

These are why the fork exists. Non-negotiable:

- Sign-in uses the OAuth device-code flow against the user's own Azure app registration (`AZURE_SETUP.md`). No borrowed first-party client IDs, no scraped session tokens, no unofficial Teams endpoints.
- Network traffic goes to `login.microsoftonline.com` and `graph.microsoft.com` only.
- Anything holding a token or profile is written through `config::write_private`, owner-only on Unix.
- Tokens never reach stdout, stderr or a log.
- Scopes stay at `User.Read Chat.ReadWrite offline_access`. A new scope needs a reason in the commit.

## Common Commands

```bash
cargo run                                          # needs a client ID in config.json or CLIENT_ID
cargo test
cargo +nightly fmt                                 # rustfmt.toml uses unstable options
cargo clippy --all-targets -- -D warnings          # what CI runs
```

## Linting

`rustfmt.toml` and `clippy.toml` are the [scadoshi-rust](https://github.com/scadoshi/scadoshi-rust) files. The scadoshi-rust `[lints]` set is not in `Cargo.toml` yet: against the inherited code it raises about 190 hits, 38 of them string or slice indexing that can panic on multi-byte text. Adopting it is the next cleanup, one category per commit, ending with the block pasted into `Cargo.toml`. New code follows the set already: no `unwrap`, `expect` or indexing outside tests, and a suppression is `#[expect(lint, reason = "...")]`, never `#[allow]`.

## Commit Guidelines

See `development/commit_guidelines.md`. The short version: one-line messages, no emojis, never any AI-agent signature or Co-Authored-By trailer. Never push without being asked.

## Comments

A comment says what the code does, in the present tense, and only when the code does not say it itself. No history, no "without this", no restating the name. The full rules with examples are in `development/comment_guidelines.md`.

## Markdown Conventions

- Do not hard-wrap prose. One paragraph is one line; one bullet is one line.
- Code fences, tables and indented blocks keep their own formatting.
- No em dashes. American English.
