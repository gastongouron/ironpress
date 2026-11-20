//! Bidirectional text reordering (UAX #9) for mixed LTR/RTL content.

use crate::layout::engine::TextRun;
use unicode_bidi::{BidiInfo, Level};

const OBJECT_REPLACEMENT_CHARACTER: char = '\u{fffc}';

/// Reorder text runs according to the Unicode Bidirectional Algorithm.
///
/// Takes a list of text runs in logical order and returns them split into
/// one run per level-run, arranged in visual left-to-right order. Each
/// resulting run's text is kept in LOGICAL order (not pre-reversed) so that
/// the downstream text shaper (harfrust, via `guess_segment_properties`)
/// can apply the correct RTL shaping itself. Pre-reversing here would cause
/// harfrust to reverse a second time, undoing the bidi reorder.
pub(crate) fn reorder_runs_bidi(
    runs: &[TextRun],
    paragraph_rtl: bool,
    bidi_override: bool,
) -> Vec<TextRun> {
    if runs.is_empty() {
        return Vec::new();
    }

    let mut full_text = String::new();
    for run in runs {
        if run.text.is_empty() && run.inline_box.is_some() {
            full_text.push(OBJECT_REPLACEMENT_CHARACTER);
        } else {
            full_text.push_str(&run.text);
        }
    }
    if full_text.is_empty() {
        return runs.to_vec();
    }

    // `unicode-bidi: bidi-override`: the characters' intrinsic bidi classes are
    // ignored and the content is laid out strictly in sequence according to
    // `direction` (css-writing-modes-4 §2.4). For `direction: rtl` this means
    // the entire visual order is the reverse of logical order; for `ltr` the
    // logical order is kept. The text inside each emitted run is reversed so the
    // shaper does not re-flip it (harfrust reverses RTL-shaped runs itself, but
    // these glyphs are forced, not naturally RTL, so we pre-reverse here).
    if bidi_override {
        if !paragraph_rtl {
            return runs.to_vec();
        }
        let starts = logical_starts(runs);
        let mut result: Vec<TextRun> = Vec::with_capacity(runs.len());
        for (index, run) in runs.iter().enumerate().rev() {
            if run.text.is_empty() && run.inline_box.is_some() {
                let mut object = run.clone();
                object.reverse_inline_edge_role();
                object.metadata.reading.logical_start = starts[index];
                result.push(object);
                continue;
            }
            if run.text.is_empty() {
                continue;
            }
            let mut reversed = TextRun {
                text: run.text.chars().rev().collect(),
                ..run.clone()
            };
            reversed.metadata.reading.logical_start = starts[index];
            reversed.metadata.reading.reversed = true;
            result.push(reversed);
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

    // Build per-char mapping. Atomic inline boxes participate in UAX #9 as an
    // object replacement character, but retain their empty authored text.
    let mut char_info: Vec<(char, usize, usize, bool)> = Vec::new();
    let mut byte_offset = 0;
    for (run_idx, run) in runs.iter().enumerate() {
        if run.text.is_empty() && run.inline_box.is_some() {
            char_info.push((OBJECT_REPLACEMENT_CHARACTER, byte_offset, run_idx, true));
            byte_offset += OBJECT_REPLACEMENT_CHARACTER.len_utf8();
            continue;
        }
        for ch in run.text.chars() {
            char_info.push((ch, byte_offset, run_idx, false));
            byte_offset += ch.len_utf8();
        }
    }

    let mut result: Vec<TextRun> = Vec::new();

    for byte_range in vis_ranges.iter() {
        // Each visual run's characters remain in *logical* order inside the
        // run — the shaper (harfrust) will flip RTL glyphs itself. Only the
        // *order of runs* is visual (left-to-right on the page), so we emit
        // runs in the order `visual_runs` gives them.
        // Find chars in this byte range (already in logical order).
        let segment_chars: Vec<(char, usize, usize, bool)> = char_info
            .iter()
            .filter(|(_, bo, _, _)| byte_range.contains(bo))
            .map(|(ch, bo, ri, object)| (*ch, *bo, *ri, *object))
            .collect();

        // Group consecutive chars by run index. Characters remain logical
        // inside each paint run, while paint runs at an odd bidi level reverse
        // as units into visual order.
        let range_is_rtl = vis_levels
            .get(byte_range.start)
            .is_some_and(|level| level.is_rtl());
        let mut range_runs = Vec::new();
        let mut current_text = String::new();
        let mut current_run_idx: Option<usize> = None;
        let mut current_start = 0;

        for (ch, start, run_idx, is_object) in &segment_chars {
            if *is_object {
                if let Some(prev_idx) = current_run_idx.take() {
                    if !current_text.is_empty() {
                        range_runs.push(segment(
                            &runs[prev_idx],
                            std::mem::take(&mut current_text),
                            current_start,
                        ));
                    }
                }
                let mut object = runs[*run_idx].clone();
                if range_is_rtl {
                    object.reverse_inline_edge_role();
                }
                object.metadata.reading.logical_start = *start;
                range_runs.push(object);
                continue;
            }
            if current_run_idx == Some(*run_idx) {
                current_text.push(*ch);
            } else {
                if let Some(prev_idx) = current_run_idx {
                    if !current_text.is_empty() {
                        range_runs.push(segment(
                            &runs[prev_idx],
                            std::mem::take(&mut current_text),
                            current_start,
                        ));
                    }
                }
                current_run_idx = Some(*run_idx);
                current_start = *start;
                current_text.push(*ch);
            }
        }

        if let Some(idx) = current_run_idx {
            if !current_text.is_empty() {
                range_runs.push(segment(
                    &runs[idx],
                    std::mem::take(&mut current_text),
                    current_start,
                ));
            }
        }
        if range_is_rtl {
            range_runs.reverse();
        }
        result.extend(range_runs);
    }

    if result.is_empty() {
        runs.to_vec()
    } else {
        move_first_visual_run_leading_spaces_to_end(&mut result);
        move_trailing_spaces_to_previous_visual_run(&mut result);
        result
    }
}

/// The piece of `run` holding `text`, which starts at byte `logical_start` of
/// the paragraph's logical text.
fn segment(run: &TextRun, text: String, logical_start: usize) -> TextRun {
    let mut segment = TextRun {
        text,
        ..run.clone()
    };
    segment.metadata.reading.logical_start = logical_start;
    segment
}

/// Byte offset of each run in the paragraph's logical text, an atomic inline
/// box counting as its object replacement character.
fn logical_starts(runs: &[TextRun]) -> Vec<usize> {
    let mut offset = 0;
    runs.iter()
        .map(|run| {
            let start = offset;
            offset += if run.text.is_empty() && run.inline_box.is_some() {
                OBJECT_REPLACEMENT_CHARACTER.len_utf8()
            } else {
                run.text.len()
            };
            start
        })
        .collect()
}

fn move_first_visual_run_leading_spaces_to_end(runs: &mut [TextRun]) {
    let Some(first) = runs.first_mut() else {
        return;
    };
    let count = first.text.chars().take_while(|ch| *ch == ' ').count();
    if count == 0 || count == first.text.chars().count() {
        return;
    }
    let mut rest = first.text.chars().skip(count).collect::<String>();
    for _ in 0..count {
        rest.push(' ');
    }
    first.text = rest;
}

fn move_trailing_spaces_to_previous_visual_run(runs: &mut [TextRun]) {
    for idx in 1..runs.len() {
        let trailing: String = runs[idx]
            .text
            .chars()
            .rev()
            .take_while(|ch| *ch == ' ')
            .collect();
        if trailing.is_empty() {
            continue;
        }
        let count = trailing.chars().count();
        for _ in 0..count {
            runs[idx].text.pop();
        }
        for _ in 0..count {
            runs[idx - 1].text.push(' ');
        }
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

pub(crate) fn first_strong_is_rtl(text: &str) -> bool {
    for ch in text.chars() {
        let c = ch as u32;
        if (0x0041..=0x005a).contains(&c)
            || (0x0061..=0x007a).contains(&c)
            || (0x00c0..=0x02af).contains(&c)
        {
            return false;
        }
        if (0x0600..=0x06FF).contains(&c)
            || (0x0750..=0x077F).contains(&c)
            || (0x08A0..=0x08FF).contains(&c)
            || (0xFB50..=0xFDFF).contains(&c)
            || (0xFE70..=0xFEFF).contains(&c)
            || (0x0590..=0x05FF).contains(&c)
            || (0xFB1D..=0xFB4F).contains(&c)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::{
        InlineBox, InlineDecoration, InlineDecorationId, InlineEdge, InlineEdgeSide,
        TextRunMetadata,
    };

    fn make_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn make_edge(side: InlineEdgeSide, width: f32) -> TextRun {
        TextRun {
            inline_box: Some(Box::new(InlineBox::advance_only(width))),
            metadata: TextRunMetadata {
                inline_edge: Some(InlineEdge {
                    side,
                    decoration: InlineDecoration {
                        id: InlineDecorationId::from_index(0),
                        background_color: None,
                        padding: crate::types::EdgeSizes::ZERO,
                        border_radii: crate::types::CornerRadii::ZERO,
                    },
                }),
                ..Default::default()
            },
            ..Default::default()
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
    fn rtl_reordering_preserves_and_reverses_inline_objects() {
        let runs = vec![
            make_edge(InlineEdgeSide::Opening, 20.0),
            make_run("שלום"),
            make_edge(InlineEdgeSide::Closing, 10.0),
        ];

        let result = reorder_runs_bidi(&runs, true, false);

        assert_eq!(result.len(), 3);
        assert!(result[0].is_opening_inline_edge());
        assert_eq!(result[0].atomic_inline_advance(), Some(10.0));
        assert_eq!(result[1].text, "שלום");
        assert!(!result[1].is_inline_edge());
        assert!(!result[2].is_opening_inline_edge());
        assert_eq!(result[2].atomic_inline_advance(), Some(20.0));
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

#[cfg(test)]
mod reading_order_tests {
    use super::*;

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            ..Default::default()
        }
    }

    /// UAX #9: runs of a right-to-left paragraph paint in visual order, and
    /// each keeps the byte offset of its text in the logical paragraph, so
    /// the logical order can be recovered.
    #[test]
    fn reordered_runs_keep_their_logical_start() {
        let visual = reorder_runs_bidi(&[run("אב"), run("גד")], true, false);
        let texts: Vec<&str> = visual.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, ["גד", "אב"]);
        let starts: Vec<usize> = visual
            .iter()
            .map(|run| run.metadata.reading.logical_start)
            .collect();
        assert_eq!(starts, [4, 0]);
        assert!(visual.iter().all(|run| !run.metadata.reading.reversed));
    }

    /// css-writing-modes-4 §2.4: under `bidi-override` with `direction: rtl`
    /// the characters are painted reversed; the run says so and reads its
    /// logical text back (ISO 32000-1 14.9.4).
    #[test]
    fn overridden_runs_read_their_logical_text() {
        let visual = reorder_runs_bidi(&[run("ab"), run("cd")], true, true);
        let texts: Vec<&str> = visual.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, ["dc", "ba"]);
        let starts: Vec<usize> = visual
            .iter()
            .map(|run| run.metadata.reading.logical_start)
            .collect();
        assert_eq!(starts, [2, 0]);
        assert_eq!(
            visual[0].metadata.actual_text(&visual[0].text).as_deref(),
            Some("cd")
        );
        assert_eq!(run("ab").metadata.actual_text("ab"), None);
    }
}
