//! App-local default identity. Never reads or modifies Git/global credentials/config.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "commit-identity.json";
const LIMIT: u64 = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub name: String,
    pub email: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    identity: Option<Identity>,
}
#[derive(Debug, Serialize)]
pub struct State {
    pub identity: Option<Identity>,
    pub revision: String,
    pub warning: Option<String>,
}
pub(crate) fn validate(identity: &Identity) -> Result<(), String> {
    if identity.name.trim() != identity.name
        || identity.email.trim() != identity.email
        || identity.name.is_empty()
        || identity.name.len() > 256
        || identity.email.len() > 256
        || !identity.email.contains('@')
        || identity
            .name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>'))
        || identity
            .email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
    {
        return Err("INVALID_COMMIT_IDENTITY".into());
    }
    git2::Signature::now(&identity.name, &identity.email).map_err(|_| "INVALID_COMMIT_IDENTITY")?;
    Ok(())
}
fn check(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err("IDENTITY_SYMLINK".into()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("IDENTITY_ACCESS_FAILED".into()),
        }
    }
    Ok(())
}
fn bytes(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = dir.join(FILE);
    check(&path)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("IDENTITY_ACCESS_FAILED".into()),
        Ok(m) if !m.is_file() || m.len() > LIMIT => return Err("INVALID_IDENTITY_FILE".into()),
        Ok(_) => (),
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "IDENTITY_ACCESS_FAILED")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "IDENTITY_READ_FAILED")?;
    if bytes.len() as u64 > LIMIT {
        return Err("INVALID_IDENTITY_FILE".into());
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
            identity: None,
            revision,
            warning: None,
        });
    };
    let parsed = serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|r| r.version == 1 && r.identity.as_ref().is_none_or(|i| validate(i).is_ok()));
    match parsed {
        Some(r) => Ok(State {
            identity: r.identity,
            revision,
            warning: None,
        }),
        None => Ok(State {
            identity: None,
            revision,
            warning: Some(
                "Saved identity is invalid. Save or clear it to replace the damaged settings."
                    .into(),
            ),
        }),
    }
}
pub fn save(dir: &Path, expected: &str, identity: Option<Identity>) -> Result<State, String> {
    if expected.len() > 64 {
        return Err("INVALID_IDENTITY_REVISION".into());
    }
    if let Some(i) = &identity {
        validate(i)?;
    }
    check(dir)?;
    fs::create_dir_all(dir).map_err(|_| "IDENTITY_ACCESS_FAILED")?;
    let lock_path = dir.join("commit-identity.lock");
    check(&lock_path)?;
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
        return Err("INVALID_IDENTITY_LOCK".into());
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
        .map_err(|_| "IDENTITY_LOCK_FAILED")?;
    lock.try_lock().map_err(|_| "IDENTITY_BUSY")?;
    // Persistent inode, released by closing handle, including on errors/process exit.
    if revision(&bytes(dir)?) != expected {
        return Err("STALE_IDENTITY: Reload settings before saving.".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "IDENTITY_WRITE_FAILED")?;
    temp.write_all(
        &serde_json::to_vec(&Record {
            version: 1,
            identity,
        })
        .map_err(|_| "IDENTITY_SERIALIZE_FAILED")?,
    )
    .map_err(|_| "IDENTITY_WRITE_FAILED")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "IDENTITY_SYNC_FAILED")?;
    temp.persist(dir.join(FILE))
        .map_err(|_| "IDENTITY_WRITE_FAILED")?;
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| "IDENTITY_SYNC_FAILED")?;
    read(dir)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> Identity {
        Identity {
            name: "개발자 Test".into(),
            email: "test@example.invalid".into(),
        }
    }
    #[test]
    fn save_reload_clear_and_git_signature() {
        let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let s = read(d.path()).unwrap();
        let saved = save(d.path(), &s.revision, Some(identity())).unwrap();
        assert_eq!(read(d.path()).unwrap().identity, Some(identity()));
        let i = saved.identity.unwrap();
        let sig = git2::Signature::now(&i.name, &i.email).unwrap();
        assert_eq!(sig.name().unwrap(), i.name);
        assert_eq!(sig.email().unwrap(), i.email);
        let cleared = save(d.path(), &saved.revision, None).unwrap();
        assert_eq!(cleared.identity, None);
        assert!(read(d.path()).unwrap().warning.is_none());
    }
    #[test]
    fn stale_and_invalid_input_preserve_file() {
        let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let first = save(d.path(), "missing", Some(identity())).unwrap();
        let original = fs::read(d.path().join(FILE)).unwrap();
        assert!(save(d.path(), "missing", None)
            .unwrap_err()
            .contains("STALE"));
        for name in ["", " name", "a\nb", "a<b"] {
            let mut i = identity();
            i.name = name.into();
            assert!(save(d.path(), &first.revision, Some(i)).is_err());
        }
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), original);
    }
    #[test]
    fn corrupt_record_is_readable_and_explicitly_repairable() {
        let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        for data in [
            b"invalid".as_slice(),
            br#"{"version":9,"identity":null}"#,
            br#"{"version":1,"identity":null,"extra":1}"#,
        ] {
            fs::write(d.path().join(FILE), data).unwrap();
            let s = read(d.path()).unwrap();
            assert!(s.warning.is_some());
            assert_eq!(fs::read(d.path().join(FILE)).unwrap(), data);
            save(d.path(), &s.revision, Some(identity())).unwrap();
            assert!(read(d.path()).unwrap().warning.is_none());
        }
    }
    #[test]
    fn contention_and_oversized_files_refuse_without_overwrite() {
        let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let s = save(d.path(), "missing", Some(identity())).unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(d.path().join("commit-identity.lock"))
            .unwrap();
        handle.try_lock().unwrap();
        assert!(save(d.path(), &s.revision, None)
            .unwrap_err()
            .contains("BUSY"));
        drop(handle);
        fs::write(d.path().join(FILE), vec![b'a'; LIMIT as usize + 1]).unwrap();
        assert!(read(d.path()).is_err());
        assert!(save(d.path(), &s.revision, None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn private_file_and_symlink_refusal() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let d = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        save(d.path(), "missing", Some(identity())).unwrap();
        assert_eq!(
            fs::metadata(d.path().join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let other = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        symlink(d.path().join(FILE), other.path().join(FILE)).unwrap();
        assert!(read(other.path()).is_err());
        assert!(save(other.path(), "missing", None).is_err());
    }
}
