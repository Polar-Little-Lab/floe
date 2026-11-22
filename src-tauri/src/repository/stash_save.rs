//! Build all stash objects and a durable backup before clearing worktree changes.
use super::checkout::{digest, entries, inspect_tree, matches, replace, save, sync_dir};
use super::*;
const DIR: &str = ".git/branchdesk-stash-save";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    #[serde(default)]
    keep_index: bool,
    branch: String,
    head: String,
    stash: String,
    checkpoint: String,
    before_tree: String,
    before_index: String,
    after_index: String,
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(DIR).exists()
}
fn archive(root: &Path) -> Result<()> {
    fs::rename(
        root.join(DIR),
        root.join(".git")
            .join(format!("branchdesk-save-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    sync_dir(&root.join(".git"))
}
fn capture(repo: &Repository, root: &Path, index: &mut git2::Index, path: &Path) -> Result<()> {
    let full = rel(root, path)?;
    if !full.exists() {
        if index.get_path(path, 0).is_some() {
            index.remove_path(path).map_err(err)?;
        }
        return Ok(());
    }
    let data = bounded(&full, FILE_LIMIT)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&full).map_err(err)?.permissions().mode() & 0o111 != 0 {
            0o100755
        } else {
            0o100644
        }
    };
    #[cfg(not(unix))]
    let mode = index.get_path(path, 0).map(|e| e.mode).unwrap_or(0o100644);
    index
        .add(&IndexEntry {
            ctime: IndexTime::new(0, 0),
            mtime: IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size: data.len() as u32,
            id: repo.blob(&data).map_err(err)?,
            flags: 0,
            flags_extended: 0,
            path: path.to_str().ok_or("UNSUPPORTED_PATH")?.as_bytes().to_vec(),
        })
        .map_err(err)
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    message: &str,
    include_untracked: bool,
    lock: &mut IndexLock,
    fail: Option<usize>,
) -> Result<()> {
    execute_options(session,repo,revision,message,SaveOptions{include_untracked,keep_index:false},lock,fail)
}
#[derive(Clone,Copy)]
pub(super) struct SaveOptions {pub include_untracked:bool,pub keep_index:bool}
pub(super) fn execute_options(session:&mut Session,repo:&Repository,revision:&str,message:&str,options:SaveOptions,lock:&mut IndexLock,fail:Option<usize>)->Result<()> {
    let SaveOptions{include_untracked,keep_index}=options;
    if message.len() > 1000 || message.contains(['\0', '\n', '\r']) {
        return Err("INVALID_STASH_MESSAGE".into());
    }
    let branch = repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        .ok_or("UNSUPPORTED_DETACHED_STASH")?
        .to_string();
    let mut tx = checkout::locks(repo, &branch, &branch)?;
    tx.lock_ref("refs/stash").map_err(err)?;
    let snapshot = session.fresh(revision)?;
    if snapshot.stashes.len() >= MAX_FILES {
        return Err("LIMIT: stash count".into());
    }
    let head = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let base = head.tree().map_err(err)?;
    let mut original = git2::Index::open(&lock.path).map_err(err)?;
    let staged = repo
        .find_tree(original.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    inspect_tree(repo, &base)?;
    inspect_tree(repo, &staged)?;
    let mut work = git2::Index::new().map_err(err)?;
    work.read_tree(&staged).map_err(err)?;
    let mut extra = git2::Index::new().map_err(err)?;
    for change in &snapshot.changes {
        let path = session.files.get(&change.id).ok_or("INVALID_FILE_ID")?;
        let tracked = original.get_path(path, 0).is_some() || base.get_path(path).is_ok();
        if tracked {
            capture(repo, &session.root, &mut work, path)?;
        } else if include_untracked {
            capture(repo, &session.root, &mut extra, path)?;
        }
    }
    let work_tree = repo
        .find_tree(work.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    let extra_tree = repo
        .find_tree(extra.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    inspect_tree(repo, &work_tree)?;
    inspect_tree(repo, &extra_tree)?;
    if staged.id() == base.id() && work_tree.id() == base.id() && extra.is_empty() {
        return Err("NOTHING_TO_STASH".into());
    }
    for entry in extra.iter() {
        work.add(&entry).map_err(err)?;
    }
    let before = repo
        .find_tree(work.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    let cleanup_target=if keep_index {&staged}else{&base};
    let planned = entries(repo, &before, cleanup_target)?;
    for entry in &planned {
        if !matches(&session.root, entry, false)? {
            return Err("STALE: worktree changed".into());
        }
    }
    session.fresh(revision)?;
    let signature = git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?;
    let index_commit = repo
        .commit(
            None,
            &signature,
            &signature,
            "index snapshot",
            &staged,
            &[&head],
        )
        .map_err(err)?;
    let index_commit = repo.find_commit(index_commit).map_err(err)?;
    let untracked_commit = if extra.is_empty() {
        None
    } else {
        Some(
            repo.find_commit(
                repo.commit(
                    None,
                    &signature,
                    &signature,
                    "untracked snapshot",
                    &extra_tree,
                    &[],
                )
                .map_err(err)?,
            )
            .map_err(err)?,
        )
    };
    let mut parents = vec![&head, &index_commit];
    if let Some(ref extra) = untracked_commit {
        parents.push(extra);
    }
    let label = format!(
        "On {}: {}",
        snapshot.branch,
        if message.trim().is_empty() {
            "WIP"
        } else {
            message
        }
    );
    let stash = repo
        .commit(None, &signature, &signature, &label, &work_tree, &parents)
        .map_err(err)?;
    let stash_commit = repo.find_commit(stash).map_err(err)?;
    // A hidden local ref pins every original byte, including the combined tree,
    // before cleanup. Retention is explicit future work; never silently prune it.
    let checkpoint = format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4());
    repo.commit(
        Some(&checkpoint),
        &signature,
        &signature,
        "stash save recovery checkpoint",
        &before,
        &[&stash_commit],
    )
    .map_err(err)?;
    let before_index = bounded(&session.root.join(".git/index"), 20 * 1024 * 1024)?;
    if !keep_index {
        original.read_tree(&base).map_err(err)?;
        original.write().map_err(err)?;
    }
    let after_index = bounded(&lock.path, 20 * 1024 * 1024)?;
    let record = Record {
        version: 1,
        keep_index,
        branch,
        head: head.id().to_string(),
        stash: stash.to_string(),
        checkpoint,
        before_tree: before.id().to_string(),
        before_index: digest(&before_index),
        after_index: digest(&after_index),
    };
    let dir = session.root.join(DIR);
    fs::create_dir(&dir).map_err(err)?;
    save(&dir.join("index.before"), &before_index)?;
    save(&dir.join("index.after"), &after_index)?;
    save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    sync_dir(&dir)?;
    sync_dir(&session.root.join(".git"))?;
    journal_locks::record(&session.root, &dir, Some(&record.branch), true, false)?;
    let inject = |step| -> Result<()> {
        journal_locks::checkpoint(&session.root, "save", step);
        if fail == Some(step) {
            Err("INJECTED_INTERRUPTION".into())
        } else {
            Ok(())
        }
    };
    inject(0)?;
    // Commit exactly one public reference. The checkpoint is already durable.
    repo.reference_ensure_log("refs/stash").map_err(err)?;
    tx.set_target("refs/stash", stash, Some(&signature), &label)
        .map_err(err)?;
    tx.commit().map_err(err)?;
    if repo.refname_to_id("refs/stash").map_err(err)? != stash
        || repo
            .reflog("refs/stash")
            .map_err(err)?
            .get(0)
            .map(|e| e.id_new())
            != Some(stash)
    {
        return Err("STASH_PUBLICATION_INCOMPLETE".into());
    }
    inject(1)?;
    // Reacquire HEAD/current branch locks after the publication transaction.
    let _head_lock = checkout::locks(repo, &record.branch, &record.branch)?;
    journal_locks::record(&session.root, &dir, Some(&record.branch), false, false)?;
    if repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        != Some(record.branch.as_str())
        || repo.head().map_err(err)?.target() != Some(head.id())
    {
        return Err("RECOVERY_CONFLICT: HEAD changed".into());
    }
    for (i, entry) in planned.iter().enumerate() {
        if !matches(&session.root, entry, false)? {
            return Err("RECOVERY_CONFLICT: worktree changed".into());
        }
        let blob = cleanup_target
            .get_path(Path::new(&entry.path))
            .ok()
            .map(|e| repo.find_blob(e.id()))
            .transpose()
            .map_err(err)?;
        replace(
            &session.root,
            &dir,
            entry,
            blob.as_ref().map(|b| b.content()),
            entry.after_mode,
        )?;
        inject(i + 2)?;
    }
    checkout::publish_index(&session.root, &dir, lock)?;
    inject(planned.len() + 2)?;
    for entry in &planned {
        if !matches(&session.root, entry, true)? {
            return Err("RECOVERY_CONFLICT: cleanup result changed".into());
        }
    }
    if digest(&bounded(
        &session.root.join(".git/index"),
        20 * 1024 * 1024,
    )?) != record.after_index
    {
        return Err("RECOVERY_CONFLICT: index cleanup changed".into());
    }
    archive(&session.root)
}
pub(super) fn recover(root: &Path, repo: &Repository, lock: &mut IndexLock) -> Result<()> {
    let dir = root.join(DIR);
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 4096)?).map_err(err)?;
    if record.version != 1
        || !record
            .checkpoint
            .starts_with("refs/branchdesk/checkpoints/")
    {
        return Err("INVALID_SAVE_JOURNAL".into());
    }
    let _tx = checkout::locks(repo, &record.branch, &record.branch)?;
    journal_locks::record(root, &dir, Some(&record.branch), false, false)?;
    if repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        != Some(record.branch.as_str())
        || repo.head().map_err(err)?.target().map(|x| x.to_string()) != Some(record.head.clone())
    {
        return Err("RECOVERY_CONFLICT: HEAD changed".into());
    }
    let checkpoint = repo
        .find_reference(&record.checkpoint)
        .map_err(err)?
        .peel_to_commit()
        .map_err(err)?;
    if checkpoint.tree_id().to_string() != record.before_tree
        || checkpoint.parent_count() != 1
        || checkpoint.parent_id(0).map_err(err)?.to_string() != record.stash
    {
        return Err("INVALID_SAVE_CHECKPOINT".into());
    }
    let stash = checkpoint.parent(0).map_err(err)?;
    if !(2..=3).contains(&stash.parent_count())
        || stash.parent_id(0).map_err(err)?.to_string() != record.head
    {
        return Err("INVALID_SAVE_CHECKPOINT".into());
    }
    let before = checkpoint.tree().map_err(err)?;
    // The stash index parent authorizes the keep-index cleanup target, including recovery.
    let after=if record.keep_index {stash.parent(1).map_err(err)?.tree().map_err(err)?}else{repo.head().map_err(err)?.peel_to_commit().map_err(err)?.tree().map_err(err)?};
    let planned = entries(repo, &before, &after)?;
    let before_bytes = bounded(&dir.join("index.before"), 20 * 1024 * 1024)?;
    let after_bytes = bounded(&dir.join("index.after"), 20 * 1024 * 1024)?;
    let mut before_index = git2::Index::open(&dir.join("index.before")).map_err(err)?;
    let mut after_index = git2::Index::open(&dir.join("index.after")).map_err(err)?;
    if digest(&before_bytes) != record.before_index
        || digest(&after_bytes) != record.after_index
        || before_index.has_conflicts()
        || before_index.write_tree_to(repo).map_err(err)? != stash.parent(1).map_err(err)?.tree_id()
        || after_index.has_conflicts()
        || after_index.write_tree_to(repo).map_err(err)? != after.id()
    {
        return Err("INVALID_SAVE_BACKUP".into());
    }
    let live = digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?);
    if live != record.before_index && live != record.after_index {
        return Err("RECOVERY_CONFLICT: index changed".into());
    }
    for entry in &planned {
        if !matches(root, entry, false)? && !matches(root, entry, true)? {
            return Err("RECOVERY_CONFLICT: worktree changed".into());
        }
    }
    journal_locks::checkpoint(root, "save-recover", 0);
    for entry in &planned {
        if !matches(root, entry, false)? {
            if !matches(root, entry, true)? {
                return Err("RECOVERY_CONFLICT: worktree changed".into());
            }
            let blob = before
                .get_path(Path::new(&entry.path))
                .ok()
                .map(|e| repo.find_blob(e.id()))
                .transpose()
                .map_err(err)?;
            replace(
                root,
                &dir,
                entry,
                blob.as_ref().map(|b| b.content()),
                entry.before_mode,
            )?;
        }
    }
    fs::write(&lock.path, &before_bytes).map_err(err)?;
    journal_locks::record(root, &dir, Some(&record.branch), false, false)?;
    journal_locks::checkpoint(root, "save-recover", 1);
    checkout::publish_index(root, &dir, lock)?;
    journal_locks::checkpoint(root, "save-recover", 2);
    archive(root)
}

#[cfg(test)]
mod keep_index_tests {
 use super::*;
 fn fixture()->tempfile::TempDir{
  let d=tempfile::tempdir().unwrap();let repo=Repository::init(d.path()).unwrap();let mut cfg=repo.config().unwrap();cfg.set_str("user.name","Fixture").unwrap();cfg.set_str("user.email","fixture@example.invalid").unwrap();
  fs::write(d.path().join("file"),"base\n").unwrap();fs::write(d.path().join("deleted"),"base delete\n").unwrap();
  let mut index=repo.index().unwrap();for n in ["file","deleted"]{index.add_path(Path::new(n)).unwrap();}index.write().unwrap();let tree=repo.find_tree(index.write_tree().unwrap()).unwrap();let sig=git2::Signature::now("Fixture","fixture@example.invalid").unwrap();repo.commit(Some("HEAD"),&sig,&sig,"base",&tree,&[]).unwrap();
  fs::write(d.path().join("file"),"staged\n").unwrap();fs::write(d.path().join("added"),"staged addition\n").unwrap();index.add_path(Path::new("file")).unwrap();index.add_path(Path::new("added")).unwrap();index.write().unwrap();
  fs::write(d.path().join("file"),"working\n").unwrap();fs::write(d.path().join("added"),"working addition\n").unwrap();fs::remove_file(d.path().join("deleted")).unwrap();fs::write(d.path().join("untracked"),"extra\n").unwrap();d
 }
 fn git(root:&Path,args:&[&str])->String{let o=std::process::Command::new("git").arg("--no-optional-locks").arg("-C").arg(root).args(args).output().unwrap();assert!(o.status.success(),"{:?}",o);String::from_utf8(o.stdout).unwrap()}
 #[test]fn keep_index_matches_git_and_preserves_original_index_bytes(){
  for include_untracked in [false,true]{
   let d=fixture();let oracle=fixture();let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();let bytes=fs::read(d.path().join(".git/index")).unwrap();let head=git(d.path(),&["rev-parse","HEAD"]);
   let after=session.mutate(&before.revision,Action::SaveStashKeepingIndex{message:"keep fixture".into(),include_untracked}).unwrap();assert_eq!(after.stashes.len(),1);assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),bytes);
   let mut args=vec!["stash","push","--keep-index","-m","keep fixture"];if include_untracked{args.push("--include-untracked");}git(oracle.path(),&args);
   assert_eq!(git(d.path(),&["status","--porcelain"]),git(oracle.path(),&["status","--porcelain"]));
   for n in ["file","added","deleted","untracked"]{assert_eq!(fs::read(d.path().join(n)).ok(),fs::read(oracle.path().join(n)).ok(),"{n}");}
   for reference in ["refs/stash^{tree}","refs/stash^2^{tree}"]{assert_eq!(git(d.path(),&["rev-parse",reference]),git(oracle.path(),&["rev-parse",reference]));}
   if include_untracked{assert_eq!(git(d.path(),&["rev-parse","refs/stash^3^{tree}"]),git(oracle.path(),&["rev-parse","refs/stash^3^{tree}"]));}
   assert_eq!(git(d.path(),&["rev-parse","HEAD"]),head);assert!(session.mutate(&before.revision,Action::SaveStashKeepingIndex{message:"stale".into(),include_untracked}).is_err());
  }
 }
 #[test]fn keep_index_returned_interruptions_restore_original_three_areas(){
  for step in 0..=6{
   let d=fixture();let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();let index=fs::read(d.path().join(".git/index")).unwrap();let status=git(d.path(),&["status","--porcelain"]);
   let repo=open(d.path()).unwrap();{let mut lock=IndexLock::acquire(d.path()).unwrap();assert!(execute_options(&mut session,&repo,&before.revision,"interrupted",SaveOptions{include_untracked:true,keep_index:true},&mut lock,Some(step)).unwrap_err().contains("INJECTED"));}
   let after=session.snapshot().unwrap();session.mutate(&after.revision,Action::RecoverCheckout).unwrap();assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);assert_eq!(git(d.path(),&["status","--porcelain"]),status);assert_eq!(fs::read(d.path().join("file")).unwrap(),b"working\n");assert_eq!(fs::read(d.path().join("added")).unwrap(),b"working addition\n");assert!(!d.path().join("deleted").exists());assert_eq!(fs::read(d.path().join("untracked")).unwrap(),b"extra\n");
  }
 }
 #[test]fn keep_index_actual_kill_and_recovery_rekill_restore_original_state(){
  let cases=(0..=6).map(|step|(step,None)).chain((0..=2).map(|recovery|(3,Some(recovery))));
  for (step,again) in cases{
   let d=fixture();let index=fs::read(d.path().join(".git/index")).unwrap();let status=git(d.path(),&["status","--porcelain"]);let head=git(d.path(),&["rev-parse","HEAD"]);
   super::super::tests::kill_keep_stash(d.path(),"save",step);
   if let Some(n)=again{super::super::tests::kill_keep_stash(d.path(),"save-recover",n);}
   let mut session=Session::new(d.path().into()).unwrap();let state=session.snapshot().unwrap();assert!(state.recovery_pending);let state=session.mutate(&state.revision,Action::RecoverCheckout).unwrap();assert!(!state.recovery_pending);
   assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);assert_eq!(git(d.path(),&["status","--porcelain"]),status);assert_eq!(git(d.path(),&["rev-parse","HEAD"]),head);
   assert_eq!(fs::read(d.path().join("file")).unwrap(),b"working\n");assert_eq!(fs::read(d.path().join("added")).unwrap(),b"working addition\n");assert!(!d.path().join("deleted").exists());assert_eq!(fs::read(d.path().join("untracked")).unwrap(),b"extra\n");assert!(!d.path().join(".git/index.lock").exists());
  }
 }
 #[test]fn keep_index_legacy_record_compatibility_and_mode_tamper_refusal(){
  for keep in [false,true]{
   let d=fixture();let mut session=Session::new(d.path().into()).unwrap();let before=session.snapshot().unwrap();let repo=open(d.path()).unwrap();let bytes=fs::read(d.path().join(".git/index")).unwrap();
   {let mut lock=IndexLock::acquire(d.path()).unwrap();assert!(execute_options(&mut session,&repo,&before.revision,"legacy",SaveOptions{include_untracked:true,keep_index:keep},&mut lock,Some(2)).is_err());}
   let record=d.path().join(DIR).join("record.json");let mut value:serde_json::Value=serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();value.as_object_mut().unwrap().remove("keep_index");fs::write(&record,serde_json::to_vec(&value).unwrap()).unwrap();
   let state=session.snapshot().unwrap();let result=session.mutate(&state.revision,Action::RecoverCheckout);
   if keep {assert!(result.unwrap_err().contains("INVALID_SAVE_BACKUP"));}else{result.unwrap();assert_eq!(fs::read(d.path().join("file")).unwrap(),b"working\n");}
   assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),bytes);
  }
 }
 #[test]fn keep_index_recovery_preserves_later_worktree_and_index_edits(){
  for change_index in [false,true]{
   let d=fixture();let mut session=Session::new(d.path().into()).unwrap();let state=session.snapshot().unwrap();let repo=open(d.path()).unwrap();
   {let mut lock=IndexLock::acquire(d.path()).unwrap();assert!(execute_options(&mut session,&repo,&state.revision,"foreign",SaveOptions{include_untracked:true,keep_index:true},&mut lock,Some(2)).is_err());}
   fs::write(d.path().join("file"),"later user edit\n").unwrap();if change_index{git(d.path(),&["add","--","file"]);}
   let index=fs::read(d.path().join(".git/index")).unwrap();let files=["file","added","deleted","untracked"].map(|n|fs::read(d.path().join(n)).ok());
   let state=session.snapshot().unwrap();assert!(session.mutate(&state.revision,Action::RecoverCheckout).unwrap_err().contains("RECOVERY_CONFLICT"));assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);assert_eq!(["file","added","deleted","untracked"].map(|n|fs::read(d.path().join(n)).ok()),files);
  }
 }

 #[cfg(unix)]
 #[test]
 fn recovery_refuses_replaced_untracked_directory_or_symlink_and_can_retry() {
  for keep_index in [false,true] { for symbolic in [false,true] {
   let d=fixture();
   let outside=tempfile::tempdir().unwrap();
   let target=outside.path().join("preserve");
   fs::write(&target,b"outside user bytes\n").unwrap();
   let mut session=Session::new(d.path().into()).unwrap();
   let before=session.snapshot().unwrap();
   let repo=open(d.path()).unwrap();
   {
    let mut lock=IndexLock::acquire(d.path()).unwrap();
    let error=execute_options(&mut session,&repo,&before.revision,"collision",SaveOptions{include_untracked:true,keep_index},&mut lock,Some(2)).unwrap_err();
    assert!(error.contains("INJECTED_INTERRUPTION"));
   }
   let path=d.path().join("untracked");
   let prior=fs::read(&path).ok();
   if path.exists() {fs::remove_file(&path).unwrap();}
   if symbolic {std::os::unix::fs::symlink(&target,&path).unwrap();}
   else {fs::create_dir(&path).unwrap();fs::write(path.join("new-work"),b"later directory bytes\n").unwrap();}
   let index=fs::read(d.path().join(".git/index")).unwrap();
   let tracked=["file","added","deleted"].map(|n|fs::read(d.path().join(n)).ok());
   let head=git(d.path(),&["rev-parse","HEAD"]);
   let stash=git(d.path(),&["rev-parse","refs/stash"]);
   if symbolic {
    // The public snapshot rejects this layout before offering any mutation.
    assert!(session.snapshot().unwrap_err().contains("UNSUPPORTED_LAYOUT"));
    // Also exercise the recovery boundary directly, without weakening that guard.
    let mut lock=IndexLock::acquire(d.path()).unwrap();
    assert!(recover(d.path(),&repo,&mut lock).is_err());
   } else {
    let state=session.snapshot().unwrap();
    assert!(session.mutate(&state.revision,Action::RecoverCheckout).is_err());
   }
   assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
   assert_eq!(["file","added","deleted"].map(|n|fs::read(d.path().join(n)).ok()),tracked);
   assert_eq!(fs::read(&target).unwrap(),b"outside user bytes\n");
   assert_eq!(git(d.path(),&["rev-parse","HEAD"]),head);
   assert_eq!(git(d.path(),&["rev-parse","refs/stash"]),stash);
   assert!(d.path().join(DIR).exists());
   if symbolic {
    assert_eq!(fs::read_link(&path).unwrap(),target);
    fs::remove_file(&path).unwrap();
   } else {
    assert_eq!(fs::read(path.join("new-work")).unwrap(),b"later directory bytes\n");
    fs::remove_file(path.join("new-work")).unwrap();fs::remove_dir(&path).unwrap();
   }
   if let Some(bytes)=prior {fs::write(&path,bytes).unwrap();}
   let state=session.snapshot().unwrap();
   let recovered=session.mutate(&state.revision,Action::RecoverCheckout).unwrap();
   assert!(!recovered.recovery_pending);
   assert_eq!(fs::read(&path).unwrap(),b"extra\n");
   assert_eq!(fs::read(d.path().join("file")).unwrap(),b"working\n");
   assert_eq!(fs::read(d.path().join("added")).unwrap(),b"working addition\n");
   assert_eq!(fs::read(&target).unwrap(),b"outside user bytes\n");
   assert_eq!(git(d.path(),&["rev-parse","HEAD"]),head);
   assert_eq!(git(d.path(),&["rev-parse","refs/stash"]),stash);
  }}
 }

}
