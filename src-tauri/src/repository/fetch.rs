//! Journaled publication of configured remote-tracking references and FETCH_HEAD.
use super::*;
use crate::network_worker::{Advertisement, Downloaded};
use branch_rename::{read_optional, Change};
/// Apply only the exact stale references approved before Fetch; never expand scope.
pub fn prune_confirmed(session:&mut Session,snapshot:Snapshot,remote:&str,download:&Downloaded,approved:&[String])->Result<Snapshot>{
    if approved.is_empty(){return Ok(snapshot);}
    let fresh=session.prune_preview(&snapshot.revision,remote,download.advertisement())?;
    if fresh.removed_refs!=approved{return Err("PRUNE_TARGETS_CHANGED".into());}
    session.prune_remote(&snapshot.revision,remote,download.advertisement())
}
const JOURNAL: &str = ".git/branchdesk-fetch";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    #[serde(default)]
    non_commit_sources: Vec<String>,
    #[serde(default)]
    implicit_tags: Vec<String>,
    #[serde(default)]
    pack_pin: Option<crate::network_worker::PackPin>,
    remote: String,
    endpoint: String,
    seconds: i64,
    mappings: Vec<fetch_plan::Mapping>,
    changes: Vec<Change>,
}
fn name(remote: &str) -> Result<()> {
    if remote.is_empty()
        || remote.len() > 200
        || remote.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/remotes/{remote}/probe"))
    {
        return Err("INVALID_REMOTE_NAME".into());
    }
    Ok(())
}
pub fn preview(
    session: &mut Session,
    revision: &str,
    remote: &str,
    advertisement: &Advertisement,
) -> Result<fetch_plan::Plan> {
    name(remote)?;
    let snapshot = session.fresh(revision)?;
    if let Some(reason) = snapshot.write_block {
        return Err(format!("READ_ONLY: {reason}"));
    }
    if (sequence_state::exists(&session.root) || rebase::draft::exists(&session.root))
        || replay_draft::exists(&session.root)
        || merge_draft::exists(&session.root)
        || open(&session.root)?.state() != git2::RepositoryState::Clean
    {
        return Err("FETCH_PENDING_OPERATION".into());
    }
    let plan = fetch_plan::preview(session, revision, remote, advertisement)?;
    check_destinations(&plan, remote)?;
    Ok(plan)
}
fn check_destinations(plan: &fetch_plan::Plan, remote: &str) -> Result<()> {
    for mapping in &plan.mappings {
        if mapping.destination.as_ref().is_some_and(|target| {
            !target.starts_with(&format!("refs/remotes/{remote}/"))
                && !target.starts_with("refs/tags/")
        }) {
            return Err(
                "FETCH_TRACKING_REFS_ONLY: local-branch destinations are not enabled yet".into(),
            );
        }
    }
    for mapping in &plan.mappings {
        if mapping
            .destination
            .as_ref()
            .is_some_and(|d| d.starts_with("refs/tags/"))
            && mapping
                .before
                .as_ref()
                .is_some_and(|old| old != &mapping.after)
            && !mapping.force
        {
            return Err(
                "FETCH_TAG_EXISTS: refusing to replace an existing tag without a forced refspec"
                    .into(),
            );
        }
    }
    Ok(())
}
pub fn preview_download(
    session: &mut Session,
    revision: &str,
    remote: &str,
    download: &Downloaded,
) -> Result<fetch_plan::Plan> {
    let base = preview(session, revision, remote, download.advertisement())?;
    let plan = fetch_tags::augment(&session.root, remote, download, base)?;
    check_destinations(&plan, remote)?;
    Ok(plan)
}
fn fetch_head(record: &Record) -> Result<Vec<u8>> {
    let mut merge_sources = std::collections::HashSet::new();
    // Version 1 journals retain their exact historical all-not-for-merge bytes.
    if record.version >= 2 {
        let head = record
            .changes
            .get(1)
            .and_then(|c| c.before.as_deref())
            .ok_or("INVALID_FETCH_JOURNAL")?;
        let values = remote_remove::entries(
            record.changes[0]
                .before
                .as_deref()
                .ok_or("INVALID_FETCH_JOURNAL")?,
        )?;
        merge_sources.extend(fetch_plan::merge_candidates(&values, head, &record.remote)?);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut lines = Vec::new();
    for mapping in &record.mappings {
        if seen.insert(&mapping.source) {
            let merge = merge_sources.contains(&mapping.source)
                && !record.non_commit_sources.contains(&mapping.source);
            lines.push((
                merge,
                format!(
                    "{}\t{}\t{} of {}\n",
                    mapping.after,
                    if merge { "" } else { "not-for-merge" },
                    mapping.source,
                    record.endpoint
                ),
            ));
        }
    }
    // Git lists merge candidates before the remaining fetched references.
    lines.sort_by_key(|(merge, _)| !merge);
    Ok(lines
        .into_iter()
        .map(|(_, line)| line)
        .collect::<String>()
        .into_bytes())
}
fn log_line(record: &Record, mapping: &fetch_plan::Mapping) -> Vec<u8> {
    format!(
        "{} {} FLOE <branchdesk@localhost> {} +0000\tfetch {}\n",
        mapping
            .before
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000"),
        mapping.after,
        record.seconds,
        record.remote
    )
    .into_bytes()
}
fn logging_for(config: &[u8], destination: &str) -> Result<bool> {
    let enabled = branch_rename::logging_enabled(config)?;
    if !destination.starts_with("refs/tags/") {
        return Ok(enabled);
    }
    Ok(remote_remove::entries(config)?
        .iter()
        .rev()
        .find(|(k, _)| k == "core.logallrefupdates")
        .is_some_and(|(_, value)| value == "always"))
}
fn non_commit_sources(repo: &Repository, mappings: &[fetch_plan::Mapping]) -> Result<Vec<String>> {
    let mut result = std::collections::BTreeSet::new();
    for mapping in mappings {
        let mut object = repo
            .find_object(Oid::from_str(&mapping.after).map_err(err)?, None)
            .map_err(|_| "FETCH_OBJECT_MISSING")?;
        let mut depth = 0;
        while let Some(tag) = object.as_tag() {
            depth += 1;
            if depth > 128 {
                return Err("FETCH_TAG_DEPTH_LIMIT".into());
            }
            object = tag.target().map_err(|_| "FETCH_OBJECT_INVALID")?;
        }
        if object.kind() != Some(git2::ObjectType::Commit) {
            result.insert(mapping.source.clone());
        }
    }
    Ok(result.into_iter().collect())
}
fn prepare(root: &Path, remote: &str, endpoint: &str, plan: fetch_plan::Plan) -> Result<Record> {
    let mut record = Record {
        version: 5,
        implicit_tags: Vec::new(),
        non_commit_sources: Vec::new(),
        pack_pin: None,
        remote: remote.into(),
        endpoint: endpoint.into(),
        seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(err)?
            .as_secs()
            .try_into()
            .map_err(err)?,
        mappings: plan.mappings,
        changes: Vec::new(),
    };
    for path in ["config", "HEAD", "packed-refs"] {
        let bytes = read_optional(root, path)?;
        record.changes.push(Change {
            path: path.into(),
            before: bytes.clone(),
            after: bytes,
        });
    }
    let config_values = remote_remove::entries(
        record.changes[0]
            .before
            .as_deref()
            .ok_or("INVALID_FETCH_JOURNAL")?,
    )?;
    for mapping in &record.mappings {
        if !fetch_plan::permits(
            &config_values,
            remote,
            &mapping.source,
            mapping.destination.as_deref(),
            record.changes[1].before.as_deref(),
        )? {
            record.implicit_tags.push(mapping.source.clone());
        }
    }
    record.implicit_tags.sort();
    record.implicit_tags.dedup();
    for mapping in &record.mappings {
        if mapping.before.as_deref() == Some(&mapping.after) {
            continue;
        }
        if let Some(destination) = &mapping.destination {
            record.changes.push(Change {
                path: destination.clone(),
                before: read_optional(root, destination)?,
                after: Some(format!("{}\n", mapping.after).into_bytes()),
            });
            let path = format!("logs/{destination}");
            let before = read_optional(root, &path)?;
            let mut after = before.clone().unwrap_or_default();
            after.extend(log_line(&record, mapping));
            let logging = logging_for(
                record.changes[0]
                    .before
                    .as_deref()
                    .ok_or("INVALID_FETCH_JOURNAL")?,
                destination,
            )?;
            let after = (logging || before.is_some()).then_some(after);
            record.changes.push(Change {
                path,
                before,
                after,
            });
        }
    }
    record.changes.push(Change {
        path: "FETCH_HEAD".into(),
        before: read_optional(root, "FETCH_HEAD")?,
        after: Some(fetch_head(&record)?),
    });
    verify(&record)?;
    Ok(record)
}
fn oid(value: &str) -> Result<()> {
    if value.len() != 40
        || !value.bytes().all(|b| b.is_ascii_hexdigit())
        || Oid::from_str(value).map_err(err)? == Oid::ZERO_SHA1
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    Ok(())
}
fn verify(record: &Record) -> Result<()> {
    name(&record.remote)?;
    if !matches!(record.version, 1..=5)
        || record.seconds < 0
        || record.mappings.len() > 2000
        || record.changes.len() < 4
        || serde_json::to_vec(record).map_err(err)?.len() > 32 * 1024 * 1024
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    if (record.version < 4 && !record.non_commit_sources.is_empty())
        || record.non_commit_sources.windows(2).any(|w| w[0] >= w[1])
        || record
            .non_commit_sources
            .iter()
            .any(|source| !record.mappings.iter().any(|m| &m.source == source))
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    if let Some(pin) = &record.pack_pin {
        if record.version < 3 {
            return Err("INVALID_FETCH_JOURNAL".into());
        }
        pin.validate()?;
    }
    if remote_endpoint::https(&record.endpoint)?.as_str() != record.endpoint {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    for (i, path) in ["config", "HEAD", "packed-refs"].iter().enumerate() {
        let change = &record.changes[i];
        if change.path != *path || change.before != change.after {
            return Err("INVALID_FETCH_JOURNAL".into());
        }
    }
    let config = record.changes[0]
        .before
        .as_deref()
        .ok_or("INVALID_FETCH_JOURNAL")?;
    let values = remote_remove::entries(config)?;
    if (record.version < 5 && !record.implicit_tags.is_empty())
        || record.implicit_tags.windows(2).any(|w| w[0] >= w[1])
        || (!record.implicit_tags.is_empty()
            && !fetch_tags::implicit_allowed(&values, &record.remote)?)
        || record.implicit_tags.iter().any(|tag| {
            !record.mappings.iter().any(|m| {
                &m.source == tag
                    && m.destination.as_ref() == Some(tag)
                    && tag.starts_with("refs/tags/")
                    && !m.force
                    && (fetch_tags::policy(&values, &record.remote).ok()
                        == Some(fetch_tags::Policy::All)
                        || m.before.is_none())
            })
        })
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    for tag in &record.implicit_tags {
        if fetch_tags::policy(&values, &record.remote)? == fetch_tags::Policy::All
            && fetch_plan::excludes(&values, &record.remote, tag)?
        {
            return Err("INVALID_FETCH_JOURNAL".into());
        }
    }
    let urls = values
        .iter()
        .filter(|(key, _)| key == &format!("remote.{}.url", record.remote))
        .map(|(_, v)| v)
        .collect::<Vec<_>>();
    if urls.len() != 1 || remote_endpoint::https(urls[0])?.as_str() != record.endpoint {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    if record.changes[1].before.is_none()
        || record.changes.iter().any(|change| {
            change
                .before
                .as_ref()
                .is_some_and(|v| v.len() > 1024 * 1024)
                || change.after.as_ref().is_some_and(|v| v.len() > 1024 * 1024)
        })
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }

    let mut folded_destinations = std::collections::BTreeSet::new();
    let mut index = 3;
    let mut destinations = std::collections::HashSet::new();
    for mapping in &record.mappings {
        oid(&mapping.after)?;
        if let Some(before) = &mapping.before {
            oid(before)?;
        }
        if (mapping.source != "HEAD"
            && (!mapping.source.starts_with("refs/")
                || !git2::Reference::is_valid_name(&mapping.source)))
            || (!(record.implicit_tags.contains(&mapping.source)
                && mapping.destination.as_ref() == Some(&mapping.source)
                && !mapping.force)
                && !fetch_plan::permits(
                    &values,
                    &record.remote,
                    &mapping.source,
                    mapping.destination.as_deref(),
                    if record.version >= 2 {
                        record.changes[1].before.as_deref()
                    } else {
                        None
                    },
                )?)
        {
            return Err("INVALID_FETCH_JOURNAL".into());
        }
        if let Some(destination) = &mapping.destination {
            if !(destination.starts_with(&format!("refs/remotes/{}/", record.remote))
                || (record.version >= 4 && destination.starts_with("refs/tags/")))
                || !git2::Reference::is_valid_name(destination)
                || !destinations.insert(destination)
                || !folded_destinations.insert(destination.to_lowercase())
            {
                return Err("INVALID_FETCH_JOURNAL".into());
            }
            if record.version >= 4
                && (mapping.force
                    != fetch_plan::permits_force(
                        &values,
                        &record.remote,
                        &mapping.source,
                        Some(destination),
                    )?
                    || (destination.starts_with("refs/tags/")
                        && mapping
                            .before
                            .as_ref()
                            .is_some_and(|old| old != &mapping.after)
                        && !mapping.force))
            {
                return Err("INVALID_FETCH_JOURNAL".into());
            }
            if mapping.before.as_deref() == Some(&mapping.after) {
                continue;
            }
            let change = record.changes.get(index).ok_or("INVALID_FETCH_JOURNAL")?;
            if change.path != *destination
                || change.after != Some(format!("{}\n", mapping.after).into_bytes())
            {
                return Err("INVALID_FETCH_JOURNAL".into());
            }
            if let Some(before) = &change.before {
                if Some(before.as_slice())
                    != mapping
                        .before
                        .as_ref()
                        .map(|oid| format!("{oid}\n").into_bytes())
                        .as_deref()
                {
                    return Err("INVALID_FETCH_JOURNAL".into());
                }
            }
            if change.before.is_none() {
                let packed =
                    std::str::from_utf8(record.changes[2].before.as_deref().unwrap_or_default())
                        .map_err(|_| "INVALID_FETCH_JOURNAL")?;
                let old = packed
                    .lines()
                    .filter_map(|line| line.split_once(' '))
                    .filter(|(_, name)| *name == destination)
                    .map(|(oid, _)| oid)
                    .collect::<Vec<_>>();
                if old.len() > 1 || old.first().copied() != mapping.before.as_deref() {
                    return Err("INVALID_FETCH_JOURNAL".into());
                }
            }
            let log = record
                .changes
                .get(index + 1)
                .ok_or("INVALID_FETCH_JOURNAL")?;
            let mut expected = log.before.clone().unwrap_or_default();
            expected.extend(log_line(record, mapping));
            let logging = if record.version >= 4 {
                logging_for(config, destination)?
            } else {
                branch_rename::logging_enabled(config)?
            };
            if log.path != format!("logs/{destination}")
                || log.after != (logging || log.before.is_some()).then_some(expected)
            {
                return Err("INVALID_FETCH_JOURNAL".into());
            }
            index += 2;
        }
    }
    for (parent, child) in folded_destinations
        .iter()
        .zip(folded_destinations.iter().skip(1))
    {
        if child.starts_with(&format!("{parent}/")) {
            return Err("INVALID_FETCH_JOURNAL".into());
        }
    }
    if record.changes.len() != index + 1
        || record.changes[index].path != "FETCH_HEAD"
        || record.changes[index].after != Some(fetch_head(record)?)
    {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    Ok(())
}
fn paths(record: &Record) -> Vec<String> {
    record
        .changes
        .iter()
        .map(|change| format!("{}.lock", change.path))
        .collect()
}
fn record_locks(root: &Path, record: &Record) -> Result<()> {
    let mut paths = paths(record);
    paths.push("index.lock".into());
    journal_locks::record_paths(root, &root.join(JOURNAL), paths)
}
fn apply(root: &Path, record: &Record) -> Result<()> {
    verify(record)?;
    if let Some(pin) = &record.pack_pin {
        pin.check(&root.join(".git"))?;
    }
    let repo = open(root)?;
    if record.version >= 4
        && non_commit_sources(&repo, &record.mappings)? != record.non_commit_sources
    {
        return Err("INVALID_FETCH_OBJECT_POLICY".into());
    }
    for mapping in &record.mappings {
        repo.find_object(Oid::from_str(&mapping.after).map_err(err)?, None)
            .map_err(|_| "FETCH_OBJECT_MISSING")?;
    }
    for change in &record.changes {
        let current = read_optional(root, &change.path)?;
        if current != change.before && current != change.after {
            return Err("RECOVERY_CONFLICT: fetch metadata changed".into());
        }
    }
    checkpoint(root, 0);
    for (i, change) in record.changes.iter().enumerate() {
        let current = read_optional(root, &change.path)?;
        if current != change.after {
            if current != change.before {
                return Err("RECOVERY_CONFLICT: fetch metadata changed".into());
            }
            let path = root.join(".git").join(&change.path);
            let mut next =
                tempfile::NamedTempFile::new_in(path.parent().ok_or("INVALID_FETCH_JOURNAL")?)
                    .map_err(err)?;
            std::io::Write::write_all(
                &mut next,
                change.after.as_deref().ok_or("INVALID_FETCH_JOURNAL")?,
            )
            .map_err(err)?;
            next.as_file().sync_all().map_err(err)?;
            next.persist(&path).map_err(err)?;
            checkout::sync_dir(path.parent().unwrap())?;
        }
        checkpoint(root, i + 1);
    }
    Ok(())
}
pub fn execute(
    session: &mut Session,
    revision: &str,
    remote: &str,
    download: &Downloaded,
) -> Result<Snapshot> {
    let plan = preview_download(session, revision, remote, download)?;
    let _lease = lease::OperationLease::acquire(&session.root)?;
    let index = IndexLock::acquire(&session.root)?;
    let mut initial = prepare(
        &session.root,
        remote,
        &download.advertisement().endpoint,
        plan,
    )?;
    let locks = remote_remove::acquire_paths(&session.root, paths(&initial))?;
    let plan = preview_download(session, revision, remote, download)?;
    if plan.mappings != initial.mappings {
        return Err("STALE_STATE".into());
    }
    for change in &initial.changes {
        if read_optional(&session.root, &change.path)? != change.before {
            return Err("STALE_STATE".into());
        }
    }
    let selected = download.project(
        &initial
            .mappings
            .iter()
            .map(|m| m.after.clone())
            .collect::<Vec<_>>(),
    )?;
    initial.pack_pin = match selected.pin(&session.root.join(".git")) {
        Err(error) if error == "FETCH_PACK_ALREADY_PINNED" => {
            if let Some(hash) = selected.pack_hash() {
                cleanup_completed_pin(&session.root, hash)?;
            }
            selected.pin(&session.root.join(".git"))?
        }
        result => result?,
    };
    if let Err(error) = selected.import_into(&session.root.join(".git")) {
        if let Some(pin) = &initial.pack_pin {
            let _ = pin.release(&session.root.join(".git"));
        }
        return Err(error);
    }
    let classify = (|| {
        initial.non_commit_sources = non_commit_sources(&open(&session.root)?, &initial.mappings)?;
        let fetch_head = fetch_head(&initial)?;
        initial
            .changes
            .last_mut()
            .ok_or("INVALID_FETCH_JOURNAL")?
            .after = Some(fetch_head);
        verify(&initial)
    })();
    if let Err(error) = classify {
        if let Some(pin) = &initial.pack_pin {
            let _ = pin.release(&session.root.join(".git"));
        }
        return Err(error);
    }
    let dir = session.root.join(JOURNAL);
    if let Err(error) = fs::create_dir(&dir) {
        if let Some(pin) = &initial.pack_pin {
            let _ = pin.release(&session.root.join(".git"));
        }
        return Err(err(error));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(err)?;
    }
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&initial).map_err(err)?,
    )?;
    record_locks(&session.root, &initial)?;
    checkout::sync_dir(&session.root.join(".git"))?;
    apply(&session.root, &initial)?;
    drop(locks);
    drop(index);
    finish(&session.root)?;
    session.snapshot()
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
pub(super) fn recover(root: &Path) -> Result<()> {
    let dir = root.join(JOURNAL);
    let meta = fs::symlink_metadata(&dir).map_err(err)?;
    if !meta.is_dir() || is_link(&meta) {
        return Err("INVALID_FETCH_JOURNAL".into());
    }
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 32 * 1024 * 1024)?)
            .map_err(|_| "INVALID_FETCH_JOURNAL")?;
    verify(&record)?;
    let _locks = remote_remove::acquire_paths(root, paths(&record))?;
    record_locks(root, &record)?;
    apply(root, &record)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    let record: Record = serde_json::from_slice(&bounded(
        &root.join(JOURNAL).join("record.json"),
        32 * 1024 * 1024,
    )?)
    .map_err(|_| "INVALID_FETCH_JOURNAL")?;
    verify(&record)?;
    fs::rename(
        root.join(JOURNAL),
        root.join(format!("{JOURNAL}-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, 100);
    if let Some(pin) = &record.pack_pin {
        pin.release(&root.join(".git"))?;
    }
    Ok(())
}
// Caller holds the repository operation lease and metadata locks. Only validated
// completed archives authorize cleanup; unrecorded and pending pins stay intact.
fn cleanup_completed_pin(root: &Path, hash: &str) -> Result<()> {
    match fs::symlink_metadata(root.join(JOURNAL)) {
        Ok(_) => return Err("FETCH_PENDING_OPERATION".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(err(error)),
    }
    let mut pins = Vec::new();
    let mut bytes = 0u64;
    for (count, entry) in fs::read_dir(root.join(".git")).map_err(err)?.enumerate() {
        if count >= 4096 {
            return Err("FETCH_ARCHIVE_SCAN_LIMIT".into());
        }
        let entry = entry.map_err(err)?;
        let name = entry.file_name();
        let Some(suffix) = name
            .to_str()
            .and_then(|s| s.strip_prefix("branchdesk-fetch-completed-"))
        else {
            continue;
        };
        if !Uuid::parse_str(suffix).is_ok_and(|id| id.to_string() == suffix) {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(err)?;
        if !meta.is_dir() || is_link(&meta) {
            continue;
        }
        let path = entry.path().join("record.json");
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() || is_link(&meta) || meta.len() > 32 * 1024 * 1024 {
            continue;
        }
        bytes = bytes.saturating_add(meta.len());
        if bytes > 64 * 1024 * 1024 {
            return Err("FETCH_ARCHIVE_SCAN_LIMIT".into());
        }
        let Ok(data) = bounded(&path, meta.len()) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<Record>(&data) else {
            continue;
        };
        if verify(&record).is_err() {
            continue;
        }
        // A directory name alone is not evidence that publication completed.
        // Changed metadata also conservatively leaves ownership for explicit review.
        if !record.changes.iter().all(|change| {
            read_optional(root, &change.path).is_ok_and(|current| current == change.after)
        }) {
            continue;
        }
        if let Some(pin) = record.pack_pin.filter(|pin| pin.matches_hash(hash)) {
            pins.push(pin);
        }
    }
    // Finish bounded discovery before changing any file.
    for pin in pins {
        pin.release(&root.join(".git"))?;
    }
    Ok(())
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_FETCH_ROOT").as_deref() == Some(root.as_os_str())
        && std::env::var("BRANCHDESK_FETCH_STEP").ok().as_deref() == Some(step.to_string().as_str())
    {
        fs::write(root.join(".git/fetch-ready"), step.to_string()).unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_worker::RemoteRef;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const URL: &str = "https://example.com/repo.git";
    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let source = Repository::init_bare(root.path().join("source")).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let tree = source
            .find_tree(source.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let first = source
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "remote main",
                &tree,
                &[],
            )
            .unwrap();
        source.set_head("refs/heads/main").unwrap();
        source
            .commit(
                Some("refs/heads/topic"),
                &sig,
                &sig,
                "remote topic",
                &tree,
                &[&source.find_commit(first).unwrap()],
            )
            .unwrap();
        let local = Repository::init(root.path().join("target")).unwrap();
        let tree = local
            .find_tree(local.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        local
            .commit(Some("HEAD"), &sig, &sig, "local", &tree, &[])
            .unwrap();
        local.index().unwrap().write().unwrap();
        local.remote("origin", URL).unwrap();
        fs::write(root.path().join("target/dirty.txt"), "keep worktree").unwrap();
        root
    }
    fn download(root: &Path) -> Downloaded {
        let source = Repository::open_bare(root.join("source")).unwrap();
        let mut refs: Vec<RemoteRef> = source
            .references()
            .unwrap()
            .map(|r| {
                let r = r.unwrap();
                RemoteRef {
                    name: r.name().unwrap().into(),
                    oid: r.target().unwrap().to_string(),
                    symbolic_target: None,
                }
            })
            .collect();
        refs.push(RemoteRef {
            name: "HEAD".into(),
            oid: source.head().unwrap().target().unwrap().to_string(),
            symbolic_target: Some("refs/heads/main".into()),
        });
        Downloaded::fixture(
            &source,
            Advertisement {
                endpoint: URL.into(),
                refs,
            },
        )
    }
    fn preserved(root: &Path) -> Vec<Vec<u8>> {
        [".git/HEAD", ".git/index", ".git/config", "dirty.txt"]
            .iter()
            .map(|name| fs::read(root.join(name)).unwrap())
            .collect()
    }
    fn check(root: &Path) {
        let repo = Repository::open(root).unwrap();
        for branch in ["main", "topic"] {
            assert!(repo
                .find_reference(&format!("refs/remotes/origin/{branch}"))
                .unwrap()
                .peel_to_commit()
                .is_ok());
        }
        assert!(fs::read_to_string(root.join(".git/FETCH_HEAD"))
            .unwrap()
            .contains("not-for-merge"));
    }
    #[test]
    fn fetch_imports_and_updates_tracking_refs_without_worktree_or_head_change() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let before = preserved(&root);
        let data = download(fixture.path());
        let mut session = Session::new(root.clone()).unwrap();
        let initial = session.snapshot().unwrap();
        let result = execute(&mut session, &initial.revision, "origin", &data).unwrap();
        assert!(!result.recovery_pending);
        assert_eq!(preserved(&root), before);
        check(&root);
        let log = fs::read(root.join(".git/logs/refs/remotes/origin/main")).unwrap();
        let result = execute(&mut session, &result.revision, "origin", &data).unwrap();
        assert!(!result.recovery_pending);
        assert_eq!(
            fs::read(root.join(".git/logs/refs/remotes/origin/main")).unwrap(),
            log
        );
        assert_eq!(preserved(&root), before);
        let checked = Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(&root)
            .args(["fsck", "--strict", "--no-reflogs"])
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }
    #[test]
    fn fetch_packed_tracking_update_preserves_packed_bytes_and_appends_reflog() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        let old = repo.head().unwrap().target().unwrap();
        repo.reference("refs/remotes/origin/main", old, false, "old tracking")
            .unwrap();
        assert!(Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(&root)
            .args(["pack-refs", "--all", "--prune"])
            .status()
            .unwrap()
            .success());
        let packed = fs::read(root.join(".git/packed-refs")).unwrap();
        let before = preserved(&root);
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        check(&root);
        assert_eq!(fs::read(root.join(".git/packed-refs")).unwrap(), packed);
        assert_eq!(preserved(&root), before);
        assert!(
            fs::read_to_string(root.join(".git/logs/refs/remotes/origin/main"))
                .unwrap()
                .contains(&format!("{old} "))
        );
    }
    #[test]
    fn actual_fetch_tracking_refs_and_fetch_head_oids_match_git() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        let oracle = tempfile::tempdir().unwrap();
        let repo = Repository::init(oracle.path()).unwrap();
        repo.remote("origin", fixture.path().join("source").to_str().unwrap())
            .unwrap();
        let output = Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(oracle.path())
            .args([
                "-c",
                "protocol.file.allow=always",
                "-c",
                "remote.origin.followRemoteHEAD=never",
                "fetch",
                "--no-tags",
                "origin",
            ])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let refs = |root: &Path| {
            let repo = Repository::open(root).unwrap();
            repo.references_glob("refs/remotes/origin/*")
                .unwrap()
                .map(|r| {
                    let r = r.unwrap();
                    (
                        r.name().unwrap().to_owned(),
                        r.target().unwrap().to_string(),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        assert_eq!(refs(&root), refs(oracle.path()));
        let heads = |root: &Path| {
            let mut values = fs::read_to_string(root.join(".git/FETCH_HEAD"))
                .unwrap()
                .lines()
                .map(|line| line.split('\t').take(2).collect::<Vec<_>>().join("\t"))
                .collect::<Vec<_>>();
            values.sort();
            values
        };
        assert_eq!(heads(&root), heads(oracle.path()));
    }
    #[test]
    fn upstream_fetch_head_flags_and_order_match_git_matrix() {
        for refspec in [
            None,
            Some("+refs/heads/main:refs/remotes/origin/main"),
            Some("refs/heads/main"),
            Some("UNSET"),
            Some("HEAD"),
        ] {
            for (remote, merges, detached, exclude_topic) in [
                (Some("origin"), vec!["refs/heads/topic"], false, false),
                (None, vec!["refs/heads/topic"], false, false),
                (Some("other"), vec!["refs/heads/topic"], false, false),
                (Some("."), vec!["refs/heads/topic"], false, false),
                (Some("origin"), vec!["refs/heads/topic"], true, false),
                (
                    Some("origin"),
                    vec!["refs/heads/topic", "refs/heads/main"],
                    false,
                    false,
                ),
                (Some("origin"), vec!["refs/heads/topic"], false, true),
                (None, vec![], false, false),
                (Some("origin"), vec!["refs/heads/missing"], false, false),
            ] {
                let fixture = fixture();
                let root = fixture.path().join("target").canonicalize().unwrap();
                let oracle = tempfile::tempdir().unwrap();
                let oracle_repo = Repository::init(oracle.path()).unwrap();
                let sig = git2::Signature::now("Oracle", "oracle@example.invalid").unwrap();
                let tree = oracle_repo
                    .find_tree(oracle_repo.treebuilder(None).unwrap().write().unwrap())
                    .unwrap();
                oracle_repo
                    .commit(Some("HEAD"), &sig, &sig, "oracle", &tree, &[])
                    .unwrap();
                drop(tree);
                oracle_repo
                    .remote("origin", fixture.path().join("source").to_str().unwrap())
                    .unwrap();
                for repo in [Repository::open(&root).unwrap(), oracle_repo] {
                    let branch = repo
                        .head()
                        .unwrap()
                        .name()
                        .unwrap()
                        .strip_prefix("refs/heads/")
                        .unwrap()
                        .to_owned();
                    let mut config = repo.config().unwrap();
                    if let Some(refspec) = refspec {
                        if refspec == "UNSET" {
                            config.remove_multivar("remote.origin.fetch", ".*").unwrap();
                        } else {
                            config.set_str("remote.origin.fetch", refspec).unwrap();
                        }
                    }
                    if let Some(remote) = remote {
                        config
                            .set_str(&format!("branch.{branch}.remote"), remote)
                            .unwrap();
                    }
                    for source in &merges {
                        config
                            .set_multivar(&format!("branch.{branch}.merge"), "a^", source)
                            .unwrap();
                    }
                    if exclude_topic {
                        config
                            .set_multivar("remote.origin.fetch", "a^", "^refs/heads/topic")
                            .unwrap();
                    }
                    if detached {
                        repo.set_head_detached(repo.head().unwrap().target().unwrap())
                            .unwrap();
                    }
                }
                let before = preserved(&root);
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                execute(
                    &mut session,
                    &snapshot.revision,
                    "origin",
                    &download(fixture.path()),
                )
                .unwrap();
                assert_eq!(preserved(&root), before);
                let output = Command::new(crate::test_git_executable())
                    .arg("-C")
                    .arg(oracle.path())
                    .args([
                        "-c",
                        "protocol.file.allow=always",
                        "-c",
                        "remote.origin.followRemoteHEAD=never",
                        "fetch",
                        "--no-tags",
                        "origin",
                    ])
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let heads = |root: &Path| {
                    fs::read_to_string(root.join(".git/FETCH_HEAD"))
                        .unwrap()
                        .lines()
                        .map(|line| line.split('\t').take(2).collect::<Vec<_>>().join("\t"))
                        .collect::<Vec<_>>()
                };
                assert_eq!(heads(&root),heads(oracle.path()),"refspec {refspec:?}, remote {remote:?}, merges {merges:?}, detached {detached}, excluded {exclude_topic}");
                let tracking = |path: &Path| {
                    let repo = Repository::open(path).unwrap();
                    repo.references_glob("refs/remotes/origin/*")
                        .unwrap()
                        .map(|r| {
                            let r = r.unwrap();
                            (
                                r.name().unwrap().to_owned(),
                                r.target().unwrap().to_string(),
                            )
                        })
                        .collect::<std::collections::BTreeMap<_, _>>()
                };
                assert_eq!(tracking(&root), tracking(oracle.path()));
            }
        }
    }
    #[test]
    fn fetch_unborn_upstream_preserves_head_and_absent_index() {
        let fixture = fixture();
        let target = tempfile::tempdir().unwrap();
        let root = target.path().canonicalize().unwrap();
        let repo = Repository::init(&root).unwrap();
        repo.set_head("refs/heads/local").unwrap();
        repo.remote("origin", URL).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("branch.local.remote", "origin").unwrap();
        config
            .set_str("branch.local.merge", "refs/heads/topic")
            .unwrap();
        fs::write(root.join("dirty.txt"), "untracked data").unwrap();
        let before =
            [".git/HEAD", ".git/config", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap());
        assert!(!root.join(".git/index").exists());
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        assert!(!root.join(".git/index").exists());
        assert!(matches!(repo.head(), Err(error) if error.code() == git2::ErrorCode::UnbornBranch));
        assert_eq!(
            before,
            [".git/HEAD", ".git/config", "dirty.txt"].map(|p| fs::read(root.join(p)).unwrap())
        );
        let heads = fs::read_to_string(root.join(".git/FETCH_HEAD")).unwrap();
        assert!(heads
            .lines()
            .next()
            .unwrap()
            .contains("\t\trefs/heads/topic"));
        check(&root);
    }
    #[test]
    fn legacy_fetch_journal_preserves_v1_no_merge_bytes() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        let branch = repo
            .head()
            .unwrap()
            .name()
            .unwrap()
            .strip_prefix("refs/heads/")
            .unwrap()
            .to_owned();
        repo.config()
            .unwrap()
            .set_str(&format!("branch.{branch}.remote"), "origin")
            .unwrap();
        repo.config()
            .unwrap()
            .set_str(&format!("branch.{branch}.merge"), "refs/heads/topic")
            .unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let data = download(fixture.path());
        let plan = preview(
            &mut session,
            &snapshot.revision,
            "origin",
            data.advertisement(),
        )
        .unwrap();
        let mut record = prepare(&root, "origin", URL, plan).unwrap();
        assert_eq!(record.version, 5);
        let modern = String::from_utf8(fetch_head(&record).unwrap()).unwrap();
        assert!(modern
            .lines()
            .next()
            .unwrap()
            .contains("\t\trefs/heads/topic"));
        for version in [4, 3, 2] {
            record.version = version;
            verify(&record).unwrap();
        }
        record.version = 1;
        let legacy = fetch_head(&record).unwrap();
        assert!(String::from_utf8_lossy(&legacy)
            .lines()
            .all(|line| line.contains("\tnot-for-merge\t")));
        record.changes.last_mut().unwrap().after = Some(legacy.clone());
        verify(&record).unwrap();
        record.changes.last_mut().unwrap().after = Some(modern.into_bytes());
        assert!(verify(&record).is_err());
        record.changes.last_mut().unwrap().after = Some(legacy);
        record.version = 6;
        assert!(verify(&record).is_err());
    }
    #[test]
    fn fetch_respects_disabled_creation_of_remote_reflogs() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        Repository::open(&root)
            .unwrap()
            .config()
            .unwrap()
            .set_bool("core.logallrefupdates", false)
            .unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        check(&root);
        assert!(!root.join(".git/logs/refs/remotes/origin/main").exists());
        assert!(!root.join(".git/logs/refs/remotes/origin/topic").exists());
    }
    #[test]
    fn fetch_refuses_stale_fetch_head_external_locks_and_other_destinations() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let data = download(fixture.path());
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        fs::write(root.join(".git/FETCH_HEAD"), "external").unwrap();
        assert!(execute(&mut session, &snapshot.revision, "origin", &data)
            .unwrap_err()
            .contains("STALE_STATE"));
        let snapshot = session.snapshot().unwrap();
        fs::write(root.join(".git/FETCH_HEAD.lock"), "foreign").unwrap();
        assert!(execute(&mut session, &snapshot.revision, "origin", &data)
            .unwrap_err()
            .contains("EXTERNAL_LOCK"));
        assert_eq!(
            fs::read_to_string(root.join(".git/FETCH_HEAD.lock")).unwrap(),
            "foreign"
        );
        fs::remove_file(root.join(".git/FETCH_HEAD.lock")).unwrap();
        assert!(!pending(&root));
        Repository::open(&root)
            .unwrap()
            .config()
            .unwrap()
            .set_str("remote.origin.fetch", "+refs/heads/*:refs/heads/*")
            .unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(execute(&mut session, &snapshot.revision, "origin", &data)
            .unwrap_err()
            .contains("TRACKING_REFS_ONLY"));
        assert!(!pending(&root));
    }
    #[test]
    #[ignore = "requires selected release worker and public HTTPS"]
    fn actual_release_download_fetches_into_synthetic_worktree() {
        let binary = std::env::var_os("BRANCHDESK_WORKER_BINARY").expect("select built worker");
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let url = "https://github.com/morluto/awesome-reverse-engineering.git";
        Repository::open(&root)
            .unwrap()
            .config()
            .unwrap()
            .set_str("remote.origin.url", url)
            .unwrap();
        let before = preserved(&root);
        let data = Downloaded::fixture_binary(&binary, url).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let plan = preview_download(&mut session, &snapshot.revision, "origin", &data).unwrap();
        assert!(!plan.mappings.is_empty());
        let result = execute(&mut session, &snapshot.revision, "origin", &data).unwrap();
        assert!(!result.recovery_pending);
        assert_eq!(preserved(&root), before);
        let repo = Repository::open(&root).unwrap();
        for mapping in &plan.mappings {
            if let Some(destination) = &mapping.destination {
                assert_eq!(
                    repo.find_reference(destination)
                        .unwrap()
                        .target()
                        .unwrap()
                        .to_string(),
                    mapping.after
                );
            }
        }
        let checked = Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(&root)
            .args(["fsck", "--strict", "--no-reflogs"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        println!("actual release fetch published {} configured mappings; HEAD/index/config/worktree preserved; git fsck strict passed",plan.mappings.len());
    }
    #[test]
    fn fetch_crash_child() {
        let Some(root) = std::env::var_os("BRANCHDESK_FETCH_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        if std::env::var_os("BRANCHDESK_FETCH_RECOVER").is_some() {
            session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
        } else {
            execute(
                &mut session,
                &snapshot.revision,
                "origin",
                &download(root.parent().unwrap()),
            )
            .unwrap();
        }
    }
    fn kill(root: &Path, step: usize, recovery: bool) {
        let _ = fs::remove_file(root.join(".git/fetch-ready"));
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "repository::fetch::tests::fetch_crash_child",
                "--nocapture",
            ])
            .env("BRANCHDESK_FETCH_ROOT", root)
            .env("BRANCHDESK_FETCH_STEP", step.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if recovery {
            command.env("BRANCHDESK_FETCH_RECOVER", "1");
        }
        let mut child = command.spawn().unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while !root.join(".git/fetch-ready").exists() {
            if Instant::now() > until {
                let _ = child.kill();
                let _ = child.wait();
                panic!("fetch checkpoint timeout");
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before checkpoint {step}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn fetch_recovers_every_metadata_boundary_and_repeated_recovery_kill() {
        for step in 0..=8 {
            let fixture = fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            let repo = Repository::open(&root).unwrap();
            let branch = repo
                .head()
                .unwrap()
                .name()
                .unwrap()
                .strip_prefix("refs/heads/")
                .unwrap()
                .to_owned();
            let mut config = repo.config().unwrap();
            config
                .set_str(&format!("branch.{branch}.remote"), "origin")
                .unwrap();
            config
                .set_str(&format!("branch.{branch}.merge"), "refs/heads/topic")
                .unwrap();
            let before = preserved(&root);
            kill(&root, step, false);
            kill(&root, step, true);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(snapshot.recovery_pending);
            let result = session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!result.recovery_pending);
            assert_eq!(preserved(&root), before);
            check(&root);
            assert!(fs::read_to_string(root.join(".git/FETCH_HEAD"))
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .contains("\t\trefs/heads/topic"));
            for path in [
                "index.lock",
                "FETCH_HEAD.lock",
                "HEAD.lock",
                "config.lock",
                "packed-refs.lock",
            ] {
                assert!(!root.join(".git").join(path).exists());
            }
        }
    }
    #[test]
    fn fetch_retries_after_archive_kill_but_preserves_unowned_or_invalid_pins() {
        for mode in [
            "owned",
            "foreign",
            "invalid",
            "unrecorded",
            "linked",
            "changed",
        ] {
            let fixture = fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            let before = preserved(&root);
            kill(&root, 100, false);
            assert!(!pending(&root));
            let archive = fs::read_dir(root.join(".git"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("branchdesk-fetch-completed-")
                })
                .unwrap();
            let pin = fs::read_dir(root.join(".git/objects/pack"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "keep"))
                .unwrap();
            match mode {
                "foreign" => {
                    fs::write(&pin, "external owner\n").unwrap();
                }
                "changed" => {
                    fs::write(root.join(".git/FETCH_HEAD"), "external fetch\n").unwrap();
                }
                "invalid" => {
                    let path = archive.join("record.json");
                    let mut record: Record =
                        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                    record.changes[3].path = "../dirty.txt".into();
                    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
                }
                "unrecorded" => {
                    fs::rename(&archive, root.join("saved-archive")).unwrap();
                }
                "linked" => {
                    let saved = root.join("saved-archive");
                    fs::rename(&archive, &saved).unwrap();
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&saved, &archive).unwrap();
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&saved, &archive).unwrap();
                }
                _ => {}
            }
            let pin_before = fs::read(&pin).unwrap();
            if mode == "owned" {
                // A second interrupted retry must not depend on the first archive's token.
                kill(&root, 100, false);
            }
            if mode == "linked" {
                assert!(Session::new(root.clone())
                    .err()
                    .unwrap()
                    .contains("UNSUPPORTED_LAYOUT"));
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                cleanup_completed_pin(&root, download(fixture.path()).pack_hash().unwrap())
                    .unwrap();
                assert_eq!(fs::read(&pin).unwrap(), pin_before);
                assert_eq!(preserved(&root), before);
                continue;
            }
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let result = execute(
                &mut session,
                &snapshot.revision,
                "origin",
                &download(fixture.path()),
            );
            if mode == "owned" {
                assert!(!result.unwrap().recovery_pending);
                assert!(!pin.exists());
                check(&root);
            } else {
                assert!(
                    result.unwrap_err().contains("FETCH_PACK_ALREADY_PINNED"),
                    "{mode}"
                );
                assert_eq!(fs::read(&pin).unwrap(), pin_before);
                assert!(!pending(&root));
            }
            assert_eq!(preserved(&root), before);
        }
    }
    #[test]
    fn completed_pin_cleanup_refuses_pending_and_bounded_scan_without_deletion() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        kill(&root, 100, false);
        let data = download(fixture.path());
        let hash = data.pack_hash().unwrap();
        let path = root.join(format!(".git/objects/pack/pack-{hash}.keep"));
        let bytes = fs::read(&path).unwrap();
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        fs::create_dir(root.join(JOURNAL)).unwrap();
        assert!(cleanup_completed_pin(&root, hash)
            .unwrap_err()
            .contains("FETCH_PENDING_OPERATION"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir(root.join(JOURNAL)).unwrap();
        for i in 0..4096 {
            fs::write(root.join(format!(".git/scan-entry-{i}")), "").unwrap();
        }
        assert!(cleanup_completed_pin(&root, hash)
            .unwrap_err()
            .contains("FETCH_ARCHIVE_SCAN_LIMIT"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        for i in 0..4096 {
            fs::remove_file(root.join(format!(".git/scan-entry-{i}"))).unwrap();
        }
        cleanup_completed_pin(&root, hash).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn fetch_pack_pin_survives_gc_after_kill_and_is_released_after_recovery() {
        for protected in [true, false] {
            let fixture = fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            let before = preserved(&root);
            let remote_oid = Repository::open_bare(fixture.path().join("source"))
                .unwrap()
                .refname_to_id("refs/heads/topic")
                .unwrap();
            kill(&root, 0, false);
            let pin = fs::read_dir(root.join(".git/objects/pack"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "keep"))
                .unwrap();
            if !protected {
                fs::remove_file(&pin).unwrap();
            }
            let gc = || {
                let output = Command::new(crate::test_git_executable())
                    .arg("-C")
                    .arg(&root)
                    .args([
                        "-c",
                        "gc.packRefs=false",
                        "-c",
                        "gc.writeCommitGraph=false",
                        // Reflog expiry takes HEAD.lock, deliberately retained by
                        // the crash fixture. Exercise object GC without changing refs.
                        "-c",
                        "gc.reflogExpire=never",
                        "-c",
                        "gc.reflogExpireUnreachable=never",
                        "gc",
                        "--prune=now",
                    ])
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            };
            gc();
            assert_eq!(
                Repository::open(&root)
                    .unwrap()
                    .find_commit(remote_oid)
                    .is_ok(),
                protected
            );
            if protected {
                kill(&root, 0, true);
                gc();
            }
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let result = session.mutate(&snapshot.revision, Action::RecoverCheckout);
            if protected {
                assert!(!result.unwrap().recovery_pending);
                check(&root);
                assert!(!pin.exists());
            } else {
                assert!(result.unwrap_err().contains("FETCH_PACK_PIN_CHANGED"));
                assert!(pending(&root));
            }
            assert_eq!(preserved(&root), before);
        }
    }
    #[test]
    fn fetch_preserves_foreign_pack_pin_and_refuses_replaced_recovery_pin() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let data = download(fixture.path());
        let pin = data.pin(&root.join(".git")).unwrap().unwrap();
        let path = fs::read_dir(root.join(".git/objects/pack"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "keep"))
            .unwrap();
        pin.release(&root.join(".git")).unwrap();
        fs::write(&path, "foreign owner\n").unwrap();
        let before = preserved(&root);
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(execute(&mut session, &snapshot.revision, "origin", &data)
            .unwrap_err()
            .contains("FETCH_PACK_ALREADY_PINNED"));
        assert!(!pending(&root));
        assert_eq!(fs::read_to_string(&path).unwrap(), "foreign owner\n");
        fs::remove_file(&path).unwrap();
        kill(&root, 0, false);
        fs::write(&path, "replacement owner\n").unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(session
            .mutate(&snapshot.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("FETCH_PACK_PIN_CHANGED"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement owner\n");
        assert_eq!(preserved(&root), before);
    }
    #[test]
    fn fetch_upstream_only_survives_publication_and_recovery_kills() {
        for step in [0, 6] {
            let fixture = fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            let repo = Repository::open(&root).unwrap();
            let branch = repo
                .head()
                .unwrap()
                .name()
                .unwrap()
                .strip_prefix("refs/heads/")
                .unwrap()
                .to_owned();
            let mut config = repo.config().unwrap();
            config
                .set_str(
                    "remote.origin.fetch",
                    "+refs/heads/main:refs/remotes/origin/main",
                )
                .unwrap();
            config
                .set_str(&format!("branch.{branch}.remote"), "origin")
                .unwrap();
            config
                .set_str(&format!("branch.{branch}.merge"), "refs/heads/topic")
                .unwrap();
            let before = preserved(&root);
            kill(&root, step, false);
            kill(&root, step, true);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let result = session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!result.recovery_pending);
            assert_eq!(preserved(&root), before);
            assert!(repo.find_reference("refs/remotes/origin/main").is_ok());
            assert!(repo.find_reference("refs/remotes/origin/topic").is_err());
            assert!(fs::read_to_string(root.join(".git/FETCH_HEAD"))
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .contains("\t\trefs/heads/topic"));
        }
    }
    #[test]
    fn fetch_recovery_refuses_external_ref_change_and_tampered_paths() {
        for tamper in [false, true] {
            let fixture = fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            kill(&root, 0, false);
            if tamper {
                let path = root.join(JOURNAL).join("record.json");
                let mut record: Record = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record.changes[3].path = "../dirty.txt".into();
                fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
            } else {
                fs::write(
                    root.join(".git/refs/remotes/origin/main"),
                    "1111111111111111111111111111111111111111\n",
                )
                .unwrap();
            }
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            journal_locks::release(&root, &_lease).unwrap();
            let _index = IndexLock::acquire(&root).unwrap();
            assert!(recover(&root).is_err());
            assert_eq!(
                fs::read_to_string(root.join("dirty.txt")).unwrap(),
                "keep worktree"
            );
            assert!(pending(&root));
        }
    }
    fn tag_fixture() -> tempfile::TempDir {
        let fixture = fixture();
        Repository::open(fixture.path().join("target"))
            .unwrap()
            .config()
            .unwrap()
            .set_str("remote.origin.tagopt", "--no-tags")
            .unwrap();
        let source = Repository::open_bare(fixture.path().join("source")).unwrap();
        let commit = source.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        source
            .tag_lightweight("light", commit.as_object(), false)
            .unwrap();
        let tag = source
            .tag("annotated", commit.as_object(), &sig, "annotation", false)
            .unwrap();
        source
            .tag(
                "nested",
                &source.find_object(tag, None).unwrap(),
                &sig,
                "nested tag",
                false,
            )
            .unwrap();
        let blob = source.blob(b"tagged blob").unwrap();
        source
            .tag_lightweight("blob", &source.find_object(blob, None).unwrap(), false)
            .unwrap();
        source
            .tag_lightweight("tree", commit.tree().unwrap().as_object(), false)
            .unwrap();
        fixture
    }
    fn tag_refs(root: &Path) -> std::collections::BTreeMap<String, String> {
        Repository::open(root)
            .unwrap()
            .references_glob("refs/tags/*")
            .unwrap()
            .map(|r| {
                let r = r.unwrap();
                (
                    r.name().unwrap().to_owned(),
                    r.target().unwrap().to_string(),
                )
            })
            .collect()
    }
    fn fetch_head_flags(root: &Path) -> Vec<String> {
        fs::read_to_string(root.join(".git/FETCH_HEAD"))
            .unwrap()
            .lines()
            .map(|l| l.split('\t').take(2).collect::<Vec<_>>().join("\t"))
            .collect()
    }
    fn tag_git(root: &Path, args: &[&str]) -> std::process::Output {
        Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
    }
    #[test]
    fn explicit_tag_fetch_matches_git_objects_flags_and_reflog_policy() {
        for source in ["*", "light", "annotated", "nested", "blob", "tree"] {
            for logging in ["true", "false", "always"] {
                let fixture = tag_fixture();
                let root = fixture.path().join("target").canonicalize().unwrap();
                let oracle = tempfile::tempdir().unwrap();
                let spec = format!("refs/tags/{source}:refs/tags/{source}");
                let repo = Repository::open(&root).unwrap();
                let mut config = repo.config().unwrap();
                config.set_str("remote.origin.fetch", &spec).unwrap();
                config.set_str("core.logallrefupdates", logging).unwrap();
                let other = Repository::init(oracle.path()).unwrap();
                other
                    .remote("origin", fixture.path().join("source").to_str().unwrap())
                    .unwrap();
                let mut other_config = other.config().unwrap();
                other_config.set_str("remote.origin.fetch", &spec).unwrap();
                other_config
                    .set_str("core.logallrefupdates", logging)
                    .unwrap();
                let before = preserved(&root);
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                execute(
                    &mut session,
                    &snapshot.revision,
                    "origin",
                    &download(fixture.path()),
                )
                .unwrap();
                let result = tag_git(
                    oracle.path(),
                    &[
                        "-c",
                        "protocol.file.allow=always",
                        "fetch",
                        "--no-tags",
                        "origin",
                    ],
                );
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(tag_refs(&root), tag_refs(oracle.path()), "{spec} {logging}");
                assert_eq!(
                    fetch_head_flags(&root),
                    fetch_head_flags(oracle.path()),
                    "{spec} {logging}"
                );
                for name in tag_refs(&root).keys() {
                    assert_eq!(
                        root.join(".git/logs").join(name).exists(),
                        oracle.path().join(".git/logs").join(name).exists()
                    );
                }
                assert_eq!(preserved(&root), before);
                assert!(tag_git(&root, &["fsck", "--strict"]).status.success());
            }
        }
    }
    #[test]
    fn tag_replacement_requires_force_and_preserves_packed_bytes_and_existing_log() {
        let fixture = tag_fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let oracle = tempfile::tempdir().unwrap();
        let oracle_repo = Repository::init(oracle.path()).unwrap();
        oracle_repo
            .remote("origin", fixture.path().join("source").to_str().unwrap())
            .unwrap();
        oracle_repo
            .config()
            .unwrap()
            .set_str("remote.origin.fetch", "refs/tags/light:refs/tags/light")
            .unwrap();
        assert!(tag_git(
            oracle.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "fetch",
                "--no-tags",
                "origin"
            ]
        )
        .status
        .success());
        let repo = Repository::open(&root).unwrap();
        let mut config = repo.config().unwrap();
        config
            .set_str("remote.origin.fetch", "refs/tags/light:refs/tags/light")
            .unwrap();
        config.set_str("core.logallrefupdates", "always").unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        assert!(tag_git(&root, &["pack-refs", "--all", "--prune"])
            .status
            .success());
        let packed = fs::read(root.join(".git/packed-refs")).unwrap();
        let old_log = fs::read(root.join(".git/logs/refs/tags/light")).unwrap();
        config.set_bool("core.logallrefupdates", false).unwrap();
        let source = Repository::open_bare(fixture.path().join("source")).unwrap();
        let new = source.blob(b"retagged blob").unwrap();
        source
            .reference("refs/tags/light", new, true, "retag")
            .unwrap();
        let before = preserved(&root);
        let old = tag_refs(&root);
        let snapshot = session.snapshot().unwrap();
        assert!(execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path())
        )
        .unwrap_err()
        .contains("FETCH_TAG_EXISTS"));
        assert_eq!(tag_refs(&root), old);
        assert_eq!(preserved(&root), before);
        assert!(!pending(&root));
        assert!(!tag_git(
            oracle.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "fetch",
                "--no-tags",
                "origin"
            ]
        )
        .status
        .success());
        assert_eq!(tag_refs(&root), tag_refs(oracle.path()));
        oracle_repo
            .config()
            .unwrap()
            .set_str("remote.origin.fetch", "+refs/tags/light:refs/tags/light")
            .unwrap();
        assert!(tag_git(
            oracle.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "fetch",
                "--no-tags",
                "origin"
            ]
        )
        .status
        .success());
        config
            .set_str("remote.origin.fetch", "+refs/tags/light:refs/tags/light")
            .unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        assert_eq!(tag_refs(&root), tag_refs(oracle.path()));
        assert_eq!(fetch_head_flags(&root), fetch_head_flags(oracle.path()));
        assert_eq!(repo.refname_to_id("refs/tags/light").unwrap(), new);
        assert_eq!(fs::read(root.join(".git/packed-refs")).unwrap(), packed);
        assert!(fs::read(root.join(".git/logs/refs/tags/light"))
            .unwrap()
            .starts_with(&old_log));
        assert!(fetch_head_flags(&root)[0].ends_with("not-for-merge"));
    }
    #[test]
    fn tag_fetch_recovers_all_metadata_boundaries_and_second_kill() {
        for tag in ["annotated", "blob"] {
            for step in 0..=6 {
                let fixture = tag_fixture();
                let root = fixture.path().join("target").canonicalize().unwrap();
                Repository::open(&root)
                    .unwrap()
                    .config()
                    .unwrap()
                    .set_str(
                        "remote.origin.fetch",
                        &format!("refs/tags/{tag}:refs/tags/{tag}"),
                    )
                    .unwrap();
                let before = preserved(&root);
                kill(&root, step, false);
                kill(&root, step, true);
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                let result = session
                    .mutate(&snapshot.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!result.recovery_pending);
                let source = Repository::open_bare(fixture.path().join("source")).unwrap();
                assert_eq!(
                    Repository::open(&root)
                        .unwrap()
                        .refname_to_id(&format!("refs/tags/{tag}"))
                        .unwrap(),
                    source.refname_to_id(&format!("refs/tags/{tag}")).unwrap()
                );
                assert_eq!(preserved(&root), before);
                assert_eq!(
                    fetch_head_flags(&root)[0].ends_with("not-for-merge"),
                    tag == "blob"
                );
            }
        }
    }
    #[test]
    fn tag_journal_rejects_forged_force_type_flags_and_legacy_destination() {
        for mode in ["force", "type", "legacy"] {
            let fixture = tag_fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            Repository::open(&root)
                .unwrap()
                .config()
                .unwrap()
                .set_str("remote.origin.fetch", "refs/tags/blob:refs/tags/blob")
                .unwrap();
            kill(&root, 0, false);
            let path = root.join(JOURNAL).join("record.json");
            let mut record: Record = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match mode {
                "force" => record.mappings[0].force = true,
                "type" => {
                    record.non_commit_sources.clear();
                    record.changes.last_mut().unwrap().after = Some(fetch_head(&record).unwrap());
                }
                "legacy" => {
                    record.version = 3;
                    record.non_commit_sources.clear();
                    record.changes.last_mut().unwrap().after = Some(fetch_head(&record).unwrap());
                }
                _ => unreachable!(),
            }
            fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
            let before = preserved(&root);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .is_err());
            assert!(tag_refs(&root).is_empty());
            assert_eq!(preserved(&root), before);
        }
    }
    #[test]
    fn automatic_and_all_tag_policies_match_git_selection_and_flags() {
        for policy in ["auto", "--tags", "--no-tags"] {
            for mode in [
                "tracking",
                "source-only",
                "no-config",
                "negative",
                "existing",
            ] {
                let fixture = tag_fixture();
                let root = fixture.path().join("target").canonicalize().unwrap();
                let source = Repository::open_bare(fixture.path().join("source")).unwrap();
                let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
                let tree = source
                    .head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .tree()
                    .unwrap();
                let orphan = source
                    .commit(None, &sig, &sig, "unrelated tag", &tree, &[])
                    .unwrap();
                source
                    .tag_lightweight("orphan", &source.find_object(orphan, None).unwrap(), false)
                    .unwrap();
                let oracle = tempfile::tempdir().unwrap();
                let other = Repository::init(oracle.path()).unwrap();
                other
                    .remote("origin", fixture.path().join("source").to_str().unwrap())
                    .unwrap();
                assert!(tag_git(
                    oracle.path(),
                    &[
                        "-c",
                        "protocol.file.allow=always",
                        "fetch",
                        "--no-tags",
                        fixture.path().join("target").to_str().unwrap(),
                        "HEAD"
                    ]
                )
                .status
                .success());
                let repo = Repository::open(&root).unwrap();
                for r in [&repo, &other] {
                    let mut config = r.config().unwrap();
                    if policy == "auto" {
                        let _ = config.remove("remote.origin.tagopt");
                    } else {
                        config.set_str("remote.origin.tagopt", policy).unwrap();
                    }
                    match mode {
                        "source-only" => config
                            .set_str("remote.origin.fetch", "refs/heads/main")
                            .unwrap(),
                        "no-config" => config.remove_multivar("remote.origin.fetch", ".*").unwrap(),
                        "negative" => config
                            .set_multivar("remote.origin.fetch", "a^", "^refs/tags/light")
                            .unwrap(),
                        "existing" => {
                            let oid = r.blob(b"existing local tag").unwrap();
                            r.reference("refs/tags/light", oid, false, "fixture")
                                .unwrap();
                        }
                        _ => {}
                    }
                }
                let before = preserved(&root);
                let old_tags = tag_refs(&root);
                let data = download(fixture.path());
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                let preview = preview_download(&mut session, &snapshot.revision, "origin", &data);
                assert_eq!(tag_refs(&root), old_tags);
                assert_eq!(preserved(&root), before);
                let actual = execute(&mut session, &snapshot.revision, "origin", &data);
                let oracle_result = tag_git(
                    oracle.path(),
                    &["-c", "protocol.file.allow=always", "fetch", "origin"],
                );
                if mode == "existing" && policy == "--tags" {
                    assert!(!oracle_result.status.success());
                    assert!(preview.is_err());
                    assert!(actual.unwrap_err().contains("FETCH_TAG_EXISTS"));
                    assert_eq!(tag_refs(&root), old_tags);
                } else {
                    assert!(
                        oracle_result.status.success(),
                        "{}",
                        String::from_utf8_lossy(&oracle_result.stderr)
                    );
                    actual.unwrap();
                    assert_eq!(tag_refs(&root), tag_refs(oracle.path()), "{policy} {mode}");
                    assert_eq!(
                        fetch_head_flags(&root),
                        fetch_head_flags(oracle.path()),
                        "{policy} {mode}"
                    );
                }
                assert_eq!(preserved(&root), before);
            }
        }
    }
    #[test]
    fn automatic_tags_follow_ancestors_and_local_objects_not_unrelated_commits() {
        for local_blob in [false, true] {
            let fixture = tag_fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            let repo = Repository::open(&root).unwrap();
            repo.config()
                .unwrap()
                .remove("remote.origin.tagopt")
                .unwrap();
            repo.config()
                .unwrap()
                .set_str(
                    "remote.origin.fetch",
                    "refs/heads/main:refs/remotes/origin/main",
                )
                .unwrap();
            let source = Repository::open_bare(fixture.path().join("source")).unwrap();
            let parent = source.head().unwrap().peel_to_commit().unwrap();
            let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
            let mut builder = source.treebuilder(None).unwrap();
            builder
                .insert("file", source.blob(b"new file").unwrap(), 0o100644)
                .unwrap();
            let tree = source.find_tree(builder.write().unwrap()).unwrap();
            source
                .commit(
                    Some("refs/heads/main"),
                    &sig,
                    &sig,
                    "descendant",
                    &tree,
                    &[&parent],
                )
                .unwrap();
            let orphan = source
                .commit(None, &sig, &sig, "unrelated", &tree, &[])
                .unwrap();
            source
                .tag_lightweight(
                    "unrelated",
                    &source.find_object(orphan, None).unwrap(),
                    false,
                )
                .unwrap();
            let oracle = tempfile::tempdir().unwrap();
            let other = Repository::init(oracle.path()).unwrap();
            other
                .remote("origin", fixture.path().join("source").to_str().unwrap())
                .unwrap();
            other
                .config()
                .unwrap()
                .set_str(
                    "remote.origin.fetch",
                    "refs/heads/main:refs/remotes/origin/main",
                )
                .unwrap();
            assert!(tag_git(
                oracle.path(),
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "fetch",
                    "--no-tags",
                    fixture.path().join("target").to_str().unwrap(),
                    "HEAD"
                ]
            )
            .status
            .success());
            if local_blob {
                repo.blob(b"tagged blob").unwrap();
                other.blob(b"tagged blob").unwrap();
            }
            let mut session = Session::new(root.clone()).unwrap();
            for _round in 0..3 {
                let snapshot = session.snapshot().unwrap();
                execute(
                    &mut session,
                    &snapshot.revision,
                    "origin",
                    &download(fixture.path()),
                )
                .unwrap();
                assert!(tag_git(
                    oracle.path(),
                    &["-c", "protocol.file.allow=always", "fetch", "origin"]
                )
                .status
                .success());
                assert_eq!(tag_refs(&root), tag_refs(oracle.path()));
                assert_eq!(fetch_head_flags(&root), fetch_head_flags(oracle.path()));
                assert!(!tag_refs(&root).contains_key("refs/tags/unrelated"));
                assert_eq!(tag_refs(&root).contains_key("refs/tags/blob"), local_blob);
                let current = Repository::open(&root).unwrap();
                assert!(!current.odb().unwrap().exists(orphan));
                assert_eq!(
                    current
                        .odb()
                        .unwrap()
                        .exists(source.refname_to_id("refs/tags/blob").unwrap()),
                    local_blob
                );
            }
        }
    }
    #[test]
    fn implicit_tags_survive_actual_kill_and_second_recovery_kill() {
        for policy in ["auto", "--tags"] {
            for boundary in [0, 3, 4, 12, 16] {
                let fixture = tag_fixture();
                let root = fixture.path().join("target").canonicalize().unwrap();
                let repo = Repository::open(&root).unwrap();
                if policy == "auto" {
                    repo.config()
                        .unwrap()
                        .remove("remote.origin.tagopt")
                        .unwrap();
                } else {
                    repo.config()
                        .unwrap()
                        .set_str("remote.origin.tagopt", policy)
                        .unwrap();
                }
                let before = preserved(&root);
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                let data = download(fixture.path());
                let plan =
                    preview_download(&mut session, &snapshot.revision, "origin", &data).unwrap();
                let expected = plan
                    .mappings
                    .iter()
                    .filter(|m| m.source.starts_with("refs/tags/"))
                    .map(|m| (m.source.clone(), m.after.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let count = prepare(&root, "origin", URL, plan).unwrap().changes.len();
                let step = if boundary == 16 { count } else { boundary };
                kill(&root, step, false);
                kill(&root, step, true);
                let snapshot = session.snapshot().unwrap();
                let next = session
                    .mutate(&snapshot.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!next.recovery_pending);
                assert_eq!(tag_refs(&root), expected);
                assert_eq!(preserved(&root), before);
            }
        }
    }
    #[test]
    fn implicit_tag_journal_refuses_other_destinations_and_changed_policy() {
        for mode in ["destination", "policy", "legacy"] {
            let fixture = tag_fixture();
            let root = fixture.path().join("target").canonicalize().unwrap();
            Repository::open(&root)
                .unwrap()
                .config()
                .unwrap()
                .remove("remote.origin.tagopt")
                .unwrap();
            kill(&root, 0, false);
            let path = root.join(JOURNAL).join("record.json");
            let mut record: Record = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert!(!record.implicit_tags.is_empty());
            match mode {
                "destination" => {
                    let tag = &record.implicit_tags[0];
                    record
                        .mappings
                        .iter_mut()
                        .find(|m| &m.source == tag)
                        .unwrap()
                        .destination = Some("refs/remotes/origin/injected".into());
                }
                "policy" => {
                    let config = record.changes[0].before.as_mut().unwrap();
                    config.extend_from_slice(b"\n[remote \"origin\"]\ntagopt = --no-tags\n");
                    record.changes[0].after = record.changes[0].before.clone();
                }
                "legacy" => record.version = 4,
                _ => unreachable!(),
            }
            assert!(verify(&record).is_err());
            fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
            let before = preserved(&root);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .is_err());
            assert_eq!(preserved(&root), before);
            assert!(tag_refs(&root).is_empty());
        }
    }
    #[test]
    fn empty_fetch_selection_imports_no_objects_and_preserves_worktree() {
        let fixture = tag_fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        repo.config()
            .unwrap()
            .set_str("remote.origin.fetch", "^refs/*")
            .unwrap();
        let inventory = |repo: &Repository| {
            let mut ids = std::collections::BTreeSet::new();
            repo.odb()
                .unwrap()
                .foreach(|oid| {
                    ids.insert(*oid);
                    true
                })
                .unwrap();
            ids
        };
        let old = inventory(&repo);
        let before = preserved(&root);
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        execute(
            &mut session,
            &snapshot.revision,
            "origin",
            &download(fixture.path()),
        )
        .unwrap();
        assert_eq!(inventory(&Repository::open(&root).unwrap()), old);
        assert_eq!(preserved(&root), before);
        assert_eq!(fs::read(root.join(".git/FETCH_HEAD")).unwrap(), b"");
    }
    #[test]
    fn selected_pack_pin_survives_gc_and_actual_kill_before_ref_publication() {
        let fixture = fixture();
        let root = fixture.path().join("target").canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        repo.config()
            .unwrap()
            .set_str(
                "remote.origin.fetch",
                "refs/heads/main:refs/remotes/origin/main",
            )
            .unwrap();
        let source = Repository::open_bare(fixture.path().join("source")).unwrap();
        let wanted = source.refname_to_id("refs/heads/main").unwrap();
        let unwanted = source.refname_to_id("refs/heads/topic").unwrap();
        let original = download(fixture.path());
        let selected = original.project(&[wanted.to_string()]).unwrap();
        assert_ne!(selected.pack_hash(), original.pack_hash());
        let before = preserved(&root);
        kill(&root, 0, false);
        let record: Record =
            serde_json::from_slice(&fs::read(root.join(JOURNAL).join("record.json")).unwrap())
                .unwrap();
        assert!(record
            .pack_pin
            .as_ref()
            .unwrap()
            .matches_hash(selected.pack_hash().unwrap()));
        // Preserve crash-owned ref locks while collecting unpinned objects.
        let gc = tag_git(&root, &[
            "-c", "gc.packRefs=false", "-c", "gc.writeCommitGraph=false",
            "-c", "gc.reflogExpire=never", "-c", "gc.reflogExpireUnreachable=never",
            "gc", "--prune=now",
        ]);
        assert!(gc.status.success(), "{}", String::from_utf8_lossy(&gc.stderr));
        let reopened = Repository::open(&root).unwrap();
        assert!(reopened.odb().unwrap().exists(wanted));
        assert!(!reopened.odb().unwrap().exists(unwanted));
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let result = session
            .mutate(&snapshot.revision, Action::RecoverCheckout)
            .unwrap();
        assert!(!result.recovery_pending);
        assert_eq!(
            Repository::open(&root)
                .unwrap()
                .refname_to_id("refs/remotes/origin/main")
                .unwrap(),
            wanted
        );
        assert_eq!(preserved(&root), before);
    }
}
