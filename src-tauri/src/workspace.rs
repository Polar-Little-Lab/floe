//! App-private last workspace; paths are observations, never trust grants.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "workspace.json";
const LIMIT: u64 = 65536;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub paths: Vec<String>,
    pub active: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    workspace: Option<Workspace>,
}
#[derive(Debug, Serialize)]
pub struct State {
    pub workspace: Option<Workspace>,
    pub revision: String,
    pub warning: Option<String>,
}
fn validate(workspace: &Workspace) -> Result<(), String> {
    if workspace.paths.len() > super::MAX_SESSIONS {
        return Err("WORKSPACE_LIMIT".into());
    }
    let mut seen = std::collections::HashSet::new();
    for path in &workspace.paths {
        super::repository_path(path)?;
        if !seen.insert(path) {
            return Err("DUPLICATE_WORKSPACE_PATH".into());
        }
    }
    if workspace.active.as_ref().is_some_and(|p| !seen.contains(p)) {
        return Err("INVALID_ACTIVE_WORKSPACE_PATH".into());
    }
    Ok(())
}
fn check(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err("WORKSPACE_SYMLINK".into()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("WORKSPACE_ACCESS_FAILED".into()),
        }
    }
    Ok(())
}
fn bytes(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = dir.join(FILE);
    check(&path)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("WORKSPACE_ACCESS_FAILED".into()),
        Ok(m) if !m.is_file() || m.len() > LIMIT => return Err("INVALID_WORKSPACE_FILE".into()),
        Ok(_) => (),
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "WORKSPACE_ACCESS_FAILED")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "WORKSPACE_READ_FAILED")?;
    if bytes.len() as u64 > LIMIT {
        return Err("INVALID_WORKSPACE_FILE".into());
    }
    Ok(Some(bytes))
}
fn revision(bytes: &Option<Vec<u8>>) -> String {
    match bytes {
        None => "missing".into(),
        Some(bytes) => format!("{:x}", Sha256::digest(bytes)),
    }
}
pub fn read(dir: &Path) -> Result<State, String> {
    let bytes = bytes(dir)?;
    let revision = revision(&bytes);
    let Some(bytes) = bytes else {
        return Ok(State {
            workspace: None,
            revision,
            warning: None,
        });
    };
    let parsed = serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|r| r.version == 1 && r.workspace.as_ref().is_none_or(|i| validate(i).is_ok()));
    match parsed {
        Some(r) => Ok(State {
            workspace: r.workspace,
            revision,
            warning: None,
        }),
        None => Ok(State {
            workspace: None,
            revision,
            warning: Some(
                "Saved workspace is invalid. Start a new session explicitly to replace it.".into(),
            ),
        }),
    }
}
#[cfg(test)]
fn checkpoint(dir: &Path, stage: &str) {
    if std::env::var("BRANCHDESK_WORKSPACE_STAGE").ok().as_deref() == Some(stage) {
        fs::write(dir.join("workspace-test-ready"), stage).unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
// Explicitly unlock before close: a concurrent subprocess may briefly inherit
// the file description before exec, extending a close-only flock lifetime.
struct HeldLock(fs::File);
impl Drop for HeldLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
pub fn save(dir: &Path, expected: &str, workspace: Option<Workspace>) -> Result<State, String> {
    if expected.len() > 64 {
        return Err("INVALID_WORKSPACE_REVISION".into());
    }
    if let Some(i) = &workspace {
        validate(i)?;
    }
    check(dir)?;
    fs::create_dir_all(dir).map_err(|_| "WORKSPACE_ACCESS_FAILED")?;
    let lock_path = dir.join("workspace.lock");
    check(&lock_path)?;
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
        return Err("INVALID_WORKSPACE_LOCK".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(lock_path)
        .map_err(|_| "WORKSPACE_LOCK_FAILED")?;
    lock.try_lock().map_err(|_| "WORKSPACE_BUSY")?;
    let _held_lock = HeldLock(lock);
    // Persistent inode; guard releases on every return, OS releases on process exit.
    if revision(&bytes(dir)?) != expected {
        return Err("STALE_WORKSPACE: Reload settings before saving.".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    let encoded = serde_json::to_vec(&Record {
        version: 1,
        workspace,
    })
    .map_err(|_| "WORKSPACE_SERIALIZE_FAILED")?;
    if encoded.len() as u64 > LIMIT {
        return Err("WORKSPACE_LIMIT".into());
    }
    #[cfg(test)]
    checkpoint(dir, "before-write");
    temp.write_all(&encoded)
        .map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "WORKSPACE_SYNC_FAILED")?;
    #[cfg(test)]
    checkpoint(dir, "before-publish");
    temp.persist(dir.join(FILE))
        .map_err(|_| "WORKSPACE_WRITE_FAILED")?;
    #[cfg(test)]
    checkpoint(dir, "after-publish");
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| "WORKSPACE_SYNC_FAILED")?;
    read(dir)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> tempfile::TempDir {
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }
    fn value(d: &Path) -> Workspace {
        let paths = vec![
            d.join("one").to_string_lossy().into_owned(),
            d.join("한글 two").to_string_lossy().into_owned(),
        ];
        Workspace {
            active: Some(paths[1].clone()),
            paths,
        }
    }
    #[test]
    fn restart_preserves_order_active_and_stale_writers_refuse() {
        let d = dir();
        let v = value(d.path());
        let saved = save(d.path(), "missing", Some(v.clone())).unwrap();
        assert_eq!(read(d.path()).unwrap().workspace, Some(v));
        let before = fs::read(d.path().join(FILE)).unwrap();
        assert!(save(d.path(), "missing", None)
            .unwrap_err()
            .contains("STALE"));
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), before);
        assert!(save(d.path(), &saved.revision, None)
            .unwrap()
            .workspace
            .is_none());
    }
    #[test]
    fn completed_lock_scope_releases_even_with_duplicate_handle() {
        let d = dir();
        let path = d.path().join("workspace.lock");
        let owner = fs::OpenOptions::new().read(true).write(true)
            .create_new(true).open(&path).unwrap();
        owner.try_lock().unwrap();
        let duplicate = owner.try_clone().unwrap();
        let guard = HeldLock(owner);
        let contender = fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(contender.try_lock().is_err());
        drop(guard);
        contender.try_lock().unwrap();
        contender.unlock().unwrap();
        drop(duplicate);
    }
    #[test]
    fn invalid_records_and_foreign_lock_never_overwrite() {
        let d = dir();
        let s = save(d.path(), "missing", Some(value(d.path()))).unwrap();
        let before = fs::read(d.path().join(FILE)).unwrap();
        let mut v = value(d.path());
        v.paths.push(v.paths[0].clone());
        assert!(save(d.path(), &s.revision, Some(v)).is_err());
        let mut v = value(d.path());
        v.active = Some("/outside".into());
        assert!(save(d.path(), &s.revision, Some(v)).is_err());
        let mut v = value(d.path());
        v.paths[0] = "relative".into();
        assert!(save(d.path(), &s.revision, Some(v)).is_err());
        let mut v = value(d.path());
        v.paths = (0..13)
            .map(|i| d.path().join(i.to_string()).to_string_lossy().into_owned())
            .collect();
        assert!(save(d.path(), &s.revision, Some(v)).is_err());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(d.path().join("workspace.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(save(d.path(), &s.revision, None)
            .unwrap_err()
            .contains("BUSY"));
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), before);
    }
    #[test]
    fn damaged_workspace_requires_explicit_repair_and_oversize_refuses() {
        let d = dir();
        fs::write(d.path().join(FILE), b"broken").unwrap();
        let s = read(d.path()).unwrap();
        assert!(s.warning.is_some());
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), b"broken");
        assert!(save(d.path(), &s.revision, None).unwrap().warning.is_none());
        fs::write(d.path().join(FILE), vec![b'x'; LIMIT as usize + 1]).unwrap();
        assert!(read(d.path()).is_err());
        assert!(save(d.path(), "missing", None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_refusal_and_private_permissions() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let d = dir();
        save(d.path(), "missing", None).unwrap();
        assert_eq!(
            fs::metadata(d.path().join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let other = dir();
        symlink(d.path().join(FILE), other.path().join(FILE)).unwrap();
        assert!(read(other.path()).is_err());
        assert!(save(other.path(), "missing", None).is_err());
    }
    #[test]
    fn workspace_child_job() {
        let Ok(path) = std::env::var("BRANCHDESK_WORKSPACE_CHILD") else {
            return;
        };
        let dir = Path::new(&path);
        let revision = read(dir).unwrap().revision;
        save(dir, &revision, Some(value(dir))).unwrap();
    }
    #[test]
    fn actual_kills_preserve_atomic_workspace_and_release_writer_lock() {
        for existing in [false, true] {
            for stage in ["before-write", "before-publish", "after-publish"] {
                let d = dir();
                let dir = d.path();
                if existing {
                    save(
                        dir,
                        "missing",
                        Some(Workspace {
                            paths: vec![dir.join("old").to_str().unwrap().into()],
                            active: None,
                        }),
                    )
                    .unwrap();
                }
                let old = read(dir).unwrap();
                // Independent private witness must survive every abrupt exit.
                fs::write(dir.join("unrelated-draft"), b"unsaved text witness").unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "workspace::tests::workspace_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_WORKSPACE_CHILD", dir)
                    .env("BRANCHDESK_WORKSPACE_STAGE", stage)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !dir.join("workspace-test-ready").exists() {
                    if let Some(status) = child.try_wait().unwrap() {
                        panic!("worker exited before checkpoint: {status}");
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("workspace checkpoint timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let actual = read(dir).unwrap();
                assert!(actual.warning.is_none());
                if stage == "after-publish" {
                    assert_eq!(actual.workspace, Some(value(dir)));
                } else {
                    assert_eq!(actual.workspace, old.workspace);
                    assert_eq!(actual.revision, old.revision);
                }
                assert_eq!(
                    fs::read(dir.join("unrelated-draft")).unwrap(),
                    b"unsaved text witness"
                );
                // A fresh writer must acquire the persistent inode after process death.
                let next = save(dir, &actual.revision, None).unwrap();
                assert!(next.workspace.is_none());
                if stage == "after-publish" {
                    assert!(save(dir, &old.revision, old.workspace)
                        .unwrap_err()
                        .contains("STALE"));
                }
            }
        }
    }
}
