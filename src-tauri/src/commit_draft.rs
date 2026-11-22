//! App-private repository commit drafts; no Git metadata writes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "commit-draft.json";
const LIMIT: u64 = 262144;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub message: String,
    pub description: String,
    pub author: String,
    pub email: String,
    pub amend: bool,
    pub head_oid: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    draft: Option<Draft>,
}
#[derive(Debug, Serialize)]
pub struct State {
    pub draft: Option<Draft>,
    pub revision: String,
    pub warning: Option<String>,
}
fn validate(draft: &Draft) -> Result<(), String> {
    if draft.message.chars().count() > 10000
        || draft.description.chars().count() > 10000
        || draft.author.chars().count() > 256
        || draft.email.chars().count() > 256
        || [
            &draft.message,
            &draft.description,
            &draft.author,
            &draft.email,
        ]
        .iter()
        .any(|s| s.contains('\0'))
        || draft
            .head_oid
            .as_ref()
            .is_some_and(|s| s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("INVALID_COMMIT_DRAFT".into());
    }
    Ok(())
}
pub fn directory(app_data: &Path, root: &Path) -> Result<std::path::PathBuf, String> {
    // Root comes only from an already-open canonical Session, never a UI path/key.
    let key = format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()));
    let dir = app_data.join("commit-drafts").join(key);
    check(&dir)?;
    Ok(dir)
}
fn check(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err("DRAFT_SYMLINK".into()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("DRAFT_ACCESS_FAILED".into()),
        }
    }
    Ok(())
}
fn bytes(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = dir.join(FILE);
    check(&path)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("DRAFT_ACCESS_FAILED".into()),
        Ok(m) if !m.is_file() || m.len() > LIMIT => return Err("INVALID_DRAFT_FILE".into()),
        Ok(_) => (),
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "DRAFT_ACCESS_FAILED")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "DRAFT_READ_FAILED")?;
    if bytes.len() as u64 > LIMIT {
        return Err("INVALID_DRAFT_FILE".into());
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
            draft: None,
            revision,
            warning: None,
        });
    };
    let parsed = serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|r| r.version == 1 && r.draft.as_ref().is_none_or(|i| validate(i).is_ok()));
    match parsed {
        Some(r) => Ok(State {
            draft: r.draft,
            revision,
            warning: None,
        }),
        None => Ok(State {
            draft: None,
            revision,
            warning: Some("Saved draft is invalid. It was left unchanged.".into()),
        }),
    }
}
pub fn save(dir: &Path, expected: &str, draft: Option<Draft>) -> Result<State, String> {
    if expected.len() > 64 {
        return Err("INVALID_DRAFT_REVISION".into());
    }
    if let Some(i) = &draft {
        validate(i)?;
    }
    check(dir)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|_| "DRAFT_ACCESS_FAILED")?;
    let lock_path = dir.join("commit-draft.lock");
    check(&lock_path)?;
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
        return Err("INVALID_DRAFT_LOCK".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path).map_err(|_| "DRAFT_LOCK_FAILED")?;
    lock.try_lock().map_err(|_| "DRAFT_BUSY")?;
    // Persistent inode, released by closing handle, including on errors/process exit.
    if revision(&bytes(dir)?) != expected {
        return Err("STALE_DRAFT: Reload settings before saving.".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "DRAFT_WRITE_FAILED")?;
    temp.write_all(
        &serde_json::to_vec(&Record { version: 1, draft }).map_err(|_| "DRAFT_SERIALIZE_FAILED")?,
    )
    .map_err(|_| "DRAFT_WRITE_FAILED")?;
    temp.as_file().sync_all().map_err(|_| "DRAFT_SYNC_FAILED")?;
    temp.persist(dir.join(FILE))
        .map_err(|_| "DRAFT_WRITE_FAILED")?;
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| "DRAFT_SYNC_FAILED")?;
    read(dir)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> Draft {
        Draft {
            message: "한글 draft".into(),
            description: "body\nsecond line".into(),
            author: String::new(),
            email: String::new(),
            amend: true,
            head_oid: Some("a".repeat(40)),
        }
    }
    fn temp() -> tempfile::TempDir {
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }
    #[test]
    fn restart_isolation_stale_clear_and_private_storage() {
        let d = temp();
        let a = directory(d.path(), Path::new("/repo/a")).unwrap();
        let b = directory(d.path(), Path::new("/repo/b")).unwrap();
        assert_ne!(a, b);
        let s = save(&a, "missing", Some(draft())).unwrap();
        assert_eq!(read(&a).unwrap().draft, Some(draft()));
        assert!(read(&b).unwrap().draft.is_none());
        let before = fs::read(a.join(FILE)).unwrap();
        assert!(save(&a, "missing", None).unwrap_err().contains("STALE"));
        assert_eq!(fs::read(a.join(FILE)).unwrap(), before);
        assert!(save(&a, &s.revision, None).unwrap().draft.is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(a.join(FILE)).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(fs::metadata(a).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
    #[test]
    fn bounds_corruption_and_foreign_lock_preserve_data() {
        let d = temp();
        let initial = save(d.path(), "missing", Some(draft())).unwrap();
        let original = fs::read(d.path().join(FILE)).unwrap();
        for change in 0..4 {
            let mut v = draft();
            match change {
                0 => v.message = "a".repeat(10001),
                1 => v.description = "\0".into(),
                2 => v.author = "a".repeat(257),
                _ => v.head_oid = Some("invalid".into()),
            };
            assert!(save(d.path(), &initial.revision, Some(v)).is_err());
            assert_eq!(fs::read(d.path().join(FILE)).unwrap(), original);
        }
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(d.path().join("commit-draft.lock"))
            .unwrap();
        lock.lock().unwrap();
        assert!(save(d.path(), &initial.revision, None)
            .unwrap_err()
            .contains("BUSY"));
        drop(lock);
        fs::write(d.path().join(FILE), b"damaged").unwrap();
        let s = read(d.path()).unwrap();
        assert!(s.warning.is_some());
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), b"damaged");
        save(d.path(), &s.revision, None).unwrap();
        fs::write(d.path().join(FILE), vec![0; LIMIT as usize + 1]).unwrap();
        assert!(read(d.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_draft_and_parent_refused() {
        use std::os::unix::fs::symlink;
        let d = temp();
        let target = d.path().join("outside");
        fs::write(&target, b"preserve").unwrap();
        symlink(&target, d.path().join(FILE)).unwrap();
        assert!(read(d.path()).is_err());
        assert!(save(d.path(), "missing", None).is_err());
        assert_eq!(fs::read(target).unwrap(), b"preserve");
        let link = d.path().join("link");
        symlink(d.path(), &link).unwrap();
        assert!(directory(&link, Path::new("/repo")).is_err());
    }
}
