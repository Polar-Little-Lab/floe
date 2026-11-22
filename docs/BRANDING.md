# FLOE naming and compatibility

The service and desktop product are **FLOE**. The isolated testing edition is **FLOE Dev Test**. Application titles, native menus/prompts, UI copy, package metadata, executable (`floe`), new CI artifacts and new release titles use this name.

Compatibility identifiers are not product branding. Existing bundle identifiers (`dev.branchdesk.desktop` and its `.devtest` variant), credential namespaces, browser preference keys, IPC names, native ABI symbols, Git recovery paths/refs and test fault-injection variables retain their original values. Changing these without migration could hide saved settings, credentials or unfinished recovery records, or let old/new applications bypass their shared repository lock.

Historical release files, source snapshots and vendored upstream provenance remain unchanged. New packages use `FLOE-macos-arm64.zip`, `FLOE-windows-x64-setup.exe` and the equivalent `FLOE-Dev-Test-*` names. The GitHub repository URL remains unchanged.

Manual validation TODO: application/menu title on both platforms, installation beside the testing edition, existing settings and credentials, old interrupted-operation recovery, and downloading the correctly named CI artifacts. No local build or runtime tests were executed for this rename.
