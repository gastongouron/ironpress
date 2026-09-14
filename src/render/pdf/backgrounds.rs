use super::geometry::{PdfRect, RoundedRect};
use super::transforms::{PageContentTransform, PdfPaintSpace};
use super::{ImageRef, PdfWriter};
use crate::render::background::BackgroundPaintContext;
use crate::render::shading::{PdfRgb, ShadingEntry};
use crate::style::computed::BackgroundClip;
use crate::types::CornerRadii;

#[derive(Debug, Clone, Copy)]
pub(super) struct PdfBackgroundPaintContext {
    pub(super) background: BackgroundPaintContext,
    pub(super) paint_space: Option<PdfPaintSpace>,
}

impl PdfBackgroundPaintContext {
    pub(super) const fn local(background: BackgroundPaintContext) -> Self {
        Self {
            background,
            paint_space: None,
        }
    }

    pub(super) const fn in_default_space(
        background: BackgroundPaintContext,
        paint_space: PdfPaintSpace,
    ) -> Self {
        Self {
            background,
            paint_space: Some(paint_space),
        }
    }
}

pub(super) struct PdfBackgroundResources<'a> {
    pub(super) writer: &'a mut PdfWriter,
    pub(super) images: &'a mut Vec<ImageRef>,
    pub(super) shadings: &'a mut Vec<ShadingEntry>,
    pub(super) shading_counter: &'a mut usize,
    pub(super) ext_gstates: Option<&'a mut Vec<(String, f32)>>,
    /// Loaded custom (bundled) fonts, so `<text>` inside a CSS
    /// background-image SVG can shape and render with a registered custom
    /// family instead of falling back to base-14 standard fonts.
    pub(super) custom_fonts:
        Option<&'a std::collections::HashMap<String, crate::parser::ttf::TtfFont>>,
    /// Subsetted/prepared custom fonts mirroring what body text embedded, so
    /// background SVG text references the same font resource and subset
    /// glyph-id remapping.
    pub(super) prepared_custom_fonts: Option<&'a crate::render::pdf_fonts::PreparedCustomFonts>,
}

impl<'a> PdfBackgroundResources<'a> {
    pub(super) fn new(
        writer: &'a mut PdfWriter,
        images: &'a mut Vec<ImageRef>,
        shadings: &'a mut Vec<ShadingEntry>,
        shading_counter: &'a mut usize,
        ext_gstates: Option<&'a mut Vec<(String, f32)>>,
    ) -> Self {
        Self {
            writer,
            images,
            shadings,
            shading_counter,
            ext_gstates,
            custom_fonts: None,
            prepared_custom_fonts: None,
        }
    }

    /// Wire the custom-font context through so background-image SVG `<text>`
    /// resolves registered families exactly like foreground SVG text.
    pub(super) fn with_custom_fonts(
        mut self,
        custom_fonts: &'a std::collections::HashMap<String, crate::parser::ttf::TtfFont>,
        prepared_custom_fonts: &'a crate::render::pdf_fonts::PreparedCustomFonts,
    ) -> Self {
        self.custom_fonts = Some(custom_fonts);
        self.prepared_custom_fonts = Some(prepared_custom_fonts);
        self
    }
}

/// Paint one resolved solid background through its final vector clip.
///
/// Callers resolve background-origin, background-clip, rounded corners, and
/// opaque-border coverage before reaching this homogeneous paint operation.
pub(super) fn paint_solid_background(
    content: &mut String,
    color: crate::types::Color,
    painting_box: RoundedRect,
    ext_gstates: &mut Vec<(String, f32)>,
    alpha_counter: &mut usize,
) {
    let alpha = color.alpha();
    if alpha <= 0.0 || painting_box.rect.is_empty() {
        return;
    }
    let needs_alpha = alpha < 1.0;
    if needs_alpha {
        let name = format!("GSbackground{alpha_counter}");
        *alpha_counter += 1;
        ext_gstates.push((name.clone(), alpha));
        content.push_str(&format!("/{name} gs\n"));
    }
    content.push_str(&PdfRgb::from(color).fill_operator());
    content.push_str(&painting_box.path_or_rect());
    content.push_str("f\n");
    if needs_alpha {
        content.push_str("/GSDefault gs\n");
    }
}

/// A square content/padding clip can use physical device coordinates followed
/// by CSS-pixel paint. Text clips need glyph outlines and rounded clips need
/// their own geometry.
pub(super) fn is_device_clippable_box_background(
    clip: BackgroundClip,
    radii: CornerRadii,
    page_transform: PageContentTransform,
    clip_rect: PdfRect,
) -> bool {
    if !radii.is_zero() {
        return false;
    }
    match clip {
        BackgroundClip::Padding | BackgroundClip::Content => true,
        BackgroundClip::Border => page_transform.page_edge_contact(clip_rect).any(),
        BackgroundClip::Text => false,
    }
}

/// Paint a rectangular CSS background through physical device clipping when
/// the paint genuinely extends beyond its clip.
///
/// A same-rectangle clip is semantically redundant, and Chrome emits the
/// fill directly in that case. Keeping the redundant PDF clip changes
/// Poppler's half-open edge coverage, so preserve the direct-fill form.
/// Returns `false` for non-print writers so their ordinary point-space
/// fallback remains intact.
pub(super) fn paint_device_clipped_css_solid(
    content: &mut String,
    page_transform: PageContentTransform,
    paint: PdfRect,
    clip: PdfRect,
    color: PdfRgb,
) -> bool {
    if paint.is_empty() || clip.is_empty() {
        return false;
    }
    if paint == clip {
        content.push_str(&color.fill_operator());
        content.push_str(&paint.rect_path());
        content.push_str("f\n");
        return true;
    }
    let Some(device) = page_transform.device_space() else {
        return false;
    };

    content.push_str("q\n");
    let page_edges = page_transform.page_edge_contact(clip);
    if page_edges.any() {
        content.push_str(&device.layout_edge_correction_operator(page_edges));
    }
    content.push_str(&device.enter_operator());
    content.push_str(&device.layout_rect(clip).rect_path());
    content.push_str("W* n\n");
    content.push_str(&device.css_page_operator());
    content.push_str(&color.fill_operator());
    content.push_str(&device.css_page_rect(paint).rect_path());
    content.push_str("f\nQ\n");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_paint_and_clip_emit_a_direct_fill() {
        let rect = PdfRect::new(0.0, 10.0, 20.0, 30.0);
        let mut content = String::new();

        assert!(paint_device_clipped_css_solid(
            &mut content,
            PageContentTransform::default(),
            rect,
            rect,
            PdfRgb::from((0.25, 0.5, 0.75)),
        ));

        assert_eq!(content, "0.25 0.5 0.75 rg\n0 10 20 30 re\nf\n");
    }
}
