//! Setup Maker — builder of Inno-style Windows installers (Linux + Windows).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod compile;
#[cfg(feature = "gui")]
mod gui;
mod images;
mod inno_exe;
mod iss;
mod pe;
mod project_io;
mod stubs;
mod sysinfo;

use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "setupmaker", version, about = "Конструктор установщиков Windows в стиле Inno Setup")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Открыть графический редактор
    Gui { project: Option<PathBuf> },
    /// Собрать установщик из проекта (.smp) или скрипта Inno (.iss)
    Build {
        project: PathBuf,
        /// Переопределить define: -D InnoDir=/path
        #[arg(short = 'D', value_parser = parse_kv)]
        define: Vec<(String, String)>,
        /// Проверить CRC всех файлов после сборки
        #[arg(long)]
        verify: bool,
    },
    /// Конвертировать .iss или готовый Inno setup.exe в проект .smp
    Import {
        /// Скрипт .iss или скомпилированный установщик Inno Setup (setup.exe)
        script: PathBuf,
        /// Для .iss — файл .smp; для setup.exe — папка проекта
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(short = 'D', value_parser = parse_kv)]
        define: Vec<(String, String)>,
    },
    /// Упаковать папку во внешний архив .smpk
    Pack {
        folder: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = "lzma2")]
        method: String,
        #[arg(long, default_value_t = 9)]
        level: u32,
        #[arg(long, default_value_t = 64)]
        dict: u32,
        #[arg(long, default_value_t = 0)]
        threads: u32,
    },
    /// Создать новый проект по шаблону
    New {
        name: String,
        #[arg(long, default_value = "")]
        exe: String,
        #[arg(long, default_value = "")]
        files: String,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Проверить целостность готового установщика
    Verify { setup: PathBuf },
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).ok_or_else(|| "ожидается NAME=VALUE".into())
}

#[cfg(windows)]
fn attach_console() {
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX);
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args().len() > 1 {
        attach_console();
    }
    let cli = Cli::parse();
    let code = match cli.cmd {
        None => run_gui(None),
        Some(Cmd::Gui { project }) => run_gui(project),
        Some(cmd) => match run_cli(cmd) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Ошибка: {}", e);
                1
            }
        },
    };
    std::process::exit(code);
}

#[cfg(feature = "gui")]
fn run_gui(p: Option<PathBuf>) -> i32 {
    match gui::run(p) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("GUI error: {}", e);
            1
        }
    }
}

#[cfg(not(feature = "gui"))]
fn run_gui(_: Option<PathBuf>) -> i32 {
    eprintln!("Сборка без GUI. Используйте: setupmaker build <project.smp>");
    2
}

fn run_cli(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Build { project, define, verify } => {
            let defs: BTreeMap<String, String> = define.into_iter().collect();
            let (p, dir, warns) = project_io::load_any(&project, &defs)?;
            for w in warns {
                eprintln!("Импорт: {}", w);
            }
            let opts = compile::BuildOptions {
                defines: defs,
                progress: Arc::new(progress_bar()),
                ..Default::default()
            };
            let r = compile::build(&p, &dir, &opts)?;
            eprintln!();
            if verify {
                compile::verify(&r.output, &opts.log)?;
            }
            for e in &r.extra {
                println!("+ {}", e.display());
            }
            Ok(())
        }
        Cmd::Import { script, out, define: _ } if is_exe(&script) => {
            let dir = out.unwrap_or_else(|| default_import_dir(&script));
            let log = |s: &str| {
                if s.starts_with('\r') {
                    eprint!("{}", s);
                } else {
                    eprintln!("{}", s);
                }
            };
            let r = inno_exe::import(&script, &dir, &log)?;
            for w in &r.imported.warnings {
                eprintln!("Предупреждение: {}", w);
            }
            project_io::save(&r.imported.project, &r.project_path)?;
            println!("Проект сохранён: {}", r.project_path.display());
            Ok(())
        }
        Cmd::Import { script, out, define } => {
            let defs: BTreeMap<String, String> = define.into_iter().collect();
            let r = iss::import(&script, &defs)?;
            for w in &r.warnings {
                eprintln!("Предупреждение: {}", w);
            }
            let out = out.unwrap_or_else(|| script.with_extension("smp"));
            project_io::save(&r.project, &out)?;
            println!("Проект сохранён: {}", out.display());
            Ok(())
        }
        Cmd::Pack { folder, out, method, level, dict, threads } => {
            let mut p = smk_core::project::Project::default();
            p.compression.method = method;
            p.compression.level = level;
            p.compression.dict_mb = dict;
            p.compression.threads = threads;
            let opts = compile::BuildOptions { progress: Arc::new(progress_bar()), ..Default::default() };
            let (raw, packed) = compile::pack_folder(&folder, &out, &p, &opts)?;
            eprintln!();
            println!("{}: {} -> {} ({:.1}%)", out.display(), compile::human(raw), compile::human(packed), packed as f64 * 100.0 / raw.max(1) as f64);
            Ok(())
        }
        Cmd::New { name, exe, files, out } => {
            let mut p = smk_core::project::Project::template(&name, &exe, &files);
            p.app.app_id = compile::new_guid();
            project_io::save(&p, &out)?;
            println!("Проект создан: {}", out.display());
            Ok(())
        }
        Cmd::Verify { setup } => {
            let log: compile::Log = Arc::new(|s| println!("{}", s));
            compile::verify(&setup, &log).map(|_| ())
        }
        Cmd::Gui { .. } => unreachable!(),
    }
}

fn is_exe(p: &std::path::Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false)
}

/// `.../GTA/setup.exe` -> `.../GTA/setup_project`
pub fn default_import_dir(setup: &std::path::Path) -> PathBuf {
    let stem = setup.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "setup".into());
    setup.parent().unwrap_or(std::path::Path::new(".")).join(format!("{}_project", stem))
}

fn progress_bar() -> impl Fn(f32) + Send + Sync {
    let last = std::sync::Mutex::new(-1i32);
    move |f: f32| {
        let pct = (f * 100.0) as i32;
        let mut l = last.lock().unwrap();
        if pct != *l {
            *l = pct;
            let filled = (pct / 4) as usize;
            eprint!("\r[{}{}] {:3}%", "#".repeat(filled), ".".repeat(25 - filled.min(25)), pct);
        }
    }
}
