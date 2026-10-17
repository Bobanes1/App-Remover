//! Side-effecting operations. File deletion works everywhere; the registry, service,
//! task and process helpers shell out to the standard Windows tools.
use std::fs;
use std::io;
use std::path::Path;

use crate::model::RegKey;
use crate::safety;

pub trait System: Send + Sync {
    fn kill_processes_in(&self, directory: &str);
    fn kill_process_names(&self, names: &[&str]);
    fn delete_service(&self, name: &str) -> Result<(), String>;
    fn delete_task(&self, name: &str) -> Result<(), String>;
    fn reg_export(&self, key: &RegKey, file: &Path) -> Result<(), String>;
    fn reg_delete_key(&self, key: &RegKey) -> Result<(), String>;
    fn reg_delete_value(&self, key: &RegKey, name: &str) -> Result<(), String>;

    /// Delete a file or folder; returns a short note on success.
    fn delete_path(&self, path: &str) -> Result<String, String> {
        force_remove(Path::new(path)).map(|_| "removed".into()).map_err(|e| e.to_string())
    }
}

/// Recursive delete that clears read-only flags and never follows symlinks or
/// junctions (the link itself is removed, never what it points to).
pub fn force_remove(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        return fs::remove_dir(path).or_else(|_| fs::remove_file(path));
    }
    if ft.is_dir() {
        for entry in fs::read_dir(path)? {
            force_remove(&entry?.path())?;
        }
        clear_readonly(path, &meta);
        return fs::remove_dir(path);
    }
    clear_readonly(path, &meta);
    fs::remove_file(path)
}

fn clear_readonly(path: &Path, meta: &fs::Metadata) {
    let mut perms = meta.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
    }
}

/// The real thing (Windows).
pub struct RealSystem;

#[cfg(windows)]
fn run(args: &[&str]) -> (i32, String) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    match std::process::Command::new(args[0]).args(&args[1..]).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code().unwrap_or(-1), s.trim().to_string())
        }
        Err(e) => (-1, e.to_string()),
    }
}

#[cfg(not(windows))]
fn run(_args: &[&str]) -> (i32, String) {
    (-1, "only supported on Windows".into())
}

fn check((code, out): (i32, String), what: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else if out.is_empty() {
        Err(format!("{what} failed ({code})"))
    } else {
        Err(out)
    }
}

fn reg_path(key: &RegKey) -> String {
    key.to_string()
}

impl System for RealSystem {
    fn kill_processes_in(&self, directory: &str) {
        let dir = directory.trim();
        if !Path::new(dir).is_absolute() || Path::new(dir).parent().is_none() {
            return;
        }
        for (pid, _) in procs::list() {
            if let Some(image) = procs::image_path(pid) {
                if safety::is_within(&image, dir) && procs::may_stop(pid, &image) {
                    procs::terminate(pid);
                }
            }
        }
    }

    fn kill_process_names(&self, names: &[&str]) {
        for (pid, exe) in procs::list() {
            if names.iter().any(|n| n.eq_ignore_ascii_case(&exe)) {
                if let Some(image) = procs::image_path(pid) {
                    if procs::may_stop(pid, &image) {
                        procs::terminate(pid);
                    }
                }
            }
        }
    }

    fn delete_service(&self, name: &str) -> Result<(), String> {
        run(&["sc", "stop", name]);
        let (code, out) = run(&["sc", "delete", name]);
        if code != 0 && !out.contains("1060") {
            // 1060 = the service does not exist (already gone)
            return check((code, out), "sc delete");
        }
        Ok(())
    }

    fn delete_task(&self, name: &str) -> Result<(), String> {
        check(run(&["schtasks", "/Delete", "/TN", name, "/F"]), "schtasks")
    }

    fn reg_export(&self, key: &RegKey, file: &Path) -> Result<(), String> {
        let view = format!("/reg:{}", key.view);
        check(run(&["reg", "export", &reg_path(key), &file.to_string_lossy(), "/y", &view]), "reg export")
    }

    fn reg_delete_key(&self, key: &RegKey) -> Result<(), String> {
        let view = format!("/reg:{}", key.view);
        check(run(&["reg", "delete", &reg_path(key), "/f", &view]), "reg delete")
    }

    fn reg_delete_value(&self, key: &RegKey, name: &str) -> Result<(), String> {
        let view = format!("/reg:{}", key.view);
        check(run(&["reg", "delete", &reg_path(key), "/v", name, "/f", &view]), "reg delete")
    }

    fn delete_path(&self, path: &str) -> Result<String, String> {
        let p = Path::new(path);
        let first = match force_remove(p) {
            Ok(()) => return Ok("removed".into()),
            Err(e) => e.to_string(),
        };
        // Files still in use: let Windows delete them during the next restart.
        if schedule_delete_on_reboot(p) {
            return Ok("in use: will be deleted at next restart".into());
        }
        Err(first)
    }
}

#[cfg(windows)]
fn schedule_delete_on_reboot(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};

    fn collect(p: &Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                let ep = e.path();
                if e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                    collect(&ep, out);
                }
                out.push(ep);
            }
        }
    }
    let mut targets = Vec::new();
    if fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        collect(path, &mut targets); // children before their parent
    }
    targets.push(path.to_path_buf());
    let mut ok = false;
    for t in targets {
        let wide: Vec<u16> = t.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
        ok |= unsafe { MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) } != 0;
    }
    ok
}

#[cfg(not(windows))]
fn schedule_delete_on_reboot(_path: &Path) -> bool {
    false
}

/// Does nothing: used by `--demo` so no real change can ever happen.
pub struct NoopSystem;

impl System for NoopSystem {
    fn kill_processes_in(&self, _: &str) {}
    fn kill_process_names(&self, _: &[&str]) {}
    fn delete_service(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn delete_task(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn reg_export(&self, _: &RegKey, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn reg_delete_key(&self, _: &RegKey) -> Result<(), String> {
        Ok(())
    }
    fn reg_delete_value(&self, _: &RegKey, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn delete_path(&self, _: &str) -> Result<String, String> {
        Ok("(demo) not deleted".into())
    }
}

/// Running processes via the Windows API (no PowerShell / taskkill child processes).
#[cfg(windows)]
mod procs {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    use crate::safety;

    /// (pid, exe file name) of every running process.
    pub fn list() -> Vec<(u32, String)> {
        let mut out = Vec::new();
        // SAFETY: plain Win32 calls; `entry` is a correctly sized, zero-initialised
        // PROCESSENTRY32W and the snapshot handle is closed before returning.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                let name = &entry.szExeFile;
                let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                out.push((entry.th32ProcessID, String::from_utf16_lossy(&name[..len])));
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        out
    }

    /// Full path of a process's executable, if we may look at it.
    pub fn image_path(pid: u32) -> Option<String> {
        // SAFETY: the handle is checked for null and closed; `buf`/`len` describe a valid buffer.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = vec![0u16; 32_768];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(h);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// Never stop App Remover itself, the kernel's pseudo-processes, or anything
    /// running from the Windows folder - whatever an uninstall entry claims.
    pub fn may_stop(pid: u32, image: &str) -> bool {
        // SAFETY: no arguments, cannot fail.
        let me = unsafe { GetCurrentProcessId() };
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        pid > 4 && pid != me && Path::new(image).is_absolute() && !safety::is_within(image, &sysroot)
    }

    pub fn terminate(pid: u32) -> bool {
        // SAFETY: the handle is checked for null and closed.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            ok
        }
    }
}

#[cfg(not(windows))]
mod procs {
    pub fn list() -> Vec<(u32, String)> {
        Vec::new()
    }
    pub fn image_path(_pid: u32) -> Option<String> {
        None
    }
    pub fn may_stop(_pid: u32, _image: &str) -> bool {
        false
    }
    pub fn terminate(_pid: u32) -> bool {
        false
    }
}
