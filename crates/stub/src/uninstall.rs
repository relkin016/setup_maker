//! Uninstaller: the stub without payload, copied as `unins000.exe`, with the
//! log in `unins000.dat`. It re-launches itself from %TEMP% so that it can
//! delete its own exe and the application folder.

use crate::consts::Ctx;
use crate::install::{self, LogEntry, UninstData};
use crate::{reg, ui, w};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct Args {
    pub silent: bool,
    pub very_silent: bool,
    pub suppress: bool,
    pub tmp_copy_of: Option<(PathBuf, u32)>,
    pub rest: Vec<String>,
}

pub fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { silent: false, very_silent: false, suppress: false, tmp_copy_of: None, rest: vec![] };
    let mut i = 0;
    while i < argv.len() {
        let s = &argv[i];
        let u = s.to_ascii_uppercase();
        if u == "/__TMPCOPY" && i + 2 < argv.len() {
            a.tmp_copy_of = Some((PathBuf::from(&argv[i + 1]), argv[i + 2].parse().unwrap_or(0)));
            i += 3;
            continue;
        }
        match u.as_str() {
            "/SILENT" => a.silent = true,
            "/VERYSILENT" => {
                a.silent = true;
                a.very_silent = true
            }
            "/SUPPRESSMSGBOXES" => a.suppress = true,
            _ => {}
        }
        a.rest.push(s.clone());
        i += 1;
    }
    a
}

fn quote_args(v: &[String]) -> String {
    v.iter().map(|s| if s.contains(' ') { format!("\"{}\"", s) } else { s.clone() }).collect::<Vec<_>>().join(" ")
}

pub fn main(argv: &[String]) -> i32 {
    let a = parse_args(argv);
    let me = w::module_path();

    // Stage 1: copy to %TEMP% and relaunch from there.
    let Some((orig, parent_pid)) = a.tmp_copy_of.clone() else {
        let tmp = std::env::temp_dir().join(format!("_smk_un{}.exe", std::process::id()));
        if std::fs::copy(&me, &tmp).is_err() {
            return run(&a, &me, &me);
        }
        let args = format!("/__TMPCOPY \"{}\" {} {}", me.display(), std::process::id(), quote_args(&a.rest));
        let ok = std::process::Command::new(&tmp).raw_arg(&args).spawn().is_ok();
        return if ok { 0 } else { run(&a, &me, &me) };
    };
    wait_pid(parent_pid);
    let code = run(&a, &orig, &me);
    // schedule deletion of this temporary copy
    let _ = std::process::Command::new("cmd.exe")
        .raw_arg(format!("/c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", me.display()))
        .creation_flags(0x0800_0000)
        .spawn();
    code
}

fn wait_pid(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::*;
    if pid == 0 {
        return;
    }
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 30000);
            CloseHandle(h);
        }
    }
}

fn run(a: &Args, orig_exe: &Path, me: &Path) -> i32 {
    let dat_path = orig_exe.with_extension("dat");
    let data = std::fs::read(&dat_path).map_err(|e| e.to_string()).and_then(|b| install::read_dat(&b));
    let lang = data.as_ref().map(|d| d.lang.clone()).unwrap_or_else(|_| smk_core::messages::lang_from_langid(w::user_ui_lang()).to_string());
    let mut ctx = Ctx { lang, admin: w::is_admin(), ..Default::default() };
    let d = match data {
        Ok(d) => d,
        Err(_) => {
            if !a.suppress {
                w::msgbox(std::ptr::null_mut(), &ctx.msg("UninstallDataCorrupted"), &ctx.msg("UninstallAppTitle"), 0x10);
            }
            return 1;
        }
    };
    ctx.messages = d.messages.clone();
    ctx.app = d.app.clone();
    ctx.group = d.group.clone();
    ctx.app_name = d.app_name.clone();
    ctx.app_ver_name = d.app_name.clone();
    ctx.uninstallexe = orig_exe.to_string_lossy().to_string();
    ctx.src = orig_exe.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();

    if d.admin && !ctx.admin {
        let mut args = a.rest.clone();
        args.insert(0, format!("/__TMPCOPY \"{}\" 0", orig_exe.display()));
        return match w::run_elevated(me, &args.join(" ")) {
            Some(c) => c as i32,
            None => 1,
        };
    }

    let title = ctx.msgf("UninstallAppFullTitle", &[&d.app_name]);
    if !a.silent {
        let q = ctx.msgf("ConfirmUninstall", &[&d.app_name]);
        if w::msgbox(std::ptr::null_mut(), &q, &title, 0x24 /* MB_YESNO|MB_ICONQUESTION */) != 6 {
            return 2;
        }
    }

    let prog = Arc::new(AtomicU32::new(0));
    let ok = if a.very_silent {
        do_uninstall(&d, &ctx, &prog, orig_exe)
    } else {
        let label = ctx.msgf("UninstallStatusLabel", &[&d.app_name]);
        let heading = ctx.msg("WizardUninstalling");
        let d2 = d.clone();
        let c2 = ctx.clone();
        let o2 = orig_exe.to_path_buf();
        ui::progress::run(&title, &heading, &label, &ctx.msg("ButtonCancel"), prog.clone(), move |p| do_uninstall(&d2, &c2, &p, &o2))
    };

    if !a.silent {
        let key = if ok { "UninstalledAll" } else { "UninstalledMost" };
        w::msgbox(std::ptr::null_mut(), &ctx.msgf(key, &[&d.app_name]), &title, 0x40);
    }
    if d.restart && !a.silent {
        let q = ctx.msgf("UninstallRestart", &[&d.app_name]);
        if w::msgbox(std::ptr::null_mut(), &q, &title, 0x24) == 6 {
            w::reboot();
        }
    }
    0
}

fn delete_file(p: &str) -> bool {
    let path = Path::new(p);
    if !path.exists() {
        return true;
    }
    w::clear_readonly(path);
    if std::fs::remove_file(path).is_ok() {
        return true;
    }
    // locked: delete on reboot
    let wp = w::wide(p);
    unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileExW(wp.as_ptr(), std::ptr::null(), 4 /* DELAY_UNTIL_REBOOT */) };
    false
}

pub fn do_uninstall(d: &UninstData, ctx: &Ctx, prog: &AtomicU32, orig_exe: &Path) -> bool {
    let mut all_ok = true;
    let total = (d.entries.len() + d.uninstall_run.len() + d.uninstall_delete.len() + 2).max(1) as u32;
    let mut step = 0u32;
    let mut tick = || {
        step += 1;
        prog.store(step * 1000 / total, Ordering::Relaxed);
    };

    for r in &d.uninstall_run {
        install::run_item(r, ctx);
        tick();
    }
    let exe_s = orig_exe.to_string_lossy().to_string();
    let dat_s = orig_exe.with_extension("dat").to_string_lossy().to_string();
    for e in d.entries.iter().rev() {
        match e {
            LogEntry::File { path, .. } => {
                if path.eq_ignore_ascii_case(&exe_s) || path.eq_ignore_ascii_case(&dat_s) {
                    // removed last
                } else if !delete_file(path) {
                    all_ok = false;
                }
            }
            LogEntry::Dir { path } => {
                let _ = std::fs::remove_dir(path);
            }
            LogEntry::RegValue { root, sub, name } => {
                if let Some(k) = reg::root_of(root, d.admin).and_then(|r| reg::open(r, sub, true)) {
                    reg::delete_value(&k, name);
                }
            }
            LogEntry::RegKey { root, sub, mode } => {
                if let Some(r) = reg::root_of(root, d.admin) {
                    if mode == "deletekey" {
                        reg::delete_tree(r, sub);
                    } else {
                        reg::delete_if_empty(r, sub);
                    }
                }
            }
        }
        tick();
    }
    for del in &d.uninstall_delete {
        let p = ctx.expand(&del.path);
        // never wipe a drive root or system folders by accident
        if p.len() > 3 {
            install::delete_item(&p, &del.kind);
        }
        tick();
    }
    if let Some(r) = reg::root_of(&d.reg_root, d.admin) {
        reg::delete_tree(r, &d.reg_key);
    }
    delete_file(&dat_s);
    delete_file(&exe_s);
    // remove now-empty folders that were created by setup
    for e in d.entries.iter().rev() {
        if let LogEntry::Dir { path } = e {
            let _ = std::fs::remove_dir(path);
        }
    }
    if let Some(parent) = orig_exe.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    prog.store(1000, Ordering::Relaxed);
    all_ok
}
