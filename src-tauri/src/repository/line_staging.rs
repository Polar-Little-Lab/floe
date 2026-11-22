//! Compose selected +/- lines from a freshly computed forward diff. Never edit working files.
use super::*;
use std::collections::BTreeSet;
#[derive(Debug, Clone, Deserialize)]
pub struct Selection { pub hunk: usize, pub line: usize }

#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct SelectableLine {pub hunk:usize,pub line:usize,pub old_line:Option<u32>,pub new_line:Option<u32>}
pub(super) fn describe(diff:&git2::Diff<'_>)->Result<Vec<SelectableLine>>{
 if hunks::describe(diff)?.is_empty(){return Ok(Vec::new());}
 let patch=git2::Patch::from_diff(diff,0).map_err(err)?.ok_or("INVALID_DIFF")?;
 let mut lines=Vec::new();
 for h in 0..patch.num_hunks(){for n in 0..patch.num_lines_in_hunk(h).map_err(err)?{
  let line=patch.line_in_hunk(h,n).map_err(err)?;
  if matches!(line.origin(),'+'|'-'){lines.push(SelectableLine{hunk:h,line:n,old_line:line.old_lineno(),new_line:line.new_lineno()});}
 }}
 if lines.len()>10000{return Ok(Vec::new());}Ok(lines)
}

fn compose(base: &[u8], patch: &git2::Patch<'_>, selected: &[Selection], unstage: bool) -> Result<Vec<u8>> {
    if selected.is_empty() || selected.len() > 10000 { return Err("INVALID_LINE_SELECTION".into()); }
    let mut wanted = BTreeSet::new();
    for item in selected {
        if !wanted.insert((item.hunk,item.line)) { return Err("DUPLICATE_LINE_SELECTION".into()); }
        let line = patch.line_in_hunk(item.hunk,item.line).map_err(err)?;
        if !matches!(line.origin(), '+'|'-') { return Err("INVALID_LINE_SELECTION: context or metadata".into()); }
    }
    let source:Vec<&[u8]>=base.split_inclusive(|b| *b==b'\n').collect();
    let mut cursor=0usize; let mut output=Vec::new();
    for h in 0..patch.num_hunks() {
        let (header,count)=patch.hunk(h).map_err(err)?;
        let (start,lines)=if unstage {(header.new_start(),header.new_lines())}else{(header.old_start(),header.old_lines())};
        // Empty ranges name the line BEFORE insertion; nonempty ranges are one-based.
        let offset=if lines==0 {start as usize}else{start.saturating_sub(1) as usize};
        if offset<cursor || offset>source.len() {return Err("INVALID_LINE_RANGE".into());}
        for line in &source[cursor..offset] {output.extend_from_slice(line);} cursor=offset;
        for n in 0..count {
            let line=patch.line_in_hunk(h,n).map_err(err)?; let origin=line.origin();
            if !matches!(origin,' '|'-'|'+') {continue;} // EOF markers carry no file bytes.
            let chosen=wanted.contains(&(h,n));
            let consumes=origin==' ' || origin==if unstage {'+'}else{'-'};
            if consumes {
                if source.get(cursor).copied()!=Some(line.content()) {return Err("LINE_PREIMAGE_MISMATCH".into());}
                cursor+=1;
            }
            let keep=origin==' ' || if consumes {!chosen}else{chosen};
            if keep {output.extend_from_slice(line.content());}
            if output.len()>FILE_LIMIT as usize {return Err("LIMIT: staged file".into());}
        }
    }
    for line in &source[cursor..] {output.extend_from_slice(line);}
    if output.len()>FILE_LIMIT as usize {return Err("LIMIT: staged file".into());}
    Ok(output)
}

pub(super) fn execute(session:&mut Session,repo:&Repository,revision:&str,id:&str,selected:&[Selection],unstage:bool,lock:&mut IndexLock)->Result<()> {
    let branch=external_merge::head_lock_target(repo)?;
    let _refs=checkout::locks(repo,&branch,&branch)?;session.fresh(revision)?;
    let path=session.files.get(id).ok_or("INVALID_FILE_ID")?.clone();
    let mut index=git2::Index::open(&lock.path).map_err(err)?;
    if index.has_conflicts(){return Err("CONFLICT".into());}
    let mut opts=DiffOptions::new();opts.pathspec(&path).disable_pathspec_match(true).include_untracked(true).show_untracked_content(true).recurse_untracked_dirs(true).max_size(FILE_LIMIT as i64);
    let head=match repo.head(){Ok(h)=>Some(h.peel_to_tree().map_err(err)?),Err(e) if e.code()==git2::ErrorCode::UnbornBranch=>None,Err(e)=>return Err(err(e))};
    let diff=if unstage {repo.diff_tree_to_index(head.as_ref(),Some(&index),Some(&mut opts))}else{repo.diff_index_to_workdir(Some(&index),Some(&mut opts))}.map_err(err)?;
    if hunks::describe(&diff)?.is_empty() || render_diff(&diff)?.truncated {return Err("UNSUPPORTED_LINE_DIFF".into());}
    let patch=git2::Patch::from_diff(&diff,0).map_err(err)?.ok_or("INVALID_DIFF")?;
    let delta=diff.get_delta(0).ok_or("INVALID_DIFF")?;
    let entry=index.get_path(&path,0);
    let base=match &entry {Some(e)=>repo.find_blob(e.id).map_err(err)?.content().to_vec(),None=>Vec::new()};
    let result=compose(&base,&patch,selected,unstage)?;
    // A partial deletion keeps an empty tracked file unless all changed lines were selected.
    let mut changed=0;
    for h in 0..patch.num_hunks(){for n in 0..patch.num_lines_in_hunk(h).map_err(err)? {if matches!(patch.line_in_hunk(h,n).map_err(err)?.origin(),'+'|'-'){changed+=1;}}}
    let removing=selected.len()==changed && result.is_empty() && if unstage {matches!(delta.status(),git2::Delta::Added)}else{matches!(delta.status(),git2::Delta::Deleted)};
    if removing {index.remove_path(&path).map_err(err)?;} else {
        let mode=entry.as_ref().map(|e|e.mode).unwrap_or_else(||if unstage {delta.old_file().mode().into()}else{delta.new_file().mode().into()});
        let blob=repo.blob(&result).map_err(err)?;
        let entry=git2::IndexEntry{ctime:git2::IndexTime::new(0,0),mtime:git2::IndexTime::new(0,0),dev:0,ino:0,mode,uid:0,gid:0,file_size:result.len() as u32,id:blob,flags:0,flags_extended:0,path:path.to_str().ok_or("INVALID_PATH")?.as_bytes().to_vec()};
        index.add(&entry).map_err(err)?;
    }
    index.write().map_err(err)?;session.fresh(revision)?;
    external_merge::publish(session,revision,lock,&branch,None)
}

#[cfg(test)]
mod tests {
 use super::*;
 fn patch<'a>(before:&'a [u8],after:&'a [u8])->git2::Patch<'a>{git2::Patch::from_buffers(before,Some(Path::new("file")),after,Some(Path::new("file")),None).unwrap()}
 fn select(p:&git2::Patch<'_>,origin:char)->Vec<Selection>{(0..p.num_hunks()).flat_map(|h|(0..p.num_lines_in_hunk(h).unwrap()).filter_map(move |n|(p.line_in_hunk(h,n).unwrap().origin()==origin).then_some(Selection{hunk:h,line:n}))).collect()}
 #[test]fn literal_bytes_forward_reverse_and_invalid_selections(){
  for (before,after) in [(b"old\n".as_slice(),b"new\n".as_slice()),(b"old\r\n",b"new\r\n"),(b"old",b"new"),("이전\n".as_bytes(),"변경\n".as_bytes())] {
   let p=patch(before,after);let removed=select(&p,'-');let added=select(&p,'+');
   assert_eq!(compose(before,&p,&removed,false).unwrap(),b"");
   assert_eq!(compose(before,&p,&added,false).unwrap(),[before,after].concat());
   assert_eq!(compose(after,&p,&added,true).unwrap(),b"");
   assert_eq!(compose(after,&p,&removed,true).unwrap(),[before,after].concat());
   let all=[removed,added].concat();assert_eq!(compose(before,&p,&all,false).unwrap(),after);assert_eq!(compose(after,&p,&all,true).unwrap(),before);
   assert!(compose(before,&p,&[],false).is_err());assert!(compose(before,&p,&[all[0].clone(),all[0].clone()],false).is_err());assert!(compose(before,&p,&[Selection{hunk:99,line:0}],false).is_err());assert!(compose(b"wrong\n",&p,&all,false).is_err());
  }
  let p=patch(b"same\nold\n",b"same\nnew\n");assert!(compose(b"same\nold\n",&p,&[Selection{hunk:0,line:0}],false).is_err());
 }
 #[test]fn public_stage_unstage_preserves_files_head_and_other_staged_entries(){
  let d=tempfile::tempdir().unwrap();let repo=Repository::init(d.path()).unwrap();
  fs::write(d.path().join("file"),b"old\n").unwrap();fs::write(d.path().join("other"),b"keep\n").unwrap();
  let mut index=repo.index().unwrap();for name in ["file","other"]{index.add_path(Path::new(name)).unwrap();}index.write().unwrap();
  let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();let head=repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
  fs::write(d.path().join("file"),b"new\n").unwrap();let mut session=Session::new(d.path().into()).unwrap();let s=session.snapshot().unwrap();
  let p=patch(b"old\n",b"new\n");let lines=select(&p,'+');
  let preview=session.diff(&s.revision,&file_id("file"),false).unwrap();
  assert_eq!(preview.selectable_lines.len(),2);
  assert_eq!((preview.selectable_lines[1].hunk,preview.selectable_lines[1].line,preview.selectable_lines[1].old_line,preview.selectable_lines[1].new_line),(0,lines[0].line,None,Some(1)));

  let s2=session.mutate(&s.revision,Action::StageLines{file_id:file_id("file"),lines:lines.clone(),unstage:false}).unwrap();
  let index=open(d.path()).unwrap().index().unwrap();assert_eq!(repo.find_blob(index.get_path(Path::new("file"),0).unwrap().id).unwrap().content(),b"old\nnew\n");
  assert_eq!(repo.find_blob(index.get_path(Path::new("other"),0).unwrap().id).unwrap().content(),b"keep\n");assert_eq!(repo.head().unwrap().target(),Some(head));assert_eq!(fs::read(d.path().join("file")).unwrap(),b"new\n");
  assert!(session.mutate(&s.revision,Action::StageLines{file_id:file_id("file"),lines,unstage:false}).is_err());
  let p=patch(b"old\n",b"old\nnew\n");session.mutate(&s2.revision,Action::StageLines{file_id:file_id("file"),lines:select(&p,'+'),unstage:true}).unwrap();
  let index=open(d.path()).unwrap().index().unwrap();assert_eq!(repo.find_blob(index.get_path(Path::new("file"),0).unwrap().id).unwrap().content(),b"old\n");assert_eq!(fs::read(d.path().join("file")).unwrap(),b"new\n");
 }
 #[test]fn new_and_deleted_files_support_partial_selection_and_reverse(){
  for deleted in [false,true]{
   let d=tempfile::tempdir().unwrap();let repo=Repository::init(d.path()).unwrap();fs::write(d.path().join("other"),"keep\n").unwrap();
   let mut index=repo.index().unwrap();index.add_path(Path::new("other")).unwrap();
   if deleted{fs::write(d.path().join("file"),"one\ntwo\n").unwrap();index.add_path(Path::new("file")).unwrap();}
   index.write().unwrap();let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
   if deleted{fs::remove_file(d.path().join("file")).unwrap();}else{fs::write(d.path().join("file"),"one\ntwo\n").unwrap();}
   let mut session=Session::new(d.path().into()).unwrap();let a=session.snapshot().unwrap();
   let a=session.mutate(&a.revision,Action::StageLines{file_id:file_id("file"),lines:vec![Selection{hunk:0,line:0}],unstage:false}).unwrap();
   let index=open(d.path()).unwrap().index().unwrap();let entry=index.get_path(Path::new("file"),0).unwrap();assert_eq!(repo.find_blob(entry.id).unwrap().content(),if deleted{b"two\n"}else{b"one\n"});
   let a=session.mutate(&a.revision,Action::StageLines{file_id:file_id("file"),lines:vec![Selection{hunk:0,line:0}],unstage:true}).unwrap();
   let index=open(d.path()).unwrap().index().unwrap();if deleted{let e=index.get_path(Path::new("file"),0).unwrap();assert_eq!(e.mode,0o100644);assert_eq!(repo.find_blob(e.id).unwrap().content(),b"one\ntwo\n");}else{assert!(index.get_path(Path::new("file"),0).is_none());}
   if deleted{
    let a=session.mutate(&a.revision,Action::StageLines{file_id:file_id("file"),lines:vec![Selection{hunk:0,line:0},Selection{hunk:0,line:1}],unstage:false}).unwrap();
    assert!(open(d.path()).unwrap().index().unwrap().get_path(Path::new("file"),0).is_none());
    session.mutate(&a.revision,Action::StageLines{file_id:file_id("file"),lines:vec![Selection{hunk:0,line:0}],unstage:true}).unwrap();
    let index=open(d.path()).unwrap().index().unwrap();let e=index.get_path(Path::new("file"),0).unwrap();assert_eq!(e.mode,0o100644);assert_eq!(repo.find_blob(e.id).unwrap().content(),b"one\n");assert!(!d.path().join("file").exists());
   }
  }
 }

 fn multi_fixture()->(tempfile::TempDir,String,String){
  let d=tempfile::tempdir().unwrap();let repo=Repository::init(d.path()).unwrap();
  let base=(0..40).map(|n|format!("line {n}\n")).collect::<String>();let changed=base.replace("line 2\n","first change\n").replace("line 35\n","last change\n");
  fs::write(d.path().join("file"),&base).unwrap();fs::write(d.path().join("other"),"keep\n").unwrap();
  let mut index=repo.index().unwrap();for path in ["file","other"]{index.add_path(Path::new(path)).unwrap();}index.write().unwrap();
  let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
  fs::write(d.path().join("file"),&changed).unwrap();(d,base,changed)
 }
 fn git(root:&Path,args:&[&str])->String{let output=std::process::Command::new("git").arg("-C").arg(root).args(args).env("GIT_CONFIG_NOSYSTEM","1").output().unwrap();assert!(output.status.success(),"{:?}",output);String::from_utf8(output.stdout).unwrap()}
 #[test]fn disjoint_hunks_match_git_index_and_reverse_without_worktree_changes(){
  let (d,base,changed)=multi_fixture();let (oracle,_,_)=multi_fixture();
  let expected=base.replace("line 2\n","line 2\nfirst change\n").replace("line 35\n","");
  fs::write(oracle.path().join("file"),&expected).unwrap();git(oracle.path(),&["add","--","file"]);let expected_tree=git(oracle.path(),&["write-tree"]);
  let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();let head=git(d.path(),&["rev-parse","HEAD"]);
  let preview=session.diff(&before.revision,&file_id("file"),false).unwrap();assert_eq!(preview.hunks.len(),2);
  let lines=preview.selectable_lines.iter().filter(|l|l.new_line==Some(3)||l.old_line==Some(36)).map(|l|Selection{hunk:l.hunk,line:l.line}).collect::<Vec<_>>();assert_eq!(lines.len(),2);
  session.mutate(&before.revision,Action::StageLines{file_id:file_id("file"),lines,unstage:false}).unwrap();assert_eq!(git(d.path(),&["write-tree"]),expected_tree);assert_eq!(git(d.path(),&["show",":file"]),expected);assert_eq!(fs::read_to_string(d.path().join("file")).unwrap(),changed);assert_eq!(git(d.path(),&["rev-parse","HEAD"]),head);
  // Git write-tree may update the index cache-tree extension; acquire a fresh reviewed snapshot.
  let after=session.snapshot().unwrap();
  let preview=session.diff(&after.revision,&file_id("file"),true).unwrap();let lines=preview.selectable_lines.iter().map(|l|Selection{hunk:l.hunk,line:l.line}).collect();session.mutate(&after.revision,Action::StageLines{file_id:file_id("file"),lines,unstage:true}).unwrap();assert_eq!(git(d.path(),&["show",":file"]),base);assert_eq!(fs::read_to_string(d.path().join("file")).unwrap(),changed);
 }
 #[test]fn foreign_lock_and_binary_diff_are_refused_without_index_changes(){
  let (d,_,_)=multi_fixture();let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();let bytes=fs::read(d.path().join(".git/index")).unwrap();
  fs::write(d.path().join(".git/index.lock"),"foreign owner").unwrap();
  let action=||Action::StageLines{file_id:file_id("file"),lines:vec![Selection{hunk:0,line:0}],unstage:false};
  assert!(session.mutate(&before.revision,action()).unwrap_err().contains("EXTERNAL_LOCK"));assert_eq!(fs::read_to_string(d.path().join(".git/index.lock")).unwrap(),"foreign owner");fs::remove_file(d.path().join(".git/index.lock")).unwrap();
  fs::write(d.path().join("file"),b"binary\0payload").unwrap();let before=session.snapshot().unwrap();assert!(session.diff(&before.revision,&file_id("file"),false).unwrap().selectable_lines.is_empty());assert!(session.mutate(&before.revision,action()).is_err());assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),bytes);
 }
 #[test]fn line_publication_actual_kills_recover_even_after_second_recovery_kill(){
  for step in 0..=1 {for again in [None,Some(2),Some(3)]{
   let (d,_,_)=multi_fixture();let index=fs::read(d.path().join(".git/index")).unwrap();let head=fs::read(d.path().join(".git/HEAD")).unwrap();
   super::super::tests::kill_draft(d.path(),"lines",&format!("index:{step}"));
   fs::write(d.path().join("file"),"later user edit\n").unwrap();
   if let Some(n)=again{super::super::tests::kill_draft(d.path(),"recover",&format!("index:{n}"));}
   let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();session.mutate(&before.revision,Action::RecoverCheckout).unwrap();
   assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(),head);assert_eq!(fs::read_to_string(d.path().join("file")).unwrap(),"later user edit\n");assert!(!d.path().join(".git/index.lock").exists());
  }}
 }

 #[test]
 fn reviewed_lines_refuse_external_changes_then_retry_preserving_other_entries() {
  for unstage in [false,true] { for external_index in [false,true] {
   let (d,base,_) = multi_fixture();
   if unstage { git(d.path(), &["add","--","file"]); }
   let mut session=Session::new(d.path().into()).unwrap();
   let reviewed=session.snapshot().unwrap();
   let preview=session.diff(&reviewed.revision,&file_id("file"),unstage).unwrap();
   let selected=preview.selectable_lines.iter().map(|l|Selection{hunk:l.hunk,line:l.line}).collect();
   if external_index {
    fs::write(d.path().join("other"),"externally staged work\n").unwrap();
    git(d.path(), &["add","--","other"]);
   } else {
    fs::write(d.path().join("file"),"later external file edit\n").unwrap();
   }
   let index=fs::read(d.path().join(".git/index")).unwrap();
   let head=git(d.path(), &["rev-parse","HEAD"]);
   let file=fs::read(d.path().join("file")).unwrap();
   let other=fs::read(d.path().join("other")).unwrap();
   let other_staged=git(d.path(), &["show",":other"]);
   let failure=session.mutate(&reviewed.revision,Action::StageLines{file_id:file_id("file"),lines:selected,unstage}).unwrap_err();
   assert!(failure.starts_with("STALE_STATE"),"{unstage}/{external_index}: {failure}");
   assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
   assert_eq!(fs::read(d.path().join("file")).unwrap(),file);
   assert_eq!(fs::read(d.path().join("other")).unwrap(),other);
   assert_eq!(git(d.path(), &["rev-parse","HEAD"]),head);
   assert!(!d.path().join(".git/index.lock").exists());

   // Re-review the real current patch; never replay old line coordinates.
   let fresh=session.snapshot().unwrap();
   let preview=session.diff(&fresh.revision,&file_id("file"),unstage).unwrap();
   let selected=preview.selectable_lines.iter().map(|l|Selection{hunk:l.hunk,line:l.line}).collect();
   session.mutate(&fresh.revision,Action::StageLines{file_id:file_id("file"),lines:selected,unstage}).unwrap();
   assert_eq!(git(d.path(), &["show",":file"]).as_bytes(),if unstage {base.as_bytes()}else{file.as_slice()});
   assert_eq!(git(d.path(), &["show",":other"]),other_staged);
   assert_eq!(fs::read(d.path().join("file")).unwrap(),file);
   assert_eq!(fs::read(d.path().join("other")).unwrap(),other);
   assert_eq!(git(d.path(), &["rev-parse","HEAD"]),head);
   assert!(!d.path().join(".git/index.lock").exists());
  }}
 }

}
