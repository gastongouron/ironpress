use super::*;
use crate::layout::elements::{LayoutNode, LayoutVisitor, TextBlock};

/// Glyph baseline shift (PDF points, up positive) for a text run.
///
/// css2 §10.8.1: `super`/`sub` move a text run's baseline up/down by a fraction
/// of the PARENT (line) font size — not the shrunk superscript's own size — so a
/// 40%- and a 100%-size superscript on one line are raised by the same amount
/// (matching Chrome). All other values leave the run on the line baseline. Atomic
/// inline boxes are aligned elsewhere (they carry their own geometry), so this
/// only affects pure-text runs.
pub(super) fn run_vertical_align_shift(run: &TextRun, parent_font_size: f32) -> f32 {
    if run.inline_box.is_some() {
        return 0.0;
    }
    run.glyph_baseline_shift(parent_font_size)
}

/// Line-box baseline shift caused by CSS `vertical-align` alone.
///
/// A `font-variant-position` fallback shifts only the painted glyph; it keeps
/// its inline box baseline-aligned and therefore does not participate here.
pub(super) fn run_line_box_baseline_shift(run: &TextRun, parent_font_size: f32) -> f32 {
    if run.inline_box.is_some() {
        return 0.0;
    }
    run.vertical_align_shift(parent_font_size)
}

pub(super) fn run_line_height_for_vertical_align(run: &TextRun) -> f32 {
    let factor = if run.line_height_factor.is_finite() {
        run.line_height_factor.max(0.0)
    } else {
        1.2
    };
    run.line_height_font_size() * factor
}

/// True when a run is a floated `::first-letter` drop cap (css-pseudo-4 §2.2 +
/// css2 §9.5). The drop cap is the only text run whose line-height factor was
/// deliberately capped below the surrounding line (`apply_first_letter_style`
/// sets it to `block_line_height / cap_font_size`, well under 1) so the enlarged
/// glyph overflows its line box instead of inflating it.
pub(super) fn is_drop_cap_run(run: &TextRun) -> bool {
    run.inline_box.is_none() && run.metadata.is_drop_cap
}

/// The visual top of a run's glyphs above the baseline, in points. Prefers the
/// actual glyph bounding-box top (`yMax`) of the run's first letter so accent
/// space reserved by the font ascender is excluded; falls back to the ascender
/// metric when the glyph has no measurable outline.
pub(super) fn run_glyph_top(
    run: &TextRun,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    let ch = run.text.chars().find(|c| !c.is_whitespace());
    if let (Some(ch), FontFamily::Custom(name)) = (ch, &run.font_family)
        && let Some((_, ttf)) = crate::system_fonts::find_font(
            custom_fonts,
            name,
            run.bold,
            run.font_style.is_slanted(),
        )
        && let Some(ratio) = ttf.glyph_top_ratio(ch)
    {
        return ratio * run.font_size;
    }
    let (ascender_ratio, _) = crate::fonts::font_metrics_ratios(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        custom_fonts,
    );
    ascender_ratio * run.font_size
}

/// Extra baseline offset (PDF up-positive) for a drop-cap run so its glyph TOP
/// aligns with the TOP of the surrounding first line's text, then drops downward
/// across the spanned lines — matching how browsers position a floated
/// `::first-letter` (css-pseudo-4 §2.2). Painted at the line baseline a cap-sized
/// glyph would overflow far ABOVE the box; lowering it so its glyph top meets the
/// line's text top seats it correctly. `line_text_top` is the surrounding line's
/// glyph top above the baseline (the drop cap is excluded from it).
pub(super) fn drop_cap_baseline_shift(
    run: &TextRun,
    line_text_top: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    if !is_drop_cap_run(run) {
        return 0.0;
    }
    let cap_top = run_glyph_top(run, custom_fonts);
    // Negative => move the glyph DOWN (PDF y grows up). Never raise it above the
    // line top (clamp at 0) so a small/normal-sized first-letter is unaffected.
    (line_text_top - cap_top).min(0.0)
}

/// The surrounding (non-drop-cap) text's glyph top above the baseline for a line,
/// in points — the reference the drop-cap glyph top is seated against. Zero when
/// the line carries no ordinary text runs.
pub(super) fn line_text_top(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    line.runs
        .iter()
        .filter(|r| r.inline_box.is_none() && !is_drop_cap_run(r) && !r.text.trim().is_empty())
        .map(|r| run_glyph_top(r, custom_fonts))
        .fold(0.0f32, f32::max)
}

#[derive(Clone, Copy)]
pub(super) struct LineBoxMetrics {
    pub(super) ascender: f32,
    pub(super) descender: f32,
    pub(super) half_leading: f32,
}

/// Per-run asymmetric line-box extents (above/below the baseline, in points) for
/// a line that contains a `vertical-align: super`/`sub` text run.
///
/// css2 §10.8.1: each inline text box contributes its half-leading-padded glyph
/// box (ascent+half / descent+half about its own baseline); a super/sub run has
/// that box shifted up/down by `parent_font_size * RATIO`. The line box is the
/// union, so it grows only on the shifted side. `wrap_text_runs` sizes the line
/// with the identical formula, so the painted baseline and the laid-out line
/// height stay consistent.
pub(super) fn line_shifted_text_extents(
    line: &TextLine,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> (f32, f32) {
    // Runs that left line-height unspecified fall back to the largest resolved
    // factor on the line (the parent text's), excluding drop caps (< 0.9).
    let rep_factor = line
        .runs
        .iter()
        .filter(|r| r.inline_box.is_none() && !is_drop_cap_run(r))
        .map(|r| r.line_height_factor)
        .fold(0.0f32, f32::max);
    let rep_factor = if rep_factor > 0.0 { rep_factor } else { 1.2 };
    let mut above = 0.0f32;
    let mut below = 0.0f32;
    for run in line.runs.iter().filter(|r| r.inline_box.is_none()) {
        // Drop caps overflow the line box and must not raise it (see above).
        if is_drop_cap_run(run) {
            continue;
        }
        let (asc_r, desc_r) = crate::fonts::font_metrics_ratios(
            run.css_font_family(),
            run.bold,
            run.font_style.is_slanted(),
            custom_fonts,
        );
        let logical_font_size = run.line_height_font_size();
        let asc = asc_r * logical_font_size;
        let desc = desc_r * logical_font_size;
        let factor = if run.line_height_factor.is_finite() {
            run.line_height_factor
        } else {
            rep_factor
        };
        let half = ((run.line_height_font_size() * factor - (asc + desc)) / 2.0).max(0.0);
        let shift = run_line_box_baseline_shift(run, parent_font_size);
        above = above.max(asc + half + shift);
        below = below.max(desc + half - shift);
    }
    (above, below)
}

pub(super) fn line_authored_text_extents(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> (f32, f32) {
    line.runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        .filter(|r| !is_drop_cap_run(r))
        .fold((0.0f32, 0.0f32), |(above, below), run| {
            let (asc_r, desc_r) = crate::fonts::font_metrics_ratios(
                run.css_font_family(),
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            );
            let logical_font_size = run.line_height_font_size();
            let asc = asc_r * logical_font_size;
            let desc = desc_r * logical_font_size;
            let half = (run_line_height_for_vertical_align(run) - (asc + desc)) / 2.0;
            (above.max(asc + half), below.max(desc + half))
        })
}

pub(super) fn line_box_metrics(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> LineBoxMetrics {
    line_box_metrics_with_resolved_baseline(line, custom_fonts, true)
}

fn line_box_metrics_with_resolved_baseline(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    use_resolved_baseline: bool,
) -> LineBoxMetrics {
    if use_resolved_baseline && let Some(baseline_ascent) = line.baseline_ascent {
        return LineBoxMetrics {
            ascender: baseline_ascent,
            descender: (line.height - baseline_ascent).max(0.0),
            half_leading: 0.0,
        };
    }
    // `super`/`sub` shifts are a fraction of the parent (surrounding) font size.
    let parent_font_size = crate::layout::text::line_primary_font_size(&line.runs);
    let (ascender, descender) = line
        .runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        // A floated `::first-letter` drop cap stays inline on the first line but
        // is out of flow (css2 §9.5): its enlarged glyph overflows the line box
        // downward and must NOT raise the line's ascent/descent. It is marked by
        // an explicit line-height factor capped well below 1 (its line box was
        // reduced to the surrounding line height in `apply_first_letter_style`).
        .filter(|r| !is_drop_cap_run(r))
        .fold((0.0f32, 0.0f32), |(max_ascender, max_descender), run| {
            let (ascender_ratio, descender_ratio) = crate::fonts::font_metrics_ratios(
                run.css_font_family(),
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            );
            (
                max_ascender.max(ascender_ratio * run.line_height_font_size()),
                max_descender.max(descender_ratio * run.line_height_font_size()),
            )
        });
    // The block's strut establishes the line box BEFORE inline-level boxes are
    // aligned (CSS2 §10.8): the requested `line.height` is split into the text's
    // ascent/descent plus symmetric half-leading. The baseline therefore sits at
    // `strut_above = text_ascent + half_leading` below the line-box top. When the
    // line-height already exceeds the text's content extent, that leading is real
    // space ABOVE/BELOW the baseline that an inline box may occupy WITHOUT growing
    // the line box or moving the baseline. We fold the half-leading into the
    // returned ascent/descent (and report `half_leading = 0`) so downstream
    // `ascender + half_leading` / `descender + half_leading` sums are unchanged
    // for pure text, while a baseline box only pushes the baseline when it pokes
    // past the strut's leading-padded edges (matching Chrome).
    let strut_half_leading = (line.height - (ascender + descender)) / 2.0;
    // A `vertical-align: super`/`sub` text run shifts its half-leading-padded
    // glyph box off the baseline (css2 §10.8.1); the line then grows ONLY on the
    // shifted side. The symmetric strut split above cannot express that, so for
    // such lines compute the per-run asymmetric extents instead — the same model
    // `wrap_text_runs` used to size the line, keeping layout and paint consistent.
    let has_text_shift = line.runs.iter().any(|run| {
        run.inline_box.is_none() && run_line_box_baseline_shift(run, parent_font_size) != 0.0
    });
    let (mut above, mut below) = if has_text_shift {
        line_shifted_text_extents(line, parent_font_size, custom_fonts)
    } else {
        let authored = line_authored_text_extents(line, custom_fonts);
        if authored.0 + authored.1 > 0.0 {
            authored
        } else {
            (
                ascender + strut_half_leading,
                descender + strut_half_leading,
            )
        }
    };
    let authored_total = above + below;
    if authored_total > 0.0 && line.height > authored_total {
        below += line.height - authored_total;
    }

    // A baseline-aligned inline box contributes `baseline_ascent` above the line
    // baseline and `height - baseline_ascent` below it (CSS2 §10.8.1). It raises
    // the line's ascent/descent ONLY when it extends past the strut's edges; a box
    // that fits inside the existing leading leaves the baseline put. A box without
    // a content baseline sits entirely above the baseline (its bottom edge rests
    // on it). Top/middle/bottom boxes don't move the baseline; they only widen the
    // line box, which `line.height` already reflects from the wrap pass.
    // x-height of the parent text (pt), for a `vertical-align: middle` box whose
    // centre sits at `baseline + x-height/2`.
    let line_x_height = line_primary_x_height_ratio(&line.runs, custom_fonts) * parent_font_size;
    for run in &line.runs {
        if let Some(inline) = run.inline_box.as_deref()
            && matches!(
                inline.vertical_align,
                VerticalAlign::Baseline
                    | VerticalAlign::Sub
                    | VerticalAlign::Super
                    | VerticalAlign::Length(_)
                    | VerticalAlign::Percent(_)
                    | VerticalAlign::Middle
            )
        {
            // Box ascent above its own baseline and descent below it.
            let box_ascent = inline.baseline_ascent.unwrap_or(inline.height);
            let box_descent = (inline.height - box_ascent).max(0.0);
            // Sub/super shift the box's baseline relative to the line baseline,
            // moving its extents by a fraction of the run font size; middle centres
            // the box on `baseline + x-height/2`.
            let (box_above, box_below) = match inline.vertical_align {
                VerticalAlign::Sub => (
                    box_ascent - parent_font_size * SUB_SHIFT_RATIO,
                    box_descent + parent_font_size * SUB_SHIFT_RATIO,
                ),
                VerticalAlign::Super => (
                    box_ascent + parent_font_size * SUPER_SHIFT_RATIO,
                    box_descent - parent_font_size * SUPER_SHIFT_RATIO,
                ),
                VerticalAlign::Length(v) => (box_ascent + v, box_descent - v),
                VerticalAlign::Percent(p) => {
                    let shift = run_line_height_for_vertical_align(run) * p;
                    (box_ascent + shift, box_descent - shift)
                }
                VerticalAlign::Middle => (
                    inline.height / 2.0 + line_x_height / 2.0,
                    inline.height / 2.0 - line_x_height / 2.0,
                ),
                _ => (box_ascent, box_descent),
            };
            above = above.max(box_above.max(0.0));
            below = below.max(box_below.max(0.0));
        }
    }

    LineBoxMetrics {
        ascender: above,
        descender: below,
        half_leading: 0.0,
    }
}

pub(super) fn flex_cell_align(cell: &FlexCell, align_items: AlignItems) -> AlignItems {
    cell.effective_cross_alignment(align_items)
}

/// Distance from a flex item's border-box top to its inline-block baseline.
pub(super) fn flex_cell_baseline(
    cell: &FlexCell,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<f32> {
    let mut prior = 0.0;
    let last = cell
        .lines
        .iter()
        .filter(|line| line.runs.iter().any(|run| !run.text.is_empty()))
        .inspect(|line| prior += line.height)
        .last();
    let Some(last) = last else {
        return cell
            .nested_elements
            .iter()
            .find_map(|element| element.atomic_inline_baseline())
            .map(|baseline| cell.border.top.width + cell.padding.top + baseline.baseline_offset());
    };
    prior -= last.height;
    let metrics = line_box_metrics(last, custom_fonts);
    Some(cell.border.top.width + cell.padding.top + prior + metrics.half_leading + metrics.ascender)
}

pub(super) fn flex_line_max_baseline(
    cells: &[FlexCell],
    line_id: FlexLineId,
    align_items: AlignItems,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<f32> {
    cells
        .iter()
        .filter(|cell| {
            cell.line_id == line_id && flex_cell_align(cell, align_items) == AlignItems::Baseline
        })
        .filter_map(|cell| flex_cell_baseline(cell, custom_fonts))
        .reduce(f32::max)
}

pub(super) fn upright_vertical_line_metrics(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> LineBoxMetrics {
    let (ascender, descender) = line
        .runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        .filter(|r| !is_drop_cap_run(r))
        .fold((0.0f32, 0.0f32), |(max_ascender, max_descender), run| {
            let (ascender_ratio, descender_ratio) =
                if crate::text::contains_cjk_vertical_text(&run.text) {
                    crate::text::upright_vertical_font_metrics(run, custom_fonts).map_or_else(
                        || {
                            crate::fonts::font_metrics_ratios(
                                run.css_font_family(),
                                run.bold,
                                run.font_style.is_slanted(),
                                custom_fonts,
                            )
                        },
                        |metrics| metrics.line_ratios(),
                    )
                } else {
                    crate::fonts::font_metrics_ratios(
                        run.css_font_family(),
                        run.bold,
                        run.font_style.is_slanted(),
                        custom_fonts,
                    )
                };
            (
                max_ascender.max(ascender_ratio * run.font_size),
                max_descender.max(descender_ratio * run.font_size),
            )
        });
    if ascender + descender == 0.0 {
        return line_box_metrics(line, custom_fonts);
    }
    // An upright CJK character or `text-combine-upright` composition occupies
    // an exact typographic square. A font's ascent plus descent may exceed
    // that slot; preserve its negative half-leading so the square stays
    // centered. Plain upright ASCII retains the established line-box metrics.
    let square_unit = line.runs.iter().any(|run| {
        run.metadata.text_combine_upright.is_active()
            || (run.inline_box.is_none()
                && run
                    .text
                    .chars()
                    .any(|ch| !ch.is_ascii() && !ch.is_whitespace()))
    });
    let half_leading = (line.height - (ascender + descender)) / 2.0;
    let half_leading = if square_unit {
        half_leading
    } else {
        half_leading.max(0.0)
    };
    LineBoxMetrics {
        ascender: ascender + half_leading,
        descender: descender + half_leading,
        half_leading: 0.0,
    }
}

/// Raw font ascent/descent of the PARENT's text content area on a line, i.e. the
/// extent of the parent's actual glyphs above/below the baseline WITHOUT the
/// strut's half-leading. `vertical-align: text-top`/`text-bottom` align an inline
/// box to these edges (css2 §10.8.1), which lie inside the line box when the line
/// is taller than the parent font box. Inline boxes themselves are excluded; if
/// the line carries no text, both are zero and callers fall back to the line-box
/// edge.
pub(super) fn line_text_content_extents(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> (f32, f32) {
    line.runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        .filter(|r| !is_drop_cap_run(r))
        .fold((0.0f32, 0.0f32), |(max_ascent, max_descent), run| {
            let (ascender_ratio, descender_ratio) = crate::fonts::font_metrics_ratios(
                run.css_font_family(),
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            );
            (
                max_ascent.max(ascender_ratio * run.font_size),
                max_descent.max(descender_ratio * run.font_size),
            )
        })
}

/// Estimate line width using TTF metrics for custom fonts.
pub(super) fn estimate_line_width_with_fonts(
    line: &TextLine,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    line.runs
        .iter()
        .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
        .sum()
}

/// Sanitize a font name for use as a PDF name object (remove spaces, special chars).
pub(crate) fn sanitize_pdf_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

pub(super) fn line_text_content(line: &TextLine) -> String {
    line.runs.iter().map(|r| r.text.as_str()).collect()
}

pub(super) fn fixed_textblock_flow_adjustments(elements: &[(f32, LayoutNode)]) -> Vec<f32> {
    let mut adjustment = 0.0;
    elements
        .iter()
        .map(|(_, element)| {
            let current = adjustment;
            if !element.is_page_paint_continuation() {
                adjustment += fixed_textblock_flow_overage(element);
            }
            current
        })
        .collect()
}

pub(super) fn element_uses_flow_y_adjustment(element: &dyn LayoutElement) -> bool {
    if element.is_page_paint_continuation() {
        return false;
    }

    struct UsesAdjustment(bool);

    impl LayoutVisitor for UsesAdjustment {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = !element.positioning.scheme.is_absolute();
        }
    }

    let mut uses = UsesAdjustment(true);
    element.accept(&mut uses);
    uses.0
}

pub(super) fn fixed_textblock_flow_overage(element: &dyn LayoutElement) -> f32 {
    #[derive(Default)]
    struct Overage(f32);

    impl LayoutVisitor for Overage {
        fn visit_text_block(&mut self, element: &TextBlock) {
            if !element.box_model.size.height.is_definite() {
                return;
            }
            let Some(block_height) = element.box_model.size.height.used() else {
                return;
            };
            if element.positioning.scheme.is_absolute()
                || element.flow.float != Float::None
                || element.clipping.rect.is_some()
            {
                return;
            }
            let text_height = element.lines.iter().map(|line| line.height).sum::<f32>();
            let content_height = element.box_model.padding.vertical() + text_height;
            self.0 = (content_height - block_height).max(0.0);
        }
    }

    let mut overage = Overage::default();
    element.accept(&mut overage);
    overage.0
}

pub(super) fn text_block_total_height(
    lines: &[TextLine],
    padding: EdgeSizes,
    block_height: Option<f32>,
    _clips: bool,
) -> f32 {
    let text_height: f32 = lines.iter().map(|line| line.height).sum();
    let content_h = padding.vertical() + text_height;
    // A provided `block_height` is the used padding-box height. Inline content
    // can overflow that box, but the box itself does not grow.
    block_height.unwrap_or(content_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upright_metrics_keep_ascii_in_its_one_em_slot() {
        let line = TextLine {
            runs: vec![TextRun {
                text: "12".to_string(),
                font_size: 24.0,
                font_family: FontFamily::Helvetica,
                metadata: crate::layout::engine::TextRunMetadata {
                    text_combine_upright: crate::style::computed::TextCombineUpright::All,
                    ..Default::default()
                },
                ..Default::default()
            }],
            height: 20.0,
            ..Default::default()
        };
        let fonts = HashMap::new();

        let metrics = upright_vertical_line_metrics(&line, &fonts);
        assert!((metrics.ascender + metrics.descender - line.height).abs() < f32::EPSILON);
    }
}
