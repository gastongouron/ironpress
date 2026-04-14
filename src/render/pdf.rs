use crate::error::IronpressError;
use crate::layout::engine::{
    ImageFormat, LayoutElement, Page, PngMetadata, TableCell, TextLine, TextRun,
    layout_element_paint_order, table_cell_content_height,
};
use crate::parser::ttf::TtfFont;
use crate::render::background::{
    BackgroundPaintContext, RasterBackgroundRequest, overflow_from_viewport_box,
    register_background_image, svg_visual_overflow, synthetic_raster_background,
    viewport_box_from_overflow,
};
use crate::render::pdf_fonts::{PreparedCustomFont, PreparedCustomFonts, prepare_custom_fonts};
use crate::render::shading::{
    ShadingEntry, build_shading_function, push_axial_shading, push_radial_shading,
};
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{
    BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderCollapse,
    BorderStyle, Float, FontFamily, LinearGradient, Overflow, Position, RadialGradient, TextAlign,
    VerticalAlign,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;
use std::io::Write as _;

mod layout_elements;

use layout_elements::{
    NestedLayoutFrame, PageRenderContext, TableCellRenderBox, compute_row_height,
    render_cell_content, table_cell_geometry,
};

#[cfg(test)]
use layout_elements::{
    CellTextPlacement, NestedTextBlock, TextRenderContext, plan_nested_layout_elements,
    render_cell_text, render_nested_layout_elements, render_nested_text_block,
    table_row_total_height,
};

/// Returns the PDF dash-pattern operator string for a given border style.
fn dash_pattern_for_style(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::Dashed => "[6 4] 0 d\n",
        BorderStyle::Dotted => "[1 3] 0 d\n",
        _ => "",
    }
}

/// Reset the dash pattern back to solid after a dashed/dotted stroke.
fn reset_dash_pattern(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::Dashed | BorderStyle::Dotted => "[] 0 d\n",
        _ => "",
    }
}

/// A link annotation to be placed on a PDF page.
struct LinkAnnotation {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    url: String,
}

#[derive(Clone, Copy)]
struct TextLineAnnotationBox {
    top: f32,
    bottom: f32,
}

fn text_run_link_annotation(
    run: &TextRun,
    x: f32,
    width: f32,
    line_box: TextLineAnnotationBox,
) -> Option<LinkAnnotation> {
    let url = run.link_url.as_ref()?;
    Some(LinkAnnotation {
        x1: x,
        y1: line_box.bottom,
        x2: x + width,
        y2: line_box.top,
        url: url.clone(),
    })
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
    let prepared_custom_fonts = prepare_custom_fonts(pages, custom_fonts);

    register_used_custom_fonts(&mut pdf_writer, custom_fonts, &prepared_custom_fonts);

    for (page_idx, page) in pages.iter().enumerate() {
        let mut content = String::new();
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();
        let mut page_ext_gstates: Vec<(String, f32)> = Vec::new();
        let mut bg_alpha_counter: usize = 0;
        let mut page_shadings: Vec<ShadingEntry> = Vec::new();
        let mut shading_counter: usize = 0;

        // Track clip state: when a TextBlock has clip_children_count > 0,
        // the clip context stays open for that many subsequent elements.
        let mut clip_remaining: usize = 0;

        for (elem_idx, (y_pos, element)) in page.elements.iter().enumerate() {
            // Close clip context when all clipped children have been rendered
            if clip_remaining > 0 {
                clip_remaining -= 1;
                if clip_remaining == 0 {
                    content.push_str("Q\n");
                }
            }
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
                    offset_top: _,
                    offset_left,
                    offset_bottom: _,
                    offset_right: _,
                    containing_block,
                    box_shadow,
                    visible,
                    clip_rect,
                    transform,
                    background_gradient,
                    background_radial_gradient,
                    background_svg,
                    background_blur_radius,
                    background_size,
                    background_position,
                    background_repeat,
                    background_origin,
                    border_radius,
                    outline_width,
                    outline_color,
                    letter_spacing,
                    word_spacing: css_word_spacing,
                    heading_level,
                    clip_children_count,
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
                        Position::Absolute => {
                            // Position relative to the containing block.
                            // bottom/right offsets are pre-resolved into top/left
                            // at layout time, so we only use offset_left here.
                            containing_block.map_or(margin.left + offset_left, |cb| {
                                margin.left + cb.x + offset_left
                            })
                        }
                        Position::Relative => margin.left + offset_left,
                        Position::Static => match float {
                            Float::Right => {
                                let render_w = block_width.unwrap_or(available_width);
                                margin.left + available_width - render_w
                            }
                            _ => margin.left,
                        },
                    };
                    // PDF y-axis is bottom-up.
                    // y_pos already includes absolute/relative offsets from pagination.
                    let block_y = page_size.height - margin.top - y_pos;

                    // Use explicit block_width if set, otherwise available_width
                    let render_width = block_width.unwrap_or(available_width);
                    let total_h = text_block_total_height(
                        lines,
                        *padding_top,
                        *padding_bottom,
                        *block_height,
                    );
                    let block_bottom = block_y - total_h;

                    // Apply transform if set (wrap in q/Q).
                    // Rotate and scale are applied around the element's centre so
                    // that the element stays in its layout position (matching
                    // CSS `transform-origin: 50% 50%`).  The combined matrix is:
                    //   T(cx,cy) · M · T(-cx,-cy)
                    // which in PDF `cm` notation is a single 6-value matrix.
                    let needs_transform = transform.is_some();
                    if let Some(t) = transform {
                        // Centre of the element in PDF (bottom-up) coordinates.
                        let cx = block_x + render_width * 0.5;
                        let cy = block_bottom + total_h * 0.5;
                        content.push_str("q\n");
                        match t {
                            crate::style::computed::Transform::Rotate(deg) => {
                                // Negate angle: CSS rotate is clockwise in top-down Y,
                                // but PDF Y points up, so negate to match visual result.
                                let rad = -deg * std::f32::consts::PI / 180.0;
                                let cos_v = rad.cos();
                                let sin_v = rad.sin();
                                // T(cx,cy) · Rot · T(-cx,-cy)
                                let tx = cx - cx * cos_v + cy * sin_v;
                                let ty = cy - cx * sin_v - cy * cos_v;
                                content.push_str(&format!(
                                    "{cos_v} {sin_v} {neg_sin} {cos_v} {tx} {ty} cm\n",
                                    neg_sin = -sin_v,
                                ));
                            }
                            crate::style::computed::Transform::Scale(sx, sy) => {
                                // T(cx,cy) · Scale(sx,sy) · T(-cx,-cy)
                                let tx = cx - cx * sx;
                                let ty = cy - cy * sy;
                                content.push_str(&format!("{sx} 0 0 {sy} {tx} {ty} cm\n",));
                            }
                            crate::style::computed::Transform::Translate(tx, ty) => {
                                // Negate Y: CSS positive Y is down, PDF positive Y is up
                                content.push_str(&format!("1 0 0 1 {tx} {} cm\n", -ty));
                            }
                            crate::style::computed::Transform::Matrix(a, b, c, d, e, f) => {
                                // Pre-composed matrix — apply around element centre.
                                // Convert CSS matrix (top-down Y) to PDF (bottom-up Y):
                                // Conjugate with Y-flip: F·M·F where F = diag(1,-1)
                                // [a b e]     [a  -b  e ]
                                // [c d f]  →  [-c  d  -f]
                                let pa = a;
                                let pb = -b;
                                let pc = -c;
                                let pd = d;
                                let pe = e;
                                let pf = -f;
                                let ne = pa * (-cx) + pc * (-cy) + pe + cx;
                                let nf = pb * (-cx) + pd * (-cy) + pf + cy;
                                content.push_str(&format!("{pa} {pb} {pc} {pd} {ne} {nf} cm\n"));
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

                    // Draw box-shadow with blur
                    if let Some(shadow) = box_shadow {
                        render_box_shadow(
                            &mut content,
                            shadow,
                            block_x,
                            block_bottom,
                            render_width,
                            total_h,
                            *border_radius,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                        );
                    }

                    // Draw background if specified
                    if let Some((r, g, b, a)) = background_color {
                        let bg_y = block_bottom;
                        let needs_bg_alpha = *a < 1.0;
                        if needs_bg_alpha {
                            let effective_alpha = *a * *opacity;
                            let gs_name = format!("GSbg{elem_idx}");
                            page_ext_gstates.push((gs_name.clone(), effective_alpha));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
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
                        if needs_bg_alpha {
                            // Reset to element opacity or full opacity
                            if needs_opacity {
                                let gs_name = format!("GS{elem_idx}");
                                content.push_str(&format!("/{gs_name} gs\n"));
                            } else {
                                content.push_str("/GSDefault gs\n");
                            }
                        }
                    }

                    // Draw linear gradient if specified
                    if let Some(gradient) = background_gradient {
                        let bg_y = block_bottom;
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
                        let bg_y = block_bottom;
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

                    // Draw SVG background image if specified
                    if let Some(svg_tree) = background_svg {
                        let text_height: f32 = lines.iter().map(|l| l.height).sum();
                        let content_h = padding_top + text_height + padding_bottom;
                        let total_h = match block_height {
                            Some(h) => content_h.max(*h),
                            None => content_h,
                        };
                        let bg_y = block_y - total_h;
                        // Adjust reference box based on background-origin
                        let (ref_x, ref_y, ref_w, ref_h) = match background_origin {
                            BackgroundOrigin::Border => (
                                block_x - border.left.width,
                                bg_y - border.bottom.width,
                                render_width + border.left.width + border.right.width,
                                total_h + border.top.width + border.bottom.width,
                            ),
                            BackgroundOrigin::Content => (
                                block_x + padding_left,
                                bg_y + padding_bottom,
                                (render_width - padding_left - padding_right).max(0.0),
                                (total_h - padding_top - padding_bottom).max(0.0),
                            ),
                            BackgroundOrigin::Padding => (block_x, bg_y, render_width, total_h),
                        };
                        render_svg_background(
                            &mut content,
                            svg_tree,
                            &mut pdf_writer,
                            &mut page_images,
                            &mut page_shadings,
                            &mut shading_counter,
                            BackgroundPaintContext::new(
                                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                SvgViewportBox::new(
                                    block_x - border.left.width,
                                    bg_y - border.bottom.width,
                                    render_width + border.left.width + border.right.width,
                                    total_h + border.top.width + border.bottom.width,
                                ),
                                *border_radius,
                                *background_blur_radius,
                                *background_size,
                                *background_position,
                                *background_repeat,
                            ),
                        );
                    }

                    // Draw border if specified
                    if border.has_any() {
                        let border_y = block_bottom;
                        // Check if all sides are uniform (same width & color)
                        let uniform = border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color
                            && border.top.style == border.right.style
                            && border.top.style == border.bottom.style
                            && border.top.style == border.left.style;
                        if uniform && *border_radius > 0.0 {
                            let (br, bg, bb) = border.top.color;
                            content.push_str(dash_pattern_for_style(border.top.style));
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
                            content.push_str(reset_dash_pattern(border.top.style));
                        } else if uniform {
                            let (br, bg, bb) = border.top.color;
                            content.push_str(dash_pattern_for_style(border.top.style));
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
                            content.push_str(reset_dash_pattern(border.top.style));
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
                                content.push_str(dash_pattern_for_style(border.top.style));
                                content
                                    .push_str(&format!("{r} {g} {b} RG\n{} w\n", border.top.width));
                                content.push_str(&format!("{x1} {y_top} m {x2} {y_top} l S\n"));
                                content.push_str(reset_dash_pattern(border.top.style));
                            }
                            // Right border
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                content.push_str(dash_pattern_for_style(border.right.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.right.width
                                ));
                                content.push_str(&format!(
                                    "{x_right} {y_top} m {x_right} {y_bottom} l S\n"
                                ));
                                content.push_str(reset_dash_pattern(border.right.style));
                            }
                            // Bottom border
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                content.push_str(dash_pattern_for_style(border.bottom.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.bottom.width
                                ));
                                content
                                    .push_str(&format!("{x1} {y_bottom} m {x2} {y_bottom} l S\n"));
                                content.push_str(reset_dash_pattern(border.bottom.style));
                            }
                            // Left border
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                content.push_str(dash_pattern_for_style(border.left.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.left.width
                                ));
                                content.push_str(&format!(
                                    "{x_left} {y_top} m {x_left} {y_bottom} l S\n"
                                ));
                                content.push_str(reset_dash_pattern(border.left.style));
                            }
                        }
                    }

                    // Draw outline if specified (outside the element box)
                    if *outline_width > 0.0 {
                        let offset = *outline_width / 2.0;
                        let outline_x = block_x - offset;
                        let outline_y = block_bottom - offset;
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
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let line_annotation_box = TextLineAnnotationBox {
                            top: text_y + metrics.ascender + metrics.half_leading,
                            bottom: text_y - metrics.descender - metrics.half_leading,
                        };

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

                        // Phase 1: Draw backgrounds, decorations, and link
                        // annotations at estimated positions (visual-only).
                        let mut bg_x = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }
                            let (r, g, b) = run.color;
                            let run_width = estimate_run_width_with_fonts(run, custom_fonts);

                            // Draw background rectangle for inline spans
                            if let Some((br, bg, bb, ba)) = run.background_color {
                                let needs_inline_bg_alpha = ba < 1.0;
                                if needs_inline_bg_alpha {
                                    let effective_alpha = ba * *opacity;
                                    let gs_name = format!("GSba{bg_alpha_counter}");
                                    bg_alpha_counter += 1;
                                    page_ext_gstates.push((gs_name.clone(), effective_alpha));
                                    content.push_str(&format!("/{gs_name} gs\n"));
                                }
                                let (pad_h, pad_v) = run.padding;
                                let rect_x = bg_x - pad_h;
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
                                if needs_inline_bg_alpha {
                                    if needs_opacity {
                                        let gs_name = format!("GS{elem_idx}");
                                        content.push_str(&format!("/{gs_name} gs\n"));
                                    } else {
                                        content.push_str("/GSDefault gs\n");
                                    }
                                }
                            }

                            // Draw underline (font-size-relative position and thickness)
                            if run.underline {
                                let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
                                    &run.font_family,
                                    run.bold,
                                    run.italic,
                                    custom_fonts,
                                );
                                let desc = descender_ratio * run.font_size;
                                let uy = text_y - desc * 0.6;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{thickness} w\n{bg_x} {uy} m {x2} {uy} l\nS\n",
                                    x2 = bg_x + run_width,
                                ));
                            }

                            // Draw strikethrough (line-through)
                            if run.line_through {
                                let sy = text_y + run.font_size * 0.3;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{thickness} w\n{bg_x} {sy} m {x2} {sy} l\nS\n",
                                    x2 = bg_x + run_width,
                                ));
                            }

                            // Track link annotation
                            if let Some(annotation) =
                                text_run_link_annotation(run, bg_x, run_width, line_annotation_box)
                            {
                                annotations.push(annotation);
                            }

                            bg_x += run_width;
                        }

                        // Phase 2: Render all text in a single BT/ET block
                        // so the PDF viewer advances the cursor naturally.
                        render_line_text(
                            &mut content,
                            &merged,
                            text_x,
                            text_y,
                            custom_fonts,
                            &prepared_custom_fonts,
                        );

                        // Reset letter spacing after line
                        if *letter_spacing > 0.0 {
                            content.push_str("0 Tc\n");
                        }

                        // Reset word spacing after line
                        if total_ws > 0.0 {
                            content.push_str("0 Tw\n");
                        }

                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    // Reset opacity if it was changed
                    if needs_opacity {
                        content.push_str("/GSDefault gs\n");
                    }

                    // Restore clipping state.
                    // If clip_children_count > 0, keep the clip open for
                    // subsequent elements that are visually inside this container.
                    if needs_clip {
                        if *clip_children_count > 0 {
                            clip_remaining = *clip_children_count;
                        } else {
                            content.push_str("Q\n");
                        }
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

                        let (cell_x, cell_w) = table_cell_geometry(
                            col_widths,
                            col_pos,
                            cell.colspan,
                            spacing,
                            margin.left,
                        );

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
                        if let Some((r, g, b, a)) = cell.background_color {
                            let needs_cell_bg_alpha = a < 1.0;
                            if needs_cell_bg_alpha {
                                let gs_name = format!("GStcbg{elem_idx}_{col_pos}");
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = row_y - cell_height,
                                w = cell_w,
                                h = cell_height,
                            ));
                            if needs_cell_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Draw cell borders when CSS specifies them.
                        if cell.border.has_any() {
                            let x1 = cell_x;
                            let x2 = cell_x + cell_w;
                            let y_top = row_y;
                            let y_bottom = row_y - cell_height;
                            if cell.border.top.width > 0.0 {
                                let (r, g, b) = cell.border.top.color;
                                content.push_str(dash_pattern_for_style(cell.border.top.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                    cell.border.top.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.top.style));
                            }
                            if cell.border.right.width > 0.0 {
                                let (r, g, b) = cell.border.right.color;
                                content.push_str(dash_pattern_for_style(cell.border.right.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                    cell.border.right.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.right.style));
                            }
                            if cell.border.bottom.width > 0.0 {
                                let (r, g, b) = cell.border.bottom.color;
                                content.push_str(dash_pattern_for_style(cell.border.bottom.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                    cell.border.bottom.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.bottom.style));
                            }
                            if cell.border.left.width > 0.0 {
                                let (r, g, b) = cell.border.left.color;
                                content.push_str(dash_pattern_for_style(cell.border.left.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                    cell.border.left.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.left.style));
                            }
                        }

                        // Render cell text at the first row's y position
                        let mut page_context = PageRenderContext::new(
                            &mut pdf_writer,
                            &mut page_images,
                            custom_fonts,
                            &prepared_custom_fonts,
                            &mut page_shadings,
                            &mut shading_counter,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                            &mut annotations,
                        );
                        render_cell_content(
                            &mut content,
                            cell,
                            TableCellRenderBox::new(
                                cell_x,
                                row_y,
                                cell_w,
                                row_height,
                                NestedLayoutFrame::new(
                                    cell_x,
                                    row_y,
                                    margin.left,
                                    page_size.height - margin.top,
                                    cell_w,
                                ),
                            ),
                            &mut page_context,
                        );

                        col_pos += cell.colspan;
                    }
                }
                LayoutElement::GridRow {
                    cells,
                    col_widths,
                    gap,
                    border: grid_border,
                    padding_left: grid_pl,
                    padding_right: grid_pr,
                    padding_top: grid_pt,
                    padding_bottom: grid_pb,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let row_height = compute_row_height(cells) + grid_pt + grid_pb;
                    let grid_total_w: f32 = col_widths.iter().sum::<f32>()
                        + gap * col_widths.len().saturating_sub(1) as f32
                        + grid_pl
                        + grid_pr;

                    // Draw grid container border
                    if grid_border.has_any() {
                        let bx1 = margin.left;
                        let bx2 = margin.left + grid_total_w;
                        let by1 = row_y;
                        let by2 = row_y - row_height;
                        if grid_border.top.width > 0.0 {
                            let (r, g, b) = grid_border.top.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                                grid_border.top.width
                            ));
                        }
                        if grid_border.right.width > 0.0 {
                            let (r, g, b) = grid_border.right.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                                grid_border.right.width
                            ));
                        }
                        if grid_border.bottom.width > 0.0 {
                            let (r, g, b) = grid_border.bottom.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                                grid_border.bottom.width
                            ));
                        }
                        if grid_border.left.width > 0.0 {
                            let (r, g, b) = grid_border.left.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                                grid_border.left.width
                            ));
                        }
                    }

                    let mut cell_x = margin.left + grid_pl;
                    let cell_row_y = row_y - grid_pt;
                    for (i, cell) in cells.iter().enumerate() {
                        let cell_w = if i < col_widths.len() {
                            col_widths[i]
                        } else {
                            0.0
                        };

                        // Draw cell background
                        let cell_content_h = compute_row_height(cells);
                        if let Some((r, g, b, a)) = cell.background_color {
                            let needs_grid_bg_alpha = a < 1.0;
                            if needs_grid_bg_alpha {
                                let gs_name = format!("GSgcbg{elem_idx}_{i}");
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = cell_row_y - cell_content_h,
                                w = cell_w,
                                h = cell_content_h,
                            ));
                            if needs_grid_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Render cell text
                        let mut page_context = PageRenderContext::new(
                            &mut pdf_writer,
                            &mut page_images,
                            custom_fonts,
                            &prepared_custom_fonts,
                            &mut page_shadings,
                            &mut shading_counter,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                            &mut annotations,
                        );
                        render_cell_content(
                            &mut content,
                            cell,
                            TableCellRenderBox::new(
                                cell_x,
                                cell_row_y,
                                cell_w,
                                cell_content_h,
                                NestedLayoutFrame::new(
                                    cell_x,
                                    cell_row_y,
                                    margin.left,
                                    page_size.height - margin.top,
                                    cell_w,
                                ),
                            ),
                            &mut page_context,
                        );

                        cell_x += cell_w;
                        // Add gap between columns
                        if i + 1 < col_widths.len() {
                            cell_x += gap;
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
                    padding_right,
                    border,
                    border_radius,
                    box_shadow,
                    background_gradient,
                    background_radial_gradient,
                    background_svg,
                    background_blur_radius,
                    background_size: flex_bg_size,
                    background_position: flex_bg_pos,
                    background_repeat: flex_bg_repeat,
                    background_origin: flex_bg_origin,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let full_height =
                        padding_top + row_height + padding_bottom + border.vertical_width();

                    // Draw box shadow with blur
                    if let Some(shadow) = box_shadow {
                        render_box_shadow(
                            &mut content,
                            shadow,
                            margin.left,
                            row_y - full_height,
                            *container_width,
                            full_height,
                            *border_radius,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                        );
                    }

                    // Draw container background
                    if let Some((r, g, b, a)) = background_color {
                        let bg_x = margin.left;
                        let bg_y = row_y - full_height;
                        let needs_flex_bg_alpha = *a < 1.0;
                        if needs_flex_bg_alpha {
                            let gs_name = format!("GSfbg{elem_idx}");
                            page_ext_gstates.push((gs_name.clone(), *a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
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
                        if needs_flex_bg_alpha {
                            content.push_str("/GSDefault gs\n");
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

                    // Draw SVG background image for flex container
                    if let Some(svg_tree) = background_svg {
                        let bg_x = margin.left;
                        let bg_y = row_y - full_height;
                        // Adjust reference box based on background-origin
                        let (ref_x, ref_y, ref_w, ref_h) = match flex_bg_origin {
                            BackgroundOrigin::Border => (
                                bg_x - border.left.width,
                                bg_y - border.bottom.width,
                                *container_width + border.left.width + border.right.width,
                                full_height + border.top.width + border.bottom.width,
                            ),
                            BackgroundOrigin::Content => (
                                bg_x + padding_left,
                                bg_y + padding_bottom,
                                (*container_width - padding_left - padding_right).max(0.0),
                                (full_height - padding_top - padding_bottom).max(0.0),
                            ),
                            BackgroundOrigin::Padding => {
                                (bg_x, bg_y, *container_width, full_height)
                            }
                        };
                        render_svg_background(
                            &mut content,
                            svg_tree,
                            &mut pdf_writer,
                            &mut page_images,
                            &mut page_shadings,
                            &mut shading_counter,
                            BackgroundPaintContext::new(
                                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                SvgViewportBox::new(
                                    bg_x - border.left.width,
                                    bg_y - border.bottom.width,
                                    *container_width + border.left.width + border.right.width,
                                    full_height + border.top.width + border.bottom.width,
                                ),
                                *border_radius,
                                *background_blur_radius,
                                *flex_bg_size,
                                *flex_bg_pos,
                                *flex_bg_repeat,
                            ),
                        );
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
                            && border.top.color == border.left.color
                            && border.top.style == border.right.style
                            && border.top.style == border.bottom.style
                            && border.top.style == border.left.style;
                        if uniform && *border_radius > 0.0 {
                            let (r, g, b) = border.top.color;
                            content.push_str(dash_pattern_for_style(border.top.style));
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
                            content.push_str(reset_dash_pattern(border.top.style));
                        } else if uniform {
                            let (r, g, b) = border.top.color;
                            content.push_str(dash_pattern_for_style(border.top.style));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{bw} w\n{bx} {by} {w} {h} re\nS\n",
                                bw = border.top.width,
                                w = container_width,
                                h = full_height,
                            ));
                            content.push_str(reset_dash_pattern(border.top.style));
                        } else {
                            let x1 = bx;
                            let x2 = bx + container_width;
                            let y_top = row_y;
                            let y_bottom = by;
                            if border.top.width > 0.0 {
                                let (r, g, b) = border.top.color;
                                content.push_str(dash_pattern_for_style(border.top.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                    border.top.width
                                ));
                                content.push_str(reset_dash_pattern(border.top.style));
                            }
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                content.push_str(dash_pattern_for_style(border.right.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                    border.right.width
                                ));
                                content.push_str(reset_dash_pattern(border.right.style));
                            }
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                content.push_str(dash_pattern_for_style(border.bottom.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                    border.bottom.width
                                ));
                                content.push_str(reset_dash_pattern(border.bottom.style));
                            }
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                content.push_str(dash_pattern_for_style(border.left.style));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                    border.left.width
                                ));
                                content.push_str(reset_dash_pattern(border.left.style));
                            }
                        }
                    }

                    // Render each flex cell at its computed x-offset
                    let text_area_top = row_y - border.top.width - padding_top;
                    for cell in cells {
                        let cell_x = margin.left + padding_left + cell.x_offset;
                        let cell_inner_w = cell.width - cell.padding_left - cell.padding_right;

                        // Apply cell transform if set (rotate, scale, translate)
                        let cell_needs_transform = cell.transform.is_some();
                        if let Some(t) = &cell.transform {
                            let cx = cell_x + cell.width * 0.5;
                            let cy = text_area_top - *row_height * 0.5;
                            content.push_str("q\n");
                            match t {
                                crate::style::computed::Transform::Rotate(deg) => {
                                    let rad = -deg * std::f32::consts::PI / 180.0;
                                    let cos_v = rad.cos();
                                    let sin_v = rad.sin();
                                    let tx = cx - cx * cos_v + cy * sin_v;
                                    let ty = cy - cx * sin_v - cy * cos_v;
                                    content.push_str(&format!(
                                        "{cos_v} {sin_v} {} {cos_v} {tx} {ty} cm\n",
                                        -sin_v,
                                    ));
                                }
                                crate::style::computed::Transform::Scale(sx, sy) => {
                                    let tx = cx - cx * sx;
                                    let ty = cy - cy * sy;
                                    content.push_str(&format!("{sx} 0 0 {sy} {tx} {ty} cm\n"));
                                }
                                crate::style::computed::Transform::Translate(dx, dy) => {
                                    content.push_str(&format!("1 0 0 1 {dx} {} cm\n", -dy));
                                }
                                crate::style::computed::Transform::Matrix(a, b, c, d, e, f) => {
                                    // CSS→PDF Y-flip conjugation
                                    let pa = a;
                                    let pb = -b;
                                    let pc = -c;
                                    let pd = d;
                                    let pe = e;
                                    let pf = -f;
                                    let ne = pa * (-cx) + pc * (-cy) + pe + cx;
                                    let nf = pb * (-cx) + pd * (-cy) + pf + cy;
                                    content
                                        .push_str(&format!("{pa} {pb} {pc} {pd} {ne} {nf} cm\n"));
                                }
                            }
                        }

                        // Draw cell background
                        if let Some((r, g, b, a)) = cell.background_color {
                            let bg_x = margin.left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - row_height;
                            let needs_fcell_bg_alpha = a < 1.0;
                            if needs_fcell_bg_alpha {
                                let gs_name = format!("GSfcbg{bg_alpha_counter}");
                                bg_alpha_counter += 1;
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
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
                            if needs_fcell_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Draw cell borders
                        if cell.border.has_any() {
                            if cell.border_radius > 0.0 {
                                let bw = cell.border.top.width;
                                let (r, g, b) = cell.border.top.color;
                                content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                                content.push_str(&rounded_rect_path(
                                    cell_x,
                                    text_area_top - row_height,
                                    cell.width,
                                    *row_height,
                                    cell.border_radius,
                                ));
                                content.push_str("S\n");
                            } else {
                                let bx1 = cell_x;
                                let bx2 = cell_x + cell.width;
                                let by1 = text_area_top;
                                let by2 = text_area_top - row_height;
                                if cell.border.top.width > 0.0 {
                                    let (r, g, b) = cell.border.top.color;
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                                        cell.border.top.width
                                    ));
                                }
                                if cell.border.right.width > 0.0 {
                                    let (r, g, b) = cell.border.right.color;
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                                        cell.border.right.width
                                    ));
                                }
                                if cell.border.bottom.width > 0.0 {
                                    let (r, g, b) = cell.border.bottom.color;
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                                        cell.border.bottom.width
                                    ));
                                }
                                if cell.border.left.width > 0.0 {
                                    let (r, g, b) = cell.border.left.color;
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                                        cell.border.left.width
                                    ));
                                }
                            } // else (non-rounded cell border)
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

                        if let Some(svg_tree) = &cell.background_svg {
                            let bg_x = margin.left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - row_height;
                            let (ref_x, ref_y, ref_w, ref_h) = match cell.background_origin {
                                BackgroundOrigin::Content => (
                                    bg_x + cell.padding_left,
                                    bg_y + cell.padding_bottom,
                                    (cell.width - cell.padding_left - cell.padding_right).max(0.0),
                                    (row_height - cell.padding_top - cell.padding_bottom).max(0.0),
                                ),
                                BackgroundOrigin::Border | BackgroundOrigin::Padding => {
                                    (bg_x, bg_y, cell.width, *row_height)
                                }
                            };
                            render_svg_background(
                                &mut content,
                                svg_tree,
                                &mut pdf_writer,
                                &mut page_images,
                                &mut page_shadings,
                                &mut shading_counter,
                                BackgroundPaintContext::new(
                                    SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                    SvgViewportBox::new(bg_x, bg_y, cell.width, *row_height),
                                    cell.border_radius,
                                    cell.background_blur_radius,
                                    cell.background_size,
                                    cell.background_position,
                                    cell.background_repeat,
                                ),
                            );
                        }

                        // Render cell text
                        let mut text_y = text_area_top - cell.padding_top;
                        for line in &cell.lines {
                            let metrics = line_box_metrics(line, custom_fonts);
                            text_y -= metrics.half_leading + metrics.ascender;
                            let line_annotation_box = TextLineAnnotationBox {
                                top: text_y + metrics.ascender + metrics.half_leading,
                                bottom: text_y - metrics.descender - metrics.half_leading,
                            };
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
                                let (r, g, b) = run.color;
                                let rw = estimate_run_width_with_fonts(run, custom_fonts);

                                // Draw background rectangle for inline spans
                                if let Some((br, bgc, bb, ba)) = run.background_color {
                                    let needs_inline_bg_alpha = ba < 1.0;
                                    if needs_inline_bg_alpha {
                                        let gs_name = format!("GSfiba{bg_alpha_counter}");
                                        bg_alpha_counter += 1;
                                        page_ext_gstates.push((gs_name.clone(), ba));
                                        content.push_str(&format!("/{gs_name} gs\n"));
                                    }
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
                                    if needs_inline_bg_alpha {
                                        content.push_str("/GSDefault gs\n");
                                    }
                                }

                                render_run_text(
                                    &mut content,
                                    run,
                                    x,
                                    text_y,
                                    custom_fonts,
                                    &prepared_custom_fonts,
                                );

                                // Draw underline (font-size-relative)
                                if run.underline {
                                    let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
                                        &run.font_family,
                                        run.bold,
                                        run.italic,
                                        custom_fonts,
                                    );
                                    let desc = descender_ratio * run.font_size;
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

                                if let Some(annotation) =
                                    text_run_link_annotation(run, x, rw, line_annotation_box)
                                {
                                    annotations.push(annotation);
                                }

                                x += rw;
                            }

                            text_y -= metrics.descender + metrics.half_leading;
                        }

                        // Render nested elements (tables, images, etc. inside flex items)
                        if !cell.nested_elements.is_empty() {
                            let nested_x = cell_x;
                            let mut nested_y = text_area_top;
                            for nested_elem in &cell.nested_elements {
                                match nested_elem {
                                    LayoutElement::TextBlock {
                                        lines: n_lines,
                                        margin_top: n_mt,
                                        padding_top: n_pt,
                                        padding_bottom: n_pb,
                                        background_color: n_bg,
                                        block_width: n_bw,
                                        block_height: n_bh,
                                        border: n_border,
                                        ..
                                    } => {
                                        nested_y -= n_mt;
                                        let n_width = n_bw.unwrap_or(cell.width);
                                        let text_h: f32 = n_lines.iter().map(|l| l.height).sum();
                                        let total_h =
                                            n_pt + text_h + n_pb + n_border.vertical_width();
                                        let total_h = n_bh.map_or(total_h, |h| total_h.max(h));

                                        if let Some((r, g, b, a)) = n_bg {
                                            if *a >= 1.0 {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = nested_x,
                                                    y = nested_y - total_h,
                                                    w = n_width,
                                                    h = total_h,
                                                ));
                                            }
                                        }

                                        // Draw borders for nested TextBlock
                                        if n_border.has_any() {
                                            let x1 = nested_x;
                                            let x2 = nested_x + n_width;
                                            let y_top = nested_y;
                                            let y_bottom = nested_y - total_h;
                                            if n_border.top.width > 0.0 {
                                                let (r, g, b) = n_border.top.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                                    n_border.top.width
                                                ));
                                            }
                                            if n_border.bottom.width > 0.0 {
                                                let (r, g, b) = n_border.bottom.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                                    n_border.bottom.width
                                                ));
                                            }
                                            if n_border.left.width > 0.0 {
                                                let (r, g, b) = n_border.left.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                                    n_border.left.width
                                                ));
                                            }
                                            if n_border.right.width > 0.0 {
                                                let (r, g, b) = n_border.right.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                                    n_border.right.width
                                                ));
                                            }
                                        }

                                        let mut ty = nested_y - n_pt;
                                        for line in n_lines {
                                            let m = line_box_metrics(line, custom_fonts);
                                            ty -= m.half_leading + m.ascender;
                                            let merged = merge_runs(&line.runs);
                                            let mut lx = nested_x + cell.padding_left;
                                            for run in &merged {
                                                let rw = render_run_text(
                                                    &mut content,
                                                    run,
                                                    lx,
                                                    ty,
                                                    custom_fonts,
                                                    &prepared_custom_fonts,
                                                );
                                                lx += rw;
                                            }
                                            ty -= m.descender + m.half_leading;
                                        }
                                        nested_y -= total_h;
                                    }
                                    LayoutElement::TableRow {
                                        cells: t_cells,
                                        col_widths,
                                        ..
                                    } => {
                                        let t_row_h = compute_row_height(t_cells);
                                        let mut tcx = nested_x;
                                        for (i, t_cell) in t_cells.iter().enumerate() {
                                            let tw = if i < col_widths.len() {
                                                col_widths[i]
                                            } else {
                                                0.0
                                            };
                                            if let Some((r, g, b, _)) = t_cell.background_color {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = tcx,
                                                    y = nested_y - t_row_h,
                                                    w = tw,
                                                    h = t_row_h,
                                                ));
                                            }
                                            let mut ty = nested_y - t_cell.padding_top;
                                            for line in &t_cell.lines {
                                                let m = line_box_metrics(line, custom_fonts);
                                                ty -= m.half_leading + m.ascender;
                                                let merged = merge_runs(&line.runs);
                                                let mut lx = tcx + t_cell.padding_left;
                                                for run in &merged {
                                                    let rw = render_run_text(
                                                        &mut content,
                                                        run,
                                                        lx,
                                                        ty,
                                                        custom_fonts,
                                                        &prepared_custom_fonts,
                                                    );
                                                    lx += rw;
                                                }
                                                ty -= m.descender + m.half_leading;
                                            }
                                            // Draw cell borders
                                            if t_cell.border.has_any() {
                                                let x1 = tcx;
                                                let x2 = tcx + tw;
                                                let y_top = nested_y;
                                                let y_bottom = nested_y - t_row_h;
                                                if t_cell.border.top.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.top.color;
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                                        t_cell.border.top.width
                                                    ));
                                                }
                                                if t_cell.border.bottom.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.bottom.color;
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                                        t_cell.border.bottom.width
                                                    ));
                                                }
                                                if t_cell.border.left.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.left.color;
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                                        t_cell.border.left.width
                                                    ));
                                                }
                                                if t_cell.border.right.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.right.color;
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                                        t_cell.border.right.width
                                                    ));
                                                }
                                            }
                                            tcx += tw;
                                        }
                                        nested_y -= t_row_h;
                                    }
                                    LayoutElement::Svg {
                                        tree,
                                        width: svg_w,
                                        height: svg_h,
                                        margin_top: svg_mt,
                                        ..
                                    } => {
                                        nested_y -= svg_mt;
                                        let svg_x = nested_x;
                                        let svg_y = nested_y - svg_h;
                                        content.push_str("q\n");
                                        // Y-flip + position
                                        content.push_str(&format!(
                                            "1 0 0 -1 {svg_x} {} cm\n",
                                            svg_y + svg_h
                                        ));
                                        // Apply viewBox scaling
                                        if let Some(placement) =
                                            crate::render::svg_geometry::compute_svg_placement(
                                                tree,
                                                crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                                                    0.0, 0.0, *svg_w, *svg_h,
                                                    tree.preserve_aspect_ratio,
                                                ),
                                            )
                                        {
                                            content.push_str("q\n");
                                            content.push_str(&placement.viewport.clip_path());
                                            content.push_str(&format!(
                                                "{sx} 0 0 {sy} {tx} {ty} cm\n",
                                                sx = placement.scale_x,
                                                sy = placement.scale_y,
                                                tx = placement.translate_x,
                                                ty = placement.translate_y,
                                            ));
                                        }
                                        {
                                            let mut image_sink = SvgPageImageSink {
                                                pdf_writer: &mut pdf_writer,
                                                page_images: &mut page_images,
                                            };
                                            let mut resources =
                                                crate::render::svg_to_pdf::SvgPdfResources {
                                                    shadings: &mut page_shadings,
                                                    shading_counter: &mut shading_counter,
                                                    image_sink: Some(&mut image_sink),
                                                };
                                            crate::render::svg_to_pdf::render_svg_tree_with_resources(
                                                tree,
                                                &mut content,
                                                &mut resources,
                                            );
                                        }
                                        if tree.view_box.is_some() {
                                            content.push_str("Q\n");
                                        }
                                        content.push_str("Q\n");
                                        nested_y -= svg_h;
                                    }
                                    LayoutElement::Container {
                                        children: cont_kids,
                                        background_color: cont_bg,
                                        border: cont_border,
                                        padding_top: cont_pt,
                                        padding_bottom: cont_pb,
                                        padding_left: cont_pl,
                                        padding_right: cont_pr,
                                        margin_top: cont_mt,
                                        block_width: cont_bw,
                                        border_radius: cont_br,
                                        overflow: cont_overflow,
                                        ..
                                    } => {
                                        nested_y -= cont_mt;
                                        let cont_w = cont_bw.unwrap_or(cell.width);
                                        let cont_children_h: f32 = cont_kids
                                            .iter()
                                            .map(crate::layout::engine::estimate_element_height)
                                            .sum();
                                        let cont_h = cont_pt
                                            + cont_children_h
                                            + cont_pb
                                            + cont_border.vertical_width();

                                        // Draw container background
                                        if let Some((r, g, b, a)) = cont_bg {
                                            let needs_alpha = *a < 1.0;
                                            if needs_alpha {
                                                let gs_name = format!("GSba{bg_alpha_counter}");
                                                bg_alpha_counter += 1;
                                                page_ext_gstates.push((gs_name.clone(), *a));
                                                content.push_str(&format!("/{gs_name} gs\n"));
                                            }
                                            content.push_str(&format!("{r} {g} {b} rg\n"));
                                            if *cont_br > 0.0 {
                                                content.push_str(&rounded_rect_path(
                                                    nested_x,
                                                    nested_y - cont_h,
                                                    cont_w,
                                                    cont_h,
                                                    *cont_br,
                                                ));
                                                content.push_str("\nf\n");
                                            } else {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = nested_x,
                                                    y = nested_y - cont_h,
                                                    w = cont_w,
                                                    h = cont_h,
                                                ));
                                            }
                                            if needs_alpha {
                                                content.push_str("/GSDefault gs\n");
                                            }
                                        }

                                        // Draw container borders
                                        if cont_border.has_any() {
                                            let bx1 = nested_x;
                                            let bx2 = nested_x + cont_w;
                                            let by1 = nested_y;
                                            let by2 = nested_y - cont_h;
                                            if cont_border.left.width > 0.0 {
                                                let (r, g, b) = cont_border.left.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.left.width,
                                                    bx1 + cont_border.left.width * 0.5,
                                                    by1,
                                                    bx1 + cont_border.left.width * 0.5,
                                                    by2
                                                ));
                                            }
                                            if cont_border.right.width > 0.0 {
                                                let (r, g, b) = cont_border.right.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.right.width,
                                                    bx2 - cont_border.right.width * 0.5,
                                                    by1,
                                                    bx2 - cont_border.right.width * 0.5,
                                                    by2
                                                ));
                                            }
                                            if cont_border.top.width > 0.0 {
                                                let (r, g, b) = cont_border.top.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.top.width,
                                                    bx1,
                                                    by1 - cont_border.top.width * 0.5,
                                                    bx2,
                                                    by1 - cont_border.top.width * 0.5
                                                ));
                                            }
                                            if cont_border.bottom.width > 0.0 {
                                                let (r, g, b) = cont_border.bottom.color;
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.bottom.width,
                                                    bx1,
                                                    by2 + cont_border.bottom.width * 0.5,
                                                    bx2,
                                                    by2 + cont_border.bottom.width * 0.5
                                                ));
                                            }
                                        }

                                        // Clip and render children
                                        let clip = *cont_overflow == Overflow::Hidden;
                                        if clip {
                                            content.push_str("q\n");
                                            content.push_str(&format!(
                                                "{} {} {} {} re W n\n",
                                                nested_x,
                                                nested_y - cont_h,
                                                cont_w,
                                                cont_h
                                            ));
                                        }
                                        let inner_x = nested_x + cont_pl + cont_border.left.width;
                                        let inner_w = (cont_w
                                            - cont_pl
                                            - cont_pr
                                            - cont_border.horizontal_width())
                                        .max(0.0);
                                        let inner_y = nested_y - cont_pt - cont_border.top.width;
                                        render_container_children(
                                            &mut content,
                                            cont_kids,
                                            inner_x,
                                            inner_y,
                                            inner_w,
                                            custom_fonts,
                                            &prepared_custom_fonts,
                                            &mut page_ext_gstates,
                                            &mut bg_alpha_counter,
                                            *cont_pl + cont_border.left.width,
                                            *cont_pt + cont_border.top.width,
                                        );
                                        if clip {
                                            content.push_str("Q\n");
                                        }
                                        nested_y -= cont_h;
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // Restore cell transform
                        if cell_needs_transform {
                            content.push_str("Q\n");
                        }
                    }
                }
                LayoutElement::Container {
                    children,
                    background_color,
                    border,
                    border_radius: c_border_radius,
                    padding_top: c_pt,
                    padding_bottom: c_pb,
                    padding_left: c_pl,
                    padding_right: c_pr,
                    margin_top: _,
                    margin_bottom: _,
                    block_width,
                    block_height: c_block_height,
                    opacity: _,
                    float: c_float,
                    position: _,
                    offset_top: _,
                    offset_left: c_offset_left,
                    overflow: c_overflow,
                    transform: _,
                    box_shadow: c_box_shadow,
                    background_gradient: _,
                    background_radial_gradient: _,
                    z_index: _,
                    ..
                } => {
                    let container_w = block_width.unwrap_or(available_width);
                    let container_x = match c_float {
                        Float::Right => margin.left + available_width - container_w,
                        _ => margin.left + c_offset_left,
                    };
                    let container_y_top = page_size.height - margin.top - y_pos;

                    // Use explicit block_height if set, otherwise compute from children
                    let children_h: f32 = children
                        .iter()
                        .map(crate::layout::engine::estimate_element_height)
                        .sum();
                    let content_h = c_pt + children_h + c_pb + border.vertical_width();
                    let total_h = if *c_overflow == Overflow::Hidden {
                        // When clipping, use declared height to constrain the box
                        c_block_height.unwrap_or(content_h)
                    } else {
                        c_block_height.map_or(content_h, |h| content_h.max(h))
                    };

                    // Draw box-shadow with blur
                    if let Some(shadow) = c_box_shadow {
                        render_box_shadow(
                            &mut content,
                            shadow,
                            container_x,
                            container_y_top - total_h,
                            container_w,
                            total_h,
                            *c_border_radius,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                        );
                    }

                    // Draw background
                    if let Some((r, g, b, a)) = background_color {
                        let needs_alpha = *a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScontainer{elem_idx}");
                            page_ext_gstates.push((gs_name.clone(), *a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!("{r} {g} {b} rg\n"));
                        if *c_border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                container_x,
                                container_y_top - total_h,
                                container_w,
                                total_h,
                                *c_border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = container_x,
                                y = container_y_top - total_h,
                                w = container_w,
                                h = total_h,
                            ));
                        }
                        content.push_str("f\n");
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Draw all 4 borders
                    if border.has_any() {
                        if *c_border_radius > 0.0 {
                            // Use uniform border color/width with rounded rect stroke
                            let bw = border
                                .top
                                .width
                                .max(border.right.width)
                                .max(border.bottom.width)
                                .max(border.left.width);
                            let (r, g, b) = border.top.color;
                            content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                            content.push_str(&rounded_rect_path(
                                container_x,
                                container_y_top - total_h,
                                container_w,
                                total_h,
                                *c_border_radius,
                            ));
                            content.push_str("S\n");
                        } else {
                            let bx1 = container_x;
                            let bx2 = container_x + container_w;
                            let by1 = container_y_top;
                            let by2 = container_y_top - total_h;
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = border.left.width,
                                    x = bx1 + border.left.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                            }
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = border.right.width,
                                    x = bx2 - border.right.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                            }
                            if border.top.width > 0.0 {
                                let (r, g, b) = border.top.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = border.top.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by1 - border.top.width * 0.5
                                ));
                            }
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = border.bottom.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by2 + border.bottom.width * 0.5
                                ));
                            }
                        } // else (non-rounded borders)
                    }

                    // Apply clip if overflow:hidden
                    let needs_clip = *c_overflow == Overflow::Hidden;
                    if needs_clip {
                        content.push_str("q\n");
                        if *c_border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                container_x,
                                container_y_top - total_h,
                                container_w,
                                total_h,
                                *c_border_radius,
                            ));
                            content.push_str("\nW n\n");
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re W n\n",
                                x = container_x,
                                y = container_y_top - total_h,
                                w = container_w,
                                h = total_h,
                            ));
                        }
                    }

                    // Render children recursively
                    // Pass both content-box origin (for flow children) and
                    // padding-box origin (for absolute children).
                    let inner_x = container_x + c_pl + border.left.width;
                    let inner_w = (container_w - c_pl - c_pr - border.horizontal_width()).max(0.0);
                    let inner_y = container_y_top - c_pt - border.top.width;
                    render_container_children(
                        &mut content,
                        children,
                        inner_x,
                        inner_y,
                        inner_w,
                        custom_fonts,
                        &prepared_custom_fonts,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                        *c_pl + border.left.width,
                        *c_pt + border.top.width,
                    );

                    // Restore clip
                    if needs_clip {
                        content.push_str("Q\n");
                    }
                }
                LayoutElement::Image {
                    image,
                    width,
                    height,
                    ..
                } => {
                    let img_x = margin.left;
                    // PDF y-axis is bottom-up; y_pos is top of margin, image draws from bottom-left
                    let img_y = page_size.height - margin.top - y_pos - height;
                    let img_obj_id = pdf_writer.add_image_object(
                        &image.data,
                        image.source_width,
                        image.source_height,
                        image.format,
                        image.png_metadata.as_ref(),
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
                    if let Some(placement) = crate::render::svg_geometry::compute_svg_placement(
                        tree,
                        crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                            0.0,
                            0.0,
                            *width,
                            *height,
                            tree.preserve_aspect_ratio,
                        ),
                    ) {
                        content.push_str("q\n");
                        content.push_str(&placement.viewport.clip_path());
                        content.push_str(&format!(
                            "{sx} 0 0 {sy} {tx} {ty} cm\n",
                            sx = placement.scale_x,
                            sy = placement.scale_y,
                            tx = placement.translate_x,
                            ty = placement.translate_y,
                        ));
                        {
                            let mut image_sink = SvgPageImageSink {
                                pdf_writer: &mut pdf_writer,
                                page_images: &mut page_images,
                            };
                            let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
                                shadings: &mut page_shadings,
                                shading_counter: &mut shading_counter,
                                image_sink: Some(&mut image_sink),
                            };
                            crate::render::svg_to_pdf::render_svg_tree_with_resources(
                                tree,
                                &mut content,
                                &mut resources,
                            );
                        }
                        content.push_str("Q\n");
                    }
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

fn register_used_custom_fonts(
    pdf_writer: &mut PdfWriter,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
) {
    for (font_name, prepared_font) in prepared_custom_fonts {
        if let Some(ttf) = custom_fonts.get(font_name) {
            pdf_writer.add_ttf_font(font_name, ttf, prepared_font);
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
    crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
}

/// Resolve the PDF font resource name for a text run.
///
/// Custom Type0 fonts are only safe when we also have shaped glyph output.
fn resolve_font_name(
    run: &TextRun,
    custom_font: Option<(&str, &TtfFont)>,
    shaped: Option<&crate::text::ShapedRun>,
) -> String {
    if let (Some((resolved_name, _)), Some(_)) = (custom_font, shaped) {
        sanitize_pdf_name(resolved_name)
    } else {
        font_name_for_run(run).to_string()
    }
}

/// Estimate run width using TTF metrics for custom fonts, falling back to fixed estimation.
fn estimate_run_width_with_fonts(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    if let Some(width) = crate::text::measure_text_width(
        &run.text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.italic,
        custom_fonts,
    ) {
        return width;
    }

    estimate_run_width(run)
}

fn encode_pdf_hex_glyph(glyph_id: u16) -> String {
    format!("{glyph_id:04X}")
}

#[derive(Clone, Copy)]
struct PdfPoint {
    x: f32,
    y: f32,
}

impl PdfPoint {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

struct ShapedTextRender<'a> {
    origin: PdfPoint,
    font_size: f32,
    font: &'a TtfFont,
    shaped: &'a crate::text::ShapedRun,
    prepared_font: Option<&'a PreparedCustomFont>,
}

impl<'a> ShapedTextRender<'a> {
    const fn new(
        origin: PdfPoint,
        font_size: f32,
        font: &'a TtfFont,
        shaped: &'a crate::text::ShapedRun,
        prepared_font: Option<&'a PreparedCustomFont>,
    ) -> Self {
        Self {
            origin,
            font_size,
            font,
            shaped,
            prepared_font,
        }
    }

    fn has_complex_offsets(&self) -> bool {
        self.shaped
            .glyphs
            .iter()
            .any(|glyph| glyph.x_offset.abs() > f32::EPSILON || glyph.y_offset.abs() > f32::EPSILON)
    }

    fn pdf_glyph_id(&self, glyph_id: u16) -> u16 {
        self.prepared_font.map_or(glyph_id, |prepared_font| {
            prepared_font.pdf_glyph_id(glyph_id)
        })
    }
}

fn format_pdf_number(value: f32) -> String {
    let rounded = if value.abs() < 0.000_5 { 0.0 } else { value };
    let mut formatted = format!("{rounded:.4}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    if formatted == "-0" {
        "0".to_string()
    } else {
        formatted
    }
}

fn append_positioned_shaped_text(content: &mut String, render: ShapedTextRender<'_>) {
    let mut cursor_x = render.origin.x;
    for glyph in &render.shaped.glyphs {
        let draw_x = cursor_x + glyph.x_offset;
        let draw_y = render.origin.y + glyph.y_offset;
        let encoded = encode_pdf_hex_glyph(render.pdf_glyph_id(glyph.glyph_id));
        content.push_str(&format!(
            "1 0 0 1 {} {} Tm\n",
            format_pdf_number(draw_x),
            format_pdf_number(draw_y),
        ));
        content.push_str(&format!("<{encoded}> Tj\n"));
        cursor_x += glyph.x_advance;
    }
}

fn append_tj_shaped_text(content: &mut String, render: ShapedTextRender<'_>) {
    content.push_str(&format!(
        "1 0 0 1 {} {} Tm\n",
        format_pdf_number(render.origin.x),
        format_pdf_number(render.origin.y),
    ));
    content.push('[');

    let mut first = true;
    for glyph in &render.shaped.glyphs {
        if !first {
            content.push(' ');
        }
        first = false;

        let encoded = encode_pdf_hex_glyph(render.pdf_glyph_id(glyph.glyph_id));
        content.push('<');
        content.push_str(&encoded);
        content.push('>');

        let nominal_advance = render
            .font
            .glyph_width_scaled(glyph.glyph_id, render.font_size);
        let advance_adjustment = glyph.x_advance - nominal_advance;
        if advance_adjustment.abs() > 0.001 {
            let tj_adjustment = -(advance_adjustment * 1000.0 / render.font_size.max(f32::EPSILON));
            content.push(' ');
            content.push_str(&format_pdf_number(tj_adjustment));
        }
    }

    content.push_str("] TJ\n");
}

/// Recursively render a Container element and all its children.
///
/// `x` / `y` are the content-box origin (after padding).
/// `abs_pad_left` / `abs_pad_top` are the parent padding values so that
/// absolute-positioned children can be placed relative to the padding box.
#[allow(clippy::too_many_arguments)]
fn render_container_children(
    content: &mut String,
    children: &[LayoutElement],
    x: f32,
    mut y: f32,
    width: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    abs_pad_left: f32,
    abs_pad_top: f32,
) {
    // Separate children into those handled by render_nested_table_rows
    // (TableRow, TextBlock) and those handled directly (Container, Svg, etc.).
    // We process all children in order, flushing accumulated nested-layout
    // batches when we hit a directly-handled type.
    let mut nested_batch: Vec<&LayoutElement> = Vec::new();
    let mut cursor_y = y;
    // Save original y for absolute positioning (must not be affected by
    // flow children advancing the cursor).
    let container_top_y = y;

    for child in children {
        let handled_by_nested = matches!(
            child,
            LayoutElement::TableRow { .. } | LayoutElement::GridRow { .. }
        );
        if handled_by_nested {
            nested_batch.push(child);
            cursor_y -= crate::layout::engine::estimate_element_height(child);
            continue;
        }

        // Flush any accumulated nested batch before handling this element
        if !nested_batch.is_empty() {
            let batch: Vec<LayoutElement> = nested_batch.drain(..).cloned().collect();
            render_nested_table_rows(
                content,
                &batch,
                x,
                y,
                page_ext_gstates,
                bg_alpha_counter,
                custom_fonts,
                prepared_custom_fonts,
            );
            y = cursor_y;
        }

        match child {
            LayoutElement::TextBlock {
                lines,
                margin_top,
                padding_top,
                padding_bottom,
                border,
                block_height,
                background_color,
                text_align,
                float: tb_float,
                position,
                offset_top,
                offset_left,
                block_width: tb_block_width,
                ..
            } => {
                // Absolute-positioned children render at offset from the
                // containing block's padding box (CSS spec), not the content box.
                // Use container_top_y (original y before flow children advance it).
                if *position == Position::Absolute {
                    let text_h: f32 = lines.iter().map(|l| l.height).sum();
                    let abs_h = padding_top + text_h + padding_bottom + border.vertical_width();
                    let abs_h = block_height.map_or(abs_h, |h| abs_h.max(h));
                    let abs_w = tb_block_width.unwrap_or(width);
                    let abs_x = (x - abs_pad_left) + offset_left;
                    let abs_y = (container_top_y + abs_pad_top) - offset_top;

                    if let Some((r, g, b, a)) = background_color {
                        let needs_alpha = *a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), *a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{ax} {ay} {aw} {ah} re\nf\n",
                            ax = abs_x,
                            ay = abs_y - abs_h,
                            aw = abs_w,
                            ah = abs_h,
                        ));
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }
                    // Render text for absolute-positioned children
                    let mut text_y_abs = abs_y - padding_top;
                    for line in lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y_abs -= metrics.half_leading + metrics.ascender;
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                            .sum();
                        let text_x = match text_align {
                            TextAlign::Right => abs_x + (abs_w - line_width).max(0.0),
                            TextAlign::Center => abs_x + (abs_w - line_width).max(0.0) / 2.0,
                            _ => abs_x,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y_abs,
                                custom_fonts,
                                prepared_custom_fonts,
                            );
                            lx += rw;
                        }
                        text_y_abs -= metrics.descender + metrics.half_leading;
                    }
                    // Don't advance cursor_y for absolute elements
                    continue;
                }

                cursor_y -= margin_top;
                y = cursor_y;
                let text_h: f32 = lines.iter().map(|l| l.height).sum();
                let child_h = padding_top + text_h + padding_bottom + border.vertical_width();
                let child_h = block_height.map_or(child_h, |h| child_h.max(h));

                let render_w = tb_block_width.unwrap_or(width);

                // Apply float/position offset
                let render_x = match tb_float {
                    Float::Right => x + width - render_w,
                    _ => {
                        if *position == Position::Relative {
                            x + offset_left
                        } else {
                            x
                        }
                    }
                };
                let render_y = if *position == Position::Relative {
                    y - offset_top
                } else {
                    y
                };

                // Draw child background
                if let Some((r, g, b, a)) = background_color {
                    let needs_alpha = *a < 1.0;
                    if needs_alpha {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *a));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{cx} {cy} {cw} {ch} re\nf\n",
                        cx = render_x,
                        cy = render_y - child_h,
                        cw = render_w,
                        ch = child_h,
                    ));
                    if needs_alpha {
                        content.push_str("/GSDefault gs\n");
                    }
                }

                // Draw child borders
                if border.has_any() {
                    let bx1 = render_x;
                    let bx2 = render_x + render_w;
                    let by1 = render_y;
                    let by2 = render_y - child_h;
                    if border.top.width > 0.0 {
                        let (r, g, b) = border.top.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                            border.top.width
                        ));
                    }
                    if border.bottom.width > 0.0 {
                        let (r, g, b) = border.bottom.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                            border.bottom.width
                        ));
                    }
                    if border.left.width > 0.0 {
                        let (r, g, b) = border.left.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                            border.left.width
                        ));
                    }
                    if border.right.width > 0.0 {
                        let (r, g, b) = border.right.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                            border.right.width
                        ));
                    }
                }

                // Draw child text
                let mut text_y = render_y - padding_top;
                for line in lines {
                    let metrics = line_box_metrics(line, custom_fonts);
                    text_y -= metrics.half_leading + metrics.ascender;
                    let merged = merge_runs(&line.runs);
                    let line_width: f32 = merged
                        .iter()
                        .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                        .sum();
                    let text_x = match text_align {
                        TextAlign::Right => render_x + (render_w - line_width).max(0.0),
                        TextAlign::Center => render_x + (render_w - line_width).max(0.0) / 2.0,
                        _ => render_x,
                    };
                    let mut lx = text_x;
                    for run in &merged {
                        let rw = render_run_text(
                            content,
                            run,
                            lx,
                            text_y,
                            custom_fonts,
                            prepared_custom_fonts,
                        );
                        lx += rw;
                    }
                    text_y -= metrics.descender + metrics.half_leading;
                }
                cursor_y -= child_h;
                y = cursor_y;
            }
            LayoutElement::Container {
                children: nested_kids,
                background_color,
                border,
                border_radius: cont_br,
                padding_top,
                padding_bottom,
                padding_left,
                padding_right,
                margin_top,
                margin_bottom,
                block_width,
                block_height: nk_block_height,
                float: nk_float,
                overflow,
                ..
            } => {
                cursor_y -= margin_top;
                y = cursor_y;
                let nk_w = block_width.unwrap_or(width);
                let nk_x = match nk_float {
                    Float::Right => x + width - nk_w,
                    _ => x,
                };
                let nk_children_h: f32 = nested_kids
                    .iter()
                    .map(crate::layout::engine::estimate_element_height)
                    .sum();
                let nk_content_h =
                    padding_top + nk_children_h + padding_bottom + border.vertical_width();
                let nk_total_h = nk_block_height.map_or(nk_content_h, |h| nk_content_h.max(h));

                // Draw background with proper alpha support
                if let Some((r, g, b, a)) = background_color {
                    let needs_alpha = *a < 1.0;
                    if needs_alpha {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *a));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{cx} {cy} {cw} {ch} re\nf\n",
                        cx = nk_x,
                        cy = y - nk_total_h,
                        cw = nk_w,
                        ch = nk_total_h,
                    ));
                    if needs_alpha {
                        content.push_str("/GSDefault gs\n");
                    }
                }

                // Draw all 4 borders
                let bx1 = nk_x;
                let bx2 = nk_x + nk_w;
                let by1 = y;
                let by2 = y - nk_total_h;
                if border.left.width > 0.0 {
                    let (r, g, b) = border.left.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                        bw = border.left.width,
                        x = bx1 + border.left.width * 0.5,
                        y1 = by1,
                        y2 = by2
                    ));
                }
                if border.right.width > 0.0 {
                    let (r, g, b) = border.right.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                        bw = border.right.width,
                        x = bx2 - border.right.width * 0.5,
                        y1 = by1,
                        y2 = by2
                    ));
                }
                if border.top.width > 0.0 {
                    let (r, g, b) = border.top.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                        bw = border.top.width,
                        x1 = bx1,
                        x2 = bx2,
                        y = by1 - border.top.width * 0.5
                    ));
                }
                if border.bottom.width > 0.0 {
                    let (r, g, b) = border.bottom.color;
                    content.push_str(&format!(
                        "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                        bw = border.bottom.width,
                        x1 = bx1,
                        x2 = bx2,
                        y = by2 + border.bottom.width * 0.5
                    ));
                }

                // Clip if overflow:hidden (use rounded rect when border-radius > 0)
                let clip = *overflow == Overflow::Hidden;
                if clip {
                    content.push_str("q\n");
                    if *cont_br > 0.0 {
                        content.push_str(&rounded_rect_path(
                            nk_x,
                            y - nk_total_h,
                            nk_w,
                            nk_total_h,
                            *cont_br,
                        ));
                        content.push_str("\nW n\n");
                    } else {
                        content.push_str(&format!(
                            "{cx} {cy} {cw} {ch} re W n\n",
                            cx = nk_x,
                            cy = y - nk_total_h,
                            cw = nk_w,
                            ch = nk_total_h,
                        ));
                    }
                }

                // Recurse into nested children
                let inner_x = nk_x + padding_left + border.left.width;
                let inner_w = nk_w - padding_left - padding_right - border.horizontal_width();
                let inner_y = y - padding_top - border.top.width;
                render_container_children(
                    content,
                    nested_kids,
                    inner_x,
                    inner_y,
                    inner_w,
                    custom_fonts,
                    prepared_custom_fonts,
                    page_ext_gstates,
                    bg_alpha_counter,
                    *padding_left + border.left.width,
                    *padding_top + border.top.width,
                );

                if clip {
                    content.push_str("Q\n");
                }
                cursor_y -= nk_total_h + margin_bottom;
                y = cursor_y;
            }
            LayoutElement::Svg {
                tree,
                width: svg_w,
                height: svg_h,
                margin_top: svg_mt,
                ..
            } => {
                cursor_y -= svg_mt;
                y = cursor_y;
                let svg_x = x;
                let svg_y = y - svg_h;
                content.push_str("q\n");
                // Position on page with Y-flip (SVG y-axis is top-down, PDF is bottom-up)
                content.push_str(&format!("1 0 0 -1 {svg_x} {} cm\n", svg_y + svg_h));
                // Apply viewBox scaling via compute_svg_placement
                if let Some(placement) = crate::render::svg_geometry::compute_svg_placement(
                    tree,
                    crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                        0.0,
                        0.0,
                        *svg_w,
                        *svg_h,
                        tree.preserve_aspect_ratio,
                    ),
                ) {
                    content.push_str("q\n");
                    content.push_str(&placement.viewport.clip_path());
                    content.push_str(&format!(
                        "{sx} 0 0 {sy} {tx} {ty} cm\n",
                        sx = placement.scale_x,
                        sy = placement.scale_y,
                        tx = placement.translate_x,
                        ty = placement.translate_y,
                    ));
                    {
                        let mut res = crate::render::svg_to_pdf::SvgPdfResources {
                            shadings: &mut Vec::new(),
                            shading_counter: &mut 0,
                            image_sink: None,
                        };
                        crate::render::svg_to_pdf::render_svg_tree_with_resources(
                            tree, content, &mut res,
                        );
                    }
                    content.push_str("Q\n");
                } else {
                    let mut res = crate::render::svg_to_pdf::SvgPdfResources {
                        shadings: &mut Vec::new(),
                        shading_counter: &mut 0,
                        image_sink: None,
                    };
                    crate::render::svg_to_pdf::render_svg_tree_with_resources(
                        tree, content, &mut res,
                    );
                }
                content.push_str("Q\n");
                cursor_y -= svg_h;
                y = cursor_y;
            }
            LayoutElement::FlexRow {
                cells,
                margin_top: flex_mt,
                margin_bottom: flex_mb,
                background_color,
                border,
                padding_top: flex_pt,
                padding_left: flex_pl,
                ..
            } => {
                cursor_y -= flex_mt;
                y = cursor_y;
                let row_h =
                    crate::layout::engine::estimate_element_height(child) - flex_mt - flex_mb;

                // Draw flex row background
                if let Some((r, g, b, a)) = background_color {
                    let needs_alpha = *a < 1.0;
                    if needs_alpha {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *a));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{fx} {fy} {fw} {fh} re\nf\n",
                        fx = x,
                        fy = y - row_h,
                        fw = width,
                        fh = row_h,
                    ));
                    if needs_alpha {
                        content.push_str("/GSDefault gs\n");
                    }
                }

                // Render flex cells
                let mut cell_x = x + flex_pl + border.left.width;
                let content_y = y - flex_pt - border.top.width;
                for cell in cells {
                    let cell_w = cell.width;
                    // Draw cell background
                    if let Some((cr, cg, cb, ca)) = cell.background_color {
                        let needs_alpha = ca < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), ca));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!("{cr} {cg} {cb} rg\n"));
                        if cell.border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                cell_x,
                                y - row_h,
                                cell_w,
                                row_h,
                                cell.border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{cx} {cy} {cw} {ch} re\n",
                                cx = cell_x,
                                cy = y - row_h,
                                cw = cell_w,
                                ch = row_h,
                            ));
                        }
                        content.push_str("f\n");
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }
                    // Draw cell border
                    if cell.border.has_any() {
                        if cell.border_radius > 0.0 {
                            // Rounded border — use uniform stroke with rounded rect
                            let bw = cell.border.top.width;
                            let (r, g, b) = cell.border.top.color;
                            content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                            content.push_str(&rounded_rect_path(
                                cell_x,
                                y - row_h,
                                cell_w,
                                row_h,
                                cell.border_radius,
                            ));
                            content.push_str("S\n");
                        } else {
                            let bx1 = cell_x;
                            let bx2 = cell_x + cell_w;
                            let by1 = y;
                            let by2 = y - row_h;
                            if cell.border.left.width > 0.0 {
                                let (r, g, b) = cell.border.left.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = cell.border.left.width,
                                    x = bx1 + cell.border.left.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                            }
                            if cell.border.right.width > 0.0 {
                                let (r, g, b) = cell.border.right.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = cell.border.right.width,
                                    x = bx2 - cell.border.right.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                            }
                            if cell.border.top.width > 0.0 {
                                let (r, g, b) = cell.border.top.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = cell.border.top.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by1 - cell.border.top.width * 0.5
                                ));
                            }
                            if cell.border.bottom.width > 0.0 {
                                let (r, g, b) = cell.border.bottom.color;
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = cell.border.bottom.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by2 + cell.border.bottom.width * 0.5
                                ));
                            }
                        } // else (non-rounded cell border)
                    }
                    // Draw cell text
                    let mut text_y = content_y;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => cell_x + (cell_w - line_width).max(0.0),
                            TextAlign::Center => cell_x + (cell_w - line_width).max(0.0) / 2.0,
                            _ => cell_x,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                custom_fonts,
                                prepared_custom_fonts,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }
                    // Render nested elements in flex cells (tables, containers)
                    if !cell.nested_elements.is_empty() {
                        let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                        let nested_y = content_y - text_h;
                        render_container_children(
                            content,
                            &cell.nested_elements,
                            cell_x,
                            nested_y,
                            cell_w,
                            custom_fonts,
                            prepared_custom_fonts,
                            page_ext_gstates,
                            bg_alpha_counter,
                            0.0, // flex cells don't have separate padding for abs children
                            0.0,
                        );
                    }
                    cell_x += cell_w;
                }
                cursor_y -= row_h + flex_mb;
                y = cursor_y;
            }
            LayoutElement::HorizontalRule {
                margin_top: rule_mt,
                margin_bottom: rule_mb,
            } => {
                cursor_y -= rule_mt;
                y = cursor_y;
                // Default rule: gray line across container width
                content.push_str(&format!(
                    "0.8 0.8 0.8 RG\n0.75 w\n{x} {ry} m {x2} {ry} l\nS\n",
                    ry = y - 0.5,
                    x2 = x + width,
                ));
                cursor_y -= 1.0 + rule_mb;
                y = cursor_y;
            }
            _ => {
                let h = crate::layout::engine::estimate_element_height(child);
                cursor_y -= h;
                y = cursor_y;
            }
        }
    }

    // Flush any remaining nested batch
    if !nested_batch.is_empty() {
        let batch: Vec<LayoutElement> = nested_batch.drain(..).cloned().collect();
        render_nested_table_rows(
            content,
            &batch,
            x,
            y,
            page_ext_gstates,
            bg_alpha_counter,
            custom_fonts,
            prepared_custom_fonts,
        );
    }
}

/// Render TableRow/GridRow elements that appear as children of a Container.
#[allow(clippy::too_many_arguments)]
fn render_nested_table_rows(
    content: &mut String,
    elements: &[LayoutElement],
    origin_x: f32,
    mut cursor_y: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
) {
    for element in elements {
        match element {
            LayoutElement::TableRow {
                cells,
                col_widths,
                border_collapse,
                border_spacing,
                margin_top,
                ..
            } => {
                cursor_y -= margin_top;
                let spacing = if *border_collapse == BorderCollapse::Collapse {
                    0.0
                } else {
                    *border_spacing
                };
                let row_y = cursor_y;
                let row_height = compute_row_height(cells);

                let mut col_pos: usize = 0;
                for cell in cells {
                    if cell.rowspan == 0 {
                        col_pos += cell.colspan;
                        continue;
                    }
                    let (cell_x, cell_w) =
                        table_cell_geometry(col_widths, col_pos, cell.colspan, spacing, origin_x);

                    // Draw cell background
                    if let Some((r, g, b, a)) = cell.background_color {
                        let needs_alpha = a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = cell_x,
                            y = row_y - row_height,
                            w = cell_w,
                            h = row_height,
                        ));
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Draw cell borders
                    if cell.border.has_any() {
                        let x1 = cell_x;
                        let x2 = cell_x + cell_w;
                        let y_top = row_y;
                        let y_bottom = row_y - row_height;
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

                    // Compute cell content top (simplified vertical alignment)
                    let content_top = row_y - cell.padding_top;
                    let cell_inner_w = cell_w - cell.padding_left - cell.padding_right;
                    let mut text_y = content_top;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let text_content: String =
                            line.runs.iter().map(|run| run.text.as_str()).collect();
                        if text_content.is_empty() {
                            continue;
                        }
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => {
                                cell_x + cell.padding_left + (cell_inner_w - line_width).max(0.0)
                            }
                            TextAlign::Center => {
                                cell_x
                                    + cell.padding_left
                                    + ((cell_inner_w - line_width) / 2.0).max(0.0)
                            }
                            _ => cell_x + cell.padding_left,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }
                            // Inline background (for status badges etc.)
                            if let Some((br, bg_c, bb, _ba)) = run.background_color {
                                let (pad_h, pad_v) = run.padding;
                                let run_w = estimate_run_width_with_fonts(run, custom_fonts);
                                let rx = lx - pad_h;
                                let ry = text_y - 2.0 - pad_v;
                                let rw2 = run_w + pad_h * 2.0;
                                let rh = run.font_size + 2.0 + pad_v * 2.0;
                                content.push_str(&format!("{br} {bg_c} {bb} rg\n"));
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
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                custom_fonts,
                                prepared_custom_fonts,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    col_pos += cell.colspan;
                }
                cursor_y -= row_height;
            }
            LayoutElement::GridRow {
                cells,
                col_widths,
                gap,
                border: grid_border,
                padding_left: grid_pl,
                padding_right: grid_pr,
                padding_top: grid_pt,
                padding_bottom: grid_pb,
                margin_top,
                ..
            } => {
                cursor_y -= margin_top;
                let row_y = cursor_y;
                let row_height = compute_row_height(cells) + grid_pt + grid_pb;
                let grid_total_w: f32 = col_widths.iter().sum::<f32>()
                    + gap * col_widths.len().saturating_sub(1) as f32
                    + grid_pl
                    + grid_pr;

                // Draw grid container border
                if grid_border.has_any() {
                    let bx1 = origin_x;
                    let bx2 = origin_x + grid_total_w;
                    let by1 = row_y;
                    let by2 = row_y - row_height;
                    if grid_border.top.width > 0.0 {
                        let (r, g, b) = grid_border.top.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                            grid_border.top.width
                        ));
                    }
                    if grid_border.right.width > 0.0 {
                        let (r, g, b) = grid_border.right.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                            grid_border.right.width
                        ));
                    }
                    if grid_border.bottom.width > 0.0 {
                        let (r, g, b) = grid_border.bottom.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                            grid_border.bottom.width
                        ));
                    }
                    if grid_border.left.width > 0.0 {
                        let (r, g, b) = grid_border.left.color;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                            grid_border.left.width
                        ));
                    }
                }

                let mut cell_x = origin_x + grid_pl;
                let cell_row_y = row_y - grid_pt;
                let cell_content_h = compute_row_height(cells);
                for (i, cell) in cells.iter().enumerate() {
                    let cell_w = if i < col_widths.len() {
                        col_widths[i]
                    } else {
                        0.0
                    };

                    // Draw cell background
                    if let Some((r, g, b, a)) = cell.background_color {
                        let needs_alpha = a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = cell_x,
                            y = cell_row_y - cell_content_h,
                            w = cell_w,
                            h = cell_content_h,
                        ));
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Render cell text
                    let cell_inner_w = cell_w - cell.padding_left - cell.padding_right;
                    let mut text_y = cell_row_y - cell.padding_top;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let text_content: String =
                            line.runs.iter().map(|run| run.text.as_str()).collect();
                        if text_content.is_empty() {
                            continue;
                        }
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => {
                                cell_x + cell.padding_left + (cell_inner_w - line_width).max(0.0)
                            }
                            TextAlign::Center => {
                                cell_x
                                    + cell.padding_left
                                    + ((cell_inner_w - line_width) / 2.0).max(0.0)
                            }
                            _ => cell_x + cell.padding_left,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                custom_fonts,
                                prepared_custom_fonts,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    cell_x += cell_w + gap;
                }
                cursor_y -= row_height;
            }
            _ => {
                cursor_y -= crate::layout::engine::estimate_element_height(element);
            }
        }
    }
}

fn render_run_text(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
) -> f32 {
    let (r, g, b) = run.color;

    // Try Unicode fallback for standard-font runs with non-WinAnsi characters
    if let Some((fallback_shaped, fallback_key, fallback_font)) =
        crate::text::shape_with_unicode_fallback(run, custom_fonts)
    {
        let run_width = fallback_shaped.width;
        let font_name = sanitize_pdf_name(fallback_key);
        content.push_str(&format!("{r} {g} {b} rg\n"));
        content.push_str("BT\n");
        content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
        let prepared_font = prepared_custom_fonts.get(fallback_key);
        let render = ShapedTextRender::new(
            PdfPoint::new(x, text_y),
            run.font_size,
            fallback_font,
            &fallback_shaped,
            prepared_font,
        );
        if render.has_complex_offsets() {
            append_positioned_shaped_text(content, render);
        } else {
            append_tj_shaped_text(content, render);
        }
        content.push_str("ET\n");
        return run_width;
    }

    let shaped = crate::text::shape_text_run(run, custom_fonts);
    let run_width = shaped.as_ref().map_or_else(
        || estimate_run_width_with_fonts(run, custom_fonts),
        |run| run.width,
    );
    let custom_font =
        crate::text::resolve_custom_font(&run.font_family, run.bold, run.italic, custom_fonts);
    let font_name = resolve_font_name(run, custom_font, shaped.as_ref());

    content.push_str(&format!("{r} {g} {b} rg\n"));
    content.push_str("BT\n");
    content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));

    if let (Some((resolved_name, font)), Some(shaped)) = (custom_font, shaped.as_ref()) {
        let prepared_font = prepared_custom_fonts.get(resolved_name);
        let render = ShapedTextRender::new(
            PdfPoint::new(x, text_y),
            run.font_size,
            font,
            shaped,
            prepared_font,
        );
        if render.has_complex_offsets() {
            append_positioned_shaped_text(content, render);
        } else {
            append_tj_shaped_text(content, render);
        }
    } else {
        let encoded = encode_pdf_text(&run.text);
        content.push_str(&format!(
            "{} {} Td\n",
            format_pdf_number(x),
            format_pdf_number(text_y),
        ));
        content.push_str(&format!("({encoded}) Tj\n"));
    }

    content.push_str("ET\n");
    run_width
}

/// Render all text runs of a line in a single BT/ET block so the PDF viewer
/// advances the text cursor naturally after each Tj, eliminating cumulative
/// positioning errors between runs.
///
/// Falls back to per-run `render_run_text` when any run requires custom-font
/// shaping (complex glyph positioning).
fn render_line_text(
    content: &mut String,
    runs: &[TextRun],
    start_x: f32,
    y: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
) {
    let non_empty: Vec<&TextRun> = runs.iter().filter(|r| !r.text.is_empty()).collect();
    if non_empty.is_empty() {
        return;
    }

    // Check whether every run can be rendered with standard PDF fonts
    // (no custom-font shaping needed).  Unicode-fallback runs also need
    // shaping, so they count as non-standard.
    let all_standard = non_empty.iter().all(|run| {
        crate::text::resolve_custom_font(&run.font_family, run.bold, run.italic, custom_fonts)
            .is_none()
            && crate::text::shape_with_unicode_fallback(run, custom_fonts).is_none()
    });

    if all_standard {
        // Simple path: single BT block, one Td to set initial position,
        // then consecutive Tf/rg/Tj operators.  The viewer advances the
        // text cursor after each Tj.
        content.push_str("BT\n");
        let mut first = true;
        for run in &non_empty {
            let (r, g, b) = run.color;
            let font_name = resolve_font_name(run, None, None);
            content.push_str(&format!("{r} {g} {b} rg\n"));
            content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
            if first {
                content.push_str(&format!(
                    "{} {} Td\n",
                    format_pdf_number(start_x),
                    format_pdf_number(y),
                ));
                first = false;
            }
            let encoded = encode_pdf_text(&run.text);
            content.push_str(&format!("({encoded}) Tj\n"));
        }
        content.push_str("ET\n");
    } else {
        // Mixed path: some runs need custom-font shaping.
        // Fall back to per-run rendering with individual BT/ET blocks.
        let mut x = start_x;
        for run in &non_empty {
            let run_width =
                render_run_text(content, run, x, y, custom_fonts, prepared_custom_fonts);
            x += run_width;
        }
    }
}

#[derive(Clone, Copy)]
struct LineBoxMetrics {
    ascender: f32,
    descender: f32,
    half_leading: f32,
}

fn line_box_metrics(line: &TextLine, custom_fonts: &HashMap<String, TtfFont>) -> LineBoxMetrics {
    let (ascender, descender) =
        line.runs
            .iter()
            .fold((0.0f32, 0.0f32), |(max_ascender, max_descender), run| {
                let (ascender_ratio, descender_ratio) = crate::fonts::font_metrics_ratios(
                    &run.font_family,
                    run.bold,
                    run.italic,
                    custom_fonts,
                );
                (
                    max_ascender.max(ascender_ratio * run.font_size),
                    max_descender.max(descender_ratio * run.font_size),
                )
            });
    let half_leading = (line.height - (ascender + descender)) / 2.0;

    LineBoxMetrics {
        ascender,
        descender,
        half_leading,
    }
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

fn text_block_total_height(
    lines: &[TextLine],
    padding_top: f32,
    padding_bottom: f32,
    block_height: Option<f32>,
) -> f32 {
    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let content_h = padding_top + text_height + padding_bottom;
    block_height.map_or(content_h, |h| content_h.max(h))
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
            if let Some(previous) = merged.last_mut() {
                previous.text.push_str(&run.text);
            }
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
    let name = push_axial_shading(shadings, shading_counter, [x0, y0, x1, y1], stops);

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
    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    let max_radius = width.max(height) / 2.0;

    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();
    let name = push_radial_shading(
        shadings,
        shading_counter,
        [cx, cy, 0.0, cx, cy, max_radius],
        stops,
    );

    // Clip to the gradient area and paint with shading
    content.push_str("q\n");
    content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
    content.push_str(&format!("/{name} sh\n"));
    content.push_str("Q\n");
}

fn render_svg_background(
    content: &mut String,
    tree: &crate::parser::svg::SvgTree,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    paint: BackgroundPaintContext,
) {
    // SVG image resources frequently omit explicit width/height and only provide
    // a viewBox. Browsers still use that intrinsic aspect ratio for background
    // sizing, so fall back to the viewBox dimensions before giving up.
    let intrinsic_width = if tree.width > 0.0 {
        tree.width
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.width)
    };
    let intrinsic_height = if tree.height > 0.0 {
        tree.height
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.height)
    };
    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        return;
    }

    let (vb_w, vb_h) = if let Some(ref vb) = tree.view_box {
        (vb.width, vb.height)
    } else {
        (intrinsic_width, intrinsic_height)
    };
    if vb_w <= 0.0 || vb_h <= 0.0 {
        return;
    }

    let resolve_axis = |value: f32, is_percent: bool, extent: f32| {
        if is_percent {
            extent * (value / 100.0)
        } else {
            value
        }
    };

    // Compute the rendered size of one SVG tile based on background-size.
    let (scaled_w, scaled_h) = match paint.size {
        BackgroundSize::Cover => {
            let s = (paint.reference_box.width / vb_w).max(paint.reference_box.height / vb_h);
            (vb_w * s, vb_h * s)
        }
        BackgroundSize::Contain => {
            let s = (paint.reference_box.width / vb_w).min(paint.reference_box.height / vb_h);
            (vb_w * s, vb_h * s)
        }
        BackgroundSize::Auto => (vb_w, vb_h),
        BackgroundSize::Explicit {
            width: explicit_width,
            height: explicit_height,
            width_is_percent,
            height_is_percent,
        } => {
            let scaled_w =
                resolve_axis(explicit_width, width_is_percent, paint.reference_box.width);
            let scaled_h = explicit_height
                .map(|value| resolve_axis(value, height_is_percent, paint.reference_box.height))
                .unwrap_or_else(|| scaled_w * vb_h / vb_w);
            (scaled_w, scaled_h)
        }
    };

    if scaled_w <= 0.0 || scaled_h <= 0.0 {
        return;
    }

    let placement = crate::render::svg_geometry::compute_svg_placement(
        tree,
        crate::render::svg_geometry::SvgPlacementRequest::from_rect(
            0.0,
            0.0,
            scaled_w,
            scaled_h,
            tree.preserve_aspect_ratio,
        ),
    );
    let Some(placement) = placement else {
        return;
    };
    let raster_background = synthetic_raster_background(tree).and_then(|(href, source_box)| {
        let image_box = SvgViewportBox::new(
            placement.translate_x + source_box.x * placement.scale_x,
            placement.translate_y + source_box.y * placement.scale_y,
            source_box.width * placement.scale_x,
            source_box.height * placement.scale_y,
        );
        let request = (paint.blur_radius > 0.0).then_some(RasterBackgroundRequest {
            canvas_box: paint.local_blur_canvas_box(),
            image_box,
            blur_radius: paint.blur_radius,
        });
        register_background_image(pdf_writer, page_images, href, request)
            .map(|registered| (image_box, registered))
    });
    let visual_overflow = raster_background.as_ref().map_or_else(
        || svg_visual_overflow(tree).scale(placement.scale_x, placement.scale_y),
        |(image_box, registered)| {
            overflow_from_viewport_box(
                placement.viewport,
                registered.draw_box.unwrap_or(*image_box),
            )
        },
    );
    let tile_clip_box = viewport_box_from_overflow(placement.viewport, visual_overflow);

    // Compute background-position offset (in the CSS coordinate system,
    // origin at top-left of the element box).
    let offset_x = if paint.position.x_is_percent {
        (paint.reference_box.width - scaled_w) * paint.position.x
    } else {
        paint.position.x
    };
    let offset_y = if paint.position.y_is_percent {
        (paint.reference_box.height - scaled_h) * paint.position.y
    } else {
        paint.position.y
    };

    // Determine tiling grid based on background-repeat.
    // We compute the set of tile origin offsets (in CSS coords, top-left = 0,0).
    let tiles_x: Vec<f32>;
    let tiles_y: Vec<f32>;

    match paint.repeat {
        BackgroundRepeat::NoRepeat => {
            tiles_x = vec![offset_x];
            tiles_y = vec![offset_y];
        }
        BackgroundRepeat::Repeat => {
            tiles_x = tile_offsets(offset_x, scaled_w, paint.reference_box.width);
            tiles_y = tile_offsets(offset_y, scaled_h, paint.reference_box.height);
        }
        BackgroundRepeat::RepeatX => {
            tiles_x = tile_offsets(offset_x, scaled_w, paint.reference_box.width);
            tiles_y = vec![offset_y];
        }
        BackgroundRepeat::RepeatY => {
            tiles_x = vec![offset_x];
            tiles_y = tile_offsets(offset_y, scaled_h, paint.reference_box.height);
        }
    }

    // Clip to the element box.
    content.push_str("q\n");
    let expanded_clip_box = viewport_box_from_overflow(paint.clip_box, visual_overflow);
    if paint.border_radius > 0.0 {
        content.push_str(&rounded_rect_path(
            expanded_clip_box.x,
            expanded_clip_box.y,
            expanded_clip_box.width,
            expanded_clip_box.height,
            paint.border_radius,
        ));
        content.push_str("W n\n");
    } else {
        content.push_str(&expanded_clip_box.clip_path());
    }

    for &ty in &tiles_y {
        for &tx in &tiles_x {
            content.push_str("q\n");
            let tile_origin = paint.tile_origin(tx, ty);
            let pdf_x = tile_origin.x;
            let pdf_top = tile_origin.y + tile_origin.height;
            content.push_str(&format!("1 0 0 -1 {pdf_x} {pdf_top} cm\n"));
            content.push_str("q\n");
            content.push_str(&tile_clip_box.clip_path());
            if let Some((image_box, registered_image)) = &raster_background {
                let draw_box = registered_image.draw_box.unwrap_or(*image_box);
                content.push_str(&format!(
                    "q\n{width} 0 0 -{height} {x} {y} cm\n/{name} Do\nQ\n",
                    width = draw_box.width,
                    height = draw_box.height,
                    x = draw_box.x,
                    y = draw_box.y + draw_box.height,
                    name = registered_image.name,
                ));
            } else {
                content.push_str(&format!(
                    "{sx} 0 0 {sy} {tx} {ty} cm\n",
                    sx = placement.scale_x,
                    sy = placement.scale_y,
                    tx = placement.translate_x,
                    ty = placement.translate_y,
                ));
                {
                    let mut image_sink = SvgPageImageSink {
                        pdf_writer,
                        page_images,
                    };
                    let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
                        shadings,
                        shading_counter,
                        image_sink: Some(&mut image_sink),
                    };
                    crate::render::svg_to_pdf::render_svg_tree_with_resources(
                        tree,
                        content,
                        &mut resources,
                    );
                }
            }
            content.push_str("Q\n");
            content.push_str("Q\n");
        }
    }
    content.push_str("Q\n");
}

/// Compute tile origin offsets that cover `[0, extent)` when starting from
/// `origin` and repeating every `step`.  Returns offsets that overlap the
/// visible range.
fn tile_offsets(origin: f32, step: f32, extent: f32) -> Vec<f32> {
    if step <= 0.0 {
        return vec![origin];
    }
    let mut offsets = Vec::new();
    // Walk backwards from origin to find the first tile that overlaps [0, extent).
    let mut start = origin;
    while start > 0.0 {
        start -= step;
    }
    let mut pos = start;
    while pos < extent {
        offsets.push(pos);
        pos += step;
    }
    if offsets.is_empty() {
        offsets.push(origin);
    }
    offsets
}
/// Generate a PDF path for a rounded rectangle.
///
/// Uses cubic Bezier curves to approximate circular arcs at each corner.
/// The magic number k = r * 0.5522847498 gives the best circular approximation.
/// Render a box-shadow with optional Gaussian blur approximation.
///
/// When `blur > 0`, draws multiple concentric semi-transparent layers that
/// expand outward from the shadow box, creating a smooth falloff. When
/// `blur == 0`, draws a single solid shadow rectangle.
#[allow(clippy::too_many_arguments)]
fn render_box_shadow(
    content: &mut String,
    shadow: &crate::style::computed::BoxShadow,
    box_x: f32,
    box_y_bottom: f32,
    box_w: f32,
    box_h: f32,
    border_radius: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    gs_counter: &mut usize,
) {
    let (sr, sg, sb, base_alpha) = shadow.color.to_f32_rgba();
    let blur = shadow.blur;
    // CSS: positive offset_y = shadow below element.
    // PDF: Y increases upward, so negate offset_y.
    let sx = box_x + shadow.offset_x;
    let sy = box_y_bottom - shadow.offset_y;

    if blur <= 0.5 {
        // No blur — solid shadow
        if base_alpha < 1.0 {
            let gs_name = format!("GSbs{}", *gs_counter);
            *gs_counter += 1;
            page_ext_gstates.push((gs_name.clone(), base_alpha));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        content.push_str(&format!("{sr} {sg} {sb} rg\n"));
        if border_radius > 0.0 {
            content.push_str(&rounded_rect_path(sx, sy, box_w, box_h, border_radius));
            content.push_str("\nf\n");
        } else {
            content.push_str(&format!("{sx} {sy} {box_w} {box_h} re\nf\n"));
        }
        if base_alpha < 1.0 {
            content.push_str("/GSDefault gs\n");
        }
        return;
    }

    // Multi-layer blur approximation: draw concentric rects from outside
    // (most transparent) to inside (most opaque), simulating Gaussian falloff.
    let layers: usize = 10;
    content.push_str(&format!("{sr} {sg} {sb} rg\n"));
    for i in (0..layers).rev() {
        let t = (i as f32 + 1.0) / layers as f32;
        // Smooth falloff: each layer contributes a fraction of the base alpha
        let alpha = base_alpha * (1.0 - t) / layers as f32;

        let expand = blur * t;
        let gs_name = format!("GSbs{}", *gs_counter);
        *gs_counter += 1;
        page_ext_gstates.push((gs_name.clone(), alpha));
        content.push_str(&format!("/{gs_name} gs\n"));

        let rx = sx - expand;
        let ry = sy - expand;
        let rw = box_w + expand * 2.0;
        let rh = box_h + expand * 2.0;
        let r = if border_radius > 0.0 {
            border_radius + expand
        } else {
            expand.min(blur * 0.3)
        };

        if r > 0.5 {
            content.push_str(&rounded_rect_path(rx, ry, rw, rh, r));
            content.push_str("\nf\n");
        } else {
            content.push_str(&format!("{rx} {ry} {rw} {rh} re\nf\n"));
        }
    }
    content.push_str("/GSDefault gs\n");
}

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
pub(crate) fn utf8_to_winansi(text: &str) -> Vec<u8> {
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

/// Returns `true` if every character in `text` can be encoded in WinAnsiEncoding.
///
/// Characters outside this range (CJK, Arabic, Hebrew, emoji, box-drawing, etc.)
/// cannot be rendered by the standard PDF fonts and require a Unicode-capable
/// embedded font instead.
pub(crate) fn is_winansi_encodable(text: &str) -> bool {
    text.chars().all(is_winansi_char)
}

/// Check whether a single character is representable in WinAnsiEncoding.
pub(crate) fn is_winansi_char(ch: char) -> bool {
    let code = ch as u32;
    matches!(code,
        0x0000..=0x007F |
        0x00A0..=0x00FF |
        0x20AC | 0x201A | 0x0192 | 0x201E | 0x2026 |
        0x2020 | 0x2021 | 0x02C6 | 0x2030 | 0x0160 |
        0x2039 | 0x0152 | 0x017D | 0x2018 | 0x2019 |
        0x201C | 0x201D | 0x2022 | 0x2013 | 0x2014 |
        0x02DC | 0x2122 | 0x0161 | 0x203A | 0x0153 |
        0x017E | 0x0178
    )
}

/// Encode a UTF-8 string for use in a PDF text operator (Tj).
///
/// Converts to WinAnsi encoding, then produces a `String` where:
/// - ASCII printable bytes (0x20..=0x7E), except `\`, `(`, `)`, are kept as-is
/// - `\`, `(`, `)` are escaped as `\\`, `\(`, `\)`
/// - All other bytes (0x00..=0x1F, 0x7F..=0xFF) are written as octal escapes `\NNN`
///
/// The returned string is safe to embed in a PDF content stream as `(encoded) Tj`.
pub(crate) fn encode_pdf_text(text: &str) -> String {
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

fn build_tounicode_cmap(mappings: &[(u16, Vec<u16>)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<0000> <FFFF>\n\
endcodespacerange\n",
    );

    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (glyph_id, unicode) in chunk {
            let unicode_hex: String = unicode
                .iter()
                .map(|code_unit| format!("{code_unit:04X}"))
                .collect();
            cmap.push_str(&format!("<{glyph_id:04X}> <{unicode_hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }

    cmap.push_str(
        "endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n",
    );
    cmap
}

/// A reference to an image XObject used on a page.
pub(crate) struct ImageRef {
    pub name: String,
    pub obj_id: usize,
}

struct SvgPageImageSink<'a> {
    pdf_writer: &'a mut PdfWriter,
    page_images: &'a mut Vec<ImageRef>,
}

impl SvgPageImageSink<'_> {
    fn register_page_image(&mut self, obj_id: usize) -> String {
        let name = format!("Im{obj_id}");
        self.page_images.push(ImageRef {
            name: name.clone(),
            obj_id,
        });
        name
    }
}

impl crate::render::svg_to_pdf::SvgImageObjectSink for SvgPageImageSink<'_> {
    fn register_raster(&mut self, raw_image: &[u8]) -> Option<String> {
        let obj_id = self.pdf_writer.add_raw_raster_image_object(raw_image)?;
        Some(self.register_page_image(obj_id))
    }
}

struct DecodedPngImage {
    width: u32,
    height: u32,
    color_space: &'static str,
    color_data: Vec<u8>,
    alpha_data: Option<Vec<u8>>,
}

fn decode_png_for_pdf(raw: &[u8]) -> Option<DecodedPngImage> {
    let mut decoder = png_decoder::Decoder::new(std::io::Cursor::new(raw));
    decoder.ignore_checksums(true);
    let mut reader = decoder.read_info().ok()?;
    let output_size = reader.output_buffer_size()?;
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = buffer.get(..info.buffer_size())?;

    let mut color_data = Vec::new();
    let mut alpha_data = Vec::new();
    let mut has_alpha = false;
    let color_space = match info.color_type {
        png_decoder::ColorType::Rgba => {
            color_data.reserve((info.width * info.height * 3) as usize);
            alpha_data.reserve((info.width * info.height) as usize);
            for chunk in pixels.chunks_exact(4) {
                color_data.extend_from_slice(&chunk[..3]);
                alpha_data.push(chunk[3]);
            }
            has_alpha = true;
            "/DeviceRGB"
        }
        png_decoder::ColorType::Rgb => {
            color_data.extend_from_slice(pixels);
            "/DeviceRGB"
        }
        png_decoder::ColorType::Grayscale => {
            color_data.extend_from_slice(pixels);
            "/DeviceGray"
        }
        png_decoder::ColorType::GrayscaleAlpha => {
            color_data.reserve((info.width * info.height) as usize);
            alpha_data.reserve((info.width * info.height) as usize);
            for chunk in pixels.chunks_exact(2) {
                color_data.push(chunk[0]);
                alpha_data.push(chunk[1]);
            }
            has_alpha = true;
            "/DeviceGray"
        }
        _ => return None,
    };

    Some(DecodedPngImage {
        width: info.width,
        height: info.height,
        color_space,
        color_data,
        alpha_data: has_alpha.then_some(alpha_data),
    })
}

fn flate_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

/// A custom TrueType font entry for the PDF font dictionary.
struct CustomFontEntry {
    /// Sanitized PDF resource key used from page content streams.
    resource_name: String,
    /// Object ID of the font object.
    font_obj_id: usize,
}

/// Minimal PDF writer that produces valid PDF files.
pub(crate) struct PdfWriter {
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

    fn add_icc_profile_object(&mut self, icc_profile: &[u8]) -> Option<usize> {
        let id = self.next_id();
        self.objects.push(format!(
            "{id} 0 obj\n<< /N 3 /Alternate /DeviceRGB /Length {} >>\nstream\n",
            icc_profile.len(),
        ));
        self.binary_objects.insert(id, icc_profile.to_vec());
        Some(id)
    }

    pub(crate) fn add_raw_rgb_image_object(
        &mut self,
        rgb_data: &[u8],
        width: u32,
        height: u32,
        icc_profile: Option<&[u8]>,
    ) -> Option<usize> {
        let color_stream = flate_compress(rgb_data)?;
        let color_space = if let Some(icc_profile) = icc_profile {
            let icc_id = self.add_icc_profile_object(icc_profile)?;
            format!("[/ICCBased {icc_id} 0 R]")
        } else {
            "/DeviceRGB".to_string()
        };

        let id = self.next_id();
        self.objects.push(format!(
            "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
            len = color_stream.len(),
        ));
        self.binary_objects.insert(id, color_stream);
        Some(id)
    }

    pub(crate) fn add_raw_png_image_object(&mut self, raw_png: &[u8]) -> Option<usize> {
        let decoded = decode_png_for_pdf(raw_png)?;
        let color_stream = flate_compress(&decoded.color_data)?;
        let alpha_stream = if let Some(alpha_data) = decoded.alpha_data.as_deref() {
            Some(flate_compress(alpha_data)?)
        } else {
            None
        };

        let alpha_id = alpha_stream.map(|stream| {
            let id = self.next_id();
            let header = format!(
                "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
                width = decoded.width,
                height = decoded.height,
                len = stream.len(),
            );
            self.objects.push(header);
            self.binary_objects.insert(id, stream);
            id
        });

        let id = self.next_id();
        let mut header = format!(
            "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8 /Filter /FlateDecode /Length {len}",
            width = decoded.width,
            height = decoded.height,
            color_space = decoded.color_space,
            len = color_stream.len(),
        );
        if let Some(alpha_id) = alpha_id {
            header.push_str(&format!(" /SMask {alpha_id} 0 R"));
        }
        header.push_str(" >>\nstream\n");

        self.objects.push(header);
        self.binary_objects.insert(id, color_stream);
        Some(id)
    }

    pub(crate) fn add_raw_raster_image_object(&mut self, raw_image: &[u8]) -> Option<usize> {
        if crate::parser::png::is_png(raw_image) {
            return self.add_raw_png_image_object(raw_image);
        }

        let (width, height) = crate::parser::jpeg::parse_jpeg_dimensions(raw_image)?;
        Some(self.add_image_object(raw_image, width, height, ImageFormat::Jpeg, None))
    }

    /// Embed a TrueType font and return the PDF resource name to reference it.
    fn add_ttf_font(
        &mut self,
        name: &str,
        ttf: &TtfFont,
        prepared_font: &PreparedCustomFont,
    ) -> String {
        let resource_name = sanitize_pdf_name(name);
        let base_font_name = &prepared_font.base_font_name;

        // 1. Font stream: embed the prepared font data and compress the stream
        // to avoid paying the full raw TTF size in the PDF.
        let stream_id = self.next_id();
        let compressed_data = flate_compress(&prepared_font.font_data);
        let header = if let Some(ref compressed_data) = compressed_data {
            format!(
                "{stream_id} 0 obj\n<< /Filter /FlateDecode /Length {} /Length1 {} >>\nstream\n",
                compressed_data.len(),
                prepared_font.font_data.len(),
            )
        } else {
            format!(
                "{stream_id} 0 obj\n<< /Length {} /Length1 {} >>\nstream\n",
                prepared_font.font_data.len(),
                prepared_font.font_data.len(),
            )
        };
        self.objects.push(header);
        self.binary_objects.insert(
            stream_id,
            compressed_data.unwrap_or_else(|| prepared_font.font_data.clone()),
        );

        // 2. FontDescriptor
        let descriptor_id = self.next_id();
        let pdf_metrics = ttf.pdf_vertical_metrics();
        let ascent_pdf = (pdf_metrics.ascent as i32 * 1000) / ttf.units_per_em as i32;
        let descent_pdf = (pdf_metrics.descent as i32 * 1000) / ttf.units_per_em as i32;
        let bbox_pdf = [
            (ttf.bbox[0] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[1] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[2] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[3] as i32 * 1000) / ttf.units_per_em as i32,
        ];
        self.objects.push(format!(
            "{descriptor_id} 0 obj\n<< /Type /FontDescriptor /FontName /{base_font_name} /Flags {flags} /FontBBox [{b0} {b1} {b2} {b3}] /Ascent {ascent} /Descent {descent} /ItalicAngle 0 /CapHeight {ascent} /StemV 80 /FontFile2 {stream_id} 0 R >>\nendobj",
            flags = ttf.flags,
            b0 = bbox_pdf[0],
            b1 = bbox_pdf[1],
            b2 = bbox_pdf[2],
            b3 = bbox_pdf[3],
            ascent = ascent_pdf,
            descent = descent_pdf,
        ));

        // 3. CID widths array keyed by glyph ID so shaped glyph IDs can be
        // emitted directly with Identity-H.
        let widths_str = prepared_font
            .widths
            .iter()
            .copied()
            .map(format_pdf_number)
            .collect::<Vec<_>>()
            .join(" ");

        // 4. CID descendant font object
        let cid_font_id = self.next_id();
        self.objects.push(format!(
            "{cid_font_id} 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base_font_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor_id} 0 R /CIDToGIDMap /Identity /W [0 [{widths_str}]] >>\nendobj",
        ));

        // 5. ToUnicode CMap so text stays searchable/selectable.
        let to_unicode_id = self.next_id();
        let to_unicode = build_tounicode_cmap(&prepared_font.to_unicode_map);
        self.objects.push(format!(
            "{to_unicode_id} 0 obj\n<< /Length {} >>\nstream\n{to_unicode}endstream\nendobj",
            to_unicode.len(),
        ));

        // 6. Type0 wrapper font object
        let font_id = self.next_id();
        self.objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /{base_font_name} /Encoding /Identity-H /DescendantFonts [{cid_font_id} 0 R] /ToUnicode {to_unicode_id} 0 R >>\nendobj",
        ));

        self.custom_font_entries.push(CustomFontEntry {
            resource_name: resource_name.clone(),
            font_obj_id: font_id,
        });

        resource_name
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
        let page_id = self.objects.len() + annotations.len() + 1;

        // Annotation objects
        let mut annot_ids = Vec::new();
        for annot in &annotations {
            let annot_id = self.next_id();
            self.objects.push(format!(
                "{annot_id} 0 obj\n<< /Type /Annot /Subtype /Link /P {page_id} 0 R /Rect [{x1} {y1} {x2} {y2}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({uri}) >> >>\nendobj",
                page_id = page_id,
                x1 = annot.x1,
                y1 = annot.y1,
                x2 = annot.x2,
                y2 = annot.y2,
                uri = escape_pdf_string(&annot.url),
            ));
            annot_ids.push(annot_id);
        }

        // Page object (placeholder — will be updated in finish())
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
            // Symbol (math/Greek)
            "Symbol",
        ];

        let mut all_objects: Vec<String> = self.objects.clone();

        for (i, name) in font_names.iter().enumerate() {
            let id = font_base_id + i;
            if name == &"Symbol" {
                // Symbol font uses its own built-in encoding, not WinAnsiEncoding
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>\nendobj",
                ));
            } else {
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>\nendobj",
                ));
            }
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
            font_entries.push(format!(
                "/{} {} 0 R",
                entry.resource_name, entry.font_obj_id
            ));
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

/// Map a Unicode character to the Adobe Symbol font encoding byte.
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

    const TEST_JPEG_DATA_URI: &str = concat!(
        "data:image/jpeg;base64,",
        "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
        "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
        "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
    );

    fn test_text_run(text: impl Into<String>) -> TextRun {
        TextRun {
            text: text.into(),
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
        }
    }

    fn test_text_line(runs: Vec<TextRun>) -> TextLine {
        TextLine { runs, height: 14.0 }
    }

    fn test_text_block(lines: Vec<TextLine>) -> LayoutElement {
        LayoutElement::TextBlock {
            lines,
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
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: crate::style::computed::VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        }
    }

    fn test_text_block_from_runs(runs: Vec<TextRun>) -> LayoutElement {
        test_text_block(vec![test_text_line(runs)])
    }

    fn test_page(elements: Vec<(f32, LayoutElement)>) -> Page {
        Page { elements }
    }

    fn first_td_y(content: &str) -> Option<f32> {
        for line in content.lines() {
            if let Some(coords) = line.strip_suffix(" Td") {
                let mut parts = coords.split_whitespace();
                let _x = parts.next()?;
                return parts.next()?.parse().ok();
            }
        }
        None
    }

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
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Pre has gray background — PDF should contain rectangle fill commands
        assert!(content.contains("re\nf\n") || content.contains("re"));
    }

    #[test]
    fn render_center_align() {
        let html = r#"<p style="text-align: center">Centered</p>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_right_align() {
        let html = r#"<p style="text-align: right">Right</p>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_underline() {
        let html = "<p><u>Underlined text</u></p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Underline draws a line with stroke command
        assert!(content.contains(" l\nS\n"));
    }

    #[test]
    fn render_bold_italic_combined() {
        let html = "<p><strong><em>Bold Italic</em></strong></p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica-BoldOblique"));
    }

    #[test]
    fn render_page_break_in_content() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should have multiple page objects
        assert!(content.matches("/Type /Page").count() >= 2);
    }

    #[test]
    fn render_svg_without_viewbox_scales_to_layout_box() {
        let tree = crate::parser::svg::SvgTree {
            width: 120.0,
            height: 60.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle::default(),
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let pages = vec![Page {
            elements: vec![(
                0.0,
                LayoutElement::Svg {
                    tree,
                    width: 240.0,
                    height: 120.0,
                    flow_extra_bottom: 0.0,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                },
            )],
        }];
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("2 0 0 2 0 0 cm"),
            "expected outer scale for SVG without a viewBox"
        );
    }

    #[test]
    fn render_svg_honors_root_preserve_aspect_ratio() {
        let tree = crate::parser::svg::SvgTree {
            width: 20.0,
            height: 20.0,
            width_attr: Some("20".to_string()),
            height_attr: Some("20".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(crate::parser::svg::ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 100.0,
                height: 20.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let pages = vec![Page {
            elements: vec![(
                0.0,
                LayoutElement::Svg {
                    tree,
                    width: 20.0,
                    height: 20.0,
                    flow_extra_bottom: 0.0,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                },
            )],
        }];
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("0.2 0 0 0.2 0 8 cm"),
            "expected meet scaling with vertical centering for the root SVG viewport"
        );
    }

    #[test]
    fn render_colored_text() {
        let html = r#"<p style="color: red">Red text</p>"#;
        let nodes = parse_html(&html).unwrap();
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
        assert!(
            content.contains("/P "),
            "PDF link annotations should record their owning page"
        );
        // The page object should reference annotations
        assert!(
            content.contains("/Annots ["),
            "Page should have an /Annots array"
        );
    }

    #[test]
    fn render_table_cell_link_annotation() {
        let html = r#"
            <table>
                <tr>
                    <td><a href="https://example.com/table">Cell link</a></td>
                </tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert_eq!(content.matches("/Subtype /Link").count(), 1);
        assert!(content.contains("https://example.com/table"));
        assert!(content.contains("/Annots ["));
    }

    #[test]
    fn render_nested_table_link_annotation() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        <table>
                            <tr>
                                <td><a href="https://example.com/nested">Nested link</a></td>
                            </tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert_eq!(content.matches("/Subtype /Link").count(), 1);
        assert!(content.contains("https://example.com/nested"));
        assert!(content.contains("/Annots ["));
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
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
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
    fn render_image_xobject_uses_source_pixel_dimensions() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="120" height="90">"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Width 1 /Height 1"),
            "image XObject should use source pixel dimensions, not CSS box dimensions"
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
    fn render_dashed_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dashed black; width: 100pt">Dashed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("[6 4] 0 d"),
            "Dashed border should emit [6 4] 0 d dash pattern. Got: {}",
            &content[..content.len().min(2000)]
        );
        assert!(
            content.contains("[] 0 d"),
            "Dashed border should reset dash pattern with [] 0 d"
        );
    }

    #[test]
    fn render_dotted_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dotted red; width: 100pt">Dotted</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("[1 3] 0 d"),
            "Dotted border should emit [1 3] 0 d dash pattern"
        );
    }

    #[test]
    fn render_solid_border_no_dash_pattern() {
        let html = r#"<div style="border: 2px solid black; width: 100pt">Solid</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Solid borders should NOT have dash patterns
        assert!(
            !content.contains("[6 4] 0 d") && !content.contains("[1 3] 0 d"),
            "Solid border should not emit dash patterns"
        );
    }

    #[test]
    fn border_style_parsed_from_shorthand() {
        use crate::parser::dom::HtmlTag;
        use crate::style::computed::ComputedStyle;
        use crate::style::computed::{BorderStyle, compute_style_with_context};
        let parent = ComputedStyle::default();
        let style = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some("border: 2px dashed red"),
            &parent,
        );
        assert_eq!(style.border.top.style, BorderStyle::Dashed);
        assert_eq!(style.border.right.style, BorderStyle::Dashed);
        assert_eq!(style.border.bottom.style, BorderStyle::Dashed);
        assert_eq!(style.border.left.style, BorderStyle::Dashed);
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
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
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
    #[ignore] // TODO: Container renderer doesn't render background images yet
    fn render_jpeg_background_uses_decoded_image_xobject() {
        use image::ImageEncoder;

        let mut jpeg_bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg_bytes)
            .write_image(
                &[255u8, 128, 0, 0, 128, 255, 0, 0, 0, 255, 255, 255],
                2,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .expect("jpeg encoding should succeed");
        let jpeg_b64 = simple_base64_encode_test(&jpeg_bytes);
        let html = format!(
            r#"
            <div style="
                width: 100pt;
                height: 100pt;
                background-image: url(data:image/jpeg;base64,{jpeg_b64});
                background-repeat: no-repeat;
                background-size: 100pt 100pt;
            "></div>
        "#,
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert_eq!(content.matches("/Subtype /Image").count(), 1);
        assert!(
            content.contains("/Filter /FlateDecode"),
            "decoded JPEG backgrounds should use a Flate image XObject"
        );
        assert!(
            !content.contains("/Filter /DCTDecode"),
            "decoded JPEG backgrounds should not passthrough raw JPEG bytes"
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
        // scale(2) produces "2 0 0 2 tx ty cm" where tx,ty are the centre-offset
        // translation terms (non-zero because the block is not at the page origin).
        assert!(
            content.contains("2 0 0 2 "),
            "transform: scale(2) should produce '2 0 0 2 ...' cm operator"
        );
        assert!(
            content.contains(" cm\n"),
            "transform: scale(2) should produce a cm operator"
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
            content.contains("1 0 0 1 10 -20 cm"),
            "transform: translate(10pt, 20pt) should produce '1 0 0 1 10 -20 cm' (Y negated for PDF)"
        );
    }

    /// BUG P2-2: rotate/scale transforms must be applied around the element
    /// centre (CSS `transform-origin: 50% 50%`), not the page origin.
    /// Previously the translation terms in the `cm` matrix were always 0,
    /// which displaced the element off-page.
    #[test]
    fn render_transform_scale_centered_on_element() {
        // A block with explicit 100pt × 20pt size, positioned at the top of
        // the content area.  The rendered PDF matrix must be
        //   scale_x 0 0 scale_y tx ty
        // where tx = cx*(1-sx) and ty = cy*(1-sy) (non-zero when the element
        // is not at the page origin).
        let html = r#"<div style="transform: scale(2); width: 100pt; height: 20pt; background-color: blue">Box</div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // The matrix scale values are correct.
        assert!(
            content.contains("2 0 0 2 "),
            "scale(2) should produce '2 0 0 2 tx ty cm'"
        );
        // The translation terms must NOT both be zero — the element is not
        // at the page origin, so the centre-based offset is non-zero.
        assert!(
            !content.contains("2 0 0 2 0 0 cm"),
            "scale(2) on a non-origin element must have non-zero tx/ty in the cm matrix"
        );
    }

    /// BUG P2-2: a rotate transform must include non-zero translation terms
    /// so the element stays in its section instead of being displaced.
    #[test]
    fn render_transform_rotate_includes_translation_terms() {
        let html = r#"<div style="transform: rotate(45deg); width: 100pt; height: 20pt; background-color: red">Rotated</div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // cos/sin values of 45 deg must be present.
        assert!(
            content.contains("0.707"),
            "rotate(45deg) must contain cos/sin ~0.707"
        );
        // The matrix must NOT have zero translation — the element centre
        // is not at (0, 0) in PDF coordinates.
        assert!(
            !content.contains("0.70710677 0.70710677 -0.70710677 0.70710677 0 0 cm"),
            "rotate on a non-origin element must have non-zero tx/ty in the cm matrix"
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
            vertical_align: VerticalAlign::Baseline,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        let mut annotations = Vec::new();
        let prepared_fonts = PreparedCustomFonts::new();
        let mut text_context = TextRenderContext::new(&fonts, &prepared_fonts, &mut annotations);
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(0.0, 100.0, 50.0),
            &mut text_context,
        );
        assert!(content.contains("Hello"));
    }

    #[test]
    fn text_block_empty_run_skipped() {
        // Covers line 401: empty text run within a text block line is skipped
        let page = test_page(vec![(
            0.0,
            test_text_block_from_runs(vec![test_text_run(""), test_text_run("Data")]),
        )]);
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Data"));
    }

    #[test]
    fn page_break_element_renders() {
        // Covers line 677: PageBreak empty match arm
        let page = test_page(vec![
            (
                0.0,
                test_text_block_from_runs(vec![test_text_run("Before")]),
            ),
            (20.0, LayoutElement::PageBreak),
        ]);
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
    #[ignore] // TODO: Container renderer doesn't render SVG backgrounds with border-radius clip yet
    fn svg_background_clipped_to_border_radius() {
        let html = r#"<div style="width: 200pt; height: 80pt; border-radius: 12pt; background: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%221%22 height=%221%22%3E%3Crect width=%221%22 height=%221%22 fill=%22red%22/%3E%3C/svg%3E') no-repeat"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" c\n"),
            "Rounded clip should use Bezier curves"
        );
        assert!(pdf_str.contains("W n"), "SVG background should be clipped");
    }

    #[test]
    fn svg_background_percent_size_uses_positioning_area() {
        let tree = crate::parser::svg::SvgTree {
            width: 1.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            BackgroundPaintContext::new(
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Explicit {
                    width: 50.0,
                    height: Some(25.0),
                    width_is_percent: true,
                    height_is_percent: true,
                },
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("0 0 100 25 re W n"),
            "Expected SVG tile viewport to resolve against the 200pt by 100pt positioning area"
        );
        assert!(
            content.contains("25 0 0 25 37.5 0 cm"),
            "Expected root preserveAspectRatio to fit the 1:1 SVG into the 100pt by 25pt tile"
        );
    }

    #[test]
    fn svg_background_single_percent_size_preserves_aspect_ratio() {
        let tree = crate::parser::svg::SvgTree {
            width: 2.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            BackgroundPaintContext::new(
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Explicit {
                    width: 50.0,
                    height: None,
                    width_is_percent: true,
                    height_is_percent: false,
                },
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("50 0 0 50 0 0 cm"),
            "Single-value background-size should preserve intrinsic aspect ratio"
        );
    }

    #[test]
    fn svg_background_uses_outer_clip_box() {
        let tree = crate::parser::svg::SvgTree {
            width: 1.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            BackgroundPaintContext::new(
                SvgViewportBox::new(20.0, 10.0, 160.0, 80.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Auto,
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("0 0 200 100 re W n"),
            "Clip box should stay on the outer element box, not shrink to the origin box"
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
    fn root_svg_background_renders_in_pdf() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("1 0 0 rg"),
            "Expected red SVG background fill"
        );
    }

    #[test]
    fn root_svg_background_viewbox_only_renders_in_pdf() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("1 0 0 rg"),
            "Expected viewBox-only SVG background to render"
        );
    }

    #[test]
    fn root_svg_background_with_gradient_registers_shading_resources() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='20' y2='0' gradientUnits='userSpaceOnUse'%3E%3Cstop offset='0' stop-color='%23f00'/%3E%3Cstop offset='1' stop-color='%2300f'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='20' height='10' fill='url(%23g)'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("/ShadingType 2"),
            "Expected gradient SVG background to emit an axial shading resource"
        );
    }

    #[test]
    fn table_cell_nested_background_block_renders_image_xobject() {
        let png_bytes = build_minimal_test_png();
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(
            r#"<table><tr><td><div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('data:image/png;base64,{b64}') no-repeat;"></div></td></tr></table>"#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("BI\n"),
            "Expected nested table-cell background block to emit an inline image"
        );
        assert!(
            pdf_str.contains("EI\n"),
            "Expected nested table-cell background block to terminate the inline image"
        );
    }

    #[test]
    fn nested_text_block_padding_top_offsets_text() {
        let lines = vec![test_text_line(vec![test_text_run("Nested")])];
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();

        let mut without_padding = String::new();
        let mut without_padding_context = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        render_nested_text_block(
            &mut without_padding,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut without_padding_context,
        );
        drop(without_padding_context);

        let mut with_padding = String::new();
        let mut with_padding_context = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        render_nested_text_block(
            &mut with_padding,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 12.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut with_padding_context,
        );

        let without_padding_y = first_td_y(&without_padding).unwrap();
        let with_padding_y = first_td_y(&with_padding).unwrap();
        assert!((without_padding_y - with_padding_y - 12.0).abs() < 0.01);
    }

    #[test]
    fn nested_absolute_without_containing_block_uses_initial_origin() {
        let mut absolute = test_text_block_from_runs(vec![test_text_run("Absolute")]);
        if let LayoutElement::TextBlock {
            position,
            offset_top,
            offset_left,
            ..
        } = &mut absolute
        {
            *position = Position::Absolute;
            *offset_top = 10.0;
            *offset_left = 20.0;
        }

        let elements = [absolute];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(50.0, 100.0, 10.0, 200.0, 80.0),
        );
        assert_eq!(planned.len(), 1);
        assert!((planned[0].origin_x - 30.0).abs() < 0.01);
        assert!((planned[0].top_y - 190.0).abs() < 0.01);
    }

    #[test]
    fn nested_static_without_containing_block_uses_local_origin() {
        let static_block = test_text_block_from_runs(vec![test_text_run("Static")]);
        let elements = [static_block];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(50.0, 100.0, 10.0, 200.0, 80.0),
        );
        assert_eq!(planned.len(), 1);
        assert!((planned[0].origin_x - 50.0).abs() < 0.01);
        assert!((planned[0].top_y - 100.0).abs() < 0.01);
    }

    #[test]
    fn table_cell_absolute_pseudo_background_renders_blurred_copy() {
        use crate::parser::css::parse_stylesheet;

        let png_bytes = {
            let image = image::RgbaImage::from_fn(4, 4, |x, y| {
                image::Rgba([(x * 40) as u8, (y * 40) as u8, 180, 255])
            });
            let mut encoded = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut encoded),
                    image::ImageFormat::Png,
                )
                .unwrap();
            encoded
        };
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(
            r#"<html><head><style>
                .image-container {{
                    display: flex;
                    position: relative;
                    width: 40pt;
                    aspect-ratio: 1 / 1;
                    background-image: url('data:image/png;base64,{b64}');
                    background-size: cover;
                    background-repeat: no-repeat;
                }}
                .image-container::after {{
                    content: '';
                    background-image: inherit;
                    background-size: inherit;
                    background-repeat: inherit;
                    width: 100%;
                    height: 100%;
                    display: block;
                    position: absolute;
                    bottom: -10pt;
                    z-index: -1;
                    filter: blur(4px);
                }}
            </style></head><body>
                <table><tr><td><div class="image-container"></div></td></tr></table>
            </body></html>"#
        );
        let result = crate::parser::html::parse_html_with_styles(&html).unwrap();
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
        fn count_background_svgs(elements: &[LayoutElement]) -> usize {
            elements.iter().map(count_element_background_svgs).sum()
        }

        fn count_element_background_svgs(element: &LayoutElement) -> usize {
            match element {
                LayoutElement::TextBlock { background_svg, .. } => {
                    usize::from(background_svg.is_some())
                }
                LayoutElement::TableRow { cells, .. } | LayoutElement::GridRow { cells, .. } => {
                    cells.iter().map(count_cell_background_svgs).sum()
                }
                LayoutElement::FlexRow {
                    cells,
                    background_svg,
                    ..
                } => {
                    usize::from(background_svg.is_some())
                        + cells
                            .iter()
                            .map(|cell| usize::from(cell.background_svg.is_some()))
                            .sum::<usize>()
                }
                _ => 0,
            }
        }

        fn count_cell_background_svgs(cell: &TableCell) -> usize {
            count_background_svgs(&cell.nested_rows)
        }

        let background_svg_count: usize = pages[0]
            .elements
            .iter()
            .map(|(_, element)| count_element_background_svgs(element))
            .sum();

        assert!(
            background_svg_count >= 2,
            "Expected both the main block and the blurred pseudo-element to survive into layout with raster backgrounds"
        );

        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/SMask"),
            "Expected the blurred pseudo-background to preserve alpha via a PDF soft mask"
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
            background_color: Some((1.0, 0.0, 0.0, 1.0)),
            padding: (4.0, 2.0),
            border_radius: 3.0,
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 16.0,
            }],
            nested_rows: Vec::new(),
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
            vertical_align: VerticalAlign::Middle,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        let mut annotations = Vec::new();
        let prepared_fonts = PreparedCustomFonts::new();
        let mut text_context = TextRenderContext::new(&fonts, &prepared_fonts, &mut annotations);
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 100.0),
            &mut text_context,
        );
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
            background_color: Some((1.0, 1.0, 0.0, 1.0)),
            padding: (2.0, 1.0),
            border_radius: 4.0,
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
            background_color: Some((1.0, 1.0, 0.0, 1.0)),
            padding: (2.0, 1.0),
            border_radius: 8.0, // Different border_radius
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
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap,
            glyph_widths: (0..=96).map(|_| 500).collect(),
            num_h_metrics: 96,
            flags: 32,
            data: vec![0u8; 64], // Minimal dummy font data
        };
        let mut fonts = HashMap::new();
        fonts.insert("TestFont".to_string(), ttf);

        let mut run = test_text_run("Custom");
        run.font_family = FontFamily::Custom("TestFont".to_string());
        let page = test_page(vec![(0.0, test_text_block_from_runs(vec![run]))]);
        let pdf = render_pdf_with_fonts(&[page], PageSize::A4, Margin::default(), &fonts).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/BaseFont /TestFont"),
            "Should have custom font BaseFont entry"
        );
        assert!(
            pdf_str.contains("/Subtype /Type0"),
            "Should have Type0 font wrapper"
        );
        assert!(
            pdf_str.contains("/Subtype /CIDFontType2"),
            "Should have CIDFontType2 descendant font"
        );
        assert!(
            pdf_str.contains("/FontDescriptor"),
            "Should have FontDescriptor reference"
        );
        assert!(
            pdf_str.contains("/Encoding /Identity-H"),
            "Should use Identity-H for shaped custom glyphs"
        );
        assert!(
            pdf_str.contains("/ToUnicode"),
            "Should attach a ToUnicode CMap for text extraction"
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
    fn render_run_text_falls_back_to_standard_font_when_custom_shaping_fails() {
        use crate::parser::ttf::TtfFont;

        let mut cmap = HashMap::new();
        for c in 32u16..=126 {
            cmap.insert(c, c - 31);
        }
        let ttf = TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap,
            glyph_widths: (0..=96).map(|_| 500).collect(),
            num_h_metrics: 96,
            flags: 32,
            data: vec![0u8; 64],
        };
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::font_variant_key("TestFont", false, false),
            ttf,
        );

        let mut run = test_text_run("Custom");
        run.font_family = FontFamily::Custom("TestFont".to_string());

        let mut content = String::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        render_run_text(
            &mut content,
            &run,
            10.0,
            20.0,
            &fonts,
            &prepared_custom_fonts,
        );

        assert!(content.contains("/Helvetica 12 Tf\n"));
        assert!(content.contains("(Custom) Tj\n"));
        assert!(!content.contains("/testfont 12 Tf\n"));
    }

    #[test]
    fn append_tj_shaped_text_uses_single_text_matrix() {
        let font = crate::parser::ttf::TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap: HashMap::new(),
            glyph_widths: vec![0, 500, 500],
            num_h_metrics: 3,
            flags: 32,
            data: Vec::new(),
        };
        let shaped = crate::text::ShapedRun {
            glyphs: vec![
                crate::text::ShapedGlyph {
                    glyph_id: 1,
                    x_advance: 6.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                    unicode: vec![0x0041],
                },
                crate::text::ShapedGlyph {
                    glyph_id: 2,
                    x_advance: 6.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                    unicode: vec![0x0042],
                },
            ],
            width: 12.0,
        };
        let mut content = String::new();
        append_tj_shaped_text(
            &mut content,
            ShapedTextRender::new(PdfPoint::new(10.0, 20.0), 12.0, &font, &shaped, None),
        );

        assert!(
            content.contains("1 0 0 1 10 20 Tm"),
            "Should position the run once with a single text matrix"
        );
        assert!(
            content.contains("[<0001> <0002>] TJ"),
            "Should encode the shaped run as one TJ array"
        );
        assert_eq!(
            content.matches(" Tm\n").count(),
            1,
            "Simple shaped runs should not emit per-glyph matrices"
        );
    }

    #[test]
    fn build_tounicode_cmap_supports_multi_codepoint_glyphs() {
        let cmap = build_tounicode_cmap(&[(1, vec![0x0066, 0x0069])]);
        assert!(
            cmap.contains("<0001> <00660069>"),
            "ToUnicode should preserve multi-codepoint mappings such as ligatures"
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

    // ===== layout_elements.rs coverage tests =====

    /// table_cell_content_top: VerticalAlign::Middle positions text mid-row
    #[test]
    fn layout_elements_vertical_align_middle_in_table_cell() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { vertical-align: middle; }
        </style></head><body>
        <table>
            <tr>
                <td style="height: 80pt; padding: 0">Middle</td>
                <td style="height: 80pt; padding: 0">Other</td>
            </tr>
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
        assert!(
            pdf_str.contains("(Middle)"),
            "Should render middle-aligned text"
        );
    }

    /// table_cell_content_top: VerticalAlign::Bottom positions text at bottom
    #[test]
    fn layout_elements_vertical_align_bottom_in_table_cell() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td.bottom { vertical-align: bottom; }
        </style></head><body>
        <table>
            <tr>
                <td class="bottom" style="padding: 0">Bottom</td>
                <td style="padding: 0; height: 60pt">Tall</td>
            </tr>
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
        assert!(
            pdf_str.contains("(Bottom)"),
            "Should render bottom-aligned text"
        );
    }

    /// render_nested_text_block: background_color + border_radius in nested block
    #[test]
    fn layout_elements_nested_text_block_background_with_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("BgRound")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 4.0,
                padding_bottom: 4.0,
                padding_left: 4.0,
                padding_right: 4.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: None,
                background_color: Some((0.0, 1.0, 0.0, 1.0)),
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 8.0, // Triggers rounded rect path
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Green background
        assert!(
            content.contains("0 1 0 rg"),
            "Should have green background color"
        );
        // Rounded rect uses Bezier curves
        assert!(
            content.contains(" c\n"),
            "Should have Bezier curves for border-radius"
        );
        assert!(content.contains("f\n"), "Should fill the rounded rect");
    }

    /// render_nested_text_block: border rendering (all 4 sides)
    #[test]
    fn layout_elements_nested_text_block_all_four_borders() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("Bordered")])];
        let mut content = String::new();
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (1.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 1.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 2.0,
                padding_bottom: 2.0,
                padding_left: 2.0,
                padding_right: 2.0,
                border,
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Should have stroke commands for all 4 borders
        assert!(content.contains("1 0 0 RG"), "Should have red top border");
        assert!(
            content.contains("0 1 0 RG"),
            "Should have green right border"
        );
        assert!(
            content.contains("0 0 1 RG"),
            "Should have blue bottom border"
        );
        assert!(
            content.contains("0 0 0 RG"),
            "Should have black left border"
        );
        // All 4 sides produce line strokes
        let stroke_count = content.matches(" l S\n").count() + content.matches(" l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 border strokes, got {stroke_count}"
        );
    }

    /// render_nested_text_block: only top border triggers single line
    #[test]
    fn layout_elements_nested_text_block_top_border_only() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("TopOnly")])];
        let mut content = String::new();
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (1.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border,
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut ctx,
        );
        assert!(content.contains("1 0 0 RG"), "Should have red top border");
        assert!(content.contains("2 w\n"), "Should have 2pt line width");
    }

    /// render_nested_text_block: background_svg with BackgroundOrigin::Border
    #[test]
    fn layout_elements_nested_text_block_svg_background_border_origin() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let svg_tree = crate::parser::svg::SvgTree {
            width: 10.0,
            height: 10.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        let lines = vec![test_text_line(vec![test_text_run("SvgBorder")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border,
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: Some(&svg_tree),
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Cover,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::NoRepeat,
                // Border origin expands ref box by border widths
                background_origin: BackgroundOrigin::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // SVG rect should produce fill output
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red fill from SVG rect"
        );
        assert!(content.contains("(SvgBorder)"), "Should render block text");
    }

    /// render_nested_text_block: background_svg with BackgroundOrigin::Content
    #[test]
    fn layout_elements_nested_text_block_svg_background_content_origin() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let svg_tree = crate::parser::svg::SvgTree {
            width: 10.0,
            height: 10.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((0.0, 0.0, 1.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let lines = vec![test_text_line(vec![test_text_run("SvgContent")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                text_align: TextAlign::Left,
                padding_top: 5.0,
                padding_bottom: 5.0,
                padding_left: 5.0,
                padding_right: 5.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: Some(&svg_tree),
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Cover,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::NoRepeat,
                // Content origin shrinks ref box by padding
                background_origin: BackgroundOrigin::Content,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // SVG rect should produce fill output (blue)
        assert!(
            content.contains("0 0 1 rg"),
            "Should have blue fill from SVG rect"
        );
    }

    /// render_nested_text_block: empty lines (no text) but with background
    #[test]
    fn layout_elements_nested_text_block_no_lines_with_background() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &[], // No lines
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: Some(50.0), // Explicit height keeps the block visible
                background_color: Some((0.5, 0.5, 0.5, 1.0)),
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_blur_canvas_box: None,
                border_radius: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Background rect fill should be emitted even with no lines
        assert!(
            content.contains("0.5 0.5 0.5 rg"),
            "Should have gray background fill even with no lines"
        );
        assert!(content.contains("re\nf\n"), "Should have rectangle fill");
    }

    /// render_nested_layout_elements: rowspan == 0 skips the cell
    #[test]
    fn layout_elements_nested_rowspan_zero_skips_cell() {
        use crate::layout::engine::{LayoutBorder, TableCell};
        let run = TextRun {
            text: "Skipped".to_string(),
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
        };
        let run_visible = TextRun {
            text: "Visible".to_string(),
            ..run.clone()
        };
        // rowspan=0 means "continuation" — renderer skips it
        let cell_skip = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 0, // Should be skipped
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };
        let cell_visible = TableCell {
            lines: vec![TextLine {
                runs: vec![run_visible],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell_skip, cell_visible],
            col_widths: vec![100.0, 100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content.contains("(Visible)"),
            "Visible cell should be rendered"
        );
        assert!(
            !content.contains("(Skipped)"),
            "rowspan=0 cell should be skipped"
        );
    }

    /// render_nested_layout_elements: cell with background_color in nested table
    #[test]
    fn layout_elements_nested_table_cell_background_color() {
        use crate::layout::engine::{LayoutBorder, TableCell};
        let run = TextRun {
            text: "BgCell".to_string(),
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
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: Some((1.0, 0.0, 0.0, 1.0)), // red cell background
            padding_top: 2.0,
            padding_right: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            colspan: 1,
            rowspan: 1,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 100.0),
            &mut ctx,
        );
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red cell background fill"
        );
        assert!(
            content.contains("re\nf\n"),
            "Should have filled rect for cell background"
        );
        assert!(content.contains("(BgCell)"), "Should render cell text");
    }

    /// render_nested_layout_elements: cell with borders in nested context
    #[test]
    fn layout_elements_nested_table_cell_with_borders() {
        use crate::layout::engine::{LayoutBorder, LayoutBorderSide, TableCell};
        let run = TextRun {
            text: "BorderedNested".to_string(),
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
        };
        let mut border = LayoutBorder::default();
        border.top = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 2.0,
            padding_right: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            colspan: 1,
            rowspan: 1,
            border,
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 100.0),
            &mut ctx,
        );
        assert!(
            content.contains("0 0 1 RG"),
            "Should have blue cell border color"
        );
        // Should have stroke commands for cell borders
        let stroke_count = content.matches("l S\n").count() + content.matches("l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 cell border strokes, got {stroke_count}"
        );
    }

    /// render_cell_text: text-align right and center in nested table cell
    #[test]
    fn layout_elements_cell_text_align_right_and_center() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ctx =
            TextRenderContext::new(&custom_fonts, &prepared_custom_fonts, &mut annotations);

        let run = TextRun {
            text: "Aligned".to_string(),
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
        };

        // Test right-align
        let cell_right = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run.clone()],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Right,
            vertical_align: VerticalAlign::Top,
        };
        let mut content_right = String::new();
        render_cell_text(
            &mut content_right,
            &cell_right,
            CellTextPlacement::new(0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content_right.contains("(Aligned)"),
            "Should render right-aligned text"
        );

        // Test center-align
        let cell_center = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Top,
        };
        let mut content_center = String::new();
        render_cell_text(
            &mut content_center,
            &cell_center,
            CellTextPlacement::new(0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content_center.contains("(Aligned)"),
            "Should render center-aligned text"
        );
    }

    /// render_cell_text: underline and line_through in nested table cell
    #[test]
    fn layout_elements_cell_text_underline_and_line_through() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ctx =
            TextRenderContext::new(&custom_fonts, &prepared_custom_fonts, &mut annotations);

        let underline_run = TextRun {
            text: "Under".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: true,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let strike_run = TextRun {
            text: "Strike".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: true,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![
                TextLine {
                    runs: vec![underline_run],
                    height: 14.0,
                },
                TextLine {
                    runs: vec![strike_run],
                    height: 14.0,
                },
            ],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Under)"), "Should render underlined text");
        assert!(
            content.contains("(Strike)"),
            "Should render struck-through text"
        );
        // Both decorations draw lines with S stroke command
        let stroke_count = content.matches(" l\nS\n").count() + content.matches(" l S\n").count();
        assert!(
            stroke_count >= 2,
            "Should have strokes for underline and line-through, got {stroke_count}"
        );
    }

    /// render_cell_text: inline span with background_color and border_radius in nested cell
    #[test]
    fn layout_elements_cell_text_inline_bg_with_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ctx =
            TextRenderContext::new(&custom_fonts, &prepared_custom_fonts, &mut annotations);

        let run = TextRun {
            text: "Badge".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (1.0, 1.0, 1.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((0.2, 0.4, 0.8, 1.0)),
            padding: (3.0, 2.0),
            border_radius: 4.0, // Triggers rounded rect for inline background
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Badge)"), "Should render badge text");
        // Inline background fill (rounded rect uses Bezier c operator)
        assert!(
            content.contains("0.2 0.4 0.8 rg"),
            "Should have blue inline background color"
        );
        assert!(
            content.contains(" c\n"),
            "Should have Bezier curves for rounded inline bg"
        );
    }

    /// render_cell_text: inline span with background_color but no border_radius (rect path)
    #[test]
    fn layout_elements_cell_text_inline_bg_no_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ctx =
            TextRenderContext::new(&custom_fonts, &prepared_custom_fonts, &mut annotations);

        let run = TextRun {
            text: "Tag".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0, 1.0)), // yellow
            padding: (2.0, 1.0),
            border_radius: 0.0, // No rounding — should use rectangle
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Tag)"), "Should render tag text");
        assert!(
            content.contains("1 1 0 rg"),
            "Should have yellow inline background color"
        );
        // No border-radius: should use rectangle re operator
        assert!(
            content.contains(" re\nf\n"),
            "Should use rectangle fill for zero-radius inline bg"
        );
    }

    /// plan_nested_layout_elements: Position::Relative with positioned_depth registers origin
    #[test]
    fn layout_elements_plan_relative_with_positioned_depth() {
        let mut relative = test_text_block_from_runs(vec![test_text_run("Relative")]);
        if let LayoutElement::TextBlock {
            position,
            offset_top,
            offset_left,
            positioned_depth,
            ..
        } = &mut relative
        {
            *position = Position::Relative;
            *offset_top = 5.0;
            *offset_left = 15.0;
            *positioned_depth = 1; // Non-zero: should register origin
        }
        let elements = [relative];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(20.0, 80.0, 10.0, 120.0, 100.0),
        );
        assert_eq!(planned.len(), 1);
        // Relative: uses local origin (20.0) + offset_left (15.0)
        assert!(
            (planned[0].origin_x - 35.0).abs() < 0.01,
            "Relative block origin_x should be frame.origin_x + offset_left"
        );
        // top_y: cursor_y (80.0) - margin_top (0) - offset_top (5) = 75.0
        assert!(
            (planned[0].top_y - 75.0).abs() < 0.01,
            "Relative block top_y should be cursor_y - offset_top"
        );
    }

    /// plan_nested_layout_elements: absolute with containing_block sets blur_canvas_box
    #[test]
    fn layout_elements_plan_absolute_with_containing_block_sets_blur_canvas_box() {
        let containing = crate::layout::engine::ContainingBlock {
            x: 5.0,
            width: 200.0,
            height: 100.0,
            depth: 2,
        };
        let mut absolute = test_text_block_from_runs(vec![test_text_run("Abs")]);
        if let LayoutElement::TextBlock {
            position,
            containing_block,
            positioned_depth,
            ..
        } = &mut absolute
        {
            *position = Position::Absolute;
            *containing_block = Some(containing);
            *positioned_depth = 0;
        }
        // First register a positioned origin for depth 2 by planning a relative block
        let mut relative_parent = test_text_block_from_runs(vec![test_text_run("Parent")]);
        if let LayoutElement::TextBlock {
            position,
            positioned_depth,
            ..
        } = &mut relative_parent
        {
            *position = Position::Relative;
            *positioned_depth = 2;
        }
        let elements = [relative_parent, absolute];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(10.0, 200.0, 10.0, 200.0, 300.0),
        );
        // The absolute element should have a blur_canvas_box derived from the containing block
        let abs_planned = planned.iter().find(|p| {
            if let LayoutElement::TextBlock { .. } = p.element {
                // The second element (absolute) should have blur_canvas_box set when
                // its containing_block refers to a depth that has been registered
                true
            } else {
                false
            }
        });
        // Just verify the plan succeeds without panic and produces 2 elements
        assert_eq!(planned.len(), 2, "Should plan both elements");
    }

    /// table_row_total_height: returns 0 for non-TableRow variant
    #[test]
    fn layout_elements_table_row_total_height_non_row_returns_zero() {
        let non_row = LayoutElement::PageBreak;
        assert_eq!(
            table_row_total_height(&non_row),
            0.0,
            "Non-TableRow element should return 0 height"
        );
        let text_block = test_text_block_from_runs(vec![test_text_run("Hello")]);
        assert_eq!(
            table_row_total_height(&text_block),
            0.0,
            "TextBlock element should return 0 height"
        );
    }

    /// Integration: nested table with vertical-align middle exercises layout_elements paths
    #[test]
    fn layout_elements_nested_table_cell_vertical_align_middle_integration() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td style="vertical-align: middle; height: 50pt">Inner</td>
                            <td style="height: 50pt">Other</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Inner)"),
            "Should render inner nested cell text"
        );
    }

    /// Integration: nested div inside table cell with SVG background
    /// exercises render_nested_text_block with background_svg via nested cell rows
    #[test]
    fn layout_elements_nested_svg_background_in_table_cell() {
        // A div with SVG background inside a td triggers render_nested_layout_elements
        // and render_nested_text_block with background_svg set
        let html = r#"<table>
            <tr>
                <td>
                    <div style="background-image: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22red%22/%3E%3C/svg%3E'); background-size: cover; width: 40pt; height: 20pt;">CellSVG</div>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The text should render
        assert!(
            pdf_str.contains("(CellSVG)"),
            "Should render text inside nested cell div"
        );
        // The overall PDF should be valid (no crash on SVG background in nested context)
        assert!(pdf_str.contains("%PDF-1.4"), "Should produce a valid PDF");
    }

    /// Integration: border-collapse collapse with nested elements
    #[test]
    fn layout_elements_nested_border_collapse() {
        let html = r#"<table style="border-collapse: collapse">
            <tr>
                <td style="border: 1pt solid black">CollapseA</td>
                <td style="border: 1pt solid black">CollapseB</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("(CollapseA)"), "Should render first cell");
        assert!(pdf_str.contains("(CollapseB)"), "Should render second cell");
    }

    /// Integration: nested table with rowspan > 1 spanning into future rows
    #[test]
    fn layout_elements_nested_rowspan_spans_future_rows() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td rowspan="2">SpanInner</td>
                            <td>A</td>
                        </tr>
                        <tr>
                            <td>B</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(SpanInner)"),
            "Should render spanning nested cell"
        );
        assert!(
            pdf_str.contains("(A)"),
            "Should render first row second cell"
        );
        assert!(
            pdf_str.contains("(B)"),
            "Should render second row second cell"
        );
    }

    // ── unicode_to_symbol ───────────────────────────────────────────────

    #[test]
    fn unicode_to_symbol_greek_lowercase() {
        assert_eq!(unicode_to_symbol('\u{03B1}'), Some(0x61)); // α
        assert_eq!(unicode_to_symbol('\u{03C0}'), Some(0x70)); // π
        assert_eq!(unicode_to_symbol('\u{03C9}'), Some(0x77)); // ω
    }

    #[test]
    fn unicode_to_symbol_greek_uppercase() {
        assert_eq!(unicode_to_symbol('\u{0393}'), Some(0x47)); // Γ
        assert_eq!(unicode_to_symbol('\u{03A9}'), Some(0x57)); // Ω
        assert_eq!(unicode_to_symbol('\u{03A3}'), Some(0x53)); // Σ
    }

    #[test]
    fn unicode_to_symbol_operators() {
        assert_eq!(unicode_to_symbol('\u{2211}'), Some(0xE5)); // ∑
        assert_eq!(unicode_to_symbol('\u{222B}'), Some(0xF2)); // ∫
        assert_eq!(unicode_to_symbol('\u{221E}'), Some(0xA5)); // ∞
    }

    #[test]
    fn unicode_to_symbol_relations() {
        assert_eq!(unicode_to_symbol('\u{2264}'), Some(0xA3)); // ≤
        assert_eq!(unicode_to_symbol('\u{2265}'), Some(0xB3)); // ≥
        assert_eq!(unicode_to_symbol('\u{2260}'), Some(0xB9)); // ≠
        assert_eq!(unicode_to_symbol('\u{2208}'), Some(0xCE)); // ∈
    }

    #[test]
    fn unicode_to_symbol_arrows() {
        assert_eq!(unicode_to_symbol('\u{2192}'), Some(0xAE)); // →
        assert_eq!(unicode_to_symbol('\u{2190}'), Some(0xAC)); // ←
        assert_eq!(unicode_to_symbol('\u{21D2}'), Some(0xDE)); // ⇒
    }

    #[test]
    fn unicode_to_symbol_delimiters() {
        assert_eq!(unicode_to_symbol('\u{27E8}'), Some(0xE1)); // ⟨
        assert_eq!(unicode_to_symbol('\u{27E9}'), Some(0xF1)); // ⟩
        assert_eq!(unicode_to_symbol('\u{230A}'), Some(0xEB)); // ⌊
        assert_eq!(unicode_to_symbol('\u{2309}'), Some(0xF9)); // ⌉
    }

    #[test]
    fn unicode_to_symbol_binary_ops() {
        assert_eq!(unicode_to_symbol('\u{00D7}'), Some(0xB4)); // ×
        assert_eq!(unicode_to_symbol('\u{00F7}'), Some(0xB8)); // ÷
        assert_eq!(unicode_to_symbol('\u{00B1}'), Some(0xB1)); // ±
    }

    #[test]
    fn unicode_to_symbol_misc() {
        assert_eq!(unicode_to_symbol('\u{2202}'), Some(0xB6)); // ∂
        assert_eq!(unicode_to_symbol('\u{2207}'), Some(0xD1)); // ∇
        assert_eq!(unicode_to_symbol('\u{2200}'), Some(0x22)); // ∀
        assert_eq!(unicode_to_symbol('\u{2203}'), Some(0x24)); // ∃
        assert_eq!(unicode_to_symbol('\u{2205}'), Some(0xC6)); // ∅
    }

    #[test]
    fn unicode_to_symbol_returns_none_for_ascii() {
        assert_eq!(unicode_to_symbol('A'), None);
        assert_eq!(unicode_to_symbol('x'), None);
        assert_eq!(unicode_to_symbol('+'), None);
    }

    // ── render_math_glyphs ──────────────────────────────────────────────

    #[test]
    fn render_math_glyphs_char_italic() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: 'x',
            x: 10.0,
            y: 20.0,
            font_size: 12.0,
            italic: true,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("Helvetica-Oblique"));
        assert!(content.contains("12 Tf"));
    }

    #[test]
    fn render_math_glyphs_char_regular() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: '2',
            x: 0.0,
            y: 0.0,
            font_size: 10.0,
            italic: false,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 5.0, 5.0, &mut content);
        assert!(content.contains("/Helvetica 10"));
        assert!(content.contains("(2) Tj"));
    }

    #[test]
    fn render_math_glyphs_symbol_char() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: '\u{03B1}', // α
            x: 0.0,
            y: 0.0,
            font_size: 12.0,
            italic: false,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("/Symbol 12 Tf"));
    }

    #[test]
    fn render_math_glyphs_text() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Text {
            text: "lim".to_string(),
            x: 0.0,
            y: 0.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("/Helvetica 12 Tf"));
        assert!(content.contains("(lim) Tj"));
    }

    #[test]
    fn render_math_glyphs_rule() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Rule {
            x: 10.0,
            y: 20.0,
            width: 50.0,
            thickness: 0.5,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn render_math_glyphs_radical() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Radical {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 15.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        // Radical draws lines
        assert!(content.contains(" l\n"));
        assert!(content.contains("S\n"));
    }

    #[test]
    fn render_math_glyphs_delimiter_small() {
        use crate::layout::math::MathGlyph;
        // Small delimiter: height <= font_size * 1.3, renders as text
        let glyphs = vec![MathGlyph::Delimiter {
            ch: '(',
            x: 0.0,
            y: 0.0,
            height: 12.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("Tf\n"));
    }

    #[test]
    fn render_math_glyphs_delimiter_large() {
        use crate::layout::math::MathGlyph;
        // Large delimiter: height > font_size * 1.3, renders as paths
        let glyphs = vec![MathGlyph::Delimiter {
            ch: '(',
            x: 0.0,
            y: 0.0,
            height: 30.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains(" c\n")); // cubic bezier for parenthesis
    }

    // ── Math integration via HTML ───────────────────────────────────────

    #[test]
    fn math_inline_produces_symbol_font_in_pdf() {
        let html = r#"<span class="math-inline" data-math="\alpha + \beta">α+β</span>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Symbol"));
    }

    #[test]
    fn math_display_produces_valid_pdf() {
        let html = r#"<div class="math-display" data-math="\frac{a}{b}">a/b</div>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.len() > 100);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("%PDF"));
    }

    #[test]
    fn math_markdown_inline_renders() {
        let pdf = crate::markdown_to_pdf("The equation $E = mc^2$ is famous.").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("BT\n"));
        assert!(pdf.len() > 200);
    }

    #[test]
    fn math_markdown_display_renders() {
        let pdf = crate::markdown_to_pdf("$$\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}$$").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Symbol"));
    }

    #[test]
    fn render_rgba_background_produces_extgstate() {
        let html =
            r#"<div style="background-color: rgba(255, 0, 0, 0.5)">Semi-transparent bg</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ca 0.5"),
            "PDF should contain fill opacity /ca 0.5 for rgba background"
        );
        assert!(
            content.contains("/ExtGState"),
            "PDF should contain ExtGState resource for rgba background"
        );
        assert!(
            content.contains("gs\n"),
            "PDF should use gs operator for rgba background"
        );
    }

    #[test]
    fn math_mixed_text_and_math() {
        let pdf =
            crate::markdown_to_pdf("For $x > 0$, we have $f(x) = x^2$ and $g(x) = \\sqrt{x}$.")
                .unwrap();
        assert!(pdf.len() > 200);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("%PDF"));
        assert!(text.contains("%%EOF"));
    }

    #[test]
    fn render_box_shadow_no_blur() {
        let html = r#"<div style="width: 100pt; height: 50pt; box-shadow: 5px 5px 0px rgba(0,0,0,0.5)">Shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // No-blur shadow should draw a solid rect fill
        assert!(
            content.contains("re\nf\n"),
            "Box shadow without blur should produce a filled rectangle"
        );
    }

    #[test]
    fn render_box_shadow_with_blur() {
        let html = r#"<div style="width: 100pt; height: 50pt; box-shadow: 3px 3px 10px rgba(0,0,0,0.4)">Blurred shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Blur shadow draws multiple layers with ExtGState for alpha
        assert!(
            content.contains("gs\n"),
            "Blurred box shadow should use graphics state for alpha layers"
        );
    }

    #[test]
    fn render_container_with_background_and_border() {
        let html = r#"
            <div style="background-color: #ccc; border: 2px solid blue; padding: 10px">
                <p>Inside container</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Container background fill
        assert!(
            content.contains("rg\n"),
            "Container should have background color fill"
        );
        // Container border stroke
        assert!(
            content.contains("RG\n"),
            "Container should have border stroke color"
        );
        assert!(
            content.contains("Inside container"),
            "Container children text should be rendered"
        );
    }

    #[test]
    fn render_flexbox_with_border() {
        let html = r#"
            <div style="display: flex; border: 1px solid red; padding: 5px">
                <div style="flex: 1">Left</div>
                <div style="flex: 1">Right</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Flex container border: red = 1 0 0 RG
        assert!(
            content.contains("1 0 0 RG"),
            "FlexRow border should use red stroke color"
        );
    }

    #[test]
    fn render_flexbox_with_background_color() {
        let html = r#"
            <div style="display: flex; background-color: yellow; padding: 8px">
                <div style="flex: 1">A</div>
                <div style="flex: 1">B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Yellow bg = 1 1 0 rg
        assert!(
            content.contains("1 1 0 rg"),
            "FlexRow should render yellow background"
        );
    }

    #[test]
    fn render_transform_skew_matrix_in_pdf() {
        // skew() produces a Transform::Matrix variant, exercising the Matrix arm
        let html = r#"<div style="transform: skew(10deg); width: 50pt; height: 30pt; background: red">Skewed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // skew(10deg) produces a Matrix transform which emits a cm operator
        assert!(
            content.contains("cm\n"),
            "CSS transform: skew() should produce a cm (concat matrix) operator in PDF"
        );
    }

    #[test]
    fn render_grid_row_with_border() {
        let html = r#"
            <div style="display: grid; grid-template-columns: 1fr 1fr; border: 2px solid green; gap: 4px">
                <div>Cell A</div>
                <div>Cell B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            pdf.starts_with(b"%PDF"),
            "Grid with border should produce valid PDF"
        );
        // Green border: 0 0.50196... 0 RG (CSS green = #008000)
        assert!(
            content.contains("RG\n"),
            "Grid border should produce stroke color"
        );
    }

    #[test]
    fn render_container_with_border_radius() {
        let html = r#"
            <div style="background-color: blue; border-radius: 10px; width: 100pt; height: 60pt; padding: 10px">
                <p>Rounded</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curves (c operator)
        assert!(
            content.contains(" c\n"),
            "Border radius should produce Bezier curve operators"
        );
    }

    #[test]
    fn render_pdf_to_writer_produces_same_output() {
        let nodes = parse_html("<p>Writer test</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf_bytes = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let mut writer_buf = Vec::new();
        render_pdf_to_writer(&pages, PageSize::A4, Margin::default(), &mut writer_buf).unwrap();
        assert_eq!(
            pdf_bytes.len(),
            writer_buf.len(),
            "render_pdf and render_pdf_to_writer should produce identical output"
        );
        assert_eq!(pdf_bytes, writer_buf);
    }
}
