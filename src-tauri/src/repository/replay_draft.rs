//! Durable replay resolutions; live files remain unchanged until Continue.
use super::*;
const FILE: &str = ".git/branchdesk-replay-draft.json";
const LIMIT: u64 = 20 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    version: u32,
    pub branch: String,
    pub head: String,
    pub source: String,
    pub kind: ReplayKind,
    pub mainline: Option<usize>,
    pub resolutions: HashMap<String, merge_draft::Resolution>,
}
pub(super) fn exists(root: &Path) -> bool {
    fs::symlink_metadata(root.join(FILE)).is_ok()
}
pub(super) fn read(root: &Path) -> Result<Option<Draft>> {
    if !exists(root) {
        return Ok(None);
    }
    let draft: Draft = serde_json::from_slice(&bounded(&root.join(FILE), LIMIT)?).map_err(err)?;
    if draft.version != 1
        || draft.resolutions.len() > MAX_FILES
        || draft.branch.len() > 200
        || !git2::Reference::is_valid_name(&format!("refs/heads/{}", draft.branch))
        || draft.head.len() != 40
        || draft.source.len() != 40
    {
        return Err("INVALID_REPLAY_DRAFT".into());
    }
    Oid::from_str(&draft.head).map_err(err)?;
    Oid::from_str(&draft.source).map_err(err)?;
    for resolution in draft.resolutions.values() {
        validate_resolution(resolution)?;
    }
    Ok(Some(draft))
}
pub(super) fn validate_resolution(resolution: &merge_draft::Resolution) -> Result<()> {
    if let merge_draft::Resolution::Text { text } = resolution {
        if text.len() > FILE_LIMIT as usize || text.contains('\0') {
            return Err("INVALID_RESOLUTION_TEXT".into());
        }
    }
    Ok(())
}
fn write(root: &Path, draft: &Draft, before: Option<&[u8]>) -> Result<()> {
    let bytes = serde_json::to_vec(draft).map_err(err)?;
    if bytes.len() as u64 > LIMIT {
        return Err("LIMIT: replay draft".into());
    }
    let current = if exists(root) {
        Some(bounded(&root.join(FILE), LIMIT)?)
    } else {
        None
    };
    if current.as_deref() != before {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    checkpoint(root, "write", 0);
    let mut temp = tempfile::Builder::new()
        .prefix("branchdesk-replay-draft-")
        .suffix(".next")
        .tempfile_in(root.join(".git"))
        .map_err(err)?;
    std::io::Write::write_all(&mut temp, &bytes).map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    checkpoint(root, "write", 1);
    // Caller holds the repository lease. Compare again before replacement to
    // preserve ordinary external edits; hostile filesystem races remain open.
    let current = if exists(root) {
        Some(bounded(&root.join(FILE), LIMIT)?)
    } else {
        None
    };
    if current.as_deref() != before {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    temp.persist(root.join(FILE)).map_err(err)?;
    checkpoint(root, "write", 2);
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "write", 3);
    Ok(())
}
pub(super) fn start(
    session: &mut Session,
    revision: &str,
    source: &str,
    kind: ReplayKind,
    mainline: Option<usize>,
) -> Result<()> {
    if exists(&session.root) || merge_draft::exists(&session.root) {
        return Err("DRAFT_EXISTS".into());
    }
    let plan = replay::preview(session, revision, source, kind, mainline)?;
    let snap = session.fresh(revision)?;
    if plan.conflicts.is_empty()
        || plan.blockers.iter().any(|b| b != "CONFLICTS")
        || snap.write_block.is_some()
    {
        return Err("REPLAY_DRAFT_REQUIRES_CLEAN_CONFLICTS".into());
    }
    write(
        &session.root,
        &Draft {
            version: 1,
            branch: snap.branch,
            head: plan.head,
            source: source.into(),
            kind,
            mainline,
            resolutions: HashMap::new(),
        },
        None,
    )
}
pub(super) fn save(
    session: &mut Session,
    revision: &str,
    id: &str,
    resolution: merge_draft::Resolution,
) -> Result<()> {
    validate_resolution(&resolution)?;
    let bytes = bounded(&session.root.join(FILE), LIMIT)?;
    let mut draft = read(&session.root)?.ok_or("NO_REPLAY_DRAFT")?;
    let plan = replay::preview(session, revision, &draft.source, draft.kind, draft.mainline)?;
    let snap = session.fresh(revision)?;
    if draft.head != plan.head
        || draft.branch != snap.branch
        || plan.blockers.iter().any(|b| b != "CONFLICTS")
    {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    let conflict = plan
        .conflicts
        .iter()
        .find(|c| {
            [&c.ancestor, &c.ours, &c.theirs]
                .into_iter()
                .flatten()
                .any(|v| v.id == id)
        })
        .ok_or("INVALID_REPLAY_CONFLICT")?;
    let paths: std::collections::HashSet<_> =
        [&conflict.ancestor, &conflict.ours, &conflict.theirs]
            .into_iter()
            .flatten()
            .map(|v| &v.path)
            .collect();
    if paths.len() != 1 {
        return Err("UNSUPPORTED_RENAME_CONFLICT".into());
    }
    draft.resolutions.insert(id.into(), resolution);
    write(&session.root, &draft, Some(&bytes))
}
pub(super) fn skip_empty(session: &mut Session, revision: &str) -> Result<()> {
    let bytes = bounded(&session.root.join(FILE), LIMIT)?;
    let draft = read(&session.root)?.ok_or("NO_REPLAY_DRAFT")?;
    let snap = session.fresh(revision)?;
    if snap.branch != draft.branch || snap.head_oid.as_deref() != Some(&draft.head) {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    let plan = replay::calculate_resolved(
        session,
        revision,
        &draft.source,
        draft.kind,
        draft.mainline,
        Some(&draft.resolutions),
    )?
    .0;
    if !plan.empty || !plan.blockers.is_empty() {
        return Err("REPLAY_NOT_EMPTY".into());
    }
    session.fresh(revision)?;
    if bounded(&session.root.join(FILE), LIMIT)? != bytes {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    archive(&session.root)?;
    Ok(())
}

pub(super) fn archive(root: &Path) -> Result<PathBuf> {
    // Preserve malformed regular JSON too, so Abort can unblock without deletion.
    let bytes = bounded(&root.join(FILE), LIMIT)?;
    let destination = root.join(format!(
        ".git/branchdesk-replay-draft-aborted-{}.json",
        Uuid::new_v4()
    ));
    if bounded(&root.join(FILE), LIMIT)? != bytes {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    checkpoint(root, "archive", 0);
    fs::rename(root.join(FILE), &destination).map_err(err)?;
    checkpoint(root, "archive", 1);
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "archive", 2);
    Ok(destination)
}

const COMPLETION: &str = ".git/branchdesk-replay-completion.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    version: u32,
    branch: String,
    new_oid: String,
    draft_hash: String,
}
pub(super) fn mark_completed(
    root: &Path,
    old: Oid,
    new: Oid,
    record: &replay::Record,
) -> Result<()> {
    let bytes = bounded(&root.join(FILE), LIMIT)?;
    let draft = read(root)?.ok_or("NO_REPLAY_DRAFT")?;
    if draft.head != old.to_string()
        || draft.source != record.source
        || draft.kind != record.kind
        || draft.mainline != record.mainline
        || serde_json::to_value(Some(&draft.resolutions)).map_err(err)?
            != serde_json::to_value(&record.resolutions).map_err(err)?
    {
        return Err("STALE_REPLAY_DRAFT".into());
    }
    if fs::symlink_metadata(root.join(COMPLETION)).is_ok() {
        bounded(&root.join(COMPLETION), 4096)?;
    }
    let marker = Completion {
        version: 1,
        branch: draft.branch,
        new_oid: new.to_string(),
        draft_hash: checkout::digest(&bytes),
    };
    let mut temp = tempfile::Builder::new()
        .prefix("branchdesk-replay-completion-")
        .tempfile_in(root.join(".git"))
        .map_err(err)?;
    std::io::Write::write_all(&mut temp, &serde_json::to_vec(&marker).map_err(err)?)
        .map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    temp.persist(root.join(COMPLETION)).map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}
pub(super) fn completion_ready(root: &Path, repo: &Repository) -> Result<bool> {
    if checkout::pending(root) || !exists(root) || !root.join(COMPLETION).exists() {
        return Ok(false);
    }
    let marker: Completion =
        serde_json::from_slice(&bounded(&root.join(COMPLETION), 4096)?).map_err(err)?;
    if marker.version != 1 {
        return Err("INVALID_REPLAY_COMPLETION".into());
    }
    let bytes = bounded(&root.join(FILE), LIMIT)?;
    if marker.draft_hash != checkout::digest(&bytes) {
        return Ok(false);
    }
    let draft = read(root)?.ok_or("NO_REPLAY_DRAFT")?;
    let head = repo.head().map_err(err)?;
    if head.shorthand().map_err(err)? != marker.branch
        || head.target().map(|id| id.to_string()) != Some(marker.new_oid.clone())
        || marker.branch != draft.branch
    {
        return Ok(false);
    }
    replay::verify_commit(
        repo,
        Oid::from_str(&draft.head).map_err(err)?,
        Oid::from_str(&marker.new_oid).map_err(err)?,
        &replay::Record {
            kind: draft.kind,
            source: draft.source,
            mainline: draft.mainline,
            resolutions: Some(draft.resolutions),
        },
    )?;
    Ok(true)
}
pub(super) fn after_commit_checkpoint(root: &Path) {
    checkpoint(root, "finished", 0);
}

fn checkpoint(root: &Path, operation: &str, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_REPLAY_DRAFT_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_REPLAY_DRAFT_POINT")
            .ok()
            .as_deref()
            == Some(format!("{operation}:{step}").as_str())
    {
        fs::write(root.join(".git/replay-draft-test.ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, operation, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(repo: &Repository, reference: &str, text: &str, parents: &[Oid]) -> Oid {
        let mut b = repo.treebuilder(None).unwrap();
        b.insert("file", repo.blob(text.as_bytes()).unwrap(), 0o100644)
            .unwrap();
        let tree = repo.find_tree(b.write().unwrap()).unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|p| repo.find_commit(*p).unwrap())
            .collect();
        let sig = git2::Signature::now("Draft", "draft@example.invalid").unwrap();
        repo.commit(
            Some(reference),
            &sig,
            &sig,
            "Draft fixture",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn fixture() -> (tempfile::TempDir, Oid) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let base = commit(&repo, "refs/heads/main", "base\n", &[]);
        let target = commit(&repo, "refs/heads/topic", "incoming\n", &[base]);
        commit(&repo, "refs/heads/main", "current\n", &[base]);
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        (d, target)
    }
    #[test]
    fn restart_save_and_abort_preserve_index_head_and_worktree() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            let (d, target) = fixture();
            let root = d.path().canonicalize().unwrap();
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            let index = fs::read(root.join(".git/index")).unwrap();
            let head = fs::read(root.join(".git/HEAD")).unwrap();
            let file = fs::read(root.join("file")).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            start(
                &mut session,
                &snap.revision,
                &target.to_string(),
                kind,
                None,
            )
            .unwrap();
            drop(session);
            let draft = read(&root).unwrap().unwrap();
            assert_eq!(draft.source, target.to_string());
            assert!(draft.resolutions.is_empty());
            let mut session = Session::new(root.clone()).unwrap();
            for resolution in [
                merge_draft::Resolution::Ours {},
                merge_draft::Resolution::Theirs {},
                merge_draft::Resolution::Delete {},
                merge_draft::Resolution::Text {
                    text: "resolved\n".into(),
                },
            ] {
                let snap = session.snapshot().unwrap();
                save(&mut session, &snap.revision, &file_id("file"), resolution).unwrap();
                assert_eq!(read(&root).unwrap().unwrap().resolutions.len(), 1);
            }
            let bytes = fs::read(root.join(FILE)).unwrap();
            let path = archive(&root).unwrap();
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert!(read(&root).unwrap().is_none());
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head);
            assert_eq!(fs::read(root.join("file")).unwrap(), file);
        }
    }
    #[test]
    fn invalid_resolution_dirty_state_and_malformed_draft_do_not_overwrite() {
        let (d, target) = fixture();
        let root = d.path().canonicalize().unwrap();
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        let mut s = Session::new(root.clone()).unwrap();
        let snap = s.snapshot().unwrap();
        start(
            &mut s,
            &snap.revision,
            &target.to_string(),
            ReplayKind::CherryPick,
            None,
        )
        .unwrap();
        let before = fs::read(root.join(FILE)).unwrap();
        for (id, resolution) in [
            ("wrong".into(), merge_draft::Resolution::Ours {}),
            (
                file_id("file"),
                merge_draft::Resolution::Text {
                    text: "bad\0text".into(),
                },
            ),
        ] {
            let snap = s.snapshot().unwrap();
            assert!(save(&mut s, &snap.revision, &id, resolution).is_err());
            assert_eq!(fs::read(root.join(FILE)).unwrap(), before);
        }
        fs::write(root.join("file"), "external edit").unwrap();
        let snap = s.snapshot().unwrap();
        assert!(save(
            &mut s,
            &snap.revision,
            &file_id("file"),
            merge_draft::Resolution::Ours {}
        )
        .unwrap_err()
        .contains("STALE"));
        assert_eq!(fs::read(root.join(FILE)).unwrap(), before);
        fs::write(root.join(FILE), b"malformed {").unwrap();
        assert!(read(&root).is_err());
        let dest = archive(&root).unwrap();
        assert_eq!(fs::read(dest).unwrap(), b"malformed {");
        assert_eq!(fs::read(root.join("file")).unwrap(), b"external edit");
    }
    #[test]
    fn replay_draft_child_job() {
        let Some(root) = std::env::var_os("BRANCHDESK_REPLAY_DRAFT_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        match std::env::var("BRANCHDESK_REPLAY_DRAFT_ACTION")
            .unwrap()
            .as_str()
        {
            "start" => {
                let target = open(&root)
                    .unwrap()
                    .refname_to_id("refs/heads/topic")
                    .unwrap();
                start(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    ReplayKind::CherryPick,
                    None,
                )
                .unwrap();
            }
            "save" => save(
                &mut session,
                &snap.revision,
                &file_id("file"),
                merge_draft::Resolution::Text {
                    text: "resolved after restart\n".into(),
                },
            )
            .unwrap(),
            "archive" => {
                archive(&root).unwrap();
            }
            "continue" | "continueEmpty" => {
                drop(_lease);
                session
                    .mutate(
                        &snap.revision,
                        Action::ContinueReplay {
                            allow_empty: std::env::var("BRANCHDESK_REPLAY_DRAFT_ACTION").unwrap()
                                == "continueEmpty",
                            message: "Resolved replay".into(),
                            author: "Resolver".into(),
                            email: "resolver@example.invalid".into(),
                        },
                    )
                    .unwrap();
            }
            "skip" => {
                drop(_lease);
                session.mutate(&snap.revision, Action::SkipReplay).unwrap();
            }
            "recover" => {
                drop(_lease);
                session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
            }
            _ => panic!("invalid test action"),
        }
    }
    #[test]
    fn actual_kill_start_save_and_archive_never_publish_partial_json() {
        for action in ["start", "save", "archive"] {
            for step in 0..=if action == "archive" { 2 } else { 3 } {
                let (d, target) = fixture();
                let root = d.path().canonicalize().unwrap();
                let original_index = fs::read(root.join(".git/index")).unwrap();
                let original_head = open(&root).unwrap().head().unwrap().target().unwrap();
                if action != "start" {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    start(
                        &mut session,
                        &snap.revision,
                        &target.to_string(),
                        ReplayKind::CherryPick,
                        None,
                    )
                    .unwrap();
                }
                let before = fs::read(root.join(FILE)).ok();
                let ready = root.join(".git/replay-draft-test.ready");
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::replay_draft::tests::replay_draft_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_REPLAY_DRAFT_ROOT", &root)
                    .env("BRANCHDESK_REPLAY_DRAFT_ACTION", action)
                    .env(
                        "BRANCHDESK_REPLAY_DRAFT_POINT",
                        format!(
                            "{}:{step}",
                            if action == "archive" {
                                "archive"
                            } else {
                                "write"
                            }
                        ),
                    )
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !ready.exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout {action}:{step}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                let draft = read(&root).unwrap();
                if action == "archive" && step > 0 {
                    assert!(draft.is_none());
                    let files: Vec<_> = fs::read_dir(root.join(".git"))
                        .unwrap()
                        .map(|e| e.unwrap().path())
                        .filter(|p| {
                            p.file_name()
                                .unwrap()
                                .to_string_lossy()
                                .starts_with("branchdesk-replay-draft-aborted-")
                        })
                        .collect();
                    assert_eq!(files.len(), 1);
                    assert_eq!(fs::read(&files[0]).ok(), before);
                } else if (action == "start" && step < 2)
                    || (action == "save" && step < 2)
                    || (action == "archive" && step == 0)
                {
                    assert_eq!(fs::read(root.join(FILE)).ok(), before);
                } else {
                    let draft = draft.unwrap();
                    assert_eq!(
                        draft.resolutions.len(),
                        if action == "save" { 1 } else { 0 }
                    );
                }
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), original_index);
                assert_eq!(
                    open(&root).unwrap().head().unwrap().target(),
                    Some(original_head)
                );
                assert_eq!(fs::read(root.join("file")).unwrap(), b"current\n");
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                if !exists(&root) {
                    start(
                        &mut session,
                        &snap.revision,
                        &target.to_string(),
                        ReplayKind::CherryPick,
                        None,
                    )
                    .unwrap();
                }
                let snap = session.snapshot().unwrap();
                save(
                    &mut session,
                    &snap.revision,
                    &file_id("file"),
                    merge_draft::Resolution::Ours {},
                )
                .unwrap();
            }
        }
    }
    #[test]
    fn public_draft_actions_continue_resolved_commit_and_block_unrelated_writes() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for choice in ["theirs", "text", "delete", "ours"] {
                let (d, target) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let head = snap.head_oid.clone().unwrap();
                let started = session
                    .mutate(
                        &snap.revision,
                        Action::StartReplay {
                            oid: target.to_string(),
                            operation: kind,
                            mainline: None,
                        },
                    )
                    .unwrap();
                assert!(started.replay_draft.is_some());
                assert_ne!(started.revision, snap.revision);
                assert!(session
                    .mutate(&started.revision, Action::StageAll)
                    .unwrap_err()
                    .contains("REPLAY_IN_PROGRESS"));
                assert!(session
                    .mutate(
                        &started.revision,
                        Action::ContinueReplay {
                            allow_empty: choice == "ours",
                            message: "Resolve".into(),
                            author: "Resolver".into(),
                            email: "resolve@example.invalid".into()
                        }
                    )
                    .unwrap_err()
                    .contains("UNRESOLVED"));
                let resolution = match choice {
                    "ours" => merge_draft::Resolution::Ours {},
                    "theirs" => merge_draft::Resolution::Theirs {},
                    "delete" => merge_draft::Resolution::Delete {},
                    _ => merge_draft::Resolution::Text {
                        text: "resolved by user\n".into(),
                    },
                };
                let saved = session
                    .mutate(
                        &started.revision,
                        Action::ResolveReplay {
                            file_id: file_id("file"),
                            resolution,
                        },
                    )
                    .unwrap();
                assert_ne!(saved.revision, started.revision);
                if choice == "ours" {
                    assert!(session
                        .mutate(
                            &saved.revision,
                            Action::ContinueReplay {
                                allow_empty: false,
                                message: "Resolve".into(),
                                author: "Resolver".into(),
                                email: "resolve@example.invalid".into()
                            }
                        )
                        .unwrap_err()
                        .contains("EMPTY_REPLAY"));
                }
                let finished = session
                    .mutate(
                        &saved.revision,
                        Action::ContinueReplay {
                            allow_empty: choice == "ours",
                            message: "Resolve".into(),
                            author: "Resolver".into(),
                            email: "resolve@example.invalid".into(),
                        },
                    )
                    .unwrap();
                assert!(finished.replay_draft.is_none());
                assert!(finished.changes.is_empty());
                let repo = open(&root).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap().to_string(), head);
                assert_eq!(repo.refname_to_id("refs/heads/topic").unwrap(), target);
                if choice == "delete" {
                    assert!(!root.join("file").exists());
                } else {
                    assert_eq!(
                        fs::read_to_string(root.join("file")).unwrap(),
                        if choice == "text" {
                            "resolved by user\n"
                        } else if choice == "ours" {
                            "current\n"
                        } else if kind == ReplayKind::CherryPick {
                            "incoming\n"
                        } else {
                            "base\n"
                        }
                    );
                }
            }
        }
    }
    #[test]
    fn skip_empty_replay_preserves_head_index_worktree_and_rejects_nonempty_or_stale() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for as_text in [false, true] {
                let (d, target) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let head = snap.head_oid.clone();
                let index = fs::read(root.join(".git/index")).unwrap();
                let file = fs::read(root.join("file")).unwrap();
                let started = session
                    .mutate(
                        &snap.revision,
                        Action::StartReplay {
                            oid: target.to_string(),
                            operation: kind,
                            mainline: None,
                        },
                    )
                    .unwrap();
                assert!(session
                    .mutate(&started.revision, Action::SkipReplay)
                    .unwrap_err()
                    .contains("UNRESOLVED"));
                let saved = session
                    .mutate(
                        &started.revision,
                        Action::ResolveReplay {
                            file_id: file_id("file"),
                            resolution: merge_draft::Resolution::Theirs {},
                        },
                    )
                    .unwrap();
                assert!(!session
                    .replay_preview(&saved.revision, &target.to_string(), kind, None)
                    .unwrap()
                    .resolved_empty
                    .unwrap());
                assert!(session
                    .mutate(&saved.revision, Action::SkipReplay)
                    .unwrap_err()
                    .contains("REPLAY_NOT_EMPTY"));
                let resolution = if as_text {
                    merge_draft::Resolution::Text {
                        text: "current\n".into(),
                    }
                } else {
                    merge_draft::Resolution::Ours {}
                };
                let saved = session
                    .mutate(
                        &saved.revision,
                        Action::ResolveReplay {
                            file_id: file_id("file"),
                            resolution,
                        },
                    )
                    .unwrap();
                assert_eq!(
                    session
                        .replay_preview(&saved.revision, &target.to_string(), kind, None)
                        .unwrap()
                        .resolved_empty,
                    Some(true)
                );
                let bytes = fs::read(root.join(FILE)).unwrap();
                fs::write(root.join("file"), "external change").unwrap();
                assert!(session
                    .mutate(&saved.revision, Action::SkipReplay)
                    .unwrap_err()
                    .starts_with("STALE"));
                let dirty = session.snapshot().unwrap();
                assert!(session.mutate(&dirty.revision, Action::SkipReplay).is_err());
                assert_eq!(fs::read(root.join(FILE)).unwrap(), bytes);
                assert_eq!(
                    fs::read_to_string(root.join("file")).unwrap(),
                    "external change"
                );
                fs::write(root.join("file"), &file).unwrap();
                let fresh = session.snapshot().unwrap();
                let finished = session.mutate(&fresh.revision, Action::SkipReplay).unwrap();
                assert!(finished.replay_draft.is_none());
                assert_eq!(finished.head_oid, head);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), file);
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .refname_to_id("refs/heads/topic")
                        .unwrap(),
                    target
                );
                assert!(fs::read_dir(root.join(".git")).unwrap().flatten().any(|f| f
                    .file_name()
                    .to_string_lossy()
                    .starts_with("branchdesk-replay-draft-aborted-")
                    && fs::read(f.path()).unwrap() == bytes));
                assert!(!root.join(".git/index.lock").exists());
            }
        }
    }

    #[test]
    fn resolved_preview_rejects_external_branch_switch_at_same_head() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            let (d, target) = fixture();
            let root = d.path().canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let started = session
                .mutate(
                    &snap.revision,
                    Action::StartReplay {
                        oid: target.to_string(),
                        operation: kind,
                        mainline: None,
                    },
                )
                .unwrap();
            let saved = session
                .mutate(
                    &started.revision,
                    Action::ResolveReplay {
                        file_id: file_id("file"),
                        resolution: merge_draft::Resolution::Ours {},
                    },
                )
                .unwrap();
            assert_eq!(
                session
                    .replay_preview(&saved.revision, &target.to_string(), kind, None)
                    .unwrap()
                    .resolved_empty,
                Some(true)
            );
            let bytes = fs::read(root.join(FILE)).unwrap();
            let repo = open(&root).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            repo.branch("external", &head, false).unwrap();
            repo.set_head("refs/heads/external").unwrap();
            let snap = session.snapshot().unwrap();
            assert_eq!(
                session
                    .replay_preview(&snap.revision, &target.to_string(), kind, None)
                    .unwrap_err(),
                "STALE_REPLAY_DRAFT"
            );
            assert_eq!(
                session
                    .mutate(&snap.revision, Action::SkipReplay)
                    .unwrap_err(),
                "STALE_REPLAY_DRAFT"
            );
            assert_eq!(fs::read(root.join(FILE)).unwrap(), bytes);
            assert_eq!(repo.head().unwrap().target(), Some(head.id()));
            assert_eq!(fs::read(root.join("file")).unwrap(), b"current\n");
        }
    }

    #[test]
    fn public_abort_preserves_dirty_edits_and_archives_damaged_draft() {
        let (d, target) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut s = Session::new(root.clone()).unwrap();
        let snap = s.snapshot().unwrap();
        s.mutate(
            &snap.revision,
            Action::StartReplay {
                oid: target.to_string(),
                operation: ReplayKind::CherryPick,
                mainline: None,
            },
        )
        .unwrap();
        fs::write(root.join("file"), "external dirty edit").unwrap();
        fs::write(root.join(FILE), b"malformed").unwrap();
        let snap = s.snapshot().unwrap();
        assert!(snap.replay_draft_error.is_some());
        let after = s.mutate(&snap.revision, Action::AbortReplay).unwrap();
        assert!(after.replay_draft_error.is_none());
        assert!(after.replay_draft.is_none());
        assert_eq!(fs::read(root.join("file")).unwrap(), b"external dirty edit");
    }
    fn kill_continue(root: &Path, step: usize, recovery: bool) {
        let ready = root.join(".git/branchdesk-checkout/child.ready");
        let _ = fs::remove_file(&ready);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::replay_draft::tests::replay_draft_child_job",
                "--nocapture",
            ])
            .env("BRANCHDESK_REPLAY_DRAFT_ROOT", root)
            .env(
                "BRANCHDESK_REPLAY_DRAFT_ACTION",
                if recovery { "recover" } else { "continue" },
            )
            .env("BRANCHDESK_CRASH_ROOT", root)
            .env(
                if recovery {
                    "BRANCHDESK_RECOVERY_STEP"
                } else {
                    "BRANCHDESK_CRASH_STEP"
                },
                step.to_string(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("continue checkpoint timeout {step}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn resolved_continue_actual_kills_preserve_draft_and_retry_after_rollback() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for step in [100, 101, 102, 0, 1, 2, 3, 103, 104] {
                let (d, target) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let head = snap.head_oid.clone();
                let index = fs::read(root.join(".git/index")).unwrap();
                let file = fs::read(root.join("file")).unwrap();
                let snap = session
                    .mutate(
                        &snap.revision,
                        Action::StartReplay {
                            oid: target.to_string(),
                            operation: kind,
                            mainline: None,
                        },
                    )
                    .unwrap();
                session
                    .mutate(
                        &snap.revision,
                        Action::ResolveReplay {
                            file_id: file_id("file"),
                            resolution: merge_draft::Resolution::Text {
                                text: "resolved\n".into(),
                            },
                        },
                    )
                    .unwrap();
                let draft_bytes = fs::read(root.join(FILE)).unwrap();
                drop(session);
                kill_continue(&root, step, false);
                if step == 2 {
                    kill_continue(&root, 1, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let snap = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!snap.recovery_pending);
                assert_eq!(snap.head_oid, head);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), file);
                assert_eq!(fs::read(root.join(FILE)).unwrap(), draft_bytes);
                let snap = session
                    .mutate(
                        &snap.revision,
                        Action::ContinueReplay {
                            allow_empty: false,
                            message: "Resolved replay".into(),
                            author: "Resolver".into(),
                            email: "resolver@example.invalid".into(),
                        },
                    )
                    .unwrap();
                assert!(snap.replay_draft.is_none());
                assert!(snap.changes.is_empty());
                assert_eq!(fs::read(root.join("file")).unwrap(), b"resolved\n");
            }
        }
    }
    fn kill_after_commit(root: &Path, point: &str) {
        kill_draft_action(root, point, "continue");
    }
    fn kill_draft_action(root: &Path, point: &str, action: &str) {
        let ready = root.join(".git/replay-draft-test.ready");
        let _ = fs::remove_file(&ready);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::replay_draft::tests::replay_draft_child_job",
                "--nocapture",
            ])
            .env("BRANCHDESK_REPLAY_DRAFT_ROOT", root)
            .env("BRANCHDESK_REPLAY_DRAFT_ACTION", action)
            .env("BRANCHDESK_REPLAY_DRAFT_POINT", point)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                panic!(
                    "completion child failed {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("completion timeout {point}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn skip_actual_kill_preserves_repository_and_can_finish_after_restart() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in ["archive:0", "archive:1", "archive:2"] {
                let (d, target) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let head = snap.head_oid.clone();
                let index = fs::read(root.join(".git/index")).unwrap();
                let file = fs::read(root.join("file")).unwrap();
                let snap = session
                    .mutate(
                        &snap.revision,
                        Action::StartReplay {
                            oid: target.to_string(),
                            operation: kind,
                            mainline: None,
                        },
                    )
                    .unwrap();
                session
                    .mutate(
                        &snap.revision,
                        Action::ResolveReplay {
                            file_id: file_id("file"),
                            resolution: merge_draft::Resolution::Ours {},
                        },
                    )
                    .unwrap();
                let bytes = fs::read(root.join(FILE)).unwrap();
                drop(session);
                kill_draft_action(&root, point, "skip");
                assert!(!root.join(".git/index.lock").exists());
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), file);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert_eq!(snap.head_oid, head);
                assert!(!snap.recovery_pending);
                assert!(!snap.replay_completion_ready);
                if point == "archive:0" {
                    assert_eq!(fs::read(root.join(FILE)).unwrap(), bytes);
                    // A second interrupted retry still leaves the same resumable draft.
                    drop(session);
                    kill_draft_action(&root, point, "skip");
                    session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    let finished = session.mutate(&snap.revision, Action::SkipReplay).unwrap();
                    assert!(finished.replay_draft.is_none());
                } else {
                    assert!(snap.replay_draft.is_none());
                }
                let archives: Vec<_> = fs::read_dir(root.join(".git"))
                    .unwrap()
                    .flatten()
                    .filter(|f| {
                        f.file_name()
                            .to_string_lossy()
                            .starts_with("branchdesk-replay-draft-aborted-")
                    })
                    .collect();
                assert_eq!(archives.len(), 1);
                assert_eq!(fs::read(archives[0].path()).unwrap(), bytes);
                assert_eq!(session.snapshot().unwrap().head_oid, head);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), file);
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .refname_to_id("refs/heads/topic")
                        .unwrap(),
                    target
                );
                let _lease = lease::OperationLease::acquire(&root).unwrap();
            }
        }
    }

    #[test]
    fn actual_kill_completed_replay_finishes_archive_without_duplicate_commit_or_losing_edits() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for keep_empty in [false, true] {
                for point in ["finished:0", "archive:0", "archive:1", "archive:2"] {
                    let (d, target) = fixture();
                    let root = d.path().canonicalize().unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    let old = snap.head_oid.clone().unwrap();
                    let snap = session
                        .mutate(
                            &snap.revision,
                            Action::StartReplay {
                                oid: target.to_string(),
                                operation: kind,
                                mainline: None,
                            },
                        )
                        .unwrap();
                    session
                        .mutate(
                            &snap.revision,
                            Action::ResolveReplay {
                                file_id: file_id("file"),
                                resolution: if keep_empty {
                                    merge_draft::Resolution::Ours {}
                                } else {
                                    merge_draft::Resolution::Text {
                                        text: "resolved\n".into(),
                                    }
                                },
                            },
                        )
                        .unwrap();
                    drop(session);
                    if keep_empty {
                        kill_draft_action(&root, point, "continueEmpty");
                    } else {
                        kill_after_commit(&root, point);
                    }
                    let repo = open(&root).unwrap();
                    let new = repo.head().unwrap().peel_to_commit().unwrap();
                    assert_eq!(new.parent_id(0).unwrap().to_string(), old);
                    let new_id = new.id();
                    let index = fs::read(root.join(".git/index")).unwrap();
                    fs::write(root.join("file"), "external edit after completed commit").unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    assert!(!snap.recovery_pending);
                    if point == "finished:0" || point == "archive:0" {
                        assert!(snap.replay_completion_ready);
                        let done = session
                            .mutate(
                                &snap.revision,
                                Action::ContinueReplay {
                                    allow_empty: false,
                                    message: String::new(),
                                    author: String::new(),
                                    email: String::new(),
                                },
                            )
                            .unwrap();
                        assert!(done.replay_draft.is_none());
                        assert!(!done.replay_completion_ready);
                    } else {
                        assert!(snap.replay_draft.is_none());
                    }
                    assert_eq!(open(&root).unwrap().head().unwrap().target(), Some(new_id));
                    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                    assert_eq!(
                        fs::read(root.join("file")).unwrap(),
                        b"external edit after completed commit"
                    );
                }
            }
        }
    }
    #[test]
    fn completion_marker_never_finalizes_a_changed_draft() {
        let (d, target) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let snap = session
            .mutate(
                &snap.revision,
                Action::StartReplay {
                    oid: target.to_string(),
                    operation: ReplayKind::CherryPick,
                    mainline: None,
                },
            )
            .unwrap();
        session
            .mutate(
                &snap.revision,
                Action::ResolveReplay {
                    file_id: file_id("file"),
                    resolution: merge_draft::Resolution::Text {
                        text: "resolved\n".into(),
                    },
                },
            )
            .unwrap();
        drop(session);
        kill_after_commit(&root, "finished:0");
        let mut draft = read(&root).unwrap().unwrap();
        draft.resolutions.insert(
            file_id("file"),
            merge_draft::Resolution::Text {
                text: "external draft edit".into(),
            },
        );
        let bytes = serde_json::to_vec(&draft).unwrap();
        fs::write(root.join(FILE), &bytes).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(!snap.replay_completion_ready);
        assert!(session
            .mutate(
                &snap.revision,
                Action::ContinueReplay {
                    allow_empty: false,
                    message: "x".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into()
                }
            )
            .unwrap_err()
            .contains("STALE_REPLAY_DRAFT"));
        assert_eq!(fs::read(root.join(FILE)).unwrap(), bytes);
    }
}
