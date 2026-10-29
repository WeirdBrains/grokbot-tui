#!/usr/bin/env sh
# Build grokbot-tui from this checkout and install it. No `cargo install`, no
# crates.io install, no downloaded prebuilt binaries. Building still needs a Rust
# toolchain (cargo + rustc); crate sources come from Cargo.lock, or from ./vendor
# with --vendor/--offline so the build can run without network access.
#
#   ./install-local.sh [--prefix DIR] [--destdir DIR] [--ffmpeg] [--vendor]
#                      [--offline] [--no-build] [--uninstall]
#
# Default prefix: ~/.local for a normal user, /usr/local for root.
# Env equivalents: PREFIX, DESTDIR, FFMPEG=1, VENDOR=1, OFFLINE=1, NOBUILD=1.
set -eu
ROOT=$(cd "$(dirname "$0")" && pwd)
NAME=grokbot-tui
if [ "$(id -u)" = 0 ]; then def_prefix=/usr/local; else def_prefix="$HOME/.local"; fi
PREFIX=${PREFIX:-$def_prefix}
DESTDIR=${DESTDIR:-}
FFMPEG=${FFMPEG:-0}; VENDOR=${VENDOR:-0}; OFFLINE=${OFFLINE:-0}; NOBUILD=${NOBUILD:-0}
ACTION=install
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX=$2; shift ;;
    --prefix=*) PREFIX=${1#*=} ;;
    --destdir) DESTDIR=$2; shift ;;
    --destdir=*) DESTDIR=${1#*=} ;;
    --ffmpeg) FFMPEG=1 ;;
    --vendor) VENDOR=1 ;;
    --offline) OFFLINE=1 ;;
    --no-build) NOBUILD=1 ;;
    --build-only) ACTION=build ;;
    --uninstall) ACTION=uninstall ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

BINDIR="$DESTDIR$PREFIX/bin"
DOCDIR="$DESTDIR$PREFIX/share/doc/$NAME"
LICDIR="$DESTDIR$PREFIX/share/licenses/$NAME"
DATADIR="$DESTDIR$PREFIX/share/$NAME"
MANDIR="$DESTDIR$PREFIX/share/man/man1"
BASHDIR="$DESTDIR$PREFIX/share/bash-completion/completions"
ZSHDIR="$DESTDIR$PREFIX/share/zsh/site-functions"
FISHDIR="$DESTDIR$PREFIX/share/fish/vendor_completions.d"
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
BIN="$TARGET_DIR/release/$NAME"

build() {
  command -v cargo >/dev/null 2>&1 || {
    echo "cargo not found: building from source needs a Rust toolchain (https://rustup.rs or your distro's rust package)." >&2
    exit 1; }
  set -- build --release --locked -p "$NAME"
  [ "$FFMPEG" = 1 ] && set -- "$@" --features ffmpeg-static
  if [ "$VENDOR" = 1 ] && [ ! -d "$ROOT/vendor" ]; then
    (cd "$ROOT" && cargo vendor --locked vendor >/dev/null)
  fi
  if [ -d "$ROOT/vendor" ] && { [ "$VENDOR" = 1 ] || [ "$OFFLINE" = 1 ]; }; then
    set -- "$@" --offline \
      --config 'source.crates-io.replace-with="vendored-sources"' \
      --config "source.vendored-sources.directory=\"$ROOT/vendor\""
  elif [ "$OFFLINE" = 1 ]; then
    set -- "$@" --offline
  fi
  (cd "$ROOT" && cargo "$@")
}

inst() { # mode src dest-dir [dest-name]
  [ -f "$2" ] || return 0
  mkdir -p "$3"
  cp "$2" "$3/${4:-$(basename "$2")}"
  chmod "$1" "$3/${4:-$(basename "$2")}"
  echo "  $3/${4:-$(basename "$2")}"
}

install_files() {
  [ -x "$BIN" ] || { echo "no binary at $BIN; build first (drop --no-build)" >&2; exit 1; }
  echo "Installing $NAME into ${DESTDIR}${PREFIX}:"
  inst 755 "$BIN" "$BINDIR"
  inst 644 "$ROOT/README.md" "$DOCDIR"
  inst 644 "$ROOT/CHANGELOG.md" "$DOCDIR"
  inst 644 "$ROOT/THIRD_PARTY.md" "$DOCDIR"
  inst 644 "$ROOT/LICENSE" "$LICDIR"
  for f in "$ROOT"/themes/*.toml; do inst 644 "$f" "$DATADIR/themes"; done
  inst 644 "$ROOT/keymap/default.toml" "$DATADIR/keymap"
  inst 644 "$ROOT/notify-rules/default.toml" "$DATADIR/notify-rules"
  for f in "$ROOT"/packaging/man/*.1; do inst 644 "$f" "$MANDIR"; done
  inst 644 "$ROOT/packaging/completions/$NAME.bash" "$BASHDIR" "$NAME"
  inst 644 "$ROOT/packaging/completions/_$NAME" "$ZSHDIR"
  inst 644 "$ROOT/packaging/completions/$NAME.fish" "$FISHDIR"
  case ":$PATH:" in *":$PREFIX/bin:"*) ;; *)
    [ -z "$DESTDIR" ] && echo "Note: $PREFIX/bin is not on PATH; add it: export PATH=\"$PREFIX/bin:\$PATH\"" ;;
  esac
}

uninstall() {
  echo "Removing $NAME from ${DESTDIR}${PREFIX}"
  rm -f "$BINDIR/$NAME" "$MANDIR/$NAME.1" "$BASHDIR/$NAME" "$ZSHDIR/_$NAME" "$FISHDIR/$NAME.fish"
  rm -rf "$DOCDIR" "$LICDIR" "$DATADIR"
  for d in "$BINDIR" "$MANDIR" "$BASHDIR" "$ZSHDIR" "$FISHDIR" "$DESTDIR$PREFIX/share/doc" \
           "$DESTDIR$PREFIX/share/licenses" "$DESTDIR$PREFIX/share"; do
    rmdir "$d" 2>/dev/null || true  # only if now empty
  done
  # Your settings and history (~/.config, ~/.local/share/grokbot-tui data) are not touched.
}

case "$ACTION" in
  build) build ;;
  install) [ "$NOBUILD" = 1 ] || build; install_files ;;
  uninstall) uninstall ;;
esac
