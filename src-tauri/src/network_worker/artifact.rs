//! Parent-side envelope and on-disk artifact checks, after the worker exits.
use super::{pack::Download, Advertisement};
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
fn valid_name(name: &str) -> bool {
    let plain = name.strip_suffix("^{}").unwrap_or(name);
    name.len() <= 1024
        && !name.chars().any(char::is_control)
        && (plain == "HEAD"
            || (plain.starts_with("refs/") && git2::Reference::is_valid_name(plain)))
}
fn valid_oid(oid: &str) -> bool {
    oid.len() == 40
        && oid.bytes().all(|b| b.is_ascii_hexdigit())
        && oid != "0000000000000000000000000000000000000000"
}
pub(super) fn valid_advertisement(value: &Advertisement, url: &str) -> bool {
    let mut names = std::collections::HashSet::new();
    value.endpoint == url
        && value.refs.len() <= 2000
        && value.refs.iter().all(|reference| {
            valid_name(&reference.name)
                && valid_oid(&reference.oid)
                && names.insert(&reference.name)
                && reference.symbolic_target.as_deref().is_none_or(valid_name)
        })
}
fn regular(path: &Path, maximum: u64) -> Result<u64, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > maximum {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err("REMOTE_ARTIFACT_INVALID".into());
        }
    }
    Ok(meta.len())
}
pub(super) fn checksum(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let length = regular(path, maximum)?;
    if length < 20 {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    let mut file = fs::File::open(path).map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    let mut hash = Sha1::new();
    let mut limited = (&mut file).take(length - 20);
    let mut buffer = [0; 64 * 1024];
    let mut read = 0;
    loop {
        let n = limited
            .read(&mut buffer)
            .map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
        if n == 0 {
            break;
        }
        read += n as u64;
        hash.update(&buffer[..n]);
    }
    let mut trailer = [0; 20];
    file.read_exact(&mut trailer)
        .map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    let mut extra = [0; 1];
    if read != length - 20
        || file
            .read(&mut extra)
            .map_err(|_| "REMOTE_ARTIFACT_INVALID")?
            != 0
        || hash.finalize().as_slice() != trailer
    {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    Ok(trailer.to_vec())
}
pub(super) fn verify(root: &Path, url: &str, value: &Download) -> Result<(), String> {
    if !valid_advertisement(&value.advertisement, url)
        || value.objects > 100000
        || value.received_bytes > 128 * 1024 * 1024
    {
        return Err("REMOTE_OUTPUT_INVALID".into());
    }
    let Some(hash) = &value.pack_hash else {
        if value.objects != 0
            || value.received_bytes != 0
            || !value.advertisement.refs.is_empty()
            || root.join("quarantine.git").symlink_metadata().is_ok()
        {
            return Err("REMOTE_OUTPUT_INVALID".into());
        }
        return Ok(());
    };
    if !valid_oid(hash)
        || value.objects == 0
        || value.received_bytes < 32
        || value.advertisement.refs.is_empty()
    {
        return Err("REMOTE_OUTPUT_INVALID".into());
    }
    let mut directory = root.to_path_buf();
    for component in ["quarantine.git", "objects", "pack"] {
        directory.push(component);
        let meta = fs::symlink_metadata(&directory).map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("REMOTE_ARTIFACT_INVALID".into());
        }
    }
    let pack = directory.join(format!("pack-{hash}.pack"));
    let index = directory.join(format!("pack-{hash}.idx"));
    if regular(&pack, 128 * 1024 * 1024)? != value.received_bytes {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    let pack_digest = checksum(&pack, 128 * 1024 * 1024)?;
    let actual = pack_digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if &actual != hash {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    checksum(&index, 8 * 1024 * 1024)?;
    let mut file = fs::File::open(pack).map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    let mut header = [0; 12];
    file.read_exact(&mut header)
        .map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    if &header[..4] != b"PACK"
        || !matches!(u32::from_be_bytes(header[4..8].try_into().unwrap()), 2 | 3)
        || u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize != value.objects
    {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    let mut file = fs::File::open(index).map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    file.seek(SeekFrom::End(-40))
        .map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    let mut indexed = [0; 20];
    file.read_exact(&mut indexed)
        .map_err(|_| "REMOTE_ARTIFACT_INVALID")?;
    if indexed.as_slice() != pack_digest {
        return Err("REMOTE_ARTIFACT_INVALID".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_worker::{supervise_job, RemoteRef, Response};
    use std::{process::Command, time::Duration};
    const URL: &str = "https://example.com/r";
    fn fixture() -> (tempfile::TempDir, Download) {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(source.path()).unwrap();
        let oid = repo.blob(b"fixture bytes").unwrap();
        let mut builder = repo.packbuilder().unwrap();
        builder.insert_object(oid, None).unwrap();
        let directory = root.path().join("quarantine.git/objects/pack");
        fs::create_dir_all(&directory).unwrap();
        builder.write(&directory, 0o600).unwrap();
        let hash = builder.name().unwrap().unwrap().to_owned();
        let received_bytes = fs::metadata(directory.join(format!("pack-{hash}.pack")))
            .unwrap()
            .len();
        (
            root,
            Download {
                advertisement: Advertisement {
                    endpoint: URL.into(),
                    refs: vec![RemoteRef {
                        name: "refs/tags/blob".into(),
                        oid: oid.to_string(),
                        symbolic_target: None,
                    }],
                },
                pack_hash: Some(hash),
                received_bytes,
                objects: 1,
            },
        )
    }
    #[test]
    fn validates_real_pack_and_rejects_corruption_metadata_and_index_mismatch() {
        let (root, mut value) = fixture();
        verify(root.path(), URL, &value).unwrap();
        value.objects += 1;
        assert!(verify(root.path(), URL, &value).is_err());
        value.objects -= 1;
        let path = root.path().join(format!(
            "quarantine.git/objects/pack/pack-{}.pack",
            value.pack_hash.as_ref().unwrap()
        ));
        let original = fs::read(&path).unwrap();
        let mut bytes = original.clone();
        bytes[12] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            verify(root.path(), URL, &value).unwrap_err(),
            "REMOTE_ARTIFACT_INVALID"
        );
        fs::write(&path, original).unwrap();
        let index = path.with_extension("idx");
        let mut bytes = fs::read(&index).unwrap();
        bytes[12] ^= 1;
        fs::write(&index, bytes).unwrap();
        assert!(verify(root.path(), URL, &value).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_hardlinks_and_directory_redirection() {
        let (root, value) = fixture();
        let directory = root.path().join("quarantine.git/objects/pack");
        let pack = directory.join(format!("pack-{}.pack", value.pack_hash.as_ref().unwrap()));
        let other = root.path().join("saved.pack");
        fs::rename(&pack, &other).unwrap();
        std::os::unix::fs::symlink(&other, &pack).unwrap();
        assert!(verify(root.path(), URL, &value).is_err());
        fs::remove_file(&pack).unwrap();
        fs::hard_link(&other, &pack).unwrap();
        assert!(verify(root.path(), URL, &value).is_err());
        fs::remove_file(&other).unwrap();
        verify(root.path(), URL, &value).unwrap();
        let saved = root.path().join("saved-dir");
        fs::rename(&directory, &saved).unwrap();
        std::os::unix::fs::symlink(saved, &directory).unwrap();
        assert!(verify(root.path(), URL, &value).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn supervisor_owns_download_until_drop_and_removes_failed_artifact() {
        let (root, value) = fixture();
        let response = serde_json::to_string(&Response::Download { value }).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "/bin/cp -R \"$1\" quarantine.git; printf '%s' \"$2\"",
                "fixture",
            ])
            .arg(root.path().join("quarantine.git"))
            .arg(&response);
        let completed = supervise_job(command, URL, Duration::from_secs(3), true).unwrap();
        let private = completed._private.path().to_owned();
        assert!(private.join("quarantine.git").is_dir());
        assert!(matches!(completed.response, Response::Download { .. }));
        drop(completed);
        assert!(!private.exists());
        let marker = root.path().join("cwd");
        let mut command = Command::new("/bin/sh");
        command.args(["-c","pwd > \"$3\"; /bin/cp -R \"$1\" quarantine.git; printf 'bad' > quarantine.git/objects/pack/bad; printf '%s' \"$2\"","fixture"]).arg(root.path().join("quarantine.git")).arg(response.replace("\"objects\":1","\"objects\":2")).arg(&marker);
        assert!(supervise_job(command, URL, Duration::from_secs(3), true).is_err());
        let private = fs::read_to_string(marker).unwrap();
        assert!(!Path::new(private.trim()).exists());
    }
    #[cfg(unix)]
    #[test]
    fn supervisor_removes_download_workspace_after_timeout_and_wrong_response() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("cwd");
        for body in ["pwd > \"$1\"; exec /bin/sleep 30", "pwd > \"$1\"; printf '%s' '{\"kind\":\"success\",\"value\":{\"endpoint\":\"https://example.com/r\",\"refs\":[]}}'"] {
            let mut command=Command::new("/bin/sh");command.args(["-c",body,"fixture"]).arg(&marker);
            assert!(supervise_job(command,URL,Duration::from_millis(150),true).is_err());
            let private=fs::read_to_string(&marker).unwrap();assert!(!Path::new(private.trim()).exists());
        }
    }
    #[test]
    #[ignore = "requires an explicitly selected built worker and public HTTPS network"]
    fn parent_actual_download_worker_retains_and_cleans_verified_pack() {
        let binary = std::env::var_os("BRANCHDESK_WORKER_BINARY")
            .expect("select the built worker explicitly");
        let mut command = Command::new(binary);
        command.arg("--branchdesk-remote-worker");
        let completed = supervise_job(
            command,
            "https://github.com/morluto/awesome-reverse-engineering.git",
            Duration::from_secs(20),
            true,
        )
        .unwrap();
        let directory = completed._private.path().to_owned();
        let Response::Download { value } = &completed.response else {
            panic!("expected download")
        };
        assert!(value.objects > 0);
        assert!(directory.join("quarantine.git/objects/pack").is_dir());
        println!(
            "parent verified {} objects, {} pack bytes, {} refs",
            value.objects,
            value.received_bytes,
            value.advertisement.refs.len()
        );
        drop(completed);
        assert!(!directory.exists());
    }
    #[test]
    fn empty_download_and_envelope_validation() {
        let root = tempfile::tempdir().unwrap();
        let mut value = Download {
            advertisement: Advertisement {
                endpoint: URL.into(),
                refs: Vec::new(),
            },
            pack_hash: None,
            received_bytes: 0,
            objects: 0,
        };
        verify(root.path(), URL, &value).unwrap();
        value.objects = 1;
        assert!(verify(root.path(), URL, &value).is_err());
        value.objects = 0;
        value.advertisement.endpoint = "https://other.example/r".into();
        assert!(verify(root.path(), URL, &value).is_err());
        let (_, mut value) = fixture();
        value.advertisement.refs[0].oid = "0".repeat(40);
        assert!(!valid_advertisement(&value.advertisement, URL));
    }
}
