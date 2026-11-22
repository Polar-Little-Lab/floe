use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Layout { pub private: PathBuf, pub common: PathBuf, pub linked: bool, pub submodule: bool }
fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if !meta.is_dir() || is_link(&meta) { return Err("UNSUPPORTED_GIT_DIRECTORY".into()); }
    Ok(())
}
fn pointer(path: &Path, prefix: &str) -> Result<String> {
    let bytes = bounded(path, 4096)?;
    let text = std::str::from_utf8(&bytes).map_err(err)?.trim_end_matches(['\r','\n']);
    let value = text.strip_prefix(prefix).ok_or("INVALID_WORKTREE_LINK")?;
    if value.is_empty() || value.chars().any(char::is_control) { return Err("INVALID_WORKTREE_LINK".into()); }
    Ok(value.into())
}
fn resolve(base: &Path, value: &str) -> Result<PathBuf> { base.join(value).canonicalize().map_err(err) }
impl Layout {
    pub fn read(root: &Path) -> Result<Self> {
        let marker = root.join(".git");
        let meta = fs::symlink_metadata(&marker).map_err(|_|"NOT_REPOSITORY".to_string())?;
        if is_link(&meta) { return Err("UNSUPPORTED_GIT_LINK".into()); }
        if meta.is_dir() { return Ok(Self { private: marker.clone(), common: marker, linked: false, submodule: false }); }
        if !meta.is_file() { return Err("UNSUPPORTED_LAYOUT".into()); }
        let private = resolve(root, &pointer(&marker, "gitdir: ")?)?;
        // Standard absorbed submodule: metadata stays under an ancestor's .git/modules.
        for ancestor in root.ancestors().skip(1) {
            let git=ancestor.join(".git");
            let modules=git.join("modules");
            if !private.starts_with(&modules) || private==modules {continue;}
            directory(&git)?;directory(&modules)?;
            let mut component=modules.clone();
            for part in private.strip_prefix(&modules).map_err(err)?.components(){component.push(part);directory(&component)?;}
            let config=remote_remove::read_only_entries(&bounded(&private.join("config"),65536)?)?;
            let paths:Vec<_>=config.iter().filter(|(key,_)|key=="core.worktree").collect();
            if paths.len()!=1 || resolve(&private,&paths[0].1)?!=root {return Err("INVALID_SUBMODULE_LINK".into());}
            return Ok(Self{private:private.clone(),common:private,linked:true,submodule:true});
        }
        let parent = private.parent().ok_or("INVALID_WORKTREE_LINK")?;
        let common = parent.parent().ok_or("INVALID_WORKTREE_LINK")?.to_path_buf();
        if parent.file_name() != Some(std::ffi::OsStr::new("worktrees")) || common.file_name() != Some(std::ffi::OsStr::new(".git")) { return Err("UNSUPPORTED_LINKED_LAYOUT".into()); }
        for path in [&private,parent,&common] { directory(path)?; }
        if resolve(&private, &pointer(&private.join("commondir"), "")?)? != common
            || resolve(&private, &pointer(&private.join("gitdir"), "")?)? != marker.canonicalize().map_err(err)? {
            return Err("INVALID_WORKTREE_LINK".into());
        }
        Ok(Self { private, common, linked: true, submodule: false })
    }
    pub fn file(&self, name: &str) -> PathBuf {
        if matches!(name, "config" | "packed-refs" | "logs/refs/stash") { self.common.join(name) } else { self.private.join(name) }
    }
}

/// Transaction inputs are checkout-local even when refs and objects are shared.
pub(super) fn private_file(root:&Path,name:&str)->Result<PathBuf> {
    if Path::new(name).components().any(|p|!matches!(p,std::path::Component::Normal(_))) {return Err("INVALID_PRIVATE_PATH".into());}
    Ok(Layout::read(root)?.private.join(name))
}
pub(super) fn journal_owner(root:&Path)->Result<Option<PathBuf>> {
    let layout=Layout::read(root)?;
    if layout.linked {Ok(Some(layout.private.canonicalize().map_err(err)?))} else {Ok(None)}
}

#[cfg(test)]
mod submodule_tests {
    use super::*;
    #[test]
    fn absorbed_submodule_opens_read_only_and_rejects_bad_backlink() {
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path().canonicalize().unwrap();
        Repository::init(&root).unwrap();
        let child=root.join("libs/module with spaces");
        fs::create_dir_all(&child).unwrap();
        let repo=Repository::init(&child).unwrap();
        drop(repo);
        let metadata=root.join(".git/modules/module with spaces");
        fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        fs::rename(child.join(".git"),&metadata).unwrap();
        fs::write(child.join(".git"),format!("gitdir: {}\n",metadata.display())).unwrap();
        let mut config=Config::open(&metadata.join("config")).unwrap();
        config.set_str("core.worktree",child.to_str().unwrap()).unwrap();
        let layout=Layout::read(&child).unwrap();
        assert!(layout.linked&&layout.submodule);
        assert_eq!(layout.private,metadata);
        assert!(validate(&child).unwrap().unwrap().contains("read-only"));
        let mut session=Session::new(child.clone()).unwrap();
        let snapshot=session.snapshot().unwrap();
        assert!(session.mutate(&snapshot.revision,Action::StageAll).is_err());
        config.set_str("core.worktree",root.to_str().unwrap()).unwrap();
        assert!(Layout::read(&child).is_err());
        config.set_str("core.worktree",child.to_str().unwrap()).unwrap();
        fs::write(metadata.join("commondir"),"../other").unwrap();
        assert!(Session::new(child).is_err());
    }
}


#[cfg(test)]
pub(super) mod write_isolation_tests {
    use super::*;
    pub(in crate::repository) fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp=tempfile::tempdir().unwrap();let base=temp.path().canonicalize().unwrap();
        let main=base.join("main");let first=base.join("linked-first");let second=base.join("linked-second");
        let repo=Repository::init(&main).unwrap();
        fs::write(main.join("hello.txt"),"hello\n").unwrap();
        let mut index=repo.index().unwrap();index.add_path(Path::new("hello.txt")).unwrap();index.write().unwrap();
        let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
        repo.worktree("first",&first,None).unwrap();
        repo.worktree("second",&second,None).unwrap();
        (temp,main,first,second)
    }
    #[test]
    fn common_lease_excludes_main_and_sibling_worktrees_and_releases() {
        let (_temp,main,first,second)=fixture();
        for owner in [&main,&first,&second] {
            let held=lease::OperationLease::acquire(owner).unwrap();
            for other in [&main,&first,&second] {assert!(lease::OperationLease::acquire(other).err().unwrap().contains("BUSY_REPOSITORY"));}
            drop(held);
            for other in [&main,&first,&second] {drop(lease::OperationLease::acquire(other).unwrap());}
        }
        for linked in [&first,&second] {assert!(!Layout::read(linked).unwrap().private.join("branchdesk-operation.lock").exists());}
        assert!(main.join(".git/branchdesk-operation.lock").is_file());
    }
    #[test]
    fn linked_index_publish_is_private_and_preserves_other_checkouts() {
        let (_temp,main,first,second)=fixture();
        let layout=Layout::read(&first).unwrap();let sibling=Layout::read(&second).unwrap();
        let main_index=fs::read(main.join(".git/index")).unwrap();
        let sibling_index=fs::read(sibling.private.join("index")).unwrap();
        let original=fs::read(layout.private.join("index")).unwrap();
        let main_head=fs::read(main.join(".git/HEAD")).unwrap();
        let repo=Repository::open(&first).unwrap();
        fs::write(first.join("hello.txt"),"linked change\n").unwrap();
        let mut guard=IndexLock::acquire(&first).unwrap();
        assert_eq!(guard.path,layout.private.join("index.lock"));
        assert_eq!(fs::read(&guard.path).unwrap(),original);
        let mut index=git2::Index::open(&guard.path).unwrap();
        let mut entry=repo.index().unwrap().get_path(Path::new("hello.txt"),0).unwrap();
        entry.id=repo.blob(b"linked change\n").unwrap();entry.file_size=14;
        index.add(&entry).unwrap();index.write().unwrap();
        guard.publish(&layout.private.join("index")).unwrap();drop(guard);
        assert_eq!(git2::Index::open(&layout.private.join("index")).unwrap().get_path(Path::new("hello.txt"),0).unwrap().id,entry.id);
        assert_eq!(fs::read(main.join(".git/index")).unwrap(),main_index);
        assert_eq!(fs::read(sibling.private.join("index")).unwrap(),sibling_index);
        assert_eq!(fs::read(main.join(".git/HEAD")).unwrap(),main_head);
        assert_eq!(fs::read(main.join("hello.txt")).unwrap(),b"hello\n");
        assert_eq!(fs::read(second.join("hello.txt")).unwrap(),b"hello\n");
        assert!(!layout.private.join("index.lock").exists());
        assert!(!main.join(".git/index.lock").exists());
    }
    #[test]
    fn existing_linked_lock_is_preserved_and_sibling_index_lock_is_independent() {
        let (_temp,main,first,second)=fixture();let layout=Layout::read(&first).unwrap();
        let path=layout.private.join("index.lock");fs::write(&path,b"external lock").unwrap();
        assert!(IndexLock::acquire(&first).is_err());
        assert_eq!(fs::read(&path).unwrap(),b"external lock");
        drop(IndexLock::acquire(&main).unwrap());drop(IndexLock::acquire(&second).unwrap());
        assert_eq!(fs::read(path).unwrap(),b"external lock");
    }
    #[test]
    fn linked_missing_index_cleans_owned_lock_without_touching_main_index() {
        let (_temp,main,first,_second)=fixture();let layout=Layout::read(&first).unwrap();
        let before=fs::read(main.join(".git/index")).unwrap();
        fs::remove_file(layout.private.join("index")).unwrap();
        assert!(IndexLock::acquire(&first).err().unwrap().contains("MISSING_INDEX"));
        assert!(!layout.private.join("index.lock").exists());
        assert_eq!(fs::read(main.join(".git/index")).unwrap(),before);
    }
    #[test]
    fn unmigrated_linked_mutation_gate_stays_closed() {
        let (_temp,main,first,_second)=fixture();
        let before=fs::read(main.join(".git/index")).unwrap();
        let mut session=Session::new(first.clone()).unwrap();let snapshot=session.snapshot().unwrap();
        assert!(session.mutate(&snapshot.revision,Action::CreateBranch{name:"unsupported".into()}).is_err());
        assert_eq!(fs::read(main.join(".git/index")).unwrap(),before);
        assert!(!Layout::read(&first).unwrap().private.join("index.lock").exists());
    }
    #[test]
    fn broken_backlink_prevents_lock_creation_in_both_scopes() {
        let (_temp,main,first,_second)=fixture();let layout=Layout::read(&first).unwrap();
        fs::write(layout.private.join("gitdir"),main.join(".git").to_str().unwrap()).unwrap();
        assert!(IndexLock::acquire(&first).is_err());assert!(lease::OperationLease::acquire(&first).is_err());
        assert!(!layout.private.join("index.lock").exists());assert!(!main.join(".git/branchdesk-operation.lock").exists());
    }
}
