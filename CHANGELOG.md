# Changelog

## Unreleased

Starling forks [nospor/teams-tui](https://github.com/nospor/teams-tui) at its last release, v0.7.1. That project's changelog covers the history before the fork.

- `O` signs out so you can sign in as another account, and `c` on the sign-in screen changes the client ID.
- Themes: `t` opens a picker with orangui's 50 themes in dark and light, previewing as you move. The terminal's own colors stay the default.
- Sign-in, loading and the client ID prompt happen in the TUI. `.env` files are no longer read; `CLIENT_ID` still overrides the saved client ID.
- Shift+Enter starts a new line in terminals with the kitty keyboard protocol; Alt+Enter works everywhere. Ctrl+C quits from any screen.
- Your own account is left out of chat member lists by user ID, so one-on-one chats are named after the other person.
- Token, profile and config files are readable by your user only, and the client ID is never borrowed from Microsoft's own apps.
- Message text is stripped of control characters, so a message cannot send escape sequences to your terminal.
- Times show in local time, text can be selected with the mouse, and a chat with no messages no longer shows "Loading messages..." forever.
