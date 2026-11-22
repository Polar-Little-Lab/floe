//! Journaled roll-forward of a local branch rename with guarded metadata publication.
//! Publication must hold HEAD, both branch refs, config and packed-refs locks.
use super::*;
const METADATA_LIMIT: u64 = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Change {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub version: u32,
    pub old: String,
    pub new: String,
    pub oid: String,
    pub current: bool,
    #[serde(default)]
    pub unborn: bool,
    #[serde(default)]
    pub event_time: Option<(i64, i32)>,
    pub changes: Vec<Change>,
}
fn branch_name(name: &str) -> Result<String> {
    let full = format!("refs/heads/{name}");
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || name == "HEAD"
        || !git2::Reference::is_valid_name(&full)
    {
        return Err("INVALID_BRANCH_NAME".into());
    }
    Ok(full)
}
pub(super) fn read_optional(root: &Path, path: &str) -> Result<Option<Vec<u8>>> {
    let mut checked = root.join(".git");
    for component in Path::new(path).components() {
        let Component::Normal(part) = component else {
            return Err("INVALID_METADATA_PATH".into());
        };
        checked.push(part);
        match fs::symlink_metadata(&checked) {
            Ok(m) if is_link(&m) => return Err("UNSUPPORTED_SYMLINK".into()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(e)),
        }
    }
    bounded(&checked, METADATA_LIMIT).map(Some)
}
fn config_after(bytes: &[u8], old: &str, new: &str) -> Result<Vec<u8>> {
    remote_remove::entries(bytes)?;
    // Edit a private copy with libgit2's parser. Never load user/system config.
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, bytes).map_err(err)?;
    let mut config = Config::open(&path).map_err(err)?;
    let from = format!("branch.{old}");
    let to = format!("branch.{new}");
    let mut values: Vec<(String, String)> = Vec::new();
    {
        let mut entries = config.entries(None).map_err(err)?;
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(err)?;
            let name = entry.name().map_err(err)?;
            let Some((section, key)) = name.rsplit_once('.') else {
                continue;
            };
            if section == to {
                return Err("BRANCH_CONFIG_EXISTS".into());
            }
            if section == from {
                values.push((key.to_string(), entry.value().map_err(err)?.to_string()));
            }
        }
    }
    if values.is_empty() {
        return Ok(bytes.to_vec());
    }
    let keys: std::collections::HashSet<_> = values.iter().map(|(key, _)| key).collect();
    for key in keys {
        config
            .remove_multivar(&format!("{from}.{key}"), ".*")
            .map_err(err)?;
    }
    for (key, value) in values {
        config
            .set_multivar(&format!("{to}.{key}"), "a^", &value)
            .map_err(err)?;
    }
    drop(config);
    bounded(&path, 65536)
}
pub(super) fn logging_enabled(bytes: &[u8]) -> Result<bool> {
    remote_remove::entries(bytes)?;

    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, bytes).map_err(err)?;
    let config = Config::open(&path).map_err(err)?;
    match config.get_bool("core.logallrefupdates") {
        Ok(value) => Ok(value),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(true),
        Err(_) if config.get_string("core.logallrefupdates").ok().as_deref() == Some("always") => {
            Ok(true)
        }
        Err(e) => Err(err(e)),
    }
}
fn event_line(oid: &str, old: &str, new: &str, time: (i64, i32)) -> Result<Vec<u8>> {
    let (seconds, offset) = time;
    if seconds < 0 || !(-1439..=1439).contains(&offset) {
        return Err("INVALID_RENAME_TIME".into());
    }
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.abs();
    Ok(format!("{oid} {oid} FLOE <local@floe.invalid> {seconds} {sign}{:02}{:02}\tBranch: renamed refs/heads/{old} to refs/heads/{new}\n",minutes/60,minutes%60).into_bytes())
}
fn append_event(before: &Option<Vec<u8>>, event: &[u8], enabled: bool) -> Option<Vec<u8>> {
    if before.is_none() && !enabled {
        return None;
    }
    let mut bytes = before.clone().unwrap_or_default();
    bytes.extend_from_slice(event);
    Some(bytes)
}
pub(super) fn without_packed_ref(bytes: &[u8], old: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes).map_err(|_| "INVALID_PACKED_REFS")?;
    let mut out = String::new();
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        if line
            .trim_end()
            .split_once(' ')
            .is_some_and(|(_, name)| name == old)
        {
            removed = true;
            continue;
        }
        if removed && line.starts_with('^') {
            removed = false;
            continue;
        }
        removed = false;
        out.push_str(line);
    }
    Ok(out.into_bytes())
}
pub(super) fn prepare(root: &Path, repo: &Repository, old: &str, new: &str) -> Result<Plan> {
    if let Some(reason) = validate(root)? {
        return Err(format!("READ_ONLY: {reason}"));
    }
    if repo.state() != git2::RepositoryState::Clean {
        return Err("UNSUPPORTED_STATE".into());
    }
    let from = branch_name(old)?;
    let to = branch_name(new)?;
    if from == to {
        return Err("SAME_BRANCH_NAME".into());
    }
    // Parent/child names cannot coexist while a rename is partly published.
    if from.starts_with(&(to.clone() + "/")) || to.starts_with(&(from.clone() + "/")) {
        return Err("REF_DIRECTORY_COLLISION".into());
    }
    let unborn = matches!(repo.head(), Err(ref e) if e.code() == git2::ErrorCode::UnbornBranch)
        && repo
            .find_reference("HEAD")
            .map_err(err)?
            .symbolic_target()
            .map_err(err)?
            == Some(from.as_str());
    let oid = if unborn {
        Oid::ZERO_SHA1
    } else {
        let branch = repo
            .find_branch(old, git2::BranchType::Local)
            .map_err(err)?;
        let oid = branch.get().target().ok_or("SYMBOLIC_BRANCH_UNSUPPORTED")?;
        repo.find_commit(oid).map_err(err)?;
        oid
    };
    match repo.find_reference(&to) {
        Ok(_) => return Err("REF_EXISTS".into()),
        Err(e) if e.code() == git2::ErrorCode::NotFound => {}
        Err(e) => return Err(err(e)),
    }
    if read_optional(root, &to)?.is_some() {
        return Err("REF_EXISTS".into());
    }
    for reference in repo.references().map_err(err)? {
        let reference = reference.map_err(err)?;
        let name = reference.name().map_err(err)?;
        if name.starts_with(&(to.clone() + "/")) || to.starts_with(&(name.to_string() + "/")) {
            return Err("REF_DIRECTORY_COLLISION".into());
        }
    }
    let head = read_optional(root, "HEAD")?.ok_or("MISSING_HEAD")?;
    let current = repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        == Some(from.as_str());
    let config = read_optional(root, "config")?.ok_or("MISSING_CONFIG")?;
    let renamed_config = config_after(&config, old, new)?;
    let logging = logging_enabled(&config)?;
    let signature = git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?;
    let when = signature.when();
    let event_time = (when.seconds(), when.offset_minutes());
    let event = event_line(&oid.to_string(), old, new, event_time)?;
    let old_log = format!("logs/{from}");
    let new_log = format!("logs/{to}");
    let log = read_optional(root, &old_log)?;
    if read_optional(root, &new_log)?.is_some() {
        return Err("REFLOG_EXISTS".into());
    }
    // No ref or reflog is created before the first commit. Preserve absence
    // explicitly in the journal so external creation prevents recovery.
    if unborn && (log.is_some() || read_optional(root, "logs/HEAD")?.is_some()) {
        return Err("UNSUPPORTED_UNBORN_REFLOG".into());
    }
    let mut changes = vec![
        Change {
            path: to,
            before: None,
            after: (!unborn).then(|| format!("{oid}\n").into_bytes()),
        },
        Change {
            path: "config".into(),
            before: Some(config),
            after: Some(renamed_config),
        },
        Change {
            path: new_log,
            before: None,
            after: if unborn {
                None
            } else {
                append_event(&log, &event, logging)
            },
        },
        Change {
            path: "HEAD".into(),
            before: Some(head.clone()),
            after: Some(if current {
                format!("ref: refs/heads/{new}\n").into_bytes()
            } else {
                head
            }),
        },
        Change {
            path: from.clone(),
            before: read_optional(root, &from)?,
            after: None,
        },
        Change {
            path: old_log,
            before: log,
            after: None,
        },
    ];
    if let Some(packed) = read_optional(root, "packed-refs")? {
        changes.push(Change {
            path: "packed-refs".into(),
            after: Some(without_packed_ref(&packed, &from)?),
            before: Some(packed),
        });
    }
    if current && !unborn {
        let head_log = read_optional(root, "logs/HEAD")?;
        changes.push(Change {
            path: "logs/HEAD".into(),
            after: append_event(&head_log, &event, logging),
            before: head_log,
        });
    }
    Ok(Plan {
        version: 1,
        event_time: (!unborn).then_some(event_time),
        unborn,
        old: old.into(),
        new: new.into(),
        oid: oid.to_string(),
        current,
        changes,
    })
}

const JOURNAL: &str = ".git/branchdesk-branch-rename";
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
struct MetadataLocks(Vec<(PathBuf, String)>);
impl Drop for MetadataLocks {
    fn drop(&mut self) {
        for (path, identity) in &self.0 {
            if lease::identity(path).ok().as_ref() == Some(identity)
                && bounded(path, 1).ok().as_deref() == Some(&[])
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn metadata_locks(root: &Path) -> Result<MetadataLocks> {
    let mut locks = MetadataLocks(vec![]);
    for name in ["config.lock", "packed-refs.lock"] {
        let path = root.join(".git").join(name);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(err)?;
        let identity = lease::identity(&path)?;
        locks.0.push((path, identity));
    }
    Ok(locks)
}
fn record_locks(root: &Path, dir: &Path, plan: &Plan) -> Result<()> {
    journal_locks::record_paths(
        root,
        dir,
        vec![
            "index.lock".into(),
            "HEAD.lock".into(),
            "config.lock".into(),
            "packed-refs.lock".into(),
            format!("refs/heads/{}.lock", plan.old),
            format!("refs/heads/{}.lock", plan.new),
        ],
    )
}
fn verify_plan(plan: &Plan) -> Result<()> {
    let from = branch_name(&plan.old)?;
    let to = branch_name(&plan.new)?;
    if plan.version != 1
        || from == to
        || from.starts_with(&(to.clone() + "/"))
        || to.starts_with(&(from.clone() + "/"))
    {
        return Err("INVALID_RENAME_JOURNAL".into());
    }
    Oid::from_str(&plan.oid).map_err(err)?;
    let mut expected = vec![
        to.clone(),
        "config".into(),
        format!("logs/{to}"),
        "HEAD".into(),
        from.clone(),
        format!("logs/{from}"),
    ];
    if plan.changes.iter().any(|c| c.path == "packed-refs") {
        expected.push("packed-refs".into());
    }
    if plan.event_time.is_some() && plan.current {
        expected.push("logs/HEAD".into());
    }
    if plan.changes.iter().map(|c| &c.path).ne(expected.iter()) {
        return Err("INVALID_RENAME_PATHS".into());
    }
    for change in &plan.changes {
        if change
            .before
            .iter()
            .chain(change.after.iter())
            .any(|b| b.len() as u64 > METADATA_LIMIT)
        {
            return Err("INVALID_RENAME_SIZE".into());
        }
    }
    let c = &plan.changes;
    let target_bytes = (!plan.unborn).then(|| format!("{}\n", plan.oid).into_bytes());
    if plan.unborn
        && (!plan.current
            || plan.oid != Oid::ZERO_SHA1.to_string()
            || plan.event_time.is_some()
            || c[4].before.is_some()
            || c[5].before.is_some())
    {
        return Err("INVALID_UNBORN_RENAME".into());
    }
    if c[0].before.is_some()
        || c[0].after != target_bytes
        || c[2].before.is_some()
        || c[4].after.is_some()
        || c[5].after.is_some()
    {
        return Err("INVALID_RENAME_CONTENT".into());
    }
    if let Some(time) = plan.event_time {
        let event = event_line(&plan.oid, &plan.old, &plan.new, time)?;
        let logging = logging_enabled(c[1].before.as_deref().ok_or("INVALID_RENAME_CONFIG")?)?;
        if c[2].after != append_event(&c[5].before, &event, logging) {
            return Err("INVALID_RENAME_REFLOG".into());
        }
        if plan.current {
            let log = c.last().ok_or("INVALID_RENAME_HEAD_LOG")?;
            if log.after != append_event(&log.before, &event, logging) {
                return Err("INVALID_RENAME_HEAD_LOG".into());
            }
        }
    } else if c[2].after != c[5].before {
        return Err("INVALID_RENAME_REFLOG".into());
    }
    let head = c[3].before.as_deref().ok_or("INVALID_RENAME_HEAD")?;
    let old_head = format!("ref: {from}\n");
    if plan.current != (head == old_head.as_bytes())
        || c[3].after.as_deref()
            != Some(if plan.current {
                format!("ref: {to}\n").into_bytes()
            } else {
                head.to_vec()
            })
            .as_deref()
    {
        return Err("INVALID_RENAME_HEAD".into());
    }
    if c[1].after
        != Some(config_after(
            c[1].before.as_deref().ok_or("INVALID_RENAME_CONFIG")?,
            &plan.old,
            &plan.new,
        )?)
    {
        return Err("INVALID_RENAME_CONFIG".into());
    }
    if c.get(6).is_some_and(|c| c.path == "packed-refs")
        && c[6].after
            != Some(without_packed_ref(
                c[6].before.as_deref().ok_or("INVALID_PACKED_REFS")?,
                &from,
            )?)
    {
        return Err("INVALID_RENAME_PACKED_REFS".into());
    }
    if c[4]
        .before
        .as_ref()
        .is_some_and(|b| b != format!("{}\n", plan.oid).as_bytes())
    {
        return Err("INVALID_RENAME_SOURCE".into());
    }
    Ok(())
}
fn apply(root: &Path, repo: &Repository, plan: &Plan) -> Result<()> {
    verify_plan(plan)?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err("RECOVERY_CONFLICT: repository state changed".into());
    }
    if plan.unborn {
        for name in [&plan.old, &plan.new] {
            match repo.find_reference(&branch_name(name)?) {
                Err(e) if e.code() == git2::ErrorCode::NotFound => {}
                _ => return Err("RECOVERY_CONFLICT: unborn branch now exists".into()),
            }
        }
        if read_optional(root, "logs/HEAD")?.is_some() {
            return Err("RECOVERY_CONFLICT: unborn HEAD reflog now exists".into());
        }
    } else {
        repo.find_commit(Oid::from_str(&plan.oid).map_err(err)?)
            .map_err(err)?;
    }
    // Verify the whole set before any publication. Unrelated worktree edits are fine.
    for change in &plan.changes {
        let now = read_optional(root, &change.path)?;
        if now != change.before && now != change.after {
            return Err(format!("RECOVERY_CONFLICT: {} changed", change.path));
        }
    }
    checkpoint(root, 0);
    for (index, change) in plan.changes.iter().enumerate() {
        let now = read_optional(root, &change.path)?;
        if now != change.after {
            if now != change.before {
                return Err(format!("RECOVERY_CONFLICT: {} changed", change.path));
            }
            let path = root.join(".git").join(&change.path);
            if let Some(bytes) = &change.after {
                let parent = path.parent().ok_or("INVALID_METADATA_PATH")?;
                fs::create_dir_all(parent).map_err(err)?;
                let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(err)?;
                std::io::Write::write_all(&mut temp, bytes).map_err(err)?;
                temp.as_file().sync_all().map_err(err)?;
                temp.persist(&path).map_err(err)?;
                checkout::sync_dir(parent)?;
            } else if now.is_some() {
                fs::remove_file(&path).map_err(err)?;
                checkout::sync_dir(path.parent().ok_or("INVALID_METADATA_PATH")?)?;
            }
        }
        checkpoint(root, index + 1);
    }
    Ok(())
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    old: &str,
    new: &str,
) -> Result<()> {
    let initial = prepare(&session.root, repo, old, new)?;
    let _refs = checkout::locks(repo, &branch_name(old)?, &branch_name(new)?)?;
    let _metadata = metadata_locks(&session.root)?;
    session.fresh(revision)?;
    let plan = prepare(&session.root, repo, old, new)?;
    if plan.oid != initial.oid {
        return Err("STALE_STATE".into());
    }
    verify_plan(&plan)?;
    let dir = session.root.join(JOURNAL);
    fs::create_dir(&dir).map_err(err)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&plan).map_err(err)?,
    )?;
    record_locks(&session.root, &dir, &plan)?;
    checkout::sync_dir(&session.root.join(".git"))?;
    apply(&session.root, repo, &plan)
}
pub(super) fn recover(root: &Path, repo: &Repository) -> Result<()> {
    let dir = root.join(JOURNAL);
    let metadata = fs::symlink_metadata(&dir).map_err(err)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err("INVALID_RENAME_JOURNAL_DIRECTORY".into());
    }
    let plan: Plan = serde_json::from_slice(&bounded(&dir.join("record.json"), 64 * 1024 * 1024)?)
        .map_err(err)?;
    verify_plan(&plan)?;
    let _refs = checkout::locks(repo, &branch_name(&plan.old)?, &branch_name(&plan.new)?)?;
    let _metadata = metadata_locks(root)?;
    record_locks(root, &dir, &plan)?;
    apply(root, repo, &plan)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!("{JOURNAL}-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_RENAME_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_RENAME_STEP").ok().as_deref()
            == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/rename-test-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Repository) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initial",
                &repo.find_tree(tree_id).unwrap(),
                &[],
            )
            .unwrap();
        repo.branch("source", &repo.find_commit(oid).unwrap(), false)
            .unwrap();
        repo.set_head("refs/heads/source").unwrap();
        repo.index().unwrap().write().unwrap();
        (d, repo)
    }
    fn after<'a>(plan: &'a Plan, path: &str) -> &'a Option<Vec<u8>> {
        &plan.changes.iter().find(|c| c.path == path).unwrap().after
    }
    #[test]
    fn rename_publishes_metadata_preserving_index_worktree_and_history() {
        let (d, repo) = fixture();
        fs::write(d.path().join("dirty.txt"), "staged").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("dirty.txt")).unwrap();
        index.write().unwrap();
        fs::write(d.path().join("dirty.txt"), "working").unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        let log = fs::read(d.path().join(".git/logs/refs/heads/source")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let result = session
            .mutate(
                &snap.revision,
                Action::RenameBranch {
                    name: "source".into(),
                    new_name: "renamed".into(),
                },
            )
            .unwrap();
        assert_eq!(result.branch, "renamed");
        assert!(!result.recovery_pending);
        assert_eq!(repo.head().unwrap().shorthand().unwrap(), "renamed");
        assert!(repo.find_branch("source", git2::BranchType::Local).is_err());
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
        assert_eq!(
            fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
            "working"
        );
        let renamed = fs::read(d.path().join(".git/logs/refs/heads/renamed")).unwrap();
        assert!(renamed.starts_with(&log));
        assert!(String::from_utf8(renamed)
            .unwrap()
            .ends_with("Branch: renamed refs/heads/source to refs/heads/renamed\n"));
    }
    fn unborn_fixture(staged: bool) -> (tempfile::TempDir, Repository) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/source").unwrap();
        let mut config = Config::open(&d.path().join(".git/config")).unwrap();
        config.set_str("branch.source.remote", "origin").unwrap();
        config
            .set_str("branch.source.merge", "refs/heads/main")
            .unwrap();
        fs::write(d.path().join("file.txt"), "staged content").unwrap();
        if staged {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("file.txt")).unwrap();
            index.write().unwrap();
        }
        fs::write(d.path().join("file.txt"), "working content").unwrap();
        (d, repo)
    }
    fn cli(root: &Path, args: &[&str]) -> Vec<u8> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    #[test]
    fn unborn_rename_matches_git_and_preserves_absent_or_staged_index() {
        for staged in [false, true] {
            let (d, repo) = unborn_fixture(staged);
            let (oracle, _) = unborn_fixture(staged);
            let root = d.path().canonicalize().unwrap();
            let before = fs::read(root.join(".git/index")).ok();
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(snapshot
                .branches
                .iter()
                .any(|b| b.name == "source" && b.current && b.oid.is_empty()));
            let result = session
                .mutate(
                    &snapshot.revision,
                    Action::RenameBranch {
                        name: "source".into(),
                        new_name: "feature/renamed".into(),
                    },
                )
                .unwrap();
            assert_eq!(result.branch, "feature/renamed");
            assert!(!result.recovery_pending);
            assert!(matches!(repo.head(), Err(ref e) if e.code() == git2::ErrorCode::UnbornBranch));
            assert_eq!(fs::read(root.join(".git/index")).ok(), before);
            assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"working content");
            cli(
                oracle.path(),
                &["branch", "-m", "source", "feature/renamed"],
            );
            for args in [
                vec!["symbolic-ref", "HEAD"],
                vec!["config", "--get-regexp", "^branch\\."],
                vec!["ls-files", "--stage"],
                vec!["status", "--porcelain=v1"],
            ] {
                assert_eq!(cli(&root, &args), cli(oracle.path(), &args));
            }
            assert!(!root.join(".git/refs/heads/feature/renamed").exists());
            assert!(!root.join(".git/logs/refs/heads/feature/renamed").exists());
            assert!(!root.join(".git/logs/HEAD").exists());
        }
    }
    #[test]
    fn unborn_rename_first_commit_targets_new_branch() {
        let (d, repo) = unborn_fixture(true);
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let renamed = session
            .mutate(
                &snap.revision,
                Action::RenameBranch {
                    name: "source".into(),
                    new_name: "renamed".into(),
                },
            )
            .unwrap();
        let committed = session
            .mutate(
                &renamed.revision,
                Action::Commit {
                    message: "First commit".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                },
            )
            .unwrap();
        assert_eq!(committed.branch, "renamed");
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(commit.parent_count(), 0);
        let tree = commit.tree().unwrap();
        let entry = tree.get_name("file.txt").unwrap();
        assert_eq!(
            repo.find_blob(entry.id()).unwrap().content(),
            b"staged content"
        );
        assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"working content");
        assert!(repo.find_reference("refs/heads/source").is_err());
    }
    #[test]
    fn unborn_recovery_refuses_externally_created_packed_destination() {
        let (d, repo) = unborn_fixture(false);
        let root = d.path().canonicalize().unwrap();
        killed(&root, 2, false);
        let sig = git2::Signature::now("External", "external@example.invalid").unwrap();
        let tree = repo.treebuilder(None).unwrap().write().unwrap();
        let oid = repo
            .commit(
                None,
                &sig,
                &sig,
                "external",
                &repo.find_tree(tree).unwrap(),
                &[],
            )
            .unwrap();
        let packed = format!("# pack-refs with: peeled\n{oid} refs/heads/renamed\n");
        fs::write(root.join(".git/packed-refs"), &packed).unwrap();
        let head = fs::read(root.join(".git/HEAD")).unwrap();
        let config = fs::read(root.join(".git/config")).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session
            .mutate(&snap.revision, Action::RecoverCheckout)
            .err()
            .unwrap()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head);
        assert_eq!(fs::read(root.join(".git/config")).unwrap(), config);
        assert_eq!(
            fs::read(root.join(".git/packed-refs")).unwrap(),
            packed.as_bytes()
        );
        assert!(pending(&root));
    }
    #[test]
    fn unborn_actual_kill_each_boundary_recovers_without_creating_commit_or_index() {
        for staged in [false, true] {
            for step in 0..=6 {
                let (d, repo) = unborn_fixture(staged);
                let root = d.path().canonicalize().unwrap();
                let before = fs::read(root.join(".git/index")).ok();
                killed(&root, step, false);
                if step == 2 {
                    killed(&root, 4, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let result = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(result.branch, "renamed");
                assert!(!result.recovery_pending);
                assert!(
                    matches!(repo.head(), Err(ref e) if e.code() == git2::ErrorCode::UnbornBranch)
                );
                assert_eq!(fs::read(root.join(".git/index")).ok(), before);
                assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"working content");
                assert!(!root.join(".git/logs/HEAD").exists());
                assert_eq!(
                    cli(&root, &["config", "--get", "branch.renamed.remote"]),
                    b"origin\n"
                );
            }
        }
    }
    #[test]
    fn rename_child_job() {
        let Some(root) = std::env::var_os("BRANCHDESK_RENAME_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut session = Session::new(root).unwrap();
        let snap = session.snapshot().unwrap();
        let action = if std::env::var_os("BRANCHDESK_RENAME_RECOVER").is_some() {
            Action::RecoverCheckout
        } else {
            Action::RenameBranch {
                name: "source".into(),
                new_name: "renamed".into(),
            }
        };
        session.mutate(&snap.revision, action).unwrap();
    }
    fn killed(root: &Path, step: usize, recovery: bool) {
        let ready = root.join(".git/rename-test-ready");
        let _ = fs::remove_file(&ready);
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "repository::branch_rename::tests::rename_child_job",
            "--nocapture",
        ])
        .env("BRANCHDESK_RENAME_ROOT", root)
        .env("BRANCHDESK_RENAME_STEP", step.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
        if recovery {
            cmd.env("BRANCHDESK_RENAME_RECOVER", "1");
        }
        let mut child = cmd.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                panic!("child failed: {}", String::from_utf8_lossy(&output.stderr));
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("rename checkpoint timeout {step}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn actual_kill_each_publication_boundary_and_second_recovery_resume() {
        for packed in [false, true] {
            for step in 0..=if packed { 8 } else { 7 } {
                let (d, repo) = fixture();
                let root = d.path().canonicalize().unwrap();
                let oid = repo.head().unwrap().target().unwrap();
                fs::write(root.join("dirty.txt"), "keep working data").unwrap();
                if packed {
                    fs::write(
                        root.join(".git/packed-refs"),
                        format!("# pack-refs with: peeled\n{oid} refs/heads/source\n"),
                    )
                    .unwrap();
                    fs::remove_file(root.join(".git/refs/heads/source")).unwrap();
                }
                killed(&root, step, false);
                if step == 3 {
                    killed(&root, 4, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                assert!(snap.write_block.is_some());
                let recovered = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!recovered.recovery_pending);
                let repo = open(&root).unwrap();
                assert_eq!(repo.head().unwrap().shorthand().unwrap(), "renamed");
                assert_eq!(repo.head().unwrap().target(), Some(oid));
                assert!(repo.find_branch("source", git2::BranchType::Local).is_err());
                for log in ["logs/refs/heads/renamed", "logs/HEAD"] {
                    let bytes = fs::read_to_string(root.join(".git").join(log)).unwrap();
                    assert_eq!(
                        bytes
                            .matches("Branch: renamed refs/heads/source to refs/heads/renamed")
                            .count(),
                        1
                    );
                }

                assert_eq!(
                    fs::read_to_string(root.join("dirty.txt")).unwrap(),
                    "keep working data"
                );
            }
        }
    }
    #[test]
    fn legacy_journal_without_event_remains_recoverable() {
        let (d, repo) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut plan = prepare(&root, &repo, "source", "renamed").unwrap();
        plan.event_time = None;
        plan.changes.retain(|c| c.path != "logs/HEAD");
        plan.changes[2].after = plan.changes[5].before.clone();
        let mut value = serde_json::to_value(&plan).unwrap();
        value.as_object_mut().unwrap().remove("event_time");
        fs::create_dir(root.join(JOURNAL)).unwrap();
        fs::write(
            root.join(JOURNAL).join("record.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let mut session = Session::new(root).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(snap.recovery_pending);
        assert_eq!(
            session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap()
                .branch,
            "renamed"
        );
    }
    #[test]
    fn recovery_refuses_foreign_metadata_and_invalid_journal_paths() {
        let (d, _repo) = fixture();
        let root = d.path().canonicalize().unwrap();
        killed(&root, 1, false);
        let config = root.join(".git/config");
        let mut bytes = fs::read(&config).unwrap();
        bytes.extend_from_slice(b"\n# external edit\n");
        fs::write(&config, &bytes).unwrap();
        let lease = lease::OperationLease::acquire(&root).unwrap();
        journal_locks::release(&root, &lease).unwrap();
        let repo = open(&root).unwrap();
        assert!(recover(&root, &repo)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(&config).unwrap(), bytes);
        let record = root.join(JOURNAL).join("record.json");
        let mut plan: Plan = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        plan.changes[0].path = "../../outside".into();
        fs::write(&record, serde_json::to_vec(&plan).unwrap()).unwrap();
        assert!(recover(&root, &repo)
            .unwrap_err()
            .contains("INVALID_RENAME_PATHS"));
    }
    #[test]
    fn preparation_moves_tracking_multivars_and_preserves_repository_bytes() {
        let (d, repo) = fixture();
        let config_path = d.path().join(".git/config");
        let mut cfg = Config::open(&config_path).unwrap();
        cfg.set_str("branch.source.remote", "origin").unwrap();
        cfg.set_multivar("branch.source.merge", "a^", "refs/heads/main")
            .unwrap();
        cfg.set_multivar("branch.source.merge", "a^", "refs/heads/extra")
            .unwrap();
        cfg.set_str("branch.source.child.remote", "keep").unwrap();
        drop(cfg);
        let config = fs::read(&config_path).unwrap();
        let head = fs::read(d.path().join(".git/HEAD")).unwrap();
        let log = fs::read(d.path().join(".git/logs/refs/heads/source")).unwrap();
        let plan = prepare(d.path(), &repo, "source", "new.branch").unwrap();
        assert!(plan.current);
        assert_eq!(
            after(&plan, "HEAD"),
            &Some(b"ref: refs/heads/new.branch\n".to_vec())
        );
        assert!(after(&plan, "logs/refs/heads/new.branch")
            .as_ref()
            .unwrap()
            .starts_with(&log));
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), after(&plan, "config").as_ref().unwrap()).unwrap();
        let cfg = Config::open(temp.path()).unwrap();
        assert_eq!(
            cfg.get_string("branch.new.branch.remote").unwrap(),
            "origin"
        );
        assert_eq!(
            cfg.get_string("branch.source.child.remote").unwrap(),
            "keep"
        );
        let mut values = cfg.multivar("branch.new.branch.merge", None).unwrap();
        let mut found = Vec::new();
        while let Some(v) = values.next() {
            found.push(v.unwrap().value().unwrap().to_owned());
        }
        assert_eq!(found, vec!["refs/heads/main", "refs/heads/extra"]);
        assert!(cfg.get_string("branch.source.remote").is_err());
        assert_eq!(fs::read(config_path).unwrap(), config);
        assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        assert!(repo
            .find_branch("new.branch", git2::BranchType::Local)
            .is_err());
    }
    #[test]
    fn rejects_collisions_bad_names_and_destination_settings() {
        let (d, repo) = fixture();
        for name in [
            "source",
            "HEAD",
            "-option",
            "a..b",
            "bad\0name",
            "source/child",
        ] {
            assert!(prepare(d.path(), &repo, "source", name).is_err());
        }
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("taken", &commit, false).unwrap();
        assert!(prepare(d.path(), &repo, "source", "taken").is_err());
        Config::open(&d.path().join(".git/config"))
            .unwrap()
            .set_str("branch.destination.remote", "origin")
            .unwrap();
        assert!(prepare(d.path(), &repo, "source", "destination")
            .unwrap_err()
            .contains("BRANCH_CONFIG_EXISTS"));
    }
    #[test]
    fn noncurrent_and_detached_head_remain_exact_and_packed_source_is_removed() {
        let (d, repo) = fixture();
        let oid = repo.head().unwrap().target().unwrap();
        repo.set_head_detached(oid).unwrap();
        fs::write(
            d.path().join(".git/packed-refs"),
            format!(
                "# pack-refs with: peeled\n{oid} refs/heads/source\n{oid} refs/heads/untouched\n"
            ),
        )
        .unwrap();
        fs::remove_file(d.path().join(".git/refs/heads/source")).unwrap();
        let plan = prepare(d.path(), &repo, "source", "renamed").unwrap();
        assert!(!plan.current);
        assert_eq!(
            after(&plan, "HEAD"),
            &Some(fs::read(d.path().join(".git/HEAD")).unwrap())
        );
        assert!(
            !String::from_utf8(after(&plan, "packed-refs").clone().unwrap())
                .unwrap()
                .contains("refs/heads/source")
        );
        assert!(
            String::from_utf8(after(&plan, "packed-refs").clone().unwrap())
                .unwrap()
                .contains("refs/heads/untouched")
        );
    }
}
