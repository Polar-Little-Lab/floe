# Apple notarization

FLOE follows pGit's Developer ID + App Store Connect Team API Key setup, adapted to Tauri rather than Electron. Both packaging workflows require the following repository Actions secrets; values must never be committed or printed:

- `APPLE_CERTIFICATE_P12_BASE64`: selected Developer ID identity and private key, exported as encrypted PKCS#12.
- `APPLE_CERTIFICATE_PASSWORD`: export password.
- `APPLE_SIGNING_IDENTITY`: full Developer ID Application identity name.
- `APPLE_API_KEY_BASE64`: App Store Connect Team API private key.
- `APPLE_API_KEY_ID`: key identifier.
- `APPLE_API_ISSUER`: issuer identifier.

GitHub cannot return existing pGit secret values. Provision FLOE through a secure local input path from the original credentials. Do not log or expose pGit secrets via Actions to retrieve them.

The macOS job imports the certificate into an ephemeral runner keychain and writes the API key with restricted permissions. Tauri receives the signing identity and notarization credentials through environment variables. The job verifies Developer ID, hardened runtime, timestamp, the stapled ticket and Gatekeeper, then repeats ticket/Gatekeeper checks after extracting the distributed ZIP. Temporary credentials are removed in an always-run cleanup step. There is no unsigned fallback in release CI.

The base configuration's ad-hoc identity remains useful only for local/test tooling. The CI identity environment variable overrides it. API credentials must match the Developer ID team. No App Store provisioning profile is needed for this outside-App-Store distribution.

Validation performed on preparation: shell syntax and source diff review. Credential provisioning and a full signed/notarized CI run remain required before these changes establish a distributable artifact. Previous releases remain unchanged by workflow edits.

References: pGit `docs/APPLE_SIGNING.md`, `.github/workflows/release-macos.yml`; https://v2.tauri.app/distribute/sign/macos/.
