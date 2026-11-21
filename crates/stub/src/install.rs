//! Installation engine (runs on a worker thread; UI talks via `Shared`).

use crate::archives;
use crate::consts::Ctx;
use crate::payload::Payload;
use crate::{reg, shell, w};
use serde::{Deserialize, Serialize};
use smk_core::expr;
use smk_core::format::FileRec;
use smk_core::project::{Conditions, DeleteItem, Project, RunItem};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Everything the user chose in the wizard (or on the command line).
#[derive(Clone, Default, Debug)]
pub struct Selection {
    pub dir: String,
    pub group: String,
    pub no_icons: bool,
    pub setup_type: String,
    pub components: Vec<String>,
    pub tasks: Vec<String>,
    pub silent: bool,
    pub very_silent: bool,
    pub suppress_msgboxes: bool,
}

#[derive(Default, Clone)]
pub struct Progress {
    pub status: String,
    pub file: String,
    pub done: u64,
    pub total: u64,
    /// Current archive progress 0..=1000, -1 = hidden.
    pub sub: i32,
    pub finished: bool,
    pub error: Option<String>,
    pub cancelled: bool,
}

pub enum Ask {
    /// Locate a missing disk slice / archive. Returns folder.
    Disk { file: String, disk: usize, prompt: String, dir: String },
    /// Write error: Retry / Ignore / Abort.
    FileError { path: String, err: String },
}

pub enum Reply {
    Folder(Option<PathBuf>),
    Retry,
    Ignore,
    Abort,
}

pub struct Shared {
    pub progress: Mutex<Progress>,
    pub cancel: AtomicBool,
    pub ask: Box<dyn Fn(Ask) -> Reply + Send + Sync>,
}

impl Shared {
    pub fn new(ask: Box<dyn Fn(Ask) -> Reply + Send + Sync>) -> Arc<Shared> {
        Arc::new(Shared { progress: Mutex::new(Progress { sub: -1, ..Default::default() }), cancel: AtomicBool::new(false), ask })
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn status(&self, s: &str) {
        self.progress.lock().unwrap().status = s.to_string();
    }
    fn file(&self, s: &str) {
        self.progress.lock().unwrap().file = s.to_string();
    }
    fn add(&self, n: u64) {
        self.progress.lock().unwrap().done += n;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LogEntry {
    File { path: String, fresh: bool },
    Dir { path: String },
    RegKey { root: String, sub: String, mode: String },
    RegValue { root: String, sub: String, name: String },
}

/// Saved next to the uninstaller (unins000.dat).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct UninstData {
    pub app_id: String,
    pub app_name: String,
    pub lang: String,
    pub admin: bool,
    pub app: String,
    pub group: String,
    pub reg_root: String,
    pub reg_key: String,
    pub restart: bool,
    pub entries: Vec<LogEntry>,
    pub uninstall_run: Vec<RunItem>,
    pub uninstall_delete: Vec<DeleteItem>,
    pub messages: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

pub fn uninstall_reg_key(app_id: &str) -> String {
    let id = app_id.trim();
    format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}_smk", id)
}

/// Evaluate a condition block with the current selection.
pub fn cond_ok(c: &Conditions, sel: &Selection, ctx: &Ctx) -> bool {
    let comps = &sel.components;
    let tasks = &sel.tasks;
    expr::eval_str(&c.components, &mut |n, _| comps.iter().any(|x| x.eq_ignore_ascii_case(n)))
        && expr::eval_str(&c.tasks, &mut |n, _| tasks.iter().any(|x| x.eq_ignore_ascii_case(n)))
        && check_ok(&c.check, ctx)
}

/// Built-in check functions.
pub fn check_ok(s: &str, ctx: &Ctx) -> bool {
    expr::eval_str(s, &mut |name, args| {
        let a0 = args.first().map(|a| ctx.expand(a)).unwrap_or_default();
        match name.to_ascii_lowercase().as_str() {
            "is64bitinstallmode" => cfg!(target_pointer_width = "64"),
            "iswin64" => w::is_win64(),
            "isadmin" | "isadmininstallmode" | "isadminloggedon" => ctx.admin,
            "fileexists" => {
                if smk_core::wildcard::has_wildcard(&a0) {
                    !archives::find_archives(Path::new(&ctx.src), &a0).is_empty()
                } else {
                    Path::new(&a0).is_file()
                }
            }
            "direxists" => Path::new(&a0).is_dir(),
            "regkeyexists" => reg::full_key_exists(&a0),
            "regvalueexists" => {
                let a1 = args.get(1).map(|a| ctx.expand(a)).unwrap_or_default();
                reg::read_string(&a0, &a1).is_some()
            }
            "windowsversionatleast" | "minversion" => w::version_at_least(&a0),
            "checkerror" => true,
            _ => true,
        }
    })
}

/// Drop components that are unavailable on this machine: their `check` is
/// false, or (with `hide_if_no_archives`) they have no embedded files of
/// their own and none of their archives (masks like `patch*.zip`, or optional
/// plain names) are found next to setup.exe.
/// Children go together with the parent. If something was dropped and a
/// single component is left, it becomes fixed (nothing to choose from).
pub fn prune_components(h: &mut smk_core::format::Header, ctx: &Ctx) {
    use smk_core::expr::mentions;
    let m = &h.manifest;
    let n = m.components.len();
    let own_files = |name: &str| h.streams.iter().any(|s| !s.files.is_empty() && mentions(&s.when.components, name));
    let own_archives = |name: &str| m.archives.iter().filter(|a| mentions(&a.when.components, name)).collect::<Vec<_>>();
    let parent_of = |i: usize| {
        let (p, _) = m.components[i].name.rsplit_once('\\')?;
        m.components.iter().position(|c| c.name.eq_ignore_ascii_case(p))
    };
    let mut hide = vec![false; n];
    for (i, c) in m.components.iter().enumerate() {
        let by_check = !c.check.trim().is_empty() && !check_ok(&c.check, ctx);
        let by_archives = c.hide_if_no_archives && {
            let own = own_archives(&c.name);
            !own_files(&c.name)
                && !own.is_empty()
                // a mask (`patch*.zip`) means "any number of files"; a plain
                // name counts only if optional; disk-prompted ones may be on another disk
                && own.iter().all(|a| a.disk_prompt.trim().is_empty() && (!a.required || smk_core::wildcard::has_wildcard(&a.file)))
                && own.iter().all(|a| archives::find_archives(Path::new(&ctx.src), &ctx.expand(&a.file)).is_empty())
        };
        hide[i] = by_check || by_archives;
    }
    // children follow a hidden parent
    for i in 0..n {
        let mut p = parent_of(i);
        while let Some(k) = p {
            if hide[k] {
                hide[i] = true;
                break;
            }
            p = parent_of(k);
        }
    }
    // an empty group whose children are all hidden goes too (deepest first)
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(m.components[i].name.matches('\\').count()));
    for i in order {
        let c = &m.components[i];
        let kids: Vec<usize> = (0..n).filter(|&j| parent_of(j) == Some(i)).collect();
        if !hide[i] && c.hide_if_no_archives && !kids.is_empty() && kids.iter().all(|&k| hide[k]) && !own_files(&c.name) && own_archives(&c.name).is_empty() {
            hide[i] = true;
        }
    }
    if !hide.iter().any(|&x| x) {
        return;
    }
    let mut it = hide.into_iter();
    let m = &mut h.manifest;
    m.components.retain(|_| !it.next().unwrap());
    if m.components.len() == 1 {
        m.components[0].fixed = true;
        m.components[0].checked = true;
    }
}

pub struct Installer<'a> {
    pub p: &'a Payload,
    pub m: &'a Project,
    pub ctx: Ctx,
    pub sel: Selection,
    pub sh: Arc<Shared>,
    pub log: Vec<LogEntry>,
    pub tmp: PathBuf,
    pub uninst_dat: Option<PathBuf>,
}

type R<T> = Result<T, String>;

impl<'a> Installer<'a> {
    fn ok(&self, c: &Conditions) -> bool {
        cond_ok(c, &self.sel, &self.ctx)
    }

    fn x(&self, s: &str) -> String {
        self.ctx.expand(s)
    }

    fn check_cancel(&self) -> R<()> {
        if self.sh.cancelled() {
            Err("__cancel__".into())
        } else {
            Ok(())
        }
    }

    /// Create directory chain, logging created dirs.
    pub fn mkdirs(&mut self, dir: &Path) -> R<()> {
        let mut missing = Vec::new();
        let mut d = dir.to_path_buf();
        while !d.exists() {
            missing.push(d.clone());
            if !d.pop() {
                break;
            }
        }
        for m in missing.iter().rev() {
            std::fs::create_dir(m).or_else(|e| if m.is_dir() { Ok(()) } else { Err(e) }).map_err(|e| {
                format!("{}\n{}", self.ctx.msgf("ErrorCreatingDir", &[&m.to_string_lossy()]), e)
            })?;
            self.log.push(LogEntry::Dir { path: m.to_string_lossy().to_string() });
        }
        Ok(())
    }

    /// Bytes this installation will write (selected streams + archives).
    pub fn total_work(&self) -> u64 {
        let mut t = 0;
        for s in &self.p.header.streams {
            if self.ok(&s.when) {
                t += s.raw_size;
            }
        }
        for a in &self.m.archives {
            if self.ok(&a.when) {
                t += archive_weight(a, &self.ctx);
            }
        }
        t.max(1)
    }

    pub fn run(&mut self) -> R<()> {
        let total = self.total_work();
        self.sh.progress.lock().unwrap().total = total;
        self.sh.status(&self.ctx.msg("StatusPreparing"));

        // tools -> {tmp}
        for r in &self.p.header.resources {
            if let Some(name) = r.name.strip_prefix("tool:") {
                if let Some(data) = self.p.resource(&r.name) {
                    let _ = std::fs::write(self.tmp.join(name), data);
                }
            }
        }

        // [InstallDelete]
        for d in &self.m.install_delete {
            delete_item(&self.x(&d.path), &d.kind);
        }

        self.sh.status(&self.ctx.msg("StatusCreateDirs"));
        let app = PathBuf::from(&self.ctx.app);
        self.mkdirs(&app)?;
        for d in &self.m.dirs {
            if self.ok(&d.when) {
                let p = PathBuf::from(self.x(&d.name));
                let before = self.log.len();
                self.mkdirs(&p)?;
                if !d.uninstall {
                    self.log.truncate(before);
                }
            }
        }
        for d in &self.p.header.dirs {
            if self.ok(&d.when) {
                let p = PathBuf::from(self.x(&d.path));
                self.mkdirs(&p)?;
            }
        }

        // files
        self.sh.status(&self.ctx.msg("StatusExtractFiles"));
        let streams = self.p.header.streams.clone();
        for s in &streams {
            if !self.ok(&s.when) {
                continue;
            }
            self.check_cancel()?;
            self.extract_stream(s)?;
        }

        // external archives
        let archives = self.m.archives.clone();
        if archives.iter().any(|a| self.ok(&a.when)) {
            self.sh.status(&self.ctx.msg("StatusExtractArchives"));
        }
        for a in &archives {
            if !self.ok(&a.when) {
                continue;
            }
            self.check_cancel()?;
            self.extract_archive(a)?;
        }

        // shortcuts
        self.sh.status(&self.ctx.msg("StatusCreateIcons"));
        for s in &self.m.shortcuts {
            if !self.ok(&s.when) {
                continue;
            }
            if self.sel.no_icons && s.name.to_ascii_lowercase().starts_with("{group}") {
                continue;
            }
            let name = self.x(&s.name);
            let target = self.x(&s.target);
            let path = PathBuf::from(&name);
            if let Some(parent) = path.parent() {
                self.mkdirs(parent)?;
            }
            let (args, wd, icon, comment) = (self.x(&s.params), self.x(&s.workdir), self.x(&s.icon), self.x(&s.comment));
            let l = shell::Lnk { path: &name, target: &target, args: &args, workdir: &wd, icon: &icon, icon_index: s.icon_index, comment: &comment };
            if shell::create(&l).is_ok() {
                self.log.push(LogEntry::File { path: shell::written_path(&l), fresh: true });
            }
        }

        // registry
        self.sh.status(&self.ctx.msg("StatusCreateRegistryEntries"));
        for r in &self.m.registry {
            if !self.ok(&r.when) {
                continue;
            }
            let Some(root) = reg::root_of(&r.root, self.ctx.admin) else { continue };
            let rootn = reg::root_name(&r.root, self.ctx.admin);
            let sub = self.x(&r.subkey);
            if r.delete_key_first {
                reg::delete_tree(root, &sub);
            }
            if r.delete_value {
                if let Some(k) = reg::open(root, &sub, true) {
                    reg::delete_value(&k, &self.x(&r.value_name));
                }
                continue;
            }
            let Some((k, created)) = reg::create(root, &sub) else { continue };
            let vname = self.x(&r.value_name);
            if r.value_type != "none" && !(r.only_if_missing && reg::value_exists(&k, &vname)) {
                reg::set_typed(&k, &vname, &r.value_type, &self.x(&r.value_data));
            }
            match r.uninstall.as_str() {
                "deletekey" => self.log.push(LogEntry::RegKey { root: rootn, sub, mode: "deletekey".into() }),
                "deletekeyifempty" => self.log.push(LogEntry::RegKey { root: rootn, sub, mode: "ifempty".into() }),
                "deletevalue" if r.value_type != "none" => {
                    if created {
                        self.log.push(LogEntry::RegKey { root: rootn.clone(), sub: sub.clone(), mode: "ifempty".into() });
                    }
                    self.log.push(LogEntry::RegValue { root: rootn, sub, name: vname })
                }
                _ => {}
            }
        }

        // uninstaller
        if self.m.uninstall.enabled {
            self.sh.status(&self.ctx.msg("StatusSavingUninstall"));
            self.write_uninstaller()?;
        }

        // [Run] (non-postinstall)
        self.sh.status(&self.ctx.msg("StatusRunProgram"));
        for r in &self.m.run {
            if r.postinstall || !self.ok(&r.when) {
                continue;
            }
            if r.skip_if_silent && self.sel.silent {
                continue;
            }
            if !r.status_msg.is_empty() {
                self.sh.status(&self.x(&r.status_msg));
            }
            run_item(r, &self.ctx);
        }

        // deleteafterinstall
        for f in &self.p.header.files {
            if f.delete_after {
                let p = self.x(&f.dest);
                let _ = std::fs::remove_file(&p);
                self.log.retain(|e| !matches!(e, LogEntry::File { path, .. } if path.eq_ignore_ascii_case(&p)));
            }
        }
        // the uninstaller has already saved the log; rewrite it with final state
        if self.m.uninstall.enabled {
            self.save_log()?;
        }
        let mut pr = self.sh.progress.lock().unwrap();
        pr.done = pr.total;
        Ok(())
    }

    fn locate(&self, file: &str, disk: usize, prompt: &str) -> Option<PathBuf> {
        if self.sel.very_silent && self.sel.suppress_msgboxes {
            return None;
        }
        match (self.sh.ask)(Ask::Disk { file: file.to_string(), disk, prompt: prompt.to_string(), dir: self.ctx.src.clone() }) {
            Reply::Folder(f) => f,
            _ => None,
        }
    }

    fn extract_stream(&mut self, s: &smk_core::format::StreamInfo) -> R<()> {
        let files: Vec<FileRec> = s.files.iter().map(|&i| self.p.header.files[i as usize].clone()).collect();
        let idx: Vec<u32> = s.files.clone();
        let sh = self.sh.clone();
        let this_ctx = self.ctx.clone();
        let p = self.p;
        let mut loc = |name: &str, disk: usize| -> Option<PathBuf> { self_locate(&sh, &this_ctx, name, disk) };
        let mut reader = p.stream_reader(s, &mut loc).map_err(|e| e.to_string())?;
        let mut written: HashMap<u32, PathBuf> = HashMap::new();
        for (k, f) in files.iter().enumerate() {
            self.check_cancel()?;
            let dest = PathBuf::from(self.x(&f.dest));
            self.sh.file(&dest.to_string_lossy());
            if let Some(parent) = dest.parent() {
                self.mkdirs(parent)?;
            }
            if let Some(orig) = f.dup_of {
                if let Some(src) = written.get(&orig) {
                    let fresh = !dest.exists();
                    w::clear_readonly(&dest);
                    if std::fs::copy(src, &dest).is_ok() {
                        if f.attrs != 0 {
                            w::set_attributes(&dest, f.attrs);
                        }
                        if f.uninstall {
                            self.log.push(LogEntry::File { path: dest.to_string_lossy().to_string(), fresh });
                        }
                    }
                }
                self.sh.add(f.size);
                continue;
            }
            let skip = dest.exists()
                && match f.overwrite {
                    2 => true,
                    1 => newer_or_same(&dest, f.mtime),
                    _ => false,
                };
            let fresh = !dest.exists();
            if skip {
                copy_n(&mut reader, &mut io::sink(), f.size, &self.sh).map_err(|e| e.to_string())?;
                written.insert(idx[k], dest.clone());
                continue;
            }
            loop {
                match write_file(&mut reader, &dest, f, &self.sh) {
                    Ok(()) => break,
                    Err(WriteErr::Read(e)) => {
                        if self.sh.cancelled() {
                            return Err("__cancel__".into());
                        }
                        return Err(format!("{}\n{}", self.ctx.msg("CorruptedSetup"), e));
                    }
                    Err(WriteErr::Write(e, consumed)) => {
                        if self.sel.suppress_msgboxes {
                            return Err(format!("{}\n{}\n{}", self.ctx.msg("ErrorWriteFile"), dest.display(), e));
                        }
                        match (self.sh.ask)(Ask::FileError { path: dest.to_string_lossy().to_string(), err: e.to_string() }) {
                            Reply::Retry if consumed == 0 => continue,
                            Reply::Retry | Reply::Ignore => {
                                // can't rewind a solid stream: skip the rest of this file
                                copy_n(&mut reader, &mut io::sink(), f.size - consumed, &self.sh).map_err(|e| e.to_string())?;
                                break;
                            }
                            _ => return Err("__cancel__".into()),
                        }
                    }
                }
            }
            if f.uninstall {
                self.log.push(LogEntry::File { path: dest.to_string_lossy().to_string(), fresh });
            }
            written.insert(idx[k], dest);
        }
        Ok(())
    }

    fn extract_archive(&mut self, a: &smk_core::project::ArchiveItem) -> R<()> {
        let pattern = self.x(&a.file);
        let mut search_dir = PathBuf::from(&self.ctx.src);
        let weight = archive_weight(a, &self.ctx);
        let start_done = self.sh.progress.lock().unwrap().done;
        loop {
            let found = archives::find_archives(&search_dir, &pattern);
            if !found.is_empty() {
                let total_size: u64 = found.iter().map(|f| f.metadata().map(|m| m.len()).unwrap_or(1)).sum::<u64>().max(1);
                let mut base_read = 0u64;
                for f in &found {
                    self.check_cancel()?;
                    let dest = PathBuf::from(self.x(&a.dest));
                    self.mkdirs(&dest)?;
                    let fmt = if a.format.is_empty() {
                        smk_core::project::archive_format_by_name(&f.to_string_lossy()).to_string()
                    } else {
                        a.format.to_ascii_lowercase()
                    };
                    let flen = f.metadata().map(|m| m.len()).unwrap_or(1).max(1);
                    let sh = self.sh.clone();
                    let tmp = self.tmp.clone();
                    let mut prog = |read: u64, name: &str| {
                        let frac = (base_read + read.min(flen)) as f64 / total_size as f64;
                        let mut pr = sh.progress.lock().unwrap();
                        pr.done = start_done + (weight as f64 * frac) as u64;
                        pr.sub = ((read.min(flen) as f64 / flen as f64) * 1000.0) as i32;
                        if !name.is_empty() {
                            pr.file = name.to_string();
                        }
                        !sh.cancelled()
                    };
                    let mut hooks = archives::Hooks { created: Vec::new(), progress: &mut prog };
                    let res = archives::extract(&fmt, f, &dest, &self.x(&a.password), &tmp, &mut hooks);
                    for c in hooks.created {
                        let path = c.path.to_string_lossy().to_string();
                        self.log.push(if c.is_dir { LogEntry::Dir { path } } else { LogEntry::File { path, fresh: c.fresh } });
                    }
                    self.sh.progress.lock().unwrap().sub = -1;
                    if let Err(e) = res {
                        if self.sh.cancelled() {
                            return Err("__cancel__".into());
                        }
                        return Err(format!("{}\n{}\n\n{}", self.ctx.msg("ErrorArchive"), f.display(), e));
                    }
                    if a.delete_after {
                        let _ = std::fs::remove_file(f);
                    }
                    base_read += flen;
                }
                self.sh.progress.lock().unwrap().done = start_done + weight;
                return Ok(());
            }
            if !a.required {
                self.sh.add(weight);
                return Ok(());
            }
            // missing: disk prompt
            let prompt = if a.disk_prompt.is_empty() { String::new() } else { self.x(&a.disk_prompt) };
            match self.locate(&pattern, 0, &prompt) {
                Some(dir) => search_dir = dir,
                None => {
                    if self.sh.cancelled() {
                        return Err("__cancel__".into());
                    }
                    return Err(format!("{}\n{}", self.ctx.msg("ArchiveNotFound"), search_dir.join(&pattern).display()));
                }
            }
        }
    }

    fn uninst_paths(&self) -> (PathBuf, PathBuf) {
        let dir = PathBuf::from(self.x(&self.m.uninstall.files_dir));
        // reuse an existing slot with the same app id, else first free one
        for i in 0..1000 {
            let exe = dir.join(format!("unins{:03}.exe", i));
            let dat = dir.join(format!("unins{:03}.dat", i));
            if !dat.exists() && !exe.exists() {
                return (exe, dat);
            }
            if let Ok(d) = std::fs::read(&dat).map_err(|e| e.to_string()).and_then(|b| read_dat(&b)) {
                if d.app_id == self.m.app.app_id {
                    return (exe, dat);
                }
            }
        }
        (dir.join("unins000.exe"), dir.join("unins000.dat"))
    }

    fn write_uninstaller(&mut self) -> R<()> {
        let (exe, dat) = self.uninst_paths();
        if let Some(p) = exe.parent() {
            self.mkdirs(p)?;
        }
        // merge an existing log (upgrade over a previous installation)
        if let Ok(old) = std::fs::read(&dat).map_err(|e| e.to_string()).and_then(|b| read_dat(&b)) {
            let mut merged = old.entries;
            for e in self.log.drain(..) {
                if !merged.contains(&e) {
                    merged.push(e);
                }
            }
            self.log = merged;
        }
        let stub = self.p.stub_bytes().map_err(|e| e.to_string())?;
        let fresh = !exe.exists();
        w::clear_readonly(&exe);
        std::fs::write(&exe, stub).map_err(|e| format!("{}\n{}\n{}", self.ctx.msg("ErrorWriteFile"), exe.display(), e))?;
        self.ctx.uninstallexe = exe.to_string_lossy().to_string();
        self.log.push(LogEntry::File { path: exe.to_string_lossy().to_string(), fresh });
        self.log.push(LogEntry::File { path: dat.to_string_lossy().to_string(), fresh: !dat.exists() });

        // icon for Add/Remove programs
        let mut icon = self.x(&self.m.uninstall.display_icon);
        if let Some(ico) = self.p.resource("uninstall.ico") {
            let ip = exe.with_extension("ico");
            if std::fs::write(&ip, ico).is_ok() {
                self.log.push(LogEntry::File { path: ip.to_string_lossy().to_string(), fresh: true });
                icon = ip.to_string_lossy().to_string();
            }
        }
        if icon.is_empty() {
            icon = if self.m.app.exe.is_empty() {
                exe.to_string_lossy().to_string()
            } else {
                format!("{}\\{}", self.ctx.app, self.m.app.exe)
            };
        }

        // Add/Remove programs entry
        let root = if self.ctx.admin { "HKLM" } else { "HKCU" };
        let key = uninstall_reg_key(&self.m.app.app_id);
        if let Some((k, _)) = reg::create(reg::root_of(root, self.ctx.admin).unwrap(), &key) {
            let m = &self.m.app;
            let name = if self.m.uninstall.display_name.is_empty() { m.display_name() } else { self.x(&self.m.uninstall.display_name) };
            let q = format!("\"{}\"", exe.display());
            reg::set_string(&k, "DisplayName", &name);
            reg::set_string(&k, "DisplayVersion", &m.version);
            reg::set_string(&k, "Publisher", &m.publisher);
            reg::set_string(&k, "UninstallString", &q);
            reg::set_string(&k, "QuietUninstallString", &format!("{} /SILENT", q));
            reg::set_string(&k, "DisplayIcon", &icon);
            reg::set_string(&k, "InstallLocation", &format!("{}\\", self.ctx.app));
            reg::set_string(&k, "Inno Setup: App Path", &self.ctx.app);
            reg::set_string(&k, "SetupMaker: Group", &self.sel.group);
            reg::set_string(&k, "SetupMaker: Language", &self.ctx.lang);
            if !m.url.is_empty() {
                reg::set_string(&k, "URLInfoAbout", &m.url);
            }
            if !m.support_url.is_empty() {
                reg::set_string(&k, "HelpLink", &m.support_url);
            }
            if !m.updates_url.is_empty() {
                reg::set_string(&k, "URLUpdateInfo", &m.updates_url);
            }
            if !m.contact.is_empty() {
                reg::set_string(&k, "Contact", &m.contact);
            }
            if !m.comments.is_empty() {
                reg::set_string(&k, "Comments", &m.comments);
            }
            let size_kb = (self.sh.progress.lock().unwrap().total / 1024) as u32;
            reg::set_dword(&k, "EstimatedSize", size_kb);
            reg::set_dword(&k, "NoModify", 1);
            reg::set_dword(&k, "NoRepair", 1);
        }
        self.uninst_dat = Some(dat);
        self.save_log()
    }

    fn save_log(&self) -> R<()> {
        let Some(dat) = &self.uninst_dat else { return Ok(()) };
        let d = UninstData {
            app_id: self.m.app.app_id.clone(),
            app_name: self.m.app.display_name(),
            lang: self.ctx.lang.clone(),
            admin: self.ctx.admin,
            app: self.ctx.app.clone(),
            group: self.sel.group.clone(),
            reg_root: if self.ctx.admin { "HKLM".into() } else { "HKCU".into() },
            reg_key: uninstall_reg_key(&self.m.app.app_id),
            restart: self.m.uninstall.restart_computer,
            entries: self.log.clone(),
            uninstall_run: self.m.uninstall_run.clone(),
            uninstall_delete: self.m.uninstall_delete.clone(),
            messages: self.m.messages.clone(),
        };
        let json = serde_json::to_vec(&d).map_err(|e| e.to_string())?;
        let mut out = b"SMKUNDAT".to_vec();
        out.extend_from_slice(&json);
        std::fs::write(dat, out).map_err(|e| e.to_string())
    }

    /// Undo everything this session created.
    pub fn rollback(&mut self) {
        self.sh.status(&self.ctx.msg("StatusRollback"));
        for e in self.log.iter().rev() {
            match e {
                LogEntry::File { path, fresh: true } => {
                    w::clear_readonly(Path::new(path));
                    let _ = std::fs::remove_file(path);
                }
                LogEntry::Dir { path } => {
                    let _ = std::fs::remove_dir(path);
                }
                LogEntry::RegValue { root, sub, name } => {
                    if let Some(k) = reg::root_of(root, self.ctx.admin).and_then(|r| reg::open(r, sub, true)) {
                        reg::delete_value(&k, name);
                    }
                }
                LogEntry::RegKey { root, sub, mode } => {
                    if let Some(r) = reg::root_of(root, self.ctx.admin) {
                        if mode == "deletekey" {
                            reg::delete_tree(r, sub);
                        } else {
                            reg::delete_if_empty(r, sub);
                        }
                    }
                }
                _ => {}
            }
        }
        if self.m.uninstall.enabled && self.uninst_dat.is_some() {
            // only remove the ARP entry if no previous install existed
            let key = uninstall_reg_key(&self.m.app.app_id);
            if !self.log.iter().any(|e| matches!(e, LogEntry::File { fresh: false, .. })) {
                if let Some(r) = reg::root_of(if self.ctx.admin { "HKLM" } else { "HKCU" }, self.ctx.admin) {
                    reg::delete_tree(r, &key);
                }
            }
        }
    }
}

impl<'a> Installer<'a> {
    pub fn new(p: &'a Payload, m: &'a Project, ctx: Ctx, sel: Selection, sh: Arc<Shared>, tmp: PathBuf) -> Self {
        Installer { p, m, ctx, sel, sh, log: Vec::new(), tmp, uninst_dat: None }
    }
}

fn self_locate(sh: &Shared, ctx: &Ctx, name: &str, disk: usize) -> Option<PathBuf> {
    match (sh.ask)(Ask::Disk { file: name.to_string(), disk, prompt: String::new(), dir: ctx.src.clone() }) {
        Reply::Folder(f) => f,
        _ => None,
    }
}

pub fn read_dat(b: &[u8]) -> R<UninstData> {
    if b.len() < 8 || &b[..8] != b"SMKUNDAT" {
        return Err("bad uninstall data".into());
    }
    serde_json::from_slice(&b[8..]).map_err(|e| e.to_string())
}

pub fn archive_weight(a: &smk_core::project::ArchiveItem, ctx: &Ctx) -> u64 {
    if a.size_hint > 0 {
        return a.size_hint;
    }
    let pattern = ctx.expand(&a.file);
    let found = archives::find_archives(Path::new(&ctx.src), &pattern);
    let s: u64 = found.iter().map(|f| f.metadata().map(|m| m.len()).unwrap_or(0)).sum();
    // unpacked data is usually ~2-3x bigger
    (s * 2).max(1 << 20)
}

fn newer_or_same(dest: &Path, mtime: u64) -> bool {
    use std::os::windows::fs::MetadataExt;
    match std::fs::metadata(dest) {
        Ok(m) => m.last_write_time() >= mtime,
        Err(_) => false,
    }
}

enum WriteErr {
    Read(io::Error),
    Write(io::Error, u64),
}

fn write_file(r: &mut dyn Read, dest: &Path, f: &FileRec, sh: &Shared) -> Result<(), WriteErr> {
    if dest.exists() {
        w::clear_readonly(dest);
    }
    let file = std::fs::File::create(dest).map_err(|e| WriteErr::Write(e, 0))?;
    let mut out = io::BufWriter::with_capacity(1 << 20, file);
    let mut left = f.size;
    let mut buf = vec![0u8; 1 << 18];
    let mut crc = crc32fast::Hasher::new();
    let mut consumed = 0u64;
    while left > 0 {
        if sh.cancelled() {
            return Err(WriteErr::Read(io::Error::new(io::ErrorKind::Interrupted, "cancelled")));
        }
        let n = (left as usize).min(buf.len());
        r.read_exact(&mut buf[..n]).map_err(WriteErr::Read)?;
        consumed += n as u64;
        crc.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| WriteErr::Write(e, consumed))?;
        left -= n as u64;
        sh.add(n as u64);
    }
    let file = out.into_inner().map_err(|e| WriteErr::Write(e.into_error(), consumed))?;
    if f.crc != 0 && crc.finalize() != f.crc {
        return Err(WriteErr::Read(io::Error::new(io::ErrorKind::InvalidData, format!("CRC error: {}", dest.display()))));
    }
    w::set_mtime(&file, f.mtime);
    drop(file);
    if f.attrs != 0 {
        w::set_attributes(dest, f.attrs);
    }
    Ok(())
}

fn copy_n(r: &mut dyn Read, w: &mut dyn Write, mut n: u64, sh: &Shared) -> io::Result<()> {
    let mut buf = vec![0u8; 1 << 18];
    while n > 0 {
        let k = (n as usize).min(buf.len());
        r.read_exact(&mut buf[..k])?;
        w.write_all(&buf[..k])?;
        n -= k as u64;
        sh.add(k as u64);
    }
    Ok(())
}

pub fn delete_item(path: &str, kind: &str) {
    let has_wild = smk_core::wildcard::has_wildcard(path);
    let targets: Vec<PathBuf> = if has_wild {
        let p = Path::new(path);
        let (dir, pat) = (p.parent().unwrap_or(Path::new(".")), p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|e| smk_core::wildcard::matches(&pat, &e.file_name().unwrap_or_default().to_string_lossy())).collect())
            .unwrap_or_default()
    } else {
        vec![PathBuf::from(path)]
    };
    for t in targets {
        match kind {
            "dirifempty" => {
                let _ = std::fs::remove_dir(&t);
            }
            "files" => {
                if t.is_file() {
                    w::clear_readonly(&t);
                    let _ = std::fs::remove_file(&t);
                }
            }
            _ => {
                if t.is_dir() {
                    remove_tree(&t);
                } else {
                    w::clear_readonly(&t);
                    let _ = std::fs::remove_file(&t);
                }
            }
        }
    }
}

pub fn remove_tree(p: &Path) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                remove_tree(&path);
            } else {
                w::clear_readonly(&path);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let _ = std::fs::remove_dir(p);
}

/// Execute a [Run] entry. Returns false if it could not be started.
pub fn run_item(r: &RunItem, ctx: &Ctx) -> bool {
    use std::os::windows::process::CommandExt;
    let file = ctx.expand(&r.filename);
    let params = ctx.expand(&r.params);
    let wd = ctx.expand(&r.workdir);
    if r.skip_if_missing && !Path::new(&file).exists() {
        return true;
    }
    if r.shell_exec {
        return w::shell_open(&file, &params, &wd, if r.hidden { 0 } else { 1 });
    }
    let mut cmd = std::process::Command::new(&file);
    cmd.raw_arg(&params);
    if !wd.is_empty() {
        cmd.current_dir(&wd);
    } else if let Some(p) = Path::new(&file).parent() {
        if p.is_dir() {
            cmd.current_dir(p);
        }
    }
    if r.hidden {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.spawn() {
        Ok(mut c) => {
            if !r.nowait {
                let _ = c.wait();
            }
            true
        }
        // not an exe (document, URL...): fall back to ShellExecute
        Err(_) => w::shell_open(&file, &params, &wd, if r.hidden { 0 } else { 1 }),
    }
}
