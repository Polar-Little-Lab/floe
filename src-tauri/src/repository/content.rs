//! Literal, scoped file reads from working tree, index, commit, history or stash.
use super::*;
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ContentSource {
    ReplayConflict {
        oid: String,
        operation: replay::Kind,
        mainline: Option<usize>,
        conflict: usize,
        side: MergeSide,
    },
    IndexConflict {
        side: MergeSide,
    },
    MergeConflict {
        name: String,
        conflict: usize,
        side: MergeSide,
    },
    Comparison {base:String,target:String,before:bool},
    TreeHistory {anchor:String,path:String,offset:usize,selected:String,before:bool},
    TreeFile {oid:String,path:String},
    Head {},
    CommitBefore { oid: String },
    HistoryBefore { anchor: Option<String>, offset: usize, selected: String },
    StashBefore { oid: String, area: StashArea },
    Worktree {},
    Index {},
    Commit {
        oid: String,
    },
    History {
        anchor: Option<String>,
        offset: usize,
        selected: String,
    },
    Stash {
        oid: String,
        area: StashArea,
    },
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeSide {
    Ancestor,
    Ours,
    Theirs,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lfs_object: Option<lfs::ObjectStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lfs_upload_record: Option<lfs::upload_state::Record>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lfs_upload_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImagePreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_error: Option<String>,
    pub path: String,
    pub source: String,
    pub state: String,
    pub size: u64,
    pub text: String,
    pub truncated: bool,
    pub ends_with_newline: bool,
}
#[derive(Debug, Serialize)]
pub struct ImagePreview {pub mime:String,pub data:String,pub width:usize,pub height:usize}
fn image_preview(bytes:&[u8])->std::result::Result<Option<ImagePreview>,String>{
    if bytes.len()>FILE_LIMIT as usize{return Ok(None);}
    let mime=if bytes.starts_with(b"\x89PNG\r\n\x1a\n"){"image/png"}
        else if bytes.starts_with(&[0xff,0xd8,0xff]){"image/jpeg"}
        else if bytes.starts_with(b"GIF87a")||bytes.starts_with(b"GIF89a"){"image/gif"}
        else if bytes.starts_with(b"RIFF")&&bytes.get(8..12)==Some(b"WEBP"){"image/webp"}
        else{return Ok(None);};
    let size=imagesize::blob_size(bytes).map_err(|_|"Image dimensions could not be read".to_string())?;
    if size.width==0||size.height==0||size.width>8192||size.height>8192||size.width.checked_mul(size.height).is_none_or(|pixels|pixels>16_000_000){return Err("Image preview limit: 8,192 pixels per side and 16 million pixels total".into());}
    use base64::Engine;
    Ok(Some(ImagePreview{mime:mime.into(),data:base64::engine::general_purpose::STANDARD.encode(bytes),width:size.width,height:size.height}))
}
fn empty(path: String, source: String, state: &str, size: u64) -> FileContent {
    FileContent {
        lfs_object: None,
        lfs_upload_record: None,
        lfs_upload_error: None,
        image: None,
        image_error: None,
        path,
        source,
        state: state.into(),
        size,
        text: String::new(),
        truncated: false,
        ends_with_newline: false,
    }
}
fn decode(path: String, source: String, bytes: &[u8]) -> FileContent {
    let mut result = empty(path, source, "text", bytes.len() as u64);
    match image_preview(bytes){
        Ok(Some(image))=>{result.state="binary".into();result.image=Some(image);return result;},
        Err(reason)=>{result.state="binary".into();result.image_error=Some(reason);return result;},
        Ok(None)=>{},
    }
    if bytes.contains(&0) {
        result.state = "binary".into();
        return result;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        result.state = "unsupportedEncoding".into();
        return result;
    };
    result.ends_with_newline = text.ends_with('\n');
    let mut lines = 0;
    for ch in text.chars() {
        if result.text.len() + ch.len_utf8() > 512 * 1024 || lines >= 10_000 {
            result.truncated = true;
            break;
        }
        result.text.push(ch);
        if ch == '\n' {
            lines += 1;
        }
    }
    result
}
fn blob(
    repo: &Repository,
    path: String,
    source: String,
    oid: Oid,
    mode: u32,
) -> Result<FileContent> {
    if !matches!(mode, 0o100644 | 0o100755) {
        return Err("UNSUPPORTED_FILE_MODE".into());
    }
    let (size, _) = repo.odb().map_err(err)?.read_header(oid).map_err(err)?;
    if size > FILE_LIMIT as usize {
        return Ok(empty(path, source, "tooLarge", size as u64));
    }
    let data = repo.find_blob(oid).map_err(err)?;
    Ok(decode(path, source, data.content()))
}
fn tree_file(repo: &Repository, oid: &str, path: String, source: String) -> Result<FileContent> {
    let tree = repo
        .find_commit(Oid::from_str(oid).map_err(err)?)
        .map_err(err)?
        .tree()
        .map_err(err)?;
    match tree.get_path(Path::new(&path)) {
        Ok(entry) => blob(repo, path, source, entry.id(), entry.filemode() as u32),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(empty(path, source, "deleted", 0)),
        Err(e) => Err(err(e)),
    }
}
pub(super) fn read(
    session: &mut Session,
    revision: &str,
    id: &str,
    source: ContentSource,
) -> Result<FileContent> {
    session.fresh(revision)?;
    let repo = open(&session.root)?;
    let before=matches!(&source,ContentSource::CommitBefore{..}|ContentSource::HistoryBefore{..}|ContentSource::StashBefore{..});
    match source {
        ContentSource::Comparison {base,target,before}=>{
            let comparison=super::compare::read(session,revision,&base,&target,None)?;
            let file=comparison.files.into_iter().find(|file|file.id==id).ok_or("INVALID_FILE_ID")?;
            let oid=if before{&base}else{&target};
            let result=tree_file(&repo,oid,file.path,format!("Comparison {} {}",if before{"Before"}else{"After"},&oid[..7]))?;
            session.fresh(revision)?;Ok(result)
        }
        ContentSource::TreeHistory {anchor,path,offset,selected,before}=>{
            super::tree_browser::authorize(session,revision,&anchor,&path)?;
            if path.is_empty()||file_id(&path)!=id{return Err("INVALID_FILE_ID".into());}
            let file=session.tree_file_history(revision,&anchor,&path,offset,Some(&selected))?.entries.into_iter().find(|entry|entry.oid==selected).ok_or("INVALID_HISTORY_ENTRY")?;
            let result=if before {
                let commit=repo.find_commit(Oid::from_str(&selected).map_err(err)?).map_err(err)?;
                let path=file.previous_path.unwrap_or(file.path);
                if commit.parent_count()==0{empty(path,"Before initial commit".into(),"deleted",0)}else{let parent=commit.parent_id(0).map_err(err)?.to_string();tree_file(&repo,&parent,path,format!("History first parent {}",&parent[..7]))?}
            }else{tree_file(&repo,&selected,file.path,format!("History commit {}",&selected[..7]))?};
            session.fresh(revision)?;Ok(result)
        }
        ContentSource::TreeFile {oid,path}=>{
            super::tree_browser::authorize(session,revision,&oid,&path)?;
            if path.is_empty()||file_id(&path)!=id{return Err("INVALID_FILE_ID".into());}
            let result=tree_file(&repo,&oid,path,format!("Commit tree {}",&oid[..7]))?;
            session.fresh(revision)?;Ok(result)
        }
        ContentSource::ReplayConflict {
            oid,
            operation,
            mainline,
            conflict,
            side,
        } => {
            if oid.len() != 40
                || !oid.bytes().all(|b| b.is_ascii_hexdigit())
                || conflict >= MAX_FILES
            {
                return Err("INVALID_REPLAY_SELECTION".into());
            }
            // Recompute membership and stages; caller never chooses an arbitrary blob OID.
            let (plan, scratch, _temp) =
                replay::calculate(session, revision, &oid, operation, mainline)?;
            let selected = plan
                .conflicts
                .get(conflict)
                .ok_or("INVALID_REPLAY_CONFLICT")?;
            let (version, label) = match side {
                MergeSide::Ancestor => (&selected.ancestor, "Base"),
                MergeSide::Ours => (&selected.ours, "Current"),
                MergeSide::Theirs => (&selected.theirs, "Incoming"),
            };
            let version = version.as_ref().ok_or("ABSENT_REPLAY_VERSION")?;
            if file_id(&version.path) != id {
                return Err("INVALID_FILE_ID".into());
            }
            let result = blob(
                &scratch,
                version.path.clone(),
                format!("Replay {label} · {}", &version.oid[..7]),
                Oid::from_str(&version.oid).map_err(err)?,
                0o100644,
            )?;
            session.fresh(revision)?;
            Ok(result)
        }
        ContentSource::IndexConflict { side } => {
            let path = session
                .files
                .get(id)
                .and_then(|p| p.to_str())
                .ok_or("INVALID_FILE_ID")?
                .to_string();
            let index = repo.index().map_err(err)?;
            let stage = match side {
                MergeSide::Ancestor => 1,
                MergeSide::Ours => 2,
                MergeSide::Theirs => 3,
            };
            let entry = index
                .get_path(Path::new(&path), stage)
                .ok_or("ABSENT_CONFLICT_STAGE")?;
            let result = blob(
                &repo,
                path,
                format!("Index stage {stage}"),
                entry.id,
                entry.mode,
            )?;
            session.fresh(revision)?;
            Ok(result)
        }

        ContentSource::MergeConflict {
            name,
            conflict,
            side,
        } => {
            if name.len() > 1031 || conflict >= MAX_FILES {
                return Err("INVALID_MERGE_SELECTION".into());
            }
            let plan = merge::preview(session, revision, &name)?;
            let selected = plan
                .conflicts
                .get(conflict)
                .ok_or("INVALID_MERGE_CONFLICT")?;
            let (version, label) = match side {
                MergeSide::Ancestor => (&selected.ancestor, "Base"),
                MergeSide::Ours => (&selected.ours, "Current"),
                MergeSide::Theirs => (&selected.theirs, "Incoming"),
            };
            let version = version.as_ref().ok_or("ABSENT_MERGE_VERSION")?;
            if file_id(&version.path) != id {
                return Err("INVALID_FILE_ID".into());
            }
            // OID is obtained only from a freshly recomputed merge, never supplied by the caller.
            let result = blob(
                &repo,
                version.path.clone(),
                format!("Merge {label} · {}", &version.oid[..7]),
                Oid::from_str(&version.oid).map_err(err)?,
                0o100644,
            )?;
            session.fresh(revision)?;
            Ok(result)
        }

        ContentSource::Commit { oid } | ContentSource::CommitBefore { oid } => {
            let file = session
                .commit_detail(revision, &oid, None)?
                .files
                .into_iter()
                .find(|f| f.id == id)
                .ok_or("INVALID_FILE_ID")?;
            if before {
                let commit=repo.find_commit(Oid::from_str(&oid).map_err(err)?).map_err(err)?;
                let result=if commit.parent_count()==0{Ok(empty(file.path,"Before initial commit".into(),"deleted",0))}else{let parent=commit.parent_id(0).map_err(err)?.to_string();tree_file(&repo,&parent,file.path,format!("First parent {}",&parent[..7]))};
                session.fresh(revision)?;return result;
            }
            tree_file(&repo, &oid, file.path, format!("Commit {}", &oid[..7]))
        }
        ContentSource::History {
            anchor,
            offset,
            selected,
        } | ContentSource::HistoryBefore {anchor,offset,selected} => {
            let file = history::read(session, revision, id, anchor.as_deref(), offset, None)?
                .entries
                .into_iter()
                .find(|f| f.oid == selected)
                .ok_or("INVALID_HISTORY_ENTRY")?;
            if before {
                let commit=repo.find_commit(Oid::from_str(&selected).map_err(err)?).map_err(err)?;
                let path=file.previous_path.unwrap_or(file.path);
                let result=if commit.parent_count()==0{Ok(empty(path,"Before initial commit".into(),"deleted",0))}else{let parent=commit.parent_id(0).map_err(err)?.to_string();tree_file(&repo,&parent,path,format!("History first parent {}",&parent[..7]))};
                session.fresh(revision)?;return result;
            }
            tree_file(
                &repo,
                &selected,
                file.path,
                format!("Commit {}", &selected[..7]),
            )
        }
        ContentSource::Stash { oid, area } | ContentSource::StashBefore { oid, area } => {
            let file = session
                .stash_detail(revision, &oid, None, None)?
                .groups
                .into_iter()
                .find(|g| g.area == area)
                .and_then(|g| g.files.into_iter().find(|f| f.id == id))
                .ok_or("INVALID_FILE_ID")?;
            let stash = repo
                .find_commit(Oid::from_str(&oid).map_err(err)?)
                .map_err(err)?;
            if before {
                let result=match area {
                    StashArea::Untracked=>Ok(empty(file.path,"Before untracked stash file".into(),"deleted",0)),
                    StashArea::Staged|StashArea::Unstaged=>{let parent=stash.parent_id(if area==StashArea::Staged{0}else{1}).map_err(err)?.to_string();tree_file(&repo,&parent,file.path,format!("Stash before · {:?}",area))}
                };
                session.fresh(revision)?;return result;
            }
            let target = match area {
                StashArea::Staged => stash.parent_id(1).map_err(err)?,
                StashArea::Unstaged => stash.id(),
                StashArea::Untracked => stash.parent_id(2).map_err(err)?,
            };
            tree_file(
                &repo,
                &target.to_string(),
                file.path,
                format!("Stash {} · {:?}", &oid[..7], area),
            )
        }
        ContentSource::Head {} | ContentSource::Worktree {} | ContentSource::Index {} => {
            let path = session
                .files
                .get(id)
                .and_then(|p| p.to_str())
                .ok_or("INVALID_FILE_ID")?
                .to_string();
            if matches!(source,ContentSource::Head{}) {
                let head=session.fresh(revision)?.head_oid;
                let result=match head{Some(oid)=>tree_file(&repo,&oid,path,"HEAD".into()),None=>Ok(empty(path,"Unborn HEAD".into(),"deleted",0))};
                session.fresh(revision)?;return result;
            }
            if matches!(source, ContentSource::Index {}) {
                let index = repo.index().map_err(err)?;
                if (1..=3).any(|stage| index.get_path(Path::new(&path), stage).is_some()) {
                    return Err(
                        "UNMERGED_FILE: 충돌 해결 전 index 파일은 표시할 수 없습니다.".into(),
                    );
                }
                return match index.get_path(Path::new(&path), 0) {
                    Some(entry) => blob(&repo, path, "Index".into(), entry.id, entry.mode),
                    None => Ok(empty(path, "Index".into(), "deleted", 0)),
                };
            }
            let full = rel(&session.root, Path::new(&path))?;
            let meta = match fs::symlink_metadata(&full) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(empty(path, "Working tree".into(), "deleted", 0))
                }
                Err(e) => return Err(err(e)),
            };
            if is_link(&meta) || !meta.is_file() {
                return Err("UNSUPPORTED_FILE_MODE".into());
            }
            let bytes = bounded(&full, FILE_LIMIT)?;
            let result = decode(path, "Working tree".into(), &bytes);
            session.fresh(revision)?;
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        Repository::init(d.path()).unwrap();
        let oid = commit(d.path(), Some(b"committed\n"));
        (d, oid)
    }
    fn commit(root: &Path, bytes: Option<&[u8]>) -> String {
        let r = open(root).unwrap();
        let mut index = r.index().unwrap();
        if let Some(bytes) = bytes {
            fs::write(root.join("file[1].txt"), bytes).unwrap();
            index.add_path(Path::new("file[1].txt")).unwrap();
        } else {
            fs::remove_file(root.join("file[1].txt")).unwrap();
            index.remove_path(Path::new("file[1].txt")).unwrap();
        }
        index.write().unwrap();
        let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = r.head().ok().and_then(|h| h.peel_to_commit().ok());
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        r.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "File content fixture",
            &tree,
            &parent.iter().collect::<Vec<_>>(),
        )
        .unwrap()
        .to_string()
    }
    #[test]
    fn worktree_index_and_historical_contents_remain_independent() {
        let (d, initial) = fixture();
        let latest = commit(d.path(), Some(b"second commit\r\n"));
        let r = open(d.path()).unwrap();
        fs::write(d.path().join("file[1].txt"), b"staged\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("file[1].txt")).unwrap();
        index.write().unwrap();
        fs::write(d.path().join("file[1].txt"), b"working\n").unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let id = file_id("file[1].txt");
        for (source, expected) in [
            (ContentSource::Worktree {}, "working\n"),
            (ContentSource::Index {}, "staged\n"),
            (ContentSource::Head {}, "second commit\r\n"),
            (ContentSource::Comparison{base:initial.clone(),target:latest.clone(),before:true}, "committed\n"),
            (ContentSource::Comparison{base:initial.clone(),target:latest.clone(),before:false}, "second commit\r\n"),
            (ContentSource::CommitBefore {oid:latest.clone()}, "committed\n"),
            (ContentSource::HistoryBefore {anchor:Some(latest.clone()),offset:0,selected:latest.clone()}, "committed\n"),
            (
                ContentSource::Commit {
                    oid: latest.clone(),
                },
                "second commit\r\n",
            ),
            (
                ContentSource::History {
                    anchor: Some(latest.clone()),
                    offset: 0,
                    selected: initial.clone(),
                },
                "committed\n",
            ),
        ] {
            let result = read(&mut session, &s.revision, &id, source).unwrap();
            assert_eq!(result.text, expected);
            assert_eq!(result.state, "text");
            assert!(!result.truncated);
        }
        assert!(read(&mut session,&s.revision,"unknown",ContentSource::Comparison{base:initial.clone(),target:latest.clone(),before:true}).is_err());
        assert!(read(&mut session,&s.revision,&id,ContentSource::Comparison{base:"0".repeat(40),target:latest.clone(),before:false}).is_err());
        assert!(read(&mut session,&s.revision,&id,ContentSource::Comparison{base:latest.clone(),target:latest.clone(),before:false}).is_err());
        for source in [ContentSource::CommitBefore{oid:initial.clone()},ContentSource::HistoryBefore{anchor:Some(latest),offset:0,selected:initial}] {
            let absent=read(&mut session,&s.revision,&id,source).unwrap();
            assert_eq!(absent.state,"deleted");assert!(absent.text.is_empty());
        }
        assert!(read(&mut session,&s.revision,&file_id("not-in-this-commit"),ContentSource::CommitBefore{oid:s.head_oid.clone().unwrap()}).is_err());
        assert_eq!(before, fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(s.revision, session.snapshot().unwrap().revision);
        assert_eq!(
            fs::read_to_string(d.path().join("file[1].txt")).unwrap(),
            "working\n"
        );
        assert!(read(
            &mut session,
            &s.revision,
            "../file[1].txt",
            ContentSource::Worktree {}
        )
        .is_err());
        fs::write(d.path().join("file[1].txt"), "external").unwrap();
        assert!(
            read(&mut session, &s.revision, &id, ContentSource::Index {})
                .unwrap_err()
                .starts_with("STALE_STATE")
        );
    }
    #[test]
    fn deleted_and_unsupported_content_are_explicit_and_bounded() {
        let (d, _) = fixture();
        let oid = commit(d.path(), None);
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        assert_eq!(
            read(
                &mut session,
                &s.revision,
                &file_id("file[1].txt"),
                ContentSource::Commit { oid }
            )
            .unwrap()
            .state,
            "deleted"
        );
        for (bytes, state) in [
            (vec![0, 1, 2], "binary"),
            (vec![255], "unsupportedEncoding"),
            (vec![b'x'; FILE_LIMIT as usize + 1], "tooLarge"),
        ] {
            let oid = commit(d.path(), Some(&bytes));
            let s = session.snapshot().unwrap();
            let result = read(
                &mut session,
                &s.revision,
                &file_id("file[1].txt"),
                ContentSource::Commit { oid },
            )
            .unwrap();
            assert_eq!(result.state, state);
            assert!(result.text.is_empty());
            assert_eq!(result.size, bytes.len() as u64);
        }
        let long = "한".repeat(200_000);
        let result = decode("file".into(), "test".into(), long.as_bytes());
        assert!(result.truncated);
        assert!(result.text.len() <= 512 * 1024);
        assert!(long.starts_with(&result.text));
        let result = decode(
            "file".into(),
            "test".into(),
            "x\n".repeat(10_001).as_bytes(),
        );
        assert_eq!(result.text.lines().count(), 10_000);
        assert!(result.truncated);
        let result = decode("file".into(), "test".into(), b"last");
        assert!(!result.ends_with_newline);
        assert_eq!(decode("file".into(), "test".into(), b"").state, "text");
    }
    #[test]
    fn stash_area_content_never_reads_current_worktree() {
        let (d, _) = fixture();
        let mut r = open(d.path()).unwrap();
        fs::write(d.path().join("file[1].txt"), "stash staged\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("file[1].txt")).unwrap();
        index.write().unwrap();
        fs::write(d.path().join("file[1].txt"), "stash worktree\n").unwrap();
        fs::write(d.path().join("extra.txt"), "stash untracked\n").unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let oid = r
            .stash_save(&sig, "content", Some(git2::StashFlags::INCLUDE_UNTRACKED))
            .unwrap()
            .to_string();
        fs::write(d.path().join("file[1].txt"), "current dirty\n").unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        for (area, path, expected) in [
            (StashArea::Staged, "file[1].txt", "stash staged\n"),
            (StashArea::Unstaged, "file[1].txt", "stash worktree\n"),
            (StashArea::Untracked, "extra.txt", "stash untracked\n"),
        ] {
            assert_eq!(
                read(
                    &mut session,
                    &s.revision,
                    &file_id(path),
                    ContentSource::Stash {
                        oid: oid.clone(),
                        area
                    }
                )
                .unwrap()
                .text,
                expected
            );
        }
        for (area,path,expected,state) in [
            (StashArea::Staged,"file[1].txt","committed\n","text"),
            (StashArea::Unstaged,"file[1].txt","stash staged\n","text"),
            (StashArea::Untracked,"extra.txt","","deleted"),
        ] {
            let result=read(&mut session,&s.revision,&file_id(path),ContentSource::StashBefore{oid:oid.clone(),area}).unwrap();
            assert_eq!(result.text,expected);assert_eq!(result.state,state);
        }
        assert_eq!(s.revision, session.snapshot().unwrap().revision);
    }
    #[test]
    fn refuses_conflicted_index_and_non_regular_blob_modes() {
        let (d, _) = fixture();
        let r = open(d.path()).unwrap();
        let mut index = r.index().unwrap();
        let mut entry = index.get_path(Path::new("file[1].txt"), 0).unwrap();
        index.remove_path(Path::new("file[1].txt")).unwrap();
        entry.flags = (entry.flags & !0x3000) | 0x1000;
        index.add(&entry).unwrap();
        index.write().unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        assert!(read(
            &mut session,
            &s.revision,
            &file_id("file[1].txt"),
            ContentSource::Index {}
        )
        .unwrap_err()
        .starts_with("UNMERGED_FILE"));
        assert!(blob(&r, "file".into(), "test".into(), entry.id, 0o120000)
            .unwrap_err()
            .contains("UNSUPPORTED_FILE_MODE"));
    }
    #[test]
    fn staged_and_worktree_deletions_and_invalid_requests() {
        let (d, oid) = fixture();
        fs::remove_file(d.path().join("file[1].txt")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let id = file_id("file[1].txt");
        assert_eq!(
            read(&mut session, &s.revision, &id, ContentSource::Worktree {})
                .unwrap()
                .state,
            "deleted"
        );
        assert_eq!(
            read(&mut session, &s.revision, &id, ContentSource::Index {})
                .unwrap()
                .text,
            "committed\n"
        );
        let r = open(d.path()).unwrap();
        let mut index = r.index().unwrap();
        index.remove_path(Path::new("file[1].txt")).unwrap();
        index.write().unwrap();
        let s = session.snapshot().unwrap();
        assert_eq!(
            read(&mut session, &s.revision, &id, ContentSource::Index {})
                .unwrap()
                .state,
            "deleted"
        );
        assert!(read(
            &mut session,
            &s.revision,
            &id,
            ContentSource::Commit { oid: "HEAD".into() }
        )
        .is_err());
        assert!(read(
            &mut session,
            &s.revision,
            &id,
            ContentSource::History {
                anchor: Some(oid),
                offset: 0,
                selected: "HEAD".into()
            }
        )
        .is_err());
        assert!(serde_json::from_str::<ContentSource>(
            r#"{"kind":"worktree","path":"/etc/passwd"}"#
        )
        .is_err());
    }
}
