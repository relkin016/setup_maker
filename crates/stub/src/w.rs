//! Small Win32 helpers.
#![allow(dead_code)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

pub fn from_wide_buf(b: &[u16]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

pub fn msgbox(owner: HWND, text: &str, title: &str, flags: u32) -> i32 {
    let t = wide(text);
    let c = wide(title);
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), flags | MB_SETFOREGROUND) }
}

pub fn known_folder(id: &GUID) -> String {
    unsafe {
        let mut p: *mut u16 = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(id, 0x0000_8000 /* KF_FLAG_CREATE */, std::ptr::null_mut(), &mut p);
        let s = if hr >= 0 { from_wide(p) } else { String::new() };
        if !p.is_null() {
            CoTaskMemFree(p as *const c_void);
        }
        s
    }
}

pub fn module_path() -> PathBuf {
    let mut buf = vec![0u16; 32768];
    let n = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32)
    };
    PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]))
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

pub fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// (major, minor, build, service pack major)
pub fn windows_version() -> (u32, u32, u32, u32) {
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOEXW;
    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;
    unsafe {
        let ntdll = wide("ntdll.dll");
        let h = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(ntdll.as_ptr());
        let f = windows_sys::Win32::System::LibraryLoader::GetProcAddress(h, b"RtlGetVersion\0".as_ptr());
        let mut vi: OSVERSIONINFOEXW = std::mem::zeroed();
        vi.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOEXW>() as u32;
        if let Some(f) = f {
            let f: RtlGetVersion = std::mem::transmute(f);
            f(&mut vi);
        }
        (vi.dwMajorVersion, vi.dwMinorVersion, vi.dwBuildNumber, vi.wServicePackMajor as u32)
    }
}

/// Parse Inno version strings "6.1sp1", "10.0.19041", "0,6.1sp1".
pub fn parse_min_version(s: &str) -> Option<(u32, u32, u32, u32)> {
    let s = s.trim();
    let s = s.rsplit(',').next().unwrap_or(s).trim();
    if s.is_empty() || s == "0" {
        return None;
    }
    let (ver, sp) = match s.to_ascii_lowercase().split_once("sp") {
        Some((v, sp)) => (v.to_string(), sp.trim().parse().unwrap_or(0)),
        None => (s.to_string(), 0),
    };
    let mut it = ver.split('.').map(|x| x.trim().parse::<u32>().unwrap_or(0));
    Some((it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0), sp))
}

pub fn version_at_least(min: &str) -> bool {
    match parse_min_version(min) {
        None => true,
        Some((a, b, c, sp)) => {
            let (ma, mi, bu, s) = windows_version();
            (ma, mi, bu, s) >= (a, b, c, sp) || (ma, mi) > (a, b)
        }
    }
}

pub fn total_ram_mb() -> u64 {
    use windows_sys::Win32::System::SystemInformation::*;
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut m);
        m.ullTotalPhys / (1024 * 1024)
    }
}

/// Free bytes on the volume of `path` (nearest existing ancestor).
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    loop {
        if p.exists() {
            break;
        }
        if !p.pop() {
            return None;
        }
    }
    let w = wide(&p.to_string_lossy());
    let mut free = 0u64;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
    };
    if ok != 0 {
        Some(free)
    } else {
        None
    }
}

pub fn set_attributes(path: &Path, attrs: u32) {
    let w = wide(&path.to_string_lossy());
    unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(w.as_ptr(), if attrs == 0 { 0x80 } else { attrs });
    }
}

pub fn clear_readonly(path: &Path) {
    set_attributes(path, 0);
}

/// Set the modification time from a FILETIME value.
pub fn set_mtime(f: &std::fs::File, ft: u64) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::SetFileTime;
    if ft == 0 {
        return;
    }
    let t = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    unsafe {
        SetFileTime(f.as_raw_handle() as HANDLE, std::ptr::null(), std::ptr::null(), &t);
    }
}

pub fn shell_open(path: &str, params: &str, dir: &str, show: i32) -> bool {
    let op = wide("open");
    let f = wide(path);
    let p = wide(params);
    let d = wide(dir);
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            f.as_ptr(),
            if params.is_empty() { std::ptr::null() } else { p.as_ptr() },
            if dir.is_empty() { std::ptr::null() } else { d.as_ptr() },
            show,
        )
    };
    r as isize > 32
}

/// Run elevated copy of ourselves and wait. Returns exit code or None if refused.
pub fn run_elevated(exe: &Path, args: &str) -> Option<u32> {
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let par = wide(args);
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = par.as_ptr();
        sei.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut sei) == 0 {
            return None;
        }
        WaitForSingleObject(sei.hProcess, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(sei.hProcess, &mut code);
        CloseHandle(sei.hProcess);
        Some(code)
    }
}

pub fn user_ui_lang() -> u16 {
    unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() }
}

pub fn computer_name() -> String {
    env("COMPUTERNAME")
}

pub fn user_name() -> String {
    env("USERNAME")
}

pub fn windows_dir() -> String {
    let mut b = [0u16; 260];
    let n = unsafe { windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(b.as_mut_ptr(), 260) };
    String::from_utf16_lossy(&b[..n as usize])
}

pub fn system_dir() -> String {
    let mut b = [0u16; 260];
    let n = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(b.as_mut_ptr(), 260) };
    String::from_utf16_lossy(&b[..n as usize])
}

pub fn is_win64() -> bool {
    if cfg!(target_pointer_width = "64") {
        return true;
    }
    let mut wow = 0;
    unsafe {
        windows_sys::Win32::System::Threading::IsWow64Process(windows_sys::Win32::System::Threading::GetCurrentProcess(), &mut wow);
    }
    wow != 0
}

pub fn reboot() {
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::System::Shutdown::*;
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok) != 0 {
            let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
            let name = wide("SeShutdownPrivilege");
            LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut tp.Privileges[0].Luid);
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            AdjustTokenPrivileges(tok, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
            CloseHandle(tok);
        }
        ExitWindowsEx(EWX_REBOOT, SHTDN_REASON_MAJOR_APPLICATION | SHTDN_REASON_FLAG_PLANNED);
    }
}
