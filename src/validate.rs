use crate::format::ImageFormat;
use crate::{extract_page, read_index, ExtractError, MCZIndex, ParseError};

/// Validation report for a Bunle archive.
#[derive(Debug, Clone)]
pub struct ValidateReport {
    pub version: u8,
    pub page_count: u16,
    pub total_bytes: usize,
}

/// Validate a Bunle archive: header parses, every page's index entry is in
/// bounds, and (when the `cli` feature is enabled) every page decodes.
///
/// Without the `cli` feature, this performs a structural check only; pages
/// are not decoded.
pub fn validate(data: &[u8]) -> Result<ValidateReport, ValidateError> {
    let index = read_index(data).map_err(ValidateError::Parse)?;

    for page in &index.pages {
        let _ = extract_page(data, &index, page.index as usize)
            .map_err(|e| ValidateError::Extract(page.index, e))?;
        if page.width == 0 || page.height == 0 {
            return Err(ValidateError::ZeroDimensions(page.index));
        }
    }

    #[cfg(feature = "cli")]
    decode_probe(data, &index)?;

    Ok(ValidateReport {
        version: index.version,
        page_count: index.pages.len() as u16,
        total_bytes: data.len(),
    })
}

#[cfg(feature = "cli")]
fn decode_probe(data: &[u8], index: &MCZIndex) -> Result<(), ValidateError> {
    use rayon::prelude::*;

    index
        .pages
        .par_iter()
        .try_for_each(|page| {
            let bytes = extract_page(data, index, page.index as usize)
                .map_err(|e| ValidateError::Extract(page.index, e))?;
            probe_one(page.index, page.format, bytes)
        })
}

#[cfg(feature = "cli")]
fn probe_one(idx: u16, fmt: ImageFormat, bytes: &[u8]) -> Result<(), ValidateError> {
    // JXL is not in the `image` crate's default features; structural check
    // only for it.
    if matches!(fmt, ImageFormat::Jxl) {
        return Ok(());
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ValidateError::Decode(idx, e.to_string()))?;
    reader
        .into_dimensions()
        .map_err(|e| ValidateError::Decode(idx, e.to_string()))?;
    Ok(())
}

#[cfg(not(feature = "cli"))]
#[allow(dead_code)]
fn _unused(_: ImageFormat) {}

#[derive(Debug)]
pub enum ValidateError {
    Parse(ParseError),
    Extract(u16, ExtractError),
    ZeroDimensions(u16),
    Decode(u16, String),
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "parse error: {e}"),
            Self::Extract(i, e) => write!(f, "page {i}: {e}"),
            Self::ZeroDimensions(i) => write!(f, "page {i}: zero width or height"),
            Self::Decode(i, e) => write!(f, "page {i}: decode failed: {e}"),
        }
    }
}

impl std::error::Error for ValidateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack, EncodedPage};

    #[test]
    fn valid_archive_passes_structural_check() {
        // Use raw byte payloads that aren't real WebP; structural check only
        // exercises index bounds and dims.
        let pages = vec![EncodedPage {
            data: vec![0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, b'W', b'E', b'B', b'P'],
            width: 100,
            height: 200,
            format: ImageFormat::Jxl, // skip decode probe
        }];
        let mut buf = Vec::new();
        pack(&pages, &mut buf, false).unwrap();

        let report = validate(&buf).unwrap();
        assert_eq!(report.page_count, 1);
        assert_eq!(report.version, 1);
    }

    #[test]
    fn truncated_archive_fails() {
        let pages = vec![EncodedPage {
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
            width: 10,
            height: 10,
            format: ImageFormat::Jxl,
        }];
        let mut buf = Vec::new();
        pack(&pages, &mut buf, false).unwrap();

        // Drop trailing bytes so extract bounds check trips.
        buf.truncate(buf.len() - 4);
        assert!(matches!(
            validate(&buf),
            Err(ValidateError::Extract(0, ExtractError::DataTruncated))
        ));
    }
}
