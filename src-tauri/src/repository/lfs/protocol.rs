//! LFS basic-transfer wire validation. Parsing never authorizes or sends a request.
#![allow(dead_code)] // Transfer orchestration will consume this boundary.
use serde::Deserialize;
use reqwest::header::{HeaderMap,HeaderName,HeaderValue};
use std::collections::BTreeMap;
type Result<T>=std::result::Result<T,String>;
#[derive(Clone,Copy)]
pub enum Operation{Download,Upload}
pub struct Object{pub oid:String,pub size:u64}
impl Object{
 pub fn validate(&self)->Result<()>{if self.oid.len()!=64||!self.oid.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return Err("INVALID_LFS_OBJECT_ID".into());}Ok(())}
}
pub fn request(operation:Operation,object:&Object)->Result<Vec<u8>>{
 object.validate()?;
 serde_json::to_vec(&serde_json::json!({"operation":match operation{Operation::Download=>"download",Operation::Upload=>"upload"},"transfers":["basic"],"hash_algo":"sha256","objects":[{"oid":object.oid,"size":object.size}]})).map_err(|_|"INVALID_LFS_REQUEST".into())
}
// No Debug/Serialize: URLs and action headers may contain temporary credentials.
pub struct Action{pub url:tauri::Url,pub headers:HeaderMap,deadline:Option<std::time::Instant>}
impl Action{
 pub fn ensure_current(&self)->Result<()>{if self.deadline.is_some_and(|deadline|std::time::Instant::now()>=deadline){return Err("LFS_ACTION_EXPIRED".into());}Ok(())}
}
pub struct Plan{pub transfer:Option<Action>,pub verify:Option<Action>,pub already_present:bool,pub authenticated:bool}
#[derive(Deserialize)]
struct Response{transfer:Option<String>,hash_algo:Option<String>,objects:Vec<ResponseObject>}
#[derive(Deserialize)]
struct ResponseObject{oid:String,size:u64,#[serde(default)]authenticated:bool,actions:Option<BTreeMap<String,WireAction>>,error:Option<ObjectError>}
#[derive(Deserialize)]
struct ObjectError{code:u16}
#[derive(Deserialize)]
struct WireAction{href:String,#[serde(default)]header:BTreeMap<String,String>,expires_in:Option<i32>,expires_at:Option<String>}
fn action(wire:WireAction)->Result<Action>{
 if wire.href.len()>8192||wire.href.chars().any(|c|c.is_control()||c.is_whitespace())||wire.href.contains(['\\','#']){return Err("INVALID_LFS_ACTION_URL".into());}
 // Signed object URLs may have a query. Validate authority/path with the existing
 // HTTPS boundary, then retain the query natively without logging it.
 super::super::remote_endpoint::https(wire.href.split('?').next().ok_or("INVALID_LFS_ACTION_URL")?)?;
 let url=tauri::Url::parse(&wire.href).map_err(|_|"INVALID_LFS_ACTION_URL")?;
 if wire.header.len()>32{return Err("LFS_ACTION_HEADER_LIMIT".into());}
 let mut headers=HeaderMap::new();let mut bytes=0usize;
 for(name,value)in wire.header{
  bytes=bytes.saturating_add(name.len()).saturating_add(value.len());
  if bytes>32768{return Err("LFS_ACTION_HEADER_LIMIT".into());}
  let name=HeaderName::from_bytes(name.as_bytes()).map_err(|_|"INVALID_LFS_ACTION_HEADER")?;
  if matches!(name.as_str(),"host"|"connection"|"content-length"|"transfer-encoding"|"proxy-authorization"|"proxy-authenticate"|"cookie"|"set-cookie"|"upgrade"|"te"|"trailer")||headers.contains_key(&name){return Err("UNSAFE_LFS_ACTION_HEADER".into());}
  let mut value=HeaderValue::from_str(&value).map_err(|_|"INVALID_LFS_ACTION_HEADER")?;value.set_sensitive(true);headers.insert(name,value);
 }
 let deadline=super::expiry::deadline(wire.expires_in,wire.expires_at.as_deref())?;
 Ok(Action{url,headers,deadline})
}
pub fn response(bytes:&[u8],operation:Operation,expected:&Object)->Result<Plan>{
 expected.validate()?;if bytes.len()>1024*1024{return Err("LFS_BATCH_RESPONSE_LIMIT".into());}
 let response:Response=serde_json::from_slice(bytes).map_err(|_|"INVALID_LFS_BATCH_RESPONSE")?;
 if response.transfer.as_deref().unwrap_or("basic")!="basic"||response.hash_algo.as_deref().unwrap_or("sha256")!="sha256"{return Err("UNSUPPORTED_LFS_TRANSFER".into());}
 if response.objects.len()!=1{return Err("LFS_BATCH_OBJECT_MISMATCH".into());}
 let object=response.objects.into_iter().next().ok_or("LFS_BATCH_OBJECT_MISMATCH")?;
 if object.oid!=expected.oid||object.size!=expected.size{return Err("LFS_BATCH_OBJECT_MISMATCH".into());}
 if let Some(error)=object.error{return Err(format!("LFS_OBJECT_ERROR_{}",error.code));}
 let already_present=matches!(operation,Operation::Upload)&&object.actions.is_none();
 let mut actions=object.actions.unwrap_or_default();
 let key=match operation{Operation::Download=>"download",Operation::Upload=>"upload"};
 let transfer=actions.remove(key).map(action).transpose()?;
 let verify=actions.remove("verify").map(action).transpose()?;
 if !actions.is_empty()||(!already_present&&transfer.is_none())||(matches!(operation,Operation::Download)&&verify.is_some()){return Err("INVALID_LFS_BATCH_ACTIONS".into());}
 Ok(Plan{transfer,verify,already_present,authenticated:object.authenticated})
}
