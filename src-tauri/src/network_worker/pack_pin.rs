//! Git-compatible protection of newly imported packs until fetch is archived.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackPin {
    hash: String,
    token: String,
    device: u64,
    inode: u64,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn replaced_same_bytes_and_symlink_pins_are_never_unlinked() {
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(root.path()).unwrap();
        let hash = "1234567890123456789012345678901234567890";
        let pin = PackPin::acquire(repo.path(), hash).unwrap();
        let path = repo.path().join(format!("objects/pack/pack-{hash}.keep"));
        let original = path.with_extension("saved");
        let bytes = fs::read(&path).unwrap();
        fs::rename(&path, &original).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(pin.check(repo.path()).is_err());
        pin.release(repo.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(PackPin::acquire(repo.path(), hash).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&original, &path).unwrap();
        assert!(pin.check(repo.path()).is_err());
        pin.release(repo.path()).unwrap();
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(PackPin::acquire(repo.path(), hash).is_err());
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }
}
impl PackPin {
    pub(crate) fn matches_hash(&self, hash: &str) -> bool {
        self.hash == hash
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.hash.len() != 40
            || !self.hash.bytes().all(|b| b.is_ascii_hexdigit())
            || uuid::Uuid::parse_str(&self.token).is_err()
        {
            return Err("INVALID_FETCH_PACK_PIN".into());
        }
        Ok(())
    }
    fn bytes(&self) -> Vec<u8> {
        format!("FLOE fetch {}\n", self.token).into_bytes()
    }
    pub(super) fn acquire(git_dir: &Path, hash: &str) -> Result<Self> {
        let mut pin = Self {
            hash: hash.into(),
            token: uuid::Uuid::new_v4().to_string(),
            device: 0,
            inode: 0,
        };
        pin.validate()?;
        let directory = super::object_import::pack_directory(git_dir)?;
        let path = directory.join(format!("pack-{hash}.keep"));
        let mut file =
            tempfile::NamedTempFile::new_in(&directory).map_err(|_| "FETCH_PACK_PIN_WRITE")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| "FETCH_PACK_PIN_WRITE")?;
            let meta = file
                .as_file()
                .metadata()
                .map_err(|_| "FETCH_PACK_PIN_WRITE")?;
            pin.device = meta.dev();
            pin.inode = meta.ino();
        }
        file.write_all(&pin.bytes())
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| "FETCH_PACK_PIN_WRITE")?;
        file.persist_noclobber(&path)
            .map_err(|_| "FETCH_PACK_ALREADY_PINNED")?;
        super::object_import::sync(&directory)?;
        Ok(pin)
    }
    pub(crate) fn check(&self, git_dir: &Path) -> Result<()> {
        self.validate()?;
        let directory = super::object_import::pack_directory(git_dir)?;
        let path = directory.join(format!("pack-{}.keep", self.hash));
        let meta = fs::symlink_metadata(&path).map_err(|_| "FETCH_PACK_PIN_CHANGED")?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() != self.bytes().len() as u64
        {
            return Err("FETCH_PACK_PIN_CHANGED".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.dev() != self.device || meta.ino() != self.inode || meta.nlink() != 1 {
                return Err("FETCH_PACK_PIN_CHANGED".into());
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("FETCH_PACK_PIN_CHANGED".into());
            }
        }
        let file = fs::File::open(path).map_err(|_| "FETCH_PACK_PIN_CHANGED")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let opened = file.metadata().map_err(|_| "FETCH_PACK_PIN_CHANGED")?;
            if opened.dev() != self.device || opened.ino() != self.inode || opened.nlink() != 1 {
                return Err("FETCH_PACK_PIN_CHANGED".into());
            }
        }
        let mut bytes = Vec::new();
        file.take(self.bytes().len() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "FETCH_PACK_PIN_CHANGED")?;
        if bytes != self.bytes() {
            return Err("FETCH_PACK_PIN_CHANGED".into());
        }
        Ok(())
    }
    pub(crate) fn release(&self, git_dir: &Path) -> Result<()> {
        // Never unlink a foreign/replaced pin. Windows identity-aware cleanup is pending.
        #[cfg(unix)]
        {
            if self.check(git_dir).is_err() {
                return Ok(());
            }
            let directory = super::object_import::pack_directory(git_dir)?;
            fs::remove_file(directory.join(format!("pack-{}.keep", self.hash)))
                .map_err(|_| "FETCH_PACK_PIN_RELEASE")?;
            super::object_import::sync(&directory)?;
        }
        #[cfg(not(unix))]
        let _ = git_dir;
        Ok(())
    }
}
