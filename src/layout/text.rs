use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, HtmlTag};
use crate::parser::ttf::TtfFont;
// Re-export OverflowWrap so callers of TextWrapOptions::new can use it
// without a separate import.
pub(crate) use crate::style::computed::OverflowWrap;
use crate::style::computed::{
    ComputedStyle, Display, FontFamily, FontStyle, FontWeight, WhiteSpace,
    compute_style_with_context,
};
use std::collections::HashMap;

use super::engine::{TextLine, TextRun};

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
        }
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
    // Start with line height based on max font size in this line (typography alignment)
    let base_font_size = runs
        .iter()
        .map(|r| r.font_size)
        .fold(options.default_font_size, f32::max);
    let mut line_height = base_font_size * line_height_factor;

    // Apply BiDi reordering if the text contains RTL characters.
    // This reorders runs into visual order so RTL segments display correctly
    // in the left-to-right PDF rendering context.
    let full_text: String = runs.iter().map(|r| r.text.as_str()).collect();
    let runs = if crate::bidi::has_rtl_chars(&full_text) {
        crate::bidi::reorder_runs_bidi(&runs, false)
    } else {
        runs
    };

    // Concatenate all text then re-split by words, preserving run styles.
    // For text containing \n (white-space: pre), split on newlines first,
    // then split each segment by words.
    let mut styled_words: Vec<(String, TextRun, bool)> = Vec::new();
    for run in &runs {
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
                if segment.chars().next().is_some_and(char::is_whitespace)
                    || segment.chars().last().is_some_and(char::is_whitespace)
                    || segment.contains("  ")
                {
                    styled_words.push((segment.to_string(), run.clone(), true));
                } else {
                    for word in segment.split_whitespace() {
                        styled_words.push((word.to_string(), run.clone(), false));
                    }
                }
            }
        } else if has_preserved_spacing {
            styled_words.push((run.text.clone(), run.clone(), true));
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

    while let Some((word, template, preserve_spacing)) = queue.pop_front() {
        if word == "\n" {
            // Line break
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = template.font_size * line_height_factor;
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

        let overflows = current_width + needed > options.max_width;

        if overflows && !preserve_spacing && options.overflow_wrap != OverflowWrap::Normal {
            let available_width = if current_width > 0.0 {
                options.max_width - current_width - space_width
            } else {
                options.max_width
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
                line_height = line_height.max(template.font_size * line_height_factor);
                current_runs.push(TextRun {
                    text: prefix_text,
                    ..template.clone()
                });

                lines.push(TextLine {
                    runs: std::mem::take(&mut current_runs),
                    height: line_height,
                });
                current_width = 0.0;
                line_height = template.font_size * line_height_factor;
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
            line_height = template.font_size * line_height_factor;
        }

        // When transitioning between runs with different backgrounds,
        // emit the inter-word space as a separate unstyled run so the
        // background doesn't bleed from a highlighted span into plain text.
        let needs_space = current_width > 0.0 && !preserve_spacing;
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
                    link_url: None,
                    background_color: None,
                    padding: (0.0, 0.0),
                    border_radius: 0.0,
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
        line_height = line_height.max(template.font_size * line_height_factor);

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

pub(crate) fn collect_text_runs(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
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
                            color: parent_style.color.to_f32_rgb(),
                            link_url: link_url.map(String::from),
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: bg,
                            padding: pad,
                            border_radius: br,
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
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
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
                        });
                        collect_text_runs_inner(
                            &el.children,
                            &style,
                            runs,
                            url,
                            rules,
                            fonts,
                            true,
                            &child_ancestors,
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
                                color: parent_style.color.to_f32_rgb(),
                                link_url: link_url.map(String::from),
                                font_family: resolve_style_font_family(parent_style, self.fonts),
                                background_color: parent_style
                                    .background_color
                                    .map(|c| c.to_f32_rgba()),
                                padding: text_padding,
                                border_radius: 0.0,
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
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, self.fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                        });
                        continue;
                    }

                    let mut child_ancestors = ancestors.to_vec();
                    child_ancestors.push(AncestorInfo {
                        element: el,
                        child_index: 0,
                        sibling_count: nodes.len(),
                        preceding_siblings: Vec::new(),
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
                            color: child_style.color.to_f32_rgb(),
                            link_url: child_link_url.map(String::from),
                            font_family: resolve_style_font_family(&child_style, self.fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                        });
                    }
                }
            }
        }
    }
}
