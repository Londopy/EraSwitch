// win_ops.rs
// raw Win32 calls, each behind a small safe wrapper. all the unsafe lives here.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsWindowVisible, ShowWindow, SystemParametersInfoW, SPIF_SENDCHANGE,
    SPIF_UPDATEINIFILE, SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SW_SHOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---- single instance --------------------------------------------------------

pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// None if another copy runs. wait_for_release gives a restarting copy up to
// 3 s for the old one to let go (the updater and Restart use --restarted)
pub fn acquire_single_instance(wait_for_release: bool) -> Option<InstanceGuard> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        unsafe {
            match CreateMutexW(None, true, w!("EraSwitch_SingleInstance_Mutex")) {
                Ok(handle) => {
                    if GetLastError() != ERROR_ALREADY_EXISTS {
                        return Some(InstanceGuard(handle));
                    }
                    let _ = CloseHandle(handle);
                }
                Err(_) => return None,
            }
        }
        if !wait_for_release || std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

// ---- wallpaper --------------------------------------------------------------

pub fn get_wallpaper() -> Result<String> {
    let mut buf = [0u16; 1024];
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..len]))
}

// an empty path clears the wallpaper (solid colour), which is a valid state to restore
pub fn set_wallpaper(path: &str) -> Result<()> {
    let w = wide(path);
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(w.as_ptr() as *mut _),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )?;
    }
    Ok(())
}

// ---- explorer's taskbar -----------------------------------------------------

// every top-level window of `class` that explorer.exe owns. RetroBar registers
// look-alike Shell_TrayWnd windows while it runs, so the owner matters.
fn explorer_windows(class: PCWSTR) -> Vec<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};
    let explorer = processes_named("explorer.exe");
    let mut out = Vec::new();
    let mut after = HWND::default();
    loop {
        let Ok(hwnd) = (unsafe { FindWindowExW(HWND::default(), after, class, PCWSTR::null()) })
        else {
            break;
        };
        if hwnd.0.is_null() {
            break;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if explorer.contains(&pid) {
            out.push(hwnd);
        }
        after = hwnd;
    }
    out
}

// explorer's main taskbar
fn explorer_tray() -> Option<HWND> {
    explorer_windows(w!("Shell_TrayWnd")).into_iter().next()
}

pub fn explorer_taskbar_visible() -> bool {
    explorer_tray().is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
}

// the main taskbar and the one on every other monitor: RetroBar hides them all
pub fn show_explorer_taskbar() {
    let mut all = explorer_windows(w!("Shell_TrayWnd"));
    all.extend(explorer_windows(w!("Shell_SecondaryTrayWnd")));
    for h in all {
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
        }
    }
}

fn appbar_data() -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.unwrap_or_default(),
        ..Default::default()
    }
}

// the taskbar's auto-hide/always-on-top bits (ABS_AUTOHIDE = 1, ABS_ALWAYSONTOP = 2).
// read live: the copy in the registry (StuckRects3) can lag behind.
pub fn taskbar_appbar_state() -> u32 {
    let mut d = appbar_data();
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 }
}

pub fn set_taskbar_appbar_state(state: u32) {
    // aim it at explorer's tray specifically, never RetroBar's look-alike
    let mut d = appbar_data();
    if let Some(h) = explorer_tray() {
        d.hWnd = h;
    }
    d.lParam = windows::Win32::Foundation::LPARAM(state as isize);
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

// ---- processes --------------------------------------------------------------

// pids of every process whose exe name matches (case-insensitive)
pub fn processes_named(exe_name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case(exe_name) {
                    out.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

// full path of a running process, when we're allowed to ask
pub fn process_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(h);
        r.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(
            &buf[..len as usize],
        )))
    }
}

pub fn is_running(exe: &Path) -> bool {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    processes_named(name)
        .into_iter()
        .any(|pid| process_path(pid).is_some_and(|p| same_path(&p, exe)))
}

// paths from config may use forward slashes or `.\` bits; Windows paths
// compare case-insensitively
fn normal(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    for part in s.split('\\') {
        match part {
            "" | "." if !parts.is_empty() => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("\\")
}

fn same_path(a: &Path, b: &Path) -> bool {
    normal(a) == normal(b)
}

// stops every copy of `exe` (matched by full path). returns how many were stopped
pub fn kill_all(exe: &Path) -> Result<usize> {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        bail!("{} has no file name", exe.display());
    };
    let mut n = 0;
    for pid in processes_named(name) {
        if !process_path(pid).is_some_and(|p| same_path(&p, exe)) {
            continue;
        }
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                if TerminateProcess(h, 0).is_ok() {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    Ok(n)
}

// waits until no copy of `exe` is left, up to `timeout`
pub fn wait_gone(exe: &Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while is_running(exe) {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_our_own_process() {
        let me = std::env::current_exe().unwrap();
        assert!(is_running(&me));
    }

    #[test]
    fn paths_compare_like_windows_does() {
        assert!(same_path(
            Path::new("D:/Tools/RetroBar.exe"),
            Path::new(r"d:\tools\retrobar.exe")
        ));
        assert!(same_path(
            Path::new(r"C:\a\.\b\..\RetroBar.exe"),
            Path::new(r"C:\a\RetroBar.exe")
        ));
        assert!(!same_path(
            Path::new(r"C:\a\RetroBar.exe"),
            Path::new(r"C:\b\RetroBar.exe")
        ));
    }

    #[test]
    fn wallpaper_reads_without_error() {
        assert!(get_wallpaper().is_ok());
    }
}
