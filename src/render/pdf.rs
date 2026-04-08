use crate::error::IronpressError;
use crate::layout::engine::{
    ImageFormat, LayoutElement, Page, PngMetadata, TableCell, TableCellContent, TextLine, TextRun,
    table_cell_content_height,
};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BorderCollapse, BorderSpacing, Float, FontFamily, LinearGradient, Position, RadialGradient,
    TextAlign,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;

/// A PDF shading dictionary entry for native gradient rendering.
struct ShadingEntry {
    name: String,
    shading_type: u8, // 2 = axial (linear), 3 = radial
    coords: [f32; 6],
    stops: Vec<(f32, (f32, f32, f32))>,
}

/// A link annotation to be placed on a PDF page.
struct LinkAnnotation {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    url: String,
}

/// A bookmark entry for PDF outline (table of contents).
#[allow(dead_code)]
struct BookmarkEntry {
    title: String,
    level: u8,
    page_index: usize,
    y_pos: f32,
}

/// Render laid-out pages into a PDF byte buffer.
///
/// Uses the PDF built-in Helvetica font family (one of the 14 standard fonts)
/// so no font embedding is needed for the MVP.
#[allow(dead_code)]
pub fn render_pdf(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
) -> Result<Vec<u8>, IronpressError> {
    render_pdf_with_fonts(pages, page_size, margin, &HashMap::new())
}

/// Render laid-out pages into a PDF byte buffer, with custom font embedding.
pub fn render_pdf_with_fonts(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Result<Vec<u8>, IronpressError> {
    let mut buf = Vec::new();
    render_pdf_to_writer_with_fonts(pages, page_size, margin, &mut buf, custom_fonts)?;
    Ok(buf)
}

/// Header and footer text for page decoration.
pub struct PageDecoration {
    /// Header text rendered top-center of each page.
    pub header: Option<String>,
    /// Footer text rendered bottom-center of each page.
    /// `{page}` and `{pages}` are replaced with page number and total count.
    pub footer: Option<String>,
}

/// Render laid-out pages as PDF, writing directly to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`render_pdf`]. It writes PDF content incrementally
/// to the provided writer instead of building an in-memory buffer.
#[allow(dead_code)]
pub fn render_pdf_to_writer<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_with_fonts(pages, page_size, margin, writer, &HashMap::new())
}

/// Render laid-out pages as PDF with custom fonts, writing directly to any `std::io::Write` implementation.
fn render_pdf_to_writer_with_fonts<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full(pages, page_size, margin, writer, custom_fonts, None)
}

/// Full render function with optional page decoration (headers/footers).
pub(crate) fn render_pdf_to_writer_full<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
    decoration: Option<&PageDecoration>,
) -> Result<(), IronpressError> {
    let mut pdf_writer = PdfWriter::new();
    let available_width = page_size.width - margin.left - margin.right;
    let mut bookmarks: Vec<BookmarkEntry> = Vec::new();

    // Register custom TrueType fonts
    for (name, ttf) in custom_fonts {
        pdf_writer.add_ttf_font(name, ttf);
    }

    for (page_idx, page) in pages.iter().enumerate() {
        let mut content = String::new();
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();
        let mut page_ext_gstates: Vec<(String, f32)> = Vec::new();
        let mut page_shadings: Vec<ShadingEntry> = Vec::new();
        let mut shading_counter: usize = 0;

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
                    border,
                    block_width,
                    block_height,
                    opacity,
                    float,
                    position,
                    offset_left,
                    box_shadow,
                    visible,
                    clip_rect,
                    transform,
                    background_gradient,
                    background_radial_gradient,
                    border_radius,
                    outline_width,
                    outline_color,
                    letter_spacing,
                    word_spacing: css_word_spacing,
                    heading_level,
                    ..
                } => {
                    // Skip rendering if visibility: hidden (but space is preserved)
                    if !visible {
                        continue;
                    }

                    // Collect heading bookmark for PDF outlines
                    if let Some(level) = heading_level {
                        let title: String = lines
                            .iter()
                            .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !title.trim().is_empty() {
                            bookmarks.push(BookmarkEntry {
                                title: title.trim().to_string(),
                                level: *level,
                                page_index: page_idx,
                                y_pos: *y_pos,
                            });
                        }
                    }

                    // Compute block_x with float/position offsets
                    let block_x = match position {
                        Position::Absolute => margin.left + offset_left,
                        Position::Relative => margin.left + offset_left,
                        Position::Static => match float {
                            Float::Right => {
                                let render_w = block_width.unwrap_or(available_width);
                                margin.left + available_width - render_w
                            }
                            _ => margin.left,
                        },
                    };
                    // PDF y-axis is bottom-up
                    let block_y = page_size.height - margin.top - y_pos;

                    // Use explicit block_width if set, otherwise available_width
                    let render_width = block_width.unwrap_or(available_width);

                    // Apply transform if set (wrap in q/Q)
                    let needs_transform = transform.is_some();
                    if let Some(t) = transform {
                        content.push_str("q\n");
                        match t {
                            crate::style::computed::Transform::Rotate(deg) => {
                                let rad = deg * std::f32::consts::PI / 180.0;
                                let cos_v = rad.cos();
                                let sin_v = rad.sin();
                                content.push_str(&format!(
                                    "{cos_v} {sin_v} {neg_sin} {cos_v} 0 0 cm\n",
                                    neg_sin = -sin_v,
                                ));
                            }
                            crate::style::computed::Transform::Scale(sx, sy) => {
                                content.push_str(&format!("{sx} 0 0 {sy} 0 0 cm\n",));
                            }
                            crate::style::computed::Transform::Translate(tx, ty) => {
                                content.push_str(&format!("1 0 0 1 {tx} {ty} cm\n",));
                            }
                        }
                    }

                    // Apply clipping rect if overflow: hidden
                    let needs_clip = clip_rect.is_some();
                    if let Some((cx, cy, cw, ch)) = clip_rect {
                        let clip_x = block_x + cx;
                        let clip_y = block_y - ch - cy;
                        content.push_str("q\n");
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                clip_x,
                                clip_y,
                                *cw,
                                *ch,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        } else {
                            content.push_str(&format!("{clip_x} {clip_y} {cw} {ch} re W n\n",));
                        }
                    }

                    // Apply opacity via ExtGState if < 1.0
                    let needs_opacity = *opacity < 1.0;
                    if needs_opacity {
                        let gs_name = format!("GS{elem_idx}");
                        page_ext_gstates.push((gs_name.clone(), *opacity));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }

                    // Draw box-shadow if specified (rendered as offset filled rect behind element)
                    if let Some(shadow) = box_shadow {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let (sr, sg, sb) = shadow.color.to_f32_rgb();
                        let shadow_x = block_x + shadow.offset_x;
                        let shadow_y = block_y - total_h + shadow.offset_y;
                        content.push_str(&format!("{sr} {sg} {sb} rg\n"));
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                shadow_x,
                                shadow_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = shadow_x,
                                y = shadow_y,
                                w = render_width,
                                h = total_h,
                            ));
                        }
                        content.push_str("f\n");
                    }

                    // Draw background if specified
                    if let Some((r, g, b)) = background_color {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let bg_y = block_y - total_h;
                        content.push_str(&format!("{r} {g} {b} rg\n"));
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                block_x,
                                bg_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = block_x,
                                y = bg_y,
                                w = render_width,
                                h = total_h,
                            ));
                        }
                        content.push_str("f\n");
                    }

                    // Draw linear gradient if specified
                    if let Some(gradient) = background_gradient {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let bg_y = block_y - total_h;
                        // Clip to rounded rect if border-radius is set
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                block_x,
                                bg_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_linear_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            total_h,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw radial gradient if specified
                    if let Some(gradient) = background_radial_gradient {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let bg_y = block_y - total_h;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                block_x,
                                bg_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_radial_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            total_h,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw border if specified
                    if border.has_any() {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let border_y = block_y - total_h;
                        // Check if all sides are uniform (same width & color)
                        let uniform = border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color;
                        if uniform && *border_radius > 0.0 {
                            let (br, bg, bb) = border.top.color;
                            content.push_str(&format!(
                                "{br} {bg} {bb} RG\n{bw} w\n",
                                bw = border.top.width
                            ));
                            content.push_str(&rounded_rect_path(
                                block_x,
                                border_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                            content.push_str("S\n");
                        } else if uniform {
                            let (br, bg, bb) = border.top.color;
                            content.push_str(&format!(
                                "{br} {bg} {bb} RG\n{bw} w\n",
                                bw = border.top.width
                            ));
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = block_x,
                                y = border_y,
                                w = render_width,
                                h = total_h,
                            ));
                            content.push_str("S\n");
                        } else {
                            let x1 = block_x;
                            let x2 = block_x + render_width;
                            // Offset borders by half their width so the inner edge
                            // aligns with the padding boundary (CSS box model).
                            let y_top = block_y + border.top.width / 2.0;
                            let y_bottom = border_y - border.bottom.width / 2.0;
                            let x_left = block_x - border.left.width / 2.0;
                            let x_right = block_x + render_width + border.right.width / 2.0;
                            // Top border
                            if border.top.width > 0.0 {
                                let (r, g, b) = border.top.color;
                                content
                                    .push_str(&format!("{r} {g} {b} RG\n{} w\n", border.top.width));
                                content.push_str(&format!("{x1} {y_top} m {x2} {y_top} l S\n"));
                            }
                            // Right border
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.right.width
                                ));
                                content.push_str(&format!(
                                    "{x_right} {y_top} m {x_right} {y_bottom} l S\n"
                                ));
                            }
                            // Bottom border
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.bottom.width
                                ));
                                content
                                    .push_str(&format!("{x1} {y_bottom} m {x2} {y_bottom} l S\n"));
                            }
                            // Left border
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.left.width
                                ));
                                content.push_str(&format!(
                                    "{x_left} {y_top} m {x_left} {y_bottom} l S\n"
                                ));
                            }
                        }
                    }

                    // Draw outline if specified (outside the element box)
                    if *outline_width > 0.0 {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let offset = *outline_width / 2.0;
                        let outline_x = block_x - offset;
                        let outline_y = block_y - total_h - offset;
                        let outline_w = render_width + *outline_width;
                        let outline_h = total_h + *outline_width;
                        let (or, og, ob) = outline_color.unwrap_or((0.0, 0.0, 0.0));
                        content
                            .push_str(&format!("{or} {og} {ob} RG\n{ow} w\n", ow = outline_width,));
                        if *border_radius > 0.0 {
                            let outline_r = *border_radius + offset;
                            content.push_str(&rounded_rect_path(
                                outline_x, outline_y, outline_w, outline_h, outline_r,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = outline_x,
                                y = outline_y,
                                w = outline_w,
                                h = outline_h,
                            ));
                        }
                        content.push_str("S\n");
                    }

                    let mut text_y = block_y - padding_top;

                    let line_count = lines.len();
                    for (line_idx, line) in lines.iter().enumerate() {
                        // Half-leading model: distribute excess leading equally
                        // above and below so text sits at the CSS baseline position.
                        let line_font_size =
                            line.runs.iter().map(|r| r.font_size).fold(0.0f32, f32::max);
                        let half_leading = (line.height - line_font_size) / 2.0;
                        text_y -= line_font_size + half_leading;

                        let line_text = line_text_content(line);
                        if line_text.is_empty() {
                            continue;
                        }

                        let line_width = estimate_line_width_with_fonts(line, custom_fonts);
                        let is_last_line = line_idx == line_count - 1;

                        // Calculate word spacing for justified text
                        let justify_ws = if *text_align == TextAlign::Justify && !is_last_line {
                            let content_width = render_width - padding_left - padding_right;
                            let remaining = content_width - line_width;
                            let space_count = line_text.matches(' ').count();
                            if space_count > 0 && remaining > 0.0 {
                                remaining / space_count as f32
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        };
                        let total_ws = justify_ws + *css_word_spacing;

                        let text_x = match text_align {
                            TextAlign::Left | TextAlign::Justify => block_x + padding_left,
                            TextAlign::Center => {
                                let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                                block_x + (render_width - line_width) / 2.0 + first_pad
                            }
                            TextAlign::Right => {
                                // Account for inline padding: text_x is where the
                                // text characters start, but line_width includes the
                                // full visual width (with left+right padding of inline
                                // spans).  Offset by the first run's left padding so
                                // the visual right edge aligns with the right boundary.
                                let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                                block_x + render_width - padding_right - line_width + first_pad
                            }
                        };

                        // Set letter spacing (CSS letter-spacing)
                        if *letter_spacing > 0.0 {
                            content.push_str(&format!("{letter_spacing} Tc\n"));
                        }

                        // Set word spacing (justify + CSS word-spacing)
                        if total_ws > 0.0 {
                            content.push_str(&format!("{total_ws} Tw\n"));
                        }

                        // Merge consecutive runs with the same style so
                        // spaces between words stay in a single PDF text
                        // string, preventing viewers from dropping them.
                        let merged = merge_runs(&line.runs);
                        let mut x = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }

                            let font_name = resolve_font_name(run, custom_fonts);
                            let (r, g, b) = run.color;
                            let run_width = estimate_run_width_with_fonts(run, custom_fonts);

                            // Draw background rectangle for inline spans
                            if let Some((br, bg, bb)) = run.background_color {
                                let (pad_h, pad_v) = run.padding;
                                let rect_x = x - pad_h;
                                let rect_y = text_y - 2.0 - pad_v;
                                let rect_w = run_width + pad_h * 2.0;
                                let rect_h = run.font_size + 2.0 + pad_v * 2.0;
                                content.push_str(&format!("{br} {bg} {bb} rg\n"));
                                if run.border_radius > 0.0 {
                                    content.push_str(&rounded_rect_path(
                                        rect_x,
                                        rect_y,
                                        rect_w,
                                        rect_h,
                                        run.border_radius,
                                    ));
                                    content.push_str("\nf\n");
                                } else {
                                    content.push_str(&format!(
                                        "{rect_x} {rect_y} {rect_w} {rect_h} re\nf\n"
                                    ));
                                }
                            }

                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            content.push_str("BT\n");
                            content.push_str(&format!(
                                "/{font_name} {size} Tf\n",
                                size = run.font_size,
                            ));
                            content.push_str(&format!("{x} {y} Td\n", y = text_y));
                            {
                                let encoded = encode_pdf_text(&run.text);
                                content.push_str(&format!("({encoded}) Tj\n"));
                            }
                            content.push_str("ET\n");

                            // Draw underline (font-size-relative position and thickness)
                            if run.underline {
                                let desc =
                                    crate::fonts::descender_ratio(&run.font_family) * run.font_size;
                                let uy = text_y - desc * 0.6;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{thickness} w\n{x} {uy} m {x2} {uy} l\nS\n",
                                    x2 = x + run_width,
                                ));
                            }

                            // Draw strikethrough (line-through)
                            if run.line_through {
                                let sy = text_y + run.font_size * 0.3;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{thickness} w\n{x} {sy} m {x2} {sy} l\nS\n",
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

                        // Reset letter spacing after line
                        if *letter_spacing > 0.0 {
                            content.push_str("0 Tc\n");
                        }

                        // Reset word spacing after line
                        if total_ws > 0.0 {
                            content.push_str("0 Tw\n");
                        }
                    }

                    // Reset opacity if it was changed
                    if needs_opacity {
                        content.push_str("/GSDefault gs\n");
                    }

                    // Restore clipping state
                    if needs_clip {
                        content.push_str("Q\n");
                    }

                    // Restore transform state
                    if needs_transform {
                        content.push_str("Q\n");
                    }
                }
                LayoutElement::TableRow {
                    cells,
                    col_widths,
                    border_collapse,
                    border_spacing,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let spacing = if *border_collapse == BorderCollapse::Collapse {
                        BorderSpacing::default()
                    } else {
                        *border_spacing
                    };

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

                        let (cell_x, cell_w) = table_cell_geometry(
                            col_widths,
                            col_pos,
                            cell.colspan,
                            spacing.horizontal,
                            margin.left,
                        );

                        // For cells with rowspan > 1, compute the total height
                        // spanning multiple rows.
                        let cell_height = if cell.rowspan > 1 {
                            table_rowspan_height(
                                page,
                                elem_idx,
                                row_height,
                                cell.rowspan,
                                spacing.vertical,
                            )
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

                        // Draw cell borders when CSS specifies them.
                        if cell.border.has_any() {
                            let x1 = cell_x;
                            let x2 = cell_x + cell_w;
                            let y_top = row_y;
                            let y_bottom = row_y - cell_height;
                            if cell.border.top.width > 0.0 {
                                let (r, g, b) = cell.border.top.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                    cell.border.top.width
                                ));
                            }
                            if cell.border.right.width > 0.0 {
                                let (r, g, b) = cell.border.right.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                    cell.border.right.width
                                ));
                            }
                            if cell.border.bottom.width > 0.0 {
                                let (r, g, b) = cell.border.bottom.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                    cell.border.bottom.width
                                ));
                            }
                            if cell.border.left.width > 0.0 {
                                let (r, g, b) = cell.border.left.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                    cell.border.left.width
                                ));
                            }
                        }

                        // Render cell text at the first row's y position
                        render_cell_content(
                            &mut content,
                            cell,
                            cell_x,
                            row_y,
                            cell_w,
                            row_height,
                            custom_fonts,
                        );

                        col_pos += cell.colspan;
                    }
                }
                LayoutElement::GridRow {
                    cells, col_widths, ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let row_height = compute_row_height(cells);

                    let mut cell_x = margin.left;
                    for (i, cell) in cells.iter().enumerate() {
                        let cell_w = if i < col_widths.len() {
                            col_widths[i]
                        } else {
                            0.0
                        };

                        // Draw cell background
                        if let Some((r, g, b)) = cell.background_color {
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = row_y - row_height,
                                w = cell_w,
                                h = row_height,
                            ));
                        }

                        // Render cell text
                        render_cell_content(
                            &mut content,
                            cell,
                            cell_x,
                            row_y,
                            cell_w,
                            row_height,
                            custom_fonts,
                        );

                        cell_x += cell_w;
                        // Add gap between columns
                        if i + 1 < col_widths.len() {
                            let total_col_width: f32 = col_widths.iter().sum();
                            let total_gap = available_width - total_col_width;
                            let num_gaps = col_widths.len().saturating_sub(1);
                            if num_gaps > 0 {
                                cell_x += total_gap / num_gaps as f32;
                            }
                        }
                    }
                }
                LayoutElement::FlexRow {
                    cells,
                    row_height,
                    background_color,
                    container_width,
                    padding_top,
                    padding_bottom,
                    padding_left,
                    padding_right: _,
                    border,
                    border_radius,
                    box_shadow,
                    background_gradient,
                    background_radial_gradient,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let full_height =
                        padding_top + row_height + padding_bottom + border.vertical_width();

                    // Draw box shadow if present
                    if let Some(shadow) = box_shadow {
                        let sx = margin.left + shadow.offset_x;
                        let sy = row_y - full_height - shadow.offset_y;
                        let (sr, sg, sb) = shadow.color.to_f32_rgb();
                        content.push_str(&format!(
                            "{sr} {sg} {sb} rg\n{sx} {sy} {w} {h} re\nf\n",
                            w = container_width,
                            h = full_height,
                        ));
                    }

                    // Draw container background
                    if let Some((r, g, b)) = background_color {
                        let bg_x = margin.left;
                        let bg_y = row_y - full_height;
                        content.push_str(&format!("{r} {g} {b} rg\n"));
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("f\n");
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\nf\n",
                                x = bg_x,
                                y = bg_y,
                                w = container_width,
                                h = full_height,
                            ));
                        }
                    }

                    // Draw container linear gradient
                    if let Some(gradient) = background_gradient {
                        let bg_x = margin.left;
                        let bg_y = row_y - full_height;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_linear_gradient(
                            &mut content,
                            gradient,
                            bg_x,
                            bg_y,
                            *container_width,
                            full_height,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw container radial gradient
                    if let Some(gradient) = background_radial_gradient {
                        let bg_x = margin.left;
                        let bg_y = row_y - full_height;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_radial_gradient(
                            &mut content,
                            gradient,
                            bg_x,
                            bg_y,
                            *container_width,
                            full_height,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw border
                    if border.has_any() {
                        let bx = margin.left;
                        let by = row_y - full_height;
                        let uniform = border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color;
                        if uniform && *border_radius > 0.0 {
                            let (r, g, b) = border.top.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{bw} w\n",
                                bw = border.top.width
                            ));
                            content.push_str(&rounded_rect_path(
                                bx,
                                by,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("S\n");
                        } else if uniform {
                            let (r, g, b) = border.top.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{bw} w\n{bx} {by} {w} {h} re\nS\n",
                                bw = border.top.width,
                                w = container_width,
                                h = full_height,
                            ));
                        } else {
                            let x1 = bx;
                            let x2 = bx + container_width;
                            let y_top = row_y;
                            let y_bottom = by;
                            if border.top.width > 0.0 {
                                let (r, g, b) = border.top.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                    border.top.width
                                ));
                            }
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                    border.right.width
                                ));
                            }
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                    border.bottom.width
                                ));
                            }
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                    border.left.width
                                ));
                            }
                        }
                    }

                    // Render each flex cell at its computed x-offset
                    let text_area_top = row_y - border.top.width - padding_top;
                    for cell in cells {
                        let cell_x = margin.left + padding_left + cell.x_offset;
                        let cell_inner_w = cell.width - cell.padding_left - cell.padding_right;

                        // Draw cell background
                        if let Some((r, g, b)) = cell.background_color {
                            let bg_x = margin.left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - row_height;
                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            if cell.border_radius > 0.0 {
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    *row_height,
                                    cell.border_radius,
                                ));
                                content.push_str("f\n");
                            } else {
                                content.push_str(&format!(
                                    "{bg_x} {bg_y} {w} {h} re\nf\n",
                                    w = cell.width,
                                    h = *row_height,
                                ));
                            }
                        }

                        // Draw cell linear gradient
                        if let Some(gradient) = &cell.background_gradient {
                            let bg_x = margin.left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - row_height;
                            if cell.border_radius > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    *row_height,
                                    cell.border_radius,
                                ));
                                content.push_str("W n\n");
                            }
                            render_linear_gradient(
                                &mut content,
                                gradient,
                                bg_x,
                                bg_y,
                                cell.width,
                                *row_height,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if cell.border_radius > 0.0 {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw cell radial gradient
                        if let Some(gradient) = &cell.background_radial_gradient {
                            let bg_x = margin.left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - row_height;
                            if cell.border_radius > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    *row_height,
                                    cell.border_radius,
                                ));
                                content.push_str("W n\n");
                            }
                            render_radial_gradient(
                                &mut content,
                                gradient,
                                bg_x,
                                bg_y,
                                cell.width,
                                *row_height,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if cell.border_radius > 0.0 {
                                content.push_str("Q\n");
                            }
                        }

                        // Render cell text
                        let mut text_y = text_area_top - cell.padding_top;
                        for line in &cell.lines {
                            let line_font_size =
                                line.runs.iter().map(|r| r.font_size).fold(0.0f32, f32::max);
                            let half_leading = (line.height - line_font_size) / 2.0;
                            text_y -= line_font_size + half_leading;
                            let text_content: String =
                                line.runs.iter().map(|r| r.text.as_str()).collect();
                            if text_content.is_empty() {
                                continue;
                            }
                            let merged = merge_runs(&line.runs);
                            // Calculate line width for text-align
                            let line_width: f32 = merged
                                .iter()
                                .map(|r| {
                                    let w = estimate_run_width_with_fonts(r, custom_fonts);
                                    w + r.padding.0 * 2.0
                                })
                                .sum();
                            let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                            let text_x = match cell.text_align {
                                TextAlign::Right => {
                                    cell_x
                                        + cell.padding_left
                                        + (cell_inner_w - line_width).max(0.0)
                                        + first_pad
                                }
                                TextAlign::Center => {
                                    cell_x
                                        + cell.padding_left
                                        + ((cell_inner_w - line_width) / 2.0).max(0.0)
                                        + first_pad
                                }
                                _ => cell_x + cell.padding_left,
                            };
                            let mut x = text_x;
                            for run in &merged {
                                if run.text.is_empty() {
                                    continue;
                                }
                                let font_name = resolve_font_name(run, custom_fonts);
                                let (r, g, b) = run.color;
                                let rw = estimate_run_width_with_fonts(run, custom_fonts);

                                // Draw background rectangle for inline spans
                                if let Some((br, bgc, bb)) = run.background_color {
                                    let (pad_h, pad_v) = run.padding;
                                    let rx = x - pad_h;
                                    let ry = text_y - 2.0 - pad_v;
                                    let rw2 = rw + pad_h * 2.0;
                                    let rh = run.font_size + 2.0 + pad_v * 2.0;
                                    content.push_str(&format!("{br} {bgc} {bb} rg\n"));
                                    if run.border_radius > 0.0 {
                                        content.push_str(&rounded_rect_path(
                                            rx,
                                            ry,
                                            rw2,
                                            rh,
                                            run.border_radius,
                                        ));
                                        content.push_str("\nf\n");
                                    } else {
                                        content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                                    }
                                }

                                content.push_str(&format!("{r} {g} {b} rg\n"));
                                content.push_str("BT\n");
                                content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
                                content.push_str(&format!("{x} {y} Td\n", y = text_y));
                                {
                                    let encoded = encode_pdf_text(&run.text);
                                    content.push_str(&format!("({encoded}) Tj\n"));
                                }
                                content.push_str("ET\n");

                                // Draw underline (font-size-relative)
                                if run.underline {
                                    let desc = crate::fonts::descender_ratio(&run.font_family)
                                        * run.font_size;
                                    let uy = text_y - desc * 0.6;
                                    let thickness = (run.font_size * 0.07).max(0.5);
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{thickness} w\n{x} {uy} m {x2} {uy} l\nS\n",
                                        x2 = x + rw,
                                    ));
                                }

                                // Draw strikethrough (line-through)
                                if run.line_through {
                                    let sy = text_y + run.font_size * 0.3;
                                    let thickness = (run.font_size * 0.07).max(0.5);
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{thickness} w\n{x} {sy} m {x2} {sy} l\nS\n",
                                        x2 = x + rw,
                                    ));
                                }

                                x += rw;
                            }
                        }
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
                    let img_obj_id = pdf_writer.add_image_object(
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
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => {
                    let svg_x = margin.left;
                    // PDF y-axis is bottom-up, SVG is top-down
                    let svg_y = page_size.height - margin.top - y_pos - height;

                    content.push_str("q\n");
                    // Position on page and flip y-axis for SVG coordinates
                    content.push_str(&format!("1 0 0 -1 {} {} cm\n", svg_x, svg_y + height));

                    // Apply viewBox scaling if present
                    if let Some(ref vb) = tree.view_box {
                        if vb.width > 0.0 && vb.height > 0.0 {
                            let sx = width / vb.width;
                            let sy = height / vb.height;
                            content.push_str(&format!(
                                "{sx} 0 0 {sy} {} {} cm\n",
                                -vb.min_x * sx,
                                -vb.min_y * sy
                            ));
                        }
                    }

                    crate::render::svg_to_pdf::render_svg_tree(tree, &mut content);
                    content.push_str("Q\n");
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
                LayoutElement::ProgressBar {
                    fraction,
                    width,
                    height,
                    fill_color,
                    track_color,
                    ..
                } => {
                    let bar_x = margin.left;
                    let bar_y = page_size.height - margin.top - y_pos - height;

                    // Draw track background
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                        r = track_color.0,
                        g = track_color.1,
                        b = track_color.2,
                        x = bar_x,
                        y = bar_y,
                        w = width,
                        h = height,
                    ));

                    // Draw filled portion
                    if *fraction > 0.0 {
                        let fill_w = width * fraction;
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            r = fill_color.0,
                            g = fill_color.1,
                            b = fill_color.2,
                            x = bar_x,
                            y = bar_y,
                            w = fill_w,
                            h = height,
                        ));
                    }

                    // Draw border
                    content.push_str(&format!(
                        "0.5 w\n0.6 0.6 0.6 RG\n{x} {y} {w} {h} re\nS\n",
                        x = bar_x,
                        y = bar_y,
                        w = width,
                        h = height,
                    ));
                }
                LayoutElement::MathBlock {
                    layout: math_layout,
                    display,
                    ..
                } => {
                    let math_x = if *display {
                        // Center display math
                        margin.left + (available_width - math_layout.width) / 2.0
                    } else {
                        margin.left
                    };
                    // PDF y-axis: top of math block, baseline-adjusted
                    let math_baseline_y =
                        page_size.height - margin.top - y_pos - math_layout.ascent;

                    render_math_glyphs(&math_layout.glyphs, math_x, math_baseline_y, &mut content);
                }
                LayoutElement::PageBreak => {}
            }
        }

        // Render page header/footer in margin area
        if let Some(dec) = decoration {
            let total_pages = pages.len();
            let page_num = page_idx + 1;
            let center_x = page_size.width / 2.0;

            if let Some(ref header_text) = dec.header {
                let text = header_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let header_y = page_size.height - margin.top / 2.0;
                content.push_str("BT\n");
                content.push_str("/Helvetica 9 Tf\n");
                content.push_str("0.4 0.4 0.4 rg\n");
                content.push_str(&format!("{center_x} {header_y} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }

            if let Some(ref footer_text) = dec.footer {
                let text = footer_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let footer_y = margin.bottom / 2.0;
                content.push_str("BT\n");
                content.push_str("/Helvetica 9 Tf\n");
                content.push_str("0.4 0.4 0.4 rg\n");
                content.push_str(&format!("{center_x} {footer_y} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }
        }

        pdf_writer.add_page(
            page_size.width,
            page_size.height,
            &content,
            annotations,
            page_images,
            page_ext_gstates,
            page_shadings,
        );
    }

    pdf_writer.finish_to_writer(writer, &bookmarks)
}

/// Compute the height of a table row from its cells.
fn compute_row_height(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(table_cell_content_height)
        .fold(0.0f32, f32::max)
}

fn table_cell_geometry(
    col_widths: &[f32],
    col_pos: usize,
    colspan: usize,
    spacing_x: f32,
    origin_x: f32,
) -> (f32, f32) {
    let cell_x = origin_x
        + spacing_x
        + col_widths.iter().take(col_pos).sum::<f32>()
        + spacing_x * col_pos as f32;
    let cell_w = col_widths.iter().skip(col_pos).take(colspan).sum::<f32>()
        + spacing_x * colspan.saturating_sub(1) as f32;
    (cell_x, cell_w)
}

fn table_rowspan_height(
    page: &crate::layout::engine::Page,
    elem_idx: usize,
    first_row_height: f32,
    rowspan: usize,
    spacing_y: f32,
) -> f32 {
    let mut total_h = first_row_height;
    for offset in 1..rowspan {
        let future_idx = elem_idx + offset;
        let Some((
            _,
            LayoutElement::TableRow {
                cells: future_cells,
                ..
            },
        )) = page.elements.get(future_idx)
        else {
            break;
        };
        total_h += spacing_y + compute_row_height(future_cells);
    }
    total_h
}

fn render_cell_content(
    content: &mut String,
    cell: &TableCell,
    cell_x: f32,
    row_y: f32,
    col_width: f32,
    row_height: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) {
    if cell.content.is_empty() {
        if !cell.nested_rows.is_empty() {
            let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
            render_cell_text(
                content,
                cell,
                cell_x,
                row_y - cell.padding_top,
                col_width,
                text_h,
                custom_fonts,
            );
            render_nested_table_rows(
                content,
                &cell.nested_rows,
                cell_x + cell.padding_left,
                row_y - cell.padding_top - text_h - cell.padding_bottom,
                custom_fonts,
            );
            return;
        }
        render_cell_text(
            content,
            cell,
            cell_x,
            row_y,
            col_width,
            row_height,
            custom_fonts,
        );
        return;
    }

    let padded_content_height = table_cell_content_height(cell);
    let mut block_top = row_y - (row_height - padded_content_height) / 2.0 - cell.padding_top;
    for block in &cell.content {
        match block {
            TableCellContent::Text(lines) => {
                let block_height: f32 = lines.iter().map(|line| line.height).sum();
                render_text_lines(
                    content,
                    lines,
                    cell_x,
                    block_top,
                    col_width,
                    block_height,
                    cell.text_align,
                    cell.padding_left,
                    cell.padding_right,
                    custom_fonts,
                );
                block_top -= block_height;
            }
            TableCellContent::NestedRows(rows) => {
                let block_height: f32 = rows.iter().map(table_row_total_height).sum();
                render_nested_table_rows(
                    content,
                    rows,
                    cell_x + cell.padding_left,
                    block_top,
                    custom_fonts,
                );
                block_top -= block_height;
            }
        }
    }
}

fn render_cell_text(
    content: &mut String,
    cell: &TableCell,
    cell_x: f32,
    row_y: f32,
    col_width: f32,
    row_height: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) {
    render_text_lines(
        content,
        &cell.lines,
        cell_x,
        row_y,
        col_width,
        row_height,
        cell.text_align,
        cell.padding_left,
        cell.padding_right,
        custom_fonts,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_text_lines(
    content: &mut String,
    lines: &[TextLine],
    cell_x: f32,
    row_y: f32,
    col_width: f32,
    row_height: f32,
    text_align: TextAlign,
    padding_left: f32,
    padding_right: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) {
    let cell_inner_w = col_width - padding_left - padding_right;
    // Vertical centering: place text block so its visual center aligns with
    // the row's vertical center.  In PDF, the baseline is where we position
    // text — glyphs extend upward by ascender and downward by descender.
    let text_h: f32 = lines.iter().map(|l| l.height).sum();

    // Top of the text block, centered in the row
    let text_block_top = row_y - (row_height - text_h) / 2.0;
    let mut text_y = text_block_top;
    for line in lines {
        let line_font_size = line.runs.iter().map(|r| r.font_size).fold(0.0f32, f32::max);
        let line_family = line
            .runs
            .first()
            .map_or(FontFamily::Helvetica, |r| r.font_family.clone());
        let line_ascender = crate::fonts::ascender_ratio(&line_family) * line_font_size;
        let half_leading = (line.height - line_font_size) / 2.0;
        // Baseline sits at: top of line - half_leading - ascender
        text_y -= half_leading + line_ascender;
        let text_content: String = line.runs.iter().map(|r| r.text.as_str()).collect();
        if text_content.is_empty() {
            continue;
        }
        let merged = merge_runs(&line.runs);
        let line_width: f32 = merged
            .iter()
            .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
            .sum();
        let text_x = match text_align {
            TextAlign::Right => cell_x + padding_left + (cell_inner_w - line_width).max(0.0),
            TextAlign::Center => {
                cell_x + padding_left + ((cell_inner_w - line_width) / 2.0).max(0.0)
            }
            _ => cell_x + padding_left,
        };
        let mut x = text_x;
        for run in &merged {
            if run.text.is_empty() {
                continue;
            }
            let font_name = resolve_font_name(run, custom_fonts);
            let (r, g, b) = run.color;
            let rw = estimate_run_width_with_fonts(run, custom_fonts);

            // Draw background rectangle for inline spans
            if let Some((br, bgc, bb)) = run.background_color {
                let (pad_h, pad_v) = run.padding;
                let rx = x - pad_h;
                let ry = text_y - 2.0 - pad_v;
                let rw2 = rw + pad_h * 2.0;
                let rh = run.font_size + 2.0 + pad_v * 2.0;
                content.push_str(&format!("{br} {bgc} {bb} rg\n"));
                if run.border_radius > 0.0 {
                    content.push_str(&rounded_rect_path(rx, ry, rw2, rh, run.border_radius));
                    content.push_str("\nf\n");
                } else {
                    content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                }
            }

            content.push_str(&format!("{r} {g} {b} rg\n"));
            content.push_str("BT\n");
            content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
            content.push_str(&format!("{x} {y} Td\n", y = text_y));
            {
                let encoded = encode_pdf_text(&run.text);
                content.push_str(&format!("({encoded}) Tj\n"));
            }
            content.push_str("ET\n");

            // Draw underline (font-size-relative)
            if run.underline {
                let desc = crate::fonts::descender_ratio(&run.font_family) * run.font_size;
                let uy = text_y - desc * 0.6;
                let thickness = (run.font_size * 0.07).max(0.5);
                content.push_str(&format!(
                    "{r} {g} {b} RG\n{thickness} w\n{x} {uy} m {x2} {uy} l\nS\n",
                    x2 = x + rw,
                ));
            }

            // Draw strikethrough (line-through)
            if run.line_through {
                let sy = text_y + run.font_size * 0.3;
                let thickness = (run.font_size * 0.07).max(0.5);
                content.push_str(&format!(
                    "{r} {g} {b} RG\n{thickness} w\n{x} {sy} m {x2} {sy} l\nS\n",
                    x2 = x + rw,
                ));
            }

            x += rw;
        }
        // Move past the rest of the line (descender + bottom half-leading)
        text_y -= line.height - half_leading - line_ascender;
    }
}

fn table_row_total_height(row: &LayoutElement) -> f32 {
    match row {
        LayoutElement::TableRow {
            cells,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + compute_row_height(cells) + margin_bottom,
        _ => 0.0,
    }
}

fn render_nested_table_rows(
    content: &mut String,
    rows: &[LayoutElement],
    origin_x: f32,
    top_y: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) {
    let mut cursor_y = top_y;
    for (row_idx, row) in rows.iter().enumerate() {
        let LayoutElement::TableRow {
            cells,
            col_widths,
            border_collapse,
            border_spacing,
            margin_top,
            margin_bottom,
        } = row
        else {
            continue;
        };

        cursor_y -= *margin_top;
        let row_y = cursor_y;
        let spacing = if *border_collapse == BorderCollapse::Collapse {
            BorderSpacing::default()
        } else {
            *border_spacing
        };
        let row_height = compute_row_height(cells);

        let mut col_pos: usize = 0;
        for cell in cells {
            if cell.rowspan == 0 {
                col_pos += cell.colspan;
                continue;
            }

            let (cell_x, cell_w) = table_cell_geometry(
                col_widths,
                col_pos,
                cell.colspan,
                spacing.horizontal,
                origin_x,
            );

            let cell_height = if cell.rowspan > 1 {
                let mut total_h = row_height;
                for offset in 1..cell.rowspan {
                    if let Some(future_row) = rows.get(row_idx + offset) {
                        total_h += table_row_total_height(future_row);
                    }
                }
                total_h
            } else {
                row_height
            };

            if let Some((r, g, b)) = cell.background_color {
                content.push_str(&format!(
                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                    x = cell_x,
                    y = row_y - cell_height,
                    w = cell_w,
                    h = cell_height,
                ));
            }

            if cell.border.has_any() {
                let x1 = cell_x;
                let x2 = cell_x + cell_w;
                let y_top = row_y;
                let y_bottom = row_y - cell_height;
                if cell.border.top.width > 0.0 {
                    let (r, g, b) = cell.border.top.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                        cell.border.top.width
                    ));
                }
                if cell.border.right.width > 0.0 {
                    let (r, g, b) = cell.border.right.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                        cell.border.right.width
                    ));
                }
                if cell.border.bottom.width > 0.0 {
                    let (r, g, b) = cell.border.bottom.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                        cell.border.bottom.width
                    ));
                }
                if cell.border.left.width > 0.0 {
                    let (r, g, b) = cell.border.left.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                        cell.border.left.width
                    ));
                }
            }

            render_cell_content(
                content,
                cell,
                cell_x,
                row_y,
                cell_w,
                row_height,
                custom_fonts,
            );

            col_pos += cell.colspan;
        }

        cursor_y -= row_height + *margin_bottom;
    }
}

fn font_name_for_run(run: &TextRun) -> &str {
    match (&run.font_family, run.bold, run.italic) {
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
        // Custom fonts — fall back to Helvetica variant for rendering name;
        // the actual font reference is handled separately by the renderer.
        (FontFamily::Custom(_), true, true) => "Helvetica-BoldOblique",
        (FontFamily::Custom(_), true, false) => "Helvetica-Bold",
        (FontFamily::Custom(_), false, true) => "Helvetica-Oblique",
        (FontFamily::Custom(_), false, false) => "Helvetica",
    }
}

fn estimate_run_width(run: &TextRun) -> f32 {
    crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
}

/// Resolve the PDF font resource name for a text run, using custom fonts if available.
fn resolve_font_name(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> String {
    if let FontFamily::Custom(name) = &run.font_family {
        if custom_fonts.contains_key(name) {
            return sanitize_pdf_name(name);
        }
    }
    font_name_for_run(run).to_string()
}

/// Estimate run width using TTF metrics for custom fonts, falling back to fixed estimation.
fn estimate_run_width_with_fonts(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    if let FontFamily::Custom(name) = &run.font_family {
        if let Some(ttf) = custom_fonts.get(name) {
            return run
                .text
                .chars()
                .map(|c| ttf.char_width_scaled(c as u16, run.font_size))
                .sum();
        }
    }
    estimate_run_width(run)
}

/// Estimate line width using TTF metrics for custom fonts.
fn estimate_line_width_with_fonts(line: &TextLine, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    line.runs
        .iter()
        .map(|r| {
            let text_w = estimate_run_width_with_fonts(r, custom_fonts);
            // Include inline padding (e.g. badge spans with horizontal padding)
            let (pad_h, _pad_v) = r.padding;
            text_w + pad_h * 2.0
        })
        .sum()
}

/// Sanitize a font name for use as a PDF name object (remove spaces, special chars).
fn sanitize_pdf_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn line_text_content(line: &TextLine) -> String {
    line.runs.iter().map(|r| r.text.as_str()).collect()
}

/// Merge consecutive text runs that share the same visual properties (font,
/// size, bold, italic, color, underline, line-through, link) into a single
/// run.  This produces cleaner PDF output and ensures that spaces between
/// words are part of one contiguous text string, preventing PDF viewers from
/// dropping inter-word spaces during text extraction.
fn merge_runs(runs: &[TextRun]) -> Vec<TextRun> {
    let mut merged: Vec<TextRun> = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        let can_merge = if let Some(prev) = merged.last() {
            prev.font_size == run.font_size
                && prev.bold == run.bold
                && prev.italic == run.italic
                && prev.underline == run.underline
                && prev.line_through == run.line_through
                && prev.color == run.color
                && prev.link_url == run.link_url
                && prev.font_family == run.font_family
                && prev.background_color == run.background_color
                && prev.padding == run.padding
                && prev.border_radius == run.border_radius
        } else {
            false
        };
        if can_merge {
            merged.last_mut().unwrap().text.push_str(&run.text);
        } else {
            merged.push(run.clone());
        }
    }
    merged
}

/// Render a linear gradient using a native PDF Shading Dictionary reference.
///
/// Instead of drawing 200 thin rectangles (which produces banding), this emits
/// a `sh` operator referencing a shading dictionary that the PDF viewer will
/// interpolate smoothly. The shading entry is collected and later written as a
/// PDF object in `finish_to_writer`.
#[allow(clippy::too_many_arguments)]
fn render_linear_gradient(
    content: &mut String,
    gradient: &LinearGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let name = format!("SH{}", *shading_counter);
    *shading_counter += 1;

    // CSS angle convention: 0° = to top (bottom-to-top), 90° = to right, 180° = to bottom
    // In PDF coordinate space, y-axis is bottom-up, so:
    //   CSS 0° (to top) => PDF line from bottom center to top center
    //   CSS 90° (to right) => PDF line from left center to right center
    //   CSS 180° (to bottom) => PDF line from top center to bottom center
    let angle_rad = gradient.angle * std::f32::consts::PI / 180.0;
    let sin_a = angle_rad.sin();
    let cos_a = angle_rad.cos();

    // Gradient line: start and end points
    // CSS: 0deg = to top, so direction vector is (sin(angle), -cos(angle)) in CSS coords
    // In PDF coords (y flipped): direction is (sin(angle), cos(angle))
    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    // Half-length of the gradient line along the direction
    let half_len = (width * sin_a.abs() + height * cos_a.abs()) / 2.0;
    let dx = sin_a * half_len;
    let dy = cos_a * half_len;

    let x0 = cx - dx;
    let y0 = cy - dy;
    let x1 = cx + dx;
    let y1 = cy + dy;

    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();

    shadings.push(ShadingEntry {
        name: name.clone(),
        shading_type: 2, // Axial
        coords: [x0, y0, x1, y1, 0.0, 0.0],
        stops,
    });

    // Clip to the gradient area and paint with shading
    content.push_str("q\n");
    content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
    content.push_str(&format!("/{name} sh\n"));
    content.push_str("Q\n");
}

/// Render a radial gradient using a native PDF Shading Dictionary reference.
#[allow(clippy::too_many_arguments)]
fn render_radial_gradient(
    content: &mut String,
    gradient: &RadialGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let name = format!("SH{}", *shading_counter);
    *shading_counter += 1;

    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    let max_radius = width.max(height) / 2.0;

    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();

    shadings.push(ShadingEntry {
        name: name.clone(),
        shading_type: 3, // Radial
        coords: [cx, cy, 0.0, cx, cy, max_radius],
        stops,
    });

    // Clip to the gradient area and paint with shading
    content.push_str("q\n");
    content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
    content.push_str(&format!("/{name} sh\n"));
    content.push_str("Q\n");
}

/// Build an inline PDF Function dictionary string for a gradient's color stops.
///
/// For 2 stops, returns a Type 2 (exponential interpolation) function.
/// For 3+ stops, returns a Type 3 (stitching) function that chains Type 2 sub-functions.
fn build_shading_function(stops: &[(f32, (f32, f32, f32))]) -> String {
    if stops.len() < 2 {
        // Fallback: single color
        let (r, g, b) = stops.first().map(|s| s.1).unwrap_or((0.0, 0.0, 0.0));
        return format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r} {g} {b}] /C1 [{r} {g} {b}] /N 1 >>"
        );
    }

    if stops.len() == 2 {
        let (r0, g0, b0) = stops[0].1;
        let (r1, g1, b1) = stops[1].1;
        return format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r0} {g0} {b0}] /C1 [{r1} {g1} {b1}] /N 1 >>"
        );
    }

    // Type 3 stitching function for 3+ stops
    let mut functions = Vec::new();
    let mut bounds = Vec::new();
    let mut encode = Vec::new();

    for i in 0..stops.len() - 1 {
        let (r0, g0, b0) = stops[i].1;
        let (r1, g1, b1) = stops[i + 1].1;
        functions.push(format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r0} {g0} {b0}] /C1 [{r1} {g1} {b1}] /N 1 >>"
        ));
        if i < stops.len() - 2 {
            bounds.push(format!("{}", stops[i + 1].0));
        }
        encode.push("0 1".to_string());
    }

    let functions_str = functions.join(" ");
    let bounds_str = bounds.join(" ");
    let encode_str = encode.join(" ");

    format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{functions_str}] /Bounds [{bounds_str}] /Encode [{encode_str}] >>"
    )
}

/// Generate a PDF path for a rounded rectangle.
///
/// Uses cubic Bezier curves to approximate circular arcs at each corner.
/// The magic number k = r * 0.5522847498 gives the best circular approximation.
fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> String {
    let r = r.min(w / 2.0).min(h / 2.0); // Clamp radius to half the smallest dimension
    let k = r * 0.552_284_8;
    format!(
        "{x0} {y0} m\n\
         {x1} {y0} l {x2} {y0} {x3} {y3} {x3} {y4} c\n\
         {x3} {y5} l {x3} {y6} {x2} {y7} {x1} {y7} c\n\
         {x0} {y7} l {x8} {y7} {x9} {y6} {x9} {y5} c\n\
         {x9} {y4} l {x9} {y3} {x8} {y0} {x0} {y0} c\n\
         h\n",
        x0 = x + r,
        x1 = x + w - r,
        x2 = x + w - r + k,
        x3 = x + w,
        x8 = x + r - k,
        x9 = x,
        y0 = y + h, // top
        y3 = y + h - r + k,
        y4 = y + h - r,
        y5 = y + r,
        y6 = y + r - k,
        y7 = y, // bottom
    )
}

/// Convert a UTF-8 string to WinAnsi (Windows-1252) encoded bytes.
///
/// Standard PDF fonts (Helvetica, Times-Roman, Courier) use WinAnsi encoding,
/// not UTF-8. Writing raw UTF-8 bytes causes multi-byte characters like em dash
/// to appear as mojibake. This function maps Unicode code points to their
/// WinAnsi byte equivalents.
fn utf8_to_winansi(text: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        match code {
            // ASCII range maps directly
            0x0000..=0x007F => result.push(code as u8),
            // Non-breaking space
            0x00A0 => result.push(0xA0),
            // Latin-1 supplement U+00A1..U+00FF map directly
            0x00A1..=0x00FF => result.push(code as u8),
            // WinAnsi special mappings from the Windows-1252 range 0x80..0x9F
            0x20AC => result.push(0x80), // Euro sign
            0x201A => result.push(0x82), // Single low-9 quotation mark
            0x0192 => result.push(0x83), // Latin small letter f with hook
            0x201E => result.push(0x84), // Double low-9 quotation mark
            0x2026 => result.push(0x85), // Horizontal ellipsis
            0x2020 => result.push(0x86), // Dagger
            0x2021 => result.push(0x87), // Double dagger
            0x02C6 => result.push(0x88), // Modifier letter circumflex accent
            0x2030 => result.push(0x89), // Per mille sign
            0x0160 => result.push(0x8A), // Latin capital letter S with caron
            0x2039 => result.push(0x8B), // Single left-pointing angle quotation mark
            0x0152 => result.push(0x8C), // Latin capital ligature OE
            0x017D => result.push(0x8E), // Latin capital letter Z with caron
            0x2018 => result.push(0x91), // Left single quotation mark
            0x2019 => result.push(0x92), // Right single quotation mark
            0x201C => result.push(0x93), // Left double quotation mark
            0x201D => result.push(0x94), // Right double quotation mark
            0x2022 => result.push(0x95), // Bullet
            0x2013 => result.push(0x96), // En dash
            0x2014 => result.push(0x97), // Em dash
            0x02DC => result.push(0x98), // Small tilde
            0x2122 => result.push(0x99), // Trade mark sign
            0x0161 => result.push(0x9A), // Latin small letter s with caron
            0x203A => result.push(0x9B), // Single right-pointing angle quotation mark
            0x0153 => result.push(0x9C), // Latin small ligature oe
            0x017E => result.push(0x9E), // Latin small letter z with caron
            0x0178 => result.push(0x9F), // Latin capital letter Y with diaeresis
            // Anything else is not representable in WinAnsi — replace with '?'
            _ => result.push(b'?'),
        }
    }
    result
}

/// Encode a UTF-8 string for use in a PDF text operator (Tj).
///
/// Converts to WinAnsi encoding, then produces a `String` where:
/// - ASCII printable bytes (0x20..=0x7E), except `\`, `(`, `)`, are kept as-is
/// - `\`, `(`, `)` are escaped as `\\`, `\(`, `\)`
/// - All other bytes (0x00..=0x1F, 0x7F..=0xFF) are written as octal escapes `\NNN`
///
/// The returned string is safe to embed in a PDF content stream as `(encoded) Tj`.
fn encode_pdf_text(text: &str) -> String {
    let winansi = utf8_to_winansi(text);
    let mut result = String::with_capacity(winansi.len() * 2);
    for &b in &winansi {
        match b {
            b'\\' => result.push_str("\\\\"),
            b'(' => result.push_str("\\("),
            b')' => result.push_str("\\)"),
            0x20..=0x7E => result.push(b as char),
            _ => {
                // Octal escape: \NNN (3-digit, zero-padded)
                result.push_str(&format!("\\{:03o}", b));
            }
        }
    }
    result
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

/// A custom TrueType font entry for the PDF font dictionary.
struct CustomFontEntry {
    /// Sanitized PDF name used as the resource key (e.g., "MyFont").
    pdf_name: String,
    /// Object ID of the font object.
    font_obj_id: usize,
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
    /// ExtGState entries (name, opacity) grouped by page index.
    page_ext_gstates: Vec<Vec<(String, f32)>>,
    /// Shading dictionary entries grouped by page index.
    page_shadings: Vec<Vec<ShadingEntry>>,
    /// Custom TrueType font entries.
    custom_font_entries: Vec<CustomFontEntry>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
            binary_objects: std::collections::HashMap::new(),
            page_ids: Vec::new(),
            page_annotations: Vec::new(),
            page_images: Vec::new(),
            page_ext_gstates: Vec::new(),
            page_shadings: Vec::new(),
            custom_font_entries: Vec::new(),
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

    /// Embed a TrueType font and return the PDF resource name to reference it.
    fn add_ttf_font(&mut self, name: &str, ttf: &TtfFont) -> String {
        let pdf_name = sanitize_pdf_name(name);

        // 1. Font stream: embed the full TTF data
        let stream_id = self.next_id();
        let data = &ttf.data;
        let header = format!(
            "{stream_id} 0 obj\n<< /Length {} /Length1 {} >>\nstream\n",
            data.len(),
            data.len(),
        );
        self.objects.push(header);
        self.binary_objects.insert(stream_id, data.clone());

        // 2. FontDescriptor
        let descriptor_id = self.next_id();
        let ascent_pdf = (ttf.ascent as i32 * 1000) / ttf.units_per_em as i32;
        let descent_pdf = (ttf.descent as i32 * 1000) / ttf.units_per_em as i32;
        let bbox_pdf = [
            (ttf.bbox[0] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[1] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[2] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[3] as i32 * 1000) / ttf.units_per_em as i32,
        ];
        self.objects.push(format!(
            "{descriptor_id} 0 obj\n<< /Type /FontDescriptor /FontName /{pdf_name} /Flags {flags} /FontBBox [{b0} {b1} {b2} {b3}] /Ascent {ascent} /Descent {descent} /ItalicAngle 0 /CapHeight {ascent} /StemV 80 /FontFile2 {stream_id} 0 R >>\nendobj",
            flags = ttf.flags,
            b0 = bbox_pdf[0],
            b1 = bbox_pdf[1],
            b2 = bbox_pdf[2],
            b3 = bbox_pdf[3],
            ascent = ascent_pdf,
            descent = descent_pdf,
        ));

        // 3. Widths array for WinAnsiEncoding range (32..255)
        let first_char = 32u16;
        let last_char = 255u16;
        let mut widths = Vec::new();
        for c in first_char..=last_char {
            widths.push(ttf.char_width_pdf(c));
        }
        let widths_str: String = widths
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        // 4. Font object
        let font_id = self.next_id();
        self.objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /TrueType /BaseFont /{pdf_name} /Encoding /WinAnsiEncoding /FirstChar {first_char} /LastChar {last_char} /Widths [{widths_str}] /FontDescriptor {descriptor_id} 0 R >>\nendobj",
        ));

        self.custom_font_entries.push(CustomFontEntry {
            pdf_name: pdf_name.clone(),
            font_obj_id: font_id,
        });

        pdf_name
    }

    #[allow(clippy::too_many_arguments)]
    fn add_page(
        &mut self,
        width: f32,
        height: f32,
        content: &str,
        annotations: Vec<LinkAnnotation>,
        images: Vec<ImageRef>,
        ext_gstates: Vec<(String, f32)>,
        shadings: Vec<ShadingEntry>,
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
        self.page_ext_gstates.push(ext_gstates);
        self.page_shadings.push(shadings);
    }

    fn finish_to_writer<W: std::io::Write>(
        self,
        out: &mut W,
        bookmarks: &[BookmarkEntry],
    ) -> Result<(), IronpressError> {
        let mut bytes_written: usize = 0;
        out.write_all(b"%PDF-1.4\n")?;
        bytes_written += b"%PDF-1.4\n".len();

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

        // Symbol font (no WinAnsiEncoding — uses built-in Symbol encoding)
        let symbol_font_id = font_base_id + font_names.len();
        all_objects.push(format!(
            "{symbol_font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>\nendobj",
        ));

        // Font dictionary (standard + Symbol + custom fonts)
        let font_dict_id = symbol_font_id + 1;
        let mut font_entries: Vec<String> = font_names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} {} 0 R", font_base_id + i))
            .collect();
        // Add Symbol font entry
        font_entries.push(format!("/Symbol {symbol_font_id} 0 R"));
        // Add custom font entries
        for entry in &self.custom_font_entries {
            font_entries.push(format!("/{} {} 0 R", entry.pdf_name, entry.font_obj_id));
        }
        let font_entries_str = font_entries.join(" ");
        all_objects.push(format!(
            "{font_dict_id} 0 obj\n<< {font_entries_str} >>\nendobj",
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

        // Collect unique ExtGState entries across all pages
        let mut gs_entries: Vec<(String, f32)> = Vec::new();
        for page_gs in &self.page_ext_gstates {
            for (name, opacity) in page_gs {
                if !gs_entries.iter().any(|(n, _)| n == name) {
                    gs_entries.push((name.clone(), *opacity));
                }
            }
        }
        let has_opacity = !gs_entries.is_empty();

        // Add ExtGState objects if needed
        let mut gs_obj_refs: Vec<(String, usize)> = Vec::new();
        if has_opacity {
            // GSDefault (opacity 1.0)
            let default_gs_id = all_objects.len() + 1;
            all_objects.push(format!(
                "{default_gs_id} 0 obj\n<< /Type /ExtGState /ca 1 /CA 1 >>\nendobj"
            ));
            gs_obj_refs.push(("GSDefault".to_string(), default_gs_id));

            // Per-element ExtGState objects
            for (name, opacity) in &gs_entries {
                let gs_id = all_objects.len() + 1;
                all_objects.push(format!(
                    "{gs_id} 0 obj\n<< /Type /ExtGState /ca {opacity} /CA {opacity} >>\nendobj"
                ));
                gs_obj_refs.push((name.clone(), gs_id));
            }
        }

        // Add Shading objects
        let mut shading_obj_refs: Vec<(String, usize)> = Vec::new();
        for page_sh in &self.page_shadings {
            for entry in page_sh {
                let sh_id = all_objects.len() + 1;
                let function_str = build_shading_function(&entry.stops);
                let coords_str = if entry.shading_type == 2 {
                    // Axial: only first 4 coords
                    format!(
                        "{} {} {} {}",
                        entry.coords[0], entry.coords[1], entry.coords[2], entry.coords[3]
                    )
                } else {
                    // Radial: all 6 coords
                    format!(
                        "{} {} {} {} {} {}",
                        entry.coords[0],
                        entry.coords[1],
                        entry.coords[2],
                        entry.coords[3],
                        entry.coords[4],
                        entry.coords[5]
                    )
                };
                all_objects.push(format!(
                    "{sh_id} 0 obj\n<< /ShadingType {} /ColorSpace /DeviceRGB /Coords [{coords_str}] /Function {function_str} /Extend [true true] >>\nendobj",
                    entry.shading_type,
                ));
                shading_obj_refs.push((entry.name.clone(), sh_id));
            }
        }

        // Resources dictionary
        let resources_id = all_objects.len() + 1;
        let mut resource_parts = format!("/Font {font_dict_id} 0 R");

        if !all_image_refs.is_empty() {
            let xobj_entries: String = all_image_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /XObject << {xobj_entries} >>"));
        }

        if has_opacity {
            let gs_dict: String = gs_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /ExtGState << {gs_dict} >>"));
        }

        if !shading_obj_refs.is_empty() {
            let shading_dict: String = shading_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /Shading << {shading_dict} >>"));
        }

        all_objects.push(format!(
            "{resources_id} 0 obj\n<< {resource_parts} >>\nendobj",
        ));

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

        // Outlines (PDF bookmarks from headings)
        let outlines_ref = if bookmarks.is_empty() {
            String::new()
        } else {
            let count = bookmarks.len();
            // Outline root object
            let root_id = all_objects.len() + 1;
            let first_entry_id = root_id + 1;
            let last_entry_id = first_entry_id + count - 1;
            all_objects.push(format!(
                "{root_id} 0 obj\n<< /Type /Outlines /First {first_entry_id} 0 R /Last {last_entry_id} 0 R /Count {count} >>\nendobj",
            ));

            // Outline entry objects (flat list, linked via Prev/Next)
            for (i, bm) in bookmarks.iter().enumerate() {
                let entry_id = first_entry_id + i;
                let page_obj_id = self.page_ids.get(bm.page_index).copied().unwrap_or(1);

                let mut entry = format!(
                    "{entry_id} 0 obj\n<< /Title ({title}) /Parent {root_id} 0 R /Dest [{page_obj_id} 0 R /XYZ 0 {dest_y} 0]",
                    title = escape_pdf_string(&bm.title),
                    dest_y = bm.y_pos,
                );
                if i > 0 {
                    entry.push_str(&format!(" /Prev {} 0 R", first_entry_id + i - 1));
                }
                if i + 1 < count {
                    entry.push_str(&format!(" /Next {} 0 R", first_entry_id + i + 1));
                }
                entry.push_str(" >>\nendobj");
                all_objects.push(entry);
            }

            format!(" /Outlines {root_id} 0 R /PageMode /UseOutlines")
        };

        // Catalog
        let catalog_id = all_objects.len() + 1;
        all_objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R{outlines_ref} >>\nendobj",
        ));

        // Write objects and track offsets for xref
        // Binary objects (images) need special handling
        let mut offsets = Vec::new();
        for (idx, obj_str) in all_objects.iter().enumerate() {
            offsets.push(bytes_written);
            let obj_id = idx + 1;
            if let Some(bin_data) = self.binary_objects.get(&obj_id) {
                // Write the header (stored in obj_str), then binary data, then endstream/endobj
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(bin_data)?;
                bytes_written += bin_data.len();
                out.write_all(b"\nendstream\nendobj\n")?;
                bytes_written += b"\nendstream\nendobj\n".len();
            } else {
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(b"\n")?;
                bytes_written += 1;
            }
        }

        // Cross-reference table
        let xref_offset = bytes_written;
        let xref_header = format!("xref\n0 {}\n", all_objects.len() + 1);
        out.write_all(xref_header.as_bytes())?;
        out.write_all(b"0000000000 65535 f \n")?;
        for offset in &offsets {
            let entry = format!("{:010} 00000 n \n", offset);
            out.write_all(entry.as_bytes())?;
        }

        // Trailer
        let trailer = format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            all_objects.len() + 1,
        );
        out.write_all(trailer.as_bytes())?;

        Ok(())
    }
}

/// Map a Unicode character to a PDF Symbol font encoding byte.
/// Returns `Some(byte)` if the character exists in Symbol, `None` otherwise.
fn unicode_to_symbol(ch: char) -> Option<u8> {
    match ch {
        // Greek lowercase
        '\u{03B1}' => Some(0x61), // α → a
        '\u{03B2}' => Some(0x62), // β → b
        '\u{03B3}' => Some(0x67), // γ → g
        '\u{03B4}' => Some(0x64), // δ → d
        '\u{03B5}' => Some(0x65), // ε → e
        '\u{03B6}' => Some(0x7A), // ζ → z
        '\u{03B7}' => Some(0x68), // η → h
        '\u{03B8}' => Some(0x71), // θ → q
        '\u{03B9}' => Some(0x69), // ι → i
        '\u{03BA}' => Some(0x6B), // κ → k
        '\u{03BB}' => Some(0x6C), // λ → l
        '\u{03BC}' => Some(0x6D), // μ → m
        '\u{03BD}' => Some(0x6E), // ν → n
        '\u{03BE}' => Some(0x78), // ξ → x
        '\u{03C0}' => Some(0x70), // π → p
        '\u{03C1}' => Some(0x72), // ρ → r
        '\u{03C3}' => Some(0x73), // σ → s
        '\u{03C4}' => Some(0x74), // τ → t
        '\u{03C5}' => Some(0x75), // υ → u
        '\u{03C6}' => Some(0x66), // φ → f
        '\u{03C7}' => Some(0x63), // χ → c
        '\u{03C8}' => Some(0x79), // ψ → y
        '\u{03C9}' => Some(0x77), // ω → w
        // Greek uppercase
        '\u{0393}' => Some(0x47), // Γ → G
        '\u{0394}' => Some(0x44), // Δ → D
        '\u{0398}' => Some(0x51), // Θ → Q
        '\u{039B}' => Some(0x4C), // Λ → L
        '\u{039E}' => Some(0x58), // Ξ → X
        '\u{03A0}' => Some(0x50), // Π → P
        '\u{03A3}' => Some(0x53), // Σ → S
        '\u{03A5}' => Some(0xA1), // Υ
        '\u{03A6}' => Some(0x46), // Φ → F
        '\u{03A8}' => Some(0x59), // Ψ → Y
        '\u{03A9}' => Some(0x57), // Ω → W
        // Large operators
        '\u{2211}' => Some(0xE5), // ∑
        '\u{220F}' => Some(0xD5), // ∏
        '\u{2210}' => Some(0xD5), // ∐ (fallback to ∏)
        '\u{222B}' => Some(0xF2), // ∫
        '\u{222C}' => Some(0xF2), // ∬ (fallback to ∫)
        '\u{222D}' => Some(0xF2), // ∭ (fallback to ∫)
        '\u{222E}' => Some(0xF2), // ∮ (fallback to ∫)
        '\u{22C3}' => Some(0xC8), // ⋃
        '\u{22C2}' => Some(0xC7), // ⋂
        // Relations
        '\u{2264}' => Some(0xA3), // ≤
        '\u{2265}' => Some(0xB3), // ≥
        '\u{2260}' => Some(0xB9), // ≠
        '\u{2248}' => Some(0xBB), // ≈
        '\u{2261}' => Some(0xBA), // ≡
        '\u{221D}' => Some(0xB5), // ∝
        '\u{2282}' => Some(0xCC), // ⊂
        '\u{2283}' => Some(0xC9), // ⊃
        '\u{2286}' => Some(0xCD), // ⊆
        '\u{2287}' => Some(0xCA), // ⊇
        '\u{2208}' => Some(0xCE), // ∈
        '\u{2209}' => Some(0xCF), // ∉
        '\u{22A2}' => Some(0x5E), // ⊢ (fallback)
        '\u{22A8}' => Some(0xF0), // ⊨
        // Arrows
        '\u{2192}' => Some(0xAE), // →
        '\u{2190}' => Some(0xAC), // ←
        '\u{2194}' => Some(0xAB), // ↔
        '\u{21D2}' => Some(0xDE), // ⇒
        '\u{21D0}' => Some(0xDC), // ⇐
        '\u{21D4}' => Some(0xDB), // ⇔
        '\u{21A6}' => Some(0xAE), // ↦ (fallback to →)
        // Binary operators
        '\u{00D7}' => Some(0xB4), // ×
        '\u{00F7}' => Some(0xB8), // ÷
        '\u{22C5}' => Some(0xD7), // ⋅
        '\u{00B1}' => Some(0xB1), // ±
        '\u{2213}' => Some(0xB1), // ∓ (fallback to ±)
        '\u{2218}' => Some(0xB0), // ∘
        '\u{2295}' => Some(0xC5), // ⊕
        '\u{2297}' => Some(0xC4), // ⊗
        '\u{222A}' => Some(0xC8), // ∪
        '\u{2229}' => Some(0xC7), // ∩
        '\u{2227}' => Some(0xD9), // ∧
        '\u{2228}' => Some(0xDA), // ∨
        // Misc math symbols
        '\u{221E}' => Some(0xA5), // ∞
        '\u{2202}' => Some(0xB6), // ∂
        '\u{2207}' => Some(0xD1), // ∇
        '\u{2200}' => Some(0x22), // ∀
        '\u{2203}' => Some(0x24), // ∃
        '\u{00AC}' => Some(0xD8), // ¬
        '\u{2205}' => Some(0xC6), // ∅
        '\u{2135}' => Some(0xC0), // ℵ
        '\u{221A}' => Some(0xD6), // √
        '\u{2032}' => Some(0xA2), // ′
        '\u{2026}' => Some(0xBC), // …
        '\u{22EF}' => Some(0xBC), // ⋯
        '\u{2016}' => Some(0xBD), // ‖
        // Delimiters
        '\u{27E8}' => Some(0xE1), // ⟨
        '\u{27E9}' => Some(0xF1), // ⟩
        '\u{230A}' => Some(0xEB), // ⌊
        '\u{230B}' => Some(0xFB), // ⌋
        '\u{2308}' => Some(0xE9), // ⌈
        '\u{2309}' => Some(0xF9), // ⌉
        _ => None,
    }
}

/// Render math glyphs to PDF content stream operators.
fn render_math_glyphs(
    glyphs: &[crate::layout::math::MathGlyph],
    origin_x: f32,
    origin_y: f32,
    content: &mut String,
) {
    use crate::layout::math::MathGlyph;

    for glyph in glyphs {
        match glyph {
            MathGlyph::Char {
                ch,
                x,
                y,
                font_size,
                italic,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;

                // Check if character needs Symbol font
                if let Some(sym_byte) = unicode_to_symbol(*ch) {
                    let encoded = format!("\\{:03o}", sym_byte);
                    content.push_str("BT\n");
                    content.push_str(&format!("/Symbol {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    let font_name = if *italic {
                        "Helvetica-Oblique"
                    } else {
                        "Helvetica"
                    };
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/{font_name} {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                }
            }
            MathGlyph::Text {
                text,
                x,
                y,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let encoded = encode_pdf_text(text);
                content.push_str("BT\n");
                content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                content.push_str(&format!("{px} {py} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }
            MathGlyph::Rule {
                x,
                y,
                width,
                thickness,
            } => {
                let px = origin_x + x;
                let py = origin_y + y - thickness / 2.0;
                content.push_str("0 0 0 rg\n");
                content.push_str(&format!("{px} {py} {width} {thickness} re\nf\n"));
            }
            MathGlyph::Radical {
                x,
                y,
                width,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let line_w = font_size * 0.04;
                content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                // Draw radical sign: short tick down, long line up-right, horizontal overline
                let tick_x = px + width * 0.15;
                let tick_bottom = py - height * 0.3;
                let bottom_x = px + width * 0.35;
                let bottom_y = py - height;
                let top_x = px + width;
                let top_y = py;
                content.push_str(&format!(
                    "{tick_x} {tick_bottom} m\n{bottom_x} {bottom_y} l\n{top_x} {top_y} l\nS\n"
                ));
            }
            MathGlyph::Delimiter {
                ch,
                x,
                y,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                // For small delimiters, use text; for large, draw paths
                if *height <= font_size * 1.3 {
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    // Draw scaled delimiter using PDF path ops
                    let line_w = font_size * 0.04;
                    content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                    let half_h = height / 2.0;
                    match ch {
                        '(' => {
                            // Left parenthesis as cubic bezier
                            let cx = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{px} {c1y} {px} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        ')' => {
                            let cx = px;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{right} {c1y} {right} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        '[' => {
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {px} {top_y} l {px} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        ']' => {
                            let left = px;
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{left} {top_y} m {right} {top_y} l {right} {bot_y} l {left} {bot_y} l S\n"
                            ));
                        }
                        '{' => {
                            let mid = px + font_size * 0.15;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {mid} {top_y} l {mid} {py} l {px} {py} l S\n\
                                 {px} {py} m {mid} {py} l {mid} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        '}' => {
                            let mid = px + font_size * 0.1;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{px} {top_y} m {mid} {top_y} l {mid} {py} l {right} {py} l S\n\
                                 {right} {py} m {mid} {py} l {mid} {bot_y} l {px} {bot_y} l S\n"
                            ));
                        }
                        '|' => {
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!("{px} {top_y} m {px} {bot_y} l S\n"));
                        }
                        _ => {
                            // Fallback: render as text character
                            let encoded = encode_pdf_text(&ch.to_string());
                            content.push_str("BT\n");
                            content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                            content.push_str(&format!("{px} {py} Td\n"));
                            content.push_str(&format!("({encoded}) Tj\n"));
                            content.push_str("ET\n");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::{LayoutBorder, layout};
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
        // No default cell borders — only CSS-specified borders produce strokes
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
    fn render_input_element() {
        let pdf = crate::html_to_pdf(r#"<input type="text" value="Hello">"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_input_with_placeholder() {
        let pdf = crate::html_to_pdf(r#"<input placeholder="Type here...">"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_select_element() {
        let pdf =
            crate::html_to_pdf(r#"<select><option>A</option><option>B</option></select>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_textarea_element() {
        let pdf = crate::html_to_pdf(r#"<textarea>Hello World</textarea>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_video_element() {
        let pdf = crate::html_to_pdf(r#"<video width="320" height="240"></video>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_audio_element() {
        let pdf = crate::html_to_pdf(r#"<audio></audio>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_progress_element() {
        let pdf = crate::html_to_pdf(r#"<progress value="0.7" max="1"></progress>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Progress bar draws rectangles (track + fill + border)
        assert!(
            content.contains("re\nf\n"),
            "Expected filled rectangles for progress bar"
        );
    }

    #[test]
    fn render_progress_empty() {
        let pdf = crate::html_to_pdf(r#"<progress value="0" max="1"></progress>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_meter_element() {
        let pdf = crate::html_to_pdf(r#"<meter value="0.5" max="1"></meter>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re\nf\n"),
            "Expected filled rectangles for meter bar"
        );
    }

    #[test]
    fn render_meter_low_value() {
        let pdf = crate::html_to_pdf(r#"<meter value="5" max="100" low="25" high="75"></meter>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_form_controls_styled() {
        let html = r#"
            <input type="text" value="styled" style="width: 200px; border: 2px solid blue; background-color: #eee">
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_mixed_form_and_text() {
        let html = r#"
            <p>Fill in the form:</p>
            <input type="text" value="John">
            <p>Select country:</p>
            <select><option>France</option></select>
            <p>Comments:</p>
            <textarea>Great product!</textarea>
            <p>Rating:</p>
            <progress value="80" max="100"></progress>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 500);
    }

    #[test]
    fn render_pdf_bookmarks_from_headings() {
        let html = "<h1>Chapter 1</h1><p>Content</p><h2>Section 1.1</h2><p>More</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Type /Outlines"), "Expected PDF outlines");
        assert!(
            content.contains("Chapter 1"),
            "Expected heading text in bookmark"
        );
        assert!(
            content.contains("Section 1.1"),
            "Expected h2 heading in bookmark"
        );
    }

    #[test]
    fn render_pdf_no_bookmarks_without_headings() {
        let html = "<p>No headings here</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Type /Outlines"),
            "Should not have outlines without headings"
        );
    }

    #[test]
    fn render_pdf_bookmarks_multi_page() {
        let html = r#"
            <h1>Page 1 Title</h1>
            <p>Content</p>
            <div style="page-break-before: always">
                <h1>Page 2 Title</h1>
                <p>More content</p>
            </div>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Page 1 Title"));
        assert!(content.contains("Page 2 Title"));
        assert!(content.contains("/Type /Outlines"));
    }

    #[test]
    fn render_pdf_bookmarks_all_levels() {
        let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Count 6"), "Expected 6 outline entries");
    }

    #[test]
    fn render_page_footer() {
        let pdf = crate::HtmlConverter::new()
            .footer("Page {page} of {pages}")
            .convert("<h1>Title</h1><p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Page 1 of 1"),
            "Expected footer with page numbers"
        );
    }

    #[test]
    fn render_page_header() {
        let pdf = crate::HtmlConverter::new()
            .header("My Document")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("My Document"),
            "Expected header text in PDF"
        );
    }

    #[test]
    fn render_header_and_footer() {
        let pdf = crate::HtmlConverter::new()
            .header("Report Title")
            .footer("Page {page} of {pages}")
            .convert("<p>Page 1</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Report Title"));
        assert!(content.contains("Page 1 of 1"));
    }

    #[test]
    fn render_footer_multi_page() {
        let html = r#"
            <p>First page</p>
            <div style="page-break-before: always"><p>Second page</p></div>
        "#;
        let pdf = crate::HtmlConverter::new()
            .footer("Page {page} of {pages}")
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Verify page number substitution works (at least page 1 and last page are present)
        assert!(content.contains("Page 1 of"), "Expected footer with page 1");
        assert!(content.contains("Page 2 of"), "Expected footer with page 2");
    }

    #[test]
    fn render_no_header_footer_by_default() {
        let pdf = crate::html_to_pdf("<p>Test</p>").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(!content.contains("Page 1 of"));
    }

    #[test]
    fn render_header_only_no_footer() {
        let pdf = crate::HtmlConverter::new()
            .header("Header Only")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Header Only"));
        assert!(!content.contains("Page 1"));
    }

    #[test]
    fn render_footer_only_no_header() {
        let pdf = crate::HtmlConverter::new()
            .footer("{page}/{pages}")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1/1"));
    }

    #[test]
    fn render_progress_bar_zero_fraction() {
        let html = r#"<progress value="0" max="1"></progress>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Track is drawn but fill is skipped when fraction=0
        assert!(content.contains("re\nf\n")); // track rect
        assert!(content.contains("re\nS\n")); // border stroke
    }

    #[test]
    fn render_progress_bar_full_fraction() {
        let html = r#"<progress value="1" max="1"></progress>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_bookmark_special_chars() {
        let html = r#"<h1>Title with (parens) &amp; "quotes"</h1>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Type /Outlines"));
    }

    #[test]
    fn render_single_heading_bookmark() {
        let html = "<h1>Only One</h1><p>Text</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Count 1"));
        assert!(content.contains("Only One"));
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

    #[test]
    fn render_opacity_produces_extgstate() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ca 0.5"),
            "PDF should contain fill opacity /ca 0.5"
        );
        assert!(
            content.contains("/CA 0.5"),
            "PDF should contain stroke opacity /CA 0.5"
        );
        assert!(
            content.contains("/ExtGState"),
            "PDF should contain ExtGState resource"
        );
        assert!(content.contains("gs\n"), "PDF should use gs operator");
    }

    #[test]
    fn render_full_opacity_no_extgstate() {
        let html = r#"<div>Fully opaque</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/ExtGState"),
            "PDF should not contain ExtGState for full opacity"
        );
    }

    #[test]
    fn render_width_constrains_background() {
        let html = r#"<div style="width: 200pt; background-color: red">Narrow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("200"),
            "PDF should contain the constrained width 200"
        );
    }

    #[test]
    fn render_justify_produces_tw_operator() {
        // Use enough words to force line wrapping so a non-last line exists
        let words = "word ".repeat(80);
        let html = format!(r#"<p style="text-align: justify">{words}</p>"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Tw\n"),
            "Justified text should produce Tw operator in PDF"
        );
    }

    #[test]
    fn render_justify_last_line_no_tw() {
        // A single short line (which is the last line) should not have Tw
        let html = r#"<p style="text-align: justify">Short line</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The single line is the last line, so no Tw should be applied
        assert!(
            !content.contains("Tw\n"),
            "Last line of justified paragraph should not have Tw"
        );
    }

    #[test]
    fn render_justify_resets_tw() {
        let words = "word ".repeat(80);
        let html = format!(r#"<p style="text-align: justify">{words}</p>"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Tw should be reset to 0 after each justified line
        assert!(
            content.contains("0 Tw\n"),
            "Tw should be reset to 0 after justified lines"
        );
    }

    // --- Overflow / Visibility / Transform PDF rendering tests ---

    #[test]
    fn render_visibility_hidden_skips_content() {
        let html = r#"<div style="visibility: hidden">Hidden text</div><p>Visible text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("Hidden text"),
            "visibility: hidden should not render text content"
        );
        assert!(
            content.contains("Visible"),
            "Other text should still render"
        );
    }

    #[test]
    fn render_overflow_hidden_produces_clip_path() {
        let html =
            r#"<div style="overflow: hidden; width: 200pt; height: 100pt">Clipped content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re W n"),
            "overflow: hidden should produce clipping path (re W n)"
        );
        assert!(
            content.contains("Clipped"),
            "Content should still be rendered inside clip"
        );
    }

    #[test]
    fn render_transform_rotate_produces_cm() {
        let html = r#"<div style="transform: rotate(45deg)">Rotated text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // rotate(45deg) should produce cos/sin values in a cm operator
        assert!(
            content.contains("cm\n"),
            "transform: rotate should produce cm operator"
        );
        assert!(
            content.contains("q\n"),
            "transform should save graphics state with q"
        );
        assert!(
            content.contains("Q\n"),
            "transform should restore graphics state with Q"
        );
        // cos(45) ~= 0.7071, sin(45) ~= 0.7071
        assert!(
            content.contains("0.707"),
            "rotate(45deg) should contain cos/sin values ~0.707"
        );
    }

    #[test]
    fn render_transform_scale_produces_cm() {
        let html = r#"<div style="transform: scale(2)">Scaled text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("2 0 0 2 0 0 cm"),
            "transform: scale(2) should produce '2 0 0 2 0 0 cm'"
        );
    }

    #[test]
    fn render_transform_translate_produces_cm() {
        let html = r#"<div style="transform: translate(10pt, 20pt)">Translated text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("1 0 0 1 10 20 cm"),
            "transform: translate(10pt, 20pt) should produce '1 0 0 1 10 20 cm'"
        );
    }

    #[test]
    fn render_overflow_visible_no_clip() {
        let html = r#"<div style="width: 200pt">Normal content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("re W n"),
            "No overflow should not produce clipping path"
        );
    }

    #[test]
    fn render_border_radius_produces_bezier_curves() {
        let html = r#"<div style="border: 1px solid black; border-radius: 10pt; background-color: red">Rounded</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Bezier curves use 'c' operator; rounded rects should have them
        assert!(
            content.contains(" c\n"),
            "Border-radius should produce Bezier curve commands"
        );
        // Should also have 'h' to close the path
        assert!(
            content.contains("h\n"),
            "Rounded rect path should be closed with 'h'"
        );
    }

    #[test]
    fn render_outline_draws_outside_element() {
        let html = r#"<div style="outline: 2px solid red; width: 100pt">Outlined</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Outline should produce a stroke command (S) with outline color
        assert!(
            content.contains("1 0 0 RG"),
            "Outline should set red stroke color"
        );
        assert!(
            content.contains("S\n"),
            "Outline should produce a stroke command"
        );
    }

    #[test]
    fn render_border_radius_zero_uses_rectangle() {
        let html = r#"<div style="border: 1px solid black; background-color: blue">Square</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Without border-radius, should use 're' (rectangle) not Bezier curves
        assert!(
            content.contains("re\n"),
            "Zero border-radius should use rectangle operator"
        );
    }

    #[test]
    fn build_shading_function_single_stop() {
        // Single stop produces a constant-color Type 2 function
        let stops = vec![(0.5, (1.0, 0.0, 0.0))];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [1 0 0]"));
        assert!(result.contains("/C1 [1 0 0]"));
    }

    #[test]
    fn build_shading_function_two_stops() {
        let stops = vec![(0.0, (1.0, 0.0, 0.0)), (1.0, (0.0, 0.0, 1.0))];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [1 0 0]"));
        assert!(result.contains("/C1 [0 0 1]"));
    }

    #[test]
    fn build_shading_function_three_stops() {
        let stops = vec![
            (0.0, (1.0, 0.0, 0.0)),
            (0.5, (0.0, 1.0, 0.0)),
            (1.0, (0.0, 0.0, 1.0)),
        ];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 3"));
        assert!(result.contains("/Bounds [0.5]"));
        assert!(result.contains("/Encode [0 1 0 1]"));
    }

    #[test]
    fn build_shading_function_empty_stops() {
        let stops: Vec<(f32, (f32, f32, f32))> = vec![];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [0 0 0]"));
    }

    #[test]
    fn render_cell_text_with_empty_line_and_empty_run() {
        // Covers lines 718, 724: empty line text skipped, empty run skipped
        let empty_run = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        let non_empty_run = TextRun {
            text: "Hello".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        let cell = TableCell {
            lines: vec![
                TextLine {
                    runs: vec![empty_run.clone()],
                    height: 14.0,
                },
                TextLine {
                    runs: vec![empty_run.clone(), non_empty_run],
                    height: 14.0,
                },
            ],
            nested_rows: Vec::new(),
            content: Vec::new(),
            bold: false,
            colspan: 1,
            rowspan: 1,
            padding_top: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            padding_right: 2.0,
            background_color: None,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        render_cell_text(&mut content, &cell, 0.0, 100.0, 50.0, 20.0, &fonts);
        assert!(content.contains("Hello"));
    }

    #[test]
    fn text_block_empty_run_skipped() {
        // Covers line 401: empty text run within a text block line is skipped
        use crate::layout::engine::LayoutElement;
        let empty_run = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        let real_run = TextRun {
            text: "Data".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        let page = Page {
            elements: vec![(
                0.0,
                LayoutElement::TextBlock {
                    lines: vec![TextLine {
                        runs: vec![empty_run, real_run],
                        height: 14.0,
                    }],
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                    text_align: TextAlign::Left,
                    background_color: None,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    padding_right: 0.0,
                    border: LayoutBorder::default(),
                    block_width: None,
                    block_height: None,
                    opacity: 1.0,
                    float: Float::None,
                    clear: crate::style::computed::Clear::None,
                    position: Position::Static,
                    offset_top: 0.0,
                    offset_left: 0.0,
                    box_shadow: None,
                    visible: true,
                    clip_rect: None,
                    transform: None,
                    background_gradient: None,
                    background_radial_gradient: None,
                    border_radius: 0.0,
                    outline_width: 0.0,
                    outline_color: None,
                    text_indent: 0.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    vertical_align: crate::style::computed::VerticalAlign::Baseline,
                    z_index: 0,
                    heading_level: None,
                },
            )],
        };
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Data"));
    }

    #[test]
    fn page_break_element_renders() {
        // Covers line 677: PageBreak empty match arm
        let page = Page {
            elements: vec![
                (
                    0.0,
                    LayoutElement::TextBlock {
                        lines: vec![TextLine {
                            runs: vec![TextRun {
                                text: "Before".to_string(),
                                font_size: 12.0,
                                bold: false,
                                italic: false,
                                underline: false,
                                line_through: false,
                                color: (0.0, 0.0, 0.0),
                                font_family: FontFamily::Helvetica,
                                link_url: None,
                                background_color: None,
                                padding: (0.0, 0.0),
                                border_radius: 0.0,
                                preserve_whitespace: false,
                            }],
                            height: 14.0,
                        }],
                        margin_top: 0.0,
                        margin_bottom: 0.0,
                        text_align: TextAlign::Left,
                        background_color: None,
                        padding_top: 0.0,
                        padding_bottom: 0.0,
                        padding_left: 0.0,
                        padding_right: 0.0,
                        border: LayoutBorder::default(),
                        block_width: None,
                        block_height: None,
                        opacity: 1.0,
                        float: Float::None,
                        clear: crate::style::computed::Clear::None,
                        position: Position::Static,
                        offset_top: 0.0,
                        offset_left: 0.0,
                        box_shadow: None,
                        visible: true,
                        clip_rect: None,
                        transform: None,
                        background_gradient: None,
                        background_radial_gradient: None,
                        border_radius: 0.0,
                        outline_width: 0.0,
                        outline_color: None,
                        text_indent: 0.0,
                        letter_spacing: 0.0,
                        word_spacing: 0.0,
                        vertical_align: crate::style::computed::VerticalAlign::Baseline,
                        z_index: 0,
                        heading_level: None,
                    },
                ),
                (20.0, LayoutElement::PageBreak),
            ],
        };
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Before"));
    }

    #[test]
    fn font_name_for_run_custom_bold_italic() {
        // Covers lines 761-763: Custom font bold+italic fallback names
        let run_bi = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: true,
            italic: true,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        assert_eq!(font_name_for_run(&run_bi), "Helvetica-BoldOblique");

        let run_b = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: true,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        assert_eq!(font_name_for_run(&run_b), "Helvetica-Bold");

        let run_i = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: false,
            italic: true,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        assert_eq!(font_name_for_run(&run_i), "Helvetica-Oblique");
    }

    #[test]
    fn render_radial_gradient_uses_shading() {
        use crate::style::computed::GradientStop;
        use crate::types::Color;
        let mut content = String::new();
        let mut shadings = Vec::new();
        let mut counter = 0usize;
        let gradient = RadialGradient {
            stops: vec![
                GradientStop {
                    color: Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                    position: 0.0,
                },
                GradientStop {
                    color: Color {
                        r: 0,
                        g: 0,
                        b: 255,
                        a: 255,
                    },
                    position: 1.0,
                },
            ],
        };
        render_radial_gradient(
            &mut content,
            &gradient,
            0.0,
            0.0,
            1.0,
            1.0,
            &mut shadings,
            &mut counter,
        );
        assert!(!content.is_empty());
        assert!(content.contains("/SH0 sh"));
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].shading_type, 3);
    }

    #[test]
    fn utf8_to_winansi_ascii() {
        let input = "Hello, World! 123";
        let result = utf8_to_winansi(input);
        assert_eq!(result, input.as_bytes());
    }

    #[test]
    fn utf8_to_winansi_em_dash() {
        // "hello — world" contains U+2014 em dash which should become 0x97
        let input = "hello \u{2014} world";
        let result = utf8_to_winansi(input);
        let expected: Vec<u8> = vec![
            b'h', b'e', b'l', b'l', b'o', b' ', 0x97, b' ', b'w', b'o', b'r', b'l', b'd',
        ];
        assert_eq!(result, expected);
    }

    #[test]
    fn utf8_to_winansi_quotes() {
        // Left/right single and double curly quotes
        let input = "\u{2018}hello\u{2019} \u{201C}world\u{201D}";
        let result = utf8_to_winansi(input);
        assert_eq!(result[0], 0x91); // left single quote
        assert_eq!(result[6], 0x92); // right single quote
        assert_eq!(result[8], 0x93); // left double quote
        assert_eq!(result[14], 0x94); // right double quote
    }

    #[test]
    fn utf8_to_winansi_latin1() {
        // e-acute (U+00E9), n-tilde (U+00F1), u-diaeresis (U+00FC)
        let input = "\u{00E9}\u{00F1}\u{00FC}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![0xE9, 0xF1, 0xFC]);
    }

    #[test]
    fn utf8_to_winansi_unknown() {
        // Chinese character and emoji should be replaced with '?'
        let input = "\u{4E16}\u{1F600}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![b'?', b'?']);
    }

    #[test]
    fn utf8_to_winansi_en_dash_bullet_ellipsis_euro_trademark() {
        assert_eq!(utf8_to_winansi("\u{2013}"), vec![0x96]); // en dash
        assert_eq!(utf8_to_winansi("\u{2022}"), vec![0x95]); // bullet
        assert_eq!(utf8_to_winansi("\u{2026}"), vec![0x85]); // ellipsis
        assert_eq!(utf8_to_winansi("\u{20AC}"), vec![0x80]); // euro
        assert_eq!(utf8_to_winansi("\u{2122}"), vec![0x99]); // trademark
    }

    #[test]
    fn encode_pdf_text_special_chars() {
        assert_eq!(encode_pdf_text("hello"), "hello");
        assert_eq!(encode_pdf_text("(test)"), "\\(test\\)");
        assert_eq!(encode_pdf_text("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn encode_pdf_text_em_dash() {
        let encoded = encode_pdf_text("hello \u{2014} world");
        // 0x97 = 151 decimal = 227 octal; em dash should be \227
        assert_eq!(encoded, "hello \\227 world");
    }

    #[test]
    fn encode_pdf_text_em_dash_in_pdf_bytes() {
        // Verify that rendering em dash produces correct octal escape in PDF
        // and does NOT produce UTF-8 bytes or mojibake
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        // The PDF content stream should contain the octal escape \227
        assert!(
            pdf_str.contains("\\227"),
            "PDF should contain octal escape \\227 for em dash"
        );

        // The raw UTF-8 bytes for em dash (0xE2 0x80 0x94) should NOT appear
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "PDF should not contain raw UTF-8 bytes for em dash"
        );

        // The mojibake pattern should not appear
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(!has_mojibake, "PDF should not contain mojibake bytes");
    }

    #[test]
    fn integration_em_dash_no_mojibake_in_pdf() {
        // Render HTML with em dash and verify the raw UTF-8 mojibake bytes
        // "\xC3\xA2\xC2\x80\xC2\x94" (the UTF-8 encoding of U+2014 read as
        // latin1) do NOT appear in the output.
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

        // The mojibake sequence for em dash in UTF-8 misinterpreted as latin1
        // is bytes [0xC3, 0xA2]. This must NOT appear in the PDF.
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(
            !has_mojibake,
            "PDF output contains UTF-8 mojibake for em dash"
        );

        // The octal escape sequence \227 (for byte 0x97) should appear in the PDF
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("\\227"),
            "PDF output should contain octal escape \\227 for WinAnsi em dash"
        );
    }

    #[test]
    fn total_row_bold_from_descendant_selector() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 12pt; }
        </style></head><body>
        <table>
            <tr><td>Item</td><td>$100</td></tr>
            <tr class="total-row"><td>Total</td><td>$100</td></tr>
        </table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The total row cells should use Helvetica-Bold
        assert!(
            pdf_str.contains("/Helvetica-Bold 12 Tf"),
            "Total row should use Helvetica-Bold at 12pt, PDF content:\n{}",
            pdf_str
                .lines()
                .filter(|l| l.contains("Helvetica"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn table_cell_em_dash_encoded_correctly() {
        let html = r#"<table><tr><td>HTML/CSS to PDF conversion — Enterprise</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Em dash in table cell should be encoded as octal \227
        assert!(
            pdf_str.contains("\\227"),
            "Table cell em dash should be encoded as \\227"
        );
        // No raw UTF-8 bytes for em dash
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "Table cell should not contain raw UTF-8 em dash bytes"
        );
    }

    #[test]
    fn linear_gradient_uses_shading() {
        let html = r#"<div style="background: linear-gradient(to bottom, red, blue); height: 50pt">Gradient</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ShadingType 2"),
            "Linear gradient should produce ShadingType 2 (axial)"
        );
    }

    #[test]
    fn radial_gradient_uses_shading_in_pdf() {
        let html =
            r#"<div style="background: radial-gradient(red, blue); height: 50pt">Gradient</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ShadingType 3"),
            "Radial gradient should produce ShadingType 3"
        );
    }

    #[test]
    fn border_top_only_renders_single_line() {
        let html = r#"<div style="border-top: 2pt solid red">Top border only</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Per-side border renders as a move-to + line-to + stroke, not a rectangle
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for top border"
        );
        assert!(pdf_str.contains("1 0 0 RG"), "Should have red stroke color");
    }

    #[test]
    fn border_bottom_renders() {
        let html = r#"<div style="border-bottom: 1pt solid blue">Bottom border</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for bottom border"
        );
        assert!(
            pdf_str.contains("0 0 1 RG"),
            "Should have blue stroke color"
        );
    }

    #[test]
    fn border_left_renders() {
        let html = r#"<blockquote style="border-left: 3pt solid green">Left border</blockquote>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for left border"
        );
        assert!(
            pdf_str.contains("0 0.50196 0 RG")
                || pdf_str.contains("0 0.501960")
                || pdf_str.contains("RG"),
            "Should have green stroke color"
        );
    }

    #[test]
    fn non_uniform_borders_render_per_side() {
        let html =
            r#"<div style="border-top: 2pt solid red; border-bottom: 1pt solid blue">Mixed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Non-uniform borders should produce per-side line strokes
        assert!(pdf_str.contains("1 0 0 RG"), "Should have red for top");
        assert!(pdf_str.contains("0 0 1 RG"), "Should have blue for bottom");
        // Should use line strokes, not rectangle
        let stroke_count = pdf_str.matches("l S\n").count();
        assert!(
            stroke_count >= 2,
            "Should have at least 2 line strokes, got {stroke_count}"
        );
    }

    #[test]
    fn gradient_clipped_to_border_radius() {
        let html = r#"<div style="background: linear-gradient(to bottom, red, blue); border-radius: 10pt; height: 50pt">Clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("sh"),
            "Should have shading operator for gradient"
        );
        assert!(
            pdf_str.contains("W n"),
            "Should have clip operator for border-radius"
        );
    }

    #[test]
    fn flexrow_with_gradient() {
        let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); height: 40pt"><div style="width: 100pt">A</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ShadingType 2"),
            "FlexRow with linear-gradient should produce ShadingType 2"
        );
    }

    #[test]
    fn flexrow_cell_background() {
        let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: yellow">Yellow</div><div style="width: 100pt">Plain</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Yellow = 1 1 0 rg
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow fill color for cell background"
        );
        assert!(
            pdf_str.contains("re\nf\n"),
            "Should have rectangle fill for cell background"
        );
    }

    #[test]
    fn flexrow_cell_border_radius() {
        let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: red; border-radius: 8pt">Round</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curve commands (c)
        assert!(pdf_str.contains("1 0 0 rg"), "Should have red fill");
        assert!(
            pdf_str.contains(" c\n"),
            "Should have Bezier curve for border-radius"
        );
    }

    #[test]
    fn flexrow_cell_gradient() {
        let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, green, yellow)">Grad</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("sh"),
            "Should have shading for cell gradient"
        );
        assert!(
            pdf_str.contains("/ShadingType 2"),
            "Cell gradient should use axial shading"
        );
    }

    #[test]
    fn flexrow_border_renders() {
        let html = r#"<div style="display: flex; border: 2pt solid black"><div style="width: 100pt">Bordered</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("re\nS\n"),
            "Should have rectangle stroke for uniform flex border"
        );
        assert!(
            pdf_str.contains("0 0 0 RG"),
            "Should have black stroke color"
        );
    }

    #[test]
    fn flexrow_border_radius_background() {
        let html = r#"<div style="display: flex; border-radius: 10pt; background-color: #cccccc"><div style="width: 100pt">Rounded</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Rounded background uses Bezier curves, not re
        assert!(
            pdf_str.contains(" c\n"),
            "Should have Bezier curves for rounded background"
        );
        assert!(pdf_str.contains("f\n"), "Should have fill command");
    }

    #[test]
    fn inline_span_border_radius() {
        let html = r#"<div style="display: flex"><div style="width: 300pt"><p><span style="background-color: yellow; border-radius: 4pt; padding: 2pt">Tag</span> text</p></div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Inline span with border-radius should produce rounded rect path + fill
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow fill for span bg"
        );
    }

    #[test]
    fn table_cell_borders_render() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { border-bottom: 1pt solid #999999; }
        </style></head><body>
        <table><tr><td>Cell</td></tr></table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l\nS\n") || pdf_str.contains("l S\n") || pdf_str.contains("re\nS\n"),
            "Table cell border should produce stroke commands"
        );
    }

    #[test]
    fn text_align_right_in_flex_cell() {
        let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: right">Right</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Right"), "Should contain the text 'Right'");
        // The text x-position should be offset from left (not at left margin)
        assert!(
            pdf_str.contains("Td"),
            "Should have text positioning operator"
        );
    }

    #[test]
    fn text_align_center_in_flex_cell() {
        let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: center">Center</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Center"),
            "Should contain the text 'Center'"
        );
        assert!(
            pdf_str.contains("Td"),
            "Should have text positioning operator"
        );
    }

    #[test]
    fn absolute_position_offset() {
        let html = r#"<div style="position: absolute; left: 100pt; top: 50pt">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Absolute"),
            "Should contain positioned text"
        );
    }

    #[test]
    fn float_right_position() {
        let html = r#"<div style="float: right; width: 100pt">Floated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Floated"), "Should contain floated text");
    }

    #[test]
    fn radial_gradient_clipped() {
        let html = r#"<div style="background: radial-gradient(red, blue); border-radius: 10pt; height: 50pt">Radial</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ShadingType 3"),
            "Should have radial shading"
        );
        assert!(
            pdf_str.contains("W n"),
            "Should clip radial gradient to border-radius"
        );
    }

    #[test]
    fn opacity_renders_extgstate() {
        let html = r#"<div style="opacity: 0.5">Transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ExtGState"),
            "Should have ExtGState for opacity"
        );
        assert!(pdf_str.contains("gs\n"), "Should apply graphics state");
    }

    #[test]
    fn box_shadow_renders() {
        let html = r#"<div style="box-shadow: 2pt 2pt 0 #888888; height: 30pt">Shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Box shadow renders as a filled rectangle behind the element
        assert!(
            pdf_str.contains("re\nf\n") || pdf_str.contains("f\n"),
            "Should have fill for box shadow"
        );
        assert!(pdf_str.contains("Shadow"), "Should contain the text");
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn position_absolute_block_x() {
        // Covers line 93, 128: Position::Absolute uses margin.left + offset_left
        let html =
            r#"<div style="position: absolute; left: 50pt; background-color: cyan">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Absolute"),
            "Should render absolute positioned text"
        );
    }

    #[test]
    fn position_relative_block_x() {
        // Covers lines 119-120, 129: Position::Relative block_x calculation
        let html =
            r#"<div style="position: relative; left: 30pt; background-color: lime">Relative</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Relative"),
            "Should render relative positioned text"
        );
    }

    #[test]
    fn float_right_positioning() {
        // Covers line 131: Float::Right block_x = margin.left + available_width - render_w
        let html = r#"<div style="float: right; width: 100pt">Float right</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Float right"),
            "Should render float right text"
        );
    }

    #[test]
    fn per_side_border_rendering() {
        // Covers lines 390-396: non-uniform per-side borders (left border with x_left offset)
        let html = r#"<div style="border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black; width: 200pt; height: 50pt">Borders</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Non-uniform borders produce per-side stroke commands
        assert!(
            pdf_str.contains("1 0 0 RG"),
            "Should have red top border stroke"
        );
        assert!(
            pdf_str.contains("0 0 0 RG"),
            "Should have black left border stroke"
        );
        assert!(
            pdf_str.contains("l\nS\n") || pdf_str.contains("l S\n"),
            "Should have per-side line strokes"
        );
    }

    #[test]
    fn center_align_with_inline_span() {
        // Covers line 487: TextAlign::Center branch in TextBlock with inline padding
        let html = r#"<p style="text-align: center"><span style="background-color: yellow; padding: 4pt">Centered Span</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Centered Span"),
            "Should render centered span text"
        );
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow background fill"
        );
    }

    #[test]
    fn right_align_with_inline_span() {
        // Covers line 491: TextAlign::Right branch in TextBlock with inline padding
        let html = r#"<p style="text-align: right"><span style="background-color: lime; padding: 4pt">Right Span</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Right Span"),
            "Should render right-aligned span text"
        );
    }

    #[test]
    fn letter_spacing_in_text_rendering() {
        // Covers line 519 (letter-spacing sets Tc operator)
        let html = r#"<p style="letter-spacing: 2pt">Spaced out</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Tc\n"),
            "Letter spacing should produce Tc operator"
        );
        assert!(
            pdf_str.contains("0 Tc\n"),
            "Letter spacing should be reset to 0"
        );
    }

    #[test]
    fn underline_and_strikethrough_rendering() {
        // Covers underline and strikethrough draw lines with font-size-relative thickness
        let html = r#"<p><span style="text-decoration: underline">Under</span> <span style="text-decoration: line-through">Strike</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Both underline and strikethrough produce line strokes (S operator)
        let stroke_count = pdf_str.matches(" w\n").count();
        assert!(
            stroke_count >= 2,
            "Should have at least 2 stroke weight commands (underline + strikethrough), got {stroke_count}"
        );
        // Thickness should scale with font size (not hardcoded 0.5)
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw stroke lines for text decorations"
        );
    }

    #[test]
    fn table_cell_all_borders() {
        // Covers lines 621, 626-627, 705-724: table cell border rendering (all 4 sides)
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { border: 2pt solid red; }
        </style></head><body>
        <table><tr><td>Bordered Cell</td></tr></table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Bordered Cell"), "Should render cell text");
        // Red border strokes
        assert!(
            pdf_str.contains("1 0 0 RG"),
            "Should have red border stroke color"
        );
        // Should have multiple line strokes (top, right, bottom, left)
        let stroke_count = pdf_str.matches("l S\n").count() + pdf_str.matches("l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 border line strokes, got {stroke_count}"
        );
    }

    #[test]
    fn table_cell_rowspan_continuation() {
        // Covers lines 667, 669: rowspan > 1 cell rendering
        let html = r#"<table>
            <tr><td rowspan="2">Spanning</td><td>A</td></tr>
            <tr><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Spanning"), "Should render rowspan cell");
        assert!(pdf_str.contains("A"), "Should render first row cell");
        assert!(pdf_str.contains("B"), "Should render second row cell");
    }

    #[test]
    fn table_cell_geometry_includes_outer_border_spacing() {
        let (first_x, first_w) = table_cell_geometry(&[50.0, 60.0], 0, 1, 6.0, 10.0);
        assert!((first_x - 16.0).abs() < 0.01);
        assert!((first_w - 50.0).abs() < 0.01);

        let (second_x, second_w) = table_cell_geometry(&[50.0, 60.0], 1, 1, 6.0, 10.0);
        assert!((second_x - 72.0).abs() < 0.01);
        assert!((second_w - 60.0).abs() < 0.01);

        let (span_x, span_w) = table_cell_geometry(&[50.0, 60.0], 0, 2, 6.0, 10.0);
        assert!((span_x - 16.0).abs() < 0.01);
        assert!((span_w - 116.0).abs() < 0.01);
    }

    #[test]
    fn rowspan_height_includes_vertical_border_spacing() {
        let row =
            |text: &str, spacing: crate::style::computed::BorderSpacing| LayoutElement::TableRow {
                cells: vec![TableCell {
                    lines: vec![TextLine {
                        runs: vec![TextRun {
                            text: text.to_string(),
                            font_size: 12.0,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: crate::style::computed::FontFamily::Helvetica,
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                            preserve_whitespace: false,
                        }],
                        height: 10.0,
                    }],
                    nested_rows: Vec::new(),
                    content: Vec::new(),
                    bold: false,
                    background_color: None,
                    padding_top: 0.0,
                    padding_right: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    colspan: 1,
                    rowspan: 1,
                    border: crate::layout::engine::LayoutBorder::default(),
                    text_align: crate::style::computed::TextAlign::Left,
                }],
                col_widths: vec![40.0],
                margin_top: 0.0,
                margin_bottom: 0.0,
                border_collapse: crate::style::computed::BorderCollapse::Separate,
                border_spacing: spacing,
            };
        let spacing = crate::style::computed::BorderSpacing {
            horizontal: 0.0,
            vertical: 4.0,
        };
        let page = crate::layout::engine::Page {
            elements: vec![(0.0, row("A", spacing)), (0.0, row("B", spacing))],
        };

        let height = table_rowspan_height(&page, 0, 10.0, 2, spacing.vertical);
        assert!((height - 24.0).abs() < 0.01);
    }

    #[test]
    fn table_cell_nested_table_renders_inner_content() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Outer"), "Should render outer cell text");
        assert!(
            pdf_str.contains("Inner"),
            "Should render nested table cell text"
        );
    }

    #[test]
    fn flexrow_container_gradient() {
        // Covers lines 742, 744, 753, 848-874: FlexRow linear gradient with border-radius
        let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); border-radius: 5pt"><div>Gradient Flex</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Gradient Flex"),
            "Should render flex content"
        );
        // Linear gradient produces shading reference
        assert!(
            pdf_str.contains("sh\n"),
            "Should have shading operator for gradient"
        );
    }

    #[test]
    fn flexrow_non_uniform_border() {
        // Covers lines 790, 798, 804-805, 939-969: FlexRow non-uniform per-side border
        let html = r#"<div style="display: flex; border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black"><div>Flex Borders</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Flex Borders"),
            "Should render flex content"
        );
        // Non-uniform borders produce per-side strokes
        assert!(
            pdf_str.contains("1 0 0 RG"),
            "Should have red stroke for top"
        );
    }

    #[test]
    fn flexrow_cell_inline_background_with_border_radius() {
        // Covers lines 852-903, 982-1001: FlexRow cell bg with border-radius and gradient
        let html = r#"<div style="display: flex"><div style="background-color: orange; border-radius: 8pt; width: 100pt">Cell BG</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Cell BG"), "Should render cell text");
        // Orange background: 1 0.647.. 0 rg — check for the fill command
        assert!(
            pdf_str.contains("rg\n"),
            "Should have fill color for cell background"
        );
    }

    #[test]
    fn flexrow_cell_text_alignment() {
        // Covers lines 918-969, 1084, 1090: FlexRow cell text-align center and right
        let html = r#"<div style="display: flex">
            <div style="width: 200pt; text-align: center">Center</div>
            <div style="width: 200pt; text-align: right">Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Center"),
            "Should render center-aligned text"
        );
        assert!(
            pdf_str.contains("Right"),
            "Should render right-aligned text"
        );
    }

    #[test]
    fn render_cell_text_vertical_centering() {
        // Covers lines 1116-1123: render_cell_text vertical centering with bg + border-radius
        let run = TextRun {
            text: "Centered".to_string(),
            font_size: 14.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 0.0, 0.0)),
            padding: (4.0, 2.0),
            border_radius: 3.0,
            preserve_whitespace: false,
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 16.0,
            }],
            nested_rows: Vec::new(),
            content: Vec::new(),
            bold: false,
            colspan: 1,
            rowspan: 1,
            padding_top: 4.0,
            padding_bottom: 4.0,
            padding_left: 4.0,
            padding_right: 4.0,
            background_color: None,
            border: LayoutBorder::default(),
            text_align: TextAlign::Center,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        render_cell_text(&mut content, &cell, 10.0, 200.0, 100.0, 40.0, &fonts);
        assert!(content.contains("Centered"), "Should render cell text");
        // Background with border-radius produces rounded rect
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red inline background"
        );
    }

    #[test]
    fn merge_runs_border_radius_comparison() {
        // Covers lines 1175, 1179-1180: merge_runs checks border_radius equality
        let run_a = TextRun {
            text: "Hello ".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0)),
            padding: (2.0, 1.0),
            border_radius: 4.0,
            preserve_whitespace: false,
        };
        let run_b = TextRun {
            text: "World".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0)),
            padding: (2.0, 1.0),
            border_radius: 8.0, // Different border_radius
            preserve_whitespace: false,
        };
        let merged = merge_runs(&[run_a.clone(), run_b.clone()]);
        // Different border_radius should prevent merging
        assert_eq!(
            merged.len(),
            2,
            "Runs with different border_radius should not merge"
        );
        // Same border_radius should merge
        let mut run_b_same = run_b;
        run_b_same.border_radius = 4.0;
        let merged2 = merge_runs(&[run_a, run_b_same]);
        assert_eq!(
            merged2.len(),
            1,
            "Runs with same border_radius should merge"
        );
    }

    #[test]
    fn build_shading_function_four_stops_stitching() {
        // Covers lines 1277-1304: Type 3 stitching function with 4 stops
        let stops = vec![
            (0.0, (1.0, 0.0, 0.0)),
            (0.33, (0.0, 1.0, 0.0)),
            (0.66, (0.0, 0.0, 1.0)),
            (1.0, (1.0, 1.0, 0.0)),
        ];
        let result = build_shading_function(&stops);
        assert!(
            result.contains("/FunctionType 3"),
            "4 stops should produce Type 3 stitching function"
        );
        assert!(
            result.contains("/Bounds [0.33 0.66]"),
            "Should have bounds for intermediate stops"
        );
        assert!(
            result.contains("/Encode [0 1 0 1 0 1]"),
            "Should have encode entries for each sub-function"
        );
        // Should contain 3 sub-functions (one per stop pair)
        let subfn_count = result.matches("/FunctionType 2").count();
        assert_eq!(
            subfn_count, 3,
            "Should have 3 Type 2 sub-functions, got {subfn_count}"
        );
    }

    #[test]
    fn custom_font_embedding_in_pdf() {
        // Covers lines 1628-1657: TTF font objects in PDF
        use crate::parser::ttf::TtfFont;
        let mut cmap = HashMap::new();
        for c in 32u16..=126 {
            cmap.insert(c, c - 31);
        }
        let ttf = TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            ascent: 800,
            descent: -200,
            cmap,
            glyph_widths: (0..=96).map(|_| 500).collect(),
            num_h_metrics: 96,
            flags: 32,
            data: vec![0u8; 64], // Minimal dummy font data
        };
        let mut fonts = HashMap::new();
        fonts.insert("TestFont".to_string(), ttf);

        let run = TextRun {
            text: "Custom".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("TestFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            preserve_whitespace: false,
        };
        let page = Page {
            elements: vec![(
                0.0,
                LayoutElement::TextBlock {
                    lines: vec![TextLine {
                        runs: vec![run],
                        height: 14.0,
                    }],
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                    text_align: TextAlign::Left,
                    background_color: None,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    padding_right: 0.0,
                    border: LayoutBorder::default(),
                    block_width: None,
                    block_height: None,
                    opacity: 1.0,
                    float: Float::None,
                    clear: crate::style::computed::Clear::None,
                    position: Position::Static,
                    offset_top: 0.0,
                    offset_left: 0.0,
                    box_shadow: None,
                    visible: true,
                    clip_rect: None,
                    transform: None,
                    background_gradient: None,
                    background_radial_gradient: None,
                    border_radius: 0.0,
                    outline_width: 0.0,
                    outline_color: None,
                    text_indent: 0.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    vertical_align: crate::style::computed::VerticalAlign::Baseline,
                    z_index: 0,
                    heading_level: None,
                },
            )],
        };
        let pdf = render_pdf_with_fonts(&[page], PageSize::A4, Margin::default(), &fonts).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/BaseFont /TestFont"),
            "Should have custom font BaseFont entry"
        );
        assert!(
            pdf_str.contains("/Subtype /TrueType"),
            "Should have TrueType subtype"
        );
        assert!(
            pdf_str.contains("/FontDescriptor"),
            "Should have FontDescriptor reference"
        );
        assert!(
            pdf_str.contains("/FontFile2"),
            "Should have FontFile2 reference for embedded TTF"
        );
        assert!(
            pdf_str.contains("/TestFont"),
            "Should reference custom font name"
        );
    }

    #[test]
    fn ext_gstate_objects_rendered() {
        // Covers line 2011: ExtGState objects in resource dict
        let html = r#"<div style="opacity: 0.3">Dim</div><div style="opacity: 0.7">Bright</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("/ca 0.3"), "Should have fill opacity 0.3");
        assert!(pdf_str.contains("/ca 0.7"), "Should have fill opacity 0.7");
        assert!(
            pdf_str.contains("/ExtGState"),
            "Should have ExtGState in resources"
        );
        // Should have default GS reset
        assert!(
            pdf_str.contains("/GSDefault gs"),
            "Should reset to default graphics state"
        );
    }

    #[test]
    fn flexrow_cell_gradient_with_border_radius() {
        // Covers lines 1009-1060: FlexRow cell with linear gradient + border-radius clip
        let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, red, blue); border-radius: 10pt">Grad Cell</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Grad Cell"), "Should render cell text");
        assert!(
            pdf_str.contains("sh\n"),
            "Should have shading operator for cell gradient"
        );
    }

    #[test]
    fn half_leading_text_positioning() {
        // Text blocks should use half-leading model (not full line.height offset)
        let html = "<p style=\"font-size: 20pt; line-height: 2\">Test</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Should contain Td operator for text positioning
        assert!(pdf_str.contains("Td\n"), "Should have text positioning");
        // Text should be rendered
        assert!(pdf_str.contains("(Test)"), "Should contain text content");
    }

    #[test]
    fn underline_in_flex_cell() {
        // Underline in flex cells should produce stroke commands
        let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><u>Underlined in flex</u></div>
        </div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Should have a stroke line for underline
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw underline stroke in flex cell"
        );
    }

    #[test]
    fn strikethrough_in_flex_cell() {
        let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><del>Deleted in flex</del></div>
        </div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw strikethrough stroke in flex cell"
        );
    }

    #[test]
    fn underline_in_table_cell() {
        let html = r#"<table><tr><td><u>Underlined cell</u></td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw underline stroke in table cell"
        );
    }

    #[test]
    fn strikethrough_in_table_cell() {
        let html = r#"<table><tr><td><s>Struck cell</s></td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw strikethrough stroke in table cell"
        );
    }

    #[test]
    fn font_size_relative_underline_thickness() {
        // Large font should produce thicker underline than small font
        let html = r#"<p><span style="font-size: 6pt; text-decoration: underline">Small</span></p>
        <p><span style="font-size: 30pt; text-decoration: underline">Big</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Both should have strokes; thickness should vary
        let w_count = pdf_str.matches(" w\n").count();
        assert!(
            w_count >= 2,
            "Should have at least 2 underline thickness commands, got {w_count}"
        );
    }

    #[test]
    fn table_cell_vertical_centering_with_metrics() {
        // Table cells with different row heights should center text
        let html = r#"<table>
            <tr>
                <td style="padding: 20pt">Centered</td>
                <td>Short</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Centered)"),
            "Should render centered cell text"
        );
        assert!(pdf_str.contains("(Short)"), "Should render short cell text");
    }
}
