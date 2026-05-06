use std::io;
use std::path::Path;

use crate::format::MCZIndex;

/// Unpack BNL data into a directory as `000.<ext>`, `001.<ext>`, ...
/// Filename width is derived from page count to keep alphabetical sort.
/// Returns the index that was unpacked.
pub fn unpack(data: &[u8], out_dir: &Path) -> Result<MCZIndex, UnpackError> {
    let index = crate::read_index(data).map_err(UnpackError::Parse)?;
    std::fs::create_dir_all(out_dir).map_err(UnpackError::Io)?;

    let width = digit_width(index.pages.len());
    for page in &index.pages {
        let bytes = crate::extract_page(data, &index, page.index as usize)
            .map_err(UnpackError::Extract)?;
        let path = out_dir.join(format!(
            "{:0width$}.{}",
            page.index,
            page.format.extension(),
            width = width
        ));
        std::fs::write(&path, bytes).map_err(UnpackError::Io)?;
    }

    Ok(index)
}

fn digit_width(n: usize) -> usize {
    let last = n.saturating_sub(1).max(1);
    last.to_string().len().max(3)
}

#[derive(Debug)]
pub enum UnpackError {
    Io(io::Error),
    Parse(crate::ParseError),
    Extract(crate::ExtractError),
}

impl std::fmt::Display for UnpackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Parse(e) => write!(f, "parse error: {e}"),
            Self::Extract(e) => write!(f, "extract error: {e}"),
        }
    }
}

impl std::error::Error for UnpackError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack, EncodedPage, ImageFormat};

    #[test]
    fn unpack_roundtrip() {
        let pages = vec![
            EncodedPage {
                data: vec![0xFF, 0xD8, 0xFF, 0xE0],
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
        pack(&pages, &mut buf, false).unwrap();

        let tmp = std::env::temp_dir().join("bunle_unpack_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let index = unpack(&buf, &tmp).unwrap();
        assert_eq!(index.pages.len(), 2);

        let p0 = std::fs::read(tmp.join("000.jpg")).unwrap();
        assert_eq!(p0, vec![0xFF, 0xD8, 0xFF, 0xE0]);
        let p1 = std::fs::read(tmp.join("001.webp")).unwrap();
        assert_eq!(p1, vec![0x52, 0x49, 0x46, 0x46]);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn digit_width_minimum_three() {
        assert_eq!(digit_width(1), 3);
        assert_eq!(digit_width(10), 3);
        assert_eq!(digit_width(1000), 3);
        assert_eq!(digit_width(1001), 4);
        assert_eq!(digit_width(65536), 5);
    }
}
