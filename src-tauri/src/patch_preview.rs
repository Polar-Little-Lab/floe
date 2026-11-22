use std::{fs::File,io::Read,path::Path};
use serde::Serialize;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Entry {pub id:String,pub path:String,pub previous_path:Option<String>,pub status:String,pub text:String}
#[derive(Serialize)]
pub struct Preview {pub name:String,pub bytes:usize,pub text:String,pub files:Vec<Entry>}
pub fn parse(name:String,text:String)->Result<Preview,String>{
    crate::patch_export::validate(&text,false)?;
    if text.lines().count()>10000{return Err("PATCH_LIMIT: preview supports at most 10000 lines".into());}
    if text.lines().any(|line|line=="GIT binary patch"){return Err("UNSUPPORTED_PATCH: encoded binary payloads are not supported".into());}
    let diff=parse_diff(&text)?;
    if diff.deltas().len()>500{return Err("PATCH_LIMIT: preview supports at most 500 changed files".into());}
    let sections:Vec<_>=text.split("\ndiff --git ").enumerate().map(|(index,part)|if index==0{part.to_owned()}else{format!("diff --git {part}")}).collect();
    if sections.len()!=diff.deltas().len(){return Err("UNSUPPORTED_PATCH: ambiguous file boundaries".into());}
    let mut files=Vec::new();
    for (index,delta) in diff.deltas().enumerate(){
        let old=delta.old_file().path().and_then(Path::to_str).map(str::to_owned);
        let path=delta.new_file().path().and_then(Path::to_str).map(str::to_owned).or_else(||old.clone()).ok_or("UNSUPPORTED_PATCH_PATH")?;
        let status=match delta.status(){git2::Delta::Added=>"added",git2::Delta::Deleted=>"deleted",git2::Delta::Renamed=>"renamed",git2::Delta::Copied=>"copied",git2::Delta::Typechange=>"typechange",_=>"modified"};
        let mut patch=sections[index].clone();if index+1<sections.len(){patch.push('\n');}
        files.push(Entry{id:index.to_string(),previous_path:old.filter(|value|value!=&path),path,status:status.into(),text:patch});
    }
    Ok(Preview{name,bytes:text.len(),text,files})
}
pub fn read(path:&Path)->Result<Preview,String>{
    let metadata=std::fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !metadata.is_file()||metadata.len()>5*1024*1024{return Err("INVALID_PATCH_FILE: select a regular file up to 5 MiB".into());}
    let file=File::open(path).map_err(|e|e.to_string())?;
    if !file.metadata().map_err(|e|e.to_string())?.is_file(){return Err("INVALID_PATCH_FILE".into());}
    let mut bytes=Vec::new();file.take(5*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    let text=String::from_utf8(bytes).map_err(|_|"UNSUPPORTED_PATCH_ENCODING: UTF-8 only")?;
    parse(path.file_name().and_then(|name|name.to_str()).unwrap_or("Patch").into(),text)
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn parses_and_preserves_two_file_patches_without_following_paths(){
  let first="diff --git a/one b/one\n--- a/one\n+++ b/one\n@@ -1 +1 @@\n-old\n+new\n";
  let second="diff --git a/two b/two\n--- a/two\n+++ b/two\n@@ -1 +1 @@\n-before\n+after\n";
  let input=format!("{first}{second}");let preview=parse("test.patch".into(),input.clone()).unwrap();
  assert_eq!(preview.text,input);assert_eq!(preview.files.len(),2);assert_eq!(preview.files[0].text,first);assert_eq!(preview.files[1].text,second);
  assert_eq!(preview.files[0].path,"one");assert!(parse("bad".into(),"diff --git malformed".into()).is_err());
  assert!(parse("binary".into(),"diff --git a/x b/x\nGIT binary patch\n".into()).is_err());
 }
}

// Git emits empty-file patches without ---/+++ lines; libgit2 requires them.
// Normalize only canonical empty-blob add/delete headers, retaining displayed input.
pub fn parse_diff(text:&str)->Result<git2::Diff<'static>,String>{
    let mut normalized=String::with_capacity(text.len());
    for (number,section) in text.split("\ndiff --git ").enumerate(){
        let section=if number==0{section.to_owned()}else{format!("diff --git {section}")};
        let lines:Vec<_>=section.lines().collect();
        let section_start=normalized.len();
        normalized.push_str(&section);
        if !section.ends_with('\n'){normalized.push('\n');}
        if lines.len()!=3{continue;}
        let add=lines[1]=="new file mode 100644"||lines[1]=="new file mode 100755";
        let delete=lines[1]=="deleted file mode 100644"||lines[1]=="deleted file mode 100755";
        if !add&&!delete{continue;}
        let Some((old,new))=lines[2].strip_prefix("index ").and_then(|s|s.split_once(".."))else{continue;};
        let empty=|s:&str|s.len()>=7&&s.len()<=40&&"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".starts_with(s);
        let zero=|s:&str|s.len()>=7&&s.len()<=40&&s.bytes().all(|b|b==b'0');
        if !(if add{zero(old)&&empty(new)}else{empty(old)&&zero(new)}){continue;}
        let header=lines[0].strip_prefix("diff --git ").ok_or("INVALID_PATCH")?;
        let unquote=|s:&str|s.strip_prefix('"').and_then(|s|s.strip_suffix('"')).unwrap_or(s).to_owned();
        let candidates:Vec<_>=header.match_indices(' ').filter_map(|(at,_)|{let(a,b)=header.split_at(at);let b=&b[1..];let left=unquote(a);let right=unquote(b);match(left.strip_prefix("a/"),right.strip_prefix("b/")){(Some(x),Some(y))if !x.is_empty()&&x==y=>Some((a,b)),_=>None}}).collect();
        if candidates.len()!=1{return Err("UNSUPPORTED_PATCH: ambiguous empty-file path".into());}
        let (a,b)=candidates[0];
        // Quoted names keep spaces unambiguous in both Git and libgit2 headers.
        let quote=|s:&str|if s.starts_with('"'){s.to_owned()}else{format!("\"{}\"",s.replace('\\',"\\\\").replace('"',"\\\"").replace('\t',"\\t").replace('\r',"\\r"))};
        let a=quote(a);let b=quote(b);
        normalized.truncate(section_start);
        normalized.push_str(&format!("diff --git {a} {b}\n{}\n{}\n--- {}\n+++ {}\n",lines[1],lines[2],if add{"/dev/null"}else{&a},if delete{"/dev/null"}else{&b}));
    }
    git2::Diff::from_buffer(normalized.as_bytes()).map_err(|error|format!("INVALID_PATCH: {}",error.message()))
}

#[cfg(test)]
mod empty_patch_tests {
 use super::*;
 #[test]
 fn canonical_empty_headers_preserve_original_and_reject_unproven_empty_blobs(){
  for header in ["a/empty b/empty","a/empty name b/empty name", "\"a/tab\\tname\" \"b/tab\\tname\""]{
   let patch=format!("diff --git {header}\nnew file mode 100644\nindex 0000000..e69de29\n");
   let parsed=parse("empty.patch".into(),patch.clone()).unwrap();assert_eq!(parsed.text,patch);assert_eq!(parsed.files[0].text,patch);assert_eq!(parsed.files[0].status,"added");assert_eq!(parsed.files.len(),1);
   for oid in ["e69", "1234567", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5390"]{assert!(parse("bad.patch".into(),patch.replace("e69de29",oid)).is_err());}
  }
  let patch="diff --git a/empty b/empty\nnew file mode 100644\nindex 0000000..e69de29\ndiff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
  let parsed=parse("mixed.patch".into(),patch.into()).unwrap();assert_eq!(parsed.files.len(),2);assert_eq!(parsed.files.iter().map(|file|file.text.as_str()).collect::<String>(),patch);
 }
}
