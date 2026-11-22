use super::*;
#[derive(Clone,Copy,Debug,Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Kind {Labels,Assignees,Reviewers,Branches}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Page {pub items:Vec<String>,pub next_page:Option<u32>,pub limit_reached:bool,pub default_branch:Option<String>}
pub fn validate(owner:&str,repo:&str,page:u32)->Result<(),String>{
 endpoint(owner,repo)?;
 if !(1..=30).contains(&page){return Err("INVALID_METADATA_PAGE".into());}Ok(())
}
pub fn read(owner:&str,repo:&str,kind:Kind,page:u32,credential:&crate::credential_vault::Credential)->Result<Page,String>{
 validate(owner,repo,page)?;
 let (path,field)=match kind{Kind::Labels=>("labels","name"),Kind::Assignees=>("assignees","login"),Kind::Reviewers=>("collaborators","login"),Kind::Branches=>("branches","name")};
 let http=client(credential)?;
 let repository_endpoint=endpoint(owner,repo)?;
 let default_branch=if matches!(kind,Kind::Branches)&&page==1{
  let repository=send(&http,&repository_endpoint,reqwest::Method::GET,None,false)?;
  if !repository["full_name"].as_str().is_some_and(|name|name.eq_ignore_ascii_case(&format!("{owner}/{repo}"))){return Err("INVALID_REPOSITORY_OPTIONS".into());}
  match &repository["default_branch"]{
   Value::Null=>None,
   Value::String(name) if branch(name)=>Some(name.clone()),
   _=>return Err("INVALID_REPOSITORY_DEFAULT_BRANCH".into()),
  }
 }else{None};
 let affiliation=if matches!(kind,Kind::Reviewers){"&affiliation=all"}else{""};
 let value=send(&http,&format!("{}/{path}?per_page=100&page={page}{affiliation}",endpoint(owner,repo)?),reqwest::Method::GET,None,false)?;
 let items=if matches!(kind,Kind::Branches){
  let entries=value.as_array().filter(|items|items.len()<=100).ok_or("INVALID_BRANCH_OPTIONS")?;
  let names=entries.iter().map(|entry|entry["name"].as_str().filter(|name|branch(name)).map(str::to_owned).ok_or("INVALID_BRANCH_OPTIONS".to_string())).collect::<Result<Vec<_>,_>>()?;
  if names.iter().collect::<std::collections::BTreeSet<_>>().len()!=names.len(){return Err("INVALID_BRANCH_OPTIONS".into());}names
 }else{detail_names(&value,field)?.ok_or("INVALID_METADATA_OPTIONS")?};
 match kind{Kind::Branches=>{},Kind::Labels=>{label_set(&items)?;},Kind::Assignees|Kind::Reviewers=>{account_set(&items)?;}}
 let full=items.len()==100;
 Ok(Page{items,next_page:if full&&page<30{Some(page+1)}else{None},limit_reached:full&&page==30,default_branch})
}
