//! Binary layout of setup.exe payload and of `.smpk` archives.
//!
//! setup.exe:
//! ```text
//! [stub PE][resources][stream data (segment 0)][header (zstd json)][footer 48 bytes]
//! ```
//! With disk spanning, stream data lives in `<base>-1.bin`, `<base>-2.bin`...
//! Stream offsets are "virtual": segments are concatenated logically.
//!
//! .smpk:
//! ```text
//! [magic 8][version u32][crc u32][header_len u64][header (zstd json)][stream data]
//! ```

use crate::codec::Codec;
use crate::project::{Conditions, Project};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Seek, SeekFrom};

pub const PAYLOAD_MAGIC: &[u8; 8] = b"SMKSETUP";
pub const SMPK_MAGIC: &[u8; 8] = b"SMPKARC1";
pub const FORMAT_VERSION: u32 = 1;
pub const FOOTER_LEN: usize = 48;

/// File attribute bits (Windows FILE_ATTRIBUTE_*).
pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_SYSTEM: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Footer {
    pub stub_len: u64,
    pub header_off: u64,
    pub header_len: u64,
    pub header_crc: u32,
    pub version: u32,
}

impl Footer {
    pub fn to_bytes(&self) -> [u8; FOOTER_LEN] {
        let mut b = [0u8; FOOTER_LEN];
        b[0..8].copy_from_slice(&self.stub_len.to_le_bytes());
        b[8..16].copy_from_slice(&self.header_off.to_le_bytes());
        b[16..24].copy_from_slice(&self.header_len.to_le_bytes());
        b[24..28].copy_from_slice(&self.header_crc.to_le_bytes());
        b[28..32].copy_from_slice(&self.version.to_le_bytes());
        // 32..40 reserved
        b[40..48].copy_from_slice(PAYLOAD_MAGIC);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Footer> {
        if b.len() != FOOTER_LEN || &b[40..48] != PAYLOAD_MAGIC {
            return None;
        }
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        Some(Footer {
            stub_len: u64_at(0),
            header_off: u64_at(8),
            header_len: u64_at(16),
            header_crc: u32::from_le_bytes(b[24..28].try_into().unwrap()),
            version: u32::from_le_bytes(b[28..32].try_into().unwrap()),
        })
    }

    /// Read the footer from the end of a file. `None` = no payload.
    pub fn read_from<R: Read + Seek>(r: &mut R) -> io::Result<Option<Footer>> {
        let len = r.seek(SeekFrom::End(0))?;
        if len < FOOTER_LEN as u64 {
            return Ok(None);
        }
        r.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
        let mut b = [0u8; FOOTER_LEN];
        r.read_exact(&mut b)?;
        Ok(Footer::from_bytes(&b))
    }
}

/// One solid compressed stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamInfo {
    pub codec: Codec,
    /// Virtual offset in the data space.
    pub offset: u64,
    pub size: u64,
    pub raw_size: u64,
    /// Indices into `files`, in stream order.
    pub files: Vec<u32>,
    #[serde(default, skip_serializing_if = "Conditions::is_empty")]
    pub when: Conditions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FileRec {
    /// Destination path with constants, e.g. `{app}\bin\a.dll`
    /// (in smpk: path relative to the extraction folder).
    pub dest: String,
    pub size: u64,
    /// FILETIME (100ns since 1601).
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub attrs: u32,
    #[serde(default)]
    pub crc: u32,
    pub stream: u32,
    /// Same content as an earlier file in the same stream (not stored again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dup_of: Option<u32>,
    /// 0 = always, 1 = if newer, 2 = never (only if missing).
    #[serde(default)]
    pub overwrite: u8,
    #[serde(default = "t")]
    pub uninstall: bool,
    #[serde(default)]
    pub delete_after: bool,
}

fn t() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DirRec {
    pub path: String,
    #[serde(default = "t")]
    pub uninstall: bool,
    #[serde(default, skip_serializing_if = "Conditions::is_empty")]
    pub when: Conditions,
}

/// An embedded blob (image, license text, tool, plugin).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResRec {
    pub name: String,
    /// Absolute offset in setup.exe.
    pub offset: u64,
    pub size: u64,
    pub raw_size: u64,
    pub codec: Codec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    /// File name next to setup.exe; empty = setup.exe itself.
    pub file: String,
    /// Absolute offset of the data inside that file.
    pub base: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Header {
    pub builder_version: String,
    pub manifest: Project,
    pub files: Vec<FileRec>,
    pub dirs: Vec<DirRec>,
    pub streams: Vec<StreamInfo>,
    pub resources: Vec<ResRec>,
    pub segments: Vec<Segment>,
    /// sha256-like hash of the password (hex), empty = none.
    #[serde(default)]
    pub password_hash: String,
    #[serde(default)]
    pub password_salt: String,
}

impl Header {
    pub fn resource(&self, name: &str) -> Option<&ResRec> {
        self.resources.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SmpkHeader {
    pub files: Vec<FileRec>,
    pub dirs: Vec<String>,
    /// Offsets relative to the start of the data area.
    pub streams: Vec<StreamInfo>,
    pub total_raw: u64,
}

/// Encode a header as zstd-compressed JSON.
pub fn encode_json<T: Serialize>(v: &T) -> io::Result<Vec<u8>> {
    let json = serde_json::to_vec(v).map_err(io::Error::other)?;
    zstd::bulk::compress(&json, 19)
}

pub fn decode_json<T: for<'de> Deserialize<'de>>(data: &[u8]) -> io::Result<T> {
    let mut raw = Vec::new();
    zstd::stream::read::Decoder::new(data)?.read_to_end(&mut raw)?;
    serde_json::from_slice(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn read_smpk_header<R: Read>(r: &mut R) -> io::Result<(SmpkHeader, u64)> {
    let mut pre = [0u8; 24];
    r.read_exact(&mut pre)?;
    if &pre[0..8] != SMPK_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an smpk archive"));
    }
    let crc = u32::from_le_bytes(pre[12..16].try_into().unwrap());
    let len = u64::from_le_bytes(pre[16..24].try_into().unwrap());
    if len > 1 << 30 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "smpk header too large"));
    }
    let mut h = vec![0u8; len as usize];
    r.read_exact(&mut h)?;
    if crc32fast::hash(&h) != crc {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "smpk header corrupted"));
    }
    Ok((decode_json(&h)?, 24 + len))
}

pub fn smpk_prefix(header_bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(24 + header_bytes.len());
    v.extend_from_slice(SMPK_MAGIC);
    v.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    v.extend_from_slice(&crc32fast::hash(header_bytes).to_le_bytes());
    v.extend_from_slice(&(header_bytes.len() as u64).to_le_bytes());
    v.extend_from_slice(header_bytes);
    v
}

/// Simple salted hash for the password page (FNV-1a based, 256 bit).
/// Not meant as strong crypto: Inno-style "password protection" deters casual use.
pub fn password_hash(password: &str, salt: &str) -> String {
    let mut out = String::new();
    let data: Vec<u8> = salt.bytes().chain(password.bytes()).collect();
    for lane in 0u64..4 {
        let mut h: u64 = 0xcbf29ce484222325 ^ lane.wrapping_mul(0x9E3779B97F4A7C15);
        for _ in 0..1000 {
            for &b in &data {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h ^= h >> 29;
        }
        out.push_str(&format!("{:016x}", h));
    }
    out
}
