# Gitflow implementation status

Reference inspected: parent research `screenshots/live/020-preferences-gitflow.jpg`. Required screen order: Branches (Main/Master, Develop), Prefixes (Feature, Release, Hotfix, Version Tag), Initialize Gitflow.

Implemented backend read: read_gitflow_settings reads the six standard Gitflow configuration keys, reports missing configured branches, bounds/rejects malformed values, checks repository revision, and uses the main-window capability and shared native operation guard. Missing configuration returns null rather than pretending defaults were initialized. It does not change Git configuration or execute git-flow.

Still required: settings UI connection, transactional initialization/config updates, feature/release/hotfix start and finish, merge-conflict/recovery handling and version tagging. Gitflow is not complete.

Deferred verification:
- [ ] Unconfigured repository returns missing values without writes.
- [ ] Standard existing Gitflow configuration and custom prefixes are read correctly.
- [ ] Missing main/develop refs, invalid/control-character/oversized config and stale revisions are reported.
- [ ] Main-window command capability and CI compilation on macOS/Windows.
- [ ] Later initialization/finish operations preserve config, refs and worktree on cancellation/failure, with recoverable interrupted writes.

No local build/test run for this implementation.

## Preferences integration
Connected repository-specific Gitflow navigation and settings search. The screen uses shared SettingsSection/SettingField components in the reference Branches → Prefixes order. Values are read from Git configuration when the Gitflow page opens; no fabricated initialized defaults are shown. Reload obtains a current repository snapshot, shows failures, retains previously loaded values, and ignores responses after leaving the page. Empty version-tag prefix is distinguished from missing configuration. Editing/initialization remain explicitly unavailable.

- [ ] Opening Gitflow loads the current repository only; switching repositories/pages ignores old responses.
- [ ] Settings search reaches Main/Master, Develop and each prefix field.
- [ ] Missing configuration, empty tag prefix, missing branch and read failures display distinct states.
- [ ] Reload after an external configuration change updates values without writing files.
- [ ] Compare rendered screen to the inspected GitKraken reference; not yet visually verified.

No local build/test execution. Initialization/start/finish remain outstanding.

## Journaled configuration write
Added setGitflowConfiguration to the native mutation pipeline. Configuration validates branch names, distinct main/develop, distinct feature/release/hotfix prefixes and tag prefix. Both branches must already resolve to commits. The existing config lock, native mutation confirmation, before/after journal, atomic replacement and recovery verification are reused with a version-5 record. Only the six Gitflow keys are edited; unrelated config is retained. Older journal records default the new field to absent.

This is configuration writing, not complete Gitflow initialization: automatic creation of missing main/develop branches and initialization UI still need implementation.

- [ ] Valid write changes only six keys; existing identity/remotes remain intact.
- [ ] Invalid names/prefixes or missing branch refuse before config publication.
- [ ] Existing config lock, changed revision and external config modification refuse safely.
- [ ] Interrupted publication recovers using verified before/after bytes; altered journal fields reject recovery.
- [ ] Existing journal versions and unrelated identity/upstream/remote operations remain compatible.
- [ ] Disk-full and sync failures preserve recovery evidence without reporting success.

Implementation/source inspection only. No build or regression tests executed.

## Configuration editing UI
Gitflow fields now edit a draft and save through the journaled native mutation and app operation guard. Main/develop must already exist; the form explains missing branches instead of pretending full initialization is implemented. Branch/prefix validation blocks malformed input, native cancellation/failure retains the draft, and successful writes update the app snapshot. Reloading an edited draft uses the shared discard confirmation; a failed reload retains input.

- [ ] Save existing-branch configuration and verify all six keys from a fresh read.
- [ ] Cancel native confirmation, reject stale revision, and fail config writes: inputs retained and no false success.
- [ ] Saving while another operation is active does not submit a second mutation.
- [ ] Reload Keep editing retains inputs; confirmed successful reload replaces them.
- [ ] Missing branches and malformed/colliding names prevent save; backend still independently validates.
- [ ] Successful write followed by read failure distinguishes saved status from read error.

No local build/test run. Automatic branch initialization/start/finish remain pending.

## Configuration reader correction
Source review found `repository::open` intentionally replaces libgit2 config with an empty config. Gitflow reads now use the validated Layout common directory, a bounded config read and the existing include-blocking read-only parser. This reads shared config for linked worktrees without following user/global includes. The bytes are compared again before returning; concurrent config changes require reload. This supersedes the original `repo.config()` implementation, which could report valid configuration as absent.

- [ ] Existing local Gitflow keys are actually returned despite sanitized repository handles.
- [ ] Linked worktree reads use the validated common config.
- [ ] Include/includeIf/oversized config is rejected before parser indirection.
- [ ] Config changed during reading rejects the stale result.

Correction verified by source inspection only; execution remains pending.

## Initialization flow
Initialize Gitflow now composes the existing native-confirmed mutations: create missing main at current HEAD, create missing develop at main, persist configuration, then check out develop. Existing branches are not replaced. The UI shows the order and records confirmed steps. Cancellation/failure stops immediately, keeps completed work, and requires Reload before retry. Leaving the settings page stops subsequent steps. This is a resumable sequence, not an atomic transaction across all steps; each native mutation retains its own existing validation/recovery behavior.

- [ ] Existing main/missing develop, missing both, custom names, and already-configured repositories initialize from the displayed source commits.
- [ ] Cancel/fail each stage; earlier confirmed stages remain and later stages do not execute.
- [ ] Dirty checkout, occupied develop, detached HEAD, unborn HEAD and read-only policy produce correct blocks/errors without overwriting data.
- [ ] Reload after partial completion reads actual refs/config and permits a deliberate retry.
- [ ] Leave settings during native confirmation: no subsequent stage starts after the response.
- [ ] Successful final checkout selects develop and updates the graph snapshot.

No local build/test execution. Gitflow start/finish operations remain outstanding.

## Start feature/release/hotfix
Added a GITFLOW sidebar entry and panel based on `screenshots/official/60bf00d3402f-giflow-panel-2025_2x.png`: Start Feature/Release/Hotfix followed by Finish controls. Start uses saved prefixes, develop for features/releases and main for hotfixes. It creates the branch at the displayed source commit, then checks it out through existing native confirmations. A failed/cancelled checkout retains the created branch; the panel requires reload before another start. Finish controls remain explicitly unavailable.

- [ ] Verify each type's source commit, prefixed name, checkout and graph state.
- [ ] Uninitialized/missing source/read-only repositories, invalid and duplicate branch names cannot start.
- [ ] Native cancellation before creation writes nothing; checkout cancellation retains the branch and reports partial completion.
- [ ] Reload observes external changes and clears attempted state without retrying a mutation.
- [ ] Gitflow preferences entry navigates to the existing settings screen.
- [ ] Compare panel/sidebar presentation to reference on macOS and Windows.

No local builds/tests run. Finish operations and runtime validation remain outstanding.

## Start source outside graph window
CreateBranchAt now accepts a commit present either in the current graph page or at a local branch tip in the freshly validated snapshot. Gitflow main/develop may be outside the loaded history page; those valid source tips should not be rejected as INVALID_COMMIT. Arbitrary unseen/unreferenced object IDs remain rejected. Native ref locks and revision validation remain unchanged.

- [ ] Start from main/develop outside the graph page succeeds at that exact branch tip.
- [ ] Unreferenced/unseen OIDs and stale snapshots still reject branch creation.
- [ ] External source branch changes between snapshot and operation require revalidation.

CI package run 44986230402 was dispatched for e659027 (the preceding batch); it does not contain this subsequent source-window correction. No local build/test run.

## Existing configuration validation
The reader now reports incomplete and semantically invalid configuration, using the same validation as writes. Duplicate main/develop or conflicting prefixes no longer enable Start merely because all strings exist. Preferences retain fields for repair; Start blocks until a valid configuration is loaded. This also distinguishes empty tag prefix (valid) from absent tag-prefix configuration (incomplete).

- [ ] Externally configured duplicate branches/prefixes disable Start and remain editable in Preferences.
- [ ] Missing keys disable Start; an explicitly empty version-tag prefix remains valid.
- [ ] Repair, save and reload enable Start without fabricated defaults.

CI run 44986230402 remains the earlier batch; this follow-up is not included. No local tests/builds run.

## Preferences page lifetime
Preferences pages now mount their feature content only on first visit and retain it while switching categories. This preserves Gitflow edits across category navigation and prevents unvisited identity/external/commit/Gitflow panels from launching concurrent native reads on opening Preferences. Closing Preferences still ends the editing session. The shared page wrapper keeps existing headings, search targets and hidden-page layout.

- [ ] Edit Gitflow, visit General, return: draft and validation state are retained.
- [ ] Opening General does not start Gitflow/identity/commit/external-panel reads.
- [ ] Search can activate a never-visited page and focus its setting.
- [ ] Repo change/Preferences close still unmounts old repository panels and ignores stale reads.

Source review confirmed snapshot revision already includes repository config bytes; no duplicate config token was added. No local tests/builds run.

## Finish preview backend
Added a read-only finish preview: validates configuration/source prefix, resolves source and target commits, reports whether targets already contain source, and derives release/hotfix tags with collision status. Feature targets develop; release/hotfix target main then develop. Native window/operation/revision checks apply. Preview does not perform merges, checkout, tagging or deletion; execution/UI still need connection.

- [ ] Each flow resolves expected source/targets and version-prefixed tag.
- [ ] Main/develop cannot be used as flow sources; invalid prefixes/missing refs/config are rejected.
- [ ] Existing tags are reported without replacement; unrelated histories/graph errors do not fabricate ancestry.
- [ ] External changes invalidate stale previews; read-only preview leaves refs/index/worktree unchanged.

No local builds/tests run.

## Finish preview UI
The Gitflow Start/Finish panel now enables a finish preview for the checked-out matching feature/release/hotfix branch. It displays source commit, ordered target branches, existing ancestry and version-tag collision from the backend. Missing configuration, detached/main/develop/mismatched branches cannot open an inappropriate finish preview. Reload clears the old preview. Source review reference: `screenshots/official/f0c2daa4767f-finish-release-2025_2x.png`; execution fields/actions are not falsely enabled while finish execution remains incomplete.

- [ ] Correct Finish type is available for the checked-out flow branch.
- [ ] Preview displays native target refs/OIDs and existing tag warning without writes.
- [ ] Stale repository revision rejects preview; reload fetches current data.
- [ ] Switching/closing a panel during the native request does not attach its result to a different panel.

Mac packaging of e659027 succeeded in CI run 44986230402; Windows still running when recorded. Later changes including this preview are not in that run. No local build/tests run.

## Finish execution (source implementation, 2026-11-22)
- Connected Finish to existing revision-checked, confirmed native checkout/merge/tag/delete actions. Feature merges into develop; release/hotfix merge into main, create an annotated version tag, then merge into develop. Optional source deletion only follows fresh ancestry checks for every target.
- Original reference: `screenshots/official/f0c2daa4767f-finish-release-2025_2x.png` in the research workspace. Added tag message, checked Delete branch, Finish and Cancel using existing controls.
- Requires clean writable state and identity; checks preview/config/source/target equality before writes and source identity on every merge. Existing tags are never replaced. Each successful mutation updates the app snapshot.
- Conflicts hand off to the existing merge dialog. Failures report confirmed steps, refresh state, retain completed mutations, and prohibit blind retry from the form.
- Limitation: this is a sequence of individually journaled operations, not an atomic Gitflow transaction. Automatic resume after conflict or a partial tagged release is not implemented; remaining steps must be inspected. No completion claim after a conflict.

### Pending validation (not executed; no local build/tests)
- Feature fast-forward, merge commit, already-contained source, retain/delete source.
- Release/hotfix target ordering, annotated tag target/message/identity and final develop checkout.
- Cancel each native confirmation; verify completed steps and source retention.
- Conflict in first/second target; resolve/abort via existing merge dialog and inspect partial state.
- Stale preview/config/branch tips, dirty worktree, recovery lock, existing tag, missing identity.
- Failure after first merge/tag, disk-full, app exit and native journal recovery; no automatic duplicate writes.
- macOS and Windows integration/typecheck through CI still required for these changes.

## Re-entry after an interrupted Finish
- Finish now uses the shared searchable branch selector, defaulting to the current branch. It also works while checked out on main/develop after a merge conflict, cancellation or partial execution. Switching source hides the old preview; a new preview must be requested.
- Start and Finish attempts mutually lock further writes until reload. The executor rejects an active merge/rebase/replay/sequence instead of starting another operation over it.
- TODO: feature conflict → resolve → reopen Gitflow → select original feature → verify containment → delete; source switching must never execute the previous source's preview; cancellation then reload must show fresh target state.
- Tagged partial release/hotfix resume still requires a dedicated existing-tag verification path; existing tags remain blocked rather than overwritten.

## Resume with an existing version tag
- Native Finish preview now reports the existing tag object and whether it is an annotated tag resolving to the current main tip that contains the selected source.
- A matching tag can be explicitly retained with a checkbox; Finish skips tag creation and completes remaining merges/optional source deletion. Unrelated, lightweight, or moved tags remain blocked. No tag is replaced.
- Before deleting the source, recheck tag identity and eligibility alongside target ancestry. The existing tag's message and author are preserved.
- TODO (not run): interruption after tag creation → reload → explicitly retain matching tag → finish develop; reject mismatched/lightweight tag; reject moved main/source/tag; verify tag identity/message unchanged; conflict in develop with existing tag → resolve → reopen → finish cleanup. Run on macOS and Windows through CI.
- Limitation: no durable whole-flow resume journal; eligibility is reconstructed from current refs. A main branch advanced beyond the tagged commit requires manual review.

## Final cleanup guard
- Capture the newly created annotated tag object and verify its commit before moving on. Final cleanup checks both newly created and retained tags, plus the configured tag name and ordered target branches.
- TODO: replace/delete a new tag or change Gitflow configuration before source cleanup; verify the source is retained and no false Finish completion is reported.

## Gitflow sidebar reference integration
- Reference: official `60bf00d3402f-giflow-panel-2025_2x.png` and Finish Release capture. Separate expansion and Open Gitflow entry; show develop/main then feature/release/hotfix prefix folders with current-branch indication.
- Reuses ReferenceRow for selection and context menus. Checkout uses existing native mutation; Finish opens a freshly read preview for the selected branch, including a non-current branch after an interrupted Finish.
- Repository settings load only while expanded; responses from old revisions/unmounted sections are ignored. Failed reads expose a reload action and never invent default configuration.
- TODO: compare rendered sidebar against captures; prefix folders/name+SHA filter, context menu checkout/finish, active repository switch during read, missing/invalid config, keyboard navigation. No local build or test executed. These sidebar changes are later than the running preview build at 80a6436.

## Finish progress and failure visibility
- Finish reports the active checkout/merge/tag/delete or verification stage and a receipt of confirmed steps. The panel retains this receipt across reloads in the current app session, scoped to repository and labeled with source branch.
- Conflict and failure states are distinct from completion; reopening Gitflow shows where it stopped. No automatic write retry is added. Receipts are not persisted across app restart.
- Background sidebar reads now preserve foreground error messages instead of clearing a Finish failure as soon as the repository revision refreshes.
- TODO: failure/cancel at every mutation, conflict handoff/reopen, reload without losing the receipt, repository switching, final successful completion, and sidebar reload while a foreground error is displayed. Runtime validation remains pending.

## Settings navigation
- Removed the outdated Start/Finish unavailable notice and connected Open Gitflow to the current saved repository configuration. Unsaved edits, missing/invalid configuration and initialization recovery state disable navigation with guidance to save/reload first.
- TODO: save → Open Gitflow → Start/Finish, invalid settings, dirty draft, initialization failure, and unavailable repository. No local build or runtime check executed.

## Settings operation ownership
- Gitflow preferences now hold App's operation gate for the whole load, save+readback, or initialization sequence. Individual native mutations retain their own confirmation and revision checks, but sidebar reads cannot interleave between initialization steps through App.
- Busy rejection is surfaced instead of reporting success for a skipped callback. Save readback failures require reload; settings responses after unmount/repository changes are ignored.
- TODO: initialize with both sidebars expanded, save followed by sidebar refresh, competing foreground operation, close settings during native confirmation, partial initialization cancellation, stale revision and readback failure. No local build/tests run; this follow-up is newer than the running preview.6 build at 0f49aee.

## Reference folder navigation
- Gitflow prefix folders now use a shared ReferenceFolder component with keyboard-operable expand/collapse, branch counts, and aria-controls/expanded state. Reuses the current branch-item styling and original folder/chevron icons.
- Filtering opens the Gitflow section and temporarily exposes matching folder contents; clearing the filter restores each folder's previous collapse state. Prefix changes reset the corresponding folder identity.
- TODO: keyboard toggle, collapse with selected branch, name/SHA multi-term filtering through collapsed folders, filter clear restoring folder state, custom/empty prefixes and repository switch. No local build or runtime comparison performed.

- Folder state follow-up: keep existing prefix folders mounted but hidden when the search has zero matches. Clearing/changing the search therefore preserves the previous collapse choice instead of remounting an expanded folder. TODO: collapsed folder → zero-match search → clear → still collapsed.

## Command palette entry points
- Connected Gitflow, Start Feature/Release/Hotfix, and Gitflow Preferences to the existing command palette (Cmd/Ctrl+Shift+P). Start entries open the matching form after fresh repository/settings reads; they never create a branch merely by selecting a command.
- Busy/demo/write-block/missing-HEAD command states follow existing repository action availability. The form retains configuration/name/duplicate validation and native mutation confirmation.
- TODO: find commands with English/Korean keywords, open each start form, cancel without writes, invalid/uninitialized config, read-only repository, and Gitflow preferences navigation. Existing palette components/styles are retained. No local build/tests executed.
