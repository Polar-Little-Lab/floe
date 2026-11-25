//! Read-only review history and inline comments; credentials and URLs stay native.
use super::*;
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase")]
pub enum Kind { Reviews, Comments }
impl Kind { fn path(self)-> &'static str {match self { Self::Reviews=>"reviews",Self::Comments=>"comments" }} }
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Entry {
    pub id:u64,
    pub author:Option<String>,
    pub body:String,
    pub created_at:Option<String>,
    pub commit_sha:Option<String>,
    pub state:Option<String>,
    pub reply_to:Option<u64>,
    pub path:Option<String>,
    pub line:Option<u64>,
    pub original_line:Option<u64>,
    pub side:Option<String>,
    pub diff_hunk:Option<String>,
    pub url:String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Page {pub entries:Vec<Entry>,pub next_page:Option<u32>,pub limit_reached:bool}
fn optional_text(v:&Value,limit:usize,multiline:bool)->Result<Option<String>,String>{
    match v {Value::Null=>Ok(None),Value::String(s) if text(s,limit,multiline)=>Ok(Some(s.clone())),_=>Err("INVALID_PROVIDER_DISCUSSION".into())}
}
fn optional_number(v:&Value)->Result<Option<u64>,String>{
    if v.is_null(){Ok(None)}else{v.as_u64().filter(|n| number(*n)).map(Some).ok_or("INVALID_PROVIDER_DISCUSSION".into())}
}
fn parse(v:Value,owner:&str,repo:&str,n:u64,kind:Kind,page:u32)->Result<Page,String>{
    let items=v.as_array().filter(|v|v.len()<=100).ok_or("INVALID_PROVIDER_DISCUSSION")?;
    let mut entries=Vec::new();let mut ids=std::collections::HashSet::new();
    for item in items {
        let id=optional_number(&item["id"])?.ok_or("INVALID_PROVIDER_DISCUSSION")?;
        if !ids.insert(id){return Err("INVALID_PROVIDER_DISCUSSION".into());}
        let author=optional_text(&item["user"]["login"],256,false)?;
        let body=optional_text(&item["body"],65536,true)?.unwrap_or_default();
        let commit_sha=optional_text(&item["commit_id"],40,false)?;
        if commit_sha.as_ref().is_some_and(|s|!sha(s)){return Err("INVALID_PROVIDER_DISCUSSION".into());}
        let (state,reply_to,path,line,original_line,side,diff_hunk,created_at,fragment)=match kind {
            Kind::Reviews=>{
                let state=item["state"].as_str().filter(|s|matches!(*s,"APPROVED"|"CHANGES_REQUESTED"|"COMMENTED"|"DISMISSED"|"PENDING")).ok_or("INVALID_PROVIDER_DISCUSSION")?.to_owned();
                (Some(state),None,None,None,None,None,None,optional_text(&item["submitted_at"],64,false)?,format!("pullrequestreview-{id}"))
            },
            Kind::Comments=>{
                let path=optional_text(&item["path"],32768,true)?.filter(|s|!s.is_empty()).ok_or("INVALID_PROVIDER_DISCUSSION")?;
                let side=optional_text(&item["side"],5,false)?;
                if side.as_deref().is_some_and(|s|!matches!(s,"LEFT"|"RIGHT")){return Err("INVALID_PROVIDER_DISCUSSION".into());}
                let reply=optional_number(&item["in_reply_to_id"])?;
                if reply==Some(id){return Err("INVALID_PROVIDER_DISCUSSION".into());}
                (None,reply,Some(path),optional_number(&item["line"])?,optional_number(&item["original_line"])?,side,optional_text(&item["diff_hunk"],65536,true)?,optional_text(&item["created_at"],64,false)?,format!("discussion_r{id}"))
            }
        };
        entries.push(Entry{id,author,body,created_at,commit_sha,state,reply_to,path,line,original_line,side,diff_hunk,url:format!("{}#{fragment}",item_url(owner,repo,n,true))});
    }
    let full=entries.len()==100;
    Ok(Page{entries,next_page:if full&&page<30 {Some(page+1)}else{None},limit_reached:full&&page==30})
}
// Keep explicit target/snapshot fields aligned with the existing IPC contract.
#[allow(clippy::too_many_arguments)]
fn read_with(client:&reqwest::blocking::Client,endpoint:&str,owner:&str,repo:&str,n:u64,page:u32,head:&str,base:&str,kind:Kind)->Result<Page,String>{
    validate_files(owner,repo,n,page,head,base)?;
    let item=format!("{endpoint}/pulls/{n}");
    check_snapshot(client,&item,owner,repo,n,head,base)?;
    let order=if matches!(kind,Kind::Comments){"&sort=created&direction=asc"}else{""};
    let v=send(client,&format!("{item}/{}?per_page=100&page={page}{order}",kind.path()),reqwest::Method::GET,None,false)?;
    let parsed=parse(v,owner,repo,n,kind,page)?;
    check_snapshot(client,&item,owner,repo,n,head,base)?;
    Ok(parsed)
}
// Keep explicit target/snapshot fields aligned with the existing IPC contract.
#[allow(clippy::too_many_arguments)]
pub fn read(owner:&str,repo:&str,n:u64,page:u32,head:&str,base:&str,kind:Kind,credential:&crate::credential_vault::Credential)->Result<Page,String>{
    validate_files(owner,repo,n,page,head,base)?;
    read_with(&client(credential)?,&endpoint(owner,repo)?,owner,repo,n,page,head,base,kind)
}

/// General conversation comments use the issues endpoint for both PRs and issues.
pub fn validate_conversation(owner:&str,repo:&str,n:u64,page:u32)->Result<(),String>{
    endpoint(owner,repo)?;
    if !number(n)||!(1..=30).contains(&page){return Err("INVALID_PROVIDER_DISCUSSION_PAGE".into());}
    Ok(())
}
pub fn read_conversation(owner:&str,repo:&str,n:u64,page:u32,pull:bool,credential:&crate::credential_vault::Credential)->Result<Page,String>{
    validate_conversation(owner,repo,n,page)?;
    let root=endpoint(owner,repo)?;
    let value=send(&client(credential)?,&format!("{root}/issues/{n}/comments?per_page=100&page={page}"),reqwest::Method::GET,None,false)?;
    let items=value.as_array().filter(|items|items.len()<=100).ok_or("INVALID_PROVIDER_DISCUSSION")?;
    let mut entries=Vec::new();let mut ids=std::collections::HashSet::new();
    for item in items{
        let id=optional_number(&item["id"])?.ok_or("INVALID_PROVIDER_DISCUSSION")?;
        if !ids.insert(id){return Err("INVALID_PROVIDER_DISCUSSION".into());}
        entries.push(Entry{id,author:optional_text(&item["user"]["login"],256,false)?,body:optional_text(&item["body"],65536,true)?.unwrap_or_default(),created_at:optional_text(&item["created_at"],64,false)?,commit_sha:None,state:None,reply_to:None,path:None,line:None,original_line:None,side:None,diff_hunk:None,url:format!("{}#issuecomment-{id}",item_url(owner,repo,n,pull))});
    }
    let full=entries.len()==100;
    Ok(Page{entries,next_page:if full&&page<30{Some(page+1)}else{None},limit_reached:full&&page==30})
}

#[cfg(test)]
mod tests {
 use super::*;
 use std::io::Write;
 fn review(id:u64)->Value{json!({"id":id,"user":{"login":"reviewer"},"body":"<script>literal</script>","state":"APPROVED","commit_id":"a".repeat(40),"submitted_at":"2026-11-21T00:00:00Z","html_url":"https://evil.invalid"})}
 fn comment(id:u64)->Value{json!({"id":id,"user":null,"body":"reply","path":"src/a.ts","line":null,"original_line":7,"side":"RIGHT","in_reply_to_id":1,"diff_hunk":"@@ -1 +1 @@\n-a\n+b","commit_id":"b".repeat(40),"created_at":"2026-11-21T00:00:00Z"})}
 #[test]
 fn reviews_and_comments_preserve_context_and_rebuild_trusted_links(){
  let page=parse(json!([review(1)]),"o","r",7,Kind::Reviews,1).unwrap();
  assert_eq!(page.entries[0].body,"<script>literal</script>");assert_eq!(page.entries[0].state.as_deref(),Some("APPROVED"));assert_eq!(page.entries[0].url,"https://github.com/o/r/pull/7#pullrequestreview-1");
  let page=parse(json!([comment(2)]),"o","r",7,Kind::Comments,1).unwrap();let c=&page.entries[0];
  assert_eq!(c.reply_to,Some(1));assert_eq!(c.author,None);assert_eq!(c.line,None);assert_eq!(c.original_line,Some(7));assert_eq!(c.url,"https://github.com/o/r/pull/7#discussion_r2");
  let pending=json!({"id":4,"state":"PENDING","body":null,"user":null,"commit_id":null});
  assert_eq!(parse(json!([pending]),"o","r",7,Kind::Reviews,1).unwrap().entries[0].created_at,None);
 }
 #[test]
 fn malformed_ids_states_fields_and_duplicate_pages_are_rejected(){
  for (key,value) in [("id",json!(0)),("state",json!("SUCCESS")),("commit_id",json!("invalid")),("body",json!({})),("body",json!("x".repeat(65537)))] {
   let mut item=review(1);item[key]=value;assert!(parse(json!([item]),"o","r",7,Kind::Reviews,1).is_err());
  }
  for (key,value) in [("in_reply_to_id",json!(2)),("line",json!(0)),("side",json!("OTHER")),("path",json!("")),("diff_hunk",json!("x".repeat(65537)))] {
   let mut item=comment(2);item[key]=value;assert!(parse(json!([item]),"o","r",7,Kind::Comments,1).is_err());
  }
  assert!(parse(json!([review(1),review(1)]),"o","r",7,Kind::Reviews,1).is_err());
  assert!(serde_json::from_value::<Kind>(json!("../../tokens")).is_err());
 }
 #[test]
 fn pagination_reports_app_limit_without_claiming_complete_history(){
  let full=Value::Array((1..=100).map(review).collect());
  assert_eq!(parse(full.clone(),"o","r",7,Kind::Reviews,1).unwrap().next_page,Some(2));
  let last=parse(full,"o","r",7,Kind::Reviews,30).unwrap();assert!(last.limit_reached);assert_eq!(last.next_page,None);
  assert_eq!(parse(json!([]),"o","r",7,Kind::Reviews,1).unwrap().next_page,None);
 }
 fn exchange(kind:Kind,responses:Vec<(u16,Value)>)->(Result<Page,String>,Vec<String>){
  let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
  let endpoint=format!("http://{}/repos/o/r",listener.local_addr().unwrap());
  let server=std::thread::spawn(move||{
   let mut requests=Vec::new();
   for (status,value) in responses {
    let start=std::time::Instant::now();
    let mut stream=loop {match listener.accept(){Ok((s,_))=>break s,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{assert!(start.elapsed()<Duration::from_secs(5));std::thread::sleep(Duration::from_millis(5));},Err(e)=>panic!("{e}")}};
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();let mut bytes=Vec::new();let mut buf=[0;4096];
    while !bytes.windows(4).any(|w|w==b"\r\n\r\n"){let n=stream.read(&mut buf).unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);assert!(bytes.len()<16384);}
    requests.push(String::from_utf8(bytes).unwrap());let body=value.to_string();
    write!(stream,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
   }
   requests
  });
  let client=reqwest::blocking::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(2)).build().unwrap();
  let result=read_with(&client,&endpoint,"o","r",7,1,&"a".repeat(40),&"b".repeat(40),kind);
  (result,server.join().unwrap())
 }
 fn detail()->Value {json!({"number":7,"title":"PR","body":null,"state":"open","head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40)}})}
 #[test]
 fn native_reads_use_exact_resource_routes_and_discard_changed_snapshots(){
  for kind in [Kind::Reviews,Kind::Comments] {
   let value=if matches!(kind,Kind::Reviews){review(1)}else{comment(2)};
   let (result,requests)=exchange(kind,vec![(200,detail()),(200,json!([value])),(200,detail())]);assert_eq!(result.unwrap().entries.len(),1);assert_eq!(requests.len(),3);
   assert!(requests[1].starts_with(&format!("GET /repos/o/r/pulls/7/{}?per_page=100&page=1",kind.path())));
   let mut changed=detail();changed["base"]["sha"]=json!("c".repeat(40));
   assert!(exchange(kind,vec![(200,detail()),(200,json!([])),(200,changed)]).0.unwrap_err().starts_with("PROVIDER_SNAPSHOT_CHANGED"));
  }
 }
 #[test]
 fn error_responses_are_redacted_and_never_retried(){
  for (status,expected) in [(401,"PROVIDER_AUTH_REQUIRED"),(403,"PROVIDER_ACCESS_OR_RATE_LIMIT"),(302,"PROVIDER_READ_FAILED"),(500,"PROVIDER_READ_FAILED")] {
   let (result,requests)=exchange(Kind::Reviews,vec![(status,json!({"message":"SECRET"}))]);assert_eq!(result.unwrap_err(),expected);assert_eq!(requests.len(),1);
  }
 }
}
