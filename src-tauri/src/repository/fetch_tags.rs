//! Select implicit tags before importing the full downloaded pack into the local ODB.
use super::*;
use crate::network_worker::Downloaded;
#[derive(PartialEq, Eq)]
pub(super) enum Policy {
    Auto,
    All,
    None,
}
pub(super) fn policy(values: &[(String, String)], remote: &str) -> Result<Policy> {
    match values
        .iter()
        .rev()
        .find(|(k, _)| k == &format!("remote.{remote}.tagopt"))
        .map(|(_, v)| v.as_str())
    {
        None => Ok(Policy::Auto),
        Some("--tags") => Ok(Policy::All),
        Some("--no-tags") => Ok(Policy::None),
        _ => Err("UNSUPPORTED_TAG_OPTION".into()),
    }
}
pub(super) fn implicit_allowed(values: &[(String, String)], remote: &str) -> Result<bool> {
    Ok(match policy(values, remote)? {
        Policy::None => false,
        Policy::All => true,
        Policy::Auto => fetch_plan::auto_tags_enabled(values, remote)?,
    })
}
fn reachable(repo: &Repository, tips: &[Oid]) -> Result<std::collections::HashSet<Oid>> {
    let mut seen = std::collections::HashSet::new();
    let mut pending = tips.to_vec();
    let odb = repo.odb().map_err(err)?;
    while let Some(oid) = pending.pop() {
        if !seen.insert(oid) {
            continue;
        }
        if seen.len() > 100000 || pending.len() > 1000000 {
            return Err("FETCH_TAG_GRAPH_LIMIT".into());
        }
        let (size, kind) = odb.read_header(oid).map_err(err)?;
        if size > 16 * 1024 * 1024 {
            return Err("FETCH_TAG_OBJECT_LIMIT".into());
        }
        match kind {
            git2::ObjectType::Commit => {
                let c = repo.find_commit(oid).map_err(err)?;
                pending.push(c.tree_id());
                pending.extend(c.parent_ids());
            }
            git2::ObjectType::Tree => {
                for e in repo.find_tree(oid).map_err(err)?.iter() {
                    if e.filemode() != 0o160000 {
                        pending.push(e.id());
                    }
                }
            }
            git2::ObjectType::Tag => pending.push(repo.find_tag(oid).map_err(err)?.target_id()),
            git2::ObjectType::Blob => {}
            _ => return Err("FETCH_TAG_OBJECT_INVALID".into()),
        }
    }
    Ok(seen)
}
pub(super) fn augment(
    root: &Path,
    remote: &str,
    download: &Downloaded,
    mut base: fetch_plan::Plan,
) -> Result<fetch_plan::Plan> {
    let mut values = remote_remove::entries(&bounded(&root.join(".git/config"), 65536)?)?;
    if !implicit_allowed(&values, remote)? {
        return Ok(base);
    }
    let mode = policy(&values, remote)?;
    let Some(objects) = download.object_repository()? else {
        return Ok(base);
    };
    let local = open(root)?;
    let odb = local.odb().map_err(err)?;
    let tips = base
        .mappings
        .iter()
        .map(|m| Oid::from_str(&m.after).map_err(err))
        .collect::<Result<Vec<_>>>()?;
    let graph = if mode == Policy::Auto {
        reachable(&objects, &tips)?
    } else {
        Default::default()
    };
    let mut selected = Vec::new();
    for reference in &download.advertisement().refs {
        if !reference.name.starts_with("refs/tags/")
            || reference.name.ends_with("^{}")
            || base
                .mappings
                .iter()
                .any(|m| m.destination.as_deref() == Some(&reference.name))
        {
            continue;
        }
        if mode == Policy::Auto {
            match local.find_reference(&reference.name) {
                Ok(_) => continue,
                Err(e) if e.code() == git2::ErrorCode::NotFound => {}
                Err(e) => return Err(err(e)),
            }
        }
        let oid = Oid::from_str(&reference.oid).map_err(err)?;
        let mut peeled = oid;
        let mut depth = 0;
        while let Ok(tag) = objects.find_tag(peeled) {
            depth += 1;
            if depth > 128 {
                return Err("FETCH_TAG_DEPTH_LIMIT".into());
            }
            peeled = tag.target_id();
        }
        let initial =
            odb.exists(oid) || odb.exists(peeled) || tips.contains(&oid) || tips.contains(&peeled);
        if mode == Policy::All || initial || graph.contains(&oid) || graph.contains(&peeled) {
            selected.push((
                mode != Policy::All
                    && (!initial || fetch_plan::excludes(&values, remote, &reference.name)?),
                reference.name.clone(),
            ));
        }
    }
    if selected.is_empty() {
        return Ok(base);
    }
    // Git's automatic tag backfill is a separate pass: it also follows tags
    // excluded by a negative fetch refspec. --tags still honors negatives.
    let negatives = values
        .iter()
        .filter(|(key, value)| key == &format!("remote.{remote}.fetch") && value.starts_with('^'))
        .cloned()
        .collect::<Vec<_>>();
    values
        .retain(|(key, _)| key != &format!("remote.{remote}.fetch") && !key.starts_with("branch."));
    selected.sort();
    for (_, name) in selected {
        values.push((format!("remote.{remote}.fetch"), format!("{name}:{name}")));
    }
    if mode == Policy::All {
        values.extend(negatives);
    }
    let extra = fetch_plan::plan(&local, &values, remote, download.advertisement())?;
    base.mappings.extend(extra.mappings);
    if base.mappings.len() > 2000 {
        return Err("FETCH_MAPPING_LIMIT".into());
    }
    fetch_plan::validate_destinations(
        &local,
        &base
            .mappings
            .iter()
            .filter_map(|m| m.destination.clone())
            .collect::<Vec<_>>(),
    )?;
    base.excluded.extend(extra.excluded);
    base.excluded
        .retain(|name| !base.mappings.iter().any(|m| &m.source == name));
    base.excluded.sort();
    base.excluded.dedup();
    Ok(base)
}
