use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
// Re-export OverflowWrap so callers of TextWrapOptions::new can use it
// without a separate import.
pub(crate) use crate::style::computed::OverflowWrap;
use crate::style::computed::{
    BoxSizing, ComputedStyle, Display, FontFamily, FontStyle, FontWeight, WhiteSpace,
    compute_style_with_context,
};
use std::collections::HashMap;

use super::engine::{CounterState, InlineBox, LayoutBorder, TextLine, TextRun};

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
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
    )
}

pub(crate) fn resolved_line_height_factor(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if style.line_height.is_nan() {
        let font_family = resolve_style_font_family(style, fonts);
        crate::fonts::normal_line_height_factor(
            &font_family,
            style.font_weight == FontWeight::Bold,
            style.font_style == FontStyle::Italic,
            fonts,
        )
    } else {
        style.line_height
    }
}

// ---------------------------------------------------------------------------
// collapse_whitespace
// ---------------------------------------------------------------------------

pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }
    result.trim_end().to_string()
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
    if let Some(width) =
        crate::text::measure_text_width(word, font_size, font_family, bold, italic, fonts)
    {
        return width;
    }

    // Use AFM metrics for standard fonts (non-bold for layout estimation)
    crate::fonts::str_width(word, font_size, font_family, false)
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
    /// Paragraph base direction for the Unicode Bidi Algorithm. Set to `true`
    /// when the containing block has `direction: rtl` (or `dir="rtl"`).
    pub(crate) paragraph_rtl: bool,
    /// `white-space: pre-wrap`: preserve spaces/newlines but still allow soft
    /// wrapping at space boundaries. Distinguishes pre-wrap (wraps) from `pre`
    /// (which the caller renders with an unbounded width so it never wraps).
    pub(crate) pre_wrap: bool,
    /// CSS `text-indent` applied to the first formatted line: it consumes inline
    /// space at the start of the first line, so that line has less room before it
    /// wraps. Subsequent lines are unaffected.
    pub(crate) text_indent: f32,
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
            paragraph_rtl: false,
            pre_wrap: false,
            text_indent: 0.0,
        }
    }

    pub(crate) const fn with_rtl(mut self, rtl: bool) -> Self {
        self.paragraph_rtl = rtl;
        self
    }

    pub(crate) const fn with_pre_wrap(mut self, pre_wrap: bool) -> Self {
        self.pre_wrap = pre_wrap;
        self
    }

    pub(crate) const fn with_text_indent(mut self, text_indent: f32) -> Self {
        self.text_indent = text_indent;
        self
    }
}

/// Split a segment of text that preserves its internal whitespace into
/// alternating word-runs and space-runs. Used for `white-space: pre-wrap`,
/// where spaces must be preserved verbatim but lines may still wrap at the
/// boundary between a space-run and the following word. Each emitted token is
/// flagged `preserve_spacing = true` so the wrapper never injects its own
/// inter-word space; soft wrapping then happens via the generic
/// "token overflows the line" break.
fn split_preserving_spaces(
    segment: &str,
    template: &TextRun,
    out: &mut Vec<(String, TextRun, bool)>,
) {
    let mut current = String::new();
    let mut current_is_space: Option<bool> = None;
    for ch in segment.chars() {
        let is_space = ch == ' ' || ch == '\t';
        if current_is_space != Some(is_space) && !current.is_empty() {
            out.push((std::mem::take(&mut current), template.clone(), true));
        }
        current_is_space = Some(is_space);
        current.push(ch);
    }
    if !current.is_empty() {
        out.push((current, template.clone(), true));
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

/// Simple text wrapping using character width estimation.
/// Uses TTF metrics when a custom font is available.
pub(crate) fn wrap_text_runs(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextLine> {
    let line_height_factor = options.line_height_factor.max(0.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width: f32 = 0.0;
    // The line box takes its height from the inline content it contains: each
    // run uses the line-height resolved from its *own* element's style, falling
    // back to the block-level factor when the run leaves it unspecified (NaN).
    let run_line_height = |run: &TextRun| {
        let factor = if run.line_height_factor.is_nan() {
            line_height_factor
        } else {
            run.line_height_factor.max(0.0)
        };
        run.font_size * factor
    };
    // Start the line box height from the first run's line-height contribution.
    let mut line_height = runs
        .first()
        .map_or(options.default_font_size * line_height_factor, |r| {
            run_line_height(r)
        });

    // Apply BiDi reordering if the paragraph direction is RTL or the text
    // contains RTL characters. This reorders runs into visual order so
    // RTL/LTR segments display correctly in the left-to-right PDF context.
    let full_text: String = runs.iter().map(|r| r.text.as_str()).collect();
    let runs = if options.paragraph_rtl || crate::bidi::has_rtl_chars(&full_text) {
        crate::bidi::reorder_runs_bidi(&runs, options.paragraph_rtl)
    } else {
        runs
    };

    // Concatenate all text then re-split by words, preserving run styles.
    // For text containing \n (white-space: pre), split on newlines first,
    // then split each segment by words.
    let mut styled_words: Vec<(String, TextRun, bool)> = Vec::new();
    for run in &runs {
        if run.inline_box.is_some() {
            // Atomic inline box: a single, unbreakable token. `preserve_spacing`
            // is true so no inter-word space is injected before it.
            styled_words.push((String::new(), run.clone(), true));
            continue;
        }
        if run.text == "\n" {
            styled_words.push(("\n".to_string(), run.clone(), false));
            continue;
        }
        let has_newlines = run.text.contains('\n');
        let has_preserved_spacing = run.text.chars().next().is_some_and(char::is_whitespace)
            || run.text.chars().last().is_some_and(char::is_whitespace)
            || run.text.contains("  ");
        if has_newlines {
            for (seg_idx, segment) in run.text.split('\n').enumerate() {
                if seg_idx > 0 {
                    styled_words.push(("\n".to_string(), run.clone(), false));
                }
                if segment.is_empty() {
                    continue;
                }
                let preserved = segment.chars().next().is_some_and(char::is_whitespace)
                    || segment.chars().last().is_some_and(char::is_whitespace)
                    || segment.contains("  ");
                if preserved {
                    if options.pre_wrap {
                        // pre-wrap preserves spaces but still wraps at space
                        // boundaries; split so the generic overflow break can act.
                        split_preserving_spaces(segment, run, &mut styled_words);
                    } else {
                        styled_words.push((segment.to_string(), run.clone(), true));
                    }
                } else {
                    for word in segment.split_whitespace() {
                        styled_words.push((word.to_string(), run.clone(), false));
                    }
                }
            }
        } else if has_preserved_spacing {
            if options.pre_wrap {
                split_preserving_spaces(&run.text, run, &mut styled_words);
            } else {
                styled_words.push((run.text.clone(), run.clone(), true));
            }
        } else {
            for word in run.text.split_whitespace() {
                styled_words.push((word.to_string(), run.clone(), false));
            }
        }
    }

    if styled_words.is_empty() && !runs.is_empty() {
        return vec![TextLine {
            runs,
            height: line_height,
        }];
    }

    // Use a VecDeque so hyphenation remainders can be re-queued for processing.
    let mut queue: std::collections::VecDeque<(String, TextRun, bool)> =
        styled_words.into_iter().collect();

    // CSS `text-indent` only shortens the FIRST formatted line: the inline
    // content available before wrapping is `max_width - text_indent` while no
    // line has been emitted yet, and the full `max_width` afterwards.
    let line_max_width = |emitted: usize| {
        if emitted == 0 {
            (options.max_width - options.text_indent).max(0.0)
        } else {
            options.max_width
        }
    };

    while let Some((word, template, preserve_spacing)) = queue.pop_front() {
        if word == "\n" {
            // Line break
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = run_line_height(&template);
            continue;
        }

        // Atomic inline box: advance by its margin-box width and grow the line
        // box to its height. It wraps to a fresh line if it overflows.
        if let Some(inline) = template.inline_box.as_deref() {
            let box_w = inline.outer_width();
            if current_width > 0.0 && current_width + box_w > line_max_width(lines.len()) {
                lines.push(TextLine {
                    runs: std::mem::take(&mut current_runs),
                    height: line_height,
                });
                current_width = 0.0;
                line_height = run_line_height(&template);
            }
            current_width += box_w;
            // A baseline-aligned box sits above the baseline, so the line must
            // also leave room for the text descender beneath it. Approximate the
            // descender as a fraction of the run's font size.
            let box_extent = match inline.vertical_align {
                crate::style::computed::VerticalAlign::Baseline
                | crate::style::computed::VerticalAlign::Sub
                | crate::style::computed::VerticalAlign::Super => {
                    inline.height + template.font_size * 0.22
                }
                _ => inline.height,
            };
            line_height = line_height.max(box_extent);
            current_runs.push(template);
            continue;
        }

        let word_width = estimate_word_width(
            &word,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        let space_width = estimate_word_width(
            " ",
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );

        let needed = if current_width > 0.0 && !preserve_spacing {
            space_width + word_width
        } else {
            word_width
        };

        let effective_max_width = line_max_width(lines.len());
        let overflows = current_width + needed > effective_max_width;

        if overflows && !preserve_spacing && options.overflow_wrap != OverflowWrap::Normal {
            let available_width = if current_width > 0.0 {
                effective_max_width - current_width - space_width
            } else {
                effective_max_width
            };
            if let Some((prefix, remainder)) = split_word_to_fit(
                &word,
                available_width,
                template.font_size,
                &template.font_family,
                template.bold,
                template.italic,
                fonts,
            ) {
                let prefix_text = if current_width > 0.0 {
                    format!(" {prefix}")
                } else {
                    prefix
                };
                line_height = line_height.max(run_line_height(&template));
                current_runs.push(TextRun {
                    text: prefix_text,
                    ..template.clone()
                });

                lines.push(TextLine {
                    runs: std::mem::take(&mut current_runs),
                    height: line_height,
                });
                current_width = 0.0;
                line_height = run_line_height(&template);
                queue.push_front((remainder, template, false));
                continue;
            }
        }

        if overflows && current_width > 0.0 {
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = run_line_height(&template);
        }

        // When transitioning between runs with different backgrounds,
        // emit the inter-word space as a separate unstyled run so the
        // background doesn't bleed from a highlighted span into plain text.
        //
        // Skip the injected inter-word space when the previous run already
        // ends in whitespace: a generated-content string like "Note: " keeps
        // its own trailing space (preserved spacing), so adding another would
        // double the gap. CSS collapses whitespace across inline boundaries.
        let prev_ends_ws = current_runs
            .last()
            .and_then(|r: &TextRun| r.text.chars().last())
            .is_some_and(char::is_whitespace);
        let needs_space = current_width > 0.0 && !preserve_spacing && !prev_ends_ws;
        let prev_bg = current_runs
            .last()
            .and_then(|r: &TextRun| r.background_color);
        let bg_changed = prev_bg != template.background_color;

        let text = if needs_space {
            if bg_changed && template.background_color.is_some() {
                // Emit space as separate unstyled run using the PREVIOUS
                // run's font so it matches the surrounding text metrics.
                let prev_run = current_runs.last().unwrap_or(&template);
                let space = " ".to_string();
                let sw = estimate_word_width(
                    &space,
                    prev_run.font_size,
                    &prev_run.font_family,
                    prev_run.bold,
                    prev_run.italic,
                    fonts,
                );
                current_width += sw;
                current_runs.push(TextRun {
                    text: space,
                    font_size: prev_run.font_size,
                    font_family: prev_run.font_family.clone(),
                    bold: prev_run.bold,
                    italic: prev_run.italic,
                    color: prev_run.color,
                    underline: false,
                    line_through: false,
                    overline: false,
                    link_url: None,
                    background_color: None,
                    padding: (0.0, 0.0),
                    border_radius: 0.0,
                    line_height_factor: prev_run.line_height_factor,
                    inline_box: None,
                });
                word
            } else {
                format!(" {word}")
            }
        } else {
            word
        };

        let w = estimate_word_width(
            &text,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        current_width += w;
        line_height = line_height.max(run_line_height(&template));

        current_runs.push(TextRun { text, ..template });
    }

    if !current_runs.is_empty() {
        lines.push(TextLine {
            runs: current_runs,
            height: line_height,
        });
    }

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
        template.italic,
        fonts,
    );

    // Check if the line actually overflows
    let line_width = estimate_word_width(
        &total_text,
        template.font_size,
        &template.font_family,
        template.bold,
        template.italic,
        fonts,
    );
    if line_width <= max_width {
        return;
    }

    // Truncate character by character until text + ellipsis fits
    let mut truncated = String::new();
    for ch in total_text.chars() {
        truncated.push(ch);
        let w = estimate_word_width(
            &truncated,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        if w + ellipsis_width > max_width {
            truncated.pop();
            break;
        }
    }
    truncated.push_str(ellipsis);

    lines[0] = TextLine {
        runs: vec![TextRun {
            text: truncated,
            ..template
        }],
        height: line.height,
    };

    // Remove any additional lines (shouldn't exist with nowrap, but just in case)
    lines.truncate(1);
}

// ---------------------------------------------------------------------------
// push_text_run_with_fallback
// ---------------------------------------------------------------------------

/// Push a text run, splitting it into standard-font and fallback-font segments
/// when the run uses a standard PDF font and contains characters outside
/// WinAnsiEncoding.
///
/// Characters that cannot be encoded in WinAnsi (CJK, Arabic, emoji, etc.) are
/// placed into separate runs that reference the `__unicode_fallback` custom font,
/// which is rendered through the CIDFontType2/Identity-H pipeline.
pub(crate) fn push_text_run_with_fallback(
    run: TextRun,
    runs: &mut Vec<TextRun>,
    fonts: &HashMap<String, TtfFont>,
) {
    let is_standard_font = matches!(
        run.font_family,
        FontFamily::Helvetica | FontFamily::TimesRoman | FontFamily::Courier
    );

    // If using a custom font or there's no fallback loaded, push as-is.
    if !is_standard_font || !fonts.contains_key(crate::system_fonts::UNICODE_FALLBACK_KEY) {
        runs.push(run);
        return;
    }

    // If everything is WinAnsi-encodable, no splitting needed.
    if crate::render::pdf::is_winansi_encodable(&run.text) {
        runs.push(run);
        return;
    }

    // Split text into contiguous segments by font category:
    // - WinAnsi: standard PDF font (Helvetica, etc.)
    // - Emoji: emoji fallback font (Apple Color Emoji, Noto Color Emoji)
    // - Unicode: unicode fallback font (Noto Sans CJK, etc.)
    let unicode_family = FontFamily::Custom(crate::system_fonts::UNICODE_FALLBACK_KEY.to_string());
    let has_emoji_font = fonts.contains_key(crate::system_fonts::EMOJI_FALLBACK_KEY);
    let emoji_family = FontFamily::Custom(crate::system_fonts::EMOJI_FALLBACK_KEY.to_string());

    #[derive(PartialEq, Clone, Copy)]
    enum CharCategory {
        WinAnsi,
        Emoji,
        Unicode,
    }

    let categorize = |ch: char| -> CharCategory {
        if crate::render::pdf::is_winansi_char(ch) {
            CharCategory::WinAnsi
        } else if has_emoji_font && crate::fonts::is_emoji_char(ch as u32) {
            CharCategory::Emoji
        } else {
            CharCategory::Unicode
        }
    };

    let family_for = |cat: CharCategory| -> FontFamily {
        match cat {
            CharCategory::WinAnsi => run.font_family.clone(),
            CharCategory::Emoji => emoji_family.clone(),
            CharCategory::Unicode => unicode_family.clone(),
        }
    };

    let mut current = String::new();
    let mut current_cat = CharCategory::WinAnsi;

    for ch in run.text.chars() {
        let cat = categorize(ch);
        if cat != current_cat && !current.is_empty() {
            runs.push(TextRun {
                text: std::mem::take(&mut current),
                font_family: family_for(current_cat),
                ..run.clone()
            });
        }
        current_cat = cat;
        current.push(ch);
    }

    if !current.is_empty() {
        runs.push(TextRun {
            text: current,
            font_family: family_for(current_cat),
            ..run
        });
    }
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
    counter_state: &CounterState,
) -> Option<InlineBox> {
    let has_explicit_width = style.width.is_some();
    let child_w = style.width.unwrap_or(0.0);
    let child_h = style.height.unwrap_or(0.0);

    // Content width used to wrap the inner text.
    let inner_width = if has_explicit_width {
        if style.box_sizing == BoxSizing::BorderBox {
            child_w - style.padding.left - style.padding.right - style.border.horizontal_width()
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
    collect_text_runs(
        &el.children,
        style,
        &mut runs,
        None,
        rules,
        fonts,
        ancestors,
        counter_state,
    );
    let lines = if runs.is_empty() {
        Vec::new()
    } else {
        wrap_text_runs(
            runs,
            TextWrapOptions::new(
                inner_width.max(1.0),
                style.font_size,
                resolved_line_height_factor(style, fonts),
                style.overflow_wrap,
            )
            .with_rtl(style.direction_rtl),
            fonts,
        )
    };

    let content_w = if has_explicit_width {
        if style.box_sizing == BoxSizing::BorderBox {
            (child_w - style.padding.left - style.padding.right - style.border.horizontal_width())
                .max(0.0)
        } else {
            child_w
        }
    } else {
        lines
            .iter()
            .map(|l| {
                l.runs
                    .iter()
                    .map(|r| crate::fonts::str_width(&r.text, r.font_size, &r.font_family, r.bold))
                    .sum::<f32>()
            })
            .fold(0.0f32, f32::max)
    };
    let total_w =
        content_w + style.padding.left + style.padding.right + style.border.horizontal_width();

    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let content_h = if child_h > 0.0 {
        if style.box_sizing == BoxSizing::BorderBox {
            (child_h - style.padding.top - style.padding.bottom - style.border.vertical_width())
                .max(0.0)
        } else {
            child_h
        }
    } else {
        text_height
    };
    let total_h =
        content_h + style.padding.top + style.padding.bottom + style.border.vertical_width();

    Some(InlineBox {
        width: total_w,
        height: total_h,
        margin_left: style.margin.left.max(0.0),
        margin_right: style.margin.right.max(0.0),
        background_color: style.background_color.map(|c| c.to_f32_rgba()),
        border: LayoutBorder::from_computed(&style.border),
        border_radius: style.border_radius,
        padding_top: style.padding.top,
        padding_left: style.padding.left,
        vertical_align: style.vertical_align,
        lines,
        image: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_text_runs(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
    counter_state: &CounterState,
) {
    collect_text_runs_inner(
        nodes,
        parent_style,
        runs,
        link_url,
        rules,
        fonts,
        false,
        ancestors,
        counter_state,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_text_runs_inner(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    inline_parent: bool,
    ancestors: &[AncestorInfo],
    counter_state: &CounterState,
) {
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap
    );

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    // In pre/pre-wrap: preserve newlines as \n runs for line breaking
                    text.clone()
                } else {
                    collapse_whitespace(text)
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
                    let (bg, pad, br) = if inline_parent && !preserve_ws {
                        (
                            parent_style.background_color.map(|c| c.to_f32_rgba()),
                            (parent_style.padding.left, parent_style.padding.top),
                            parent_style.border_radius,
                        )
                    } else {
                        (None, (0.0, 0.0), 0.0)
                    };
                    push_text_run_with_fallback(
                        TextRun {
                            text: processed,
                            font_size: parent_style.font_size,
                            bold: parent_style.font_weight == FontWeight::Bold,
                            italic: parent_style.font_style == FontStyle::Italic,
                            underline: parent_style.text_decoration_underline,
                            line_through: parent_style.text_decoration_line_through,
                            overline: parent_style.text_decoration_overline,
                            color: parent_style.color.to_f32_rgb(),
                            link_url: link_url.map(String::from),
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: bg,
                            padding: pad,
                            border_radius: br,
                            line_height_factor: resolved_line_height_factor(parent_style, fonts),
                            inline_box: None,
                        },
                        runs,
                        fonts,
                    );
                }
            }
            DomNode::Element(el) => {
                if super::engine::collects_as_inline_text(el.tag) || el.tag == HtmlTag::Br {
                    if el.tag == HtmlTag::Br {
                        runs.push(TextRun {
                            text: "\n".to_string(),
                            font_size: parent_style.font_size,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            overline: false,
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                            line_height_factor: resolved_line_height_factor(parent_style, fonts),
                            inline_box: None,
                        });
                    } else if el.attributes.contains_key("data-math") {
                        // Skip math elements — they are rendered as MathBlock
                        // by flatten_element, not as inline text runs.
                    } else {
                        let classes = el.class_list();
                        let selector_ctx = SelectorContext {
                            ancestors: ancestors.to_vec(),
                            child_index: 0,
                            sibling_count: nodes.len(),
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        };
                        let style = compute_style_with_context(
                            el.tag,
                            el.style_attr(),
                            parent_style,
                            rules,
                            el.tag_name(),
                            &classes,
                            el.id(),
                            &el.attributes,
                            &selector_ctx,
                        );
                        let url = if el.tag == HtmlTag::A {
                            el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                        } else {
                            link_url
                        };
                        let mut child_ancestors = ancestors.to_vec();
                        child_ancestors.push(AncestorInfo {
                            element: el,
                            child_index: 0,
                            sibling_count: nodes.len(),
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        });
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
                                resolved_line_height_factor(parent_style, fonts);
                            if let Some(boxed) = build_inline_box(
                                &style,
                                el,
                                rules,
                                fonts,
                                &child_ancestors,
                                counter_state,
                            ) {
                                runs.push(TextRun {
                                    text: String::new(),
                                    font_size: parent_style.font_size,
                                    bold: false,
                                    italic: false,
                                    underline: false,
                                    line_through: false,
                                    overline: false,
                                    color: parent_style.color.to_f32_rgb(),
                                    link_url: url.map(String::from),
                                    font_family: resolve_style_font_family(parent_style, fonts),
                                    background_color: None,
                                    padding: (0.0, 0.0),
                                    border_radius: 0.0,
                                    line_height_factor,
                                    inline_box: Some(Box::new(boxed)),
                                });
                            }
                            continue;
                        }
                        // Emit ::before / ::after generated content for inline
                        // elements (e.g. <span class="label">). These flow as
                        // inline text runs around the element's own children.
                        let before = crate::style::computed::compute_pseudo_element_style(
                            &style,
                            rules,
                            el.tag_name(),
                            &classes,
                            el.id(),
                            &el.attributes,
                            &selector_ctx,
                            crate::parser::css::PseudoElement::Before,
                        );
                        let after = crate::style::computed::compute_pseudo_element_style(
                            &style,
                            rules,
                            el.tag_name(),
                            &classes,
                            el.id(),
                            &el.attributes,
                            &selector_ctx,
                            crate::parser::css::PseudoElement::After,
                        );
                        super::helpers::append_pseudo_inline_run(
                            runs,
                            before.as_ref(),
                            el,
                            fonts,
                            counter_state,
                        );
                        collect_text_runs_inner(
                            &el.children,
                            &style,
                            runs,
                            url,
                            rules,
                            fonts,
                            true,
                            &child_ancestors,
                            counter_state,
                        );
                        super::helpers::append_pseudo_inline_run(
                            runs,
                            after.as_ref(),
                            el,
                            fonts,
                            counter_state,
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FlexTextRunCollector
// ---------------------------------------------------------------------------

pub(crate) struct FlexTextRunCollector<'a> {
    pub(crate) runs: &'a mut Vec<TextRun>,
    pub(crate) rules: &'a [CssRule],
    pub(crate) fonts: &'a HashMap<String, TtfFont>,
}

impl<'a> FlexTextRunCollector<'a> {
    pub(crate) fn collect(
        &mut self,
        nodes: &[DomNode],
        parent_style: &ComputedStyle,
        link_url: Option<&str>,
        text_padding: (f32, f32),
        ancestors: &[AncestorInfo],
    ) {
        let preserve_ws = matches!(
            parent_style.white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap
        );

        for node in nodes {
            match node {
                DomNode::Text(text) => {
                    let processed = if preserve_ws {
                        text.clone()
                    } else {
                        collapse_whitespace(text)
                    };
                    // Apply CSS text-transform
                    let processed = match parent_style.text_transform {
                        crate::style::computed::TextTransform::Uppercase => {
                            processed.to_uppercase()
                        }
                        crate::style::computed::TextTransform::Lowercase => {
                            processed.to_lowercase()
                        }
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
                        push_text_run_with_fallback(
                            TextRun {
                                text: processed,
                                font_size: parent_style.font_size,
                                bold: parent_style.font_weight == FontWeight::Bold,
                                italic: parent_style.font_style == FontStyle::Italic,
                                underline: parent_style.text_decoration_underline,
                                line_through: parent_style.text_decoration_line_through,
                                overline: parent_style.text_decoration_overline,
                                color: parent_style.color.to_f32_rgb(),
                                link_url: link_url.map(String::from),
                                font_family: resolve_style_font_family(parent_style, self.fonts),
                                background_color: parent_style
                                    .background_color
                                    .map(|c| c.to_f32_rgba()),
                                padding: text_padding,
                                border_radius: 0.0,
                                line_height_factor: resolved_line_height_factor(
                                    parent_style,
                                    self.fonts,
                                ),
                                inline_box: None,
                            },
                            self.runs,
                            self.fonts,
                        );
                    }
                }
                DomNode::Element(el) => {
                    let classes = el.class_list();
                    let selector_ctx = SelectorContext {
                        ancestors: ancestors.to_vec(),
                        child_index: 0,
                        sibling_count: nodes.len(),
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    };
                    let child_style = compute_style_with_context(
                        el.tag,
                        el.style_attr(),
                        parent_style,
                        self.rules,
                        el.tag_name(),
                        &classes,
                        el.id(),
                        &el.attributes,
                        &selector_ctx,
                    );

                    if child_style.display == Display::None {
                        continue;
                    }

                    let child_padding = if child_style.display == Display::Block
                        || child_style.background_color.is_some()
                        || child_style.border.has_any()
                        || child_style.border_radius > 0.0
                    {
                        (child_style.padding.left, child_style.padding.top)
                    } else {
                        text_padding
                    };
                    let child_link_url = if el.tag == HtmlTag::A {
                        el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                    } else {
                        link_url
                    };

                    if el.tag == HtmlTag::Br {
                        self.runs.push(TextRun {
                            text: "\n".to_string(),
                            font_size: parent_style.font_size,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            overline: false,
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, self.fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                            line_height_factor: resolved_line_height_factor(
                                parent_style,
                                self.fonts,
                            ),
                            inline_box: None,
                        });
                        continue;
                    }

                    let mut child_ancestors = ancestors.to_vec();
                    child_ancestors.push(AncestorInfo {
                        element: el,
                        child_index: 0,
                        sibling_count: nodes.len(),
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    });
                    self.collect(
                        &el.children,
                        &child_style,
                        child_link_url,
                        child_padding,
                        &child_ancestors,
                    );
                    if el.tag.is_block() && !self.runs.is_empty() {
                        self.runs.push(TextRun {
                            text: "\n".to_string(),
                            font_size: child_style.font_size,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            overline: false,
                            color: child_style.color.to_f32_rgb(),
                            link_url: child_link_url.map(String::from),
                            font_family: resolve_style_font_family(&child_style, self.fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                            line_height_factor: resolved_line_height_factor(
                                &child_style,
                                self.fonts,
                            ),
                            inline_box: None,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;

    fn plain_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_size: 16.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
        }
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
}
