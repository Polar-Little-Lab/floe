//! Resolve exactly one configured fetch endpoint; no renderer-supplied URL.
use super::*;
pub fn endpoint(session: &mut Session, revision: &str, name: &str) -> Result<String> {
    let (urls, _) = configured_urls(session, revision, name)?;
    if urls.len() != 1 {
        return Err("REMOTE_REQUIRES_SINGLE_FETCH_URL".into());
    }
    Ok(remote_endpoint::https(&urls[0])?.to_string())
}
// Renderer selects only a configured URL slot, never an arbitrary vault key.
pub(crate) fn credential_endpoint(
    session: &mut Session,
    revision: &str,
    name: &str,
    push: bool,
    index: usize,
) -> Result<String> {
    let (fetch, push_urls) = configured_urls(session, revision, name)?;
    let urls = if push && !push_urls.is_empty() {
        &push_urls
    } else {
        &fetch
    };
    let value = urls.get(index).ok_or("INVALID_REMOTE_URL_INDEX")?;
    Ok(remote_endpoint::https(value)?.to_string())
}
fn configured_urls(
    session: &mut Session,
    revision: &str,
    name: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    if name.is_empty() || name.len() > 200 {
        return Err("INVALID_REMOTE_NAME".into());
    }
    session.fresh(revision)?;
    let bytes = bounded(&session.root.join(".git/config"), 65536)?;
    let private = tempfile::tempdir().map_err(|_| "REMOTE_CONFIG_FAILED")?;
    let path = private.path().join("config");
    fs::write(&path, bytes).map_err(|_| "REMOTE_CONFIG_FAILED")?;
    let config = Config::open(&path).map_err(|_| "REMOTE_CONFIG_FAILED")?;
    let fetch_key = format!("remote.{name}.url");
    let push_key = format!("remote.{name}.pushurl");
    let (mut fetch, mut push) = (Vec::new(), Vec::new());
    let mut entries = config.entries(None).map_err(|_| "REMOTE_CONFIG_FAILED")?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|_| "REMOTE_CONFIG_FAILED")?;
        let key = entry.name().map_err(|_| "REMOTE_CONFIG_FAILED")?;
        let target = if key == fetch_key {
            &mut fetch
        } else if key == push_key {
            &mut push
        } else {
            continue;
        };
        target.push(
            entry
                .value()
                .map_err(|_| "REMOTE_CONFIG_FAILED")?
                .to_owned(),
        );
    }
    Ok((fetch, push))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(url: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.remote("origin", url).unwrap();
        repo.index().unwrap().write().unwrap();
        d
    }
    #[test]
    fn resolves_only_configured_endpoint_without_repository_writes() {
        let d = fixture("https://example.com/repo.git");
        let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        let path = d.path().join(".git/config");
        let before = fs::read(&path).unwrap();
        assert_eq!(
            endpoint(&mut session, &snap.revision, "origin").unwrap(),
            "https://example.com/repo.git"
        );
        assert!(endpoint(&mut session, &snap.revision, "missing")
            .unwrap_err()
            .contains("SINGLE_FETCH_URL"));
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn rejects_stale_and_ambiguous_endpoint() {
        let d = fixture("https://example.com/repo.git");
        let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        let path = d.path().join(".git/config");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n[remote \"origin\"]\n url = https://other.example/repo.git\n");
        fs::write(path, bytes).unwrap();
        assert!(endpoint(&mut session, &snap.revision, "origin").is_err());
        let fresh = session.snapshot().unwrap();
        assert!(endpoint(&mut session, &fresh.revision, "origin")
            .unwrap_err()
            .contains("SINGLE_FETCH_URL"));
    }
    #[test]
    fn rejects_credentials_and_non_https_endpoints() {
        for url in [
            "https://user:secret@example.com/repo.git",
            "http://example.com/repo.git",
            "file:///tmp/repo",
            "git@example.com:repo",
        ] {
            let d = fixture(url);
            let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(endpoint(&mut session, &snap.revision, "origin").is_err());
        }
    }
    #[test]
    fn credential_slots_follow_push_fallback_and_refuse_stale_or_invalid_selection() {
        let d = fixture("https://EXAMPLE.com:443/repo.git");
        let mut session = Session::new(d.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        let path = d.path().join(".git/config");
        let before = fs::read(&path).unwrap();
        assert_eq!(
            credential_endpoint(&mut session, &snap.revision, "origin", true, 0).unwrap(),
            "https://example.com/repo.git"
        );
        assert!(credential_endpoint(&mut session, &snap.revision, "origin", false, 1).is_err());
        assert!(credential_endpoint(&mut session, &snap.revision, "missing", false, 0).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut changed = before.clone();
        changed.extend_from_slice(b"\n[remote \"origin\"]\n pushurl = https://push.example/r.git\n pushurl = https://other.example/r.git\n");
        fs::write(&path, &changed).unwrap();
        assert!(credential_endpoint(&mut session, &snap.revision, "origin", true, 0).is_err());
        let fresh = session.snapshot().unwrap();
        assert_eq!(
            credential_endpoint(&mut session, &fresh.revision, "origin", true, 0).unwrap(),
            "https://push.example/r.git"
        );
        assert_eq!(
            credential_endpoint(&mut session, &fresh.revision, "origin", true, 1).unwrap(),
            "https://other.example/r.git"
        );
        assert_eq!(
            credential_endpoint(&mut session, &fresh.revision, "origin", false, 0).unwrap(),
            "https://example.com/repo.git"
        );
        assert!(
            credential_endpoint(&mut session, &fresh.revision, "origin", true, usize::MAX).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
}
