use super::tests::{checkout_target, fixture};
use super::*;
#[test]
fn detached_checkout_moves_only_head_and_can_return_to_branch() {
    let d = fixture();
    let target = checkout_target(d.path());
    let repo = Repository::open(d.path()).unwrap();
    let original = repo.head().unwrap().target().unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    let mut session = Session::new(d.path().into()).unwrap();
    let before = session.snapshot().unwrap();
    let detached = session
        .mutate(
            &before.revision,
            Action::CheckoutCommit {
                oid: target.clone(),
            },
        )
        .unwrap();
    assert!(detached.detached);
    assert_eq!(detached.head_oid.as_deref(), Some(target.as_str()));
    assert!(detached.changes.is_empty());
    assert_eq!(
        repo.refname_to_id(&format!("refs/heads/{branch}")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "target\n"
    );
    let back = session
        .mutate(&detached.revision, Action::Checkout { name: branch })
        .unwrap();
    assert!(!back.detached);
    assert_eq!(back.head_oid, Some(original.to_string()));
    assert!(back.changes.is_empty());
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "hello\n"
    );
    assert!(!d.path().join("new.txt").exists());
}
#[test]
fn detached_checkout_refuses_dirty_stale_unknown_and_foreign_lock() {
    let d = fixture();
    let target = checkout_target(d.path());
    let mut session = Session::new(d.path().into()).unwrap();
    let before = session.snapshot().unwrap();
    let head = fs::read(d.path().join(".git/HEAD")).unwrap();
    for oid in ["HEAD".into(), "0".repeat(40)] {
        assert!(session
            .mutate(&before.revision, Action::CheckoutCommit { oid })
            .is_err());
    }
    fs::write(d.path().join("hello.txt"), "keep dirty").unwrap();
    assert!(session
        .mutate(
            &before.revision,
            Action::CheckoutCommit {
                oid: target.clone()
            }
        )
        .is_err());
    let dirty = session.snapshot().unwrap();
    assert!(session
        .mutate(
            &dirty.revision,
            Action::CheckoutCommit {
                oid: target.clone()
            }
        )
        .unwrap_err()
        .contains("DIRTY"));
    assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "keep dirty"
    );
    fs::write(d.path().join("hello.txt"), "hello\n").unwrap();
    let clean = session.snapshot().unwrap();
    fs::write(d.path().join(".git/HEAD.lock"), "foreign").unwrap();
    assert!(session
        .mutate(&clean.revision, Action::CheckoutCommit { oid: target })
        .is_err());
    assert_eq!(
        fs::read_to_string(d.path().join(".git/HEAD.lock")).unwrap(),
        "foreign"
    );
    assert!(!checkout::pending(d.path()));
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn suspended(root: &Path, step: usize, oid: Option<&str>, recover: bool) -> Child {
    let ready = root.join(".git/branchdesk-checkout/child.ready");
    if ready.exists() {
        fs::remove_file(&ready).unwrap();
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "repository::tests::checkout_crash_worker",
            "--nocapture",
        ])
        .env("BRANCHDESK_CRASH_ROOT", root)
        .env(
            if recover {
                "BRANCHDESK_RECOVERY_STEP"
            } else {
                "BRANCHDESK_CRASH_STEP"
            },
            step.to_string(),
        );
    if let Some(oid) = oid {
        command.env("BRANCHDESK_DETACHED_OID", oid);
    }
    let mut child = Child(
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "detached worker exited"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "detached checkpoint timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child
}
#[test]
fn detached_checkout_actual_kills_restore_symbolic_or_direct_head() {
    for kind in 0..3 {
        for step in [100, 101, 102, 0, 1, 2, 3, 4] {
            let d = fixture();
            let target = checkout_target(d.path());
            let repo = Repository::open(d.path()).unwrap();
            if kind > 0 {
                repo.set_head_detached(repo.head().unwrap().target().unwrap())
                    .unwrap();
            }
            let head = fs::read(d.path().join(".git/HEAD")).unwrap();
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let mut child = suspended(
                d.path(),
                step,
                if kind == 2 { None } else { Some(&target) },
                false,
            );
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let before = session.snapshot().unwrap();
            assert!(before.recovery_pending);
            let after = session
                .mutate(&before.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!after.recovery_pending);
            assert_eq!(after.detached, kind > 0);
            assert!(after.changes.is_empty());
            assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("new.txt").exists());
        }
    }
}
#[test]
fn detached_checkout_recovery_resumes_after_second_actual_kill() {
    for kind in 0..3 {
        for step in 0..=4 {
            let d = fixture();
            let target = checkout_target(d.path());
            let repo = Repository::open(d.path()).unwrap();
            if kind > 0 {
                repo.set_head_detached(repo.head().unwrap().target().unwrap())
                    .unwrap();
            }
            let head = fs::read(d.path().join(".git/HEAD")).unwrap();
            let mut writer = suspended(
                d.path(),
                4,
                if kind == 2 { None } else { Some(&target) },
                false,
            );
            writer.0.kill().unwrap();
            writer.0.wait().unwrap();
            let mut recovery = suspended(d.path(), step, None, true);
            recovery.0.kill().unwrap();
            recovery.0.wait().unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let after = session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!after.recovery_pending);
            assert!(after.changes.is_empty());
            assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("new.txt").exists());
        }
    }
}
#[test]
fn detached_checkout_recovery_preserves_external_head_and_invalid_version() {
    for invalid_version in [false, true] {
        let d = fixture();
        let target = checkout_target(d.path());
        let mut writer = suspended(d.path(), 4, Some(&target), false);
        writer.0.kill().unwrap();
        writer.0.wait().unwrap();
        let repo = Repository::open(d.path()).unwrap();
        if invalid_version {
            let path = d.path().join(".git/branchdesk-checkout/manifest.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            v["version"] = 1.into();
            fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
        } else {
            let sig = git2::Signature::now("External", "external@example.invalid").unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            let tree = parent.tree().unwrap();
            let oid = repo
                .commit(None, &sig, &sig, "external", &tree, &[&parent])
                .unwrap();
            fs::write(d.path().join(".git/HEAD"), format!("{oid}\n")).unwrap();
        }
        let head = fs::read(d.path().join(".git/HEAD")).unwrap();
        let file = fs::read(d.path().join("hello.txt")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(session
            .mutate(&snapshot.revision, Action::RecoverCheckout)
            .is_err());
        assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), file);
        assert!(checkout::pending(d.path()));
    }
}
#[test]
fn create_branch_at_selected_commit_preserves_staged_unstaged_and_head() {
    let d = fixture();
    let target = checkout_target(d.path());
    let repo = Repository::open(d.path()).unwrap();
    fs::write(d.path().join("hello.txt"), "staged\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("hello.txt")).unwrap();
    index.write().unwrap();
    fs::write(d.path().join("hello.txt"), "unstaged\n").unwrap();
    fs::write(d.path().join("untracked.txt"), "keep\n").unwrap();
    let head = fs::read(d.path().join(".git/HEAD")).unwrap();
    let index = fs::read(d.path().join(".git/index")).unwrap();
    let mut session = Session::new(d.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    let after = session
        .mutate(
            &s.revision,
            Action::CreateBranchAt {
                name: "selected/base".into(),
                oid: target.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        repo.refname_to_id("refs/heads/selected/base")
            .unwrap()
            .to_string(),
        target
    );
    assert_eq!(after.head_oid, s.head_oid);
    assert!(!after.detached);
    assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
        "keep\n"
    );
    for name in ["HEAD", "-bad", "bad..name", "selected/base"] {
        assert!(session
            .mutate(
                &after.revision,
                Action::CreateBranchAt {
                    name: name.into(),
                    oid: target.clone()
                }
            )
            .is_err());
    }
    assert_eq!(
        repo.refname_to_id("refs/heads/selected/base")
            .unwrap()
            .to_string(),
        target
    );
    assert!(session
        .mutate(
            &s.revision,
            Action::CreateBranchAt {
                name: "stale".into(),
                oid: target.clone()
            }
        )
        .is_err());
    assert!(session
        .mutate(
            &after.revision,
            Action::CreateBranchAt {
                name: "unknown".into(),
                oid: "0".repeat(40)
            }
        )
        .is_err());
    fs::write(d.path().join(".git/refs/heads/locked.lock"), "foreign").unwrap();
    assert!(session
        .mutate(
            &after.revision,
            Action::CreateBranchAt {
                name: "locked".into(),
                oid: target
            }
        )
        .is_err());
    assert_eq!(
        fs::read_to_string(d.path().join(".git/refs/heads/locked.lock")).unwrap(),
        "foreign"
    );
}
#[test]
fn same_commit_detach_commit_and_branch_retention_round_trip() {
    let d = fixture();
    let repo = Repository::open(d.path()).unwrap();
    let initial = repo.head().unwrap().target().unwrap().to_string();
    let original_branch = repo.head().unwrap().shorthand().unwrap().to_string();
    let mut session = Session::new(d.path().into()).unwrap();
    let mut s = session.snapshot().unwrap();
    // Even a checkout of the current commit must detach a symbolic HEAD.
    s = session
        .mutate(
            &s.revision,
            Action::CheckoutCommit {
                oid: initial.clone(),
            },
        )
        .unwrap();
    assert!(s.detached);
    s = session
        .mutate(&s.revision, Action::CheckoutCommit { oid: initial })
        .unwrap();
    assert!(s.detached);
    assert!(s.changes.is_empty());
    fs::write(d.path().join("hello.txt"), "detached work\n").unwrap();
    s = session.snapshot().unwrap();
    s = session.mutate(&s.revision, Action::StageAll).unwrap();
    s = session
        .mutate(
            &s.revision,
            Action::Commit {
                message: "Keep detached work".into(),
                author: "Test".into(),
                email: "test@example.invalid".into(),
            },
        )
        .unwrap();
    let detached_oid = s.head_oid.clone().unwrap();
    assert!(s.detached);
    s = session
        .mutate(
            &s.revision,
            Action::CreateBranchAt {
                name: "keep/work".into(),
                oid: detached_oid.clone(),
            },
        )
        .unwrap();
    assert!(s.detached);
    s = session
        .mutate(
            &s.revision,
            Action::Checkout {
                name: original_branch,
            },
        )
        .unwrap();
    assert!(!s.detached);
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "hello\n"
    );
    s = session
        .mutate(
            &s.revision,
            Action::Checkout {
                name: "keep/work".into(),
            },
        )
        .unwrap();
    assert_eq!(s.head_oid, Some(detached_oid));
    assert!(s.changes.is_empty());
    assert!(!s.detached);
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "detached work\n"
    );
}
