//! Import of compiled Inno Setup installers (`setup.exe`), Inno 5.2.5 – 6.3.x.
//!
//! Reads the loader offset table, decompresses the setup header
//! (block 1: header strings, languages, messages, types, components, tasks,
//! dirs, files, icons, ini, registry, delete, run entries, wizard images;
//! block 2: file data locations), extracts the embedded files and turns
//! everything into a Setup Maker project folder.
//!
//! Pascal `[Code]` cannot be converted. Its string constants are scanned for
//! ISDone calls (`{src}\*.arc` → external archives), the wizard font and
//! welcome/finish captions; all strings are saved to `code_strings.txt`.

use smk_core::project::*;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

type R<T> = Result<T, String>;

pub struct Imported {
    pub project: Project,
    pub warnings: Vec<String>,
}

const fn ver(a: u32, b: u32, c: u32) -> u32 {
    (a << 24) | (b << 16) | (c << 8)
}

// ---------------------------------------------------------------- reader

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
    unicode: bool,
    cp: &'static encoding_rs::Encoding,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if n > self.d.len().saturating_sub(self.p) {
            return Err("unexpected end of header".into());
        }
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> R<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> R<i32> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> R<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// Little-endian set of `n` flags (Delphi set layout).
    fn set(&mut self, n: usize) -> R<u64> {
        let b = set_bytes(n);
        let s = self.take(b)?;
        Ok(s.iter().take(8).enumerate().fold(0u64, |a, (i, x)| a | ((*x as u64) << (8 * i))))
    }
    /// Length-prefixed raw string (AnsiString).
    fn bin(&mut self) -> R<Vec<u8>> {
        let n = self.u32()? as usize;
        if n > 256 << 20 {
            return Err("bad string length".into());
        }
        Ok(self.take(n)?.to_vec())
    }
    /// Length-prefixed `String` (UTF-16 in Unicode builds, ANSI codepage otherwise).
    fn s(&mut self) -> R<String> {
        let b = self.bin()?;
        self.decode(&b, self.unicode)
    }
    fn decode(&self, b: &[u8], utf16: bool) -> R<String> {
        if utf16 {
            if b.len() % 2 != 0 {
                return Err("bad UTF-16 string".into());
            }
            let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Ok(String::from_utf16_lossy(&w))
        } else {
            Ok(self.cp.decode_without_bom_handling(b).0.into_owned())
        }
    }
    fn winver(&mut self) -> R<()> {
        self.skip(20)
    }
}

fn set_bytes(n: usize) -> usize {
    let b = n.div_ceil(8).max(1);
    if b == 3 {
        4
    } else {
        b
    }
}

// ---------------------------------------------------------------- entries

#[allow(dead_code)]
#[derive(Default, Clone, Debug)]
struct Cond {
    components: String,
    tasks: String,
    languages: String,
    check: String,
}

#[derive(Default, Debug)]
struct LangE {
    name: String,
    dialog_font: String,
    license: Vec<u8>,
    info_before: Vec<u8>,
    info_after: Vec<u8>,
    lang_id: u32,
    codepage: u32,
}

#[derive(Default, Debug)]
struct MsgE {
    name: String,
    value: String,
    lang: i32,
}

#[derive(Default, Debug)]
struct TypeE {
    name: String,
    desc: String,
    check: String,
    custom: bool,
}

#[derive(Default, Debug)]
struct CompE {
    name: String,
    desc: String,
    types: String,
    check: String,
    extra: u64,
    options: u64,
}

#[derive(Default, Debug)]
struct TaskE {
    name: String,
    desc: String,
    group: String,
    components: String,
    check: String,
    options: u64,
}

#[derive(Default, Debug)]
struct DirE {
    name: String,
    cond: Cond,
    options: u64,
}

#[allow(dead_code)]
#[derive(Default, Debug)]
struct FileE {
    source: String,
    dest: String,
    cond: Cond,
    location: u32,
    attributes: u32,
    options: u64,
    ftype: u8,
}

#[derive(Default, Debug)]
struct IconE {
    name: String,
    filename: String,
    params: String,
    workdir: String,
    icon_file: String,
    comment: String,
    cond: Cond,
    icon_index: i32,
}

#[derive(Default, Debug)]
struct RegE {
    key: String,
    name: String,
    value: Vec<u8>,
    cond: Cond,
    hive: u32,
    vtype: u8,
    options: u64,
}

#[derive(Default, Debug)]
struct DelE {
    name: String,
    kind: u8,
}

#[derive(Default, Debug)]
struct RunE {
    name: String,
    params: String,
    workdir: String,
    status: String,
    desc: String,
    cond: Cond,
    show_cmd: i32,
    wait: u8,
    options: u64,
}

#[derive(Default, Debug, Clone)]
struct DataE {
    first_slice: u32,
    chunk_start: u32,
    offset: u64,
    size: u64,
    chunk_size: u64,
    sha1: Option<[u8; 20]>,
    options: u64,
}

const DATA_CALL_OPT: u64 = 1 << 4;
const DATA_ENCRYPTED: u64 = 1 << 6;
const DATA_COMPRESSED: u64 = 1 << 7;

#[derive(Default, Debug)]
struct Entries {
    langs: Vec<LangE>,
    msgs: Vec<MsgE>,
    types: Vec<TypeE>,
    comps: Vec<CompE>,
    tasks: Vec<TaskE>,
    dirs: Vec<DirE>,
    files: Vec<FileE>,
    icons: Vec<IconE>,
    ini_count: usize,
    regs: Vec<RegE>,
    install_delete: Vec<DelE>,
    uninstall_delete: Vec<DelE>,
    run: Vec<RunE>,
    uninstall_run: Vec<RunE>,
    wizard_image: Vec<u8>,
    wizard_small: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct Counts {
    langs: u32,
    msgs: u32,
    perms: u32,
    types: u32,
    comps: u32,
    tasks: u32,
    dirs: u32,
    files: u32,
    data: u32,
    icons: u32,
    ini: u32,
    regs: u32,
    del: u32,
    undel: u32,
    run: u32,
    unrun: u32,
}

/// Version-dependent layout knobs that are probed when unsure.
#[derive(Clone, Copy, Debug)]
struct Layout {
    file_flags: usize,
    icon_toast: bool,
}

fn cond(r: &mut Rd) -> R<Cond> {
    Ok(Cond { components: r.s()?, tasks: r.s()?, languages: r.s()?, check: r.s()? })
}

/// components, tasks, languages, check, after_install, before_install
fn cond_full(r: &mut Rd) -> R<Cond> {
    let c = cond(r)?;
    r.s()?;
    r.s()?;
    Ok(c)
}

fn plausible_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_alphanumeric() || "_-. ".contains(c))
}

fn parse_entries(r: &mut Rd, v: u32, n: &Counts, lay: Layout) -> R<Entries> {
    let mut e = Entries::default();
    for _ in 0..n.langs {
        let name = r.s()?;
        if !plausible_name(&name) {
            return Err("bad language name".into());
        }
        let _language_name = r.bin()?;
        let dialog_font = r.s()?;
        let _title = r.s()?;
        let _welcome = r.s()?;
        let _copyright = r.s()?;
        let _data = r.bin()?;
        let license = r.bin()?;
        let info_before = r.bin()?;
        let info_after = r.bin()?;
        let lang_id = r.u32()?;
        let codepage = if r.unicode { 0 } else { r.u32()? };
        r.skip(16)?; // dialog/title/welcome/copyright font sizes
        if v >= ver(5, 2, 3) {
            r.u8()?;
        }
        if lang_id > 0xffff {
            return Err("bad language id".into());
        }
        e.langs.push(LangE { name, dialog_font, license, info_before, info_after, lang_id, codepage });
    }
    for _ in 0..n.msgs {
        let name = r.s()?;
        if !plausible_name(&name) {
            return Err("bad message name".into());
        }
        let value = r.s()?;
        let lang = r.i32()?;
        e.msgs.push(MsgE { name, value, lang });
    }
    for _ in 0..n.perms {
        r.bin()?;
    }
    for _ in 0..n.types {
        let name = r.s()?;
        let desc = r.s()?;
        let _languages = r.s()?;
        let check = r.s()?;
        r.winver()?;
        let opts = r.set(1)?;
        r.u8()?; // setup type
        r.u64()?; // size
        e.types.push(TypeE { name, desc, check, custom: opts & 1 != 0 });
    }
    for _ in 0..n.comps {
        let name = r.s()?;
        let desc = r.s()?;
        let types = r.s()?;
        let _languages = r.s()?;
        let check = r.s()?;
        let extra = r.u64()?;
        r.i32()?; // level
        r.u8()?; // used
        r.winver()?;
        let options = r.set(5)?;
        r.u64()?; // size
        e.comps.push(CompE { name, desc, types, check, extra, options });
    }
    for _ in 0..n.tasks {
        let name = r.s()?;
        let desc = r.s()?;
        let group = r.s()?;
        let components = r.s()?;
        let _languages = r.s()?;
        let check = r.s()?;
        r.i32()?;
        r.u8()?;
        r.winver()?;
        let options = r.set(5)?;
        e.tasks.push(TaskE { name, desc, group, components, check, options });
    }
    for _ in 0..n.dirs {
        let name = r.s()?;
        let cond = cond_full(r)?;
        r.u32()?; // attribs
        r.winver()?;
        r.u16()?; // permission
        let options = r.set(5)?;
        e.dirs.push(DirE { name, cond, options });
    }
    for _ in 0..n.files {
        let source = r.s()?;
        let dest = r.s()?;
        let _font = r.s()?;
        let _assembly = r.s()?;
        let cond = cond_full(r)?;
        r.winver()?;
        let location = r.u32()?;
        let attributes = r.u32()?;
        r.u64()?; // external size
        r.u16()?; // permission
        let options = r.set(lay.file_flags)?;
        let ftype = r.u8()?;
        if ftype > 2 {
            return Err("bad file type".into());
        }
        e.files.push(FileE { source, dest, cond, location, attributes, options, ftype });
    }
    for _ in 0..n.icons {
        let name = r.s()?;
        let filename = r.s()?;
        let params = r.s()?;
        let workdir = r.s()?;
        let icon_file = r.s()?;
        let comment = r.s()?;
        let cond = cond_full(r)?;
        if v >= ver(5, 3, 5) {
            r.s()?; // AppUserModelID
        }
        if lay.icon_toast {
            r.skip(16)?;
        }
        r.winver()?;
        let icon_index = r.i32()?;
        r.i32()?; // show command
        r.u8()?; // close on exit
        r.u16()?; // hotkey
        r.set(7)?;
        e.icons.push(IconE { name, filename, params, workdir, icon_file, comment, cond, icon_index });
    }
    for _ in 0..n.ini {
        for _ in 0..4 {
            r.s()?;
        }
        cond_full(r)?;
        r.winver()?;
        r.set(5)?;
        e.ini_count += 1;
    }
    for _ in 0..n.regs {
        let key = r.s()?;
        let name = r.s()?;
        let value = r.bin()?;
        let cond = cond_full(r)?;
        r.winver()?;
        let hive = r.u32()?;
        r.u16()?;
        let vtype = r.u8()?;
        let options = r.set(12)?;
        if vtype > 6 {
            return Err("bad registry type".into());
        }
        e.regs.push(RegE { key, name, value, cond, hive, vtype, options });
    }
    for (count, out) in [(n.del, 0), (n.undel, 1)] {
        for _ in 0..count {
            let name = r.s()?;
            cond_full(r)?;
            r.winver()?;
            let kind = r.u8()?;
            let d = DelE { name, kind };
            if out == 0 {
                e.install_delete.push(d)
            } else {
                e.uninstall_delete.push(d)
            }
        }
    }
    let run_flags = if v >= ver(6, 0, 0) { 12 } else { 10 };
    for (count, un) in [(n.run, false), (n.unrun, true)] {
        for _ in 0..count {
            let name = r.s()?;
            let params = r.s()?;
            let workdir = r.s()?;
            let _run_once = r.s()?;
            let status = r.s()?;
            let _verb = r.s()?;
            let desc = r.s()?;
            let cond = cond_full(r)?;
            r.winver()?;
            let show_cmd = r.i32()?;
            let wait = r.u8()?;
            let options = r.set(run_flags)?;
            if wait > 2 {
                return Err("bad run entry".into());
            }
            let x = RunE { name, params, workdir, status, desc, cond, show_cmd, wait, options };
            if un {
                e.uninstall_run.push(x)
            } else {
                e.run.push(x)
            }
        }
    }
    // wizard images: either `len + BMP` or `count + (len + BMP)*`
    let (a, b) = read_images(r)?;
    e.wizard_image = a;
    e.wizard_small = b;
    Ok(e)
}

fn read_images(r: &mut Rd) -> R<(Vec<u8>, Vec<u8>)> {
    let save = r.p;
    let single = |r: &mut Rd| -> R<(Vec<u8>, Vec<u8>)> {
        let a = r.bin()?;
        let b = r.bin()?;
        if (a.is_empty() || a.starts_with(b"BM")) && (b.is_empty() || b.starts_with(b"BM")) && !(a.is_empty() && b.is_empty()) {
            Ok((a, b))
        } else {
            Err("no images".into())
        }
    };
    if let Ok(x) = single(r) {
        return Ok(x);
    }
    r.p = save;
    let list = |r: &mut Rd| -> R<Vec<u8>> {
        let n = r.u32()?;
        if n > 16 {
            return Err("bad image count".into());
        }
        let mut best: Vec<u8> = Vec::new();
        for _ in 0..n {
            let x = r.bin()?;
            if !x.starts_with(b"BM") {
                return Err("bad image".into());
            }
            if x.len() > best.len() {
                best = x;
            }
        }
        Ok(best)
    };
    let a = list(r)?;
    let b = list(r)?;
    Ok((a, b))
}

fn parse_data_entries(d: &[u8], v: u32, count: u32) -> R<Vec<DataE>> {
    let mut r = Rd { d, p: 0, unicode: false, cp: encoding_rs::WINDOWS_1252 };
    let mut out = Vec::with_capacity(count as usize);
    let sum = if v >= ver(5, 3, 9) { 20 } else { 16 };
    for _ in 0..count {
        let first_slice = r.u32()?;
        let _last_slice = r.u32()?;
        let chunk_start = r.u32()?;
        let offset = r.u64()?;
        let size = r.u64()?;
        let chunk_size = r.u64()?;
        let h = r.take(sum)?;
        let sha1 = if sum == 20 { Some(h.try_into().unwrap()) } else { None };
        r.skip(16)?; // timestamp + file version
        let options = r.set(if v >= ver(5, 5, 7) { 11 } else { 9 })?;
        out.push(DataE { first_slice, chunk_start, offset, size, chunk_size, sha1, options });
    }
    Ok(out)
}

// ---------------------------------------------------------------- decompression

/// Raw (headerless) LZMA1 / LZMA2 decoder over liblzma, as used by Inno.
struct RawLzma<Rd: BufRead> {
    inner: Rd,
    strm: lzma_sys::lzma_stream,
    _opts: Box<lzma_sys::lzma_options_lzma>,
    finished: bool,
}

// the stream only holds pointers into its own boxed state
unsafe impl<Rd: BufRead + Send> Send for RawLzma<Rd> {}

impl<Rd: BufRead> RawLzma<Rd> {
    fn new(inner: Rd, lzma2: bool, dict: u32, lc: u32, lp: u32, pb: u32) -> R<Self> {
        unsafe {
            let mut opts: Box<lzma_sys::lzma_options_lzma> = Box::new(std::mem::zeroed());
            if lzma_sys::lzma_lzma_preset(&mut *opts, 6) != 0 {
                return Err("liblzma: preset".into());
            }
            opts.dict_size = dict.max(4096);
            opts.lc = lc;
            opts.lp = lp;
            opts.pb = pb;
            let filters = [
                lzma_sys::lzma_filter {
                    id: if lzma2 { lzma_sys::LZMA_FILTER_LZMA2 } else { lzma_sys::LZMA_FILTER_LZMA1 },
                    options: &mut *opts as *mut _ as *mut std::ffi::c_void,
                },
                lzma_sys::lzma_filter { id: lzma_sys::LZMA_VLI_UNKNOWN, options: std::ptr::null_mut() },
            ];
            let mut strm: lzma_sys::lzma_stream = std::mem::zeroed();
            let r = lzma_sys::lzma_raw_decoder(&mut strm, filters.as_ptr());
            if r != lzma_sys::LZMA_OK {
                return Err(format!("liblzma: ошибка инициализации ({})", r));
            }
            Ok(RawLzma { inner, strm, _opts: opts, finished: false })
        }
    }
}

impl<Rd: BufRead> Read for RawLzma<Rd> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.finished {
            return Ok(0);
        }
        loop {
            let input = self.inner.fill_buf()?;
            let eof = input.is_empty();
            self.strm.next_in = input.as_ptr();
            self.strm.avail_in = input.len();
            self.strm.next_out = buf.as_mut_ptr();
            self.strm.avail_out = buf.len();
            let action = if eof { lzma_sys::LZMA_FINISH } else { lzma_sys::LZMA_RUN };
            let ret = unsafe { lzma_sys::lzma_code(&mut self.strm, action) };
            let consumed = input.len() - self.strm.avail_in;
            self.inner.consume(consumed);
            let produced = buf.len() - self.strm.avail_out;
            match ret {
                lzma_sys::LZMA_STREAM_END => {
                    self.finished = true;
                    return Ok(produced);
                }
                lzma_sys::LZMA_OK | lzma_sys::LZMA_BUF_ERROR => {
                    if produced > 0 {
                        return Ok(produced);
                    }
                    if eof {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "конец сжатых данных"));
                    }
                }
                e => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("ошибка LZMA ({})", e))),
            }
        }
    }
}

impl<Rd: BufRead> Drop for RawLzma<Rd> {
    fn drop(&mut self) {
        unsafe { lzma_sys::lzma_end(&mut self.strm) }
    }
}

fn lzma1_reader<Rd: BufRead>(inner: Rd, props: &[u8]) -> R<RawLzma<Rd>> {
    if props.len() < 5 || props[0] >= 9 * 5 * 5 {
        return Err("bad LZMA properties".into());
    }
    let (lc, lp, pb) = (props[0] as u32 % 9, (props[0] as u32 / 9) % 5, props[0] as u32 / 45);
    let dict = u32::from_le_bytes(props[1..5].try_into().unwrap());
    RawLzma::new(inner, false, dict, lc, lp, pb)
}

fn lzma2_reader<Rd: BufRead>(inner: Rd, prop: u8) -> R<RawLzma<Rd>> {
    if prop > 40 {
        return Err("bad LZMA2 properties".into());
    }
    let dict = if prop == 40 { u32::MAX } else { (2 | (prop as u32 & 1)) << (prop / 2 + 11) };
    RawLzma::new(inner, true, dict.min(1 << 30), 3, 0, 2)
}

/// Read everything; a raw LZMA stream without end marker ends in UnexpectedEof.
fn read_lenient(mut r: impl Read) -> R<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof || !out.is_empty() => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(out)
}

/// Compressed header block: CRC, stored size, compressed flag, then
/// 4096-byte sub-blocks each prefixed with CRC32.
fn read_block(f: &mut File, pos: u64) -> R<(Vec<u8>, u64)> {
    f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
    let mut h = [0u8; 9];
    f.read_exact(&mut h).map_err(|e| e.to_string())?;
    if crc32fast::hash(&h[4..9]) != u32::from_le_bytes(h[0..4].try_into().unwrap()) {
        return Err("повреждён заголовок установщика (CRC)".into());
    }
    let stored = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
    let compressed = h[8] != 0;
    let mut raw = vec![0u8; stored];
    f.read_exact(&mut raw).map_err(|e| e.to_string())?;
    let mut data = Vec::with_capacity(stored);
    let mut p = 0;
    while p < stored {
        let n = (stored - p - 4).min(4096);
        let crc = u32::from_le_bytes(raw[p..p + 4].try_into().unwrap());
        let chunk = &raw[p + 4..p + 4 + n];
        if crc32fast::hash(chunk) != crc {
            return Err("повреждён заголовок установщика (CRC блока)".into());
        }
        data.extend_from_slice(chunk);
        p += 4 + n;
    }
    let end = pos + 9 + stored as u64;
    if !compressed {
        return Ok((data, end));
    }
    let out = read_lenient(lzma1_reader(&data[5..], &data)?)?;
    Ok((out, end))
}

/// Inno 5.2.0+ "call instruction optimization" (x86 E8/E9 filter), decode
/// direction. Applied per 64 KiB block; `addr` = block offset in the file.
pub(crate) fn untransform_calls(buf: &mut [u8], addr: u32) {
    if buf.len() < 5 {
        return;
    }
    let size = buf.len() - 4;
    let mut i = 0usize;
    while i < size {
        if buf[i] == 0xE8 || buf[i] == 0xE9 {
            i += 1;
            if buf[i + 3] == 0x00 || buf[i + 3] == 0xFF {
                let a = addr.wrapping_add(i as u32 + 4) & 0xFF_FFFF;
                let mut rel = buf[i] as u32 | (buf[i + 1] as u32) << 8 | (buf[i + 2] as u32) << 16;
                rel = rel.wrapping_sub(a);
                if rel & 0x80_0000 != 0 {
                    buf[i + 3] = !buf[i + 3];
                }
                buf[i] = rel as u8;
                buf[i + 1] = (rel >> 8) as u8;
                buf[i + 2] = (rel >> 16) as u8;
            }
            i += 4;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
pub(crate) fn transform_calls(buf: &mut [u8], addr: u32) {
    if buf.len() < 5 {
        return;
    }
    let size = buf.len() - 4;
    let mut i = 0usize;
    while i < size {
        if buf[i] == 0xE8 || buf[i] == 0xE9 {
            i += 1;
            if buf[i + 3] == 0x00 || buf[i + 3] == 0xFF {
                let a = addr.wrapping_add(i as u32 + 4) & 0xFF_FFFF;
                let mut rel = buf[i] as u32 | (buf[i + 1] as u32) << 8 | (buf[i + 2] as u32) << 16;
                if rel & 0x80_0000 != 0 {
                    buf[i + 3] = !buf[i + 3];
                }
                rel = rel.wrapping_add(a);
                buf[i] = rel as u8;
                buf[i + 1] = (rel >> 8) as u8;
                buf[i + 2] = (rel >> 16) as u8;
            }
            i += 4;
        } else {
            i += 1;
        }
    }
}

/// Reads compressed chunk bytes that may continue across disk slices.
struct SliceReader {
    files: Vec<PathBuf>,
    idx: usize,
    cur: Option<BufReader<File>>,
    left_in_slice: u64,
}

impl SliceReader {
    fn open_slice(&mut self, idx: usize, pos: Option<u64>) -> io::Result<()> {
        let p = self.files.get(idx).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("нет файла тома {}", idx + 1)))?;
        let mut f = File::open(p).map_err(|e| io::Error::new(e.kind(), format!("{}: {}", p.display(), e)))?;
        let mut magic = [0u8; 12];
        f.read_exact(&mut magic)?;
        if &magic[..6] != b"idska3" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: не том Inno Setup", p.display())));
        }
        let total = u32::from_le_bytes(magic[8..12].try_into().unwrap()) as u64;
        let start = pos.unwrap_or(12);
        f.seek(SeekFrom::Start(start))?;
        self.left_in_slice = total.saturating_sub(start);
        self.cur = Some(BufReader::new(f));
        self.idx = idx;
        Ok(())
    }
}

impl Read for SliceReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.left_in_slice == 0 {
                let next = self.idx + 1;
                if next >= self.files.len() {
                    return Ok(0);
                }
                self.open_slice(next, None)?;
                continue;
            }
            let n = (buf.len() as u64).min(self.left_in_slice) as usize;
            let r = self.cur.as_mut().unwrap().read(&mut buf[..n])?;
            if r == 0 {
                self.left_in_slice = 0;
                continue;
            }
            self.left_in_slice -= r as u64;
            return Ok(r);
        }
    }
}

fn chunk_reader(raw: Box<dyn Read>, compressed: bool) -> R<Box<dyn Read>> {
    let mut raw = BufReader::with_capacity(1 << 20, raw);
    let mut magic = [0u8; 4];
    raw.read_exact(&mut magic).map_err(|e| format!("чтение блока данных: {}", e))?;
    if &magic != b"zlb\x1a" {
        return Err("неизвестный формат блока данных Inno".into());
    }
    if !compressed {
        return Ok(Box::new(raw));
    }
    let first = {
        let b = raw.fill_buf().map_err(|e| e.to_string())?;
        if b.is_empty() {
            return Err("пустой блок данных".into());
        }
        b.to_vec()
    };
    if first[0] == 0x78 {
        return Ok(Box::new(flate2::read::ZlibDecoder::new(raw)));
    }
    if first.starts_with(b"BZh") {
        return Err("сжатие bzip2 не поддерживается".into());
    }
    if first[0] <= 40 {
        raw.consume(1);
        return Ok(Box::new(lzma2_reader(raw, first[0])?));
    }
    let mut props = [0u8; 5];
    raw.read_exact(&mut props).map_err(|e| e.to_string())?;
    Ok(Box::new(lzma1_reader(raw, &props)?))
}

// ---------------------------------------------------------------- PE icon

fn extract_icon(exe_head: &[u8]) -> Option<Vec<u8>> {
    use editpe::{ResourceEntry, ResourceEntryName};
    let img = editpe::Image::parse(exe_head).ok()?;
    let res = img.resource_directory()?;
    let root = res.root();
    let first_data = |e: &ResourceEntry| -> Option<Vec<u8>> {
        match e {
            ResourceEntry::Table(t) => {
                let k = t.entries().first().cloned()?.clone();
                match t.get(k)? {
                    ResourceEntry::Data(d) => Some(d.data().to_vec()),
                    ResourceEntry::Table(t2) => {
                        let k2 = t2.entries().first().cloned()?.clone();
                        match t2.get(k2)? {
                            ResourceEntry::Data(d) => Some(d.data().to_vec()),
                            _ => None,
                        }
                    }
                }
            }
            ResourceEntry::Data(d) => Some(d.data().to_vec()),
        }
    };
    let groups = match root.get(ResourceEntryName::ID(14))? {
        ResourceEntry::Table(t) => t,
        _ => return None,
    };
    let gk = groups.entries().first().cloned()?.clone();
    let grp = first_data(groups.get(gk)?)?;
    let icons = match root.get(ResourceEntryName::ID(3))? {
        ResourceEntry::Table(t) => t,
        _ => return None,
    };
    if grp.len() < 6 {
        return None;
    }
    let n = u16::from_le_bytes([grp[4], grp[5]]) as usize;
    let mut imgs = Vec::new();
    for i in 0..n {
        let e = grp.get(6 + i * 14..6 + i * 14 + 14)?;
        let id = u16::from_le_bytes([e[12], e[13]]) as u32;
        let data = first_data(icons.get(ResourceEntryName::ID(id))?)?;
        imgs.push((e[..12].to_vec(), data));
    }
    let mut ico = vec![0, 0, 1, 0];
    ico.extend_from_slice(&(imgs.len() as u16).to_le_bytes());
    let mut off = 6 + 16 * imgs.len() as u32;
    for (hdr, data) in &imgs {
        ico.extend_from_slice(&hdr[..8]);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&off.to_le_bytes());
        off += data.len() as u32;
    }
    for (_, data) in &imgs {
        ico.extend_from_slice(data);
    }
    Some(ico)
}

// ---------------------------------------------------------------- code scan

const KNOWN_FONTS: &[&str] = &[
    "Candara", "Calibri", "Cambria", "Consolas", "Constantia", "Corbel", "Segoe UI", "Segoe Print", "Segoe Script", "Verdana", "Georgia",
    "Trebuchet MS", "Comic Sans MS", "Impact", "Franklin Gothic Medium", "Palatino Linotype", "Book Antiqua", "Century Gothic", "Arial Black",
    "Lucida Console", "Times New Roman", "Courier New", "Arial", "Tahoma",
];

fn code_strings(code: &[u8], cp: &'static encoding_rs::Encoding) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        let t = s.trim().to_string();
        if t.chars().count() >= 3 && !out.contains(&t) {
            out.push(t);
        }
    };
    // ANSI runs
    let mut i = 0;
    while i < code.len() {
        let st = i;
        while i < code.len() && (code[i] >= 0x20 || code[i] == b'\t') && code[i] != 0x7f {
            i += 1;
        }
        if i - st >= 3 {
            push(cp.decode_without_bom_handling(&code[st..i]).0.into_owned());
        }
        i += 1;
    }
    // UTF-16LE runs (Unicode builds keep string constants as UnicodeString)
    for start in 0..2 {
        let mut cur: Vec<u16> = Vec::new();
        let mut j = start;
        while j + 1 < code.len() {
            let w = u16::from_le_bytes([code[j], code[j + 1]]);
            let ok = (0x20..0xd800).contains(&w) && w != 0x7f && !(0x80..0xa0).contains(&w);
            if ok {
                cur.push(w);
            } else {
                if cur.len() >= 4 && cur.iter().any(|c| *c >= 0x100 || (*c as u8).is_ascii_alphabetic()) {
                    let s = String::from_utf16_lossy(&cur);
                    // skip runs that are really ANSI byte pairs
                    if s.chars().filter(|c| c.is_alphanumeric() || *c == ' ').count() * 10 >= s.chars().count() * 7 {
                        push(s);
                    }
                }
                cur.clear();
            }
            j += 2;
        }
    }
    out
}

fn is_archive_name(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    [".arc", ".7z", ".zip", ".smpk", ".tar", ".tar.gz", ".tar.xz", ".bin", ".001"].iter().any(|e| l.ends_with(e))
}

// ---------------------------------------------------------------- conversion

fn split_root(dest: &str) -> (String, String) {
    let d = dest.trim_start_matches('\\');
    if d.starts_with('{') {
        if let Some(end) = d.find('}') {
            let root = &d[..=end];
            let rest = d[end + 1..].trim_start_matches('\\');
            return (root.to_string(), rest.to_string());
        }
    }
    match d.rfind('\\') {
        Some(i) => (d[..i].to_string(), d[i + 1..].to_string()),
        None => (String::new(), d.to_string()),
    }
}

/// Split `value Key: v Key2: v2` where Inno parameters leaked into a value
/// because of a missing quote in the source script.
fn split_leaked(s: &str) -> (String, Vec<(String, String)>) {
    const KEYS: &[&str] = &["Name", "Filename", "Parameters", "WorkingDir", "IconFilename", "IconIndex", "Comment", "Check", "Tasks", "Components", "Flags", "Languages"];
    let mut cuts: Vec<(usize, usize, String)> = Vec::new(); // (start, value start, key)
    for k in KEYS {
        let pat = format!(" {}: ", k);
        let mut from = 0;
        while let Some(i) = s[from..].find(&pat) {
            let at = from + i;
            cuts.push((at, at + pat.len(), k.to_ascii_lowercase()));
            from = at + pat.len();
        }
    }
    if cuts.is_empty() {
        return (s.to_string(), vec![]);
    }
    cuts.sort();
    let head = s[..cuts[0].0].trim().to_string();
    let mut out = Vec::new();
    for (n, (_, vs, k)) in cuts.iter().enumerate() {
        let end = cuts.get(n + 1).map(|c| c.0).unwrap_or(s.len());
        out.push((k.clone(), s[*vs..end].trim().trim_matches('"').to_string()));
    }
    (head, out)
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let mut out = String::new();
    for c in t.chars() {
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out.trim_matches('_').chars().take(40).collect()
}

fn safe_rel(rel: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for part in rel.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => return None,
            x if x.contains(':') => return None,
            x => p.push(x),
        }
    }
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p)
    }
}

/// ISDone / skin / callback helpers that are useless without Pascal code.
fn is_script_helper(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    ["isdone.dll", "records.inf", "innocallback.dll", "callbackctrl.dll", "isskin.dll", "isskinu.dll", "botva2.dll", "wintb.dll", "b2p.dll", "iswin7.dll", "get_hw_caps.dll", "isgsg.dll", "issurface.dll"]
        .contains(&l.as_str())
        || l.ends_with(".cjstyles")
}

fn lang_of(l: &LangE) -> &'static str {
    let by_id = smk_core::messages::lang_from_langid(l.lang_id as u16);
    if l.lang_id != 0 && (by_id != "en" || l.lang_id & 0x3ff == 0x09) {
        return by_id;
    }
    smk_core::messages::normalize_lang(&l.name)
}

fn text_ext(b: &[u8]) -> &'static str {
    if b.starts_with(b"{\\rtf") {
        "rtf"
    } else {
        "txt"
    }
}

fn to_utf8_text(b: &[u8], cp: &'static encoding_rs::Encoding) -> Vec<u8> {
    if b.starts_with(b"{\\rtf") || std::str::from_utf8(b).is_ok() {
        b.to_vec()
    } else {
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice(cp.decode_without_bom_handling(b).0.as_bytes());
        v
    }
}

pub struct ExeImport {
    pub imported: Imported,
    pub project_path: PathBuf,
}

/// Import `setup.exe` into `out_dir` (project file + files/ tools/ images).
pub fn import(setup: &Path, out_dir: &Path, log: &dyn Fn(&str)) -> R<ExeImport> {
    let mut warn: Vec<String> = Vec::new();
    let mut f = File::open(setup).map_err(|e| format!("{}: {}", setup.display(), e))?;
    let flen = f.metadata().map_err(|e| e.to_string())?.len();
    let mut head = vec![0u8; flen.min(8 << 20) as usize];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;

    // loader offset table
    let magic = b"rDlPtS\xcd\xe6\xd7\x7b\x0b\x2a";
    let t = head.windows(magic.len()).position(|w| w == magic).ok_or("это не установщик Inno Setup 5.1.5+ (таблица смещений не найдена)")?;
    let rd32 = |o: usize| u32::from_le_bytes(head[o..o + 4].try_into().unwrap());
    if rd32(t + 12) != 1 {
        return Err(format!("неподдерживаемая версия таблицы смещений ({}) — Inno Setup 6.4+ пока не поддерживается", rd32(t + 12)));
    }
    let offset0 = rd32(t + 32) as u64;
    let offset1 = rd32(t + 36) as u64;

    f.seek(SeekFrom::Start(offset0)).map_err(|e| e.to_string())?;
    let mut id = [0u8; 64];
    f.read_exact(&mut id).map_err(|e| e.to_string())?;
    let id_str = String::from_utf8_lossy(&id).trim_end_matches('\0').to_string();
    let nums: Vec<u32> = id_str
        .split('(')
        .nth(1)
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    if !id_str.starts_with("Inno Setup Setup Data") || nums.len() < 3 {
        return Err(format!("неизвестный формат данных: {}", id_str));
    }
    let v = ver(nums[0], nums[1], nums[2]);
    let unicode = id_str.contains("(u)") || id_str.contains("u)") || nums[0] >= 6;
    log(&format!("{} ({})", id_str, if unicode { "Unicode" } else { "ANSI" }));
    if v < ver(5, 2, 5) {
        return Err("поддерживаются установщики Inno Setup 5.2.5 – 6.3.x".into());
    }
    if v >= ver(6, 4, 0) {
        return Err("Inno Setup 6.4+ пока не поддерживается".into());
    }

    let (b1, end1) = read_block(&mut f, offset0 + 64)?;
    let (b2, _) = read_block(&mut f, end1)?;

    // header strings: up to and including compiled code
    let n_before_license = 24
        + (v >= ver(5, 3, 8)) as usize
        + (v >= ver(5, 3, 10)) as usize
        + (v >= ver(5, 5, 0)) as usize
        + (v >= ver(5, 5, 6)) as usize
        + 2 * (v >= ver(5, 6, 1)) as usize
        + 2 * (v >= ver(6, 3, 0)) as usize;
    let sig = (ver(5, 2, 1)..ver(5, 3, 10)).contains(&v) as usize;
    let total_strings = n_before_license + 3 + sig + 1;

    let parse_all = |cp: &'static encoding_rs::Encoding| -> R<(Vec<Vec<u8>>, Entries, Counts)> {
        let mut r = Rd { d: &b1, p: 0, unicode, cp };
        let mut hs = Vec::new();
        for _ in 0..total_strings {
            hs.push(r.bin()?);
        }
        if !unicode {
            r.skip(32)?; // lead bytes
        }
        let mut c = [0u32; 16];
        for x in c.iter_mut() {
            *x = r.u32()?;
        }
        let n = Counts {
            langs: c[0],
            msgs: c[1],
            perms: c[2],
            types: c[3],
            comps: c[4],
            tasks: c[5],
            dirs: c[6],
            files: c[7],
            data: c[8],
            icons: c[9],
            ini: c[10],
            regs: c[11],
            del: c[12],
            undel: c[13],
            run: c[14],
            unrun: c[15],
        };
        if c.iter().any(|x| *x > 1_000_000) || n.langs == 0 {
            return Err("неверные счётчики записей".into());
        }
        let base_file_flags = if v >= ver(5, 2, 5) { 32 } else { 31 };
        let layouts = [
            Layout { file_flags: base_file_flags, icon_toast: v >= ver(6, 1, 0) },
            Layout { file_flags: base_file_flags + 2, icon_toast: v >= ver(6, 1, 0) },
            Layout { file_flags: base_file_flags, icon_toast: v < ver(6, 1, 0) },
            Layout { file_flags: base_file_flags + 2, icon_toast: v < ver(6, 1, 0) },
        ];
        let tail_start = r.p;
        for lay in layouts {
            for off in tail_start..(tail_start + 1024).min(b1.len()) {
                let mut rr = Rd { d: &b1, p: off, unicode, cp };
                if let Ok(e) = parse_entries(&mut rr, v, &n, lay) {
                    return Ok((hs, e, n));
                }
            }
        }
        Err("не удалось разобрать записи установщика (неизвестный вариант формата)".into())
    };
    let (_, e0, _) = parse_all(encoding_rs::WINDOWS_1252)?;
    let cp = if unicode {
        encoding_rs::UTF_16LE
    } else {
        let c = e0.langs.first().map(|l| l.codepage).unwrap_or(1252);
        codepage_enc(c)
    };
    let ansi_cp = if unicode {
        let cyr = e0.langs.first().map(|l| ["ru", "uk"].contains(&lang_of(l))).unwrap_or(false);
        codepage_enc(if cyr { 1251 } else { 1252 })
    } else {
        cp
    };
    let (hs, e, counts) = if unicode { parse_all(encoding_rs::WINDOWS_1252)? } else { parse_all(cp)? };
    let hrd = Rd { d: &[], p: 0, unicode, cp };
    let hstr = |i: usize| -> String { hs.get(i).map(|b| hrd.decode(b, unicode).unwrap_or_default()).unwrap_or_default() };
    let data = parse_data_entries(&b2, v, counts.data)?;
    log(&format!(
        "Записей: языков {}, задач {}, компонентов {}, файлов {}, ярлыков {}, реестр {}, запуск {}",
        e.langs.len(),
        e.tasks.len(),
        e.comps.len(),
        e.files.len(),
        e.icons.len(),
        e.regs.len(),
        e.run.len()
    ));

    std::fs::create_dir_all(out_dir).map_err(|er| format!("{}: {}", out_dir.display(), er))?;
    let wr = |name: &str, b: &[u8]| -> R<()> { std::fs::write(out_dir.join(name), b).map_err(|er| format!("{}: {}", name, er)) };

    // ---- project
    let mut p = Project::default();
    p.app.name = hstr(0);
    p.app.verb_name = hstr(1);
    if p.app.verb_name == format!("{} {}", hstr(0), hstr(9)) {
        p.app.verb_name.clear();
    }
    p.app.app_id = guid_or_raw(&hstr(2));
    p.app.copyright = hstr(3);
    p.app.publisher = hstr(4);
    p.app.url = hstr(5);
    p.app.support_url = hstr(7);
    p.app.updates_url = hstr(8);
    p.app.version = hstr(9);
    p.app.contact = hstr(21);
    p.app.comments = hstr(22);
    p.defines.insert("AppName".into(), p.app.name.clone());
    p.setup.default_dir = hstr(10);
    p.setup.default_group = hstr(11);
    let base = hstr(12);
    p.uninstall.files_dir = if hstr(13).is_empty() { "{app}".into() } else { hstr(13) };
    p.uninstall.display_name = hstr(14);
    p.uninstall.display_icon = hstr(15);
    if n_before_license >= 26 && hstr(25).eq_ignore_ascii_case("no") {
        p.uninstall.enabled = false;
    }
    p.output.base_name = if base.is_empty() || base.eq_ignore_ascii_case("setup") { "setup".into() } else { base.clone() };

    let raw = |i: usize| hs.get(i).cloned().unwrap_or_default();
    for (i, field, name) in [(n_before_license, 0, "license"), (n_before_license + 1, 1, "info_before"), (n_before_license + 2, 2, "info_after")] {
        let b = raw(i);
        if b.is_empty() {
            continue;
        }
        let file = format!("{}.{}", name, text_ext(&b));
        wr(&file, &to_utf8_text(&b, ansi_cp))?;
        match field {
            0 => p.setup.license_file = file,
            1 => p.setup.info_before_file = file,
            _ => p.setup.info_after_file = file,
        }
    }
    let code = raw(total_strings - 1);

    // languages
    p.interface.languages.clear();
    let mut lang_ids: Vec<&'static str> = Vec::new();
    for (k, l) in e.langs.iter().enumerate() {
        let id = lang_of(l);
        lang_ids.push(id);
        let mut le = Language { id: id.into(), ..Default::default() };
        for (b, kind, slot) in [(&l.license, "license", 0), (&l.info_before, "info_before", 1), (&l.info_after, "info_after", 2)] {
            if b.is_empty() {
                continue;
            }
            let file = format!("{}_{}.{}", kind, id, text_ext(b));
            let lcp = if l.codepage != 0 { codepage_enc(l.codepage) } else { ansi_cp };
            wr(&file, &to_utf8_text(b, lcp))?;
            match slot {
                0 => le.license_file = file,
                1 => le.info_before_file = file,
                _ => le.info_after_file = file,
            }
        }
        if !p.interface.languages.iter().any(|x| x.id == id) {
            p.interface.languages.push(le);
        } else if k > 0 {
            warn.push(format!("язык «{}» не поддерживается и заменён на {}", l.name, id));
        }
        if !["en", "ru", "uk"].contains(&id) {
            warn.push(format!("язык «{}» не поддерживается", l.name));
        }
    }
    let font_default = ["", "Tahoma", "MS Shell Dlg", "MS Shell Dlg 2", "MS Sans Serif", "Segoe UI"];
    if let Some(l) = e.langs.first() {
        if !font_default.iter().any(|x| x.eq_ignore_ascii_case(&l.dialog_font)) {
            p.interface.font = l.dialog_font.clone();
        }
    }

    // custom messages
    for m in &e.msgs {
        let key = if m.lang < 0 { "*".to_string() } else { lang_ids.get(m.lang as usize).copied().unwrap_or("*").to_string() };
        p.messages.entry(key).or_default().insert(m.name.clone(), m.value.clone());
    }

    let drop_check = |c: &str| -> String {
        let t = c.trim();
        if t.eq_ignore_ascii_case("CheckError") || t.eq_ignore_ascii_case("not ISDoneError") || t.eq_ignore_ascii_case("ISDoneError") {
            String::new()
        } else {
            t.to_string()
        }
    };
    let conds = |c: &Cond| Conditions { components: c.components.clone(), tasks: c.tasks.clone(), check: drop_check(&c.check) };
    let mut checks: Vec<String> = Vec::new();

    for t in &e.types {
        p.types.push(SetupType { name: t.name.clone(), description: t.desc.clone(), custom: t.custom });
        checks.push(drop_check(&t.check));
    }
    for c in &e.comps {
        p.components.push(Component {
            name: c.name.clone(),
            description: c.desc.clone(),
            types: c.types.split([' ', ',']).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect(),
            fixed: c.options & 1 != 0,
            exclusive: c.options & 8 != 0,
            checked: true,
            extra_disk_space: c.extra,
            ..Default::default()
        });
        checks.push(drop_check(&c.check));
    }
    for t in &e.tasks {
        p.tasks.push(Task {
            name: t.name.clone(),
            description: t.desc.clone(),
            group: t.group.clone(),
            exclusive: t.options & 1 != 0,
            unchecked: t.options & 2 != 0,
            components: t.components.clone(),
            check: drop_check(&t.check),
            ..Default::default()
        });
    }
    for d in &e.dirs {
        p.dirs.push(DirItem { name: d.name.clone(), uninstall: d.options & 1 == 0, when: conds(&d.cond) });
    }

    // ---- files: group into folders by destination root + conditions
    #[derive(Hash, PartialEq, Eq, Clone)]
    struct GKey {
        root: String,
        when: Conditions,
        uninstall: bool,
        overwrite: &'static str,
        delete_after: bool,
        attrs: String,
    }
    let mut groups: Vec<(GKey, String)> = Vec::new();
    let mut targets: HashMap<usize, Vec<PathBuf>> = HashMap::new(); // data index -> output paths
    let mut tools: Vec<String> = Vec::new();
    let mut tmp_files: Vec<String> = Vec::new();
    let mut external = 0;
    for fe in &e.files {
        if fe.ftype != 0 {
            continue;
        }
        if fe.location == u32::MAX || fe.location as usize >= data.len() {
            external += 1;
            continue;
        }
        let (root, rel) = split_root(&fe.dest);
        let Some(relp) = safe_rel(&rel) else {
            warn.push(format!("пропущен файл с некорректным путём: {}", fe.dest));
            continue;
        };
        if root.eq_ignore_ascii_case("{tmp}") {
            let name = relp.to_string_lossy().replace('\\', "/");
            tmp_files.push(name.clone());
            if is_script_helper(&name) {
                continue;
            }
            let out = Path::new("tools").join(&relp);
            targets.entry(fe.location as usize).or_default().push(out);
            tools.push(format!("tools\\{}", name.replace('/', "\\")));
            continue;
        }
        let opts = fe.options;
        let mut attrs = Vec::new();
        for (bit, name) in [(1u32, "readonly"), (2, "hidden"), (4, "system")] {
            if fe.attributes & bit != 0 {
                attrs.push(name);
            }
        }
        let key = GKey {
            root: root.clone(),
            when: conds(&fe.cond),
            uninstall: opts & (1 << 1) == 0,
            overwrite: if opts & (1 << 16) != 0 { "never" } else { "always" },
            delete_after: opts & (1 << 3) != 0,
            attrs: attrs.join(" "),
        };
        let gi = match groups.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let mut name = sanitize(root.trim_matches(|c| c == '{' || c == '}'));
                if name.is_empty() {
                    name = "other".into();
                }
                for extra in [&key.when.components, &key.when.tasks] {
                    let s = sanitize(extra);
                    if !s.is_empty() {
                        name = format!("{}_{}", name, s);
                    }
                }
                let mut uniq = name.clone();
                let mut k = 2;
                while groups.iter().any(|(_, n)| n.eq_ignore_ascii_case(&uniq)) {
                    uniq = format!("{}_{}", name, k);
                    k += 1;
                }
                groups.push((key.clone(), uniq));
                groups.len() - 1
            }
        };
        let out = Path::new("files").join(&groups[gi].1).join(&relp);
        targets.entry(fe.location as usize).or_default().push(out);
    }
    if external > 0 {
        warn.push(format!("{} внешних файлов (flags: external) пропущено — добавьте их вручную", external));
    }
    for (k, folder) in &groups {
        p.files.push(FileItem {
            source: format!("files\\{}\\*", folder),
            dest_dir: if k.root.is_empty() { "{app}".into() } else { k.root.clone() },
            recursive: true,
            empty_dirs: false,
            overwrite: k.overwrite.into(),
            uninstall: k.uninstall,
            delete_after_install: k.delete_after,
            attributes: k.attrs.clone(),
            when: k.when.clone(),
            ..Default::default()
        });
    }
    tools.dedup();
    for t in tools {
        p.tools.push(ToolFile { source: t, name: String::new() });
    }

    // ---- shortcuts, registry, run, delete
    let mut repaired = 0;
    for i in &e.icons {
        let mut sc = Shortcut {
            name: i.name.clone(),
            target: i.filename.clone(),
            params: i.params.clone(),
            workdir: i.workdir.clone(),
            icon: i.icon_file.clone(),
            icon_index: i.icon_index,
            comment: i.comment.clone(),
            ask: String::new(),
            when: conds(&i.cond),
        };
        // `Filename: "x IconFilename: y Check: z"` (quoting bug in the source script)
        let (target, leaked) = split_leaked(&sc.target);
        if !leaked.is_empty() {
            repaired += 1;
            sc.target = target;
            for (k, val) in leaked {
                match k.as_str() {
                    "iconfilename" if sc.icon.is_empty() => sc.icon = val,
                    "parameters" if sc.params.is_empty() => sc.params = val,
                    "workingdir" if sc.workdir.is_empty() => sc.workdir = val,
                    "comment" if sc.comment.is_empty() => sc.comment = val,
                    "tasks" if sc.when.tasks.is_empty() => sc.when.tasks = val,
                    "components" if sc.when.components.is_empty() => sc.when.components = val,
                    "check" if sc.when.check.is_empty() => sc.when.check = drop_check(&val),
                    _ => {}
                }
            }
        }
        p.shortcuts.push(sc);
    }
    if repaired > 0 {
        warn.push(format!("исправлено ярлыков с ошибкой кавычек в исходном скрипте: {} (параметры попали внутрь Filename)", repaired));
    }
    for r in &e.regs {
        let root = match r.hive & 0x7fff_ffff {
            0 => "HKCR",
            1 => "HKCU",
            2 => "HKLM",
            3 => "HKU",
            5 => "HKCC",
            _ => "HKA",
        };
        let vt = ["none", "string", "expandsz", "dword", "binary", "multisz", "qword"][r.vtype as usize];
        let data = if r.vtype == 4 {
            r.value.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
        } else {
            hrd.decode(&r.value, unicode).unwrap_or_default()
        };
        let o = r.options;
        let uninstall = if o & (1 << 3) != 0 {
            "deletekey"
        } else if o & (1 << 4) != 0 {
            "deletekeyifempty"
        } else if o & (1 << 1) != 0 {
            "deletevalue"
        } else {
            "none"
        };
        p.registry.push(RegistryItem {
            root: root.into(),
            subkey: r.key.clone(),
            value_type: vt.into(),
            value_name: r.name.clone(),
            value_data: data,
            uninstall: uninstall.into(),
            only_if_missing: o & 1 != 0,
            delete_key_first: o & (1 << 6) != 0,
            delete_value: o & (1 << 7) != 0,
            when: conds(&r.cond),
        });
    }
    let run_item = |r: &RunE| RunItem {
        filename: r.name.clone(),
        params: r.params.clone(),
        workdir: r.workdir.clone(),
        description: r.desc.clone(),
        status_msg: r.status.clone(),
        postinstall: r.options & (1 << 2) != 0,
        unchecked: r.options & (1 << 3) != 0,
        nowait: r.wait == 1,
        hidden: r.show_cmd == 0,
        shell_exec: r.options & 1 != 0,
        skip_if_silent: r.options & (1 << 4) != 0,
        skip_if_missing: r.options & (1 << 1) != 0,
        when: conds(&r.cond),
    };
    p.run = e.run.iter().map(run_item).collect();
    p.uninstall_run = e.uninstall_run.iter().map(run_item).collect();
    let del_kind = |k: u8| match k {
        0 => "files",
        2 => "dirifempty",
        _ => "filesandordirs",
    };
    p.install_delete = e.install_delete.iter().map(|d| DeleteItem { path: d.name.clone(), kind: del_kind(d.kind).into() }).collect();
    p.uninstall_delete = e.uninstall_delete.iter().map(|d| DeleteItem { path: d.name.clone(), kind: del_kind(d.kind).into() }).collect();
    if e.ini_count > 0 {
        warn.push(format!("{} записей [INI] не перенесено (не поддерживается)", e.ini_count));
    }

    // ---- images & icon
    if !e.wizard_image.is_empty() {
        wr("wizard.bmp", &e.wizard_image)?;
        p.interface.wizard_image = "wizard.bmp".into();
    }
    if !e.wizard_small.is_empty() {
        wr("wizard_small.bmp", &e.wizard_small)?;
        p.interface.wizard_small_image = "wizard_small.bmp".into();
    }
    if let Some(ico) = extract_icon(&head) {
        wr("setup.ico", &ico)?;
        p.interface.setup_icon = "setup.ico".into();
    }

    // ---- [Code]: ISDone archives, font, captions
    let strs = code_strings(&code, ansi_cp);
    if !code.is_empty() {
        wr("code_strings.txt", strs.join("\r\n").as_bytes())?;
        warn.push("секция [Code] не переносится; строки из неё сохранены в code_strings.txt".into());
    }
    let lower_code = String::from_utf8_lossy(&code).to_ascii_lowercase();
    let uses_isdone = lower_code.contains("isdone.dll") || lower_code.contains("isarcextract") || tmp_files.iter().any(|t| t.eq_ignore_ascii_case("isdone.dll"));
    let mut masks: Vec<String> = Vec::new();
    for s in &strs {
        let l = s.to_ascii_lowercase();
        if let Some(rest) = l.strip_prefix("{src}\\") {
            if is_archive_name(rest) {
                let orig = s[6..].to_string();
                if !masks.iter().any(|m| m.eq_ignore_ascii_case(&orig)) {
                    masks.push(orig);
                }
            }
        }
    }
    let src_dir = setup.parent().unwrap_or(Path::new("."));
    let stem = setup.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut found_names: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(src_dir) {
        for en in rd.flatten() {
            let n = en.file_name().to_string_lossy().to_string();
            let l = n.to_ascii_lowercase();
            let is_slice = l.starts_with(&format!("{}-", stem.to_ascii_lowercase())) && l.ends_with(".bin");
            if en.path().is_file() && is_archive_name(&n) && !is_slice {
                found_names.push(n);
            }
        }
    }
    found_names.sort();
    for m in &masks {
        p.archives.push(ArchiveItem { file: m.clone(), dest: "{app}".into(), ..Default::default() });
    }
    for n in &found_names {
        if !masks.iter().any(|m| smk_core::wildcard::matches(&m.to_ascii_lowercase(), &n.to_ascii_lowercase())) {
            p.archives.push(ArchiveItem { file: n.clone(), dest: "{app}".into(), ..Default::default() });
        }
    }
    if !p.archives.is_empty() {
        let names: Vec<String> = p.archives.iter().map(|a| a.file.clone()).collect();
        warn.push(format!("внешние архивы ({}) — назначение {{app}}; проверьте папки распаковки и положите архивы рядом с собранным setup.exe", names.join(", ")));
        let total: u64 = found_names.iter().filter_map(|n| std::fs::metadata(src_dir.join(n)).ok()).map(|m| m.len()).sum();
        if total > 0 {
            p.setup.extra_disk_space = total * 2;
        }
    }
    if p.archives.iter().any(|a| a.detect_format() == "arc") && !p.tools.iter().any(|t| t.source.to_ascii_lowercase().contains("unarc")) {
        warn.push(
            "для .arc нужен unarc.dll (или unarc.exe) из FreeArc: положите его в tools/ и добавьте в «Инструменты» (ISDone.dll из оригинала не используется)".into(),
        );
    }
    if uses_isdone {
        log("Обнаружен ISDone: распаковка архивов перенесена в раздел «Архивы»");
    }
    if p.interface.font.is_empty() {
        if let Some(fnt) = KNOWN_FONTS.iter().find(|f| strs.iter().any(|s| s == **f) && !font_default.contains(f) && **f != "Arial") {
            p.interface.font = fnt.to_string();
        }
    }
    let starts = |s: &str, pre: &[&str]| pre.iter().any(|x| s.starts_with(x));
    if let Some(s) = strs.iter().find(|s| starts(s, &["Вас приветствует", "Welcome to", "Ласкаво просимо", "Вітаємо"])) {
        p.messages.entry("*".into()).or_default().insert("WelcomeLabel1".into(), s.clone());
    }
    if let Some(s) = strs.iter().find(|s| starts(s, &["Завершение установки", "Completing", "Завершення"])) {
        p.messages.entry("*".into()).or_default().insert("FinishedHeadingLabel".into(), s.clone());
    }
    if checks.iter().any(|c| !c.is_empty()) || p.tasks.iter().any(|t| !t.check.is_empty()) {
        warn.push("функции Check из [Code] сохранены как есть; неизвестные функции при установке считаются истинными".into());
    }

    // ---- extract embedded files
    extract_files(&mut f, setup, &stem, offset1, &data, &targets, out_dir, v, log, &mut warn)?;

    let project_path = out_dir.join(format!("{}.smp", if base.is_empty() { "setup".to_string() } else { sanitize(&base) }));
    let mut p = p;
    p.simplify_shortcut_tasks();
    Ok(ExeImport { imported: Imported { project: p, warnings: warn }, project_path })
}

/// `{{GUID}` / `{GUID}` -> `{GUID}`; any other AppId is kept as is.
fn guid_or_raw(s: &str) -> String {
    let core = s.trim().trim_matches(|c| c == '{' || c == '}');
    if core.len() == 36 && core.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        format!("{{{}}}", core)
    } else {
        s.trim().to_string()
    }
}

fn codepage_enc(cp: u32) -> &'static encoding_rs::Encoding {
    let label = match cp {
        65001 => "utf-8".to_string(),
        936 => "gbk".into(),
        932 => "shift_jis".into(),
        949 => "euc-kr".into(),
        950 => "big5".into(),
        866 => "ibm866".into(),
        874 => "windows-874".into(),
        c => format!("windows-{}", c),
    };
    encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
}

#[allow(clippy::too_many_arguments)]
fn extract_files(
    f: &mut File,
    setup: &Path,
    stem: &str,
    offset1: u64,
    data: &[DataE],
    targets: &HashMap<usize, Vec<PathBuf>>,
    out_dir: &Path,
    v: u32,
    log: &dyn Fn(&str),
    warn: &mut Vec<String>,
) -> R<()> {
    // chunks: (slice, start) -> data indices sorted by offset
    let mut chunks: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, d) in data.iter().enumerate() {
        if targets.contains_key(&i) {
            chunks.entry((d.first_slice, d.chunk_start)).or_default().push(i);
        }
    }
    let total: u64 = chunks.values().flatten().map(|i| data[*i].size).sum();
    let mut done: u64 = 0;
    let mut last_pct = u64::MAX;
    let dir = setup.parent().unwrap_or(Path::new("."));
    let slices: Vec<PathBuf> = if offset1 == 0 {
        let mut v = Vec::new();
        for n in 1..=999 {
            let p = dir.join(format!("{}-{}.bin", stem, n));
            if !p.is_file() {
                break;
            }
            v.push(p);
        }
        v
    } else {
        vec![]
    };
    let mut bad = 0usize;
    for ((slice, start), mut idx) in chunks {
        idx.sort_by_key(|i| data[*i].offset);
        let d0 = &data[idx[0]];
        if d0.options & DATA_ENCRYPTED != 0 {
            warn.push(format!("{} зашифрованных файлов пропущено (пароль установщика)", idx.len()));
            continue;
        }
        let raw: Box<dyn Read> = if offset1 != 0 {
            let mut fh = f.try_clone().map_err(|e| e.to_string())?;
            fh.seek(SeekFrom::Start(offset1 + start as u64)).map_err(|e| e.to_string())?;
            Box::new(fh.take(4 + d0.chunk_size))
        } else {
            let mut sr = SliceReader { files: slices.clone(), idx: 0, cur: None, left_in_slice: 0 };
            sr.open_slice(slice as usize, Some(start as u64)).map_err(|e| format!("тома установщика: {}", e))?;
            Box::new(sr.take(4 + d0.chunk_size))
        };
        let mut rd = chunk_reader(raw, d0.options & DATA_COMPRESSED != 0)?;
        let mut pos: u64 = 0;
        for i in idx {
            let d = &data[i];
            if d.offset < pos {
                // same data twice in a chunk is not expected; skip
                continue;
            }
            io::copy(&mut (&mut rd).take(d.offset - pos), &mut io::sink()).map_err(|e| e.to_string())?;
            pos = d.offset;
            let outs: Vec<PathBuf> = targets[&i].iter().map(|p| out_dir.join(p)).collect();
            let mut files: Vec<File> = Vec::new();
            for o in &outs {
                if let Some(parent) = o.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
                }
                files.push(File::create(o).map_err(|e| format!("{}: {}", o.display(), e))?);
            }
            let mut hasher = sha1_smol::Sha1::new();
            let call_opt = d.options & DATA_CALL_OPT != 0;
            let mut left = d.size;
            let mut buf = vec![0u8; 0x10000];
            let mut addr: u32 = 0;
            while left > 0 {
                let n = left.min(buf.len() as u64) as usize;
                rd.read_exact(&mut buf[..n]).map_err(|e| format!("распаковка {}: {}", outs[0].display(), e))?;
                if call_opt && v >= ver(5, 2, 0) {
                    untransform_calls(&mut buf[..n], addr);
                }
                addr = addr.wrapping_add(n as u32);
                hasher.update(&buf[..n]);
                for fh in files.iter_mut() {
                    fh.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                }
                left -= n as u64;
                done += n as u64;
                let pct = done * 100 / total.max(1);
                if pct != last_pct {
                    last_pct = pct;
                    log(&format!("\rИзвлечение файлов: {}%", pct));
                }
            }
            pos += d.size;
            if let Some(h) = d.sha1 {
                if hasher.digest().bytes() != h {
                    bad += 1;
                    warn.push(format!("контрольная сумма не совпала: {}", outs[0].display()));
                }
            }
        }
    }
    log("");
    if bad > 0 {
        return Err(format!("{} файлов извлечено с ошибкой контрольной суммы", bad));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_transform_roundtrip() {
        let mut data: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        for k in (0..data.len() - 8).step_by(97) {
            data[k] = 0xE8;
            data[k + 4] = if k % 2 == 0 { 0 } else { 0xFF };
        }
        let orig = data.clone();
        for (bi, block) in data.chunks_mut(0x10000).enumerate() {
            transform_calls(block, (bi * 0x10000) as u32);
        }
        assert_ne!(orig, data);
        for (bi, block) in data.chunks_mut(0x10000).enumerate() {
            untransform_calls(block, (bi * 0x10000) as u32);
        }
        assert_eq!(orig, data);
    }

    #[test]
    fn set_sizes() {
        assert_eq!(set_bytes(5), 1);
        assert_eq!(set_bytes(9), 2);
        assert_eq!(set_bytes(17), 4);
        assert_eq!(set_bytes(32), 4);
        assert_eq!(set_bytes(34), 5);
    }

    #[test]
    fn leaked_params() {
        let (t, l) = split_leaked("https://x.io IconFilename: {app}\\icon.ico Check: CheckError");
        assert_eq!(t, "https://x.io");
        assert_eq!(l, vec![("iconfilename".to_string(), "{app}\\icon.ico".to_string()), ("check".to_string(), "CheckError".to_string())]);
        assert!(split_leaked("{app}\\game.exe").1.is_empty());
    }

    #[test]
    fn roots() {
        assert_eq!(split_root("{app}\\Cleo\\a.cs"), ("{app}".to_string(), "Cleo\\a.cs".to_string()));
        assert_eq!(split_root("{userdocs}\\X\\y.dat"), ("{userdocs}".to_string(), "X\\y.dat".to_string()));
        assert!(safe_rel("..\\x").is_none());
        assert_eq!(sanitize("voice\\rus or cheat"), "voice_rus_or_cheat");
    }
}
