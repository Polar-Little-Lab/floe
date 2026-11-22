use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: usize = 20;
const MAX_BYTES: u64 = 256 * 1024;
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    paths: Vec<String>,
}
fn file(dir: &Path) -> PathBuf {
    dir.join("recent-repositories.json")
}
// Persistent lock inode: removing it could split exclusion across processes.
struct WriteLease(fs::File);
impl WriteLease {
    fn acquire(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join("recent-repositories.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err("INVALID_RECENTS_LOCK".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let handle = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        handle
            .try_lock()
            .map_err(|e| format!("BUSY_RECENTS: {e}"))?;
        Ok(Self(handle))
    }
}
impl Drop for WriteLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
#[cfg(test)]
fn checkpoint(dir: &Path, stage: &str) {
    if std::env::var("BRANCHDESK_RECENT_STAGE").ok().as_deref() == Some(stage) {
        fs::write(dir.join("test-ready"), stage).unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
pub fn read(dir: &Path) -> Result<Vec<String>, String> {
    let path = file(dir);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("INVALID_RECENTS_FILE".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("RECENTS_LIMIT".into());
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| "INVALID_RECENTS_JSON")?;
    if record.version != 1 || record.paths.len() > LIMIT {
        return Err("INVALID_RECENTS_VERSION_OR_COUNT".into());
    }
    let mut unique = std::collections::HashSet::new();
    for path in &record.paths {
        super::repository_path(path)?;
        if !unique.insert(path) {
            return Err("DUPLICATE_RECENT_PATH".into());
        }
    }
    Ok(record.paths)
}
fn write(dir: &Path, paths: Vec<String>) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&Record { version: 1, paths }).map_err(|e| e.to_string())?;
    temp.write_all(&bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    #[cfg(test)]
    checkpoint(dir, "before-publish");
    temp.persist(file(dir)).map_err(|e| e.to_string())?;
    #[cfg(test)]
    checkpoint(dir, "after-publish");
    Ok(())
}
pub fn remember(dir: &Path, path: &Path) -> Result<(), String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let path = canonical
        .to_str()
        .ok_or("UNSUPPORTED_RECENT_PATH")?
        .to_owned();
    // canonicalize returns a verbatim drive path on Windows; store the normal
    // absolute form accepted by the same open-path boundary.
    #[cfg(windows)]
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned();
    super::repository_path(&path)?;
    let _lease = WriteLease::acquire(dir)?;
    let mut paths = read(dir)?;
    paths.retain(|p| p != &path);
    paths.insert(0, path);
    paths.truncate(LIMIT);
    write(dir, paths)
}
pub fn forget(dir: &Path, path: &str) -> Result<Vec<String>, String> {
    let _lease = WriteLease::acquire(dir)?;
    let mut paths = read(dir)?;
    paths.retain(|p| p != path);
    write(dir, paths.clone())?;
    Ok(paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_deduplicated_bounded_and_persisted() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state");
        assert!(read(&dir).unwrap().is_empty());
        for n in 0..23 {
            let p = root.path().join(format!("repo-{n}"));
            fs::create_dir(&p).unwrap();
            remember(&dir, &p).unwrap();
        }
        let paths = read(&dir).unwrap();
        assert_eq!(paths.len(), 20);
        assert!(paths[0].ends_with("repo-22"));
        assert!(paths[19].ends_with("repo-3"));
        remember(&dir, &root.path().join("repo-10")).unwrap();
        let paths = read(&dir).unwrap();
        assert_eq!(paths.len(), 20);
        assert!(paths[0].ends_with("repo-10"));
        forget(&dir, &paths[0]).unwrap();
        assert_eq!(read(&dir).unwrap().len(), 19);
        assert!(root.path().join("repo-10").exists());
        assert!(remember(&dir, &root.path().join("missing")).is_err());
        assert_eq!(read(&dir).unwrap().len(), 19);
    }
    #[test]
    fn corrupt_or_untrusted_records_are_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [
            b"broken".to_vec(),
            br#"{"version":2,"paths":[]}"#.to_vec(),
            br#"{"version":1,"paths":["https://example.invalid"]}"#.to_vec(),
            vec![b'x'; MAX_BYTES as usize + 1],
        ] {
            fs::write(file(root.path()), &bytes).unwrap();
            assert!(read(root.path()).is_err());
            assert!(remember(root.path(), root.path()).is_err());
            assert!(forget(root.path(), "anything").is_err());
            assert_eq!(fs::read(file(root.path())).unwrap(), bytes);
        }
    }
    #[test]
    fn moved_repository_stays_listed_until_forgotten_without_accessing_it() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state");
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("dirty.txt"), "uncommitted").unwrap();
        remember(&dir, &repo).unwrap();
        let stored = read(&dir).unwrap()[0].clone();
        let moved = root.path().join("moved");
        fs::rename(&repo, &moved).unwrap();
        assert_eq!(read(&dir).unwrap(), vec![stored.clone()]);
        assert!(forget(&dir, &stored).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(moved.join("dirty.txt")).unwrap(),
            "uncommitted"
        );
    }
    #[test]
    fn writers_refuse_contention_and_preserve_other_updates() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state");
        remember(&dir, root.path()).unwrap();
        let before = fs::read(file(&dir)).unwrap();
        let held = WriteLease::acquire(&dir).unwrap();
        assert!(remember(&dir, &dir)
            .unwrap_err()
            .starts_with("BUSY_RECENTS:"));
        assert!(forget(&dir, "anything")
            .unwrap_err()
            .starts_with("BUSY_RECENTS:"));
        assert_eq!(fs::read(file(&dir)).unwrap(), before);
        let duplicate = held.0.try_clone().unwrap();
        drop(held);
        remember(&dir, &dir).unwrap();
        assert_eq!(read(&dir).unwrap().len(), 2);
        drop(duplicate);
    }
    #[test]
    fn recent_child_job() {
        let Ok(dir) = std::env::var("BRANCHDESK_RECENT_CHILD") else {
            return;
        };
        let dir = PathBuf::from(dir);
        remember(&dir, &dir.join("next-repo")).unwrap();
    }
    #[test]
    fn actual_kill_preserves_complete_old_or_new_list_and_releases_lock() {
        for stage in ["before-publish", "after-publish"] {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("state");
            remember(&dir, root.path()).unwrap();
            let old = read(&dir).unwrap();
            fs::create_dir(dir.join("next-repo")).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "recent::tests::recent_child_job", "--nocapture"])
                .env("BRANCHDESK_RECENT_CHILD", &dir)
                .env("BRANCHDESK_RECENT_STAGE", stage)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !dir.join("test-ready").exists() {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("child exited before checkpoint: {status}");
                }
                if std::time::Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("checkpoint timeout {stage}");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            let survived = read(&dir).unwrap();
            if stage == "before-publish" {
                assert_eq!(survived, old);
            } else {
                assert_eq!(survived.len(), 2);
                assert!(survived[0].ends_with("next-repo"));
            }
            remember(&dir, root.path()).unwrap();
            assert_eq!(read(&dir).unwrap()[0], old[0]);
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_record_rejected_and_repository_alias_deduplicated() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state");
        fs::create_dir(&dir).unwrap();
        let target = root.path().join("target");
        fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, file(&dir)).unwrap();
        assert!(remember(&dir, root.path()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
        let other = root.path().join("other-state");
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(root.path(), &alias).unwrap();
        remember(&other, root.path()).unwrap();
        remember(&other, &alias).unwrap();
        assert_eq!(read(&other).unwrap().len(), 1);
    }
}
