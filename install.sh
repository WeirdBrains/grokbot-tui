#!/usr/bin/env sh
# Kept for old instructions: installs from this checkout via install-local.sh
# (default prefix ~/.local, no `cargo install`). See `./install-local.sh --help`.
exec "$(dirname "$0")/install-local.sh" "$@"
