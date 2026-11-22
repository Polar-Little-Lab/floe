//! Immutable pack publication: verified copies, no overwrite, pack before index.
use super::artifact::checksum;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
pub(super) fn pack_directory(git_dir: &Path) -> Result<PathBuf> {
    let mut path = git_dir.to_owned();
    for part in [None, Some("objects"), Some("pack")] {
        if let Some(part) = part {
            path.push(part);
        }
        let meta = fs::symlink_metadata(&path).map_err(|_| "OBJECT_IMPORT_DIRECTORY")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("OBJECT_IMPORT_DIRECTORY".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("OBJECT_IMPORT_DIRECTORY".into());
            }
        }
    }
    Ok(path)
}
fn existing(path: &Path, maximum: u64, expected: &[u8]) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if checksum(path, maximum)? != expected {
                return Err("OBJECT_IMPORT_CONFLICT".into());
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("OBJECT_IMPORT_CONFLICT".into()),
    }
}
pub(super) fn sync(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| "OBJECT_IMPORT_SYNC")?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}
pub(super) fn publish(source: &Path, destination: &Path, hash: &str) -> Result<()> {
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("OBJECT_IMPORT_HASH".into());
    }
    let source = pack_directory(source)?;
    let target = pack_directory(destination)?;
    let files = [("pack", 128 * 1024 * 1024), ("idx", 8 * 1024 * 1024)];
    let mut copies = Vec::new();
    let mut digests = Vec::new();
    // Preflight both files and conflicts before publishing either object-store file.
    for (extension, limit) in files {
        let name = format!("pack-{hash}.{extension}");
        let source = source.join(&name);
        let target = target.join(&name);
        let digest = checksum(&source, limit)?;
        if extension == "pack"
            && digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                != hash
        {
            return Err("OBJECT_IMPORT_HASH".into());
        }
        let present = existing(&target, limit, &digest)?;
        let copy = if present {
            None
        } else {
            let mut copy = tempfile::NamedTempFile::new_in(target.parent().unwrap())
                .map_err(|_| "OBJECT_IMPORT_WRITE")?;
            let mut input = fs::File::open(source)
                .map_err(|_| "OBJECT_IMPORT_READ")?
                .take(limit + 1);
            let size = std::io::copy(&mut input, &mut copy).map_err(|_| "OBJECT_IMPORT_WRITE")?;
            if size > limit || checksum(copy.path(), limit)? != digest {
                return Err("OBJECT_IMPORT_CHANGED".into());
            }
            copy.as_file()
                .sync_all()
                .map_err(|_| "OBJECT_IMPORT_SYNC")?;
            Some(copy)
        };
        digests.push(digest);
        copies.push((target, limit, copy));
    }
    // The index's penultimate hash must identify this exact pack.
    use std::io::{Seek, SeekFrom};
    let index_path = copies[1]
        .2
        .as_ref()
        .map(|copy| copy.path())
        .unwrap_or(copies[1].0.as_path());
    let mut index = fs::File::open(index_path).map_err(|_| "OBJECT_IMPORT_READ")?;
    index
        .seek(SeekFrom::End(-40))
        .map_err(|_| "OBJECT_IMPORT_READ")?;
    let mut associated = [0; 20];
    index
        .read_exact(&mut associated)
        .map_err(|_| "OBJECT_IMPORT_READ")?;
    if associated.as_slice() != digests[0] {
        return Err("OBJECT_IMPORT_INDEX".into());
    }
    checkpoint(destination, 0);
    for (step, ((path, limit, copy), digest)) in copies.into_iter().zip(digests).enumerate() {
        if let Some(copy) = copy {
            match copy.persist_noclobber(&path) {
                Ok(file) => file.sync_all().map_err(|_| "OBJECT_IMPORT_SYNC")?,
                Err(error) => {
                    if error.error.kind() != std::io::ErrorKind::AlreadyExists
                        || !existing(&path, limit, &digest)?
                    {
                        return Err("OBJECT_IMPORT_CONFLICT".into());
                    }
                }
            }
            sync(&target)?;
        }
        checkpoint(destination, step + 1);
    }
    Ok(())
}
fn checkpoint(destination: &Path, step: usize) {
    #[cfg(test)]
    {
        if std::env::var_os("BRANCHDESK_IMPORT_DEST").as_deref() == Some(destination.as_os_str())
            && std::env::var("BRANCHDESK_IMPORT_STEP").ok().as_deref()
                == Some(step.to_string().as_str())
        {
            fs::write(destination.join("import-marker"), step.to_string()).unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
    #[cfg(not(test))]
    let _ = (destination, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    fn fixture() -> (tempfile::TempDir, String, git2::Oid) {
        let root = tempfile::tempdir().unwrap();
        let source = git2::Repository::init_bare(root.path().join("source")).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let blob = source.blob(b"remote content\n").unwrap();
        let mut builder = source.treebuilder(None).unwrap();
        builder.insert("remote.txt", blob, 0o100644).unwrap();
        let tree = source.find_tree(builder.write().unwrap()).unwrap();
        let oid = source
            .commit(Some("HEAD"), &sig, &sig, "remote", &tree, &[])
            .unwrap();
        let mut pack = source.packbuilder().unwrap();
        pack.insert_commit(oid).unwrap();
        pack.write(&source.path().join("objects/pack"), 0o600)
            .unwrap();
        let hash = pack.name().unwrap().unwrap().to_owned();
        let target = git2::Repository::init(root.path().join("target")).unwrap();
        let blob = target.blob(b"original\n").unwrap();
        let mut builder = target.treebuilder(None).unwrap();
        builder.insert("tracked.txt", blob, 0o100644).unwrap();
        let tree = target.find_tree(builder.write().unwrap()).unwrap();
        target
            .commit(Some("HEAD"), &sig, &sig, "local", &tree, &[])
            .unwrap();
        let mut index = target.index().unwrap();
        index.read_tree(&tree).unwrap();
        index.write().unwrap();
        fs::write(root.path().join("target/tracked.txt"), "unstaged changes\n").unwrap();
        fs::write(root.path().join("target/dirty.txt"), "untracked\n").unwrap();
        (root, hash, oid)
    }
    fn preserved(root: &Path) -> Vec<Vec<u8>> {
        [
            "target/.git/HEAD",
            "target/.git/config",
            "target/.git/index",
            "target/tracked.txt",
            "target/dirty.txt",
        ]
        .iter()
        .map(|path| fs::read(root.join(path)).unwrap())
        .collect()
    }
    fn check_target(root: &Path, oid: git2::Oid) {
        let target = root.join("target/.git");
        let repo = git2::Repository::open(&target).unwrap();
        assert!(repo.find_commit(oid).is_ok());
        assert_eq!(repo.references().unwrap().count(), 1);
        let result = Command::new("/usr/bin/git")
            .arg("--git-dir")
            .arg(target)
            .args(["fsck", "--strict", "--no-reflogs"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn imports_objects_idempotently_without_changing_refs_index_or_worktree() {
        let (root, hash, oid) = fixture();
        let before = preserved(root.path());
        let target = root.path().join("target/.git");
        publish(&root.path().join("source"), &target, &hash).unwrap();
        publish(&root.path().join("source"), &target, &hash).unwrap();
        assert_eq!(preserved(root.path()), before);
        check_target(root.path(), oid);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            for extension in ["pack", "idx"] {
                assert_eq!(
                    fs::metadata(target.join(format!("objects/pack/pack-{hash}.{extension}")))
                        .unwrap()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
    }
    #[test]
    fn refuses_corrupt_existing_destination_and_damaged_source_before_publication() {
        for damaged_source in [false, true] {
            let (root, hash, _) = fixture();
            let before = preserved(root.path());
            let source = root.path().join("source");
            let target = root.path().join("target/.git");
            let damaged = if damaged_source { &source } else { &target };
            let path = damaged.join(format!("objects/pack/pack-{hash}.idx"));
            fs::write(&path, b"do not overwrite").unwrap();
            assert!(publish(&source, &target, &hash).is_err());
            assert_eq!(fs::read(path).unwrap(), b"do not overwrite");
            assert!(!target
                .join(format!("objects/pack/pack-{hash}.pack"))
                .exists());
            assert_eq!(preserved(root.path()), before);
        }
    }
    #[test]
    fn rejects_valid_index_belonging_to_another_pack_before_publication() {
        let (root, hash, _) = fixture();
        let source = root.path().join("source");
        let target = root.path().join("target/.git");
        let other = git2::Repository::init_bare(root.path().join("other")).unwrap();
        let oid = other.blob(b"different object").unwrap();
        let mut builder = other.packbuilder().unwrap();
        builder.insert_object(oid, None).unwrap();
        builder
            .write(&other.path().join("objects/pack"), 0o600)
            .unwrap();
        let other_hash = builder.name().unwrap().unwrap();
        fs::copy(
            other
                .path()
                .join(format!("objects/pack/pack-{other_hash}.idx")),
            source.join(format!("objects/pack/pack-{hash}.idx")),
        )
        .unwrap();
        assert_eq!(
            publish(&source, &target, &hash).unwrap_err(),
            "OBJECT_IMPORT_INDEX"
        );
        assert!(!target
            .join(format!("objects/pack/pack-{hash}.pack"))
            .exists());
        assert!(!target
            .join(format!("objects/pack/pack-{hash}.idx"))
            .exists());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_destination_symlink_without_touching_external_directory() {
        let (root, hash, _) = fixture();
        let target = root.path().join("target/.git");
        let external = root.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::remove_dir(target.join("objects/pack")).unwrap();
        std::os::unix::fs::symlink(&external, target.join("objects/pack")).unwrap();
        assert_eq!(
            publish(&root.path().join("source"), &target, &hash).unwrap_err(),
            "OBJECT_IMPORT_DIRECTORY"
        );
        assert_eq!(fs::read_dir(external).unwrap().count(), 0);
    }
    #[test]
    fn import_crash_child() {
        let Some(destination) = std::env::var_os("BRANCHDESK_IMPORT_DEST") else {
            return;
        };
        let source = std::env::var_os("BRANCHDESK_IMPORT_SOURCE").unwrap();
        let hash = std::env::var("BRANCHDESK_IMPORT_HASH").unwrap();
        publish(Path::new(&source), Path::new(&destination), &hash).unwrap();
    }
    #[test]
    fn actual_process_kills_at_publication_boundaries_are_idempotent() {
        for step in 0..=2 {
            let (root, hash, oid) = fixture();
            let before = preserved(root.path());
            let source = root.path().join("source");
            let target = root.path().join("target/.git");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "network_worker::object_import::tests::import_crash_child",
                    "--nocapture",
                ])
                .env("BRANCHDESK_IMPORT_SOURCE", &source)
                .env("BRANCHDESK_IMPORT_DEST", &target)
                .env("BRANCHDESK_IMPORT_HASH", &hash)
                .env("BRANCHDESK_IMPORT_STEP", step.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !target.join("import-marker").exists() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("import checkpoint timed out");
                }
                assert!(child.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(5));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(preserved(root.path()), before);
            publish(&source, &target, &hash).unwrap();
            assert_eq!(preserved(root.path()), before);
            check_target(root.path(), oid);
        }
    }
}
