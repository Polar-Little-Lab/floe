use super::*;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Detail {pub name:String,pub object_oid:String,pub target_oid:String,pub commit_oid:Option<String>,pub annotated:bool,pub message:String,pub tagger:Option<String>,pub email:Option<String>,pub time:Option<i64>,pub truncated:bool}
impl Session {
    pub fn tag_detail(&mut self,revision:&str,name:&str)->Result<Detail>{
        self.fresh(revision)?;
        let reference_name=format!("refs/tags/{name}");
        if name.is_empty()||name.len()>1024||!git2::Reference::is_valid_name(&reference_name){return Err("INVALID_TAG".into());}
        let repo=open(&self.root)?;
        let reference=repo.find_reference(&reference_name).map_err(err)?;
        let oid=reference.target().ok_or("UNSUPPORTED_SYMBOLIC_TAG")?;
        let (size,kind)=repo.odb().map_err(err)?.read_header(oid).map_err(err)?;
        if kind==git2::ObjectType::Tag&&size>1024*1024{return Err("TAG_MESSAGE_LIMIT".into());}
        let mut detail=Detail{name:name.into(),object_oid:oid.to_string(),target_oid:oid.to_string(),commit_oid:None,annotated:false,message:String::new(),tagger:None,email:None,time:None,truncated:false};
        if kind==git2::ObjectType::Tag {
            let tag=repo.find_tag(oid).map_err(err)?;
            detail.annotated=true;detail.target_oid=tag.target_id().to_string();
            let message=tag.message_bytes().unwrap_or_default();detail.truncated=message.len()>65536;
            detail.message=String::from_utf8_lossy(&message[..message.len().min(65536)]).into_owned();
            if let Some(tagger)=tag.tagger(){detail.tagger=tagger.name().ok().map(|s|s.chars().take(256).collect());detail.email=tagger.email().ok().map(|s|s.chars().take(256).collect());detail.time=Some(tagger.when().seconds());};
        }
        detail.commit_oid=reference.peel_to_commit().ok().map(|commit|commit.id().to_string());
        self.fresh(revision)?;
        Ok(detail)
    }
}
