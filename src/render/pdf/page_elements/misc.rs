use super::*;
use crate::layout::elements::{MathBlock, ProgressBar};
use crate::render::pdf::layout_elements::TextRenderContext;

/// Paint the fixed UA representation of an `<hr>` at its block-start edge.
pub(in crate::render::pdf) fn paint_horizontal_rule(
    content: &mut String,
    origin: PdfPoint,
    width: f32,
) {
    content.push_str(&format!(
        "0.5 w\n0 0 0 RG\n{x1} {y} m {x2} {y} l\nS\n",
        x1 = origin.x,
        y = origin.y,
        x2 = origin.x + width,
    ));
}

/// Paint a progress/meter box at an already-resolved border-box rectangle.
pub(in crate::render::pdf) fn paint_progress_bar(
    content: &mut String,
    element: &ProgressBar,
    rect: PdfRect,
) {
    let (track_r, track_g, track_b) = element.colors.track.to_f32_rgb();
    content.push_str(&format!(
        "{track_r} {track_g} {track_b} rg\n{}f\n",
        rect.rect_path(),
    ));
    if element.fraction > 0.0 {
        let (fill_r, fill_g, fill_b) = element.colors.fill.to_f32_rgb();
        let fill = PdfRect::new(
            rect.left,
            rect.bottom,
            rect.width * element.fraction,
            rect.height,
        );
        content.push_str(&format!(
            "{fill_r} {fill_g} {fill_b} rg\n{}f\n",
            fill.rect_path(),
        ));
    }
    content.push_str(&format!("0.5 w\n0.6 0.6 0.6 RG\n{}S\n", rect.rect_path(),));
}

/// Paint a math block from a block-start origin shared by page and child flow.
///
/// The formula is one marked-content sequence of its element. Accessible
/// output paints every glyph an embedded face covers as a text run (ISO
/// 14289-1 7.21.4.1); rules, radicals, scaled delimiters and uncovered
/// symbols keep their historical painters.
pub(in crate::render::pdf) fn paint_math_block(
    content: &mut String,
    element: &MathBlock,
    origin: PdfPoint,
    available_width: f32,
    text: &mut TextRenderContext<'_>,
) {
    let x = if element.display {
        origin.x + (available_width - element.layout.width) / 2.0
    } else {
        origin.x
    };
    let baseline = origin.y - element.layout.ascent;
    let mark = text.pdf_writer.structure.begin_content(
        content,
        element.group.owner,
        crate::render::pdf::structure::MarkKind::Text,
    );
    if text.pdf_writer.structure.font_policy().embeds_side_text() {
        for glyph in &element.layout.glyphs {
            match glyph.embedded_run(text.custom_fonts) {
                Some(embedded) => {
                    paint_embedded_math_run(content, embedded, PdfPoint::new(x, baseline), text)
                }
                None => render_math_glyphs(std::slice::from_ref(glyph), x, baseline, content),
            }
        }
    } else {
        render_math_glyphs(&element.layout.glyphs, x, baseline, content);
    }
    text.pdf_writer.structure.end(content, mark);
}

fn paint_embedded_math_run(
    content: &mut String,
    embedded: EmbeddedMathRun,
    origin: PdfPoint,
    text: &mut TextRenderContext<'_>,
) {
    let runs = [embedded.run];
    paint_horizontal_line_text(
        content,
        &runs,
        HorizontalLinePaint {
            origin: PdfPoint::new(origin.x + embedded.dx, origin.y + embedded.dy),
            line_ascender: 0.0,
            justification_word_spacing: 0.0,
            text_space: PdfContentSpace::Points,
        },
        text.custom_fonts,
        text.prepared_custom_fonts,
        text.pdf_writer,
        text.page_images,
    );
}
