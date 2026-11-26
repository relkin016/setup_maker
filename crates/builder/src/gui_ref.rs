//! "Справочник": every constant and variable usable in project strings.
//! Mirrors `smk-stub/src/consts.rs` (install-time) and
//! `compile::apply_defines` (build-time) — keep in sync.

use eframe::egui::{self, Color32, RichText, Ui};
use smk_core::project::Project;

/// (group title, group hint, [(syntax, description)])
pub const GROUPS: &[(&str, &str, &[(&str, &str)])] = &[
    (
        "Папки установки",
        "Подставляются на компьютере пользователя во время установки.",
        &[
            ("{app}", "Папка программы, выбранная пользователем"),
            ("{group}", "Папка программы в меню «Пуск» (Программы\\<группа>)"),
            ("{src}", "Папка, где лежит setup.exe"),
            ("{srcexe}", "Полный путь к самому setup.exe"),
            ("{tmp}", "Временная папка установщика (удаляется после установки)"),
            ("{uninstallexe}", "Полный путь к деинсталлятору"),
        ],
    ),
    (
        "Program Files",
        "«auto…» — для всех пользователей при установке с правами администратора, иначе для текущего.",
        &[
            ("{pf}", "Program Files (то же, что {commonpf})"),
            ("{commonpf}", "Program Files"),
            ("{autopf}", "Program Files; без админ-прав — AppData\\Local\\Programs"),
            ("{pf32}", "Program Files (x86) — 32-битная"),
            ("{commonpf32}", "Program Files (x86) — 32-битная"),
            ("{pf64}", "Program Files — 64-битная (на 32-битной Windows — обычная)"),
            ("{commonpf64}", "Program Files — 64-битная"),
            ("{cf}", "Program Files\\Common Files"),
            ("{commoncf}", "Program Files\\Common Files"),
            ("{cf32}", "Program Files (x86)\\Common Files"),
        ],
    ),
    (
        "Система",
        "",
        &[
            ("{win}", "Папка Windows (C:\\Windows)"),
            ("{sys}", "Windows\\System32"),
            ("{syswow64}", "Windows\\SysWOW64"),
            ("{sd}", "Системный диск (C:)"),
            ("{fonts}", "Windows\\Fonts"),
            ("{computername}", "Имя компьютера"),
            ("{username}", "Имя текущего пользователя"),
            ("{lang}", "Язык установщика: ru / en / uk"),
            ("{\\}", "Символ «\\»"),
            ("{{", "Символ «{»"),
        ],
    ),
    (
        "Рабочий стол и меню «Пуск»",
        "",
        &[
            ("{autodesktop}", "Рабочий стол (всех или текущего пользователя)"),
            ("{desktop}", "То же, что {autodesktop}"),
            ("{userdesktop}", "Рабочий стол текущего пользователя"),
            ("{commondesktop}", "Общий рабочий стол"),
            ("{autoprograms}", "Меню «Пуск» → Программы"),
            ("{userprograms}", "Меню «Пуск» → Программы (текущий пользователь)"),
            ("{commonprograms}", "Меню «Пуск» → Программы (все пользователи)"),
            ("{userstartmenu}", "Меню «Пуск» (текущий пользователь)"),
            ("{commonstartmenu}", "Меню «Пуск» (все пользователи)"),
            ("{autostartup}", "Автозагрузка"),
            ("{userstartup}", "Автозагрузка (текущий пользователь)"),
            ("{commonstartup}", "Автозагрузка (все пользователи)"),
            ("{quicklaunch}", "Панель быстрого запуска"),
            ("{sendto}", "Папка «Отправить»"),
        ],
    ),
    (
        "Данные пользователя",
        "",
        &[
            ("{userappdata}", "AppData\\Roaming (то же, что {appdata})"),
            ("{appdata}", "AppData\\Roaming"),
            ("{localappdata}", "AppData\\Local"),
            ("{commonappdata}", "ProgramData — общие данные всех пользователей"),
            ("{autoappdata}", "ProgramData"),
            ("{userdocs}", "Документы (то же, что {mydocs})"),
            ("{mydocs}", "Документы"),
            ("{commondocs}", "Общие документы"),
            ("{userfavorites}", "Избранное"),
            ("{usertemplates}", "Шаблоны"),
        ],
    ),
    (
        "Выражения",
        "«|по умолчанию» можно опустить. Внутри можно использовать другие константы.",
        &[
            ("{%NAME|по умолчанию}", "Переменная окружения, напр. {%USERPROFILE}"),
            ("{reg:HKLM\\Ключ,Значение|по умолчанию}", "Строка из реестра (HKLM, HKCU, HKCR, HKU)"),
            ("{param:Имя|по умолчанию}", "Параметр командной строки setup.exe /Имя=значение"),
            ("{cm:Сообщение,арг1,арг2}", "Текст сообщения на языке установки; %1, %2 — аргументы"),
        ],
    ),
    (
        "Переменные сборки {#…}",
        "Подставляются при сборке, работают в любой строке проекта. Свои — в «Информация → Переменные».",
        &[
            ("{#AppName}", "Название программы"),
            ("{#AppVersion}", "Версия"),
            ("{#AppPublisher}", "Издатель"),
            ("{#AppExeName}", "Главный exe программы"),
            ("{#AppUrl}", "Сайт (также {#AppURL})"),
            ("{#VersionType}", "Тип версии"),
            ("{#StringChange(Имя, \"что\", \"на что\")}", "Значение переменной с заменой подстроки"),
        ],
    ),
    (
        "В текстах сообщений",
        "Только в текстах диалогов и сообщений (раздел «Диалоги»).",
        &[
            ("[name]", "Название программы"),
            ("[name/ver]", "Название и версия"),
            ("%1, %2 …", "Аргументы сообщения"),
            ("%n", "Перенос строки"),
        ],
    ),
];

#[derive(Default)]
pub struct RefState {
    pub open: bool,
    filter: String,
    copied: Option<String>,
}

fn matches(f: &str, a: &str, b: &str) -> bool {
    f.is_empty() || a.to_lowercase().contains(f) || b.to_lowercase().contains(f)
}

fn row(ui: &mut Ui, st: &mut RefState, syntax: &str, desc: &str, muted: Color32) {
    let copied = st.copied.as_deref() == Some(syntax);
    let txt = RichText::new(syntax).monospace().color(if copied { Color32::from_rgb(0x1E, 0x8E, 0x3E) } else { Color32::from_rgb(0x2B, 0x6C, 0xD9) });
    let r = ui.add(egui::Label::new(txt).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Нажмите, чтобы скопировать");
    if r.clicked() {
        ui.ctx().copy_text(syntax.to_string());
        st.copied = Some(syntax.to_string());
    }
    ui.label(RichText::new(desc).color(muted));
    ui.end_row();
}

pub fn window(ctx: &egui::Context, st: &mut RefState, p: &Project, text: Color32, muted: Color32) {
    if !st.open {
        return;
    }
    let mut open = st.open;
    egui::Window::new("📖  Справочник переменных")
        .open(&mut open)
        .default_size([640.0, 560.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("🔍");
                ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Поиск: pf, рабочий стол, реестр…").desired_width(f32::INFINITY));
            });
            ui.label(RichText::new("Нажмите на константу, чтобы скопировать её в буфер обмена.").size(12.0).color(muted));
            ui.separator();
            let f = st.filter.trim().to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut any = false;
                for (gi, (title, hint, items)) in GROUPS.iter().enumerate() {
                    let rows: Vec<_> = items.iter().filter(|(a, b)| matches(&f, a, b)).collect();
                    if rows.is_empty() {
                        continue;
                    }
                    any = true;
                    ui.add_space(6.0);
                    ui.label(RichText::new(*title).strong().size(14.5).color(text));
                    if !hint.is_empty() {
                        ui.label(RichText::new(*hint).size(12.0).color(muted));
                    }
                    egui::Grid::new(("ref", gi)).num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                        for (a, b) in rows {
                            row(ui, st, a, b, muted);
                        }
                    });
                }
                // project's own defines
                let own: Vec<(String, String)> = p
                    .defines
                    .iter()
                    .map(|(k, v)| (format!("{{#{}}}", k), if v.is_empty() { "(пусто)".to_string() } else { format!("= {}", v) }))
                    .filter(|(a, b)| matches(&f, a, b))
                    .collect();
                if !own.is_empty() {
                    any = true;
                    ui.add_space(6.0);
                    ui.label(RichText::new("Переменные этого проекта").strong().size(14.5).color(text));
                    egui::Grid::new("ref_own").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                        for (a, b) in &own {
                            row(ui, st, a, b, muted);
                        }
                    });
                }
                // message ids for {cm:...}
                let lang = p.interface.languages.first().map(|l| l.id.as_str()).unwrap_or("ru");
                let msgs: Vec<_> = smk_core::messages::all(lang).iter().filter(|(k, v)| matches(&f, k, v) || f.contains("cm:")).collect();
                if !msgs.is_empty() {
                    any = true;
                    ui.add_space(6.0);
                    let h = egui::CollapsingHeader::new(RichText::new(format!("Сообщения для {{cm:…}} ({})", msgs.len())).strong().size(14.5).color(text))
                        .id_salt("ref_msgs")
                        .default_open(!f.is_empty());
                    h.show(ui, |ui| {
                        egui::Grid::new("ref_msgs_grid").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                            for (k, v) in msgs {
                                let syntax = format!("{{cm:{}}}", k);
                                row(ui, st, &syntax, &v.replace('\n', " "), muted);
                            }
                        });
                    });
                }
                if !any {
                    ui.label(RichText::new("Ничего не найдено").color(muted));
                }
            });
        });
    st.open = open;
}
