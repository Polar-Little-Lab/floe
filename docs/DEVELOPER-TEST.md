# FLOE Dev Test

Download the matching package from the `v0.1.0-devtest.*` GitHub release. CI publishes the release only when both platforms succeed. Successful platform packages are also available in the Actions run's Artifacts for 14 days.

- Apple Silicon Mac: unzip `FLOE-Dev-Test-macos-arm64.zip` and copy `FLOE Dev Test.app` to Applications. Intel Mac is not included.
- Windows x64: run `FLOE-Dev-Test-windows-x64-setup.exe` (current-user installation).
- No development server, Node, Rust, REA or Ghidra installation is required. External Git/SSH integrations can still use system tools/configuration.

## Developer settings

This is a release build with the `developer-test` Cargo feature. Web Inspector opens automatically at launch; restart the app to reopen it after closing. Normal previews do not enable this feature. CSP, IPC origin restrictions, native credential prompts and repository write policies remain enabled.

App name, identifier (`dev.branchdesk.desktop.devtest`), app data directory and the app's HTTPS credential service namespace are separate from the normal application. Existing settings/logins are not copied. Selected Git repositories and system Git/SSH configuration are NOT isolated: use disposable clones and test accounts.

New CI packages require Developer ID signing, hardened runtime, Apple notarization, stapled tickets and Gatekeeper verification, including after ZIP extraction. Missing credentials or failed checks stop publication. Older devtest.4 and earlier artifacts do not acquire notarization automatically. Windows remains unsigned. Do not globally disable system security.

## Manual acceptance TODO

1. Launchpad: open a test repository, reopen a recent repository, change Settings and restart to check persistence.
2. Diff, stage/unstage, commit, create/switch branches, fetch/pull/push against a disposable remote.
3. PR/Issue create/edit/review/approve/merge with a test account; worktree create/connect/remove and Gitflow flows.
4. LFS index scan, Fetch/Push, cancellation, cached-object reuse and upload-record inspection/acknowledgement. Actual LFS checkout/pull/prune remains disabled; checkout preview is read-only.
5. Use a disposable repository on a size-limited test volume for disk-full and process-termination recovery. Do not fill the system disk or use personal repositories.

Packaging success is not runtime acceptance. Check the source commit and linked Actions run for this download: results from an earlier revision do not prove this package passed. CI covers synthetic repository scenarios; provider accounts, interactive prompts and installed-app behavior still require the manual checks above.

### Download and launch acceptance (not yet completed)

- macOS: download the release ZIP through a browser, extract it with Archive Utility, copy the app to Applications and launch it with the download quarantine intact. Confirm Gatekeeper accepts it without removing quarantine or disabling security. Then close and reopen the app and open a disposable repository.
- Windows: install the release EXE as a standard user, launch it, close/reopen it and uninstall it. Windows signing is not configured; distinguish a publisher warning from an installation or runtime failure.
- Record the exact release tag, manifest commit, OS version and outcome. CI signature/archive checks alone do not complete either interactive acceptance item.

## Reporting a problem

Include OS version, commit/runId from `manifest-<platform>.json`, reproduction steps and expected/actual behavior. Copy only relevant console errors; redact tokens, authorization headers, signed URLs and personal paths. There is no automatic log upload. Compare downloads with `SHA256SUMS-<platform>.txt`.

Developer-console configuration reference: https://v2.tauri.app/develop/debug/
