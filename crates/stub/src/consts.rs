//! Expansion of Inno-style constants: `{app}`, `{pf}`, `{group}`,
//! `{cm:LaunchProgram,App}`, `{%PATH}`, `{reg:HKLM\Key,Value|def}` ...

use crate::w;
use smk_core::messages;
use std::collections::BTreeMap;
use windows_sys::Win32::UI::Shell::*;

#[derive(Clone, Default)]
pub struct Ctx {
    pub lang: String,
    pub admin: bool,
    pub app: String,
    pub group: String,
    pub src: String,
    pub srcexe: String,
    pub tmp: String,
    pub uninstallexe: String,
    pub app_name: String,
    pub app_ver_name: String,
    /// Custom messages: lang ("*" = any) -> name -> text.
    pub messages: BTreeMap<String, BTreeMap<String, String>>,
    /// `/Name=value` command line parameters (`{param:Name}`).
    pub params: BTreeMap<String, String>,
}

impl Ctx {
    /// Message lookup: custom (lang, then "*") -> built-in.
    pub fn msg(&self, key: &str) -> String {
        for l in [self.lang.as_str(), "*"] {
            if let Some(m) = self.messages.get(l) {
                if let Some((_, v)) = m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    return self.expand(v);
                }
            }
        }
        // languages may also be keyed by long names ("russian")
        for (l, m) in &self.messages {
            if messages::normalize_lang(l) == self.lang && l != "*" {
                if let Some((_, v)) = m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    return self.expand(v);
                }
            }
        }
        let s = messages::builtin(&self.lang, key).unwrap_or(key).to_string();
        self.subst_names(&s)
    }

    pub fn msgf(&self, key: &str, args: &[&str]) -> String {
        messages::fmt(&self.msg(key), args)
    }

    pub fn subst_names(&self, s: &str) -> String {
        s.replace("[name/ver]", &self.app_ver_name).replace("[name]", &self.app_name)
    }

    fn folder(&self, name: &str) -> Option<String> {
        let common = self.admin;
        let kf = |id| w::known_folder(id);
        Some(match name {
            "app" => self.app.clone(),
            "group" => {
                let base = if common { kf(&FOLDERID_CommonPrograms) } else { kf(&FOLDERID_Programs) };
                if self.group.is_empty() {
                    base
                } else {
                    format!("{}\\{}", base, self.group)
                }
            }
            "src" => self.src.clone(),
            "srcexe" => self.srcexe.clone(),
            "tmp" => self.tmp.clone(),
            "uninstallexe" => self.uninstallexe.clone(),
            "pf" | "autopf" | "commonpf" => {
                if !common && name == "autopf" {
                    format!("{}\\Programs", kf(&FOLDERID_LocalAppData))
                } else {
                    kf(&FOLDERID_ProgramFiles)
                }
            }
            "pf32" | "commonpf32" => kf(&FOLDERID_ProgramFilesX86),
            "pf64" | "commonpf64" => {
                let v = w::env("ProgramW6432");
                if v.is_empty() {
                    kf(&FOLDERID_ProgramFiles)
                } else {
                    v
                }
            }
            "cf" | "commoncf" => kf(&FOLDERID_ProgramFilesCommon),
            "cf32" => kf(&FOLDERID_ProgramFilesCommonX86),
            "win" => w::windows_dir(),
            "sys" => w::system_dir(),
            "syswow64" => format!("{}\\SysWOW64", w::windows_dir()),
            "sd" => w::env("SystemDrive"),
            "fonts" => kf(&FOLDERID_Fonts),
            "userdesktop" => kf(&FOLDERID_Desktop),
            "commondesktop" => kf(&FOLDERID_PublicDesktop),
            "autodesktop" | "desktop" => {
                if common {
                    kf(&FOLDERID_PublicDesktop)
                } else {
                    kf(&FOLDERID_Desktop)
                }
            }
            "userprograms" => kf(&FOLDERID_Programs),
            "commonprograms" => kf(&FOLDERID_CommonPrograms),
            "autoprograms" => {
                if common {
                    kf(&FOLDERID_CommonPrograms)
                } else {
                    kf(&FOLDERID_Programs)
                }
            }
            "userstartmenu" => kf(&FOLDERID_StartMenu),
            "commonstartmenu" => kf(&FOLDERID_CommonStartMenu),
            "userstartup" => kf(&FOLDERID_Startup),
            "commonstartup" => kf(&FOLDERID_CommonStartup),
            "autostartup" => {
                if common {
                    kf(&FOLDERID_CommonStartup)
                } else {
                    kf(&FOLDERID_Startup)
                }
            }
            "userappdata" | "appdata" => kf(&FOLDERID_RoamingAppData),
            "localappdata" => kf(&FOLDERID_LocalAppData),
            "commonappdata" | "autoappdata" => kf(&FOLDERID_ProgramData),
            "userdocs" | "mydocs" => kf(&FOLDERID_Documents),
            "commondocs" => kf(&FOLDERID_PublicDocuments),
            "userfavorites" => kf(&FOLDERID_Favorites),
            "sendto" => kf(&FOLDERID_SendTo),
            "quicklaunch" => format!("{}\\Microsoft\\Internet Explorer\\Quick Launch", kf(&FOLDERID_RoamingAppData)),
            "usertemplates" => kf(&FOLDERID_Templates),
            "computername" => w::computer_name(),
            "username" => w::user_name(),
            "lang" => self.lang.clone(),
            "\\" => "\\".into(),
            _ => return None,
        })
    }

    /// Expand all constants in `s`. `{{` is a literal `{`.
    pub fn expand(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 32);
        let c: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < c.len() {
            if c[i] == '{' {
                if i + 1 < c.len() && c[i + 1] == '{' {
                    out.push('{');
                    i += 2;
                    continue;
                }
                // find matching brace (nesting allowed)
                let mut depth = 0;
                let mut j = i;
                while j < c.len() {
                    if c[j] == '{' {
                        depth += 1;
                    } else if c[j] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                if j >= c.len() {
                    out.extend(&c[i..]);
                    break;
                }
                let inner: String = c[i + 1..j].iter().collect();
                out.push_str(&self.constant(&inner));
                i = j + 1;
            } else {
                out.push(c[i]);
                i += 1;
            }
        }
        out
    }

    fn constant(&self, inner: &str) -> String {
        let lower = inner.to_ascii_lowercase();
        if let Some(v) = self.folder(&lower) {
            return v;
        }
        if let Some(rest) = inner.strip_prefix("cm:") {
            let mut parts = split_args(rest);
            let key = parts.remove(0);
            let args: Vec<String> = parts.iter().map(|a| self.expand(a)).collect();
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            return self.msgf(&key, &refs);
        }
        if let Some(rest) = inner.strip_prefix('%') {
            let (name, def) = rest.split_once('|').unwrap_or((rest, ""));
            let v = w::env(name);
            return if v.is_empty() { self.expand(def) } else { v };
        }
        if let Some(rest) = lower.strip_prefix("param:") {
            let orig = &inner[6..];
            let (name, def) = orig.split_once('|').unwrap_or((orig, ""));
            let _ = rest;
            return self.params.get(&name.to_ascii_lowercase()).cloned().unwrap_or_else(|| self.expand(def));
        }
        if lower.starts_with("reg:") {
            let body = &inner[4..];
            let (spec, def) = body.split_once('|').unwrap_or((body, ""));
            let (key, val) = spec.rsplit_once(',').unwrap_or((spec, ""));
            if let Some(v) = crate::reg::read_string(&self.expand(key), &self.expand(val)) {
                return v;
            }
            return self.expand(def);
        }
        // unknown: keep as is
        format!("{{{}}}", inner)
    }
}

/// Split `a,b,c` respecting nested braces.
fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                out.last_mut().unwrap().push(ch)
            }
            '}' => {
                depth -= 1;
                out.last_mut().unwrap().push(ch)
            }
            ',' if depth == 0 => out.push(String::new()),
            _ => out.last_mut().unwrap().push(ch),
        }
    }
    out
}
