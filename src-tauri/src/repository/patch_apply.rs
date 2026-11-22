use super::*;
pub(super) fn execute(session:&mut Session,repo:&Repository,revision:&str,text:&str,lock:&mut IndexLock)->Result<()>{
    let branch=external_merge::head_lock_target(repo)?;
    let _refs=checkout::locks(repo,&branch,&branch)?;
    let check=session.check_patch(revision,text,patch_check::Target::Index)?;
    if !check.applicable{return Err(format!("PATCH_NOT_APPLICABLE: {}",check.message));}
    let diff=crate::patch_preview::parse_diff(text)?;
    let mut paths=std::collections::BTreeSet::new();
    for delta in diff.deltas(){for file in [delta.old_file(),delta.new_file()]{if let Some(path)=file.path(){rel(&session.root,path)?;paths.insert(path.to_path_buf());}}}
    let mut index=git2::Index::open(&lock.path).map_err(err)?;
    if index.has_conflicts(){return Err("UNRESOLVED_CONFLICTS".into());}
    let tree=repo.find_tree(index.write_tree_to(repo).map_err(err)?).map_err(err)?;
    let patched=repo.apply_to_tree(&tree,&diff,None).map_err(err)?;
    if patched.has_conflicts(){return Err("PATCH_CONFLICT".into());}
    for entry in patched.iter(){
        let path=Path::new(std::str::from_utf8(&entry.path).map_err(err)?);
        if !paths.contains(path){
            let previous=index.get_path(path,0).ok_or("UNEXPECTED_PATCH_ENTRY")?;
            if previous.id!=entry.id||previous.mode!=entry.mode{return Err("UNEXPECTED_PATCH_CHANGE".into());}
            continue;
        }
        if !matches!(entry.mode,0o100644|0o100755){return Err(format!("INVALID_PATCH_RESULT: {} mode {:o}",path.display(),entry.mode));}
        let (size,_)=repo.odb().map_err(err)?.read_header(entry.id).map_err(err)?;
        if size>FILE_LIMIT as usize{return Err("PATCH_LIMIT: resulting file exceeds 5 MiB".into());}
    }
    for path in &paths{if index.get_path(path,0).is_some(){index.remove_path(path).map_err(err)?;}}
    for entry in patched.iter(){let path=Path::new(std::str::from_utf8(&entry.path).map_err(err)?);if paths.contains(path){index.add(&entry).map_err(err)?;}}
    index.write().map_err(err)?;
    session.fresh(revision)?;
    external_merge::publish(session,revision,lock,&branch,None)
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn applies_only_to_index_preserves_other_entries_and_refuses_stale(){
  let temp=tempfile::tempdir().unwrap();let repo=Repository::init(temp.path()).unwrap();
  for(name,text)in[("file","old\n"),("untouched","keep\n")]{fs::write(temp.path().join(name),text).unwrap();}
  let mut index=repo.index().unwrap();for name in ["file","untouched"]{index.add_path(Path::new(name)).unwrap();}index.write().unwrap();
  let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();let head=repo.commit(Some("HEAD"),&sig,&sig,"Initial",&tree,&[]).unwrap();
  let mut session=Session::new(temp.path().into()).unwrap();let before=session.snapshot().unwrap();
  let patch="diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
  let after=session.mutate(&before.revision,Action::ApplyPatchIndex{text:patch.into()}).unwrap();assert_ne!(before.revision,after.revision);
  let index=open(temp.path()).unwrap().index().unwrap();let content=repo.find_blob(index.get_path(Path::new("file"),0).unwrap().id).unwrap();assert_eq!(content.content(),b"new\n");
  assert_eq!(fs::read(temp.path().join("file")).unwrap(),b"old\n");assert_eq!(repo.head().unwrap().target(),Some(head));
  assert_eq!(repo.find_blob(index.get_path(Path::new("untouched"),0).unwrap().id).unwrap().content(),b"keep\n");
  let saved=fs::read(temp.path().join(".git/index")).unwrap();
  assert!(session.mutate(&before.revision,Action::ApplyPatchIndex{text:patch.into()}).is_err());
  assert!(session.mutate(&after.revision,Action::ApplyPatchIndex{text:patch.into()}).is_err());
  assert_eq!(saved,fs::read(temp.path().join(".git/index")).unwrap());
  let changes="diff --git a/file b/file\ndeleted file mode 100644\n--- a/file\n+++ /dev/null\n@@ -1 +0,0 @@\n-new\ndiff --git a/added b/added\nnew file mode 100644\n--- /dev/null\n+++ b/added\n@@ -0,0 +1 @@\n+created\n";
  session.mutate(&after.revision,Action::ApplyPatchIndex{text:changes.into()}).unwrap();
  let index=open(temp.path()).unwrap().index().unwrap();assert!(index.get_path(Path::new("file"),0).is_none());
  assert_eq!(repo.find_blob(index.get_path(Path::new("added"),0).unwrap().id).unwrap().content(),b"created\n");
  assert!(!temp.path().join("added").exists());assert_eq!(fs::read(temp.path().join("file")).unwrap(),b"old\n");
  assert!(index.get_path(Path::new("untouched"),0).is_some());
 }
}

#[cfg(test)]
#[path="patch_apply_matrix.rs"]
mod matrix;
