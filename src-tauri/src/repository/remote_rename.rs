//! Remote rename: one deterministic intent, guarded resumable metadata changes.
use super::*;
use branch_rename::{read_optional, without_packed_ref, Change};
const JOURNAL: &str = ".git/branchdesk-remote-rename";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    name: String,
    target: String,
    symbolic: bool,
    loose: Option<Vec<u8>>,
    log: Option<Vec<u8>>,
    resolved: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    old: String,
    new: String,
    config: Vec<u8>,
    packed: Option<Vec<u8>>,
    sources: Vec<Source>,
    seconds: i64,
    offset: i32,
}
fn names(old: &str, new: &str) -> Result<()> {
    for name in [old, new] {
        if name.is_empty()
            || name.len() > 200
            || name.starts_with('-')
            || !git2::Reference::is_valid_name(&format!("refs/remotes/{name}/probe"))
        {
            return Err("INVALID_REMOTE_NAME".into());
        }
    }
    if old.eq_ignore_ascii_case(new)
        || old
            .to_lowercase()
            .starts_with(&format!("{}/", new.to_lowercase()))
        || new
            .to_lowercase()
            .starts_with(&format!("{}/", old.to_lowercase()))
    {
        return Err("REMOTE_NAME_COLLISION".into());
    }
    Ok(())
}
fn config_after(bytes: &[u8], old: &str, new: &str) -> Result<(Vec<u8>, bool)> {
    names(old, new)?;
    let values = remote_remove::entries(bytes)?;
    let from = format!("remote.{old}");
    let to = format!("remote.{new}");
    if values.iter().any(|(k, _)| {
        k.rsplit_once('.')
            .is_some_and(|(s, _)| s.eq_ignore_ascii_case(&to))
    }) {
        return Err("REMOTE_EXISTS".into());
    }
    if !values
        .iter()
        .any(|(k, _)| k.rsplit_once('.').is_some_and(|(s, _)| s == from))
    {
        return Err("REMOTE_NOT_FOUND".into());
    }
    let context = format!(":refs/remotes/{old}/");
    let replacement = format!(":refs/remotes/{new}/");
    let move_refs = values
        .iter()
        .any(|(k, v)| k == &format!("{from}.fetch") && v.contains(&context));
    let dir = tempfile::tempdir().map_err(err)?;
    let path = dir.path().join("config");
    fs::write(&path, bytes).map_err(err)?;
    let mut config = Config::open(&path).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    let mut keys = std::collections::BTreeSet::new();
    let mut output = Vec::new();
    for (key, value) in &values {
        let (section, suffix) = key.rsplit_once('.').ok_or("INVALID_REMOTE_CONFIG")?;
        if section == from {
            keys.insert(key.clone());
            output.push((
                format!("{to}.{suffix}"),
                if suffix == "fetch" {
                    value.replacen(&context, &replacement, 1)
                } else {
                    value.clone()
                },
            ));
        } else if (section.starts_with("branch.") && matches!(suffix, "remote" | "pushremote")
            || (move_refs && key == "remote.pushdefault"))
            && value == old
        {
            keys.insert(key.clone());
        }
    }
    // Preserve all values when rewriting a multivalued tracking/default key.
    for (key, value) in &values {
        if keys.contains(key) && !key.rsplit_once('.').is_some_and(|(s, _)| s == from) {
            output.push((
                key.clone(),
                if value == old {
                    new.into()
                } else {
                    value.clone()
                },
            ));
        }
    }
    for key in keys {
        config
            .remove_multivar(&key, ".*")
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    for (key, value) in output {
        config
            .set_multivar(&key, "a^", &value)
            .map_err(|_| "REMOTE_CONFIG_EDIT_FAILED")?;
    }
    drop(config);
    let after = bounded(&path, 65536)?;
    remote_remove::entries(&after)?;
    Ok((after, move_refs))
}
fn oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn renamed(plan: &Plan, name: &str) -> Result<String> {
    let prefix = format!("refs/remotes/{}/", plan.old);
    let suffix = name
        .strip_prefix(&prefix)
        .ok_or("INVALID_REMOTE_RENAME_REF")?;
    let new = format!("refs/remotes/{}/{suffix}", plan.new);
    if !git2::Reference::is_valid_name(&new) {
        return Err("INVALID_REMOTE_RENAME_REF".into());
    }
    Ok(new)
}
fn changes(plan: &Plan) -> Result<Vec<Change>> {
    if plan.version != 1 || plan.sources.len() > MAX_FILES || !(-1439..=1439).contains(&plan.offset)
    {
        return Err("INVALID_REMOTE_RENAME_JOURNAL".into());
    }
    let (config, move_refs) = config_after(&plan.config, &plan.old, &plan.new)?;
    if !move_refs && !plan.sources.is_empty() {
        return Err("INVALID_REMOTE_RENAME_JOURNAL".into());
    }
    let mut packed_sources = std::collections::HashMap::new();
    if let Some(bytes) = &plan.packed {
        let text = std::str::from_utf8(bytes).map_err(|_| "INVALID_PACKED_REFS")?;
        for line in text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with(['#', '^']))
        {
            let (target, name) = line.split_once(' ').ok_or("INVALID_PACKED_REFS")?;
            if !oid(target)
                || !git2::Reference::is_valid_name(name)
                || packed_sources.insert(name, target).is_some()
            {
                return Err("INVALID_PACKED_REFS".into());
            }
        }
    }
    if move_refs
        && packed_sources.keys().any(|name| {
            name.starts_with(&format!("refs/remotes/{}/", plan.old))
                && !plan.sources.iter().any(|source| &source.name == name)
        })
    {
        return Err("INVALID_REMOTE_RENAME_SOURCE".into());
    }
    let mut result = vec![Change {
        path: "config".into(),
        before: Some(plan.config.clone()),
        after: Some(config),
    }];
    let mut packed = plan.packed.clone();
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;
    for source in &plan.sources {
        if !seen.insert(source.name.clone()) {
            return Err("INVALID_REMOTE_RENAME_JOURNAL".into());
        }
        let new = renamed(plan, &source.name)?;
        let before_target = if source.symbolic {
            renamed(plan, &source.target)?;
            format!("ref: {}\n", source.target)
        } else {
            if !oid(&source.target) {
                return Err("INVALID_REMOTE_RENAME_OID".into());
            }
            format!("{}\n", source.target)
        };
        if source
            .loose
            .as_ref()
            .is_some_and(|b| b != before_target.as_bytes())
        {
            return Err("INVALID_REMOTE_RENAME_SOURCE".into());
        }
        if source.loose.is_none()
            && (source.symbolic
                || packed_sources.get(source.name.as_str()).copied()
                    != Some(source.target.as_str()))
        {
            return Err("INVALID_REMOTE_RENAME_SOURCE".into());
        }
        let target = if source.symbolic {
            format!("ref: {}\n", renamed(plan, &source.target)?)
        } else {
            before_target
        };
        if let Some(bytes) = &mut packed {
            *bytes = without_packed_ref(bytes, &source.name)?;
        }
        let mut log = source.log.clone();
        total += source.loose.as_ref().map_or(0, Vec::len) + log.as_ref().map_or(0, Vec::len);
        if total > 8 * 1024 * 1024 {
            return Err("LIMIT: remote rename metadata".into());
        }
        if let Some(resolved) = &source.resolved {
            if !oid(resolved) {
                return Err("INVALID_REMOTE_RENAME_OID".into());
            }
        }
        if let (Some(log), Some(resolved)) = (&mut log, &source.resolved) {
            let sign = if plan.offset < 0 { '-' } else { '+' };
            let offset = plan.offset.abs();
            log.extend_from_slice(format!("{resolved} {resolved} FLOE <local@floe.invalid> {} {sign}{:02}{:02}\tremote: renamed {} to {new}\n",plan.seconds,offset/60,offset%60,source.name).as_bytes());
        }
        if log.as_ref().is_some_and(|bytes| bytes.len() > 1024 * 1024) {
            return Err("LIMIT: remote reflog".into());
        }
        result.extend([
            Change {
                path: new.clone(),
                before: None,
                after: Some(target.into_bytes()),
            },
            Change {
                path: format!("logs/{new}"),
                before: None,
                after: log,
            },
            Change {
                path: source.name.clone(),
                before: source.loose.clone(),
                after: None,
            },
            Change {
                path: format!("logs/{}", source.name),
                before: source.log.clone(),
                after: None,
            },
        ]);
    }
    // Remove old packed entries before loose ref publication; recovery permits either image.
    result.insert(
        1,
        Change {
            path: "packed-refs".into(),
            before: plan.packed.clone(),
            after: packed,
        },
    );
    Ok(result)
}
fn prepare(
    root: &Path,
    repo: &Repository,
    old: &str,
    new: &str,
    time: Option<(i64, i32)>,
) -> Result<Plan> {
    let config = bounded(&root.join(".git/config"), 65536)?;
    let (_, move_refs) = config_after(&config, old, new)?;
    let t = git2::Signature::now("FLOE", "local@floe.invalid")
        .map_err(err)?
        .when();
    let (seconds, offset) = time.unwrap_or((t.seconds(), t.offset_minutes()));
    let mut plan = Plan {
        version: 1,
        old: old.into(),
        new: new.into(),
        config,
        packed: read_optional(root, "packed-refs")?,
        sources: vec![],
        seconds,
        offset,
    };
    if move_refs {
        let prefix = format!("refs/remotes/{old}/");
        let mut total = 0usize;
        for (index, r) in repo.references().map_err(err)?.enumerate() {
            if index >= MAX_FILES {
                return Err("LIMIT: refs".into());
            }
            let r = r.map_err(err)?;
            let name = r.name().map_err(err)?;
            if name.starts_with(&format!("refs/remotes/{new}/")) {
                return Err("REMOTE_REF_EXISTS".into());
            }
            if !name.starts_with(&prefix) {
                continue;
            }
            let new = renamed(&plan, name)?;
            if repo.find_reference(&new).is_ok()
                || read_optional(root, &new)?.is_some()
                || read_optional(root, &format!("logs/{new}"))?.is_some()
            {
                return Err("REMOTE_REF_EXISTS".into());
            }
            let symbolic = r.symbolic_target().map_err(err)?;
            let target = symbolic
                .map(str::to_owned)
                .or_else(|| r.target().map(|id| id.to_string()))
                .ok_or("INVALID_REMOTE_REF")?;
            if symbolic.is_some() {
                renamed(&plan, &target).map_err(|_| "UNSUPPORTED_REMOTE_SYMBOLIC_TARGET")?;
            }
            let log = read_optional(root, &format!("logs/{name}"))?;
            total += log.as_ref().map_or(0, Vec::len);
            if total > 8 * 1024 * 1024 {
                return Err("LIMIT: remote rename metadata".into());
            }
            plan.sources.push(Source {
                name: name.into(),
                target,
                symbolic: symbolic.is_some(),
                loose: read_optional(root, name)?,
                log,
                resolved: r
                    .resolve()
                    .ok()
                    .and_then(|r| r.target())
                    .map(|id| id.to_string()),
            });
        }
    }
    plan.sources.sort_by(|a, b| a.name.cmp(&b.name));
    changes(&plan)?;
    Ok(plan)
}
fn lock_paths(plan: &Plan) -> Result<Vec<String>> {
    let mut paths = vec!["config.lock".into(), "packed-refs.lock".into()];
    for c in changes(plan)? {
        if c.path.starts_with("refs/") || c.path.starts_with("logs/") {
            paths.push(format!("{}.lock", c.path));
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
fn record_locks(root: &Path, dir: &Path, plan: &Plan) -> Result<()> {
    let mut paths = lock_paths(plan)?;
    paths.push("index.lock".into());
    journal_locks::record_paths(root, dir, paths)
}
fn apply(root: &Path, plan: &Plan) -> Result<()> {
    let changes = changes(plan)?;
    let (_, move_refs) = config_after(&plan.config, &plan.old, &plan.new)?;
    let repo = open(root)?;
    let old = format!("refs/remotes/{}/", plan.old);
    let new = format!("refs/remotes/{}/", plan.new);
    for r in repo.references().map_err(err)? {
        let r = r.map_err(err)?;
        let name = r.name().map_err(err)?;
        if (name.starts_with(&old) || name.starts_with(&new))
            && move_refs
            && !changes.iter().any(|c| c.path == name)
        {
            return Err("RECOVERY_CONFLICT: new remote tracking ref".into());
        }
    }
    for c in &changes {
        let now = read_optional(root, &c.path)?;
        if now != c.before && now != c.after {
            return Err(format!("RECOVERY_CONFLICT: {} changed", c.path));
        }
    }
    checkpoint(root, 0);
    for (index, c) in changes.iter().enumerate() {
        let now = read_optional(root, &c.path)?;
        if now != c.after {
            if now != c.before {
                return Err("RECOVERY_CONFLICT: remote rename metadata changed".into());
            }
            let path = root.join(".git").join(&c.path);
            let parent = path.parent().ok_or("INVALID_METADATA_PATH")?;
            if let Some(bytes) = &c.after {
                fs::create_dir_all(parent).map_err(err)?;
                let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(err)?;
                std::io::Write::write_all(&mut temp, bytes).map_err(err)?;
                temp.as_file().sync_all().map_err(err)?;
                temp.persist(&path).map_err(err)?;
            } else if now.is_some() {
                fs::remove_file(&path).map_err(err)?;
            }
            checkout::sync_dir(parent)?;
        }
        checkpoint(root, index + 1);
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
    old: &str,
    new: &str,
) -> Result<()> {
    let initial = prepare(&session.root, repo, old, new, None)?;
    let _locks = remote_remove::acquire_paths(&session.root, lock_paths(&initial)?)?;
    session.fresh(revision)?;
    let plan = prepare(
        &session.root,
        repo,
        old,
        new,
        Some((initial.seconds, initial.offset)),
    )?;
    if changes(&initial)? != changes(&plan)? {
        return Err("STALE_STATE".into());
    }
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
        return Err("INVALID_REMOTE_RENAME_JOURNAL".into());
    }
    let plan: Plan = serde_json::from_slice(&bounded(&dir.join("record.json"), 64 * 1024 * 1024)?)
        .map_err(|_| "INVALID_REMOTE_RENAME_JOURNAL")?;
    let _locks = remote_remove::acquire_paths(root, lock_paths(&plan)?)?;
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
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_REMOTE_RENAME_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_REMOTE_RENAME_STEP")
            .ok()
            .as_deref()
            == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/remote-rename-test-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

#[cfg(test)]
mod tests {
    use super::super::remote_remove::tests::{config, fixture, git, refs};
    use super::*;
    fn action() -> Action {
        Action::RenameRemote {
            name: "origin".into(),
            new_name: "upstream".into(),
        }
    }
    #[test]
    fn rename_matches_git_cli_and_preserves_worktree() {
        for kind in [
            "standard",
            "packed",
            "shared",
            "custom",
            "negative-own",
            "dangling",
            "source-only",
        ] {
            let d = fixture(kind);
            let oracle = fixture(kind);
            let root = d.path();
            let index = fs::read(root.join(".git/index")).unwrap();
            let head = fs::read(root.join(".git/HEAD")).unwrap();
            git(oracle.path(), &["remote", "rename", "origin", "upstream"]);
            let mut session = Session::new(root.into()).unwrap();
            let snap = session.snapshot().unwrap();
            let result = session.mutate(&snap.revision, action()).unwrap();
            assert!(!result.recovery_pending);
            assert_eq!(refs(root), refs(oracle.path()), "{kind}");
            assert_eq!(config(root), config(oracle.path()), "{kind}");
            for name in ["refs/remotes/origin/HEAD", "refs/remotes/upstream/HEAD"] {
                assert_eq!(
                    read_optional(root, name).unwrap(),
                    read_optional(oracle.path(), name).unwrap(),
                    "{kind} {name}"
                );
            }
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), head);
            assert_eq!(
                fs::read_to_string(root.join("dirty.txt")).unwrap(),
                "keep dirty"
            );
        }
    }
    #[test]
    fn rename_refuses_collisions_stale_state_and_foreign_locks() {
        for destination in ["origin", "Origin", "origin/child", "keep", "../bad"] {
            let d = fixture("standard");
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = config(d.path());
            let before_refs = refs(d.path());
            assert!(session
                .mutate(
                    &snap.revision,
                    Action::RenameRemote {
                        name: "origin".into(),
                        new_name: destination.into()
                    }
                )
                .is_err());
            assert_eq!(config(d.path()), before);
            assert_eq!(refs(d.path()), before_refs);
            assert!(!pending(d.path()));
        }
        for lock in [
            "config.lock",
            "packed-refs.lock",
            "refs/remotes/upstream/main.lock",
            "logs/refs/remotes/origin/main.lock",
        ] {
            let d = fixture("standard");
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = config(d.path());
            let path = d.path().join(".git").join(lock);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "foreign").unwrap();
            assert!(session
                .mutate(&snap.revision, action())
                .unwrap_err()
                .contains("EXTERNAL_LOCK"));
            assert_eq!(fs::read_to_string(path).unwrap(), "foreign");
            assert_eq!(config(d.path()), before);
            assert!(!pending(d.path()));
        }
        let d = fixture("standard");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        fs::write(d.path().join("dirty.txt"), "external edit").unwrap();
        let before = config(d.path());
        assert!(session
            .mutate(&snap.revision, action())
            .unwrap_err()
            .contains("STALE"));
        assert_eq!(config(d.path()), before);
    }
    #[test]
    fn rename_recovery_refuses_external_metadata_changes() {
        for edit in ["config", "old-ref", "new-ref"] {
            let d = fixture("standard");
            let root = d.path().canonicalize().unwrap();
            kill(&root, 0, false);
            if edit == "config" {
                let path = root.join(".git/config");
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(b"\n[external]\nvalue=keep\n");
                fs::write(path, bytes).unwrap();
            } else {
                let repo = Repository::open(&root).unwrap();
                repo.reference(
                    &format!(
                        "refs/remotes/{}/external",
                        if edit == "old-ref" {
                            "origin"
                        } else {
                            "upstream"
                        }
                    ),
                    repo.head().unwrap().target().unwrap(),
                    false,
                    "external",
                )
                .unwrap();
            }
            let before = config(&root);
            let before_refs = refs(&root);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap_err()
                .contains("RECOVERY_CONFLICT"));
            assert_eq!(config(&root), before);
            assert_eq!(refs(&root), before_refs);
            assert!(pending(&root));
        }
    }
    #[test]
    fn rename_rejects_forged_ref_paths_and_missing_packed_sources() {
        let d = fixture("packed");
        let repo = open(d.path()).unwrap();
        let mut plan = prepare(d.path(), &repo, "origin", "upstream", None).unwrap();
        plan.sources.retain(|s| s.name.ends_with("HEAD"));
        assert!(changes(&plan).is_err());
        let mut plan = prepare(d.path(), &repo, "origin", "upstream", None).unwrap();
        plan.sources[0].name = "refs/heads/main".into();
        assert!(changes(&plan).is_err());
        let mut plan = prepare(d.path(), &repo, "origin", "upstream", None).unwrap();
        plan.packed = None;
        assert!(changes(&plan).is_err());
    }
    #[test]
    fn rename_reflog_matches_git_event_and_retains_prior_bytes() {
        for kind in ["standard", "packed", "dangling"] {
            for existing in [true, false] {
                let d = fixture(kind);
                let oracle = fixture(kind);
                if !existing {
                    for root in [d.path(), oracle.path()] {
                        fs::remove_dir_all(root.join(".git/logs/refs/remotes/origin")).unwrap();
                    }
                }
                let prior = read_optional(d.path(), "logs/refs/remotes/origin/main").unwrap();
                git(oracle.path(), &["remote", "rename", "origin", "upstream"]);
                let mut session = Session::new(d.path().into()).unwrap();
                let snap = session.snapshot().unwrap();
                session.mutate(&snap.revision, action()).unwrap();
                for branch in ["HEAD", "main"] {
                    let path = format!("logs/refs/remotes/upstream/{branch}");
                    let ours = read_optional(d.path(), &path).unwrap();
                    let theirs = read_optional(oracle.path(), &path).unwrap();
                    assert_eq!(
                        ours.is_some(),
                        theirs.is_some(),
                        "{kind} existing={existing} {branch}"
                    );
                    if let (Some(ours), Some(theirs)) = (ours, theirs) {
                        let a = String::from_utf8(ours).unwrap();
                        let b = String::from_utf8(theirs).unwrap();
                        assert_eq!(
                            a.lines().count(),
                            b.lines().count(),
                            "{kind} existing={existing} {branch}"
                        );
                        assert_eq!(
                            a.lines().last().unwrap().split_once('\t').unwrap().1,
                            b.lines().last().unwrap().split_once('\t').unwrap().1,
                            "{kind} existing={existing} {branch}"
                        );
                        if branch == "main" {
                            if let Some(prior) = &prior {
                                assert!(a.as_bytes().starts_with(prior));
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn rename_refuses_occupied_destination_before_publishing_journal() {
        let d = fixture("standard");
        let repo = open(d.path()).unwrap();
        repo.reference(
            "refs/remotes/upstream/other",
            repo.head().unwrap().target().unwrap(),
            false,
            "external",
        )
        .unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = config(d.path());
        assert!(session
            .mutate(&snap.revision, action())
            .unwrap_err()
            .contains("REMOTE_REF_EXISTS"));
        assert!(!pending(d.path()));
        assert_eq!(config(d.path()), before);
    }
    #[test]
    fn rename_worker() {
        let Ok(root) = std::env::var("BRANCHDESK_REMOTE_RENAME_ROOT") else {
            return;
        };
        let mut s = Session::new(root.into()).unwrap();
        let a = s.snapshot().unwrap();
        s.mutate(
            &a.revision,
            if std::env::var_os("BRANCHDESK_REMOTE_RENAME_RECOVER").is_some() {
                Action::RecoverCheckout
            } else {
                action()
            },
        )
        .unwrap();
    }
    fn kill(root: &Path, step: usize, recover: bool) {
        let ready = root.join(".git/remote-rename-test-ready");
        let _ = fs::remove_file(&ready);
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "repository::remote_rename::tests::rename_worker",
            "--nocapture",
        ])
        .env("BRANCHDESK_REMOTE_RENAME_ROOT", root)
        .env("BRANCHDESK_REMOTE_RENAME_STEP", step.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if recover {
            cmd.env("BRANCHDESK_REMOTE_RENAME_RECOVER", "1");
        }
        let mut child = cmd.spawn().unwrap();
        let started = std::time::Instant::now();
        while !ready.exists() {
            if let Some(s) = child.try_wait().unwrap() {
                panic!("worker exited: {s}, step {step}, recover {recover}");
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
    fn rename_recovers_each_boundary_after_two_actual_kills() {
        for kind in ["standard", "packed", "custom", "dangling"] {
            let probe = fixture(kind);
            let repo = open(probe.path()).unwrap();
            let count = changes(&prepare(probe.path(), &repo, "origin", "upstream", None).unwrap())
                .unwrap()
                .len();
            for step in 0..=count {
                let d = fixture(kind);
                let root = d.path().canonicalize().unwrap();
                let oracle = fixture(kind);
                git(oracle.path(), &["remote", "rename", "origin", "upstream"]);
                let index = fs::read(root.join(".git/index")).unwrap();
                kill(&root, step, false);
                kill(&root, step, true);
                let mut s = Session::new(root.clone()).unwrap();
                let a = s.snapshot().unwrap();
                assert!(a.recovery_pending);
                let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert!(!b.recovery_pending);
                assert_eq!(refs(&root), refs(oracle.path()), "{kind} {step}");
                assert_eq!(config(&root), config(oracle.path()), "{kind} {step}");
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            }
        }
    }
}
