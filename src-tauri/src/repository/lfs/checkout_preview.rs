//! Read-only checkout inputs. A preview never grants permission to replace a file.
use super::super::*;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Preview{
 pub path:String,
 pub index_oid:String,
 pub oid:String,
 pub size:String,
 pub executable:bool,
 pub worktree_state:String,
 pub cache:super::ObjectStatus,
}
pub(super) fn read(session:&mut Session,revision:&str,path:&str,expected_oid:&str,expected_size:&str)->Result<Preview>{
 session.fresh(revision)?;
 if path.len()>4096||path.chars().any(char::is_control){return Err("INVALID_LFS_CHECKOUT_PATH".into());}
 let destination=rel(&session.root,Path::new(path))?;
 let repo=open(&session.root)?;let index=repo.index().map_err(err)?;
 if (1..=3).any(|stage|index.get_path(Path::new(path),stage).is_some()){return Err("LFS_CHECKOUT_CONFLICTED_PATH".into());}
 let entry=index.get_path(Path::new(path),0).ok_or("LFS_CHECKOUT_PATH_NOT_INDEXED")?;
 if !matches!(entry.mode,0o100644|0o100755)||entry.id.is_zero(){return Err("LFS_CHECKOUT_UNSUPPORTED_MODE".into());}
 let (length,kind)=repo.odb().map_err(err)?.read_header(entry.id).map_err(err)?;
 if length>=1024||kind!=git2::ObjectType::Blob{return Err("LFS_CHECKOUT_NOT_POINTER".into());}
 let blob=repo.find_blob(entry.id).map_err(err)?;
 let text=std::str::from_utf8(blob.content()).map_err(|_|"LFS_CHECKOUT_NOT_POINTER")?;
 let (oid,size)=super::pointer(text).ok_or("LFS_CHECKOUT_NOT_POINTER")?;
 if oid!=expected_oid||size.to_string()!=expected_size{return Err("STALE_LFS_CHECKOUT_OBJECT".into());}
 let worktree_state=match fs::symlink_metadata(&destination){
  Err(error) if error.kind()==std::io::ErrorKind::NotFound=>"missing",
  Err(error)=>return Err(err(error)),
  Ok(meta) if !meta.is_file()||is_link(&meta)=>"unsupported",
  Ok(meta) if meta.len()!=blob.size() as u64=>"different",
  Ok(_)=>{if bounded(&destination,1023)?==blob.content(){"pointer"}else{"different"}},
 };
 let cache=super::inspect(&session.root,text).ok_or("LFS_CHECKOUT_NOT_POINTER")?;
 let result=Preview{path:path.into(),index_oid:entry.id.to_string(),oid:oid.into(),size:size.to_string(),executable:entry.mode==0o100755,worktree_state:worktree_state.into(),cache};
 drop(blob);drop(index);drop(repo);session.fresh(revision)?;
 Ok(result)
}
impl Session{
 pub fn lfs_checkout_preview(&mut self,revision:&str,path:&str,oid:&str,size:&str)->Result<Preview>{read(self,revision,path,oid,size)}
}
