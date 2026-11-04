# Troubleshooting

Common problems and fixes. The same list is in the app: `/help troubleshooting`.

## Troubleshooting

### `grokbot-tui: command not found` after install

The binary lands in `$CARGO_HOME/bin` (usually `~/.cargo/bin`). Put that directory on
`PATH`, then refresh the shell's command cache:

```bash
# zsh
rehash
# bash
hash -r
```

Optional (zsh): auto-rehash when a command is not found —

```zsh
zstyle ':completion:*' rehash true
```

Or run `make install` (or `./install-local.sh`) from a checkout; it tells you if the bin directory is not on PATH.
