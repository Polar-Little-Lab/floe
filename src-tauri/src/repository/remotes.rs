//! Read local remote configuration without enabling transports or inherited config.
use super::*;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub tag_policy: String,
    pub fetch_urls: Vec<String>,
    pub push_urls: Vec<String>,
    pub fetch_refspecs: Vec<String>,
    pub branches: Vec<RemoteBranch>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    pub name: String,
    pub oid: Option<String>,
    pub symbolic_target: Option<String>,
}
pub(super) fn display_url(raw: &str) -> String {
    let clean = raw.split(['?', '#']).next().unwrap_or("");
    let parsed_https = clean.starts_with("https://")
        && tauri::Url::parse(clean).is_ok_and(|url| url.host_str().is_some());
    if (clean.contains("::") && !parsed_https) || clean.chars().any(char::is_control) {
        return "[unsupported remote URL]".into();
    }
    if let Some((scheme, rest)) = clean.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "file") {
            return "[unsupported remote URL]".into();
        }
        let (authority, path) = rest
            .split_once('/')
            .map(|(a, p)| (a, format!("/{p}")))
            .unwrap_or((rest, String::new()));
        let authority = authority
            .rsplit_once('@')
            .map(|(_, host)| format!("[redacted]@{host}"))
            .unwrap_or_else(|| authority.into());
        return format!("{scheme}://{authority}{path}");
    }
    if let Some((_, host_path)) = clean.rsplit_once('@') {
        return format!("[redacted]@{host_path}");
    }
    clean.into()
}
pub(super) fn inspect(root: &Path, repo: &Repository) -> Result<Vec<Remote>> {
    let layout=layout::Layout::read(root)?;
    let raw = bounded(&layout.common.join("config"), 65536)?;
    if layout.submodule {remote_remove::read_only_entries(&raw)?;}else{remote_remove::entries(&raw)?;}
    // Parse a bounded private copy. Config error messages can contain URL secrets.
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, raw).map_err(err)?;
    let config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    let mut map = std::collections::BTreeMap::<String, Remote>::new();
    let mut entries = config.entries(None).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|_| "INVALID_REMOTE_CONFIG")?;
        let name = entry.name().map_err(|_| "INVALID_REMOTE_CONFIG")?;
        let Some(remote_key) = name.strip_prefix("remote.") else {
            continue;
        };
        let Some((name, key)) = remote_key.rsplit_once('.') else {
            continue;
        };
        if name.is_empty() || name.len() > 200 {
            return Err("INVALID_REMOTE_NAME".into());
        }
        if map.len() >= MAX_FILES && !map.contains_key(name) {
            return Err("LIMIT: remotes".into());
        }
        let remote = map.entry(name.into()).or_insert_with(|| Remote {
            name: name.into(),
            tag_policy: "auto".into(),
            fetch_urls: Vec::new(),
            push_urls: Vec::new(),
            fetch_refspecs: Vec::new(),
            branches: Vec::new(),
        });
        if !matches!(key, "url" | "pushurl" | "fetch" | "tagopt") {
            continue;
        }
        let value = entry.value().map_err(|_| "INVALID_REMOTE_CONFIG")?;
        if value.len() > 4096 {
            return Err("LIMIT: remote value".into());
        }
        match key {
            "url" => remote.fetch_urls.push(display_url(value)),
            "pushurl" => remote.push_urls.push(display_url(value)),
            "fetch" => remote.fetch_refspecs.push(value.into()),
            "tagopt" => {
                remote.tag_policy = match value {
                    "--tags" => "all",
                    "--no-tags" => "none",
                    _ => "unsupported",
                }
                .into()
            }
            _ => unreachable!(),
        }
    }
    let mut count = 0;
    for entry in repo.references_glob("refs/remotes/*").map_err(err)? {
        let reference = entry.map_err(err)?;
        count += 1;
        if count > MAX_FILES {
            return Err("LIMIT: remote refs".into());
        }
        let Some(full) = reference.name().map_err(err)?.strip_prefix("refs/remotes/") else {
            continue;
        };
        // Prefer the longest configured name, including names containing slashes.
        let owner = map
            .keys()
            .filter(|name| {
                full.strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|name| name.len())
            .cloned();
        if let Some(owner) = owner {
            let name = full[owner.len() + 1..].into();
            map.get_mut(&owner).unwrap().branches.push(RemoteBranch {
                name,
                oid: reference.target().map(|id| id.to_string()),
                symbolic_target: reference.symbolic_target().map_err(err)?.map(str::to_owned),
            });
        }
    }
    for remote in map.values_mut() {
        remote.branches.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(map.into_values().collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn https_ipv6_display_preserves_host_without_exposing_credentials() {
        assert_eq!(
            display_url("https://[2001:db8::1]:8443/repo.git"),
            "https://[2001:db8::1]:8443/repo.git"
        );
        assert_eq!(
            display_url("https://user:secret@[2001:db8::1]/r?token=secret"),
            "https://[redacted]@[2001:db8::1]/r"
        );
        assert_eq!(display_url("ext::helper"), "[unsupported remote URL]");
    }
    #[test]
    fn remote_inspection_preserves_repository_and_redacts_endpoint_secrets() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        let sig = git2::Signature::now("Remote Test", "remote@invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let head = repo
            .commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        repo.reference("refs/remotes/team/origin/main", head, false, "test")
            .unwrap();
        repo.reference_symbolic(
            "refs/remotes/team/origin/HEAD",
            "refs/remotes/team/origin/main",
            false,
            "test",
        )
        .unwrap();
        let mut config = repo.config().unwrap();
        config
            .set_str(
                "remote.team/origin.url",
                "https://user:secret@example.invalid/repo.git?token=private#fragment",
            )
            .unwrap();
        config
            .set_str("remote.team/origin.pushurl", "git@example.invalid:repo.git")
            .unwrap();
        config
            .set_str(
                "remote.team/origin.fetch",
                "+refs/heads/*:refs/remotes/team/origin/*",
            )
            .unwrap();
        config
            .set_str("remote.team.url", "ext::dangerous helper")
            .unwrap();
        drop(config);
        let original = fs::read(d.path().join(".git/config")).unwrap();
        let found = inspect(d.path(), &repo).unwrap();
        assert_eq!(found.len(), 2);
        let remote = &found[1];
        assert_eq!(remote.name, "team/origin");
        assert_eq!(
            remote.fetch_urls,
            vec!["https://[redacted]@example.invalid/repo.git"]
        );
        assert_eq!(
            remote.push_urls,
            vec!["[redacted]@example.invalid:repo.git"]
        );
        assert_eq!(remote.branches.len(), 2);
        assert!(remote.branches[0].symbolic_target.is_some());
        assert_eq!(found[0].fetch_urls, vec!["[unsupported remote URL]"]);
        let encoded = serde_json::to_string(&found).unwrap();
        for secret in ["secret", "private", "fragment", "dangerous"] {
            assert!(!encoded.contains(secret));
        }
        assert_eq!(fs::read(d.path().join(".git/config")).unwrap(), original);
        assert_eq!(repo.head().unwrap().target(), Some(head));
        assert!(!d.path().join(".git/config.lock").exists());
    }
}
