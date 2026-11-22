//! Full, non-thin fetch into disposable storage. Never publishes refs or user files.
use super::*;
use std::{collections::HashSet, path::Path};
const PACK_LIMIT: u64 = 128 * 1024 * 1024;
const OBJECT_LIMIT: usize = 100_000;
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Download {
    pub advertisement: Advertisement,
    pub pack_hash: Option<String>,
    pub received_bytes: u64,
    pub objects: usize,
}
pub(super) fn download(
    url: &str,
    client: &reqwest::blocking::Client,
    directory: &Path,
) -> Result<Download, String> {
    let advertisement = inspect_with_client(url, client)?;
    let mut wanted = std::collections::BTreeSet::new();
    for reference in &advertisement.refs {
        if !reference.name.ends_with("^{}") {
            wanted.insert(reference.oid.clone());
        }
    }
    if wanted.is_empty() {
        return Ok(Download {
            advertisement,
            pack_hash: None,
            received_bytes: 0,
            objects: 0,
        });
    }
    let mut request = String::new();
    // No haves, thin-pack, side-band, or deltas requiring local objects.
    for oid in wanted {
        let line = format!("want {oid}\n");
        request.push_str(&format!("{:04x}{line}", line.len() + 4));
    }
    request.push_str("00000009done\n");
    let target = format!(
        "{}/git-upload-pack",
        advertisement.endpoint.trim_end_matches('/')
    );
    let response = client
        .post(target)
        .header("Content-Type", "application/x-git-upload-pack-request")
        .header("Accept", "application/x-git-upload-pack-result")
        .body(request)
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                "REMOTE_TIMEOUT"
            } else {
                "REMOTE_CONNECTION_FAILED"
            }
        })?;
    if response.status().is_redirection() {
        return Err("REMOTE_REDIRECT_REJECTED".into());
    }
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err("REMOTE_AUTH_REQUIRED".into());
    }
    if !response.status().is_success() {
        return Err("REMOTE_CONNECTION_FAILED".into());
    }
    if response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim())
        != Some("application/x-git-upload-pack-result")
    {
        return Err("REMOTE_PACK_INVALID".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size > PACK_LIMIT + 8)
    {
        return Err("REMOTE_PACK_LIMIT".into());
    }
    let (hash, received_bytes, objects) = index_response(response, directory, &advertisement)?;
    Ok(Download {
        advertisement,
        pack_hash: Some(hash),
        received_bytes,
        objects,
    })
}
pub(super) fn index_response(
    mut response: impl Read,
    directory: &Path,
    advertisement: &Advertisement,
) -> Result<(String, u64, usize), String> {
    let mut nak = [0; 8];
    response
        .read_exact(&mut nak)
        .map_err(|_| "REMOTE_PACK_INVALID")?;
    if &nak != b"0008NAK\n" {
        return Err("REMOTE_PACK_INVALID".into());
    }
    let mut header = [0; 12];
    response
        .read_exact(&mut header)
        .map_err(|_| "REMOTE_PACK_INVALID")?;
    if &header[..4] != b"PACK"
        || !matches!(u32::from_be_bytes(header[4..8].try_into().unwrap()), 2 | 3)
    {
        return Err("REMOTE_PACK_INVALID".into());
    }
    let count = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    if count > OBJECT_LIMIT {
        return Err("REMOTE_OBJECT_LIMIT".into());
    }
    // Must be an exclusively created directory under the worker's disposable cwd.
    let staging = tempfile::tempdir_in(directory.parent().ok_or("REMOTE_QUARANTINE_FAILED")?)
        .map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true).external_template(false);
    let repository = git2::Repository::init_opts(staging.path(), &options)
        .map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
    repository
        .set_config(&git2::Config::new().map_err(|_| "REMOTE_QUARANTINE_FAILED")?)
        .map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
    let mut index = git2::Indexer::new(None, &staging.path().join("objects/pack"), 0o600, true)
        .map_err(|_| "REMOTE_PACK_INVALID")?;
    index.progress(|progress| {
        progress.total_objects() <= OBJECT_LIMIT && progress.received_bytes() <= PACK_LIMIT as usize
    });
    index
        .write_all(&header)
        .map_err(|_| "REMOTE_PACK_INVALID")?;
    let mut received = 12u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|_| "REMOTE_CONNECTION_FAILED")?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > PACK_LIMIT {
            return Err("REMOTE_PACK_LIMIT".into());
        }
        index
            .write_all(&buffer[..n])
            .map_err(|_| "REMOTE_PACK_INVALID")?;
    }
    let hash = index.commit().map_err(|_| "REMOTE_PACK_INVALID")?;
    verify_graph(&repository, advertisement)?;
    std::fs::create_dir(directory).map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
    git2::Repository::init_opts(directory, &options).map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
    super::object_import::publish(staging.path(), directory, &hash)
        .map_err(|_| "REMOTE_OBJECT_IMPORT_FAILED")?;
    Ok((hash, received, count))
}
fn verify_graph(repo: &git2::Repository, advertisement: &Advertisement) -> Result<(), String> {
    let odb = repo.odb().map_err(|_| "REMOTE_PACK_INVALID")?;
    let mut pending = Vec::new();
    for reference in &advertisement.refs {
        let oid = git2::Oid::from_str(&reference.oid).map_err(|_| "INVALID_REMOTE_REF")?;
        let (_, kind) = odb
            .read_header(oid)
            .map_err(|_| "REMOTE_PACK_MISSING_OBJECT")?;
        if reference.name.starts_with("refs/heads/") && kind != git2::ObjectType::Commit {
            return Err("REMOTE_BRANCH_NOT_COMMIT".into());
        }
        pending.push(oid);
    }
    let mut seen = HashSet::new();
    while let Some(oid) = pending.pop() {
        if !seen.insert(oid) {
            continue;
        }
        if seen.len() > OBJECT_LIMIT {
            return Err("REMOTE_OBJECT_LIMIT".into());
        }
        let (size, kind) = odb
            .read_header(oid)
            .map_err(|_| "REMOTE_PACK_MISSING_OBJECT")?;
        if size > 16 * 1024 * 1024 {
            return Err("REMOTE_OBJECT_SIZE_LIMIT".into());
        }
        match kind {
            git2::ObjectType::Commit => {
                let commit = repo.find_commit(oid).map_err(|_| "REMOTE_PACK_INVALID")?;
                pending.push(commit.tree_id());
                pending.extend(commit.parent_ids());
            }
            git2::ObjectType::Tree => {
                let tree = repo.find_tree(oid).map_err(|_| "REMOTE_PACK_INVALID")?;
                for entry in tree.iter() {
                    if entry.filemode() != 0o160000 {
                        pending.push(entry.id());
                    }
                }
            }
            git2::ObjectType::Tag => pending.push(
                repo.find_tag(oid)
                    .map_err(|_| "REMOTE_PACK_INVALID")?
                    .target_id(),
            ),
            git2::ObjectType::Blob => {}
            _ => return Err("REMOTE_PACK_INVALID".into()),
        }
        if pending.len() > OBJECT_LIMIT * 2 {
            return Err("REMOTE_OBJECT_LIMIT".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn advertisement(oid: git2::Oid) -> Advertisement {
        Advertisement {
            endpoint: "https://example.com/r".into(),
            refs: vec![RemoteRef {
                name: "refs/heads/main".into(),
                oid: oid.to_string(),
                symbolic_target: None,
            }],
        }
    }
    #[test]
    fn rejects_pack_header_limits_before_creating_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let quarantine = dir.path().join("quarantine.git");
        for (version, count, expected) in [
            (2, 100001, "REMOTE_OBJECT_LIMIT"),
            (9, 1, "REMOTE_PACK_INVALID"),
        ] {
            let mut bytes = b"0008NAK\nPACK".to_vec();
            bytes.extend_from_slice(&(version as u32).to_be_bytes());
            bytes.extend_from_slice(&(count as u32).to_be_bytes());
            assert_eq!(
                index_response(
                    bytes.as_slice(),
                    &quarantine,
                    &advertisement(git2::Oid::ZERO_SHA1)
                )
                .unwrap_err(),
                expected
            );
            assert!(!quarantine.exists());
        }
    }
    #[test]
    fn connectivity_rejects_missing_parent_and_noncommit_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(dir.path()).unwrap();
        let blob = repo.blob(b"blob").unwrap();
        assert_eq!(
            verify_graph(&repo, &advertisement(blob)).unwrap_err(),
            "REMOTE_BRANCH_NOT_COMMIT"
        );
        let tree = repo.treebuilder(None).unwrap().write().unwrap();
        let commit=format!("tree {tree}\nparent {}\nauthor Fixture <fixture@example.invalid> 1 +0000\ncommitter Fixture <fixture@example.invalid> 1 +0000\n\nmissing parent\n","1".repeat(40));
        let oid = repo
            .odb()
            .unwrap()
            .write(git2::ObjectType::Commit, commit.as_bytes())
            .unwrap();
        assert_eq!(
            verify_graph(&repo, &advertisement(oid)).unwrap_err(),
            "REMOTE_PACK_MISSING_OBJECT"
        );
    }
}
