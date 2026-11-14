#!/usr/bin/env bash
# Builds the native macOS app: the Rust capture core (crates/ffi) as a static
# library, the SwiftUI app in macos/ linked against it, laid out as
# dist/RapidCap.app.
#
# Needs the macOS 26 SDK for Liquid Glass. The Command Line Tools are enough;
# Xcode is not required.
#
# The bundle is not cosmetic. macOS records the Screen Recording grant against
# a bundle identifier and its code signature, so a bare binary makes the
# terminal the grantee. An ad-hoc signature's requirement is the code hash,
# which changes every build; signing with any certificate, even a self-signed
# one, pins `identifier "com.inspire.rapidcap" and certificate leaf = H"..."`
# instead, which survives rebuilds. Set RAPIDCAP_SIGN_IDENTITY (and
# RAPIDCAP_SIGN_KEYCHAIN) for that; see scripts/macos-signing-identity.sh.
#
# FFmpeg ships inside the bundle, next to the app binary, where the core looks
# first (`ffmpeg_path`): a static arm64 build from martin-riedl.de, pinned by
# version and checksum, the same 9.0.2 the Windows package carries. It makes
# the GIFs, muxes system audio and is the fallback recorder, so without it a
# fresh Mac could only take screenshots and video.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# .cargo/config.toml moves the target directory, so ask Cargo where it is.
target="$(cd "$repo" && cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"

cargo build --manifest-path "$repo/Cargo.toml" -p rapidcap-ffi --release --locked
swift build --package-path "$repo/macos" -c release \
  -Xlinker -L"$target/release" \
  -Xswiftc -warnings-as-errors
bin="$(swift build --package-path "$repo/macos" -c release --show-bin-path)"

app="$repo/dist/RapidCap.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/RapidCap" "$app/Contents/MacOS/RapidCap"
cp "$repo/macos/Info.plist" "$app/Contents/Info.plist"
cp "$repo/crates/desktop/assets/rapidcap.icns" "$app/Contents/Resources/rapidcap.icns"
# English and Uzbek UI text; macOS picks one by the user's language order.
cp -R "$repo"/macos/Resources/*.lproj "$app/Contents/Resources/"

ffmpeg_url=https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip
ffmpeg_sha=c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924
ffmpeg_zip="$target/ffmpeg-9.0.2-macos-arm64.zip"
if ! echo "$ffmpeg_sha  $ffmpeg_zip" | shasum -a 256 -c --status 2>/dev/null; then
  curl -fsSL -o "$ffmpeg_zip" "$ffmpeg_url"
  echo "$ffmpeg_sha  $ffmpeg_zip" | shasum -a 256 -c --status
fi
unzip -oq "$ffmpeg_zip" ffmpeg -d "$app/Contents/MacOS"
cp "$repo/assets/ffmpeg/LICENSE.txt" "$app/Contents/Resources/FFMPEG-LICENSE.txt"
cat > "$app/Contents/Resources/FFMPEG-SOURCE.txt" <<EOF
FFmpeg 9.0.2, static macOS arm64 build by martin-riedl.de. GPL v3.
Source code: https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz
Build and configuration: https://ffmpeg.martin-riedl.de/info/detail/macos/arm64/1789931890_9.0.2
Binary: $ffmpeg_url (sha256 of the zip: $ffmpeg_sha)
EOF

identity="${RAPIDCAP_SIGN_IDENTITY:--}"
keychain="${RAPIDCAP_SIGN_KEYCHAIN:-}"
sign=(codesign --force --sign "$identity")
if [ -n "$keychain" ]; then sign+=(--keychain "$keychain"); fi
# Inside out: the bundle's signature seals the FFmpeg binary as it is.
"${sign[@]}" "$app/Contents/MacOS/ffmpeg"
"${sign[@]}" "$app"

"$app/Contents/MacOS/RapidCap" --probe > /dev/null
"$app/Contents/MacOS/ffmpeg" -hide_banner -version > /dev/null

cd "$repo/dist"
find RapidCap.app -type f -exec shasum -a 256 {} + > SHA256SUMS.txt

# RAPIDCAP_INSTALL=1 moves the bundle into /Applications. Launchers and
# Spotlight's app results look there, not in a build folder.
if [ "${RAPIDCAP_INSTALL:-0}" = 1 ]; then
  pkill -x RapidCap || true
  lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  # A fresh bundle, not one copied over the old: Finder and the launchers
  # key their icon cache on the bundle folder, and an update in place kept
  # showing the previous icon.
  rm -rf /Applications/RapidCap.app
  ditto "$app" /Applications/RapidCap.app
  touch /Applications/RapidCap.app
  # Moved, not copied: a second RapidCap.app in dist/ showed up in Spotlight
  # beside the installed one.
  "$lsregister" -u "$app" || true
  rm -rf "$app"
  "$lsregister" -f /Applications/RapidCap.app
  app=/Applications/RapidCap.app
fi
echo "$app"
