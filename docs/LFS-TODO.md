# Git LFS implementation status

Reference: https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md (inspected 2026-11-22). Standard cache layout is lfs/objects/<first two>/<next two>/<SHA-256> under the shared Git directory.

## Implemented source, not runtime-verified
- Existing file viewer identifies canonical and legacy v1 pointer text.
- Native file-content reads now inspect the standard local object cache: missing, unavailable/unsafe path, size mismatch, hash mismatch, present but unverified, or verified bytes. This does not invoke git-lfs or install any tool.
- Reject non-directory/symlink cache components and non-regular/symlink object files under the existing repository filesystem policy. Only lowercase SHA-256 pointer IDs form paths. Custom/global storage is not searched.
- Verify size and SHA-256 up to 16 MiB; larger files report size-only presence. Never imply that this bounded inspection implements checkout or establishes future object integrity.

## Remaining required implementation
- Batch-wide fetch/pull/push orchestration. Single-object Batch/download/upload/verify are connected below; runtime verification remains pending.
- Checkout with destination/content preflight, backup/recovery journal, streaming data, executable mode and revision handling.
- LFS track/untrack attributes and staging semantics; do not bypass the current unsupported-filter write policy.
- Remote lock listing, acquire/release with ownership and stale-state handling.
- Custom storage/extensions policy, linked-worktree behavior, cancellation, disk-full and interrupted-operation recovery.

## Validation TODO
- Canonical/legacy/malformed pointers, unknown keys and oversized numeric values.
- Missing cache/object, empty object, correct object, wrong size/hash, >16 MiB object.
- Symlink/reparse/cache-directory substitutions, permission denial, object changed during inspection, linked repository common directory.
- Non-pointer file behavior and file-content response compatibility on macOS and Windows.

No local build or tests were run. Native inspection is later than the active preview.8 CI source at dafa0df.

## Basic-transfer protocol boundary
- Added single-object download/upload batch request encoding and response validation based on https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md.
- Requires matching SHA-256 ID/size and basic/sha256 transfer, distinguishes omitted upload actions (already present) from missing download actions, validates HTTPS action URL authority/path while retaining signed queries natively, bounds response/header sizes, rejects unsafe transport headers and duplicate case-insensitive names. Action values are sensitive and are not Debug/Serialize types.
- This parser is not yet connected to HTTP/UI. It grants no network access. Before use, the orchestrator must authorize each endpoint, bind credentials to it, validate expiry at send time (including full expires_at timestamp), stream and verify bytes, and publish atomically. Never forward a batch credential to another host or automatically retry an upload.
- TODO: mismatched/duplicate objects, non-basic transfer, server object errors, upload already present, missing download action, signed query, unsafe/duplicate headers, header/response limits, expired actions and full timestamp interpretation. No tests executed.

## Streaming cache publication
- Added a reusable native cache writer accepting a bounded-by-object-size reader, a cancellation callback, and a validated object identity. It uses 64 KiB chunks, detects short/extra data, verifies SHA-256, syncs the temporary file, and publishes without replacing an existing object.
- Existing/concurrently published objects must pass size/hash verification. Failure/cancellation drops the temporary file on normal unwinding; corrupted existing objects remain untouched. No worktree/index/ref mutation is performed.
- Reuses validated shared Git layout and rejects link/non-directory cache components. The repository's existing filesystem policy remains necessary; this is not a sandbox against concurrent hostile filesystem replacement.
- Not wired to HTTP/commands yet. Crash leftovers with the `.branchdesk-lfs-` prefix may remain; journal-based cleanup is still required. Windows power-loss directory durability remains unverified, consistent with existing sync_dir behavior.
- TODO: empty/large/chunked streams, early EOF/extra byte/hash mismatch, interrupted reads/writes, disk full, cancellation before/during/after verification, publication collision with valid/corrupt object, sync failure after publication, symlink/reparse paths, process termination. No local build/tests run.

## Action URL expiry
- Batch action parsing now creates a native monotonic deadline from expires_in, or a strict uppercase, second-precision RFC3339 expires_at when the relative field is absent. Relative expiry takes precedence as required by the Batch API.
- Added a request-time ensure_current guard for the upcoming transport. Reject malformed date/time/offset, impossible calendar dates, expired actions and unsupported leap-second timestamps; no URL/header values are reflected in errors.
- Existing endpoint-bound TransportCredential must remain bound to its exact approved URL; a transfer action on another host cannot inherit the batch credential. Transport integration remains pending.
- TODO: leap years, invalid month/day, UTC/positive/negative offsets, boundary expiry, precedence, elapsed monotonic deadline, malformed/non-ASCII timestamp, clock changes after parsing. No local build/tests executed.

## Native Batch/download transport primitives
- Added HTTPS-only Batch POST and object GET functions using existing endpoint-bound native credentials. Clients disable redirects, retry, proxy and Referer, with connection/whole-request timeouts. Errors expose status codes, not response bodies, signed URLs or tokens.
- Batch response size is bounded before protocol validation. Download checks exact separately approved action URL, expiration, credential/header conflicts, content encoding and declared length, then streams into verified cache publication.
- Batch credentials are not implicitly passed to object actions. Signed-query actions can use server-supplied headers; the existing credential binding continues rejecting query URLs rather than widening authentication scope.
- These functions are not yet reachable from a command/UI; no network request was made during development. Native access prompts, repository revision/operation ownership, cancellation wiring and progress remain necessary before enabling downloads. Upload/verify and checkout/locks still remain.
- TODO: scoped authentication, cross-host action approval, redirects, 401/403/429/5xx, oversized JSON, signed URLs, expired action, compressed response refusal, length/hash mismatch, timeout/cancel, disk full and atomic publication. No local build/tests executed.

## Single-object download integration
- File Content → Download LFS object opens a shared inline form using existing controls. An explicit HTTPS LFS base endpoint is required; no guessed endpoint, external analysis tool or git-lfs installation.
- The native command owns the operation gate, validates repository revision and object identity, confirms Batch and exact object destinations separately, and streams into the verified cache. Batch credentials are not forwarded to transfer actions. Signed URL/header authorization stays native.
- UI distinguishes cancellation, verified creation, verified existing object and failure. Duplicate submission is blocked; a failed attempt requires an explicit cache refresh before retrying. The latest cache inspection is labeled separately from the transfer result.
- Checkout is not performed. Byte progress, batch-wide actions and crash-leftover cleanup remain unimplemented. Cooperative in-flight cancellation is now connected below; native dialogs must still be dismissed and blocked network reads are bounded by timeout.
- GitKraken reference inspected: `../screenshots/official/0519f9b00328-lfs-actions-2025_2x.png` (LFS toolbar action menu). This file-scoped form is an implementation entry point; the reference's repository-wide fetch/checkout/pull/push/prune menu remains outstanding. No claim of full visual parity.
- TODO: file-view action visibility and disabled states; endpoint validation; cancel either prompt; private/public and cross-host objects; signed URL expiry during prompt; duplicate clicks; switching files while pending; existing corrupt/valid object; stale revision; timeout, disk full and process termination. Execute on macOS and Windows via isolated fixtures. No local tests/build or network transfers executed.

## Cooperative download cancellation
- Each download has a unique request ID. A native registry owns its cancellation flag and drops the active entry on completion/unwinding. Late IDs cannot cancel a different subsequent request.
- Cancel IPC validates the main window but deliberately does not acquire the operation gate held by the transfer. The original gate remains held until the worker actually exits; requesting cancellation does not enable concurrent writes.
- Cancellation is checked before/after authorization and Batch response, before the object request, after a blocking response/read, between chunks and before atomic publication. Normal cancellation discards the temporary file. Once publication succeeds, the outcome stays completed; cancellation does not delete an existing verified object.
- UI reports requested vs completed cancellation separately. Network reads may wait until the request timeout; native dialogs are not forcibly dismissed. Late IPC responses cannot overwrite another request's status.
- TODO: cancel before worker entry, native dialogs, Batch wait, GET headers/body, hash verification, sync/publication boundary; stale request ID and rapid cancel/retry; blocked request and timeout; normal temporary-file cleanup; frontend unmount; application close while cancellation is pending. No local tests/build executed.

## Refresh and retry in the file viewer
- Added Refresh cache status using the existing revision-scoped file-content read. It updates the same viewer only if the original content is still selected, so a delayed result cannot replace a newly selected file.
- Refresh re-reads the pointer and bounded cache status; it never contacts the server or changes files. Its successful completion re-enables explicit download retry without closing the file. A refresh error keeps the previous attempted state and is shown even with the download form collapsed.
- Large-object inspection remains size-only above 16 MiB; the separate successful transfer result still records full streaming hash verification performed during download.
- TODO: refresh after success/cancel/failure, changed pointer, deleted file, stale repository revision, file switching during refresh, repeated refresh clicks and global busy ownership. No local build/test executed.

## Single-object upload and server verification
- Added upload to the shared `LfsTransferForm`, using the existing FilterSelect and native authorization flows. Batch, upload and optional verification destinations require individual approval; no Batch credential is forwarded. No Git branch/index/worktree write or analysis-engine installation occurs.
- Source cache bytes are copied into an anonymous temporary file while checking exact size/SHA-256. Upload streams this frozen copy with Content-Length; memory is bounded, but temporary disk space equal to the object is required. No remote write starts before local verification completes.
- Implements Basic PUT and optional verification POST following [the upstream Basic Transfer API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/basic-transfers.md), inspected 2026-11-22. Uses the existing reqwest 0.13.5 [sized streaming body](https://docs.rs/reqwest/0.13.5/reqwest/blocking/struct.Body.html#method.sized). Redirects/retries remain disabled. Remote already-present, upload accepted and verification completed are distinct UI outcomes.
- Cancellation before PUT is safe cancellation. Interruption after PUT starts is an unknown remote outcome; a completed upload with cancelled/failed verification is explicitly verification-pending. Neither condition is silently retried. The form requires acknowledgement of remote-state review before enabling another upload; the durable journal described below now also enforces this across restarts.
- Reuses request-scoped cancellation. Blocked native dialogs/network calls still require dismissal or timeout. Verification response success takes precedence over a late cancellation; no remote deletion is attempted.
- TODO: cache missing/corrupt/source modified during snapshot; temporary disk exhaustion; anonymous/private Batch; remote already-present; independent signed/authenticated PUT/verify hosts; expired verify URL before and after PUT; zero/large objects; early server success; 200/201/204 upload responses; verify failure; cancel/timeout before/during/after PUT; restart with ambiguous remote outcome; byte progress and durable recovery journal. No local build/tests or remote upload executed.

## Durable upload outcome and explicit review
- Before object PUT, save and sync `branchdesk-lfs-upload.json` in the shared Git directory, using atomic publication and the existing OS-owned common-repository operation lease. The lease remains held through PUT and verify, excluding other FLOE processes/worktrees; it is released by the OS on process exit.
- Record only UUID, object SHA-256/decimal size, canonical base endpoint without query/userinfo, verification requirement and status. No signed action URL, authorization header or credential is recorded.
- Pending survives interruption; accepted is written after a successful PUT, verified after successful optional POST, and notSent for confirmed cancellation before PUT. Failure to persist an outcome is reported separately from remote success. Pending and accepted-with-required-verification block new uploads natively; malformed records also block rather than being overwritten.
- File-content inspection loads the shared last-upload record. The transfer form reloads it after an upload attempt, shows unresolved outcomes even after reopening, and provides native-confirmed acknowledgement. Acknowledgement changes only the local record to reviewed, never resends/deletes remote bytes or claims remote verification.
- A lease/record ID check prevents acknowledging an active or replaced record. Terminal/reviewed records may be replaced by the next explicit upload. This is a single latest-outcome record, not an audit history. Preflight errors before journaling still use the form's local retry acknowledgement.
- TODO: kill before/after intent sync, during PUT, after server acceptance/before local status, during verify; disk full at each save; stale acknowledgement ID; two processes/linked worktrees; invalid/symlink record; record changed externally; Windows directory durability/power loss; file switching while automatic record refresh completes. No local tests/build executed.

## Remote observation for interrupted uploads
- Last recorded upload now offers Check server status. The native prompt uses the recorded canonical endpoint and object identity, not the editable transfer form values. It sends only an Upload Batch metadata request; returned PUT/verify actions are discarded and object bytes are never resent by this check.
- Persist the server's already-present vs upload-requested response with a timestamp under the common operation lease. Re-read the full expected record before saving so a stale check cannot overwrite another operation's result. No action URLs or headers enter the record or renderer.
- Existing records without an observation remain readable. A failed/cancelled check does not replace prior evidence. Observations remain separate from pending/accepted/verified state: the server's claim is not an independent byte/hash check and does not automatically acknowledge an unresolved upload or authorize retry.
- Native-confirmed review remains available after observing the result. Record display is available in the pointer file view and the repository-wide recovery entry point described below.
- TODO: existing/missing/upload-requested object, auth refusal, malformed response, stale record after native prompt, concurrent process lease, disk-full while saving observation, old-record migration, repeated check with changed response, rejected network request and no object PUT/verify side effects. No local build/tests executed.

## Repository-wide LFS recovery entry point
- Added an LFS dropdown adjacent to Terminal, following the observed GitKraken toolbar/menu placement (`../screenshots/official/0519f9b00328-lfs-actions-2025_2x.png`). Checkout/pull/prune-all entries remain disabled; Fetch all and Push all are connected to the current-index batches described below. Upload recovery opens a working dialog; the command palette exposes the same entry point.
- The dialog reads the shared durable record without requiring the original pointer file to exist or be open. It supports reload, native-authorized server observation and native-confirmed review, using the same LfsUploadRecordView component as the file transfer form.
- Added a reusable ToolbarActionMenu with keyboard navigation, Escape/focus return, outside-click closure, disabled items and existing design tokens. No claim of full visual parity without rendered comparison.
- Reads remain bound to the opening repository revision. Stale repository errors require closing and refreshing the repository; the panel does not silently replace the active snapshot or clear existing file/draft state. Record action buttons are disabled after errors until reload.
- TODO: toolbar and command palette entry, missing/corrupt record, original file removed, worktree-shared record, keyboard menu/dialog focus, no duplicate initial read, busy gate, stale revision and failed reload, repository close/switch while the panel is mounted. No local build/tests executed.

## Current-index object inventory
- Repository LFS panel now lists recognized pointer blobs from regular stage-zero index entries, including unchanged tracked files. Scan in pages of 1,000 index entries; inspect ODB header before reading a candidate (<1,024 bytes), exclude conflict stages/symlinks/gitlinks/intent-to-add, count undisplayable UTF-8 paths, and recheck repository revision after reading.
- Page-local path filter and previous/next controls explicitly describe their scan range. Unstaged pointers and other revisions are outside this index view; empty pages do not imply no pointers in remaining pages.
- Selecting a pointer performs the existing bounded local cache inspection. The same LfsTransferForm provides explicit endpoint download/upload, scoped cancellation, cache/record refresh and recovery operations. No external git-lfs/REA/Ghidra tool is used or installed.
- Record-read failures are separate from inventory errors: downloads remain possible, but uploads are blocked by the malformed/unreadable journal. A repository-wide reload clears selection to avoid acting on a stale pointer.
- TODO: unchanged/staged pointers, duplicate object IDs at different paths, page boundaries and empty middle pages, filename encodings, stage conflicts and modes, oversized/non-pointer blobs, changed index during scan, selection/transfer/cancel/refresh, malformed journal alongside valid inventory, both platforms. No local build/tests executed.

## Whole-index object catalog
- Added an explicit cancellable scan through every current-index page, keeping the repository revision fixed. Report progress and publish a complete catalog only after the final page succeeds. Cancellation, changed page totals, invalid offsets and failures never produce a complete inventory label.
- Group identical SHA-256 objects across paths; conflicting declared sizes are rejected. Search all collected paths; render groups in 200-object pages with bounded path expansion. The first path supplies the displayed pointer metadata.
- Bound the collected catalog to 20,000 pointer paths / 16 MiB of path-and-pointer UTF-8 data; exceeding this produces an explicit error and leaves the existing paged view available. Non-UTF-8 path omissions are counted. This is a current-index catalog, not history-wide enumeration or automatic bulk transfer.
- TODO: multi-page empty scans, duplicates across boundaries, conflicting sizes, cancellation between/during reads, changing revision, limits, global filtering and object pagination. No local tests/build executed.

## Fetch all current-index objects
- Enabled the reference-position LFS Fetch all menu entry. It opens the repository panel and scans the current index; network requests begin only after entering an LFS endpoint and explicitly starting the fetch. This is not history-wide fetch or automatic checkout.
- The queue uses the complete deduplicated catalog, including paths hidden by a UI filter. Existing native server/action approvals remain required for each object. One frontend operation gate owns the queue; native per-object commands still validate repository revision and exact credential scopes.
- Show completed/downloaded/already-cached counts and the current path. A cancelled/failed object stops the queue before another request. Stop requests cancel the active native request and prevent the next object; normal component teardown does the same. Requests blocked on native dialogs/network reads retain the documented cancellation delay.
- Completed cache objects are retained. New fetch is an explicit action from the existing scan and re-verifies existing bytes; no automatic retry/resume or claim of rollback. A catalog-limit/scan failure does not enable whole-index fetching.
- TODO: deduplicated objects, zero/one/multiple objects, cancellation in/between requests and after final success, failure on object N, cache retained after interruption, stale revision, native prompt cancellation, component teardown, new fetch after failure, path filter scope, no checkout/index/ref changes. No local build/tests executed.

## Push all current-index objects
- Generalized the existing queue into LfsBatchTransferPanel, shared by fetch and push with the existing FilterSelect. The LFS Push all menu opens a whole-index scan with upload selected. It uploads deduplicated objects already in the standard cache; it does not push Git refs or scan all history.
- Each object goes through native snapshot/hash verification, separate endpoint approvals, durable upload intent and optional verification. Queue counts uploaded, server-verified and already-present outcomes separately. Stop, failed/uncertain upload and verification failure prevent the next object from starting.
- Reload the durable upload record after any batch result. Unresolved/unreadable records disable another upload batch; the native gate independently enforces review. Download remains available. Completed remote writes are not rolled back, and retry is always explicit.
- TODO: mixed already-present/upload/verify results, missing cache object after earlier success, cancellation before/during/after PUT, uncertain outcome on object N, record-write failure, native review followed by new batch, switching fetch/push mode after completion and hidden-filter paths. No local build/tests executed.

## Catalog path filtering correction
- Replaced the first-200-path cutoff with LfsObjectPaths: filter across every collected path before paginating the matching paths in groups of 100. Objects with a match beyond the first 200 paths now expose that matching path instead of only their unrelated leading paths.
- Keep rendering bounded and reset path pagination when the object/query changes. Clarified scan cancellation when no previous index page was loaded through the toolbar shortcut.
- TODO: a shared object with >200 paths, match only on the last path, case-insensitive filter, query change from a later path page, expanding multiple groups, and cancelled initial toolbar scan. No local build/tests executed.

## Checkout preflight inputs
- Added a read-only preview for a selected indexed LFS path. It validates repository revision, normalized/non-symlink path policy, regular stage-zero index mode, absence of conflict stages, bounded pointer blob and exact expected object ID/size.
- Reports exact pointer bytes, missing working file, differing bytes or unsupported file kind, plus index blob ID, executable mode and bounded cache inspection. A differing file may already be hydrated or locally modified; preview never treats it as safe to overwrite.
- Recheck repository revision after collecting the preview. No backup, file replacement, index write, network operation or permission change is performed. The UI explicitly leaves checkout execution unavailable.
- Required continuation: transactional streaming replacement, durable before/after record, recover/finish rules, ownership and lease checks, no-clobber missing-file creation, filter-specific write-policy handling, and full object verification immediately before publish. The preview is not execution authorization and must be revalidated at write time.
- TODO: exact/missing/modified/hydrated working file, symlink/directory/ancestor substitution, conflict stages, changed pointer or index revision, unsupported modes, absent/corrupt/large cache, executable flag and both platforms. No local build/tests executed.

## Unexposed checkout staging primitive
- Added internal preparation of a verified cached object into a private temporary directory while holding the repository operation lease. Capture the exact indexed pointer backup, original mode and a manifest; recheck revision, pointer bytes and mode before returning preparation. Share the hash/size-checked streaming copy with upload snapshots.
- This helper has no IPC entry point and performs no worktree replacement. Checkout execution remains disabled pending transaction publication, Git locking, filter policy and recovery integration. Normal drops remove temporary preparation; process-kill leftovers still need explicit recovery/cleanup design.
- TODO: absent/corrupt cache, insufficient space, cancellation, stale index, modified pointer/mode, preparation cleanup and interrupted process recovery on both platforms. No local builds or tests executed.

## Upload record ownership correction
- Revalidate that journal publication still targets the common repository protected by the operation lease, including after a remote operation. Refuse publication if linked-worktree/common-directory mapping changed.
- Outcome transitions compare the complete prior record instead of only ID/status, preventing silently overwriting changed object, endpoint or observation fields.
- TODO: change the common-directory mapping while awaiting an upload response; replace same-ID/status record fields; verify publication is refused and no retry occurs. No runtime tests executed.

## Fetch cache fast path
- Before any Batch request or network credential prompt, streaming-verify an existing standard-cache object at full size/SHA-256. Return already-present only after repository revision and cancellation are checked again. This makes explicit batch restarts reuse successfully downloaded objects without contacting the server.
- Missing cache paths proceed to the existing download flow; corrupt objects, unsupported directory/file kinds and unreadable paths fail without overwriting them or contacting the remote.
- TODO: offline cached fetch, mixed cached/missing batch, large object cancellation, zero-byte objects, corrupted bytes/size, symlink paths and revision changes during hashing. No runtime tests executed.
