//! Direct tree-to-tree comparison. Never changes refs, index or working files.
use super::*;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub base_oid: String,
    pub target_oid: String,
    pub files: Vec<Change>,
    pub after_only: usize,
    pub before_only: usize,
    pub counts_limited: bool,
    pub diff: Option<DiffResult>,
}
fn unique_count(repo:&Repository, tip:Oid, exclude:Oid, deadline:std::time::Instant)->Result<(usize,bool)> {
    let mut walk=repo.revwalk().map_err(err)?;
    walk.push(tip).map_err(err)?;walk.hide(exclude).map_err(err)?;
    let mut count=0;
    for oid in walk {
        if count>=20000 || std::time::Instant::now()>=deadline {return Ok((count,true));}
        oid.map_err(err)?;count+=1;
    }
    Ok((count,false))
}
pub fn read(
    session: &mut Session,
    revision: &str,
    base: &str,
    target: &str,
    id: Option<&str>,
) -> Result<Comparison> {
    for oid in [base, target] {
        if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("INVALID_COMMIT".into());
        }
    }
    if id.is_some_and(|id| id.len() > 128) {
        return Err("INVALID_FILE_ID".into());
    }
    let snapshot = session.fresh(revision)?;
    for oid in [base, target] {
        let known=snapshot.commits.iter().any(|c|c.oid==oid)
            || snapshot.head_oid.as_deref()==Some(oid)
            || snapshot.branches.iter().any(|branch|branch.oid==oid)
            || snapshot.tags.iter().any(|tag|tag.oid.as_deref()==Some(oid))
            || snapshot.remotes.iter().any(|remote|remote.branches.iter().any(|branch|branch.oid.as_deref()==Some(oid)));
        if !known {return Err("INVALID_COMMIT: select a loaded commit or current reference tip".into());}
    }
    let repo = open(&session.root)?;
    let before = repo
        .find_commit(Oid::from_str(base).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let after = repo
        .find_commit(Oid::from_str(target).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    let mut options = DiffOptions::new();
    options.max_size(FILE_LIMIT as i64);
    let changes = repo
        .diff_tree_to_tree(Some(&before), Some(&after), Some(&mut options))
        .map_err(err)?;
    if changes.deltas().len() > MAX_FILES {
        return Err("LIMIT: comparison files exceed 2,000".into());
    }
    let mut files = Vec::new();
    for delta in changes.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(Path::to_str)
            .ok_or("UNSUPPORTED_PATH")?
            .to_string();
        files.push(Change {
            id: file_id(&path),
            path,
            staged: false,
            unstaged: false,
            status: match delta.status() {
                git2::Delta::Added => "added",
                git2::Delta::Deleted => "deleted",
                _ => "modified",
            }
            .into(),
        });
    }
    let diff = if let Some(id) = id {
        let file = files
            .iter()
            .find(|file| file.id == id)
            .ok_or("INVALID_FILE_ID")?;
        let mut options = DiffOptions::new();
        options
            .pathspec(&file.path)
            .disable_pathspec_match(true)
            .max_size(FILE_LIMIT as i64);
        Some(render_diff(
            &repo
                .diff_tree_to_tree(Some(&before), Some(&after), Some(&mut options))
                .map_err(err)?,
        )?)
    } else {
        None
    };
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
    let base_id=Oid::from_str(base).map_err(err)?;let target_id=Oid::from_str(target).map_err(err)?;
    let (after_only,after_limited)=unique_count(&repo,target_id,base_id,deadline)?;
    let (before_only,before_limited)=unique_count(&repo,base_id,target_id,deadline)?;
    session.fresh(revision)?;
    Ok(Comparison {
        after_only,before_only,counts_limited:after_limited||before_limited,
        base_oid: base.into(),
        target_oid: target.into(),
        files,
        diff,
    })
}
#[cfg(test)]
mod tests {
    use super::super::tests::{checkout_target, fixture};
    use super::*;
    #[test]
    fn comparison_direction_same_commit_and_stale_checks_preserve_repository() {
        let d = fixture();
        let target = checkout_target(d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        fs::write(d.path().join("hello.txt"), "dirty work\n").unwrap();
        let snapshot = session.snapshot().unwrap();
        let base = snapshot.head_oid.as_ref().unwrap();
        let head = fs::read(d.path().join(".git/HEAD")).unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let config = fs::read(d.path().join(".git/config")).unwrap();
        let forward = read(
            &mut session,
            &snapshot.revision,
            base,
            &target,
            Some(&file_id("hello.txt")),
        )
        .unwrap();
        let patch = forward.diff.unwrap().text;
        assert!(patch.contains("-hello"));
        assert!(patch.contains("+target"));
        assert!(!patch.contains("dirty work"));
        assert_eq!(
            forward
                .files
                .iter()
                .find(|f| f.path == "new.txt")
                .unwrap()
                .status,
            "added"
        );
        let reverse = read(
            &mut session,
            &snapshot.revision,
            &target,
            base,
            Some(&file_id("hello.txt")),
        )
        .unwrap();
        let patch = reverse.diff.unwrap().text;
        assert!(patch.contains("-target"));
        assert!(patch.contains("+hello"));
        assert_eq!(
            reverse
                .files
                .iter()
                .find(|f| f.path == "new.txt")
                .unwrap()
                .status,
            "deleted"
        );
        assert!(read(&mut session, &snapshot.revision, base, base, None)
            .unwrap()
            .files
            .is_empty());
        for oid in ["HEAD", "0000000000000000000000000000000000000000"] {
            assert!(read(&mut session, &snapshot.revision, oid, &target, None).is_err());
        }
        assert!(read(
            &mut session,
            &snapshot.revision,
            base,
            &target,
            Some("forged")
        )
        .is_err());
        assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(d.path().join(".git/config")).unwrap(), config);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "dirty work\n"
        );
        fs::write(d.path().join("hello.txt"), "changed again").unwrap();
        assert!(read(&mut session, &snapshot.revision, base, &target, None)
            .unwrap_err()
            .contains("STALE"));
    }
    #[test]
    fn comparison_literal_paths_binary_deleted_and_hidden_commit_scope() {
        let d = fixture();
        let repo = Repository::open(d.path()).unwrap();
        let base = repo.head().unwrap().peel_to_commit().unwrap();
        let mut index = repo.index().unwrap();
        let mut e = index.get_path(Path::new("hello.txt"), 0).unwrap();
        index.remove_path(Path::new("hello.txt")).unwrap();
        for (path, bytes) in [
            ("special[1].txt", b"literal\n".as_slice()),
            ("special1.txt", b"must not leak\n"),
            ("binary.dat", b"\0\x01\0\x02"),
        ] {
            e.path = path.as_bytes().to_vec();
            e.id = repo.blob(bytes).unwrap();
            e.file_size = bytes.len() as u32;
            index.add(&e).unwrap();
        }
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let sig = git2::Signature::now("Compare", "compare@example.invalid").unwrap();
        let target = repo
            .commit(None, &sig, &sig, "Comparison target", &tree, &[&base])
            .unwrap();
        repo.reference("refs/heads/compare", target, false, "fixture")
            .unwrap();
        let hidden = repo.commit(None, &sig, &sig, "Hidden", &tree, &[]).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let base = base.id().to_string();
        let target = target.to_string();
        let literal = read(
            &mut session,
            &snapshot.revision,
            &base,
            &target,
            Some(&file_id("special[1].txt")),
        )
        .unwrap();
        let patch = literal.diff.unwrap().text;
        assert!(patch.contains("+literal"));
        assert!(!patch.contains("must not leak"));
        let deleted = read(
            &mut session,
            &snapshot.revision,
            &base,
            &target,
            Some(&file_id("hello.txt")),
        )
        .unwrap();
        assert!(deleted.diff.unwrap().text.contains("-hello"));
        let binary = read(
            &mut session,
            &snapshot.revision,
            &base,
            &target,
            Some(&file_id("binary.dat")),
        )
        .unwrap();
        assert!(!binary.diff.unwrap().text.contains('\0'));
        assert!(read(
            &mut session,
            &snapshot.revision,
            &hidden.to_string(),
            &target,
            None
        )
        .is_err());
    }
    #[test]
    fn comparison_bounds_file_count_and_patch_output() {
        let d = fixture();
        let repo = Repository::open(d.path()).unwrap();
        let base = repo.head().unwrap().peel_to_commit().unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        let mut index = repo.index().unwrap();
        let mut entry = index.get_path(Path::new("hello.txt"), 0).unwrap();
        let bytes = b"line\n".repeat(150000);
        entry.id = repo.blob(&bytes).unwrap();
        entry.file_size = bytes.len() as u32;
        index.add(&entry).unwrap();
        let sig = git2::Signature::now("Compare", "compare@example.invalid").unwrap();
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let large = repo
            .commit(None, &sig, &sig, "Large patch", &tree, &[&base])
            .unwrap();
        repo.reference("refs/heads/large", large, false, "fixture")
            .unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let result = read(
            &mut session,
            &snapshot.revision,
            &base.id().to_string(),
            &large.to_string(),
            Some(&file_id("hello.txt")),
        )
        .unwrap()
        .diff
        .unwrap();
        assert!(result.truncated);
        assert!(result.text.len() <= 512 * 1024);
        entry.id = repo.blob(b"x\n").unwrap();
        entry.file_size = 2;
        for n in 0..=MAX_FILES {
            entry.path = format!("file-{n:04}.txt").into_bytes();
            index.add(&entry).unwrap();
        }
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let many = repo
            .commit(None, &sig, &sig, "Too many", &tree, &[&base])
            .unwrap();
        repo.reference("refs/heads/many", many, false, "fixture")
            .unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(read(
            &mut session,
            &snapshot.revision,
            &base.id().to_string(),
            &many.to_string(),
            None
        )
        .unwrap_err()
        .contains("LIMIT"));
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
    }
}
