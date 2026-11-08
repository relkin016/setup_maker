//! Compression codecs: LZMA2 (xz container, optional x86 BCJ), Zstandard
//! (long-distance matching, multithreaded), Brotli, Store.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    #[default]
    None,
    Lzma2,
    Zstd,
    Brotli,
}

impl Codec {
    pub fn from_name(s: &str) -> Codec {
        match s.trim().to_ascii_lowercase().as_str() {
            "lzma2" | "lzma" | "xz" => Codec::Lzma2,
            "zstd" | "zst" => Codec::Zstd,
            "brotli" | "br" => Codec::Brotli,
            _ => Codec::None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Codec::None => "none",
            Codec::Lzma2 => "lzma2",
            Codec::Zstd => "zstd",
            Codec::Brotli => "brotli",
        }
    }
}

/// Streaming decoder.
pub fn decoder<'a, R: Read + 'a>(codec: Codec, r: R) -> io::Result<Box<dyn Read + 'a>> {
    Ok(match codec {
        Codec::None => Box::new(r),
        Codec::Lzma2 => {
            // u64::MAX memlimit, concatenated streams allowed.
            let stream = xz2::stream::Stream::new_stream_decoder(u64::MAX, xz2::stream::CONCATENATED)
                .map_err(io::Error::other)?;
            Box::new(xz2::read::XzDecoder::new_stream(io::BufReader::with_capacity(1 << 20, r), stream))
        }
        Codec::Zstd => {
            let mut d = zstd::stream::read::Decoder::new(r)?;
            d.window_log_max(31)?;
            Box::new(d)
        }
        Codec::Brotli => Box::new(brotli::Decompressor::new(r, 1 << 20)),
    })
}

pub fn decompress_all(codec: Codec, data: &[u8], raw_size: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(raw_size as usize);
    decoder(codec, data)?.read_to_end(&mut out)?;
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct EncodeOpts {
    pub codec: Codec,
    pub level: u32,
    pub extreme: bool,
    pub dict_mb: u32,
    pub threads: u32,
    pub bcj: bool,
}

impl EncodeOpts {
    pub fn from_project(c: &crate::project::Compression, threads: u32) -> EncodeOpts {
        EncodeOpts {
            codec: Codec::from_name(&c.method),
            level: c.level,
            extreme: c.extreme,
            dict_mb: c.dict_mb.max(1),
            threads,
            bcj: c.bcj,
        }
    }
    /// Approximate encoder memory per thread in bytes.
    pub fn mem_per_thread(&self) -> u64 {
        let d = self.dict_mb as u64 * 1024 * 1024;
        match self.codec {
            Codec::Lzma2 => d * 11 + (64 << 20),
            Codec::Zstd => d * 3 + (64 << 20),
            Codec::Brotli => (1u64 << 24) * 6,
            Codec::None => 1 << 20,
        }
    }
}

/// Writer that must be finished explicitly.
pub trait FinishWrite: Write {
    fn finish_box(self: Box<Self>) -> io::Result<()>;
}

#[cfg(feature = "encode")]
mod enc {
    use super::*;

    struct Plain<W: Write>(W);
    impl<W: Write> Write for Plain<W> {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl<W: Write> FinishWrite for Plain<W> {
        fn finish_box(mut self: Box<Self>) -> io::Result<()> {
            self.0.flush()
        }
    }

    impl<W: Write> FinishWrite for xz2::write::XzEncoder<W> {
        fn finish_box(self: Box<Self>) -> io::Result<()> {
            let mut w = (*self).finish()?;
            w.flush()
        }
    }

    impl<'a, W: Write> FinishWrite for zstd::stream::write::Encoder<'a, W> {
        fn finish_box(self: Box<Self>) -> io::Result<()> {
            let mut w = (*self).finish()?;
            w.flush()
        }
    }

    struct Br<W: Write>(brotli::CompressorWriter<W>);
    impl<W: Write> Write for Br<W> {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl<W: Write> FinishWrite for Br<W> {
        fn finish_box(self: Box<Self>) -> io::Result<()> {
            let mut w = self.0.into_inner();
            w.flush()
        }
    }

    /// Streaming encoder. `bcj` is honoured for LZMA2 only.
    pub fn encoder<'a, W: Write + 'a>(o: &EncodeOpts, bcj: bool, w: W) -> io::Result<Box<dyn FinishWrite + 'a>> {
        Ok(match o.codec {
            Codec::None => Box::new(Plain(w)),
            Codec::Lzma2 => {
                let mut preset = o.level.min(9);
                if o.extreme {
                    preset |= 0x8000_0000; // LZMA_PRESET_EXTREME
                }
                let mut lz = xz2::stream::LzmaOptions::new_preset(preset).map_err(io::Error::other)?;
                let dict = (o.dict_mb as u64 * 1024 * 1024).clamp(1 << 16, 1536 << 20) as u32;
                lz.dict_size(dict);
                if o.level >= 9 {
                    lz.nice_len(273);
                }
                let mut filters = xz2::stream::Filters::new();
                if bcj {
                    filters.x86();
                }
                filters.lzma2(&lz);
                let stream = if o.threads > 1 {
                    let block = (dict as u64 * 3).max(8 << 20);
                    xz2::stream::MtStreamBuilder::new()
                        .threads(o.threads)
                        .block_size(block)
                        .filters(filters)
                        .check(xz2::stream::Check::Crc32)
                        .encoder()
                        .map_err(io::Error::other)?
                } else {
                    xz2::stream::Stream::new_stream_encoder(&filters, xz2::stream::Check::Crc32)
                        .map_err(io::Error::other)?
                };
                Box::new(xz2::write::XzEncoder::new_stream(w, stream))
            }
            Codec::Zstd => {
                let level = (o.level as i32).clamp(1, 22);
                let mut e = zstd::stream::write::Encoder::new(w, level)?;
                let wlog = (o.dict_mb.max(1) as u64 * 1024 * 1024).ilog2().clamp(17, 30);
                e.include_checksum(true)?;
                e.long_distance_matching(true)?;
                e.window_log(wlog)?;
                if o.threads > 1 {
                    e.multithread(o.threads)?;
                }
                Box::new(e)
            }
            Codec::Brotli => {
                let q = o.level.min(11);
                let lgwin = (o.dict_mb.max(1) as u64 * 1024 * 1024).ilog2().clamp(16, 24);
                Box::new(Br(brotli::CompressorWriter::new(w, 1 << 20, q, lgwin)))
            }
        })
    }
}

#[cfg(feature = "encode")]
pub use enc::encoder;
