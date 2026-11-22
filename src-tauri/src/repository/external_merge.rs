//! External Git merge inspection and journaled staging of user-resolved worktree files.
use super::*;
fn journal(root:&Path)->Result<PathBuf>{layout::private_file(root,"branchdesk-merge-index")}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMerge {
    pub heads: Vec<String>,
    pub conflicts: Vec<merge::MergeConflict>,
}
pub(super) fn inspect(root: &Path, repo: &Repository) -> Result<Option<ExternalMerge>> {
    if !layout::private_file(root,"MERGE_HEAD")?.exists() {
        return Ok(None);
    }
    let text = String::from_utf8(bounded(&layout::private_file(root,"MERGE_HEAD")?, 4096)?).map_err(err)?;
    let mut heads = Vec::new();
    for line in text.lines() {
        let oid = Oid::from_str(line).map_err(err)?;
        repo.find_commit(oid).map_err(err)?;
        if heads.len() >= 16 || heads.contains(&oid.to_string()) {
            return Err("UNSUPPORTED_MERGE_HEADS".into());
        }
        heads.push(oid.to_string());
    }
    if heads.is_empty() {
        return Err("INVALID_MERGE_HEAD".into());
    }
    let index = repo.index().map_err(err)?;
    let mut conflicts = Vec::new();
    for c in index.conflicts().map_err(err)? {
        if conflicts.len() >= MAX_FILES {
            return Err("LIMIT: merge conflicts".into());
        }
        let c = c.map_err(err)?;
        conflicts.push(merge::MergeConflict {
            ancestor: merge::version(c.ancestor)?,
            ours: merge::version(c.our)?,
            theirs: merge::version(c.their)?,
        });
    }
    Ok(Some(ExternalMerge { heads, conflicts }))
}
pub(super) fn pending(root: &Path) -> bool {
    journal(root).is_ok_and(|path|path.exists())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    #[serde(default,skip_serializing_if="Option::is_none")]
    owner: Option<PathBuf>,
    before: String,
    #[serde(default)]
    before_absent: bool,
    after: String,
    metadata: String,
}
fn metadata(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    for name in ["HEAD", "MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "ORIG_HEAD"] {
        hash.update(name.as_bytes());
        let path = layout::private_file(root,name)?;
        if path.exists() {
            let b = bounded(&path, 20 * 1024 * 1024)?;
            hash.update((b.len() as u64).to_le_bytes());
            hash.update(b);
        } else {
            hash.update(b"absent");
        }
    }
    let repo = open(root)?;
    match repo.head() {
        Ok(head) => hash.update(head.target().ok_or("INVALID_HEAD")?.as_bytes()),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => hash.update(b"unborn"),
        Err(e) => return Err(err(e)),
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn stage(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    id: &str,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    let branch = head_lock_target(repo)?;
    let _refs = checkout::locks(repo, &branch, &branch)?;
    session.fresh(revision)?;
    let state = inspect(&session.root, repo)?.ok_or("NO_EXTERNAL_MERGE")?;
    if state.heads.len() != 1 {
        return Err("UNSUPPORTED_OCTOPUS_MERGE".into());
    }
    let path = state
        .conflicts
        .iter()
        .flat_map(|c| [&c.ancestor, &c.ours, &c.theirs])
        .flatten()
        .find(|v| v.id == id)
        .map(|v| v.path.clone())
        .ok_or("INVALID_MERGE_CONFLICT")?;
    let full = rel(&session.root, Path::new(&path))?;
    let mut index = git2::Index::open(&lock.path).map_err(err)?;
    let original = index
        .get_path(Path::new(&path), 2)
        .or_else(|| index.get_path(Path::new(&path), 3))
        .or_else(|| index.get_path(Path::new(&path), 1))
        .ok_or("INVALID_MERGE_CONFLICT")?;
    index.conflict_remove(Path::new(&path)).map_err(err)?;
    if full.exists() {
        let bytes = bounded(&full, FILE_LIMIT)?;
        let mut entry = original;
        entry.id = repo.blob(&bytes).map_err(err)?;
        entry.file_size = bytes.len() as u32;
        entry.flags &= !0x3000;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            entry.mode = if fs::metadata(&full).map_err(err)?.permissions().mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
        }
        if !matches!(entry.mode, 0o100644 | 0o100755) {
            return Err("UNSUPPORTED_FILE_MODE".into());
        }
        index.add(&entry).map_err(err)?;
    }
    index.write().map_err(err)?;
    publish(session, revision, lock, &branch, fail)
}
// Shared index-only checkpoint for external resolutions and hunk selection.
pub(super) fn publish(
    session: &mut Session,
    revision: &str,
    lock: &IndexLock,
    branch: &str,
    fail: Option<usize>,
) -> Result<()> {
    session.fresh(revision)?;
    let before_absent = !layout::private_file(&session.root,"index")?.exists();
    let before = if before_absent {
        Vec::new()
    } else {
        bounded(&layout::private_file(&session.root,"index")?, 20 * 1024 * 1024)?
    };
    let after = bounded(&lock.path, 20 * 1024 * 1024)?;
    let record = Record {
        version: 1,
        owner: layout::journal_owner(&session.root)?,
        before: checkout::digest(&before),
        before_absent,
        after: checkout::digest(&after),
        metadata: metadata(&session.root)?,
    };
    let dir = journal(&session.root)?;
    fs::create_dir(&dir).map_err(err)?;
    checkout::save(&dir.join("index.before"), &before)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    journal_locks::record(&session.root, &dir, Some(branch), false, false)?;
    checkout::sync_dir(&layout::Layout::read(&session.root)?.private)?;
    checkpoint(&session.root, 0);
    if fail == Some(0) {
        return Err("INJECTED_INTERRUPTION".into());
    }
    session.fresh(revision)?;
    checkout::publish_index(&session.root, &dir, lock)?;
    checkpoint(&session.root, 1);
    if fail == Some(1) {
        return Err("INJECTED_INTERRUPTION".into());
    }
    Ok(())
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        journal(root)?,
        layout::private_file(root,&format!("branchdesk-merge-index-completed-{}",Uuid::new_v4()))?,
    )
    .map_err(err)?;
    checkout::sync_dir(&layout::Layout::read(root)?.private)
}
pub(super) fn recover(root: &Path, lock: &mut IndexLock) -> Result<()> {
    let repo = open(root)?;
    let branch = head_lock_target(&repo)?;
    let _refs = checkout::locks(&repo, &branch, &branch)?;
    let dir = journal(root)?;
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 16 * 1024)?).map_err(err)?;
    if record.version != 1 || record.owner != layout::journal_owner(root)? || record.metadata != metadata(root)? {
        return Err("RECOVERY_CONFLICT: external merge metadata changed".into());
    }
    let before = bounded(&dir.join("index.before"), 20 * 1024 * 1024)?;
    let absent = !layout::private_file(root,"index")?.exists();
    let current = if absent {
        checkout::digest(&[])
    } else {
        checkout::digest(&bounded(&layout::private_file(root,"index")?, 20 * 1024 * 1024)?)
    };
    if (absent && !record.before_absent)
        || (record.before_absent && !before.is_empty())
        || (record.before_absent && !absent && current != record.after)
        || checkout::digest(&before) != record.before
        || (current != record.before && current != record.after)
    {
        return Err("RECOVERY_CONFLICT: index changed".into());
    }
    fs::write(&lock.path, before).map_err(err)?;
    journal_locks::record(root, &dir, Some(&branch), false, false)?;
    checkpoint(root, 2);
    if record.before_absent {
        if !absent {
            fs::remove_file(layout::private_file(root,"index")?).map_err(err)?;
        }
        checkout::sync_dir(&layout::Layout::read(root)?.private)?;
    } else {
        checkout::publish_index(root, &dir, lock)?;
    }
    checkpoint(root, 3);
    Ok(())
}

fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_DRAFT_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_DRAFT_CHECKPOINT").ok().as_deref()
            == Some(format!("index:{step}").as_str())
    {
        fs::write(layout::private_file(root,"branchdesk-draft-test.ready").unwrap(), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

// Preserve symbolic unborn branch identity and detached HEAD locking.
pub(super) fn head_lock_target(repo: &Repository) -> Result<String> {
    let head = repo.find_reference("HEAD").map_err(err)?;
    Ok(head
        .symbolic_target()
        .map_err(err)?
        .unwrap_or("HEAD")
        .to_string())
}
