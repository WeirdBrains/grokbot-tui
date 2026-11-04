# MCP server

Letting other agents (Claude Code, Cursor, …) talk to your Grok Bots through grokbot-tui.

## MCP server (1.31.0)

grokbot-tui can be an [MCP](https://modelcontextprotocol.io) server, so other agents
(Claude Code, Cursor, Codex, another grokbot-tui) can talk to your Grok Bots. It uses the
signed-in account from `~/.grokbot-tui` (pick one with `--account ALIAS`; the default is
the last one you used in the TUI).

```sh
grokbot-tui mcp serve                 # stdio (the client starts it), same as --stdio
grokbot-tui mcp serve --http          # streamable HTTP on 127.0.0.1:8765/mcp
grokbot-tui mcp serve --http 127.0.0.1:9000 --read-only
grokbot-tui mcp token                 # print the HTTP bearer token (--rotate for a new one)
grokbot-tui mcp config                # show the settings in use
```

**Tools:** `list_bots`, `get_bot`, `send_message` (text plus optional local files),
`read_messages`, `wait_for_reply` (waits for the bot's answer to a `send_message`),
`list_pending_cards`, `answer_card`, `list_attachments`, `download_attachment`.
`answer_card` handles every kind the TUI does: questions (`value`), approvals and local
tool permissions (`action: approve|deny`), secret requests (`value`, never echoed back),
forms (`values: {field_id: text}`), drafts (`action: send|discard`) and `action: dismiss`.
**Resources:** `grokbot://bots` (JSON) and `grokbot://bots/{id}/transcript` (plain text).

**HTTP** listens on loopback only and every request needs
`Authorization: Bearer <token>`. The token is made on first use and kept in
`<data dir>/mcp-http-token` (mode 0600); only `grokbot-tui mcp token` prints it.

**Settings** go in `<data dir>/mcp.toml` (see `docs/mcp/mcp.toml.example`):

```toml
[mcp.server]
allowed_bots = ["Grokbot-cli"]   # ids or names; empty = all bots
read_only = false                # refuse send_message / answer_card
http_addr = "127.0.0.1:8765"
audit = true                     # ~/.cache/grokbot-tui/mcp-audit.log
```

The audit log has one line per call (time, transport, tool, bot id, card kind/action,
ok/error), never message text, answers, secrets, tokens or download URLs.

**Connecting a client.** Claude Code:

```sh
claude mcp add grokbot -- grokbot-tui --account default mcp serve --stdio
# or HTTP (start `grokbot-tui mcp serve --http` first):
claude mcp add --transport http grokbot-http http://127.0.0.1:8765/mcp \
  --header "Authorization: Bearer $(grokbot-tui mcp token)"
```

Cursor (`~/.cursor/mcp.json`) and Claude Code (`.mcp.json`) examples are in
`docs/mcp/cursor.mcp.json` and `docs/mcp/claude-code.mcp.json`; the HTTP entries read the
token from `GROKBOT_TUI_MCP_TOKEN` (`export GROKBOT_TUI_MCP_TOKEN=$(grokbot-tui mcp token)`).
