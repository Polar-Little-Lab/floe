//! Stream verified objects into the standard cache, without modifying a checkout.
#![allow(dead_code)] // Shared primitives also support forthcoming upload/checkout flows.
use super::protocol::Object;
use super::super::{checkout,err,fs,is_link,layout,Path,PathBuf,Result};
use sha2::{Digest,Sha256};
use std::io::{Read,Write};
#[derive(Debug)]
pub enum Published{Created,AlreadyPresent}
fn directory(path:&Path)->Result<()>{let meta=fs::symlink_metadata(path).map_err(err)?;if !meta.is_dir()||is_link(&meta){return Err("UNSUPPORTED_LFS_CACHE_DIRECTORY".into());}Ok(())}
fn directory_path(root:&Path,object:&Object,create:bool)->Result<PathBuf>{
 object.validate()?;let mut path=layout::Layout::read(root)?.common;directory(&path)?;
 for part in ["lfs","objects",&object.oid[..2],&object.oid[2..4]]{
  let parent=path.clone();path.push(part);
  if create {match fs::create_dir(&path){Ok(())=>{checkout::sync_dir(&parent)?;},Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(error)=>return Err(err(error))}}
  directory(&path)?;
 }
 Ok(path)
}
fn stream<R:Read,W:Write,F:Fn()->bool>(reader:&mut R,writer:&mut W,object:&Object,cancelled:&F)->Result<()>{
 let mut remaining=object.size;let mut hash=Sha256::new();let mut buffer=[0u8;65536];
 while remaining>0{
  if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
  let length=remaining.min(buffer.len() as u64) as usize;
  let read=reader.read(&mut buffer[..length]);
  if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
  let read=read.map_err(|_|"LFS_OBJECT_READ_FAILED")?;
  if read==0{return Err("LFS_OBJECT_SIZE_MISMATCH".into());}
  writer.write_all(&buffer[..read]).map_err(|_|"LFS_OBJECT_WRITE_FAILED")?;
  hash.update(&buffer[..read]);remaining-=read as u64;
 }
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let extra=reader.read(&mut buffer[..1]);
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 if extra.map_err(|_|"LFS_OBJECT_READ_FAILED")?!=0{return Err("LFS_OBJECT_SIZE_MISMATCH".into());}
 if format!("{:x}",hash.finalize())!=object.oid{return Err("LFS_OBJECT_HASH_MISMATCH".into());}
 Ok(())
}
/// Missing objects may be fetched; existing invalid objects must never be silently replaced.
pub(crate) fn verify_if_present<F:Fn()->bool>(root:&Path,object:&Object,cancelled:&F)->Result<bool>{
 object.validate()?;
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let mut path=layout::Layout::read(root)?.common;directory(&path)?;
 for part in ["lfs","objects",&object.oid[..2],&object.oid[2..4]]{
  path.push(part);
  match fs::symlink_metadata(&path){
   Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),
   Err(error)=>return Err(err(error)),
   Ok(meta) if meta.is_dir()&&!is_link(&meta)=>{},
   Ok(_)=>return Err("UNSUPPORTED_LFS_CACHE_DIRECTORY".into()),
  }
 }
 path.push(&object.oid);
 match fs::symlink_metadata(&path){
  Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),
  Err(error)=>return Err(err(error)),
  Ok(_)=>{},
 }
 verify(root,object,cancelled)?;Ok(true)
}
pub fn verify<F:Fn()->bool>(root:&Path,object:&Object,cancelled:&F)->Result<()>{
 let path=directory_path(root,object,false)?.join(&object.oid);
 let meta=fs::symlink_metadata(&path).map_err(err)?;
 if !meta.is_file()||is_link(&meta)||meta.len()!=object.size{return Err("INVALID_LFS_CACHED_OBJECT".into());}
 let mut file=fs::File::open(&path).map_err(err)?;
 if file.metadata().map_err(err)?.len()!=object.size{return Err("LFS_OBJECT_CHANGED".into());}
 stream(&mut file,&mut std::io::sink(),object,cancelled)
}
/// Caller must own the app/repository operation guards and authorize the source.
/// The reader must enforce its own network timeout; it never receives credentials here.
pub fn publish<R:Read,F:Fn()->bool>(root:&Path,object:&Object,reader:&mut R,cancelled:&F)->Result<Published>{
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let directory=directory_path(root,object,true)?;let destination=directory.join(&object.oid);
 match fs::symlink_metadata(&destination){
  Ok(_)=>{verify(root,object,cancelled)?;return Ok(Published::AlreadyPresent);},
  Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
  Err(error)=>return Err(err(error)),
 }
 let mut temporary=tempfile::Builder::new().prefix(".branchdesk-lfs-").tempfile_in(&directory).map_err(err)?;
 stream(reader,&mut temporary,object,cancelled)?;
 temporary.as_file().sync_all().map_err(|_|"LFS_OBJECT_SYNC_FAILED")?;
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 if directory_path(root,object,false)?!=directory{return Err("LFS_CACHE_CHANGED".into());}
 match temporary.persist_noclobber(&destination){
  Ok(_)=>{checkout::sync_dir(&directory).map_err(|_|"LFS_OBJECT_PUBLISHED_DURABILITY_UNKNOWN")?;Ok(Published::Created)},
  Err(error) if error.error.kind()==std::io::ErrorKind::AlreadyExists=>{verify(root,object,cancelled)?;Ok(Published::AlreadyPresent)},
  Err(_)=>Err("LFS_OBJECT_PUBLICATION_FAILED".into()),
 }
}

/// Copy only bytes matching the declared immutable object identity.
pub(super) fn copy_verified<W:Write,F:Fn()->bool>(root:&Path,object:&Object,writer:&mut W,cancelled:&F)->Result<()>{
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let path=directory_path(root,object,false)?.join(&object.oid);
 let meta=fs::symlink_metadata(&path).map_err(err)?;
 if !meta.is_file()||is_link(&meta)||meta.len()!=object.size{return Err("INVALID_LFS_CACHED_OBJECT".into());}
 let mut source=fs::File::open(&path).map_err(err)?;
 if source.metadata().map_err(err)?.len()!=object.size{return Err("LFS_OBJECT_CHANGED".into());}
 stream(&mut source,writer,object,cancelled)
}
/// Freeze and verify the source before any remote write. The anonymous temporary
/// file is removed on close and is never published into the object cache.
pub(crate) fn upload_source<F:Fn()->bool>(root:&Path,object:&Object,cancelled:&F)->Result<fs::File>{
 let mut frozen=tempfile::tempfile().map_err(|_|"LFS_UPLOAD_SNAPSHOT_FAILED")?;
 copy_verified(root,object,&mut frozen,cancelled)?;
 std::io::Seek::rewind(&mut frozen).map_err(|_|"LFS_UPLOAD_SNAPSHOT_FAILED")?;
 Ok(frozen)
}
