//! Reading the payload appended to setup.exe (and external disk slices).

use smk_core::codec;
use smk_core::format::{self, Footer, Header, StreamInfo};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub struct Payload {
    pub exe: PathBuf,
    pub src_dir: PathBuf,
    pub footer: Footer,
    pub header: Header,
}

impl Payload {
    /// `Ok(None)` = the exe has no payload (it is an uninstaller copy).
    pub fn open(exe: &Path) -> io::Result<Option<Payload>> {
        let mut f = File::open(exe)?;
        let Some(footer) = Footer::read_from(&mut f)? else { return Ok(None) };
        if footer.version != format::FORMAT_VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported payload version"));
        }
        f.seek(SeekFrom::Start(footer.header_off))?;
        let mut h = vec![0u8; footer.header_len as usize];
        f.read_exact(&mut h)?;
        if crc32fast::hash(&h) != footer.header_crc {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header crc mismatch"));
        }
        let header: Header = format::decode_json(&h)?;
        let src_dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        Ok(Some(Payload { exe: exe.to_path_buf(), src_dir, footer, header }))
    }

    pub fn resource(&self, name: &str) -> Option<Vec<u8>> {
        let r = self.header.resource(name)?;
        let mut f = File::open(&self.exe).ok()?;
        f.seek(SeekFrom::Start(r.offset)).ok()?;
        let mut buf = vec![0u8; r.size as usize];
        f.read_exact(&mut buf).ok()?;
        codec::decompress_all(r.codec, &buf, r.raw_size).ok()
    }

    pub fn has_resource(&self, name: &str) -> bool {
        self.header.resource(name).is_some()
    }

    /// The stub part of the exe (used as the uninstaller).
    pub fn stub_bytes(&self) -> io::Result<Vec<u8>> {
        let mut f = File::open(&self.exe)?;
        let mut v = vec![0u8; self.footer.stub_len as usize];
        f.read_exact(&mut v)?;
        Ok(v)
    }

    /// Decoder for one stream. `locate` is asked for the folder containing a
    /// missing slice file (disk prompt); return `None` to abort.
    pub fn stream_reader<'a>(
        &'a self,
        s: &StreamInfo,
        locate: &'a mut dyn FnMut(&str, usize) -> Option<PathBuf>,
    ) -> io::Result<Box<dyn Read + 'a>> {
        let raw = SegReader::new(self, s.offset, s.size, locate)?;
        codec::decoder(s.codec, raw)
    }
}

/// Reads a virtual range of the data space spanning several segments.
pub struct SegReader<'a> {
    p: &'a Payload,
    pos: u64,
    end: u64,
    cur: Option<(usize, File, u64)>, // (segment index, file, remaining in segment)
    locate: &'a mut dyn FnMut(&str, usize) -> Option<PathBuf>,
    dirs: Vec<PathBuf>,
}

impl<'a> SegReader<'a> {
    fn new(
        p: &'a Payload,
        offset: u64,
        size: u64,
        locate: &'a mut dyn FnMut(&str, usize) -> Option<PathBuf>,
    ) -> io::Result<Self> {
        Ok(SegReader { p, pos: offset, end: offset + size, cur: None, locate, dirs: vec![p.src_dir.clone()] })
    }

    fn open_at(&mut self, pos: u64) -> io::Result<()> {
        let mut start = 0u64;
        for (i, seg) in self.p.header.segments.iter().enumerate() {
            if pos < start + seg.size {
                let local = pos - start;
                let path = if seg.file.is_empty() {
                    self.p.exe.clone()
                } else {
                    self.find_slice(&seg.file, i)?
                };
                let mut f = File::open(&path)?;
                f.seek(SeekFrom::Start(seg.base + local))?;
                self.cur = Some((i, f, seg.size - local));
                return Ok(());
            }
            start += seg.size;
        }
        Err(io::Error::new(io::ErrorKind::UnexpectedEof, "offset beyond data"))
    }

    fn find_slice(&mut self, name: &str, disk: usize) -> io::Result<PathBuf> {
        loop {
            for d in &self.dirs {
                let c = d.join(name);
                if c.is_file() {
                    return Ok(c);
                }
            }
            match (self.locate)(name, disk) {
                Some(dir) => self.dirs.insert(0, dir),
                None => return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            }
        }
    }
}

impl Read for SegReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.end || buf.is_empty() {
            return Ok(0);
        }
        if self.cur.as_ref().map(|c| c.2 == 0).unwrap_or(true) {
            self.open_at(self.pos)?;
        }
        let (_, f, left) = self.cur.as_mut().unwrap();
        let want = (buf.len() as u64).min(*left).min(self.end - self.pos) as usize;
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated setup data"));
        }
        *left -= n as u64;
        self.pos += n as u64;
        Ok(n)
    }
}

