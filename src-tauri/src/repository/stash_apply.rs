//! Apply a stash through planned tree merges and a guarded rollback checkpoint.
use super::checkout::{digest, entries, inspect_tree, matches, replace, save, sync_dir};
use super::*;
const DIR: &str = ".git/branchdesk-stash-apply";
const COMPLETED: &str = ".git/branchdesk-stash-applied";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    branch: String,
    head: String,
    before_tree: String,
    after_tree: String,
    before_index: String,
    after_index: String,
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(DIR).exists() || root.join(COMPLETED).exists()
}
// Atomic directory rename transfers the durable lock ownership record along
// with the completed apply receipt. Keep it until the outer index lock is gone.
pub(super) fn record_completed_lock(root: &Path) -> Result<()> {
    if root.join(COMPLETED).exists() {
        journal_locks::record_paths(root, &root.join(COMPLETED), vec!["index.lock".into()])?;
    }
    Ok(())
}
pub(super) fn finish(root: &Path) -> Result<()> {
    if root.join(COMPLETED).exists() {
        fs::rename(root.join(COMPLETED), root.join(".git").join(format!("branchdesk-apply-completed-{}", Uuid::new_v4()))).map_err(err)?;
        sync_dir(&root.join(".git"))?;
    }
    Ok(())
}
fn archive(root: &Path) -> Result<()> {
    fs::rename(
        root.join(DIR),
        root.join(".git")
            .join(format!("branchdesk-apply-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    sync_dir(&root.join(".git"))
}
fn merge(
    repo: &Repository,
    base: &git2::Tree<'_>,
    current: &git2::Tree<'_>,
    saved: &git2::Tree<'_>,
) -> Result<git2::Index> {
    inspect_tree(repo, saved)?;
    let result = repo.merge_trees(base, current, saved, None).map_err(err)?;
    if result.has_conflicts() {
        return Err(
            "STASH_CONFLICT: stash와 현재 브랜치가 충돌합니다. 파일을 변경하지 않았습니다.".into(),
        );
    }
    Ok(result)
}
pub(super) fn apply(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    oid: &str,
    restore_index: bool,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<Applied> {
    let branch = repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        .ok_or("UNSUPPORTED_DETACHED_STASH")?
        .to_string();
    let mut tx = checkout::locks(repo, &branch, &branch)?;
    tx.lock_ref("refs/stash").map_err(err)?;
    let snapshot = session.fresh(revision)?;
    if !snapshot.changes.is_empty() {
        return Err("DIRTY_STASH_APPLY: 먼저 현재 변경을 커밋하거나 보관하세요.".into());
    }
    if !snapshot.stashes.iter().any(|s| s.oid == oid) {
        return Err("INVALID_STASH".into());
    }
    let stash = repo
        .find_commit(Oid::from_str(oid).map_err(err)?)
        .map_err(err)?;
    if !(2..=3).contains(&stash.parent_count()) {
        return Err("UNSUPPORTED_STASH_LAYOUT".into());
    }
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let current = head.tree().map_err(err)?;
    let base_commit = stash.parent(0).map_err(err)?;
    let base = base_commit.tree().map_err(err)?;
    inspect_tree(repo, &current)?;
    inspect_tree(repo, &base)?;
    let index_commit = stash.parent(1).map_err(err)?;
    if index_commit.parent_count() != 1
        || index_commit.parent_id(0).map_err(err)? != base_commit.id()
    {
        return Err("UNSUPPORTED_STASH_LAYOUT".into());
    }
    let mut work = merge(repo, &base, &current, &stash.tree().map_err(err)?)?;
    let target_index = if restore_index {
        merge(repo, &base, &current, &index_commit.tree().map_err(err)?)?
            .write_tree_to(repo)
            .map_err(err)?
    } else {
        current.id()
    };
    if stash.parent_count() == 3 {
        let untracked = stash.parent(2).map_err(err)?.tree().map_err(err)?;
        inspect_tree(repo, &untracked)?;
        let mut extras = git2::Index::new().map_err(err)?;
        extras.read_tree(&untracked).map_err(err)?;
        for entry in extras.iter() {
            let path = Path::new(std::str::from_utf8(&entry.path).map_err(err)?);
            if work.get_path(path, 0).is_some() || current.get_path(path).is_ok() {
                return Err("STASH_UNTRACKED_COLLISION".into());
            }
            work.add(&entry).map_err(err)?;
        }
    }
    let target = repo
        .find_tree(work.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    let planned = entries(repo, &current, &target)?;
    for entry in &planned {
        if !matches(&session.root, entry, false)? {
            return Err("STASH_WORKTREE_COLLISION".into());
        }
    }
    let before_index = bounded(&session.root.join(".git/index"), 20 * 1024 * 1024)?;
    let mut index = git2::Index::open(&lock.path).map_err(err)?;
    index
        .read_tree(&repo.find_tree(target_index).map_err(err)?)
        .map_err(err)?;
    index.write().map_err(err)?;
    let after_index = bounded(&lock.path, 20 * 1024 * 1024)?;
    let record = Record {
        version: 1,
        branch,
        head: head.id().to_string(),
        before_tree: current.id().to_string(),
        after_tree: target.id().to_string(),
        before_index: digest(&before_index),
        after_index: digest(&after_index),
    };
    let dir = session.root.join(DIR);
    fs::create_dir(&dir).map_err(err)?;
    save(&dir.join("index.before"), &before_index)?;
    save(&dir.join("index.after"), &after_index)?;
    save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    sync_dir(&dir)?;
    sync_dir(&session.root.join(".git"))?;
    journal_locks::record(&session.root, &dir, Some(&record.branch), true, false)?;
    let inject = |step| -> Result<()> {
        journal_locks::checkpoint(&session.root, "apply", step);
        if fail == Some(step) {
            Err("INJECTED_INTERRUPTION".into())
        } else {
            Ok(())
        }
    };
    inject(0)?;
    for (i, entry) in planned.iter().enumerate() {
        if !matches(&session.root, entry, false)? {
            return Err("RECOVERY_CONFLICT: worktree changed".into());
        }
        let blob = target
            .get_path(Path::new(&entry.path))
            .ok()
            .map(|e| repo.find_blob(e.id()))
            .transpose()
            .map_err(err)?;
        replace(
            &session.root,
            &dir,
            entry,
            blob.as_ref().map(|b| b.content()),
            entry.after_mode,
        )?;
        inject(i + 1)?;
    }
    checkout::publish_index(&session.root, &dir, lock)?;
    inject(planned.len() + 1)?;
    let applied = Applied {
        branch: record.branch,
        head: record.head,
        before_tree: record.before_tree,
        after_tree: record.after_tree,
        after_index: record.after_index,
    };
    applied.verify(&session.root, repo)?;
    drop(tx);
    journal_locks::record_paths(&session.root, &dir, vec!["index.lock".into()])?;
    fs::rename(&dir, session.root.join(COMPLETED)).map_err(err)?;
    sync_dir(&session.root.join(".git"))?;
    Ok(applied)
}
/// A receipt for the exact file/index/HEAD result; verified again before pop drops.
#[derive(Debug)]
pub(super) struct Applied {
    branch: String,
    head: String,
    before_tree: String,
    after_tree: String,
    after_index: String,
}
impl Applied {
    pub(super) fn verify(&self, root: &Path, repo: &Repository) -> Result<()> {
        if repo
            .find_reference("HEAD")
            .map_err(err)?
            .symbolic_target()
            .map_err(err)?
            != Some(self.branch.as_str())
            || repo.head().map_err(err)?.target().map(|id| id.to_string())
                != Some(self.head.clone())
            || digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?) != self.after_index
        {
            return Err("STASH_APPLY_RESULT_CHANGED".into());
        }
        let before = repo
            .find_tree(Oid::from_str(&self.before_tree).map_err(err)?)
            .map_err(err)?;
        let after = repo
            .find_tree(Oid::from_str(&self.after_tree).map_err(err)?)
            .map_err(err)?;
        for entry in entries(repo, &before, &after)? {
            if !matches(root, &entry, true)? {
                return Err("STASH_APPLY_RESULT_CHANGED".into());
            }
        }
        Ok(())
    }
}
pub(super) fn pop(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    oid: &str,
    restore_index: bool,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    // Reject duplicate identities before applying: drop cannot identify one safely yet.
    let snap = session.fresh(revision)?;
    if snap.stashes.iter().filter(|s| s.oid == oid).count() != 1 {
        return Err("AMBIGUOUS_OR_MISSING_STASH".into());
    }
    let applied = apply(session, repo, revision, oid, restore_index, lock, None)?;
    journal_locks::checkpoint(&session.root, "pop", 0);
    let mut finish = || -> Result<()> {
        if fail == Some(0) {
            return Err("INJECTED_INTERRUPTION".into());
        }
        // Capture revision, then validate the receipt. Subsequent changes fail
        // drop's locked fresh check instead of silently authorizing a new state.
        let next = session.snapshot()?.revision;
        applied.verify(&session.root, repo)?;
        journal_locks::checkpoint(&session.root, "pop", 1);
        stash_write::drop_stash(
            session,
            repo,
            &next,
            oid,
            fail.and_then(|n| n.checked_sub(1)),
        )
    };
    finish().map_err(|e| format!("POP_APPLIED_DROP_INCOMPLETE: 적용은 완료됐습니다. stash 삭제 상태를 새로 확인하세요. 적용을 반복하지 마세요. {e}"))
}
pub(super) fn recover(root: &Path, repo: &Repository, lock: &mut IndexLock) -> Result<()> {
    if root.join(COMPLETED).exists() {
        // Apply already completed. Recovery only releases recorded locks and
        // retires the receipt after the caller drops its recovery index lock.
        // Never reapply/rollback files or implicitly drop the preserved stash.
        journal_locks::record_paths(root, &root.join(COMPLETED), vec!["index.lock".into()])?;
        journal_locks::checkpoint(root, "pop-recover", 0);
        return Ok(());
    }
    let dir = root.join(DIR);
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 4096)?).map_err(err)?;
    if record.version != 1 {
        return Err("INVALID_APPLY_JOURNAL".into());
    }
    let _tx = checkout::locks(repo, &record.branch, &record.branch)?;
    journal_locks::record(root, &dir, Some(&record.branch), false, false)?;
    if repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        != Some(record.branch.as_str())
        || repo.head().map_err(err)?.target().map(|id| id.to_string()) != Some(record.head.clone())
    {
        return Err("RECOVERY_CONFLICT: HEAD changed".into());
    }
    let before = repo
        .find_tree(Oid::from_str(&record.before_tree).map_err(err)?)
        .map_err(err)?;
    let after = repo
        .find_tree(Oid::from_str(&record.after_tree).map_err(err)?)
        .map_err(err)?;
    if repo
        .head()
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?
        .tree_id()
        != before.id()
    {
        return Err("INVALID_APPLY_JOURNAL".into());
    }
    let planned = entries(repo, &before, &after)?;
    let index_before = bounded(&dir.join("index.before"), 20 * 1024 * 1024)?;
    let index_after = bounded(&dir.join("index.after"), 20 * 1024 * 1024)?;
    let mut backup = git2::Index::open(&dir.join("index.before")).map_err(err)?;
    if backup.has_conflicts()
        || backup.write_tree_to(repo).map_err(err)? != before.id()
        || digest(&index_before) != record.before_index
        || digest(&index_after) != record.after_index
    {
        return Err("INVALID_APPLY_BACKUP".into());
    }
    let live = digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?);
    if live != record.before_index && live != record.after_index {
        return Err("RECOVERY_CONFLICT: index changed".into());
    }
    for e in &planned {
        if !matches(root, e, false)? && !matches(root, e, true)? {
            return Err("RECOVERY_CONFLICT: worktree changed".into());
        }
    }
    journal_locks::checkpoint(root, "apply-recover", 0);
    for e in &planned {
        if !matches(root, e, false)? {
            if !matches(root, e, true)? {
                return Err("RECOVERY_CONFLICT: worktree changed".into());
            }
            let blob = before
                .get_path(Path::new(&e.path))
                .ok()
                .map(|e| repo.find_blob(e.id()))
                .transpose()
                .map_err(err)?;
            replace(
                root,
                &dir,
                e,
                blob.as_ref().map(|b| b.content()),
                e.before_mode,
            )?;
        }
    }
    fs::write(&lock.path, &index_before).map_err(err)?;
    journal_locks::record(root, &dir, Some(&record.branch), false, false)?;
    journal_locks::checkpoint(root, "apply-recover", 1);
    checkout::publish_index(root, &dir, lock)?;
    journal_locks::checkpoint(root, "apply-recover", 2);
    archive(root)
}
