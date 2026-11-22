//! Bounded first-parent file history, following exact-content renames.
use super::*;
const PAGE: usize = 100;
const MAX_SCAN: usize = 10_000;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub oid: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
    pub path: String,
    pub previous_path: Option<String>,
    pub status: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistory {
    pub path: String,
    pub entries: Vec<HistoryEntry>,
    pub offset: usize,
    pub scanned: usize,
    pub next_offset: Option<usize>,
    pub limit_reached: bool,
    pub diff: Option<DiffResult>,
}
pub(super) fn read(session:&mut Session,revision:&str,id:&str,anchor:Option<&str>,offset:usize,selected:Option<&str>)->Result<FileHistory>{
    read_impl(session,revision,id,anchor,offset,selected,None)
}
impl Session {
    pub fn tree_file_history(&mut self,revision:&str,oid:&str,path:&str,offset:usize,selected:Option<&str>)->Result<FileHistory>{
        super::tree_browser::authorize(self,revision,oid,path)?;
        if path.is_empty()||selected.is_some_and(|value|value.len()!=40||!value.bytes().all(|c|c.is_ascii_hexdigit())){return Err("INVALID_HISTORY_ENTRY".into());}
        let repo=open(&self.root)?;
        let tree=repo.find_commit(Oid::from_str(oid).map_err(err)?).map_err(err)?.tree().map_err(err)?;
        let entry=tree.get_path(Path::new(path)).map_err(err)?;
        if !matches!(entry.filemode(),0o100644|0o100755){return Err("UNSUPPORTED_FILE_MODE".into());}
        read_impl(self,revision,&file_id(path),Some(oid),offset,selected,Some(path))
    }
}
fn read_impl(
    session: &mut Session,
    revision: &str,
    id: &str,
    anchor: Option<&str>,
    offset: usize,
    selected: Option<&str>,
    tree_path: Option<&str>,
) -> Result<FileHistory> {
    if offset >= MAX_SCAN || !offset.is_multiple_of(PAGE) {
        return Err("INVALID_HISTORY_PAGE".into());
    }
    session.fresh(revision)?;
    let original = if let Some(path)=tree_path {path.to_owned()} else if let Some(oid) = anchor {
        session
            .commit_detail(revision, oid, None)?
            .files
            .into_iter()
            .find(|f| f.id == id)
            .ok_or("INVALID_FILE_ID")?
            .path
    } else {
        session
            .files
            .get(id)
            .and_then(|p| p.to_str())
            .ok_or("INVALID_FILE_ID")?
            .to_string()
    };
    let repo = open(&session.root)?;
    let start = if let Some(oid) = anchor {
        Oid::from_str(oid).map_err(err)?
    } else {
        repo.head()
            .map_err(err)?
            .peel_to_commit()
            .map_err(err)?
            .id()
    };
    let mut current = Some(repo.find_commit(start).map_err(err)?);
    let mut path = original.clone();
    let mut result = Vec::new();
    let mut scanned = 0;
    let mut patch = None;
    while let Some(commit) = current {
        let parent = if commit.parent_count() > 0 {
            Some(commit.parent(0).map_err(err)?)
        } else {
            None
        };
        let old = parent.as_ref().map(|p| p.tree()).transpose().map_err(err)?;
        let tree = commit.tree().map_err(err)?;
        let mut options = DiffOptions::new();
        options.max_size(FILE_LIMIT as i64);
        let mut diff = repo
            .diff_tree_to_tree(old.as_ref(), Some(&tree), Some(&mut options))
            .map_err(err)?;
        if diff.deltas().len() > MAX_FILES {
            return Err("LIMIT: history commit files".into());
        }
        let mut find = git2::DiffFindOptions::new();
        find.renames(true).exact_match_only(true);
        diff.find_similar(Some(&mut find)).map_err(err)?;
        let changed = diff.deltas().enumerate().find(|(_, d)| {
            d.new_file()
                .path()
                .or_else(|| d.old_file().path())
                .and_then(Path::to_str)
                == Some(path.as_str())
                || (scanned == 0 && delta_is_old_rename(d, &path))
        });
        let mut stop = false;
        if let Some((_, delta)) = changed {
            let previous = if delta.status() == git2::Delta::Renamed {
                delta
                    .old_file()
                    .path()
                    .and_then(Path::to_str)
                    .map(str::to_string)
            } else {
                None
            };
            let display_path = if previous.is_some() {
                delta
                    .new_file()
                    .path()
                    .and_then(Path::to_str)
                    .unwrap_or(&path)
                    .to_string()
            } else {
                path.clone()
            };
            if scanned >= offset {
                let oid = commit.id().to_string();
                if selected == Some(oid.as_str()) {
                    let mut opts = DiffOptions::new();
                    opts.pathspec(&display_path)
                        .disable_pathspec_match(true)
                        .max_size(FILE_LIMIT as i64);
                    if let Some(old_path) = &previous {
                        opts.pathspec(old_path);
                    }
                    let mut file_diff = repo
                        .diff_tree_to_tree(old.as_ref(), Some(&tree), Some(&mut opts))
                        .map_err(err)?;
                    file_diff.find_similar(Some(&mut find)).map_err(err)?;
                    patch = Some(render_diff(&file_diff)?);
                }
                result.push(HistoryEntry {
                    oid,
                    summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                        .chars()
                        .take(1000)
                        .collect(),
                    author: commit
                        .author()
                        .name()
                        .unwrap_or("Unknown")
                        .chars()
                        .take(256)
                        .collect(),
                    time: commit.time().seconds(),
                    path: display_path,
                    previous_path: previous.clone(),
                    status: match delta.status() {
                        git2::Delta::Added => "added",
                        git2::Delta::Deleted => "deleted",
                        git2::Delta::Renamed => "renamed",
                        _ => "modified",
                    }
                    .into(),
                });
            }
            if let Some(previous) = previous {
                path = previous;
            }
            // A newly added path begins a new identity; do not mix an older,
            // unrelated deleted file that happened to use the same pathname.
            if delta.status() == git2::Delta::Added {
                stop = true;
            }
        }
        scanned += 1;
        current = if stop { None } else { parent };
        if scanned >= offset + PAGE || current.is_none() {
            break;
        }
    }
    if selected.is_some() && patch.is_none() {
        return Err("INVALID_HISTORY_ENTRY".into());
    }
    Ok(FileHistory {
        path: original,
        entries: result,
        offset,
        scanned: scanned.saturating_sub(offset),
        next_offset: if current.is_some() && scanned < MAX_SCAN {
            Some(scanned)
        } else {
            None
        },
        limit_reached: current.is_some() && scanned >= MAX_SCAN,
        diff: patch,
    })
}

fn delta_is_old_rename(delta: &git2::DiffDelta<'_>, path: &str) -> bool {
    delta.status() == git2::Delta::Renamed
        && delta.old_file().path().and_then(Path::to_str) == Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(root: &Path, message: &str) -> String {
        let repo = Repository::open(root).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.update_all(["*"], None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
            .to_string()
    }
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        Repository::init(d.path()).unwrap();
        fs::write(d.path().join("hello.txt"), "hello\n").unwrap();
        commit(d.path(), "Initial");
        d
    }
    #[test]
    fn deletion_and_exact_rename_history_are_read_only() {
        let d = fixture();
        fs::rename(d.path().join("hello.txt"), d.path().join("new[1].txt")).unwrap();
        let renamed = commit(d.path(), "Rename");
        fs::write(d.path().join("new[1].txt"), "changed\n").unwrap();
        commit(d.path(), "Modify");
        fs::remove_file(d.path().join("new[1].txt")).unwrap();
        let deleted = commit(d.path(), "Delete");
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        let page = read(
            &mut session,
            &snapshot.revision,
            &file_id("new[1].txt"),
            Some(&deleted),
            0,
            Some(&renamed),
        )
        .unwrap();
        assert_eq!(
            page.entries
                .iter()
                .map(|e| e.status.as_str())
                .collect::<Vec<_>>(),
            ["deleted", "modified", "renamed", "added"]
        );
        assert_eq!(page.scanned, 4);
        assert_eq!(page.next_offset, None);
        assert_eq!(page.entries[2].previous_path.as_deref(), Some("hello.txt"));
        assert!(page.diff.unwrap().text.contains("rename from hello.txt"));
        let old = read(
            &mut session,
            &snapshot.revision,
            &file_id("hello.txt"),
            Some(&renamed),
            0,
            None,
        )
        .unwrap();
        assert_eq!(old.entries.len(), 2);
        assert_eq!(old.entries[0].path, "new[1].txt");
        assert_eq!(before, fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(snapshot.revision, session.snapshot().unwrap().revision);
        assert!(!d.path().join("new[1].txt").exists());
    }
    #[test]
    fn pages_and_selection_are_bounded_and_validated() {
        let d = fixture();
        for n in 0..102 {
            fs::write(d.path().join("hello.txt"), format!("revision {n}\n")).unwrap();
            commit(d.path(), "Edit");
        }
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let anchor = &snapshot.commits[0].oid;
        let id = file_id("hello.txt");
        let first = read(&mut session, &snapshot.revision, &id, Some(anchor), 0, None).unwrap();
        let second = read(
            &mut session,
            &snapshot.revision,
            &id,
            Some(anchor),
            100,
            None,
        )
        .unwrap();
        assert_eq!(first.entries.len(), 100);
        assert_eq!(first.next_offset, Some(100));
        assert_eq!(second.entries.len(), 3);
        assert_eq!(second.scanned, 3);
        assert_eq!(second.next_offset, None);
        assert!(!first
            .entries
            .iter()
            .any(|a| second.entries.iter().any(|b| a.oid == b.oid)));
        assert!(read(
            &mut session,
            &snapshot.revision,
            &id,
            Some(anchor),
            0,
            Some(&second.entries[0].oid)
        )
        .unwrap_err()
        .contains("INVALID_HISTORY_ENTRY"));
        for offset in [1, 99, 10_000, usize::MAX] {
            assert!(read(
                &mut session,
                &snapshot.revision,
                &id,
                Some(anchor),
                offset,
                None
            )
            .is_err());
        }
        assert!(read(
            &mut session,
            &snapshot.revision,
            "../../secret",
            Some(anchor),
            0,
            None
        )
        .is_err());
        assert!(read(&mut session, &snapshot.revision, &id, Some("HEAD"), 0, None).is_err());
        fs::write(d.path().join("hello.txt"), "external edit").unwrap();
        assert!(
            read(&mut session, &snapshot.revision, &id, Some(anchor), 0, None)
                .unwrap_err()
                .starts_with("STALE_STATE")
        );
    }
    #[test]
    fn readded_path_starts_a_new_identity_and_wip_uses_head() {
        let d = fixture();
        fs::remove_file(d.path().join("hello.txt")).unwrap();
        commit(d.path(), "Delete");
        fs::write(d.path().join("hello.txt"), "new identity\n").unwrap();
        let added = commit(d.path(), "Recreate");
        fs::write(d.path().join("hello.txt"), "uncommitted\n").unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let page = read(
            &mut session,
            &snapshot.revision,
            &snapshot.changes[0].id,
            None,
            0,
            Some(&added),
        )
        .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].status, "added");
        let patch = page.diff.unwrap().text;
        assert!(patch.contains("+new identity"));
        assert!(!patch.contains("uncommitted"));
    }
}
