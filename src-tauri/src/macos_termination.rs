//! AppKit termination bridge; installed and replied only on the main thread.
use std::sync::OnceLock;
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
extern "C" {
    fn branchdesk_install_termination(callback: extern "C" fn() -> bool) -> i32;
    fn branchdesk_reply_termination(saved: bool) -> bool;
}
extern "C" fn request() -> bool {
    // Never unwind across the Objective-C/C callback boundary. Keep data on error.
    std::panic::catch_unwind(|| APP.get().is_none_or(super::request_shutdown)).unwrap_or(true)
}
pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
    APP.set(app.clone())
        .map_err(|_| "TERMINATION_ALREADY_INSTALLED")?;
    // SAFETY: setup runs on AppKit's main thread; callback is static and the
    // Objective-C bridge verifies thread/delegate/absence before adding a method.
    let result = unsafe { branchdesk_install_termination(request) };
    if result != 0 {
        return Err(format!("TERMINATION_INSTALL_FAILED:{result}"));
    }
    Ok(())
}
pub fn finish(app: &tauri::AppHandle, saved: bool) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        // SAFETY: explicitly dispatched to the main thread; the bridge checks
        // outstanding AppKit ownership before replying, preventing double reply.
        let replied = unsafe { branchdesk_reply_termination(saved) };
        if saved && !replied {
            handle.exit(0);
        }
    })
    .map_err(|_| "TERMINATION_REPLY_FAILED".into())
}
