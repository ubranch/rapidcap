#!/usr/bin/env bash
# Create a self-signed codesigning identity so the Screen Recording grant
# survives rebuilds.
#
# TCC stores a *code requirement*, not a path. An ad-hoc signed bundle has no
# certificate to name, so the requirement degrades to the code hash - which
# changes on every build. The toggle in System Settings stays on, tccd refuses
# with "Failed to match existing code requirement", and AVFoundation answers
# that refusal by blocking inside avformat_open_input rather than returning an
# error. Signing with a certificate, self-signed or not, makes the requirement
# `identifier "..." and certificate leaf = H"..."`, which every later build
# signed by the same certificate satisfies.
#
# The identity lives in its own keychain rather than the login keychain: the
# login keychain cannot be unlocked from an SSH session ("User interaction is
# not allowed"), and a dedicated keychain can be unlocked from a script with a
# password this file generates and never prints.
#
# Run once. Then export the two variables it prints and run package-macos.sh.
set -euo pipefail

name="RapidCap Local Signing"
keychain="rapidcap-signing.keychain"
keychain_db="$HOME/Library/Keychains/$keychain-db"
password_file="$HOME/.rapidcap-signing.pass"

if [ -f "$keychain_db" ] && [ "${1:-}" != "--recreate" ]; then
  echo "$keychain_db already exists; pass --recreate to replace it" >&2
  exit 1
fi

umask 077
openssl rand -hex 24 > "$password_file"
password="$(cat "$password_file")"

security delete-keychain "$keychain" 2>/dev/null || true
security create-keychain -p "$password" "$keychain"
# Without this the keychain relocks on a timer and codesign fails with the
# uninformative errSecInternalComponent.
security set-keychain-settings -lut 100000000 "$keychain"
security unlock-keychain -p "$password" "$keychain"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 7300 \
  -keyout "$work/key.pem" -out "$work/cert.pem" \
  -subj "/CN=$name" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning"
# -legacy keeps the PKCS#12 readable by the Security framework, which does not
# accept OpenSSL 3's default AES-256-CBC/PBES2 encoding.
openssl pkcs12 -export -legacy -name "$name" \
  -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12" \
  -passout "pass:$password"

security import "$work/identity.p12" -k "$keychain" -P "$password" \
  -A -T /usr/bin/codesign
# The ACL above is not sufficient on its own: without a partition list entry
# for codesign, signing fails with errSecInternalComponent.
security set-key-partition-list \
  -S apple-tool:,apple:,codesign:,unsigned: -s -k "$password" \
  -D "$name" -t private "$keychain" > /dev/null

# list-keychains replaces the search list wholesale, so the existing entries
# have to be repeated or codesign stops seeing the login keychain.
existing="$(security list-keychains -d user | tr -d ' "' | grep -v "$keychain" || true)"
# shellcheck disable=SC2086
security list-keychains -d user -s $existing "$keychain_db"

cp "$work/cert.pem" "$HOME/.rapidcap-signing.crt"

cat <<INSTRUCTIONS

Identity created. Build a signed bundle with:

  export RAPIDCAP_SIGN_IDENTITY="$name"
  export RAPIDCAP_SIGN_KEYCHAIN="$keychain_db"
  security unlock-keychain -p "\$(cat $password_file)" "\$keychain_db"
  scripts/package-macos.sh

The certificate is not trusted by the system, which codesign does not require;
"security find-identity -v" will keep reporting zero *valid* identities.

Grant Screen Recording once after the first signed build. Later builds signed
by this same certificate keep the grant.
INSTRUCTIONS
