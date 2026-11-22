use std::path::Path;
pub fn reveal(path:&Path,select:bool)->Result<(),String>{
    if !path.is_absolute()||path.canonicalize().ok().as_deref()!=Some(path){return Err("FILE_MANAGER_PATH_UNAVAILABLE".into());}
    #[cfg(target_os="macos")]
    {
        use std::{process::{Command,Stdio},time::{Duration,Instant}};
        let mut command=Command::new("/usr/bin/open");
        if select {command.arg("-R");}else{command.args(["-a","/System/Library/CoreServices/Finder.app"]);}
        let mut child=command.arg("--").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"FILE_MANAGER_LAUNCH_FAILED")?;
        let deadline=Instant::now()+Duration::from_secs(5);
        loop{match child.try_wait(){Ok(Some(status))=>return if status.success(){Ok(())}else{Err("FILE_MANAGER_LAUNCH_FAILED".into())},Ok(None) if Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(25)),_=>{let _=child.kill();let _=child.wait();return Err("FILE_MANAGER_LAUNCH_FAILED".into());}}}
    }
    #[cfg(target_os="windows")]
    {let _=select;windows_reveal(path)}
    #[cfg(not(any(target_os="macos",target_os="windows")))]
    {let _=select;Err("FILE_MANAGER_PLATFORM_NOT_SUPPORTED".into())}
}

#[cfg(target_os="windows")]
fn windows_reveal(path:&Path)->Result<(),String>{
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{System::Com::{CoInitializeEx,CoUninitialize,CoTaskMemFree,COINIT_APARTMENTTHREADED},UI::Shell::{SHParseDisplayName,SHOpenFolderAndSelectItems}};
    // Dedicated COM apartment: a pooled worker may already have another model.
    let owned=path.to_owned();
    std::thread::spawn(move||{
        let initialized=unsafe{CoInitializeEx(std::ptr::null(),COINIT_APARTMENTTHREADED as u32)};
        if initialized<0{return Err("FILE_MANAGER_COM_FAILED".into());}
        struct Apartment;impl Drop for Apartment{fn drop(&mut self){unsafe{CoUninitialize();}}}
        let _apartment=Apartment;
        let wide:Vec<u16>=owned.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut item=std::ptr::null_mut();
        let parsed=unsafe{SHParseDisplayName(wide.as_ptr(),std::ptr::null_mut(),&mut item,0,std::ptr::null_mut())};
        if parsed<0||item.is_null(){if !item.is_null(){unsafe{CoTaskMemFree(item.cast());}}return Err("FILE_MANAGER_PATH_UNAVAILABLE".into());}
        // cidl=0 means open the parent and select this exact file/folder PIDL.
        let result=unsafe{SHOpenFolderAndSelectItems(item,0,std::ptr::null(),0)};
        unsafe{CoTaskMemFree(item.cast());}
        if result<0{Err("FILE_MANAGER_LAUNCH_FAILED".into())}else{Ok(())}
    }).join().map_err(|_|"FILE_MANAGER_WORKER_FAILED".to_string())?
}
