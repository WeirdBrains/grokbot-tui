#!/usr/bin/env bash
# Install grokbot-tui into CARGO_HOME/bin (or ~/.cargo/bin) and remind the shell
# to find it. Prefer the cargo-dist installer from a GitHub Release when one is
# available; this script wraps a local `cargo install --path .` for developers.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$BIN_DIR"
if command -v cargo >/dev/null 2>&1; then
  cargo install --path "$ROOT" --locked --force
else
  echo "cargo not found; install Rust from https://rustup.rs" >&2
  exit 1
fi
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo "Note: $BIN_DIR is not on PATH. Add it, then open a new shell."
    echo "  export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac
# zsh caches PATH lookups; rehash so `grokbot-tui` is found without a new shell.
if [ -n "${ZSH_VERSION:-}" ]; then
  rehash 2>/dev/null || true
elif [ -n "${BASH_VERSION:-}" ]; then
  hash -r 2>/dev/null || true
fi
echo "Installed: $(command -v grokbot-tui 2>/dev/null || echo "$BIN_DIR/grokbot-tui")"
echo "If your shell still says 'command not found', run: rehash   # zsh"
echo "                                            or: hash -r # bash"
