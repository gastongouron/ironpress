use super::*;
use crate::layout::elements::TextBlock;

pub(super) fn render_text_block_lines(
    content: &mut String,
    element: &TextBlock,
    box_geometry: BoxPaintGeometry,
    frame: PageElementFrame<'_>,
    opacity_active: bool,
    text_space: PdfContentSpace,
    ctx: &mut PageRenderContext<'_>,
) {
    let geometry = box_geometry.layout();
    let lines = &element.lines;
    let padding = &element.box_model.padding;
    let border = &element.box_model.border;
    let writing_mode = &element.text.writing_mode;
    let text_align = &element.text.alignment;
    let text_indent = &element.text.indent;
    let background_blur_radius = &element.paint.background.layers.blur_radius;
    let background_color = &element.paint.background.color;
    let background_gradient = &element.paint.background.layers.gradient;
    let background_radial_gradient = &element.paint.background.layers.radial_gradient;
    let background_conic_gradient = &element.paint.background.layers.conic_gradient;
    let background_size = &element.paint.background.layers.size;
    let background_position = &element.paint.background.layers.position;
    let background_repeat = &element.paint.background.layers.repeat;
    let background_origin = &element.paint.background.layers.origin;
    let background_clip = &element.paint.background.layers.clip;
    let background_blend_mode = &element.paint.background.blend_mode;
    let opacity = &element.paint.group.effects.opacity;
    let block_x = geometry.border_box.left;
    let block_y = geometry.border_box.top();
    let render_width = geometry.border_box.width;
    let elem_idx = frame.element_index;
    let page_size = frame.page_size;
    let needs_opacity = opacity_active;
    let background_geometry = box_geometry.background(
        *background_origin,
        *background_clip,
        element.paint.border_radii,
    );
    let tb_reference = background_geometry.positioning_area.generated_image_box();
    let tb_text_clip_background = *background_clip == BackgroundClip::Text;
    let tb_layer_box =
        background_layer_box(*background_size, *background_position, *background_repeat);
    let tb_bg_blend_mode = background_blend_mode.background_layer(0);
    let tb_bg_blended = tb_bg_blend_mode != crate::style::computed::BlendMode::Normal;
    let gradient_area = |attachment| {
        if attachment == Some(BackgroundAttachment::Fixed) {
            LayerPaintArea::new(
                PdfRect::new(0.0, 0.0, page_size.width, page_size.height),
                background_geometry.painting_box.rect,
            )
        } else {
            LayerPaintArea::new(tb_reference, background_geometry.image_destination_box)
        }
    };

    // Text content is inset from the border-box top by the top
    // border width and the top padding.
    let content_top = block_y - border.top.width - padding.top;
    let mut baseline_cursor =
        TextBaselineCursor::new(content_top, ctx.text.pdf_writer.page_content_transform);

    // Horizontal insets: `block_x` / `render_width` are the
    // border-box left / width, so the content area starts after
    // the left border + left padding and is narrowed by the
    // horizontal borders + paddings.
    let border_left = border.left.width;
    let border_right = border.right.width;
    // Content-box left edge and width (content + padding ⇒ here we
    // keep padding in `content_x`/`content_width` because the text
    // branches add `padding.left`/`padding.right` themselves; this
    // pair is the PADDING box).
    let padding_box_x = block_x + border_left;
    let padding_box_w = (render_width - border_left - border_right).max(0.0);

    // CSS `writing-mode: vertical-rl` (css-writing-modes-4 §3.1).
    // The box geometry stays physical/axis-aligned (already laid
    // out above); only the inline text is set vertically. With the
    // default `text-orientation: mixed`, Latin runs are rotated 90°
    // clockwise (set sideways) and flow top-to-bottom in the first
    // (right-most) column.
    //
    // We lay the run out horizontally as usual, then apply a single
    // `cm` that rotates 90° clockwise (PDF `[0 -1 1 0]`, which maps
    // local +x→PDF −y "down" and local +y→PDF +x "right") and
    // translates so the horizontal text's content-top-left anchors
    // at the content box and the column hugs the right edge. The
    // wrapper is scoped to the glyph-drawing loop only, so the
    // background/border/outline (painted earlier) stay upright.
    let line_metadata = lines
        .first()
        .map_or(Default::default(), |line| line.metadata);
    let vertical_lr = line_metadata.writing_mode.is_vertical_lr();
    let upright_vertical = line_metadata.text_orientation_upright;
    let vertical = writing_mode.is_vertical() && !upright_vertical;
    let content_right = padding_box_x + padding_box_w - padding.right;
    let content_left = padding_box_x + padding.left;
    let vertical_transform = if vertical {
        // Content-box edges in PDF (y-up) coordinates. `text_y`
        // currently sits at the content-area top (block_y − top
        // border − top padding) before any line advance.
        // matrix maps (gx, gy) → (gy + e, −gx + f):
        //   glyph top (gy = content_top) → X = content_right (column
        //     hugs the right edge), and
        //   text start (gx = content_left) → Y = content_top (text
        //     begins at the top of the column, flowing downward).
        let column_x = if vertical_lr {
            content_left + lines.first().map_or(0.0, |line| line.height)
        } else {
            content_right
        };
        let e = column_x - content_top;
        let f = content_top + content_left;
        content.push_str("q\n");
        content.push_str(&format!("0 -1 1 0 {e} {f} cm\n"));
        Some((e, f))
    } else {
        None
    };
    let line_count = lines.len();
    for (line_idx, line) in lines.iter().enumerate() {
        let metrics = if upright_vertical {
            upright_vertical_line_metrics(line, ctx.text.custom_fonts)
        } else {
            line_box_metrics(line, ctx.text.custom_fonts)
        };
        let text_y = if vertical {
            baseline_cursor.next_raw(metrics)
        } else {
            baseline_cursor.next_horizontal(metrics)
        };
        let line_annotation_top = text_y + metrics.ascender + metrics.half_leading;
        let line_annotation_bottom = text_y - metrics.descender - metrics.half_leading;

        let line_text = line_text_content(line);
        let has_inline_box = line.runs.iter().any(|r| r.inline_box.is_some());
        if line_text.is_empty() && !has_inline_box {
            continue;
        }

        let line_width = if upright_vertical {
            line.runs
                .iter()
                .map(|run| {
                    text_combine_advance(run, ctx.text.custom_fonts).unwrap_or_else(|| {
                        estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                    })
                })
                .sum()
        } else {
            estimate_line_width_with_fonts(line, ctx.text.custom_fonts)
        };
        let is_last_line = line_idx == line_count - 1;

        // Calculate word spacing for justified text
        let justify_ws = if *text_align == TextAlign::Justify && !is_last_line {
            let first_line_indent = if line_idx == 0 { *text_indent } else { 0.0 };
            let content_width = padding_box_w - padding.horizontal() - first_line_indent;
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
        // CSS `text-indent` shifts the start of the FIRST line's
        // inline content. For start-edge alignment (left/justify)
        // it offsets the text origin; for center/right it consumes
        // available width on the start side, recentring/reflowing
        // the first line within the remaining space.
        let first_line_indent = if line_idx == 0 { *text_indent } else { 0.0 };
        // Drop-cap float exclusion: the line is shifted right so
        // its inline content wraps beside the floated
        // `::first-letter` (css-pseudo-4 §2.2 + css2 §9.5).
        let line_inset = line.x_offset;
        let text_x = match text_align {
            TextAlign::Left | TextAlign::Justify => {
                if upright_vertical {
                    let upright_box_width = line
                        .runs
                        .iter()
                        .find(|r| r.inline_box.is_none())
                        .map(run_line_height_for_vertical_align)
                        .unwrap_or(line.height);
                    content_right - (upright_box_width + line_width) / 2.0 + line_inset
                } else {
                    padding_box_x + padding.left + first_line_indent + line_inset
                }
            }
            TextAlign::Center => {
                padding_box_x
                    + first_line_indent
                    + (padding_box_w - first_line_indent - line_width) / 2.0
            }
            TextAlign::Right => padding_box_x + padding_box_w - padding.right - line_width,
        };
        // Merge consecutive runs with the same style so
        // spaces between words stay in a single PDF text
        // string, preventing viewers from dropping them.
        let merged = crate::text::coalesce_text_runs(&line.runs);
        let mut text_clip_line_painted = false;
        if tb_text_clip_background {
            if let Some(gradient) = background_gradient {
                let gradient = linear_with_background_layer(gradient, tb_layer_box);
                if gradient.layer_box.attachment != Some(BackgroundAttachment::Local) {
                    content.push_str("q\n");
                    if push_marked_line_text_clip(
                        content,
                        line,
                        &merged,
                        PdfPoint::new(text_x, text_y),
                        justify_ws,
                        element.paint.group.owner,
                        ctx,
                    ) {
                        if tb_bg_blended {
                            content.push_str("q\n");
                            begin_blend_mode(content, ctx.page_ext_gstates, tb_bg_blend_mode);
                        }
                        render_linear_gradient(
                            content,
                            &gradient,
                            GradientBackdrop::isolated_linear_layer(
                                *background_color,
                                background_radial_gradient.is_some()
                                    || background_conic_gradient.is_some(),
                                tb_bg_blend_mode,
                            ),
                            gradient_area(gradient.layer_box.attachment),
                            ctx.shadings,
                            ctx.shading_counter,
                            ctx.text.pdf_writer,
                            ctx.text.page_images,
                        );
                        if tb_bg_blended {
                            content.push_str("Q\n");
                        }
                        text_clip_line_painted = true;
                    }
                    content.push_str("Q\n");
                }
            } else if let Some(gradient) = background_radial_gradient {
                let gradient = radial_with_background_layer(gradient, tb_layer_box);
                if gradient.layer_box.attachment != Some(BackgroundAttachment::Local) {
                    content.push_str("q\n");
                    if push_marked_line_text_clip(
                        content,
                        line,
                        &merged,
                        PdfPoint::new(text_x, text_y),
                        justify_ws,
                        element.paint.group.owner,
                        ctx,
                    ) {
                        if tb_bg_blended {
                            content.push_str("q\n");
                            begin_blend_mode(content, ctx.page_ext_gstates, tb_bg_blend_mode);
                        }
                        render_radial_gradient(
                            content,
                            &gradient,
                            gradient_area(gradient.layer_box.attachment),
                            ctx.shadings,
                            ctx.shading_counter,
                            ctx.text.pdf_writer,
                            ctx.text.page_images,
                        );
                        if tb_bg_blended {
                            content.push_str("Q\n");
                        }
                        text_clip_line_painted = true;
                    }
                    content.push_str("Q\n");
                }
            } else if let Some(gradient) = background_conic_gradient {
                let gradient = conic_with_background_layer(gradient, tb_layer_box);
                if gradient.layer_box.attachment != Some(BackgroundAttachment::Local) {
                    content.push_str("q\n");
                    if push_marked_line_text_clip(
                        content,
                        line,
                        &merged,
                        PdfPoint::new(text_x, text_y),
                        justify_ws,
                        element.paint.group.owner,
                        ctx,
                    ) {
                        if tb_bg_blended {
                            content.push_str("q\n");
                            begin_blend_mode(content, ctx.page_ext_gstates, tb_bg_blend_mode);
                        }
                        render_conic_gradient(
                            content,
                            &gradient,
                            gradient_area(gradient.layer_box.attachment),
                            ctx.text.pdf_writer,
                            ctx.text.page_images,
                        );
                        if tb_bg_blended {
                            content.push_str("Q\n");
                        }
                        text_clip_line_painted = true;
                    }
                    content.push_str("Q\n");
                }
            }
        }

        // Phase 1: Draw backgrounds, decorations, and link
        // ctx.text.annotations at estimated positions (visual-only).
        let line_top_y = text_y + metrics.ascender + metrics.half_leading;
        let line_bottom_y = text_y - metrics.descender - metrics.half_leading;
        // Parent text content-area edges for `text-top`/`text-bottom`
        // (parent glyph ascent/descent, no half-leading). Fall back to
        // the line-box edges when the line carries no parent text.
        let (text_ascent, text_descent) = line_text_content_extents(line, ctx.text.custom_fonts);
        let line_text_top_y = if text_ascent > 0.0 {
            text_y + text_ascent
        } else {
            line_top_y
        };
        let line_text_bottom_y = if text_descent > 0.0 {
            text_y - text_descent
        } else {
            line_bottom_y
        };
        let mut bg_x = text_x;
        // Relatively-positioned inline boxes paint in the positioned
        // layer — above in-flow siblings on the line, in source order
        // (CSS 2.1 §9.9.1 painting order). Defer them so a later
        // in-flow inline-block can't paint over an earlier offset one.
        let mut deferred_inline: Vec<(&crate::layout::engine::InlineBox, f32, f32, f32)> =
            Vec::new();
        for (run_idx, run) in merged.iter().enumerate() {
            // Atomic inline box (display: inline-block): paint the
            // box and its inner content, then advance the cursor.
            if let Some(inline) = run.inline_box.as_deref() {
                let ibx = bg_x + inline.margin_left;
                let run_line_height = run_line_height_for_vertical_align(run);
                if !run.is_inline_edge() {
                    if inline.rel_offset_x != 0.0 || inline.rel_offset_y != 0.0 {
                        deferred_inline.push((inline, ibx, run.font_size, run_line_height));
                    } else {
                        render_inline_box(
                            content,
                            inline,
                            ibx,
                            text_y,
                            page_size.height,
                            line_top_y,
                            line_bottom_y,
                            line_text_top_y,
                            line_text_bottom_y,
                            run.font_size,
                            run_line_height,
                            line_primary_x_height_ratio(&merged, ctx.text.custom_fonts),
                            ctx.text.custom_fonts,
                            ctx.text.prepared_custom_fonts,
                            ctx.page_ext_gstates,
                            ctx.bg_alpha_counter,
                            ctx.shadings,
                            ctx.shading_counter,
                            ctx.text.pdf_writer,
                            ctx.text.page_images,
                        );
                    }
                }
                bg_x += run.atomic_inline_advance().unwrap_or_default();
                continue;
            }
            if run.text.is_empty() {
                continue;
            }
            let run_width = if upright_vertical {
                text_combine_advance(run, ctx.text.custom_fonts)
                    .unwrap_or_else(|| estimate_run_width_with_fonts(run, ctx.text.custom_fonts))
            } else {
                estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
            };
            // Draw background rectangle for inline spans
            if let Some(background) = run.background_color {
                let (br, bg, bb, ba) = background.to_f32_rgba();
                let needs_inline_bg_alpha = ba < 1.0;
                if needs_inline_bg_alpha {
                    let effective_alpha = ba * *opacity;
                    let gs_name = format!("GSba{}", ctx.bg_alpha_counter);
                    *ctx.bg_alpha_counter += 1;
                    ctx.page_ext_gstates
                        .push((gs_name.clone(), effective_alpha));
                    content.push_str(&format!("/{gs_name} gs\n"));
                }
                let rect_x = bg_x - run.padding.left;
                let rect_w = run_width + run.padding.horizontal();
                let (rect_y, rect_h) =
                    inline_background_y_and_height(run, text_y, run.padding, ctx.text.custom_fonts);
                content.push_str(&format!("{br} {bg} {bb} rg\n"));
                content.push_str(
                    &PdfRect::new(rect_x, rect_y, rect_w, rect_h)
                        .rounded(run.border_radii)
                        .path_or_rect(),
                );
                content.push_str("f\n");
                if needs_inline_bg_alpha {
                    if needs_opacity {
                        let gs_name = format!("GS{elem_idx}");
                        content.push_str(&format!("/{gs_name} gs\n"));
                    } else {
                        content.push_str("/GSDefault gs\n");
                    }
                }
            }

            if vertical {
                let previous = merged[..run_idx]
                    .iter()
                    .rev()
                    .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
                let decoration = HorizontalRunDecorations::new(
                    run,
                    bg_x,
                    run_width,
                    text_y,
                    ctx.text.custom_fonts,
                )
                .continuing_after(previous);
                decoration.paint_shadows(content);
                decoration.paint_below_text(content);
                decoration.paint_above_text(content);
                if decoration_is_emphasis(run) {
                    render_text_emphasis_marks(
                        content,
                        run,
                        TextEmphasisPlacement {
                            origin: PdfPoint::new(bg_x, text_y),
                            color: run.metadata.emphasis.color,
                        },
                        ctx.text.custom_fonts,
                        ctx.text.prepared_custom_fonts,
                        ctx.text.pdf_writer,
                        ctx.text.page_images,
                    );
                }
            }

            // Track link annotation
            if let Some(annotation) = text_run_link_annotation(
                run,
                PdfRect::new(
                    bg_x,
                    line_annotation_bottom,
                    run_width,
                    line_annotation_top - line_annotation_bottom,
                ),
            ) {
                ctx.text.annotations.push(annotation);
            }

            bg_x += run_width;
        }

        // Paint deferred relatively-positioned inline boxes on top
        // of the in-flow line content, preserving source order.
        for (inline, ibx, fs, run_line_height) in deferred_inline {
            render_inline_box(
                content,
                inline,
                ibx,
                text_y,
                page_size.height,
                line_top_y,
                line_bottom_y,
                line_text_top_y,
                line_text_bottom_y,
                fs,
                run_line_height,
                line_primary_x_height_ratio(&merged, ctx.text.custom_fonts),
                ctx.text.custom_fonts,
                ctx.text.prepared_custom_fonts,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
        }

        // Phase 2: Render all text in a single BT/ET block
        // so the PDF viewer advances the cursor naturally.
        let mut blurred_line = false;
        if *background_blur_radius > 0.0 {
            let mut lx = text_x;
            blurred_line = true;
            for run in &merged {
                if run.inline_box.is_some() {
                    lx += run.atomic_inline_advance().unwrap_or_default();
                    continue;
                }
                if run.text.is_empty() {
                    continue;
                }
                let mut picture = String::new();
                if !render_text_shadow_blur(
                    &mut picture,
                    run,
                    lx,
                    text_y,
                    *background_blur_radius * 2.0,
                    run.color.to_f32_rgba(),
                    ctx.text.custom_fonts,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                ) {
                    blurred_line = false;
                    break;
                }
                // The text is in the picture: the run's mark reads it through
                // /ActualText (ISO 32000-1 14.9.4).
                ctx.text.pdf_writer.structure.append_content(
                    content,
                    run.metadata.structure,
                    crate::render::pdf::structure::MarkKind::rasterized_run(run),
                    &picture,
                );
                lx += if upright_vertical {
                    text_combine_advance(run, ctx.text.custom_fonts).unwrap_or_else(|| {
                        estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                    })
                } else {
                    estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                };
            }
        }
        if !blurred_line && !text_clip_line_painted {
            if upright_vertical {
                render_upright_vertical_line_text(
                    content,
                    &merged,
                    UprightLinePosition::new(
                        PdfPoint::new(text_x, text_y),
                        VerticalLineCrossAxis::from_content_edges(
                            content_left,
                            content_right,
                            vertical_lr,
                        ),
                    ),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    justify_ws,
                    line_text_top(line, ctx.text.custom_fonts),
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
            } else if let Some((vertical_e, vertical_f)) = vertical_transform {
                render_vertical_mixed_line_text(
                    content,
                    &merged,
                    text_x,
                    text_y,
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    justify_ws,
                    line_text_top(line, ctx.text.custom_fonts),
                    vertical_e,
                    vertical_f,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
            } else {
                paint_horizontal_line_text(
                    content,
                    &merged,
                    HorizontalLinePaint {
                        origin: PdfPoint::new(text_x, text_y),
                        line_ascender: line_text_top(line, ctx.text.custom_fonts),
                        justification_word_spacing: justify_ws,
                        text_space,
                    },
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
            }
        }
    }

    // Close the `vertical-rl` rotation wrapper opened before the
    // line loop (scoped to glyph drawing only).
    if vertical {
        content.push_str("Q\n");
    }
}

/// Clip to the glyphs of `line` (`background-clip: text`). The text object
/// is the line's content, read through `/ActualText` because a synthetic
/// weight repeats each outline inside it (ISO 32000-1 9.3.6, 14.9.4).
#[allow(clippy::too_many_arguments)]
fn push_marked_line_text_clip(
    content: &mut String,
    line: &crate::layout::engine::TextLine,
    merged: &[crate::layout::engine::TextRun],
    origin: PdfPoint,
    word_spacing: f32,
    owner: crate::accessibility::StructureRef,
    ctx: &mut PageRenderContext<'_>,
) -> bool {
    let mut clip = String::new();
    if !push_line_text_clip(
        &mut clip,
        merged,
        origin.x,
        origin.y,
        ctx.text.custom_fonts,
        ctx.text.prepared_custom_fonts,
        word_spacing,
        line_text_top(line, ctx.text.custom_fonts),
    ) {
        return false;
    }
    ctx.text.pdf_writer.structure.append_content(
        content,
        owner,
        crate::render::pdf::structure::MarkKind::shown_once(std::slice::from_ref(line)),
        &clip,
    );
    true
}
