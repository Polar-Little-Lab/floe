//! WIP observations only for already trusted, open sessions. Persisted paths
//! never cause discovery, opening, canonicalization or repository mutations.
use serde::Serialize;
use super::{named_workspaces::Group,repository::Session};
#[derive(Clone,Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct LocalStatus {
    pub state:&'static str,pub repo_id:Option<String>,pub branch:Option<String>,
    pub changed:Option<usize>,pub staged:Option<usize>,pub unstaged:Option<usize>,pub warning:Option<String>,
}
#[derive(Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct Entry {pub repository:String,pub local_path:Option<String>,#[serde(flatten)] pub local:LocalStatus}
#[derive(Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct Listing {pub workspace_id:String,pub entries:Vec<Entry>}
fn unavailable(state:&'static str)->LocalStatus {LocalStatus{state,repo_id:None,branch:None,changed:None,staged:None,unstaged:None,warning:None}}
pub fn read(group:&Group,sessions:&mut [Session])->Listing {
    let mut cache=std::collections::HashMap::<String,LocalStatus>::new();
    let entries=group.repositories.iter().map(|member|{
        let local=match member.local_path.as_deref(){
            None=>unavailable("unmapped"),
            Some(path)=>cache.entry(path.to_string()).or_insert_with(||{
                // Mappings created from trusted sessions use their canonical
                // paths. Unknown aliases must be remapped, not opened here.
                let Some(session)=sessions.iter_mut().find(|s|s.root_path()==std::path::Path::new(path)) else{return unavailable("notOpen");};
                let id=Some(session.id().to_string());
                match session.snapshot(){
                    Ok(snapshot)=>LocalStatus{state:if snapshot.changes.is_empty(){"clean"}else{"changes"},repo_id:id,branch:Some(snapshot.branch),changed:Some(snapshot.changes.len()),staged:Some(snapshot.changes.iter().filter(|c|c.staged).count()),unstaged:Some(snapshot.changes.iter().filter(|c|c.unstaged).count()),warning:snapshot.write_block},
                    Err(error)=>LocalStatus{repo_id:id,warning:Some(error),..unavailable("error")},
                }
            }).clone(),
        };
        Entry{repository:format!("{}/{}",member.owner,member.repo),local_path:member.local_path.clone(),local}
    }).collect();
    Listing{workspace_id:group.id.clone(),entries}
}
#[cfg(test)]
mod tests {
 use super::*;use super::super::named_workspaces::Repository;
 fn group(path:Option<String>)->Group {Group{icon:String::new(),description:String::new(),color:0,id:"g".into(),name:"Test".into(),views:vec![],repositories:vec![Repository{owner:"o".into(),repo:"r".into(),local_path:path}]}}
 fn fixture()->tempfile::TempDir {let d=tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();git2::Repository::init(d.path()).unwrap();d}
 #[test] fn unmapped_and_unopened_are_not_clean_and_never_open_paths(){
  let d=fixture();let path=d.path().to_string_lossy().into_owned();let closed=read(&group(Some(path)),&mut []);assert_eq!(closed.entries[0].local.state,"notOpen");assert_eq!(closed.entries[0].local.changed,None);
  let missing=read(&group(Some(d.path().join("missing").to_string_lossy().into_owned())),&mut []);assert_eq!(missing.entries[0].local.state,"notOpen");assert!(!d.path().join("missing").exists());assert_eq!(read(&group(None),&mut []).entries[0].local.state,"unmapped");
 }
 #[test] fn trusted_session_reports_real_dirty_areas_without_writes(){
  let d=fixture();let mut session=Session::new(d.path().to_path_buf()).unwrap();let g=group(Some(d.path().to_string_lossy().into_owned()));assert_eq!(read(&g,std::slice::from_mut(&mut session)).entries[0].local.state,"clean");
  std::fs::write(d.path().join("file"),"staged\n").unwrap();let repo=git2::Repository::open(d.path()).unwrap();let mut index=repo.index().unwrap();index.add_path(std::path::Path::new("file")).unwrap();index.write().unwrap();std::fs::write(d.path().join("file"),"working\n").unwrap();std::fs::write(d.path().join("new"),"new\n").unwrap();let before=std::fs::read(d.path().join(".git/index")).unwrap();let result=read(&g,std::slice::from_mut(&mut session));let status=&result.entries[0].local;assert_eq!(status.state,"changes");assert_eq!(status.changed,Some(2));assert_eq!(status.staged,Some(1));assert_eq!(status.unstaged,Some(2));assert_eq!(status.repo_id.as_deref(),Some(session.id()));assert_eq!(std::fs::read(d.path().join(".git/index")).unwrap(),before);assert_eq!(std::fs::read_to_string(d.path().join("file")).unwrap(),"working\n");
 }
 #[test] fn missing_trusted_root_is_error_not_empty_worktree(){
  let d=fixture();let mut session=Session::new(d.path().to_path_buf()).unwrap();let g=group(Some(d.path().to_string_lossy().into_owned()));let moved=d.path().join(".git-moved");std::fs::rename(d.path().join(".git"),&moved).unwrap();let result=read(&g,std::slice::from_mut(&mut session));assert_eq!(result.entries[0].local.state,"error");assert!(result.entries[0].local.warning.is_some());assert_eq!(result.entries[0].local.changed,None);
 }
}
