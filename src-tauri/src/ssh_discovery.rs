//! Unauthenticated host-key observation only. No credentials or trust writes.
//! Test-only until native verification and worker integration are complete.
use crate::{ssh_endpoint, ssh_trust::{Key, Scope}};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{path::Path, process::{Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

#[derive(Debug)]
pub(super) struct Observed { pub scope: Scope, pub key: Key }

fn parse(url: &str, family: &str, bytes: &[u8]) -> Result<Observed, String> {
    let endpoint = ssh_endpoint::parse(url)?;
    if bytes.len() > 16 * 1024 { return Err("SSH_SCAN_LIMIT".into()); }
    let text = std::str::from_utf8(bytes).map_err(|_| "SSH_SCAN_INVALID")?;
    let expected = if endpoint.port == 22 { endpoint.host.clone() }
        else { format!("[{}]:{}", endpoint.host, endpoint.port) };
    let mut found = None;
    for line in text.lines() {
        if line.is_empty() { continue; }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != expected { return Err("SSH_SCAN_SCOPE_MISMATCH".into()); }
        let bytes = STANDARD.decode(fields[2]).map_err(|_| "SSH_SCAN_INVALID")?;
        let key = Key::from_blob(&bytes)?;
        if key.algorithm() != fields[1] || STANDARD.encode(&bytes) != fields[2] {
            return Err("SSH_SCAN_INVALID".into());
        }
        let matches = match family {
            "ed25519" => key.algorithm() == "ssh-ed25519",
            "rsa" => key.algorithm() == "ssh-rsa",
            "ecdsa" => key.algorithm().starts_with("ecdsa-sha2-nistp"),
            _ => false,
        };
        if !matches { return Err("SSH_SCAN_ALGORITHM_MISMATCH".into()); }
        if found.replace(key).is_some() { return Err("SSH_SCAN_AMBIGUOUS".into()); }
    }
    Ok(Observed { scope: Scope::new(&endpoint.host, endpoint.port)?,
                  key: found.ok_or("SSH_HOST_KEY_MISSING")? })
}

fn scan(url: &str, family: &str, directory: &Path, deadline: Duration,
        cancelled: Arc<AtomicBool>) -> Result<Observed, String> {
    let endpoint = ssh_endpoint::parse(url)?;
    if !matches!(family, "rsa" | "ed25519" | "ecdsa") { return Err("SSH_SCAN_ALGORITHM_INVALID".into()); }
    if cancelled.load(Ordering::Acquire) { return Err("SSH_CANCELLED".into()); }
    let mut child = Command::new("/usr/bin/ssh-keyscan").env_clear()
        .env("HOME", directory).env("PATH", "/usr/bin:/bin").env("LC_ALL", "C")
        .current_dir(directory).args(["-q", "-T", "2", "-t", family, "-p", &endpoint.port.to_string()])
        .arg(&endpoint.host).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|_| "SSH_SCAN_START_FAILED")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().ok_or("SSH_SCAN_PIPE_FAILED")?;
    let stderr = child.stderr.take().ok_or("SSH_SCAN_PIPE_FAILED")?;
    let flag = overflow.clone();
    let output = std::thread::spawn(move || crate::ssh_process::drain(stdout, flag));
    let flag = overflow.clone();
    let errors = std::thread::spawn(move || crate::ssh_process::drain(stderr, flag));
    let started = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Acquire) { break Err("SSH_CANCELLED"); }
        if overflow.load(Ordering::Acquire) { break Err("SSH_SCAN_LIMIT"); }
        if started.elapsed() >= deadline { break Err("SSH_TIMEOUT"); }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break Err("SSH_SCAN_WAIT_FAILED"),
        }
    };
    if status.is_err() { let _ = child.kill(); }
    let _ = child.wait();
    let bytes = output.join().map_err(|_| "SSH_SCAN_READ_FAILED")?;
    let diagnostic = errors.join().map_err(|_| "SSH_SCAN_READ_FAILED")?;
    drop(diagnostic?);
    if overflow.load(Ordering::Acquire) { return Err("SSH_SCAN_LIMIT".into()); }
    if !status?.success() { return Err("SSH_HOST_KEY_UNAVAILABLE".into()); }
    parse(url, family, &bytes?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn encoded() -> String {
        let mut blob = Vec::new();
        for part in [b"ssh-ed25519".as_slice(), &[7;32]] {
            blob.extend_from_slice(&(part.len() as u32).to_be_bytes()); blob.extend_from_slice(part);
        }
        STANDARD.encode(blob)
    }
    #[test]
    fn discovered_key_is_scoped_structurally_valid_and_never_implicitly_trusted() {
        let url = "ssh://fixture@example.invalid:2222/repo";
        let line = format!("[example.invalid]:2222 ssh-ed25519 {}\n", encoded());
        let observed = parse(url, "ed25519", line.as_bytes()).unwrap();
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let state = crate::ssh_trust::read(dir.path()).unwrap();
        assert_eq!(state.verify(&observed.scope, &observed.key).unwrap(), crate::ssh_trust::Verification::Unknown);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        for bad in [line.replace(":2222", ":2223"), line.replace("example.invalid", "other.invalid"),
                    line.replace("ssh-ed25519", "ssh-rsa"), line.repeat(2), line.clone()+"extra\n",
                    line.replace(&encoded(), "not-base64")] {
            assert!(parse(url, "ed25519", bad.as_bytes()).is_err());
        }
        assert!(parse(url, "rsa", line.as_bytes()).is_err());
        assert!(parse(url, "ed25519", &vec![b'x';16385]).is_err());
        assert_eq!(parse(url,"ed25519",b"").unwrap_err(), "SSH_HOST_KEY_MISSING");
    }
    #[test]
    fn cancelled_or_invalid_scan_never_starts() {
        assert_eq!(scan("host.invalid:repo", "ed25519", Path::new("/missing"), Duration::from_secs(1), Arc::new(AtomicBool::new(true))).unwrap_err(), "SSH_CANCELLED");
        assert_eq!(scan("host.invalid:repo", "rsa,ed25519", Path::new("/missing"), Duration::from_secs(1), Arc::new(AtomicBool::new(false))).unwrap_err(), "SSH_SCAN_ALGORITHM_INVALID");
    }
    #[test]
    #[ignore = "explicit isolated loopback discovery fixture"]
    fn fixture_scan() {
        let value: serde_json::Value = serde_json::from_str(&std::env::var("BRANCHDESK_SSH_SCAN_FIXTURE").unwrap()).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread = value["cancelMs"].as_u64().map(|ms| { let flag=cancelled.clone(); std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(ms)); flag.store(true, Ordering::Release); }) });
        let result = scan(value["url"].as_str().unwrap(), value["family"].as_str().unwrap(), Path::new(value["directory"].as_str().unwrap()), Duration::from_millis(value["deadlineMs"].as_u64().unwrap()), cancelled)
            .and_then(|observed| Ok(serde_json::json!({"scope":observed.scope,"algorithm":observed.key.algorithm(),"fingerprint":observed.key.fingerprint()?})));
        if let Some(thread)=thread { thread.join().unwrap(); }
        std::fs::write(value["result"].as_str().unwrap(),serde_json::to_vec(&result).unwrap()).unwrap();
    }
}
