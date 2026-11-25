#!/usr/bin/env bash
set -euo pipefail
: "${RUNNER_TEMP:?GitHub runner required}"
signing_dir="$RUNNER_TEMP/floe-apple-signing"
if [[ -f "$signing_dir/signing.keychain-db" ]]; then
  security delete-keychain "$signing_dir/signing.keychain-db"
fi
rm -f "$signing_dir/certificate.p12" "$signing_dir/AuthKey.p8"
rmdir "$signing_dir" 2>/dev/null || true
