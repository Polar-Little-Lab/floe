//! Direct transaction tests; session/UI writes stay gated until recovery routing is migrated.
use super::*;
fn preserved(main:&Path,other:&Path)->Vec<Vec<u8>> {
    let a=layout::Layout::read(main).unwrap();let b=layout::Layout::read(other).unwrap();
    [a.private.join("index"),a.private.join("HEAD"),b.private.join("index"),b.private.join("HEAD"),main.join("hello.txt"),other.join("hello.txt")].iter().map(|p|fs::read(p).unwrap()).collect()
}
fn stage(session:&mut Session,fail:Option<usize>)->Result<()> {
    let _lease=lease::OperationLease::acquire(&session.root)?;
    let snap=session.snapshot()?;let repo=open(&session.root)?;
    let branch=external_merge::head_lock_target(&repo)?;let _refs=checkout::locks(&repo,&branch,&branch)?;
    let lock=IndexLock::acquire(&session.root)?;
    let mut index=git2::Index::open(&lock.path).map_err(err)?;
    let bytes=fs::read(session.root.join("hello.txt")).map_err(err)?;
    let mut entry=index.get_path(Path::new("hello.txt"),0).ok_or("NO_FIXTURE_ENTRY")?;
    entry.id=repo.blob(&bytes).map_err(err)?;entry.file_size=bytes.len() as u32;index.add(&entry).map_err(err)?;index.write().map_err(err)?;
    external_merge::publish(session,&snap.revision,&lock,&branch,fail)
}
#[test]
fn linked_stage_interruption_recovers_private_index_and_preserves_later_work(){
    for step in [0,1] {
        let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);
        let index=layout::private_file(&first,"index").unwrap();let original=fs::read(&index).unwrap();
        fs::write(first.join("hello.txt"),"staged change\n").unwrap();let mut session=Session::new(first.clone()).unwrap();
        assert_eq!(stage(&mut session,Some(step)).unwrap_err(),"INJECTED_INTERRUPTION");
        assert!(external_merge::pending(&first));assert!(!external_merge::pending(&main));
        fs::write(first.join("hello.txt"),"later edit\n").unwrap();drop(session);
        let lease=lease::OperationLease::acquire(&first).unwrap();journal_locks::release(&first,&lease).unwrap();
        let mut lock=IndexLock::acquire(&first).unwrap();external_merge::recover(&first,&mut lock).unwrap();external_merge::recover(&first,&mut lock).unwrap();external_merge::finish(&first).unwrap();drop(lock);
        assert_eq!(fs::read(index).unwrap(),original);assert_eq!(fs::read(first.join("hello.txt")).unwrap(),b"later edit\n");assert_eq!(preserved(&main,&other),before);assert!(!external_merge::pending(&first));
    }
}
#[test]
fn linked_stage_then_commit_and_recovery_reuse_exact_commit(){
    let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);
    let main_tip=open(&main).unwrap().head().unwrap().target();let other_tip=open(&other).unwrap().head().unwrap().target();
    fs::write(first.join("hello.txt"),"linked staged\n").unwrap();let mut session=Session::new(first.clone()).unwrap();stage(&mut session,None).unwrap();external_merge::finish(&first).unwrap();
    let snap=session.snapshot().unwrap();assert!(snap.changes.iter().any(|c|c.staged));let repo=open(&first).unwrap();let parent=repo.head().unwrap().target().unwrap();
    let lease=lease::OperationLease::acquire(&first).unwrap();let lock=IndexLock::acquire(&first).unwrap();
    commit_write::execute(&mut session,&repo,&snap.revision,"linked commit","Fixture","fixture@example.invalid",false).unwrap();drop(lock);drop(repo);drop(session);
    assert!(commit_write::pending(&first));assert!(!commit_write::pending(&main));journal_locks::release(&first,&lease).unwrap();
    let lock=IndexLock::acquire(&first).unwrap();let repo=open(&first).unwrap();let oid=repo.head().unwrap().target().unwrap();assert_ne!(oid,parent);
    commit_write::recover(&first,&repo).unwrap();commit_write::recover(&first,&repo).unwrap();commit_write::finish(&first).unwrap();drop(lock);
    let commit=repo.find_commit(oid).unwrap();assert_eq!(commit.parent_id(0).unwrap(),parent);assert_eq!(repo.head().unwrap().target(),Some(oid));
    let tree=commit.tree().unwrap();let blob=repo.find_blob(tree.get_path(Path::new("hello.txt")).unwrap().id()).unwrap();assert_eq!(blob.content(),b"linked staged\n");
    assert_eq!(open(&main).unwrap().head().unwrap().target(),main_tip);assert_eq!(open(&other).unwrap().head().unwrap().target(),other_tip);assert_eq!(preserved(&main,&other),before);
}
#[test]
fn linked_recovery_rejects_copied_stage_record_before_index_change(){
    let (_temp,_main,first,other)=layout::write_isolation_tests::fixture();fs::write(first.join("hello.txt"),"change\n").unwrap();let mut session=Session::new(first.clone()).unwrap();stage(&mut session,Some(0)).unwrap_err();
    let source=layout::private_file(&first,"branchdesk-merge-index").unwrap();let destination=layout::private_file(&other,"branchdesk-merge-index").unwrap();fs::create_dir(&destination).unwrap();
    for file in ["record.json","index.before"] {fs::copy(source.join(file),destination.join(file)).unwrap();}
    let index=layout::private_file(&other,"index").unwrap();let before=fs::read(&index).unwrap();let mut lock=IndexLock::acquire(&other).unwrap();assert!(external_merge::recover(&other,&mut lock).is_err());assert_eq!(fs::read(index).unwrap(),before);
}
#[test]
fn linked_index_publication_rejects_foreign_lock_and_journal(){
    let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);let lock=IndexLock::acquire(&first).unwrap();
    let journal=layout::private_file(&first,"branchdesk-merge-index").unwrap();fs::create_dir(&journal).unwrap();
    assert!(checkout::publish_index(&other,&journal,&lock).is_err());let wrong=layout::private_file(&other,"branchdesk-merge-index").unwrap();fs::create_dir(&wrong).unwrap();assert!(checkout::publish_index(&first,&wrong,&lock).is_err());assert_eq!(preserved(&main,&other),before);
}

#[test]
fn linked_transaction_crash_worker(){
    let Some(root)=std::env::var_os("BRANCHDESK_LINKED_TX_ROOT") else{return;};let root=PathBuf::from(root);
    let mut session=Session::new(root.clone()).unwrap();
    let snap=session.snapshot().unwrap();
    let action=match std::env::var("BRANCHDESK_LINKED_TX_KIND").unwrap().as_str() {
        "index"=>Action::StageAll,
        "hunk"=>Action::StageHunk{file_id:file_id("hello.txt"),hunk:0,unstage:false},
        "lines"=>{let diff=session.diff(&snap.revision,&file_id("hello.txt"),false).unwrap();Action::StageLines{file_id:file_id("hello.txt"),lines:diff.selectable_lines.iter().map(|l|line_staging::Selection{hunk:l.hunk,line:l.line}).collect(),unstage:false}},
        _=>Action::Commit{message:"crash linked commit".into(),author:"Fixture".into(),email:"fixture@example.invalid".into()}
    };
    session.mutate(&snap.revision,action).unwrap();
    panic!("worker missed checkpoint");
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
#[test]
fn linked_actual_process_kill_recovers_stage_and_commit_boundaries(){
    for kind in ["index","hunk","lines","commit"] {for step in [0,1] {
        let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);let main_tip=open(&main).unwrap().head().unwrap().target();
        let index=layout::private_file(&first,"index").unwrap();let original=fs::read(&index).unwrap();fs::write(first.join("hello.txt"),"child staged\n").unwrap();
        if kind=="commit" {let mut session=Session::new(first.clone()).unwrap();stage(&mut session,None).unwrap();external_merge::finish(&first).unwrap();}
        let ready=layout::private_file(&first,"branchdesk-draft-test.ready").unwrap();
        let mut child=ChildGuard(std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","repository::linked_transactions::linked_transaction_crash_worker","--nocapture"])
            .env("BRANCHDESK_LINKED_TX_ROOT",&first).env("BRANCHDESK_LINKED_TX_KIND",kind).env("BRANCHDESK_DRAFT_ROOT",&first).env("BRANCHDESK_DRAFT_CHECKPOINT",format!("{}:{step}",if kind=="commit" {"commit"}else{"index"}))
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit()).spawn().unwrap());
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(15);
        while !ready.exists(){assert!(child.0.try_wait().unwrap().is_none(),"child exited before checkpoint");assert!(std::time::Instant::now()<deadline,"child checkpoint timeout");std::thread::sleep(std::time::Duration::from_millis(10));}
        assert!(lease::OperationLease::acquire(&main).is_err());child.0.kill().unwrap();child.0.wait().unwrap();
        fs::write(first.join("hello.txt"),"later work after crash\n").unwrap();
        let expected=if kind=="commit" {let record:serde_json::Value=serde_json::from_slice(&fs::read(layout::private_file(&first,"branchdesk-commit").unwrap().join("record.json")).unwrap()).unwrap();Some(record["new"].as_str().unwrap().to_owned())} else {None};
        let mut session=Session::new(first.clone()).unwrap();let snapshot=session.snapshot().unwrap();assert!(snapshot.recovery_pending);assert!(!snapshot.linked_local_writes);assert!(session.mutate(&snapshot.revision,Action::StageAll).err().unwrap().contains("RECOVERY_REQUIRED"));
        let recovered=session.mutate(&snapshot.revision,Action::RecoverCheckout).unwrap();assert!(!recovered.recovery_pending);assert!(recovered.linked_local_writes);
        if let Some(oid)=expected{assert_eq!(open(&first).unwrap().head().unwrap().target().unwrap().to_string(),oid);} else {assert_eq!(fs::read(&index).unwrap(),original);}
        assert_eq!(fs::read(first.join("hello.txt")).unwrap(),b"later work after crash\n");assert_eq!(preserved(&main,&other),before);assert_eq!(open(&main).unwrap().head().unwrap().target(),main_tip);assert!(!layout::private_file(&first,"index.lock").unwrap().exists());assert!(!layout::private_file(&first,"HEAD.lock").unwrap().exists());
    }}
}

#[test]
#[cfg(unix)]
fn index_publication_accepts_same_directory_alias_but_not_other_checkout(){
    let (_temp,main,_first,_other)=layout::write_isolation_tests::fixture();let alias=main.parent().unwrap().join("main-alias");std::os::unix::fs::symlink(&main,&alias).unwrap();
    let before=fs::read(main.join(".git/index")).unwrap();let lock=IndexLock::acquire(&alias).unwrap();let journal=main.join(".git/branchdesk-merge-index");fs::create_dir(&journal).unwrap();checkout::publish_index(&main,&journal,&lock).unwrap();assert_eq!(fs::read(main.join(".git/index")).unwrap(),before);
}

#[test]
fn public_linked_stage_unstage_selection_commit_preserves_other_worktrees(){
    let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);let main_tip=open(&main).unwrap().head().unwrap().target();
    fs::write(first.join("hello.txt"),"public stage\n").unwrap();let mut session=Session::new(first.clone()).unwrap();let initial=session.snapshot().unwrap();assert!(initial.linked_local_writes);
    let staged=session.mutate(&initial.revision,Action::StageAll).unwrap();assert!(staged.changes[0].staged);assert!(session.mutate(&initial.revision,Action::UnstageAll).is_err());
    let unstaged=session.mutate(&staged.revision,Action::UnstageFiles{file_ids:vec![staged.changes[0].id.clone()]}).unwrap();assert!(!unstaged.changes[0].staged);
    let staged=session.mutate(&unstaged.revision,Action::StageFiles{file_ids:vec![unstaged.changes[0].id.clone()]}).unwrap();
    let committed=session.mutate(&staged.revision,Action::Commit{message:"public linked".into(),author:"Fixture".into(),email:"fixture@example.invalid".into()}).unwrap();assert!(committed.changes.is_empty());assert_eq!(open(&main).unwrap().head().unwrap().target(),main_tip);assert_eq!(preserved(&main,&other),before);
    for action in [Action::CreateBranch{name:"blocked".into()},Action::AmendCommit{message:"blocked".into(),author:"Fixture".into(),email:"fixture@example.invalid".into()}] {assert_eq!(session.mutate(&committed.revision,action).err().unwrap(),"UNSUPPORTED_LINKED_OPERATION");}
}
#[test]
fn linked_policy_blocks_attributes_hooks_private_config_and_duplicate_checkout(){
    for kind in 0..4 {
        let (_temp,main,first,other)=layout::write_isolation_tests::fixture();fs::write(first.join("hello.txt"),"change\n").unwrap();
        match kind {0=>fs::write(first.join(".gitattributes"),"*.txt filter=custom\n").unwrap(),1=>fs::write(main.join(".git/hooks/pre-commit"),"exit 1\n").unwrap(),2=>fs::write(layout::private_file(&first,"config.worktree").unwrap(),"[core]\n").unwrap(),_=>{let head=fs::read(layout::private_file(&first,"HEAD").unwrap()).unwrap();fs::write(layout::private_file(&other,"HEAD").unwrap(),head).unwrap();}}
        let index=layout::private_file(&first,"index").unwrap();let before=fs::read(&index).unwrap();let mut session=Session::new(first).unwrap();let snap=session.snapshot().unwrap();assert!(!snap.linked_local_writes);assert!(session.mutate(&snap.revision,Action::StageAll).err().unwrap().contains("READ_ONLY"));assert_eq!(fs::read(index).unwrap(),before);
    }
}

#[test]
fn public_linked_partial_staging_roundtrip_and_stale_refusal(){
    for use_lines in [false,true] {
        let (_temp,main,first,other)=layout::write_isolation_tests::fixture();let before=preserved(&main,&other);
        let base=(0..30).map(|n|format!("line {n}\n")).collect::<String>();
        fs::write(first.join("hello.txt"),&base).unwrap();let mut session=Session::new(first.clone()).unwrap();
        let snap=session.snapshot().unwrap();let snap=session.mutate(&snap.revision,Action::StageAll).unwrap();
        session.mutate(&snap.revision,Action::Commit{message:"multihunk base".into(),author:"Fixture".into(),email:"fixture@example.invalid".into()}).unwrap();
        let changed=base.replace("line 2\n","changed 2\n").replace("line 25\n","changed 25\n");fs::write(first.join("hello.txt"),&changed).unwrap();
        let snap=session.snapshot().unwrap();let id=file_id("hello.txt");let preview=session.diff(&snap.revision,&id,false).unwrap();assert_eq!(preview.hunks.len(),2);
        let action=|preview:&DiffResult,unstage| if use_lines {Action::StageLines{file_id:id.clone(),lines:preview.selectable_lines.iter().filter(|l|l.hunk==0).map(|l|line_staging::Selection{hunk:l.hunk,line:l.line}).collect(),unstage}} else {Action::StageHunk{file_id:id.clone(),hunk:0,unstage}};
        let index=layout::private_file(&first,"index").unwrap();let original=fs::read(&index).unwrap();
        assert!(session.mutate(&snap.revision,Action::StageHunk{file_id:id.clone(),hunk:999,unstage:false}).is_err());assert_eq!(fs::read(&index).unwrap(),original);
        let after=session.mutate(&snap.revision,action(&preview,false)).unwrap();
        let repo=open(&first).unwrap();let blob=|repo:&Repository|repo.find_blob(git2::Index::open(&index).unwrap().get_path(Path::new("hello.txt"),0).unwrap().id).unwrap().content().to_vec();
        assert_eq!(blob(&repo),base.replace("line 2\n","changed 2\n").as_bytes());
        let staged_index=fs::read(&index).unwrap();assert!(session.mutate(&snap.revision,action(&preview,false)).is_err());assert_eq!(fs::read(&index).unwrap(),staged_index);
        let reverse=session.diff(&after.revision,&id,true).unwrap();session.mutate(&after.revision,action(&reverse,true)).unwrap();assert_eq!(blob(&repo),base.as_bytes());
        assert_eq!(fs::read(first.join("hello.txt")).unwrap(),changed.as_bytes());assert_eq!(preserved(&main,&other),before);
    }
}
