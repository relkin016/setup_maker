//! Project editor GUI (Smart Install Maker–like: sections on the left,
//! forms on the right). Built with egui, runs on Linux and Windows.

use crate::{compile, iss, project_io};
use eframe::egui::{self, Color32, RichText, Ui};
use smk_core::project::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Info,
    Files,
    Archives,
    Tools,
    Dialogs,
    Interface,
    Components,
    Tasks,
    Shortcuts,
    Registry,
    Run,
    Requirements,
    Uninstall,
    Compression,
    Build,
}

const SECTIONS: &[(Section, &str, &str)] = &[
    (Section::Info, "ℹ", "Информация"),
    (Section::Files, "🗀", "Файлы"),
    (Section::Archives, "📦", "Внешние архивы"),
    (Section::Tools, "🔧", "Инструменты"),
    (Section::Dialogs, "☰", "Диалоги"),
    (Section::Interface, "🎨", "Интерфейс"),
    (Section::Components, "☑", "Компоненты"),
    (Section::Tasks, "✔", "Задачи"),
    (Section::Shortcuts, "🔗", "Ярлыки"),
    (Section::Registry, "🗝", "Реестр"),
    (Section::Run, "▶", "Запуск программ"),
    (Section::Requirements, "⚙", "Требования"),
    (Section::Uninstall, "🗑", "Деинсталляция"),
    (Section::Compression, "💾", "Сжатие и вывод"),
    (Section::Build, "🔨", "Сборка"),
];

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
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 760.0]).with_min_inner_size([900.0, 560.0]).with_title("Setup Maker"),
        ..Default::default()
    };
    eframe::run_native(
        "Setup Maker",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            let mut app = App::new();
            if let Some(p) = project {
                app.open(&p);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

fn ser(p: &Project) -> String {
    toml::to_string(p).unwrap_or_default()
}

impl App {
    fn new() -> App {
        let mut p = Project::template("My Program", "MyProgram.exe", "");
        p.app.app_id = compile::new_guid();
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
            s.log.push(format!("Импорт {} → {}", p.display(), dir.display()));
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
        let is_iss = p.extension().map(|e| e.eq_ignore_ascii_case("iss")).unwrap_or(false);
        let r = if is_iss {
            iss::import(p, &Default::default()).map(|i| {
                for w in &i.warnings {
                    self.build.lock().unwrap().log.push(format!("Импорт: {}", w));
                }
                i.project
            })
        } else {
            project_io::load(p)
        };
        match r {
            Ok(proj) => {
                self.project = proj;
                self.path = if is_iss { Some(p.with_extension("smp")) } else { Some(p.to_path_buf()) };
                self.saved = if is_iss { String::new() } else { ser(&self.project) };
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
        if self.path.is_none() || self.dirty() {
            if !self.save(false) {
                return;
            }
        }
        let p = self.project.clone();
        let dir = self.base_dir();
        let st = self.build.clone();
        {
            let mut s = st.lock().unwrap();
            *s = BuildState { running: true, ..Default::default() };
        }
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

    /// Make a picked path relative to the project folder when possible.
    fn rel(&self, p: &Path) -> String {
        let base = self.base_dir();
        match p.strip_prefix(&base) {
            Ok(r) => r.to_string_lossy().replace('/', "\\"),
            Err(_) => p.to_string_lossy().to_string(),
        }
    }

    fn pick_file(&self, filters: &[(&str, &[&str])]) -> Option<String> {
        let mut d = rfd::FileDialog::new().set_directory(self.base_dir());
        for (n, e) in filters {
            d = d.add_filter(*n, e);
        }
        d.pick_file().map(|p| self.rel(&p))
    }

    fn pick_folder(&self) -> Option<String> {
        rfd::FileDialog::new().set_directory(self.base_dir()).pick_folder().map(|p| self.rel(&p))
    }
}

// ------------------------------------------------------------------ widgets

fn text(ui: &mut Ui, label: &str, v: &mut String, hint: &str) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(f32::INFINITY));
    ui.end_row();
}

fn path_row(app: &App, ui: &mut Ui, label: &str, v: &mut String, filters: &[(&str, &[&str])], folder: bool) {
    ui.label(label);
    ui.horizontal(|ui| {
        let w = ui.available_width() - 90.0;
        ui.add(egui::TextEdit::singleline(v).desired_width(w.max(100.0)));
        if ui.button("Обзор…").clicked() {
            let r = if folder { app.pick_folder() } else { app.pick_file(filters) };
            if let Some(p) = r {
                *v = p;
            }
        }
    });
    ui.end_row();
}

fn check(ui: &mut Ui, label: &str, v: &mut bool) {
    ui.label("");
    ui.checkbox(v, label);
    ui.end_row();
}

fn combo(ui: &mut Ui, label: &str, id: &str, v: &mut String, opts: &[(&str, &str)]) {
    ui.label(label);
    let cur = opts.iter().find(|o| o.0 == v.as_str()).map(|o| o.1).unwrap_or(v.as_str()).to_string();
    egui::ComboBox::from_id_salt(id).selected_text(cur).width(260.0).show_ui(ui, |ui| {
        for (k, n) in opts {
            ui.selectable_value(v, k.to_string(), *n);
        }
    });
    ui.end_row();
}

fn conditions(ui: &mut Ui, c: &mut Conditions) {
    text(ui, "Компоненты:", &mut c.components, "напр. voice\\rus or voice\\eng");
    text(ui, "Задачи:", &mut c.tasks, "напр. desktopicon");
    text(ui, "Проверка (Check):", &mut c.check, "Is64BitInstallMode, not IsWin64, FileExists('{app}\\x.exe')");
}

fn grid(ui: &mut Ui, id: &str, f: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([12.0, 6.0]).min_col_width(170.0).show(ui, f);
}

fn heading(ui: &mut Ui, t: &str, sub: &str) {
    ui.label(RichText::new(t).size(20.0).strong());
    if !sub.is_empty() {
        ui.label(RichText::new(sub).color(Color32::from_gray(90)));
    }
    ui.separator();
}

/// Master/detail list editor: list on the left, form on the right.
fn list_editor<T: Default + Clone>(
    ui: &mut Ui,
    id: &str,
    items: &mut Vec<T>,
    sel: &mut Option<usize>,
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
        if ui.add_enabled(has && sel.unwrap() > 0, egui::Button::new("⬆")).clicked() {
            let i = sel.unwrap();
            items.swap(i, i - 1);
            *sel = Some(i - 1);
        }
        if ui.add_enabled(has && sel.unwrap() + 1 < items.len(), egui::Button::new("⬇")).clicked() {
            let i = sel.unwrap();
            items.swap(i, i + 1);
            *sel = Some(i + 1);
        }
        ui.label(format!("всего: {}", items.len()));
    });
    ui.add_space(4.0);
    let h = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(330.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt(format!("{}_list", id)).max_height(h - 20.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(310.0);
                    for (i, it) in items.iter().enumerate() {
                        let t = title(it);
                        let t = if t.trim().is_empty() { "(пусто)".to_string() } else { t };
                        if ui.selectable_label(*sel == Some(i), t).clicked() {
                            *sel = Some(i);
                        }
                    }
                });
            });
        });
        ui.vertical(|ui| {
            egui::ScrollArea::vertical().id_salt(format!("{}_form", id)).max_height(h - 10.0).show(ui, |ui| match sel {
                Some(i) if *i < items.len() => form(ui, &mut items[*i]),
                _ => {
                    ui.label(RichText::new("Выберите элемент слева или нажмите «Добавить».").color(Color32::GRAY));
                }
            });
        });
    });
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
        let title = format!(
            "Setup Maker {} — {}{}",
            smk_core::VERSION,
            self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "новый проект".into()),
            if self.dirty() { " *" } else { "" }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("🗋 Новый").clicked() {
                    let mut p = Project::template("My Program", "MyProgram.exe", "");
                    p.app.app_id = compile::new_guid();
                    self.project = p;
                    self.path = None;
                    self.saved = String::new();
                    self.sel = [None; 12];
                }
                if ui.button("🗁 Открыть").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Проекты", &["smp", "iss"]).pick_file() {
                        self.open(&p);
                    }
                }
                if ui.button("💾 Сохранить").clicked() {
                    self.save(false);
                }
                if ui.button("Сохранить как…").clicked() {
                    self.save(true);
                }
                if ui.button("🗁 Импорт .iss").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Inno Setup", &["iss"]).pick_file() {
                        self.open(&p);
                    }
                }
                if ui.button("📦 Импорт setup.exe").on_hover_text("Разобрать готовый установщик Inno Setup 5.2.5–6.3 в проект (файлы, задачи, ярлыки, картинки, архивы ISDone)").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Установщик Inno Setup", &["exe"]).pick_file() {
                        self.import_exe(&p);
                    }
                }
                ui.separator();
                let running = self.build.lock().unwrap().running;
                if ui.add_enabled(!running, egui::Button::new(RichText::new("🔨 Собрать (F9)").strong())).clicked() {
                    self.start_build();
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                let s = self.build.lock().unwrap();
                if s.running {
                    ui.add(egui::ProgressBar::new(s.progress).desired_width(240.0).show_percentage());
                    ctx.request_repaint_after(std::time::Duration::from_millis(150));
                }
            });
        });
        egui::SidePanel::left("sections").resizable(false).exact_width(200.0).show(ctx, |ui| {
            ui.add_space(6.0);
            for (s, icon, name) in SECTIONS {
                let t = RichText::new(format!(" {}   {}", icon, name)).size(15.0);
                let r = ui.allocate_ui_with_layout(egui::vec2(186.0, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_sized([186.0, 28.0], egui::SelectableLabel::new(self.section == *s, t))
                });
                if r.inner.clicked() {
                    self.section = *s;
                }
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("main").auto_shrink([false, false]).show(ui, |ui| match self.section {
                Section::Info => self.ui_info(ui),
                Section::Files => self.ui_files(ui),
                Section::Archives => self.ui_archives(ui),
                Section::Tools => self.ui_tools(ui),
                Section::Dialogs => self.ui_dialogs(ui),
                Section::Interface => self.ui_interface(ui),
                Section::Components => self.ui_components(ui),
                Section::Tasks => self.ui_tasks(ui),
                Section::Shortcuts => self.ui_shortcuts(ui),
                Section::Registry => self.ui_registry(ui),
                Section::Run => self.ui_run(ui),
                Section::Requirements => self.ui_requirements(ui),
                Section::Uninstall => self.ui_uninstall(ui),
                Section::Compression => self.ui_compression(ui),
                Section::Build => self.ui_build(ui, ctx),
            });
        });
    }
}

impl App {
    fn ui_info(&mut self, ui: &mut Ui) {
        heading(ui, "Информация о программе", "Отображается в мастере установки, в «Программы и компоненты» и в свойствах setup.exe");
        let a = &mut self.project.app;
        grid(ui, "info", |ui| {
            text(ui, "Название:", &mut a.name, "My Program");
            text(ui, "Версия:", &mut a.version, "1.0");
            text(ui, "Тип версии:", &mut a.version_type, "stable / beta");
            text(ui, "Издатель:", &mut a.publisher, "");
            text(ui, "Главный exe (в {app}):", &mut a.exe, "MyProgram.exe");
            text(ui, "Сайт:", &mut a.url, "https://");
            text(ui, "Поддержка:", &mut a.support_url, "");
            text(ui, "Обновления:", &mut a.updates_url, "");
            text(ui, "Контакт:", &mut a.contact, "e-mail");
            text(ui, "Комментарий:", &mut a.comments, "");
            text(ui, "Copyright:", &mut a.copyright, "");
            text(ui, "Название с версией:", &mut a.verb_name, "пусто = «Название Версия»");
            ui.label("AppId (GUID):");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut a.app_id).desired_width(330.0));
                if ui.button("Сгенерировать").clicked() {
                    a.app_id = compile::new_guid();
                }
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.collapsing("Ресурс версии setup.exe (необязательно)", |ui| {
            let v = &mut self.project.app.version_info;
            grid(ui, "vi", |ui| {
                text(ui, "Версия файла:", &mut v.version, "1.0.0.0 (пусто = версия программы)");
                text(ui, "Компания:", &mut v.company, "");
                text(ui, "Copyright:", &mut v.copyright, "");
                text(ui, "Продукт:", &mut v.product_name, "");
                text(ui, "Версия продукта:", &mut v.product_version, "");
                text(ui, "Описание:", &mut v.description, "");
            });
        });
        ui.collapsing("Переменные {#Имя} (defines)", |ui| {
            ui.label("Используются в любых полях как {#Имя}. Удобно для путей: {#InnoDir}\\MyApp\\files\\*");
            let mut remove = None;
            let keys: Vec<String> = self.project.defines.keys().cloned().collect();
            for k in keys {
                ui.horizontal(|ui| {
                    ui.label(format!("{} =", k));
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
            if ui.button("➕ Переменная").clicked() {
                let mut n = 1;
                while self.project.defines.contains_key(&format!("Var{}", n)) {
                    n += 1;
                }
                self.project.defines.insert(format!("Var{}", n), String::new());
            }
        });
    }

    fn ui_files(&mut self, ui: &mut Ui) {
        heading(ui, "Файлы", "Файлы и папки, упаковываемые внутрь установщика. Источник — файл, папка или маска (папка\\*). Назначение — с константами {app}, {sys}, {userappdata}...");
        ui.horizontal(|ui| {
            if ui.button("📄 Добавить файлы…").clicked() {
                if let Some(fs) = rfd::FileDialog::new().set_directory(self.base_dir()).pick_files() {
                    for f in fs {
                        let src = self.rel(&f);
                        self.project.files.push(FileItem { source: src, ..Default::default() });
                    }
                }
            }
            if ui.button("🗀 Добавить папку…").clicked() {
                if let Some(d) = self.pick_folder() {
                    self.project.files.push(FileItem { source: format!("{}\\*", d), ..Default::default() });
                }
            }
        });
        let app = &*self as *const App;
        let files = &mut self.project.files;
        list_editor(ui, "files", files, &mut self.sel[0], |f| format!("{}  →  {}", f.source, f.dest_dir), |ui, f| {
            let app = unsafe { &*app };
            grid(ui, "file", |ui| {
                path_row(app, ui, "Источник:", &mut f.source, &[], false);
                text(ui, "Папка назначения:", &mut f.dest_dir, "{app}");
                text(ui, "Новое имя файла:", &mut f.dest_name, "только для одиночного файла");
                check(ui, "Включая подпапки", &mut f.recursive);
                check(ui, "Создавать пустые папки", &mut f.empty_dirs);
                ui.label("Исключить:");
                let mut ex = f.excludes.join(", ");
                if ui.add(egui::TextEdit::singleline(&mut ex).hint_text("*.pdb, temp\\*").desired_width(f32::INFINITY)).changed() {
                    f.excludes = ex.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
                ui.end_row();
                combo(ui, "Перезапись:", "ow", &mut f.overwrite, &[("always", "всегда"), ("ifnewer", "если новее"), ("never", "только если нет")]);
                check(ui, "Удалять при деинсталляции", &mut f.uninstall);
                check(ui, "Пропустить, если источник не найден", &mut f.skip_if_missing);
                check(ui, "Удалить после установки", &mut f.delete_after_install);
                text(ui, "Атрибуты:", &mut f.attributes, "readonly hidden system");
                conditions(ui, &mut f.when);
            });
        });
    }

    fn ui_archives(&mut self, ui: &mut Ui) {
        heading(
            ui,
            "Внешние архивы",
            "Архивы лежат рядом с setup.exe и распаковываются при установке. Достаточно указать имя: data1.arc, data*.7z, voice.smpk -> {app}\\sound",
        );
        ui.label(RichText::new("Форматы: .smpk (свой, LZMA2/Zstd/Brotli — можно создать прямо здесь), .7z, .zip, .tar(.gz/.xz/.zst/.br), .arc (FreeArc — нужен unarc.dll или unarc.exe в «Инструментах»)").small());
        ui.collapsing("Быстрый ввод: одна строка — один архив", |ui| {
            ui.add(egui::TextEdit::multiline(&mut self.archive_quick).hint_text("data1.arc\ndata2.arc\nvoice_rus.7z -> {app}\\sound").desired_rows(5).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            if ui.button("Добавить в список").clicked() {
                for l in self.archive_quick.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    self.project.archives.push(ArchiveItem::from_short(l));
                }
                self.archive_quick.clear();
            }
        });
        let app = &*self as *const App;
        list_editor(ui, "arcs", &mut self.project.archives, &mut self.sel[1], |a| format!("{}  →  {}", a.file, a.dest), |ui, a| {
            let app = unsafe { &*app };
            grid(ui, "arc", |ui| {
                text(ui, "Имя файла (рядом с setup.exe):", &mut a.file, "data1.arc  или  data*.arc");
                text(ui, "Распаковать в:", &mut a.dest, "{app}");
                combo(ui, "Формат:", "afmt", &mut a.format, &[("", "авто по расширению"), ("smpk", "smpk"), ("7z", "7z"), ("zip", "zip"), ("tar", "tar"), ("tar.gz", "tar.gz"), ("tar.xz", "tar.xz"), ("tar.zst", "tar.zst"), ("tar.br", "tar.br"), ("arc", "FreeArc .arc")]);
                text(ui, "Пароль:", &mut a.password, "");
                check(ui, "Обязателен (ошибка, если не найден)", &mut a.required);
                text(ui, "Запрос диска, если не найден:", &mut a.disk_prompt, "Пожалуйста, вставьте диск 2…");
                check(ui, "Удалить архив после распаковки", &mut a.delete_after);
                path_row(app, ui, "Упаковать из папки при сборке:", &mut a.pack_from, &[], true);
                ui.label("");
                ui.label(RichText::new("Если задана папка — сборщик сам создаст .smpk рядом с setup.exe теми же настройками сжатия").small());
                ui.end_row();
                conditions(ui, &mut a.when);
            });
        });
    }

    fn ui_tools(&mut self, ui: &mut Ui) {
        heading(ui, "Инструменты", "Вспомогательные файлы, которые распаковываются только во временную папку {tmp} (как Flags: dontcopy в Inno): unarc.dll, precomp.exe, srep.exe и т. п.");
        let app = &*self as *const App;
        list_editor(ui, "tools", &mut self.project.tools, &mut self.sel[2], |t| t.source.clone(), |ui, t| {
            let app = unsafe { &*app };
            grid(ui, "tool", |ui| {
                path_row(app, ui, "Файл:", &mut t.source, &[], false);
                text(ui, "Имя в {tmp}:", &mut t.name, "пусто = как у файла");
            });
        });
    }

    fn ui_dialogs(&mut self, ui: &mut Ui) {
        heading(ui, "Диалоги мастера", "Страницы как в Inno Setup. Страницы лицензии/информации/компонентов/задач появляются автоматически, если для них есть данные.");
        let app = &*self as *const App;
        let app = unsafe { &*app };
        let s = &mut self.project.setup;
        grid(ui, "dlg", |ui| {
            check(ui, "Приветствие", &mut s.show_welcome_page);
            path_row(app, ui, "Лицензия (rtf/txt):", &mut s.license_file, &[("RTF/TXT", &["rtf", "txt"])], false);
            path_row(app, ui, "Информация до установки:", &mut s.info_before_file, &[("RTF/TXT", &["rtf", "txt"])], false);
            text(ui, "Пароль установки:", &mut s.password, "пусто = без пароля");
            check(ui, "Выбор папки установки", &mut s.show_dir_page);
            text(ui, "Папка по умолчанию:", &mut s.default_dir, "{pf}\\{#AppName}");
            check(ui, "Разрешить установку в корень диска", &mut s.allow_root_directory);
            check(ui, "Использовать папку предыдущей установки", &mut s.use_previous_dir);
            check(ui, "Выбор папки в меню «Пуск»", &mut s.show_group_page);
            text(ui, "Папка в меню «Пуск»:", &mut s.default_group, "{#AppName}");
            check(ui, "Флажок «Не создавать папку в меню Пуск»", &mut s.allow_no_icons);
            check(ui, "Страница «Всё готово к установке»", &mut s.show_ready_page);
            check(ui, "Показывать папку на странице «Готово»", &mut s.always_show_dir_on_ready);
            check(ui, "Показывать группу на странице «Готово»", &mut s.always_show_group_on_ready);
            path_row(app, ui, "Информация после установки:", &mut s.info_after_file, &[("RTF/TXT", &["rtf", "txt"])], false);
            check(ui, "Страница завершения", &mut s.show_finished_page);
            check(ui, "Показывать время установки", &mut s.show_install_time);
            check(ui, "Предложить перезагрузку", &mut s.restart_required);
            check(ui, "Откат изменений при ошибке/отмене", &mut s.rollback_on_error);
            text(ui, "Заголовок окна:", &mut s.window_title, "пусто = «Установка — Название»");
            let mb = s.extra_disk_space / 1048576;
            let mut mbv = mb;
            ui.label("Доп. место на диске, МБ:");
            ui.add(egui::DragValue::new(&mut mbv).range(0..=10_000_000));
            ui.end_row();
            if mbv != mb {
                s.extra_disk_space = mbv * 1048576;
            }
        });
        ui.add_space(8.0);
        ui.collapsing("Свои тексты мастера (переопределение сообщений)", |ui| {
            ui.label("Ключи как в Inno: WelcomeLabel1, WelcomeLabel2, FinishedLabel... Язык «*» — для всех языков. [name] — название программы.");
            for key in ["WelcomeLabel1", "WelcomeLabel2", "FinishedHeadingLabel", "FinishedLabel"] {
                ui.horizontal(|ui| {
                    ui.label(format!("{:<22}", key));
                    let m = self.project.messages.entry("*".into()).or_default();
                    let mut v = m.get(key).cloned().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut v).hint_text("стандартный текст").desired_width(520.0)).changed() {
                        if v.is_empty() {
                            m.remove(key);
                        } else {
                            m.insert(key.into(), v);
                        }
                    }
                });
            }
            self.project.messages.retain(|_, m| !m.is_empty());
        });
    }

    fn ui_interface(&mut self, ui: &mut Ui) {
        heading(ui, "Интерфейс", "Внешний вид — классический мастер Inno Setup. Картинки: большая 164×314 (×2 для HiDPI), маленькая 55×55. BMP/PNG/JPG.");
        let app = &*self as *const App;
        let app = unsafe { &*app };
        let imgs: &[(&str, &[&str])] = &[("Изображения", &["bmp", "png", "jpg", "jpeg", "gif"])];
        let i = &mut self.project.interface;
        grid(ui, "ui", |ui| {
            path_row(app, ui, "Большая картинка:", &mut i.wizard_image, imgs, false);
            path_row(app, ui, "Маленькая картинка:", &mut i.wizard_small_image, imgs, false);
            path_row(app, ui, "Иконка setup.exe:", &mut i.setup_icon, &[("ICO", &["ico"])], false);
            text(ui, "Цвет картинки по умолчанию:", &mut i.accent_color, "#1F5DA8");
            text(ui, "Шрифт мастера:", &mut i.font, "системный (например, Candara, Segoe UI)");
            ui.label("Размер шрифта, pt:");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut i.font_size).range(0..=20));
                ui.label(RichText::new("0 = по умолчанию (9)").weak());
            });
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Языки установщика").strong());
        let s = &mut self.project.setup;
        grid(ui, "langdlg", |ui| {
            combo(ui, "Диалог выбора языка:", "ld", &mut s.language_dialog, &[("auto", "если языков больше одного"), ("always", "всегда"), ("never", "никогда")]);
        });
        for (id, name) in smk_core::messages::LANGS {
            let idx = i.languages.iter().position(|l| smk_core::messages::normalize_lang(&l.id) == *id);
            let mut on = idx.is_some();
            ui.horizontal(|ui| {
                if ui.checkbox(&mut on, *name).changed() {
                    if on {
                        i.languages.push(Language { id: id.to_string(), ..Default::default() });
                    } else if let Some(k) = idx {
                        i.languages.remove(k);
                    }
                }
            });
            if let Some(k) = i.languages.iter().position(|l| smk_core::messages::normalize_lang(&l.id) == *id) {
                let l = &mut i.languages[k];
                ui.indent(id, |ui| {
                    grid(ui, &format!("lang{}", id), |ui| {
                        path_row(app, ui, "Лицензия для языка:", &mut l.license_file, &[("RTF/TXT", &["rtf", "txt"])], false);
                        path_row(app, ui, "Инфо до установки:", &mut l.info_before_file, &[("RTF/TXT", &["rtf", "txt"])], false);
                        path_row(app, ui, "Инфо после установки:", &mut l.info_after_file, &[("RTF/TXT", &["rtf", "txt"])], false);
                    });
                });
            }
        }
    }

    fn ui_components(&mut self, ui: &mut Ui) {
        heading(ui, "Компоненты", "Типы установки и дерево компонентов. Иерархия через «\\»: voice, voice\\rus. «Исключающий» = радиокнопка среди соседей.");
        ui.label(RichText::new("Типы установки").strong());
        list_editor(ui, "types", &mut self.project.types, &mut self.sel[3], |t| format!("{} — {}", t.name, t.description), |ui, t| {
            grid(ui, "type", |ui| {
                text(ui, "Имя:", &mut t.name, "full");
                text(ui, "Описание:", &mut t.description, "{cm:FullInstallation}");
                check(ui, "Выборочная (custom)", &mut t.custom);
            });
        });
        ui.separator();
        ui.label(RichText::new("Компоненты").strong());
        let types: Vec<String> = self.project.types.iter().map(|t| t.name.clone()).collect();
        list_editor(ui, "comps", &mut self.project.components, &mut self.sel[4], |c| format!("{}{}", "   ".repeat(c.name.matches('\\').count()), c.description), |ui, c| {
            grid(ui, "comp", |ui| {
                text(ui, "Имя:", &mut c.name, "voice\\rus");
                text(ui, "Описание:", &mut c.description, "Русская озвучка");
                check(ui, "Обязательный (fixed)", &mut c.fixed);
                check(ui, "Исключающий (exclusive)", &mut c.exclusive);
                check(ui, "Отмечен по умолчанию", &mut c.checked);
                let mut mb = c.extra_disk_space / 1048576;
                ui.label("Доп. место, МБ:");
                if ui.add(egui::DragValue::new(&mut mb)).changed() {
                    c.extra_disk_space = mb * 1048576;
                }
                ui.end_row();
                ui.label("Входит в типы:");
                ui.horizontal_wrapped(|ui| {
                    for t in &types {
                        let mut on = c.types.contains(t);
                        if ui.checkbox(&mut on, t).changed() {
                            if on {
                                c.types.push(t.clone());
                            } else {
                                c.types.retain(|x| x != t);
                            }
                        }
                    }
                });
                ui.end_row();
            });
        });
    }

    fn ui_tasks(&mut self, ui: &mut Ui) {
        heading(ui, "Задачи", "Флажки на странице «Дополнительные задачи» (ярлык на рабочем столе, установка .NET и т. п.). Используйте имя задачи в условиях файлов, ярлыков, запуска.");
        list_editor(ui, "tasks", &mut self.project.tasks, &mut self.sel[5], |t| format!("{} — {}", t.name, t.description), |ui, t| {
            grid(ui, "task", |ui| {
                text(ui, "Имя:", &mut t.name, "desktopicon");
                text(ui, "Описание:", &mut t.description, "{cm:CreateDesktopIcon}");
                text(ui, "Группа:", &mut t.group, "{cm:AdditionalIcons}");
                check(ui, "Не отмечена по умолчанию", &mut t.unchecked);
                check(ui, "Исключающая (радиокнопка в группе)", &mut t.exclusive);
                text(ui, "Только для компонентов:", &mut t.components, "");
                text(ui, "Проверка (Check):", &mut t.check, "");
                text(ui, "Мин. версия Windows:", &mut t.min_version, "6.1");
                text(ui, "Только ниже версии:", &mut t.only_below_version, "6.1");
            });
        });
    }

    fn ui_shortcuts(&mut self, ui: &mut Ui) {
        heading(ui, "Ярлыки", "Имя ярлыка — путь без .lnk: {group}\\Программа, {autodesktop}\\Программа. Цель может быть URL — создастся .url");
        list_editor(ui, "lnk", &mut self.project.shortcuts, &mut self.sel[6], |s| s.name.clone(), |ui, s| {
            grid(ui, "lnk", |ui| {
                text(ui, "Ярлык:", &mut s.name, "{group}\\{#AppName}");
                text(ui, "Объект:", &mut s.target, "{app}\\{#AppExeName}");
                text(ui, "Параметры:", &mut s.params, "");
                text(ui, "Рабочая папка:", &mut s.workdir, "");
                text(ui, "Иконка:", &mut s.icon, "{app}\\app.ico");
                ui.label("Индекс иконки:");
                ui.add(egui::DragValue::new(&mut s.icon_index));
                ui.end_row();
                text(ui, "Комментарий:", &mut s.comment, "");
                conditions(ui, &mut s.when);
            });
        });
    }

    fn ui_registry(&mut self, ui: &mut Ui) {
        heading(ui, "Реестр", "Записи реестра. HKA = HKLM для администратора, иначе HKCU.");
        list_editor(ui, "reg", &mut self.project.registry, &mut self.sel[7], |r| format!("{}\\{} [{}]", r.root, r.subkey, r.value_name), |ui, r| {
            grid(ui, "reg", |ui| {
                combo(ui, "Корень:", "root", &mut r.root, &[("HKA", "HKA (авто)"), ("HKLM", "HKLM"), ("HKCU", "HKCU"), ("HKCR", "HKCR"), ("HKU", "HKU")]);
                text(ui, "Раздел:", &mut r.subkey, "Software\\MyCompany\\MyApp");
                combo(ui, "Тип:", "vt", &mut r.value_type, &[("none", "только раздел"), ("string", "строка"), ("expandsz", "расширяемая строка"), ("multisz", "мультистрока"), ("dword", "DWORD"), ("qword", "QWORD"), ("binary", "двоичный")]);
                text(ui, "Имя параметра:", &mut r.value_name, "");
                text(ui, "Значение:", &mut r.value_data, "{app}");
                combo(ui, "При удалении:", "un", &mut r.uninstall, &[("none", "оставить"), ("deletevalue", "удалить параметр"), ("deletekeyifempty", "удалить раздел, если пуст"), ("deletekey", "удалить раздел")]);
                check(ui, "Только если параметра нет", &mut r.only_if_missing);
                check(ui, "Сначала удалить раздел", &mut r.delete_key_first);
                check(ui, "Удалить параметр (вместо записи)", &mut r.delete_value);
                conditions(ui, &mut r.when);
            });
        });
    }

    fn run_form(ui: &mut Ui, r: &mut RunItem, uninstall: bool) {
        grid(ui, "run", |ui| {
            text(ui, "Файл:", &mut r.filename, "{app}\\{#AppExeName}  или  {src}\\bin\\net.exe");
            text(ui, "Параметры:", &mut r.params, "/q");
            text(ui, "Рабочая папка:", &mut r.workdir, "");
            if !uninstall {
                check(ui, "После установки (флажок на последней странице)", &mut r.postinstall);
                text(ui, "Текст флажка:", &mut r.description, "{cm:LaunchProgram,{#AppName}}");
                check(ui, "Флажок снят по умолчанию", &mut r.unchecked);
                text(ui, "Статус во время запуска:", &mut r.status_msg, "Установка .NET...");
                check(ui, "Пропускать при тихой установке", &mut r.skip_if_silent);
            }
            check(ui, "Не ждать завершения", &mut r.nowait);
            check(ui, "Скрытое окно", &mut r.hidden);
            check(ui, "Открыть через оболочку (документы, URL)", &mut r.shell_exec);
            check(ui, "Пропустить, если файла нет", &mut r.skip_if_missing);
            conditions(ui, &mut r.when);
        });
    }

    fn ui_run(&mut self, ui: &mut Ui) {
        heading(ui, "Запуск программ", "[Run]: во время установки (например, установка Python/.NET из {src}\\bin) или флажки «Запустить программу» на последней странице.");
        list_editor(ui, "run", &mut self.project.run, &mut self.sel[8], |r| format!("{}{}", if r.postinstall { "☑ " } else { "▶ " }, r.filename), |ui, r| Self::run_form(ui, r, false));
        ui.separator();
        ui.label(RichText::new("При деинсталляции ([UninstallRun])").strong());
        list_editor(ui, "urun", &mut self.project.uninstall_run, &mut self.sel[9], |r| r.filename.clone(), |ui, r| Self::run_form(ui, r, true));
    }

    fn ui_requirements(&mut self, ui: &mut Ui) {
        heading(ui, "Требования", "");
        let r = &mut self.project.requirements;
        grid(ui, "req", |ui| {
            text(ui, "Минимальная Windows:", &mut r.min_windows, "6.1sp1 (Win7 SP1), 10.0");
            combo(ui, "Разрядность установщика:", "arch", &mut r.arch, &[("auto", "авто (x64; x86 если нужен 32-битный unarc.dll)"), ("x64", "x64 — Program Files"), ("x86", "x86 — Program Files (x86)")]);
            combo(ui, "Права:", "priv", &mut r.privileges, &[("admin", "администратор (UAC)"), ("lowest", "текущий пользователь")]);
            ui.label("Мин. ОЗУ, МБ:");
            ui.add(egui::DragValue::new(&mut r.min_ram_mb).range(0..=1_000_000));
            ui.end_row();
        });
        ui.label(RichText::new("Примечание: стаб собирается современным Rust и требует Windows 10/11 (на Windows 7 не запустится).").small().color(Color32::from_rgb(160, 90, 0)));
    }

    fn ui_uninstall(&mut self, ui: &mut Ui) {
        heading(ui, "Деинсталляция", "Деинсталлятор unins000.exe, запись в «Программы и компоненты», журнал установленных файлов.");
        let app = &*self as *const App;
        let app = unsafe { &*app };
        let u = &mut self.project.uninstall;
        grid(ui, "un", |ui| {
            check(ui, "Создавать деинсталлятор", &mut u.enabled);
            text(ui, "Папка деинсталлятора:", &mut u.files_dir, "{app}");
            text(ui, "Имя в списке программ:", &mut u.display_name, "пусто = «Название Версия»");
            text(ui, "Иконка (путь при установке):", &mut u.display_icon, "{app}\\{#AppExeName}");
            path_row(app, ui, "Иконка из файла .ico:", &mut u.icon_file, &[("ICO", &["ico"])], false);
            check(ui, "Предложить перезагрузку после удаления", &mut u.restart_computer);
        });
        ui.separator();
        ui.label(RichText::new("Удалить перед установкой ([InstallDelete])").strong());
        list_editor(ui, "idel", &mut self.project.install_delete, &mut self.sel[10], |d| d.path.clone(), |ui, d| {
            grid(ui, "idel", |ui| {
                text(ui, "Путь:", &mut d.path, "{app}\\old.dll");
                combo(ui, "Тип:", "k1", &mut d.kind, &[("files", "файлы"), ("filesandordirs", "файлы и папки"), ("dirifempty", "папку, если пуста")]);
            });
        });
        ui.separator();
        ui.label(RichText::new("Удалить при деинсталляции ([UninstallDelete])").strong());
        list_editor(ui, "udel", &mut self.project.uninstall_delete, &mut self.sel[11], |d| d.path.clone(), |ui, d| {
            grid(ui, "udel", |ui| {
                text(ui, "Путь:", &mut d.path, "{app}");
                combo(ui, "Тип:", "k2", &mut d.kind, &[("files", "файлы"), ("filesandordirs", "файлы и папки"), ("dirifempty", "папку, если пуста")]);
            });
        });
    }

    fn ui_compression(&mut self, ui: &mut Ui) {
        heading(ui, "Сжатие и вывод", "");
        let c = &mut self.project.compression;
        grid(ui, "comp", |ui| {
            combo(
                ui,
                "Метод:",
                "meth",
                &mut c.method,
                &[("lzma2", "LZMA2 (xz) — максимальное сжатие"), ("zstd", "Zstandard — очень быстрая распаковка"), ("brotli", "Brotli"), ("none", "без сжатия")],
            );
            let max = match c.method.as_str() {
                "zstd" => 22,
                "brotli" => 11,
                _ => 9,
            };
            ui.label("Уровень:");
            ui.add(egui::Slider::new(&mut c.level, 0..=max));
            ui.end_row();
            if c.method == "lzma2" {
                check(ui, "Extreme (медленнее, чуть лучше)", &mut c.extreme);
                check(ui, "BCJ-фильтр x86 для exe/dll", &mut c.bcj);
            }
            ui.label("Словарь / окно, МБ:");
            ui.add(egui::Slider::new(&mut c.dict_mb, 1..=1024).logarithmic(true));
            ui.end_row();
            ui.label("Потоков (0 = авто):");
            ui.add(egui::DragValue::new(&mut c.threads).range(0..=256));
            ui.end_row();
            check(ui, "Хранить одинаковые файлы один раз", &mut c.dedupe);
        });
        ui.label(
            RichText::new(
                "LZMA2 со словарём 64–256 МБ даёт сжатие уровня FreeArc -m4/-m5 без precomp/srep. Zstd 19–22 с длинным окном сжимает чуть хуже, \
                 но распаковывается в 5–10 раз быстрее. Brotli хорош для текстов/веба, для exe обычно уступает LZMA2. \
                 Память распаковки ≈ размер словаря.",
            )
            .small(),
        );
        ui.separator();
        let o = &mut self.project.output;
        grid(ui, "out", |ui| {
            text(ui, "Папка вывода:", &mut o.dir, "compiled");
            text(ui, "Имя setup-файла:", &mut o.base_name, "{#AppName}_setup");
            check(ui, "Разбивать на тома (setup-1.bin, setup-2.bin …)", &mut o.disk_spanning);
            if o.disk_spanning {
                ui.label("Размер тома, МБ:");
                ui.add(egui::DragValue::new(&mut o.slice_size_mb).range(1..=1_000_000));
                ui.end_row();
            }
        });
    }

    fn ui_build(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        heading(ui, "Сборка", "F9 — собрать. После сборки установщик автоматически проверяется (распаковка + CRC всех файлов).");
        let (running, progress, output, err) = {
            let s = self.build.lock().unwrap();
            (s.running, s.progress, s.output.clone(), s.error.clone())
        };
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new(RichText::new("🔨 Собрать установщик").size(16.0))).clicked() {
                self.start_build();
            }
            if running && ui.button("Отменить").clicked() {
                self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if let Some(o) = &output {
                if ui.button("📂 Открыть папку").clicked() {
                    if let Some(d) = o.parent() {
                        open_folder(d);
                    }
                }
            }
        });
        ui.add(egui::ProgressBar::new(progress).show_percentage().animate(running));
        if let Some(e) = err {
            ui.colored_label(Color32::RED, e);
        } else if let Some(o) = output {
            ui.colored_label(Color32::from_rgb(0, 130, 0), format!("Готово: {}", o.display()));
        }
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        ui.separator();
        let log = self.build.lock().unwrap().log.join("\n");
        egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut log.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(20));
        });
    }
}

fn open_folder(d: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(d).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(d).spawn();
}
