use super::*;
use crate::layout::elements::TextBlock;

pub(super) fn render_text_child(
    content: &mut String,
    child: &TextBlock,
    child_index: usize,
    flow: &ContainerFlowContext<'_>,
    flow_position: FlowPosition,
    abs_origins: &mut HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) -> FlowPosition {
    let x = flow.frame.content_origin.x;
    let width = flow.frame.width();
    let self_pad_origin = flow.frame.padding_origin;
    let container_top_y = flow.container_top_y;
    let flow_top_by_index = flow.flow_top_by_index;
    let float_top_by_index = flow.float_top_by_index;
    let left_float_bottom = flow.left_float_bottom;
    let right_float_bottom = flow.right_float_bottom;
    let device_space_available = flow.device_space_available;
    let phase = flow.paint_phase;
    let FlowPosition {
        y: _,
        mut cursor_y,
        previous_margin_bottom: mut prev_margin_bottom,
    } = flow_position;
    let mut y;
    let lines = &child.lines;
    let margin_top = &child.box_model.margins.start;
    let margin_bottom = &child.box_model.margins.end;
    let padding = &child.box_model.padding;
    let border = &child.box_model.border;
    let block_height = &child.box_model.size.height;
    let background_color = &child.paint.background.color;
    let tb_bg_gradient = &child.paint.background.layers.gradient;
    let tb_bg_radial = &child.paint.background.layers.radial_gradient;
    let tb_bg_conic = &child.paint.background.layers.conic_gradient;
    let tb_bg_svg = ctx
        .text
        .pdf_writer
        .resolve_background_svg(&child.paint.background.layers);
    let tb_bg_svg = &tb_bg_svg;
    let tb_bg_blur = &child.paint.background.layers.blur_radius;
    let tb_bg_size = &child.paint.background.layers.size;
    let tb_bg_position = &child.paint.background.layers.position;
    let tb_bg_repeat = &child.paint.background.layers.repeat;
    let tb_bg_origin = &child.paint.background.layers.origin;
    let tb_bg_clip = &child.paint.background.layers.clip;
    let text_align = &child.text.alignment;
    let tb_float = &child.flow.float;
    let tb_clear = &child.flow.clear;
    let position = &child.positioning.scheme;
    let offset_top = &child.positioning.insets.top;
    let offset_left = &child.positioning.insets.left;
    let offset_bottom = &child.positioning.insets.bottom;
    let tb_opacity = &child.paint.group.effects.opacity;
    let tb_mix_blend = &child.paint.group.effects.mix_blend_mode;
    let tb_box_shadows = &child.paint.shadows;
    let tb_bg_blend = &child.paint.background.blend_mode;
    let tb_block_width = &child.box_model.size.width;
    let tb_clip_rect = &child.clipping.rect;
    let tb_box_transform = &child.paint.group.transform;
    let tb_transform = &tb_box_transform.value;
    let tb_radii = &child.paint.border_radii;
    let tb_text_indent = &child.text.indent;
    let tb_writing_mode = &child.text.writing_mode;
    let tb_containing_block = &child.positioning.containing_block;
    // Absolute-positioned children render at offset from the
    // containing block's padding box (CSS spec), not the content box.
    // Use container_top_y (original y before flow children advance it).
    if position.is_absolute() {
        let text_h: f32 = lines.iter().map(|l| l.height).sum();
        // `block_height` remains a padding-box height after pagination has
        // turned this text block into an absolute fragment.  Add the border
        // exactly once, just as the in-flow text path does.
        let content_pad_box = padding.vertical() + text_h;
        let pad_box_h = block_height.resolve(content_pad_box);
        let abs_h = pad_box_h + border.vertical_width();
        let abs_w = tb_block_width.resolve(width);
        // Anchor to the nearest positioned ancestor's padding box
        // (resolved by containing-block depth), skipping any static
        // intermediate container this box is nested inside.
        let anchor = abs_child_anchor(tb_containing_block, abs_origins, self_pad_origin);
        let abs_x = anchor.x + offset_left;
        let abs_y = anchor.y - offset_top;
        let tb_geometry = LayoutBoxGeometry::from_layout(
            PdfRect::from_top(abs_x, abs_y, abs_w, abs_h),
            border,
            *padding,
            child.paint.border_image.as_ref(),
        );
        let tb_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(tb_geometry);
        let tb_paint_geometry = tb_box_geometry.painting();
        let tb_background_geometry =
            tb_box_geometry.background(*tb_bg_origin, *tb_bg_clip, *tb_radii);
        let tb_background_box = tb_background_geometry.painting_box;
        let tb_fragment_geometry = tb_box_geometry.fragment(child.fragmentation.box_fragmentation);

        if phase == ElementPaintPhase::All
            && let Some(t) = tb_transform
            && is_projected_transform(t)
            && lines.is_empty()
            && tb_bg_gradient.is_none()
            && tb_bg_radial.is_none()
            && tb_bg_conic.is_none()
            && tb_bg_svg.is_none()
            && tb_radii.is_zero()
            && tb_box_shadows.is_empty()
            && *tb_opacity == 1.0
            && *tb_mix_blend == crate::style::computed::BlendMode::Normal
            && child.paint.group.effects.masking.clip_path.is_none()
            && child.paint.group.effects.masking.image.is_none()
        {
            render_projected_solid_box(
                content,
                ctx.text.pdf_writer.page_content_transform,
                tb_box_transform,
                tb_paint_geometry,
                *background_color,
                border,
            );
            return flow_position;
        }

        let group = PaintGroupScope::begin(content, child, tb_fragment_geometry, ctx);

        if phase.paints_decoration() {
            render_box_shadows(
                content,
                tb_box_shadows,
                tb_fragment_geometry,
                *tb_radii,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                ctx.text.pdf_writer,
            );
        }

        if phase.paints_decoration()
            && let Some(color) = background_color
        {
            let (r, g, b, a) = color.to_f32_rgba();
            let needs_alpha = a < 1.0;
            if needs_alpha {
                let gs_name = format!("GScca{}", ctx.bg_alpha_counter);
                *ctx.bg_alpha_counter += 1;
                ctx.page_ext_gstates.push((gs_name.clone(), a));
                content.push_str(&format!("/{gs_name} gs\n"));
            }
            content.push_str(&PdfRgb::from((r, g, b)).fill_operator());
            content.push_str(&tb_background_box.path_or_rect());
            content.push_str("f\n");
            if needs_alpha {
                content.push_str("/GSDefault gs\n");
            }
        }
        if phase.paints_decoration()
            && let Some(svg_tree) = tb_bg_svg
        {
            let bg_blend_mode = tb_bg_blend.background_layer(0);
            render_block_svg_background(
                content,
                svg_tree,
                PdfBackgroundResources::new(
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    ctx.shadings,
                    ctx.shading_counter,
                    Some(ctx.page_ext_gstates),
                )
                .with_custom_fonts(ctx.text.custom_fonts, ctx.text.prepared_custom_fonts),
                BlockBackground {
                    geometry: tb_box_geometry,
                    border_radii: *tb_radii,
                    size: *tb_bg_size,
                    position: *tb_bg_position,
                    repeat: *tb_bg_repeat,
                    origin: *tb_bg_origin,
                    clip: *tb_bg_clip,
                    blur_radius: *tb_bg_blur,
                    blend_mode: bg_blend_mode,
                },
            );
        }
        if phase.paints_decoration() {
            render_box_shadows_inset(
                content,
                tb_box_shadows,
                tb_fragment_geometry,
                *tb_radii,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                ctx.text.pdf_writer,
            );
            paint_box_decoration(
                content,
                tb_fragment_geometry,
                border,
                *tb_radii,
                child.paint.border_image.as_ref(),
                BorderPaintResources::from_page(ctx),
            );
        }
        // Render text for absolute-positioned children
        let mut baseline_cursor = TextBaselineCursor::new(
            abs_y - padding.top,
            ctx.text.pdf_writer.page_content_transform,
        );
        for line in lines.iter().filter(|_| phase.paints_contents()) {
            let metrics = line_box_metrics(line, ctx.text.custom_fonts);
            let text_y_abs = baseline_cursor.next_horizontal(metrics);
            let merged = crate::text::coalesce_text_runs(&line.runs);
            let line_width: f32 = merged
                .iter()
                .map(|r| estimate_run_width_with_fonts(r, ctx.text.custom_fonts))
                .sum();
            let text_x = match text_align {
                TextAlign::Right => abs_x + (abs_w - line_width).max(0.0),
                TextAlign::Center => abs_x + (abs_w - line_width).max(0.0) / 2.0,
                _ => abs_x,
            };
            let mut lx = text_x;
            for (run_index, run) in merged.iter().enumerate() {
                let run_width = estimate_run_width_with_fonts(run, ctx.text.custom_fonts);
                let previous = merged[..run_index]
                    .iter()
                    .rev()
                    .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
                let decoration = HorizontalRunDecorations::new(
                    run,
                    lx,
                    run_width,
                    text_y_abs,
                    ctx.text.custom_fonts,
                )
                .continuing_after(previous);
                let rw = decoration.paint_text(
                    content,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
                lx += rw;
            }
        }
        group.finish(content, ctx);
        // Don't advance cursor_y for absolute elements
        return flow_position;
    }

    // A floated block is out of normal flow: it is pinned at its
    // precomputed top (the flow cursor at its source position) and
    // does NOT advance `cursor_y`. An in-flow block collapses its
    // margin-top with the previous sibling, after first clearing any
    // floats it must drop below.
    let is_float = *tb_float != Float::None;
    let planned_flow_top = flow_top_by_index.get(&child_index).copied();
    if let Some(top) = planned_flow_top {
        y = top;
    } else if is_float {
        // Place the float from the shared simulator's top so its
        // paint position (it paints last) matches the flow.
        let rel_top = float_top_by_index.get(&child_index).copied().unwrap_or(0.0);
        y = container_top_y - rel_top;
    } else {
        if *tb_clear != Clear::None {
            cursor_y = clear_cursor(
                cursor_y,
                *tb_clear,
                left_float_bottom,
                right_float_bottom,
                &mut prev_margin_bottom,
            );
        }
        cursor_y -= collapsed_margin_top_extra(*margin_top, prev_margin_bottom);
        y = cursor_y;
    }
    let text_h: f32 = lines.iter().map(|l| l.height).sum();
    // `block_height` is a *padding-box* height (TextBlock convention),
    // so the painted border box adds the border on top. Compute the
    // padding-box height first (content vs. explicit), then add the
    // border once — mirroring paginate's `estimate_element_height`
    // (`effective_h + border.vertical_width()`) so the painted box
    // matches the flow. Folding the border into the value compared
    // against `block_height` (the old `max(content+border, bh)`)
    // rendered a border-box-sized child short by its border.
    let content_pad_box = padding.vertical() + text_h;
    // A provided `block_height` is the used padding-box height. Tall
    // inline content may overflow it, but it must not enlarge a box
    // with a definite CSS `height`.
    let pad_box_h = block_height.resolve(content_pad_box);
    let child_h = pad_box_h + border.vertical_width();

    let render_w = tb_block_width.resolve(width);
    let vertical_column_paint_h = if *offset_bottom > 0.0 && tb_writing_mode.is_vertical() {
        *offset_bottom + border.vertical_width()
    } else {
        child_h
    };

    // Resolve ordinary flow, relative translation, and sticky scrollport
    // constraints through the shared positioning contract. Sticky insets are
    // constraints from the scrollport edges, not unconditional translations.
    let normal_inline_offset = match tb_float {
        Float::Right => width - render_w,
        _ => 0.0,
    };
    let used_origin = child.positioning.resolve_in_flow_origin(
        crate::types::Point::new(normal_inline_offset, container_top_y - y),
        crate::types::Size::new(render_w, vertical_column_paint_h),
        flow.frame.size,
    );
    let render_x = x + used_origin.x;
    let render_y = container_top_y - used_origin.y;
    let tb_geometry = LayoutBoxGeometry::from_layout(
        PdfRect::from_top(render_x, render_y, render_w, vertical_column_paint_h),
        border,
        *padding,
        child.paint.border_image.as_ref(),
    );
    let tb_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(tb_geometry);
    let tb_paint_geometry = tb_box_geometry.painting();
    let tb_background_geometry = tb_box_geometry.background(*tb_bg_origin, *tb_bg_clip, *tb_radii);
    let tb_background_box = tb_background_geometry.painting_box;
    let tb_gradient_area = LayerPaintArea::new(
        tb_background_geometry
            .positioning_area
            .generated_image_box(),
        tb_background_geometry.image_destination_box,
    );
    let tb_fragment_geometry = tb_box_geometry.fragment(child.fragmentation.box_fragmentation);

    // CSS `filter: blur()` on a nested solid box (css-filter-effects-1
    // §4.1): rasterize the bg fill + border, gaussian-blur it, and
    // embed it overflowing the border box. Restricted to a plain solid
    // box (no gradient/SVG bg, no text, no clip, square corners) so the
    // vector paint path is byte-unchanged for everything else.
    if phase == ElementPaintPhase::All
        && *tb_bg_blur > 0.0
        && child.paint.group.effects.is_identity()
        && child.paint.group.transform.value.is_none()
        && lines.is_empty()
        && tb_bg_gradient.is_none()
        && tb_bg_radial.is_none()
        && tb_bg_conic.is_none()
        && tb_bg_svg.is_none()
        && tb_clip_rect.is_none()
        && *tb_bg_clip == BackgroundClip::Border
        && tb_radii.is_zero()
        && tb_box_shadows.is_empty()
        && child.paint.outline.width == 0.0
        && child.paint.background.blend_mode == crate::style::computed::BlendMode::Normal
        && let Some(blurred) = crate::render::blur::blur_box(
            tb_paint_geometry.border_box.width,
            tb_paint_geometry.border_box.height,
            *background_color,
            border,
            *tb_bg_blur,
            ctx.text.pdf_writer.opts.raster_quality.filter_dpi,
        )
    {
        let img_obj_id = ctx.text.pdf_writer.add_image_object(
            &blurred.asset.data,
            blurred.asset.source_width,
            blurred.asset.source_height,
            blurred.asset.format,
            blurred.asset.png_metadata.as_ref(),
        );
        let img_name = format!("Im{img_obj_id}");
        let ov = blurred.overflow_pt;
        content.push_str(&format!(
            "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
            w = tb_paint_geometry.border_box.width + 2.0 * ov,
            h = tb_paint_geometry.border_box.height + 2.0 * ov,
            ix = tb_paint_geometry.border_box.left - ov,
            iy = tb_paint_geometry.border_box.bottom - ov,
            name = img_name,
        ));
        ctx.text.page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
        // Advance the flow cursor exactly as the normal block path
        // below (the filter does not change layout).
        if is_float {
            prev_margin_bottom = 0.0;
        } else if planned_flow_top.is_some() {
            prev_margin_bottom = *margin_bottom;
        } else {
            cursor_y -= child_h + *margin_bottom;
            y = cursor_y;
            prev_margin_bottom = *margin_bottom;
        }
        return FlowPosition::new(y, cursor_y, prev_margin_bottom);
    }

    let tb_group = PaintGroupScope::begin(content, child, tb_fragment_geometry, ctx);

    if phase.paints_decoration() {
        render_box_shadows(
            content,
            tb_box_shadows,
            tb_fragment_geometry,
            *tb_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // Draw child background
        if let Some(color) = background_color {
            let (r, g, b, a) = color.to_f32_rgba();
            let needs_alpha = a < 1.0;
            if needs_alpha {
                let gs_name = format!("GScca{}", ctx.bg_alpha_counter);
                *ctx.bg_alpha_counter += 1;
                ctx.page_ext_gstates.push((gs_name.clone(), a));
                content.push_str(&format!("/{gs_name} gs\n"));
            }
            let color = PdfRgb::from((r, g, b));
            let uses_device_css_clip = device_space_available
                && tb_transform.is_none()
                && is_device_clippable_box_background(
                    *tb_bg_clip,
                    tb_background_box.radii,
                    ctx.text.pdf_writer.page_content_transform,
                    tb_background_box.rect,
                )
                && paint_device_clipped_css_solid(
                    content,
                    ctx.text.pdf_writer.page_content_transform,
                    tb_paint_geometry.border_box,
                    tb_background_box.rect,
                    color,
                );
            if !uses_device_css_clip {
                content.push_str(&color.fill_operator());
                content.push_str(&tb_background_box.path_or_rect());
                content.push_str("f\n");
            }
            if needs_alpha {
                content.push_str("/GSDefault gs\n");
            }
        }

        // `background-blend-mode`: the background image layers (gradient)
        // blend against the background color painted above. Scope the
        // blend gstate to a `q`..`Q` around the gradient paint.
        let bg_blend_mode = tb_bg_blend.background_layer(0);
        let bg_blended = bg_blend_mode != crate::style::computed::BlendMode::Normal;
        // Draw linear gradient background
        if let Some(gradient) = tb_bg_gradient {
            if bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, bg_blend_mode);
            }
            let rounded_clip = tb_background_box.push_rounded_clip(content);
            render_linear_gradient(
                content,
                gradient,
                GradientBackdrop::isolated_linear_layer(
                    *background_color,
                    tb_bg_radial.is_some() || tb_bg_conic.is_some() || tb_bg_svg.is_some(),
                    bg_blend_mode,
                ),
                tb_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if rounded_clip {
                content.push_str("Q\n");
            }
            if bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw radial gradient background
        if let Some(gradient) = tb_bg_radial {
            if bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, bg_blend_mode);
            }
            let rounded_clip = tb_background_box.push_rounded_clip(content);
            render_radial_gradient(
                content,
                gradient,
                tb_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if rounded_clip {
                content.push_str("Q\n");
            }
            if bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw conic gradient background
        if let Some(gradient) = tb_bg_conic {
            if bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, bg_blend_mode);
            }
            let rounded_clip = tb_background_box.push_rounded_clip(content);
            render_conic_gradient(
                content,
                gradient,
                tb_gradient_area,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if rounded_clip {
                content.push_str("Q\n");
            }
            if bg_blended {
                content.push_str("Q\n");
            }
        }

        if let Some(svg_tree) = tb_bg_svg {
            render_block_svg_background(
                content,
                svg_tree,
                PdfBackgroundResources::new(
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    ctx.shadings,
                    ctx.shading_counter,
                    Some(ctx.page_ext_gstates),
                )
                .with_custom_fonts(ctx.text.custom_fonts, ctx.text.prepared_custom_fonts),
                BlockBackground {
                    geometry: tb_box_geometry,
                    border_radii: *tb_radii,
                    size: *tb_bg_size,
                    position: *tb_bg_position,
                    repeat: *tb_bg_repeat,
                    origin: *tb_bg_origin,
                    clip: *tb_bg_clip,
                    blur_radius: *tb_bg_blur,
                    blend_mode: bg_blend_mode,
                },
            );
        }

        render_box_shadows_inset(
            content,
            tb_box_shadows,
            tb_fragment_geometry,
            *tb_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );
        paint_box_decoration(
            content,
            tb_fragment_geometry,
            border,
            *tb_radii,
            child.paint.border_image.as_ref(),
            BorderPaintResources::from_page(ctx),
        );
    }

    // Apply the overflow clip (if any) around the text so overflowing
    // lines are cut at the padding box, matching Chrome. The
    // background and border above are drawn unclipped so the border
    // stays fully visible; only the inline/line-box content is
    // clipped. `tb_clip_rect` is set by layout for
    // overflow:hidden/clip/scroll/auto.
    let tb_needs_clip = phase.paints_contents() && tb_clip_rect.is_some();
    if tb_needs_clip {
        tb_paint_geometry
            .rounded_padding_box(*tb_radii)
            .push_clip(content);
    }

    // Draw child text. Inset from the border-box top by BOTH the top
    // border width and the top padding (matching the primary text path
    // at the top of this fn); omitting the border placed the first
    // baseline `border-top` px too high inside bordered clip boxes.
    let content_top = tb_geometry.content_box().top();
    let mut baseline_cursor =
        TextBaselineCursor::new(content_top, ctx.text.pdf_writer.page_content_transform);
    let line_metadata = lines
        .first()
        .map_or(Default::default(), |line| line.metadata);
    let vertical_lr = line_metadata.writing_mode.is_vertical_lr();
    let upright_vertical = line_metadata.text_orientation_upright;
    let vertical = phase.paints_contents() && tb_writing_mode.is_vertical() && !upright_vertical;
    let vertical_transform = if vertical {
        let content_left = tb_geometry.content_box().left;
        let content_right = tb_geometry.content_box().right();
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
    let mut tb_first_line = true;
    for line in lines.iter().filter(|_| phase.paints_contents()) {
        let metrics = line_box_metrics(line, ctx.text.custom_fonts);
        let text_y = if vertical {
            baseline_cursor.next_raw(metrics)
        } else {
            baseline_cursor.next_horizontal(metrics)
        };
        let merged = crate::text::coalesce_text_runs(&line.runs);
        let line_width: f32 = merged
            .iter()
            .map(|run| {
                if upright_vertical {
                    text_combine_advance(run, ctx.text.custom_fonts).unwrap_or_else(|| {
                        estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                    })
                } else {
                    estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                }
            })
            .sum();
        // CSS `text-indent` shifts only the first line's start. List
        // items pass a negative value so an `outside` marker (the
        // leading run) hangs left into the padding while the text
        // lands at the content edge.
        let first_line_indent = if tb_first_line { *tb_text_indent } else { 0.0 };
        tb_first_line = false;
        // Horizontal insets from the border-box edge. `render_x`/`render_w`
        // are the BORDER box, so the content box starts after the left
        // border + left padding and is narrowed by both horizontal borders
        // and paddings — mirroring the primary text path
        // (`padding_box_x = block_x + border_left`,
        // content = `padding_box_x + padding.left`) and the vertical inset
        // in this same arm (`render_y - border.top.width - padding.top`).
        // For left/justify the text starts at the content-box left; for
        // right/center it is aligned within the content box. (Previously
        // this branch used `render_x + padding.left`, dropping the left
        // border so text in bordered clip/nested boxes sat `border-left`
        // px too far left.)
        let content_x = tb_geometry.content_box().left;
        let content_w = tb_geometry.content_box().width;
        // Drop-cap float exclusion: shift the line right so its text
        // wraps beside the floated `::first-letter` (css2 §9.5).
        let line_inset = line.x_offset;
        let text_x = match text_align {
            TextAlign::Right => content_x + (content_w - line_width).max(0.0),
            TextAlign::Center => content_x + (content_w - line_width).max(0.0) / 2.0,
            _ => content_x + first_line_indent + line_inset,
        };
        let line_top_y = text_y + metrics.ascender + metrics.half_leading;
        let line_bottom_y = text_y - metrics.descender - metrics.half_leading;
        // Parent text content-area edges for `text-top`/`text-bottom`.
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
        let mut lx = text_x;
        for (run_index, run) in merged.iter().enumerate() {
            // Atomic inline box (e.g. a `list-style-image` marker):
            // paint the box/image and advance by its outer width;
            // The glyph painter would shape its empty text and draw
            // nothing, dropping the marker entirely.
            if let Some(inline) = run.inline_box.as_deref() {
                if !run.is_inline_edge() {
                    render_inline_box(
                        content,
                        inline,
                        lx + inline.margin_left,
                        text_y,
                        ctx.text.page_height,
                        line_top_y,
                        line_bottom_y,
                        line_text_top_y,
                        line_text_bottom_y,
                        run.font_size,
                        run_line_height_for_vertical_align(run),
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
                lx += run.atomic_inline_advance().unwrap_or_default();
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
            // Per-run inline background (e.g. a `::first-letter`/
            // `::first-line` `background-color`, or a highlighted
            // inline span): paint the rectangle behind the glyphs
            // before drawing the text. Mirrors the other line-box
            // render paths (table cells, absolute boxes).
            if let Some(background) = run.background_color {
                let (br, bgc, bb, ba) = background.to_f32_rgba();
                let needs_inline_bg_alpha = ba < 1.0;
                if needs_inline_bg_alpha {
                    let gs_name = format!("GStbiba{}", ctx.bg_alpha_counter);
                    *ctx.bg_alpha_counter += 1;
                    ctx.page_ext_gstates.push((gs_name.clone(), ba));
                    content.push_str(&format!("/{gs_name} gs\n"));
                }
                let rx = lx - run.padding.left;
                let rw2 = run_width + run.padding.horizontal();
                let (ry, rh) =
                    inline_background_y_and_height(run, text_y, run.padding, ctx.text.custom_fonts);
                content.push_str(&PdfRgb::from((br, bgc, bb)).fill_operator());
                content.push_str(
                    &PdfRect::new(rx, ry, rw2, rh)
                        .rounded(run.border_radii)
                        .path_or_rect(),
                );
                content.push_str("f\n");
                if needs_inline_bg_alpha {
                    content.push_str("/GSDefault gs\n");
                }
            }
            // A floated `::first-letter` drop cap is lowered so its
            // glyph top sits on the line's text top (css-pseudo-4 §2.2).
            let run_y = text_y
                + drop_cap_baseline_shift(
                    run,
                    line_text_top(line, ctx.text.custom_fonts),
                    ctx.text.custom_fonts,
                );
            let decoration = (!vertical).then(|| {
                let previous = merged[..run_index]
                    .iter()
                    .rev()
                    .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
                HorizontalRunDecorations::new(run, lx, run_width, run_y, ctx.text.custom_fonts)
                    .continuing_after(previous)
            });
            if let Some(decoration) = &decoration {
                decoration.paint_shadows(content);
                render_run_text_shadows(
                    content,
                    run,
                    lx,
                    run_y,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
                decoration.paint_below_text(content);
            }
            let rw = if *tb_bg_blur > 0.0
                && render_text_shadow_blur(
                    content,
                    run,
                    lx,
                    run_y,
                    *tb_bg_blur * 2.0,
                    run.color.to_f32_rgba(),
                    ctx.text.custom_fonts,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                ) {
                if upright_vertical {
                    text_combine_advance(run, ctx.text.custom_fonts).unwrap_or_else(|| {
                        estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                    })
                } else {
                    estimate_run_width_with_fonts(run, ctx.text.custom_fonts)
                }
            } else if upright_vertical && run.metadata.text_combine_upright.is_active() {
                render_text_combine_run(
                    content,
                    run,
                    lx,
                    run_y,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                )
            } else if let Some((vertical_e, vertical_f)) = vertical_transform
                && vertical_mixed_upright_run(run)
            {
                render_vertical_mixed_upright_run(
                    content,
                    run,
                    lx,
                    run_y,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    vertical_e,
                    vertical_f,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                )
            } else if decoration.is_some() {
                render_run_glyphs_without_shadows(
                    content,
                    run,
                    lx,
                    run_y,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                )
            } else {
                render_run_glyphs(
                    content,
                    run,
                    lx,
                    run_y,
                    crate::layout::text::line_primary_font_size(&merged),
                    ctx.text.custom_fonts,
                    ctx.text.prepared_custom_fonts,
                    0.0,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                )
            };
            if let Some(decoration) = &decoration {
                decoration.paint_above_text(content);
            }
            lx += rw;
        }
    }
    if vertical {
        content.push_str("Q\n");
    }
    if tb_needs_clip {
        content.push_str("Q\n");
    }
    tb_group.finish(content, ctx);
    if is_float {
        // A float does not advance the flow cursor (its bottom is
        // already tracked via the simulator for `clear`). It breaks
        // the margin-collapse chain for the next in-flow sibling.
        let _ = child_h;
        prev_margin_bottom = 0.0;
    } else if planned_flow_top.is_some() {
        prev_margin_bottom = *margin_bottom;
    } else {
        // Advance past the box AND its margin-bottom so a following
        // in-flow sibling sits below the margin gap (e.g. stacked
        // `<p>`s inside a multicol column keep their `margin-bottom`).
        // Record this block's margin-bottom so the next sibling
        // collapses its margin-top against it (CSS adjacent-margin
        // collapsing), mirroring the Container arm below.
        cursor_y -= child_h + *margin_bottom;
        y = cursor_y;
        prev_margin_bottom = *margin_bottom;
    }

    FlowPosition::new(y, cursor_y, prev_margin_bottom)
}
