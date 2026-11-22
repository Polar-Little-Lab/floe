//! Persistent virtual rebase resolutions; no live files change before final Continue.
use super::*;
const FILE: &str = ".git/branchdesk-rebase-draft.json";
const LIMIT: u64 = 20 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    version: u32,
    pub id: String,
    pub branch: String,
    pub head: String,
    pub(super) record: execution::Record,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub branch: String,
    pub head: String,
    pub upstream: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub branch: String,
    pub upstream: String,
    pub revision: String,
    pub pending: Option<execution::Pending>,
    pub result_oid: String,
    pub completed: bool,
    pub skipped: Vec<String>,
}
pub(in crate::repository) fn exists(root: &Path) -> bool {
    !matches!(fs::symlink_metadata(root.join(FILE)),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
}
fn bytes(root: &Path) -> Result<Option<Vec<u8>>> {
    if exists(root) {
        Ok(Some(bounded(&root.join(FILE), LIMIT)?))
    } else {
        Ok(None)
    }
}
fn read(root: &Path) -> Result<Option<Draft>> {
    let Some(bytes) = bytes(root)? else {
        return Ok(None);
    };
    let draft: Draft = serde_json::from_slice(&bytes).map_err(|_| "INVALID_REBASE_DRAFT")?;
    if draft.version != 1
        || Uuid::parse_str(&draft.id).is_err()
        || draft.branch.len() > 200
        || !git2::Reference::is_valid_name(&format!("refs/heads/{}", draft.branch))
        || draft.head.len() != 40
        || draft.record.resolutions.len() > replay_sequence::MAX_STEPS
        || draft.record.skipped.len() > replay_sequence::MAX_STEPS
    {
        return Err("INVALID_REBASE_DRAFT".into());
    }
    Oid::from_str(&draft.head).map_err(err)?;
    for values in draft.record.resolutions.values() {
        if values.len() > MAX_FILES {
            return Err("LIMIT: rebase resolutions".into());
        }
        for r in values.values() {
            replay_draft::validate_resolution(r)?;
        }
    }
    execution::identity(&draft.record)?;
    Ok(Some(draft))
}
pub(in crate::repository) fn summary(root: &Path) -> Result<Option<Summary>> {
    Ok(read(root)?.map(|d| Summary {
        id: d.id,
        branch: d.branch,
        head: d.head,
        upstream: d.record.upstream,
    }))
}
fn save(root: &Path, draft: &Draft, before: Option<&[u8]>) -> Result<()> {
    let value = serde_json::to_vec(draft).map_err(err)?;
    if value.len() as u64 > LIMIT {
        return Err("LIMIT: rebase draft".into());
    }
    if bytes(root)?.as_deref() != before {
        return Err("STALE_REBASE_DRAFT".into());
    }
    checkpoint(root, "save", 0);
    let mut temp = tempfile::Builder::new()
        .prefix("branchdesk-rebase-")
        .suffix(".next")
        .tempfile_in(root.join(".git"))
        .map_err(err)?;
    std::io::Write::write_all(&mut temp, &value).map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    checkpoint(root, "save", 1);
    if bytes(root)?.as_deref() != before {
        return Err("STALE_REBASE_DRAFT".into());
    }
    temp.persist(root.join(FILE)).map_err(err)?;
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "save", 2);
    Ok(())
}
fn pin_names(d: &Draft) -> [(String, String); 2] {
    [
        (
            format!("refs/branchdesk/checkpoints/rebase-{}-original", d.id),
            d.head.clone(),
        ),
        (
            format!("refs/branchdesk/checkpoints/rebase-{}-upstream", d.id),
            d.record.upstream_oid.clone(),
        ),
    ]
}
fn pin(root: &Path, d: &Draft) -> Result<()> {
    let repo = open(root)?;
    let mut tx = repo.transaction().map_err(err)?;
    for (name, id) in pin_names(d) {
        tx.lock_ref(&name).map_err(err)?;
        if repo.find_reference(&name).is_ok() {
            return Err("REBASE_PIN_EXISTS".into());
        }
        let oid = Oid::from_str(&id).map_err(err)?;
        repo.find_commit(oid).map_err(err)?;
        tx.set_target(&name, oid, None, "rebase: preserve draft inputs")
            .map_err(err)?;
    }
    tx.commit().map_err(err)
}
fn state(
    session: &mut Session,
    revision: &str,
) -> Result<(Draft, execution::Calculation, Snapshot)> {
    let snap = session.fresh(revision)?;
    let d = read(&session.root)?.ok_or("NO_REBASE_DRAFT")?;
    if snap.branch != d.branch {
        return Err("STALE_REBASE_BRANCH".into());
    }
    let repo = open(&session.root)?;
    for (name, id) in pin_names(&d) {
        if repo.refname_to_id(&name).map_err(err)?.to_string() != id {
            return Err("STALE_REBASE_PIN".into());
        }
    }
    let calc = execution::calculate(&repo, Oid::from_str(&d.head).map_err(err)?, &d.record)?;
    if snap.head_oid.as_deref() != Some(&d.head)
        && !(calc.pending.is_none()
            && snap.head_oid.as_deref() == Some(calc.oid.to_string().as_str()))
    {
        return Err("STALE_REBASE_HEAD".into());
    }
    Ok((d, calc, snap))
}
impl Session {
    pub fn rebase_draft_preview(&mut self, revision: &str) -> Result<View> {
        let (d, calc, snap) = state(self, revision)?;
        let completed = calc.pending.is_none()
            && calc.oid.to_string() != d.head
            && snap.head_oid.as_deref() == Some(calc.oid.to_string().as_str());
        Ok(View {
            id: d.id,
            branch: d.branch,
            upstream: d.record.upstream,
            revision: revision.into(),
            pending: calc.pending,
            result_oid: calc.oid.to_string(),
            completed,
            skipped: d.record.skipped,
        })
    }
}
pub(in crate::repository) fn start(
    session: &mut Session,
    revision: &str,
    upstream: &str,
    name: &str,
    email: &str,
) -> Result<()> {
    if exists(&session.root) {
        return Err("REBASE_DRAFT_EXISTS".into());
    }
    let plan = session.rebase_preview(revision, upstream)?;
    if plan.blockers.iter().any(|b| b != "CONFLICTS") {
        return Err("REBASE_BLOCKED".into());
    }
    let time = git2::Signature::now("FLOE", "local@floe.invalid")
        .map_err(err)?
        .when();
    let d = Draft {
        version: 1,
        id: Uuid::new_v4().to_string(),
        branch: plan.branch,
        head: plan.head_oid,
        record: execution::Record {
            upstream: upstream.into(),
            upstream_oid: plan.upstream_oid,
            name: name.into(),
            email: email.into(),
            seconds: time.seconds(),
            offset: time.offset_minutes(),
            resolutions: HashMap::new(),
            skipped: vec![],
        },
    };
    execution::identity(&d.record)?;
    session.fresh(revision)?;
    pin(&session.root, &d)?;
    let after = session.snapshot()?;
    if after.branch != d.branch
        || after.head_oid.as_deref() != Some(&d.head)
        || !after.changes.is_empty()
        || after.write_block.is_some()
    {
        return Err("STALE_REBASE_START".into());
    }
    save(&session.root, &d, None)
}
pub(in crate::repository) fn resolve(
    session: &mut Session,
    revision: &str,
    id: &str,
    source: &str,
    file: &str,
    resolution: merge_draft::Resolution,
) -> Result<()> {
    let before = bytes(&session.root)?.ok_or("NO_REBASE_DRAFT")?;
    let (mut d, calc, snap) = state(session, revision)?;
    if snap.head_oid.as_deref() != Some(&d.head) || snap.write_block.is_some() {
        return Err("REBASE_NOT_EDITABLE".into());
    }
    let pending = calc.pending.ok_or("REBASE_ALREADY_RESOLVED")?;
    if d.id != id || pending.source != source || !pending.conflicts.iter().any(|c| c.id == file) {
        return Err("STALE_REBASE_CONFLICT".into());
    }
    replay_draft::validate_resolution(&resolution)?;
    d.record
        .resolutions
        .entry(source.into())
        .or_default()
        .insert(file.into(), resolution);
    execution::calculate(
        &open(&session.root)?,
        Oid::from_str(&d.head).map_err(err)?,
        &d.record,
    )?;
    session.fresh(revision)?;
    save(&session.root, &d, Some(&before))
}
pub(in crate::repository) fn skip(
    session: &mut Session,
    revision: &str,
    id: &str,
    source: &str,
) -> Result<()> {
    let before = bytes(&session.root)?.ok_or("NO_REBASE_DRAFT")?;
    let (mut d, calc, snap) = state(session, revision)?;
    if snap.head_oid.as_deref() != Some(&d.head) || snap.write_block.is_some() {
        return Err("REBASE_NOT_EDITABLE".into());
    }
    if d.id != id || calc.pending.as_ref().map(|p| p.source.as_str()) != Some(source) {
        return Err("STALE_REBASE_CONFLICT".into());
    }
    d.record.resolutions.remove(source);
    d.record.skipped.push(source.into());
    execution::calculate(
        &open(&session.root)?,
        Oid::from_str(&d.head).map_err(err)?,
        &d.record,
    )?;
    session.fresh(revision)?;
    save(&session.root, &d, Some(&before))
}
fn archive(root: &Path) -> Result<()> {
    checkpoint(root, "archive", 0);
    fs::rename(
        root.join(FILE),
        root.join(format!("{FILE}-archived-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "archive", 1);
    Ok(())
}
pub(in crate::repository) fn abort(session: &mut Session, revision: &str) -> Result<()> {
    let snap = session.fresh(revision)?;
    if snap.recovery_pending {
        return Err("RECOVERY_REQUIRED".into());
    }
    if !exists(&session.root) {
        return Err("NO_REBASE_DRAFT".into());
    }
    // Archiving a malformed draft discards only draft metadata, never Git data.
    if read(&session.root).is_ok() {
        let (d, calc, snap) = state(session, revision)?;
        if snap.head_oid.as_deref() != Some(&d.head) && calc.oid.to_string() != d.head {
            return Err("REBASE_ALREADY_APPLIED".into());
        }
    }
    archive(&session.root)
}
pub(in crate::repository) fn finish(session: &mut Session, revision: &str) -> Result<()> {
    let (d, calc, snap) = state(session, revision)?;
    if snap.recovery_pending {
        return Err("RECOVERY_REQUIRED".into());
    }
    if calc.pending.is_some() {
        return Err("UNRESOLVED_REBASE_CONFLICTS".into());
    }
    if snap.head_oid.as_deref() == Some(calc.oid.to_string().as_str()) {
        return archive(&session.root);
    }
    if snap.write_block.is_some() || !snap.changes.is_empty() {
        return Err("DIRTY_REBASE_CONTINUE".into());
    }
    let repo = open(&session.root)?;
    let mut index = IndexLock::acquire(&session.root)?;
    let prepared = execution::Prepared {
        head: Oid::from_str(&d.head).map_err(err)?,
        oid: calc.oid,
        objects: calc.objects,
        record: d.record,
    };
    checkout::rebase(session, &repo, revision, &prepared, &mut index)?;
    drop(index);
    checkpoint(&session.root, "applied", 0);
    archive(&session.root)
}
fn checkpoint(root: &Path, kind: &str, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_REBASE_DRAFT_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_REBASE_DRAFT_STEP")
            .ok()
            .as_deref()
            == Some(format!("{kind}:{step}").as_str())
    {
        fs::write(root.join(".git/rebase-draft-ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, kind, step);
}
