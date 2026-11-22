//! Test-only native-consent boundary. No renderer DTO or real dialog is wired.
use crate::{ssh_discovery::Observed, ssh_endpoint::{self, Endpoint}, ssh_trust::{self, Scope, Verification}};
use std::path::Path;

#[derive(Debug)]
struct Review {
    endpoint: Endpoint,
    algorithm: String,
    fingerprint: String,
}

#[derive(Debug)]
struct Grant {
    endpoint: Endpoint,
    scope: Scope,
    revision: String,
    fingerprint: String,
}

impl Grant {
    // Call immediately before using destination-scoped credentials/transport.
    // This is not an instantaneous revocation mechanism for an active session.
    fn revalidate(&self, directory: &Path, url: &str, observed: &Observed) -> Result<(), String> {
        if ssh_endpoint::parse(url)? != self.endpoint || observed.scope != self.scope
            || observed.key.fingerprint()? != self.fingerprint {
            return Err("SSH_AUTHORIZATION_SCOPE_MISMATCH".into());
        }
        let state = ssh_trust::read(directory)?;
        if state.revision != self.revision { return Err("STALE_SSH_AUTHORIZATION".into()); }
        if state.verify(&self.scope, &observed.key)? != Verification::Match {
            return Err("SSH_HOST_NOT_TRUSTED".into());
        }
        Ok(())
    }
}

fn authorize(directory: &Path, url: &str, observed: &Observed,
             confirm_first: impl FnOnce(&Review) -> Result<bool, String>) -> Result<Option<Grant>, String> {
    let endpoint = ssh_endpoint::parse(url)?;
    let scope = Scope::new(&endpoint.host, endpoint.port)?;
    if scope != observed.scope { return Err("SSH_DISCOVERY_SCOPE_MISMATCH".into()); }
    let review = Review { endpoint, algorithm: observed.key.algorithm().into(), fingerprint: observed.key.fingerprint()? };
    let state = ssh_trust::read(directory)?;
    let state = match state.verify(&scope, &observed.key)? {
        Verification::Changed { .. } => return Err("SSH_HOST_KEY_CHANGED".into()),
        Verification::Match => state,
        Verification::Unknown => {
            if !confirm_first(&review)? { return Ok(None); }
            // The complete frozen revision and observed key survive the native
            // prompt. A concurrent write cannot turn approval into key rotation.
            ssh_trust::trust_first(directory, &state.revision, scope.clone(), observed.key.clone())?
        }
    };
    Ok(Some(Grant { endpoint: review.endpoint, scope, revision: state.revision,
                    fingerprint: review.fingerprint }))
}

#[cfg(test)]
mod tests {
    use super::*;
    const URL: &str = "ssh://fixture@example.invalid:2222/team/repo";
    fn dir() -> tempfile::TempDir { tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap() }
    fn observation(byte: u8) -> Observed {
        let mut bytes = Vec::new();
        for field in [b"ssh-ed25519".as_slice(), &[byte;32]] {
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes()); bytes.extend_from_slice(field);
        }
        Observed { scope: Scope::new("example.invalid",2222).unwrap(), key:ssh_trust::Key::from_blob(&bytes).unwrap() }
    }
    #[test]
    fn cancellation_and_dialog_error_have_no_persistent_side_effects() {
        let d=dir(); let observed=observation(1);
        assert!(authorize(d.path(),URL,&observed,|review| {
            assert_eq!(review.endpoint,ssh_endpoint::parse(URL).unwrap());
            assert_eq!(review.algorithm,"ssh-ed25519");
            assert_eq!(review.fingerprint,observed.key.fingerprint().unwrap()); Ok(false)
        }).unwrap().is_none());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(),0);
        assert_eq!(authorize(d.path(),URL,&observed, |_| Err("NATIVE_DIALOG_FAILED".into())).unwrap_err(),"NATIVE_DIALOG_FAILED");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(),0);
    }
    #[test]
    fn explicit_first_approval_and_known_key_bind_full_operation_destination() {
        let d=dir(); let observed=observation(1);
        let grant=authorize(d.path(),URL,&observed, |_| Ok(true)).unwrap().unwrap();
        grant.revalidate(d.path(),URL,&observed).unwrap();
        let known=authorize(d.path(),URL,&observed, |_| panic!("known key must not ask first-trust consent")).unwrap().unwrap();
        assert_eq!(known.revision,grant.revision);
        for url in ["ssh://other@example.invalid:2222/team/repo", "ssh://fixture@example.invalid:2222/other/repo",
                    "ssh://fixture@example.invalid:2223/team/repo", "fixture@example.invalid:team/repo"] {
            assert_eq!(grant.revalidate(d.path(),url,&observed).unwrap_err(),"SSH_AUTHORIZATION_SCOPE_MISMATCH");
        }
        assert_eq!(grant.revalidate(d.path(),URL,&observation(2)).unwrap_err(),"SSH_AUTHORIZATION_SCOPE_MISMATCH");
        // Host pin may authorize a separate operation only with a new bound grant.
        let other="ssh://other@example.invalid:2222/other/repo";
        authorize(d.path(),other,&observed, |_| panic!()).unwrap().unwrap().revalidate(d.path(),other,&observed).unwrap();
    }
    #[test]
    fn changed_key_and_wrong_discovery_scope_never_reach_consent() {
        let d=dir(); let observed=observation(1);
        authorize(d.path(),URL,&observed, |_| Ok(true)).unwrap();
        let before=ssh_trust::read(d.path()).unwrap().revision;
        assert_eq!(authorize(d.path(),URL,&observation(2), |_| panic!()).unwrap_err(),"SSH_HOST_KEY_CHANGED");
        assert_eq!(authorize(d.path(),"ssh://fixture@other.invalid:2222/team/repo",&observed, |_| panic!()).unwrap_err(),"SSH_DISCOVERY_SCOPE_MISMATCH");
        assert_eq!(ssh_trust::read(d.path()).unwrap().revision,before);
    }
    #[test]
    fn concurrent_approval_forget_and_corruption_invalidate_frozen_decisions() {
        let d=dir(); let observed=observation(1);
        let result=authorize(d.path(),URL,&observed, |_| {
            let state=ssh_trust::read(d.path())?;
            ssh_trust::trust_first(d.path(),&state.revision,observed.scope.clone(),observation(2).key)?;
            Ok(true)
        });
        assert_eq!(result.unwrap_err(),"STALE_SSH_TRUST");
        assert_eq!(ssh_trust::read(d.path()).unwrap().verify(&observed.scope,&observation(2).key).unwrap(),Verification::Match);
        let d=dir();
        let grant=authorize(d.path(),URL,&observed, |_| Ok(true)).unwrap().unwrap();
        ssh_trust::forget(d.path(),&grant.revision,&observed.scope,&grant.fingerprint).unwrap();
        assert_eq!(grant.revalidate(d.path(),URL,&observed).unwrap_err(),"STALE_SSH_AUTHORIZATION");
        std::fs::write(d.path().join("ssh-host-pins.json"),"corrupt").unwrap();
        assert_eq!(authorize(d.path(),URL,&observed, |_| panic!()).unwrap_err(),"INVALID_SSH_TRUST_FILE");
        assert_eq!(grant.revalidate(d.path(),URL,&observed).unwrap_err(),"INVALID_SSH_TRUST_FILE");
    }
}
