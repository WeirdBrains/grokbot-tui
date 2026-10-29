#!/usr/bin/env sh
# grokbot-tui @VERSION@ setup: pick a build, then install it.
#
#   standard  pure-Rust media (GIF, H.264 MP4 via the bundled OpenH264, MP3/AAC/FLAC/
#             Vorbis/WAV); other video codecs use a system ffmpeg if you have one.
#   ffmpeg    the same plus FFmpeg linked in: every common codec, nothing to install.
#             Larger download.
#
# Both are GPL-3.0-or-later; see THIRD_PARTY.md (incl. the H.264 patent note).
#
# Usage: curl -LsSf <release>/grokbot-tui-setup.sh | sh                 (asks)
#        ... | sh -s -- --variant ffmpeg   |   --variant standard        (no prompt)
set -eu
VERSION="@VERSION@"
BASE="https://github.com/WeirdBrains/grokbot-tui/releases/download/v$VERSION"
variant=""
while [ $# -gt 0 ]; do
  case "$1" in
    --variant) variant="${2:-}"; shift 2 ;;
    --variant=*) variant="${1#*=}"; shift ;;
    --ffmpeg) variant=ffmpeg; shift ;;
    --standard) variant=standard; shift ;;
    -h|--help) sed -n '2,13p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done
if [ -z "$variant" ]; then
  if [ -r /dev/tty ]; then
    printf 'grokbot-tui %s: which build?\n  1) standard (smaller; common video via the built-in decoder)\n  2) ffmpeg   (FFmpeg compiled in: plays nearly any video)\nChoice [1]: ' "$VERSION" > /dev/tty
    read -r ans < /dev/tty || ans=""
    case "$ans" in 2|f|ffmpeg) variant=ffmpeg ;; *) variant=standard ;; esac
  else
    variant=standard
  fi
fi
fetch() { # url dest
  if command -v curl >/dev/null 2>&1; then curl -fLsS "$1" -o "$2"; else wget -qO "$2" "$1"; fi
}
if [ "$variant" = standard ]; then
  tmp=$(mktemp)
  fetch "$BASE/grokbot-tui-installer.sh" "$tmp"
  sh "$tmp"
  rm -f "$tmp"
  exit 0
fi
[ "$variant" = ffmpeg ] || { echo "variant must be standard or ffmpeg" >&2; exit 2; }
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64|Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *) echo "no -ffmpeg build for $(uname -s) $(uname -m); installing standard" >&2
     exec sh "$0" --variant standard ;;
esac
name="grokbot-tui-ffmpeg-$target"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
if ! fetch "$BASE/$name.tar.xz" "$dir/$name.tar.xz" 2>/dev/null; then
  echo "this release has no -ffmpeg build for $target; installing standard" >&2
  fetch "$BASE/grokbot-tui-installer.sh" "$dir/i.sh"
  sh "$dir/i.sh"
  exit 0
fi
fetch "$BASE/$name.tar.xz.sha256" "$dir/sum"
want=$(cut -d' ' -f1 < "$dir/sum")
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$dir/$name.tar.xz" | cut -d' ' -f1)
else got=$(shasum -a 256 "$dir/$name.tar.xz" | cut -d' ' -f1); fi
[ "$want" = "$got" ] || { echo "checksum mismatch for $name.tar.xz" >&2; exit 1; }
tar -xJf "$dir/$name.tar.xz" -C "$dir"
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$bin"
install -m 755 "$dir/$name/grokbot-tui" "$bin/grokbot-tui"
doc="${XDG_DATA_HOME:-$HOME/.local/share}/grokbot-tui"
mkdir -p "$doc"
cp "$dir/$name/LICENSE" "$dir/$name/THIRD_PARTY.md" "$doc/"
echo "Installed grokbot-tui $VERSION (ffmpeg build) to $bin/grokbot-tui"
case ":$PATH:" in *":$bin:"*) ;; *) echo "Add $bin to your PATH." ;; esac
