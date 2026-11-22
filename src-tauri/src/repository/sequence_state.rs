//! Durable sequence progress and journaled step execution.
//! Caller must hold the repository lease. Publication never writes HEAD/index/worktree.
use super::*;
const FILE: &str = ".git/branchdesk-sequence.json";
const LIMIT: u64 = 256 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    message: String,
    name: String,
    email: String,
    seconds: i64,
    offset: i32,
    allow_empty: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    ordinal: usize,
    before: String,
    after: String,
    #[serde(default)]
    intent: Option<Intent>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Completion {
    Applied { before: String, after: String },
    Skipped { head: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    id: String,
    branch: String,
    original_head: String,
    expected_head: String,
    selections: Vec<ReplaySelection>,
    completed: Vec<Completion>,
    pending: Option<Pending>,
    #[serde(default)]
    aborting: bool,
    #[serde(default)]
    resolutions: HashMap<String, merge_draft::Resolution>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub branch: String,
    pub original_head: String,
    pub expected_head: String,
    pub total: usize,
    pub completed: usize,
    pub pending: bool,
    pub aborting: bool,
    pub current: Option<ReplaySelection>,
    pub resolutions: HashMap<String, merge_draft::Resolution>,
}
pub(super) fn exists(root: &Path) -> bool {
    !matches!(fs::symlink_metadata(root.join(FILE)), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}
pub(super) fn progress(root: &Path) -> Result<Option<Progress>> {
    Ok(read(root)?.map(|record| Progress {
        id: record.id,
        branch: record.branch,
        original_head: record.original_head,
        expected_head: record.expected_head,
        total: record.selections.len(),
        completed: record.completed.len(),
        pending: record.pending.is_some(),
        aborting: record.aborting,
        current: record.selections.get(record.completed.len()).cloned(),
        resolutions: record.resolutions,
    }))
}
// A started sequence preserves its selected objects even if their visible branch
// disappears or they leave the bounded graph. Authorize only the current step,
// with its exact operation/mainline and unchanged branch/expected HEAD.
pub(super) fn authorizes_current(
    root: &Path,
    snap: &Snapshot,
    oid: &str,
    kind: ReplayKind,
    mainline: Option<usize>,
) -> Result<bool> {
    let Some(record) = read(root)? else {
        return Ok(false);
    };
    let Some(selection) = record.selections.get(record.completed.len()) else {
        return Ok(false);
    };
    if snap.branch != record.branch
        || snap.head_oid.as_deref() != Some(&record.expected_head)
        || selection.oid != oid
        || selection.kind != kind
        || selection.mainline != mainline
    {
        return Ok(false);
    }
    verify_pins(&open(root)?, &record)?;
    Ok(true)
}
fn oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
impl Record {
    fn validate(&self) -> Result<()> {
        if self.version != 1
            || Uuid::parse_str(&self.id).is_err()
            || self.branch.len() > 200
            || !git2::Reference::is_valid_name(&format!("refs/heads/{}", self.branch))
            || !oid(&self.original_head)
            || !oid(&self.expected_head)
            || self.selections.is_empty()
            || self.selections.len() > replay_sequence::MAX_STEPS
            || self.completed.len() > self.selections.len()
        {
            return Err("INVALID_SEQUENCE_STATE".into());
        }
        if self.resolutions.len() > MAX_FILES {
            return Err("LIMIT: sequence resolutions".into());
        }
        for resolution in self.resolutions.values() {
            replay_draft::validate_resolution(resolution)?;
        }
        for step in &self.selections {
            if !oid(&step.oid) || step.mainline == Some(0) {
                return Err("INVALID_SEQUENCE_SELECTION".into());
            }
        }
        let mut head = &self.original_head;
        for completion in &self.completed {
            match completion {
                Completion::Applied { before, after } => {
                    if before != head || !oid(after) || after == before {
                        return Err("INVALID_SEQUENCE_CHAIN".into());
                    }
                    head = after;
                }
                Completion::Skipped { head: skipped } => {
                    if skipped != head {
                        return Err("INVALID_SEQUENCE_CHAIN".into());
                    }
                }
            }
        }
        if head != &self.expected_head {
            return Err("INVALID_SEQUENCE_CHAIN".into());
        }
        if let Some(p) = &self.pending {
            if let Some(intent) = &p.intent {
                intent.validate()?;
            }
            if p.ordinal != self.completed.len()
                || p.ordinal >= self.selections.len()
                || p.before != self.expected_head
                || !oid(&p.after)
                || p.after == p.before
            {
                return Err("INVALID_SEQUENCE_PENDING".into());
            }
        }
        Ok(())
    }
    fn prepare(&mut self, expected_after: &str) -> Result<()> {
        self.validate()?;
        if self.pending.is_some()
            || self.completed.len() == self.selections.len()
            || !oid(expected_after)
            || expected_after == self.expected_head
        {
            return Err("SEQUENCE_NOT_READY".into());
        }
        self.pending = Some(Pending {
            ordinal: self.completed.len(),
            before: self.expected_head.clone(),
            after: expected_after.into(),
            intent: None,
        });
        Ok(())
    }
    // A future execution adapter must verify the recorded commit against source/resolutions
    // before calling this. Matching an OID alone is not proof of a valid replay.
    fn acknowledge(&mut self, observed_head: &str) -> Result<()> {
        self.validate()?;
        let pending = self.pending.as_ref().ok_or("NO_SEQUENCE_PENDING_STEP")?;
        if observed_head != pending.after {
            return Err("STALE_SEQUENCE_HEAD".into());
        }
        let pending = self.pending.take().unwrap();
        self.expected_head = pending.after.clone();
        self.resolutions.clear();
        self.completed.push(Completion::Applied {
            before: pending.before,
            after: pending.after,
        });
        Ok(())
    }
    fn skip(&mut self, observed_head: &str) -> Result<()> {
        self.validate()?;
        if self.pending.is_some() || self.completed.len() == self.selections.len() {
            return Err("SEQUENCE_NOT_READY".into());
        }
        if observed_head != self.expected_head {
            return Err("STALE_SEQUENCE_HEAD".into());
        }
        self.resolutions.clear();
        self.completed.push(Completion::Skipped {
            head: self.expected_head.clone(),
        });
        Ok(())
    }
}
impl Intent {
    fn validate(&self) -> Result<()> {
        if self.message.trim().is_empty()
            || self.message.len() > 10000
            || self.message.contains('\0')
            || self.name.trim().is_empty()
            || self.name.len() > 256
            || self.email.trim().is_empty()
            || self.email.len() > 256
            || self.offset.unsigned_abs() > 1439
        {
            return Err("INVALID_SEQUENCE_INTENT".into());
        }
        git2::Signature::new(&self.name, &self.email, &self.time()).map_err(err)?;
        Ok(())
    }
    fn time(&self) -> git2::Time {
        git2::Time::new(self.seconds, self.offset)
    }
}
fn pin_names(record: &Record) -> Vec<(String, String)> {
    std::iter::once((
        format!(
            "refs/branchdesk/checkpoints/sequence-{}-original",
            record.id
        ),
        record.original_head.clone(),
    ))
    .chain(record.selections.iter().enumerate().map(|(i, s)| {
        (
            format!(
                "refs/branchdesk/checkpoints/sequence-{}-source-{i}",
                record.id
            ),
            s.oid.clone(),
        )
    }))
    .collect()
}
fn pin(repo: &Repository, record: &Record) -> Result<()> {
    let pins = pin_names(record);
    let mut tx = repo.transaction().map_err(err)?;
    for (name, _) in &pins {
        tx.lock_ref(name).map_err(err)?;
        if repo.find_reference(name).is_ok() {
            return Err("SEQUENCE_PIN_EXISTS".into());
        }
    }
    for (name, oid) in &pins {
        let oid = Oid::from_str(oid).map_err(err)?;
        repo.find_commit(oid).map_err(err)?;
        tx.set_target(name, oid, None, "sequence: preserve input")
            .map_err(err)?;
    }
    tx.commit().map_err(err)
}
fn verify_pins(repo: &Repository, record: &Record) -> Result<()> {
    for (name, id) in pin_names(record) {
        if repo.refname_to_id(&name).map_err(err)?.to_string() != id {
            return Err("STALE_SEQUENCE_PIN".into());
        }
    }
    Ok(())
}
fn prepare_next(session: &mut Session, revision: &str, intent: Intent) -> Result<()> {
    intent.validate()?;
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let mut record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    if record.aborting {
        return Err("SEQUENCE_ABORT_IN_PROGRESS".into());
    }
    let snap = session.fresh(revision)?;
    if snap.branch != record.branch || snap.head_oid.as_deref() != Some(&record.expected_head) {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    if record.pending.is_some() || record.completed.len() >= record.selections.len() {
        return Err("SEQUENCE_NOT_READY".into());
    }
    if checkout::pending(&session.root) {
        return Err("RECOVERY_REQUIRED".into());
    }
    verify_pins(&open(&session.root)?, &record)?;
    let selection = &record.selections[record.completed.len()];
    let prepared = replay::prepare_resolved(
        session,
        revision,
        &selection.oid,
        selection.kind,
        selection.mainline,
        (&intent.message, &intent.name, &intent.email),
        replay::PrepareOptions {
            allow_empty: intent.allow_empty,
            committer_time: Some(intent.time()),
            resolutions: (!record.resolutions.is_empty()).then_some(&record.resolutions),
        },
    )?;
    record.prepare(&prepared.oid.to_string())?;
    record.pending.as_mut().unwrap().intent = Some(intent);
    session.fresh(revision)?;
    publish(&session.root, &record, Some(&before))
}
fn apply_pending(session: &mut Session, revision: &str) -> Result<()> {
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let mut record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    if record.aborting {
        return Err("SEQUENCE_ABORT_IN_PROGRESS".into());
    }
    let pending = record.pending.clone().ok_or("NO_SEQUENCE_PENDING_STEP")?;
    let intent = pending.intent.as_ref().ok_or("MISSING_SEQUENCE_INTENT")?;
    let snap = session.fresh(revision)?;
    if snap.branch != record.branch {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    if checkout::pending(&session.root) {
        return Err("RECOVERY_REQUIRED".into());
    }
    let repo = open(&session.root)?;
    verify_pins(&repo, &record)?;
    let selection = &record.selections[pending.ordinal];
    if snap.head_oid.as_deref() == Some(&pending.before) {
        let mut lock = IndexLock::acquire(&session.root)?;
        checkout::replay_commit(
            session,
            &repo,
            revision,
            &checkout::ReplayInput {
                committer_time: Some(intent.time()),
                expected_oid: Some(Oid::from_str(&pending.after).map_err(err)?),
                allow_empty: intent.allow_empty,
                complete_draft: false,
                resolutions: (!record.resolutions.is_empty()).then_some(&record.resolutions),
                oid: &selection.oid,
                kind: selection.kind,
                mainline: selection.mainline,
                message: &intent.message,
                author: &intent.name,
                email: &intent.email,
            },
            &mut lock,
            None,
        )?;
        drop(lock);
    } else if snap.head_oid.as_deref() != Some(&pending.after) {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    checkpoint(&session.root, 10);
    let repo = open(&session.root)?;
    if repo
        .head()
        .map_err(err)?
        .target()
        .map(|id| id.to_string())
        .as_deref()
        != Some(&pending.after)
    {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    replay::verify_commit(
        &repo,
        Oid::from_str(&pending.before).map_err(err)?,
        Oid::from_str(&pending.after).map_err(err)?,
        &replay::Record {
            kind: selection.kind,
            source: selection.oid.clone(),
            mainline: selection.mainline,
            resolutions: (!record.resolutions.is_empty()).then_some(record.resolutions.clone()),
        },
    )?;
    let commit = repo
        .find_commit(Oid::from_str(&pending.after).map_err(err)?)
        .map_err(err)?;
    let signature = commit.committer();
    let message = git2::message_prettify(&intent.message, None).map_err(err)?;
    if signature.name_bytes() != intent.name.as_bytes()
        || signature.email_bytes() != intent.email.as_bytes()
        || signature.when() != intent.time()
        || commit.message_bytes() != message.as_bytes()
    {
        return Err("INVALID_SEQUENCE_COMMIT".into());
    }
    if selection.kind == ReplayKind::Revert
        && (commit.author().name_bytes() != intent.name.as_bytes()
            || commit.author().email_bytes() != intent.email.as_bytes()
            || commit.author().when() != intent.time())
    {
        return Err("INVALID_SEQUENCE_AUTHOR".into());
    }
    record.acknowledge(&pending.after)?;
    publish(&session.root, &record, Some(&before))
}

fn bytes(root: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(root.join(FILE)) {
        Ok(_) => Ok(Some(bounded(&root.join(FILE), LIMIT)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
fn read(root: &Path) -> Result<Option<Record>> {
    bytes(root)?
        .map(|bytes| {
            let record: Record = serde_json::from_slice(&bytes).map_err(err)?;
            record.validate()?;
            Ok(record)
        })
        .transpose()
}
fn publish(root: &Path, record: &Record, before: Option<&[u8]>) -> Result<()> {
    record.validate()?;
    let data = serde_json::to_vec(record).map_err(err)?;
    if data.len() as u64 > LIMIT {
        return Err("LIMIT: sequence state".into());
    }
    if bytes(root)?.as_deref() != before {
        return Err("STALE_SEQUENCE_STATE".into());
    }
    checkpoint(root, 0);
    let mut temp = tempfile::Builder::new()
        .prefix("branchdesk-sequence-")
        .suffix(".next")
        .tempfile_in(root.join(".git"))
        .map_err(err)?;
    std::io::Write::write_all(&mut temp, &data).map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    checkpoint(root, 1);
    if bytes(root)?.as_deref() != before {
        return Err("STALE_SEQUENCE_STATE".into());
    }
    temp.persist(root.join(FILE)).map_err(err)?;
    checkpoint(root, 2);
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, 3);
    Ok(())
}
fn begin(
    session: &mut Session,
    revision: &str,
    selections: Vec<ReplaySelection>,
) -> Result<Record> {
    if bytes(&session.root)?.is_some() {
        return Err("SEQUENCE_EXISTS".into());
    }
    let plan = replay_sequence::preview(session, revision, &selections)?;
    if plan.blockers.iter().any(|b| b != "CONFLICTS") {
        return Err("SEQUENCE_BLOCKED".into());
    }
    let snap = session.fresh(revision)?;
    let record = Record {
        version: 1,
        id: Uuid::new_v4().to_string(),
        branch: snap.branch,
        original_head: plan.head.clone(),
        expected_head: plan.head,
        selections,
        completed: Vec::new(),
        pending: None,
        aborting: false,
        resolutions: HashMap::new(),
    };
    pin(&open(&session.root)?, &record)?;
    // Our checkpoint refs change the revision. Validate live branch/head/clean
    // state after pinning rather than comparing against the pre-pin revision.
    let after = session.snapshot()?;
    if after.branch != record.branch
        || after.head_oid.as_deref() != Some(&record.original_head)
        || !after.changes.is_empty()
        || after.write_block.is_some()
        || open(&session.root)?.state() != git2::RepositoryState::Clean
    {
        return Err("STALE_SEQUENCE_START".into());
    }
    publish(&session.root, &record, None)?;
    Ok(record)
}
pub(super) fn start(
    session: &mut Session,
    revision: &str,
    selections: Vec<ReplaySelection>,
) -> Result<()> {
    begin(session, revision, selections)?;
    Ok(())
}
pub(super) fn advance(
    session: &mut Session,
    revision: &str,
    message: String,
    name: String,
    email: String,
    allow_empty: bool,
) -> Result<()> {
    session.fresh(revision)?;
    let record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    if record.aborting {
        return Err("SEQUENCE_ABORT_IN_PROGRESS".into());
    }
    if record.completed.len() == record.selections.len() {
        return Err("SEQUENCE_COMPLETE".into());
    }
    if record.pending.is_none() {
        let now = git2::Signature::now(&name, &email).map_err(err)?.when();
        prepare_next(
            session,
            revision,
            Intent {
                message,
                name,
                email,
                allow_empty,
                seconds: now.seconds(),
                offset: now.offset_minutes(),
            },
        )?;
    }
    // Publishing the frozen intent changes the snapshot revision.
    let fresh = session.snapshot()?;
    apply_pending(session, &fresh.revision)
}
pub(super) fn save_resolution(
    session: &mut Session,
    revision: &str,
    sequence_id: &str,
    ordinal: usize,
    file_id: &str,
    resolution: merge_draft::Resolution,
) -> Result<()> {
    replay_draft::validate_resolution(&resolution)?;
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let mut record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    if record.aborting {
        return Err("SEQUENCE_ABORT_IN_PROGRESS".into());
    }
    if record.id != sequence_id || record.completed.len() != ordinal {
        return Err("STALE_SEQUENCE_STEP".into());
    }
    if record.pending.is_some() {
        return Err("SEQUENCE_INTENT_FROZEN".into());
    }
    let selection = record.selections.get(ordinal).ok_or("SEQUENCE_COMPLETE")?;
    let snap = session.fresh(revision)?;
    if snap.branch != record.branch || snap.head_oid.as_deref() != Some(&record.expected_head) {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    if checkout::pending(&session.root) || snap.write_block.is_some() {
        return Err("RECOVERY_REQUIRED".into());
    }
    verify_pins(&open(&session.root)?, &record)?;
    let plan = replay::preview(
        session,
        revision,
        &selection.oid,
        selection.kind,
        selection.mainline,
    )?;
    if plan.blockers.iter().any(|b| b != "CONFLICTS") {
        return Err("SEQUENCE_BLOCKED".into());
    }
    let conflict = plan
        .conflicts
        .iter()
        .find(|c| {
            [&c.ancestor, &c.ours, &c.theirs]
                .into_iter()
                .flatten()
                .any(|v| v.id == file_id)
        })
        .ok_or("INVALID_REPLAY_CONFLICT")?;
    let paths: std::collections::HashSet<_> =
        [&conflict.ancestor, &conflict.ours, &conflict.theirs]
            .into_iter()
            .flatten()
            .map(|v| &v.path)
            .collect();
    if paths.len() != 1 {
        return Err("UNSUPPORTED_RENAME_CONFLICT".into());
    }
    record.resolutions.insert(file_id.into(), resolution);
    session.fresh(revision)?;
    publish(&session.root, &record, Some(&before))
}
pub(super) fn skip_step(
    session: &mut Session,
    revision: &str,
    sequence_id: &str,
    ordinal: usize,
) -> Result<()> {
    let snap = session.fresh(revision)?;
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let mut record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    if record.aborting {
        return Err("SEQUENCE_ABORT_IN_PROGRESS".into());
    }
    if record.id != sequence_id || record.completed.len() != ordinal {
        return Err("STALE_SEQUENCE_STEP".into());
    }
    if checkout::pending(&session.root) {
        return Err("RECOVERY_REQUIRED".into());
    }
    if snap.branch != record.branch || snap.head_oid.as_deref() != Some(&record.expected_head) {
        return Err("STALE_SEQUENCE_HEAD: resume any already applied step first".into());
    }
    if !snap.changes.is_empty() || snap.write_block.is_some() {
        return Err("SEQUENCE_BLOCKED".into());
    }
    verify_pins(&open(&session.root)?, &record)?;
    // A prepared but unpublished candidate can be discarded. Published commits
    // must be acknowledged through resume; the expected HEAD check above refuses them.
    record.pending = None;
    record.skip(&record.expected_head.clone())?;
    session.fresh(revision)?;
    publish(&session.root, &record, Some(&before))
}
pub(super) fn verify_abort_journal(
    root: &Path,
    repo: &Repository,
    branch: &str,
    old: &str,
    original: &str,
) -> Result<()> {
    let record = read(root)?.ok_or("NO_SEQUENCE")?;
    if !record.aborting
        || branch != format!("refs/heads/{}", record.branch)
        || old != record.expected_head
        || original != record.original_head
    {
        return Err("INVALID_SEQUENCE_REWIND_JOURNAL".into());
    }
    verify_pins(repo, &record)
}
pub(super) fn abort(session: &mut Session, revision: &str) -> Result<()> {
    let snap = session.fresh(revision)?;
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let mut record = read(&session.root)?.ok_or("NO_SEQUENCE")?;
    let repo = open(&session.root)?;
    verify_pins(&repo, &record)?;
    if checkout::pending(&session.root) {
        return Err("RECOVERY_REQUIRED".into());
    }
    if snap.branch != record.branch {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    if record.aborting && snap.head_oid.as_deref() == Some(&record.original_head) {
        return stop(session, revision);
    }
    if snap.head_oid.as_deref() != Some(&record.expected_head) {
        return Err("STALE_SEQUENCE_HEAD: resume the applied step before abort".into());
    }
    if !snap.changes.is_empty() || snap.write_block.is_some() {
        return Err("SEQUENCE_BLOCKED".into());
    }
    // Check the complete applied chain before rewinding; skipped steps add no commits.
    for completion in &record.completed {
        if let Completion::Applied { before, after } = completion {
            let commit = repo
                .find_commit(Oid::from_str(after).map_err(err)?)
                .map_err(err)?;
            if commit.parent_count() != 1
                || commit.parent_id(0).map_err(err)?.to_string() != *before
            {
                return Err("INVALID_SEQUENCE_CHAIN".into());
            }
        }
    }
    if !record.aborting {
        record.aborting = true;
        publish(&session.root, &record, Some(&before))?;
    }
    let fresh = session.snapshot()?;
    if record.expected_head != record.original_head {
        let mut lock = IndexLock::acquire(&session.root)?;
        checkout::rewind_sequence(
            session,
            &repo,
            &fresh.revision,
            &record.original_head,
            &mut lock,
        )?;
    }
    checkpoint(&session.root, 30);
    let fresh = session.snapshot()?;
    if fresh.head_oid.as_deref() != Some(&record.original_head) || fresh.branch != record.branch {
        return Err("STALE_SEQUENCE_HEAD".into());
    }
    stop(session, &fresh.revision)
}
pub(super) fn stop(session: &mut Session, revision: &str) -> Result<()> {
    session.fresh(revision)?;
    if checkout::pending(&session.root) {
        return Err("RECOVERY_REQUIRED".into());
    }
    let before = bytes(&session.root)?.ok_or("NO_SEQUENCE")?;
    let destination = session.root.join(format!(
        ".git/branchdesk-sequence-stopped-{}.json",
        Uuid::new_v4()
    ));
    session.fresh(revision)?;
    if bytes(&session.root)?.as_deref() != Some(before.as_slice()) {
        return Err("STALE_SEQUENCE_STATE".into());
    }
    checkpoint(&session.root, 20);
    fs::rename(session.root.join(FILE), destination).map_err(err)?;
    checkpoint(&session.root, 21);
    checkout::sync_dir(&session.root.join(".git"))?;
    checkpoint(&session.root, 22);
    Ok(())
}
#[cfg(not(test))]
fn checkpoint(_root: &Path, _step: usize) {}
#[cfg(test)]
fn checkpoint(root: &Path, step: usize) {
    if std::env::var_os("BRANCHDESK_SEQUENCE_STATE_ROOT").as_deref() != Some(root.as_os_str())
        || std::env::var("BRANCHDESK_SEQUENCE_STATE_STEP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            != Some(step)
    {
        return;
    }
    fs::write(root.join(".git/sequence-state.ready"), step.to_string()).unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Vec<ReplaySelection>) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let tree = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree).unwrap();
        let sig = git2::Signature::now("Sequence", "sequence@example.invalid").unwrap();
        let base = repo
            .commit(Some("HEAD"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        let mut builder = repo.treebuilder(Some(&tree)).unwrap();
        builder
            .insert("a.txt", repo.blob(b"first\n").unwrap(), 0o100644)
            .unwrap();
        let first_tree = repo.find_tree(builder.write().unwrap()).unwrap();
        builder
            .insert("b.txt", repo.blob(b"second\n").unwrap(), 0o100644)
            .unwrap();
        let second_tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let first = repo
            .commit(
                Some("refs/heads/topic"),
                &sig,
                &sig,
                "one",
                &first_tree,
                &[&repo.find_commit(base).unwrap()],
            )
            .unwrap();
        let second = repo
            .commit(
                Some("refs/heads/topic"),
                &sig,
                &sig,
                "two",
                &second_tree,
                &[&repo.find_commit(first).unwrap()],
            )
            .unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        (
            d,
            vec![first, second]
                .into_iter()
                .map(|id| ReplaySelection {
                    oid: id.to_string(),
                    kind: ReplayKind::CherryPick,
                    mainline: None,
                })
                .collect(),
        )
    }
    fn conflict_fixture(kind: ReplayKind) -> (tempfile::TempDir, Vec<ReplaySelection>) {
        let (d, mut selections) = fixture();
        let source = selections[0].oid.clone();
        configure_execution(d.path(), &mut selections, kind);
        let repo = open(d.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let mut builder = repo.treebuilder(Some(&head.tree().unwrap())).unwrap();
        builder
            .insert("a.txt", repo.blob(b"ours\n").unwrap(), 0o100644)
            .unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = git2::Signature::now("Conflict", "conflict@invalid").unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "conflicting main",
            &tree,
            &[&head],
        )
        .unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        (
            d,
            vec![ReplaySelection {
                oid: source,
                kind,
                mainline: None,
            }],
        )
    }
    #[test]
    fn public_sequence_conflicts_persist_restart_and_apply_all_resolution_kinds() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for choice in 0..4 {
                let (d, selections) = conflict_fixture(kind);
                let selection = selections[0].clone();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let started = session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                let id = started.sequence_progress.as_ref().unwrap().id.clone();
                let index = fs::read(root.join(".git/index")).unwrap();
                let plan = session
                    .replay_preview(&started.revision, &selection.oid, kind, None)
                    .unwrap();
                assert_eq!(plan.conflicts.len(), 1);
                let c = &plan.conflicts[0];
                let file_id = c
                    .ours
                    .as_ref()
                    .or(c.theirs.as_ref())
                    .or(c.ancestor.as_ref())
                    .unwrap()
                    .id
                    .clone();
                assert!(session
                    .mutate(
                        &started.revision,
                        Action::ContinueSequence {
                            message: "resolve".into(),
                            author: "Test".into(),
                            email: "test@invalid".into(),
                            allow_empty: true
                        }
                    )
                    .is_err());
                let resolution = match choice {
                    0 => merge_draft::Resolution::Ours {},
                    1 => merge_draft::Resolution::Theirs {},
                    2 => merge_draft::Resolution::Delete {},
                    _ => merge_draft::Resolution::Text {
                        text: "resolved\n".into(),
                    },
                };
                assert_eq!(
                    session
                        .mutate(
                            &started.revision,
                            Action::ResolveSequence {
                                sequence_id: id.clone(),
                                ordinal: 0,
                                file_id: "wrong".into(),
                                resolution: resolution.clone()
                            }
                        )
                        .err()
                        .unwrap(),
                    "INVALID_REPLAY_CONFLICT"
                );
                let saved = session
                    .mutate(
                        &started.revision,
                        Action::ResolveSequence {
                            sequence_id: id,
                            ordinal: 0,
                            file_id,
                            resolution,
                        },
                    )
                    .unwrap();
                assert_eq!(
                    saved.sequence_progress.as_ref().unwrap().resolutions.len(),
                    1
                );
                assert_eq!(saved.head_oid, started.head_oid);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"ours\n");
                drop(session);
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let done = session
                    .mutate(
                        &snap.revision,
                        Action::ContinueSequence {
                            message: "resolve".into(),
                            author: "Test".into(),
                            email: "test@invalid".into(),
                            allow_empty: true,
                        },
                    )
                    .unwrap();
                assert!(done.changes.is_empty());
                assert_eq!(done.sequence_progress.as_ref().unwrap().completed, 1);
                assert!(done
                    .sequence_progress
                    .as_ref()
                    .unwrap()
                    .resolutions
                    .is_empty());
                let expected = match choice {
                    0 => Some(b"ours\n".to_vec()),
                    1 if kind == ReplayKind::CherryPick => Some(b"first\n".to_vec()),
                    3 => Some(b"resolved\n".to_vec()),
                    _ => None,
                };
                assert_eq!(fs::read(root.join("a.txt")).ok(), expected);
                assert!(!root.join(".git/branchdesk-replay-completion.json").exists());
                assert!(!checkout::pending(&root));
                let repo = open(&root).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(
                    commit.parent_id(0).unwrap().to_string(),
                    started.head_oid.unwrap()
                );
            }
        }
    }
    #[test]
    fn resolved_sequence_checkout_kills_retain_saved_resolution_and_retry_exact_oid() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in [1, 3] {
                let (d, selections) = conflict_fixture(kind);
                let selection = selections[0].clone();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let started = session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                let plan = session
                    .replay_preview(&started.revision, &selection.oid, kind, None)
                    .unwrap();
                let c = &plan.conflicts[0];
                let file_id = c
                    .ours
                    .as_ref()
                    .or(c.theirs.as_ref())
                    .or(c.ancestor.as_ref())
                    .unwrap()
                    .id
                    .clone();
                let saved = session
                    .mutate(
                        &started.revision,
                        Action::ResolveSequence {
                            sequence_id: started.sequence_progress.as_ref().unwrap().id.clone(),
                            ordinal: 0,
                            file_id,
                            resolution: merge_draft::Resolution::Text {
                                text: "resolved\n".into(),
                            },
                        },
                    )
                    .unwrap();
                {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    prepare_next(&mut session, &saved.revision, intent()).unwrap();
                }
                let pending = read(&root).unwrap().unwrap().pending.unwrap();
                let state = bytes(&root).unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                kill_checkout(&root, point, false);
                if point == 3 {
                    kill_checkout(&root, 1, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let recovered = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(bytes(&root).unwrap(), state);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"ours\n");
                let done = session
                    .mutate(
                        &recovered.revision,
                        Action::ContinueSequence {
                            message: String::new(),
                            author: String::new(),
                            email: String::new(),
                            allow_empty: false,
                        },
                    )
                    .unwrap();
                assert_eq!(done.head_oid.as_deref(), Some(pending.after.as_str()));
                assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"resolved\n");
                assert_eq!(done.sequence_progress.unwrap().completed, 1);
                assert!(!checkout::pending(&root));
            }
        }
    }
    fn abort_fixture(kind: ReplayKind, count: usize) -> (tempfile::TempDir, String) {
        let (d, mut selections) = fixture();
        for selection in &mut selections {
            selection.kind = kind;
        }
        configure_execution(d.path(), &mut selections, kind);
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root).unwrap();
        let snap = session.snapshot().unwrap();
        let original = snap.head_oid.clone().unwrap();
        let mut current = session
            .mutate(&snap.revision, Action::StartSequence { selections })
            .unwrap();
        for step in 0..count {
            current = session
                .mutate(
                    &current.revision,
                    Action::ContinueSequence {
                        message: format!("step {step}"),
                        author: "Abort Test".into(),
                        email: "abort@invalid".into(),
                        allow_empty: false,
                    },
                )
                .unwrap();
        }
        (d, original)
    }
    #[test]
    fn public_abort_restores_original_tree_after_zero_one_or_two_steps_both_kinds() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for count in 0..=2 {
                let (d, original) = abort_fixture(kind, count);
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let applied = snap.head_oid.clone().unwrap();
                let done = session
                    .mutate(&snap.revision, Action::AbortSequence)
                    .unwrap();
                assert_eq!(done.head_oid.as_deref(), Some(original.as_str()));
                assert!(done.changes.is_empty());
                assert!(done.sequence_progress.is_none());
                assert_eq!(root.join("a.txt").exists(), kind == ReplayKind::Revert);
                assert_eq!(root.join("b.txt").exists(), kind == ReplayKind::Revert);
                assert!(!checkout::pending(&root));
                let archives: Vec<_> = fs::read_dir(root.join(".git"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("branchdesk-sequence-stopped-")
                    })
                    .collect();
                assert_eq!(archives.len(), 1);
                let archived: Record =
                    serde_json::from_slice(&fs::read(&archives[0]).unwrap()).unwrap();
                assert!(archived.aborting);
                let output = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(["-c", "gc.auto=0", "gc", "--prune=now"])
                    .output()
                    .unwrap();
                assert!(output.status.success());
                open(&root)
                    .unwrap()
                    .find_commit(Oid::from_str(&applied).unwrap())
                    .unwrap();
            }
        }
    }
    #[test]
    fn abort_dirty_and_external_head_guards_preserve_data_and_progress() {
        let (d, _) = abort_fixture(ReplayKind::CherryPick, 1);
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        fs::write(root.join("a.txt"), b"external edits").unwrap();
        let snap = session.snapshot().unwrap();
        let state = bytes(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        assert_eq!(
            session
                .mutate(&snap.revision, Action::AbortSequence)
                .err()
                .unwrap(),
            "SEQUENCE_BLOCKED"
        );
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"external edits");
        assert_eq!(bytes(&root).unwrap(), state);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        let repo = open(&root).unwrap();
        repo.reference(
            "refs/heads/other",
            repo.head().unwrap().target().unwrap(),
            false,
            "test",
        )
        .unwrap();
        repo.set_head("refs/heads/other").unwrap();
        let snap = session.snapshot().unwrap();
        assert_eq!(
            session
                .mutate(&snap.revision, Action::AbortSequence)
                .err()
                .unwrap(),
            "STALE_SEQUENCE_HEAD"
        );
        assert_eq!(bytes(&root).unwrap(), state);
    }
    #[test]
    fn abort_progress_and_finish_kills_retry_without_losing_later_edits() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in [0, 1, 2, 3, 30, 20, 21, 22] {
                let (d, original) = abort_fixture(kind, 1);
                let root = d.path().canonicalize().unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", "abort")
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", point.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();

                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let reverted = point >= 20;
                if reverted {
                    assert_eq!(snap.head_oid.as_deref(), Some(original.as_str()));
                    fs::write(root.join("later.txt"), b"edit after rewind").unwrap();
                }
                if exists(&root) {
                    let current = session.snapshot().unwrap();
                    session
                        .mutate(&current.revision, Action::AbortSequence)
                        .unwrap();
                }
                let done = session.snapshot().unwrap();
                assert_eq!(done.head_oid.as_deref(), Some(original.as_str()));
                assert!(done.sequence_progress.is_none());
                assert!(!checkout::pending(&root));
                if reverted {
                    assert_eq!(
                        fs::read(root.join("later.txt")).unwrap(),
                        b"edit after rewind"
                    );
                }
                let archives: Vec<_> = fs::read_dir(root.join(".git"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("branchdesk-sequence-stopped-")
                    })
                    .collect();
                assert_eq!(archives.len(), 1);
                let archived: Record =
                    serde_json::from_slice(&fs::read(&archives[0]).unwrap()).unwrap();
                assert!(archived.aborting);
                assert_eq!(archived.original_head, original);
            }
        }
    }
    #[test]
    fn abort_recovery_refuses_later_file_edits_without_losing_abort_intent() {
        let (d, _) = abort_fixture(ReplayKind::CherryPick, 1);
        let root = d.path().canonicalize().unwrap();
        kill_checkout_action(&root, 1, false, "abort");
        fs::write(root.join("a.txt"), b"external after abort interruption").unwrap();
        let state = bytes(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session
            .mutate(&snap.revision, Action::RecoverCheckout)
            .err()
            .unwrap()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(
            fs::read(root.join("a.txt")).unwrap(),
            b"external after abort interruption"
        );
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(bytes(&root).unwrap(), state);
        assert!(checkout::pending(&root));
    }
    #[test]
    fn abort_checkout_kills_restore_applied_state_then_retry_rewinds_to_original() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in [100, 101, 102, 0, 1, 2, 3] {
                let (d, original) = abort_fixture(kind, 1);
                let root = d.path().canonicalize().unwrap();
                let applied = open(&root).unwrap().head().unwrap().target().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let files = ["a.txt", "b.txt"].map(|p| fs::read(root.join(p)).ok());
                kill_checkout_action(&root, point, false, "abort");
                if point == 2 {
                    kill_checkout_action(&root, 1, true, "abort");
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                assert!(snap.recovery_pending);
                assert!(snap.sequence_progress.as_ref().unwrap().aborting);
                let restored = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(
                    restored.head_oid.as_deref(),
                    Some(applied.to_string().as_str())
                );
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(
                    ["a.txt", "b.txt"].map(|p| fs::read(root.join(p)).ok()),
                    files
                );
                let done = session
                    .mutate(&restored.revision, Action::AbortSequence)
                    .unwrap();
                assert_eq!(done.head_oid.as_deref(), Some(original.as_str()));
                assert!(done.changes.is_empty());
                assert!(done.sequence_progress.is_none());
            }
        }
    }
    #[test]
    fn public_skip_preserves_head_and_files_and_rejects_duplicate_or_wrong_sequence() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for prepared in [false, true] {
                let (d, mut selections) = fixture();
                for selection in &mut selections {
                    selection.kind = kind;
                }
                configure_execution(d.path(), &mut selections, kind);
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let started = session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                let id = started.sequence_progress.as_ref().unwrap().id.clone();
                if prepared {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    prepare_next(&mut session, &started.revision, intent()).unwrap();
                }
                let snap = session.snapshot().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let files = ["a.txt", "b.txt"].map(|p| fs::read(root.join(p)).ok());
                let wrong = session
                    .mutate(
                        &snap.revision,
                        Action::SkipSequence {
                            sequence_id: "wrong".into(),
                            ordinal: 0,
                        },
                    )
                    .err()
                    .unwrap();
                assert_eq!(wrong, "STALE_SEQUENCE_STEP");
                let skipped = session
                    .mutate(
                        &snap.revision,
                        Action::SkipSequence {
                            sequence_id: id.clone(),
                            ordinal: 0,
                        },
                    )
                    .unwrap();
                assert_eq!(skipped.head_oid, snap.head_oid);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(
                    ["a.txt", "b.txt"].map(|p| fs::read(root.join(p)).ok()),
                    files
                );
                assert_eq!(skipped.sequence_progress.as_ref().unwrap().completed, 1);
                assert!(!skipped.sequence_progress.as_ref().unwrap().pending);
                assert_eq!(
                    session
                        .mutate(
                            &skipped.revision,
                            Action::SkipSequence {
                                sequence_id: id,
                                ordinal: 0
                            }
                        )
                        .err()
                        .unwrap(),
                    "STALE_SEQUENCE_STEP"
                );
                let done = session
                    .mutate(
                        &skipped.revision,
                        Action::ContinueSequence {
                            message: "remaining step".into(),
                            author: "Test".into(),
                            email: "test@invalid".into(),
                            allow_empty: false,
                        },
                    )
                    .unwrap();
                assert_eq!(done.sequence_progress.as_ref().unwrap().completed, 2);
                assert!(done.changes.is_empty());
                assert!(!root.join("a.txt").exists());
                assert!(root.join("b.txt").exists());
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .head()
                        .unwrap()
                        .peel_to_commit()
                        .unwrap()
                        .parent_id(0)
                        .unwrap()
                        .to_string(),
                    snap.head_oid.unwrap()
                );
            }
        }
    }
    #[test]
    fn public_skip_refuses_dirty_state_and_already_published_unacknowledged_commit() {
        let (d, selections) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let started = session
            .mutate(&snap.revision, Action::StartSequence { selections })
            .unwrap();
        let id = started.sequence_progress.as_ref().unwrap().id.clone();
        fs::write(root.join("external.txt"), b"keep").unwrap();
        let snap = session.snapshot().unwrap();
        let state = bytes(&root).unwrap();
        assert_eq!(
            session
                .mutate(
                    &snap.revision,
                    Action::SkipSequence {
                        sequence_id: id.clone(),
                        ordinal: 0
                    }
                )
                .err()
                .unwrap(),
            "SEQUENCE_BLOCKED"
        );
        assert_eq!(bytes(&root).unwrap(), state);
        assert_eq!(fs::read(root.join("external.txt")).unwrap(), b"keep");
        fs::remove_file(root.join("external.txt")).unwrap();
        let snap = session.snapshot().unwrap();
        let pending;
        {
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            prepare_next(&mut session, &snap.revision, intent()).unwrap();
            pending = bytes(&root).unwrap().unwrap();
            let snap = session.snapshot().unwrap();
            apply_pending(&mut session, &snap.revision).unwrap();
        }
        // Restore pre-acknowledgment state to model the post-commit crash gap.
        fs::write(root.join(FILE), &pending).unwrap();
        let snap = session.snapshot().unwrap();
        let head = snap.head_oid.clone();
        assert!(session
            .mutate(
                &snap.revision,
                Action::SkipSequence {
                    sequence_id: id,
                    ordinal: 0
                }
            )
            .err()
            .unwrap()
            .starts_with("STALE_SEQUENCE_HEAD"));
        assert_eq!(bytes(&root).unwrap().unwrap(), pending);
        let resumed = session
            .mutate(
                &snap.revision,
                Action::ContinueSequence {
                    message: String::new(),
                    author: String::new(),
                    email: String::new(),
                    allow_empty: false,
                },
            )
            .unwrap();
        assert_eq!(resumed.head_oid, head);
        assert_eq!(resumed.sequence_progress.unwrap().completed, 1);
    }
    #[test]
    fn public_sequence_resumes_pinned_steps_after_source_branch_deletion_and_gc() {
        let (d, selections) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let started = session
            .mutate(&snap.revision, Action::StartSequence { selections })
            .unwrap();
        open(&root)
            .unwrap()
            .find_reference("refs/heads/topic")
            .unwrap()
            .delete()
            .unwrap();
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "gc.auto=0", "gc", "--prune=now"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let pinned_source = started
            .sequence_progress
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .oid
            .clone();
        {
            let repo = open(&root).unwrap();
            repo.reference(
                "refs/heads/same-head",
                repo.head().unwrap().target().unwrap(),
                false,
                "test branch",
            )
            .unwrap();
            repo.set_head("refs/heads/same-head").unwrap();
        }
        let foreign = session.snapshot().unwrap();
        assert_eq!(
            session
                .replay_preview(
                    &foreign.revision,
                    &pinned_source,
                    ReplayKind::CherryPick,
                    None
                )
                .err()
                .unwrap(),
            "INVALID_COMMIT"
        );
        open(&root).unwrap().set_head("refs/heads/main").unwrap();
        for ordinal in 0..2 {
            drop(session);
            session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let selection = snap
                .sequence_progress
                .as_ref()
                .unwrap()
                .current
                .as_ref()
                .unwrap();
            assert!(!snap.commits.iter().any(|c| c.oid == selection.oid));
            let done = session
                .mutate(
                    &snap.revision,
                    Action::ContinueSequence {
                        message: format!("pinned step {ordinal}"),
                        author: "Pin Test".into(),
                        email: "pin@invalid".into(),
                        allow_empty: false,
                    },
                )
                .unwrap();
            assert_eq!(
                done.sequence_progress.as_ref().unwrap().completed,
                ordinal + 1
            );
            assert!(done.changes.is_empty());
        }
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"first\n");
        assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"second\n");
        let repo = open(&root).unwrap();
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent(0)
                .unwrap()
                .parent_id(0)
                .unwrap()
                .to_string(),
            started.head_oid.unwrap()
        );
        assert!(repo.find_reference("refs/heads/topic").is_err());
        let snap = session.snapshot().unwrap();
        let finished = session
            .mutate(&snap.revision, Action::StopSequence)
            .unwrap();
        assert_eq!(
            session
                .replay_preview(
                    &finished.revision,
                    &pinned_source,
                    ReplayKind::CherryPick,
                    None
                )
                .err()
                .unwrap(),
            "INVALID_COMMIT"
        );
    }
    #[test]
    fn public_sequence_actions_apply_resume_and_finish_without_losing_later_edits() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            let (d, mut selections) = fixture();
            for selection in &mut selections {
                selection.kind = kind;
            }
            configure_execution(d.path(), &mut selections, kind);
            let root = d.path().canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let started = session
                .mutate(&snap.revision, Action::StartSequence { selections })
                .unwrap();
            assert_eq!(started.sequence_progress.as_ref().unwrap().completed, 0);
            let next = session
                .mutate(
                    &started.revision,
                    Action::ContinueSequence {
                        message: "first public step".into(),
                        author: "Public".into(),
                        email: "public@invalid".into(),
                        allow_empty: false,
                    },
                )
                .unwrap();
            assert_eq!(next.sequence_progress.as_ref().unwrap().completed, 1);
            {
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                prepare_next(&mut session, &next.revision, intent()).unwrap();
            }
            let expected = read(&root).unwrap().unwrap().pending.unwrap().after;
            drop(session);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let done = session
                .mutate(
                    &snap.revision,
                    Action::ContinueSequence {
                        message: String::new(),
                        author: String::new(),
                        email: String::new(),
                        allow_empty: false,
                    },
                )
                .unwrap();
            assert_eq!(done.head_oid.as_deref(), Some(expected.as_str()));
            assert_eq!(done.sequence_progress.as_ref().unwrap().completed, 2);
            assert!(done.changes.is_empty());
            let record = bytes(&root).unwrap().unwrap();
            let index = fs::read(root.join(".git/index")).unwrap();
            fs::write(root.join("later.txt"), b"keep my work").unwrap();
            let snap = session.snapshot().unwrap();
            let finished = session
                .mutate(&snap.revision, Action::StopSequence)
                .unwrap();
            assert!(finished.sequence_progress.is_none());
            assert!(finished.sequence_error.is_none());
            assert_eq!(finished.head_oid, done.head_oid);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(root.join("later.txt")).unwrap(), b"keep my work");
            let archived: Vec<_> = fs::read_dir(root.join(".git"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("branchdesk-sequence-stopped-")
                })
                .collect();
            assert_eq!(archived.len(), 1);
            assert_eq!(fs::read(&archived[0]).unwrap(), record);
            session
                .mutate(
                    &finished.revision,
                    Action::CreateBranch {
                        name: "after-sequence".into(),
                    },
                )
                .unwrap();
        }
    }
    fn save_test_resolution(session: &mut Session, text: &str) -> Snapshot {
        let snap = session.snapshot().unwrap();
        let progress = snap.sequence_progress.as_ref().unwrap();
        let selection = progress.current.as_ref().unwrap();
        let plan = session
            .replay_preview(
                &snap.revision,
                &selection.oid,
                selection.kind,
                selection.mainline,
            )
            .unwrap();
        let c = &plan.conflicts[0];
        let id = c
            .ours
            .as_ref()
            .or(c.theirs.as_ref())
            .or(c.ancestor.as_ref())
            .unwrap()
            .id
            .clone();
        session
            .mutate(
                &snap.revision,
                Action::ResolveSequence {
                    sequence_id: progress.id.clone(),
                    ordinal: progress.completed,
                    file_id: id,
                    resolution: merge_draft::Resolution::Text { text: text.into() },
                },
            )
            .unwrap()
    }
    #[test]
    fn public_resolution_save_kills_keep_old_or_complete_map_and_resume_both_kinds() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in 0..=3 {
                let (d, selections) = conflict_fixture(kind);
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let started = session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                save_test_resolution(&mut session, "old saved text\n");
                let before = bytes(&root).unwrap().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", "save")
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", point.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();

                let mut session = Session::new(root.clone()).unwrap();
                let restarted = session.snapshot().unwrap();
                assert_eq!(restarted.head_oid, started.head_oid);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"ours\n");
                if point < 2 {
                    assert_eq!(bytes(&root).unwrap().unwrap(), before);
                }
                let map = &restarted.sequence_progress.as_ref().unwrap().resolutions;
                assert_eq!(map.len(), 1);
                match map.values().next().unwrap() {
                    merge_draft::Resolution::Text { text } => assert_eq!(
                        text,
                        if point < 2 {
                            "old saved text\n"
                        } else {
                            "new saved text\n"
                        }
                    ),
                    _ => panic!("wrong resolution"),
                }
                let saved = save_test_resolution(&mut session, "new saved text\n");
                assert_eq!(
                    saved.sequence_progress.as_ref().unwrap().resolutions.len(),
                    1
                );
                let done = session
                    .mutate(
                        &saved.revision,
                        Action::ContinueSequence {
                            message: "after save restart".into(),
                            author: "Save Test".into(),
                            email: "save@invalid".into(),
                            allow_empty: false,
                        },
                    )
                    .unwrap();
                assert_eq!(done.sequence_progress.as_ref().unwrap().completed, 1);
                assert!(done.changes.is_empty());
                assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"new saved text\n");
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .head()
                        .unwrap()
                        .peel_to_commit()
                        .unwrap()
                        .parent_id(0)
                        .unwrap()
                        .to_string(),
                    started.head_oid.unwrap()
                );
            }
        }
    }
    #[test]
    fn public_skip_publication_kills_never_skip_the_following_step_on_retry() {
        for prepared in [false, true] {
            for point in 0..=3 {
                let (d, selections) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let started = session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                let id = started.sequence_progress.as_ref().unwrap().id.clone();
                if prepared {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    prepare_next(&mut session, &started.revision, intent()).unwrap();
                }
                let before = bytes(&root).unwrap().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", "skip")
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", point.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();

                let mut session = Session::new(root.clone()).unwrap();
                let restarted = session.snapshot().unwrap();
                let completed = restarted.sequence_progress.as_ref().unwrap().completed;
                if point < 2 {
                    assert_eq!(bytes(&root).unwrap().unwrap(), before);
                    assert_eq!(completed, 0);
                } else {
                    assert_eq!(completed, 1);
                }
                let result = session.mutate(
                    &restarted.revision,
                    Action::SkipSequence {
                        sequence_id: id,
                        ordinal: 0,
                    },
                );
                if completed == 0 {
                    result.unwrap();
                } else {
                    assert_eq!(result.err().unwrap(), "STALE_SEQUENCE_STEP");
                }
                let done = session.snapshot().unwrap();
                assert_eq!(done.sequence_progress.as_ref().unwrap().completed, 1);
                assert!(!done.sequence_progress.as_ref().unwrap().pending);
                assert_eq!(done.head_oid, started.head_oid);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert!(!root.join("a.txt").exists());
                assert!(!root.join("b.txt").exists());
            }
        }
    }
    #[test]
    fn public_stop_archive_kills_preserve_valid_and_damaged_progress_exactly_once() {
        for damaged in [false, true] {
            for point in [20, 21, 22] {
                let (d, selections) = fixture();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                session
                    .mutate(&snap.revision, Action::StartSequence { selections })
                    .unwrap();
                if damaged {
                    fs::write(root.join(FILE), b"broken progress").unwrap();
                }
                let before = bytes(&root).unwrap().unwrap();
                let snap = session.snapshot().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                fs::write(root.join("later.txt"), b"external edit").unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", "stop")
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", point.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let restarted = session.snapshot().unwrap();
                if exists(&root) {
                    session
                        .mutate(&restarted.revision, Action::StopSequence)
                        .unwrap();
                }
                assert!(!exists(&root));
                assert_eq!(session.snapshot().unwrap().head_oid, snap.head_oid);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(root.join("later.txt")).unwrap(), b"external edit");
                let archived: Vec<_> = fs::read_dir(root.join(".git"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("branchdesk-sequence-stopped-")
                    })
                    .collect();
                assert_eq!(archived.len(), 1);
                assert_eq!(fs::read(&archived[0]).unwrap(), before);
            }
        }
    }
    #[test]
    fn product_snapshot_tracks_progress_and_blocks_writes_even_for_damaged_state() {
        let (d, selections) = fixture();
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let before = session.snapshot().unwrap();
        {
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            begin(&mut session, &before.revision, selections).unwrap();
        }
        let started = session.snapshot().unwrap();
        assert_ne!(started.revision, before.revision);
        let progress = started.sequence_progress.as_ref().unwrap();
        assert_eq!(
            (progress.completed, progress.total, progress.pending),
            (0, 2, false)
        );
        assert!(started.sequence_error.is_none());
        let index = fs::read(root.join(".git/index")).unwrap();
        for damaged in [false, true] {
            if damaged {
                fs::write(root.join(FILE), b"broken sequence").unwrap();
            }
            let snap = session.snapshot().unwrap();
            if damaged {
                assert_ne!(snap.revision, started.revision);
                assert!(snap.sequence_progress.is_none());
                assert!(snap
                    .sequence_error
                    .as_ref()
                    .unwrap()
                    .contains("INVALID_SEQUENCE_STATE"));
                assert!(session
                    .mutate(&started.revision, Action::StageAll)
                    .err()
                    .unwrap()
                    .contains("STALE_STATE"));
            }
            let state = bytes(&root).unwrap();
            for action in [
                Action::StageAll,
                Action::CreateBranch {
                    name: "blocked".into(),
                },
            ] {
                assert!(session
                    .mutate(&snap.revision, action)
                    .err()
                    .unwrap()
                    .contains("SEQUENCE_IN_PROGRESS"));
            }
            assert_eq!(bytes(&root).unwrap(), state);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(session.snapshot().unwrap().head_oid, before.head_oid);
            assert!(open(&root)
                .unwrap()
                .find_reference("refs/heads/blocked")
                .is_err());
        }
    }
    #[test]
    fn progress_roundtrip_refuses_stale_and_invalid_chain_without_repository_writes() {
        let (d, selections) = fixture();
        let root = d.path().canonicalize().unwrap();
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let head = snap.head_oid.clone();
        let index = fs::read(root.join(".git/index")).unwrap();
        let mut record = begin(&mut session, &snap.revision, selections.clone()).unwrap();
        assert!(begin(&mut session, &snap.revision, selections).is_err());
        let old = bytes(&root).unwrap().unwrap();
        record.prepare(&"a".repeat(40)).unwrap();
        publish(&root, &record, Some(&old)).unwrap();
        let prepared = bytes(&root).unwrap().unwrap();
        let mut restored = read(&root).unwrap().unwrap();
        assert!(restored.skip(&restored.expected_head.clone()).is_err());
        assert!(restored.acknowledge(&"b".repeat(40)).is_err());
        restored.acknowledge(&"a".repeat(40)).unwrap();
        assert!(restored.acknowledge(&"a".repeat(40)).is_err());
        publish(&root, &restored, Some(&prepared)).unwrap();
        assert_eq!(
            publish(&root, &record, Some(&old)).unwrap_err(),
            "STALE_SEQUENCE_STATE"
        );
        let before = bytes(&root).unwrap().unwrap();
        restored.skip(&"a".repeat(40)).unwrap();
        publish(&root, &restored, Some(&before)).unwrap();
        assert!(restored.prepare(&"b".repeat(40)).is_err());
        let mut forged = restored.clone();
        forged.expected_head = "b".repeat(40);
        assert!(forged.validate().is_err());
        assert_eq!(session.snapshot().unwrap().head_oid, head);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        fs::write(root.join(FILE), b"broken json").unwrap();
        assert!(read(&root).is_err());
        assert_eq!(fs::read(root.join(FILE)).unwrap(), b"broken json");
    }
    fn configure_execution(root: &Path, selections: &mut [ReplaySelection], kind: ReplayKind) {
        if kind == ReplayKind::Revert {
            let repo = open(root).unwrap();
            repo.find_reference("refs/heads/main")
                .unwrap()
                .set_target(
                    Oid::from_str(&selections.last().unwrap().oid).unwrap(),
                    "test setup",
                )
                .unwrap();
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .unwrap();
            selections.reverse();
        }
    }
    fn intent() -> Intent {
        Intent {
            message: "Sequence step".into(),
            name: "Sequence User".into(),
            email: "sequence@example.invalid".into(),
            seconds: 1_700_000_000,
            offset: 540,
            allow_empty: true,
        }
    }
    #[test]
    fn frozen_intent_executes_ordered_steps_and_refuses_tampered_candidate() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            let (d, mut selections) = fixture();
            for selection in &mut selections {
                selection.kind = kind;
            }
            configure_execution(d.path(), &mut selections, kind);
            let root = d.path().canonicalize().unwrap();
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let original = snap.head_oid.clone().unwrap();
            let record = begin(&mut session, &snap.revision, selections).unwrap();
            verify_pins(&open(&root).unwrap(), &record).unwrap();
            for ordinal in 0..2 {
                let snap = session.snapshot().unwrap();
                prepare_next(&mut session, &snap.revision, intent()).unwrap();
                let original_bytes = bytes(&root).unwrap().unwrap();
                let pending = read(&root).unwrap().unwrap().pending.unwrap();
                let mut tampered = read(&root).unwrap().unwrap();
                tampered
                    .pending
                    .as_mut()
                    .unwrap()
                    .intent
                    .as_mut()
                    .unwrap()
                    .message = "different intent".into();
                publish(&root, &tampered, Some(&original_bytes)).unwrap();
                let snap = session.snapshot().unwrap();
                assert_eq!(
                    apply_pending(&mut session, &snap.revision).unwrap_err(),
                    "STALE_SEQUENCE_INTENT"
                );
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .head()
                        .unwrap()
                        .target()
                        .unwrap()
                        .to_string(),
                    pending.before
                );
                fs::write(root.join(FILE), &original_bytes).unwrap();
                drop(session);
                session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                apply_pending(&mut session, &snap.revision).unwrap();
                let done = read(&root).unwrap().unwrap();
                assert_eq!(done.completed.len(), ordinal + 1);
                assert!(done.pending.is_none());
                assert_eq!(done.expected_head, pending.after);
                let repo = open(&root).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(commit.id().to_string(), pending.after);
                assert_eq!(commit.parent_id(0).unwrap().to_string(), pending.before);
                assert_eq!(commit.committer().when(), intent().time());
                assert_eq!(
                    root.join("a.txt").exists(),
                    kind == ReplayKind::CherryPick || ordinal == 0
                );
                assert_eq!(
                    root.join("b.txt").exists(),
                    kind == ReplayKind::CherryPick && ordinal == 1
                );
                assert!(session.snapshot().unwrap().changes.is_empty());
            }
            let done = read(&root).unwrap().unwrap();
            assert_eq!(done.original_head, original);
            let snap = session.snapshot().unwrap();
            assert_eq!(
                apply_pending(&mut session, &snap.revision).unwrap_err(),
                "NO_SEQUENCE_PENDING_STEP"
            );
            verify_pins(&open(&root).unwrap(), &done).unwrap();
        }
    }
    #[test]
    fn source_pins_survive_branch_deletion_and_gc_and_tampering_blocks_apply() {
        let (d, selections) = fixture();
        let root = d.path().canonicalize().unwrap();
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let record = begin(&mut session, &snap.revision, selections).unwrap();
        open(&root)
            .unwrap()
            .find_reference("refs/heads/topic")
            .unwrap()
            .delete()
            .unwrap();
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "gc.auto=0", "gc", "--prune=now"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        verify_pins(&open(&root).unwrap(), &record).unwrap();
        for selection in &record.selections {
            open(&root)
                .unwrap()
                .find_commit(Oid::from_str(&selection.oid).unwrap())
                .unwrap();
        }
        let snap = session.snapshot().unwrap();
        assert!(!snap
            .commits
            .iter()
            .any(|c| c.oid == record.selections[0].oid));
        session
            .replay_preview(
                &snap.revision,
                &record.selections[0].oid,
                ReplayKind::CherryPick,
                None,
            )
            .unwrap();
        assert!(session
            .replay_preview(
                &snap.revision,
                &record.selections[1].oid,
                ReplayKind::CherryPick,
                None
            )
            .is_err());
        assert!(session
            .replay_preview(
                &snap.revision,
                &record.selections[0].oid,
                ReplayKind::Revert,
                None
            )
            .is_err());
        assert!(session
            .replay_preview(
                &snap.revision,
                &record.selections[0].oid,
                ReplayKind::CherryPick,
                Some(1)
            )
            .is_err());
        // Restore the visible ref for the independent changed-pin refusal check.
        open(&root)
            .unwrap()
            .reference(
                "refs/heads/topic",
                Oid::from_str(&record.selections.last().unwrap().oid).unwrap(),
                false,
                "test restore",
            )
            .unwrap();
        let snap = session.snapshot().unwrap();
        prepare_next(&mut session, &snap.revision, intent()).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let state = bytes(&root).unwrap();
        let source_pin = &pin_names(&record)[1].0;
        open(&root)
            .unwrap()
            .find_reference(source_pin)
            .unwrap()
            .set_target(Oid::from_str(&record.original_head).unwrap(), "test tamper")
            .unwrap();
        let snap = session.snapshot().unwrap();
        assert_eq!(
            apply_pending(&mut session, &snap.revision).unwrap_err(),
            "STALE_SEQUENCE_PIN"
        );
        assert_eq!(
            session.snapshot().unwrap().head_oid.as_deref(),
            Some(record.original_head.as_str())
        );
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(bytes(&root).unwrap(), state);
        assert!(!root.join("a.txt").exists());
    }
    #[test]
    fn actual_kill_after_step_commit_finishes_progress_without_duplicate_or_lost_edits() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in [10, 0, 1, 2, 3] {
                let (d, mut selections) = fixture();
                for selection in &mut selections {
                    selection.kind = kind;
                }
                configure_execution(d.path(), &mut selections, kind);
                let root = d.path().canonicalize().unwrap();
                let expected;
                {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    begin(&mut session, &snap.revision, selections).unwrap();
                    let snap = session.snapshot().unwrap();
                    prepare_next(&mut session, &snap.revision, intent()).unwrap();
                    expected = read(&root).unwrap().unwrap().pending.unwrap().after;
                }
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", "apply")
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", point.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .head()
                        .unwrap()
                        .target()
                        .unwrap()
                        .to_string(),
                    expected
                );
                let index = fs::read(root.join(".git/index")).unwrap();
                fs::write(root.join("later.txt"), "external edit after commit").unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                if read(&root).unwrap().unwrap().pending.is_some() {
                    apply_pending(&mut session, &snap.revision).unwrap();
                }
                let done = read(&root).unwrap().unwrap();
                assert_eq!(done.completed.len(), 1);
                assert!(done.pending.is_none());
                assert_eq!(done.expected_head, expected);
                assert_eq!(
                    open(&root)
                        .unwrap()
                        .head()
                        .unwrap()
                        .target()
                        .unwrap()
                        .to_string(),
                    expected
                );
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(root.join("later.txt")).unwrap(),
                    "external edit after commit"
                );
            }
        }
    }

    fn kill_checkout(root: &Path, step: usize, recovery: bool) {
        kill_checkout_action(root, step, recovery, "apply")
    }
    fn kill_checkout_action(root: &Path, step: usize, recovery: bool, action: &str) {
        let ready = root.join(".git/branchdesk-checkout/child.ready");
        let _ = fs::remove_file(&ready);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::sequence_state::tests::sequence_state_child_job",
                "--nocapture",
            ])
            .env("BRANCHDESK_SEQUENCE_STATE_ROOT", root)
            .env(
                "BRANCHDESK_SEQUENCE_STATE_ACTION",
                if recovery { "recover" } else { action },
            )
            .env("BRANCHDESK_CRASH_ROOT", root)
            .env(
                if recovery {
                    "BRANCHDESK_RECOVERY_STEP"
                } else {
                    "BRANCHDESK_CRASH_STEP"
                },
                step.to_string(),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("checkpoint timeout {step}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn checkout_kills_restore_pending_intent_then_resume_both_sequence_steps() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for point in [100, 101, 102, 0, 1, 2, 3] {
                let (d, mut selections) = fixture();
                for selection in &mut selections {
                    selection.kind = kind;
                }
                configure_execution(d.path(), &mut selections, kind);
                let root = d.path().canonicalize().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let original_files: Vec<_> = ["a.txt", "b.txt"]
                    .map(|p| fs::read(root.join(p)).ok())
                    .into();
                let expected;
                let state;
                {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    begin(&mut session, &snap.revision, selections).unwrap();
                    let snap = session.snapshot().unwrap();
                    prepare_next(&mut session, &snap.revision, intent()).unwrap();
                    expected = read(&root).unwrap().unwrap().pending.unwrap();
                    state = bytes(&root).unwrap();
                }
                kill_checkout(&root, point, false);
                {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    assert!(snap.recovery_pending);
                    assert_eq!(
                        apply_pending(&mut session, &snap.revision).unwrap_err(),
                        "RECOVERY_REQUIRED"
                    );
                    assert_eq!(bytes(&root).unwrap(), state);
                }
                if point == 2 {
                    kill_checkout(&root, 1, true);
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let recovered = session
                    .mutate(&snap.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!recovered.recovery_pending);
                assert_eq!(
                    recovered.head_oid.as_deref(),
                    Some(expected.before.as_str())
                );
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                for (i, path) in ["a.txt", "b.txt"].iter().enumerate() {
                    assert_eq!(fs::read(root.join(path)).ok(), original_files[i]);
                }
                assert_eq!(bytes(&root).unwrap(), state);
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                apply_pending(&mut session, &recovered.revision).unwrap();
                let first = session.snapshot().unwrap();
                assert_eq!(first.head_oid.as_deref(), Some(expected.after.as_str()));
                prepare_next(&mut session, &first.revision, intent()).unwrap();
                let snap = session.snapshot().unwrap();
                apply_pending(&mut session, &snap.revision).unwrap();
                let done = read(&root).unwrap().unwrap();
                assert_eq!(done.completed.len(), 2);
                assert!(done.pending.is_none());
                assert!(session.snapshot().unwrap().changes.is_empty());
                for path in ["a.txt", "b.txt"] {
                    assert_eq!(root.join(path).exists(), kind == ReplayKind::CherryPick);
                }
                let repo = open(&root).unwrap();
                let last = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(last.parent_id(0).unwrap().to_string(), expected.after);
                assert_eq!(
                    last.parent(0).unwrap().parent_id(0).unwrap().to_string(),
                    expected.before
                );
                verify_pins(&repo, &done).unwrap();
            }
        }
    }
    #[test]
    fn interrupted_sequence_recovery_preserves_later_edits_and_pending_intent() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            let (d, mut selections) = fixture();
            for selection in &mut selections {
                selection.kind = kind;
            }
            configure_execution(d.path(), &mut selections, kind);
            let root = d.path().canonicalize().unwrap();
            {
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                begin(&mut session, &snap.revision, selections).unwrap();
                let snap = session.snapshot().unwrap();
                prepare_next(&mut session, &snap.revision, intent()).unwrap();
            }
            kill_checkout(&root, 1, false);
            let path = root.join(if kind == ReplayKind::CherryPick {
                "a.txt"
            } else {
                "b.txt"
            });
            let after = fs::read(&path).ok();
            fs::write(&path, b"later external changes\n").unwrap();
            let index = fs::read(root.join(".git/index")).unwrap();
            let state = bytes(&root).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let error = session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .err()
                .unwrap();
            assert!(error.contains("RECOVERY_CONFLICT"), "{error}");
            assert_eq!(fs::read(&path).unwrap(), b"later external changes\n");
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(bytes(&root).unwrap(), state);
            assert_eq!(session.snapshot().unwrap().head_oid, snap.head_oid);
            assert!(checkout::pending(&root));
            // Simulate the user saving their changes outside the repository and
            // restoring the interrupted operation's file state before retrying.
            let saved = tempfile::tempdir().unwrap();
            fs::copy(&path, saved.path().join("saved.txt")).unwrap();
            match after {
                Some(data) => fs::write(&path, data).unwrap(),
                None => fs::remove_file(&path).unwrap(),
            }
            let snap = session.snapshot().unwrap();
            let recovered = session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(bytes(&root).unwrap(), state);
            let _lease = lease::OperationLease::acquire(&root).unwrap();
            apply_pending(&mut session, &recovered.revision).unwrap();
            assert_eq!(read(&root).unwrap().unwrap().completed.len(), 1);
            assert_eq!(
                fs::read(saved.path().join("saved.txt")).unwrap(),
                b"later external changes\n"
            );
        }
    }
    #[test]
    fn sequence_state_child_job() {
        let Some(root) = std::env::var_os("BRANCHDESK_SEQUENCE_STATE_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").as_deref() == Ok("abort") {
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            session
                .mutate(&snap.revision, Action::AbortSequence)
                .unwrap();
            return;
        }
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").as_deref() == Ok("save") {
            let mut session = Session::new(root.clone()).unwrap();
            save_test_resolution(&mut session, "new saved text\n");
            return;
        }
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").as_deref() == Ok("skip") {
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let id = snap.sequence_progress.as_ref().unwrap().id.clone();
            session
                .mutate(
                    &snap.revision,
                    Action::SkipSequence {
                        sequence_id: id,
                        ordinal: 0,
                    },
                )
                .unwrap();
            return;
        }
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").as_deref() == Ok("stop") {
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            session
                .mutate(&snap.revision, Action::StopSequence)
                .unwrap();
            return;
        }
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").as_deref() == Ok("recover") {
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
            return;
        }
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        if std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION").unwrap() == "apply" {
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            apply_pending(&mut session, &snap.revision).unwrap();
            return;
        }
        let before = bytes(&root).unwrap().unwrap();
        let mut record = read(&root).unwrap().unwrap();
        match std::env::var("BRANCHDESK_SEQUENCE_STATE_ACTION")
            .unwrap()
            .as_str()
        {
            "prepare" => record.prepare(&"a".repeat(40)).unwrap(),
            "acknowledge" => record.acknowledge(&"a".repeat(40)).unwrap(),
            "skip" => record.skip(&record.expected_head.clone()).unwrap(),
            _ => panic!("invalid test action"),
        };
        publish(&root, &record, Some(&before)).unwrap();
    }
    #[test]
    fn actual_kill_progress_publication_is_old_or_complete_and_restartable() {
        for action in ["prepare", "acknowledge", "skip"] {
            for step in 0..=3 {
                let (d, selections) = fixture();
                let root = d.path().canonicalize().unwrap();
                let index = fs::read(root.join(".git/index")).unwrap();
                let head = open(&root).unwrap().head().unwrap().target();
                {
                    let _lease = lease::OperationLease::acquire(&root).unwrap();
                    let mut session = Session::new(root.clone()).unwrap();
                    let snap = session.snapshot().unwrap();
                    let mut record = begin(&mut session, &snap.revision, selections).unwrap();
                    if action == "acknowledge" {
                        let before = bytes(&root).unwrap().unwrap();
                        record.prepare(&"a".repeat(40)).unwrap();
                        publish(&root, &record, Some(&before)).unwrap();
                    }
                }
                let before = bytes(&root).unwrap().unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::sequence_state::tests::sequence_state_child_job",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_SEQUENCE_STATE_ROOT", &root)
                    .env("BRANCHDESK_SEQUENCE_STATE_ACTION", action)
                    .env("BRANCHDESK_SEQUENCE_STATE_STEP", step.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while !root.join(".git/sequence-state.ready").exists() {
                    if child.try_wait().unwrap().is_some() {
                        let output = child.wait_with_output().unwrap();
                        panic!("child failed {}", String::from_utf8_lossy(&output.stderr));
                    }
                    if std::time::Instant::now() > deadline {
                        child.kill().unwrap();
                        child.wait().unwrap();
                        panic!("checkpoint timeout");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let _lease = lease::OperationLease::acquire(&root).unwrap();
                let mut restored = read(&root).unwrap().unwrap();
                if step < 2 {
                    assert_eq!(bytes(&root).unwrap().unwrap(), before);
                    match action {
                        "prepare" => restored.prepare(&"a".repeat(40)).unwrap(),
                        "acknowledge" => restored.acknowledge(&"a".repeat(40)).unwrap(),
                        "skip" => restored.skip(&restored.expected_head.clone()).unwrap(),
                        _ => unreachable!(),
                    };
                    publish(&root, &restored, Some(&before)).unwrap();
                }
                let final_record = read(&root).unwrap().unwrap();
                if action == "prepare" {
                    assert!(final_record.pending.is_some());
                    assert!(final_record.completed.is_empty());
                } else {
                    assert!(final_record.pending.is_none());
                    assert_eq!(final_record.completed.len(), 1);
                }
                assert_eq!(open(&root).unwrap().head().unwrap().target(), head);
                assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
                assert!(!root.join(".git/index.lock").exists());
            }
        }
    }
}
