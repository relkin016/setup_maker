//! Small modal dialogs: language selection and "insert next disk".

use super::*;
use crate::consts::Ctx;
use std::cell::RefCell;
use std::path::PathBuf;

struct DlgState {
    done: bool,
    ok: bool,
    combo: HWND,
    edit: HWND,
    browse_title: String,
}

thread_local! {
    static DLG: RefCell<Option<DlgState>> = const { RefCell::new(None) };
}

const ID_BROWSE: i32 = 100;

unsafe extern "system" fn dlg_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            match id {
                1 | 2 => {
                    DLG.with(|d| {
                        if let Some(s) = d.borrow_mut().as_mut() {
                            s.done = true;
                            s.ok = id == 1;
                        }
                    });
                    return 0;
                }
                ID_BROWSE => {
                    let (edit, title) = DLG.with(|d| d.borrow().as_ref().map(|s| (s.edit, s.browse_title.clone()))).unwrap_or((std::ptr::null_mut(), String::new()));
                    if let Some(p) = browse_folder(hwnd, &title, &get_text(edit)) {
                        set_text(edit, &p);
                    }
                    return 0;
                }
                _ => {}
            }
        }
        WM_CLOSE => {
            DLG.with(|d| {
                if let Some(s) = d.borrow_mut().as_mut() {
                    s.done = true;
                    s.ok = false;
                }
            });
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn make_dialog(owner: HWND, title: &str, cw: i32, ch: i32) -> HWND {
    register_class("SmkDialog", Some(dlg_proc));
    let cn = wide("SmkDialog");
    let t = wide(title);
    unsafe {
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        AdjustWindowRectEx(&mut r, style, 0, WS_EX_DLGMODALFRAME);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            cn.as_ptr(),
            t.as_ptr(),
            style,
            0,
            0,
            r.right - r.left,
            r.bottom - r.top,
            owner,
            std::ptr::null_mut(),
            hinst(),
            std::ptr::null(),
        );
        center(hwnd, owner);
        hwnd
    }
}

/// "Select Setup Language". Returns chosen language id.
pub fn select_language(langs: &[String], default: &str) -> Option<String> {
    let ctx = Ctx { lang: default.to_string(), ..Default::default() };
    let s = dpi() as f32 / 96.0;
    let sc = |v: i32| (v as f32 * s) as i32;
    let f = make_fonts();
    let hwnd = make_dialog(std::ptr::null_mut(), &ctx.msg("SelectLanguageTitle"), sc(297), sc(125));
    let icon = child(hwnd, "STATIC", "", WS_VISIBLE | SS_ICON as u32, 0, sc(12), sc(12), sc(32), sc(32), -1, f.normal);
    unsafe {
        SendMessageW(icon, STM_SETICON, app_icon(sc(32)) as usize, 0);
    }
    child(hwnd, "STATIC", &ctx.msg("SelectLanguageLabel"), WS_VISIBLE | SS_NOPREFIX as u32, 0, sc(56), sc(12), sc(229), sc(36), -1, f.normal);
    let combo = child(hwnd, "COMBOBOX", "", WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32, 0, sc(56), sc(52), sc(229), sc(200), 10, f.normal);
    let mut sel = 0;
    for (i, l) in langs.iter().enumerate() {
        let n = wide(smk_core::messages::lang_name(l));
        unsafe {
            SendMessageW(combo, CB_ADDSTRING, 0, n.as_ptr() as isize);
        }
        if l == default {
            sel = i;
        }
    }
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, sel, 0);
    }
    child(hwnd, "BUTTON", &ctx.msg("ButtonOK"), WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, sc(129), sc(93), sc(75), sc(23), 1, f.normal);
    child(hwnd, "BUTTON", &ctx.msg("ButtonCancel"), WS_VISIBLE | WS_TABSTOP, 0, sc(210), sc(93), sc(75), sc(23), 2, f.normal);
    DLG.with(|d| *d.borrow_mut() = Some(DlgState { done: false, ok: false, combo, edit: std::ptr::null_mut(), browse_title: String::new() }));
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(combo);
        modal_loop(hwnd, &|| DLG.with(|d| d.borrow().as_ref().map(|s| s.done).unwrap_or(true)));
    }
    let ok = DLG.with(|d| d.borrow().as_ref().map(|s| s.ok).unwrap_or(false));
    let i = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    unsafe {
        DestroyWindow(hwnd);
    }
    if ok && i >= 0 {
        langs.get(i as usize).cloned()
    } else {
        None
    }
}

/// Ask for the folder that contains `file` (next disk / missing archive).
pub fn disk_prompt(owner: HWND, ctx: &Ctx, file: &str, disk: usize, dir: &str, prompt: &str) -> Option<PathBuf> {
    let s = dpi() as f32 / 96.0;
    let sc = |v: i32| (v as f32 * s) as i32;
    let f = make_fonts();
    let mut dir = dir.to_string();
    loop {
        let hwnd = make_dialog(owner, &ctx.msg("ChangeDiskTitle"), sc(377), sc(165));
        let text = if !prompt.is_empty() {
            prompt.to_string()
        } else {
            ctx.msgf("SelectDiskLabel2", &[&(disk.max(1) + 0).to_string()])
        };
        let icon = child(hwnd, "STATIC", "", WS_VISIBLE | SS_ICON as u32, 0, sc(12), sc(12), sc(32), sc(32), -1, f.normal);
        unsafe {
            SendMessageW(icon, STM_SETICON, LoadIconW(std::ptr::null_mut(), IDI_INFORMATION) as usize, 0);
        }
        child(hwnd, "STATIC", &format!("{}\n\n{}", text, file), WS_VISIBLE | SS_NOPREFIX as u32, 0, sc(56), sc(12), sc(309), sc(72), -1, f.normal);
        child(hwnd, "STATIC", &ctx.msg("PathLabel"), WS_VISIBLE, 0, sc(12), sc(92), sc(353), sc(14), -1, f.normal);
        let edit = child(hwnd, "EDIT", &dir, WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32, WS_EX_CLIENTEDGE, sc(12), sc(108), sc(272), sc(21), 11, f.normal);
        child(hwnd, "BUTTON", &ctx.msg("ButtonBrowse"), WS_VISIBLE | WS_TABSTOP, 0, sc(290), sc(107), sc(75), sc(23), ID_BROWSE, f.normal);
        child(hwnd, "BUTTON", &ctx.msg("ButtonOK"), WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, sc(209), sc(136), sc(75), sc(23), 1, f.normal);
        child(hwnd, "BUTTON", &ctx.msg("ButtonCancel"), WS_VISIBLE | WS_TABSTOP, 0, sc(290), sc(136), sc(75), sc(23), 2, f.normal);
        DLG.with(|d| *d.borrow_mut() = Some(DlgState { done: false, ok: false, combo: std::ptr::null_mut(), edit, browse_title: ctx.msg("BrowseDialogLabel") }));
        unsafe {
            if !owner.is_null() {
                EnableWindow(owner, 0);
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            SetFocus(edit);
            modal_loop(hwnd, &|| DLG.with(|d| d.borrow().as_ref().map(|s| s.done).unwrap_or(true)));
            if !owner.is_null() {
                EnableWindow(owner, 1);
            }
        }
        let ok = DLG.with(|d| d.borrow().as_ref().map(|s| s.ok).unwrap_or(false));
        dir = get_text(edit);
        unsafe {
            DestroyWindow(hwnd);
            if !owner.is_null() {
                SetForegroundWindow(owner);
            }
        }
        if !ok {
            return None;
        }
        let d = PathBuf::from(&dir);
        let name = std::path::Path::new(file).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if !crate::archives::find_archives(&d, &name).is_empty() {
            return Some(d);
        }
        crate::w::msgbox(owner, &ctx.msgf("FileNotInDir2", &[file, &dir]), &ctx.msg("ErrorTitle"), 0x30);
    }
}

