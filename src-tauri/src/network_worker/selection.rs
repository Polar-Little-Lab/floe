//! Repackage only the selected roots and their complete object closure.
use super::*;
use std::collections::BTreeSet;
fn failure(_: impl std::fmt::Display) -> String {
    "REMOTE_SELECTION_FAILED".into()
}
pub(super) fn closure(
    repo: &git2::Repository,
    tips: &[git2::Oid],
) -> Result<BTreeSet<git2::Oid>, String> {
    let odb = repo.odb().map_err(failure)?;
    let mut pending = tips.to_vec();
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    while let Some(oid) = pending.pop() {
        if !seen.insert(oid) {
            continue;
        }
        if seen.len() > 100000 || pending.len() > 1000000 {
            return Err("REMOTE_SELECTION_LIMIT".into());
        }
        let (size, kind) = odb.read_header(oid).map_err(failure)?;
        bytes = bytes.saturating_add(size);
        if size > 16 * 1024 * 1024 || bytes > 256 * 1024 * 1024 {
            return Err("REMOTE_SELECTION_LIMIT".into());
        }
        match kind {
            git2::ObjectType::Commit => {
                let commit = repo.find_commit(oid).map_err(failure)?;
                pending.push(commit.tree_id());
                pending.extend(commit.parent_ids());
            }
            git2::ObjectType::Tree => {
                for entry in repo.find_tree(oid).map_err(failure)?.iter() {
                    if entry.filemode() != 0o160000 {
                        pending.push(entry.id());
                    }
                }
            }
            git2::ObjectType::Tag => pending.push(repo.find_tag(oid).map_err(failure)?.target_id()),
            git2::ObjectType::Blob => {}
            _ => return Err("REMOTE_SELECTION_OBJECT_INVALID".into()),
        }
    }
    Ok(seen)
}
pub(super) fn inventory(repo: &git2::Repository) -> Result<BTreeSet<git2::Oid>, String> {
    let mut objects = BTreeSet::new();
    repo.odb()
        .map_err(failure)?
        .foreach(|oid| {
            objects.insert(*oid);
            objects.len() <= 100000
        })
        .map_err(failure)?;
    Ok(objects)
}
impl Downloaded {
    pub(crate) fn project(&self, roots: &[String]) -> Result<Self, String> {
        if roots.len() > 2000 {
            return Err("REMOTE_SELECTION_LIMIT".into());
        }
        // Verify the original artifact even for an empty selection.
        let source = self.object_repository()?;
        let tips = roots
            .iter()
            .map(|oid| git2::Oid::from_str(oid).map_err(failure))
            .collect::<Result<BTreeSet<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();
        let private = tempfile::tempdir().map_err(failure)?;
        let advertisement = Advertisement {
            endpoint: self.advertisement().endpoint.clone(),
            refs: tips
                .iter()
                .enumerate()
                .map(|(index, oid)| RemoteRef {
                    name: format!("refs/tags/selection/{index}"),
                    oid: oid.to_string(),
                    symbolic_target: None,
                })
                .collect(),
        };
        let mut value = pack::Download {
            advertisement,
            pack_hash: None,
            received_bytes: 0,
            objects: 0,
        };
        if !tips.is_empty() {
            let source = source.ok_or("REMOTE_SELECTION_OBJECT_MISSING")?;
            let selected = closure(&source, &tips)?;
            let destination = private.path().join("quarantine.git");
            let mut options = git2::RepositoryInitOptions::new();
            options.bare(true).external_template(false);
            let result = git2::Repository::init_opts(&destination, &options).map_err(failure)?;
            result
                .set_config(&git2::Config::new().map_err(failure)?)
                .map_err(failure)?;
            let hash = if selected == inventory(&source)? {
                // Keep byte-identical full packs, including existing pin identity.
                let hash = self.pack_hash().ok_or("REMOTE_SELECTION_OBJECT_MISSING")?;
                object_import::publish(source.path(), &destination, hash)?;
                hash.to_owned()
            } else {
                let mut builder = source.packbuilder().map_err(failure)?;
                builder.set_threads(1);
                for oid in &selected {
                    builder.insert_object(*oid, None).map_err(failure)?;
                }
                builder
                    .write(&destination.join("objects/pack"), 0o600)
                    .map_err(failure)?;
                builder
                    .name()
                    .map_err(failure)?
                    .ok_or("REMOTE_SELECTION_FAILED")?
                    .to_owned()
            };
            value.received_bytes =
                std::fs::metadata(destination.join(format!("objects/pack/pack-{hash}.pack")))
                    .map_err(failure)?
                    .len();
            value.objects = selected.len();
            value.pack_hash = Some(hash);
            artifact::verify(private.path(), &value.advertisement.endpoint, &value)?;
            let verified = git2::Repository::open_bare(&destination).map_err(failure)?;
            verified
                .set_config(&git2::Config::new().map_err(failure)?)
                .map_err(failure)?;
            if inventory(&verified)? != selected || closure(&verified, &tips)? != selected {
                return Err("REMOTE_SELECTION_OBJECT_MISMATCH".into());
            }
        }
        artifact::verify(private.path(), &value.advertisement.endpoint, &value)?;
        Ok(Self {
            completed: Completed {
                response: Response::Download { value },
                _private: private,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_pack_contains_exact_tag_and_commit_closure_and_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(dir.path()).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let blob = repo.blob(b"selected bytes").unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("file", blob, 0o100644).unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let parent = repo
            .commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[])
            .unwrap();
        let commit = repo
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "child",
                &tree,
                &[&repo.find_commit(parent).unwrap()],
            )
            .unwrap();
        let tag = repo
            .tag(
                "v1",
                &repo.find_object(commit, None).unwrap(),
                &sig,
                "tag",
                false,
            )
            .unwrap();
        let extra = repo.blob(b"unrelated bytes").unwrap();
        let data = Downloaded::fixture(
            &repo,
            Advertisement {
                endpoint: "https://example.com/r".into(),
                refs: vec![
                    RemoteRef {
                        name: "refs/tags/v1".into(),
                        oid: tag.to_string(),
                        symbolic_target: None,
                    },
                    RemoteRef {
                        name: "refs/tags/extra".into(),
                        oid: extra.to_string(),
                        symbolic_target: None,
                    },
                ],
            },
        );
        let projected = data.project(&[tag.to_string()]).unwrap();
        let expected = [tag, commit, parent, tree.id(), blob].into_iter().collect();
        assert_eq!(
            inventory(&projected.object_repository().unwrap().unwrap()).unwrap(),
            expected
        );
        assert_ne!(projected.pack_hash(), data.pack_hash());
        assert_eq!(
            projected.pack_hash(),
            data.project(&[tag.to_string(), commit.to_string(), tag.to_string()])
                .unwrap()
                .pack_hash()
        );
        assert_eq!(
            data.project(&[tag.to_string(), extra.to_string()])
                .unwrap()
                .pack_hash(),
            data.pack_hash()
        );
        assert!(data
            .project(&["0000000000000000000000000000000000000001".into()])
            .is_err());
        let empty = data.project(&[]).unwrap();
        assert!(empty.pack_hash().is_none());
        assert!(empty.object_repository().unwrap().is_none());
        assert_eq!(data.advertisement().refs.len(), 2);
        let destination = tempfile::tempdir().unwrap();
        let target = git2::Repository::init_bare(destination.path()).unwrap();
        let path = projected.completed._private.path().join(format!(
            "quarantine.git/objects/pack/pack-{}.pack",
            projected.pack_hash().unwrap()
        ));
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"tampered")
            .unwrap();
        assert!(projected.pin(target.path()).is_err());
        assert!(projected.import_into(target.path()).is_err());
        assert!(inventory(&target).unwrap().is_empty());
    }
}
