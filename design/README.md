# FLOE design sources (development only)

- `brand/floe-icon-source.png`: exact user-provided image, unchanged. Tauri icon tooling converts it to the desktop PNG/ICNS/ICO files in `src-tauri/icons/`. No generated replacement or background removal was applied.
- `reference/screenshots/official`: collected GitKraken reference artwork. Original URLs and hashes are preserved in `reference/inventory/official-assets.json`; third-party ownership remains unchanged.
- `reference/screenshots/live`: project screen captures used during reconstruction.
- `reference/reports`: implementation screenshots. Capture names retain their historical product names.
- `reference/manifest.json`: image inventory with content hashes and sizes.

Account/provisioning/keychain, identity-setting and terminal captures are excluded from publication. Logs, compiled app backups, fixture Git databases, extracted application JavaScript, analysis engines and caches are not design assets and were not copied.

This entire directory is outside Vite's `public` and Tauri's frontend distribution. Only the converted application icons are release inputs. CI checks the distribution boundary before uploading installers. These reference captures do not establish current runtime acceptance.
