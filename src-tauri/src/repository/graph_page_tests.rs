use super::*;
fn history(count: usize) -> (tempfile::TempDir, Vec<String>) {
    let d = super::tests::fixture();
    let repo = Repository::open(d.path()).unwrap();
    let mut parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = parent.tree().unwrap();
    let sig = git2::Signature::now("Pager", "pager@example.invalid").unwrap();
    let mut ids = vec![parent.id().to_string()];
    for n in 1..count {
        let oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                &format!("Commit {n}"),
                &tree,
                &[&parent],
            )
            .unwrap();
        ids.push(oid.to_string());
        parent = repo.find_commit(oid).unwrap();
    }
    (d, ids)
}
#[test]
fn graph_pages_cover_all_history_with_no_duplicates_and_gate_old_page_requests() {
    let (d, ids) = history(1201);
    let head = fs::read(d.path().join(".git/HEAD")).unwrap();
    let index = fs::read(d.path().join(".git/index")).unwrap();
    let mut session = Session::new(d.path().into()).unwrap();
    let first = session.snapshot().unwrap();
    assert_eq!(first.commits.len(), 500);
    assert_eq!(first.graph_offset, 0);
    assert!(first.has_more);
    let second = session
        .graph_page(&first.revision, GraphPageDirection::Older)
        .unwrap();
    assert_eq!(second.graph_offset, 500);
    assert_ne!(second.revision, first.revision);
    assert!(session
        .graph_page(&first.revision, GraphPageDirection::Older)
        .unwrap_err()
        .contains("STALE"));
    assert!(session
        .commit_detail(&second.revision, &first.commits[1].oid, None)
        .is_err());
    let current_detail = session
        .commit_detail(&second.revision, &first.commits[0].oid, None)
        .unwrap();
    assert_eq!(current_detail.oid, first.commits[0].oid);
    let comparison = compare::read(
        &mut session,
        &second.revision,
        &second.commits[0].oid,
        &second.commits[1].oid,
        None,
    )
    .unwrap();
    assert!(comparison.files.is_empty());
    let third = session
        .graph_page(&second.revision, GraphPageDirection::Older)
        .unwrap();
    assert_eq!(third.graph_offset, 1000);
    assert_eq!(third.commits.len(), 201);
    assert!(!third.has_more);
    let all: Vec<_> = first
        .commits
        .iter()
        .chain(&second.commits)
        .chain(&third.commits)
        .map(|c| c.oid.clone())
        .collect();
    assert_eq!(all, ids.iter().rev().cloned().collect::<Vec<_>>());
    let output = std::process::Command::new("git")
        .args([
            "-C",
            d.path().to_str().unwrap(),
            "rev-list",
            "--topo-order",
            "HEAD",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        all.iter().map(String::as_str).collect::<Vec<_>>()
    );
    let end = session
        .graph_page(&third.revision, GraphPageDirection::Older)
        .unwrap();
    assert_eq!(end.revision, third.revision);
    let newer = session
        .graph_page(&end.revision, GraphPageDirection::Newer)
        .unwrap();
    assert_eq!(newer.graph_offset, 500);
    assert_eq!(newer.commits[0].oid, second.commits[0].oid);
    let selected = newer.commits[100].oid.clone();
    let created = session
        .mutate(
            &newer.revision,
            Action::CreateBranchAt {
                name: "older/commit".into(),
                oid: selected.clone(),
            },
        )
        .unwrap();
    assert_eq!(created.graph_offset, 500);
    assert_eq!(
        Repository::open(d.path())
            .unwrap()
            .refname_to_id("refs/heads/older/commit")
            .unwrap()
            .to_string(),
        selected
    );
    let latest = session
        .graph_page(&created.revision, GraphPageDirection::Latest)
        .unwrap();
    assert_eq!(latest.graph_offset, 0);
    assert_eq!(latest.commits[0].oid, first.commits[0].oid);
    assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
        "hello\n"
    );
}
#[test]
fn graph_pages_are_session_local_and_reset_when_history_shrinks() {
    let (d, ids) = history(601);
    let mut session = Session::new(d.path().into()).unwrap();
    let first = session.snapshot().unwrap();
    let second = session
        .graph_page(&first.revision, GraphPageDirection::Older)
        .unwrap();
    assert_eq!(second.graph_offset, 500);
    let mut other = Session::new(d.path().into()).unwrap();
    assert_eq!(other.snapshot().unwrap().graph_offset, 0);
    let repo = Repository::open(d.path()).unwrap();
    let name = repo.head().unwrap().name().unwrap().to_string();
    repo.reference(
        &name,
        Oid::from_str(&ids[99]).unwrap(),
        true,
        "fixture rewind",
    )
    .unwrap();
    assert!(session
        .graph_page(&second.revision, GraphPageDirection::Newer)
        .unwrap_err()
        .contains("STALE"));
    let fresh = session.snapshot().unwrap();
    assert_eq!(fresh.graph_offset, 0);
    assert_eq!(fresh.commits.len(), 100);
    assert!(!fresh.has_more);
}
#[test]
fn unborn_head_still_exposes_existing_branch_history() {
    let (d, ids) = history(3);
    Repository::open(d.path())
        .unwrap()
        .set_head("refs/heads/new-unborn")
        .unwrap();
    let mut session = Session::new(d.path().into()).unwrap();
    let s = session.snapshot().unwrap();
    assert!(s.head_oid.is_none());
    assert_eq!(s.branch, "new-unborn");
    assert_eq!(s.commits.len(), 3);
    assert_eq!(s.commits[0].oid, ids[2]);
}
