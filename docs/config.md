# Configuration and environment variables

grokbot-tui keeps its settings as preferences in `context.db` (change them in the Settings
window, with `/prefs`, or see [Settings](settings.md)). Command-line flags and environment
variables override them for one run. The full flag list is the man page
([`man/grokbot-tui.1`](../man/grokbot-tui.1), `man ./man/grokbot-tui.1`), generated from
the same definitions as `--help`.

## Command-line flags

| flag | what |
|------|------|
| `--mock` | Offline mock API: fake bots and echo replies (any token except `bad` logs in) |
| `--account <ALIAS>` | Start in this account (also `GROKBOT_TUI_ACCOUNT`) |
| `--data-dir <DIR>` | Data directory holding `context.db` (default `~/.grokbot-tui`) |
| `--theme <NAME>` | Use this theme for this run only |
| `--list-themes` | List the theme files and exit |
| `mcp serve [--stdio\|--http [ADDR]]`, `mcp token`, `mcp config` | The [MCP server](mcp.md) |
| `-h`, `--help` / `-V`, `--version` | Help and version |

## Environment variables

| variable | what |
|----------|------|
| `GROKBOT_TUI_TOKEN` | Gateway token override for the startup account (never saved). |
| `GROKBOT_TUI_GATEWAY_URL` | Gateway URL for that token (else the stored URL). |
| `GROKBOT_TUI_GATEWAY_HEADERS` | JSON object of extra gateway headers. |
| `GROKBOT_TUI_CURSOR_API_BASE_URL` | Cursor backend for the EnsureSandBox exchange (must be *.cursor.sh / *.cursor.com). |
| `GROKBOT_TUI_CURSOR_CLIENT_VERSION` | x-cursor-client-version (env > pref cursor.client_version > detected app > 0.70.2). |
| `GROKBOT_TUI_CURSOR_CLIENT_TYPE` | x-cursor-client-type (default sand). |
| `GROKBOT_TUI_SAND_BOX_NAMESPACE` | x-sand-box-namespace (default prod). |
| `GROKBOT_TUI_ACCOUNT` | Starting account alias (same as --account). |
| `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` | Set to 1 to allow http(s) loopback gateways (development). |
| `GROKBOT_TUI_APP_DIR` | Grok Bot desktop install dir, for version detection. |
| `GROKBOT_TUI_MAX_RESPONSE_BYTES` | Cap on one gateway response body in bytes (default 64 MiB). |
| `GROKBOT_TUI_NO_THEME_DOWNLOAD` | Set to 1 to never download the default theme. |
| `GROKBOT_TUI_THEME_URL` | Download themes from here instead of the release tag (<url><name>.toml). |
| `GROKBOT_TUI_NO_HYPERLINKS` | Set (to anything) to turn off OSC 8 hyperlinks. |
| `GROKBOT_TUI_FORCE_SSH` | 0 or 1: override SSH detection (links use the built-in browser over SSH). |
| `GROKBOT_TUI_FUZZBALL` | off: skip the Fuzzball capability query at startup. |
| `GROKBOT_TUI_IMAGES` | halfblocks: skip the terminal graphics query and draw images with half blocks. |

Every older `GB_*` name (for example `GB_GROKBOT_TOKEN`) still works when its
`GROKBOT_TUI_*` name is not set; the new name wins when both are set. The library's own
names `GROKBOT_ALLOW_LOCAL_GATEWAY` and `GROKBOT_APP_DIR` work too. The same list is in the
app: `/help env`.

`docs/config.md`, `/help env` and the man page are checked against the source by
`cargo test` (`envvars::tests`), so a new variable can't ship undocumented.

## Files

- Keymap: see [Keybindings](keybindings.md) (`keymap/` in the repo has examples).
- Notification rules: see [Notifications](notifications.md) (`notify-rules/` has examples).
- Themes: `<data dir>/themes/*.toml`, see [Themes](themes.md).
