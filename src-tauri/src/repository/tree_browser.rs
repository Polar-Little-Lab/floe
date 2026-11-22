use super::*;
#[derive(Serialize)]
pub struct Entry {pub id:String,pub name:String,pub path:String,pub kind:String,pub oid:String}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Listing {pub oid:String,pub path:String,pub entries:Vec<Entry>,pub truncated:bool,pub offset:usize,pub total:usize,pub next_offset:Option<usize>}
pub(super) fn authorize(session:&mut Session,revision:&str,oid:&str,path:&str)->Result<()> {
    if oid.len()!=40||!oid.bytes().all(|c|c.is_ascii_hexdigit())||path.len()>4096||path.contains(['\0','\\'])||(!path.is_empty()&&path.split('/').any(|part|part.is_empty()||part=="."||part=="..")){return Err("INVALID_TREE_SELECTION".into());}
    let snapshot=session.fresh(revision)?;
    if snapshot.head_oid.as_deref()!=Some(oid)&&!snapshot.commits.iter().any(|commit|commit.oid==oid){return Err("INVALID_COMMIT".into());}
    Ok(())
}
pub fn read(session:&mut Session,revision:&str,oid:&str,path:&str,offset:usize)->Result<Listing>{
    authorize(session,revision,oid,path)?;
    let repo=open(&session.root)?;
    let root=repo.find_commit(Oid::from_str(oid).map_err(err)?).map_err(err)?.tree().map_err(err)?;
    let tree=if path.is_empty(){root}else{let entry=root.get_path(Path::new(path)).map_err(err)?;if entry.kind()!=Some(git2::ObjectType::Tree){return Err("NOT_A_TREE".into());}repo.find_tree(entry.id()).map_err(err)?};
    if offset>tree.len(){return Err("INVALID_TREE_OFFSET".into());}
    let mut entries=Vec::new();
    for entry in tree.iter().skip(offset).take(MAX_FILES){
        let name=entry.name().map_err(err)?.to_string();
        let full=if path.is_empty(){name.clone()}else{format!("{path}/{name}")};
        if full.len()>4096||name.contains(['\0','\\','/'])||name=="."||name==".."{return Err("UNSUPPORTED_PATH".into());}
        let kind=match entry.filemode(){0o040000=>"folder",0o100644|0o100755=>"file",0o120000=>"symlink",0o160000=>"submodule",_=>"unsupported"};
        entries.push(Entry{id:file_id(&full),name,path:full,kind:kind.into(),oid:entry.id().to_string()});
    }
    entries.sort_by(|a,b|(a.kind!="folder").cmp(&(b.kind!="folder")).then_with(||a.name.cmp(&b.name)));
    session.fresh(revision)?;
    let end=offset+entries.len();let next_offset=if end<tree.len(){Some(end)}else{None};
    Ok(Listing{oid:oid.into(),path:path.into(),entries,truncated:next_offset.is_some(),offset,total:tree.len(),next_offset})
}

#[derive(Serialize)]
pub struct SearchHit {pub id:String,pub path:String,pub folder:String,pub offset:usize}
#[derive(Serialize)]
pub struct Search {pub query:String,pub hits:Vec<SearchHit>,pub scanned:usize,pub truncated:bool,pub skipped:usize}
pub fn search(session:&mut Session,revision:&str,oid:&str,query:&str)->Result<Search>{
    authorize(session,revision,oid,"")?;
    if query.len()>512||query.trim().is_empty()||query.contains('\0'){return Err("INVALID_SEARCH_QUERY".into());}
    let terms:Vec<_>=query.split_whitespace().map(str::to_lowercase).collect();
    let repo=open(&session.root)?;
    let tree=repo.find_commit(Oid::from_str(oid).map_err(err)?).map_err(err)?.tree().map_err(err)?;
    let mut result=Search{query:query.into(),hits:Vec::new(),scanned:0,truncated:false,skipped:0};
    let mut offsets=std::collections::HashMap::<String,usize>::new();
    tree.walk(git2::TreeWalkMode::PreOrder,|prefix,entry|{
        if result.scanned>=50000||result.hits.len()>=200||prefix.bytes().filter(|c|*c==b'/').count()>64 {result.truncated=true;return git2::TreeWalkResult::Abort;}
        result.scanned+=1;
        let position=offsets.entry(prefix.into()).or_default();let offset=(*position/MAX_FILES)*MAX_FILES;*position+=1;
        let Ok(name)=entry.name() else {result.skipped+=1;return git2::TreeWalkResult::Ok;};
        let path=format!("{prefix}{name}");
        if path.len()>4096||path.contains(['\0','\\'])||path.split('/').any(|part|part.is_empty()||part=="."||part=="..") {result.skipped+=1;return git2::TreeWalkResult::Ok;}
        if matches!(entry.filemode(),0o100644|0o100755)&&terms.iter().all(|term|path.to_lowercase().contains(term)){
            result.hits.push(SearchHit{id:file_id(&path),path,folder:prefix.trim_end_matches('/').into(),offset});
        }
        git2::TreeWalkResult::Ok
    }).or_else(|e|if result.truncated{Ok(())}else{Err(e)}).map_err(err)?;
    session.fresh(revision)?;Ok(result)
}

#[cfg(test)]
mod search_tests {
    use super::*;
    #[test]
    fn search_finds_nested_file_page_and_bounds_results_without_mutation(){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::init(temp.path()).unwrap();
        let blob=repo.blob(b"fixture\n").unwrap();
        let mut folder=repo.treebuilder(None).unwrap();
        for index in 0..2001{folder.insert(format!("file-{index:04}.txt"),blob,0o100644).unwrap();}
        folder.insert("z-target.txt",blob,0o100644).unwrap();
        let mut root=repo.treebuilder(None).unwrap();root.insert("folder",folder.write().unwrap(),0o040000).unwrap();
        let tree=repo.find_tree(root.write().unwrap()).unwrap();
        let signature=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();
        let oid=repo.commit(Some("HEAD"),&signature,&signature,"Initial",&tree,&[]).unwrap().to_string();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
        let before=fs::read(temp.path().join(".git/index")).unwrap();
        let mut session=Session::new(temp.path().into()).unwrap();let snapshot=session.snapshot().unwrap();
        let result=search(&mut session,&snapshot.revision,&oid,"FOLDER target").unwrap();
        assert_eq!(result.hits.len(),1);assert!(!result.truncated);
        let hit=&result.hits[0];assert_eq!(hit.path,"folder/z-target.txt");assert_eq!(hit.offset,2000);
        let listing=read(&mut session,&snapshot.revision,&oid,&hit.folder,hit.offset).unwrap();
        assert!(listing.entries.iter().any(|entry|entry.id==hit.id));
        let limited=search(&mut session,&snapshot.revision,&oid,"file-").unwrap();assert_eq!(limited.hits.len(),200);assert!(limited.truncated);
        assert!(search(&mut session,&snapshot.revision,&oid,"  ").is_err());
        assert!(search(&mut session,&snapshot.revision,&"0".repeat(40),"target").is_err());
        assert_eq!(before,fs::read(temp.path().join(".git/index")).unwrap());
    }
}
