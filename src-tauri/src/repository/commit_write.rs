//! Journaled roll-forward of an ordinary (including initial/detached) commit.
use super::*;
#[cfg(test)]
const JOURNAL: &str = ".git/branchdesk-commit";
fn journal(root:&Path)->Result<PathBuf>{layout::private_file(root,"branchdesk-commit")}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    #[serde(default,skip_serializing_if="Option::is_none")]
    owner: Option<PathBuf>,
    #[serde(default)]
    amend: bool,
    target: String,
    old: Option<String>,
    new: String,
    index: String,
}
pub(super) fn pending(root: &Path) -> bool {
    journal(root).is_ok_and(|path|path.exists())
}
fn target_oid(repo: &Repository, target: &str) -> Result<Option<String>> {
    match repo.find_reference(target) {
        Ok(r) => Ok(Some(r.target().ok_or("INVALID_COMMIT_TARGET")?.to_string())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    message: &str,
    author: &str,
    email: &str,
    amend: bool,
) -> Result<()> {
    if message.trim().is_empty()
        || message.len() > 10000
        || message.contains('\0')
        || author.trim().is_empty()
        || author.len() > 256
        || author.contains(['\n', '\r', '\0'])
        || email.len() > 256
        || !email.contains('@')
        || email.contains(['\n', '\r', '\0'])
    {
        return Err("INVALID_INPUT: message/author/email".into());
    }
    // Match non-editor Git commit -m whitespace cleanup, retaining # lines.
    let message = git2::message_prettify(message, None).map_err(err)?;
    if message.is_empty() {
        return Err("INVALID_INPUT: empty cleaned message".into());
    }
    let signature = git2::Signature::now(author, email).map_err(err)?;
    let target = external_merge::head_lock_target(repo)?;
    let tx = checkout::locks(repo, &target, &target)?;
    let snapshot = session.fresh(revision)?;
    if !amend && !snapshot.changes.iter().any(|c| c.staged) {
        return Err("EMPTY_COMMIT".into());
    }
    let old = target_oid(repo, &target)?;
    let parent = old
        .as_ref()
        .map(|id| {
            repo.find_commit(Oid::from_str(id).map_err(err)?)
                .map_err(err)
        })
        .transpose()?;
    let mut index = repo.index().map_err(err)?;
    if index.has_conflicts() {
        return Err("CONFLICT".into());
    }
    let tree = repo
        .find_tree(index.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    checkout::inspect_tree(repo, &tree)?;
    if amend && parent.is_none() {
        return Err("NO_COMMIT_TO_AMEND".into());
    }
    let original_author = parent.as_ref().map(|c| c.author());
    let parent_ids: Vec<_> = if amend {
        parent.as_ref().unwrap().parent_ids().collect()
    } else {
        parent.iter().map(|c| c.id()).collect()
    };
    let parent_commits: Vec<_> = parent_ids
        .into_iter()
        .map(|id| repo.find_commit(id).map_err(err))
        .collect::<Result<_>>()?;
    let parents: Vec<_> = parent_commits.iter().collect();
    let new = repo
        .commit(
            None,
            if amend {
                original_author.as_ref().unwrap()
            } else {
                &signature
            },
            &signature,
            &message,
            &tree,
            &parents,
        )
        .map_err(err)?;
    session.fresh(revision)?;
    let record = Record {
        version: 1,
        owner: layout::journal_owner(&session.root)?,
        amend,
        target: target.clone(),
        old,
        new: new.to_string(),
        index: checkout::digest(&bounded(
            &layout::private_file(&session.root,"index")?,
            20 * 1024 * 1024,
        )?),
    };
    if amend {
        repo.reference(
            &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
            parent.as_ref().unwrap().id(),
            false,
            "pre-amend recovery checkpoint",
        )
        .map_err(err)?;
    }
    repo.reference(
        &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
        new,
        false,
        "commit recovery checkpoint",
    )
    .map_err(err)?;
    let dir = journal(&session.root)?;
    fs::create_dir(&dir).map_err(err)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    journal_locks::record(&session.root, &dir, Some(&target), false, false)?;
    checkout::sync_dir(&layout::Layout::read(&session.root)?.private)?;
    complete(&session.root, repo, tx, &record)
}
fn complete(
    root: &Path,
    repo: &Repository,
    mut tx: git2::Transaction<'_>,
    record: &Record,
) -> Result<()> {
    if record.version != 1 || record.owner != layout::journal_owner(root)? {
        return Err("INVALID_COMMIT_JOURNAL".into());
    }
    if repo.state() != git2::RepositoryState::Clean
        || external_merge::head_lock_target(repo)? != record.target
    {
        return Err("RECOVERY_CONFLICT: HEAD/state changed".into());
    }
    let new = repo
        .find_commit(Oid::from_str(&record.new).map_err(err)?)
        .map_err(err)?;
    let parents: Vec<_> = new.parent_ids().map(|id| id.to_string()).collect();
    let expected = if record.amend {
        let previous = repo
            .find_commit(
                Oid::from_str(record.old.as_deref().ok_or("INVALID_AMEND_JOURNAL")?)
                    .map_err(err)?,
            )
            .map_err(err)?;
        let original = previous.author();
        let author = new.author();
        if original.name_bytes() != author.name_bytes()
            || original.email_bytes() != author.email_bytes()
            || original.when().seconds() != author.when().seconds()
            || original.when().offset_minutes() != author.when().offset_minutes()
        {
            return Err("INVALID_AMEND_AUTHOR".into());
        }
        previous
            .parent_ids()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    } else {
        record.old.iter().cloned().collect()
    };
    if parents != expected {
        return Err("INVALID_COMMIT_PARENTS".into());
    }
    let current = target_oid(repo, &record.target)?;
    if (current != record.old && current.as_deref() != Some(record.new.as_str()))
        || checkout::digest(&bounded(&layout::private_file(root,"index")?, 20 * 1024 * 1024)?) != record.index
    {
        return Err("RECOVERY_CONFLICT: ref/index changed".into());
    }
    let mut index = repo.index().map_err(err)?;
    if index.has_conflicts() || index.write_tree_to(repo).map_err(err)? != new.tree_id() {
        return Err("INVALID_COMMIT_TREE".into());
    }
    checkpoint(root, 0);
    if current == record.old {
        tx.set_target(
            &record.target,
            new.id(),
            Some(&new.committer()),
            "commit: FLOE",
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    checkpoint(root, 1);
    Ok(())
}
pub(super) fn recover(root: &Path, repo: &Repository) -> Result<()> {
    let dir = journal(root)?;
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 16 * 1024)?).map_err(err)?;
    if record.owner != layout::journal_owner(root)? {return Err("INVALID_COMMIT_JOURNAL_OWNER".into());}
    let tx = checkout::locks(repo, &record.target, &record.target)?;
    journal_locks::record(root, &dir, Some(&record.target), false, false)?;
    complete(root, repo, tx, &record)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        journal(root)?,
        layout::private_file(root,&format!("branchdesk-commit-completed-{}",Uuid::new_v4()))?,
    )
    .map_err(err)?;
    checkout::sync_dir(&layout::Layout::read(root)?.private)
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_DRAFT_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_DRAFT_CHECKPOINT").ok().as_deref()
            == Some(format!("commit:{step}").as_str())
    {
        fs::write(layout::private_file(root,"branchdesk-draft-test.ready").unwrap(), "ready").unwrap();
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
    fn fixture(kind: usize) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.external_template(false).initial_head("main");
        let repo = Repository::init_opts(d.path(), &opts).unwrap();
        fs::write(d.path().join("file.txt"), "base\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        if kind != 0 {
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("A", "a@example.invalid").unwrap();
            let old = repo
                .commit(Some("HEAD"), &sig, &sig, "Base", &tree, &[])
                .unwrap();
            if kind == 2 {
                repo.set_head_detached(old).unwrap();
            }
        }
        fs::write(d.path().join("file.txt"), "staged\n").unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        fs::write(d.path().join("file.txt"), "unstaged\n").unwrap();
        d
    }
    fn amend_action() -> Action {
        Action::AmendCommit {
            message: "Amended message".into(),
            author: "Committer".into(),
            email: "committer@example.invalid".into(),
        }
    }
    #[test]
    fn amend_preserves_original_author_and_zero_one_or_merge_parents() {
        for parent_count in 0..=2 {
            for message_only in [false, true] {
                let d = fixture(1);
                let repo = open(d.path()).unwrap();
                let base = repo.head().unwrap().peel_to_commit().unwrap();
                let sig = git2::Signature::now("Original", "original@example.invalid").unwrap();
                if parent_count > 0 {
                    let other = repo
                        .commit(None, &sig, &sig, "Other", &base.tree().unwrap(), &[])
                        .unwrap();
                    let other = repo.find_commit(other).unwrap();
                    let parents = if parent_count == 1 {
                        vec![&base]
                    } else {
                        vec![&base, &other]
                    };
                    repo.commit(
                        Some("HEAD"),
                        &sig,
                        &sig,
                        "Previous",
                        &base.tree().unwrap(),
                        &parents,
                    )
                    .unwrap();
                }
                let old = repo.head().unwrap().peel_to_commit().unwrap();
                if message_only {
                    let mut index = repo.index().unwrap();
                    index.read_tree(&old.tree().unwrap()).unwrap();
                    index.write().unwrap();
                }
                let index = fs::read(d.path().join(".git/index")).unwrap();
                let mut session = Session::new(d.path().into()).unwrap();
                let snapshot = session.snapshot().unwrap();
                session.mutate(&snapshot.revision, amend_action()).unwrap();
                let new = repo.head().unwrap().peel_to_commit().unwrap();
                assert_ne!(new.id(), old.id());
                assert_eq!(
                    new.parent_ids().collect::<Vec<_>>(),
                    old.parent_ids().collect::<Vec<_>>()
                );
                assert_eq!(new.author().name_bytes(), old.author().name_bytes());
                assert_eq!(new.author().email_bytes(), old.author().email_bytes());
                assert_eq!(new.author().when().seconds(), old.author().when().seconds());
                assert_eq!(new.committer().name().unwrap(), "Committer");
                assert_eq!(new.message().unwrap(), "Amended message\n");
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(d.path().join("file.txt")).unwrap(),
                    "unstaged\n"
                );
                let entry = new.tree().unwrap().get_path(Path::new("file.txt")).unwrap();
                assert_eq!(
                    repo.find_blob(entry.id()).unwrap().content(),
                    if message_only {
                        b"base\n".as_slice()
                    } else {
                        b"staged\n".as_slice()
                    }
                );
                assert!(repo
                    .references_glob("refs/branchdesk/checkpoints/*")
                    .unwrap()
                    .any(|r| r.unwrap().target() == Some(old.id())));
            }
        }
    }
    #[test]
    fn amend_actual_kill_and_recovery_keep_exact_oid_including_detached_head() {
        for kind in [1, 2] {
            for step in 0..=1 {
                let d = fixture(kind);
                let repo = open(d.path()).unwrap();
                let old = repo.head().unwrap().peel_to_commit().unwrap();
                super::super::tests::kill_draft(d.path(), "amend", &format!("commit:{step}"));
                let record: Record = serde_json::from_slice(
                    &fs::read(d.path().join(JOURNAL).join("record.json")).unwrap(),
                )
                .unwrap();
                super::super::tests::kill_draft(d.path(), "recover", &format!("commit:{step}"));
                let mut session = Session::new(d.path().into()).unwrap();
                let snapshot = session.snapshot().unwrap();
                session
                    .mutate(&snapshot.revision, Action::RecoverCheckout)
                    .unwrap();
                let new = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(new.id().to_string(), record.new);
                assert_eq!(
                    new.parent_ids().collect::<Vec<_>>(),
                    old.parent_ids().collect::<Vec<_>>()
                );
                assert_eq!(new.author().name_bytes(), old.author().name_bytes());
                assert_eq!(repo.head_detached().unwrap(), kind == 2);
            }
        }
    }
    #[test]
    fn amend_refuses_unborn_and_stale_requests_before_journal() {
        let d = fixture(0);
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(&a.revision, amend_action())
            .unwrap_err()
            .contains("NO_COMMIT_TO_AMEND"));
        assert!(!pending(d.path()));
        let d = fixture(1);
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        fs::write(d.path().join("file.txt"), "later").unwrap();
        assert!(s
            .mutate(&a.revision, amend_action())
            .unwrap_err()
            .contains("STALE_STATE"));
        assert!(!pending(d.path()));
    }
    #[test]
    fn commit_actual_kill_and_recovery_reentry_reuse_exact_commit() {
        for kind in 0..=2 {
            for step in 0..=1 {
                for again in [None, Some(0), Some(1)] {
                    let d = fixture(kind);
                    let before = fs::read(d.path().join(".git/index")).unwrap();
                    let old = open(d.path()).unwrap().head().ok().and_then(|h| h.target());
                    super::super::tests::kill_draft(d.path(), "commit", &format!("commit:{step}"));
                    let record: Record = serde_json::from_slice(
                        &fs::read(d.path().join(JOURNAL).join("record.json")).unwrap(),
                    )
                    .unwrap();
                    if let Some(n) = again {
                        super::super::tests::kill_draft(
                            d.path(),
                            "recover",
                            &format!("commit:{n}"),
                        );
                    }
                    fs::write(d.path().join("file.txt"), "later local edit\n").unwrap();
                    let mut session = Session::new(d.path().into()).unwrap();
                    let snapshot = session.snapshot().unwrap();
                    assert!(snapshot.recovery_pending);
                    let after = session
                        .mutate(&snapshot.revision, Action::RecoverCheckout)
                        .unwrap();
                    assert!(!after.recovery_pending);
                    let repo = open(d.path()).unwrap();
                    let commit = repo.head().unwrap().peel_to_commit().unwrap();
                    assert_eq!(commit.id().to_string(), record.new);
                    assert_eq!(
                        commit.parent_ids().collect::<Vec<_>>(),
                        old.into_iter().collect::<Vec<_>>()
                    );
                    assert_eq!(repo.head_detached().unwrap(), kind == 2);
                    let entry = commit
                        .tree()
                        .unwrap()
                        .get_path(Path::new("file.txt"))
                        .unwrap();
                    assert_eq!(repo.find_blob(entry.id()).unwrap().content(), b"staged\n");
                    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
                    assert_eq!(
                        fs::read_to_string(d.path().join("file.txt")).unwrap(),
                        "later local edit\n"
                    );
                    assert!(!d.path().join(".git/index.lock").exists());
                    assert!(!d.path().join(".git/HEAD.lock").exists());
                }
            }
        }
    }
    #[test]
    fn commit_recovery_refuses_foreign_index_or_head() {
        for change_head in [false, true] {
            let d = fixture(1);
            super::super::tests::kill_draft(d.path(), "commit", "commit:0");
            {
                let lease = lease::OperationLease::acquire(d.path()).unwrap();
                journal_locks::release(d.path(), &lease).unwrap();
            }
            let repo = open(d.path()).unwrap();
            if change_head {
                repo.set_head_detached(repo.head().unwrap().target().unwrap())
                    .unwrap();
            } else {
                let mut index = repo.index().unwrap();
                index.add_path(Path::new("file.txt")).unwrap();
                index.write().unwrap();
            }
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let head = fs::read(d.path().join(".git/HEAD")).unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let error = session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap_err();
            assert!(error.contains("RECOVERY_CONFLICT"), "{error}");
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        }
    }
    #[test]
    fn invalid_commit_inputs_do_not_create_journal_or_change_head() {
        let d = fixture(1);
        let repo = open(d.path()).unwrap();
        let head = repo.head().unwrap().target();
        let mut s = Session::new(d.path().into()).unwrap();
        for (message, author, email) in [
            ("", "A", "a@example.invalid"),
            ("bad\0message", "A", "a@example.invalid"),
            ("Message", "A\nB", "a@example.invalid"),
            ("Message", "A", "a@example.invalid\n"),
        ] {
            let a = s.snapshot().unwrap();
            assert!(s
                .mutate(
                    &a.revision,
                    Action::Commit {
                        message: message.into(),
                        author: author.into(),
                        email: email.into()
                    }
                )
                .unwrap_err()
                .contains("INVALID_INPUT"));
            assert!(!pending(d.path()));
            assert_eq!(repo.head().unwrap().target(), head);
        }
    }
}
