#!/usr/bin/env bash
# Sign a bare macOS binary with a Developer ID Application certificate and notarize it.
# A bare binary cannot be stapled, so Gatekeeper checks notarization online.
#
# Usage: sign-macos-cli.sh <binary>
# Required environment: APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD,
# APPLE_SIGNING_IDENTITY, APPLE_API_KEY (key ID), APPLE_API_ISSUER, APPLE_API_KEY_P8 (key text).
set -euo pipefail
set +x

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <binary>" >&2
  exit 2
fi

binary="$1"
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_P8; do
  if [ -z "${!name:-}" ]; then
    echo "missing required environment variable: $name" >&2
    exit 1
  fi
done

work="$(mktemp -d)"
keychain="$work/signing.keychain-db"
key_file="$work/AuthKey.p8"
original_keychains="$(security list-keychains -d user | tr -d '"' | xargs)"

cleanup() {
  # shellcheck disable=SC2086
  security list-keychains -d user -s $original_keychains >/dev/null 2>&1 || true
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

keychain_password="$(openssl rand -hex 24)"
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$work/certificate.p12"

security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
# shellcheck disable=SC2086
security list-keychains -d user -s "$keychain" $original_keychains
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null

codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$binary"
codesign --verify --strict --verbose=2 "$binary"

printf '%s' "$APPLE_API_KEY_P8" > "$key_file"
chmod 600 "$key_file"

zip_file="$work/$(basename "$binary").zip"
ditto -c -k --keepParent "$binary" "$zip_file"

result="$(xcrun notarytool submit "$zip_file" \
  --key "$key_file" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" \
  --wait --output-format json)"
status="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))')"
submission_id="$(printf '%s' "$result" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id",""))')"

if [ "$status" != "Accepted" ]; then
  echo "notarization finished with status '$status' (submission $submission_id)" >&2
  if [ -n "$submission_id" ]; then
    xcrun notarytool log "$submission_id" \
      --key "$key_file" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" >&2 || true
  fi
  exit 1
fi

echo "notarized $binary (submission $submission_id)"
