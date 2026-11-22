# macOS packaging correction

The downloaded devtest.3 ZIP matched its published SHA-256. Its application failed `codesign --verify --deep --strict` with `code has no resources but signature indicates they must be present`. Inspection showed a linker-signed executable, no sealed bundle resources, and an unbound Info.plist. This was a packaging/signing defect, not evidence of a corrupt download.

Re-signing a disposable extracted copy as a complete ad-hoc app made strict verification pass. CI now sets Tauri's macOS signing identity to `-`, verifies the complete app before archiving, then extracts the actual distributable ZIP and verifies it again, including the resource seal. A failure blocks artifact upload and release publication.

This does not establish Developer ID trust, notarization, Gatekeeper acceptance or application runtime behavior. Apple notarization credentials are not configured. The preview can still require explicit per-app approval in macOS Privacy & Security; do not disable Gatekeeper globally.

Reference: https://v2.tauri.app/distribute/sign/macos/
