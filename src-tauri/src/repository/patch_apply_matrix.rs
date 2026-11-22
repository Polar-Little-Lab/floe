use super::*;
use std::process::{Command,Stdio};
use std::io::Write;
fn fixture(initial:Option<&[u8]>)->tempfile::TempDir{
 let temp=tempfile::tempdir().unwrap();let mut opts=git2::RepositoryInitOptions::new();opts.external_template(false).initial_head("main");
 let repo=Repository::init_opts(temp.path(),&opts).unwrap();
 if let Some(bytes)=initial{
  fs::write(temp.path().join("file"),bytes).unwrap();fs::write(temp.path().join("unrelated"),b"keep staged\n").unwrap();
  let mut index=repo.index().unwrap();index.add_path(Path::new("file")).unwrap();index.add_path(Path::new("unrelated")).unwrap();index.write().unwrap();
  let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();repo.commit(Some("HEAD"),&sig,&sig,"Initial",&tree,&[]).unwrap();
 }
 temp
}
fn entries(root:&Path)->Vec<(Vec<u8>,u32,Vec<u8>)>{let repo=open(root).unwrap();repo.index().unwrap().iter().map(|entry|(entry.path,entry.mode,repo.find_blob(entry.id).unwrap().content().to_vec())).collect()}
fn apply(root:&Path,patch:&str)->Result<Snapshot>{let mut s=Session::new(root.into())?;let a=s.snapshot()?;s.mutate(&a.revision,Action::ApplyPatchIndex{text:patch.into()})}
fn git_apply(root:&Path,patch:&str){let mut child=Command::new("git").args(["-C",root.to_str().unwrap(),"-c","core.hooksPath=/dev/null","apply","--cached","--whitespace=nowarn","-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();child.stdin.take().unwrap().write_all(patch.as_bytes()).unwrap();let result=child.wait_with_output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));}
const CHANGE:&str="diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
const ADD:&str="diff --git a/added b/added\nnew file mode 100644\n--- /dev/null\n+++ b/added\n@@ -0,0 +1 @@\n+new\n";
#[test]
fn git_cli_parity_for_rename_mode_empty_file_and_missing_newline(){
 let cases:[(&[u8],&str);6]=[
 (b"old\n","diff --git a/file b/renamed\nsimilarity index 100%\nrename from file\nrename to renamed\n"),
 (b"old\n","diff --git a/file b/file\nold mode 100644\nnew mode 100755\n"),
 (b"old\n","diff --git a/empty b/empty\nnew file mode 100644\nindex 0000000..e69de29\n"),
 (b"","diff --git a/file b/file\ndeleted file mode 100644\nindex e69de29..0000000\n"),
 (b"old\n","diff --git a/empty name b/empty name\nnew file mode 100644\nindex 0000000..e69de29\n"),
 (b"old","diff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n")];
 for(initial,patch)in cases{let ours=fixture(Some(initial));let oracle=fixture(Some(initial));let head=fs::read(ours.path().join(".git/HEAD")).unwrap();
  git_apply(oracle.path(),patch);apply(ours.path(),patch).unwrap_or_else(|error|panic!("{error}: {patch}"));assert_eq!(entries(ours.path()),entries(oracle.path()),"{patch}");assert_eq!(fs::read(ours.path().join("file")).unwrap(),initial);assert_eq!(fs::read(ours.path().join(".git/HEAD")).unwrap(),head);assert!(!ours.path().join("renamed").exists());assert!(!ours.path().join("empty").exists());
 }
}
#[test]
fn unborn_application_matches_git_and_can_commit_without_worktree_file(){
 let ours=fixture(None);let oracle=fixture(None);git_apply(oracle.path(),ADD);apply(ours.path(),ADD).unwrap();assert_eq!(entries(ours.path()),entries(oracle.path()));assert!(!ours.path().join("added").exists());
 let mut session=Session::new(ours.path().into()).unwrap();let before=session.snapshot().unwrap();let after=session.mutate(&before.revision,Action::Commit{message:"Patched first commit".into(),author:"Fixture".into(),email:"fixture@example.invalid".into()}).unwrap();assert!(after.head_oid.is_some());assert!(!ours.path().join("added").exists());
}
#[test]
fn failed_second_file_and_invalid_paths_modes_preserve_index_and_worktree(){
 let temp=fixture(Some(b"old\n"));let index=fs::read(temp.path().join(".git/index")).unwrap();let head=open(temp.path()).unwrap().head().unwrap().target();
 let bad_second=format!("{ADD}{}",CHANGE.replace("-old","-mismatch"));
 for patch in [bad_second,CHANGE.replace("a/file","a/.git/config").replace("b/file","b/.git/config"),"diff --git a/link b/link\nnew file mode 120000\n--- /dev/null\n+++ b/link\n@@ -0,0 +1 @@\n+outside\n".into(),"not a patch".into()]{assert!(apply(temp.path(),&patch).is_err(),"{patch}");assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(),index);assert_eq!(open(temp.path()).unwrap().head().unwrap().target(),head);assert_eq!(fs::read(temp.path().join("file")).unwrap(),b"old\n");assert!(!temp.path().join("added").exists());assert!(!temp.path().join(".git/index.lock").exists());}
}
#[test]
fn foreign_index_lock_is_never_removed(){let temp=fixture(Some(b"old\n"));let before=entries(temp.path());fs::write(temp.path().join(".git/index.lock"),"foreign owner").unwrap();assert!(apply(temp.path(),CHANGE).is_err());assert_eq!(entries(temp.path()),before);assert_eq!(fs::read_to_string(temp.path().join(".git/index.lock")).unwrap(),"foreign owner");}
#[test]
fn actual_kill_and_recovery_rekill_restore_index_and_preserve_later_work(){
 for(unborn,step,recovery_step)in [(false,0,None),(false,1,None),(true,0,None),(true,1,None),(false,1,Some(2)),(false,1,Some(3))]{
  let temp=fixture(if unborn{None}else{Some(b"old\n")});let before=fs::read(temp.path().join(".git/index")).ok();let head=fs::read(temp.path().join(".git/HEAD")).unwrap();fs::write(temp.path().join("test-input.patch"),if unborn{ADD}else{CHANGE}).unwrap();
  super::super::tests::kill_draft(temp.path(),"patchIndex",&format!("index:{step}"));
  fs::write(temp.path().join("file"),b"later working edits\n").unwrap();
  if let Some(step)=recovery_step{super::super::tests::kill_draft(temp.path(),"recover",&format!("index:{step}"));}
  let mut session=Session::new(temp.path().into()).unwrap();let snapshot=session.snapshot().unwrap();assert!(snapshot.recovery_pending);let recovered=session.mutate(&snapshot.revision,Action::RecoverCheckout).unwrap();assert!(!recovered.recovery_pending);
  assert_eq!(fs::read(temp.path().join(".git/index")).ok(),before);assert_eq!(fs::read(temp.path().join(".git/HEAD")).unwrap(),head);assert_eq!(fs::read(temp.path().join("file")).unwrap(),b"later working edits\n");assert!(!temp.path().join(".git/index.lock").exists());
 }
}
