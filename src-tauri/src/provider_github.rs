//! Read-only GitHub.com PR adapter. Provider credentials are deliberately not
//! borrowed from Git transport. Authenticated accounts remain a separate scope.
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};
const LIMIT: u64 = 2 * 1024 * 1024;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub draft: Option<bool>,
    pub updated_at: Option<String>,
    pub head_branch: String,
    pub base_branch: String,
    pub requested_reviewers: Vec<String>,
    pub labels: Option<Vec<String>>,
    pub milestone: Option<Milestone>,
    pub url: String,
}
#[derive(Debug, Serialize)]
pub struct Milestone { pub number: String, pub title: String }
#[derive(Deserialize)]
struct RawMilestone { number: u64, title: String }
#[derive(Deserialize)]
struct Label { name: String }
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub provider: &'static str,
    pub host: &'static str,
    pub repository: String,
    pub items: Vec<PullRequest>,
    pub page: u32,
    pub next_page: Option<u32>,
}
#[derive(Deserialize)]
struct User { login: String }
#[derive(Deserialize)]
struct Branch { #[serde(rename="ref")] name: String }
#[derive(Deserialize)]
struct Raw {
    id: u64, number: u64, title: String, state: String,
    updated_at: Option<String>,
    user: Option<User>, draft: Option<bool>, head: Branch, base: Branch,
    #[serde(default)] requested_reviewers: Vec<User>,
    labels: Option<Vec<Label>>, milestone: Option<RawMilestone>,
}
fn segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s != "." && s != ".."
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub fn validate(owner: &str, repo: &str, page: u32) -> Result<(), String> {
    if !segment(owner) || !segment(repo) || !(1..=1000).contains(&page) {
        return Err("INVALID_PROVIDER_REPOSITORY_OR_PAGE".into());
    }
    Ok(())
}
fn text_ok(s: &str, limit: usize) -> bool {
    s.len() <= limit && !s.chars().any(|c| c.is_control())
}
fn decode(bytes: &[u8], owner: &str, repo: &str, page: u32, next: bool) -> Result<Page, String> {
    if bytes.len() as u64 > LIMIT { return Err("PROVIDER_RESPONSE_TOO_LARGE".into()); }
    let raw: Vec<Raw> = serde_json::from_slice(bytes).map_err(|_| "INVALID_PROVIDER_RESPONSE")?;
    if raw.len() > 100 { return Err("INVALID_PROVIDER_RESPONSE".into()); }
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for r in raw {
        if r.id == 0 || r.number == 0 || r.number > 9_007_199_254_740_991 || !seen.insert(r.id) || r.state != "open"
            || r.updated_at.as_ref().is_some_and(|value|!text_ok(value,64))
            || !text_ok(&r.title, 4096) || !text_ok(&r.head.name, 1024) || !text_ok(&r.base.name, 1024)
            || r.user.as_ref().is_some_and(|u| !text_ok(&u.login, 256))
            || r.labels.as_ref().is_some_and(|labels|labels.len()>100 || labels.iter().any(|l|!text_ok(&l.name,256)))
            || r.milestone.as_ref().is_some_and(|m|m.number==0 || !text_ok(&m.title,1024))
            || r.requested_reviewers.len() > 100 || r.requested_reviewers.iter().any(|u| !text_ok(&u.login, 256)) {
            return Err("INVALID_PROVIDER_RESPONSE".into());
        }
        items.push(PullRequest {id:r.id.to_string(),number:r.number,title:r.title,author:r.user.map(|u|u.login),draft:r.draft,updated_at:r.updated_at,
            head_branch:r.head.name,base_branch:r.base.name,requested_reviewers:r.requested_reviewers.into_iter().map(|u|u.login).collect(),
            labels:r.labels.map(|labels|labels.into_iter().map(|l|l.name).collect()),
            milestone:r.milestone.map(|m|Milestone{number:m.number.to_string(),title:m.title}),
            url:format!("https://github.com/{owner}/{repo}/pull/{}",r.number)});
    }
    if next && page == 1000 { return Err("PROVIDER_PAGE_LIMIT".into()); }
    Ok(Page {provider:"github",host:"github.com",repository:format!("{owner}/{repo}"),items,page,next_page:next.then_some(page+1)})
}
pub fn read(owner: &str, repo: &str, page: u32) -> Result<Page, String> {
    validate(owner,repo,page)?;
    fetch(&format!("https://api.github.com/repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100&page={page}"),owner,repo,page)
}
fn fetch(url: &str, owner: &str, repo: &str, page: u32) -> Result<Page, String> {
    let (bytes,next)=fetch_bytes(url)?;
    decode(&bytes,owner,repo,page,next)
}
fn fetch_bytes(url: &str) -> Result<(Vec<u8>,bool),String> {
    let client=reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(15))
        .build().map_err(|_|"PROVIDER_CLIENT_FAILED")?;
    let response=client.get(url).header("User-Agent","FLOE/0.1")
        .header("Accept","application/vnd.github+json").header("X-GitHub-Api-Version","2026-03-10")
        .send().map_err(|e|if e.is_timeout(){"PROVIDER_TIMEOUT"}else{"PROVIDER_NETWORK_FAILED"})?;
    let code=response.status().as_u16();
    if code != 200 { return Err(match code {301..=399=>"PROVIDER_REDIRECT_REFUSED",401=>"PROVIDER_AUTH_REQUIRED",403|429=>"PROVIDER_ACCESS_OR_RATE_LIMIT",404=>"PROVIDER_NOT_FOUND_OR_PRIVATE",500..=599=>"PROVIDER_UNAVAILABLE",_=>"PROVIDER_REQUEST_FAILED"}.into()); }
    if response.content_length().is_some_and(|n|n>LIMIT) {return Err("PROVIDER_RESPONSE_TOO_LARGE".into());}
    // Link targets are never followed. Only the presence of a next relation is
    // consumed; the next request is rebuilt against our fixed GitHub endpoint.
    let next=response.headers().get_all("link").iter().filter_map(|v|v.to_str().ok()).any(|v|v.split(',').any(|link|link.split(';').skip(1).any(|part|part.trim()=="rel=\"next\"")));
    let mut bytes=Vec::new();response.take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"PROVIDER_READ_FAILED")?;
    if bytes.len() as u64>LIMIT {return Err("PROVIDER_RESPONSE_TOO_LARGE".into());}
    Ok((bytes,next))
}

#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Issue {
    pub id: String, pub number: u64, pub title: String, pub author: Option<String>,
    pub assignees: Option<Vec<String>>, pub labels: Option<Vec<String>>,
    pub milestone: Option<Milestone>, pub updated_at: Option<String>, pub url: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct IssuePage {
    pub provider: &'static str, pub host: &'static str, pub repository: String,
    pub page: u32, pub next_page: Option<u32>, pub items: Vec<Issue>,
}
#[derive(Deserialize)]
struct RawIssue {
    id:u64, number:u64, title:String, state:String, user:Option<User>, updated_at:Option<String>,
    assignees:Option<Vec<User>>, labels:Option<Vec<Label>>, milestone:Option<RawMilestone>,
}
pub fn read_issues(owner:&str,repo:&str,page:u32)->Result<IssuePage,String>{
    validate(owner,repo,page)?;
    let (bytes,next)=fetch_bytes(&format!("https://api.github.com/repos/{owner}/{repo}/issues?state=open&sort=updated&direction=desc&per_page=100&page={page}"))?;
    decode_issues(&bytes,owner,repo,page,next)
}
fn decode_issues(bytes:&[u8],owner:&str,repo:&str,page:u32,next:bool)->Result<IssuePage,String>{
    validate(owner,repo,page)?;
    if bytes.len() as u64>LIMIT {return Err("PROVIDER_RESPONSE_TOO_LARGE".into());}
    let raw:Vec<serde_json::Value>=serde_json::from_slice(bytes).map_err(|_|"INVALID_PROVIDER_RESPONSE")?;
    if raw.len()>100 {return Err("INVALID_PROVIDER_RESPONSE".into());}
    let mut items=Vec::new();let mut seen=std::collections::HashSet::new();
    for value in raw {
        let object=value.as_object().ok_or("INVALID_PROVIDER_RESPONSE")?;
        // GitHub mixes PRs into this endpoint. Presence is authoritative even if
        // metadata is null: never turn an ambiguous PR into an ordinary issue.
        if object.contains_key("pull_request") {continue;}
        let row:RawIssue=serde_json::from_value(value).map_err(|_|"INVALID_PROVIDER_RESPONSE")?;
        if row.id==0 || !seen.insert(row.id) || row.number==0 || row.number>9_007_199_254_740_991 || row.state!="open"
            || row.updated_at.as_ref().is_some_and(|value|!text_ok(value,64))
            || !text_ok(&row.title,4096) || row.user.as_ref().is_some_and(|u|!text_ok(&u.login,256))
            || row.assignees.as_ref().is_some_and(|v|v.len()>100 || v.iter().any(|u|!text_ok(&u.login,256)))
            || row.labels.as_ref().is_some_and(|v|v.len()>100 || v.iter().any(|l|!text_ok(&l.name,256)))
            || row.milestone.as_ref().is_some_and(|m|m.number==0 || !text_ok(&m.title,1024)) {
            return Err("INVALID_PROVIDER_RESPONSE".into());
        }
        items.push(Issue{updated_at:row.updated_at,id:row.id.to_string(),number:row.number,title:row.title,author:row.user.map(|u|u.login),
            assignees:row.assignees.map(|v|v.into_iter().map(|u|u.login).collect()),
            labels:row.labels.map(|v|v.into_iter().map(|l|l.name).collect()),
            milestone:row.milestone.map(|m|Milestone{number:m.number.to_string(),title:m.title}),
            url:format!("https://github.com/{owner}/{repo}/issues/{}",row.number)});
    }
    if next && page==1000 {return Err("PROVIDER_PAGE_LIMIT".into());}
    Ok(IssuePage{provider:"github",host:"github.com",repository:format!("{owner}/{repo}"),page,next_page:next.then_some(page+1),items})
}

#[cfg(test)]
mod tests {
    use super::*;
    const BODY:&str=r#"[{"id":42,"number":7,"title":"Fix <literal>","state":"open","user":{"login":"author"},"draft":false,"head":{"ref":"feature"},"base":{"ref":"main"},"requested_reviewers":[{"login":"reviewer"}],"html_url":"https://evil.invalid/secret"}]"#;
    #[test] fn issues_exclude_prs_without_losing_pagination() {
        let mixed=serde_json::json!([
            {"id":9007199254740993u64,"number":8,"title":"Issue <literal>","state":"open","user":null,"assignees":[{"login":"person"}],"labels":[{"name":"bug"}],"milestone":{"number":9,"title":"Release"},"html_url":"https://evil.invalid"},
            {"pull_request":{"url":"https://evil.invalid"}}, {"pull_request":null}
        ]);
        let page=decode_issues(&serde_json::to_vec(&mixed).unwrap(),"o","r",1,true).unwrap();
        assert_eq!(page.items.len(),1);assert_eq!(page.next_page,Some(2));
        let issue=&page.items[0];assert_eq!(issue.id,"9007199254740993");assert_eq!(issue.url,"https://github.com/o/r/issues/8");
        assert_eq!(issue.author,None);assert_eq!(issue.assignees,Some(vec!["person".into()]));assert_eq!(issue.labels,Some(vec!["bug".into()]));assert_eq!(issue.milestone.as_ref().unwrap().number,"9");
        let page=decode_issues(br#"[{"pull_request":{}}]"#,"o","r",2,true).unwrap();assert!(page.items.is_empty());assert_eq!(page.next_page,Some(3));
        assert_eq!(decode_issues(b"[]","o","r",3,false).unwrap().next_page,None);
    }
    #[test] fn issues_reject_malformed_duplicate_and_oversized_values() {
        for body in ["null","[null]","[{}]","secret"] {assert_eq!(decode_issues(body.as_bytes(),"o","r",1,false).unwrap_err(),"INVALID_PROVIDER_RESPONSE");}
        let base=serde_json::json!({"id":1,"number":1,"title":"Issue","state":"open","user":null});
        let decode_value=|v:serde_json::Value|decode_issues(&serde_json::to_vec(&v).unwrap(),"o","r",1,false);
        assert!(decode_value(serde_json::json!([base.clone(),base.clone()])).is_err());
        for (key,value) in [("id",serde_json::json!(0)),("number",serde_json::json!(9007199254740992u64)),("state",serde_json::json!("closed")),("title",serde_json::json!("bad\ntext")),("assignees",serde_json::json!([{"login":"bad\nuser"}])),("labels",serde_json::json!([{"name":"bad\nlabel"}])),("milestone",serde_json::json!({"number":0,"title":"x"}))] {
            let mut row=base.clone();row[key]=value;assert!(decode_value(serde_json::json!([row])).is_err(),"{key}");
        }
        assert!(decode_value(serde_json::json!(vec![serde_json::json!({"pull_request":{}});101])).is_err());
        assert!(decode_issues(&vec![b' ';LIMIT as usize+1],"o","r",1,false).is_err());
        assert!(decode_issues(b"[]","o","r",1000,true).is_err());
        let unknown=decode_value(serde_json::json!([base])).unwrap();assert!(unknown.items[0].assignees.is_none());assert!(unknown.items[0].labels.is_none());
    }
    #[test] fn issues_actual_http_uses_shared_bounded_transport() {
        use std::io::{Read,Write};
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/issues?state=open&page=2",listener.local_addr().unwrap());
        let worker=std::thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut bytes=[0;4096];let n=stream.read(&mut bytes).unwrap();let request=String::from_utf8_lossy(&bytes[..n]);assert!(request.starts_with("GET /issues?state=open&page=2 "));assert!(!request.to_lowercase().contains("authorization:"));let body=r#"[{"pull_request":{}}]"#;write!(stream,"HTTP/1.1 200 OK\r\nLink: <https://evil.invalid/>; rel=\"next\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();});
        let(bytes,next)=fetch_bytes(&url).unwrap();worker.join().unwrap();let page=decode_issues(&bytes,"o","r",2,next).unwrap();assert!(page.items.is_empty());assert_eq!(page.next_page,Some(3));
    }
    #[test] fn repository_scope_and_unknown_metadata() {
        for bad in ["", "..", "a/b", "x?token=secret", "x%2f", "x\n", "한글"] {assert!(validate(bad,"repo",1).is_err());}
        assert!(validate("owner","repo",0).is_err());assert!(validate("owner","repo",1001).is_err());
        let p=decode(BODY.as_bytes(),"owner","repo",1,true).unwrap();assert_eq!(p.next_page,Some(2));assert_eq!(p.items[0].url,"https://github.com/owner/repo/pull/7");
        assert_eq!(p.items[0].requested_reviewers,vec!["reviewer"]);
        let unknown=BODY.replace("\"draft\":false,","").replace("{\"login\":\"author\"}","null");let p=decode(unknown.as_bytes(),"owner","repo",1,false).unwrap();assert_eq!(p.items[0].draft,None);assert_eq!(p.items[0].author,None);
    }
    #[test] fn invalid_response_is_bounded_and_not_echoed() {
        for body in ["{secret}","null","[{}]"] {assert_eq!(decode(body.as_bytes(),"o","r",1,false).unwrap_err(),"INVALID_PROVIDER_RESPONSE");}
        assert!(decode(&vec![b' ';LIMIT as usize+1],"o","r",1,false).unwrap_err().contains("TOO_LARGE"));
        assert!(decode(BODY.replace("\"open\"","\"closed\"").as_bytes(),"o","r",1,false).is_err());
        assert!(decode(BODY.as_bytes(),"o","r",1000,true).is_err());
    }
    #[test] fn label_and_milestone_mapping_rejects_invalid_metadata() {
        let mut value:serde_json::Value=serde_json::from_str(BODY).unwrap();
        value[0]["labels"]=serde_json::json!([{"name":"bug"},{"name":"한글 <literal>"}]);
        value[0]["milestone"]=serde_json::json!({"number":9007199254740993u64,"title":"Release"});
        let bytes=serde_json::to_vec(&value).unwrap();let page=decode(&bytes,"o","r",1,false).unwrap();
        assert_eq!(page.items[0].labels.as_ref().unwrap(),&vec!["bug","한글 <literal>"]);
        assert_eq!(page.items[0].milestone.as_ref().unwrap().number,"9007199254740993");
        value[0]["labels"][0]["name"]=serde_json::json!("bad\nlabel");
        assert!(decode(&serde_json::to_vec(&value).unwrap(),"o","r",1,false).is_err());
        value[0]["labels"]=serde_json::json!([]);value[0]["milestone"]=serde_json::Value::Null;
        let page=decode(&serde_json::to_vec(&value).unwrap(),"o","r",1,false).unwrap();
        assert_eq!(page.items[0].labels,Some(vec![]));assert!(page.items[0].milestone.is_none());
    }
    #[test] fn actual_http_is_read_only_and_does_not_follow_redirects() {
        use std::io::{Read,Write};
        for (status,headers,body,expected) in [("200 OK","Link: <https://evil.invalid/>; rel=\"next\"\r\n",BODY,None),("302 Found","Location: https://evil.invalid/\r\n","",Some("PROVIDER_REDIRECT_REFUSED")),("401 Unauthorized","","secret token",Some("PROVIDER_AUTH_REQUIRED")),("429 Too Many Requests","","",Some("PROVIDER_ACCESS_OR_RATE_LIMIT"))] {
            let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/pulls",listener.local_addr().unwrap());
            let worker=std::thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut bytes=[0;4096];let n=stream.read(&mut bytes).unwrap();let request=String::from_utf8_lossy(&bytes[..n]);assert!(request.starts_with("GET /pulls "));assert!(!request.to_lowercase().contains("authorization:"));write!(stream,"HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();});
            let result=fetch(&url,"owner","repo",1);worker.join().unwrap();if let Some(error)=expected {assert_eq!(result.unwrap_err(),error);}else{assert_eq!(result.unwrap().next_page,Some(2));}
        }
    }
}
