//! Project editor GUI in the spirit of Smart Install Maker: sections on the
//! left, simple forms on the right, plain checkboxes instead of script syntax.
//! Built with egui, runs on Linux and Windows.

use crate::{compile, project_io};
use eframe::egui::{self, vec2, Align2, Color32, FontId, Margin, RichText, Rounding, Sense, Stroke, Ui};
use smk_core::project::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ palette

const ACCENT: Color32 = Color32::from_rgb(0x2B, 0x6C, 0xD9);
const ACCENT_HOVER: Color32 = Color32::from_rgb(0x22, 0x5C, 0xC0);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xE3, 0xEC, 0xFB);
const SIDEBAR: Color32 = Color32::from_rgb(0x1C, 0x25, 0x33);
const SIDEBAR_HOVER: Color32 = Color32::from_rgb(0x2A, 0x36, 0x48);
const SIDEBAR_TEXT: Color32 = Color32::from_rgb(0xD5, 0xDD, 0xE8);
const SIDEBAR_MUTED: Color32 = Color32::from_rgb(0x7D, 0x8A, 0x9C);
const BG: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF8);
const CARD: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(0xDD, 0xE2, 0xEA);
const TEXT: Color32 = Color32::from_rgb(0x1F, 0x27, 0x33);
const MUTED: Color32 = Color32::from_rgb(0x6B, 0x76, 0x85);
const HOVER: Color32 = Color32::from_rgb(0xF0, 0xF3, 0xF8);
const OK: Color32 = Color32::from_rgb(0x1E, 0x8E, 0x3E);
const ERR: Color32 = Color32::from_rgb(0xC6, 0x28, 0x28);
const WARN: Color32 = Color32::from_rgb(0xA8, 0x62, 0x00);

// ------------------------------------------------------------------ sections

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Info,
    Files,
    Shortcuts,
    Run,
    Dialogs,
    Interface,
    Requirements,
    Components,
    Tasks,
    Archives,
    Registry,
    Uninstall,
    Tools,
    Output,
    Build,
}

const GROUPS: &[(&str, &[(Section, &str, &str)])] = &[
    (
        "ПРОГРАММА",
        &[
            (Section::Info, "ℹ", "Информация"),
            (Section::Files, "🗀", "Файлы"),
            (Section::Shortcuts, "🔗", "Ярлыки"),
            (Section::Run, "▶", "Запуск программ"),
        ],
    ),
    (
        "УСТАНОВЩИК",
        &[
            (Section::Dialogs, "☰", "Диалоги"),
            (Section::Interface, "🎨", "Оформление"),
            (Section::Requirements, "⚙", "Требования"),
        ],
    ),
    (
        "ДОПОЛНИТЕЛЬНО",
        &[
            (Section::Components, "☑", "Компоненты"),
            (Section::Tasks, "✔", "Флажки"),
            (Section::Archives, "📦", "Внешние архивы"),
            (Section::Registry, "🗝", "Реестр"),
            (Section::Uninstall, "🗑", "Удаление"),
            (Section::Tools, "🔧", "Инструменты"),
        ],
    ),
    ("СБОРКА", &[(Section::Output, "💾", "Сжатие и вывод"), (Section::Build, "🔨", "Сборка")]),
];

fn section_header(s: Section) -> (&'static str, &'static str) {
    match s {
        Section::Info => ("Информация о программе", "Основные сведения и самые частые настройки установки."),
        Section::Files => ("Файлы", "Что упаковать в установщик и куда это скопировать на компьютере пользователя."),
        Section::Shortcuts => ("Ярлыки", "Ярлыки в меню «Пуск», на рабочем столе и в других местах."),
        Section::Run => ("Запуск программ", "Программы, запускаемые во время установки или по флажку в конце."),
        Section::Dialogs => ("Диалоги", "Какие страницы показывает мастер установки и что на них написано."),
        Section::Interface => ("Оформление", "Картинки, иконка, шрифт и языки установщика."),
        Section::Requirements => ("Требования", "Версия Windows, разрядность, права администратора, память."),
        Section::Components => ("Компоненты", "Части программы, которые пользователь может выбрать (например, озвучка)."),
        Section::Tasks => ("Флажки", "Свои флажки на странице «Дополнительные задачи». Их можно привязать к файлам, реестру, запуску."),
        Section::Archives => ("Внешние архивы", "Архивы рядом с setup.exe, которые распаковываются при установке."),
        Section::Registry => ("Реестр", "Записи реестра, создаваемые при установке."),
        Section::Uninstall => ("Удаление", "Деинсталлятор, запись в «Программы и компоненты», очистка файлов."),
        Section::Tools => ("Инструменты", "Вспомогательные файлы, нужные только во время установки (unarc.dll, precomp.exe…)."),
        Section::Output => ("Сжатие и вывод", "Метод сжатия и куда положить готовый setup.exe."),
        Section::Build => ("Сборка", "F9 — собрать. После сборки установщик проверяется автоматически."),
    }
}

/// Install-time folders with human names.
const FOLDERS: &[(&str, &str)] = &[
    ("{app}", "Папка программы"),
    ("{pf}", "Program Files"),
    ("{commonappdata}", "ProgramData (для всех)"),
    ("{userappdata}", "AppData\\Roaming"),
    ("{localappdata}", "AppData\\Local"),
    ("{userdocs}", "Документы"),
    ("{autodesktop}", "Рабочий стол"),
    ("{sys}", "Windows\\System32"),
    ("{win}", "Папка Windows"),
    ("{src}", "Папка с setup.exe"),
    ("{tmp}", "Временная папка"),
];

const INSTALL_ROOTS: &[(&str, &str)] = &[
    ("{pf}", "Program Files"),
    ("{sd}", "Системный диск (C:)"),
    ("{localappdata}", "AppData\\Local"),
    ("{userappdata}", "AppData\\Roaming"),
    ("{commonappdata}", "ProgramData"),
    ("{userdocs}", "Документы"),
];

const LOCATIONS: &[(&str, &str)] = &[
    ("{group}", "Меню «Пуск», папка программы"),
    ("{autoprograms}", "Меню «Пуск», Программы"),
    ("{autodesktop}", "Рабочий стол"),
    ("{autostartup}", "Автозагрузка"),
    ("{app}", "Папка программы"),
    ("{quicklaunch}", "Панель быстрого запуска"),
];

fn location_icon(loc: &str) -> &'static str {
    match loc {
        "{group}" | "{autoprograms}" | "{userprograms}" | "{commonprograms}" => "☰",
        l if is_desktop(l) => "🖵",
        "{autostartup}" | "{userstartup}" | "{commonstartup}" => "⏻",
        _ => "🗀",
    }
}

// ------------------------------------------------------------------ state

#[derive(Default)]
struct BuildState {
    running: bool,
    progress: f32,
    log: Vec<String>,
    output: Option<PathBuf>,
    error: Option<String>,
    /// Project produced by a background setup.exe import, to be opened.
    imported: Option<PathBuf>,
}

struct App {
    project: Project,
    path: Option<PathBuf>,
    saved: String,
    section: Section,
    sel: [Option<usize>; 12],
    build: Arc<Mutex<BuildState>>,
    cancel: Arc<AtomicBool>,
    status: String,
    archive_quick: String,
}

pub fn run(project: Option<PathBuf>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1220.0, 800.0]).with_min_inner_size([940.0, 600.0]).with_title("Setup Maker"),
        ..Default::default()
    };
    eframe::run_native(
        "Setup Maker",
        options,
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx);
            let mut app = App::new();
            if let Some(p) = project {
                app.open(&p);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = HOVER;
    v.window_rounding = Rounding::same(8.0);
    v.menu_rounding = Rounding::same(6.0);
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    let r = Rounding::same(5.0);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xEC, 0xEF, 0xF4);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xE4, 0xE8, 0xEF);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xDF, 0xE6, 0xF2);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0xDF, 0xE6, 0xF2);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.rounding = r;
    v.widgets.active.weak_bg_fill = ACCENT_SOFT;
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.rounding = r;
    v.widgets.open = v.widgets.active;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(14.0));
        s.text_styles.insert(Button, FontId::proportional(14.0));
        s.text_styles.insert(Small, FontId::proportional(12.0));
        s.text_styles.insert(Heading, FontId::proportional(22.0));
        s.text_styles.insert(Monospace, FontId::monospace(13.0));
        s.spacing.item_spacing = vec2(8.0, 7.0);
        s.spacing.button_padding = vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.combo_width = 240.0;
    });
}

fn ser(p: &Project) -> String {
    toml::to_string(p).unwrap_or_default()
}

fn new_project() -> Project {
    let mut p = Project::template("Моя программа", "MyProgram.exe", "");
    p.app.app_id = compile::new_guid();
    p
}

impl App {
    fn new() -> App {
        let p = new_project();
        let saved = ser(&p);
        App {
            project: p,
            path: None,
            saved,
            section: Section::Info,
            sel: [None; 12],
            build: Arc::new(Mutex::new(BuildState::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            status: "Новый проект".into(),
            archive_quick: String::new(),
        }
    }

    fn dirty(&self) -> bool {
        ser(&self.project) != self.saved
    }

    fn base_dir(&self) -> PathBuf {
        self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }

    /// Import a compiled Inno Setup installer into `<dir>/setup_project/`
    /// (in the background; the project opens when done).
    fn import_exe(&mut self, p: &Path) {
        if self.build.lock().unwrap().running {
            return;
        }
        let dir = crate::default_import_dir(p);
        let st = self.build.clone();
        {
            let mut s = st.lock().unwrap();
            *s = BuildState { running: true, ..Default::default() };
            s.log.push(format!("Импорт {} » {}", p.display(), dir.display()));
        }
        self.section = Section::Build;
        self.status = "Импорт установщика…".into();
        let p = p.to_path_buf();
        std::thread::spawn(move || {
            let st2 = st.clone();
            let log = move |s: &str| {
                let mut b = st2.lock().unwrap();
                if let Some(pct) = s.strip_prefix("\rИзвлечение файлов: ").and_then(|x| x.trim_end_matches('%').parse::<f32>().ok()) {
                    b.progress = pct / 100.0;
                } else if !s.is_empty() && !s.starts_with('\r') {
                    b.log.push(s.to_string());
                }
            };
            let r = crate::inno_exe::import(&p, &dir, &log).and_then(|r| {
                project_io::save(&r.imported.project, &r.project_path)?;
                Ok(r)
            });
            let mut b = st.lock().unwrap();
            b.running = false;
            match r {
                Ok(r) => {
                    for w in &r.imported.warnings {
                        b.log.push(format!("Импорт: {}", w));
                    }
                    b.log.push(format!("Проект сохранён: {}", r.project_path.display()));
                    b.imported = Some(r.project_path);
                }
                Err(e) => {
                    b.log.push(format!("Ошибка импорта: {}", e));
                    b.error = Some(e);
                }
            }
        });
    }

    fn open(&mut self, p: &Path) {
        if p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false) {
            self.import_exe(p);
            return;
        }
        match project_io::load(p) {
            Ok(mut proj) => {
                proj.simplify_shortcut_tasks();
                self.project = proj;
                self.path = Some(p.to_path_buf());
                self.saved = ser(&self.project);
                self.sel = [None; 12];
                self.status = format!("Открыт: {}", p.display());
            }
            Err(e) => self.status = format!("Ошибка: {}", e),
        }
    }

    fn save(&mut self, as_new: bool) -> bool {
        let path = if as_new || self.path.is_none() {
            match rfd::FileDialog::new().add_filter("Проект Setup Maker", &["smp"]).set_file_name("project.smp").save_file() {
                Some(p) => p,
                None => return false,
            }
        } else {
            self.path.clone().unwrap()
        };
        match project_io::save(&self.project, &path) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.saved = ser(&self.project);
                self.status = format!("Сохранено: {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("Ошибка сохранения: {}", e);
                false
            }
        }
    }

    fn start_build(&mut self) {
        if self.build.lock().unwrap().running {
            return;
        }
        if (self.path.is_none() || self.dirty()) && !self.save(false) {
            return;
        }
        let p = self.project.clone();
        let dir = self.base_dir();
        let st = self.build.clone();
        *st.lock().unwrap() = BuildState { running: true, ..Default::default() };
        self.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        self.section = Section::Build;
        let (s1, s2) = (st.clone(), st.clone());
        let opts = compile::BuildOptions {
            log: Arc::new(move |l| s1.lock().unwrap().log.push(l.to_string())),
            progress: Arc::new(move |f| s2.lock().unwrap().progress = f),
            cancel: self.cancel.clone(),
            defines: Default::default(),
        };
        std::thread::spawn(move || {
            let r = compile::build(&p, &dir, &opts);
            let mut s = st.lock().unwrap();
            s.running = false;
            match r {
                Ok(res) => {
                    s.output = Some(res.output.clone());
                    drop(s);
                    let _ = compile::verify(&res.output, &opts.log);
                }
                Err(e) => {
                    s.log.push(format!("ОШИБКА: {}", e));
                    s.error = Some(e);
                }
            }
        });
    }

    fn count(&self, s: Section) -> usize {
        let p = &self.project;
        match s {
            Section::Files => p.files.len(),
            Section::Shortcuts => p.shortcuts.len(),
            Section::Run => p.run.len() + p.uninstall_run.len(),
            Section::Components => p.components.len(),
            Section::Tasks => p.tasks.len(),
            Section::Archives => p.archives.len(),
            Section::Registry => p.registry.len(),
            Section::Tools => p.tools.len(),
            _ => 0,
        }
    }
}

// ------------------------------------------------------------------ helpers

/// Make a picked path relative to the project folder when possible.
fn rel(base: &Path, p: &Path) -> String {
    match p.strip_prefix(base) {
        Ok(r) => r.to_string_lossy().replace('/', "\\"),
        Err(_) => p.to_string_lossy().to_string(),
    }
}

fn pick_file(base: &Path, filters: &[(&str, &[&str])]) -> Option<String> {
    let mut d = rfd::FileDialog::new().set_directory(base);
    for (n, e) in filters {
        d = d.add_filter(*n, e);
    }
    d.pick_file().map(|p| rel(base, &p))
}

fn pick_folder(base: &Path) -> Option<String> {
    rfd::FileDialog::new().set_directory(base).pick_folder().map(|p| rel(base, &p))
}

/// Turns `{app}\x`, `{cm:LaunchProgram,{#AppName}}` etc. into readable text.
struct Pretty {
    defs: BTreeMap<String, String>,
    msgs: BTreeMap<String, String>,
}

impl Pretty {
    fn new(p: &Project) -> Pretty {
        let mut defs = p.defines.clone();
        defs.entry("AppName".into()).or_insert_with(|| p.app.name.clone());
        let mut msgs = BTreeMap::new();
        for l in ["*", "ru"] {
            if let Some(m) = p.messages.get(l) {
                msgs.extend(m.clone());
            }
        }
        Pretty { defs, msgs }
    }

    fn text(&self, s: &str) -> String {
        let mut s = s.to_string();
        for (k, v) in &self.defs {
            s = s.replace(&format!("{{#{}}}", k), v);
        }
        for _ in 0..16 {
            let Some(a) = s.find("{cm:") else { break };
            let Some(len) = s[a..].find('}') else { break };
            let inner = s[a + 4..a + len].to_string();
            let mut parts = inner.split(',');
            let key = parts.next().unwrap_or("").trim();
            let args: Vec<&str> = parts.map(str::trim).collect();
            let msg = self.msgs.get(key).map(|m| m.as_str()).or_else(|| smk_core::messages::builtin("ru", key));
            let txt = match msg {
                Some(m) => smk_core::messages::fmt(m, &args).replace('&', ""),
                None => inner.clone(),
            };
            s.replace_range(a..a + len + 1, &txt);
        }
        s
    }

    fn path(&self, s: &str) -> String {
        let t = self.text(s);
        if t.eq_ignore_ascii_case("{uninstallexe}") {
            return "Деинсталлятор".into();
        }
        let (loc, rest) = split_location(&t);
        match FOLDERS.iter().chain(LOCATIONS).chain(INSTALL_ROOTS).find(|f| f.0.eq_ignore_ascii_case(loc)) {
            Some(f) if rest.is_empty() => f.1.to_string(),
            Some(f) => format!("{}\\{}", f.1, rest),
            None => t,
        }
    }
}

/// Names and readable descriptions of components and checkboxes, for condition pickers.
#[derive(Clone)]
struct Choices {
    comps: Vec<(String, String)>,
    tasks: Vec<(String, String)>,
}

impl Choices {
    fn new(p: &Project, pr: &Pretty) -> Choices {
        Choices {
            comps: p.components.iter().filter(|c| !c.name.is_empty()).map(|c| (c.name.clone(), pr.text(&c.description))).collect(),
            tasks: p.tasks.iter().filter(|t| !t.name.is_empty()).map(|t| (t.name.clone(), pr.text(&t.description))).collect(),
        }
    }
}

fn join_root(root: &str, tail: &str) -> String {
    let tail = tail.trim_start_matches('\\');
    if tail.is_empty() {
        root.to_string()
    } else {
        format!("{}\\{}", root, tail)
    }
}

// ------------------------------------------------------------------ widgets

/// Width of a value field in forms (fixed, so cards never overflow).
fn field_w(ui: &Ui) -> f32 {
    let grid_w = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("smk_field_w")));
    ui.available_width().min(grid_w.unwrap_or(f32::INFINITY)).clamp(160.0, 560.0)
}

fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(16.0, 14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).size(15.5).strong().color(TEXT));
                ui.add_space(4.0);
            }
            add(ui);
        });
    ui.add_space(12.0);
}

fn hint(ui: &mut Ui, t: &str) {
    ui.label(RichText::new(t).size(12.5).color(MUTED));
}

fn grid(ui: &mut Ui, id: &str, f: impl FnOnce(&mut Ui)) {
    // Grid cells don't limit their width, so value fields get it from here.
    let w = ui.available_width() - 190.0 - 16.0 - 4.0;
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("smk_field_w"), w));
    egui::Grid::new(id).num_columns(2).spacing([16.0, 8.0]).min_col_width(190.0).show(ui, f);
}

fn label(ui: &mut Ui, t: &str) {
    ui.label(RichText::new(t).color(MUTED));
}

fn text(ui: &mut Ui, l: &str, v: &mut String, hint: &str) -> bool {
    label(ui, l);
    let r = ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(field_w(ui))).changed();
    ui.end_row();
    r
}

/// Text field that shows standard translated texts (`{cm:...}`) readably.
fn loc_text(ui: &mut Ui, l: &str, v: &mut String, hint_text: &str, pr: &Pretty) {
    label(ui, l);
    if v.contains("{cm:") {
        ui.horizontal(|ui| {
            ui.label(RichText::new(pr.text(v)).color(TEXT));
            hint(ui, "(на языке установщика)");
            if ui.small_button("Изменить").clicked() {
                *v = pr.text(v);
            }
        });
    } else {
        ui.add(egui::TextEdit::singleline(v).hint_text(hint_text).desired_width(field_w(ui)));
    }
    ui.end_row();
}

fn path_row(base: &Path, ui: &mut Ui, l: &str, v: &mut String, filters: &[(&str, &[&str])], folder: bool) {
    label(ui, l);
    ui.horizontal(|ui| {
        let w = field_w(ui) - 90.0;
        ui.add(egui::TextEdit::singleline(v).desired_width(w.max(120.0)));
        if ui.button("Обзор…").clicked() {
            let r = if folder { pick_folder(base) } else { pick_file(base, filters) };
            if let Some(p) = r {
                *v = p;
            }
        }
    });
    ui.end_row();
}

fn check(ui: &mut Ui, l: &str, v: &mut bool) -> bool {
    ui.label("");
    let r = ui.checkbox(v, l).changed();
    ui.end_row();
    r
}

fn combo(ui: &mut Ui, l: &str, id: &str, v: &mut String, opts: &[(&str, &str)]) {
    label(ui, l);
    combo_box(ui, id, v, opts, 300.0);
    ui.end_row();
}

fn combo_box(ui: &mut Ui, id: &str, v: &mut String, opts: &[(&str, &str)], w: f32) -> bool {
    let before = v.clone();
    let cur = opts.iter().find(|o| o.0 == v.as_str()).map(|o| o.1).unwrap_or(v.as_str()).to_string();
    egui::ComboBox::from_id_salt(id).selected_text(cur).width(w).show_ui(ui, |ui| {
        for (k, n) in opts {
            ui.selectable_value(v, k.to_string(), *n);
        }
    });
    *v != before
}

/// Combo with known folders + editable rest of the path.
fn folder_field(ui: &mut Ui, id: &str, v: &mut String, roots: &[(&str, &str)], tail_hint: &str) {
    let total = field_w(ui);
    ui.horizontal(|ui| {
        root_combo(ui, id, v, roots);
        let (loc, tail) = split_location(v);
        if roots.iter().any(|r| r.0.eq_ignore_ascii_case(loc)) {
            let loc = loc.to_string();
            let mut t = tail.to_string();
            ui.label(RichText::new("\\").color(MUTED));
            if ui.add(egui::TextEdit::singleline(&mut t).hint_text(tail_hint).desired_width((total - 262.0).max(120.0))).changed() {
                *v = join_root(&loc, &t);
            }
        } else {
            ui.add(egui::TextEdit::singleline(v).hint_text("C:\\Путь").desired_width((total - 240.0).max(120.0)));
        }
    });
}

fn root_combo(ui: &mut Ui, id: &str, v: &mut String, roots: &[(&str, &str)]) {
    let (loc, tail) = split_location(v);
    let cur = roots.iter().position(|r| r.0.eq_ignore_ascii_case(loc));
    let tail = tail.to_string();
    let mut sel = cur;
    egui::ComboBox::from_id_salt(id).width(230.0).selected_text(cur.map(|i| roots[i].1).unwrap_or("Другой путь")).show_ui(ui, |ui| {
        for (i, r) in roots.iter().enumerate() {
            ui.selectable_value(&mut sel, Some(i), r.1);
        }
        ui.selectable_value(&mut sel, None, "Другой путь…");
    });
    if sel != cur {
        let tail = if cur.is_some() { tail } else { String::new() };
        *v = match sel {
            Some(i) => join_root(roots[i].0, &tail),
            None => join_root("C:", &tail),
        };
    }
}

/// `a or b or c` -> [a, b, c]; None for anything more complex.
fn or_list(e: &str) -> Option<Vec<String>> {
    if e.trim().is_empty() {
        return Some(vec![]);
    }
    let parts: Vec<String> = e.split(" or ").map(|s| s.trim().to_string()).collect();
    let simple = parts.iter().all(|p| !p.is_empty() && !p.contains(|c: char| c.is_whitespace() || "()!&|".contains(c)) && !p.eq_ignore_ascii_case("not"));
    simple.then_some(parts)
}

/// Checkboxes over `items`, stored as an `a or b` expression.
fn multi_pick(ui: &mut Ui, expr: &mut String, items: &[(String, String)], empty: &str) {
    match or_list(expr) {
        Some(mut sel) => {
            ui.horizontal_wrapped(|ui| {
                if items.is_empty() {
                    hint(ui, empty);
                }
                let mut changed = false;
                for (n, d) in items {
                    let pos = sel.iter().position(|s| s.eq_ignore_ascii_case(n));
                    let mut on = pos.is_some();
                    let d = if d.trim().is_empty() { n.as_str() } else { d.as_str() };
                    if ui.checkbox(&mut on, d).changed() {
                        changed = true;
                        match pos {
                            Some(i) => {
                                sel.remove(i);
                            }
                            None => sel.push(n.clone()),
                        }
                    }
                }
                if changed {
                    *expr = sel.join(" or ");
                }
            });
        }
        None => {
            ui.add(egui::TextEdit::singleline(expr).desired_width(field_w(ui)));
        }
    }
}

fn conditions(ui: &mut Ui, c: &mut Conditions, ch: &Choices) {
    if !ch.comps.is_empty() || !c.components.is_empty() {
        label(ui, "Только для компонентов:");
        multi_pick(ui, &mut c.components, &ch.comps, "");
        ui.end_row();
    }
    if !ch.tasks.is_empty() || !c.tasks.is_empty() {
        label(ui, "Только если отмечен флажок:");
        multi_pick(ui, &mut c.tasks, &ch.tasks, "");
        ui.end_row();
    }
    if !c.check.is_empty() {
        text(ui, "Доп. условие:", &mut c.check, "");
    }
}

fn advanced_check(ui: &mut Ui, c: &mut Conditions) {
    if c.check.is_empty() {
        ui.collapsing("Условие для опытных", |ui| {
            hint(ui, "Выражение: IsWin64, not Is64BitInstallMode, FileExists('{app}\\x.exe'), RegKeyExists('HKLM\\…')");
            ui.add(egui::TextEdit::singleline(&mut c.check).desired_width(field_w(ui)));
        });
    }
}

/// Item row in a list (custom-painted, full width).
fn list_row(ui: &mut Ui, selected: bool, text: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 5.0, ACCENT_SOFT);
        p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 1.5, ACCENT);
    } else if resp.hovered() {
        p.rect_filled(rect, 5.0, HOVER);
    }
    p.with_clip_rect(rect.shrink2(vec2(4.0, 0.0))).text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(14.0), TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Master/detail list editor: list on the left, form on the right.
fn list_editor<T: Default + Clone>(
    ui: &mut Ui,
    id: &str,
    items: &mut Vec<T>,
    sel: &mut Option<usize>,
    empty: &str,
    title: impl Fn(&T) -> String,
    mut form: impl FnMut(&mut Ui, &mut T),
) {
    ui.horizontal(|ui| {
        if ui.button("➕ Добавить").clicked() {
            items.push(T::default());
            *sel = Some(items.len() - 1);
        }
        let has = sel.map(|i| i < items.len()).unwrap_or(false);
        if ui.add_enabled(has, egui::Button::new("⎘ Копия")).clicked() {
            let i = sel.unwrap();
            items.insert(i + 1, items[i].clone());
            *sel = Some(i + 1);
        }
        if ui.add_enabled(has, egui::Button::new("✖ Удалить")).clicked() {
            items.remove(sel.unwrap());
            *sel = None;
        }
        if ui.add_enabled(has && sel.unwrap() > 0, egui::Button::new("⬆")).on_hover_text("Выше").clicked() {
            let i = sel.unwrap();
            items.swap(i, i - 1);
            *sel = Some(i - 1);
        }
        if ui.add_enabled(has && sel.unwrap() + 1 < items.len(), egui::Button::new("⬇")).on_hover_text("Ниже").clicked() {
            let i = sel.unwrap();
            items.swap(i, i + 1);
            *sel = Some(i + 1);
        }
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(330.0);
            egui::Frame::none().fill(CARD).stroke(Stroke::new(1.0_f32, BORDER)).rounding(Rounding::same(8.0)).inner_margin(Margin::same(6.0)).show(ui, |ui| {
                ui.set_width(316.0);
                egui::ScrollArea::vertical().id_salt(format!("{}_list", id)).max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                    if items.is_empty() {
                        ui.add_space(8.0);
                        ui.vertical_centered(|ui| hint(ui, empty));
                        ui.add_space(8.0);
                    }
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, it) in items.iter().enumerate() {
                        let t = title(it);
                        let t = if t.trim().is_empty() { "(без названия)".to_string() } else { t };
                        if list_row(ui, *sel == Some(i), &t).clicked() {
                            *sel = Some(i);
                        }
                    }
                });
            });
        });
        ui.add_space(4.0);
        ui.vertical(|ui| match sel {
            Some(i) if *i < items.len() => {
                egui::Frame::none().fill(CARD).stroke(Stroke::new(1.0_f32, BORDER)).rounding(Rounding::same(8.0)).inner_margin(Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    form(ui, &mut items[*i]);
                });
            }
            _ => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| hint(ui, "Выберите элемент в списке или нажмите «Добавить»."));
            }
        });
    });
}

/// Big selectable tile with a title and a description line.
fn tile(ui: &mut Ui, sel: bool, title: &str, desc: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(250.0, 56.0), Sense::click());
    let fill = if sel {
        ACCENT_SOFT
    } else if resp.hovered() {
        HOVER
    } else {
        CARD
    };
    let p = ui.painter();
    p.rect(rect, 8.0, fill, Stroke::new(if sel { 1.5_f32 } else { 1.0 }, if sel { ACCENT } else { BORDER }));
    p.text(rect.left_top() + vec2(14.0, 10.0), Align2::LEFT_TOP, title, FontId::proportional(15.0), if sel { ACCENT } else { TEXT });
    p.text(rect.left_top() + vec2(14.0, 32.0), Align2::LEFT_TOP, desc, FontId::proportional(12.0), MUTED);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn unique_name(prefix: &str, taken: &[String]) -> String {
    let mut n = 1;
    loop {
        let s = format!("{}{}", prefix, n);
        if !taken.iter().any(|t| t.eq_ignore_ascii_case(&s)) {
            return s;
        }
        n += 1;
    }
}

fn mb_field(ui: &mut Ui, bytes: &mut u64) {
    let mut mb = *bytes / 1_048_576;
    if ui.add(egui::DragValue::new(&mut mb).range(0..=10_000_000).suffix(" МБ")).changed() {
        *bytes = mb * 1_048_576;
    }
}

fn hex_to_rgb(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

// ------------------------------------------------------------------ app

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let imported = self.build.lock().unwrap().imported.take();
        if let Some(p) = imported {
            self.open(&p);
            self.section = Section::Build;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F9)) {
            self.start_build();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save(false);
        }
        let dirty = self.dirty();
        let title = format!(
            "Setup Maker {} — {}{}",
            smk_core::VERSION,
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "новый проект".into()),
            if dirty { " *" } else { "" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        self.sidebar(ctx);
        self.toolbar(ctx, dirty);
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::none().fill(CARD).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(Margin::symmetric(14.0, 5.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(12.5).color(MUTED));
                    let s = self.build.lock().unwrap();
                    if s.running {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(egui::ProgressBar::new(s.progress).desired_width(220.0).show_percentage());
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                    }
                });
            });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(Margin { left: 24.0, right: 18.0, top: 18.0, bottom: 8.0 })).show(ctx, |ui| {
            let (t, sub) = section_header(self.section);
            ui.label(RichText::new(t).size(23.0).strong().color(TEXT));
            ui.label(RichText::new(sub).color(MUTED));
            ui.add_space(12.0);
            egui::ScrollArea::vertical().id_salt("main").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(1100.0));
                match self.section {
                    Section::Info => self.ui_info(ui),
                    Section::Files => self.ui_files(ui),
                    Section::Shortcuts => self.ui_shortcuts(ui),
                    Section::Run => self.ui_run(ui),
                    Section::Dialogs => self.ui_dialogs(ui),
                    Section::Interface => self.ui_interface(ui),
                    Section::Requirements => self.ui_requirements(ui),
                    Section::Components => self.ui_components(ui),
                    Section::Tasks => self.ui_tasks(ui),
                    Section::Archives => self.ui_archives(ui),
                    Section::Registry => self.ui_registry(ui),
                    Section::Uninstall => self.ui_uninstall(ui),
                    Section::Tools => self.ui_tools(ui),
                    Section::Output => self.ui_output(ui),
                    Section::Build => self.ui_build(ui, ctx),
                }
                ui.add_space(16.0);
            });
        });
    }
}

impl App {
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(222.0)
            .frame(egui::Frame::none().fill(SIDEBAR).inner_margin(Margin::symmetric(10.0, 14.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                    ui.painter().rect_filled(r, 8.0, ACCENT);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "📦", FontId::proportional(19.0), Color32::WHITE);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("Setup Maker").size(16.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new(format!("версия {}", smk_core::VERSION)).size(11.5).color(SIDEBAR_MUTED));
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("nav_scroll").show(ui, |ui| {
                    for (g, items) in GROUPS {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(*g).size(11.0).strong().color(SIDEBAR_MUTED));
                        });
                        ui.add_space(2.0);
                        for (s, icon, name) in items.iter() {
                            if self.nav_item(ui, *s, icon, name).clicked() {
                                self.section = *s;
                            }
                        }
                    }
                });
            });
    }

    fn nav_item(&self, ui: &mut Ui, s: Section, icon: &str, name: &str) -> egui::Response {
        let sel = self.section == s;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        let p = ui.painter();
        if sel {
            p.rect_filled(rect, 6.0, ACCENT);
        } else if resp.hovered() {
            p.rect_filled(rect, 6.0, SIDEBAR_HOVER);
        }
        let col = if sel { Color32::WHITE } else { SIDEBAR_TEXT };
        p.text(rect.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), col);
        p.text(rect.left_center() + vec2(40.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(14.5), col);
        let n = self.count(s);
        if n > 0 {
            let c = if sel { Color32::from_rgb(0xCF, 0xDE, 0xF7) } else { SIDEBAR_MUTED };
            p.text(rect.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, n.to_string(), FontId::proportional(12.5), c);
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn toolbar(&mut self, ctx: &egui::Context, dirty: bool) {
        egui::TopBottomPanel::top("toolbar")
            .frame(egui::Frame::none().fill(CARD).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(Margin::symmetric(14.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("🗋  Новый").clicked() {
                        self.project = new_project();
                        self.path = None;
                        self.saved = String::new();
                        self.sel = [None; 12];
                        self.section = Section::Info;
                        self.status = "Новый проект".into();
                    }
                    if ui.button("🗁  Открыть").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Проект Setup Maker", &["smp"]).pick_file() {
                            self.open(&p);
                        }
                    }
                    let save = if dirty { "💾  Сохранить •" } else { "💾  Сохранить" };
                    if ui.button(save).on_hover_text("Ctrl+S").clicked() {
                        self.save(false);
                    }
                    if ui.button("Сохранить как…").clicked() {
                        self.save(true);
                    }
                    ui.separator();
                    if ui
                        .button("📥  Импорт setup.exe")
                        .on_hover_text("Разобрать готовый установщик Inno Setup 5.2.5–6.3 в проект: файлы, ярлыки, картинки, архивы")
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Установщик", &["exe"]).pick_file() {
                            self.import_exe(&p);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let running = self.build.lock().unwrap().running;
                        let b = egui::Button::new(RichText::new("🔨  Собрать  F9").strong().color(Color32::WHITE))
                            .fill(if running { MUTED } else { ACCENT })
                            .rounding(Rounding::same(6.0))
                            .min_size(vec2(140.0, 30.0));
                        let r = ui.add_enabled(!running, b);
                        if r.hovered() && !running {
                            ui.painter().rect_stroke(r.rect, 6.0, Stroke::new(1.0_f32, ACCENT_HOVER));
                        }
                        if r.clicked() {
                            self.start_build();
                        }
                    });
                });
            });
    }

    // ------------------------------------------------------------- Info

    fn ui_info(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        card(ui, "Программа", |ui| {
            let a = &mut self.project.app;
            let old_name = a.name.clone();
            let old_exe = a.exe.clone();
            grid(ui, "info", |ui| {
                text(ui, "Название:", &mut a.name, "Моя программа");
                text(ui, "Версия:", &mut a.version, "1.0");
                text(ui, "Издатель:", &mut a.publisher, "Название компании или автора");
                text(ui, "Сайт:", &mut a.url, "https://");
                label(ui, "Главный файл программы:");
                ui.horizontal(|ui| {
                    let w = field_w(ui) - 90.0;
                    ui.add(egui::TextEdit::singleline(&mut a.exe).hint_text("MyProgram.exe (путь внутри папки программы)").desired_width(w.max(120.0)));
                    if ui.button("Обзор…").clicked() {
                        if let Some(p) = pick_file(&base, &[("Программы", &["exe"])]) {
                            a.exe = p.rsplit('\\').next().unwrap_or(&p).to_string();
                        }
                    }
                });
                ui.end_row();
                text(ui, "Copyright:", &mut a.copyright, "© 2026 …");
            });
            // keep {#AppName} in sync with the name
            if a.name != old_name && self.project.defines.get("AppName").map(|v| v == &old_name).unwrap_or(true) {
                self.project.defines.insert("AppName".into(), a.name.clone());
            }
            if a.exe != old_exe && !old_exe.is_empty() {
                let (o, n) = (format!("{{app}}\\{}", old_exe), format!("{{app}}\\{}", a.exe));
                for s in &mut self.project.shortcuts {
                    if s.target.eq_ignore_ascii_case(&o) {
                        s.target = n.clone();
                    }
                }
                for r in &mut self.project.run {
                    if r.filename.eq_ignore_ascii_case(&o) {
                        r.filename = n.clone();
                    }
                }
            }
        });
        card(ui, "Ярлыки и запуск", |ui| self.quick_options(ui));
        card(ui, "Куда устанавливать", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "quickdir", |ui| {
                label(ui, "Папка установки:");
                folder_field(ui, "qdir", &mut s.default_dir, INSTALL_ROOTS, "{#AppName}");
                ui.end_row();
                text(ui, "Папка в меню «Пуск»:", &mut s.default_group, "{#AppName}");
            });
            hint(ui, "{#AppName} подставляет название программы.");
        });
        ui.collapsing(RichText::new("Дополнительные сведения").color(MUTED), |ui| {
            card(ui, "", |ui| {
                let a = &mut self.project.app;
                grid(ui, "info2", |ui| {
                    text(ui, "Тип версии:", &mut a.version_type, "stable / beta");
                    text(ui, "Поддержка:", &mut a.support_url, "https://");
                    text(ui, "Обновления:", &mut a.updates_url, "https://");
                    text(ui, "Контакт:", &mut a.contact, "e-mail");
                    text(ui, "Комментарий:", &mut a.comments, "");
                    text(ui, "Название с версией:", &mut a.verb_name, "пусто = «Название Версия»");
                    label(ui, "Код программы (GUID):");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut a.app_id).desired_width(330.0));
                        if ui.button("Новый").clicked() {
                            a.app_id = compile::new_guid();
                        }
                    });
                    ui.end_row();
                });
                hint(ui, "По этому коду установщик узнаёт уже установленную программу и ставит обновление поверх. Не меняйте его между версиями.");
            });
            card(ui, "Свойства setup.exe", |ui| {
                let v = &mut self.project.app.version_info;
                grid(ui, "vi", |ui| {
                    text(ui, "Версия файла:", &mut v.version, "пусто = версия программы");
                    text(ui, "Компания:", &mut v.company, "");
                    text(ui, "Copyright:", &mut v.copyright, "");
                    text(ui, "Продукт:", &mut v.product_name, "");
                    text(ui, "Версия продукта:", &mut v.product_version, "");
                    text(ui, "Описание:", &mut v.description, "");
                });
            });
            card(ui, "Переменные", |ui| {
                hint(ui, "Подставляются в любое поле как {#Имя}. Например, путь к исходникам: {#Src}\\bin");
                self.defines_editor(ui);
            });
        });
    }

    fn quick_options(&mut self, ui: &mut Ui) {
        let p = &mut self.project;
        if p.app.exe.trim().is_empty() {
            ui.label(RichText::new("Укажите главный файл программы — тогда можно будет создать ярлыки и запуск.").color(WARN));
            return;
        }
        let exe = format!("{{app}}\\{}", p.app.exe);
        let is_exe = |t: &str| t.eq_ignore_ascii_case(&exe);
        let find = |p: &Project, f: &dyn Fn(&str) -> bool| p.shortcuts.iter().position(|s| f(split_location(&s.name).0) && is_exe(&s.target));

        // Start menu
        let start = find(p, &|l| l == "{group}");
        let mut on = start.is_some();
        if ui.checkbox(&mut on, "Ярлык в меню «Пуск»").changed() {
            match start {
                Some(i) => {
                    p.shortcuts.remove(i);
                }
                None => p.shortcuts.insert(0, Shortcut { name: "{group}\\{#AppName}".into(), target: exe.clone(), ..Default::default() }),
            }
        }

        // Desktop
        let desk = find(p, &|l| is_desktop(l));
        ui.horizontal(|ui| {
            let mut on = desk.is_some();
            if ui.checkbox(&mut on, "Ярлык на рабочем столе").changed() {
                match desk {
                    Some(i) => {
                        p.shortcuts.remove(i);
                    }
                    None => p.shortcuts.push(Shortcut { name: "{autodesktop}\\{#AppName}".into(), target: exe.clone(), ask: "checked".into(), ..Default::default() }),
                }
            }
            if let Some(i) = find(p, &|l| is_desktop(l)) {
                let s = &mut p.shortcuts[i];
                if s.when.tasks.is_empty() {
                    ui.add_space(12.0);
                    combo_box(ui, "desk_ask", &mut s.ask, &[("", "создавать всегда"), ("checked", "спросить (флажок отмечен)"), ("unchecked", "спросить (флажок снят)")], 230.0);
                } else {
                    hint(ui, "  (по условию — см. «Ярлыки»)");
                }
            }
        });

        // Uninstall shortcut
        let un = p.shortcuts.iter().position(|s| s.target.eq_ignore_ascii_case("{uninstallexe}"));
        let mut on = un.is_some();
        if ui.checkbox(&mut on, "Ярлык «Удалить программу» в меню «Пуск»").changed() {
            match un {
                Some(i) => {
                    p.shortcuts.remove(i);
                }
                None => p.shortcuts.push(Shortcut { name: "{group}\\{cm:UninstallProgram,{#AppName}}".into(), target: "{uninstallexe}".into(), ..Default::default() }),
            }
        }

        // Launch after install
        let run = p.run.iter().position(|r| r.postinstall && is_exe(&r.filename));
        let mut on = run.is_some();
        if ui.checkbox(&mut on, "Предложить запустить программу после установки").changed() {
            match run {
                Some(i) => {
                    p.run.remove(i);
                }
                None => p.run.push(RunItem {
                    filename: exe.clone(),
                    description: "{cm:LaunchProgram,{#AppName}}".into(),
                    postinstall: true,
                    nowait: true,
                    skip_if_silent: true,
                    ..Default::default()
                }),
            }
        }
        hint(ui, "Остальные ярлыки и программы — в разделах «Ярлыки» и «Запуск программ».");
    }

    fn defines_editor(&mut self, ui: &mut Ui) {
        let mut remove = None;
        let mut rename: Option<(String, String)> = None;
        let keys: Vec<String> = self.project.defines.keys().cloned().collect();
        for k in keys {
            ui.horizontal(|ui| {
                // The name is edited in a temp buffer and applied on focus loss,
                // renaming a BTreeMap key every keystroke would reorder rows.
                let id = ui.make_persistent_id(("define_name", &k));
                let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| k.clone());
                let resp = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(150.0));
                if resp.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(id));
                    let new = buf.trim().trim_start_matches("{#").trim_end_matches('}').trim().to_string();
                    if !new.is_empty() && new != k && !self.project.defines.contains_key(&new) {
                        rename = Some((k.clone(), new));
                    }
                } else if resp.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, buf));
                }
                ui.label("=");
                if let Some(v) = self.project.defines.get_mut(&k) {
                    ui.add(egui::TextEdit::singleline(v).desired_width(420.0));
                }
                if ui.button("✖").clicked() {
                    remove = Some(k.clone());
                }
            });
        }
        if let Some(k) = remove {
            self.project.defines.remove(&k);
        }
        if let Some((old, new)) = rename {
            if let Some(v) = self.project.defines.remove(&old) {
                self.project.defines.insert(new, v);
            }
        }
        if ui.button("➕ Переменная").clicked() {
            let keys: Vec<String> = self.project.defines.keys().cloned().collect();
            self.project.defines.insert(unique_name("Var", &keys), String::new());
        }
    }

    // ------------------------------------------------------------- Files

    fn ui_files(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        ui.horizontal(|ui| {
            if ui.button("📄  Добавить файлы…").clicked() {
                if let Some(fs) = rfd::FileDialog::new().set_directory(&base).pick_files() {
                    for f in fs {
                        self.project.files.push(FileItem { source: rel(&base, &f), ..Default::default() });
                    }
                    self.sel[0] = Some(self.project.files.len() - 1);
                }
            }
            if ui.button("🗀  Добавить папку…").clicked() {
                if let Some(d) = pick_folder(&base) {
                    self.project.files.push(FileItem { source: format!("{}\\*", d), ..Default::default() });
                    self.sel[0] = Some(self.project.files.len() - 1);
                }
            }
        });
        ui.add_space(6.0);
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        list_editor(
            ui,
            "files",
            &mut self.project.files,
            &mut self.sel[0],
            "Добавьте файлы или папку программы",
            |f| format!("{}  »  {}", f.source, pr.path(&f.dest_dir)),
            |ui, f| {
                grid(ui, "file", |ui| {
                    path_row(&base, ui, "Источник:", &mut f.source, &[], false);
                    label(ui, "Куда установить:");
                    folder_field(ui, "fdest", &mut f.dest_dir, FOLDERS, "подпапка");
                    ui.end_row();
                    text(ui, "Переименовать в:", &mut f.dest_name, "только для одного файла");
                    check(ui, "Включая подпапки", &mut f.recursive);
                    combo(ui, "Если файл уже есть:", "ow", &mut f.overwrite, &[("always", "заменить"), ("ifnewer", "заменить, если новее"), ("never", "оставить старый")]);
                    check(ui, "Удалять при удалении программы", &mut f.uninstall);
                    conditions(ui, &mut f.when, &ch);
                });
                ui.collapsing("Ещё", |ui| {
                    grid(ui, "file2", |ui| {
                        label(ui, "Исключить:");
                        let mut ex = f.excludes.join(", ");
                        if ui.add(egui::TextEdit::singleline(&mut ex).hint_text("*.pdb, temp\\*").desired_width(field_w(ui))).changed() {
                            f.excludes = ex.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        }
                        ui.end_row();
                        check(ui, "Создавать пустые папки", &mut f.empty_dirs);
                        check(ui, "Пропустить, если источник не найден", &mut f.skip_if_missing);
                        check(ui, "Удалить после установки", &mut f.delete_after_install);
                        text(ui, "Атрибуты:", &mut f.attributes, "readonly hidden system");
                    });
                    advanced_check(ui, &mut f.when);
                });
            },
        );
    }

    // ------------------------------------------------------------- Shortcuts

    fn ui_shortcuts(&mut self, ui: &mut Ui) {
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        let exe = self.project.app.exe.clone();
        let main = format!("{{app}}\\{}", exe);
        list_editor(
            ui,
            "lnk",
            &mut self.project.shortcuts,
            &mut self.sel[6],
            "Ярлыков нет",
            |s| {
                let (loc, leaf) = split_location(&s.name);
                let place = LOCATIONS.iter().find(|l| l.0.eq_ignore_ascii_case(loc)).map(|l| l.1.split(", ").next().unwrap_or(l.1)).unwrap_or(loc);
                let ask = if s.ask.is_empty() { "" } else { "  (спросить)" };
                format!("{}  {}  —  {}{}", location_icon(loc), pr.text(leaf), place, ask)
            },
            |ui, s| {
                if s.name.is_empty() {
                    s.name = "{group}\\{#AppName}".into();
                    if s.target.is_empty() {
                        s.target = main.clone();
                    }
                }
                grid(ui, "lnk", |ui| {
                    label(ui, "Где создать:");
                    root_combo(ui, "lnkloc", &mut s.name, LOCATIONS);
                    ui.end_row();
                    let (loc, leaf) = split_location(&s.name);
                    if LOCATIONS.iter().any(|l| l.0.eq_ignore_ascii_case(loc)) {
                        let loc = loc.to_string();
                        let mut l = leaf.to_string();
                        loc_text(ui, "Название ярлыка:", &mut l, "{#AppName}", &pr);
                        if l != leaf {
                            s.name = join_root(&loc, &l);
                        }
                    } else {
                        text(ui, "Путь ярлыка:", &mut s.name, "");
                    }
                    label(ui, "На что указывает:");
                    let mut kind = if !exe.is_empty() && s.target.eq_ignore_ascii_case(&main) {
                        "main"
                    } else if s.target.eq_ignore_ascii_case("{uninstallexe}") {
                        "un"
                    } else {
                        "other"
                    }
                    .to_string();
                    let opts: &[(&str, &str)] = &[("main", "главный файл программы"), ("un", "деинсталлятор"), ("other", "другой файл или ссылка…")];
                    if combo_box(ui, "lnkkind", &mut kind, opts, 300.0) {
                        s.target = match kind.as_str() {
                            "main" => main.clone(),
                            "un" => "{uninstallexe}".into(),
                            _ => "{app}\\".into(),
                        };
                    }
                    ui.end_row();
                    if kind == "other" {
                        label(ui, "Файл:");
                        folder_field(ui, "lnktarget", &mut s.target, FOLDERS, "file.exe  или  https://…");
                        ui.end_row();
                    }
                    combo(ui, "Создавать:", "lnkask", &mut s.ask, &[("", "всегда"), ("checked", "спросить (флажок отмечен)"), ("unchecked", "спросить (флажок снят)")]);
                    text(ui, "Параметры запуска:", &mut s.params, "");
                    conditions(ui, &mut s.when, &ch);
                });
                ui.collapsing("Ещё", |ui| {
                    grid(ui, "lnk2", |ui| {
                        text(ui, "Рабочая папка:", &mut s.workdir, "пусто = папка файла");
                        text(ui, "Иконка:", &mut s.icon, "пусто = иконка файла");
                        label(ui, "Номер иконки:");
                        ui.add(egui::DragValue::new(&mut s.icon_index));
                        ui.end_row();
                        text(ui, "Подсказка:", &mut s.comment, "");
                    });
                    advanced_check(ui, &mut s.when);
                });
            },
        );
    }

    // ------------------------------------------------------------- Run

    fn run_form(ui: &mut Ui, r: &mut RunItem, uninstall: bool, ch: &Choices, pr: &Pretty) {
        grid(ui, if uninstall { "urun" } else { "run" }, |ui| {
            if !uninstall {
                label(ui, "Когда запускать:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut r.postinstall, false, "во время установки");
                    ui.radio_value(&mut r.postinstall, true, "флажком на последней странице");
                });
                ui.end_row();
            }
            label(ui, "Файл:");
            folder_field(ui, if uninstall { "urunf" } else { "runf" }, &mut r.filename, FOLDERS, "program.exe");
            ui.end_row();
            text(ui, "Параметры:", &mut r.params, "/quiet");
            if !uninstall {
                if r.postinstall {
                    loc_text(ui, "Текст флажка:", &mut r.description, "Запустить …", pr);
                    let mut checked = !r.unchecked;
                    if check(ui, "Флажок отмечен", &mut checked) {
                        r.unchecked = !checked;
                    }
                } else {
                    text(ui, "Текст во время запуска:", &mut r.status_msg, "Установка DirectX…");
                }
            }
            let mut wait = !r.nowait;
            if check(ui, "Ждать завершения", &mut wait) {
                r.nowait = !wait;
            }
            check(ui, "Скрыть окно", &mut r.hidden);
            check(ui, "Открыть как документ или ссылку", &mut r.shell_exec);
            conditions(ui, &mut r.when, ch);
        });
        ui.collapsing("Ещё", |ui| {
            grid(ui, "run2", |ui| {
                text(ui, "Рабочая папка:", &mut r.workdir, "");
                check(ui, "Пропустить, если файла нет", &mut r.skip_if_missing);
                if !uninstall {
                    check(ui, "Не запускать при тихой установке", &mut r.skip_if_silent);
                }
            });
            advanced_check(ui, &mut r.when);
        });
    }

    fn ui_run(&mut self, ui: &mut Ui) {
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        card(ui, "При установке", |ui| {
            hint(ui, "Например, установка DirectX / Visual C++ из {src}\\redist или флажок «Запустить программу» в конце.");
            list_editor(
                ui,
                "run",
                &mut self.project.run,
                &mut self.sel[8],
                "Ничего не запускается",
                |r| {
                    if r.postinstall {
                        let d = pr.text(&r.description);
                        format!("☑  {}", if d.is_empty() { pr.path(&r.filename) } else { d })
                    } else {
                        format!("▶  {}", pr.path(&r.filename))
                    }
                },
                |ui, r| Self::run_form(ui, r, false, &ch, &pr),
            );
        });
        card(ui, "При удалении программы", |ui| {
            list_editor(ui, "urun", &mut self.project.uninstall_run, &mut self.sel[9], "Ничего не запускается", |r| format!("▶  {}", pr.path(&r.filename)), |ui, r| {
                Self::run_form(ui, r, true, &ch, &pr)
            });
        });
    }

    // ------------------------------------------------------------- Dialogs

    fn msg_field(&mut self, ui: &mut Ui, label_text: &str, key: &str, multiline: bool) {
        label(ui, label_text);
        let m = self.project.messages.entry("*".into()).or_default();
        let mut v = m.get(key).cloned().unwrap_or_default();
        let def = smk_core::messages::builtin("ru", key).unwrap_or("").replace("[name/ver]", "[name]");
        let te = if multiline { egui::TextEdit::multiline(&mut v).desired_rows(2) } else { egui::TextEdit::singleline(&mut v) };
        if ui.add(te.hint_text(def).desired_width(field_w(ui))).changed() {
            if v.is_empty() {
                m.remove(key);
            } else {
                m.insert(key.into(), v);
            }
        }
        ui.end_row();
    }

    fn ui_dialogs(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        let rtf: &[(&str, &[&str])] = &[("RTF/TXT", &["rtf", "txt"])];
        card(ui, "Приветствие", |ui| {
            grid(ui, "dw", |ui| {
                check(ui, "Показывать страницу приветствия", &mut self.project.setup.show_welcome_page);
                if self.project.setup.show_welcome_page {
                    self.msg_field(ui, "Заголовок:", "WelcomeLabel1", false);
                    self.msg_field(ui, "Текст:", "WelcomeLabel2", true);
                }
            });
        });
        card(ui, "Лицензия, информация, пароль", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "dl", |ui| {
                path_row(&base, ui, "Лицензионное соглашение:", &mut s.license_file, rtf, false);
                path_row(&base, ui, "Информация до установки:", &mut s.info_before_file, rtf, false);
                path_row(&base, ui, "Информация после установки:", &mut s.info_after_file, rtf, false);
                text(ui, "Пароль установки:", &mut s.password, "пусто = без пароля");
            });
            hint(ui, "Пустое поле — страница не показывается.");
        });
        card(ui, "Папка установки", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "dd", |ui| {
                check(ui, "Разрешить выбрать папку", &mut s.show_dir_page);
                label(ui, "Папка по умолчанию:");
                folder_field(ui, "ddir", &mut s.default_dir, INSTALL_ROOTS, "{#AppName}");
                ui.end_row();
                check(ui, "Запоминать папку прошлой установки", &mut s.use_previous_dir);
                check(ui, "Разрешить установку в корень диска", &mut s.allow_root_directory);
            });
        });
        card(ui, "Меню «Пуск»", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "dg", |ui| {
                check(ui, "Разрешить выбрать папку в меню «Пуск»", &mut s.show_group_page);
                text(ui, "Название папки:", &mut s.default_group, "{#AppName}");
                check(ui, "Флажок «Не создавать папку в меню «Пуск»»", &mut s.allow_no_icons);
            });
        });
        card(ui, "Всё готово к установке", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "dr", |ui| {
                check(ui, "Показывать сводку перед установкой", &mut s.show_ready_page);
                if s.show_ready_page {
                    check(ui, "В сводке — папку установки", &mut s.always_show_dir_on_ready);
                    check(ui, "В сводке — папку меню «Пуск»", &mut s.always_show_group_on_ready);
                }
            });
        });
        card(ui, "Завершение", |ui| {
            grid(ui, "df", |ui| {
                check(ui, "Показывать страницу завершения", &mut self.project.setup.show_finished_page);
                if self.project.setup.show_finished_page {
                    self.msg_field(ui, "Заголовок:", "FinishedHeadingLabel", false);
                    self.msg_field(ui, "Текст:", "FinishedLabel", true);
                    check(ui, "Показывать время установки", &mut self.project.setup.show_install_time);
                }
                check(ui, "Предложить перезагрузить компьютер", &mut self.project.setup.restart_required);
            });
            hint(ui, "[name] в тексте — название программы.");
        });
        self.project.messages.retain(|_, m| !m.is_empty());
        card(ui, "Прочее", |ui| {
            let s = &mut self.project.setup;
            grid(ui, "do", |ui| {
                text(ui, "Заголовок окна:", &mut s.window_title, "пусто = «Установка — Название»");
                check(ui, "Отменять все изменения при ошибке или отмене", &mut s.rollback_on_error);
                label(ui, "Доп. место на диске:");
                mb_field(ui, &mut s.extra_disk_space);
                ui.end_row();
            });
            hint(ui, "Доп. место учитывается в проверке свободного места (например, под данные внешних архивов).");
        });
    }

    // ------------------------------------------------------------- Interface

    fn ui_interface(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        let imgs: &[(&str, &[&str])] = &[("Изображения", &["bmp", "png", "jpg", "jpeg", "gif"])];
        let i = &mut self.project.interface;
        card(ui, "Картинки и иконка", |ui| {
            grid(ui, "ui", |ui| {
                path_row(&base, ui, "Большая картинка (слева):", &mut i.wizard_image, imgs, false);
                path_row(&base, ui, "Маленькая (справа вверху):", &mut i.wizard_small_image, imgs, false);
                path_row(&base, ui, "Иконка setup.exe:", &mut i.setup_icon, &[("ICO", &["ico"])], false);
                label(ui, "Цвет, если нет картинки:");
                ui.horizontal(|ui| {
                    let mut rgb = hex_to_rgb(&i.accent_color).unwrap_or([0x1F, 0x5D, 0xA8]);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        i.accent_color = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                    }
                    ui.add(egui::TextEdit::singleline(&mut i.accent_color).hint_text("#1F5DA8").desired_width(90.0));
                });
                ui.end_row();
            });
            hint(ui, "Большая картинка 164×314 (или 328×628 для HiDPI), маленькая 55×55. BMP, PNG или JPG.");
        });
        card(ui, "Шрифт", |ui| {
            grid(ui, "font", |ui| {
                text(ui, "Шрифт мастера:", &mut i.font, "системный (Segoe UI, Candara…)");
                label(ui, "Размер:");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut i.font_size).range(0..=20).suffix(" pt"));
                    hint(ui, "0 = по умолчанию (9 pt)");
                });
                ui.end_row();
            });
        });
        card(ui, "Языки", |ui| {
            let s = &mut self.project.setup;
            for (id, name) in smk_core::messages::LANGS {
                let idx = i.languages.iter().position(|l| smk_core::messages::normalize_lang(&l.id) == *id);
                let mut on = idx.is_some();
                if ui.checkbox(&mut on, *name).changed() {
                    if on {
                        i.languages.push(Language { id: id.to_string(), ..Default::default() });
                    } else if let Some(k) = idx {
                        i.languages.remove(k);
                    }
                }
            }
            ui.add_space(4.0);
            grid(ui, "langdlg", |ui| {
                combo(ui, "Окно выбора языка:", "ld", &mut s.language_dialog, &[("auto", "если языков больше одного"), ("always", "всегда"), ("never", "никогда")]);
            });
            if i.languages.len() > 1 {
                ui.collapsing("Отдельная лицензия / информация для каждого языка", |ui| {
                    for l in i.languages.iter_mut() {
                        let id = smk_core::messages::normalize_lang(&l.id);
                        ui.label(RichText::new(smk_core::messages::lang_name(id)).strong());
                        grid(ui, &format!("lang{}", id), |ui| {
                            let rtf: &[(&str, &[&str])] = &[("RTF/TXT", &["rtf", "txt"])];
                            path_row(&base, ui, "Лицензия:", &mut l.license_file, rtf, false);
                            path_row(&base, ui, "Инфо до установки:", &mut l.info_before_file, rtf, false);
                            path_row(&base, ui, "Инфо после установки:", &mut l.info_after_file, rtf, false);
                        });
                    }
                });
            }
        });
    }

    // ------------------------------------------------------------- Requirements

    fn ui_requirements(&mut self, ui: &mut Ui) {
        let r = &mut self.project.requirements;
        card(ui, "", |ui| {
            grid(ui, "req", |ui| {
                combo(ui, "Минимальная Windows:", "minwin", &mut r.min_windows, &[("6.1sp1", "Windows 7 SP1"), ("6.2", "Windows 8"), ("6.3", "Windows 8.1"), ("10.0", "Windows 10"), ("10.0.22000", "Windows 11")]);
                combo(ui, "Разрядность:", "arch", &mut r.arch, &[("auto", "автоматически"), ("x64", "64-бит (Program Files)"), ("x86", "32-бит (Program Files (x86))")]);
                combo(ui, "Права:", "priv", &mut r.privileges, &[("admin", "администратор (запрос UAC)"), ("lowest", "обычный пользователь")]);
                label(ui, "Минимум памяти:");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.min_ram_mb).range(0..=1_000_000).suffix(" МБ"));
                    hint(ui, "0 = не проверять");
                });
                ui.end_row();
            });
        });
        ui.label(RichText::new("⚠ Сам установщик работает на Windows 10 и 11 (на Windows 7 не запустится).").color(WARN));
    }

    // ------------------------------------------------------------- Components

    fn ui_components(&mut self, ui: &mut Ui) {
        let pr = Pretty::new(&self.project);
        card(ui, "Компоненты", |ui| {
            hint(ui, "Пользователь отмечает их на странице «Выбор компонентов». Вложенность — через «\\» в коде: voice, voice\\rus.");
            let types: Vec<(String, String)> = self.project.types.iter().map(|t| (t.name.clone(), pr.text(&t.description))).collect();
            list_editor(
                ui,
                "comps",
                &mut self.project.components,
                &mut self.sel[4],
                "Компонентов нет — устанавливается всё",
                |c| format!("{}{}", "      ".repeat(c.name.matches('\\').count()), if c.description.is_empty() { c.name.clone() } else { pr.text(&c.description) }),
                |ui, c| {
                    grid(ui, "comp", |ui| {
                        loc_text(ui, "Название:", &mut c.description, "Русская озвучка", &pr);
                        text(ui, "Код:", &mut c.name, "voice\\rus");
                        check(ui, "Отмечен по умолчанию", &mut c.checked);
                        check(ui, "Обязательный (нельзя снять)", &mut c.fixed);
                        check(ui, "Выбор одного из соседних (переключатель)", &mut c.exclusive);
                        label(ui, "Доп. место на диске:");
                        mb_field(ui, &mut c.extra_disk_space);
                        ui.end_row();
                        if !types.is_empty() {
                            label(ui, "Входит в типы установки:");
                            ui.horizontal_wrapped(|ui| {
                                for (t, d) in &types {
                                    let mut on = c.types.contains(t);
                                    if ui.checkbox(&mut on, if d.is_empty() { t } else { d }).changed() {
                                        if on {
                                            c.types.push(t.clone());
                                        } else {
                                            c.types.retain(|x| x != t);
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                },
            );
        });
        card(ui, "Типы установки", |ui| {
            hint(ui, "Необязательно. Список вверху страницы компонентов: «Полная», «Компактная», «Выборочная»…");
            list_editor(ui, "types", &mut self.project.types, &mut self.sel[3], "Типов нет", |t| pr.text(&t.description), |ui, t| {
                grid(ui, "type", |ui| {
                    combo(
                        ui,
                        "Название:",
                        "tdesc",
                        &mut t.description,
                        &[("{cm:FullInstallation}", "Полная установка"), ("{cm:CompactInstallation}", "Компактная установка"), ("{cm:CustomInstallation}", "Выборочная установка")],
                    );
                    text(ui, "Или свой текст:", &mut t.description, "");
                    text(ui, "Код:", &mut t.name, "full");
                    check(ui, "Выборочная (пользователь сам отмечает компоненты)", &mut t.custom);
                });
            });
        });
        let names: Vec<String> = self.project.components.iter().map(|c| c.name.clone()).collect();
        for c in self.project.components.iter_mut().filter(|c| c.name.is_empty()) {
            c.name = unique_name("comp", &names);
        }
        let names: Vec<String> = self.project.types.iter().map(|t| t.name.clone()).collect();
        for t in self.project.types.iter_mut().filter(|t| t.name.is_empty()) {
            t.name = unique_name("type", &names);
        }
    }

    // ------------------------------------------------------------- Tasks

    fn ui_tasks(&mut self, ui: &mut Ui) {
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        hint(ui, "Флажки для ярлыков создаются автоматически (раздел «Ярлыки», «Создавать: спросить»). Здесь — свои: «Установить DirectX», «Ассоциировать файлы»…");
        ui.add_space(6.0);
        list_editor(
            ui,
            "tasks",
            &mut self.project.tasks,
            &mut self.sel[5],
            "Своих флажков нет",
            |t| format!("{}  {}", if t.unchecked { "☐" } else { "☑" }, pr.text(&t.description)),
            |ui, t| {
                grid(ui, "task", |ui| {
                    loc_text(ui, "Текст флажка:", &mut t.description, "Установить DirectX", &pr);
                    loc_text(ui, "Заголовок группы:", &mut t.group, "необязательно", &pr);
                    let mut on = !t.unchecked;
                    if check(ui, "Отмечен по умолчанию", &mut on) {
                        t.unchecked = !on;
                    }
                    check(ui, "Переключатель (один из группы)", &mut t.exclusive);
                    if !ch.comps.is_empty() || !t.components.is_empty() {
                        label(ui, "Только для компонентов:");
                        multi_pick(ui, &mut t.components, &ch.comps, "");
                        ui.end_row();
                    }
                });
                ui.collapsing("Ещё", |ui| {
                    grid(ui, "task2", |ui| {
                        text(ui, "Код:", &mut t.name, "");
                        text(ui, "Условие:", &mut t.check, "IsWin64");
                        text(ui, "Мин. версия Windows:", &mut t.min_version, "6.1");
                        text(ui, "Только ниже версии:", &mut t.only_below_version, "");
                    });
                });
                hint(ui, "Чтобы флажок что-то делал, отметьте его в условиях файла, архива, реестра или запуска.");
            },
        );
        let names: Vec<String> = self.project.tasks.iter().map(|t| t.name.clone()).collect();
        for t in self.project.tasks.iter_mut().filter(|t| t.name.is_empty()) {
            t.name = unique_name("task", &names);
        }
    }

    // ------------------------------------------------------------- Archives

    fn ui_archives(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        hint(ui, "Форматы: .smpk (свой, можно собрать прямо здесь), .7z, .zip, .tar(.gz/.xz/.zst/.br), FreeArc .arc (нужен unarc.dll в «Инструментах»).");
        ui.collapsing("Быстрый ввод: одна строка — один архив", |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.archive_quick).hint_text("data1.arc\ndata2.arc\nvoice_rus.7z -> {app}\\sound").desired_rows(5).desired_width(field_w(ui)).font(egui::TextStyle::Monospace));
            if ui.button("Добавить в список").clicked() {
                for l in self.archive_quick.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    self.project.archives.push(ArchiveItem::from_short(l));
                }
                self.archive_quick.clear();
            }
        });
        ui.add_space(6.0);
        list_editor(ui, "arcs", &mut self.project.archives, &mut self.sel[1], "Архивов нет", |a| format!("📦  {}  »  {}", a.file, pr.path(&a.dest)), |ui, a| {
            grid(ui, "arc", |ui| {
                text(ui, "Имя файла (рядом с setup.exe):", &mut a.file, "data1.arc  или  data*.arc");
                label(ui, "Распаковать в:");
                folder_field(ui, "adest", &mut a.dest, FOLDERS, "подпапка");
                ui.end_row();
                text(ui, "Пароль:", &mut a.password, "");
                check(ui, "Обязателен (ошибка, если не найден)", &mut a.required);
                text(ui, "Если не найден — попросить:", &mut a.disk_prompt, "Вставьте диск 2…");
                check(ui, "Удалить архив после распаковки", &mut a.delete_after);
                path_row(&base, ui, "Собрать из папки:", &mut a.pack_from, &[], true);
                conditions(ui, &mut a.when, &ch);
            });
            hint(ui, "«Собрать из папки» — сборщик сам создаст .smpk рядом с setup.exe.");
            ui.collapsing("Ещё", |ui| {
                grid(ui, "arc2", |ui| {
                    combo(ui, "Формат:", "afmt", &mut a.format, &[("", "по расширению"), ("smpk", "smpk"), ("7z", "7z"), ("zip", "zip"), ("tar", "tar"), ("tar.gz", "tar.gz"), ("tar.xz", "tar.xz"), ("tar.zst", "tar.zst"), ("tar.br", "tar.br"), ("arc", "FreeArc .arc")]);
                });
                advanced_check(ui, &mut a.when);
            });
        });
    }

    // ------------------------------------------------------------- Registry

    fn ui_registry(&mut self, ui: &mut Ui) {
        let pr = Pretty::new(&self.project);
        let ch = Choices::new(&self.project, &pr);
        list_editor(
            ui,
            "reg",
            &mut self.project.registry,
            &mut self.sel[7],
            "Записей нет",
            |r| format!("🗝  {}\\{}{}", r.root, pr.text(&r.subkey), if r.value_name.is_empty() { String::new() } else { format!("  [{}]", r.value_name) }),
            |ui, r| {
                grid(ui, "reg", |ui| {
                    combo(ui, "Раздел:", "root", &mut r.root, &[("HKA", "авто (HKLM для админа, иначе HKCU)"), ("HKLM", "HKEY_LOCAL_MACHINE"), ("HKCU", "HKEY_CURRENT_USER"), ("HKCR", "HKEY_CLASSES_ROOT"), ("HKU", "HKEY_USERS")]);
                    text(ui, "Ключ:", &mut r.subkey, "Software\\Компания\\Программа");
                    text(ui, "Имя параметра:", &mut r.value_name, "пусто = по умолчанию");
                    combo(ui, "Тип:", "vt", &mut r.value_type, &[("string", "строка"), ("expandsz", "строка с %переменными%"), ("multisz", "многострочный"), ("dword", "DWORD (число)"), ("qword", "QWORD"), ("binary", "двоичный"), ("none", "только создать ключ")]);
                    text(ui, "Значение:", &mut r.value_data, "{app}");
                    combo(ui, "При удалении программы:", "un", &mut r.uninstall, &[("deletevalue", "удалить параметр"), ("deletekeyifempty", "удалить ключ, если пуст"), ("deletekey", "удалить ключ целиком"), ("none", "оставить")]);
                    conditions(ui, &mut r.when, &ch);
                });
                ui.collapsing("Ещё", |ui| {
                    grid(ui, "reg2", |ui| {
                        check(ui, "Только если параметра ещё нет", &mut r.only_if_missing);
                        check(ui, "Сначала удалить ключ", &mut r.delete_key_first);
                        check(ui, "Удалить параметр (вместо записи)", &mut r.delete_value);
                    });
                    advanced_check(ui, &mut r.when);
                });
            },
        );
    }

    // ------------------------------------------------------------- Uninstall

    fn ui_uninstall(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        let pr = Pretty::new(&self.project);
        card(ui, "Деинсталлятор", |ui| {
            let u = &mut self.project.uninstall;
            grid(ui, "un", |ui| {
                check(ui, "Создавать деинсталлятор и запись в «Программы и компоненты»", &mut u.enabled);
                if u.enabled {
                    text(ui, "Название в списке программ:", &mut u.display_name, "пусто = «Название Версия»");
                    text(ui, "Иконка в списке:", &mut u.display_icon, "пусто = главный файл программы");
                    path_row(&base, ui, "Или файл .ico из проекта:", &mut u.icon_file, &[("ICO", &["ico"])], false);
                    label(ui, "Папка деинсталлятора:");
                    folder_field(ui, "undir", &mut u.files_dir, FOLDERS, "");
                    ui.end_row();
                    check(ui, "Предложить перезагрузку после удаления", &mut u.restart_computer);
                }
            });
        });
        let kinds: &[(&str, &str)] = &[("filesandordirs", "файлы и папки"), ("files", "только файлы"), ("dirifempty", "папку, если пуста")];
        card(ui, "Удалять при удалении программы", |ui| {
            hint(ui, "Обычно — вся папка программы, включая файлы, созданные после установки (настройки, сохранения).");
            list_editor(ui, "udel", &mut self.project.uninstall_delete, &mut self.sel[11], "Ничего дополнительно", |d| format!("🗑  {}", pr.path(&d.path)), |ui, d| {
                grid(ui, "udel", |ui| {
                    label(ui, "Путь:");
                    folder_field(ui, "udelp", &mut d.path, FOLDERS, "*.log");
                    ui.end_row();
                    combo(ui, "Что удалить:", "k2", &mut d.kind, kinds);
                });
            });
        });
        card(ui, "Удалять перед установкой", |ui| {
            hint(ui, "Например, старые файлы прошлой версии.");
            list_editor(ui, "idel", &mut self.project.install_delete, &mut self.sel[10], "Ничего", |d| format!("🗑  {}", pr.path(&d.path)), |ui, d| {
                grid(ui, "idel", |ui| {
                    label(ui, "Путь:");
                    folder_field(ui, "idelp", &mut d.path, FOLDERS, "old.dll");
                    ui.end_row();
                    combo(ui, "Что удалить:", "k1", &mut d.kind, kinds);
                });
            });
        });
    }

    // ------------------------------------------------------------- Tools

    fn ui_tools(&mut self, ui: &mut Ui) {
        let base = self.base_dir();
        hint(ui, "Распаковываются во временную папку {tmp} и удаляются после установки. Нужны для FreeArc (unarc.dll) и запуска своих утилит.");
        ui.add_space(6.0);
        list_editor(ui, "tools", &mut self.project.tools, &mut self.sel[2], "Инструментов нет", |t| format!("🔧  {}", t.source), |ui, t| {
            grid(ui, "tool", |ui| {
                path_row(&base, ui, "Файл:", &mut t.source, &[], false);
                text(ui, "Имя во временной папке:", &mut t.name, "пусто = как у файла");
            });
        });
    }

    // ------------------------------------------------------------- Output

    fn ui_output(&mut self, ui: &mut Ui) {
        let c = &mut self.project.compression;
        card(ui, "Сжатие", |ui| {
            let preset = match (c.method.as_str(), c.level) {
                ("lzma2", 9) if c.extreme => "max",
                ("lzma2", _) => "custom",
                ("zstd", _) => "fast",
                ("none", _) => "none",
                _ => "custom",
            };
            let mut p = preset.to_string();
            ui.horizontal_wrapped(|ui| {
                for (k, t, d) in [
                    ("max", "Максимальное", "LZMA2, меньше размер"),
                    ("fast", "Быстрая установка", "Zstd, распаковка в 5–10 раз быстрее"),
                    ("none", "Без сжатия", "для уже сжатых данных"),
                ] {
                    if tile(ui, p == k, t, d) {
                        p = k.to_string();
                    }
                }
            });
            if p != preset {
                match p.as_str() {
                    "max" => {
                        c.method = "lzma2".into();
                        c.level = 9;
                        c.extreme = true;
                        c.dict_mb = c.dict_mb.max(64);
                    }
                    "fast" => {
                        c.method = "zstd".into();
                        c.level = 19;
                    }
                    "none" => c.method = "none".into(),
                    _ => {}
                }
            }
            ui.add_space(4.0);
            ui.collapsing("Тонкая настройка", |ui| {
                grid(ui, "comp", |ui| {
                    combo(ui, "Метод:", "meth", &mut c.method, &[("lzma2", "LZMA2"), ("zstd", "Zstandard"), ("brotli", "Brotli"), ("none", "без сжатия")]);
                    let max = match c.method.as_str() {
                        "zstd" => 22,
                        "brotli" => 11,
                        _ => 9,
                    };
                    label(ui, "Уровень:");
                    ui.add(egui::Slider::new(&mut c.level, 0..=max));
                    ui.end_row();
                    if c.method == "lzma2" {
                        check(ui, "Extreme (медленнее, чуть лучше)", &mut c.extreme);
                        check(ui, "Фильтр для exe/dll (BCJ x86)", &mut c.bcj);
                    }
                    label(ui, "Словарь:");
                    ui.add(egui::Slider::new(&mut c.dict_mb, 1..=1024).logarithmic(true).suffix(" МБ"));
                    ui.end_row();
                    label(ui, "Потоков:");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut c.threads).range(0..=256));
                        hint(ui, "0 = авто");
                    });
                    ui.end_row();
                    check(ui, "Хранить одинаковые файлы один раз", &mut c.dedupe);
                });
                hint(ui, "Для распаковки нужно ОЗУ примерно размером со словарь.");
            });
        });
        card(ui, "Готовый установщик", |ui| {
            let o = &mut self.project.output;
            grid(ui, "out", |ui| {
                text(ui, "Папка:", &mut o.dir, "compiled");
                text(ui, "Имя файла (без .exe):", &mut o.base_name, "{#AppName}_setup");
                check(ui, "Разбить на части (setup-1.bin, setup-2.bin…)", &mut o.disk_spanning);
                if o.disk_spanning {
                    label(ui, "Размер части:");
                    ui.add(egui::DragValue::new(&mut o.slice_size_mb).range(1..=1_000_000).suffix(" МБ"));
                    ui.end_row();
                }
            });
        });
    }

    // ------------------------------------------------------------- Build

    fn ui_build(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let (running, progress, output, err) = {
            let s = self.build.lock().unwrap();
            (s.running, s.progress, s.output.clone(), s.error.clone())
        };
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                let b = egui::Button::new(RichText::new("🔨  Собрать установщик").size(16.0).strong().color(Color32::WHITE))
                    .fill(if running { MUTED } else { ACCENT })
                    .rounding(Rounding::same(6.0))
                    .min_size(vec2(220.0, 38.0));
                if ui.add_enabled(!running, b).clicked() {
                    self.start_build();
                }
                if running && ui.button("Отменить").clicked() {
                    self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if let Some(o) = &output {
                    if ui.button("📂  Открыть папку").clicked() {
                        if let Some(d) = o.parent() {
                            open_folder(d);
                        }
                    }
                }
            });
            if running || progress > 0.0 {
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage().animate(running).fill(ACCENT));
            }
            ui.add_space(4.0);
            if let Some(e) = err {
                ui.label(RichText::new(format!("✖  {}", e)).color(ERR));
            } else if let Some(o) = output {
                ui.label(RichText::new(format!("✔  Готово: {}", o.display())).color(OK).strong());
            } else if running {
                hint(ui, "Идёт сборка…");
            }
        });
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let log = self.build.lock().unwrap().log.join("\n");
        egui::Frame::none().fill(Color32::from_rgb(0x1C, 0x22, 0x2B)).rounding(Rounding::same(8.0)).inner_margin(Margin::same(12.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).max_height(460.0).auto_shrink([false, false]).show(ui, |ui| {
                let shown = if log.is_empty() { "Журнал сборки пуст." } else { log.as_str() };
                ui.label(RichText::new(shown).monospace().color(Color32::from_rgb(0xD0, 0xD7, 0xE1)));
            });
        });
    }
}

fn open_folder(d: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(d).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(d).spawn();
}
