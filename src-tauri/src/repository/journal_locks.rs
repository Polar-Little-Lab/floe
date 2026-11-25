//! Only release recorded, unchanged Git lock markers while holding the OS lease.
use super::*;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    path: String,
    identity: String,
    digest: String,
}
// Linked journals carry their private metadata owner, preventing accidental
// replay of a sibling's manifest. Main-worktree v1 arrays remain compatible.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkedMarkers { version: u8, private: PathBuf, markers: Vec<Marker> }
fn lock_path(layout: &layout::Layout, path: &str) -> PathBuf {
    if matches!(path, "config.lock" | "packed-refs.lock") || path.starts_with("refs/") || path.starts_with("logs/refs/") {
        layout.common.join(path)
    } else { layout.private.join(path) }
}
fn allowed(path: &str) -> bool {
    matches!(
        path,
        "FETCH_HEAD.lock"
            | "config.lock"
            | "packed-refs.lock"
            | "index.lock"
            | "HEAD.lock"
            | "AUTO_MERGE.lock"
            | "refs/stash.lock"
            | "logs/refs/stash.lock"
    ) || path.strip_suffix(".lock").is_some_and(|name| {
        (name.starts_with("refs/heads/") || name.starts_with("refs/remotes/"))
            && git2::Reference::is_valid_name(name)
            || name.strip_prefix("logs/").is_some_and(|r| {
                r.starts_with("refs/remotes/") && git2::Reference::is_valid_name(r)
            })
    })
}
fn allowed_for(path: &str, fetch: bool) -> bool {
    allowed(path)
        || (fetch
            && path.strip_suffix(".lock").is_some_and(|name| {
                let reference = name.strip_prefix("logs/").unwrap_or(name);
                reference.starts_with("refs/tags/") && git2::Reference::is_valid_name(reference)
            }))
}
pub(super) fn record(
    root: &Path,
    dir: &Path,
    branch: Option<&str>,
    stash: bool,
    log: bool,
) -> Result<()> {
    record_inner(root, dir, branch, stash, log, false)
}
pub(super) fn record_merge(root: &Path, dir: &Path, branch: &str) -> Result<()> {
    record_inner(root, dir, Some(branch), false, false, true)
}
fn record_inner(
    root: &Path,
    dir: &Path,
    branch: Option<&str>,
    stash: bool,
    log: bool,
    auto_merge: bool,
) -> Result<()> {
    let mut paths = vec!["index.lock".to_string()];
    if auto_merge {
        paths.push("AUTO_MERGE.lock".into());
    }
    if let Some(branch) = branch {
        paths.extend(["HEAD.lock".into(), format!("{branch}.lock")]);
    }
    if stash {
        paths.push("refs/stash.lock".into());
    }
    if log {
        paths.push("logs/refs/stash.lock".into());
    }
    record_paths(root, dir, paths)
}
pub(super) fn record_paths(root: &Path, dir: &Path, mut paths: Vec<String>) -> Result<()> {
    let layout=layout::Layout::read(root)?;
    if dir.parent().ok_or("INVALID_JOURNAL_PATH")?.canonicalize().map_err(err)? != layout.private.canonicalize().map_err(err)? {return Err("INVALID_JOURNAL_PATH".into());}
    paths.sort();
    paths.dedup();
    let mut markers = Vec::new();
    for path in paths {
        if !allowed_for(
            &path,
            dir.file_name().is_some_and(|n| n == "branchdesk-fetch"),
        ) {
            return Err("INVALID_LOCK_PATH".into());
        }
        let full = lock_path(&layout, &path);
        if full.exists() {
            markers.push(Marker {
                path,
                identity: lease::identity(&full)?,
                digest: checkout::digest(&bounded(&full, 20 * 1024 * 1024)?),
            });
        }
    }
    let next = dir.join(format!("locks-{}.next", Uuid::new_v4()));
    let bytes=if layout.linked {serde_json::to_vec(&LinkedMarkers{version:2,private:layout.private.canonicalize().map_err(err)?,markers})} else {serde_json::to_vec(&markers)}.map_err(err)?;
    checkout::save(&next, &bytes)?;
    fs::rename(next, dir.join("locks.json")).map_err(err)?;
    checkout::sync_dir(dir)
}
pub(super) fn release(root: &Path, lease: &lease::OperationLease) -> Result<()> {
    let layout=layout::Layout::read(root)?;
    lease.require_repository(&layout)?;
    let mut markers: Vec<Marker> = Vec::new();
    let mut owners = std::collections::HashMap::new();
    for name in [
        "branchdesk-fetch",
        "branchdesk-remote-rename",
        "branchdesk-remote-remove",
        "branchdesk-remote-config",
        "branchdesk-remote-head",
        "branchdesk-commit",
        "branchdesk-branch-rename",
        "branchdesk-merge-abort",
        "branchdesk-merge-finish",
        "branchdesk-merge-index",
        "branchdesk-stash-apply",
        "branchdesk-stash-applied",
        "branchdesk-stash-save",
        "branchdesk-stash-drop",
    ] {
        let file = layout.private.join(name).join("locks.json");
        if file.exists() {
            let bytes=bounded(&file,1024*1024)?;
            let loaded=if layout.linked {
                let record:LinkedMarkers=serde_json::from_slice(&bytes).map_err(err)?;
                if record.version!=2 || record.private!=layout.private.canonicalize().map_err(err)? {return Err("INVALID_LOCK_OWNER".into());}
                record.markers
            } else {serde_json::from_slice::<Vec<Marker>>(&bytes).map_err(err)?};
            if loaded
                .iter()
                .any(|marker| !allowed_for(&marker.path, name == "branchdesk-fetch"))
            {
                return Err("INVALID_LOCK_RECORD".into());
            }
            let mut local = std::collections::HashSet::new();
            for marker in &loaded {
                if !local.insert(marker.path.clone()) { return Err("INVALID_LOCK_RECORD".into()); }
                if let Some(previous) = owners.insert(marker.path.clone(), name) {
                    if marker.path != "index.lock" || previous != "branchdesk-stash-applied" || name != "branchdesk-stash-drop" {
                        return Err("INVALID_LOCK_RECORD".into());
                    }
                }
            }
            markers.extend(loaded);
        }
    }
    let mut seen = std::collections::HashMap::new();
    for marker in &markers {
        if let Some((identity, digest)) = seen.insert(&marker.path, (&marker.identity, &marker.digest)) {
            // A completed apply and an in-flight drop may own the same unchanged
            // marker. Conflicting identities must never authorize removal.
            if identity != &marker.identity || digest != &marker.digest {
                return Err("INVALID_LOCK_RECORD".into());
            }
        }
        check(&layout, marker)?;
    }
    // Check all first: a foreign marker leaves even matching markers untouched.
    for marker in &markers {
        check(&layout, marker)?;
        let path = lock_path(&layout, &marker.path);
        if path.exists() {
            fs::remove_file(path).map_err(err)?;
        }
    }
    checkout::sync_dir(&layout.private)?;
    if layout.private!=layout.common {checkout::sync_dir(&layout.common)?;}
    Ok(())
}
fn check(layout: &layout::Layout, marker: &Marker) -> Result<()> {
    let path = lock_path(layout, &marker.path);
    if path.exists()
        && (lease::identity(&path)? != marker.identity
            || checkout::digest(&bounded(&path, 20 * 1024 * 1024)?) != marker.digest)
    {
        return Err("EXTERNAL_LOCK: changed lock preserved".into());
    }
    Ok(())
}
pub(super) fn checkpoint(root: &Path, kind: &str, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_STASH_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_STASH_KIND").ok().as_deref() == Some(kind)
        && std::env::var("BRANCHDESK_STASH_STEP").ok().as_deref() == Some(step.to_string().as_str())
    {
        fs::write(
            root.join(".git/branchdesk-stash-test.ready"),
            step.to_string(),
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, kind, step);
}


#[cfg(test)]
mod linked_tests {
    use super::*;
    fn setup(root:&Path)->(layout::Layout,PathBuf){
        let layout=layout::Layout::read(root).unwrap();let dir=layout.private.join("branchdesk-commit");fs::create_dir(&dir).unwrap();(layout,dir)
    }
    fn marker(layout:&layout::Layout,name:&str,bytes:&[u8])->PathBuf{
        assert!(allowed_for(name,true));let p=lock_path(layout,name);fs::create_dir_all(p.parent().unwrap()).unwrap();fs::write(&p,bytes).unwrap();p
    }
    #[test]
    fn linked_recovery_releases_own_private_and_shared_markers_only(){
        let (_temp,main,first,second)=layout::write_isolation_tests::fixture();let (layout,dir)=setup(&first);
        let main_lock=main.join(".git/index.lock");fs::write(&main_lock,b"main owner").unwrap();
        let sibling=layout::Layout::read(&second).unwrap().private.join("index.lock");fs::write(&sibling,b"sibling owner").unwrap();
        let names=["index.lock","HEAD.lock","refs/heads/first.lock","config.lock","packed-refs.lock"];
        let paths:Vec<_>=names.iter().map(|n|marker(&layout,n,b"owned")).collect();
        record_paths(&first,&dir,names.iter().map(|n|n.to_string()).collect()).unwrap();
        let lease=lease::OperationLease::acquire(&first).unwrap();release(&first,&lease).unwrap();release(&first,&lease).unwrap();
        assert!(paths.iter().all(|p|!p.exists()));assert_eq!(fs::read(main_lock).unwrap(),b"main owner");assert_eq!(fs::read(sibling).unwrap(),b"sibling owner");
    }
    #[test]
    fn sibling_copy_of_journal_is_rejected_before_any_lock_removal(){
        let (_temp,_main,first,second)=layout::write_isolation_tests::fixture();let (layout,dir)=setup(&first);let (other,other_dir)=setup(&second);
        let original=marker(&layout,"index.lock",b"first");let sibling=marker(&other,"index.lock",b"second");
        record_paths(&first,&dir,vec!["index.lock".into()]).unwrap();fs::copy(dir.join("locks.json"),other_dir.join("locks.json")).unwrap();
        let lease=lease::OperationLease::acquire(&second).unwrap();assert!(release(&second,&lease).unwrap_err().contains("INVALID_LOCK_OWNER"));
        assert_eq!(fs::read(original).unwrap(),b"first");assert_eq!(fs::read(sibling).unwrap(),b"second");
    }
    #[test]
    fn changed_shared_marker_preserves_all_owned_private_markers(){
        let (_temp,_main,first,_second)=layout::write_isolation_tests::fixture();let (layout,dir)=setup(&first);
        let private=marker(&layout,"index.lock",b"private");let shared=marker(&layout,"refs/heads/first.lock",b"shared");
        record_paths(&first,&dir,vec!["index.lock".into(),"refs/heads/first.lock".into()]).unwrap();fs::write(&shared,b"external").unwrap();
        let lease=lease::OperationLease::acquire(&first).unwrap();assert!(release(&first,&lease).unwrap_err().contains("EXTERNAL_LOCK"));
        assert_eq!(fs::read(private).unwrap(),b"private");assert_eq!(fs::read(shared).unwrap(),b"external");
    }
    #[test]
    fn foreign_repository_lease_cannot_authorize_release(){
        let (_temp,_main,first,_second)=layout::write_isolation_tests::fixture();let (layout,dir)=setup(&first);
        let path=marker(&layout,"index.lock",b"owned");record_paths(&first,&dir,vec!["index.lock".into()]).unwrap();
        let unrelated=super::super::tests::fixture();let lease=lease::OperationLease::acquire(unrelated.path()).unwrap();
        assert_eq!(release(&first,&lease).unwrap_err(),"INVALID_OPERATION_LEASE");assert_eq!(fs::read(path).unwrap(),b"owned");
    }
    #[test]
    fn linked_journal_rejects_legacy_unscoped_and_traversal_records(){
        let (_temp,_main,first,_second)=layout::write_isolation_tests::fixture();let (layout,dir)=setup(&first);
        let path=marker(&layout,"index.lock",b"owned");record_paths(&first,&dir,vec!["index.lock".into()]).unwrap();
        let bytes=fs::read(dir.join("locks.json")).unwrap();let mut record:LinkedMarkers=serde_json::from_slice(&bytes).unwrap();
        let lease=lease::OperationLease::acquire(&first).unwrap();fs::write(dir.join("locks.json"),serde_json::to_vec(&record.markers).unwrap()).unwrap();assert!(release(&first,&lease).is_err());
        record.markers[0].path="../index.lock".into();fs::write(dir.join("locks.json"),serde_json::to_vec(&record).unwrap()).unwrap();assert_eq!(release(&first,&lease).unwrap_err(),"INVALID_LOCK_RECORD");assert_eq!(fs::read(path).unwrap(),b"owned");
    }
    #[test]
    fn linked_record_refuses_common_journal_and_maps_fetch_refs_to_common(){
        let (_temp,main,first,_second)=layout::write_isolation_tests::fixture();let (layout,_dir)=setup(&first);
        let wrong=main.join(".git/branchdesk-fetch");fs::create_dir(&wrong).unwrap();assert_eq!(record_paths(&first,&wrong,vec![]).unwrap_err(),"INVALID_JOURNAL_PATH");assert!(!wrong.join("locks.json").exists());
        let dir=layout.private.join("branchdesk-fetch");fs::create_dir(&dir).unwrap();
        let names=["FETCH_HEAD.lock","refs/tags/v1.lock","logs/refs/tags/v1.lock","refs/remotes/origin/main.lock"];
        let paths:Vec<_>=names.iter().map(|n|marker(&layout,n,b"fetch")).collect();record_paths(&first,&dir,names.iter().map(|n|n.to_string()).collect()).unwrap();
        assert!(paths[0].starts_with(&layout.private));assert!(paths[1..].iter().all(|p|p.starts_with(&layout.common)));
        let lease=lease::OperationLease::acquire(&main).unwrap();release(&first,&lease).unwrap();assert!(paths.iter().all(|p|!p.exists()));
    }
}
