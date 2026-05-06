use crate::{extract_page, read_index, ExtractError, ParseError};

/// Validation report for a Bunle archive.
#[derive(Debug, Clone)]
pub struct ValidateReport {
    pub version: u8,
    pub page_count: u16,
    pub total_bytes: usize,
}

/// Structural validation: header parses, every page index entry is in bounds,
/// no zero dimensions. Does not decode pixel data — corrupted pixels surface
/// in the consumer that actually decodes the image.
pub fn validate(data: &[u8]) -> Result<ValidateReport, ValidateError> {
    let index = read_index(data).map_err(ValidateError::Parse)?;

    for page in &index.pages {
        let _ = extract_page(data, &index, page.index as usize)
            .map_err(|e| ValidateError::Extract(page.index, e))?;
        if page.width == 0 || page.height == 0 {
            return Err(ValidateError::ZeroDimensions(page.index));
        }
    }

    Ok(ValidateReport {
        version: index.version,
        page_count: index.pages.len() as u16,
        total_bytes: data.len(),
    })
}

#[derive(Debug)]
pub enum ValidateError {
    Parse(ParseError),
    Extract(u16, ExtractError),
    ZeroDimensions(u16),
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "parse error: {e}"),
            Self::Extract(i, e) => write!(f, "page {i}: {e}"),
            Self::ZeroDimensions(i) => write!(f, "page {i}: zero width or height"),
        }
    }
}

impl std::error::Error for ValidateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack, EncodedPage, ImageFormat};

    #[test]
    fn valid_archive_passes() {
        let pages = vec![EncodedPage {
            data: vec![0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, b'W', b'E', b'B', b'P'],
            width: 100,
            height: 200,
            format: ImageFormat::WebP,
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
            format: ImageFormat::WebP,
        }];
        let mut buf = Vec::new();
        pack(&pages, &mut buf, false).unwrap();

        buf.truncate(buf.len() - 4);
        assert!(matches!(
            validate(&buf),
            Err(ValidateError::Extract(0, ExtractError::DataTruncated))
        ));
    }
}
