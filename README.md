# FLOE — local development build

macOS / Windows 공통 Tauri 2 + React + Rust 앱. 현재 **로컬 개발용 alpha**이며 배포 준비가 끝난 제품이 아닙니다.

## 실행

```sh
npm ci
npm run tauri dev
```

Rust 도구가 PATH에 없으면 `export PATH="$HOME/.cargo/bin:$PATH"`를 먼저 실행합니다. `.node-version`과 `rust-toolchain.toml`, 두 lockfile에 개발 환경을 기록했습니다.

macOS 빌드: `npm run tauri build -- --bundles app`
Windows 빌드: Visual Studio C++ Build Tools와 WebView2가 있는 Windows에서 `npm run tauri build -- --bundles nsis`. Windows용 설정 파일은 준비했으나 이 Mac에서 Windows 실행 검증은 하지 않았습니다.

## 구현된 동작

- 네이티브 폴더 선택 → 로컬 저장소 신뢰 확인 → snapshot 조회
- 부모 OID 기반 그래프, branch/tag 표시, 커밋 검색·메타데이터
- staged/unstaged 파일 목록, 일반 텍스트 diff
- 커밋의 전체 메시지·변경 파일·파일별 diff; 최초 커밋은 빈 트리, 병합은 첫 부모와 비교
- 파일 단위 및 전체 stage/unstage, commit, HEAD에서 branch 생성
- 병합된 비활성 branch 삭제, 경량 tag 생성·삭제, 기존 주석 tag 조회
- 변경 없는 regular-file 작업 트리의 branch checkout 및 중단 복구 (백업 보관)
- 모든 refs와 stash reflog를 snapshot revision에 포함하여 외부 ref 변경 감지
- stash 목록 및 staged/unstaged/untracked 세 영역의 파일·diff 조회 (현재 작업 파일 수정 없음)
- 네이티브 쓰기 확인, 변경 revision 재검사, Git 표준 index 잠금
- 다크/라이트 설정, 실제 저장소와 구분한 샘플 화면

AI/Agents 버튼은 사용자 지시에 따라 비활성화되어 있으며 구현 대상에서 제외합니다.

상단 Pull/Push/Terminal/Undo, 일부 사이드바 항목은 **외형만 연결**되어 있으며 클릭 시 미구현 상태를 알려줍니다. GitKraken의 전체 기능 구현을 완료했다는 의미가 아닙니다. 전체 stage/unstage는 한 번의 네이티브 확인 후 index.lock 안에서 준비하고 완성된 index를 한 번 교체합니다. 실패 시 index 일부만 반영하지 않으며 작업 파일 내용은 변경하지 않습니다. OS 경로 경쟁에 대한 완전한 격리를 의미하지는 않습니다.

## 원본 화면 리소스

사용자 요청에 따라 원본 GitKraken 12.7.0의 **CSS 파일, Open Sans 폰트, 정적 이미지와 아이콘 경로 데이터, 테마 값**을 로컬 설치본에서 가져왔습니다. 이전 자체 디자인은 `../analysis/previous-ui/`에 보관했습니다.

- `public/vendor/gitkraken/src/css/styles.css`: 원본 바이트 그대로 로드
- `public/vendor/gitkraken/provenance.json`: 파일별 출처·해시
- `src/original-icons.json`: 렌더러에서 정적으로 추출한 원본 SVG path 37개
- `src/original-theme.css`: 원본 테마 변수 270개
- `scripts/extract-visuals.mjs`: AST 리터럴만 추출. renderer 실행 없음
- `scripts/install-original-visuals.py`: 설치된 app.asar에서 정적 리소스만 복사

레이아웃 JSX와 Rust backend는 새로 작성했습니다. 원본 렌더러 JS를 실행하거나 로그인·라이선스 로직을 가져오지 않았습니다. 원본 전체 DOM/모든 화면을 그대로 추출한 완성본은 아닙니다. 현재 메인 화면을 우선 검증하며 아바타와 일부 상태·아이콘 배치는 추가 정합 작업이 남습니다. 원본 리소스의 출처·권리 표기를 유지하며 공개 배포용 자산 결정은 별도 작업입니다.

## 보안 범위

외부 Git 프로세스·shell 실행과 네트워크 기능이 없는 libgit2 기반 초기 어댑터입니다. 미신뢰 저장소 sandbox는 아직 제공하지 않습니다. 본인이 관리하는 일반 로컬 저장소만 엽니다.

- 링크/junction, bare/linked worktree, alternates, include/extensions 설정 등은 거부
- 속성/필터/서명/실제 훅이 있는 저장소는 쓰기 차단
- 시스템·사용자 Git config는 사용하지 않고 명시적 빈 config로 교체
- 렌더러에 fs/shell/http 범용 권한 없음; 등록 창·origin과 opaque repo/file ID 검사, 커밋 조회는 현재 그래프의 OID로 제한
- 동시 요청 제한, 상태 fingerprint, index.lock 소유권 처리
- 500 commit / 2,000 changed file / 파일 5 MiB / 변경 합계 50 MiB / 트리 깊이 64 제한

파일 검사와 libgit2 동작 사이의 OS 경로 교체 경쟁에 대한 완전한 핸들 격리, 파서 process sandbox·강제 취소, 악성 pack fuzz, 배포 서명·updater·vault는 미구현입니다. 따라서 설계 문서의 보안 출시 게이트를 통과한 상태가 아닙니다. 사용자/전역 Git 설정을 무시하므로 필터·서명 등의 기존 개발 환경과 동등한 의미를 보장하지 않습니다. 이 제약은 다음 엔진 개발 단계에서 해결해야 합니다.

## 검증

```sh
npm run check
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm audit
cargo audit --file src-tauri/Cargo.lock
```

현재 Rust 시험 등록 67개(하위 프로세스 실행용 helper 1개 포함), 프런트엔드 시험 6개(그래프 3개, AI 제외/참조 일관성 2개, stash 영역 선택·HTML 이스케이프 1개). 테스트는 임시 저장소를 사용합니다. 원래 촬영 fixture의 충돌 상태를 변경하지 않습니다. `tests/native-fixture/`는 별도 네이티브 smoke용 합성 저장소입니다.

자세한 실제 결과·미검증 상태는 `../reports/development-validation.json`, 의존성 결과는 `../reports/npm-audit.json`, `../reports/cargo-audit.json`에 기록합니다. cargo-audit의 취약점 수와 유지보수/unsound 경고는 구분해야 합니다.

## 지속 구현 기준

[기능별 합격 시나리오](../docs/design/07-acceptance.md)와 [기능 상태](../reports/feature-status.json)에 따라 비 AI 기능을 계속 구현합니다. Windows 또는 출시 게이트가 미실행이면 해당 기능을 제품 전체 완료로 표시하지 않습니다.

참조 변경은 HEAD·현재 branch·대상 ref를 잠근 뒤 snapshot을 다시 확인하고 단일 ref만 수정합니다. libgit2의 다중 ref transaction은 rollback 원자성을 보장하지 않으므로 여러 ref 변경을 원자적이라고 가정하지 않습니다. 브랜치 삭제는 현재 HEAD에 병합된 비활성 branch만 허용하며 강제 삭제는 지원하지 않습니다. 경량 및 주석 tag를 생성하고 기존 tag를 조회·삭제할 수 있습니다.

[작업 파일 변경·복구 프로토콜](../docs/design/08-worktree-recovery.md)은 checkout/stash writer의 구현 계약입니다. clean checkout과 오류 반환 후 복구를 구현했습니다. manifest 기록 후 백업 준비 중단과 실제 프로세스 강제 종료·재복구 시험을 추가했습니다. dirty checkout은 아직 미구현이며, 최초 manifest 게시 이전 중단·power-loss 및 Windows 실행 검증은 남아 있습니다.

Checkout 백업은 `.git/branchdesk-checkout`에 기록하고 완료 시 `.git/branchdesk-checkout-completed-<uuid>`로 보관합니다. 완료 백업의 자동 정리는 아직 없으며 반복 작업 시 디스크를 사용합니다. 중단 기록이 있으면 추가 쓰기를 차단합니다. 복구 시 원본/목표와 다른 파일·index·ref를 발견하면 덮어쓰지 않고 중단합니다. 정상 오류 반환 외에 실제 하위 프로세스 강제 종료도 시험합니다. OS lease로 살아 있는 작성자를 차단하고, 복구 기록과 파일 식별자·내용이 모두 같은 Git lock만 정리합니다. 복구 중 재종료도 시험했습니다. 완성된 manifest 이전의 중단과 악의적 경로 교체 경쟁까지 해결한 것은 아닙니다.

Windows adapter 컴파일 확인: `cargo check --locked --manifest-path tests/platform-lease-check/Cargo.toml --target x86_64-pc-windows-msvc`. 이 probe는 실제 `repository/lease.rs`를 포함하여 API를 검사하지만 Tauri 전체 빌드·Windows 실행을 대신하지 않습니다. Rust 최소 버전은 File::try_lock 사용에 따라 1.89이며 고정 개발 toolchain은 1.99.1입니다.

Stash details의 Drop stash는 OID를 재확인한 뒤 선택 항목만 삭제합니다. 중복 OID는 현재 모호한 선택으로 거부합니다. `.git/branchdesk-stash-drop/record.json`에 원본/목표 reflog와 ref를 기록하며, 중간 오류 후 Recover operation은 승인된 삭제를 마무리합니다(checkout 복구는 원상복원). 원본 또는 목표 상태와 다른 reflog/ref는 보존합니다. stash drop은 반환 오류 단계 시험을 통과했으나 실제 kill 후 남은 lock의 자동 정리, disk-full/power-loss, Windows 및 native UI 클릭 검증은 아직 남아 있습니다. 완료 기록은 로컬 archive로 보관합니다.

Stash details의 Apply stash는 현재 HEAD와 stash base/저장 tree의 3-way merge를 사전 계산하고 staged 복원 옵션을 지원합니다. untracked 파일도 복원하며 stash 자체는 유지합니다. 현재 clean worktree만 지원하고 dirty/detached 상태 및 병합 충돌은 쓰기 전에 거부합니다. `.git/branchdesk-stash-apply`가 남으면 Recover operation으로 원래 작업 파일과 index를 복원합니다. 원본/목표와 다른 파일·index·HEAD는 보존하고 복구를 거부합니다. 반환 오류 4개 경계는 검증했으나 실제 kill 잠금 정리, 준비 중단, power-loss, native UI 및 Windows 실행은 아직 미검증입니다. 임시 목표 tree가 Git GC로 제거되면 복구가 차단될 수 있어 장기 객체 보존 정책도 남아 있습니다.

Pop is now connected in the toolbar (latest stash) and stash details (selected stash). It verifies the applied files/index/HEAD before dropping the same OID. Conflicts retain the stash. An error after apply but before the drop journal preserves both applied state and stash; it does not auto-replay apply. Once the drop journal exists, recovery finishes drop only. Duplicate OIDs are rejected before apply. Native clicks, actual kill and Windows remain unverified.

Stash toolbar now opens a save dialog with an optional message and Include untracked files. Save builds index/worktree/untracked objects, pins a recovery commit under refs/branchdesk/checkpoints/, publishes refs/stash, then clears only captured changes. An interrupted save rolls back original files/index and retains any already-published stash. Checkpoint refs and archives currently have no retention cleanup. Save supports ordinary attached repositories; keep-index, staged-only, patch selection, ignored-file inclusion, hard-kill recovery and Windows/native UI validation remain open.

Stash crash-recovery update: macOS real child-process termination is now tested at 5 save, 4 apply and 4 drop boundaries, plus three repeated-recovery termination boundaries per operation. Recovery holds the kernel lease, validates recorded lock identity and bytes, and only then removes matching markers. Replaced/forged/duplicate lock records preserve files and stop recovery. Earlier unverified hard-kill notes are superseded only for these tested boundaries; acquisition/record gaps, libgit2 internal publication, power loss and Windows remain open.

Native verification update: macOS stash save and apply roundtrip passed for tracked staged/unstaged content; Git independently confirmed exact content and stash retention. Mutation cancellation now shows a normal notice and retains selection. See ../reports/native-stash-flow.json and captures 045-047. Native pop/drop execution and other stash options remain unverified.

Create tag now offers Annotated tag with message, tagger name and email. The backend creates an annotation object first, then publishes the single locked tag ref after checking the selected commit and revision. Duplicate/invalid names, invalid payloads, stale selections and foreign locks are rejected. Tests verify exact message/tagger/target, peeled graph labels, and worktree/index/HEAD preservation. Native annotated-tag creation, tag-message inspection UI, signed tags and Windows runtime remain open.

파일 diff의 History 버튼에서 커밋별 파일 이력과 당시 diff를 조회합니다. 첫 번째 부모를 따라가며 내용이 동일한 rename을 추적하고, 삭제 후 같은 경로에 새로 추가된 파일은 별도 이력으로 취급합니다. 페이지당 100개, 최대 10,000개 커밋을 검사합니다. blame·전체 DAG·유사도 기반 rename·네이티브 이력 화면 및 Windows 검증은 남아 있습니다.

History에서 커밋을 선택한 뒤 Blame을 누르면 당시 파일의 줄별 작성자·커밋·원본 경로를 확인할 수 있습니다. 첫 번째 부모 기준이며 작업 중인 내용은 포함하지 않습니다. UTF-8 텍스트 최대 5 MiB/10,000줄, 최대 10,000개 조상 커밋을 지원하며 이력 경계 도달 시 표시합니다.

File 보기는 선택한 작업 파일/index/커밋/이력/stash 영역의 실제 내용을 표시합니다. 삭제·이진·비 UTF-8·5 MiB 초과 파일은 상태를 표시하며 텍스트 미리보기는 최대 512 KiB/10,000줄입니다. 표시 제한과 파일 끝 개행 여부를 구분합니다.

Diff 화면에서 Unified/Split을 전환할 수 있습니다. Split은 연속 삭제·추가 블록을 순서대로 짝짓고 각 파일의 실제 줄 번호를 표시합니다. 단어 단위 매칭은 아직 없으며, 해석할 수 없는 patch는 통합 보기로 돌아갑니다. 최대 10,000행을 표시합니다.

Open repository는 폴더 선택 또는 절대 경로 직접 입력을 지원합니다. 폴더 선택이 동작하지 않으면 경로를 입력할 수 있습니다. 두 방식 모두 신뢰 확인과 동일한 저장소 검증을 거치며, 열기 실패 시 기존 저장소를 유지합니다. 경로의 ~·환경 변수·따옴표는 확장하거나 제거하지 않습니다.

LOCAL 브랜치의 Preview merge에서 현재 브랜치로 병합할 때의 fast-forward 여부, 자동 병합 diff, 충돌의 Base/Current/Incoming 경로와 객체를 조회합니다. 원본 저장소를 변경하지 않도록 계산용 객체는 임시 저장소에 보관합니다. 현재는 미리보기 단계이며 병합 적용·충돌 해결·continue/abort는 아직 연결되지 않았습니다.

Clean fast-forward merge is available from a local branch’s merge preview. It preserves the current branch and uses the existing interrupted-operation recovery control to restore a checkpoint. Merge commits and conflict resolution are pending. Latest local checks: 91 Rust registered tests (one harness), 21 frontend tests; macOS native merge button and Windows runtime remain unverified.

Merge preview now supports a true two-parent merge commit for clean, diverged histories with one merge base. Enter message, author and email; conflict resolution/continue/abort are still pending. Recovery retains both parents through a checkpoint ref. Native merge UI and Windows remain unverified. Latest checks: 94-test full Rust suite plus one new focused test, 22 frontend tests.

Conflict preview now opens Base, Current and Incoming file contents through validated stage selections. Resolution storage and continue/abort are pending. Latest full Rust run: 96 tests (one harness); frontend: 23. An intermittent stash apply assertion remains unresolved despite a passing full rerun and 60 focused repeats. Native conflict UI and Windows remain unverified.

Conflict resolution now supports durable drafts with current/incoming/manual text/delete choices, restart/resume, Continue and Abort. Continue creates a two-parent commit through the recovery journal; Abort preserves external file edits. Drafts keep the live index/worktree untouched until Continue. External Git conflict-state interoperability, rename conflicts, native UI, Windows and draft-specific actual-kill tests remain pending.

Draft crash hardening now covers 22 actual process-kill checkpoints across save/abort/continue, including rollback and retry. Corrupt regular JSON drafts can be archived without altering worktree/index. Latest checks: full 102 Rust suite plus new actual-continue-kill test (103 registered), 24 frontend tests. Native/Windows and remaining interruption gaps are tracked in the acceptance document.

External Git merges are now inspected from MERGE_HEAD and unmerged index stages. Read Base/Current/Incoming, edit the working file externally, then stage its resolution; other conflicts and merge metadata are preserved. Index rollback is journaled. External Continue/Abort remain pending, as do native/Windows and external-journal actual-kill tests. Latest checks: Rust 108, frontend 25.

External Git merge Continue now commits the resolved index with both parents and preserves unstaged edits. Journaled roll-forward recovery reuses the recorded commit and finishes merge metadata/AUTO_MERGE cleanup; five real process-kill checkpoints passed. External Abort remains pending. Full checks: Rust 112 (one harness), frontend 26. Native UI and Windows remain unverified.

External Abort is now connected: it backs up and restores merge-affected files/index to HEAD, preserves unrelated local edits, and resumes from recorded interruption points. Eight actual process-kill checkpoints passed. Extra unstaged edits on affected staged files and MERGE_AUTOSTASH are refused. Full checks: Rust 117, frontend 27; macOS bundle rebuilt. CLI parity, native/Windows and advanced merge cases remain open.

The Rust test suite now requires Git CLI for six isolated differential tests of external merge Stage/Continue/Abort. These cover text/deletion/binary results, unrelated dirty files, rename/rename Abort and Unix modes. Product Git operations still use libgit2. External Continue now terminates commit messages with LF. Full Rust 123 tests passed; native UI/Windows and advanced merge cases remain open.

Modified text files now support per-hunk Stage/Unstage in Unified and Split diffs, with guarded index rollback and actual process-kill/repeated-recovery tests. Working files and unrelated staged entries are preserved. Added/deleted files, mode changes, line selection, native hunk UI and Windows remain outside this milestone.

Hunk additions/deletions extension: regular text additions (including untracked workdir files) and deletions now expose Stage/Unstage hunk controls. Untracked patches are normalized into additions by parsing only the backend-generated bounded patch. Deletion removes only the selected path from the original index; inverse staging restores that file from HEAD. A new/deleted file has one whole-content hunk; this is not line selection. Tests cover literal bracket paths, missing final newline, CRLF worktree preservation, round-trip index presence/content, unrelated staged files, and real process termination after index publication for both add/delete followed by exact-index rollback. Binary, mode-changing and empty-file patches still have no hunk controls (whole-file operations remain available). Unborn HEAD, native UI, Windows runtime, initial journal gaps and power-loss/disk-full remain open.

Unborn repository index support: index.lock can now contain a valid empty Git index v2 when HEAD is a valid unborn branch and the live index is absent. Opening/snapshotting alone does not create index. Existing committed repositories with a missing index remain rejected. Hunk operations lock the symbolic HEAD target, diff staged data against an empty tree for unborn HEAD, and preserve the symbolic branch. Whole-file/hunk stage and unstage plus an initial zero-parent commit are tested.

The shared index journal records whether index was originally absent (optional field defaults false for prior records). Recovery validates unborn HEAD identity and permits only the recorded post-index or true absence, then restores absence without changing working files. Empty foreign index and changed unborn branch are rejected. Six initial/repeated-recovery process-kill scenarios cover both index publication boundaries and both rollback boundaries, with retry after recovery. Detached hunk recovery exposed duplicate HEAD.lock ownership records; generation now deduplicates lock paths, while recovery continues rejecting duplicate records as invalid. Initial acquisition/record gaps, power loss/disk-full, native UI and Windows remain open. Ordinary whole-file index writes and initial commit do not gain the hunk journal's crash guarantees from this change.

Whole-file index crash recovery: Stage, StageAll, Unstage and UnstageAll now hold HEAD/current-branch reference locks, revalidate revision and publish through the same before/after index journal as hunk staging. Index lock is retained through publication, then dropped before journal archival. Pending work uses the existing recovery control to restore the exact prior index (or absence for an unborn repository). Working files are never rewritten by this recovery.

Coverage added: all four actions killed before and after index publication (eight cases), batch stage/unstage killed again at both recovery publication boundaries (four scenarios), first stage in an unborn repository killed at both publication positions with rollback-to-absence and successful retry, and refusal of subsequent external index edits. Post-interruption worktree edits and binary bytes remain unchanged. Initial lock acquisition/journal-record gaps, archive transition, power loss/disk-full, index-object retention, native UI and Windows remain open. Initial/ordinary commit writes still do not have this journal; the prior note about non-journaled whole-file operations is superseded here.

Ordinary commit journal: commits on an existing branch, an unborn branch and detached HEAD now create the commit object without updating a reference, retain it under a checkpoint ref, record the expected HEAD target/prior OID/exact new OID/index digest, then publish through a locked ref transaction. Recovery validates the saved commit parents and tree against the unchanged index, verifies symbolic/detached HEAD identity and old-or-new ref state, and reuses the same commit. It preserves working-file edits and index bytes. Commit recovery appears through the existing pending-operation control; no new UI rendering is claimed.

Actual process-kill coverage: three HEAD modes x two publication checkpoints x (normal recovery or a second kill at either recovery checkpoint) = 18 scenarios, 30 kills. All require the exact recorded resulting OID, expected parents/staged blob, original index, later worktree edits and released owned markers. Changed HEAD/index recovery is refused; invalid NUL/newline identity/message input is rejected before journal creation. Normal stage/commit/unstage also remains covered. Initial checkpoint/record/ownership gaps, internal libgit2 ref/reflog writes, archive transition, retention, native UI, Windows, power loss and disk full remain unverified. Amend remains pending.

Amend current HEAD: existing composer checkbox now enables message-only or staged-content amend and preloads the actual HEAD message when the composer is empty. Snapshot exposes headOid so branch traversal order cannot select an unrelated commit. No HEAD/read-only/busy states disable the control; native confirmation explicitly says the last commit is replaced. Identity fields supply committer; original author identity/time and zero/one/multiple ordered parents are preserved. Successful mutation or changed snapshot resets amend mode. AI remains disabled.

Backend AmendCommit uses the existing locked commit journal with a backward-compatible amend flag. Recovery validates original author and old commit parents, accepts the recorded old/new ref state and reuses the exact amended OID. Both previous and replacement commits receive retention pins. Tests cover root/single-parent/merge commits with staged or message-only changes, original author, new committer, unchanged index/worktree, old-commit reachability, absent HEAD/stale request refusal, and four ordinary/detached actual publication-kill scenarios each followed by a second recovery kill. UI tests verify writable-HEAD gating and replacement/author wording. Native UI, Windows, independent CLI amend parity, reset-author option, general message cleanup/signature semantics, initial record/internal ref gaps and archive retention remain open.

Independent commit/amend Git comparison: isolated Git CLI fixtures compare root/single-parent/merge/detached amend, staged and message-only modes, simple and whitespace-heavy messages (16 combinations), plus initial/ordinary new commits (two combinations). Compared staged tree, ordered parent OIDs, original author identity/date for amend, committer identity, complete message, stage entries, status, working files, detached state and HEAD reflog tip. Commit OIDs/committer timestamps intentionally differ; full reflog messages/history are not compared.

The first independent test found missing final LF in app commit/amend messages. The implementation now uses libgit2 message_prettify without comment stripping to match default non-editor git commit -m whitespace cleanup: trim surrounding blank lines, trailing whitespace, collapse repeated blank lines and terminate with LF while retaining # text. Before-fix failure is retained in reports/amend-parity-before.log. All eight Git parity tests passed after the fix, including the original six merge tests. Custom cleanup modes/configuration, editor templates, signed commits, reset-author, native UI and Windows remain outside this evidence. Existing external merge Continue has only its earlier LF fix; whitespace-cleanup parity for that separate path is still pending.

Native macOS amend/hunk validation: app window now accessible; Desktop repository config open waited for TCC DesktopFolder authorization, without a prompt in the captured app window. No OS permissions changed. Synthetic copy in /private/tmp opened normally. Message-only amend through checkbox/native confirmation preserved original author and root parents, used new committer and updated message. Split view staged only second hunk; Unified unstaged it, restoring original index blob while preserving both worktree edits.

Real UI defect found: imported original `.hunk button{display:none}` hid Unified actions despite backend/component tests passing. Scoped app override now displays its hunk button; header width is bounded. Verified on rebuilt app through successful Unstage. Captures052–054, reports/native-amend.json and native-open-sample.txt preserve evidence. Two descriptor/CLI-index tests added; full Rust149 and Clippy passed, final macOS22.94MiB built. CLI fixture test reran after inherited GIT_* cleanup. Frontend behavior unchanged from29-test pass; CSS checked natively. Windows, remaining native variants, DesktopFolder permission-wait UX and native picker remain open.

Repository open wait UX: after ten seconds with an operation still pending, the open dialog explains that folder/confirmation windows or macOS file-access prompts may need attention. This is a status hint, not a timeout: it never cancels/retries/releases the native operation or claims completion. Controls remain disabled until the real promise settles; Escape no longer closes dialogs during an active operation. The timer is cleared on completion/replacement, preventing old operations from producing later hints. Three frontend tests cover delay/cleanup and pending-state wording/disabled controls; frontend32 passed. This does not resolve the OS DesktopFolder prompt itself or implement cancellation of a blocked file open. Native long-wait interaction, OS permission handling and Windows remain open.

새 저장소: Open repository의 Create new에서 **아직 없는 폴더의 절대 경로**와 초기 브랜치를 입력합니다. 부모 폴더는 미리 존재해야 합니다. 기존 경로는 덮어쓰지 않으며 생성 실패 시 현재 저장소를 유지합니다. 생성 도중 실패한 새 폴더는 확인용으로 남을 수 있습니다. 첫 커밋 전에도 지정한 브랜치 이름을 표시합니다. 생성 UI 네이티브 조작 및 Windows 실행은 아직 검증하지 않았습니다.

후속 검증: macOS 실제 앱의 생성·취소·기존 폴더 거부·첫 stage/commit을 확인했습니다(reports/native-init.json). 앞서 기재한 네이티브 생성 UI 미검증 상태를 갱신합니다. Windows 실행은 미검증입니다.

최근 저장소: 성공적으로 열거나 생성한 저장소는 앱 데이터 폴더에 최대 20개까지 최근순으로 저장됩니다. Open repository에서 목록 항목을 선택하면 기존 확인 절차를 거쳐 다시 엽니다. Remove는 목록에서만 제거하며 파일을 삭제하지 않습니다. 목록 파일 오류가 있어도 직접 경로로 열기는 사용할 수 있습니다. 네이티브 목록 UI와 Windows 검증은 남아 있습니다.

복수 저장소 탭: 최대 12개 저장소를 동시에 열 수 있으며 같은 경로 재열기는 기존 탭을 재사용합니다. 전환 시 실제 상태를 다시 읽고 커밋 초안을 메모리에 보관합니다. 초안이 있는 탭 닫기는 확인을 거치며 실제 저장소 파일과 인덱스는 그대로 유지됩니다. 앱 종료 후 탭/초안 복원과 탭별 diff/스크롤 복원은 아직 구현하지 않았습니다. macOS 기본 전환·초안 복원·닫기 취소·비활성 탭 닫기·최근 목록 재열기는 실제 UI에서 검증했습니다.

로컬 브랜치 Rename: LOCAL 목록의 연필 버튼에서 새 이름을 입력합니다. 동일 이름·기존 대상·경로 충돌은 거부하고 HEAD/추적 설정/기존 reflog를 함께 옮깁니다. 중단된 변경은 복구 기록을 통해 완료 방향으로 재개하며, 외부 프로그램이 관련 메타데이터를 수정했다면 복구를 거부합니다. 기존 reflog 이력을 보존하고 rename 이벤트를 추가합니다. 복구를 반복해도 이벤트는 중복 추가되지 않습니다. 첫 커밋 전 브랜치와 case-only/parent-child 이름 변경, CLI 세부 일치, 네이티브/Windows 검증은 남아 있습니다.

Native rename update: macOS current dirty branch rename and existing-name refusal passed, including exact index/worktree preservation. Input autocorrection fixed and lowercase collision retested. Frontend38 and release build passed; Windows and remaining rename variants are still unverified.

첫 커밋 전 브랜치도 LOCAL 목록의 Rename으로 이름을 변경할 수 있습니다. HEAD와 추적 설정을 옮기며 새 커밋/ref/reflog를 생성하지 않습니다. 스테이징 상태와 작업 파일을 유지합니다. 이 경로의 네이티브 UI 및 Windows 실행 검증은 남아 있습니다.

첫 커밋 전 rename 네이티브 검증 완료: macOS 실제 앱에서 LOCAL 행·이름 변경·첫 커밋을 수행하고 Git 상태를 비교했습니다. 스테이징된 내용만 커밋되고 나머지 작업 파일 수정은 유지됩니다. Windows 검증은 남아 있습니다.

커밋 상세의 **Cherry-pick / Revert preview**에서 적용/역적용 diff와 충돌을 계산할 수 있습니다. 병합 커밋은 기준 부모를 선택합니다. 현재 미리보기만 제공하며 실제 적용과 충돌 해결은 구현 중입니다.

Cherry-pick / Revert의 충돌 없는 결과를 Apply로 반영할 수 있습니다. 메시지와 committer를 입력하며 cherry-pick은 원저자와 작성 시각을 유지합니다. 중단된 작업의 Recover는 원래 HEAD·인덱스·작업 파일로 되돌립니다. 충돌 해결, 빈 결과와 여러 커밋 연속 처리는 아직 미구현이며 네이티브·Windows 실행 검증은 남아 있습니다.

macOS 실제 앱에서 clean cherry-pick과 revert의 미리보기·적용을 검증했습니다. 커밋 부모, 원저자/committer, 변경 전후 tree와 clean 상태를 Git으로 확인했습니다. 충돌·mainline 네이티브 시험과 Windows는 아직 남아 있습니다.

Replay 충돌에서 **Start conflict resolution**을 누르면 해결 초안을 시작합니다. Current/Incoming·직접 편집·삭제를 저장하고, 모든 충돌 해결 후 Continue로 반영합니다. Resume으로 재개하고 Abort로 초안을 보관할 수 있습니다. 중단된 Continue의 Recover는 원본 작업 상태와 저장된 초안으로 돌아갑니다. 빈 결과·skip·복잡한 rename 충돌과 네이티브/Windows 검증은 남아 있습니다.

Replay 커밋 반영 후 초안 정리 전에 앱이 종료되면 **Finish completed replay**로 정리만 마칩니다. 새 커밋을 반복 생성하지 않고, 완료 후 추가한 작업 파일 수정도 유지합니다. 완료 기록이 현재 HEAD·초안과 일치할 때만 제공됩니다.

Native conflict resolution survives restart and Continue creates exactly one commit; HEAD/index/worktree/author/source refs and draft archive verified. Unresolved diff no longer implies deletion; pending resolution drafts disable ordinary writes. Frontend47 and macOS release passed. Windows, native Abort/Finish and empty/skip remain pending.

Replay preview now reads Base/Current/Incoming conflict versions through recomputed private stages and the existing bounded content viewer. Source files stay unchanged; stale requests and mismatched selections fail. Rust198 (five helpers), frontend48 and clippy passed; native content read and Windows checks pending.

Native replay reads verified for both operations with46 repository hashes unchanged. Fully resolved drafts equal to HEAD now offer Skip without a commit; backend revalidates state and archives draft. Targeted replay_draft10/frontend49/clippy/macOS release passed; native Skip and Skip-specific interruption tests pending.

Skip recovery verified at6 archive kill boundaries plus2 retry kills. Native cherry-pick Skip cancel/confirm preserves HEAD/index/worktree/source refs and one exact draft archive. Solved conflict status corrected; frontend49, replay_draft11 and clippy passed. Native revert Skip, empty-commit policy, sequences and Windows remain.

Clean empty replay results can now be kept as an explicit empty commit (default off). Preview distinguishes originally empty from resulting empty. Four cherry-pick/revert cases match Git CLI trees;12 kill boundaries plus2 recovery kills passed. Full Rust202 (five helpers), frontend50 and clippy passed. Native opt-in, resolved-draft Keep and Windows remain unverified/incomplete.

Resolved empty conflict drafts now offer explicit Keep as well as Skip. Continue defaults to refusing empty commits. Completion/archive kill matrix16 cases includes8 empty-result cases; Finish preserves completed HEAD and subsequent edits. Full202/frontend50/clippy passed; native Keep and Windows remain pending.

Empty replay now covers root and both explicit merge parents in six Git CLI tree-parity cases. Explicit Keep cannot override dirty/detached/stale/index-lock guards. Fixed stale draft preview after external branch change at the same HEAD. Native empty Keep remains unverified because live app window observation is currently unavailable.

Replay sequence preview supports ordered commit/operation/mainline selection and add/remove/move steps. Private intermediate trees match Git CLI sequential application and stop at first conflict without source writes. IPC and UI are connected; execution/resume/abort and native validation remain pending. Frontend52 and clippy passed.

## Replay sequences and remotes

Ordered cherry-pick/revert sequences support start, inspect, saved conflict resolutions, apply/resume, explicit empty commits, skip and Finish. Stop & keep commits archives progress without rewinding. Abort rewinds a clean sequence to its original HEAD/tree through journaled checkout. Dirty initial Abort is refused; finishing an already-completed rewind preserves later edits. Source/original pins and completed archives are retained. Current exact pinned step may resume after source branch deletion/GC. Initial selection remains limited to the loaded graph, different-path conflict resolution is unsupported, and Windows runtime remains unverified.

REMOTE lists redacted endpoints and tracking refs, navigates loaded remote-only commits, and supports local Add/Edit and removal preview. Edit replaces all fetch/push URLs while preserving refspecs; blank push restores fetch fallback. Removal preserves shared tracking references and local branches, clears matching upstream/default-push settings, and uses guarded metadata journals for recovery. Negative source exclusions, source-only/empty destinations and HEAD aliases are supported; existing dangling symbolic refs are preserved. Invalid refspecs refuse removal. Remote rename updates default tracking paths and branch upstream/push configuration with resumable metadata journals; custom fetch destinations stay unchanged, following Git CLI behavior. Clone/fetch/pull/push/auth transport is not connected yet.

Latest evidence and remaining cases are in ../reports/feature-status.json, ../reports/development-validation.json and ../docs/design/07-acceptance.md. Neither successful builds nor isolated native scenarios establish full macOS completion.

Default commit identity can be saved, reloaded or cleared in Preferences. It is stored locally for FLOE without modifying Git configuration. Empty operation identity fields use the saved pair; explicit overrides remain available.
