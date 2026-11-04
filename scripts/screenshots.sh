#!/usr/bin/env bash
# Website screenshots, redone when a view changes (and each release that changes the look).
#
# 1. The ignored test `app::screenshots::site_screenshots` drives the TUI with the offline
#    mock API in a temp data dir (never ~/.grokbot-tui, never a real account) and writes
#    each screen as HTML to target/screenshots/, with list.tsv (name, caption).
# 2. Headless Chromium renders each page to PNG (no window is ever opened) and
#    ImageMagick shrinks it into site/screenshots/.
# 3. site/shots.tsv (the site's list) is copied from list.tsv.
#
# Needs: cargo, chromium, magick (ImageMagick).
set -euo pipefail
cd "$(dirname "$0")/.."
out=site/screenshots
cargo test -q site_screenshots -- --ignored >/dev/null
mkdir -p "$out"
rm -f "$out"/*.png
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT
while IFS=$'\t' read -r name caption; do
  timeout 120 chromium --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
    --user-data-dir="$profile" --window-size=1524,864 \
    --screenshot="target/screenshots/$name.png" "file://$PWD/target/screenshots/$name.html" >/dev/null 2>&1
  magick "target/screenshots/$name.png" -strip -colors 256 "PNG8:$out/$name.png"
done < target/screenshots/list.tsv
cp target/screenshots/list.tsv site/shots.tsv
echo "$(wc -l < site/shots.tsv) screenshots in $out"
