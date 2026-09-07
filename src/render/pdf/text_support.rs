use super::*;
use crate::layout::text_emphasis::TextEmphasisMetrics;

mod decoration;

pub(super) use decoration::*;

pub(super) fn register_used_custom_fonts(
    pdf_writer: &mut PdfWriter,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
) {
    for (font_name, prepared_font) in prepared_custom_fonts {
        if let Some(ttf) = custom_fonts.get(prepared_font.source_font_name(font_name)) {
            pdf_writer.add_ttf_font(font_name, ttf, prepared_font);
        }
    }
}

pub(super) fn font_name_for_run(run: &TextRun) -> &str {
    match (&run.font_family, run.bold, run.font_style.is_slanted()) {
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

pub(super) fn estimate_run_width(run: &TextRun) -> f32 {
    crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
}

pub(super) fn text_run_letter_spacing(run: &TextRun) -> f32 {
    run.metadata.spacing.letter
}

/// Resolve the PDF font resource name for a text run.
///
/// Custom Type0 fonts are only safe when we also have shaped glyph output.
pub(super) fn resolve_font_name(
    run: &TextRun,
    custom_font: Option<(&str, &TtfFont)>,
    shaped: Option<&crate::text::ShapedRun>,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> String {
    if let (Some((resolved_name, _)), Some(_)) = (custom_font, shaped) {
        sanitize_pdf_name(&prepared_font_name_for_run(
            resolved_name,
            run,
            custom_fonts,
        ))
    } else {
        font_name_for_run(run).to_string()
    }
}

pub(super) fn inline_background_y_and_height(
    run: &TextRun,
    text_y: f32,
    padding: EdgeSizes,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> (f32, f32) {
    let line_height = crate::fonts::font_line_metrics(
        run.css_font_family(),
        run.font_size,
        run.bold,
        run.font_style.is_slanted(),
        custom_fonts,
    )
    .normal_line_height();
    let strut = crate::layout::text::LineStrut::from_font(
        run.css_font_family(),
        run.font_size,
        run.bold,
        run.font_style.is_slanted(),
        line_height,
        custom_fonts,
    );
    (
        text_y - strut.below - padding.bottom,
        strut.above + strut.below + padding.vertical(),
    )
}

pub(super) fn decoration_is_emphasis(run: &TextRun) -> bool {
    run.metadata.emphasis.mark
}

pub(super) fn text_emphasis_baseline_shift(run: &TextRun) -> f32 {
    TextEmphasisMetrics::from_run(run).baseline_shift
}

pub(super) fn is_cjk_codepoint(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0xac00..=0xd7af
    )
}

/// Construct Chrome's filled-dot emphasis mark as a normal shaped glyph.
///
/// Keeping the mark as text, rather than approximating it with a circle,
/// preserves the selected font's outline and side bearings.
pub(crate) fn emphasis_mark_run(run: &TextRun) -> TextRun {
    TextRun {
        text: "•".to_owned(),
        font_size: run.font_size * TextEmphasisMetrics::MARK_FONT_SCALE,
        bold: run.bold,
        font_style: run.font_style,
        color: run.color,
        font_family: run.css_font_family().clone(),
        font_synthesis: run.font_synthesis,
        shaping: run.shaping,
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextEmphasisPlacement {
    pub(super) origin: PdfPoint,
    pub(super) color: crate::types::Color,
}

pub(super) fn render_text_emphasis_marks(
    content: &mut String,
    run: &TextRun,
    placement: TextEmphasisPlacement,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let metrics = TextEmphasisMetrics::from_run(run);
    let mut mark = emphasis_mark_run(run);
    mark.color = placement.color;
    // Emphasis marks are ruby annotations (css-text-decor-4 §3.4). Chrome
    // resolves the annotation's inline extent upward to its CSS-pixel grid
    // before centering it over the base character.
    let mark_width =
        crate::fonts::ceil_to_css_pixel(estimate_run_width_with_fonts(&mark, custom_fonts));
    let mark_baseline =
        crate::fonts::round_to_css_pixel(placement.origin.y + metrics.mark_baseline_offset);
    let mut cx = placement.origin.x;
    for ch in run.text.chars() {
        let chs = ch.to_string();
        let adv = crate::layout::text::estimate_word_width(
            &chs,
            run.font_size,
            &run.font_family,
            run.bold,
            run.font_style.is_slanted(),
            custom_fonts,
        );
        if !ch.is_whitespace() {
            render_run_glyphs(
                content,
                &mark,
                cx + (adv - mark_width) / 2.0,
                mark_baseline,
                mark.font_size,
                custom_fonts,
                prepared_custom_fonts,
                0.0,
                pdf_writer,
                page_images,
            );
        }
        cx += adv;
    }
}

pub(super) fn estimate_run_width_with_fonts(
    run: &TextRun,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    if let Some(advance) = run.atomic_inline_advance() {
        return advance;
    }
    if let Some(width) = crate::text::measure_text_width_with_shaping(
        &run.text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        run.shaping,
        custom_fonts,
    ) {
        return run.text_advance(width, &run.text);
    }

    run.text_advance(estimate_run_width(run), &run.text)
}

pub(crate) fn encode_pdf_hex_glyph(glyph_id: u16) -> String {
    format!("{glyph_id:04X}")
}
