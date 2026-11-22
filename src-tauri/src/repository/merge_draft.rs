//! Durable conflict resolutions. Live worktree/index remain untouched until Continue.
use super::*;
const FILE: &str = ".git/branchdesk-merge-draft.json";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Resolution {
    Ours {},
    Theirs {},
    Delete {},
    Text { text: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    version: u32,
    pub name: String,
    pub branch: String,
    pub head: String,
    pub target: String,
    pub resolutions: HashMap<String, Resolution>,
}
pub(super) fn exists(root: &Path) -> bool {
    root.join(FILE).exists()
}
pub(super) fn read(root: &Path) -> Result<Option<Draft>> {
    if !exists(root) {
        return Ok(None);
    }
    let d: Draft =
        serde_json::from_slice(&bounded(&root.join(FILE), 20 * 1024 * 1024)?).map_err(err)?;
    if d.version != 1 || d.resolutions.len() > MAX_FILES {
        return Err("INVALID_MERGE_DRAFT".into());
    }
    Ok(Some(d))
}
fn write(root: &Path, draft: &Draft) -> Result<()> {
    let bytes = serde_json::to_vec(draft).map_err(err)?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("LIMIT: merge draft 20 MiB".into());
    }
    let temp = root.join(format!(
        ".git/branchdesk-merge-draft-{}.next",
        Uuid::new_v4()
    ));
    checkpoint(root, "write", 0);
    checkout::save(&temp, &bytes)?;
    checkpoint(root, "write", 1);
    fs::rename(temp, root.join(FILE)).map_err(err)?;
    checkpoint(root, "write", 2);
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "write", 3);
    Ok(())
}
pub(super) fn start(session: &mut Session, revision: &str, name: &str) -> Result<()> {
    if exists(&session.root) {
        return Err("MERGE_DRAFT_EXISTS".into());
    }
    let plan = merge::preview(session, revision, name)?;
    let snapshot = session.fresh(revision)?;
    if plan.kind != "conflicts" || !snapshot.changes.is_empty() || snapshot.write_block.is_some() {
        return Err("MERGE_DRAFT_REQUIRES_CLEAN_CONFLICTED_PLAN".into());
    }
    write(
        &session.root,
        &Draft {
            version: 1,
            name: name.into(),
            branch: snapshot.branch,
            head: plan.head_oid,
            target: plan.target_oid,
            resolutions: HashMap::new(),
        },
    )
}
pub(super) fn save_resolution(
    session: &mut Session,
    revision: &str,
    id: &str,
    resolution: Resolution,
) -> Result<()> {
    let mut draft = read(&session.root)?.ok_or("NO_MERGE_DRAFT")?;
    let plan = merge::preview(session, revision, &draft.name)?;
    if plan.head_oid != draft.head
        || plan.target_oid != draft.target
        || plan.current_branch != draft.branch
    {
        return Err("STALE_MERGE_DRAFT".into());
    }
    let c = plan
        .conflicts
        .into_iter()
        .find(|c| {
            [&c.ancestor, &c.ours, &c.theirs]
                .into_iter()
                .flatten()
                .any(|v| v.id == id)
        })
        .ok_or("INVALID_MERGE_CONFLICT")?;
    let paths: std::collections::HashSet<_> = [&c.ancestor, &c.ours, &c.theirs]
        .into_iter()
        .flatten()
        .map(|v| &v.path)
        .collect();
    if paths.len() != 1 {
        return Err("UNSUPPORTED_RENAME_CONFLICT".into());
    }
    if let Resolution::Text { text } = &resolution {
        if text.len() > FILE_LIMIT as usize || text.contains('\0') {
            return Err("INVALID_RESOLUTION_TEXT".into());
        }
    }
    draft.resolutions.insert(id.into(), resolution);
    write(&session.root, &draft)
}
pub(super) fn archive(root: &Path) -> Result<()> {
    if !exists(root) {
        return Err("NO_MERGE_DRAFT".into());
    }
    checkpoint(root, "archive", 0);
    fs::rename(
        root.join(FILE),
        root.join(format!(
            ".git/branchdesk-merge-draft-completed-{}.json",
            Uuid::new_v4()
        )),
    )
    .map_err(err)?;
    checkpoint(root, "archive", 1);
    checkout::sync_dir(&root.join(".git"))?;
    checkpoint(root, "archive", 2);
    Ok(())
}
pub(super) fn apply(
    root: &Path,
    repo: &Repository,
    head: Oid,
    target: Oid,
    index: &mut git2::Index,
) -> Result<()> {
    let draft = read(root)?.ok_or("MERGE_CONFLICT: resolution required")?;
    if draft.head != head.to_string()
        || draft.target != target.to_string()
        || repo.head().map_err(err)?.shorthand().map_err(err)? != draft.branch
    {
        return Err("STALE_MERGE_DRAFT".into());
    }
    resolve_index(repo, index, &draft.resolutions)
}
pub(super) fn resolve_index(
    repo: &Repository,
    index: &mut git2::Index,
    resolutions: &HashMap<String, Resolution>,
) -> Result<()> {
    let conflicts = index
        .conflicts()
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    if conflicts.len() != resolutions.len() {
        return Err("UNRESOLVED_MERGE_CONFLICTS".into());
    }
    for c in conflicts {
        let paths: std::collections::HashSet<_> = [&c.ancestor, &c.our, &c.their]
            .into_iter()
            .flatten()
            .map(|e| e.path.clone())
            .collect();
        if paths.len() != 1 {
            return Err("UNSUPPORTED_RENAME_CONFLICT".into());
        }
        let path =
            String::from_utf8(paths.into_iter().next().ok_or("INVALID_CONFLICT")?).map_err(err)?;
        let resolution = resolutions
            .get(&file_id(&path))
            .ok_or("UNRESOLVED_MERGE_CONFLICTS")?;
        let mut chosen = match resolution {
            Resolution::Ours {} => c.our,
            Resolution::Theirs {} => c.their,
            Resolution::Delete {} => None,
            Resolution::Text { text } => {
                if text.len() > FILE_LIMIT as usize || text.contains('\0') {
                    return Err("INVALID_RESOLUTION_TEXT".into());
                }
                let mut entry = c.our.or(c.their).or(c.ancestor).ok_or("INVALID_CONFLICT")?;
                entry.id = repo.blob(text.as_bytes()).map_err(err)?;
                entry.file_size = text.len() as u32;
                Some(entry)
            }
        };
        index.conflict_remove(Path::new(&path)).map_err(err)?;
        if let Some(ref mut entry) = chosen {
            entry.flags &= !0x3000; // stage zero; preserve path-length flags
            index.add(entry).map_err(err)?;
        }
    }
    if index.has_conflicts() {
        return Err("UNRESOLVED_MERGE_CONFLICTS".into());
    }
    Ok(())
}

fn checkpoint(root: &Path, kind: &str, step: usize) {
    #[cfg(test)]
    if std::env::var_os("BRANCHDESK_DRAFT_ROOT")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .as_deref()
        == Some(root)
        && std::env::var("BRANCHDESK_DRAFT_CHECKPOINT").ok().as_deref()
            == Some(format!("{kind}:{step}").as_str())
    {
        fs::write(root.join(".git/branchdesk-draft-test.ready"), "ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    #[cfg(not(test))]
    let _ = (root, kind, step);
}
