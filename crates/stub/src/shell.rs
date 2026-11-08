//! Shortcut (.lnk / .url) creation via IShellLinkW.

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub struct Lnk<'a> {
    pub path: &'a str,
    pub target: &'a str,
    pub args: &'a str,
    pub workdir: &'a str,
    pub icon: &'a str,
    pub icon_index: i32,
    pub comment: &'a str,
}

pub fn create(l: &Lnk) -> Result<(), String> {
    let t = l.target.to_ascii_lowercase();
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:") || t.starts_with("www.") {
        // Internet shortcut
        let url = if t.starts_with("www.") { format!("http://{}", l.target) } else { l.target.to_string() };
        let mut s = format!("[InternetShortcut]\r\nURL={}\r\n", url);
        if !l.icon.is_empty() {
            s.push_str(&format!("IconFile={}\r\nIconIndex={}\r\n", l.icon, l.icon_index));
        }
        return std::fs::write(format!("{}.url", l.path), s).map_err(|e| e.to_string());
    }
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(l.target)).map_err(|e| e.to_string())?;
        if !l.args.is_empty() {
            link.SetArguments(&HSTRING::from(l.args)).map_err(|e| e.to_string())?;
        }
        let wd = if l.workdir.is_empty() {
            std::path::Path::new(l.target).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            l.workdir.to_string()
        };
        if !wd.is_empty() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(wd.as_str()));
        }
        if !l.icon.is_empty() {
            let _ = link.SetIconLocation(&HSTRING::from(l.icon), l.icon_index);
        }
        if !l.comment.is_empty() {
            let _ = link.SetDescription(&HSTRING::from(l.comment));
        }
        let pf: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        let file = HSTRING::from(format!("{}.lnk", l.path));
        pf.Save(PCWSTR(file.as_ptr()), true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Path of the file actually written by `create`.
pub fn written_path(l: &Lnk) -> String {
    let t = l.target.to_ascii_lowercase();
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:") || t.starts_with("www.") {
        format!("{}.url", l.path)
    } else {
        format!("{}.lnk", l.path)
    }
}
