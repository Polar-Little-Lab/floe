//! Test-only macOS OpenSSH process prototype. Not registered as a native command.
//! Caller-provided files belong to a synthetic private fixture, never user config.
use crate::ssh_endpoint;
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;

fn trusted_advertisement(url: &str, trust: (&Path, &str), identity: &Path,
                         directory: &Path, deadline: Duration,
                         cancelled: Arc<AtomicBool>, want: Option<&[String]>) -> Result<tempfile::NamedTempFile, String> {
    let endpoint = ssh_endpoint::parse(url)?;
    if endpoint.user.is_none() { return Err("SSH_USER_REQUIRED".into()); }
    if cancelled.load(Ordering::Acquire) { return Err("SSH_CANCELLED".into()); }
    let scope = crate::ssh_trust::Scope::new(&endpoint.host, endpoint.port)?;
    let pin = crate::ssh_trust::export_pin(trust.0, trust.1, &scope)?;
    // RAII keeps the frozen approved pin alive through kill/reap and pipe joins.
    advertisement(url, pin.path(), identity, directory, deadline, cancelled, want)
}

pub(super) fn drain(mut reader: impl Read, overflow: Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let size = reader.read(&mut buffer).map_err(|_| "SSH_READ_FAILED")?;
        if size == 0 { return Ok(bytes); }
        if bytes.len() + size > OUTPUT_LIMIT {
            overflow.store(true, Ordering::Release);
        } else if !overflow.load(Ordering::Acquire) {
            bytes.extend_from_slice(&buffer[..size]);
        }
    }
}

fn spool(mut reader: impl Read, mut file: tempfile::NamedTempFile, limit: u64,
         stop: Arc<AtomicBool>) -> Result<tempfile::NamedTempFile, String> {
    let result = (|| {
        let mut received = 0u64;
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).map_err(|_| "SSH_READ_FAILED")?;
            if count == 0 { break; }
            received += count as u64;
            if received > limit { return Err("SSH_OUTPUT_LIMIT".into()); }
            file.write_all(&buffer[..count]).map_err(|_| "SSH_SPOOL_FAILED")?;
        }
        file.seek(SeekFrom::Start(0)).map_err(|_| "SSH_SPOOL_FAILED")?;
        Ok(file)
    })();
    if result.is_err() { stop.store(true, Ordering::Release); }
    result
}

fn negotiate(mut reader: impl Read, mut input: impl Write, mut capture: tempfile::NamedTempFile,
             request: (String, Option<Vec<String>>, Vec<u8>), limit: u64,
             cancelled: Arc<AtomicBool>, abort: Arc<AtomicBool>) -> Result<tempfile::NamedTempFile, String> {
    struct CopyReader<'a, R> { reader: &'a mut R, file: &'a mut std::fs::File, received: u64 }
    impl<R: Read> Read for CopyReader<'_, R> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let count = self.reader.read(output)?;
            self.file.write_all(&output[..count])?;
            self.received += count as u64;
            Ok(count)
        }
    }
    let result = (|| {
        let mut tee = CopyReader { reader: &mut reader, file: capture.as_file_mut(), received: 0 };
        let advertisement = crate::network_worker::read_ssh_advertisement(&request.0, &mut tee)
            .map_err(|error| if tee.received == 0 { "SSH_CONNECTION_FAILED".to_string() } else { error })?;
        let received = tee.received;
        if request.1.as_ref().is_some_and(|oids| oids.iter().any(|oid| !advertisement.refs.iter().any(|r| r.oid.eq_ignore_ascii_case(oid)))) {
            return Err("SSH_WANT_NOT_ADVERTISED".into());
        }
        if cancelled.load(Ordering::Acquire) { return Err("SSH_CANCELLED".into()); }
        input.write_all(&request.2).map_err(|_| "SSH_WRITE_FAILED")?;
        drop(input);
        spool(reader, capture, limit.checked_sub(received).ok_or("SSH_OUTPUT_LIMIT")?, abort.clone())
    })();
    if result.is_err() { abort.store(true, Ordering::Release); }
    result
}

fn fetch_request(wanted: Option<&[String]>) -> Result<Vec<u8>, String> {
    let Some(wanted) = wanted else { return Ok(b"0000".to_vec()); };
    if wanted.is_empty() || wanted.len() > 2000 { return Err("SSH_WANT_LIMIT".into()); }
    let mut unique = std::collections::BTreeSet::new();
    for oid in wanted {
        if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("SSH_WANT_INVALID".into()); }
        unique.insert(oid.to_ascii_lowercase());
    }
    let mut request = String::new();
    for oid in unique { let line = format!("want {oid}\n"); request.push_str(&format!("{:04x}{line}", line.len() + 4)); }
    request.push_str("00000009done\n");
    Ok(request.into_bytes())
}

fn launch_files(pin: &Path, identity: &Path, directory: &Path) -> Result<tempfile::TempDir, String> {
    use std::os::unix::fs::PermissionsExt;
    let pin = pin.canonicalize().map_err(|_| "SSH_PIN_UNAVAILABLE")?;
    let identity = identity.canonicalize().map_err(|_| "SSH_KEY_UNAVAILABLE")?;
    if !pin.is_file() { return Err("SSH_PIN_UNAVAILABLE".into()); }
    if !identity.is_file() { return Err("SSH_KEY_UNAVAILABLE".into()); }
    let directory = tempfile::Builder::new().prefix("ssh-launch-")
        .permissions(std::fs::Permissions::from_mode(0o700)).tempdir_in(directory)
        .map_err(|_| "SSH_PRIVATE_DIRECTORY_FAILED")?;
    // Kernel symlink resolution treats targets literally. OpenSSH sees only
    // fixed relative names, never %-tokens, ${variables}, or a key sidecar.
    // Private key contents are not copied. Original-file replacement races still
    // need an ownership/lifecycle design before production promotion.
    std::os::unix::fs::symlink(pin, directory.path().join("known_hosts"))
        .map_err(|_| "SSH_PRIVATE_DIRECTORY_FAILED")?;
    std::os::unix::fs::symlink(identity, directory.path().join("identity"))
        .map_err(|_| "SSH_PRIVATE_DIRECTORY_FAILED")?;
    Ok(directory)
}

fn advertisement(url: &str, pin: &Path, identity: &Path, directory: &Path,
                 deadline: Duration, cancelled: Arc<AtomicBool>, want: Option<&[String]>) -> Result<tempfile::NamedTempFile, String> {
    let request_bytes = fetch_request(want)?;
    let endpoint = ssh_endpoint::parse(url)?;
    let user = endpoint.user.as_ref().ok_or("SSH_USER_REQUIRED")?;
    if cancelled.load(Ordering::Acquire) { return Err("SSH_CANCELLED".into()); }
    // These files are fixture-owned. Production promotion still requires pin
    // export, ownership validation, native authorization and lifecycle design.
    if !pin.is_absolute() || !identity.is_absolute() || !directory.is_absolute() {
        return Err("SSH_PRIVATE_PATH_REQUIRED".into());
    }
    let launch = launch_files(pin, identity, directory)?;
    let mut command = Command::new("/usr/bin/ssh");
    command.env_clear().env("HOME", directory).env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C").current_dir(launch.path())
        .args(["-F", "/dev/null", "-T", "-i", "identity"])
        .args(["-o", "UserKnownHostsFile=known_hosts"]);
    for option in [
        "BatchMode=yes", "StrictHostKeyChecking=yes", "GlobalKnownHostsFile=/dev/null",
        "KnownHostsCommand=none", "UpdateHostKeys=no", "VerifyHostKeyDNS=no",
        "IdentityAgent=none", "IdentitiesOnly=yes", "CertificateFile=none",
        "PreferredAuthentications=publickey", "PasswordAuthentication=no",
        "KbdInteractiveAuthentication=no", "HostbasedAuthentication=no",
        "ForwardAgent=no", "ForwardX11=no", "ClearAllForwardings=yes",
        "ProxyCommand=none", "ProxyJump=none", "ControlMaster=no", "ControlPath=none",
        "PermitLocalCommand=no", "RequestTTY=no", "ConnectionAttempts=1",
        "ConnectTimeout=2", "LogLevel=ERROR",
        "HostKeyAlgorithms=ssh-ed25519,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,rsa-sha2-512,rsa-sha2-256",
    ] { command.args(["-o", option]); }
    let capture = tempfile::Builder::new().prefix("ssh-output-").tempfile_in(directory)
        .map_err(|_| "SSH_SPOOL_FAILED")?;
    let limit = if want.is_some() { 128 * 1024 * 1024 + OUTPUT_LIMIT as u64 } else { OUTPUT_LIMIT as u64 };
    let mut child = command.args(["-p", &endpoint.port.to_string(), "-l", user])
        .arg(&endpoint.host).arg(endpoint.upload_command())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|_| "SSH_START_FAILED")?;
    let input = child.stdin.take().ok_or("SSH_PIPE_FAILED")?;
    let abort = Arc::new(AtomicBool::new(false));
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().ok_or("SSH_PIPE_FAILED")?;
    let stderr = child.stderr.take().ok_or("SSH_PIPE_FAILED")?;
    let stdout_abort = abort.clone();
    let transfer_cancelled = cancelled.clone();
    let request = (url.to_string(), want.map(<[String]>::to_vec), request_bytes);
    let stderr_overflow = overflow.clone();
    let output = std::thread::spawn(move || negotiate(stdout, input, capture, request, limit, transfer_cancelled, stdout_abort));
    let errors = std::thread::spawn(move || drain(stderr, stderr_overflow));
    let started = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Acquire) { break Err("SSH_CANCELLED".to_string()); }
        if overflow.load(Ordering::Acquire) { break Err("SSH_OUTPUT_LIMIT".to_string()); }
        if abort.load(Ordering::Acquire) { break Err("SSH_PROTOCOL_ABORTED".to_string()); }
        if started.elapsed() >= deadline { break Err("SSH_TIMEOUT".to_string()); }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break Err("SSH_WAIT_FAILED".to_string()),
        }
    };
    if status.is_err() { let _ = child.kill(); }
    let _ = child.wait();
    let bytes = output.join().map_err(|_| "SSH_READER_FAILED")?;
    let diagnostic = errors.join().map_err(|_| "SSH_READER_FAILED")?;
    // Never surface peer-controlled stderr, which can contain reflected secrets.
    drop(diagnostic?);
    if let Err(error) = &status {
        if error != "SSH_PROTOCOL_ABORTED" { return Err(error.clone()); }
    }
    if status.as_ref().is_ok_and(|status| !status.success()) { return Err("SSH_CONNECTION_FAILED".into()); }
    let capture = bytes?;
    if overflow.load(Ordering::Acquire) { return Err("SSH_OUTPUT_LIMIT".into()); }
    status?;
    Ok(capture)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_key_aliases_are_private_ephemeral_and_leave_originals_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("keys %h ${HOME} ' ");
        std::fs::create_dir(&source).unwrap();
        let key = source.join("identity %p");
        let pin = source.join("pin %d");
        std::fs::write(&key, "synthetic key bytes").unwrap();
        std::fs::write(&pin, "synthetic pin bytes").unwrap();
        let launch = launch_files(&pin, &key, root.path()).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(launch.path()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::read(launch.path().join("identity")).unwrap(), b"synthetic key bytes");
        assert_eq!(std::fs::read(launch.path().join("known_hosts")).unwrap(), b"synthetic pin bytes");
        assert!(std::fs::symlink_metadata(launch.path().join("identity")).unwrap().file_type().is_symlink());
        assert!(!launch.path().join("identity-cert.pub").exists());
        let path = launch.path().to_owned(); drop(launch); assert!(!path.exists());
        assert_eq!(std::fs::read(&key).unwrap(), b"synthetic key bytes");
        assert_eq!(std::fs::read(&pin).unwrap(), b"synthetic pin bytes");
        assert_eq!(launch_files(&pin, &source, root.path()).unwrap_err(), "SSH_KEY_UNAVAILABLE");
        assert_eq!(launch_files(&source, &key, root.path()).unwrap_err(), "SSH_PIN_UNAVAILABLE");
    }
    #[test]
    fn multiple_wants_are_bounded_normalized_and_deduplicated() {
        let first = "ab".repeat(20);
        let second = "cd".repeat(20);
        let request = fetch_request(Some(&[second.clone(), first.to_uppercase(), first.clone()])).unwrap();
        assert_eq!(request, format!("0032want {first}\n0032want {second}\n00000009done\n").into_bytes());
        assert_eq!(fetch_request(Some(&[])).unwrap_err(), "SSH_WANT_LIMIT");
        assert_eq!(fetch_request(Some(&vec![first; 2001])).unwrap_err(), "SSH_WANT_LIMIT");
        let many = (1..=2000).map(|n| format!("{n:040x}")).collect::<Vec<_>>();
        assert_eq!(fetch_request(Some(&many)).unwrap().len(), 100013);
        assert_eq!(fetch_request(None).unwrap(), b"0000");
    }
    #[test]
    fn spool_streams_and_rewinds_but_removes_partial_output_on_limit_or_read_failure() {
        let directory = tempfile::tempdir().unwrap();
        let data = vec![17; 256 * 1024];
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        let path = file.path().to_owned();
        let stop = Arc::new(AtomicBool::new(false));
        let mut capture = spool(data.as_slice(), file, data.len() as u64, stop.clone()).unwrap();
        assert!(!stop.load(Ordering::Acquire));
        assert_eq!(capture.stream_position().unwrap(), 0);
        let mut actual = Vec::new(); capture.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, data);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(capture.as_file().metadata().unwrap().permissions().mode() & 0o777, 0o600);
        drop(capture); assert!(!path.exists());
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        let path = file.path().to_owned();
        assert_eq!(spool(data.as_slice(), file, 8193, stop.clone()).unwrap_err(), "SSH_OUTPUT_LIMIT");
        assert!(stop.load(Ordering::Acquire)); assert!(!path.exists());
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic failure"))
            }
        }
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        let path = file.path().to_owned(); stop.store(false, Ordering::Release);
        assert_eq!(spool(Broken, file, 8193, stop.clone()).unwrap_err(), "SSH_READ_FAILED");
        assert!(stop.load(Ordering::Acquire)); assert!(!path.exists());
    }
    #[test]
    fn malformed_want_cannot_inject_protocol_or_start_process() {
        for oid in ["", "HEAD", "--all", "1111111111111111111111111111111111111111\n0000",
                    "gggggggggggggggggggggggggggggggggggggggg", "111111111111111111111111111111111111111"] {
            assert_eq!(advertisement("fixture@host.invalid:repo", Path::new("/missing"),
                Path::new("/missing"), Path::new("/missing"), Duration::from_secs(1),
                Arc::new(AtomicBool::new(false)), Some(&[oid.into()])).map(|_| ()), Err("SSH_WANT_INVALID".into()));
        }
    }
    #[test]
    fn missing_user_refused_before_process_start() {
        assert_eq!(advertisement("host.invalid:repo", Path::new("/missing"),
            Path::new("/missing"), Path::new("/missing"), Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)), None).map(|_| ()),
            Err("SSH_USER_REQUIRED".into()));
    }

    #[test]
    fn pre_cancelled_request_never_spawns() {
        assert_eq!(advertisement("fixture@host.invalid:repo", Path::new("/missing"),
            Path::new("/missing"), Path::new("/missing"), Duration::from_secs(1),
            Arc::new(AtomicBool::new(true)), None).map(|_| ()), Err("SSH_CANCELLED".into()));
    }

    /// Driven by scripts/test-ssh-engine.py with only synthetic fixture paths.
    #[test]
    #[ignore = "requires explicit isolated loopback fixture"]
    fn fixture_client() {
        let config: serde_json::Value = serde_json::from_str(
            &std::env::var("BRANCHDESK_SSH_FIXTURE").expect("fixture required")).unwrap();
        let get = |name: &str| config[name].as_str().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = config["cancelAfterMs"].as_u64().map(|delay| {
            let cancelled = cancelled.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(delay));
                cancelled.store(true, Ordering::Release);
            })
        });
        let deadline = Duration::from_millis(config["deadlineMs"].as_u64().unwrap());
        let wants: Option<Vec<String>> = if let Some(values) = config["wantOids"].as_array() {
            Some(values.iter().map(|value| value.as_str().unwrap().to_owned()).collect())
        } else { config["wantOid"].as_str().map(|oid| vec![oid.into()]) };
        let want = wants.as_deref();
        let result = if config["trustStore"].as_bool() == Some(true) {
            use base64::Engine;
            let dir = tempfile::Builder::new().prefix("trust % ${HOME} ' ")
                .tempdir_in(Path::new(get("directory")).canonicalize().unwrap()).unwrap();
            let endpoint = ssh_endpoint::parse(get("url")).unwrap();
            let scope = crate::ssh_trust::Scope::new(&endpoint.host, endpoint.port).unwrap();
            let raw = std::fs::read_to_string(get("pin")).unwrap();
            let mut state = crate::ssh_trust::read(dir.path()).unwrap();
            if let Some(blob) = raw.split_whitespace().nth(2) {
                let bytes = base64::engine::general_purpose::STANDARD.decode(blob).unwrap();
                let key = crate::ssh_trust::Key::from_blob(&bytes).unwrap();
                state = crate::ssh_trust::trust_first(dir.path(), &state.revision, scope, key).unwrap();
            }
            let result = trusted_advertisement(get("url"), (dir.path(), &state.revision),
                Path::new(get("identity")), Path::new(get("directory")), deadline, cancelled, want);
            assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry|
                entry.unwrap().file_name().to_string_lossy().starts_with("ssh-pin-")));
            result
        } else {
            advertisement(get("url"), Path::new(get("pin")), Path::new(get("identity")),
                Path::new(get("directory")), deadline, cancelled, want)
        };
        let result = result.and_then(|mut capture| {
            let received = capture.as_file().metadata().unwrap().len();
            if let Some(path) = config["metrics"].as_str() {
                std::fs::write(path, serde_json::to_vec(&received).unwrap()).unwrap();
            }
            let advertisement = crate::network_worker::read_ssh_advertisement(get("url"), capture.as_file_mut())?;
            if let Some(path) = config["advertisement"].as_str() {
                std::fs::write(path, serde_json::to_vec(&advertisement).unwrap()).unwrap();
            }
            if let Some(oids) = want {
                if oids.iter().any(|oid| !advertisement.refs.iter().any(|reference| reference.oid.eq_ignore_ascii_case(oid))) {
                    return Err("SSH_WANT_NOT_ADVERTISED".into());
                }
                let pack_start = capture.stream_position().unwrap();
                if config["corruptPack"].as_bool() == Some(true) {
                    capture.seek(SeekFrom::End(-1)).unwrap();
                    let mut byte = [0]; capture.read_exact(&mut byte).unwrap();
                    capture.seek(SeekFrom::End(-1)).unwrap(); capture.write_all(&[byte[0] ^ 1]).unwrap();
                    capture.seek(SeekFrom::Start(pack_start)).unwrap();
                }
                let selected = crate::network_worker::Advertisement {
                    endpoint: advertisement.endpoint,
                    refs: advertisement.refs.into_iter().filter(|reference|
                        oids.iter().any(|oid| reference.oid.eq_ignore_ascii_case(oid))).collect(),
                };
                crate::network_worker::index_ssh_fixture_pack(capture.as_file_mut(), Path::new(get("quarantine")), &selected)?;
                Ok(Vec::new())
            } else {
                if capture.stream_position().unwrap() != received { return Err("SSH_ADVERTISEMENT_TRAILING".into()); }
                capture.seek(SeekFrom::Start(0)).map_err(|_| "SSH_SPOOL_FAILED")?;
                let mut bytes = Vec::new();
                capture.read_to_end(&mut bytes).map_err(|_| "SSH_READ_FAILED")?;
                Ok(bytes)
            }
        });
        assert!(!std::fs::read_dir(get("directory")).unwrap().any(|entry|
            { let name = entry.unwrap().file_name(); let name = name.to_string_lossy();
              name.starts_with("ssh-output-") || name.starts_with("ssh-launch-") }));
        if let Some(thread) = cancellation { thread.join().unwrap(); }
        std::fs::write(get("result"), serde_json::to_vec(&result).unwrap()).unwrap();
    }
}
