mod format;
mod pack;
mod unpack;
mod validate;

pub use format::{ImageFormat, MCZIndex, PageInfo, ParseError, HEADER_SIZE, INDEX_ENTRY_SIZE, COVER_PREFIX};
pub use pack::{EncodedPage, pack, PackError, MAX_PAGES, MAX_PAGE_BYTES, MAX_ARCHIVE_BYTES};
pub use unpack::{unpack, UnpackError};
pub use validate::{validate, ValidateError, ValidateReport};
#[cfg(feature = "cli")]
pub use pack::pack_dir;

/// Read index from MCZ data (supports plain MCZ and WebP+MCZ polyglot).
pub fn read_index(data: &[u8]) -> Result<MCZIndex, ParseError> {
    let off = format::mcz_offset(data)?;
    let (version, page_count) = format::parse_header(data, off)?;
    let index = format::parse_index(data, off, page_count)?;
    Ok(MCZIndex {
        version,
        pages: index,
    })
}

/// Extract raw page data (WebP/JPEG bytes) from MCZ.
pub fn extract_page<'a>(data: &'a [u8], index: &MCZIndex, page: usize) -> Result<&'a [u8], ExtractError> {
    let info = index.pages.get(page).ok_or(ExtractError::PageOutOfRange)?;
    let start = info.offset as usize;
    let end = start + info.size as usize;
    if end > data.len() {
        return Err(ExtractError::DataTruncated);
    }
    Ok(&data[start..end])
}

#[derive(Debug)]
pub enum ExtractError {
    PageOutOfRange,
    DataTruncated,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PageOutOfRange => write!(f, "page index out of range"),
            Self::DataTruncated => write!(f, "MCZ data truncated"),
        }
    }
}

impl std::error::Error for ExtractError {}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let pages = vec![
            EncodedPage {
                data: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
                width: 690,
                height: 1024,
                format: ImageFormat::Jpeg,
            },
            EncodedPage {
                data: vec![0x52, 0x49, 0x46, 0x46],
                width: 690,
                height: 980,
                format: ImageFormat::WebP,
            },
        ];

        let mut buf = Vec::new();
        let index = pack(&pages, &mut buf, false).unwrap();
        assert_eq!(index.pages.len(), 2);
        assert_eq!(index.version, 1);

        // Read index back
        let index2 = read_index(&buf).unwrap();
        assert_eq!(index2.pages.len(), 2);
        assert_eq!(index2.pages[0].width, 690);
        assert_eq!(index2.pages[0].height, 1024);
        assert_eq!(index2.pages[0].format, ImageFormat::Jpeg);
        assert_eq!(index2.pages[1].width, 690);
        assert_eq!(index2.pages[1].height, 980);
        assert_eq!(index2.pages[1].format, ImageFormat::WebP);

        // Data offset = 8 header + 2 * 16 index = 40
        assert_eq!(index2.pages[0].offset, 40);
        assert_eq!(index2.pages[0].size, 6);
        assert_eq!(index2.pages[1].offset, 46);
        assert_eq!(index2.pages[1].size, 4);

        // Extract pages
        let p0 = extract_page(&buf, &index2, 0).unwrap();
        assert_eq!(p0, &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);

        let p1 = extract_page(&buf, &index2, 1).unwrap();
        assert_eq!(p1, &[0x52, 0x49, 0x46, 0x46]);

        // Out of range
        assert!(extract_page(&buf, &index2, 5).is_err());
    }

    #[test]
    fn single_page() {
        let pages = vec![EncodedPage {
            data: vec![1, 2, 3],
            width: 100,
            height: 200,
            format: ImageFormat::WebP,
        }];

        let mut buf = Vec::new();
        pack(&pages, &mut buf, false).unwrap();

        let index = read_index(&buf).unwrap();
        assert_eq!(index.pages.len(), 1);
        assert_eq!(index.pages[0].offset, 24); // 8 + 1*16
        assert_eq!(extract_page(&buf, &index, 0).unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn polyglot_riff_chunk() {
        let pages = vec![EncodedPage {
            data: vec![1, 2, 3],
            width: 800,
            height: 1200,
            format: ImageFormat::WebP,
        }];

        let mut buf = Vec::new();
        pack(&pages, &mut buf, true).unwrap();

        // Starts with RIFF (WebP)
        assert_eq!(&buf[0..4], b"RIFF");
        assert_eq!(&buf[8..12], b"WEBP");

        // RIFF size matches file size
        let riff_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        assert_eq!(8 + riff_size, buf.len());

        // MCZd chunk inside RIFF at offset 38
        assert_eq!(&buf[38..42], b"MCZd");

        // MCZ magic inside MCZd at offset 46
        assert_eq!(&buf[COVER_PREFIX..COVER_PREFIX + 4], b"MCZ\x01");

        // read_index auto-detects
        let index = read_index(&buf).unwrap();
        assert_eq!(index.pages.len(), 1);
        assert_eq!(index.pages[0].offset as usize, COVER_PREFIX + 24); // 46 + 8 + 1*16
        assert_eq!(extract_page(&buf, &index, 0).unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn bad_magic() {
        let data = b"NOTMCZ\x00\x00";
        assert!(read_index(data).is_err());
    }
}
