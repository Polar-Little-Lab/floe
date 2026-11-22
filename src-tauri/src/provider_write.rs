//! Explicit GitHub.com writes. Never follows redirects or retries a mutation.
pub mod discussion;
pub mod metadata;
pub mod checks;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    ChangeReviewers { number:u64, reviewers:Vec<String>, remove:bool, #[serde(rename="expectedReviewers")] expected_reviewers:Vec<String> },
    CreateIssue {
        title: String,
        body: String,
    },
    SetAssignees { number:u64, pull:bool, assignees:Vec<String>, #[serde(rename="expectedAssignees")] expected_assignees:Vec<String> },
    SetLabels { number:u64, pull:bool, labels:Vec<String>, #[serde(rename="expectedLabels")] expected_labels:Vec<String> },
    SetItemState { number:u64, pull:bool, state:State, #[serde(rename="expectedState")] expected_state:State },
    UpdateIssue {
        number: u64,
        title: String,
        body: String,
        state: State,
    },
    CreatePull {
        title: String,
        body: String,
        head: String,
        base: String,
        draft: bool,
    },
    UpdatePull {
        number: u64,
        title: String,
        body: String,
        state: State,
    },
    Review {
        number: u64,
        sha: String,
        body: String,
        event: Review,
    },
    DeleteComment { number:u64, pull:bool, #[serde(rename="commentId")] comment_id:u64, #[serde(rename="expectedBody")] expected_body:String },
    UpdateComment { number:u64, pull:bool, #[serde(rename="commentId")] comment_id:u64, #[serde(rename="expectedBody")] expected_body:String, body:String },
    CreateComment { number:u64, pull:bool, body:String },
    ReplyReview {
        number: u64,
        #[serde(rename = "commentId")]
        comment_id: u64,
        body: String,
    },
    Merge {
        number: u64,
        sha: String,
        method: MergeStrategy,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Open,
    Closed,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Review {
    Comment,
    Approve,
    RequestChanges,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: State,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub author: Option<String>,
    pub head_branch: Option<String>,
    pub base_branch: Option<String>,
    pub reviewers: Option<Vec<String>>,
    pub assignees: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub milestone: Option<String>,
    pub draft: Option<bool>,
    pub merged: Option<bool>,
    pub mergeable: Option<bool>,
    pub changed_files: Option<u64>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub commits: Option<u64>,
    pub url: String,
}
#[derive(Debug, Serialize)]
pub struct Receipt {
    pub number: u64,
    pub url: String,
    pub merged: bool,
}
struct Request {
    method: reqwest::Method,
    path: String,
    body: Value,
    number: Option<u64>,
    pull: bool,
    review: bool,
    merge: bool,
}
fn number(n: u64) -> bool {
    n > 0 && n <= 9_007_199_254_740_991
}
fn text(s: &str, limit: usize, multiline: bool) -> bool {
    s.len() <= limit
        && !s
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
}
fn sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn branch(s: &str) -> bool {
    !s.is_empty() && text(s, 1024, false) && !s.starts_with('-') && !s.contains(['?', '#', '\\'])
}
fn label_set(labels:&[String])->Result<std::collections::BTreeSet<String>,String>{
    if labels.len()>100||labels.iter().any(|label|label.is_empty()||!text(label,256,false)){return Err("INVALID_PROVIDER_LABELS".into());}
    let set=labels.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    if set.len()!=labels.len(){return Err("INVALID_PROVIDER_LABELS".into());}Ok(set)
}
fn account_set(accounts:&[String])->Result<std::collections::BTreeSet<String>,String>{
    label_set(accounts)?;
    if accounts.iter().any(|name|name.chars().any(char::is_whitespace)){return Err("INVALID_PROVIDER_ASSIGNEES".into());}
    let set=accounts.iter().map(|name|name.to_ascii_lowercase()).collect::<std::collections::BTreeSet<_>>();
    if set.len()!=accounts.len(){return Err("INVALID_PROVIDER_ASSIGNEES".into());}Ok(set)
}
fn request(action: &Action) -> Result<Request, String> {
    let (method, path, body, n, pull, review, merge) = match action {
        Action::CreateIssue { title, body } => (
            reqwest::Method::POST,
            "issues".into(),
            json!({"title":title,"body":body}),
            None,
            false,
            false,
            false,
        ),
        Action::ChangeReviewers {number,reviewers,remove,expected_reviewers} => {
            let changed=account_set(reviewers)?;let previous=account_set(expected_reviewers)?;
            if changed.is_empty() || (*remove&&!changed.is_subset(&previous)) || (!*remove&&!changed.is_disjoint(&previous)) || (!*remove&&previous.len()+changed.len()>100) {return Err("INVALID_REVIEWER_CHANGE".into());}
            (if *remove{reqwest::Method::DELETE}else{reqwest::Method::POST},format!("pulls/{number}/requested_reviewers"),json!({"reviewers":reviewers}),Some(*number),true,false,false)
        },
        Action::SetAssignees {number,pull,assignees,expected_assignees} => {
            account_set(assignees)?;account_set(expected_assignees)?;
            (reqwest::Method::PATCH,format!("issues/{number}"),json!({"assignees":assignees}),Some(*number),*pull,false,false)
        },
        Action::SetLabels {number,pull,labels,expected_labels} => {
            label_set(labels)?;label_set(expected_labels)?;
            (reqwest::Method::PUT,format!("issues/{number}/labels"),json!({"labels":labels}),Some(*number),*pull,false,false)
        },
        Action::SetItemState {number,pull,state,expected_state} => {
            if std::mem::discriminant(state)==std::mem::discriminant(expected_state){return Err("INVALID_PROVIDER_STATE_CHANGE".into());}
            (reqwest::Method::PATCH,format!("{}/{number}",if *pull{"pulls"}else{"issues"}),json!({"state":state}),Some(*number),*pull,false,false)
        },
        Action::UpdateIssue {
            number,
            title,
            body,
            state,
        } => (
            reqwest::Method::PATCH,
            format!("issues/{number}"),
            json!({"title":title,"body":body,"state":state}),
            Some(*number),
            false,
            false,
            false,
        ),
        Action::CreatePull {
            title,
            body,
            head,
            base,
            draft,
        } => {
            if !branch(head) || !branch(base) || head == base {
                return Err("INVALID_PROVIDER_BRANCH".into());
            }
            (
                reqwest::Method::POST,
                "pulls".into(),
                json!({"title":title,"body":body,"head":head,"base":base,"draft":draft}),
                None,
                true,
                false,
                false,
            )
        }
        Action::UpdatePull {
            number,
            title,
            body,
            state,
        } => (
            reqwest::Method::PATCH,
            format!("pulls/{number}"),
            json!({"title":title,"body":body,"state":state}),
            Some(*number),
            true,
            false,
            false,
        ),
        Action::Review {
            number,
            sha: oid,
            body,
            event,
        } => {
            if !sha(oid) || (!matches!(event, Review::Approve) && body.trim().is_empty()) {
                return Err("INVALID_PROVIDER_REVIEW".into());
            }
            (
                reqwest::Method::POST,
                format!("pulls/{number}/reviews"),
                json!({"commit_id":oid,"body":body,"event":event}),
                Some(*number),
                true,
                true,
                false,
            )
        }
        Action::DeleteComment {number,pull,comment_id,expected_body} => {
            if !self::number(*comment_id)||!text(expected_body,65536,true){return Err("INVALID_PROVIDER_COMMENT".into());}
            (reqwest::Method::DELETE,format!("issues/comments/{comment_id}"),Value::Null,Some(*number),*pull,false,false)
        },
        Action::UpdateComment {number,pull,comment_id,expected_body,body} => {
            if !self::number(*comment_id)||!text(expected_body,65536,true)||body.trim().is_empty(){return Err("INVALID_PROVIDER_COMMENT".into());}
            (reqwest::Method::PATCH,format!("issues/comments/{comment_id}"),json!({"body":body}),Some(*number),*pull,false,false)
        },
        Action::CreateComment {number,pull,body} => {
            if body.trim().is_empty(){return Err("INVALID_PROVIDER_COMMENT".into());}
            (reqwest::Method::POST,format!("issues/{number}/comments"),json!({"body":body}),Some(*number),*pull,false,false)
        },
        Action::ReplyReview {number,comment_id,body} => {
            if !self::number(*comment_id) || body.trim().is_empty() {return Err("INVALID_PROVIDER_REPLY".into());}
            (reqwest::Method::POST,format!("pulls/{number}/comments/{comment_id}/replies"),json!({"body":body}),Some(*number),true,true,false)
        },
        Action::Merge {
            number,
            sha: oid,
            method,
        } => {
            if !sha(oid) {
                return Err("INVALID_PROVIDER_SHA".into());
            }
            (
                reqwest::Method::PUT,
                format!("pulls/{number}/merge"),
                json!({"sha":oid,"merge_method":method}),
                Some(*number),
                true,
                false,
                true,
            )
        }
    };
    if n.is_some_and(|n| !number(n)) {
        return Err("INVALID_PROVIDER_NUMBER".into());
    }
    if let Some(title) = body.get("title").and_then(Value::as_str) {
        if title.trim().is_empty() || !text(title, 256, false) {
            return Err("INVALID_PROVIDER_TITLE".into());
        }
    }
    if body
        .get("body")
        .and_then(Value::as_str)
        .is_some_and(|s| !text(s, 65536, true))
    {
        return Err("INVALID_PROVIDER_BODY".into());
    }
    Ok(Request {
        method,
        path,
        body,
        number: n,
        pull,
        review,
        merge,
    })
}
pub fn validate(owner: &str, repo: &str, action: &Action) -> Result<(), String> {
    crate::provider_github::validate(owner, repo, 1)?;
    request(action)?;
    Ok(())
}
pub fn endpoint(owner: &str, repo: &str) -> Result<String, String> {
    crate::provider_github::validate(owner, repo, 1)?;
    Ok(format!("https://api.github.com/repos/{owner}/{repo}"))
}
pub fn describe(owner: &str, repo: &str, action: &Action) -> Result<String, String> {
    validate(owner, repo, action)?;
    Ok(format!("GitHub {owner}/{repo}\n{}\n\nThis changes the remote repository. No automatic retry. For edits, supplied title/body replace the current values. Review/merge targets the displayed commit.",serde_json::to_string_pretty(action).map_err(|_|"INVALID_PROVIDER_ACTION")?))
}
fn client(
    credential: &crate::credential_vault::Credential,
) -> Result<reqwest::blocking::Client, String> {
    let value = credential.bearer_authorization()?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, value);
    reqwest::blocking::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .default_headers(headers)
        .build()
        .map_err(|_| "PROVIDER_CLIENT_FAILED".into())
}
fn send(
    client: &reqwest::blocking::Client,
    url: &str,
    method: reqwest::Method,
    body: Option<&Value>,
    write: bool,
) -> Result<Value, String> {
    let deletion=write&&method==reqwest::Method::DELETE&&body.is_none();
    let mut req = client
        .request(method, url)
        .header("User-Agent", "FLOE/0.1")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10");
    if let Some(body) = body {
        req = req
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(body).map_err(|_| "INVALID_PROVIDER_ACTION")?);
    }
    let unknown = if write {
        "PROVIDER_WRITE_OUTCOME_UNKNOWN: inspect GitHub before retrying"
    } else {
        "PROVIDER_READ_FAILED"
    };
    let response = req.send().map_err(|_| unknown)?;
    if deletion {
        if response.status().as_u16()==204{return Ok(Value::Null);}
        if matches!(response.status().as_u16(),200|201){return Err(unknown.into());}
    }
    match response.status().as_u16() {
        200 | 201 => (),
        401 => return Err("PROVIDER_AUTH_REQUIRED".into()),
        403 | 429 => return Err("PROVIDER_ACCESS_OR_RATE_LIMIT".into()),
        404 => return Err("PROVIDER_NOT_FOUND_OR_PRIVATE".into()),
        405 | 409 | 412 | 422 => {
            return Err(
                "PROVIDER_WRITE_REJECTED: refresh state and permissions before retrying".into(),
            )
        }
        _ => return Err(unknown.into()),
    }
    if response
        .content_length()
        .is_some_and(|n| n > 2 * 1024 * 1024)
    {
        return Err(unknown.into());
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unknown)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(unknown.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| unknown.into())
}
fn item_url(owner: &str, repo: &str, n: u64, pull: bool) -> String {
    format!(
        "https://github.com/{owner}/{repo}/{}/{n}",
        if pull { "pull" } else { "issues" }
    )
}
pub fn inspect(
    owner: &str,
    repo: &str,
    n: u64,
    pull: bool,
    credential: &crate::credential_vault::Credential,
) -> Result<Detail, String> {
    let endpoint = endpoint(owner, repo)?;
    if !number(n) {
        return Err("INVALID_PROVIDER_NUMBER".into());
    }
    let value = send(
        &client(credential)?,
        &format!("{endpoint}/{}/{n}", if pull { "pulls" } else { "issues" }),
        reqwest::Method::GET,
        None,
        false,
    )?;
    detail(value, owner, repo, n, pull)
}
fn detail_text(value: &Value, limit: usize) -> Result<Option<String>, String> {
    if value.is_null() { return Ok(None); }
    value.as_str().filter(|s| text(s,limit,false)).map(|s|Some(s.to_owned())).ok_or_else(||"INVALID_PROVIDER_DETAIL_METADATA".into())
}
fn detail_names(value: &Value, field: &str) -> Result<Option<Vec<String>>, String> {
    if value.is_null() { return Ok(None); }
    let entries=value.as_array().filter(|items|items.len()<=100).ok_or("INVALID_PROVIDER_DETAIL_METADATA")?;
    entries.iter().map(|item|detail_text(&item[field],256)?.ok_or_else(||"INVALID_PROVIDER_DETAIL_METADATA".to_string())).collect::<Result<Vec<_>,_>>().map(Some)
}
fn detail_count(value:&Value)->Result<Option<u64>,String>{
    if value.is_null(){return Ok(None);}
    value.as_u64().filter(|n|*n<=9_007_199_254_740_991).map(Some).ok_or_else(||"INVALID_PROVIDER_DETAIL_COUNT".into())
}
fn detail_bool(value: &Value) -> Result<Option<bool>,String> {
    if value.is_null(){Ok(None)}else{value.as_bool().map(Some).ok_or_else(||"INVALID_PROVIDER_DETAIL_METADATA".into())}
}
fn detail(v: Value, owner: &str, repo: &str, n: u64, pull: bool) -> Result<Detail, String> {
    if v["number"].as_u64() != Some(n) || (!pull && v.get("pull_request").is_some()) {
        return Err("INVALID_PROVIDER_DETAIL".into());
    }
    let title = v["title"]
        .as_str()
        .ok_or("INVALID_PROVIDER_DETAIL")?
        .to_owned();
    let body = match &v["body"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => return Err("INVALID_PROVIDER_DETAIL".into()),
    };
    if !text(&title, 256, false) || !text(&body, 65536, true) {
        return Err("INVALID_PROVIDER_DETAIL".into());
    }
    let state =
        serde_json::from_value(v["state"].clone()).map_err(|_| "INVALID_PROVIDER_DETAIL")?;
    let head_sha = if pull {
        let s = v["head"]["sha"]
            .as_str()
            .filter(|s| sha(s))
            .ok_or("INVALID_PROVIDER_DETAIL")?;
        Some(s.into())
    } else {
        None
    };
    Ok(Detail {
        number: n,
        title,
        body,
        state,
        head_sha,
        base_sha: if pull { Some(v["base"]["sha"].as_str().filter(|s| sha(s)).ok_or("INVALID_PROVIDER_DETAIL")?.into()) } else { None },
        author: detail_text(&v["user"]["login"],256)?,
        head_branch: if pull { detail_text(&v["head"]["label"],1024)?.or(detail_text(&v["head"]["ref"],1024)?) } else { None },
        base_branch: if pull { detail_text(&v["base"]["label"],1024)?.or(detail_text(&v["base"]["ref"],1024)?) } else { None },
        reviewers: if pull { detail_names(&v["requested_reviewers"],"login")? } else { None },
        assignees: detail_names(&v["assignees"],"login")?,
        labels: detail_names(&v["labels"],"name")?,
        milestone: detail_text(&v["milestone"]["title"],256)?,
        draft: if pull { detail_bool(&v["draft"])? } else { None },
        merged: if pull { detail_bool(&v["merged"])? } else { None },
        mergeable: if pull { detail_bool(&v["mergeable"])? } else { None },
        changed_files: if pull { detail_count(&v["changed_files"])? } else { None },
        additions: if pull { detail_count(&v["additions"])? } else { None },
        deletions: if pull { detail_count(&v["deletions"])? } else { None },
        commits: if pull { detail_count(&v["commits"])? } else { None },
        url: item_url(owner, repo, n, pull),
    })
}
/// A page is bound to both sides of the PR, checked before and after reading.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullFiles {
    pub files: Vec<PullFile>,
    pub next_page: Option<u32>,
    pub limit_reached: bool,
}
pub fn validate_files(owner: &str, repo: &str, n: u64, page: u32, head: &str, base: &str) -> Result<(), String> {
    endpoint(owner, repo)?;
    if !number(n) || !(1..=30).contains(&page) || !sha(head) || !sha(base) {
        return Err("INVALID_PROVIDER_FILES_REQUEST".into());
    }
    Ok(())
}
fn parse_files(v: Value, page: u32) -> Result<PullFiles, String> {
    let items=v.as_array().filter(|items| items.len()<=100).ok_or("INVALID_PROVIDER_FILES")?;
    let mut files=Vec::new();
    let mut paths=std::collections::HashSet::new();
    for item in items {
        // These paths are display data only: never use them for filesystem access.
        let path=item["filename"].as_str().filter(|s| !s.is_empty() && s.len()<=32768 && !s.contains('\0')).ok_or("INVALID_PROVIDER_FILES")?.to_owned();
        if !paths.insert(path.clone()) { return Err("INVALID_PROVIDER_FILES".into()); }
        let previous_path=match &item["previous_filename"] {
            Value::Null=>None,
            Value::String(s) if !s.is_empty() && s.len()<=32768 && !s.contains('\0')=>Some(s.clone()),
            _=>return Err("INVALID_PROVIDER_FILES".into()),
        };
        let status=item["status"].as_str().filter(|s| matches!(*s,"added"|"removed"|"modified"|"renamed"|"copied"|"changed"|"unchanged")).ok_or("INVALID_PROVIDER_FILES")?.into();
        let count=|key:&str| item[key].as_u64().filter(|n| *n<=9_007_199_254_740_991).ok_or("INVALID_PROVIDER_FILES");
        let patch=match &item["patch"] {
            Value::Null=>None,
            Value::String(s) if s.len()<=1024*1024=>Some(s.clone()),
            _=>return Err("INVALID_PROVIDER_FILES".into()),
        };
        files.push(PullFile {path,previous_path,status,additions:count("additions")?,deletions:count("deletions")?,patch});
    }
    let full=files.len()==100;
    Ok(PullFiles {files,next_page:if full && page<30 {Some(page+1)} else {None},limit_reached:full && page==30})
}
fn check_snapshot(client:&reqwest::blocking::Client,item_url:&str,owner:&str,repo:&str,n:u64,head:&str,base:&str)->Result<(),String>{
    let v=send(client,item_url,reqwest::Method::GET,None,false)?;
    let d=detail(v,owner,repo,n,true)?;
    if d.head_sha.as_deref()!=Some(head) || d.base_sha.as_deref()!=Some(base) {
        return Err("PROVIDER_SNAPSHOT_CHANGED: refresh the pull request before reviewing".into());
    }
    Ok(())
}
fn files_with(client: &reqwest::blocking::Client, endpoint: &str, owner: &str, repo: &str, n: u64, page: u32, head: &str, base: &str) -> Result<PullFiles, String> {
    validate_files(owner,repo,n,page,head,base)?;
    let item_url=format!("{endpoint}/pulls/{n}");
    check_snapshot(client,&item_url,owner,repo,n,head,base)?;
    let v=send(client,&format!("{item_url}/files?per_page=100&page={page}"),reqwest::Method::GET,None,false)?;
    let result=parse_files(v,page)?;
    check_snapshot(client,&item_url,owner,repo,n,head,base)?;
    Ok(result)
}
pub fn pull_files(owner: &str, repo: &str, n: u64, page: u32, head: &str, base: &str, credential: &crate::credential_vault::Credential) -> Result<PullFiles,String> {
    validate_files(owner,repo,n,page,head,base)?;
    files_with(&client(credential)?,&endpoint(owner,repo)?,owner,repo,n,page,head,base)
}
pub fn execute(
    owner: &str,
    repo: &str,
    action: &Action,
    credential: &crate::credential_vault::Credential,
) -> Result<Receipt, String> {
    let endpoint = endpoint(owner, repo)?;
    execute_with(&client(credential)?, &endpoint, owner, repo, action)
}
fn execute_with(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    owner: &str,
    repo: &str,
    action: &Action,
) -> Result<Receipt, String> {
    validate(owner, repo, action)?;
    let r = request(action)?;
    if let Action::Merge {number:n,sha:expected_sha,..}=action {
        let previous=send(client,&format!("{endpoint}/pulls/{n}"),reqwest::Method::GET,None,false)?;
        if previous["number"].as_u64()!=Some(*n)||previous["state"].as_str()!=Some("open")||previous["merged"].as_bool()!=Some(false){return Err("PROVIDER_PULL_NOT_OPEN".into());}
        if previous["draft"].as_bool()!=Some(false){return Err("PROVIDER_PULL_DRAFT_OR_UNKNOWN".into());}
        if previous["head"]["sha"].as_str()!=Some(expected_sha.as_str()){return Err("PROVIDER_PULL_CHANGED: refresh before merging".into());}
        match previous["mergeable"].as_bool(){Some(true)=>{},Some(false)=>return Err("PROVIDER_MERGE_CONFLICTS".into()),None=>return Err("PROVIDER_MERGEABILITY_PENDING: refresh before merging".into())}
    }
    if let Action::ChangeReviewers {number:n,expected_reviewers,..}=action {
        let previous=send(client,&format!("{endpoint}/pulls/{n}"),reqwest::Method::GET,None,false)?;
        if previous["number"].as_u64()!=Some(*n)||previous["state"].as_str()!=Some("open")||previous["merged"].as_bool()!=Some(false){return Err("PROVIDER_PULL_NOT_OPEN".into());}
        let reviewers=detail_names(&previous["requested_reviewers"],"login")?.ok_or("INVALID_PROVIDER_REVIEWERS")?;
        if account_set(&reviewers)?!=account_set(expected_reviewers)?{return Err("PROVIDER_REVIEWERS_CHANGED: refresh before editing reviewers".into());}
    }
    if let Action::SetAssignees {number:n,pull,expected_assignees,..}=action {
        let previous=send(client,&format!("{endpoint}/issues/{n}"),reqwest::Method::GET,None,false)?;
        if previous["number"].as_u64()!=Some(*n)||previous.get("pull_request").is_some()!=*pull{return Err("PROVIDER_ITEM_TARGET_MISMATCH".into());}
        let assignees=detail_names(&previous["assignees"],"login")?.ok_or("INVALID_PROVIDER_ASSIGNEES")?;
        if account_set(&assignees)?!=account_set(expected_assignees)?{return Err("PROVIDER_ASSIGNEES_CHANGED: refresh item before editing assignees".into());}
    }
    if let Action::SetLabels {number:n,pull,expected_labels,..}=action {
        let previous=send(client,&format!("{endpoint}/issues/{n}"),reqwest::Method::GET,None,false)?;
        if previous["number"].as_u64()!=Some(*n)||previous.get("pull_request").is_some()!=*pull{return Err("PROVIDER_ITEM_TARGET_MISMATCH".into());}
        let labels=detail_names(&previous["labels"],"name")?.ok_or("INVALID_PROVIDER_LABELS")?;
        if label_set(&labels)?!=label_set(expected_labels)?{return Err("PROVIDER_LABELS_CHANGED: refresh item before editing labels".into());}
    }
    if let Action::SetItemState {number:n,pull,state,expected_state}=action {
        let previous=send(client,&format!("{endpoint}/{}",r.path),reqwest::Method::GET,None,false)?;
        if previous["number"].as_u64()!=Some(*n)||(!*pull&&previous.get("pull_request").is_some()){return Err("PROVIDER_ITEM_TARGET_MISMATCH".into());}
        if previous["state"]!=json!(expected_state){return Err("PROVIDER_ITEM_STATE_CHANGED: refresh item before changing its state".into());}
        if *pull&&matches!(state,State::Open)&&previous["merged"].as_bool()==Some(true){return Err("PROVIDER_PULL_ALREADY_MERGED".into());}
    }
    if let Action::UpdateComment {number:n,comment_id,expected_body,..}|Action::DeleteComment {number:n,comment_id,expected_body,..}=action {
        let previous=send(client,&format!("{endpoint}/issues/comments/{comment_id}"),reqwest::Method::GET,None,false)?;
        if previous["id"].as_u64()!=Some(*comment_id)||previous["issue_url"].as_str()!=Some(format!("{endpoint}/issues/{n}").as_str()){return Err("PROVIDER_COMMENT_TARGET_MISMATCH".into());}
        if previous["body"].as_str()!=Some(expected_body.as_str()){return Err("PROVIDER_COMMENT_CHANGED: refresh comments before changing them".into());}
    }
    let value = send(
        client,
        &format!("{endpoint}/{}", r.path),
        r.method,
        if matches!(action,Action::DeleteComment{..}){None}else{Some(&r.body)},
        true,
    )?;
    if let Action::DeleteComment {number:n,pull,..}=action {
        return Ok(Receipt{number:*n,url:item_url(owner,repo,*n,*pull),merged:false});
    }
    let unknown = "PROVIDER_WRITE_OUTCOME_UNKNOWN: inspect GitHub before retrying";
    if let Action::ChangeReviewers {number:n,reviewers,remove,expected_reviewers}=action {
        if value["number"].as_u64()!=Some(*n)||value["url"].as_str()!=Some(format!("{endpoint}/pulls/{n}").as_str()){return Err(unknown.into());}
        let returned=detail_names(&value["requested_reviewers"],"login").map_err(|_|unknown)?.ok_or(unknown)?;
        let mut expected=account_set(expected_reviewers).map_err(|_|unknown)?;
        for login in account_set(reviewers).map_err(|_|unknown)? {if *remove{expected.remove(&login);}else{expected.insert(login);}}
        if account_set(&returned).map_err(|_|unknown)?!=expected{return Err(unknown.into());}
        return Ok(Receipt{number:*n,url:item_url(owner,repo,*n,true),merged:false});
    }
    if let Action::SetAssignees {number:n,pull,assignees,..}=action {
        if value["number"].as_u64()!=Some(*n)||value.get("pull_request").is_some()!=*pull{return Err(unknown.into());}
        let returned=detail_names(&value["assignees"],"login").map_err(|_|unknown)?.ok_or(unknown)?;
        if account_set(&returned).map_err(|_|unknown)?!=account_set(assignees).map_err(|_|unknown)?{return Err(unknown.into());}
        return Ok(Receipt{number:*n,url:item_url(owner,repo,*n,*pull),merged:false});
    }
    if let Action::SetLabels {number:n,pull,labels,..}=action {
        let returned=detail_names(&value,"name").map_err(|_|unknown)?.ok_or(unknown)?;
        if label_set(&returned).map_err(|_|unknown)?!=label_set(labels).map_err(|_|unknown)?{return Err(unknown.into());}
        return Ok(Receipt{number:*n,url:item_url(owner,repo,*n,*pull),merged:false});
    }
    if let Action::SetItemState {state,..}=action {
        if value["state"]!=json!(state){return Err(unknown.into());}
    }
    if let Action::ReplyReview {comment_id,..}=action {
        if value["in_reply_to_id"].as_u64()!=Some(*comment_id) {return Err(unknown.into());}
    }
    let comment_id=if let Action::CreateComment {number:n,body,..}|Action::UpdateComment {number:n,body,..}=action {
        let id=value["id"].as_u64().filter(|n|number(*n)).ok_or(unknown)?;
        if value["issue_url"].as_str()!=Some(format!("{endpoint}/issues/{n}").as_str())||value["body"].as_str()!=Some(body.as_str()){return Err(unknown.into());}
        if let Action::UpdateComment {comment_id,..}=action {if id!=*comment_id{return Err(unknown.into());}}
        Some(id)
    }else{None};
    let n = if comment_id.is_some(){r.number.unwrap()} else if r.merge {
        if value["merged"].as_bool() != Some(true) || !value["sha"].as_str().is_some_and(sha) {
            return Err(unknown.into());
        }
        r.number.unwrap()
    } else if r.review {
        if value["id"].as_u64().filter(|n| *n > 0).is_none() {
            return Err(unknown.into());
        }
        r.number.unwrap()
    } else {
        let n = value["number"]
            .as_u64()
            .filter(|n| number(*n))
            .ok_or(unknown)?;
        if r.number.is_some_and(|expected| expected != n) {
            return Err(unknown.into());
        }
        n
    };
    Ok(Receipt {
        number: n,
        url: if let Some(id)=comment_id {format!("{}#issuecomment-{id}",item_url(owner,repo,n,r.pull))}else{item_url(owner, repo, n, r.pull)},
        merged: r.merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn exchange(
        action: &Action,
        status: u16,
        response: Value,
    ) -> (Result<Receipt, String>, String) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/repos/o/r", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..split]);
                    let length = header
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            let body = response.to_string();
            write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let result = execute_with(&client, &endpoint, "o", "r", action);
        (result, server.join().unwrap())
    }
    fn actions() -> Vec<Action> {
        vec![
            Action::CreateIssue {
                title: "issue".into(),
                body: "body".into(),
            },
            Action::UpdateIssue {
                number: 7,
                title: "edited".into(),
                body: "replacement".into(),
                state: State::Closed,
            },
            Action::CreatePull {
                title: "pull".into(),
                body: "body".into(),
                head: "topic".into(),
                base: "main".into(),
                draft: true,
            },
            Action::UpdatePull {
                number: 7,
                title: "edited pull".into(),
                body: "description".into(),
                state: State::Open,
            },
            Action::Review {
                number: 7,
                sha: "a".repeat(40),
                body: "looks good".into(),
                event: Review::Approve,
            },
            Action::Merge {
                number: 7,
                sha: "a".repeat(40),
                method: MergeStrategy::Squash,
            },
        ]
    }
    #[test]
    fn six_actions_send_exact_method_route_body_and_return_trusted_url() {
        for (i, action) in actions().iter().enumerate() {
            let expected = request(action).unwrap();
            let response = if i == 5 {
                json!({"merged":true,"sha":"b".repeat(40)})
            } else if i == 4 {
                json!({"id":99})
            } else {
                json!({"number":7,"html_url":"https://evil.invalid/"})
            };
            let (result, wire) = exchange(action, if i == 5 { 200 } else { 201 }, response);
            let result = result.unwrap();
            assert_eq!(result.number, 7);
            assert_eq!(result.merged, i == 5);
            assert_eq!(
                result.url,
                format!(
                    "https://github.com/o/r/{}/7",
                    if i < 2 { "issues" } else { "pull" }
                )
            );
            assert!(wire.starts_with(&format!(
                "{} /repos/o/r/{} HTTP/1.1",
                expected.method, expected.path
            )));
            assert_eq!(
                serde_json::from_str::<Value>(wire.split_once("\r\n\r\n").unwrap().1).unwrap(),
                expected.body
            );
        }
    }
    #[test]
    fn errors_do_not_echo_remote_body_and_never_report_false_success() {
        for (status, code) in [
            (401, "PROVIDER_AUTH_REQUIRED"),
            (403, "PROVIDER_ACCESS_OR_RATE_LIMIT"),
            (404, "PROVIDER_NOT_FOUND_OR_PRIVATE"),
            (409, "PROVIDER_WRITE_REJECTED"),
            (422, "PROVIDER_WRITE_REJECTED"),
            (429, "PROVIDER_ACCESS_OR_RATE_LIMIT"),
            (500, "PROVIDER_WRITE_OUTCOME_UNKNOWN"),
            (302, "PROVIDER_WRITE_OUTCOME_UNKNOWN"),
        ] {
            let (result, _) = exchange(&actions()[0], status, json!({"message":"SECRET_TOKEN"}));
            let err = result.unwrap_err();
            assert!(err.starts_with(code));
            assert!(!err.contains("SECRET_TOKEN"));
        }
        for (action, response) in [
            (actions()[1].clone(), json!({"number":8})),
            (actions()[4].clone(), json!({"id":0})),
            (
                actions()[5].clone(),
                json!({"merged":false,"sha":"b".repeat(40)}),
            ),
            (actions()[5].clone(), json!({"merged":true,"sha":"invalid"})),
        ] {
            assert!(exchange(&action, 200, response)
                .0
                .unwrap_err()
                .starts_with("PROVIDER_WRITE_OUTCOME_UNKNOWN"));
        }
    }
    #[test]
    fn validation_rejects_bad_targets_before_any_network() {
        assert!(validate("../x", "r", &actions()[0]).is_err());
        for action in [
            Action::CreateIssue {
                title: " ".into(),
                body: String::new(),
            },
            Action::Review {
                number: 0,
                sha: "a".repeat(40),
                body: String::new(),
                event: Review::Approve,
            },
            Action::Review {
                number: 1,
                sha: "a".repeat(40),
                body: String::new(),
                event: Review::RequestChanges,
            },
            Action::Merge {
                number: 1,
                sha: "bad".into(),
                method: MergeStrategy::Merge,
            },
            Action::CreatePull {
                title: "t".into(),
                body: String::new(),
                head: "main".into(),
                base: "main".into(),
                draft: false,
            },
        ] {
            assert!(validate("o", "r", &action).is_err());
        }
        assert!(crate::credential_vault::Credential::new("user".into(), "a\nb".into()).is_err());
        assert!(crate::credential_vault::Credential::new("user".into(), "".into()).is_err());
        assert!(serde_json::from_value::<Action>(json!({"kind":"merge","number":1,"sha":"a".repeat(40),"method":"merge","url":"http://evil.invalid"})).is_err());
    }
    #[test]
    fn bearer_header_is_sensitive_and_credential_remains_redacted() {
        let credential =
            crate::credential_vault::Credential::new("user".into(), "synthetic-token".into())
                .unwrap();
        let header = credential.bearer_authorization().unwrap();
        assert!(header.is_sensitive());
        assert_eq!(header.to_str().unwrap(), "Bearer synthetic-token");
        assert!(!format!("{credential:?}").contains("synthetic-token"));
    }
    #[test]
    fn reply_targets_exact_pr_comment_and_validates_body_before_sending(){
        let action=Action::ReplyReview {number:7,comment_id:42,body:"Reply text".into()};
        let (receipt,wire)=exchange(&action,201,json!({"id":99,"in_reply_to_id":42,"html_url":"https://evil.invalid"}));
        assert_eq!(receipt.unwrap().url,"https://github.com/o/r/pull/7");
        assert!(wire.starts_with("POST /repos/o/r/pulls/7/comments/42/replies HTTP/1.1"));
        assert!(wire.ends_with("{\"body\":\"Reply text\"}"));
        for action in [Action::ReplyReview{number:7,comment_id:0,body:"text".into()},Action::ReplyReview{number:7,comment_id:42,body:" ".into()}] {assert!(validate("o","r",&action).is_err());}
        assert!(exchange(&action,201,json!({"id":0})).0.unwrap_err().starts_with("PROVIDER_WRITE_OUTCOME_UNKNOWN"));
    }
    #[test]
    fn details_preserve_literal_text_and_reject_wrong_item_or_pull_as_issue() {
        let v = json!({"number":7,"title":"<script>literal</script>","body":null,"state":"open","head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40)}});
        let d = detail(v.clone(), "o", "r", 7, true).unwrap();
        assert_eq!(d.title, "<script>literal</script>");
        assert_eq!(d.body, "");
        assert_eq!(d.head_sha, Some("a".repeat(40)));
        assert!(detail(v.clone(), "o", "r", 8, true).is_err());
        let mut v = v;
        v["pull_request"] = Value::Null;
        assert!(detail(v, "o", "r", 7, false).is_err());
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use std::io::Write;
    fn item(head:&str,base:&str)->Value {json!({"number":7,"title":"PR","body":null,"state":"open","head":{"sha":head},"base":{"sha":base}})}
    fn file(path:&str)->Value {json!({"filename":path,"status":"modified","additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+new"})}
    fn exchange(responses:Vec<(u16,Value)>)->(Result<PullFiles,String>,Vec<String>){
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint=format!("http://{}/repos/o/r",listener.local_addr().unwrap());
        let server=std::thread::spawn(move||{
            let mut requests=Vec::new();
            for (status,value) in responses {
                let start=std::time::Instant::now();
                let mut stream=loop {
                    match listener.accept(){Ok((s,_))=>break s,Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{assert!(start.elapsed()<Duration::from_secs(5),"missing request");std::thread::sleep(Duration::from_millis(5));},Err(e)=>panic!("{e}")}
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes=Vec::new();let mut buf=[0;4096];
                while !bytes.windows(4).any(|w| w==b"\r\n\r\n") {let n=stream.read(&mut buf).unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);assert!(bytes.len()<16384);}
                requests.push(String::from_utf8(bytes).unwrap());
                let body=value.to_string();
                write!(stream,"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        let client=reqwest::blocking::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(Duration::from_secs(2)).build().unwrap();
        let result=files_with(&client,&endpoint,"o","r",7,1,&"a".repeat(40),&"b".repeat(40));
        (result,server.join().unwrap())
    }
    #[test]
    fn files_read_checks_both_commits_and_uses_only_fixed_get_routes(){
        let d=item(&"a".repeat(40),&"b".repeat(40));
        let (result,requests)=exchange(vec![(200,d.clone()),(200,json!([file("src/a.ts")])),(200,d)]);
        let files=result.unwrap();assert_eq!(files.files[0].path,"src/a.ts");assert_eq!(files.next_page,None);
        assert_eq!(requests.len(),3);assert!(requests[0].starts_with("GET /repos/o/r/pulls/7 HTTP/1.1"));assert!(requests[1].starts_with("GET /repos/o/r/pulls/7/files?per_page=100&page=1 HTTP/1.1"));assert!(requests[2].starts_with("GET /repos/o/r/pulls/7 HTTP/1.1"));
    }
    #[test]
    fn files_discard_changed_head_or_base_before_and_after_read(){
        let expected=item(&"a".repeat(40),&"b".repeat(40));
        for changed in [item(&"c".repeat(40),&"b".repeat(40)),item(&"a".repeat(40),&"c".repeat(40))] {
            assert!(exchange(vec![(200,changed.clone())]).0.unwrap_err().starts_with("PROVIDER_SNAPSHOT_CHANGED"));
            assert!(exchange(vec![(200,expected.clone()),(200,json!([file("a")])),(200,changed)]).0.unwrap_err().starts_with("PROVIDER_SNAPSHOT_CHANGED"));
        }
    }
    #[test]
    fn files_pagination_limits_and_missing_patch_are_explicit(){
        let full=Value::Array((0..100).map(|i|file(&format!("file{i}"))).collect());
        assert_eq!(parse_files(full.clone(),1).unwrap().next_page,Some(2));
        let last=parse_files(full,30).unwrap();assert!(last.limit_reached);assert_eq!(last.next_page,None);
        let mut renamed=file("new");renamed["previous_filename"]=json!("old");renamed["status"]=json!("renamed");renamed.as_object_mut().unwrap().remove("patch");
        let result=parse_files(json!([renamed]),1).unwrap();assert_eq!(result.files[0].previous_path.as_deref(),Some("old"));assert!(result.files[0].patch.is_none());
        assert!(parse_files(json!([file("duplicate"),file("duplicate")]),1).is_err());
        for value in [json!({}),json!([{"filename":"a"}]),json!([file("")]),Value::Array((0..101).map(|i|file(&i.to_string())).collect())] {assert!(parse_files(value,1).is_err());}
        for (owner,page,head) in [("../evil",1,"a".repeat(40)),("o",0,"a".repeat(40)),("o",31,"a".repeat(40)),("o",1,"bad".into())] {assert!(validate_files(owner,"r",7,page,&head,&"b".repeat(40)).is_err());}
    }
    #[test]
    fn file_read_errors_do_not_leak_bodies_or_retry(){
        for (status,code) in [(401,"PROVIDER_AUTH_REQUIRED"),(403,"PROVIDER_ACCESS_OR_RATE_LIMIT"),(302,"PROVIDER_READ_FAILED"),(500,"PROVIDER_READ_FAILED")] {
            let (result,requests)=exchange(vec![(status,json!({"message":"SECRET_TOKEN"}))]);
            let error=result.unwrap_err();assert_eq!(error,code);assert!(!error.contains("SECRET_TOKEN"));assert_eq!(requests.len(),1);
        }
    }
}
