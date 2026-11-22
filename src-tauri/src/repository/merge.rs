//! Merge analysis in a private temporary object database. The source repository is read-only.
use super::*;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictVersion {
    pub id: String,
    pub path: String,
    pub oid: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeConflict {
    pub ancestor: Option<ConflictVersion>,
    pub ours: Option<ConflictVersion>,
    pub theirs: Option<ConflictVersion>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePlan {
    pub revision: String,
    pub current_branch: String,
    pub target_branch: String,
    pub head_oid: String,
    pub target_oid: String,
    pub base_oid: Option<String>,
    pub kind: String,
    pub blockers: Vec<String>,
    pub files: Vec<Change>,
    pub conflicts: Vec<MergeConflict>,
    pub diff: Option<DiffResult>,
}
// ':' cannot occur in a Git branch name, so this encoding cannot collide with
// existing local branch selections carried by the same typed merge commands.
pub(super) fn reference_name(name: &str) -> Result<String> {
    if let Some(reference) = name.strip_prefix("remote:").or_else(||name.strip_prefix("pull-ff:")).or_else(||name.strip_prefix("pull-ff-only:")) {
        if reference.len() > 1024
            || !reference.starts_with("refs/remotes/")
            || !git2::Reference::is_valid_name(reference)
        {
            return Err("INVALID_REMOTE_MERGE_TARGET".into());
        }
        Ok(reference.into())
    } else {
        let full = format!("refs/heads/{name}");
        if name.len() > 200 || name.starts_with('-') || !git2::Reference::is_valid_name(&full) {
            return Err("INVALID_BRANCH".into());
        }
        Ok(full)
    }
}
pub(super) fn display_name(name: &str) -> &str {
    name.strip_prefix("remote:refs/remotes/").or_else(||name.strip_prefix("pull-ff:refs/remotes/")).or_else(||name.strip_prefix("pull-ff-only:refs/remotes/")).unwrap_or(name)
}
pub(super) fn version(entry: Option<git2::IndexEntry>) -> Result<Option<ConflictVersion>> {
    entry
        .map(|e| {
            let path = String::from_utf8(e.path).map_err(|_| "UNSUPPORTED_PATH")?;
            Ok(ConflictVersion {
                id: file_id(&path),
                path,
                oid: e.id.to_string(),
            })
        })
        .transpose()
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum FastForwardPolicy {
    Allow,
    Only,
    Never,
}
pub(super) fn ff_policy(root: &Path, remote: bool) -> Result<FastForwardPolicy> {
    let entries = remote_remove::entries(&bounded(&root.join(".git/config"), 65536)?)?;
    let get = |key: &str| {
        entries
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let value = if remote {
        get("pull.ff").or_else(|| get("merge.ff"))
    } else {
        get("merge.ff")
    };
    match value {
        None => Ok(FastForwardPolicy::Allow),
        Some("only") => Ok(FastForwardPolicy::Only),
        Some(value) => match Config::parse_bool(value) {
            Ok(true) => Ok(FastForwardPolicy::Allow),
            Ok(false) => Ok(FastForwardPolicy::Never),
            Err(_) => Err("MERGE_FF_POLICY_UNSUPPORTED".into()),
        },
    }
}
// The typed target encoding carries per-Pull policy through preview, draft and
// execution. ':' is invalid in ordinary branch names; targets still validate
// as refs/remotes entries through reference_name.
pub(super) fn target_ff_policy(root:&Path,name:&str)->Result<FastForwardPolicy>{
    if name.starts_with("pull-ff:"){reference_name(name)?;return Ok(FastForwardPolicy::Allow);}
    if name.starts_with("pull-ff-only:"){reference_name(name)?;return Ok(FastForwardPolicy::Only);}
    ff_policy(root,name.starts_with("remote:"))
}
pub(super) fn preview(session: &mut Session, revision: &str, name: &str) -> Result<MergePlan> {
    let snap = session.fresh(revision)?;
    let policy = target_ff_policy(&session.root, name)?;
    let repo = open(&session.root)?;
    let target = repo
        .find_reference(&reference_name(name)?)
        .map_err(|_| "INVALID_BRANCH")?;
    if target.symbolic_target().map_err(err)?.is_some() {
        return Err("SYMBOLIC_MERGE_TARGET".into());
    }
    let head = repo.head().map_err(err)?;
    if !head.is_branch() {
        return Err("UNSUPPORTED_DETACHED_MERGE".into());
    }
    let ours = head.peel_to_commit().map_err(err)?;
    let theirs = target.peel_to_commit().map_err(err)?;
    let mut plan = MergePlan {
        revision: revision.into(),
        current_branch: snap.branch,
        target_branch: name.into(),
        head_oid: ours.id().to_string(),
        target_oid: theirs.id().to_string(),
        base_oid: None,
        kind: String::new(),
        blockers: Vec::new(),
        files: Vec::new(),
        conflicts: Vec::new(),
        diff: None,
    };
    if let Some(block) = snap.write_block {
        plan.blockers.push(block);
    }
    if !snap.changes.is_empty() {
        plan.blockers
            .push("현재 작업 파일 또는 index에 변경이 있습니다.".into());
    }
    if ours.id() == theirs.id()
        || repo
            .graph_descendant_of(ours.id(), theirs.id())
            .map_err(err)?
    {
        plan.kind = "upToDate".into();
        plan.base_oid = Some(theirs.id().to_string());
        return Ok(plan);
    }
    let fast_forward = repo
        .graph_descendant_of(theirs.id(), ours.id())
        .map_err(err)?;
    let base = if fast_forward {
        ours.id()
    } else {
        let bases = match repo.merge_bases(ours.id(), theirs.id()) {
            Ok(bases) => bases,
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                plan.kind = "unrelated".into();
                plan.blockers
                    .push("공통 조상이 없는 이력의 병합은 아직 지원하지 않습니다.".into());
                return Ok(plan);
            }
            Err(e) => return Err(err(e)),
        };
        if bases.len() != 1 {
            plan.kind = "multipleBases".into();
            plan.blockers
                .push("여러 merge base를 재귀적으로 병합하는 처리가 필요합니다.".into());
            return Ok(plan);
        }
        bases[0]
    };
    plan.base_oid = Some(base.to_string());
    for oid in [ours.id(), theirs.id(), base] {
        checkout::inspect_tree(
            &repo,
            &repo.find_commit(oid).map_err(err)?.tree().map_err(err)?,
        )?;
    }
    // libgit2 merge_trees can create auto-merged blobs. Never place preview objects in the user's ODB.
    let temporary = tempfile::tempdir().map_err(err)?;
    let mut init = git2::RepositoryInitOptions::new();
    init.bare(true).external_template(false);
    let scratch = Repository::init_opts(temporary.path(), &init).map_err(err)?;
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
    let before = scratch
        .find_commit(ours.id())
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let target_tree = scratch
        .find_commit(theirs.id())
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let base_tree = scratch
        .find_commit(base)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let merged = if fast_forward {
        None
    } else {
        Some(
            scratch
                .merge_trees(&base_tree, &before, &target_tree, None)
                .map_err(err)?,
        )
    };
    if let Some(index) = &merged {
        for conflict in index.conflicts().map_err(err)? {
            if plan.conflicts.len() >= MAX_FILES {
                return Err("LIMIT: merge conflicts".into());
            }
            let c = conflict.map_err(err)?;
            plan.conflicts.push(MergeConflict {
                ancestor: version(c.ancestor)?,
                ours: version(c.our)?,
                theirs: version(c.their)?,
            });
        }
    }
    if !fast_forward && policy == FastForwardPolicy::Only {
        plan.blockers.push(
            "FF_ONLY: upstream has diverged; merge commit is disabled by configuration".into(),
        );
    }
    plan.kind = if fast_forward && policy != FastForwardPolicy::Never {
        "fastForward"
    } else if plan.conflicts.is_empty() {
        "mergeCommit"
    } else {
        "conflicts"
    }
    .into();
    if !plan.conflicts.is_empty() {
        plan.blockers.push("충돌 파일을 해결해야 합니다.".into());
    }
    let mut options = DiffOptions::new();
    options.max_size(FILE_LIMIT as i64);
    let diff = if let Some(index) = &merged {
        scratch
            .diff_tree_to_index(Some(&before), Some(index), Some(&mut options))
            .map_err(err)?
    } else {
        scratch
            .diff_tree_to_tree(Some(&before), Some(&target_tree), Some(&mut options))
            .map_err(err)?
    };
    if diff.deltas().len() > MAX_FILES {
        return Err("LIMIT: merge changed files".into());
    }
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(Path::to_str)
            .ok_or("UNSUPPORTED_PATH")?
            .to_string();
        plan.files.push(Change {
            id: file_id(&path),
            path,
            staged: false,
            unstaged: false,
            status: match delta.status() {
                git2::Delta::Added => "added",
                git2::Delta::Deleted => "deleted",
                git2::Delta::Conflicted => "conflict",
                _ => "modified",
            }
            .into(),
        });
    }
    plan.diff = Some(render_diff(&diff)?);
    session.fresh(revision)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make(repo: &Repository, reference: &str, contents: &str, parents: &[Oid]) -> Oid {
        let mut tree = repo.treebuilder(None).unwrap();
        tree.insert(
            "file.txt",
            repo.blob(contents.as_bytes()).unwrap(),
            0o100644,
        )
        .unwrap();
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|oid| repo.find_commit(*oid).unwrap())
            .collect();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(
            Some(reference),
            &sig,
            &sig,
            "Merge fixture",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn fixture() -> (tempfile::TempDir, Oid) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let base = make(&repo, "refs/heads/main", "one\nshared\nthree\n", &[]);
        checkout(&repo, d.path());
        (d, base)
    }
    fn checkout(repo: &Repository, root: &Path) {
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let blob = repo
            .find_blob(tree.get_name("file.txt").unwrap().id())
            .unwrap();
        fs::write(root.join("file.txt"), blob.content()).unwrap();
        let mut index = repo.index().unwrap();
        index.read_tree(&tree).unwrap();
        index.write().unwrap();
    }
    fn fingerprint(root: &Path) -> HashMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, path: &Path, out: &mut HashMap<PathBuf, Vec<u8>>) {
            for e in fs::read_dir(path).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    walk(root, &e.path(), out)
                } else {
                    out.insert(
                        e.path().strip_prefix(root).unwrap().into(),
                        fs::read(e.path()).unwrap(),
                    );
                }
            }
        }
        let mut out = HashMap::new();
        walk(root, root, &mut out);
        out
    }
    #[test]
    fn clean_three_way_merge_uses_private_objects_and_preserves_entire_repository() {
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        let ours = make(&repo, "refs/heads/main", "FIRST\nshared\nthree\n", &[base]);
        let theirs = make(&repo, "refs/heads/topic", "one\nshared\nTHIRD\n", &[base]);
        checkout(&repo, d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let plan = preview(&mut session, &s.revision, "topic").unwrap();
        assert_eq!(plan.kind, "mergeCommit");
        assert_eq!(plan.head_oid, ours.to_string());
        assert_eq!(plan.target_oid, theirs.to_string());
        assert_eq!(plan.base_oid, Some(base.to_string()));
        assert!(plan.conflicts.is_empty());
        assert!(plan.blockers.is_empty());
        assert_eq!(plan.files.len(), 1);
        assert!(plan.diff.unwrap().text.contains("+THIRD"));
        assert_eq!(before, fingerprint(d.path()));
    }
    #[test]
    fn conflicts_report_all_stage_identities_without_touching_index_or_files() {
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        make(&repo, "refs/heads/main", "OURS\nshared\nthree\n", &[base]);
        make(
            &repo,
            "refs/heads/topic",
            "THEIRS\nshared\nthree\n",
            &[base],
        );
        checkout(&repo, d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let plan = preview(&mut session, &s.revision, "topic").unwrap();
        assert_eq!(plan.kind, "conflicts");
        assert_eq!(plan.conflicts.len(), 1);
        let c = &plan.conflicts[0];
        assert_eq!(c.ours.as_ref().unwrap().path, "file.txt");
        assert_ne!(c.ours.as_ref().unwrap().oid, c.theirs.as_ref().unwrap().oid);
        assert!(c.ancestor.is_some());
        assert!(!plan.blockers.is_empty());
        assert_eq!(before, fingerprint(d.path()));
    }
    #[test]
    fn fast_forward_up_to_date_unrelated_and_dirty_are_distinguished() {
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        make(&repo, "refs/heads/topic", "changed\n", &[base]);
        make(&repo, "refs/heads/unrelated", "orphan\n", &[]);
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        assert_eq!(
            preview(&mut session, &s.revision, "main").unwrap().kind,
            "upToDate"
        );
        assert_eq!(
            preview(&mut session, &s.revision, "topic").unwrap().kind,
            "fastForward"
        );
        assert_eq!(
            preview(&mut session, &s.revision, "unrelated")
                .unwrap()
                .kind,
            "unrelated"
        );
        fs::write(d.path().join("file.txt"), "dirty\n").unwrap();
        assert!(preview(&mut session, &s.revision, "topic")
            .unwrap_err()
            .starts_with("STALE_STATE"));
        let dirty = session.snapshot().unwrap();
        let plan = preview(&mut session, &dirty.revision, "topic").unwrap();
        assert_eq!(plan.kind, "fastForward");
        assert!(!plan.blockers.is_empty());
        assert!(preview(&mut session, &dirty.revision, "../../unknown").is_err());
        repo.set_head_detached(base).unwrap();
        let detached = session.snapshot().unwrap();
        assert!(preview(&mut session, &detached.revision, "topic")
            .unwrap_err()
            .starts_with("UNSUPPORTED_DETACHED"));
    }
    #[test]
    fn modify_delete_conflict_keeps_absent_incoming_stage_explicit() {
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        make(&repo, "refs/heads/main", "edited\n", &[base]);
        let empty = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let parent = repo.find_commit(base).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(
            Some("refs/heads/topic"),
            &sig,
            &sig,
            "Delete",
            &empty,
            &[&parent],
        )
        .unwrap();
        checkout(&repo, d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let plan = preview(&mut session, &s.revision, "topic").unwrap();
        assert_eq!(plan.kind, "conflicts");
        assert!(plan.conflicts[0].ancestor.is_some());
        assert!(plan.conflicts[0].ours.is_some());
        assert!(plan.conflicts[0].theirs.is_none());
        assert!(content::read(
            &mut session,
            &s.revision,
            &file_id("file.txt"),
            content::ContentSource::MergeConflict {
                name: "topic".into(),
                conflict: 0,
                side: content::MergeSide::Theirs
            }
        )
        .unwrap_err()
        .contains("ABSENT_MERGE_VERSION"));
        assert_eq!(before, fingerprint(d.path()));
    }
    #[test]
    fn conflict_content_is_scoped_to_recomputed_stages_and_preserves_repository() {
        use content::{ContentSource, MergeSide};
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        make(&repo, "refs/heads/main", "OURS\nshared\nthree\n", &[base]);
        make(
            &repo,
            "refs/heads/topic",
            "THEIRS\nshared\nthree\n",
            &[base],
        );
        checkout(&repo, d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let source = |side, conflict| ContentSource::MergeConflict {
            name: "topic".into(),
            conflict,
            side,
        };
        let id = file_id("file.txt");
        for (side, expected) in [
            (MergeSide::Ancestor, "one\nshared\nthree\n"),
            (MergeSide::Ours, "OURS\nshared\nthree\n"),
            (MergeSide::Theirs, "THEIRS\nshared\nthree\n"),
        ] {
            let result =
                content::read(&mut session, &snapshot.revision, &id, source(side, 0)).unwrap();
            assert_eq!(result.text, expected);
            assert_eq!(result.state, "text");
            assert!(!result.truncated);
        }
        assert!(content::read(
            &mut session,
            &snapshot.revision,
            "wrong",
            source(MergeSide::Ours, 0)
        )
        .unwrap_err()
        .contains("INVALID_FILE_ID"));
        assert!(content::read(
            &mut session,
            &snapshot.revision,
            &id,
            source(MergeSide::Ours, 1)
        )
        .unwrap_err()
        .contains("INVALID_MERGE_CONFLICT"));
        assert!(content::read(
            &mut session,
            &snapshot.revision,
            &id,
            source(MergeSide::Ours, usize::MAX)
        )
        .is_err());
        assert_eq!(before, fingerprint(d.path()));
        let parent = repo.refname_to_id("refs/heads/topic").unwrap();
        make(&repo, "refs/heads/topic", "OTHER\n", &[parent]);
        assert!(content::read(
            &mut session,
            &snapshot.revision,
            &id,
            source(MergeSide::Ours, 0)
        )
        .is_err());
        assert!(serde_json::from_str::<ContentSource>(r#"{"kind":"mergeConflict","name":"topic","conflict":0,"side":"ours","oid":"arbitrary"}"#).is_err());
        assert!(serde_json::from_str::<ContentSource>(
            r#"{"kind":"mergeConflict","name":"topic","conflict":0,"side":"unknown"}"#
        )
        .is_err());
    }
    #[test]
    fn criss_cross_history_is_not_misrepresented_as_single_base_merge() {
        let (d, base) = fixture();
        let repo = open(d.path()).unwrap();
        let a = make(&repo, "refs/heads/main", "a\n", &[base]);
        let b = make(&repo, "refs/heads/topic", "b\n", &[base]);
        make(&repo, "refs/heads/main", "ab\n", &[a, b]);
        make(&repo, "refs/heads/topic", "ba\n", &[b, a]);
        checkout(&repo, d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let plan = preview(&mut session, &s.revision, "topic").unwrap();
        assert_eq!(plan.kind, "multipleBases");
        assert!(plan.base_oid.is_none());
        assert!(!plan.blockers.is_empty());
        assert_eq!(before, fingerprint(d.path()));
    }
}
