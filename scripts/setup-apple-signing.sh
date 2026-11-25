#!/usr/bin/env bash
set -euo pipefail
set +x
umask 077
: "${RUNNER_TEMP:?GitHub runner required}"
: "${GITHUB_ENV:?GitHub runner required}"
: "${APPLE_CERTIFICATE_P12_BASE64:?Missing signing certificate}"
: "${APPLE_CERTIFICATE_PASSWORD:?Missing certificate password}"
: "${APPLE_SIGNING_IDENTITY:?Missing Developer ID identity}"
: "${APPLE_API_KEY_BASE64:?Missing notarization API key}"
: "${APPLE_API_KEY_ID:?Missing notarization key ID}"
: "${APPLE_API_ISSUER:?Missing notarization issuer}"
[[ "$APPLE_SIGNING_IDENTITY" == 'Developer ID Application: '* ]] || exit 1
signing_dir="$RUNNER_TEMP/floe-apple-signing"
mkdir -m 700 "$signing_dir"
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$signing_dir/certificate.p12"
printf '%s' "$APPLE_API_KEY_BASE64" | base64 --decode > "$signing_dir/AuthKey.p8"
keychain_password="$(openssl rand -hex 32)"
echo "::add-mask::$keychain_password"
keychain_path="$signing_dir/signing.keychain-db"
security create-keychain -p "$keychain_password" "$keychain_path"
security set-keychain-settings -lut 21600 "$keychain_path"
security unlock-keychain -p "$keychain_password" "$keychain_path"
security import "$signing_dir/certificate.p12" -k "$keychain_path" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain_path" >/dev/null
security list-keychains -d user -s "$keychain_path"
security find-identity -v -p codesigning "$keychain_path" | grep -F -- "$APPLE_SIGNING_IDENTITY" >/dev/null
rm "$signing_dir/certificate.p12"
printf 'APPLE_API_KEY_PATH=%s\n' "$signing_dir/AuthKey.p8" >> "$GITHUB_ENV"
