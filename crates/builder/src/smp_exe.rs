//! Decompiler of installers built by Setup Maker itself.
//!
//! The payload header (see `smk_core::format`) carries the whole project
//! manifest minus the things the builder turned into data: `[[files]]` items
//! (→ solid streams), tools / license / info texts / wizard images / uninstall
//! icon (→ resources), setup icon (→ PE resources of the stub). This module
//! reads the header, extracts all of that back into a project folder and
//! rebuilds the missing project fields, so the result rebuilds into an
//! equivalent setup.exe.

use crate::inno_exe::{extract_icon, safe_rel, sanitize, split_root, ExeImport, Imported};
use smk_core::codec;
use smk_core::format::{self, FileRec, Footer, Header};
use smk_core::project::{Conditions, FileItem, Project, ToolFile};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(ctx: &str) -> impl Fn(E) -> String + '_ {
    move |err| format!("{}: {}", ctx, err)
}

/// Is `path` an installer built by Setup Maker (payload footer present)?
pub fn is_smp_exe(path: &Path) -> bool {
    File::open(path).ok().and_then(|mut f| Footer::read_from(&mut f).ok().flatten()).is_some()
}

/// Read and validate the payload header.
pub fn read_header(setup: &Path) -> R<(Footer, Header)> {
    let mut f = File::open(setup).map_err(e(&setup.display().to_string()))?;
    let footer = Footer::read_from(&mut f).map_err(|e| e.to_string())?.ok_or("это не установщик Setup Maker (нет блока данных)")?;
    if footer.version != format::FORMAT_VERSION {
        return Err(format!("неподдерживаемая версия формата установщика ({}), ожидалась {}", footer.version, format::FORMAT_VERSION));
    }
    if footer.header_len > 1 << 30 {
        return Err("повреждённый заголовок установщика".into());
    }
    f.seek(SeekFrom::Start(footer.header_off)).map_err(|e| e.to_string())?;
    let mut hb = vec![0u8; footer.header_len as usize];
    f.read_exact(&mut hb).map_err(|e| e.to_string())?;
    if crc32fast::hash(&hb) != footer.header_crc {
        return Err("CRC заголовка не совпадает — установщик повреждён".into());
    }
    let h: Header = format::decode_json(&hb).map_err(|e| format!("заголовок: {}", e))?;
    Ok((footer, h))
}

// ---------------------------------------------------------------- data space reader

/// Sequential reader of a virtual range spread over setup.exe and `<base>-N.bin` slices.
struct SegReader<'a> {
    h: &'a Header,
    exe: &'a Path,
    dir: &'a Path,
    pos: u64,
    end: u64,
    cur: Option<(File, u64)>, // (file, bytes left in this segment)
}

impl<'a> SegReader<'a> {
    fn open_at(&mut self) -> io::Result<()> {
        let mut start = 0u64;
        for seg in &self.h.segments {
            if self.pos < start + seg.size {
                let local = self.pos - start;
                let path = if seg.file.is_empty() { self.exe.to_path_buf() } else { self.dir.join(&seg.file) };
                let mut f = File::open(&path).map_err(|err| io::Error::new(err.kind(), format!("{}: {}", path.display(), err)))?;
                f.seek(SeekFrom::Start(seg.base + local))?;
                self.cur = Some((f, seg.size - local));
                return Ok(());
            }
            start += seg.size;
        }
        Err(io::Error::new(io::ErrorKind::UnexpectedEof, "смещение за пределами данных"))
    }
}

impl Read for SegReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.end || buf.is_empty() {
            return Ok(0);
        }
        if self.cur.as_ref().map(|c| c.1 == 0).unwrap_or(true) {
            self.open_at()?;
        }
        let (f, left) = self.cur.as_mut().unwrap();
        let want = (buf.len() as u64).min(*left).min(self.end - self.pos) as usize;
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "данные установщика обрезаны"));
        }
        *left -= n as u64;
        self.pos += n as u64;
        Ok(n)
    }
}

// ---------------------------------------------------------------- helpers

fn read_resource(f: &mut File, r: &format::ResRec) -> R<Vec<u8>> {
    f.seek(SeekFrom::Start(r.offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; r.size as usize];
    f.read_exact(&mut buf).map_err(|e| format!("ресурс {}: {}", r.name, e))?;
    codec::decompress_all(r.codec, &buf, r.raw_size).map_err(|e| format!("ресурс {}: {}", r.name, e))
}

/// Raw BGRA (`[w][h][pixels]`, see images.rs) -> PNG.
fn bgra_to_png(d: &[u8]) -> R<Vec<u8>> {
    if d.len() < 8 {
        return Err("слишком короткая картинка".into());
    }
    let w = u32::from_le_bytes(d[0..4].try_into().unwrap());
    let h = u32::from_le_bytes(d[4..8].try_into().unwrap());
    let px = &d[8..];
    if w == 0 || h == 0 || px.len() as u64 != w as u64 * h as u64 * 4 {
        return Err(format!("неверный размер картинки {}x{}", w, h));
    }
    let mut rgba = Vec::with_capacity(px.len());
    for c in px.chunks_exact(4) {
        rgba.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
    }
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("картинка")?;
    let mut out = io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

fn text_ext(b: &[u8]) -> &'static str {
    if b.starts_with(b"{\\rtf") {
        "rtf"
    } else {
        "txt"
    }
}

fn attrs_str(a: u32) -> String {
    let mut v = Vec::new();
    if a & format::ATTR_READONLY != 0 {
        v.push("readonly");
    }
    if a & format::ATTR_HIDDEN != 0 {
        v.push("hidden");
    }
    if a & format::ATTR_SYSTEM != 0 {
        v.push("system");
    }
    v.join(" ")
}

fn filetime_to_system(ft: u64) -> Option<SystemTime> {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    let t = ft.checked_sub(EPOCH_DIFF)?;
    Some(UNIX_EPOCH + Duration::from_nanos(t.saturating_mul(100)))
}

/// Unique folder name under `files/`.
fn unique_dir(used: &mut Vec<String>, want: &str) -> String {
    let base = if want.is_empty() { "root".to_string() } else { want.to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
        name = format!("{}_{}", base, n);
        n += 1;
    }
    used.push(name.clone());
    name
}

/// Key that decides which `[[files]]` item a file belongs to.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    root: String,
    when: Conditions,
    overwrite: u8,
    uninstall: bool,
    delete_after: bool,
    attrs: u32,
}

struct Group {
    key: GroupKey,
    folder: String, // "files/<name>"
    empty_dirs: bool,
}

// ---------------------------------------------------------------- import

/// Decompile a Setup Maker `setup.exe` into `out_dir`.
pub fn import(setup: &Path, out_dir: &Path, log: &dyn Fn(&str)) -> R<ExeImport> {
    let mut warn: Vec<String> = Vec::new();
    let (footer, h) = read_header(setup)?;
    log(&format!("Установщик Setup Maker v{}: {} {}", h.builder_version, h.manifest.app.name, h.manifest.app.version));
    if h.builder_version != smk_core::VERSION {
        log(&format!("Собран версией {}, текущая — {}", h.builder_version, smk_core::VERSION));
    }
    std::fs::create_dir_all(out_dir).map_err(e(&out_dir.display().to_string()))?;
    let wr = |name: &str, data: &[u8]| -> R<()> {
        let p = out_dir.join(name);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(e(&d.display().to_string()))?;
        }
        std::fs::write(&p, data).map_err(e(&p.display().to_string()))
    };

    let mut p: Project = h.manifest.clone();
    let mut f = File::open(setup).map_err(e(&setup.display().to_string()))?;

    // ---- setup icon from the stub's PE resources
    let mut stub = vec![0u8; footer.stub_len.min(64 << 20) as usize];
    f.read_exact(&mut stub).map_err(|e| e.to_string())?;
    if let Some(ico) = extract_icon(&stub) {
        let default_icon = ["x64", "x86"].iter().filter_map(|a| crate::stubs::get(a).ok()).filter_map(|s| extract_icon(&s)).any(|d| d == ico);
        if !default_icon {
            wr("setup.ico", &ico)?;
            p.interface.setup_icon = "setup.ico".into();
        }
    }
    drop(stub);

    // ---- resources
    for r in &h.resources {
        let data = match read_resource(&mut f, r) {
            Ok(d) => d,
            Err(err) => {
                warn.push(err);
                continue;
            }
        };
        let name = r.name.as_str();
        if name == "img:wizard" || name == "img:small" {
            let file = if name == "img:wizard" { "wizard.png" } else { "wizard_small.png" };
            match bgra_to_png(&data) {
                Ok(png) => {
                    wr(file, &png)?;
                    if name == "img:wizard" {
                        p.interface.wizard_image = file.into();
                    } else {
                        p.interface.wizard_small_image = file.into();
                    }
                }
                Err(err) => warn.push(format!("{}: {}", name, err)),
            }
        } else if let Some(tool) = name.strip_prefix("tool:") {
            let Some(rel) = safe_rel(tool) else {
                warn.push(format!("пропущен инструмент с недопустимым именем: {}", tool));
                continue;
            };
            let file_name = rel.file_name().unwrap().to_string_lossy().to_string();
            let src = format!("tools/{}", file_name);
            wr(&src, &data)?;
            let name = if file_name == tool { String::new() } else { tool.to_string() };
            p.tools.push(ToolFile { source: src, name });
        } else if name == "uninstall.ico" {
            wr("uninstall.ico", &data)?;
            p.uninstall.icon_file = "uninstall.ico".into();
        } else {
            // license / infobefore / infoafter [:lang]
            let (kind, lang) = name.split_once(':').unwrap_or((name, ""));
            if !["license", "infobefore", "infoafter"].contains(&kind) {
                warn.push(format!("неизвестный ресурс пропущен: {}", name));
                continue;
            }
            let file = if lang.is_empty() { format!("{}.{}", kind, text_ext(&data)) } else { format!("{}_{}.{}", kind, sanitize(lang), text_ext(&data)) };
            wr(&file, &data)?;
            let slot = if lang.is_empty() {
                Some(match kind {
                    "license" => &mut p.setup.license_file,
                    "infobefore" => &mut p.setup.info_before_file,
                    _ => &mut p.setup.info_after_file,
                })
            } else {
                p.interface.languages.iter_mut().find(|l| l.id.eq_ignore_ascii_case(lang)).map(|l| match kind {
                    "license" => &mut l.license_file,
                    "infobefore" => &mut l.info_before_file,
                    _ => &mut l.info_after_file,
                })
            };
            match slot {
                Some(s) => *s = file,
                None => warn.push(format!("{}: язык {} не найден в проекте, файл сохранён как {}", kind, lang, file)),
            }
        }
    }

    // ---- plan [[files]] items
    let when_of: Vec<Conditions> = {
        let mut v = vec![Conditions::default(); h.files.len()];
        for s in &h.streams {
            for &i in &s.files {
                if let Some(x) = v.get_mut(i as usize) {
                    *x = s.when.clone();
                }
            }
        }
        v
    };
    let mut groups: Vec<Group> = Vec::new();
    let mut used_names: Vec<String> = Vec::new();
    let mut group_of = |key: GroupKey, groups: &mut Vec<Group>| -> usize {
        if let Some(i) = groups.iter().position(|g| g.key == key) {
            return i;
        }
        let name = unique_dir(&mut used_names, &sanitize(&key.root).to_lowercase());
        groups.push(Group { key, folder: format!("files/{}", name), empty_dirs: false });
        groups.len() - 1
    };
    // (group, relative path) per file record
    let mut place: Vec<Option<(usize, PathBuf)>> = Vec::with_capacity(h.files.len());
    for (i, fr) in h.files.iter().enumerate() {
        let (root, rel) = split_root(&fr.dest);
        let Some(rel) = safe_rel(&rel) else {
            warn.push(format!("пропущен файл с недопустимым путём: {}", fr.dest));
            place.push(None);
            continue;
        };
        let key = GroupKey {
            root,
            when: when_of[i].clone(),
            overwrite: fr.overwrite,
            uninstall: fr.uninstall,
            delete_after: fr.delete_after,
            attrs: fr.attrs,
        };
        let g = group_of(key, &mut groups);
        place.push(Some((g, rel)));
    }
    // empty directories: attach to a group with the same root (or a new one)
    for d in &h.dirs {
        let (root, rel) = split_root(&d.path);
        let Some(rel) = safe_rel(&rel) else { continue };
        let g = match groups.iter().position(|g| g.key.root == root) {
            Some(g) => g,
            None => group_of(GroupKey { root, when: d.when.clone(), overwrite: 0, uninstall: d.uninstall, delete_after: false, attrs: 0 }, &mut groups),
        };
        groups[g].empty_dirs = true;
        let full = out_dir.join(&groups[g].folder).join(rel);
        std::fs::create_dir_all(&full).map_err(e(&full.display().to_string()))?;
    }

    // ---- extract streams
    let dir = setup.parent().unwrap_or(Path::new("."));
    let total: u64 = h.files.iter().filter(|r| r.dup_of.is_none()).map(|r| r.size).sum();
    let mut done = 0u64;
    let mut last_pct = -1i32;
    let mut written: Vec<Option<PathBuf>> = vec![None; h.files.len()];
    let mut buf = vec![0u8; 1 << 20];
    let mut crc_errors = 0usize;
    if !h.files.is_empty() {
        log("Извлечение файлов...");
    }
    for s in &h.streams {
        let raw = SegReader { h: &h, exe: setup, dir, pos: s.offset, end: s.offset + s.size, cur: None };
        let mut rd = codec::decoder(s.codec, raw).map_err(|e| e.to_string())?;
        for &i in &s.files {
            let i = i as usize;
            let fr: &FileRec = h.files.get(i).ok_or("повреждённый заголовок: индекс файла")?;
            let target = place[i].as_ref().map(|(g, rel)| out_dir.join(&groups[*g].folder).join(rel));
            if let Some(t) = target.as_ref().and_then(|t| t.parent()) {
                std::fs::create_dir_all(t).map_err(e(&t.display().to_string()))?;
            }
            if let Some(orig) = fr.dup_of {
                // content already extracted earlier in this stream
                match (written.get(orig as usize).cloned().flatten(), &target) {
                    (Some(src), Some(t)) => {
                        std::fs::copy(&src, t).map_err(e(&t.display().to_string()))?;
                        written[i] = Some(t.clone());
                    }
                    (None, Some(_)) => warn.push(format!("не найден оригинал дубликата: {}", fr.dest)),
                    _ => {}
                }
            } else {
                let mut out = match &target {
                    Some(t) => Some(io::BufWriter::with_capacity(1 << 20, File::create(t).map_err(e(&t.display().to_string()))?)),
                    None => None,
                };
                let mut crc = crc32fast::Hasher::new();
                let mut left = fr.size;
                while left > 0 {
                    let n = (left as usize).min(buf.len());
                    rd.read_exact(&mut buf[..n]).map_err(|err| {
                        if err.kind() == io::ErrorKind::NotFound {
                            format!("не найден файл данных (тома -N.bin должны лежать рядом с setup.exe): {}", err)
                        } else {
                            format!("{}: {}", fr.dest, err)
                        }
                    })?;
                    crc.update(&buf[..n]);
                    if let Some(o) = out.as_mut() {
                        o.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                    }
                    left -= n as u64;
                    done += n as u64;
                    let pct = (done * 100 / total.max(1)) as i32;
                    if pct != last_pct {
                        last_pct = pct;
                        log(&format!("\rИзвлечение файлов: {}%", pct));
                    }
                }
                if let Some(o) = out {
                    o.into_inner().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
                }
                if crc.finalize() != fr.crc {
                    crc_errors += 1;
                    warn.push(format!("CRC не совпадает: {}", fr.dest));
                }
                written[i] = target.clone();
            }
            if let (Some(t), Some(mt)) = (&target, filetime_to_system(fr.mtime)) {
                if let Ok(file) = File::options().write(true).open(t) {
                    let _ = file.set_modified(mt);
                }
            }
        }
    }
    if !h.files.is_empty() {
        log("");
        log(&format!("Извлечено файлов: {}{}", written.iter().filter(|w| w.is_some()).count(), if crc_errors > 0 { format!(", ошибок CRC: {}", crc_errors) } else { String::new() }));
    }

    // ---- [[files]] items
    let mut items: Vec<(GroupKey, FileItem)> = groups
        .iter()
        .map(|g| {
            let k = &g.key;
            let item = FileItem {
                source: g.folder.clone(),
                dest_dir: k.root.clone(),
                dest_name: String::new(),
                recursive: true,
                empty_dirs: g.empty_dirs,
                excludes: vec![],
                overwrite: match k.overwrite {
                    1 => "ifnewer".into(),
                    2 => "never".into(),
                    _ => "always".into(),
                },
                uninstall: k.uninstall,
                skip_if_missing: false,
                delete_after_install: k.delete_after,
                attributes: attrs_str(k.attrs),
                when: k.when.clone(),
            };
            (k.clone(), item)
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    p.files = items.into_iter().map(|(_, it)| it).collect();

    // ---- things that cannot be recovered / need attention
    if !h.password_hash.is_empty() {
        warn.push("установщик защищён паролем: сам пароль не восстановить (хранится только хэш) — задайте его заново в «Настройках»".into());
    }
    if Path::new(&p.output.dir).is_absolute() || p.output.dir.contains(':') {
        p.output.dir = "output".into();
    }
    let mut missing: Vec<String> = Vec::new();
    for a in &p.archives {
        if !a.file.contains('*') && !a.file.contains('?') && !a.file.contains('{') && !dir.join(&a.file).is_file() {
            missing.push(a.file.clone());
        }
    }
    if !p.archives.is_empty() {
        let names: Vec<String> = p.archives.iter().map(|a| a.file.clone()).collect();
        warn.push(format!("внешние архивы ({}) не входят в setup.exe — положите их рядом с собранным установщиком", names.join(", ")));
    }
    if !missing.is_empty() {
        warn.push(format!("рядом с исходным setup.exe нет архивов: {}", missing.join(", ")));
    }

    let base = if p.output.base_name.is_empty() { "setup".to_string() } else { sanitize(&p.output.base_name) };
    let project_path = out_dir.join(format!("{}.smp", if base.is_empty() { "setup" } else { &base }));
    Ok(ExeImport { imported: Imported { project: p, warnings: warn }, project_path })
}

/// Import any supported installer: Setup Maker or Inno Setup.
pub fn import_any(setup: &Path, out_dir: &Path, log: &dyn Fn(&str)) -> R<ExeImport> {
    if is_smp_exe(setup) {
        import(setup, out_dir, log)
    } else {
        crate::inno_exe::import(setup, out_dir, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smk_core::codec::{Codec, EncodeOpts};
    use smk_core::format::{DirRec, ResRec, Segment, StreamInfo};

    fn zst(data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        let o = EncodeOpts { codec: Codec::Zstd, level: 3, extreme: false, dict_mb: 1, threads: 1, bcj: false };
        let mut enc = codec::encoder(&o, false, &mut v).unwrap();
        enc.write_all(data).unwrap();
        enc.finish_box().unwrap();
        v
    }

    fn rec(dest: &str, data: &[u8], stream: u32, dup_of: Option<u32>) -> FileRec {
        FileRec { dest: dest.into(), size: data.len() as u64, crc: crc32fast::hash(data), stream, dup_of, mtime: 133_000_000_000_000_000, ..Default::default() }
    }

    #[test]
    fn roundtrip_synthetic_installer() {
        let tmp = std::env::temp_dir().join(format!("smk_decomp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let exe_path = tmp.join("setup.exe");

        let a = b"hello app".to_vec();
        let b = b"config data".to_vec();
        let c = b"tool in sys".to_vec();
        let mut exe = b"MZ not a real stub".to_vec();
        let stub_len = exe.len() as u64;

        // resources
        let mut resources = Vec::new();
        let mut add_res = |exe: &mut Vec<u8>, name: &str, data: &[u8]| {
            let packed = zst(data);
            resources.push(ResRec { name: name.into(), offset: exe.len() as u64, size: packed.len() as u64, raw_size: data.len() as u64, codec: Codec::Zstd });
            exe.extend_from_slice(&packed);
        };
        let mut img = Vec::new();
        img.extend_from_slice(&2u32.to_le_bytes());
        img.extend_from_slice(&1u32.to_le_bytes());
        img.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255]);
        add_res(&mut exe, "img:wizard", &img);
        add_res(&mut exe, "tool:unarc.dll", b"MZtool");
        add_res(&mut exe, "license", b"{\\rtf1 lic}");

        // streams: 0 = {app} (a, b, dup of a), 1 = {sys} with conditions
        let data_base = exe.len() as u64;
        let mut s0 = a.clone();
        s0.extend_from_slice(&b);
        let p0 = zst(&s0);
        let p1 = zst(&c);
        exe.extend_from_slice(&p0);
        exe.extend_from_slice(&p1);
        let files = vec![
            rec("{app}\\bin\\a.txt", &a, 0, None),
            rec("{app}\\cfg\\b.ini", &b, 0, None),
            rec("{app}\\bin\\copy_of_a.txt", &a, 0, Some(0)),
            FileRec { attrs: format::ATTR_HIDDEN, overwrite: 1, ..rec("{sys}\\c.dll", &c, 1, None) },
        ];
        let when = Conditions { components: "main".into(), ..Default::default() };
        let streams = vec![
            StreamInfo { codec: Codec::Zstd, offset: 0, size: p0.len() as u64, raw_size: s0.len() as u64, files: vec![0, 1, 2], when: Conditions::default() },
            StreamInfo { codec: Codec::Zstd, offset: p0.len() as u64, size: p1.len() as u64, raw_size: c.len() as u64, files: vec![3], when: when.clone() },
        ];
        let mut manifest = Project::default();
        manifest.app.name = "Demo".into();
        manifest.app.app_id = "{11111111-2222-3333-4444-555555555555}".into();
        manifest.output.base_name = "demo_setup".into();
        let header = Header {
            builder_version: "0.0.0".into(),
            manifest,
            files,
            dirs: vec![DirRec { path: "{app}\\logs".into(), uninstall: true, when: Conditions::default() }],
            streams,
            resources,
            segments: vec![Segment { file: String::new(), base: data_base, size: (p0.len() + p1.len()) as u64 }],
            password_hash: "x".into(),
            password_salt: "y".into(),
        };
        let hb = format::encode_json(&header).unwrap();
        let header_off = exe.len() as u64;
        exe.extend_from_slice(&hb);
        let footer = Footer { stub_len, header_off, header_len: hb.len() as u64, header_crc: crc32fast::hash(&hb), version: format::FORMAT_VERSION };
        exe.extend_from_slice(&footer.to_bytes());
        std::fs::write(&exe_path, &exe).unwrap();

        assert!(is_smp_exe(&exe_path));
        let out = tmp.join("proj");
        let r = import_any(&exe_path, &out, &|_| {}).unwrap();
        let p = &r.imported.project;

        assert_eq!(r.project_path, out.join("demo_setup.smp"));
        assert_eq!(p.app.app_id, "{11111111-2222-3333-4444-555555555555}");
        assert_eq!(p.files.len(), 2);
        let app = p.files.iter().find(|f| f.dest_dir == "{app}").unwrap();
        let sys = p.files.iter().find(|f| f.dest_dir == "{sys}").unwrap();
        assert!(app.empty_dirs && app.recursive);
        assert_eq!(sys.when, when);
        assert_eq!(sys.overwrite, "ifnewer");
        assert_eq!(sys.attributes, "hidden");
        let rd = |item: &FileItem, rel: &str| std::fs::read(out.join(&item.source).join(rel)).unwrap();
        assert_eq!(rd(app, "bin/a.txt"), a);
        assert_eq!(rd(app, "bin/copy_of_a.txt"), a);
        assert_eq!(rd(app, "cfg/b.ini"), b);
        assert_eq!(rd(sys, "c.dll"), c);
        assert!(out.join(&app.source).join("logs").is_dir());

        assert_eq!(p.tools.len(), 1);
        assert_eq!(std::fs::read(out.join(&p.tools[0].source)).unwrap(), b"MZtool");
        assert_eq!(p.setup.license_file, "license.rtf");
        assert_eq!(p.interface.wizard_image, "wizard.png");
        let png = image::open(out.join("wizard.png")).unwrap().to_rgba8();
        assert_eq!(png.get_pixel(0, 0).0, [0, 0, 255, 255]); // BGRA 255,0,0 = blue
        assert!(r.imported.warnings.iter().any(|w| w.contains("паролем")));
        assert!(r.imported.warnings.iter().all(|w| !w.contains("CRC")));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
