#!/bin/sh
# Render src-tauri/dmg/background.html into the DMG window background. Finder
# picks the matching resolution out of a multi-resolution TIFF, so the image is
# rendered at 1x and 2x and combined. Needs Google Chrome; the result is
# committed, so this only has to be rerun after editing the HTML.
set -eu
cd "$(dirname "$0")/.."

chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
dir=src-tauri/dmg
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for scale in 1 2; do
  "$chrome" --headless=new --disable-gpu --hide-scrollbars \
    --window-size=660,400 --force-device-scale-factor=$scale \
    --screenshot="$tmp/bg@$scale.png" "file://$PWD/$dir/background.html" 2>/dev/null
done

tiffutil -cathidpicheck "$tmp/bg@1.png" "$tmp/bg@2.png" -out "$dir/background.tiff"
echo "wrote $dir/background.tiff"
