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

## Verification update — 2026-11-25

- Actions run 44996146251 succeeded for macOS ARM64 and Windows x64; macOS Developer ID, hardened runtime, timestamp, stapled ticket and Gatekeeper checks passed, including ZIP extraction. Publication was deliberately skipped on the work branch. Public devtest.4 remains the older artifact.
- PR checks remain failing: macOS reported 15 failures, Windows 102. These are not resolved by package success.
- Corrected the TLS integration fixture's dependency on a file outside the repository: the synthetic Python server is now tracked under scripts and referenced within this checkout. Early child failure is reported directly. It is not bundled into the app.
- Updated the provider merge test server to answer the required preflight GET before the mutation, while asserting the GET route and retaining the existing mutation payload/receipt assertions. Production preflight checks remain unchanged.
- Pending: CI execution of these changes, Git GC/lock failure analysis, Windows failures, PR/main acceptance, and publication of a newly notarized release. No local application build was run.

### Remaining fixture corrections
- Crash-recovery object GC checks now disable reflog expiration and ref packing so crash-owned ref locks remain intact; unreachable object/prune and pack-pin assertions are retained. A separate synthetic Git repository reproduced the HEAD.lock diagnostic and confirmed that object-only GC avoids it without deleting the lock. This is not a passing Rust integration result.
- Windows workspace fixtures now use ordinary absolute temporary paths rather than canonicalization's extended device namespace, which repository_path intentionally rejects. Production path validation is unchanged.
- These corrections require macOS/Windows CI; outstanding Windows checkout, clone and recovery failures remain under investigation.
