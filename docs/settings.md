# Settings

The settings window and preferences.

## Settings (1.33.0)

Click **[⚙ Settings]** in the header, press **Alt-,** (Ctrl-, with the kitty keyboard
protocol), run `/settings [general|computer|vault|usage|updates|bot|local]`, or in vim mode
press `s` (keymap action `settings.open`). Esc goes back.

| Tab | Rows | How they work |
|---|---|---|
| General | account, sign out, theme, accent, language, spell check, microphone, hardware acceleration, network debugger, notification sound, time zone, auto-review, auto-review rules, security keys | Account opens the account switcher; sign out asks y/n. The rest have no gateway API: *Open in Grok Bot →* opens `grokbot://app/v1/settings?id=<row>` |
| Computer | local execution, computers, update computer, reset computer | Open in Grok Bot (reset asks y/n first) |
| Vault | vault | Open in Grok Bot |
| Usage & Billing | usage, plan, cancel trial, on-demand, billing | Open in Grok Bot (cancel trial asks y/n first) |
| Updates | update status, channel, automatic updates; grokbot-tui version | App rows open in Grok Bot; the version links to GitHub releases |
| Bot | name, title, description, avatar shape/color, notify on updates, hidden from sidebar; notifications, voice, harness (read-only) | Read from `listAgents`, saved with `updateAgent`, `setAgentNotifyOnUpdates`, `setAgentHiddenFromSidebar` |
| Local (grokbot-tui) | links.open_with, appearance, theme, header, sidebar, files panel, mouse, spinner, key scheme, … | grokbot-tui's own prefs on this computer, not synced |

Every link (chat, Settings, attachments) goes through `links.open_with`: the built-in text
browser for web pages, or the system browser. Non-web links such as `grokbot://` always use
the system opener (in SSH it asks first). Destructive account actions are never run from
the TUI.
