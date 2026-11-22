//! CI observations for a specific PR head, independent of merge eligibility.
use super::*;
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase")]
pub enum Kind { CheckRuns, CommitStatuses }
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Entry {pub id:u64,pub name:String,pub status:String,pub conclusion:Option<String>,pub description:Option<String>,pub updated_at:Option<String>,pub details_url:Option<String>}
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Page {pub entries:Vec<Entry>,pub total_count:u64,pub next_page:Option<u32>,pub limit_reached:bool,pub combined_state:Option<String>}
fn string(v:&Value,limit:usize)->Result<String,String>{v.as_str().filter(|s|!s.is_empty()&&text(s,limit,true)).map(str::to_owned).ok_or("INVALID_PROVIDER_CHECKS".into())}
fn optional(v:&Value,limit:usize)->Result<Option<String>,String>{if v.is_null(){Ok(None)}else{v.as_str().filter(|s|text(s,limit,true)).map(|s|Some(s.to_owned())).ok_or("INVALID_PROVIDER_CHECKS".into())}}
fn count(v:&Value)->Result<u64,String>{v.as_u64().filter(|n|*n<=9_007_199_254_740_991).ok_or("INVALID_PROVIDER_CHECKS".into())}
fn details_url(v:&Value)->Option<String>{
 let raw=v.as_str().filter(|s|s.len()<=4096&&!s.chars().any(char::is_control))?;
 let url=reqwest::Url::parse(raw).ok()?;
 (url.scheme()=="https"&&url.host_str().is_some()&&url.username().is_empty()&&url.password().is_none()).then(||url.into())
}
fn parse(v:Value,kind:Kind,head:&str,page:u32)->Result<Page,String>{
 let total_count=count(&v["total_count"])?;
 let (items,combined_state)=match kind {
  Kind::CheckRuns=>(v["check_runs"].as_array().ok_or("INVALID_PROVIDER_CHECKS")?,None),
  Kind::CommitStatuses=>{
   if v["sha"].as_str()!=Some(head){return Err("INVALID_PROVIDER_CHECKS_SHA".into());}
   let state=string(&v["state"],64)?;
   (v["statuses"].as_array().ok_or("INVALID_PROVIDER_CHECKS")?,Some(state))
  }
 };
 if items.len()>100 || items.len() as u64>total_count {return Err("INVALID_PROVIDER_CHECKS".into());}
 let mut entries=Vec::new();let mut ids=std::collections::HashSet::new();
 for item in items {
  let id=count(&item["id"])?;
  if id==0||!ids.insert(id){return Err("INVALID_PROVIDER_CHECKS".into());}
  let (name,status,conclusion,description,updated_at,url)=match kind {
   Kind::CheckRuns=>{
    if item["head_sha"].as_str()!=Some(head){return Err("INVALID_PROVIDER_CHECKS_SHA".into());}
    (string(&item["name"],1024)?,string(&item["status"],64)?,optional(&item["conclusion"],64)?,optional(&item["output"]["title"],4096)?,optional(&item["completed_at"],64)?.or(optional(&item["started_at"],64)?),details_url(&item["details_url"]))
   },
   Kind::CommitStatuses=>(string(&item["context"],1024)?,string(&item["state"],64)?,None,optional(&item["description"],4096)?,optional(&item["updated_at"],64)?,details_url(&item["target_url"]))
  };
  entries.push(Entry{id,name,status,conclusion,description,updated_at,details_url:url});
 }
 let more=total_count>u64::from(page)*100;
 if more && entries.len()!=100 {return Err("PROVIDER_CHECKS_CHANGED: reload checks before relying on this list".into());}
 Ok(Page {entries,total_count,next_page:if more&&page<30{Some(page+1)}else{None},limit_reached:more&&page==30,combined_state})
}
fn read_with(client:&reqwest::blocking::Client,endpoint:&str,owner:&str,repo:&str,n:u64,page:u32,head:&str,base:&str,kind:Kind)->Result<Page,String>{
 validate_files(owner,repo,n,page,head,base)?;
 let item=format!("{endpoint}/pulls/{n}");
 check_snapshot(client,&item,owner,repo,n,head,base)?;
 let resource=match kind{Kind::CheckRuns=>"check-runs",Kind::CommitStatuses=>"status"};
 let filter=if matches!(kind,Kind::CheckRuns){"&filter=latest"}else{""};
 let v=send(client,&format!("{endpoint}/commits/{head}/{resource}?per_page=100&page={page}{filter}"),reqwest::Method::GET,None,false)?;
 let parsed=parse(v,kind,head,page)?;
 check_snapshot(client,&item,owner,repo,n,head,base)?;
 Ok(parsed)
}
pub fn read(owner:&str,repo:&str,n:u64,page:u32,head:&str,base:&str,kind:Kind,credential:&crate::credential_vault::Credential)->Result<Page,String>{
 validate_files(owner,repo,n,page,head,base)?;
 read_with(&client(credential)?,&endpoint(owner,repo)?,owner,repo,n,page,head,base,kind)
}

#[cfg(test)]
mod tests{
 use super::*;use std::io::Write;
 fn run(id:u64)->Value{json!({"id":id,"name":"CI","head_sha":"a".repeat(40),"status":"completed","conclusion":"success","output":{"title":"<script>literal</script>"},"completed_at":"2026-11-21T00:00:00Z","details_url":"https://ci.example/build/1"})}
 fn status(id:u64)->Value{json!({"id":id,"context":"ci/test","state":"pending","description":null,"updated_at":null,"target_url":"https://ci.example/build/2"})}
 fn runs(items:Vec<Value>,total:u64)->Value{json!({"total_count":total,"check_runs":items})}
 fn statuses(items:Vec<Value>,total:u64)->Value{json!({"sha":"a".repeat(40),"state":"pending","total_count":total,"statuses":items})}
 #[test]
 fn checks_preserve_states_and_copy_only_safe_https_urls(){
  let page=parse(runs(vec![run(1)],1),Kind::CheckRuns,&"a".repeat(40),1).unwrap();assert_eq!(page.entries[0].conclusion.as_deref(),Some("success"));assert_eq!(page.entries[0].description.as_deref(),Some("<script>literal</script>"));assert_eq!(page.entries[0].details_url.as_deref(),Some("https://ci.example/build/1"));assert_eq!(page.combined_state,None);
  let page=parse(statuses(vec![status(2)],1),Kind::CommitStatuses,&"a".repeat(40),1).unwrap();assert_eq!(page.combined_state.as_deref(),Some("pending"));assert_eq!(page.entries[0].status,"pending");
  for url in ["javascript:alert(1)","file:///etc/passwd","http://ci.example","https://user:secret@ci.example","https://ci.example/\nsecret"]{assert!(details_url(&json!(url)).is_none());}
  let mut future=run(1);future["status"]=json!("future_state");future["conclusion"]=Value::Null;assert_eq!(parse(runs(vec![future],1),Kind::CheckRuns,&"a".repeat(40),1).unwrap().entries[0].status,"future_state");
 }
 #[test]
 fn checks_reject_wrong_commits_invalid_counts_and_duplicate_ids(){
  assert!(parse(runs(vec![run(1)],1),Kind::CheckRuns,&"b".repeat(40),1).is_err());
  assert!(parse(statuses(vec![status(1)],1),Kind::CommitStatuses,&"b".repeat(40),1).is_err());
  for value in [runs(vec![run(1)],0),runs(vec![run(1),run(1)],2),runs(vec![run(0)],1),json!({}),runs((1..=101).map(run).collect(),101)]{assert!(parse(value,Kind::CheckRuns,&"a".repeat(40),1).is_err());}
  assert!(parse(runs(vec![run(1)],101),Kind::CheckRuns,&"a".repeat(40),1).unwrap_err().starts_with("PROVIDER_CHECKS_CHANGED"));
 }
 #[test]
 fn empty_results_and_pagination_never_invent_passed_checks(){
  let empty=parse(runs(vec![],0),Kind::CheckRuns,&"a".repeat(40),1).unwrap();assert!(empty.entries.is_empty());assert!(empty.combined_state.is_none());
  let full=runs((1..=100).map(run).collect(),3001);let first=parse(full.clone(),Kind::CheckRuns,&"a".repeat(40),1).unwrap();assert_eq!(first.next_page,Some(2));
  let last=parse(full,Kind::CheckRuns,&"a".repeat(40),30).unwrap();assert!(last.limit_reached);assert_eq!(last.next_page,None);
 }
 fn detail()->Value{json!({"number":7,"title":"PR","body":null,"state":"open","head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40)}})}
 fn exchange(kind:Kind,responses:Vec<(u16,Value)>)->(Result<Page,String>,Vec<String>){
  let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();let endpoint=format!("http://{}/repos/o/r",listener.local_addr().unwrap());
  let server=std::thread::spawn(move||{let mut requests=Vec::new();for(status,value)in responses{let start=std::time::Instant::now();let mut stream=loop{match listener.accept(){Ok((s,_))=>break s,Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>{assert!(start.elapsed()<Duration::from_secs(5));std::thread::sleep(Duration::from_millis(5));},Err(e)=>panic!("{e}")}};stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();let mut bytes=Vec::new();let mut buf=[0;4096];while !bytes.windows(4).any(|w|w==b"\r\n\r\n"){let n=stream.read(&mut buf).unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);assert!(bytes.len()<16384);}requests.push(String::from_utf8(bytes).unwrap());let body=value.to_string();write!(stream,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();}requests});
  let client=reqwest::blocking::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(2)).build().unwrap();let result=read_with(&client,&endpoint,"o","r",7,1,&"a".repeat(40),&"b".repeat(40),kind);(result,server.join().unwrap())
 }
 #[test]
 fn fixed_sha_routes_checked_before_and_after_read(){
  for (kind,value,path) in [(Kind::CheckRuns,runs(vec![run(1)],1),"check-runs?per_page=100&page=1&filter=latest"),(Kind::CommitStatuses,statuses(vec![status(1)],1),"status?per_page=100&page=1")]{
   let(result,requests)=exchange(kind,vec![(200,detail()),(200,value.clone()),(200,detail())]);assert_eq!(result.unwrap().entries.len(),1);assert_eq!(requests.len(),3);assert!(requests[1].starts_with(&format!("GET /repos/o/r/commits/{}/{path} HTTP/1.1","a".repeat(40))));
   let mut changed=detail();changed["head"]["sha"]=json!("c".repeat(40));assert!(exchange(kind,vec![(200,detail()),(200,value),(200,changed)]).0.unwrap_err().starts_with("PROVIDER_SNAPSHOT_CHANGED"));
  }
 }
 #[test]
 fn permissions_and_server_failures_are_not_success_or_leaked_bodies(){
  for(status,expected)in[(403,"PROVIDER_ACCESS_OR_RATE_LIMIT"),(404,"PROVIDER_NOT_FOUND_OR_PRIVATE"),(302,"PROVIDER_READ_FAILED"),(500,"PROVIDER_READ_FAILED")]{let(result,requests)=exchange(Kind::CheckRuns,vec![(200,detail()),(status,json!({"message":"SECRET"}))]);assert_eq!(result.unwrap_err(),expected);assert_eq!(requests.len(),2);}
 }
}
