use super::*;
use crate::layout::elements::{Container, LayoutNode, LayoutVisitor, TextBlock};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_simple_text_block(
    img: &mut image::RgbaImage,
    px_per_pt: f32,
    x_pt: f32,
    y_pt: f32,
    width_pt: f32,
    height_pt: f32,
    background: Option<crate::types::Color>,
    lines: &[TextLine],
    padding: EdgeSizes,
    border: &crate::layout::engine::LayoutBorder,
    text_align: TextAlign,
    text_indent: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) -> Option<()> {
    if width_pt <= 0.0
        || height_pt <= 0.0
        || border.has_visible()
        || lines.iter().flat_map(|line| &line.runs).any(|run| {
            run.metadata.spacing != Default::default() || run.metadata.boundary.total() != 0.0
        })
    {
        return None;
    }
    if let Some(bg) = background {
        fill_rgba_rect(
            img,
            px_per_pt,
            x_pt,
            y_pt,
            width_pt,
            height_pt,
            bg.to_f32_rgba(),
        );
    }
    let content_w = (width_pt - border.horizontal_width() - padding.horizontal()).max(0.0);
    let mut baseline_cursor =
        crate::render::blur::RasterBaselineCursor::new(y_pt + border.top.width + padding.top, 0.0);
    for (line_idx, line) in lines.iter().enumerate() {
        let metrics = line_box_metrics(line, custom_fonts);
        let baseline_y = baseline_cursor.next(crate::render::blur::RasterBaselineAdvance::new(
            metrics.half_leading + metrics.ascender,
            metrics.descender + metrics.half_leading,
        ));
        let merged = crate::text::coalesce_text_runs(&line.runs);
        let line_width: f32 = merged
            .iter()
            .map(|run| {
                if run.inline_box.is_some()
                    || !run.decorations.is_empty()
                    || run.background_color.is_some()
                    || !run.text_shadow.is_empty()
                    || run.vertical_align != VerticalAlign::Baseline
                {
                    return f32::NAN;
                }
                estimate_run_width_with_fonts(run, custom_fonts)
            })
            .sum();
        if !line_width.is_finite() {
            return None;
        }
        let first_line_indent = if line_idx == 0 { text_indent } else { 0.0 };
        let text_x = match text_align {
            TextAlign::Right => {
                x_pt + border.left.width
                    + padding.left
                    + first_line_indent
                    + (content_w - first_line_indent - line_width).max(0.0)
            }
            TextAlign::Center => {
                x_pt + border.left.width
                    + padding.left
                    + first_line_indent
                    + (content_w - first_line_indent - line_width).max(0.0) / 2.0
            }
            _ => x_pt + border.left.width + padding.left + first_line_indent,
        } + line.x_offset;
        let mut run_x = text_x;
        for run in &merged {
            if run.text.is_empty() {
                continue;
            }
            let (_, font) = crate::text::resolve_custom_font(
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            )?;
            let shaped = crate::text::shape_text_run(run, custom_fonts)?;
            let synthetic_bold_width = run.synthetic_bold_stroke_width(custom_fonts);
            let raster = crate::render::blur::rasterize_run_alpha(
                crate::render::blur::GlyphRasterRequest {
                    font,
                    font_size: font.adjusted_font_size(run.font_size),
                    glyphs: &shaped.glyphs,
                    style: crate::render::blur::GlyphRasterStyle {
                        embolden: synthetic_bold_width.unwrap_or_default(),
                        shear: run.synthetic_italic_shear(custom_fonts).unwrap_or_default(),
                    },
                    origin: crate::render::blur::GlyphBaselineOrigin::top_down(run_x, baseline_y),
                    dpi: filter_dpi,
                },
            )?;
            let placement = raster.placement;
            composite_text_mask(
                img,
                &raster.mask,
                placement.mask_origin.x,
                placement.mask_origin.y,
                run.color,
            );
            run_x += estimate_run_width_with_fonts(run, custom_fonts);
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blurred_simple_text_block(
    width_pt: f32,
    height_pt: f32,
    background: Option<crate::types::Color>,
    lines: &[TextLine],
    padding: EdgeSizes,
    border: &crate::layout::engine::LayoutBorder,
    text_align: TextAlign,
    text_indent: f32,
    blur_radius_pt: f32,
    filter_dpi: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<crate::render::blur::BlurredRaster> {
    if blur_radius_pt <= 0.0 {
        return None;
    }
    let px_per_pt = crate::render::raster_scale::RasterScale::at_dpi(filter_dpi).pixels_per_point();
    let px_w = (width_pt * px_per_pt).round().max(1.0) as u32;
    let px_h = (height_pt * px_per_pt).round().max(1.0) as u32;
    let mut img = image::RgbaImage::new(px_w, px_h);
    paint_simple_text_block(
        &mut img,
        px_per_pt,
        0.0,
        0.0,
        width_pt,
        height_pt,
        background,
        lines,
        padding,
        border,
        text_align,
        text_indent,
        custom_fonts,
        filter_dpi,
    )?;
    crate::render::blur::blur_painted_buffer(&img, blur_radius_pt, filter_dpi)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blurred_simple_container_group(
    children: &[LayoutNode],
    width_pt: f32,
    height_pt: f32,
    background: Option<crate::types::Color>,
    border: &crate::layout::engine::LayoutBorder,
    padding: EdgeSizes,
    blur_radius_pt: f32,
    filter_dpi: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<crate::render::blur::BlurredRaster> {
    if width_pt <= 0.0 || height_pt <= 0.0 || blur_radius_pt <= 0.0 || border.has_visible() {
        return None;
    }
    let px_per_pt = crate::render::raster_scale::RasterScale::at_dpi(filter_dpi).pixels_per_point();
    let px_w = (width_pt * px_per_pt).round().max(1.0) as u32;
    let px_h = (height_pt * px_per_pt).round().max(1.0) as u32;
    let mut img = image::RgbaImage::new(px_w, px_h);
    if let Some(bg) = background {
        fill_rgba_rect(
            &mut img,
            px_per_pt,
            0.0,
            0.0,
            width_pt,
            height_pt,
            bg.to_f32_rgba(),
        );
    }

    let content_w = (width_pt - padding.horizontal()).max(0.0);
    let mut cursor_y = padding.top;
    let mut prev_margin_bottom = 0.0;

    struct ChildPainter<'a> {
        image: &'a mut image::RgbaImage,
        pixels_per_point: f32,
        parent_padding: EdgeSizes,
        content_width: f32,
        cursor_y: &'a mut f32,
        previous_margin_end: &'a mut f32,
        fonts: &'a dyn crate::font_registry::FontRegistry,
        filter_dpi: f32,
        recognized: bool,
        valid: bool,
    }

    impl LayoutVisitor for ChildPainter<'_> {
        fn visit_container(&mut self, element: &Container) {
            self.recognized = true;
            if !element.paint.visible
                || !element.children.is_empty()
                || element.positioning.scheme != Position::Static
                || element.flow.float != Float::None
                || element.paint.group.effects.opacity < 1.0
                || element.paint.group.effects.mix_blend_mode
                    != crate::style::computed::BlendMode::Normal
                || element.paint.group.transform.value.is_some()
                || element.paint.group.effects.masking.clip_path.is_some()
                || element.paint.group.effects.masking.image.is_some()
                || !element.paint.shadows.is_empty()
                || element.paint.background.layers.has_image()
                || element.paint.background.layers.blur_radius > 0.0
                || element.box_model.border.has_visible()
                || !element.paint.border_radii.is_zero()
                || element.paint.outline.width > 0.0
            {
                self.valid = false;
                return;
            }
            let margins = element.box_model.margins;
            *self.cursor_y += collapsed_margin_top_extra(margins.start, *self.previous_margin_end);
            let child_width = element.box_model.size.resolve_width(self.content_width);
            let child_height = element
                .box_model
                .size
                .height
                .resolve(element.box_model.padding.vertical());
            let child_x = self.parent_padding.left + element.positioning.insets.left;
            let child_y = *self.cursor_y + element.positioning.insets.top;
            if let Some(background) = element.paint.background.color {
                fill_rgba_rect(
                    self.image,
                    self.pixels_per_point,
                    child_x,
                    child_y,
                    child_width,
                    child_height,
                    background.to_f32_rgba(),
                );
            }
            *self.cursor_y += child_height + margins.end;
            *self.previous_margin_end = margins.end;
        }

        fn visit_text_block(&mut self, element: &TextBlock) {
            self.recognized = true;
            if element.positioning.scheme != Position::Static
                || element.flow.float != Float::None
                || element.paint.group.effects.opacity < 1.0
                || element.paint.group.effects.mix_blend_mode
                    != crate::style::computed::BlendMode::Normal
                || element.paint.group.transform.value.is_some()
                || element.clipping.rect.is_some()
                || element.paint.background.layers.has_image()
                || element.paint.background.layers.blur_radius > 0.0
                || element.box_model.border.has_visible()
                || !element.paint.border_radii.is_zero()
                || element.paint.outline.width > 0.0
                || element.lines.iter().flat_map(|line| &line.runs).any(|run| {
                    run.metadata.spacing != Default::default()
                        || run.metadata.boundary.total() != 0.0
                })
            {
                self.valid = false;
                return;
            }
            let margins = element.box_model.margins;
            *self.cursor_y += collapsed_margin_top_extra(margins.start, *self.previous_margin_end);
            let text_height = element.lines.iter().map(|line| line.height).sum::<f32>();
            let natural_height = element.box_model.padding.vertical() + text_height;
            let child_height = element
                .box_model
                .size
                .height
                .used()
                .map_or(natural_height, |height| natural_height.max(height));
            let child_width = element.box_model.size.resolve_width(self.content_width);
            let child_x = self.parent_padding.left + element.positioning.insets.left;
            let child_y = *self.cursor_y + element.positioning.insets.top;
            if let Some(background) = element.paint.background.color {
                fill_rgba_rect(
                    self.image,
                    self.pixels_per_point,
                    child_x,
                    child_y,
                    child_width,
                    child_height,
                    background.to_f32_rgba(),
                );
            }
            let mut baseline_cursor = crate::render::blur::RasterBaselineCursor::new(
                child_y + element.box_model.padding.top,
                0.0,
            );
            for line in &element.lines {
                let metrics = line_box_metrics(line, self.fonts);
                let baseline_y =
                    baseline_cursor.next(crate::render::blur::RasterBaselineAdvance::new(
                        metrics.half_leading + metrics.ascender,
                        metrics.descender + metrics.half_leading,
                    ));
                let merged = crate::text::coalesce_text_runs(&line.runs);
                let line_width = merged
                    .iter()
                    .map(|run| estimate_run_width_with_fonts(run, self.fonts))
                    .sum::<f32>();
                let text_x = match element.text.alignment {
                    TextAlign::Right => child_x + (child_width - line_width).max(0.0),
                    TextAlign::Center => child_x + (child_width - line_width).max(0.0) / 2.0,
                    _ => child_x,
                } + element.box_model.padding.left
                    + line.x_offset;
                let mut run_x = text_x;
                for run in &merged {
                    let Some((_, font)) = crate::text::resolve_custom_font(
                        &run.font_family,
                        run.bold,
                        run.font_style.is_slanted(),
                        self.fonts,
                    ) else {
                        self.valid = false;
                        return;
                    };
                    let Some(shaped) = crate::text::shape_text_run(run, self.fonts) else {
                        self.valid = false;
                        return;
                    };
                    let Some(raster) = crate::render::blur::rasterize_run_alpha(
                        crate::render::blur::GlyphRasterRequest {
                            font,
                            font_size: font.adjusted_font_size(run.font_size),
                            glyphs: &shaped.glyphs,
                            style: crate::render::blur::GlyphRasterStyle {
                                embolden: run
                                    .synthetic_bold_stroke_width(self.fonts)
                                    .unwrap_or_default(),
                                shear: run.synthetic_italic_shear(self.fonts).unwrap_or_default(),
                            },
                            origin: crate::render::blur::GlyphBaselineOrigin::top_down(
                                run_x, baseline_y,
                            ),
                            dpi: self.filter_dpi,
                        },
                    ) else {
                        self.valid = false;
                        return;
                    };
                    composite_text_mask(
                        self.image,
                        &raster.mask,
                        raster.placement.mask_origin.x,
                        raster.placement.mask_origin.y,
                        run.color,
                    );
                    run_x += estimate_run_width_with_fonts(run, self.fonts);
                }
            }
            *self.cursor_y += child_height + margins.end;
            *self.previous_margin_end = margins.end;
        }
    }

    for child in children {
        let mut painter = ChildPainter {
            image: &mut img,
            pixels_per_point: px_per_pt,
            parent_padding: padding,
            content_width: content_w,
            cursor_y: &mut cursor_y,
            previous_margin_end: &mut prev_margin_bottom,
            fonts: custom_fonts,
            filter_dpi,
            recognized: false,
            valid: true,
        };
        child.accept(&mut painter);
        if !painter.recognized || !painter.valid {
            return None;
        }
    }
    crate::render::blur::blur_painted_buffer(&img, blur_radius_pt, filter_dpi)
}
