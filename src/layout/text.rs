#[cfg(test)]
use crate::parser::css::SelectorContext;
use crate::parser::css::{AncestorInfo, CssRule, PseudoElement};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
// Re-export OverflowWrap so callers of TextWrapOptions::new can use it
// without a separate import.
pub(crate) use crate::style::computed::OverflowWrap;
use crate::style::computed::{
    BoxSizing, ComputedStyle, ContentItem, Display, Float, FontFamily, FontStyle,
    FontVariantPosition, FontWeight, IntrinsicWidthKeyword, LEADER_PLACEHOLDER_END,
    LEADER_PLACEHOLDER_START, TARGET_PLACEHOLDER_END, TARGET_PLACEHOLDER_START, VerticalAlign,
    WhiteSpace, compute_pseudo_element_style_with_font_metrics,
    compute_style_with_context_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::{CornerRadii, EdgeSizes};
use std::borrow::Cow;
use std::collections::HashMap;

use super::engine::{
    CounterState, FootnoteBodyStyle, FootnoteLinkData, InlineBox, InlineBoxPaint, LayoutBorder,
    RunWhitespace, SyntheticFontWeight, TextLine, TextRun, TextShaping, decode_footnote_link,
    encode_footnote_link_data,
};
use super::helpers::DropCap;
use super::inline_formatting::{
    GeneratedContentStyles, InlineContentSequence, InlineFormattingRole, InlineSiblingCursor,
};
use super::list_markers::format_counter_value;
use super::text_emphasis::TextEmphasisMetrics;

fn footnote_pseudo_content(style: Option<&ComputedStyle>, marker: &str) -> Option<String> {
    let items = &style?.content;
    (!items.is_empty()).then(|| {
        items
            .iter()
            .map(|item| match item {
                ContentItem::String(text) => text.clone(),
                ContentItem::Counter(name, counter_style)
                    if name.eq_ignore_ascii_case("footnote") =>
                {
                    marker.parse().map_or_else(
                        |_| marker.to_string(),
                        |value| format_counter_value(counter_style, value),
                    )
                }
                _ => String::new(),
            })
            .collect()
    })
}

fn resolve_target_attrs(runs: &mut [TextRun], element: &ElementNode) {
    for run in runs {
        if !run.text.contains(TARGET_PLACEHOLDER_START) {
            continue;
        }
        let needle = "attr(href)";
        if run.text.contains(needle) {
            let value = element
                .attributes
                .get("href")
                .map(String::as_str)
                .unwrap_or_default();
            run.text = run.text.replace(needle, value);
        }
    }
}

fn apply_inline_parent_background(
    runs: &mut [TextRun],
    start: usize,
    parent_style: &ComputedStyle,
    decoration_id: crate::layout::engine::InlineDecorationId,
) {
    if parent_style.white_space == WhiteSpace::Pre {
        return;
    }
    let Some(bg) = parent_style.background_color else {
        return;
    };
    let padding = decoration_padding(parent_style, Some(bg));
    let radius = decoration_radius(parent_style, Some(bg));
    for run in &mut runs[start..] {
        if run.inline_box.is_none() && run.background_color.is_none() {
            run.background_color = Some(bg);
            run.padding = padding;
            run.border_radii = radius;
        }
        if run.inline_box.is_none()
            && run.background_color == Some(bg)
            && !parent_style.writing_mode.is_vertical()
            && run.metadata.inline_decoration.is_none()
        {
            run.metadata.inline_decoration = Some(decoration_id);
        }
    }
}

/// Used advances at the opening and closing edge of an inline box.
///
/// Keeping the pair together makes edge ownership explicit after the DOM is
/// flattened into text runs. Vertical edges are paint-only for non-replaced
/// inline boxes and therefore do not belong in this inline-axis value.
#[derive(Clone, Copy)]
struct InlineHorizontalEdges {
    start: f32,
    end: f32,
    decoration: crate::layout::engine::InlineDecoration,
}

impl InlineHorizontalEdges {
    /// Resolve the physical horizontal edges of one computed inline style.
    fn from_style(
        style: &ComputedStyle,
        parent_direction_rtl: bool,
        decoration_id: crate::layout::engine::InlineDecorationId,
    ) -> Self {
        let left = style.margin.left + style.padding.left;
        let right = style.padding.right + style.margin.right;
        if parent_direction_rtl {
            return Self {
                start: right,
                end: left,
                decoration: Self::decoration(style, decoration_id),
            };
        }
        Self {
            start: left,
            end: right,
            decoration: Self::decoration(style, decoration_id),
        }
    }

    fn decoration(
        style: &ComputedStyle,
        id: crate::layout::engine::InlineDecorationId,
    ) -> crate::layout::engine::InlineDecoration {
        let background_color = style.background_color;
        crate::layout::engine::InlineDecoration {
            id,
            background_color,
            padding: decoration_padding(style, background_color),
            border_radii: decoration_radius(style, background_color),
        }
    }

    /// Add edge advances to the flattened content owned by this inline box.
    fn apply(self, runs: &mut Vec<TextRun>, start: usize) {
        let retains_painted_edges = self.decoration.background_color.is_some()
            && (!self.decoration.padding.is_zero() || !self.decoration.border_radii.is_zero());
        if self.start != 0.0 || retains_painted_edges {
            runs.insert(
                start,
                TextRun {
                    inline_box: Some(Box::new(InlineBox::advance_only(self.start))),
                    metadata: crate::layout::engine::TextRunMetadata {
                        inline_edge: Some(crate::layout::engine::InlineEdge {
                            side: crate::layout::engine::InlineEdgeSide::Opening,
                            decoration: self.decoration,
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }
        if self.end != 0.0 || retains_painted_edges {
            runs.push(TextRun {
                inline_box: Some(Box::new(InlineBox::advance_only(self.end))),
                metadata: crate::layout::engine::TextRunMetadata {
                    inline_edge: Some(crate::layout::engine::InlineEdge {
                        side: crate::layout::engine::InlineEdgeSide::Closing,
                        decoration: self.decoration,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
    }
}

/// Add one non-replaced inline box's horizontal edges to its run sequence.
fn apply_inline_horizontal_edges(
    runs: &mut Vec<TextRun>,
    start: usize,
    style: &ComputedStyle,
    parent_direction_rtl: bool,
    decoration_id: crate::layout::engine::InlineDecorationId,
) {
    if style.writing_mode.is_vertical() {
        return;
    }
    InlineHorizontalEdges::from_style(style, parent_direction_rtl, decoration_id)
        .apply(runs, start);
}

// ---------------------------------------------------------------------------
// resolve_style_font_family / resolved_line_height_factor
// ---------------------------------------------------------------------------

pub(crate) fn resolve_style_font_family(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> FontFamily {
    crate::system_fonts::resolve_font_family(
        &style.font_stack,
        fonts,
        style.font_weight.is_bold(),
        style.font_style.is_slanted(),
        style.font_stretch,
    )
}

/// CSS Fonts' per-face used font size.
///
/// `font-size-adjust` is inherited style state, but it must be applied only
/// after a concrete font face has been chosen for a text run. Box-model `em`
/// lengths and the computed line-height deliberately continue to use
/// `style.font_size`.
pub(crate) fn used_font_size(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> f32 {
    let family = resolve_style_font_family(style, fonts);
    let aspect = match &family {
        FontFamily::Custom(name) => crate::system_fonts::find_font(
            fonts,
            name,
            style.font_weight.is_bold(),
            style.font_style.is_slanted(),
        )
        .map_or(0.5, |(_, font)| {
            font.font_size_adjust_x_height_ratio(style.font_size)
        }),
        _ => 0.5,
    };
    style
        .font_size_adjust
        .used_font_size(style.font_size, aspect)
}

pub(crate) fn resolved_line_height_factor(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let used = used_line_height(style, fonts);
    if style.font_size > 0.0 {
        used / style.font_size
    } else {
        0.0
    }
}

/// The line-height multiplier a [`TextRun`] needs after `font-size-adjust`
/// has changed its used font size. The underlying line-height stays based on
/// the computed font size; only the run's multiplier changes representation.
pub(crate) fn text_run_line_height_factor(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let run_font_size = used_font_size(style, fonts);
    if run_font_size > 0.0 {
        used_line_height(style, fonts) / run_font_size
    } else {
        0.0
    }
}

pub(crate) fn used_line_height(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> f32 {
    if let Some(absolute) = style.line_height_absolute {
        absolute.max(0.0)
    } else if style.line_height.is_nan() {
        let font_family = resolve_style_font_family(style, fonts);
        crate::fonts::font_line_metrics(
            &font_family,
            style.font_size,
            style.font_weight.is_bold(),
            style.font_style.is_slanted(),
            fonts,
        )
        .normal_line_height()
    } else {
        style.font_size * style.line_height.max(0.0)
    }
}

fn style_run_bold(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> bool {
    if !style.font_weight.is_bold() {
        return false;
    }
    if style.font_synthesis_weight {
        return true;
    }
    let family = crate::system_fonts::resolve_font_family(
        &style.font_stack,
        fonts,
        true,
        style.font_style.is_slanted(),
        style.font_stretch,
    );
    match family {
        FontFamily::Custom(name) => {
            crate::system_fonts::find_font(fonts, &name, true, style.font_style.is_slanted())
                .is_some_and(|(_, font)| font.is_bold)
        }
        _ => true,
    }
}

fn style_run_font_style(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> FontStyle {
    if !style.font_style.is_slanted() {
        return FontStyle::Normal;
    }
    if style.font_synthesis_style {
        return style.font_style;
    }
    let family = crate::system_fonts::resolve_font_family(
        &style.font_stack,
        fonts,
        style.font_weight.is_bold(),
        true,
        style.font_stretch,
    );
    if match family {
        FontFamily::Custom(name) => {
            crate::system_fonts::find_font(fonts, &name, style.font_weight.is_bold(), true)
                .is_some_and(|(_, font)| font.is_italic)
        }
        _ => true,
    } {
        style.font_style
    } else {
        Default::default()
    }
}

pub(crate) fn mark_synthetic_weight_run(
    run: &mut TextRun,
    requested_weight: FontWeight,
    fonts: &HashMap<String, TtfFont>,
) {
    if !requested_weight.is_bold() || !matches!(run.font_family, FontFamily::Custom(_)) {
        return;
    }
    if run.bold
        && crate::system_fonts::needs_faux_bold(
            fonts,
            run.font_family.name(),
            run.bold,
            run.font_style.is_slanted(),
        )
    {
        run.font_synthesis.weight = SyntheticFontWeight::Auto;
    } else if !run.bold
        && crate::system_fonts::needs_faux_bold(
            fonts,
            run.font_family.name(),
            true,
            run.font_style.is_slanted(),
        )
    {
        run.font_synthesis.weight = SyntheticFontWeight::Suppressed;
    }
}

fn decoration_padding(style: &ComputedStyle, background: Option<crate::types::Color>) -> EdgeSizes {
    background.map_or(EdgeSizes::ZERO, |_| style.padding)
}

fn decoration_radius(
    style: &ComputedStyle,
    background: Option<crate::types::Color>,
) -> CornerRadii {
    if background.is_some() {
        return style.resolve_corner_radii(style.font_size, style.font_size);
    }
    CornerRadii::ZERO
}

fn styled_text_run(
    text: String,
    style: &ComputedStyle,
    link_url: Option<&str>,
    background_color: Option<crate::types::Color>,
    padding: EdgeSizes,
    fonts: &HashMap<String, TtfFont>,
) -> TextRun {
    let font_size = used_font_size(style, fonts);
    TextRun {
        text,
        font_size: font_size * style.font_variant_position.glyph_scale(),
        bold: style_run_bold(style, fonts),
        font_style: style_run_font_style(style, fonts),
        decorations: style.text_decorations.active(style.color),
        color: style.color,
        link_url: link_url.map(String::from),
        font_family: resolve_style_font_family(style, fonts),
        background_color,
        padding,
        border_radii: decoration_radius(style, background_color),
        line_height_factor: text_run_line_height_factor(style, fonts),
        line_height_basis: font_size,
        font_variant_position: style.font_variant_position,
        vertical_align: style.vertical_align,
        text_shadow: style.text_shadow.clone(),
        shaping: text_run_shaping(style),
        metadata: text_run_metadata(style),
        ..Default::default()
    }
}

pub(crate) fn text_run_metadata(style: &ComputedStyle) -> crate::layout::engine::TextRunMetadata {
    crate::layout::engine::TextRunMetadata {
        font_locale: style.font_locale,
        emphasis: crate::layout::text_emphasis::TextEmphasis {
            mark: style.text_emphasis_mark,
            color: style.text_emphasis_color,
            position: style.text_emphasis_position,
            ..Default::default()
        },
        spacing: crate::layout::elements::TextSpacing::from_style(style),
        text_combine_upright: style.text_combine_upright,
        is_drop_cap: false,
        ..Default::default()
    }
}

/// Resolve the OpenType feature controls that affect run geometry.
///
/// Non-zero `letter-spacing` prevents discretionary ligatures, but it does not
/// disable kerning: CSS Fonts resolves those controls independently.
pub(crate) fn text_run_shaping(style: &ComputedStyle) -> TextShaping {
    TextShaping {
        ligatures: style.ligatures_enabled,
        kerning: style.font_kerning_enabled,
    }
    .tracked(style.letter_spacing)
}

/// Resolve flattened paint-run boundaries while their nearest common inline
/// ancestor is still known.
///
/// A boundary already claimed by a nested inline remains untouched when an
/// outer scope is resolved. This preserves the CSS Text rule that tracking
/// belongs to the innermost element containing both typographic units.
pub(crate) trait InlineTextSequence {
    fn resolve_unclaimed_boundaries(&mut self, spacing: crate::layout::elements::TextSpacing);
}

impl InlineTextSequence for [TextRun] {
    fn resolve_unclaimed_boundaries(&mut self, spacing: crate::layout::elements::TextSpacing) {
        let mut previous: Option<usize> = None;
        for current in 0..self.len() {
            if self[current].forces_line_break() {
                previous = None;
                continue;
            }
            if !self[current].has_typographic_unit() {
                continue;
            }
            if let Some(previous_index) = previous {
                let (before, after) = self.split_at_mut(current);
                let previous_run = &mut before[previous_index];
                let current_run = &after[0];
                let advance = if previous_run.joins_typographically(current_run) {
                    spacing.letter
                } else {
                    Default::default()
                };
                previous_run
                    .metadata
                    .boundary
                    .resolve_letter_spacing(advance);
            }
            previous = Some(current);
        }
    }
}

fn is_drop_cap_marker_run(run: &TextRun) -> bool {
    run.inline_box.is_none() && run.metadata.is_drop_cap
}

fn estimate_text_width_for_run(text: &str, run: &TextRun, fonts: &HashMap<String, TtfFont>) -> f32 {
    let measured_text = target_placeholder_measure_text(text);
    let raw_width = crate::text::measure_text_width_with_shaping(
        &measured_text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        run.shaping,
        fonts,
    )
    .unwrap_or_else(|| {
        estimate_word_width(
            &measured_text,
            run.font_size,
            &run.font_family,
            run.bold,
            run.font_style.is_slanted(),
            fonts,
        )
    });
    run.metadata
        .spacing
        .add_internal_advance(raw_width, &measured_text)
}

/// Measure one complete laid-out run through the same advance model used by
/// the PDF painter.
pub(crate) fn measure_text_run_advance(run: &TextRun, fonts: &HashMap<String, TtfFont>) -> f32 {
    run.atomic_inline_advance()
        .unwrap_or_else(|| run.inline_advance(estimate_text_width_for_run(&run.text, run, fonts)))
}

fn target_placeholder_measure_text(text: &str) -> Cow<'_, str> {
    if !text.contains(TARGET_PLACEHOLDER_START) {
        return Cow::Borrowed(text);
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(TARGET_PLACEHOLDER_START) {
        out.push_str(&rest[..start]);
        let payload_start = start + TARGET_PLACEHOLDER_START.len();
        let Some(end_rel) = rest[payload_start..].find(TARGET_PLACEHOLDER_END) else {
            out.push_str(&rest[start..]);
            return Cow::Owned(out);
        };
        let payload = &rest[payload_start..payload_start + end_rel];
        if payload.starts_with("counter|") {
            out.push('0');
        }
        rest = &rest[payload_start + end_rel + TARGET_PLACEHOLDER_END.len()..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The parent (line) font size that a `vertical-align: super`/`sub` baseline
/// shift is measured against.
///
/// css2 §10.8.1 raises/lowers the box "to the position appropriate for
/// super/subscripts of the *parent's* box" — so the shift is a fraction of the
/// PARENT element's font size, NOT the shrunk `<sup>`/`<sub>`'s own (usually
/// `font-size: smaller`) size. Chrome confirms this: a 40%-size and a 100%-size
/// superscript on the same line are raised by the *same* amount. We take the
/// parent size as the largest baseline-aligned text run on the line (the
/// surrounding ordinary text the shifted run sits within); when the line has no
/// such text we fall back to the largest run present so a lone shifted run keeps
/// its own size.
pub(crate) fn line_primary_font_size(runs: &[crate::layout::engine::TextRun]) -> f32 {
    let baseline_text = runs
        .iter()
        .filter(|r| {
            r.inline_box.is_none()
                && matches!(r.vertical_align, VerticalAlign::Baseline)
                && has_non_collapsible_text(&r.text)
        })
        .map(TextRun::line_height_font_size)
        .fold(0.0f32, f32::max);
    if baseline_text > 0.0 {
        return baseline_text;
    }
    runs.iter()
        .filter(|r| r.inline_box.is_none())
        .map(TextRun::line_height_font_size)
        .fold(0.0f32, f32::max)
}

fn line_primary_x_height_ratio(runs: &[TextRun], fonts: &HashMap<String, TtfFont>) -> f32 {
    let primary = runs
        .iter()
        .filter(|run| {
            run.inline_box.is_none()
                && run.vertical_align == VerticalAlign::Baseline
                && has_non_collapsible_text(&run.text)
        })
        .max_by(|left, right| {
            left.font_size
                .partial_cmp(&right.font_size)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| runs.iter().find(|run| run.inline_box.is_none()));
    if let Some(run) = primary
        && let FontFamily::Custom(name) = run.css_font_family()
        && let Some((_, font)) =
            crate::system_fonts::find_font(fonts, name, run.bold, run.font_style.is_slanted())
    {
        return font.x_height_ratio();
    }
    0.5
}

// ---------------------------------------------------------------------------
// collapse_whitespace
// ---------------------------------------------------------------------------

/// CSS Text document white-space characters which collapse under `normal`.
/// Unicode space separators such as NBSP, EN SPACE, and EM SPACE are text;
/// Rust's broader `char::is_whitespace` predicate is not the CSS contract.
pub(crate) const fn is_collapsible_space(character: char) -> bool {
    matches!(character, '\u{0009}' | '\u{000A}' | '\u{000D}' | '\u{0020}')
}

/// Whether text contains anything outside the CSS document whitespace set.
pub(crate) fn has_non_collapsible_text(text: &str) -> bool {
    text.chars()
        .any(|character| !is_collapsible_space(character))
}

pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    for c in text.chars() {
        if is_collapsible_space(c) {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }
    result.trim_end_matches(is_collapsible_space).to_string()
}

/// Collapse whitespace for `white-space: pre-line` (css-text-3 §4.1.1).
///
/// Like `normal`, runs of spaces and tabs collapse to a single space and
/// collapsible spaces around a segment break are removed — BUT forced segment
/// breaks (newlines, U+000A) are *preserved* as explicit line breaks rather
/// than collapsed into a space. The newline is emitted verbatim so the line
/// breaker (which splits on `\n`) creates a forced break there.
pub(crate) fn collapse_whitespace_pre_line(text: &str) -> String {
    let mut result = String::new();
    // Tracks whether the previous emitted char was a collapsible space, so a
    // run of spaces collapses to one. Starts `true` so a leading space at the
    // very start of the block is dropped, and is reset to `true` after every
    // newline so a space leading the next segment is likewise dropped.
    let mut last_was_space = true;
    for c in text.chars() {
        if c == '\n' {
            // A segment break removes any collapsible space that precedes it
            // (and likewise will swallow following spaces via `last_was_space`).
            while result.ends_with(' ') {
                result.pop();
            }
            result.push('\n');
            last_was_space = true; // suppress a leading space on the next line
        } else if is_collapsible_space(c) {
            // Collapse spaces/tabs to a single space, but never lead a segment
            // (start of string or just after a newline) with one.
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }
    // Trailing collapsible space at the very end is removed.
    while result.ends_with(' ') {
        result.pop();
    }
    result
}

fn collect_inline_plain_text(nodes: &[DomNode], out: &mut String) {
    for node in nodes {
        match node {
            DomNode::Text(text) => {
                out.push_str(text);
                out.push(' ');
            }
            DomNode::Element(el) => collect_inline_plain_text(&el.children, out),
        }
    }
}

// ---------------------------------------------------------------------------
// expand_tabs
// ---------------------------------------------------------------------------

/// Expand `U+0009` TAB characters in preserved (`white-space: pre`/`pre-wrap`/
/// `break-spaces`) text to runs of spaces that advance to the next tab stop
/// (css-text-3 §6.3).
///
/// Tab stops are spaced `tab_size` apart, measured from the start of each line
/// (reset after every `\n`). `tab_size` is the resolved stop distance in points
/// (e.g. `tab-size: N` × the space advance, or an absolute `<length>`).
/// `space_advance` is the width of one space glyph; preceding content width is
/// accumulated from the actual glyph advances so the alignment is correct for
/// both monospace and proportional fonts. Each tab is replaced by the (>=1)
/// number of spaces whose total advance lands on (or just past) the next stop —
/// exact for monospace and a close approximation otherwise. Returns the input
/// unchanged when it contains no tab.
fn expand_tabs(
    text: &str,
    tab_size: f32,
    space_advance: f32,
    char_advance: impl Fn(char) -> f32,
) -> String {
    if !text.contains('\t') || tab_size <= 0.0 || space_advance <= 0.0 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    // Horizontal position since the start of the current line, in points.
    let mut x = 0.0f32;
    for ch in text.chars() {
        match ch {
            '\n' => {
                out.push('\n');
                x = 0.0;
            }
            '\t' => {
                // Distance to the next tab stop (a strictly positive advance:
                // a tab sitting exactly on a stop still moves to the next one).
                let next_stop = ((x / tab_size).floor() + 1.0) * tab_size;
                let needed = (next_stop - x).max(space_advance);
                // Round to the nearest whole number of spaces (>= 1).
                let count = (needed / space_advance).round().max(1.0) as usize;
                for _ in 0..count {
                    out.push(' ');
                }
                x += count as f32 * space_advance;
            }
            other => {
                out.push(other);
                x += char_advance(other);
            }
        }
    }
    out
}

/// Expand TABs in a preserved-whitespace text node using a style's resolved
/// `tab-size` and the space advance of its font (css-text-3 §6.3). No-op when
/// the text has no tab. Used by the `pre`/`pre-wrap`/`break-spaces` text paths.
pub(crate) fn expand_pre_tabs(
    text: &str,
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let bold = style.font_weight.is_bold();
    let italic = style.font_style.is_slanted();
    let family = resolve_style_font_family(style, fonts);
    let font_size = used_font_size(style, fonts);
    let space_advance = estimate_word_width(" ", font_size, &family, bold, italic, fonts);
    if space_advance <= 0.0 {
        return text.to_string();
    }
    // `tab_size >= 0` is a count of space advances; a negative value encodes an
    // absolute length in points (see `apply_style_map`).
    let tab_distance = if style.tab_size >= 0.0 {
        style.tab_size * space_advance
    } else {
        -style.tab_size
    };
    expand_tabs(text, tab_distance, space_advance, |c| {
        estimate_word_width(&c.to_string(), font_size, &family, bold, italic, fonts)
    })
}

// ---------------------------------------------------------------------------
// estimate_word_width
// ---------------------------------------------------------------------------

/// Estimate the width of a word given its font settings and available custom fonts.
pub(crate) fn estimate_word_width(
    word: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let cjk_em_width = word
        .chars()
        .filter(|ch| is_cjk_char(*ch) || is_cjk_closing_punctuation(*ch))
        .count() as f32
        * font_size;
    if let Some(width) =
        crate::text::measure_text_width(word, font_size, font_family, bold, italic, fonts)
    {
        return width.max(cjk_em_width);
    }

    // Use AFM metrics for standard fonts (non-bold for layout estimation)
    crate::fonts::str_width(word, font_size, font_family, false).max(cjk_em_width)
}

// ---------------------------------------------------------------------------
// TextWrapOptions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) struct TextWrapOptions {
    pub(crate) max_width: f32,
    pub(crate) default_font_size: f32,
    pub(crate) line_height_factor: f32,
    pub(crate) overflow_wrap: OverflowWrap,
    pub(crate) word_break_keep_all: bool,
    pub(crate) hyphens_manual: bool,
    /// Paragraph base direction for the Unicode Bidi Algorithm. Set to `true`
    /// when the containing block has `direction: rtl` (or `dir="rtl"`).
    pub(crate) paragraph_rtl: bool,
    /// `unicode-bidi: bidi-override`: lay the inline content out strictly in
    /// sequence according to `direction`, overriding intrinsic bidi classes
    /// (css-writing-modes-4 §2.4).
    pub(crate) bidi_override: bool,
    /// `unicode-bidi: plaintext`: resolve paragraph direction separately for
    /// forced-break-delimited text segments instead of using one base direction.
    pub(crate) bidi_plaintext: bool,
    /// Source whitespace and wrapping behavior for the current formatting
    /// context. Keeping these related choices together prevents `pre` from
    /// accidentally being treated like collapsed `normal` whitespace merely
    /// because its caller uses an unbounded wrapping width.
    whitespace: TextWhitespacePolicy,
    /// CSS `text-indent` applied to the first formatted line: it consumes inline
    /// space at the start of the first line, so that line has less room before it
    /// wraps. Subsequent lines are unaffected.
    pub(crate) text_indent: f32,
    /// Complete geometry for a dropped initial letter. Keeping its line span and
    /// side-bearing kerning together ensures the wrapped line origins agree with
    /// the originating inline glyph.
    pub(crate) drop_cap: Option<DropCap>,
    /// Zero-width inline box contributed by the containing block to each line.
    pub(crate) parent_strut: Option<LineStrut>,
}

impl TextWrapOptions {
    pub(crate) const fn new(
        max_width: f32,
        default_font_size: f32,
        line_height_factor: f32,
        overflow_wrap: OverflowWrap,
    ) -> Self {
        Self {
            max_width,
            default_font_size,
            line_height_factor,
            overflow_wrap,
            word_break_keep_all: false,
            hyphens_manual: true,
            paragraph_rtl: false,
            bidi_override: false,
            bidi_plaintext: false,
            whitespace: TextWhitespacePolicy::COLLAPSE,
            text_indent: 0.0,
            drop_cap: None,
            parent_strut: None,
        }
    }

    /// Apply a dropped initial letter's complete wrapping geometry.
    pub(crate) const fn with_drop_cap(mut self, drop_cap: Option<DropCap>) -> Self {
        self.drop_cap = drop_cap;
        self
    }

    fn drop_cap_exclusion_width(&self, line_index: usize) -> f32 {
        self.drop_cap
            .filter(|drop_cap| line_index > 0 && drop_cap.spans_line(line_index))
            .map_or(0.0, DropCap::exclusion_width)
    }

    fn drop_cap_line_inset(&self, line_index: usize) -> f32 {
        self.drop_cap
            .map_or(0.0, |drop_cap| drop_cap.line_inset(line_index))
    }

    pub(crate) const fn with_parent_strut(mut self, strut: LineStrut) -> Self {
        self.parent_strut = Some(strut);
        self
    }

    pub(crate) const fn with_rtl(mut self, rtl: bool) -> Self {
        self.paragraph_rtl = rtl;
        self
    }

    /// Set `unicode-bidi: bidi-override` for the inline content.
    pub(crate) const fn with_bidi_override(mut self, bidi_override: bool) -> Self {
        self.bidi_override = bidi_override;
        self
    }

    pub(crate) const fn with_bidi_plaintext(mut self, bidi_plaintext: bool) -> Self {
        self.bidi_plaintext = bidi_plaintext;
        self
    }

    pub(crate) const fn with_word_break_keep_all(mut self, keep_all: bool) -> Self {
        self.word_break_keep_all = keep_all;
        self
    }

    pub(crate) const fn with_hyphens_manual(mut self, manual: bool) -> Self {
        self.hyphens_manual = manual;
        self
    }

    pub(crate) const fn with_white_space(mut self, white_space: WhiteSpace) -> Self {
        self.whitespace = TextWhitespacePolicy::from_css(white_space);
        if !self.whitespace.allows_soft_wraps {
            self.max_width = f32::MAX;
        }
        self
    }

    pub(crate) const fn with_text_indent(mut self, text_indent: f32) -> Self {
        self.text_indent = text_indent;
        self
    }
}

/// The white-space behaviors the line breaker needs after CSS parsing.
///
/// The policy owns both source-space preservation and soft-wrap permission so
/// every formatting context applies `pre` and `nowrap` identically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextWhitespacePolicy {
    preserve_source: bool,
    wrap_preserved: bool,
    allows_soft_wraps: bool,
    break_spaces: bool,
}

impl TextWhitespacePolicy {
    const COLLAPSE: Self = Self {
        preserve_source: false,
        wrap_preserved: false,
        allows_soft_wraps: true,
        break_spaces: false,
    };

    const NO_WRAP: Self = Self {
        preserve_source: false,
        wrap_preserved: false,
        allows_soft_wraps: false,
        break_spaces: false,
    };

    const PRESERVE: Self = Self {
        preserve_source: true,
        wrap_preserved: false,
        allows_soft_wraps: false,
        break_spaces: false,
    };

    const PRE_WRAP: Self = Self {
        preserve_source: true,
        wrap_preserved: true,
        allows_soft_wraps: true,
        break_spaces: false,
    };

    const BREAK_SPACES: Self = Self {
        preserve_source: true,
        wrap_preserved: true,
        allows_soft_wraps: true,
        break_spaces: true,
    };

    const fn from_css(white_space: WhiteSpace) -> Self {
        match white_space {
            WhiteSpace::Pre => Self::PRESERVE,
            WhiteSpace::PreWrap => Self::PRE_WRAP,
            WhiteSpace::BreakSpaces => Self::BREAK_SPACES,
            WhiteSpace::NoWrap => Self::NO_WRAP,
            WhiteSpace::Normal | WhiteSpace::PreLine => Self::COLLAPSE,
        }
    }
}

/// The containing block contributes an invisible zero-width "strut" to every
/// line box (CSS2 10.8.1). These are its extents about the shared baseline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineStrut {
    pub(crate) above: f32,
    pub(crate) below: f32,
}

impl LineStrut {
    /// Resolve the containing-block strut for a single font.
    ///
    /// Document flow uses the same CSS-pixel metric resolution as Chromium's
    /// HTML line layout. This resolves font metrics once; fractional line
    /// advances remain fractional through PDF serialization.
    pub(crate) fn from_font(
        family: &FontFamily,
        font_size: f32,
        bold: bool,
        italic: bool,
        line_height: f32,
        fonts: &HashMap<String, TtfFont>,
    ) -> Self {
        let metrics = crate::fonts::font_line_metrics(family, font_size, bold, italic, fonts);
        Self::from_metrics(metrics, line_height)
    }

    /// Resolve a strut from unquantized font metrics.
    ///
    /// Page-margin boxes are centered directly in PDF coordinates. Their
    /// fractional metric contribution must survive that centering operation.
    pub(crate) fn from_exact_font(
        family: &FontFamily,
        font_size: f32,
        bold: bool,
        italic: bool,
        line_height: f32,
        fonts: &HashMap<String, TtfFont>,
    ) -> Self {
        let metrics = crate::fonts::exact_font_line_metrics(family, font_size, bold, italic, fonts);
        let leading = line_height - metrics.ascent - metrics.descent;
        Self::from_above(metrics.ascent + leading / 2.0, line_height)
    }

    fn from_metrics(metrics: crate::fonts::FontLineMetrics, line_height: f32) -> Self {
        let leading = line_height - metrics.ascent - metrics.descent;
        let above_leading = crate::fonts::floor_to_css_pixel(leading / 2.0);
        Self::from_above(metrics.ascent + above_leading, line_height)
    }

    fn from_above(above: f32, line_height: f32) -> Self {
        Self {
            above,
            below: line_height - above,
        }
    }
}

fn line_extents(
    family: &FontFamily,
    font_size: f32,
    bold: bool,
    italic: bool,
    line_height: f32,
    fonts: &HashMap<String, TtfFont>,
) -> LineStrut {
    LineStrut::from_font(family, font_size, bold, italic, line_height, fonts)
}

/// Resolve the containing block's line strut once, before its child runs are
/// consumed by the wrapping pass.
pub(crate) fn parent_line_strut(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> LineStrut {
    let family = resolve_style_font_family(style, fonts);
    let font_size = used_font_size(style, fonts);
    line_extents(
        &family,
        font_size,
        style.font_weight.is_bold(),
        style.font_style.is_slanted(),
        used_line_height(style, fonts),
        fonts,
    )
}

/// Resolve a line's used height and baseline from its inline boxes. A scalar
/// `max(line-height)` is insufficient when one run has the tallest ascent while
/// another run (including the parent strut) has the deepest descent: CSS2 10.8.1
/// takes the maximum independently on each side of the shared baseline.
fn resolve_line_box_metrics(
    runs: &[TextRun],
    parent: Option<LineStrut>,
    fallback_line_height_factor: f32,
    fonts: &HashMap<String, TtfFont>,
) -> Option<(f32, f32)> {
    let mut above = parent.map_or(f32::NEG_INFINITY, |strut| strut.above);
    let mut below = parent.map_or(f32::NEG_INFINITY, |strut| strut.below);
    let shift_basis = line_primary_font_size(runs);
    let line_x_height = line_primary_x_height_ratio(runs, fonts) * shift_basis;
    for run in runs {
        if let Some(inline) = run.inline_box.as_deref() {
            let box_ascent = inline.baseline_ascent.unwrap_or(inline.height);
            let box_descent = (inline.height - box_ascent).max(0.0);
            let (box_above, box_below) = match inline.vertical_align {
                VerticalAlign::Sub => (
                    box_ascent - shift_basis * crate::render::pdf::SUB_SHIFT_RATIO,
                    box_descent + shift_basis * crate::render::pdf::SUB_SHIFT_RATIO,
                ),
                VerticalAlign::Super => (
                    box_ascent + shift_basis * crate::render::pdf::SUPER_SHIFT_RATIO,
                    box_descent - shift_basis * crate::render::pdf::SUPER_SHIFT_RATIO,
                ),
                VerticalAlign::Length(value) => (box_ascent + value, box_descent - value),
                VerticalAlign::Percent(percent) => {
                    let factor = if run.line_height_factor.is_finite() {
                        run.line_height_factor.max(0.0)
                    } else {
                        fallback_line_height_factor.max(0.0)
                    };
                    let shift = run.line_height_font_size() * factor * percent;
                    (box_ascent + shift, box_descent - shift)
                }
                VerticalAlign::Middle => (
                    inline.height / 2.0 + line_x_height / 2.0,
                    inline.height / 2.0 - line_x_height / 2.0,
                ),
                VerticalAlign::Baseline => (box_ascent, box_descent),
                // These align after the baseline-aligned line box has been
                // established. Their own box height is included below.
                VerticalAlign::Top
                | VerticalAlign::TextTop
                | VerticalAlign::Bottom
                | VerticalAlign::TextBottom => continue,
            };
            above = above.max(box_above.max(0.0));
            below = below.max(box_below.max(0.0));
            continue;
        }
        if is_drop_cap_marker_run(run) || !has_non_collapsible_text(&run.text) {
            continue;
        }
        let factor = if run.line_height_factor.is_finite() {
            run.line_height_factor.max(0.0)
        } else {
            fallback_line_height_factor.max(0.0)
        };
        let extents = line_extents(
            run.css_font_family(),
            run.line_height_font_size(),
            run.bold,
            run.font_style.is_slanted(),
            run.line_height_font_size() * factor,
            fonts,
        );
        let shift = run.vertical_align_shift(shift_basis);
        above = above.max(extents.above + shift);
        below = below.max(extents.below - shift);
    }
    // `text-emphasis` uses a ruby annotation (css-text-decor-4 §3.4). When its
    // selected annotation side cannot fit in the base run's leading, Chrome
    // extends the line's block-end extent. `over` may also lower the base into
    // block-start leading; `under` keeps that base baseline in place. Multiple
    // emphasized runs share one line-box expansion.
    let emphasis_end_extension = runs
        .iter()
        .map(|run| TextEmphasisMetrics::from_run(run).line_box_end_extension)
        .fold(0.0f32, f32::max);
    below += emphasis_end_extension;
    if !above.is_finite() || !below.is_finite() {
        return None;
    }
    let deferred_box_height = runs
        .iter()
        .filter_map(|run| run.inline_box.as_deref())
        .filter(|inline| {
            matches!(
                inline.vertical_align,
                VerticalAlign::Top
                    | VerticalAlign::TextTop
                    | VerticalAlign::Bottom
                    | VerticalAlign::TextBottom
            )
        })
        .map(|inline| inline.height)
        .fold(0.0f32, f32::max);
    Some(((above + below).max(deferred_box_height), above))
}

/// One wrappable token (a word, a preserved space-run, a `\n`, or an atomic
/// inline box) together with the style it was split from and two flags that
/// control inter-word spacing.
struct StyledWord {
    text: String,
    /// Index into the prepared run arena owned by the wrapping/measurement
    /// pass. Tokens never duplicate the comparatively large style record.
    run_index: usize,
    /// The token preserves its own internal whitespace (pre-wrap space runs,
    /// atomic boxes): the wrapper never injects an inter-word space before it.
    preserve_spacing: bool,
    /// The token is directly adjacent to the previous token in the source with
    /// no collapsible whitespace at the boundary (e.g. a `::before` pseudo run
    /// abutting the element's own text). The wrapper must not synthesise an
    /// inter-word space before it even though it starts a new run.
    joins_prev: bool,
    /// A legal soft-wrap opportunity exists immediately before this token.
    /// This is independent from spacing: a segment after a hyphen joins the
    /// previous segment visually but may wrap, while adjacent styled runs in a
    /// single source word join visually and may not wrap between the runs.
    break_before: bool,
    /// The source run contains a typographic unit immediately before this
    /// token. If both remain on one line, their mechanical token boundary
    /// becomes one ordinary internal tracking interval.
    has_internal_predecessor: bool,
    /// Whether this token owns the source run's outgoing inline boundary.
    /// Wrapping can split one run into multiple tokens, but its boundary
    /// advance must be charged exactly once.
    ends_run: bool,
}

impl StyledWord {
    fn leading_tracking(&self, run: &TextRun, line_has_content: bool) -> f32 {
        if line_has_content && self.has_internal_predecessor && !self.joins_prev {
            run.metadata.spacing.letter
        } else {
            0.0
        }
    }

    fn outgoing_advance(&self, run: &TextRun) -> f32 {
        if self.ends_run {
            run.metadata.boundary.total()
        } else {
            Default::default()
        }
    }
}

/// Intrinsic inline sizes derived from the same styled token stream and width
/// arithmetic used by [`wrap_text_runs`].  Keeping these in points avoids any
/// pixel-grid or raster-output adjustment in layout geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextIntrinsicWidths {
    pub(crate) min_content: f32,
    pub(crate) max_content: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterWordSpace {
    None,
    /// The collapsed space is shaped together with the following word.
    CurrentRun,
    /// A highlighted following run must not paint the preceding space, so the
    /// space is shaped with the previous run and emitted separately.
    PreviousRun,
}

/// Width pieces for one ordinary text token.  `end_width` deliberately applies
/// the pieces in emission order.  This is important at an exact fit: changing
/// `(line + space) + word` into `line + (space + word)` can change an `f32` by
/// one ULP even when all three inputs are identical.
#[derive(Clone, Copy, Debug)]
struct NormalTokenMeasurement {
    inter_word_space: InterWordSpace,
    leading_width: f32,
    text_width: f32,
    word_width: f32,
}

impl NormalTokenMeasurement {
    fn end_width(self, line_width: f32) -> f32 {
        let with_leading = if self.inter_word_space == InterWordSpace::PreviousRun {
            line_width + self.leading_width
        } else {
            line_width
        };
        with_leading + self.text_width
    }
}

#[derive(Clone, Copy)]
struct NormalTokenContext<'a> {
    preceding_runs: &'a [TextRun],
    line_has_content: bool,
    previous_ends_whitespace: bool,
    preserve_spacing: bool,
    joins_previous: bool,
    leading_tracking: f32,
    outgoing_advance: f32,
}

fn measure_normal_token(
    paint_word: &str,
    template: &TextRun,
    context: NormalTokenContext<'_>,
    fonts: &HashMap<String, TtfFont>,
) -> NormalTokenMeasurement {
    let word_width =
        estimate_text_width_for_run(paint_word, template, fonts) + context.outgoing_advance;
    let contextual_width = context
        .joins_previous
        .then(|| joined_token_advance(context.preceding_runs, paint_word, template, fonts))
        .flatten()
        .map(|width| width + context.outgoing_advance)
        .unwrap_or(word_width);
    let needs_space = context.line_has_content
        && !context.preserve_spacing
        && !context.previous_ends_whitespace
        && !context.joins_previous;
    if !needs_space {
        return NormalTokenMeasurement {
            inter_word_space: InterWordSpace::None,
            leading_width: 0.0,
            text_width: context.leading_tracking + contextual_width,
            word_width,
        };
    }

    let previous_run = context.preceding_runs.last();
    let previous_background = previous_run.and_then(|run| run.background_color);
    if previous_background != template.background_color && template.background_color.is_some() {
        let previous_run = previous_run.unwrap_or(template);
        NormalTokenMeasurement {
            inter_word_space: InterWordSpace::PreviousRun,
            leading_width: context.leading_tracking
                + estimate_text_width_for_run(" ", previous_run, fonts),
            text_width: word_width,
            word_width,
        }
    } else {
        let mut spaced = String::with_capacity(paint_word.len() + 1);
        spaced.push(' ');
        spaced.push_str(paint_word);
        NormalTokenMeasurement {
            inter_word_space: InterWordSpace::CurrentRun,
            leading_width: 0.0,
            text_width: context.leading_tracking
                + estimate_text_width_for_run(&spaced, template, fonts)
                + context.outgoing_advance,
            word_width,
        }
    }
}

/// Incremental advance for a token that remains in the shaping buffer of the
/// immediately preceding source text. The whole trailing buffer is reshaped,
/// then its prior advance is subtracted; this retains ligatures, contextual
/// substitutions, pair positioning, and letter spacing across soft-wrap token
/// boundaries without allowing shaping across a paint boundary.
fn joined_token_advance(
    preceding_runs: &[TextRun],
    text: &str,
    template: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let last = preceding_runs.last()?;
    if !crate::text::text_runs_share_shaping_buffer(last, template) {
        return None;
    }

    let mut start = preceding_runs.len() - 1;
    while start > 0
        && crate::text::text_runs_share_shaping_buffer(
            &preceding_runs[start - 1],
            &preceding_runs[start],
        )
    {
        start -= 1;
    }
    let prefix = preceding_runs[start..]
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    let mut combined = String::with_capacity(prefix.len() + text.len());
    combined.push_str(&prefix);
    combined.push_str(text);

    let prefix_width = estimate_text_width_for_run(&prefix, &preceding_runs[start], fonts);
    let measured_combined = estimate_text_width_for_run(&combined, template, fonts);
    let advance = measured_combined - prefix_width;
    advance.is_finite().then_some(advance)
}

#[derive(Clone, Copy, Debug)]
struct InlineTokenMeasurement {
    leading_width: f32,
    box_width: f32,
}

impl InlineTokenMeasurement {
    fn end_width(self, line_width: f32) -> f32 {
        (line_width + self.leading_width) + self.box_width
    }
}

fn measure_inline_token(
    template: &TextRun,
    line_has_content: bool,
    previous_ends_whitespace: bool,
    joins_prev: bool,
    fonts: &HashMap<String, TtfFont>,
) -> InlineTokenMeasurement {
    let leading_width = if line_has_content && !joins_prev && !previous_ends_whitespace {
        estimate_text_width_for_run(" ", template, fonts)
    } else {
        0.0
    };
    InlineTokenMeasurement {
        leading_width,
        box_width: template.atomic_inline_advance().unwrap_or_default(),
    }
}

/// Measure an opening edge together with the first token inside its inline box.
///
/// CSS2 keeps the opening edge on the fragment containing the inline's first
/// content. Looking ahead here prevents the edge from fitting alone at the end
/// of a line while preserving the source-order run sequence used for paint.
fn opening_edge_group_width<'a>(
    edge: &TextRun,
    following: impl Iterator<Item = &'a StyledWord>,
    runs: &[TextRun],
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let mut width = edge.atomic_inline_advance().unwrap_or_default();
    for token in following {
        let run = &runs[token.run_index];
        if run.is_opening_inline_edge() {
            width += run.atomic_inline_advance().unwrap_or_default();
            continue;
        }
        if token.text == "\n" {
            return width;
        }
        if run.inline_box.is_some() {
            return width + run.atomic_inline_advance().unwrap_or_default();
        }
        return width + estimate_text_width_for_run(&strip_soft_hyphens(&token.text), run, fonts);
    }
    width
}

/// Split a segment of text that preserves its internal whitespace into
/// alternating word-runs and space-runs. Used for `white-space: pre-wrap`,
/// where spaces must be preserved verbatim but lines may still wrap at the
/// boundary between a space-run and the following word. Each emitted token is
/// flagged `preserve_spacing = true` so the wrapper never injects its own
/// inter-word space; soft wrapping then happens via the generic
/// "token overflows the line" break.
fn split_preserving_spaces(segment: &str, run_index: usize, out: &mut Vec<StyledWord>) {
    let mut current = String::new();
    let mut current_is_space: Option<bool> = None;
    for ch in segment.chars() {
        let is_space = ch == ' ' || ch == '\t';
        if current_is_space != Some(is_space) && !current.is_empty() {
            out.push(StyledWord {
                text: std::mem::take(&mut current),
                run_index,
                preserve_spacing: true,
                joins_prev: false,
                break_before: true,
                has_internal_predecessor: false,
                ends_run: false,
            });
        }
        current_is_space = Some(is_space);
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(StyledWord {
            text: current,
            run_index,
            preserve_spacing: true,
            joins_prev: false,
            break_before: true,
            has_internal_predecessor: false,
            ends_run: false,
        });
    }
}

/// Split a whitespace-delimited word at UAX #14 hyphen opportunities.
///
/// The Unicode algorithm supplies the base opportunities; CSS Text 4's
/// interoperability tailoring additionally permits a break before a digit when
/// the hyphen follows a letter or digit (for example `LABEL-02`). The hyphen
/// remains on the preceding segment, and later segments stay in the same
/// shaping buffer.
fn push_word_with_hyphen_breaks(
    word: &str,
    run_index: usize,
    out: &mut Vec<StyledWord>,
    keep_all: bool,
    hyphens_manual: bool,
) {
    // Cross-reference content is resolved only after pagination establishes
    // target pages. Keep its internal marker as one token: a normal hyphen
    // break would split the marker across runs and make the resolver unable to
    // recognize it. Its width is measured as the final page-number placeholder.
    if word.contains(TARGET_PLACEHOLDER_START) {
        out.push(StyledWord {
            text: word.to_string(),
            run_index,
            preserve_spacing: false,
            joins_prev: false,
            break_before: true,
            has_internal_predecessor: false,
            ends_run: false,
        });
        return;
    }

    if should_break_as_char_tokens(word, keep_all) {
        push_char_break_tokens(word, run_index, out);
        return;
    }

    let break_offsets = unicode_linebreak::linebreaks(word)
        .map(|(offset, _)| offset)
        .filter(|offset| *offset < word.len())
        .filter(|offset| {
            word[..*offset]
                .chars()
                .next_back()
                .is_some_and(|character| {
                    character == '-' || (hyphens_manual && character == '\u{00ad}')
                })
        })
        .collect::<Vec<_>>();
    let mut seg = String::new();
    let mut first = true;
    for (offset, character) in word.char_indices() {
        seg.push(character);
        let boundary = offset + character.len_utf8();
        let css_hyphen_digit_tailoring = character == '-'
            && word[..offset].chars().next_back().is_some_and(|preceding| {
                preceding.is_alphabetic()
                    || unicode_linebreak::break_property(preceding as u32)
                        == unicode_linebreak::BreakClass::Numeric
            })
            && word[boundary..].chars().next().is_some_and(|following| {
                unicode_linebreak::break_property(following as u32)
                    == unicode_linebreak::BreakClass::Numeric
            });
        if break_offsets.binary_search(&boundary).is_ok() || css_hyphen_digit_tailoring {
            out.push(StyledWord {
                text: std::mem::take(&mut seg),
                run_index,
                preserve_spacing: false,
                joins_prev: !first,
                break_before: true,
                has_internal_predecessor: false,
                ends_run: false,
            });
            first = false;
        }
    }
    if !seg.is_empty() {
        out.push(StyledWord {
            text: seg,
            run_index,
            preserve_spacing: false,
            joins_prev: !first,
            break_before: true,
            has_internal_predecessor: false,
            ends_run: false,
        });
    }
}

fn strip_soft_hyphens(text: &str) -> String {
    if !text.contains('\u{00ad}') {
        return text.to_string();
    }
    text.chars().filter(|&c| c != '\u{00ad}').collect()
}

fn finalize_soft_hyphen_line(runs: &mut [TextRun], broke_line: bool) {
    let Some(last_idx) = runs.iter().rposition(|run| run.inline_box.is_none()) else {
        return;
    };
    for (idx, run) in runs.iter_mut().enumerate() {
        if !run.text.contains('\u{00ad}') {
            continue;
        }
        if broke_line && idx == last_idx && run.text.ends_with('\u{00ad}') {
            while run.text.ends_with('\u{00ad}') {
                run.text.pop();
            }
            run.text.push('-');
        } else {
            run.text = strip_soft_hyphens(&run.text);
        }
    }
}

fn resolved_text_line(
    runs: Vec<TextRun>,
    height: f32,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> TextLine {
    let metrics = resolve_line_box_metrics(
        &runs,
        options.parent_strut,
        options.line_height_factor,
        fonts,
    );
    TextLine {
        runs,
        height: metrics.map_or(height, |(used_height, _)| used_height),
        baseline_ascent: metrics.map(|(_, baseline)| baseline),
        x_offset: 0.0,
        metadata: Default::default(),
    }
}

/// Retain horizontal decoration only where the inline's physical edge exists.
///
/// A non-replaced inline split across lines paints its background on every
/// fragment, but its left padding/radii belong only to the leftmost fragment
/// and its right padding/radii only to the rightmost one (CSS2 §9.4.2).
fn resolve_inline_fragment_decorations(lines: &mut [TextLine]) {
    for line in lines {
        let edges: Vec<_> = line
            .runs
            .iter()
            .filter_map(|run| run.metadata.inline_edge)
            .collect();
        let decoration_ids: Vec<_> = line
            .runs
            .iter()
            .filter_map(|run| run.metadata.inline_decoration)
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });
        for decoration_id in decoration_ids {
            let matching: Vec<_> = line
                .runs
                .iter()
                .enumerate()
                .filter(|(_, run)| run.metadata.inline_decoration == Some(decoration_id))
                .map(|(index, _)| index)
                .collect();
            let (Some(first), Some(last)) = (matching.first().copied(), matching.last().copied())
            else {
                continue;
            };
            let opening = edges.iter().find(|edge| {
                edge.decoration.id == decoration_id
                    && edge.side == crate::layout::engine::InlineEdgeSide::Opening
            });
            let closing = edges.iter().find(|edge| {
                edge.decoration.id == decoration_id
                    && edge.side == crate::layout::engine::InlineEdgeSide::Closing
            });
            for index in matching {
                let run = &mut line.runs[index];
                run.padding.left = 0.0;
                run.padding.right = 0.0;
                run.border_radii = run.border_radii.clear_left();
                run.border_radii = run.border_radii.clear_right();
            }
            if let Some(edge) = opening {
                let run = &mut line.runs[first];
                run.padding.left = edge.decoration.padding.left;
                run.border_radii.top_left = edge.decoration.border_radii.top_left;
                run.border_radii.bottom_left = edge.decoration.border_radii.bottom_left;
            }
            if let Some(edge) = closing {
                let run = &mut line.runs[last];
                run.padding.right = edge.decoration.padding.right;
                run.border_radii.top_right = edge.decoration.border_radii.top_right;
                run.border_radii.bottom_right = edge.decoration.border_radii.bottom_right;
            }
        }
    }
}

fn push_wrapped_line(
    lines: &mut Vec<TextLine>,
    runs: &mut Vec<TextRun>,
    height: f32,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) {
    finalize_soft_hyphen_line(runs, true);
    if let Some(last) = runs.last_mut() {
        last.metadata.boundary.discard();
    }
    lines.push(resolved_text_line(
        std::mem::take(runs),
        height,
        options,
        fonts,
    ));
}

fn is_cjk_char(ch: char) -> bool {
    let c = ch as u32;
    (0x3040..=0x30ff).contains(&c)
        || (0x3400..=0x4dbf).contains(&c)
        || (0x4e00..=0x9fff).contains(&c)
        || (0xf900..=0xfaff).contains(&c)
        || (0xac00..=0xd7af).contains(&c)
}

fn is_cjk_closing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '、' | '。' | '，' | '．' | '！' | '？' | '）' | '］' | '｝' | '」' | '』'
    )
}

fn should_break_as_char_tokens(word: &str, keep_all: bool) -> bool {
    if word.chars().count() <= 1 {
        return false;
    }
    let has_cjk = word.chars().any(is_cjk_char);
    if has_cjk {
        return !keep_all;
    }
    // `line-break:anywhere` is not represented in ComputedStyle yet, but CSS
    // Text's anywhere behavior is needed for punctuation-heavy unspaced runs.
    // Restrict arbitrary Latin breaks to runs that carry visible punctuation so
    // ordinary prose words still keep their normal unbreakable min-content.
    if word
        .chars()
        .any(|ch| matches!(ch, '/' | '+' | '(' | ')' | '[' | ']' | '{' | '}'))
    {
        return true;
    }
    false
}

fn push_char_break_tokens(word: &str, run_index: usize, out: &mut Vec<StyledWord>) {
    let mut first = true;
    for ch in word.chars() {
        if is_cjk_closing_punctuation(ch)
            && let Some(prev) = out.last_mut()
        {
            prev.text.push(ch);
            continue;
        }
        out.push(StyledWord {
            text: ch.to_string(),
            run_index,
            preserve_spacing: false,
            joins_prev: !first,
            break_before: true,
            has_internal_predecessor: false,
            ends_run: false,
        });
        first = false;
    }
}

// ---------------------------------------------------------------------------
// split_word_to_fit
// ---------------------------------------------------------------------------

/// Split a long word at the last character boundary that still fits within
/// `available_width`, without inserting hyphen characters.
pub(crate) fn split_word_to_fit(
    word: &str,
    available_width: f32,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<(String, String)> {
    if word.is_empty() || available_width <= 0.0 {
        return None;
    }

    let mut best_boundary = None;
    for (index, _) in word.char_indices().skip(1) {
        let prefix = &word[..index];
        let prefix_width = estimate_word_width(prefix, font_size, font_family, bold, italic, fonts);
        if prefix_width <= available_width {
            best_boundary = Some(index);
        } else {
            break;
        }
    }

    let boundary = best_boundary?;
    Some((word[..boundary].to_string(), word[boundary..].to_string()))
}

// ---------------------------------------------------------------------------
// wrap_text_runs
// ---------------------------------------------------------------------------

fn expand_leader_placeholders(
    runs: Vec<TextRun>,
    max_width: f32,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextRun> {
    let leader_count: usize = runs
        .iter()
        .map(|run| run.text.matches(LEADER_PLACEHOLDER_START).count())
        .sum();
    if leader_count == 0 {
        return runs;
    }

    let base_width: f32 = runs
        .iter()
        .map(|run| {
            if run.inline_box.is_some() {
                run.atomic_inline_advance().unwrap_or_default()
            } else {
                let text = remove_leader_placeholders(&run.text);
                estimate_word_width(
                    &text,
                    run.font_size,
                    &run.font_family,
                    run.bold,
                    run.font_style.is_slanted(),
                    fonts,
                )
            }
        })
        .sum();
    let available = if max_width.is_finite() && max_width < 100_000.0 {
        (max_width - base_width).max(0.0) / leader_count as f32
    } else {
        0.0
    };

    runs.into_iter()
        .flat_map(|run| {
            if run.text.contains(LEADER_PLACEHOLDER_START) && run.inline_box.is_none() {
                expand_leader_run(run, available, fonts)
            } else {
                vec![run]
            }
        })
        .collect()
}

fn remove_leader_placeholders(text: &str) -> String {
    replace_leader_placeholders_raw(text, |_| String::new())
}

fn expand_leader_run(
    run: TextRun,
    available: f32,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut rest = run.text.as_str();
    while let Some(start) = rest.find(LEADER_PLACEHOLDER_START) {
        push_leader_text_run(&mut out, &run, &rest[..start]);
        let payload_start = start + LEADER_PLACEHOLDER_START.len();
        let Some(end_rel) = rest[payload_start..].find(LEADER_PLACEHOLDER_END) else {
            push_leader_text_run(&mut out, &run, &rest[start..]);
            return out;
        };
        let pattern = &rest[payload_start..payload_start + end_rel];
        let (leading_space, leader, trailing_space) =
            leader_replacement_parts(pattern, available, &run, fonts);
        if leading_space > 0.0 {
            out.push(leader_spacer_run(&run, leading_space));
        }
        push_leader_text_run(&mut out, &run, &leader);
        if trailing_space > 0.0 {
            out.push(leader_spacer_run(&run, trailing_space));
        }
        rest = &rest[payload_start + end_rel + LEADER_PLACEHOLDER_END.len()..];
    }
    push_leader_text_run(&mut out, &run, rest);
    out
}

fn push_leader_text_run(out: &mut Vec<TextRun>, template: &TextRun, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push(TextRun {
        text: text.to_string(),
        ..template.clone()
    });
}

fn leader_spacer_run(template: &TextRun, width: f32) -> TextRun {
    TextRun {
        text: String::new(),
        background_color: None,
        padding: EdgeSizes::ZERO,
        border_radii: CornerRadii::ZERO,
        inline_box: Some(Box::new(InlineBox {
            width,
            vertical_align: template.vertical_align,
            baseline_ascent: Some(0.0),
            ..InlineBox::default()
        })),
        ..template.clone()
    }
}

fn leader_replacement_parts(
    pattern: &str,
    available: f32,
    run: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> (f32, String, f32) {
    let pattern = if pattern.is_empty() { "." } else { pattern };
    let pattern_width = estimate_word_width(
        pattern,
        run.font_size,
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        fonts,
    );
    let max_count = if pattern_width > 0.0 && available > 0.0 {
        (available / pattern_width).floor() as usize
    } else {
        0
    }
    .min(512);
    let hidden_count = usize::from(max_count > 0);
    let mut count = max_count.saturating_sub(hidden_count);
    while count > 0 {
        let candidate = pattern.repeat(count);
        let width = estimate_word_width(
            &candidate,
            run.font_size,
            &run.font_family,
            run.bold,
            run.font_style.is_slanted(),
            fonts,
        );
        if width <= available {
            let hidden = pattern_width * hidden_count as f32;
            let leading = (available - width - hidden).max(0.0);
            return (leading, candidate, hidden.min((available - width).max(0.0)));
        }
        count -= 1;
    }
    (0.0, String::new(), available.max(0.0))
}

fn replace_leader_placeholders_raw<F>(text: &str, mut replacement: F) -> String
where
    F: FnMut(&str) -> String,
{
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(LEADER_PLACEHOLDER_START) {
        out.push_str(&rest[..start]);
        let payload_start = start + LEADER_PLACEHOLDER_START.len();
        let Some(end_rel) = rest[payload_start..].find(LEADER_PLACEHOLDER_END) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let payload = &rest[payload_start..payload_start + end_rel];
        let replacement = replacement(payload);
        out.push_str(&replacement);
        rest = &rest[payload_start + end_rel + LEADER_PLACEHOLDER_END.len()..];
    }
    out.push_str(rest);
    out
}

/// Apply width-dependent generated content and visual bidi ordering before
/// both intrinsic measurement and line wrapping.  Returning owned runs keeps
/// the public layout API consuming: table sizing does not clone its run list.
struct PreparedTextRuns {
    runs: Vec<TextRun>,
    inferred_rtl: bool,
}

fn prepare_text_runs(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> PreparedTextRuns {
    let runs = expand_leader_placeholders(runs, options.max_width, fonts);
    let full_text: String = runs.iter().map(|run| run.text.as_str()).collect();
    let inferred_rtl = !options.paragraph_rtl
        && !options.bidi_plaintext
        && !options.bidi_override
        && crate::bidi::first_strong_is_rtl(&full_text);
    let bidi_rtl = options.paragraph_rtl || inferred_rtl;
    let runs = if !options.bidi_plaintext
        && (options.bidi_override || bidi_rtl || crate::bidi::has_rtl_chars(&full_text))
    {
        crate::bidi::reorder_runs_bidi(&runs, bidi_rtl, options.bidi_override)
    } else {
        runs
    };
    PreparedTextRuns { runs, inferred_rtl }
}

/// Produce the exact token stream consumed by the line wrapper.  Intrinsic
/// sizing uses this function too, so run boundaries, preserved whitespace,
/// hyphen/CJK opportunities, and atomic inline boxes cannot drift between the
/// table sizing and final wrapping passes.
fn tokenize_text_runs(runs: &[TextRun], options: TextWrapOptions) -> Vec<StyledWord> {
    let mut styled_words = Vec::new();
    let mut prev_run_ends_ws = true;
    for (run_index, run) in runs.iter().enumerate() {
        let run_starts_ws = run.text.chars().next().is_some_and(is_collapsible_space);
        // The first emitted word of this run is directly adjacent to the prior
        // content when neither side of the boundary had whitespace.
        let first_word_joins = !prev_run_ends_ws && !run_starts_ws;
        let run_ends_ws = run.text.chars().last().is_some_and(is_collapsible_space);

        if run.inline_box.is_some() {
            // Atomic inline box (`display: inline-block`): a single, unbreakable
            // token that takes part in inline spacing like a word.
            styled_words.push(StyledWord {
                text: String::new(),
                run_index,
                preserve_spacing: false,
                joins_prev: !prev_run_ends_ws,
                break_before: prev_run_ends_ws,
                has_internal_predecessor: false,
                ends_run: true,
            });
            prev_run_ends_ws = false;
            continue;
        }
        if run.text == "\n" {
            styled_words.push(StyledWord {
                text: "\n".to_string(),
                run_index,
                preserve_spacing: false,
                joins_prev: false,
                break_before: false,
                has_internal_predecessor: false,
                ends_run: true,
            });
            prev_run_ends_ws = true;
            continue;
        }
        // Index of the first non-newline word emitted from this run.  It is the
        // only token that can directly abut the preceding source run.
        let first_word_idx = styled_words.len();
        let has_newlines = run.text.contains('\n');
        let keep_trailing_space_before_rtl = run_ends_ws
            && !crate::bidi::has_rtl_chars(&run.text)
            && runs
                .get(run_index + 1)
                .is_some_and(|next| crate::bidi::has_rtl_chars(&next.text));
        // Under `white-space: normal`, spaces at a styled-run boundary still
        // collapse and remain ordinary break opportunities. Treating a leading
        // space as preserved makes the entire following run unbreakable, which
        // is especially visible after generated inline content such as a
        // footnote call. Only preserving white-space modes retain its source
        // whitespace.
        let preserves_source_spacing = run.metadata.whitespace.preserves_source_spacing()
            || (options.whitespace.preserve_source && (run_starts_ws || run.text.contains("  ")));
        if has_newlines {
            for (seg_idx, segment) in run.text.split('\n').enumerate() {
                if seg_idx > 0 {
                    styled_words.push(StyledWord {
                        text: "\n".to_string(),
                        run_index,
                        preserve_spacing: false,
                        joins_prev: false,
                        break_before: false,
                        has_internal_predecessor: false,
                        ends_run: false,
                    });
                }
                if segment.is_empty() {
                    continue;
                }
                let preserved = segment.chars().next().is_some_and(is_collapsible_space)
                    || segment.chars().last().is_some_and(is_collapsible_space)
                    || segment.contains("  ");
                if preserved {
                    if options.whitespace.wrap_preserved {
                        split_preserving_spaces(segment, run_index, &mut styled_words);
                    } else {
                        styled_words.push(StyledWord {
                            text: segment.to_string(),
                            run_index,
                            preserve_spacing: true,
                            joins_prev: false,
                            break_before: true,
                            has_internal_predecessor: false,
                            ends_run: false,
                        });
                    }
                } else {
                    for word in segment
                        .split(is_collapsible_space)
                        .filter(|word| !word.is_empty())
                    {
                        push_word_with_hyphen_breaks(
                            word,
                            run_index,
                            &mut styled_words,
                            options.word_break_keep_all,
                            options.hyphens_manual,
                        );
                    }
                }
            }
        } else if preserves_source_spacing {
            if options.whitespace.wrap_preserved {
                split_preserving_spaces(&run.text, run_index, &mut styled_words);
            } else {
                styled_words.push(StyledWord {
                    text: run.text.clone(),
                    run_index,
                    preserve_spacing: true,
                    joins_prev: false,
                    break_before: true,
                    has_internal_predecessor: false,
                    ends_run: false,
                });
            }
        } else {
            for word in run
                .text
                .split(is_collapsible_space)
                .filter(|word| !word.is_empty())
            {
                push_word_with_hyphen_breaks(
                    word,
                    run_index,
                    &mut styled_words,
                    options.word_break_keep_all,
                    options.hyphens_manual,
                );
            }
        }

        let mut has_internal_predecessor = false;
        for token in &mut styled_words[first_word_idx..] {
            if token.text == "\n" {
                has_internal_predecessor = false;
                continue;
            }
            token.has_internal_predecessor = has_internal_predecessor;
            has_internal_predecessor = true;
        }
        if first_word_joins
            && let Some(first) = styled_words.get_mut(first_word_idx)
            && first.text != "\n"
        {
            first.joins_prev = true;
            first.break_before = false;
        }
        if keep_trailing_space_before_rtl
            && let Some(last) = styled_words.last_mut()
            && last.text != "\n"
            && !last.text.ends_with(is_collapsible_space)
        {
            last.text.push(' ');
            prev_run_ends_ws = false;
        } else {
            prev_run_ends_ws = run_ends_ws;
        }
        if let Some(last) = styled_words
            .get_mut(first_word_idx..)
            .and_then(|tokens| tokens.last_mut())
        {
            last.ends_run = true;
        }
    }
    styled_words
}

fn measure_styled_token_end(
    token: &StyledWord,
    runs: &[TextRun],
    line_width: f32,
    preceding_runs: &[TextRun],
    previous_ends_whitespace: bool,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let run = &runs[token.run_index];
    if run.inline_box.is_some() {
        return measure_inline_token(
            run,
            line_width > 0.0,
            previous_ends_whitespace,
            token.joins_prev,
            fonts,
        )
        .end_width(line_width);
    }

    let is_preserved_space = token.preserve_spacing
        && !token.text.is_empty()
        && token.text.chars().all(|ch| ch == ' ' || ch == '\t');
    if options.whitespace.wrap_preserved && is_preserved_space {
        return line_width
            + token.leading_tracking(run, line_width > 0.0)
            + estimate_text_width_for_run(&token.text, run, fonts);
    }

    let paint_word = strip_soft_hyphens(&token.text);
    measure_normal_token(
        &paint_word,
        run,
        NormalTokenContext {
            preceding_runs,
            line_has_content: line_width > 0.0,
            previous_ends_whitespace,
            preserve_spacing: token.preserve_spacing,
            joins_previous: token.joins_prev,
            leading_tracking: token.leading_tracking(run, line_width > 0.0),
            outgoing_advance: token.outgoing_advance(run),
        },
        fonts,
    )
    .end_width(line_width)
}

fn next_up_nonnegative(value: f32) -> f32 {
    if value.is_nan() || value == f32::INFINITY {
        value
    } else if value == 0.0 {
        f32::from_bits(1)
    } else {
        debug_assert!(value > 0.0);
        f32::from_bits(value.to_bits() + 1)
    }
}

/// Compare layout widths with a scale-relative arithmetic roundoff bound.  The
/// bound is deliberately below half an ULP at the compared magnitude: equal
/// computations survive harmless evaluation noise, while the immediately
/// preceding representable width remains observably too small.
fn width_exceeds_limit(width: f32, limit: f32) -> bool {
    if width <= limit {
        return false;
    }
    if !width.is_finite() || !limit.is_finite() {
        return true;
    }
    let scale = width.abs().max(limit.abs()).max(f32::MIN_POSITIVE);
    let arithmetic_roundoff = scale * (f32::EPSILON * 0.25);
    width - limit > arithmetic_roundoff
}

/// Smallest representable outer width whose subtraction-based available width
/// can hold `content_width`.  This is an exact inverse of the wrapper's
/// `outer - exclusion` calculation, not a visual tolerance or geometry fudge.
pub(crate) fn required_outer_width(content_width: f32, exclusion: f32) -> f32 {
    if !content_width.is_finite() || !exclusion.is_finite() {
        return content_width;
    }
    let mut outer = (content_width + exclusion).max(0.0);
    while (outer - exclusion).max(0.0) < content_width {
        let next = next_up_nonnegative(outer);
        if next == outer {
            break;
        }
        outer = next;
    }
    outer
}

/// Measure min-content and max-content inline sizes using the exact styled
/// tokens and advance association used by [`wrap_text_runs`].  The run list is
/// consumed so callers performing a separate sizing pass do not need to clone
/// it. `allow_soft_wrap` is false for `white-space: nowrap`/`pre`.
pub(crate) fn measure_text_intrinsic_widths(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    allow_soft_wrap: bool,
    fonts: &HashMap<String, TtfFont>,
) -> TextIntrinsicWidths {
    let PreparedTextRuns { runs, .. } = prepare_text_runs(runs, options, fonts);
    let tokens = tokenize_text_runs(&runs, options);
    if tokens.is_empty() {
        return TextIntrinsicWidths::default();
    }

    let mut max_content = 0.0f32;
    let mut max_line_width = 0.0f32;
    let mut max_context = Vec::new();
    let mut max_previous_ends_whitespace = true;
    let mut forced_line_index = 0usize;

    let mut min_content = 0.0f32;
    let mut min_group_width = 0.0f32;
    let mut min_context = Vec::new();
    let mut min_previous_ends_whitespace = true;
    let mut min_group_index = 0usize;

    let line_exclusion = |line_index: usize| {
        if line_index == 0 {
            options.text_indent
        } else {
            options.drop_cap_exclusion_width(line_index)
        }
    };

    for token in &tokens {
        let run = &runs[token.run_index];
        if token.text == "\n" && run.inline_box.is_none() {
            max_content = max_content.max(required_outer_width(
                max_line_width,
                line_exclusion(forced_line_index),
            ));
            max_line_width = 0.0;
            max_context.clear();
            max_previous_ends_whitespace = true;
            forced_line_index += 1;
            min_group_width = 0.0;
            min_context.clear();
            min_previous_ends_whitespace = true;
            continue;
        }

        max_line_width = measure_styled_token_end(
            token,
            &runs,
            max_line_width,
            &max_context,
            max_previous_ends_whitespace,
            options,
            fonts,
        );
        push_token_shaping_context(&mut max_context, token, run);
        max_previous_ends_whitespace =
            run.inline_box.is_none() && token.text.chars().last().is_some_and(is_collapsible_space);

        if token.break_before || min_context.is_empty() {
            min_group_width = 0.0;
            min_context.clear();
            min_previous_ends_whitespace = true;
            min_group_index += 1;
        }

        let is_break_spaces_run = options.whitespace.break_spaces
            && token.preserve_spacing
            && !token.text.is_empty()
            && token.text.chars().all(|ch| ch == ' ' || ch == '\t');
        if is_break_spaces_run {
            // Every preserved space is its own break opportunity.
            min_group_width = estimate_text_width_for_run(" ", run, fonts);
        } else {
            min_group_width = measure_styled_token_end(
                token,
                &runs,
                min_group_width,
                &min_context,
                min_previous_ends_whitespace,
                options,
                fonts,
            );
        }
        push_token_shaping_context(&mut min_context, token, run);
        let min_exclusion = if min_group_index == 1 && forced_line_index == 0 {
            options.text_indent
        } else {
            0.0
        };
        min_content = min_content.max(required_outer_width(min_group_width, min_exclusion));
        min_previous_ends_whitespace =
            run.inline_box.is_none() && token.text.chars().last().is_some_and(is_collapsible_space);
    }

    max_content = max_content.max(required_outer_width(
        max_line_width,
        line_exclusion(forced_line_index),
    ));
    if !allow_soft_wrap {
        min_content = max_content;
    } else if options.overflow_wrap == OverflowWrap::Anywhere {
        // CSS Text 3 makes the emergency opportunities introduced by
        // `anywhere` part of min-content sizing. `break-word` deliberately
        // does not. Derive this from the same prepared runs used above so
        // shaping, letter spacing, fallback fonts, and atomic inline boxes do
        // not acquire a second intrinsic-measurement implementation.
        min_content = runs
            .iter()
            .map(|run| {
                run.inline_box.as_deref().map_or_else(
                    || {
                        run.text
                            .chars()
                            .filter(|character| *character != '\n')
                            .map(|character| {
                                estimate_text_width_for_run(&character.to_string(), run, fonts)
                            })
                            .fold(0.0_f32, f32::max)
                    },
                    InlineBox::outer_width,
                )
            })
            .fold(0.0_f32, f32::max);
        min_content = required_outer_width(min_content, options.text_indent);
    }
    TextIntrinsicWidths {
        min_content,
        max_content,
    }
}

fn push_token_shaping_context(context: &mut Vec<TextRun>, token: &StyledWord, template: &TextRun) {
    context.push(template.text_fragment(strip_soft_hyphens(&token.text), token.ends_run));
}

/// Simple text wrapping using character width estimation.
/// Uses TTF metrics when a custom font is available.
pub(crate) fn wrap_text_runs(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextLine> {
    let PreparedTextRuns {
        mut runs,
        inferred_rtl,
    } = prepare_text_runs(runs, options, fonts);
    crate::layout::text_emphasis::resolve_text_emphasis_metrics(&mut runs, fonts);
    let line_height_factor = options.line_height_factor.max(0.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width: f32 = 0.0;
    // The baseline-shift of a `vertical-align: super`/`sub` run is a fraction of
    // the PARENT font size (css2 §10.8.1), measured here once from the surrounding
    // ordinary text so the line box grows by the correct amount regardless of the
    // shifted run's own (shrunk) size.
    let shift_basis_fs = line_primary_font_size(&runs);
    // The line box takes its height from the inline content it contains: each
    // run uses the line-height resolved from its *own* element's style, falling
    // back to the block-level factor when the run leaves it unspecified (NaN).
    let authored_run_line_height = |run: &TextRun| {
        let factor = if run.line_height_factor.is_nan() {
            line_height_factor
        } else {
            run.line_height_factor.max(0.0)
        };
        run.line_height_font_size() * factor
    };
    let run_line_height = authored_run_line_height;
    // Start the line box height from the first run's line-height contribution.
    let mut line_height = runs
        .first()
        .map_or(options.default_font_size * line_height_factor, |r| {
            run_line_height(r)
        });

    let styled_words = tokenize_text_runs(&runs, options);

    if styled_words.is_empty() && !runs.is_empty() {
        let mut lines = vec![resolved_text_line(runs, line_height, options, fonts)];
        resolve_inline_fragment_decorations(&mut lines);
        return lines;
    }

    // Use a VecDeque so hyphenation remainders can be re-queued for processing.
    let mut queue: std::collections::VecDeque<StyledWord> = styled_words.into_iter().collect();

    // `white-space: break-spaces` line-break bookkeeping (css-text-3 §3). A soft
    // wrap opportunity exists only *after* a preserved space, so a word placed
    // after such spaces cannot itself end a line — if the next space would
    // overflow (spaces never hang under break-spaces), the line must roll back to
    // the last opportunity, sending the trailing word to the next line.
    // `bs_break_run_idx` is the count of runs in `current_runs` at the last such
    // opportunity (just after a placed space run).
    let mut bs_break_run_idx: usize = 0;

    // CSS `text-indent` only shortens the FIRST formatted line: the inline
    // content available before wrapping is `max_width - text_indent` while no
    // line has been emitted yet, and the full `max_width` afterwards.
    //
    // A dropped initial reduces the room on later overlapping lines. Its
    // originating line carries the initial glyph directly; subsequent lines use
    // the same kerned exclusion width used to place their inline origin.
    let drop_cap_offset = |emitted: usize| options.drop_cap_exclusion_width(emitted);
    let line_max_width = |emitted: usize| {
        let dc = drop_cap_offset(emitted);
        if emitted == 0 {
            (options.max_width - options.text_indent - dc).max(0.0)
        } else {
            (options.max_width - dc).max(0.0)
        }
    };

    while let Some(token) = queue.pop_front() {
        let template = &runs[token.run_index];
        let leading_tracking = token.leading_tracking(template, current_width > 0.0);
        let outgoing_advance = token.outgoing_advance(template);
        let StyledWord {
            text: word,
            run_index,
            preserve_spacing,
            joins_prev,
            break_before,
            has_internal_predecessor,
            ends_run,
        } = token;
        let template = &runs[run_index];
        if word == "\n" {
            // Line break
            push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
            current_width = 0.0;
            line_height = run_line_height(template);
            bs_break_run_idx = 0;
            continue;
        }

        // `white-space: break-spaces` preserved-space token (css-text-3 §3).
        // Differs from pre-wrap in two ways: every preserved space is itself a
        // soft wrap opportunity (so a long run may break mid-run), and preserved
        // spaces NEVER hang — they always occupy width at the line end. Because a
        // line may break only *after* a space, a word placed after a space run
        // cannot end a line on its own: if a later space would overflow, the line
        // rolls back to the opportunity after the last fitting space, moving the
        // trailing word to the next line. We record that opportunity here.
        if options.whitespace.break_spaces
            && preserve_spacing
            && template.inline_box.is_none()
            && !word.is_empty()
            && word.chars().all(|c| c == ' ' || c == '\t')
        {
            let single_sp = estimate_text_width_for_run(" ", template, fonts);
            // Place spaces one at a time so the run can break between adjacent
            // spaces (a wrap opportunity exists after each).
            let mut pending = String::new();
            let mut follows_internal_unit = has_internal_predecessor && current_width > 0.0;
            for c in word.chars() {
                let tracking = if follows_internal_unit {
                    template.metadata.spacing.letter
                } else {
                    Default::default()
                };
                if current_width > 0.0
                    && width_exceeds_limit(
                        current_width + tracking + single_sp,
                        line_max_width(lines.len()),
                    )
                {
                    // This space overflows. Spaces do not hang under break-spaces,
                    // so the line must break. The line may only end after a space,
                    // so any word placed since the last opportunity
                    // (`bs_break_run_idx`) cannot stay: roll it back onto the next
                    // line. Spaces already accumulated in `pending` belong to this
                    // overflowing run and break here too.
                    if !pending.is_empty() {
                        current_runs
                            .push(template.text_fragment(std::mem::take(&mut pending), false));
                    }
                    // Split off the trailing word (runs after the last opportunity)
                    // so it begins the next line. Clamp the index in case an
                    // earlier word-overflow flush shortened `current_runs`.
                    let split_at = bs_break_run_idx.min(current_runs.len());
                    let rolled: Vec<TextRun> = current_runs.split_off(split_at);
                    line_height = line_height.max(run_line_height(template));
                    push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
                    current_width = 0.0;
                    line_height = options.default_font_size * line_height_factor;
                    bs_break_run_idx = 0;
                    // Re-place the rolled-back word at the start of the new line.
                    for r in crate::text::coalesce_text_runs(&rolled) {
                        current_width += measure_text_run_advance(&r, fonts);
                        line_height = line_height.max(run_line_height(&r));
                        current_runs.push(r);
                    }
                    follows_internal_unit = has_internal_predecessor && current_width > 0.0;
                }
                pending.push(c);
                current_width += (if follows_internal_unit {
                    template.metadata.spacing.letter
                } else {
                    Default::default()
                }) + single_sp;
                follows_internal_unit = true;
            }
            if !pending.is_empty() {
                line_height = line_height.max(run_line_height(template));
                current_runs.push(template.text_fragment(pending, ends_run));
            }
            // A soft wrap opportunity now exists after these spaces: a following
            // word may be rolled back to here if it cannot be followed by its own
            // trailing space without overflow.
            bs_break_run_idx = current_runs.len();
            continue;
        }

        // `white-space: pre-wrap` preserved-space token. Under pre-wrap a run of
        // spaces is preserved verbatim but is also a soft-wrap opportunity
        // (css-text-3 §4.1.1): when the spaces fall at the end of a line they
        // "hang" — they sit on the current line and are NOT carried to the start
        // of the next line. So a space token that would overflow does not move
        // the box right; instead the line is flushed *after* the spaces and the
        // following word begins the next line. A space token at the very start of
        // a line (current_width == 0) is preserved as a genuine leading space
        // only when it did not arise from a soft wrap — which here means it was
        // the literal first token of the segment (handled by emitting it).
        if options.whitespace.wrap_preserved
            && preserve_spacing
            && template.inline_box.is_none()
            && !word.is_empty()
            && word.chars().all(|c| c == ' ' || c == '\t')
        {
            let sp_width = estimate_text_width_for_run(&word, template, fonts);
            let total_width = leading_tracking + sp_width;
            if current_width > 0.0
                && width_exceeds_limit(current_width + total_width, line_max_width(lines.len()))
            {
                // The spaces hang past the line edge: keep them on the current
                // line, then break. The next token starts a fresh line with no
                // carried-over leading space.
                line_height = line_height.max(run_line_height(template));
                current_runs.push(template.text_fragment(word, false));
                push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
                current_width = 0.0;
                line_height = options.default_font_size * line_height_factor;
                continue;
            }
            // Otherwise the spaces fit on the line: emit them verbatim.
            current_width += total_width;
            line_height = line_height.max(run_line_height(template));
            current_runs.push(template.text_fragment(word, ends_run));
            continue;
        }

        // Atomic inline box: advance by its margin-box width and grow the line
        // box to its height. It wraps to a fresh line if it overflows.
        if let Some(inline) = template.inline_box.as_deref() {
            // A collapsible space in the source before the box (`!joins_prev`)
            // renders as an inter-word space, exactly like a space before a word —
            // but only when the preceding run did not already emit a trailing space
            // (a `preserve_spacing` run, e.g. " text ", keeps its own trailing
            // space, so adding another here would double the gap).
            let prev_emitted_ws = current_runs
                .last()
                .and_then(|r: &TextRun| r.text.chars().last())
                .is_some_and(is_collapsible_space);
            let mut measurement = measure_inline_token(
                template,
                current_width > 0.0,
                prev_emitted_ws,
                joins_prev,
                fonts,
            );
            let end_width = if template.is_opening_inline_edge() {
                current_width
                    + measurement.leading_width
                    + opening_edge_group_width(template, queue.iter(), &runs, fonts)
            } else {
                measurement.end_width(current_width)
            };
            let has_preceding_content =
                current_runs.iter().any(|run| !run.is_opening_inline_edge());
            if has_preceding_content
                && (break_before || template.is_opening_inline_edge())
                && width_exceeds_limit(end_width, line_max_width(lines.len()))
            {
                push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
                current_width = 0.0;
                line_height = run_line_height(template);
                measurement = measure_inline_token(template, false, true, joins_prev, fonts);
            }
            if measurement.leading_width > 0.0 {
                // Emit the inter-word space as a run so the box advances past it.
                let mut space = template.text_fragment(" ".to_string(), false);
                space.inline_box = None;
                space.vertical_align = VerticalAlign::Baseline;
                current_runs.push(space);
            }
            current_width = measurement.end_width(current_width);
            if template.is_inline_edge() {
                current_runs.push(template.clone());
                continue;
            }
            // CSS2 §10.8: the line box must be tall enough to contain every
            // inline-level box after vertical alignment. For baseline/sub/super
            // boxes the height is the sum of the line's total extent ABOVE the
            // baseline and the total extent BELOW it, each the max over the box
            // and the surrounding text. A box's own baseline sits `baseline_ascent`
            // below its top edge (CSS2 §10.8.1); with no content baseline the
            // whole box rests above the line baseline (its bottom edge on it).
            // Sub/super then shift the box's baseline down/up relative to the line
            // baseline, moving its extents to the opposite side.
            let box_extent = match inline.vertical_align {
                crate::style::computed::VerticalAlign::Baseline
                | crate::style::computed::VerticalAlign::Sub
                | crate::style::computed::VerticalAlign::Super
                | crate::style::computed::VerticalAlign::Length(_)
                | crate::style::computed::VerticalAlign::Percent(_) => {
                    let box_ascent = inline.baseline_ascent.unwrap_or(inline.height);
                    let box_descent = (inline.height - box_ascent).max(0.0);
                    let shift = match inline.vertical_align {
                        crate::style::computed::VerticalAlign::Sub => {
                            -shift_basis_fs * crate::render::pdf::SUB_SHIFT_RATIO
                        }
                        crate::style::computed::VerticalAlign::Super => {
                            shift_basis_fs * crate::render::pdf::SUPER_SHIFT_RATIO
                        }
                        crate::style::computed::VerticalAlign::Length(v) => v,
                        crate::style::computed::VerticalAlign::Percent(p) => {
                            authored_run_line_height(template) * p
                        }
                        _ => 0.0,
                    };
                    let box_above = (box_ascent + shift).max(0.0);
                    let box_below = (box_descent - shift).max(0.0);
                    // The surrounding text's own extents above and below the line
                    // baseline (CSS2 §10.8.1): font ascent/descent plus SYMMETRIC
                    // half-leading — NOT a split proportional to ascent:descent.
                    // The two agree only at zero leading; for a larger line-height
                    // the proportional form misplaces the baseline, so a baseline-
                    // aligned box's overhang is measured against the wrong edge.
                    let (asc_ratio, desc_ratio) = crate::fonts::font_metrics_ratios(
                        template.css_font_family(),
                        template.bold,
                        template.font_style.is_slanted(),
                        fonts,
                    );
                    let lh = run_line_height(template);
                    let content = (asc_ratio + desc_ratio) * template.font_size;
                    let half_leading = ((lh - content) / 2.0).max(0.0);
                    let text_above = asc_ratio * template.font_size + half_leading;
                    let text_below = desc_ratio * template.font_size + half_leading;
                    box_above.max(text_above) + box_below.max(text_below)
                }
                crate::style::computed::VerticalAlign::Middle => {
                    // Middle centres the box on the parent's mid-x-height
                    // (baseline + x-height/2), so it reaches `h/2 + x-height/2`
                    // above the baseline and `h/2 - x-height/2` below — the line
                    // box must reserve that, combined with the surrounding text's
                    // own half-leading extents. (Mirrors render `line_box_metrics`.)
                    let (asc_ratio, desc_ratio) = crate::fonts::font_metrics_ratios(
                        template.css_font_family(),
                        template.bold,
                        template.font_style.is_slanted(),
                        fonts,
                    );
                    let lh = run_line_height(template);
                    let content = (asc_ratio + desc_ratio) * template.font_size;
                    let half_leading = ((lh - content) / 2.0).max(0.0);
                    let text_above = asc_ratio * template.font_size + half_leading;
                    let text_below = desc_ratio * template.font_size + half_leading;
                    let xh_ratio = if let crate::style::computed::FontFamily::Custom(name) =
                        template.css_font_family()
                    {
                        crate::system_fonts::find_font(
                            fonts,
                            name,
                            template.bold,
                            template.font_style.is_slanted(),
                        )
                        .map_or(0.5, |(_, f)| f.x_height_ratio())
                    } else {
                        0.5
                    };
                    let xh = xh_ratio * shift_basis_fs;
                    let box_above = inline.height / 2.0 + xh / 2.0;
                    let box_below = (inline.height / 2.0 - xh / 2.0).max(0.0);
                    box_above.max(text_above) + box_below.max(text_below)
                }
                _ => inline.height,
            };
            line_height = line_height.max(box_extent);
            current_runs.push(template.clone());
            continue;
        }

        let paint_word = strip_soft_hyphens(&word);
        let previous_run = current_runs.last();
        let previous_ends_whitespace = previous_run
            .and_then(|run| run.text.chars().last())
            .is_some_and(is_collapsible_space);
        let mut measurement = measure_normal_token(
            &paint_word,
            template,
            NormalTokenContext {
                preceding_runs: &current_runs,
                line_has_content: current_width > 0.0,
                previous_ends_whitespace,
                preserve_spacing,
                joins_previous: joins_prev,
                leading_tracking,
                outgoing_advance,
            },
            fonts,
        );
        let effective_max_width = line_max_width(lines.len());
        let overflows =
            width_exceeds_limit(measurement.end_width(current_width), effective_max_width);

        // `overflow-wrap: break-word`/`anywhere` (and the `word-break: break-all`
        // alias) break inside a word only as a LAST RESORT (css-text-3 §5.2): a
        // word is broken at an arbitrary point only when it cannot fit on a line
        // by itself. So if the word still has content beside it on the current
        // line but would fit alone on the next line, we must first wrap to that
        // next line (handled below) rather than break it mid-line. The emergency
        // split only applies when the word overflows a fresh, empty line.
        let fresh_line_width = if current_width > 0.0 {
            line_max_width(lines.len() + 1)
        } else {
            effective_max_width
        };
        let must_break_word = overflows
            && !preserve_spacing
            && options.overflow_wrap != OverflowWrap::Normal
            && measurement.word_width > fresh_line_width;

        if must_break_word {
            // When the line already has content, finish it first so the long
            // word starts breaking at the left edge of a fresh line — matching
            // Chrome, which keeps the preceding whole word(s) alone on their line.
            if current_width > 0.0 {
                push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
                current_width = 0.0;
                line_height = run_line_height(template);
                measurement = measure_normal_token(
                    &paint_word,
                    template,
                    NormalTokenContext {
                        preceding_runs: &[],
                        line_has_content: false,
                        previous_ends_whitespace: true,
                        preserve_spacing,
                        joins_previous: joins_prev,
                        leading_tracking: 0.0,
                        outgoing_advance,
                    },
                    fonts,
                );
            }
            let available_width = line_max_width(lines.len());
            if let Some((prefix, remainder)) = split_word_to_fit(
                &paint_word,
                available_width,
                template.font_size,
                &template.font_family,
                template.bold,
                template.font_style.is_slanted(),
                fonts,
            ) {
                // The current line was already flushed above, so the prefix
                // starts a fresh line with no leading inter-word space.
                line_height = line_height.max(run_line_height(template));
                current_runs.push(template.text_fragment(prefix, false));

                push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
                current_width = 0.0;
                line_height = run_line_height(template);
                queue.push_front(StyledWord {
                    text: remainder,
                    run_index,
                    preserve_spacing: false,
                    joins_prev: false,
                    break_before: true,
                    has_internal_predecessor: true,
                    ends_run,
                });
                continue;
            }
        }

        if overflows && current_width > 0.0 && break_before {
            push_wrapped_line(&mut lines, &mut current_runs, line_height, options, fonts);
            current_width = 0.0;
            line_height = run_line_height(template);
            bs_break_run_idx = 0;
            measurement = measure_normal_token(
                &paint_word,
                template,
                NormalTokenContext {
                    preceding_runs: &[],
                    line_has_content: false,
                    previous_ends_whitespace: true,
                    preserve_spacing,
                    joins_previous: joins_prev,
                    leading_tracking: 0.0,
                    outgoing_advance,
                },
                fonts,
            );
        }

        // When transitioning between runs with different backgrounds,
        // emit the inter-word space as a separate unstyled run so the
        // background doesn't bleed from a highlighted span into plain text.
        //
        let run_word = if word.contains('\u{00ad}') {
            word
        } else {
            paint_word
        };
        let text = match measurement.inter_word_space {
            InterWordSpace::PreviousRun => {
                // Emit space as separate unstyled run using the PREVIOUS
                // run's font so it matches the surrounding text metrics.
                let prev_run = current_runs.last().unwrap_or(template);
                current_runs.push(TextRun {
                    text: " ".to_string(),
                    decorations: Vec::new(),
                    link_url: None,
                    background_color: None,
                    padding: EdgeSizes::ZERO,
                    border_radii: CornerRadii::ZERO,
                    inline_box: None,
                    ..prev_run.clone()
                });
                run_word
            }
            InterWordSpace::CurrentRun => {
                format!(" {run_word}")
            }
            InterWordSpace::None => run_word,
        };
        current_width = measurement.end_width(current_width);
        line_height = line_height.max(run_line_height(template));

        current_runs.push(template.text_fragment(text, ends_run));
    }

    if !current_runs.is_empty() {
        finalize_soft_hyphen_line(&mut current_runs, false);
        lines.push(resolved_text_line(
            current_runs,
            line_height,
            options,
            fonts,
        ));
    }

    // Apply the initial letter's shared margin-box geometry to every impacted
    // line, including the negatively kerned originating line.
    if options.drop_cap.is_some() {
        for (i, line) in lines.iter_mut().enumerate() {
            line.x_offset = options.drop_cap_line_inset(i);
        }
    }

    if options.bidi_plaintext {
        for line in &mut lines {
            let line_text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
            if line_text.is_empty() {
                continue;
            }
            let line_rtl = crate::bidi::first_strong_is_rtl(&line_text);
            if line_rtl || crate::bidi::has_rtl_chars(&line_text) {
                line.runs = crate::bidi::reorder_runs_bidi(&line.runs, line_rtl, false);
            }
            if line_rtl && options.max_width.is_finite() {
                let line_width = crate::layout::helpers::measure_runs_width(&line.runs, fonts);
                line.x_offset += (options.max_width - line_width).max(0.0);
            }
        }
    } else if inferred_rtl {
        for line in &mut lines {
            let line_width = crate::layout::helpers::measure_runs_width(&line.runs, fonts);
            line.x_offset += (options.max_width - line_width).max(0.0);
        }
    }

    resolve_inline_fragment_decorations(&mut lines);

    lines
}

// ---------------------------------------------------------------------------
// apply_text_overflow_ellipsis
// ---------------------------------------------------------------------------

/// Apply text-overflow: ellipsis by truncating lines and appending "...".
pub(crate) fn apply_text_overflow_ellipsis(
    lines: &mut Vec<TextLine>,
    max_width: f32,
    fonts: &HashMap<String, TtfFont>,
    rtl: bool,
) {
    // With nowrap, there should be only one line. Truncate it if it overflows.
    if lines.is_empty() {
        return;
    }
    // Merge all runs into a single string, keeping the style of the first run.
    let line = &lines[0];
    let total_text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
    if line.runs.is_empty() {
        return;
    }
    let template = line.runs[0].clone();
    let ellipsis = "...";
    let ellipsis_width = estimate_word_width(
        ellipsis,
        template.font_size,
        &template.font_family,
        template.bold,
        template.font_style.is_slanted(),
        fonts,
    );

    // Check if the line actually overflows
    let line_width = estimate_word_width(
        &total_text,
        template.font_size,
        &template.font_family,
        template.bold,
        template.font_style.is_slanted(),
        fonts,
    );
    if line_width <= max_width {
        return;
    }

    // Truncate character by character until text + ellipsis fits.
    let mut truncated = String::new();
    let chars: Vec<char> = total_text.chars().collect();
    let iter: Box<dyn Iterator<Item = char>> = if rtl {
        Box::new(chars.iter().rev().copied())
    } else {
        Box::new(chars.iter().copied())
    };
    for ch in iter {
        truncated.push(ch);
        let w = estimate_word_width(
            &truncated,
            template.font_size,
            &template.font_family,
            template.bold,
            template.font_style.is_slanted(),
            fonts,
        );
        if w + ellipsis_width > max_width {
            truncated.pop();
            break;
        }
    }
    if rtl {
        truncated = truncated.chars().rev().collect();
        truncated.insert_str(0, ellipsis);
    } else {
        truncated.push_str(ellipsis);
    }

    lines[0] = TextLine {
        runs: vec![TextRun {
            text: truncated,
            ..template
        }],
        height: line.height,
        baseline_ascent: line.baseline_ascent,
        x_offset: line.x_offset,
        metadata: line.metadata,
    };

    // Remove any additional lines (shouldn't exist with nowrap, but just in case)
    lines.truncate(1);
}

// ---------------------------------------------------------------------------
// push_styled_run (font-variant / font-feature-settings)
// ---------------------------------------------------------------------------

/// Synthetic small-caps scale: lowercase letters render as uppercase glyphs at
/// this fraction of the font size, so their cap-height lands near the normal
/// x-height — matching how browsers synthesise small-caps for faces without a
/// real `smcp` feature (css-fonts-4 §6.5). Chromium uses a 70% synthesized
/// cap for these bundled faces, which also keeps each transformed glyph's
/// advance on the same scale as its outline.
const SMALL_CAPS_SCALE: f32 = 0.7;

/// Chromium rounds a synthesized small-cap face to the nearest CSS pixel before
/// shaping it. This is distinct from merely scaling a painted outline: the
/// rounded size supplies the glyph advances used by the line layout as well.
fn synthesized_small_caps_font_size(font_size: f32) -> f32 {
    ((font_size / crate::fonts::PT_PER_CSS_PX) * SMALL_CAPS_SCALE).round()
        * crate::fonts::PT_PER_CSS_PX
}

/// Push the text for one styled inline fragment, applying `font-variant: small-caps`
/// synthesis and the `font-feature-settings` ligature flag (css-fonts-3/4) before
/// the standard fallback-splitting in [`push_text_run_with_fallback`].
///
/// `template` carries the run's resolved style (font, color, decorations, …) and
/// the already text-transformed text. For small-caps, the text is split at
/// case boundaries: characters that are already uppercase keep the full size,
/// while lowercase characters are uppercased and emitted at [`SMALL_CAPS_SCALE`].
fn push_styled_run(
    template: TextRun,
    caps: crate::style::computed::FontVariantCaps,
    synthesize_small_caps: bool,
    requested_weight: FontWeight,
    runs: &mut Vec<TextRun>,
    fonts: &HashMap<String, TtfFont>,
) {
    use crate::style::computed::FontVariantCaps;

    let mut template = template;
    mark_synthetic_weight_run(&mut template, requested_weight, fonts);

    if caps != FontVariantCaps::SmallCaps || !synthesize_small_caps {
        push_text_run_with_fallback(template, runs, fonts);
        return;
    }

    // Split into runs of "already uppercase / non-letter" (full size) vs
    // "lowercase" (uppercased + scaled). Characters that don't change under
    // uppercasing (digits, punctuation, already-capital letters) stay full size.
    let base_size = template.font_size;
    let small_size = synthesized_small_caps_font_size(base_size);
    let mut current = String::new();
    let mut current_small = false;

    let flush = |text: &mut String, small: bool, runs: &mut Vec<TextRun>| {
        if text.is_empty() {
            return;
        }
        let mut run = template.clone();
        run.text = std::mem::take(text);
        run.font_size = if small { small_size } else { base_size };
        if small {
            run.font_synthesis.small_caps = true;
        } else if run.text.chars().all(char::is_whitespace) {
            run.metadata.whitespace = RunWhitespace::Preserve;
        }
        push_text_run_with_fallback(run, runs, fonts);
    };

    for ch in template.text.chars() {
        // A character is "small-capped" when uppercasing actually changes it
        // (i.e. it is a lowercase letter).
        let upper: String = ch.to_uppercase().collect();
        let is_small = upper != ch.to_string();
        if !current.is_empty() && is_small != current_small {
            flush(&mut current, current_small, runs);
        }
        current_small = is_small;
        if is_small {
            current.push_str(&upper);
        } else {
            current.push(ch);
        }
    }
    flush(&mut current, current_small, runs);
}

// ---------------------------------------------------------------------------
// push_text_run_with_fallback
// ---------------------------------------------------------------------------

/// Split a text run where grapheme clusters resolve to different font families.
///
/// Authored faces retain priority. Missing clusters use the registered fallback
/// chain for the inherited language, keeping layout measurement and PDF paint on
/// the same face.
pub(crate) fn push_text_run_with_fallback(
    run: TextRun,
    runs: &mut Vec<TextRun>,
    fonts: &HashMap<String, TtfFont>,
) {
    if run.text.is_empty() {
        runs.push(run);
        return;
    }

    let fallback_fonts = crate::font_pack::FontFallbacks::new(run.metadata.font_locale, fonts);
    if fallback_fonts.is_empty() {
        runs.push(run);
        return;
    }

    let authored_faces = crate::text::AuthoredFontFaces::resolve(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        fonts,
    );
    let family_for = |cluster: &str| -> FontFamily {
        if authored_faces.covers(cluster) {
            return run.font_family.clone();
        }
        fallback_fonts
            .resolve_cluster(cluster)
            .map(|key| FontFamily::Custom(key.to_string()))
            .unwrap_or_else(|| run.font_family.clone())
    };

    let mut current = String::new();
    let mut current_family = run.font_family.clone();

    for cluster in unicode_segmentation::UnicodeSegmentation::graphemes(run.text.as_str(), true) {
        let family = family_for(cluster);
        if family != current_family && !current.is_empty() {
            runs.push(
                TextRun {
                    text: std::mem::take(&mut current),
                    ..run.clone()
                }
                .with_glyph_fallback(current_family),
            );
        }
        current_family = family;
        current.push_str(cluster);
    }

    if !current.is_empty() {
        runs.push(
            TextRun {
                text: current,
                ..run
            }
            .with_glyph_fallback(current_family),
        );
    }
}

fn min_content_anywhere_width(runs: &[TextRun], fonts: &HashMap<String, TtfFont>) -> f32 {
    runs.iter()
        .filter(|r| r.inline_box.is_none())
        .flat_map(|r| {
            r.text.chars().map(|ch| {
                estimate_word_width(
                    &ch.to_string(),
                    r.font_size,
                    &r.font_family,
                    r.bold,
                    r.font_style.is_slanted(),
                    fonts,
                )
            })
        })
        .fold(0.0f32, f32::max)
}

// ---------------------------------------------------------------------------
// collect_text_runs / collect_text_runs_inner
// ---------------------------------------------------------------------------

/// Build the atomic inline box for a `display: inline-block` element that
/// appears amongst inline text. Resolves the border-box geometry the same way
/// `layout_inline_block_group` does and pre-wraps the inner text.
fn build_inline_box(
    style: &ComputedStyle,
    el: &ElementNode,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
    counter_state: &mut CounterState,
    resources: &mut crate::security::resources::ResourceLoader,
) -> Option<InlineBox> {
    let has_explicit_width = style.width.is_some();
    let child_w = style.width.unwrap_or(0.0);
    let child_h = style.height.unwrap_or(0.0);

    // Content width used to wrap the inner text.
    let inner_width = if has_explicit_width {
        if style.box_sizing == BoxSizing::BorderBox {
            child_w - style.padding.horizontal() - style.border.horizontal_width()
        } else {
            child_w
        }
        .max(0.0)
    } else {
        // Shrink-to-fit with no constraint: use a generous width so the
        // inner text measures at its natural width on one line.
        f32::MAX
    };

    let mut runs = Vec::new();
    InlineRunCollector::new(rules, fonts, counter_state, resources).collect(
        InlineContentSequence::new(&el.children),
        style,
        &mut runs,
        None,
        ancestors,
    );
    let wrap_inner_width = if !has_explicit_width
        && style.width_keyword == Some(IntrinsicWidthKeyword::MinContent)
        && style.overflow_wrap == OverflowWrap::Anywhere
    {
        min_content_anywhere_width(&runs, fonts)
    } else {
        inner_width
    };
    let lines = if runs.is_empty() {
        Vec::new()
    } else {
        wrap_text_runs(
            runs,
            TextWrapOptions::new(
                wrap_inner_width,
                used_font_size(style, fonts),
                text_run_line_height_factor(style, fonts),
                style.overflow_wrap,
            )
            .with_white_space(style.white_space)
            .with_parent_strut(parent_line_strut(style, fonts))
            .with_rtl(style.direction_rtl)
            .with_bidi_override(style.bidi_override)
            .with_bidi_plaintext(style.bidi_plaintext)
            .with_word_break_keep_all(style.word_break_keep_all)
            .with_hyphens_manual(style.hyphens_manual),
            fonts,
        )
    };

    let content_w = if has_explicit_width {
        if style.box_sizing == BoxSizing::BorderBox {
            (child_w - style.padding.horizontal() - style.border.horizontal_width()).max(0.0)
        } else {
            child_w
        }
    } else if style.width_keyword == Some(IntrinsicWidthKeyword::MinContent)
        && style.overflow_wrap == OverflowWrap::Anywhere
    {
        lines
            .iter()
            .map(|l| crate::layout::helpers::measure_runs_width(&l.runs, fonts))
            .fold(0.0f32, f32::max)
    } else {
        // Shrink-to-fit width must use the REAL bundled-font advances per line
        // (the former str_width is Helvetica AFM and mis-sizes a ParitySans run,
        // giving the auto-width box the wrong width). measure_runs_width measures
        // each run with its actual font (and inline-box outer widths).
        lines
            .iter()
            .map(|l| crate::layout::helpers::measure_runs_width(&l.runs, fonts))
            .fold(0.0f32, f32::max)
    };
    let total_w = content_w + style.padding.horizontal() + style.border.horizontal_width();

    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let content_h = if child_h > 0.0 {
        if style.box_sizing == BoxSizing::BorderBox {
            (child_h - style.padding.vertical() - style.border.vertical_width()).max(0.0)
        } else {
            child_h
        }
    } else {
        text_height
    };
    let total_h = content_h + style.padding.vertical() + style.border.vertical_width();

    // CSS2 §10.8.1: the baseline of an `inline-block` is the baseline of its
    // LAST in-flow line box, expressed here as the distance from the box's top
    // border edge down to that baseline. When the box establishes its own
    // formatting context with non-visible overflow, or has no in-flow line box,
    // the baseline is the bottom margin edge (handled at paint time when this is
    // `None`).
    let baseline_ascent = if style.overflow.clips() || lines.is_empty() {
        None
    } else {
        let prior_lines_h: f32 = lines[..lines.len() - 1].iter().map(|l| l.height).sum();
        let last = &lines[lines.len() - 1];
        last.baseline_ascent.map(|baseline| {
            style.border.top.used_width() + style.padding.top + prior_lines_h + baseline
        })
    };

    Some(InlineBox {
        width: total_w,
        height: total_h,
        margin_left: style.margin.left.max(0.0),
        margin_right: style.margin.right.max(0.0),
        paint: InlineBoxPaint {
            background_color: style.background_color,
            border: LayoutBorder::from_computed(&style.border, style.color),
            border_image: style.border_image.paint(),
            border_radii: style.resolve_corner_radii(total_w, total_h),
            ..InlineBoxPaint::default()
        },
        padding: style.padding,
        vertical_align: style.vertical_align,
        baseline_ascent,
        lines,
        image: None,
        // CSS `position: relative` shifts the painted box without changing its
        // in-flow inline slot. `left`/`top` win over `right`/`bottom`.
        rel_offset_x: if style.position.is_relative() {
            style.left.or(style.right.map(|r| -r)).unwrap_or(0.0)
        } else {
            0.0
        },
        rel_offset_y: if style.position.is_relative() {
            style.top.or(style.bottom.map(|b| -b)).unwrap_or(0.0)
        } else {
            0.0
        },
    })
}

/// Shared state for flattening one inline source sequence into styled runs.
///
/// Grid, flex, block, table, and inline-block layout all use this collector.
/// Keeping selector rules, fonts, and counter state together makes it
/// impossible for a formatting-context-specific text path to omit generated
/// content or counter scope propagation.
pub(crate) struct InlineRunCollector<'a> {
    rules: &'a [CssRule],
    fonts: &'a HashMap<String, TtfFont>,
    counter_state: &'a mut CounterState,
    resources: &'a mut crate::security::resources::ResourceLoader,
    next_inline_decoration: usize,
    context: InlineRunContext,
}

/// Formatting-context policy for atomic boxes collected as text runs.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum InlineRunContext {
    #[default]
    Standard,
    TableCell,
}

impl InlineRunContext {
    fn accepts(self, role: InlineFormattingRole, element: &ElementNode) -> bool {
        match self {
            Self::Standard => role.uses_text_run_layout(element),
            Self::TableCell => role.participates_in_table_cell_text_flow(),
        }
    }
}

impl<'a> InlineRunCollector<'a> {
    pub(crate) fn new(
        rules: &'a [CssRule],
        fonts: &'a HashMap<String, TtfFont>,
        counter_state: &'a mut CounterState,
        resources: &'a mut crate::security::resources::ResourceLoader,
    ) -> Self {
        Self {
            rules,
            fonts,
            counter_state,
            resources,
            next_inline_decoration: 0,
            context: InlineRunContext::Standard,
        }
    }

    pub(crate) fn in_context(mut self, context: InlineRunContext) -> Self {
        self.context = context;
        self
    }

    pub(crate) fn collect(
        &mut self,
        sequence: InlineContentSequence<'_>,
        parent_style: &ComputedStyle,
        runs: &mut Vec<TextRun>,
        link_url: Option<&str>,
        ancestors: &[AncestorInfo],
    ) {
        let first_new_run = runs.len();
        collect_text_runs_inner(
            sequence,
            parent_style,
            runs,
            link_url,
            self.rules,
            self.fonts,
            self.context,
            false,
            ancestors,
            self.counter_state,
            self.resources,
            &mut self.next_inline_decoration,
        );
        let boundary_start = first_new_run.saturating_sub(1);
        runs[boundary_start..].resolve_unclaimed_boundaries(
            crate::layout::elements::TextSpacing::from_style(parent_style),
        );
    }

    pub(crate) fn collect_box_content(
        &mut self,
        nodes: &[DomNode],
        parent_style: &ComputedStyle,
        runs: &mut Vec<TextRun>,
        link_url: Option<&str>,
        ancestors: &[AncestorInfo],
    ) {
        self.collect(
            InlineContentSequence::new(nodes),
            parent_style,
            runs,
            link_url,
            ancestors,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_text_runs_inner(
    sequence: InlineContentSequence<'_>,
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    context: InlineRunContext,
    inline_parent: bool,
    ancestors: &[AncestorInfo],
    counter_state: &mut CounterState,
    resources: &mut crate::security::resources::ResourceLoader,
    next_inline_decoration: &mut usize,
) {
    let first_run = runs.len();
    sequence.append_before(runs, fonts, counter_state, resources);
    let mut siblings = InlineSiblingCursor::starting_at(
        sequence.source_nodes(),
        sequence.starting_element_index(),
    );
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
    );

    // `pre-line` collapses spaces/tabs like `normal` but keeps forced segment
    // breaks (newlines) as explicit line breaks (css-text-3 §4.1.1).
    let pre_line = parent_style.white_space == WhiteSpace::PreLine;

    for node in sequence.nodes() {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    // In pre/pre-wrap: preserve newlines as \n runs for line breaking,
                    // and expand tabs to the next tab stop (css-text-3 §6.3).
                    expand_pre_tabs(text, parent_style, fonts)
                } else if pre_line {
                    collapse_whitespace_pre_line(text)
                } else {
                    // CSS whitespace collapsing applies across the *whole* inline
                    // formatting context, not per text node. `collapse_whitespace`
                    // trims each node's edges, which would silently drop the space
                    // in `<span>a</span> <span>b</span>` (a lone-space text node
                    // between two inline elements). Re-attach a single collapsible
                    // edge space when the node carried one AND it sits between
                    // inline content (i.e. not at the block's leading edge), so the
                    // line breaker — which is whitespace-aware — keeps the inter-
                    // element space without synthesising spurious spaces elsewhere
                    // (e.g. between a `::before` run and the element's own text).
                    let mut collapsed = collapse_whitespace(text);
                    let starts_ws = text.chars().next().is_some_and(is_collapsible_space);
                    let ends_ws = text.chars().last().is_some_and(is_collapsible_space);
                    // An atomic inline box (`display: inline-block`) carries empty
                    // text but is still inline content: a collapsible space after it
                    // must be preserved (CSS2 §9.1 / css-text-3 §4.1), e.g.
                    // `Ag <span class=ib></span> text` keeps the space before
                    // "text". So treat a preceding inline box as trailing content,
                    // and (since a box never ends in whitespace) re-attach the space.
                    let prev_is_inline_box = runs
                        .last()
                        .is_some_and(|r: &TextRun| r.inline_box.is_some());
                    let prev_has_trailing_content = prev_is_inline_box
                        || runs
                            .last()
                            .is_some_and(|r: &TextRun| !r.text.is_empty() && r.text != "\n");
                    if starts_ws && prev_has_trailing_content {
                        let prev_ends_ws = runs
                            .last()
                            .and_then(|r: &TextRun| r.text.chars().last())
                            .is_some_and(is_collapsible_space);
                        if !prev_ends_ws {
                            collapsed.insert(0, ' ');
                        }
                    }
                    if ends_ws && !collapsed.is_empty() && !collapsed.ends_with(' ') {
                        collapsed.push(' ');
                    }
                    collapsed
                };
                // Apply CSS text-transform
                let processed = match parent_style.text_transform {
                    crate::style::computed::TextTransform::Uppercase => processed.to_uppercase(),
                    crate::style::computed::TextTransform::Lowercase => processed.to_lowercase(),
                    crate::style::computed::TextTransform::Capitalize => {
                        let mut result = String::with_capacity(processed.len());
                        let mut prev_is_space = true;
                        for c in processed.chars() {
                            if prev_is_space && c.is_alphabetic() {
                                for uc in c.to_uppercase() {
                                    result.push(uc);
                                }
                            } else {
                                result.push(c);
                            }
                            prev_is_space = c.is_whitespace();
                        }
                        result
                    }
                    crate::style::computed::TextTransform::None => processed,
                };
                if !processed.is_empty() {
                    // Only propagate background_color when the immediate
                    // parent is an inline element (e.g. <span>).  Block-level
                    // backgrounds are drawn by the TextBlock itself.
                    // In preformatted blocks (<pre>), skip inline backgrounds
                    // to avoid overlapping rects that hide subsequent lines.
                    let (bg, padding) =
                        if inline_parent && parent_style.white_space != WhiteSpace::Pre {
                            let bg = parent_style.background_color;
                            (bg, decoration_padding(parent_style, bg))
                        } else {
                            (None, EdgeSizes::ZERO)
                        };
                    push_styled_run(
                        styled_text_run(processed, parent_style, link_url, bg, padding, fonts),
                        parent_style.font_variant_caps,
                        parent_style.font_synthesis_small_caps,
                        parent_style.font_weight,
                        runs,
                        fonts,
                    );
                }
            }
            DomNode::Element(el) => {
                let selector_ctx = siblings.next_context(el, ancestors);
                if el.tag == HtmlTag::Br {
                    runs.push(TextRun {
                        text: "\n".to_string(),
                        font_size: used_font_size(parent_style, fonts),
                        font_family: resolve_style_font_family(parent_style, fonts),
                        line_height_factor: text_run_line_height_factor(parent_style, fonts),
                        ..Default::default()
                    });
                    continue;
                }

                let classes = el.class_list();
                let style = compute_style_with_context_with_font_metrics(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &selector_ctx,
                    FontMetrics::new(fonts),
                );
                let role = InlineFormattingRole::of(el, &style);
                if context.accepts(role, el) {
                    if el.attributes.contains_key("data-math") {
                        // Math elements are rendered as MathBlock by
                        // flatten_element, not as inline text runs.
                    } else {
                        let counter_scope = counter_state.enter_element(&style);
                        if style.float == Float::Footnote {
                            let mut footnote_text = String::new();
                            collect_inline_plain_text(&el.children, &mut footnote_text);
                            let footnote_text = collapse_whitespace(&footnote_text);
                            if !footnote_text.is_empty() {
                                let marker = (counter_state.get("footnote")
                                    + runs
                                        .iter()
                                        .filter(|run| {
                                            run.link_url
                                                .as_deref()
                                                .and_then(decode_footnote_link)
                                                .is_some()
                                        })
                                        .count() as i32
                                    + 1)
                                .to_string();
                                let call_pseudo = compute_pseudo_element_style_with_font_metrics(
                                    &style,
                                    rules,
                                    el.tag_name(),
                                    &classes,
                                    el.id(),
                                    &el.attributes,
                                    &selector_ctx,
                                    PseudoElement::FootnoteCall,
                                    FontMetrics::new(fonts),
                                );
                                let marker_pseudo = compute_pseudo_element_style_with_font_metrics(
                                    &style,
                                    rules,
                                    el.tag_name(),
                                    &classes,
                                    el.id(),
                                    &el.attributes,
                                    &selector_ctx,
                                    PseudoElement::FootnoteMarker,
                                    FontMetrics::new(fonts),
                                );
                                let authored_call_text =
                                    footnote_pseudo_content(call_pseudo.as_ref(), &marker);
                                let marker_prefix =
                                    footnote_pseudo_content(marker_pseudo.as_ref(), &marker)
                                        .unwrap_or_else(|| "{marker}. ".to_string());
                                let marker_color = marker_pseudo
                                    .as_ref()
                                    .map_or(style.color, |pseudo| pseudo.color);
                                let call_style = call_pseudo.as_ref().unwrap_or(&style);
                                let call_variant_position = call_pseudo
                                    .as_ref()
                                    .map_or(FontVariantPosition::Super, |pseudo| {
                                        pseudo.font_variant_position
                                    });
                                let call_font_size = used_font_size(call_style, fonts);
                                let call_text =
                                    authored_call_text.unwrap_or_else(|| marker.clone());
                                push_styled_run(
                                    TextRun {
                                        text: call_text,
                                        font_size: call_font_size
                                            * call_variant_position.glyph_scale(),
                                        bold: style_run_bold(call_style, fonts),
                                        font_style: style_run_font_style(call_style, fonts),
                                        color: call_style.color,
                                        link_url: Some(encode_footnote_link_data(
                                            &FootnoteLinkData {
                                                marker: marker.clone(),
                                                text: footnote_text,
                                                marker_prefix,
                                                body: FootnoteBodyStyle {
                                                    font_size: used_font_size(&style, fonts),
                                                    bold: style_run_bold(&style, fonts),
                                                    italic: style_run_font_style(&style, fonts)
                                                        .is_slanted(),
                                                    color: style.color,
                                                    font_family: resolve_style_font_family(
                                                        &style, fonts,
                                                    ),
                                                    line_height_factor: text_run_line_height_factor(
                                                        &style, fonts,
                                                    ),
                                                },
                                                marker_color,
                                                formatting: style.footnote,
                                            },
                                        )),
                                        font_family: resolve_style_font_family(call_style, fonts),
                                        line_height_factor: text_run_line_height_factor(
                                            call_style, fonts,
                                        ),
                                        line_height_basis: call_font_size,
                                        font_variant_position: call_variant_position,
                                        vertical_align: call_pseudo
                                            .as_ref()
                                            .map_or(VerticalAlign::Baseline, |pseudo| {
                                                pseudo.vertical_align
                                            }),
                                        text_shadow: call_style.text_shadow.clone(),
                                        shaping: text_run_shaping(call_style),
                                        metadata: text_run_metadata(call_style),
                                        ..Default::default()
                                    },
                                    call_style.font_variant_caps,
                                    call_style.font_synthesis_small_caps,
                                    call_style.font_weight,
                                    runs,
                                    fonts,
                                );
                            }
                            counter_state.leave_element(counter_scope);
                            continue;
                        }
                        let url = if el.tag == HtmlTag::A {
                            el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                        } else {
                            link_url
                        };
                        let mut child_ancestors = ancestors.to_vec();
                        child_ancestors.push(selector_ctx.as_ancestor(el));
                        // `display: inline-block` is an atomic inline box: it
                        // takes part in line layout with its own box geometry
                        // rather than flowing its content as bare text. SVGs are
                        // excluded (they need their own block layout via cm).
                        let is_atomic_inline_block = style.display == Display::InlineBlock
                            && el.tag != HtmlTag::Svg
                            && !el
                                .children
                                .iter()
                                .any(|c| matches!(c, DomNode::Element(e) if e.tag == HtmlTag::Svg));
                        if is_atomic_inline_block {
                            let line_height_factor =
                                text_run_line_height_factor(parent_style, fonts);
                            if let Some(boxed) = build_inline_box(
                                &style,
                                el,
                                rules,
                                fonts,
                                &child_ancestors,
                                counter_state,
                                resources,
                            ) {
                                runs.push(TextRun {
                                    font_size: used_font_size(parent_style, fonts),
                                    color: parent_style.color,
                                    link_url: url.map(String::from),
                                    font_family: resolve_style_font_family(parent_style, fonts),
                                    line_height_factor,
                                    inline_box: Some(Box::new(boxed)),
                                    ..Default::default()
                                });
                            }
                            counter_state.leave_element(counter_scope);
                            continue;
                        }
                        // Generated children and authored children are one source-
                        // ordered inline sequence. Routing all three through the
                        // shared collector prevents an element-specific pseudo path
                        // from diverging in counters, spacing, or selector context.
                        let generated = GeneratedContentStyles::resolve(
                            el,
                            &style,
                            rules,
                            &selector_ctx,
                            fonts,
                        );
                        let element_start = runs.len();
                        let decoration_id = crate::layout::engine::InlineDecorationId::from_index(
                            *next_inline_decoration,
                        );
                        *next_inline_decoration += 1;
                        collect_text_runs_inner(
                            InlineContentSequence::with_generated(
                                &el.children,
                                generated.boxes(el),
                            ),
                            &style,
                            runs,
                            url,
                            rules,
                            fonts,
                            context,
                            true,
                            &child_ancestors,
                            counter_state,
                            resources,
                            next_inline_decoration,
                        );
                        apply_inline_horizontal_edges(
                            runs,
                            element_start,
                            &style,
                            parent_style.direction_rtl,
                            decoration_id,
                        );
                        apply_inline_parent_background(runs, element_start, &style, decoration_id);
                        resolve_target_attrs(&mut runs[element_start..], el);
                        runs[element_start..].resolve_unclaimed_boundaries(
                            crate::layout::elements::TextSpacing::from_style(&style),
                        );
                        counter_state.leave_element(counter_scope);
                    }
                }
            }
        }
    }
    sequence.append_after(runs, fonts, counter_state, resources);
    runs[first_run..].resolve_unclaimed_boundaries(
        crate::layout::elements::TextSpacing::from_style(parent_style),
    );
}

#[cfg(test)]
mod indent_tests {
    use super::*;

    #[test]
    fn line_strut_splits_leading_on_the_css_pixel_grid() {
        let strut = LineStrut::from_font(
            &FontFamily::Helvetica,
            12.0,
            false,
            false,
            18.0,
            &HashMap::new(),
        );

        assert_eq!(strut.above, 12.0);
        assert_eq!(strut.below, 6.0);
        assert!((strut.above + strut.below - 18.0).abs() < f32::EPSILON);
    }

    #[test]
    fn document_flow_strut_uses_css_pixel_font_metrics() {
        let strut = LineStrut::from_font(
            &FontFamily::Custom("ParitySans".to_string()),
            12.0,
            false,
            false,
            18.0,
            &parity_sans_fonts(),
        );

        assert_eq!(strut.above, 12.75);
        assert_eq!(strut.below, 5.25);
    }

    fn inline_box_from_markup(markup: &str) -> InlineBox {
        let nodes = crate::parser::html::parse_html(markup).expect("valid inline fixture");
        let DomNode::Element(element) = &nodes[0] else {
            panic!("fixture root must be an element");
        };
        let parent = ComputedStyle::default();
        let style =
            crate::style::computed::compute_style(element.tag, element.style_attr(), &parent);
        let mut counter_state = CounterState::default();
        let mut resources = crate::security::resources::ResourceLoader::default();
        build_inline_box(
            &style,
            element,
            &[],
            &HashMap::new(),
            &[],
            &mut counter_state,
            &mut resources,
        )
        .expect("atomic inline box")
    }

    fn inline_runs(markup: &str) -> Vec<TextRun> {
        let nodes = crate::parser::html::parse_html(markup).expect("valid inline fixture");
        let DomNode::Element(element) = &nodes[0] else {
            panic!("fixture root must be an element");
        };
        let parent = ComputedStyle::default();
        let style =
            crate::style::computed::compute_style(element.tag, element.style_attr(), &parent);
        let fonts = HashMap::new();
        let mut counter_state = CounterState::default();
        let mut resources = crate::security::resources::ResourceLoader::default();
        let mut runs = Vec::new();
        InlineRunCollector::new(&[], &fonts, &mut counter_state, &mut resources)
            .collect_box_content(&element.children, &style, &mut runs, None, &[]);
        runs
    }

    fn inline_run_width(markup: &str) -> f32 {
        crate::layout::helpers::measure_runs_width(&inline_runs(markup), &HashMap::new())
    }

    #[test]
    fn inline_horizontal_padding_and_margin_add_layout_advance() {
        let plain = inline_run_width("<div>A<span>B</span>C</div>");
        let padded = inline_run_width("<div>A<span style='padding-right:40px'>B</span>C</div>");
        let margined = inline_run_width("<div>A<span style='margin-right:40px'>B</span>C</div>");

        assert!((padded - plain - 30.0).abs() < 0.001, "{padded} vs {plain}");
        assert!(
            (margined - plain - 30.0).abs() < 0.001,
            "{margined} vs {plain}"
        );
    }

    #[test]
    fn inline_edges_compose_across_both_sides_and_nested_spans() {
        let plain = inline_run_width("<div>A<span>B</span>C</div>");
        let both_sides = inline_run_width(
            "<div>A<span style='margin-left:8px;padding-left:12px;\
             padding-right:16px;margin-right:4px'>B</span>C</div>",
        );
        let nested = inline_run_width(
            "<div>A<span style='padding:0 10px'><span style='margin:0 6px'>\
             B</span></span>C</div>",
        );

        assert!((both_sides - plain - 30.0).abs() < 0.001);
        assert!((nested - plain - 24.0).abs() < 0.001);
    }

    #[test]
    fn negative_inline_margin_offsets_padding_advance() {
        let plain = inline_run_width("<div>AB<span>C</span>D</div>");
        let offset = inline_run_width(
            "<div>AB<span style='padding-right:40px;margin-right:-10px'>C</span>D</div>",
        );
        let negative = inline_run_width("<div>AB<span style='margin-right:-8px'>C</span>D</div>");

        assert!((offset - plain - 22.5).abs() < 0.001);
        assert!((negative - plain + 6.0).abs() < 0.001);
    }

    #[test]
    fn rtl_inline_edges_keep_their_physical_left_and_right_widths() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div style='direction:rtl'><span style='padding-left:12px;padding-right:40px'>\
             שלום</span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(200.0, 12.0, 1.2, OverflowWrap::Normal).with_rtl(true),
            &fonts,
        );
        let runs = &lines[0].runs;

        assert!(runs[0].is_opening_inline_edge());
        assert_eq!(runs[0].atomic_inline_advance(), Some(9.0));
        assert_eq!(line_text(&lines[0]), "שלום");
        assert_eq!(
            runs.last().and_then(TextRun::atomic_inline_advance),
            Some(30.0)
        );
    }

    #[test]
    fn inline_fragment_edges_follow_the_parent_inline_progression() {
        let child = ComputedStyle {
            direction_rtl: true,
            padding: EdgeSizes {
                left: 9.0,
                right: 30.0,
                ..EdgeSizes::ZERO
            },
            ..Default::default()
        };
        let decoration = crate::layout::engine::InlineDecorationId::from_index(0);

        let ltr_parent = InlineHorizontalEdges::from_style(&child, false, decoration);
        let rtl_parent = InlineHorizontalEdges::from_style(&child, true, decoration);

        assert_eq!((ltr_parent.start, ltr_parent.end), (9.0, 30.0));
        assert_eq!((rtl_parent.start, rtl_parent.end), (30.0, 9.0));
    }

    #[test]
    fn physical_horizontal_edges_do_not_advance_vertical_inline_content() {
        let runs = inline_runs(
            "<div style='writing-mode:vertical-rl'><span \
             style='padding-left:12px;margin-right:40px;background:#ddd;\
             border-radius:8px'>縦</span></div>",
        );

        assert!(!runs.iter().any(TextRun::is_inline_edge));
        assert!(
            runs.iter()
                .filter(|run| !run.text.is_empty())
                .all(|run| run.metadata.inline_decoration.is_none())
        );
    }

    #[test]
    fn inline_edges_stay_with_their_content_when_wrapping() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div><span style='padding-left:20px;padding-right:20px'>\
             alpha beta</span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(50.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );

        assert_eq!(lines.len(), 2);
        assert!(
            lines[0]
                .runs
                .first()
                .is_some_and(|run| run.inline_box.is_some())
        );
        assert_eq!(line_text(&lines[0]), "alpha");
        assert_eq!(line_text(&lines[1]), "beta");
        assert!(
            lines[1]
                .runs
                .last()
                .is_some_and(|run| run.inline_box.is_some())
        );
    }

    #[test]
    fn wrapped_inline_decorations_keep_each_horizontal_edge_once() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div><span style='padding:0 12px;background:#ddd'>\
             alpha <b>beta</b> gamma</span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(90.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );
        let horizontal_padding = |line: &TextLine| {
            line.runs.iter().fold((0.0, 0.0), |(left, right), run| {
                (left + run.padding.left, right + run.padding.right)
            })
        };

        assert_eq!(lines.len(), 2);
        assert_eq!(horizontal_padding(&lines[0]), (9.0, 0.0));
        assert_eq!(horizontal_padding(&lines[1]), (0.0, 9.0));
    }

    #[test]
    fn rounded_inline_without_padding_retains_only_its_fragment_corners() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div><span style='background:#ddd;border-radius:8px'>\
             alpha beta gamma</span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(70.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );

        assert_eq!(lines.len(), 2);
        assert!(lines[0].runs.first().is_some_and(TextRun::is_inline_edge));
        assert!(lines[1].runs.last().is_some_and(TextRun::is_inline_edge));
        let first_text = lines[0]
            .runs
            .iter()
            .find(|run| !run.text.is_empty())
            .expect("first inline fragment text");
        let last_text = lines[1]
            .runs
            .iter()
            .rev()
            .find(|run| !run.text.is_empty())
            .expect("last inline fragment text");
        assert_ne!(
            first_text.border_radii.top_left,
            crate::types::CornerRadius::ZERO
        );
        assert_eq!(
            first_text.border_radii.top_right,
            crate::types::CornerRadius::ZERO
        );
        assert_eq!(
            last_text.border_radii.top_left,
            crate::types::CornerRadius::ZERO
        );
        assert_ne!(
            last_text.border_radii.top_right,
            crate::types::CornerRadius::ZERO
        );
    }

    #[test]
    fn opening_inline_edge_wraps_with_the_first_span_word() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div>alpha <span style='padding-left:20px;padding-right:20px'>\
             beta gamma</span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(50.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );

        assert_eq!(line_text(&lines[0]), "alpha");
        assert_eq!(line_text(&lines[1]), "beta");
        assert!(
            lines[1].runs.first().is_some_and(TextRun::is_inline_edge),
            "the opening edge must move with beta"
        );
    }

    #[test]
    fn nested_opening_edges_wrap_with_the_innermost_word() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div>alpha <span style='padding-left:8px'><span style='padding-left:12px'>\
             beta</span></span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(50.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );

        assert_eq!(line_text(&lines[0]), "alpha");
        assert_eq!(line_text(&lines[1]), "beta");
        assert!(lines[1].runs[0].is_opening_inline_edge());
        assert!(lines[1].runs[1].is_opening_inline_edge());
    }

    #[test]
    fn nested_opening_edges_overflow_together_when_the_group_cannot_fit() {
        let fonts = HashMap::new();
        let runs = inline_runs(
            "<div><span style='padding-left:12px'><span style='padding-left:12px'>\
             beta</span></span></div>",
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(10.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].runs[0].is_opening_inline_edge());
        assert!(lines[0].runs[1].is_opening_inline_edge());
        assert_eq!(line_text(&lines[0]), "beta");
    }

    #[test]
    fn vertical_inline_edges_do_not_enlarge_line_boxes() {
        let fonts = HashMap::new();
        let options = TextWrapOptions::new(200.0, 3.0, 1.0, OverflowWrap::Normal);
        let plain = wrap_text_runs(
            inline_runs("<div style='font-size:4px;line-height:1'>A<span>B</span>C</div>"),
            options,
            &fonts,
        );
        let padded = wrap_text_runs(
            inline_runs(
                "<div style='font-size:4px;line-height:1'>A<span style='padding:40px'>B</span>C</div>",
            ),
            options,
            &fonts,
        );

        assert_eq!(padded[0].height, plain[0].height);
        assert_eq!(padded[0].baseline_ascent, plain[0].baseline_ascent);
    }

    #[test]
    fn inline_wrapping_and_min_content_preserve_subpoint_widths() {
        let half = inline_box_from_markup(
            r#"<span style="display:inline-block;width:0.5pt;font-size:0.5pt;line-height:1;overflow-wrap:anywhere">i i i i</span>"#,
        );
        let thousandth = inline_box_from_markup(
            r#"<span style="display:inline-block;width:0.001pt;font-size:0.5pt;line-height:1;overflow-wrap:anywhere">i i i i</span>"#,
        );
        let intrinsic = inline_box_from_markup(
            r#"<span style="display:inline-block;width:min-content;font-size:0.001pt;line-height:1;overflow-wrap:anywhere">iiii</span>"#,
        );

        assert_eq!(half.width, 0.5);
        assert_eq!(thousandth.width, 0.001);
        assert!(half.lines.len() > 1, "0.5pt inline box wrapped at 1pt");
        assert!(
            thousandth.lines.len() > half.lines.len(),
            "0.001pt inline box was promoted to the half-point or 1pt layout lane: {} vs {} lines",
            thousandth.lines.len(),
            half.lines.len()
        );
        assert_eq!(
            intrinsic.lines.len(),
            4,
            "min-content anywhere must wrap at one subpoint glyph, not 1pt"
        );
        assert!(intrinsic.width < 0.001);
    }

    #[test]
    fn footnote_pseudo_current_color_uses_originating_foreground() {
        let rules =
            crate::parser::css::parse_stylesheet("aside::footnote-call { color: currentColor }");
        let mut parent = ComputedStyle::default();
        parent.color = crate::types::Color::rgb(20, 40, 60);
        let pseudo = crate::style::computed::compute_pseudo_element_style(
            &parent,
            &rules,
            "aside",
            &[],
            None,
            &HashMap::new(),
            &SelectorContext::default(),
            PseudoElement::FootnoteCall,
        )
        .expect("footnote call style");

        assert_eq!(pseudo.color.to_f32_rgb(), parent.color.to_f32_rgb());
        assert_eq!(pseudo.font_variant_position, FontVariantPosition::Super);
    }

    #[test]
    fn font_variant_position_scales_glyphs_without_shrinking_line_height() {
        let style = ComputedStyle {
            font_size: 12.0,
            line_height: 1.5,
            font_variant_position: FontVariantPosition::Super,
            ..Default::default()
        };
        let run = styled_text_run(
            "1".to_string(),
            &style,
            None,
            None,
            EdgeSizes::ZERO,
            &HashMap::new(),
        );

        assert_eq!(run.font_size, 9.6);
        assert_eq!(run.line_height_font_size(), 12.0);
        assert_eq!(run.font_variant_position, FontVariantPosition::Super);
        assert_eq!(run.vertical_align_shift(12.0), 0.0);
        assert!(run.glyph_baseline_shift(12.0) > 0.0);

        let normal = styled_text_run(
            "1".to_string(),
            &ComputedStyle {
                font_size: 12.0,
                line_height: 1.5,
                ..Default::default()
            },
            None,
            None,
            EdgeSizes::ZERO,
            &HashMap::new(),
        );
        let options = TextWrapOptions::new(100.0, 12.0, 1.5, OverflowWrap::Normal);
        let variant_line = wrap_text_runs(vec![run], options, &HashMap::new());
        let normal_line = wrap_text_runs(vec![normal], options, &HashMap::new());

        assert_eq!(variant_line[0].height, normal_line[0].height);
        assert_eq!(
            variant_line[0].baseline_ascent,
            normal_line[0].baseline_ascent
        );
    }

    #[test]
    fn footnote_pseudo_counter_preserves_the_authored_counter_style() {
        let style = ComputedStyle {
            content: vec![ContentItem::Counter(
                "footnote".to_string(),
                crate::style::computed::ListStyleType::UpperRoman,
            )],
            ..Default::default()
        };

        assert_eq!(
            footnote_pseudo_content(Some(&style), "4"),
            Some("IV".to_string())
        );
    }

    fn plain_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_size: 16.0,
            ..Default::default()
        }
    }

    fn parity_sans_fonts() -> HashMap<String, TtfFont> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
        HashMap::from([("paritysans".to_string(), font)])
    }

    fn parity_font(path: &str) -> TtfFont {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts")
                .join(path),
        )
        .expect("parity test font");
        crate::parser::ttf::parse_ttf(bytes).expect("valid parity TTF")
    }

    #[test]
    fn fallback_splitting_resolves_mixed_text_by_grapheme_cluster() {
        let japanese = crate::parser::ttf::parse_ttf(
            include_bytes!("../../tests/fonts/IronpressCjkVertical.ttf").to_vec(),
        )
        .expect("valid Japanese fixture font");
        let emoji = crate::parser::ttf::parse_ttf(
            include_bytes!("../../tests/fonts/NotoEmoji-TestSubset.ttf").to_vec(),
        )
        .expect("valid emoji fixture font");
        let fonts = HashMap::from([
            (
                crate::font_pack::CJK_JAPANESE_FALLBACK_KEY.to_string(),
                japanese,
            ),
            (crate::system_fonts::EMOJI_FALLBACK_KEY.to_string(), emoji),
        ]);
        let mut run = plain_run("Hello 第 😀");
        run.metadata.font_locale = crate::font_pack::FontLocale::Japanese;
        let mut runs = Vec::new();

        push_text_run_with_fallback(run, &mut runs, &fonts);

        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0].text, "Hello ");
        assert_eq!(runs[1].font_family.name(), "__cjk_japanese_fallback");
        assert_eq!(runs[1].css_font_family().name(), "Helvetica");
        assert_eq!(runs[2].text, " ");
        assert_eq!(runs[3].font_family.name(), "__emoji_fallback");
    }

    #[test]
    fn fallback_glyph_face_does_not_change_the_css_line_box() {
        let japanese = crate::parser::ttf::parse_ttf(
            include_bytes!("../../tests/fonts/IronpressCjkVertical.ttf").to_vec(),
        )
        .expect("valid Japanese fixture font");
        let fonts = HashMap::from([
            ("paritysans".to_string(), parity_font("ParitySans.ttf")),
            (
                crate::font_pack::CJK_JAPANESE_FALLBACK_KEY.to_string(),
                japanese,
            ),
        ]);
        let mut run = plain_run("第");
        run.font_family = FontFamily::Custom("ParitySans".to_string());
        run.line_height_basis = run.font_size;
        run.line_height_factor = 1.5;
        run.metadata.font_locale = crate::font_pack::FontLocale::Japanese;
        let parent = LineStrut::from_font(
            &run.font_family,
            run.font_size,
            run.bold,
            run.font_style.is_slanted(),
            run.font_size * run.line_height_factor,
            &fonts,
        );
        let expected = resolve_line_box_metrics(
            std::slice::from_ref(&run),
            Some(parent),
            run.line_height_factor,
            &fonts,
        );
        let mut fallback_runs = Vec::new();

        push_text_run_with_fallback(run, &mut fallback_runs, &fonts);

        assert_eq!(
            resolve_line_box_metrics(&fallback_runs, Some(parent), 1.5, &fonts),
            expected
        );
    }

    #[test]
    fn font_face_unicode_range_precedes_optional_fallback_packs() {
        let mut range_a = parity_font("ParitySerif.ttf");
        range_a
            .cmap
            .retain(|codepoint, _| *codepoint == u32::from('A'));
        let mut range_b = parity_font("ParitySans.ttf");
        range_b
            .cmap
            .retain(|codepoint, _| *codepoint == u32::from('B'));
        let fonts = HashMap::from([
            ("rangepick".to_string(), range_a),
            ("rangepick__fontface_1".to_string(), range_b),
            (
                crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
                parity_font("ParitySans.ttf"),
            ),
        ]);
        let mut run = plain_run("B");
        run.font_family = FontFamily::Custom("RangePick".to_string());
        let mut runs = Vec::new();

        push_text_run_with_fallback(run, &mut runs, &fonts);

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].font_family.name(), "RangePick");
    }

    #[test]
    fn fallback_splitting_preserves_empty_runs_that_carry_line_geometry() {
        let fonts = HashMap::from([(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            parity_font("ParitySans.ttf"),
        )]);
        let mut run = plain_run("");
        run.inline_box = Some(Box::new(InlineBox {
            width: 12.0,
            ..InlineBox::default()
        }));
        let mut runs = Vec::new();

        push_text_run_with_fallback(run, &mut runs, &fonts);

        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].inline_box.as_ref().map(|box_| box_.width),
            Some(12.0)
        );
    }

    #[test]
    fn hyphen_breaks_follow_uax14_and_css_digit_tailoring() {
        let parts = |word: &str| {
            let mut tokens = Vec::new();
            push_word_with_hyphen_breaks(word, 0, &mut tokens, false, true);
            tokens
                .into_iter()
                .map(|token| token.text)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            parts("MATERIALS-LABEL-02-LONG"),
            ["MATERIALS-", "LABEL-", "02-", "LONG"]
        );
        assert_eq!(parts("-5"), ["-5"]);
    }

    #[test]
    fn line_extents_use_css_pixel_font_metrics_and_block_start_leading() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        for (font_size, line_height) in [
            (12.0, 15.1875),
            (12.0, 15.375),
            (12.0, 18.0),
            (18.0, 18.0),
            (30.0, 30.0),
        ] {
            let extents = line_extents(&family, font_size, false, false, line_height, &fonts);
            let metrics = crate::fonts::font_line_metrics(&family, font_size, false, false, &fonts);
            let above_leading = crate::fonts::floor_to_css_pixel(
                (line_height - metrics.ascent - metrics.descent) / 2.0,
            );
            assert!(
                (extents.above - (metrics.ascent + above_leading)).abs() < 0.000_01,
                "font size {font_size}pt"
            );
            assert!(
                (extents.below - (line_height - metrics.ascent - above_leading)).abs() < 0.000_01,
                "font size {font_size}pt"
            );
            assert_eq!(
                extents.above + extents.below,
                line_height,
                "font size {font_size}pt"
            );
        }
    }

    #[test]
    fn decorated_parent_uses_pure_line_height_and_explicit_metadata() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut style = ComputedStyle::default();
        style.font_family = family.clone();
        style.font_stack = crate::style::computed::FontStack::from_family(family);
        style.font_size = 12.0;
        style.line_height = 1.5;
        style.text_decorations.current.style = crate::style::computed::TextDecorationStyle::Wavy;
        style.text_decorations.current.thickness = Some(1.25);
        style.text_decorations.current.underline_offset = Some(2.5);
        style.text_emphasis_mark = true;
        style.text_emphasis_color = crate::types::Color::rgb(21, 101, 192);

        let factor = resolved_line_height_factor(&style, &fonts);
        let strut = parent_line_strut(&style, &fonts);
        let metadata = text_run_metadata(&style);

        assert_eq!(factor, 1.5);
        assert!(factor.is_finite());
        let expected = LineStrut::from_font(&style.font_family, 12.0, false, false, 18.0, &fonts);
        assert!((strut.above - expected.above).abs() < f32::EPSILON);
        assert!((strut.below - expected.below).abs() < f32::EPSILON);
        assert_eq!(
            style.text_decorations.current.style,
            crate::style::computed::TextDecorationStyle::Wavy
        );
        assert_eq!(style.text_decorations.current.thickness, Some(1.25));
        assert_eq!(style.text_decorations.current.underline_offset, Some(2.5));
        assert!(metadata.emphasis.mark);
        assert_eq!(
            metadata.emphasis.color,
            crate::types::Color::rgb(21, 101, 192)
        );
    }

    #[test]
    fn decoration_padding_preserves_all_authored_edges() {
        let mut style = ComputedStyle::default();
        style.padding = EdgeSizes::new(1.25, 2.5, 3.75, 5.0);

        assert_eq!(
            decoration_padding(
                &style,
                Some(crate::types::Color::from_srgb(0.2, 0.3, 0.4, 1.0)),
            ),
            style.padding
        );
        assert_eq!(decoration_padding(&style, None), EdgeSizes::ZERO);
    }

    #[test]
    fn typed_font_synthesis_coexists_with_decoration_geometry() {
        let fonts = parity_sans_fonts();
        let padding = EdgeSizes::new(1.0, 2.0, 3.0, 4.0);
        let background = Some(crate::types::Color::from_srgb(0.9, 0.8, 0.2, 1.0));
        let mut weighted = plain_run("Heavy");
        weighted.font_family = FontFamily::Custom("ParitySans".to_string());
        weighted.bold = true;
        weighted.background_color = background;
        weighted.padding = padding;

        mark_synthetic_weight_run(&mut weighted, FontWeight::Number(900), &fonts);

        assert_eq!(weighted.font_synthesis.weight, SyntheticFontWeight::Auto);
        assert_eq!(weighted.padding, padding);
        assert_eq!(weighted.background_color, background);

        let mut suppressed = weighted.clone();
        suppressed.bold = false;
        suppressed.font_synthesis.weight = SyntheticFontWeight::Auto;
        mark_synthetic_weight_run(&mut suppressed, FontWeight::Bold, &fonts);
        assert_eq!(
            suppressed.font_synthesis.weight,
            SyntheticFontWeight::Suppressed
        );
        assert_eq!(suppressed.padding, padding);

        let mut small_caps = plain_run("small");
        small_caps.background_color = background;
        small_caps.padding = padding;
        let mut synthesized = Vec::new();
        push_styled_run(
            small_caps,
            crate::style::computed::FontVariantCaps::SmallCaps,
            true,
            FontWeight::Normal,
            &mut synthesized,
            &HashMap::new(),
        );

        assert_eq!(synthesized.len(), 1);
        assert_eq!(synthesized[0].text, "SMALL");
        assert_eq!(synthesized[0].font_size, 11.25);
        assert!(synthesized[0].font_synthesis.small_caps);
        assert_eq!(synthesized[0].padding, padding);
        assert_eq!(synthesized[0].background_color, background);
    }

    #[test]
    fn synthesized_small_caps_round_to_css_pixels_before_shaping() {
        assert_eq!(synthesized_small_caps_font_size(29.0 * 0.75), 15.0);
        assert_eq!(synthesized_small_caps_font_size(31.0 * 0.75), 16.5);
        assert_eq!(synthesized_small_caps_font_size(34.0 * 0.75), 18.0);
        assert_eq!(synthesized_small_caps_font_size(35.0 * 0.75), 18.75);
    }

    #[test]
    fn synthesized_small_caps_keep_intervening_spaces_at_the_base_size() {
        let mut runs = Vec::new();
        push_styled_run(
            plain_run("a b"),
            crate::style::computed::FontVariantCaps::SmallCaps,
            true,
            FontWeight::Normal,
            &mut runs,
            &HashMap::new(),
        );

        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "A");
        assert_eq!(runs[0].font_size, 11.25);
        assert_eq!(runs[1].text, " ");
        assert_eq!(runs[1].font_size, 16.0);
        assert_eq!(runs[1].metadata.whitespace, RunWhitespace::Preserve);
        assert_eq!(runs[2].text, "B");
        assert_eq!(runs[2].font_size, 11.25);
    }

    #[test]
    fn synthetic_weight_is_consistent_across_font_sizes() {
        let fonts = parity_sans_fonts();
        for (font_size, expected_stroke) in [(22.5, 0.755_208_3), (34.5, 1.078_125)] {
            let mut run = plain_run("UNDER");
            run.font_size = font_size;
            run.bold = true;
            run.font_family = FontFamily::Custom("ParitySans".to_string());
            run.text_shadow.push(crate::style::computed::BoxShadow {
                offset_x: 0.0,
                offset_y: 0.0,
                blur: 0.0,
                spread: 0.0,
                color: crate::types::Color::BLACK,
                color_source: crate::style::computed::ColorSource::Absolute,
                inset: false,
            });

            mark_synthetic_weight_run(&mut run, FontWeight::Bold, &fonts);

            assert_eq!(run.font_synthesis.weight, SyntheticFontWeight::Auto);
            assert_eq!(
                run.synthetic_bold_stroke_width(&fonts),
                Some(expected_stroke)
            );
        }
    }

    #[test]
    fn normal_parent_line_height_comes_from_resolved_font_metrics() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut style = ComputedStyle::default();
        style.font_family = family.clone();
        style.font_stack = crate::style::computed::FontStack::from_family(family);
        style.font_size = 12.0;
        style.line_height = f32::NAN;
        style.line_height_absolute = None;

        assert_eq!(used_line_height(&style, &fonts), 14.25);
        assert_eq!(resolved_line_height_factor(&style, &fonts), 1.1875);
    }

    #[test]
    fn inherited_parent_strut_contributes_below_large_child_span() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut child = plain_run("Baseline Hxy");
        child.font_family = family.clone();
        child.font_size = 30.0; // 40 CSS px
        child.line_height_factor = 1.0;
        let parent = line_extents(&family, 12.0, false, false, 18.0, &fonts);
        let options =
            TextWrapOptions::new(300.0, 12.0, 1.5, OverflowWrap::Normal).with_parent_strut(parent);
        let lines = wrap_text_runs(vec![child], options, &fonts);

        assert_eq!(lines[0].baseline_ascent, Some(25.5));
        assert_eq!(lines[0].height, 30.75);
    }

    #[test]
    fn parent_strut_combines_ascent_and_descent_across_multiple_runs() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut large = plain_run("Large");
        large.font_family = family.clone();
        large.font_size = 30.0;
        large.line_height_factor = 1.0;
        let mut parent_sized = plain_run(" small");
        parent_sized.font_family = family.clone();
        parent_sized.font_size = 12.0;
        parent_sized.line_height_factor = 1.5;
        let parent = line_extents(&family, 12.0, false, false, 18.0, &fonts);
        let options =
            TextWrapOptions::new(300.0, 12.0, 1.5, OverflowWrap::Normal).with_parent_strut(parent);

        let lines = wrap_text_runs(vec![large, parent_sized], options, &fonts);

        assert_eq!(lines[0].baseline_ascent, Some(25.5));
        assert_eq!(lines[0].height, 30.75);
    }

    #[test]
    fn empty_inline_content_still_gets_the_parent_strut() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut empty = plain_run("");
        empty.font_family = family.clone();
        empty.font_size = 12.0;
        empty.line_height_factor = 1.5;
        let parent = line_extents(&family, 12.0, false, false, 18.0, &fonts);
        let options =
            TextWrapOptions::new(300.0, 12.0, 1.5, OverflowWrap::Normal).with_parent_strut(parent);

        let lines = wrap_text_runs(vec![empty], options, &fonts);

        assert_eq!(lines[0].baseline_ascent, Some(parent.above));
        assert!((lines[0].height - 18.0).abs() < 0.000_1);
    }

    #[test]
    fn wholly_contained_parent_strut_does_not_enlarge_the_line() {
        let fonts = parity_sans_fonts();
        let family = FontFamily::Custom("ParitySans".to_string());
        let mut child = plain_run("Tall child");
        child.font_family = family.clone();
        child.font_size = 30.0;
        child.line_height_factor = 1.5;
        let parent = line_extents(&family, 12.0, false, false, 18.0, &fonts);
        let base = TextWrapOptions::new(300.0, 12.0, 1.5, OverflowWrap::Normal);

        let without = wrap_text_runs(vec![child.clone()], base, &fonts);
        let with = wrap_text_runs(vec![child], base.with_parent_strut(parent), &fonts);

        assert!((with[0].height - without[0].height).abs() < 0.000_1);
        assert!(
            (with[0].baseline_ascent.unwrap() - without[0].baseline_ascent.unwrap()).abs()
                < 0.000_1
        );
    }

    #[test]
    fn pre_line_collapses_spaces_but_keeps_newlines() {
        // css-text-3 §4.1.1: pre-line collapses runs of spaces/tabs to a single
        // space, removes collapsible spaces around a segment break, but PRESERVES
        // the forced segment break (newline) so the line breaker forces a break.
        assert_eq!(
            collapse_whitespace_pre_line("alpha    beta\ngamma delta"),
            "alpha beta\ngamma delta"
        );
        // Spaces adjacent to the newline are removed; leading/trailing collapse.
        assert_eq!(collapse_whitespace_pre_line("  a   \n   b  "), "a\nb");
        // Tabs collapse like spaces; multiple newlines are each preserved.
        assert_eq!(collapse_whitespace_pre_line("x\t\ty\n\nz"), "x y\n\nz");
        // Contrast with `normal` collapse, which drops the newline entirely.
        assert_eq!(collapse_whitespace("alpha\nbeta"), "alpha beta");
    }

    #[test]
    fn unicode_spacing_characters_are_not_collapsible_css_spaces() {
        let unicode_spaces = "\u{00A0}\u{00A0}\u{2002}\u{2003}";

        assert_eq!(
            collapse_whitespace(&format!(" \tX{unicode_spaces}Y{unicode_spaces}")),
            format!("X{unicode_spaces}Y{unicode_spaces}"),
        );
    }

    #[test]
    fn unicode_spacing_characters_are_renderable_text() {
        assert!(has_non_collapsible_text("\u{00A0}\u{2002}\u{2003}"));
        assert!(!has_non_collapsible_text(" \t\n\r"));
    }

    #[test]
    fn no_break_space_keeps_one_unbreakable_intrinsic_group() {
        let fonts = parity_sans_fonts();
        let run = parity_run("Alpha\u{00A0}Beta");
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);
        let intrinsic = measure_text_intrinsic_widths(vec![run.clone()], options, true, &fonts);

        assert_eq!(intrinsic.min_content, intrinsic.max_content);
        let lines = wrap_text_runs(
            vec![run],
            options_at_width(options, previous_f32(intrinsic.max_content)),
            &fonts,
        );
        assert_eq!(lines.len(), 1, "NBSP is not a soft wrap opportunity");
        assert_eq!(line_text(&lines[0]), "Alpha\u{00A0}Beta");
    }

    #[test]
    fn text_indent_shrinks_only_the_first_line() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let runs = vec![plain_run("aaaa bbbb cccc dddd")];
        // Width chosen so several words fit on the first line with no indent,
        // but a large indent forces an earlier first-line wrap.
        let opts = TextWrapOptions::new(140.0, 16.0, 1.2, OverflowWrap::Normal);

        let no_indent = wrap_text_runs(runs.clone(), opts, &fonts);
        let indented = wrap_text_runs(runs, opts.with_text_indent(60.0), &fonts);

        let count_words = |line: &TextLine| {
            line.runs
                .iter()
                .map(|r| r.text.split_whitespace().count())
                .sum::<usize>()
        };
        // The indent consumes inline space on the FIRST line only, so it holds
        // fewer words than the un-indented first line.
        assert!(
            count_words(&indented[0]) < count_words(&no_indent[0]),
            "indented first line should hold fewer words: {} vs {}",
            count_words(&indented[0]),
            count_words(&no_indent[0])
        );
        // A subsequent line is unaffected by the indent and can still hold the
        // full un-indented width's worth of words.
        assert!(
            indented.len() >= 2,
            "indented paragraph should wrap to at least two lines"
        );
    }

    fn parity_run(text: &str) -> TextRun {
        let mut run = plain_run(text);
        run.font_family = FontFamily::Custom("ParitySans".to_string());
        run
    }

    fn options_at_width(mut options: TextWrapOptions, width: f32) -> TextWrapOptions {
        options.max_width = width;
        options
    }

    fn previous_f32(value: f32) -> f32 {
        assert!(value.is_finite() && value > 0.0);
        f32::from_bits(value.to_bits() - 1)
    }

    #[test]
    fn intrinsic_max_content_is_the_wrappers_exact_fit_boundary() {
        let fonts = parity_sans_fonts();
        let runs = vec![parity_run("cell text")];
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);
        let intrinsic = measure_text_intrinsic_widths(runs.clone(), options, true, &fonts);

        let exact = wrap_text_runs(
            runs.clone(),
            options_at_width(options, intrinsic.max_content),
            &fonts,
        );
        let below = wrap_text_runs(
            runs,
            options_at_width(options, previous_f32(intrinsic.max_content)),
            &fonts,
        );

        assert_eq!(exact.len(), 1, "max-content must fit without layout slack");
        assert_eq!(below.len(), 2, "one representable step below must wrap");
        assert!(intrinsic.min_content < intrinsic.max_content);
    }

    #[test]
    fn anywhere_tokens_use_the_same_ligature_buffer_as_paint() {
        let fonts = parity_sans_fonts();
        let run = parity_run("verification/path");
        let prefix_width = estimate_text_width_for_run("verification", &run, &fonts);
        let options = TextWrapOptions::new(prefix_width, 16.0, 1.2, OverflowWrap::Normal);

        let lines = wrap_text_runs(vec![run], options, &fonts);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "verification");
        assert_eq!(line_text(&lines[1]), "/path");
    }

    #[test]
    fn intrinsic_and_wrap_share_cross_run_background_space_metrics() {
        let fonts = parity_sans_fonts();
        let first = parity_run("alpha ");
        let mut highlighted = parity_run("beta");
        highlighted.font_size = 13.0;
        highlighted.bold = true;
        highlighted.background_color = Some(crate::types::Color::from_srgb(0.9, 0.8, 0.2, 1.0));
        let runs = vec![first, highlighted];
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);
        let intrinsic = measure_text_intrinsic_widths(runs.clone(), options, true, &fonts);

        let exact = wrap_text_runs(
            runs.clone(),
            options_at_width(options, intrinsic.max_content),
            &fonts,
        );
        let below = wrap_text_runs(
            runs,
            options_at_width(options, previous_f32(intrinsic.max_content)),
            &fonts,
        );

        assert_eq!(exact.len(), 1);
        assert_eq!(below.len(), 2);
        assert_eq!(line_text(&exact[0]), "alpha beta");
        assert_eq!(exact[0].runs[1].text, " ");
        assert!(exact[0].runs[1].background_color.is_none());
    }

    #[test]
    fn adjacent_styled_runs_form_one_min_content_word() {
        let fonts = parity_sans_fonts();
        let first = parity_run("inter");
        let mut second = parity_run("national");
        second.bold = true;
        let runs = vec![first, second];
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);
        let intrinsic = measure_text_intrinsic_widths(runs.clone(), options, true, &fonts);

        assert_eq!(intrinsic.min_content, intrinsic.max_content);
        let lines = wrap_text_runs(
            runs,
            options_at_width(options, previous_f32(intrinsic.min_content)),
            &fonts,
        );
        assert_eq!(lines.len(), 1, "a style boundary is not a wrap opportunity");
        assert_eq!(line_text(&lines[0]), "international");
    }

    #[test]
    fn common_inline_ancestor_owns_tracking_across_paint_runs() {
        let mut runs = vec![parity_run("A"), parity_run("B")];

        runs.as_mut_slice()
            .resolve_unclaimed_boundaries(crate::layout::elements::TextSpacing::new(0.7, 0.0));

        assert_eq!(runs[0].metadata.boundary.total(), 0.7);
        assert_eq!(runs[1].metadata.boundary.total(), 0.0);
    }

    #[test]
    fn nested_inline_tracking_ownership_survives_outer_resolution() {
        let mut runs = vec![
            parity_run("A"),
            parity_run("B"),
            parity_run("C"),
            parity_run("D"),
        ];

        runs[1..3]
            .resolve_unclaimed_boundaries(crate::layout::elements::TextSpacing::new(2.0, 0.0));
        runs.as_mut_slice()
            .resolve_unclaimed_boundaries(crate::layout::elements::TextSpacing::new(1.0, 0.0));

        assert_eq!(runs[0].metadata.boundary.total(), 1.0);
        assert_eq!(runs[1].metadata.boundary.total(), 2.0);
        assert_eq!(runs[2].metadata.boundary.total(), 1.0);
    }

    #[test]
    fn only_the_final_wrapped_fragment_keeps_the_source_boundary() {
        let mut run = parity_run("alpha beta");
        run.metadata.boundary.resolve_letter_spacing(1.25);

        let prefix = run.text_fragment("alpha".to_owned(), false);
        let suffix = run.text_fragment("beta".to_owned(), true);

        assert_eq!(prefix.metadata.boundary.total(), 0.0);
        assert_eq!(suffix.metadata.boundary.total(), 1.25);
    }

    #[test]
    fn tracking_counts_extended_grapheme_clusters_not_code_points() {
        let spacing = crate::layout::elements::TextSpacing::new(3.0, 0.0);

        assert_eq!(spacing.add_internal_advance(5.0, "e\u{301}"), 5.0);
        assert_eq!(spacing.add_internal_advance(5.0, "e\u{301}x"), 8.0);
    }

    #[test]
    fn collapsed_word_tokens_preserve_every_internal_tracking_boundary() {
        let fonts = parity_sans_fonts();
        let mut run = parity_run("alpha beta");
        run.metadata.spacing.letter = 3.0;
        let expected = estimate_text_width_for_run(&run.text, &run, &fonts);
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);

        let intrinsic = measure_text_intrinsic_widths(vec![run], options, true, &fonts);

        assert_eq!(intrinsic.max_content, expected);
    }

    #[test]
    fn preserved_space_tokens_preserve_tracking_and_word_spacing() {
        let fonts = parity_sans_fonts();
        let mut run = parity_run("alpha   beta");
        run.metadata.spacing = crate::layout::elements::TextSpacing::new(2.0, 5.0);
        let expected = estimate_text_width_for_run(&run.text, &run, &fonts);
        let base = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);

        for white_space in [WhiteSpace::PreWrap, WhiteSpace::BreakSpaces] {
            let intrinsic = measure_text_intrinsic_widths(
                vec![run.clone()],
                base.with_white_space(white_space),
                true,
                &fonts,
            );

            assert_eq!(intrinsic.max_content, expected);
        }
    }

    #[test]
    fn intrinsic_width_includes_letter_spacing_and_nowrap_minimum() {
        let fonts = parity_sans_fonts();
        let plain = vec![parity_run("tracked words")];
        let mut tracked_run = parity_run("tracked words");
        tracked_run.metadata.spacing.letter = 1.25;
        let tracked = vec![tracked_run];
        let options = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);

        let plain_width = measure_text_intrinsic_widths(plain, options, true, &fonts).max_content;
        let intrinsic = measure_text_intrinsic_widths(tracked.clone(), options, false, &fonts);
        assert!(intrinsic.max_content > plain_width);
        assert_eq!(intrinsic.min_content, intrinsic.max_content);
        assert_eq!(
            wrap_text_runs(
                tracked,
                options_at_width(options, intrinsic.max_content),
                &fonts,
            )
            .len(),
            1
        );
    }

    #[test]
    fn anywhere_but_not_break_word_reduces_the_shared_min_content_measure() {
        let fonts = parity_sans_fonts();
        let mut run = parity_run("Supercalifragilistic");
        run.metadata.spacing.letter = 0.75;
        let base = TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal);

        let normal = measure_text_intrinsic_widths(vec![run.clone()], base, true, &fonts);
        let break_word = measure_text_intrinsic_widths(
            vec![run.clone()],
            TextWrapOptions {
                overflow_wrap: OverflowWrap::BreakWord,
                ..base
            },
            true,
            &fonts,
        );
        let anywhere = measure_text_intrinsic_widths(
            vec![run.clone()],
            TextWrapOptions {
                overflow_wrap: OverflowWrap::Anywhere,
                ..base
            },
            true,
            &fonts,
        );
        let widest_character = run
            .text
            .chars()
            .map(|character| estimate_text_width_for_run(&character.to_string(), &run, &fonts))
            .fold(0.0_f32, f32::max);

        assert_eq!(normal.min_content, normal.max_content);
        assert_eq!(break_word, normal);
        assert_eq!(anywhere.max_content, normal.max_content);
        assert_eq!(anywhere.min_content, widest_character);
    }

    #[test]
    fn styled_tokens_store_run_indices_instead_of_style_copies() {
        assert!(
            std::mem::size_of::<StyledWord>() < std::mem::size_of::<TextRun>(),
            "a token must remain smaller than the style arena entry it references"
        );
    }

    #[test]
    fn preserved_spaces_obey_exact_and_previous_float_boundaries() {
        let fonts = parity_sans_fonts();
        for options in [
            TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal)
                .with_white_space(WhiteSpace::PreWrap),
            TextWrapOptions::new(500.0, 16.0, 1.2, OverflowWrap::Normal)
                .with_white_space(WhiteSpace::BreakSpaces),
        ] {
            let runs = vec![parity_run("a    \n")];
            let intrinsic = measure_text_intrinsic_widths(runs.clone(), options, true, &fonts);
            let exact = wrap_text_runs(
                runs.clone(),
                options_at_width(options, intrinsic.max_content),
                &fonts,
            );
            let below = wrap_text_runs(
                runs,
                options_at_width(options, previous_f32(intrinsic.max_content)),
                &fonts,
            );

            assert_eq!(exact.len(), 1, "exact preserved-space width must fit");
            assert_eq!(
                below.len(),
                2,
                "the preceding representable width must cross the space boundary"
            );
        }
    }

    #[test]
    fn pre_and_nowrap_disable_soft_wrapping_at_the_option_boundary() {
        let fonts = parity_sans_fonts();
        let narrow = TextWrapOptions::new(1.0, 16.0, 1.2, OverflowWrap::Normal);

        let pre = wrap_text_runs(
            vec![parity_run("  alpha   beta  ")],
            narrow.with_white_space(WhiteSpace::Pre),
            &fonts,
        );
        assert_eq!(pre.len(), 1);
        assert_eq!(line_text(&pre[0]), "  alpha   beta  ");

        let nowrap = wrap_text_runs(
            vec![parity_run("alpha beta gamma")],
            narrow.with_white_space(WhiteSpace::NoWrap),
            &fonts,
        );
        assert_eq!(nowrap.len(), 1);
    }

    #[test]
    fn word_break_keep_all_keeps_cjk_punctuation_runs_whole() {
        assert!(should_break_as_char_tokens("你好/世界", false));
        assert!(!should_break_as_char_tokens("你好/世界", true));
    }

    /// Concatenate a line's runs back into its rendered text.
    fn line_text(line: &TextLine) -> String {
        line.runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn break_spaces_rolls_word_past_non_hanging_spaces() {
        // css-text-3 §3 break-spaces: preserved spaces never hang and a wrap
        // opportunity exists after each. With `alpha    beta gamma`, once `alpha`
        // plus the four preserved spaces fill most of the line, `beta` fits but
        // its own trailing space would overflow — and since that space cannot
        // hang, the line must roll back to after the four spaces, sending `beta`
        // to the next line. Contrast with pre-wrap, where the trailing space
        // hangs so `alpha    beta` stays together.
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let runs = vec![plain_run("alpha    beta gamma")];
        // Helvetica metrics: pick a width that fits "alpha" + 4 spaces + "beta"
        // but not the following space, mirroring the parity fixtures.
        let alpha =
            estimate_word_width("alpha", 16.0, &FontFamily::Helvetica, false, false, &fonts);
        let beta = estimate_word_width("beta", 16.0, &FontFamily::Helvetica, false, false, &fonts);
        let sp = estimate_word_width(" ", 16.0, &FontFamily::Helvetica, false, false, &fonts);
        // Room for alpha + 4 spaces + beta, but not a 5th space after beta.
        let width = alpha + 4.0 * sp + beta + sp * 0.5;
        let opts = TextWrapOptions::new(width, 16.0, 1.2, OverflowWrap::Normal);

        let pre_wrap = wrap_text_runs(
            runs.clone(),
            opts.with_white_space(WhiteSpace::PreWrap),
            &fonts,
        );
        let break_spaces =
            wrap_text_runs(runs, opts.with_white_space(WhiteSpace::BreakSpaces), &fonts);

        // pre-wrap keeps the trailing space hanging, so beta stays on line 1.
        assert!(
            line_text(&pre_wrap[0]).contains("beta"),
            "pre-wrap line 1 should keep beta: {:?}",
            line_text(&pre_wrap[0])
        );
        // break-spaces rolls beta onto the next line; line 1 ends after the
        // preserved spaces.
        assert!(
            !line_text(&break_spaces[0]).contains("beta"),
            "break-spaces line 1 should not contain beta: {:?}",
            line_text(&break_spaces[0])
        );
        assert!(
            line_text(&break_spaces[1]).trim_start().starts_with("beta"),
            "break-spaces line 2 should start with beta: {:?}",
            line_text(&break_spaces[1])
        );
        // The four preserved spaces are retained (non-hanging) at the end of
        // line 1: "alpha" + four spaces.
        assert_eq!(line_text(&break_spaces[0]), "alpha    ");
    }
}
