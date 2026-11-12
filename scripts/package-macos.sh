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
# FFmpeg is not vendored: the core searches PATH and both Homebrew prefixes.
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
