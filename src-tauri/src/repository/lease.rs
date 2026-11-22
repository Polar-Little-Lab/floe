use super::*;

/// Kernel-owned exclusion between FLOE processes. Unlike Git's marker
/// files, this lock is released by the OS when the process terminates.
pub(super) struct OperationLease {
    _file: fs::File,
    common: PathBuf,
}
impl OperationLease {
    pub(super) fn acquire(root: &Path) -> Result<Self> {
        // Branch refs and object storage are shared by all linked worktrees.
        // Serialize FLOE mutations across the entire common repository.
        let layout = layout::Layout::read(root)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(layout.common.join("branchdesk-operation.lock"))
            .map_err(err)?;
        file.try_lock()
            .map_err(|e| format!("BUSY_REPOSITORY: {e}"))?;
        Ok(Self { _file: file, common: layout.common.canonicalize().map_err(err)? })
    }
    pub(super) fn require_repository(&self, layout: &layout::Layout) -> Result<()> {
        if layout.common.canonicalize().map_err(err)? != self.common {
            return Err("INVALID_OPERATION_LEASE".into());
        }
        Ok(())
    }
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        // Closing alone can leave flock held by an inherited/duplicated file
        // description. The operation is done, so release explicitly before close.
        let _ = self._file.unlock();
    }
}

pub(super) fn identity(path: &Path) -> Result<String> {
    let file = fs::File::open(path).map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata().map_err(err)?;
        Ok(format!(
            "unix:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.ctime(),
            m.ctime_nsec()
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: file owns a live handle and info is a correctly sized writable buffer.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(err(std::io::Error::last_os_error()));
        }
        Ok(format!(
            "windows:{}:{}:{}:{}:{}",
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
            info.ftCreationTime.dwHighDateTime,
            info.ftCreationTime.dwLowDateTime
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err("UNSUPPORTED_FILE_IDENTITY".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_lease_releases_even_while_a_duplicate_handle_lives() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".git")).unwrap();
        let lease = OperationLease::acquire(d.path()).unwrap();
        let inherited = lease._file.try_clone().unwrap();
        assert!(OperationLease::acquire(d.path()).is_err());
        drop(lease);
        // Models the file description surviving briefly in a forked child.
        let next = OperationLease::acquire(d.path()).unwrap();
        drop(inherited);
        assert!(OperationLease::acquire(d.path()).is_err());
        drop(next);
        assert!(OperationLease::acquire(d.path()).is_ok());
    }
}
