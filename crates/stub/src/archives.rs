//! External archive extraction: smpk (own format), 7z, zip, tar.*, FreeArc.

use smk_core::codec;
use smk_core::format::{self, FileRec};
use std::cell::Cell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Created {
    pub path: PathBuf,
    pub is_dir: bool,
    pub fresh: bool,
}

pub struct Hooks<'a> {
    pub created: Vec<Created>,
    /// (bytes of archive consumed, current file) -> continue?
    pub progress: &'a mut dyn FnMut(u64, &str) -> bool,
}

type R<T> = Result<T, String>;

/// Find archives matching `pattern` (may contain wildcards / subfolders) in `dir`.
pub fn find_archives(dir: &Path, pattern: &str) -> Vec<PathBuf> {
    let pat = Path::new(pattern);
    let full = if pat.is_absolute() { pat.to_path_buf() } else { dir.join(pat) };
    let name = full.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if !smk_core::wildcard::has_wildcard(&name) {
        return if full.is_file() { vec![full] } else { vec![] };
    }
    let parent = full.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| dir.to_path_buf());
    let mut v: Vec<PathBuf> = std::fs::read_dir(&parent)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && smk_core::wildcard::matches(&name, &p.file_name().unwrap_or_default().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    v
}

/// Sanitize an archive entry name into a relative path.
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

struct Counting {
    f: File,
    n: Rc<Cell<u64>>,
}
impl Read for Counting {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let k = self.f.read(b)?;
        self.n.set(self.n.get() + k as u64);
        Ok(k)
    }
}
impl Seek for Counting {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        let r = self.f.seek(p)?;
        // seeking (random access formats): report position as progress
        self.n.set(r);
        Ok(r)
    }
}

impl Hooks<'_> {
    fn mkdirs(&mut self, d: &Path) -> R<()> {
        let mut missing = Vec::new();
        let mut c = d.to_path_buf();
        while !c.exists() {
            missing.push(c.clone());
            if !c.pop() {
                break;
            }
        }
        for m in missing.iter().rev() {
            std::fs::create_dir(m).or_else(|e| if m.is_dir() { Ok(()) } else { Err(e) }).map_err(|e| format!("{}: {}", m.display(), e))?;
            self.created.push(Created { path: m.clone(), is_dir: true, fresh: true });
        }
        Ok(())
    }

    /// Stream `r` into `dest`; `n` = counter of archive bytes read.
    fn write(&mut self, dest: &Path, r: &mut dyn Read, n: &Rc<Cell<u64>>, expect: Option<(u64, u32)>) -> R<()> {
        if let Some(p) = dest.parent() {
            self.mkdirs(p)?;
        }
        let fresh = !dest.exists();
        if !fresh {
            crate::w::clear_readonly(dest);
        }
        let shown = dest.to_string_lossy().to_string();
        if !(self.progress)(n.get(), &shown) {
            return Err("cancelled".into());
        }
        let f = File::create(dest).map_err(|e| format!("{}: {}", dest.display(), e))?;
        self.created.push(Created { path: dest.to_path_buf(), is_dir: false, fresh });
        let mut out = io::BufWriter::with_capacity(1 << 20, f);
        let mut buf = vec![0u8; 1 << 18];
        let mut crc = crc32fast::Hasher::new();
        let mut left = expect.map(|e| e.0);
        loop {
            let want = match left {
                Some(0) => break,
                Some(l) => (l as usize).min(buf.len()),
                None => buf.len(),
            };
            let k = r.read(&mut buf[..want]).map_err(|e| e.to_string())?;
            if k == 0 {
                if left.is_some() {
                    return Err(format!("unexpected end of data: {}", dest.display()));
                }
                break;
            }
            crc.update(&buf[..k]);
            out.write_all(&buf[..k]).map_err(|e| format!("{}: {}", dest.display(), e))?;
            if let Some(l) = left.as_mut() {
                *l -= k as u64;
            }
            if !(self.progress)(n.get(), "") {
                return Err("cancelled".into());
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        if let Some((_, c)) = expect {
            if c != 0 && crc.finalize() != c {
                return Err(format!("CRC error: {}", dest.display()));
            }
        }
        Ok(())
    }
}

pub fn extract(fmt: &str, archive: &Path, dest: &Path, password: &str, tmp: &Path, h: &mut Hooks) -> R<()> {
    let n = Rc::new(Cell::new(0u64));
    let open = || -> R<Counting> { Ok(Counting { f: File::open(archive).map_err(|e| format!("{}: {}", archive.display(), e))?, n: n.clone() }) };
    match fmt {
        "smpk" => extract_smpk(open()?, dest, h, &n),
        "7z" => {
            let mut err: Option<String> = None;
            let mut rd = sevenz_rust2::SevenZReader::new(open()?, sevenz_rust2::Password::from(password)).map_err(|e| e.to_string())?;
            let res = rd.for_each_entries(|entry, r| {
                let Some(rel) = safe_rel(entry.name()) else { return Ok(true) };
                let out = dest.join(rel);
                let res = if entry.is_directory() {
                    h.mkdirs(&out)
                } else {
                    h.write(&out, r, &n, None)
                };
                match res {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        err = Some(e);
                        Ok(false)
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            res.map_err(|e| e.to_string())
        }
        "zip" => {
            let mut z = zip::ZipArchive::new(open()?).map_err(|e| e.to_string())?;
            for i in 0..z.len() {
                let mut e = if password.is_empty() {
                    z.by_index(i).map_err(|e| e.to_string())?
                } else {
                    z.by_index_decrypt(i, password.as_bytes()).map_err(|e| e.to_string())?
                };
                let Some(rel) = e.enclosed_name() else { continue };
                let out = dest.join(rel);
                if e.is_dir() {
                    h.mkdirs(&out)?;
                } else {
                    let size = e.size();
                    let crc = e.crc32();
                    h.write(&out, &mut e, &n, Some((size, crc)))?;
                }
            }
            Ok(())
        }
        t if t.starts_with("tar") => {
            let f = open()?;
            let r: Box<dyn Read> = match t {
                "tar.gz" => Box::new(flate2::read::GzDecoder::new(io::BufReader::new(f))),
                "tar.xz" => codec::decoder(codec::Codec::Lzma2, f).map_err(|e| e.to_string())?,
                "tar.zst" => codec::decoder(codec::Codec::Zstd, f).map_err(|e| e.to_string())?,
                "tar.br" => codec::decoder(codec::Codec::Brotli, f).map_err(|e| e.to_string())?,
                _ => Box::new(f),
            };
            let mut a = tar::Archive::new(r);
            for e in a.entries().map_err(|e| e.to_string())? {
                let mut e = e.map_err(|e| e.to_string())?;
                let name = e.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
                let Some(rel) = safe_rel(&name) else { continue };
                let out = dest.join(rel);
                match e.header().entry_type() {
                    tar::EntryType::Directory => h.mkdirs(&out)?,
                    tar::EntryType::Regular | tar::EntryType::Continuous | tar::EntryType::GNUSparse => {
                        let size = e.size();
                        h.write(&out, &mut e, &n, Some((size, 0)))?
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        "arc" => crate::freearc::extract(archive, dest, password, tmp, h),
        other => Err(format!("unsupported archive format: {}", other)),
    }
}

fn extract_smpk(mut f: Counting, dest: &Path, h: &mut Hooks, n: &Rc<Cell<u64>>) -> R<()> {
    let (hdr, data_start) = format::read_smpk_header(&mut f).map_err(|e| e.to_string())?;
    for d in &hdr.dirs {
        if let Some(rel) = safe_rel(d) {
            h.mkdirs(&dest.join(rel))?;
        }
    }
    let mut written: std::collections::HashMap<u32, PathBuf> = Default::default();
    for s in &hdr.streams {
        f.seek(SeekFrom::Start(data_start + s.offset)).map_err(|e| e.to_string())?;
        let raw = (&mut f).take(s.size);
        let mut r = codec::decoder(s.codec, raw).map_err(|e| e.to_string())?;
        for &i in &s.files {
            let fr: &FileRec = &hdr.files[i as usize];
            let Some(rel) = safe_rel(&fr.dest) else { return Err(format!("bad name {}", fr.dest)) };
            let out = dest.join(rel);
            if let Some(o) = fr.dup_of {
                if let Some(src) = written.get(&o) {
                    if let Some(p) = out.parent() {
                        h.mkdirs(p)?;
                    }
                    let fresh = !out.exists();
                    crate::w::clear_readonly(&out);
                    std::fs::copy(src, &out).map_err(|e| format!("{}: {}", out.display(), e))?;
                    h.created.push(Created { path: out.clone(), is_dir: false, fresh });
                }
                continue;
            }
            h.write(&out, &mut r, n, Some((fr.size, fr.crc)))?;
            if fr.attrs != 0 {
                crate::w::set_attributes(&out, fr.attrs);
            }
            if fr.mtime != 0 {
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&out) {
                    crate::w::set_mtime(&file, fr.mtime);
                }
            }
            written.insert(i, out);
        }
    }
    Ok(())
}
