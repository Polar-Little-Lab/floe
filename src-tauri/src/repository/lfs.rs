//! Git LFS cache inspection and explicit-transfer foundations; no automatic filters or tool installation.
mod expiry;
pub mod protocol;
pub mod cache;
pub mod upload_state;
pub mod checkout_preview;
mod checkout_prepare;
pub(crate) mod transport;
use super::*;
#[derive(Debug,Serialize)]
#[serde(rename_all="camelCase")]
pub struct ObjectStatus {pub state:String,pub message:String}
fn status(state:&str,message:&str)->ObjectStatus{ObjectStatus{state:state.into(),message:message.into()}}
fn pointer(text:&str)->Option<(&str,u64)>{
 if text.len()>=1024||!text.ends_with('\n')||text.contains('\r'){return None;}
 let mut lines=text.strip_suffix('\n')?.split('\n');
 if !matches!(lines.next()?,"version https://git-lfs.github.com/spec/v1"|"version https://hawser.github.com/spec/v1"){return None;}
 let(mut oid,mut size,mut previous)=(None,None,"");
 for line in lines {let(key,value)=line.split_once(' ')?;if key.is_empty()||value.is_empty()||key<=previous||key=="version"||!key.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'.'||b==b'-'){return None;}previous=key;
  match key{"oid"=>{let hash=value.strip_prefix("sha256:")?;if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return None;}oid=Some(hash);},"size"=>{if value.is_empty()||!value.bytes().all(|b|b.is_ascii_digit())||(value.len()>1&&value.starts_with('0')){return None;}size=Some(value.parse().ok()?);},_=>{}}
 }
 Some((oid?,size?))
}
pub(super) fn inspect(root:&Path,text:&str)->Option<ObjectStatus>{
 let(oid,size)=pointer(text)?;
 Some(match inspect_object(root,oid,size){Ok(value)=>value,Err(_)=>status("unavailable","Standard local LFS cache could not be inspected safely.")})
}
fn inspect_object(root:&Path,oid:&str,size:u64)->Result<ObjectStatus>{
 let layout=layout::Layout::read(root)?;
 let mut path=layout.common;
 for part in ["lfs","objects",&oid[..2],&oid[2..4]]{
  path.push(part);
  match fs::symlink_metadata(&path){Ok(meta) if meta.is_dir()&&!is_link(&meta)=>{},Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(status("missing","Object is absent from the standard local LFS cache. Custom storage and remote availability were not checked.")),_=>return Err("UNSUPPORTED_LFS_CACHE".into())}
 }
 path.push(oid);
 let meta=match fs::symlink_metadata(&path){Ok(meta)=>meta,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(status("missing","Object is absent from the standard local LFS cache. Custom storage and remote availability were not checked.")),Err(error)=>return Err(err(error))};
 if !meta.is_file()||is_link(&meta){return Err("UNSUPPORTED_LFS_OBJECT".into());}
 if meta.len()!=size{return Ok(status("sizeMismatch","Cached object size does not match the pointer. Do not use it for checkout."));}
 const VERIFY_LIMIT:u64=16*1024*1024;
 if size>VERIFY_LIMIT{return Ok(status("presentUnverified","Object exists with the declared size. SHA-256 verification is limited to 16 MiB in this preview."));}
 let bytes=bounded(&path,VERIFY_LIMIT)?;
 if bytes.len() as u64!=size{return Ok(status("sizeMismatch","Cached object changed size while being inspected."));}
 if format!("{:x}",Sha256::digest(&bytes))!=oid{return Ok(status("hashMismatch","Cached object SHA-256 does not match the pointer. Do not use it for checkout."));}
 Ok(status("verified","Standard local cached bytes match the pointer size and SHA-256. This is an inspection snapshot, not checkout authorization."))
}
impl Session{
 pub fn lfs_review_root(&mut self,revision:&str)->Result<PathBuf>{self.fresh(revision)?;layout::Layout::read(&self.root)?;Ok(self.root.clone())}

 pub fn lfs_transfer_root(&mut self,revision:&str,oid:&str,size:&str)->Result<PathBuf>{
  self.fresh(revision)?;
  if size.is_empty()||!size.bytes().all(|b|b.is_ascii_digit())||(size.len()>1&&size.starts_with('0')){return Err("INVALID_LFS_OBJECT_SIZE".into());}
  let object=protocol::Object{oid:oid.into(),size:size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?};object.validate()?;
  layout::Layout::read(&self.root)?;
  Ok(self.root.clone())
 }
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct IndexedPointer{pub path:String,pub text:String}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct IndexPage{pub entries:Vec<IndexedPointer>,pub offset:usize,pub next_offset:Option<usize>,pub total_index_entries:usize,pub skipped_non_utf8:usize}
impl Session{
 pub fn lfs_index_page(&mut self,revision:&str,offset:usize)->Result<IndexPage>{
  self.fresh(revision)?;
  let repo=open(&self.root)?;let index=repo.index().map_err(err)?;let total=index.len();
  if offset>total{return Err("INVALID_LFS_INDEX_OFFSET".into());}
  let end=offset.saturating_add(1000).min(total);let odb=repo.odb().map_err(err)?;
  let mut entries=Vec::new();let mut skipped_non_utf8=0;
  for entry in index.iter().skip(offset).take(end-offset){
   if entry.flags&0x3000!=0||!matches!(entry.mode,0o100644|0o100755)||entry.id.is_zero(){continue;}
   let (size,kind)=odb.read_header(entry.id).map_err(err)?;
   if size>=1024||kind!=git2::ObjectType::Blob{continue;}
   let blob=repo.find_blob(entry.id).map_err(err)?;
   let Ok(text)=std::str::from_utf8(blob.content()) else{continue};
   if pointer(text).is_none(){continue;}
   let Ok(path)=String::from_utf8(entry.path) else{skipped_non_utf8+=1;continue};
   entries.push(IndexedPointer{path,text:text.into()});
  }
  drop(odb);drop(index);drop(repo);self.fresh(revision)?;
  Ok(IndexPage{entries,offset,next_offset:(end<total).then_some(end),total_index_entries:total,skipped_non_utf8})
 }
 pub fn lfs_object_status(&mut self,revision:&str,oid:&str,size:&str)->Result<ObjectStatus>{
  let root=self.lfs_transfer_root(revision,oid,size)?;
  let result=inspect_object(&root,oid,size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?);
  self.fresh(revision)?;
  Ok(result.unwrap_or_else(|_|status("unavailable","Standard local LFS cache could not be inspected safely.")))
 }
}
