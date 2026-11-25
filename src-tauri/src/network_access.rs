//! Native authorization and credential selection. No secrets cross renderer IPC.
use crate::{
    credential_vault::{self, Credential},
    network_worker::auth::TransportCredential,
};

// Native credential entry is macOS-only; other platforms currently offer anonymous access.
enum Mode {
    Anonymous,
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Entered {
        credential: Credential,
        remember: bool,
    },
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Saved,
}
pub(crate) struct Choice {
    endpoint: String,
    mode: Mode,
}
impl Choice {
    pub(crate) fn resolve(self, endpoint: &str) -> Result<Option<TransportCredential>, String> {
        self.resolve_using(endpoint, credential_vault::get, credential_vault::put)
    }
    fn resolve_using(
        self,
        endpoint: &str,
        load: impl FnOnce(&str) -> Result<Option<Credential>, &'static str>,
        save: impl FnOnce(&str, &Credential) -> Result<(), &'static str>,
    ) -> Result<Option<TransportCredential>, String> {
        let canonical = crate::repository::remote_endpoint::https(endpoint)?.to_string();
        if canonical != self.endpoint {
            return Err("REMOTE_CREDENTIAL_SCOPE_MISMATCH".into());
        }
        let credential = match self.mode {
            Mode::Anonymous => return Ok(None),
            Mode::Saved => load(&canonical)?.ok_or("CREDENTIAL_NOT_FOUND")?,
            Mode::Entered {
                credential,
                remember,
            } => {
                if remember {
                    save(&canonical, &credential)?;
                }
                credential
            }
        };
        TransportCredential::new(&canonical, &credential).map(Some)
    }
}

// Return a frozen authorization value so the caller releases the repository
// mutex before entering a synchronous OS Keychain operation.
pub(crate) fn confirmed_credential_endpoint(
    session: &mut crate::repository::Session,
    revision: &str,
    name: &str,
    push: bool,
    index: usize,
    confirmed_endpoint: &str,
) -> Result<String, String> {
    let current = crate::repository::remote_network::credential_endpoint(
        session, revision, name, push, index,
    )?;
    if current != confirmed_endpoint {
        return Err("REMOTE_CREDENTIAL_SCOPE_MISMATCH".into());
    }
    Ok(current)
}
#[cfg(test)]
fn forget_using(
    session: &mut crate::repository::Session,
    revision: &str,
    name: &str,
    push: bool,
    index: usize,
    confirmed_endpoint: &str,
    remove: impl FnOnce(&str) -> Result<(), &'static str>,
) -> Result<(), String> {
    let endpoint =
        confirmed_credential_endpoint(session, revision, name, push, index, confirmed_endpoint)?;
    remove(&endpoint).map_err(str::to_owned)
}

#[cfg(target_os = "macos")]
fn decode_choice(
    endpoint: String,
    status: i32,
    mode: i32,
    remember: bool,
    bytes: &[u8],
) -> Result<Option<Choice>, String> {
    if status == 0 {
        return Ok(None);
    }
    if status != 1 {
        return Err("NATIVE_CREDENTIAL_DIALOG_FAILED".into());
    }
    let mode = match mode {
        0 if bytes.is_empty() && !remember => Mode::Anonymous,
        2 if bytes.is_empty() && !remember => Mode::Saved,
        1 => Mode::Entered {
            credential: Credential::decode(bytes)?,
            remember,
        },
        _ => return Err("INVALID_NATIVE_CREDENTIAL_RESULT".into()),
    };
    Ok(Some(Choice { endpoint, mode }))
}

// Called from existing spawn_blocking command bodies, never the AppKit thread.
pub(crate) fn confirm(
    window: &tauri::WebviewWindow,
    app: &tauri::AppHandle,
    endpoint: &str,
    title: &str,
    message: String,
) -> Result<Option<Choice>, String> {
    let endpoint = crate::repository::remote_endpoint::https(endpoint)?.to_string();
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        let detail = message
            .strip_prefix(&endpoint)
            .unwrap_or(&message)
            .trim_start_matches('\n');
        let address =
            std::ffi::CString::new(endpoint.clone()).map_err(|_| "INVALID_CREDENTIAL_ENDPOINT")?;
        let title = std::ffi::CString::new(title).map_err(|_| "INVALID_NETWORK_PROMPT")?;
        let message = std::ffi::CString::new(detail).map_err(|_| "INVALID_NETWORK_PROMPT")?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let mut bytes = zeroize::Zeroizing::new(vec![0u8; 16384]);
            let mut length = 0usize;
            let mut mode = 0i32;
            let mut remember = false;
            // SAFETY: dispatched to AppKit main thread, all C strings and output
            // pointers remain alive through the synchronous native modal call.
            let status = unsafe {
                branchdesk_prompt_credential(
                    address.as_ptr(),
                    title.as_ptr(),
                    message.as_ptr(),
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    &mut length,
                    &mut mode,
                    &mut remember,
                )
            };
            let result = if length > bytes.len() {
                Err("INVALID_NATIVE_CREDENTIAL_RESULT".into())
            } else {
                decode_choice(endpoint, status, mode, remember, &bytes[..length])
            };
            let _ = sender.send(result);
        })
        .map_err(|_| "NATIVE_CREDENTIAL_DIALOG_FAILED")?;
        receiver
            .recv()
            .map_err(|_| "NATIVE_CREDENTIAL_DIALOG_FAILED")?
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
        if !app
            .dialog()
            .message(format!(
                "{message}\n\nAnonymous HTTPS only on this platform."
            ))
            .parent(window)
            .title(title)
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
        {
            return Ok(None);
        }
        Ok(Some(Choice {
            endpoint,
            mode: Mode::Anonymous,
        }))
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn branchdesk_prompt_credential(
        endpoint: *const std::ffi::c_char,
        title: *const std::ffi::c_char,
        message: *const std::ffi::c_char,
        output: *mut u8,
        capacity: usize,
        length: *mut usize,
        mode: *mut i32,
        remember: *mut bool,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    const URL: &str = "https://example.invalid/repo.git";
    fn secret() -> Credential {
        Credential::new("synthetic".into(), "token".into()).unwrap()
    }
    fn never_load(_: &str) -> Result<Option<Credential>, &'static str> {
        panic!("vault must not be read");
    }
    fn never_save(_: &str, _: &Credential) -> Result<(), &'static str> {
        panic!("vault must not be changed");
    }
    #[test]
    fn anonymous_and_use_once_never_access_vault() {
        let anonymous = Choice {
            endpoint: URL.into(),
            mode: Mode::Anonymous,
        };
        assert!(anonymous
            .resolve_using(URL, never_load, never_save)
            .unwrap()
            .is_none());
        let once = Choice {
            endpoint: URL.into(),
            mode: Mode::Entered {
                credential: secret(),
                remember: false,
            },
        };
        assert!(once
            .resolve_using(URL, never_load, never_save)
            .unwrap()
            .is_some());
    }
    #[test]
    fn saved_and_remember_require_exact_scope_and_preserve_storage_errors() {
        let saved = Choice {
            endpoint: URL.into(),
            mode: Mode::Saved,
        };
        assert!(saved
            .resolve_using(
                URL,
                |url| {
                    assert_eq!(url, URL);
                    Ok(Some(secret()))
                },
                never_save
            )
            .unwrap()
            .is_some());
        let missing = Choice {
            endpoint: URL.into(),
            mode: Mode::Saved,
        };
        assert_eq!(
            missing
                .resolve_using(URL, |_| Ok(None), never_save)
                .err()
                .unwrap(),
            "CREDENTIAL_NOT_FOUND"
        );
        let remember = Choice {
            endpoint: URL.into(),
            mode: Mode::Entered {
                credential: secret(),
                remember: true,
            },
        };
        assert_eq!(
            remember
                .resolve_using(URL, never_load, |_, _| Err("KEYCHAIN_SIGNING_REQUIRED"))
                .err()
                .unwrap(),
            "KEYCHAIN_SIGNING_REQUIRED"
        );
        let wrong = Choice {
            endpoint: URL.into(),
            mode: Mode::Entered {
                credential: secret(),
                remember: true,
            },
        };
        assert_eq!(
            wrong
                .resolve_using("https://else.invalid/repo.git", never_load, never_save)
                .err()
                .unwrap(),
            "REMOTE_CREDENTIAL_SCOPE_MISMATCH"
        );
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn cancel_and_invalid_native_results_never_produce_credentials() {
        assert!(decode_choice(URL.into(), 0, 1, true, &secret().encode())
            .unwrap()
            .is_none());
        assert!(decode_choice(URL.into(), 1, 0, true, &[]).is_err());
        assert!(decode_choice(URL.into(), 1, 2, false, b"unexpected").is_err());
        assert!(decode_choice(URL.into(), 1, 1, false, b"invalid").is_err());
        assert!(decode_choice(URL.into(), -1, 0, false, &[]).is_err());
        let choice = decode_choice(URL.into(), 1, 1, false, &secret().encode())
            .unwrap()
            .unwrap();
        assert!(choice
            .resolve_using(URL, never_load, never_save)
            .unwrap()
            .is_some());
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn native_bridge_refuses_background_thread_without_opening_ui() {
        let value = std::ffi::CString::new(URL).unwrap();
        let mut bytes = zeroize::Zeroizing::new(vec![0u8; 16384]);
        let (mut length, mut mode, mut remember) = (0, 0, false);
        // Rust tests run off AppKit's main thread; bridge must refuse before UI.
        let status = unsafe {
            branchdesk_prompt_credential(
                value.as_ptr(),
                value.as_ptr(),
                value.as_ptr(),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut length,
                &mut mode,
                &mut remember,
            )
        };
        assert_eq!(status, -1);
    }
    #[test]
    fn forget_requires_current_confirmed_slot_and_never_changes_repository() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", URL).unwrap();
        repo.index().unwrap().write().unwrap();
        let mut session =
            crate::repository::Session::new(dir.path().canonicalize().unwrap()).unwrap();
        let snap = session.snapshot().unwrap();
        let config = std::fs::read(dir.path().join(".git/config")).unwrap();
        let index = std::fs::read(dir.path().join(".git/index")).unwrap();
        let head = std::fs::read(dir.path().join(".git/HEAD")).unwrap();
        let calls = std::cell::Cell::new(0);
        forget_using(
            &mut session,
            &snap.revision,
            "origin",
            false,
            0,
            URL,
            |endpoint| {
                assert_eq!(endpoint, URL);
                calls.set(calls.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(
            forget_using(
                &mut session,
                &snap.revision,
                "origin",
                false,
                0,
                "https://else.invalid/r",
                |_| panic!("wrong scope must not delete")
            )
            .unwrap_err(),
            "REMOTE_CREDENTIAL_SCOPE_MISMATCH"
        );
        assert_eq!(
            forget_using(&mut session, &snap.revision, "origin", true, 0, URL, |_| {
                Err("KEYCHAIN_LOCKED_OR_DENIED")
            })
            .unwrap_err(),
            "KEYCHAIN_LOCKED_OR_DENIED"
        );
        assert_eq!(
            std::fs::read(dir.path().join(".git/config")).unwrap(),
            config
        );
        assert_eq!(std::fs::read(dir.path().join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(dir.path().join(".git/HEAD")).unwrap(), head);
        repo.remote_set_url("origin", "https://changed.invalid/r")
            .unwrap();
        assert!(forget_using(
            &mut session,
            &snap.revision,
            "origin",
            false,
            0,
            URL,
            |_| panic!("stale scope must not delete")
        )
        .is_err());
    }
}
