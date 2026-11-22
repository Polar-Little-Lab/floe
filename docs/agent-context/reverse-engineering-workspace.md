# Reverse engineering workspace

Use installed upstream skills according to the requested task:
- `$reverse-engineering`: default for artifact triage and CLI-based analysis; includes tool detection, platform guides, and analysis helpers.
- `$ghidra-headless`: Ghidra batch decompilation and structured exports when Ghidra and its required JDK are available. Resolve `{baseDir}` to `/Users/polar/.codex/skills/ghidra-headless`.
- `$reverse-engineer-anything`: REA evidence-oriented investigation workflows when REA tools are connected or its CLI is available. Skill installation alone does not register MCP or install REA.

For the generic skill, resolve `${CLAUDE_SKILL_DIR}` to `/Users/polar/.codex/skills/reverse-engineering`; Codex does not supply that variable automatically.
Preserve original inputs in `samples/`, derived files in `analysis/`, helpers in `scripts/`, and reports in `reports/`. Use Korean unless requested otherwise. Record hashes and address conventions. Verify actual tool availability before using an engine. Upstream prerequisite examples may be outdated: use requirements of the installed version.

`skill-sources.json` records pinned upstream commits, installed hashes, and local compatibility adjustments. The former custom skills are archived under `backups/original-local-skills/`; do not treat them as installed active skills.
