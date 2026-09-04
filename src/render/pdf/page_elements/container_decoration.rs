use super::*;
use crate::layout::elements::Container;

pub(super) fn paint_container_decoration(
    content: &mut String,
    element: &Container,
    frame: PageElementFrame<'_>,
    geometry: FragmentPaintGeometry,
    ctx: &mut PageRenderContext<'_>,
) {
    let background_color = &element.paint.background.color;
    let border = &element.box_model.border;
    let c_border_radii = &element.paint.border_radii;
    let c_outline_width = &element.paint.outline.width;
    let c_outline_color = &element.paint.outline.color;
    let c_outline_offset = &element.paint.outline.offset;
    let c_visible = &element.paint.visible;
    let c_box_shadow = &element.paint.shadows;
    let c_bg_gradient = &element.paint.background.layers.gradient;
    let c_bg_radial = &element.paint.background.layers.radial_gradient;
    let c_bg_conic = &element.paint.background.layers.conic_gradient;
    let c_bg_svg = ctx
        .text
        .pdf_writer
        .resolve_background_svg(&element.paint.background.layers);
    let c_bg_svg = &c_bg_svg;
    let c_bg_size = &element.paint.background.layers.size;
    let c_bg_position = &element.paint.background.layers.position;
    let c_bg_repeat = &element.paint.background.layers.repeat;
    let c_bg_origin = &element.paint.background.layers.origin;
    let c_bg_clip = &element.paint.background.layers.clip;
    let c_bg_blend = &element.paint.background.blend_mode;
    let c_bg_blur = &element.paint.background.layers.blur_radius;
    let page_size = frame.page_size;
    let elem_idx = frame.element_index;
    let c_geometry = geometry.painting();
    let container_x = c_geometry.border_box.left;
    let container_y_top = c_geometry.border_box.top();
    let container_w = c_geometry.border_box.width;
    let total_h = c_geometry.border_box.height;
    let container_box = c_geometry.rounded_border_box(*c_border_radii);
    let c_visible_self = *c_visible;
    let transformed_paint_space = ctx.text.pdf_writer.transformed_paint_space(ctx.paint_box);

    // Self-decoration (background / border / outline / shadow) is
    // suppressed when this box is `visibility: hidden`; children
    // (which may override back to visible) are still rendered below.
    if c_visible_self {
        // Draw box-shadow with blur
        render_box_shadows(
            content,
            c_box_shadow,
            geometry,
            *c_border_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // The box `background-clip` confines the painted fill to.
        let background_geometry = geometry.background(*c_bg_origin, *c_bg_clip, *c_border_radii);
        let c_clip_box = background_geometry.painting_box;
        let c_image_destination = background_geometry.image_destination_box;
        let c_gradient_reference = background_geometry.positioning_area.generated_image_box();
        let c_image_reference = background_geometry.positioning_area.intrinsic_image_box();
        let c_needs_clip = *c_bg_clip != BackgroundClip::Border || c_clip_box != container_box;
        let gradient_area = |attachment| {
            if attachment == Some(BackgroundAttachment::Fixed) {
                LayerPaintArea::new(
                    PdfRect::new(0.0, 0.0, page_size.width, page_size.height),
                    c_clip_box.rect,
                )
            } else {
                LayerPaintArea::new(c_gradient_reference, c_image_destination)
            }
        };

        // Draw background
        if let Some(background) = background_color {
            let (r, g, b, a) = background.to_f32_rgba();
            let needs_alpha = a < 1.0;
            if needs_alpha {
                let gs_name = format!("GScontainer{elem_idx}");
                ctx.page_ext_gstates.push((gs_name.clone(), a));
                content.push_str(&format!("/{gs_name} gs\n"));
            }
            let color = PdfRgb::from((r, g, b));
            let uses_device_css_clip = transformed_paint_space.is_none()
                && is_device_clippable_box_background(
                    *c_bg_clip,
                    c_clip_box.radii,
                    ctx.text.pdf_writer.page_content_transform,
                    c_clip_box.rect,
                )
                && paint_device_clipped_css_solid(
                    content,
                    ctx.text.pdf_writer.page_content_transform,
                    container_box.rect,
                    c_clip_box.rect,
                    color,
                );
            if !uses_device_css_clip && c_needs_clip {
                content.push_str(&color.fill_operator());
                // Clip the fill to the clip box; a non-uniform
                // rounded fill cannot also be clipped, so fall
                // back to a rectangular clip-box fill.
                c_clip_box.push_clip(content);
                content.push_str(&c_clip_box.rect.rect_path());
                content.push_str("f\n");
                content.push_str("Q\n");
            } else if !uses_device_css_clip {
                content.push_str(&color.fill_operator());
                content.push_str(&container_box.path_or_rect());
                content.push_str("f\n");
            }
            if needs_alpha {
                content.push_str("/GSDefault gs\n");
            }
        }

        // Gradients are positioned in the background-origin box;
        // background-clip only confines where that paint shows.
        let gradient_clip = *c_bg_clip != BackgroundClip::Text;
        let c_layer_box = background_layer_box(*c_bg_size, *c_bg_position, *c_bg_repeat);
        let c_bg_blend_mode = c_bg_blend.background_layer(0);
        let c_bg_blended = c_bg_blend_mode != crate::style::computed::BlendMode::Normal;
        // Draw container linear gradient
        if let Some(gradient) = c_bg_gradient
            && !gradient.layer_box.paint_above_raster
            && gradient.layer_box.attachment != Some(BackgroundAttachment::Local)
        {
            let gradient = linear_with_background_layer(gradient, c_layer_box);
            if c_bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, c_bg_blend_mode);
            }
            if gradient_clip {
                c_clip_box.push_clip(content);
            }
            render_linear_gradient(
                content,
                &gradient,
                GradientBackdrop::isolated_linear_layer(
                    *background_color,
                    c_bg_radial.is_some() || c_bg_conic.is_some() || c_bg_svg.is_some(),
                    c_bg_blend_mode,
                ),
                gradient_area(gradient.layer_box.attachment),
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if gradient_clip {
                content.push_str("Q\n");
            }
            if c_bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw container radial gradient
        if let Some(gradient) = c_bg_radial
            && gradient.layer_box.attachment != Some(BackgroundAttachment::Local)
        {
            let gradient = radial_with_background_layer(gradient, c_layer_box);
            if c_bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, c_bg_blend_mode);
            }
            if gradient_clip {
                c_clip_box.push_clip(content);
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
            if gradient_clip {
                content.push_str("Q\n");
            }
            if c_bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw container conic gradient
        if let Some(gradient) = c_bg_conic
            && gradient.layer_box.attachment != Some(BackgroundAttachment::Local)
        {
            let gradient = conic_with_background_layer(gradient, c_layer_box);
            if c_bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, c_bg_blend_mode);
            }
            if gradient_clip {
                c_clip_box.push_clip(content);
            }
            render_conic_gradient(
                content,
                &gradient,
                gradient_area(gradient.layer_box.attachment),
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if gradient_clip {
                content.push_str("Q\n");
            }
            if c_bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw SVG / raster background image if specified.
        // `background-origin` sets the positioning area; the
        // reference box is derived from the BORDER box by
        // insetting the border (padding box) and border+padding
        // (content box).
        if let Some(svg_tree) = c_bg_svg {
            if c_bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, c_bg_blend_mode);
            }
            let paint = BackgroundPaintContext::new(
                c_image_reference.into(),
                c_image_destination.into(),
                c_clip_box.radii,
                *c_bg_blur,
                *c_bg_size,
                *c_bg_position,
                *c_bg_repeat,
            );
            let paint = transformed_paint_space.map_or_else(
                || PdfBackgroundPaintContext::local(paint),
                |paint_space| PdfBackgroundPaintContext::in_default_space(paint, paint_space),
            );
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
                paint,
            );
            if c_bg_blended {
                content.push_str("Q\n");
            }
        }
        if let Some(gradient) = c_bg_gradient
            && gradient.layer_box.paint_above_raster
            && gradient.layer_box.attachment != Some(BackgroundAttachment::Local)
        {
            let gradient = linear_with_background_layer(gradient, c_layer_box);
            if c_bg_blended {
                content.push_str("q\n");
                begin_blend_mode(content, ctx.page_ext_gstates, c_bg_blend_mode);
            }
            if gradient_clip {
                c_clip_box.push_clip(content);
            }
            render_linear_gradient(
                content,
                &gradient,
                GradientBackdrop::isolated_linear_layer(
                    *background_color,
                    c_bg_radial.is_some() || c_bg_conic.is_some() || c_bg_svg.is_some(),
                    c_bg_blend_mode,
                ),
                gradient_area(gradient.layer_box.attachment),
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if gradient_clip {
                content.push_str("Q\n");
            }
            if c_bg_blended {
                content.push_str("Q\n");
            }
        }

        // Draw inset box-shadow (after container background, before borders).
        render_box_shadows_inset(
            content,
            c_box_shadow,
            geometry,
            *c_border_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // Missing fragment edges are zero-width sides in the same border ring
        // used by complete boxes. This keeps the cut open without a second
        // fragment-only border implementation.
        if border.has_visible() || element.paint.border_image.is_some() {
            paint_box_decoration(
                content,
                geometry,
                border,
                *c_border_radii,
                element.paint.border_image.as_ref(),
                BorderPaintResources::from_page(ctx),
            );
        }

        // Draw outline (outside the border box, honouring
        // `outline-offset`). Top-level containers previously dropped
        // the outline entirely.
        if *c_outline_width > 0.0 {
            let gap = *c_outline_offset + *c_outline_width / 2.0;
            let ol_x = container_x - gap;
            let ol_y = container_y_top - total_h - gap;
            let ol_w = container_w + 2.0 * gap;
            let ol_h = total_h + 2.0 * gap;
            let (or, og, ob) = c_outline_color
                .unwrap_or(crate::types::Color::BLACK)
                .to_f32_rgb();
            content.push_str(&format!("{or} {og} {ob} RG\n{c_outline_width} w\n"));
            content.push_str(
                &RoundedRect::new(
                    PdfRect::new(ol_x, ol_y, ol_w, ol_h),
                    c_border_radii.grow(gap),
                )
                .path_or_rect(),
            );
            content.push_str("S\n");
        }
    } // end `if c_visible_self` — container self-decoration

    // Print scrollbars (css-overflow-3): a `scroll` axis always
    // reserves a gutter + paints a non-interactive scrollbar; an
    // `auto` axis does so only when its content overflows. The
    // content clip is inset by the gutter on each scrolling axis.
}
