//! Cherry-pick/revert analysis. Generated objects live only in a private ODB.
//! Clean and resolved results are journaled through checkout.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    CherryPick,
    Revert,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: String,
    pub head: String,
    pub target: String,
    pub parent: Option<String>,
    pub kind: Kind,
    pub result_tree: Option<String>,
    pub conflicts: Vec<merge::MergeConflict>,
    pub blockers: Vec<String>,
    pub empty: bool,
    pub originally_empty: bool,
    pub resolved_empty: Option<bool>,
    pub diff: DiffResult,
}
pub(super) fn preview(
    session: &mut Session,
    revision: &str,
    oid: &str,
    kind: Kind,
    mainline: Option<usize>,
) -> Result<Plan> {
    Ok(calculate(session, revision, oid, kind, mainline)?.0)
}
pub(super) fn calculate(
    session: &mut Session,
    revision: &str,
    oid: &str,
    kind: Kind,
    mainline: Option<usize>,
) -> Result<(Plan, Repository, tempfile::TempDir)> {
    calculate_resolved(session, revision, oid, kind, mainline, None)
}
pub(super) fn calculate_resolved(
    session: &mut Session,
    revision: &str,
    oid: &str,
    kind: Kind,
    mainline: Option<usize>,
    resolutions: Option<&HashMap<String, merge_draft::Resolution>>,
) -> Result<(Plan, Repository, tempfile::TempDir)> {
    let snap = session.fresh(revision)?;
    if !snap.commits.iter().any(|c| c.oid == oid)
        && !sequence_state::authorizes_current(&session.root, &snap, oid, kind, mainline)?
    {
        return Err("INVALID_COMMIT".into());
    }
    let repo = open(&session.root)?;
    let head = repo.head().map_err(err)?;
    let ours = head.peel_to_commit().map_err(err)?;
    let target = repo
        .find_commit(Oid::from_str(oid).map_err(err)?)
        .map_err(err)?;
    let parent = match (target.parent_count(), mainline) {
        (0, None) => None,
        (1, None) => Some(target.parent_id(0).map_err(err)?),
        (n, Some(index)) if index > 0 && index <= n => {
            Some(target.parent_id(index - 1).map_err(err)?)
        }
        (n, None) if n > 1 => return Err("MAINLINE_REQUIRED".into()),
        _ => return Err("INVALID_MAINLINE".into()),
    };
    for id in [Some(ours.id()), Some(target.id()), parent]
        .into_iter()
        .flatten()
    {
        checkout::inspect_tree(
            &repo,
            &repo.find_commit(id).map_err(err)?.tree().map_err(err)?,
        )?;
    }
    let temp = tempfile::tempdir().map_err(err)?;
    let mut init = git2::RepositoryInitOptions::new();
    init.bare(true).external_template(false);
    let scratch = Repository::init_opts(temp.path(), &init).map_err(err)?;
    scratch
        .set_config(&Config::new().map_err(err)?)
        .map_err(err)?;
    scratch
        .odb()
        .map_err(err)?
        .add_disk_alternate(
            repo.path()
                .join("objects")
                .to_str()
                .ok_or("UNSUPPORTED_PATH")?,
        )
        .map_err(err)?;
    let current = scratch
        .find_commit(ours.id())
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let selected = scratch
        .find_commit(target.id())
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let previous = match parent {
        Some(id) => scratch.find_commit(id).map_err(err)?.tree().map_err(err)?,
        None => {
            let id = scratch
                .treebuilder(None)
                .map_err(err)?
                .write()
                .map_err(err)?;
            scratch.find_tree(id).map_err(err)?
        }
    };
    let (base, incoming) = match kind {
        Kind::CherryPick => (&previous, &selected),
        Kind::Revert => (&selected, &previous),
    };
    let mut index = scratch
        .merge_trees(base, &current, incoming, None)
        .map_err(err)?;
    let mut conflicts = Vec::new();
    for conflict in index.conflicts().map_err(err)? {
        if conflicts.len() >= MAX_FILES {
            return Err("LIMIT: replay conflicts".into());
        }
        let c = conflict.map_err(err)?;
        conflicts.push(merge::MergeConflict {
            ancestor: merge::version(c.ancestor)?,
            ours: merge::version(c.our)?,
            theirs: merge::version(c.their)?,
        });
    }
    if let Some(resolutions) = resolutions {
        merge_draft::resolve_index(&scratch, &mut index, resolutions)?;
    }
    let unresolved = index.has_conflicts();
    let result = if !unresolved {
        Some(index.write_tree_to(&scratch).map_err(err)?)
    } else {
        None
    };
    if let Some(id) = result {
        checkout::inspect_tree(&scratch, &scratch.find_tree(id).map_err(err)?)?;
    }
    let mut options = DiffOptions::new();
    options.max_size(FILE_LIMIT as i64);
    let diff = scratch
        .diff_tree_to_index(Some(&current), Some(&index), Some(&mut options))
        .map_err(err)?;
    if diff.deltas().len() > MAX_FILES {
        return Err("LIMIT: replay files".into());
    }
    let mut blockers = Vec::new();
    if let Some(block) = snap.write_block {
        blockers.push(block);
    }
    if !snap.changes.is_empty() {
        blockers.push("DIRTY_WORKTREE".into());
    }
    if !head.is_branch() {
        blockers.push("DETACHED_HEAD".into());
    }
    if repo.state() != git2::RepositoryState::Clean {
        blockers.push("OPERATION_IN_PROGRESS".into());
    }
    if unresolved {
        blockers.push("CONFLICTS".into());
    }
    let plan = Plan {
        revision: revision.into(),
        head: ours.id().to_string(),
        target: oid.into(),
        parent: parent.map(|id| id.to_string()),
        kind,
        result_tree: result.map(|id| id.to_string()),
        empty: result == Some(current.id()),
        originally_empty: selected.id() == previous.id(),
        resolved_empty: None,
        conflicts,
        blockers,
        diff: render_diff(&diff)?,
    };
    session.fresh(revision)?;
    drop(diff);
    drop(index);
    drop(previous);
    drop(selected);
    drop(current);
    Ok((plan, scratch, temp))
}

// Preparation never writes to the source ODB.
pub(super) struct Prepared {
    pub oid: Oid,
    pub head: Oid,
    pub objects: Vec<(Oid, git2::ObjectType, Vec<u8>)>,
}
#[cfg(test)]
pub(super) fn prepare_commit(
    session: &mut Session,
    revision: &str,
    oid: &str,
    kind: Kind,
    mainline: Option<usize>,
    identity: (&str, &str, &str),
) -> Result<Prepared> {
    prepare_resolved(
        session,
        revision,
        oid,
        kind,
        mainline,
        identity,
        PrepareOptions {
            resolutions: None,
            allow_empty: false,
            committer_time: None,
        },
    )
}
#[derive(Default)]
pub(super) struct PrepareOptions<'a> {
    pub resolutions: Option<&'a HashMap<String, merge_draft::Resolution>>,
    pub allow_empty: bool,
    pub committer_time: Option<git2::Time>,
}
pub(super) fn prepare_resolved(
    session: &mut Session,
    revision: &str,
    oid: &str,
    kind: Kind,
    mainline: Option<usize>,
    identity: (&str, &str, &str),
    options: PrepareOptions<'_>,
) -> Result<Prepared> {
    let (message, name, email) = identity;
    if message.trim().is_empty()
        || message.len() > 10000
        || message.contains('\0')
        || name.trim().is_empty()
        || name.len() > 256
        || email.trim().is_empty()
        || email.len() > 256
    {
        return Err("INVALID_REPLAY_IDENTITY_OR_MESSAGE".into());
    }
    let committer = if let Some(time) = options.committer_time {
        if time.offset_minutes().unsigned_abs() > 1439 {
            return Err("INVALID_REPLAY_TIME".into());
        }
        git2::Signature::new(name, email, &time).map_err(err)?
    } else {
        git2::Signature::now(name, email).map_err(err)?
    };
    let (plan, scratch, _temporary) =
        calculate_resolved(session, revision, oid, kind, mainline, options.resolutions)?;
    if !plan.blockers.is_empty() {
        return Err(format!("REPLAY_BLOCKED: {}", plan.blockers.join(", ")));
    }
    if plan.empty && !options.allow_empty {
        return Err("EMPTY_REPLAY".into());
    }
    let current = scratch
        .find_commit(Oid::from_str(&plan.head).map_err(err)?)
        .map_err(err)?;
    let target = scratch
        .find_commit(Oid::from_str(oid).map_err(err)?)
        .map_err(err)?;
    let original_author = target.author();
    let author = if kind == Kind::CherryPick {
        &original_author
    } else {
        &committer
    };
    let tree = scratch
        .find_tree(
            Oid::from_str(plan.result_tree.as_deref().ok_or("UNRESOLVED_REPLAY")?).map_err(err)?,
        )
        .map_err(err)?;
    let message = git2::message_prettify(message, None).map_err(err)?;
    if message.trim().is_empty() {
        return Err("INVALID_REPLAY_MESSAGE".into());
    }
    let id = scratch
        .commit(None, author, &committer, &message, &tree, &[&current])
        .map_err(err)?;
    let source_repo = open(&session.root)?;
    let source = source_repo.odb().map_err(err)?;
    let private = scratch.odb().map_err(err)?;
    let mut pending = vec![id, tree.id()];
    let mut seen = std::collections::HashSet::new();
    let mut objects = Vec::new();
    let mut bytes = 0usize;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) || source.exists(id) {
            continue;
        }
        if seen.len() > 100_000 {
            return Err("LIMIT: replay objects".into());
        }
        let object = private.read(id).map_err(err)?;
        bytes = bytes
            .checked_add(object.data().len())
            .ok_or("LIMIT: replay bytes")?;
        if bytes > 50 * 1024 * 1024 {
            return Err("LIMIT: replay bytes".into());
        }
        if object.kind() == git2::ObjectType::Tree {
            let tree = scratch.find_tree(id).map_err(err)?;
            for entry in tree.iter() {
                pending.push(entry.id());
            }
        }
        objects.push((id, object.kind(), object.data().to_vec()));
    }
    session.fresh(revision)?;
    Ok(Prepared {
        oid: id,
        head: current.id(),
        objects,
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub kind: Kind,
    pub source: String,
    pub mainline: Option<usize>,
    #[serde(default)]
    pub resolutions: Option<HashMap<String, merge_draft::Resolution>>,
}
pub(super) fn verify_commit(repo: &Repository, old: Oid, new: Oid, record: &Record) -> Result<()> {
    let commit = repo.find_commit(new).map_err(err)?;
    if commit.parent_count() != 1 || commit.parent_id(0).map_err(err)? != old {
        return Err("INVALID_REPLAY_PARENTS".into());
    }
    let selected = repo
        .find_commit(Oid::from_str(&record.source).map_err(err)?)
        .map_err(err)?;
    let parent = match (selected.parent_count(), record.mainline) {
        (0, None) => None,
        (1, None) => Some(selected.parent_id(0).map_err(err)?),
        (n, Some(i)) if i > 0 && i <= n => Some(selected.parent_id(i - 1).map_err(err)?),
        _ => return Err("INVALID_REPLAY_MAINLINE".into()),
    };
    let temp = tempfile::tempdir().map_err(err)?;
    let mut init = git2::RepositoryInitOptions::new();
    init.bare(true).external_template(false);
    let scratch = Repository::init_opts(temp.path(), &init).map_err(err)?;
    scratch
        .set_config(&Config::new().map_err(err)?)
        .map_err(err)?;
    scratch
        .odb()
        .map_err(err)?
        .add_disk_alternate(
            repo.path()
                .join("objects")
                .to_str()
                .ok_or("UNSUPPORTED_PATH")?,
        )
        .map_err(err)?;
    let ours = scratch.find_commit(old).map_err(err)?.tree().map_err(err)?;
    let target = scratch
        .find_commit(selected.id())
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let previous = match parent {
        Some(id) => scratch.find_commit(id).map_err(err)?.tree().map_err(err)?,
        None => {
            let id = scratch
                .treebuilder(None)
                .map_err(err)?
                .write()
                .map_err(err)?;
            scratch.find_tree(id).map_err(err)?
        }
    };
    for tree in [&ours, &target, &previous] {
        checkout::inspect_tree(&scratch, tree)?;
    }
    let (base, incoming) = if record.kind == Kind::CherryPick {
        (&previous, &target)
    } else {
        (&target, &previous)
    };
    let mut index = scratch
        .merge_trees(base, &ours, incoming, None)
        .map_err(err)?;
    if let Some(resolutions) = &record.resolutions {
        merge_draft::resolve_index(&scratch, &mut index, resolutions)?;
    }
    if index.has_conflicts() || index.write_tree_to(&scratch).map_err(err)? != commit.tree_id() {
        return Err("INVALID_REPLAY_TREE".into());
    }
    if record.kind == Kind::CherryPick {
        let a = commit.author();
        let b = selected.author();
        if a.name_bytes() != b.name_bytes()
            || a.email_bytes() != b.email_bytes()
            || a.when() != b.when()
        {
            return Err("INVALID_REPLAY_AUTHOR".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make(repo: &Repository, reference: &str, files: &[(&str, &str)], parents: &[Oid]) -> Oid {
        let mut tree = repo.treebuilder(None).unwrap();
        for (name, contents) in files {
            tree.insert(*name, repo.blob(contents.as_bytes()).unwrap(), 0o100644)
                .unwrap();
        }
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|id| repo.find_commit(*id).unwrap())
            .collect();
        let sig = git2::Signature::now("Replay Test", "replay@example.invalid").unwrap();
        repo.commit(
            Some(reference),
            &sig,
            &sig,
            "Selected commit",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn checkout(repo: &Repository) {
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }
    fn fingerprint(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(root, &entry.path(), out);
                } else {
                    out.insert(
                        entry.path().strip_prefix(root).unwrap().into(),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        visit(root, root, &mut out);
        out
    }
    fn git(root: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap()
    }
    #[test]
    fn replay_conflict_content_is_scoped_and_read_only() {
        use super::super::content::{self, ContentSource, MergeSide};
        for kind in [Kind::CherryPick, Kind::Revert] {
            for binary in [false, true] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base_text = if binary { "base\0" } else { "base\n" };
                let target_text = if binary { "target\0" } else { "target\n" };
                let current_text = if binary { "current\0" } else { "current\n" };
                let base = make(&repo, "refs/heads/main", &[("file.txt", base_text)], &[]);
                let target = make(
                    &repo,
                    "refs/heads/topic",
                    &[("file.txt", target_text)],
                    &[base],
                );
                make(
                    &repo,
                    "refs/heads/main",
                    &[("file.txt", current_text)],
                    &[base],
                );
                checkout(&repo);
                let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
                let snap = session.snapshot().unwrap();
                let before = fingerprint(d.path());
                let id = file_id("file.txt");
                let source = |side, conflict| ContentSource::ReplayConflict {
                    oid: target.to_string(),
                    operation: kind,
                    mainline: None,
                    conflict,
                    side,
                };
                for (side, expected, label) in [
                    (
                        MergeSide::Ancestor,
                        if kind == Kind::CherryPick {
                            base_text
                        } else {
                            target_text
                        },
                        "Base",
                    ),
                    (MergeSide::Ours, current_text, "Current"),
                    (
                        MergeSide::Theirs,
                        if kind == Kind::CherryPick {
                            target_text
                        } else {
                            base_text
                        },
                        "Incoming",
                    ),
                ] {
                    let result =
                        content::read(&mut session, &snap.revision, &id, source(side, 0)).unwrap();
                    assert_eq!(result.state, if binary { "binary" } else { "text" });
                    assert_eq!(result.text, if binary { "" } else { expected });
                    assert!(result.source.contains(label));
                    assert_eq!(result.size, expected.len() as u64);
                }
                assert_eq!(
                    content::read(
                        &mut session,
                        &snap.revision,
                        "forged",
                        source(MergeSide::Ours, 0)
                    )
                    .unwrap_err(),
                    "INVALID_FILE_ID"
                );
                assert_eq!(
                    content::read(
                        &mut session,
                        &snap.revision,
                        &id,
                        source(MergeSide::Ours, 1)
                    )
                    .unwrap_err(),
                    "INVALID_REPLAY_CONFLICT"
                );
                assert_eq!(
                    content::read(
                        &mut session,
                        &snap.revision,
                        &id,
                        source(MergeSide::Ours, MAX_FILES)
                    )
                    .unwrap_err(),
                    "INVALID_REPLAY_SELECTION"
                );
                assert!(content::read(
                    &mut session,
                    &snap.revision,
                    &id,
                    ContentSource::ReplayConflict {
                        oid: "HEAD".into(),
                        operation: kind,
                        mainline: None,
                        conflict: 0,
                        side: MergeSide::Ours
                    }
                )
                .is_err());
                assert_eq!(fingerprint(d.path()), before);
                fs::write(d.path().join("file.txt"), "external change").unwrap();
                assert!(content::read(
                    &mut session,
                    &snap.revision,
                    &id,
                    source(MergeSide::Ours, 0)
                )
                .unwrap_err()
                .starts_with("STALE"));
                assert_eq!(
                    fs::read_to_string(d.path().join("file.txt")).unwrap(),
                    "external change"
                );
            }
        }
    }

    #[test]
    fn replay_content_refuses_absent_versions_and_truncates_large_text() {
        use super::super::content::{self, ContentSource, MergeSide};
        for kind in [Kind::CherryPick, Kind::Revert] {
            let d = tempfile::tempdir().unwrap();
            let repo = Repository::init(d.path()).unwrap();
            repo.set_head("refs/heads/main").unwrap();
            let base = make(&repo, "refs/heads/main", &[("file.txt", "base")], &[]);
            let large = "x".repeat(600 * 1024);
            let target = make(&repo, "refs/heads/topic", &[("file.txt", &large)], &[base]);
            make(&repo, "refs/heads/main", &[], &[base]);
            checkout(&repo);
            let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = fingerprint(d.path());
            let source = |side| ContentSource::ReplayConflict {
                oid: target.to_string(),
                operation: kind,
                mainline: None,
                conflict: 0,
                side,
            };
            let id = file_id("file.txt");
            assert_eq!(
                content::read(&mut session, &snap.revision, &id, source(MergeSide::Ours))
                    .unwrap_err(),
                "ABSENT_REPLAY_VERSION"
            );
            let side = if kind == Kind::CherryPick {
                MergeSide::Theirs
            } else {
                MergeSide::Ancestor
            };
            let result = content::read(&mut session, &snap.revision, &id, source(side)).unwrap();
            assert_eq!(result.size, large.len() as u64);
            assert_eq!(result.text.len(), 512 * 1024);
            assert!(result.truncated);
            assert_eq!(fingerprint(d.path()), before);
        }
    }

    #[test]
    fn replay_preview_matches_git_clean_conflict_root_and_empty_without_source_writes() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for case in ["clean", "conflict", "root", "empty"] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = make(
                    &repo,
                    "refs/heads/main",
                    &[("file.txt", "one\nshared\nthree\n")],
                    &[],
                );
                let target = if case == "root" {
                    base
                } else {
                    make(
                        &repo,
                        "refs/heads/topic",
                        &[("file.txt", "ONE\nshared\nthree\n")],
                        &[base],
                    )
                };
                let contents = if case == "root" {
                    if kind == Kind::CherryPick {
                        vec![("other.txt", "other\n")]
                    } else {
                        vec![("file.txt", "one\nshared\nthree\n")]
                    }
                } else if case == "conflict" {
                    vec![("file.txt", "CONFLICT\nshared\nthree\n")]
                } else if (kind == Kind::CherryPick) == (case == "empty") {
                    vec![("file.txt", "ONE\nshared\nthree\n")]
                } else {
                    vec![("file.txt", "one\nshared\nthree\n")]
                };
                let head = make(&repo, "refs/heads/main", &contents, &[base]);
                checkout(&repo);
                let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
                let snap = session.snapshot().unwrap();
                let before = fingerprint(d.path());
                let plan = preview(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    kind,
                    None,
                )
                .unwrap();
                assert_eq!(
                    fingerprint(d.path()),
                    before,
                    "preview wrote source {kind:?}/{case}"
                );
                assert_eq!(plan.revision, snap.revision);
                assert_eq!(plan.head, head.to_string());
                assert_eq!(plan.target, target.to_string());
                assert_eq!(plan.kind, kind);
                assert_eq!(
                    plan.parent,
                    if case == "root" {
                        None
                    } else {
                        Some(base.to_string())
                    }
                );
                let command = if kind == Kind::CherryPick {
                    "cherry-pick"
                } else {
                    "revert"
                };
                let applied = git(d.path(), &[command, "--no-commit", &target.to_string()]);
                if case == "conflict" {
                    assert!(!applied.status.success());
                    assert!(!plan.conflicts.is_empty());
                    assert!(plan.blockers.iter().any(|b| b == "CONFLICTS"));
                    let mut actual = repo.index().unwrap();
                    actual.read(true).unwrap();
                    let c = actual.conflicts().unwrap().next().unwrap().unwrap();
                    assert_eq!(
                        plan.conflicts[0].ancestor.as_ref().map(|c| c.oid.clone()),
                        c.ancestor.map(|e| e.id.to_string())
                    );
                    assert_eq!(
                        plan.conflicts[0].ours.as_ref().map(|c| c.oid.clone()),
                        c.our.map(|e| e.id.to_string())
                    );
                    assert_eq!(
                        plan.conflicts[0].theirs.as_ref().map(|c| c.oid.clone()),
                        c.their.map(|e| e.id.to_string())
                    );
                } else {
                    assert!(
                        applied.status.success(),
                        "{}",
                        String::from_utf8_lossy(&applied.stderr)
                    );
                    let tree = git(d.path(), &["write-tree"]);
                    assert!(tree.status.success());
                    assert_eq!(
                        plan.result_tree.as_deref(),
                        Some(String::from_utf8_lossy(&tree.stdout).trim())
                    );
                    assert_eq!(plan.empty, case == "empty");
                    assert!(plan.blockers.is_empty());
                    assert!(!plan.diff.truncated);
                }
            }
        }
    }
    #[test]
    fn merge_mainline_matches_git_and_invalid_selection_is_read_only() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for mainline in [1, 2] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = make(&repo, "refs/heads/main", &[("file.txt", "base\n")], &[]);
                let p1 = make(&repo, "refs/heads/one", &[("file.txt", "one\n")], &[base]);
                let p2 = make(&repo, "refs/heads/two", &[("file.txt", "two\n")], &[base]);
                let target = make(
                    &repo,
                    "refs/heads/merged",
                    &[("file.txt", "merged\n")],
                    &[p1, p2],
                );
                let content = if kind == Kind::Revert {
                    "merged\n"
                } else if mainline == 1 {
                    "one\n"
                } else {
                    "two\n"
                };
                make(&repo, "refs/heads/main", &[("file.txt", content)], &[base]);
                checkout(&repo);
                let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
                let snap = session.snapshot().unwrap();
                let before = fingerprint(d.path());
                for invalid in [None, Some(0), Some(3)] {
                    assert!(preview(
                        &mut session,
                        &snap.revision,
                        &target.to_string(),
                        kind,
                        invalid
                    )
                    .unwrap_err()
                    .contains("MAINLINE"));
                }
                let plan = preview(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    kind,
                    Some(mainline),
                )
                .unwrap();
                assert_eq!(
                    plan.parent,
                    Some(if mainline == 1 { p1 } else { p2 }.to_string())
                );
                assert_eq!(fingerprint(d.path()), before);
                let command = if kind == Kind::CherryPick {
                    "cherry-pick"
                } else {
                    "revert"
                };
                let applied = git(
                    d.path(),
                    &[
                        command,
                        "--no-commit",
                        "--mainline",
                        &mainline.to_string(),
                        &target.to_string(),
                    ],
                );
                assert!(
                    applied.status.success(),
                    "{}",
                    String::from_utf8_lossy(&applied.stderr)
                );
                let tree = git(d.path(), &["write-tree"]);
                assert!(tree.status.success());
                assert_eq!(
                    plan.result_tree.as_deref(),
                    Some(String::from_utf8_lossy(&tree.stdout).trim())
                );
            }
        }
    }
    #[test]
    fn stale_dirty_detached_and_unknown_commit_guards() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let target = make(&repo, "refs/heads/main", &[("file.txt", "base\n")], &[]);
        checkout(&repo);
        let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(preview(
            &mut session,
            &snap.revision,
            &Oid::ZERO_SHA1.to_string(),
            Kind::Revert,
            None
        )
        .unwrap_err()
        .contains("INVALID_COMMIT"));
        fs::write(d.path().join("file.txt"), "dirty").unwrap();
        assert!(preview(
            &mut session,
            &snap.revision,
            &target.to_string(),
            Kind::Revert,
            None
        )
        .unwrap_err()
        .contains("STALE"));
        let dirty = session.snapshot().unwrap();
        let plan = preview(
            &mut session,
            &dirty.revision,
            &target.to_string(),
            Kind::Revert,
            None,
        )
        .unwrap();
        assert!(plan.blockers.contains(&"DIRTY_WORKTREE".into()));
        repo.set_head_detached(target).unwrap();
        let detached = session.snapshot().unwrap();
        let plan = preview(
            &mut session,
            &detached.revision,
            &target.to_string(),
            Kind::Revert,
            None,
        )
        .unwrap();
        assert!(plan.blockers.contains(&"DETACHED_HEAD".into()));
    }
    fn make_entries(
        repo: &Repository,
        reference: &str,
        files: &[(&str, &[u8], i32)],
        parents: &[Oid],
    ) -> Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, bytes, mode) in files {
            builder
                .insert(*name, repo.blob(bytes).unwrap(), *mode)
                .unwrap();
        }
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|id| repo.find_commit(*id).unwrap())
            .collect();
        let sig = git2::Signature::now("Replay Test", "replay@example.invalid").unwrap();
        repo.commit(
            Some(reference),
            &sig,
            &sig,
            "File operation",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn replay_file_operations_match_git_and_preserve_source() {
        type Files<'a> = Vec<(&'a str, &'a [u8], i32)>;
        let cases: Vec<(&str, Files<'_>, Files<'_>, bool)> = vec![
            ("delete", vec![("file", b"old\n", 0o100644)], vec![], false),
            ("add", vec![], vec![("file", b"new\n", 0o100644)], false),
            (
                "binary",
                vec![("file", b"old\0bytes", 0o100644)],
                vec![("file", b"new\0bytes", 0o100644)],
                false,
            ),
            (
                "rename",
                vec![("old", b"contents\n", 0o100644)],
                vec![("new", b"contents\n", 0o100644)],
                false,
            ),
            (
                "executable",
                vec![("file", b"#!/bin/sh\n", 0o100644)],
                vec![("file", b"#!/bin/sh\n", 0o100755)],
                false,
            ),
            (
                "binary-conflict",
                vec![("file", b"old\0bytes", 0o100644)],
                vec![("file", b"new\0bytes", 0o100644)],
                true,
            ),
            (
                "modify-delete",
                vec![("file", b"old\n", 0o100644)],
                vec![],
                true,
            ),
        ];
        for kind in [Kind::CherryPick, Kind::Revert] {
            for (name, before, after, conflict) in &cases {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = make_entries(&repo, "refs/heads/main", before, &[]);
                let target = make_entries(&repo, "refs/heads/topic", after, &[base]);
                let ours = if *conflict {
                    vec![("file", b"external\0edit".as_slice(), 0o100644)]
                } else if kind == Kind::CherryPick {
                    before.clone()
                } else {
                    after.clone()
                };
                make_entries(&repo, "refs/heads/main", &ours, &[base]);
                checkout(&repo);
                let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
                let snap = session.snapshot().unwrap();
                let original = fingerprint(d.path());
                let plan = preview(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    kind,
                    None,
                )
                .unwrap();
                assert_eq!(fingerprint(d.path()), original, "{kind:?}/{name}");
                let command = if kind == Kind::CherryPick {
                    "cherry-pick"
                } else {
                    "revert"
                };
                let applied = git(d.path(), &[command, "--no-commit", &target.to_string()]);
                assert_eq!(
                    applied.status.success(),
                    plan.conflicts.is_empty(),
                    "{kind:?}/{name}: {}",
                    String::from_utf8_lossy(&applied.stderr)
                );
                if applied.status.success() {
                    let tree = git(d.path(), &["write-tree"]);
                    assert!(tree.status.success());
                    assert_eq!(
                        plan.result_tree.as_deref(),
                        Some(String::from_utf8_lossy(&tree.stdout).trim()),
                        "{kind:?}/{name}"
                    );
                } else {
                    let mut actual = repo.index().unwrap();
                    actual.read(true).unwrap();
                    let mut stages = Vec::new();
                    for c in actual.conflicts().unwrap() {
                        let c = c.unwrap();
                        for (stage, entry) in [(1, c.ancestor), (2, c.our), (3, c.their)] {
                            if let Some(e) = entry {
                                stages.push((
                                    stage,
                                    String::from_utf8(e.path).unwrap(),
                                    e.id.to_string(),
                                ));
                            }
                        }
                    }
                    let mut expected = Vec::new();
                    for c in &plan.conflicts {
                        for (stage, entry) in [(1, &c.ancestor), (2, &c.ours), (3, &c.theirs)] {
                            if let Some(e) = entry {
                                expected.push((stage, e.path.clone(), e.oid.clone()));
                            }
                        }
                    }
                    stages.sort();
                    expected.sort();
                    assert_eq!(stages, expected, "{kind:?}/{name}");
                }
            }
        }
    }
    #[test]
    fn prepared_commit_bundle_preserves_source_and_original_author_then_imports() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            let d = tempfile::tempdir().unwrap();
            let repo = Repository::init(d.path()).unwrap();
            repo.set_head("refs/heads/main").unwrap();
            let base = make(
                &repo,
                "refs/heads/main",
                &[("file", "one\nshared\nthree\n")],
                &[],
            );
            let target = make(
                &repo,
                "refs/heads/topic",
                &[("file", "one\nshared\nTHREE\n")],
                &[base],
            );
            let ours = if kind == Kind::CherryPick {
                "ONE\nshared\nthree\n"
            } else {
                "ONE\nshared\nTHREE\n"
            };
            let head = make(&repo, "refs/heads/main", &[("file", ours)], &[base]);
            checkout(&repo);
            let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = fingerprint(d.path());
            let prepared = prepare_commit(
                &mut session,
                &snap.revision,
                &target.to_string(),
                kind,
                None,
                (
                    "  Replay message  \n\n",
                    "New Committer",
                    "committer@example.invalid",
                ),
            )
            .unwrap();
            assert_eq!(fingerprint(d.path()), before);
            assert_eq!(prepared.head, head);
            assert!(!repo.odb().unwrap().exists(prepared.oid));
            let imported = tempfile::tempdir().unwrap();
            let receiver = Repository::init_bare(imported.path()).unwrap();
            let odb = receiver.odb().unwrap();
            odb.add_disk_alternate(repo.path().join("objects").to_str().unwrap())
                .unwrap();
            for (id, kind, bytes) in &prepared.objects {
                assert!(!repo.odb().unwrap().exists(*id));
                assert_eq!(odb.write(*kind, bytes).unwrap(), *id);
            }
            let commit = receiver.find_commit(prepared.oid).unwrap();
            assert_eq!(commit.parent_count(), 1);
            assert_eq!(commit.parent_id(0).unwrap(), head);
            assert_eq!(commit.committer().name().unwrap(), "New Committer");
            assert_eq!(commit.message().unwrap(), "  Replay message\n");
            if kind == Kind::CherryPick {
                let original = repo.find_commit(target).unwrap();
                assert_eq!(commit.author().name_bytes(), original.author().name_bytes());
                assert_eq!(
                    commit.author().email_bytes(),
                    original.author().email_bytes()
                );
                assert_eq!(commit.author().when(), original.author().when());
            } else {
                assert_eq!(commit.author().name().unwrap(), "New Committer");
            }
            let entry = commit.tree().unwrap().get_name("file").unwrap().id();
            let expected = if kind == Kind::CherryPick {
                b"ONE\nshared\nTHREE\n".as_slice()
            } else {
                b"ONE\nshared\nthree\n".as_slice()
            };
            assert_eq!(receiver.find_blob(entry).unwrap().content(), expected);
            let command = if kind == Kind::CherryPick {
                "cherry-pick"
            } else {
                "revert"
            };
            assert!(
                git(d.path(), &[command, "--no-commit", &target.to_string()])
                    .status
                    .success()
            );
            let tree = git(d.path(), &["write-tree"]);
            assert!(tree.status.success());
            assert_eq!(
                commit.tree_id().to_string(),
                String::from_utf8_lossy(&tree.stdout).trim()
            );
        }
    }
    #[test]
    fn preparation_refuses_empty_conflicting_dirty_and_invalid_input_without_source_writes() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let base = make(&repo, "refs/heads/main", &[("file", "base\n")], &[]);
        let target = make(&repo, "refs/heads/topic", &[("file", "target\n")], &[base]);
        checkout(&repo);
        let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        assert!(prepare_commit(
            &mut session,
            &snap.revision,
            &base.to_string(),
            Kind::CherryPick,
            None,
            ("msg", "A", "a@example.invalid")
        )
        .err()
        .unwrap()
        .contains("EMPTY_REPLAY"));
        assert!(prepare_commit(
            &mut session,
            &snap.revision,
            &target.to_string(),
            Kind::CherryPick,
            None,
            ("", "A", "a@example.invalid")
        )
        .is_err());
        assert_eq!(fingerprint(d.path()), before);
        make(
            &repo,
            "refs/heads/main",
            &[("file", "conflicting\n")],
            &[base],
        );
        checkout(&repo);
        let snap = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        assert!(prepare_commit(
            &mut session,
            &snap.revision,
            &target.to_string(),
            Kind::CherryPick,
            None,
            ("msg", "A", "a@example.invalid")
        )
        .err()
        .unwrap()
        .contains("CONFLICTS"));
        assert_eq!(fingerprint(d.path()), before);
        fs::write(d.path().join("file"), "dirty").unwrap();
        let snap = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        assert!(prepare_commit(
            &mut session,
            &snap.revision,
            &target.to_string(),
            Kind::CherryPick,
            None,
            ("msg", "A", "a@example.invalid")
        )
        .err()
        .unwrap()
        .contains("DIRTY_WORKTREE"));
        assert_eq!(fingerprint(d.path()), before);
    }
    fn execution_fixture(kind: Kind) -> (tempfile::TempDir, Oid, Oid) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let base = make(
            &repo,
            "refs/heads/main",
            &[("file", "one\nshared\nthree\n")],
            &[],
        );
        let target = make(
            &repo,
            "refs/heads/topic",
            &[("file", "one\nshared\nTHREE\n")],
            &[base],
        );
        let content = if kind == Kind::CherryPick {
            "ONE\nshared\nthree\n"
        } else {
            "ONE\nshared\nTHREE\n"
        };
        let head = make(&repo, "refs/heads/main", &[("file", content)], &[base]);
        checkout(&repo);
        (d, head, target)
    }
    fn action(kind: Kind, target: Oid) -> Action {
        Action::ReplayCommit {
            allow_empty: false,
            oid: target.to_string(),
            operation: kind,
            mainline: None,
            message: "Replay test".into(),
            author: "Committer".into(),
            email: "committer@example.invalid".into(),
        }
    }
    #[test]
    fn explicit_empty_replay_creates_one_parent_commit_only_when_requested() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for original in [true, false] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = make(&repo, "refs/heads/main", &[("file", "base\n")], &[]);
                let target = make(
                    &repo,
                    "refs/heads/topic",
                    &[("file", if original { "base\n" } else { "changed\n" })],
                    &[base],
                );
                let content = if !original && kind == Kind::CherryPick {
                    "changed\n"
                } else {
                    "base\n"
                };
                let head = make(&repo, "refs/heads/main", &[("file", content)], &[base]);
                checkout(&repo);
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let plan = preview(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    kind,
                    None,
                )
                .unwrap();
                assert!(plan.empty);
                assert_eq!(plan.originally_empty, original);
                assert!(session
                    .mutate(&snap.revision, action(kind, target))
                    .unwrap_err()
                    .contains("EMPTY_REPLAY"));
                assert_eq!(repo.head().unwrap().target(), Some(head));
                let snap = session.snapshot().unwrap();
                let mut keep = action(kind, target);
                if let Action::ReplayCommit { allow_empty, .. } = &mut keep {
                    *allow_empty = true;
                }
                let done = session.mutate(&snap.revision, keep).unwrap();
                assert!(done.changes.is_empty());
                let fresh = open(&root).unwrap();
                let commit = fresh.head().unwrap().peel_to_commit().unwrap();
                assert_ne!(commit.id(), head);
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap(), head);
                assert_eq!(commit.tree_id(), fresh.find_commit(head).unwrap().tree_id());
                assert_eq!(fresh.refname_to_id("refs/heads/topic").unwrap(), target);
                assert_eq!(fs::read_to_string(root.join("file")).unwrap(), content);
                assert_eq!(
                    fresh.index().unwrap().write_tree().unwrap(),
                    commit.tree_id()
                );
                if kind == Kind::CherryPick {
                    assert_eq!(
                        commit.author().name_bytes(),
                        fresh.find_commit(target).unwrap().author().name_bytes()
                    );
                }
                assert!(!checkout::pending(&root));
                // Git CLI explicit no-commit replay also produces the unchanged tree.
                let oracle = tempfile::tempdir().unwrap();
                let out = git(
                    oracle.path(),
                    &["clone", "--no-hardlinks", d.path().to_str().unwrap(), "."],
                );
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert!(
                    git(oracle.path(), &["checkout", "--detach", &head.to_string()])
                        .status
                        .success()
                );
                let out = git(
                    oracle.path(),
                    &[
                        if kind == Kind::CherryPick {
                            "cherry-pick"
                        } else {
                            "revert"
                        },
                        "--no-commit",
                        &target.to_string(),
                    ],
                );
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(
                    String::from_utf8(git(oracle.path(), &["write-tree"]).stdout)
                        .unwrap()
                        .trim(),
                    commit.tree_id().to_string()
                );
            }
        }
    }

    #[test]
    fn keep_empty_never_overrides_dirty_detached_or_stale_guards() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            let d = tempfile::tempdir().unwrap();
            let repo = Repository::init(d.path()).unwrap();
            repo.set_head("refs/heads/main").unwrap();
            let head = make(&repo, "refs/heads/main", &[("file", "base\n")], &[]);
            let target = make(&repo, "refs/heads/topic", &[("file", "base\n")], &[head]);
            checkout(&repo);
            let root = d.path().canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let mut keep = action(kind, target);
            if let Action::ReplayCommit { allow_empty, .. } = &mut keep {
                *allow_empty = true;
            }
            let index = fs::read(root.join(".git/index")).unwrap();
            fs::write(root.join("file"), "external change").unwrap();
            assert!(session
                .mutate(&snap.revision, keep.clone())
                .unwrap_err()
                .starts_with("STALE"));
            let dirty = session.snapshot().unwrap();
            assert!(session
                .mutate(&dirty.revision, keep.clone())
                .unwrap_err()
                .contains("DIRTY_CHECKOUT"));
            assert_eq!(
                fs::read_to_string(root.join("file")).unwrap(),
                "external change"
            );
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            fs::write(root.join("file"), "base\n").unwrap();
            repo.set_head_detached(head).unwrap();
            let detached = session.snapshot().unwrap();
            assert!(session
                .mutate(&detached.revision, keep.clone())
                .unwrap_err()
                .contains("DETACHED"));
            repo.set_head("refs/heads/main").unwrap();
            let snap = session.snapshot().unwrap();
            fs::write(root.join(".git/index.lock"), "external lock").unwrap();
            assert!(session.mutate(&snap.revision, keep).is_err());
            assert_eq!(
                fs::read_to_string(root.join(".git/index.lock")).unwrap(),
                "external lock"
            );
            assert_eq!(repo.head().unwrap().target(), Some(head));
        }
    }

    #[test]
    fn keep_empty_root_and_explicit_merge_parent_match_git_trees() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for case in [0, 1, 2] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = make(&repo, "refs/heads/main", &[("file", "base\n")], &[]);
                let side = make(&repo, "refs/heads/side", &[("file", "side\n")], &[base]);
                let target = if case == 0 {
                    make(&repo, "refs/heads/topic", &[], &[])
                } else {
                    make(
                        &repo,
                        "refs/heads/topic",
                        &[("file", "base\n")],
                        &[base, side],
                    )
                };
                let content = if case == 2 && kind == Kind::Revert {
                    "side\n"
                } else {
                    "base\n"
                };
                let head = make(&repo, "refs/heads/main", &[("file", content)], &[base]);
                checkout(&repo);
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let mainline = if case == 0 { None } else { Some(case) };
                if case > 0 {
                    assert!(preview(
                        &mut session,
                        &snap.revision,
                        &target.to_string(),
                        kind,
                        None
                    )
                    .unwrap_err()
                    .contains("MAINLINE_REQUIRED"));
                }
                let plan = preview(
                    &mut session,
                    &snap.revision,
                    &target.to_string(),
                    kind,
                    mainline,
                )
                .unwrap();
                assert!(plan.empty);
                assert_eq!(plan.originally_empty, case != 2);
                let mut keep = action(kind, target);
                if let Action::ReplayCommit {
                    allow_empty,
                    mainline: m,
                    ..
                } = &mut keep
                {
                    *allow_empty = true;
                    *m = mainline;
                }
                let done = session.mutate(&snap.revision, keep).unwrap();
                assert!(done.changes.is_empty());
                let repo = open(&root).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap(), head);
                assert_eq!(commit.tree_id(), repo.find_commit(head).unwrap().tree_id());
                let oracle = tempfile::tempdir().unwrap();
                assert!(git(
                    oracle.path(),
                    &["clone", "--no-hardlinks", d.path().to_str().unwrap(), "."]
                )
                .status
                .success());
                assert!(
                    git(oracle.path(), &["checkout", "--detach", &head.to_string()])
                        .status
                        .success()
                );
                let target_string = target.to_string();
                let mainline_string = case.to_string();
                let mut args = vec![
                    if kind == Kind::CherryPick {
                        "cherry-pick"
                    } else {
                        "revert"
                    },
                    "--no-commit",
                ];
                if case > 0 {
                    args.extend(["-m", &mainline_string]);
                }
                args.push(&target_string);
                let out = git(oracle.path(), &args);
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(
                    String::from_utf8(git(oracle.path(), &["write-tree"]).stdout)
                        .unwrap()
                        .trim(),
                    commit.tree_id().to_string()
                );
            }
        }
    }

    #[test]
    fn frozen_committer_time_recreates_the_same_private_commit_after_restart() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            let (d, head, target) = execution_fixture(kind);
            let root = d.path().canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = fingerprint(&root);
            let prepare = |session: &mut Session, revision: &str, time| {
                prepare_resolved(
                    session,
                    revision,
                    &target.to_string(),
                    kind,
                    None,
                    ("Durable step", "Sequence Test", "sequence@example.invalid"),
                    PrepareOptions {
                        committer_time: Some(time),
                        ..Default::default()
                    },
                )
            };
            let time = git2::Time::new(1_700_000_000, 540);
            let first = prepare(&mut session, &snap.revision, time).unwrap();
            assert_eq!(first.head, head);
            drop(session);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let second = prepare(&mut session, &snap.revision, time).unwrap();
            assert_eq!(first.oid, second.oid);
            let objects = |p: &Prepared| {
                p.objects
                    .iter()
                    .map(|(id, _, bytes)| (id.to_string(), bytes.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            assert_eq!(objects(&first), objects(&second));
            let different = prepare(
                &mut session,
                &snap.revision,
                git2::Time::new(1_700_000_001, 540),
            )
            .unwrap();
            assert_ne!(first.oid, different.oid);
            assert_eq!(
                prepare(
                    &mut session,
                    &snap.revision,
                    git2::Time::new(1_700_000_000, 1440)
                )
                .err()
                .unwrap(),
                "INVALID_REPLAY_TIME"
            );
            assert_eq!(fingerprint(&root), before);
        }
    }

    #[test]
    fn public_replay_action_updates_current_branch_and_preserves_source_branch() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            let (d, head, target) = execution_fixture(kind);
            let root = d.path().canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let result = session
                .mutate(&snap.revision, action(kind, target))
                .unwrap();
            assert_eq!(result.branch, "main");
            assert!(result.changes.is_empty());
            assert!(!result.recovery_pending);
            let repo = open(&root).unwrap();
            let new = repo.head().unwrap().peel_to_commit().unwrap();
            verify_commit(
                &repo,
                head,
                new.id(),
                &Record {
                    kind,
                    source: target.to_string(),
                    mainline: None,
                    resolutions: None,
                },
            )
            .unwrap();
            assert_eq!(repo.refname_to_id("refs/heads/topic").unwrap(), target);
            let expected = if kind == Kind::CherryPick {
                "ONE\nshared\nTHREE\n"
            } else {
                "ONE\nshared\nthree\n"
            };
            assert_eq!(fs::read_to_string(root.join("file")).unwrap(), expected);
        }
    }
    #[test]
    fn replay_child_job() {
        let Some(root) = std::env::var_os("BRANCHDESK_REPLAY_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let mut action = if std::env::var_os("BRANCHDESK_RECOVERY_STEP").is_some() {
            Action::RecoverCheckout
        } else {
            let repo = open(&root).unwrap();
            let kind = if std::env::var("BRANCHDESK_REPLAY_KIND").unwrap() == "revert" {
                Kind::Revert
            } else {
                Kind::CherryPick
            };
            action(kind, repo.refname_to_id("refs/heads/topic").unwrap())
        };
        if let Action::ReplayCommit { allow_empty, .. } = &mut action {
            *allow_empty = std::env::var("BRANCHDESK_REPLAY_KEEP_EMPTY").as_deref() == Ok("1");
        }
        session.mutate(&snap.revision, action).unwrap();
    }
    fn kill_replay(root: &Path, kind: Kind, step: usize, recovery: bool) {
        kill_replay_with_empty(root, kind, step, recovery, false);
    }
    fn kill_replay_with_empty(
        root: &Path,
        kind: Kind,
        step: usize,
        recovery: bool,
        keep_empty: bool,
    ) {
        let ready = root.join(".git/branchdesk-checkout/child.ready");
        let _ = fs::remove_file(&ready);
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "repository::replay::tests::replay_child_job",
                "--nocapture",
            ])
            .env("BRANCHDESK_REPLAY_TEST_ROOT", root)
            .env(
                "BRANCHDESK_REPLAY_KEEP_EMPTY",
                if keep_empty { "1" } else { "0" },
            )
            .env("BRANCHDESK_CRASH_ROOT", root)
            .env(
                "BRANCHDESK_REPLAY_KIND",
                if kind == Kind::Revert {
                    "revert"
                } else {
                    "cherry"
                },
            )
            .env(
                if recovery {
                    "BRANCHDESK_RECOVERY_STEP"
                } else {
                    "BRANCHDESK_CRASH_STEP"
                },
                step.to_string(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("checkpoint timeout {step}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn empty_replay_actual_kill_and_recovery_restore_original_head() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for step in [100, 101, 102, 0, 1, 2] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let head = make(&repo, "refs/heads/main", &[("file", "unchanged\n")], &[]);
                let target = make(
                    &repo,
                    "refs/heads/topic",
                    &[("file", "unchanged\n")],
                    &[head],
                );
                checkout(&repo);
                let root = d.path().canonicalize().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                kill_replay_with_empty(&root, kind, step, false, true);
                if step == 2 {
                    kill_replay_with_empty(&root, kind, 1, true, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let restored = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(restored.head_oid, Some(head.to_string()));
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), b"unchanged\n");
                let mut keep = action(kind, target);
                if let Action::ReplayCommit { allow_empty, .. } = &mut keep {
                    *allow_empty = true;
                }
                let done = session.mutate(&restored.revision, keep).unwrap();
                assert!(!done.recovery_pending);
                let repo = open(&root).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap(), head);
                assert_eq!(commit.tree_id(), repo.find_commit(head).unwrap().tree_id());
                assert_eq!(repo.refname_to_id("refs/heads/topic").unwrap(), target);
            }
        }
    }

    #[test]
    fn actual_replay_kill_boundaries_and_recovery_kill_restore_original_state() {
        for kind in [Kind::CherryPick, Kind::Revert] {
            for step in [100, 101, 102, 0, 1, 2, 3] {
                let (d, head, target) = execution_fixture(kind);
                let root = d.path().canonicalize().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let file = fs::read(root.join("file")).unwrap();
                kill_replay(&root, kind, step, false);
                if step == 2 {
                    kill_replay(&root, kind, 1, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let result = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!result.recovery_pending);
                let repo = open(&root).unwrap();
                assert_eq!(repo.head().unwrap().target(), Some(head));
                assert_eq!(repo.refname_to_id("refs/heads/topic").unwrap(), target);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("file")).unwrap(), file);
                let result = session
                    .mutate(&result.revision, action(kind, target))
                    .unwrap();
                assert!(result.changes.is_empty());
                assert!(!result.recovery_pending);
            }
        }
    }
    #[test]
    fn replay_recovery_preserves_external_edits_and_rejects_forged_source() {
        for forged in [false, true] {
            let (d, head, _) = execution_fixture(Kind::CherryPick);
            let root = d.path().canonicalize().unwrap();
            kill_replay(&root, Kind::CherryPick, 1, false);
            if forged {
                let path = root.join(".git/branchdesk-checkout/manifest.json");
                let mut record: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record["replay"]["source"] = serde_json::Value::String(head.to_string());
                fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
            } else {
                fs::write(root.join("file"), "external changes").unwrap();
            }
            let file = fs::read(root.join("file")).unwrap();
            let index = fs::read(root.join(".git/index")).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let error = session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .err()
                .unwrap();
            assert!(
                error.contains(if forged {
                    "INVALID_REPLAY"
                } else {
                    "RECOVERY_CONFLICT"
                }),
                "{error}"
            );
            assert_eq!(fs::read(root.join("file")).unwrap(), file);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert!(checkout::pending(&root));
        }
    }
}
