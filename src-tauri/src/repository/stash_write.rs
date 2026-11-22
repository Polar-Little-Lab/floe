//! Stash drop uses a durable roll-forward record: reflog and ref are not atomic.
use super::*;
use std::io::Write;
const DIR: &str = ".git/branchdesk-stash-drop";
const LOG: &str = ".git/logs/refs/stash";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    index: usize,
    before_tip: String,
    after_tip: Option<String>,
    before_log: Vec<u8>,
    after_log: Vec<u8>,
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(DIR).exists()
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}
fn sync(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path).map_err(err)?.sync_all().map_err(err)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn tip(repo: &Repository) -> Result<Option<String>> {
    match repo.refname_to_id("refs/stash") {
        Ok(oid) => Ok(Some(oid.to_string())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
fn read_log(root: &Path) -> Result<Vec<u8>> {
    if !root.join(LOG).exists() {
        return Ok(Vec::new());
    }
    bounded(&root.join(LOG), FILE_LIMIT)
}
// Preserve original signature/message bytes. Git's reflog is oldest-first on disk.
fn remove_entry(bytes: &[u8], index: usize) -> Result<Vec<u8>> {
    if !bytes.ends_with(b"\n") {
        return Err("INVALID_STASH_LOG".into());
    }
    let mut lines: Vec<Vec<u8>> = bytes
        .split_inclusive(|b| *b == b'\n')
        .map(Vec::from)
        .collect();
    if index >= lines.len() || lines.len() > MAX_FILES {
        return Err("INVALID_STASH_INDEX".into());
    }
    for line in &lines {
        if line.len() < 83
            || line[40] != b' '
            || line[81] != b' '
            || !line[..40]
                .iter()
                .chain(&line[41..81])
                .all(u8::is_ascii_hexdigit)
        {
            return Err("INVALID_STASH_LOG".into());
        }
    }
    let pos = lines.len() - 1 - index;
    lines.remove(pos);
    if pos < lines.len() {
        let previous = if pos == 0 {
            vec![b'0'; 40]
        } else {
            lines[pos - 1][41..81].to_vec()
        };
        lines[pos][..40].copy_from_slice(&previous);
    }
    Ok(lines.concat())
}
pub(super) fn drop_stash(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    oid: &str,
    fail: Option<usize>,
) -> Result<()> {
    let parsed = Oid::from_str(oid).map_err(err)?;
    let mut tx = repo.transaction().map_err(err)?;
    tx.lock_ref("refs/stash").map_err(err)?;
    let snap = session.fresh(revision)?;
    let selected: Vec<_> = snap
        .stashes
        .iter()
        .filter(|s| s.oid == parsed.to_string())
        .collect();
    if selected.len() != 1 {
        return Err("AMBIGUOUS_OR_MISSING_STASH".into());
    }
    if tip(repo)? != snap.stashes.first().map(|s| s.oid.clone()) {
        return Err("INCONSISTENT_STASH_REFLOG".into());
    }
    let before_log = read_log(&session.root)?;
    let after_log = remove_entry(&before_log, selected[0].index)?;
    let after_tip = snap
        .stashes
        .iter()
        .find(|s| s.index != selected[0].index)
        .map(|s| s.oid.clone());
    let record = Record {
        version: 1,
        index: selected[0].index,
        before_tip: tip(repo)?.ok_or("MISSING_STASH")?,
        after_tip,
        before_log,
        after_log,
    };
    let dir = session.root.join(DIR);
    fs::create_dir(&dir).map_err(err)?;
    save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    sync(&dir)?;
    sync(&session.root.join(".git"))?;
    journal_locks::record(&session.root, &dir, None, true, false)?;
    journal_locks::checkpoint(&session.root, "drop", 0);
    if fail == Some(0) {
        return Err("INJECTED_INTERRUPTION".into());
    }
    finish(&session.root, repo, tx, &record, fail, false)
}
pub(super) fn recover(root: &Path, repo: &Repository) -> Result<()> {
    let record: Record = serde_json::from_slice(&bounded(
        &root.join(DIR).join("record.json"),
        40 * 1024 * 1024,
    )?)
    .map_err(err)?;
    if record.version != 1
        || record.before_log.len() as u64 > FILE_LIMIT
        || record.after_log.len() as u64 > FILE_LIMIT
    {
        return Err("INVALID_STASH_JOURNAL".into());
    }
    // Only a single removed entry is a valid planned transformation.
    if remove_entry(&record.before_log, record.index)? != record.after_log {
        return Err("INVALID_STASH_JOURNAL".into());
    }
    let log_tip = |bytes: &[u8]| -> Result<Option<String>> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let line = bytes
            .split_inclusive(|b| *b == b'\n')
            .next_back()
            .ok_or("INVALID_STASH_LOG")?;
        let value =
            std::str::from_utf8(line.get(41..81).ok_or("INVALID_STASH_LOG")?).map_err(err)?;
        Ok(Some(Oid::from_str(value).map_err(err)?.to_string()))
    };
    if log_tip(&record.before_log)? != Some(record.before_tip.clone())
        || log_tip(&record.after_log)? != record.after_tip
    {
        return Err("INVALID_STASH_JOURNAL".into());
    }
    let mut tx = repo.transaction().map_err(err)?;
    tx.lock_ref("refs/stash").map_err(err)?;
    journal_locks::record(root, &root.join(DIR), None, true, false)?;
    journal_locks::checkpoint(root, "drop-recover", 0);
    finish(root, repo, tx, &record, None, true)
}
fn finish(
    root: &Path,
    repo: &Repository,
    mut tx: git2::Transaction<'_>,
    record: &Record,
    fail: Option<usize>,
    recovering: bool,
) -> Result<()> {
    let current = tip(repo)?;
    if current != Some(record.before_tip.clone()) && current != record.after_tip {
        return Err("RECOVERY_CONFLICT: stash ref changed".into());
    }
    let log = read_log(root)?;
    if log != record.before_log && log != record.after_log {
        return Err("RECOVERY_CONFLICT: stash log changed".into());
    }
    // Reflog lock interoperates with Git. Never unlink an existing foreign lock.
    let log_lock = root.join(".git/logs/refs/stash.lock");
    save(&log_lock, &record.after_log)?;
    journal_locks::record(root, &root.join(DIR), None, true, true)?;
    journal_locks::checkpoint(root, if recovering { "drop-recover" } else { "drop" }, 10);
    if let Err(e) = fs::rename(&log_lock, root.join(LOG)) {
        let _ = fs::remove_file(&log_lock);
        return Err(err(e));
    }
    sync(&root.join(".git/logs/refs"))?;
    journal_locks::checkpoint(root, if recovering { "drop-recover" } else { "drop" }, 1);
    if fail == Some(1) {
        return Err("INJECTED_INTERRUPTION".into());
    }
    if let Some(target) = &record.after_tip {
        // Suppress an extra reflog entry: the new log is already the intended list.
        tx.set_reflog("refs/stash", repo.reflog("refs/stash").map_err(err)?)
            .map_err(err)?;
        tx.set_target(
            "refs/stash",
            Oid::from_str(target).map_err(err)?,
            Some(&git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?),
            "stash: drop",
        )
        .map_err(err)?;
    } else if current.is_some() {
        tx.remove("refs/stash").map_err(err)?;
    }
    tx.commit().map_err(err)?;
    journal_locks::checkpoint(root, if recovering { "drop-recover" } else { "drop" }, 2);
    if fail == Some(2) {
        return Err("INJECTED_INTERRUPTION".into());
    }
    if tip(repo)? != record.after_tip || read_log(root)? != record.after_log {
        return Err("STASH_RESULT_MISMATCH".into());
    }
    fs::rename(
        root.join(DIR),
        root.join(".git")
            .join(format!("branchdesk-stash-completed-{}", Uuid::new_v4())),
    )
    .map_err(err)?;
    sync(&root.join(".git"))
}
