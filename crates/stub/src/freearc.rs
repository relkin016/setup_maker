//! FreeArc `.arc` support. FreeArc has no Rust implementation, so we use the
//! user's own FreeArc binaries embedded as tools (extracted to `{tmp}`):
//!  * `unarc.dll` (Bulat Ziganshin's FreeArcExtract API, like ISDone/ISArcExtract)
//!    — loaded in-process; must match the stub bitness (unarc.dll is usually
//!    32-bit, so the builder selects the x86 stub when it is present);
//!  * otherwise `unarc.exe` / `arc.exe` run as a hidden child process.

use crate::archives::{Created, Hooks};
use crate::w::wide;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

type R<T> = Result<T, String>;

type Callback = unsafe extern "system" fn(what: *const c_char, int1: i32, int2: i32, s: *const c_char) -> i32;
type FreeArcExtract = unsafe extern "C" fn(cb: Callback, ...) -> i32;

struct CbState<'a, 'b> {
    hooks: &'a mut Hooks<'b>,
    total_mb: i32,
    dest: std::path::PathBuf,
    cancelled: bool,
    arc_len: f64,
}

thread_local! {
    static STATE: std::cell::Cell<*mut ()> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

unsafe extern "system" fn callback(what: *const c_char, int1: i32, _int2: i32, s: *const c_char) -> i32 {
    let p = STATE.with(|c| c.get()) as *mut CbState;
    if p.is_null() || what.is_null() {
        return 0;
    }
    let st = &mut *p;
    let what = CStr::from_ptr(what).to_string_lossy();
    match what.as_ref() {
        "origsize" | "total" => st.total_mb = int1.max(1),
        "filename" if !s.is_null() => {
            let name = CStr::from_ptr(s).to_string_lossy().to_string();
            if let Some(rel) = crate::archives::safe_rel(&name) {
                let full = st.dest.join(rel);
                st.hooks.created.push(Created { path: full.clone(), is_dir: false, fresh: true });
                if !(st.hooks.progress)(0, &full.to_string_lossy()) {
                    st.cancelled = true;
                }
            }
        }
        "write" => {
            // int1 = MB written; report as a fraction of 1 GiB "virtual" bytes
            let frac = int1 as f64 / st.total_mb.max(1) as f64;
            if !(st.hooks.progress)((frac.min(1.0) * st.arc_len) as u64, "") {
                st.cancelled = true;
            }
        }
        _ => {}
    }
    if st.cancelled {
        -127
    } else {
        0
    }
}

pub fn extract(archive: &Path, dest: &Path, password: &str, tmp: &Path, h: &mut Hooks) -> R<()> {
    let dll = tmp.join("unarc.dll");
    if dll.is_file() {
        if let Some(r) = extract_dll(&dll, archive, dest, password, h) {
            return r;
        }
    }
    for exe in ["unarc.exe", "arc.exe"] {
        let p = tmp.join(exe);
        if p.is_file() {
            return extract_exe(&p, archive, dest, password, h);
        }
    }
    Err("FreeArc archive: add unarc.dll or unarc.exe/arc.exe to the project tools".into())
}

/// `None` if the DLL could not be loaded (e.g. wrong bitness) -> try the exe.
fn extract_dll(dll: &Path, archive: &Path, dest: &Path, password: &str, h: &mut Hooks) -> Option<R<()>> {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::*;
    unsafe {
        let w = wide(&dll.to_string_lossy());
        let m = LoadLibraryW(w.as_ptr());
        if m.is_null() {
            return None;
        }
        let f = GetProcAddress(m, b"FreeArcExtract\0".as_ptr())?;
        let f: FreeArcExtract = std::mem::transmute(f);
        // FreeArc DLL works with ANSI paths
        let a = |s: String| CString::new(s).unwrap_or_default();
        let cmd = a("x".into());
        let yes = a("-o+".into());
        let dp = a(format!("-dp{}", dest.display()));
        let pw = a(if password.is_empty() { String::new() } else { format!("-p{}", password) });
        let wd = a(format!("-w{}", dest.display()));
        let dd = a("--".into());
        let arc = a(archive.to_string_lossy().to_string());
        let empty = a(String::new());
        let mut st = CbState { hooks: h, total_mb: 1, dest: dest.to_path_buf(), cancelled: false, arc_len: archive.metadata().map(|m| m.len()).unwrap_or(1) as f64 };
        STATE.with(|c| c.set(&mut st as *mut CbState as *mut ()));
        let pwp = if password.is_empty() { empty.as_ptr() } else { pw.as_ptr() };
        let code = f(
            callback,
            cmd.as_ptr(),
            yes.as_ptr(),
            dp.as_ptr(),
            wd.as_ptr(),
            pwp,
            dd.as_ptr(),
            arc.as_ptr(),
            empty.as_ptr(),
            empty.as_ptr(),
            empty.as_ptr(),
            std::ptr::null::<c_char>(),
        );
        STATE.with(|c| c.set(std::ptr::null_mut()));
        FreeLibrary(m);
        if st.cancelled {
            return Some(Err("cancelled".into()));
        }
        Some(if code == 0 { Ok(()) } else { Err(format!("unarc.dll error code {}", code)) })
    }
}

fn extract_exe(exe: &Path, archive: &Path, dest: &Path, password: &str, h: &mut Hooks) -> R<()> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let mut args = format!("x -o+ -dp\"{}\"", dest.display());
    if !password.is_empty() {
        args.push_str(&format!(" -p\"{}\"", password));
    }
    args.push_str(&format!(" -- \"{}\"", archive.display()));
    let mut child = std::process::Command::new(exe)
        .raw_arg(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| format!("{}: {}", exe.display(), e))?;
    let mut out = child.stdout.take().unwrap();
    let mut buf = [0u8; 4096];
    let mut tail = String::new();
    loop {
        let n = out.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        tail.push_str(&String::from_utf8_lossy(&buf[..n]));
        // progress lines look like "Extracting ... 42%"
        if let Some(pos) = tail.rfind('%') {
            let digits: String = tail[..pos].chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<Vec<_>>().into_iter().rev().collect();
            if let Ok(pct) = digits.parse::<f64>() {
                if !(h.progress)((pct / 100.0 * archive.metadata().map(|m| m.len()).unwrap_or(1) as f64) as u64, "") {
                    let _ = child.kill();
                    return Err("cancelled".into());
                }
            }
        }
        if tail.len() > 512 {
            tail = tail[tail.len() - 256..].to_string();
        }
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{} exit code {:?}", exe.display(), st.code()))
    }
}
