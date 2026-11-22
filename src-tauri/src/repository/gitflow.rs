use super::*;

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Settings {
    pub main: Option<String>,
    pub develop: Option<String>,
    pub feature: Option<String>,
    pub release: Option<String>,
    pub hotfix: Option<String>,
    pub version_tag: Option<String>,
    pub missing_branches: Vec<String>,
    pub warnings: Vec<String>,
}
fn value(entries: &[(String,String)], key: &str) -> Result<Option<String>> {
    match entries.iter().rev().find(|(name,_)|name==key).map(|(_,value)|value) {
        Some(value) if value.len() <= 1024 && !value.chars().any(char::is_control) => Ok(Some(value.clone())),
        Some(_) => Err("INVALID_GITFLOW_CONFIG".into()),
        None => Ok(None),
    }
}

impl Session {
    pub fn gitflow_settings(&mut self, revision: &str) -> Result<Settings> {
        self.fresh(revision)?;
        let repo = open(&self.root)?;
        let layout=layout::Layout::read(&self.root)?;
        let config_path=layout.common.join("config");
        let before=bounded(&config_path,65536)?;
        let config=remote_remove::read_only_entries(&before)?;
        let main = value(&config,"gitflow.branch.master")?;
        let develop = value(&config,"gitflow.branch.develop")?;
        let mut missing_branches = Vec::new();
        for name in [&main,&develop].into_iter().flatten() {
            let reference=format!("refs/heads/{name}");
            if !git2::Reference::is_valid_name(&reference) || repo.find_reference(&reference).is_err() {
                missing_branches.push(name.clone());
            }
        }
        let mut result = Settings {main,develop,feature:value(&config,"gitflow.prefix.feature")?,release:value(&config,"gitflow.prefix.release")?,hotfix:value(&config,"gitflow.prefix.hotfix")?,version_tag:value(&config,"gitflow.prefix.versiontag")?,missing_branches,warnings:Vec::new()};
        if let (Some(main),Some(develop),Some(feature),Some(release),Some(hotfix),Some(version_tag))=(&result.main,&result.develop,&result.feature,&result.release,&result.hotfix,&result.version_tag) {
            let configuration=Configuration{main:main.clone(),develop:develop.clone(),feature:feature.clone(),release:release.clone(),hotfix:hotfix.clone(),version_tag:version_tag.clone()};
            if let Err(reason)=configuration.validate(){result.warnings.push(reason);}
        } else {result.warnings.push("GITFLOW_CONFIGURATION_INCOMPLETE".into());}
        self.fresh(revision)?;
        if bounded(&config_path,65536)?!=before{return Err("GITFLOW_CONFIG_CHANGED: reload settings".into());}
        Ok(result)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct Configuration {
 pub main:String, pub develop:String, pub feature:String, pub release:String, pub hotfix:String, pub version_tag:String,
}
impl Configuration {
 pub fn validate(&self)->Result<()> {
  let valid=|name:&str| !name.is_empty() && name.len()<=200 && !name.starts_with('-') && git2::Reference::is_valid_name(&format!("refs/heads/{name}"));
  if !valid(&self.main)||!valid(&self.develop)||self.main==self.develop {return Err("INVALID_GITFLOW_BRANCHES".into());}
  let prefixes=[&self.feature,&self.release,&self.hotfix];
  for prefix in prefixes {if prefix.len()>180||!valid(&format!("{prefix}example")){return Err("INVALID_GITFLOW_PREFIX".into());}}
  if self.feature==self.release||self.feature==self.hotfix||self.release==self.hotfix{return Err("GITFLOW_PREFIXES_MUST_DIFFER".into());}
  if self.version_tag.len()>180||!git2::Reference::is_valid_name(&format!("refs/tags/{}1.0.0",self.version_tag)){return Err("INVALID_GITFLOW_TAG_PREFIX".into());}
  Ok(())
 }
 pub(super) fn pairs(&self)->[(&str,&str);6]{[("gitflow.branch.master",&self.main),("gitflow.branch.develop",&self.develop),("gitflow.prefix.feature",&self.feature),("gitflow.prefix.release",&self.release),("gitflow.prefix.hotfix",&self.hotfix),("gitflow.prefix.versiontag",&self.version_tag)]}
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all="camelCase")]
pub enum FlowKind {Feature,Release,Hotfix}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct FinishTarget {pub branch:String,pub oid:String,pub already_contains_source:bool}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct FinishPreview {pub revision:String,pub kind:FlowKind,pub source:String,pub source_oid:String,pub targets:Vec<FinishTarget>,pub tag:Option<String>,pub tag_exists:bool,pub reusable_tag:bool,pub tag_object_oid:Option<String>}
impl Session {
 pub fn gitflow_finish_preview(&mut self,revision:&str,kind:FlowKind,source:&str)->Result<FinishPreview>{
  let settings=self.gitflow_settings(revision)?;
  if !settings.warnings.is_empty()||!settings.missing_branches.is_empty(){return Err("GITFLOW_CONFIGURATION_INVALID".into());}
  let main=settings.main.as_deref().ok_or("GITFLOW_CONFIGURATION_INCOMPLETE")?;
  let develop=settings.develop.as_deref().ok_or("GITFLOW_CONFIGURATION_INCOMPLETE")?;
  let prefix=match kind{FlowKind::Feature=>settings.feature.as_deref(),FlowKind::Release=>settings.release.as_deref(),FlowKind::Hotfix=>settings.hotfix.as_deref()}.ok_or("GITFLOW_CONFIGURATION_INCOMPLETE")?;
  let suffix=source.strip_prefix(prefix).filter(|name|!name.is_empty()).ok_or("GITFLOW_SOURCE_PREFIX_MISMATCH")?;
  if source==main||source==develop||source.len()>200||!git2::Reference::is_valid_name(&format!("refs/heads/{source}")){return Err("INVALID_GITFLOW_SOURCE".into());}
  let repo=open(&self.root)?;
  let commit=repo.find_reference(&format!("refs/heads/{source}")).and_then(|reference|reference.peel_to_commit()).map_err(|_|"GITFLOW_SOURCE_MISSING")?;
  let names=match kind{FlowKind::Feature=>vec![develop],FlowKind::Release|FlowKind::Hotfix=>vec![main,develop]};
  let mut targets=Vec::new();
  for name in names {
   let target=repo.find_reference(&format!("refs/heads/{name}")).and_then(|reference|reference.peel_to_commit()).map_err(|_|"GITFLOW_TARGET_MISSING")?;
   let contained=target.id()==commit.id()||repo.graph_descendant_of(target.id(),commit.id()).map_err(err)?;
   targets.push(FinishTarget{branch:name.into(),oid:target.id().to_string(),already_contains_source:contained});
  }
  let tag=match kind{FlowKind::Feature=>None,_=>Some(format!("{}{}",settings.version_tag.as_deref().ok_or("GITFLOW_CONFIGURATION_INCOMPLETE")?,suffix))};
  let tag_exists=if let Some(tag)=&tag {
   let reference=format!("refs/tags/{tag}");if !git2::Reference::is_valid_name(&reference){return Err("INVALID_GITFLOW_TAG".into());}
   match repo.find_reference(&reference){Ok(_)=>true,Err(error) if error.code()==git2::ErrorCode::NotFound=>false,Err(error)=>return Err(err(error))}
  }else{false};
  // Reuse only an annotated tag at the current main tip containing this source.
  // A tag pointing elsewhere is never replaced or silently accepted.
  let mut reusable_tag=false;
  let mut tag_object_oid=None;
  if tag_exists {
   if let Some(name)=&tag {
    let reference=repo.find_reference(&format!("refs/tags/{name}")).map_err(err)?;
    tag_object_oid=reference.target().map(|oid|oid.to_string());
    if let Ok(annotation)=reference.peel_to_tag() {
     if let Ok(tagged)=annotation.target().and_then(|object|object.peel_to_commit()) {
      reusable_tag=targets.first().map(|target|target.already_contains_source&&target.oid==tagged.id().to_string()).unwrap_or(false);
     }
    }
   }
  }
  self.fresh(revision)?;
  Ok(FinishPreview{revision:revision.into(),kind,source:source.into(),source_oid:commit.id().to_string(),targets,tag,tag_exists,reusable_tag,tag_object_oid})
 }
}
