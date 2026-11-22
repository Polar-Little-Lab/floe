use super::{replace_repository, repository_path, Snapshot, Store};
use std::path::{Component, Path};

pub fn validate(path: &str, branch: &str) -> Result<std::path::PathBuf, String> {
    let path = repository_path(path)?;
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path.file_name().is_none()
        || path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".git"))
        || branch.is_empty()
        || branch.len() > 200
        || branch.starts_with('-')
        || branch == "HEAD"
        || !git2::Reference::is_valid_name(&format!("refs/heads/{branch}"))
    {
        return Err(
            "INVALID_INIT: 새 폴더의 절대 경로와 유효한 초기 브랜치 이름을 입력하세요.".into(),
        );
    }
    Ok(path)
}

pub fn create(store: &Store, path: &Path, branch: &str) -> Result<Snapshot, String> {
    if store.lock().map_err(|_| "LOCK_ERROR")?.len() >= super::MAX_SESSIONS {
        return Err("TAB_LIMIT: 최대 12개 저장소를 열 수 있습니다.".into());
    }
    let path = validate(path.to_str().ok_or("INVALID_PATH")?, branch)?;
    let parent = path
        .parent()
        .ok_or("INVALID_PATH")?
        .canonicalize()
        .map_err(|e| format!("INIT_PARENT: 상위 폴더를 먼저 만드세요. {e}"))?;
    let target = parent.join(path.file_name().ok_or("INVALID_PATH")?);
    // Exclusive creation refuses existing directories, files and dangling symlinks.
    std::fs::create_dir(&target)
        .map_err(|e| format!("INIT_CREATE: 기존 경로는 덮어쓰지 않습니다. {e}"))?;
    let result = (|| {
        let mut options = git2::RepositoryInitOptions::new();
        options
            .no_reinit(true)
            .mkdir(false)
            .mkpath(false)
            .external_template(false)
            .initial_head(branch);
        let repository =
            git2::Repository::init_opts(&target, &options).map_err(|e| format!("INIT_GIT: {e}"))?;
        drop(repository);
        replace_repository(store, target.clone())
    })();
    // Never recursively delete after failure: another process may have written here.
    result.map_err(|e| format!("{e}\n생성한 폴더를 보존했습니다: {}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn creates_empty_repository_with_explicit_initial_branch() {
        let root = tempfile::tempdir().unwrap();
        for branch in ["main", "feature/start"] {
            let store: Store = Arc::new(Mutex::new(Vec::new()));
            let path = root.path().join(branch.replace('/', "-"));
            let snapshot = create(&store, &path, branch).unwrap();
            assert_eq!(snapshot.branch, branch);
            assert!(snapshot.head_oid.is_none());
            assert!(snapshot.commits.is_empty());
            assert!(snapshot.changes.is_empty());
            let repo = git2::Repository::open(&path).unwrap();
            assert_eq!(
                repo.find_reference("HEAD")
                    .unwrap()
                    .symbolic_target()
                    .unwrap(),
                Some(format!("refs/heads/{branch}").as_str())
            );
            assert!(snapshot.write_block.is_none());
            for entry in std::fs::read_dir(path.join(".git/hooks")).unwrap() {
                assert!(entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".sample"));
            }
        }
    }
    #[test]
    fn rejects_existing_paths_and_keeps_previous_session() {
        let root = tempfile::tempdir().unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let old = create(&store, &root.path().join("old"), "main").unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, "keep").unwrap();
        for path in [
            root.path().join("old"),
            file.clone(),
            root.path().join("missing/child"),
        ] {
            assert!(create(&store, &path, "main").is_err());
            assert_eq!(store.lock().unwrap()[0].id(), old.repo_id);
        }
        assert_eq!(std::fs::read_to_string(file).unwrap(), "keep");
    }
    #[test]
    fn invalid_branch_never_creates_directory() {
        let root = tempfile::tempdir().unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let path = root.path().join("new");
        for branch in [
            "", "HEAD", "-option", "a..b", "a\0b", "a\nb", "a.lock", "a b",
        ] {
            assert!(create(&store, &path, branch).is_err(), "{branch:?}");
            assert!(!path.exists());
        }
        assert!(validate("relative", "main").is_err());
        assert!(validate(&root.path().join("../new").to_string_lossy(), "main").is_err());
    }
    #[test]
    fn newly_created_repository_supports_first_stage_and_commit() {
        let root = tempfile::tempdir().unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let path = root.path().join("project with spaces;$(literal)");
        create(&store, &path, "development").unwrap();
        std::fs::write(path.join("hello.txt"), "hello\n").unwrap();
        let mut guard = store.lock().unwrap();
        let session = &mut guard[0];
        let snapshot = session.snapshot().unwrap();
        let staged = session
            .mutate(&snapshot.revision, crate::repository::Action::StageAll)
            .unwrap();
        let committed = session
            .mutate(
                &staged.revision,
                crate::repository::Action::Commit {
                    message: "Initial commit".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                },
            )
            .unwrap();
        assert_eq!(committed.branch, "development");
        assert_eq!(committed.commits.len(), 1);
        assert!(committed.changes.is_empty());
        assert_eq!(committed.commits[0].refs, vec!["development"]);
        let repo = git2::Repository::open(path).unwrap();
        assert!(repo
            .references_glob("refs/branchdesk/checkpoints/*")
            .unwrap()
            .next()
            .is_some());
        let oid = repo.head().unwrap().target().unwrap();
        repo.reference("refs/branchdesk/checkpoints/test-extra", oid, false, "test")
            .unwrap();
        let changed = session.snapshot().unwrap();
        assert_ne!(committed.revision, changed.revision);
        assert_eq!(changed.commits[0].refs, vec!["development"]);
        // A user branch with similar text is still a normal branch.
        repo.branch(
            "branchdesk/checkpoints/user",
            &repo.find_commit(oid).unwrap(),
            false,
        )
        .unwrap();
        let changed = session.snapshot().unwrap();
        assert!(changed.commits[0]
            .refs
            .contains(&"branchdesk/checkpoints/user".to_string()));
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            0
        );
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_target_without_touching_destination() {
        let root = tempfile::tempdir().unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let target = root.path().join("target");
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(create(&store, &link, "main").is_err());
        assert!(!target.exists());
        assert!(std::fs::symlink_metadata(link)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
