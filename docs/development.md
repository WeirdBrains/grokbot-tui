# Development

Building, testing and the in-app help.

## Development

```sh
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests never contact real servers. They use temp directories and local `wiremock` servers, and loopback is enabled only through `Policy::local_testing()`.

## In-app help

Press **F1** (or **?** on an empty input) or type `/help`. Help opens in a scrollable
popup and is never written to chat history. Topics: `login`, `login desktop`
(recommended), `login cursor`, `login gateway`, `keys`, `accounts`, `prefs`, `env`,
`sync`, `delivery`, `troubleshooting`, plus `/help <command>`. Each page lists related
topics; Tab/←→ selects one, Enter opens it, Backspace goes back, Esc closes.
`/help <keyword>` (or `/` inside the help window) searches every topic and command
page and lists matches with a snippet; pick one with Tab/←→ and Enter. A bare
`/login` opens the login help. Mistyped commands get a "did you mean" suggestion.
