use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use memmap2::Mmap;
use pyo3::exceptions::{PyIOError, PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

fn fmt_str(f: bunle::ImageFormat) -> &'static str {
    match f {
        bunle::ImageFormat::WebP => "webp",
        bunle::ImageFormat::Jpeg => "jpeg",
        bunle::ImageFormat::Jxl => "jxl",
    }
}

fn page_info_to_dict<'py>(py: Python<'py>, p: &bunle::PageInfo) -> Bound<'py, PyDict> {
    let d = PyDict::new_bound(py);
    d.set_item("index", p.index).unwrap();
    d.set_item("width", p.width).unwrap();
    d.set_item("height", p.height).unwrap();
    d.set_item("format", fmt_str(p.format)).unwrap();
    d.set_item("offset", p.offset).unwrap();
    d.set_item("size", p.size).unwrap();
    d
}

/// Pack all images in `in_dir` into a Bunle archive at `out_file`.
///
/// WebP/JPEG/JXL inputs are stored byte-identical (passthrough). PNG/BMP/TIFF
/// are encoded to WebP at `quality` (1-100, default 80). Set `cover=False` to
/// skip the polyglot RIFF/WebP wrapper.
#[pyfunction]
#[pyo3(signature = (in_dir, out_file, quality=80, cover=true))]
fn pack_dir(in_dir: PathBuf, out_file: PathBuf, quality: u8, cover: bool) -> PyResult<()> {
    bunle::pack_dir(&in_dir, &out_file, quality, cover)
        .map(|_| ())
        .map_err(|e| PyValueError::new_err(format!("{e}")))
}

/// Unpack `archive` into `out_dir` as `<i>.<ext>` files (zero-padded width).
#[pyfunction]
fn unpack(archive: PathBuf, out_dir: PathBuf) -> PyResult<usize> {
    let data = std::fs::read(&archive).map_err(|e| PyIOError::new_err(e.to_string()))?;
    let index = bunle::unpack(&data, &out_dir)
        .map_err(|e| PyValueError::new_err(format!("{e}")))?;
    Ok(index.pages.len())
}

/// Return archive metadata as a dict: { version, page_count, total_bytes,
/// pages: [ {index, width, height, format, offset, size}, ... ] }.
#[pyfunction]
fn info<'py>(py: Python<'py>, archive: PathBuf) -> PyResult<Bound<'py, PyDict>> {
    let data = std::fs::read(&archive).map_err(|e| PyIOError::new_err(e.to_string()))?;
    let index = bunle::read_index(&data)
        .map_err(|e| PyValueError::new_err(format!("{e}")))?;

    let d = PyDict::new_bound(py);
    d.set_item("version", index.version)?;
    d.set_item("page_count", index.pages.len())?;
    d.set_item("total_bytes", data.len())?;
    let pages: Vec<Bound<'py, PyDict>> =
        index.pages.iter().map(|p| page_info_to_dict(py, p)).collect();
    d.set_item("pages", pages)?;
    Ok(d)
}

/// Validate archive structure (header, index bounds, dimensions). Raises on
/// failure. Does not decode pixel data.
#[pyfunction]
fn validate(archive: PathBuf) -> PyResult<()> {
    let data = std::fs::read(&archive).map_err(|e| PyIOError::new_err(e.to_string()))?;
    bunle::validate(&data)
        .map(|_| ())
        .map_err(|e| PyValueError::new_err(format!("{e}")))
}

/// Random-access reader for a Bunle archive, backed by mmap.
///
/// Use as a context manager or call `close()` when done.
#[pyclass]
struct Reader {
    /// Holds the mmap alive while pages are read.
    mmap: Option<Arc<Mmap>>,
    index: bunle::MCZIndex,
}

#[pymethods]
impl Reader {
    #[new]
    fn new(archive: PathBuf) -> PyResult<Self> {
        let file = File::open(&archive).map_err(|e| PyIOError::new_err(e.to_string()))?;
        // SAFETY: mmap of a regular file we own a handle to. The file is not
        // mutated while the mmap is alive (Bunle archives are read-only here).
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| PyIOError::new_err(e.to_string()))?;
        let index = bunle::read_index(&mmap)
            .map_err(|e| PyValueError::new_err(format!("{e}")))?;
        Ok(Self { mmap: Some(Arc::new(mmap)), index })
    }

    #[getter]
    fn page_count(&self) -> usize {
        self.index.pages.len()
    }

    #[getter]
    fn version(&self) -> u8 {
        self.index.version
    }

    /// Return raw encoded bytes for page `i` (copy from the mmapped region).
    fn page<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyBytes>> {
        let mmap = self.mmap.as_ref().ok_or_else(|| PyValueError::new_err("reader is closed"))?;
        let bytes = bunle::extract_page(mmap, &self.index, i)
            .map_err(|e| match e {
                bunle::ExtractError::PageOutOfRange => PyIndexError::new_err(format!("page {i} out of range")),
                bunle::ExtractError::DataTruncated => PyValueError::new_err("archive data truncated"),
            })?;
        Ok(PyBytes::new_bound(py, bytes))
    }

    /// Return per-page metadata dict for page `i`.
    fn info<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        let p = self.index.pages.get(i)
            .ok_or_else(|| PyIndexError::new_err(format!("page {i} out of range")))?;
        Ok(page_info_to_dict(py, p))
    }

    /// Iterate (index, bytes) over all pages.
    fn pages<'py>(&self, py: Python<'py>) -> PyResult<Vec<(usize, Bound<'py, PyBytes>)>> {
        (0..self.index.pages.len())
            .map(|i| Ok((i, self.page(py, i)?)))
            .collect()
    }

    fn close(&mut self) {
        self.mmap = None;
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: PyObject,
        _exc_val: PyObject,
        _exc_tb: PyObject,
    ) -> bool {
        self.close();
        false
    }

    fn __len__(&self) -> usize {
        self.index.pages.len()
    }
}

#[pymodule]
#[pyo3(name = "bunle")]
fn bunle_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pack_dir, m)?)?;
    m.add_function(wrap_pyfunction!(unpack, m)?)?;
    m.add_function(wrap_pyfunction!(info, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add_class::<Reader>()?;
    Ok(())
}
