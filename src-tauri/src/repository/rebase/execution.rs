use super::*;
type Objects = Vec<(Oid, git2::ObjectType, Vec<u8>)>;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::repository) struct Record {
    pub(super) upstream: String,
    pub(super) upstream_oid: String,
    pub(super) name: String,
    pub(super) email: String,
    pub(super) seconds: i64,
    pub(super) offset: i32,
    #[serde(default)]
    pub(super) resolutions: HashMap<String, HashMap<String, merge_draft::Resolution>>,
    #[serde(default)]
    pub(super) skipped: Vec<String>,
}
pub(in crate::repository) struct Prepared {
    pub oid: Oid,
    pub head: Oid,
    pub record: Record,
    pub objects: Objects,
}
pub(super) fn identity(record: &Record) -> Result<git2::Signature<'static>> {
    if record.name.trim().is_empty()
        || record.email.trim().is_empty()
        || record.name.len() > 256
        || record.email.len() > 256
        || [&record.name, &record.email]
            .iter()
            .any(|s| s.chars().any(|c| c.is_control() || matches!(c, '<' | '>')))
        || record.offset.unsigned_abs() > 1439
        || record.seconds < 0
    {
        return Err("INVALID_REBASE_IDENTITY".into());
    }
    merge::reference_name(&record.upstream)?;
    git2::Signature::new(
        &record.name,
        &record.email,
        &git2::Time::new(record.seconds, record.offset),
    )
    .map_err(err)
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub id: String,
    pub path: String,
    pub ancestor: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    pub binary: bool,
    pub present: [bool; 3],
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub source: String,
    pub ordinal: usize,
    pub conflicts: Vec<Conflict>,
    pub resolutions: HashMap<String, merge_draft::Resolution>,
}
pub(super) struct Calculation {
    pub oid: Oid,
    pub objects: Objects,
    pub pending: Option<Pending>,
}
fn build(repo: &Repository, old: Oid, record: &Record) -> Result<(Oid, Objects)> {
    let result = calculate(repo, old, record)?;
    if result.pending.is_some() {
        return Err("REBASE_CONFLICTS_REQUIRE_RESOLUTION".into());
    }
    Ok((result.oid, result.objects))
}
fn conflict_preview(
    repo: &Repository,
    index: &git2::Index,
    resolutions: &HashMap<String, merge_draft::Resolution>,
) -> Result<Vec<Conflict>> {
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for entry in index.conflicts().map_err(err)? {
        let entry = entry.map_err(err)?;
        let versions = [entry.ancestor, entry.our, entry.their];
        let paths = versions
            .iter()
            .flatten()
            .map(|e| e.path.as_slice())
            .collect::<HashSet<_>>();
        if paths.len() != 1 {
            return Err("UNSUPPORTED_RENAME_CONFLICT".into());
        }
        let path = String::from_utf8(paths.into_iter().next().unwrap().to_vec()).map_err(err)?;
        let mut binary = false;
        let mut values = Vec::new();
        for entry in &versions {
            let text = if let Some(entry) = entry {
                let blob = repo.find_blob(entry.id).map_err(err)?;
                bytes = bytes
                    .checked_add(blob.size())
                    .ok_or("LIMIT: rebase conflict bytes")?;
                if bytes > 20 * 1024 * 1024 {
                    return Err("LIMIT: rebase conflict bytes".into());
                }
                if blob.is_binary() {
                    binary = true;
                    None
                } else {
                    match std::str::from_utf8(blob.content()) {
                        Ok(s) => Some(s.to_owned()),
                        Err(_) => {
                            binary = true;
                            None
                        }
                    }
                }
            } else {
                None
            };
            values.push(text);
        }
        out.push(Conflict {
            id: file_id(&path),
            path,
            ancestor: values.remove(0),
            ours: values.remove(0),
            theirs: values.remove(0),
            binary,
            present: [
                versions[0].is_some(),
                versions[1].is_some(),
                versions[2].is_some(),
            ],
        });
        if out.len() > MAX_FILES {
            return Err("LIMIT: rebase conflicts".into());
        }
    }
    if resolutions
        .keys()
        .any(|id| !out.iter().any(|c| &c.id == id))
    {
        return Err("STALE_REBASE_RESOLUTION".into());
    }
    for r in resolutions.values() {
        replay_draft::validate_resolution(r)?;
    }
    Ok(out)
}
pub(super) fn calculate(repo: &Repository, old: Oid, record: &Record) -> Result<Calculation> {
    let committer = identity(record)?;
    let onto = Oid::from_str(&record.upstream_oid).map_err(err)?;
    let target = repo.find_commit(onto).map_err(err)?;
    repo.merge_base(old, onto)
        .map_err(|_| "UNRELATED_HISTORIES")?;
    if old == onto || linear_to(repo, old, onto)? {
        if !record.resolutions.is_empty() || !record.skipped.is_empty() {
            return Err("INVALID_REBASE_DECISIONS".into());
        }
        return Ok(Calculation {
            oid: old,
            objects: vec![],
            pending: None,
        });
    }
    if repo.graph_descendant_of(onto, old).map_err(err)? {
        if !record.resolutions.is_empty() || !record.skipped.is_empty() {
            return Err("INVALID_REBASE_DECISIONS".into());
        }
        return Ok(Calculation {
            oid: onto,
            objects: vec![],
            pending: None,
        });
    }
    let patches = commits(repo, onto, old)?
        .into_iter()
        .map(|id| patch(repo, id))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<HashSet<_>>();
    let sources = commits(repo, old, onto)?
        .into_iter()
        .filter_map(|id| match repo.find_commit(id) {
            Ok(c) if c.parent_count() > 1 => None,
            Ok(_) => Some(Ok(id)),
            Err(e) => Some(Err(err(e))),
        })
        .collect::<Result<Vec<_>>>()?;
    let valid = sources
        .iter()
        .filter_map(|id| match patch(repo, *id) {
            Ok(Some(p)) if patches.contains(&p) => None,
            _ => Some(id.to_string()),
        })
        .collect::<HashSet<_>>();
    if record.resolutions.len() > replay_sequence::MAX_STEPS
        || record.skipped.len() > replay_sequence::MAX_STEPS
        || record.skipped.iter().collect::<HashSet<_>>().len() != record.skipped.len()
        || record
            .resolutions
            .keys()
            .chain(record.skipped.iter())
            .any(|id| !valid.contains(id))
        || record
            .skipped
            .iter()
            .any(|id| record.resolutions.contains_key(id))
    {
        return Err("INVALID_REBASE_DECISIONS".into());
    }
    let temporary = tempfile::tempdir().map_err(err)?;
    let mut opts = git2::RepositoryInitOptions::new();
    opts.bare(true).external_template(false);
    let scratch = Repository::init_opts(temporary.path(), &opts).map_err(err)?;
    scratch
        .set_config(&Config::new().map_err(err)?)
        .map_err(err)?;
    scratch
        .odb()
        .map_err(err)?
        .add_disk_alternate(
            repo.path()
                .join("objects")
                .to_str()
                .ok_or("UNSUPPORTED_PATH")?,
        )
        .map_err(err)?;
    let mut current = target.id();
    let mut count = 0;
    for oid in sources {
        if patch(repo, oid)?.is_some_and(|p| patches.contains(&p)) {
            continue;
        }
        count += 1;
        if count > replay_sequence::MAX_STEPS {
            return Err("LIMIT: rebase steps".into());
        }
        if record.skipped.contains(&oid.to_string()) {
            continue;
        }
        let source = scratch.find_commit(oid).map_err(err)?;
        if source.parent_count() != 1 {
            return Err("REBASE_ROOT_UNSUPPORTED".into());
        }
        let previous = source.parent(0).map_err(err)?.tree().map_err(err)?;
        let selected = source.tree().map_err(err)?;
        let parent = scratch.find_commit(current).map_err(err)?;
        let ours = parent.tree().map_err(err)?;
        let mut index = scratch
            .merge_trees(&previous, &ours, &selected, None)
            .map_err(err)?;
        let resolutions = record
            .resolutions
            .get(&oid.to_string())
            .cloned()
            .unwrap_or_default();
        let resolved_conflict = index.has_conflicts();
        if resolved_conflict {
            let conflicts = conflict_preview(&scratch, &index, &resolutions)?;
            if resolutions.len() != conflicts.len() {
                return Ok(Calculation {
                    oid: current,
                    objects: vec![],
                    pending: Some(Pending {
                        source: oid.to_string(),
                        ordinal: count - 1,
                        conflicts,
                        resolutions,
                    }),
                });
            }
            merge_draft::resolve_index(&scratch, &mut index, &resolutions)?;
        } else if !resolutions.is_empty() {
            return Err("STALE_REBASE_RESOLUTION".into());
        }
        let tree = scratch
            .find_tree(index.write_tree_to(&scratch).map_err(err)?)
            .map_err(err)?;
        checkout::inspect_tree(&scratch, &tree)?;
        if tree.id() == ours.id() && selected.id() != previous.id() {
            continue;
        }
        let message = source
            .message()
            .map_err(|_| "UNSUPPORTED_COMMIT_MESSAGE_ENCODING")?;
        if message.len() > 10000 {
            return Err("LIMIT: rebase commit message".into());
        }
        let normalized;
        let message = if resolved_conflict {
            normalized = git2::message_prettify(message, None).map_err(err)?;
            normalized.as_str()
        } else {
            message
        };
        current = scratch
            .commit(
                None,
                &source.author(),
                &committer,
                message,
                &tree,
                &[&parent],
            )
            .map_err(err)?;
    }
    let source = repo.odb().map_err(err)?;
    let private = scratch.odb().map_err(err)?;
    let mut pending = vec![current];
    let mut seen = HashSet::new();
    let mut objects = Vec::new();
    let mut bytes = 0usize;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) || source.exists(id) {
            continue;
        }
        if seen.len() > 100_000 {
            return Err("LIMIT: rebase objects".into());
        }
        let object = private.read(id).map_err(err)?;
        bytes = bytes
            .checked_add(object.data().len())
            .ok_or("LIMIT: rebase bytes")?;
        if bytes > 50 * 1024 * 1024 {
            return Err("LIMIT: rebase bytes".into());
        }
        match object.kind() {
            git2::ObjectType::Commit => {
                let c = scratch.find_commit(id).map_err(err)?;
                pending.push(c.tree_id());
                pending.extend(c.parent_ids());
            }
            git2::ObjectType::Tree => {
                for e in scratch.find_tree(id).map_err(err)?.iter() {
                    pending.push(e.id());
                }
            }
            _ => {}
        }
        objects.push((id, object.kind(), object.data().to_vec()));
    }
    Ok(Calculation {
        oid: current,
        objects,
        pending: None,
    })
}
pub(in crate::repository) fn prepare(
    session: &mut Session,
    revision: &str,
    upstream: &str,
    name: &str,
    email: &str,
) -> Result<Prepared> {
    let plan = session.rebase_preview(revision, upstream)?;
    if !plan.blockers.is_empty() {
        return Err(format!("REBASE_BLOCKED: {}", plan.blockers.join(", ")));
    }
    let time = git2::Signature::now("FLOE", "local@floe.invalid")
        .map_err(err)?
        .when();
    let record = Record {
        upstream: upstream.into(),
        upstream_oid: plan.upstream_oid,
        name: name.into(),
        email: email.into(),
        seconds: time.seconds(),
        offset: time.offset_minutes(),
        resolutions: HashMap::new(),
        skipped: Vec::new(),
    };
    let head = Oid::from_str(&plan.head_oid).map_err(err)?;
    let repo = open(&session.root)?;
    let (oid, objects) = build(&repo, head, &record)?;
    session.fresh(revision)?;
    Ok(Prepared {
        oid,
        head,
        record,
        objects,
    })
}
pub(in crate::repository) fn verify(
    repo: &Repository,
    old: Oid,
    new: Oid,
    record: &Record,
) -> Result<()> {
    if build(repo, old, record)?.0 != new {
        return Err("INVALID_REBASE_JOURNAL_RESULT".into());
    }
    Ok(())
}
