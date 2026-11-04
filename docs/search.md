# Searching messages

Local full-text search across every bot's messages and attachments.

## Searching messages (1.39.0)

**Ctrl-F** or `/search <words>` (alias `/find`; vim keymap action `search.open`, bound to
`ctrl-f`) searches everything cached locally for the active account, never the network:

- every bot's messages in `context.db` (team bots and groups too, once synced), via an
  SQLite FTS5 index (`messages_fts`, kept in sync by triggers, backfilled on upgrade);
- attachment names and paths;
- the text of files you send from this computer (UTF-8, no NUL bytes, ≤ 256 KiB), cached
  in `attachment_text` at send time. Images, binaries, larger files and files the bot sent
  are matched by name only.

Every word must match, case-insensitively, as a word prefix (`kube` finds Kubernetes).
Results are newest first (up to 200), each with the bot, age and time, a highlighted
snippet and `[📎 file …]` / `[📎 in …]` for attachment matches. ↑/↓, PgUp/PgDn and the
wheel move; **Enter** or a click opens that bot's chat, scrolls to the message and flashes
it; **Ctrl-O** (vim `o`) also opens the matched attachment; Esc closes (vim: `q`; `/` or
`i` edits the words).

Search never blocks the UI (1.39.1): queries run on a worker thread with their own
SQLite connection (WAL mode), debounced while you type, and older results are dropped
when a newer query is in flight. After an upgrade, existing messages are indexed in the
background in small batches; until that finishes the search box shows `indexing… N%`
and older messages appear when it is done. The backfill records its progress in
`context.db`, so it resumes where it stopped if you quit. The text of files you send is
read and cached on a worker thread too.
