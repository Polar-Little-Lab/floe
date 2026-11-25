//! Fetch the configured upstream and return its guarded merge or rebase workflow.
use super::*;
use crate::network_worker::Downloaded;
#[derive(Clone,Copy,Debug,Deserialize)]
#[serde(rename_all="camelCase")]
pub enum Mode { Ff, FfOnly, Rebase }
pub fn selection_with_mode(session:&mut Session,revision:&str,remote:&str,mode:Option<Mode>)->Result<Selection>{
    let mut selected=selection(session,revision,remote)?;
    match mode{
        Some(Mode::Rebase)=>{selected.rebase=true;selected.ff_policy=merge::FastForwardPolicy::Allow;},
        Some(Mode::Ff)=>{selected.rebase=false;selected.ff_policy=merge::FastForwardPolicy::Allow;},
        Some(Mode::FfOnly)=>{selected.rebase=false;selected.ff_policy=merge::FastForwardPolicy::Only;},
        None=>{},
    }
    Ok(selected)
}
#[derive(Debug, PartialEq, Eq)]
pub struct Selection {
    pub remote: String,
    pub source: String,
    branch: String,
    head: String,
    ff_policy: merge::FastForwardPolicy,
    rebase: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub snapshot: Snapshot,
    #[serde(flatten)]
    pub integration: Integration,
    pub remote: String,
    pub source: String,
}
#[derive(Debug, Serialize)]
#[serde(tag = "mode", content = "plan", rename_all = "camelCase")]
pub enum Integration {
    Merge(MergePlan),
    Rebase(rebase::Plan),
}
#[cfg(test)]
impl Prepared {
    fn merge_plan(&self) -> &MergePlan {
        match &self.integration {
            Integration::Merge(plan) => plan,
            _ => panic!("expected merge"),
        }
    }
}
impl Selection {
    pub fn mode_label(&self) -> &'static str {
        if self.rebase {
            "rebase"
        } else if self.ff_policy==merge::FastForwardPolicy::Only {
            "fast-forward only"
        } else {
            "merge"
        }
    }
}
pub fn selection(session: &mut Session, revision: &str, remote: &str) -> Result<Selection> {
    let snapshot = session.fresh(revision)?;
    if snapshot.detached || snapshot.head_oid.is_none() {
        return Err("PULL_REQUIRES_COMMITTED_BRANCH".into());
    }
    if let Some(block) = snapshot.write_block {
        return Err(format!("READ_ONLY: {block}"));
    }
    if !snapshot.changes.is_empty() {
        return Err("PULL_DIRTY_WORKTREE: commit or stash changes first".into());
    }
    if (sequence_state::exists(&session.root) || rebase::draft::exists(&session.root))
        || replay_draft::exists(&session.root)
        || merge_draft::exists(&session.root)
        || open(&session.root)?.state() != git2::RepositoryState::Clean
    {
        return Err("PULL_PENDING_OPERATION".into());
    }
    let values = remote_remove::entries(&bounded(&session.root.join(".git/config"), 65536)?)?;
    let get = |key: &str| {
        values
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, value)| value.as_str())
    };
    let configured_remote =
        get(&format!("branch.{}.remote", snapshot.branch)).ok_or("PULL_UPSTREAM_NOT_CONFIGURED")?;
    if configured_remote != remote || remote == "." {
        return Err("PULL_REMOTE_MISMATCH".into());
    }
    let merge_key = format!("branch.{}.merge", snapshot.branch);
    let sources = values
        .iter()
        .filter(|(key, _)| key == &merge_key)
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    if sources.len() != 1
        || !sources[0].starts_with("refs/heads/")
        || !git2::Reference::is_valid_name(sources[0])
    {
        return Err("PULL_REQUIRES_SINGLE_UPSTREAM_BRANCH".into());
    }
    let disabled = |value: &str| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        )
    };
    let rebase =
        match get(&format!("branch.{}.rebase", snapshot.branch)).or_else(|| get("pull.rebase")) {
            None => false,
            Some(value) => Config::parse_bool(value).map_err(|_| "PULL_REBASE_MODE_UNSUPPORTED")?,
        };
    if ["pull.twohead", "pull.octopus"]
        .iter()
        .any(|key| get(key).is_some())
        || get(&format!("branch.{}.mergeoptions", snapshot.branch)).is_some()
        || get("merge.renormalize").is_some_and(|v| !disabled(v))
    {
        return Err("PULL_MERGE_OPTIONS_UNSUPPORTED".into());
    }
    let ff_policy = if rebase {
        match get("pull.ff") {
            None => merge::FastForwardPolicy::Allow,
            Some("only") => merge::FastForwardPolicy::Only,
            Some(v) => {
                Config::parse_bool(v).map_err(|_| "UNSUPPORTED_FF_POLICY")?;
                merge::FastForwardPolicy::Allow
            }
        }
    } else {
        merge::ff_policy(&session.root, true)?
    };
    let rebase = rebase && ff_policy != merge::FastForwardPolicy::Only;
    Ok(Selection {
        remote: remote.into(),
        source: sources[0].clone(),
        branch: snapshot.branch,
        head: snapshot.head_oid.unwrap(),
        ff_policy,
        rebase,
    })
}
#[cfg(test)]
pub fn prepare(
    session: &mut Session,
    revision: &str,
    remote: &str,
    download: &Downloaded,
) -> Result<Prepared> {
    prepare_with_options(session,revision,remote,download,None,None)
}
pub fn prepare_with_options(session:&mut Session,revision:&str,remote:&str,download:&Downloaded,mode:Option<Mode>,prune:Option<&[String]>)->Result<Prepared>{
    let selected = selection_with_mode(session, revision, remote,mode)?;
    let plan = fetch::preview_download(session, revision, remote, download)?;
    let mappings = plan
        .mappings
        .iter()
        .filter(|m| m.source == selected.source)
        .collect::<Vec<_>>();
    if mappings.len() != 1 {
        return Err("PULL_UPSTREAM_NOT_FETCHED_OR_AMBIGUOUS".into());
    }
    let destination = mappings[0]
        .destination
        .as_deref()
        .ok_or("PULL_SOURCE_ONLY_UPSTREAM_UNSUPPORTED")?
        .to_owned();
    let target = mappings[0].after.clone();
    let prior_tracking = mappings[0].before.clone();
    if selection_with_mode(session, revision, remote,mode)? != selected {
        return Err("STALE_STATE".into());
    }
    let snapshot = fetch::execute(session, revision, remote, download)?;
    let snapshot=if let Some(approved)=prune{
        fetch::prune_confirmed(session,snapshot,remote,download,approved).map_err(|error|format!("PULL_FETCH_COMPLETED_PRUNE_FAILED: Fetch completed; integration not started. {error}"))?
    }else{snapshot};
    if selection_with_mode(session, &snapshot.revision, remote,mode)? != selected {
        return Err("PULL_BRANCH_CHANGED_AFTER_FETCH".into());
    }
    let prefix=match mode{Some(Mode::Ff)=>"pull-ff",Some(Mode::FfOnly)=>"pull-ff-only",_=>"remote"};
    let upstream = format!("{prefix}:{destination}");
    let integration = if selected.rebase {
        // A force-updated upstream needs Git's fork-point/--onto range semantics.
        // Refuse rather than resurrect commits removed from upstream.
        let repo = open(&session.root)?;
        if let Some(before) = prior_tracking {
            let old = Oid::from_str(&before).map_err(err)?;
            let new = Oid::from_str(&target).map_err(err)?;
            if old != new && !repo.graph_descendant_of(new, old).map_err(err)? {
                return Err("PULL_REBASE_REWRITTEN_UPSTREAM: fetch completed; fork-point integration required; local branch unchanged".into());
            }
        }
        // Keep refusing after an earlier Fetch already installed a rewritten
        // tip. Old tracking tips in the reflog can still be the local fork point.
        let head = Oid::from_str(&selected.head).map_err(err)?;
        let new = Oid::from_str(&target).map_err(err)?;
        let tracking_log = match repo.reflog(&destination) {
            Ok(log) => Some(log),
            Err(error) if error.code() == git2::ErrorCode::NotFound => None,
            Err(error) => return Err(format!("PULL_REBASE_REFLOG: {error}")),
        };
        if let Some(log) = tracking_log {
            if log.len() > 4096 {
                return Err(
                    "PULL_REBASE_REFLOG_LIMIT: fetch completed; local branch unchanged".into(),
                );
            }
            for entry in log.iter() {
                for old in [entry.id_old(), entry.id_new()] {
                    if old.is_zero() || old == new || repo.find_commit(old).is_err() {
                        continue;
                    }
                    if (old == head || repo.graph_descendant_of(head, old).map_err(err)?)
                        && !repo.graph_descendant_of(new, old).map_err(err)?
                    {
                        return Err("PULL_REBASE_REWRITTEN_UPSTREAM: fetch completed; historical fork-point integration required; local branch unchanged".into());
                    }
                }
            }
        }
        let plan = session.rebase_preview(&snapshot.revision, &upstream)?;
        if plan.upstream_oid != target {
            return Err("PULL_UPSTREAM_CHANGED_AFTER_FETCH".into());
        }
        Integration::Rebase(plan)
    } else {
        let plan = merge::preview(session, &snapshot.revision, &upstream)?;
        if plan.target_oid != target {
            return Err("PULL_UPSTREAM_CHANGED_AFTER_FETCH".into());
        }
        if selected.ff_policy == merge::FastForwardPolicy::Only
            && !matches!(plan.kind.as_str(), "upToDate" | "fastForward")
        {
            return Err(
                "PULL_FF_ONLY: fetch completed, but upstream has diverged; local branch unchanged"
                    .into(),
            );
        }
        Integration::Merge(plan)
    };
    Ok(Prepared {
        snapshot,
        integration,
        remote: selected.remote,
        source: selected.source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_worker::{Advertisement, RemoteRef};
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const URL: &str = "https://example.com/pull.git";
    fn git(root: &Path, args: &[&str]) -> std::process::Output {
        Command::new(crate::test_git_executable())
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
    }
    fn success(root: &Path, args: &[&str]) -> String {
        let output = git(root, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn commit(repo: &Repository, path: &str, content: &[u8]) -> Oid {
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let tree = parent.as_ref().map(|parent| parent.tree().unwrap());
        let mut builder = repo.treebuilder(tree.as_ref()).unwrap();
        builder
            .insert(path, repo.blob(content).unwrap(), 0o100644)
            .unwrap();
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "fixture",
            &tree,
            &parent.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn fixture(kind: &str) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let source = Repository::init_bare(temp.path().join("source")).unwrap();
        source.set_head("refs/heads/main").unwrap();
        commit(&source, "shared.txt", b"base\n");
        success(
            temp.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "clone",
                "--template=",
                source.path().to_str().unwrap(),
                temp.path().join("target").to_str().unwrap(),
            ],
        );
        let target = Repository::open(temp.path().join("target")).unwrap();
        target
            .config()
            .unwrap()
            .set_str("remote.origin.url", URL)
            .unwrap();
        if matches!(kind, "mergeCommit" | "conflicts") {
            let file = if kind == "conflicts" {
                "shared.txt"
            } else {
                "local.txt"
            };
            let id = commit(&target, file, b"local\n");
            fs::write(temp.path().join("target").join(file), "local\n").unwrap();
            let mut index = target.index().unwrap();
            index
                .read_tree(&target.find_commit(id).unwrap().tree().unwrap())
                .unwrap();
            index.write().unwrap();
        }
        if kind != "upToDate" {
            commit(
                &source,
                if kind == "mergeCommit" {
                    "remote.txt"
                } else {
                    "shared.txt"
                },
                b"remote\n",
            );
        }
        temp
    }
    fn downloaded(root: &Path) -> Downloaded {
        let source = Repository::open_bare(root.join("source")).unwrap();
        let oid = source.head().unwrap().target().unwrap().to_string();
        Downloaded::fixture(
            &source,
            Advertisement {
                endpoint: URL.into(),
                refs: vec![
                    RemoteRef {
                        name: "HEAD".into(),
                        oid: oid.clone(),
                        symbolic_target: Some("refs/heads/main".into()),
                    },
                    RemoteRef {
                        name: "refs/heads/main".into(),
                        oid,
                        symbolic_target: None,
                    },
                ],
            },
        )
    }
    fn bytes(root: &Path) -> Vec<Vec<u8>> {
        [".git/HEAD", ".git/index", ".git/config", "shared.txt"]
            .iter()
            .map(|p| fs::read(root.join(p)).unwrap())
            .collect()
    }
    #[test]
    fn pull_merge_flows_match_git_tree_parents_and_preserve_fetch_preview_worktree() {
        for (kind, noff) in [
            ("upToDate", false),
            ("fastForward", false),
            ("mergeCommit", false),
            ("conflicts", false),
            ("upToDate", true),
            ("fastForward", true),
            ("mergeCommit", true),
            ("conflicts", true),
        ] {
            let temp = fixture(kind);
            let root = temp.path().join("target").canonicalize().unwrap();
            if noff {
                Repository::open(&root)
                    .unwrap()
                    .config()
                    .unwrap()
                    .set_bool("pull.ff", false)
                    .unwrap();
            }
            let oracle = temp.path().join("oracle");
            success(
                temp.path(),
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "clone",
                    "--template=",
                    temp.path().join("target").to_str().unwrap(),
                    oracle.to_str().unwrap(),
                ],
            );
            success(
                &oracle,
                &[
                    "remote",
                    "set-url",
                    "origin",
                    temp.path().join("source").to_str().unwrap(),
                ],
            );
            success(
                &oracle,
                &["-c", "protocol.file.allow=always", "fetch", "origin"],
            );
            let before = bytes(&root);
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let prepared = prepare(
                &mut session,
                &snapshot.revision,
                "origin",
                &downloaded(temp.path()),
            )
            .unwrap();
            let expected = if noff && kind == "fastForward" {
                "mergeCommit"
            } else {
                kind
            };
            assert_eq!(prepared.merge_plan().kind, expected);
            assert_eq!(bytes(&root), before);
            assert_eq!(
                prepared.merge_plan().target_branch,
                "remote:refs/remotes/origin/main"
            );
            let mut snapshot = prepared.snapshot.clone();
            match expected {
                "fastForward" => {
                    snapshot = session
                        .mutate(
                            &snapshot.revision,
                            Action::FastForwardMerge {
                                name: prepared.merge_plan().target_branch.clone(),
                            },
                        )
                        .unwrap();
                }
                "mergeCommit" => {
                    snapshot = session
                        .mutate(
                            &snapshot.revision,
                            Action::MergeCommit {
                                name: prepared.merge_plan().target_branch.clone(),
                                message: "Merge upstream".into(),
                                author: "Fixture".into(),
                                email: "fixture@example.invalid".into(),
                            },
                        )
                        .unwrap();
                }
                "conflicts" => {
                    snapshot = session
                        .mutate(
                            &snapshot.revision,
                            Action::StartMerge {
                                name: prepared.merge_plan().target_branch.clone(),
                            },
                        )
                        .unwrap();
                    let conflict = &prepared.merge_plan().conflicts[0];
                    snapshot = session
                        .mutate(
                            &snapshot.revision,
                            Action::ResolveMerge {
                                file_id: conflict.ours.as_ref().unwrap().id.clone(),
                                resolution: merge_draft::Resolution::Ours {},
                            },
                        )
                        .unwrap();
                    snapshot = session
                        .mutate(
                            &snapshot.revision,
                            Action::ContinueMerge {
                                message: "Merge upstream".into(),
                                author: "Fixture".into(),
                                email: "fixture@example.invalid".into(),
                            },
                        )
                        .unwrap();
                }
                _ => {}
            }
            let output = git(
                &oracle,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "merge",
                    if noff { "--no-ff" } else { "--ff" },
                    "--no-edit",
                    "origin/main",
                ],
            );
            if kind == "conflicts" {
                assert!(!output.status.success());
                fs::write(oracle.join("shared.txt"), "local\n").unwrap();
                success(&oracle, &["add", "shared.txt"]);
                success(
                    &oracle,
                    &[
                        "-c",
                        "user.name=Fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                        "commit",
                        "-m",
                        "Merge upstream",
                    ],
                );
            } else {
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            assert_eq!(
                success(&root, &["rev-parse", "HEAD^{tree}"]),
                success(&oracle, &["rev-parse", "HEAD^{tree}"])
            );
            assert_eq!(
                success(&root, &["show", "-s", "--format=%P", "HEAD"]),
                success(&oracle, &["show", "-s", "--format=%P", "HEAD"])
            );
            assert_eq!(snapshot.branch, "main");
            assert!(snapshot.changes.is_empty());
            assert!(snapshot.merge_draft.is_none());
            success(&root, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn local_merge_policy_ignores_pull_override_and_only_blocks_divergence() {
        for kind in ["fastForward", "mergeCommit"] {
            let temp = fixture(kind);
            let root = temp.path().join("target").canonicalize().unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let prepared = prepare(
                &mut session,
                &snapshot.revision,
                "origin",
                &downloaded(temp.path()),
            )
            .unwrap();
            let repo = Repository::open(&root).unwrap();
            repo.reference(
                "refs/heads/incoming",
                Oid::from_str(&prepared.merge_plan().target_oid).unwrap(),
                false,
                "fixture",
            )
            .unwrap();
            let mut config = repo.config().unwrap();
            config
                .set_str(
                    "merge.ff",
                    if kind == "fastForward" {
                        "false"
                    } else {
                        "only"
                    },
                )
                .unwrap();
            config.set_bool("pull.ff", true).unwrap();
            let snapshot = session.snapshot().unwrap();
            let plan = merge::preview(&mut session, &snapshot.revision, "incoming").unwrap();
            assert_eq!(plan.kind, "mergeCommit");
            let before = bytes(&root);
            let action = Action::MergeCommit {
                name: "incoming".into(),
                message: "Merge incoming".into(),
                author: "Fixture".into(),
                email: "fixture@example.invalid".into(),
            };
            if kind == "fastForward" {
                assert!(plan.blockers.is_empty());
                let next = session.mutate(&snapshot.revision, action).unwrap();
                let head = repo
                    .find_commit(Oid::from_str(next.head_oid.as_ref().unwrap()).unwrap())
                    .unwrap();
                assert_eq!(head.parent_count(), 2);
                assert_eq!(
                    head.parent_id(0).unwrap().to_string(),
                    snapshot.head_oid.unwrap()
                );
                assert_eq!(
                    head.parent_id(1).unwrap().to_string(),
                    prepared.merge_plan().target_oid
                );
                assert_eq!(head.tree_id(), head.parent(1).unwrap().tree_id());
                assert!(next.changes.is_empty());
            } else {
                assert!(plan.blockers.iter().any(|b| b.contains("FF_ONLY")));
                assert!(session
                    .mutate(&snapshot.revision, action)
                    .unwrap_err()
                    .contains("FF_ONLY"));
                assert_eq!(bytes(&root), before);
            }
        }
    }
    #[test]
    fn pull_ff_policy_precedence_and_mutation_guards() {
        for (merge_ff, pull_ff, expected) in [
            ("false", None, "mergeCommit"),
            ("false", Some("TRUE"), "fastForward"),
            ("true", Some("FALSE"), "mergeCommit"),
            ("false", Some("only"), "fastForward"),
            ("true", Some("0"), "mergeCommit"),
            ("false", Some("2"), "fastForward"),
        ] {
            let temp = fixture("fastForward");
            let root = temp.path().join("target").canonicalize().unwrap();
            let repo = Repository::open(&root).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("merge.ff", merge_ff).unwrap();
            if let Some(v) = pull_ff {
                config.set_str("pull.ff", v).unwrap();
            }
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let prepared = prepare(
                &mut session,
                &snapshot.revision,
                "origin",
                &downloaded(temp.path()),
            )
            .unwrap();
            assert_eq!(prepared.merge_plan().kind, expected);
            let before = bytes(&root);
            let action = if expected == "mergeCommit" {
                Action::FastForwardMerge {
                    name: prepared.merge_plan().target_branch.clone(),
                }
            } else {
                Action::MergeCommit {
                    name: prepared.merge_plan().target_branch.clone(),
                    message: "merge".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                }
            };
            assert!(session.mutate(&prepared.snapshot.revision, action).is_err());
            assert_eq!(bytes(&root), before);
            // A policy edit after preview invalidates the displayed plan revision.
            config.set_str("pull.ff", "false").unwrap();
            config.set_str("test.marker", "changed").unwrap();
            assert!(session
                .mutate(
                    &prepared.snapshot.revision,
                    Action::FastForwardMerge {
                        name: prepared.merge_plan().target_branch.clone()
                    }
                )
                .unwrap_err()
                .contains("STALE_STATE"));
        }
    }
    #[test]
    fn pull_preflight_rejects_dirty_missing_upstream_and_unsupported_policies_without_fetch() {
        for mode in [
            "dirty",
            "detached",
            "missing",
            "remote",
            "rebase",
            "invalid-ff",
            "strategy",
            "source-only",
            "excluded",
        ] {
            let temp = fixture("fastForward");
            let root = temp.path().join("target").canonicalize().unwrap();
            let repo = Repository::open(&root).unwrap();
            let mut config = repo.config().unwrap();
            match mode {
                "dirty" => fs::write(root.join("shared.txt"), "dirty\n").unwrap(),
                "detached" => repo
                    .set_head_detached(repo.head().unwrap().target().unwrap())
                    .unwrap(),
                "missing" => config.remove_multivar("branch.main.merge", ".*").unwrap(),
                "remote" => config.set_str("branch.main.remote", "other").unwrap(),
                "rebase" => config.set_str("pull.rebase", "merges").unwrap(),
                "invalid-ff" => config.set_str("pull.ff", "invalid").unwrap(),
                "strategy" => config.set_str("pull.twohead", "ours").unwrap(),
                "source-only" => config
                    .set_str("remote.origin.fetch", "refs/heads/main")
                    .unwrap(),
                "excluded" => config
                    .set_multivar("remote.origin.fetch", "a^", "^refs/heads/main")
                    .unwrap(),
                _ => unreachable!(),
            }
            let before = bytes(&root);
            let old = repo.refname_to_id("refs/remotes/origin/main").unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert!(
                prepare(
                    &mut session,
                    &snapshot.revision,
                    "origin",
                    &downloaded(temp.path())
                )
                .is_err(),
                "{mode}"
            );
            assert_eq!(bytes(&root), before);
            assert_eq!(repo.refname_to_id("refs/remotes/origin/main").unwrap(), old);
            assert!(!root.join(".git/FETCH_HEAD").exists());
        }
    }
    #[test]
    fn pull_ff_only_fetches_but_refuses_divergence_and_remote_merge_rejects_stale_target() {
        let temp = fixture("mergeCommit");
        let root = temp.path().join("target").canonicalize().unwrap();
        let repo = Repository::open(&root).unwrap();
        repo.config().unwrap().set_str("pull.ff", "only").unwrap();
        let before = bytes(&root);
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(prepare(
            &mut session,
            &snapshot.revision,
            "origin",
            &downloaded(temp.path())
        )
        .unwrap_err()
        .contains("PULL_FF_ONLY"));
        assert_eq!(bytes(&root), before);
        assert!(root.join(".git/FETCH_HEAD").exists());
        repo.config().unwrap().set_bool("pull.ff", true).unwrap();
        let snapshot = session.snapshot().unwrap();
        let prepared = prepare(
            &mut session,
            &snapshot.revision,
            "origin",
            &downloaded(temp.path()),
        )
        .unwrap();
        repo.reference(
            "refs/remotes/origin/main",
            repo.head().unwrap().target().unwrap(),
            true,
            "external",
        )
        .unwrap();
        assert!(session
            .mutate(
                &prepared.snapshot.revision,
                Action::MergeCommit {
                    name: prepared.merge_plan().target_branch.clone(),
                    message: "Merge".into(),
                    author: "Fixture".into(),
                    email: "fixture@example.invalid".into()
                }
            )
            .unwrap_err()
            .contains("STALE_STATE"));
        assert_eq!(fs::read(root.join("shared.txt")).unwrap(), before[3]);
    }
    #[test]
    fn remote_merge_crash_child() {
        let Some(root) = std::env::var_os("BRANCHDESK_PULL_ROOT") else {
            return;
        };
        let mut session = Session::new(PathBuf::from(root)).unwrap();
        let snapshot = session.snapshot().unwrap();
        let action = if std::env::var_os("BRANCHDESK_PULL_MERGE").is_some() {
            Action::MergeCommit {
                name: "remote:refs/remotes/origin/main".into(),
                message: "Merge upstream".into(),
                author: "Fixture".into(),
                email: "fixture@example.invalid".into(),
            }
        } else {
            Action::FastForwardMerge {
                name: "remote:refs/remotes/origin/main".into(),
            }
        };
        session.mutate(&snapshot.revision, action).unwrap();
    }
    #[test]
    fn remote_merge_journal_recovers_actual_kills_without_moving_tracking_ref() {
        for kind in ["fastForward", "mergeCommit", "noff"] {
            for step in [0, 1, 2, 3, 100, 101, 102] {
                let temp = fixture(kind);
                let root = temp.path().join("target").canonicalize().unwrap();
                if kind == "noff" {
                    Repository::open(&root)
                        .unwrap()
                        .config()
                        .unwrap()
                        .set_bool("pull.ff", false)
                        .unwrap();
                }
                let mut session = Session::new(root.clone()).unwrap();
                let snapshot = session.snapshot().unwrap();
                let prepared = prepare(
                    &mut session,
                    &snapshot.revision,
                    "origin",
                    &downloaded(temp.path()),
                )
                .unwrap();
                let old_head = prepared.snapshot.head_oid.clone();
                let before = bytes(&root);
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "repository::pull::tests::remote_merge_crash_child",
                        "--nocapture",
                    ])
                    .env("BRANCHDESK_PULL_ROOT", &root)
                    .env("BRANCHDESK_CRASH_ROOT", &root)
                    .env("BRANCHDESK_CRASH_STEP", step.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                if kind != "fastForward" {
                    command.env("BRANCHDESK_PULL_MERGE", "1");
                }
                let mut child = command.spawn().unwrap();
                let ready = root.join(".git/branchdesk-checkout/child.ready");
                let deadline = Instant::now() + Duration::from_secs(15);
                while !ready.exists() {
                    if Instant::now() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("remote merge checkpoint timeout {kind} {step}");
                    }
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "remote merge child exited {kind} {step}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                child.kill().unwrap();
                child.wait().unwrap();
                let snapshot = session.snapshot().unwrap();
                assert!(snapshot.recovery_pending);
                let recovered = session
                    .mutate(&snapshot.revision, Action::RecoverCheckout)
                    .unwrap();
                assert!(!recovered.recovery_pending);
                assert_eq!(recovered.head_oid, old_head);
                assert_eq!(bytes(&root), before);
                assert_eq!(
                    Repository::open(&root)
                        .unwrap()
                        .refname_to_id("refs/remotes/origin/main")
                        .unwrap()
                        .to_string(),
                    prepared.merge_plan().target_oid
                );
            }
        }
    }
    #[test]
    fn pull_rebase_policy_boolean_precedence_and_ff_only_match_git_selection() {
        for (global, branch, ff, merge_ff, rebase_mode) in [
            ("true", None, None, None, true),
            ("false", Some("true"), None, None, true),
            ("true", Some("false"), None, None, false),
            ("yes", None, None, None, true),
            ("2", None, None, None, true),
            ("true", None, Some("only"), None, false),
            ("true", None, Some("false"), Some("only"), true),
            ("true", None, None, Some("only"), true),
        ] {
            let temp = fixture("fastForward");
            let root = temp.path().join("target");
            let repo = Repository::open(&root).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("pull.rebase", global).unwrap();
            if let Some(v) = branch {
                config.set_str("branch.main.rebase", v).unwrap();
            }
            if let Some(v) = ff {
                config.set_str("pull.ff", v).unwrap();
            }
            if let Some(v) = merge_ff {
                config.set_str("merge.ff", v).unwrap();
            }
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let selected = selection(&mut session, &snap.revision, "origin").unwrap();
            assert_eq!(
                selected.rebase, rebase_mode,
                "{global} {branch:?} {ff:?} {merge_ff:?}"
            );
            let prepared = prepare(
                &mut session,
                &snap.revision,
                "origin",
                &downloaded(temp.path()),
            )
            .unwrap();
            let serialized = serde_json::to_value(&prepared).unwrap();
            assert_eq!(
                serialized["mode"],
                if rebase_mode { "rebase" } else { "merge" }
            );
            assert!(serialized["plan"].is_object());
            config
                .set_str(
                    "branch.main.rebase",
                    if rebase_mode { "false" } else { "true" },
                )
                .unwrap();
            assert!(
                selection(&mut session, &prepared.snapshot.revision, "origin")
                    .unwrap_err()
                    .contains("STALE_STATE")
            );
        }
        for value in ["merges", "m", "interactive", "i", "invalid"] {
            let temp = fixture("fastForward");
            let root = temp.path().join("target");
            Repository::open(&root)
                .unwrap()
                .config()
                .unwrap()
                .set_str("pull.rebase", value)
                .unwrap();
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            let before = bytes(&root);
            assert!(selection(&mut s, &snap.revision, "origin")
                .unwrap_err()
                .contains("REBASE_MODE_UNSUPPORTED"));
            assert_eq!(bytes(&root), before);
        }
    }
    #[test]
    fn pull_rebase_clean_and_conflict_workflows_match_git_with_fetch_preservation() {
        for kind in ["upToDate", "fastForward", "mergeCommit", "conflicts"] {
            let temp = fixture(kind);
            let root = temp.path().join("target").canonicalize().unwrap();
            let oracle = temp.path().join("oracle");
            success(
                temp.path(),
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "clone",
                    "--template=",
                    temp.path().join("target").to_str().unwrap(),
                    oracle.to_str().unwrap(),
                ],
            );
            success(
                &oracle,
                &[
                    "remote",
                    "set-url",
                    "origin",
                    temp.path().join("source").to_str().unwrap(),
                ],
            );
            // Cloning the local branch labels its tip as origin/main. Restore
            // the original tracking ref and discard that artificial reflog so
            // Git's pull fork-point logic sees the same pre-fetch history.
            let tracking = success(&root, &["rev-parse", "refs/remotes/origin/main"]);
            success(&oracle, &["update-ref", "-d", "refs/remotes/origin/main"]);
            success(
                &oracle,
                &["update-ref", "refs/remotes/origin/main", tracking.trim()],
            );
            for path in [&root, &oracle] {
                let repo = Repository::open(path).unwrap();
                repo.config()
                    .unwrap()
                    .set_bool("pull.rebase", true)
                    .unwrap();
            }
            let before = bytes(&root);
            let original = success(&root, &["rev-parse", "HEAD"]);
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            let prepared =
                prepare(&mut s, &snap.revision, "origin", &downloaded(temp.path())).unwrap();
            assert_eq!(bytes(&root), before);
            let Integration::Rebase(plan) = prepared.integration else {
                panic!("expected rebase");
            };
            assert_eq!(success(&root, &["rev-parse", "HEAD"]), original);
            let mut current = prepared.snapshot;
            if kind == "conflicts" {
                current = s
                    .mutate(
                        &current.revision,
                        Action::StartRebase {
                            upstream: plan.upstream,
                            author: "Fixture".into(),
                            email: "fixture@example.invalid".into(),
                        },
                    )
                    .unwrap();
                let view = s.rebase_draft_preview(&current.revision).unwrap();
                let pending = view.pending.unwrap();
                current = s
                    .mutate(
                        &current.revision,
                        Action::ResolveRebase {
                            draft_id: view.id,
                            source: pending.source,
                            file_id: pending.conflicts[0].id.clone(),
                            resolution: merge_draft::Resolution::Theirs {},
                        },
                    )
                    .unwrap();
                current = s.mutate(&current.revision, Action::ContinueRebase).unwrap();
            } else if plan.kind != "upToDate" {
                current = s
                    .mutate(
                        &current.revision,
                        Action::Rebase {
                            upstream: plan.upstream,
                            author: "Fixture".into(),
                            email: "fixture@example.invalid".into(),
                        },
                    )
                    .unwrap();
            }
            let out = git(
                &oracle,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "pull",
                    "--no-edit",
                ],
            );
            if kind == "conflicts" {
                assert!(!out.status.success());
                fs::write(oracle.join("shared.txt"), "local\n").unwrap();
                success(&oracle, &["add", "shared.txt"]);
                success(
                    &oracle,
                    &[
                        "-c",
                        "core.editor=true",
                        "-c",
                        "user.name=Fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                        "rebase",
                        "--continue",
                    ],
                );
            } else {
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            assert_eq!(
                success(&root, &["rev-parse", "HEAD^{tree}"]),
                success(&oracle, &["rev-parse", "HEAD^{tree}"]),
                "{kind}"
            );
            assert_eq!(
                success(
                    &root,
                    &["log", "--format=%an <%ae>%n%B", "origin/main..HEAD"]
                ),
                success(
                    &oracle,
                    &["log", "--format=%an <%ae>%n%B", "origin/main..HEAD"]
                ),
                "{kind}"
            );
            assert_eq!(
                success(&root, &["rev-list", "--count", "origin/main..HEAD"]),
                success(&oracle, &["rev-list", "--count", "origin/main..HEAD"])
            );
            assert!(current.changes.is_empty());
            assert!(current.rebase_draft.is_none());
            success(&root, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn pull_rebase_ff_only_and_force_updated_upstream_leave_local_branch_unchanged() {
        for force_update in [false, true] {
            let temp = fixture("mergeCommit");
            let root = temp.path().join("target");
            let repo = Repository::open(&root).unwrap();
            let mut config = repo.config().unwrap();
            config.set_bool("pull.rebase", true).unwrap();
            if force_update {
                let source = Repository::open(temp.path().join("source")).unwrap();
                let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
                let tree = source.head().unwrap().peel_to_tree().unwrap();
                let replacement = source
                    .commit(None, &sig, &sig, "rewritten root", &tree, &[])
                    .unwrap();
                source
                    .reference("refs/heads/main", replacement, true, "force fixture")
                    .unwrap();
            } else {
                config.set_str("pull.ff", "only").unwrap();
            }
            let original = success(&root, &["rev-parse", "HEAD"]);
            let before = bytes(&root);
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            let error =
                prepare(&mut s, &snap.revision, "origin", &downloaded(temp.path())).unwrap_err();
            assert!(
                error.contains(if force_update {
                    "REWRITTEN_UPSTREAM"
                } else {
                    "PULL_FF_ONLY"
                }),
                "{error}"
            );
            assert_eq!(success(&root, &["rev-parse", "HEAD"]), original);
            assert_eq!(bytes(&root), before);
            assert!(root.join(".git/FETCH_HEAD").exists());
            // Fetch persists even when integration refuses. A second Pull must
            // not forget the force update and resurrect removed upstream work.
            let refreshed = s.snapshot().unwrap();
            let repeated = prepare(
                &mut s,
                &refreshed.revision,
                "origin",
                &downloaded(temp.path()),
            )
            .unwrap_err();
            assert!(
                repeated.contains(if force_update {
                    "REWRITTEN_UPSTREAM"
                } else {
                    "PULL_FF_ONLY"
                }),
                "{repeated}"
            );
            assert_eq!(bytes(&root), before);
            assert_eq!(success(&root, &["rev-parse", "HEAD"]), original);
        }
    }
}
