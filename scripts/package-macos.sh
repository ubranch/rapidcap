#!/usr/bin/env bash
# The macOS counterpart to package.ps1: lays the release binary out as
# RapidCap.app.
#
# The bundle is not cosmetic here. macOS records the Screen Recording grant
# against a bundle identifier and its code signature, so a bare binary run from
# a terminal makes the *terminal* the grantee - every capture then comes back
# empty, or asks again on each rebuild. Ad-hoc signing is enough to keep the
# grant attached across rebuilds on one machine; shipping to another machine
# needs a Developer ID and notarisation, which this script deliberately leaves
# to whoever holds the certificate.
#
# FFmpeg is not vendored. assets/ffmpeg holds audited *Windows* binaries, and
# ffmpeg_path() already falls back to PATH, which is where Homebrew puts it.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"

rustup run 1.97.1 cargo build -p rapidcap-desktop --release --locked

app="$repo/dist/RapidCap.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"

cp "$target/release/RapidCap" "$app/Contents/MacOS/RapidCap"
cp "$repo/crates/desktop/Info.plist" "$app/Contents/Info.plist"

# `-` is the ad-hoc identity. Signing has to come after everything is in place,
# because the signature covers the whole bundle.
codesign --force --sign - "$app"

"$app/Contents/MacOS/RapidCap" --probe > /dev/null

cd "$repo/dist"
find RapidCap.app -type f -exec shasum -a 256 {} + > SHA256SUMS.txt
echo "$app"
