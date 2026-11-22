//! Independent Git CLI oracle. Commands are confined to disposable synthetic repositories.
use super::*;
fn git(root: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.current_dir(root)
        .args([
            "-c",
            "user.name=Parity",
            "-c",
            "user.email=parity@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            root.join(".git/branchdesk-empty-test-config"),
        )
        .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00+00:00")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .expect("Git CLI required for parity tests")
}
fn ok(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = git(root, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    ok(root, &["init", "--template=", "-b", "main"]);
    fs::write(root.join("file.txt"), "base\n").unwrap();
    fs::write(root.join("unrelated.txt"), "original\n").unwrap();
    ok(root, &["add", "."]);
    ok(root, &["commit", "-m", "Base"]);
    ok(root, &["checkout", "-b", "topic"]);
    fs::write(root.join("file.txt"), "incoming\n").unwrap();
    fs::write(root.join("added.txt"), "incoming file\n").unwrap();
    ok(root, &["add", "."]);
    ok(root, &["commit", "-m", "Topic"]);
    ok(root, &["checkout", "main"]);
    fs::write(root.join("file.txt"), "ours\n").unwrap();
    ok(root, &["add", "file.txt"]);
    ok(root, &["commit", "-m", "Ours"]);
    fs::write(root.join("unrelated.txt"), "pre-merge local edit\n").unwrap();
    fs::write(root.join("untracked.txt"), "keep\n").unwrap();
    let merge = git(root, &["merge", "--no-edit", "topic"]);
    assert_eq!(
        merge.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&merge.stderr)
    );
    assert!(root.join(".git/MERGE_HEAD").exists());
    assert!(root.join(".git/AUTO_MERGE").exists());
    d
}
fn worktree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| {
            (
                e.file_name().into_string().unwrap(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}
fn assert_equivalent(a: &Path, b: &Path) {
    assert_eq!(worktree(a), worktree(b), "worktree differs from Git CLI");
    assert_eq!(
        ok(a, &["ls-files", "--stage", "-z"]),
        ok(b, &["ls-files", "--stage", "-z"]),
        "index stages differ"
    );
    assert_eq!(
        ok(a, &["status", "--porcelain=v1", "-z"]),
        ok(b, &["status", "--porcelain=v1", "-z"])
    );
    for name in [
        "MERGE_HEAD",
        "MERGE_MSG",
        "MERGE_MODE",
        "AUTO_MERGE",
        "ORIG_HEAD",
    ] {
        assert_eq!(
            fs::read(a.join(".git").join(name)).ok(),
            fs::read(b.join(".git").join(name)).ok(),
            "metadata {name}"
        );
    }
}
#[test]
fn abort_matches_git_for_conflicts_resolutions_and_premerge_dirty_files() {
    for resolved in [false, true] {
        let app = fixture();
        let cli = fixture();
        if resolved {
            for root in [app.path(), cli.path()] {
                fs::write(root.join("file.txt"), "resolved\n").unwrap();
                ok(root, &["add", "file.txt"]);
            }
        }
        let mut session = Session::new(app.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        session
            .mutate(&s.revision, Action::AbortExternalMerge)
            .unwrap();
        ok(cli.path(), &["merge", "--abort"]);
        assert_equivalent(app.path(), cli.path());
        assert_eq!(
            ok(app.path(), &["rev-parse", "HEAD"]),
            ok(cli.path(), &["rev-parse", "HEAD"])
        );
    }
}
#[test]
fn abort_matches_git_refusal_for_new_unstaged_resolution_edit() {
    let app = fixture();
    let cli = fixture();
    for root in [app.path(), cli.path()] {
        fs::write(root.join("file.txt"), "resolved\n").unwrap();
        ok(root, &["add", "file.txt"]);
        fs::write(root.join("file.txt"), "later edit\n").unwrap();
    }
    let mut session = Session::new(app.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    assert!(session
        .mutate(&s.revision, Action::AbortExternalMerge)
        .unwrap_err()
        .contains("UNSTAGED"));
    assert!(!git(cli.path(), &["merge", "--abort"]).status.success());
    assert_equivalent(app.path(), cli.path());
}
#[test]
fn stage_and_continue_match_git_tree_parents_and_metadata() {
    let app = fixture();
    let cli = fixture();
    for root in [app.path(), cli.path()] {
        fs::write(root.join("file.txt"), "resolved\n").unwrap();
    }
    let mut session = Session::new(app.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    let s = session
        .mutate(
            &s.revision,
            Action::StageExternalResolution {
                file_id: file_id("file.txt"),
            },
        )
        .unwrap();
    ok(cli.path(), &["add", "file.txt"]);
    assert_eq!(
        ok(app.path(), &["ls-files", "--stage", "-z"]),
        ok(cli.path(), &["ls-files", "--stage", "-z"])
    );
    // Refresh after independent CLI inspection, whose stat-cache refresh may change index bytes.
    let _ = s;
    let s = session.snapshot().unwrap();
    session
        .mutate(
            &s.revision,
            Action::ContinueExternalMerge {
                message: "Resolved merge".into(),
                author: "Parity".into(),
                email: "parity@example.invalid".into(),
            },
        )
        .unwrap();
    ok(cli.path(), &["commit", "-m", "Resolved merge"]);
    assert_equivalent(app.path(), cli.path());
    assert_eq!(
        ok(
            app.path(),
            &["show", "-s", "--format=%T%n%P%n%an%n%ae%n%B", "HEAD"]
        ),
        ok(
            cli.path(),
            &["show", "-s", "--format=%T%n%P%n%an%n%ae%n%B", "HEAD"]
        )
    );
    let head = ok(app.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        ok(app.path(), &["reflog", "-1", "--format=%H", "HEAD"]),
        head,
        "HEAD reflog should point to completed merge"
    );
}
#[test]
fn deleted_and_binary_resolutions_match_git_stage_and_continue() {
    for bytes in [None, Some(b"resolved\0binary\xff".as_slice())] {
        let app = fixture();
        let cli = fixture();
        for root in [app.path(), cli.path()] {
            if let Some(bytes) = bytes {
                fs::write(root.join("file.txt"), bytes).unwrap();
            } else {
                fs::remove_file(root.join("file.txt")).unwrap();
            }
        }
        let mut session = Session::new(app.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let s = session
            .mutate(
                &s.revision,
                Action::StageExternalResolution {
                    file_id: file_id("file.txt"),
                },
            )
            .unwrap();
        ok(cli.path(), &["add", "-u", "file.txt"]);
        session
            .mutate(
                &s.revision,
                Action::ContinueExternalMerge {
                    message: "Resolved merge".into(),
                    author: "Parity".into(),
                    email: "parity@example.invalid".into(),
                },
            )
            .unwrap();
        ok(cli.path(), &["commit", "-m", "Resolved merge"]);
        assert_equivalent(app.path(), cli.path());
        assert_eq!(
            ok(app.path(), &["show", "-s", "--format=%T%n%P", "HEAD"]),
            ok(cli.path(), &["show", "-s", "--format=%T%n%P", "HEAD"])
        );
    }
}
fn rename_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    ok(root, &["init", "--template=", "-b", "main"]);
    fs::write(root.join("original.txt"), "shared content\n").unwrap();
    ok(root, &["add", "."]);
    ok(root, &["commit", "-m", "Base"]);
    ok(root, &["checkout", "-b", "topic"]);
    ok(root, &["mv", "original.txt", "incoming.txt"]);
    ok(root, &["commit", "-m", "Incoming rename"]);
    ok(root, &["checkout", "main"]);
    ok(root, &["mv", "original.txt", "current.txt"]);
    ok(root, &["commit", "-m", "Current rename"]);
    assert_eq!(
        git(root, &["merge", "--no-edit", "topic"]).status.code(),
        Some(1)
    );
    d
}
#[test]
fn rename_rename_conflict_abort_matches_git() {
    let app = rename_fixture();
    let cli = rename_fixture();
    let mut session = Session::new(app.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    assert!(!s.external_merge.as_ref().unwrap().conflicts.is_empty());
    session
        .mutate(&s.revision, Action::AbortExternalMerge)
        .unwrap();
    ok(cli.path(), &["merge", "--abort"]);
    assert_equivalent(app.path(), cli.path());
}
#[cfg(unix)]
#[test]
fn abort_restores_conflict_file_mode_like_git() {
    use std::os::unix::fs::PermissionsExt;
    let app = fixture();
    let cli = fixture();
    for root in [app.path(), cli.path()] {
        fs::set_permissions(root.join("file.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut session = Session::new(app.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    session
        .mutate(&s.revision, Action::AbortExternalMerge)
        .unwrap();
    ok(cli.path(), &["merge", "--abort"]);
    assert_equivalent(app.path(), cli.path());
    assert_eq!(
        fs::metadata(app.path().join("file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        fs::metadata(cli.path().join("file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    );
}

fn amend_fixture(kind: usize, staged: bool) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    ok(root, &["init", "--template=", "-b", "main"]);
    fs::write(root.join("file.txt"), "base\n").unwrap();
    ok(root, &["add", "."]);
    ok(root, &["commit", "-m", "Base"]);
    if kind == 2 {
        ok(root, &["checkout", "-b", "topic"]);
        fs::write(root.join("topic.txt"), "topic\n").unwrap();
        ok(root, &["add", "."]);
        ok(root, &["commit", "-m", "Topic"]);
        ok(root, &["checkout", "main"]);
    }
    if kind > 0 {
        fs::write(root.join("main.txt"), "main\n").unwrap();
        ok(root, &["add", "."]);
        ok(root, &["commit", "-m", "Main"]);
    }
    if kind == 2 {
        ok(root, &["merge", "--no-edit", "topic"]);
    }
    if kind == 3 {
        ok(root, &["checkout", "--detach", "HEAD"]);
    }
    if staged {
        fs::write(root.join("file.txt"), "staged\r\n").unwrap();
        ok(root, &["add", "file.txt"]);
    }
    fs::write(root.join("file.txt"), "unstaged later\n").unwrap();
    fs::write(root.join("untracked.txt"), "keep\n").unwrap();
    d
}
#[test]
fn amend_matches_git_message_author_parents_and_three_areas() {
    for kind in 0..=3 {
        for staged in [false, true] {
            for message in [
                "New summary",
                "\n\nSubject  \t\n\n\nBody\t \n# retain comment\n\n",
            ] {
                let app = amend_fixture(kind, staged);
                let cli = amend_fixture(kind, staged);
                let before = fs::read(app.path().join(".git/index")).unwrap();
                let mut session = Session::new(app.path().into()).unwrap();
                let s = session.snapshot().unwrap();
                session
                    .mutate(
                        &s.revision,
                        Action::AmendCommit {
                            message: message.into(),
                            author: "Parity".into(),
                            email: "parity@example.invalid".into(),
                        },
                    )
                    .unwrap();
                assert_eq!(fs::read(app.path().join(".git/index")).unwrap(), before);
                ok(cli.path(), &["commit", "--amend", "-m", message]);
                assert_equivalent(app.path(), cli.path());
                let args = [
                    "show",
                    "-s",
                    "--format=%T%n%P%n%an%n%ae%n%aI%n%cn%n%ce%n%B",
                    "HEAD",
                ];
                assert_eq!(
                    ok(app.path(), &args),
                    ok(cli.path(), &args),
                    "amend kind={kind} staged={staged} message={message:?}"
                );
                let a = open(app.path()).unwrap();
                let b = open(cli.path()).unwrap();
                assert_eq!(a.head_detached().unwrap(), b.head_detached().unwrap());
                assert_eq!(
                    a.reflog("HEAD").unwrap().get(0).unwrap().id_new(),
                    a.head().unwrap().target().unwrap()
                );
            }
        }
    }
}
#[test]
fn initial_and_ordinary_commit_message_matches_git_without_editor() {
    for initial in [false, true] {
        let app = amend_fixture(0, true);
        let cli = amend_fixture(0, true);
        if initial {
            for root in [app.path(), cli.path()] {
                ok(root, &["update-ref", "-d", "refs/heads/main"]);
            }
        }
        let message = "\nSubject\t \n\n\nBody\r\n# retained\n\n";
        let mut session = Session::new(app.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        session
            .mutate(
                &s.revision,
                Action::Commit {
                    message: message.into(),
                    author: "Parity".into(),
                    email: "parity@example.invalid".into(),
                },
            )
            .unwrap();
        ok(cli.path(), &["commit", "-m", message]);
        assert_equivalent(app.path(), cli.path());
        let args = [
            "show",
            "-s",
            "--format=%T%n%P%n%an%n%ae%n%cn%n%ce%n%B",
            "HEAD",
        ];
        assert_eq!(ok(app.path(), &args), ok(cli.path(), &args));
    }
}

#[test]
fn branch_rename_matches_git_refs_tracking_and_reflog_messages() {
    for logging in [false, true] {
        for current in [false, true] {
            for packed in [false, true] {
                let app = tempfile::tempdir().unwrap();
                let oracle = tempfile::tempdir().unwrap();
                for root in [app.path(), oracle.path()] {
                    ok(root, &["init", "--template=", "-b", "main"]);
                    if !logging {
                        ok(root, &["config", "core.logallrefupdates", "false"]);
                    }
                    fs::write(root.join("file.txt"), "base\n").unwrap();
                    ok(root, &["add", "."]);
                    ok(root, &["commit", "-m", "initial"]);
                    ok(root, &["branch", "topic"]);
                    ok(root, &["config", "branch.topic.remote", "origin"]);
                    ok(
                        root,
                        &["config", "--add", "branch.topic.merge", "refs/heads/main"],
                    );
                    ok(
                        root,
                        &["config", "--add", "branch.topic.merge", "refs/heads/extra"],
                    );
                    if current {
                        ok(root, &["checkout", "topic"]);
                    }
                    if packed {
                        ok(root, &["pack-refs", "--all"]);
                    }
                    fs::write(root.join("file.txt"), "dirty worktree\n").unwrap();
                }
                ok(oracle.path(), &["branch", "-m", "topic", "renamed"]);
                let mut session = Session::new(app.path().into()).unwrap();
                let snap = session.snapshot().unwrap();
                session
                    .mutate(
                        &snap.revision,
                        Action::RenameBranch {
                            name: "topic".into(),
                            new_name: "renamed".into(),
                        },
                    )
                    .unwrap();
                for args in [
                    vec!["show-ref", "--heads"],
                    vec!["symbolic-ref", "HEAD"],
                    vec!["config", "--get-regexp", "^branch\\."],
                    vec!["status", "--porcelain"],
                    vec!["ls-files", "--stage"],
                    vec!["reflog", "show", "--format=%H %gs", "renamed"],
                    vec!["reflog", "show", "--format=%H %gs", "HEAD"],
                ] {
                    assert_eq!(
                        ok(app.path(), &args),
                        ok(oracle.path(), &args),
                        "{args:?}, current={current},packed={packed}"
                    );
                }
                assert_eq!(worktree(app.path()), worktree(oracle.path()));
            }
        }
    }
}
#[test]
fn detached_checkout_matches_git_head_index_files_and_branch_refs() {
    fn setup() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        ok(root, &["init", "--template=", "-b", "main"]);
        fs::write(root.join("one.txt"), "base\n").unwrap();
        ok(root, &["add", "."]);
        ok(root, &["commit", "-m", "Base"]);
        ok(root, &["checkout", "-b", "topic"]);
        fs::write(root.join("one.txt"), "topic\n").unwrap();
        fs::write(root.join("added.txt"), "added\n").unwrap();
        ok(root, &["add", "."]);
        ok(root, &["commit", "-m", "Topic"]);
        ok(root, &["checkout", "main"]);
        d
    }
    for initially_detached in [false, true] {
        let app = setup();
        let cli = setup();
        if initially_detached {
            ok(app.path(), &["checkout", "--detach", "HEAD"]);
            ok(cli.path(), &["checkout", "--detach", "HEAD"]);
        }
        let mut session = Session::new(app.path().into()).unwrap();
        for (detach, name) in [
            (true, "topic"),
            (true, "main"),
            (false, "topic"),
            (false, "main"),
        ] {
            let snapshot = session.snapshot().unwrap();
            if detach {
                let oid = String::from_utf8(ok(cli.path(), &["rev-parse", name]))
                    .unwrap()
                    .trim()
                    .to_string();
                session
                    .mutate(&snapshot.revision, Action::CheckoutCommit { oid })
                    .unwrap();
                ok(cli.path(), &["checkout", "--detach", name]);
            } else {
                session
                    .mutate(&snapshot.revision, Action::Checkout { name: name.into() })
                    .unwrap();
                ok(cli.path(), &["checkout", name]);
            }
            assert_equivalent(app.path(), cli.path());
            assert_eq!(
                fs::read(app.path().join(".git/HEAD")).unwrap(),
                fs::read(cli.path().join(".git/HEAD")).unwrap()
            );
            assert_eq!(
                ok(
                    app.path(),
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname)",
                        "refs/heads/"
                    ]
                ),
                ok(
                    cli.path(),
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname)",
                        "refs/heads/"
                    ]
                )
            );
        }
    }
}
#[test]
fn create_branch_at_matches_git_without_changing_checkout_or_dirty_files() {
    for detached in [false, true] {
        let app = amend_fixture(1, true);
        let cli = amend_fixture(1, true);
        if detached {
            ok(app.path(), &["checkout", "--detach", "HEAD"]);
            ok(cli.path(), &["checkout", "--detach", "HEAD"]);
        }
        let target = String::from_utf8(ok(cli.path(), &["rev-parse", "HEAD^"]))
            .unwrap()
            .trim()
            .to_string();
        let mut session = Session::new(app.path().into()).unwrap();
        let before = session.snapshot().unwrap();
        session
            .mutate(
                &before.revision,
                Action::CreateBranchAt {
                    name: "selected/base".into(),
                    oid: target.clone(),
                },
            )
            .unwrap();
        ok(cli.path(), &["branch", "selected/base", &target]);
        assert_equivalent(app.path(), cli.path());
        assert_eq!(
            fs::read(app.path().join(".git/HEAD")).unwrap(),
            fs::read(cli.path().join(".git/HEAD")).unwrap()
        );
        assert_eq!(
            ok(
                app.path(),
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/heads/"
                ]
            ),
            ok(
                cli.path(),
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/heads/"
                ]
            )
        );
        let a = Repository::open(app.path()).unwrap();
        let b = Repository::open(cli.path()).unwrap();
        assert_eq!(
            a.reflog("refs/heads/selected/base")
                .unwrap()
                .get(0)
                .unwrap()
                .message_bytes(),
            b.reflog("refs/heads/selected/base")
                .unwrap()
                .get(0)
                .unwrap()
                .message_bytes()
        );
    }
}
#[test]
fn comparison_matches_git_direct_trees_including_merge_second_parent() {
    let d = amend_fixture(2, true);
    let mut session = Session::new(d.path().into()).unwrap();
    let snapshot = session.snapshot().unwrap();
    let head = fs::read(d.path().join(".git/HEAD")).unwrap();
    let index = fs::read(d.path().join(".git/index")).unwrap();
    let work = worktree(d.path());
    for (before, after) in [
        ("main", "topic"),
        ("topic", "main"),
        ("main^", "main"),
        ("main", "main"),
    ] {
        let base = String::from_utf8(ok(d.path(), &["rev-parse", before]))
            .unwrap()
            .trim()
            .to_string();
        let target = String::from_utf8(ok(d.path(), &["rev-parse", after]))
            .unwrap()
            .trim()
            .to_string();
        let comparison =
            compare::read(&mut session, &snapshot.revision, &base, &target, None).unwrap();
        let names = String::from_utf8(ok(
            d.path(),
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--name-only",
                &base,
                &target,
            ],
        ))
        .unwrap();
        assert_eq!(
            comparison
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            names.lines().collect::<Vec<_>>()
        );
        for file in comparison.files {
            let result = compare::read(
                &mut session,
                &snapshot.revision,
                &base,
                &target,
                Some(&file.id),
            )
            .unwrap();
            let patch = String::from_utf8(ok(
                d.path(),
                &[
                    "--literal-pathspecs",
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--no-color",
                    &base,
                    &target,
                    "--",
                    &file.path,
                ],
            ))
            .unwrap();
            assert_eq!(result.diff.unwrap().text, patch);
        }
    }
    assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
    assert_eq!(worktree(d.path()), work);
}
