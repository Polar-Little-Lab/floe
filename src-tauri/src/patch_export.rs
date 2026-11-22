use std::io::Write;
use std::path::{Component,Path};
pub fn validate(text:&str,truncated:bool)->Result<(),String>{
    if truncated{return Err("INCOMPLETE_PATCH: truncated previews cannot be exported".into());}
    if !text.starts_with("diff --git ")||text.contains('\0')||text.len()>5*1024*1024{return Err("INVALID_PATCH: expected a Git diff up to 5 MiB".into());}
    Ok(())
}
pub fn save(path:&Path,text:&str,truncated:bool)->Result<String,String>{
    validate(text,truncated)?;
    if !path.is_absolute()||!path.extension().is_some_and(|value|value.eq_ignore_ascii_case("patch")){return Err("INVALID_PATCH_PATH: choose a new .patch file".into());}
    let parent=path.parent().ok_or("INVALID_PATCH_PATH")?.canonicalize().map_err(|e|e.to_string())?;
    let destination=parent.join(path.file_name().ok_or("INVALID_PATCH_PATH")?);
    if destination.components().any(|part|matches!(part,Component::Normal(name) if name.eq_ignore_ascii_case(".git"))){return Err("INVALID_PATCH_PATH: Git metadata is not an export destination".into());}
    let mut temporary=tempfile::NamedTempFile::new_in(&parent).map_err(|e|e.to_string())?;
    temporary.write_all(text.as_bytes()).map_err(|e|e.to_string())?;
    temporary.as_file().sync_all().map_err(|e|e.to_string())?;
    temporary.persist_noclobber(&destination).map_err(|e|if e.error.kind()==std::io::ErrorKind::AlreadyExists{"PATCH_EXISTS: choose a new filename; existing files are not replaced".into()}else{e.error.to_string()})?;
    Ok(destination.to_string_lossy().into_owned())
}
#[cfg(test)]
mod tests{
 use super::*;
 #[test] fn exports_exact_diff_without_replacing_existing_files(){
  let temp=tempfile::tempdir().unwrap();let path=temp.path().join("change.patch");
  let text="diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
  save(&path,text,false).unwrap();assert_eq!(std::fs::read_to_string(&path).unwrap(),text);
  assert!(save(&path,"diff --git changed",false).is_err());assert_eq!(std::fs::read_to_string(&path).unwrap(),text);
  assert!(save(&temp.path().join("partial.patch"),text,true).is_err());
  assert!(save(&temp.path().join("config"),text,false).is_err());
  std::fs::create_dir(temp.path().join(".git")).unwrap();assert!(save(&temp.path().join(".git/change.patch"),text,false).is_err());
 }
}
