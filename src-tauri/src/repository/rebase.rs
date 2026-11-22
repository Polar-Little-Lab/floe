//! Read-only default rebase planning. Generated trees stay in the replay scratch ODB.
use super::*;
pub mod draft;
pub(super) mod execution;
use std::collections::HashSet;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: String,
    pub branch: String,
    pub upstream: String,
    pub head_oid: String,
    pub upstream_oid: String,
    pub base_oid: Option<String>,
    pub kind: String,
    pub selections: Vec<ReplaySelection>,
    pub skipped_equivalent: Vec<String>,
    pub flattened_merges: Vec<String>,
    pub replay: Option<ReplaySequencePlan>,
    pub result_tree: Option<String>,
    pub blockers: Vec<String>,
}
fn commits(repo: &Repository, tip: Oid, hide: Oid) -> Result<Vec<Oid>> {
    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
        .map_err(err)?;
    walk.push(tip).map_err(err)?;
    walk.hide(hide).map_err(err)?;
    let mut out = Vec::new();
    for oid in walk {
        if out.len() >= 4096 {
            return Err("LIMIT: rebase history".into());
        }
        out.push(oid.map_err(err)?);
    }
    Ok(out)
}
fn linear_to(repo: &Repository, mut tip: Oid, base: Oid) -> Result<bool> {
    for _ in 0..4096 {
        if tip == base {
            return Ok(true);
        }
        let commit = repo.find_commit(tip).map_err(err)?;
        if commit.parent_count() != 1 {
            return Ok(false);
        }
        tip = commit.parent_id(0).map_err(err)?;
    }
    Err("LIMIT: rebase history".into())
}
#[derive(PartialEq, Eq, Hash)]
struct PatchKey {
    text: Oid,
    binary: Vec<(Vec<u8>, Oid, Oid)>,
}
fn patch(repo: &Repository, oid: Oid) -> Result<Option<PatchKey>> {
    let commit = repo.find_commit(oid).map_err(err)?;
    if commit.parent_count() != 1 {
        return Ok(None);
    }
    let parent = commit.parent(0).map_err(err)?;
    if parent.tree_id() == commit.tree_id() {
        return Ok(None);
    }
    let before = parent.tree().map_err(err)?;
    let after = commit.tree().map_err(err)?;
    checkout::inspect_tree(repo, &before)?;
    checkout::inspect_tree(repo, &after)?;
    let diff = repo
        .diff_tree_to_tree(Some(&before), Some(&after), None)
        .map_err(err)?;
    if diff.deltas().len() > MAX_FILES {
        return Err("LIMIT: rebase patch".into());
    }
    let text = diff.patchid(None).map_err(err)?;
    // libgit2's default binary patch text contains only the file names.
    // Include both blob identities so distinct binary edits cannot be skipped
    // as equivalent commits. Identical binary patches still compare equal.
    let mut binary = Vec::new();
    for delta in diff.deltas() {
        let old = delta.old_file();
        let new = delta.new_file();
        if delta.flags().contains(git2::DiffFlags::BINARY) {
            binary.push((
                new.path_bytes()
                    .or(old.path_bytes())
                    .unwrap_or_default()
                    .to_vec(),
                old.id(),
                new.id(),
            ));
        }
    }
    Ok(Some(PatchKey { text, binary }))
}
impl Session {
    pub fn rebase_preview(&mut self, revision: &str, upstream: &str) -> Result<Plan> {
        let snap = self.fresh(revision)?;
        let repo = open(&self.root)?;
        let head = repo
            .head()
            .map_err(|_| "REBASE_REQUIRES_COMMITTED_BRANCH")?
            .peel_to_commit()
            .map_err(err)?;
        let reference = merge::reference_name(upstream)?;
        let target = repo
            .find_reference(&reference)
            .map_err(|_| "REBASE_UPSTREAM_NOT_FOUND")?
            .peel_to_commit()
            .map_err(err)?;
        checkout::inspect_tree(&repo, &head.tree().map_err(err)?)?;
        checkout::inspect_tree(&repo, &target.tree().map_err(err)?)?;
        let base = match repo.merge_base(head.id(), target.id()) {
            Ok(id) => Some(id),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(err(e)),
        };
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
        if repo.state() != git2::RepositoryState::Clean
            || draft::exists(&self.root)
            || sequence_state::exists(&self.root)
            || replay_draft::exists(&self.root)
            || merge_draft::exists(&self.root)
        {
            blockers.push("OPERATION_IN_PROGRESS".into());
        }
        let mut plan = Plan {
            revision: revision.into(),
            branch: snap.branch,
            upstream: upstream.into(),
            head_oid: head.id().to_string(),
            upstream_oid: target.id().to_string(),
            base_oid: base.map(|id| id.to_string()),
            kind: "replay".into(),
            selections: vec![],
            skipped_equivalent: vec![],
            flattened_merges: vec![],
            replay: None,
            result_tree: None,
            blockers,
        };
        if base.is_none() {
            plan.kind = "unrelated".into();
            plan.blockers.push("UNRELATED_HISTORIES".into());
        } else if head.id() == target.id() || linear_to(&repo, head.id(), target.id())? {
            plan.kind = "upToDate".into();
            plan.result_tree = Some(head.tree_id().to_string());
        } else if repo
            .graph_descendant_of(target.id(), head.id())
            .map_err(err)?
        {
            plan.kind = "fastForward".into();
            plan.result_tree = Some(target.tree_id().to_string());
        } else {
            let upstream_patches = commits(&repo, target.id(), head.id())?
                .into_iter()
                .map(|id| patch(&repo, id))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<HashSet<_>>();
            for oid in commits(&repo, head.id(), target.id())? {
                let commit = repo.find_commit(oid).map_err(err)?;
                if commit.parent_count() > 1 {
                    plan.flattened_merges.push(oid.to_string());
                    continue;
                }
                if patch(&repo, oid)?.is_some_and(|p| upstream_patches.contains(&p)) {
                    plan.skipped_equivalent.push(oid.to_string());
                    continue;
                }
                if plan.selections.len() >= replay_sequence::MAX_STEPS {
                    return Err("LIMIT: rebase steps".into());
                }
                plan.selections.push(ReplaySelection {
                    oid: oid.to_string(),
                    kind: ReplayKind::CherryPick,
                    mainline: None,
                });
            }
            if plan.selections.is_empty() {
                plan.result_tree = Some(target.tree_id().to_string());
            } else {
                let replay = replay_sequence::preview_from(
                    self,
                    revision,
                    &plan.selections,
                    Some(target.id()),
                )?;
                plan.result_tree = replay.result_tree.clone();
                plan.blockers.extend(replay.blockers.clone());
                plan.replay = Some(replay);
            }
        }
        plan.blockers.sort();
        plan.blockers.dedup();
        self.fresh(revision)?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::super::remote_remove::tests::git;
    use super::*;
    fn commit(
        repo: &Repository,
        reference: &str,
        values: [&str; 3],
        parents: &[Oid],
        message: &str,
    ) -> Oid {
        let mut tree = repo.treebuilder(None).unwrap();
        for (path, text) in ["a", "b", "c"].iter().zip(values) {
            tree.insert(*path, repo.blob(text.as_bytes()).unwrap(), 0o100644)
                .unwrap();
        }
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let parents = parents
            .iter()
            .map(|id| repo.find_commit(*id).unwrap())
            .collect::<Vec<_>>();
        let sig = git2::Signature::now("Rebase Test", "rebase@invalid").unwrap();
        repo.commit(
            Some(reference),
            &sig,
            &sig,
            message,
            &tree,
            &parents.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn fixture(kind: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path()).unwrap();
        repo.set_head("refs/heads/topic").unwrap();
        let base = commit(
            &repo,
            "refs/heads/topic",
            ["a0\n", "b0\n", "c0\n"],
            &[],
            "base",
        );
        let first = commit(
            &repo,
            "refs/heads/topic",
            ["a1\n", "b0\n", "c0\n"],
            &[base],
            "first",
        );
        let topic = if kind == "empty" {
            commit(
                &repo,
                "refs/heads/topic",
                ["a1\n", "b0\n", "c0\n"],
                &[first],
                "intentional empty",
            )
        } else {
            commit(
                &repo,
                "refs/heads/topic",
                ["a2\n", "b0\n", "c0\n"],
                &[first],
                "second",
            )
        };
        match kind {
            "equivalent" => {
                let copied = commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a1\n", "b0\n", "c0\n"],
                    &[base],
                    "copied first",
                );
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a1\n", "b1\n", "c0\n"],
                    &[copied],
                    "upstream",
                );
            }
            "becomes-empty" => {
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a1\n", "b1\n", "c0\n"],
                    &[base],
                    "combined upstream",
                );
            }
            "conflict" => {
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["different\n", "b0\n", "c0\n"],
                    &[base],
                    "conflicting upstream",
                );
            }
            "up-to-date" => {
                repo.reference("refs/heads/upstream", base, false, "fixture")
                    .unwrap();
            }
            "fast-forward" => {
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a2\n", "b1\n", "c0\n"],
                    &[topic],
                    "upstream",
                );
            }
            "unrelated" => {
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["other\n", "b0\n", "c0\n"],
                    &[],
                    "unrelated",
                );
            }
            "merge" | "merge-ahead" => {
                let side = commit(
                    &repo,
                    "refs/heads/side",
                    ["a0\n", "b0\n", "c1\n"],
                    &[base],
                    "side",
                );
                commit(
                    &repo,
                    "refs/heads/topic",
                    ["a2\n", "b0\n", "c1\n"],
                    &[topic, side],
                    "merge side",
                );
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a0\n", "b1\n", "c0\n"],
                    &[base],
                    "upstream",
                );
            }
            _ => {
                commit(
                    &repo,
                    "refs/heads/upstream",
                    ["a0\n", "b1\n", "c0\n"],
                    &[base],
                    "upstream",
                );
            }
        }
        if kind == "merge-ahead" {
            repo.reference("refs/heads/upstream", base, true, "fixture")
                .unwrap();
        }
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Rebase Test").unwrap();
        config.set_str("user.email", "rebase@invalid").unwrap();
        d
    }
    fn fingerprint(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    visit(root, &e.path(), out);
                } else {
                    out.insert(
                        e.path().strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(e.path()).unwrap(),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        visit(root, root, &mut out);
        out
    }
    #[test]
    fn default_rebase_plan_matches_git_trees_empty_policy_and_merge_flattening_without_writes() {
        for kind in [
            "clean",
            "equivalent",
            "becomes-empty",
            "empty",
            "merge",
            "merge-ahead",
            "up-to-date",
            "fast-forward",
        ] {
            let d = fixture(kind);
            let oracle = fixture(kind);
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            let before = fingerprint(d.path());
            let plan = session.rebase_preview(&snap.revision, "upstream").unwrap();
            assert!(plan.blockers.is_empty(), "{kind}: {:?}", plan.blockers);
            assert_eq!(fingerprint(d.path()), before);
            git(
                oracle.path(),
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "rebase",
                    "--no-fork-point",
                    "upstream",
                ],
            );
            assert_eq!(
                plan.result_tree.as_deref(),
                Some(git(oracle.path(), &["rev-parse", "HEAD^{tree}"]).trim()),
                "{kind}"
            );
            if let Some(replay) = &plan.replay {
                let kept = replay
                    .steps
                    .iter()
                    .filter(|step| !step.empty || step.originally_empty)
                    .count();
                let actual: usize = git(oracle.path(), &["rev-list", "--count", "upstream..HEAD"])
                    .trim()
                    .parse()
                    .unwrap();
                assert_eq!(kept, actual, "commit count {kind}");
            }
            if kind == "equivalent" {
                assert_eq!(plan.skipped_equivalent.len(), 1);
                assert_eq!(plan.selections.len(), 1);
            }
            if kind == "becomes-empty" {
                let step = &plan.replay.as_ref().unwrap().steps[0];
                assert!(step.empty);
                assert!(!step.originally_empty);
            }
            if kind == "empty" {
                let step = plan.replay.as_ref().unwrap().steps.last().unwrap();
                assert!(step.empty && step.originally_empty);
            }
            if kind == "merge" || kind == "merge-ahead" {
                assert_eq!(plan.flattened_merges.len(), 1);
                assert_eq!(plan.selections.len(), 3);
            }
            if kind == "up-to-date" {
                assert_eq!(plan.kind, "upToDate");
            }
            if kind == "fast-forward" {
                assert_eq!(plan.kind, "fastForward");
            }
        }
    }
    #[test]
    fn conflict_plan_matches_git_unmerged_paths_and_is_read_only() {
        let d = fixture("conflict");
        let oracle = fixture("conflict");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        let plan = session.rebase_preview(&snap.revision, "upstream").unwrap();
        assert!(plan.blockers.contains(&"CONFLICTS".into()));
        assert_eq!(plan.replay.as_ref().unwrap().stopped_at, Some(0));
        assert!(plan.result_tree.is_none());
        assert_eq!(fingerprint(d.path()), before);
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(oracle.path())
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "rebase",
                "--no-fork-point",
                "upstream",
            ])
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(!out.status.success());
        let paths = git(oracle.path(), &["diff", "--name-only", "--diff-filter=U"]);
        assert_eq!(paths.trim(), "a");
        let conflict = &plan.replay.unwrap().steps[0].conflicts[0];
        assert_eq!(conflict.ours.as_ref().unwrap().path, "a");
        assert_eq!(conflict.theirs.as_ref().unwrap().path, "a");
    }
    #[test]
    fn dirty_detached_unrelated_missing_and_stale_are_explicit() {
        let d = fixture("unrelated");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let plan = session.rebase_preview(&snap.revision, "upstream").unwrap();
        assert!(plan.blockers.contains(&"UNRELATED_HISTORIES".into()));
        let d = fixture("clean");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session.rebase_preview(&snap.revision, "missing").is_err());
        assert!(session.rebase_preview(&snap.revision, "../bad").is_err());
        fs::write(d.path().join("a"), "dirty").unwrap();
        assert!(session
            .rebase_preview(&snap.revision, "upstream")
            .unwrap_err()
            .contains("STALE_STATE"));
        let snap = session.snapshot().unwrap();
        assert!(session
            .rebase_preview(&snap.revision, "upstream")
            .unwrap()
            .blockers
            .contains(&"DIRTY_WORKTREE".into()));
        let repo = Repository::open(d.path()).unwrap();
        repo.set_head_detached(repo.head().unwrap().target().unwrap())
            .unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session
            .rebase_preview(&snap.revision, "upstream")
            .unwrap()
            .blockers
            .contains(&"DETACHED_HEAD".into()));
    }
    #[test]
    fn clean_execution_matches_git_and_preserves_authors_messages_and_upstream() {
        for kind in [
            "clean",
            "equivalent",
            "becomes-empty",
            "empty",
            "merge",
            "merge-ahead",
            "up-to-date",
            "fast-forward",
        ] {
            let d = fixture(kind);
            let oracle = fixture(kind);
            let upstream = git(d.path(), &["rev-parse", "upstream"]);
            let mut session = Session::new(d.path().into()).unwrap();
            let snap = session.snapshot().unwrap();
            let next = session
                .mutate(
                    &snap.revision,
                    Action::Rebase {
                        upstream: "upstream".into(),
                        author: "Committer Test".into(),
                        email: "committer@invalid".into(),
                    },
                )
                .unwrap();
            git(
                oracle.path(),
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "rebase",
                    "--no-fork-point",
                    "upstream",
                ],
            );
            assert_eq!(
                git(d.path(), &["rev-parse", "HEAD^{tree}"]),
                git(oracle.path(), &["rev-parse", "HEAD^{tree}"]),
                "{kind}"
            );
            assert_eq!(
                git(
                    d.path(),
                    &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
                ),
                git(
                    oracle.path(),
                    &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
                ),
                "{kind}"
            );
            assert_eq!(
                git(d.path(), &["rev-list", "--count", "upstream..HEAD"]),
                git(oracle.path(), &["rev-list", "--count", "upstream..HEAD"])
            );
            assert_eq!(git(d.path(), &["rev-parse", "upstream"]), upstream);
            assert_eq!(next.branch, "topic");
            assert!(next.changes.is_empty());
            assert!(!next.recovery_pending);
            assert!(git(d.path(), &["status", "--porcelain"]).is_empty());
            git(d.path(), &["fsck", "--strict"]);
            if kind == "clean" {
                assert_eq!(
                    git(d.path(), &["log", "-1", "--format=%cn <%ce>"]).trim(),
                    "Committer Test <committer@invalid>"
                );
            }
        }
    }
    #[test]
    fn execution_conflict_stale_and_invalid_identity_preserve_repository() {
        let d = fixture("conflict");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = fingerprint(d.path());
        assert!(session
            .mutate(
                &snap.revision,
                Action::Rebase {
                    upstream: "upstream".into(),
                    author: "Test".into(),
                    email: "t@invalid".into()
                }
            )
            .unwrap_err()
            .contains("REBASE_BLOCKED"));
        // OS lease bookkeeping may exist after attempted mutation; compare Git data and work files.
        for (path, bytes) in before {
            assert_eq!(fs::read(d.path().join(path)).unwrap(), bytes);
        }
        let d = fixture("clean");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let before = git(d.path(), &["rev-parse", "HEAD"]);
        assert!(session
            .mutate(
                &snap.revision,
                Action::Rebase {
                    upstream: "upstream".into(),
                    author: "bad\nidentity".into(),
                    email: "t@invalid".into()
                }
            )
            .is_err());
        assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), before);
        fs::write(d.path().join("a"), "later edit").unwrap();
        assert!(session
            .mutate(
                &snap.revision,
                Action::Rebase {
                    upstream: "upstream".into(),
                    author: "Test".into(),
                    email: "t@invalid".into()
                }
            )
            .unwrap_err()
            .contains("STALE_STATE"));
    }
    #[test]
    fn rebase_child() {
        let Some(root) = std::env::var_os("BRANCHDESK_REBASE_ROOT") else {
            return;
        };
        let mut session = Session::new(PathBuf::from(root)).unwrap();
        let snap = session.snapshot().unwrap();
        if std::env::var("BRANCHDESK_REBASE_RECOVER_MODE")
            .ok()
            .as_deref()
            == Some("1")
        {
            session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
        } else if snap.rebase_draft.is_some() {
            session
                .mutate(&snap.revision, Action::ContinueRebase)
                .unwrap();
        } else {
            session
                .mutate(
                    &snap.revision,
                    Action::Rebase {
                        upstream: "upstream".into(),
                        author: "Test".into(),
                        email: "t@invalid".into(),
                    },
                )
                .unwrap();
        }
    }
    fn kill_rebase(root: &Path, step: usize, recover: bool) {
        let ready = root.join(".git/branchdesk-checkout/child.ready");
        let _ = fs::remove_file(&ready);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::rebase::tests::rebase_child",
                "--nocapture",
            ])
            .env("BRANCHDESK_REBASE_ROOT", root)
            .env("BRANCHDESK_CRASH_ROOT", root)
            .env(
                if recover {
                    "BRANCHDESK_RECOVERY_STEP"
                } else {
                    "BRANCHDESK_CRASH_STEP"
                },
                step.to_string(),
            )
            .env(
                "BRANCHDESK_REBASE_RECOVER_MODE",
                if recover { "1" } else { "0" },
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        while !ready.exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("rebase child exited {status}");
            }
            if start.elapsed() > std::time::Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("rebase checkpoint timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn rebase_actual_kills_restore_original_then_allow_retry() {
        for step in [100, 101, 102, 0, 1, 2, 3] {
            let d = fixture("clean");
            let root = d.path().canonicalize().unwrap();
            let original = git(&root, &["rev-parse", "HEAD"]);
            let before_index = fs::read(root.join(".git/index")).unwrap();
            kill_rebase(&root, step, false);
            if step != 3 {
                assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
            }
            git(&root, &["gc", "--prune=now"]);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(snap.recovery_pending);
            let recovered = session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!recovered.recovery_pending);
            assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), before_index);
            let next = session
                .mutate(
                    &recovered.revision,
                    Action::Rebase {
                        upstream: "upstream".into(),
                        author: "Test".into(),
                        email: "t@invalid".into(),
                    },
                )
                .unwrap();
            assert!(next.changes.is_empty());
            git(&root, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn rebase_second_recovery_kills_restore_original_and_reject_tampered_intent() {
        for step in 0..=3 {
            let d = fixture("clean");
            let root = d.path().canonicalize().unwrap();
            let original = git(&root, &["rev-parse", "HEAD"]);
            kill_rebase(&root, 3, false);
            kill_rebase(&root, step, true);
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
            assert!(git(&root, &["status", "--porcelain"]).is_empty());
        }
        for field in ["name", "upstream_oid"] {
            let d = fixture("clean");
            let root = d.path().canonicalize().unwrap();
            kill_rebase(&root, 0, false);
            let file = root.join(".git/branchdesk-checkout/manifest.json");
            let mut record: serde_json::Value =
                serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            record["rebase"][field] = serde_json::Value::String(if field == "name" {
                "forged committer".into()
            } else {
                git(&root, &["rev-parse", "HEAD"]).trim().into()
            });
            fs::write(file, serde_json::to_vec(&record).unwrap()).unwrap();
            let original = git(&root, &["rev-parse", "HEAD"]);
            let index = fs::read(root.join(".git/index")).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            assert!(session
                .mutate(&snap.revision, Action::RecoverCheckout)
                .is_err());
            assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        }
    }
    fn start_conflict(session: &mut Session) -> Snapshot {
        let snap = session.snapshot().unwrap();
        session
            .mutate(
                &snap.revision,
                Action::StartRebase {
                    upstream: "upstream".into(),
                    author: "Rebase Committer".into(),
                    email: "committer@invalid".into(),
                },
            )
            .unwrap()
    }
    fn resolve_current(
        session: &mut Session,
        snap: &Snapshot,
        resolution: merge_draft::Resolution,
    ) -> Snapshot {
        let view = session.rebase_draft_preview(&snap.revision).unwrap();
        let pending = view.pending.unwrap();
        session
            .mutate(
                &snap.revision,
                Action::ResolveRebase {
                    draft_id: view.id,
                    source: pending.source,
                    file_id: pending.conflicts[0].id.clone(),
                    resolution,
                },
            )
            .unwrap()
    }
    #[test]
    fn conflict_draft_resumes_resolutions_and_final_commit_matches_git() {
        let d = fixture("conflict");
        let oracle = fixture("conflict");
        let root = d.path();
        let before = ["HEAD", "index"].map(|p| fs::read(root.join(".git").join(p)).unwrap());
        let original = git(root, &["rev-parse", "HEAD"]);
        let mut session = Session::new(root.into()).unwrap();
        let snap = start_conflict(&mut session);
        let view = session.rebase_draft_preview(&snap.revision).unwrap();
        let pending = view.pending.unwrap();
        assert_eq!(pending.conflicts[0].ours.as_deref(), Some("different\n"));
        assert_eq!(pending.conflicts[0].theirs.as_deref(), Some("a1\n"));
        assert!(session
            .mutate(&snap.revision, Action::StageAll)
            .unwrap_err()
            .contains("REBASE_IN_PROGRESS"));
        let saved = resolve_current(&mut session, &snap, merge_draft::Resolution::Theirs {});
        assert!(saved.rebase_draft.is_some());
        assert_eq!(git(root, &["rev-parse", "HEAD"]), original);
        assert_eq!(
            ["HEAD", "index"].map(|p| fs::read(root.join(".git").join(p)).unwrap()),
            before
        );
        assert_eq!(fs::read_to_string(root.join("a")).unwrap(), "a2\n");
        drop(session);
        git(root, &["gc", "--prune=now"]);
        let mut session = Session::new(root.into()).unwrap();
        let snap = session.snapshot().unwrap();
        assert!(session
            .rebase_draft_preview(&snap.revision)
            .unwrap()
            .pending
            .is_none());
        let next = session
            .mutate(&snap.revision, Action::ContinueRebase)
            .unwrap();
        assert!(next.rebase_draft.is_none());
        assert!(next.changes.is_empty());
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(oracle.path())
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "rebase",
                "--no-fork-point",
                "upstream",
            ])
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(!out.status.success());
        fs::write(oracle.path().join("a"), "a1\n").unwrap();
        git(oracle.path(), &["add", "a"]);
        git(
            oracle.path(),
            &["-c", "core.editor=true", "rebase", "--continue"],
        );
        assert_eq!(
            git(root, &["rev-parse", "HEAD^{tree}"]),
            git(oracle.path(), &["rev-parse", "HEAD^{tree}"])
        );
        assert_eq!(
            git(root, &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]),
            git(
                oracle.path(),
                &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
            )
        );
        git(root, &["fsck", "--strict"]);
    }
    #[test]
    fn rebase_skip_abort_and_stale_resolution_preserve_live_files() {
        let d = fixture("conflict");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = start_conflict(&mut session);
        let original = git(d.path(), &["rev-parse", "HEAD"]);
        let oldview = session.rebase_draft_preview(&snap.revision).unwrap();
        let saved = resolve_current(&mut session, &snap, merge_draft::Resolution::Ours {});
        let view = session.rebase_draft_preview(&saved.revision).unwrap();
        assert!(view.pending.is_some());
        assert_ne!(
            view.pending.as_ref().unwrap().source,
            oldview.pending.as_ref().unwrap().source
        );
        assert!(session
            .mutate(&snap.revision, Action::ContinueRebase)
            .unwrap_err()
            .contains("STALE_STATE"));
        let pending = view.pending.unwrap();
        let skipped = session
            .mutate(
                &saved.revision,
                Action::SkipRebase {
                    draft_id: view.id,
                    source: pending.source,
                },
            )
            .unwrap();
        assert!(session
            .rebase_draft_preview(&skipped.revision)
            .unwrap()
            .pending
            .is_none());
        let next = session
            .mutate(&skipped.revision, Action::ContinueRebase)
            .unwrap();
        assert_eq!(
            next.head_oid.as_deref(),
            Some(git(d.path(), &["rev-parse", "upstream"]).trim())
        );
        let d = fixture("conflict");
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = start_conflict(&mut session);
        assert!(session
            .mutate(&snap.revision, Action::ContinueRebase)
            .unwrap_err()
            .contains("UNRESOLVED_REBASE"));
        fs::write(d.path().join("a"), "keep later work").unwrap();
        let snap = session.snapshot().unwrap();
        session.mutate(&snap.revision, Action::AbortRebase).unwrap();
        assert_eq!(
            fs::read_to_string(d.path().join("a")).unwrap(),
            "keep later work"
        );
        assert!(!draft::exists(d.path()));
        assert!(!original.is_empty());
    }
    #[test]
    fn rebase_draft_worker() {
        let Some(root) = std::env::var_os("BRANCHDESK_REBASE_DRAFT_ROOT") else {
            return;
        };
        let mut session = Session::new(PathBuf::from(root)).unwrap();
        let snap = session.snapshot().unwrap();
        match std::env::var("BRANCHDESK_REBASE_DRAFT_JOB")
            .unwrap()
            .as_str()
        {
            "start" => {
                start_conflict(&mut session);
            }
            "resolve" => {
                resolve_current(&mut session, &snap, merge_draft::Resolution::Theirs {});
            }
            "finish" => {
                session
                    .mutate(&snap.revision, Action::ContinueRebase)
                    .unwrap();
            }
            "abort" => {
                session.mutate(&snap.revision, Action::AbortRebase).unwrap();
            }
            _ => panic!("unknown job"),
        }
    }
    fn kill_draft(root: &Path, job: &str, checkpoint: &str) {
        let ready = root.join(".git/rebase-draft-ready");
        let _ = fs::remove_file(&ready);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::rebase::tests::rebase_draft_worker",
                "--nocapture",
            ])
            .env("BRANCHDESK_REBASE_DRAFT_ROOT", root)
            .env("BRANCHDESK_REBASE_DRAFT_JOB", job)
            .env("BRANCHDESK_REBASE_DRAFT_STEP", checkpoint)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        while !ready.exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("draft child exited {status}");
            }
            if start.elapsed() > std::time::Duration::from_secs(20) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("draft checkpoint timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn rebase_draft_actual_save_and_completion_kills_resume_without_duplicate_commits() {
        for job in ["start", "resolve"] {
            for step in ["save:0", "save:1", "save:2"] {
                let d = fixture("conflict");
                let root = d.path().canonicalize().unwrap();
                let original = git(&root, &["rev-parse", "HEAD"]);
                if job == "resolve" {
                    let mut s = Session::new(root.clone()).unwrap();
                    start_conflict(&mut s);
                }
                kill_draft(&root, job, step);
                assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
                let mut s = Session::new(root.clone()).unwrap();
                let mut snap = s.snapshot().unwrap();
                if snap.rebase_draft.is_none() {
                    snap = start_conflict(&mut s);
                }
                if s.rebase_draft_preview(&snap.revision)
                    .unwrap()
                    .pending
                    .is_some()
                {
                    snap = resolve_current(&mut s, &snap, merge_draft::Resolution::Theirs {});
                }
                s.mutate(&snap.revision, Action::ContinueRebase).unwrap();
                assert_eq!(
                    git(&root, &["rev-list", "--count", "upstream..HEAD"]).trim(),
                    "2"
                );
            }
        }
        for step in ["applied:0", "archive:0", "archive:1"] {
            let d = fixture("conflict");
            let root = d.path().canonicalize().unwrap();
            let mut s = Session::new(root.clone()).unwrap();
            let snap = start_conflict(&mut s);
            resolve_current(&mut s, &snap, merge_draft::Resolution::Theirs {});
            drop(s);
            kill_draft(&root, "finish", step);
            let final_head = git(&root, &["rev-parse", "HEAD"]);
            fs::write(root.join("later.txt"), "preserve").unwrap();
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            if snap.rebase_draft.is_some() {
                assert!(s.rebase_draft_preview(&snap.revision).unwrap().completed);
                s.mutate(&snap.revision, Action::ContinueRebase).unwrap();
            }
            assert_eq!(git(&root, &["rev-parse", "HEAD"]), final_head);
            assert_eq!(
                git(&root, &["rev-list", "--count", "upstream..HEAD"]).trim(),
                "2"
            );
            assert_eq!(
                fs::read_to_string(root.join("later.txt")).unwrap(),
                "preserve"
            );
        }
    }
    #[test]
    fn resolved_rebase_checkout_kills_roll_back_then_resume_saved_draft() {
        for step in [100, 101, 102, 0, 1, 2] {
            let d = fixture("conflict");
            let root = d.path().canonicalize().unwrap();
            let original = git(&root, &["rev-parse", "HEAD"]);
            let mut s = Session::new(root.clone()).unwrap();
            let snap = start_conflict(&mut s);
            resolve_current(&mut s, &snap, merge_draft::Resolution::Theirs {});
            drop(s);
            // Resolved result has the original tree; index and ref publication are steps 1/2.
            kill_rebase(&root, step, false);
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            let recovered = s.mutate(&snap.revision, Action::RecoverCheckout).unwrap();
            assert_eq!(git(&root, &["rev-parse", "HEAD"]), original);
            assert!(recovered.rebase_draft.is_some());
            let done = s
                .mutate(&recovered.revision, Action::ContinueRebase)
                .unwrap();
            assert!(done.rebase_draft.is_none());
            assert!(done.changes.is_empty());
            git(&root, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn edited_resolution_is_durable_and_damaged_draft_abort_preserves_work() {
        let d = fixture("conflict");
        let mut s = Session::new(d.path().into()).unwrap();
        let snap = start_conflict(&mut s);
        let next = resolve_current(
            &mut s,
            &snap,
            merge_draft::Resolution::Text {
                text: "custom resolution\n".into(),
            },
        );
        let view = s.rebase_draft_preview(&next.revision).unwrap();
        assert!(view.pending.is_some());
        assert_eq!(
            view.pending.unwrap().conflicts[0].ours.as_deref(),
            Some("custom resolution\n")
        );
        assert_eq!(fs::read_to_string(d.path().join("a")).unwrap(), "a2\n");
        fs::write(
            d.path().join(".git/branchdesk-rebase-draft.json"),
            "malformed",
        )
        .unwrap();
        let snap = s.snapshot().unwrap();
        assert!(snap.rebase_draft_error.is_some());
        fs::write(d.path().join("a"), "external work").unwrap();
        let snap = s.snapshot().unwrap();
        s.mutate(&snap.revision, Action::AbortRebase).unwrap();
        assert_eq!(
            fs::read_to_string(d.path().join("a")).unwrap(),
            "external work"
        );
        assert!(!draft::exists(d.path()));
    }
    #[test]
    fn binary_delete_and_add_conflict_resolutions_match_git_after_restart() {
        for (base, local, upstream) in [
            (
                Some("base\0bytes"),
                Some("local\0bytes"),
                Some("remote\0bytes"),
            ),
            (Some("base\n"), Some("local\n"), None),
            (Some("base\n"), None, Some("remote\n")),
            (None, Some("local\n"), Some("remote\n")),
        ] {
            for choice in ["ours", "theirs", "delete"] {
                let make = || {
                    let dir = tempfile::tempdir().unwrap();
                    let repo = Repository::init(dir.path()).unwrap();
                    repo.set_head("refs/heads/topic").unwrap();
                    let create = |reference: &str,
                                  content: Option<&str>,
                                  parent: Option<Oid>,
                                  message: &str| {
                        let mut tree = repo.treebuilder(None).unwrap();
                        tree.insert("keep", repo.blob(b"unchanged\n").unwrap(), 0o100644)
                            .unwrap();
                        if let Some(content) = content {
                            tree.insert("a", repo.blob(content.as_bytes()).unwrap(), 0o100644)
                                .unwrap();
                        }
                        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
                        let parent = parent.map(|id| repo.find_commit(id).unwrap());
                        let sig = git2::Signature::now("Rebase Test", "rebase@invalid").unwrap();
                        repo.commit(
                            Some(reference),
                            &sig,
                            &sig,
                            message,
                            &tree,
                            &parent.iter().collect::<Vec<_>>(),
                        )
                        .unwrap()
                    };
                    let root = create("refs/heads/topic", base, None, "base\n");
                    create("refs/heads/topic", local, Some(root), "local\n");
                    create("refs/heads/upstream", upstream, Some(root), "remote\n");
                    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                        .unwrap();
                    repo.config()
                        .unwrap()
                        .set_str("user.name", "Rebase Test")
                        .unwrap();
                    repo.config()
                        .unwrap()
                        .set_str("user.email", "rebase@invalid")
                        .unwrap();
                    dir
                };
                let d = make();
                let oracle = make();
                let original = git(d.path(), &["rev-parse", "HEAD"]);
                let index = fs::read(d.path().join(".git/index")).unwrap();
                let mut session = Session::new(d.path().into()).unwrap();
                let snap = start_conflict(&mut session);
                let view = session.rebase_draft_preview(&snap.revision).unwrap();
                let pending = view.pending.unwrap();
                assert_eq!(pending.conflicts.len(), 1);
                let conflict = &pending.conflicts[0];
                assert_eq!(
                    conflict.present,
                    [base.is_some(), upstream.is_some(), local.is_some()]
                );
                assert_eq!(conflict.binary, base.is_some_and(|v| v.contains('\0')));
                let resolution = match choice {
                    "ours" => merge_draft::Resolution::Ours {},
                    "theirs" => merge_draft::Resolution::Theirs {},
                    _ => merge_draft::Resolution::Delete {},
                };
                resolve_current(&mut session, &snap, resolution);
                assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), original);
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read(d.path().join("a")).ok(),
                    local.map(|v| v.as_bytes().to_vec())
                );
                drop(session);
                git(d.path(), &["gc", "--prune=now"]);
                let mut session = Session::new(d.path().into()).unwrap();
                let snap = session.snapshot().unwrap();
                let done = session
                    .mutate(&snap.revision, Action::ContinueRebase)
                    .unwrap();
                assert!(done.changes.is_empty());
                assert!(done.rebase_draft.is_none());
                let output = std::process::Command::new("git")
                    .arg("-C")
                    .arg(oracle.path())
                    .args([
                        "-c",
                        "core.hooksPath=/dev/null",
                        "rebase",
                        "--no-fork-point",
                        "upstream",
                    ])
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .output()
                    .unwrap();
                assert!(!output.status.success());
                let expected = match choice {
                    "ours" => upstream,
                    "theirs" => local,
                    _ => None,
                };
                if let Some(content) = expected {
                    fs::write(oracle.path().join("a"), content).unwrap();
                } else if oracle.path().join("a").exists() {
                    fs::remove_file(oracle.path().join("a")).unwrap();
                }
                git(oracle.path(), &["add", "-A"]);
                if expected == upstream {
                    git(oracle.path(), &["rebase", "--skip"]);
                } else {
                    git(
                        oracle.path(),
                        &["-c", "core.editor=true", "rebase", "--continue"],
                    );
                }
                assert_eq!(
                    git(d.path(), &["rev-parse", "HEAD^{tree}"]),
                    git(oracle.path(), &["rev-parse", "HEAD^{tree}"]),
                    "{base:?} {local:?} {upstream:?} {choice}"
                );
                assert_eq!(
                    git(
                        d.path(),
                        &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
                    ),
                    git(
                        oracle.path(),
                        &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
                    )
                );
                assert_eq!(
                    fs::read(d.path().join("a")).ok(),
                    expected.map(|v| v.as_bytes().to_vec())
                );
                git(d.path(), &["fsck", "--strict"]);
            }
        }
    }
    #[test]
    fn multiple_files_and_successive_conflicts_keep_saved_choices_scoped_to_source() {
        let make = || {
            let d = fixture("conflict");
            let repo = Repository::open(d.path()).unwrap();
            let base = repo.revparse_single("topic~2").unwrap().id();
            repo.find_reference("refs/heads/topic")
                .unwrap()
                .delete()
                .unwrap();
            repo.find_reference("refs/heads/upstream")
                .unwrap()
                .delete()
                .unwrap();
            let first = commit(
                &repo,
                "refs/heads/topic",
                ["a1\n", "b1\n", "c0\n"],
                &[base],
                "first\n",
            );
            commit(
                &repo,
                "refs/heads/topic",
                ["a2\n", "b2\n", "c0\n"],
                &[first],
                "second\n",
            );
            commit(
                &repo,
                "refs/heads/upstream",
                ["remote-a\n", "remote-b\n", "c0\n"],
                &[base],
                "upstream\n",
            );
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .unwrap();
            d
        };
        let d = make();
        let oracle = make();
        let mut session = Session::new(d.path().into()).unwrap();
        let mut snap = start_conflict(&mut session);
        let first = session.rebase_draft_preview(&snap.revision).unwrap();
        let pending = first.pending.unwrap();
        assert_eq!(pending.conflicts.len(), 2);
        for (index, c) in pending.conflicts.iter().enumerate() {
            snap = session
                .mutate(
                    &snap.revision,
                    Action::ResolveRebase {
                        draft_id: first.id.clone(),
                        source: pending.source.clone(),
                        file_id: c.id.clone(),
                        resolution: merge_draft::Resolution::Ours {},
                    },
                )
                .unwrap();
            if index == 0 {
                let view = session.rebase_draft_preview(&snap.revision).unwrap();
                assert_eq!(view.pending.as_ref().unwrap().source, pending.source);
                assert_eq!(view.pending.unwrap().resolutions.len(), 1);
                assert!(session
                    .mutate(&snap.revision, Action::ContinueRebase)
                    .unwrap_err()
                    .contains("UNRESOLVED_REBASE"));
            }
        }
        drop(session);
        let mut session = Session::new(d.path().into()).unwrap();
        snap = session.snapshot().unwrap();
        let second = session.rebase_draft_preview(&snap.revision).unwrap();
        let next = second.pending.unwrap();
        assert_ne!(next.source, pending.source);
        assert_eq!(next.conflicts.len(), 2);
        assert!(next.resolutions.is_empty());
        assert!(session
            .mutate(
                &snap.revision,
                Action::ResolveRebase {
                    draft_id: second.id.clone(),
                    source: pending.source,
                    file_id: next.conflicts[0].id.clone(),
                    resolution: merge_draft::Resolution::Delete {}
                }
            )
            .is_err());
        for c in next.conflicts {
            snap = session
                .mutate(
                    &snap.revision,
                    Action::ResolveRebase {
                        draft_id: second.id.clone(),
                        source: next.source.clone(),
                        file_id: c.id,
                        resolution: merge_draft::Resolution::Text {
                            text: format!("resolved-{}\n", c.path),
                        },
                    },
                )
                .unwrap();
        }
        session
            .mutate(&snap.revision, Action::ContinueRebase)
            .unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(oracle.path())
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap()
        };
        assert!(!run(&[
            "-c",
            "core.hooksPath=/dev/null",
            "rebase",
            "--no-fork-point",
            "upstream"
        ])
        .status
        .success());
        assert!(!run(&["rebase", "--skip"]).status.success());
        for path in ["a", "b"] {
            fs::write(oracle.path().join(path), format!("resolved-{path}\n")).unwrap();
        }
        git(oracle.path(), &["add", "-A"]);
        git(
            oracle.path(),
            &["-c", "core.editor=true", "rebase", "--continue"],
        );
        assert_eq!(
            git(d.path(), &["rev-parse", "HEAD^{tree}"]),
            git(oracle.path(), &["rev-parse", "HEAD^{tree}"])
        );
        assert_eq!(
            git(
                d.path(),
                &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
            ),
            git(
                oracle.path(),
                &["log", "--format=%an <%ae>%n%B", "upstream..HEAD"]
            )
        );
    }
    #[test]
    fn equivalent_binary_patches_still_skip_but_different_bytes_do_not() {
        let d = fixture("conflict");
        let repo = Repository::open(d.path()).unwrap();
        let base = repo.revparse_single("topic~2").unwrap().id();
        repo.find_reference("refs/heads/topic")
            .unwrap()
            .delete()
            .unwrap();
        repo.find_reference("refs/heads/upstream")
            .unwrap()
            .delete()
            .unwrap();
        let local = commit(
            &repo,
            "refs/heads/topic",
            ["same\0bytes", "b0\n", "c0\n"],
            &[base],
            "local",
        );
        let upstream = commit(
            &repo,
            "refs/heads/upstream",
            ["same\0bytes", "b0\n", "c0\n"],
            &[base],
            "upstream",
        );
        assert!(patch(&repo, local).unwrap() == patch(&repo, upstream).unwrap());
        let other = commit(
            &repo,
            "refs/heads/other",
            ["other\0bytes", "b0\n", "c0\n"],
            &[base],
            "other",
        );
        assert!(patch(&repo, local).unwrap() != patch(&repo, other).unwrap());
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snap = session.snapshot().unwrap();
        let plan = session.rebase_preview(&snap.revision, "upstream").unwrap();
        assert_eq!(plan.skipped_equivalent, vec![local.to_string()]);
        let done = session
            .mutate(
                &snap.revision,
                Action::Rebase {
                    upstream: "upstream".into(),
                    author: "Tester".into(),
                    email: "test@invalid".into(),
                },
            )
            .unwrap();
        assert_eq!(done.head_oid, Some(upstream.to_string()));
    }
}
