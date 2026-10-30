# CLAUDE.md

Guidance for Claude Code and any other AI assistant working in this repository.

## Project Overview

Starling is a terminal client for Microsoft Teams chats, built on [ratatui](https://ratatui.rs) and crossterm. It started as a fork of [nospor/teams-tui](https://github.com/nospor/teams-tui) (MIT, deprecated upstream) and diverges from it freely. The name is the bird: starlings fly in coordinated flocks and mimic what they hear.

- One binary crate. `main.rs` sets up the terminal; `event_loop.rs` runs sign-in, loading and the chat screen, applying background results and keys; `app.rs` holds UI state and the chat order; `ui.rs` draws; `message.rs` turns message HTML into plain text; `api.rs` calls Microsoft Graph; `auth.rs` runs the device-code flow; `config.rs` owns the config folder.
- Event handlers never touch the network. They queue a `Request`, and `EventLoop::dispatch` spawns it. Tests drive the handlers and assert on the queue.
- The UI stays on ratatui. Don't swap the TUI framework. `development/ratatui.md` has the patterns and styling rules.
- Everything after launch happens in the TUI, sign-in included. Nothing prints to stdout.

## Security Rules

These are why the fork exists. Non-negotiable:

- Sign-in uses the OAuth device-code flow against the user's own Azure app registration (`AZURE_SETUP.md`). No borrowed first-party client IDs, no scraped session tokens, no unofficial Teams endpoints.
- Network traffic goes to `login.microsoftonline.com` and `graph.microsoft.com` only.
- Anything holding a token or profile is written through `config::write_private`, owner-only on Unix, inside a folder created owner-only.
- Text from the network goes through `message::sanitize` or `message::clean_text` before it is drawn, so it cannot carry a terminal escape sequence.
- Tokens never reach stdout, stderr or a log.
- Scopes stay at `User.Read Chat.ReadWrite offline_access`. A new scope needs a reason in the commit.

## Common Commands

```bash
cargo run                                          # asks for a client ID on first run
cargo audit                                        # what the weekly audit job runs
cargo test
cargo +nightly fmt                                 # rustfmt.toml uses unstable options
cargo clippy --all-targets -- -D warnings          # what CI runs
```

## Linting

The [scadoshi-rust](https://github.com/scadoshi/scadoshi-rust) set: `[lints]` in `Cargo.toml`, plus its `clippy.toml` and `rustfmt.toml`. Pedantic clippy is on, and nothing may panic outside tests: `unwrap`, `expect`, `panic`, indexing and string slicing are denied, so string handling uses `split_once`, `strip_prefix` and `get`. Arithmetic is saturating or checked. A suppression is `#[expect(lint, reason = "...")]`, never `#[allow]`. CI treats warnings as errors.

## Commit Guidelines

See `development/commit_guidelines.md`. The short version: one-line messages, no emojis, never any AI-agent signature or Co-Authored-By trailer. Never push without being asked.

## Comments

A comment says what the code does, in the present tense, and only when the code does not say it itself. No history, no "without this", no restating the name. The full rules with examples are in `development/comment_guidelines.md`.

## Markdown Conventions

- Do not hard-wrap prose. One paragraph is one line; one bullet is one line.
- Code fences, tables and indented blocks keep their own formatting.
- No em dashes. American English.
