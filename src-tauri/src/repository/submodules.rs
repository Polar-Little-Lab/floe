use super::*;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Entry { pub path:String, pub head_oid:Option<String>, pub index_oid:Option<String>, pub directory_present:bool, pub conflicted:bool, pub path_error:Option<String> }
#[derive(Serialize)]
pub struct Listing { pub entries:Vec<Entry>, pub declarations:Vec<Declaration>, pub warning:Option<String>, pub truncated:bool }
#[derive(Default,Serialize)]
#[serde(rename_all="camelCase")]
pub struct Declaration { pub name:String, pub path:Option<String>, pub url:Option<String>, pub branch:Option<String>, pub path_error:Option<String> }
fn declarations(root:&Path)->Result<(Vec<Declaration>,bool)> {
    let path=root.join(".gitmodules");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok((Vec::new(),false)),
        Err(_)=>return Err("Cannot read .gitmodules".into()),
        Ok(meta) if !meta.is_file()||is_link(&meta)=>return Err("Unsupported .gitmodules file".into()),
        Ok(_)=>{},
    }
    let bytes=bounded(&path,65536).map_err(|_|"Cannot read bounded .gitmodules".to_string())?;
    let values=remote_remove::entries(&bytes).map_err(|_|"Invalid or unsupported .gitmodules; includes are not followed".to_string())?;
    let mut declarations=std::collections::BTreeMap::<String,Declaration>::new();
    let mut truncated=false;
    for (key,value) in values {
        let Some(key)=key.strip_prefix("submodule.") else {continue};
        let Some((name,field))=key.rsplit_once('.') else {continue};
        if !matches!(field,"path"|"url"|"branch") {continue;}
        if !declarations.contains_key(name)&&declarations.len()>=500 {truncated=true;continue;}
        let entry=declarations.entry(name.into()).or_insert_with(||Declaration{name:name.into(),..Default::default()});
        match field {"path"=>entry.path=Some(value),"url"=>entry.url=Some(remotes::display_url(&value)),"branch"=>entry.branch=Some(value),_=>{}}
    }
    let mut result:Vec<_>=declarations.into_values().collect();
    for entry in &mut result {
        entry.path_error=match &entry.path {Some(path)=>rel(root,Path::new(path)).err(),None=>Some("Missing path declaration".into())};
    }
    Ok((result,truncated))
}
pub use super::checkout_status::Checkout;
impl Session {
    pub fn submodule_checkout(&mut self,revision:&str,path:&str)->Result<Checkout>{
        let full=self.submodule_path(revision,path)?;
        let checkout=checkout_status::read(full.clone(),path)?;
        if self.submodule_path(revision,path)?!=full {return Err("SUBMODULE_CHANGED".into());}
        Ok(checkout)
    }
    pub fn submodule_path(&mut self,revision:&str,path:&str)->Result<PathBuf>{
        if path.len()>4096{return Err("INVALID_PATH".into());}
        let listing=self.submodules(revision)?;
        let entry=listing.entries.iter().find(|entry|entry.path==path).ok_or("INVALID_SUBMODULE")?;
        if !entry.directory_present || entry.path_error.is_some(){return Err("SUBMODULE_NOT_AVAILABLE".into());}
        let full=rel(&self.root,Path::new(path))?.canonicalize().map_err(err)?;
        if !full.starts_with(&self.root)||full==self.root{return Err("INVALID_SUBMODULE_PATH".into());}
        validate(&full)?;
        self.fresh(revision)?;Ok(full)
    }
    pub fn submodules(&mut self, revision:&str)->Result<Listing> {
        self.fresh(revision)?;
        let repo=open(&self.root)?;
        let mut links=std::collections::BTreeMap::<String,(Option<String>,Option<String>,bool)>::new();
        let mut truncated=false;
        if let Ok(head)=repo.head().and_then(|r|r.peel_to_commit()) {
            let tree=head.tree().map_err(err)?;
            let mut visited=0usize;
            tree.walk(git2::TreeWalkMode::PreOrder,|prefix,entry|{
                visited+=1;
                if visited>50000 || links.len()>=500 {truncated=true;return git2::TreeWalkResult::Abort;}
                if entry.filemode()==0o160000 {
                    if let Ok(name)=entry.name(){links.insert(format!("{prefix}{name}"),(Some(entry.id().to_string()),None,false));}
                }
                git2::TreeWalkResult::Ok
            }).or_else(|e|if truncated {Ok(())}else{Err(e)}).map_err(err)?;
        }
        let index=repo.index().map_err(err)?;
        for (visited,entry) in index.iter().enumerate() {
            if visited>=50000 {truncated=true;break;}
            if entry.mode!=0o160000 {continue;}
            let path=std::str::from_utf8(&entry.path).map_err(err)?.to_owned();
            if !links.contains_key(&path)&&links.len()>=500 {truncated=true;continue;}
            let value=links.entry(path).or_default();
            if (entry.flags >> 12) & 3 != 0 {value.2=true;} else {value.1=Some(entry.id.to_string());}
        }
        let mut entries=Vec::new();
        for (path,(head_oid,index_oid,conflicted)) in links {
            let (directory_present,path_error)=match rel(&self.root,Path::new(&path)) {
                Ok(full)=>(full.is_dir(),None),Err(error)=>(false,Some(error)),
            };
            entries.push(Entry{path,head_oid,index_oid,directory_present,conflicted,path_error});
        }
        let (declarations,warning)=match declarations(&self.root) {
            Ok((values,limited))=>{truncated|=limited;(values,None)},
            Err(error)=>(Vec::new(),Some(error)),
        };
        self.fresh(revision)?;
        Ok(Listing{entries,declarations,warning,truncated})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkout_reports_local_head_and_dirty_files_without_writes(){
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path().canonicalize().unwrap();
        Repository::init(&root).unwrap();
        let child=root.join("module with spaces");
        let repo=Repository::init(&child).unwrap();
        fs::write(child.join("tracked.txt"),"committed\n").unwrap();
        let mut index=repo.index().unwrap();index.add_path(Path::new("tracked.txt")).unwrap();index.write().unwrap();
        let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        let oid=repo.commit(Some("HEAD"),&signature,&signature,"Initial",&tree,&[]).unwrap();
        let result=std::process::Command::new("git").arg("-C").arg(&root).args(["update-index","--add","--cacheinfo"]).arg(format!("160000,{oid},module with spaces")).output().unwrap();
        assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
        fs::write(child.join("tracked.txt"),"local change\n").unwrap();
        fs::write(child.join("new.txt"),"untracked\n").unwrap();
        let parent_index=fs::read(root.join(".git/index")).unwrap();
        let child_index=fs::read(child.join(".git/index")).unwrap();
        let mut session=Session::new(root.clone()).unwrap();let snapshot=session.snapshot().unwrap();
        let checkout=session.submodule_checkout(&snapshot.revision,"module with spaces").unwrap();
        assert_eq!(checkout.head_oid,Some(oid.to_string()));
        assert!(!checkout.detached);assert_eq!(checkout.staged,0);assert_eq!(checkout.unstaged,2);assert_eq!(checkout.changed,2);
        assert!(session.submodule_checkout(&snapshot.revision,"../outside").is_err());
        assert!(session.submodule_checkout(&snapshot.revision,"unknown").is_err());
        assert_eq!(parent_index,fs::read(root.join(".git/index")).unwrap());
        assert_eq!(child_index,fs::read(child.join(".git/index")).unwrap());
        assert_eq!(fs::read_to_string(child.join("tracked.txt")).unwrap(),"local change\n");
    }
}
