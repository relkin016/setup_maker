//! Unpacked size of external archives, read from their headers/indexes
//! without decompressing the data (disk space estimate in the wizard).
//!
//! Handled here (no extra dependencies): smpk, tar, tar.gz, tar.xz, tar.zst.
//! zip / 7z are handled by the stub (it owns those crates); tar.br and
//! FreeArc have no size index -> `None` (caller falls back to a guess).

use crate::format;
use std::io::{Read, Seek, SeekFrom};

/// Exact (or close) unpacked size; `None` if the format has no index or
/// the file does not look like `fmt`.
pub fn raw_size<R: Read + Seek>(fmt: &str, r: &mut R) -> Option<u64> {
    match fmt {
        "smpk" => {
            r.seek(SeekFrom::Start(0)).ok()?;
            let (h, _) = format::read_smpk_header(r).ok()?;
            // duplicates are not stored, but they are written to disk
            Some(h.files.iter().map(|f| f.size).sum())
        }
        "tar" => r.seek(SeekFrom::End(0)).ok(),
        "tar.gz" => gzip_size(r),
        "tar.xz" => xz_size(r),
        "tar.zst" => zstd_size(r),
        _ => None,
    }
}

fn read_at<R: Read + Seek>(r: &mut R, pos: u64, n: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(pos)).ok()?;
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).ok()?;
    Some(b)
}

/// gzip: ISIZE of the last member (size mod 4 GiB).
fn gzip_size<R: Read + Seek>(r: &mut R) -> Option<u64> {
    let len = r.seek(SeekFrom::End(0)).ok()?;
    if len < 18 || read_at(r, 0, 2)? != [0x1f, 0x8b] {
        return None;
    }
    let t = read_at(r, len - 4, 4)?;
    let mut s = u32::from_le_bytes(t.try_into().ok()?) as u64;
    // > 4 GiB wraps around: unpacked data is never smaller than packed
    while s < len {
        s += 1 << 32;
    }
    Some(s)
}

fn vli(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let c = *b.get(*pos)?;
        *pos += 1;
        v |= ((c & 0x7f) as u64) << (7 * i);
        if c & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// xz: sum of uncompressed sizes from the index of every stream.
fn xz_size<R: Read + Seek>(r: &mut R) -> Option<u64> {
    let mut end = r.seek(SeekFrom::End(0)).ok()?;
    let mut total = 0u64;
    let mut streams = 0;
    while end > 0 {
        // stream padding (zero bytes, multiple of 4)
        while end >= 4 && read_at(r, end - 4, 4)? == [0, 0, 0, 0] {
            end -= 4;
        }
        if end < 24 {
            return None;
        }
        let foot = read_at(r, end - 12, 12)?;
        if &foot[10..12] != b"YZ" {
            return None;
        }
        let bsize = (u32::from_le_bytes(foot[4..8].try_into().ok()?) as u64 + 1) * 4;
        if bsize > 256 << 20 {
            return None;
        }
        let idx_start = (end - 12).checked_sub(bsize)?;
        let idx = read_at(r, idx_start, bsize as usize)?;
        if idx[0] != 0 {
            return None;
        }
        let mut pos = 1;
        let n = vli(&idx, &mut pos)?;
        let mut blocks = 0u64;
        for _ in 0..n {
            let unpadded = vli(&idx, &mut pos)?;
            total = total.checked_add(vli(&idx, &mut pos)?)?;
            blocks = blocks.checked_add((unpadded + 3) & !3)?;
        }
        end = idx_start.checked_sub(blocks)?.checked_sub(12)?;
        if read_at(r, end, 6)? != [0xfd, b'7', b'z', b'X', b'Z', 0] {
            return None;
        }
        streams += 1;
    }
    (streams > 0).then_some(total)
}

/// zstd: sum of Frame_Content_Size of all frames (walks block headers to
/// find frame boundaries). `None` if any frame lacks the size field.
fn zstd_size<R: Read + Seek>(r: &mut R) -> Option<u64> {
    let len = r.seek(SeekFrom::End(0)).ok()?;
    let mut pos = 0u64;
    let mut total = 0u64;
    let mut frames = 0;
    while pos < len {
        let h = read_at(r, pos, 4)?;
        let magic = u32::from_le_bytes(h.try_into().ok()?);
        if magic & 0xffff_fff0 == 0x184d_2a50 {
            let s = read_at(r, pos + 4, 4)?;
            pos += 8 + u32::from_le_bytes(s.try_into().ok()?) as u64;
            continue;
        }
        if magic != 0xfd2f_b528 {
            return None;
        }
        let fhd = read_at(r, pos + 4, 1)?[0];
        let single = fhd & 0x20 != 0;
        let fcs_len = match fhd >> 6 {
            0 if single => 1,
            0 => return None,
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let did_len = [0u64, 1, 2, 4][(fhd & 3) as usize];
        let fcs_pos = pos + 5 + if single { 0 } else { 1 } + did_len;
        let f = read_at(r, fcs_pos, fcs_len)?;
        let mut v = [0u8; 8];
        v[..fcs_len].copy_from_slice(&f);
        let mut size = u64::from_le_bytes(v);
        if fcs_len == 2 {
            size += 256;
        }
        total = total.checked_add(size)?;
        // blocks
        let mut p = fcs_pos + fcs_len as u64;
        loop {
            let b = read_at(r, p, 3)?;
            let bh = b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16;
            let csize = match (bh >> 1) & 3 {
                0 | 2 => (bh >> 3) as u64,
                1 => 1,
                _ => return None,
            };
            p += 3 + csize;
            if bh & 1 != 0 {
                break;
            }
            if p > len {
                return None;
            }
        }
        if fhd & 4 != 0 {
            p += 4;
        }
        pos = p;
        frames += 1;
    }
    (frames > 0 && pos == len).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn xz_single_and_concatenated() {
        let a = data(300_000);
        let mut e = xz2::write::XzEncoder::new(Vec::new(), 6);
        e.write_all(&a).unwrap();
        let mut x = e.finish().unwrap();
        assert_eq!(raw_size("tar.xz", &mut Cursor::new(&x)), Some(300_000));
        let mut e = xz2::write::XzEncoder::new(Vec::new(), 1);
        e.write_all(&a[..1000]).unwrap();
        x.extend_from_slice(&[0, 0, 0, 0]);
        x.extend(e.finish().unwrap());
        assert_eq!(raw_size("tar.xz", &mut Cursor::new(&x)), Some(301_000));
        assert_eq!(raw_size("tar.xz", &mut Cursor::new(&a)), None);
    }

    #[test]
    fn zstd_frames() {
        let a = data(1_000_000);
        let mut z = zstd::bulk::compress(&a, 3).unwrap();
        assert_eq!(raw_size("tar.zst", &mut Cursor::new(&z)), Some(1_000_000));
        z.extend(zstd::bulk::compress(&a[..500], 3).unwrap());
        assert_eq!(raw_size("tar.zst", &mut Cursor::new(&z)), Some(1_000_500));
        // streaming without a known size -> no content size
        let mut e = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        e.write_all(&a).unwrap();
        let s = e.finish().unwrap();
        assert_eq!(raw_size("tar.zst", &mut Cursor::new(&s)), None);
    }

    #[test]
    fn gzip_trailer() {
        let mut g = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        g.extend_from_slice(&[0u8; 20]);
        g.extend_from_slice(&123_456u32.to_le_bytes());
        assert_eq!(raw_size("tar.gz", &mut Cursor::new(&g)), Some(123_456));
    }
}
