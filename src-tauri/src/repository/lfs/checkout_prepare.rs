//! Verified staging inputs for checkout. No worktree replacement is exposed yet.
#![allow(dead_code)] // Transactional publication/recovery must precede command integration.
use super::super::*;
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest{
 pub version:u32,
 pub path:String,
 pub index_oid:String,
 pub oid:String,
 pub size:String,
 pub before_present:bool,
 pub before_digest:Option<String>,
 pub before_mode:Option<u32>,
 pub after_mode:u32,
}
pub(super) struct Prepared{
 pub directory:tempfile::TempDir,
 pub manifest:Manifest,
 _lease:lease::OperationLease,
}
fn mode(path:&Path)->Result<u32>{
 #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;Ok(fs::metadata(path).map_err(err)?.permissions().mode()&0o777)}
 #[cfg(not(unix))]{let _=path;Ok(0o644)}
}
pub(super) fn prepare<F:Fn()->bool>(session:&mut Session,revision:&str,path:&str,oid:&str,size:&str,cancelled:&F)->Result<Prepared>{
 let lease=lease::OperationLease::acquire(&session.root)?;
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let before=super::checkout_preview::read(session,revision,path,oid,size)?;
 if !matches!(before.worktree_state.as_str(),"pointer"|"missing"){return Err("LFS_CHECKOUT_WORKTREE_CHANGED".into());}
 let destination=rel(&session.root,Path::new(path))?;
 let before_present=before.worktree_state=="pointer";
 let before_bytes=if before_present{Some(bounded(&destination,1023)?)}else{None};
 let before_mode=if before_present{Some(mode(&destination)?)}else{None};
 // Freeze the exact indexed pointer, not arbitrary bytes that changed after preview.
 let repo=open(&session.root)?;let blob=repo.find_blob(Oid::from_str(&before.index_oid).map_err(err)?).map_err(err)?;
 if before_bytes.as_deref().is_some_and(|bytes|bytes!=blob.content()){return Err("LFS_CHECKOUT_WORKTREE_CHANGED".into());}
 drop(blob);drop(repo);
 let layout=layout::Layout::read(&session.root)?;lease.require_repository(&layout)?;
 let directory=tempfile::Builder::new().prefix("branchdesk-lfs-prepare-").tempdir_in(&layout.private).map_err(err)?;
 let manifest=Manifest{version:1,path:path.into(),index_oid:before.index_oid.clone(),oid:oid.into(),size:size.into(),before_present,before_digest:before_bytes.as_ref().map(|bytes|checkout::digest(bytes)),before_mode,after_mode:if before.executable{0o755}else{0o644}};
 if let Some(bytes)=before_bytes.as_ref(){checkout::save(&directory.path().join("before"),bytes)?;}
 let object=super::protocol::Object{oid:oid.into(),size:size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?};
 let staged_path=directory.path().join("after");
 let mut staged=fs::OpenOptions::new().write(true).create_new(true).open(&staged_path).map_err(err)?;
 super::cache::copy_verified(&session.root,&object,&mut staged,cancelled)?;
 #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;staged.set_permissions(fs::Permissions::from_mode(manifest.after_mode)).map_err(err)?;}
 staged.sync_all().map_err(err)?;drop(staged);
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 // Prepare never authorizes publication: re-check all captured inputs, and a
 // future publisher must perform these checks again while owning Git locks.
 let latest=super::checkout_preview::read(session,revision,path,oid,size)?;
 if latest.index_oid!=manifest.index_oid||latest.executable!=before.executable||latest.worktree_state!=before.worktree_state{return Err("STALE_LFS_CHECKOUT_PREPARATION".into());}
 if before_present&&(bounded(&destination,1023)?.as_slice()!=before_bytes.as_deref().ok_or("MISSING_LFS_POINTER_BACKUP")?||Some(mode(&destination)?)!=before_mode){return Err("LFS_CHECKOUT_WORKTREE_CHANGED".into());}
 checkout::save(&directory.path().join("manifest.json"),&serde_json::to_vec(&manifest).map_err(err)?)?;
 checkout::sync_dir(directory.path())?;checkout::sync_dir(&layout.private)?;
 Ok(Prepared{directory,manifest,_lease:lease})
}
