//! Host-key pinning prototype, not yet connected to native consent or SSH transport.
//! Public-key structure/fingerprint checks do not verify a handshake signature.
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
    Engine,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
const FILE: &str = "ssh-host-pins.json";
const LIMIT: u64 = 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    host: String,
    port: u16,
}
impl Scope {
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let authority = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.into()
        };
        let endpoint = crate::ssh_endpoint::parse(&format!("ssh://{authority}:{port}/probe"))?;
        if endpoint.user.is_some() || endpoint.path != "/probe" || endpoint.port != port {
            return Err("INVALID_SSH_TRUST_SCOPE".into());
        }
        Ok(Self {
            host: endpoint.host,
            port: endpoint.port,
        })
    }
    fn validate(&self) -> Result<()> {
        if Self::new(&self.host, self.port)? != *self {
            return Err("INVALID_SSH_TRUST_SCOPE".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    algorithm: String,
    blob: String,
}
struct Wire<'a>(&'a [u8]);
impl<'a> Wire<'a> {
    fn string(&mut self) -> Result<&'a [u8]> {
        let n = self.0.get(..4).ok_or("INVALID_SSH_HOST_KEY")?;
        let n = u32::from_be_bytes(n.try_into().unwrap()) as usize;
        let value = self
            .0
            .get(4..4usize.checked_add(n).ok_or("INVALID_SSH_HOST_KEY")?)
            .ok_or("INVALID_SSH_HOST_KEY")?;
        self.0 = &self.0[4 + n..];
        Ok(value)
    }
}
fn mpint(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.is_empty() || bytes[0] & 0x80 != 0 {
        return Err("INVALID_SSH_HOST_KEY".into());
    }
    if bytes[0] == 0 {
        if bytes.len() < 2 || bytes[1] & 0x80 == 0 {
            return Err("INVALID_SSH_HOST_KEY".into());
        }
        Ok(&bytes[1..])
    } else {
        Ok(bytes)
    }
}
impl Key {
    #[cfg(target_os = "macos")]
    pub fn algorithm(&self) -> &str { &self.algorithm }
    pub fn from_blob(blob: &[u8]) -> Result<Self> {
        if blob.len() > 8192 {
            return Err("SSH_HOST_KEY_LIMIT".into());
        }
        let mut wire = Wire(blob);
        let algorithm = std::str::from_utf8(wire.string()?).map_err(|_| "INVALID_SSH_HOST_KEY")?;
        match algorithm {
            "ssh-ed25519" => {
                if wire.string()?.len() != 32 {
                    return Err("INVALID_SSH_HOST_KEY".into());
                }
            }
            "ssh-rsa" => {
                let e = mpint(wire.string()?)?;
                let n = mpint(wire.string()?)?;
                if e.len() > 8
                    || e.last().is_none_or(|b| b & 1 == 0)
                    || e.len() == 1 && e[0] < 3
                    || n.len() < 256
                    || n.len() > 1024
                    || ((n.len() - 1) * 8 + 8 - n[0].leading_zeros() as usize) < 2048
                    || n.last().is_none_or(|b| b & 1 == 0)
                {
                    return Err("UNSUPPORTED_SSH_RSA_KEY".into());
                }
            }
            "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => {
                let curve = algorithm.strip_prefix("ecdsa-sha2-").unwrap();
                if wire.string() != Ok(curve.as_bytes()) {
                    return Err("INVALID_SSH_HOST_KEY".into());
                }
                let point = wire.string()?;
                let length = match curve {
                    "nistp256" => 65,
                    "nistp384" => 97,
                    _ => 133,
                };
                if point.len() != length || point[0] != 4 {
                    return Err("INVALID_SSH_HOST_KEY".into());
                }
            }
            _ => return Err("UNSUPPORTED_SSH_HOST_KEY".into()),
        }
        if !wire.0.is_empty() {
            return Err("INVALID_SSH_HOST_KEY".into());
        }
        Ok(Self {
            algorithm: algorithm.into(),
            blob: STANDARD.encode(blob),
        })
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        if self.blob.len() > 11000 {
            return Err("SSH_HOST_KEY_LIMIT".into());
        }
        let bytes = STANDARD
            .decode(&self.blob)
            .map_err(|_| "INVALID_SSH_HOST_KEY")?;
        if Self::from_blob(&bytes)? != *self {
            return Err("INVALID_SSH_HOST_KEY".into());
        }
        Ok(bytes)
    }
    pub fn fingerprint(&self) -> Result<String> {
        Ok(format!(
            "SHA256:{}",
            STANDARD_NO_PAD.encode(Sha256::digest(self.bytes()?))
        ))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    scope: Scope,
    key: Key,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    pins: Vec<Pin>,
}
#[derive(Debug)]
pub struct State {
    pub revision: String,
    pub pins: Vec<Pin>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Verification {
    Unknown,
    Match,
    Changed { expected: String, presented: String },
}
impl State {
    pub fn verify(&self, scope: &Scope, key: &Key) -> Result<Verification> {
        scope.validate()?;
        let presented = key.fingerprint()?;
        match self.pins.iter().find(|p| &p.scope == scope) {
            None => Ok(Verification::Unknown),
            Some(pin) if &pin.key == key => Ok(Verification::Match),
            Some(pin) => Ok(Verification::Changed {
                expected: pin.key.fingerprint()?,
                presented,
            }),
        }
    }
}

/// Export one already-approved pin for a frozen store revision. The caller must
/// retain the handle until the child exits; dropping it removes only this file.
/// This does not discover or approve a host key.
pub fn export_pin(dir: &Path, expected: &str, scope: &Scope) -> Result<tempfile::NamedTempFile> {
    scope.validate()?;
    let state = read(dir)?;
    if state.revision != expected {
        return Err("SSH_TRUST_STALE".into());
    }
    let pin = state.pins.iter().find(|pin| &pin.scope == scope)
        .ok_or("SSH_HOST_UNTRUSTED")?;
    let host = if scope.port == 22 {
        scope.host.clone()
    } else {
        format!("[{}]:{}", scope.host, scope.port)
    };
    let mut output = tempfile::Builder::new().prefix("ssh-pin-").tempfile_in(dir)
        .map_err(|_| "SSH_PIN_EXPORT_FAILED")?;
    writeln!(output, "{} {} {}", host, pin.key.algorithm, pin.key.blob)
        .map_err(|_| "SSH_PIN_EXPORT_FAILED")?;
    output.as_file().sync_all().map_err(|_| "SSH_PIN_EXPORT_FAILED")?;
    Ok(output)
}
fn check(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err("SSH_TRUST_SYMLINK".into()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("SSH_TRUST_ACCESS_FAILED".into()),
        }
    }
    Ok(())
}
fn bytes(dir: &Path) -> Result<Option<Vec<u8>>> {
    let path = dir.join(FILE);
    check(&path)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("SSH_TRUST_ACCESS_FAILED".into()),
        Ok(m) if !m.is_file() || m.len() > LIMIT => return Err("INVALID_SSH_TRUST_FILE".into()),
        Ok(_) => {}
    }
    let mut b = Vec::new();
    fs::File::open(path)
        .map_err(|_| "SSH_TRUST_READ_FAILED")?
        .take(LIMIT + 1)
        .read_to_end(&mut b)
        .map_err(|_| "SSH_TRUST_READ_FAILED")?;
    if b.len() as u64 > LIMIT {
        return Err("SSH_TRUST_LIMIT".into());
    }
    Ok(Some(b))
}
fn revision(bytes: &Option<Vec<u8>>) -> String {
    bytes
        .as_ref()
        .map(|b| format!("{:x}", Sha256::digest(b)))
        .unwrap_or("missing".into())
}
pub fn read(dir: &Path) -> Result<State> {
    let raw = bytes(dir)?;
    let revision = revision(&raw);
    let Some(raw) = raw else {
        return Ok(State {
            revision,
            pins: vec![],
        });
    };
    let record: Record = serde_json::from_slice(&raw).map_err(|_| "INVALID_SSH_TRUST_FILE")?;
    if record.version != 1 || record.pins.len() > 256 {
        return Err("INVALID_SSH_TRUST_FILE".into());
    }
    let mut scopes = std::collections::HashSet::new();
    for p in &record.pins {
        p.scope.validate()?;
        p.key.bytes()?;
        if !scopes.insert((&p.scope.host, p.scope.port)) {
            return Err("DUPLICATE_SSH_TRUST_SCOPE".into());
        }
    }
    Ok(State {
        revision,
        pins: record.pins,
    })
}
struct HeldLock(fs::File);
impl Drop for HeldLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn update(
    dir: &Path,
    expected: &str,
    change: impl FnOnce(&mut State) -> Result<()>,
) -> Result<State> {
    if expected.len() > 64 {
        return Err("INVALID_SSH_TRUST_REVISION".into());
    }
    check(dir)?;
    fs::create_dir_all(dir).map_err(|_| "SSH_TRUST_ACCESS_FAILED")?;
    let lockpath = dir.join("ssh-host-pins.lock");
    check(&lockpath)?;
    if fs::symlink_metadata(&lockpath).is_ok_and(|m| !m.is_file()) {
        return Err("INVALID_SSH_TRUST_LOCK".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(lockpath)
        .map_err(|_| "SSH_TRUST_LOCK_FAILED")?;
    lock.try_lock().map_err(|_| "SSH_TRUST_BUSY")?;
    let _lock = HeldLock(lock);
    let mut state = read(dir)?;
    if state.revision != expected {
        return Err("STALE_SSH_TRUST".into());
    }
    change(&mut state)?;
    if state.pins.len() > 256 {
        return Err("SSH_TRUST_LIMIT".into());
    }
    state
        .pins
        .sort_by(|a, b| (&a.scope.host, a.scope.port).cmp(&(&b.scope.host, b.scope.port)));
    let output = serde_json::to_vec(&Record {
        version: 1,
        pins: state.pins,
    })
    .map_err(|_| "SSH_TRUST_WRITE_FAILED")?;
    if output.len() as u64 > LIMIT {
        return Err("SSH_TRUST_LIMIT".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|_| "SSH_TRUST_WRITE_FAILED")?;
    temp.write_all(&output)
        .map_err(|_| "SSH_TRUST_WRITE_FAILED")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "SSH_TRUST_SYNC_FAILED")?;
    if revision(&bytes(dir)?) != expected {
        return Err("STALE_SSH_TRUST".into());
    }
    checkpoint(dir, 0);
    temp.persist(dir.join(FILE))
        .map_err(|_| "SSH_TRUST_WRITE_FAILED")?;
    #[cfg(unix)]
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|_| "SSH_TRUST_SYNC_FAILED")?;
    checkpoint(dir, 1);
    read(dir)
}
// Caller must obtain native consent for this exact host/key before calling.
pub fn trust_first(dir: &Path, expected: &str, scope: Scope, key: Key) -> Result<State> {
    scope.validate()?;
    key.bytes()?;
    update(dir, expected, |state| {
        match state.verify(&scope, &key)? {
            Verification::Unknown => state.pins.push(Pin { scope, key }),
            Verification::Match => {}
            Verification::Changed { .. } => return Err("SSH_HOST_KEY_CHANGED".into()),
        }
        Ok(())
    })
}
pub fn forget(dir: &Path, expected: &str, scope: &Scope, fingerprint: &str) -> Result<State> {
    scope.validate()?;
    update(dir, expected, |state| {
        let index = state
            .pins
            .iter()
            .position(|p| &p.scope == scope)
            .ok_or("SSH_TRUST_NOT_FOUND")?;
        if state.pins[index].key.fingerprint() != Ok(fingerprint.into()) {
            return Err("SSH_TRUST_KEY_MISMATCH".into());
        }
        state.pins.remove(index);
        Ok(())
    })
}
fn checkpoint(dir: &Path, step: usize) {
    #[cfg(not(test))]
    let _ = (dir, step);
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_TRUST_DIR").as_deref() == Some(dir.as_os_str())
        && std::env::var("BRANCHDESK_TRUST_STEP").ok() == Some(step.to_string())
    {
        fs::write(dir.join("ready"), "ready").unwrap();
        loop {
            std::thread::park()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key(byte: u8) -> Key {
        let mut blob = Vec::new();
        for field in [b"ssh-ed25519".as_slice(), &[byte; 32]] {
            blob.extend_from_slice(&(field.len() as u32).to_be_bytes());
            blob.extend_from_slice(field);
        }
        Key::from_blob(&blob).unwrap()
    }
    fn dir() -> tempfile::TempDir {
        {
            let root = std::env::temp_dir();
            let root = if cfg!(windows) { root } else { root.canonicalize().unwrap() };
            tempfile::tempdir_in(root).unwrap()
        }
    }
    fn scope() -> Scope {
        Scope::new("EXAMPLE.invalid", 22).unwrap()
    }
    #[test]
    fn export_is_single_scoped_private_revision_checked_and_ephemeral() {
        let d = dir();
        let initial = read(d.path()).unwrap();
        assert_eq!(export_pin(d.path(), &initial.revision, &scope()).unwrap_err(), "SSH_HOST_UNTRUSTED");
        let first = trust_first(d.path(), &initial.revision, scope(), key(1)).unwrap();
        let other = Scope::new("::1", 2222).unwrap();
        let state = trust_first(d.path(), &first.revision, other.clone(), key(2)).unwrap();
        assert_eq!(export_pin(d.path(), &first.revision, &scope()).unwrap_err(), "SSH_TRUST_STALE");
        assert_eq!(export_pin(d.path(), &state.revision, &Scope::new("example.invalid", 2222).unwrap()).unwrap_err(), "SSH_HOST_UNTRUSTED");
        let pin = export_pin(d.path(), &state.revision, &scope()).unwrap();
        let content = fs::read_to_string(pin.path()).unwrap();
        assert_eq!(content, format!("example.invalid ssh-ed25519 {}\n", key(1).blob));
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(pin.as_file().metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
        let path = pin.path().to_owned();
        drop(pin);
        assert!(!path.exists());
        let pin = export_pin(d.path(), &state.revision, &other).unwrap();
        assert_eq!(fs::read_to_string(pin.path()).unwrap(), format!("[::1]:2222 ssh-ed25519 {}\n", key(2).blob));
        assert_eq!(read(d.path()).unwrap().revision, state.revision);
        fs::write(d.path().join(FILE), "corrupt").unwrap();
        assert_eq!(export_pin(d.path(), &state.revision, &other).unwrap_err(), "INVALID_SSH_TRUST_FILE");
    }
    #[test]
    fn pins_scope_match_changed_stale_and_explicit_forget() {
        let d = dir();
        let initial = read(d.path()).unwrap();
        assert_eq!(
            initial.verify(&scope(), &key(1)).unwrap(),
            Verification::Unknown
        );
        let saved = trust_first(d.path(), &initial.revision, scope(), key(1)).unwrap();
        assert_eq!(
            saved.verify(&scope(), &key(1)).unwrap(),
            Verification::Match
        );
        for scope in [
            Scope::new("elsewhere.invalid", 22).unwrap(),
            Scope::new("example.invalid", 2222).unwrap(),
        ] {
            assert_eq!(
                saved.verify(&scope, &key(1)).unwrap(),
                Verification::Unknown
            );
        }
        assert!(matches!(
            saved.verify(&scope(), &key(2)).unwrap(),
            Verification::Changed { .. }
        ));
        let before = fs::read(d.path().join(FILE)).unwrap();
        assert_eq!(
            trust_first(d.path(), &saved.revision, scope(), key(2)).unwrap_err(),
            "SSH_HOST_KEY_CHANGED"
        );
        assert_eq!(
            trust_first(d.path(), "missing", scope(), key(1)).unwrap_err(),
            "STALE_SSH_TRUST"
        );
        assert_eq!(
            forget(
                d.path(),
                &saved.revision,
                &scope(),
                &key(2).fingerprint().unwrap()
            )
            .unwrap_err(),
            "SSH_TRUST_KEY_MISMATCH"
        );
        assert_eq!(fs::read(d.path().join(FILE)).unwrap(), before);
        let same = trust_first(d.path(), &saved.revision, scope(), key(1)).unwrap();
        assert_eq!(same.revision, saved.revision);
        let cleared = forget(
            d.path(),
            &same.revision,
            &scope(),
            &key(1).fingerprint().unwrap(),
        )
        .unwrap();
        assert!(cleared.pins.is_empty());
    }
    #[test]
    fn corrupt_duplicate_noncanonical_and_oversized_records_fail_closed() {
        let d = dir();
        for bytes in [
            b"bad".to_vec(),
            br#"{"version":2,"pins":[]}"#.to_vec(),
            br#"{"version":1,"pins":[],"extra":true}"#.to_vec(),
            vec![b' '; LIMIT as usize + 1],
        ] {
            fs::write(d.path().join(FILE), &bytes).unwrap();
            assert!(read(d.path()).is_err());
            assert!(trust_first(d.path(), "missing", scope(), key(1)).is_err());
            assert_eq!(fs::read(d.path().join(FILE)).unwrap(), bytes);
        }
        let pin = Pin {
            scope: scope(),
            key: key(1),
        };
        let record = Record {
            version: 1,
            pins: vec![pin.clone(), pin],
        };
        fs::write(d.path().join(FILE), serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(read(d.path()).unwrap_err(), "DUPLICATE_SSH_TRUST_SCOPE");
        let record = Record {
            version: 1,
            pins: vec![Pin {
                scope: Scope {
                    host: "EXAMPLE.invalid".into(),
                    port: 22,
                },
                key: key(1),
            }],
        };
        fs::write(d.path().join(FILE), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(read(d.path()).is_err());
    }
    #[test]
    fn malformed_key_blobs_and_algorithm_spoofing_are_refused() {
        let good = key(1).bytes().unwrap();
        for n in 0..good.len() {
            assert!(Key::from_blob(&good[..n]).is_err());
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(Key::from_blob(&extra).is_err());
        assert!(Key::from_blob(&vec![0; 8193]).is_err());
        let mut mismatched = key(1);
        mismatched.algorithm = "ssh-rsa".into();
        assert!(mismatched.bytes().is_err());
        let mut bad = good;
        bad[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Key::from_blob(&bad).is_err());
        for host in [
            "host/path",
            "user@host",
            "host?token",
            "host#fragment",
            "host\nother",
        ] {
            assert!(Scope::new(host, 22).is_err());
        }
        assert!(Scope::new("host", 0).is_err());
        assert!(Scope::new("127.1", 22).is_err());
    }
    #[test]
    fn fingerprints_match_openssh_generated_public_keys() {
        let d = dir();
        for (kind, bits) in [
            ("ed25519", "256"),
            ("rsa", "2048"),
            ("ecdsa", "256"),
            ("ecdsa", "384"),
            ("ecdsa", "521"),
        ] {
            let path = d.path().join(format!("{kind}-{bits}"));
            let result = std::process::Command::new("ssh-keygen")
                .args([
                    "-q",
                    "-t",
                    kind,
                    "-b",
                    bits,
                    "-N",
                    "",
                    "-C",
                    "synthetic fixture",
                    "-f",
                ])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let public = fs::read_to_string(path.with_extension("pub")).unwrap();
            let mut fields = public.split_whitespace();
            let algorithm = fields.next().unwrap();
            let blob = STANDARD.decode(fields.next().unwrap()).unwrap();
            let parsed = Key::from_blob(&blob).unwrap();
            assert_eq!(parsed.algorithm, algorithm);
            let oracle = std::process::Command::new("ssh-keygen")
                .args(["-lf"])
                .arg(path.with_extension("pub"))
                .args(["-E", "sha256"])
                .output()
                .unwrap();
            assert!(oracle.status.success());
            let text = String::from_utf8(oracle.stdout).unwrap();
            assert_eq!(
                text.split_whitespace().nth(1),
                Some(parsed.fingerprint().unwrap().as_str())
            );
        }
    }
    #[test]
    fn concurrent_first_trust_cannot_overwrite_another_key() {
        let d = dir();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (1..=2)
            .map(|n| {
                let path = d.path().to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    trust_first(&path, "missing", scope(), key(n))
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let state = read(d.path()).unwrap();
        assert_eq!(state.pins.len(), 1);
        assert!(results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e == "SSH_TRUST_BUSY" || e == "STALE_SSH_TRUST"));
    }
    #[test]
    #[cfg(unix)]
    fn symlink_and_foreign_lock_refusal_and_private_mode() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let d = dir();
        let external = dir();
        fs::write(external.path().join("target"), "preserve").unwrap();
        symlink(external.path().join("target"), d.path().join(FILE)).unwrap();
        assert!(read(d.path()).is_err());
        assert!(trust_first(d.path(), "missing", scope(), key(1)).is_err());
        assert_eq!(
            fs::read(external.path().join("target")).unwrap(),
            b"preserve"
        );
        fs::remove_file(d.path().join(FILE)).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(d.path().join("ssh-host-pins.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let lock = HeldLock(lock);
        assert_eq!(
            trust_first(d.path(), "missing", scope(), key(1)).unwrap_err(),
            "SSH_TRUST_BUSY"
        );
        drop(lock);
        trust_first(d.path(), "missing", scope(), key(1)).unwrap();
        assert_eq!(
            fs::metadata(d.path().join(FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn trust_crash_worker() {
        let Some(path) = std::env::var_os("BRANCHDESK_TRUST_DIR") else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        let state = read(&path).unwrap();
        trust_first(
            &path,
            &state.revision,
            Scope::new("second.invalid", 2222).unwrap(),
            key(2),
        )
        .unwrap();
    }
    #[test]
    fn process_kills_leave_old_or_complete_pin_set_and_release_lock() {
        for step in [0, 1] {
            let d = dir();
            let old = trust_first(d.path(), "missing", scope(), key(1)).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ssh_trust::tests::trust_crash_worker",
                    "--nocapture",
                ])
                .env("BRANCHDESK_TRUST_DIR", d.path())
                .env("BRANCHDESK_TRUST_STEP", step.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let start = std::time::Instant::now();
            while !d.path().join("ready").exists() {
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("worker exited {status}")
                }
                if start.elapsed() > std::time::Duration::from_secs(20) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("checkpoint timeout")
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            let current = read(d.path()).unwrap();
            assert_eq!(current.pins.len(), if step == 0 { 1 } else { 2 });
            assert_eq!(
                current.verify(&scope(), &key(1)).unwrap(),
                Verification::Match
            );
            if step == 0 {
                assert_eq!(current.revision, old.revision)
            }
            let next = trust_first(
                d.path(),
                &current.revision,
                Scope::new("second.invalid", 2222).unwrap(),
                key(2),
            )
            .unwrap();
            assert_eq!(next.pins.len(), 2);
        }
    }
}
