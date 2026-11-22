//! Native transport primitives. Only a command owning explicit access grants may call these.
#![allow(dead_code)] // Download is connected; upload/verify integration remains outstanding.
use super::{cache,protocol::{self,Action,Object,Operation,Plan}};
use crate::network_worker::auth::TransportCredential;
use reqwest::{blocking::{Client,RequestBuilder,Response},header};
use std::{io::Read,path::Path,time::Duration};
type Result<T>=std::result::Result<T,String>;
fn client(timeout:u64)->Result<Client>{
 Client::builder().https_only(true).no_proxy().referer(false)
  .redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never())
  .connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(timeout))
  .build().map_err(|_|"LFS_CLIENT_FAILED".into())
}
fn authenticated(request:RequestBuilder,endpoint:&str,credential:Option<&TransportCredential>)->Result<RequestBuilder>{
 match credential{Some(bound)=>Ok(request.header(header::AUTHORIZATION,bound.credential(endpoint)?.authorization()?)),None=>Ok(request)}
}
fn successful(response:Response)->Result<Response>{
 let code=response.status().as_u16();
 if code!=200{return Err(format!("LFS_HTTP_{code}"));}
 if response.headers().get(header::CONTENT_ENCODING).is_some_and(|value|value.as_bytes()!=b"identity"){return Err("LFS_RESPONSE_ENCODING_UNSUPPORTED".into());}
 Ok(response)
}
/// approved_endpoint is the exact LFS base URL confirmed by the native access UI.
/// Its credential is used only for the derived batch path, never an object action.
pub(crate) fn batch(approved_endpoint:&str,object:&Object,operation:Operation,credential:Option<&TransportCredential>)->Result<Plan>{
 let mut url=super::super::remote_endpoint::https(approved_endpoint)?;
 let path=format!("{}/objects/batch",url.path().trim_end_matches('/'));url.set_path(&path);
 let body=protocol::request(operation,object)?;
 let request=client(30)?.post(url).header(header::ACCEPT,"application/vnd.git-lfs+json").header(header::CONTENT_TYPE,"application/vnd.git-lfs+json").header(header::ACCEPT_ENCODING,"identity").body(body);
 let response=successful(authenticated(request,approved_endpoint,credential)?.send().map_err(|_|"LFS_BATCH_REQUEST_FAILED")?)?;
 const LIMIT:u64=1024*1024;
 if response.content_length().is_some_and(|size|size>LIMIT){return Err("LFS_BATCH_RESPONSE_LIMIT".into());}
 let mut bytes=Vec::new();response.take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"LFS_BATCH_READ_FAILED")?;
 protocol::response(&bytes,operation,object)
}
/// approved_url must be the full exact action URL separately confirmed by the user.
/// Caller holds repository operation ownership and must recheck its revision before entry.
pub(crate) fn download<F:Fn()->bool>(root:&Path,object:&Object,action:&Action,approved_url:&str,credential:Option<&TransportCredential>,cancelled:&F)->Result<cache::Published>{
 object.validate()?;action.ensure_current()?;
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 if approved_url!=action.url.as_str(){return Err("LFS_ACTION_APPROVAL_MISMATCH".into());}
 if credential.is_some()&&action.headers.contains_key(header::AUTHORIZATION){return Err("LFS_ACTION_AUTH_CONFLICT".into());}
 let request=client(120)?.get(action.url.clone()).headers(action.headers.clone()).header(header::ACCEPT_ENCODING,"identity");
 // Signed-query URLs normally carry server-provided authentication. The existing
 // credential binding rejects a query URL rather than broadening its scope.
 let request=authenticated(request,approved_url,credential)?;
 action.ensure_current()?;
 let response=request.send();
 if cancelled(){return Err("LFS_TRANSFER_CANCELLED".into());}
 let mut response=successful(response.map_err(|_|"LFS_DOWNLOAD_REQUEST_FAILED")?)?;
 if response.content_length().is_some_and(|size|size!=object.size){return Err("LFS_OBJECT_SIZE_MISMATCH".into());}
 cache::publish(root,object,&mut response,cancelled)
}

struct UploadReader{
 file:std::fs::File,
 cancelled:std::sync::Arc<std::sync::atomic::AtomicBool>,
 sent:std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl Read for UploadReader{
 fn read(&mut self,buffer:&mut [u8])->std::io::Result<usize>{
  if self.cancelled.load(std::sync::atomic::Ordering::SeqCst){return Err(std::io::Error::other("LFS_TRANSFER_CANCELLED"));}
  let count=self.file.read(buffer)?;
  self.sent.fetch_add(count as u64,std::sync::atomic::Ordering::SeqCst);
  Ok(count)
 }
}
/// No automatic retries. A transport failure after send begins has unknown remote outcome.
pub(crate) fn upload(object:&Object,source:std::fs::File,action:&Action,approved_url:&str,credential:Option<&TransportCredential>,cancelled:std::sync::Arc<std::sync::atomic::AtomicBool>)->Result<()>{
 object.validate()?;action.ensure_current()?;
 if cancelled.load(std::sync::atomic::Ordering::SeqCst){return Err("LFS_TRANSFER_CANCELLED".into());}
 if approved_url!=action.url.as_str(){return Err("LFS_ACTION_APPROVAL_MISMATCH".into());}
 if credential.is_some()&&action.headers.contains_key(header::AUTHORIZATION){return Err("LFS_ACTION_AUTH_CONFLICT".into());}
 if source.metadata().map_err(|_|"LFS_UPLOAD_SNAPSHOT_FAILED")?.len()!=object.size{return Err("LFS_OBJECT_SIZE_MISMATCH".into());}
 let sent=std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
 let body=reqwest::blocking::Body::sized(UploadReader{file:source,cancelled:cancelled.clone(),sent:sent.clone()},object.size);
 let mut request=client(120)?.put(action.url.clone()).headers(action.headers.clone()).header(header::ACCEPT_ENCODING,"identity");
 if !action.headers.contains_key(header::CONTENT_TYPE){request=request.header(header::CONTENT_TYPE,"application/octet-stream");}
 let request=authenticated(request.body(body),approved_url,credential)?;
 action.ensure_current()?;
 if cancelled.load(std::sync::atomic::Ordering::SeqCst){return Err("LFS_TRANSFER_CANCELLED".into());}
 let response=request.send().map_err(|_|"LFS_UPLOAD_OUTCOME_UNKNOWN: request interrupted; inspect the remote before retrying")?;
 if !response.status().is_success(){return Err(format!("LFS_UPLOAD_HTTP_{}: remote state may have changed; no automatic retry",response.status().as_u16()));}
 if sent.load(std::sync::atomic::Ordering::SeqCst)!=object.size{return Err("LFS_UPLOAD_OUTCOME_UNKNOWN: server responded before the complete object was read".into());}
 Ok(())
}
pub(crate) fn verify_upload(object:&Object,action:&Action,approved_url:&str,credential:Option<&TransportCredential>)->Result<()>{
 object.validate()?;action.ensure_current()?;
 if approved_url!=action.url.as_str(){return Err("LFS_ACTION_APPROVAL_MISMATCH".into());}
 if credential.is_some()&&action.headers.contains_key(header::AUTHORIZATION){return Err("LFS_ACTION_AUTH_CONFLICT".into());}
 let body=serde_json::to_vec(&serde_json::json!({"oid":object.oid,"size":object.size})).map_err(|_|"INVALID_LFS_REQUEST")?;
 let request=client(30)?.post(action.url.clone()).headers(action.headers.clone()).header(header::ACCEPT,"application/vnd.git-lfs+json").header(header::CONTENT_TYPE,"application/vnd.git-lfs+json").header(header::ACCEPT_ENCODING,"identity").body(body);
 let response=authenticated(request,approved_url,credential)?.send().map_err(|_|"LFS_VERIFY_REQUEST_FAILED")?;
 successful(response)?;Ok(())
}
