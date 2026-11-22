use super::*;
#[derive(Clone,Copy,Deserialize,Serialize)]
#[serde(rename_all="camelCase")]
pub enum Target {Worktree,Index}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Check {pub target:Target,pub repo_id:String,pub revision:String,pub repository:String,pub applicable:bool,pub files:usize,pub message:String}
impl Session {
    pub fn check_patch(&mut self,revision:&str,text:&str,target:Target)->Result<Check>{
        let snapshot=self.fresh(revision)?;
        if let Some(reason)=snapshot.write_block{return Err(format!("READ_ONLY: {reason}"));}
        crate::patch_preview::parse("Review".into(),text.into())?;
        let diff=crate::patch_preview::parse_diff(text)?;
        let original=open(&self.root)?;let original_index=original.index().map_err(err)?;
        if original_index.has_conflicts(){return Err("UNRESOLVED_CONFLICTS".into());}
        let temporary=tempfile::tempdir().map_err(err)?;
        let isolated=Repository::init(temporary.path()).map_err(err)?;
        isolated.set_config(&Config::new().map_err(err)?).map_err(err)?;
        let mut index=isolated.index().map_err(err)?;
        let mut paths=std::collections::BTreeSet::new();
        for delta in diff.deltas(){
            for file in [delta.old_file(),delta.new_file()]{
                if !matches!(file.mode(),git2::FileMode::Unreadable|git2::FileMode::Blob|git2::FileMode::BlobExecutable){return Err("UNSUPPORTED_PATCH_MODE".into());}
                if let Some(path)=file.path(){
                    let path=path.to_str().ok_or("UNSUPPORTED_PATCH_PATH")?;
                    rel(&self.root,Path::new(path))?;
                    paths.insert(path.to_owned());
                }
            }
        }
        let mut total=0usize;
        for path in paths{
            if matches!(target,Target::Index){
                if let Some(mut entry)=original_index.get_path(Path::new(&path),0){
                    if !matches!(entry.mode,0o100644|0o100755){return Err("UNSUPPORTED_PATCH_MODE".into());}
                    let (size,_)=original.odb().map_err(err)?.read_header(entry.id).map_err(err)?;
                    if size>FILE_LIMIT as usize{return Err("PATCH_LIMIT: index file exceeds 5 MiB".into());}
                    total+=size;if total>50*1024*1024{return Err("PATCH_LIMIT: index inputs exceed 50 MiB".into());}
                    let blob=original.find_blob(entry.id).map_err(err)?;
                    entry.id=isolated.blob(blob.content()).map_err(err)?;
                    index.add(&entry).map_err(err)?;
                }
                continue;
            }
            let source=rel(&self.root,Path::new(&path))?;
            let metadata=match fs::symlink_metadata(&source){Ok(value)=>value,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>continue,Err(e)=>return Err(err(e))};
            if !metadata.is_file()||is_link(&metadata){return Err("UNSUPPORTED_PATCH_PATH".into());}
            let bytes=bounded(&source,FILE_LIMIT)?;total+=bytes.len();if total>50*1024*1024{return Err("PATCH_LIMIT: working inputs exceed 50 MiB".into());}
            let destination=rel(temporary.path(),Path::new(&path))?;
            fs::create_dir_all(destination.parent().ok_or("INVALID_PATCH_PATH")?).map_err(err)?;
            fs::write(&destination,bytes).map_err(err)?;fs::set_permissions(&destination,metadata.permissions()).map_err(err)?;
            if original_index.get_path(Path::new(&path),0).is_some(){index.add_path(Path::new(&path)).map_err(err)?;}
        }
        index.write().map_err(err)?;
        let mut options=git2::ApplyOptions::new();options.check(true);
        let location=match target{Target::Worktree=>git2::ApplyLocation::WorkDir,Target::Index=>git2::ApplyLocation::Index};
        let result=isolated.apply(&diff,location,Some(&mut options));
        self.fresh(revision)?;
        let description=match target{Target::Worktree=>"working files",Target::Index=>"staged index"};
        let (applicable,message)=match result{Ok(())=>(true,format!("Whole patch matches the inspected {description}. Nothing was applied.")),Err(error)=>(false,format!("Patch does not apply to the inspected {description}: {}",error.message()))};
        Ok(Check{target,repo_id:self.id.clone(),revision:revision.into(),repository:snapshot.name,applicable,files:diff.deltas().len(),message})
    }
}
#[cfg(test)]
mod tests{
 use super::*;
 fn files(root:&Path)->Vec<(String,Vec<u8>)>{let mut result=Vec::new();fn visit(root:&Path,dir:&Path,out:&mut Vec<(String,Vec<u8>)>){for entry in fs::read_dir(dir).unwrap(){let path=entry.unwrap().path();if path.is_dir(){visit(root,&path,out)}else{out.push((path.strip_prefix(root).unwrap().to_string_lossy().into(),fs::read(path).unwrap()));}}}visit(root,root,&mut result);result.sort();result}
 #[test]fn checks_success_failure_and_paths_without_writing_original_objects(){
  let temp=tempfile::tempdir().unwrap();let repo=Repository::init(temp.path()).unwrap();fs::write(temp.path().join("file"),"old\n").unwrap();
  let mut index=repo.index().unwrap();index.add_path(Path::new("file")).unwrap();index.write().unwrap();
  let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();repo.commit(Some("HEAD"),&sig,&sig,"Initial",&tree,&[]).unwrap();
  let mut session=Session::new(temp.path().into()).unwrap();let snapshot=session.snapshot().unwrap();let before=files(temp.path());
  let patch="diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
  assert!(session.check_patch(&snapshot.revision,patch,Target::Worktree).unwrap().applicable);
  assert!(!session.check_patch(&snapshot.revision,&patch.replace("-old","-absent"),Target::Worktree).unwrap().applicable);
  assert!(session.check_patch(&snapshot.revision,&patch.replace("a/file","a/.git/config").replace("b/file","b/.git/config"),Target::Worktree).is_err());
  assert_eq!(before,files(temp.path()));
  fs::write(temp.path().join("file"),"unstaged version\n").unwrap();let snapshot=session.snapshot().unwrap();let before=files(temp.path());
  assert!(session.check_patch(&snapshot.revision,patch,Target::Index).unwrap().applicable);
  assert!(!session.check_patch(&snapshot.revision,patch,Target::Worktree).unwrap().applicable);
  assert_eq!(before,files(temp.path()));
 }
}
