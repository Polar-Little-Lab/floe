//! Fixed editor discovery; renderer preferences never become executable paths or shell text.
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Editor { Vscode }
#[derive(Serialize)]
pub struct AvailableEditor { pub id: &'static str, pub label: &'static str, pub available: bool }
fn executable(editor: Editor) -> Option<PathBuf> {
    match editor { Editor::Vscode => {
        #[cfg(target_os = "macos")]
        {
            let system = PathBuf::from("/Applications/Visual Studio Code.app");
            if system.is_dir() { return Some(system); }
            if let Some(home) = std::env::var_os("HOME") {
                let path = PathBuf::from(home).join("Applications/Visual Studio Code.app");
                if path.is_absolute() && path.is_dir() { return Some(path); }
            }
        }
        #[cfg(target_os = "windows")]
        {
            for (variable, relative) in [("LOCALAPPDATA", "Programs/Microsoft VS Code/Code.exe"), ("ProgramFiles", "Microsoft VS Code/Code.exe")] {
                if let Some(base) = std::env::var_os(variable) {
                    let path = PathBuf::from(base).join(relative);
                    if path.is_absolute() && path.is_file() { return Some(path); }
                }
            }
        }
        None
    }}
}
pub fn available() -> Vec<AvailableEditor> {
    vec![AvailableEditor {id:"vscode",label:"Visual Studio Code",available:executable(Editor::Vscode).is_some()}]
}
pub fn open(root: &Path, editor: Editor) -> Result<(), String> {
    if !root.is_absolute() || !root.is_dir() || root.canonicalize().ok().as_deref() != Some(root) {
        return Err("EDITOR_REPOSITORY_UNAVAILABLE".into());
    }
    launch(root, editor)
}
pub fn open_file(path: &Path, editor: Editor) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() || path.canonicalize().ok().as_deref() != Some(path) {
        return Err("EDITOR_FILE_UNAVAILABLE".into());
    }
    launch(path, editor)
}
fn launch(root: &Path, editor: Editor) -> Result<(), String> {
    let executable = executable(editor).ok_or("EDITOR_NOT_INSTALLED")?;
    #[cfg(target_os = "macos")]
    {
        use std::{process::{Command, Stdio}, time::{Duration, Instant}};
        let mut child = Command::new("/usr/bin/open").arg("-a").arg(executable).arg("--").arg(root)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"EDITOR_LAUNCH_FAILED")?;
        let deadline=Instant::now()+Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return if status.success(){Ok(())}else{Err("EDITOR_LAUNCH_FAILED".into())},
                Ok(None) if Instant::now()<deadline => std::thread::sleep(Duration::from_millis(25)),
                _ => {let _=child.kill();let _=child.wait();return Err("EDITOR_LAUNCH_FAILED".into());}
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::process::{Command, Stdio};
        // Repository is a positional argument, not a .cmd script or interpolated command.
        Command::new(executable).arg("--new-window").arg("--").arg(root)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|_|"EDITOR_LAUNCH_FAILED")?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { let _=executable; Err("EDITOR_PLATFORM_NOT_SUPPORTED".into()) }
}
