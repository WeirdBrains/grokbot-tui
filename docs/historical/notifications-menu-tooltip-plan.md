# Notifications, menu bar, and tooltips (1.10.0)

## Sources (acp-rs-scraper)

Repo: https://github.com/m3talsmith/acp-rs-scraper  
Commit: `a60c82a5e721e69a9c5ac5b08661f925f7fadb94`

| System | Path |
|---|---|
| Notifications | `src/notify.rs`, PLAN-3.0-CLIENT.md §7.14.13 (3.31.0) |
| Rules / alerts | `src/rules/mod.rs`, §7.14.13 (3.32.0–3.33.0) |
| Menu bar | `src/tui/widgets/menu.rs`, `MenuCommand` in `src/tui/commands.rs` |
| Tooltips | `src/tui/tooltip.rs` |

## Gist implemented

1. **Notifications:** kinds, DND, per-kind mute, toast TTL, log retained when muted.
   Rules file decides toast / mute / drop (keyword + kind). Overlay toasts; **no chat
   scroll change** (1.9.1).
2. **Menu bar:** top row of categories → existing slash commands; click + F10/keyboard.
3. **Tooltips:** hover delay, themed bubble, Help topic; motion coalesced.

## Deviations

1. Rules are a small TOML in `~/.grokbot-tui/notify-rules.toml`, not SQLCipher tables /
   sync triggers / bookmark actions. Chat has no “post synced” event — kinds are
   error/warning/info/message.
2. No desktop `notify-send` / osascript yet (board is in-terminal only).
3. Menu items are flat command lists, not scrape option widgets / Start Scanning.
4. Tooltips cover menu, theme/account switch, unread badge — not every Red Star Hit.
5. No Preferences › Notifications screen; use `/notify`.

## Lenin sync notes

- Theme parts: `toast`, `toast_warn`, `toast_error`, `menu_bar`, `menu_active`,
  `tooltip`, plus prior `unread_badge` / vim `mode_*`.
- Keymap action `input.newline` for remappable newlines.
- Notify-rules TOML shape (`[[rule]]` + `action = toast|mute|drop`) if Red Star wants
  a file-based preview path beside SQL rules.
