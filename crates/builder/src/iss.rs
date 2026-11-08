//! Import Inno Setup scripts (.iss) into a Setup Maker project.
//!
//! Supported: ISPP `#define` / `#ifdef` / `#ifndef` / `#else` / `#endif`,
//! `{#Name}` substitution, [Setup], [Languages], [Tasks], [Types],
//! [Components], [Files], [Dirs], [Icons], [Registry], [Run], [UninstallRun],
//! [InstallDelete], [UninstallDelete], [CustomMessages], [Messages].
//! In [Code], active (not commented) ISDone calls `ISArcExtract`,
//! `IS7ZipExtract`, `ISRarExtract` and `ShowChangeDiskWindow` become
//! external archives.

use crate::compile::subst;
use smk_core::project::*;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

pub struct Imported {
    pub project: Project,
    pub warnings: Vec<String>,
}

fn decode(data: &[u8]) -> String {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1251.decode(data).0.into_owned(),
    }
}

/// Parse `Key: value; Key: "quoted; value"` entries.
fn parse_entry(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let c: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < c.len() {
        while i < c.len() && (c[i].is_whitespace() || c[i] == ';') {
            i += 1;
        }
        let ks = i;
        while i < c.len() && c[i] != ':' {
            i += 1;
        }
        if i >= c.len() {
            break;
        }
        let key: String = c[ks..i].iter().collect::<String>().trim().to_ascii_lowercase();
        i += 1;
        while i < c.len() && c[i] == ' ' {
            i += 1;
        }
        let mut val = String::new();
        if i < c.len() && c[i] == '"' {
            i += 1;
            while i < c.len() {
                if c[i] == '"' {
                    if i + 1 < c.len() && c[i + 1] == '"' {
                        val.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                val.push(c[i]);
                i += 1;
            }
            while i < c.len() && c[i] != ';' {
                i += 1;
            }
        } else {
            while i < c.len() && c[i] != ';' {
                val.push(c[i]);
                i += 1;
            }
        }
        out.insert(key, val.trim().to_string());
    }
    out
}

fn flags(e: &BTreeMap<String, String>) -> HashSet<String> {
    e.get("flags").map(|f| f.split_whitespace().map(|s| s.to_ascii_lowercase()).collect()).unwrap_or_default()
}

fn when(e: &BTreeMap<String, String>) -> Conditions {
    Conditions {
        components: e.get("components").cloned().unwrap_or_default(),
        tasks: e.get("tasks").cloned().unwrap_or_default(),
        check: e.get("check").cloned().unwrap_or_default(),
    }
}

fn yes(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1")
}

fn lang_of_isl(f: &str) -> &'static str {
    let l = f.to_ascii_lowercase();
    if l.contains("russian") {
        "ru"
    } else if l.contains("ukrainian") {
        "uk"
    } else {
        "en"
    }
}

/// Windows path from the script -> path usable on this machine.
fn fix_path(p: &str) -> String {
    p.trim().trim_matches('"').to_string()
}

pub fn normalize_guid(s: &str) -> String {
    let core = s.trim().trim_matches(|c| c == '{' || c == '}');
    if core.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", core)
    }
}

/// Inno `{{` escapes -> plain text for fields that are not expanded (AppId).
fn unescape_braces(s: &str) -> String {
    s.replace("{{", "{")
}

pub fn import(path: &Path, overrides: &BTreeMap<String, String>) -> Result<Imported, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    import_str(&decode(&data), overrides)
}

pub fn import_str(text: &str, overrides: &BTreeMap<String, String>) -> Result<Imported, String> {
    let mut warn = Vec::new();
    let mut defs: BTreeMap<String, String> = BTreeMap::new();
    // ---- preprocessor
    let mut lines_out: Vec<String> = Vec::new();
    let mut stack: Vec<(bool, bool)> = Vec::new(); // (active, parent_active)
    let active = |st: &Vec<(bool, bool)>| st.iter().all(|s| s.0);
    for raw in text.lines() {
        let t = raw.trim();
        if let Some(d) = t.strip_prefix('#') {
            let d = d.trim();
            let lower = d.to_ascii_lowercase();
            if lower.starts_with("ifdef") || lower.starts_with("ifndef") {
                let name = d.split_whitespace().nth(1).unwrap_or("");
                let isdef = defs.contains_key(name) || overrides.contains_key(name);
                let cond = if lower.starts_with("ifdef") { isdef } else { !isdef };
                stack.push((cond, active(&stack)));
            } else if lower.starts_with("else") {
                if let Some(top) = stack.last_mut() {
                    top.0 = !top.0;
                }
            } else if lower.starts_with("endif") {
                stack.pop();
            } else if lower.starts_with("define") && active(&stack) {
                let rest = d[6..].trim();
                let (name, val) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let mut v = val.trim().to_string();
                if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
                    v = v[1..v.len() - 1].to_string();
                }
                defs.insert(name.to_string(), v);
            }
            continue;
        }
        if active(&stack) {
            lines_out.push(raw.to_string());
        }
    }
    for (k, v) in overrides {
        defs.insert(k.clone(), v.clone());
    }
    // path-like defines stay as {#Name} in the project (editable in one place)
    let is_path_def = |k: &str| {
        let l = k.to_ascii_lowercase();
        l.contains("dir") || l.contains("path")
    };
    let full_defs = defs.clone();
    let defs: BTreeMap<String, String> = defs.into_iter().filter(|(k, _)| !is_path_def(k)).collect();

    let mut p = Project::default();
    p.interface.languages.clear();
    p.setup.default_dir = String::new();
    p.output.base_name = "setup".into();
    p.output.dir = ".".into();
    let mut section = String::new();
    let mut code = String::new();
    let mut in_block_comment = false;

    for raw in &lines_out {
        let line = raw.trim();
        if section == "code" {
            if line.starts_with('[') && line.ends_with(']') && !line.contains("(*") {
                section = line[1..line.len() - 1].to_ascii_lowercase();
                continue;
            }
            code.push_str(raw);
            code.push('\n');
            continue;
        }
        if in_block_comment {
            if line.contains("*)") || line.contains('}') {
                in_block_comment = false;
            }
            continue;
        }
        if line.is_empty() || line.starts_with(';') || line.starts_with("//") {
            continue;
        }
        if line.starts_with("(*") {
            in_block_comment = !line.contains("*)");
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        let line = subst(line, &defs);
        match section.as_str() {
            "setup" => {
                let Some((k, v)) = line.split_once('=') else { continue };
                let k = k.trim().to_ascii_lowercase();
                let v = v.trim().trim_matches('"').to_string();
                setup_key(&mut p, &k, &v, &mut warn);
            }
            "languages" => {
                let e = parse_entry(&line);
                let id = lang_of_isl(e.get("messagesfile").map(|s| s.as_str()).unwrap_or(""));
                p.interface.languages.push(Language {
                    id: id.into(),
                    license_file: e.get("licensefile").map(|s| fix_path(s)).unwrap_or_default(),
                    info_before_file: e.get("infobeforefile").map(|s| fix_path(s)).unwrap_or_default(),
                    info_after_file: e.get("infoafterfile").map(|s| fix_path(s)).unwrap_or_default(),
                });
            }
            "tasks" => {
                let e = parse_entry(&line);
                let f = flags(&e);
                p.tasks.push(Task {
                    name: e.get("name").cloned().unwrap_or_default(),
                    description: e.get("description").cloned().unwrap_or_default(),
                    group: e.get("groupdescription").cloned().unwrap_or_default(),
                    unchecked: f.contains("unchecked"),
                    exclusive: f.contains("exclusive"),
                    components: e.get("components").cloned().unwrap_or_default(),
                    check: e.get("check").cloned().unwrap_or_default(),
                    min_version: e.get("minversion").cloned().unwrap_or_default(),
                    only_below_version: e.get("onlybelowversion").cloned().unwrap_or_default(),
                });
            }
            "types" => {
                let e = parse_entry(&line);
                p.types.push(SetupType {
                    name: e.get("name").cloned().unwrap_or_default(),
                    description: e.get("description").cloned().unwrap_or_default(),
                    custom: flags(&e).contains("iscustom"),
                });
            }
            "components" => {
                let e = parse_entry(&line);
                let f = flags(&e);
                p.components.push(Component {
                    name: e.get("name").cloned().unwrap_or_default(),
                    description: e.get("description").cloned().unwrap_or_default(),
                    types: e.get("types").map(|t| t.split_whitespace().map(String::from).collect()).unwrap_or_default(),
                    fixed: f.contains("fixed"),
                    exclusive: f.contains("exclusive"),
                    checked: true,
                    extra_disk_space: e.get("extradiskspacerequired").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
            "files" => {
                let e = parse_entry(&line);
                let f = flags(&e);
                let source = fix_path(e.get("source").map(|s| s.as_str()).unwrap_or(""));
                let dest_dir = e.get("destdir").cloned().unwrap_or_else(|| "{app}".into());
                if f.contains("dontcopy") || dest_dir.eq_ignore_ascii_case("{tmp}") {
                    let lower = source.to_ascii_lowercase();
                    if lower.contains("isskin") || lower.ends_with(".cjstyles") {
                        warn.push(format!("Скин ISSkin ({}) не поддерживается — мастер имеет стандартный вид Inno", source));
                        continue;
                    }
                    if lower.ends_with("isdone.dll") || lower.ends_with("innocallback.dll") {
                        continue; // replaced by the built-in archive engine
                    }
                    p.tools.push(ToolFile { source, name: e.get("destname").cloned().unwrap_or_default() });
                    continue;
                }
                let item = FileItem {
                    source: source.clone(),
                    dest_dir: dest_dir.clone(),
                    dest_name: e.get("destname").cloned().unwrap_or_default(),
                    recursive: f.contains("recursesubdirs"),
                    empty_dirs: f.contains("createallsubdirs"),
                    excludes: e.get("excludes").map(|x| x.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
                    overwrite: if f.contains("onlyifdoesntexist") {
                        "never".into()
                    } else if f.contains("ignoreversion") {
                        "always".into()
                    } else {
                        "ifnewer".into()
                    },
                    uninstall: !f.contains("uninsneveruninstall"),
                    skip_if_missing: f.contains("skipifsourcedoesntexist"),
                    delete_after_install: f.contains("deleteafterinstall"),
                    attributes: e.get("attribs").cloned().unwrap_or_default(),
                    when: when(&e),
                };
                if f.contains("isreadme") {
                    let name = Path::new(&source.replace('\\', "/")).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    p.run.push(RunItem {
                        filename: format!("{}\\{}", dest_dir.trim_end_matches('\\'), name),
                        description: "{cm:ViewReadme}".into(),
                        postinstall: true,
                        shell_exec: true,
                        nowait: true,
                        skip_if_silent: true,
                        unchecked: true,
                        ..Default::default()
                    });
                }
                p.files.push(item);
            }
            "dirs" => {
                let e = parse_entry(&line);
                p.dirs.push(DirItem { name: e.get("name").cloned().unwrap_or_default(), uninstall: !flags(&e).contains("uninsneveruninstall"), when: when(&e) });
            }
            "icons" => {
                let e = parse_entry(&line);
                p.shortcuts.push(Shortcut {
                    name: e.get("name").cloned().unwrap_or_default(),
                    target: e.get("filename").cloned().unwrap_or_default(),
                    params: e.get("parameters").cloned().unwrap_or_default(),
                    workdir: e.get("workingdir").cloned().unwrap_or_default(),
                    icon: e.get("iconfilename").cloned().unwrap_or_default(),
                    icon_index: e.get("iconindex").and_then(|v| v.parse().ok()).unwrap_or(0),
                    comment: e.get("comment").cloned().unwrap_or_default(),
                    when: when(&e),
                });
            }
            "registry" => {
                let e = parse_entry(&line);
                let f = flags(&e);
                p.registry.push(RegistryItem {
                    root: e.get("root").cloned().unwrap_or_else(|| "HKA".into()),
                    subkey: e.get("subkey").cloned().unwrap_or_default(),
                    value_type: e.get("valuetype").map(|v| v.to_ascii_lowercase()).unwrap_or_else(|| "none".into()),
                    value_name: e.get("valuename").cloned().unwrap_or_default(),
                    value_data: e.get("valuedata").cloned().unwrap_or_default(),
                    uninstall: if f.contains("uninsdeletekey") {
                        "deletekey".into()
                    } else if f.contains("uninsdeletekeyifempty") {
                        "deletekeyifempty".into()
                    } else if f.contains("uninsdeletevalue") {
                        "deletevalue".into()
                    } else {
                        "none".into()
                    },
                    only_if_missing: f.contains("createvalueifdoesntexist"),
                    delete_key_first: f.contains("deletekey"),
                    delete_value: f.contains("deletevalue"),
                    when: when(&e),
                });
            }
            "run" | "uninstallrun" => {
                let e = parse_entry(&line);
                let f = flags(&e);
                let r = RunItem {
                    filename: e.get("filename").cloned().unwrap_or_default(),
                    params: e.get("parameters").cloned().unwrap_or_default(),
                    workdir: e.get("workingdir").cloned().unwrap_or_default(),
                    description: e.get("description").cloned().unwrap_or_default(),
                    status_msg: e.get("statusmsg").cloned().unwrap_or_default(),
                    postinstall: f.contains("postinstall"),
                    unchecked: f.contains("unchecked"),
                    nowait: f.contains("nowait"),
                    hidden: f.contains("runhidden"),
                    shell_exec: f.contains("shellexec"),
                    skip_if_silent: f.contains("skipifsilent"),
                    skip_if_missing: f.contains("skipifdoesntexist"),
                    when: when(&e),
                };
                if section == "run" {
                    p.run.push(r)
                } else {
                    p.uninstall_run.push(r)
                }
            }
            "installdelete" | "uninstalldelete" => {
                let e = parse_entry(&line);
                let d = DeleteItem {
                    path: e.get("name").cloned().unwrap_or_default(),
                    kind: e.get("type").map(|t| t.to_ascii_lowercase()).unwrap_or_else(|| "files".into()),
                };
                if section == "installdelete" {
                    p.install_delete.push(d)
                } else {
                    p.uninstall_delete.push(d)
                }
            }
            "custommessages" | "messages" => {
                let Some((k, v)) = line.split_once('=') else { continue };
                let (lang, name) = match k.split_once('.') {
                    Some((l, n)) => (smk_core::messages::normalize_lang(l).to_string(), n.trim().to_string()),
                    None => ("*".to_string(), k.trim().to_string()),
                };
                p.messages.entry(lang).or_default().insert(name, v.trim().to_string());
            }
            "isformdesigner" => {}
            other => {
                if !other.is_empty() {
                    warn.push(format!("Секция [{}] пропущена", other));
                    section = String::new();
                }
            }
        }
    }

    if !code.is_empty() {
        import_code(&code, &defs, &mut p, &mut warn);
    }

    // defaults / cleanup
    if p.interface.languages.is_empty() {
        p.interface.languages.push(Language { id: "en".into(), ..Default::default() });
    }
    if !p.setup.license_file.is_empty() {
        for l in &mut p.interface.languages {
            if l.license_file == p.setup.license_file {
                l.license_file.clear();
            }
        }
    }
    if p.setup.default_dir.is_empty() {
        p.setup.default_dir = format!("{{pf}}\\{}", p.app.name);
    }
    if p.app.exe.is_empty() {
        if let Some(n) = defs.iter().find(|(k, _)| k.to_ascii_lowercase().contains("exename")).map(|(_, v)| v.clone()) {
            p.app.exe = n;
        }
    }
    // keep user defines that are not plain app info (e.g. InnoDir) for reference
    for (k, v) in &full_defs {
        if is_path_def(k) {
            p.defines.insert(k.clone(), v.clone());
        }
    }
    Ok(Imported { project: p, warnings: warn })
}

fn setup_key(p: &mut Project, k: &str, v: &str, warn: &mut Vec<String>) {
    let s = v.to_string();
    match k {
        "appname" => p.app.name = s,
        "appvername" => p.app.verb_name = s,
        "appversion" => p.app.version = s,
        "apppublisher" => p.app.publisher = s,
        "apppublisherurl" => p.app.url = s,
        "appsupporturl" => p.app.support_url = s,
        "appupdatesurl" => p.app.updates_url = s,
        "appcontact" => p.app.contact = s,
        "appcomments" => p.app.comments = s,
        "appcopyright" => p.app.copyright = s,
        "appid" => p.app.app_id = normalize_guid(&unescape_braces(&s)),
        "defaultdirname" => p.setup.default_dir = s,
        "defaultgroupname" => p.setup.default_group = s,
        "outputdir" => p.output.dir = fix_path(&s),
        "outputbasefilename" => p.output.base_name = s,
        "setupiconfile" => p.interface.setup_icon = fix_path(&s),
        "wizardimagefile" => p.interface.wizard_image = fix_path(&s),
        "wizardsmallimagefile" => p.interface.wizard_small_image = fix_path(&s),
        "licensefile" => p.setup.license_file = fix_path(&s),
        "infobeforefile" => p.setup.info_before_file = fix_path(&s),
        "infoafterfile" => p.setup.info_after_file = fix_path(&s),
        "password" => p.setup.password = s,
        "compression" => {
            let l = s.to_ascii_lowercase();
            if l.starts_with("lzma") {
                p.compression.method = "lzma2".into();
                p.compression.level = if l.contains("ultra") || l.contains("max") { 9 } else if l.contains("fast") { 3 } else { 6 };
                p.compression.extreme = l.contains("ultra");
                if l.contains("ultra64") {
                    p.compression.dict_mb = 64;
                }
            } else if l.starts_with("zip") || l.starts_with("bzip") {
                p.compression.method = "zstd".into();
                p.compression.level = 19;
            } else if l == "none" {
                p.compression.method = "none".into();
            }
        }
        "lzmanumblockthreads" => p.compression.threads = s.parse().unwrap_or(0),
        "lzmadictionarysize" => p.compression.dict_mb = (s.parse::<u32>().unwrap_or(65536) / 1024).max(1),
        "solidcompression" | "internalcompresslevel" | "lzmauseseparateprocess" | "lzmanumfastbytes" | "lzmaalgorithm" | "lzmamatchfinder" => {}
        "diskspanning" => p.output.disk_spanning = yes(&s),
        "diskslicesize" => {
            let b: u64 = s.parse().unwrap_or(2_100_000_000);
            p.output.slice_size_mb = (b / (1024 * 1024)).max(1);
        }
        "slicesperdisk" | "diskclustersize" | "reservebytes" => {}
        "allowrootdirectory" => p.setup.allow_root_directory = yes(&s),
        "alwaysshowdironreadypage" => p.setup.always_show_dir_on_ready = yes(&s),
        "alwaysshowgrouponreadypage" => p.setup.always_show_group_on_ready = yes(&s),
        "disablewelcomepage" => p.setup.show_welcome_page = !yes(&s),
        "disabledirpage" => p.setup.show_dir_page = !(yes(&s) || s.eq_ignore_ascii_case("auto") && false),
        "disableprogramgrouppage" => p.setup.show_group_page = !yes(&s),
        "disablereadypage" => p.setup.show_ready_page = !yes(&s),
        "disablefinishedpage" => p.setup.show_finished_page = !yes(&s),
        "allownoicons" => p.setup.allow_no_icons = yes(&s),
        "usepreviousappdir" => p.setup.use_previous_dir = yes(&s),
        "minversion" => p.requirements.min_windows = s,
        "setuplogging" => p.setup.logging = yes(&s),
        "extradiskspacerequired" => p.setup.extra_disk_space = s.parse().unwrap_or(0),
        "uninstallfilesdir" => p.uninstall.files_dir = s,
        "uninstallrestartcomputer" => p.uninstall.restart_computer = yes(&s),
        "uninstalldisplayicon" => p.uninstall.display_icon = s,
        "uninstalldisplayname" => p.uninstall.display_name = s,
        "uninstallable" | "createuninstallregkey" => p.uninstall.enabled = yes(&s),
        "updateuninstalllogappname" => {}
        "privilegesrequired" => p.requirements.privileges = if s.eq_ignore_ascii_case("lowest") { "lowest".into() } else { "admin".into() },
        "architecturesinstallin64bitmode" => {
            if s.to_ascii_lowercase().contains("x64") {
                p.requirements.arch = "x64".into()
            }
        }
        "architecturesallowed" => {
            if s.to_ascii_lowercase().trim() == "x86" {
                p.requirements.arch = "x86".into()
            }
        }
        "showlanguagedialog" => {
            p.setup.language_dialog = match s.to_ascii_lowercase().as_str() {
                "yes" => "always".into(),
                "no" => "never".into(),
                _ => "auto".into(),
            }
        }
        "alwaysrestart" => p.setup.restart_required = yes(&s),
        "versioninfoversion" => p.app.version_info.version = s,
        "versioninfocompany" => p.app.version_info.company = s,
        "versioninfocopyright" => p.app.version_info.copyright = s,
        "versioninfoproductname" => p.app.version_info.product_name = s,
        "versioninfoproducttextversion" | "versioninfoproductversion" => p.app.version_info.product_version = s,
        "versioninfodescription" => p.app.version_info.description = s,
        "versioninfotextversion" => {}
        "wizardimagebackcolor" | "wizardimagestretch" | "wizardstyle" => {}
        other => warn.push(format!("[Setup] {} = {} — не поддерживается, пропущено", other, v)),
    }
}

/// Extract active ISDone archive calls from [Code].
fn import_code(code: &str, defs: &BTreeMap<String, String>, p: &mut Project, warn: &mut Vec<String>) {
    // strip (* ... *) and { ... } block comments and // line comments
    let mut clean = String::new();
    let c: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '(' && i + 1 < c.len() && c[i + 1] == '*' {
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == ')') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c[i] == '/' && i + 1 < c.len() && c[i + 1] == '/' {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c[i] == '{' && !(i + 1 < c.len() && (c[i + 1] == '#' || c[i + 1] == '{')) {
            // Pascal brace comment (constants live inside quotes, handled below)
            let mut j = i;
            let mut quoted = false;
            for k in (0..i).rev() {
                if c[k] == '\n' {
                    break;
                }
                if c[k] == '\'' {
                    quoted = !quoted;
                }
            }
            if !quoted {
                while j < c.len() && c[j] != '}' {
                    j += 1;
                }
                i = j + 1;
                continue;
            }
        }
        clean.push(c[i]);
        i += 1;
    }
    let clean = subst(&clean, defs);
    let mut pending_disk: Option<String> = None;
    for stmt in clean.split(';') {
        let s = stmt.trim();
        let lower = s.to_ascii_lowercase();
        let strings: Vec<String> = s.split('\'').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, x)| x.to_string()).collect();
        if lower.contains("showchangediskwindow") {
            pending_disk = strings.first().cloned();
            continue;
        }
        let fmt = if lower.contains("isarcextract") {
            "arc"
        } else if lower.contains("is7zipextract") {
            "7z"
        } else if lower.contains("israrextract") {
            warn.push("ISRarExtract: RAR не поддерживается встроенным распаковщиком — переупакуйте в 7z/smpk".into());
            continue;
        } else if lower.contains("isprecompextract") || lower.contains("issrepextract") {
            warn.push("Precomp/SREP шаги нужно оформить как инструменты + команды [Run]".into());
            continue;
        } else {
            continue;
        };
        if strings.len() < 2 {
            continue;
        }
        let file = strings[0].trim_start_matches("{src}\\").to_string();
        let dest = strings[1].trim_end_matches('\\').to_string();
        // component number (first numeric argument) is informational only
        let mut a = ArchiveItem { file, dest, format: fmt.into(), ..Default::default() };
        if let Some(d) = pending_disk.take() {
            a.disk_prompt = d;
        }
        if strings.len() > 2 && !strings[2].is_empty() && fmt == "7z" {
            a.password = strings[2].clone();
        }
        p.archives.push(a);
    }
    if p.archives.iter().any(|a| a.format == "arc") && !p.tools.iter().any(|t| t.source.to_ascii_lowercase().contains("unarc")) {
        warn.push("Для .arc архивов добавьте unarc.dll (или unarc.exe) в «Инструменты»".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry() {
        let e = parse_entry(r#"Name: desktopicon; Description: {cm:CreateDesktopIcon}; GroupDescription: "a; b"; Flags: unchecked"#);
        assert_eq!(e["groupdescription"], "a; b");
        assert_eq!(e["flags"], "unchecked");
    }
    #[test]
    fn script() {
        let s = r#"#define AppName "Test"
#define InnoDir "D:\inno"
;#define Components
[Setup]
AppId={{33C3E75F-645E-473B-AACE-5AED658BF59B}}
AppName={#AppName}
Compression=lzma/ultra
DefaultDirName={pf}\{#AppName}
[Languages]
Name: russian; MessagesFile: compiler:Languages\Russian.isl; LicenseFile: {#InnoDir}\GNU.rtf;
[Files]
Source: {#InnoDir}\{#AppName}\progFolder\*; DestDir: {app}; Flags: ignoreversion recursesubdirs createallsubdirs
[Code]
procedure X;
begin
  if not ISArcExtract ( 0, 0, ExpandConstant('{src}\data1.arc'), ExpandConstant('{app}'), false, '', '', ExpandConstant('{app}'), false) then break;
(*
  if not ISArcExtract ( 0, 0, ExpandConstant('{src}\data2.arc'), ExpandConstant('{app}'), false, '', '', '', false) then break;
*)
end;
"#;
        let r = import_str(s, &BTreeMap::new()).unwrap();
        let p = r.project;
        assert_eq!(p.app.name, "Test");
        assert_eq!(p.app.app_id, "{33C3E75F-645E-473B-AACE-5AED658BF59B}");
        assert_eq!(p.interface.languages[0].id, "ru");
        assert_eq!(p.files[0].source, "{#InnoDir}\\Test\\progFolder\\*");
        assert_eq!(p.defines["InnoDir"], "D:\\inno");
        assert_eq!(p.archives.len(), 1);
        assert_eq!(p.archives[0].file, "data1.arc");
    }
}
