//! setup.exe icon and version resource (via editpe).

use smk_core::project::Project;

pub fn is_32bit(pe: &[u8]) -> bool {
    if pe.len() < 0x40 || &pe[0..2] != b"MZ" {
        return false;
    }
    let off = u32::from_le_bytes(pe[0x3c..0x40].try_into().unwrap()) as usize;
    if pe.len() < off + 6 {
        return false;
    }
    u16::from_le_bytes([pe[off + 4], pe[off + 5]]) == 0x14c
}

fn ver4(s: &str) -> (u32, u32) {
    let mut it = s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(|x| x.parse::<u32>().unwrap_or(0) & 0xffff);
    let (a, b, c, d) = (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0));
    ((a << 16) | b, (c << 16) | d)
}

pub fn patch(stub: &[u8], icon: Option<&[u8]>, p: &Project) -> Result<Vec<u8>, String> {
    let mut image = editpe::Image::parse(stub.to_vec()).map_err(|e| e.to_string())?;
    let mut res = image.resource_directory().cloned().unwrap_or_default();
    if let Some(ico) = icon {
        res.set_main_icon(ico).map_err(|e| format!("иконка: {}", e))?;
    }
    if let Ok(Some(mut vi)) = res.get_version_info() {
        let v = &p.app.version_info;
        let or = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { a.to_string() };
        let version = or(&v.version, &p.app.version);
        let (ms, ls) = ver4(&version);
        vi.info.file_version = editpe::types::VersionU32 { major: ms, minor: ls };
        vi.info.product_version = editpe::types::VersionU32 { major: ms, minor: ls };
        let product_version = or(&v.product_version, &format!("{} {}", p.app.version, p.app.version_type).trim().to_string());
        let pairs = [
            ("CompanyName", or(&v.company, &p.app.publisher)),
            ("FileDescription", or(&v.description, &format!("{} Setup", p.app.name))),
            ("FileVersion", version.clone()),
            ("LegalCopyright", or(&v.copyright, &or(&p.app.copyright, &p.app.publisher))),
            ("ProductName", or(&v.product_name, &p.app.name)),
            ("ProductVersion", product_version),
            ("Comments", or(&p.app.comments, "Installer built with Setup Maker")),
        ];
        if let Some(t) = vi.strings.first_mut() {
            for (k, val) in pairs {
                t.strings.insert(k.to_string(), val);
            }
        }
        res.set_version_info(&vi).map_err(|e| format!("версия: {}", e))?;
    }
    image.set_resource_directory(res).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    image.write_writer(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}
