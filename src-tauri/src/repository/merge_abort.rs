//! Guarded roll-forward abort of external merges; unrelated worktree edits are preserved.
use super::*;
const JOURNAL: &str = ".git/branchdesk-merge-abort";
const META: [&str; 3] = ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE"];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    preparing: bool,
    branch: String,
    head: String,
    before_index: String,
    after_index: String,
    entries: Vec<checkout::Entry>,
    metadata: Vec<Option<Vec<u8>>>,
    auto_merge: Option<String>,
    orig_head: Option<Vec<u8>>,
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
fn save_record(dir: &Path, r: &Record) -> Result<()> {
    let next = dir.join(format!("record-{}.next", Uuid::new_v4()));
    checkout::save(&next, &serde_json::to_vec(r).map_err(err)?)?;
    fs::rename(next, dir.join("record.json")).map_err(err)?;
    checkout::sync_dir(dir)
}
fn paths(repo: &Repository, tree: &git2::Tree<'_>, index: &git2::Index) -> Result<Vec<String>> {
    let diff = repo
        .diff_tree_to_index(Some(tree), Some(index), None)
        .map_err(err)?;
    if diff.deltas().len() > MAX_FILES {
        return Err("LIMIT: abort files".into());
    }
    let mut paths = std::collections::BTreeSet::new();
    for delta in diff.deltas() {
        for file in [delta.old_file(), delta.new_file()] {
            if let Some(p) = file.path() {
                paths.insert(p.to_str().ok_or("UNSUPPORTED_PATH")?.to_string());
            }
        }
    }
    Ok(paths.into_iter().collect())
}
fn mode(path: &Path, fallback: u32) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fallback;
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
        Ok(fallback)
    }
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    if session.root.join(".git/MERGE_AUTOSTASH").exists() {
        return Err("UNSUPPORTED_MERGE_AUTOSTASH".into());
    }
    let head = repo.head().map_err(err)?;
    let branch = head.name().map_err(err)?.to_string();
    let tx = merge_finish::locks(repo, &branch)?;
    session.fresh(revision)?;
    let state = external_merge::inspect(&session.root, repo)?.ok_or("NO_EXTERNAL_MERGE")?;
    if state.heads.len() != 1 {
        return Err("UNSUPPORTED_OCTOPUS_MERGE".into());
    }
    let commit = head.peel_to_commit().map_err(err)?;
    let tree = commit.tree().map_err(err)?;
    checkout::inspect_tree(repo, &tree)?;
    let orig = session.root.join(".git/ORIG_HEAD");
    if orig.exists()
        && String::from_utf8(bounded(&orig, 128)?).map_err(err)?.trim() != commit.id().to_string()
    {
        return Err("ABORT_HEAD_MOVED".into());
    }
    let mut index = git2::Index::open(&lock.path).map_err(err)?;
    let mut entries = Vec::new();
    let mut backups = Vec::new();
    let mut total = 0;
    for path in paths(repo, &tree, &index)? {
        let full = rel(&session.root, Path::new(&path))?;
        let before = if full.exists() {
            Some(bounded(&full, FILE_LIMIT)?)
        } else {
            None
        };
        let staged = index.get_path(Path::new(&path), 0);
        let conflict = (1..=3).any(|n| index.get_path(Path::new(&path), n).is_some());
        if !conflict {
            if let Some(e) = &staged {
                if repo.odb().map_err(err)?.read_header(e.id).map_err(err)?.0 > FILE_LIMIT as usize
                {
                    return Err("LIMIT: abort index blob".into());
                }
            }
            let staged_bytes = staged
                .as_ref()
                .map(|e| repo.find_blob(e.id).map(|b| b.content().to_vec()))
                .transpose()
                .map_err(err)?;
            if staged_bytes != before
                || (before.is_some()
                    && mode(&full, staged.as_ref().map(|e| e.mode).unwrap_or(0o100644))?
                        != staged.as_ref().map(|e| e.mode).unwrap_or(0))
            {
                return Err("ABORT_WOULD_OVERWRITE_UNSTAGED_EDIT".into());
            }
        }
        let after = tree.get_path(Path::new(&path)).ok();
        let after_bytes = after
            .as_ref()
            .map(|e| repo.find_blob(e.id()).map(|b| b.content().to_vec()))
            .transpose()
            .map_err(err)?;
        total += before.as_ref().map_or(0, Vec::len) + after_bytes.as_ref().map_or(0, Vec::len);
        if total > 50 * 1024 * 1024 {
            return Err("LIMIT: abort bytes".into());
        }
        entries.push(checkout::Entry {
            path,
            before: before.as_ref().map(|b| checkout::digest(b)),
            after: after_bytes.as_ref().map(|b| checkout::digest(b)),
            before_mode: if before.is_some() {
                mode(&full, staged.as_ref().map(|e| e.mode).unwrap_or(0o100644))?
            } else {
                0
            },
            after_mode: after.as_ref().map_or(0, |e| e.filemode() as u32),
        });
        backups.push(before);
    }
    let before_index = bounded(&session.root.join(".git/index"), 20 * 1024 * 1024)?;
    index.read_tree(&tree).map_err(err)?;
    index.write().map_err(err)?;
    let mut metadata = Vec::new();
    for name in META {
        let p = session.root.join(".git").join(name);
        metadata.push(if p.exists() {
            Some(bounded(&p, 1024 * 1024)?)
        } else {
            None
        });
    }
    let mut record = Record {
        version: 1,
        preparing: true,
        branch: branch.clone(),
        head: commit.id().to_string(),
        before_index: checkout::digest(&before_index),
        after_index: checkout::digest(&bounded(&lock.path, 20 * 1024 * 1024)?),
        entries,
        metadata,
        auto_merge: merge_finish::auto_merge(repo)?,
        orig_head: if orig.exists() {
            Some(bounded(&orig, 128)?)
        } else {
            None
        },
    };
    let dir = session.root.join(JOURNAL);
    fs::create_dir(&dir).map_err(err)?;
    save_record(&dir, &record)?;
    journal_locks::record_merge(&session.root, &dir, &branch)?;
    checkout::sync_dir(&session.root.join(".git"))?;
    checkout::save(&dir.join("index.before"), &before_index)?;
    for (i, b) in backups.iter().enumerate() {
        if let Some(b) = b {
            checkout::save(&dir.join(format!("{i}.before")), b)?;
        }
    }
    checkout::save(
        &dir.join("index.after"),
        &bounded(&lock.path, 20 * 1024 * 1024)?,
    )?;
    record.preparing = false;
    save_record(&dir, &record)?;
    complete(&session.root, repo, tx, &record, lock, fail)
}
fn complete(
    root: &Path,
    repo: &Repository,
    mut tx: git2::Transaction<'_>,
    r: &Record,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    if r.version != 1 || r.entries.len() > MAX_FILES || r.metadata.len() != 3 {
        return Err("INVALID_ABORT_JOURNAL".into());
    }
    if repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        != Some(r.branch.as_str())
        || repo.refname_to_id(&r.branch).map_err(err)?.to_string() != r.head
    {
        return Err("RECOVERY_CONFLICT: HEAD changed".into());
    }
    let orig = root.join(".git/ORIG_HEAD");
    let actual_orig = if orig.exists() {
        Some(bounded(&orig, 128)?)
    } else {
        None
    };
    if actual_orig != r.orig_head || root.join(".git/MERGE_AUTOSTASH").exists() {
        return Err("RECOVERY_CONFLICT: ORIG_HEAD/autostash changed".into());
    }
    let tree = repo
        .find_commit(Oid::from_str(&r.head).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    checkout::inspect_tree(repo, &tree)?;
    let dir = root.join(JOURNAL);
    let current = checkout::digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?);
    if current != r.before_index && current != r.after_index {
        return Err("RECOVERY_CONFLICT: index changed".into());
    }
    let auto = merge_finish::auto_merge(repo)?;
    if (r.preparing || auto.is_some()) && auto != r.auto_merge {
        return Err("RECOVERY_CONFLICT: AUTO_MERGE changed".into());
    }
    for (name, expected) in META.into_iter().zip(&r.metadata) {
        let p = root.join(".git").join(name);
        if r.preparing && expected.is_some() && !p.exists() {
            return Err("RECOVERY_CONFLICT: preparation metadata removed".into());
        }
        if p.exists() && Some(bounded(&p, 1024 * 1024)?) != *expected {
            return Err("RECOVERY_CONFLICT: merge metadata changed".into());
        }
    }
    for e in &r.entries {
        if !checkout::matches(root, e, false)? && !checkout::matches(root, e, true)? {
            return Err("RECOVERY_CONFLICT: file edited after abort began".into());
        }
    }
    if r.preparing {
        if current != r.before_index {
            return Err("RECOVERY_CONFLICT: preparation index changed".into());
        }
        for e in &r.entries {
            if !checkout::matches(root, e, false)? {
                return Err("RECOVERY_CONFLICT: preparation file changed".into());
            }
        }
        return Ok(());
    }
    let bytes = bounded(&dir.join("index.before"), 20 * 1024 * 1024)?;
    if checkout::digest(&bytes) != r.before_index {
        return Err("INVALID_ABORT_BACKUP".into());
    }
    let before_index = git2::Index::open(&dir.join("index.before")).map_err(err)?;
    if paths(repo, &tree, &before_index)?
        != r.entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>()
    {
        return Err("INVALID_ABORT_PATHS".into());
    }
    for (i, e) in r.entries.iter().enumerate() {
        if e.before.is_some()
            && checkout::digest(&bounded(&dir.join(format!("{i}.before")), FILE_LIMIT)?)
                != *e.before.as_ref().unwrap()
        {
            return Err("INVALID_ABORT_BACKUP".into());
        }
        let target = tree.get_path(Path::new(&e.path)).ok();
        let hash = target
            .as_ref()
            .map(|e| {
                repo.find_blob(e.id())
                    .map(|b| checkout::digest(b.content()))
            })
            .transpose()
            .map_err(err)?;
        if hash != e.after || target.as_ref().map_or(0, |e| e.filemode() as u32) != e.after_mode {
            return Err("INVALID_ABORT_TARGET".into());
        }
    }
    let after_bytes = bounded(&dir.join("index.after"), 20 * 1024 * 1024)?;
    let mut after_index = git2::Index::open(&dir.join("index.after")).map_err(err)?;
    if checkout::digest(&after_bytes) != r.after_index
        || after_index.has_conflicts()
        || after_index.write_tree_to(repo).map_err(err)? != tree.id()
    {
        return Err("INVALID_ABORT_INDEX".into());
    }
    fs::write(&lock.path, after_bytes).map_err(err)?;
    journal_locks::record_merge(root, &dir, &r.branch)?;
    let injected = |n| {
        checkpoint(root, n);
        if fail == Some(n) {
            Err("INJECTED_INTERRUPTION".to_string())
        } else {
            Ok(())
        }
    };
    injected(0)?;
    for (i, e) in r.entries.iter().enumerate() {
        if !checkout::matches(root, e, true)? {
            if !checkout::matches(root, e, false)? {
                return Err("RECOVERY_CONFLICT".into());
            }
            let b = tree
                .get_path(Path::new(&e.path))
                .ok()
                .map(|e| repo.find_blob(e.id()))
                .transpose()
                .map_err(err)?;
            checkout::replace(root, &dir, e, b.as_ref().map(|b| b.content()), e.after_mode)?;
        }
        injected(i + 1)?;
    }
    checkout::publish_index(root, &dir, lock)?;
    injected(r.entries.len() + 1)?;
    if auto.is_some() {
        tx.remove("AUTO_MERGE").map_err(err)?;
    }
    tx.commit().map_err(err)?;
    injected(r.entries.len() + 2)?;
    for (i, name) in META.into_iter().enumerate() {
        let p = root.join(".git").join(name);
        if p.exists() {
            if Some(bounded(&p, 1024 * 1024)?) != r.metadata[i] {
                return Err("RECOVERY_CONFLICT: metadata changed".into());
            }
            fs::remove_file(p).map_err(err)?;
            checkout::sync_dir(&root.join(".git"))?;
        }
        injected(r.entries.len() + 3 + i)?;
    }
    Ok(())
}
pub(super) fn recover(root: &Path, repo: &Repository, lock: &mut IndexLock) -> Result<()> {
    let dir = root.join(JOURNAL);
    let r: Record = serde_json::from_slice(&bounded(&dir.join("record.json"), 16 * 1024 * 1024)?)
        .map_err(err)?;
    let tx = merge_finish::locks(repo, &r.branch)?;
    complete(root, repo, tx, &r, lock, None)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!(
            ".git/branchdesk-merge-abort-completed-{}",
            Uuid::new_v4()
        )),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}

fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_DRAFT_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_DRAFT_CHECKPOINT").ok().as_deref()
            == Some(format!("abort:{step}").as_str())
    {
        fs::write(root.join(".git/branchdesk-draft-test.ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}
