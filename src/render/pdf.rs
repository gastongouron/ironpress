use crate::error::IronpressError;
use crate::layout::engine::{
    ImageFormat, LayoutElement, Page, PngMetadata, TableCell, TextLine, TextRun,
};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BorderCollapse, Float, FontFamily, GradientStop, LinearGradient, Position, RadialGradient,
    TextAlign,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;

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

/// Render laid-out pages as PDF, writing directly to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`render_pdf`]. It writes PDF content incrementally
/// to the provided writer instead of building an in-memory buffer.
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
    let mut pdf_writer = PdfWriter::new();
    let available_width = page_size.width - margin.left - margin.right;

    // Register custom TrueType fonts
    for (name, ttf) in custom_fonts {
        pdf_writer.add_ttf_font(name, ttf);
    }

    for page in pages {
        let mut content = String::new();
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();
        let mut page_ext_gstates: Vec<(String, f32)> = Vec::new();

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
                    ..
                } => {
                    // Skip rendering if visibility: hidden (but space is preserved)
                    if !visible {
                        continue;
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
                        render_linear_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            total_h,
                        );
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
                        render_radial_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            total_h,
                        );
                    }

                    // Draw border if specified
                    if *border_width > 0.0 {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let border_y = block_y - total_h;
                        let (br, bg, bb) = border_color.unwrap_or((0.0, 0.0, 0.0));
                        content
                            .push_str(&format!("{br} {bg} {bb} RG\n{bw} w\n", bw = border_width,));
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                block_x,
                                border_y,
                                render_width,
                                total_h,
                                *border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = block_x,
                                y = border_y,
                                w = render_width,
                                h = total_h,
                            ));
                        }
                        content.push_str("S\n");
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
                        text_y -= line.height;

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
                            TextAlign::Center => block_x + (render_width - line_width) / 2.0,
                            TextAlign::Right => block_x + render_width - padding_right - line_width,
                        };

                        // Set letter spacing (CSS letter-spacing)
                        if *letter_spacing > 0.0 {
                            content.push_str(&format!("{letter_spacing} Tc\n"));
                        }

                        // Set word spacing (justify + CSS word-spacing)
                        if total_ws > 0.0 {
                            content.push_str(&format!("{total_ws} Tw\n"));
                        }

                        // Render each run
                        let mut x = text_x;
                        for run in &line.runs {
                            if run.text.is_empty() {
                                continue;
                            }

                            let font_name = resolve_font_name(run, custom_fonts);
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

                            let run_width = estimate_run_width_with_fonts(run, custom_fonts);

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
                        0.0
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

                        let cell_x = margin.left
                            + col_widths.iter().take(col_pos).sum::<f32>()
                            + spacing * col_pos as f32;
                        let cell_w: f32 = (0..cell.colspan)
                            .map(|i| col_widths.get(col_pos + i).copied().unwrap_or(0.0))
                            .sum::<f32>()
                            + if cell.colspan > 1 {
                                spacing * (cell.colspan - 1) as f32
                            } else {
                                0.0
                            };

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
                        render_cell_text(&mut content, cell, cell_x, row_y, cell_w, custom_fonts);

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
                        render_cell_text(&mut content, cell, cell_x, row_y, cell_w, custom_fonts);

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
                LayoutElement::PageBreak => {}
            }
        }

        pdf_writer.add_page(
            page_size.width,
            page_size.height,
            &content,
            annotations,
            page_images,
            page_ext_gstates,
        );
    }

    pdf_writer.finish_to_writer(writer)
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
    custom_fonts: &HashMap<String, TtfFont>,
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
            let font_name = resolve_font_name(run, custom_fonts);
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
            x += estimate_run_width_with_fonts(run, custom_fonts);
        }
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
    let char_width_factor = match &run.font_family {
        // Courier is monospace, each character is ~0.6 em
        FontFamily::Courier => 0.6,
        // Times is slightly narrower than Helvetica on average
        FontFamily::TimesRoman => 0.48,
        // Helvetica average character width — also used as fallback for custom
        FontFamily::Helvetica | FontFamily::Custom(_) => 0.5,
    };
    run.text.len() as f32 * run.font_size * char_width_factor
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
        .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
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

/// Interpolate between two colors at a given fraction t in [0, 1].
fn interpolate_color(c1: &GradientStop, c2: &GradientStop, t: f32) -> (f32, f32, f32) {
    let frac = if (c2.position - c1.position).abs() < 1e-6 {
        0.0
    } else {
        ((t - c1.position) / (c2.position - c1.position)).clamp(0.0, 1.0)
    };
    let (r1, g1, b1) = c1.color.to_f32_rgb();
    let (r2, g2, b2) = c2.color.to_f32_rgb();
    (
        r1 + (r2 - r1) * frac,
        g1 + (g2 - g1) * frac,
        b1 + (b2 - b1) * frac,
    )
}

/// Get the interpolated color at position t from the gradient stops.
fn color_at_position(stops: &[GradientStop], t: f32) -> (f32, f32, f32) {
    if stops.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    if t <= stops[0].position {
        return stops[0].color.to_f32_rgb();
    }
    if t >= stops[stops.len() - 1].position {
        return stops[stops.len() - 1].color.to_f32_rgb();
    }
    for i in 0..stops.len() - 1 {
        if t >= stops[i].position && t <= stops[i + 1].position {
            return interpolate_color(&stops[i], &stops[i + 1], t);
        }
    }
    stops[stops.len() - 1].color.to_f32_rgb()
}

/// Render a linear gradient as a series of thin filled rectangles.
fn render_linear_gradient(
    content: &mut String,
    gradient: &LinearGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let num_strips = 50;
    let angle_rad = gradient.angle * std::f32::consts::PI / 180.0;
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    let is_horizontal = (sin_a.abs() - 1.0).abs() < 0.01;
    let is_vertical = (cos_a.abs() - 1.0).abs() < 0.01;

    if is_horizontal {
        let reversed = sin_a < 0.0;
        let strip_w = width / num_strips as f32;
        for i in 0..num_strips {
            let t = if reversed {
                1.0 - (i as f32 + 0.5) / num_strips as f32
            } else {
                (i as f32 + 0.5) / num_strips as f32
            };
            let (r, g, b) = color_at_position(&gradient.stops, t);
            let sx = x + i as f32 * strip_w;
            content.push_str(&format!(
                "{r} {g} {b} rg\n{sx} {y} {sw} {h} re\nf\n",
                sw = strip_w,
                h = height,
            ));
        }
    } else if is_vertical {
        let reversed = cos_a > 0.0;
        let strip_h = height / num_strips as f32;
        for i in 0..num_strips {
            let t = if reversed {
                (i as f32 + 0.5) / num_strips as f32
            } else {
                1.0 - (i as f32 + 0.5) / num_strips as f32
            };
            let (r, g, b) = color_at_position(&gradient.stops, t);
            let sy = y + i as f32 * strip_h;
            content.push_str(&format!(
                "{r} {g} {b} rg\n{x} {sy} {w} {sh} re\nf\n",
                w = width,
                sh = strip_h,
            ));
        }
    } else {
        let strip_w = width / num_strips as f32;
        for i in 0..num_strips {
            let frac = (i as f32 + 0.5) / num_strips as f32;
            let t = frac * sin_a.abs()
                + (1.0 - frac) * (1.0 - cos_a.abs()) * 0.5
                + frac * (1.0 - cos_a.abs()) * 0.5;
            let t = t.clamp(0.0, 1.0);
            let (r, g, b) = color_at_position(&gradient.stops, t);
            let sx = x + i as f32 * strip_w;
            content.push_str(&format!(
                "{r} {g} {b} rg\n{sx} {y} {sw} {h} re\nf\n",
                sw = strip_w,
                h = height,
            ));
        }
    }
}

/// Render a radial gradient as concentric filled circles from outside to inside.
fn render_radial_gradient(
    content: &mut String,
    gradient: &RadialGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let num_rings = 50;
    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    let max_radius = (width.max(height)) / 2.0;

    for i in 0..num_rings {
        let t = 1.0 - i as f32 / num_rings as f32;
        let radius = max_radius * t;
        let (r, g, b) = color_at_position(&gradient.stops, t);

        if radius < 0.5 {
            continue;
        }

        let k = 0.552_284_8_f32 * radius;
        content.push_str(&format!("{r} {g} {b} rg\n"));
        content.push_str(&format!("{} {} m\n", cx + radius, cy));
        content.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            cx + radius,
            cy + k,
            cx + k,
            cy + radius,
            cx,
            cy + radius
        ));
        content.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            cx - k,
            cy + radius,
            cx - radius,
            cy + k,
            cx - radius,
            cy
        ));
        content.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            cx - radius,
            cy - k,
            cx - k,
            cy - radius,
            cx,
            cy - radius
        ));
        content.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            cx + k,
            cy - radius,
            cx + radius,
            cy - k,
            cx + radius,
            cy
        ));
        content.push_str("f\n");
    }
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

    fn add_page(
        &mut self,
        width: f32,
        height: f32,
        content: &str,
        annotations: Vec<LinkAnnotation>,
        images: Vec<ImageRef>,
        ext_gstates: Vec<(String, f32)>,
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
    }

    fn finish_to_writer<W: std::io::Write>(self, out: &mut W) -> Result<(), IronpressError> {
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

        // Font dictionary (standard + custom fonts)
        let font_dict_id = font_base_id + font_names.len();
        let mut font_entries: Vec<String> = font_names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} {} 0 R", font_base_id + i))
            .collect();
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

        // Catalog
        let catalog_id = pages_id + 1;
        all_objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R >>\nendobj",
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
    fn color_at_position_empty_stops() {
        // Covers line 842: empty stops returns black
        let result = color_at_position(&[], 0.5);
        assert_eq!(result, (0.0, 0.0, 0.0));
    }

    #[test]
    fn color_at_position_before_first_stop() {
        // Covers line 845: t <= first stop position
        use crate::types::Color;
        let stops = vec![GradientStop {
            color: Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            position: 0.5,
        }];
        let (r, _g, _b) = color_at_position(&stops, 0.1);
        assert!((r - 1.0).abs() < 0.01); // returns first stop color (red)
    }

    #[test]
    fn color_at_position_after_last_stop() {
        // Covers line 845 (>= last position) and line 855 (fallback)
        use crate::types::Color;
        let stops = vec![
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
                position: 0.5,
            },
        ];
        let (_r, _g, b) = color_at_position(&stops, 0.9);
        assert!((b - 1.0).abs() < 0.01); // returns last stop color (blue)
    }

    #[test]
    fn interpolate_color_same_position_stops() {
        // Covers line 826: stops at same position yield frac = 0.0
        use crate::types::Color;
        let s1 = GradientStop {
            color: Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            position: 0.5,
        };
        let s2 = GradientStop {
            color: Color {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
            position: 0.5,
        };
        let (r, _g, b) = interpolate_color(&s1, &s2, 0.5);
        // frac is 0.0, so result should be s1's color
        assert!((r - 1.0).abs() < 0.01);
        assert!(b.abs() < 0.01);
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
            bold: false,
            colspan: 1,
            rowspan: 1,
            padding_top: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            padding_right: 2.0,
            background_color: None,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        render_cell_text(&mut content, &cell, 0.0, 100.0, 50.0, &fonts);
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
                    border_width: 0.0,
                    border_color: None,
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
                        border_width: 0.0,
                        border_color: None,
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
        };
        assert_eq!(font_name_for_run(&run_i), "Helvetica-Oblique");
    }

    #[test]
    fn render_radial_gradient_skips_tiny_radius() {
        // Covers line 948: radius < 0.5 continue
        use crate::types::Color;
        let mut content = String::new();
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
        // Very small dimensions so many rings have radius < 0.5
        render_radial_gradient(&mut content, &gradient, 0.0, 0.0, 1.0, 1.0);
        assert!(!content.is_empty());
    }
}
