# FLOE development

## Priorities
- Match observed GitKraken screen composition, navigation and entry points, including Settings and Launchpad. Reuse shared components and existing button styling. AI features are excluded.
- Inspect a relevant original capture before changing a screen; do not claim visual fidelity without comparison. Keep reference provenance.
- Implement in complete screen/feature batches. Record unexecuted validation as TODO; never present implementation or historical tests as current acceptance.
- Do not build locally. Build and package through GitHub Actions. Public preview release is authorized. macOS CI uses Developer ID signing and notarization; publish only after the required CI and package verification checks pass. Windows signing is not configured.
- Use isolated synthetic repositories for mutation checks. Do not mutate personal repositories. Never commit credentials, signing keys, caches or build artifacts.

## Repository skills
The installed upstream skills are vendored in `.agents/skills/`:
- `reverse-engineering`: artifact triage and platform-specific analysis.
- `ghidra-headless`: Ghidra headless analysis when Ghidra/JDK are available.
- `reverse-engineer-anything`: evidence-based REA workflows when its tools are available.

Read the relevant SKILL.md when applicable. Resolve skill-relative paths against its directory in this checkout; do not assume a user's global skill path. Installing skill instructions does not install analysis engines or register MCP servers. Ordinary source implementation and CI packaging do not require running reverse-engineering engines.

Analysis engines are developer-only tools. Never add REA, Ghidra, their MCP servers, installers, or skill directories to application runtime dependencies, bundled resources, sidecars, or install/update hooks. End users must not download or run them to use FLOE. Vendored skill instructions do not authorize automatic engine installation. Keep development analysis setup separate from desktop release packaging.

`skill-sources.json` preserves upstream repository, pinned commit, local compatibility changes and file hashes. `docs/agent-context/` preserves the parent workspace instructions as historical context, not overriding the current priorities above. Research captures referenced there remain in the original research workspace unless explicitly included here.
