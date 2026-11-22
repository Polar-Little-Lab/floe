use std::path::Path;

pub fn open(root: &Path) -> Result<(), String> {
    if !root.is_absolute() || !root.is_dir() || root.canonicalize().ok().as_deref() != Some(root) {
        return Err("TERMINAL_REPOSITORY_UNAVAILABLE".into());
    }
    #[cfg(target_os = "macos")]
    {
        use std::{process::{Command, Stdio}, time::{Duration, Instant}};
        // An argument, never shell source. The renderer supplies only a session ID.
        let mut child = Command::new("/usr/bin/open")
            .args(["-a", "/System/Applications/Utilities/Terminal.app", "--"])
            .arg(root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|_| "TERMINAL_LAUNCH_FAILED")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return if status.success() { Ok(()) } else { Err("TERMINAL_LAUNCH_FAILED".into()) },
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
                _ => { let _ = child.kill(); let _ = child.wait(); return Err("TERMINAL_LAUNCH_FAILED".into()); }
            }
        }
    }
    #[cfg(target_os = "windows")]
    { windows::open(root) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("TERMINAL_PLATFORM_NOT_SUPPORTED".into())
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{ffi::OsString,os::windows::ffi::{OsStrExt,OsStringExt},path::Path};
    use windows_sys::Win32::{Foundation::CloseHandle,System::{SystemInformation::GetSystemDirectoryW,Threading::{CreateProcessW,CREATE_NEW_CONSOLE,PROCESS_INFORMATION,STARTUPINFOW}}};
    pub(super) fn open(root:&Path)->Result<(),String>{
        // cmd.exe cannot retain a UNC working directory. Refuse rather than open
        // somewhere else. Strip only a canonical local-drive verbatim prefix.
        let mut cwd:Vec<u16>=root.as_os_str().encode_wide().collect();
        let prefix:Vec<u16>="\\\\?\\".encode_utf16().collect();
        if cwd.starts_with(&prefix){cwd.drain(..prefix.len());}
        if cwd.len()<3 || !((cwd[0]>=b'A' as u16&&cwd[0]<=b'Z' as u16)||(cwd[0]>=b'a' as u16&&cwd[0]<=b'z' as u16)) || cwd[1]!=b':' as u16 || cwd[2]!=b'\\' as u16 || cwd.contains(&0){
            return Err("TERMINAL_LOCAL_DRIVE_REQUIRED".into());
        }
        cwd.push(0);
        let mut system=vec![0u16;32768];
        let length=unsafe{GetSystemDirectoryW(system.as_mut_ptr(),system.len() as u32)} as usize;
        if length==0||length>=system.len(){return Err("TERMINAL_SYSTEM_DIRECTORY_UNAVAILABLE".into());}
        let executable=std::path::PathBuf::from(OsString::from_wide(&system[..length])).join("cmd.exe");
        let application:Vec<u16>=executable.as_os_str().encode_wide().chain(Some(0)).collect();
        // Fixed command line: no repository path, user preference, PATH lookup,
        // shell interpolation or cmd AutoRun. Current directory is a separate API argument.
        let mut command:Vec<u16>="cmd.exe /D /K".encode_utf16().chain(Some(0)).collect();
        let mut startup:STARTUPINFOW=unsafe{std::mem::zeroed()};startup.cb=std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process:PROCESS_INFORMATION=unsafe{std::mem::zeroed()};
        let ok=unsafe{CreateProcessW(application.as_ptr(),command.as_mut_ptr(),std::ptr::null(),std::ptr::null(),0,CREATE_NEW_CONSOLE,std::ptr::null(),cwd.as_ptr(),&startup,&mut process)};
        if ok==0{return Err("TERMINAL_LAUNCH_FAILED".into());}
        // The terminal is user-owned. Release handles without waiting or terminating it.
        unsafe{CloseHandle(process.hThread);CloseHandle(process.hProcess);}
        Ok(())
    }
}
