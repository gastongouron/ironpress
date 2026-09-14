use super::*;
use crate::layout::elements::Container;
use crate::layout::engine::StackingContext;
use crate::render::pdf::affine_solids::AffineSolidGroup;

fn push_nested_background_clip(
    content: &mut String,
    clip: RoundedRect,
    force_rectangular_clip: bool,
) -> bool {
    if force_rectangular_clip {
        clip.push_clip(content);
        true
    } else {
        clip.push_rounded_clip(content)
    }
}

pub(super) fn render_nested_container(
    content: &mut String,
    child: &Container,
    child_index: usize,
    flow: &ContainerFlowContext<'_>,
    position: FlowPosition,
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
    } = position;
    let mut y;
    let nested_kids = &child.children;
    let background_color = &child.paint.background.color;
    let background_gradient = &child.paint.background.layers.gradient;
    let background_radial_gradient = &child.paint.background.layers.radial_gradient;
    let background_conic_gradient = &child.paint.background.layers.conic_gradient;
    let border = &child.box_model.border;
    let padding = &child.box_model.padding;
    let margin_top = &child.box_model.margins.start;
    let margin_bottom = &child.box_model.margins.end;
    let block_width = &child.box_model.size.width;
    let nk_block_height = &child.box_model.size.height;
    let nk_stacking_context = &child.paint.group.effects.stacking_context;
    let nk_bg_blend = &child.paint.background.blend_mode;
    let nk_visible = &child.paint.visible;
    let nk_float = &child.flow.float;
    let nk_clear = &child.flow.clear;
    let overflow = &child.overflow.combined;
    let nk_overflow_x = &child.overflow.x;
    let nk_overflow_y = &child.overflow.y;
    let nk_position = &child.positioning.scheme;
    let nk_offset_top = &child.positioning.insets.top;
    let nk_offset_left = &child.positioning.insets.left;
    let nk_box_transform = &child.paint.group.transform;
    let nk_transform = &nk_box_transform.value;
    let nk_box_shadow = &child.paint.shadows;
    let nk_bg_svg = ctx
        .text
        .pdf_writer
        .resolve_background_svg(&child.paint.background.layers);
    let nk_bg_svg = &nk_bg_svg;
    let nk_bg_size = &child.paint.background.layers.size;
    let nk_bg_position = &child.paint.background.layers.position;
    let nk_bg_repeat = &child.paint.background.layers.repeat;
    let nk_bg_origin = &child.paint.background.layers.origin;
    let nk_bg_clip = &child.paint.background.layers.clip;
    let nk_bg_blur = &child.paint.background.layers.blur_radius;
    let nk_outline_width = &child.paint.outline.width;
    let nk_outline_color = &child.paint.outline.color;
    let nk_outline_offset = &child.paint.outline.offset;
    let cont_radii = &child.paint.border_radii;
    let nk_positioned_depth = &child.positioning.containing_block_depth;
    let nk_containing_block = &child.positioning.containing_block;
    // Absolute-positioned containers (e.g. an empty position:absolute
    // div) must render at their inset offset from the containing
    // block's padding box, mirroring the TextBlock abspos arm — not
    // in normal flow. Without this, nested abspos boxes rendered at
    // the parent's content-box origin (top/left silently dropped).
    let nk_is_abs = nk_position.is_absolute();
    // In-flow containers collapse their margin-top against the
    // previous in-flow sibling's margin-bottom; floats and absolutes
    // are out of flow and take their margin-top in full.
    let nk_is_float = !nk_is_abs && *nk_float != Float::None;
    let nk_in_flow = !nk_is_abs && !nk_is_float;
    let planned_flow_top = flow_top_by_index.get(&child_index).copied();
    if let Some(top) = planned_flow_top {
        y = top;
    } else if nk_in_flow {
        if *nk_clear != Clear::None {
            cursor_y = clear_cursor(
                cursor_y,
                *nk_clear,
                left_float_bottom,
                right_float_bottom,
                &mut prev_margin_bottom,
            );
        }
        cursor_y -= collapsed_margin_top_extra(*margin_top, prev_margin_bottom);
        y = cursor_y;
    } else if nk_is_float {
        // Floated container: pinned at its precomputed top (the flow
        // cursor at its source position); does not advance the cursor.
        let rel_top = float_top_by_index.get(&child_index).copied().unwrap_or(0.0);
        y = container_top_y - rel_top;
    } else {
        // Absolute: positioned from the container top below.
        y = cursor_y;
    }
    let nk_w = block_width.resolve(width);
    let nk_children_h: f32 = collapsed_children_height(nested_kids);
    let nk_content_h = padding.vertical() + nk_children_h + border.vertical_width();
    let nk_total_h = nk_block_height.resolve(nk_content_h);
    // Absolute Containers anchor to their containing block's padding
    // box (resolved by depth, skipping static intermediates).
    let nk_anchor = abs_child_anchor(nk_containing_block, abs_origins, self_pad_origin);
    let normal_inline_offset = match nk_float {
        Float::Right => width - nk_w,
        _ => 0.0,
    };
    let flow_origin = child.positioning.resolve_in_flow_origin(
        crate::types::Point::new(normal_inline_offset, container_top_y - y),
        crate::types::Size::new(nk_w, nk_total_h),
        flow.frame.size,
    );
    let (nk_x, nk_top_y) = if nk_is_abs {
        (nk_anchor.x + nk_offset_left, nk_anchor.y - nk_offset_top)
    } else {
        (x + flow_origin.x, container_top_y - flow_origin.y)
    };
    // A definite `block_height` (set only for an explicit `height`)
    // is a hard border-box size: per CSS, oversized content overflows
    // the box (clipped or visible per `overflow`) rather than growing
    // it. Honour the declared height directly regardless of `overflow`
    // — only an auto height (`None`) expands to fit children. (The old
    // `content_h.max(h)` for non-hidden overflow wrongly inflated the
    // box to the child height, e.g. an `overflow:visible` box grew to
    // its oversized child instead of letting the child spill out.)
    let nk_geometry = LayoutBoxGeometry::from_layout(
        PdfRect::from_top(nk_x, nk_top_y, nk_w, nk_total_h),
        border,
        *padding,
        child.paint.border_image.as_ref(),
    );
    let nk_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(nk_geometry);
    let nk_paint_geometry = nk_box_geometry.painting();
    let nk_fragment_geometry = nk_box_geometry.fragment(child.fragmentation);
    let nk_border_box = nk_paint_geometry.rounded_border_box(*cont_radii);
    let background_geometry =
        nk_fragment_geometry.background(*nk_bg_origin, *nk_bg_clip, *cont_radii);
    let nk_gradient_reference = background_geometry.positioning_area.generated_image_box();
    let nk_image_reference = background_geometry.positioning_area.intrinsic_image_box();
    let nk_background_clip = background_geometry.painting_box;
    let nk_gradient_area = LayerPaintArea::new(
        nk_gradient_reference,
        background_geometry.image_destination_box,
    );
    let force_background_clip = child.fragmentation.reference_slice.is_some()
        || *nk_bg_clip != BackgroundClip::Border
        || nk_background_clip != nk_border_box;

    // CSS `filter: blur()` on a solid box (css-filter-effects-1 §4.1):
    // rasterize this empty container's bg fill + border, gaussian-blur
    // it, and embed it overflowing the border box. Restricted to a
    // plain solid box (no children, no gradient/SVG bg, no
    // transform/opacity/clip/mask wrapper, square corners) so the
    // vector paint path is byte-unchanged for everything else.
    if phase == ElementPaintPhase::All
        && *nk_visible
        && *nk_bg_blur > 0.0
        && nested_kids.is_empty()
        && background_gradient.is_none()
        && background_radial_gradient.is_none()
        && background_conic_gradient.is_none()
        && nk_bg_svg.is_none()
        && nk_transform.is_none()
        && cont_radii.is_zero()
        && *nk_outline_width == 0.0
        && let Some(blurred) = crate::render::blur::blur_box(
            nk_paint_geometry.border_box.width,
            nk_paint_geometry.border_box.height,
            *background_color,
            border,
            *nk_bg_blur,
            ctx.text.pdf_writer.opts.raster_quality.filter_dpi,
        )
    {
        let group = PaintGroupScope::begin(content, child, nk_fragment_geometry, ctx);
        let ov = blurred.overflow_pt;
        let img_obj_id = ctx.text.pdf_writer.add_image_object(
            &blurred.asset.data,
            blurred.asset.source_width,
            blurred.asset.source_height,
            blurred.asset.format,
            blurred.asset.png_metadata.as_ref(),
        );
        let img_name = format!("Im{img_obj_id}");
        content.push_str(&format!(
            "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
            w = nk_paint_geometry.border_box.width + 2.0 * ov,
            h = nk_paint_geometry.border_box.height + 2.0 * ov,
            ix = nk_paint_geometry.border_box.left - ov,
            iy = nk_paint_geometry.border_box.bottom - ov,
            name = img_name,
        ));
        ctx.text.page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
        group.finish(content, ctx);
        // Advance the flow cursor exactly as the normal container path
        // (the filter does not change layout).
        if nk_is_float {
            prev_margin_bottom = 0.0;
        } else if planned_flow_top.is_some() {
            prev_margin_bottom = *margin_bottom;
        } else if !nk_is_abs {
            cursor_y -= nk_total_h + margin_bottom;
            y = cursor_y;
            prev_margin_bottom = *margin_bottom;
        }
        return FlowPosition::new(y, cursor_y, prev_margin_bottom);
    }

    // `visibility: hidden` keeps the box's space (cursor still
    // advances below). Per CSS2 §11.2 it suppresses only THIS box's own
    // painting — a `visibility: visible` descendant must still render —
    // so the subtree (wrappers + children) is always emitted; the
    // box's own decoration is gated on `nk_visible` further down.
    {
        if phase == ElementPaintPhase::All
            && *nk_visible
            && device_space_available
            && let Some(t) = nk_transform
            && !is_projected_transform(t)
            && background_gradient.is_none()
            && background_radial_gradient.is_none()
            && background_conic_gradient.is_none()
            && nk_bg_svg.is_none()
            && *nk_bg_blur == 0.0
            && cont_radii.is_zero()
            && *nk_outline_width == 0.0
            && nk_box_shadow.is_empty()
            && child.paint.group.effects.is_identity()
            && !overflow.clips()
            && {
                render_affine_solid_group(
                    content,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    AffineSolidGroup {
                        transform: nk_box_transform,
                        geometry: nk_paint_geometry,
                        background: *background_color,
                        border,
                        children: nested_kids,
                    },
                )
            }
        {
            if nk_is_float {
                prev_margin_bottom = 0.0;
            } else if planned_flow_top.is_some() {
                prev_margin_bottom = *margin_bottom;
            } else if !nk_is_abs {
                cursor_y -= nk_total_h + margin_bottom;
                y = cursor_y;
                prev_margin_bottom = *margin_bottom;
            }
            return FlowPosition::new(y, cursor_y, prev_margin_bottom);
        }
        if phase == ElementPaintPhase::All
            && *nk_visible
            && device_space_available
            && let Some(t) = nk_transform
            && !is_projected_transform(t)
            && projected_solid_children_are_empty(nested_kids)
            && background_gradient.is_none()
            && background_radial_gradient.is_none()
            && background_conic_gradient.is_none()
            && nk_bg_svg.is_none()
            && *nk_bg_blur == 0.0
            && cont_radii.is_zero()
            && *nk_outline_width == 0.0
            && nk_box_shadow.is_empty()
            && child.paint.group.effects.is_identity()
            && !overflow.clips()
            && {
                render_affine_solid_box(
                    content,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    nk_box_transform,
                    nk_paint_geometry,
                    *background_color,
                    border,
                )
            }
        {
            if nk_is_float {
                prev_margin_bottom = 0.0;
            } else if planned_flow_top.is_some() {
                prev_margin_bottom = *margin_bottom;
            } else if !nk_is_abs {
                cursor_y -= nk_total_h + margin_bottom;
                y = cursor_y;
                prev_margin_bottom = *margin_bottom;
            }
            return FlowPosition::new(y, cursor_y, prev_margin_bottom);
        }
        if phase == ElementPaintPhase::All
            && *nk_visible
            && let Some(t) = nk_transform
            && is_projected_transform(t)
            && projected_solid_children_are_empty(nested_kids)
            && background_gradient.is_none()
            && background_radial_gradient.is_none()
            && background_conic_gradient.is_none()
            && nk_bg_svg.is_none()
            && *nk_bg_blur == 0.0
            && cont_radii.is_zero()
            && *nk_outline_width == 0.0
            && nk_box_shadow.is_empty()
            && child.paint.group.effects.is_identity()
            && *nk_bg_blend == crate::style::computed::BlendMode::Normal
            && *nk_bg_clip == BackgroundClip::Border
            && !overflow.clips()
        {
            render_projected_solid_box(
                content,
                ctx.text.pdf_writer.page_content_transform,
                nk_box_transform,
                nk_paint_geometry,
                *background_color,
                border,
            );
            if nk_is_float {
                prev_margin_bottom = 0.0;
            } else if planned_flow_top.is_some() {
                prev_margin_bottom = *margin_bottom;
            } else if !nk_is_abs {
                cursor_y -= nk_total_h + margin_bottom;
                y = cursor_y;
                prev_margin_bottom = *margin_bottom;
            }
            return FlowPosition::new(y, cursor_y, prev_margin_bottom);
        }
        let nk_group = PaintGroupScope::begin(content, child, nk_fragment_geometry, ctx);

        // CSS2 §11.2: self-decoration (background / border / outline /
        // shadow) is suppressed when this box is `visibility: hidden`,
        // but the opacity/transform/clip wrappers and the children
        // (which may override back to `visible`) are still emitted.
        if phase.paints_decoration() && *nk_visible {
            // Draw outset box-shadow (before the background, so it sits
            // behind the element). Nested containers previously dropped
            // box-shadow entirely; the top-level Container arm handles it
            // the same way.
            render_box_shadows(
                content,
                nk_box_shadow,
                nk_fragment_geometry,
                *cont_radii,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                ctx.text.pdf_writer,
            );

            // Draw the solid layer through the same resolved background
            // geometry used by gradients and images. In particular, this
            // preserves the paint-only bleed inset below opaque rounded
            // borders for nested boxes instead of reverting to the raw
            // border box.
            if let Some(color) = background_color {
                paint_solid_background(
                    content,
                    *color,
                    nk_background_clip,
                    ctx.page_ext_gstates,
                    ctx.bg_alpha_counter,
                );
            }

            // `background-blend-mode`: the background image layers (gradient /
            // SVG) blend against the background color painted above. Scope the
            // blend gstate to a `q`..`Q` around each background-image paint.
            let nk_bg_blend_mode = nk_bg_blend.background_layer(0);
            let nk_bg_blended = nk_bg_blend_mode != crate::style::computed::BlendMode::Normal;
            let nk_layer_box = background_layer_box(*nk_bg_size, *nk_bg_position, *nk_bg_repeat);

            // Draw linear gradient
            if let Some(gradient) = background_gradient {
                let gradient = linear_with_background_layer(gradient, nk_layer_box);
                if nk_bg_blended {
                    content.push_str("q\n");
                    begin_blend_mode(content, ctx.page_ext_gstates, nk_bg_blend_mode);
                }
                let rounded_clip =
                    push_nested_background_clip(content, nk_background_clip, force_background_clip);
                render_linear_gradient(
                    content,
                    &gradient,
                    GradientBackdrop::isolated_linear_layer(
                        *background_color,
                        background_radial_gradient.is_some()
                            || background_conic_gradient.is_some()
                            || nk_bg_svg.is_some(),
                        nk_bg_blend_mode,
                    ),
                    nk_gradient_area,
                    ctx.shadings,
                    ctx.shading_counter,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
                if rounded_clip {
                    content.push_str("Q\n");
                }
                if nk_bg_blended {
                    content.push_str("Q\n");
                }
            }

            // Draw radial gradient
            if let Some(gradient) = background_radial_gradient {
                let gradient = radial_with_background_layer(gradient, nk_layer_box);
                if nk_bg_blended {
                    content.push_str("q\n");
                    begin_blend_mode(content, ctx.page_ext_gstates, nk_bg_blend_mode);
                }
                let rounded_clip =
                    push_nested_background_clip(content, nk_background_clip, force_background_clip);
                render_radial_gradient(
                    content,
                    &gradient,
                    nk_gradient_area,
                    ctx.shadings,
                    ctx.shading_counter,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
                if rounded_clip {
                    content.push_str("Q\n");
                }
                if nk_bg_blended {
                    content.push_str("Q\n");
                }
            }

            // Draw conic gradient
            if let Some(gradient) = background_conic_gradient {
                let gradient = conic_with_background_layer(gradient, nk_layer_box);
                if nk_bg_blended {
                    content.push_str("q\n");
                    begin_blend_mode(content, ctx.page_ext_gstates, nk_bg_blend_mode);
                }
                let rounded_clip =
                    push_nested_background_clip(content, nk_background_clip, force_background_clip);
                render_conic_gradient(
                    content,
                    &gradient,
                    nk_gradient_area,
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                );
                if rounded_clip {
                    content.push_str("Q\n");
                }
                if nk_bg_blended {
                    content.push_str("Q\n");
                }
            }

            // Draw SVG background image if specified
            if let Some(svg_tree) = nk_bg_svg {
                if nk_bg_blended {
                    content.push_str("q\n");
                    begin_blend_mode(content, ctx.page_ext_gstates, nk_bg_blend_mode);
                }
                render_svg_background(
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
                    PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
                        nk_image_reference.into(),
                        background_geometry.image_destination_box.into(),
                        nk_background_clip.radii,
                        *nk_bg_blur,
                        *nk_bg_size,
                        *nk_bg_position,
                        *nk_bg_repeat,
                    )),
                );
                if nk_bg_blended {
                    content.push_str("Q\n");
                }
            }

            // Draw inset box-shadow (after the backgrounds, before the
            // borders/content) so it paints over the element fill.
            render_box_shadows_inset(
                content,
                nk_box_shadow,
                nk_fragment_geometry,
                *cont_radii,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                ctx.text.pdf_writer,
            );

            if border.has_visible() || child.paint.border_image.is_some() {
                paint_box_decoration(
                    content,
                    nk_fragment_geometry,
                    border,
                    *cont_radii,
                    child.paint.border_image.as_ref(),
                    BorderPaintResources::from_page(ctx),
                );
            }

            // Draw outline if specified (a uniform stroke outside the
            // border box). `outline-offset` widens the gap between the
            // border edge and the outline; the stroke centerline sits half
            // the outline width beyond the offset edge so the outline stays
            // entirely outside the box. Mirrors the TextBlock outline arm.
            if *nk_outline_width > 0.0 {
                let gap = *nk_outline_offset + *nk_outline_width / 2.0;
                let (or, og, ob) = nk_outline_color
                    .unwrap_or(crate::types::Color::BLACK)
                    .to_f32_rgb();
                content.push_str(&format!("{or} {og} {ob} RG\n{nk_outline_width} w\n"));
                content.push_str(
                    &nk_paint_geometry
                        .border_box
                        .outset_uniform(gap)
                        .rounded(cont_radii.grow(gap))
                        .path_or_rect(),
                );
                content.push_str("S\n");
            }
        } // end `if *nk_visible` — nested container self-decoration

        // Decide print scrollbars (css-overflow-3): a `scroll` axis
        // always reserves a gutter and paints a (non-interactive)
        // scrollbar; an `auto` axis does so only when its content
        // overflows. Chrome renders these in print, insetting the
        // content clip by the gutter on each scrolling axis.
        let padding_box = nk_geometry.padding_box();
        let paint_padding_box = nk_paint_geometry.padding_box();
        let content_box = nk_geometry.content_box();
        let content_avail_w = content_box.width;
        let content_avail_h = content_box.height;
        let (over_w, over_h) = children_overflow_extent(nested_kids);
        let over_ratio_h = if content_avail_w > 0.0 {
            over_w / content_avail_w
        } else {
            0.0
        };
        let over_ratio_v = if content_avail_h > 0.0 {
            over_h / content_avail_h
        } else {
            0.0
        };
        // No rounded scrollbars: a rounded box clips its scrollbar
        // chrome away, so only paint on square scroll containers.
        let scroll_ok = cont_radii.is_zero();
        let has_v = scroll_ok
            && match nk_overflow_y {
                Overflow::Scroll => true,
                Overflow::Auto => over_ratio_v > 1.001,
                _ => false,
            };
        let has_h = scroll_ok
            && match nk_overflow_x {
                Overflow::Scroll => true,
                Overflow::Auto => over_ratio_h > 1.001,
                _ => false,
            };
        let sb = SCROLLBAR_THICKNESS_PT;
        let v_gutter = if has_v { sb } else { 0.0 };
        let h_gutter = if has_h { sb } else { 0.0 };
        let scrollport_w = (content_avail_w - v_gutter).max(0.0);
        let scrollport_h = (content_avail_h - h_gutter).max(0.0);
        let thumb_ratio_h = if scrollport_w > 0.0 {
            over_w / scrollport_w
        } else {
            over_ratio_h
        };
        let thumb_ratio_v = if scrollport_h > 0.0 {
            over_h / scrollport_h
        } else {
            over_ratio_v
        };

        // Clip if overflow clips (hidden/clip/scroll/auto). CSS clips
        // at the PADDING box (border box inset by the border widths)
        // and follows the rounded corners when border-radius is set.
        // Scroll containers inset the clip by the reserved gutter so
        // content does not paint under the scrollbar.
        let clip = overflow.clips();
        let content_clip = clip.then(|| {
            let path = if has_v || has_h {
                // Rectangular clip inset by the per-side border and the
                // reserved gutter (right gutter for vertical, bottom for
                // horizontal — matching the LTR/top-anchored UA layout).
                let scroll_clip = paint_padding_box.inset(EdgeSizes {
                    right: v_gutter,
                    bottom: h_gutter,
                    ..Default::default()
                });
                scroll_clip.rect_path()
            } else {
                nk_paint_geometry
                    .rounded_padding_box(*cont_radii)
                    .path_or_rect()
            };
            ContentClip::from_path(path)
        });
        if let Some(clip) = &content_clip {
            clip.begin(content, &mut ctx.stacking);
        }

        // Recurse into nested children
        let inner_x = content_box.left;
        let inner_w = content_box.width;
        let inner_y = content_box.top();
        // Record this box's padding-box origin keyed by its
        // positioned depth so absolutely-positioned descendants nested
        // inside static intermediates anchor here (their CB), not to
        // the static container they are physically nested in.
        if *nk_positioned_depth > 0
            && (nk_position.is_positioned()
                || nk_transform.is_some()
                || *nk_stacking_context == StackingContext::Filter)
        {
            abs_origins.insert(
                *nk_positioned_depth,
                PdfPoint::new(padding_box.left, padding_box.top()),
            );
        }
        render_container_children(
            content,
            nested_kids,
            ContainerFrame::new(
                PdfPoint::new(inner_x, inner_y),
                crate::types::Size::new(inner_w, content_box.height),
                PdfPoint::new(padding_box.left, padding_box.top()),
            ),
            abs_origins,
            ctx,
            ContainerRenderOptions {
                device_space_available: device_space_available && nk_transform.is_none(),
                paint_phase: phase,
                stacking_scope: StackingScope::for_element(child),
            },
        );

        if let Some(clip) = &content_clip {
            clip.finish(content, &mut ctx.stacking);
        }

        // Paint the print scrollbar chrome in the reserved gutter,
        // AFTER the content clip is closed (the gutter lies outside
        // the inset content clip) but inside the box decoration group.
        if phase.paints_decoration() && (has_v || has_h) {
            paint_scrollbars(
                content,
                paint_padding_box.left,
                paint_padding_box.bottom,
                paint_padding_box.width,
                paint_padding_box.height,
                has_v,
                has_h,
                thumb_ratio_v.max(1.0),
                thumb_ratio_h.max(1.0),
            );
        }
        nk_group.finish(content, ctx);
    } // end nested-container subtree (wrappers + children)
    // Out-of-flow containers (absolute / float) don't advance the
    // flow cursor. A float's bottom is tracked via the simulator for
    // later `clear` siblings; it breaks the margin-collapse chain.
    if nk_is_float {
        prev_margin_bottom = 0.0;
    } else if planned_flow_top.is_some() {
        prev_margin_bottom = *margin_bottom;
    } else if !nk_is_abs {
        cursor_y -= nk_total_h + margin_bottom;
        y = cursor_y;
        // Remember this in-flow block's margin-bottom so the next
        // sibling collapses against it; floats don't collapse.
        prev_margin_bottom = *margin_bottom;
    }

    FlowPosition::new(y, cursor_y, prev_margin_bottom)
}
