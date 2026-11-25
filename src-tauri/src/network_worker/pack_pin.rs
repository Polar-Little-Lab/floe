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
        #[cfg(windows)]
        {
            (pin.device, pin.inode) = windows_identity(file.as_file())?;
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
        #[cfg(windows)]
        if windows_identity(&file)? != (self.device, self.inode) {
            return Err("FETCH_PACK_PIN_CHANGED".into());
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
        // Never unlink a foreign/replaced pin.
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
        #[cfg(windows)]
        {
            use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
            use windows_sys::Win32::{Foundation::GENERIC_READ, Storage::FileSystem::{
                DELETE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_DISPOSITION_INFO,
                FileDispositionInfo, SetFileInformationByHandle,
            }};
            self.validate()?;
            let directory = super::object_import::pack_directory(git_dir)?;
            let path = directory.join(format!("pack-{}.keep", self.hash));
            // Exclusive access prevents content edits and path replacement between
            // verification and deletion. Delete the verified handle, not a reopened path.
            let file = match fs::OpenOptions::new()
                .access_mode(GENERIC_READ | DELETE)
                .share_mode(0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path) {
                    Ok(file) => file,
                    Err(_) => return Ok(()), // absent, busy or inaccessible: preserve
                };
            if windows_identity(&file).ok() != Some((self.device, self.inode)) {
                return Ok(());
            }
            let mut bytes = Vec::new();
            (&file).take(self.bytes().len() as u64 + 1).read_to_end(&mut bytes)
                .map_err(|_| "FETCH_PACK_PIN_CHANGED")?;
            if bytes != self.bytes() { return Ok(()); }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: the owned handle has DELETE access and the buffer/type/size agree.
            if unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo,
                &disposition as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32) } == 0 {
                return Err("FETCH_PACK_PIN_RELEASE".into());
            }
            drop(file);
        }
        #[cfg(not(any(unix, windows)))]
        let _ = git_dir;
        Ok(())
    }
}

#[cfg(windows)]
fn windows_identity(file: &fs::File) -> Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_DIRECTORY,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: file owns a live handle; info is a valid, correctly sized output buffer.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
        || info.nNumberOfLinks != 1
        || info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) != 0 {
        return Err("FETCH_PACK_PIN_CHANGED".into());
    }
    Ok((u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)))
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn legacy_identity_busy_handles_and_modified_content_are_preserved() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(root.path()).unwrap();
        let hash = "1234567890123456789012345678901234567890";
        let path = repo.path().join(format!("objects/pack/pack-{hash}.keep"));
        let pin = PackPin::acquire(repo.path(), hash).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut legacy: PackPin = serde_json::from_slice(&serde_json::to_vec(&pin).unwrap()).unwrap();
        legacy.device = 0;
        legacy.inode = 0;
        assert!(legacy.check(repo.path()).is_err());
        legacy.release(repo.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let busy = fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        pin.release(repo.path()).unwrap();
        assert!(path.exists());
        drop(busy);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(&path, b"external replacement contents").unwrap();
        assert!(pin.check(repo.path()).is_err());
        pin.release(repo.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"external replacement contents");
    }
    #[test]
    fn owned_pin_releases_but_identical_replacement_and_hardlink_survive() {
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(root.path()).unwrap();
        let hash = "1234567890123456789012345678901234567890";
        let path = repo.path().join(format!("objects/pack/pack-{hash}.keep"));
        let pin = PackPin::acquire(repo.path(), hash).unwrap();
        pin.check(repo.path()).unwrap();
        pin.release(repo.path()).unwrap();
        assert!(!path.exists());
        let pin = PackPin::acquire(repo.path(), hash).unwrap();
        let bytes = fs::read(&path).unwrap();
        let original = path.with_extension("saved");
        fs::rename(&path, &original).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(pin.check(repo.path()).is_err());
        pin.release(repo.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        fs::hard_link(&original, &path).unwrap();
        assert!(pin.check(repo.path()).is_err());
        pin.release(repo.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }
}
