use super::*;
#[derive(Serialize)]
pub struct Entry { pub name: String, pub path: String, pub branch: String, pub valid: bool, pub locked: bool, #[serde(rename="pruneToken")] pub prune_token: Option<String> }
#[derive(Serialize)]
pub struct Listing { pub entries: Vec<Entry>, pub truncated: bool }
fn metadata_dir(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.len() > 200 || Path::new(name).components().count() != 1 || !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) {
        return Err("INVALID_WORKTREE_NAME".into());
    }
    let parent = root.join(".git/worktrees");
    for path in [&parent, &parent.join(name)] {
        match fs::symlink_metadata(path) {
            Ok(meta) if !meta.is_dir() || is_link(&meta) => return Err("UNSUPPORTED_WORKTREE_METADATA".into()),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(err(e)),
        }
    }
    Ok(parent.join(name))
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct RemovalPreview {
    pub name:String,
    pub path:String,
    pub branch:String,
    pub head_oid:Option<String>,
    pub blockers:Vec<String>,
    pub token:Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct RemovalReceipt {pub snapshot:Snapshot,pub backup_path:String,pub manifest_path:String}
impl Session {
    pub fn worktree_removal_preview(&mut self,revision:&str,name:&str)->Result<RemovalPreview>{
        self.fresh(revision)?;
        if self.layout.linked{return Err("WORKTREE_REMOVE_FROM_MAIN_ONLY".into());}
        let main=open(&self.root)?;
        let entry=main.find_worktree(name).map_err(err)?;
        let metadata=metadata_dir(&self.root,name)?;
        let raw_path=entry.path();
        let path_meta=fs::symlink_metadata(raw_path).map_err(err)?;
        if !path_meta.is_dir()||is_link(&path_meta){return Err("UNSUPPORTED_WORKTREE_PATH".into());}
        let path=self.worktree_path(revision,name)?;
        if path.starts_with(&self.root)||self.root.starts_with(&path){return Err("OVERLAPPING_WORKTREE_PATH".into());}
        let checkout=checkout_status::read(path.clone(),path.to_str().ok_or("UNSUPPORTED_PATH")?)?;
        let linked=Repository::open(&path).map_err(err)?;
        let mut blockers=Vec::new();
        if fs::symlink_metadata(metadata.join("locked")).is_ok(){blockers.push("Worktree is locked. Unlock it before removal.".into());}
        if checkout.changed>0{blockers.push("Worktree has staged, unstaged or untracked changes.".into());}
        if checkout.write_block.is_some(){blockers.push("Worktree is read-only or has a pending recovery operation.".into());}
        if linked.state()!=git2::RepositoryState::Clean{blockers.push("A Git operation is in progress in this worktree.".into());}
        let mut options=git2::StatusOptions::new();
        options.include_ignored(true).include_untracked(true).recurse_untracked_dirs(true);
        let statuses=linked.statuses(Some(&mut options)).map_err(err)?;
        if statuses.iter().any(|entry|entry.status().contains(git2::Status::IGNORED)){blockers.push("Worktree contains ignored files; removal must preserve or explicitly handle them.".into());}
        if linked.submodules().map_err(err)?.len()>0{blockers.push("Worktree contains submodule registrations; recursive removal is not supported.".into());}
        let digest=match metadata_digest(&metadata){Ok(value)=>Some(value),Err(_)=>{blockers.push("Worktree metadata is locked, unsupported or exceeds inspection limits.".into());None}};
        let token=if blockers.is_empty(){
            let mut hash=Sha256::new();
            for value in [revision,name,path.to_str().ok_or("UNSUPPORTED_PATH")?,checkout.head_oid.as_deref().unwrap_or(""),digest.as_deref().unwrap_or("")]{hash.update((value.len() as u64).to_le_bytes());hash.update(value.as_bytes());}
            Some(format!("{:x}",hash.finalize()))
        }else{None};
        if self.worktree_path(revision,name)?!=path{return Err("WORKTREE_CHANGED".into());}
        Ok(RemovalPreview{name:name.into(),path:checkout.path,branch:checkout.branch,head_oid:checkout.head_oid,blockers,token})
    }
    pub fn remove_worktree_archived(&mut self,revision:&str,name:&str,expected:&str)->Result<RemovalReceipt>{
        use std::io::Write;
        if expected.len()!=64||!expected.bytes().all(|c|c.is_ascii_hexdigit()){return Err("INVALID_WORKTREE_REMOVAL_TOKEN".into());}
        let snapshot=self.fresh(revision)?;
        if let Some(reason)=snapshot.write_block{return Err(format!("READ_ONLY: {reason}"));}
        let _lease=lease::OperationLease::acquire(&self.root)?;
        let preview=self.worktree_removal_preview(revision,name)?;
        if preview.token.as_deref()!=Some(expected){return Err("WORKTREE_REMOVAL_CHANGED: inspect worktree again".into());}
        let source=PathBuf::from(&preview.path);
        let parent=source.parent().ok_or("INVALID_WORKTREE_PATH")?;
        let metadata=metadata_dir(&self.root,name)?;
        let backup=parent.join(format!(".branchdesk-removed-worktree-{}",Uuid::new_v4()));
        fs::create_dir(&backup).map_err(err)?;
        let manifest=backup.join("removal.json");
        let bytes=serde_json::to_vec(&serde_json::json!({"version":1,"repository":self.root.to_str().ok_or("UNSUPPORTED_PATH")?,"name":name,"originalPath":preview.path,"metadataPath":metadata.to_str().ok_or("UNSUPPORTED_PATH")?,"token":expected,"headOid":preview.head_oid,"branch":preview.branch})).map_err(err)?;
        let mut record=fs::OpenOptions::new().write(true).create_new(true).open(&manifest).map_err(err)?;
        record.write_all(&bytes).map_err(err)?;record.sync_all().map_err(err)?;drop(record);
        checkout::sync_dir(&backup)?;checkout::sync_dir(parent)?;
        if self.worktree_removal_preview(revision,name)?.token.as_deref()!=Some(expected){return Err(format!("WORKTREE_REMOVAL_CHANGED: no checkout moved; empty backup record at {}",backup.display()));}
        // Each rename stays on its source filesystem. Metadata is staged beside
        // the common Git directory so a checkout on another volume is supported.
        let metadata_backup=self.root.join(".git").join(format!("branchdesk-removed-worktree-{}",Uuid::new_v4()));
        fs::create_dir(&metadata_backup).map_err(err)?;
        let metadata_manifest=metadata_backup.join("checkout-backup.txt");
        let mut link=fs::OpenOptions::new().write(true).create_new(true).open(&metadata_manifest).map_err(err)?;
        link.write_all(backup.to_str().ok_or("UNSUPPORTED_PATH")?.as_bytes()).map_err(err)?;link.sync_all().map_err(err)?;drop(link);
        let mut link=fs::OpenOptions::new().write(true).create_new(true).open(backup.join("metadata-backup.txt")).map_err(err)?;
        link.write_all(metadata_backup.to_str().ok_or("UNSUPPORTED_PATH")?.as_bytes()).map_err(err)?;link.sync_all().map_err(err)?;drop(link);
        checkout::sync_dir(&metadata_backup)?;checkout::sync_dir(&self.root.join(".git"))?;checkout::sync_dir(&backup)?;
        fs::rename(&source,backup.join("checkout")).map_err(err)?;
        checkout::sync_dir(parent).and_then(|_|checkout::sync_dir(&backup)).map_err(|error|format!("WORKTREE_REMOVAL_PARTIAL: checkout moved; inspect {}; {error}",manifest.display()))?;
        if let Err(error)=fs::rename(&metadata,metadata_backup.join("metadata")){
            // Do not overwrite a path recreated by another process.
            let restored=matches!(fs::symlink_metadata(&source),Err(ref e) if e.kind()==std::io::ErrorKind::NotFound)&&fs::rename(backup.join("checkout"),&source).is_ok();
            return Err(format!("WORKTREE_REMOVAL_PARTIAL: metadata move failed; checkout restored={restored}; inspect {}; {error}",manifest.display()));
        }
        checkout::sync_dir(metadata.parent().ok_or("INVALID_WORKTREE_LINK")?).and_then(|_|checkout::sync_dir(&metadata_backup)).map_err(|error|format!("WORKTREE_REMOVAL_APPLIED_SYNC_FAILED: inspect {}; {error}",manifest.display()))?;
        let snapshot=self.snapshot().map_err(|error|format!("WORKTREE_REMOVAL_APPLIED_REFRESH_FAILED: inspect {}; {error}",manifest.display()))?;
        Ok(RemovalReceipt{snapshot,backup_path:backup.to_str().ok_or("UNSUPPORTED_PATH")?.into(),manifest_path:manifest.to_str().ok_or("UNSUPPORTED_PATH")?.into()})
    }
    pub fn worktree_checkout(&mut self,revision:&str,name:&str)->Result<checkout_status::Checkout>{
        let full=self.worktree_path(revision,name)?;
        let label=full.to_str().ok_or("UNSUPPORTED_PATH")?.to_owned();
        let checkout=checkout_status::read(full.clone(),&label)?;
        if self.worktree_path(revision,name)?!=full{return Err("WORKTREE_CHANGED".into());}
        Ok(checkout)
    }
    pub fn worktree_path(&mut self, revision: &str, name: &str) -> Result<PathBuf> {
        self.fresh(revision)?;
        let metadata = metadata_dir(&self.root, name)?.canonicalize().map_err(err)?;
        let repo = open(&self.root)?;
        let worktree = repo.find_worktree(name).map_err(err)?;
        worktree.validate().map_err(err)?;
        let root = worktree.path().canonicalize().map_err(err)?;
        let bytes = bounded(&root.join(".git"), 4096)?;
        let marker = std::str::from_utf8(&bytes).map_err(err)?.trim_end();
        let target = marker.strip_prefix("gitdir: ").ok_or("INVALID_WORKTREE_LINK")?;
        let target = Path::new(target);
        let resolved = if target.is_absolute() { target.to_path_buf() } else { root.join(target) };
        if resolved.canonicalize().map_err(err)? != metadata { return Err("INVALID_WORKTREE_LINK".into()); }
        Ok(root)
    }
    pub fn worktrees(&mut self, revision: &str) -> Result<Listing> {
        self.fresh(revision)?;
        let repo = open(&self.root)?;
        let names = repo.worktrees().map_err(err)?;
        let mut result = Listing { entries: Vec::new(), truncated: names.len() > 200 };
        for name in names.iter().take(200) {
            let name = name.map_err(err)?.ok_or("UNSUPPORTED_WORKTREE_NAME")?;
            let metadata = metadata_dir(&self.root, name)?;
            let worktree = repo.find_worktree(name).map_err(err)?;
            let head = bounded(&metadata.join("HEAD"), 2048).ok().and_then(|b|String::from_utf8(b).ok()).unwrap_or_default();
            let branch = head.trim().strip_prefix("ref: refs/heads/").unwrap_or(head.trim()).to_owned();
            result.entries.push(Entry { name: name.into(), path: worktree.path().to_str().ok_or("UNSUPPORTED_PATH")?.into(), branch, valid: worktree.validate().is_ok(), locked: fs::symlink_metadata(metadata.join("locked")).is_ok(), prune_token: prune_candidate(&self.root,name).ok() });
        }
        Ok(result)
    }
}
// A prune archives metadata only. It never deletes a checkout directory or branch.
fn metadata_digest(directory:&Path)->Result<String>{
    fn visit(base:&Path,path:&Path,hash:&mut Sha256,count:&mut usize,total:&mut u64,depth:usize)->Result<()> {
        if depth>16{return Err("WORKTREE_METADATA_LIMIT".into());}
        let mut children=fs::read_dir(path).map_err(err)?.map(|item|item.map(|item|item.path())).collect::<std::io::Result<Vec<_>>>().map_err(err)?;
        children.sort();
        for child in children {
            *count+=1;if *count>512{return Err("WORKTREE_METADATA_LIMIT".into());}
            let meta=fs::symlink_metadata(&child).map_err(err)?;
            let name=child.file_name().and_then(|n|n.to_str()).ok_or("UNSUPPORTED_WORKTREE_METADATA")?;
            if name=="locked"||name.ends_with(".lock"){return Err("WORKTREE_LOCKED".into());}
            if is_link(&meta){return Err("UNSUPPORTED_WORKTREE_METADATA".into());}
            let relative=child.strip_prefix(base).map_err(err)?.to_str().ok_or("UNSUPPORTED_WORKTREE_METADATA")?;
            hash.update((relative.len() as u64).to_le_bytes());hash.update(relative.as_bytes());
            if meta.is_dir(){hash.update(b"directory");visit(base,&child,hash,count,total,depth+1)?;}
            else if meta.is_file(){let bytes=bounded(&child,20*1024*1024)?;*total+=bytes.len() as u64;if *total>64*1024*1024{return Err("WORKTREE_METADATA_LIMIT".into());}hash.update(b"file");hash.update((bytes.len() as u64).to_le_bytes());hash.update(bytes);}
            else{return Err("UNSUPPORTED_WORKTREE_METADATA".into());}
        }
        Ok(())
    }
    let mut hash=Sha256::new();visit(directory,directory,&mut hash,&mut 0,&mut 0,0)?;Ok(format!("{:x}",hash.finalize()))
}
fn missing_checkout(root:&Path,name:&str)->Result<PathBuf>{
    let worktree=open(root)?.find_worktree(name).map_err(err)?;
    let path=worktree.path().to_path_buf();
    // symlink_metadata deliberately treats dangling symlinks as existing data.
    match fs::symlink_metadata(&path){Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(path),Ok(_)=>Err("WORKTREE_STILL_EXISTS: remove the checkout explicitly; prune only missing worktrees".into()),Err(e)=>Err(err(e))}
}
fn prune_candidate(root:&Path,name:&str)->Result<String>{
    let directory=metadata_dir(root,name)?;
    missing_checkout(root,name)?;
    metadata_digest(&directory)
}
pub(super) fn prune(session:&mut Session,revision:&str,name:&str,expected:&str)->Result<()> {
    use std::io::Write;
    if expected.len()!=64||!expected.bytes().all(|c|c.is_ascii_hexdigit()){return Err("INVALID_WORKTREE_PRUNE_TOKEN".into());}
    session.fresh(revision)?;
    let directory=metadata_dir(&session.root,name)?;
    let path=missing_checkout(&session.root,name)?;
    if prune_candidate(&session.root,name)?!=expected{return Err("STALE_WORKTREE_METADATA: reload worktrees".into());}
    let archives=session.root.join(".git/branchdesk-pruned-worktrees");
    match fs::symlink_metadata(&archives){Ok(m) if !m.is_dir()||is_link(&m)=>return Err("UNSUPPORTED_WORKTREE_ARCHIVE".into()),Ok(_)=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir(&archives).map_err(err)?,Err(e)=>return Err(err(e))}
    let archive=archives.join(Uuid::new_v4().to_string());fs::create_dir(&archive).map_err(err)?;
    let manifest=serde_json::to_vec(&serde_json::json!({"version":1,"name":name,"path":path.to_str().ok_or("UNSUPPORTED_PATH")?,"metadataDigest":expected})).map_err(err)?;
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(archive.join("manifest.json")).map_err(err)?;
    file.write_all(&manifest).map_err(err)?;file.sync_all().map_err(err)?;drop(file);
    checkout::sync_dir(&archive)?;checkout::sync_dir(&archives)?;
    // No metadata is removed if the checkout, lock or metadata changed during preparation.
    if prune_candidate(&session.root,name)?!=expected{return Err("STALE_WORKTREE_METADATA: reload worktrees".into());}
    fs::rename(&directory,archive.join("metadata")).map_err(err)?;
    // The single rename is the commit point. A crash leaves either the registration
    // or its complete archive. Branch refs and checkout files are untouched.
    checkout::sync_dir(directory.parent().ok_or("INVALID_WORKTREE_LINK")?).and_then(|_|checkout::sync_dir(&archive)).map_err(|e|format!("WORKTREE_PRUNE_APPLIED_SYNC_FAILED: metadata archived at {}; {e}",archive.display()))?;
    Ok(())
}
pub(super) fn set_lock(session: &mut Session, revision: &str, name: &str, expected_locked: bool, locked: bool) -> Result<()> {
    use std::io::Write;
    session.fresh(revision)?;
    let directory = metadata_dir(&session.root, name)?;
    open(&session.root)?.find_worktree(name).map_err(err)?;
    let path = directory.join("locked");
    let present = match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !is_link(&meta) => true,
        Ok(_) => return Err("UNSUPPORTED_WORKTREE_LOCK".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(err(e)),
    };
    if present != expected_locked { return Err("STALE_WORKTREE_LOCK".into()); }
    if locked == present { return Ok(()); }
    if locked {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(err)?;
        file.write_all(b"Locked in FLOE\n").map_err(err)?;
        file.sync_all().map_err(err)?;
    } else { fs::remove_file(&path).map_err(err)?; }
    checkout::sync_dir(&directory)
}
pub(super) fn require_branch_available(root: &Path, repo: &Repository, branch: &str) -> Result<()> {
    for name in repo.worktrees().map_err(err)?.iter() {
        let directory = metadata_dir(root, name.map_err(err)?.ok_or("UNSUPPORTED_WORKTREE_NAME")?)?;
        let bytes = bounded(&directory.join("HEAD"), 2048)?;
        let head = std::str::from_utf8(&bytes).map_err(err)?.trim();
        if head == format!("ref: refs/heads/{branch}") {
            return Err("BRANCH_IN_OTHER_WORKTREE: branch is checked out in another worktree".into());
        }
    }
    Ok(())
}
pub(super) fn create(session: &mut Session, repo: &Repository, revision: &str, name: &str, path: &str, branch: Option<&str>) -> Result<()> {
    if name.is_empty() || name.len() > 80 || !name.bytes().all(|b|b.is_ascii_alphanumeric() || b == b'-' || b == b'_') || name.starts_with('-') {
        return Err("INVALID_WORKTREE_NAME: use letters, numbers, hyphen or underscore".into());
    }
    let requested = crate::init_repository::validate(path, branch.unwrap_or(name))?;
    let parent = requested.parent().ok_or("INVALID_PATH")?.canonicalize().map_err(err)?;
    let target = parent.join(requested.file_name().ok_or("INVALID_PATH")?);
    if target.starts_with(&session.root) { return Err("WORKTREE_MUST_BE_OUTSIDE_REPOSITORY".into()); }
    for path in [&target, &metadata_dir(&session.root, name)?] {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            _ => return Err("WORKTREE_PATH_EXISTS".into()),
        }
    }
    let source = match branch { Some(branch) => format!("refs/heads/{branch}"), None => external_merge::head_lock_target(repo)? };
    if !source.starts_with("refs/heads/") { return Err("WORKTREE_REQUIRES_LOCAL_BRANCH".into()); }
    let _source_locks = checkout::locks(repo, &source, &source)?;
    session.fresh(revision)?;
    let reference = repo.find_reference(&source).map_err(err)?;
    let commit = repo.find_commit(reference.target().ok_or("WORKTREE_REQUIRES_DIRECT_BRANCH")?).map_err(err)?;
    checkout::inspect_tree(repo, &commit.tree().map_err(err)?)?;
    let mut options = git2::WorktreeAddOptions::new();
    if branch.is_some() { options.reference(Some(&reference)); }
    // libgit2 uses exclusive mkdir for metadata and destination. Do not delete
    // partial output on failure: external files may have appeared in the folder.
    repo.worktree(name, &target, Some(&options)).map_err(|e|format!("WORKTREE_CREATE: {e}. Partial files/branch may remain for inspection: {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod checkout_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d=tempfile::tempdir().unwrap();let root=d.path().join("main");let target=d.path().join("linked");
        let repo=Repository::init(&root).unwrap();fs::write(root.join("file"),"committed\n").unwrap();
        let mut index=repo.index().unwrap();index.add_path(Path::new("file")).unwrap();index.write().unwrap();
        let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
        fs::write(root.join("file"),"staged\n").unwrap();index.add_path(Path::new("file")).unwrap();index.write().unwrap();
        fs::write(root.join("file"),"working\n").unwrap();fs::write(root.join("untracked"),"new work\n").unwrap();(d,root,target)
    }
    fn preserved(root:&Path)->Vec<Vec<u8>>{[".git/index",".git/HEAD","file","untracked"].iter().map(|p|fs::read(root.join(p)).unwrap()).collect()}
    fn create_action(target:&Path)->Action{Action::CreateWorktree{name:"linked".into(),path:target.to_str().unwrap().into(),branch:None}}
    fn prepare_missing()->(tempfile::TempDir,PathBuf,PathBuf,Session,String){
        let (temp,root,target)=fixture();let mut session=Session::new(root.clone()).unwrap();let snap=session.snapshot().unwrap();
        let snap=session.mutate(&snap.revision,create_action(&target)).unwrap();assert!(session.worktrees(&snap.revision).unwrap().entries[0].prune_token.is_none());
        fs::remove_dir_all(&target).unwrap();let snap=session.snapshot().unwrap();let token=session.worktrees(&snap.revision).unwrap().entries[0].prune_token.clone().unwrap();
        (temp,root,target,session,token)
    }
    #[test]
    fn prune_missing_archives_metadata_preserving_branch_main_index_and_files(){
        let (_temp,root,target,mut session,token)=prepare_missing();let before=preserved(&root);let metadata=root.join(".git/worktrees/linked");let old_digest=metadata_digest(&metadata).unwrap();let head=fs::read(metadata.join("HEAD")).unwrap();
        let snap=session.snapshot().unwrap();let snap=session.mutate(&snap.revision,Action::PruneWorktree{name:"linked".into(),expected_metadata:token.clone()}).unwrap();
        assert!(session.worktrees(&snap.revision).unwrap().entries.is_empty());assert!(!metadata.exists());assert!(!target.exists());assert_eq!(preserved(&root),before);assert!(!root.join(".git/index.lock").exists());
        let archive=fs::read_dir(root.join(".git/branchdesk-pruned-worktrees")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(metadata_digest(&archive.join("metadata")).unwrap(),old_digest);assert_eq!(fs::read(archive.join("metadata/HEAD")).unwrap(),head);
        let manifest:serde_json::Value=serde_json::from_slice(&fs::read(archive.join("manifest.json")).unwrap()).unwrap();assert_eq!(manifest["metadataDigest"],token);assert_eq!(manifest["name"],"linked");
        assert!(open(&root).unwrap().find_reference("refs/heads/linked").is_ok());
        // The archived metadata can be put back without losing any bytes.
        fs::rename(archive.join("metadata"),&metadata).unwrap();let snap=session.snapshot().unwrap();assert_eq!(session.worktrees(&snap.revision).unwrap().entries.len(),1);assert_eq!(metadata_digest(&metadata).unwrap(),old_digest);
    }
    #[test]
    fn prune_rejects_locks_reappearing_checkout_and_changed_metadata(){
        for case in ["locked","git-lock","reappeared","changed"] {
            let (_temp,root,target,mut session,token)=prepare_missing();let metadata=root.join(".git/worktrees/linked");
            match case {"locked"=>fs::write(metadata.join("locked"),"protected").unwrap(),"git-lock"=>fs::write(metadata.join("index.lock"),"external").unwrap(),"reappeared"=>{fs::create_dir(&target).unwrap();fs::write(target.join("user-file"),"keep").unwrap();},_=>fs::write(metadata.join("HEAD"),"ref: refs/heads/changed\n").unwrap()}
            let before=preserved(&root);let head=fs::read(metadata.join("HEAD")).unwrap();let snap=session.snapshot().unwrap();
            assert!(session.mutate(&snap.revision,Action::PruneWorktree{name:"linked".into(),expected_metadata:token}).is_err());assert_eq!(preserved(&root),before);assert_eq!(fs::read(metadata.join("HEAD")).unwrap(),head);assert!(!root.join(".git/branchdesk-pruned-worktrees").exists());
            if case=="reappeared" {assert_eq!(fs::read(target.join("user-file")).unwrap(),b"keep");}
        }
    }
    #[test]
    fn prune_archive_failure_and_stale_main_revision_leave_registration_untouched(){
        for case in ["archive-file","stale"] {
            let (_temp,root,_target,mut session,token)=prepare_missing();let snap=session.snapshot().unwrap();
            if case=="archive-file" {fs::write(root.join(".git/branchdesk-pruned-worktrees"),"user file").unwrap();}else{fs::write(root.join("file"),"later edit").unwrap();}
            let before=preserved(&root);assert!(session.mutate(&snap.revision,Action::PruneWorktree{name:"linked".into(),expected_metadata:token}).is_err());assert_eq!(preserved(&root),before);assert!(root.join(".git/worktrees/linked/HEAD").exists());
        }
    }
    #[cfg(unix)]
    #[test]
    fn prune_does_not_follow_checkout_symlink_or_archive_symlink(){
        use std::os::unix::fs::symlink;
        for archive_link in [false,true] {
            let (temp,root,target,mut session,token)=prepare_missing();let outside=temp.path().join("outside");fs::create_dir(&outside).unwrap();fs::write(outside.join("keep"),"keep").unwrap();
            let snap=session.snapshot().unwrap();if archive_link {symlink(&outside,root.join(".git/branchdesk-pruned-worktrees")).unwrap();}else{symlink(temp.path().join("nonexistent"),&target).unwrap();}
            assert!(session.mutate(&snap.revision,Action::PruneWorktree{name:"linked".into(),expected_metadata:token}).is_err());assert_eq!(fs::read(outside.join("keep")).unwrap(),b"keep");assert!(root.join(".git/worktrees/linked/HEAD").exists());
        }
    }
    #[test]
    fn create_worktree_keeps_main_edits_and_protects_checked_out_branch() {
        let (_d,root,target)=fixture();let before=preserved(&root);let mut session=Session::new(root.clone()).unwrap();let state=session.snapshot().unwrap();
        let after=session.mutate(&state.revision,create_action(&target)).unwrap();
        assert_eq!(preserved(&root),before);assert_eq!(fs::read(target.join("file")).unwrap(),b"committed\n");assert!(!target.join("untracked").exists());
        let repo=Repository::open(&root).unwrap();assert_eq!(repo.refname_to_id("refs/heads/linked").unwrap(),repo.head().unwrap().target().unwrap());
        let listing=session.worktrees(&after.revision).unwrap();assert_eq!(listing.entries.len(),1);assert!(listing.entries[0].valid);assert_eq!(listing.entries[0].branch,"linked");
        assert!(session.mutate(&after.revision,Action::DeleteBranch{name:"linked".into()}).unwrap_err().contains("BRANCH_IN_OTHER_WORKTREE"));
        assert_eq!(preserved(&root),before);assert!(repo.find_reference("refs/heads/linked").is_ok());
    }
    #[test]
    fn existing_inside_and_stale_worktree_destinations_preserve_user_data() {
        for case in ["existing","inside","stale"] {
            let (_d,root,target)=fixture();let mut session=Session::new(root.clone()).unwrap();let state=session.snapshot().unwrap();
            let target=if case=="inside" {root.join("inside")}else{target};
            if case=="existing" {fs::create_dir(&target).unwrap();fs::write(target.join("user-file"),"preserve\n").unwrap();}
            if case=="stale" {fs::write(root.join("file"),"later edit\n").unwrap();}
            let before=preserved(&root);assert!(session.mutate(&state.revision,create_action(&target)).is_err());assert_eq!(preserved(&root),before);
            if case=="existing" {assert_eq!(fs::read(target.join("user-file")).unwrap(),b"preserve\n");}else{assert!(!target.exists());}
            assert!(!root.join(".git/worktrees/linked").exists());assert!(Repository::open(&root).unwrap().find_reference("refs/heads/linked").is_err());
        }
    }
    #[test]
    fn creating_another_worktree_cannot_reuse_checked_out_branch() {
        let (_d,root,target)=fixture();let mut session=Session::new(root.clone()).unwrap();let state=session.snapshot().unwrap();
        let current=state.branch.clone();let before=preserved(&root);
        assert!(session.mutate(&state.revision,Action::CreateWorktree{name:"duplicate".into(),path:target.to_str().unwrap().into(),branch:Some(current)}).is_err());
        assert!(!target.exists());assert!(!root.join(".git/worktrees/duplicate").exists());assert_eq!(preserved(&root),before);
        let state=session.snapshot().unwrap();let state=session.mutate(&state.revision,create_action(&target)).unwrap();
        let duplicate=target.with_file_name("duplicate");
        assert!(session.mutate(&state.revision,Action::CreateWorktree{name:"duplicate".into(),path:duplicate.to_str().unwrap().into(),branch:Some("linked".into())}).is_err());
        assert!(!duplicate.exists());assert!(!root.join(".git/worktrees/duplicate").exists());assert_eq!(preserved(&root),before);
        assert_eq!(fs::read(target.join("file")).unwrap(),b"committed\n");
    }
    #[test]
    fn worktree_lock_expected_state_refuses_stale_unlock_and_preserves_files() {
        let (_d,root,target)=fixture();let mut session=Session::new(root.clone()).unwrap();let state=session.snapshot().unwrap();
        let state=session.mutate(&state.revision,create_action(&target)).unwrap();let before=preserved(&root);let linked=fs::read(target.join("file")).unwrap();
        let state=session.mutate(&state.revision,Action::SetWorktreeLock{name:"linked".into(),expected_locked:false,locked:true}).unwrap();
        assert!(session.worktrees(&state.revision).unwrap().entries[0].locked);
        let lock=root.join(".git/worktrees/linked/locked");let lock_bytes=fs::read(&lock).unwrap();
        assert!(session.mutate(&state.revision,Action::SetWorktreeLock{name:"linked".into(),expected_locked:false,locked:false}).unwrap_err().contains("STALE_WORKTREE_LOCK"));
        assert_eq!(fs::read(&lock).unwrap(),lock_bytes);
        let state=session.snapshot().unwrap();let state=session.mutate(&state.revision,Action::SetWorktreeLock{name:"linked".into(),expected_locked:true,locked:false}).unwrap();
        assert!(!session.worktrees(&state.revision).unwrap().entries[0].locked);assert!(!lock.exists());
        assert_eq!(preserved(&root),before);assert_eq!(fs::read(target.join("file")).unwrap(),linked);
    }
    #[test]
    fn worktree_checkout_reports_linked_head_and_changes_without_writes(){
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path().join("main");let linked=temp.path().join("linked folder");
        let repo=Repository::init(&root).unwrap();
        fs::write(root.join("file.txt"),"committed\n").unwrap();
        let mut index=repo.index().unwrap();index.add_path(Path::new("file.txt")).unwrap();index.write().unwrap();
        let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        let oid=repo.commit(Some("HEAD"),&signature,&signature,"Initial",&tree,&[]).unwrap();
        repo.worktree("linked",&linked,None).unwrap();
        fs::write(linked.join("file.txt"),"local change\n").unwrap();
        let main_index=fs::read(root.join(".git/index")).unwrap();
        let linked_index=fs::read(root.join(".git/worktrees/linked/index")).unwrap();
        let mut session=Session::new(root.clone()).unwrap();let snapshot=session.snapshot().unwrap();
        let checkout=session.worktree_checkout(&snapshot.revision,"linked").unwrap();
        assert_eq!(checkout.head_oid,Some(oid.to_string()));assert_eq!(checkout.changed,1);assert_eq!(checkout.unstaged,1);assert_eq!(checkout.staged,0);
        assert!(checkout.write_block.unwrap().contains("Stage, Unstage and Commit"));
        assert!(Session::new(linked.clone()).unwrap().snapshot().unwrap().linked_local_writes);
        assert!(session.worktree_checkout(&snapshot.revision,"../outside").is_err());
        assert_eq!(main_index,fs::read(root.join(".git/index")).unwrap());
        assert_eq!(linked_index,fs::read(root.join(".git/worktrees/linked/index")).unwrap());
        assert_eq!(fs::read_to_string(linked.join("file.txt")).unwrap(),"local change\n");
    }
}
