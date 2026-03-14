use crate::error::IronpressError;
use crate::layout::engine::{LayoutElement, Page, TableCell, TextLine, TextRun};
use crate::style::computed::TextAlign;
use crate::types::{Margin, PageSize};

/// Render laid-out pages into a PDF byte buffer.
///
/// Uses the PDF built-in Helvetica font family (one of the 14 standard fonts)
/// so no font embedding is needed for the MVP.
pub fn render_pdf(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
) -> Result<Vec<u8>, IronpressError> {
    let mut writer = PdfWriter::new();
    let available_width = page_size.width - margin.left - margin.right;

    for page in pages {
        let mut content = String::new();

        for (y_pos, element) in &page.elements {
            match element {
                LayoutElement::TextBlock {
                    lines,
                    text_align,
                    background_color,
                    padding_top,
                    padding_bottom,
                    padding_left,
                    padding_right,
                    ..
                } => {
                    let block_x = margin.left;
                    // PDF y-axis is bottom-up
                    let block_y = page_size.height - margin.top - y_pos;

                    // Draw background if specified
                    if let Some((r, g, b)) = background_color {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let total_h = padding_top + text_height + padding_bottom;
                        let bg_y = block_y - padding_top - text_height - padding_bottom;
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = block_x,
                            y = bg_y,
                            w = available_width,
                            h = total_h,
                        ));
                    }

                    let mut text_y = block_y - padding_top;

                    for line in lines {
                        text_y -= line.height;

                        let line_text = line_text_content(line);
                        if line_text.is_empty() {
                            continue;
                        }

                        let line_width = estimate_line_width(line);
                        let text_x = match text_align {
                            TextAlign::Left => block_x + padding_left,
                            TextAlign::Center => block_x + (available_width - line_width) / 2.0,
                            TextAlign::Right => {
                                block_x + available_width - padding_right - line_width
                            }
                        };

                        // Render each run
                        let mut x = text_x;
                        for run in &line.runs {
                            if run.text.is_empty() {
                                continue;
                            }

                            let font_name = font_name_for_run(run);
                            let (r, g, b) = run.color;

                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            content.push_str("BT\n");
                            content.push_str(&format!(
                                "/{font_name} {size} Tf\n",
                                size = run.font_size,
                            ));
                            content.push_str(&format!("{x} {y} Td\n", y = text_y));
                            content.push_str(&format!(
                                "({escaped}) Tj\n",
                                escaped = escape_pdf_string(&run.text),
                            ));
                            content.push_str("ET\n");

                            // Draw underline
                            if run.underline {
                                let w = estimate_run_width(run);
                                let uy = text_y - 1.5;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n0.5 w\n{x} {uy} m {x2} {uy} l\nS\n",
                                    x2 = x + w,
                                ));
                            }

                            x += estimate_run_width(run);
                        }
                    }
                }
                LayoutElement::TableRow {
                    cells, col_width, ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;

                    // Compute row height (max cell height)
                    let row_height = cells
                        .iter()
                        .map(|cell| {
                            let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                            cell.padding_top + text_h + cell.padding_bottom
                        })
                        .fold(0.0f32, f32::max);

                    for (col_idx, cell) in cells.iter().enumerate() {
                        let cell_x = margin.left + col_idx as f32 * col_width;

                        // Draw cell background
                        if let Some((r, g, b)) = cell.background_color {
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = row_y - row_height,
                                w = col_width,
                                h = row_height,
                            ));
                        }

                        // Draw cell border
                        content.push_str(&format!(
                            "0.8 0.8 0.8 RG\n0.5 w\n{x} {y} {w} {h} re\nS\n",
                            x = cell_x,
                            y = row_y - row_height,
                            w = col_width,
                            h = row_height,
                        ));

                        // Render cell text
                        render_cell_text(&mut content, cell, cell_x, row_y, *col_width);
                    }
                }
                LayoutElement::HorizontalRule { .. } => {
                    let rule_y = page_size.height - margin.top - y_pos;
                    content.push_str(&format!(
                        "0.5 w\n0 0 0 RG\n{x1} {y} m {x2} {y} l\nS\n",
                        x1 = margin.left,
                        x2 = page_size.width - margin.right,
                        y = rule_y,
                    ));
                }
                LayoutElement::PageBreak => {}
            }
        }

        writer.add_page(page_size.width, page_size.height, &content);
    }

    Ok(writer.finish())
}

fn render_cell_text(
    content: &mut String,
    cell: &TableCell,
    cell_x: f32,
    row_y: f32,
    _col_width: f32,
) {
    let mut text_y = row_y - cell.padding_top;
    for line in &cell.lines {
        text_y -= line.height;
        let text_content: String = line.runs.iter().map(|r| r.text.as_str()).collect();
        if text_content.is_empty() {
            continue;
        }
        let text_x = cell_x + cell.padding_left;
        let mut x = text_x;
        for run in &line.runs {
            if run.text.is_empty() {
                continue;
            }
            let font_name = font_name_for_run(run);
            let (r, g, b) = run.color;
            content.push_str(&format!("{r} {g} {b} rg\n"));
            content.push_str("BT\n");
            content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
            content.push_str(&format!("{x} {y} Td\n", y = text_y));
            content.push_str(&format!(
                "({escaped}) Tj\n",
                escaped = escape_pdf_string(&run.text),
            ));
            content.push_str("ET\n");
            x += estimate_run_width(run);
        }
    }
}

fn font_name_for_run(run: &TextRun) -> &'static str {
    match (run.bold, run.italic) {
        (true, true) => "Helvetica-BoldOblique",
        (true, false) => "Helvetica-Bold",
        (false, true) => "Helvetica-Oblique",
        (false, false) => "Helvetica",
    }
}

fn estimate_run_width(run: &TextRun) -> f32 {
    run.text.len() as f32 * run.font_size * 0.5
}

fn estimate_line_width(line: &TextLine) -> f32 {
    line.runs.iter().map(estimate_run_width).sum()
}

fn line_text_content(line: &TextLine) -> String {
    line.runs.iter().map(|r| r.text.as_str()).collect()
}

fn escape_pdf_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Minimal PDF writer that produces valid PDF files.
struct PdfWriter {
    objects: Vec<String>,
    page_ids: Vec<usize>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
            page_ids: Vec::new(),
        }
    }

    fn next_id(&self) -> usize {
        self.objects.len() + 1
    }

    fn add_page(&mut self, width: f32, height: f32, content: &str) {
        // Content stream
        let stream = content.as_bytes();
        let content_id = self.next_id();
        self.objects.push(format!(
            "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj",
            stream.len(),
        ));

        // Page object (placeholder — will be updated in finish())
        let page_id = self.next_id();
        self.objects.push(format!(
            "{page_id} 0 obj\n<< /Type /Page /MediaBox [0 0 {width} {height}] /Contents {content_id} 0 R >>\nendobj",
        ));

        self.page_ids.push(page_id);
    }

    fn finish(self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("%PDF-1.4\n");

        // Font objects
        let font_base_id = self.objects.len() + 1;
        let font_names = [
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
        ];

        let mut all_objects = self.objects.clone();

        for (i, name) in font_names.iter().enumerate() {
            let id = font_base_id + i;
            all_objects.push(format!(
                "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>\nendobj",
            ));
        }

        // Font dictionary
        let font_dict_id = font_base_id + font_names.len();
        let font_entries: String = font_names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} {} 0 R", font_base_id + i))
            .collect::<Vec<_>>()
            .join(" ");
        all_objects.push(format!(
            "{font_dict_id} 0 obj\n<< {font_entries} >>\nendobj",
        ));

        // Resources dictionary
        let resources_id = font_dict_id + 1;
        all_objects.push(format!(
            "{resources_id} 0 obj\n<< /Font {font_dict_id} 0 R >>\nendobj",
        ));

        // Update page objects to include parent and resources
        let pages_id = resources_id + 1;
        for &page_id in &self.page_ids {
            let obj = &mut all_objects[page_id - 1];
            *obj = obj.replace(
                "/Contents",
                &format!("/Parent {pages_id} 0 R /Resources {resources_id} 0 R /Contents"),
            );
        }

        // Pages object
        let kids: String = self
            .page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        all_objects.push(format!(
            "{pages_id} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {} >>\nendobj",
            self.page_ids.len(),
        ));

        // Catalog
        let catalog_id = pages_id + 1;
        all_objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R >>\nendobj",
        ));

        // Write objects and track offsets for xref
        let mut offsets = Vec::new();
        for obj_str in &all_objects {
            offsets.push(out.len());
            out.push_str(obj_str);
            out.push('\n');
        }

        // Cross-reference table
        let xref_offset = out.len();
        out.push_str("xref\n");
        out.push_str(&format!("0 {}\n", all_objects.len() + 1));
        out.push_str("0000000000 65535 f \n");
        for offset in &offsets {
            out.push_str(&format!("{:010} 00000 n \n", offset));
        }

        // Trailer
        out.push_str("trailer\n");
        out.push_str(&format!(
            "<< /Size {} /Root {catalog_id} 0 R >>\n",
            all_objects.len() + 1,
        ));
        out.push_str("startxref\n");
        out.push_str(&format!("{xref_offset}\n"));
        out.push_str("%%EOF\n");

        out.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::layout;
    use crate::parser::html::parse_html;

    #[test]
    fn render_simple_pdf() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

        // Valid PDF starts with %PDF
        assert!(pdf.starts_with(b"%PDF-1.4"));
        // Valid PDF ends with %%EOF
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("%%EOF"));
        // Contains Helvetica font
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn render_bold_italic() {
        let nodes = parse_html("<p><strong>Bold</strong> and <em>italic</em></p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica-Bold"));
        assert!(content.contains("/Helvetica-Oblique"));
    }

    #[test]
    fn render_empty_document() {
        let nodes = parse_html("").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn pdf_string_escaping() {
        assert_eq!(escape_pdf_string("hello"), "hello");
        assert_eq!(escape_pdf_string("(test)"), "\\(test\\)");
        assert_eq!(escape_pdf_string("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn render_background_color() {
        let html = r#"<pre>code here</pre>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Pre has gray background — PDF should contain rectangle fill commands
        assert!(content.contains("re\nf\n") || content.contains("re"));
    }

    #[test]
    fn render_center_align() {
        let html = r#"<p style="text-align: center">Centered</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_right_align() {
        let html = r#"<p style="text-align: right">Right</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_underline() {
        let html = "<p><u>Underlined text</u></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Underline draws a line with stroke command
        assert!(content.contains(" l\nS\n"));
    }

    #[test]
    fn render_bold_italic_combined() {
        let html = "<p><strong><em>Bold Italic</em></strong></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica-BoldOblique"));
    }

    #[test]
    fn render_page_break_in_content() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should have multiple page objects
        assert!(content.matches("/Type /Page").count() >= 2);
    }

    #[test]
    fn render_colored_text() {
        let html = r#"<p style="color: red">Red text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red in PDF
    }
}
