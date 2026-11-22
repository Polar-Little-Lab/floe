//! Recompute a literal file diff and apply only the selected hunk to the index tree.
use super::*;
#[derive(Debug, Serialize)]
pub struct Hunk {
    pub index: usize,
    pub header: String,
}
pub(super) fn describe(diff: &git2::Diff<'_>) -> Result<Vec<Hunk>> {
    if diff.deltas().len() != 1 {
        return Ok(Vec::new());
    }
    let delta = diff.get_delta(0).ok_or("INVALID_DIFF")?;
    let regular = |mode| matches!(mode, git2::FileMode::Blob | git2::FileMode::BlobExecutable);
    let supported = match delta.status() {
        git2::Delta::Modified => {
            regular(delta.old_file().mode()) && delta.old_file().mode() == delta.new_file().mode()
        }
        git2::Delta::Added | git2::Delta::Untracked => regular(delta.new_file().mode()),
        git2::Delta::Deleted => regular(delta.old_file().mode()),
        _ => false,
    };
    if !supported {
        return Ok(Vec::new());
    }
    let Some(patch) = git2::Patch::from_diff(diff, 0).map_err(err)? else {
        return Ok(Vec::new());
    };
    if patch.num_hunks() > 2000 {
        return Err("LIMIT: diff hunks".into());
    }
    (0..patch.num_hunks())
        .map(|index| {
            let (h, _) = patch.hunk(index).map_err(err)?;
            Ok(Hunk {
                index,
                header: String::from_utf8_lossy(h.header()).trim_end().to_string(),
            })
        })
        .collect()
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    id: &str,
    selected: usize,
    unstage: bool,
    lock: &mut IndexLock,
) -> Result<()> {
    let branch = external_merge::head_lock_target(repo)?;
    let _refs = checkout::locks(repo, &branch, &branch)?;
    session.fresh(revision)?;
    let path = session.files.get(id).ok_or("INVALID_FILE_ID")?.clone();
    let mut opts = DiffOptions::new();
    opts.pathspec(&path)
        .disable_pathspec_match(true)
        .include_untracked(true)
        .show_untracked_content(true)
        .recurse_untracked_dirs(true)
        .max_size(FILE_LIMIT as i64)
        .reverse(unstage);
    let mut index = git2::Index::open(&lock.path).map_err(err)?;
    if index.has_conflicts() {
        return Err("CONFLICT".into());
    }
    let head = match repo.head() {
        Ok(head) => Some(head.peel_to_tree().map_err(err)?),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
        Err(e) => return Err(err(e)),
    };
    let diff = if unstage {
        repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))
    } else {
        repo.diff_index_to_workdir(Some(&index), Some(&mut opts))
    }
    .map_err(err)?;
    let hunks = describe(&diff)?;
    if selected >= hunks.len() {
        return Err("INVALID_OR_UNSUPPORTED_HUNK".into());
    }
    if render_diff(&diff)?.truncated {
        return Err("LIMIT: hunk preview truncated".into());
    }
    // Untracked workdir patches must be parsed as additions for libgit2 apply.
    // The patch comes from the bounded, freshly recomputed diff, never from IPC.
    let normalized;
    let diff = if diff
        .get_delta(0)
        .is_some_and(|d| d.status() == git2::Delta::Untracked)
    {
        let mut patch = git2::Patch::from_diff(&diff, 0)
            .map_err(err)?
            .ok_or("INVALID_DIFF")?;
        normalized = git2::Diff::from_buffer(&patch.to_buf().map_err(err)?).map_err(err)?;
        &normalized
    } else {
        &diff
    };
    let before_tree = repo
        .find_tree(index.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    let mut options = git2::ApplyOptions::new();
    let mut count = 0;
    options.hunk_callback(move |h| {
        if h.is_none() {
            return false;
        }
        let chosen = count == selected;
        count += 1;
        chosen
    });
    let result = repo
        .apply_to_tree(&before_tree, diff, Some(&mut options))
        .map_err(err)?;
    // apply_to_tree returns the patched file set, not an index containing unrelated paths.
    // Merge only entries for this validated path into the original index.
    if result.has_conflicts() {
        return Err("CONFLICT".into());
    }
    if let Some(entry) = result.get_path(&path, 0) {
        index.add(&entry).map_err(err)?;
    } else if diff
        .get_delta(0)
        .is_some_and(|d| d.status() == git2::Delta::Deleted)
    {
        index.remove_path(&path).map_err(err)?;
    } else {
        return Err("MISSING_PATCHED_FILE".into());
    }
    index.write().map_err(err)?;
    session.fresh(revision)?;
    external_merge::publish(session, revision, lock, &branch, None)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, String, String) {
        let d = tempfile::tempdir().unwrap();
        let mut options = git2::RepositoryInitOptions::new();
        options.external_template(false);
        Repository::init_opts(d.path(), &options).unwrap();
        let repo = open(d.path()).unwrap();
        let base = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
        fs::write(d.path().join("file[1].txt"), &base).unwrap();
        fs::write(d.path().join("other.txt"), "original\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file[1].txt")).unwrap();
        index.add_path(Path::new("other.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Base", &tree, &[])
            .unwrap();
        let changed = base
            .replace("line 2\n", "first change\ninserted A\ninserted B\n")
            .replace("line 35\n", "last change\n");
        fs::write(d.path().join("file[1].txt"), &changed).unwrap();
        fs::write(d.path().join("other.txt"), "already staged\n").unwrap();
        index.add_path(Path::new("other.txt")).unwrap();
        index.write().unwrap();
        (d, base, changed)
    }
    fn unborn_fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.external_template(false).initial_head("main");
        Repository::init_opts(d.path(), &opts).unwrap();
        fs::write(d.path().join("new[1].txt"), "first file\n").unwrap();
        d
    }
    #[test]
    fn unborn_hunk_and_whole_file_staging_can_create_first_commit() {
        for hunk in [false, true] {
            let d = unborn_fixture();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            assert!(!d.path().join(".git/index").exists());
            assert_eq!(
                s.diff(&a.revision, &file_id("new[1].txt"), false)
                    .unwrap()
                    .hunks
                    .len(),
                1
            );
            let a = s
                .mutate(
                    &a.revision,
                    if hunk {
                        Action::StageHunk {
                            file_id: file_id("new[1].txt"),
                            hunk: 0,
                            unstage: false,
                        }
                    } else {
                        Action::StageAll
                    },
                )
                .unwrap();
            assert!(a.changes[0].staged);
            let a = s
                .mutate(
                    &a.revision,
                    if hunk {
                        Action::StageHunk {
                            file_id: file_id("new[1].txt"),
                            hunk: 0,
                            unstage: true,
                        }
                    } else {
                        Action::UnstageAll
                    },
                )
                .unwrap();
            assert!(!a.changes[0].staged);
            let a = s.mutate(&a.revision, Action::StageAll).unwrap();
            let a = s
                .mutate(
                    &a.revision,
                    Action::Commit {
                        message: "Initial".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into(),
                    },
                )
                .unwrap();
            assert_eq!(a.branch, "main");
            assert!(a.changes.is_empty());
            let repo = open(d.path()).unwrap();
            assert_eq!(
                repo.head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .parent_count(),
                0
            );
            assert_eq!(
                fs::read_to_string(d.path().join("new[1].txt")).unwrap(),
                "first file\n"
            );
        }
    }
    #[test]
    fn unborn_hunk_crash_restores_absent_index_even_after_second_recovery_kill() {
        for step in 0..=1 {
            for again in [None, Some(2), Some(3)] {
                let d = unborn_fixture();
                let head = fs::read(d.path().join(".git/HEAD")).unwrap();
                super::super::tests::kill_draft(d.path(), "hunkNew", &format!("index:{step}"));
                if let Some(n) = again {
                    super::super::tests::kill_draft(d.path(), "recover", &format!("index:{n}"));
                }
                let mut s = Session::new(d.path().into()).unwrap();
                let a = s.snapshot().unwrap();
                let a = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert!(!a.recovery_pending);
                assert!(!d.path().join(".git/index").exists());
                assert!(!d.path().join(".git/index.lock").exists());
                assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
                assert_eq!(
                    fs::read_to_string(d.path().join("new[1].txt")).unwrap(),
                    "first file\n"
                );
                s.mutate(&a.revision, Action::StageAll).unwrap();
            }
        }
    }
    #[test]
    fn unborn_recovery_refuses_foreign_empty_index_or_changed_branch() {
        for change_branch in [false, true] {
            let d = unborn_fixture();
            super::super::tests::kill_draft(d.path(), "hunkNew", "index:1");
            if change_branch {
                fs::write(d.path().join(".git/HEAD"), "ref: refs/heads/other\n").unwrap();
            } else {
                fs::write(d.path().join(".git/index"), []).unwrap();
            }
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let head = fs::read(d.path().join(".git/HEAD")).unwrap();
            // Exercise journal guards directly: a corrupt zero-byte live index may
            // be refused even earlier by snapshot, which must also remain fail-closed.
            let lease = lease::OperationLease::acquire(d.path()).unwrap();
            journal_locks::release(d.path(), &lease).unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            assert!(external_merge::recover(d.path(), &mut lock)
                .unwrap_err()
                .contains("RECOVERY_CONFLICT"));
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        }
    }
    #[test]
    fn detached_hunk_recovery_has_no_duplicate_head_lock_record() {
        let (d, _, _) = fixture();
        let repo = open(d.path()).unwrap();
        let oid = repo.head().unwrap().target().unwrap();
        repo.set_head_detached(oid).unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        super::super::tests::kill_draft(d.path(), "hunk", "index:1");
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
        assert!(repo.head_detached().unwrap());
        assert_eq!(repo.head().unwrap().target(), Some(oid));
    }
    #[test]
    fn missing_index_in_committed_repository_is_not_silently_replaced() {
        let (d, _, _) = fixture();
        fs::remove_file(d.path().join(".git/index")).unwrap();
        assert!(IndexLock::acquire(d.path())
            .err()
            .unwrap()
            .contains("MISSING_INDEX"));
        assert!(!d.path().join(".git/index").exists());
        assert!(!d.path().join(".git/index.lock").exists());
    }
    fn indexed(root: &Path, name: &str) -> String {
        let repo = open(root).unwrap();
        let entry = repo.index().unwrap().get_path(Path::new(name), 0).unwrap();
        let text = String::from_utf8(repo.find_blob(entry.id).unwrap().content().to_vec()).unwrap();
        text
    }
    #[test]
    fn added_and_deleted_text_hunks_roundtrip_without_touching_worktree() {
        for deleted in [false, true] {
            let (d, base, _) = fixture();
            let name = if deleted { "file[1].txt" } else { "new[1].txt" };
            if deleted {
                fs::remove_file(d.path().join(name)).unwrap();
            } else {
                fs::write(d.path().join(name), "new file\nno trailing newline").unwrap();
            }
            let mut session = Session::new(d.path().into()).unwrap();
            for unstage in [false, true] {
                let snapshot = session.snapshot().unwrap();
                let preview = session
                    .diff(&snapshot.revision, &file_id(name), unstage)
                    .unwrap();
                assert_eq!(
                    preview.hunks.len(),
                    1,
                    "{deleted} {unstage}: {}",
                    preview.text
                );
                session
                    .mutate(
                        &snapshot.revision,
                        Action::StageHunk {
                            file_id: file_id(name),
                            hunk: 0,
                            unstage,
                        },
                    )
                    .unwrap();
                let exists = open(d.path())
                    .unwrap()
                    .index()
                    .unwrap()
                    .get_path(Path::new(name), 0)
                    .is_some();
                assert_eq!(exists, deleted == unstage);
                if exists {
                    assert_eq!(
                        indexed(d.path(), name),
                        if deleted {
                            base.as_str()
                        } else {
                            "new file\nno trailing newline"
                        }
                    );
                }
                assert_eq!(indexed(d.path(), "other.txt"), "already staged\n");
                if deleted {
                    assert!(!d.path().join(name).exists());
                } else {
                    assert_eq!(
                        fs::read_to_string(d.path().join(name)).unwrap(),
                        "new file\nno trailing newline"
                    );
                }
            }
        }
    }
    #[test]
    fn added_and_deleted_hunks_restore_index_after_actual_publication_kill() {
        for deleted in [false, true] {
            let (d, _, _) = fixture();
            if deleted {
                fs::remove_file(d.path().join("file[1].txt")).unwrap();
            } else {
                fs::write(d.path().join("new[1].txt"), "new content\r\n").unwrap();
            }
            let before = fs::read(d.path().join(".git/index")).unwrap();
            super::super::tests::kill_draft(
                d.path(),
                if deleted { "hunkDelete" } else { "hunkNew" },
                "index:1",
            );
            let mut session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(snapshot.recovery_pending);
            session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            if deleted {
                assert!(!d.path().join("file[1].txt").exists());
            } else {
                assert_eq!(
                    fs::read(d.path().join("new[1].txt")).unwrap(),
                    b"new content\r\n"
                );
            }
        }
    }
    #[test]
    fn cli_created_index_exposes_hunks() {
        let d = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let mut command = std::process::Command::new("git");
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("GIT_") {
                    command.env_remove(key);
                }
            }
            let output = command
                .current_dir(d.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", d.path().join("absent-config"))
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        };
        git(&["init", "--template=", "-b", "main"]);
        let base = (0..40).map(|n| format!("Line {n}\n")).collect::<String>();
        fs::write(d.path().join("file.txt"), &base).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "Initial"]);
        fs::write(
            d.path().join("file.txt"),
            base.replace("Line 2\n", "First change\n")
                .replace("Line 35\n", "Second change\n"),
        )
        .unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let diff = s.diff(&a.revision, &file_id("file.txt"), false).unwrap();
        assert_eq!(diff.hunks.len(), 2, "{:?}", diff);
    }
    #[test]
    fn hunk_descriptors_match_visible_patch_headers() {
        let (d, _, _) = fixture();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let diff = session
            .diff(&snapshot.revision, &file_id("file[1].txt"), false)
            .unwrap();
        assert!(!diff.hunks.is_empty());
        for hunk in diff.hunks {
            assert!(
                diff.text.lines().any(|line| line.trim_end() == hunk.header),
                "header {:?} not found in {:?}",
                hunk.header,
                diff.text
            );
        }
    }
    #[test]
    fn actual_kill_and_second_recovery_kill_preserve_index_and_worktree() {
        for step in 0..=1 {
            for recovery_step in [None, Some(2), Some(3)] {
                let (d, _, changed) = fixture();
                let before = fs::read(d.path().join(".git/index")).unwrap();
                super::super::tests::kill_draft(d.path(), "hunk", &format!("index:{step}"));
                if let Some(n) = recovery_step {
                    super::super::tests::kill_draft(d.path(), "recover", &format!("index:{n}"));
                }
                let mut session = Session::new(d.path().into()).unwrap();
                let snapshot = session.snapshot().unwrap();
                assert!(snapshot.recovery_pending);
                let recovered = session
                    .mutate(&snapshot.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!recovered.recovery_pending);
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
                assert_eq!(
                    fs::read_to_string(d.path().join("file[1].txt")).unwrap(),
                    changed
                );
                assert_eq!(indexed(d.path(), "other.txt"), "already staged\n");
                assert!(!d.path().join(".git/index.lock").exists());
            }
        }
    }
    #[test]
    fn selected_hunks_roundtrip_and_preserve_worktree_and_other_staged_files() {
        let (d, base, changed) = fixture();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let id = file_id("file[1].txt");
        assert_eq!(
            session.diff(&s.revision, &id, false).unwrap().hunks.len(),
            2
        );
        let next = session
            .mutate(
                &s.revision,
                Action::StageHunk {
                    file_id: id.clone(),
                    hunk: 1,
                    unstage: false,
                },
            )
            .unwrap();
        assert_eq!(
            indexed(d.path(), "file[1].txt"),
            base.replace("line 35\n", "last change\n")
        );
        assert_eq!(indexed(d.path(), "other.txt"), "already staged\n");
        assert_eq!(
            fs::read_to_string(d.path().join("file[1].txt")).unwrap(),
            changed
        );
        assert!(session
            .mutate(
                &s.revision,
                Action::StageHunk {
                    file_id: id.clone(),
                    hunk: 0,
                    unstage: false
                }
            )
            .is_err());
        let next = session
            .mutate(
                &next.revision,
                Action::StageHunk {
                    file_id: id.clone(),
                    hunk: 0,
                    unstage: false,
                },
            )
            .unwrap();
        assert_eq!(indexed(d.path(), "file[1].txt"), changed);
        assert_eq!(
            session.diff(&next.revision, &id, true).unwrap().hunks.len(),
            2
        );
        session
            .mutate(
                &next.revision,
                Action::StageHunk {
                    file_id: id,
                    hunk: 1,
                    unstage: true,
                },
            )
            .unwrap();
        assert_eq!(
            indexed(d.path(), "file[1].txt"),
            base.replace("line 2\n", "first change\ninserted A\ninserted B\n")
        );
        assert_eq!(indexed(d.path(), "other.txt"), "already staged\n");
        assert_eq!(
            fs::read_to_string(d.path().join("file[1].txt")).unwrap(),
            changed
        );
    }
    #[test]
    fn interrupted_hunk_publication_rolls_back_index_and_keeps_later_worktree_edit() {
        let (d, _, _) = fixture();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        let repo = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        execute(
            &mut session,
            &repo,
            &s.revision,
            &file_id("file[1].txt"),
            0,
            false,
            &mut lock,
        )
        .unwrap();
        drop(lock);
        fs::write(d.path().join("file[1].txt"), "later edit\n").unwrap();
        let s = session.snapshot().unwrap();
        assert!(s.recovery_pending);
        let s = session
            .mutate(&s.revision, Action::RecoverCheckout)
            .unwrap();
        assert!(!s.recovery_pending);
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
        assert_eq!(
            fs::read_to_string(d.path().join("file[1].txt")).unwrap(),
            "later edit\n"
        );
    }
    #[test]
    fn rejects_invalid_hunk_binary_and_file_modes_without_index_writes() {
        let (d, _, _) = fixture();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        for id in [file_id("file[1].txt"), "unknown".into()] {
            assert!(session
                .mutate(
                    &s.revision,
                    Action::StageHunk {
                        file_id: id,
                        hunk: 999,
                        unstage: false
                    }
                )
                .is_err());
        }
        fs::write(d.path().join("file[1].txt"), b"binary\0data").unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .diff(&s.revision, &file_id("file[1].txt"), false)
            .unwrap()
            .hunks
            .is_empty());
        assert!(session
            .mutate(
                &s.revision,
                Action::StageHunk {
                    file_id: file_id("file[1].txt"),
                    hunk: 0,
                    unstage: false
                }
            )
            .is_err());
        fs::write(d.path().join("new.txt"), b"new binary\0file").unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .diff(&s.revision, &file_id("new.txt"), false)
            .unwrap()
            .hunks
            .is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(d.path().join("file[1].txt"), "text change\n").unwrap();
            fs::set_permissions(
                d.path().join("file[1].txt"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            let s = session.snapshot().unwrap();
            assert!(session
                .diff(&s.revision, &file_id("file[1].txt"), false)
                .unwrap()
                .hunks
                .is_empty());
            assert!(session
                .mutate(
                    &s.revision,
                    Action::StageHunk {
                        file_id: file_id("file[1].txt"),
                        hunk: 0,
                        unstage: false
                    }
                )
                .is_err());
        }
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
    }
}
