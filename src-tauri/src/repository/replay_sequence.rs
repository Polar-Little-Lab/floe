//! Ordered replay analysis; every intermediate tree stays in a private ODB.
use super::*;
pub const MAX_STEPS: usize = 32;
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub oid: String,
    pub kind: ReplayKind,
    pub mainline: Option<usize>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub selection: Selection,
    pub before_tree: String,
    pub result_tree: Option<String>,
    pub parent: Option<String>,
    pub originally_empty: bool,
    pub empty: bool,
    pub conflicts: Vec<merge::MergeConflict>,
    pub diff: Option<DiffResult>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: String,
    pub head: String,
    pub requested: usize,
    pub steps: Vec<Step>,
    pub stopped_at: Option<usize>,
    pub result_tree: Option<String>,
    pub blockers: Vec<String>,
}
pub(super) fn preview(
    session: &mut Session,
    revision: &str,
    selections: &[Selection],
) -> Result<Plan> {
    preview_from(session, revision, selections, None)
}
// Only internal rebase planning may provide a base and history-derived selection.
pub(super) fn preview_from(
    session: &mut Session,
    revision: &str,
    selections: &[Selection],
    onto: Option<Oid>,
) -> Result<Plan> {
    if selections.is_empty() || selections.len() > MAX_STEPS {
        return Err("INVALID_SEQUENCE_LENGTH".into());
    }
    let snap = session.fresh(revision)?;
    let repo = open(&session.root)?;
    let head = if let Some(id) = onto {
        repo.find_commit(id).map_err(err)?
    } else {
        repo.head().map_err(err)?.peel_to_commit().map_err(err)?
    };
    checkout::inspect_tree(&repo, &head.tree().map_err(err)?)?;
    // Validate the entire request, including entries after a possible conflict.
    let mut parents = Vec::new();
    for selection in selections {
        if selection.oid.len() != 40
            || !selection.oid.bytes().all(|b| b.is_ascii_hexdigit())
            || (onto.is_none() && !snap.commits.iter().any(|c| c.oid == selection.oid))
        {
            return Err("INVALID_SEQUENCE_COMMIT".into());
        }
        let commit = repo
            .find_commit(Oid::from_str(&selection.oid).map_err(err)?)
            .map_err(err)?;
        let parent = match (commit.parent_count(), selection.mainline) {
            (0, None) => None,
            (1, None) => Some(commit.parent_id(0).map_err(err)?),
            (n, Some(i)) if i > 0 && i <= n => Some(commit.parent_id(i - 1).map_err(err)?),
            (n, None) if n > 1 => return Err("MAINLINE_REQUIRED".into()),
            _ => return Err("INVALID_MAINLINE".into()),
        };
        checkout::inspect_tree(&repo, &commit.tree().map_err(err)?)?;
        if let Some(id) = parent {
            checkout::inspect_tree(
                &repo,
                &repo.find_commit(id).map_err(err)?.tree().map_err(err)?,
            )?;
        }
        parents.push(parent);
    }
    let temp = tempfile::tempdir().map_err(err)?;
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true).external_template(false);
    let scratch = Repository::init_opts(temp.path(), &options).map_err(err)?;
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
    let empty = scratch
        .treebuilder(None)
        .map_err(err)?
        .write()
        .map_err(err)?;
    let mut current = head.tree_id();
    let mut steps = Vec::new();
    let mut stopped_at = None;
    let mut total_files = 0;
    let mut total_bytes = 0u64;
    let mut patch_bytes = 0;
    for (position, (selection, parent)) in selections.iter().zip(parents).enumerate() {
        let ours = scratch.find_tree(current).map_err(err)?;
        let selected = scratch
            .find_commit(Oid::from_str(&selection.oid).map_err(err)?)
            .map_err(err)?
            .tree()
            .map_err(err)?;
        let previous = if let Some(id) = parent {
            scratch.find_commit(id).map_err(err)?.tree().map_err(err)?
        } else {
            scratch.find_tree(empty).map_err(err)?
        };
        let (base, incoming) = if selection.kind == ReplayKind::CherryPick {
            (&previous, &selected)
        } else {
            (&selected, &previous)
        };
        let mut index = scratch
            .merge_trees(base, &ours, incoming, None)
            .map_err(err)?;
        let mut conflicts = Vec::new();
        for conflict in index.conflicts().map_err(err)? {
            if conflicts.len() >= MAX_FILES {
                return Err("LIMIT: sequence conflicts".into());
            }
            let c = conflict.map_err(err)?;
            conflicts.push(merge::MergeConflict {
                ancestor: merge::version(c.ancestor)?,
                ours: merge::version(c.our)?,
                theirs: merge::version(c.their)?,
            });
        }
        let result = if conflicts.is_empty() {
            Some(index.write_tree_to(&scratch).map_err(err)?)
        } else {
            None
        };
        if let Some(id) = result {
            checkout::inspect_tree(&scratch, &scratch.find_tree(id).map_err(err)?)?;
        }
        // Charge changed objects even when shared with the source, keeping the batch bounded.
        let diff = scratch
            .diff_tree_to_index(Some(&ours), Some(&index), None)
            .map_err(err)?;
        total_files += diff.deltas().len();
        if total_files > MAX_FILES {
            return Err("LIMIT: sequence changed files".into());
        }
        for delta in diff.deltas() {
            let id = delta.new_file().id();
            if !id.is_zero() {
                total_bytes = total_bytes
                    .checked_add(scratch.odb().map_err(err)?.read_header(id).map_err(err)?.0 as u64)
                    .ok_or("LIMIT: sequence bytes")?;
            }
        }
        if total_bytes > 50 * 1024 * 1024 {
            return Err("LIMIT: sequence changed bytes".into());
        }
        let rendered = if conflicts.is_empty() {
            let d = render_diff(&diff)?;
            patch_bytes += d.text.len();
            if patch_bytes > 2 * 1024 * 1024 {
                return Err("LIMIT: sequence patch bytes".into());
            }
            Some(d)
        } else {
            None
        };
        steps.push(Step {
            selection: selection.clone(),
            before_tree: current.to_string(),
            result_tree: result.map(|o| o.to_string()),
            parent: parent.map(|o| o.to_string()),
            originally_empty: selected.id() == previous.id(),
            empty: result == Some(current),
            conflicts,
            diff: rendered,
        });
        if let Some(next) = result {
            current = next;
        } else {
            stopped_at = Some(position);
            break;
        }
    }
    let mut blockers = Vec::new();
    if let Some(reason) = snap.write_block {
        blockers.push(reason);
    }
    if !snap.changes.is_empty() {
        blockers.push("DIRTY_WORKTREE".into());
    }
    if !repo.head().map_err(err)?.is_branch() {
        blockers.push("DETACHED_HEAD".into());
    }
    if repo.state() != git2::RepositoryState::Clean {
        blockers.push("OPERATION_IN_PROGRESS".into());
    }
    if replay_draft::exists(&session.root) || merge_draft::exists(&session.root) {
        blockers.push("DRAFT_IN_PROGRESS".into());
    }
    if stopped_at.is_some() {
        blockers.push("CONFLICTS".into());
    }
    session.fresh(revision)?;
    Ok(Plan {
        revision: revision.into(),
        head: head.id().to_string(),
        requested: selections.len(),
        steps,
        stopped_at,
        result_tree: if stopped_at.is_none() {
            Some(current.to_string())
        } else {
            None
        },
        blockers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(repo: &Repository, reference: &str, a: &str, b: &str, parents: &[Oid]) -> Oid {
        let mut tree = repo.treebuilder(None).unwrap();
        for (path, text) in [("a", a), ("b", b)] {
            tree.insert(path, repo.blob(text.as_bytes()).unwrap(), 0o100644)
                .unwrap();
        }
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|id| repo.find_commit(*id).unwrap())
            .collect();
        let signature = git2::Signature::now("Sequence Test", "sequence@example.invalid").unwrap();
        repo.commit(
            Some(reference),
            &signature,
            &signature,
            "Sequence fixture",
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn fingerprint(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            dir: &Path,
            result: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
        ) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(root, &entry.path(), result);
                } else {
                    result.insert(
                        entry.path().strip_prefix(root).unwrap().into(),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut result = std::collections::BTreeMap::new();
        visit(root, root, &mut result);
        result
    }
    fn git(root: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap()
    }
    #[test]
    fn ordered_private_trees_and_first_conflict_match_git_without_source_writes() {
        for kind in [ReplayKind::CherryPick, ReplayKind::Revert] {
            for conflict in [false, true] {
                let d = tempfile::tempdir().unwrap();
                let repo = Repository::init(d.path()).unwrap();
                repo.set_head("refs/heads/main").unwrap();
                let base = commit(&repo, "refs/heads/main", "a0\n", "b0\n", &[]);
                let first = commit(&repo, "refs/heads/topic", "a1\n", "b0\n", &[base]);
                let second = commit(&repo, "refs/heads/topic", "a1\n", "b1\n", &[first]);
                let (a, b) = match (kind, conflict) {
                    (ReplayKind::CherryPick, false) => ("a0\n", "b0\n"),
                    (ReplayKind::CherryPick, true) => ("a0\n", "local\n"),
                    (ReplayKind::Revert, false) => ("a1\n", "b1\n"),
                    (ReplayKind::Revert, true) => ("local\n", "b1\n"),
                };
                commit(&repo, "refs/heads/main", a, b, &[base]);
                repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                    .unwrap();
                let root = d.path().canonicalize().unwrap();
                let mut session = Session::new(root.clone()).unwrap();
                let snap = session.snapshot().unwrap();
                let before = fingerprint(&root);
                let order = if kind == ReplayKind::CherryPick {
                    [first, second]
                } else {
                    [second, first]
                };
                let selections: Vec<_> = order
                    .iter()
                    .map(|id| Selection {
                        oid: id.to_string(),
                        kind,
                        mainline: None,
                    })
                    .collect();
                let result = preview(&mut session, &snap.revision, &selections).unwrap();
                assert_eq!(result.steps.len(), 2);
                assert_eq!(
                    result.steps[1].before_tree,
                    result.steps[0].result_tree.clone().unwrap()
                );
                assert_eq!(result.stopped_at, if conflict { Some(1) } else { None });
                assert_eq!(result.requested, 2);
                assert_eq!(fingerprint(&root), before);
                let oracle = tempfile::tempdir().unwrap();
                assert!(git(
                    oracle.path(),
                    &["clone", "--no-hardlinks", d.path().to_str().unwrap(), "."]
                )
                .status
                .success());
                for (i, selection) in selections.iter().enumerate() {
                    let out = git(
                        oracle.path(),
                        &[
                            if kind == ReplayKind::CherryPick {
                                "cherry-pick"
                            } else {
                                "revert"
                            },
                            "--no-commit",
                            &selection.oid,
                        ],
                    );
                    if conflict && i == 1 {
                        assert!(!out.status.success());
                        let index = Repository::open(oracle.path()).unwrap().index().unwrap();
                        let actual: Vec<_> = index
                            .conflicts()
                            .unwrap()
                            .map(|c| {
                                let c = c.unwrap();
                                [c.ancestor, c.our, c.their].map(|entry| {
                                    entry.map(|e| {
                                        (String::from_utf8(e.path).unwrap(), e.id.to_string())
                                    })
                                })
                            })
                            .collect();
                        let expected: Vec<_> = result.steps[i]
                            .conflicts
                            .iter()
                            .map(|c| {
                                [&c.ancestor, &c.ours, &c.theirs]
                                    .map(|v| v.as_ref().map(|v| (v.path.clone(), v.oid.clone())))
                            })
                            .collect();
                        assert_eq!(actual, expected);
                        assert!(result.steps[i].diff.is_none());
                        assert!(result.result_tree.is_none());
                    } else {
                        assert!(
                            out.status.success(),
                            "{}",
                            String::from_utf8_lossy(&out.stderr)
                        );
                        assert_eq!(
                            String::from_utf8(git(oracle.path(), &["write-tree"]).stdout)
                                .unwrap()
                                .trim(),
                            result.steps[i].result_tree.as_ref().unwrap()
                        );
                    }
                }
                let mut invalid = selections.clone();
                invalid.push(Selection {
                    oid: "f".repeat(40),
                    kind,
                    mainline: None,
                });
                assert_eq!(
                    preview(&mut session, &snap.revision, &invalid).unwrap_err(),
                    "INVALID_SEQUENCE_COMMIT"
                );
                assert!(preview(&mut session, &snap.revision, &[]).is_err());
                assert!(preview(
                    &mut session,
                    &snap.revision,
                    &vec![selections[0].clone(); MAX_STEPS + 1]
                )
                .is_err());
                assert_eq!(fingerprint(&root), before);
                fs::write(root.join("a"), "external edit").unwrap();
                assert!(preview(&mut session, &snap.revision, &selections)
                    .unwrap_err()
                    .starts_with("STALE"));
            }
        }
    }
    #[test]
    fn repeat_selection_marks_empty_and_validates_mainline_before_work() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let base = commit(&repo, "refs/heads/main", "a0", "b0", &[]);
        let first = commit(&repo, "refs/heads/topic", "a1", "b0", &[base]);
        let merged = commit(&repo, "refs/heads/merged", "a1", "b0", &[first, base]);
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        let root = d.path().canonicalize().unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = fingerprint(&root);
        let one = Selection {
            oid: first.to_string(),
            kind: ReplayKind::CherryPick,
            mainline: None,
        };
        let result = preview(&mut session, &snap.revision, &[one.clone(), one]).unwrap();
        assert!(!result.steps[0].empty);
        assert!(result.steps[1].empty);
        let merge = Selection {
            oid: merged.to_string(),
            kind: ReplayKind::CherryPick,
            mainline: None,
        };
        assert_eq!(
            preview(&mut session, &snap.revision, std::slice::from_ref(&merge)).unwrap_err(),
            "MAINLINE_REQUIRED"
        );
        assert_eq!(
            preview(
                &mut session,
                &snap.revision,
                &[Selection {
                    mainline: Some(0),
                    ..merge.clone()
                }]
            )
            .unwrap_err(),
            "INVALID_MAINLINE"
        );
        let result = preview(
            &mut session,
            &snap.revision,
            &[Selection {
                mainline: Some(2),
                ..merge
            }],
        )
        .unwrap();
        assert_eq!(
            result.steps[0].result_tree,
            Some(repo.find_commit(first).unwrap().tree_id().to_string())
        );
        assert_eq!(fingerprint(&root), before);
    }
}
