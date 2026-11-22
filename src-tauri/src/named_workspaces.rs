//! Named provider repository groups, separate from last-session tabs.
//! Local mappings are observations, never grants to open or modify repositories.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "named-workspaces.json";
const LIMIT: u64 = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
pub struct Repository {
    pub owner:String, pub repo:String, pub local_path:Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {#[serde(default)] pub icon:String,#[serde(default)] pub description:String,#[serde(default)] pub color:u8,pub id:String,pub name:String,pub repositories:Vec<Repository>,#[serde(default)] pub views:Vec<SavedView>}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum TaskKind {All,Pulls,Issues,Wips,Snoozed}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewFilters {pub author:Option<String>,pub label:Option<String>,pub milestone:Option<String>}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {pub id:String,pub name:String,pub query:String,pub kind:TaskKind,pub filters:ViewFilters}
fn bounded_text(value:&str,limit:usize)->bool {value.len()<=limit&&!value.chars().any(char::is_control)}
fn validate_views(views:&[SavedView])->Result<(),String>{
    if views.len()>50 {return Err("SAVED_VIEW_LIMIT".into());}
    let mut seen=std::collections::HashSet::new();
    for view in views {
        if view.id.is_empty()||view.id.len()>64||!view.id.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b))||!seen.insert(&view.id)
            ||view.name.trim().is_empty()||!bounded_text(&view.name,256)||!bounded_text(&view.query,4096)
            ||view.filters.author.as_ref().is_some_and(|v|!bounded_text(v,256))||view.filters.label.as_ref().is_some_and(|v|!bounded_text(v,256)) {return Err("INVALID_SAVED_VIEW".into());}
        if let Some(identity)=&view.filters.milestone {
            if identity.len()>512 {return Err("INVALID_SAVED_VIEW_MILESTONE".into());}
            let (repository,number):(String,String)=serde_json::from_str(identity).map_err(|_|"INVALID_SAVED_VIEW_MILESTONE")?;
            let (owner,repo)=repository.split_once('/').ok_or("INVALID_SAVED_VIEW_MILESTONE")?;
            super::provider_github::validate(owner,repo,1)?;
            let number_value=number.parse::<u64>().map_err(|_|"INVALID_SAVED_VIEW_MILESTONE")?;
            if number_value==0||number_value.to_string()!=number||repository.to_ascii_lowercase()!=repository
                ||serde_json::to_string(&(&repository,&number)).map_err(|_|"INVALID_SAVED_VIEW_MILESTONE")?!=*identity {return Err("INVALID_SAVED_VIEW_MILESTONE".into());}
        }
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {pub groups:Vec<Group>,pub selected:Option<String>,#[serde(default)] pub snoozes:Vec<TaskSnooze>,#[serde(default)] pub pins:Vec<TaskIdentity>}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIdentity {pub provider:String,pub host:String,pub account:String,pub repository:String,pub kind:SnoozeKind,pub id:String}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum SnoozeKind {Pulls,Issues}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSnooze {pub task:TaskIdentity,pub until:u64}
fn validate_task(t:&TaskIdentity)->Result<String,String>{
 if t.provider!="github"||t.host!="github.com"||t.account!="anonymous"||t.id.is_empty()||!bounded_text(&t.id,256){return Err("INVALID_TASK_IDENTITY".into());}
 let (owner,repo)=t.repository.split_once('/').ok_or("INVALID_TASK_REPOSITORY")?;super::provider_github::validate(owner,repo,1)?;
 if t.repository!=t.repository.to_ascii_lowercase(){return Err("INVALID_TASK_IDENTITY".into());}
 serde_json::to_string(t).map_err(|_|"INVALID_TASK_IDENTITY".into())
}
fn validate_snoozes(entries:&[TaskSnooze])->Result<(),String>{
 if entries.len()>1000{return Err("SNOOZE_LIMIT".into());}let mut seen=std::collections::HashSet::new();
 for entry in entries {if entry.until==0||entry.until>9_007_199_254_740_991||!seen.insert(validate_task(&entry.task)?){return Err("INVALID_SNOOZE".into());}}Ok(())
}
fn validate_pins(entries:&[TaskIdentity])->Result<(),String>{
 if entries.len()>1000{return Err("TASK_PIN_LIMIT".into());}let mut seen=std::collections::HashSet::new();
 for entry in entries {if !seen.insert(validate_task(entry)?){return Err("DUPLICATE_TASK_PIN".into());}}Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {version:u32,workspace:Option<Workspace>}
#[derive(Debug, Serialize)]
pub struct State {pub workspace:Option<Workspace>,pub revision:String,pub warning:Option<String>}
fn validate(workspace:&Workspace)->Result<(),String>{
    validate_snoozes(&workspace.snoozes)?;
    validate_pins(&workspace.pins)?;
    if workspace.groups.len()>50 {return Err("NAMED_WORKSPACE_LIMIT".into());}
    let mut ids=std::collections::HashSet::new();
    for group in &workspace.groups {
        if group.id.is_empty()||group.id.len()>64||!group.id.bytes().all(|b|b.is_ascii_alphanumeric()||b"-_".contains(&b))||!ids.insert(group.id.as_str())
            ||group.name.trim().is_empty()||group.name.len()>256||group.name.chars().any(char::is_control)||group.repositories.len()>100 {
            return Err("INVALID_NAMED_WORKSPACE".into());
        }
        if group.description.len()>4096 || group.description.chars().any(|c|c.is_control()&&c!='\n'&&c!='\r'&&c!='\t') || group.color>5 || !["","folder","branch","rocket","tag"].contains(&group.icon.as_str()) {
            return Err("INVALID_WORKSPACE_APPEARANCE".into());
        }
        validate_views(&group.views)?;
        let mut repos=std::collections::HashSet::new();
        for repo in &group.repositories {
            super::provider_github::validate(&repo.owner,&repo.repo,1)?;
            if !repos.insert((repo.owner.to_ascii_lowercase(),repo.repo.to_ascii_lowercase())) {return Err("DUPLICATE_WORKSPACE_REPOSITORY".into());}
            if let Some(path)=&repo.local_path {super::repository_path(path)?;}
        }
    }
    if workspace.selected.as_ref().is_some_and(|id|!ids.contains(id.as_str())) {return Err("INVALID_SELECTED_WORKSPACE".into());}
    Ok(())
}
fn check(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err("WORKSPACE_SYMLINK".into()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("WORKSPACE_ACCESS_FAILED".into()),
        }
    }
    Ok(())
}
fn bytes(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = dir.join(FILE);
    check(&path)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("WORKSPACE_ACCESS_FAILED".into()),
        Ok(m) if !m.is_file() || m.len() > LIMIT => return Err("INVALID_WORKSPACE_FILE".into()),
        Ok(_) => (),
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "WORKSPACE_ACCESS_FAILED")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "WORKSPACE_READ_FAILED")?;
    if bytes.len() as u64 > LIMIT {
        return Err("INVALID_WORKSPACE_FILE".into());
    }
    Ok(Some(bytes))
}
fn revision(bytes: &Option<Vec<u8>>) -> String {
    match bytes {
        None => "missing".into(),
        Some(bytes) => format!("{:x}", Sha256::digest(bytes)),
    }
}
pub fn read(dir: &Path) -> Result<State, String> {
    let bytes = bytes(dir)?;
    let revision = revision(&bytes);
    let Some(bytes) = bytes else {
        return Ok(State {
            workspace: None,
            revision,
            warning: None,
        });
    };
    let parsed = serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|r| r.version == 1 && r.workspace.as_ref().is_none_or(|i| validate(i).is_ok()));
    match parsed {
        Some(r) => Ok(State {
            workspace: r.workspace,
            revision,
            warning: None,
        }),
        None => Ok(State {
            workspace: None,
            revision,
            warning: Some(
                "Saved named workspaces are invalid. Original data was preserved; repair or restore the settings file before saving.".into(),
            ),
        }),
    }
}
// Explicitly unlock before close: a concurrent subprocess may briefly inherit
// the file description before exec, extending a close-only flock lifetime.
struct HeldLock(fs::File);
impl Drop for HeldLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
pub fn save(dir: &Path, expected: &str, workspace: Option<Workspace>) -> Result<State, String> {
    if expected.len() > 64 {
        return Err("INVALID_WORKSPACE_REVISION".into());
    }
    if let Some(i) = &workspace {
        validate(i)?;
    }
    check(dir)?;
    fs::create_dir_all(dir).map_err(|_| "WORKSPACE_ACCESS_FAILED")?;
    let lock_path = dir.join("named-workspaces.lock");
    check(&lock_path)?;
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
        return Err("INVALID_WORKSPACE_LOCK".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(lock_path)
        .map_err(|_| "WORKSPACE_LOCK_FAILED")?;
    lock.try_lock().map_err(|_| "WORKSPACE_BUSY")?;
    let _held_lock = HeldLock(lock);
    // Persistent inode; guard releases on every return, OS releases on process exit.
    if revision(&bytes(dir)?) != expected {
        return Err("STALE_WORKSPACE: Reload settings before saving.".into());
    }
    if read(dir)?.warning.is_some() {return Err("INVALID_NAMED_WORKSPACE_FILE: Original data preserved".into());}
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    let encoded = serde_json::to_vec(&Record {
        version: 1,
        workspace,
    })
    .map_err(|_| "WORKSPACE_SERIALIZE_FAILED")?;
    if encoded.len() as u64 > LIMIT {
        return Err("WORKSPACE_LIMIT".into());
    }
    temp.write_all(&encoded)
        .map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "WORKSPACE_SYNC_FAILED")?;
    temp.persist(dir.join(FILE))
        .map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| "WORKSPACE_SYNC_FAILED")?;
    read(dir)
}
#[cfg(test)]
mod tests {
 use super::*;
 fn dir()->tempfile::TempDir {tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()}
 fn value(d:&Path)->Workspace {Workspace{pins:vec![],snoozes:vec![],groups:vec![Group{icon:String::new(),description:String::new(),color:0,views:vec![],id:"group-1".into(),name:"팀 프로젝트".into(),repositories:vec![Repository{owner:"Owner".into(),repo:"Repo".into(),local_path:Some(d.join("missing-repo").to_string_lossy().into_owned())}]}],selected:Some("group-1".into())}}
 fn saved_view()->SavedView {SavedView{id:"view-1".into(),name:"Release bugs".into(),query:"[literal] 한글".into(),kind:TaskKind::Issues,filters:ViewFilters{author:Some("Alice".into()),label:Some("".into()),milestone:Some(r#"["owner/repo","9007199254740993"]"#.into())}}}
 #[test] fn saved_views_roundtrip_and_legacy_groups_default_empty(){
  let d=dir();let mut v=value(d.path());let legacy=serde_json::json!({"version":1,"workspace":{"groups":[{"id":"group-1","name":"Legacy","repositories":[]}],"selected":"group-1"}});fs::write(d.path().join(FILE),serde_json::to_vec(&legacy).unwrap()).unwrap();let loaded=read(d.path()).unwrap();assert!(loaded.warning.is_none());assert!(loaded.workspace.unwrap().groups[0].views.is_empty());
  v.groups[0].views.push(saved_view());let saved=save(d.path(),&loaded.revision,Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v.clone()));
  v.groups[0].name="Renamed".into();v.groups[0].repositories.clear();let updated=save(d.path(),&saved.revision,Some(v.clone())).unwrap();assert_eq!(updated.workspace.unwrap().groups[0].views,vec![saved_view()]);
 }
 #[test] fn snooze_roundtrip_identity_validation_and_stale_preservation(){
  let d=dir();let mut v=value(d.path());let task=TaskIdentity{provider:"github".into(),host:"github.com".into(),account:"anonymous".into(),repository:"owner/repo".into(),kind:SnoozeKind::Pulls,id:"9007199254740993".into()};v.snoozes.push(TaskSnooze{task,until:1_800_000_000_000});let saved=save(d.path(),"missing",Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v.clone()));let original=fs::read(d.path().join(FILE)).unwrap();
  let mut invalid=v.clone();invalid.snoozes.push(invalid.snoozes[0].clone());assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());
  for field in ["provider","host","account","repository","id"] {let mut invalid=v.clone();let t=&mut invalid.snoozes[0].task;match field {"provider"=>t.provider="other".into(),"host"=>t.host="other.example".into(),"account"=>t.account="different-user".into(),"repository"=>t.repository="Owner/repo".into(),_=>t.id="".into()};assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());}
  let mut invalid=v.clone();invalid.snoozes[0].until=u64::MAX;assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());assert!(save(d.path(),"missing",Some(v.clone())).is_err());assert_eq!(fs::read(d.path().join(FILE)).unwrap(),original);
  v.snoozes.clear();save(d.path(),&saved.revision,Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v));
 }
 #[test] fn pins_roundtrip_and_coexist_with_snooze_without_cross_type_collision(){
  let d=dir();let mut v=value(d.path());let task=TaskIdentity{provider:"github".into(),host:"github.com".into(),account:"anonymous".into(),repository:"owner/repo".into(),kind:SnoozeKind::Pulls,id:"1".into()};v.pins.push(task.clone());let mut issue=task.clone();issue.kind=SnoozeKind::Issues;v.pins.push(issue);v.snoozes.push(TaskSnooze{task,until:12345});let saved=save(d.path(),"missing",Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v.clone()));let bytes=fs::read(d.path().join(FILE)).unwrap();let mut invalid=v.clone();invalid.pins.push(invalid.pins[0].clone());assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());let mut invalid=v.clone();invalid.pins[0].host="other.example".into();assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());assert_eq!(fs::read(d.path().join(FILE)).unwrap(),bytes);v.pins.clear();save(d.path(),&saved.revision,Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v));
 }
 #[test] fn wip_view_roundtrip_retains_local_mapping(){
  let d=dir();let mut v=value(d.path());let mut view=saved_view();view.kind=TaskKind::Wips;v.groups[0].views.push(view);save(d.path(),"missing",Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v));assert!(!d.path().join("missing-repo").exists());
 }
 #[test] fn malformed_views_refused_without_changing_storage(){
  let d=dir();let mut v=value(d.path());v.groups[0].views.push(saved_view());let saved=save(d.path(),"missing",Some(v.clone())).unwrap();let original=fs::read(d.path().join(FILE)).unwrap();
  let mut bad=v.clone();bad.groups[0].views.push(saved_view());assert!(save(d.path(),&saved.revision,Some(bad)).is_err());
  for bad in [r#"["owner/repo","0"]"#,r#"["owner/repo","01"]"#,r#"["owner/repo","18446744073709551616"]"#,r#"["Owner/repo","1"]"#,r#"["owner/repo/extra","1"]"#,"not json"] {let mut invalid=v.clone();invalid.groups[0].views[0].filters.milestone=Some(bad.into());assert!(save(d.path(),&saved.revision,Some(invalid)).is_err());}
  let mut bad=v.clone();bad.groups[0].views[0].query="x".repeat(4097);assert!(save(d.path(),&saved.revision,Some(bad)).is_err());let mut bad=v.clone();bad.groups[0].views[0].name="bad\nname".into();assert!(save(d.path(),&saved.revision,Some(bad)).is_err());assert_eq!(fs::read(d.path().join(FILE)).unwrap(),original);
  let mut json:serde_json::Value=serde_json::from_slice(&original).unwrap();json["workspace"]["groups"][0]["views"][0]["kind"]=serde_json::json!("anything");assert!(serde_json::from_value::<Record>(json).is_err());
 }
 #[test] fn groups_survive_restart_missing_local_paths_and_reorder(){
  let d=dir();let mut v=value(d.path());let saved=save(d.path(),"missing",Some(v.clone())).unwrap();assert_eq!(read(d.path()).unwrap().workspace,Some(v.clone()));
  let mut second=v.groups[0].clone();second.id="group-2".into();second.name="Other".into();v.groups.insert(0,second);v.selected=Some("group-2".into());let saved=save(d.path(),&saved.revision,Some(v.clone())).unwrap();assert_eq!(saved.workspace,Some(v.clone()));
  v.groups.remove(1);assert_eq!(save(d.path(),&saved.revision,Some(v.clone())).unwrap().workspace,Some(v));assert!(!d.path().join("workspace.json").exists());assert!(!d.path().join("missing-repo").exists());
 }
 #[test] fn invalid_groups_never_replace_existing_bytes(){
  let d=dir();let v=value(d.path());let saved=save(d.path(),"missing",Some(v.clone())).unwrap();let before=fs::read(d.path().join(FILE)).unwrap();
  let mut bad=v.clone();bad.groups.push(bad.groups[0].clone());assert!(save(d.path(),&saved.revision,Some(bad)).is_err());
  let mut bad=v.clone();let mut repo=bad.groups[0].repositories[0].clone();repo.owner="owner".into();repo.repo="repo".into();bad.groups[0].repositories.push(repo);assert!(save(d.path(),&saved.revision,Some(bad)).is_err());
  let mut bad=v.clone();bad.selected=Some("absent".into());assert!(save(d.path(),&saved.revision,Some(bad)).is_err());
  let mut bad=v.clone();bad.groups[0].repositories[0].owner="evil/host".into();assert!(save(d.path(),&saved.revision,Some(bad)).is_err());
  let mut bad=v;bad.groups[0].name="  ".into();assert!(save(d.path(),&saved.revision,Some(bad)).is_err());assert_eq!(fs::read(d.path().join(FILE)).unwrap(),before);
 }
 #[test] fn stale_concurrent_writers_cannot_silently_overwrite(){
  let d=dir();let v=value(d.path());let first=save(d.path(),"missing",Some(v.clone())).unwrap();assert!(save(d.path(),"missing",Some(v.clone())).unwrap_err().contains("STALE"));
  let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));let mut threads=vec![];
  for n in 0..2 {let path=d.path().to_path_buf();let rev=first.revision.clone();let mut next=v.clone();next.groups[0].name=format!("Writer{n}");let b=barrier.clone();threads.push(std::thread::spawn(move||{b.wait();save(&path,&rev,Some(next))}));}
  let results:Vec<_>=threads.into_iter().map(|t|t.join().unwrap()).collect();assert_eq!(results.iter().filter(|r|r.is_ok()).count(),1);assert!(read(d.path()).unwrap().warning.is_none());
 }
 #[test] fn corrupted_file_preserved_even_with_matching_revision(){
  let d=dir();let original=b"{broken record}";fs::write(d.path().join(FILE),original).unwrap();let loaded=read(d.path()).unwrap();assert!(loaded.warning.is_some());assert!(save(d.path(),&loaded.revision,Some(value(d.path()))).is_err());assert_eq!(fs::read(d.path().join(FILE)).unwrap(),original);
 }
 #[cfg(unix)]
 #[test] fn symlink_file_and_lock_refused(){
  use std::os::unix::fs::symlink;
  let d=dir();let target=d.path().join("target");fs::write(&target,b"preserve").unwrap();symlink(&target,d.path().join(FILE)).unwrap();assert!(read(d.path()).is_err());fs::remove_file(d.path().join(FILE)).unwrap();symlink(&target,d.path().join("named-workspaces.lock")).unwrap();assert!(save(d.path(),"missing",Some(value(d.path()))).is_err());assert_eq!(fs::read(target).unwrap(),b"preserve");
 }
}
