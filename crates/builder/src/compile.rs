//! Build pipeline: project -> setup.exe (+ optional disk slices / packed archives).

use crate::{images, pe, stubs};
use smk_core::codec::{self, Codec, EncodeOpts};
use smk_core::format::{self, DirRec, FileRec, Footer, Header, ResRec, Segment, StreamInfo};
use smk_core::project::{Conditions, FileItem, Project};
use smk_core::wildcard;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;
pub type Prog = Arc<dyn Fn(f32) + Send + Sync>;

#[derive(Clone)]
pub struct BuildOptions {
    pub log: Log,
    pub progress: Prog,
    pub cancel: Arc<AtomicBool>,
    /// Extra / overriding defines (`-D Name=Value`).
    pub defines: BTreeMap<String, String>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            log: Arc::new(|s| println!("{}", s)),
            progress: Arc::new(|_| {}),
            cancel: Arc::new(AtomicBool::new(false)),
            defines: BTreeMap::new(),
        }
    }
}

#[allow(dead_code)]
pub struct BuildResult {
    pub output: PathBuf,
    pub extra: Vec<PathBuf>,
    pub raw_size: u64,
    pub packed_size: u64,
    pub files: usize,
    pub seconds: f64,
    pub warnings: Vec<String>,
}

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(ctx: &str) -> impl Fn(E) -> String + '_ {
    move |err| format!("{}: {}", ctx, err)
}

// ---------------------------------------------------------------- defines

/// Replace `{#Name}` (and `{#StringChange(Name, "a", "b")}`) in all strings.
pub fn apply_defines(p: &Project, extra: &BTreeMap<String, String>) -> R<Project> {
    let mut defs: BTreeMap<String, String> = BTreeMap::new();
    defs.insert("AppName".into(), p.app.name.clone());
    defs.insert("AppVersion".into(), p.app.version.clone());
    defs.insert("AppPublisher".into(), p.app.publisher.clone());
    defs.insert("AppExeName".into(), p.app.exe.clone());
    defs.insert("AppUrl".into(), p.app.url.clone());
    defs.insert("AppURL".into(), p.app.url.clone());
    defs.insert("VersionType".into(), p.app.version_type.clone());
    for (k, v) in &p.defines {
        defs.insert(k.clone(), v.clone());
    }
    for (k, v) in extra {
        defs.insert(k.clone(), v.clone());
    }
    // defines may reference each other
    for _ in 0..4 {
        let snapshot = defs.clone();
        for v in defs.values_mut() {
            *v = subst(v, &snapshot);
        }
    }
    let mut val = serde_json::to_value(p).map_err(|e| e.to_string())?;
    walk(&mut val, &defs);
    serde_json::from_value(val).map_err(|e| e.to_string())
}

fn walk(v: &mut serde_json::Value, d: &BTreeMap<String, String>) {
    match v {
        serde_json::Value::String(s) => {
            if s.contains("{#") {
                *s = subst(s, d);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| walk(x, d)),
        serde_json::Value::Object(o) => o.values_mut().for_each(|x| walk(x, d)),
        _ => {}
    }
}

fn lookup<'a>(d: &'a BTreeMap<String, String>, k: &str) -> Option<&'a String> {
    d.get(k).or_else(|| d.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v))
}

pub fn subst(s: &str, d: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{#") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let expr = after[..j].trim();
        let val = if let Some(args) = expr.strip_prefix("StringChange(").and_then(|x| x.strip_suffix(')')) {
            let parts: Vec<String> = args.splitn(3, ',').map(|x| x.trim().trim_matches('"').to_string()).collect();
            if parts.len() == 3 {
                lookup(d, &parts[0]).cloned().unwrap_or_default().replace(&parts[1], &parts[2])
            } else {
                String::new()
            }
        } else {
            match lookup(d, expr) {
                Some(v) => v.clone(),
                None => format!("{{#{}}}", expr),
            }
        };
        out.push_str(&val);
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- paths

/// Resolve a project-relative path; accepts both `\` and `/`.
pub fn resolve(base: &Path, p: &str) -> PathBuf {
    let norm = if cfg!(windows) { p.to_string() } else { p.replace('\\', "/") };
    let pb = PathBuf::from(&norm);
    let full = if pb.is_absolute() { pb } else { base.join(pb) };
    if cfg!(windows) || full.exists() {
        return full;
    }
    case_insensitive(&full).unwrap_or(full)
}

/// Windows scripts ignore case; find the real path on case-sensitive systems.
fn case_insensitive(p: &Path) -> Option<PathBuf> {
    let mut cur = PathBuf::new();
    for comp in p.components() {
        let next = cur.join(comp.as_os_str());
        if next.exists() || wildcard::has_wildcard(&comp.as_os_str().to_string_lossy()) {
            cur = next;
            continue;
        }
        let want = comp.as_os_str().to_string_lossy().to_lowercase();
        let dir = if cur.as_os_str().is_empty() { PathBuf::from(".") } else { cur.clone() };
        let found = std::fs::read_dir(&dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == want)?;
        cur = cur.join(found.file_name());
    }
    Some(cur)
}

pub fn deterministic_guid(seed: &str) -> String {
    let h = xxhash_rust::xxh3::xxh3_128(seed.as_bytes());
    let b = h.to_le_bytes();
    format!(
        "{{{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-4{:01X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6] & 15, b[7], (b[8] & 0x3f) | 0x80, b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

pub fn new_guid() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    deterministic_guid(&format!("{}-{}", t, std::process::id()))
}

fn unix_to_filetime(t: SystemTime) -> u64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() * 10_000_000 + d.subsec_nanos() as u64 / 100 + 116_444_736_000_000_000,
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------- file gathering

#[derive(Clone)]
pub struct SrcFile {
    pub src: PathBuf,
    pub dest: String,
    pub size: u64,
    pub mtime: u64,
    pub attrs: u32,
    pub item: usize,
}

fn join_dest(dir: &str, rel: &str) -> String {
    let d = dir.trim_end_matches('\\');
    if rel.is_empty() {
        d.to_string()
    } else {
        format!("{}\\{}", d, rel.replace('/', "\\"))
    }
}

fn excluded(excludes: &[String], rel: &str, name: &str) -> bool {
    excludes.iter().any(|x| {
        let x = x.replace('/', "\\");
        let x = x.trim_start_matches('\\');
        wildcard::matches(x, name) || wildcard::matches(x, rel) || wildcard::matches(&format!("*\\{}", x), &format!("\\{}", rel))
    })
}

fn attrs_of(s: &str) -> u32 {
    let mut a = 0;
    for w in s.split_whitespace() {
        match w.to_ascii_lowercase().as_str() {
            "readonly" => a |= format::ATTR_READONLY,
            "hidden" => a |= format::ATTR_HIDDEN,
            "system" => a |= format::ATTR_SYSTEM,
            _ => {}
        }
    }
    a
}

/// Expand one [Files] item into concrete files (+ empty directories).
pub fn gather_item(base: &Path, it: &FileItem, idx: usize, out: &mut Vec<SrcFile>, dirs: &mut Vec<String>) -> R<()> {
    let src = resolve(base, &it.source);
    let (root, pattern) = if src.is_dir() {
        (src.clone(), "*".to_string())
    } else {
        let name = src.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if wildcard::has_wildcard(&name) {
            (src.parent().map(|p| p.to_path_buf()).unwrap_or_default(), name)
        } else if src.is_file() {
            let md = std::fs::metadata(&src).map_err(e(&src.display().to_string()))?;
            let dest_name = if it.dest_name.is_empty() { name } else { it.dest_name.clone() };
            out.push(SrcFile {
                src: src.clone(),
                dest: join_dest(&it.dest_dir, &dest_name),
                size: md.len(),
                mtime: md.modified().map(unix_to_filetime).unwrap_or(0),
                attrs: attrs_of(&it.attributes),
                item: idx,
            });
            return Ok(());
        } else if it.skip_if_missing {
            return Ok(());
        } else {
            return Err(format!("Файл не найден: {}", src.display()));
        }
    };
    if !root.is_dir() {
        if it.skip_if_missing {
            return Ok(());
        }
        return Err(format!("Папка не найдена: {}", root.display()));
    }
    let before = out.len();
    walk_dir(&root, &root, &pattern, it, idx, out, dirs)?;
    if out.len() == before && !it.skip_if_missing && !it.empty_dirs {
        return Err(format!("Нет файлов по маске: {}", src.display()));
    }
    Ok(())
}

fn walk_dir(root: &Path, dir: &Path, pattern: &str, it: &FileItem, idx: usize, out: &mut Vec<SrcFile>, dirs: &mut Vec<String>) -> R<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(e(&dir.display().to_string()))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let mut any = false;
    for en in entries {
        let p = en.path();
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('/', "\\");
        let name = en.file_name().to_string_lossy().to_string();
        if excluded(&it.excludes, &rel, &name) {
            continue;
        }
        let ft = en.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            if it.recursive {
                walk_dir(root, &p, pattern, it, idx, out, dirs)?;
                any = true;
            }
        } else if wildcard::matches(pattern, &name) {
            let md = en.metadata().map_err(|e| e.to_string())?;
            out.push(SrcFile {
                src: p.clone(),
                dest: join_dest(&it.dest_dir, &rel),
                size: md.len(),
                mtime: md.modified().map(unix_to_filetime).unwrap_or(0),
                attrs: attrs_of(&it.attributes),
                item: idx,
            });
            any = true;
        }
    }
    if !any && it.empty_dirs && dir != root {
        let rel = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy().replace('/', "\\");
        dirs.push(join_dest(&it.dest_dir, &rel));
    }
    Ok(())
}

fn is_code(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".exe", ".dll", ".sys", ".ocx", ".cpl", ".scr", ".drv", ".efi", ".asi", ".pyd"].iter().any(|x| n.ends_with(x))
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn hash_file(p: &Path) -> io::Result<u128> {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let mut f = BufReader::with_capacity(1 << 20, File::open(p)?);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.digest128())
}

pub fn auto_threads(o: &EncodeOpts, requested: u32) -> u32 {
    if requested > 0 {
        return requested;
    }
    let cpus = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
    let ram = crate::sysinfo::total_ram_bytes().unwrap_or(8 << 30);
    let by_mem = ((ram as f64 * 0.6) / o.mem_per_thread() as f64) as u32;
    cpus.min(by_mem).max(1)
}

// ---------------------------------------------------------------- output writer

/// Writes compressed data either into the exe or into numbered slices.
struct DataWriter {
    exe: File,
    spanning: bool,
    slice_size: u64,
    base_path: PathBuf, // output without extension
    cur: Option<(File, u64)>,
    segments: Vec<Segment>,
    created: Vec<PathBuf>,
    total: u64,
}

impl DataWriter {
    fn start(&mut self, data_base: u64) {
        if !self.spanning {
            self.segments.push(Segment { file: String::new(), base: data_base, size: 0 });
        }
    }
    fn new_slice(&mut self) -> io::Result<()> {
        let n = self.segments.len() + 1;
        let name = format!("{}-{}.bin", self.base_path.file_name().unwrap().to_string_lossy(), n);
        let path = self.base_path.with_file_name(&name);
        let mut f = File::create(&path)?;
        // small signature so a wrong file is easy to recognise
        f.write_all(b"SMKSLICE")?;
        self.created.push(path);
        self.segments.push(Segment { file: name, base: 8, size: 0 });
        self.cur = Some((f, 0));
        Ok(())
    }
}

impl Write for DataWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.spanning {
            let n = self.exe.write(buf)?;
            self.segments[0].size += n as u64;
            self.total += n as u64;
            return Ok(n);
        }
        if self.cur.as_ref().map(|c| c.1 >= self.slice_size).unwrap_or(true) {
            self.new_slice()?;
        }
        let (f, used) = self.cur.as_mut().unwrap();
        let room = (self.slice_size - *used) as usize;
        let n = f.write(&buf[..buf.len().min(room)])?;
        *used += n as u64;
        self.segments.last_mut().unwrap().size += n as u64;
        self.total += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some((f, _)) = self.cur.as_mut() {
            f.flush()?;
        }
        self.exe.flush()
    }
}

struct Counter<'a, W: Write> {
    w: &'a mut W,
    n: u64,
}
impl<W: Write> Write for Counter<'_, W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let k = self.w.write(b)?;
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

// ---------------------------------------------------------------- streams

pub struct PlannedStream {
    pub when: Conditions,
    pub bcj: bool,
    pub files: Vec<usize>, // indices into the FileRec list
}

/// Group files into solid streams and dedupe identical content.
pub fn plan_streams(files: &[SrcFile], p: &Project, recs: &mut Vec<FileRec>, log: &Log) -> R<Vec<PlannedStream>> {
    let comp = &p.compression;
    let lzma = Codec::from_name(&comp.method) == Codec::Lzma2;
    let mut groups: BTreeMap<(Conditions, bool), Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        let when = p.files[f.item].when.clone();
        let bcj = lzma && comp.bcj && is_code(&f.dest);
        groups.entry((when, bcj)).or_default().push(i);
    }
    let mut streams = Vec::new();
    let mut dup_count = 0usize;
    let mut dup_bytes = 0u64;
    for ((when, bcj), mut idx) in groups {
        // solid order: by extension, then name (Inno sortfilesbyextension)
        idx.sort_by(|&a, &b| {
            let (fa, fb) = (&files[a], &files[b]);
            ext_of(&fa.dest).cmp(&ext_of(&fb.dest)).then(fa.dest.to_lowercase().cmp(&fb.dest.to_lowercase()))
        });
        let stream_no = streams.len() as u32;
        let mut seen: HashMap<(u64, u128), u32> = HashMap::new();
        let mut order = Vec::new();
        for i in idx {
            let f = &files[i];
            let it = &p.files[f.item];
            let rec_idx = recs.len() as u32;
            let mut dup_of = None;
            if comp.dedupe && f.size > 0 {
                let h = hash_file(&f.src).map_err(e(&f.src.display().to_string()))?;
                if let Some(&orig) = seen.get(&(f.size, h)) {
                    dup_of = Some(orig);
                    dup_count += 1;
                    dup_bytes += f.size;
                } else {
                    seen.insert((f.size, h), rec_idx);
                }
            }
            recs.push(FileRec {
                dest: f.dest.clone(),
                size: f.size,
                mtime: f.mtime,
                attrs: f.attrs,
                crc: 0,
                stream: stream_no,
                dup_of,
                overwrite: match it.overwrite.as_str() {
                    "ifnewer" => 1,
                    "never" => 2,
                    _ => 0,
                },
                uninstall: it.uninstall,
                delete_after: it.delete_after_install,
            });
            order.push(rec_idx as usize);
        }
        streams.push(PlannedStream { when, bcj, files: order });
    }
    if dup_count > 0 {
        log(&format!("Дубликаты: {} файлов ({}) сохранены один раз", dup_count, human(dup_bytes)));
    }
    Ok(streams)
}

/// Compress planned streams into `w`. Fills crc in `recs`.
fn write_streams<W: Write>(
    w: &mut W,
    planned: &[PlannedStream],
    srcs: &[PathBuf],
    recs: &mut [FileRec],
    opts: &EncodeOpts,
    bo: &BuildOptions,
    total_raw: u64,
    done_raw: &mut u64,
    base_offset: u64,
) -> R<Vec<StreamInfo>> {
    let mut out = Vec::new();
    let mut offset = base_offset;
    let mut buf = vec![0u8; 1 << 20];
    for ps in planned {
        let raw_size: u64 = ps.files.iter().filter(|&&i| recs[i].dup_of.is_none()).map(|&i| recs[i].size).sum();
        let mut counter = Counter { w, n: 0 };
        {
            let mut enc = codec::encoder(opts, ps.bcj, &mut counter).map_err(|e| e.to_string())?;
            for &i in &ps.files {
                if recs[i].dup_of.is_some() {
                    continue;
                }
                if bo.cancel.load(Ordering::Relaxed) {
                    return Err("Сборка отменена".into());
                }
                let mut f = File::open(&srcs[i]).map_err(e(&srcs[i].display().to_string()))?;
                let mut crc = crc32fast::Hasher::new();
                let mut left = recs[i].size;
                while left > 0 {
                    let n = f.read(&mut buf[..(left as usize).min(1 << 20)]).map_err(|e| e.to_string())?;
                    if n == 0 {
                        return Err(format!("Файл изменился во время сборки: {}", srcs[i].display()));
                    }
                    crc.update(&buf[..n]);
                    enc.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                    left -= n as u64;
                    *done_raw += n as u64;
                    (bo.progress)(*done_raw as f32 / total_raw.max(1) as f32);
                }
                recs[i].crc = crc.finalize();
            }
            enc.finish_box().map_err(|e| e.to_string())?;
        }
        let size = counter.n;
        let rec_ids: Vec<u32> = ps.files.iter().map(|&i| i as u32).collect();
        out.push(StreamInfo { codec: opts.codec, offset, size, raw_size, files: rec_ids, when: ps.when.clone() });
        offset += size;
    }
    Ok(out)
}

pub fn human(b: u64) -> String {
    let f = b as f64;
    if f >= 1073741824.0 {
        format!("{:.2} ГБ", f / 1073741824.0)
    } else if f >= 1048576.0 {
        format!("{:.1} МБ", f / 1048576.0)
    } else if f >= 1024.0 {
        format!("{:.1} КБ", f / 1024.0)
    } else {
        format!("{} Б", b)
    }
}

// ---------------------------------------------------------------- resources

fn read_text_resource(path: &Path) -> R<Vec<u8>> {
    let data = std::fs::read(path).map_err(e(&path.display().to_string()))?;
    if data.starts_with(b"{\\rtf") {
        return Ok(data);
    }
    // plain text: normalise to UTF-8
    if std::str::from_utf8(&data).is_ok() {
        return Ok(data);
    }
    let (s, _, _) = encoding_rs::WINDOWS_1251.decode(&data);
    Ok(s.into_owned().into_bytes())
}

struct ResWriter {
    list: Vec<ResRec>,
}

impl ResWriter {
    fn add(&mut self, exe: &mut File, name: &str, data: &[u8]) -> R<()> {
        let packed = zstd_compress(data)?;
        let offset = exe.stream_position().map_err(|e| e.to_string())?;
        exe.write_all(&packed).map_err(|e| e.to_string())?;
        self.list.push(ResRec { name: name.to_string(), offset, size: packed.len() as u64, raw_size: data.len() as u64, codec: Codec::Zstd });
        Ok(())
    }
}

fn zstd_compress(data: &[u8]) -> R<Vec<u8>> {
    let mut v = Vec::new();
    let o = EncodeOpts { codec: Codec::Zstd, level: 19, extreme: false, dict_mb: 8, threads: 1, bcj: false };
    {
        let mut enc = codec::encoder(&o, false, &mut v).map_err(|e| e.to_string())?;
        enc.write_all(data).map_err(|e| e.to_string())?;
        enc.finish_box().map_err(|e| e.to_string())?;
    }
    Ok(v)
}

// ---------------------------------------------------------------- smpk packing

/// Pack a folder into an `.smpk` archive. Returns (raw, packed) sizes.
pub fn pack_folder(src: &Path, out: &Path, p: &Project, bo: &BuildOptions) -> R<(u64, u64)> {
    let item = FileItem { source: src.to_string_lossy().to_string(), dest_dir: String::new(), ..Default::default() };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut tmp_p = p.clone();
    tmp_p.files = vec![item.clone()];
    gather_item(Path::new(""), &item, 0, &mut files, &mut dirs)?;
    for f in &mut files {
        f.dest = f.dest.trim_start_matches('\\').to_string();
    }
    let dirs: Vec<String> = dirs.into_iter().map(|d| d.trim_start_matches('\\').to_string()).collect();
    let mut recs = Vec::new();
    let planned = plan_streams(&files, &tmp_p, &mut recs, &bo.log)?;
    let srcs: Vec<PathBuf> = {
        // recs are in planned order; map back to sources by dest
        let by_dest: HashMap<&str, &PathBuf> = files.iter().map(|f| (f.dest.as_str(), &f.src)).collect();
        recs.iter().map(|r| by_dest[r.dest.as_str()].clone()).collect()
    };
    let total_raw: u64 = recs.iter().filter(|r| r.dup_of.is_none()).map(|r| r.size).sum();
    let mut opts = EncodeOpts::from_project(&p.compression, 1);
    opts.threads = auto_threads(&opts, p.compression.threads);
    // data goes to a temp file first; the header (with CRCs) is written in front afterwards
    let tmp = out.with_extension("smpk.tmp");
    let mut data = io::BufWriter::new(File::create(&tmp).map_err(e(&tmp.display().to_string()))?);
    let mut done = 0;
    let streams = write_streams(&mut data, &planned, &srcs, &mut recs, &opts, bo, total_raw, &mut done, 0)?;
    data.flush().map_err(|e| e.to_string())?;
    drop(data);
    let hdr = format::SmpkHeader { files: recs, dirs, streams, total_raw };
    let hb = format::encode_json(&hdr).map_err(|e| e.to_string())?;
    let mut f = File::create(out).map_err(e(&out.display().to_string()))?;
    f.write_all(&format::smpk_prefix(&hb)).map_err(|e| e.to_string())?;
    let mut t = File::open(&tmp).map_err(|e| e.to_string())?;
    io::copy(&mut t, &mut f).map_err(|e| e.to_string())?;
    drop(t);
    let _ = std::fs::remove_file(&tmp);
    let packed = f.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((total_raw, packed))
}

/// Unpacked size of an existing archive, when cheaply known.
fn archive_raw_size(path: &Path) -> Option<u64> {
    let fmt = smk_core::project::archive_format_by_name(&path.to_string_lossy());
    if fmt == "smpk" {
        let mut f = File::open(path).ok()?;
        let (h, _) = format::read_smpk_header(&mut f).ok()?;
        return Some(h.total_raw);
    }
    None
}

// ---------------------------------------------------------------- main build

/// `XXXX-...` / `{XXXX-...}` / `{{XXXX-...}` -> `{XXXX-...}`.
pub fn normalize_guid(s: &str) -> String {
    let core = s.trim().trim_matches(|c| c == '{' || c == '}');
    if core.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", core)
    }
}

pub fn validate(p: &Project) -> Vec<String> {
    let mut errs = Vec::new();
    if p.app.name.trim().is_empty() {
        errs.push("Не задано имя приложения".to_string());
    }
    let check_expr = |what: &str, s: &str, errs: &mut Vec<String>| {
        if let Err(e) = smk_core::expr::parse(s) {
            errs.push(format!("{}: ошибка в выражении '{}': {}", what, s, e));
        }
    };
    for f in &p.files {
        if f.source.trim().is_empty() {
            errs.push("Файлы: пустой источник".into());
        }
        check_expr("Файлы", &f.when.components, &mut errs);
        check_expr("Файлы", &f.when.tasks, &mut errs);
        check_expr("Файлы", &f.when.check, &mut errs);
    }
    for a in &p.archives {
        if a.file.trim().is_empty() {
            errs.push("Архивы: пустое имя файла".into());
        }
    }
    for t in &p.tasks {
        check_expr("Задачи", &t.components, &mut errs);
    }
    for c in &p.components {
        if c.name.trim().is_empty() {
            errs.push("Компоненты: пустое имя".into());
        }
        check_expr("Компоненты", &c.check, &mut errs);
    }
    errs
}

pub fn build(project: &Project, project_dir: &Path, bo: &BuildOptions) -> R<BuildResult> {
    let started = Instant::now();
    let log = bo.log.clone();
    let mut warnings = Vec::new();
    let mut p = apply_defines(project, &bo.defines)?;
    let errs = validate(&p);
    if !errs.is_empty() {
        return Err(errs.join("\n"));
    }
    if p.app.app_id.trim().is_empty() {
        p.app.app_id = deterministic_guid(&format!("{}|{}", p.app.name, p.app.publisher));
        warnings.push(format!("AppId не задан, использован {} — сохраните его в проекте, чтобы обновления ставились поверх", p.app.app_id));
    }
    p.app.app_id = normalize_guid(&p.app.app_id);
    p.expand_shortcut_asks();

    // ---- files
    log(&format!("Проект: {} {}", p.app.name, p.app.version));
    log("Сбор файлов...");
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for (i, it) in p.files.iter().enumerate() {
        gather_item(project_dir, it, i, &mut files, &mut dirs)?;
    }
    let total_raw: u64 = files.iter().map(|f| f.size).sum();
    log(&format!("Файлов: {}, объём: {}", files.len(), human(total_raw)));

    // ---- output paths
    let out_dir = resolve(project_dir, if p.output.dir.is_empty() { "." } else { &p.output.dir });
    std::fs::create_dir_all(&out_dir).map_err(e(&out_dir.display().to_string()))?;
    let base = if p.output.base_name.is_empty() { "setup".to_string() } else { p.output.base_name.clone() };
    let out_path = out_dir.join(format!("{}.exe", base));
    let tmp_path = out_dir.join(format!("{}.exe.tmp", base));

    // ---- tools / arch
    let mut tool_data: Vec<(String, Vec<u8>)> = Vec::new();
    for t in &p.tools {
        let src = resolve(project_dir, &t.source);
        let data = std::fs::read(&src).map_err(e(&src.display().to_string()))?;
        let name = if t.name.is_empty() { src.file_name().unwrap().to_string_lossy().to_string() } else { t.name.clone() };
        tool_data.push((name, data));
    }
    let arch = match p.requirements.arch.as_str() {
        "x86" => "x86",
        "x64" => "x64",
        _ => {
            let unarc32 = tool_data.iter().any(|(n, d)| n.eq_ignore_ascii_case("unarc.dll") && pe::is_32bit(d));
            if unarc32 {
                log("unarc.dll 32-битный — выбран 32-битный установщик (x86)");
                "x86"
            } else {
                "x64"
            }
        }
    };

    // ---- stub + PE resources
    let stub = stubs::get(arch)?;
    let icon = if p.interface.setup_icon.is_empty() { None } else { Some(std::fs::read(resolve(project_dir, &p.interface.setup_icon)).map_err(e(&p.interface.setup_icon))?) };
    let stub = match pe::patch(&stub, icon.as_deref(), &p) {
        Ok(s) => s,
        Err(err) => {
            warnings.push(format!("Не удалось записать иконку/версию в exe: {}", err));
            stub
        }
    };
    let mut exe = File::create(&tmp_path).map_err(e(&tmp_path.display().to_string()))?;
    exe.write_all(&stub).map_err(|e| e.to_string())?;
    let stub_len = stub.len() as u64;

    // ---- resources
    let mut res = ResWriter { list: vec![] };
    if !p.interface.wizard_image.is_empty() {
        let img = images::load_bgra(&resolve(project_dir, &p.interface.wizard_image), 328, 628)?;
        res.add(&mut exe, "img:wizard", &img)?;
    }
    if !p.interface.wizard_small_image.is_empty() {
        let img = images::load_bgra(&resolve(project_dir, &p.interface.wizard_small_image), 110, 110)?;
        res.add(&mut exe, "img:small", &img)?;
    }
    for (kind, global) in [("license", &p.setup.license_file), ("infobefore", &p.setup.info_before_file), ("infoafter", &p.setup.info_after_file)] {
        if !global.is_empty() {
            res.add(&mut exe, kind, &read_text_resource(&resolve(project_dir, global))?)?;
        }
    }
    for l in &p.interface.languages {
        let id = smk_core::messages::normalize_lang(&l.id);
        for (kind, path) in [("license", &l.license_file), ("infobefore", &l.info_before_file), ("infoafter", &l.info_after_file)] {
            if !path.is_empty() {
                res.add(&mut exe, &format!("{}:{}", kind, id), &read_text_resource(&resolve(project_dir, path))?)?;
            }
        }
    }
    for (name, data) in &tool_data {
        res.add(&mut exe, &format!("tool:{}", name), data)?;
    }
    if !p.uninstall.icon_file.is_empty() {
        let d = std::fs::read(resolve(project_dir, &p.uninstall.icon_file)).map_err(e(&p.uninstall.icon_file))?;
        res.add(&mut exe, "uninstall.ico", &d)?;
    }

    // ---- external archives (pack_from + size hints)
    let mut extra = Vec::new();
    let bo_arch = bo.clone();
    for a in p.archives.iter_mut() {
        if !a.pack_from.is_empty() {
            let src = resolve(project_dir, &a.pack_from);
            let dst = out_dir.join(&a.file);
            log(&format!("Упаковка {} -> {}", src.display(), dst.display()));
            let (raw, packed) = pack_folder(&src, &dst, &p_clone_compression(project), &bo_arch)?;
            log(&format!("  {} -> {} ({:.1}%)", human(raw), human(packed), packed as f64 * 100.0 / raw.max(1) as f64));
            a.size_hint = raw;
            extra.push(dst);
        } else if !wildcard::has_wildcard(&a.file) && !a.file.contains('{') {
            let cand = [out_dir.join(&a.file), resolve(project_dir, &a.file)];
            match cand.iter().find(|c| c.is_file()) {
                Some(c) => {
                    if let Some(s) = archive_raw_size(c) {
                        a.size_hint = s;
                    }
                    if !out_dir.join(&a.file).is_file() {
                        warnings.push(format!("Архив {} нужно положить рядом с установщиком: {}", a.file, out_dir.display()));
                    }
                }
                None => warnings.push(format!("Архив {} пока не найден — он должен лежать рядом с setup.exe при установке", a.file)),
            }
        }
    }

    // ---- streams
    let mut recs = Vec::new();
    let planned = plan_streams(&files, &p, &mut recs, &log)?;
    let by_dest: HashMap<String, PathBuf> = files.iter().map(|f| (f.dest.to_lowercase(), f.src.clone())).collect();
    let srcs: Vec<PathBuf> = recs.iter().map(|r| by_dest[&r.dest.to_lowercase()].clone()).collect();
    let stored_raw: u64 = recs.iter().filter(|r| r.dup_of.is_none()).map(|r| r.size).sum();

    let mut opts = EncodeOpts::from_project(&p.compression, 1);
    opts.threads = auto_threads(&opts, p.compression.threads);
    log(&format!(
        "Сжатие: {} уровень {}{}, словарь {} МБ, потоков {}{}",
        opts.codec.name(),
        opts.level,
        if opts.extreme && opts.codec == Codec::Lzma2 { " (extreme)" } else { "" },
        opts.dict_mb,
        opts.threads,
        if opts.bcj && opts.codec == Codec::Lzma2 { ", BCJ x86 для exe/dll" } else { "" }
    ));

    let data_base = exe.stream_position().map_err(|e| e.to_string())?;
    let out_stem = out_dir.join(&base);
    let mut dw = DataWriter {
        exe,
        spanning: p.output.disk_spanning,
        slice_size: p.output.slice_size_mb.max(1) * 1024 * 1024,
        base_path: out_stem,
        cur: None,
        segments: vec![],
        created: vec![],
        total: 0,
    };
    dw.start(data_base);
    let mut done = 0u64;
    let streams = {
        let mut bw = io::BufWriter::with_capacity(4 << 20, &mut dw);
        let s = write_streams(&mut bw, &planned, &srcs, &mut recs, &opts, bo, stored_raw, &mut done, 0)?;
        bw.flush().map_err(|e| e.to_string())?;
        s
    };
    dw.flush().map_err(|e| e.to_string())?;
    let packed = dw.total;
    let segments = dw.segments.clone();
    extra.extend(dw.created.clone());
    let mut exe = dw.exe;
    if segments.is_empty() {
        // spanning with no data at all
    }

    // ---- header
    let mut manifest = p.clone();
    manifest.files.clear();
    manifest.tools.clear();
    manifest.defines.clear();
    manifest.setup.password.clear();
    manifest.setup.license_file.clear();
    manifest.setup.info_before_file.clear();
    manifest.setup.info_after_file.clear();
    manifest.interface.wizard_image.clear();
    manifest.interface.wizard_small_image.clear();
    manifest.interface.setup_icon.clear();
    manifest.uninstall.icon_file.clear();
    for l in &mut manifest.interface.languages {
        l.id = smk_core::messages::normalize_lang(&l.id).to_string();
        l.license_file.clear();
        l.info_before_file.clear();
        l.info_after_file.clear();
    }
    for a in &mut manifest.archives {
        a.pack_from.clear();
    }
    let (password_hash, password_salt) = if p.setup.password.is_empty() {
        (String::new(), String::new())
    } else {
        let salt = format!("{:x}", xxhash_rust::xxh3::xxh3_64(new_guid().as_bytes()));
        (format::password_hash(&p.setup.password, &salt), salt)
    };
    let dir_recs: Vec<DirRec> = dirs.iter().map(|d| DirRec { path: d.clone(), uninstall: true, when: Conditions::default() }).collect();
    let header = Header {
        builder_version: smk_core::VERSION.to_string(),
        manifest,
        files: recs,
        dirs: dir_recs,
        streams,
        resources: res.list,
        segments,
        password_hash,
        password_salt,
    };
    let hb = format::encode_json(&header).map_err(|e| e.to_string())?;
    let header_off = exe.stream_position().map_err(|e| e.to_string())?;
    exe.write_all(&hb).map_err(|e| e.to_string())?;
    let footer = Footer { stub_len, header_off, header_len: hb.len() as u64, header_crc: crc32fast::hash(&hb), version: format::FORMAT_VERSION };
    exe.write_all(&footer.to_bytes()).map_err(|e| e.to_string())?;
    exe.flush().map_err(|e| e.to_string())?;
    drop(exe);
    if out_path.exists() {
        let _ = std::fs::remove_file(&out_path);
    }
    std::fs::rename(&tmp_path, &out_path).map_err(e(&out_path.display().to_string()))?;
    (bo.progress)(1.0);

    let exe_size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    let secs = started.elapsed().as_secs_f64();
    log(&format!(
        "Готово: {} ({}), данные {} -> {} ({:.1}%), {:.1} c",
        out_path.display(),
        human(exe_size),
        human(total_raw),
        human(packed),
        packed as f64 * 100.0 / total_raw.max(1) as f64,
        secs
    ));
    for w in &warnings {
        log(&format!("Предупреждение: {}", w));
    }
    Ok(BuildResult { output: out_path, extra, raw_size: total_raw, packed_size: packed, files: files.len(), seconds: secs, warnings })
}

fn p_clone_compression(p: &Project) -> Project {
    Project { compression: p.compression.clone(), ..Default::default() }
}

/// Verify an installer by decoding all streams (CRC check). Returns file count.
pub fn verify(exe: &Path, log: &Log) -> R<usize> {
    let mut f = File::open(exe).map_err(e(&exe.display().to_string()))?;
    let footer = Footer::read_from(&mut f).map_err(|e| e.to_string())?.ok_or("нет данных установщика")?;
    f.seek(SeekFrom::Start(footer.header_off)).map_err(|e| e.to_string())?;
    let mut hb = vec![0u8; footer.header_len as usize];
    f.read_exact(&mut hb).map_err(|e| e.to_string())?;
    if crc32fast::hash(&hb) != footer.header_crc {
        return Err("CRC заголовка не совпадает".into());
    }
    let h: Header = format::decode_json(&hb).map_err(|e| e.to_string())?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    let mut checked = 0;
    for s in &h.streams {
        // concatenate the stream bytes from segments
        let mut raw = Vec::with_capacity(s.size as usize);
        let mut pos = s.offset;
        let end = s.offset + s.size;
        let mut seg_start = 0u64;
        for seg in &h.segments {
            let seg_end = seg_start + seg.size;
            if pos < seg_end && end > seg_start {
                let from = pos - seg_start;
                let len = (end.min(seg_end) - pos) as usize;
                let path = if seg.file.is_empty() { exe.to_path_buf() } else { dir.join(&seg.file) };
                let mut sf = File::open(&path).map_err(e(&path.display().to_string()))?;
                sf.seek(SeekFrom::Start(seg.base + from)).map_err(|e| e.to_string())?;
                let mut chunk = vec![0u8; len];
                sf.read_exact(&mut chunk).map_err(|e| e.to_string())?;
                raw.extend_from_slice(&chunk);
                pos += len as u64;
            }
            seg_start = seg_end;
        }
        let mut r = codec::decoder(s.codec, &raw[..]).map_err(|e| e.to_string())?;
        for &i in &s.files {
            let fr = &h.files[i as usize];
            if fr.dup_of.is_some() {
                continue;
            }
            let mut left = fr.size;
            let mut crc = crc32fast::Hasher::new();
            let mut buf = vec![0u8; 1 << 20];
            while left > 0 {
                let n = (left as usize).min(buf.len());
                r.read_exact(&mut buf[..n]).map_err(|e| format!("{}: {}", fr.dest, e))?;
                crc.update(&buf[..n]);
                left -= n as u64;
            }
            if crc.finalize() != fr.crc {
                return Err(format!("CRC не совпадает: {}", fr.dest));
            }
            checked += 1;
        }
    }
    log(&format!("Проверка пройдена: {} файлов, {} потоков, ресурсов {}", checked, h.streams.len(), h.resources.len()));
    Ok(checked)
}
