# Logging in and accounts

Signing in the same way the Grok Bot desktop app does: with your **Cursor account session**
(saved under the app's data dir), not with dashboard API keys (`grok_` / `crsr_` / `xai-`).

## Authentication

All methods are per account and stored in SQLite (see below).

1. **Gateway token** — `/login gateway <url>`, then paste the token. grokbot-tui checks the URL against the allowlist,
   then validates the credential with `listAgents` (the gateway has no "who am I" call). It is saved only if that succeeds.
2. **Cursor access token** — `/login cursor`, then paste the token. grokbot-tui calls
   `POST https://api2.cursor.sh/aiserver.v1.GrokBotService/EnsureSandBox` to get a gateway URL, token and
   extra headers (e.g. `x-anyrun-network-token`). The Cursor token is stored too, so when the gateway answers
   **401** grokbot-tui re-exchanges once, retries, and saves the new session.
   Dashboard / inference API keys (`grok_…`, `crsr_…`, `xai-…`) are rejected locally
   before EnsureSandBox: they authenticate Cursor Admin/Cloud Agents or xAI inference,
   not the Grok Bot companion gateway. Prefer `/login desktop`.

3. **Desktop app Cursor session (recommended)** — Sign in with Cursor in the Grok Bot desktop app
   (browser), then `/login desktop [entry]` (or bare `/login` when session files exist).
   `/login desktop open` launches the app when installed. grokbot-tui reads the desktop app's
   saved session only when you run this command, mirroring grok-bot-cli's `app-session.js`.
   It prefers `gateway-descriptor.json` from the app data dir:
   - Linux: `$XDG_CONFIG_HOME/Grok Bot` or `~/.config/Grok Bot`;
   - macOS: `~/Library/Application Support/Grok Bot`;
   - Windows: `%APPDATA%\Grok Bot` (Roaming). If that folder has no session files,
     grokbot-tui also checks `%LOCALAPPDATA%\Grok Bot`.

   Both descriptor formats are supported: v1 `{version, encrypted}` and v2 `{version: 2, entries}`. When a v2 file has
   several entries, grokbot-tui lists them and you choose one with `/login desktop <entry>`. The payload is Chromium Safe Storage:
   - Linux `v11`: the password comes from `secret-tool lookup xdg:schema chrome_libsecret_os_crypt_password_v2 application 'Grok Bot'`;
   - Linux `v10`: the fixed `peanuts` password;
   - macOS: Keychain item `Grok Bot Safe Storage`, PBKDF2-SHA1 (`saltysalt`, 1003 rounds) and AES-128-CBC;
   - Windows: the DPAPI-protected key in `Local State` and AES-256-GCM.

   The decrypted `{baseUrl, token, headers?}` must pass the host allowlist and is validated with `listAgents`.
   It is then saved to the active account with method `desktop`. If the gateway later answers **401**, grokbot-tui re-reads the
   desktop session once automatically, for that account only. grokbot-tui never reads the desktop session in any other code path.
   You get clear errors when the app isn't installed, you aren't signed in, or `secret-tool` is missing. Tokens are never printed or logged.

   **Fallback when there is no `gateway-descriptor.json`:** some desktop builds only write `sand-secrets.json`.
   In that case `/login desktop` decrypts the `cursor-accounts` field (Chromium Safe Storage, same scheme as above),
   takes the Cursor access token, and exchanges it via EnsureSandBox exactly like `/login cursor`. The Cursor token
   itself is **not** stored — method stays `desktop` and a 401 re-reads `sand-secrets.json`. With several Cursor
   accounts, grokbot-tui lists them as `1) <keyprefix>…` and you pick with `/login desktop <number>` (tip: `/account add`
   first so each maps to its own grokbot-tui account). `local-exec-file-key`, `cursor-machine-id`, Cookies and `~/.grokbot/`
   are never read.

**Host allowlist.** Credentials are only sent to:
- gateway: `https` on `cursor.sh`, `cursor.com`, `*.cursor.sh`, `*.cursor.com`, `*.cursorvm.com`;
- EnsureSandBox backend (gets the Cursor token): `https` on `*.cursor.sh` / `*.cursor.com` only, never `*.cursorvm.com`.

Loopback gateways (`localhost`, `127.0.0.1`, `::1`) are allowed only with `GROKBOT_ALLOW_LOCAL_GATEWAY=1`. URLs containing userinfo are always rejected.
Redirects are never followed, requests time out after 30 s, responses are capped at 2 MiB, and tokens are redacted from error messages.

**Environment** (each applies only to the account selected at startup: `--account` / `GROKBOT_TUI_ACCOUNT` / last active):

| Variable | Meaning |
|---|---|
| `GROKBOT_TUI_TOKEN` | **Gateway token** override; never saved. After `/account use` switches away, that account's stored login is used |
| `GROKBOT_TUI_GATEWAY_URL` | Gateway URL for the token above (else the account's stored gateway URL) |
| `GROKBOT_TUI_GATEWAY_HEADERS` | Optional JSON object of extra gateway headers |
| `GROKBOT_TUI_CURSOR_API_BASE_URL` | Override the EnsureSandBox backend (still must be `*.cursor.sh` / `*.cursor.com`) |
| `GROKBOT_TUI_CURSOR_CLIENT_VERSION` | Override `x-cursor-client-version` (else pref `cursor.client_version`, else detect installed Grok Bot, else `0.70.2`) |
| `GROKBOT_TUI_CURSOR_CLIENT_TYPE` | Override `x-cursor-client-type` (default `sand`) |
| `GROKBOT_TUI_SAND_BOX_NAMESPACE` | Override `x-sand-box-namespace` (default `prod`) |
| `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` | `1` to permit loopback gateways (development) |
| `GROKBOT_TUI_MAX_RESPONSE_BYTES` | Cap on one gateway response body in bytes (default 64 MiB) |
| `GROKBOT_TUI_APP_DIR` | Grok Bot desktop app install directory, for client version detection |
| `GROKBOT_TUI_NO_THEME_DOWNLOAD` | `1` = never download the default theme (see [Themes](./themes.md#themes)) |
| `GROKBOT_TUI_THEME_URL` | Download themes from this URL base instead of the release tag (`<url><name>.toml`; used by the tests) |
| `GROKBOT_TUI_ACCOUNT` | Starting account alias |

`GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` and `GROKBOT_TUI_APP_DIR` are forwarded at startup to
`GROKBOT_ALLOW_LOCAL_GATEWAY` and `GROKBOT_APP_DIR`, the names the `grokbot_api` library reads (setting the
library names directly also works).

**Older names.** Before 1.2.0 these variables started with `GB_` (e.g. `GB_GROKBOT_TOKEN`,
`GB_ACCOUNT`). Every `GB_*` name still works as a fallback: at startup grokbot-tui copies its value to the
matching `GROKBOT_TUI_*` name when that one isn't set, so the new name wins when both are set.

## Accounts

Several **strictly separate accounts** (e.g. `personal` plus work accounts). Each has its own alias, login
(gateway URL, token, headers, method, Cursor token), bot list, chat transcripts, per-bot memory, and account preferences.

- Start-up account: `--account <alias>`, then `GROKBOT_TUI_ACCOUNT`, then the last active account, then `default`.
- Switch with `/account use <alias>` or click the `@alias` header.
- Removing an account deletes all of its rows (FK `ON DELETE CASCADE`).

## Send vs login

`/login desktop` uses the same session class the desktop app stores (gateway-descriptor or
EnsureSandBox from sand-secrets). If `listAgents` works but `sendPrompt` returns
**502 A Sandbox session is not permitted on this endpoint**, that is a server-side restriction
on that session class — repeating `/login desktop` will not mint a different kind of session.
Check whether the desktop app can send to the same bot; see `/help troubleshooting`.
