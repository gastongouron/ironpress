use super::*;

/// Balanced PDF wrappers for one CSS paint group.
///
/// The scope preserves the CSS post-filter order shared by ordinary boxes,
/// cells, and rasterized filter replacements: transform, clip, mask, then
/// opacity/blending of the isolated result.
pub(super) struct PaintGroupScope {
    /// The owner's marked content around a group cut into a form XObject.
    mark: super::structure::OpenMark,
    group_start: Option<usize>,
    prior_paint_transform: Option<PdfMatrix>,
    clipped: bool,
    mask: Option<MaskedTransparencyGroup>,
    opacity: f32,
    blend_mode: crate::style::computed::BlendMode,
    paint_box: PdfRect,
}

impl PaintGroupScope {
    pub(super) fn begin(
        content: &mut String,
        owner: &dyn crate::layout::elements::PaintGroupOwner,
        geometry: FragmentPaintGeometry,
        ctx: &mut PageRenderContext<'_>,
    ) -> Self {
        let group = owner.paint_group();
        let effects = &group.effects;
        let grouped = effects.needs_source_isolation();
        // Content cut into a form XObject inherits the marked content that
        // invokes the form, so the owner's mark opens before the cut point
        // and nothing inside the form allocates an MCID of its own.
        let mark = if grouped || effects.masking.image.is_some() {
            ctx.text
                .pdf_writer
                .structure
                .begin_group(content, group.owner)
        } else {
            super::structure::OpenMark::Skipped
        };
        let group_start = grouped.then_some(content.len());

        let resolved_transform = group.transform.value.as_ref().map(|transform| {
            let reference = geometry.painting().transform_reference(&group.transform);
            resolve_css_transform(transform, reference.pivot(), reference.size())
        });
        let prior_paint_transform = resolved_transform.map(|transform| {
            let prior = ctx
                .text
                .pdf_writer
                .enter_paint_transform(transform.matrix());
            content.push_str("q\n");
            push_resolved_transform_cm(content, transform);
            prior
        });

        let clipped = effects.masking.clip_path.is_some();
        if let Some(clip_path) = &effects.masking.clip_path {
            content.push_str("q\n");
            push_clip_path(
                content,
                clip_path,
                Some(&ctx.text.pdf_writer.svg_defs),
                geometry,
            );
        }

        let mask = effects.masking.image.as_ref().and_then(|source| {
            MaskedTransparencyGroup::begin(
                content,
                ctx.text.pdf_writer,
                source,
                effects.masking.mode,
                geometry,
                ctx.paint_box,
            )
        });

        Self {
            mark,
            group_start,
            prior_paint_transform,
            clipped,
            mask,
            opacity: effects.opacity,
            blend_mode: effects.mix_blend_mode,
            // A PDF form's BBox clips its stream. A CSS paint group includes
            // transformed descendants, shadows, outlines, and filter visual
            // overflow, so the principal border box is not a sound bound.
            // The page paint box is the one generic, already-clipped extent
            // containing every component's complete on-page result.
            paint_box: ctx.paint_box,
        }
    }

    pub(super) fn finish(self, content: &mut String, ctx: &mut PageRenderContext<'_>) {
        if let Some(mask) = self.mask {
            mask.finish(content, ctx.text.pdf_writer, ctx.text.page_images);
        }
        if self.clipped {
            content.push_str("Q\n");
        }
        if let Some(prior) = self.prior_paint_transform {
            content.push_str("Q\n");
            ctx.text.pdf_writer.restore_paint_transform(prior);
        }
        if let Some(group_start) = self.group_start {
            finish_transparency_group(
                content,
                group_start,
                ctx.text.pdf_writer,
                ctx.text.page_images,
                ctx.page_ext_gstates,
                self.opacity,
                self.blend_mode,
                self.paint_box,
            );
        }
        ctx.text.pdf_writer.structure.end(content, self.mark);
    }
}

pub(super) struct MaskedTransparencyGroup {
    start: usize,
    state: String,
    paint_box: PdfRect,
}

impl MaskedTransparencyGroup {
    pub(super) fn begin(
        content: &str,
        pdf_writer: &mut PdfWriter,
        source: &MaskSource,
        mode: MaskMode,
        geometry: FragmentPaintGeometry,
        paint_box: PdfRect,
    ) -> Option<Self> {
        Some(Self {
            start: content.len(),
            state: pdf_writer.add_mask_soft_mask(source, mode, geometry.positioning())?,
            paint_box,
        })
    }

    pub(super) fn finish(
        self,
        content: &mut String,
        pdf_writer: &mut PdfWriter,
        page_images: &mut Vec<ImageRef>,
    ) {
        if self.start >= content.len() || self.paint_box.is_empty() {
            return;
        }
        let stream = content[self.start..].to_string();
        if stream.trim().is_empty() {
            return;
        }
        content.truncate(self.start);
        let form = pdf_writer.add_transparency_group_form(stream, self.paint_box);
        content.push_str(&format!("q\n/{} gs\n/{} Do\nQ\n", self.state, form.name));
        page_images.push(form);
    }
}

/// Register a blend-mode ExtGState and emit its `gs` operator, returning `true`
/// when a non-`Normal` blend was applied. The gstate name encodes the PDF blend
/// mode (`GSbm<Mode>`); the writer turns that into a `<< /BM /<Mode> >>` dict.
/// Callers wrap the affected paint in `q`..`Q` so the blend (and its restore via
/// `Q`) is scoped to that paint only. For `Normal` nothing is emitted, so output
/// for non-blended elements stays byte-identical.
pub(super) fn begin_blend_mode(
    content: &mut String,
    page_ext_gstates: &mut Vec<(String, f32)>,
    mode: crate::style::computed::BlendMode,
) -> bool {
    if let Some(pdf_mode) = mode.pdf_name() {
        let gs_name = format!("GSbm{pdf_mode}");
        // Deduplicated by name in the writer, so registering the same blend mode
        // from multiple elements is harmless.
        page_ext_gstates.push((gs_name.clone(), 1.0));
        content.push_str(&format!("/{gs_name} gs\n"));
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_transparency_group(
    content: &mut String,
    group_start: usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    page_ext_gstates: &mut Vec<(String, f32)>,
    opacity: f32,
    blend_mode: crate::style::computed::BlendMode,
    paint_box: PdfRect,
) {
    if group_start >= content.len() || paint_box.is_empty() {
        return;
    }
    let group_stream = content[group_start..].to_string();
    if group_stream.trim().is_empty() {
        return;
    }
    content.truncate(group_start);

    let form_ref = pdf_writer.add_transparency_group_form(group_stream, paint_box);
    page_images.push(form_ref.clone());

    content.push_str("q\n");
    begin_blend_mode(content, page_ext_gstates, blend_mode);
    if opacity < 1.0 {
        let gs_name = format!("GSgrp{}", form_ref.obj_id);
        page_ext_gstates.push((gs_name.clone(), opacity));
        content.push_str(&format!("/{gs_name} gs\n"));
    }
    content.push_str(&format!("/{} Do\nQ\n", form_ref.name));
}

pub(super) fn fill_rgba_rect(
    img: &mut image::RgbaImage,
    px_per_pt: f32,
    x_pt: f32,
    y_pt: f32,
    w_pt: f32,
    h_pt: f32,
    color: (f32, f32, f32, f32),
) {
    if w_pt <= 0.0 || h_pt <= 0.0 || color.3 <= 0.0 {
        return;
    }
    let x0 = (x_pt * px_per_pt).round().max(0.0) as u32;
    let y0 = (y_pt * px_per_pt).round().max(0.0) as u32;
    let x1 = ((x_pt + w_pt) * px_per_pt)
        .round()
        .clamp(0.0, img.width() as f32) as u32;
    let y1 = ((y_pt + h_pt) * px_per_pt)
        .round()
        .clamp(0.0, img.height() as f32) as u32;
    let src = image::Rgba([
        (color.0 * 255.0).round().clamp(0.0, 255.0) as u8,
        (color.1 * 255.0).round().clamp(0.0, 255.0) as u8,
        (color.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        (color.3 * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
    for y in y0..y1 {
        for x in x0..x1 {
            let dst = *img.get_pixel(x, y);
            img.put_pixel(x, y, over_rgba(src, dst));
        }
    }
}

pub(super) fn over_rgba(src: image::Rgba<u8>, dst: image::Rgba<u8>) -> image::Rgba<u8> {
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return image::Rgba([0, 0, 0, 0]);
    }
    let blend = |s: u8, d: u8| {
        ((s as f32 * sa + d as f32 * da * (1.0 - sa)) / oa)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    image::Rgba([
        blend(src[0], dst[0]),
        blend(src[1], dst[1]),
        blend(src[2], dst[2]),
        (oa * 255.0).round() as u8,
    ])
}

pub(super) fn composite_text_mask(
    img: &mut image::RgbaImage,
    mask: &image::GrayImage,
    dst_x: i32,
    dst_y: i32,
    color: crate::types::Color,
) {
    let [r, g, b, color_alpha] = color.to_rgba8();
    for y in 0..mask.height() {
        for x in 0..mask.width() {
            let a =
                ((u16::from(mask.get_pixel(x, y)[0]) * u16::from(color_alpha) + 127) / 255) as u8;
            if a == 0 {
                continue;
            }
            let tx = dst_x + x as i32;
            let ty = dst_y + y as i32;
            if tx < 0 || ty < 0 || tx >= img.width() as i32 || ty >= img.height() as i32 {
                continue;
            }
            let dst = *img.get_pixel(tx as u32, ty as u32);
            img.put_pixel(
                tx as u32,
                ty as u32,
                over_rgba(image::Rgba([r, g, b, a]), dst),
            );
        }
    }
}
