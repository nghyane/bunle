use std::io::{self, Write};

use crate::format::{self, ImageFormat, MCZIndex, PageInfo, HEADER_SIZE, INDEX_ENTRY_SIZE, VERSION, COVER_PREFIX};

pub struct EncodedPage {
    pub data: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub format: ImageFormat,
}

/// Hard limits inherited from the v1 format.
pub const MAX_PAGES: usize = u16::MAX as usize;
pub const MAX_PAGE_BYTES: usize = u32::MAX as usize;
pub const MAX_ARCHIVE_BYTES: usize = u32::MAX as usize;

/// Pack pre-encoded pages into MCZ format.
/// cover=true wraps in RIFF/WebP container with MCZd chunk (polyglot).
///
/// Fails fast when v1 format limits would be exceeded.
pub fn pack(pages: &[EncodedPage], out: &mut impl Write, cover: bool) -> Result<MCZIndex, PackError> {
    if pages.len() > MAX_PAGES {
        return Err(PackError::TooManyPages(pages.len()));
    }
    for (i, p) in pages.iter().enumerate() {
        if p.data.len() > MAX_PAGE_BYTES {
            return Err(PackError::PageTooLarge(i, p.data.len()));
        }
    }

    let page_count = pages.len() as u16;
    let total_data: usize = pages.iter().map(|p| p.data.len()).sum();
    let mcz_size = HEADER_SIZE + pages.len() * INDEX_ENTRY_SIZE + total_data;
    let prefix = if cover { COVER_PREFIX } else { 0 };
    let archive_size = prefix + mcz_size + if cover { mcz_size % 2 } else { 0 };
    if archive_size > MAX_ARCHIVE_BYTES {
        return Err(PackError::ArchiveTooLarge(archive_size));
    }
    let data_start = (prefix + HEADER_SIZE + pages.len() * INDEX_ENTRY_SIZE) as u32;

    if cover {
        let (w, h) = pages.first().map_or((1u16, 1u16), |p| (p.width, p.height));

        // RIFF header: "RIFF" + size + "WEBP"
        let riff_body_size = 4 + 26 + 8 + mcz_size + (mcz_size % 2); // WEBP + VP8L chunk(26) + MCZd header(8) + data + pad
        out.write_all(b"RIFF").map_err(PackError::Io)?;
        out.write_all(&(riff_body_size as u32).to_le_bytes()).map_err(PackError::Io)?;
        out.write_all(b"WEBP").map_err(PackError::Io)?;

        // VP8L chunk: "VP8L" + size(17) + data(17) + 1 pad byte
        out.write_all(b"VP8L").map_err(PackError::Io)?;
        out.write_all(&17u32.to_le_bytes()).map_err(PackError::Io)?;
        let mut vp8l = format::VP8L_DATA;
        let val = (w as u32 - 1) | ((h as u32 - 1) << 14);
        vp8l[1..5].copy_from_slice(&val.to_le_bytes());
        out.write_all(&vp8l).map_err(PackError::Io)?;
        out.write_all(&[0u8]).map_err(PackError::Io)?; // pad to even

        // MCZd chunk header
        out.write_all(b"MCZd").map_err(PackError::Io)?;
        out.write_all(&(mcz_size as u32).to_le_bytes()).map_err(PackError::Io)?;
    }

    let mut index_pages = Vec::with_capacity(pages.len());
    let mut offset = data_start;
    for (i, page) in pages.iter().enumerate() {
        index_pages.push(PageInfo {
            index: i as u16,
            offset,
            size: page.data.len() as u32,
            width: page.width,
            height: page.height,
            format: page.format,
        });
        offset += page.data.len() as u32;
    }

    let mut header = Vec::with_capacity(HEADER_SIZE + pages.len() * INDEX_ENTRY_SIZE);
    format::write_header(&mut header, page_count);
    for p in &index_pages {
        format::write_index_entry(&mut header, p);
    }
    out.write_all(&header).map_err(PackError::Io)?;

    for page in pages {
        out.write_all(&page.data).map_err(PackError::Io)?;
    }

    // RIFF pad byte if MCZd chunk data is odd
    if cover && mcz_size % 2 != 0 {
        out.write_all(&[0u8]).map_err(PackError::Io)?;
    }

    Ok(MCZIndex {
        version: VERSION,
        pages: index_pages,
    })
}

#[derive(Debug)]
pub enum PackError {
    Io(io::Error),
    TooManyPages(usize),
    PageTooLarge(usize, usize),
    ArchiveTooLarge(usize),
    NoImages,
    Image(String, String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::TooManyPages(n) => write!(f, "too many pages: {n} > {MAX_PAGES}"),
            Self::PageTooLarge(i, n) => write!(f, "page {i} too large: {n} bytes > {MAX_PAGE_BYTES}"),
            Self::ArchiveTooLarge(n) => write!(f, "archive too large: {n} bytes > {MAX_ARCHIVE_BYTES}"),
            Self::NoImages => write!(f, "no images found in directory"),
            Self::Image(path, e) => write!(f, "failed to process {path}: {e}"),
        }
    }
}

impl std::error::Error for PackError {}

// ── CLI-only: encode + pack from directory ──────────────────────────

#[cfg(feature = "cli")]
pub use cli::*;

#[cfg(feature = "cli")]
mod cli {
    use super::*;
    use rayon::prelude::*;
    use std::path::Path;

    /// Pack images from a directory into MCZ.
    /// Compressed formats (WebP/JPEG/JXL) → passthrough (zero quality loss).
    /// Uncompressed formats (PNG/BMP/TIFF) → encode to WebP at `quality`.
    pub fn pack_dir(dir: &Path, output: &Path, quality: u8, cover: bool) -> Result<MCZIndex, PackError> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(PackError::Io)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                matches!(
                    e.path().extension().and_then(|s| s.to_str()),
                    Some("png" | "jpg" | "jpeg" | "webp" | "jxl" | "bmp" | "tiff")
                )
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        if entries.is_empty() {
            return Err(PackError::NoImages);
        }

        let pages: Vec<EncodedPage> = entries
            .par_iter()
            .map(|entry| {
                let path = entry.path();
                encode_page(&path, quality)
                    .map_err(|e| PackError::Image(path.display().to_string(), e))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut file = std::fs::File::create(output).map_err(PackError::Io)?;
        super::pack(&pages, &mut file, cover)
    }

    fn encode_page(path: &Path, quality: u8) -> Result<EncodedPage, String> {
        let raw = std::fs::read(path).map_err(|e| e.to_string())?;

        let reader = image::ImageReader::new(std::io::Cursor::new(&raw))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let detected = reader.format();

        // Already compressed → passthrough (no quality loss)
        match detected {
            Some(image::ImageFormat::WebP) | Some(image::ImageFormat::Jpeg) => {
                let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
                let fmt = if matches!(detected, Some(image::ImageFormat::WebP)) {
                    ImageFormat::WebP
                } else {
                    ImageFormat::Jpeg
                };
                return Ok(EncodedPage {
                    data: raw,
                    width: width as u16,
                    height: height as u16,
                    format: fmt,
                });
            }
            _ => {}
        }

        // Uncompressed (PNG/BMP/etc) → decode and encode to WebP
        let img = image::load_from_memory(&raw).map_err(|e| e.to_string())?;
        let (width, height) = (img.width(), img.height());
        let rgb = img.to_rgb8();
        let encoder = webp::Encoder::new(rgb.as_raw(), webp::PixelLayout::Rgb, width, height);
        let data = encoder.encode(quality as f32).to_vec();

        Ok(EncodedPage {
            data,
            width: width as u16,
            height: height as u16,
            format: ImageFormat::WebP,
        })
    }
}
