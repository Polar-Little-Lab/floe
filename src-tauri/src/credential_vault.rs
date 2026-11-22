//! Native-only HTTPS credential storage. Deliberately not an IPC command or
//! serializable public credential: transport integration must retain ownership.
//! Call on a blocking worker. This layer neither authorizes network use nor
//! retries failed authentication. Windows has no plaintext fallback.
#![allow(dead_code)] // Storage foundation; native prompt/transport integration follows.
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, &'static str>;
const MAX_RECORD: usize = 16384;

pub(crate) struct Credential {
    username: Zeroizing<String>,
    password: Zeroizing<String>,
}
impl Credential {
    pub(crate) fn new(username: String, password: String) -> Result<Self> {
        let value = Self {
            username: Zeroizing::new(username),
            password: Zeroizing::new(password),
        };
        if value.username.is_empty()
            || value.username.len() > 1024
            || value.username.contains(':')
            || value.username.chars().any(char::is_control)
            || value.password.is_empty()
            || value.password.len() > 8192
            || value.password.chars().any(char::is_control)
        {
            return Err("INVALID_HTTPS_CREDENTIAL");
        }
        Ok(value)
    }
    pub(crate) fn bearer_authorization(&self) -> Result<reqwest::header::HeaderValue> {
        if !self.password.bytes().all(|b| b.is_ascii_graphic()) { return Err("INVALID_PROVIDER_TOKEN"); }
        let mut raw = Zeroizing::new(String::from("Bearer "));
        raw.push_str(&self.password);
        let mut header = reqwest::header::HeaderValue::from_str(&raw).map_err(|_| "INVALID_PROVIDER_TOKEN")?;
        header.set_sensitive(true);
        Ok(header)
    }
    pub(crate) fn authorization(&self) -> Result<reqwest::header::HeaderValue> {
        use base64::Engine;
        let mut raw = Zeroizing::new(String::with_capacity(
            self.username.len() + self.password.len() + 1,
        ));
        raw.push_str(&self.username);
        raw.push(':');
        raw.push_str(&self.password);
        let mut encoded = Zeroizing::new(String::from("Basic "));
        base64::engine::general_purpose::STANDARD.encode_string(raw.as_bytes(), &mut encoded);
        let mut header = reqwest::header::HeaderValue::from_str(&encoded)
            .map_err(|_| "INVALID_HTTPS_CREDENTIAL")?;
        header.set_sensitive(true);
        Ok(header)
    }
    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        // Fixed binary framing avoids serializer intermediates containing secrets.
        let mut bytes = Zeroizing::new(Vec::with_capacity(
            8 + self.username.len() + self.password.len(),
        ));
        bytes.extend_from_slice(b"BDC1");
        bytes.extend_from_slice(&(self.username.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.username.as_bytes());
        bytes.extend_from_slice(self.password.as_bytes());
        bytes
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 10 || bytes.len() > MAX_RECORD || &bytes[..4] != b"BDC1" {
            return Err("INVALID_STORED_CREDENTIAL");
        }
        let count = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if count == 0 || count > 1024 || 8 + count >= bytes.len() {
            return Err("INVALID_STORED_CREDENTIAL");
        }
        let username =
            std::str::from_utf8(&bytes[8..8 + count]).map_err(|_| "INVALID_STORED_CREDENTIAL")?;
        let password =
            std::str::from_utf8(&bytes[8 + count..]).map_err(|_| "INVALID_STORED_CREDENTIAL")?;
        Self::new(username.into(), password.into()).map_err(|_| "INVALID_STORED_CREDENTIAL")
    }
}
// Prevent accidental diagnostic leakage without removing useful type identity.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential([REDACTED])")
    }
}

fn service(endpoint: &str) -> Result<std::ffi::CString> {
    let canonical = crate::repository::remote_endpoint::https(endpoint)
        .map_err(|_| "INVALID_CREDENTIAL_ENDPOINT")?;
    let hash = Sha256::digest(canonical.as_str().as_bytes());
    let namespace = if cfg!(feature = "developer-test") { "dev.branchdesk.devtest.https.v1" } else { "dev.branchdesk.https.v1" };
    std::ffi::CString::new(format!("{namespace}.{hash:x}"))
        .map_err(|_| "INVALID_CREDENTIAL_ENDPOINT")
}
fn status(code: i32) -> Result<()> {
    match code {
        0 => Ok(()),
        -25300 => Err("CREDENTIAL_NOT_FOUND"),
        -25299 => Err("CREDENTIAL_CHANGED"),
        -25308 | -25293 => Err("KEYCHAIN_LOCKED_OR_DENIED"),
        -34018 => Err("KEYCHAIN_SIGNING_REQUIRED"),
        -25291 => Err("KEYCHAIN_UNAVAILABLE"),
        -26275 => Err("INVALID_STORED_CREDENTIAL"),
        _ => Err("KEYCHAIN_OPERATION_FAILED"),
    }
}

#[cfg(target_os = "macos")]
mod native {
    unsafe extern "C" {
        pub(super) fn branchdesk_vault_put(
            service: *const std::ffi::c_char,
            bytes: *const u8,
            length: usize,
        ) -> i32;
        pub(super) fn branchdesk_vault_get(
            service: *const std::ffi::c_char,
            bytes: *mut u8,
            capacity: usize,
            length: *mut usize,
        ) -> i32;
        pub(super) fn branchdesk_vault_delete(service: *const std::ffi::c_char) -> i32;
    }
}

pub(crate) fn put(endpoint: &str, credential: &Credential) -> Result<()> {
    let name = service(endpoint)?;
    let bytes = credential.encode();
    #[cfg(target_os = "macos")]
    // SAFETY: NUL-terminated name and borrowed bytes live through synchronous call.
    return status(unsafe {
        native::branchdesk_vault_put(name.as_ptr(), bytes.as_ptr(), bytes.len())
    });
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (name, bytes);
        Err("CREDENTIAL_STORAGE_UNSUPPORTED")
    }
}
pub(crate) fn get(endpoint: &str) -> Result<Option<Credential>> {
    let name = service(endpoint)?;
    #[cfg(target_os = "macos")]
    {
        let mut bytes = Zeroizing::new(vec![0u8; MAX_RECORD]);
        let mut length = 0;
        // SAFETY: output capacity and length pointer match live Rust allocations.
        let code = unsafe {
            native::branchdesk_vault_get(
                name.as_ptr(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut length,
            )
        };
        if code == -25300 {
            return Ok(None);
        }
        status(code)?;
        if length > bytes.len() {
            return Err("INVALID_STORED_CREDENTIAL");
        }
        Credential::decode(&bytes[..length]).map(Some)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        Err("CREDENTIAL_STORAGE_UNSUPPORTED")
    }
}
pub(crate) fn delete(endpoint: &str) -> Result<()> {
    let name = service(endpoint)?;
    #[cfg(target_os = "macos")]
    {
        // SAFETY: valid C string remains alive for the synchronous operation.
        let code = unsafe { native::branchdesk_vault_delete(name.as_ptr()) };
        if code == -25300 {
            return Ok(());
        }
        status(code)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = name;
        Err("CREDENTIAL_STORAGE_UNSUPPORTED")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_is_exact_repository_origin_and_path_not_host_wide() {
        let base = service("https://example.invalid/team/a.git").unwrap();
        assert_eq!(
            base,
            service("https://EXAMPLE.invalid:443/team/a.git").unwrap()
        );
        for other in [
            "https://example.invalid/team/b.git",
            "https://example.invalid:8443/team/a.git",
            "https://else.invalid/team/a.git",
            "https://example.invalid/team/a.git/",
            "https://example.invalid/Team/a.git",
        ] {
            assert_ne!(base, service(other).unwrap());
        }
        for bad in [
            "http://example.invalid/r",
            "https://u:p@example.invalid/r",
            "https://example.invalid/r?token=x",
            "https://example.invalid/a/../r",
        ] {
            assert!(service(bad).is_err());
        }
        assert!(!base.to_str().unwrap().contains("example.invalid"));
    }
    #[test]
    fn credentials_are_bounded_redacted_and_strictly_framed() {
        let value = Credential::new("사용자".into(), "synthetic-🔐-token".into()).unwrap();
        let bytes = value.encode();
        let restored = Credential::decode(&bytes).unwrap();
        assert_eq!(*restored.username, *value.username);
        assert_eq!(*restored.password, *value.password);
        assert_eq!(format!("{value:?}"), "Credential([REDACTED])");
        for (u, p) in [
            ("", "x"),
            ("u:x", "x"),
            ("u\n", "x"),
            ("u", ""),
            ("u", "secret\r\nheader"),
        ] {
            assert!(Credential::new(u.into(), p.into()).is_err());
        }
        assert!(Credential::new("u".into(), "x".repeat(8193)).is_err());
        for bytes in [
            b"".as_slice(),
            b"BDC1\xff\xff\xff\xffxy",
            b"BDC1\0\0\0\x01u\xff",
            b"BDC1\0\0\0\x01u",
            b"bad1\0\0\0\x01up",
        ] {
            assert!(Credential::decode(bytes).is_err());
        }
    }
    #[test]
    fn keychain_failure_messages_are_fixed_and_never_include_input() {
        for (code, expected) in [
            (-25300, "CREDENTIAL_NOT_FOUND"),
            (-25308, "KEYCHAIN_LOCKED_OR_DENIED"),
            (-34018, "KEYCHAIN_SIGNING_REQUIRED"),
            (-25299, "CREDENTIAL_CHANGED"),
            (12345, "KEYCHAIN_OPERATION_FAILED"),
        ] {
            assert_eq!(status(code), Err(expected));
        }
    }
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Requires signed test host with Keychain entitlement and unlocked Data Protection Keychain"]
    fn native_synthetic_roundtrip_update_delete_and_endpoint_isolation() {
        let root = format!("https://{}.invalid", uuid::Uuid::new_v4());
        let a = format!("{root}/a");
        let b = format!("{root}/b");
        struct Cleanup(String, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
                let _ = delete(&self.1);
            }
        }
        let _cleanup = Cleanup(a.clone(), b.clone());
        assert!(get(&a).unwrap().is_none());
        put(
            &a,
            &Credential::new("synthetic".into(), "first".into()).unwrap(),
        )
        .unwrap();
        assert!(get(&b).unwrap().is_none());
        assert_eq!(*get(&a).unwrap().unwrap().password, "first");
        put(
            &a,
            &Credential::new("replacement".into(), "second".into()).unwrap(),
        )
        .unwrap();
        let updated = get(&a).unwrap().unwrap();
        assert_eq!(*updated.username, "replacement");
        assert_eq!(*updated.password, "second");
        delete(&a).unwrap();
        delete(&a).unwrap();
        assert!(get(&a).unwrap().is_none());
    }
}
