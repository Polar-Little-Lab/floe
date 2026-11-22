use super::*;

// Move one local branch only. Index and worktree bytes are never rewritten.
pub(super) fn execute(session: &mut Session, repo: &Repository, revision: &str, oid: &str) -> Result<()> {
    if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("INVALID_COMMIT_ID".into());
    }
    let target = external_merge::head_lock_target(repo)?;
    if !target.starts_with("refs/heads/") || repo.state() != git2::RepositoryState::Clean {
        return Err("SOFT_RESET_REQUIRES_LOCAL_BRANCH".into());
    }
    let mut tx = checkout::locks(repo, &target, &target)?;
    session.fresh(revision)?;
    if external_merge::head_lock_target(repo)? != target {
        return Err("STALE_HEAD".into());
    }
    let old = repo.refname_to_id(&target).map_err(err)?;
    let new = repo.find_commit(Oid::from_str(oid).map_err(err)?).map_err(err)?;
    if old == new.id() { return Ok(()); }
    // Keep the previous tip reachable independently of reflog expiration.
    // Checkpoint failure leaves the branch unchanged; a later failure may leave
    // an extra checkpoint, which is intentionally retained.
    repo.reference(&format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()), old, false,
                   "before soft reset: FLOE").map_err(err)?;
    let signature = repo.signature().or_else(|_| git2::Signature::now("FLOE", "local@floe.invalid")).map_err(err)?;
    tx.set_target(&target, new.id(), Some(&signature), "reset (soft): FLOE").map_err(err)?;
    tx.commit().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(root).args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, String, String) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        let sig = git2::Signature::new("Fixture", "fixture@example.invalid", &git2::Time::new(1700000000, 0)).unwrap();
        let mut tips = vec![];
        for content in ["base\n", "second\n"] {
            fs::write(d.path().join("file"), content).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("file")).unwrap(); index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let parents: Vec<_> = tips.last().map(|id| repo.find_commit(*id).unwrap()).into_iter().collect();
            let refs: Vec<_> = parents.iter().collect();
            tips.push(repo.commit(Some("HEAD"), &sig, &sig, content, &tree, &refs).unwrap());
        }
        fs::write(d.path().join("file"), "staged\n").unwrap();
        let mut index = repo.index().unwrap(); index.add_path(Path::new("file")).unwrap(); index.write().unwrap();
        fs::write(d.path().join("file"), "working\n").unwrap();
        fs::write(d.path().join("untracked"), "new work\n").unwrap();
        (d, tips[0].to_string(), tips[1].to_string())
    }
    fn preserved(root: &Path) -> Vec<Vec<u8>> {
        [".git/index", "file", "untracked"].iter().map(|p| fs::read(root.join(p)).unwrap()).collect()
    }
    #[test]
    fn soft_reset_matches_git_head_and_preserves_exact_index_and_worktree() {
        let (d, base, old) = fixture(); let (oracle, oracle_base, _) = fixture();
        let before = preserved(d.path());
        let mut session = Session::new(d.path().into()).unwrap(); let state = session.snapshot().unwrap();
        let after = session.mutate(&state.revision, Action::SoftReset { oid: base.clone() }).unwrap();
        git(oracle.path(), &["reset", "--soft", &oracle_base]);
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), git(oracle.path(), &["rev-parse", "HEAD"]));
        assert_eq!(git(d.path(), &["diff", "--cached"]), git(oracle.path(), &["diff", "--cached"]));
        assert_eq!(preserved(d.path()), before);
        let checkpoints = session.checkpoints(&after.revision).unwrap();
        assert_eq!(checkpoints.entries.len(), 1); assert_eq!(checkpoints.entries[0].oid, old);
        // Same-target reset must not manufacture additional recovery entries.
        let after = session.mutate(&after.revision, Action::SoftReset { oid: base }).unwrap();
        assert_eq!(session.checkpoints(&after.revision).unwrap().entries.len(), 1);
        assert_eq!(preserved(d.path()), before);
    }
    #[test]
    fn stale_review_and_foreign_branch_lock_refuse_without_moving_head() {
        for foreign_lock in [false, true] {
            let (d, base, old) = fixture(); let mut session = Session::new(d.path().into()).unwrap();
            let state = session.snapshot().unwrap();
            let branch = git(d.path(), &["symbolic-ref", "HEAD"]);
            let lock = d.path().join(".git").join(format!("{}.lock", branch.trim()));
            if foreign_lock { fs::write(&lock, b"foreign lock owner").unwrap(); }
            else { fs::write(d.path().join("file"), b"later work\n").unwrap(); }
            let before = preserved(d.path());
            let result = session.mutate(&state.revision, Action::SoftReset { oid: base });
            assert!(result.is_err());
            if foreign_lock { assert_eq!(fs::read(&lock).unwrap(), b"foreign lock owner"); fs::remove_file(&lock).unwrap(); }
            else { assert!(result.unwrap_err().starts_with("STALE_STATE")); }
            assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), old);
            assert_eq!(preserved(d.path()), before);
            let state = session.snapshot().unwrap(); assert!(session.checkpoints(&state.revision).unwrap().entries.is_empty());
            let after = session.mutate(&state.revision, Action::SoftReset { oid: git(d.path(), &["rev-parse", "HEAD^"]).trim().into() }).unwrap();
            assert_eq!(preserved(d.path()), before); assert_eq!(session.checkpoints(&after.revision).unwrap().entries.len(), 1);
        }
    }
    #[test]
    fn detached_and_noncommit_targets_refuse_without_checkpoint_or_file_writes() {
        for detached in [false, true] {
            let (d, base, old) = fixture();
            let repo = Repository::open(d.path()).unwrap();
            if detached { repo.set_head_detached(Oid::from_str(&old).unwrap()).unwrap(); }
            let mut session = Session::new(d.path().into()).unwrap(); let state = session.snapshot().unwrap();
            let before = preserved(d.path());
            let target = if detached {base} else {repo.blob(b"not a commit").unwrap().to_string()};
            assert!(session.mutate(&state.revision, Action::SoftReset { oid: target }).is_err());
            assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), old); assert_eq!(preserved(d.path()), before);
            let state = session.snapshot().unwrap(); assert!(session.checkpoints(&state.revision).unwrap().entries.is_empty());
        }
    }
    #[test]
    fn checkpoint_publication_failure_keeps_branch_and_retry_releases_locks() {
        let (d, base, old) = fixture();
        let blocker = d.path().join(".git/refs/branchdesk/checkpoints");
        fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        fs::write(&blocker, b"existing foreign ref path").unwrap();
        let mut session = Session::new(d.path().into()).unwrap(); let state = session.snapshot().unwrap();
        let before = preserved(d.path());
        assert!(session.mutate(&state.revision, Action::SoftReset { oid: base.clone() }).is_err());
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), old);
        assert_eq!(preserved(d.path()), before);
        assert_eq!(fs::read(&blocker).unwrap(), b"existing foreign ref path");
        fs::remove_file(&blocker).unwrap();
        let state = session.snapshot().unwrap();
        let after = session.mutate(&state.revision, Action::SoftReset { oid: base.clone() }).unwrap();
        assert_eq!(after.head_oid.as_deref(), Some(base.as_str()));
        assert_eq!(preserved(d.path()), before);
        assert_eq!(session.checkpoints(&after.revision).unwrap().entries[0].oid, old);
    }
    #[test]
    fn reset_checkpoint_survives_reflog_expiration_and_gc_then_restores_old_tip() {
        let (d, base, old) = fixture(); let mut session = Session::new(d.path().into()).unwrap();
        let state = session.snapshot().unwrap(); let before = preserved(d.path());
        session.mutate(&state.revision, Action::SoftReset { oid: base }).unwrap();
        git(d.path(), &["reflog", "expire", "--expire=now", "--all"]);
        git(d.path(), &["gc", "--prune=now"]);
        let state = session.snapshot().unwrap(); let saved = session.checkpoints(&state.revision).unwrap();
        assert!(saved.entries.iter().any(|entry| entry.oid == old));
        session.mutate(&state.revision, Action::SoftReset { oid: old.clone() }).unwrap();
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), old);
        assert_eq!(preserved(d.path()), before);
        git(d.path(), &["fsck", "--strict"]);
    }
}
