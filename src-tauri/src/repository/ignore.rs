use super::*;
use std::io::Write;

fn contents(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => bounded(path, 1024 * 1024).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}

pub(super) fn execute(session: &mut Session, repo: &Repository, revision: &str, id: &str) -> Result<()> {
    session.fresh(revision)?;
    let path = session.files.get(id).ok_or("INVALID_FILE_ID")?.clone();
    let name = path.to_str().ok_or("UNSUPPORTED_PATH")?;
    if name == ".gitignore" || name.chars().any(char::is_control) {
        return Err("UNSUPPORTED_IGNORE_PATH".into());
    }
    rel(&session.root, &path)?;
    if !repo.status_file(&path).map_err(err)?.is_wt_new() || repo.index().map_err(err)?.get_path(&path, 0).is_some() {
        return Err("IGNORE_REQUIRES_UNTRACKED_FILE".into());
    }
    let destination = rel(&session.root, Path::new(".gitignore"))?;
    let before = contents(&destination)?;
    let mut output = before.clone().unwrap_or_default();
    if output.contains(&0) { return Err("INVALID_GITIGNORE".into()); }
    let mut pattern = String::from("/");
    for ch in name.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\' | '#' | '!' | ' ') { pattern.push('\\'); }
        pattern.push(ch);
    }
    if !output.is_empty() && !output.ends_with(b"\n") { output.push(b'\n'); }
    output.extend_from_slice(pattern.as_bytes()); output.push(b'\n');
    if output.len() > 1024 * 1024 { return Err("GITIGNORE_LIMIT".into()); }
    // Keep the temporary file out of status so revision revalidation does not
    // observe our own preparation as a new untracked working file.
    let mut temporary = tempfile::NamedTempFile::new_in(session.root.join(".git")).map_err(err)?;
    if before.is_some() {
        temporary.as_file().set_permissions(fs::metadata(&destination).map_err(err)?.permissions()).map_err(err)?;
    }
    temporary.write_all(&output).map_err(err)?;
    temporary.as_file().sync_all().map_err(err)?;
    session.fresh(revision)?;
    if contents(&destination)? != before { return Err("STALE_GITIGNORE".into()); }
    if before.is_none() { temporary.persist_noclobber(&destination).map_err(err)?; }
    else { temporary.persist(&destination).map_err(err)?; }
    checkout::sync_dir(&session.root)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let d=tempfile::tempdir().unwrap(); let repo=Repository::init(d.path()).unwrap();
        fs::write(d.path().join("tracked"),b"tracked bytes\n").unwrap();
        let mut index=repo.index().unwrap(); index.add_path(Path::new("tracked")).unwrap(); index.write().unwrap();
        let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap(); d
    }
    fn ignored(root:&Path,name:&str)->bool {
        let result=std::process::Command::new("git").arg("--no-optional-locks").arg("-C").arg(root)
            .args(["check-ignore","--no-index","--",name]).env("GIT_CONFIG_NOSYSTEM","1").env("GIT_CONFIG_GLOBAL","/dev/null").output().unwrap();
        assert!(matches!(result.status.code(),Some(0|1)),"{result:?}"); result.status.success()
    }
    #[cfg(unix)]
    #[test]
    fn literal_special_paths_match_only_selected_file_using_git_oracle() {
        for (name,decoy) in [("star*.tmp","starX.tmp"),("query?.tmp","queryX.tmp"),("set[1].tmp","set1.tmp"),("#hash.tmp","hash.tmp"),("!bang.tmp","bang.tmp"),("space end ","space end"),("back\\slash.tmp","backslash.tmp"),("한글 파일.txt","한글파일.txt"),("nested/a*.txt","nested/ab.txt")] {
            let d=fixture(); fs::create_dir_all(d.path().join("nested")).unwrap();
            for path in [name,decoy] {fs::write(d.path().join(path),b"keep on disk\n").unwrap();}
            fs::write(d.path().join(".gitignore"),b"# existing bytes without newline").unwrap();
            let index=fs::read(d.path().join(".git/index")).unwrap();
            let mut session=Session::new(d.path().into()).unwrap();let state=session.snapshot();
            if name.contains('\\') {
                // Cross-platform path policy deliberately refuses literal backslashes.
                assert!(state.unwrap_err().contains("INVALID_PATH"));
                assert_eq!(fs::read(d.path().join(".gitignore")).unwrap(),b"# existing bytes without newline");
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
                for path in [name,decoy] {assert_eq!(fs::read(d.path().join(path)).unwrap(),b"keep on disk\n");}
                continue;
            }
            let state=state.unwrap();
            session.mutate(&state.revision,Action::IgnoreFile{file_id:file_id(name)}).unwrap();
            assert!(ignored(d.path(),name),"selected: {name}");assert!(!ignored(d.path(),decoy),"unrelated: {decoy}");
            assert!(fs::read(d.path().join(".gitignore")).unwrap().starts_with(b"# existing bytes without newline\n/"));
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
            for path in [name,decoy] {assert_eq!(fs::read(d.path().join(path)).unwrap(),b"keep on disk\n");}
        }
    }
    #[test]
    fn stale_ignore_preserves_external_rule_then_explicit_retry_appends() {
        let d=fixture();fs::write(d.path().join("selected.tmp"),b"selected bytes").unwrap();
        fs::write(d.path().join(".gitignore"),b"# initial\n").unwrap();
        let mut session=Session::new(d.path().into()).unwrap();let reviewed=session.snapshot().unwrap();
        let external=b"# external edit\n/another.tmp\n";fs::write(d.path().join(".gitignore"),external).unwrap();
        let index=fs::read(d.path().join(".git/index")).unwrap();
        assert!(session.mutate(&reviewed.revision,Action::IgnoreFile{file_id:file_id("selected.tmp")}).unwrap_err().starts_with("STALE_STATE"));
        assert_eq!(fs::read(d.path().join(".gitignore")).unwrap(),external);
        let fresh=session.snapshot().unwrap();session.mutate(&fresh.revision,Action::IgnoreFile{file_id:file_id("selected.tmp")}).unwrap();
        assert_eq!(fs::read(d.path().join(".gitignore")).unwrap(),[external.as_slice(),b"/selected.tmp\n"].concat());
        assert!(ignored(d.path(),"selected.tmp"));assert!(ignored(d.path(),"another.tmp"));
        assert_eq!(fs::read(d.path().join("selected.tmp")).unwrap(),b"selected bytes");assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
    }
    #[test]
    fn invalid_ignore_contents_and_tracked_file_are_refused_without_changes() {
        for binary in [false,true] {
            let d=fixture();fs::write(d.path().join("tracked"),b"later tracked edit").unwrap();
            fs::write(d.path().join("selected.tmp"),b"selected bytes").unwrap();
            let contents=if binary {b"bad\0rule\n".as_slice()}else{b"# preserve\n".as_slice()};fs::write(d.path().join(".gitignore"),contents).unwrap();
            let index=fs::read(d.path().join(".git/index")).unwrap();
            let mut session=Session::new(d.path().into()).unwrap();let state=session.snapshot().unwrap();
            let name=if binary {"selected.tmp"}else{"tracked"};
            assert!(session.mutate(&state.revision,Action::IgnoreFile{file_id:file_id(name)}).is_err());
            assert_eq!(fs::read(d.path().join(".gitignore")).unwrap(),contents);assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
            assert_eq!(fs::read(d.path().join("tracked")).unwrap(),b"later tracked edit");assert_eq!(fs::read(d.path().join("selected.tmp")).unwrap(),b"selected bytes");
            let state=session.snapshot().unwrap();assert!(session.mutate(&state.revision,Action::IgnoreFile{file_id:file_id(".gitignore")}).is_err());
            assert_eq!(fs::read(d.path().join(".gitignore")).unwrap(),contents);
        }
    }
}

