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
    header: Option<String>,
    footer: Option<String>,
    fonts: Vec<(String, Vec<u8>)>,
    base_path: Option<String>
}

#[pymethods]
impl HtmlConverter {
    #[new]
    fn new() -> Self {
        HtmlConverter {
            page_size: ironpress_core::types::PageSize::A4,
            margin: ironpress_core::types::Margin::default(),
            header: None,
            footer: None,
            fonts: Vec::new(),
            base_path: None
        }
    }

    /// Set uniform margin in points (72 points = 1 inch).
    fn margin(&mut self, points: f32) {
        self.margin = ironpress_core::types::Margin::uniform(points);
    }

    /// Set margins with individual values for each side (72 points = 1 inch).
    fn margin_sides(&mut self, top: f32, right: f32, bottom: f32, left: f32) {
        self.margin = ironpress_core::types::Margin::new(top, right, bottom, left);
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

    /// Set a header text rendered at the top of each page (in the top margin area).
    fn header(&mut self, header: &str) -> PyResult<()> {
        self.header = Some(String::from(header));
        Ok(())
    }

    /// Set a footer text rendered at the bottom of each page (in the bottom margin area).
    ///
    /// Use `{page}` for the current page number and `{pages}` for the total page count.
    /// For example: `"Page {page} of {pages}"`.
    fn footer(&mut self, footer: &str) -> PyResult<()> {
        self.footer = Some(String::from(footer));
        Ok(())
    }

    /// Register a custom TrueType font.
    ///
    /// The `name` should match the `font-family` value used in CSS.
    /// The `ttf_data` is the raw contents of a `.ttf` file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// import ironpress
    ///
    /// ttf_data = open('MyFont.ttf', 'rb').read()
    /// converter = ironpress.HtmlConverter()
    /// converter.add_font('MyFont', ttf_data)
    /// pdf = converter.convert('<p style="font-family: MyFont">Custom text</p>')
    /// ```
    fn add_font(&mut self, name: &str, ttf_data: Vec<u8>) -> PyResult<()> {
        self.fonts.push((name.to_string(), ttf_data));
        Ok(())
    }

    /// Set the base directory for resolving relative paths in CSS `@import`
    /// and `@font-face` rules.
    ///
    /// When set, `@import "styles.css"` will resolve the path relative to
    /// this directory, and `@font-face { src: url("fonts/MyFont.ttf") }` will
    /// load the font file from this directory.
    ///
    /// Only local file paths are supported. Remote URLs (http/https) are
    /// rejected for security.
    ///
    /// # Example
    ///
    /// ```no_run
    /// import ironpress
    /// conv = ironpress.HtmlConverter()
    /// conv.base_path('/path/to/project')
    /// pdf = conv.convert('<style>@import "styles.css";</style><p>Hello</p>')
    /// ```
    fn base_path(&mut self, path: &str) {
        self.base_path = Some(path.to_string());
    }

    /// Convert HTML to PDF bytes.
    fn convert(&self, html: &str) -> PyResult<Vec<u8>> {
        self.build_core_converter()
            .convert(html)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Convert Markdown to PDF bytes.
    fn convert_markdown(&self, markdown: &str) -> PyResult<Vec<u8>> {
        self.build_core_converter()
            .convert_markdown(markdown)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
}

impl HtmlConverter {
    fn build_core_converter(&self) -> ironpress_core::HtmlConverter {
        let mut converter = ironpress_core::HtmlConverter::new()
            .page_size(self.page_size)
            .margin(self.margin);

        if let Some(h) = self.header.as_deref() { converter = converter.header(h); }
        if let Some(f) = self.footer.as_deref() { converter = converter.footer(f); }
        if let Some(bp) = self.base_path.as_deref() { converter = converter.base_path(bp.as_ref()); }

        for (name, data) in &self.fonts {
            converter = converter.add_font(name, data.clone());
        }
        converter
    }
}

#[pymodule]
fn ironpress(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(html_to_pdf, m)?)?;
    m.add_function(wrap_pyfunction!(markdown_to_pdf, m)?)?;
    m.add_class::<HtmlConverter>()?;
    Ok(())
}
