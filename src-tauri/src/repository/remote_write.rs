//! Journaled local endpoint edits; this module never starts a transport.
use super::*;
const JOURNAL: &str = ".git/branchdesk-remote-config";
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TagPolicy {
    Auto,
    All,
    None,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    #[serde(default)]
    tag_policy: Option<TagPolicy>,
    #[serde(default)]
    upstream: Option<UpstreamEdit>,
    #[serde(default)]
    identity_edit: Option<IdentityEdit>,
    #[serde(default)]
    gitflow_edit: Option<gitflow::Configuration>,
    name: String,
    url: String,
    push_url: Option<String>,
    create: bool,
    before: Vec<u8>,
    after: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityEdit {
    identity: Option<crate::identity_settings::Identity>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RepositoryIdentity {
    pub identity: Option<crate::identity_settings::Identity>,
    pub configured: bool,
    pub warning: Option<String>,
}
pub(super) fn repository_identity(values: Option<&[(String, String)]>) -> RepositoryIdentity {
    let invalid = || {
        RepositoryIdentity{identity:None,configured:true,warning:Some("Repository identity is incomplete, invalid or unreadable. Repair it, clear it, or enter both operation identity fields. App default is not used.".into())}
    };
    let Some(values) = values else {
        return invalid();
    };
    let get = |key: &str| {
        values
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    match (get("user.name"), get("user.email")) {
        (None, None) => RepositoryIdentity {
            identity: None,
            configured: false,
            warning: None,
        },
        (Some(name), Some(email)) => {
            let identity = crate::identity_settings::Identity { name, email };
            if crate::identity_settings::validate(&identity).is_err() {
                invalid()
            } else {
                RepositoryIdentity {
                    identity: Some(identity),
                    configured: true,
                    warning: None,
                }
            }
        }
        _ => invalid(),
    }
}
fn edited_identity(
    before: &[u8],
    identity: &Option<crate::identity_settings::Identity>,
) -> Result<Vec<u8>> {
    if let Some(i) = identity {
        crate::identity_settings::validate(i)?;
    }
    let entries = remote_remove::entries(before)?;
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, before).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    for key in ["user.name", "user.email"] {
        if entries.iter().any(|(k, _)| k == key) {
            config
                .remove_multivar(key, ".*")
                .map_err(|_| "IDENTITY_CONFIG_EDIT_FAILED")?;
        }
    }
    if let Some(i) = identity {
        config
            .set_str("user.name", &i.name)
            .map_err(|_| "IDENTITY_CONFIG_EDIT_FAILED")?;
        config
            .set_str("user.email", &i.email)
            .map_err(|_| "IDENTITY_CONFIG_EDIT_FAILED")?;
    }
    drop(config);
    let after = bounded(&path, 65536)?;
    remote_remove::entries(&after)?;
    Ok(after)
}
pub(super) fn execute_identity(
    session: &mut Session,
    revision: &str,
    identity: Option<crate::identity_settings::Identity>,
) -> Result<()> {
    let _lock = ConfigLock::acquire(&session.root)?;
    session.fresh(revision)?;
    let before = bounded(&session.root.join(".git/config"), 65536)?;
    let after = edited_identity(&before, &identity)?;
    let record = Record {
        version: 4,
        tag_policy: None,
        upstream: None,
        identity_edit: Some(IdentityEdit { identity }),
        gitflow_edit: None,
        name: String::new(),
        url: String::new(),
        push_url: None,
        create: false,
        before,
        after,
    };
    publish(session, &record)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamTarget {
    pub remote: String,
    pub branch: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpstreamEdit {
    branch: String,
    target: Option<UpstreamTarget>,
}
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamConfig {
    pub remotes: Vec<String>,
    pub merges: Vec<String>,
}
pub(super) fn upstream_config(values: &[(String, String)], branch: &str) -> UpstreamConfig {
    let collect = |suffix: &str| {
        values
            .iter()
            .filter(|(k, _)| k == &format!("branch.{branch}.{suffix}"))
            .map(|(_, v)| {
                let valid = if suffix == "remote" {
                    v == "."
                        || (v.len() <= 200
                            && !v.starts_with('-')
                            && git2::Reference::is_valid_name(&format!("refs/remotes/{v}/probe")))
                } else {
                    v.strip_prefix("refs/heads/")
                        .is_some_and(|name| branch_name(name).is_ok())
                };
                if valid {
                    v.clone()
                } else {
                    "[unsupported upstream value]".into()
                }
            })
            .collect()
    };
    UpstreamConfig {
        remotes: collect("remote"),
        merges: collect("merge"),
    }
}
fn branch_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 1024
        || name.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/heads/{name}"))
    {
        return Err("INVALID_UPSTREAM_BRANCH".into());
    }
    Ok(())
}
fn edited_upstream(before: &[u8], edit: &UpstreamEdit) -> Result<Vec<u8>> {
    branch_name(&edit.branch)?;
    let entries = remote_remove::entries(before)?;
    if let Some(target) = &edit.target {
        branch_name(&target.branch)?;
        let name = &target.remote;
        if name.is_empty()
            || name.len() > 200
            || name.starts_with('-')
            || name == "."
            || !git2::Reference::is_valid_name(&format!("refs/remotes/{name}/probe"))
        {
            return Err("INVALID_REMOTE_NAME".into());
        }
        if !entries
            .iter()
            .any(|(k, _)| k == &format!("remote.{name}.url"))
        {
            return Err("REMOTE_NOT_FOUND".into());
        }
    }
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, before).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    for suffix in ["remote", "merge"] {
        let key = format!("branch.{}.{suffix}", edit.branch);
        if entries.iter().any(|(k, _)| k == &key) {
            config
                .remove_multivar(&key, ".*")
                .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
        }
    }
    if let Some(target) = &edit.target {
        config
            .set_str(&format!("branch.{}.remote", edit.branch), &target.remote)
            .map_err(err)?;
        config
            .set_str(
                &format!("branch.{}.merge", edit.branch),
                &format!("refs/heads/{}", target.branch),
            )
            .map_err(err)?;
    }
    drop(config);
    let after = bounded(&path, 65536)?;
    remote_remove::entries(&after)?;
    Ok(after)
}
pub(super) fn execute_upstream(
    session: &mut Session,
    revision: &str,
    branch: String,
    target: Option<UpstreamTarget>,
) -> Result<()> {
    let _lock = ConfigLock::acquire(&session.root)?;
    session.fresh(revision)?;
    branch_name(&branch)?;
    let repo = open(&session.root)?;
    let reference = repo
        .find_reference(&format!("refs/heads/{branch}"))
        .map_err(|_| "LOCAL_BRANCH_NOT_FOUND")?;
    if reference.symbolic_target().map_err(err)?.is_some() {
        return Err("SYMBOLIC_LOCAL_BRANCH_UNSUPPORTED".into());
    }
    reference
        .peel_to_commit()
        .map_err(|_| "LOCAL_BRANCH_NOT_COMMIT")?;
    let before = bounded(&session.root.join(".git/config"), 65536)?;
    let edit = UpstreamEdit { branch, target };
    let after = edited_upstream(&before, &edit)?;
    let record = Record {
        version: 3,
        tag_policy: None,
        upstream: Some(edit),
        identity_edit: None,
        gitflow_edit: None,
        name: String::new(),
        url: String::new(),
        push_url: None,
        create: false,
        before,
        after,
    };
    publish(session, &record)
}
fn endpoint(value: &str) -> Result<()> {
    if value.starts_with("https://") {
        return remote_endpoint::https(value).map(|_| ());
    }
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['?', '#'])
        || value.contains("::")
        || value.starts_with('-')
    {
        return Err("INVALID_REMOTE_URL".into());
    }
    if let Some((scheme, rest)) = value.split_once("://") {
        if !matches!(scheme, "https" | "ssh" | "git" | "file") || rest.is_empty() {
            return Err("UNSUPPORTED_REMOTE_URL".into());
        }
        let authority = rest.split('/').next().unwrap_or("");
        if let Some((user, host)) = authority.rsplit_once('@') {
            if scheme != "ssh" || user.is_empty() || user.contains([':', '@']) || host.is_empty() {
                return Err("REMOTE_CREDENTIALS_NOT_ALLOWED".into());
            }
        }
        if scheme != "file" && authority.is_empty() {
            return Err("INVALID_REMOTE_URL".into());
        }
        return Ok(());
    }
    if Path::new(value).is_absolute() {
        return Ok(());
    }
    if let Some((host, path)) = value.split_once(':') {
        if !host.is_empty()
            && !path.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@'))
            && !host.starts_with('@')
            && !host.ends_with('@')
            && host.matches('@').count() <= 1
        {
            return Ok(());
        }
    }
    Err("INVALID_REMOTE_URL".into())
}
fn edited(
    before: &[u8],
    name: &str,
    url: &str,
    push: &Option<String>,
    create: bool,
) -> Result<Vec<u8>> {
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || name == "HEAD"
        || !git2::Reference::is_valid_name(&format!("refs/remotes/{name}/probe"))
    {
        return Err("INVALID_REMOTE_NAME".into());
    }
    endpoint(url)?;
    if let Some(push) = push {
        endpoint(push)?;
    }
    remote_remove::entries(before)?;
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, before).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    let section = format!("remote.{name}");
    let mut keys = Vec::new();
    let mut remote_names = std::collections::HashSet::new();
    {
        let mut entries = config.entries(None).map_err(|_| "INVALID_REMOTE_CONFIG")?;
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|_| "INVALID_REMOTE_CONFIG")?;
            let key = entry.name().map_err(|_| "INVALID_REMOTE_CONFIG")?;
            if let Some((remote, _)) = key.strip_prefix("remote.").and_then(|k| k.rsplit_once('.'))
            {
                remote_names.insert(remote.to_string());
            }
            if key.rsplit_once('.').is_some_and(|(s, _)| s == section) {
                keys.push(key.to_string());
            }
        }
    }
    if create && remote_names.len() >= MAX_FILES {
        return Err("LIMIT: remotes".into());
    }
    if create == !keys.is_empty() {
        return Err(if create {
            "REMOTE_EXISTS"
        } else {
            "REMOTE_NOT_FOUND"
        }
        .into());
    }
    for suffix in ["url", "pushurl"] {
        let key = format!("{section}.{suffix}");
        if keys.contains(&key) {
            config
                .remove_multivar(&key, ".*")
                .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
        }
    }
    config
        .set_str(&format!("{section}.url"), url)
        .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    if let Some(push) = push {
        config
            .set_str(&format!("{section}.pushurl"), push)
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    if create {
        config
            .set_str(
                &format!("{section}.fetch"),
                &format!("+refs/heads/*:refs/remotes/{name}/*"),
            )
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    drop(config);
    let after = bounded(&path, 65536)?;
    remote_remove::entries(&after)?;
    Ok(after)
}
fn edited_policy(before: &[u8], name: &str, policy: TagPolicy) -> Result<Vec<u8>> {
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/remotes/{name}/probe"))
    {
        return Err("INVALID_REMOTE_NAME".into());
    }
    let entries = remote_remove::entries(before)?;
    let section = format!("remote.{name}");
    if !entries
        .iter()
        .any(|(key, _)| key.rsplit_once('.').is_some_and(|(s, _)| s == section))
    {
        return Err("REMOTE_NOT_FOUND".into());
    }
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, before).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    let key = format!("{section}.tagopt");
    if entries.iter().any(|(k, _)| k == &key) {
        config
            .remove_multivar(&key, ".*")
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    match policy {
        TagPolicy::Auto => {}
        TagPolicy::All => config
            .set_str(&key, "--tags")
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?,
        TagPolicy::None => config
            .set_str(&key, "--no-tags")
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?,
    }
    drop(config);
    let after = bounded(&path, 65536)?;
    remote_remove::entries(&after)?;
    Ok(after)
}
struct ConfigLock {
    path: PathBuf,
    identity: String,
}
impl ConfigLock {
    fn acquire(root: &Path) -> Result<Self> {
        let path = root.join(".git/config.lock");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "CONFIG_LOCKED")?;
        Ok(Self {
            identity: lease::identity(&path)?,
            path,
        })
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        if lease::identity(&self.path).ok().as_ref() == Some(&self.identity)
            && bounded(&self.path, 1).ok().as_deref() == Some(&[])
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
fn apply(root: &Path, record: &Record) -> Result<()> {
    let expected = if let Some(configuration)=&record.gitflow_edit {
        if record.version!=5||record.tag_policy.is_some()||record.upstream.is_some()||record.identity_edit.is_some()||!record.name.is_empty()||!record.url.is_empty()||record.push_url.is_some()||record.create{return Err("INVALID_REMOTE_JOURNAL".into());}
        edited_gitflow(&record.before,configuration)?
    } else { match (
        record.version,
        record.tag_policy,
        &record.upstream,
        &record.identity_edit,
    ) {
        (4, None, None, Some(edit))
            if record.name.is_empty()
                && record.url.is_empty()
                && record.push_url.is_none()
                && !record.create =>
        {
            edited_identity(&record.before, &edit.identity)?
        }
        (3, None, Some(edit), None)
            if record.name.is_empty()
                && record.url.is_empty()
                && record.push_url.is_none()
                && !record.create =>
        {
            edited_upstream(&record.before, edit)?
        }
        (1, None, None, None) => edited(
            &record.before,
            &record.name,
            &record.url,
            &record.push_url,
            record.create,
        )?,
        (2, Some(policy), None, None)
            if record.url.is_empty() && record.push_url.is_none() && !record.create =>
        {
            edited_policy(&record.before, &record.name, policy)?
        }
        _ => return Err("INVALID_REMOTE_JOURNAL".into()),
    }};
    if record.after != expected {
        return Err("INVALID_REMOTE_JOURNAL".into());
    }
    let path = root.join(".git/config");
    let now = bounded(&path, 65536)?;
    if now != record.before && now != record.after {
        return Err("RECOVERY_CONFLICT: remote config changed".into());
    }
    checkpoint(root, 0);
    if now != record.after {
        let mut temp = tempfile::NamedTempFile::new_in(root.join(".git")).map_err(err)?;
        std::io::Write::write_all(&mut temp, &record.after).map_err(err)?;
        temp.as_file().sync_all().map_err(err)?;
        if bounded(&path, 65536)? != record.before {
            return Err("RECOVERY_CONFLICT: remote config changed".into());
        }
        temp.persist(&path).map_err(err)?;
        checkout::sync_dir(&root.join(".git"))?;
    }
    checkpoint(root, 1);
    Ok(())
}
pub(super) fn execute(
    session: &mut Session,
    revision: &str,
    name: String,
    url: String,
    push_url: Option<String>,
    create: bool,
) -> Result<()> {
    // Only new edits use the SSH parser. Historical v1 journal replay keeps its
    // original validation so previously accepted configuration remains recoverable.
    for value in std::iter::once(&url).chain(push_url.iter()) {
        if value.starts_with("ssh://")
            || (!value.contains("://") && !Path::new(value).is_absolute() && value.contains(':'))
        {
            crate::ssh_endpoint::parse(value)?;
        }
    }
    let _lock = ConfigLock::acquire(&session.root)?;
    session.fresh(revision)?;
    let before = bounded(&session.root.join(".git/config"), 65536)?;
    let after = edited(&before, &name, &url, &push_url, create)?;
    let record = Record {
        version: 1,
        tag_policy: None,
        upstream: None,
        identity_edit: None,
        gitflow_edit: None,
        name,
        url,
        push_url,
        create,
        before,
        after,
    };
    publish(session, &record)
}
pub(super) fn execute_tag_policy(
    session: &mut Session,
    revision: &str,
    name: String,
    policy: TagPolicy,
) -> Result<()> {
    let _lock = ConfigLock::acquire(&session.root)?;
    session.fresh(revision)?;
    let before = bounded(&session.root.join(".git/config"), 65536)?;
    let after = edited_policy(&before, &name, policy)?;
    let record = Record {
        version: 2,
        tag_policy: Some(policy),
        upstream: None,
        identity_edit: None,
        gitflow_edit: None,
        name,
        url: String::new(),
        push_url: None,
        create: false,
        before,
        after,
    };
    publish(session, &record)
}
fn publish(session: &Session, record: &Record) -> Result<()> {
    let dir = session.root.join(JOURNAL);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir).map_err(err)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    journal_locks::record_paths(
        &session.root,
        &dir,
        vec!["index.lock".into(), "config.lock".into()],
    )?;
    checkout::sync_dir(&session.root.join(".git"))?;
    apply(&session.root, record)
}
pub(super) fn recover(root: &Path) -> Result<()> {
    let dir = root.join(JOURNAL);
    let m = fs::symlink_metadata(&dir).map_err(err)?;
    if !m.is_dir() || is_link(&m) {
        return Err("INVALID_REMOTE_JOURNAL".into());
    }
    let record: Record = serde_json::from_slice(&bounded(&dir.join("record.json"), 1024 * 1024)?)
        .map_err(|_| "INVALID_REMOTE_JOURNAL")?;
    let _lock = ConfigLock::acquire(root)?;
    journal_locks::record_paths(root, &dir, vec!["index.lock".into(), "config.lock".into()])?;
    apply(root, &record)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!("{JOURNAL}-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_REMOTE_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_REMOTE_STEP").ok().as_deref()
            == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/remote-test-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        repo.index().unwrap().write().unwrap();
        fs::write(d.path().join("dirty.txt"), "keep me").unwrap();
        d
    }
    fn action(create: bool) -> Action {
        Action::SetRemote {
            name: "origin".into(),
            url: "https://example.invalid/repo.git".into(),
            push_url: Some("git@example.invalid:repo.git".into()),
            create,
        }
    }
    #[test]
    fn config_policy_remote_words_survive_publication_and_reopen() {
        let d = fixture();
        let mut session = Session::new(d.path().into()).unwrap();
        for create in [true, false] {
            let snapshot = session.snapshot().unwrap();
            session
                .mutate(
                    &snapshot.revision,
                    Action::SetRemote {
                        name: "design-filter".into(),
                        url: "https://example.invalid/include/worktree/extensions.git".into(),
                        push_url: None,
                        create,
                    },
                )
                .unwrap();
            session = Session::new(d.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(snapshot.write_block.is_none());
            assert_eq!(snapshot.remotes[0].name, "design-filter");
        }
        assert_eq!(
            fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
            "keep me"
        );
    }
    #[test]
    fn add_update_remote_preserves_other_config_refs_and_worktree() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let head = fs::read(d.path().join(".git/HEAD")).unwrap();
        let a = s.snapshot().unwrap();
        let b = s.mutate(&a.revision, action(true)).unwrap();
        assert_eq!(b.remotes[0].name, "origin");
        assert_eq!(
            b.remotes[0].fetch_refspecs,
            vec!["+refs/heads/*:refs/remotes/origin/*"]
        );
        let before = fs::read(d.path().join(".git/config")).unwrap();
        assert!(s
            .mutate(&b.revision, action(true))
            .unwrap_err()
            .contains("REMOTE_EXISTS"));
        assert_eq!(fs::read(d.path().join(".git/config")).unwrap(), before);
        let mut c = Config::open(&d.path().join(".git/config")).unwrap();
        c.set_str("branch.master.remote", "origin").unwrap();
        c.set_multivar(
            "remote.origin.url",
            "^$",
            "https://example.invalid/second.git",
        )
        .unwrap();
        c.set_str("remote.origin.prune", "true").unwrap();
        drop(c);
        assert!(s
            .mutate(&b.revision, action(false))
            .unwrap_err()
            .contains("STALE_STATE"));
        let a = s.snapshot().unwrap();
        let b = s
            .mutate(
                &a.revision,
                Action::SetRemote {
                    name: "origin".into(),
                    url: "ssh://git@example.invalid/revised.git".into(),
                    push_url: None,
                    create: false,
                },
            )
            .unwrap();
        assert_eq!(
            b.remotes[0].fetch_urls,
            vec!["ssh://[redacted]@example.invalid/revised.git"]
        );
        assert!(b.remotes[0].push_urls.is_empty());
        let c = Config::open(&d.path().join(".git/config")).unwrap();
        assert_eq!(c.get_string("branch.master.remote").unwrap(), "origin");
        assert!(c.get_bool("remote.origin.prune").unwrap());
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
        assert_eq!(
            fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
            "keep me"
        );
        assert!(!pending(d.path()));
    }
    #[test]
    fn remote_input_and_foreign_lock_fail_without_config_changes() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/config")).unwrap();
        for url in [
            "ext::helper",
            "https://user:secret@example.invalid/r",
            "https://example.invalid:70000/r",
            "https://127.1/r",
            "https://example.invalid/a/../r",
            "https://example.invalid/r%0a",
            "https://%65xample.invalid/r",
            "https://host/r?token=secret",
            "http://host/r",
            "relative/path",
            "ssh://user:secret@host/r",
            "https://host/r\n[include]",
        ] {
            assert!(
                s.mutate(
                    &a.revision,
                    Action::SetRemote {
                        name: "origin".into(),
                        url: url.into(),
                        push_url: None,
                        create: true
                    }
                )
                .is_err(),
                "{url}"
            );
            assert_eq!(fs::read(d.path().join(".git/config")).unwrap(), before);
            assert!(!pending(d.path()));
        }
        fs::write(d.path().join(".git/config.lock"), "foreign").unwrap();
        assert!(s
            .mutate(&a.revision, action(true))
            .unwrap_err()
            .contains("CONFIG_LOCKED"));
        assert_eq!(
            fs::read_to_string(d.path().join(".git/config.lock")).unwrap(),
            "foreign"
        );
        assert_eq!(fs::read(d.path().join(".git/config")).unwrap(), before);
    }
    #[test]
    fn remote_limit_refusal_preserves_openable_config_and_allows_existing_update() {
        let d = fixture();
        let path = d.path().join(".git/config");
        let mut bytes = fs::read(&path).unwrap();
        for i in 0..MAX_FILES {
            bytes.extend_from_slice(format!("\n[remote \"r{i}\"]\nurl = /tmp/r\n").as_bytes());
        }
        fs::write(&path, &bytes).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert_eq!(a.remotes.len(), MAX_FILES);
        assert!(s
            .mutate(&a.revision, action(true))
            .unwrap_err()
            .starts_with("LIMIT: remotes"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!pending(d.path()));
        let b = s
            .mutate(
                &a.revision,
                Action::SetRemote {
                    name: "r0".into(),
                    url: "https://example.invalid/r".into(),
                    push_url: None,
                    create: false,
                },
            )
            .unwrap();
        assert_eq!(b.remotes.len(), MAX_FILES);
        assert!(!b.recovery_pending);
    }
    fn upstream_action(name: &str, remote: &str, branch: &str) -> Action {
        Action::SetUpstream {
            name: name.into(),
            target: Some(UpstreamTarget {
                remote: remote.into(),
                branch: branch.into(),
            }),
        }
    }
    #[test]
    fn upstream_git_parity_replace_clear_preserve_preferences_and_worktree() {
        let d = fixture();
        let root = d.path();
        let repo = Repository::open(root).unwrap();
        repo.remote("origin", "https://example.invalid/repo")
            .unwrap();
        let head = repo.head().unwrap();
        let name = head.shorthand().unwrap().to_string();
        repo.reference(
            "refs/remotes/origin/release/v2",
            head.target().unwrap(),
            false,
            "fixture",
        )
        .unwrap();
        let path = root.join(".git/config");
        let baseline = fs::read(&path).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["branch", "--set-upstream-to=origin/release/v2", &name])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let expected = upstream_config(
            &remote_remove::entries(&fs::read(&path).unwrap()).unwrap(),
            &name,
        );
        fs::write(&path, baseline).unwrap();
        let mut c = Config::open(&path).unwrap();
        c.set_str(&format!("branch.{name}.rebase"), "true").unwrap();
        c.set_str(&format!("branch.{name}.pushremote"), "elsewhere")
            .unwrap();
        c.set_str(&format!("branch.{name}.remote"), "old").unwrap();
        c.set_multivar(&format!("branch.{name}.merge"), "a^", "refs/heads/old1")
            .unwrap();
        c.set_multivar(&format!("branch.{name}.merge"), "a^", "refs/heads/old2")
            .unwrap();
        drop(c);
        let protected =
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
        let unrelated = |b: &[u8]| {
            remote_remove::entries(b)
                .unwrap()
                .into_iter()
                .filter(|(k, _)| {
                    k != &format!("branch.{name}.remote") && k != &format!("branch.{name}.merge")
                })
                .collect::<Vec<_>>()
        };
        let other = unrelated(&fs::read(&path).unwrap());
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        let next = session
            .mutate(
                &snap.revision,
                upstream_action(&name, "origin", "release/v2"),
            )
            .unwrap();
        let actual = next
            .branches
            .iter()
            .find(|b| b.name == name)
            .unwrap()
            .upstream
            .as_ref()
            .unwrap();
        assert_eq!(actual.remotes, expected.remotes);
        assert_eq!(actual.merges, expected.merges);
        // Pull immediately resolves the newly saved upstream, including rebase policy.
        let selection = pull::selection(&mut session, &next.revision, "origin").unwrap_err();
        assert!(selection.contains("PULL_DIRTY_WORKTREE"));
        fs::remove_file(root.join("dirty.txt")).unwrap();
        let clean = session.snapshot().unwrap();
        let selected = pull::selection(&mut session, &clean.revision, "origin").unwrap();
        assert_eq!(selected.source, "refs/heads/release/v2");
        assert_eq!(selected.mode_label(), "rebase");
        fs::write(root.join("dirty.txt"), "keep me").unwrap();
        let snap = session.snapshot().unwrap();
        let cleared = session
            .mutate(
                &snap.revision,
                Action::SetUpstream {
                    name: name.clone(),
                    target: None,
                },
            )
            .unwrap();
        let value = cleared
            .branches
            .iter()
            .find(|b| b.name == name)
            .unwrap()
            .upstream
            .as_ref()
            .unwrap();
        assert!(value.remotes.is_empty() && value.merges.is_empty());
        assert_eq!(unrelated(&fs::read(&path).unwrap()), other);
        assert_eq!(
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap()),
            protected
        );
        assert!(!cleared.recovery_pending);
    }
    #[test]
    fn upstream_refusals_stale_lock_and_missing_branch_leave_config_intact() {
        let d = fixture();
        let root = d.path();
        Repository::open(root)
            .unwrap()
            .remote("origin", "https://example.invalid/r")
            .unwrap();
        let path = root.join(".git/config");
        let before = fs::read(&path).unwrap();
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        for action in [
            upstream_action("missing", "origin", "main"),
            upstream_action(&snap.branch, "missing", "main"),
            upstream_action(&snap.branch, ".", "main"),
            upstream_action(&snap.branch, "origin", "../bad"),
            upstream_action(&snap.branch, "origin", "a\n[include]"),
        ] {
            assert!(session.mutate(&snap.revision, action).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(!pending(root));
        }
        fs::write(root.join(".git/config.lock"), "external").unwrap();
        assert!(session
            .mutate(
                &snap.revision,
                upstream_action(&snap.branch, "origin", "main")
            )
            .unwrap_err()
            .contains("CONFIG_LOCKED"));
        assert_eq!(
            fs::read(root.join(".git/config.lock")).unwrap(),
            b"external"
        );
        fs::remove_file(root.join(".git/config.lock")).unwrap();
        let mut changed = before.clone();
        changed.extend_from_slice(b"\n[other]\nx = keep\n");
        fs::write(&path, &changed).unwrap();
        assert!(session
            .mutate(
                &snap.revision,
                upstream_action(&snap.branch, "origin", "main")
            )
            .unwrap_err()
            .contains("STALE_STATE"));
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
    #[test]
    fn upstream_actual_kills_and_recovery_kills_finish_without_partial_pair() {
        for step in [0, 1] {
            let d = fixture();
            let root = d.path().canonicalize().unwrap();
            Repository::open(&root)
                .unwrap()
                .remote("origin", "https://example.invalid/r")
                .unwrap();
            fs::write(root.join(".git/upstream-test"), "1").unwrap();
            let before =
                [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
            kill(&root, step, false);
            kill(&root, step, true);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(snap.recovery_pending);
            let next = session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!next.recovery_pending);
            let upstream = next
                .branches
                .iter()
                .find(|b| b.current)
                .unwrap()
                .upstream
                .as_ref()
                .unwrap();
            assert_eq!(upstream.remotes, ["origin"]);
            assert_eq!(upstream.merges, ["refs/heads/release/v2"]);
            assert_eq!(
                [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap()),
                before
            );
        }
    }
    #[test]
    fn upstream_snapshot_hides_url_credentials_and_invalid_config_values() {
        let values = vec![
            (
                "branch.topic.remote".into(),
                "https://user:secret@example.invalid/r?token=secret".into(),
            ),
            ("branch.topic.merge".into(), "refs/heads/bad\nvalue".into()),
        ];
        let state = upstream_config(&values, "topic");
        assert_eq!(state.remotes, ["[unsupported upstream value]"]);
        assert_eq!(state.merges, ["[unsupported upstream value]"]);
        assert!(!serde_json::to_string(&state).unwrap().contains("secret"));
    }
    #[test]
    fn upstream_recovery_rejects_tampering_and_external_config_change() {
        let d = fixture();
        let root = d.path();
        let repo = Repository::open(root).unwrap();
        repo.remote("origin", "https://example.invalid/r").unwrap();
        let name = repo.head().unwrap().shorthand().unwrap().to_string();
        let path = root.join(".git/config");
        let before = fs::read(&path).unwrap();
        let edit = UpstreamEdit {
            branch: name,
            target: Some(UpstreamTarget {
                remote: "origin".into(),
                branch: "main".into(),
            }),
        };
        let after = edited_upstream(&before, &edit).unwrap();
        let mut record = Record {
            version: 3,
            tag_policy: None,
            upstream: Some(edit),
            identity_edit: None,
        gitflow_edit: None,
            name: String::new(),
            url: String::new(),
            push_url: None,
            create: false,
            before: before.clone(),
            after: after.clone(),
        };
        record
            .after
            .extend_from_slice(b"\n[other]\nx = overwritten\n");
        assert!(apply(root, &record)
            .unwrap_err()
            .contains("INVALID_REMOTE_JOURNAL"));
        assert_eq!(fs::read(&path).unwrap(), before);
        record.after = after;
        record.version = 1;
        assert!(apply(root, &record)
            .unwrap_err()
            .contains("INVALID_REMOTE_JOURNAL"));
        record.version = 3;
        let mut external = before.clone();
        external.extend_from_slice(b"\n[other]\nx = preserve\n");
        fs::write(&path, &external).unwrap();
        assert!(apply(root, &record)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(&path).unwrap(), external);
    }
    fn identity_action(clear: bool) -> Action {
        Action::SetRepositoryIdentity {
            identity: if clear {
                None
            } else {
                Some(crate::identity_settings::Identity {
                    name: "Work User".into(),
                    email: "work@example.invalid".into(),
                })
            },
        }
    }
    #[test]
    fn repository_identity_set_clear_preserves_config_and_drives_actual_commit() {
        let d = fixture();
        let root = d.path();
        let path = root.join(".git/config");
        let mut config = Config::open(&path).unwrap();
        config.set_str("user.name", "Old").unwrap();
        config.set_multivar("user.name", "a^", "Last").unwrap();
        config.set_str("user.email", "old@example.invalid").unwrap();
        config.set_str("user.useconfigonly", "true").unwrap();
        config.set_str("branch.master.rebase", "true").unwrap();
        drop(config);
        let other = |b: &[u8]| {
            remote_remove::entries(b)
                .unwrap()
                .into_iter()
                .filter(|(k, _)| k != "user.name" && k != "user.email")
                .collect::<Vec<_>>()
        };
        let unrelated = other(&fs::read(&path).unwrap());
        let protected =
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        assert_eq!(snap.repository_identity.identity.unwrap().name, "Last");
        let next = session
            .mutate(&snap.revision, identity_action(false))
            .unwrap();
        let identity = next.repository_identity.identity.unwrap();
        assert_eq!(identity.name, "Work User");
        let git = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "--local", "--get-all", "user.name"])
            .output()
            .unwrap();
        assert!(git.status.success());
        assert_eq!(git.stdout, b"Work User\n");
        assert_eq!(other(&fs::read(&path).unwrap()), unrelated);
        assert_eq!(
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap()),
            protected
        );
        let staged = session.mutate(&next.revision, Action::StageAll).unwrap();
        let committed = session
            .mutate(
                &staged.revision,
                Action::Commit {
                    message: "identity test".into(),
                    author: identity.name.clone(),
                    email: identity.email.clone(),
                },
            )
            .unwrap();
        let repo = Repository::open(root).unwrap();
        let commit = repo
            .find_commit(Oid::from_str(committed.head_oid.as_ref().unwrap()).unwrap())
            .unwrap();
        for signature in [commit.author(), commit.committer()] {
            assert_eq!(signature.name().unwrap(), "Work User");
            assert_eq!(signature.email().unwrap(), "work@example.invalid");
        }
        let cleared = session
            .mutate(&committed.revision, identity_action(true))
            .unwrap();
        assert!(!cleared.repository_identity.configured);
        assert!(cleared.repository_identity.identity.is_none());
        assert_eq!(other(&fs::read(&path).unwrap()), unrelated);
    }
    #[test]
    fn repository_identity_incomplete_invalid_stale_and_lock_guards() {
        let d = fixture();
        let root = d.path();
        let path = root.join(".git/config");
        let mut session = Session::new(root.into()).unwrap();
        let missing = session.snapshot().unwrap();
        assert!(!missing.repository_identity.configured);
        let mut c = Config::open(&path).unwrap();
        c.set_str("user.name", "Only Name").unwrap();
        drop(c);
        let partial = session.snapshot().unwrap();
        assert!(partial.repository_identity.configured);
        assert!(partial.repository_identity.identity.is_none());
        assert!(partial.repository_identity.warning.is_some());
        assert!(session
            .mutate(&missing.revision, identity_action(false))
            .unwrap_err()
            .contains("STALE_STATE"));
        let before = fs::read(&path).unwrap();
        for identity in [
            crate::identity_settings::Identity {
                name: "bad\nname".into(),
                email: "a@b".into(),
            },
            crate::identity_settings::Identity {
                name: "Valid".into(),
                email: "".into(),
            },
        ] {
            assert!(session
                .mutate(
                    &partial.revision,
                    Action::SetRepositoryIdentity {
                        identity: Some(identity)
                    }
                )
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(!pending(root));
        }
        fs::write(root.join(".git/config.lock"), "foreign").unwrap();
        assert!(session
            .mutate(&partial.revision, identity_action(false))
            .unwrap_err()
            .contains("CONFIG_LOCKED"));
        assert_eq!(fs::read(root.join(".git/config.lock")).unwrap(), b"foreign");
        fs::remove_file(root.join(".git/config.lock")).unwrap();
        let fixed = session
            .mutate(&partial.revision, identity_action(false))
            .unwrap();
        assert!(fixed.repository_identity.warning.is_none());
    }
    #[test]
    fn repository_identity_actual_kills_for_set_clear_and_recovery() {
        for clear in [false, true] {
            for step in [0, 1] {
                let d = fixture();
                let root = d.path().canonicalize().unwrap();
                if clear {
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    session
                        .mutate(&snap.revision, identity_action(false))
                        .unwrap();
                }
                fs::write(
                    root.join(".git/identity-test"),
                    if clear { "clear" } else { "set" },
                )
                .unwrap();
                let before = [".git/HEAD", ".git/index", "dirty.txt"]
                    .map(|p| fs::read(root.join(p)).unwrap());
                kill(&root, step, false);
                kill(&root, step, true);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                let next = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!next.recovery_pending);
                assert_eq!(next.repository_identity.configured, !clear);
                assert_eq!(next.repository_identity.identity.is_some(), !clear);
                assert_eq!(
                    [".git/HEAD", ".git/index", "dirty.txt"]
                        .map(|p| fs::read(root.join(p)).unwrap()),
                    before
                );
            }
        }
    }
    #[test]
    fn repository_identity_journal_rejects_mixed_variants_and_tampering() {
        let d = fixture();
        let root = d.path();
        let path = root.join(".git/config");
        let before = fs::read(&path).unwrap();
        let identity = Some(crate::identity_settings::Identity {
            name: "Work".into(),
            email: "work@example.invalid".into(),
        });
        let after = edited_identity(&before, &identity).unwrap();
        let mut record = Record {
            version: 4,
            tag_policy: None,
            upstream: None,
            identity_edit: Some(IdentityEdit { identity }),
        gitflow_edit: None,
            name: String::new(),
            url: String::new(),
            push_url: None,
            create: false,
            before: before.clone(),
            after: after.clone(),
        };
        record.tag_policy = Some(TagPolicy::All);
        assert!(apply(root, &record).is_err());
        record.tag_policy = None;
        record
            .after
            .extend_from_slice(b"\n[user]\nsigningkey = changed\n");
        assert!(apply(root, &record).is_err());
        record.after = after;
        let mut external = before.clone();
        external.extend_from_slice(b"\n[other]\nx = keep\n");
        fs::write(&path, &external).unwrap();
        assert!(apply(root, &record)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(&path).unwrap(), external);
    }
    #[test]
    fn ssh_new_config_refusals_preserve_data_but_legacy_journal_is_recoverable() {
        let d = fixture();
        let root = d.path();
        let path = root.join(".git/config");
        let before = fs::read(&path).unwrap();
        let protected =
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        for value in [
            "ssh://git@host:0/r",
            "ssh://git@host:65536/r",
            "ssh://127.1/r",
            "ssh://host/%00repo",
            "ssh://host/%2e%2e/r",
            "git@host:-option",
            "ext::helper",
        ] {
            for push in [false, true] {
                let action = Action::SetRemote {
                    name: "origin".into(),
                    url: if push {
                        "https://example.invalid/r".into()
                    } else {
                        value.into()
                    },
                    push_url: push.then(|| value.into()),
                    create: true,
                };
                assert!(session.mutate(&snap.revision, action).is_err(), "{value}");
                assert_eq!(fs::read(&path).unwrap(), before);
                assert!(!pending(root));
            }
        }
        // Old v1 accepted arbitrary SSH ports. Recovery must not get stranded
        // just because new edits now reject that address. No transport is used.
        let url = "ssh://git@host:0/r".to_string();
        let after = edited(&before, "origin", &url, &None, true).unwrap();
        let record = Record {
            version: 1,
            tag_policy: None,
            upstream: None,
            identity_edit: None,
        gitflow_edit: None,
            name: "origin".into(),
            url,
            push_url: None,
            create: true,
            before,
            after: after.clone(),
        };
        apply(root, &record).unwrap();
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap()),
            protected
        );
    }
    #[test]
    fn remote_crash_worker() {
        let Ok(root) = std::env::var("BRANCHDESK_REMOTE_ROOT") else {
            return;
        };
        let mut session = Session::new(PathBuf::from(&root)).unwrap();
        let snap = session.snapshot().unwrap();
        session
            .mutate(
                &snap.revision,
                if std::env::var_os("BRANCHDESK_REMOTE_RECOVER").is_some() {
                    Action::RecoverCheckout
                } else if PathBuf::from(&root).join(".git/identity-test").exists() {
                    identity_action(
                        fs::read_to_string(PathBuf::from(&root).join(".git/identity-test"))
                            .unwrap()
                            == "clear",
                    )
                } else if PathBuf::from(&root).join(".git/upstream-test").exists() {
                    Action::SetUpstream {
                        name: snap.branch.clone(),
                        target: Some(UpstreamTarget {
                            remote: "origin".into(),
                            branch: "release/v2".into(),
                        }),
                    }
                } else if PathBuf::from(&root).join(".git/tag-policy-test").exists() {
                    Action::SetRemoteTagPolicy {
                        name: "origin".into(),
                        policy: TagPolicy::All,
                    }
                } else {
                    action(true)
                },
            )
            .unwrap();
    }
    fn kill(root: &Path, step: usize, recovery: bool) {
        let ready = root.join(".git/remote-test-ready");
        let _ = fs::remove_file(&ready);
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "repository::remote_write::tests::remote_crash_worker",
            "--nocapture",
        ])
        .env("BRANCHDESK_REMOTE_ROOT", root)
        .env("BRANCHDESK_REMOTE_STEP", step.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if recovery {
            cmd.env("BRANCHDESK_REMOTE_RECOVER", "1");
        }
        let mut child = cmd.spawn().unwrap();
        let start = std::time::Instant::now();
        while !ready.exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("remote worker exited before checkpoint: {status}");
            }
            if start.elapsed() > std::time::Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("remote checkpoint timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn remote_actual_kills_and_second_recovery_preserve_index_and_finish_once() {
        for step in [0, 1] {
            let d = fixture();
            let root = d.path().canonicalize().unwrap();
            let before = fs::read(root.join(".git/index")).unwrap();
            kill(&root, step, false);
            kill(&root, step, true);
            let mut s = Session::new(root.clone()).unwrap();
            let a = s.snapshot().unwrap();
            assert!(a.recovery_pending);
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert_eq!(b.remotes.len(), 1);
            assert_eq!(b.remotes[0].name, "origin");
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(root.join("dirty.txt")).unwrap(),
                "keep me"
            );
            assert!(!root.join(".git/config.lock").exists());
            assert!(!root.join(".git/index.lock").exists());
        }
    }
    #[test]
    fn remote_recovery_refuses_external_config_edit() {
        let d = fixture();
        let root = d.path().canonicalize().unwrap();
        kill(&root, 0, false);
        let path = root.join(".git/config");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n[external]\n value = keep\n");
        fs::write(&path, &bytes).unwrap();
        let mut s = Session::new(root.clone()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(&a.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(pending(&root));
    }
    #[test]
    fn tag_policy_edits_preserve_endpoints_multivars_refs_and_dirty_data() {
        let d = fixture();
        let root = d.path();
        let repo = Repository::open(root).unwrap();
        repo.remote("origin", "https://user:secret@example.invalid/repo")
            .unwrap();
        let mut config = repo.config().unwrap();
        config
            .set_multivar("remote.origin.url", "a^", "ssh://git@example.invalid/other")
            .unwrap();
        config
            .set_str("remote.origin.pushurl", "git@example.invalid:repo")
            .unwrap();
        config
            .set_str("remote.origin.tagopt", "unsupported")
            .unwrap();
        let without_policy = |bytes: &[u8]| {
            remote_remove::entries(bytes)
                .unwrap()
                .into_iter()
                .filter(|(key, _)| key != "remote.origin.tagopt")
                .collect::<Vec<_>>()
        };
        let original = without_policy(&fs::read(root.join(".git/config")).unwrap());
        let before =
            [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
        let mut session = Session::new(root.into()).unwrap();
        assert_eq!(
            session.snapshot().unwrap().remotes[0].tag_policy,
            "unsupported"
        );
        for (policy, expected) in [
            (TagPolicy::All, "all"),
            (TagPolicy::None, "none"),
            (TagPolicy::Auto, "auto"),
        ] {
            let snapshot = session.snapshot().unwrap();
            let next = session
                .mutate(
                    &snapshot.revision,
                    Action::SetRemoteTagPolicy {
                        name: "origin".into(),
                        policy,
                    },
                )
                .unwrap();
            assert_eq!(next.remotes[0].tag_policy, expected);
            assert_eq!(
                without_policy(&fs::read(root.join(".git/config")).unwrap()),
                original
            );
            assert_eq!(
                [".git/HEAD", ".git/index", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap()),
                before
            );
        }
        assert!(
            !remote_remove::entries(&fs::read(root.join(".git/config")).unwrap())
                .unwrap()
                .iter()
                .any(|(key, _)| key == "remote.origin.tagopt")
        );
        let snapshot = session.snapshot().unwrap();
        config.set_str("remote.origin.tagopt", "--tags").unwrap();
        let bytes = fs::read(root.join(".git/config")).unwrap();
        assert!(session
            .mutate(
                &snapshot.revision,
                Action::SetRemoteTagPolicy {
                    name: "origin".into(),
                    policy: TagPolicy::None
                }
            )
            .unwrap_err()
            .contains("STALE_STATE"));
        assert_eq!(fs::read(root.join(".git/config")).unwrap(), bytes);
        assert!(serde_json::from_str::<Action>(
            r#"{"kind":"setRemoteTagPolicy","name":"origin","policy":"invalid"}"#
        )
        .is_err());
    }
    #[test]
    fn tag_policy_actual_kills_and_second_recovery_preserve_endpoints() {
        for step in [0, 1] {
            let d = fixture();
            let root = d.path().canonicalize().unwrap();
            Repository::open(&root)
                .unwrap()
                .remote("origin", "https://example.invalid/repo")
                .unwrap();
            fs::write(root.join(".git/tag-policy-test"), "fixture").unwrap();
            let original =
                remote_remove::entries(&fs::read(root.join(".git/config")).unwrap()).unwrap();
            kill(&root, step, false);
            kill(&root, step, true);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let next = session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(next.remotes[0].tag_policy, "all");
            assert!(!next.recovery_pending);
            assert_eq!(
                remote_remove::entries(&fs::read(root.join(".git/config")).unwrap())
                    .unwrap()
                    .into_iter()
                    .filter(|(key, _)| key != "remote.origin.tagopt")
                    .collect::<Vec<_>>(),
                original
            );
            assert_eq!(
                fs::read_to_string(root.join("dirty.txt")).unwrap(),
                "keep me"
            );
        }
    }
    #[test]
    fn tag_policy_journal_cannot_rewrite_other_configuration() {
        let d = fixture();
        let root = d.path();
        Repository::open(root)
            .unwrap()
            .remote("origin", "https://example.invalid/repo")
            .unwrap();
        let before = fs::read(root.join(".git/config")).unwrap();
        let after = edited_policy(&before, "origin", TagPolicy::All).unwrap();
        let mut record = Record {
            version: 2,
            tag_policy: Some(TagPolicy::All),
            upstream: None,
            identity_edit: None,
        gitflow_edit: None,
            name: "origin".into(),
            url: String::new(),
            push_url: None,
            create: false,
            before: before.clone(),
            after: after.clone(),
        };
        record
            .after
            .extend_from_slice(b"\n[other]\nsetting = overwritten\n");
        assert!(apply(root, &record).is_err());
        record.after = after;
        record.url = "https://example.invalid/changed".into();
        assert!(apply(root, &record).is_err());
        record.url.clear();
        record.version = 1;
        assert!(apply(root, &record).is_err());
        assert_eq!(fs::read(root.join(".git/config")).unwrap(), before);
    }
}

fn edited_gitflow(before:&[u8], configuration:&gitflow::Configuration)->Result<Vec<u8>> {
 configuration.validate()?;
 let entries=remote_remove::entries(before)?;
 let dir=tempfile::tempdir().map_err(err)?;
 let path=dir.path().join("config");fs::write(&path,before).map_err(err)?;
 let mut config=Config::open(&path).map_err(|_|"INVALID_GITFLOW_CONFIG")?;
 for (key,value) in configuration.pairs(){
  if entries.iter().any(|(name,_)|name==key){config.remove_multivar(key,".*").map_err(|_|"GITFLOW_CONFIG_EDIT_FAILED")?;}
  config.set_str(key,value).map_err(|_|"GITFLOW_CONFIG_EDIT_FAILED")?;
 }
 drop(config);let after=bounded(&path,65536)?;remote_remove::entries(&after)?;Ok(after)
}
pub(super) fn execute_gitflow(session:&mut Session, revision:&str, configuration:gitflow::Configuration)->Result<()> {
 configuration.validate()?;
 let _lock=ConfigLock::acquire(&session.root)?;session.fresh(revision)?;
 let repo=open(&session.root)?;
 for branch in [&configuration.main,&configuration.develop]{repo.find_reference(&format!("refs/heads/{branch}")).and_then(|reference|reference.peel_to_commit()).map_err(|_|"GITFLOW_BRANCH_MISSING")?;}
 let before=bounded(&session.root.join(".git/config"),65536)?;
 let after=edited_gitflow(&before,&configuration)?;
 let record=Record{version:5,tag_policy:None,upstream:None,identity_edit:None,gitflow_edit:Some(configuration),name:String::new(),url:String::new(),push_url:None,create:false,before,after};
 publish(session,&record)
}
