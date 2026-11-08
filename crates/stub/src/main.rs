//! Setup Maker installer stub. The builder appends a payload to this exe.
//! Without a payload it acts as the uninstaller.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod archives;
#[cfg(windows)]
mod consts;
#[cfg(windows)]
mod freearc;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod payload;
#[cfg(windows)]
mod reg;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod uninstall;
#[cfg(windows)]
mod w;

#[cfg(not(windows))]
fn main() {
    eprintln!("smk-stub is a Windows program; build it for a *-pc-windows-gnu target.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    let code = app::main();
    std::process::exit(code);
}

#[cfg(windows)]
mod app {
    use crate::consts::Ctx;
    use crate::install::{Ask, Installer, Reply, Selection, Shared};
    use crate::payload::Payload;
    use crate::{install, reg, shell, ui, uninstall, w};
    use smk_core::messages;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct Args {
        silent: bool,
        very_silent: bool,
        suppress: bool,
        elevated: bool,
        dir: Option<String>,
        group: Option<String>,
        no_icons: bool,
        lang: Option<String>,
        tasks: Option<Vec<String>>,
        components: Option<Vec<String>>,
        setup_type: Option<String>,
        params: BTreeMap<String, String>,
        raw: Vec<String>,
    }

    fn parse(argv: &[String]) -> Args {
        let mut a = Args {
            silent: false,
            very_silent: false,
            suppress: false,
            elevated: false,
            dir: None,
            group: None,
            no_icons: false,
            lang: None,
            tasks: None,
            components: None,
            setup_type: None,
            params: BTreeMap::new(),
            raw: vec![],
        };
        let list = |v: &str| v.split(',').map(|s| s.trim().trim_start_matches('!').to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>();
        for s in argv {
            let (k, v) = match s.split_once('=') {
                Some((k, v)) => (k.to_ascii_uppercase(), v.trim_matches('"').to_string()),
                None => (s.to_ascii_uppercase(), String::new()),
            };
            match k.as_str() {
                "/SILENT" => a.silent = true,
                "/VERYSILENT" => {
                    a.silent = true;
                    a.very_silent = true
                }
                "/SUPPRESSMSGBOXES" => a.suppress = true,
                "/__ELEVATED" => {
                    a.elevated = true;
                    continue;
                }
                "/DIR" => a.dir = Some(v.clone()),
                "/GROUP" => a.group = Some(v.clone()),
                "/NOICONS" => a.no_icons = true,
                "/LANG" => a.lang = Some(messages::normalize_lang(&v).to_string()),
                "/TASKS" => a.tasks = Some(list(&v)),
                "/COMPONENTS" => a.components = Some(list(&v)),
                "/TYPE" => a.setup_type = Some(v.clone()),
                _ => {
                    if let Some(name) = k.strip_prefix('/') {
                        if !v.is_empty() {
                            a.params.insert(name.to_ascii_lowercase(), v.clone());
                        }
                    }
                }
            }
            a.raw.push(s.clone());
        }
        a
    }

    fn quote(v: &[String]) -> String {
        v.iter().map(|s| if s.contains(' ') && !s.contains('"') { format!("\"{}\"", s) } else { s.clone() }).collect::<Vec<_>>().join(" ")
    }

    pub fn main() -> i32 {
        let argv: Vec<String> = std::env::args().skip(1).collect();
        let exe = w::module_path();
        shell::com_init();
        let payload = match Payload::open(&exe) {
            Ok(Some(p)) => p,
            Ok(None) => return uninstall::main(&argv),
            Err(e) => {
                let lang = messages::lang_from_langid(w::user_ui_lang());
                let ctx = Ctx { lang: lang.into(), ..Default::default() };
                w::msgbox(std::ptr::null_mut(), &format!("{}\n\n{}", ctx.msg("CorruptedSetup"), e), &ctx.msg("SetupAppTitle"), 0x10);
                return 1;
            }
        };
        let p: &'static Payload = Box::leak(Box::new(payload));
        let m = &p.header.manifest;
        let a = parse(&argv);

        // language
        let langs = m.language_ids();
        let sys = messages::lang_from_langid(w::user_ui_lang()).to_string();
        let mut lang = a.lang.clone().filter(|l| langs.contains(l)).unwrap_or_else(|| if langs.contains(&sys) { sys.clone() } else { langs[0].clone() });
        let mut ctx = Ctx {
            lang: lang.clone(),
            admin: w::is_admin(),
            app_name: m.app.name.clone(),
            app_ver_name: m.app.display_name(),
            messages: m.messages.clone(),
            params: a.params.clone(),
            src: p.src_dir.to_string_lossy().to_string(),
            srcexe: exe.to_string_lossy().to_string(),
            ..Default::default()
        };
        let title = ctx.msg("SetupAppTitle");
        let quiet_err = |ctx: &Ctx, text: &str| {
            if !a.suppress {
                w::msgbox(std::ptr::null_mut(), text, &ctx.msg("SetupAppTitle"), 0x10);
            }
        };

        // requirements
        if !w::version_at_least(&m.requirements.min_windows) {
            quiet_err(&ctx, &ctx.msg("WindowsVersionNotSupported"));
            return 1;
        }
        if m.requirements.privileges != "lowest" && !ctx.admin {
            if a.elevated {
                quiet_err(&ctx, &ctx.msg("AdminPrivilegesRequired"));
                return 1;
            }
            let mut args = a.raw.clone();
            if a.lang.is_none() && !a.silent {
                // the elevated instance will show the language dialog itself
            }
            args.push("/__ELEVATED".into());
            return match w::run_elevated(&exe, &quote(&args)) {
                Some(code) => code as i32,
                None => {
                    quiet_err(&ctx, &ctx.msg("AdminPrivilegesRequired"));
                    1
                }
            };
        }
        if m.requirements.min_ram_mb > 0 && !a.silent {
            let have = w::total_ram_mb();
            if have < m.requirements.min_ram_mb as u64 {
                let q = ctx.msgf("NotEnoughMemory", &[&m.requirements.min_ram_mb.to_string(), &have.to_string()]);
                if w::msgbox(std::ptr::null_mut(), &q, &title, 0x34) != 6 {
                    return 2;
                }
            }
        }

        let show_dialog = match m.setup.language_dialog.as_str() {
            "always" => true,
            "never" => false,
            _ => langs.len() > 1,
        };
        if show_dialog && !a.silent && a.lang.is_none() {
            match ui::dialogs::select_language(&langs, &lang) {
                Some(l) => lang = l,
                None => return 2,
            }
        }
        ctx.lang = lang.clone();

        // previous installation
        let key = install::uninstall_reg_key(&m.app.app_id);
        let root = if ctx.admin { "HKLM" } else { "HKCU" };
        let prev = reg::root_of(root, ctx.admin).and_then(|r| reg::open(r, &key, false));
        let prev_dir = prev.as_ref().and_then(|k| reg::get_string(k, "Inno Setup: App Path"));
        let prev_group = prev.as_ref().and_then(|k| reg::get_string(k, "SetupMaker: Group"));

        let mut dir = ctx.expand(&m.setup.default_dir);
        if m.setup.use_previous_dir {
            if let Some(d) = prev_dir.filter(|d| !d.is_empty()) {
                dir = d;
            }
        }
        if let Some(d) = &a.dir {
            dir = ctx.expand(d);
        }
        let mut group = ctx.expand(&m.setup.default_group);
        if let Some(g) = prev_group.filter(|g| !g.is_empty()) {
            group = g;
        }
        if let Some(g) = &a.group {
            group = g.clone();
        }

        // temp folder for tools
        let tmp = std::env::temp_dir().join(format!("smk-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        ctx.tmp = tmp.to_string_lossy().to_string();

        let sel = Selection {
            dir,
            group,
            no_icons: a.no_icons,
            setup_type: a.setup_type.clone().unwrap_or_default(),
            components: a.components.clone().unwrap_or_default(),
            tasks: a.tasks.clone().unwrap_or_default(),
            silent: a.silent,
            very_silent: a.very_silent,
            suppress_msgboxes: a.suppress,
        };

        let code = if a.very_silent {
            run_very_silent(p, ctx, sel, tmp.clone())
        } else {
            ui::wizard::run(ui::wizard::Init { p, ctx, sel, tmp: tmp.clone(), silent: a.silent })
        };
        install::remove_tree(&tmp);
        code
    }

    /// No UI at all (/VERYSILENT).
    fn run_very_silent(p: &'static Payload, mut ctx: Ctx, mut sel: Selection, tmp: PathBuf) -> i32 {
        let m = &p.header.manifest;
        if sel.components.is_empty() {
            let ty = m.types.iter().find(|t| t.name.eq_ignore_ascii_case(&sel.setup_type)).or_else(|| m.types.first());
            sel.components = m
                .components
                .iter()
                .filter(|c| c.fixed || ty.map(|t| c.types.iter().any(|x| x.eq_ignore_ascii_case(&t.name))).unwrap_or(c.checked))
                .map(|c| c.name.clone())
                .collect();
        }
        if sel.tasks.is_empty() {
            sel.tasks = m.tasks.iter().filter(|t| !t.unchecked).map(|t| t.name.clone()).collect();
        }
        ctx.app = sel.dir.clone();
        ctx.group = sel.group.clone();
        let suppress = sel.suppress_msgboxes;
        let lang_ctx = ctx.clone();
        let ask = Box::new(move |a: Ask| -> Reply {
            match a {
                Ask::Disk { file, disk, prompt, dir } if !suppress => Reply::Folder(ui::dialogs::disk_prompt(std::ptr::null_mut(), &lang_ctx, &file, disk, &dir, &prompt)),
                _ => Reply::Abort,
            }
        });
        let sh = Shared::new(ask);
        let mut ins = Installer::new(p, m, ctx.clone(), sel, sh, tmp);
        match ins.run() {
            Ok(()) => 0,
            Err(e) => {
                if m.setup.rollback_on_error {
                    ins.rollback();
                }
                if !suppress && e != "__cancel__" {
                    w::msgbox(std::ptr::null_mut(), &e, &ctx.msg("ErrorTitle"), 0x10);
                }
                1
            }
        }
    }
}
