#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod named_workspaces;
mod lfs_transfer;
mod workspace_wips;
mod file_manager;
mod provider_github;
mod provider_write;
mod patch_export;
mod patch_preview;
mod clone_repository;
mod commit_draft;
mod credential_vault;
mod identity_settings;
mod init_repository;
#[cfg(target_os = "macos")]
mod macos_termination;
mod network_access;
mod network_worker;
mod recent;
mod repository;
mod shutdown;
mod terminal;
mod external_editor;
mod ssh_endpoint;
#[cfg(all(test, target_os = "macos"))]
mod ssh_process;
#[cfg(all(test, target_os = "macos"))]
mod ssh_discovery;
#[cfg(all(test, target_os = "macos"))]
mod ssh_authorization;
#[cfg(test)]
mod ssh_trust;
mod workspace;
use repository::{
    Action, CommitDetail, ContentSource, DiffResult, FileBlame, FileContent, FileHistory,
    MergePlan, ReplayKind, ReplayPlan, Session, Snapshot, StashArea, StashDetail,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
type Store = Arc<Mutex<Vec<Session>>>;
const MAX_SESSIONS: usize = 12;
struct OperationGuard(Arc<AtomicBool>);
impl OperationGuard {
    fn acquire(app: &tauri::AppHandle) -> Result<Self, String> {
        let flag = app.state::<Arc<AtomicBool>>().inner().clone();
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "BUSY: 다른 작업이 진행 중입니다.".to_string())?;
        Ok(Self(flag))
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn window_check(window: &WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|_| "DENIED_ORIGIN")?;
    let bundled = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        && url.port() == Some(1420);
    if window.label() == "main" && (bundled || development) {
        Ok(())
    } else {
        Err("DENIED_WINDOW_OR_ORIGIN".into())
    }
}
fn current<'a>(state: &'a mut [Session], id: &str) -> Result<&'a mut Session, String> {
    state
        .iter_mut()
        .find(|s| s.id() == id)
        .ok_or("INVALID_REPO_ID".into())
}
fn repository_path(value: &str) -> Result<std::path::PathBuf, String> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || value.starts_with("//")
        || value.starts_with(r"\\")
    {
        return Err("INVALID_PATH: 로컬 저장소의 절대 경로를 입력하세요.".into());
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return Err(
            "INVALID_PATH: 절대 경로만 지원합니다. ~와 환경 변수는 확장하지 않습니다.".into(),
        );
    }
    Ok(path)
}
fn replace_repository(store: &Store, path: std::path::PathBuf) -> Result<Snapshot, String> {
    let mut session = Session::new(path)?;
    let mut sessions = store.lock().map_err(|_| "LOCK_ERROR")?;
    if let Some(existing) = sessions
        .iter_mut()
        .find(|s| s.root_path() == session.root_path())
    {
        return existing.snapshot();
    }
    if sessions.len() >= MAX_SESSIONS {
        return Err("TAB_LIMIT: 최대 12개 저장소를 열 수 있습니다.".into());
    }
    let snapshot = session.snapshot()?;
    sessions.push(session);
    Ok(snapshot)
}
fn confirm_repository(
    window: &WebviewWindow,
    app: &tauri::AppHandle,
    path: std::path::PathBuf,
) -> Result<Option<Snapshot>, String> {
    let message = format!("{}\n\n선택한 저장소를 로컬 Git 라이브러리로 읽습니다. 본인이 관리하는 저장소만 여세요. 네트워크·훅·외부 명령은 실행하지 않습니다.", path.display());
    if !app
        .dialog()
        .message(message)
        .parent(window)
        .title("저장소 열기")
        .buttons(MessageDialogButtons::OkCancel)
        .blocking_show()
    {
        return Ok(None);
    }
    let snapshot = replace_repository(app.state::<Store>().inner(), path.clone())?;
    remember_repository(app, &path);
    Ok(Some(snapshot))
}
#[derive(serde::Serialize)]
struct RecentRepositories {
    paths: Vec<String>,
    warning: Option<String>,
}
fn remember_repository(app: &tauri::AppHandle, path: &std::path::Path) {
    let result = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| recent::remember(&dir, path));
    if let Ok(mut warning) = app.state::<Mutex<Option<String>>>().lock() {
        *warning = result.err();
    }
}
fn close_session(store: &Store, id: &str) -> Result<(), String> {
    let mut sessions = store.lock().map_err(|_| "LOCK_ERROR")?;
    let index = sessions
        .iter()
        .position(|s| s.id() == id)
        .ok_or("INVALID_REPO_ID")?;
    sessions.remove(index);
    Ok(())
}
fn draft_directory(app: &tauri::AppHandle, repo_id: &str) -> Result<std::path::PathBuf, String> {
    let store = app.state::<Store>();
    let mut sessions = store.lock().map_err(|_| "LOCK_ERROR")?;
    let root = current(&mut sessions, repo_id)?.root_path();
    commit_draft::directory(
        &app.path().app_data_dir().map_err(|_| "DRAFT_PATH_FAILED")?,
        root,
    )
}
#[tauri::command]
async fn read_commit_draft(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
) -> Result<commit_draft::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        commit_draft::read(&draft_directory(&app, &repo_id)?)
    })
    .await
    .map_err(|_| "DRAFT_TASK_FAILED")?
}
#[tauri::command]
async fn save_commit_draft(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    draft: Option<commit_draft::Draft>,
) -> Result<commit_draft::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        commit_draft::save(&draft_directory(&app, &repo_id)?, &revision, draft)
    })
    .await
    .map_err(|_| "DRAFT_TASK_FAILED")?
}
#[tauri::command]
async fn read_identity_settings(
    window: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<identity_settings::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|_| "IDENTITY_PATH_FAILED")?;
        identity_settings::read(&dir)
    })
    .await
    .map_err(|_| "IDENTITY_TASK_FAILED")?
}
#[tauri::command]
async fn save_identity_settings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    revision: String,
    identity: Option<identity_settings::Identity>,
) -> Result<identity_settings::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|_| "IDENTITY_PATH_FAILED")?;
        identity_settings::save(&dir, &revision, identity)
    })
    .await
    .map_err(|_| "IDENTITY_TASK_FAILED")?
}
// Resolve only currently authorized sessions. The renderer cannot persist arbitrary paths.
fn workspace_for_sessions(
    sessions: &[Session],
    ids: &[String],
    active: Option<&str>,
) -> Result<workspace::Workspace, String> {
    if ids.len() > MAX_SESSIONS {
        return Err("WORKSPACE_LIMIT".into());
    }
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err("DUPLICATE_WORKSPACE_SESSION".into());
        }
        let session = sessions
            .iter()
            .find(|s| s.id() == id)
            .ok_or("INVALID_REPO_ID")?;
        paths.push(
            session
                .root_path()
                .to_str()
                .ok_or("INVALID_PATH_ENCODING")?
                .to_string(),
        );
    }
    let active = match active {
        Some(id) => Some(
            paths[ids
                .iter()
                .position(|i| i == id)
                .ok_or("INVALID_ACTIVE_SESSION")?]
            .clone(),
        ),
        None => None,
    };
    Ok(workspace::Workspace { paths, active })
}
#[tauri::command]
async fn forget_github_credential(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String)->Result<bool,String>{
    window_check(&window)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _operation=operation;
        if !cfg!(target_os="macos"){return Err("CREDENTIAL_STORAGE_UNSUPPORTED".into());}
        if !app.dialog().message(format!("{endpoint}\n\nRemove FLOE's saved credential for this exact GitHub API repository URL from the local Keychain. This does not revoke the token on GitHub, change Git data, or remove credentials for Git remote URLs or other API repositories."))
            .parent(&window).title("Forget saved GitHub credential").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(false);}
        credential_vault::delete(&endpoint)?;
        Ok(true)
    }).await.map_err(|_|"CREDENTIAL_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn github_write(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,action:provider_write::Action)->Result<Option<provider_write::Receipt>,String>{
    window_check(&window)?;provider_write::validate(&owner,&repo,&action)?;
    let operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _operation=operation;
        let endpoint=provider_write::endpoint(&owner,&repo)?;
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"GitHub remote change",provider_write::describe(&owner,&repo,&action)?)? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::execute(&owner,&repo,&action,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WRITE_OUTCOME_UNKNOWN")?
}
#[tauri::command]
async fn github_item_detail(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,number:u64,pull:bool)->Result<Option<provider_write::Detail>,String>{
    window_check(&window)?;let endpoint=provider_write::endpoint(&owner,&repo)?;
    let _operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read GitHub item",format!("Read {} #{number} in {owner}/{repo}. Token is used only for this GitHub API repository.",if pull{"pull request"}else{"issue"}))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::inspect(&owner,&repo,number,pull,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn github_pull_files(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,number:u64,page:u32,head:String,base:String)->Result<Option<provider_write::PullFiles>,String>{
    window_check(&window)?;
    provider_write::validate_files(&owner,&repo,number,page,&head,&base)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let _operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read pull request files",format!("Read changed files for {owner}/{repo} #{number}, page {page}. No local files or remote data are changed."))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::pull_files(&owner,&repo,number,page,&head,&base,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn github_metadata_options(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,kind:provider_write::metadata::Kind,page:u32)->Result<Option<provider_write::metadata::Page>,String>{
    window_check(&window)?;
    provider_write::metadata::validate(&owner,&repo,page)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _operation=operation;
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read repository options",format!("Read {kind:?} for {owner}/{repo}, page {page}. No remote changes are submitted."))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::metadata::read(&owner,&repo,kind,page,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn github_item_comments(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,number:u64,page:u32,pull:bool)->Result<Option<provider_write::discussion::Page>,String>{
    window_check(&window)?;
    provider_write::discussion::validate_conversation(&owner,&repo,number,page)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _operation=operation;
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read conversation comments",format!("Read general comments for {owner}/{repo} #{number}, page {page}. No remote changes are submitted."))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::discussion::read_conversation(&owner,&repo,number,page,pull,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn github_pull_discussion(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,number:u64,page:u32,head:String,base:String,kind:provider_write::discussion::Kind)->Result<Option<provider_write::discussion::Page>,String>{
    window_check(&window)?;
    provider_write::validate_files(&owner,&repo,number,page,&head,&base)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let _operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read pull request discussion",format!("Read {kind:?} for {owner}/{repo} #{number}, page {page}. No remote changes are submitted."))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::discussion::read(&owner,&repo,number,page,&head,&base,kind,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn github_pull_checks(window:WebviewWindow,app:tauri::AppHandle,owner:String,repo:String,number:u64,page:u32,head:String,base:String,kind:provider_write::checks::Kind)->Result<Option<provider_write::checks::Page>,String>{
    window_check(&window)?;
    provider_write::validate_files(&owner,&repo,number,page,&head,&base)?;
    let endpoint=provider_write::endpoint(&owner,&repo)?;
    let _operation=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Read pull request build status",format!("Read {kind:?} for {owner}/{repo} #{number}, page {page}. Commit {head}. No checks are triggered or changed."))? else{return Ok(None)};
        let bound=choice.resolve(&endpoint)?.ok_or("PROVIDER_AUTH_REQUIRED")?;
        let credential=bound.credential(&endpoint)?;
        provider_write::checks::read(&owner,&repo,number,page,&head,&base,kind,&credential).map(Some)
    }).await.map_err(|_|"PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn read_github_pull_requests(window: WebviewWindow, app: tauri::AppHandle, owner: String, repo: String, page: u32) -> Result<provider_github::Page, String> {
    window_check(&window)?;
    provider_github::validate(&owner, &repo, page)?;
    let _operation = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || provider_github::read(&owner, &repo, page))
        .await.map_err(|_| "PROVIDER_WORKER_FAILED")?
}
#[tauri::command]
async fn read_github_issues(window: WebviewWindow, app: tauri::AppHandle, owner: String, repo: String, page: u32) -> Result<provider_github::IssuePage, String> {
    window_check(&window)?;
    provider_github::validate(&owner, &repo, page)?;
    let _operation = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || provider_github::read_issues(&owner, &repo, page))
        .await.map_err(|_| "PROVIDER_WORKER_FAILED")?
}
#[derive(serde::Serialize)]
#[serde(rename_all="camelCase")]
struct OpenRepositoryPath {repo_id:String,path:String}
#[tauri::command]
async fn read_open_repository_paths(window:WebviewWindow,app:tauri::AppHandle)->Result<Vec<OpenRepositoryPath>,String>{
    window_check(&window)?;
    let store=app.state::<Store>();let sessions=store.lock().map_err(|_|"LOCK_ERROR")?;
    Ok(sessions.iter().map(|s|OpenRepositoryPath{repo_id:s.id().to_string(),path:s.root_path().to_string_lossy().into_owned()}).collect())
}
#[tauri::command]
async fn read_workspace_wips(window:WebviewWindow,app:tauri::AppHandle,workspace_id:String)->Result<workspace_wips::Listing,String>{
    window_check(&window)?;
    if workspace_id.len()>64 {return Err("INVALID_WORKSPACE_ID".into());}
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let settings=named_workspaces::read(&app.path().app_data_dir().map_err(|_|"WORKSPACE_PATH_FAILED")?)?;
        if settings.warning.is_some(){return Err("INVALID_NAMED_WORKSPACE_FILE".into());}
        let group=settings.workspace.and_then(|w|w.groups.into_iter().find(|g|g.id==workspace_id)).ok_or("WORKSPACE_NOT_FOUND")?;
        let store=app.state::<Store>();let mut sessions=store.lock().map_err(|_|"LOCK_ERROR")?;
        Ok(workspace_wips::read(&group,&mut sessions))
    }).await.map_err(|_|"WORKER_ERROR")?
}
#[tauri::command]
async fn read_named_workspaces(window:WebviewWindow,app:tauri::AppHandle)->Result<named_workspaces::State,String>{
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move||named_workspaces::read(&app.path().app_data_dir().map_err(|_|"WORKSPACE_PATH_FAILED")?))
        .await.map_err(|_|"WORKER_ERROR")?
}
#[tauri::command]
async fn save_named_workspaces(window:WebviewWindow,app:tauri::AppHandle,revision:String,workspace:named_workspaces::Workspace)->Result<named_workspaces::State,String>{
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move||named_workspaces::save(&app.path().app_data_dir().map_err(|_|"WORKSPACE_PATH_FAILED")?,&revision,Some(workspace)))
        .await.map_err(|_|"WORKER_ERROR")?
}
#[tauri::command]
async fn read_workspace(
    window: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<workspace::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        workspace::read(
            &app.path()
                .app_data_dir()
                .map_err(|_| "WORKSPACE_PATH_FAILED")?,
        )
    })
    .await
    .map_err(|_| "WORKER_ERROR")?
}
#[tauri::command]
async fn save_workspace(
    window: WebviewWindow,
    app: tauri::AppHandle,
    revision: String,
    ids: Vec<String>,
    active: Option<String>,
) -> Result<workspace::State, String> {
    window_check(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<Store>();
        let sessions = store.lock().map_err(|_| "LOCK_ERROR")?;
        let value = workspace_for_sessions(&sessions, &ids, active.as_deref())?;
        workspace::save(
            &app.path()
                .app_data_dir()
                .map_err(|_| "WORKSPACE_PATH_FAILED")?,
            &revision,
            Some(value),
        )
    })
    .await
    .map_err(|_| "WORKER_ERROR")?
}
#[tauri::command]
async fn close_repository(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
) -> Result<(), String> {
    window_check(&window)?;
    let _guard = OperationGuard::acquire(&app)?;
    close_session(app.state::<Store>().inner(), &repo_id)
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteInspection {
    #[serde(flatten)]
    advertisement: network_worker::Advertisement,
    fetch_plan: Option<repository::fetch_plan::Plan>,
    fetch_plan_warning: Option<String>,
}
#[tauri::command]
async fn inspect_remote(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
) -> Result<Option<RemoteInspection>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
  let _guard=guard;
  let endpoint=repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
  let Some(access)=network_access::confirm(&window,&app,&endpoint,"원격 연결 확인",format!("{endpoint}\n\n이 HTTPS 서버에 연결하여 브랜치와 태그 목록을 읽습니다. 로컬 저장소를 변경하지 않습니다."))? else {return Ok(None);};
  // Recheck after the human confirmation and again after the network request.
  repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
  let credential=access.resolve(&endpoint)?;
        let value=network_worker::run_with_credential(&endpoint,credential.as_ref())?;
  repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
  let plan=repository::fetch_plan::preview(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&value);
  let (fetch_plan,fetch_plan_warning)=match plan {Ok(plan)=>(Some(plan),None),Err(error)=>(None,Some(error))};
  Ok(Some(RemoteInspection{advertisement:value,fetch_plan,fetch_plan_warning}))
 }).await.map_err(|_|"REMOTE_WORKER_FAILED".to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialSlot {
    name: String,
    push: bool,
    index: usize,
}
#[tauri::command]
async fn forget_remote_credential(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    slot: CredentialSlot,
) -> Result<bool, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move|| {
        let _guard=guard;
        if !cfg!(target_os="macos") {return Err("CREDENTIAL_STORAGE_UNSUPPORTED".into());}
        let endpoint=repository::remote_network::credential_endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&slot.name,slot.push,slot.index)?;
        if !app.dialog().message(format!("{endpoint}\n\n이 정확한 URL에 대해 FLOE가 저장한 로그인 정보를 키체인에서 삭제합니다. 같은 URL을 쓰는 다른 저장소에도 적용됩니다. 서버 연결, Git 변경, 서버 토큰 폐기는 하지 않습니다. 다른 URL의 로그인은 유지합니다."))
            .parent(&window).title("Forget saved login").buttons(MessageDialogButtons::OkCancel).blocking_show() {return Ok(false);}
        let confirmed_endpoint=network_access::confirmed_credential_endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&slot.name,slot.push,slot.index,&endpoint)?;
        // The temporary Store lock is dropped at the preceding semicolon.
        credential_vault::delete(&confirmed_endpoint)?;
        Ok(true)
    }).await.map_err(|_|"CREDENTIAL_WORKER_FAILED".to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PushExecutionRequest {
    name: String,
    branch: String,
    source_oid: String,
    remote_oid: Option<String>,
}
#[tauri::command]
async fn execute_push(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    request: PushExecutionRequest,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=guard;
        let PushExecutionRequest {name,branch,source_oid,remote_oid}=request;
        let selected=repository::push::selection(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&branch)?;
        if selected.source_oid!=source_oid {return Err("STALE_PUSH_SELECTION".into());}
        let Some(access)=network_access::confirm(&window,&app,&selected.endpoint,"Push branch",format!("{}\n{} → {}\nCommit: {}\n\n이 커밋을 원격 브랜치에 전송합니다. 결과가 확인되지 않으면 자동 재전송하지 않고 기록을 남깁니다.",selected.endpoint,selected.source,selected.destination,selected.source_oid))? else {return Ok(None);};
        if repository::push::selection(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&branch)?!=selected {return Err("STALE_PUSH_SELECTION".into());}
        let credential=access.resolve(&selected.endpoint)?;
        let prepared=repository::push::prepare_pack(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&selected)?;
        let record=repository::push_state::begin(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&selected,remote_oid)?;
        let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.root_path().to_path_buf();
        let old=record.old_oid.clone().unwrap_or_else(||"0".repeat(40));
        let outcome=network_worker::run_push_with_credential(&record.endpoint,&old,&record.new_oid,&record.destination,&prepared,credential.as_ref());
        use repository::push_state::Status;
        match outcome {
            network_worker::PushOutcome::Accepted=>{repository::push_state::finish(&root,&record.id,Status::Accepted,None)?;},
            network_worker::PushOutcome::Rejected(reason)=>{repository::push_state::finish(&root,&record.id,Status::Rejected,Some(reason))?;},
            network_worker::PushOutcome::BeforeSend(code)=>{repository::push_state::finish(&root,&record.id,Status::NotSent,Some(code))?;},
            network_worker::PushOutcome::Unknown=>{},
        }
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.snapshot().map(Some)
    }).await.map_err(|_|"PUSH_WORKER_FAILED_CHECK_SAVED_RECORD".to_string())?
}
#[tauri::command]
async fn check_push_result(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    id: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=guard;let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.root_path().to_path_buf();
        let record=repository::push_state::read(&root)?.ok_or("NO_PUSH_RECORD")?;
        if record.id!=id || record.status!=repository::push_state::Status::Pending {return Err("STALE_PUSH_RECORD".into());}
        let Some(access)=network_access::confirm(&window,&app,&record.endpoint,"Check Push result",format!("{}\n{}\n\n기록된 원격 브랜치의 현재 커밋만 조회합니다. 재전송하지 않으며 조회 결과는 과거 Push의 성공 여부를 확정하지 않습니다.",record.endpoint,record.destination))? else {return Ok(None);};
        let latest=repository::push_state::read(&root)?.ok_or("NO_PUSH_RECORD")?;
        if latest.id!=record.id || latest.status!=record.status || latest.endpoint!=record.endpoint || latest.destination!=record.destination || latest.new_oid!=record.new_oid || latest.old_oid!=record.old_oid {return Err("STALE_PUSH_RECORD".into());}
        let credential=access.resolve(&record.endpoint)?;
        let advertisement=network_worker::run_push_preview_with_credential(&record.endpoint,credential.as_ref())?;
        repository::push_state::observe(&root,&id,&advertisement.advertisement)?;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.snapshot().map(Some)
    }).await.map_err(|_|"PUSH_RESULT_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn acknowledge_push(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    id: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let root = current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .root_path()
        .to_path_buf();
        let record = repository::push_state::read(&root)?.ok_or("NO_PUSH_RECORD")?;
        if record.id != id {
            return Err("STALE_PUSH_RECORD".into());
        }
        if !app
            .dialog()
            .message(if record.status==repository::push_state::Status::Pending {"Push 성공 여부는 여전히 미확정입니다. 원격 조회 결과와 전송 기록을 보관하고 차단을 해제합니다. 재전송하거나 원격 변경을 취소하지 않습니다."}else{"확인된 Push 결과 기록을 보관하고 닫습니다. 원격 변경을 취소하지 않습니다."})
            .parent(&window)
            .title("Close Push result")
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
        {
            return Ok(None);
        }
        if record.status==repository::push_state::Status::Pending {repository::push_state::acknowledge_observed(&root,&id)?;}else{repository::push_state::acknowledge(&root,&id)?;}
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .snapshot()
        .map(Some)
    })
    .await
    .map_err(|_| "PUSH_RECORD_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn preview_push(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
    branch: String,
) -> Result<Option<repository::push::Plan>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=guard;
        let selected=repository::push::selection(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&branch)?;
        let Some(access)=network_access::confirm(&window,&app,&selected.endpoint,"Preview Push",format!("{}\n{} → {}\n\nPush 대상 서버의 브랜치 정보를 조회합니다. 커밋 전송이나 원격 변경은 하지 않습니다.",selected.endpoint,selected.source,selected.destination))? else {return Ok(None);};
        if repository::push::selection(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&branch)?!=selected {return Err("STALE_PUSH_SELECTION".into());}
        let credential=access.resolve(&selected.endpoint)?;
        let advertisement=network_worker::run_push_preview_with_credential(&selected.endpoint,credential.as_ref())?;
        let mut plan=repository::push::preview(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&selected,&advertisement.advertisement)?;
        if !advertisement.supports_report_status() {plan.blockers.push("PUSH_REPORT_STATUS_REQUIRED".into());}
        Ok(Some(plan))
    }).await.map_err(|_|"PUSH_PREVIEW_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn prepare_pull(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
    mode: Option<repository::pull::Mode>,
    auto_prune: Option<bool>,
) -> Result<Option<repository::pull::Prepared>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let selected = repository::pull::selection_with_mode(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name, mode)?;
        let endpoint = repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name)?;
        let Some(access)=network_access::confirm(&window,&app,&endpoint,"Pull upstream",format!("{endpoint}\nUpstream: {}\nMode: {}\n\nFetch로 설정된 원격 추적 참조·태그를 갱신한 뒤 선택된 병합 또는 rebase 미리보기를 엽니다. 로컬 브랜치와 작업 파일은 미리보기에서 적용할 때 변경됩니다. 미리보기를 닫아도 Fetch 결과는 유지됩니다.", selected.source, selected.mode_label()))? else {return Ok(None);};
        repository::pull::selection_with_mode(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name, mode)?;
        let credential=access.resolve(&endpoint)?;
        let download = network_worker::run_download_with_credential(&endpoint,credential.as_ref())?;
        let prune=if auto_prune.unwrap_or(false){
            let plan=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.prune_preview(&revision,&name,download.advertisement())?;
            if !plan.removed_refs.is_empty(){
                let lines=plan.removed_refs.iter().take(30).cloned().collect::<Vec<_>>().join("\n");
                if !app.dialog().message(format!("{endpoint}\n\nAuto-Prune: {}개 원격 추적 참조를 Fetch 후 정리합니다.\n{lines}\n\n계속하면 Fetch 및 정리 후 Pull 미리보기를 엽니다. 취소하면 저장소 반영을 시작하지 않습니다.",plan.removed_refs.len())).parent(&window).title("Pull Auto-Prune").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(None);}
            }
            Some(plan.removed_refs)
        }else{None};
        repository::pull::prepare_with_options(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name, &download, mode, prune.as_deref()).map(Some)
    }).await.map_err(|_| "PULL_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn fetch_remote(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
    auto_prune: Option<bool>,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=guard;
        let endpoint=repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
        let Some(access)=network_access::confirm(&window,&app,&endpoint,"Fetch 다운로드",format!("{endpoint}\n\nGit 객체를 내려받아 검증합니다. 인증 방식은 아래에서 선택합니다. 저장소 반영 전 변경 목록을 확인합니다."))? else {return Ok(None);};
        repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
        let credential=access.resolve(&endpoint)?;
        let download=network_worker::run_download_with_credential(&endpoint,credential.as_ref())?;
        let plan=repository::fetch::preview_download(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&download)?;
        let prune=if auto_prune.unwrap_or(false){Some(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.prune_preview(&revision,&name,download.advertisement())?)}else{None};
        let prune_notice=match &prune{Some(plan)=>format!("Auto-Prune: 삭제된 원격 추적 참조 {}개를 Fetch 후 정리합니다.\n{}",plan.removed_refs.len(),plan.removed_refs.iter().take(30).cloned().collect::<Vec<_>>().join("\n")),None=>"Auto-Prune 꺼짐: 삭제된 추적 브랜치는 별도의 Prune으로 정리할 수 있습니다.".into()};
        let changed=plan.mappings.iter().filter(|m|m.before.as_deref()!=Some(m.after.as_str())).count();
        let lines=plan.mappings.iter().take(30).map(|m|format!("{} → {}",m.source,m.destination.as_deref().unwrap_or("FETCH_HEAD"))).collect::<Vec<_>>().join("\n");
        if !app.dialog().message(format!("{endpoint}\n\n{lines}\n\n총 {}개 선택, {}개 변경 후보. 설정된 원격 추적 참조·태그와 FETCH_HEAD를 반영합니다. 작업 파일·로컬 브랜치는 유지합니다. 강제 refspec으로 지정된 기존 태그는 교체될 수 있습니다. 태그는 원격 설정의 자동 추적/전체/제외 정책을 따릅니다. 원격 HEAD는 별도의 HEAD 설정으로 변경할 수 있습니다. {prune_notice}",plan.mappings.len(),changed)).parent(&window).title("Fetch 반영").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(None);}
        let fetched=repository::fetch::execute(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name,&download)?;
        if prune.as_ref().is_some_and(|plan|!plan.removed_refs.is_empty()){
            let result=(||->Result<Snapshot,String>{
                let store=app.state::<Store>();
                let mut locked=store.lock().map_err(|_|"LOCK_ERROR")?;
                let session=current(&mut locked,&repo_id)?;
                let approved=prune.as_ref().ok_or("MISSING_PRUNE_PLAN")?;
                repository::fetch::prune_confirmed(session,fetched,&name,&download,&approved.removed_refs)
            })();
            return result.map(Some).map_err(|error|format!("FETCH_COMPLETED_PRUNE_FAILED: Fetch completed, but Auto-Prune failed. Refresh before retrying prune. {error}"));
        }
        Ok(Some(fetched))
    }).await.map_err(|_|"REMOTE_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn set_remote_head(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
    branch: Option<String>,
    auto: bool,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    if auto && branch.is_some() {
        return Err("INVALID_REMOTE_HEAD_MODE".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=guard;
        let branch=if auto {
            let endpoint=repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
            let Some(access)=network_access::confirm(&window,&app,&endpoint,"원격 HEAD 조회",format!("{endpoint}\n\n서버의 기본 브랜치를 조회합니다. 인증 방식은 아래에서 선택합니다."))? else {return Ok(None);};
            repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&name)?;
            let credential=access.resolve(&endpoint)?;
        Some(repository::remote_head::advertised_branch(&network_worker::run_with_credential(&endpoint,credential.as_ref())?)?)
        } else {branch};
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.remote_head_preview(&revision,&name,branch.as_deref())?;
        let target=branch.as_ref().map(|b|format!("refs/remotes/{name}/{b}")).unwrap_or_else(||"해제".into());
        if !app.dialog().message(format!("refs/remotes/{name}/HEAD → {target}\n\n로컬 원격 HEAD만 변경합니다. 서버의 기본 브랜치, 현재 로컬 브랜치와 작업 파일은 유지합니다.")).parent(&window).title("원격 HEAD 변경").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(None);}
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.set_remote_head(&revision,&name,branch.as_deref()).map(Some)
    }).await.map_err(|_|"REMOTE_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn prune_remote(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let endpoint = repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name)?;
        let Some(access)=network_access::confirm(&window,&app,&endpoint,"Prune 조회",format!("{endpoint}\n\n원격 참조 목록을 조회하여 삭제된 추적 브랜치를 찾습니다. 인증 방식은 아래에서 선택합니다."))? else {return Ok(None);};
        repository::remote_network::endpoint(current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?, &revision, &name)?;
        let credential=access.resolve(&endpoint)?;
        let advertisement = network_worker::run_with_credential(&endpoint,credential.as_ref())?;
        let plan = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.prune_preview(&revision, &name, &advertisement)?;
        let lines = plan.removed_refs.iter().take(30).cloned().collect::<Vec<_>>().join("\n");
        if !app.dialog().message(format!("{endpoint}\n\n{lines}\n\n삭제 대상 {}개. 로컬 브랜치·태그·공유 참조·심볼릭 HEAD·설정·작업 파일은 보존합니다. 서버에는 변경하지 않습니다. 조회 시점의 원격 목록을 기준으로 처리합니다.", plan.removed_refs.len())).parent(&window).title("Prune 반영").buttons(MessageDialogButtons::OkCancel).blocking_show() { return Ok(None); }
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.prune_remote(&revision, &name, &advertisement).map(Some)
    }).await.map_err(|_| "REMOTE_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn recent_repositories(
    window: WebviewWindow,
    app: tauri::AppHandle,
    forget: Option<String>,
) -> Result<RecentRepositories, String> {
    window_check(&window)?;
    if forget.as_ref().is_some_and(|p| p.len() > 4096) {
        return Err("INVALID_PATH".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let paths = match forget {
            Some(path) => recent::forget(&dir, &path)?,
            None => recent::read(&dir)?,
        };
        let warning = app
            .state::<Mutex<Option<String>>>()
            .lock()
            .map_err(|_| "LOCK_ERROR")?
            .clone();
        Ok(RecentRepositories { paths, warning })
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn choose_worktree_parent(window: WebviewWindow, app: tauri::AppHandle) -> Result<Option<String>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let Some(path) = app.dialog().file().set_parent(&window)
            .set_title("Choose parent folder for the new worktree")
            .blocking_pick_folder() else { return Ok(None); };
        let path = path.into_path().map_err(|_| "INVALID_PATH")?;
        let path = path.to_str().ok_or("UNSUPPORTED_PATH")?;
        Ok(Some(path.to_owned()))
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn choose_repository(
    window: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let Some(path) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("로컬 Git 저장소 선택")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(|_| "INVALID_PATH")?;
        confirm_repository(&window, &app, path)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn open_repository_path(
    window: WebviewWindow,
    app: tauri::AppHandle,
    path: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let path = repository_path(&path)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        confirm_repository(&window, &app, path)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn clone_repository_path(
    window: WebviewWindow,
    app: tauri::AppHandle,
    path: String,
    url: String,
    branch: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let target = clone_repository::destination(&path)?;
    let endpoint = repository::remote_endpoint::https(&url)?.to_string();
    clone_repository::validate_branch(&branch)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        if app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?.len() >= MAX_SESSIONS { return Err("TAB_LIMIT".into()); }
        let selection = if branch.is_empty() { "원격 기본 브랜치" } else { &branch };
        let Some(access)=network_access::confirm(&window,&app,&endpoint,"Clone repository",format!("{endpoint}\n→ {}\n\n체크아웃: {selection}\nHTTPS 저장소를 새 폴더로 복제합니다. 기존 경로는 대체하지 않습니다. 심볼릭 링크·submodule·속성 필터가 필요한 작업 트리는 아직 지원하지 않습니다.", target.display()))? else {return Ok(None);};
        let credential=access.resolve(&endpoint)?;
        let download = network_worker::run_download_with_credential(&endpoint,credential.as_ref())?;
        let snapshot = clone_repository::create(app.state::<Store>().inner(), &target, &download, &branch)?;
        remember_repository(&app, &target);
        Ok(Some(snapshot))
    }).await.map_err(|_| "CLONE_WORKER_FAILED".to_string())?
}
#[tauri::command]
async fn init_repository_path(
    window: WebviewWindow,
    app: tauri::AppHandle,
    path: String,
    branch: String,
) -> Result<Option<Snapshot>, String> {
    window_check(&window)?;
    let path = init_repository::validate(&path, &branch)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        if !app.dialog().message(format!("{}\n\n초기 브랜치: {}\n새 폴더에 로컬 저장소를 생성합니다. 기존 경로는 덮어쓰지 않습니다.", path.display(), branch))
            .parent(&window).title("저장소 생성").buttons(MessageDialogButtons::OkCancel).blocking_show() {
            return Ok(None);
        }
        let snapshot = init_repository::create(app.state::<Store>().inner(), &path, &branch)?;
        remember_repository(&app, &path);
        Ok(Some(snapshot))
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_rebase_draft(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
) -> Result<repository::rebase::draft::View, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .rebase_draft_preview(&revision)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_rebase(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    upstream: String,
) -> Result<repository::rebase::Plan, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .rebase_preview(&revision, &upstream)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_replay_sequence(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    selections: Vec<repository::ReplaySelection>,
) -> Result<repository::ReplaySequencePlan, String> {
    window_check(&window)?;
    if selections.is_empty() || selections.len() > 32 {
        return Err("INVALID_SEQUENCE_LENGTH".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .replay_sequence_preview(&revision, &selections)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_replay(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    oid: String,
    kind: ReplayKind,
    mainline: Option<usize>,
) -> Result<ReplayPlan, String> {
    window_check(&window)?;
    if oid.len() != 40 || !oid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("INVALID_COMMIT".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .replay_preview(&revision, &oid, kind, mainline)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_merge(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
) -> Result<MergePlan, String> {
    window_check(&window)?;
    if name.len() > 1031 {
        return Err("INVALID_BRANCH".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .merge_preview(&revision, &name)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn open_worktree_terminal(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String, name: String) -> Result<bool, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let path = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.worktree_path(&revision, &name)?;
        if !app.dialog().message(format!("{}\n\nOpen a terminal in this linked worktree folder. Your normal shell startup settings apply.", path.display())).parent(&window).title("Open worktree terminal").buttons(MessageDialogButtons::OkCancel).blocking_show() { return Ok(false); }
        let confirmed = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.worktree_path(&revision, &name)?;
        if confirmed != path { return Err("STALE_WORKTREE_PATH".into()); }
        terminal::open(&path)?;
        Ok(true)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn reveal_worktree_path(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,name:String)->Result<(),String>{
    window_check(&window)?;
    if name.len()>80{return Err("INVALID_WORKTREE_NAME".into());}
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let path=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.worktree_path(&revision,&name)?;
        file_manager::reveal(&path,false)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn reveal_repository_path(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,file_id:Option<String>)->Result<(),String>{
    window_check(&window)?;
    if file_id.as_ref().is_some_and(|id|id.len()>128){return Err("INVALID_FILE_ID".into());}
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let (path,select)=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.reveal_path(&revision,file_id.as_deref())?;
        file_manager::reveal(&path,select)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
fn available_external_editors(window: WebviewWindow) -> Result<Vec<external_editor::AvailableEditor>, String> {
    window_check(&window)?;
    Ok(external_editor::available())
}
#[tauri::command]
async fn open_repository_editor(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, editor: external_editor::Editor) -> Result<(), String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let root = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.root_path().to_path_buf();
        external_editor::open(&root, editor)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn open_file_in_editor(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String, file_id: String, editor: external_editor::Editor) -> Result<(), String> {
    window_check(&window)?;
    if file_id.len() > 128 { return Err("INVALID_FILE_ID".into()); }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let path = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.editor_file_path(&revision, &file_id)?;
        external_editor::open_file(&path, editor)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn open_repository_terminal(window: WebviewWindow, app: tauri::AppHandle, repo_id: String) -> Result<(), String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let root = current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.root_path().to_path_buf();
        terminal::open(&root)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_snapshot(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
) -> Result<Snapshot, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .snapshot()
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn remove_worktree_archived(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,name:String,token:String)->Result<Option<repository::worktrees::RemovalReceipt>,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let preview={let store=app.state::<Store>();let mut sessions=store.lock().map_err(|_|"LOCK_ERROR")?;
            let preview=current(&mut sessions,&repo_id)?.worktree_removal_preview(&revision,&name)?;
            if sessions.iter().any(|session|session.root_path().starts_with(std::path::Path::new(&preview.path))){return Err("WORKTREE_OPEN_IN_APP".into());}preview};
        if preview.token.as_deref()!=Some(token.as_str()){return Err("WORKTREE_REMOVAL_CHANGED".into());}
        if !app.dialog().message(format!("Remove worktree {name} at {}?\n\nThe checkout will move to a sibling recovery folder, and registration metadata will be archived. Branches and commits remain. Backups consume disk space; no permanent deletion occurs.",preview.path)).parent(&window).title("Remove this worktree").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(None);}
        let store=app.state::<Store>();let mut sessions=store.lock().map_err(|_|"LOCK_ERROR")?;
        if sessions.iter().any(|session|session.root_path().starts_with(std::path::Path::new(&preview.path))){return Err("WORKTREE_OPEN_IN_APP".into());}
        current(&mut sessions,&repo_id)?.remove_worktree_archived(&revision,&name,&token).map(Some)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_worktree_removal(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,name:String)->Result<repository::worktrees::RemovalPreview,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let store=app.state::<Store>();let mut sessions=store.lock().map_err(|_|"LOCK_ERROR")?;
        let mut preview=current(&mut sessions,&repo_id)?.worktree_removal_preview(&revision,&name)?;
        if sessions.iter().any(|session|session.root_path().starts_with(std::path::Path::new(&preview.path))){preview.blockers.push("Close repository tabs inside this worktree before removal.".into());preview.token=None;}
        Ok(preview)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_worktree_checkout(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,name:String)->Result<repository::checkout_status::Checkout,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.worktree_checkout(&revision,&name)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_submodule_checkout(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,path:String)->Result<repository::submodules::Checkout,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.submodule_checkout(&revision,&path)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn resolve_submodule_path(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,path:String)->Result<String,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let path=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.submodule_path(&revision,&path)?;
        path.to_str().map(str::to_owned).ok_or_else(||"UNSUPPORTED_PATH".into())
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_submodules(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String) -> Result<repository::submodules::Listing, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.submodules(&revision)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn search_commit_files(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,query:String)->Result<repository::tree_browser::Search,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        repository::tree_browser::search(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&oid,&query)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_commit_tree(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,path:String,offset:Option<usize>)->Result<repository::tree_browser::Listing,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        repository::tree_browser::read(current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?,&revision,&oid,&path,offset.unwrap_or(0))
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_tag_detail(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String, name: String) -> Result<repository::tag_detail::Detail, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.tag_detail(&revision, &name)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_gitflow_finish(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,kind:repository::gitflow::FlowKind,source:String)->Result<repository::gitflow::FinishPreview,String>{
 window_check(&window)?;
 let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{let _guard=guard;current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.gitflow_finish_preview(&revision,kind,&source)}).await.map_err(|_|"WORKER_ERROR".to_string())?
}
fn confirm_lfs_upload_action(window:&WebviewWindow,app:&tauri::AppHandle,action:&repository::lfs::protocol::Action,authenticated:bool,title:&str,message:String)->Result<Option<Option<network_worker::auth::TransportCredential>>,String>{
 action.ensure_current()?;
 let url=action.url.as_str();
 if action.url.query().is_none()&&!action.headers.contains_key(reqwest::header::AUTHORIZATION)&&!authenticated{
  match network_access::confirm(window,app,url,title,message)?{Some(choice)=>Ok(Some(choice.resolve(url)?)),None=>Ok(None)}
 }else{
  if app.dialog().message(format!("{url}\n\n{message}\nServer-supplied signed URL/headers are used only for this exact destination. Batch credentials are not forwarded."))
   .parent(window).title(title).buttons(MessageDialogButtons::OkCancel).blocking_show(){Ok(Some(None))}else{Ok(None)}
 }
}
#[tauri::command]
async fn upload_lfs_object(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,size:String,endpoint:String,request_id:String)->Result<Option<String>,String>{
 window_check(&window)?;
 let endpoint=repository::remote_endpoint::https(&endpoint)?.to_string();
 let operation=OperationGuard::acquire(&app)?;
 let transfer=app.state::<lfs_transfer::Transfers>().begin(request_id)?;
 let result=tauri::async_runtime::spawn_blocking(move||{
  let _operation=operation;transfer.check()?;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  repository::lfs::upload_state::ensure_ready(&root)?;
  let object=repository::lfs::protocol::Object{oid:oid.clone(),size:size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?};
  let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Upload Git LFS object",format!("Request upload of cached object {oid} ({size} bytes) to this LFS server. The upload and optional verification destinations will be confirmed separately. No branch, commit, index or working file is changed."))? else{return Ok(None)};
  transfer.check()?;let credential=choice.resolve(&endpoint)?;
  let source=repository::lfs::cache::upload_source(&root,&object,&||transfer.cancelled())?;
  current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  transfer.check()?;
  let plan=repository::lfs::transport::batch(&endpoint,&object,repository::lfs::protocol::Operation::Upload,credential.as_ref());
  transfer.check()?;let plan=plan?;
  if plan.already_present{return Ok(Some("remotePresent".into()));}
  let action=plan.transfer.ok_or("LFS_UPLOAD_ACTION_MISSING")?;
  let Some(upload_credential)=confirm_lfs_upload_action(&window,&app,&action,plan.authenticated,"Confirm LFS upload destination",format!("Send the full {size}-byte cached file with SHA-256 {oid} to this destination. This changes remote object storage; interrupted writes have an unknown outcome and are not retried automatically."))? else{return Ok(None)};
  transfer.check()?;
  let verify_credential=if let Some(verify)=plan.verify.as_ref(){
   let Some(credential)=confirm_lfs_upload_action(&window,&app,verify,plan.authenticated,"Confirm LFS verification destination",format!("After upload, send SHA-256 {oid} and size {size} to this verification endpoint. The file contents are not sent to this endpoint."))? else{return Ok(None)};
   credential
  }else{None};
  transfer.check()?;
  current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  if let Some(verify)=plan.verify.as_ref(){verify.ensure_current()?;}
  let mut journal=repository::lfs::upload_state::Pending::begin(&root,&object,&endpoint,plan.verify.is_some())?;
  if let Err(error)=repository::lfs::transport::upload(&object,source,&action,action.url.as_str(),upload_credential.as_ref(),transfer.flag()){
   if error=="LFS_TRANSFER_CANCELLED"{journal.mark(repository::lfs::upload_state::Status::NotSent)?;}
   return Err(error);
  }
  journal.mark(repository::lfs::upload_state::Status::Accepted).map_err(|_|"LFS_UPLOAD_ACCEPTED_RECORD_FAILED: remote accepted upload but local outcome recording failed; inspect before retrying")?;
  if let Some(verify)=plan.verify.as_ref(){
   if transfer.cancelled(){return Err("LFS_UPLOAD_SENT_VERIFY_PENDING: upload accepted; verification cancelled; inspect remote state before retrying".into());}
   repository::lfs::transport::verify_upload(&object,verify,verify.url.as_str(),verify_credential.as_ref()).map_err(|error|format!("LFS_UPLOAD_SENT_VERIFY_PENDING: upload accepted; {error}; do not assume verification succeeded"))?;
   journal.mark(repository::lfs::upload_state::Status::Verified).map_err(|_|"LFS_UPLOAD_VERIFIED_RECORD_FAILED: remote verification succeeded but local outcome recording failed")?;
   Ok(Some("uploadedVerified".into()))
  }else{Ok(Some("uploaded".into()))}
 }).await.map_err(|_|"LFS_UPLOAD_WORKER_FAILED: remote outcome may be unknown; inspect before retrying".to_string())?;
 match result{Err(error) if error=="LFS_TRANSFER_CANCELLED"=>Ok(Some("cancelled".into())),other=>other}
}
#[tauri::command]
async fn preview_lfs_checkout(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,path:String,oid:String,size:String)->Result<repository::lfs::checkout_preview::Preview,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{let _guard=guard;current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_checkout_preview(&revision,&path,&oid,&size)}).await.map_err(|_|"LFS_CHECKOUT_PREVIEW_FAILED".to_string())?
}
#[tauri::command]
async fn read_lfs_index(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,offset:usize)->Result<repository::lfs::IndexPage,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{let _guard=guard;current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_index_page(&revision,offset)}).await.map_err(|_|"LFS_INDEX_READ_FAILED".to_string())?
}
#[tauri::command]
async fn inspect_lfs_cache(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,size:String)->Result<repository::lfs::ObjectStatus,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{let _guard=guard;current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_object_status(&revision,&oid,&size)}).await.map_err(|_|"LFS_CACHE_INSPECTION_FAILED".to_string())?
}
#[tauri::command]
async fn read_lfs_upload_record(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String)->Result<Option<repository::lfs::upload_state::Record>,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{
  let _guard=guard;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_review_root(&revision)?;
  repository::lfs::upload_state::read(&root)
 }).await.map_err(|_|"LFS_RECORD_READ_FAILED".to_string())?
}
#[tauri::command]
async fn inspect_lfs_upload_remote(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,record_id:String)->Result<Option<repository::lfs::upload_state::Observation>,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{
  let _guard=guard;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_review_root(&revision)?;
  let record=repository::lfs::upload_state::read(&root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
  if record.id!=record_id{return Err("STALE_LFS_UPLOAD_RECORD".into());}
  let Some(choice)=network_access::confirm(&window,&app,&record.endpoint,"Check LFS server object status",format!("Ask the recorded LFS server whether object {} ({} bytes) still needs uploading. Only its ID and size are sent in a Batch request. No object PUT, verification callback or automatic retry is performed.",record.oid,record.size))? else{return Ok(None)};
  let credential=choice.resolve(&record.endpoint)?;
  current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_review_root(&revision)?;
  let check=repository::lfs::upload_state::Check::begin(&root,&record)?;
  let object=repository::lfs::protocol::Object{oid:record.oid.clone(),size:record.size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?};
  let plan=repository::lfs::transport::batch(&record.endpoint,&object,repository::lfs::protocol::Operation::Upload,credential.as_ref())?;
  check.finish(plan.already_present).map(Some)
 }).await.map_err(|_|"LFS_REMOTE_INSPECTION_FAILED".to_string())?
}
#[tauri::command]
async fn acknowledge_lfs_upload(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,record_id:String)->Result<bool,String>{
 window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
 tauri::async_runtime::spawn_blocking(move||{
  let _guard=guard;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_review_root(&revision)?;
  let record=repository::lfs::upload_state::read(&root)?.ok_or("NO_LFS_UPLOAD_RECORD")?;
  if record.id!=record_id||!record.unresolved(){return Err("STALE_LFS_UPLOAD_RECORD".into());}
  if !app.dialog().message(format!("LFS server: {}\nObject: {}\nSize: {} bytes\n\nThe previous upload outcome has not been fully confirmed. Continue only after reviewing the remote object state. This acknowledges the local record; it does not verify, resend or delete the remote object.",record.endpoint,record.oid,record.size)).parent(&window).title("Acknowledge LFS upload outcome").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(false);}
  current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_review_root(&revision)?;
  repository::lfs::upload_state::acknowledge(&root,&record_id)?;Ok(true)
 }).await.map_err(|_|"LFS_REVIEW_WORKER_FAILED".to_string())?
}
#[tauri::command]
fn cancel_lfs_download(window:WebviewWindow,app:tauri::AppHandle,request_id:String)->Result<bool,String>{
 window_check(&window)?;
 // Cancellation must remain callable while the download owns OperationGuard.
 app.state::<lfs_transfer::Transfers>().cancel(&request_id)
}
#[tauri::command]
async fn download_lfs_object(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,size:String,endpoint:String,request_id:String)->Result<Option<String>,String>{
 window_check(&window)?;
 let endpoint=repository::remote_endpoint::https(&endpoint)?.to_string();
 let operation=OperationGuard::acquire(&app)?;
 let transfer=app.state::<lfs_transfer::Transfers>().begin(request_id)?;
 let result=tauri::async_runtime::spawn_blocking(move||{
  let _operation=operation;
  transfer.check()?;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  let object=repository::lfs::protocol::Object{oid:oid.clone(),size:size.parse().map_err(|_|"INVALID_LFS_OBJECT_SIZE")?};
  if repository::lfs::cache::verify_if_present(&root,&object,&||transfer.cancelled())?{
   current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
   transfer.check()?;return Ok(Some("alreadyPresent".into()));
  }
  let Some(choice)=network_access::confirm(&window,&app,&endpoint,"Download Git LFS object",format!("Request LFS object {oid} ({size} bytes) from this LFS server. The object download URL will be confirmed separately. Only the local object cache is written; checkout files are unchanged."))? else{return Ok(None)};
  transfer.check()?;
  let credential=choice.resolve(&endpoint)?;
  current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  transfer.check()?;
  let plan=repository::lfs::transport::batch(&endpoint,&object,repository::lfs::protocol::Operation::Download,credential.as_ref());
  transfer.check()?;
  let plan=plan?;
  let action=plan.transfer.ok_or("LFS_DOWNLOAD_ACTION_MISSING")?;
  action.ensure_current()?;
  let url=action.url.as_str().to_string();
  let object_credential=if action.url.query().is_none()&&!action.headers.contains_key(reqwest::header::AUTHORIZATION)&&!plan.authenticated {
   let Some(choice)=network_access::confirm(&window,&app,&url,"Download LFS object bytes",format!("Download {size} bytes for SHA-256 {oid} into this repository's standard LFS cache. Credentials from the batch server are not forwarded."))? else{return Ok(None)};
   transfer.check()?;
   choice.resolve(&url)?
  }else{
   if !app.dialog().message(format!("{url}\n\nDownload {size} bytes for SHA-256 {oid}. Server-supplied signed URL/headers are used only for this exact destination. Git credentials are not forwarded. No checkout files are changed."))
    .parent(&window).title("Confirm LFS object destination").buttons(MessageDialogButtons::OkCancel).blocking_show(){return Ok(None);}
   None
  };
  transfer.check()?;
  let root=current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.lfs_transfer_root(&revision,&oid,&size)?;
  let result=repository::lfs::transport::download(&root,&object,&action,&url,object_credential.as_ref(),&||transfer.cancelled())?;
  Ok(Some(match result{repository::lfs::cache::Published::Created=>"created",repository::lfs::cache::Published::AlreadyPresent=>"alreadyPresent"}.into()))
 }).await.map_err(|_|"LFS_WORKER_FAILED: inspect the object cache before retrying".to_string())?;
 match result{Err(error) if error=="LFS_TRANSFER_CANCELLED"=>Ok(Some("cancelled".into())),other=>other}
}
#[tauri::command]
async fn read_gitflow_settings(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String) -> Result<repository::gitflow::Settings, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.gitflow_settings(&revision)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_worktrees(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String) -> Result<repository::worktrees::Listing, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.worktrees(&revision)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_checkpoints(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String) -> Result<repository::checkpoints::Listing, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.checkpoints(&revision)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn search_commits(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String, query: String) -> Result<repository::search::Results, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.search_commits(&revision, &query)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_graph_at(window: WebviewWindow, app: tauri::AppHandle, repo_id: String, revision: String, offset: usize) -> Result<Snapshot, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(&mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?, &repo_id)?.graph_at(&revision, offset)
    }).await.map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn set_graph_page_size(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,size:usize)->Result<Snapshot,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.set_graph_page_size(&revision,size)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn extend_graph(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String)->Result<Snapshot,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.extend_graph(&revision)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_graph_page(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    direction: repository::GraphPageDirection,
) -> Result<Snapshot, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .graph_page(&revision, direction)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_diff(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    file_id: String,
    staged: bool,
) -> Result<DiffResult, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .diff(&revision, &file_id, staged)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_commit(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    oid: String,
    file_id: Option<String>,
) -> Result<CommitDetail, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .commit_detail(&revision, &oid, file_id.as_deref())
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompareRequest {
    base_oid: String,
    target_oid: String,
    file_id: Option<String>,
}
#[tauri::command]
async fn compare_commits(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    request: CompareRequest,
) -> Result<repository::compare::Comparison, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        repository::compare::read(
            current(
                &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
                &repo_id,
            )?,
            &revision,
            &request.base_oid,
            &request.target_oid,
            request.file_id.as_deref(),
        )
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryRequest {
    file_id: String,
    anchor: Option<String>,
    offset: usize,
    selected: Option<String>,
}
// Keep the established named IPC parameters; two arguments are injected Tauri handles.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn read_tree_file_history(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,path:String,offset:usize,selected:Option<String>)->Result<FileHistory,String>{
    window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{let _guard=guard;current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.tree_file_history(&revision,&oid,&path,offset,selected.as_deref())}).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_file_history(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    request: HistoryRequest,
) -> Result<FileHistory, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .file_history(
            &revision,
            &request.file_id,
            request.anchor.as_deref(),
            request.offset,
            request.selected.as_deref(),
        )
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_tree_file_blame(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,oid:String,path:String)->Result<FileBlame,String>{
    window_check(&window)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.tree_file_blame(&revision,&oid,&path)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn check_patch_preview(window:WebviewWindow,app:tauri::AppHandle,repo_id:String,revision:String,text:String,target:repository::patch_check::Target)->Result<repository::patch_check::Check,String>{
    window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        current(&mut app.state::<Store>().lock().map_err(|_|"LOCK_ERROR")?,&repo_id)?.check_patch(&revision,&text,target)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn open_patch_preview(window:WebviewWindow,app:tauri::AppHandle)->Result<Option<patch_preview::Preview>,String>{
    window_check(&window)?;let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let Some(file)=app.dialog().file().set_parent(&window).set_title("Open patch for review (no repository changes)").add_filter("Git patch",&["patch","diff"]).blocking_pick_file()else{return Ok(None);};
        let path=file.into_path().map_err(|_|"INVALID_PATCH_PATH")?;
        patch_preview::read(&path).map(Some)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn export_patch(window:WebviewWindow,app:tauri::AppHandle,text:String,truncated:bool)->Result<Option<String>,String>{
    window_check(&window)?;
    patch_export::validate(&text,truncated)?;
    let guard=OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=guard;
        let Some(file)=app.dialog().file().set_parent(&window).set_title("Save new patch file (existing files are not replaced)").set_file_name("changes.patch").add_filter("Git patch",&["patch"]).blocking_save_file()else{return Ok(None);};
        let mut path=file.into_path().map_err(|_|"INVALID_PATCH_PATH")?;
        if path.extension().is_none(){path.set_extension("patch");}
        patch_export::save(&path,&text,truncated).map(Some)
    }).await.map_err(|_|"WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_file_blame(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    request: HistoryRequest,
) -> Result<FileBlame, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .file_blame(
            &revision,
            &request.file_id,
            request.anchor.as_deref(),
            request.offset,
            request.selected.as_deref().ok_or("INVALID_HISTORY_ENTRY")?,
        )
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentRequest {
    file_id: String,
    source: ContentSource,
}
#[tauri::command]
async fn read_file_content(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    request: ContentRequest,
) -> Result<FileContent, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .file_content(&revision, &request.file_id, request.source)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn read_stash(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    oid: String,
    area: Option<StashArea>,
    file_id: Option<String>,
) -> Result<StashDetail, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .stash_detail(&revision, &oid, area, file_id.as_deref())
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn mutate(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    action: Action,
) -> Result<Snapshot, String> {
    window_check(&window)?;
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let label = {
            let store = app.state::<Store>();
            let mut state = store.lock().map_err(|_| "LOCK_ERROR")?;
            current(&mut state, &repo_id)?.mutation_label(&action)?
        };
        if !app
            .dialog()
            .message(format!("{label}\n\n이 로컬 저장소 변경을 실행할까요?"))
            .parent(&window)
            .title("Git 작업 확인")
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
        {
            return Err("CANCELLED: 작업을 취소했습니다.".into());
        }
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .mutate(&revision, action)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
async fn preview_remote_remove(
    window: WebviewWindow,
    app: tauri::AppHandle,
    repo_id: String,
    revision: String,
    name: String,
) -> Result<repository::RemoteRemovePreview, String> {
    window_check(&window)?;
    if name.len() > 200 {
        return Err("INVALID_REMOTE_NAME".into());
    }
    let guard = OperationGuard::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        current(
            &mut app.state::<Store>().lock().map_err(|_| "LOCK_ERROR")?,
            &repo_id,
        )?
        .remote_remove_preview(&revision, &name)
    })
    .await
    .map_err(|_| "WORKER_ERROR".to_string())?
}
#[tauri::command]
fn register_shutdown_handler(window: WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    window_check(&window)?;
    app.state::<Mutex<shutdown::Gate>>()
        .lock()
        .map_err(|_| "LOCK_ERROR")?
        .registered = true;
    Ok(())
}
#[tauri::command]
fn finish_shutdown(
    window: WebviewWindow,
    app: tauri::AppHandle,
    id: String,
    saved: bool,
) -> Result<(), String> {
    window_check(&window)?;
    let busy = app.state::<Arc<AtomicBool>>().load(Ordering::SeqCst);
    let exit = app
        .state::<Mutex<shutdown::Gate>>()
        .lock()
        .map_err(|_| "LOCK_ERROR")?
        .finish(&id, saved, busy)?;
    #[cfg(target_os = "macos")]
    if let Err(error) = macos_termination::finish(&app, exit) {
        if let Ok(mut gate) = app.state::<Mutex<shutdown::Gate>>().lock() {
            gate.allowed = false;
        }
        return Err(error);
    }
    #[cfg(not(target_os = "macos"))]
    if exit {
        app.exit(0);
    }
    if !exit && saved && busy {
        return Err("BUSY: Finish the current operation before quitting.".into());
    }
    Ok(())
}
fn request_shutdown(app: &tauri::AppHandle) -> bool {
    let state = app.state::<Mutex<shutdown::Gate>>();
    let Ok(mut gate) = state.lock() else {
        return true;
    };
    if gate.allowed {
        return false;
    }
    // Before the UI can open a repository, no composer exists to flush.
    if !gate.registered
        && app.state::<Store>().lock().is_ok_and(|s| s.is_empty())
        && !app.state::<Arc<AtomicBool>>().load(Ordering::SeqCst)
    {
        return false;
    }
    if let Some(id) = gate.request() {
        if let Some(window) = app.get_webview_window("main") {
            if window
                .emit("branchdesk://shutdown-request", id.clone())
                .is_err()
            {
                let _ = gate.finish(&id, false, false);
            }
        }
    }
    true
}
#[cfg(target_os = "macos")]
fn application_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let menu = Menu::default(app)?;
    // The default macOS Quit item calls NSApplication.terminate directly, bypassing
    // RunEvent::ExitRequested. Keep standard menus, replace the application menu.
    menu.remove_at(0)?;
    let quit = MenuItem::with_id(
        app,
        "branchdesk.quit",
        "Quit FLOE",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let application = Submenu::with_items(
        app,
        "FLOE",
        true,
        &[
            &P::about(app, Some("About FLOE"), None)?,
            &P::separator(app)?,
            &P::services(app, None)?,
            &P::separator(app)?,
            &P::hide(app, None)?,
            &P::hide_others(app, None)?,
            &P::show_all(app, None)?,
            &P::separator(app)?,
            &quit,
        ],
    )?;
    menu.insert(&application, 0)?;
    Ok(menu)
}
fn main() {
    if let Some(code) = network_worker::entry() {
        std::process::exit(code);
    }
    tauri::Builder::default()
        .manage(Mutex::new(shutdown::Gate::default()))
        .manage(Arc::new(Mutex::new(Vec::<Session>::new())))
        .manage(Arc::new(AtomicBool::new(false)))
        .manage(lfs_transfer::Transfers::default())
        .manage(Mutex::new(None::<String>))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(feature = "developer-test")]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            #[cfg(target_os = "macos")]
            {
                app.set_menu(application_menu(app.handle())?)?;
                macos_termination::install(app.handle())?;
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "branchdesk.quit" {
                app.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_worktree_terminal,
            reveal_worktree_path,
            preview_gitflow_finish,
            read_gitflow_settings,
            download_lfs_object,
            upload_lfs_object,
            acknowledge_lfs_upload,
            inspect_lfs_upload_remote,
            read_lfs_upload_record,
            read_lfs_index,
            preview_lfs_checkout,
            inspect_lfs_cache,
            cancel_lfs_download,
            read_worktrees,
            read_tag_detail,
            read_commit_tree,
            search_commit_files,
            read_submodules,
            resolve_submodule_path,
            read_submodule_checkout,
            read_worktree_checkout,
            preview_worktree_removal,
            remove_worktree_archived,
            read_checkpoints,
            search_commits,
            read_graph_at,
            open_repository_terminal,
            available_external_editors,
            open_repository_editor,
            open_file_in_editor,
            reveal_repository_path,
            read_workspace,
            read_named_workspaces,
            read_open_repository_paths,
            read_workspace_wips,
            save_named_workspaces,
            github_write,
            github_item_detail,
            github_pull_files,
            github_pull_discussion,
            github_item_comments,
            github_metadata_options,
            forget_github_credential,
            github_pull_checks,
            read_github_pull_requests,
            read_github_issues,
            save_workspace,
            register_shutdown_handler,
            finish_shutdown,
            read_commit_draft,
            save_commit_draft,
            read_identity_settings,
            save_identity_settings,
            choose_worktree_parent,
            choose_repository,
            open_repository_path,
            init_repository_path,
            clone_repository_path,
            recent_repositories,
            inspect_remote,
            forget_remote_credential,
            fetch_remote,
            prune_remote,
            set_remote_head,
            prepare_pull,
            preview_push,
            execute_push,
            acknowledge_push,
            check_push_result,
            close_repository,
            read_snapshot,
            read_graph_page,
            extend_graph,
            set_graph_page_size,
            preview_remote_remove,
            preview_merge,
            preview_replay,
            preview_replay_sequence,
            preview_rebase,
            preview_rebase_draft,
            read_diff,
            read_commit,
            compare_commits,
            read_file_history,
            read_tree_file_history,
            read_file_blame,
            export_patch,
            open_patch_preview,
            check_patch_preview,
            read_tree_file_blame,
            read_file_content,
            read_stash,
            mutate
        ])
        .build(tauri::generate_context!())
        .expect("FLOE failed to start")
        .run(|app, event| {
            match &event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    if request_shutdown(app) {
                        api.prevent_exit();
                    }
                }
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "main" && request_shutdown(app) => {
                    api.prevent_close();
                }
                _ => (),
            }

            let reveal = matches!(event, tauri::RunEvent::Ready);
            #[cfg(target_os = "macos")]
            let reveal = reveal || matches!(event, tauri::RunEvent::Reopen { .. });
            if reveal {
                if let Err(error) = reveal_main_window(app) {
                    eprintln!("Unable to reveal main window: {error}");
                }
            }
        });
}

fn reveal_main_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = if let Some(window) = app.get_webview_window("main") {
        window
    } else {
        let config = app
            .config()
            .app
            .windows
            .iter()
            .find(|config| config.label == "main")
            .expect("main window config");
        tauri::WebviewWindowBuilder::from_config(app, config)?.build()?
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

#[cfg(test)]
mod repository_open_tests {
    use super::*;
    #[test]
    fn explicit_paths_are_literal_absolute_and_local() {
        for value in [
            "",
            "relative/repo",
            "~/repo",
            "$HOME/repo",
            "https://example.invalid/repo",
            "//server/share",
            r"\\server\share",
            "/tmp/repo\n",
            "/tmp/\0repo",
        ] {
            assert!(repository_path(value).is_err(), "accepted {value:?}");
        }
        assert!(repository_path(&"x".repeat(4097)).is_err());
        #[cfg(unix)]
        {
            let path = "/tmp/repo with spaces;$(literal)";
            assert_eq!(repository_path(path).unwrap().to_str(), Some(path));
        }
        #[cfg(windows)]
        {
            assert!(repository_path(r"C:\Users\Test\repository").is_ok());
            assert!(repository_path(r"C:relative").is_err());
            assert!(repository_path(r"\\?\C:\repo").is_err());
        }
    }
    #[test]
    fn workspace_resolves_only_live_sessions_and_retains_requested_order() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        git2::Repository::init(one.path()).unwrap();
        git2::Repository::init(two.path()).unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let a = replace_repository(&store, one.path().into()).unwrap();
        let b = replace_repository(&store, two.path().into()).unwrap();
        let ids = vec![b.repo_id.clone(), a.repo_id.clone()];
        let v = workspace_for_sessions(&store.lock().unwrap(), &ids, Some(&a.repo_id)).unwrap();
        assert_eq!(
            v.paths,
            vec![
                two.path().canonicalize().unwrap().to_str().unwrap(),
                one.path().canonicalize().unwrap().to_str().unwrap()
            ]
        );
        assert_eq!(v.active.as_deref(), Some(v.paths[1].as_str()));
        assert!(workspace_for_sessions(
            &store.lock().unwrap(),
            &[a.repo_id.clone(), a.repo_id.clone()],
            None
        )
        .is_err());
        assert!(workspace_for_sessions(&store.lock().unwrap(), &ids, Some("forged")).is_err());
        close_session(&store, &b.repo_id).unwrap();
        assert!(workspace_for_sessions(&store.lock().unwrap(), &ids, None).is_err());
    }
    #[test]
    fn failed_repository_open_preserves_current_session_and_worktree() {
        let good = tempfile::tempdir().unwrap();
        git2::Repository::init(good.path()).unwrap();
        std::fs::write(good.path().join("dirty.txt"), "uncommitted").unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let before = replace_repository(&store, good.path().into()).unwrap();
        let invalid = tempfile::tempdir().unwrap();
        assert!(replace_repository(&store, invalid.path().into()).is_err());
        assert!(replace_repository(&store, invalid.path().join("does not exist")).is_err());
        let after = store.lock().unwrap()[0].snapshot().unwrap();
        assert_eq!(before.repo_id, after.repo_id);
        assert_eq!(before.revision, after.revision);
        assert_eq!(
            std::fs::read_to_string(good.path().join("dirty.txt")).unwrap(),
            "uncommitted"
        );
    }
    #[test]
    fn reopen_reuses_session_close_invalidates_it_without_touching_files() {
        let root = tempfile::tempdir().unwrap();
        git2::Repository::init(root.path()).unwrap();
        std::fs::write(root.path().join("dirty"), "keep").unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let old = replace_repository(&store, root.path().into()).unwrap();
        let same = replace_repository(&store, root.path().join(".")).unwrap();
        assert_eq!(old.repo_id, same.repo_id);
        assert_eq!(store.lock().unwrap().len(), 1);
        close_session(&store, &old.repo_id).unwrap();
        assert!(current(&mut store.lock().unwrap(), &old.repo_id).is_err());
        assert!(close_session(&store, &old.repo_id).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("dirty")).unwrap(),
            "keep"
        );
        let reopened = replace_repository(&store, root.path().into()).unwrap();
        assert_ne!(old.repo_id, reopened.repo_id);
    }
    #[test]
    fn session_limit_preserves_open_tabs_and_refuses_creation_before_mkdir() {
        let root = tempfile::tempdir().unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let mut ids = Vec::new();
        for n in 0..MAX_SESSIONS {
            let path = root.path().join(n.to_string());
            git2::Repository::init(&path).unwrap();
            ids.push(replace_repository(&store, path).unwrap().repo_id);
        }
        let extra = root.path().join("extra");
        git2::Repository::init(&extra).unwrap();
        assert!(replace_repository(&store, extra.clone())
            .unwrap_err()
            .starts_with("TAB_LIMIT:"));
        let new = root.path().join("new");
        assert!(init_repository::create(&store, &new, "main").is_err());
        assert!(!new.exists());
        assert_eq!(
            replace_repository(&store, root.path().join("0"))
                .unwrap()
                .repo_id,
            ids[0]
        );
        close_session(&store, &ids[0]).unwrap();
        replace_repository(&store, extra).unwrap();
        assert_eq!(store.lock().unwrap().len(), MAX_SESSIONS);
    }
    #[test]
    fn sessions_isolate_mutations_and_refresh_external_changes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for p in [a.path(), b.path()] {
            git2::Repository::init(p).unwrap();
            std::fs::write(p.join("file"), "before").unwrap();
        }
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let first = replace_repository(&store, a.path().into()).unwrap();
        let second = replace_repository(&store, b.path().into()).unwrap();
        let mut sessions = store.lock().unwrap();
        current(&mut sessions, &second.repo_id)
            .unwrap()
            .mutate(&second.revision, Action::StageAll)
            .unwrap();
        assert!(!a.path().join(".git/index").exists());
        assert!(b.path().join(".git/index").exists());
        std::fs::write(a.path().join("file"), "external edit").unwrap();
        let session = current(&mut sessions, &first.repo_id).unwrap();
        assert!(session.mutate(&first.revision, Action::StageAll).is_err());
        let refreshed = session.snapshot().unwrap();
        assert_ne!(first.revision, refreshed.revision);
        session
            .mutate(&refreshed.revision, Action::StageAll)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(a.path().join("file")).unwrap(),
            "external edit"
        );
        assert_eq!(
            std::fs::read_to_string(b.path().join("file")).unwrap(),
            "before"
        );
    }
    #[test]
    fn opening_another_repository_preserves_both_session_handles() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        git2::Repository::init(a.path()).unwrap();
        git2::Repository::init(b.path()).unwrap();
        let store: Store = Arc::new(Mutex::new(Vec::new()));
        let old = replace_repository(&store, a.path().into()).unwrap();
        let new = replace_repository(&store, b.path().into()).unwrap();
        assert_ne!(old.repo_id, new.repo_id);
        let mut guard = store.lock().unwrap();
        assert!(current(&mut guard, &old.repo_id).is_ok());
        assert!(current(&mut guard, &new.repo_id).is_ok());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod disk_fault_tests;
