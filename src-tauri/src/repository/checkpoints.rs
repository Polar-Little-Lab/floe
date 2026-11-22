use super::*;
#[derive(Serialize)]
pub struct Entry { pub reference: String, pub oid: String, pub summary: String, pub author: String, pub time: i64 }
#[derive(Serialize)]
pub struct Listing { pub entries: Vec<Entry>, pub truncated: bool }
pub(super) fn validate(reference: &str, oid: &str) -> Result<()> {
    if reference.len() > 1024 || !reference.starts_with("refs/branchdesk/checkpoints/") || !git2::Reference::is_valid_name(reference)
        || oid.len() != 40 || !oid.bytes().all(|b|b.is_ascii_hexdigit()) { return Err("INVALID_CHECKPOINT".into()); }
    Ok(())
}

impl Session {
    pub fn checkpoints(&mut self, revision: &str) -> Result<Listing> {
        self.fresh(revision)?;
        let repo = open(&self.root)?;
        let mut result = Listing { entries: Vec::new(), truncated: false };
        for (index, reference) in repo.references_glob("refs/branchdesk/checkpoints/*").map_err(err)?.enumerate() {
            if index >= 5000 { result.truncated = true; break; }
            let reference = reference.map_err(err)?;
            let name = reference.name().map_err(err)?;
            let oid = reference.target().ok_or("INVALID_CHECKPOINT")?;
            let commit = repo.find_commit(oid).map_err(err)?;
            result.entries.push(Entry { reference: name.into(), oid: oid.to_string(), summary: commit.summary().ok().flatten().unwrap_or("(non UTF-8 message)").chars().take(4096).collect(), author: commit.author().name().unwrap_or("Unknown").chars().take(256).collect(), time: commit.time().seconds() });
        }
        result.entries.sort_by(|a,b| b.time.cmp(&a.time).then(a.reference.cmp(&b.reference)));
        if result.entries.len() > 500 { result.entries.truncate(500); result.truncated = true; }
        self.fresh(revision)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHECKPOINT: &str = "refs/branchdesk/checkpoints/recovery-fixture";
    fn git(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(root)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid"])
            .args(args).env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}"); String::from_utf8(out.stdout).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, String, String) {
        let d = tempfile::tempdir().unwrap(); Repository::init(d.path()).unwrap();
        fs::write(d.path().join("file"), "base\n").unwrap();
        git(d.path(), &["add", "file"]); git(d.path(), &["commit", "-m", "base"]);
        let old = git(d.path(), &["rev-parse", "HEAD"]).trim().to_owned();
        fs::write(d.path().join("file"), "second\n").unwrap();
        git(d.path(), &["commit", "-am", "second"]);
        let head = git(d.path(), &["rev-parse", "HEAD"]).trim().to_owned();
        git(d.path(), &["update-ref", CHECKPOINT, &old]);
        fs::write(d.path().join("file"), "staged\n").unwrap(); git(d.path(), &["add", "file"]);
        fs::write(d.path().join("file"), "working\n").unwrap();
        fs::write(d.path().join("untracked"), "new\n").unwrap();
        (d, old, head)
    }
    fn preserved(root: &Path) -> Vec<Vec<u8>> {
        [".git/HEAD", ".git/index", "file", "untracked"].iter().map(|p| fs::read(root.join(p)).unwrap()).collect()
    }
    fn action(oid: &str) -> Action { Action::CreateRecoveryBranch {reference: CHECKPOINT.into(), oid: oid.into(), name: "recovery/saved".into()} }
    #[test]
    fn listed_checkpoint_creates_exact_branch_without_checkout_or_index_changes() {
        let (d, old, head) = fixture(); let before = preserved(d.path());
        let mut session = Session::new(d.path().into()).unwrap(); let state = session.snapshot().unwrap();
        let listing = session.checkpoints(&state.revision).unwrap(); assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].oid, old); assert_eq!(listing.entries[0].reference, CHECKPOINT);
        session.mutate(&state.revision, action(&old)).unwrap();
        assert_eq!(git(d.path(), &["rev-parse", "refs/heads/recovery/saved"]).trim(), old);
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), head); assert_eq!(preserved(d.path()), before);
        git(d.path(), &["fsck", "--strict"]);
    }
    #[test]
    fn changed_or_deleted_checkpoint_refuses_old_selection_then_fresh_retry_succeeds() {
        for deleted in [false, true] {
            let (d, old, head) = fixture(); let mut session = Session::new(d.path().into()).unwrap();
            let state = session.snapshot().unwrap(); assert_eq!(session.checkpoints(&state.revision).unwrap().entries[0].oid, old);
            if deleted {git(d.path(), &["update-ref", "-d", CHECKPOINT]);} else {git(d.path(), &["update-ref", CHECKPOINT, &head]);}
            let before = preserved(d.path());
            assert!(session.mutate(&state.revision, action(&old)).is_err());
            let repo = Repository::open(d.path()).unwrap(); assert!(repo.find_reference("refs/heads/recovery/saved").is_err());
            assert_eq!(preserved(d.path()), before); assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), head);
            if deleted {assert!(repo.find_reference(CHECKPOINT).is_err());} else {assert_eq!(repo.refname_to_id(CHECKPOINT).unwrap().to_string(), head);}
            git(d.path(), &["update-ref", CHECKPOINT, &old]);
            let fresh = session.snapshot().unwrap(); session.mutate(&fresh.revision, action(&old)).unwrap();
            assert_eq!(preserved(d.path()), before); assert_eq!(git(d.path(), &["rev-parse", "refs/heads/recovery/saved"]).trim(), old);
        }
    }
    #[test]
    fn foreign_source_or_destination_lock_preserved_and_explicit_retry_succeeds() {
        for reference in [CHECKPOINT, "refs/heads/recovery/saved"] {
            let (d, old, head) = fixture(); let mut session = Session::new(d.path().into()).unwrap();
            let state = session.snapshot().unwrap(); let before = preserved(d.path());
            let lock = d.path().join(".git").join(format!("{reference}.lock"));
            fs::create_dir_all(lock.parent().unwrap()).unwrap(); fs::write(&lock, "foreign owner").unwrap();
            assert!(session.mutate(&state.revision, action(&old)).is_err());
            assert_eq!(fs::read(&lock).unwrap(), b"foreign owner"); assert_eq!(preserved(d.path()), before);
            assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), head);
            assert!(Repository::open(d.path()).unwrap().find_reference("refs/heads/recovery/saved").is_err());
            fs::remove_file(lock).unwrap(); let fresh = session.snapshot().unwrap();
            session.mutate(&fresh.revision, action(&old)).unwrap(); assert_eq!(preserved(d.path()), before);
        }
    }
    #[test]
    fn namespace_spoof_existing_branch_and_symbolic_checkpoint_never_overwrite_refs() {
        let (d, old, head) = fixture(); let mut session = Session::new(d.path().into()).unwrap();
        let before = preserved(d.path());
        for (reference, name) in [("refs/heads/main", "recovery/new"), ("refs/branchdesk/checkpoints/../bad", "recovery/new"), (CHECKPOINT, "../bad")] {
            let state = session.snapshot().unwrap();
            assert!(session.mutate(&state.revision, Action::CreateRecoveryBranch{reference:reference.into(),oid:old.clone(),name:name.into()}).is_err());
        }
        git(d.path(), &["branch", "recovery/saved", &head]); let state=session.snapshot().unwrap();
        assert!(session.mutate(&state.revision, action(&old)).unwrap_err().contains("REF_EXISTS"));
        assert_eq!(git(d.path(), &["rev-parse", "refs/heads/recovery/saved"]).trim(), head);
        git(d.path(), &["branch", "-D", "recovery/saved"]);
        let repo = Repository::open(d.path()).unwrap(); repo.reference_symbolic(CHECKPOINT,"HEAD",true,"fixture").unwrap();
        let state=session.snapshot().unwrap(); assert!(session.checkpoints(&state.revision).is_err());
        assert!(session.mutate(&state.revision, action(&head)).is_err());
        assert!(repo.find_reference("refs/heads/recovery/saved").is_err());
        assert_eq!(repo.find_reference(CHECKPOINT).unwrap().symbolic_target().unwrap(),Some("HEAD"));
        assert_eq!(preserved(d.path()), before); assert_eq!(git(d.path(), &["rev-parse", "HEAD"]).trim(), head);
    }
}
