//! Build privately, validate, then publish a new repository without replacement.
use crate::{network_worker::Downloaded, repository, Session, Snapshot, Store};
use git2::{Oid, Repository};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
fn error(e: impl std::fmt::Display) -> String {
    format!("CLONE_FAILED: {e}")
}

pub fn destination(value: &str) -> Result<PathBuf> {
    let path = crate::init_repository::validate(value, "main")?;
    let parent = path
        .parent()
        .ok_or("INVALID_PATH")?
        .canonicalize()
        .map_err(|_| "CLONE_PARENT_MISSING")?;
    let target = parent.join(path.file_name().ok_or("INVALID_PATH")?);
    match fs::symlink_metadata(&target) {
        Ok(_) => Err("CLONE_DESTINATION_EXISTS".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(e) => Err(error(e)),
    }
}
fn branch_name(branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.len() > 200
        || branch.starts_with('-')
        || branch == "HEAD"
        || !git2::Reference::is_valid_name(&format!("refs/heads/{branch}"))
    {
        return Err("INVALID_CLONE_BRANCH".into());
    }
    Ok(())
}
pub fn validate_branch(branch: &str) -> Result<()> {
    if !branch.is_empty() {
        branch_name(branch)?;
    }
    Ok(())
}
fn populate(root: &Path, download: &Downloaded, selected: &str) -> Result<()> {
    validate_branch(selected)?;
    let advertisement = download.advertisement();
    let endpoint = repository::remote_endpoint::https(&advertisement.endpoint)?;
    let mut options = git2::RepositoryInitOptions::new();
    options
        .no_reinit(true)
        .external_template(false)
        .initial_head("main");
    let repo = Repository::init_opts(root, &options).map_err(error)?;
    repo.set_config(&git2::Config::new().map_err(error)?)
        .map_err(error)?;
    let mut config = git2::Config::open(&root.join(".git/config")).map_err(error)?;
    config
        .set_str("remote.origin.url", endpoint.as_str())
        .map_err(error)?;
    config
        .set_str("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        .map_err(error)?;
    download.import_into(&root.join(".git"))?;
    let mut refs = BTreeMap::new();
    let mut head = None;
    for reference in &advertisement.refs {
        if reference.name == "HEAD" {
            head = Some(reference);
            continue;
        }
        if reference.name.ends_with("^{}") {
            continue;
        }
        let target = if let Some(name) = reference.name.strip_prefix("refs/heads/") {
            format!("refs/remotes/origin/{name}")
        } else if reference.name.starts_with("refs/tags/") {
            reference.name.clone()
        } else {
            continue;
        };
        if !git2::Reference::is_valid_name(&target)
            || refs.insert(target, reference.oid.clone()).is_some()
        {
            return Err("INVALID_CLONE_REFS".into());
        }
    }
    let mut chosen = None;
    let mut detached = None;
    if !selected.is_empty() {
        let oid = refs
            .get(&format!("refs/remotes/origin/{selected}"))
            .ok_or("CLONE_BRANCH_NOT_FOUND")?;
        chosen = Some((selected.to_owned(), oid.clone()));
    } else if let Some(head) = head {
        if let Some(name) = head
            .symbolic_target
            .as_deref()
            .and_then(|s| s.strip_prefix("refs/heads/"))
        {
            branch_name(name)?;
            if refs.get(&format!("refs/remotes/origin/{name}")) != Some(&head.oid) {
                return Err("INVALID_CLONE_HEAD".into());
            }
            chosen = Some((name.to_owned(), head.oid.clone()));
        } else {
            detached = Some(head.oid.clone());
        }
    }
    let mut names = refs.keys().map(|s| s.to_lowercase()).collect::<Vec<_>>();
    if chosen.is_some() {
        names.push("refs/remotes/origin/head".into());
    }
    names.sort();
    if names
        .windows(2)
        .any(|pair| pair[0] == pair[1] || pair[1].starts_with(&format!("{}/", pair[0])))
    {
        return Err("CLONE_REF_COLLISION".into());
    }
    for (name, oid) in &refs {
        let oid = Oid::from_str(oid).map_err(error)?;
        if name.starts_with("refs/remotes/") {
            repo.find_commit(oid)
                .map_err(|_| "INVALID_CLONE_BRANCH_OBJECT")?;
        }
        repo.reference(name, oid, false, "clone: origin")
            .map_err(error)?;
    }
    let checkout = chosen.as_ref().map(|(_, oid)| oid).or(detached.as_ref());
    if let Some(oid) = checkout {
        let commit = repo
            .find_commit(Oid::from_str(oid).map_err(error)?)
            .map_err(|_| "INVALID_CLONE_HEAD_OBJECT")?;
        let tree = commit.tree().map_err(error)?;
        repository::inspect_clone_tree(&repo, &tree)?;
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut total = 0usize;
        let mut failure = None;
        tree.walk(git2::TreeWalkMode::PreOrder, |prefix, entry| {
            if entry.kind() == Some(git2::ObjectType::Tree) {
                directories.push(format!("{prefix}{}", entry.name().unwrap_or("")));
            }
            if entry.kind() == Some(git2::ObjectType::Blob) {
                match repo.find_blob(entry.id()) {
                    Ok(blob) => {
                        total = total.saturating_add(blob.size());
                        if total > 128 * 1024 * 1024 {
                            failure = Some("CLONE_WORKTREE_LIMIT");
                            return git2::TreeWalkResult::Abort;
                        }
                        files.push((
                            format!("{prefix}{}", entry.name().unwrap_or("")),
                            entry.id(),
                            entry.filemode(),
                        ));
                    }
                    Err(_) => {
                        failure = Some("CLONE_OBJECT_MISSING");
                        return git2::TreeWalkResult::Abort;
                    }
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(error)?;
        if let Some(error) = failure {
            return Err(error.into());
        }
        // Exclusive creation detects filesystem-normalized aliases too, without
        // silently merging two differently named directories from the Git tree.
        for name in directories {
            fs::create_dir(root.join(name)).map_err(|_| "CLONE_DIRECTORY_COLLISION")?;
        }
        for (name, oid, mode) in files {
            let path = root.join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(error)?;
            file.write_all(repo.find_blob(oid).map_err(error)?.content())
                .map_err(error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(if mode == 0o100755 {
                    0o755
                } else {
                    0o644
                }))
                .map_err(error)?;
            }
            #[cfg(not(unix))]
            let _ = mode;
            file.sync_all().map_err(error)?;
        }
        let mut index = repo.index().map_err(error)?;
        index.read_tree(&tree).map_err(error)?;
        index.write().map_err(error)?;
        if let Some((branch, _)) = &chosen {
            repo.reference(
                &format!("refs/heads/{branch}"),
                commit.id(),
                false,
                "clone: origin",
            )
            .map_err(error)?;
            repo.set_head(&format!("refs/heads/{branch}"))
                .map_err(error)?;
            config
                .set_str(&format!("branch.{branch}.remote"), "origin")
                .map_err(error)?;
            config
                .set_str(
                    &format!("branch.{branch}.merge"),
                    &format!("refs/heads/{branch}"),
                )
                .map_err(error)?;
            // origin/HEAD follows the server default, not an explicit checkout override.
            if let Some(default) = head
                .and_then(|h| h.symbolic_target.as_deref())
                .and_then(|s| s.strip_prefix("refs/heads/"))
            {
                if refs.contains_key(&format!("refs/remotes/origin/{default}")) {
                    repo.reference_symbolic(
                        "refs/remotes/origin/HEAD",
                        &format!("refs/remotes/origin/{default}"),
                        false,
                        "clone: remote HEAD",
                    )
                    .map_err(error)?;
                }
            }
        } else {
            repo.set_head_detached(commit.id()).map_err(error)?;
        }
    }
    Ok(())
}
fn sync_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path).map_err(error)? {
        let entry = entry.map_err(error)?;
        if entry.file_type().map_err(error)?.is_dir() {
            sync_tree(&entry.path())?;
        } else {
            // FlushFileBuffers requires GENERIC_WRITE on Windows. These files
            // belong to our unpublished staging tree; never truncate them.
            fs::OpenOptions::new()
                .read(true)
                .write(cfg!(windows))
                .open(entry.path())
                .and_then(|f| f.sync_all())
                .map_err(error)?;
        }
    }
    #[cfg(unix)]
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(error)?;
    Ok(())
}
fn publish(source: &Path, target: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let source = CString::new(source.as_os_str().as_bytes()).map_err(error)?;
        let target = CString::new(target.as_os_str().as_bytes()).map_err(error)?;
        if unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 0x4) } != 0 {
            return Err("CLONE_PUBLISH_REFUSED".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let target = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        } == 0
        {
            return Err("CLONE_PUBLISH_REFUSED".into());
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (source, target);
        return Err("CLONE_PLATFORM_UNSUPPORTED".into());
    }
    #[allow(unreachable_code)]
    Ok(())
}
pub fn create(store: &Store, path: &Path, download: &Downloaded, branch: &str) -> Result<Snapshot> {
    let target = destination(path.to_str().ok_or("INVALID_PATH")?)?;
    validate_branch(branch)?;
    if store.lock().map_err(|_| "LOCK_ERROR")?.len() >= crate::MAX_SESSIONS {
        return Err("TAB_LIMIT".into());
    }
    let parent = target.parent().ok_or("INVALID_PATH")?;
    let staging = tempfile::Builder::new()
        .prefix(".branchdesk-clone-")
        .tempdir_in(parent)
        .map_err(error)?;
    let root = staging.path().join("repository");
    populate(&root, download, branch)?;
    let staged_snapshot = Session::new(root.clone())?.snapshot()?;
    if !staged_snapshot.changes.is_empty() {
        #[cfg(test)]
        eprintln!("Synthetic clone status mismatch: {}", serde_json::to_string(&staged_snapshot.changes).unwrap());
        return Err("CLONE_WORKTREE_MISMATCH".into());
    }
    sync_tree(&root)?;
    checkpoint(&target, 0);
    let mut sessions = store.lock().map_err(|_| "LOCK_ERROR")?;
    if sessions.len() >= crate::MAX_SESSIONS {
        return Err("TAB_LIMIT".into());
    }
    publish(&root, &target)?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(error)?;
    checkpoint(&target, 1);
    let mut session = Session::new(target)?;
    let snapshot = session.snapshot()?;
    sessions.push(session);
    Ok(snapshot)
}
fn checkpoint(target: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_CLONE_TARGET").as_deref() == Some(target.as_os_str())
        && std::env::var("BRANCHDESK_CLONE_STEP").ok().as_deref() == Some(step.to_string().as_str())
    {
        fs::write(
            target.parent().unwrap().join("clone-ready"),
            step.to_string(),
        )
        .unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[cfg(not(test))]
    let _ = (target, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_worker::{Advertisement, RemoteRef};
    use std::{
        process::{Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    const URL: &str = "https://example.com/repository.git";
    fn store() -> Store {
        Arc::new(Mutex::new(Vec::new()))
    }
    fn fixture(root: &Path, unsupported: bool) -> Repository {
        let source = Repository::init_bare(root.join("source")).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let mut tree = source.treebuilder(None).unwrap();
        let mut nested = source.treebuilder(None).unwrap();
        nested
            .insert(
                "nested.txt",
                source.blob(b"nested data\n").unwrap(),
                0o100644,
            )
            .unwrap();
        tree.insert("nested docs", nested.write().unwrap(), 0o040000)
            .unwrap();
        drop(nested);
        tree.insert("readme.txt", source.blob(b"hello\n").unwrap(), 0o100644)
            .unwrap();
        tree.insert(
            "run.sh",
            source.blob(b"#!/bin/sh\nprintf hello\n").unwrap(),
            0o100755,
        )
        .unwrap();
        tree.insert("binary.bin", source.blob(&[0, 255, 1]).unwrap(), 0o100644)
            .unwrap();
        if unsupported {
            tree.insert(
                ".gitattributes",
                source.blob(b"*.txt filter=external\n").unwrap(),
                0o100644,
            )
            .unwrap();
        }
        let oid = source
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "main",
                &source.find_tree(tree.write().unwrap()).unwrap(),
                &[],
            )
            .unwrap();
        source.set_head("refs/heads/main").unwrap();
        source
            .reference("refs/tags/v1", oid, false, "fixture")
            .unwrap();
        source
            .tag(
                "v2",
                source.find_commit(oid).unwrap().as_object(),
                &sig,
                "annotated",
                false,
            )
            .unwrap();
        tree.insert("readme.txt", source.blob(b"topic\n").unwrap(), 0o100644)
            .unwrap();
        source
            .commit(
                Some("refs/heads/feature/topic"),
                &sig,
                &sig,
                "topic",
                &source.find_tree(tree.write().unwrap()).unwrap(),
                &[&source.find_commit(oid).unwrap()],
            )
            .unwrap();
        drop(tree);
        source
    }
    fn advertisement(repo: &Repository) -> Advertisement {
        let mut refs = repo
            .references()
            .unwrap()
            .map(|r| {
                let r = r.unwrap();
                RemoteRef {
                    name: r.name().unwrap().into(),
                    oid: r.target().unwrap().to_string(),
                    symbolic_target: None,
                }
            })
            .collect::<Vec<_>>();
        refs.push(RemoteRef {
            name: "HEAD".into(),
            oid: repo.head().unwrap().target().unwrap().to_string(),
            symbolic_target: Some("refs/heads/main".into()),
        });
        Advertisement {
            endpoint: URL.into(),
            refs,
        }
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    #[test]
    fn clone_default_and_selected_branch_match_git_refs_index_and_files() {
        for branch in ["", "feature/topic"] {
            let temp = tempfile::tempdir().unwrap();
            let source = fixture(temp.path(), false);
            let target = temp.path().join("target with spaces");
            let snapshot = create(
                &store(),
                &target,
                &Downloaded::fixture(&source, advertisement(&source)),
                branch,
            )
            .unwrap();
            assert!(snapshot.changes.is_empty());
            assert!(snapshot.write_block.is_none());
            assert_eq!(
                snapshot.branch,
                if branch.is_empty() { "main" } else { branch }
            );
            let mut args = vec![
                "-c",
                "protocol.file.allow=always",
                "clone",
                "--no-local",
                "--template=",
            ];
            if !branch.is_empty() {
                args.extend(["--branch", branch]);
            }
            let oracle = temp.path().join("oracle");
            args.extend([source.path().to_str().unwrap(), oracle.to_str().unwrap()]);
            git(temp.path(), &args);
            assert_eq!(
                git(
                    &target,
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname) %(symref)"
                    ]
                ),
                git(
                    &oracle,
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname) %(symref)"
                    ]
                )
            );
            assert_eq!(
                git(&target, &["ls-files", "--stage"]),
                git(&oracle, &["ls-files", "--stage"])
            );
            for name in [
                "readme.txt",
                "run.sh",
                "binary.bin",
                "nested docs/nested.txt",
            ] {
                assert_eq!(
                    fs::read(target.join(name)).unwrap(),
                    fs::read(oracle.join(name)).unwrap()
                );
            }
            assert_eq!(git(&target, &["status", "--porcelain"]), "");
            assert_eq!(
                git(&target, &["config", "--get", "remote.origin.url"]).trim(),
                URL
            );
            assert_eq!(
                git(
                    &target,
                    &["rev-parse", "--symbolic-full-name", "@{upstream}"]
                ),
                git(
                    &oracle,
                    &["rev-parse", "--symbolic-full-name", "@{upstream}"]
                )
            );
            git(&target, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn clone_empty_detached_and_missing_remote_head_are_explicit_states() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(temp.path(), false);
        for mode in ["empty", "detached", "missing"] {
            let mut ad = advertisement(&source);
            if mode == "empty" {
                ad.refs.clear();
            }
            if mode == "detached" {
                ad.refs
                    .iter_mut()
                    .find(|r| r.name == "HEAD")
                    .unwrap()
                    .symbolic_target = None;
            }
            if mode == "missing" {
                ad.refs.retain(|r| r.name != "HEAD");
            }
            let target = temp.path().join(mode);
            let result = create(&store(), &target, &Downloaded::fixture(&source, ad), "").unwrap();
            assert_eq!(result.detached, mode == "detached");
            assert_eq!(result.head_oid.is_some(), mode == "detached");
            assert_eq!(target.join("readme.txt").exists(), mode == "detached");
            assert!(result.changes.is_empty());
        }
    }
    #[test]
    fn clone_refuses_collisions_unsafe_tree_and_missing_branch_without_destination() {
        for mode in ["attributes", "case", "missing", "bad-head"] {
            let temp = tempfile::tempdir().unwrap();
            let source = fixture(temp.path(), mode == "attributes");
            let mut ad = advertisement(&source);
            if mode == "case" {
                ad.refs.push(RemoteRef {
                    name: "refs/heads/MAIN".into(),
                    oid: source.head().unwrap().target().unwrap().to_string(),
                    symbolic_target: None,
                });
            }
            if mode == "bad-head" {
                ad.refs
                    .iter_mut()
                    .find(|r| r.name == "HEAD")
                    .unwrap()
                    .symbolic_target = Some("refs/heads/missing".into());
            }
            let target = temp.path().join("target");
            let sessions = store();
            assert!(
                create(
                    &sessions,
                    &target,
                    &Downloaded::fixture(&source, ad),
                    if mode == "missing" {
                        "no-such-branch"
                    } else {
                        ""
                    }
                )
                .is_err(),
                "{mode}"
            );
            assert!(!target.exists());
            assert!(sessions.lock().unwrap().is_empty());
            assert!(!fs::read_dir(temp.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".branchdesk-clone-")));
        }
    }
    #[test]
    fn clone_never_replaces_existing_destinations_even_at_atomic_publish() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(temp.path(), false);
        let data = Downloaded::fixture(&source, advertisement(&source));
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), "user content").unwrap();
        assert!(create(&store(), &target, &data, "").is_err());
        let staged = temp.path().join("staged");
        fs::create_dir(&staged).unwrap();
        assert!(publish(&staged, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("keep")).unwrap(),
            "user content"
        );
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(publish(&staged, &empty).is_err());
        assert!(staged.exists());
        #[cfg(unix)]
        {
            let link = temp.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(create(&store(), &link, &data, "").is_err());
            assert!(publish(&staged, &link).is_err());
        }
    }
    #[test]
    fn clone_refuses_directory_aliases_on_normalizing_filesystems() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("é");
        let second = temp.path().join("e\u{301}");
        fs::create_dir(&first).unwrap();
        let normalizes = fs::create_dir(&second).is_err();
        fs::remove_dir(&first).unwrap();
        if !normalizes {
            fs::remove_dir(&second).unwrap();
        }
        let source = fixture(temp.path(), false);
        let mut child = source.treebuilder(None).unwrap();
        child
            .insert("file.txt", source.blob(b"data").unwrap(), 0o100644)
            .unwrap();
        let child = child.write().unwrap();
        let mut tree = source.treebuilder(None).unwrap();
        tree.insert("é", child, 0o040000).unwrap();
        tree.insert("e\u{301}", child, 0o040000).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let commit = source
            .commit(
                None,
                &sig,
                &sig,
                "alias directories",
                &source.find_tree(tree.write().unwrap()).unwrap(),
                &[],
            )
            .unwrap();
        source
            .reference("refs/heads/main", commit, true, "fixture")
            .unwrap();
        let target = temp.path().join("target");
        let result = create(
            &store(),
            &target,
            &Downloaded::fixture(&source, advertisement(&source)),
            "",
        );
        if normalizes {
            assert!(result.unwrap_err().contains("CLONE_DIRECTORY_COLLISION"));
            assert!(!target.exists());
        } else {
            assert!(result.unwrap().changes.is_empty());
        }
    }
    #[test]
    fn clone_rejects_git_metadata_aliases_before_worktree_publication() {
        for name in [".git", ".g\u{200c}it", ".git\u{feff}", "GIT~1"] {
            let temp = tempfile::tempdir().unwrap();
            let source = fixture(temp.path(), false);
            let blob = source.blob(b"untrusted metadata").unwrap();
            let mut raw = format!("100644 {name}\0").into_bytes();
            raw.extend_from_slice(blob.as_bytes());
            let tree = source
                .odb()
                .unwrap()
                .write(git2::ObjectType::Tree, &raw)
                .unwrap();
            let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
            let commit = source
                .commit(
                    None,
                    &sig,
                    &sig,
                    "unsafe path",
                    &source.find_tree(tree).unwrap(),
                    &[],
                )
                .unwrap();
            source
                .reference("refs/heads/main", commit, true, "fixture")
                .unwrap();
            let target = temp.path().join("target");
            let failure = create(
                &store(),
                &target,
                &Downloaded::fixture(&source, advertisement(&source)),
                "",
            )
            .unwrap_err();
            assert!(
                failure.contains("UNSUPPORTED_CHECKOUT_PATH"),
                "{name}: {failure}"
            );
            assert!(!target.exists());
        }
    }
    #[test]
    #[ignore = "requires selected release worker and public HTTPS"]
    fn actual_release_worker_clones_public_repository() {
        let binary = std::env::var_os("BRANCHDESK_WORKER_BINARY").expect("select built worker");
        let url = "https://github.com/morluto/awesome-reverse-engineering.git";
        let download = Downloaded::fixture_binary(&binary, url).unwrap();
        let expected = download
            .advertisement()
            .refs
            .iter()
            .find(|r| r.name == "HEAD")
            .unwrap()
            .oid
            .clone();
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("clone");
        let snapshot = create(&store(), &target, &download, "").unwrap();
        assert_eq!(snapshot.head_oid.as_deref(), Some(expected.as_str()));
        assert!(snapshot.changes.is_empty());
        assert!(snapshot.write_block.is_none());
        git(&target, &["fsck", "--strict"]);
        println!("actual release clone: HEAD {}, {} tracked files, clean worktree, Git fsck strict passed", expected, git(&target, &["ls-files"]).lines().count());
    }
    #[test]
    fn clone_crash_child() {
        let Some(target) = std::env::var_os("BRANCHDESK_CLONE_TARGET") else {
            return;
        };
        let target = PathBuf::from(target);
        let source = Repository::open_bare(target.parent().unwrap().join("source")).unwrap();
        create(
            &store(),
            &target,
            &Downloaded::fixture(&source, advertisement(&source)),
            "",
        )
        .unwrap();
    }
    #[test]
    fn clone_actual_kill_exposes_no_partial_destination_and_preserves_complete_publication() {
        for step in [0, 1] {
            let temp = tempfile::tempdir().unwrap();
            let _source = fixture(temp.path(), false);
            let target = temp.path().canonicalize().unwrap().join("target");
            let ready = target.parent().unwrap().join("clone-ready");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "clone_repository::tests::clone_crash_child",
                    "--nocapture",
                ])
                .env("BRANCHDESK_CLONE_TARGET", &target)
                .env("BRANCHDESK_CLONE_STEP", step.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !ready.exists() {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("clone checkpoint timeout");
                }
                assert!(child.try_wait().unwrap().is_none(), "clone child exited");
                std::thread::sleep(Duration::from_millis(5));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(target.exists(), step == 1);
            if step == 1 {
                assert!(Session::new(target.clone())
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .changes
                    .is_empty());
                git(&target, &["fsck", "--strict"]);
                assert_eq!(fs::read(target.join("readme.txt")).unwrap(), b"hello\n");
            }
        }
    }
}
