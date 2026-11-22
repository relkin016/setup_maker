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

/// Workaround for GNU ld (mingw): it emits bogus IMAGE_REL_BASED_HIGHLOW base
/// relocations (type 3) on the 32-bit TLS-offset displacements in x64 code
/// (`mov rcx, gs:[0x58]; ... lea rsi, [rax+0xb8]`). HIGHLOW is invalid in PE32+;
/// under ASLR the loader adds the image-base delta to those offsets and the
/// stub crashes at start-up. Turning them into ABSOLUTE (type 0) padding is safe.
pub fn fix_pe64_relocs(d: &mut [u8]) {
    let u16at = |d: &[u8], o: usize| u16::from_le_bytes([d[o], d[o + 1]]);
    let u32at = |d: &[u8], o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    if d.len() < 0x100 || &d[0..2] != b"MZ" {
        return;
    }
    let e = u32at(d, 0x3c) as usize;
    if d.len() < e + 24 + 112 + 48 || &d[e..e + 4] != b"PE\0\0" || u16at(d, e + 24) != 0x20b {
        return; // not PE32+
    }
    let nsec = u16at(d, e + 6) as usize;
    let opt = u16at(d, e + 20) as usize;
    let dir = e + 24 + 112 + 5 * 8; // base relocation directory
    let (rel_rva, rel_sz) = (u32at(d, dir), u32at(d, dir + 4) as usize);
    if rel_rva == 0 || rel_sz == 0 {
        return;
    }
    let mut ro = None;
    for i in 0..nsec {
        let o = e + 24 + opt + 40 * i;
        let (vs, va, rs, rp) = (u32at(d, o + 8), u32at(d, o + 12), u32at(d, o + 16), u32at(d, o + 20));
        if rel_rva >= va && rel_rva < va + vs.max(rs) {
            ro = Some((rp + rel_rva - va) as usize);
            break;
        }
    }
    let Some(ro) = ro else { return };
    let end = (ro + rel_sz).min(d.len());
    let mut p = ro;
    while p + 8 <= end {
        let size = u32at(d, p + 4) as usize;
        if size < 8 || p + size > end {
            break;
        }
        for i in 0..(size - 8) / 2 {
            let o = p + 8 + 2 * i;
            if u16at(d, o) >> 12 == 3 {
                d[o] = 0;
                d[o + 1] = 0;
            }
        }
        p += size;
    }
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
    fix_pe64_relocs(&mut out);
    Ok(out)
}
