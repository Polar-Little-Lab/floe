//! Clean-worktree checkout with a durable, guarded rollback record.
//! This is not a filesystem sandbox or a multi-file atomic transaction.
use super::*;
use std::io::Write;

const JOURNAL: &str = ".git/branchdesk-checkout";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) path: String,
    pub(super) before: Option<String>,
    pub(super) after: Option<String>,
    pub(super) before_mode: u32,
    pub(super) after_mode: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    #[serde(default)]
    preparing: bool,
    #[serde(default)]
    fast_forward: bool,
    #[serde(default)]
    sequence_rewind: bool,
    #[serde(default)]
    merge_source: Option<String>,
    #[serde(default)]
    replay: Option<replay::Record>,
    #[serde(default)]
    rebase: Option<rebase::execution::Record>,
    old_branch: String,
    new_branch: String,
    old_oid: String,
    new_oid: String,
    before_index: String,
    after_index: String,
    entries: Vec<Entry>,
    #[serde(default)]
    locks: Vec<OwnedLock>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedLock {
    path: String,
    identity: String,
    digest: String,
}
fn validate_header(record: &Journal) -> Result<()> {
    if !matches!(record.version, 1..=4)
        || (record.version != 4 && record.rebase.is_some())
        || (record.version == 2
            && (record.fast_forward
                || record.sequence_rewind
                || record.replay.is_some()
                || record.merge_source.is_some()
                || (record.old_branch != "HEAD" && record.new_branch != "HEAD")))
    {
        return Err("INVALID_JOURNAL".into());
    }
    if record.version == 3
        && (!record.fast_forward
            || record.sequence_rewind
            || record.replay.is_some()
            || !record.old_branch.starts_with("refs/heads/")
            || !record.new_branch.starts_with("refs/remotes/"))
    {
        return Err("INVALID_JOURNAL".into());
    }
    if record.version == 4
        && (record.rebase.is_none()
            || !record.fast_forward
            || record.sequence_rewind
            || record.replay.is_some()
            || record.merge_source.is_some()
            || record.old_branch != record.new_branch)
    {
        return Err("INVALID_REBASE_JOURNAL".into());
    }
    for (index, name) in [&record.old_branch, &record.new_branch]
        .into_iter()
        .enumerate()
    {
        if !(record.version == 2 && name == "HEAD")
            && (!(name.starts_with("refs/heads/")
                || (record.version == 3 && index == 1 && name.starts_with("refs/remotes/")))
                || !git2::Reference::is_valid_name(name))
        {
            return Err("INVALID_JOURNAL_REF".into());
        }
    }
    Ok(())
}
fn lock_paths(old: &str, new: &str) -> Vec<String> {
    let mut paths = vec![
        "index.lock".into(),
        "HEAD.lock".into(),
        format!("{old}.lock"),
        format!("{new}.lock"),
    ];
    paths.sort();
    paths.dedup();
    paths
}
fn capture_locks(root: &Path, old: &str, new: &str) -> Result<Vec<OwnedLock>> {
    lock_paths(old, new)
        .into_iter()
        .map(|path| {
            let full = root.join(".git").join(&path);
            Ok(OwnedLock {
                identity: lease::identity(&full)?,
                digest: digest(&bounded(&full, 20 * 1024 * 1024)?),
                path,
            })
        })
        .collect()
}
pub(super) fn release_owned_locks(root: &Path, _lease: &lease::OperationLease) -> Result<()> {
    let record: Journal = serde_json::from_slice(&bounded(
        &root.join(JOURNAL).join("manifest.json"),
        32 * 1024 * 1024,
    )?)
    .map_err(err)?;
    validate_header(&record)?;
    let allowed = lock_paths(&record.old_branch, &record.new_branch);
    let mut seen = std::collections::HashSet::new();
    for lock in &record.locks {
        if !allowed.contains(&lock.path) || !seen.insert(&lock.path) {
            return Err("INVALID_JOURNAL_LOCK".into());
        }
        let path = root.join(".git").join(&lock.path);
        if path.exists()
            && (lease::identity(&path)? != lock.identity
                || digest(&bounded(&path, 20 * 1024 * 1024)?) != lock.digest)
        {
            return Err("EXTERNAL_LOCK: 복구 기록과 다른 잠금을 보존했습니다.".into());
        }
    }
    // No other FLOE process holds the kernel lease. Existing Git markers
    // are removed only when their file identity AND bytes match our checkpoint.
    // Hostile path replacement races still require the planned handle sandbox.
    for lock in &record.locks {
        let path = root.join(".git").join(&lock.path);
        if path.exists() {
            if lease::identity(&path)? != lock.identity
                || digest(&bounded(&path, 20 * 1024 * 1024)?) != lock.digest
            {
                return Err("EXTERNAL_LOCK".into());
            }
            fs::remove_file(path).map_err(err)?;
        }
    }
    sync_dir(&root.join(".git"))
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn update_record(dir: &Path, record: &Journal) -> Result<()> {
    if serde_json::to_vec(record).map_err(err)?.len() > 32 * 1024 * 1024 {
        return Err("LIMIT: checkout journal".into());
    }
    let next = dir.join(format!("manifest-{}.next", Uuid::new_v4()));
    save(&next, &serde_json::to_vec(record).map_err(err)?)?;
    fs::rename(next, dir.join("manifest.json")).map_err(err)?;
    sync_dir(dir)
}
pub(super) fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path).map_err(err)?.sync_all().map_err(err)?;
    }
    // Windows directory durability still requires native power-loss verification.
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
pub(super) fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)
}
#[cfg(unix)]
fn mode(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(
            if fs::metadata(path).map_err(err)?.permissions().mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            },
        )
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(0o100644)
    }
}
fn content(root: &Path, entry: &Entry) -> Result<Option<String>> {
    let path = rel(root, Path::new(&entry.path))?;
    match fs::symlink_metadata(&path) {
        Ok(_) => Ok(Some(digest(&bounded(&path, FILE_LIMIT)?))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
pub(super) fn matches(root: &Path, e: &Entry, after: bool) -> Result<bool> {
    let expected = if after { &e.after } else { &e.before };
    if &content(root, e)? != expected {
        return Ok(false);
    }
    #[cfg(unix)]
    if expected.is_some()
        && mode(&rel(root, Path::new(&e.path))?)?
            != if after { e.after_mode } else { e.before_mode }
    {
        return Ok(false);
    }
    Ok(true)
}
fn safe_name(path: &str) -> bool {
    path.split('/').all(|part| {
        let upper = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !part.is_empty()
            && !part.to_ascii_uppercase().starts_with("GIT~")
            && !part.chars().any(|c| matches!(c as u32, 0x200c..=0x200f | 0x202a..=0x202e | 0x206a..=0x206f | 0xfeff))
            && !part.ends_with([' ', '.'])
            && !part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*:\\".contains(c))
            && !matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(upper.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && matches!(upper.as_bytes()[3], b'1'..=b'9'))
    })
}
pub(crate) fn inspect_tree(repo: &Repository, tree: &git2::Tree<'_>) -> Result<()> {
    let mut failure = None;
    let mut seen = std::collections::HashSet::new();
    let mut count = 0;
    let walked = tree.walk(git2::TreeWalkMode::PreOrder, |prefix, e| {
        count += 1;
        let Ok(name) = e.name() else {
            failure = Some("UNSUPPORTED_PATH");
            return git2::TreeWalkResult::Abort;
        };
        let path = format!("{prefix}{name}");
        if count > 100_000
            || path.split('/').count() > 64
            || !safe_name(&path)
            || name.eq_ignore_ascii_case(".git")
            || !seen.insert(path.to_lowercase())
        {
            failure = Some("UNSUPPORTED_CHECKOUT_PATH");
            return git2::TreeWalkResult::Abort;
        }
        if !matches!(e.filemode(), 0o040000 | 0o100644 | 0o100755)
            || name.eq_ignore_ascii_case(".gitattributes")
        {
            failure = Some("UNSUPPORTED_CHECKOUT_MODE_OR_ATTRIBUTES");
            return git2::TreeWalkResult::Abort;
        }
        if e.kind() == Some(git2::ObjectType::Blob)
            && repo
                .find_blob(e.id())
                .map(|b| b.size() as u64 > FILE_LIMIT)
                .unwrap_or(true)
        {
            failure = Some("LIMIT: checkout blob");
            return git2::TreeWalkResult::Abort;
        }
        git2::TreeWalkResult::Ok
    });
    if let Some(failure) = failure {
        return Err(failure.into());
    }
    walked.map_err(err)?;
    Ok(())
}
pub(super) fn entries(
    repo: &Repository,
    old: &git2::Tree<'_>,
    new: &git2::Tree<'_>,
) -> Result<Vec<Entry>> {
    inspect_tree(repo, old)?;
    inspect_tree(repo, new)?;
    let diff = repo
        .diff_tree_to_tree(Some(old), Some(new), None)
        .map_err(err)?;
    if diff.deltas().len() > MAX_FILES {
        return Err("LIMIT: checkout files".into());
    }
    let mut result = Vec::new();
    let mut total = 0usize;
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(Path::to_str)
            .ok_or("UNSUPPORTED_PATH")?
            .to_string();
        let before = old.get_path(Path::new(&path)).ok();
        let after = new.get_path(Path::new(&path)).ok();
        let mut hash = |entry: Option<&git2::TreeEntry<'_>>| -> Result<Option<String>> {
            entry
                .map(|e| {
                    let blob = repo.find_blob(e.id()).map_err(err)?;
                    total += blob.size();
                    if total > 50 * 1024 * 1024 {
                        return Err("LIMIT: checkout bytes".into());
                    }
                    Ok(digest(blob.content()))
                })
                .transpose()
        };
        result.push(Entry {
            path,
            before: hash(before.as_ref())?,
            after: hash(after.as_ref())?,
            before_mode: before.map(|e| e.filemode() as u32).unwrap_or(0),
            after_mode: after.map(|e| e.filemode() as u32).unwrap_or(0),
        });
    }
    Ok(result)
}
pub(super) fn locks<'a>(
    repo: &'a Repository,
    old: &str,
    new: &str,
) -> Result<git2::Transaction<'a>> {
    let mut names = vec!["HEAD", old, new];
    names.sort();
    names.dedup();
    let mut tx = repo.transaction().map_err(err)?;
    for name in names {
        if name != "HEAD"
            && (!(name.starts_with("refs/heads/") || name.starts_with("refs/remotes/"))
                || !git2::Reference::is_valid_name(name))
        {
            return Err("INVALID_JOURNAL_REF".into());
        }
        tx.lock_ref(name).map_err(err)?;
    }
    Ok(tx)
}
pub(super) fn replace(
    root: &Path,
    journal: &Path,
    e: &Entry,
    bytes: Option<&[u8]>,
    target_mode: u32,
) -> Result<()> {
    let path = rel(root, Path::new(&e.path))?;
    if let Some(bytes) = bytes {
        let parent = path.parent().ok_or("INVALID_PATH")?;
        fs::create_dir_all(parent).map_err(err)?;
        rel(root, Path::new(&e.path))?;
        let temp = journal.join(format!("write-{}.tmp", Uuid::new_v4()));
        save(&temp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(target_mode & 0o777))
                .map_err(err)?;
        }
        #[cfg(not(unix))]
        {
            let _ = target_mode;
        }
        fs::rename(&temp, &path).map_err(err)?;
        sync_dir(parent)?;
    } else if path.exists() {
        fs::remove_file(&path).map_err(err)?;
        sync_dir(path.parent().ok_or("INVALID_PATH")?)?;
    }
    Ok(())
}
fn cleanup(root: &Path, _record: &Journal) -> Result<()> {
    // Rename the complete checkpoint rather than deleting rollback inputs piecemeal.
    // Retention/garbage collection is a separate operation; archives remain local.
    fs::rename(
        root.join(JOURNAL),
        root.join(".git")
            .join(format!("branchdesk-checkout-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    sync_dir(&root.join(".git"))
}
pub(super) fn publish_index(root: &Path, journal: &Path, lock: &IndexLock) -> Result<()> {
    // Keep the interoperable index.lock until HEAD is updated as well.
    // Moving index.lock itself would let a second writer acquire it too early.
    let layout=layout::Layout::read(root)?;
    let private=layout.private.canonicalize().map_err(err)?;
    if lock.path.file_name()!=Some(std::ffi::OsStr::new("index.lock"))
        || lock.path.parent().ok_or("INVALID_INDEX_LOCK_PATH")?.canonicalize().map_err(err)?!=private
        || journal.parent().ok_or("INVALID_JOURNAL_PATH")?.canonicalize().map_err(err)?!=private {
        return Err("INVALID_INDEX_PUBLICATION_SCOPE".into());
    }
    let next = journal.join(format!("index-{}.next", Uuid::new_v4()));
    save(&next, &bounded(&lock.path, 20 * 1024 * 1024)?)?;
    fs::rename(&next, layout.private.join("index")).map_err(err)?;
    sync_dir(&layout.private)
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    branch: &str,
    lock: &mut IndexLock,
    fail_after: Option<usize>,
) -> Result<()> {
    execute_inner(
        session,
        repo,
        revision,
        branch,
        lock,
        fail_after,
        Mode::Checkout,
    )
}
pub(super) fn detached(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    oid: &str,
    lock: &mut IndexLock,
    fail_after: Option<usize>,
) -> Result<()> {
    if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("INVALID_COMMIT_ID".into());
    }
    execute_inner(
        session,
        repo,
        revision,
        "HEAD",
        lock,
        fail_after,
        Mode::Detached(oid),
    )
}
pub(super) fn fast_forward(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    branch: &str,
    lock: &mut IndexLock,
    fail_after: Option<usize>,
) -> Result<()> {
    execute_inner(
        session,
        repo,
        revision,
        branch,
        lock,
        fail_after,
        Mode::FastForward,
    )
}
pub(super) struct MergeInput<'a> {
    pub name: &'a str,
    pub message: &'a str,
    pub author: &'a str,
    pub email: &'a str,
}
enum Mode<'a> {
    Checkout,
    Detached(&'a str),
    FastForward,
    Merge(&'a MergeInput<'a>),
    Replay(&'a ReplayInput<'a>),
    SequenceRewind(&'a str),
    Rebase(&'a rebase::execution::Prepared),
}
pub(super) fn merge_commit(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    input: &MergeInput<'_>,
    lock: &mut IndexLock,
    fail_after: Option<usize>,
) -> Result<()> {
    execute_inner(
        session,
        repo,
        revision,
        input.name,
        lock,
        fail_after,
        Mode::Merge(input),
    )
}
pub(super) struct ReplayInput<'a> {
    pub complete_draft: bool,
    pub committer_time: Option<git2::Time>,
    pub expected_oid: Option<Oid>,
    pub allow_empty: bool,
    pub resolutions: Option<&'a HashMap<String, merge_draft::Resolution>>,
    pub oid: &'a str,
    pub kind: ReplayKind,
    pub mainline: Option<usize>,
    pub message: &'a str,
    pub author: &'a str,
    pub email: &'a str,
}
pub(super) fn rewind_sequence(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    original: &str,
    lock: &mut IndexLock,
) -> Result<()> {
    let branch = session.fresh(revision)?.branch;
    execute_inner(
        session,
        repo,
        revision,
        &branch,
        lock,
        None,
        Mode::SequenceRewind(original),
    )
}
pub(super) fn replay_commit(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    input: &ReplayInput<'_>,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    let branch = session.fresh(revision)?.branch;
    execute_inner(
        session,
        repo,
        revision,
        &branch,
        lock,
        fail,
        Mode::Replay(input),
    )
}
pub(super) fn rebase(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    prepared: &rebase::execution::Prepared,
    lock: &mut IndexLock,
) -> Result<()> {
    let branch = session.fresh(revision)?.branch;
    execute_inner(
        session,
        repo,
        revision,
        &branch,
        lock,
        None,
        Mode::Rebase(prepared),
    )
}
fn execute_inner(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    branch: &str,
    lock: &mut IndexLock,
    fail_after: Option<usize>,
    mode: Mode<'_>,
) -> Result<()> {
    let sequence_rewind = matches!(mode, Mode::SequenceRewind(_));
    let complete_replay_draft = matches!(&mode, Mode::Replay(input) if input.complete_draft);
    let fast_forward = !matches!(mode, Mode::Checkout | Mode::Detached(_));
    let full = if matches!(mode, Mode::Detached(_)) {
        "HEAD".into()
    } else if matches!(mode, Mode::FastForward | Mode::Merge(_)) {
        merge::reference_name(branch)?
    } else {
        format!("refs/heads/{branch}")
    };
    if branch.len()
        > (if full.starts_with("refs/remotes/") {
            1031
        } else {
            200
        })
        || branch.starts_with('-')
        || (full != "HEAD" && !git2::Reference::is_valid_name(&full))
    {
        return Err("INVALID_BRANCH".into());
    }
    let old_branch = repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        .unwrap_or("HEAD")
        .to_string();
    if old_branch == "HEAD" && fast_forward {
        return Err("UNSUPPORTED_DETACHED_OPERATION".into());
    }
    let mut tx = locks(repo, &old_branch, &full)?;
    let snap = session.fresh(revision)?;
    if !snap.changes.is_empty() {
        return Err("DIRTY_CHECKOUT: 먼저 변경을 커밋하거나 stash에 보관하세요.".into());
    }
    if full == old_branch
        && !matches!(
            mode,
            Mode::Replay(_) | Mode::SequenceRewind(_) | Mode::Detached(_) | Mode::Rebase(_)
        )
    {
        return Ok(());
    }
    let old = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let target = if let Mode::Detached(oid) = mode {
        repo.find_commit(Oid::from_str(oid).map_err(err)?)
            .map_err(err)?
    } else {
        let reference = repo.find_reference(&full).map_err(err)?;
        if matches!(mode, Mode::FastForward | Mode::Merge(_))
            && reference.symbolic_target().map_err(err)?.is_some()
        {
            return Err("SYMBOLIC_MERGE_TARGET".into());
        }
        reference.peel_to_commit().map_err(err)?
    };
    let ff_policy = if matches!(mode, Mode::FastForward | Mode::Merge(_)) {
        merge::target_ff_policy(&session.root, branch)?
    } else {
        merge::FastForwardPolicy::Allow
    };
    if matches!(mode, Mode::FastForward) {
        if ff_policy == merge::FastForwardPolicy::Never {
            return Err("NO_FF: configuration requires a merge commit".into());
        }
        if old.id() == target.id()
            || repo
                .graph_descendant_of(old.id(), target.id())
                .map_err(err)?
        {
            return Ok(());
        }
        if !repo
            .graph_descendant_of(target.id(), old.id())
            .map_err(err)?
        {
            return Err("NON_FAST_FORWARD: merge commit required".into());
        }
    }
    let old_tree = old.tree().map_err(err)?;
    let merge_source = if matches!(mode, Mode::Merge(_)) {
        Some(target.id().to_string())
    } else {
        None
    };
    let replay_record = if let Mode::Replay(input) = &mode {
        Some(replay::Record {
            kind: input.kind,
            source: input.oid.into(),
            mainline: input.mainline,
            resolutions: input.resolutions.cloned(),
        })
    } else {
        None
    };
    let rebase_record = if let Mode::Rebase(prepared) = &mode {
        Some(prepared.record.clone())
    } else {
        None
    };
    let target = if let Mode::Rebase(prepared) = &mode {
        if prepared.head != old.id() {
            return Err("STALE_STATE".into());
        }
        if prepared.oid == old.id() {
            return Ok(());
        }
        let odb = repo.odb().map_err(err)?;
        for (id, kind, bytes) in &prepared.objects {
            if odb.write(*kind, bytes).map_err(err)? != *id {
                return Err("INVALID_REBASE_OBJECT".into());
            }
        }
        rebase::execution::verify(repo, old.id(), prepared.oid, &prepared.record)?;
        repo.reference(
            &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
            old.id(),
            false,
            "rebase: preserve original head",
        )
        .map_err(err)?;
        repo.find_commit(prepared.oid).map_err(err)?
    } else if let Mode::SequenceRewind(original) = mode {
        let target = repo
            .find_commit(Oid::from_str(original).map_err(err)?)
            .map_err(err)?;
        sequence_state::verify_abort_journal(
            &session.root,
            repo,
            &old_branch,
            &old.id().to_string(),
            original,
        )?;
        if !repo
            .graph_descendant_of(old.id(), target.id())
            .map_err(err)?
        {
            return Err("INVALID_SEQUENCE_ANCESTRY".into());
        }
        target
    } else if let Mode::Replay(input) = mode {
        let prepared = replay::prepare_resolved(
            session,
            revision,
            input.oid,
            input.kind,
            input.mainline,
            (input.message, input.author, input.email),
            replay::PrepareOptions {
                resolutions: input.resolutions,
                allow_empty: input.allow_empty,
                committer_time: input.committer_time,
            },
        )?;
        if input.expected_oid.is_some_and(|id| id != prepared.oid) {
            return Err("STALE_SEQUENCE_INTENT".into());
        }
        if prepared.head != old.id() {
            return Err("STALE_STATE".into());
        }
        let odb = repo.odb().map_err(err)?;
        for (id, kind, bytes) in prepared.objects {
            if odb.write(kind, &bytes).map_err(err)? != id {
                return Err("INVALID_REPLAY_OBJECT".into());
            }
        }
        let record = replay_record.as_ref().ok_or("INVALID_REPLAY_RECORD")?;
        replay::verify_commit(repo, old.id(), prepared.oid, record)?;
        repo.reference(
            &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
            Oid::from_str(input.oid).map_err(err)?,
            false,
            "replay source checkpoint",
        )
        .map_err(err)?;
        repo.find_commit(prepared.oid).map_err(err)?
    } else if let Mode::Merge(input) = mode {
        if input.message.trim().is_empty()
            || input.message.len() > 10000
            || input.message.contains('\0')
            || input.author.trim().is_empty()
            || input.author.len() > 256
            || input.email.trim().is_empty()
            || input.email.len() > 256
        {
            return Err("INVALID_MERGE_IDENTITY_OR_MESSAGE".into());
        }
        let sig = git2::Signature::now(input.author, input.email).map_err(err)?;
        let bases = repo.merge_bases(old.id(), target.id()).map_err(err)?;
        if bases.len() != 1 {
            return Err("UNSUPPORTED_MULTIPLE_MERGE_BASES".into());
        }
        if ff_policy == merge::FastForwardPolicy::Only {
            return Err("FF_ONLY: merge commit disabled by configuration".into());
        }
        if bases[0] == target.id()
            || (bases[0] == old.id() && ff_policy != merge::FastForwardPolicy::Never)
        {
            return Err("MERGE_NOT_DIVERGED: use fast-forward".into());
        }
        let base = repo
            .find_commit(bases[0])
            .map_err(err)?
            .tree()
            .map_err(err)?;
        let theirs = target.tree().map_err(err)?;
        for t in [&old_tree, &base, &theirs] {
            inspect_tree(repo, t)?;
        }
        let mut merged = repo
            .merge_trees(&base, &old_tree, &theirs, None)
            .map_err(err)?;
        if merged.has_conflicts() {
            merge_draft::apply(&session.root, repo, old.id(), target.id(), &mut merged)?;
        }
        let tree = repo
            .find_tree(merged.write_tree_to(repo).map_err(err)?)
            .map_err(err)?;
        inspect_tree(repo, &tree)?;
        let oid = repo
            .commit(None, &sig, &sig, input.message, &tree, &[&old, &target])
            .map_err(err)?;
        repo.find_commit(oid).map_err(err)?
    } else {
        target
    };
    let tree = target.tree().map_err(err)?;
    let planned = entries(repo, &old_tree, &tree)?;
    for e in &planned {
        if !matches(&session.root, e, false)? {
            return Err("CHECKOUT_COLLISION: 작업 파일이 목표와 충돌합니다.".into());
        }
    }
    if old_branch == "HEAD" || full == "HEAD" {
        for oid in [old.id(), target.id()] {
            repo.reference(
                &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
                oid,
                false,
                "detached checkout recovery checkpoint",
            )
            .map_err(err)?;
        }
    }
    if sequence_rewind {
        repo.reference(
            &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
            old.id(),
            false,
            "sequence abort: preserve applied head",
        )
        .map_err(err)?;
    }
    if merge_source.is_some()
        || replay_record.is_some()
        || sequence_rewind
        || rebase_record.is_some()
    {
        // Retain generated objects and both parents across interrupted recovery.
        repo.reference(
            &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
            target.id(),
            false,
            "merge recovery checkpoint",
        )
        .map_err(err)?;
    }
    let mut index = git2::Index::open(&lock.path).map_err(err)?;
    index.read_tree(&tree).map_err(err)?;
    index.write().map_err(err)?;
    let before_index = bounded(&session.root.join(".git/index"), 20 * 1024 * 1024)?;
    let mut record = Journal {
        version: if rebase_record.is_some() {
            4
        } else if full.starts_with("refs/remotes/") {
            3
        } else if old_branch == "HEAD" || full == "HEAD" {
            2
        } else {
            1
        },
        preparing: true,
        fast_forward,
        sequence_rewind,
        merge_source,
        replay: replay_record,
        rebase: rebase_record,
        locks: capture_locks(&session.root, &old_branch, &full)?,
        old_branch,
        new_branch: full.clone(),
        old_oid: old.id().to_string(),
        new_oid: target.id().to_string(),
        before_index: digest(&before_index),
        after_index: digest(&bounded(&lock.path, 20 * 1024 * 1024)?),
        entries: planned,
    };
    let dir = session.root.join(JOURNAL);
    fs::create_dir(&dir).map_err(err)?;
    update_record(&dir, &record)?;
    sync_dir(&session.root.join(".git"))?;
    #[cfg(test)]
    crash_checkpoint(&session.root, 100);
    save(&dir.join("index.before"), &before_index)?;
    #[cfg(test)]
    crash_checkpoint(&session.root, 101);
    for (i, e) in record.entries.iter().enumerate() {
        if e.before.is_some() {
            save(
                &dir.join(format!("{i}.before")),
                &bounded(&rel(&session.root, Path::new(&e.path))?, FILE_LIMIT)?,
            )?;
        }
    }
    #[cfg(test)]
    crash_checkpoint(&session.root, 102);
    record.preparing = false;
    update_record(&dir, &record)?;
    sync_dir(&dir)?;
    sync_dir(&session.root.join(".git"))?;
    let injected = |step| {
        #[cfg(test)]
        crash_checkpoint(&session.root, step);
        if fail_after == Some(step) {
            Err("INJECTED_INTERRUPTION".to_string())
        } else {
            Ok(())
        }
    };
    injected(0)?;
    for (i, e) in record.entries.iter().enumerate() {
        if !matches(&session.root, e, false)? {
            return Err("RECOVERY_REQUIRED: concurrent worktree edit".into());
        }
        let blob = tree
            .get_path(Path::new(&e.path))
            .ok()
            .map(|e| repo.find_blob(e.id()))
            .transpose()
            .map_err(err)?;
        replace(
            &session.root,
            &dir,
            e,
            blob.as_ref().map(|b| b.content()),
            e.after_mode,
        )?;
        injected(i + 1)?;
    }
    publish_index(&session.root, &dir, lock)?;
    injected(record.entries.len() + 1)?;
    let sig = git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?;
    if fast_forward {
        tx.set_target(
            &record.old_branch,
            target.id(),
            Some(&sig),
            if record.sequence_rewind {
                "sequence: abort to original HEAD"
            } else if record.rebase.is_some() {
                "rebase: finish"
            } else if let Some(replay) = &record.replay {
                if replay.kind == ReplayKind::CherryPick {
                    "cherry-pick: commit"
                } else {
                    "revert: commit"
                }
            } else if record.merge_source.is_some() {
                "merge: commit"
            } else {
                "merge: fast-forward"
            },
        )
        .map_err(err)?;
    } else if full == "HEAD" {
        tx.set_target("HEAD", target.id(), Some(&sig), "checkout: detached commit")
            .map_err(err)?;
    } else {
        tx.set_symbolic_target("HEAD", &full, Some(&sig), "checkout: branch switch")
            .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    injected(record.entries.len() + 2)?;
    if let Some(replay) = &record.replay {
        if complete_replay_draft {
            #[cfg(test)]
            crash_checkpoint(&session.root, 103);
            replay_draft::mark_completed(&session.root, old.id(), target.id(), replay)?;
            #[cfg(test)]
            crash_checkpoint(&session.root, 104);
        }
    }
    cleanup(&session.root, &record)
}
#[cfg(test)]
fn crash_checkpoint(root: &Path, step: usize) {
    pause_checkpoint(root, "BRANCHDESK_CRASH_STEP", step);
}
#[cfg(test)]
fn pause_checkpoint(root: &Path, key: &str, step: usize) {
    if std::env::var_os("BRANCHDESK_CRASH_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var(key).ok().as_deref() == Some(step.to_string().as_str())
    {
        fs::write(root.join(JOURNAL).join("child.ready"), step.to_string()).unwrap();
        loop {
            std::thread::park();
        }
    }
}
fn recovery_checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    pause_checkpoint(root, "BRANCHDESK_RECOVERY_STEP", step);
    #[cfg(not(test))]
    let _ = (root, step);
}
pub(super) fn recover(root: &Path, repo: &Repository, lock: &mut IndexLock) -> Result<()> {
    let dir = root.join(JOURNAL);
    let mut record: Journal =
        serde_json::from_slice(&bounded(&dir.join("manifest.json"), 32 * 1024 * 1024)?)
            .map_err(err)?;
    validate_header(&record)?;
    if record.entries.len() > MAX_FILES {
        return Err("INVALID_JOURNAL".into());
    }
    let mut tx = locks(repo, &record.old_branch, &record.new_branch)?;
    record.locks = capture_locks(root, &record.old_branch, &record.new_branch)?;
    update_record(&dir, &record)?;
    let (current_oid, head) = if record.version == 2 {
        for (name, oid) in [
            (&record.old_branch, &record.old_oid),
            (&record.new_branch, &record.new_oid),
        ] {
            if name != "HEAD" && repo.refname_to_id(name).map_err(err)?.to_string() != *oid {
                return Err("RECOVERY_CONFLICT: branch changed".into());
            }
        }
        let reference = repo.find_reference("HEAD").map_err(err)?;
        let head = reference
            .symbolic_target()
            .map_err(err)?
            .unwrap_or("HEAD")
            .to_string();
        let oid = repo
            .head()
            .map_err(err)?
            .peel_to_commit()
            .map_err(err)?
            .id()
            .to_string();
        if !((head == record.old_branch && oid == record.old_oid)
            || (head == record.new_branch && oid == record.new_oid))
        {
            return Err("RECOVERY_CONFLICT: HEAD changed".into());
        }
        (oid, head)
    } else {
        let current_oid = repo
            .refname_to_id(&record.old_branch)
            .map_err(err)?
            .to_string();
        if (current_oid != record.old_oid
            && !(record.fast_forward && current_oid == record.new_oid))
            || (record.replay.is_none()
                && record.rebase.is_none()
                && !record.sequence_rewind
                && repo
                    .refname_to_id(&record.new_branch)
                    .map_err(err)?
                    .to_string()
                    != *record.merge_source.as_ref().unwrap_or(&record.new_oid))
            || (record.fast_forward
                && record.rebase.is_none()
                && !record.sequence_rewind
                && !repo
                    .graph_descendant_of(
                        Oid::from_str(&record.new_oid).map_err(err)?,
                        Oid::from_str(&record.old_oid).map_err(err)?,
                    )
                    .map_err(err)?)
        {
            return Err("RECOVERY_CONFLICT: branch changed".into());
        }
        let head = repo.find_reference("HEAD").map_err(err)?;
        let head = head
            .symbolic_target()
            .map_err(err)?
            .ok_or("RECOVERY_CONFLICT: detached HEAD")?
            .to_string();
        if head != record.old_branch && (record.fast_forward || head != record.new_branch) {
            return Err("RECOVERY_CONFLICT: HEAD changed".into());
        }
        (current_oid, head)
    };
    let old_tree = repo
        .find_commit(Oid::from_str(&record.old_oid).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let new_tree = repo
        .find_commit(Oid::from_str(&record.new_oid).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    if let Some(source) = &record.merge_source {
        let commit = repo
            .find_commit(Oid::from_str(&record.new_oid).map_err(err)?)
            .map_err(err)?;
        if !record.fast_forward
            || commit.parent_count() != 2
            || commit.parent_id(0).map_err(err)?.to_string() != record.old_oid
            || commit.parent_id(1).map_err(err)?.to_string() != *source
        {
            return Err("INVALID_MERGE_JOURNAL_PARENTS".into());
        }
    }
    if let Some(replay) = &record.replay {
        if !record.fast_forward
            || record.merge_source.is_some()
            || record.old_branch != record.new_branch
        {
            return Err("INVALID_REPLAY_JOURNAL".into());
        }
        replay::verify_commit(
            repo,
            Oid::from_str(&record.old_oid).map_err(err)?,
            Oid::from_str(&record.new_oid).map_err(err)?,
            replay,
        )?;
    }
    if record.sequence_rewind {
        if !record.fast_forward
            || record.replay.is_some()
            || record.merge_source.is_some()
            || record.old_branch != record.new_branch
        {
            return Err("INVALID_SEQUENCE_REWIND_JOURNAL".into());
        }
        sequence_state::verify_abort_journal(
            root,
            repo,
            &record.old_branch,
            &record.old_oid,
            &record.new_oid,
        )?;
        if !repo
            .graph_descendant_of(
                Oid::from_str(&record.old_oid).map_err(err)?,
                Oid::from_str(&record.new_oid).map_err(err)?,
            )
            .map_err(err)?
        {
            return Err("INVALID_SEQUENCE_ANCESTRY".into());
        }
    }
    if let Some(rebase) = &record.rebase {
        rebase::execution::verify(
            repo,
            Oid::from_str(&record.old_oid).map_err(err)?,
            Oid::from_str(&record.new_oid).map_err(err)?,
            rebase,
        )?;
    }
    let expected = entries(repo, &old_tree, &new_tree)?;
    if serde_json::to_vec(&expected).map_err(err)?
        != serde_json::to_vec(&record.entries).map_err(err)?
    {
        return Err("INVALID_JOURNAL_ENTRIES".into());
    }
    if record.preparing {
        // No worktree, live index, or HEAD writes precede the prepared marker.
        // Do not trust partial backup files or overwrite subsequent user edits.
        if current_oid != record.old_oid
            || head != record.old_branch
            || digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?) != record.before_index
        {
            return Err("RECOVERY_CONFLICT: preparation state changed".into());
        }
        for entry in &record.entries {
            if !matches(root, entry, false)? {
                return Err("RECOVERY_CONFLICT: preparation file changed".into());
            }
        }
        return cleanup(root, &record);
    }
    let index_before = bounded(&dir.join("index.before"), 20 * 1024 * 1024)?;
    let mut backup_index = git2::Index::open(&dir.join("index.before")).map_err(err)?;
    if backup_index.has_conflicts()
        || backup_index.write_tree_to(repo).map_err(err)? != old_tree.id()
    {
        return Err("INVALID_BACKUP_INDEX".into());
    }
    let index_now = digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?);
    if digest(&index_before) != record.before_index
        || (index_now != record.before_index && index_now != record.after_index)
    {
        return Err("RECOVERY_CONFLICT: index changed".into());
    }
    let mut backups = Vec::new();
    for (i, e) in record.entries.iter().enumerate() {
        if !matches(root, e, false)? && !matches(root, e, true)? {
            return Err("RECOVERY_CONFLICT: 작업 후 변경된 파일을 보존했습니다.".into());
        }
        let bytes = if e.before.is_some() {
            Some(bounded(&dir.join(format!("{i}.before")), FILE_LIMIT)?)
        } else {
            None
        };
        if bytes.as_ref().map(|b| digest(b)) != e.before {
            return Err("INVALID_BACKUP".into());
        }
        backups.push(bytes);
    }
    // Refresh ownership before restoration so a second crash can be recovered.
    fs::write(&lock.path, &index_before).map_err(err)?;
    record.locks = capture_locks(root, &record.old_branch, &record.new_branch)?;
    update_record(&dir, &record)?;
    recovery_checkpoint(root, 0);
    for (i, (e, bytes)) in record.entries.iter().zip(backups.iter()).enumerate() {
        if !matches(root, e, false)? {
            if !matches(root, e, true)? {
                return Err("RECOVERY_CONFLICT".into());
            }
            replace(root, &dir, e, bytes.as_deref(), e.before_mode)?;
        }
        recovery_checkpoint(root, i + 1);
    }
    publish_index(root, &dir, lock)?;
    recovery_checkpoint(root, record.entries.len() + 1);
    let sig = git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?;
    if record.fast_forward {
        tx.set_target(
            &record.old_branch,
            Oid::from_str(&record.old_oid).map_err(err)?,
            Some(&sig),
            "merge: recover interrupted operation",
        )
        .map_err(err)?;
    } else if record.old_branch == "HEAD" {
        tx.set_target(
            "HEAD",
            Oid::from_str(&record.old_oid).map_err(err)?,
            Some(&sig),
            "checkout: recover detached HEAD",
        )
        .map_err(err)?;
    } else {
        tx.set_symbolic_target(
            "HEAD",
            &record.old_branch,
            Some(&sig),
            "checkout: recover interrupted operation",
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    recovery_checkpoint(root, record.entries.len() + 2);
    cleanup(root, &record)
}
