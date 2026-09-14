use super::backgrounds::{PdfBackgroundPaintContext, PdfBackgroundResources};
use super::geometry::{BoxPaintGeometry, PdfRect};
use super::patterns::{
    LayerPaintArea, LayerTilePattern, RepeatModes, paint_page_tiling_pattern, paint_tiling_pattern,
};
use super::{SvgPageImageSink, begin_blend_mode};
use crate::render::background::{
    BackgroundPaintContext, RasterBackgroundRequest, SvgVisualOverflow, overflow_from_viewport_box,
    register_background_image, svg_visual_overflow, synthetic_raster_background,
    viewport_box_from_overflow,
};
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{
    BackgroundClip, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize,
    BlendMode,
};
use crate::types::CornerRadii;
use crate::util::AxisRepeatPattern;

#[derive(Debug, Clone, Copy)]
pub(super) struct BlockBackground {
    pub(super) geometry: BoxPaintGeometry,
    pub(super) border_radii: CornerRadii,
    pub(super) size: BackgroundSize,
    pub(super) position: BackgroundPosition,
    pub(super) repeat: BackgroundRepeat,
    pub(super) origin: BackgroundOrigin,
    pub(super) clip: BackgroundClip,
    pub(super) blur_radius: f32,
    pub(super) blend_mode: BlendMode,
}

/// Map the complete visual cell from the SVG's top-down tile coordinates into
/// the PDF page's bottom-up coordinate system.
fn placed_tile_visual_box(
    tile: PdfRect,
    viewport: SvgViewportBox,
    overflow: SvgVisualOverflow,
) -> PdfRect {
    let visual = viewport_box_from_overflow(viewport, overflow);
    PdfRect::new(
        tile.left + visual.x,
        tile.top() - visual.y - visual.height,
        visual.width,
        visual.height,
    )
}

/// A no-repeat background whose complete visual cell misses the fragment clip
/// cannot contribute paint. Resolve this before allocating page resources so
/// continuation fragments do not retain invisible image or SVG objects.
fn single_tile_misses_clip(
    pattern: LayerTilePattern,
    tile: PdfRect,
    viewport: SvgViewportBox,
    clip: SvgViewportBox,
    overflow: SvgVisualOverflow,
) -> bool {
    if !pattern.is_single() {
        return false;
    }

    let tile = placed_tile_visual_box(tile, viewport, overflow);
    let clip = viewport_box_from_overflow(clip, overflow);
    let clip = PdfRect::new(clip.x, clip.y, clip.width, clip.height);
    tile.intersection(clip).is_none()
}

pub(super) fn render_block_svg_background(
    content: &mut String,
    tree: &crate::parser::svg::SvgTree,
    mut resources: PdfBackgroundResources<'_>,
    background: BlockBackground,
) {
    let geometry =
        background
            .geometry
            .background(background.origin, background.clip, background.border_radii);
    let clip = geometry.painting_box;
    let reference = geometry.positioning_area.intrinsic_image_box();
    let blended = background.blend_mode != BlendMode::Normal
        && resources.ext_gstates.as_deref_mut().is_some_and(|states| {
            content.push_str("q\n");
            begin_blend_mode(content, states, background.blend_mode);
            true
        });
    render_svg_background(
        content,
        tree,
        resources,
        PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
            reference.into(),
            geometry.image_destination_box.into(),
            clip.radii,
            background.blur_radius,
            background.size,
            background.position,
            background.repeat,
        )),
    );
    if blended {
        content.push_str("Q\n");
    }
}

pub(super) fn render_svg_background(
    content: &mut String,
    tree: &crate::parser::svg::SvgTree,
    resources: PdfBackgroundResources<'_>,
    paint: PdfBackgroundPaintContext,
) {
    let PdfBackgroundResources {
        writer: pdf_writer,
        images: page_images,
        shadings,
        shading_counter,
        ext_gstates,
        custom_fonts,
        prepared_custom_fonts,
    } = resources;
    let PdfBackgroundPaintContext {
        background: paint,
        paint_space,
    } = paint;
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
        BackgroundSize::Auto => {
            // SVG dimensions are in CSS pixels; convert to points (1px = 0.75pt)
            (intrinsic_width * 0.75, intrinsic_height * 0.75)
        }
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
        BackgroundSize::ExplicitAuto {
            width,
            height,
            width_is_percent,
            height_is_percent,
        } => match (width, height) {
            (Some(w), Some(h)) => (
                resolve_axis(w, width_is_percent, paint.reference_box.width),
                resolve_axis(h, height_is_percent, paint.reference_box.height),
            ),
            (Some(w), None) => {
                let scaled_w = resolve_axis(w, width_is_percent, paint.reference_box.width);
                (scaled_w, scaled_w * vb_h / vb_w)
            }
            (None, Some(h)) => {
                let scaled_h = resolve_axis(h, height_is_percent, paint.reference_box.height);
                (scaled_h * vb_w / vb_h, scaled_h)
            }
            (None, None) => (intrinsic_width * 0.75, intrinsic_height * 0.75),
        },
    };

    if scaled_w <= 0.0 || scaled_h <= 0.0 {
        return;
    }

    // When `background-size` fixes BOTH dimensions explicitly, the image is
    // scaled to exactly that box, ignoring its intrinsic aspect ratio
    // (css-backgrounds-3 §3.9). `cover`/`contain` already derive an
    // aspect-correct target box, so the source's `preserveAspectRatio` (which
    // would re-fit it) must be neutralised; only `auto` keeps the ratio.
    let stretch_to_box = matches!(
        paint.size,
        BackgroundSize::Cover
            | BackgroundSize::Contain
            | BackgroundSize::Explicit {
                height: Some(_),
                ..
            }
    );
    let placement_par = if stretch_to_box {
        crate::parser::svg::SvgPreserveAspectRatio::None
    } else {
        tree.preserve_aspect_ratio
    };
    // Compute background-position offset (in the CSS coordinate system,
    // origin at top-left of the element box).
    let offset_x = if paint.position.x_is_percent {
        (paint.reference_box.width - scaled_w) * paint.position.x
    } else if paint.position.x < 0.0 {
        (paint.reference_box.width - scaled_w) + paint.position.x
    } else {
        paint.position.x
    };
    let offset_y = if paint.position.y_is_percent {
        (paint.reference_box.height - scaled_h) * paint.position.y
    } else if paint.position.y < 0.0 {
        (paint.reference_box.height - scaled_h) + paint.position.y
    } else {
        paint.position.y
    };

    let repeat = RepeatModes::from(paint.repeat);
    let Some(x_pattern) = AxisRepeatPattern::new_layout(
        repeat.horizontal,
        offset_x,
        scaled_w,
        paint.reference_box.width,
    ) else {
        return;
    };
    let Some(y_pattern) = AxisRepeatPattern::new_layout(
        repeat.vertical,
        offset_y,
        scaled_h,
        paint.reference_box.height,
    ) else {
        return;
    };
    let (scaled_w, scaled_h) = (x_pattern.tile_size(), y_pattern.tile_size());
    let tile_pattern = LayerTilePattern::new(
        LayerPaintArea::new(
            PdfRect::new(
                paint.reference_box.x,
                paint.reference_box.y,
                paint.reference_box.width,
                paint.reference_box.height,
            ),
            PdfRect::new(
                paint.clip_box.x,
                paint.clip_box.y,
                paint.clip_box.width,
                paint.clip_box.height,
            ),
        ),
        x_pattern,
        y_pattern,
    );
    let Some(first_tile) = tile_pattern.first_tile() else {
        return;
    };
    let placement = crate::render::svg_geometry::compute_svg_placement(
        tree,
        crate::render::svg_geometry::SvgPlacementRequest::from_rect(
            0.0,
            0.0,
            scaled_w,
            scaled_h,
            placement_par,
        ),
    );
    let Some(placement) = placement else {
        return;
    };
    let source_visual_overflow =
        svg_visual_overflow(tree).scale(placement.scale_x, placement.scale_y);
    if paint.blur_radius <= 0.0
        && single_tile_misses_clip(
            tile_pattern,
            first_tile,
            placement.viewport,
            paint.clip_box,
            source_visual_overflow,
        )
    {
        return;
    }
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
            filter_dpi: pdf_writer.opts.raster_quality.filter_dpi,
        });
        register_background_image(pdf_writer, page_images, href, image_box, request)
            .map(|registered| (image_box, registered))
    });
    let visual_overflow = raster_background.as_ref().map_or_else(
        || source_visual_overflow,
        |(image_box, registered)| {
            overflow_from_viewport_box(
                placement.viewport,
                registered.draw_box.unwrap_or(*image_box),
            )
        },
    );
    let tile_clip_box = viewport_box_from_overflow(placement.viewport, visual_overflow);

    let mut cell = String::from("q\n");
    cell.push_str(&tile_clip_box.clip_path());
    if let Some((image_box, registered_image)) = &raster_background {
        let draw_box = registered_image.draw_box.unwrap_or(*image_box);
        cell.push_str(&format!(
            "q\n{width} 0 0 -{height} {x} {y} cm\n/{name} Do\nQ\n",
            width = draw_box.width,
            height = draw_box.height,
            x = draw_box.x,
            y = draw_box.y + draw_box.height,
            name = registered_image.name,
        ));
    } else {
        cell.push_str(&format!(
            "{sx} 0 0 {sy} {tx} {ty} cm\n",
            sx = placement.scale_x,
            sy = placement.scale_y,
            tx = placement.translate_x,
            ty = placement.translate_y,
        ));
        let mut image_sink = SvgPageImageSink {
            pdf_writer,
            page_images,
        };
        let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
            shadings,
            shading_counter,
            ext_gstates,
            image_sink: Some(&mut image_sink),
            raster_scale_x: placement.scale_x.abs(),
            raster_scale_y: placement.scale_y.abs(),
            // SVG used as a CSS background image: thread the caller's font
            // context through so custom-font `<text>` resolves registered
            // families exactly like foreground SVG text (standard fonts remain
            // the fallback when no context is wired up, e.g. in tests).
            custom_fonts,
            prepared_custom_fonts,
        };
        crate::render::svg_to_pdf::render_svg_tree_with_resources(tree, &mut cell, &mut resources);
    }
    cell.push_str("Q\n");

    // Clip to the element box.
    content.push_str("q\n");
    let expanded_clip_box = viewport_box_from_overflow(paint.clip_box, visual_overflow);
    content.push_str(
        &PdfRect::new(
            expanded_clip_box.x,
            expanded_clip_box.y,
            expanded_clip_box.width,
            expanded_clip_box.height,
        )
        .rounded(paint.border_radii)
        .path_or_rect(),
    );
    content.push_str("W n\n");

    if tile_pattern.is_single() {
        content.push_str("q\n");
        content.push_str(&format!(
            "1 0 0 -1 {} {} cm\n",
            first_tile.left,
            first_tile.top()
        ));
        content.push_str(&cell);
        content.push_str("Q\n");
    } else {
        if let Some((image_box, registered)) = &raster_background
            && registered.draw_box.is_none()
            && image_box.x == 0.0
            && image_box.y == 0.0
            && image_box.width == scaled_w
            && image_box.height == scaled_h
        {
            let source = registered.pixel_dimensions;
            if let Some(paint_space) = paint_space {
                if let Some(pattern) = tile_pattern.pdf_page_raster_pattern(source, paint_space) {
                    let stream = format!(
                        "q\n{width} 0 0 -{height} {left} {top} cm\n/{name} Do\nQ\n",
                        width = source.width,
                        height = source.height,
                        left = pattern.bbox.left,
                        top = pattern.bbox.top(),
                        name = registered.name,
                    );
                    if let Some(name) = pdf_writer.add_page_tiling_pattern(stream, pattern) {
                        paint_page_tiling_pattern(content, &name, tile_pattern.paint_box());
                        content.push_str("Q\n");
                        return;
                    }
                }
            } else if let Some(pattern) = tile_pattern.pdf_raster_pattern(source)
                && let stream = format!(
                    "q\n0 0 {width} {height} re W n\n{width} 0 0 {height} 0 0 cm\n/{name} Do\nQ\n",
                    width = source.width,
                    height = source.height,
                    name = registered.name,
                )
                && let Some(form) = pdf_writer.add_tiling_pattern(stream, pattern)
            {
                paint_tiling_pattern(content, &form, tile_pattern.paint_box());
                page_images.push(form);
                content.push_str("Q\n");
                return;
            }
        }
        let mut stream = format!("q\n1 0 0 -1 0 {scaled_h} cm\n");
        stream.push_str(&cell);
        stream.push_str("Q\n");
        let bbox = PdfRect::new(
            tile_clip_box.x,
            scaled_h - tile_clip_box.y - tile_clip_box.height,
            tile_clip_box.width,
            tile_clip_box.height,
        );
        if let Some(form) = tile_pattern
            .pdf_pattern(bbox)
            .and_then(|spec| pdf_writer.add_tiling_pattern(stream, spec))
        {
            paint_tiling_pattern(content, &form, tile_pattern.paint_box());
            page_images.push(form);
        }
    }
    content.push_str("Q\n");
}
