//! Bidirectional text reordering (UAX #9) for mixed LTR/RTL content.

use crate::layout::engine::TextRun;
use unicode_bidi::{BidiInfo, Level};

/// Reorder text runs according to the Unicode Bidirectional Algorithm.
///
/// Takes a list of text runs in logical order and returns them split into
/// one run per level-run, arranged in visual left-to-right order. Each
/// resulting run's text is kept in LOGICAL order (not pre-reversed) so that
/// the downstream text shaper (rustybuzz, via `guess_segment_properties`)
/// can apply the correct RTL shaping itself. Pre-reversing here would cause
/// rustybuzz to reverse a second time, undoing the bidi reorder.
pub(crate) fn reorder_runs_bidi(
    runs: &[TextRun],
    paragraph_rtl: bool,
    bidi_override: bool,
) -> Vec<TextRun> {
    if runs.is_empty() {
        return Vec::new();
    }

    let full_text: String = runs.iter().map(|r| r.text.as_str()).collect();
    if full_text.is_empty() {
        return runs.to_vec();
    }

    // `unicode-bidi: bidi-override`: the characters' intrinsic bidi classes are
    // ignored and the content is laid out strictly in sequence according to
    // `direction` (css-writing-modes-4 §2.4). For `direction: rtl` this means
    // the entire visual order is the reverse of logical order; for `ltr` the
    // logical order is kept. The text inside each emitted run is reversed so the
    // shaper does not re-flip it (rustybuzz reverses RTL-shaped runs itself, but
    // these glyphs are forced, not naturally RTL, so we pre-reverse here).
    if bidi_override {
        if !paragraph_rtl {
            return runs.to_vec();
        }
        let mut result: Vec<TextRun> = Vec::with_capacity(runs.len());
        for run in runs.iter().rev() {
            if run.text.is_empty() {
                continue;
            }
            result.push(TextRun {
                text: run.text.chars().rev().collect(),
                ..run.clone()
            });
        }
        return if result.is_empty() {
            runs.to_vec()
        } else {
            result
        };
    }

    let default_level = if paragraph_rtl {
        Level::rtl()
    } else {
        Level::ltr()
    };
    let bidi_info = BidiInfo::new(&full_text, Some(default_level));

    if bidi_info.paragraphs.is_empty() {
        return runs.to_vec();
    }

    let para = &bidi_info.paragraphs[0];
    let line = para.range.clone();

    // Get visual runs: `vis_levels` is per-byte resolved levels; `vis_ranges`
    // are byte ranges already in visual (left-to-right) order. We look up a
    // run's level via its starting byte.
    let (vis_levels, vis_ranges) = bidi_info.visual_runs(para, line);

    // Check if purely LTR — return unchanged
    if vis_ranges.len() == 1
        && vis_ranges[0].start < vis_levels.len()
        && vis_levels[vis_ranges[0].start].is_ltr()
    {
        return runs.to_vec();
    }

    // Build per-char mapping: (char, byte_offset, run_index)
    let mut char_info: Vec<(char, usize, usize)> = Vec::new();
    let mut byte_offset = 0;
    for (run_idx, run) in runs.iter().enumerate() {
        for ch in run.text.chars() {
            char_info.push((ch, byte_offset, run_idx));
            byte_offset += ch.len_utf8();
        }
    }

    let mut result: Vec<TextRun> = Vec::new();

    for byte_range in vis_ranges.iter() {
        // Each visual run's characters remain in *logical* order inside the
        // run — the shaper (rustybuzz) will flip RTL glyphs itself. Only the
        // *order of runs* is visual (left-to-right on the page), so we emit
        // runs in the order `visual_runs` gives them.
        // Find chars in this byte range (already in logical order).
        let segment_chars: Vec<(char, usize)> = char_info
            .iter()
            .filter(|(_, bo, _)| byte_range.contains(bo))
            .map(|(ch, _, ri)| (*ch, *ri))
            .collect();

        // Group consecutive chars by run index and emit
        let mut current_text = String::new();
        let mut current_run_idx: Option<usize> = None;

        for (ch, run_idx) in &segment_chars {
            if current_run_idx == Some(*run_idx) {
                current_text.push(*ch);
            } else {
                if let Some(prev_idx) = current_run_idx {
                    if !current_text.is_empty() {
                        result.push(TextRun {
                            text: std::mem::take(&mut current_text),
                            ..runs[prev_idx].clone()
                        });
                    }
                }
                current_run_idx = Some(*run_idx);
                current_text.push(*ch);
            }
        }

        if let Some(idx) = current_run_idx {
            if !current_text.is_empty() {
                result.push(TextRun {
                    text: std::mem::take(&mut current_text),
                    ..runs[idx].clone()
                });
            }
        }
    }

    if result.is_empty() {
        runs.to_vec()
    } else {
        result
    }
}

/// Returns true if the text contains any RTL characters (Arabic, Hebrew, etc.)
pub(crate) fn has_rtl_chars(text: &str) -> bool {
    text.chars().any(|ch| {
        let c = ch as u32;
        (0x0600..=0x06FF).contains(&c)
            || (0x0750..=0x077F).contains(&c)
            || (0x08A0..=0x08FF).contains(&c)
            || (0xFB50..=0xFDFF).contains(&c)
            || (0xFE70..=0xFEFF).contains(&c)
            || (0x0590..=0x05FF).contains(&c)
            || (0xFB1D..=0xFB4F).contains(&c)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::computed::{FontFamily, VerticalAlign};

    fn make_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        }
    }

    #[test]
    fn pure_ltr_unchanged() {
        let runs = vec![make_run("Hello World")];
        let result = reorder_runs_bidi(&runs, false, false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "Hello World");
    }

    #[test]
    fn has_rtl_detects_arabic() {
        assert!(has_rtl_chars("مرحبا"));
        assert!(has_rtl_chars("שלום"));
        assert!(!has_rtl_chars("Hello World"));
        assert!(!has_rtl_chars("你好世界"));
    }

    #[test]
    fn mixed_ltr_rtl_reorders() {
        let runs = vec![make_run("Hello مرحبا World")];
        let result = reorder_runs_bidi(&runs, false, false);
        let combined: String = result.iter().map(|r| r.text.as_str()).collect();
        // Should contain all characters
        assert!(combined.contains("Hello"));
        assert!(combined.contains("World"));
        // Arabic chars should be present (possibly reversed)
        assert!(
            combined
                .chars()
                .any(|c| (0x0600..=0x06FF).contains(&(c as u32)))
        );
    }

    #[test]
    fn empty_runs() {
        let result = reorder_runs_bidi(&[], false, false);
        assert!(result.is_empty());
    }

    #[test]
    fn bidi_override_rtl_reverses_ltr_text() {
        // `unicode-bidi: bidi-override` + `direction: rtl` forces strictly
        // sequential RTL layout, so Latin "ABCDEF" displays as "FEDCBA".
        let runs = vec![make_run("ABCDEF")];
        let result = reorder_runs_bidi(&runs, true, true);
        let combined: String = result.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(combined, "FEDCBA");
    }

    #[test]
    fn bidi_override_ltr_keeps_order() {
        let runs = vec![make_run("ABCDEF")];
        let result = reorder_runs_bidi(&runs, false, true);
        let combined: String = result.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(combined, "ABCDEF");
    }
}
