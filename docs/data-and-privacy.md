# Data directory and privacy

What grokbot-tui stores, where, and what it never stores or sends.

## Storage

SQLite at `~/.grokbot-tui/context.db` (directory `0700`, file `0600`; override with `--data-dir`), via `rusqlite` with bundled SQLite. Schema version is tracked in `PRAGMA user_version`.

**Moving from `~/.gb`.** Before 1.2.0 the data lived in `~/.gb`. On startup (unless `--data-dir` is given):

- If `~/.grokbot-tui` doesn't exist and `~/.gb` does, `~/.gb` is renamed to `~/.grokbot-tui`.
- If that rename fails (e.g. the two are on different filesystems), the folder is copied, every file is
  verified, and `~/.gb` is left in place with a notice. If even the copy fails, grokbot-tui keeps using `~/.gb`.
- If both exist, `~/.grokbot-tui` is used and a one-time notice says `~/.gb` was ignored.

User data is never deleted; remove `~/.gb` yourself once you no longer need it.

| Table | Key | Columns |
|---|---|---|
| `accounts` | `id` | `alias` (unique, case-insensitive), `created_at` |
| `credentials` | `account_id` → accounts | `method` (`gateway`/`cursor`/`desktop`), `gateway_url`, `gateway_token`, `gateway_headers` (JSON), `cursor_token`, `desktop_entry`, `created_at`, `updated_at` |
| `bots` | `(account_id, bot_id)` → accounts | `name`, `description`, `hidden`, `is_group` |
| `messages` | `id`; `(account_id, bot_id)` → bots | `role`, `text`, `remote_id` (transcript id or `pending:<nonce>`, unique per account+bot), `created_at` |
| `memory` | `(account_id, bot_id, key)` → bots | `value`, `updated_at` |
| `account_prefs` | `(account_id, key)` → accounts | `value` |
| `global_prefs` | `key` | `value` |

**Migrations:** v1→v2 moved single-account data into account `default`. v2→v3 replaces the credentials table with the gateway schema.
The old placeholder-API tokens are **dropped** because they never pointed at a real server, so log in again. v2→v3 also adds `messages.remote_id` and the bot flags, and deletes the dead `api_base_url`, `request_timeout_secs` and `paths.*` prefs.
v3→v4 allows method `desktop` and adds `credentials.desktop_entry`, keeping existing logins.
The earlier one-time import from `~/.grokbot-tui` was removed, since everything it imported is now dead.

**Preferences:** account-scoped `default_bot`, `active_bot` (automatic), `show_hidden_bots` (`false`); global `ui.sidebar_width` (`24`), `ui.theme` (`default`), `ui.theme_download` (`true`), `ui.theme_mode` (`dark`/`light`/`system`, default `system`), `active_account`.

## Chat feed and the database

SQLite (`~/.grokbot-tui/context.db`) is the single source of truth for the chat feed. The body
is always rebuilt from `store.messages(bot)`: polled messages and bot replies are merged
into the DB first, and your own message is written as a pending row and the feed is
rebuilt immediately (so it still appears instantly). Notices — command echoes, info,
errors, delivery warnings — are transient: they are kept in memory per bot, shown in a
separate "notices (not saved)" section below the chat, and never written to the DB.

## grokbot_api

The gateway client is published separately as [`grokbot_api`](https://crates.io/crates/grokbot_api)
([source](https://github.com/WeirdBrains/grokbot_api), [docs](https://docs.rs/grokbot_api)). It covers every gateway method grok-bot-cli uses: listAgents, sendPrompt,
getAgentTranscriptTail, getAgentThread, createAgent, updateAgent, deleteAgent, setAgentNotifyOnUpdates,
setAgentHiddenFromSidebar, createGroup, setGroupMembers, getAgentWorkflows, importAgentWorkflowText,
deleteAgentWorkflow, resolveLocalToolPermission and resolveAutoReviewApproval. It also covers EnsureSandBox and the URL policy.
