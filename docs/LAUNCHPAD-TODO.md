# Launchpad repository management — 2026-11-22

Implemented: repository toolbar Browse / Clone / Init / New Workspace; New Workspace enters the task view and opens the existing workspace editor through the shared navigation guard. Existing editor drafts are retained. Creation selects the saved workspace through the existing persistence flow. Repository sections reuse TaskGroup for individual collapse and independent Collapse All / Expand All controls. Existing row action components and button styling are retained.

Reference inspected: `screenshots/official/e8631c247363-add-new-workspace-2025_2x.png` in the parent research workspace (GitKraken Repository Management), plus `259605735409-select-workspace-launchpad-2025_2x.png` and `7537e5a7be42-gkd-10-2-edit-workspace.png`. Reference captures are not packaged in the app.

## Deferred verification
- [ ] Browse / Clone / Init still open their existing dialogs.
- [ ] New Workspace opens the shared editor; saving persists and selects the workspace.
- [ ] Canceling a provider draft navigation prompt leaves the repository screen unchanged.
- [ ] An existing workspace editor draft is retained when reopening it from repository management.
- [ ] Loading, save failure, revision conflict and unavailable workspace settings prevent unsafe duplicate writes and keep errors visible.
- [ ] Collapse/expand controls affect repository groups independently of task groups; row actions remain reachable after expanding.
- [ ] Compare the rendered screen with the original reference; visual parity is not yet verified.

No local build or test run was performed for this batch. The in-flight preview packages the earlier release commit, not these subsequent UI edits.

## Local repository linking
The workspace management section now loads open repository paths when opened, uses the shared searchable FilterSelect, preserves an existing closed-repository mapping, and provides explicit loading/error/empty states. Changing links retains the existing revision-checked workspace save. Async reads are invalidated on unmount; linking is keyed by workspace and repository identity rather than row position.

- [ ] Opening management loads paths without requiring an initial manual refresh.
- [ ] Search, link, unlink, and switch workspace; confirm WIP results use the saved mapping.
- [ ] Closed mapped repositories retain their displayed path until explicitly unlinked.
- [ ] Load failure, retry, unmount during load, and repeated refresh do not apply stale responses.
- [ ] Save failure/revision conflict leaves the prior mapping visible and displays the existing error.
- [ ] Confirm rendered layout against references; this local-link panel is a FLOE-specific extension, not claimed pixel parity.

## General preferences → Launchpad
Reference inspected: parent research capture `screenshots/live/012-preferences-general.jpg`. Existing field order and shared SettingField layout are preserved. The Launchpad entry now saves the selected start view and other valid general preferences before opening Launchpad. Failed storage writes keep Settings open. Invalid branch/history-page values have explicit field/error states; unsaved changes can be reverted to persisted values. Persistence rejects unknown Launchpad view names.

- [ ] Choose each start view, Save and Open Launchpad, and confirm the selected page is shown.
- [ ] Block localStorage writes; confirm the error stays visible and Launchpad does not open.
- [ ] Invalid branch names and fractional/out-of-range history sizes disable both save actions.
- [ ] Discard unsaved changes restores persisted values without changing Git configuration.
- [ ] Confirm graph size, lazy-load and auto-prune existing consumers still receive saved preference changes.

Implementation only; no local builds or tests executed.

## Provider stale-state recovery
PR merge/reviewer and issue metadata/state preflight failures now lock further writes until an explicit item refresh, with specific recovery text. Refresh remains available for a stale description draft and uses the existing discard confirmation. Unknown write outcomes retain their stricter no-retry handling. Backend preflight checks are unchanged.

- [ ] Change the remote head/state/reviewers/labels/assignees after loading; rejected writes lock repeat submission until refresh.
- [ ] Keep editing in the discard prompt retains the draft and the stale lock; confirmed successful refresh clears the lock.
- [ ] Cancelled/failed reads do not unlock stale writes.
- [ ] Mergeability pending/conflicts give the correct recovery guidance without automatic retry.
- [ ] Update the older provider-write CI fixture to serve the merge preflight GET and subsequent write separately; current single-response fixture does not model that protocol. No test execution in this batch.

## Refresh failure preserves drafts
Item refresh no longer clears the existing detail tree before the request succeeds. Description, comment, metadata and child discussion drafts remain mounted during cancelled/failed reads. Successful refresh replaces the snapshot and clears accepted drafts together. Failed reads lock writes against the old snapshot; Refresh remains available while editing and still asks before discarding drafts.

- [ ] With description/comment/reply drafts, confirm refresh then cancel native read: old data and drafts remain.
- [ ] Repeat with network failure: old data stays visible; writes remain locked until successful refresh.
- [ ] Successful refresh replaces details and resets child drafts exactly once.
- [ ] Reject discard confirmation: no read is started and drafts stay unchanged.
- [ ] Initial detail-load failure still shows a retry action without claiming cached data exists.

Source review only; runtime verification remains deferred.

## Review reply submission
Reply editors now have a synchronous in-flight guard, sending state, handled rejected promises, and explicit success/unconfirmed messages. Failed/unconfirmed submissions preserve text. Closing an editor retains its draft and shows Continue reply; Copy reply draft allows recovering text while the parent panel is locked. Success clears only the submitted draft and requests manual discussion refresh; no write is automatically retried.

- [ ] Rapid submit/Enter triggers one reply request.
- [ ] Rejected promise and unconfirmed/cancelled write keep text and clear sending state.
- [ ] Successful reply clears text and shows refresh guidance.
- [ ] Cancel/continue preserves text; global navigation still detects the hidden draft.
- [ ] Unknown-outcome parent lock prevents resubmission while permitting draft copy.

No local build/test execution for this change.

## Discussion pagination protects reply drafts
Refreshing or loading another code-comment page now checks active/hidden reply drafts through the existing draft-presence callbacks. The shared discard dialog is shown before replacing a thread list. Confirmed drafts reset only after a successful read; cancelled/failed reads leave editors and text mounted. Parent item navigation continues receiving the same dirty state.

- [ ] Draft a reply on a later page; Refresh and Keep editing preserve page and text without a request.
- [ ] Confirm refresh, then cancel/fail the native read: draft remains and navigation still warns.
- [ ] Confirm successful refresh: removed/replaced threads no longer retain abandoned drafts.
- [ ] Load more with a draft uses the same confirmation; no repeated fetch on double click.
- [ ] Parent item refresh and close continue recognizing drafts in collapsed editors.

No local builds or tests run; visual/runtime validation pending.

## Closed linked repositories in WIPs
- WipTable now exposes Open Repo for a mapped repository in the `notOpen` state. The workspace WIP view passes this through Launchpad's existing draft-protected navigation to the existing native repository-open flow.
- Already-open entries keep View Repo. Unmapped entries do not invent a path, and missing/error states do not silently trigger opening. Existing UI controls/table styling are reused.
- Reference inspected: `screenshots/official/7575643d61d4-launchpad-highlight-2025_2x.png` for Launchpad composition. This local WIP action is a FLOE extension, not a claim of pixel parity.
- TODO: closed mapped repository → Open Repo → native approval → repository tab; cancel approval; missing/moved folder; dirty provider draft and keep/discard navigation; disabled state; unmapped rows; returning to WIPs and refreshing the open state. No local build/tests executed. Later than preview.6's source commit.

## WIP initial loading and visibility
- Shared useInitialVisibleRead now supports a changing scope key. Named workspace WIPs perform one initial read after workspace management is idle and metadata-only filters are cleared; failure requires explicit Refresh.
- Personal WIPs no longer clear results and re-read whenever unrelated parent busy state toggles. Only repository scope changes reset the list; manual Refresh remains available.
- Workspace WIPs offer Show clean repositories, matching the existing personal view. Clean entries with warnings remain visible even when this is off, so recovery/read-only warnings do not disappear.
- TODO: first entry, management-busy transition, workspace local-link changes, filter clearing, failed read without automatic retry, parent busy toggle without duplicate reads, clean-with-warning visibility, stale response after scope change, and manual retry. No local build/tests executed.

## Integrations preferences: GitHub repository access
- Enabled Integrations in Preferences with the reference provider-list/detail composition, shared ScreenNavigation/SettingsSection/SettingField/FilterSelect, and the existing GitHub remote suggestions. Reference: `screenshots/official/c4ed03a1e201-gkc-github-enterprise-server-integration.png`.
- GitHub.com Check access reads the first repository branch page through the existing native authenticated metadata command and App operation gate. Tokens stay in native credential UI/storage; this page never receives them. No automatic request is sent on mount.
- Success is labeled repository read access only, not persistent account connection or PR write permission. Cancellation and failure stay distinct. Other providers remain explicitly unavailable; OAuth/account management/disconnect are not implemented by this change.
- Added preference search for the GitHub repository field. TODO: valid/invalid scope, remote suggestion, native cancel, authentication failure, successful empty/paged branch list, read-only token, double submit, settings close during request, platform credential availability, reference comparison. No local build/tests run.

## Forget a saved GitHub API credential
- Added a main-window-only native command and macOS Integrations action for removing the Keychain entry bound to the exact validated GitHub API repository URL. Reuses credential_vault deletion and OperationGuard; a native dialog identifies the scope before deletion.
- Does not revoke the server token, change Git state, touch Git transport credentials, or return secrets to the webview. Missing entries are idempotent success. Windows persistence is unsupported, so no deletion action is offered there.
- TODO: save credential for two repository URLs, delete one and verify isolation; no existing entry; native cancel; Keychain denial; malformed owner/repo; wrong window; busy rejection; changed form input blocked during request. No credentials were deleted during development, and no local build/tests executed. New command compilation/permission generation remains for CI.

## Integrations → Launchpad handoff
- Open Launchpad from GitHub Integrations prefills the validated personal PR/Issue repository scope on a new Launchpad mount. Default view and saved workspace selection are preserved; users can select Personal to use that scope.
- If Launchpad is already open behind Settings, Return to Launchpad preserves its existing scope, view and drafts instead of forcing a remount or retargeting requests.
- No credentials enter this handoff and opening the screen does not claim remote write permissions. TODO: manual valid scope, remote suggestion, malformed scope rejected, normal Launchpad entry after close, existing draft retention, saved workspace versus personal view. No local build/tests executed.
