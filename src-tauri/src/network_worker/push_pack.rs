//! Full non-thin Push pack: only the selected commit's complete object closure.
use super::*;
use git2::{Oid, Repository};
pub(crate) struct PreparedPack {
    private: tempfile::TempDir,
    metadata: pack::Download,
}
impl PreparedPack {
    pub(super) fn stage(&self, destination: &std::path::Path) -> Result<(), String> {
        self.summary()?;
        std::fs::create_dir(destination).map_err(failure)?;
        let mut options = git2::RepositoryInitOptions::new();
        options.bare(true).external_template(false);
        Repository::init_opts(destination.join("quarantine.git"), &options).map_err(failure)?;
        let hash = self
            .metadata
            .pack_hash
            .as_ref()
            .ok_or("PUSH_PACK_INVALID")?;
        for extension in ["pack", "idx"] {
            let relative = format!("quarantine.git/objects/pack/pack-{hash}.{extension}");
            let source = self.private.path().join(&relative);
            let target = destination.join(&relative);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(target).map_err(failure)?;
            let mut input = std::fs::File::open(source).map_err(failure)?;
            std::io::copy(&mut input, &mut output).map_err(failure)?;
        }
        artifact::verify(
            destination,
            &self.metadata.advertisement.endpoint,
            &self.metadata,
        )?;
        let data = serde_json::to_vec(&self.metadata).map_err(failure)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(destination.join("metadata.json"))
            .map_err(failure)?
            .write_all(&data)
            .map_err(failure)?;
        Ok(())
    }
    pub(super) fn load_staged(root: &std::path::Path) -> Result<Self, String> {
        let meta = std::fs::symlink_metadata(root.join("metadata.json")).map_err(failure)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 16384 {
            return Err("PUSH_PACK_INVALID".into());
        }
        let metadata: pack::Download =
            serde_json::from_slice(&std::fs::read(root.join("metadata.json")).map_err(failure)?)
                .map_err(failure)?;
        if metadata.advertisement.refs.len() != 1
            || metadata.advertisement.refs[0].name != "refs/heads/selected"
            || metadata.advertisement.endpoint != "https://push-artifact.invalid/selected"
        {
            return Err("PUSH_PACK_INVALID".into());
        }
        artifact::verify(root, &metadata.advertisement.endpoint, &metadata)?;
        // Rebuild clean config in owned storage; never open staged config or hooks.
        let private = tempfile::tempdir().map_err(failure)?;
        let destination = private.path().join("quarantine.git");
        let mut options = git2::RepositoryInitOptions::new();
        options.bare(true).external_template(false);
        let repo = Repository::init_opts(&destination, &options).map_err(failure)?;
        repo.set_config(&git2::Config::new().map_err(failure)?)
            .map_err(failure)?;
        let hash = metadata.pack_hash.as_ref().ok_or("PUSH_PACK_INVALID")?;
        for extension in ["pack", "idx"] {
            let relative = format!("objects/pack/pack-{hash}.{extension}");
            std::fs::copy(
                root.join("quarantine.git").join(&relative),
                destination.join(relative),
            )
            .map_err(failure)?;
        }
        artifact::verify(private.path(), &metadata.advertisement.endpoint, &metadata)?;
        let source = Oid::from_str(&metadata.advertisement.refs[0].oid).map_err(failure)?;
        repo.find_commit(source).map_err(failure)?;
        if selection::inventory(&repo)? != selection::closure(&repo, &[source])? {
            return Err("PUSH_PACK_OBJECT_MISMATCH".into());
        }
        Ok(Self { private, metadata })
    }
    pub fn summary(&self) -> Result<(u64, usize), String> {
        artifact::verify(
            self.private.path(),
            &self.metadata.advertisement.endpoint,
            &self.metadata,
        )?;
        Ok((self.metadata.received_bytes, self.metadata.objects))
    }
    pub(super) fn validate_update(&self, old: &str, new: &str) -> Result<(), String> {
        self.summary()?;
        if self.metadata.advertisement.refs[0].oid != new {
            return Err("PUSH_SOURCE_MISMATCH".into());
        }
        let old = Oid::from_str(old).map_err(|_| "PUSH_COMMAND_INVALID")?;
        let new = Oid::from_str(new).map_err(|_| "PUSH_COMMAND_INVALID")?;
        if !old.is_zero() {
            let repo = Repository::open_bare(self.private.path().join("quarantine.git"))
                .map_err(failure)?;
            if old == new || !repo.graph_descendant_of(new, old).unwrap_or(false) {
                return Err("PUSH_NON_FAST_FORWARD".into());
            }
        }
        Ok(())
    }
    pub(super) fn bytes(&self) -> Result<Vec<u8>, String> {
        self.summary()?;
        let hash = self
            .metadata
            .pack_hash
            .as_ref()
            .ok_or("PUSH_PACK_INVALID")?;
        std::fs::read(
            self.private
                .path()
                .join(format!("quarantine.git/objects/pack/pack-{hash}.pack")),
        )
        .map_err(|_| "PUSH_PACK_INVALID".into())
    }
}
fn failure(_: impl std::fmt::Display) -> String {
    "PUSH_PACK_PREPARATION_FAILED".into()
}
pub(crate) fn prepare(repo: &Repository, source: Oid) -> Result<PreparedPack, String> {
    repo.find_commit(source).map_err(failure)?;
    let objects = selection::closure(repo, &[source])?;
    let private = tempfile::tempdir().map_err(failure)?;
    let destination = private.path().join("quarantine.git");
    let mut options = git2::RepositoryInitOptions::new();
    options.bare(true).external_template(false);
    let output = Repository::init_opts(&destination, &options).map_err(failure)?;
    output
        .set_config(&git2::Config::new().map_err(failure)?)
        .map_err(failure)?;
    let mut builder = repo.packbuilder().map_err(failure)?;
    builder.set_threads(1);
    for oid in &objects {
        builder.insert_object(*oid, None).map_err(failure)?;
    }
    builder
        .write(&destination.join("objects/pack"), 0o600)
        .map_err(failure)?;
    let hash = builder
        .name()
        .map_err(failure)?
        .ok_or("PUSH_PACK_PREPARATION_FAILED")?
        .to_owned();
    let bytes = std::fs::metadata(destination.join(format!("objects/pack/pack-{hash}.pack")))
        .map_err(failure)?
        .len();
    let metadata = pack::Download {
        advertisement: Advertisement {
            endpoint: "https://push-artifact.invalid/selected".into(),
            refs: vec![RemoteRef {
                name: "refs/heads/selected".into(),
                oid: source.to_string(),
                symbolic_target: None,
            }],
        },
        pack_hash: Some(hash),
        received_bytes: bytes,
        objects: objects.len(),
    };
    artifact::verify(private.path(), &metadata.advertisement.endpoint, &metadata)?;
    let verified = Repository::open_bare(&destination).map_err(failure)?;
    verified
        .set_config(&git2::Config::new().map_err(failure)?)
        .map_err(failure)?;
    if selection::inventory(&verified)? != objects
        || selection::closure(&verified, &[source])? != objects
    {
        return Err("PUSH_PACK_OBJECT_MISMATCH".into());
    }
    Ok(PreparedPack { private, metadata })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    #[test]
    fn prepared_pack_contains_exact_history_and_git_accepts_it_without_source_changes() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(d.path().join("source")).unwrap();
        let sig = git2::Signature::now("Push fixture", "push@invalid").unwrap();
        let blob = repo.blob(b"selected\0bytes").unwrap();
        let extra = repo.blob(b"unrelated secret fixture").unwrap();
        let mut tree = repo.treebuilder(None).unwrap();
        tree.insert("file", blob, 0o100644).unwrap();
        let tree = repo.find_tree(tree.write().unwrap()).unwrap();
        let base = repo
            .commit(Some("refs/heads/main"), &sig, &sig, "base", &tree, &[])
            .unwrap();
        let tip = repo
            .commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "tip",
                &tree,
                &[&repo.find_commit(base).unwrap()],
            )
            .unwrap();
        let before = selection::inventory(&repo).unwrap();
        let config = std::fs::read(repo.path().join("config")).unwrap();
        let prepared = prepare(&repo, tip).unwrap();
        assert!(prepared
            .validate_update(&base.to_string(), &tip.to_string())
            .is_ok());
        assert!(prepared
            .validate_update(&"0".repeat(40), &tip.to_string())
            .is_ok());
        assert!(prepared
            .validate_update(&tip.to_string(), &tip.to_string())
            .is_err());
        assert!(prepared
            .validate_update(&extra.to_string(), &tip.to_string())
            .is_err());
        assert!(prepared
            .validate_update(&"0".repeat(40), &base.to_string())
            .is_err());
        let stage = tempfile::tempdir().unwrap();
        let outbound = stage.path().join("outbound");
        prepared.stage(&outbound).unwrap();
        assert!(prepared.stage(&outbound).is_err());
        std::fs::write(
            outbound.join("quarantine.git/config"),
            "[include]\npath=/must/not/be/read\n",
        )
        .unwrap();
        let loaded = PreparedPack::load_staged(&outbound).unwrap();
        assert_eq!(loaded.bytes().unwrap(), prepared.bytes().unwrap());
        assert!(loaded
            .validate_update(&base.to_string(), &tip.to_string())
            .is_ok());
        let hash = prepared.metadata.pack_hash.as_ref().unwrap();
        std::fs::write(
            outbound.join(format!("quarantine.git/objects/pack/pack-{hash}.pack")),
            b"corrupt",
        )
        .unwrap();
        assert!(PreparedPack::load_staged(&outbound).is_err());
        let again = prepare(&repo, tip).unwrap();
        assert_eq!(prepared.bytes().unwrap(), again.bytes().unwrap());
        assert_eq!(prepared.summary().unwrap().1, 4);
        let projected =
            Repository::open_bare(prepared.private.path().join("quarantine.git")).unwrap();
        assert!(projected.find_blob(extra).is_err());
        assert!(projected.find_commit(base).is_ok());
        assert_eq!(before, selection::inventory(&repo).unwrap());
        assert_eq!(config, std::fs::read(repo.path().join("config")).unwrap());
        assert_eq!(repo.refname_to_id("refs/heads/main").unwrap(), tip);
        let server = d.path().join("server");
        Repository::init_bare(&server).unwrap();
        let body = super::super::push_wire::request(
            &"0".repeat(40),
            &tip.to_string(),
            "refs/heads/main",
            &["report-status".into()],
            &prepared.bytes().unwrap(),
        )
        .unwrap();
        let mut child = Command::new("/usr/bin/git")
            .args(["receive-pack", "--stateless-rpc"])
            .arg(&server)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&body).unwrap();
        let result = child.wait_with_output().unwrap();
        assert_eq!(
            super::super::push_wire::report(&result.stdout, "refs/heads/main").unwrap(),
            super::super::push_wire::Report::Accepted
        );
        let received = Repository::open_bare(&server).unwrap();
        assert_eq!(received.refname_to_id("refs/heads/main").unwrap(), tip);
        assert!(received.find_blob(extra).is_err());
        assert_eq!(
            received.find_blob(blob).unwrap().content(),
            b"selected\0bytes"
        );
        let path = prepared.private.path().to_path_buf();
        drop(prepared);
        assert!(!path.exists());
    }
    #[test]
    fn rejects_noncommit_and_detects_pack_corruption_before_use() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repository::init_bare(d.path()).unwrap();
        let blob = repo.blob(b"x").unwrap();
        assert!(prepare(&repo, blob).is_err());
        let sig = git2::Signature::now("test", "test@invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let tip = repo
            .commit(None, &sig, &sig, "empty tree", &tree, &[])
            .unwrap();
        let prepared = prepare(&repo, tip).unwrap();
        let path = prepared.private.path().join(format!(
            "quarantine.git/objects/pack/pack-{}.pack",
            prepared.metadata.pack_hash.as_ref().unwrap()
        ));
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(prepared.summary().is_err());
        assert!(prepared.bytes().is_err());
    }
}
