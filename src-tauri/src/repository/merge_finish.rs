//! Roll-forward completion of an external merge: commit reference then guarded metadata cleanup.
use super::*;
const JOURNAL: &str = ".git/branchdesk-merge-finish";
const META: [&str; 3] = ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE"];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    branch: String,
    old: String,
    new: String,
    index: String,
    files: Vec<Option<Vec<u8>>>,
    auto_merge: Option<String>,
}
pub(super) fn pending(root: &Path) -> bool {
    root.join(JOURNAL).exists()
}
pub(super) fn locks<'a>(repo: &'a Repository, branch: &str) -> Result<git2::Transaction<'a>> {
    if !branch.starts_with("refs/heads/") || !git2::Reference::is_valid_name(branch) {
        return Err("INVALID_BRANCH".into());
    }
    let mut tx = repo.transaction().map_err(err)?;
    for name in ["AUTO_MERGE", "HEAD", branch] {
        tx.lock_ref(name).map_err(err)?;
    }
    Ok(tx)
}
pub(super) fn auto_merge(repo: &Repository) -> Result<Option<String>> {
    match repo.find_reference("AUTO_MERGE") {
        Ok(r) => Ok(Some(r.target().ok_or("INVALID_AUTO_MERGE")?.to_string())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}
pub(super) fn execute(
    session: &mut Session,
    repo: &Repository,
    revision: &str,
    input: &checkout::MergeInput<'_>,
    fail: Option<usize>,
) -> Result<()> {
    if session.root.join(".git/MERGE_AUTOSTASH").exists() {
        return Err("UNSUPPORTED_MERGE_AUTOSTASH".into());
    }
    if input.message.trim().is_empty()
        || input.message.len() > 10000
        || input.message.contains('\0')
        || input.author.trim().is_empty()
        || input.author.len() > 256
        || input.email.trim().is_empty()
        || input.email.len() > 256
    {
        return Err("INVALID_MERGE_IDENTITY_OR_MESSAGE".into());
    }
    let signature = git2::Signature::now(input.author, input.email).map_err(err)?;
    let branch = repo.head().map_err(err)?.name().map_err(err)?.to_string();
    let tx = locks(repo, &branch)?;
    session.fresh(revision)?;
    let state = external_merge::inspect(&session.root, repo)?.ok_or("NO_EXTERNAL_MERGE")?;
    if state.heads.len() != 1 || !state.conflicts.is_empty() {
        return Err("UNRESOLVED_OR_UNSUPPORTED_MERGE".into());
    }
    let old = repo.head().map_err(err)?.peel_to_commit().map_err(err)?;
    let incoming = repo
        .find_commit(Oid::from_str(&state.heads[0]).map_err(err)?)
        .map_err(err)?;
    let mut index = repo.index().map_err(err)?;
    if index.has_conflicts() {
        return Err("UNRESOLVED_MERGE_CONFLICTS".into());
    }
    let tree = repo
        .find_tree(index.write_tree_to(repo).map_err(err)?)
        .map_err(err)?;
    checkout::inspect_tree(repo, &tree)?;
    let message = if input.message.ends_with('\n') {
        input.message.to_string()
    } else {
        format!("{}\n", input.message)
    };
    let new = repo
        .commit(
            None,
            &signature,
            &signature,
            &message,
            &tree,
            &[&old, &incoming],
        )
        .map_err(err)?;
    let mut files = Vec::new();
    for name in META {
        let path = session.root.join(".git").join(name);
        files.push(if path.exists() {
            Some(bounded(&path, 1024 * 1024)?)
        } else {
            None
        });
    }
    let record = Record {
        version: 1,
        branch: branch.clone(),
        old: old.id().to_string(),
        new: new.to_string(),
        index: checkout::digest(&bounded(
            &session.root.join(".git/index"),
            20 * 1024 * 1024,
        )?),
        files,
        auto_merge: auto_merge(repo)?,
    };
    repo.reference(
        &format!("refs/branchdesk/checkpoints/{}", Uuid::new_v4()),
        new,
        false,
        "external merge recovery checkpoint",
    )
    .map_err(err)?;
    let dir = session.root.join(JOURNAL);
    fs::create_dir(&dir).map_err(err)?;
    checkout::save(
        &dir.join("record.json"),
        &serde_json::to_vec(&record).map_err(err)?,
    )?;
    journal_locks::record_merge(&session.root, &dir, &branch)?;
    checkout::sync_dir(&session.root.join(".git"))?;
    complete(&session.root, repo, tx, &record, fail)
}
fn complete(
    root: &Path,
    repo: &Repository,
    mut tx: git2::Transaction<'_>,
    record: &Record,
    fail: Option<usize>,
) -> Result<()> {
    if record.version != 1 || record.files.len() != 3 {
        return Err("INVALID_MERGE_FINISH_JOURNAL".into());
    }
    let new = repo
        .find_commit(Oid::from_str(&record.new).map_err(err)?)
        .map_err(err)?;
    let incoming =
        String::from_utf8(record.files[0].clone().ok_or("INVALID_MERGE_HEAD")?).map_err(err)?;
    if new.parent_count() != 2
        || new.parent_id(0).map_err(err)?.to_string() != record.old
        || new.parent_id(1).map_err(err)?.to_string() != incoming.trim()
    {
        return Err("INVALID_MERGE_PARENTS".into());
    }
    if repo
        .find_reference("HEAD")
        .map_err(err)?
        .symbolic_target()
        .map_err(err)?
        != Some(record.branch.as_str())
    {
        return Err("RECOVERY_CONFLICT: HEAD changed".into());
    }
    let current = repo.refname_to_id(&record.branch).map_err(err)?.to_string();
    if (current != record.old && current != record.new)
        || checkout::digest(&bounded(&root.join(".git/index"), 20 * 1024 * 1024)?) != record.index
    {
        return Err("RECOVERY_CONFLICT: ref/index changed".into());
    }
    let auto = auto_merge(repo)?;
    if auto.is_some() && auto != record.auto_merge {
        return Err("RECOVERY_CONFLICT: AUTO_MERGE changed".into());
    }
    for (name, expected) in META.into_iter().zip(&record.files) {
        let path = root.join(".git").join(name);
        if current == record.old && expected.is_some() && !path.exists() {
            return Err("RECOVERY_CONFLICT: merge metadata removed before commit".into());
        }
        if path.exists() && Some(bounded(&path, 1024 * 1024)?) != *expected {
            return Err("RECOVERY_CONFLICT: merge metadata changed".into());
        }
    }
    let injected = |step| {
        checkpoint(root, step);
        if fail == Some(step) {
            Err("INJECTED_INTERRUPTION".to_string())
        } else {
            Ok(())
        }
    };
    injected(0)?;
    if current == record.old {
        tx.set_target(
            &record.branch,
            new.id(),
            Some(&new.committer()),
            "merge: complete external merge",
        )
        .map_err(err)?;
    }
    if auto.is_some() {
        tx.remove("AUTO_MERGE").map_err(err)?;
    }
    tx.commit().map_err(err)?;
    injected(1)?;
    for (i, name) in META.into_iter().enumerate() {
        let path = root.join(".git").join(name);
        if path.exists() {
            if Some(bounded(&path, 1024 * 1024)?) != record.files[i] {
                return Err("RECOVERY_CONFLICT: merge metadata changed".into());
            }
            fs::remove_file(path).map_err(err)?;
            checkout::sync_dir(&root.join(".git"))?;
        }
        injected(i + 2)?;
    }
    Ok(())
}
pub(super) fn recover(root: &Path, repo: &Repository) -> Result<()> {
    let dir = root.join(JOURNAL);
    let record: Record =
        serde_json::from_slice(&bounded(&dir.join("record.json"), 16 * 1024 * 1024)?)
            .map_err(err)?;
    let tx = locks(repo, &record.branch)?;
    journal_locks::record_merge(root, &dir, &record.branch)?;
    complete(root, repo, tx, &record, None)
}
pub(super) fn finish(root: &Path) -> Result<()> {
    fs::rename(
        root.join(JOURNAL),
        root.join(format!(
            ".git/branchdesk-merge-finish-completed-{}",
            Uuid::new_v4()
        )),
    )
    .map_err(err)?;
    checkout::sync_dir(&root.join(".git"))
}

fn checkpoint(root: &Path, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_DRAFT_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_DRAFT_CHECKPOINT").ok().as_deref()
            == Some(format!("finish:{step}").as_str())
    {
        fs::write(root.join(".git/branchdesk-draft-test.ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, step);
}
