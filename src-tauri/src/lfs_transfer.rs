//! Cooperative cancellation scoped to one renderer request, never a subsequent transfer.
use std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}};

struct Active { id:String, cancelled:Arc<AtomicBool> }
#[derive(Clone,Default)]
pub(crate) struct Transfers(Arc<Mutex<Option<Active>>>);
pub(crate) struct Transfer { registry:Transfers, id:String, cancelled:Arc<AtomicBool> }
impl Transfers {
 pub(crate) fn begin(&self,id:String)->Result<Transfer,String>{
  if !(16..=64).contains(&id.len())||!id.bytes().all(|byte|byte.is_ascii_alphanumeric()||byte==b'-'){
   return Err("INVALID_LFS_REQUEST_ID".into());
  }
  let mut active=self.0.lock().map_err(|_|"LFS_TRANSFER_LOCK_ERROR")?;
  if active.is_some(){return Err("LFS_TRANSFER_BUSY".into());}
  let cancelled=Arc::new(AtomicBool::new(false));
  *active=Some(Active{id:id.clone(),cancelled:cancelled.clone()});
  Ok(Transfer{registry:self.clone(),id,cancelled})
 }
 pub(crate) fn cancel(&self,id:&str)->Result<bool,String>{
  let active=self.0.lock().map_err(|_|"LFS_TRANSFER_LOCK_ERROR")?;
  if let Some(active)=active.as_ref().filter(|active|active.id==id){
   active.cancelled.store(true,Ordering::SeqCst);return Ok(true);
  }
  Ok(false)
 }
}
impl Transfer {
 pub(crate) fn flag(&self)->Arc<AtomicBool>{self.cancelled.clone()}
 pub(crate) fn cancelled(&self)->bool{self.cancelled.load(Ordering::SeqCst)}
 pub(crate) fn check(&self)->Result<(),String>{
  if self.cancelled(){Err("LFS_TRANSFER_CANCELLED".into())}else{Ok(())}
 }
}
impl Drop for Transfer {
 fn drop(&mut self){
  if let Ok(mut active)=self.registry.0.lock(){
   if active.as_ref().is_some_and(|active|active.id==self.id&&Arc::ptr_eq(&active.cancelled,&self.cancelled)){*active=None;}
  }
 }
}
