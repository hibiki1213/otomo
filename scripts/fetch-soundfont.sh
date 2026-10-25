#!/bin/sh
# Fetch the General MIDI SoundFont the app ships with (GeneralUser GS v1.471)
# into src-tauri/resources/, where the bundle picks it up. It is 31 MB, so it
# is downloaded at build time rather than kept in git; the checksum pins the
# exact file. Does nothing once the files are in place.
set -eu
cd "$(dirname "$0")/.."

base=https://raw.githubusercontent.com/ad-si/GeneralUser/d637ee5c4145b0bb1b49903c5dfb3b564d171704
dir=src-tauri/resources
mkdir -p "$dir"

fetch() {
  name=$1 dest=$2 sum=$3
  if [ -f "$dir/$dest" ] && echo "$sum  $dir/$dest" | shasum -a 256 -c - >/dev/null 2>&1; then
    return
  fi
  echo "downloading $name"
  curl -fL --retry 3 -o "$dir/$dest.part" "$base/$name"
  if ! echo "$sum  $dir/$dest.part" | shasum -a 256 -c - >/dev/null; then
    rm -f "$dir/$dest.part"
    echo "checksum mismatch for $name" >&2
    exit 1
  fi
  mv "$dir/$dest.part" "$dir/$dest"
}

fetch GeneralUser.sf2 GeneralUser.sf2 f45b6b4a68b6bf3d792fcbb6d7de24dc701a0f89c5900a21ef3aaece993b839a
fetch LICENSE.txt GeneralUser-LICENSE.txt 1bcbb56622f2708d73b852b14d633d058b36f615b71fabe4453bfe7f9512a716
