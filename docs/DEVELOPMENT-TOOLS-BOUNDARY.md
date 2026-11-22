# Development analysis tools and desktop distribution

REA and Ghidra are optional tools for developers studying reference artifacts. FLOE users do not need either tool. Vendored `.agents/skills/` files contain agent instructions and helpers, not application features or runtime dependencies.

## Source inspection — 2026-11-22

- `package.json` has no analysis-engine dependencies or install hooks; its build runs TypeScript and Vite.
- `src-tauri/Cargo.toml` has no REA/Ghidra dependency.
- `src-tauri/tauri.conf.json` packages the Vite `dist` output and declares no bundled resource or external binary for analysis tools. The Windows override selects NSIS and current-user installation only.
- Runtime source, packaging scripts, and GitHub workflows contain no REA/Ghidra engine references in the inspected search.
- `.github/workflows/package.yml` installs build dependencies on CI workers, builds the desktop applications, and uploads the macOS app archive and Windows installer. It does not install analysis engines.

This is source/configuration evidence, not inspection of the contents or installation behavior of a published binary. No local build or test was run for this review.

## Release validation TODO

- Inspect the packaged macOS bundle and Windows installer payload for unintended developer skill files, analysis engines, or installer scripts.
- Confirm a clean installation and first launch do not request or download REA/Ghidra. Keep normal platform prerequisites distinct from these developer tools.
- Preserve this separation when changing bundle resources, sidecars, install/update hooks, or dependency lifecycle scripts.


## macOS preview.5 artifact inspection — 2026-11-22

Inspected the uploaded draft-release asset from Actions run `44987167737`, commit `80a64363b0879988f94571a2ee2497994d8bee12`, without extracting or launching the application.

- Archive: `FLOE-macos-arm64.zip`, 17,121,585 bytes.
- SHA-256: `cd05fec45231370961588a3946a6b8568b323f938af07e313af059f02cb0ba84`; matches both the uploaded manifest and checksum file.
- The ZIP has five entries: the app directory, `Contents/`, `Contents/MacOS/`, the `branchdesk` executable, and `Contents/Info.plist`.
- No separate REA/Ghidra/JDK engine, `.agents` directory, skill file, node_modules tree, or installer script is packaged as a ZIP entry.

Scope: this proves the archive inventory and checksum consistency for this exact macOS asset. The executable's embedded contents and first-launch network behavior were not inspected. Windows installer inspection remains pending. No runtime test, local build, or analysis-engine installation was performed. Temporary downloaded review files were removed after recording the evidence.
