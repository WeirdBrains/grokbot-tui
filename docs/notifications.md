# Notifications

Grok Bot notifications, the notification menu, toasts and rules.

## Grok Bot notifications (1.34.0)

The header **🔔** shows how many Grok Bot notifications are unread. Click it, press
**Alt-b**, run `/notifications`, or press `N` in vim mode to open the page.

- **Where they come from:** Grok Bot system notices in a chat (`kind: notice`), new
  messages and "waiting for you" in bots you aren't viewing (from the bot list refresh),
  and pending cards. The gateway has no notifications or usage API, so **low credits /
  usage** notifications are derived from notice or message text that mentions running out
  of usage or credits; they open Settings → Usage & Billing.
- **Toasts** appear as small boxes in the top-right corner. Click one to open its context
  (the chat, the card, or Settings) and mark it read. **[x]** or the timeout
  (`notifications.toast_timeout_secs`, default 20) only hides the toast: it stays unread.
- **Toast keys (1.35.0):** **Alt-x** closes the newest toast and **Alt-Shift-x** closes all
  of them; like the `[x]`, they stay unread. **Alt-o** opens the newest one and marks it
  read, like clicking it. These are plain Alt keys, so they work over SSH and tmux without
  the kitty protocol. Rebind them with the keymap actions `notifications.dismiss_toast`,
  `notifications.dismiss_all_toasts` and `notifications.open_toast`.
- **Page:** newest first. Enter opens, `m` toggles read, `a` marks all read, `c` or
  **[Clear all]** clears (asks first). `/notifications read` and `/notifications clear` do
  the same from the prompt. Read state and clearing are stored on this computer (per
  account); the API has no way to clear on the server.

## Notifications, menu bar, tooltips

Toasts honour `~/.grokbot-tui/notify-rules.toml` and `/notify` (dnd / per-kind). They never
jump the chat scroll. **F10** opens the menu bar. Hover shows a short tooltip after a delay.
