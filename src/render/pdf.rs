use crate::error::IronpressError;
use crate::layout::engine::{
    ImageFormat, LayoutElement, Page, PngMetadata, TableCell, TextLine, TextRun,
};
use crate::style::computed::{FontFamily, TextAlign};
use crate::types::{Margin, PageSize};

/// A link annotation to be placed on a PDF page.
struct LinkAnnotation {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    url: String,
}

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
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();

        for (elem_idx, (y_pos, element)) in page.elements.iter().enumerate() {
            match element {
                LayoutElement::TextBlock {
                    lines,
                    text_align,
                    background_color,
                    padding_top,
                    padding_bottom,
                    padding_left,
                    padding_right,
                    border_width,
                    border_color,
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

                    // Draw border if specified
                    if *border_width > 0.0 {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let total_h = padding_top + text_height + padding_bottom;
                        let border_y = block_y - padding_top - text_height - padding_bottom;
                        let (br, bg, bb) = border_color.unwrap_or((0.0, 0.0, 0.0));
                        content.push_str(&format!(
                            "{br} {bg} {bb} RG\n{bw} w\n{x} {y} {w} {h} re\nS\n",
                            bw = border_width,
                            x = block_x,
                            y = border_y,
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

                            let run_width = estimate_run_width(run);

                            // Draw underline
                            if run.underline {
                                let uy = text_y - 1.5;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n0.5 w\n{x} {uy} m {x2} {uy} l\nS\n",
                                    x2 = x + run_width,
                                ));
                            }

                            // Draw strikethrough (line-through)
                            if run.line_through {
                                let sy = text_y + run.font_size * 0.3;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n0.5 w\n{x} {sy} m {x2} {sy} l\nS\n",
                                    x2 = x + run_width,
                                ));
                            }

                            // Track link annotation
                            if let Some(url) = &run.link_url {
                                annotations.push(LinkAnnotation {
                                    x1: x,
                                    y1: text_y - 2.0,
                                    x2: x + run_width,
                                    y2: text_y + run.font_size,
                                    url: url.clone(),
                                });
                            }

                            x += run_width;
                        }
                    }
                }
                LayoutElement::TableRow {
                    cells, col_width, ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;

                    // Compute row height (max cell height, excluding rowspan > 1 cells)
                    let row_height = compute_row_height(cells);

                    // Track column position accounting for colspan
                    let mut col_pos: usize = 0;
                    for cell in cells.iter() {
                        // Skip phantom cells (rowspan = 0); they are placeholders
                        // for cells spanning from previous rows.
                        if cell.rowspan == 0 {
                            col_pos += cell.colspan;
                            continue;
                        }

                        let cell_x = margin.left + col_pos as f32 * col_width;
                        let cell_w = col_width * cell.colspan as f32;

                        // For cells with rowspan > 1, compute the total height
                        // spanning multiple rows.
                        let cell_height = if cell.rowspan > 1 {
                            let mut total_h = row_height;
                            for offset in 1..cell.rowspan {
                                let future_idx = elem_idx + offset;
                                if future_idx < page.elements.len() {
                                    if let LayoutElement::TableRow {
                                        cells: future_cells,
                                        ..
                                    } = &page.elements[future_idx].1
                                    {
                                        total_h += compute_row_height(future_cells);
                                    }
                                }
                            }
                            total_h
                        } else {
                            row_height
                        };

                        // Draw cell background
                        if let Some((r, g, b)) = cell.background_color {
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = row_y - cell_height,
                                w = cell_w,
                                h = cell_height,
                            ));
                        }

                        // Draw cell border
                        content.push_str(&format!(
                            "0.8 0.8 0.8 RG\n0.5 w\n{x} {y} {w} {h} re\nS\n",
                            x = cell_x,
                            y = row_y - cell_height,
                            w = cell_w,
                            h = cell_height,
                        ));

                        // Render cell text at the first row's y position
                        render_cell_text(&mut content, cell, cell_x, row_y, cell_w);

                        col_pos += cell.colspan;
                    }
                }
                LayoutElement::Image {
                    data,
                    width,
                    height,
                    format,
                    png_metadata,
                    ..
                } => {
                    let img_x = margin.left;
                    // PDF y-axis is bottom-up; y_pos is top of margin, image draws from bottom-left
                    let img_y = page_size.height - margin.top - y_pos - height;
                    let img_obj_id = writer.add_image_object(
                        data,
                        *width as u32,
                        *height as u32,
                        *format,
                        png_metadata.as_ref(),
                    );
                    let img_name = format!("Im{img_obj_id}");
                    content.push_str(&format!(
                        "q\n{w} 0 0 {h} {x} {y} cm\n/{name} Do\nQ\n",
                        w = width,
                        h = height,
                        x = img_x,
                        y = img_y,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
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

        writer.add_page(
            page_size.width,
            page_size.height,
            &content,
            annotations,
            page_images,
        );
    }

    Ok(writer.finish())
}

/// Compute the height of a table row from its cells.
fn compute_row_height(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(|cell| {
            let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
            cell.padding_top + text_h + cell.padding_bottom
        })
        .fold(0.0f32, f32::max)
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
    match (run.font_family, run.bold, run.italic) {
        // Helvetica (sans-serif)
        (FontFamily::Helvetica, true, true) => "Helvetica-BoldOblique",
        (FontFamily::Helvetica, true, false) => "Helvetica-Bold",
        (FontFamily::Helvetica, false, true) => "Helvetica-Oblique",
        (FontFamily::Helvetica, false, false) => "Helvetica",
        // Times Roman (serif)
        (FontFamily::TimesRoman, true, true) => "Times-BoldItalic",
        (FontFamily::TimesRoman, true, false) => "Times-Bold",
        (FontFamily::TimesRoman, false, true) => "Times-Italic",
        (FontFamily::TimesRoman, false, false) => "Times-Roman",
        // Courier (monospace)
        (FontFamily::Courier, true, true) => "Courier-BoldOblique",
        (FontFamily::Courier, true, false) => "Courier-Bold",
        (FontFamily::Courier, false, true) => "Courier-Oblique",
        (FontFamily::Courier, false, false) => "Courier",
    }
}

fn estimate_run_width(run: &TextRun) -> f32 {
    let char_width_factor = match run.font_family {
        // Courier is monospace, each character is ~0.6 em
        FontFamily::Courier => 0.6,
        // Times is slightly narrower than Helvetica on average
        FontFamily::TimesRoman => 0.48,
        // Helvetica average character width
        FontFamily::Helvetica => 0.5,
    };
    run.text.len() as f32 * run.font_size * char_width_factor
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

/// A reference to an image XObject used on a page.
struct ImageRef {
    name: String,
    obj_id: usize,
}

/// Minimal PDF writer that produces valid PDF files.
struct PdfWriter {
    objects: Vec<String>,
    /// Raw binary objects stored separately (index corresponds to objects slot).
    binary_objects: std::collections::HashMap<usize, Vec<u8>>,
    page_ids: Vec<usize>,
    /// Annotation object IDs grouped by page index.
    page_annotations: Vec<Vec<usize>>,
    /// Image references grouped by page index.
    page_images: Vec<Vec<ImageRef>>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
            binary_objects: std::collections::HashMap::new(),
            page_ids: Vec::new(),
            page_annotations: Vec::new(),
            page_images: Vec::new(),
        }
    }

    fn next_id(&self) -> usize {
        self.objects.len() + 1
    }

    /// Add an image as a PDF XObject and return its object ID.
    fn add_image_object(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        format: ImageFormat,
        png_metadata: Option<&PngMetadata>,
    ) -> usize {
        let id = self.next_id();
        let header = match format {
            ImageFormat::Jpeg => {
                format!(
                    "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {len} >>\nstream\n",
                    len = data.len(),
                )
            }
            ImageFormat::Png => {
                let meta = png_metadata.expect("PNG metadata required for PNG images");
                let color_space = match meta.channels {
                    1 | 2 => "/DeviceGray",
                    _ => "/DeviceRGB",
                };
                format!(
                    "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent {bpc} /Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns {width} /Colors {channels} /BitsPerComponent {bpc} >> /Length {len} >>\nstream\n",
                    bpc = meta.bit_depth,
                    channels = meta.channels,
                    len = data.len(),
                )
            }
        };
        self.objects.push(header);
        self.binary_objects.insert(id, data.to_vec());
        id
    }

    fn add_page(
        &mut self,
        width: f32,
        height: f32,
        content: &str,
        annotations: Vec<LinkAnnotation>,
        images: Vec<ImageRef>,
    ) {
        // Content stream
        let stream = content.as_bytes();
        let content_id = self.next_id();
        self.objects.push(format!(
            "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj",
            stream.len(),
        ));

        // Annotation objects
        let mut annot_ids = Vec::new();
        for annot in &annotations {
            let annot_id = self.next_id();
            self.objects.push(format!(
                "{annot_id} 0 obj\n<< /Type /Annot /Subtype /Link /Rect [{x1} {y1} {x2} {y2}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({uri}) >> >>\nendobj",
                x1 = annot.x1,
                y1 = annot.y1,
                x2 = annot.x2,
                y2 = annot.y2,
                uri = escape_pdf_string(&annot.url),
            ));
            annot_ids.push(annot_id);
        }

        // Page object (placeholder — will be updated in finish())
        let page_id = self.next_id();
        self.objects.push(format!(
            "{page_id} 0 obj\n<< /Type /Page /MediaBox [0 0 {width} {height}] /Contents {content_id} 0 R >>\nendobj",
        ));

        self.page_ids.push(page_id);
        self.page_annotations.push(annot_ids);
        self.page_images.push(images);
    }

    fn finish(self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");

        // Font objects
        let font_base_id = self.objects.len() + 1;
        let font_names = [
            // Helvetica (sans-serif)
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            // Times Roman (serif)
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            // Courier (monospace)
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
        ];

        let mut all_objects: Vec<String> = self.objects.clone();

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

        // Collect all image object IDs used across all pages
        let mut all_image_refs: Vec<(&str, usize)> = Vec::new();
        for page_imgs in &self.page_images {
            for img in page_imgs {
                if !all_image_refs.iter().any(|(_, id)| *id == img.obj_id) {
                    all_image_refs.push((&img.name, img.obj_id));
                }
            }
        }

        // Resources dictionary
        let resources_id = font_dict_id + 1;
        if all_image_refs.is_empty() {
            all_objects.push(format!(
                "{resources_id} 0 obj\n<< /Font {font_dict_id} 0 R >>\nendobj",
            ));
        } else {
            let xobj_entries: String = all_image_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            all_objects.push(format!(
                "{resources_id} 0 obj\n<< /Font {font_dict_id} 0 R /XObject << {xobj_entries} >> >>\nendobj",
            ));
        }

        // Update page objects to include parent, resources, and annotations
        let pages_id = resources_id + 1;
        for (idx, &page_id) in self.page_ids.iter().enumerate() {
            let obj = &mut all_objects[page_id - 1];
            let annot_ids = &self.page_annotations[idx];
            let mut extra = format!("/Parent {pages_id} 0 R /Resources {resources_id} 0 R");
            if !annot_ids.is_empty() {
                let annots_str: String = annot_ids
                    .iter()
                    .map(|id| format!("{id} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ");
                extra.push_str(&format!(" /Annots [{annots_str}]"));
            }
            *obj = obj.replace("/Contents", &format!("{extra} /Contents"));
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
        // Binary objects (images) need special handling
        let mut offsets = Vec::new();
        for (idx, obj_str) in all_objects.iter().enumerate() {
            offsets.push(out.len());
            let obj_id = idx + 1;
            if let Some(bin_data) = self.binary_objects.get(&obj_id) {
                // Write the header (stored in obj_str), then binary data, then endstream/endobj
                out.extend_from_slice(obj_str.as_bytes());
                out.extend_from_slice(bin_data);
                out.extend_from_slice(b"\nendstream\nendobj\n");
            } else {
                out.extend_from_slice(obj_str.as_bytes());
                out.push(b'\n');
            }
        }

        // Cross-reference table
        let xref_offset = out.len();
        let xref_header = format!("xref\n0 {}\n", all_objects.len() + 1);
        out.extend_from_slice(xref_header.as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            let entry = format!("{:010} 00000 n \n", offset);
            out.extend_from_slice(entry.as_bytes());
        }

        // Trailer
        let trailer = format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            all_objects.len() + 1,
        );
        out.extend_from_slice(trailer.as_bytes());

        out
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

    #[test]
    fn render_table_basic() {
        let html = r#"
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Cell borders are drawn with rectangle stroke
        assert!(content.contains("re\nS\n"));
        assert!(content.contains("Name"));
        assert!(content.contains("Alice"));
    }

    #[test]
    fn render_table_with_background() {
        let html = r#"
            <table>
                <tr><td style="background-color: yellow">Highlighted</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Background fill command
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn render_empty_line_skipped() {
        let html = "<p>Above</p><br><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Above"));
        assert!(content.contains("Below"));
    }

    #[test]
    fn render_empty_run_skipped() {
        let html = "<p>Text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_page_break_element() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Multiple pages rendered
        assert!(content.matches("/Type /Page ").count() >= 2);
    }

    #[test]
    fn render_cell_text_empty_line_skipped() {
        let html = r#"<table><tr><td></td><td>Content</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Content"));
    }

    #[test]
    fn render_horizontal_rule() {
        let html = "<p>Above</p><hr><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // HR draws a line with stroke
        assert!(content.contains(" l\nS\n"));
    }

    #[test]
    fn render_link_annotation() {
        let html = r#"<p><a href="https://example.com">Click here</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should contain a Link annotation with the URI
        assert!(
            content.contains("/Subtype /Link"),
            "PDF should contain a Link annotation"
        );
        assert!(
            content.contains("/S /URI"),
            "PDF should contain a URI action"
        );
        assert!(
            content.contains("https://example.com"),
            "PDF should contain the link URL"
        );
        // The page object should reference annotations
        assert!(
            content.contains("/Annots ["),
            "Page should have an /Annots array"
        );
    }

    #[test]
    fn render_link_no_annotation_without_href() {
        // An <a> tag without href should not produce an annotation
        let html = "<p><a>No link</a></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Subtype /Link"),
            "PDF should not contain a Link annotation without href"
        );
    }

    #[test]
    fn render_link_url_escaped() {
        // URL with parentheses should be properly escaped
        let html = r#"<p><a href="https://example.com/page(1)">Link</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Subtype /Link"));
        assert!(content.contains(r"https://example.com/page\(1\)"));
    }

    #[test]
    fn render_multiple_links() {
        let html =
            r#"<p><a href="https://one.com">One</a> and <a href="https://two.com">Two</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("https://one.com"));
        assert!(content.contains("https://two.com"));
        // Should have two Link annotations
        assert_eq!(
            content.matches("/Subtype /Link").count(),
            2,
            "Should have exactly 2 link annotations"
        );
    }

    #[test]
    fn render_page_without_links_has_no_annots() {
        let html = "<p>No links here</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Annots"),
            "Page without links should not have /Annots"
        );
    }

    #[test]
    fn render_image_contains_xobject() {
        // Use a data URI with a tiny JPEG-like payload
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAC/9k=" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_no_image_no_xobject() {
        let html = "<p>No images here</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/XObject"),
            "PDF without images should not contain /XObject"
        );
    }

    #[test]
    fn render_border_draws_rectangle_stroke() {
        let html = r#"<div style="border: 1px solid black">Bordered text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Border draws a rectangle with stroke (re + S)
        assert!(
            content.contains("re\nS\n"),
            "PDF should contain rectangle stroke for border"
        );
        // The stroke color should be black (0 0 0 RG)
        assert!(
            content.contains("0 0 0 RG"),
            "Border stroke color should be black"
        );
    }

    #[test]
    fn render_border_with_custom_color() {
        let html = r#"<div style="border: 2px solid red">Red border</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Red border: 1 0 0 RG
        assert!(
            content.contains("1 0 0 RG"),
            "Border stroke color should be red"
        );
        assert!(
            content.contains("re\nS\n"),
            "PDF should contain rectangle stroke for border"
        );
    }

    #[test]
    fn render_times_roman_font_family() {
        let html = r#"<p style="font-family: serif">Serif text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "PDF should use Times-Roman for serif font-family"
        );
    }

    #[test]
    fn render_times_bold_italic() {
        let html =
            r#"<p style="font-family: serif"><strong><em>Bold Italic Serif</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-BoldItalic"),
            "PDF should use Times-BoldItalic for bold italic serif"
        );
    }

    #[test]
    fn render_times_bold() {
        let html = r#"<p style="font-family: times"><strong>Bold Serif</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Bold"),
            "PDF should use Times-Bold for bold serif"
        );
    }

    #[test]
    fn render_times_italic() {
        let html = r#"<p style="font-family: serif"><em>Italic Serif</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Italic"),
            "PDF should use Times-Italic for italic serif"
        );
    }

    #[test]
    fn render_courier_font_family() {
        let html = r#"<p style="font-family: monospace">Monospace text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier ") || content.contains("/Courier\n"),
            "PDF should use Courier for monospace font-family"
        );
    }

    #[test]
    fn render_courier_bold_italic() {
        let html =
            r#"<p style="font-family: courier"><strong><em>Bold Italic Mono</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-BoldOblique"),
            "PDF should use Courier-BoldOblique for bold italic monospace"
        );
    }

    #[test]
    fn render_courier_bold() {
        let html = r#"<p style="font-family: monospace"><strong>Bold Mono</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Bold"),
            "PDF should use Courier-Bold for bold monospace"
        );
    }

    #[test]
    fn render_courier_oblique() {
        let html = r#"<p style="font-family: courier"><em>Italic Mono</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Oblique"),
            "PDF should use Courier-Oblique for italic monospace"
        );
    }

    #[test]
    fn render_font_family_via_stylesheet() {
        let html = r#"
            <html>
            <head><style>p { font-family: serif }</style></head>
            <body><p>Styled serif</p></body>
            </html>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "Stylesheet font-family should produce Times-Roman"
        );
    }

    #[test]
    fn render_jpeg_image_contains_xobject() {
        // Use a data URI with a tiny JPEG-like payload
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAC/9k=" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_png_image_contains_flatedecode() {
        // Build a minimal valid PNG as base64 data URI
        let png_bytes = build_minimal_test_png();
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="100" height="100">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with PNG image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /FlateDecode"),
            "PNG image should use FlateDecode filter"
        );
        assert!(
            content.contains("/Predictor 15"),
            "PNG image should have Predictor 15 in DecodeParms"
        );
        assert!(
            content.contains("/Colors 3"),
            "RGB PNG should have Colors 3"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_png_grayscale_image() {
        let png_bytes = build_test_png_with_color_type(0); // Grayscale
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="50" height="50">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Filter /FlateDecode"));
        assert!(content.contains("/ColorSpace /DeviceGray"));
        assert!(content.contains("/Colors 1"));
    }

    /// Build a minimal valid PNG (1x1 RGB, 8-bit).
    fn build_minimal_test_png() -> Vec<u8> {
        build_test_png_with_color_type(2) // RGB
    }

    fn build_test_png_with_color_type(color_type: u8) -> Vec<u8> {
        let mut png = Vec::new();
        // PNG signature
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR chunk (13 bytes data)
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // width
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
        ihdr.push(8); // bit depth
        ihdr.push(color_type);
        ihdr.push(0); // compression
        ihdr.push(0); // filter
        ihdr.push(0); // interlace
        append_png_chunk(&mut png, b"IHDR", &ihdr);
        // IDAT chunk with dummy zlib-compressed data
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_png_chunk(&mut png, b"IDAT", &idat);
        // IEND
        append_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn append_png_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
    }

    fn simple_base64_encode_test(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        let mut i = 0;
        while i < data.len() {
            let b0 = data[i] as u32;
            let b1 = if i + 1 < data.len() {
                data[i + 1] as u32
            } else {
                0
            };
            let b2 = if i + 2 < data.len() {
                data[i + 2] as u32
            } else {
                0
            };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if i + 1 < data.len() {
                result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if i + 2 < data.len() {
                result.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            i += 3;
        }
        result
    }

    #[test]
    fn render_all_12_fonts_registered() {
        let html = "<p>Test</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // All 12 standard font variants should be registered as font objects
        for name in &[
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
        ] {
            assert!(
                content.contains(&format!("/BaseFont /{name}")),
                "PDF should register font {name}"
            );
        }
    }
}
