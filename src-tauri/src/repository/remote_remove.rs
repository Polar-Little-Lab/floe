//! Local remote removal with guarded, resumable metadata publication.
use super::*;
use branch_rename::{read_optional, without_packed_ref, Change};
const JOURNAL: &str = ".git/branchdesk-remote-remove";
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub name: String,
    pub removed_refs: Vec<String>,
    pub preserved_refs: Vec<String>,
    pub removed_config_keys: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    #[serde(default)]
    prune_advertisement: Option<crate::network_worker::Advertisement>,
    name: String,
    refs: Vec<String>,
    changes: Vec<Change>,
}
pub(super) fn entries(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let result=read_only_entries(bytes)?;
    if result.iter().any(|(key,_)|key=="core.worktree"){return Err("UNSUPPORTED_CONFIG".into());}
    Ok(result)
}
// Caller must validate any core.worktree pointer before using a repository layout.
pub(super) fn read_only_entries(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    config_policy::preflight(bytes)?;
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, bytes).map_err(err)?;
    let config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    let mut result = Vec::new();
    let mut values = config.entries(None).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    while let Some(e) = values.next() {
        let e = e.map_err(|_| "INVALID_REMOTE_CONFIG")?;
        result.push((
            e.name().map_err(|_| "INVALID_REMOTE_CONFIG")?.into(),
            e.value().map_err(|_| "INVALID_REMOTE_CONFIG")?.into(),
        ));
    }
    Ok(result)
}
#[derive(Default)]
struct FetchRules {
    positive: Vec<(String, Option<String>)>,
    negative: Vec<String>,
}
fn pattern_capture<'a>(pattern: &str, value: &'a str) -> Option<&'a str> {
    let (start, end) = pattern.split_once('*')?;
    value.strip_prefix(start)?.strip_suffix(end)
}
fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern.contains('*') {
        pattern_capture(pattern, value).is_some()
    } else {
        pattern == value
    }
}
fn valid_pattern(value: &str) -> bool {
    !value.is_empty()
        && value.matches('*').count() <= 1
        && git2::Reference::is_valid_name(&format!("refs/{}", value.replace('*', "probe")))
}
impl FetchRules {
    fn add(&mut self, value: &str) -> Result<()> {
        if let Some(src) = value.strip_prefix('^') {
            let src = if src == "@" { "HEAD" } else { src };
            if !valid_pattern(src)
                || src.contains(':')
                || (src.len() == 40 && src.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("UNSUPPORTED_FETCH_REFSPEC".into());
            }
            self.negative.push(src.into());
            return Ok(());
        }
        let spec = value.strip_prefix('+').unwrap_or(value);
        let (src, dst) = spec
            .split_once(':')
            .map(|(s, d)| (s, Some(d)))
            .unwrap_or((spec, None));
        let src = if src == "@" { "HEAD" } else { src };
        if (!src.is_empty() && !valid_pattern(src))
            || dst.is_some_and(|d| !d.is_empty() && !valid_pattern(d))
            || src.matches('*').count() != dst.unwrap_or("").matches('*').count()
        {
            return Err("UNSUPPORTED_FETCH_REFSPEC".into());
        }
        self.positive
            .push((src.into(), dst.filter(|d| !d.is_empty()).map(str::to_owned)));
        Ok(())
    }
    fn tracks(&self, reference: &str) -> bool {
        // Negative rules operate on source names, reconstructed from destinations.
        // Exact-query handling follows Git's refspec_find_negative_match behavior.
        let excluded = self.positive.iter().any(|(src, dst)| {
            let mapped = if src.contains('*') {
                dst.as_ref()
                    .and_then(|d| pattern_capture(d, reference))
                    .map(|capture| src.replacen('*', capture, 1))
            } else if reference == src {
                Some(src.clone())
            } else {
                None
            };
            mapped.is_some_and(|source| self.negative.iter().any(|n| pattern_matches(n, &source)))
        });
        !excluded
            && self
                .positive
                .iter()
                .any(|(_, dst)| dst.as_ref().is_some_and(|d| pattern_matches(d, reference)))
    }
}
fn selection(bytes: &[u8], name: &str, refs: &[String]) -> Result<Preview> {
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/remotes/{name}/probe"))
    {
        return Err("INVALID_REMOTE_NAME".into());
    }
    if refs.len() > MAX_FILES {
        return Err("LIMIT: refs".into());
    }
    let values = entries(bytes)?;
    let section = format!("remote.{name}");
    if !values
        .iter()
        .any(|(k, _)| k.rsplit_once('.').is_some_and(|(s, _)| s == section))
    {
        return Err("REMOTE_NOT_FOUND".into());
    }
    let mut rules = std::collections::BTreeMap::<String, FetchRules>::new();
    let mut removed_keys = Vec::new();
    let mut branches = Vec::new();
    for (key, value) in &values {
        if let Some((s, k)) = key.rsplit_once('.') {
            if s == section {
                removed_keys.push(key.clone());
            }
            if k == "fetch" && s.starts_with("remote.") {
                rules.entry(s.into()).or_default().add(value)?;
            }
            if s.starts_with("branch.") && k == "remote" && value == name {
                branches.push(s.to_string());
            }
            if s.starts_with("branch.") && k == "pushremote" && value == name {
                removed_keys.push(key.clone());
            }
        }
        if key == "remote.pushdefault" && value == name {
            removed_keys.push(key.clone());
        }
    }
    for (key, _) in &values {
        if key.rsplit_once('.').is_some_and(|(s, k)| {
            branches.iter().any(|b| b == s) && matches!(k, "remote" | "merge")
        }) {
            removed_keys.push(key.clone());
        }
    }
    let mut removed_refs = Vec::new();
    let mut preserved_refs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in refs {
        if !r.starts_with("refs/") || !git2::Reference::is_valid_name(r) || !seen.insert(r) {
            return Err("INVALID_REMOTE_REFS".into());
        }
        if rules.get(&section).is_some_and(|rules| rules.tracks(r)) {
            if r.starts_with("refs/remotes/")
                && !rules
                    .iter()
                    .any(|(name, rules)| name != &section && rules.tracks(r))
            {
                removed_refs.push(r.clone());
            } else {
                preserved_refs.push(r.clone());
            }
        }
    }
    removed_refs.sort();
    preserved_refs.sort();
    removed_keys.sort();
    removed_keys.dedup();
    Ok(Preview {
        name: name.into(),
        removed_refs,
        preserved_refs,
        removed_config_keys: removed_keys,
    })
}

fn select_mode(
    bytes: &[u8],
    name: &str,
    refs: &[String],
    advertisement: Option<&crate::network_worker::Advertisement>,
) -> Result<Preview> {
    let Some(ad) = advertisement else {
        return selection(bytes, name, refs);
    };
    // Keep the existing local-namespace and shared-remote preservation policy.
    let mut preview = selection(bytes, name, refs)?;
    let values = entries(bytes)?;
    let urls: Vec<_> = values
        .iter()
        .filter(|(k, _)| k == &format!("remote.{name}.url"))
        .collect();
    if urls.len() != 1 || remote_endpoint::https(&urls[0].1)?.as_str() != ad.endpoint {
        return Err("PRUNE_ENDPOINT_MISMATCH".into());
    }
    if ad.refs.len() > 2000 {
        return Err("REMOTE_REF_LIMIT".into());
    }
    let mut advertised = std::collections::HashSet::new();
    for r in &ad.refs {
        let refname = r.name.strip_suffix("^{}").unwrap_or(&r.name);
        if r.name.len() > 4096
            || (refname != "HEAD"
                && (!refname.starts_with("refs/") || !git2::Reference::is_valid_name(refname)))
            || r.oid.len() != 40
            || !r.oid.bytes().all(|b| b.is_ascii_hexdigit())
            || r.oid.bytes().all(|b| b == b'0')
            || !advertised.insert(r.name.as_str())
            || r.symbolic_target.as_ref().is_some_and(|s| {
                s.len() > 4096 || !s.starts_with("refs/") || !git2::Reference::is_valid_name(s)
            })
        {
            return Err("INVALID_PRUNE_ADVERTISEMENT".into());
        }
    }
    let mut rules = FetchRules::default();
    for (key, value) in &values {
        if key == &format!("remote.{name}.fetch") {
            rules.add(value)?;
        }
    }
    // Ref pruning only: never rewrite configuration, FETCH_HEAD, tags or local branches.
    preview.removed_config_keys.clear();
    preview.removed_refs.retain(|reference| {
        let live = rules.positive.iter().any(|(src, dst)| {
            let Some(dst) = dst else {
                return false;
            };
            let source = if dst.contains('*') {
                pattern_capture(dst, reference).map(|c| src.replacen('*', c, 1))
            } else if dst == reference {
                Some(src.clone())
            } else {
                None
            };
            source.is_some_and(|s| advertised.contains(s.as_str()))
        });
        if live {
            preview.preserved_refs.push(reference.clone());
        }
        !live
    });
    preview.preserved_refs.sort();
    Ok(preview)
}
impl Session {
    pub fn prune_preview(
        &mut self,
        revision: &str,
        name: &str,
        ad: &crate::network_worker::Advertisement,
    ) -> Result<Preview> {
        let snapshot = self.fresh(revision)?;
        if let Some(reason) = snapshot.write_block {
            return Err(format!("READ_ONLY: {reason}"));
        }
        let repo = open(&self.root)?;
        if repo.state() != git2::RepositoryState::Clean
            || (sequence_state::exists(&self.root) || rebase::draft::exists(&self.root))
            || replay_draft::exists(&self.root)
            || merge_draft::exists(&self.root)
        {
            return Err("PRUNE_PENDING_OPERATION".into());
        }
        Ok(prepare_mode(&self.root, &repo, name, Some(ad))?.1)
    }
    pub fn prune_remote(
        &mut self,
        revision: &str,
        name: &str,
        ad: &crate::network_worker::Advertisement,
    ) -> Result<Snapshot> {
        self.prune_preview(revision, name, ad)?;
        let _lease = lease::OperationLease::acquire(&self.root)?;
        let index = IndexLock::acquire(&self.root)?;
        let repo = open(&self.root)?;
        execute_mode(self, &repo, revision, name, Some(ad))?;
        drop(index);
        finish(&self.root)?;
        self.snapshot()
    }
}

fn config_after(before: &[u8], keys: &[String]) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, before).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    for key in keys {
        config
            .remove_multivar(key, ".*")
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    drop(config);
    bounded(&path, 65536)
}
fn prepare(root: &Path, repo: &Repository, name: &str) -> Result<(Plan, Preview)> {
    prepare_mode(root, repo, name, None)
}
fn prepare_mode(
    root: &Path,
    repo: &Repository,
    name: &str,
    advertisement: Option<&crate::network_worker::Advertisement>,
) -> Result<(Plan, Preview)> {
    let mut refs = Vec::new();
    for r in repo.references().map_err(err)? {
        let r = r.map_err(err)?;
        // Match Git's traversal: existing dangling symbolic refs are not candidates.
        if r.symbolic_target_bytes().is_some() && (advertisement.is_some() || r.resolve().is_err())
        {
            continue;
        }
        refs.push(r.name().map_err(err)?.into());
        if refs.len() > MAX_FILES {
            return Err("LIMIT: refs".into());
        }
    }
    refs.sort();
    let before = bounded(&root.join(".git/config"), 65536)?;
    let preview = select_mode(&before, name, &refs, advertisement)?;
    let mut changes = vec![Change {
        path: "config".into(),
        after: Some(if advertisement.is_some() {
            before.clone()
        } else {
            config_after(&before, &preview.removed_config_keys)?
        }),
        before: Some(before),
    }];
    if let Some(before) = read_optional(root, "packed-refs")? {
        let mut after = before.clone();
        for r in &preview.removed_refs {
            after = without_packed_ref(&after, r)?;
        }
        changes.push(Change {
            path: "packed-refs".into(),
            before: Some(before),
            after: Some(after),
        });
    }
    if !changes.iter().any(|c| c.path == "packed-refs") {
        changes.push(Change {
            path: "packed-refs".into(),
            before: None,
            after: None,
        });
    }
    let mut total_bytes = 0usize;
    for r in &preview.removed_refs {
        for path in [r.clone(), format!("logs/{r}")] {
            let before = read_optional(root, &path)?;
            total_bytes += before.as_ref().map_or(0, Vec::len);
            if total_bytes > 8 * 1024 * 1024 {
                return Err("LIMIT: remote removal metadata".into());
            }
            changes.push(Change {
                before,
                path,
                after: None,
            });
        }
    }
    Ok((
        Plan {
            version: if advertisement.is_some() { 2 } else { 1 },
            prune_advertisement: advertisement.cloned(),
            name: name.into(),
            refs,
            changes,
        },
        preview,
    ))
}
fn verify(plan: &Plan) -> Result<Preview> {
    if !matches!(
        (plan.version, plan.prune_advertisement.is_some()),
        (1, false) | (2, true)
    ) || plan.changes.len() > 2 + MAX_FILES * 2
    {
        return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
    }
    let config = plan
        .changes
        .first()
        .ok_or("INVALID_REMOTE_REMOVE_JOURNAL")?;
    if config.path != "config" {
        return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
    }
    let before = config
        .before
        .as_ref()
        .ok_or("INVALID_REMOTE_REMOVE_JOURNAL")?;
    let preview = select_mode(
        before,
        &plan.name,
        &plan.refs,
        plan.prune_advertisement.as_ref(),
    )?;
    let expected = if plan.version == 2 {
        before.clone()
    } else {
        config_after(before, &preview.removed_config_keys)?
    };
    if config.after.as_ref() != Some(&expected) {
        return Err("INVALID_REMOTE_REMOVE_CONFIG".into());
    }
    let mut seen = std::collections::HashSet::new();
    for change in &plan.changes {
        if !seen.insert(&change.path) {
            return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
        }
        if change.path == "config" {
            continue;
        }
        if change.path == "packed-refs" {
            if change.before.is_none() && change.after.is_none() {
                continue;
            }
            let before = change.before.as_ref().ok_or("INVALID_PACKED_REFS")?;
            let mut after = before.clone();
            for r in &preview.removed_refs {
                after = without_packed_ref(&after, r)?;
            }
            if change.after.as_ref() != Some(&after) {
                return Err("INVALID_REMOTE_REMOVE_PACKED_REFS".into());
            }
        } else if (plan.version == 2
            && change.path.starts_with("refs/")
            && change
                .before
                .as_ref()
                .is_some_and(|b| b.starts_with(b"ref: ")))
            || change.after.is_some()
            || !preview
                .removed_refs
                .iter()
                .any(|r| change.path == *r || change.path == format!("logs/{r}"))
        {
            return Err("INVALID_REMOTE_REMOVE_PATH".into());
        }
    }
    if !seen.contains(&"packed-refs".to_string()) {
        return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
    }
    for r in &preview.removed_refs {
        if !seen.contains(r) || !seen.contains(&format!("logs/{r}")) {
            return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
        }
    }
    Ok(preview)
}
pub(super) struct Locks(Vec<(PathBuf, String)>);
impl Drop for Locks {
    fn drop(&mut self) {
        for (path, id) in &self.0 {
            if lease::identity(path).ok().as_ref() == Some(id)
                && bounded(path, 1).ok().as_deref() == Some(&[])
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn lock_paths(plan: &Plan) -> Result<Vec<String>> {
    let preview = verify(plan)?;
    let mut paths = vec!["config.lock".into(), "packed-refs.lock".into()];
    for r in preview.removed_refs {
        paths.push(format!("{r}.lock"));
        paths.push(format!("logs/{r}.lock"));
    }
    paths.sort();
    if paths.iter().map(|p| p.len() + 256).sum::<usize>() > 900 * 1024 {
        return Err("LIMIT: remote removal locks".into());
    }
    Ok(paths)
}
fn locks(root: &Path, plan: &Plan) -> Result<Locks> {
    acquire_paths(root, lock_paths(plan)?)
}
pub(super) fn acquire_paths(root: &Path, mut paths: Vec<String>) -> Result<Locks> {
    paths.sort();
    paths.dedup();
    if paths.iter().map(|p| p.len() + 256).sum::<usize>() > 900 * 1024 {
        return Err("LIMIT: metadata locks".into());
    }
    let mut held = Locks(Vec::new());
    for path in paths {
        // Validate every existing ancestor before creating lock parent directories.
        let _ = read_optional(root, &path)?;
        let path = root.join(".git").join(path);
        fs::create_dir_all(path.parent().ok_or("INVALID_LOCK_PATH")?).map_err(err)?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "EXTERNAL_LOCK")?;
        held.0.push((path.clone(), lease::identity(&path)?));
    }
    Ok(held)
}
fn record_locks(root: &Path, dir: &Path, plan: &Plan) -> Result<()> {
    let mut paths = lock_paths(plan)?;
    paths.push("index.lock".into());
    journal_locks::record_paths(root, dir, paths)
}
fn apply(root: &Path, plan: &Plan) -> Result<()> {
    let preview = verify(plan)?;
    let repo = open(root)?;
    let mut current_refs = Vec::new();
    for r in repo.references().map_err(err)? {
        let r = r.map_err(err)?;
        if r.symbolic_target_bytes().is_some() && (plan.version == 2 || r.resolve().is_err()) {
            continue;
        }
        current_refs.push(r.name().map_err(err)?.to_string());
        if current_refs.len() > MAX_FILES {
            return Err("LIMIT: refs".into());
        }
    }
    let current = select_mode(
        plan.changes[0].before.as_ref().unwrap(),
        &plan.name,
        &current_refs,
        plan.prune_advertisement.as_ref(),
    )?;
    if current
        .removed_refs
        .iter()
        .any(|r| !preview.removed_refs.contains(r))
    {
        return Err("RECOVERY_CONFLICT: new remote tracking ref".into());
    }
    for change in &plan.changes {
        let now = read_optional(root, &change.path)?;
        if now != change.before && now != change.after {
            return Err(format!("RECOVERY_CONFLICT: {} changed", change.path));
        }
    }
    checkpoint(root, 0);
    for (i, change) in plan.changes.iter().enumerate() {
        let now = read_optional(root, &change.path)?;
        if now != change.after {
            if now != change.before {
                return Err("RECOVERY_CONFLICT: remote metadata changed".into());
            }
            let path = root.join(".git").join(&change.path);
            if let Some(bytes) = &change.after {
                let mut temp =
                    tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(err)?;
                std::io::Write::write_all(&mut temp, bytes).map_err(err)?;
                temp.as_file().sync_all().map_err(err)?;
                temp.persist(&path).map_err(err)?;
            } else if now.is_some() {
                fs::remove_file(&path).map_err(err)?;
            }
            checkout::sync_dir(path.parent().unwrap())?;
        }
        checkpoint(root, i + 1);
    }
    Ok(())
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    name: &str,
) -> Result<()> {
    execute_mode(session, repo, revision, name, None)
}
fn execute_mode(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    name: &str,
    advertisement: Option<&crate::network_worker::Advertisement>,
) -> Result<()> {
    let (initial, _) = prepare_mode(&session.root, repo, name, advertisement)?;
    let _locks = locks(&session.root, &initial)?;
    session.fresh(revision)?;
    let (plan, _) = prepare_mode(&session.root, repo, name, advertisement)?;
    if plan.changes != initial.changes || plan.refs != initial.refs {
        return Err("STALE_STATE".into());
    }
    let dir = session.root.join(JOURNAL);
    #[cfg(unix)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir).map_err(err)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&plan).map_err(err)?,
    )?;
    record_locks(&session.root, &dir, &plan)?;
    checkout::sync_dir(&session.root.join(".git"))?;
    apply(&session.root, &plan)
}
pub(super) fn recover(root: &Path) -> Result<()> {
    let dir = root.join(JOURNAL);
    let m = fs::symlink_metadata(&dir).map_err(err)?;
    if !m.is_dir() || is_link(&m) {
        return Err("INVALID_REMOTE_REMOVE_JOURNAL".into());
    }
    let plan: Plan = serde_json::from_slice(&bounded(&dir.join("record.json"), 64 * 1024 * 1024)?)
        .map_err(|_| "INVALID_REMOTE_REMOVE_JOURNAL")?;
    let _locks = locks(root, &plan)?;
    record_locks(root, &dir, &plan)?;
    apply(root, &plan)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!("{JOURNAL}-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}
impl Session {
    pub fn remote_remove_preview(&mut self, revision: &str, name: &str) -> Result<Preview> {
        self.fresh(revision)?;
        let repo = open(&self.root)?;
        let (_, preview) = prepare(&self.root, &repo, name)?;
        Ok(preview)
    }
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_REMOVE_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_REMOVE_STEP").ok().as_deref()
            == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/remove-test-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::repository) fn git(root: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }
    pub(in crate::repository) fn fixture(kind: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = Repository::init(d.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@invalid").unwrap();
        let tree = r
            .find_tree(r.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let id = r
            .commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        r.index().unwrap().write().unwrap();
        let mut c = r.config().unwrap();
        for name in ["origin", "keep"] {
            c.set_str(
                &format!("remote.{name}.url"),
                "https://example.invalid/repo",
            )
            .unwrap();
            c.set_str(
                &format!("remote.{name}.fetch"),
                &format!("+refs/heads/*:refs/remotes/{name}/*"),
            )
            .unwrap();
            r.reference(&format!("refs/remotes/{name}/main"), id, false, "fixture")
                .unwrap();
        }
        r.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
            false,
            "fixture",
        )
        .unwrap();
        for (k, v) in [
            ("branch.local.remote", "origin"),
            ("branch.local.merge", "refs/heads/main"),
            ("branch.local.pushremote", "origin"),
            ("branch.local.description", "preserve"),
            ("remote.pushdefault", "origin"),
        ] {
            c.set_str(k, v).unwrap();
        }
        match kind {
            "shared" => c
                .set_multivar(
                    "remote.keep.fetch",
                    "^$",
                    "+refs/heads/*:refs/remotes/origin/*",
                )
                .unwrap(),
            "custom" => {
                c.set_str("remote.origin.fetch", "+refs/heads/*:refs/remotes/custom/*")
                    .unwrap();
                r.reference("refs/remotes/custom/main", id, false, "fixture")
                    .unwrap();
            }
            "local" => {
                c.set_str("remote.origin.fetch", "+refs/heads/*:refs/heads/import/*")
                    .unwrap();
                r.reference("refs/heads/import/main", id, false, "fixture")
                    .unwrap();
            }
            "dangling" => {
                r.reference_symbolic(
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/missing",
                    true,
                    "fixture",
                )
                .unwrap();
            }
            "negative-alias" => {
                c.set_str("remote.origin.fetch", "*:refs/remotes/origin/*")
                    .unwrap();
                c.set_multivar("remote.origin.fetch", "^$", "^@").unwrap();
            }
            "negative-own" => c
                .set_multivar("remote.origin.fetch", "^$", "^refs/heads/main")
                .unwrap(),
            "negative-shared" => {
                c.set_multivar(
                    "remote.keep.fetch",
                    "^$",
                    "+refs/heads/*:refs/remotes/origin/*",
                )
                .unwrap();
                c.set_multivar("remote.keep.fetch", "^$", "^refs/heads/main")
                    .unwrap();
            }
            "source-only" => c.set_str("remote.origin.fetch", "refs/heads/main").unwrap(),
            "empty-destination" => c
                .set_str("remote.origin.fetch", "refs/heads/main:")
                .unwrap(),
            "negative-exact" => {
                c.set_str(
                    "remote.origin.fetch",
                    "refs/heads/main:refs/remotes/origin/main",
                )
                .unwrap();
                c.set_multivar("remote.origin.fetch", "^$", "^refs/heads/main")
                    .unwrap();
            }
            "negative-wildcard" => c
                .set_multivar("remote.origin.fetch", "^$", "^refs/heads/*")
                .unwrap(),
            _ => {}
        }
        drop(c);
        if kind == "packed" {
            git(d.path(), &["pack-refs", "--all", "--prune"]);
        }
        fs::write(d.path().join("dirty.txt"), "keep dirty").unwrap();
        d
    }
    pub(in crate::repository) fn refs(root: &Path) -> Vec<String> {
        let mut x = git(root, &["for-each-ref", "--format=%(refname)"])
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        x.sort();
        x
    }
    pub(in crate::repository) fn config(root: &Path) -> Vec<String> {
        let mut x = git(root, &["config", "--local", "--list"])
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        x.sort();
        x
    }
    #[test]
    fn removal_matches_git_cli_for_standard_packed_shared_custom_and_local_destinations() {
        for kind in [
            "standard",
            "packed",
            "shared",
            "custom",
            "local",
            "negative-own",
            "negative-shared",
            "source-only",
            "empty-destination",
            "negative-exact",
            "negative-wildcard",
            "dangling",
            "negative-alias",
        ] {
            let d = fixture(kind);
            let oracle = fixture(kind);
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let head = fs::read(d.path().join(".git/HEAD")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let p = s.remote_remove_preview(&a.revision, "origin").unwrap();
            if kind == "shared" || kind == "local" {
                assert!(p.removed_refs.is_empty());
                assert!(!p.preserved_refs.is_empty());
            }
            git(oracle.path(), &["remote", "remove", "origin"]);
            let b = s
                .mutate(
                    &a.revision,
                    Action::RemoveRemote {
                        name: "origin".into(),
                    },
                )
                .unwrap();
            assert_eq!(refs(d.path()), refs(oracle.path()), "{kind}");
            // for-each-ref hides dangling symbolic refs; compare their raw bytes too.
            assert_eq!(
                read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap(),
                read_optional(oracle.path(), "refs/remotes/origin/HEAD").unwrap(),
                "{kind}"
            );
            assert_eq!(config(d.path()), config(oracle.path()), "{kind}");
            assert_eq!(b.remotes.len(), 1);
            assert_eq!(b.remotes[0].name, "keep");
            assert!(!b.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
            assert_eq!(
                fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
                "keep dirty"
            );
        }
    }
    #[test]
    fn removal_refuses_stale_foreign_locks_and_unsupported_refspec_without_changes() {
        let d = fixture("standard");
        let root = d.path();
        let mut s = Session::new(root.into()).unwrap();
        let a = s.snapshot().unwrap();
        let original = config(root);
        let original_refs = refs(root);
        fs::write(root.join(".git/config.lock"), "foreign").unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::RemoveRemote {
                    name: "origin".into()
                }
            )
            .unwrap_err()
            .contains("EXTERNAL_LOCK"));
        assert_eq!(
            fs::read_to_string(root.join(".git/config.lock")).unwrap(),
            "foreign"
        );
        assert_eq!(config(root), original);
        assert_eq!(refs(root), original_refs);
        assert!(!pending(root));
        fs::remove_file(root.join(".git/config.lock")).unwrap();
        let mut c = Config::open(&root.join(".git/config")).unwrap();
        c.set_str("external.value", "added").unwrap();
        drop(c);
        assert!(s
            .mutate(
                &a.revision,
                Action::RemoveRemote {
                    name: "origin".into()
                }
            )
            .unwrap_err()
            .contains("STALE_STATE"));
        let mut c = Config::open(&root.join(".git/config")).unwrap();
        c.set_multivar(
            "remote.origin.fetch",
            "^$",
            "^refs/heads/private/*:refs/remotes/invalid/*",
        )
        .unwrap();
        drop(c);
        let a = s.snapshot().unwrap();
        let before = config(root);
        assert!(s
            .mutate(
                &a.revision,
                Action::RemoveRemote {
                    name: "origin".into()
                }
            )
            .unwrap_err()
            .contains("UNSUPPORTED_FETCH_REFSPEC"));
        assert_eq!(config(root), before);
        assert_eq!(refs(root), original_refs);
        assert!(!pending(root));
    }
    #[test]
    fn forged_removal_plan_cannot_remove_local_refs_or_unrelated_metadata() {
        let d = fixture("standard");
        let repo = open(d.path()).unwrap();
        let (mut plan, _) = prepare(d.path(), &repo, "origin").unwrap();
        plan.changes.push(Change {
            path: "refs/heads/local".into(),
            before: None,
            after: None,
        });
        assert!(verify(&plan).is_err());
        plan.changes.pop();
        plan.changes[0].after = Some(b"[core]\n bare=true\n".to_vec());
        assert!(verify(&plan).is_err());
    }
    #[test]
    fn remove_worker() {
        let Ok(root) = std::env::var("BRANCHDESK_REMOVE_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut s = Session::new(root.clone()).unwrap();
        let a = s.snapshot().unwrap();
        if root.join(".git/prune-test").exists()
            && std::env::var_os("BRANCHDESK_REMOVE_RECOVER").is_none()
        {
            s.prune_remote(&a.revision, "origin", &empty_ad()).unwrap();
            return;
        }
        s.mutate(
            &a.revision,
            if std::env::var_os("BRANCHDESK_REMOVE_RECOVER").is_some() {
                Action::RecoverCheckout
            } else {
                Action::RemoveRemote {
                    name: "origin".into(),
                }
            },
        )
        .unwrap();
    }
    fn kill(root: &Path, step: usize, recover: bool) {
        let ready = root.join(".git/remove-test-ready");
        let _ = fs::remove_file(&ready);
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "repository::remote_remove::tests::remove_worker",
            "--nocapture",
        ])
        .env("BRANCHDESK_REMOVE_ROOT", root)
        .env("BRANCHDESK_REMOVE_STEP", step.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if recover {
            cmd.env("BRANCHDESK_REMOVE_RECOVER", "1");
        }
        let mut child = cmd.spawn().unwrap();
        let started = std::time::Instant::now();
        while !ready.exists() {
            if let Some(s) = child.try_wait().unwrap() {
                panic!("worker exited: {s}");
            }
            if started.elapsed() > std::time::Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("checkpoint timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn remove_actual_kills_each_metadata_boundary_and_second_recovery() {
        for kind in ["standard", "packed"] {
            for step in 0..=6 {
                let d = fixture(kind);
                let root = d.path().canonicalize().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                kill(&root, step, false);
                kill(&root, step, true);
                let mut s = Session::new(root.clone()).unwrap();
                let a = s.snapshot().unwrap();
                assert!(a.recovery_pending);
                let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert!(!b.recovery_pending);
                assert_eq!(b.remotes.len(), 1);
                assert!(!refs(&root)
                    .iter()
                    .any(|r| r.starts_with("refs/remotes/origin/")));
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(root.join("dirty.txt")).unwrap(),
                    "keep dirty"
                );
            }
        }
    }
    #[test]
    fn negative_and_source_only_removal_resume_each_boundary_without_losing_exclusions() {
        for kind in ["negative-own", "negative-shared", "source-only"] {
            let probe = fixture(kind);
            let repo = open(probe.path()).unwrap();
            let count = prepare(probe.path(), &repo, "origin")
                .unwrap()
                .0
                .changes
                .len();
            for step in 0..=count {
                let d = fixture(kind);
                let root = d.path().canonicalize().unwrap();
                let oracle = fixture(kind);
                let before = fs::read(root.join(".git/index")).unwrap();
                git(oracle.path(), &["remote", "remove", "origin"]);
                kill(&root, step, false);
                kill(&root, step, true);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let result = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!result.recovery_pending);
                assert_eq!(refs(&root), refs(oracle.path()), "{kind} {step}");
                assert_eq!(config(&root), config(oracle.path()), "{kind} {step}");
                assert_eq!(
                    read_optional(&root, "refs/remotes/origin/HEAD").unwrap(),
                    read_optional(oracle.path(), "refs/remotes/origin/HEAD").unwrap()
                );
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), before);
            }
        }
    }
    #[test]
    fn removal_recovery_preserves_external_config_and_new_tracking_refs() {
        for config_edit in [false, true] {
            let d = fixture("standard");
            let root = d.path().canonicalize().unwrap();
            kill(&root, 0, false);
            if config_edit {
                // Simulate an editor ignoring the stale Git lock after process death.
                let path = root.join(".git/config");
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(b"\n[external]\nvalue = keep\n");
                fs::write(path, bytes).unwrap();
            } else {
                let repo = Repository::open(&root).unwrap();
                repo.reference(
                    "refs/remotes/origin/new",
                    repo.head().unwrap().target().unwrap(),
                    false,
                    "external",
                )
                .unwrap();
            }
            let before = config(&root);
            let oldrefs = refs(&root);
            let mut s = Session::new(root.clone()).unwrap();
            let a = s.snapshot().unwrap();
            assert!(s
                .mutate(&a.revision, Action::RecoverCheckout)
                .unwrap_err()
                .contains("RECOVERY_CONFLICT"));
            assert_eq!(config(&root), before);
            assert_eq!(refs(&root), oldrefs);
            assert!(pending(&root));
        }
    }
    fn empty_ad() -> crate::network_worker::Advertisement {
        crate::network_worker::Advertisement {
            endpoint: "https://example.invalid/repo".into(),
            refs: vec![],
        }
    }
    #[test]
    fn prune_matches_git_for_stale_packed_custom_negative_and_source_only_refs() {
        for kind in [
            "standard",
            "packed",
            "custom",
            "negative-own",
            "negative-exact",
            "negative-wildcard",
            "source-only",
            "empty-destination",
            "dangling",
        ] {
            let d = fixture(kind);
            let oracle = fixture(kind);
            let remote = tempfile::tempdir().unwrap();
            Repository::init_bare(remote.path()).unwrap();
            git(
                oracle.path(),
                &[
                    "remote",
                    "set-url",
                    "origin",
                    remote.path().to_str().unwrap(),
                ],
            );
            git(oracle.path(), &["remote", "prune", "origin"]);
            let before = ["config", "index", "HEAD"].map(|p| read_optional(d.path(), p).unwrap());
            let sym = read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            session
                .prune_remote(&snap.revision, "origin", &empty_ad())
                .unwrap();
            assert_eq!(refs(d.path()), refs(oracle.path()), "{kind}");
            assert_eq!(
                read_optional(d.path(), "refs/remotes/origin/HEAD").unwrap(),
                sym
            );
            assert_eq!(
                ["config", "index", "HEAD"].map(|p| read_optional(d.path(), p).unwrap()),
                before
            );
            assert_eq!(
                fs::read_to_string(d.path().join("dirty.txt")).unwrap(),
                "keep dirty"
            );
        }
    }
    #[test]
    fn prune_preserves_live_shared_local_and_tag_refs_and_rejects_bad_advertisements() {
        for kind in ["standard", "shared", "local"] {
            let d = fixture(kind);
            let repo = open(d.path()).unwrap();
            let oid = repo.head().unwrap().target().unwrap();
            repo.reference("refs/tags/local", oid, false, "fixture")
                .unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            let mut ad = empty_ad();
            ad.refs.push(crate::network_worker::RemoteRef {
                name: "refs/heads/main".into(),
                oid: oid.to_string(),
                symbolic_target: None,
            });
            let before = refs(d.path());
            assert!(session
                .prune_preview(&snap.revision, "origin", &ad)
                .unwrap()
                .removed_refs
                .is_empty());
            session.prune_remote(&snap.revision, "origin", &ad).unwrap();
            assert_eq!(refs(d.path()), before);
            let snap = session.snapshot().unwrap();
            ad.endpoint = "https://different.invalid/repo".into();
            assert!(session
                .prune_preview(&snap.revision, "origin", &ad)
                .is_err());
            ad.endpoint = empty_ad().endpoint;
            ad.refs[0].oid = "0".repeat(40);
            assert!(session
                .prune_preview(&snap.revision, "origin", &ad)
                .is_err());
            if kind != "standard" {
                assert!(session
                    .prune_preview(&snap.revision, "origin", &empty_ad())
                    .unwrap()
                    .removed_refs
                    .is_empty());
            }
        }
    }
    #[test]
    fn prune_actual_kills_each_boundary_and_recovery_without_config_loss() {
        for kind in ["standard", "packed"] {
            for step in 0..=4 {
                let d = fixture(kind);
                let root = d.path().canonicalize().unwrap();
                fs::write(root.join(".git/prune-test"), "1").unwrap();
                let before = ["config", "index", "HEAD", "refs/remotes/origin/HEAD"]
                    .map(|p| read_optional(&root, p).unwrap());
                kill(&root, step, false);
                kill(&root, step, true);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let next = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!next.recovery_pending);
                assert!(!refs(&root).contains(&"refs/remotes/origin/main".to_string()));
                assert_eq!(
                    ["config", "index", "HEAD", "refs/remotes/origin/HEAD"]
                        .map(|p| read_optional(&root, p).unwrap()),
                    before
                );
            }
        }
    }
    #[test]
    fn prune_rejects_stale_locks_and_forged_config_or_symbolic_deletion() {
        let d = fixture("standard");
        let root = d.path();
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        fs::write(root.join(".git/config.lock"), "foreign").unwrap();
        assert!(session
            .prune_remote(&snap.revision, "origin", &empty_ad())
            .unwrap_err()
            .contains("EXTERNAL_LOCK"));
        assert_eq!(
            fs::read_to_string(root.join(".git/config.lock")).unwrap(),
            "foreign"
        );
        fs::remove_file(root.join(".git/config.lock")).unwrap();
        let repo = open(root).unwrap();
        let (mut plan, _) = prepare_mode(root, &repo, "origin", Some(&empty_ad())).unwrap();
        plan.changes[0].after = Some(b"[core]\n bare=true\n".to_vec());
        assert!(verify(&plan).is_err());
        plan.changes[0].after = plan.changes[0].before.clone();
        plan.changes[2].before = Some(b"ref: refs/heads/master\n".to_vec());
        assert!(verify(&plan).is_err());
        Config::open(&root.join(".git/config"))
            .unwrap()
            .set_str("external.value", "keep")
            .unwrap();
        assert!(session
            .prune_remote(&snap.revision, "origin", &empty_ad())
            .unwrap_err()
            .contains("STALE_STATE"));
        assert!(!pending(root));
    }
    #[test]
    fn prune_overlapping_sources_keep_destination_if_any_source_is_live() {
        let d = fixture("standard");
        let mut config = Config::open(&d.path().join(".git/config")).unwrap();
        config
            .set_multivar(
                "remote.origin.fetch",
                "^$",
                "refs/heads/alias:refs/remotes/origin/main",
            )
            .unwrap();
        drop(config);
        let repo = open(d.path()).unwrap();
        let mut ad = empty_ad();
        ad.refs.push(crate::network_worker::RemoteRef {
            name: "refs/heads/alias".into(),
            oid: repo.head().unwrap().target().unwrap().to_string(),
            symbolic_target: None,
        });
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session
            .prune_preview(&snap.revision, "origin", &ad)
            .unwrap()
            .removed_refs
            .is_empty());
        ad.refs.clear();
        assert_eq!(
            session
                .prune_preview(&snap.revision, "origin", &ad)
                .unwrap()
                .removed_refs,
            vec!["refs/remotes/origin/main"]
        );
    }
    #[test]
    fn prune_recovery_preserves_external_edits_and_new_stale_refs() {
        for config_edit in [false, true] {
            let d = fixture("standard");
            let root = d.path().canonicalize().unwrap();
            fs::write(root.join(".git/prune-test"), "1").unwrap();
            kill(&root, 0, false);
            if config_edit {
                let path = root.join(".git/config");
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(b"\n[external]\nvalue=keep\n");
                fs::write(path, bytes).unwrap();
            } else {
                let repo = Repository::open(&root).unwrap();
                repo.reference(
                    "refs/remotes/origin/new",
                    repo.head().unwrap().target().unwrap(),
                    false,
                    "external",
                )
                .unwrap();
            }
            let before = config(&root);
            let oldrefs = refs(&root);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap_err()
                .contains("RECOVERY_CONFLICT"));
            assert_eq!(config(&root), before);
            assert_eq!(refs(&root), oldrefs);
        }
    }
}
