//! Opt-in real ENOSPC test, restricted to a disposable mounted disk image.
use crate::{repository::{Action, Session}, Store};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::{Arc, Mutex}};
struct Filled(PathBuf);
impl Drop for Filled { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
fn fill(root: &Path) -> Filled {
    let path = root.join("enospc-filler");
    let guard = Filled(path.clone());
    let mut f = fs::File::create(path).unwrap();
    let block = vec![0x5a; 1024 * 1024];
    let mut total = 0;
    loop {
        match f.write_all(&block) {
            Ok(()) => { total += block.len(); assert!(total < 80 * 1024 * 1024, "volume exceeds safe fixture bound"); }
            Err(e) => { assert_eq!(e.raw_os_error(), Some(28)); break; }
        }
    }
    loop {
        match f.write_all(&[1; 4096]) {
            Ok(()) => (),
            Err(e) => { assert_eq!(e.raw_os_error(), Some(28)); break; }
        }
    }
    guard
}
#[test]
#[ignore = "requires dedicated <=64MiB HFS disk image via BRANCHDESK_DISK_FAULT_ROOT"]
fn real_enospc_preserves_git_and_settings_then_retries() {
    let root = PathBuf::from(std::env::var("BRANCHDESK_DISK_FAULT_ROOT").unwrap()).canonicalize().unwrap();
    assert!(root.starts_with("/Volumes/FLOEFault-" ) || root.file_name().unwrap().to_string_lossy().starts_with("FLOEFault-"));
    assert_eq!(fs::read(root.join(".branchdesk-disposable-volume")).unwrap(), b"disposable-enospc-fixture-v1");
    let store: Store = Arc::new(Mutex::new(Vec::new()));
    let repo_path = root.join("repository");
    crate::init_repository::create(&store, &repo_path, "main").unwrap();
    fs::write(repo_path.join("file.txt"), "original\n").unwrap();
    let mut session = Session::new(repo_path.clone()).unwrap();
    let s = session.snapshot().unwrap();
    let s = session.mutate(&s.revision, Action::StageAll).unwrap();
    session.mutate(&s.revision, Action::Commit{message:"baseline".into(),author:"Synthetic".into(),email:"test@example.invalid".into()}).unwrap();
    fs::write(repo_path.join("file.txt"), "pending user edit\n").unwrap();
    let s = session.snapshot().unwrap();
    let index = fs::read(repo_path.join(".git/index")).unwrap();
    let head = s.head_oid.clone();
    let settings = root.join("settings");
    crate::recent::remember(&settings,&repo_path).unwrap();
    let ws = crate::workspace::Workspace{paths:vec![repo_path.to_str().unwrap().into()],active:Some(repo_path.to_str().unwrap().into())};
    let w = crate::workspace::read(&settings).unwrap();
    let w = crate::workspace::save(&settings,&w.revision,Some(ws.clone())).unwrap();
    let recent = fs::read(settings.join("recent-repositories.json")).unwrap();
    let workspace = fs::read(settings.join("workspace.json")).unwrap();
    let full = fill(&root);
    assert!(session.mutate(&s.revision,Action::StageAll).is_err());
    assert_eq!(fs::read(repo_path.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(repo_path.join("file.txt")).unwrap(), b"pending user edit\n");
    assert!(!repo_path.join(".git/index.lock").exists());
    assert!(crate::recent::remember(&settings,&root).is_err());
    assert_eq!(fs::read(settings.join("recent-repositories.json")).unwrap(),recent);
    assert!(crate::workspace::save(&settings,&w.revision,None).is_err());
    assert_eq!(fs::read(settings.join("workspace.json")).unwrap(),workspace);
    assert!(crate::init_repository::create(&store,&root.join("failed-init"),"main").is_err());
    assert_eq!(store.lock().unwrap().len(),1);
    drop(full);
    let s = session.snapshot().unwrap();
    assert_eq!(s.head_oid,head);
    let s = if s.recovery_pending { session.mutate(&s.revision,Action::RecoverCheckout).unwrap() } else { s };
    let s = session.mutate(&s.revision,Action::StageAll).unwrap();
    let staged_index = fs::read(repo_path.join(".git/index")).unwrap();
    let full = fill(&root);
    assert!(session.mutate(&s.revision,Action::Commit{message:"disk-full commit".into(),author:"Synthetic".into(),email:"test@example.invalid".into()}).is_err());
    assert_eq!(git2::Repository::open(&repo_path).unwrap().head().unwrap().target().unwrap().to_string(),head.clone().unwrap());
    assert_eq!(fs::read(repo_path.join(".git/index")).unwrap(),staged_index);
    assert_eq!(fs::read(repo_path.join("file.txt")).unwrap(),b"pending user edit\n");
    drop(full);
    let s = session.snapshot().unwrap();
    let s = if s.recovery_pending { session.mutate(&s.revision,Action::RecoverCheckout).unwrap() } else { s };
    let s = session.mutate(&s.revision,Action::Commit{message:"after space restored".into(),author:"Synthetic".into(),email:"test@example.invalid".into()}).unwrap();
    assert_ne!(s.head_oid,head); assert!(s.changes.is_empty());
    crate::recent::remember(&settings,&root).unwrap();
    crate::workspace::save(&settings,&w.revision,None).unwrap();
    crate::init_repository::create(&store,&root.join("retry-init"),"main").unwrap();
    let check = std::process::Command::new(crate::test_git_executable()).args(["-C",repo_path.to_str().unwrap(),"fsck","--strict"]).output().unwrap();
    assert!(check.status.success(),"{}",String::from_utf8_lossy(&check.stderr));
}
