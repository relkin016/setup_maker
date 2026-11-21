//! Project model. Stored by the builder as TOML (`*.smp`), embedded into the
//! installer (after resolving build-time paths) as JSON.
//!
//! Strings may contain Inno-style constants (`{app}`, `{pf}`, `{group}`,
//! `{cm:LaunchProgram,App}` ...) which are expanded at install time, and
//! build-time defines `{#Name}` which are expanded by the builder.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_empty(s: &str) -> bool {
    s.is_empty()
}
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}
fn default_app_dir() -> String {
    "{app}".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Project {
    pub app: AppInfo,
    pub setup: SetupOptions,
    pub interface: Interface,
    pub requirements: Requirements,
    pub compression: Compression,
    pub output: Output,
    pub uninstall: Uninstall,
    /// Build-time defines, usable as `{#Name}` in any string.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub defines: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<SetupType>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Task>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileItem>,
    /// External archives next to setup.exe (or packed by the builder).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub archives: Vec<ArchiveItem>,
    /// Helper files extracted to `{tmp}` only (like Inno `dontcopy`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolFile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<DirItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<Shortcut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegistryItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub run: Vec<RunItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uninstall_run: Vec<RunItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub install_delete: Vec<DeleteItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uninstall_delete: Vec<DeleteItem>,
    /// Custom messages: language id ("*" = all) -> name -> text.
    /// Can also override built-in messages (e.g. `WelcomeLabel1`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub messages: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub version_type: String,
    pub publisher: String,
    /// Main executable, relative to `{app}`.
    #[serde(skip_serializing_if = "is_empty")]
    pub exe: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub support_url: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub updates_url: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub contact: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub comments: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub copyright: String,
    /// Unique id (GUID). Used for the uninstall registry key.
    pub app_id: String,
    /// Display name with version ("AppVerName"). Empty = "name version".
    #[serde(skip_serializing_if = "is_empty")]
    pub verb_name: String,
    pub version_info: VersionInfo,
}

impl AppInfo {
    pub fn display_name(&self) -> String {
        if !self.verb_name.is_empty() {
            self.verb_name.clone()
        } else if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version)
        }
    }
}

/// Version resource of setup.exe. Empty fields fall back to app info.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct VersionInfo {
    #[serde(skip_serializing_if = "is_empty")]
    pub version: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub company: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub copyright: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub product_name: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub product_version: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupOptions {
    pub default_dir: String,
    pub default_group: String,
    pub allow_root_directory: bool,
    /// Show "Don't create a Start Menu folder" checkbox.
    pub allow_no_icons: bool,
    pub use_previous_dir: bool,
    pub show_welcome_page: bool,
    pub show_dir_page: bool,
    pub show_group_page: bool,
    pub show_ready_page: bool,
    pub show_finished_page: bool,
    pub always_show_dir_on_ready: bool,
    pub always_show_group_on_ready: bool,
    #[serde(skip_serializing_if = "is_empty")]
    pub license_file: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub info_before_file: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub info_after_file: String,
    /// Installation password (stored hashed inside setup.exe).
    #[serde(skip_serializing_if = "is_empty")]
    pub password: String,
    /// Extra required disk space in bytes (e.g. for external archives).
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub extra_disk_space: u64,
    pub logging: bool,
    pub restart_required: bool,
    /// "auto" (only if >1 language), "always", "never".
    pub language_dialog: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub window_title: String,
    /// Show "installation time" on the finish page.
    pub show_install_time: bool,
    /// Undo everything when installation fails or is cancelled.
    pub rollback_on_error: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self {
            default_dir: "{pf}\\{#AppName}".into(),
            default_group: "{#AppName}".into(),
            allow_root_directory: false,
            allow_no_icons: true,
            use_previous_dir: true,
            show_welcome_page: true,
            show_dir_page: true,
            show_group_page: true,
            show_ready_page: true,
            show_finished_page: true,
            always_show_dir_on_ready: true,
            always_show_group_on_ready: true,
            license_file: String::new(),
            info_before_file: String::new(),
            info_after_file: String::new(),
            password: String::new(),
            extra_disk_space: 0,
            logging: false,
            restart_required: false,
            language_dialog: "auto".into(),
            window_title: String::new(),
            show_install_time: true,
            rollback_on_error: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Interface {
    /// Large image (welcome/finish pages), 164x314 at 100%.
    #[serde(skip_serializing_if = "is_empty")]
    pub wizard_image: String,
    /// Small image (top-right corner of inner pages), 55x55 at 100%.
    #[serde(skip_serializing_if = "is_empty")]
    pub wizard_small_image: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub setup_icon: String,
    /// Background colour for the default gradient image (#RRGGBB).
    #[serde(skip_serializing_if = "is_empty")]
    pub accent_color: String,
    /// Wizard font face (e.g. "Candara"). Empty = system dialog font.
    #[serde(skip_serializing_if = "is_empty")]
    pub font: String,
    /// Wizard font size in points. 0 = system default (9 pt).
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub font_size: u32,
    pub languages: Vec<Language>,
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            wizard_image: String::new(),
            wizard_small_image: String::new(),
            setup_icon: String::new(),
            accent_color: String::new(),
            font: String::new(),
            font_size: 0,
            languages: vec![Language { id: "ru".into(), ..Default::default() }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Language {
    /// "en", "ru", "uk".
    pub id: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub license_file: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub info_before_file: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub info_after_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Requirements {
    /// Minimum Windows version, Inno syntax: "6.1sp1", "10.0", "6.1.7601".
    pub min_windows: String,
    /// "x64", "x86" or "auto" (x64 unless a 32-bit-only plugin is used).
    pub arch: String,
    /// "admin" or "lowest".
    pub privileges: String,
    /// Required RAM in MB (warning only). 0 = no check.
    pub min_ram_mb: u32,
}

impl Default for Requirements {
    fn default() -> Self {
        Self { min_windows: "6.1sp1".into(), arch: "auto".into(), privileges: "admin".into(), min_ram_mb: 0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Compression {
    /// "lzma2" (xz, best ratio), "zstd" (very fast unpacking), "brotli", "none".
    pub method: String,
    /// lzma2: 0-9, zstd: 1-22, brotli: 0-11.
    pub level: u32,
    /// lzma2 "extreme" preset.
    pub extreme: bool,
    /// Dictionary / window size in MB.
    pub dict_mb: u32,
    /// Compression threads, 0 = auto.
    pub threads: u32,
    /// x86 BCJ filter for exe/dll (better ratio for code).
    pub bcj: bool,
    /// Store identical files only once.
    pub dedupe: bool,
}

impl Default for Compression {
    fn default() -> Self {
        Self { method: "lzma2".into(), level: 9, extreme: true, dict_mb: 64, threads: 0, bcj: true, dedupe: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Output {
    pub dir: String,
    pub base_name: String,
    /// Store data in external `<base>-N.bin` slices (DVD/FAT32 friendly).
    pub disk_spanning: bool,
    pub slice_size_mb: u64,
}

impl Default for Output {
    fn default() -> Self {
        Self { dir: "compiled".into(), base_name: "{#AppName}_setup".into(), disk_spanning: false, slice_size_mb: 2000 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Uninstall {
    pub enabled: bool,
    pub files_dir: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub display_name: String,
    /// Runtime path for Add/Remove Programs icon (e.g. `{app}\App.exe`).
    #[serde(skip_serializing_if = "is_empty")]
    pub display_icon: String,
    /// Build-time .ico file installed next to the uninstaller and used as icon.
    #[serde(skip_serializing_if = "is_empty")]
    pub icon_file: String,
    pub restart_computer: bool,
}

impl Default for Uninstall {
    fn default() -> Self {
        Self {
            enabled: true,
            files_dir: "{app}".into(),
            display_name: String::new(),
            display_icon: String::new(),
            icon_file: String::new(),
            restart_computer: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SetupType {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "is_false")]
    pub custom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Component {
    /// Hierarchical name: `voice\rus` is a child of `voice`.
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub fixed: bool,
    /// Radio-button among siblings.
    #[serde(skip_serializing_if = "is_false")]
    pub exclusive: bool,
    /// Checked by default when there are no types.
    #[serde(skip_serializing_if = "is_true")]
    pub checked: bool,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub extra_disk_space: u64,
    /// Show the component only if this condition is true
    /// (e.g. `FileExists('{src}\patch*.zip')`).
    #[serde(skip_serializing_if = "is_empty")]
    pub check: String,
    /// Hide the component when it has no embedded files of its own and none
    /// of its archives (masks like `patch*.zip`, or optional plain names)
    /// are found next to setup.exe.
    #[serde(skip_serializing_if = "is_true")]
    pub hide_if_no_archives: bool,
}

impl Default for Component {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            types: vec![],
            fixed: false,
            exclusive: false,
            checked: true,
            extra_disk_space: 0,
            check: String::new(),
            hide_if_no_archives: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Task {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub group: String,
    #[serde(skip_serializing_if = "is_false")]
    pub unchecked: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub exclusive: bool,
    #[serde(skip_serializing_if = "is_empty")]
    pub components: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub check: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub min_version: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub only_below_version: String,
}

/// Common install-time conditions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[serde(default)]
pub struct Conditions {
    #[serde(skip_serializing_if = "is_empty")]
    pub components: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub tasks: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub check: String,
}

impl Conditions {
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.tasks.is_empty() && self.check.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FileItem {
    /// File, folder, or wildcard (`progFolder\*`), relative to the project file.
    pub source: String,
    pub dest_dir: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub dest_name: String,
    pub recursive: bool,
    pub empty_dirs: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub excludes: Vec<String>,
    /// "always", "ifnewer", "never" (only if it doesn't exist).
    pub overwrite: String,
    /// false = never uninstall (`uninsneveruninstall`).
    pub uninstall: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub skip_if_missing: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub delete_after_install: bool,
    /// "readonly hidden system".
    #[serde(skip_serializing_if = "is_empty")]
    pub attributes: String,
    #[serde(flatten)]
    pub when: Conditions,
}

impl Default for FileItem {
    fn default() -> Self {
        Self {
            source: String::new(),
            dest_dir: default_app_dir(),
            dest_name: String::new(),
            recursive: true,
            empty_dirs: true,
            excludes: vec![],
            overwrite: "always".into(),
            uninstall: true,
            skip_if_missing: false,
            delete_after_install: false,
            attributes: String::new(),
            when: Conditions::default(),
        }
    }
}

/// External archive. In TOML can be written as a plain string:
/// `archives = ["data1.arc", "voice.7z -> {app}\\sound"]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(from = "ArchiveRepr")]
pub struct ArchiveItem {
    /// File name relative to setup.exe folder (`{src}`). Wildcards allowed.
    pub file: String,
    pub dest: String,
    /// "" = auto by extension: smpk, 7z, zip, tar[.gz|.xz|.zst|.br], arc.
    #[serde(skip_serializing_if = "is_empty")]
    pub format: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub password: String,
    /// If set and the archive is missing: ask to insert the disk with this text.
    #[serde(skip_serializing_if = "is_empty")]
    pub disk_prompt: String,
    /// Error if missing (otherwise silently skipped).
    pub required: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub delete_after: bool,
    /// Build-time: pack this folder into `file` (smpk) next to setup.exe.
    #[serde(skip_serializing_if = "is_empty")]
    pub pack_from: String,
    /// Unpacked size estimate (filled by the builder when possible).
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub size_hint: u64,
    #[serde(flatten)]
    pub when: Conditions,
}

impl Default for ArchiveItem {
    fn default() -> Self {
        Self {
            file: String::new(),
            dest: default_app_dir(),
            format: String::new(),
            password: String::new(),
            disk_prompt: String::new(),
            required: true,
            delete_after: false,
            pack_from: String::new(),
            size_hint: 0,
            when: Conditions::default(),
        }
    }
}

impl ArchiveItem {
    /// Parse the short form: `name` or `name -> dest`.
    pub fn from_short(s: &str) -> Self {
        let (file, dest) = match s.split_once("->") {
            Some((f, d)) => (f.trim().to_string(), d.trim().to_string()),
            None => (s.trim().to_string(), default_app_dir()),
        };
        ArchiveItem { file, dest, ..Default::default() }
    }

    pub fn detect_format(&self) -> String {
        if !self.format.is_empty() {
            return self.format.to_ascii_lowercase();
        }
        archive_format_by_name(&self.file).to_string()
    }
}

pub fn archive_format_by_name(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".smpk") {
        "smpk"
    } else if n.ends_with(".7z") || n.contains(".7z.") {
        "7z"
    } else if n.ends_with(".zip") {
        "zip"
    } else if n.ends_with(".tar") {
        "tar"
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        "tar.gz"
    } else if n.ends_with(".tar.xz") || n.ends_with(".txz") {
        "tar.xz"
    } else if n.ends_with(".tar.zst") || n.ends_with(".tzst") {
        "tar.zst"
    } else if n.ends_with(".tar.br") {
        "tar.br"
    } else if n.ends_with(".arc") || n.ends_with(".bin") {
        "arc"
    } else {
        "smpk"
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ArchiveRepr {
    Short(String),
    Full(ArchiveFull),
}

#[derive(Deserialize)]
#[serde(default)]
struct ArchiveFull {
    file: String,
    dest: String,
    format: String,
    password: String,
    disk_prompt: String,
    required: bool,
    delete_after: bool,
    pack_from: String,
    size_hint: u64,
    #[serde(flatten)]
    when: Conditions,
}

impl Default for ArchiveFull {
    fn default() -> Self {
        let d = ArchiveItem::default();
        Self {
            file: d.file,
            dest: d.dest,
            format: d.format,
            password: d.password,
            disk_prompt: d.disk_prompt,
            required: d.required,
            delete_after: d.delete_after,
            pack_from: d.pack_from,
            size_hint: d.size_hint,
            when: d.when,
        }
    }
}

impl From<ArchiveRepr> for ArchiveItem {
    fn from(r: ArchiveRepr) -> Self {
        match r {
            ArchiveRepr::Short(s) => ArchiveItem::from_short(&s),
            ArchiveRepr::Full(f) => ArchiveItem {
                file: f.file,
                dest: f.dest,
                format: f.format,
                password: f.password,
                disk_prompt: f.disk_prompt,
                required: f.required,
                delete_after: f.delete_after,
                pack_from: f.pack_from,
                size_hint: f.size_hint,
                when: f.when,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ToolFile {
    pub source: String,
    /// Name inside `{tmp}`. Empty = source file name.
    #[serde(skip_serializing_if = "is_empty")]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DirItem {
    pub name: String,
    pub uninstall: bool,
    #[serde(flatten)]
    pub when: Conditions,
}

impl Default for DirItem {
    fn default() -> Self {
        Self { name: String::new(), uninstall: true, when: Conditions::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Shortcut {
    /// Shortcut path without `.lnk`: `{group}\App`, `{commondesktop}\App`.
    pub name: String,
    /// Target file or URL.
    pub target: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub params: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub workdir: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub icon: String,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub icon_index: i32,
    #[serde(skip_serializing_if = "is_empty")]
    pub comment: String,
    /// Ask the user with a checkbox in the installer:
    /// "" = always create, "checked" / "unchecked" = checkbox state by default.
    /// The builder turns it into a task.
    #[serde(skip_serializing_if = "is_empty")]
    pub ask: String,
    #[serde(flatten)]
    pub when: Conditions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RegistryItem {
    /// HKLM, HKCU, HKCR, HKU, HKA (HKLM when admin, else HKCU).
    pub root: String,
    pub subkey: String,
    /// none, string, expandsz, multisz, dword, qword, binary.
    pub value_type: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub value_name: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub value_data: String,
    /// On uninstall: "none", "deletekey", "deletekeyifempty", "deletevalue".
    pub uninstall: String,
    #[serde(skip_serializing_if = "is_false")]
    pub only_if_missing: bool,
    /// Delete the key before writing (Inno `deletekey`).
    #[serde(skip_serializing_if = "is_false")]
    pub delete_key_first: bool,
    /// Delete the value instead of writing (Inno `deletevalue`).
    #[serde(skip_serializing_if = "is_false")]
    pub delete_value: bool,
    #[serde(flatten)]
    pub when: Conditions,
}

impl Default for RegistryItem {
    fn default() -> Self {
        Self {
            root: "HKA".into(),
            subkey: String::new(),
            value_type: "string".into(),
            value_name: String::new(),
            value_data: String::new(),
            uninstall: "deletevalue".into(),
            only_if_missing: false,
            delete_key_first: false,
            delete_value: false,
            when: Conditions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct RunItem {
    pub filename: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub params: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub workdir: String,
    /// Checkbox text on the finish page (postinstall).
    #[serde(skip_serializing_if = "is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "is_empty")]
    pub status_msg: String,
    /// Run from the finish page (checkbox), otherwise during installation.
    #[serde(skip_serializing_if = "is_false")]
    pub postinstall: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub unchecked: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub nowait: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Open via ShellExecute (documents, URLs).
    #[serde(skip_serializing_if = "is_false")]
    pub shell_exec: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub skip_if_silent: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub skip_if_missing: bool,
    #[serde(flatten)]
    pub when: Conditions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeleteItem {
    pub path: String,
    /// "files", "filesandordirs", "dirifempty".
    pub kind: String,
}

impl Default for DeleteItem {
    fn default() -> Self {
        Self { path: String::new(), kind: "filesandordirs".into() }
    }
}

/// `{group}\\App` -> (`{group}`, `App`). No leading constant -> ("", whole).
pub fn split_location(path: &str) -> (&str, &str) {
    if path.starts_with('{') {
        if let Some(end) = path.find('}') {
            let rest = &path[end + 1..];
            return (&path[..=end], rest.trim_start_matches('\\'));
        }
    }
    ("", path)
}

pub fn is_desktop(loc: &str) -> bool {
    matches!(loc, "{autodesktop}" | "{desktop}" | "{commondesktop}" | "{userdesktop}")
}

impl Project {
    /// A sensible new project: Start menu + desktop shortcut (asked), launch checkbox.
    pub fn template(name: &str, exe: &str, files_dir: &str) -> Project {
        let mut p = Project::default();
        p.app.name = name.to_string();
        p.app.version = "1.0".into();
        p.app.version_type = "stable".into();
        p.app.exe = exe.to_string();
        p.app.app_id = String::new();
        p.defines.insert("AppName".into(), name.to_string());
        if !files_dir.is_empty() {
            p.files.push(FileItem { source: format!("{}\\*", files_dir), ..Default::default() });
        }
        if !exe.is_empty() {
            p.shortcuts.push(Shortcut {
                name: "{group}\\{#AppName}".into(),
                target: format!("{{app}}\\{}", exe),
                ..Default::default()
            });
            p.shortcuts.push(Shortcut {
                name: "{autodesktop}\\{#AppName}".into(),
                target: format!("{{app}}\\{}", exe),
                ask: "checked".into(),
                ..Default::default()
            });
            p.run.push(RunItem {
                filename: format!("{{app}}\\{}", exe),
                description: "{cm:LaunchProgram,{#AppName}}".into(),
                postinstall: true,
                nowait: true,
                skip_if_silent: true,
                ..Default::default()
            });
        }
        p.shortcuts.push(Shortcut {
            name: "{group}\\{cm:UninstallProgram,{#AppName}}".into(),
            target: "{uninstallexe}".into(),
            ..Default::default()
        });
        p.uninstall_delete.push(DeleteItem { path: "{app}".into(), kind: "filesandordirs".into() });
        p
    }

    /// Builder step: shortcuts with `ask` get a checkbox (task) in the installer.
    /// Shortcuts of the same kind and default state share one checkbox.
    pub fn expand_shortcut_asks(&mut self) {
        let mut made: Vec<(String, String)> = Vec::new(); // (description|ask, task name)
        for i in 0..self.shortcuts.len() {
            let ask = std::mem::take(&mut self.shortcuts[i].ask);
            if ask.is_empty() {
                continue;
            }
            let (loc, leaf) = split_location(&self.shortcuts[i].name);
            let desc = match loc {
                l if is_desktop(l) => "{cm:CreateDesktopIcon}".to_string(),
                "{quicklaunch}" => "{cm:CreateQuickLaunchIcon}".to_string(),
                _ => format!("{{cm:CreateShortcutIcon,{}}}", leaf.replace(',', " ")),
            };
            let key = format!("{}|{}", desc, ask);
            let task = match made.iter().find(|m| m.0 == key) {
                Some(m) => m.1.clone(),
                None => {
                    let mut n = made.len() + 1;
                    while self.tasks.iter().any(|t| t.name.eq_ignore_ascii_case(&format!("shortcut{}", n))) {
                        n += 1;
                    }
                    let name = format!("shortcut{}", n);
                    self.tasks.push(Task {
                        name: name.clone(),
                        description: desc,
                        group: "{cm:AdditionalIcons}".into(),
                        unchecked: ask == "unchecked",
                        ..Default::default()
                    });
                    made.push((key, name.clone()));
                    name
                }
            };
            let t = &mut self.shortcuts[i].when.tasks;
            *t = if t.trim().is_empty() { task } else { format!("({}) and {}", t, task) };
        }
    }

    /// Reverse of [`expand_shortcut_asks`]: a plain task used only as the
    /// condition of shortcuts becomes the shortcuts' `ask` flag (used after import).
    pub fn simplify_shortcut_tasks(&mut self) {
        let mut i = 0;
        while i < self.tasks.len() {
            let t = &self.tasks[i];
            let name = t.name.clone();
            let plain = !t.exclusive && t.components.is_empty() && t.check.is_empty() && t.min_version.is_empty() && t.only_below_version.is_empty();
            let used_by_sc = self.shortcuts.iter().any(|s| s.when.tasks.trim().eq_ignore_ascii_case(&name));
            if !plain || name.is_empty() || !used_by_sc || self.task_used_outside_shortcuts(&name) {
                i += 1;
                continue;
            }
            let ask = if t.unchecked { "unchecked" } else { "checked" };
            for s in self.shortcuts.iter_mut().filter(|s| s.when.tasks.trim().eq_ignore_ascii_case(&name)) {
                s.when.tasks.clear();
                s.ask = ask.into();
            }
            self.tasks.remove(i);
        }
    }

    fn task_used_outside_shortcuts(&self, name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        let hit = |e: &str| {
            let e = e.to_ascii_lowercase();
            e.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\\')).any(|w| w == n)
        };
        let sc_complex = self.shortcuts.iter().any(|s| !s.when.tasks.trim().eq_ignore_ascii_case(name) && hit(&s.when.tasks));
        sc_complex
            || self.files.iter().any(|x| hit(&x.when.tasks))
            || self.archives.iter().any(|x| hit(&x.when.tasks))
            || self.dirs.iter().any(|x| hit(&x.when.tasks))
            || self.registry.iter().any(|x| hit(&x.when.tasks))
            || self.run.iter().any(|x| hit(&x.when.tasks))
            || self.uninstall_run.iter().any(|x| hit(&x.when.tasks))
    }

    /// Language ids used by the installer (at least one).
    pub fn language_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.interface.languages.iter().map(|l| crate::messages::normalize_lang(&l.id).to_string()).collect();
        v.dedup();
        if v.is_empty() {
            v.push("en".into());
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_ask_roundtrip() {
        let mut p = Project::template("App", "app.exe", "");
        p.expand_shortcut_asks();
        assert_eq!(p.tasks.len(), 1);
        assert_eq!(p.tasks[0].description, "{cm:CreateDesktopIcon}");
        let desk = p.shortcuts.iter().find(|s| s.name.starts_with("{autodesktop}")).unwrap();
        assert_eq!(desk.when.tasks, p.tasks[0].name);
        assert!(desk.ask.is_empty());
        p.simplify_shortcut_tasks();
        assert!(p.tasks.is_empty());
        assert_eq!(p.shortcuts.iter().find(|s| s.name.starts_with("{autodesktop}")).unwrap().ask, "checked");
    }

    #[test]
    fn task_used_by_files_is_kept() {
        let mut p = Project::template("App", "app.exe", "");
        p.expand_shortcut_asks();
        let t = p.tasks[0].name.clone();
        p.files.push(FileItem { source: "x".into(), when: Conditions { tasks: format!("not {}", t), ..Default::default() }, ..Default::default() });
        p.simplify_shortcut_tasks();
        assert_eq!(p.tasks.len(), 1);
    }

    #[test]
    fn location_split() {
        assert_eq!(split_location("{group}\\App"), ("{group}", "App"));
        assert_eq!(split_location("C:\\x"), ("", "C:\\x"));
    }
}
