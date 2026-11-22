//! One durable upload outcome per shared repository. Pending never authorizes replay.
use super::super::{*,lease::OperationLease};
const FILE:&str="branchdesk-lfs-upload.json";
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub enum Status{Pending,Accepted,Verified,NotSent,Reviewed}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Observation{pub present:bool,pub checked_at:u64}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Record{
 version:u32,
 pub id:String,
 pub oid:String,
 pub size:String,
 pub endpoint:String,
 pub requires_verify:bool,
 pub status:Status,
 #[serde(default,skip_serializing_if="Option::is_none")]
 pub observation:Option<Observation>,
}
impl Record{
 pub fn unresolved(&self)->bool{self.status==Status::Pending||(self.status==Status::Accepted&&self.requires_verify)}
 fn validate(&self)->Result<()>{
  if self.version!=1||Uuid::parse_str(&self.id).is_err()||self.size.is_empty()||!self.size.bytes().all(|b|b.is_ascii_digit())||(self.size.len()>1&&self.size.starts_with('0')){return Err("INVALID_LFS_UPLOAD_RECORD".into());}
  super::protocol::Object{oid:self.oid.clone(),size:self.size.parse().map_err(|_|"INVALID_LFS_UPLOAD_RECORD")?}.validate()?;
  if remote_endpoint::https(&self.endpoint)?.as_str()!=self.endpoint{return Err("INVALID_LFS_UPLOAD_RECORD".into());}
  if self.observation.as_ref().is_some_and(|observation|observation.checked_at==0||observation.checked_at>253402300799){return Err("INVALID_LFS_UPLOAD_OBSERVATION".into());}
  Ok(())
 }
}
fn bytes(root:&Path)->Result<Option<Vec<u8>>>{
 let path=layout::Layout::read(root)?.common.join(FILE);
 match fs::symlink_metadata(&path){Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(error)=>Err(err(error)),Ok(_)=>bounded(&path,16384).map(Some)}
}
pub fn read(root:&Path)->Result<Option<Record>>{
 let Some(bytes)=bytes(root)? else{return Ok(None)};
 let record:Record=serde_json::from_slice(&bytes).map_err(|_|"INVALID_LFS_UPLOAD_RECORD")?;
 record.validate()?;Ok(Some(record))
}
pub fn ensure_ready(root:&Path)->Result<()>{
 if read(root)?.is_some_and(|record|record.unresolved()){return Err("LFS_UPLOAD_REVIEW_REQUIRED: refresh cache status and review the previous upload outcome before retrying".into());}
 Ok(())
}
fn save(root:&Path,record:&Record,before:Option<&[u8]>,lease:&OperationLease)->Result<()>{
 record.validate()?;
 if bytes(root)?.as_deref()!=before{return Err("STALE_LFS_UPLOAD_RECORD".into());}
 let layout=layout::Layout::read(root)?;lease.require_repository(&layout)?;
 let directory=layout.common;
 let mut temporary=tempfile::Builder::new().prefix("branchdesk-lfs-upload-").suffix(".next").tempfile_in(&directory).map_err(err)?;
 std::io::Write::write_all(&mut temporary,&serde_json::to_vec(record).map_err(err)?).map_err(err)?;
 temporary.as_file().sync_all().map_err(err)?;
 if bytes(root)?.as_deref()!=before{return Err("STALE_LFS_UPLOAD_RECORD".into());}
 lease.require_repository(&layout::Layout::read(root)?)?;
 if before.is_none(){temporary.persist_noclobber(directory.join(FILE)).map_err(err)?;}else{temporary.persist(directory.join(FILE)).map_err(err)?;}
 checkout::sync_dir(&directory)
}
pub struct Pending{root:PathBuf,record:Record,_lease:OperationLease}
impl Pending{
 pub fn begin(root:&Path,object:&super::protocol::Object,endpoint:&str,requires_verify:bool)->Result<Self>{
  let lease=OperationLease::acquire(root)?;ensure_ready(root)?;
  let before=bytes(root)?;
  let record=Record{version:1,id:Uuid::new_v4().to_string(),oid:object.oid.clone(),size:object.size.to_string(),endpoint:endpoint.into(),requires_verify,status:Status::Pending,observation:None};
  save(root,&record,before.as_deref(),&lease)?;
  Ok(Self{root:root.into(),record,_lease:lease})
 }
 pub fn mark(&mut self,status:Status)->Result<()>{
  let before=bytes(&self.root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
  let previous=read(&self.root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
  if previous!=self.record{return Err("STALE_LFS_UPLOAD_RECORD".into());}
  if !matches!((&self.record.status,&status),(Status::Pending,Status::Accepted|Status::NotSent)|(Status::Accepted,Status::Verified)){return Err("INVALID_LFS_UPLOAD_TRANSITION".into());}
  let mut next=self.record.clone();next.status=status;
  save(&self.root,&next,Some(&before),&self._lease)?;self.record=next;Ok(())
 }
}
pub fn acknowledge(root:&Path,id:&str)->Result<()>{
 let _lease=OperationLease::acquire(root)?;
 let before=bytes(root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
 let mut record=read(root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
 if record.id!=id||!record.unresolved(){return Err("STALE_LFS_UPLOAD_RECORD".into());}
 record.status=Status::Reviewed;save(root,&record,Some(&before),&_lease)
}

/// Hold the same common-repository lease while checking and saving an observation.
pub struct Check{root:PathBuf,record:Record,_lease:OperationLease}
impl Check{
 pub fn begin(root:&Path,expected:&Record)->Result<Self>{
  let lease=OperationLease::acquire(root)?;
  if read(root)?.as_ref()!=Some(expected){return Err("STALE_LFS_UPLOAD_RECORD".into());}
  Ok(Self{root:root.into(),record:expected.clone(),_lease:lease})
 }
 pub fn finish(mut self,present:bool)->Result<Observation>{
  let before=bytes(&self.root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
  if read(&self.root)?.as_ref()!=Some(&self.record){return Err("STALE_LFS_UPLOAD_RECORD".into());}
  let observation=Observation{present,checked_at:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"INVALID_SYSTEM_TIME")?.as_secs()};
  self.record.observation=Some(observation.clone());
  save(&self.root,&self.record,Some(&before),&self._lease)?;Ok(observation)
 }
}
