use super::*;
use crate::layout::elements::{Image, PaintGroupOwner, Svg};
use crate::render::pdf::structure::MarkKind;
use std::borrow::Cow;

/// A Figure mark whose `/BBox` is the painted box in default user space.
fn figure_mark(paint_box: PdfRect, pdf_writer: &PdfWriter) -> MarkKind {
    MarkKind::Figure {
        bbox: pdf_writer.page_content_transform.transform_rect(paint_box),
    }
}

/// Final source and destination used to paint one replaced raster.
///
/// Edge-preserving images can discard an exactly aligned invisible source
/// region. This is both smaller and avoids antialiasing an otherwise redundant
/// PDF clip across hard source-pixel boundaries.
struct ReplacedRasterPaint<'a> {
    source: Cow<'a, crate::layout::engine::RasterImageAsset>,
    bounds: PdfRect,
    clip: bool,
}

impl<'a> ReplacedRasterPaint<'a> {
    fn resolve(
        source: &'a crate::layout::engine::RasterImageAsset,
        content: PdfRect,
        sampling: crate::layout::elements::ImageSampling,
    ) -> Self {
        let source_content_size = sampling.replaced.fragment.map_or(
            crate::types::Size::new(content.width, content.height),
            |fragment| fragment.source_content_size,
        );
        let placement = crate::layout::images::compute_image_placement(
            source_content_size.width,
            source_content_size.height,
            source.source_width,
            source.source_height,
            sampling.replaced.object_fit,
            sampling.replaced.object_position,
        );
        if sampling.replaced.fragment.is_none()
            && sampling.rendering.preserves_source_edges()
            && placement.clip
            && let Some(paint) = Self::cropped_to_visible_pixels(source, content, placement)
        {
            return paint;
        }

        let fragment_offset = sampling
            .replaced
            .fragment
            .map_or(crate::types::Vector::ZERO, |fragment| {
                fragment.content_offset
            });
        Self {
            source: Cow::Borrowed(source),
            bounds: PdfRect::new(
                content.left + placement.offset_x - fragment_offset.x,
                content.top() - placement.offset_y - placement.height + fragment_offset.y,
                placement.width,
                placement.height,
            ),
            clip: placement.clip || sampling.replaced.fragment.is_some(),
        }
    }

    fn cropped_to_visible_pixels(
        source: &crate::layout::engine::RasterImageAsset,
        content: PdfRect,
        placement: crate::layout::images::ImagePlacement,
    ) -> Option<Self> {
        let visible_left = placement.offset_x.max(0.0);
        let visible_top = placement.offset_y.max(0.0);
        let visible_right = (placement.offset_x + placement.width).min(content.width);
        let visible_bottom = (placement.offset_y + placement.height).min(content.height);
        let visible_width = visible_right - visible_left;
        let visible_height = visible_bottom - visible_top;
        if visible_width <= 0.0 || visible_height <= 0.0 {
            return None;
        }

        let source_rect = crate::types::Rect::from_xywh(
            (visible_left - placement.offset_x) * source.source_width as f32 / placement.width,
            (visible_top - placement.offset_y) * source.source_height as f32 / placement.height,
            visible_width * source.source_width as f32 / placement.width,
            visible_height * source.source_height as f32 / placement.height,
        );
        let crop = crate::layout::images::RasterCrop::aligned(
            source_rect,
            crate::util::RasterDimensions {
                width: source.source_width,
                height: source.source_height,
            },
        )?;
        let source = crate::layout::images::crop_raster_asset(source, crop)?;
        Some(Self {
            source: Cow::Owned(source),
            bounds: PdfRect::new(
                content.left + visible_left,
                content.top() - visible_top - visible_height,
                visible_width,
                visible_height,
            ),
            clip: false,
        })
    }
}

fn paint_image_effect_raster(
    content: &mut String,
    effect: &crate::layout::engine::ImageEffectRaster,
    image_box: PdfRect,
    rendering: crate::style::computed::ImageRendering,
    ctx: &mut PageRenderContext<'_>,
) {
    let effect_box = image_box.outset_uniform(effect.overflow);
    let image_id = ctx.text.pdf_writer.add_layout_image_object(
        &effect.image,
        effect_box.width,
        effect_box.height,
        rendering,
    );
    let image_name = format!("Im{image_id}");
    push_raster_xobject(
        content,
        &image_name,
        effect_box,
        &effect.image,
        ctx.text.pdf_writer,
    );
    ctx.text.page_images.push(ImageRef {
        name: image_name,
        obj_id: image_id,
    });
}

pub(in crate::render::pdf) fn render_image(
    content: &mut String,
    element: &Image,
    frame: PageElementFrame<'_>,
    ctx: &mut PageRenderContext<'_>,
) {
    let image_box = PdfRect::from_top(
        frame.margin.left + element.positioning.insets.left,
        frame.page_size.height - frame.margin.top - frame.y_pos - element.positioning.insets.top,
        element.geometry.size.width,
        element.geometry.size.height,
    );
    if ctx.text.pdf_writer.opts.occlusion_cull {
        let raster = if element.paint.raster_overflow.is_zero() {
            let overflow = element
                .paint
                .filter_effect
                .as_ref()
                .map_or(0.0, |effect| effect.overflow);
            image_box.outset_uniform(overflow)
        } else {
            image_box.outset(element.paint.raster_overflow)
        };
        if raster_is_occluded(frame.occlusion_coverers, raster, frame.element_index) {
            return;
        }
    }
    paint_image_box(content, element, image_box, ctx);
}

/// Paint one replaced raster box using the complete shared effects contract.
pub(in crate::render::pdf) fn paint_image_box(
    content: &mut String,
    element: &Image,
    image_box: PdfRect,
    ctx: &mut PageRenderContext<'_>,
) {
    let image = &element.source;
    let geometry = LayoutBoxGeometry::from_layout(
        image_box,
        &element.geometry.border,
        EdgeSizes::ZERO,
        element.paint.border_image.as_ref(),
    );
    let box_geometry = ctx.text.pdf_writer.resolve_box_geometry(geometry);
    let paint_geometry = box_geometry.painting();
    let fragment_geometry = box_geometry.fragment(Default::default());
    let paint_box = paint_geometry.border_box;
    let raster_overflow = element.paint.raster_overflow;
    if !raster_overflow.is_zero() {
        let expanded_box = paint_box.outset(raster_overflow);
        let group = PaintGroupScope::begin(content, element, fragment_geometry, ctx);
        let image_id = ctx.text.pdf_writer.add_image_object_with_interpolation(
            &image.data,
            image.source_width,
            image.source_height,
            image.format,
            image.png_metadata.as_ref(),
            PdfImageInterpolation::for_css_image_rendering(element.sampling.rendering),
        );
        let image_name = format!("Im{image_id}");
        let kind = figure_mark(expanded_box, ctx.text.pdf_writer);
        let figure =
            ctx.text
                .pdf_writer
                .structure
                .begin_content(content, element.paint_group().owner, kind);
        push_raster_xobject(
            content,
            &image_name,
            expanded_box,
            image,
            ctx.text.pdf_writer,
        );
        ctx.text.pdf_writer.structure.end(content, figure);
        ctx.text.page_images.push(ImageRef {
            name: image_name,
            obj_id: image_id,
        });
        group.finish(content, ctx);
        return;
    }

    let group = PaintGroupScope::begin(content, element, fragment_geometry, ctx);
    if let Some(effect) = &element.paint.filter_effect {
        paint_image_effect_raster(content, effect, paint_box, element.sampling.rendering, ctx);
    }
    if let Some(background) = element.paint.background_color
        && background.alpha() > 0.0
    {
        let background_box =
            paint_geometry.background_clip_box(BackgroundClip::Border, element.paint.border_radii);
        paint_solid_background(
            content,
            background,
            background_box,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
        );
    }

    let image_content = paint_geometry.padding_box();
    let paint = ReplacedRasterPaint::resolve(image, image_content, element.sampling);
    let image_id = ctx.text.pdf_writer.add_layout_image_object(
        paint.source.as_ref(),
        paint.bounds.width,
        paint.bounds.height,
        element.sampling.rendering,
    );
    let image_name = format!("Im{image_id}");
    let kind = figure_mark(paint.bounds, ctx.text.pdf_writer);
    let figure =
        ctx.text
            .pdf_writer
            .structure
            .begin_content(content, element.paint_group().owner, kind);
    if paint.clip {
        content.push_str("q\n");
        content.push_str(&format!("{}W n\n", image_content.rect_path()));
    }
    push_raster_xobject(
        content,
        &image_name,
        paint.bounds,
        paint.source.as_ref(),
        ctx.text.pdf_writer,
    );
    if paint.clip {
        content.push_str("Q\n");
    }
    ctx.text.pdf_writer.structure.end(content, figure);
    ctx.text.page_images.push(ImageRef {
        name: image_name,
        obj_id: image_id,
    });
    paint_box_decoration(
        content,
        fragment_geometry,
        &element.geometry.border,
        element.paint.border_radii,
        element.paint.border_image.as_ref(),
        BorderPaintResources::from_page(ctx),
    );
    group.finish(content, ctx);
}

pub(in crate::render::pdf) fn render_svg(
    content: &mut String,
    element: &Svg,
    frame: PageElementFrame<'_>,
    ctx: &mut PageRenderContext<'_>,
) {
    let page_size = frame.page_size;
    let margin = frame.margin;
    let y_pos = &frame.y_pos;
    let elem_idx = frame.element_index;
    let occlusion_coverers = frame.occlusion_coverers;
    let width = &element.geometry.size.width;
    let height = &element.geometry.size.height;
    let offset_top = &element.positioning.insets.top;
    let offset_left = &element.positioning.insets.left;
    let svg_x = margin.left + offset_left;
    // PDF y-axis is bottom-up, SVG is top-down
    let svg_y = page_size.height - margin.top - y_pos - height - offset_top;

    // Occlusion culling (default off): skip embedding the SVG (and
    // any rasters it would register) when a later opaque coverer
    // fully hides its box.
    if ctx.text.pdf_writer.opts.occlusion_cull {
        let raster = PdfRect::new(svg_x, svg_y, *width, *height);
        if raster_is_occluded(occlusion_coverers, raster, elem_idx) {
            return;
        }
    }
    paint_svg_box(
        content,
        element,
        PdfRect::new(svg_x, svg_y, *width, *height),
        ctx,
    );
}

/// Paint one replaced SVG box after layout has chosen its absolute PDF-space
/// rectangle. Page-flow and nested-flow callers deliberately share this
/// function so neither path can implement a reduced effects contract.
pub(in crate::render::pdf) fn paint_svg_box(
    content: &mut String,
    element: &Svg,
    svg_box: PdfRect,
    ctx: &mut PageRenderContext<'_>,
) {
    let geometry = LayoutBoxGeometry::from_layout(
        svg_box,
        &element.geometry.border,
        EdgeSizes::ZERO,
        element.paint.border_image.as_ref(),
    );
    let box_geometry = ctx.text.pdf_writer.resolve_box_geometry(geometry);
    let paint_geometry = box_geometry.painting();
    let fragment_geometry = box_geometry.fragment(Default::default());
    let group = PaintGroupScope::begin(content, element, fragment_geometry, ctx);
    if let Some(background) = element.paint.background_color
        && background.alpha() > 0.0
    {
        let background_box =
            paint_geometry.background_clip_box(BackgroundClip::Border, element.paint.border_radii);
        paint_solid_background(
            content,
            background,
            background_box,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
        );
    }

    let svg_content = paint_geometry.padding_box();
    let content_w = svg_content.width;
    let content_h = svg_content.height;
    let kind = figure_mark(svg_content, ctx.text.pdf_writer);
    let figure =
        ctx.text
            .pdf_writer
            .structure
            .begin_content(content, element.paint_group().owner, kind);
    content.push_str("q\n");
    // Position on page and flip y-axis for SVG coordinates
    content.push_str(&format!(
        "1 0 0 -1 {} {} cm\n",
        svg_content.left,
        svg_content.top()
    ));
    if let Some(placement) = crate::render::svg_geometry::compute_replaced_svg_placement(
        &element.tree,
        crate::types::Size::new(content_w, content_h),
        element.replaced,
    ) {
        content.push_str("q\n");
        content.push_str(&format!("0 0 {content_w} {content_h} re\nW n\n"));
        content.push_str(&format!(
            "{sx} 0 0 {sy} {tx} {ty} cm\n",
            sx = placement.scale_x,
            sy = placement.scale_y,
            tx = placement.translate_x,
            ty = placement.translate_y,
        ));
        {
            let font_policy = ctx.text.pdf_writer.structure.font_policy();
            let mut image_sink = SvgPageImageSink {
                pdf_writer: ctx.text.pdf_writer,
                page_images: ctx.text.page_images,
            };
            let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
                shadings: ctx.shadings,
                shading_counter: ctx.shading_counter,
                ext_gstates: Some(ctx.page_ext_gstates),
                image_sink: Some(&mut image_sink),
                raster_scale_x: placement.scale_x.abs(),
                raster_scale_y: placement.scale_y.abs(),
                custom_fonts: Some(ctx.text.custom_fonts),
                prepared_custom_fonts: Some(ctx.text.prepared_custom_fonts),
                font_policy,
            };
            crate::render::svg_to_pdf::render_svg_tree_with_resources(
                &element.tree,
                content,
                &mut resources,
            );
        }
        content.push_str("Q\n");
    }
    content.push_str("Q\n");
    ctx.text.pdf_writer.structure.end(content, figure);
    paint_box_decoration(
        content,
        fragment_geometry,
        &element.geometry.border,
        element.paint.border_radii,
        element.paint.border_image.as_ref(),
        BorderPaintResources::from_page(ctx),
    );
    group.finish(content, ctx);
}
