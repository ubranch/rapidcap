#!/usr/bin/env bash
# The macOS counterpart to package.ps1: lays the release binary out as
# RapidCap.app.
#
# The bundle is not cosmetic here. macOS records the Screen Recording grant
# against a bundle identifier and its code signature, so a bare binary run from
# a terminal makes the *terminal* the grantee and every capture comes back
# empty.
#
# Ad-hoc signing is not enough to keep that grant. An ad-hoc signature has no
# certificate, so the requirement TCC stores is the code hash itself, and the
# hash changes with every build - the toggle in System Settings stays on while
# tccd quietly refuses with "Failed to match existing code requirement", and
# AVFoundation answers a refusal by blocking in avformat_open_input forever.
# Signing with any certificate, even a self-signed one, replaces that hash with
# `identifier "com.inspire.rapidcap" and certificate leaf = H"..."`, which
# survives rebuilds. Set RAPIDCAP_SIGN_IDENTITY to the name of a codesigning
# identity to get that; see scripts/macos-signing-identity.sh. Shipping to another machine
# still needs a Developer ID and notarisation.
#
# FFmpeg is not vendored. assets/ffmpeg holds audited *Windows* binaries, and
# ffmpeg_path() searches PATH and then the two Homebrew prefixes, because a
# bundle launched from Finder never sees the PATH a shell profile sets up.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"

rustup run 1.97.1 cargo build -p rapidcap-desktop --release --locked

app="$repo/dist/RapidCap.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$target/release/RapidCap" "$app/Contents/MacOS/RapidCap"
cp "$repo/crates/desktop/Info.plist" "$app/Contents/Info.plist"
cp "$repo/crates/desktop/assets/rapidcap.icns" "$app/Contents/Resources/rapidcap.icns"

# Signing has to come after everything is in place, because the signature
# covers the whole bundle. `-` is the ad-hoc identity, kept as the default so a
# checkout with no certificate still produces a runnable bundle.
identity="${RAPIDCAP_SIGN_IDENTITY:--}"
keychain="${RAPIDCAP_SIGN_KEYCHAIN:-}"
if [ -n "$keychain" ]; then
  codesign --force --sign "$identity" --keychain "$keychain" "$app"
else
  codesign --force --sign "$identity" "$app"
fi

"$app/Contents/MacOS/RapidCap" --probe > /dev/null

cd "$repo/dist"
find RapidCap.app -type f -exec shasum -a 256 {} + > SHA256SUMS.txt
echo "$app"
