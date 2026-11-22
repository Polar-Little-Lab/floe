//! Durable remote-operation intent. Pending never authorizes an automatic resend.
use super::*;
const FILE: &str = ".git/branchdesk-push.json";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Pending,
    Accepted,
    Rejected,
    NotSent,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationKind {
    TargetPresent,
    PreviousTarget,
    DifferentTarget,
    Missing,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub kind: ObservationKind,
    pub oid: Option<String>,
    pub checked_at: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    version: u32,
    pub id: String,
    pub remote: String,
    pub endpoint: String,
    pub source: String,
    pub destination: String,
    pub old_oid: Option<String>,
    pub new_oid: String,
    pub status: Status,
    pub detail: Option<String>,
    #[serde(default)]
    pub observation: Option<Observation>,
}
pub fn exists(root: &Path) -> bool {
    !matches!(fs::symlink_metadata(root.join(FILE)),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
}
fn bytes(root: &Path) -> Result<Option<Vec<u8>>> {
    if exists(root) {
        Ok(Some(bounded(&root.join(FILE), 16384)?))
    } else {
        Ok(None)
    }
}
fn valid(r: &Record) -> bool {
    let oid = |s: &str| s.len() == 40 && Oid::from_str(s).is_ok_and(|id| !id.is_zero());
    r.version == 1
        && Uuid::parse_str(&r.id).is_ok()
        && !r.remote.is_empty()
        && r.remote.len() <= 200
        && remote_endpoint::https(&r.endpoint).is_ok_and(|u| u.as_str() == r.endpoint)
        && [&r.source, &r.destination].iter().all(|s| {
            s.len() <= 1035 && s.starts_with("refs/heads/") && git2::Reference::is_valid_name(s)
        })
        && oid(&r.new_oid)
        && r.old_oid.as_ref().is_none_or(|s| oid(s) && s != &r.new_oid)
        && r.detail
            .as_ref()
            .is_none_or(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
        && r.observation.as_ref().is_none_or(|o| {
            o.oid.as_ref().is_none_or(|id| oid(id))
                && o.kind == observation_kind(r, o.oid.as_deref())
        })
        && match r.status {
            Status::Pending | Status::Accepted => r.detail.is_none(),
            _ => r.detail.is_some(),
        }
}
pub fn read(root: &Path) -> Result<Option<Record>> {
    let Some(data) = bytes(root)? else {
        return Ok(None);
    };
    let record: Record = serde_json::from_slice(&data).map_err(|_| "INVALID_PUSH_RECORD")?;
    if !valid(&record) {
        return Err("INVALID_PUSH_RECORD".into());
    }
    Ok(Some(record))
}
fn save(root: &Path, record: &Record, before: Option<&[u8]>) -> Result<()> {
    if !valid(record) {
        return Err("INVALID_PUSH_RECORD".into());
    }
    if bytes(root)?.as_deref() != before {
        return Err("STALE_PUSH_RECORD".into());
    }
    let value = serde_json::to_vec(record).map_err(err)?;
    let mut temp = tempfile::Builder::new()
        .prefix("branchdesk-push-")
        .suffix(".next")
        .tempfile_in(root.join(".git"))
        .map_err(err)?;
    std::io::Write::write_all(&mut temp, &value).map_err(err)?;
    temp.as_file().sync_all().map_err(err)?;
    checkpoint(root, 0);
    if bytes(root)?.as_deref() != before {
        return Err("STALE_PUSH_RECORD".into());
    }
    if before.is_none() {
        temp.persist_noclobber(root.join(FILE)).map_err(err)?;
    } else {
        temp.persist(root.join(FILE)).map_err(err)?;
    }
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, 1);
    Ok(())
}
pub fn begin(
    session: &mut Session,
    revision: &str,
    selected: &push::Selection,
    old: Option<String>,
) -> Result<Record> {
    let _lease = lease::OperationLease::acquire(&session.root)?;
    if exists(&session.root) {
        return Err("PUSH_PENDING_RESULT".into());
    }
    let branch = selected
        .destination
        .strip_prefix("refs/heads/")
        .ok_or("INVALID_PUSH_BRANCH")?;
    if push::selection(session, revision, &selected.remote, branch)? != *selected {
        return Err("STALE_PUSH_SELECTION".into());
    }
    let repo = open(&session.root)?;
    let new = Oid::from_str(&selected.source_oid).map_err(err)?;
    if let Some(old) = &old {
        let old = Oid::from_str(old).map_err(err)?;
        if old == new || !repo.graph_descendant_of(new, old).map_err(err)? {
            return Err("PUSH_NON_FAST_FORWARD".into());
        }
    }
    let record = Record {
        version: 1,
        id: Uuid::new_v4().to_string(),
        remote: selected.remote.clone(),
        endpoint: selected.endpoint.clone(),
        source: selected.source.clone(),
        destination: selected.destination.clone(),
        old_oid: old,
        new_oid: selected.source_oid.clone(),
        status: Status::Pending,
        detail: None,
        observation: None,
    };
    if !valid(&record) {
        return Err("INVALID_PUSH_RECORD".into());
    }
    repo.reference(
        &format!("refs/branchdesk/checkpoints/push-{}", record.id),
        new,
        false,
        "push: preserve selected commit",
    )
    .map_err(err)?;
    let after_pin = session.snapshot()?;
    if push::selection(session, &after_pin.revision, &selected.remote, branch)? != *selected {
        return Err("STALE_PUSH_SELECTION".into());
    }
    save(&session.root, &record, None)?;
    Ok(record)
}
pub fn finish(root: &Path, id: &str, status: Status, detail: Option<String>) -> Result<Record> {
    let _lease = lease::OperationLease::acquire(root)?;
    let before = bytes(root)?.ok_or("NO_PUSH_RECORD")?;
    let mut record = read(root)?.ok_or("NO_PUSH_RECORD")?;
    if record.id != id || record.status != Status::Pending || status == Status::Pending {
        return Err("STALE_PUSH_RECORD".into());
    }
    record.status = status;
    record.detail = detail;
    save(root, &record, Some(&before))?;
    Ok(record)
}
fn observation_kind(record: &Record, oid: Option<&str>) -> ObservationKind {
    match oid {
        Some(value) if value == record.new_oid => ObservationKind::TargetPresent,
        Some(value) if Some(value) == record.old_oid.as_deref() => ObservationKind::PreviousTarget,
        Some(_) => ObservationKind::DifferentTarget,
        None => ObservationKind::Missing,
    }
}
pub fn observe(
    root: &Path,
    id: &str,
    advertisement: &crate::network_worker::Advertisement,
) -> Result<Record> {
    let _lease = lease::OperationLease::acquire(root)?;
    let before = bytes(root)?.ok_or("NO_PUSH_RECORD")?;
    let mut record = read(root)?.ok_or("NO_PUSH_RECORD")?;
    if record.id != id || record.status != Status::Pending {
        return Err("STALE_PUSH_RECORD".into());
    }
    if !crate::network_worker::valid_advertisement(advertisement, &record.endpoint) {
        return Err("PUSH_ADVERTISEMENT_INVALID".into());
    }
    let remote = advertisement
        .refs
        .iter()
        .find(|r| r.name == record.destination);
    if remote.is_some_and(|r| r.symbolic_target.is_some()) {
        return Err("PUSH_SYMBOLIC_TARGET_UNSUPPORTED".into());
    }
    let oid = remote.map(|r| r.oid.clone());
    record.observation = Some(Observation {
        kind: observation_kind(&record, oid.as_deref()),
        oid,
        checked_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(err)?
            .as_secs(),
    });
    save(root, &record, Some(&before))?;
    Ok(record)
}
pub fn acknowledge_observed(root: &Path, id: &str) -> Result<()> {
    archive(root, id, true)
}
pub fn acknowledge(root: &Path, id: &str) -> Result<()> {
    archive(root, id, false)
}
fn archive(root: &Path, id: &str, allow_observed: bool) -> Result<()> {
    let _lease = lease::OperationLease::acquire(root)?;
    let record = read(root)?.ok_or("NO_PUSH_RECORD")?;
    if record.id != id {
        return Err("STALE_PUSH_RECORD".into());
    }
    if record.status == Status::Pending && !(allow_observed && record.observation.is_some()) {
        return Err("PUSH_RESULT_UNCONFIRMED".into());
    }
    let archive = root.join(format!(".git/branchdesk-push-{}.archived", record.id));
    if archive.symlink_metadata().is_ok() {
        return Err("PUSH_ARCHIVE_EXISTS".into());
    }
    fs::rename(root.join(FILE), archive).map_err(err)?;
    checkout::sync_dir(&root.join(".git"))?;
    Ok(())
}
fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_PUSH_STATE_ROOT")
        .map(PathBuf::from)
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_PUSH_STATE_STEP").ok() == Some(step.to_string())
    {
        fs::write(root.join(".git/push-state.ready"), b"ready").unwrap();
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
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let sig = git2::Signature::now("State fixture", "state@invalid").unwrap();
        let mut b = repo.treebuilder(None).unwrap();
        b.insert("file", repo.blob(b"unchanged").unwrap(), 0o100644)
            .unwrap();
        let tree = repo.find_tree(b.write().unwrap()).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "root", &tree, &[])
            .unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        repo.remote("origin", "https://example.com/push.git")
            .unwrap();
        d
    }
    fn start(root: &Path) -> Record {
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        let selected = push::selection(&mut session, &snap.revision, "origin", "main").unwrap();
        begin(&mut session, &snap.revision, &selected, None).unwrap()
    }
    #[test]
    fn restart_retains_frozen_intent_and_acknowledgement_preserves_live_data() {
        let d = fixture();
        let root = d.path().canonicalize().unwrap();
        let before = [".git/HEAD", ".git/index", ".git/config", "file"]
            .map(|p| fs::read(root.join(p)).unwrap());
        let record = start(&root);
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        assert_eq!(snap.push_record.unwrap().id, record.id);
        assert_eq!(read(&root).unwrap().unwrap().status, Status::Pending);
        assert!(
            push::selection(&mut session, &snap.revision, "origin", "main")
                .unwrap_err()
                .contains("PUSH_PENDING_RESULT")
        );
        assert!(acknowledge(&root, &record.id)
            .unwrap_err()
            .contains("UNCONFIRMED"));
        assert!(finish(&root, "wrong", Status::Accepted, None).is_err());
        assert!(finish(&root, &record.id, Status::Rejected, None).is_err());
        let result = finish(
            &root,
            &record.id,
            Status::Rejected,
            Some("server policy".into()),
        )
        .unwrap();
        assert_eq!(result.status, Status::Rejected);
        assert!(finish(&root, &record.id, Status::Accepted, None).is_err());
        acknowledge(&root, &record.id).unwrap();
        assert!(read(&root).unwrap().is_none());
        assert!(root
            .join(format!(".git/branchdesk-push-{}.archived", record.id))
            .exists());
        assert_eq!(
            before,
            [".git/HEAD", ".git/index", ".git/config", "file"]
                .map(|p| fs::read(root.join(p)).unwrap())
        );
        fs::write(root.join(FILE), b"{bad}").unwrap();
        let snap = session.snapshot().unwrap();
        assert!(snap.push_record_error.is_some());
        assert!(push::selection(&mut session, &snap.revision, "origin", "main").is_err());
    }
    #[test]
    fn source_pin_survives_branch_deletion_and_gc() {
        let d = fixture();
        let root = d.path().canonicalize().unwrap();
        let record = start(&root);
        let repo = Repository::open(&root).unwrap();
        repo.find_reference("refs/heads/main")
            .unwrap()
            .delete()
            .unwrap();
        let out = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "core.hooksPath=/dev/null", "gc", "--prune=now"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(Repository::open(&root)
            .unwrap()
            .find_commit(Oid::from_str(&record.new_oid).unwrap())
            .is_ok());
        assert_eq!(read(&root).unwrap().unwrap().status, Status::Pending);
    }
    #[test]
    fn push_state_child() {
        let Some(root) = std::env::var_os("BRANCHDESK_PUSH_STATE_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        if std::env::var("BRANCHDESK_PUSH_STATE_JOB").unwrap() == "begin" {
            start(&root);
        } else {
            let record = read(&root).unwrap().unwrap();
            if std::env::var("BRANCHDESK_PUSH_STATE_JOB").unwrap() == "observe" {
                let ad = crate::network_worker::Advertisement {
                    endpoint: record.endpoint.clone(),
                    refs: vec![crate::network_worker::RemoteRef {
                        name: record.destination.clone(),
                        oid: record.new_oid.clone(),
                        symbolic_target: None,
                    }],
                };
                observe(&root, &record.id, &ad).unwrap();
            } else {
                finish(&root, &record.id, Status::Accepted, None).unwrap();
            }
        }
    }
    #[test]
    fn actual_kills_leave_no_intent_or_pending_before_send_and_pending_or_confirmed_after_result() {
        for job in ["begin", "finish", "observe"] {
            for step in 0..2 {
                let d = fixture();
                let root = d.path().canonicalize().unwrap();
                if job != "begin" {
                    start(&root);
                }
                let before =
                    [".git/HEAD", ".git/index", "file"].map(|p| fs::read(root.join(p)).unwrap());
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "repository::push_state::tests::push_state_child",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_PUSH_STATE_ROOT", &root)
                    .env("BRANCHDESK_PUSH_STATE_JOB", job)
                    .env("BRANCHDESK_PUSH_STATE_STEP", step.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(20);
                while !root.join(".git/push-state.ready").exists() {
                    assert!(child.try_wait().unwrap().is_none(), "child exited");
                    assert!(Instant::now() < deadline, "checkpoint timed out");
                    std::thread::sleep(Duration::from_millis(10));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let actual = read(&root).unwrap();
                if job == "observe" {
                    assert_eq!(actual.as_ref().unwrap().observation.is_some(), step == 1);
                }
                if job == "begin" && step == 0 {
                    assert!(actual.is_none());
                } else {
                    assert_eq!(
                        actual.unwrap().status,
                        if job == "finish" && step == 1 {
                            Status::Accepted
                        } else {
                            Status::Pending
                        }
                    );
                }
                assert_eq!(
                    before,
                    [".git/HEAD", ".git/index", "file"].map(|p| fs::read(root.join(p)).unwrap())
                );
                let _lease = lease::OperationLease::acquire(&root).unwrap();
            }
        }
    }
    #[test]
    fn remote_observations_never_claim_acknowledgement_and_explicit_archive_preserves_uncertainty()
    {
        let d = fixture();
        let root = d.path().canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        let base = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("test", "test@invalid").unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "child",
            &base.tree().unwrap(),
            &[&base],
        )
        .unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let selected = push::selection(&mut session, &snap.revision, "origin", "main").unwrap();
        let record = begin(
            &mut session,
            &snap.revision,
            &selected,
            Some(base.id().to_string()),
        )
        .unwrap();
        assert!(acknowledge_observed(&root, &record.id).is_err());
        let before = [".git/HEAD", ".git/index", ".git/config", "file"]
            .map(|p| fs::read(root.join(p)).unwrap());
        for (oid, kind) in [
            (Some(record.new_oid.clone()), ObservationKind::TargetPresent),
            (record.old_oid.clone(), ObservationKind::PreviousTarget),
            (Some("3".repeat(40)), ObservationKind::DifferentTarget),
            (None, ObservationKind::Missing),
        ] {
            let ad = crate::network_worker::Advertisement {
                endpoint: record.endpoint.clone(),
                refs: oid
                    .map(|id| crate::network_worker::RemoteRef {
                        name: record.destination.clone(),
                        oid: id,
                        symbolic_target: None,
                    })
                    .into_iter()
                    .collect(),
            };
            let observed = observe(&root, &record.id, &ad).unwrap();
            assert_eq!(observed.status, Status::Pending);
            assert_eq!(observed.observation.unwrap().kind, kind);
            assert_eq!(read(&root).unwrap().unwrap().status, Status::Pending);
            assert!(acknowledge(&root, &record.id).is_err());
        }
        let saved = bytes(&root).unwrap();
        let wrong = crate::network_worker::Advertisement {
            endpoint: "https://other.example/r".into(),
            refs: vec![],
        };
        assert!(observe(&root, &record.id, &wrong).is_err());
        assert_eq!(bytes(&root).unwrap(), saved);
        acknowledge_observed(&root, &record.id).unwrap();
        let archived: Record = serde_json::from_slice(
            &fs::read(root.join(format!(".git/branchdesk-push-{}.archived", record.id))).unwrap(),
        )
        .unwrap();
        assert_eq!(archived.status, Status::Pending);
        assert!(archived.observation.is_some());
        assert!(!exists(&root));
        assert_eq!(
            before,
            [".git/HEAD", ".git/index", ".git/config", "file"]
                .map(|p| fs::read(root.join(p)).unwrap())
        );
    }
}
