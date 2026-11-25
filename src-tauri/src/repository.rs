pub mod lfs;
pub mod gitflow;
mod patch_apply;
pub mod patch_check;
pub mod checkout_status;
pub mod tree_browser;
pub mod tag_detail;
pub mod submodules;
mod config_policy;
pub mod rebase;
mod remotes;
mod replay;
mod replay_sequence;
mod sequence_state;
pub use replay_sequence::{Plan as ReplaySequencePlan, Selection as ReplaySelection};
mod replay_draft;
pub use replay::{Kind as ReplayKind, Plan as ReplayPlan};
mod blame;
mod branch_rename;
pub mod fetch;
pub mod fetch_plan;
mod fetch_tags;
#[cfg(test)]
mod git_parity;
pub mod pull;
pub mod push;
pub mod push_state;
pub(crate) mod remote_endpoint;
pub mod remote_head;
pub mod remote_network;
mod remote_remove;
mod remote_rename;
mod remote_write;
pub use blame::FileBlame;
pub use remote_remove::Preview as RemoteRemovePreview;
pub use remote_write::{TagPolicy, UpstreamTarget};
mod content;
pub use content::{ContentSource, FileContent};
mod commit_write;
mod external_merge;
mod merge;
mod merge_abort;
mod merge_draft;
mod merge_finish;
pub use merge::MergePlan;
mod checkout;
pub(crate) use checkout::inspect_tree as inspect_clone_tree;
pub mod compare;
#[cfg(test)]
mod detached_tests;
#[cfg(test)]
mod graph_page_tests;
mod history;
mod hunks;
mod line_staging;
pub use history::FileHistory;
mod journal_locks;
mod lease;
mod stash_apply;
mod stash_save;
mod soft_reset;
mod ignore;
pub mod search;
pub mod checkpoints;
pub mod worktrees;
mod layout;
mod stash_write;
use git2::{
    Config, DiffFormat, DiffOptions, IndexEntry, IndexTime, Oid, Repository, RepositoryOpenFlags,
    Sort, StatusOptions,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const FILE_LIMIT: u64 = 5 * 1024 * 1024;
const MAX_FILES: usize = 2000;
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    format!("GIT_ERROR: {e}")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub author: String,
    pub time: i64,
    pub refs: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub path: String,
    pub staged: bool,
    pub unstaged: bool,
    pub status: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub oid: String,
    pub current: bool,
    pub upstream: Option<remote_write::UpstreamConfig>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub name: String,
    pub oid: Option<String>,
    pub annotated: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct Stash {
    pub oid: String,
    pub index: usize,
    pub message: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StashArea {
    Staged,
    Unstaged,
    Untracked,
}
#[derive(Debug, Serialize)]
pub struct StashGroup {
    pub area: StashArea,
    pub files: Vec<Change>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashDetail {
    pub oid: String,
    pub message: String,
    pub base_oid: String,
    pub groups: Vec<StashGroup>,
    pub diff: Option<DiffResult>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub repository_identity: remote_write::RepositoryIdentity,
    pub repo_id: String,
    pub name: String,
    pub branch: String,
    pub detached: bool,
    pub revision: String,
    pub commits: Vec<Commit>,
    pub branches: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub stashes: Vec<Stash>,
    pub remotes: Vec<remotes::Remote>,
    pub changes: Vec<Change>,
    pub has_more: bool,
    pub graph_offset: usize,
    pub graph_page_size: usize,
    pub graph_window_size: usize,
    pub write_block: Option<String>,
    pub recovery_pending: bool,
    pub linked_local_writes: bool,
    pub head_oid: Option<String>,
    pub rebase_draft: Option<rebase::draft::Summary>,
    pub rebase_draft_error: Option<String>,
    pub push_record: Option<push_state::Record>,
    pub push_record_error: Option<String>,
    pub merge_draft: Option<merge_draft::Draft>,
    pub merge_draft_error: Option<String>,
    pub replay_draft: Option<replay_draft::Draft>,
    pub replay_draft_error: Option<String>,
    pub sequence_progress: Option<sequence_state::Progress>,
    pub sequence_error: Option<String>,
    pub replay_completion_ready: bool,
    pub external_merge: Option<external_merge::ExternalMerge>,
}
#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub text: String,
    pub truncated: bool,
    pub hunks: Vec<hunks::Hunk>,
    #[serde(rename="selectableLines")]
    pub selectable_lines: Vec<line_staging::SelectableLine>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub oid: String,
    pub message: String,
    pub parent_oid: Option<String>,
    pub files: Vec<Change>,
    pub diff: Option<DiffResult>,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    SetGitflowConfiguration { configuration: gitflow::Configuration },
    SetRepositoryIdentity {
        identity: Option<crate::identity_settings::Identity>,
    },
    SetUpstream {
        name: String,
        target: Option<UpstreamTarget>,
    },
    RenameRemote {
        name: String,
        new_name: String,
    },
    RemoveRemote {
        name: String,
    },
    SetRemoteTagPolicy {
        name: String,
        policy: TagPolicy,
    },
    SetRemote {
        name: String,
        url: String,
        push_url: Option<String>,
        create: bool,
    },
    StartSequence {
        selections: Vec<ReplaySelection>,
    },
    ContinueSequence {
        message: String,
        author: String,
        email: String,
        #[serde(default)]
        allow_empty: bool,
    },
    ResolveSequence {
        sequence_id: String,
        ordinal: usize,
        file_id: String,
        resolution: merge_draft::Resolution,
    },
    SkipSequence {
        sequence_id: String,
        ordinal: usize,
    },
    AbortSequence,
    StopSequence,
    StartReplay {
        oid: String,
        operation: ReplayKind,
        mainline: Option<usize>,
    },
    ResolveReplay {
        file_id: String,
        resolution: merge_draft::Resolution,
    },
    AbortReplay,
    SkipReplay,
    ContinueReplay {
        #[serde(default)]
        allow_empty: bool,
        message: String,
        author: String,
        email: String,
    },
    ReplayCommit {
        #[serde(default)]
        allow_empty: bool,
        oid: String,
        operation: ReplayKind,
        mainline: Option<usize>,
        message: String,
        author: String,
        email: String,
    },
    RenameBranch {
        name: String,
        new_name: String,
    },
    ApplyPatchIndex {text:String},
    StageLines { file_id: String, lines: Vec<line_staging::Selection>, unstage: bool },
    StageHunk {
        file_id: String,
        hunk: usize,
        unstage: bool,
    },
    AbortExternalMerge,
    ContinueExternalMerge {
        message: String,
        author: String,
        email: String,
    },
    StageExternalResolution {
        file_id: String,
    },
    StartMerge {
        name: String,
    },
    ResolveMerge {
        file_id: String,
        resolution: merge_draft::Resolution,
    },
    AbortMerge,
    ContinueMerge {
        message: String,
        author: String,
        email: String,
    },
    MergeCommit {
        name: String,
        message: String,
        author: String,
        email: String,
    },
    StartRebase {
        upstream: String,
        author: String,
        email: String,
    },
    ResolveRebase {
        draft_id: String,
        source: String,
        file_id: String,
        resolution: merge_draft::Resolution,
    },
    SkipRebase {
        draft_id: String,
        source: String,
    },
    ContinueRebase,
    AbortRebase,
    Rebase {
        upstream: String,
        author: String,
        email: String,
    },
    FastForwardMerge {
        name: String,
    },
    Checkout {
        name: String,
    },
    CheckoutCommit {
        oid: String,
    },
    RecoverCheckout,
    SoftReset { oid: String },
    IgnoreFile { file_id: String },
    CreateRecoveryBranch { reference: String, oid: String, name: String },
    CreateWorktree { name: String, path: String, branch: Option<String> },
    SetWorktreeLock { name: String, expected_locked: bool, locked: bool },
    PruneWorktree { name: String, expected_metadata: String },
    SaveStashKeepingIndex { message:String, include_untracked:bool },
    SaveStash {
        message: String,
        include_untracked: bool,
    },
    PopStash {
        oid: String,
        restore_index: bool,
    },
    ApplyStash {
        oid: String,
        restore_index: bool,
    },
    DropStash {
        oid: String,
    },
    StageAll,
    UnstageAll,
    StageFiles { file_ids: Vec<String> },
    UnstageFiles { file_ids: Vec<String> },
    Stage {
        file_id: String,
    },
    Unstage {
        file_id: String,
    },
    AmendCommit {
        message: String,
        author: String,
        email: String,
    },
    Commit {
        message: String,
        author: String,
        email: String,
    },
    CreateBranch {
        name: String,
    },
    CreateBranchAt {
        name: String,
        oid: String,
    },
    DeleteBranch {
        name: String,
    },
    CreateAnnotatedTag {
        name: String,
        oid: String,
        message: String,
        author: String,
        email: String,
    },
    CreateTag {
        name: String,
        oid: String,
    },
    DeleteTag {
        name: String,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphPageDirection {
    Older,
    Newer,
    Latest,
}
pub struct Session {
    root: PathBuf,
    layout: layout::Layout,
    id: String,
    files: HashMap<String, PathBuf>,
    revision: String,
    graph_offset: usize,
    graph_page_size: usize,
    graph_window_size: usize,
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if !meta.is_file() || is_link(&meta) || meta.len() > limit {
        return Err("UNSUPPORTED_FILE: 일반 파일만 5 MiB까지 처리합니다.".into());
    }
    // Every use also checks the repository policy; this is not an OS sandbox.
    use std::io::Read;
    let mut out = Vec::new();
    fs::File::open(path)
        .map_err(err)?
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(err)?;
    if out.len() as u64 > limit {
        return Err("LIMIT: 파일 크기가 변경되었습니다.".into());
    }
    Ok(out)
}
fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn check_tree(dir: &Path, count: &mut usize, attrs: &mut bool, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err("LIMIT: 디렉터리 깊이가 너무 큽니다.".into());
    }
    for item in fs::read_dir(dir).map_err(err)? {
        let item = item.map_err(err)?;
        *count += 1;
        if *count > 100_000 {
            return Err("LIMIT: 초기 버전은 100,000개 이하 파일의 저장소만 지원합니다.".into());
        }
        let meta = fs::symlink_metadata(item.path()).map_err(err)?;
        if is_link(&meta) {
            return Err(
                "UNSUPPORTED_LAYOUT: 심볼릭 링크/junction이 있는 저장소는 아직 지원하지 않습니다."
                    .into(),
            );
        }
        if item.file_name() == ".gitattributes" {
            *attrs = true;
        }
        if meta.is_dir() {
            check_tree(&item.path(), count, attrs, depth + 1)?;
        } else if !meta.is_file() {
            return Err("UNSUPPORTED_LAYOUT: 특수 파일은 지원하지 않습니다.".into());
        }
    }
    Ok(())
}
fn validate(root: &Path) -> Result<Option<String>> {
    let layout = layout::Layout::read(root)?;
    if layout.linked && !layout.submodule {
        validate(layout.common.parent().ok_or("INVALID_WORKTREE_LINK")?)?;
        let mut count = 0;
        let mut attrs = false;
        check_tree(root, &mut count, &mut attrs, 0)?;
        return Ok(Some("Linked worktree: read-only prototype. Graph, diffs and history are available; use Terminal for Git writes.".into()));
    }
    let git = if layout.submodule {layout.private.clone()} else {root.join(".git")};
    let meta = fs::symlink_metadata(&git).map_err(|_| {
        "NOT_REPOSITORY: .git 디렉터리가 있는 저장소 루트를 선택하세요.".to_string()
    })?;
    if !meta.is_dir() || is_link(&meta) {
        return Err(
            "UNSUPPORTED_LAYOUT: bare/linked worktree/submodule은 아직 지원하지 않습니다.".into(),
        );
    }
    for path in [
        "objects/info/alternates",
        "objects/info/http-alternates",
        "commondir",
        "config.worktree",
        "shallow",
    ] {
        if git.join(path).exists() {
            return Err(format!("UNSUPPORTED_LAYOUT: {path}"));
        }
    }
    let raw=bounded(&git.join("config"),65536)?;
    let config=if layout.submodule {remote_remove::read_only_entries(&raw)?}else{remote_remove::entries(&raw)?};
    let mut count = 0;
    let mut attrs = false;
    check_tree(root, &mut count, &mut attrs, 0)?;
    let complex =
        attrs || git.join("info/attributes").exists() || config_policy::requires_read_only(&config);
    let active_hooks = git
        .join("hooks")
        .read_dir()
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| !e.file_name().to_string_lossy().ends_with(".sample"));
    Ok(if layout.submodule {
        Some("Submodule: read-only prototype. Graph, diffs and history are available; nested Git writes are not implemented.".into())
    } else if complex || active_hooks {
        Some("속성·필터·서명 또는 훅이 있는 저장소는 읽기 전용입니다. 해당 동작을 생략하고 커밋하지 않습니다.".into())
    } else {
        None
    })
}
fn open(root: &Path) -> Result<Repository> {
    let layout = layout::Layout::read(root)?;
    let repo = Repository::open_ext(
        &layout.private,
        RepositoryOpenFlags::BARE | RepositoryOpenFlags::NO_SEARCH | RepositoryOpenFlags::NO_DOTGIT,
        std::iter::empty::<&str>(),
    )
    .map_err(err)?;
    repo.set_config(&Config::new().map_err(err)?).map_err(err)?;
    repo.set_workdir(root, false).map_err(err)?;
    Ok(repo)
}
fn rel(root: &Path, path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("INVALID_PATH".into());
    }
    let mut result = root.to_path_buf();
    for part in path.components() {
        let s = part.as_os_str().to_string_lossy();
        if s.eq_ignore_ascii_case(".git") || s.contains(':') || s.contains('\\') {
            return Err("INVALID_PATH".into());
        }
        result.push(part);
        match fs::symlink_metadata(&result) {
            Ok(m) if is_link(&m) => return Err("UNSUPPORTED_LINK".into()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(result)
}
fn file_id(path: &str) -> String {
    format!("{:x}", Sha256::digest(path.as_bytes()))
}
impl Session {
    pub fn new(root: PathBuf) -> Result<Self> {
        let root = root.canonicalize().map_err(err)?;
        validate(&root)?;
        Ok(Self {
            layout: layout::Layout::read(&root)?,
            root,
            id: Uuid::new_v4().to_string(),
            files: HashMap::new(),
            revision: String::new(),
            graph_offset: 0,
            graph_page_size: 500,
            graph_window_size: 500,
        })
    }
    pub fn reveal_path(&mut self,revision:&str,id:Option<&str>)->Result<(PathBuf,bool)>{
        self.fresh(revision)?;
        let Some(id)=id else{return Ok((self.root.clone(),false));};
        let relative=self.files.get(id).ok_or("INVALID_FILE_ID")?;
        let mut path=rel(&self.root,relative)?;
        if path.exists(){return Ok((path,true));}
        while path!=self.root&&!path.exists(){if !path.pop(){return Err("INVALID_PATH".into());}}
        if !path.starts_with(&self.root)||!path.is_dir(){return Err("INVALID_PATH".into());}
        Ok((path,false))
    }
    pub fn editor_file_path(&mut self, revision: &str, id: &str) -> Result<PathBuf> {
        self.fresh(revision)?;
        let relative = self.files.get(id).ok_or("INVALID_FILE_ID")?;
        let path = rel(&self.root, relative)?;
        if !path.is_file() || path.canonicalize().ok().as_deref() != Some(path.as_path()) {
            return Err("EDITOR_FILE_UNAVAILABLE".into());
        }
        Ok(path)
    }
    pub fn root_path(&self) -> &Path {
        &self.root
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let layout = layout::Layout::read(&self.root)?;
        if layout != self.layout { return Err("REPOSITORY_LAYOUT_CHANGED: reopen the repository".into()); }
        let (sequence_progress, sequence_error) = if layout.linked { (None, None) } else { match sequence_state::progress(&self.root) {
            Ok(progress) => (progress, None),
            Err(error) => (None, Some(format!("INVALID_SEQUENCE_STATE: {error}"))),
        } };
        let (replay_draft, replay_draft_error) = if layout.linked { (None, None) } else { match replay_draft::read(&self.root) {
            Ok(d) => (d, None),
            Err(e) => (None, Some(format!("INVALID_REPLAY_DRAFT: {e}"))),
        } };
        let (rebase_draft, rebase_draft_error) = if layout.linked { (None, None) } else { match rebase::draft::summary(&self.root) {
            Ok(d) => (d, None),
            Err(e) => (None, Some(e)),
        } };
        let (push_record, push_record_error) = if layout.linked { (None, None) } else { match push_state::read(&self.root) {
            Ok(r) => (r, None),
            Err(e) => (None, Some(e)),
        } };
        let (merge_draft, merge_draft_error) = if layout.linked { (None, None) } else { match merge_draft::read(&self.root) {
            Ok(draft) => (draft, None),
            Err(error) => (None, Some(format!("INVALID_MERGE_DRAFT: {error}"))),
        } };
        let recovery_pending = (layout.linked && !layout.submodule && linked_writes::pending(&self.root)) || (!layout.linked && (fetch::pending(&self.root)
            || remote_rename::pending(&self.root)
            || remote_head::pending(&self.root)
            || remote_remove::pending(&self.root)
            || remote_write::pending(&self.root)
            || branch_rename::pending(&self.root)
            || commit_write::pending(&self.root)
            || merge_abort::pending(&self.root)
            || merge_finish::pending(&self.root)
            || external_merge::pending(&self.root)
            || checkout::pending(&self.root)
            || stash_write::pending(&self.root)
            || stash_apply::pending(&self.root)
            || stash_save::pending(&self.root)));
        let linked_policy=if layout.linked&&!layout.submodule {linked_writes::policy(&self.root)?} else {None};
        let linked_local_writes=layout.linked&&!layout.submodule&&linked_policy.is_none()&&!recovery_pending;
        let write_block = if layout.linked&&!layout.submodule {
            Some(linked_policy.unwrap_or_else(||if recovery_pending {"RECOVERY_REQUIRED: recover the interrupted linked worktree operation.".into()} else {"Linked worktree: Stage, Unstage and Commit are available. Other writes are not implemented yet.".into()}))
        } else {validate(&self.root)?.or_else(|| {
            recovery_pending.then(|| "RECOVERY_REQUIRED: 중단된 Git 작업을 복구해야 합니다.".into())
        })};
        let mut repo = open(&self.root)?;
        let mut stashes = Vec::new();
        repo.stash_foreach(|index, message, oid| {
            stashes.push(Stash {
                index,
                oid: oid.to_string(),
                message: message.chars().take(4096).collect(),
            });
            stashes.len() <= MAX_FILES
        })
        .map_err(err)?;
        if stashes.len() > MAX_FILES {
            return Err("LIMIT: stash 2,000개 이하만 지원합니다.".into());
        }
        let head = repo.head().ok();
        let branch = head
            .as_ref()
            .and_then(|h| h.shorthand().ok())
            .map(str::to_owned)
            .or_else(|| {
                repo.find_reference("HEAD").ok().and_then(|h| {
                    h.symbolic_target()
                        .ok()
                        .flatten()
                        .and_then(|name| name.strip_prefix("refs/heads/"))
                        .map(str::to_owned)
                })
            })
            .unwrap_or_else(|| "unborn".into());
        let upstream_values = bounded(&layout.common.join("config"), 65536)
            .ok()
            .and_then(|b| if layout.submodule {remote_remove::read_only_entries(&b).ok()}else{remote_remove::entries(&b).ok()});
        let mut branches = Vec::new();
        let mut tags = Vec::new();
        let mut ref_fingerprints = Vec::new();
        let mut remote_tips = Vec::new();
        let mut labels: HashMap<Oid, Vec<String>> = HashMap::new();
        for (reference_index, reference) in repo.references().map_err(err)?.enumerate() {
            if reference_index >= 2000 {
                return Err("LIMIT: refs 2,000개 이하만 지원합니다.".into());
            }
            let r = reference.map_err(err)?;
            ref_fingerprints.push((
                r.name_bytes().to_vec(),
                r.target().map(|o| o.to_string()),
                r.symbolic_target_bytes().map(|s| s.to_vec()),
            ));
            // Recovery pins are revision inputs, not user-facing branch labels.
            if r.name_bytes().starts_with(b"refs/branchdesk/") {
                continue;
            }
            if r.is_tag() {
                let commit = r.peel_to_commit().ok();
                let name = r.shorthand().map_err(err)?.to_string();
                if let Some(c) = &commit {
                    labels.entry(c.id()).or_default().push(name.clone());
                }
                tags.push(Tag {
                    name,
                    oid: commit.map(|c| c.id().to_string()),
                    annotated: r.peel_to_tag().is_ok(),
                });
                continue;
            }
            if let Some(oid) = r.target() {
                if r.is_remote() {
                    remote_tips.push(oid);
                }
                let name = r.shorthand().unwrap_or("(non UTF-8)").to_string();
                labels.entry(oid).or_default().push(name.clone());
                if r.is_branch() {
                    branches.push(Branch {
                        upstream: upstream_values
                            .as_ref()
                            .map(|v| remote_write::upstream_config(v, &name)),
                        current: name == branch,
                        name,
                        oid: oid.to_string(),
                    });
                }
            }
        }
        if matches!(repo.head(), Err(ref e) if e.code() == git2::ErrorCode::UnbornBranch) {
            branches.push(Branch {
                upstream: upstream_values
                    .as_ref()
                    .map(|v| remote_write::upstream_config(v, &branch)),
                name: branch.clone(),
                oid: String::new(),
                current: true,
            });
        }
        let mut commits = Vec::new();
        if head.as_ref().and_then(|h| h.target()).is_some()
            || branches.iter().any(|b| !b.oid.is_empty())
            || !remote_tips.is_empty()
            || tags.iter().any(|t| t.oid.is_some())
        {
            let walk = search::walk(&repo)?;
            for oid in walk.skip(self.graph_offset).take(self.graph_window_size+1) {
                let c = repo.find_commit(oid.map_err(err)?).map_err(err)?;
                commits.push(Commit {
                    oid: c.id().to_string(),
                    parents: c.parent_ids().map(|x| x.to_string()).collect(),
                    summary: c
                        .summary()
                        .ok()
                        .flatten()
                        .unwrap_or("(non UTF-8 message)")
                        .chars()
                        .take(4096)
                        .collect(),
                    author: c
                        .author()
                        .name()
                        .unwrap_or("Unknown")
                        .chars()
                        .take(256)
                        .collect(),
                    time: c.time().seconds(),
                    refs: labels.remove(&c.id()).unwrap_or_default(),
                });
            }
        }
        if commits.is_empty() && self.graph_offset > 0 {
            self.graph_offset = 0;
            return self.snapshot();
        }
        let has_more = commits.len() > self.graph_window_size;
        commits.truncate(self.graph_window_size);
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .exclude_submodules(true)
            .update_index(false);
        let statuses = repo.statuses(Some(&mut opts)).map_err(err)?;
        if statuses.len() > MAX_FILES {
            return Err("LIMIT: 변경 파일 2,000개 이하만 지원합니다.".into());
        }
        let mut changes = Vec::new();
        self.files.clear();
        let mut hash = Sha256::new();
        hash.update(self.graph_offset.to_le_bytes());
        hash.update(self.graph_page_size.to_le_bytes());
        hash.update(self.graph_window_size.to_le_bytes());
        ref_fingerprints.sort();
        hash.update(serde_json::to_vec(&ref_fingerprints).map_err(err)?);
        for name in [
            "HEAD",
            "index",
            "config",
            "packed-refs",
            "FETCH_HEAD",
            "logs/refs/stash",
            "branchdesk-merge-draft.json",
            "branchdesk-replay-draft.json",
            "branchdesk-rebase-draft.json",
            "branchdesk-push.json",
            "branchdesk-replay-completion.json",
            "branchdesk-sequence.json",
            "MERGE_AUTOSTASH",
            "MERGE_HEAD",
            "MERGE_MSG",
            "MERGE_MODE",
            "ORIG_HEAD",
        ] {
            let path = layout.file(name);
            if path.exists() {
                hash.update(bounded(&path, 20 * 1024 * 1024)?);
            }
        }
        if let Some(h) = &head {
            if let Some(oid) = h.target() {
                hash.update(oid.as_bytes());
            }
        }
        let mut total_bytes = 0usize;
        for entry in statuses.iter() {
            let path = entry
                .path()
                .map_err(|_| "UNSUPPORTED_PATH: 비 UTF-8 경로는 아직 지원하지 않습니다.")?;
            let full = rel(&self.root, Path::new(path))?;
            let status = entry.status();
            hash.update(path.as_bytes());
            hash.update(status.bits().to_le_bytes());
            if full.is_file() {
                let bytes = bounded(&full, FILE_LIMIT)?;
                total_bytes += bytes.len();
                if total_bytes > 50 * 1024 * 1024 {
                    return Err("LIMIT: 변경 파일 합계는 50 MiB 이하만 지원합니다.".into());
                }
                hash.update(bytes);
            }
            let id = file_id(path);
            self.files.insert(id.clone(), PathBuf::from(path));
            changes.push(Change {
                id,
                path: path.to_string(),
                staged: status.is_index_new()
                    || status.is_index_modified()
                    || status.is_index_deleted()
                    || status.is_index_renamed()
                    || status.is_index_typechange(),
                unstaged: status.is_wt_new()
                    || status.is_wt_modified()
                    || status.is_wt_deleted()
                    || status.is_wt_renamed()
                    || status.is_wt_typechange(),
                status: if status.is_conflicted() {
                    "conflict"
                } else if status.is_wt_new() || status.is_index_new() {
                    "added"
                } else if status.is_wt_deleted() || status.is_index_deleted() {
                    "deleted"
                } else {
                    "modified"
                }
                .into(),
            });
        }
        self.revision = format!("{:x}", hash.finalize());
        Ok(Snapshot {
            repository_identity: remote_write::repository_identity(upstream_values.as_deref()),
            repo_id: self.id.clone(),
            name: self
                .root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            branch,
            detached: repo.head_detached().map_err(err)?,
            revision: self.revision.clone(),
            commits,
            branches,
            tags,
            stashes,
            remotes: remotes::inspect(&self.root, &repo)?,
            changes,
            has_more,
            graph_offset: self.graph_offset,
            graph_page_size: self.graph_page_size,
            graph_window_size: self.graph_window_size,
            write_block,
            recovery_pending,
            linked_local_writes,
            head_oid: head
                .as_ref()
                .and_then(|h| h.target())
                .map(|id| id.to_string()),
            rebase_draft,
            rebase_draft_error,
            push_record,
            push_record_error,
            merge_draft,
            merge_draft_error,
            replay_completion_ready: !layout.linked && replay_draft::completion_ready(&self.root, &repo)
                .unwrap_or(false),
            replay_draft,
            replay_draft_error,
            sequence_progress,
            sequence_error,
            external_merge: if layout.linked { None } else { external_merge::inspect(&self.root, &repo)? },
        })
    }
    pub fn set_graph_page_size(&mut self,revision:&str,size:usize)->Result<Snapshot>{
        if !(500..=5000).contains(&size){return Err("INVALID_GRAPH_PAGE_SIZE".into());}
        let current=self.fresh(revision)?;
        if size==self.graph_page_size{return Ok(current);}
        let previous=(self.graph_offset,self.graph_page_size,self.graph_window_size);
        self.graph_offset=0;self.graph_page_size=size;self.graph_window_size=size;
        match self.snapshot(){Ok(snapshot)=>Ok(snapshot),Err(error)=>{self.graph_offset=previous.0;self.graph_page_size=previous.1;self.graph_window_size=previous.2;Err(error)}}
    }
    pub fn extend_graph(&mut self,revision:&str)->Result<Snapshot>{
        let current=self.fresh(revision)?;
        if !current.has_more||self.graph_window_size>=20000{return Ok(current);}
        let previous=self.graph_window_size;
        self.graph_window_size=(previous+self.graph_page_size).min(20000);
        match self.snapshot(){Ok(snapshot)=>Ok(snapshot),Err(error)=>{self.graph_window_size=previous;Err(error)}}
    }
    pub fn graph_page(
        &mut self,
        revision: &str,
        direction: GraphPageDirection,
    ) -> Result<Snapshot> {
        let current = self.fresh(revision)?;
        let next = match direction {
            GraphPageDirection::Older if current.has_more => self
                .graph_offset
                .checked_add(self.graph_window_size)
                .ok_or("GRAPH_PAGE_LIMIT")?,
            GraphPageDirection::Older => return Ok(current),
            GraphPageDirection::Newer => self.graph_offset.saturating_sub(self.graph_page_size),
            GraphPageDirection::Latest => 0,
        };
        let previous = self.graph_offset;
        let previous_window=self.graph_window_size;
        self.graph_offset = next;
        self.graph_window_size=self.graph_page_size;
        match self.snapshot() {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                self.graph_offset = previous;
                self.graph_window_size=previous_window;
                Err(error)
            }
        }
    }
    fn fresh(&mut self, revision: &str) -> Result<Snapshot> {
        let snap = self.snapshot()?;
        if snap.revision != revision {
            return Err(
                "STALE_STATE: 저장소가 변경되었습니다. 새로고침 후 다시 시도하세요.".into(),
            );
        }
        Ok(snap)
    }
    pub fn diff(&mut self, revision: &str, id: &str, staged: bool) -> Result<DiffResult> {
        self.fresh(revision)?;
        let path = self.files.get(id).ok_or("INVALID_FILE_ID")?;
        let repo = open(&self.root)?;
        let mut opts = DiffOptions::new();
        opts.pathspec(path)
            .disable_pathspec_match(true)
            .include_untracked(true)
            .show_untracked_content(true)
            .recurse_untracked_dirs(true)
            .max_size(FILE_LIMIT as i64);
        let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        let diff = if staged {
            repo.diff_tree_to_index(tree.as_ref(), None, Some(&mut opts))
        } else {
            repo.diff_index_to_workdir(None, Some(&mut opts))
        }
        .map_err(err)?;
        let mut result = render_diff(&diff)?;
        if !result.truncated {
            result.hunks = hunks::describe(&diff)?;
            result.selectable_lines = line_staging::describe(&diff)?;
        }
        Ok(result)
    }
    pub fn replay_sequence_preview(
        &mut self,
        revision: &str,
        selections: &[ReplaySelection],
    ) -> Result<ReplaySequencePlan> {
        replay_sequence::preview(self, revision, selections)
    }
    pub fn replay_preview(
        &mut self,
        revision: &str,
        oid: &str,
        kind: ReplayKind,
        mainline: Option<usize>,
    ) -> Result<ReplayPlan> {
        let mut plan = replay::preview(self, revision, oid, kind, mainline)?;
        if let Some(draft) = replay_draft::read(&self.root)? {
            if draft.source == oid
                && draft.kind == kind
                && draft.mainline == mainline
                && (draft.head != plan.head || draft.branch != self.fresh(revision)?.branch)
            {
                return Err("STALE_REPLAY_DRAFT".into());
            }
            if draft.source == oid
                && draft.kind == kind
                && draft.mainline == mainline
                && draft.head == plan.head
                && draft.resolutions.len() == plan.conflicts.len()
                && !plan.conflicts.is_empty()
            {
                let resolved = replay::calculate_resolved(
                    self,
                    revision,
                    oid,
                    kind,
                    mainline,
                    Some(&draft.resolutions),
                )?
                .0;
                plan.resolved_empty = Some(resolved.empty && resolved.blockers.is_empty());
            }
        }
        Ok(plan)
    }
    pub fn merge_preview(&mut self, revision: &str, name: &str) -> Result<MergePlan> {
        merge::preview(self, revision, name)
    }
    pub fn file_content(
        &mut self,
        revision: &str,
        id: &str,
        source: ContentSource,
    ) -> Result<FileContent> {
        let mut content=content::read(self, revision, id, source)?;
        if content.state=="text"&&!content.truncated{content.lfs_object=lfs::inspect(&self.root,&content.text);if content.lfs_object.is_some(){match lfs::upload_state::read(&self.root){Ok(record)=>content.lfs_upload_record=record,Err(_)=>content.lfs_upload_error=Some("LFS upload record could not be read safely; new uploads are blocked until it is repaired.".into())}}}
        self.fresh(revision)?;
        Ok(content)
    }
    pub fn file_blame(
        &mut self,
        revision: &str,
        id: &str,
        anchor: Option<&str>,
        offset: usize,
        selected: &str,
    ) -> Result<FileBlame> {
        blame::read(self, revision, id, anchor, offset, selected)
    }
    pub fn file_history(
        &mut self,
        revision: &str,
        id: &str,
        anchor: Option<&str>,
        offset: usize,
        selected: Option<&str>,
    ) -> Result<FileHistory> {
        history::read(self, revision, id, anchor, offset, selected)
    }
    pub fn commit_detail(
        &mut self,
        revision: &str,
        oid: &str,
        id: Option<&str>,
    ) -> Result<CommitDetail> {
        let snap = self.fresh(revision)?;
        if !snap.commits.iter().any(|c| c.oid == oid) && snap.head_oid.as_deref() != Some(oid) {
            return Err("INVALID_COMMIT: 현재 그래프의 커밋을 선택하세요.".into());
        }
        let repo = open(&self.root)?;
        let commit = repo
            .find_commit(Oid::from_str(oid).map_err(err)?)
            .map_err(err)?;
        let tree = commit.tree().map_err(err)?;
        // Merge commits are compared against their first parent, explicitly shown in the UI.
        let parent = if commit.parent_count() > 0 {
            Some(commit.parent(0).map_err(err)?)
        } else {
            None
        };
        let parent_tree = parent.as_ref().map(|p| p.tree()).transpose().map_err(err)?;
        let mut opts = DiffOptions::new();
        opts.max_size(FILE_LIMIT as i64);
        let diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
            .map_err(err)?;
        if diff.deltas().len() > MAX_FILES {
            return Err("LIMIT: 커밋 변경 파일은 2,000개까지 지원합니다.".into());
        }
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(Path::to_str)
                .ok_or("UNSUPPORTED_PATH")?
                .to_string();
            files.push(Change {
                id: file_id(&path),
                path,
                staged: false,
                unstaged: false,
                status: match delta.status() {
                    git2::Delta::Added => "added",
                    git2::Delta::Deleted => "deleted",
                    _ => "modified",
                }
                .into(),
            });
        }
        let patch = if let Some(id) = id {
            let file = files.iter().find(|f| f.id == id).ok_or("INVALID_FILE_ID")?;
            let mut opts = DiffOptions::new();
            opts.pathspec(&file.path)
                .disable_pathspec_match(true)
                .max_size(FILE_LIMIT as i64);
            Some(render_diff(
                &repo
                    .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
                    .map_err(err)?,
            )?)
        } else {
            None
        };
        Ok(CommitDetail {
            oid: oid.into(),
            message: String::from_utf8_lossy(commit.message_bytes())
                .chars()
                .take(10000)
                .collect(),
            parent_oid: parent.map(|p| p.id().to_string()),
            files,
            diff: patch,
        })
    }
    pub fn stash_detail(
        &mut self,
        revision: &str,
        oid: &str,
        area: Option<StashArea>,
        id: Option<&str>,
    ) -> Result<StashDetail> {
        if area.is_some() != id.is_some() {
            return Err("INVALID_STASH_SELECTION".into());
        }
        let snap = self.fresh(revision)?;
        if !snap.stashes.iter().any(|s| s.oid == oid) {
            return Err("INVALID_STASH".into());
        }
        let repo = open(&self.root)?;
        let stash = repo
            .find_commit(Oid::from_str(oid).map_err(err)?)
            .map_err(err)?;
        if !(2..=3).contains(&stash.parent_count()) {
            return Err("UNSUPPORTED_STASH_LAYOUT".into());
        }
        let base = stash.parent(0).map_err(err)?;
        let base_tree = base.tree().map_err(err)?;
        let index_tree = stash.parent(1).map_err(err)?.tree().map_err(err)?;
        let worktree = stash.tree().map_err(err)?;
        let untracked = if stash.parent_count() == 3 {
            Some(stash.parent(2).map_err(err)?.tree().map_err(err)?)
        } else {
            None
        };
        let mut groups = Vec::new();
        let mut patch = None;
        for (kind, old, new) in [
            (StashArea::Staged, Some(&base_tree), Some(&index_tree)),
            (StashArea::Unstaged, Some(&index_tree), Some(&worktree)),
            (StashArea::Untracked, None, untracked.as_ref()),
        ] {
            let mut files = Vec::new();
            if let Some(new) = new {
                let mut opts = DiffOptions::new();
                opts.max_size(FILE_LIMIT as i64);
                let diff = repo
                    .diff_tree_to_tree(old, Some(new), Some(&mut opts))
                    .map_err(err)?;
                if diff.deltas().len() > MAX_FILES {
                    return Err("LIMIT: stash 변경 파일 2,000개까지 지원합니다.".into());
                }
                for delta in diff.deltas() {
                    let path = delta
                        .new_file()
                        .path()
                        .or_else(|| delta.old_file().path())
                        .and_then(Path::to_str)
                        .ok_or("UNSUPPORTED_PATH")?
                        .to_string();
                    files.push(Change {
                        id: file_id(&path),
                        path,
                        staged: false,
                        unstaged: false,
                        status: match delta.status() {
                            git2::Delta::Added => "added",
                            git2::Delta::Deleted => "deleted",
                            _ => "modified",
                        }
                        .into(),
                    });
                }
                if area == Some(kind) {
                    let file = files
                        .iter()
                        .find(|f| Some(f.id.as_str()) == id)
                        .ok_or("INVALID_FILE_ID")?;
                    let mut opts = DiffOptions::new();
                    opts.pathspec(&file.path)
                        .disable_pathspec_match(true)
                        .max_size(FILE_LIMIT as i64);
                    patch = Some(render_diff(
                        &repo
                            .diff_tree_to_tree(old, Some(new), Some(&mut opts))
                            .map_err(err)?,
                    )?);
                }
            } else if area == Some(kind) {
                return Err("INVALID_FILE_ID".into());
            }
            groups.push(StashGroup { area: kind, files });
        }
        Ok(StashDetail {
            oid: oid.into(),
            message: String::from_utf8_lossy(stash.message_bytes())
                .chars()
                .take(10000)
                .collect(),
            base_oid: base.id().to_string(),
            groups,
            diff: patch,
        })
    }
    pub fn mutation_label(&self, action: &Action) -> Result<String> {
        match action {
            Action::SetGitflowConfiguration {configuration}=>configuration.validate()?,
            Action::ApplyPatchIndex{text}=>{crate::patch_preview::parse("Apply".into(),text.clone())?;},
            Action::StageFiles { file_ids } | Action::UnstageFiles { file_ids } => {
                if file_ids.is_empty() || file_ids.len()>MAX_FILES || file_ids.iter().any(|id|id.len()!=64||!id.bytes().all(|b|b.is_ascii_hexdigit())) {return Err("INVALID_FILE_SELECTION".into());}
            },
            Action::CreateRecoveryBranch { reference, oid, .. } => checkpoints::validate(reference, oid)?,
            Action::SetRepositoryIdentity {
                identity: Some(identity),
            } => crate::identity_settings::validate(identity)?,
            Action::SetUpstream { name, target }
                if name.len() > 1024
                    || target
                        .as_ref()
                        .is_some_and(|t| t.remote.len() > 200 || t.branch.len() > 1024) =>
            {
                return Err("INVALID_INPUT".into())
            }
            Action::SoftReset { oid } | Action::CheckoutCommit { oid } | Action::CreateBranchAt { oid, .. }
                if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                return Err("INVALID_COMMIT_ID".into())
            }
            Action::ReplayCommit {
                oid,
                message,
                author,
                email,
                ..
            } if oid.len() != 40
                || !oid.bytes().all(|c| c.is_ascii_hexdigit())
                || message.len() > 10000
                || author.len() > 256
                || email.len() > 256 =>
            {
                return Err("INVALID_INPUT".into())
            }
            Action::RenameBranch { name, new_name } if name.len() > 200 || new_name.len() > 200 => {
                return Err("INVALID_INPUT".into())
            }
            Action::MergeCommit {
                message,
                author,
                email,
                ..
            }
            | Action::Commit {
                message,
                author,
                email,
            } if message.len() > 10000 || author.len() > 256 || email.len() > 256 => {
                return Err("INVALID_INPUT".into())
            }
            Action::MergeCommit { name, .. }
            | Action::FastForwardMerge { name }
            | Action::StartMerge { name } => {
                merge::reference_name(name)?;
            }
            Action::SetRemoteTagPolicy { name, .. }
            | Action::RemoveRemote { name }
            | Action::Checkout { name }
            | Action::CreateBranch { name }
            | Action::CreateBranchAt { name, .. }
            | Action::DeleteBranch { name }
            | Action::CreateAnnotatedTag { name, .. }
            | Action::CreateTag { name, .. }
            | Action::DeleteTag { name }
                if name.len() > 200 =>
            {
                return Err("INVALID_INPUT".into())
            }
            _ => (),
        }
        Ok(match action {
            Action::StartRebase {upstream,..}=>format!("Start rebase resolution draft onto {upstream}"),
            Action::ResolveRebase {..}=>"Save rebase conflict resolution".into(),
            Action::SkipRebase {..}=>"Skip current rebase commit".into(),
            Action::ContinueRebase=>"Apply resolved rebase or finish completed draft".into(),
            Action::AbortRebase=>"Discard rebase resolutions; preserve current files".into(),
            Action::Rebase {upstream,author,email} => {merge::reference_name(upstream)?;if author.len()>256 || email.len()>256{return Err("INVALID_INPUT".into());}format!("Rebase current branch onto {upstream}; replaces local commit history")},
            Action::RenameRemote { name, new_name } => format!("Rename remote {name} to {new_name} and update local tracking configuration"),
            Action::RemoveRemote { name } => format!("Remove local remote configuration and unshared tracking refs: {name} (server unchanged)"),
            Action::SetGitflowConfiguration {configuration} => format!("Configure Gitflow: main {}, develop {}; changes only Gitflow configuration keys; branches and files remain unchanged",configuration.main,configuration.develop),
            Action::SetRepositoryIdentity{identity} => match identity {Some(i)=>format!("Set repository commit identity: {} <{}>; updates only this repository user.name and user.email",i.name,i.email),None=>"Clear this repository user.name and user.email; app default applies when operation fields are empty".into()},
            Action::SetUpstream {name,target} => match target {Some(t)=>format!("Set Pull upstream for {name} to {}/{}; changes local tracking configuration only",t.remote,t.branch),None=>format!("Clear Pull upstream for {name}")},
            Action::SetRemoteTagPolicy { name, policy } => format!("Change fetched tag policy for {name}: {policy:?}"),
            Action::SetRemote { create, .. } => if *create { "Add local remote configuration" } else { "Replace remote fetch/push endpoints" }.into(),
            Action::StartSequence { selections } => {
                format!("Start replay sequence ({} steps)", selections.len())
            }
            Action::ContinueSequence { .. } => {
                "Apply or resume one sequence step using saved intent".into()
            }
            Action::ResolveSequence { .. } => "Save current sequence conflict resolution".into(),
            Action::SkipSequence { ordinal, .. } => format!(
                "Skip sequence step {} without creating a commit",
                ordinal.saturating_add(1)
            ),
            Action::AbortSequence => "Abort sequence: rewind to original HEAD and restore its files (clean worktree required)".into(),
            Action::StopSequence => {
                "Stop sequence and archive progress; keep all applied commits and current files"
                    .into()
            }
            Action::StartReplay { oid, operation, .. } => {
                format!("Start {operation:?} resolution: {oid}")
            }
            Action::ResolveReplay { .. } => "Save replay resolution".into(),
            Action::AbortReplay => "Abort replay resolution (archive draft)".into(),
            Action::SkipReplay => "Skip empty replay without creating a commit".into(),
            Action::ContinueReplay { .. } => "Continue replay with saved resolutions".into(),
            Action::ReplayCommit { oid, operation, .. } => format!("{operation:?} commit: {oid}"),
            Action::RenameBranch { name, new_name } => {
                format!("Rename local branch: {name} → {new_name}")
            }
            Action::ApplyPatchIndex{text}=>{
                let patch=crate::patch_preview::parse("Apply".into(),text.clone())?;
                let paths=patch.files.iter().take(20).map(|file|file.path.as_str()).collect::<Vec<_>>().join("\n");
                format!("Apply whole patch to staged index only ({} files). Working files and HEAD remain unchanged.\nSHA-256: {:x}\n{}{}",patch.files.len(),Sha256::digest(text.as_bytes()),paths,if patch.files.len()>20{"\n… additional files shown in patch preview"}else{""})
            },
            Action::StageLines {file_id,lines,unstage}=>{
                let path=self.files.get(file_id).ok_or("INVALID_FILE_ID")?;
                if lines.is_empty()||lines.len()>10000{return Err("INVALID_LINE_SELECTION".into());}
                format!("{} {} selected diff lines in {}. Working files remain unchanged.",if *unstage {"Unstage"}else{"Stage"},lines.len(),path.display())
            },
            Action::StageHunk { hunk, unstage, .. } => format!(
                "{} hunk {}",
                if *unstage { "Unstage" } else { "Stage" },
                hunk.saturating_add(1)
            ),
            Action::AbortExternalMerge => {
                "Abort external merge: discard merge resolutions and restore affected paths to HEAD"
                    .into()
            }
            Action::ContinueExternalMerge { .. } => {
                "Complete external Git merge with staged index".into()
            }
            Action::StageExternalResolution { .. } => {
                "Stage resolved worktree file in external Git merge".into()
            }
            Action::StartMerge { name } => format!("Start conflict resolution: {}", merge::display_name(name)),
            Action::ResolveMerge { .. } => "Save merge conflict resolution".into(),
            Action::AbortMerge => "Abort pending merge resolution".into(),
            Action::ContinueMerge { .. } => "Continue merge and create two-parent commit".into(),
            Action::MergeCommit { name, .. } => format!("Create merge commit from branch: {}", merge::display_name(name)),
            Action::CreateAnnotatedTag { name, oid, .. } => format!(
                "Create annotated tag: {name} at {}",
                oid.chars().take(12).collect::<String>()
            ),
            Action::SaveStashKeepingIndex{message,include_untracked}=>format!("Save stash and keep staged changes (include untracked: {include_untracked}): {}",message.chars().take(120).collect::<String>()),
            Action::SaveStash {
                message,
                include_untracked,
            } => format!(
                "Save stash (include untracked: {include_untracked}): {}",
                message.chars().take(120).collect::<String>()
            ),
            Action::PopStash { oid, restore_index } => format!(
                "Pop stash (apply then drop): {} (restore index: {restore_index})",
                oid.chars().take(40).collect::<String>()
            ),
            Action::ApplyStash { oid, restore_index } => format!(
                "Apply stash: {} (restore index: {restore_index})",
                oid.chars().take(40).collect::<String>()
            ),
            Action::DropStash { oid } => {
                format!("Drop stash: {}", oid.chars().take(40).collect::<String>())
            }
            Action::FastForwardMerge { name } => {
                format!("Fast-forward merge into current branch: {}", merge::display_name(name))
            }
            Action::CheckoutCommit {oid} => format!("Checkout commit {oid} in detached HEAD (no branch moves; clean worktree required)"),
            Action::Checkout { name } => format!("Checkout branch: {name}"),
            Action::RecoverCheckout => "Recover interrupted Git operation".into(),
            Action::SoftReset { oid } => format!("Move current branch to {oid}; keep staged changes and working files. Published history may diverge. Previous tip is retained as a recovery checkpoint."),
            Action::IgnoreFile { file_id } => format!("Add exact path to repository .gitignore (keep file on disk): {}", self.files.get(file_id).ok_or("INVALID_FILE_ID")?.display()),
            Action::CreateRecoveryBranch { reference, oid, name } => format!("Create recovery branch {name} from {reference} at {oid}. Current branch, files and index stay unchanged."),
            Action::CreateWorktree { name, path, branch } => format!("Create worktree {name} at {path}; branch: {}. Current worktree stays unchanged. Supported linked worktrees allow Stage, Unstage and Commit; other writes require an external Git client.", branch.as_deref().unwrap_or(name)),
            Action::PruneWorktree { name, .. } => format!("Prune missing worktree {name}. Only its registration is removed; branch refs and checkout files are untouched. Metadata is retained under .git/branchdesk-pruned-worktrees for manual recovery. Locked or existing checkouts cannot be pruned."),
            Action::SetWorktreeLock { name, locked, .. } => format!("{} worktree {name}. This protects against pruning/removal; it does not prevent file editing.", if *locked {"Lock"} else {"Unlock"}),
            Action::StageFiles { file_ids } | Action::UnstageFiles { file_ids } => {
                let verb=if matches!(action,Action::UnstageFiles{..}){"Unstage"}else{"Stage"};
                let paths=file_ids.iter().take(20).map(|id|self.files.get(id).map(|p|p.to_string_lossy().into_owned()).ok_or("INVALID_FILE_ID")).collect::<std::result::Result<Vec<_>,_>>()?;
                format!("{verb} {} selected files\n{}{}",file_ids.len(),paths.join("\n"),if file_ids.len()>20 {"\n… (remaining files listed in the app review)"}else{""})
            },
            Action::StageAll => "Stage all unstaged changes".into(),
            Action::UnstageAll => "Unstage all staged changes".into(),
            Action::Stage { file_id } => format!(
                "Stage: {}",
                self.files.get(file_id).ok_or("INVALID_FILE_ID")?.display()
            ),
            Action::Unstage { file_id } => format!(
                "Unstage: {}",
                self.files.get(file_id).ok_or("INVALID_FILE_ID")?.display()
            ),
            Action::AmendCommit { message, .. } => format!(
                "Amend current HEAD (replace last commit, preserve original author): {}",
                message.chars().take(120).collect::<String>()
            ),
            Action::Commit { message, .. } => {
                format!("Commit: {}", message.chars().take(120).collect::<String>())
            }
            Action::CreateBranchAt {name,oid} => format!("Create branch {name} at selected commit {oid} (keep current checkout)"),
            Action::CreateBranch { name } => format!("Create branch: {name}"),
            Action::DeleteBranch { name } => format!("Delete merged branch: {name}"),
            Action::CreateTag { name, oid } => format!(
                "Create tag: {name} at {}",
                oid.chars().take(12).collect::<String>()
            ),
            Action::DeleteTag { name } => format!("Delete tag: {name}"),
        })
    }
    pub fn mutate(&mut self, revision: &str, action: Action) -> Result<Snapshot> {
        let snap = self.fresh(revision)?;
        if self.layout.linked&&!self.layout.submodule {
            if !linked_writes::supports(&action){return Err("UNSUPPORTED_LINKED_OPERATION".into());}
            if let Some(reason)=linked_writes::policy(&self.root)? {return Err(format!("READ_ONLY: {reason}"));}
            if snap.recovery_pending&&!matches!(action,Action::RecoverCheckout){return Err("RECOVERY_REQUIRED".into());}
            if !snap.recovery_pending&&matches!(action,Action::RecoverCheckout){return Err("NO_PENDING_RECOVERY".into());}
        } else if let Some(reason) = &snap.write_block {
            if !matches!(action, Action::RecoverCheckout) || validate(&self.root)?.is_some() {
                return Err(format!("READ_ONLY: {reason}"));
            }
        }
        if !self.layout.linked && sequence_state::exists(&self.root)
            && !matches!(
                action,
                Action::RecoverCheckout
                    | Action::ContinueSequence { .. }
                    | Action::ResolveSequence { .. }
                    | Action::SkipSequence { .. }
                    | Action::AbortSequence
                    | Action::StopSequence
            )
        {
            return Err(
                "SEQUENCE_IN_PROGRESS: pending sequence must be resolved before other writes"
                    .into(),
            );
        }
        if !self.layout.linked && rebase::draft::exists(&self.root)
            && !matches!(
                action,
                Action::ResolveRebase { .. }
                    | Action::SkipRebase { .. }
                    | Action::ContinueRebase
                    | Action::AbortRebase
                    | Action::RecoverCheckout
            )
        {
            return Err("REBASE_IN_PROGRESS".into());
        }
        if !self.layout.linked && replay_draft::exists(&self.root)
            && !matches!(
                action,
                Action::ResolveReplay { .. }
                    | Action::ContinueReplay { .. }
                    | Action::AbortReplay
                    | Action::SkipReplay
                    | Action::RecoverCheckout
            )
        {
            return Err("REPLAY_IN_PROGRESS: continue or abort the pending replay".into());
        }
        if !self.layout.linked && merge_draft::exists(&self.root)
            && !matches!(
                action,
                Action::ResolveMerge { .. }
                    | Action::AbortMerge
                    | Action::ContinueMerge { .. }
                    | Action::RecoverCheckout
            )
        {
            return Err("MERGE_IN_PROGRESS: continue or abort the pending merge".into());
        }
        let lease = lease::OperationLease::acquire(&self.root)?;
        if matches!(action, Action::RecoverCheckout) && checkout::pending(&self.root) {
            checkout::release_owned_locks(&self.root, &lease)?;
        }
        if matches!(action, Action::RecoverCheckout) {
            journal_locks::release(&self.root, &lease)?;
        }
        let repo = open(&self.root)?;
        if repo.state() != git2::RepositoryState::Clean
            && !(repo.state() == git2::RepositoryState::Merge
                && matches!(
                    action,
                    Action::StageExternalResolution { .. }
                        | Action::AbortExternalMerge
                        | Action::ContinueExternalMerge { .. }
                        | Action::RecoverCheckout
                ))
        {
            return Err("UNSUPPORTED_STATE: merge/rebase 중에는 변경할 수 없습니다.".into());
        }
        match &action {
            Action::StartRebase { .. }
            | Action::ResolveRebase { .. }
            | Action::SkipRebase { .. }
            | Action::ContinueRebase
            | Action::AbortRebase => {
                if self.root.join(".git/index.lock").exists() {
                    return Err("EXTERNAL_LOCK".into());
                }
                match action {
                    Action::StartRebase {
                        upstream,
                        author,
                        email,
                    } => rebase::draft::start(self, revision, &upstream, &author, &email)?,
                    Action::ResolveRebase {
                        draft_id,
                        source,
                        file_id,
                        resolution,
                    } => rebase::draft::resolve(
                        self, revision, &draft_id, &source, &file_id, resolution,
                    )?,
                    Action::SkipRebase { draft_id, source } => {
                        rebase::draft::skip(self, revision, &draft_id, &source)?
                    }
                    Action::ContinueRebase => rebase::draft::finish(self, revision)?,
                    Action::AbortRebase => rebase::draft::abort(self, revision)?,
                    _ => unreachable!(),
                }
                return self.snapshot();
            }
            _ => {}
        }
        match action {
            Action::StartSequence { selections } => {
                if self.root.join(".git/index.lock").exists() {
                    return Err("EXTERNAL_LOCK".into());
                }
                sequence_state::start(self, revision, selections)?;
                return self.snapshot();
            }
            Action::ContinueSequence {
                message,
                author,
                email,
                allow_empty,
            } => {
                sequence_state::advance(self, revision, message, author, email, allow_empty)?;
                return self.snapshot();
            }
            Action::ResolveSequence {
                sequence_id,
                ordinal,
                file_id,
                resolution,
            } => {
                sequence_state::save_resolution(
                    self,
                    revision,
                    &sequence_id,
                    ordinal,
                    &file_id,
                    resolution,
                )?;
                return self.snapshot();
            }
            Action::SkipSequence {
                sequence_id,
                ordinal,
            } => {
                sequence_state::skip_step(self, revision, &sequence_id, ordinal)?;
                return self.snapshot();
            }
            Action::AbortSequence => {
                sequence_state::abort(self, revision)?;
                return self.snapshot();
            }
            Action::StopSequence => {
                sequence_state::stop(self, revision)?;
                return self.snapshot();
            }
            _ => {}
        }
        match &action {
            Action::StartReplay { .. }
            | Action::ResolveReplay { .. }
            | Action::AbortReplay
            | Action::SkipReplay => {
                if self.root.join(".git/index.lock").exists() {
                    return Err("EXTERNAL_LOCK".into());
                }
                self.fresh(revision)?;
                match action {
                    Action::StartReplay {
                        oid,
                        operation,
                        mainline,
                    } => replay_draft::start(self, revision, &oid, operation, mainline)?,
                    Action::ResolveReplay {
                        file_id,
                        resolution,
                    } => replay_draft::save(self, revision, &file_id, resolution)?,
                    Action::SkipReplay => replay_draft::skip_empty(self, revision)?,
                    Action::AbortReplay => {
                        replay_draft::archive(&self.root)?;
                    }
                    _ => unreachable!(),
                }
                return self.snapshot();
            }
            _ => {}
        }
        // Draft-only operations never publish index/worktree data. Avoid leaving an
        // unjournaled index.lock behind if the process dies while saving a draft.
        match &action {
            Action::StartMerge { .. } | Action::ResolveMerge { .. } | Action::AbortMerge => {
                if self.root.join(".git/index.lock").exists() {
                    return Err("EXTERNAL_LOCK".into());
                }
                self.fresh(revision)?;
                match action {
                    Action::StartMerge { name } => merge_draft::start(self, revision, &name)?,
                    Action::ResolveMerge {
                        file_id,
                        resolution,
                    } => merge_draft::save_resolution(self, revision, &file_id, resolution)?,
                    Action::AbortMerge => merge_draft::archive(&self.root)?,
                    _ => unreachable!(),
                }
                return self.snapshot();
            }
            _ => (),
        }
        // Prune never writes the index. Avoid an unrelated index.lock which could
        // survive a crash without a journal, while retaining the process lease.
        if let Action::PruneWorktree {name,expected_metadata}=&action {
            worktrees::prune(self,revision,name,expected_metadata)?;
            return self.snapshot();
        }
        let mut lock = IndexLock::acquire(&self.root)?;
        match &action {
            Action::Checkout { name } | Action::DeleteBranch { name } | Action::RenameBranch { name, .. } => worktrees::require_branch_available(&self.root, &repo, name)?,
            _ => {},
        }
        // Re-check after taking Git's interoperable index lock.
        self.fresh(revision)?;
        let mut index = git2::Index::open(&lock.path).map_err(err)?;
        if index.has_conflicts()
            && !matches!(
                action,
                Action::StageExternalResolution { .. }
                    | Action::AbortExternalMerge
                    | Action::RecoverCheckout
            )
        {
            return Err("CONFLICT: 충돌을 먼저 해결하세요.".into());
        }
        match action {
            Action::RenameRemote { name, new_name } => {
                remote_rename::execute(self, &repo, revision, &name, &new_name)?;
                drop(lock);
                remote_rename::finish(&self.root)?;
                return self.snapshot();
            }
            Action::SoftReset { oid } => soft_reset::execute(self, &repo, revision, &oid)?,
            Action::IgnoreFile { file_id } => ignore::execute(self, &repo, revision, &file_id)?,
            Action::CreateWorktree { name, path, branch } => worktrees::create(self, &repo, revision, &name, &path, branch.as_deref())?,
            Action::SetWorktreeLock { name, expected_locked, locked } => worktrees::set_lock(self, revision, &name, expected_locked, locked)?,
            Action::PruneWorktree { .. } => unreachable!(),
            Action::RemoveRemote { name } => {
                remote_remove::execute(self, &repo, revision, &name)?;
                drop(lock);
                remote_remove::finish(&self.root)?;
                return self.snapshot();
            }
            Action::SetGitflowConfiguration {configuration} => {
                remote_write::execute_gitflow(self,revision,configuration)?;
                drop(lock);remote_write::finish(&self.root)?;return self.snapshot();
            }
            Action::SetRepositoryIdentity { identity } => {
                remote_write::execute_identity(self, revision, identity)?;
                drop(lock);
                remote_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::SetUpstream { name, target } => {
                remote_write::execute_upstream(self, revision, name, target)?;
                drop(lock);
                remote_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::SetRemoteTagPolicy { name, policy } => {
                remote_write::execute_tag_policy(self, revision, name, policy)?;
                drop(lock);
                remote_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::SetRemote {
                name,
                url,
                push_url,
                create,
            } => {
                remote_write::execute(self, revision, name, url, push_url, create)?;
                drop(lock);
                remote_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::RenameBranch { name, new_name } => {
                branch_rename::execute(self, &repo, revision, &name, &new_name)?;
                drop(lock);
                branch_rename::finish(&self.root)?;
                return self.snapshot();
            }
            Action::ApplyPatchIndex{text}=>{
                patch_apply::execute(self,&repo,revision,&text,&mut lock)?;
                drop(lock);external_merge::finish(&self.root)?;return self.snapshot();
            }
            Action::StageLines {file_id,lines,unstage}=>{
                line_staging::execute(self,&repo,revision,&file_id,&lines,unstage,&mut lock)?;
                drop(lock);external_merge::finish(&self.root)?;return self.snapshot();
            }
            Action::StageHunk {
                file_id,
                hunk,
                unstage,
            } => {
                hunks::execute(self, &repo, revision, &file_id, hunk, unstage, &mut lock)?;
                drop(lock);
                external_merge::finish(&self.root)?;
                return self.snapshot();
            }

            Action::AbortExternalMerge => {
                merge_abort::execute(self, &repo, revision, &mut lock, None)?;
                drop(lock);
                merge_abort::finish(&self.root)?;
                return self.snapshot();
            }

            Action::ContinueExternalMerge {
                message,
                author,
                email,
            } => {
                merge_finish::execute(
                    self,
                    &repo,
                    revision,
                    &checkout::MergeInput {
                        name: "",
                        message: &message,
                        author: &author,
                        email: &email,
                    },
                    None,
                )?;
                drop(lock);
                merge_finish::finish(&self.root)?;
                return self.snapshot();
            }

            Action::StageExternalResolution { file_id } => {
                external_merge::stage(self, &repo, revision, &file_id, &mut lock, None)?;
                drop(lock);
                external_merge::finish(&self.root)?;
                return self.snapshot();
            }

            Action::StartMerge { name } => {
                merge_draft::start(self, revision, &name)?;
            }
            Action::ResolveMerge {
                file_id,
                resolution,
            } => {
                merge_draft::save_resolution(self, revision, &file_id, resolution)?;
            }
            Action::AbortMerge => {
                merge_draft::archive(&self.root)?;
            }
            Action::ContinueMerge {
                message,
                author,
                email,
            } => {
                let draft = merge_draft::read(&self.root)?.ok_or("NO_MERGE_DRAFT")?;
                if snap.branch != draft.branch
                    || repo.head().map_err(err)?.target().map(|o| o.to_string())
                        != Some(draft.head.clone())
                    || repo
                        .refname_to_id(&merge::reference_name(&draft.name)?)
                        .map_err(err)?
                        .to_string()
                        != draft.target
                {
                    return Err("STALE_MERGE_DRAFT".into());
                }
                checkout::merge_commit(
                    self,
                    &repo,
                    revision,
                    &checkout::MergeInput {
                        name: &draft.name,
                        message: &message,
                        author: &author,
                        email: &email,
                    },
                    &mut lock,
                    None,
                )?;
                drop(lock);
                merge_draft::archive(&self.root)?;
                return self.snapshot();
            }

            Action::SaveStashKeepingIndex{message,include_untracked}=>stash_save::execute_options(self,&repo,revision,&message,stash_save::SaveOptions{include_untracked,keep_index:true},&mut lock,None)?,
            Action::SaveStash {
                message,
                include_untracked,
            } => stash_save::execute(
                self,
                &repo,
                revision,
                &message,
                include_untracked,
                &mut lock,
                None,
            )?,
            Action::PopStash { oid, restore_index } => {
                stash_apply::pop(self, &repo, revision, &oid, restore_index, &mut lock, None)?
            }
            Action::ApplyStash { oid, restore_index } => {
                stash_apply::apply(self, &repo, revision, &oid, restore_index, &mut lock, None)?;
            }
            Action::StartRebase { .. }
            | Action::ResolveRebase { .. }
            | Action::SkipRebase { .. }
            | Action::ContinueRebase
            | Action::AbortRebase
            | Action::StartSequence { .. }
            | Action::ContinueSequence { .. }
            | Action::ResolveSequence { .. }
            | Action::SkipSequence { .. }
            | Action::AbortSequence
            | Action::StopSequence
            | Action::StartReplay { .. }
            | Action::ResolveReplay { .. }
            | Action::AbortReplay
            | Action::SkipReplay => {
                unreachable!()
            }
            Action::ContinueReplay {
                allow_empty,
                message,
                author,
                email,
            } => {
                if replay_draft::completion_ready(&self.root, &repo)? {
                    drop(lock);
                    replay_draft::archive(&self.root)?;
                    return self.snapshot();
                }
                let draft = replay_draft::read(&self.root)?.ok_or("NO_REPLAY_DRAFT")?;
                if snap.branch != draft.branch
                    || repo.head().map_err(err)?.target().map(|o| o.to_string())
                        != Some(draft.head.clone())
                {
                    return Err("STALE_REPLAY_DRAFT".into());
                }
                checkout::replay_commit(
                    self,
                    &repo,
                    revision,
                    &checkout::ReplayInput {
                        complete_draft: true,
                        committer_time: None,
                        expected_oid: None,
                        allow_empty,
                        resolutions: Some(&draft.resolutions),
                        oid: &draft.source,
                        kind: draft.kind,
                        mainline: draft.mainline,
                        message: &message,
                        author: &author,
                        email: &email,
                    },
                    &mut lock,
                    None,
                )?;
                drop(lock);
                replay_draft::after_commit_checkpoint(&self.root);
                replay_draft::archive(&self.root)?;
                return self.snapshot();
            }
            Action::ReplayCommit {
                allow_empty,
                oid,
                operation,
                mainline,
                message,
                author,
                email,
            } => {
                checkout::replay_commit(
                    self,
                    &repo,
                    revision,
                    &checkout::ReplayInput {
                        complete_draft: false,
                        committer_time: None,
                        expected_oid: None,
                        allow_empty,
                        resolutions: None,
                        oid: &oid,
                        kind: operation,
                        mainline,
                        message: &message,
                        author: &author,
                        email: &email,
                    },
                    &mut lock,
                    None,
                )?;
            }
            Action::MergeCommit {
                name,
                message,
                author,
                email,
            } => {
                checkout::merge_commit(
                    self,
                    &repo,
                    revision,
                    &checkout::MergeInput {
                        name: &name,
                        message: &message,
                        author: &author,
                        email: &email,
                    },
                    &mut lock,
                    None,
                )?;
            }
            Action::Rebase {
                upstream,
                author,
                email,
            } => {
                let prepared =
                    rebase::execution::prepare(self, revision, &upstream, &author, &email)?;
                checkout::rebase(self, &repo, revision, &prepared, &mut lock)?;
            }
            Action::FastForwardMerge { name } => {
                checkout::fast_forward(self, &repo, revision, &name, &mut lock, None)?
            }
            Action::CheckoutCommit { oid } => {
                checkout::detached(self, &repo, revision, &oid, &mut lock, None)?;
            }
            Action::Checkout { name } => {
                checkout::execute(self, &repo, revision, &name, &mut lock, None)?
            }
            Action::DropStash { oid } => {
                stash_write::drop_stash(self, &repo, revision, &oid, None)?
            }
            Action::RecoverCheckout => {
                if self.layout.linked {
                    if commit_write::pending(&self.root) {commit_write::recover(&self.root,&repo)?;drop(lock);commit_write::finish(&self.root)?;}
                    else if external_merge::pending(&self.root) {external_merge::recover(&self.root,&mut lock)?;drop(lock);external_merge::finish(&self.root)?;}
                    else {return Err("NO_PENDING_RECOVERY".into());}
                    return self.snapshot();
                }
                if fetch::pending(&self.root) {
                    fetch::recover(&self.root)?;
                    drop(lock);
                    fetch::finish(&self.root)?;
                    return self.snapshot();
                }
                if remote_rename::pending(&self.root) {
                    remote_rename::recover(&self.root)?;
                    drop(lock);
                    remote_rename::finish(&self.root)?;
                    return self.snapshot();
                }
                if remote_head::pending(&self.root) {
                    remote_head::recover(&self.root)?;
                    drop(lock);
                    remote_head::finish(&self.root)?;
                    return self.snapshot();
                }
                if remote_remove::pending(&self.root) {
                    remote_remove::recover(&self.root)?;
                    drop(lock);
                    remote_remove::finish(&self.root)?;
                    return self.snapshot();
                }
                if remote_write::pending(&self.root) {
                    remote_write::recover(&self.root)?;
                    drop(lock);
                    remote_write::finish(&self.root)?;
                    return self.snapshot();
                }
                if branch_rename::pending(&self.root) {
                    branch_rename::recover(&self.root, &repo)?;
                    drop(lock);
                    branch_rename::finish(&self.root)?;
                    return self.snapshot();
                }
                if commit_write::pending(&self.root) {
                    commit_write::recover(&self.root, &repo)?;
                    drop(lock);
                    commit_write::finish(&self.root)?;
                    return self.snapshot();
                }
                if merge_abort::pending(&self.root) {
                    merge_abort::recover(&self.root, &repo, &mut lock)?;
                    drop(lock);
                    merge_abort::finish(&self.root)?;
                    return self.snapshot();
                }
                if merge_finish::pending(&self.root) {
                    merge_finish::recover(&self.root, &repo)?;
                    drop(lock);
                    merge_finish::finish(&self.root)?;
                    return self.snapshot();
                }
                if external_merge::pending(&self.root) {
                    external_merge::recover(&self.root, &mut lock)?;
                    drop(lock);
                    external_merge::finish(&self.root)?;
                    return self.snapshot();
                }
                if stash_save::pending(&self.root) {
                    stash_save::recover(&self.root, &repo, &mut lock)?;
                } else if stash_write::pending(&self.root) {
                    stash_apply::record_completed_lock(&self.root)?;
                    stash_write::recover(&self.root, &repo)?;
                    if stash_apply::pending(&self.root) {
                        stash_apply::recover(&self.root, &repo, &mut lock)?;
                    }
                } else if stash_apply::pending(&self.root) {
                    stash_apply::recover(&self.root, &repo, &mut lock)?;
                } else {
                    checkout::recover(&self.root, &repo, &mut lock)?;
                }
            }
            Action::Stage { .. } | Action::StageAll | Action::StageFiles { .. } => {
                let branch = external_merge::head_lock_target(&repo)?;
                let _refs = checkout::locks(&repo, &branch, &branch)?;
                self.fresh(revision)?;
                let ids: Vec<String> = match action {
                    Action::Stage { file_id } => vec![file_id],
                    Action::StageFiles { file_ids } => selected_file_ids(&snap,file_ids,false)?,
                    _ => snap
                        .changes
                        .iter()
                        .filter(|c| c.unstaged)
                        .map(|c| c.id.clone())
                        .collect(),
                };
                for id in ids {
                    let path = self.files.get(&id).ok_or("INVALID_FILE_ID")?;
                    let full = rel(&self.root, path)?;
                    if full.exists() {
                        let data = bounded(&full, FILE_LIMIT)?;
                        let bytes = path.to_str().ok_or("UNSUPPORTED_PATH")?.as_bytes().to_vec();
                        let mode = {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                if full.metadata().map_err(err)?.permissions().mode() & 0o111 != 0 {
                                    0o100755
                                } else {
                                    0o100644
                                }
                            }
                            #[cfg(not(unix))]
                            {
                                index.get_path(path, 0).map(|e| e.mode).unwrap_or(0o100644)
                            }
                        };
                        let entry = IndexEntry {
                            ctime: IndexTime::new(0, 0),
                            mtime: IndexTime::new(0, 0),
                            dev: 0,
                            ino: 0,
                            mode,
                            uid: 0,
                            gid: 0,
                            file_size: data.len() as u32,
                            id: repo.blob(&data).map_err(err)?,
                            flags: 0,
                            flags_extended: 0,
                            path: bytes,
                        };
                        index.add(&entry).map_err(err)?;
                    } else {
                        index.remove_path(path).map_err(err)?;
                    }
                }
                self.fresh(revision)?;
                index.write().map_err(err)?;
                external_merge::publish(self, revision, &lock, &branch, None)?;
                drop(lock);
                external_merge::finish(&self.root)?;
                return self.snapshot();
            }
            Action::Unstage { .. } | Action::UnstageAll | Action::UnstageFiles { .. } => {
                let branch = external_merge::head_lock_target(&repo)?;
                let _refs = checkout::locks(&repo, &branch, &branch)?;
                self.fresh(revision)?;
                let ids: Vec<String> = match action {
                    Action::Unstage { file_id } => vec![file_id],
                    Action::UnstageFiles { file_ids } => selected_file_ids(&snap,file_ids,true)?,
                    _ => snap
                        .changes
                        .iter()
                        .filter(|c| c.staged)
                        .map(|c| c.id.clone())
                        .collect(),
                };
                for id in ids {
                    let path = self.files.get(&id).ok_or("INVALID_FILE_ID")?;
                    let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
                    let entry = tree.as_ref().and_then(|t| t.get_path(path).ok());
                    if let Some(e) = entry {
                        let item = IndexEntry {
                            ctime: IndexTime::new(0, 0),
                            mtime: IndexTime::new(0, 0),
                            dev: 0,
                            ino: 0,
                            mode: e.filemode() as u32,
                            uid: 0,
                            gid: 0,
                            file_size: 0,
                            id: e.id(),
                            flags: 0,
                            flags_extended: 0,
                            path: path.to_str().ok_or("UNSUPPORTED_PATH")?.as_bytes().to_vec(),
                        };
                        index.add(&item).map_err(err)?;
                    } else {
                        index.remove_path(path).map_err(err)?;
                    }
                }
                self.fresh(revision)?;
                index.write().map_err(err)?;
                external_merge::publish(self, revision, &lock, &branch, None)?;
                drop(lock);
                external_merge::finish(&self.root)?;
                return self.snapshot();
            }
            Action::AmendCommit {
                message,
                author,
                email,
            } => {
                commit_write::execute(self, &repo, revision, &message, &author, &email, true)?;
                drop(lock);
                commit_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::Commit {
                message,
                author,
                email,
            } => {
                commit_write::execute(self, &repo, revision, &message, &author, &email, false)?;
                drop(lock);
                commit_write::finish(&self.root)?;
                return self.snapshot();
            }
            Action::CreateBranch { .. }
            | Action::CreateBranchAt { .. }
            | Action::DeleteBranch { .. }
            | Action::CreateRecoveryBranch { .. }
            | Action::CreateAnnotatedTag { .. }
            | Action::CreateTag { .. }
            | Action::DeleteTag { .. } => self.mutate_ref(&repo, revision, action)?,
        }
        drop(lock);
        stash_apply::finish(&self.root)?;
        self.snapshot()
    }
    fn mutate_ref(&mut self, repo: &Repository, revision: &str, action: Action) -> Result<()> {
        let (name, namespace) = match &action {
            Action::CreateBranch { name }
            | Action::CreateRecoveryBranch { name, .. }
            | Action::CreateBranchAt { name, .. }
            | Action::DeleteBranch { name } => (name, "heads"),
            Action::CreateAnnotatedTag { name, .. }
            | Action::CreateTag { name, .. }
            | Action::DeleteTag { name } => (name, "tags"),
            _ => return Err("INVALID_ACTION".into()),
        };
        let full = format!("refs/{namespace}/{name}");
        if name.is_empty()
            || name.len() > 200
            || name.starts_with('-')
            || (namespace == "heads" && name == "HEAD")
            || !git2::Reference::is_valid_name(&full)
        {
            return Err("INVALID_REF_NAME".into());
        }
        // Lock HEAD, its current target and the changed ref, then revalidate the preview.
        // git2 transactions are NOT atomic across multiple writes. Only ONE ref is modified here.
        let mut locks = vec!["HEAD".to_string(), full.clone()];
        if let Action::CreateRecoveryBranch { reference, oid, .. } = &action {
            checkpoints::validate(reference, oid)?;
            locks.push(reference.clone());
        }
        if let Some(current) = repo
            .find_reference("HEAD")
            .map_err(err)?
            .symbolic_target()
            .map_err(err)?
        {
            locks.push(current.into());
        }
        locks.sort();
        locks.dedup();
        let mut tx = repo.transaction().map_err(err)?;
        for reference in locks {
            tx.lock_ref(&reference).map_err(err)?;
        }
        let snap = self.fresh(revision)?;
        let existing = match repo.find_reference(&full) {
            Ok(reference) => Some(reference),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(err(e)),
        };
        let signature =
            git2::Signature::now("FLOE", "local@floe.invalid").map_err(err)?;
        match action {
            Action::CreateBranchAt { oid, .. } => {
                if existing.is_some() {
                    return Err("REF_EXISTS".into());
                }
                if oid.len() != 40
                    || !oid.bytes().all(|b| b.is_ascii_hexdigit())
                    || !(snap.commits.iter().any(|c| c.oid == oid)
                        || snap.branches.iter().any(|branch| branch.oid == oid))
                {
                    return Err("INVALID_COMMIT".into());
                }
                let target = repo
                    .find_commit(Oid::from_str(&oid).map_err(err)?)
                    .map_err(err)?;
                tx.set_target(
                    &full,
                    target.id(),
                    Some(&signature),
                    &format!("branch: Created from {oid}"),
                )
                .map_err(err)?;
            }
            Action::CreateRecoveryBranch { reference, oid, .. } => {
                if existing.is_some() { return Err("REF_EXISTS".into()); }
                let expected = Oid::from_str(&oid).map_err(err)?;
                if repo.find_reference(&reference).map_err(err)?.target() != Some(expected) { return Err("STALE_CHECKPOINT".into()); }
                repo.find_commit(expected).map_err(err)?;
                tx.set_target(&full, expected, Some(&signature), "branch: Created from recovery checkpoint").map_err(err)?;
            }
            Action::CreateBranch { .. } => {
                if existing.is_some() {
                    return Err("REF_EXISTS".into());
                }
                let oid = repo
                    .head()
                    .map_err(err)?
                    .peel_to_commit()
                    .map_err(err)?
                    .id();
                tx.set_target(&full, oid, Some(&signature), "branch: Created from HEAD")
                    .map_err(err)?;
            }
            Action::CreateAnnotatedTag {
                name,
                oid,
                message,
                author,
                email,
            } => {
                if existing.is_some() {
                    return Err("REF_EXISTS".into());
                }
                if message.trim().is_empty()
                    || message.len() > 10000
                    || message.contains('\0')
                    || author.trim().is_empty()
                    || author.len() > 256
                    || author.chars().any(|c| c.is_control())
                    || email.len() > 256
                    || !email.contains('@')
                    || email.chars().any(|c| c.is_control())
                {
                    return Err("INVALID_TAG_INPUT".into());
                }
                if !snap.commits.iter().any(|c| c.oid == oid) {
                    return Err("INVALID_COMMIT".into());
                }
                let target = repo
                    .find_commit(Oid::from_str(&oid).map_err(err)?)
                    .map_err(err)?;
                let tagger = git2::Signature::now(&author, &email).map_err(err)?;
                // Create the object without publishing a ref, then publish only
                // the locked tag ref. A failure leaves at most an unreachable object.
                let annotation = repo
                    .tag_annotation_create(&name, target.as_object(), &tagger, &message)
                    .map_err(err)?;
                tx.set_target(
                    &full,
                    annotation,
                    Some(&tagger),
                    "tag: Created annotated tag",
                )
                .map_err(err)?;
            }
            Action::CreateTag { oid, .. } => {
                if existing.is_some() {
                    return Err("REF_EXISTS".into());
                }
                if !snap.commits.iter().any(|c| c.oid == oid) {
                    return Err("INVALID_COMMIT".into());
                }
                tx.set_target(
                    &full,
                    Oid::from_str(&oid).map_err(err)?,
                    Some(&signature),
                    "tag: Created from selected commit",
                )
                .map_err(err)?;
            }
            Action::DeleteBranch { .. } => {
                let reference = existing.ok_or("REF_NOT_FOUND")?;
                let head = repo.head().map_err(err)?;
                if head.name().map_err(err)? == full.as_str() {
                    return Err("CURRENT_BRANCH: 현재 브랜치는 삭제할 수 없습니다.".into());
                }
                let head_oid = head.peel_to_commit().map_err(err)?.id();
                let branch_oid = reference.peel_to_commit().map_err(err)?.id();
                if head_oid != branch_oid
                    && !repo
                        .graph_descendant_of(head_oid, branch_oid)
                        .map_err(err)?
                {
                    return Err(
                        "UNMERGED_BRANCH: 현재 HEAD에 병합되지 않은 브랜치는 보존합니다.".into(),
                    );
                }
                tx.remove(&full).map_err(err)?;
            }
            Action::DeleteTag { .. } => {
                existing.ok_or("REF_NOT_FOUND")?;
                tx.remove(&full).map_err(err)?;
            }
            _ => return Err("INVALID_ACTION".into()),
        }
        tx.commit().map_err(err)
    }
}

fn render_diff(diff: &git2::Diff<'_>) -> Result<DiffResult> {
    let mut text = String::new();
    let mut truncated = false;
    diff.print(DiffFormat::Patch, |_, _, line| {
        if text.len() + line.content().len().saturating_mul(3) + 1 > 512 * 1024 {
            truncated = true;
            return false;
        }
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin());
        }
        text.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .or_else(|e| if truncated { Ok(()) } else { Err(e) })
    .map_err(err)?;
    if text.is_empty() {
        text = "표시할 텍스트 diff가 없습니다. 바이너리 또는 변경 없는 파일입니다.".into();
    }
    Ok(DiffResult {
        text,
        truncated,
        hunks: Vec::new(),
        selectable_lines: Vec::new(),
    })
}

// Cooperates with Git and other clients using the standard index.lock convention.
// Keep the lock until a fully written replacement index is ready; never remove
// another process's pre-existing lock.
const EMPTY_INDEX: [u8; 32] = [
    68, 73, 82, 67, 0, 0, 0, 2, 0, 0, 0, 0, 57, 216, 144, 19, 158, 229, 53, 108, 126, 245, 114, 33,
    108, 235, 205, 39, 170, 65, 249, 223,
];
struct IndexLock {
    path: PathBuf,
    owned: bool,
}
impl IndexLock {
    #[cfg(test)]
    fn publish(&mut self, destination: &Path) -> Result<()> {
        fs::OpenOptions::new()
            .read(true)
            .write(cfg!(windows))
            .open(&self.path)
            .map_err(err)?
            .sync_all()
            .map_err(err)?;
        fs::rename(&self.path, destination).map_err(err)?;
        self.owned = false;
        Ok(())
    }
    fn acquire(root: &Path) -> Result<Self> {
        use std::io::Write;
        let layout = layout::Layout::read(root)?;
        let path = layout.private.join("index.lock");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| {
                "EXTERNAL_LOCK: 다른 Git 작업이 진행 중이거나 index 잠금이 남아 있습니다."
                    .to_string()
            })?;
        let guard = Self { path, owned: true };
        let index = layout.private.join("index");
        if index.exists() {
            file.write_all(&bounded(&index, 20 * 1024 * 1024)?)
                .map_err(err)?;
        } else {
            let repo = open(root)?;
            match repo.head() {
                Err(e) if e.code() == git2::ErrorCode::UnbornBranch => (),
                _ => return Err("MISSING_INDEX: existing repository index must be restored".into()),
            }
            // Git index v2: DIRC, version 2, zero entries, SHA-1 of the header.
            // Write only our private lock; opening an unborn repo must not create index.
            file.write_all(&EMPTY_INDEX).map_err(err)?;
        }
        file.sync_all().map_err(err)?;
        drop(file);
        Ok(guard)
    }
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}


fn selected_file_ids(snapshot:&Snapshot,ids:Vec<String>,staged:bool)->Result<Vec<String>> {
    if ids.is_empty()||ids.len()>MAX_FILES {return Err("INVALID_FILE_SELECTION".into());}
    let mut unique=std::collections::HashSet::new();
    for id in &ids {
        if !unique.insert(id)||!snapshot.changes.iter().any(|change|change.id==*id&&if staged {change.staged}else{change.unstaged}) {return Err("INVALID_FILE_SELECTION".into());}
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
        let mut i = r.index().unwrap();
        i.add_path(Path::new("hello.txt")).unwrap();
        i.write().unwrap();
        let t = r.find_tree(i.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        r.commit(Some("HEAD"), &sig, &sig, "Initial", &t, &[])
            .unwrap();
        dir
    }
    #[test]
    fn remote_only_history_is_visible_without_moving_head_or_index() {
        for unborn in [false, true] {
            let dir = fixture();
            let repo = Repository::open(dir.path()).unwrap();
            let initial = repo.head().unwrap().peel_to_commit().unwrap();
            let sig = git2::Signature::now("Remote Author", "remote@example.invalid").unwrap();
            let tree = initial.tree().unwrap();
            let remote = repo
                .commit(None, &sig, &sig, "Remote-only commit", &tree, &[&initial])
                .unwrap();
            repo.reference("refs/remotes/origin/main", remote, false, "fixture")
                .unwrap();
            // Internal safety pins must not leak otherwise-unreachable commits into the graph.
            let hidden = repo
                .commit(None, &sig, &sig, "Hidden checkpoint", &tree, &[&initial])
                .unwrap();
            repo.reference("refs/branchdesk/checkpoints/test", hidden, false, "fixture")
                .unwrap();
            if unborn {
                repo.set_head("refs/heads/new-local").unwrap();
            }
            let head_bytes = fs::read(dir.path().join(".git/HEAD")).unwrap();
            let index_bytes = fs::read(dir.path().join(".git/index")).unwrap();
            let mut session = Session::new(dir.path().into()).unwrap();
            let snapshot = session.snapshot().unwrap();
            assert_eq!(
                snapshot
                    .commits
                    .iter()
                    .filter(|c| c.oid == initial.id().to_string())
                    .count(),
                1
            );
            assert!(!snapshot.commits.iter().any(|c| c.oid == hidden.to_string()));
            let found = snapshot
                .commits
                .iter()
                .find(|c| c.oid == remote.to_string())
                .unwrap();
            assert!(found.refs.contains(&"origin/main".into()));
            let detail = session
                .commit_detail(&snapshot.revision, &remote.to_string(), None)
                .unwrap();
            assert_eq!(detail.message, "Remote-only commit");
            assert_eq!(detail.parent_oid, Some(initial.id().to_string()));
            assert_eq!(fs::read(dir.path().join(".git/HEAD")).unwrap(), head_bytes);
            assert_eq!(
                fs::read(dir.path().join(".git/index")).unwrap(),
                index_bytes
            );
        }
    }

    #[test]
    fn stage_commit_and_unstage_preserve_worktree() {
        let dir = fixture();
        fs::write(dir.path().join("hello.txt"), "hello\nworld\n").unwrap();
        let mut s = Session::new(dir.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let id = a.changes[0].id.clone();
        assert!(s
            .diff(&a.revision, &id, false)
            .unwrap()
            .text
            .contains("+world"));
        let b = s
            .mutate(
                &a.revision,
                Action::Stage {
                    file_id: id.clone(),
                },
            )
            .unwrap();
        assert!(b.changes[0].staged);
        let c = s
            .mutate(
                &b.revision,
                Action::Unstage {
                    file_id: id.clone(),
                },
            )
            .unwrap();
        assert!(!c.changes[0].staged);
        let d = s
            .mutate(&c.revision, Action::Stage { file_id: id })
            .unwrap();
        let e = s
            .mutate(
                &d.revision,
                Action::Commit {
                    message: "Second".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                },
            )
            .unwrap();
        assert!(e.changes.is_empty());
        assert_eq!(e.commits[0].summary, "Second");
        assert_eq!(
            fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
            "hello\nworld\n"
        );
    }
    #[test]
    fn batch_stage_unstage_preserves_partial_index_and_worktree() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "staged version\n").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let a = s.mutate(&a.revision, Action::StageAll).unwrap();
        assert!(a.changes[0].staged);
        fs::write(d.path().join("hello.txt"), "working version\n").unwrap();
        fs::write(d.path().join("new.txt"), "new file\n").unwrap();
        let a = s.snapshot().unwrap();
        let b = s.mutate(&a.revision, Action::StageAll).unwrap();
        assert_eq!(b.changes.len(), 2);
        assert!(b.changes.iter().all(|c| c.staged && !c.unstaged));
        assert!(s
            .diff(&b.revision, &file_id("hello.txt"), true)
            .unwrap()
            .text
            .contains("+working version"));
        let c = s.mutate(&b.revision, Action::UnstageAll).unwrap();
        assert!(c.changes.iter().all(|c| !c.staged && c.unstaged));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "working version\n"
        );
        assert_eq!(
            fs::read_to_string(d.path().join("new.txt")).unwrap(),
            "new file\n"
        );
        fs::remove_file(d.path().join("hello.txt")).unwrap();
        let a = s.snapshot().unwrap();
        let b = s.mutate(&a.revision, Action::StageAll).unwrap();
        assert!(b
            .changes
            .iter()
            .any(|c| c.path == "hello.txt" && c.status == "deleted" && c.staged));
        let c = s.mutate(&b.revision, Action::UnstageAll).unwrap();
        assert!(c.changes.iter().all(|c| !c.staged));
        assert!(!d.path().join("hello.txt").exists());
    }
    #[test]
    fn whole_file_stage_and_unstage_actual_kill_restore_exact_index() {
        for action in ["stageAll", "unstageAll", "stageFile", "unstageFile"] {
            for step in 0..=1 {
                let d = fixture();
                dirty_three_areas(d.path());
                fs::write(d.path().join("binary.bin"), b"binary\0data").unwrap();
                let index = fs::read(d.path().join(".git/index")).unwrap();
                let head = fs::read(d.path().join(".git/HEAD")).unwrap();
                kill_draft(d.path(), action, &format!("index:{step}"));
                // A working-file edit after interruption must survive index rollback.
                fs::write(d.path().join("hello.txt"), "later worktree edit\n").unwrap();
                let mut s = Session::new(d.path().into()).unwrap();
                let a = s.snapshot().unwrap();
                assert!(a.recovery_pending);
                let a = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert!(!a.recovery_pending);
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "later worktree edit\n"
                );
                assert_eq!(
                    fs::read(d.path().join("binary.bin")).unwrap(),
                    b"binary\0data"
                );
                assert!(!d.path().join(".git/index.lock").exists());
            }
        }
    }
    #[test]
    fn whole_file_batch_recovery_can_resume_after_second_kill() {
        for action in ["stageAll", "unstageAll"] {
            for step in 2..=3 {
                let d = fixture();
                dirty_three_areas(d.path());
                let index = fs::read(d.path().join(".git/index")).unwrap();
                kill_draft(d.path(), action, "index:1");
                kill_draft(d.path(), "recover", &format!("index:{step}"));
                let mut s = Session::new(d.path().into()).unwrap();
                let a = s.snapshot().unwrap();
                s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "worktree version\n"
                );
            }
        }
    }
    #[test]
    fn whole_file_first_stage_crash_restores_absent_index_and_can_retry() {
        for step in 0..=1 {
            let d = tempfile::tempdir().unwrap();
            let mut opts = git2::RepositoryInitOptions::new();
            opts.external_template(false).initial_head("main");
            Repository::init_opts(d.path(), &opts).unwrap();
            fs::write(d.path().join("hello.txt"), "first\n").unwrap();
            kill_draft(d.path(), "stageAll", &format!("index:{step}"));
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let a = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!d.path().join(".git/index").exists());
            let a = s.mutate(&a.revision, Action::StageAll).unwrap();
            assert!(a.changes[0].staged);
            assert!(a.commits.is_empty());
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "first\n"
            );
        }
    }
    #[test]
    fn whole_file_recovery_refuses_later_external_index_edits() {
        let d = fixture();
        dirty_three_areas(d.path());
        kill_draft(d.path(), "stageAll", "index:1");
        // Simulate another Git client after stale owned markers have been released.
        {
            let lease = lease::OperationLease::acquire(d.path()).unwrap();
            journal_locks::release(d.path(), &lease).unwrap();
        }
        fs::write(d.path().join("hello.txt"), "external staged content\n").unwrap();
        let repo = open(d.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("hello.txt")).unwrap();
        index.write().unwrap();
        let bytes = fs::read(d.path().join(".git/index")).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let error = s.mutate(&a.revision, Action::RecoverCheckout).unwrap_err();
        assert!(error.contains("RECOVERY_CONFLICT"), "{error}");
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), bytes);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "external staged content\n"
        );
    }
    #[test]
    fn failed_batch_does_not_publish_any_index_changes() {
        let d = fixture();
        fs::write(d.path().join("a.txt"), "a").unwrap();
        fs::write(d.path().join("z.txt"), "z").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        fs::write(d.path().join("z.txt"), "changed after preview").unwrap();
        assert!(s
            .mutate(&a.revision, Action::StageAll)
            .unwrap_err()
            .starts_with("STALE_STATE"));
        assert_eq!(index, fs::read(d.path().join(".git/index")).unwrap());
        assert!(!d.path().join(".git/index.lock").exists());
        let a = s.snapshot().unwrap();
        fs::write(d.path().join(".git/index.lock"), "external").unwrap();
        assert!(s
            .mutate(&a.revision, Action::StageAll)
            .unwrap_err()
            .starts_with("EXTERNAL_LOCK"));
        assert_eq!(index, fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(
            fs::read_to_string(d.path().join(".git/index.lock")).unwrap(),
            "external"
        );
    }
    #[test]
    fn history_diff_reads_committed_tree_not_working_changes() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let initial = a.commits[0].oid.clone();
        let detail = s
            .commit_detail(&a.revision, &initial, Some(&file_id("hello.txt")))
            .unwrap();
        assert!(detail.parent_oid.is_none());
        assert_eq!(detail.files[0].status, "added");
        assert!(detail.diff.unwrap().text.contains("+hello"));
        fs::remove_file(d.path().join("hello.txt")).unwrap();
        fs::write(d.path().join("special[1].txt"), "committed\n").unwrap();
        fs::write(d.path().join("binary.dat"), [0, 1, 0, 2]).unwrap();
        let a = s.snapshot().unwrap();
        let a = s.mutate(&a.revision, Action::StageAll).unwrap();
        let a = s
            .mutate(
                &a.revision,
                Action::Commit {
                    message: "Second\n\nFull message".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                },
            )
            .unwrap();
        let oid = a.commits[0].oid.clone();
        fs::write(d.path().join("special[1].txt"), "uncommitted\n").unwrap();
        let a = s.snapshot().unwrap();
        let detail = s
            .commit_detail(&a.revision, &oid, Some(&file_id("special[1].txt")))
            .unwrap();
        assert_eq!(detail.parent_oid, Some(initial));
        assert!(detail.message.contains("Full message"));
        assert_eq!(detail.files.len(), 3);
        let patch = detail.diff.unwrap().text;
        assert!(patch.contains("+committed"));
        assert!(!patch.contains("+uncommitted"));
        let deleted = s
            .commit_detail(&a.revision, &oid, Some(&file_id("hello.txt")))
            .unwrap();
        assert!(deleted.diff.unwrap().text.contains("-hello"));
        let binary = s
            .commit_detail(&a.revision, &oid, Some(&file_id("binary.dat")))
            .unwrap();
        assert!(!binary.diff.unwrap().text.contains('\0'));
        assert!(s.commit_detail(&a.revision, &oid, Some("unknown")).is_err());
        assert!(s.commit_detail(&a.revision, "HEAD", None).is_err());
    }
    #[test]
    fn merge_history_compares_with_first_parent() {
        let d = fixture();
        let r = Repository::open(d.path()).unwrap();
        let first = r.head().unwrap().peel_to_commit().unwrap();
        let tree = first.tree().unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let side = r
            .commit(None, &sig, &sig, "Side", &tree, &[&first])
            .unwrap();
        let side = r.find_commit(side).unwrap();
        let merge = r
            .commit(Some("HEAD"), &sig, &sig, "Merge", &tree, &[&first, &side])
            .unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let detail = s
            .commit_detail(&a.revision, &merge.to_string(), None)
            .unwrap();
        assert_eq!(detail.parent_oid, Some(first.id().to_string()));
        assert!(detail.files.is_empty());
    }
    #[test]
    fn patch_output_is_bounded_even_for_a_single_long_line() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "x".repeat(600_000)).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let diff = s.diff(&a.revision, &file_id("hello.txt"), false).unwrap();
        assert!(diff.truncated);
        assert!(diff.text.len() <= 512 * 1024);
    }
    #[test]
    fn stale_request_cannot_write() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "one").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        fs::write(d.path().join("hello.txt"), "two").unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::Stage {
                    file_id: a.changes[0].id.clone()
                }
            )
            .unwrap_err()
            .starts_with("STALE_STATE"));
    }
    #[test]
    fn traversal_and_unknown_ids_rejected() {
        let d = fixture();
        assert!(rel(d.path(), Path::new("../outside")).is_err());
        assert!(rel(d.path(), Path::new(".git/config")).is_err());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s.diff(&a.revision, "unknown", false).is_err());
    }
    #[test]
    fn existing_index_lock_is_not_removed() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "change").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        fs::write(d.path().join(".git/index.lock"), "external-owner").unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::Stage {
                    file_id: a.changes[0].id.clone()
                }
            )
            .unwrap_err()
            .starts_with("EXTERNAL_LOCK"));
        assert_eq!(
            fs::read_to_string(d.path().join(".git/index.lock")).unwrap(),
            "external-owner"
        );
    }
    #[test]
    fn published_lock_does_not_remove_next_owners_lock() {
        let d = fixture();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        lock.publish(&d.path().join(".git/index")).unwrap();
        fs::write(d.path().join(".git/index.lock"), "next-owner").unwrap();
        drop(lock);
        assert_eq!(
            fs::read_to_string(d.path().join(".git/index.lock")).unwrap(),
            "next-owner"
        );
    }
    #[test]
    fn checkout_crash_worker() {
        if let Ok(root) = std::env::var("BRANCHDESK_DRAFT_ROOT") {
            let mut session = Session::new(PathBuf::from(root)).unwrap();
            let s = session.snapshot().unwrap();
            let action = match std::env::var("BRANCHDESK_DRAFT_ACTION").unwrap().as_str() {
                "start" => Action::StartMerge {
                    name: "feature/switch".into(),
                },
                "resolve" => Action::ResolveMerge {
                    file_id: file_id("hello.txt"),
                    resolution: merge_draft::Resolution::Text {
                        text: "latest resolution\n".into(),
                    },
                },
                "abort" => Action::AbortMerge,
                "recover" => Action::RecoverCheckout,
                "lines" => {
                    let preview=session.diff(&s.revision,&file_id("file"),false).unwrap();
                    let line=preview.selectable_lines.iter().find(|l|l.new_line.is_some()).unwrap();
                    Action::StageLines{file_id:file_id("file"),lines:vec![line_staging::Selection{hunk:line.hunk,line:line.line}],unstage:false}
                },
                "hunk" => Action::StageHunk {
                    file_id: file_id("file[1].txt"),
                    hunk: 1,
                    unstage: false,
                },
                "hunkNew" => Action::StageHunk {
                    file_id: file_id("new[1].txt"),
                    hunk: 0,
                    unstage: false,
                },
                "hunkDelete" => Action::StageHunk {
                    file_id: file_id("file[1].txt"),
                    hunk: 0,
                    unstage: false,
                },
                "amend" => Action::AmendCommit {
                    message: "Amended message".into(),
                    author: "Committer".into(),
                    email: "committer@example.invalid".into(),
                },
                "commit" => Action::Commit {
                    message: "Crash commit".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into(),
                },
                "patchIndex" => Action::ApplyPatchIndex {text: fs::read_to_string(session.root.join("test-input.patch")).unwrap()},
                "stageAll" => Action::StageAll,
                "unstageAll" => Action::UnstageAll,
                "stageFile" => Action::Stage {
                    file_id: file_id("hello.txt"),
                },
                "unstageFile" => Action::Unstage {
                    file_id: file_id("hello.txt"),
                },
                "externalAbort" => Action::AbortExternalMerge,
                "externalContinue" => Action::ContinueExternalMerge {
                    message: "External completed".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into(),
                },
                "continue" => Action::ContinueMerge {
                    message: "Resolved merge".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into(),
                },
                _ => panic!("unknown draft test action"),
            };
            session.mutate(&s.revision, action).unwrap();
            panic!("draft worker missed checkpoint");
        }

        if let Ok(root) = std::env::var("BRANCHDESK_STASH_ROOT") {
            let mut s = Session::new(PathBuf::from(root)).unwrap();
            let a = s.snapshot().unwrap();
            let kind = std::env::var("BRANCHDESK_STASH_KIND").unwrap();
            let action = if kind.ends_with("-recover") {
                Action::RecoverCheckout
            } else {
                match kind.as_str() {
                    "save" if std::env::var("BRANCHDESK_TEST_KEEP_INDEX").as_deref()==Ok("true") => Action::SaveStashKeepingIndex{message:"crash keep index".into(),include_untracked:true},
                    "save" => Action::SaveStash {
                        message: "crash test".into(),
                        include_untracked: true,
                    },
                    "apply" => Action::ApplyStash {
                        oid: a.stashes[0].oid.clone(),
                        restore_index: true,
                    },
                    "pop" => Action::PopStash { oid: a.stashes[0].oid.clone(), restore_index: true },
                    "drop" if std::env::var("BRANCHDESK_TEST_POP").as_deref()==Ok("true") => Action::PopStash {oid:a.stashes[0].oid.clone(),restore_index:true},
                    "drop" => Action::DropStash {
                        oid: a.stashes[0].oid.clone(),
                    },
                    _ => panic!("unknown stash worker kind"),
                }
            };
            s.mutate(&a.revision, action).unwrap();
            panic!("stash child missed checkpoint");
        }
        let Ok(root) = std::env::var("BRANCHDESK_CRASH_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        if std::env::var_os("BRANCHDESK_RECOVERY_STEP").is_some() {
            let mut session = Session::new(root.clone()).unwrap();
            let snapshot = session.snapshot().unwrap();
            session
                .mutate(&snapshot.revision, Action::RecoverCheckout)
                .unwrap();
            panic!("recovery child missed checkpoint");
        }
        let _lease = lease::OperationLease::acquire(&root).unwrap();
        let mut session = Session::new(root.clone()).unwrap();
        let snapshot = session.snapshot().unwrap();
        let repo = open(&root).unwrap();
        let mut lock = IndexLock::acquire(&root).unwrap();
        if std::env::var_os("BRANCHDESK_MERGE_COMMIT").is_some() {
            checkout::merge_commit(
                &mut session,
                &repo,
                &snapshot.revision,
                &checkout::MergeInput {
                    name: "feature/switch",
                    message: "Merge",
                    author: "Merger",
                    email: "merge@example.invalid",
                },
                &mut lock,
                None,
            )
            .unwrap();
            panic!("merge child missed checkpoint");
        }
        if let Ok(oid) = std::env::var("BRANCHDESK_DETACHED_OID") {
            checkout::detached(
                &mut session,
                &repo,
                &snapshot.revision,
                &oid,
                &mut lock,
                None,
            )
            .unwrap();
            panic!("detached child missed checkpoint");
        }
        let execute = if std::env::var_os("BRANCHDESK_FAST_FORWARD").is_some() {
            checkout::fast_forward
        } else {
            checkout::execute
        };
        execute(
            &mut session,
            &repo,
            &snapshot.revision,
            "feature/switch",
            &mut lock,
            None,
        )
        .unwrap();
        panic!("child did not reach configured crash checkpoint");
    }
    struct CrashChild(std::process::Child);
    impl Drop for CrashChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn suspended_stash(root: &Path, kind: &str, step: usize, keep_index:bool, pop:bool) -> CrashChild {
        let ready = root.join(".git/branchdesk-stash-test.ready");
        if ready.exists() {
            fs::remove_file(&ready).unwrap();
        }
        let mut child = CrashChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "repository::tests::checkout_crash_worker",
                    "--nocapture",
                ])
                .env("BRANCHDESK_STASH_ROOT", root)
                .env("BRANCHDESK_STASH_KIND", kind)
                .env("BRANCHDESK_STASH_STEP", step.to_string())
                .env("BRANCHDESK_TEST_KEEP_INDEX",keep_index.to_string())
                .env("BRANCHDESK_TEST_POP",pop.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "stash child exited before {kind}:{step}"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "stash child timeout {kind}:{step}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child
    }
    pub(super) fn kill_keep_stash(root:&Path,kind:&str,step:usize){
        let mut child=suspended_stash(root,kind,step,true,false);
        assert!(lease::OperationLease::acquire(root).is_err());
        child.0.kill().unwrap();child.0.wait().unwrap();
    }
    fn kill_stash(root: &Path, kind: &str, step: usize) {
        let mut child = suspended_stash(root, kind, step, false, false);
        assert!(lease::OperationLease::acquire(root).is_err());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
    }
    #[test]
    fn stash_save_actual_kill_restores_original_dirty_files_and_index() {
        for step in 0..=4 {
            let d = fixture();
            dirty_three_areas(d.path());
            let index = fs::read(d.path().join(".git/index")).unwrap();
            kill_stash(d.path(), "save", step);
            assert!(d.path().join(".git/index.lock").exists());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(
                fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
                "untracked version\n"
            );
            assert!(!d.path().join(".git/index.lock").exists());
        }
    }
    #[test]
    fn stash_apply_actual_kill_restores_clean_state_and_retains_stash() {
        for step in 0..=3 {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let index = fs::read(d.path().join(".git/index")).unwrap();
            kill_stash(d.path(), "apply", step);
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("untracked.txt").exists());
            assert_eq!(b.stashes[0].oid, oid);
        }
    }
    #[test]
    fn stash_drop_actual_kill_finishes_only_requested_drop() {
        for step in [0, 10, 1, 2] {
            let d = fixture();
            three_stashes(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let original = s.snapshot().unwrap();
            kill_stash(d.path(), "drop", step);
            let a = s.snapshot().unwrap();
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert_eq!(b.stashes.len(), 2);
            assert_eq!(b.stashes[0].oid, original.stashes[1].oid);
            assert_eq!(b.stashes[1].oid, original.stashes[2].oid);
            assert!(b.changes.is_empty());
        }
    }
    #[test]
    fn stash_recovery_can_resume_after_second_actual_kill() {
        for (kind, crash_step) in [("save", 4), ("apply", 3), ("drop", 1)] {
            for step in 0..=2 {
                let d = fixture();
                if kind == "save" {
                    dirty_three_areas(d.path());
                } else {
                    stash_three_areas(d.path());
                }
                kill_stash(d.path(), kind, crash_step);
                kill_stash(d.path(), &format!("{kind}-recover"), step);
                let mut s = Session::new(d.path().into()).unwrap();
                let a = s.snapshot().unwrap();
                let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
                assert!(!b.recovery_pending);
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    if kind == "save" {
                        "worktree version\n"
                    } else {
                        "hello\n"
                    }
                );
                assert_eq!(d.path().join("untracked.txt").exists(), kind == "save");
                if kind == "drop" {
                    assert!(b.stashes.is_empty());
                }
            }
        }
    }
    #[test]
    fn stash_recovery_rejects_forged_and_duplicate_lock_paths_before_unlink() {
        for duplicate in [false, true] {
            let d = fixture();
            dirty_three_areas(d.path());
            kill_stash(d.path(), "save", 0);
            let path = d.path().join(".git/branchdesk-stash-save/locks.json");
            let mut markers: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let mut extra = markers[0].clone();
            if !duplicate {
                extra["path"] = serde_json::json!("../protected.txt");
            }
            markers.as_array_mut().unwrap().push(extra);
            fs::write(path, serde_json::to_vec(&markers).unwrap()).unwrap();
            fs::write(d.path().join("protected.txt"), "preserve").unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            assert!(s
                .mutate(&a.revision, Action::RecoverCheckout)
                .unwrap_err()
                .contains("INVALID_LOCK_RECORD"));
            assert!(d.path().join(".git/index.lock").exists());
            assert_eq!(
                fs::read_to_string(d.path().join("protected.txt")).unwrap(),
                "preserve"
            );
        }
    }
    #[test]
    fn stash_recovery_does_not_remove_replaced_foreign_log_lock() {
        let d = fixture();
        stash_three_areas(d.path());
        kill_stash(d.path(), "drop", 10);
        let path = d.path().join(".git/logs/refs/stash.lock");
        let bytes = fs::read(&path).unwrap();
        fs::rename(&path, d.path().join(".git/original-log-lock")).unwrap();
        fs::write(&path, &bytes).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(&a.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("EXTERNAL_LOCK"));
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(d.path().join(".git/index.lock").exists());
        assert!(d.path().join(".git/refs/stash.lock").exists());
    }
    fn suspended_checkout(root: &Path, step: usize) -> CrashChild {
        suspended_operation(root, step, false)
    }
    fn suspended_operation(root: &Path, step: usize, recovery: bool) -> CrashChild {
        let ready = root.join(".git/branchdesk-checkout/child.ready");
        if ready.exists() {
            fs::remove_file(ready).unwrap();
        }
        let mut child = CrashChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "repository::tests::checkout_crash_worker",
                    "--nocapture",
                ])
                .env("BRANCHDESK_CRASH_ROOT", root)
                .env(
                    if recovery {
                        "BRANCHDESK_RECOVERY_STEP"
                    } else {
                        "BRANCHDESK_CRASH_STEP"
                    },
                    step.to_string(),
                )
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let start = std::time::Instant::now();
        while !root.join(".git/branchdesk-checkout/child.ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "checkout child exited before checkpoint"
            );
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "child readiness timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child
    }
    #[test]
    fn real_process_kill_releases_lease_and_recovers_owned_git_locks() {
        for step in [0, 1, 2, 3, 4, 100, 101, 102] {
            let d = fixture();
            checkout_target(d.path());
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let mut child = suspended_checkout(d.path(), step);
            assert!(
                lease::OperationLease::acquire(d.path()).is_err(),
                "live writer must keep kernel lease"
            );
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            assert!(
                d.path().join(".git/index.lock").exists(),
                "abrupt exit must leave Git marker"
            );
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            assert!(a.recovery_pending);
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert!(b.changes.is_empty());
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("new.txt").exists());
            assert!(!d.path().join(".git/index.lock").exists());
        }
    }
    #[test]
    fn interrupted_preparation_preserves_subsequent_edits() {
        for edit_index in [false, true] {
            let d = fixture();
            checkout_target(d.path());
            let mut child = suspended_checkout(d.path(), 101);
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let path = d.path().join(if edit_index {
                ".git/index"
            } else {
                "hello.txt"
            });
            let bytes = if edit_index {
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(b"external edit");
                bytes
            } else {
                b"user edit after interruption".to_vec()
            };
            fs::write(&path, &bytes).unwrap();
            // Recovery itself must reject without overwriting the new bytes.
            let root = d.path().canonicalize().unwrap();
            let lease = lease::OperationLease::acquire(&root).unwrap();
            checkout::release_owned_locks(&root, &lease).unwrap();
            let repo = Repository::open(&root).unwrap();
            let mut lock = IndexLock::acquire(&root).unwrap();
            assert!(checkout::recover(&root, &repo, &mut lock)
                .unwrap_err()
                .contains("RECOVERY_CONFLICT"));
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert!(checkout::pending(&root));
        }
    }
    #[test]
    fn recovery_can_resume_after_a_second_process_kill() {
        for step in 0..=4 {
            let d = fixture();
            checkout_target(d.path());
            let mut writer = suspended_checkout(d.path(), 3);
            writer.0.kill().unwrap();
            writer.0.wait().unwrap();
            let mut recovery = suspended_operation(d.path(), step, true);
            assert!(lease::OperationLease::acquire(d.path()).is_err());
            recovery.0.kill().unwrap();
            recovery.0.wait().unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s.mutate(&a.revision, Action::RecoverCheckout).unwrap();
            assert!(!b.recovery_pending);
            assert!(b.changes.is_empty());
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("new.txt").exists());
        }
    }
    #[test]
    fn replaced_foreign_lock_is_not_removed_even_with_identical_bytes() {
        let d = fixture();
        checkout_target(d.path());
        let mut child = suspended_checkout(d.path(), 1);
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let path = d.path().join(".git/index.lock");
        let bytes = fs::read(&path).unwrap();
        // Keep the old inode allocated so the replacement cannot reuse its identity.
        fs::rename(&path, d.path().join(".git/old-owned-index-lock")).unwrap();
        fs::write(&path, &bytes).unwrap();
        let foreign_identity = lease::identity(&path).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(&a.revision, Action::RecoverCheckout)
            .unwrap_err()
            .starts_with("EXTERNAL_LOCK"));
        assert_eq!(lease::identity(&path).unwrap(), foreign_identity);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(
            d.path().join(".git/HEAD.lock").exists(),
            "all locks must validate before any removal"
        );
    }
    #[test]
    fn checkout_switches_clean_branch_and_preserves_git_state() {
        let d = fixture();
        let target = checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let b = s
            .mutate(
                &a.revision,
                Action::Checkout {
                    name: "feature/switch".into(),
                },
            )
            .unwrap();
        assert_eq!(b.branch, "feature/switch");
        assert_eq!(b.commits[0].oid, target);
        assert!(b.changes.is_empty());
        assert!(!b.recovery_pending);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "target\n"
        );
        assert_eq!(
            fs::read_to_string(d.path().join("new.txt")).unwrap(),
            "new\n"
        );
        let c = s
            .mutate(
                &b.revision,
                Action::Checkout {
                    name: a.branch.clone(),
                },
            )
            .unwrap();
        assert_eq!(c.branch, a.branch);
        assert!(c.changes.is_empty());
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "hello\n"
        );
        assert!(!d.path().join("new.txt").exists());
    }
    pub(super) fn checkout_target(root: &Path) -> String {
        let r = Repository::open(root).unwrap();
        let parent = r.head().unwrap().peel_to_commit().unwrap();
        let mut i = r.index().unwrap();
        let mut entry = i.get_path(Path::new("hello.txt"), 0).unwrap();
        entry.id = r.blob(b"target\n").unwrap();
        entry.file_size = 7;
        i.add(&entry).unwrap();
        entry.path = b"new.txt".to_vec();
        entry.id = r.blob(b"new\n").unwrap();
        entry.file_size = 4;
        i.add(&entry).unwrap();
        let tree = r.find_tree(i.write_tree_to(&r).unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let oid = r
            .commit(None, &sig, &sig, "Target branch", &tree, &[&parent])
            .unwrap();
        r.branch("feature/switch", &r.find_commit(oid).unwrap(), false)
            .unwrap();
        oid.to_string()
    }
    fn diverged_merge_fixture(root: &Path, conflict: bool) -> (Oid, Oid) {
        let target = Oid::from_str(&checkout_target(root)).unwrap();
        let repo = open(root).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let path = if conflict { "hello.txt" } else { "ours.txt" };
        fs::write(root.join(path), b"ours\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let ours = repo
            .commit(Some("HEAD"), &sig, &sig, "Our branch", &tree, &[&parent])
            .unwrap();
        (ours, target)
    }
    #[test]
    fn merge_commit_preserves_both_parents_and_recovers_write_boundaries() {
        for step in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            let d = fixture();
            let (ours, theirs) = diverged_merge_fixture(d.path(), false);
            let mut session = Session::new(d.path().into()).unwrap();
            let before = session.snapshot().unwrap();
            let original_index = fs::read(d.path().join(".git/index")).unwrap();
            let repo = open(d.path()).unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            let result = checkout::merge_commit(
                &mut session,
                &repo,
                &before.revision,
                &checkout::MergeInput {
                    name: "feature/switch",
                    message: "Merge topic",
                    author: "Merger",
                    email: "merge@example.invalid",
                },
                &mut lock,
                step,
            );
            drop(lock);
            if step.is_some() {
                assert!(result.unwrap_err().contains("INJECTED"));
                let pending = session.snapshot().unwrap();
                session
                    .mutate(&pending.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(repo.head().unwrap().target(), Some(ours));
                assert_eq!(
                    fs::read(d.path().join(".git/index")).unwrap(),
                    original_index
                );
                assert!(!d.path().join("new.txt").exists());
            } else {
                result.unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(commit.parent_count(), 2);
                assert_eq!(commit.parent_id(0).unwrap(), ours);
                assert_eq!(commit.parent_id(1).unwrap(), theirs);
                assert_eq!(commit.message().unwrap(), "Merge topic");
                assert_eq!(commit.author().name().unwrap(), "Merger");
                assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"target\n");
            }
            assert_eq!(fs::read(d.path().join("ours.txt")).unwrap(), b"ours\n");
            let after = session.snapshot().unwrap();
            assert_eq!(after.branch, before.branch);
            assert!(after.changes.is_empty());
            assert!(!after.recovery_pending);
            assert_eq!(
                repo.refname_to_id("refs/heads/feature/switch").unwrap(),
                theirs
            );
        }
    }
    #[test]
    fn merge_commit_publishes_auto_merged_blob_through_action() {
        let d = fixture();
        let repo = open(d.path()).unwrap();
        let original = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let make = |text: &[u8], parent: &git2::Commit<'_>| {
            let mut index = git2::Index::new().unwrap();
            index.read_tree(&parent.tree().unwrap()).unwrap();
            let mut entry = index.get_path(Path::new("hello.txt"), 0).unwrap();
            entry.id = repo.blob(text).unwrap();
            entry.file_size = text.len() as u32;
            index.add(&entry).unwrap();
            let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
            repo.commit(None, &sig, &sig, "Test", &tree, &[parent])
                .unwrap()
        };
        let base = repo
            .find_commit(make(b"first\nsecond\nthird\n", &original))
            .unwrap();
        let ours = make(b"FIRST\nsecond\nthird\n", &base);
        let theirs = make(b"first\nsecond\nTHIRD\n", &base);
        let current = repo.head().unwrap().name().unwrap().to_string();
        repo.reference(&current, ours, true, "fixture").unwrap();
        repo.reference("refs/heads/topic", theirs, false, "fixture")
            .unwrap();
        fs::write(d.path().join("hello.txt"), b"FIRST\nsecond\nthird\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .read_tree(&repo.find_commit(ours).unwrap().tree().unwrap())
            .unwrap();
        index.write().unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let before = session.snapshot().unwrap();
        assert!(before.changes.is_empty());
        let after = session
            .mutate(
                &before.revision,
                Action::MergeCommit {
                    name: "topic".into(),
                    message: "Combine edits".into(),
                    author: "Merger".into(),
                    email: "merge@example.invalid".into(),
                },
            )
            .unwrap();
        assert!(after.changes.is_empty());
        assert_eq!(
            fs::read(d.path().join("hello.txt")).unwrap(),
            b"FIRST\nsecond\nTHIRD\n"
        );
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(commit.parent_id(0).unwrap(), ours);
        assert_eq!(commit.parent_id(1).unwrap(), theirs);
        let entry = commit
            .tree()
            .unwrap()
            .get_path(Path::new("hello.txt"))
            .unwrap();
        assert_eq!(
            repo.find_blob(entry.id()).unwrap().content(),
            b"FIRST\nsecond\nTHIRD\n"
        );
    }
    #[test]
    fn merge_draft_survives_restart_and_continues_all_resolution_kinds() {
        for (resolution, expected) in [
            (merge_draft::Resolution::Ours {}, Some("ours\n")),
            (merge_draft::Resolution::Theirs {}, Some("target\n")),
            (
                merge_draft::Resolution::Text {
                    text: "combined\n".into(),
                },
                Some("combined\n"),
            ),
            (merge_draft::Resolution::Delete {}, None),
        ] {
            let d = fixture();
            let (ours, theirs) = diverged_merge_fixture(d.path(), true);
            let mut session = Session::new(d.path().into()).unwrap();
            let before = session.snapshot().unwrap();
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let started = session
                .mutate(
                    &before.revision,
                    Action::StartMerge {
                        name: "feature/switch".into(),
                    },
                )
                .unwrap();
            assert!(started.merge_draft.is_some());
            assert!(session
                .mutate(
                    &started.revision,
                    Action::ContinueMerge {
                        message: "Merge".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into()
                    }
                )
                .unwrap_err()
                .contains("UNRESOLVED"));
            let saved = session
                .mutate(
                    &started.revision,
                    Action::ResolveMerge {
                        file_id: file_id("hello.txt"),
                        resolution,
                    },
                )
                .unwrap();
            assert_ne!(started.revision, saved.revision);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"ours\n");
            assert!(session
                .mutate(&started.revision, Action::AbortMerge)
                .is_err());
            let mut session = Session::new(d.path().into()).unwrap();
            let resumed = session.snapshot().unwrap();
            assert_eq!(resumed.merge_draft.unwrap().resolutions.len(), 1);
            let after = session
                .mutate(
                    &resumed.revision,
                    Action::ContinueMerge {
                        message: "Resolved".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into(),
                    },
                )
                .unwrap();
            assert!(after.merge_draft.is_none());
            assert!(after.changes.is_empty());
            let repo = open(d.path()).unwrap();
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(commit.parent_id(0).unwrap(), ours);
            assert_eq!(commit.parent_id(1).unwrap(), theirs);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt"))
                    .ok()
                    .as_deref(),
                expected
            );
        }
    }
    #[test]
    fn merge_draft_interrupted_continue_restores_and_retains_resolutions() {
        for step in 0..=4 {
            let d = fixture();
            let (ours, _) = diverged_merge_fixture(d.path(), true);
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let s = session
                .mutate(
                    &s.revision,
                    Action::StartMerge {
                        name: "feature/switch".into(),
                    },
                )
                .unwrap();
            let s = session
                .mutate(
                    &s.revision,
                    Action::ResolveMerge {
                        file_id: file_id("hello.txt"),
                        resolution: merge_draft::Resolution::Text {
                            text: "resolved\n".into(),
                        },
                    },
                )
                .unwrap();
            let repo = open(d.path()).unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            assert!(checkout::merge_commit(
                &mut session,
                &repo,
                &s.revision,
                &checkout::MergeInput {
                    name: "feature/switch",
                    message: "Merge",
                    author: "A",
                    email: "a@example.invalid"
                },
                &mut lock,
                Some(step)
            )
            .unwrap_err()
            .contains("INJECTED"));
            drop(lock);
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let s = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(repo.head().unwrap().target(), Some(ours));
            assert_eq!(s.merge_draft.as_ref().unwrap().resolutions.len(), 1);
            let s = session
                .mutate(
                    &s.revision,
                    Action::ContinueMerge {
                        message: "Merge".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into(),
                    },
                )
                .unwrap();
            assert!(s.merge_draft.is_none());
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "resolved\n"
            );
        }
    }
    pub(super) fn kill_draft(root: &Path, action: &str, checkpoint: &str) {
        let recovery_step = checkpoint.strip_prefix("recovery:");
        let checkout_step = checkpoint.strip_prefix("checkout:");
        let ready = root.join(if checkout_step.is_some() || recovery_step.is_some() {
            ".git/branchdesk-checkout/child.ready"
        } else {
            ".git/branchdesk-draft-test.ready"
        });
        if ready.exists() {
            fs::remove_file(&ready).unwrap();
        }
        let mut child = CrashChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "repository::tests::checkout_crash_worker"])
                .env("BRANCHDESK_DRAFT_ROOT", root)
                .env("BRANCHDESK_DRAFT_ACTION", action)
                .env("BRANCHDESK_DRAFT_CHECKPOINT", checkpoint)
                .env("BRANCHDESK_CRASH_ROOT", root)
                .env("BRANCHDESK_CRASH_STEP", checkout_step.unwrap_or("none"))
                .env("BRANCHDESK_RECOVERY_STEP", recovery_step.unwrap_or("none"))
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready.exists() {
            assert!(child.0.try_wait().unwrap().is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(lease::OperationLease::acquire(root).is_err());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(
            (checkpoint.starts_with("commit:")
                || checkpoint.starts_with("finish:")
                || checkpoint.starts_with("abort:")
                || checkpoint.starts_with("index:"))
                || checkout_step.is_some()
                || recovery_step.is_some()
                || !root.join(".git/index.lock").exists(),
            "draft-only/finished operation left an index marker"
        );
    }
    #[test]
    fn merge_draft_actual_kill_keeps_old_or_new_complete_resolution() {
        for action in ["start", "resolve"] {
            for step in 0..=3 {
                let d = fixture();
                diverged_merge_fixture(d.path(), true);
                let mut session = Session::new(d.path().into()).unwrap();
                let s = session.snapshot().unwrap();
                if action == "resolve" {
                    let s = session
                        .mutate(
                            &s.revision,
                            Action::StartMerge {
                                name: "feature/switch".into(),
                            },
                        )
                        .unwrap();
                    session
                        .mutate(
                            &s.revision,
                            Action::ResolveMerge {
                                file_id: file_id("hello.txt"),
                                resolution: merge_draft::Resolution::Ours {},
                            },
                        )
                        .unwrap();
                }
                let index = fs::read(d.path().join(".git/index")).unwrap();
                let head = open(d.path()).unwrap().head().unwrap().target();
                kill_draft(d.path(), action, &format!("write:{step}"));
                let mut session = Session::new(d.path().into()).unwrap();
                let s = session.snapshot().unwrap();
                if action == "start" {
                    assert_eq!(s.merge_draft.is_some(), step >= 2);
                } else {
                    let resolution = s
                        .merge_draft
                        .as_ref()
                        .unwrap()
                        .resolutions
                        .get(&file_id("hello.txt"))
                        .unwrap();
                    assert_eq!(
                        matches!(resolution, merge_draft::Resolution::Text { .. }),
                        step >= 2
                    );
                }
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert_eq!(open(d.path()).unwrap().head().unwrap().target(), head);
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "ours\n"
                );
                if s.merge_draft.is_some() {
                    session.mutate(&s.revision, Action::AbortMerge).unwrap();
                }
            }
        }
    }
    #[test]
    fn merge_draft_actual_kill_archival_never_repeats_completed_commit() {
        for action in ["abort", "continue"] {
            for step in 0..=2 {
                let d = fixture();
                let (ours, theirs) = diverged_merge_fixture(d.path(), true);
                let mut session = Session::new(d.path().into()).unwrap();
                let s = session.snapshot().unwrap();
                let s = session
                    .mutate(
                        &s.revision,
                        Action::StartMerge {
                            name: "feature/switch".into(),
                        },
                    )
                    .unwrap();
                session
                    .mutate(
                        &s.revision,
                        Action::ResolveMerge {
                            file_id: file_id("hello.txt"),
                            resolution: merge_draft::Resolution::Text {
                                text: "resolved\n".into(),
                            },
                        },
                    )
                    .unwrap();
                kill_draft(d.path(), action, &format!("archive:{step}"));
                let mut session = Session::new(d.path().into()).unwrap();
                let s = session.snapshot().unwrap();
                assert_eq!(s.merge_draft.is_some(), step == 0);
                let repo = open(d.path()).unwrap();
                let commit = repo.head().unwrap().peel_to_commit().unwrap();
                if action == "continue" {
                    assert_eq!(commit.parent_id(0).unwrap(), ours);
                    assert_eq!(commit.parent_id(1).unwrap(), theirs);
                    assert_eq!(
                        fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                        "resolved\n"
                    );
                    assert!(session
                        .mutate(
                            &s.revision,
                            Action::ContinueMerge {
                                message: "Again".into(),
                                author: "A".into(),
                                email: "a@example.invalid".into()
                            }
                        )
                        .is_err());
                    assert_eq!(repo.head().unwrap().target(), Some(commit.id()));
                } else {
                    assert_eq!(commit.id(), ours);
                }
                if step == 0 {
                    session.mutate(&s.revision, Action::AbortMerge).unwrap();
                }
            }
        }
    }
    #[test]
    fn merge_draft_actual_kill_during_continue_recovers_and_retries() {
        for step in [0, 1, 2, 3, 4, 100, 101, 102] {
            let d = fixture();
            let (ours, theirs) = diverged_merge_fixture(d.path(), true);
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let s = session
                .mutate(
                    &s.revision,
                    Action::StartMerge {
                        name: "feature/switch".into(),
                    },
                )
                .unwrap();
            session
                .mutate(
                    &s.revision,
                    Action::ResolveMerge {
                        file_id: file_id("hello.txt"),
                        resolution: merge_draft::Resolution::Text {
                            text: "resolved\n".into(),
                        },
                    },
                )
                .unwrap();
            kill_draft(d.path(), "continue", &format!("checkout:{step}"));
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let s = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(open(d.path()).unwrap().head().unwrap().target(), Some(ours));
            assert_eq!(s.merge_draft.as_ref().unwrap().resolutions.len(), 1);
            let after = session
                .mutate(
                    &s.revision,
                    Action::ContinueMerge {
                        message: "Retry".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into(),
                    },
                )
                .unwrap();
            assert!(after.merge_draft.is_none());
            assert!(!after.recovery_pending);
            let repo = open(d.path()).unwrap();
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(commit.parent_id(0).unwrap(), ours);
            assert_eq!(commit.parent_id(1).unwrap(), theirs);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "resolved\n"
            );
        }
    }
    #[test]
    fn merge_draft_recovery_survives_a_second_actual_kill() {
        for step in 0..=4 {
            let d = fixture();
            let (ours, theirs) = diverged_merge_fixture(d.path(), true);
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let s = session
                .mutate(
                    &s.revision,
                    Action::StartMerge {
                        name: "feature/switch".into(),
                    },
                )
                .unwrap();
            session
                .mutate(
                    &s.revision,
                    Action::ResolveMerge {
                        file_id: file_id("hello.txt"),
                        resolution: merge_draft::Resolution::Text {
                            text: "resolved\n".into(),
                        },
                    },
                )
                .unwrap();
            kill_draft(d.path(), "continue", "checkout:4");
            kill_draft(d.path(), "recover", &format!("recovery:{step}"));
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let s = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert_eq!(open(d.path()).unwrap().head().unwrap().target(), Some(ours));
            assert_eq!(s.merge_draft.as_ref().unwrap().resolutions.len(), 1);
            let after = session
                .mutate(
                    &s.revision,
                    Action::ContinueMerge {
                        message: "Retry".into(),
                        author: "A".into(),
                        email: "a@example.invalid".into(),
                    },
                )
                .unwrap();
            assert!(after.merge_draft.is_none());
            assert!(!after.recovery_pending);
            let repo = open(d.path()).unwrap();
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(commit.parent_id(0).unwrap(), ours);
            assert_eq!(commit.parent_id(1).unwrap(), theirs);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "resolved\n"
            );
        }
    }
    fn external_conflict_fixture(root: &Path) {
        let (_, theirs) = diverged_merge_fixture(root, true);
        let repo = open(root).unwrap();
        let annotated = repo.find_annotated_commit(theirs).unwrap();
        repo.merge(&[&annotated], None, None).unwrap();
        assert!(repo.index().unwrap().has_conflicts());
    }
    #[test]
    fn external_merge_resolved_file_staging_preserves_merge_metadata_and_worktree() {
        for delete in [false, true] {
            let d = fixture();
            external_conflict_fixture(d.path());
            let repo = open(d.path()).unwrap();
            let head = repo.head().unwrap().target();
            let merge_head = fs::read(d.path().join(".git/MERGE_HEAD")).unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert_eq!(s.external_merge.as_ref().unwrap().conflicts.len(), 1);
            for (side, expected) in [
                (content::MergeSide::Ours, "ours\n"),
                (content::MergeSide::Theirs, "target\n"),
            ] {
                let c = content::read(
                    &mut session,
                    &s.revision,
                    &file_id("hello.txt"),
                    ContentSource::IndexConflict { side },
                )
                .unwrap();
                assert_eq!(c.text, expected);
            }
            if delete {
                fs::remove_file(d.path().join("hello.txt")).unwrap();
            } else {
                fs::write(d.path().join("hello.txt"), "resolved externally\n").unwrap();
            }
            let s = session.snapshot().unwrap();
            let after = session
                .mutate(
                    &s.revision,
                    Action::StageExternalResolution {
                        file_id: file_id("hello.txt"),
                    },
                )
                .unwrap();
            assert!(after.external_merge.unwrap().conflicts.is_empty());
            let index = open(d.path()).unwrap().index().unwrap();
            assert!(!index.has_conflicts());
            let entry = index.get_path(Path::new("hello.txt"), 0);
            if delete {
                assert!(entry.is_none());
                assert!(!d.path().join("hello.txt").exists());
            } else {
                assert_eq!(
                    repo.find_blob(entry.unwrap().id).unwrap().content(),
                    b"resolved externally\n"
                );
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "resolved externally\n"
                );
            }
            assert_eq!(
                fs::read(d.path().join(".git/MERGE_HEAD")).unwrap(),
                merge_head
            );
            assert_eq!(repo.head().unwrap().target(), head);
        }
    }
    #[test]
    fn external_merge_stage_preserves_other_conflicts_and_rejects_stale_metadata() {
        let d = fixture();
        external_conflict_fixture(d.path());
        let repo = open(d.path()).unwrap();
        let mut index = repo.index().unwrap();
        for stage in 1..=3 {
            let mut entry = index.get_path(Path::new("hello.txt"), stage).unwrap();
            entry.path = b"other.txt".to_vec();
            index.add(&entry).unwrap();
        }
        index.write().unwrap();
        fs::write(d.path().join("other.txt"), "other conflict\n").unwrap();
        fs::write(d.path().join("hello.txt"), b"binary\0resolution").unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/index")).unwrap();
        assert!(session
            .mutate(
                &s.revision,
                Action::StageExternalResolution {
                    file_id: "wrong".into()
                }
            )
            .is_err());
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
        fs::write(d.path().join(".git/MERGE_MSG"), "external message change").unwrap();
        assert!(session
            .mutate(
                &s.revision,
                Action::StageExternalResolution {
                    file_id: file_id("hello.txt")
                }
            )
            .is_err());
        let s = session.snapshot().unwrap();
        let s = session
            .mutate(
                &s.revision,
                Action::StageExternalResolution {
                    file_id: file_id("hello.txt"),
                },
            )
            .unwrap();
        assert_eq!(s.external_merge.unwrap().conflicts.len(), 1);
        let index = open(d.path()).unwrap().index().unwrap();
        assert!(index.get_path(Path::new("other.txt"), 2).is_some());
        let entry = index.get_path(Path::new("hello.txt"), 0).unwrap();
        assert_eq!(
            repo.find_blob(entry.id).unwrap().content(),
            b"binary\0resolution"
        );
    }
    #[test]
    fn external_merge_index_recovery_preserves_foreign_merge_metadata() {
        let d = fixture();
        external_conflict_fixture(d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let repo = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        external_merge::stage(
            &mut session,
            &repo,
            &s.revision,
            &file_id("hello.txt"),
            &mut lock,
            Some(1),
        )
        .unwrap_err();
        drop(lock);
        let index = fs::read(d.path().join(".git/index")).unwrap();
        fs::write(d.path().join(".git/MERGE_MSG"), "external message change").unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .mutate(&s.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read_to_string(d.path().join(".git/MERGE_MSG")).unwrap(),
            "external message change"
        );
    }
    fn resolved_external_fixture(root: &Path) -> (Oid, Oid) {
        external_conflict_fixture(root);
        let repo = open(root).unwrap();
        let old = repo.head().unwrap().target().unwrap();
        let incoming = Oid::from_str(
            fs::read_to_string(root.join(".git/MERGE_HEAD"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        fs::write(root.join("hello.txt"), "resolved externally\n").unwrap();
        let mut session = Session::new(root.into()).unwrap();
        let s = session.snapshot().unwrap();
        session
            .mutate(
                &s.revision,
                Action::StageExternalResolution {
                    file_id: file_id("hello.txt"),
                },
            )
            .unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        repo.reference("AUTO_MERGE", tree.id(), true, "fixture")
            .unwrap();
        (old, incoming)
    }
    #[test]
    fn external_merge_continue_commits_index_preserves_dirty_files_and_orig_head() {
        let d = fixture();
        let (old, incoming) = resolved_external_fixture(d.path());
        let orig = fs::read(d.path().join(".git/ORIG_HEAD")).ok();
        fs::write(d.path().join("hello.txt"), "new unstaged edit\n").unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let after = session
            .mutate(
                &s.revision,
                Action::ContinueExternalMerge {
                    message: "Complete external merge".into(),
                    author: "Merger".into(),
                    email: "merge@example.invalid".into(),
                },
            )
            .unwrap();
        assert!(after.external_merge.is_none());
        assert!(!after.recovery_pending);
        let repo = open(d.path()).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(commit.parent_id(0).unwrap(), old);
        assert_eq!(commit.parent_id(1).unwrap(), incoming);
        assert_eq!(commit.message().unwrap(), "Complete external merge\n");
        assert_eq!(commit.author().name().unwrap(), "Merger");
        let blob = repo
            .find_blob(
                commit
                    .tree()
                    .unwrap()
                    .get_path(Path::new("hello.txt"))
                    .unwrap()
                    .id(),
            )
            .unwrap();
        assert_eq!(blob.content(), b"resolved externally\n");
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "new unstaged edit\n"
        );
        for name in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE"] {
            assert!(!d.path().join(".git").join(name).exists());
        }
        assert!(repo.find_reference("AUTO_MERGE").is_err());
        assert_eq!(fs::read(d.path().join(".git/ORIG_HEAD")).ok(), orig);
    }
    #[test]
    fn external_merge_continue_interruptions_roll_forward_without_duplicate_commit() {
        for step in 0..=4 {
            let d = fixture();
            let (old, incoming) = resolved_external_fixture(d.path());
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let repo = open(d.path()).unwrap();
            let lock = IndexLock::acquire(d.path()).unwrap();
            assert!(merge_finish::execute(
                &mut session,
                &repo,
                &s.revision,
                &checkout::MergeInput {
                    name: "",
                    message: "Complete",
                    author: "A",
                    email: "a@example.invalid"
                },
                Some(step)
            )
            .unwrap_err()
            .contains("INJECTED"));
            drop(lock);
            let record: serde_json::Value = serde_json::from_slice(
                &fs::read(d.path().join(".git/branchdesk-merge-finish/record.json")).unwrap(),
            )
            .unwrap();
            let expected = record["new"].as_str().unwrap();
            fs::write(d.path().join("hello.txt"), "later edit\n").unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let after = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!after.recovery_pending);
            assert!(after.external_merge.is_none());
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(commit.id().to_string(), expected);
            assert_eq!(commit.parent_id(0).unwrap(), old);
            assert_eq!(commit.parent_id(1).unwrap(), incoming);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "later edit\n"
            );
        }
    }
    #[test]
    fn external_merge_continue_actual_kill_finishes_recorded_commit() {
        for step in 0..=4 {
            let d = fixture();
            let (old, incoming) = resolved_external_fixture(d.path());
            let index = fs::read(d.path().join(".git/index")).unwrap();
            kill_draft(d.path(), "externalContinue", &format!("finish:{step}"));
            let record: serde_json::Value = serde_json::from_slice(
                &fs::read(d.path().join(".git/branchdesk-merge-finish/record.json")).unwrap(),
            )
            .unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let after = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!after.recovery_pending);
            assert!(after.external_merge.is_none());
            let repo = open(d.path()).unwrap();
            let commit = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(commit.id().to_string(), record["new"].as_str().unwrap());
            assert_eq!(commit.parent_id(0).unwrap(), old);
            assert_eq!(commit.parent_id(1).unwrap(), incoming);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert!(!d.path().join(".git/index.lock").exists());
            assert!(repo.find_reference("AUTO_MERGE").is_err());
        }
    }
    #[test]
    fn external_merge_continue_recovery_rejects_foreign_metadata_and_unresolved_index() {
        let d = fixture();
        external_conflict_fixture(d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .mutate(
                &s.revision,
                Action::ContinueExternalMerge {
                    message: "No".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into()
                }
            )
            .is_err());
        assert!(!merge_finish::pending(d.path()));
        let d = fixture();
        resolved_external_fixture(d.path());
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let repo = open(d.path()).unwrap();
        let lock = IndexLock::acquire(d.path()).unwrap();
        merge_finish::execute(
            &mut session,
            &repo,
            &s.revision,
            &checkout::MergeInput {
                name: "",
                message: "Complete",
                author: "A",
                email: "a@example.invalid",
            },
            Some(1),
        )
        .unwrap_err();
        drop(lock);
        let head = repo.head().unwrap().target();
        fs::write(d.path().join(".git/MERGE_MSG"), "foreign message").unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .mutate(&s.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(repo.head().unwrap().target(), head);
        assert_eq!(
            fs::read_to_string(d.path().join(".git/MERGE_MSG")).unwrap(),
            "foreign message"
        );
    }
    #[test]
    fn external_merge_abort_restores_head_and_preserves_unrelated_dirty_files() {
        let d = fixture();
        external_conflict_fixture(d.path());
        let repo = open(d.path()).unwrap();
        let head = repo.head().unwrap().target();
        fs::write(d.path().join("ours.txt"), "unrelated local edit\n").unwrap();
        fs::write(d.path().join("untracked.keep"), "keep\n").unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let after = session
            .mutate(&s.revision, Action::AbortExternalMerge)
            .unwrap();
        assert!(after.external_merge.is_none());
        assert!(!after.recovery_pending);
        assert_eq!(repo.head().unwrap().target(), head);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "ours\n"
        );
        assert!(!d.path().join("new.txt").exists());
        assert_eq!(
            fs::read_to_string(d.path().join("ours.txt")).unwrap(),
            "unrelated local edit\n"
        );
        assert_eq!(
            fs::read_to_string(d.path().join("untracked.keep")).unwrap(),
            "keep\n"
        );
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        assert!(!index.has_conflicts());
        assert_eq!(
            index.write_tree_to(&repo).unwrap(),
            repo.head().unwrap().peel_to_tree().unwrap().id()
        );
    }
    #[test]
    fn external_merge_abort_rejects_unstaged_edits_and_autostash_before_writing() {
        for autostash in [false, true] {
            let d = fixture();
            resolved_external_fixture(d.path());
            if autostash {
                fs::write(d.path().join(".git/MERGE_AUTOSTASH"), "preserve").unwrap();
            } else {
                fs::write(d.path().join("hello.txt"), "new edit\n").unwrap();
            }
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let before = fs::read(d.path().join("hello.txt")).unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let error = session
                .mutate(&s.revision, Action::AbortExternalMerge)
                .unwrap_err();
            if autostash {
                let s = session.snapshot().unwrap();
                assert!(session
                    .mutate(
                        &s.revision,
                        Action::ContinueExternalMerge {
                            message: "Complete".into(),
                            author: "A".into(),
                            email: "a@example.invalid".into()
                        }
                    )
                    .unwrap_err()
                    .contains("AUTOSTASH"));
                assert!(!merge_finish::pending(d.path()));
            }
            assert!(
                error.contains(if autostash { "AUTOSTASH" } else { "UNSTAGED" }),
                "{error}"
            );
            assert!(!merge_abort::pending(d.path()));
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), before);
        }
    }
    #[test]
    fn external_merge_abort_interruptions_finish_abort_and_preserve_head() {
        for step in 0..=7 {
            let d = fixture();
            external_conflict_fixture(d.path());
            let repo = open(d.path()).unwrap();
            let head = repo.head().unwrap().target();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            assert!(
                merge_abort::execute(&mut session, &repo, &s.revision, &mut lock, Some(step))
                    .unwrap_err()
                    .contains("INJECTED")
            );
            drop(lock);
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let after = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(after.external_merge.is_none());
            assert!(!after.recovery_pending);
            assert_eq!(repo.head().unwrap().target(), head);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "ours\n"
            );
            assert!(!d.path().join("new.txt").exists());
            assert!(after.changes.is_empty());
        }
    }
    #[test]
    fn external_merge_abort_actual_kill_finishes_abort() {
        for step in 0..=7 {
            let d = fixture();
            external_conflict_fixture(d.path());
            let repo = open(d.path()).unwrap();
            let head = repo.head().unwrap().target();
            kill_draft(d.path(), "externalAbort", &format!("abort:{step}"));
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let after = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(after.external_merge.is_none());
            assert!(!after.recovery_pending);
            assert_eq!(repo.head().unwrap().target(), head);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "ours\n"
            );
            assert!(!d.path().join("new.txt").exists());
            assert!(after.changes.is_empty());
        }
    }
    #[test]
    fn external_merge_abort_recovery_refuses_post_interruption_file_edits() {
        let d = fixture();
        external_conflict_fixture(d.path());
        let repo = open(d.path()).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        merge_abort::execute(&mut session, &repo, &s.revision, &mut lock, Some(1)).unwrap_err();
        drop(lock);
        fs::write(d.path().join("hello.txt"), "later edit").unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let s = session.snapshot().unwrap();
        assert!(session
            .mutate(&s.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "later edit"
        );
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
    }
    #[test]
    fn external_merge_index_interruption_restores_stages_without_overwriting_edits() {
        for step in 0..=1 {
            let d = fixture();
            external_conflict_fixture(d.path());
            fs::write(d.path().join("hello.txt"), "resolved\n").unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let s = session.snapshot().unwrap();
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let repo = open(d.path()).unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            assert!(external_merge::stage(
                &mut session,
                &repo,
                &s.revision,
                &file_id("hello.txt"),
                &mut lock,
                Some(step)
            )
            .unwrap_err()
            .contains("INJECTED"));
            drop(lock);
            fs::write(d.path().join("hello.txt"), "new external edit\n").unwrap();
            let s = session.snapshot().unwrap();
            assert!(s.recovery_pending);
            let after = session
                .mutate(&s.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(!after.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "new external edit\n"
            );
        }
    }
    #[test]
    fn merge_draft_corruption_is_visible_and_abort_preserves_raw_evidence() {
        for bytes in [b"{broken".as_slice(), b"{}", br#"{"version":999,"name":"x","branch":"main","head":"x","target":"y","resolutions":{}}"#] {
            let d=fixture();
            fs::write(d.path().join(".git/branchdesk-merge-draft.json"),bytes).unwrap();
            let index=fs::read(d.path().join(".git/index")).unwrap();
            let file=fs::read(d.path().join("hello.txt")).unwrap();
            let mut session=Session::new(d.path().into()).unwrap();let s=session.snapshot().unwrap();
            assert!(s.merge_draft.is_none());assert!(s.merge_draft_error.is_some());
            assert!(session.mutate(&s.revision,Action::StageAll).unwrap_err().contains("MERGE_IN_PROGRESS"));
            let after=session.mutate(&s.revision,Action::AbortMerge).unwrap();
            assert!(after.merge_draft_error.is_none());
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(),file);
            let archive=fs::read_dir(d.path().join(".git")).unwrap().map(|e|e.unwrap()).find(|e|e.file_name().to_string_lossy().starts_with("branchdesk-merge-draft-completed-")).unwrap();
            assert_eq!(fs::read(archive.path()).unwrap(),bytes);
        }
    }
    #[test]
    fn merge_draft_abort_preserves_external_edits_and_other_writes_are_blocked() {
        let d = fixture();
        diverged_merge_fixture(d.path(), true);
        let mut session = Session::new(d.path().into()).unwrap();
        let s = session.snapshot().unwrap();
        let s = session
            .mutate(
                &s.revision,
                Action::StartMerge {
                    name: "feature/switch".into(),
                },
            )
            .unwrap();
        assert!(session
            .mutate(&s.revision, Action::StageAll)
            .unwrap_err()
            .contains("MERGE_IN_PROGRESS"));
        assert!(session
            .mutate(
                &s.revision,
                Action::ResolveMerge {
                    file_id: "fake".into(),
                    resolution: merge_draft::Resolution::Ours {}
                }
            )
            .is_err());
        assert!(serde_json::from_str::<Action>(
            r#"{"kind":"resolveMerge","file_id":"a","resolution":{"kind":"ours","extra":true}}"#
        )
        .is_err());
        let repo = open(d.path()).unwrap();
        repo.reference(
            "refs/heads/feature/switch",
            repo.head().unwrap().target().unwrap(),
            true,
            "external move",
        )
        .unwrap();
        let changed = session.snapshot().unwrap();
        assert!(session
            .mutate(
                &changed.revision,
                Action::ContinueMerge {
                    message: "Merge".into(),
                    author: "A".into(),
                    email: "a@example.invalid".into()
                }
            )
            .unwrap_err()
            .contains("STALE_MERGE_DRAFT"));
        fs::write(d.path().join("hello.txt"), "external edit").unwrap();
        let s = session.snapshot().unwrap();
        let after = session.mutate(&s.revision, Action::AbortMerge).unwrap();
        assert!(after.merge_draft.is_none());
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "external edit"
        );
    }
    #[test]
    fn merge_commit_conflict_preserves_worktree_index_and_refs() {
        let d = fixture();
        let (ours, theirs) = diverged_merge_fixture(d.path(), true);
        let mut session = Session::new(d.path().into()).unwrap();
        let before = session.snapshot().unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        assert!(session
            .mutate(
                &before.revision,
                Action::MergeCommit {
                    name: "feature/switch".into(),
                    message: "Merge".into(),
                    author: "Merger".into(),
                    email: "merge@example.invalid".into()
                }
            )
            .unwrap_err()
            .contains("MERGE_CONFLICT"));
        let repo = open(d.path()).unwrap();
        assert_eq!(repo.head().unwrap().target(), Some(ours));
        assert_eq!(
            repo.refname_to_id("refs/heads/feature/switch").unwrap(),
            theirs
        );
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"ours\n");
        assert!(!checkout::pending(d.path()));
    }
    #[test]
    fn fast_forward_merge_keeps_branch_and_recovers_every_write_boundary() {
        for step in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            let d = fixture();
            let target = checkout_target(d.path());
            let mut session = Session::new(d.path().into()).unwrap();
            let before = session.snapshot().unwrap();
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let original_head = open(d.path())
                .unwrap()
                .head()
                .unwrap()
                .target()
                .unwrap()
                .to_string();
            if let Some(step) = step {
                let repo = open(d.path()).unwrap();
                let mut lock = IndexLock::acquire(d.path()).unwrap();
                assert!(checkout::fast_forward(
                    &mut session,
                    &repo,
                    &before.revision,
                    "feature/switch",
                    &mut lock,
                    Some(step)
                )
                .unwrap_err()
                .contains("INJECTED"));
                drop(lock);
                let pending = session.snapshot().unwrap();
                assert!(pending.recovery_pending);
                let restored = session
                    .mutate(&pending.revision, Action::RecoverCheckout)
                    .unwrap();
                assert_eq!(restored.branch, before.branch);
                assert!(restored.changes.is_empty());
                assert_eq!(
                    repo.head().unwrap().target().unwrap().to_string(),
                    original_head
                );
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
                assert!(!d.path().join("new.txt").exists());
            } else {
                let after = session
                    .mutate(
                        &before.revision,
                        Action::FastForwardMerge {
                            name: "feature/switch".into(),
                        },
                    )
                    .unwrap();
                assert_eq!(after.branch, before.branch);
                assert!(after.changes.is_empty());
                assert!(!after.recovery_pending);
                let repo = open(d.path()).unwrap();
                assert_eq!(repo.head().unwrap().target().unwrap().to_string(), target);
                assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"target\n");
                assert_eq!(fs::read(d.path().join("new.txt")).unwrap(), b"new\n");
                let again = session
                    .mutate(
                        &after.revision,
                        Action::FastForwardMerge {
                            name: "feature/switch".into(),
                        },
                    )
                    .unwrap();
                assert!(again.changes.is_empty());
            }
        }
    }
    #[test]
    fn fast_forward_rejects_divergence_dirty_and_stale_without_writes() {
        for scenario in ["diverged", "dirty", "stale", "collision"] {
            let d = fixture();
            checkout_target(d.path());
            let mut session = Session::new(d.path().into()).unwrap();
            let old = session.snapshot().unwrap();
            let repo = open(d.path()).unwrap();
            match scenario {
                "diverged" => {
                    let parent = repo.head().unwrap().peel_to_commit().unwrap();
                    let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
                    repo.commit(
                        Some("HEAD"),
                        &sig,
                        &sig,
                        "Diverged",
                        &parent.tree().unwrap(),
                        &[&parent],
                    )
                    .unwrap();
                }
                "collision" => {
                    fs::write(d.path().join(".git/info/exclude"), "new.txt\n").unwrap();
                    fs::write(d.path().join("new.txt"), "ignored personal file").unwrap();
                }
                _ => fs::write(d.path().join("hello.txt"), "local edit").unwrap(),
            }
            let now = session.snapshot().unwrap();
            let revision = if scenario == "stale" {
                &old.revision
            } else {
                &now.revision
            };
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let bytes = fs::read(d.path().join("hello.txt")).unwrap();
            let head = repo.head().unwrap().target();
            assert!(
                session
                    .mutate(
                        revision,
                        Action::FastForwardMerge {
                            name: "feature/switch".into()
                        }
                    )
                    .is_err(),
                "{scenario}"
            );
            assert_eq!(repo.head().unwrap().target(), head);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), bytes);
            assert!(!checkout::pending(d.path()));
            if scenario == "collision" {
                assert_eq!(
                    fs::read(d.path().join("new.txt")).unwrap(),
                    b"ignored personal file"
                );
            }
        }
    }
    #[test]
    fn fast_forward_actual_process_kill_recovers() {
        for step in [0, 1, 2, 3, 4, 100, 101, 102] {
            let d = fixture();
            checkout_target(d.path());
            let repo = open(d.path()).unwrap();
            let original_head = repo.head().unwrap().target().unwrap();
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let mut child = CrashChild(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "repository::tests::checkout_crash_worker"])
                    .env("BRANCHDESK_CRASH_ROOT", d.path())
                    .env("BRANCHDESK_FAST_FORWARD", "1")
                    .env("BRANCHDESK_CRASH_STEP", step.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !d
                .path()
                .join(".git/branchdesk-checkout/child.ready")
                .exists()
            {
                assert!(child.0.try_wait().unwrap().is_none());
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(lease::OperationLease::acquire(d.path()).is_err());
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let pending = session.snapshot().unwrap();
            let restored = session
                .mutate(&pending.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(restored.changes.is_empty());
            assert!(!restored.recovery_pending);
            assert_eq!(
                open(d.path()).unwrap().head().unwrap().target(),
                Some(original_head)
            );
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert!(!d.path().join("new.txt").exists());
        }
    }
    #[test]
    fn merge_commit_actual_process_kill_recovers() {
        for step in [0, 1, 2, 3, 4, 100, 101, 102] {
            let d = fixture();
            diverged_merge_fixture(d.path(), false);
            let repo = open(d.path()).unwrap();
            let original_head = repo.head().unwrap().target().unwrap();
            let index = fs::read(d.path().join(".git/index")).unwrap();
            let mut child = CrashChild(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "repository::tests::checkout_crash_worker"])
                    .env("BRANCHDESK_CRASH_ROOT", d.path())
                    .env("BRANCHDESK_MERGE_COMMIT", "1")
                    .env("BRANCHDESK_CRASH_STEP", step.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !d
                .path()
                .join(".git/branchdesk-checkout/child.ready")
                .exists()
            {
                assert!(child.0.try_wait().unwrap().is_none());
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(lease::OperationLease::acquire(d.path()).is_err());
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            let mut session = Session::new(d.path().into()).unwrap();
            let pending = session.snapshot().unwrap();
            let restored = session
                .mutate(&pending.revision, Action::RecoverCheckout)
                .unwrap();
            assert!(restored.changes.is_empty());
            assert!(!restored.recovery_pending);
            assert_eq!(
                open(d.path()).unwrap().head().unwrap().target(),
                Some(original_head)
            );
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert!(!d.path().join("new.txt").exists());
        }
    }
    #[test]
    fn fast_forward_recovery_preserves_external_branch_move() {
        let d = fixture();
        checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let before = s.snapshot().unwrap();
        let repo = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        checkout::fast_forward(
            &mut s,
            &repo,
            &before.revision,
            "feature/switch",
            &mut lock,
            Some(4),
        )
        .unwrap_err();
        drop(lock);
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Other", "other@example.invalid").unwrap();
        let external = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "External",
                &parent.tree().unwrap(),
                &[&parent],
            )
            .unwrap();
        let snapshot = s.snapshot().unwrap();
        assert!(s
            .mutate(&snapshot.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(repo.head().unwrap().target(), Some(external));
        assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"target\n");
    }
    #[test]
    fn checkout_interruptions_restore_files_index_and_head() {
        for step in 0..=4 {
            let d = fixture();
            checkout_target(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let original_index = fs::read(d.path().join(".git/index")).unwrap();
            let r = open(d.path()).unwrap();
            let mut lock = IndexLock::acquire(d.path()).unwrap();
            assert!(checkout::execute(
                &mut s,
                &r,
                &a.revision,
                "feature/switch",
                &mut lock,
                Some(step)
            )
            .unwrap_err()
            .contains("INJECTED"));
            assert!(lock.owned && lock.path.exists());
            drop(lock);
            let b = s.snapshot().unwrap();
            assert!(b.recovery_pending);
            assert!(b.write_block.is_some());
            assert!(s
                .mutate(
                    &b.revision,
                    Action::CreateBranch {
                        name: "blocked".into()
                    }
                )
                .unwrap_err()
                .starts_with("READ_ONLY"));
            let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
            assert!(!c.recovery_pending);
            assert_eq!(c.branch, a.branch);
            assert!(c.changes.is_empty());
            assert_eq!(
                fs::read(d.path().join(".git/index")).unwrap(),
                original_index
            );
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("new.txt").exists());
        }
    }
    #[test]
    fn recovery_preserves_edits_made_after_interruption() {
        let d = fixture();
        checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        assert!(checkout::execute(
            &mut s,
            &r,
            &a.revision,
            "feature/switch",
            &mut lock,
            Some(1)
        )
        .is_err());
        drop(lock);
        fs::write(d.path().join("hello.txt"), "user edit after failure\n").unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .starts_with("RECOVERY_CONFLICT"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "user edit after failure\n"
        );
        assert!(checkout::pending(d.path()));
    }
    #[test]
    fn recovery_rejects_corrupted_backup_and_manifest_paths() {
        let d = fixture();
        checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        assert!(checkout::execute(
            &mut s,
            &r,
            &a.revision,
            "feature/switch",
            &mut lock,
            Some(1)
        )
        .is_err());
        drop(lock);
        let backup = d.path().join(".git/branchdesk-checkout/0.before");
        fs::write(&backup, "tampered").unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .starts_with("INVALID_BACKUP"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "target\n"
        );
        fs::write(&backup, "hello\n").unwrap();
        let manifest = d.path().join(".git/branchdesk-checkout/manifest.json");
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        data["entries"][0]["path"] = "../outside".into();
        fs::write(&manifest, serde_json::to_vec(&data).unwrap()).unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .starts_with("INVALID_JOURNAL_ENTRIES"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "target\n"
        );
    }
    #[test]
    fn recovery_refuses_external_index_update() {
        let d = fixture();
        checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(d.path()).unwrap();
        let mut lock = IndexLock::acquire(d.path()).unwrap();
        assert!(checkout::execute(
            &mut s,
            &r,
            &a.revision,
            "feature/switch",
            &mut lock,
            Some(1)
        )
        .is_err());
        drop(lock);
        let r = Repository::open(d.path()).unwrap();
        let mut i = r.index().unwrap();
        i.add_path(Path::new("hello.txt")).unwrap();
        i.write().unwrap();
        let external = fs::read(d.path().join(".git/index")).unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .starts_with("RECOVERY_CONFLICT"));
        assert_eq!(external, fs::read(d.path().join(".git/index")).unwrap());
    }
    #[test]
    fn checkout_rejects_target_symlink_before_any_worktree_write() {
        let d = fixture();
        let r = Repository::open(d.path()).unwrap();
        let parent = r.head().unwrap().peel_to_commit().unwrap();
        let mut i = r.index().unwrap();
        let mut e = i.get_path(Path::new("hello.txt"), 0).unwrap();
        e.path = b"escape".to_vec();
        e.mode = 0o120000;
        e.id = r.blob(b"/tmp/outside").unwrap();
        i.add(&e).unwrap();
        let tree = r.find_tree(i.write_tree_to(&r).unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let oid = r
            .commit(None, &sig, &sig, "Unsupported target", &tree, &[&parent])
            .unwrap();
        r.branch("unsafe-target", &r.find_commit(oid).unwrap(), false)
            .unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::Checkout {
                    name: "unsafe-target".into()
                }
            )
            .is_err());
        assert!(!checkout::pending(d.path()));
        assert!(!d.path().join("escape").exists());
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "hello\n"
        );
    }
    #[test]
    fn checkout_refuses_dirty_and_ignored_collisions_before_journaling() {
        let d = fixture();
        checkout_target(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        fs::write(d.path().join("hello.txt"), "dirty").unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::Checkout {
                    name: "feature/switch".into()
                }
            )
            .unwrap_err()
            .starts_with("DIRTY_CHECKOUT"));
        assert!(!checkout::pending(d.path()));
        fs::write(d.path().join("hello.txt"), "hello\n").unwrap();
        fs::write(d.path().join(".git/info/exclude"), "new.txt\n").unwrap();
        fs::write(d.path().join("new.txt"), "ignored but valuable").unwrap();
        let a = s.snapshot().unwrap();
        assert!(a.changes.is_empty());
        assert!(s
            .mutate(
                &a.revision,
                Action::Checkout {
                    name: "feature/switch".into()
                }
            )
            .unwrap_err()
            .starts_with("CHECKOUT_COLLISION"));
        assert!(!checkout::pending(d.path()));
        assert_eq!(
            fs::read_to_string(d.path().join("new.txt")).unwrap(),
            "ignored but valuable"
        );
    }
    #[test]
    fn stash_inspection_separates_index_worktree_and_untracked_without_writes() {
        let d = fixture();
        let mut r = Repository::open(d.path()).unwrap();
        fs::write(d.path().join("hello.txt"), "staged version\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("hello.txt")).unwrap();
        index.write().unwrap();
        fs::write(d.path().join("hello.txt"), "worktree version\n").unwrap();
        fs::write(d.path().join("new[1].txt"), "untracked version\n").unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let oid = r
            .stash_save(
                &sig,
                "Three areas",
                Some(git2::StashFlags::INCLUDE_UNTRACKED),
            )
            .unwrap()
            .to_string();
        fs::write(d.path().join("hello.txt"), "current work\n").unwrap();
        let index_before = fs::read(d.path().join(".git/index")).unwrap();
        let log_before = fs::read(d.path().join(".git/logs/refs/stash")).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert_eq!(a.stashes.len(), 1);
        assert_eq!(a.stashes[0].oid, oid);
        let detail = s.stash_detail(&a.revision, &oid, None, None).unwrap();
        assert!(detail.groups.iter().all(|g| g.files.len() == 1));
        let staged = s
            .stash_detail(
                &a.revision,
                &oid,
                Some(StashArea::Staged),
                Some(&file_id("hello.txt")),
            )
            .unwrap()
            .diff
            .unwrap()
            .text;
        assert!(staged.contains("-hello"));
        assert!(staged.contains("+staged version"));
        let unstaged = s
            .stash_detail(
                &a.revision,
                &oid,
                Some(StashArea::Unstaged),
                Some(&file_id("hello.txt")),
            )
            .unwrap()
            .diff
            .unwrap()
            .text;
        assert!(unstaged.contains("-staged version"));
        assert!(unstaged.contains("+worktree version"));
        assert!(!unstaged.contains("current work"));
        let untracked = s
            .stash_detail(
                &a.revision,
                &oid,
                Some(StashArea::Untracked),
                Some(&file_id("new[1].txt")),
            )
            .unwrap()
            .diff
            .unwrap()
            .text;
        assert!(untracked.contains("+untracked version"));
        assert_eq!(index_before, fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(
            log_before,
            fs::read(d.path().join(".git/logs/refs/stash")).unwrap()
        );
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "current work\n"
        );
        assert!(s
            .stash_detail(
                &a.revision,
                &oid,
                Some(StashArea::Staged),
                Some(&file_id("new[1].txt"))
            )
            .is_err());
        assert!(s
            .stash_detail(&a.revision, &oid, None, Some("unknown"))
            .is_err());
        assert!(s.stash_detail(&a.revision, "HEAD", None, None).is_err());
    }
    fn dirty_three_areas(root: &Path) {
        let r = Repository::open(root).unwrap();
        fs::write(root.join("hello.txt"), "staged version\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("hello.txt")).unwrap();
        index.write().unwrap();
        fs::write(root.join("hello.txt"), "worktree version\n").unwrap();
        fs::write(root.join("untracked.txt"), "untracked version\n").unwrap();
    }
    #[test]
    fn stash_save_roundtrip_preserves_three_areas_and_untracked_option() {
        for include_untracked in [false, true] {
            let d = fixture();
            dirty_three_areas(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s
                .mutate(
                    &a.revision,
                    Action::SaveStash {
                        message: "my snapshot".into(),
                        include_untracked,
                    },
                )
                .unwrap();
            assert_eq!(b.stashes.len(), 1);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert_eq!(d.path().join("untracked.txt").exists(), !include_untracked);
            let r = Repository::open(d.path()).unwrap();
            let stash = r
                .find_commit(Oid::from_str(&b.stashes[0].oid).unwrap())
                .unwrap();
            assert_eq!(stash.parent_count(), if include_untracked { 3 } else { 2 });
            assert_eq!(
                stash
                    .parent(1)
                    .unwrap()
                    .tree()
                    .unwrap()
                    .get_path(Path::new("hello.txt"))
                    .unwrap()
                    .id(),
                r.blob(b"staged version\n").unwrap()
            );
            assert_eq!(
                stash
                    .tree()
                    .unwrap()
                    .get_path(Path::new("hello.txt"))
                    .unwrap()
                    .id(),
                r.blob(b"worktree version\n").unwrap()
            );
            if !include_untracked {
                fs::remove_file(d.path().join("untracked.txt")).unwrap();
            }
            let c = s.snapshot().unwrap();
            let e = s
                .mutate(
                    &c.revision,
                    Action::ApplyStash {
                        oid: b.stashes[0].oid.clone(),
                        restore_index: true,
                    },
                )
                .unwrap();
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(e.stashes.len(), 1);
            assert_eq!(d.path().join("untracked.txt").exists(), include_untracked);
        }
    }
    #[test]
    fn stash_save_interruption_restores_original_files_and_index() {
        for step in 0..=4 {
            let d = fixture();
            dirty_three_areas(d.path());
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            {
                let mut lock = IndexLock::acquire(&s.root).unwrap();
                assert!(stash_save::execute(
                    &mut s,
                    &r,
                    &a.revision,
                    "fault",
                    true,
                    &mut lock,
                    Some(step)
                )
                .unwrap_err()
                .contains("INJECTED"));
            }
            let b = s.snapshot().unwrap();
            assert!(b.recovery_pending);
            let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
            assert!(!c.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(
                fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
                "untracked version\n"
            );
            assert_eq!(c.stashes.len(), if step == 0 { 0 } else { 1 });
        }
    }
    #[test]
    fn stash_save_recovery_preserves_external_file_edit() {
        let d = fixture();
        dirty_three_areas(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(&s.root).unwrap();
        {
            let mut lock = IndexLock::acquire(&s.root).unwrap();
            stash_save::execute(&mut s, &r, &a.revision, "fault", true, &mut lock, Some(2))
                .unwrap_err();
        }
        fs::write(d.path().join("hello.txt"), "later edit").unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "later edit"
        );
        assert!(stash_save::pending(&s.root));
    }
    #[test]
    fn stash_save_handles_deletion_new_binary_and_opposing_index_changes() {
        for scenario in 0..3 {
            let d = fixture();
            let r = Repository::open(d.path()).unwrap();
            match scenario {
                0 => fs::remove_file(d.path().join("hello.txt")).unwrap(),
                1 => {
                    fs::write(d.path().join("binary.bin"), [0, 1, 255, 0, 127]).unwrap();
                    let mut index = r.index().unwrap();
                    index.add_path(Path::new("binary.bin")).unwrap();
                    index.write().unwrap();
                }
                _ => {
                    fs::write(d.path().join("hello.txt"), "index only").unwrap();
                    let mut index = r.index().unwrap();
                    index.add_path(Path::new("hello.txt")).unwrap();
                    index.write().unwrap();
                    fs::write(d.path().join("hello.txt"), "hello\n").unwrap();
                }
            }
            let original_tree = r.index().unwrap().write_tree().unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s
                .mutate(
                    &a.revision,
                    Action::SaveStash {
                        message: "edge case".into(),
                        include_untracked: false,
                    },
                )
                .unwrap();
            assert!(b.changes.is_empty());
            let c = s
                .mutate(
                    &b.revision,
                    Action::PopStash {
                        oid: b.stashes[0].oid.clone(),
                        restore_index: true,
                    },
                )
                .unwrap();
            assert!(c.stashes.is_empty());
            assert_eq!(
                Repository::open(d.path())
                    .unwrap()
                    .index()
                    .unwrap()
                    .write_tree()
                    .unwrap(),
                original_tree
            );
            if scenario == 0 {
                assert!(!d.path().join("hello.txt").exists());
            }
            if scenario == 1 {
                assert_eq!(
                    fs::read(d.path().join("binary.bin")).unwrap(),
                    [0, 1, 255, 0, 127]
                );
            }
            if scenario == 2 {
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "hello\n"
                );
            }
        }
    }
    #[test]
    fn stash_save_appends_without_losing_older_stashes_and_pins_checkpoint() {
        let d = fixture();
        three_stashes(d.path());
        dirty_three_areas(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let b = s
            .mutate(
                &a.revision,
                Action::SaveStash {
                    message: "newest".into(),
                    include_untracked: true,
                },
            )
            .unwrap();
        assert_eq!(b.stashes.len(), 4);
        assert_eq!(
            b.stashes[1..]
                .iter()
                .map(|x| x.oid.clone())
                .collect::<Vec<_>>(),
            a.stashes.iter().map(|x| x.oid.clone()).collect::<Vec<_>>()
        );
        let r = Repository::open(d.path()).unwrap();
        let pins = r
            .references_glob("refs/branchdesk/checkpoints/*")
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(pins.len(), 1);
        let pinned = pins[0].peel_to_commit().unwrap();
        assert_eq!(pinned.parent_id(0).unwrap().to_string(), b.stashes[0].oid);
        assert!(pinned
            .tree()
            .unwrap()
            .get_path(Path::new("untracked.txt"))
            .is_ok());
    }
    #[test]
    fn stash_save_empty_and_invalid_messages_do_not_create_records() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::SaveStash {
                    message: "empty".into(),
                    include_untracked: false
                }
            )
            .unwrap_err()
            .contains("NOTHING_TO_STASH"));
        fs::write(d.path().join("hello.txt"), "change").unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &b.revision,
                Action::SaveStash {
                    message: "bad\nmessage".into(),
                    include_untracked: true
                }
            )
            .unwrap_err()
            .contains("INVALID_STASH_MESSAGE"));
        assert!(!stash_save::pending(&s.root));
        assert!(s.snapshot().unwrap().stashes.is_empty());
    }
    fn stash_three_areas(root: &Path) -> String {
        let mut r = Repository::open(root).unwrap();
        fs::write(root.join("hello.txt"), "staged version\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("hello.txt")).unwrap();
        index.write().unwrap();
        fs::write(root.join("hello.txt"), "worktree version\n").unwrap();
        fs::write(root.join("untracked.txt"), "untracked version\n").unwrap();
        r.stash_save(
            &git2::Signature::now("Test", "test@example.invalid").unwrap(),
            "three areas",
            Some(git2::StashFlags::INCLUDE_UNTRACKED),
        )
        .unwrap()
        .to_string()
    }
    #[test]
    fn stash_pop_restores_three_areas_then_drops_only_selected_entry() {
        for restore_index in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let mut r = Repository::open(d.path()).unwrap();
            fs::write(d.path().join("hello.txt"), "newer stash content").unwrap();
            let newest = r
                .stash_save(
                    &git2::Signature::now("Test", "test@example.invalid").unwrap(),
                    "newer",
                    None,
                )
                .unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let b = s
                .mutate(&a.revision, Action::PopStash { oid, restore_index })
                .unwrap();
            assert_eq!(b.stashes.len(), 1);
            assert_eq!(b.stashes[0].oid, newest.to_string());
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(
                fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
                "untracked version\n"
            );
            let r = Repository::open(d.path()).unwrap();
            let index = r.index().unwrap();
            let blob = r
                .find_blob(index.get_path(Path::new("hello.txt"), 0).unwrap().id)
                .unwrap();
            assert_eq!(
                blob.content(),
                if restore_index {
                    b"staged version\n".as_slice()
                } else {
                    b"hello\n".as_slice()
                }
            );
            assert_eq!(b.commits[0].oid, a.commits[0].oid);
        }
    }
    #[test]
    fn stash_pop_drop_overlap_actual_kills_and_recovery_rekills_preserve_result() {
        let cases=[0,10,1,2].into_iter().map(|step|(step,None))
            .chain([0,10,1,2].into_iter().map(|step|(0,Some(step))));
        for (step,again) in cases {
            let d=fixture();stash_three_areas(d.path());
            let mut child=suspended_stash(d.path(),"drop",step,false,true);
            child.0.kill().unwrap();child.0.wait().unwrap();
            let index=fs::read(d.path().join(".git/index")).unwrap();
            fs::write(d.path().join("later.txt"),b"later user file\n").unwrap();
            if let Some(n)=again {kill_stash(d.path(),"drop-recover",n);}
            let mut session=Session::new(d.path().into()).unwrap();
            let state=session.snapshot().unwrap();assert!(state.recovery_pending);
            let after=session.mutate(&state.revision,Action::RecoverCheckout).unwrap();
            assert!(!after.recovery_pending);assert!(after.stashes.is_empty());
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),index);
            for (name,bytes) in [("hello.txt",b"worktree version\n".as_slice()),("untracked.txt",b"untracked version\n".as_slice()),("later.txt",b"later user file\n".as_slice())] {
                assert_eq!(fs::read(d.path().join(name)).unwrap(),bytes);
            }
            assert!(!d.path().join(".git/index.lock").exists());
        }
    }
    #[test]
    fn stash_pop_completed_receipt_preserves_foreign_lock_and_later_index() {
        for foreign_lock in [false,true] {
            let d=fixture();let oid=stash_three_areas(d.path());
            kill_stash(d.path(),"pop",0);
            let index=fs::read(d.path().join(".git/index")).unwrap();
            let lock=d.path().join(".git/index.lock");
            if foreign_lock {fs::write(&lock,b"foreign lock contents").unwrap();}
            else {
                // Model an independently changed, valid index without changing the owned marker.
                let alternate=d.path().join(".git/test-later-index");
                fs::write(&alternate,&index).unwrap();
                let mut changed=git2::Index::open(&alternate).unwrap();
                changed.remove_path(Path::new("hello.txt")).unwrap();changed.write().unwrap();
                fs::rename(alternate,d.path().join(".git/index")).unwrap();
            }
            let current=fs::read(d.path().join(".git/index")).unwrap();
            let mut session=Session::new(d.path().into()).unwrap();let state=session.snapshot().unwrap();
            let result=session.mutate(&state.revision,Action::RecoverCheckout);
            if foreign_lock {assert!(result.unwrap_err().contains("EXTERNAL_LOCK"));assert_eq!(fs::read(lock).unwrap(),b"foreign lock contents");assert_eq!(current,index);}
            else {let after=result.unwrap();assert!(!after.recovery_pending);assert_eq!(after.stashes[0].oid,oid);}
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(),current);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(),b"worktree version\n");
        }
    }
    #[test]
    fn stash_pop_actual_kill_between_apply_and_drop_preserves_applied_result() {
        for (step, again) in [(0,false),(1,false),(0,true),(1,true)] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let head = open(d.path()).unwrap().head().unwrap().target().unwrap();
            kill_stash(d.path(), "pop", step);
            fs::write(d.path().join("later.txt"), b"later user file\n").unwrap();
            if again { kill_stash(d.path(), "pop-recover", 0); }
            let mut session = Session::new(d.path().into()).unwrap();
            let state = session.snapshot().unwrap();
            assert_eq!(state.stashes.len(), 1);
            assert_eq!(state.stashes[0].oid, oid);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"worktree version\n");
            assert_eq!(fs::read(d.path().join("untracked.txt")).unwrap(), b"untracked version\n");
            let index = fs::read(d.path().join(".git/index")).unwrap();
            assert!(state.recovery_pending);
            let state = session.mutate(&state.revision, Action::RecoverCheckout).unwrap();
            assert!(!state.recovery_pending);
            assert_eq!(open(d.path()).unwrap().head().unwrap().target().unwrap(), head);
            let error = session.mutate(&state.revision, Action::PopStash {oid:oid.clone(),restore_index:true}).unwrap_err();
            assert!(error.contains("DIRTY"), "{error}");
            let current = session.snapshot().unwrap();
            let after = session.mutate(&current.revision, Action::DropStash {oid}).unwrap();
            assert!(after.stashes.is_empty());
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
            assert_eq!(fs::read(d.path().join("hello.txt")).unwrap(), b"worktree version\n");
            assert_eq!(fs::read(d.path().join("later.txt")).unwrap(), b"later user file\n");
            assert!(!d.path().join(".git/index.lock").exists());
        }
    }
    #[test]
    fn stash_pop_interruption_never_reapplies_or_drops_the_next_entry() {
        for step in 0..=3 {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            {
                let mut lock = IndexLock::acquire(&s.root).unwrap();
                assert!(stash_apply::pop(
                    &mut s,
                    &r,
                    &a.revision,
                    &oid,
                    true,
                    &mut lock,
                    Some(step)
                )
                .unwrap_err()
                .contains("POP_APPLIED_DROP_INCOMPLETE"));
            }
            let b = s.snapshot().unwrap();
            let applied_index = fs::read(d.path().join(".git/index")).unwrap();
            if step == 0 {
                assert!(b.recovery_pending);
                let b = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
                assert_eq!(b.stashes[0].oid, oid);
                assert!(s
                    .mutate(
                        &b.revision,
                        Action::PopStash {
                            oid: oid.clone(),
                            restore_index: true
                        }
                    )
                    .unwrap_err()
                    .contains("DIRTY"));
            } else {
                assert!(b.recovery_pending);
                let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
                assert!(!c.recovery_pending);
                assert!(c.stashes.is_empty());
            }
            assert_eq!(
                fs::read(d.path().join(".git/index")).unwrap(),
                applied_index
            );
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(
                fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
                "untracked version\n"
            );
        }
    }
    #[test]
    fn stash_pop_conflict_preserves_stash_and_current_worktree() {
        let d = fixture();
        let oid = stash_three_areas(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        fs::write(d.path().join("hello.txt"), "conflicting HEAD").unwrap();
        let a = s.snapshot().unwrap();
        let b = s.mutate(&a.revision, Action::StageAll).unwrap();
        let c = s
            .mutate(
                &b.revision,
                Action::Commit {
                    message: "conflict".into(),
                    author: "Test".into(),
                    email: "test@example.invalid".into(),
                },
            )
            .unwrap();
        let log = fs::read(d.path().join(".git/logs/refs/stash")).unwrap();
        let failure = s
            .mutate(
                &c.revision,
                Action::PopStash {
                    oid,
                    restore_index: true,
                },
            )
            .unwrap_err();
        assert!(failure.contains("STASH_CONFLICT"), "{failure}");
        assert_eq!(
            fs::read(d.path().join(".git/logs/refs/stash")).unwrap(),
            log
        );
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "conflicting HEAD"
        );
        assert!(!d.path().join("untracked.txt").exists());
    }
    #[test]
    fn stash_pop_duplicate_oid_is_rejected_before_apply() {
        let d = fixture();
        let oid = stash_three_areas(d.path());
        let r = Repository::open(d.path()).unwrap();
        let mut log = r.reflog("refs/stash").unwrap();
        log.append(
            Oid::from_str(&oid).unwrap(),
            &git2::Signature::now("Test", "test@example.invalid").unwrap(),
            Some("duplicate"),
        )
        .unwrap();
        log.write().unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert_eq!(a.stashes.len(), 2);
        let before = fs::read(d.path().join(".git/index")).unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::PopStash {
                    oid,
                    restore_index: true
                }
            )
            .unwrap_err()
            .contains("AMBIGUOUS"));
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "hello\n"
        );
    }
    #[test]
    fn stash_apply_receipt_detects_file_and_index_changes_before_drop() {
        for index_changed in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            let mut lock = IndexLock::acquire(&s.root).unwrap();
            let receipt =
                stash_apply::apply(&mut s, &r, &a.revision, &oid, true, &mut lock, None).unwrap();
            let path = d.path().join(if index_changed {
                ".git/index"
            } else {
                "hello.txt"
            });
            let mut bytes = fs::read(&path).unwrap();
            bytes.extend_from_slice(b"external change");
            fs::write(&path, &bytes).unwrap();
            assert!(receipt
                .verify(&s.root, &r)
                .unwrap_err()
                .contains("STASH_APPLY_RESULT_CHANGED"));
            assert_eq!(r.refname_to_id("refs/stash").unwrap().to_string(), oid);
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn stash_apply_restores_three_areas_and_preserves_stash_and_head() {
        for restore_index in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let log_before = fs::read(d.path().join(".git/logs/refs/stash")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let head = a.commits[0].oid.clone();
            let b = s
                .mutate(
                    &a.revision,
                    Action::ApplyStash {
                        oid: oid.clone(),
                        restore_index,
                    },
                )
                .unwrap();
            assert_eq!(b.stashes[0].oid, oid);
            assert_eq!(b.commits[0].oid, head);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert_eq!(
                fs::read_to_string(d.path().join("untracked.txt")).unwrap(),
                "untracked version\n"
            );
            let r = Repository::open(d.path()).unwrap();
            let idx = r.index().unwrap();
            let blob = r
                .find_blob(idx.get_path(Path::new("hello.txt"), 0).unwrap().id)
                .unwrap();
            assert_eq!(
                blob.content(),
                if restore_index {
                    b"staged version\n".as_slice()
                } else {
                    b"hello\n".as_slice()
                }
            );
            assert!(idx.get_path(Path::new("untracked.txt"), 0).is_none());
            assert_eq!(
                fs::read(d.path().join(".git/logs/refs/stash")).unwrap(),
                log_before
            );
            assert!(!b.recovery_pending);
        }
    }
    #[test]
    fn stash_apply_interruption_rolls_back_index_files_and_keeps_stash() {
        for step in 0..=3 {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            {
                let mut lock = IndexLock::acquire(&s.root).unwrap();
                assert!(stash_apply::apply(
                    &mut s,
                    &r,
                    &a.revision,
                    &oid,
                    true,
                    &mut lock,
                    Some(step)
                )
                .unwrap_err()
                .contains("INJECTED"));
            }
            let b = s.snapshot().unwrap();
            assert!(b.recovery_pending);
            let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
            assert!(c.changes.is_empty());
            assert!(!c.recovery_pending);
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "hello\n"
            );
            assert!(!d.path().join("untracked.txt").exists());
            assert_eq!(c.stashes[0].oid, oid);
        }
    }
    #[test]
    fn stash_apply_rejects_dirty_and_ignored_collision_without_writes() {
        for ignored in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            if ignored {
                fs::write(d.path().join(".git/info/exclude"), "untracked.txt\n").unwrap();
            }
            let path = d.path().join(if ignored {
                "untracked.txt"
            } else {
                "hello.txt"
            });
            fs::write(&path, "preserve me").unwrap();
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let error = s
                .mutate(
                    &a.revision,
                    Action::ApplyStash {
                        oid,
                        restore_index: true,
                    },
                )
                .unwrap_err();
            assert!(
                error.contains(if ignored { "COLLISION" } else { "DIRTY" }),
                "{error}"
            );
            assert_eq!(fs::read_to_string(path).unwrap(), "preserve me");
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert!(!stash_apply::pending(&s.root));
        }
    }
    #[test]
    fn stash_apply_merges_new_head_and_rejects_content_conflicts_before_writes() {
        for conflict in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            fs::write(
                d.path()
                    .join(if conflict { "hello.txt" } else { "other.txt" }),
                "new branch content\n",
            )
            .unwrap();
            let a = s.snapshot().unwrap();
            let b = s.mutate(&a.revision, Action::StageAll).unwrap();
            let c = s
                .mutate(
                    &b.revision,
                    Action::Commit {
                        message: "advance HEAD".into(),
                        author: "Test".into(),
                        email: "test@example.invalid".into(),
                    },
                )
                .unwrap();
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let result = s.mutate(
                &c.revision,
                Action::ApplyStash {
                    oid,
                    restore_index: true,
                },
            );
            if conflict {
                let error = result.unwrap_err();
                assert!(
                    error.contains("STASH_CONFLICT"),
                    "unexpected stash conflict result: {error}"
                );
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "new branch content\n"
                );
                assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
                assert!(!d.path().join("untracked.txt").exists());
                assert!(!stash_apply::pending(&s.root));
            } else {
                let result = result.unwrap();
                assert_eq!(result.commits[0].oid, c.commits[0].oid);
                assert_eq!(
                    fs::read_to_string(d.path().join("other.txt")).unwrap(),
                    "new branch content\n"
                );
                assert_eq!(
                    fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                    "worktree version\n"
                );
            }
        }
    }
    #[test]
    fn stash_apply_recovery_rejects_changed_index_and_corrupt_backup() {
        for corrupt_backup in [false, true] {
            let d = fixture();
            let oid = stash_three_areas(d.path());
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            {
                let mut lock = IndexLock::acquire(&s.root).unwrap();
                stash_apply::apply(&mut s, &r, &a.revision, &oid, true, &mut lock, Some(1))
                    .unwrap_err();
            }
            let path = d.path().join(if corrupt_backup {
                ".git/branchdesk-stash-apply/index.after"
            } else {
                ".git/index"
            });
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(1);
            fs::write(&path, &bytes).unwrap();
            let mut lock = IndexLock::acquire(&s.root).unwrap();
            assert!(stash_apply::recover(&s.root, &r, &mut lock).is_err());
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "worktree version\n"
            );
            assert!(stash_apply::pending(&s.root));
        }
    }
    #[test]
    fn stash_apply_recovery_preserves_post_interruption_edit() {
        let d = fixture();
        let oid = stash_three_areas(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(&s.root).unwrap();
        {
            let mut lock = IndexLock::acquire(&s.root).unwrap();
            stash_apply::apply(&mut s, &r, &a.revision, &oid, true, &mut lock, Some(1))
                .unwrap_err();
        }
        fs::write(d.path().join("hello.txt"), "later user edit").unwrap();
        let b = s.snapshot().unwrap();
        assert!(s
            .mutate(&b.revision, Action::RecoverCheckout)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "later user edit"
        );
        assert!(stash_apply::pending(&s.root));
    }
    fn three_stashes(root: &Path) {
        let mut r = Repository::open(root).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        for message in ["first", "second", "third"] {
            fs::write(root.join("hello.txt"), message).unwrap();
            r.stash_save(&sig, message, None).unwrap();
        }
    }
    #[test]
    fn stash_drop_removes_only_selected_oid_and_preserves_worktree_index() {
        for selected in 0..3 {
            let d = fixture();
            three_stashes(d.path());
            fs::write(d.path().join("hello.txt"), "current work preserved").unwrap();
            let before = fs::read(d.path().join(".git/index")).unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let oid = a.stashes[selected].oid.clone();
            let b = s
                .mutate(&a.revision, Action::DropStash { oid: oid.clone() })
                .unwrap();
            assert_eq!(b.stashes.len(), 2);
            assert!(!b.stashes.iter().any(|x| x.oid == oid));
            assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
            assert_eq!(
                fs::read_to_string(d.path().join("hello.txt")).unwrap(),
                "current work preserved"
            );
            assert!(!b.recovery_pending);
            // Remove all remaining items, including the final refs/stash removal.
            let c = s
                .mutate(
                    &b.revision,
                    Action::DropStash {
                        oid: b.stashes[0].oid.clone(),
                    },
                )
                .unwrap();
            let e = s
                .mutate(
                    &c.revision,
                    Action::DropStash {
                        oid: c.stashes[0].oid.clone(),
                    },
                )
                .unwrap();
            assert!(e.stashes.is_empty());
            assert!(Repository::open(d.path())
                .unwrap()
                .find_reference("refs/stash")
                .is_err());
        }
    }
    #[test]
    fn interrupted_stash_drop_resumes_without_dropping_another_entry() {
        for step in 0..=2 {
            for selected in 0..3 {
                let d = fixture();
                three_stashes(d.path());
                let mut s = Session::new(d.path().into()).unwrap();
                let a = s.snapshot().unwrap();
                let oid = a.stashes[selected].oid.clone();
                let r = open(&s.root).unwrap();
                assert!(
                    stash_write::drop_stash(&mut s, &r, &a.revision, &oid, Some(step))
                        .unwrap_err()
                        .contains("INJECTED")
                );
                let b = s.snapshot().unwrap();
                assert!(b.recovery_pending);
                assert!(s.mutate(&b.revision, Action::StageAll).is_err());
                let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
                assert!(!c.recovery_pending);
                assert_eq!(c.stashes.len(), 2);
                assert!(!c.stashes.iter().any(|x| x.oid == oid));
            }
        }
    }
    #[test]
    fn final_stash_drop_recovers_each_partial_update() {
        for step in 0..=2 {
            let d = fixture();
            let mut r = Repository::open(d.path()).unwrap();
            fs::write(d.path().join("hello.txt"), "saved change").unwrap();
            r.stash_save(
                &git2::Signature::now("Test", "test@example.invalid").unwrap(),
                "only",
                None,
            )
            .unwrap();
            let mut s = Session::new(d.path().into()).unwrap();
            let a = s.snapshot().unwrap();
            let r = open(&s.root).unwrap();
            stash_write::drop_stash(&mut s, &r, &a.revision, &a.stashes[0].oid, Some(step))
                .unwrap_err();
            let b = s.snapshot().unwrap();
            let c = s.mutate(&b.revision, Action::RecoverCheckout).unwrap();
            assert!(c.stashes.is_empty());
            assert!(!c.recovery_pending);
            assert!(r.find_reference("refs/stash").is_err());
        }
    }
    #[test]
    fn stash_drop_rejects_stale_selection_and_foreign_ref_lock() {
        let d = fixture();
        three_stashes(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let mut r = Repository::open(d.path()).unwrap();
        r.stash_drop(2).unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::DropStash {
                    oid: a.stashes[0].oid.clone()
                }
            )
            .unwrap_err()
            .contains("STALE"));
        let b = s.snapshot().unwrap();
        let before = fs::read(d.path().join(".git/logs/refs/stash")).unwrap();
        let lock = d.path().join(".git/refs/stash.lock");
        fs::write(&lock, "external lock").unwrap();
        assert!(s
            .mutate(
                &b.revision,
                Action::DropStash {
                    oid: b.stashes[0].oid.clone()
                }
            )
            .is_err());
        assert_eq!(fs::read_to_string(lock).unwrap(), "external lock");
        assert_eq!(
            fs::read(d.path().join(".git/logs/refs/stash")).unwrap(),
            before
        );
        assert!(!stash_write::pending(&s.root));
    }
    #[test]
    fn interrupted_stash_drop_preserves_external_reflog_edit() {
        let d = fixture();
        three_stashes(d.path());
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = open(&s.root).unwrap();
        stash_write::drop_stash(&mut s, &r, &a.revision, &a.stashes[1].oid, Some(0)).unwrap_err();
        let path = d.path().join(".git/logs/refs/stash");
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"external edit\n");
        fs::write(&path, &bytes).unwrap();
        assert!(stash_write::recover(&s.root, &r)
            .unwrap_err()
            .contains("RECOVERY_CONFLICT"));
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(stash_write::pending(&s.root));
    }
    #[test]
    fn removing_older_stash_invalidates_revision_without_changing_tip() {
        let d = fixture();
        let mut r = Repository::open(d.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        fs::write(d.path().join("hello.txt"), "older").unwrap();
        r.stash_save(&sig, "older", None).unwrap();
        fs::write(d.path().join("hello.txt"), "newer").unwrap();
        let newest = r.stash_save(&sig, "newer", None).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        r.stash_drop(1).unwrap();
        assert_eq!(r.refname_to_id("refs/stash").unwrap(), newest);
        assert!(s
            .stash_detail(&a.revision, &newest.to_string(), None, None)
            .unwrap_err()
            .starts_with("STALE_STATE"));
        let b = s.snapshot().unwrap();
        assert_eq!(b.stashes.len(), 1);
        let detail = s
            .stash_detail(&b.revision, &newest.to_string(), None, None)
            .unwrap();
        assert!(detail
            .groups
            .iter()
            .find(|g| g.area == StashArea::Untracked)
            .unwrap()
            .files
            .is_empty());
    }
    #[test]
    fn malformed_stash_is_rejected_without_mutating_repository() {
        let d = fixture();
        let r = Repository::open(d.path()).unwrap();
        let oid = r.refname_to_id("HEAD").unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        r.reference("refs/stash", oid, false, "malformed").unwrap();
        let mut log = r.reflog("refs/stash").unwrap();
        log.append(oid, &sig, Some("malformed")).unwrap();
        log.write().unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .stash_detail(&a.revision, &oid.to_string(), None, None)
            .unwrap_err()
            .starts_with("UNSUPPORTED_STASH_LAYOUT"));
        assert_eq!(r.refname_to_id("refs/stash").unwrap(), oid);
    }
    fn annotated_action(name: &str, oid: &str) -> Action {
        Action::CreateAnnotatedTag {
            name: name.into(),
            oid: oid.into(),
            message: "Release notes\n\nSecond paragraph".into(),
            author: "Tag Author".into(),
            email: "tagger@example.invalid".into(),
        }
    }
    #[test]
    fn annotated_tag_creation_stores_message_tagger_and_commit_without_worktree_changes() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "keep dirty work").unwrap();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let oid = a.commits[0].oid.clone();
        let b = s
            .mutate(&a.revision, annotated_action("release/annotated", &oid))
            .unwrap();
        let listed = b
            .tags
            .iter()
            .find(|t| t.name == "release/annotated")
            .unwrap();
        assert!(listed.annotated);
        assert_eq!(listed.oid.as_deref(), Some(oid.as_str()));
        let r = Repository::open(d.path()).unwrap();
        let tag_id = r.refname_to_id("refs/tags/release/annotated").unwrap();
        assert_ne!(tag_id.to_string(), oid);
        let tag = r.find_tag(tag_id).unwrap();
        assert_eq!(tag.name().unwrap(), "release/annotated");
        assert_eq!(
            tag.message().unwrap(),
            Some("Release notes\n\nSecond paragraph")
        );
        assert_eq!(tag.target_id().to_string(), oid);
        assert_eq!(tag.target_type(), Some(git2::ObjectType::Commit));
        assert_eq!(tag.tagger().unwrap().name().unwrap(), "Tag Author");
        assert_eq!(
            tag.tagger().unwrap().email().unwrap(),
            "tagger@example.invalid"
        );
        assert!(b.commits[0].refs.contains(&"release/annotated".into()));
        let c = s
            .mutate(
                &b.revision,
                Action::DeleteTag {
                    name: "release/annotated".into(),
                },
            )
            .unwrap();
        assert!(c.tags.is_empty());
        assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "keep dirty work"
        );
        assert_eq!(r.head().unwrap().target().unwrap().to_string(), oid);
    }
    #[test]
    fn annotated_tag_duplicate_external_lock_and_stale_selection_are_rejected() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let oid = a.commits[0].oid.clone();
        let b = s
            .mutate(&a.revision, annotated_action("one", &oid))
            .unwrap();
        assert!(s
            .mutate(&b.revision, annotated_action("one", &oid))
            .unwrap_err()
            .contains("REF_EXISTS"));
        assert!(s
            .mutate(&a.revision, annotated_action("stale", &oid))
            .unwrap_err()
            .contains("STALE"));
        fs::write(d.path().join(".git/refs/tags/locked.lock"), "foreign owner").unwrap();
        assert!(s
            .mutate(&b.revision, annotated_action("locked", &oid))
            .is_err());
        assert_eq!(
            fs::read_to_string(d.path().join(".git/refs/tags/locked.lock")).unwrap(),
            "foreign owner"
        );
        let c = s.snapshot().unwrap();
        assert_eq!(c.tags.len(), 1);
        assert_eq!(c.tags[0].name, "one");
    }
    #[test]
    fn annotated_tag_invalid_payloads_do_not_publish_refs() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let oid = a.commits[0].oid.clone();
        for invalid in 0..7 {
            let mut action = annotated_action("invalid", &oid);
            if let Action::CreateAnnotatedTag {
                name,
                oid,
                message,
                author,
                email,
            } = &mut action
            {
                match invalid {
                    0 => *message = " ".into(),
                    1 => *message = "bad\0message".into(),
                    2 => *author = "".into(),
                    3 => *email = "bad\n@example.invalid".into(),
                    4 => *oid = "HEAD".into(),
                    5 => *name = "../escape".into(),
                    _ => *message = "x".repeat(10001),
                }
            }
            assert!(s.mutate(&a.revision, action).is_err());
            assert!(s.snapshot().unwrap().tags.is_empty());
        }
    }
    #[test]
    fn tags_round_trip_without_touching_worktree_or_head() {
        let d = fixture();
        fs::write(d.path().join("hello.txt"), "uncommitted\n").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let oid = a.commits[0].oid.clone();
        let index = fs::read(d.path().join(".git/index")).unwrap();
        let b = s
            .mutate(
                &a.revision,
                Action::CreateTag {
                    name: "release/stable".into(),
                    oid: oid.clone(),
                },
            )
            .unwrap();
        assert_eq!(b.tags[0].name, "release/stable");
        assert_eq!(b.tags[0].oid.as_deref(), Some(oid.as_str()));
        assert!(!b.tags[0].annotated);
        assert!(b.commits[0].refs.contains(&"release/stable".into()));
        assert_ne!(a.revision, b.revision);
        let duplicate_error = s
            .mutate(
                &b.revision,
                Action::CreateTag {
                    name: "release/stable".into(),
                    oid: oid.clone(),
                },
            )
            .unwrap_err();
        assert!(duplicate_error.contains("REF_EXISTS"), "{duplicate_error}");
        let c = s
            .mutate(
                &b.revision,
                Action::DeleteTag {
                    name: "release/stable".into(),
                },
            )
            .unwrap();
        assert!(c.tags.is_empty());
        assert_eq!(c.commits[0].oid, oid);
        assert_eq!(index, fs::read(d.path().join(".git/index")).unwrap());
        assert_eq!(
            fs::read_to_string(d.path().join("hello.txt")).unwrap(),
            "uncommitted\n"
        );
    }
    #[test]
    fn annotated_tags_resolve_to_commit_and_non_commit_tags_remain_visible() {
        let d = fixture();
        let r = Repository::open(d.path()).unwrap();
        let head = r.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        r.tag("release", head.as_object(), &sig, "Release notes", false)
            .unwrap();
        r.tag_lightweight("tree-only", head.tree().unwrap().as_object(), false)
            .unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let tag = a.tags.iter().find(|t| t.name == "release").unwrap();
        assert!(tag.annotated);
        assert_eq!(tag.oid, Some(head.id().to_string()));
        assert!(a.commits[0].refs.contains(&"release".into()));
        assert!(a
            .tags
            .iter()
            .any(|t| t.name == "tree-only" && t.oid.is_none()));
    }
    #[test]
    fn branch_delete_refuses_current_and_unmerged_but_allows_merged() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::DeleteBranch {
                    name: a.branch.clone()
                }
            )
            .unwrap_err()
            .starts_with("CURRENT_BRANCH"));
        let b = s
            .mutate(
                &a.revision,
                Action::CreateBranch {
                    name: "merged".into(),
                },
            )
            .unwrap();
        let c = s
            .mutate(
                &b.revision,
                Action::DeleteBranch {
                    name: "merged".into(),
                },
            )
            .unwrap();
        assert!(!c.branches.iter().any(|b| b.name == "merged"));
        let r = Repository::open(d.path()).unwrap();
        let parent = r.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let oid = r
            .commit(
                None,
                &sig,
                &sig,
                "Unmerged",
                &parent.tree().unwrap(),
                &[&parent],
            )
            .unwrap();
        r.branch("unmerged", &r.find_commit(oid).unwrap(), false)
            .unwrap();
        let a = s.snapshot().unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::DeleteBranch {
                    name: "unmerged".into()
                }
            )
            .unwrap_err()
            .starts_with("UNMERGED_BRANCH"));
        assert_eq!(r.refname_to_id("refs/heads/unmerged").unwrap(), oid);
    }
    #[test]
    fn external_ref_change_invalidates_preview_even_without_head_change() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let r = Repository::open(d.path()).unwrap();
        let head = r.head().unwrap().peel_to_commit().unwrap();
        r.branch("external", &head, false).unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::CreateTag {
                    name: "stale".into(),
                    oid: head.id().to_string()
                }
            )
            .unwrap_err()
            .starts_with("STALE_STATE"));
        assert!(r.find_reference("refs/tags/stale").is_err());
        let a = s.snapshot().unwrap();
        r.find_branch("external", git2::BranchType::Local)
            .unwrap()
            .delete()
            .unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::CreateBranch {
                    name: "stale".into()
                }
            )
            .unwrap_err()
            .starts_with("STALE_STATE"));
    }
    #[test]
    fn ref_lock_and_invalid_names_do_not_change_refs_or_index() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let oid = a.commits[0].oid.clone();
        for name in [
            "",
            "../escape",
            "bad\0name",
            "a.lock",
            "-option",
            "a b",
            "@{x}",
        ] {
            assert!(s
                .mutate(
                    &a.revision,
                    Action::CreateTag {
                        name: name.into(),
                        oid: oid.clone()
                    }
                )
                .unwrap_err()
                .starts_with("INVALID_REF_NAME"));
        }
        assert!(s
            .mutate(
                &a.revision,
                Action::CreateTag {
                    name: "unknown".into(),
                    oid: "f".repeat(40)
                }
            )
            .unwrap_err()
            .starts_with("INVALID_COMMIT"));
        fs::create_dir_all(d.path().join(".git/refs/tags")).unwrap();
        fs::write(
            d.path().join(".git/refs/tags/locked.lock"),
            "external-owner",
        )
        .unwrap();
        assert!(s
            .mutate(
                &a.revision,
                Action::CreateTag {
                    name: "locked".into(),
                    oid
                }
            )
            .is_err());
        assert_eq!(
            fs::read_to_string(d.path().join(".git/refs/tags/locked.lock")).unwrap(),
            "external-owner"
        );
        assert!(!d.path().join(".git/index.lock").exists());
        assert!(s.snapshot().unwrap().tags.is_empty());
    }
    #[test]
    fn ai_and_unknown_action_payloads_are_rejected() {
        assert!(serde_json::from_str::<Action>(r#"{"kind":"aiCommit","prompt":"test"}"#).is_err());
        assert!(serde_json::from_str::<Action>(
            r#"{"kind":"createTag","name":"v1","oid":"abc","force":true}"#
        )
        .is_err());
    }
    #[test]
    fn branch_creation_does_not_checkout() {
        let d = fixture();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        let b = s
            .mutate(
                &a.revision,
                Action::CreateBranch {
                    name: "feature/test".into(),
                },
            )
            .unwrap();
        assert_eq!(a.branch, b.branch);
        assert!(b.branches.iter().any(|b| b.name == "feature/test"));
    }
    #[test]
    fn config_policy_normal_names_remain_writable() {
        let d = fixture();
        let path = d.path().join(".git/config");
        let mut config = fs::read_to_string(&path).unwrap();
        config.push_str(
            r#"
# include worktree extensions sign filter
[remote "design-filter"]
url = https://example.invalid/include/worktree/extensions.git
fetch = +refs/heads/*:refs/remotes/design-filter/*
[branch "sign"]
remote = design-filter
merge = refs/heads/filter
"#,
        );
        fs::write(path, config).unwrap();
        let mut session = Session::new(d.path().into()).unwrap();
        let snapshot = session.snapshot().unwrap();
        assert!(snapshot.write_block.is_none());
        let result = session
            .mutate(
                &snapshot.revision,
                Action::CreateBranch {
                    name: "design-filter".into(),
                },
            )
            .unwrap();
        assert!(result.branches.iter().any(|b| b.name == "design-filter"));
    }
    #[test]
    fn config_include_rejected_before_open() {
        let d = fixture();
        fs::write(d.path().join(".git/config"), "[include]\npath = /outside\n").unwrap();
        assert!(Session::new(d.path().into()).is_err());
    }
    #[test]
    fn hooks_do_not_execute_and_writes_block() {
        let d = fixture();
        fs::create_dir_all(d.path().join(".git/hooks")).unwrap();
        fs::write(
            d.path().join(".git/hooks/pre-commit"),
            "touch SHOULD_NOT_EXIST",
        )
        .unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(a.write_block.is_some());
        assert!(!d.path().join("SHOULD_NOT_EXIST").exists());
    }
    #[test]
    fn attributes_block_raw_staging() {
        let d = fixture();
        fs::write(d.path().join(".gitattributes"), "* filter=evil").unwrap();
        let mut s = Session::new(d.path().into()).unwrap();
        let a = s.snapshot().unwrap();
        assert!(a.write_block.is_some());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_repo_rejected() {
        let d = fixture();
        std::os::unix::fs::symlink("/tmp", d.path().join("escape")).unwrap();
        assert!(Session::new(d.path().into()).is_err());
    }
}

#[cfg(test)]
mod linked_transactions;

mod linked_writes;
