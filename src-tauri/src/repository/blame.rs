//! Committed-file attribution. No worktree overlays or repository writes.
use super::*;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameHunk {
    pub start: usize,
    pub lines: usize,
    pub oid: String,
    pub author: String,
    pub time: i64,
    pub original_path: String,
    pub original_start: usize,
    pub boundary: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBlame {
    pub oid: String,
    pub path: String,
    pub lines: Vec<String>,
    pub hunks: Vec<BlameHunk>,
    pub history_limited: bool,
}
pub(super) fn read(
    session: &mut Session,
    revision: &str,
    id: &str,
    anchor: Option<&str>,
    offset: usize,
    selected: &str,
) -> Result<FileBlame> {
    let page = history::read(session, revision, id, anchor, offset, None)?;
    let entry = page
        .entries
        .into_iter()
        .find(|e| e.oid == selected)
        .ok_or("INVALID_HISTORY_ENTRY")?;
    if entry.status == "deleted" {
        return Err("FILE_DELETED: 이 커밋에서는 파일이 삭제되었습니다.".into());
    }
    read_path(session,revision,selected,entry.path)
}
impl Session {
    pub fn tree_file_blame(&mut self,revision:&str,oid:&str,path:&str)->Result<FileBlame>{
        tree_browser::authorize(self,revision,oid,path)?;
        if path.is_empty(){return Err("INVALID_PATH".into());}
        read_path(self,revision,oid,path.into())
    }
}
fn read_path(session:&mut Session,revision:&str,selected:&str,path:String)->Result<FileBlame>{
    let repo = open(&session.root)?;
    let oid = Oid::from_str(selected).map_err(err)?;
    let commit = repo.find_commit(oid).map_err(err)?;
    let tree = commit.tree().map_err(err)?;
    let file = tree.get_path(Path::new(&path)).map_err(err)?;
    if !matches!(file.filemode(), 0o100644 | 0o100755) {
        return Err("UNSUPPORTED_BLAME_FILE".into());
    }
    let blob = repo.find_blob(file.id()).map_err(err)?;
    if blob.size() > FILE_LIMIT as usize {
        return Err("LIMIT: blame file exceeds 5 MiB".into());
    }
    if blob.is_binary() {
        return Err("BINARY_BLAME: 이진 파일은 줄별 이력을 표시하지 않습니다.".into());
    }
    let content = std::str::from_utf8(blob.content())
        .map_err(|_| "UNSUPPORTED_BLAME_ENCODING: UTF-8 only")?;
    let lines: Vec<_> = content
        .split_terminator('\n')
        .take(10_001)
        .map(str::to_string)
        .collect();
    if lines.len() > 10_000 {
        return Err("LIMIT: blame supports at most 10000 lines".into());
    }
    // Explicit ancestry boundary prevents unbounded historical traversal.
    let mut oldest = commit;
    let mut count = 1;
    while oldest.parent_count() > 0 && count < 10_000 {
        oldest = oldest.parent(0).map_err(err)?;
        count += 1;
    }
    let history_limited = oldest.parent_count() > 0;
    let mut options = git2::BlameOptions::new();
    options
        .newest_commit(oid)
        .oldest_commit(oldest.id())
        .first_parent(true)
        .use_mailmap(false);
    let mut hunks = Vec::new();
    if !lines.is_empty() {
        let blame = repo
            .blame_file(Path::new(&path), Some(&mut options))
            .map_err(err)?;
        for hunk in blame.iter() {
            let signature = hunk.final_signature();
            hunks.push(BlameHunk {
                start: hunk.final_start_line(),
                lines: hunk.lines_in_hunk(),
                oid: hunk.final_commit_id().to_string(),
                author: signature
                    .as_ref()
                    .and_then(|s| s.name().ok())
                    .unwrap_or("Unknown")
                    .chars()
                    .take(256)
                    .collect(),
                time: signature.as_ref().map(|s| s.when().seconds()).unwrap_or(0),
                original_path: hunk
                    .path()
                    .and_then(Path::to_str)
                    .unwrap_or(&path)
                    .to_string(),
                original_start: hunk.orig_start_line(),
                boundary: hunk.is_boundary(),
            });
        }
    }
    session.fresh(revision)?;
    Ok(FileBlame {
        oid: selected.into(),
        path,
        lines,
        hunks,
        history_limited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(root: &Path, author: &str, bytes: Option<&[u8]>) -> String {
        let repo = open(root).unwrap();
        let mut index = repo.index().unwrap();
        if let Some(bytes) = bytes {
            fs::write(root.join("file.txt"), bytes).unwrap();
            index.add_path(Path::new("file.txt")).unwrap();
        } else {
            fs::remove_file(root.join("file.txt")).unwrap();
            index.remove_path(Path::new("file.txt")).unwrap();
        }
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        let sig = git2::Signature::now(author, "test@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Edit", &tree, &parents)
            .unwrap()
            .to_string()
    }
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        Repository::init(d.path()).unwrap();
        d
    }
    #[test]
    fn tree_blame_accepts_unchanged_anchor_and_rejects_out_of_scope_paths(){
        let d=fixture();
        let first=commit(d.path(),"Alice",Some(b"first\nsecond\n"));
        let anchor=commit(d.path(),"Bob",Some(b"first\nsecond\n"));
        fs::write(d.path().join("file.txt"),"local edit").unwrap();
        let index=fs::read(d.path().join(".git/index")).unwrap();
        let mut session=Session::new(d.path().into()).unwrap();let snapshot=session.snapshot().unwrap();
        let result=session.tree_file_blame(&snapshot.revision,&anchor,"file.txt").unwrap();
        assert_eq!(result.lines,["first","second"]);
        assert!(result.hunks.iter().all(|hunk|hunk.oid==first));
        for path in ["","../file.txt","/file.txt","missing.txt"]{assert!(session.tree_file_blame(&snapshot.revision,&anchor,path).is_err());}
        assert!(session.tree_file_blame(&snapshot.revision,&"0".repeat(40),"file.txt").is_err());
        assert_eq!(index,fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(fs::read_to_string(d.path().join("file.txt")).unwrap(),"local edit");
    }
    #[test]
    fn attributes_each_line_to_its_commit_and_ignores_worktree() {
        let d = fixture();
        let first = commit(d.path(), "Alice", Some(b"first\nsecond\nthird\n"));
        let second = commit(d.path(), "Bob", Some(b"first\nchanged\nthird\n"));
        fs::write(d.path().join("file.txt"), "external worktree").unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let result = read(
            &mut session,
            &snapshot.revision,
            &file_id("file.txt"),
            Some(&second),
            0,
            &second,
        )
        .unwrap();
        assert_eq!(result.lines, ["first", "changed", "third"]);
        assert!(!result.history_limited);
        assert_eq!(result.hunks.len(), 3);
        assert_eq!(
            result
                .hunks
                .iter()
                .map(|h| h.oid.as_str())
                .collect::<Vec<_>>(),
            [&first, &second, &first]
        );
        assert_eq!(
            result
                .hunks
                .iter()
                .map(|h| h.author.as_str())
                .collect::<Vec<_>>(),
            ["Alice", "Bob", "Alice"]
        );
        assert_eq!(
            result
                .hunks
                .iter()
                .map(|h| (h.start, h.lines))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 1), (3, 1)]
        );
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read_to_string(d.path().join("file.txt")).unwrap(),
            "external worktree"
        );
        assert_eq!(session.snapshot().unwrap().revision, snapshot.revision);
        let old = read(
            &mut session,
            &snapshot.revision,
            &file_id("file.txt"),
            Some(&second),
            0,
            &first,
        )
        .unwrap();
        assert_eq!(old.lines[1], "second");
        assert_eq!(old.hunks[0].lines, 3);
    }
    #[test]
    fn rejects_deleted_binary_non_utf8_oversized_and_invalid_selection() {
        for (bytes, error) in [
            (vec![0, 1, 2], "BINARY_BLAME"),
            (b"hello \xff world\n".to_vec(), "UNSUPPORTED_BLAME_ENCODING"),
            (vec![b'x'; FILE_LIMIT as usize + 1], "LIMIT"),
            ("x\n".repeat(10_001).into_bytes(), "LIMIT"),
        ] {
            let d = fixture();
            let oid = commit(d.path(), "Test", Some(&bytes));
            let mut session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let failure = read(
                &mut session,
                &snapshot.revision,
                &file_id("file.txt"),
                Some(&oid),
                0,
                &oid,
            )
            .unwrap_err();
            assert!(failure.contains(error), "expected {error}: {failure}");
        }
        let d = fixture();
        commit(d.path(), "Test", Some(b"content"));
        let oid = commit(d.path(), "Test", None);
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(read(
            &mut session,
            &snapshot.revision,
            &file_id("file.txt"),
            Some(&oid),
            0,
            &oid
        )
        .unwrap_err()
        .starts_with("FILE_DELETED"));
        assert!(read(
            &mut session,
            &snapshot.revision,
            &file_id("file.txt"),
            Some(&oid),
            0,
            "HEAD"
        )
        .unwrap_err()
        .contains("INVALID_HISTORY_ENTRY"));
        assert!(read(
            &mut session,
            &snapshot.revision,
            "../file.txt",
            Some(&oid),
            0,
            &oid
        )
        .is_err());
        fs::write(d.path().join("new.txt"), "external").unwrap();
        assert!(read(
            &mut session,
            &snapshot.revision,
            &file_id("file.txt"),
            Some(&oid),
            0,
            &oid
        )
        .unwrap_err()
        .starts_with("STALE_STATE"));
    }
    #[test]
    fn exact_rename_preserves_original_author_and_path() {
        let d = fixture();
        let original = commit(d.path(), "Alice", Some(b"line one\nline two\n"));
        let repo = open(d.path()).unwrap();
        fs::rename(d.path().join("file.txt"), d.path().join("renamed.txt")).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("file.txt")).unwrap();
        index.add_path(Path::new("renamed.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.find_commit(Oid::from_str(&original).unwrap()).unwrap();
        let sig = git2::Signature::now("Bob", "test@example.invalid").unwrap();
        let renamed = repo
            .commit(Some("HEAD"), &sig, &sig, "Rename", &tree, &[&parent])
            .unwrap()
            .to_string();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let result = read(
            &mut session,
            &snapshot.revision,
            &file_id("renamed.txt"),
            Some(&renamed),
            0,
            &renamed,
        )
        .unwrap();
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(result.hunks[0].oid, original);
        assert_eq!(result.hunks[0].author, "Alice");
        assert_eq!(result.hunks[0].original_path, "file.txt");
    }
    #[test]
    fn empty_file_and_unterminated_last_line_are_preserved() {
        for (bytes, lines) in [
            ("", vec![]),
            ("\n", vec![""]),
            ("hello\nlast", vec!["hello", "last"]),
            ("a\r\n", vec!["a\r"]),
        ] {
            let d = fixture();
            let oid = commit(d.path(), "Test", Some(bytes.as_bytes()));
            let mut session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let result = read(
                &mut session,
                &snapshot.revision,
                &file_id("file.txt"),
                Some(&oid),
                0,
                &oid,
            )
            .unwrap();
            assert_eq!(result.lines, lines);
            assert_eq!(
                result.hunks.iter().map(|h| h.lines).sum::<usize>(),
                lines.len()
            );
        }
    }
}
