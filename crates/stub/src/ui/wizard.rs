//! The Inno Setup–style wizard window.
//!
//! Geometry follows Inno Setup 6 classic wizard at 96 DPI: client 497x360,
//! welcome image 164x314, header 58 px, inner page (40,72) 417x237,
//! bevel at y=313, buttons at y=327.

use super::*;
use crate::consts::Ctx;
use crate::install::{self, Ask, Installer, Reply, Selection, Shared};
use crate::payload::Payload;
use smk_core::project::Project;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Page {
    Welcome,
    License,
    Password,
    InfoBefore,
    Dir,
    Components,
    Group,
    Tasks,
    Ready,
    Installing,
    InfoAfter,
    Finished,
}

const ORDER: [Page; 12] = [
    Page::Welcome,
    Page::License,
    Page::Password,
    Page::InfoBefore,
    Page::Dir,
    Page::Components,
    Page::Group,
    Page::Tasks,
    Page::Ready,
    Page::Installing,
    Page::InfoAfter,
    Page::Finished,
];

const ID_NEXT: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_BACK: i32 = 3;
const ID_LIC_ACCEPT: i32 = 10;
const ID_LIC_DECLINE: i32 = 11;
const ID_DIR_BROWSE: i32 = 12;
const ID_GROUP_BROWSE: i32 = 13;
const ID_NOICONS: i32 = 14;
const ID_TYPES: i32 = 15;
const ID_COMP_TREE: i32 = 16;
const ID_TASK_TREE: i32 = 17;
const ID_DIR_EDIT: i32 = 18;

const WM_APP_DONE: u32 = WM_APP + 1;
const WM_APP_ASK: u32 = WM_APP + 2;
const WM_APP_TREE: u32 = WM_APP + 3;
const TIMER_ID: usize = 1;

pub struct Init {
    pub p: &'static Payload,
    pub ctx: Ctx,
    pub sel: Selection,
    pub tmp: PathBuf,
    /// /SILENT: show only the progress page, close automatically.
    pub silent: bool,
}

struct Ctl {
    h: HWND,
    page: Option<Page>,
}

struct Wizard {
    hwnd: HWND,
    s: f32,
    f: Fonts,
    p: &'static Payload,
    m: &'static Project,
    ctx: Ctx,
    sel: Selection,
    tmp: PathBuf,
    silent: bool,
    page: Page,
    ctls: Vec<Ctl>,
    white: Vec<HWND>,
    white_brush: HBRUSH,
    img_large: Option<Image>,
    img_small: Option<Image>,
    icon_big: HICON,
    // controls
    back: HWND,
    next: HWND,
    cancel: HWND,
    hdr_title: HWND,
    hdr_desc: HWND,
    wel_title: HWND,
    wel_text: HWND,
    fin_title: HWND,
    fin_text: HWND,
    fin_time: HWND,
    fin_checks: Vec<(HWND, usize)>,
    fin_yes: HWND,
    fin_no: HWND,
    lic_accept: HWND,
    pwd_edit: HWND,
    dir_edit: HWND,
    dir_space: HWND,
    comp_types: HWND,
    comp_tree: HWND,
    comp_space: HWND,
    group_edit: HWND,
    group_noicons: HWND,
    task_tree: HWND,
    ready_memo: HWND,
    inst_status: HWND,
    inst_file: HWND,
    inst_bar: HWND,
    inst_sub: HWND,
    inst_time1: HWND,
    inst_time2: HWND,
    // state
    comp_checked: Vec<bool>,
    comp_items: Vec<isize>,
    task_checked: Vec<bool>,
    task_items: Vec<(isize, usize)>,
    shared: Option<Arc<Shared>>,
    install_start: Option<Instant>,
    install_time: Option<std::time::Duration>,
    error: Option<String>,
    exit_code: i32,
    done: bool,
}

static mut WIZ: *mut Wizard = std::ptr::null_mut();

fn wiz() -> Option<&'static mut Wizard> {
    unsafe {
        let p = WIZ;
        if p.is_null() {
            None
        } else {
            Some(&mut *p)
        }
    }
}

fn text_res(p: &Payload, kind: &str, lang: &str) -> Option<Vec<u8>> {
    p.resource(&format!("{}:{}", kind, lang)).or_else(|| p.resource(kind))
}

fn has_text_res(p: &Payload, kind: &str, lang: &str) -> bool {
    p.has_resource(&format!("{}:{}", kind, lang)) || p.has_resource(kind)
}

// richedit.h uses #pragma pack(4)
#[repr(C, packed(4))]
#[allow(non_snake_case)]
struct EDITSTREAM {
    dwCookie: usize,
    dwError: u32,
    pfnCallback: Option<unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> u32>,
}
const EM_STREAMIN: u32 = WM_USER + 73;
const SF_RTF: u32 = 2;

struct StreamCookie {
    data: Vec<u8>,
    pos: usize,
}

unsafe extern "system" fn es_cb(cookie: usize, buf: *mut u8, cb: i32, pcb: *mut i32) -> u32 {
    let c = &mut *(cookie as *mut StreamCookie);
    let n = (c.data.len() - c.pos).min(cb as usize);
    std::ptr::copy_nonoverlapping(c.data.as_ptr().add(c.pos), buf, n);
    c.pos += n;
    *pcb = n as i32;
    0
}

fn set_rich(h: HWND, data: &[u8]) {
    if data.starts_with(b"{\\rtf") {
        let mut cookie = StreamCookie { data: data.to_vec(), pos: 0 };
        let mut es = EDITSTREAM { dwCookie: &mut cookie as *mut _ as usize, dwError: 0, pfnCallback: Some(es_cb) };
        unsafe {
            SendMessageW(h, EM_STREAMIN, SF_RTF as usize, &mut es as *mut _ as isize);
        }
    } else {
        let s = String::from_utf8_lossy(data.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(data)).to_string();
        set_text(h, &s);
    }
}

impl Wizard {
    fn sc(&self, v: i32) -> i32 {
        (v as f32 * self.s).round() as i32
    }

    fn add(&mut self, class: &str, text: &str, style: u32, ex: u32, page: Option<Page>, x: i32, y: i32, w: i32, h: i32, id: i32, white: bool) -> HWND {
        let font = self.f.normal;
        let hwnd = child(self.hwnd, class, text, style, ex, self.sc(x), self.sc(y), self.sc(w), self.sc(h), id, font);
        self.ctls.push(Ctl { h: hwnd, page });
        if white {
            self.white.push(hwnd);
        }
        hwnd
    }

    /// Inner page control (coordinates relative to the inner area).
    fn inner(&mut self, class: &str, text: &str, style: u32, ex: u32, page: Page, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
        self.add(class, text, style, ex, Some(page), 40 + x, 72 + y, w, h, id, false)
    }

    fn label(&mut self, page: Page, text: &str, x: i32, y: i32, w: i32, h: i32) -> HWND {
        self.inner("STATIC", text, SS_NOPREFIX as u32, 0, page, x, y, w, h, -1)
    }

    fn create(&mut self) {
        let c = self.ctx.clone();
        let m = self.m;
        let tab = WS_TABSTOP;
        // buttons (always present)
        self.back = self.add("BUTTON", &c.msg("ButtonBack"), tab, 0, None, 249, 327, 75, 23, ID_BACK, false);
        self.next = self.add("BUTTON", &c.msg("ButtonNext"), tab | BS_DEFPUSHBUTTON as u32, 0, None, 324, 327, 75, 23, ID_NEXT, false);
        self.cancel = self.add("BUTTON", &c.msg("ButtonCancel"), tab, 0, None, 414, 327, 75, 23, ID_CANCEL, false);

        // inner header
        self.hdr_title = self.add("STATIC", "", SS_NOPREFIX as u32, 0, None, 24, 10, 404, 16, -1, true);
        set_font(self.hdr_title, self.f.bold);
        self.hdr_desc = self.add("STATIC", "", SS_NOPREFIX as u32, 0, None, 40, 26, 388, 30, -1, true);

        // welcome
        self.wel_title = self.add("STATIC", &c.msg("WelcomeLabel1"), SS_NOPREFIX as u32, 0, Some(Page::Welcome), 176, 16, 301, 54, -1, true);
        set_font(self.wel_title, self.f.big);
        self.wel_text = self.add("STATIC", &c.msg("WelcomeLabel2"), SS_NOPREFIX as u32, 0, Some(Page::Welcome), 176, 76, 301, 234, -1, true);

        // license
        let lp = Page::License;
        self.label(lp, &c.msg("LicenseLabel3"), 0, 0, 417, 28);
        let ml = (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32 | WS_VSCROLL | tab;
        let lic = self.inner("RICHEDIT50W", "", ml, WS_EX_CLIENTEDGE, lp, 0, 38, 417, 138, -1);
        if let Some(t) = text_res(self.p, "license", &c.lang) {
            set_rich(lic, &t);
        }
        self.lic_accept = self.inner("BUTTON", &c.msg("LicenseAccepted"), BS_AUTORADIOBUTTON as u32 | tab | WS_GROUP, 0, lp, 0, 184, 417, 17, ID_LIC_ACCEPT);
        let dec = self.inner("BUTTON", &c.msg("LicenseNotAccepted"), BS_AUTORADIOBUTTON as u32, 0, lp, 0, 204, 417, 17, ID_LIC_DECLINE);
        set_checked(dec, true);

        // password
        let pp = Page::Password;
        self.label(pp, &c.msg("PasswordLabel3"), 0, 0, 417, 28);
        self.inner("STATIC", &c.msg("PasswordEditLabel"), 0, 0, pp, 0, 42, 417, 14, -1);
        self.pwd_edit = self.inner("EDIT", "", ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | tab, WS_EX_CLIENTEDGE, pp, 0, 60, 417, 21, -1);

        // info before / after
        for (page, kind, click) in [(Page::InfoBefore, "infobefore", "InfoBeforeClickLabel"), (Page::InfoAfter, "infoafter", "InfoAfterClickLabel")] {
            self.label(page, &c.msg(click), 0, 0, 417, 14);
            let e = self.inner("RICHEDIT50W", "", ml, WS_EX_CLIENTEDGE, page, 0, 24, 417, 213, -1);
            if let Some(t) = text_res(self.p, kind, &c.lang) {
                set_rich(e, &t);
            }
        }

        // select dir
        let dp = Page::Dir;
        let ic = self.inner("STATIC", "", SS_ICON as u32, 0, dp, 0, 0, 32, 32, -1);
        unsafe {
            SendMessageW(ic, STM_SETICON, folder_icon() as usize, 0);
        }
        self.label(dp, &c.msg("SelectDirLabel3"), 44, 8, 373, 30);
        self.label(dp, &c.msg("SelectDirBrowseLabel"), 0, 46, 417, 30);
        self.dir_edit = self.inner("EDIT", &self.sel.dir.clone(), ES_AUTOHSCROLL as u32 | tab, WS_EX_CLIENTEDGE, dp, 0, 82, 332, 21, ID_DIR_EDIT);
        self.inner("BUTTON", &c.msg("ButtonBrowse"), tab, 0, dp, 342, 81, 75, 23, ID_DIR_BROWSE);
        self.dir_space = self.label(dp, "", 0, 221, 417, 16);

        // components
        let cp = Page::Components;
        self.label(cp, &c.msg("SelectComponentsLabel2"), 0, 0, 417, 28);
        let tree_top = if m.types.is_empty() { 32 } else { 60 };
        if !m.types.is_empty() {
            self.comp_types = self.inner("COMBOBOX", "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL | tab, 0, cp, 0, 32, 417, 200, ID_TYPES);
            for t in &m.types {
                let n = wide(&c.expand(&t.description));
                unsafe {
                    SendMessageW(self.comp_types, CB_ADDSTRING, 0, n.as_ptr() as isize);
                }
            }
        }
        let tv = (TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_DISABLEDRAGDROP | TVS_NOHSCROLL) as u32 | tab;
        self.comp_tree = self.inner("SysTreeView32", "", tv, WS_EX_CLIENTEDGE, cp, 0, tree_top, 417, 214 - tree_top, ID_COMP_TREE);
        self.comp_space = self.label(cp, "", 0, 221, 417, 16);

        // start menu group
        let gp = Page::Group;
        let ic = self.inner("STATIC", "", SS_ICON as u32, 0, gp, 0, 0, 32, 32, -1);
        unsafe {
            SendMessageW(ic, STM_SETICON, group_icon() as usize, 0);
        }
        self.label(gp, &c.msg("SelectStartMenuFolderLabel3"), 44, 8, 373, 30);
        self.label(gp, &c.msg("SelectStartMenuFolderBrowseLabel"), 0, 46, 417, 30);
        self.group_edit = self.inner("EDIT", &self.sel.group.clone(), ES_AUTOHSCROLL as u32 | tab, WS_EX_CLIENTEDGE, gp, 0, 82, 332, 21, -1);
        self.inner("BUTTON", &c.msg("ButtonBrowse"), tab, 0, gp, 342, 81, 75, 23, ID_GROUP_BROWSE);
        if m.setup.allow_no_icons {
            self.group_noicons = self.inner("BUTTON", &c.msg("NoProgramGroupCheck2"), BS_AUTOCHECKBOX as u32 | tab, 0, gp, 0, 220, 417, 17, ID_NOICONS);
            set_checked(self.group_noicons, self.sel.no_icons);
        }

        // tasks
        let tp = Page::Tasks;
        self.label(tp, &c.msg("SelectTasksLabel2"), 0, 0, 417, 28);
        self.task_tree = self.inner("SysTreeView32", "", tv, WS_EX_CLIENTEDGE, tp, 0, 36, 417, 201, ID_TASK_TREE);

        // ready
        let rp = Page::Ready;
        let ready_lbl = if m.setup.show_ready_page { "ReadyLabel2a" } else { "ReadyLabel2b" };
        self.label(rp, &c.msg(ready_lbl), 0, 0, 417, 28);
        self.ready_memo = self.inner("EDIT", "", (ES_MULTILINE | ES_READONLY) as u32 | WS_VSCROLL | tab, WS_EX_CLIENTEDGE, rp, 0, 36, 417, 201, -1);
        self.white.push(self.ready_memo);

        // installing
        let ip = Page::Installing;
        self.inst_status = self.label(ip, &c.msg("StatusPreparing"), 0, 0, 417, 16);
        self.inst_file = self.inner("STATIC", "", SS_NOPREFIX as u32 | SS_PATHELLIPSIS as u32, 0, ip, 0, 20, 417, 16, -1);
        self.inst_bar = self.inner("msctls_progress32", "", 0, 0, ip, 0, 42, 417, 21, -1);
        self.inst_sub = self.inner("msctls_progress32", "", 0, 0, ip, 0, 70, 417, 10, -1);
        self.inst_time1 = self.label(ip, "", 0, 92, 208, 16);
        self.inst_time2 = self.label(ip, "", 209, 92, 208, 16);
        unsafe {
            SendMessageW(self.inst_bar, PBM_SETRANGE32, 0, 1000);
            SendMessageW(self.inst_sub, PBM_SETRANGE32, 0, 1000);
        }

        // finished
        self.fin_title = self.add("STATIC", &c.msg("FinishedHeadingLabel"), SS_NOPREFIX as u32, 0, Some(Page::Finished), 176, 16, 301, 54, -1, true);
        set_font(self.fin_title, self.f.big);
        self.fin_text = self.add("STATIC", "", SS_NOPREFIX as u32, 0, Some(Page::Finished), 176, 76, 301, 100, -1, true);
        for (i, r) in m.run.iter().enumerate() {
            if r.postinstall {
                let h = self.add("BUTTON", &c.expand(&r.description), BS_AUTOCHECKBOX as u32 | tab, 0, Some(Page::Finished), 176, 180, 301, 17, 100 + i as i32, true);
                set_checked(h, !r.unchecked);
                self.fin_checks.push((h, i));
            }
        }
        self.fin_yes = self.add("BUTTON", &c.msg("YesRadio"), BS_AUTORADIOBUTTON as u32 | tab | WS_GROUP, 0, Some(Page::Finished), 176, 180, 301, 17, -1, true);
        self.fin_no = self.add("BUTTON", &c.msg("NoRadio"), BS_AUTORADIOBUTTON as u32, 0, Some(Page::Finished), 176, 200, 301, 17, -1, true);
        set_checked(self.fin_yes, true);
        self.fin_time = self.add("STATIC", "", SS_NOPREFIX as u32, 0, Some(Page::Finished), 176, 290, 301, 16, -1, true);

        self.build_components();
    }

    // ---------- page logic ----------

    fn uses_group(&self) -> bool {
        self.m.shortcuts.iter().any(|s| s.name.to_ascii_lowercase().starts_with("{group}"))
    }

    fn visible_tasks(&self) -> Vec<usize> {
        let comps = self.selected_components();
        self.m
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                smk_core::expr::eval_str(&t.components, &mut |n, _| comps.iter().any(|c| c.eq_ignore_ascii_case(n)))
                    && install::check_ok(&t.check, &self.ctx)
                    && crate::w::version_at_least(&t.min_version)
                    && (t.only_below_version.is_empty() || !crate::w::version_at_least(&t.only_below_version))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn enabled(&self, p: Page) -> bool {
        let s = &self.m.setup;
        let lang = &self.ctx.lang;
        match p {
            Page::Welcome => s.show_welcome_page,
            Page::License => has_text_res(self.p, "license", lang),
            Page::Password => !self.p.header.password_hash.is_empty(),
            Page::InfoBefore => has_text_res(self.p, "infobefore", lang),
            Page::Dir => s.show_dir_page,
            // nothing to choose when every component is fixed
            Page::Components => self.m.components.iter().any(|c| !c.fixed),
            Page::Group => s.show_group_page && self.uses_group(),
            Page::Tasks => !self.visible_tasks().is_empty(),
            Page::Ready => s.show_ready_page,
            Page::Installing => true,
            Page::InfoAfter => has_text_res(self.p, "infoafter", lang),
            Page::Finished => s.show_finished_page || self.error.is_some(),
        }
    }

    fn show_page(&mut self, p: Page) {
        self.page = p;
        let full = matches!(p, Page::Welcome | Page::Finished);
        for c in &self.ctls {
            if let Some(cp) = c.page {
                show(c.h, cp == p);
            }
        }
        show(self.hdr_title, !full);
        show(self.hdr_desc, !full);
        let c = &self.ctx;
        let (t, d) = match p {
            Page::License => ("WizardLicense", "LicenseLabel"),
            Page::Password => ("WizardPassword", "PasswordLabel1"),
            Page::InfoBefore => ("WizardInfoBefore", "InfoBeforeLabel"),
            Page::Dir => ("WizardSelectDir", "SelectDirDesc"),
            Page::Components => ("WizardSelectComponents", "SelectComponentsDesc"),
            Page::Group => ("WizardSelectProgramGroup", "SelectStartMenuFolderDesc"),
            Page::Tasks => ("WizardSelectTasks", "SelectTasksDesc"),
            Page::Ready => ("WizardReady", "ReadyLabel1"),
            Page::Installing => ("WizardInstalling", "InstallingLabel"),
            Page::InfoAfter => ("WizardInfoAfter", "InfoAfterLabel"),
            _ => ("", ""),
        };
        if !t.is_empty() {
            set_text(self.hdr_title, &c.msg(t));
            set_text(self.hdr_desc, &c.msg(d));
        }
        // buttons
        let first = ORDER.iter().position(|x| *x == p).unwrap_or(0);
        let has_prev = ORDER[..first].iter().any(|x| self.enabled(*x)) && first < 9;
        show(self.back, has_prev && !matches!(p, Page::Installing | Page::InfoAfter | Page::Finished));
        enable(self.back, true);
        let next_text = match p {
            Page::Ready => c.msg("ButtonInstall"),
            Page::Finished => c.msg("ButtonFinish"),
            _ => c.msg("ButtonNext"),
        };
        set_text(self.next, &next_text);
        show(self.next, p != Page::Installing);
        enable(self.next, true);
        show(self.cancel, p != Page::Finished);
        enable(self.cancel, true);
        // if the ready page is skipped, the last page before install shows "Install"
        if p != Page::Ready && p < Page::Installing && self.next_page(p) == Some(Page::Installing) {
            set_text(self.next, &c.msg("ButtonInstall"));
        }
        self.on_enter(p);
        unsafe {
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
        let focus = if p == Page::Installing { self.cancel } else { self.next };
        unsafe {
            SetFocus(focus);
        }
    }

    fn next_page(&self, from: Page) -> Option<Page> {
        let i = ORDER.iter().position(|x| *x == from)?;
        ORDER[i + 1..].iter().copied().find(|p| self.enabled(*p))
    }

    fn prev_page(&self, from: Page) -> Option<Page> {
        let i = ORDER.iter().position(|x| *x == from)?;
        ORDER[..i].iter().rev().copied().find(|p| self.enabled(*p))
    }

    fn required_bytes(&self) -> u64 {
        let comps = self.selected_components();
        let sel = Selection { components: comps.clone(), tasks: self.m.tasks.iter().map(|t| t.name.clone()).collect(), ..self.sel.clone() };
        let mut total = self.m.setup.extra_disk_space;
        for s in &self.p.header.streams {
            let c = smk_core::project::Conditions { tasks: String::new(), ..s.when.clone() };
            if install::cond_ok(&c, &sel, &self.ctx) {
                total += s.raw_size;
            }
        }
        for a in &self.m.archives {
            let c = smk_core::project::Conditions { tasks: String::new(), ..a.when.clone() };
            if install::cond_ok(&c, &sel, &self.ctx) {
                total += install::archive_weight(a, &self.ctx);
            }
        }
        for c in &self.m.components {
            if comps.iter().any(|x| x.eq_ignore_ascii_case(&c.name)) {
                total += c.extra_disk_space;
            }
        }
        total
    }

    fn mb_text(bytes: u64) -> String {
        let mb = bytes as f64 / 1048576.0;
        if mb < 10.0 {
            format!("{:.1}", mb)
        } else {
            format!("{:.0}", mb.ceil())
        }
    }

    fn on_enter(&mut self, p: Page) {
        match p {
            Page::Dir => {
                let t = self.ctx.msg("DiskSpaceMBLabel").replace("[mb]", &Self::mb_text(self.required_bytes()));
                set_text(self.dir_space, &t);
            }
            Page::Components => self.update_comp_space(),
            Page::Tasks => self.build_tasks(),
            Page::Ready => self.build_ready(),
            Page::Finished => self.build_finished(),
            Page::License => {
                enable(self.next, is_checked(self.lic_accept));
            }
            _ => {}
        }
    }

    fn validate_dir(&mut self) -> bool {
        let c = self.ctx.clone();
        let d = get_text(self.dir_edit).trim().trim_end_matches('\\').to_string();
        let title = c.msg("SetupAppTitle");
        let b = d.as_bytes();
        let ok_form = (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()) || d.starts_with("\\\\");
        if !ok_form || d.contains(['*', '?', '"', '<', '>', '|']) {
            crate::w::msgbox(self.hwnd, &c.msg("InvalidPath"), &title, 0x30);
            return false;
        }
        if d.len() <= 3 && !d.starts_with("\\\\") && !self.m.setup.allow_root_directory {
            crate::w::msgbox(self.hwnd, &c.msg("RootDirNotAllowed"), &title, 0x30);
            return false;
        }
        let root = if d.len() >= 2 && b[1] == b':' { format!("{}\\", &d[..2]) } else { d.clone() };
        if !Path::new(&root).exists() {
            crate::w::msgbox(self.hwnd, &c.msg("InvalidDrive"), &title, 0x30);
            return false;
        }
        let need = self.required_bytes();
        if let Some(free) = crate::w::free_space(Path::new(&d)) {
            if free < need {
                let q = c.msgf("DiskSpaceWarning", &[&Self::mb_text(need), &Self::mb_text(free)]);
                if crate::w::msgbox(self.hwnd, &q, &c.msg("DiskSpaceWarningTitle"), 0x34) != 6 {
                    return false;
                }
            }
        }
        self.sel.dir = if d.len() == 2 { format!("{}\\", d) } else { d };
        true
    }

    fn leave(&mut self, p: Page) -> bool {
        match p {
            Page::Password => {
                let pw = get_text(self.pwd_edit);
                let h = smk_core::format::password_hash(&pw, &self.p.header.password_salt);
                if h != self.p.header.password_hash {
                    crate::w::msgbox(self.hwnd, &self.ctx.msg("IncorrectPassword"), &self.ctx.msg("SetupAppTitle"), 0x30);
                    unsafe {
                        SetFocus(self.pwd_edit);
                    }
                    return false;
                }
            }
            Page::Dir => return self.validate_dir(),
            Page::Components => self.sel.components = self.selected_components(),
            Page::Group => {
                self.sel.no_icons = !self.group_noicons.is_null() && is_checked(self.group_noicons);
                let g = get_text(self.group_edit).trim().trim_matches('\\').to_string();
                if g.is_empty() && !self.sel.no_icons {
                    crate::w::msgbox(self.hwnd, &self.ctx.msg("MustEnterGroupName"), &self.ctx.msg("SetupAppTitle"), 0x30);
                    return false;
                }
                self.sel.group = g;
            }
            Page::Tasks => self.sel.tasks = self.selected_tasks(),
            _ => {}
        }
        true
    }

    fn go_next(&mut self) {
        if self.page == Page::Finished {
            self.finish();
            return;
        }
        if !self.leave(self.page) {
            return;
        }
        // pages that were skipped still need a selection
        if !self.enabled(Page::Components) {
            self.sel.components.clear();
        } else if self.page < Page::Components {
            self.sel.components = self.selected_components();
        }
        if !self.enabled(Page::Tasks) {
            self.sel.tasks.clear();
        } else if self.page < Page::Tasks {
            self.sel.tasks = self.default_tasks();
        }
        if let Some(n) = self.next_page(self.page) {
            if n == Page::Installing {
                self.show_page(n);
                self.start_install();
            } else {
                self.show_page(n);
            }
        } else {
            self.finish();
        }
    }

    fn go_back(&mut self) {
        if let Some(p) = self.prev_page(self.page) {
            self.show_page(p);
        }
    }

    fn confirm_exit(&self) -> bool {
        let c = &self.ctx;
        crate::w::msgbox(self.hwnd, &c.msg("ExitSetupMessage"), &c.msg("ExitSetupTitle"), 0x24) == 6
    }

    fn on_cancel(&mut self) {
        match self.page {
            Page::Finished => self.finish(),
            Page::Installing => {
                if let Some(sh) = &self.shared {
                    if !sh.cancelled() && self.confirm_exit() {
                        sh.cancel.store(true, Ordering::SeqCst);
                        enable(self.cancel, false);
                    }
                }
            }
            _ => {
                if self.confirm_exit() {
                    self.exit_code = 2;
                    self.close();
                }
            }
        }
    }

    fn close(&mut self) {
        self.done = true;
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }

    fn finish(&mut self) {
        if self.error.is_none() {
            let restart = self.m.setup.restart_required && is_checked(self.fin_yes);
            for (h, i) in &self.fin_checks {
                let r = &self.m.run[*i];
                if crate::ui::wizard::is_visible(*h) && is_checked(*h) {
                    let mut r = r.clone();
                    // postinstall entries default to nowait (don't block the wizard)
                    r.nowait = true;
                    install::run_item(&r, &self.ctx);
                }
            }
            if restart {
                crate::w::reboot();
            }
        }
        self.close();
    }

    // ---------- components ----------

    fn comp_parent(&self, i: usize) -> Option<usize> {
        let name = &self.m.components[i].name;
        let (parent, _) = name.rsplit_once('\\')?;
        self.m.components.iter().position(|c| c.name.eq_ignore_ascii_case(parent))
    }

    fn comp_children(&self, i: usize) -> Vec<usize> {
        (0..self.m.components.len()).filter(|&j| self.comp_parent(j) == Some(i)).collect()
    }

    fn selected_components(&self) -> Vec<String> {
        self.m.components.iter().zip(&self.comp_checked).filter(|(_, c)| **c).map(|(c, _)| c.name.clone()).collect()
    }

    fn apply_type(&mut self, t: usize) {
        let Some(ty) = self.m.types.get(t) else { return };
        if ty.custom {
            return;
        }
        for (i, c) in self.m.components.iter().enumerate() {
            self.comp_checked[i] = c.fixed || c.types.iter().any(|x| x.eq_ignore_ascii_case(&ty.name));
        }
    }

    fn build_components(&mut self) {
        let n = self.m.components.len();
        if n == 0 {
            return;
        }
        self.comp_checked = self.m.components.iter().map(|c| c.checked || c.fixed).collect();
        // preselected components from the command line
        if !self.sel.components.is_empty() {
            for (i, c) in self.m.components.iter().enumerate() {
                self.comp_checked[i] = c.fixed || self.sel.components.iter().any(|x| x.eq_ignore_ascii_case(&c.name));
            }
        } else if !self.m.types.is_empty() {
            let t = self.m.types.iter().position(|t| t.name.eq_ignore_ascii_case(&self.sel.setup_type)).unwrap_or(0);
            unsafe {
                SendMessageW(self.comp_types, CB_SETCURSEL, t, 0);
            }
            self.apply_type(t);
        }
        set_state_images(self.comp_tree, self.s);
        self.comp_items = vec![0; n];
        let mut items: Vec<isize> = vec![0; n];
        for i in 0..n {
            let parent = self.comp_parent(i).map(|p| items[p]).unwrap_or(TVI_ROOT as isize);
            let text = self.ctx.expand(&self.m.components[i].description);
            items[i] = tree_insert(self.comp_tree, parent, &text, i as isize, 0);
        }
        self.comp_items = items;
        for i in 0..n {
            tree_expand(self.comp_tree, self.comp_items[i]);
        }
        self.sync_comp_tree();
    }

    fn sync_comp_tree(&self) {
        for (i, c) in self.m.components.iter().enumerate() {
            let img = match (c.exclusive, self.comp_checked[i]) {
                (false, false) => 1,
                (false, true) => 2,
                (true, false) => 3,
                (true, true) => 4,
            };
            tree_set_state(self.comp_tree, self.comp_items[i], img);
        }
    }

    fn set_comp(&mut self, i: usize, on: bool) {
        self.comp_checked[i] = on;
        let kids = self.comp_children(i);
        if on {
            // non-exclusive children follow; for an exclusive group keep/select one
            let excl: Vec<usize> = kids.iter().copied().filter(|&k| self.m.components[k].exclusive).collect();
            for &k in &kids {
                if !self.m.components[k].exclusive {
                    self.set_comp(k, true);
                }
            }
            if !excl.is_empty() && !excl.iter().any(|&k| self.comp_checked[k]) {
                self.set_comp(excl[0], true);
            }
        } else {
            for k in kids {
                if !self.m.components[k].fixed {
                    self.set_comp(k, false);
                }
            }
        }
    }

    fn toggle_comp(&mut self, i: usize) {
        let c = self.m.components[i].clone();
        let cur = self.comp_checked[i];
        if c.fixed {
            return;
        }
        if c.exclusive {
            if cur {
                return; // radio can't be unchecked directly
            }
            if let Some(p) = self.comp_parent(i) {
                for k in self.comp_children(p) {
                    if k != i && self.m.components[k].exclusive {
                        self.set_comp(k, false);
                    }
                }
            } else {
                for k in 0..self.m.components.len() {
                    if k != i && self.m.components[k].exclusive && self.comp_parent(k).is_none() {
                        self.set_comp(k, false);
                    }
                }
            }
        }
        self.set_comp(i, !cur);
        if !cur {
            let mut p = self.comp_parent(i);
            while let Some(pi) = p {
                self.comp_checked[pi] = true;
                p = self.comp_parent(pi);
            }
        }
        // switch type combo to "custom" if selection no longer matches a type
        if !self.m.types.is_empty() {
            let matching = self.m.types.iter().position(|t| {
                !t.custom
                    && self.m.components.iter().enumerate().all(|(k, c)| self.comp_checked[k] == (c.fixed || c.types.iter().any(|x| x.eq_ignore_ascii_case(&t.name))))
            });
            let idx = matching.or_else(|| self.m.types.iter().position(|t| t.custom));
            if let Some(idx) = idx {
                unsafe {
                    SendMessageW(self.comp_types, CB_SETCURSEL, idx, 0);
                }
            }
        }
        self.sync_comp_tree();
        self.update_comp_space();
    }

    fn update_comp_space(&self) {
        if self.comp_space.is_null() {
            return;
        }
        let t = self.ctx.msg("ComponentsDiskSpaceMBLabel").replace("[mb]", &Self::mb_text(self.required_bytes()));
        set_text(self.comp_space, &t);
    }

    // ---------- tasks ----------

    fn default_tasks(&self) -> Vec<String> {
        if !self.sel.tasks.is_empty() {
            return self.sel.tasks.clone();
        }
        let vis = self.visible_tasks();
        let mut out = Vec::new();
        let mut excl_done = std::collections::HashSet::new();
        for i in vis {
            let t = &self.m.tasks[i];
            if t.unchecked {
                continue;
            }
            if t.exclusive {
                if !excl_done.insert(t.group.clone()) {
                    continue;
                }
            }
            out.push(t.name.clone());
        }
        out
    }

    fn build_tasks(&mut self) {
        let vis = self.visible_tasks();
        let current: Vec<String> = if self.task_items.is_empty() { self.default_tasks() } else { self.selected_tasks() };
        unsafe {
            SendMessageW(self.task_tree, TVM_DELETEITEM, 0, TVI_ROOT as isize);
        }
        set_state_images(self.task_tree, self.s);
        self.task_checked = self.m.tasks.iter().map(|t| current.iter().any(|c| c.eq_ignore_ascii_case(&t.name))).collect();
        self.task_items.clear();
        let mut last_group: Option<String> = None;
        let mut parent = TVI_ROOT as isize;
        for i in vis {
            let t = &self.m.tasks[i];
            if !t.group.is_empty() && last_group.as_deref() != Some(t.group.as_str()) {
                parent = tree_insert(self.task_tree, TVI_ROOT as isize, &self.ctx.expand(&t.group).replace('&', ""), -1, 0);
                last_group = Some(t.group.clone());
            } else if t.group.is_empty() {
                parent = TVI_ROOT as isize;
                last_group = None;
            }
            let text = self.ctx.expand(&t.description).replace('&', "");
            let h = tree_insert(self.task_tree, parent, &text, i as isize, 0);
            self.task_items.push((h, i));
            if parent != TVI_ROOT as isize {
                tree_expand(self.task_tree, parent);
            }
        }
        self.sync_task_tree();
    }

    fn sync_task_tree(&self) {
        for (h, i) in &self.task_items {
            let t = &self.m.tasks[*i];
            let img = match (t.exclusive, self.task_checked[*i]) {
                (false, false) => 1,
                (false, true) => 2,
                (true, false) => 3,
                (true, true) => 4,
            };
            tree_set_state(self.task_tree, *h, img);
        }
    }

    fn toggle_task(&mut self, i: usize) {
        let t = self.m.tasks[i].clone();
        if t.exclusive {
            if self.task_checked[i] {
                return;
            }
            for (_, k) in &self.task_items {
                let o = &self.m.tasks[*k];
                if *k != i && o.exclusive && o.group == t.group {
                    self.task_checked[*k] = false;
                }
            }
            self.task_checked[i] = true;
        } else {
            self.task_checked[i] = !self.task_checked[i];
        }
        self.sync_task_tree();
    }

    fn selected_tasks(&self) -> Vec<String> {
        self.task_items.iter().filter(|(_, i)| self.task_checked[*i]).map(|(_, i)| self.m.tasks[*i].name.clone()).collect()
    }

    fn on_tree_click(&mut self, tree_id: i32, item: isize) {
        let data = tree_get_param(if tree_id == ID_COMP_TREE { self.comp_tree } else { self.task_tree }, item);
        if data < 0 {
            return;
        }
        if tree_id == ID_COMP_TREE {
            self.toggle_comp(data as usize);
        } else {
            self.toggle_task(data as usize);
        }
    }

    // ---------- ready / finished ----------

    fn build_ready(&mut self) {
        let c = &self.ctx;
        let mut s = String::new();
        let sp = "      ";
        if self.enabled(Page::Dir) || self.m.setup.always_show_dir_on_ready {
            s.push_str(&format!("{}\n{}{}\n\n", c.msg("ReadyMemoDir"), sp, self.sel.dir));
        }
        if !self.m.types.is_empty() {
            let i = unsafe { SendMessageW(self.comp_types, CB_GETCURSEL, 0, 0) };
            if let Some(t) = self.m.types.get(i.max(0) as usize) {
                s.push_str(&format!("{}\n{}{}\n\n", c.msg("ReadyMemoType"), sp, c.expand(&t.description)));
            }
        }
        if !self.m.components.is_empty() {
            s.push_str(&format!("{}\n", c.msg("ReadyMemoComponents")));
            for (i, comp) in self.m.components.iter().enumerate() {
                if self.comp_checked[i] {
                    let depth = comp.name.matches('\\').count();
                    s.push_str(&format!("{}{}{}\n", sp, "   ".repeat(depth), c.expand(&comp.description)));
                }
            }
            s.push('\n');
        }
        if self.uses_group() && !self.sel.no_icons && (self.enabled(Page::Group) || self.m.setup.always_show_group_on_ready) {
            s.push_str(&format!("{}\n{}{}\n\n", c.msg("ReadyMemoGroup"), sp, self.sel.group));
        }
        let tasks = self.sel.tasks.clone();
        if !tasks.is_empty() {
            s.push_str(&format!("{}\n", c.msg("ReadyMemoTasks")));
            let mut last_group = String::new();
            for t in &self.m.tasks {
                if tasks.iter().any(|x| x.eq_ignore_ascii_case(&t.name)) {
                    if !t.group.is_empty() && t.group != last_group {
                        s.push_str(&format!("{}{}\n", sp, c.expand(&t.group).replace('&', "")));
                        last_group = t.group.clone();
                    }
                    let ind = if t.group.is_empty() { sp.to_string() } else { format!("{}   ", sp) };
                    s.push_str(&format!("{}{}\n", ind, c.expand(&t.description).replace('&', "")));
                }
            }
        }
        set_text(self.ready_memo, s.trim_end());
    }

    fn build_finished(&mut self) {
        let c = self.ctx.clone();
        if let Some(e) = &self.error {
            set_text(self.fin_title, &c.msg("SetupAbortedHeading"));
            set_text(self.fin_text, &format!("{}\n\n{}", c.msg("SetupAborted"), e.lines().next().unwrap_or("")));
            for (h, _) in &self.fin_checks {
                show(*h, false);
            }
            show(self.fin_yes, false);
            show(self.fin_no, false);
            show(self.fin_time, false);
            unsafe {
                let t = wide(&c.msg("ErrorTitle"));
                SetWindowTextW(self.hwnd, t.as_ptr());
            }
            return;
        }
        let restart = self.m.setup.restart_required;
        let key = if restart {
            "FinishedRestartLabel"
        } else if self.m.shortcuts.is_empty() || self.sel.no_icons {
            "FinishedLabelNoIcons"
        } else {
            "FinishedLabel"
        };
        let mut text = c.msg(key);
        if !restart {
            text.push_str("\n\n");
            text.push_str(&c.msg("ClickFinish"));
        }
        // measure text height to place checkboxes below it
        let lines = estimate_lines(self.hwnd, self.f.normal, &text, self.sc(301));
        let h = (lines * 15 + 4).min(200);
        unsafe {
            SetWindowPos(self.fin_text, std::ptr::null_mut(), self.sc(176), self.sc(76), self.sc(301), self.sc(h), SWP_NOZORDER);
        }
        set_text(self.fin_text, &text);
        let mut y = 76 + h + 10;
        show(self.fin_yes, restart);
        show(self.fin_no, restart);
        if restart {
            for (hh, _) in &self.fin_checks {
                show(*hh, false);
            }
            unsafe {
                SetWindowPos(self.fin_yes, std::ptr::null_mut(), self.sc(176), self.sc(y), 0, 0, SWP_NOSIZE | SWP_NOZORDER);
                SetWindowPos(self.fin_no, std::ptr::null_mut(), self.sc(176), self.sc(y + 22), 0, 0, SWP_NOSIZE | SWP_NOZORDER);
            }
        } else {
            for (hh, i) in self.fin_checks.clone() {
                let r = &self.m.run[i];
                let vis = install::cond_ok(&r.when, &self.sel, &self.ctx) && !(r.skip_if_silent && self.sel.silent);
                show(hh, vis);
                if vis {
                    unsafe {
                        SetWindowPos(hh, std::ptr::null_mut(), self.sc(176), self.sc(y), 0, 0, SWP_NOSIZE | SWP_NOZORDER);
                    }
                    y += 22;
                }
            }
        }
        if let (true, Some(d)) = (self.m.setup.show_install_time, self.install_time) {
            set_text(self.fin_time, &format!("{} {}", c.msg("AllElapsedTime"), fmt_dur(d.as_secs())));
        } else {
            show(self.fin_time, false);
        }
    }

    // ---------- installation ----------

    fn start_install(&mut self) {
        enable(self.back, false);
        let hwnd = self.hwnd as usize;
        let ask = Box::new(move |a: Ask| -> Reply {
            let mut boxed: (Ask, Option<Reply>) = (a, None);
            unsafe {
                SendMessageW(hwnd as HWND, WM_APP_ASK, 0, &mut boxed as *mut _ as isize);
            }
            boxed.1.unwrap_or(Reply::Abort)
        });
        let sh = Shared::new(ask);
        self.shared = Some(sh.clone());
        self.install_start = Some(Instant::now());
        let p = self.p;
        let m = self.m;
        let mut ctx = self.ctx.clone();
        ctx.app = self.sel.dir.clone();
        ctx.group = self.sel.group.clone();
        self.ctx = ctx.clone();
        let sel = self.sel.clone();
        let tmp = self.tmp.clone();
        unsafe {
            SetTimer(self.hwnd, TIMER_ID, 100, None);
        }
        std::thread::spawn(move || {
            crate::shell::com_init();
            let mut ins = Installer::new(p, m, ctx, sel, sh.clone(), tmp);
            let r = ins.run();
            if let Err(e) = &r {
                if m.setup.rollback_on_error {
                    ins.rollback();
                }
                let mut pr = sh.progress.lock().unwrap();
                if e == "__cancel__" {
                    pr.cancelled = true;
                } else {
                    pr.error = Some(e.clone());
                }
            }
            sh.progress.lock().unwrap().finished = true;
            unsafe {
                PostMessageW(hwnd as HWND, WM_APP_DONE, 0, 0);
            }
        });
    }

    fn on_timer(&mut self) {
        let Some(sh) = &self.shared else { return };
        let pr = sh.progress.lock().unwrap().clone();
        let pos = if pr.total > 0 { (pr.done as f64 / pr.total as f64 * 1000.0) as usize } else { 0 };
        unsafe {
            SendMessageW(self.inst_bar, PBM_SETPOS, pos.min(1000), 0);
        }
        set_text(self.inst_status, &pr.status);
        set_text(self.inst_file, &pr.file);
        show(self.inst_sub, pr.sub >= 0);
        if pr.sub >= 0 {
            unsafe {
                SendMessageW(self.inst_sub, PBM_SETPOS, pr.sub as usize, 0);
            }
        }
        if let Some(st) = self.install_start {
            let el = st.elapsed().as_secs();
            set_text(self.inst_time1, &format!("{} {}", self.ctx.msg("ElapsedTime"), fmt_dur(el)));
            if pr.done > 0 && pr.total > pr.done && el >= 2 {
                let rem = (el as f64 * (pr.total - pr.done) as f64 / pr.done as f64) as u64;
                set_text(self.inst_time2, &format!("{} {}", self.ctx.msg("RemainingTime"), fmt_dur(rem)));
            }
        }
    }

    fn on_done(&mut self) {
        unsafe {
            KillTimer(self.hwnd, TIMER_ID);
        }
        self.on_timer();
        self.install_time = self.install_start.map(|s| s.elapsed());
        let pr = self.shared.as_ref().map(|s| s.progress.lock().unwrap().clone()).unwrap_or_default();
        if pr.cancelled {
            self.exit_code = 2;
            if self.silent {
                self.close();
                return;
            }
            self.error = Some(self.ctx.msg("ExitSetupTitle"));
        } else if let Some(e) = pr.error {
            self.exit_code = 1;
            if !self.sel.suppress_msgboxes {
                crate::w::msgbox(self.hwnd, &e, &self.ctx.msg("ErrorTitle"), 0x10);
            }
            self.error = Some(e);
        }
        if self.silent {
            if self.error.is_none() && self.m.setup.restart_required && !self.sel.suppress_msgboxes {
                let q = self.ctx.msg("FinishedRestartLabel");
                if crate::w::msgbox(self.hwnd, &q, &self.ctx.msg("SetupAppTitle"), 0x24) == 6 {
                    crate::w::reboot();
                }
            }
            self.close();
            return;
        }
        let next = if self.error.is_some() { Some(Page::Finished) } else { self.next_page(Page::Installing) };
        match next {
            Some(p) => self.show_page(p),
            None => self.finish(),
        }
    }

    fn on_ask(&mut self, a: &mut (Ask, Option<Reply>)) {
        let c = self.ctx.clone();
        a.1 = Some(match &a.0 {
            Ask::Disk { file, disk, prompt, dir } => Reply::Folder(crate::ui::dialogs::disk_prompt(self.hwnd, &c, file, *disk, dir, prompt)),
            Ask::FileError { path, err } => {
                let t = format!("{}\n{}\n\n{}\n\n{}", c.msg("ErrorWriteFile"), path, err, c.msg("ErrorRetryAbort"));
                match crate::w::msgbox(self.hwnd, &t, &c.msg("ErrorTitle"), 0x12 /* ABORTRETRYIGNORE|ICONERROR */) {
                    4 => Reply::Retry,
                    5 => Reply::Ignore,
                    _ => Reply::Abort,
                }
            }
        });
    }

    // ---------- painting ----------

    fn paint(&self, dc: HDC) {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rc);
            let full = matches!(self.page, Page::Welcome | Page::Finished);
            let bevel = self.sc(313);
            if full {
                let img_w = self.sc(164);
                let white = RECT { left: img_w, top: 0, right: rc.right, bottom: bevel };
                FillRect(dc, &white, self.white_brush);
                if let Some(img) = &self.img_large {
                    img.draw(dc, 0, 0, img_w, bevel);
                }
                if !self.icon_big.is_null() && self.p.header.resource("img:wizard").is_none() {
                    let sz = self.sc(64);
                    DrawIconEx(dc, (img_w - sz) / 2, self.sc(40), self.icon_big, sz, sz, 0, std::ptr::null_mut(), DI_NORMAL);
                }
            } else {
                let hh = self.sc(58);
                let top = RECT { left: 0, top: 0, right: rc.right, bottom: hh };
                FillRect(dc, &top, self.white_brush);
                let size = self.sc(55);
                let x = rc.right - size - self.sc(2);
                let y = (hh - size) / 2;
                if let Some(img) = &self.img_small {
                    img.draw(dc, x, y, size, size);
                } else if !self.icon_big.is_null() {
                    let sz = self.sc(48);
                    DrawIconEx(dc, rc.right - sz - self.sc(6), (hh - sz) / 2, self.icon_big, sz, sz, 0, std::ptr::null_mut(), DI_NORMAL);
                }
                let mut line = RECT { left: 0, top: hh, right: rc.right, bottom: hh + 2 };
                DrawEdge(dc, &mut line, EDGE_ETCHED, BF_TOP);
            }
            let mut b = RECT { left: 0, top: bevel, right: rc.right, bottom: bevel + 2 };
            DrawEdge(dc, &mut b, EDGE_ETCHED, BF_TOP);
        }
    }
}

pub fn is_visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h) != 0 }
}

fn fmt_dur(s: u64) -> String {
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn estimate_lines(hwnd: HWND, font: HFONT, text: &str, width: i32) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        let old = SelectObject(dc, font as HGDIOBJ);
        let w = wide(text);
        let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
        DrawTextW(dc, w.as_ptr(), -1, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
        let mut tm: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
        let lh = tm.tmHeight.max(1);
        // convert back to 96-dpi "lines of 15px"
        let lines = (r.bottom + lh - 1) / lh;
        let scale = lh as f32 / 15.0;
        ((lines as f32) * scale).ceil() as i32
    }
}

fn folder_icon() -> HICON {
    stock_icon(3) // SIID_FOLDER
}

fn group_icon() -> HICON {
    stock_icon(4) // SIID_FOLDEROPEN
}

fn stock_icon(id: i32) -> HICON {
    use windows_sys::Win32::UI::Shell::*;
    unsafe {
        let mut sii: SHSTOCKICONINFO = std::mem::zeroed();
        sii.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
        if SHGetStockIconInfo(id, SHGSI_ICON | SHGSI_LARGEICON, &mut sii) >= 0 && !sii.hIcon.is_null() {
            return sii.hIcon;
        }
        // fallback: icon from shell32.dll (folder = index 3/4)
        let shell = wide("shell32.dll");
        let h = ExtractIconW(hinst(), shell.as_ptr(), id as u32);
        if !h.is_null() && h as isize != 1 {
            h
        } else {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        }
    }
}

// ---------- tree view helpers ----------

fn tree_insert(tree: HWND, parent: isize, text: &str, param: isize, state: u32) -> isize {
    let mut t = wide(text);
    unsafe {
        let mut ins: TVINSERTSTRUCTW = std::mem::zeroed();
        ins.hParent = parent as _;
        ins.hInsertAfter = TVI_LAST;
        ins.Anonymous.item.mask = TVIF_TEXT | TVIF_PARAM | TVIF_STATE;
        ins.Anonymous.item.pszText = t.as_mut_ptr();
        ins.Anonymous.item.lParam = param;
        ins.Anonymous.item.state = state << 12;
        ins.Anonymous.item.stateMask = TVIS_STATEIMAGEMASK;
        SendMessageW(tree, TVM_INSERTITEMW, 0, &ins as *const _ as isize)
    }
}

fn tree_expand(tree: HWND, item: isize) {
    unsafe {
        SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, item);
    }
}

fn tree_set_state(tree: HWND, item: isize, img: u32) {
    unsafe {
        let mut it: TVITEMW = std::mem::zeroed();
        it.mask = TVIF_STATE;
        it.hItem = item as _;
        it.state = img << 12;
        it.stateMask = TVIS_STATEIMAGEMASK;
        SendMessageW(tree, TVM_SETITEMW, 0, &it as *const _ as isize);
    }
}

fn tree_get_param(tree: HWND, item: isize) -> isize {
    unsafe {
        let mut it: TVITEMW = std::mem::zeroed();
        it.mask = TVIF_PARAM;
        it.hItem = item as _;
        SendMessageW(tree, TVM_GETITEMW, 0, &mut it as *mut _ as isize);
        it.lParam
    }
}

/// State image list: 0 blank, 1 box, 2 box checked, 3 radio, 4 radio checked.
fn set_state_images(tree: HWND, s: f32) {
    unsafe {
        let sz = (16.0 * s).round() as i32;
        let il = ImageList_Create(sz, sz, ILC_COLOR24, 5, 0);
        let dc = GetDC(tree);
        let mem = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, sz * 5, sz);
        let old = SelectObject(mem, bmp as HGDIOBJ);
        let bg = GetSysColorBrush(COLOR_WINDOW);
        let all = RECT { left: 0, top: 0, right: sz * 5, bottom: sz };
        FillRect(mem, &all, bg);
        let theme = OpenThemeData(tree, wide("BUTTON").as_ptr());
        for i in 1..5 {
            let inset = (sz as f32 * 0.1) as i32;
            let mut r = RECT { left: i * sz + inset, top: inset, right: (i + 1) * sz - inset, bottom: sz - inset };
            let checked = i == 2 || i == 4;
            let radio = i >= 3;
            if theme != 0 {
                let part = if radio { BP_RADIOBUTTON } else { BP_CHECKBOX };
                let st = if radio {
                    if checked { RBS_CHECKEDNORMAL } else { RBS_UNCHECKEDNORMAL }
                } else if checked {
                    CBS_CHECKEDNORMAL
                } else {
                    CBS_UNCHECKEDNORMAL
                };
                DrawThemeBackground(theme, mem, part, st, &r, std::ptr::null());
            } else {
                let mut st = if radio { DFCS_BUTTONRADIO } else { DFCS_BUTTONCHECK };
                if checked {
                    st |= DFCS_CHECKED;
                }
                DrawFrameControl(mem, &mut r, DFC_BUTTON, st);
            }
        }
        if theme != 0 {
            CloseThemeData(theme);
        }
        SelectObject(mem, old);
        DeleteDC(mem);
        ReleaseDC(tree, dc);
        ImageList_Add(il, bmp, std::ptr::null_mut());
        DeleteObject(bmp as HGDIOBJ);
        SendMessageW(tree, TVM_SETIMAGELIST, TVSIL_STATE as usize, il as isize);
    }
}

// ---------- window procedure ----------

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(wz) = wiz() else { return DefWindowProcW(hwnd, msg, wp, lp) };
    if wz.hwnd != hwnd {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    match msg {
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            let code = ((wp >> 16) & 0xffff) as u32;
            match id {
                ID_NEXT => {
                    if is_visible(wz.next) && IsWindowEnabled(wz.next) != 0 {
                        wz.go_next();
                    }
                }
                ID_CANCEL => {
                    if wz.page == Page::Finished {
                        wz.finish();
                    } else if is_visible(wz.cancel) && IsWindowEnabled(wz.cancel) != 0 {
                        wz.on_cancel();
                    }
                }
                ID_BACK => {
                    if is_visible(wz.back) {
                        wz.go_back()
                    }
                }
                ID_LIC_ACCEPT | ID_LIC_DECLINE => enable(wz.next, is_checked(wz.lic_accept)),
                ID_DIR_BROWSE => {
                    let cur = get_text(wz.dir_edit);
                    if let Some(mut pth) = browse_folder(hwnd, &wz.ctx.msg("BrowseDialogLabel"), &cur) {
                        // append the default folder name, like Inno
                        let def = Path::new(&wz.sel.dir).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                        if !def.is_empty() && !pth.to_ascii_lowercase().ends_with(&def.to_ascii_lowercase()) {
                            if !pth.ends_with('\\') {
                                pth.push('\\');
                            }
                            pth.push_str(&def);
                        }
                        set_text(wz.dir_edit, &pth);
                    }
                }
                ID_GROUP_BROWSE => {
                    let base = if wz.ctx.admin {
                        crate::w::known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_CommonPrograms)
                    } else {
                        crate::w::known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Programs)
                    };
                    if let Some(pth) = browse_folder(hwnd, &wz.ctx.msg("BrowseDialogLabel"), &base) {
                        let rel = pth.strip_prefix(&base).map(|s| s.trim_start_matches('\\').to_string()).unwrap_or_else(|| {
                            Path::new(&pth).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
                        });
                        let def = wz.sel.group.clone();
                        let v = if rel.is_empty() { def } else { rel };
                        set_text(wz.group_edit, &v);
                    }
                }
                ID_NOICONS => {
                    let on = is_checked(wz.group_noicons);
                    enable(wz.group_edit, !on);
                }
                ID_TYPES if code == CBN_SELCHANGE => {
                    let i = SendMessageW(wz.comp_types, CB_GETCURSEL, 0, 0);
                    if i >= 0 {
                        wz.apply_type(i as usize);
                        wz.sync_comp_tree();
                        wz.update_comp_space();
                    }
                }
                _ => {}
            }
            return 0;
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            let id = nm.idFrom as i32;
            if id == ID_COMP_TREE || id == ID_TASK_TREE {
                if nm.code == NM_CLICK {
                    let pos = GetMessagePos();
                    let mut ht: TVHITTESTINFO = std::mem::zeroed();
                    ht.pt = POINT { x: (pos & 0xffff) as i16 as i32, y: ((pos >> 16) & 0xffff) as i16 as i32 };
                    ScreenToClient(nm.hwndFrom, &mut ht.pt);
                    let item = SendMessageW(nm.hwndFrom, TVM_HITTEST, 0, &mut ht as *mut _ as isize);
                    if item != 0 && (ht.flags & (TVHT_ONITEMSTATEICON | TVHT_ONITEMLABEL | TVHT_ONITEMICON)) != 0 {
                        PostMessageW(hwnd, WM_APP_TREE, id as usize, item);
                    }
                } else if nm.code == TVN_KEYDOWN {
                    let kd = &*(lp as *const NMTVKEYDOWN);
                    if kd.wVKey == VK_SPACE {
                        let item = SendMessageW(nm.hwndFrom, TVM_GETNEXTITEM, TVGN_CARET as usize, 0);
                        if item != 0 {
                            PostMessageW(hwnd, WM_APP_TREE, id as usize, item);
                        }
                        return 1;
                    }
                }
            }
            return 0;
        }
        WM_APP_TREE => {
            wz.on_tree_click(wp as i32, lp);
            return 0;
        }
        WM_TIMER => {
            wz.on_timer();
            return 0;
        }
        WM_APP_DONE => {
            wz.on_done();
            return 0;
        }
        WM_APP_ASK => {
            let a = &mut *(lp as *mut (Ask, Option<Reply>));
            wz.on_ask(a);
            return 0;
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let ctl = lp as HWND;
            if wz.white.contains(&ctl) {
                let dc = wp as HDC;
                SetBkColor(dc, 0xffffff);
                if ctl == wz.fin_title && wz.error.is_some() {
                    SetTextColor(dc, 0x0000c0);
                }
                return wz.white_brush as LRESULT;
            }
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            wz.paint(dc);
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_CLOSE => {
            if wz.page == Page::Finished {
                wz.finish();
            } else {
                wz.on_cancel();
            }
            return 0;
        }
        WM_DESTROY => {
            wz.done = true;
            PostQuitMessage(wz.exit_code);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Run the wizard; returns the process exit code.
pub fn run(init: Init) -> i32 {
    init_common_controls();
    register_class("SmkWizard", Some(wnd_proc));
    let s = dpi() as f32 / 96.0;
    let m = &init.p.header.manifest;
    let accent = parse_color(&m.interface.accent_color).unwrap_or(0x1F5DA8);
    let img_large = init.p.resource("img:wizard").and_then(|d| Image::from_resource(&d)).or_else(|| default_large_image(accent));
    let img_small = init.p.resource("img:small").and_then(|d| Image::from_resource(&d));
    let ctx = init.ctx.clone();
    let mut wz = Box::new(Wizard {
        hwnd: std::ptr::null_mut(),
        s,
        f: make_fonts_with(&m.interface.font, m.interface.font_size),
        p: init.p,
        m,
        ctx: ctx.clone(),
        sel: init.sel,
        tmp: init.tmp,
        silent: init.silent,
        page: Page::Welcome,
        ctls: vec![],
        white: vec![],
        white_brush: unsafe { CreateSolidBrush(0xffffff) },
        img_large,
        img_small,
        icon_big: app_icon((64.0 * s) as i32),
        back: std::ptr::null_mut(),
        next: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        hdr_title: std::ptr::null_mut(),
        hdr_desc: std::ptr::null_mut(),
        wel_title: std::ptr::null_mut(),
        wel_text: std::ptr::null_mut(),
        fin_title: std::ptr::null_mut(),
        fin_text: std::ptr::null_mut(),
        fin_time: std::ptr::null_mut(),
        fin_checks: vec![],
        fin_yes: std::ptr::null_mut(),
        fin_no: std::ptr::null_mut(),
        lic_accept: std::ptr::null_mut(),
        pwd_edit: std::ptr::null_mut(),
        dir_edit: std::ptr::null_mut(),
        dir_space: std::ptr::null_mut(),
        comp_types: std::ptr::null_mut(),
        comp_tree: std::ptr::null_mut(),
        comp_space: std::ptr::null_mut(),
        group_edit: std::ptr::null_mut(),
        group_noicons: std::ptr::null_mut(),
        task_tree: std::ptr::null_mut(),
        ready_memo: std::ptr::null_mut(),
        inst_status: std::ptr::null_mut(),
        inst_file: std::ptr::null_mut(),
        inst_bar: std::ptr::null_mut(),
        inst_sub: std::ptr::null_mut(),
        inst_time1: std::ptr::null_mut(),
        inst_time2: std::ptr::null_mut(),
        comp_checked: vec![],
        comp_items: vec![],
        task_checked: vec![],
        task_items: vec![],
        shared: None,
        install_start: None,
        install_time: None,
        error: None,
        exit_code: 0,
        done: false,
    });
    let title = if m.setup.window_title.is_empty() { ctx.msgf("SetupWindowTitle", &[&ctx.app_name]) } else { ctx.expand(&m.setup.window_title) };
    unsafe {
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let mut r = RECT { left: 0, top: 0, right: (497.0 * s) as i32, bottom: (360.0 * s) as i32 };
        AdjustWindowRectEx(&mut r, style, 0, WS_EX_CONTROLPARENT);
        let cn = wide("SmkWizard");
        let t = wide(&title);
        WIZ = &mut *wz as *mut Wizard;
        let hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, cn.as_ptr(), t.as_ptr(), style, 0, 0, r.right - r.left, r.bottom - r.top, std::ptr::null_mut(), std::ptr::null_mut(), hinst(), std::ptr::null());
        wz.hwnd = hwnd;
        center(hwnd, std::ptr::null_mut());
        wz.create();
        let first = if wz.silent { Page::Installing } else { ORDER.iter().copied().find(|p| wz.enabled(*p)).unwrap_or(Page::Installing) };
        if wz.silent {
            wz.sel.components = if wz.sel.components.is_empty() { wz.selected_components() } else { wz.sel.components.clone() };
            wz.sel.tasks = wz.default_tasks();
        }
        wz.show_page(first);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        if first == Page::Installing {
            wz.start_install();
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let code = wz.exit_code;
        WIZ = std::ptr::null_mut();
        code
    }
}
