//! Read-only configured ref mapping. Publication and automatic tag following are separate.
use super::*;
use crate::network_worker::Advertisement;
use std::collections::{BTreeMap, HashSet};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub source: String,
    pub destination: Option<String>,
    pub before: Option<String>,
    pub after: String,
    pub force: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub remote: String,
    pub mappings: Vec<Mapping>,
    pub excluded: Vec<String>,
}
struct Rule {
    source: String,
    destination: Option<String>,
    force: bool,
}
// Read only the frozen repository-local configuration, including unborn HEAD.
pub(super) fn upstream(
    values: &[(String, String)],
    head: &[u8],
    remote: &str,
) -> Result<(bool, Vec<String>)> {
    let head = std::str::from_utf8(head).map_err(|_| "INVALID_HEAD")?;
    let Some(branch) = head
        .strip_prefix("ref: refs/heads/")
        .and_then(|s| s.strip_suffix('\n'))
    else {
        return Ok((false, Vec::new()));
    };
    let remote_key = format!("branch.{branch}.remote");
    let merge_key = format!("branch.{branch}.merge");
    let configured_remote = values
        .iter()
        .rev()
        .find(|(key, _)| key == &remote_key)
        .map(|(_, v)| v.as_str());
    let sources = values
        .iter()
        .filter(|(key, _)| key == &merge_key)
        .map(|(_, v)| v.clone())
        .collect::<Vec<_>>();
    let has_merge = configured_remote.is_some() && !sources.is_empty();
    if configured_remote != Some(remote) {
        return Ok((has_merge, Vec::new()));
    }
    if sources.len() > 128
        || sources.iter().any(|source| {
            !source.starts_with("refs/")
                || source.contains('*')
                || !git2::Reference::is_valid_name(source)
        })
    {
        return Err("UNSUPPORTED_FETCH_UPSTREAM".into());
    }
    Ok((has_merge, sources))
}
pub(super) fn merge_candidates(
    values: &[(String, String)],
    head: &[u8],
    remote: &str,
) -> Result<Vec<String>> {
    let (has_merge, sources) = upstream(values, head, remote)?;
    if sources.is_empty()
        && !values
            .iter()
            .any(|(key, _)| key == &format!("remote.{remote}.fetch"))
    {
        return Ok(vec!["HEAD".into()]);
    }
    if has_merge {
        return Ok(sources);
    }
    let key = format!("remote.{remote}.fetch");
    let first = values
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, value)| value.as_str());
    if first.is_some_and(|value| value.starts_with('^') || value.contains('*')) {
        return Ok(Vec::new());
    }
    let (rules, _) = rules(values, remote)?;
    Ok(rules
        .first()
        .map(|r| vec![r.source.clone()])
        .unwrap_or_default())
}
fn pattern(value: &str) -> bool {
    value.matches('*').count() <= 1
        && (value == "HEAD"
            || (value.starts_with("refs/")
                && git2::Reference::is_valid_name(&value.replace('*', "probe"))))
}
fn capture<'a>(pattern: &str, value: &'a str) -> Option<&'a str> {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => value.strip_prefix(prefix)?.strip_suffix(suffix),
        None => (pattern == value).then_some(""),
    }
}
fn rules(values: &[(String, String)], remote: &str) -> Result<(Vec<Rule>, Vec<String>)> {
    let key = format!("remote.{remote}.fetch");
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    for (_, value) in values.iter().filter(|(name, _)| name == &key) {
        if positive.len() + negative.len() >= 128 {
            return Err("FETCH_REFSPEC_LIMIT".into());
        }
        if value.len() > 2048 {
            return Err("FETCH_REFSPEC_LIMIT".into());
        }
        if let Some(source) = value.strip_prefix('^') {
            if !pattern(source) {
                return Err("UNSUPPORTED_FETCH_REFSPEC".into());
            }
            negative.push(source.to_owned());
            continue;
        }
        let force = value.starts_with('+');
        let value = value.strip_prefix('+').unwrap_or(value);
        let (source, destination) = value
            .split_once(':')
            .map_or((value, None), |(s, d)| (s, (!d.is_empty()).then_some(d)));
        let source = if source.is_empty() || source == "@" {
            "HEAD"
        } else {
            source
        };
        if !pattern(source)
            || destination.is_some_and(|d| {
                !pattern(d) || !d.starts_with("refs/") || d.starts_with("refs/branchdesk/")
            })
            || source.matches('*').count() != destination.unwrap_or("").matches('*').count()
        {
            return Err("UNSUPPORTED_FETCH_REFSPEC".into());
        }
        positive.push(Rule {
            source: source.into(),
            destination: destination.map(str::to_owned),
            force,
        });
    }
    if positive.is_empty() && negative.is_empty() {
        positive.push(Rule {
            source: "HEAD".into(),
            destination: None,
            force: false,
        });
    }
    Ok((positive, negative))
}
pub(super) fn excludes(values: &[(String, String)], remote: &str, source: &str) -> Result<bool> {
    Ok(rules(values, remote)?
        .1
        .iter()
        .any(|p| capture(p, source).is_some()))
}
pub(super) fn auto_tags_enabled(values: &[(String, String)], remote: &str) -> Result<bool> {
    Ok(rules(values, remote)?
        .0
        .iter()
        .any(|r| r.destination.is_some()))
}
pub(super) fn permits(
    values: &[(String, String)],
    remote: &str,
    source: &str,
    destination: Option<&str>,
    head: Option<&[u8]>,
) -> Result<bool> {
    let (positive, negative) = rules(values, remote)?;
    if negative
        .iter()
        .any(|pattern| capture(pattern, source).is_some())
    {
        return Ok(false);
    }
    let upstream_only = if let Some(head) = head {
        destination.is_none()
            && upstream(values, head, remote)?
                .1
                .iter()
                .any(|s| s == source)
    } else {
        false
    };
    Ok(upstream_only
        || positive.iter().any(|rule| {
            capture(&rule.source, source).is_some_and(|matched| {
                rule.destination
                    .as_ref()
                    .map(|target| target.replacen('*', matched, 1))
                    .as_deref()
                    == destination
            })
        }))
}
pub(super) fn permits_force(
    values: &[(String, String)],
    remote: &str,
    source: &str,
    destination: Option<&str>,
) -> Result<bool> {
    Ok(rules(values, remote)?.0.iter().any(|rule| {
        rule.force
            && capture(&rule.source, source).is_some_and(|matched| {
                rule.destination
                    .as_ref()
                    .map(|d| d.replacen('*', matched, 1))
                    .as_deref()
                    == destination
            })
    }))
}
pub fn preview(
    session: &mut Session,
    revision: &str,
    remote: &str,
    advertisement: &Advertisement,
) -> Result<Plan> {
    let endpoint = remote_network::endpoint(session, revision, remote)?;
    if endpoint != advertisement.endpoint {
        return Err("REMOTE_ENDPOINT_CHANGED".into());
    }
    let config = bounded(&session.root.join(".git/config"), 65536)?;
    let values = remote_remove::entries(&config)?;
    let repo = open(&session.root)?;
    plan(&repo, &values, remote, advertisement)
}
pub(super) fn plan(
    repo: &Repository,
    values: &[(String, String)],
    remote: &str,
    advertisement: &Advertisement,
) -> Result<Plan> {
    if advertisement.refs.len() > 2000 {
        return Err("REMOTE_REF_LIMIT".into());
    }
    let (positive, negative) = rules(values, remote)?;
    let head = bounded(&repo.path().join("HEAD"), 4096)?;
    let (_, upstream) = upstream(values, &head, remote)?;
    let no_configured_rules = !values
        .iter()
        .any(|(key, _)| key == &format!("remote.{remote}.fetch"));
    let mut output: BTreeMap<(String, Option<String>), Mapping> = BTreeMap::new();
    let mut order = Vec::new();
    let mut destinations: BTreeMap<String, String> = BTreeMap::new();
    let mut excluded = Vec::new();
    let refs = advertisement
        .refs
        .iter()
        .filter(|r| !r.name.ends_with("^{}"))
        .collect::<Vec<_>>();
    for rule in &positive {
        if no_configured_rules && !upstream.is_empty() {
            continue;
        }
        let matches = refs
            .iter()
            .filter_map(|reference| {
                capture(&rule.source, &reference.name).map(|matched| (*reference, matched))
            })
            .collect::<Vec<_>>();
        if matches.is_empty() && !rule.source.contains('*') && !refs.is_empty() {
            return Err("FETCH_SOURCE_NOT_ADVERTISED".into());
        }
        for (reference, matched) in matches {
            if negative
                .iter()
                .any(|pattern| capture(pattern, &reference.name).is_some())
            {
                excluded.push(reference.name.clone());
                continue;
            }
            let oid = Oid::from_str(&reference.oid).map_err(|_| "INVALID_REMOTE_REF")?;
            if oid == Oid::ZERO_SHA1 {
                return Err("INVALID_REMOTE_REF".into());
            }
            let destination = rule
                .destination
                .as_ref()
                .map(|pattern| pattern.replacen('*', matched, 1));
            let before = if let Some(destination) = &destination {
                if !git2::Reference::is_valid_name(destination)
                    || destination.eq_ignore_ascii_case("refs/branchdesk")
                    || destination.to_lowercase().starts_with("refs/branchdesk/")
                {
                    return Err("INVALID_FETCH_DESTINATION".into());
                }
                if let Some(source) =
                    destinations.insert(destination.clone(), reference.name.clone())
                {
                    if source != reference.name {
                        return Err("FETCH_DESTINATION_COLLISION".into());
                    }
                }
                match repo.find_reference(destination) {
                    Ok(existing) => {
                        if existing
                            .symbolic_target()
                            .map_err(|_| "INVALID_LOCAL_REF")?
                            .is_some()
                        {
                            return Err("FETCH_SYMBOLIC_DESTINATION".into());
                        }
                        existing.target().map(|oid| oid.to_string())
                    }
                    Err(error) if error.code() == git2::ErrorCode::NotFound => None,
                    Err(_) => return Err("INVALID_FETCH_DESTINATION".into()),
                }
            } else {
                None
            };
            let key = (reference.name.clone(), destination.clone());
            if let Some(existing) = output.get_mut(&key) {
                existing.force |= rule.force;
            } else {
                order.push(key.clone());
                output.insert(
                    key,
                    Mapping {
                        source: reference.name.clone(),
                        destination,
                        before,
                        after: reference.oid.clone(),
                        force: rule.force,
                    },
                );
            }
            if output.len() > 2000 {
                return Err("FETCH_MAPPING_LIMIT".into());
            }
        }
    }
    // Upstream branches outside the configured mapping are fetched into FETCH_HEAD
    // only. A missing advertised upstream is tolerated, as in git fetch.
    for source in upstream {
        if output.values().any(|m| m.source == source) {
            continue;
        }
        if negative
            .iter()
            .any(|pattern| capture(pattern, &source).is_some())
        {
            excluded.push(source);
            continue;
        }
        if let Some(reference) = refs.iter().find(|reference| reference.name == source) {
            let oid = Oid::from_str(&reference.oid).map_err(|_| "INVALID_REMOTE_REF")?;
            if oid == Oid::ZERO_SHA1 {
                return Err("INVALID_REMOTE_REF".into());
            }
            order.push((source.clone(), None));
            output.insert(
                (source.clone(), None),
                Mapping {
                    source,
                    destination: None,
                    before: None,
                    after: reference.oid.clone(),
                    force: false,
                },
            );
            if output.len() > 2000 {
                return Err("FETCH_MAPPING_LIMIT".into());
            }
        }
    }
    validate_destinations(repo, &destinations.keys().cloned().collect::<Vec<_>>())?;
    excluded.sort();
    excluded.dedup();
    Ok(Plan {
        remote: remote.into(),
        mappings: order
            .into_iter()
            .filter_map(|key| output.remove(&key))
            .collect(),
        excluded,
    })
}

pub(super) fn validate_destinations(repo: &Repository, destinations: &[String]) -> Result<()> {
    let mut existing = HashSet::new();
    for reference in repo.references().map_err(err)? {
        let reference = reference.map_err(err)?;
        let name = reference.name().map_err(|_| "INVALID_LOCAL_REF")?;
        existing.insert(name.to_owned());
        if existing.len() > 10000 {
            return Err("FETCH_LOCAL_REF_LIMIT".into());
        }
    }
    let mut all: BTreeMap<String, HashSet<&str>> = BTreeMap::new();
    for name in existing.iter().chain(destinations.iter()) {
        all.entry(name.to_lowercase()).or_default().insert(name);
    }
    for destination in destinations.iter() {
        let folded = destination.to_lowercase();
        if all.get(&folded).is_some_and(|names| names.len() > 1) {
            return Err("FETCH_DESTINATION_COLLISION".into());
        }
        let prefix = format!("{folded}/");
        if all
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(name, _)| name.starts_with(&prefix))
        {
            return Err("FETCH_DESTINATION_COLLISION".into());
        }
        let mut ancestor = folded.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if all.contains_key(parent) {
                return Err("FETCH_DESTINATION_COLLISION".into());
            }
            ancestor = parent;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_worker::RemoteRef;
    use std::process::Command;
    fn fixture() -> (tempfile::TempDir, Advertisement) {
        let directory = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(directory.path()).unwrap();
        let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let main = repo
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "main",
                &tree,
                &[],
            )
            .unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let commit = repo.find_commit(main).unwrap();
        repo.commit(
            Some("refs/heads/topic"),
            &signature,
            &signature,
            "topic",
            &tree,
            &[&commit],
        )
        .unwrap();
        repo.reference("refs/heads/excluded", main, false, "fixture")
            .unwrap();
        let tag = repo
            .tag(
                "v1",
                &repo.find_object(main, None).unwrap(),
                &signature,
                "tag",
                false,
            )
            .unwrap();
        let mut refs = repo
            .references()
            .unwrap()
            .map(|reference| {
                let reference = reference.unwrap();
                RemoteRef {
                    name: reference.name().unwrap().into(),
                    oid: reference.target().unwrap().to_string(),
                    symbolic_target: None,
                }
            })
            .collect::<Vec<_>>();
        refs.push(RemoteRef {
            name: "HEAD".into(),
            oid: main.to_string(),
            symbolic_target: Some("refs/heads/main".into()),
        });
        refs.push(RemoteRef {
            name: "refs/tags/v1^{}".into(),
            oid: repo.find_tag(tag).unwrap().target_id().to_string(),
            symbolic_target: None,
        });
        (
            directory,
            Advertisement {
                endpoint: "https://example.com/repo.git".into(),
                refs,
            },
        )
    }
    fn configured(repo: &Repository, url: &str, specs: &[&str]) -> Vec<(String, String)> {
        repo.remote("origin", url).unwrap();
        let mut config = repo.config().unwrap();
        config.remove_multivar("remote.origin.fetch", ".*").unwrap();
        for spec in specs {
            config
                .set_multivar("remote.origin.fetch", "a^", spec)
                .unwrap();
        }
        specs
            .iter()
            .map(|spec| ("remote.origin.fetch".into(), spec.to_string()))
            .collect()
    }
    #[test]
    fn configured_mapping_matches_git_fetch_positive_negative_and_source_only() {
        let (source, advertisement) = fixture();
        for specs in [
            vec!["+refs/heads/*:refs/remotes/origin/*"],
            vec![
                "+refs/heads/*:refs/remotes/origin/*",
                "^refs/heads/excluded",
            ],
            vec!["refs/heads/to*:refs/remotes/origin/team-*"],
            vec![
                "refs/heads/main:refs/remotes/origin/main",
                "refs/heads/topic:refs/remotes/origin/other",
            ],
            vec!["refs/heads/main:"],
            vec!["HEAD:"],
            vec!["refs/tags/v1:refs/tags/v1"],
        ] {
            let local = tempfile::tempdir().unwrap();
            let repo = Repository::init(local.path()).unwrap();
            let values = configured(&repo, source.path().to_str().unwrap(), &specs);
            let config = fs::read(local.path().join(".git/config")).unwrap();
            let head = fs::read(local.path().join(".git/HEAD")).unwrap();
            let result = plan(&repo, &values, "origin", &advertisement).unwrap();
            assert_eq!(fs::read(local.path().join(".git/config")).unwrap(), config);
            assert_eq!(fs::read(local.path().join(".git/HEAD")).unwrap(), head);
            assert_eq!(repo.references().unwrap().count(), 0);
            let output = Command::new("/usr/bin/git")
                .arg("-C")
                .arg(local.path())
                .args([
                    "-c",
                    "protocol.file.allow=always",
                    "-c",
                    "remote.origin.followRemoteHEAD=never",
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--no-recurse-submodules",
                    "origin",
                ])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{specs:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual = Repository::open(local.path())
                .unwrap()
                .references()
                .unwrap()
                .map(|r| {
                    let r = r.unwrap();
                    (
                        r.name().unwrap().to_string(),
                        r.target().unwrap().to_string(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let expected = result
                .mappings
                .into_iter()
                .filter_map(|mapping| mapping.destination.map(|name| (name, mapping.after)))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(actual, expected, "{specs:?}");
        }
    }
    #[test]
    fn destination_collisions_symbolic_targets_and_limits_are_rejected() {
        let (_, advertisement) = fixture();
        let local = tempfile::tempdir().unwrap();
        let repo = Repository::init(local.path()).unwrap();
        for specs in [
            vec![
                "refs/heads/main:refs/remotes/origin/same",
                "refs/heads/topic:refs/remotes/origin/same",
            ],
            vec![
                "refs/heads/main:refs/remotes/origin/a",
                "refs/heads/topic:refs/remotes/origin/a/b",
            ],
            vec![
                "refs/heads/main:refs/remotes/origin/A",
                "refs/heads/topic:refs/remotes/origin/a",
            ],
        ] {
            let values = specs
                .iter()
                .map(|s| ("remote.origin.fetch".into(), s.to_string()))
                .collect::<Vec<_>>();
            assert_eq!(
                plan(&repo, &values, "origin", &advertisement).unwrap_err(),
                "FETCH_DESTINATION_COLLISION"
            );
        }
        repo.reference_symbolic(
            "refs/remotes/origin/main",
            "refs/heads/master",
            false,
            "fixture",
        )
        .unwrap();
        let values = vec![(
            "remote.origin.fetch".into(),
            "+refs/heads/*:refs/remotes/origin/*".into(),
        )];
        assert_eq!(
            plan(&repo, &values, "origin", &advertisement).unwrap_err(),
            "FETCH_SYMBOLIC_DESTINATION"
        );
        assert!(rules(
            &vec![("remote.origin.fetch".into(), "HEAD:".into()); 129],
            "origin"
        )
        .is_err());
        let (_, mut hostile) = fixture();
        hostile.refs.push(RemoteRef {
            name: "refs/BRANCHDESK/private".into(),
            oid: hostile.refs[0].oid.clone(),
            symbolic_target: None,
        });
        let hostile_rules = vec![("remote.origin.fetch".into(), "refs/*:refs/*".into())];
        assert_eq!(
            plan(&repo, &hostile_rules, "origin", &hostile).unwrap_err(),
            "INVALID_FETCH_DESTINATION"
        );
        for spec in [
            "refs/heads/*:refs/remotes/origin/main",
            "^refs/heads/main:refs/remotes/origin/main",
            "refs/heads/main:HEAD",
            "refs/heads/main:refs/branchdesk/private",
        ] {
            assert!(rules(&[("remote.origin.fetch".into(), spec.into())], "origin").is_err());
        }
    }
    #[test]
    fn preview_records_old_values_force_and_exclusions_without_mutation() {
        let (_, advertisement) = fixture();
        let local = tempfile::tempdir().unwrap();
        let repo = Repository::init(local.path()).unwrap();
        let signature = git2::Signature::now("Local", "local@example.invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let old = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "old local",
                &tree,
                &[],
            )
            .unwrap();
        repo.reference("refs/remotes/origin/main", old, false, "fixture")
            .unwrap();
        let values = configured(
            &repo,
            &advertisement.endpoint,
            &[
                "refs/heads/*:refs/remotes/origin/*",
                "+refs/heads/main:refs/remotes/origin/main",
                "^refs/heads/excluded",
            ],
        );
        repo.index().unwrap().write().unwrap();
        fs::write(local.path().join("dirty.txt"), "keep this").unwrap();
        let index = fs::read(local.path().join(".git/index")).unwrap();
        let head = fs::read(local.path().join(".git/HEAD")).unwrap();
        let result = plan(&repo, &values, "origin", &advertisement).unwrap();
        assert_eq!(result.mappings.len(), 2);
        assert_eq!(result.excluded, vec!["refs/heads/excluded"]);
        let main = result
            .mappings
            .iter()
            .find(|m| m.source == "refs/heads/main")
            .unwrap();
        assert_eq!(main.before, Some(old.to_string()));
        assert!(main.force);
        assert_eq!(
            repo.find_reference("refs/remotes/origin/main")
                .unwrap()
                .target(),
            Some(old)
        );
        assert_eq!(fs::read(local.path().join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(local.path().join(".git/HEAD")).unwrap(), head);
        assert_eq!(
            fs::read_to_string(local.path().join("dirty.txt")).unwrap(),
            "keep this"
        );
    }
    #[test]
    fn scoped_preview_rejects_stale_config_and_changed_endpoint() {
        let (_, advertisement) = fixture();
        let local = tempfile::tempdir().unwrap();
        let repo = Repository::init(local.path()).unwrap();
        configured(
            &repo,
            &advertisement.endpoint,
            &["+refs/heads/*:refs/remotes/origin/*"],
        );
        repo.index().unwrap().write().unwrap();
        let mut session = Session::new(local.path().canonicalize().unwrap()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert_eq!(
            preview(&mut session, &snapshot.revision, "origin", &advertisement)
                .unwrap()
                .mappings
                .len(),
            3
        );
        repo.config()
            .unwrap()
            .set_str("remote.origin.url", "https://other.example/repo.git")
            .unwrap();
        assert!(
            preview(&mut session, &snapshot.revision, "origin", &advertisement)
                .unwrap_err()
                .contains("STALE_STATE")
        );
        let snapshot = session.snapshot().unwrap();
        assert_eq!(
            preview(&mut session, &snapshot.revision, "origin", &advertisement).unwrap_err(),
            "REMOTE_ENDPOINT_CHANGED"
        );
    }
}
