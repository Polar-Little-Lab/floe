# Worktree branch entry — 2026-11-22

Connected the branch context-menu action `Create worktree from <branch>` to the existing worktree creation flow. The selected branch is preselected; the worktree name receives focus. Existing branch availability checks reject checked-out/occupied branches after the listing is loaded. Existing backend revision checks and confirmation still govern creation; no checkout occurs merely by opening this menu.

Reference inspected in parent research workspace: `screenshots/official/1b16f25e2590-gkd-10-5-create-worktree.png` and `1bbf40055dbf-gkd-10-5-worktrees-actions.png`.

- [ ] Non-current local branch menu opens worktree creation with that branch selected.
- [ ] Current branch, detached/read-only repositories and missing commit disable the entry.
- [ ] A branch occupied by another worktree remains selected but unavailable; no fallback silently creates a different branch.
- [ ] Cancel leaves Git state unchanged; successful creation appears in the listing and opens through Open this worktree.
- [ ] External branch/repository changes, existing destination and partial creation failure retain existing backend error/recovery behavior.
- [ ] Validate keyboard focus and rendered menu against the capture on macOS and Windows.

Implementation only; no local build or test execution. Current released preview predates this change.

## Creation form
The branch selector uses the shared searchable FilterSelect. Name validation mirrors the backend creation rule, rejects an already-listed worktree name, and explains when the default new-branch name collides with an existing branch. Backend filesystem and revision checks remain authoritative.

- [ ] Search and select an available branch; empty selection creates a new branch from HEAD.
- [ ] Invalid/duplicate names and default-branch collisions block submission with an explanation.
- [ ] Occupied branches remain unavailable after refreshing the list.
- [ ] Existing destination, external branch creation and permissions errors still reach backend validation without silent fallback.

Source changes only; runtime and visual validation remain pending.

## Native parent-folder chooser
Added a window-checked, operation-guarded native folder chooser scoped to the main window capability. It returns the chosen path without opening a repository or writing files. The worktree form appends the validated worktree name, keeps manual path editing, handles cancellation, and prevents duplicate chooser requests. Actual creation still uses existing filesystem/revision validation and native confirmation.

- [ ] macOS folder, Windows drive root and UNC parent produce the expected child path; spaces and non-ASCII names are retained.
- [ ] Cancel keeps the prior typed destination; errors are visible and release the busy state.
- [ ] Native chooser does not create folders or modify repositories.
- [ ] Creation still rejects existing/inside-repository destinations and changed repository state.
- [ ] CI compiles the new command and generates its permission manifest; main-window capability enables only this command.

No local compilation or tests performed.

## Sidebar navigation
- Added expandable WORKTREES list matching the reference sidebar entry, using shared ReferenceGroupHeading and ReferenceRow. Gitflow now shares the same heading component.
- Branch/name/path filter, lock/missing status, open-worktree, terminal and Finder/Explorer entries are connected. Management remains the entry for removal/lock/prune; no destructive operation occurs on row selection.
- Reads occur while expanded, with loading/error/reload states and stale async response cancellation. Selected unavailable branches in the creation form retain their actual name instead of looking like the new-branch default.
- Corrected stale native creation confirmation text to describe currently supported Stage/Unstage/Commit writes in linked worktrees.
- TODO: macOS/Windows sidebar navigation, invalid worktree disabled actions, filtering, truncated lists, repository switch during read, operation failure/reload, original-capture visual comparison. No local build/tests run. This change is not included in the active CI run for 80a6436.

## Shared sidebar read scheduling
- Source inspection confirmed `read_worktrees` and `read_gitflow_settings` acquire the same native OperationGuard as writes. Replaced direct sidebar API calls with useSidebarRead through App's existing run gate.
- Reads wait while another app operation is busy, retain successful data across unrelated busy changes, and ignore results for replaced repository revisions/unmounted views. Failure requires explicit reload; no mutation retries are introduced.
- TODO: expand both groups together, switch repositories while reading, mutate while groups are open, collapsed/unmounted completion, read failure then reload. Verify no repeated requests for an unchanged revision and no read/write BUSY collision from these sidebars.
