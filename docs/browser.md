# Built-in browser and SSH

The text-mode web browser, link handling and running over SSH.

## Built-in browser and SSH

Links open in a built-in text browser by default (`links.open_with = "builtin"`; set it to
`"system"`, use `/links builtin|system|toggle` or the **Web** menu to change it). Ctrl/Alt-click
or vim NORMAL `x` uses the other opener. `/browse [url]` (alias `/web`, vim NORMAL `b`, menu
**Web → browse**) opens it directly; Tab completes URLs visited this session.

The browser is in-process and works the same on Linux, macOS and Windows: no JavaScript, no
cookies, redirects followed, 5 MiB / 20 s limits. HTML is rendered as styled text with numbered
`[n]` links; images become `[🖼 alt]` placeholders that open the image viewer; plain text,
markdown and JSON render as text; anything else offers download (`d`) or the system app (`o`).
Keys: `e`/Ctrl-L/`:` edit the URL, j/k arrows PgUp/PgDn g/G Ctrl-d/u and the wheel scroll,
Tab/Shift-Tab select links, Enter or a click follows, a number then Enter jumps, H/L or
Alt-←/→ go back/forward, `r` reloads, `/` `n` `N` search, `o` opens in the system browser,
`y` copies the URL, Esc/`q` closes.

OSC 8 hyperlinks that your terminal makes clickable are opened by the terminal itself, so they
always go to the system browser.

Over SSH (detected from `SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`, or a parent `sshd` on Linux;
`GROKBOT_TUI_FORCE_SSH=0|1` overrides) the header shows **SSH** and links always use the
built-in browser (`links.ssh_behavior = "builtin"`, the default; `"respect_pref"` follows
`links.open_with`). Anything that would launch an app on the remote machine (browser `o`,
`/open`, Ctrl-click, the image viewer's `o`) warns first and runs only if repeated within 5 s.
Settings rows that open the Grok Bot app (`grokbot://app/v1/settings?id=…`) show a built-in
browser page with the deep link instead (Grok Bot has no web settings URL) and copy the link
to your local clipboard over OSC 52, so you can open it where Grok Bot is installed.

Theme parts: `browser_frame`, `browser_url`, `browser_link_number`, `browser_heading`,
`browser_status`, `ssh_indicator`.
