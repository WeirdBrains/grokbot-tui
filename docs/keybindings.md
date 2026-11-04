# Keybindings, commands and vim mode

Default keys, slash commands, custom keymaps and the opt-in vim-like keys. The full key list is in the in-app help (`/help keys`, also in the [help reference](reference/help.md#keys)).

## Vim-like keys (opt-in)

`/keymap vim` (or pref `ui.keys=vim`) enables NORMAL / INSERT / COMMAND / VISUAL modes
with a mode badge on the input. Bindings live in `~/.grokbot-tui/keymap.toml`
(`/keymap reload` after editing). Default stays classic. See `docs/vim-mode-plan.md`.

## Commands

| Command | Description |
|---|---|
| `/help [cmd]` (`/h`, `/?`) | List commands or show one |
| `/login gateway [url]` | Log in the active account with a gateway URL plus gateway token (token prompt is masked) |
| `/login cursor` | Log in with a Cursor access token (masked), exchanged via EnsureSandBox |
| `/login desktop [entry]` | Opt-in: log in with the Grok Bot desktop app's saved session (see Authentication) |
| `/logout` | Delete this account's stored credential and clear the session. Chats and memory are kept. (The gateway has no revoke call.) |
| `/whoami` | Show the account's login details (method, gateway, header names, whether a Cursor token is stored, env override) |
| `/search <words>` (`/find`) | Search all locally cached messages and attachments across bots (also Ctrl-F); Enter jumps to the message |
| `/bots` | Refresh and list all bots, including hidden ones and groups |
| `/bot <name\|id>` (`/use`) | Switch to a bot's chat |
| `/open [n\|list]` (`/img`) | Open the newest image (then link) from the bot's replies in the system viewer (`xdg-open`, `open`, or the Windows URL handler; vim `o`). `list` numbers them |
| `/react <emoji\|:code:> [n]` | Toggle a reaction pill on the newest bot message (`n`=2 the one before; vim `r`). Local only: the Grok Bot API has no reaction endpoint |
| `/history [query]` (`/prompts`) | Browse and fuzzy-search your saved prompts; Enter puts one in the input. `/history clear` forgets them (confirmed) |
| `/reload` (`/transcript`) | Reload the active bot's full transcript from the gateway |
| `/account` (`/acct`) | List accounts (`*` = active) |
| `/account add <alias>` | Create an account |
| `/account use <alias>` | Switch account: reloads login, bots, chats, top bar |
| `/account rename <old> <new>` | Rename an account |
| `/account remove <alias>` | Delete an account and all its data; you confirm by typing the alias (the active account can't be removed) |
| `/prefs` (`/pref`) | List preferences; `/prefs get <key>`, `/prefs set <key> <value>` |
| `/theme [dark\|light\|system]` (`/mode`) | Switch dark/light mode; bare `/theme` cycles (also `F4` or a header click) |
| `/theme use [name]` | Switch theme (also `/theme <name>`); no name: pick list (also `F5`) |
| `/theme list` · `/theme reload` | List theme files (`●` = in use; broken files say why) · re-read them after editing by hand |
| `/theme edit [name]` | Open the theme editor (also `Shift-F5`) |
| `/theme init [name\|all]` | Download a shipped theme from the release (else write the minimal starter; never overwrites) |
| `/theme starter [name]` | Write a minimal starter theme (no download; never overwrites) |
| `/theme delete <name>` | Delete a theme file (confirm in a pick list) |
| `/theme export <name> [path]` | Copy a theme file out (default `<data dir>/exports/<name>-<UTC time>.toml`; never overwrites) |
| `/memory` (`/mem`) | The active bot's local memory notes; `/memory add <key> <value>`, `/memory clear [key]` |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `src/commands.rs`.
