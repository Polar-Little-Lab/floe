//! Guarded local remote HEAD changes. No transport or server mutation occurs here.
use super::*;
use branch_rename::{read_optional, without_packed_ref, Change};
const JOURNAL: &str = ".git/branchdesk-remote-head";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    remote: String,
    branch: Option<String>,
    old_oid: Option<String>,
    target_oid: Option<String>,
    seconds: i64,
    changes: Vec<Change>,
}
fn names(remote: &str, branch: Option<&str>) -> Result<(String, Option<String>)> {
    if remote.is_empty()
        || remote.len() > 200
        || remote.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/remotes/{remote}/probe"))
    {
        return Err("INVALID_REMOTE_NAME".into());
    }
    let head = format!("refs/remotes/{remote}/HEAD");
    let target = branch.map(|b| format!("refs/remotes/{remote}/{b}"));
    if branch.is_some_and(|b| b.is_empty() || b.len() > 1024 || b.starts_with('-'))
        || target.as_ref().is_some_and(|t| {
            !git2::Reference::is_valid_name(t) || t == &head || t.starts_with(&(head.clone() + "/"))
        })
    {
        return Err("INVALID_REMOTE_HEAD_TARGET".into());
    }
    Ok((head, target))
}
fn config_valid(bytes: &[u8], remote: &str) -> Result<()> {
    let section = format!("remote.{remote}");
    if !remote_remove::entries(bytes)?
        .iter()
        .any(|(k, _)| k.rsplit_once('.').is_some_and(|(s, _)| s == section))
    {
        return Err("REMOTE_NOT_FOUND".into());
    }
    Ok(())
}
fn log_after(record: &Record, before: &Option<Vec<u8>>, config: &[u8]) -> Result<Option<Vec<u8>>> {
    if record.branch.is_none() {
        return Ok(None);
    }
    if before.is_none() && !branch_rename::logging_enabled(config)? {
        return Ok(None);
    }
    let mut bytes = before.clone().unwrap_or_default();
    bytes.extend(
        format!(
            "{} {} FLOE <branchdesk@localhost> {} +0000\tremote set-head\n",
            record
                .old_oid
                .as_deref()
                .unwrap_or("0000000000000000000000000000000000000000"),
            record
                .target_oid
                .as_deref()
                .ok_or("INVALID_REMOTE_HEAD_JOURNAL")?,
            record.seconds
        )
        .bytes(),
    );
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("LIMIT: remote HEAD reflog".into());
    }
    Ok(Some(bytes))
}
fn prepare(root: &Path, remote: &str, branch: Option<&str>) -> Result<Record> {
    let (head, target) = names(remote, branch)?;
    let repo = open(root)?;
    let config = bounded(&root.join(".git/config"), 65536)?;
    config_valid(&config, remote)?;
    let old_oid = repo
        .find_reference(&head)
        .ok()
        .and_then(|r| r.resolve().ok())
        .and_then(|r| r.target())
        .map(|o| o.to_string());
    let target_oid = if let Some(t) = &target {
        let r = repo
            .find_reference(t)
            .map_err(|_| "REMOTE_HEAD_REQUIRES_FETCHED_BRANCH")?;
        // Avoid a mutable symbolic chain, including cycles or a local branch alias.
        let oid = r.target().ok_or("REMOTE_HEAD_TARGET_MUST_BE_DIRECT")?;
        repo.find_commit(oid)
            .map_err(|_| "REMOTE_HEAD_TARGET_MUST_BE_COMMIT")?;
        Some(oid.to_string())
    } else {
        None
    };
    let packed = read_optional(root, "packed-refs")?;
    let mut record = Record {
        version: 1,
        remote: remote.into(),
        branch: branch.map(str::to_owned),
        old_oid,
        target_oid,
        seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(err)?
            .as_secs() as i64,
        changes: vec![
            Change {
                path: "config".into(),
                before: Some(config.clone()),
                after: Some(config),
            },
            Change {
                path: "packed-refs".into(),
                after: packed
                    .as_ref()
                    .map(|p| without_packed_ref(p, &head))
                    .transpose()?,
                before: packed,
            },
        ],
    };
    if let Some(t) = &target {
        let before = read_optional(root, t)?;
        record.changes.push(Change {
            path: t.clone(),
            after: before.clone(),
            before,
        });
    }
    record.changes.push(Change {
        path: head.clone(),
        before: read_optional(root, &head)?,
        after: target.map(|t| format!("ref: {t}\n").into_bytes()),
    });
    let log = format!("logs/{head}");
    let before = read_optional(root, &log)?;
    let after = log_after(&record, &before, record.changes[0].before.as_ref().unwrap())?;
    record.changes.push(Change {
        path: log,
        before,
        after,
    });
    verify(&record)?;
    Ok(record)
}
fn verify(record: &Record) -> Result<()> {
    let (head, target) = names(&record.remote, record.branch.as_deref())?;
    if record.version != 1
        || record.seconds < 0
        || record.changes.len() != if target.is_some() { 5 } else { 4 }
        || record
            .old_oid
            .iter()
            .chain(record.target_oid.iter())
            .any(|s| {
                s.len() != 40
                    || !s.bytes().all(|b| b.is_ascii_hexdigit())
                    || s.bytes().all(|b| b == b'0')
            })
        || target.is_some() != record.target_oid.is_some()
    {
        return Err("INVALID_REMOTE_HEAD_JOURNAL".into());
    }
    let config = &record.changes[0];
    if config.path != "config" || config.before.is_none() || config.before != config.after {
        return Err("INVALID_REMOTE_HEAD_CONFIG".into());
    }
    config_valid(config.before.as_ref().unwrap(), &record.remote)?;
    let packed = &record.changes[1];
    if packed.path != "packed-refs"
        || packed.after
            != packed
                .before
                .as_ref()
                .map(|p| without_packed_ref(p, &head))
                .transpose()?
    {
        return Err("INVALID_REMOTE_HEAD_PACKED".into());
    }
    let mut i = 2;
    if let Some(t) = &target {
        let guard = &record.changes[i];
        i += 1;
        if guard.path != *t || guard.before != guard.after {
            return Err("INVALID_REMOTE_HEAD_TARGET_GUARD".into());
        }
        let oid = if let Some(bytes) = &guard.before {
            std::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.strip_suffix('\n'))
                .map(str::to_owned)
        } else {
            packed
                .before
                .as_ref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|s| {
                    s.lines().find_map(|line| {
                        line.split_once(' ')
                            .filter(|(_, name)| *name == t)
                            .map(|(oid, _)| oid.to_string())
                    })
                })
        };
        if oid != record.target_oid {
            return Err("INVALID_REMOTE_HEAD_TARGET_OID".into());
        }
    }
    let change = &record.changes[i];
    if change.path != head || change.after != target.map(|t| format!("ref: {t}\n").into_bytes()) {
        return Err("INVALID_REMOTE_HEAD_PATH".into());
    }
    let log = &record.changes[i + 1];
    if log.path != format!("logs/{head}")
        || log.after != log_after(record, &log.before, config.before.as_ref().unwrap())?
    {
        return Err("INVALID_REMOTE_HEAD_LOG".into());
    }
    Ok(())
}
fn paths(record: &Record) -> Result<Vec<String>> {
    verify(record)?;
    Ok(record
        .changes
        .iter()
        .map(|c| format!("{}.lock", c.path))
        .collect())
}
fn markers(root: &Path, dir: &Path, record: &Record) -> Result<()> {
    let mut paths = paths(record)?;
    paths.push("index.lock".into());
    journal_locks::record_paths(root, dir, paths)
}
fn apply(root: &Path, record: &Record) -> Result<()> {
    verify(record)?;
    // Check every guard before publishing any file, including after a partial recovery.
    for c in &record.changes {
        let now = read_optional(root, &c.path)?;
        if now != c.before && now != c.after {
            return Err(format!("RECOVERY_CONFLICT: {} changed", c.path));
        }
    }
    if let Some(oid) = &record.target_oid {
        open(root)?
            .find_commit(Oid::from_str(oid).map_err(err)?)
            .map_err(|_| "REMOTE_HEAD_TARGET_MISSING")?;
    }
    checkpoint(root, 0);
    for (i, c) in record.changes.iter().enumerate() {
        let now = read_optional(root, &c.path)?;
        if now != c.after {
            if now != c.before {
                return Err("RECOVERY_CONFLICT: remote HEAD changed".into());
            }
            let path = root.join(".git").join(&c.path);
            if let Some(bytes) = &c.after {
                let mut temp =
                    tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(err)?;
                std::io::Write::write_all(&mut temp, bytes).map_err(err)?;
                temp.as_file().sync_all().map_err(err)?;
                temp.persist(&path).map_err(err)?;
            } else if now.is_some() {
                fs::remove_file(&path).map_err(err)?;
            }
            checkout::sync_dir(path.parent().unwrap())?;
        }
        checkpoint(root, i + 1);
    }
    Ok(())
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
pub(super) fn recover(root: &Path) -> Result<()> {
    let dir = root.join(JOURNAL);
    let metadata = fs::symlink_metadata(&dir).map_err(err)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err("INVALID_REMOTE_HEAD_JOURNAL".into());
    }
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 32 * 1024 * 1024)?)
            .map_err(|_| "INVALID_REMOTE_HEAD_JOURNAL")?;
    let _locks = remote_remove::acquire_paths(root, paths(&record)?)?;
    markers(root, &dir, &record)?;
    apply(root, &record)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!("{JOURNAL}-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}
impl Session {
    pub fn remote_head_preview(
        &mut self,
        revision: &str,
        remote: &str,
        branch: Option<&str>,
    ) -> Result<()> {
        let snapshot = self.fresh(revision)?;
        if let Some(reason) = snapshot.write_block {
            return Err(format!("READ_ONLY: {reason}"));
        }
        if open(&self.root)?.state() != git2::RepositoryState::Clean
            || (sequence_state::exists(&self.root) || rebase::draft::exists(&self.root))
            || replay_draft::exists(&self.root)
            || merge_draft::exists(&self.root)
        {
            return Err("REMOTE_HEAD_PENDING_OPERATION".into());
        }
        prepare(&self.root, remote, branch)?;
        Ok(())
    }
    pub fn set_remote_head(
        &mut self,
        revision: &str,
        remote: &str,
        branch: Option<&str>,
    ) -> Result<Snapshot> {
        self.remote_head_preview(revision, remote, branch)?;
        let _lease = lease::OperationLease::acquire(&self.root)?;
        let index = IndexLock::acquire(&self.root)?;
        let initial = prepare(&self.root, remote, branch)?;
        let _locks = remote_remove::acquire_paths(&self.root, paths(&initial)?)?;
        self.fresh(revision)?;
        let checked = prepare(&self.root, remote, branch)?;
        if checked
            .changes
            .iter()
            .map(|c| (&c.path, &c.before))
            .ne(initial.changes.iter().map(|c| (&c.path, &c.before)))
            || checked.old_oid != initial.old_oid
            || checked.target_oid != initial.target_oid
        {
            return Err("STALE_STATE".into());
        }
        let record = initial;
        let dir = self.root.join(JOURNAL);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(err)?;
        checkout::save(
            &dir.join("record.json"),
            &serde_json::to_vec(&record).map_err(err)?,
        )?;
        markers(&self.root, &dir, &record)?;
        checkout::sync_dir(&self.root.join(".git"))?;
        apply(&self.root, &record)?;
        drop(index);
        finish(&self.root)?;
        self.snapshot()
    }
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_HEAD_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_HEAD_STEP").ok().as_deref() == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/head-test-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

/// Select the server's default branch, never an arbitrary first advertised branch.
pub fn advertised_branch(ad: &crate::network_worker::Advertisement) -> Result<String> {
    if ad.refs.len() > 2000 {
        return Err("REMOTE_REF_LIMIT".into());
    }
    let heads: Vec<_> = ad.refs.iter().filter(|r| r.name == "HEAD").collect();
    if heads.len() != 1 {
        return Err("REMOTE_HEAD_NOT_ADVERTISED".into());
    }
    let head = heads[0];
    let candidates: Vec<_> = ad
        .refs
        .iter()
        .filter(|r| {
            r.name.starts_with("refs/heads/")
                && r.oid == head.oid
                && head.symbolic_target.as_ref().is_none_or(|t| t == &r.name)
        })
        .collect();
    if candidates.len() != 1 {
        return Err("REMOTE_HEAD_AMBIGUOUS_OR_UNBORN".into());
    }
    let branch = candidates[0].name.strip_prefix("refs/heads/").unwrap();
    names("probe", Some(branch))?;
    Ok(branch.into())
}

#[cfg(test)]
mod tests {
    use super::super::remote_remove::tests::{fixture, git, refs};
    use super::*;
    fn setup(kind: &str) -> tempfile::TempDir {
        let d = fixture("standard");
        let repo = Repository::open(d.path()).unwrap();
        let oid = repo.head().unwrap().target().unwrap();
        repo.reference("refs/remotes/origin/next", oid, false, "fixture")
            .unwrap();
        match kind {
            "packed" => {
                git(d.path(), &["pack-refs", "--all", "--prune"]);
            }
            "direct" => {
                repo.find_reference("refs/remotes/origin/HEAD")
                    .unwrap()
                    .delete()
                    .unwrap();
                repo.reference("refs/remotes/origin/HEAD", oid, false, "fixture")
                    .unwrap();
                git(d.path(), &["pack-refs", "--all", "--prune"]);
            }
            "missing" => {
                repo.find_reference("refs/remotes/origin/HEAD")
                    .unwrap()
                    .delete()
                    .unwrap();
            }
            "dangling" => {
                repo.reference_symbolic(
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/missing",
                    true,
                    "fixture",
                )
                .unwrap();
            }
            "no-log" => {
                repo.config()
                    .unwrap()
                    .set_bool("core.logallrefupdates", false)
                    .unwrap();
                let _ = fs::remove_file(d.path().join(".git/logs/refs/remotes/origin/HEAD"));
            }
            _ => {}
        }
        d
    }
    #[test]
    fn set_and_delete_match_git_while_preserving_worktree_config_index_and_refs() {
        for kind in [
            "standard", "packed", "direct", "missing", "dangling", "no-log",
        ] {
            let d = setup(kind);
            let oracle = setup(kind);
            let before = [
                "config",
                "HEAD",
                "index",
                "refs/remotes/origin/main",
                "refs/remotes/origin/next",
            ]
            .map(|p| read_optional(d.path(), p).unwrap());
            let mut session = Session::new(d.path().into()).unwrap();
            for branch in [Some("next"), Some("main"), None, None] {
                git(
                    oracle.path(),
                    &["remote", "set-head", "origin", branch.unwrap_or("--delete")],
                );
                let snap = session.snapshot().unwrap();
                let next = session
                    .set_remote_head(&snap.revision, "origin", branch)
                    .unwrap();
                assert!(!next.recovery_pending);
                assert_eq!(
                    read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap(),
                    read_optional(oracle.path(), "refs/remotes/origin/HEAD").unwrap(),
                    "{kind} {branch:?}"
                );
                assert_eq!(refs(d.path()), refs(oracle.path()));
                let log = read_optional(d.path(), "logs/refs/remotes/origin/HEAD").unwrap();
                let oracle_log =
                    read_optional(oracle.path(), "logs/refs/remotes/origin/HEAD").unwrap();
                assert_eq!(log.is_some(), oracle_log.is_some(), "{kind}");
                if let (Some(a), Some(b)) = (log, oracle_log) {
                    assert_eq!(
                        a.split(|b| *b == b'\n').count(),
                        b.split(|b| *b == b'\n').count()
                    );
                }
                assert_eq!(
                    [
                        "config",
                        "HEAD",
                        "index",
                        "refs/remotes/origin/main",
                        "refs/remotes/origin/next"
                    ]
                    .map(|p| read_optional(d.path(), p).unwrap()),
                    before
                );
                assert_eq!(
                    fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
                    "keep dirty"
                );
            }
        }
    }
    #[test]
    fn invalid_missing_symbolic_and_stale_targets_never_change_head() {
        let d = setup("standard");
        let repo = Repository::open(d.path()).unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/alias",
            "refs/remotes/origin/main",
            false,
            "fixture",
        )
        .unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap();
        for target in ["missing", "HEAD", "../main", "alias", "", "HEAD/nested"] {
            assert!(
                session
                    .set_remote_head(&snap.revision, "origin", Some(target))
                    .is_err(),
                "{target}"
            );
        }
        assert!(session
            .set_remote_head(&snap.revision, "missing", Some("main"))
            .is_err());
        fs::write(d.path().join(".git/config.lock"), "foreign").unwrap();
        assert!(session
            .set_remote_head(&snap.revision, "origin", Some("next"))
            .unwrap_err()
            .contains("EXTERNAL_LOCK"));
        assert_eq!(
            fs::read_to_string(d.path().join(".git/config.lock")).unwrap(),
            "foreign"
        );
        fs::remove_file(d.path().join(".git/config.lock")).unwrap();
        repo.config()
            .unwrap()
            .set_str("external.keep", "true")
            .unwrap();
        assert!(session
            .set_remote_head(&snap.revision, "origin", Some("next"))
            .unwrap_err()
            .contains("STALE_STATE"));
        assert_eq!(
            read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap(),
            before
        );
        assert!(!pending(d.path()));
    }
    #[test]
    fn forged_record_cannot_mutate_target_config_or_other_namespace() {
        let d = setup("standard");
        for kind in 0..5 {
            let mut r = prepare(d.path(), "origin", Some("next")).unwrap();
            match kind {
                0 => r.changes[0].after = Some(b"[core]\nbare=true\n".to_vec()),
                1 => r.changes[2].after = None,
                2 => r.changes[3].path = "refs/heads/main".into(),
                3 => r.changes[3].after = Some(b"ref: refs/heads/main\n".to_vec()),
                _ => r.target_oid = Some("a".repeat(40)),
            }
            assert!(verify(&r).is_err());
        }
    }
    #[test]
    fn advertised_default_requires_consistent_unique_head() {
        use crate::network_worker::{Advertisement, RemoteRef};
        let r = |name: &str, sym: Option<&str>| RemoteRef {
            name: name.into(),
            oid: "a".repeat(40),
            symbolic_target: sym.map(str::to_owned),
        };
        let mut ad = Advertisement {
            endpoint: "https://example.invalid/repo".into(),
            refs: vec![
                r("HEAD", Some("refs/heads/next")),
                r("refs/heads/next", None),
                r("refs/heads/main", None),
            ],
        };
        assert_eq!(advertised_branch(&ad).unwrap(), "next");
        ad.refs[0].symbolic_target = None;
        assert!(advertised_branch(&ad).unwrap_err().contains("AMBIGUOUS"));
        ad.refs.pop();
        assert_eq!(advertised_branch(&ad).unwrap(), "next");
        ad.refs[0].symbolic_target = Some("refs/heads/unborn".into());
        assert!(advertised_branch(&ad).is_err());
        ad.refs.clear();
        assert!(advertised_branch(&ad).is_err());
    }
    #[test]
    fn head_worker() {
        let Some(root) = std::env::var_os("BRANCHDESK_HEAD_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        if std::env::var_os("BRANCHDESK_HEAD_RECOVER").is_some() {
            session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
        } else {
            session
                .set_remote_head(
                    &snap.revision,
                    "origin",
                    if root.join(".git/head-test-delete").exists() {
                        None
                    } else {
                        Some("next")
                    },
                )
                .unwrap();
        }
    }
    fn kill(root: &Path, step: usize, recover: bool) {
        let ready = root.join(".git/head-test-ready");
        let _ = fs::remove_file(&ready);
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "repository::remote_head::tests::head_worker",
                "--nocapture",
            ])
            .env("BRANCHDESK_HEAD_ROOT", root)
            .env("BRANCHDESK_HEAD_STEP", step.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if recover {
            command.env("BRANCHDESK_HEAD_RECOVER", "1");
        }
        let mut child = command.spawn().unwrap();
        let start = std::time::Instant::now();
        while !ready.exists() {
            if let Some(s) = child.try_wait().unwrap() {
                panic!("worker exited {s}");
            }
            if start.elapsed() > std::time::Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("checkpoint timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn actual_kill_and_second_recovery_at_every_set_and_delete_boundary() {
        for delete in [false, true] {
            for step in 0..=if delete { 4 } else { 5 } {
                let d = setup("packed");
                let root = d.path().canonicalize().unwrap();
                if delete {
                    fs::write(root.join(".git/head-test-delete"), "1").unwrap();
                }
                let before = ["config", "HEAD", "index", "refs/remotes/origin/next"]
                    .map(|p| read_optional(&root, p).unwrap());
                kill(&root, step, false);
                kill(&root, step, true);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let next = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!next.recovery_pending);
                assert_eq!(
                    read_optional(&root, "refs/remotes/origin/HEAD").unwrap(),
                    if delete {
                        None
                    } else {
                        Some(b"ref: refs/remotes/origin/next\n".to_vec())
                    }
                );
                assert_eq!(
                    ["config", "HEAD", "index", "refs/remotes/origin/next"]
                        .map(|p| read_optional(&root, p).unwrap()),
                    before
                );
            }
        }
    }
    #[test]
    fn recovery_refuses_external_head_target_and_config_changes() {
        for path in [
            "config",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/next",
        ] {
            let d = setup("standard");
            let root = d.path().canonicalize().unwrap();
            kill(&root, 0, false);
            let full = root.join(".git").join(path);
            let mut changed = fs::read(&full).unwrap();
            changed.extend(b"\n");
            fs::write(&full, &changed).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .is_err());
            assert_eq!(fs::read(&full).unwrap(), changed);
            assert!(pending(&root));
        }
    }
}
