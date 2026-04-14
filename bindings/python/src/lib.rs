use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Convert an HTML string to a PDF document.
///
/// Args:
///     html: HTML string to convert.
///
/// Returns:
///     PDF document as bytes.
///
/// Example:
///     >>> import ironpress
///     >>> pdf = ironpress.html_to_pdf("<h1>Hello</h1>")
///     >>> with open("output.pdf", "wb") as f:
///     ...     f.write(pdf)
#[pyfunction]
fn html_to_pdf(html: &str) -> PyResult<Vec<u8>> {
    ironpress_core::html_to_pdf(html).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Convert a Markdown string to a PDF document.
///
/// Args:
///     markdown: Markdown string to convert.
///
/// Returns:
///     PDF document as bytes.
#[pyfunction]
fn markdown_to_pdf(markdown: &str) -> PyResult<Vec<u8>> {
    ironpress_core::markdown_to_pdf(markdown).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// A configurable HTML-to-PDF converter.
///
/// Example:
///     >>> from ironpress import HtmlConverter
///     >>> converter = HtmlConverter()
///     >>> converter.page_size("Letter")
///     >>> converter.margin(36.0)
///     >>> pdf = converter.convert("<h1>Hello</h1>")
#[pyclass]
struct HtmlConverter {
    page_size: ironpress_core::types::PageSize,
    margin: ironpress_core::types::Margin,
}

#[pymethods]
impl HtmlConverter {
    #[new]
    fn new() -> Self {
        HtmlConverter {
            page_size: ironpress_core::types::PageSize::A4,
            margin: ironpress_core::types::Margin::default(),
        }
    }

    /// Set uniform margin in points (72 points = 1 inch).
    fn margin(&mut self, points: f32) {
        self.margin = ironpress_core::types::Margin::uniform(points);
    }

    /// Set page size by name ("A4", "Letter", "Legal").
    fn page_size(&mut self, name: &str) -> PyResult<()> {
        self.page_size = match name.to_lowercase().as_str() {
            "a4" => ironpress_core::types::PageSize::A4,
            "letter" => ironpress_core::types::PageSize::LETTER,
            "legal" => ironpress_core::types::PageSize::LEGAL,
            _ => return Err(PyValueError::new_err(format!("Unknown page size: {name}"))),
        };
        Ok(())
    }

    /// Convert HTML to PDF bytes.
    fn convert(&self, html: &str) -> PyResult<Vec<u8>> {
        let converter = ironpress_core::HtmlConverter::new()
            .page_size(self.page_size)
            .margin(self.margin);
        converter
            .convert(html)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Convert Markdown to PDF bytes.
    fn convert_markdown(&self, markdown: &str) -> PyResult<Vec<u8>> {
        let converter = ironpress_core::HtmlConverter::new()
            .page_size(self.page_size)
            .margin(self.margin);
        converter
            .convert_markdown(markdown)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
}

#[pymodule]
fn ironpress(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(html_to_pdf, m)?)?;
    m.add_function(wrap_pyfunction!(markdown_to_pdf, m)?)?;
    m.add_class::<HtmlConverter>()?;
    Ok(())
}
