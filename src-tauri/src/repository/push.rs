//! Explicit single-branch Push planning. No objects or refs are written here.
use super::*;
use crate::network_worker::Advertisement;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub remote: String,
    pub endpoint: String,
    pub source: String,
    pub destination: String,
    pub source_oid: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: String,
    #[serde(flatten)]
    pub selection: Selection,
    pub remote_oid: Option<String>,
    pub kind: String,
    pub outgoing: Vec<String>,
    pub outgoing_truncated: bool,
    pub blockers: Vec<String>,
    pub pack_bytes: Option<u64>,
    pub pack_objects: Option<usize>,
}
pub fn selection(
    session: &mut Session,
    revision: &str,
    name: &str,
    branch: &str,
) -> Result<Selection> {
    let snapshot = session.fresh(revision)?;
    if push_state::exists(&session.root) {
        return Err("PUSH_PENDING_RESULT".into());
    }
    if name.is_empty() || name.len() > 200 {
        return Err("INVALID_REMOTE_NAME".into());
    }
    let destination = format!("refs/heads/{branch}");
    if branch.is_empty()
        || branch.len() > 1024
        || branch.starts_with('-')
        || !git2::Reference::is_valid_name(&destination)
    {
        return Err("INVALID_PUSH_BRANCH".into());
    }
    if snapshot.detached || snapshot.head_oid.is_none() {
        return Err("PUSH_REQUIRES_COMMITTED_BRANCH".into());
    }
    if let Some(blocker) = snapshot.write_block {
        return Err(format!("READ_ONLY: {blocker}"));
    }
    let repo = open(&session.root)?;
    if repo.state() != git2::RepositoryState::Clean
        || sequence_state::exists(&session.root)
        || rebase::draft::exists(&session.root)
        || merge_draft::exists(&session.root)
        || replay_draft::exists(&session.root)
    {
        return Err("PUSH_PENDING_OPERATION".into());
    }
    let entries = remote_remove::entries(&bounded(&session.root.join(".git/config"), 65536)?)?;
    let values = |key: &str| {
        entries
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
    };
    if entries
        .iter()
        .any(|(k, _)| k.starts_with("url.") || k == &format!("remote.{name}.receivepack"))
    {
        return Err("PUSH_TRANSPORT_CONFIG_UNSUPPORTED".into());
    }
    for value in values(&format!("remote.{name}.mirror")) {
        if Config::parse_bool(&value).map_err(err)? {
            return Err("PUSH_MIRROR_UNSUPPORTED".into());
        }
    }
    let push = values(&format!("remote.{name}.pushurl"));
    let urls = if push.is_empty() {
        values(&format!("remote.{name}.url"))
    } else {
        push
    };
    if urls.len() != 1 {
        return Err("PUSH_REQUIRES_SINGLE_URL".into());
    }
    let endpoint = remote_endpoint::https(&urls[0])?.to_string();
    Ok(Selection {
        remote: name.into(),
        endpoint,
        source: format!("refs/heads/{}", snapshot.branch),
        destination,
        source_oid: snapshot.head_oid.unwrap(),
    })
}
pub fn preview(
    session: &mut Session,
    revision: &str,
    selected: &Selection,
    ad: &Advertisement,
) -> Result<Plan> {
    let branch = selected
        .destination
        .strip_prefix("refs/heads/")
        .ok_or("INVALID_PUSH_BRANCH")?;
    if selection(session, revision, &selected.remote, branch)? != *selected {
        return Err("STALE_PUSH_SELECTION".into());
    }
    if ad.endpoint != selected.endpoint || ad.refs.len() > 2000 {
        return Err("PUSH_ADVERTISEMENT_INVALID".into());
    }
    let mut names = std::collections::HashSet::new();
    for r in &ad.refs {
        let plain = r.name.strip_suffix("^{}").unwrap_or(&r.name);
        if r.oid.len() != 40
            || Oid::from_str(&r.oid).map_err(err)?.is_zero()
            || !names.insert(&r.name)
            || !(plain == "HEAD"
                || plain.starts_with("refs/") && git2::Reference::is_valid_name(plain))
        {
            return Err("PUSH_ADVERTISEMENT_INVALID".into());
        }
    }
    let repo = open(&session.root)?;
    let source = Oid::from_str(&selected.source_oid).map_err(err)?;
    let remote = ad.refs.iter().find(|r| r.name == selected.destination);
    let mut blockers = Vec::new();
    let kind = if let Some(remote) = remote {
        if remote.symbolic_target.is_some() {
            blockers.push("PUSH_SYMBOLIC_TARGET_UNSUPPORTED".into());
            "blocked"
        } else {
            let old = Oid::from_str(&remote.oid).map_err(err)?;
            if old == source {
                "upToDate"
            } else if repo.find_commit(old).is_err() {
                blockers.push("PUSH_FETCH_REQUIRED".into());
                "unknownRemote"
            } else if repo.graph_descendant_of(source, old).map_err(err)? {
                "fastForward"
            } else {
                blockers.push("PUSH_NON_FAST_FORWARD".into());
                "nonFastForward"
            }
        }
    } else if ad.refs.iter().any(|r| {
        r.name.starts_with(&(selected.destination.clone() + "/"))
            || selected.destination.starts_with(&(r.name.clone() + "/"))
    }) {
        blockers.push("PUSH_REF_NAMESPACE_CONFLICT".into());
        "blocked"
    } else {
        "createBranch"
    };
    let mut outgoing = Vec::new();
    let mut outgoing_truncated = false;
    if blockers.is_empty() && kind != "upToDate" {
        let mut walk = repo.revwalk().map_err(err)?;
        walk.set_sorting(Sort::TOPOLOGICAL).map_err(err)?;
        walk.push(source).map_err(err)?;
        if let Some(remote) = remote {
            walk.hide(Oid::from_str(&remote.oid).map_err(err)?)
                .map_err(err)?;
        }
        for oid in walk {
            if outgoing.len() == 200 {
                outgoing_truncated = true;
                break;
            }
            outgoing.push(oid.map_err(err)?.to_string());
        }
    }
    let pack_summary = if blockers.is_empty() && kind != "upToDate" {
        Some(crate::network_worker::push_pack::prepare(&repo, source)?.summary()?)
    } else {
        None
    };
    session.fresh(revision)?;
    Ok(Plan {
        revision: revision.into(),
        selection: selected.clone(),
        remote_oid: remote.map(|r| r.oid.clone()),
        kind: kind.into(),
        outgoing,
        outgoing_truncated,
        blockers,
        pack_bytes: pack_summary.map(|(bytes, _)| bytes),
        pack_objects: pack_summary.map(|(_, objects)| objects),
    })
}

pub fn prepare_pack(
    session: &mut Session,
    revision: &str,
    selected: &Selection,
) -> Result<crate::network_worker::push_pack::PreparedPack> {
    let branch = selected
        .destination
        .strip_prefix("refs/heads/")
        .ok_or("INVALID_PUSH_BRANCH")?;
    if selection(session, revision, &selected.remote, branch)? != *selected {
        return Err("STALE_PUSH_SELECTION".into());
    }
    let prepared = crate::network_worker::push_pack::prepare(
        &open(&session.root)?,
        Oid::from_str(&selected.source_oid).map_err(err)?,
    )?;
    session.fresh(revision)?;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::super::remote_remove::tests::git;
    use super::*;
    use crate::network_worker::RemoteRef;
    fn fixture() -> (tempfile::TempDir, Oid, Oid, Oid) {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init(d.path().join("local")).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let create = |reference: &str, text: &[u8], parent: Option<Oid>| {
            let mut b = repo.treebuilder(None).unwrap();
            b.insert("file", repo.blob(text).unwrap(), 0o100644)
                .unwrap();
            let tree = repo.find_tree(b.write().unwrap()).unwrap();
            let sig = git2::Signature::now("Push test", "push@invalid").unwrap();
            let parent = parent.map(|p| repo.find_commit(p).unwrap());
            repo.commit(
                Some(reference),
                &sig,
                &sig,
                "fixture",
                &tree,
                &parent.iter().collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let a = create("refs/heads/main", b"base", None);
        let b = create("refs/heads/main", b"local", Some(a));
        let c = create("refs/heads/other", b"remote", Some(a));
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        repo.remote("origin", "https://example.com/fetch.git")
            .unwrap();
        repo.config()
            .unwrap()
            .set_str("remote.origin.pushurl", "https://example.com/push.git")
            .unwrap();
        (d, a, b, c)
    }
    #[test]
    fn preview_matches_git_dry_run_without_repository_or_remote_mutation() {
        for (kind, destination) in ["new", "ff", "same", "diverged"]
            .into_iter()
            .flat_map(|kind| ["main", "release/v2"].map(move |destination| (kind, destination)))
        {
            let target = format!("refs/heads/{destination}");
            let (d, a, b, c) = fixture();
            let root = d.path().join("local");
            let server = d.path().join("server");
            git(
                d.path(),
                &[
                    "clone",
                    "--bare",
                    "--template=",
                    root.to_str().unwrap(),
                    server.to_str().unwrap(),
                ],
            );
            let old = match kind {
                "new" => None,
                "ff" => Some(a),
                "same" => Some(b),
                _ => Some(c),
            };
            if let Some(old) = old {
                git(&server, &["update-ref", &target, &old.to_string()]);
            } else {
                git(&server, &["update-ref", "-d", &target]);
            }
            fs::write(root.join("file"), b"uncommitted preserved").unwrap();
            let before = [".git/HEAD", ".git/index", ".git/config", "file"]
                .map(|p| fs::read(root.join(p)).unwrap());
            let remote_before = git(&server, &["show-ref"]);
            let mut s = Session::new(root.clone()).unwrap();
            let snap = s.snapshot().unwrap();
            let selected = selection(&mut s, &snap.revision, "origin", destination).unwrap();
            assert_eq!(selected.source, "refs/heads/main");
            assert_eq!(selected.destination, target);
            assert_eq!(selected.endpoint, "https://example.com/push.git");
            let ad = Advertisement {
                endpoint: selected.endpoint.clone(),
                refs: old
                    .map(|id| RemoteRef {
                        name: target.clone(),
                        oid: id.to_string(),
                        symbolic_target: None,
                    })
                    .into_iter()
                    .collect(),
            };
            let plan = preview(&mut s, &snap.revision, &selected, &ad).unwrap();
            assert_eq!(
                plan.kind,
                match kind {
                    "new" => "createBranch",
                    "ff" => "fastForward",
                    "same" => "upToDate",
                    _ => "nonFastForward",
                }
            );
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "protocol.file.allow=always",
                    "push",
                    "--porcelain",
                    "--dry-run",
                    server.to_str().unwrap(),
                    &format!("refs/heads/main:{target}"),
                ])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                plan.blockers.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if kind == "ff" {
                assert_eq!(plan.outgoing, vec![b.to_string()]);
            }
            if kind == "same" {
                assert!(plan.outgoing.is_empty());
            }
            assert_eq!(
                before,
                [".git/HEAD", ".git/index", ".git/config", "file"]
                    .map(|p| fs::read(root.join(p)).unwrap())
            );
            assert_eq!(remote_before, git(&server, &["show-ref"]));
        }
    }
    #[test]
    fn preview_rejects_stale_ambiguous_endpoints_and_unsafe_targets() {
        let (d, _, _, _) = fixture();
        let root = d.path().join("local");
        let repo = Repository::open(&root).unwrap();
        let mut s = Session::new(root.clone()).unwrap();
        let snap = s.snapshot().unwrap();
        for branch in ["", "../bad", "x:y", "-force", "bad.lock"] {
            assert!(selection(&mut s, &snap.revision, "origin", branch).is_err());
        }
        let selected = selection(&mut s, &snap.revision, "origin", "main").unwrap();
        let mut ad = Advertisement {
            endpoint: selected.endpoint.clone(),
            refs: vec![RemoteRef {
                name: "refs/heads/main".into(),
                oid: "1".repeat(40),
                symbolic_target: None,
            }],
        };
        assert_eq!(
            preview(&mut s, &snap.revision, &selected, &ad)
                .unwrap()
                .kind,
            "unknownRemote"
        );
        ad.refs[0].name = "refs/heads/main/nested".into();
        assert!(preview(&mut s, &snap.revision, &selected, &ad)
            .unwrap()
            .blockers
            .contains(&"PUSH_REF_NAMESPACE_CONFLICT".into()));
        ad.refs.push(ad.refs[0].clone());
        assert!(preview(&mut s, &snap.revision, &selected, &ad).is_err());
        ad.refs.clear();
        ad.endpoint = "https://wrong.example/r".into();
        assert!(preview(&mut s, &snap.revision, &selected, &ad).is_err());
        repo.config()
            .unwrap()
            .set_str("remote.origin.pushurl", "https://other.example/r")
            .unwrap();
        assert!(selection(&mut s, &snap.revision, "origin", "main")
            .unwrap_err()
            .contains("STALE_STATE"));
        let mut config = fs::read(root.join(".git/config")).unwrap();
        config.extend_from_slice(b"\n[remote \"origin\"]\n pushurl=https://third.example/r\n");
        fs::write(root.join(".git/config"), config).unwrap();
        let snap = s.snapshot().unwrap();
        assert!(selection(&mut s, &snap.revision, "origin", "main")
            .unwrap_err()
            .contains("SINGLE_URL"));
    }
    #[test]
    fn push_endpoint_fallback_and_transport_guards_preserve_config() {
        for mode in [
            "fallback",
            "credential",
            "mirror",
            "rewrite",
            "receivepack",
            "detached",
        ] {
            let (d, _, head, _) = fixture();
            let root = d.path().join("local");
            let repo = Repository::open(&root).unwrap();
            let mut config = repo.config().unwrap();
            match mode {
                "fallback" => config.remove("remote.origin.pushurl").unwrap(),
                "credential" => config
                    .set_str("remote.origin.pushurl", "https://user:secret@example.com/r")
                    .unwrap(),
                "mirror" => config.set_bool("remote.origin.mirror", true).unwrap(),
                "rewrite" => config
                    .set_str(
                        "url.https://other.example/.pushinsteadof",
                        "https://example.com/",
                    )
                    .unwrap(),
                "receivepack" => config
                    .set_str("remote.origin.receivepack", "custom-program")
                    .unwrap(),
                _ => repo.set_head_detached(head).unwrap(),
            }
            let before = fs::read(root.join(".git/config")).unwrap();
            let mut session = Session::new(root.clone()).unwrap();
            let snap = session.snapshot().unwrap();
            let result = selection(&mut session, &snap.revision, "origin", "main");
            if mode == "fallback" {
                assert_eq!(result.unwrap().endpoint, "https://example.com/fetch.git");
            } else {
                assert!(result.is_err(), "{mode}");
            }
            assert_eq!(before, fs::read(root.join(".git/config")).unwrap());
        }
    }
}
