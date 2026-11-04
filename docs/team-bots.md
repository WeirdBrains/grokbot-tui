# Team bots and groups

Besides your own bots, the sidebar lists:

- **Team bots**: bots someone on your team shared with you (e.g. a shared assistant),
  marked `·team`. They come from the Cursor backend's `ListGrokBotTeamAgents` and every
  call for them (send, history, delivery status) carries the *viewer session* that list
  hands out. grokbot-tui keeps that session per bot and re-binds it before each call.
- **Groups**: group chats (`is_group`). `/bots` refreshes the list and shows hidden bots
  and groups too.

## Requirements

Team bots need a Cursor token, so log in with `/login desktop` (recommended) or
`/login cursor`; a plain `/login gateway` login can list them but can't open them. If a
team bot shows in the list but its chat stays empty or sending fails, log in again with
one of those (see [Troubleshooting](troubleshooting.md)).

## What works

- Chatting, history, attachments, reactions and [search](search.md) work as with your
  own bots (search covers a team bot once its chat has been opened and synced).
- Their settings belong to the owner: the bot tab in [Settings](settings.md) is
  read-only for team bots.
- The [MCP server](mcp.md) lists and talks to team bots too.
