use crate::parser::css::{
    parse_inline_style, parse_length, selector_matches_with_context, specificity, AncestorInfo,
    CssRule, CssValue, PseudoElement, SelectorContext,
};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    compute_pseudo_element_style, compute_style_with_context, AlignContent, AlignItems, BoxSizing,
    ComputedStyle, ContentItem, Display, FontWeight, GridAlign, GridLine, GridTrack,
    JustifyContent, Position, TextAlign, VerticalAlign, Visibility, WhiteSpace,
};

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{flatten_element, BackgroundFields, LayoutBorder, LayoutElement};
use super::table::{GridInset, TableCell};
use super::text::{
    estimate_word_width, resolved_line_height_factor, wrap_text_runs, FlexTextRunCollector,
    TextWrapOptions,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum TrackBreadth {
    Fixed(f32),
    Percent(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RuntimeTrack {
    Fixed(f32),
    Percent(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    FitContent(f32),
    Minmax(TrackBreadth, TrackBreadth),
}

#[derive(Debug, Clone)]
struct RuntimeTrackList {
    tracks: Vec<RuntimeTrack>,
    auto_fit: Vec<bool>,
    line_names: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
struct SubgridAxis {
    tracks: Vec<f32>,
    gap: f32,
    line_names: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
struct SubgridContext {
    columns: Option<SubgridAxis>,
    rows: Option<SubgridAxis>,
}

impl RuntimeTrack {
    fn from_grid_track(track: &GridTrack) -> Self {
        match track {
            GridTrack::Fixed(v) => Self::Fixed(*v),
            GridTrack::Percent(p) => Self::Percent(*p),
            GridTrack::Fr(v) => Self::Fr(*v),
            GridTrack::Auto => Self::Auto,
            GridTrack::Minmax(min, max) => Self::Minmax(TrackBreadth::Fixed(*min), track_max(*max)),
        }
    }
}

fn track_max(max: f32) -> TrackBreadth {
    if max >= f32::MAX / 2.0 {
        TrackBreadth::Fr(1.0)
    } else {
        TrackBreadth::Fixed(max)
    }
}

/// Resolve grid column widths from track definitions.
///
/// CSS Grid track-sizing semantics:
/// - Fixed(v): uses `v` directly.
/// - Auto: sized to the column's max-content intrinsic width (passed in via
///   `auto_intrinsic_widths`, indexed by track). When the sum of fixed + auto
///   exceeds the available space, auto columns shrink proportionally.
/// - Fr(v) / Minmax(min, max): flexible tracks. The space left after the
///   fixed/percent/auto tracks is divided among them by the CSS Grid
///   "find the size of an fr" algorithm — each flexible track resolves to
///   `flex_size × flex_factor`, floored at its base (`0` for a bare `fr`, the
///   `min` for a `minmax`) and capped at its `max`, with `flex_size` found by
///   iteratively freezing clamped tracks. Equal `fr` peers therefore resolve
///   to equal widths even when their `minmax` minimums differ. If no flexible
///   tracks exist and slack remains, Auto columns absorb it (so `auto auto`
///   fills the row like Chrome does).
///
/// `auto_intrinsic_widths` must have length == tracks.len(); the value at
/// each Auto track index is that column's max-content width. Non-Auto
/// entries are ignored.
fn resolve_grid_columns(
    tracks: &[RuntimeTrack],
    available_width: f32,
    gap: f32,
    min_intrinsic_widths: &[f32],
    max_intrinsic_widths: &[f32],
) -> Vec<f32> {
    if tracks.is_empty() {
        return vec![available_width];
    }

    let num_gaps = if tracks.len() > 1 {
        (tracks.len() - 1) as f32 * gap
    } else {
        0.0
    };
    let space = (available_width - num_gaps).max(0.0);

    let min_intrinsic = |i: usize| -> f32 { min_intrinsic_widths.get(i).copied().unwrap_or(0.0) };
    let max_intrinsic = |i: usize| -> f32 { max_intrinsic_widths.get(i).copied().unwrap_or(0.0) };
    let breadth = |b: TrackBreadth, i: usize, percent_basis: f32| -> f32 {
        match b {
            TrackBreadth::Fixed(v) => v,
            TrackBreadth::Percent(p) => p * percent_basis,
            TrackBreadth::Fr(_) => 0.0,
            TrackBreadth::Auto | TrackBreadth::MinContent => min_intrinsic(i),
            TrackBreadth::MaxContent => max_intrinsic(i),
        }
    };
    let max_breadth = |b: TrackBreadth, i: usize, percent_basis: f32| -> f32 {
        match b {
            TrackBreadth::Fixed(v) => v,
            TrackBreadth::Percent(p) => p * percent_basis,
            TrackBreadth::Fr(_) => f32::MAX,
            TrackBreadth::Auto | TrackBreadth::MaxContent => max_intrinsic(i),
            TrackBreadth::MinContent => min_intrinsic(i),
        }
    };

    // First pass: bucket totals.
    let mut fixed_total: f32 = 0.0;
    let mut fr_total: f32 = 0.0;
    let mut auto_total: f32 = 0.0;
    let mut auto_count: usize = 0;
    let mut flex_count: usize = 0;

    for (i, track) in tracks.iter().enumerate() {
        match track {
            RuntimeTrack::Fixed(v) => fixed_total += *v,
            RuntimeTrack::Percent(p) => fixed_total += *p * space,
            RuntimeTrack::Fr(v) => fr_total += *v,
            RuntimeTrack::Auto => {
                auto_total += max_intrinsic(i);
                auto_count += 1;
            }
            RuntimeTrack::MinContent => fixed_total += min_intrinsic(i),
            RuntimeTrack::MaxContent => fixed_total += max_intrinsic(i),
            RuntimeTrack::FitContent(limit) => {
                fixed_total += max_intrinsic(i).min(*limit).max(min_intrinsic(i));
            }
            RuntimeTrack::Minmax(min, max) => {
                if matches!(max, TrackBreadth::Fr(_)) {
                    flex_count += 1;
                } else if matches!(max, TrackBreadth::MaxContent | TrackBreadth::Auto) {
                    fixed_total += max_breadth(*max, i, space).max(breadth(*min, i, space));
                } else {
                    flex_count += 1;
                }
            }
        }
    }

    let after_fixed = (space - fixed_total).max(0.0);
    let has_fr = fr_total + flex_count as f32 > 0.0;

    if has_fr {
        // Flexible-track regime (`fr` / `minmax(min, ...fr)` present). Auto
        // tracks size to their intrinsic max-content width; the rest of the
        // space is distributed among the flexible tracks by the CSS Grid
        // "find the size of an fr" algorithm (§12.7): every flexible track is
        // sized to `flex_size × flex_factor`, but no smaller than its base
        // minimum (0 for a bare `fr`, the `min` for a `minmax`) and no larger
        // than its `max` cap. `flex_size` is found by iteratively freezing
        // tracks whose floor/ceiling clamps them, then re-dividing the
        // remaining space among the still-flexible tracks. This makes equal
        // `1fr` peers resolve to equal widths even when their minimums differ
        // (e.g. `minmax(80px,1fr) minmax(120px,1fr)` → two equal tracks),
        // matching Chrome — unlike the old `min + share` formula which inflated
        // the larger-min track.
        let space_for_flex = (after_fixed - auto_total).max(0.0);

        // Per flexible track: (flex_factor, base_min, max_cap).
        struct Flex {
            factor: f32,
            base: f32,
            cap: f32,
        }
        let flex: Vec<Option<Flex>> = tracks
            .iter()
            .enumerate()
            .map(|(i, track)| match track {
                RuntimeTrack::Fr(v) => Some(Flex {
                    factor: *v,
                    base: 0.0,
                    cap: f32::MAX,
                }),
                RuntimeTrack::Minmax(min, max)
                    if !matches!(
                        max,
                        TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent
                    ) =>
                {
                    let factor = match max {
                        TrackBreadth::Fr(v) => *v,
                        _ => 1.0,
                    };
                    Some(Flex {
                        factor,
                        base: breadth(*min, i, space),
                        cap: max_breadth(*max, i, space),
                    })
                }
                _ => None,
            })
            .collect();

        // Iteratively resolve the shared flex size, freezing any track that
        // its base (floor) or cap (ceiling) pins, then re-dividing.
        let mut frozen = vec![false; tracks.len()];
        let mut resolved = vec![0.0_f32; tracks.len()];
        loop {
            let mut remaining = space_for_flex;
            let mut active_factor = 0.0_f32;
            for (i, f) in flex.iter().enumerate() {
                let Some(f) = f else { continue };
                if frozen[i] {
                    remaining -= resolved[i];
                } else {
                    active_factor += f.factor;
                }
            }
            remaining = remaining.max(0.0);
            if active_factor <= 0.0 {
                break;
            }
            let flex_size = remaining / active_factor;
            // Freeze the first track pinned below its base or above its cap;
            // restart so the freed/consumed space redistributes correctly.
            let mut changed = false;
            for (i, f) in flex.iter().enumerate() {
                let Some(f) = f else { continue };
                if frozen[i] {
                    continue;
                }
                let want = flex_size * f.factor;
                if want < f.base {
                    resolved[i] = f.base;
                    frozen[i] = true;
                    changed = true;
                    break;
                }
                if want > f.cap {
                    resolved[i] = f.cap;
                    frozen[i] = true;
                    changed = true;
                    break;
                }
            }
            if !changed {
                for (i, f) in flex.iter().enumerate() {
                    if let Some(f) = f {
                        if !frozen[i] {
                            resolved[i] = flex_size * f.factor;
                        }
                    }
                }
                break;
            }
        }

        let auto_shrink_scale = if auto_total > after_fixed && auto_total > 0.0 {
            after_fixed / auto_total
        } else {
            1.0
        };

        return tracks
            .iter()
            .enumerate()
            .map(|(i, track)| match track {
                RuntimeTrack::Fixed(v) => *v,
                RuntimeTrack::Percent(p) => *p * space,
                RuntimeTrack::Fr(_) | RuntimeTrack::Minmax(_, TrackBreadth::Fr(_)) => resolved[i],
                RuntimeTrack::Minmax(min, max) => {
                    max_breadth(*max, i, space).max(breadth(*min, i, space))
                }
                RuntimeTrack::Auto => max_intrinsic(i) * auto_shrink_scale,
                RuntimeTrack::MinContent => min_intrinsic(i),
                RuntimeTrack::MaxContent => max_intrinsic(i),
                RuntimeTrack::FitContent(limit) => {
                    max_intrinsic(i).min(*limit).max(min_intrinsic(i))
                }
            })
            .collect();
    }

    // No flexible tracks: auto tracks take their intrinsic width, then split
    // the remaining space EQUALLY among themselves (additive), matching
    // Chrome's track-sizing for `auto auto` layouts.
    let (auto_extra, auto_shrink_scale) = if auto_count > 0 {
        let slack = after_fixed - auto_total;
        if slack >= 0.0 {
            (slack / auto_count as f32, 1.0)
        } else {
            // Overflow — shrink auto tracks proportionally so the row fits.
            let scale = if auto_total > 0.0 {
                after_fixed / auto_total
            } else {
                0.0
            };
            (0.0, scale)
        }
    } else {
        (0.0, 1.0)
    };

    tracks
        .iter()
        .enumerate()
        .map(|(i, track)| match track {
            RuntimeTrack::Fixed(v) => *v,
            RuntimeTrack::Percent(p) => *p * space,
            RuntimeTrack::Fr(_) => 0.0,
            RuntimeTrack::Auto => max_intrinsic(i) * auto_shrink_scale + auto_extra,
            RuntimeTrack::MinContent => min_intrinsic(i),
            RuntimeTrack::MaxContent => max_intrinsic(i),
            RuntimeTrack::FitContent(limit) => max_intrinsic(i).min(*limit).max(min_intrinsic(i)),
            RuntimeTrack::Minmax(min, max) => {
                max_breadth(*max, i, space).max(breadth(*min, i, space))
            }
        })
        .collect()
}

/// Resolve a row track to a fixed height in points, if it is a definite size.
/// `fr`/`auto`/`minmax` rows return `None` (they fall back to auto sizing).
fn grid_track_fixed_height(track: &RuntimeTrack, percent_basis: Option<f32>) -> Option<f32> {
    match track {
        RuntimeTrack::Fixed(v) => Some(*v),
        RuntimeTrack::Percent(p) => percent_basis.map(|basis| *p * basis),
        RuntimeTrack::Minmax(TrackBreadth::Fixed(min), _) => Some(*min),
        _ => None,
    }
}

fn fixed_track_pattern_from_value(value: &CssValue) -> Vec<f32> {
    match value {
        CssValue::Length(v) => vec![*v],
        CssValue::Keyword(raw) => raw
            .split_whitespace()
            .filter_map(|token| match parse_length(token) {
                Some(CssValue::Length(v)) => Some(v),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn css_value_to_track_list_text(value: &CssValue) -> Option<String> {
    match value {
        CssValue::Keyword(raw) => Some(raw.clone()),
        CssValue::Length(v) => Some(format!("{v}pt")),
        CssValue::Percentage(v) => Some(format!("{v}%")),
        _ => None,
    }
}

fn winning_grid_track_declaration(
    el: &ElementNode,
    style_attr: Option<&str>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    property: &str,
) -> Option<String> {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let mut matched: Vec<(u32, usize, &CssRule)> = Vec::new();
    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &classes,
            el.id(),
            &el.attributes,
            &selector_ctx,
        ) {
            matched.push((specificity(&rule.selector), source_idx, rule));
        }
    }
    matched.sort_by_key(|(spec, source_idx, _)| (*spec, *source_idx));

    let mut normal = None;
    let mut important = None;
    for (_, _, rule) in matched {
        if let Some(value) = rule.declarations.get(property) {
            if rule.declarations.is_important(property) {
                important = css_value_to_track_list_text(value);
            } else {
                normal = css_value_to_track_list_text(value);
            }
        }
    }

    if let Some(inline) = style_attr.map(parse_inline_style) {
        if let Some(value) = inline.get(property) {
            if inline.is_important(property) {
                important = css_value_to_track_list_text(value);
            } else {
                normal = css_value_to_track_list_text(value);
            }
        }
    }

    important.or(normal)
}

fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            _ if ch == separator && paren == 0 && bracket == 0 => {
                parts.push(input[start..idx].trim().to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim().to_string());
    parts
}

fn consume_track_token(input: &str) -> (&str, &str) {
    let mut paren = 0usize;
    let mut bracket = 0usize;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            _ if ch.is_whitespace() && paren == 0 && bracket == 0 => {
                return input.split_at(idx);
            }
            _ => {}
        }
    }
    (input, "")
}

fn parse_track_length(token: &str) -> Option<f32> {
    match parse_length(token.trim()) {
        Some(CssValue::Length(v)) => Some(v),
        Some(CssValue::Number(v)) => Some(v),
        _ => token.trim().parse::<f32>().ok(),
    }
}

fn parse_track_breadth(token: &str) -> Option<TrackBreadth> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("auto") {
        Some(TrackBreadth::Auto)
    } else if token.eq_ignore_ascii_case("min-content") {
        Some(TrackBreadth::MinContent)
    } else if token.eq_ignore_ascii_case("max-content") {
        Some(TrackBreadth::MaxContent)
    } else if let Some(n) = token.strip_suffix("fr") {
        n.trim().parse::<f32>().ok().map(TrackBreadth::Fr)
    } else if let Some(n) = token.strip_suffix('%') {
        n.trim()
            .parse::<f32>()
            .ok()
            .map(|v| TrackBreadth::Percent(v / 100.0))
    } else {
        parse_track_length(token).map(TrackBreadth::Fixed)
    }
}

fn parse_runtime_track(token: &str) -> Option<RuntimeTrack> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("auto") {
        Some(RuntimeTrack::Auto)
    } else if token.eq_ignore_ascii_case("min-content") {
        Some(RuntimeTrack::MinContent)
    } else if token.eq_ignore_ascii_case("max-content") {
        Some(RuntimeTrack::MaxContent)
    } else if let Some(n) = token.strip_suffix("fr") {
        n.trim().parse::<f32>().ok().map(RuntimeTrack::Fr)
    } else if let Some(n) = token.strip_suffix('%') {
        n.trim()
            .parse::<f32>()
            .ok()
            .map(|v| RuntimeTrack::Percent(v / 100.0))
    } else if let Some(inner) = token
        .strip_prefix("fit-content(")
        .and_then(|s| s.strip_suffix(')'))
    {
        parse_track_length(inner).map(RuntimeTrack::FitContent)
    } else if let Some(inner) = token
        .strip_prefix("minmax(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts = split_top_level(inner, ',');
        if parts.len() == 2 {
            let min = parse_track_breadth(&parts[0])?;
            let max = parse_track_breadth(&parts[1])?;
            Some(RuntimeTrack::Minmax(min, max))
        } else {
            None
        }
    } else {
        parse_track_length(token).map(RuntimeTrack::Fixed)
    }
}

fn track_min_for_auto_repeat(track: RuntimeTrack) -> f32 {
    match track {
        RuntimeTrack::Fixed(v) => v,
        RuntimeTrack::Percent(_) => 0.0,
        RuntimeTrack::Fr(_) | RuntimeTrack::Auto => 0.0,
        RuntimeTrack::MinContent | RuntimeTrack::MaxContent => 0.0,
        RuntimeTrack::FitContent(limit) => limit,
        RuntimeTrack::Minmax(min, _) => match min {
            TrackBreadth::Fixed(v) => v,
            TrackBreadth::Percent(_) => 0.0,
            TrackBreadth::Fr(_) | TrackBreadth::Auto => 0.0,
            TrackBreadth::MinContent | TrackBreadth::MaxContent => 0.0,
        },
    }
}

fn auto_repeat_count(pattern: &[RuntimeTrack], available_width: f32, gap: f32) -> usize {
    if pattern.is_empty() {
        return 1;
    }
    let pattern_width = pattern
        .iter()
        .map(|t| track_min_for_auto_repeat(*t))
        .sum::<f32>()
        + gap * pattern.len().saturating_sub(1) as f32;
    let repeat_stride = pattern_width + gap;
    if repeat_stride <= 0.0 {
        1
    } else {
        ((available_width + gap) / repeat_stride).floor().max(1.0) as usize
    }
}

fn parse_runtime_track_list(value: &str, available_width: f32, gap: f32) -> RuntimeTrackList {
    let mut tracks = Vec::new();
    let mut auto_fit = Vec::new();
    let mut line_names = vec![Vec::new()];
    let mut remaining = value.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        while remaining.starts_with('[') {
            let Some(close) = remaining.find(']') else {
                break;
            };
            if let Some(slot) = line_names.last_mut() {
                slot.extend(
                    remaining[1..close]
                        .split_whitespace()
                        .map(ToString::to_string),
                );
            }
            remaining = remaining[close + 1..].trim_start();
        }
        if remaining.is_empty() {
            break;
        }

        let (token, rest) = consume_track_token(remaining);
        if let Some(inner) = token
            .strip_prefix("repeat(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let parts = split_top_level(inner, ',');
            if parts.len() == 2 {
                let count_token = parts[0].trim();
                let pattern = parse_runtime_track_list(&parts[1], available_width, gap);
                let count = if count_token.eq_ignore_ascii_case("auto-fill")
                    || count_token.eq_ignore_ascii_case("auto-fit")
                {
                    auto_repeat_count(&pattern.tracks, available_width, gap)
                } else {
                    count_token.parse::<usize>().unwrap_or(1)
                };
                let is_auto_fit = count_token.eq_ignore_ascii_case("auto-fit");
                for _ in 0..count {
                    for track in &pattern.tracks {
                        tracks.push(*track);
                        auto_fit.push(is_auto_fit);
                        line_names.push(Vec::new());
                    }
                }
            }
        } else if let Some(track) = parse_runtime_track(token) {
            tracks.push(track);
            auto_fit.push(false);
            line_names.push(Vec::new());
        }
        remaining = rest;
    }

    RuntimeTrackList {
        tracks,
        auto_fit,
        line_names,
    }
}

fn subgrid_track_declaration(
    el: &ElementNode,
    style_attr: Option<&str>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    property: &str,
) -> Option<String> {
    winning_grid_track_declaration(el, style_attr, rules, ancestors, property).and_then(|raw| {
        raw.trim()
            .to_ascii_lowercase()
            .starts_with("subgrid")
            .then_some(raw)
    })
}

fn parse_subgrid_added_line_names(raw: &str, line_count: usize) -> Vec<Vec<String>> {
    let mut names = vec![Vec::new(); line_count];
    let Some(mut remaining) = raw.trim().strip_prefix("subgrid") else {
        return names;
    };
    let mut line = 0usize;
    while !remaining.trim().is_empty() && line < line_count {
        remaining = remaining.trim_start();
        if !remaining.starts_with('[') {
            break;
        }
        let Some(close) = remaining.find(']') else {
            break;
        };
        names[line].extend(
            remaining[1..close]
                .split_whitespace()
                .map(ToString::to_string),
        );
        remaining = &remaining[close + 1..];
        line += 1;
    }
    names
}

fn subgrid_line_names(
    parent: &[Vec<String>],
    start: usize,
    span: usize,
    raw: &str,
) -> Vec<Vec<String>> {
    let line_count = span.saturating_add(1);
    let mut names = vec![Vec::new(); line_count];
    for (i, slot) in names.iter_mut().enumerate() {
        if let Some(parent_names) = parent.get(start + i) {
            slot.extend(parent_names.iter().cloned());
        }
    }
    let added = parse_subgrid_added_line_names(raw, line_count);
    for (slot, extra) in names.iter_mut().zip(added) {
        slot.extend(extra);
    }
    names
}

fn merge_line_name_lists(base: &[Vec<String>], extra: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut merged = vec![Vec::new(); base.len().max(extra.len())];
    for (i, names) in base.iter().enumerate() {
        merged[i].extend(names.iter().cloned());
    }
    for (i, names) in extra.iter().enumerate() {
        merged[i].extend(names.iter().cloned());
    }
    merged
}

#[allow(clippy::too_many_arguments)]
fn runtime_tracks_for_property(
    el: &ElementNode,
    style_attr: Option<&str>,
    style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    property: &str,
    available_width: f32,
    gap: f32,
) -> RuntimeTrackList {
    if let Some(raw) = winning_grid_track_declaration(el, style_attr, rules, ancestors, property) {
        let parsed = parse_runtime_track_list(&raw, available_width, gap);
        let computed_count = if property == "grid-template-rows" {
            style.grid_template_rows.len()
        } else {
            style.grid_template_columns.len()
        };
        if !parsed.tracks.is_empty() && parsed.tracks.len() >= computed_count {
            return parsed;
        }
    }
    let tracks: Vec<RuntimeTrack> = if property == "grid-template-rows" {
        style
            .grid_template_rows
            .iter()
            .map(RuntimeTrack::from_grid_track)
            .collect()
    } else {
        style
            .grid_template_columns
            .iter()
            .map(RuntimeTrack::from_grid_track)
            .collect()
    };
    let auto_fit = vec![false; tracks.len()];
    RuntimeTrackList {
        tracks,
        auto_fit,
        line_names: Vec::new(),
    }
}

fn matched_grid_track_pattern(
    el: &ElementNode,
    style_attr: Option<&str>,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    property: &str,
) -> Vec<f32> {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let mut matched: Vec<(u32, usize, &CssRule)> = Vec::new();
    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &classes,
            el.id(),
            &el.attributes,
            &selector_ctx,
        ) {
            matched.push((specificity(&rule.selector), source_idx, rule));
        }
    }
    matched.sort_by_key(|(spec, source_idx, _)| (*spec, *source_idx));

    let mut normal = Vec::new();
    let mut important = Vec::new();
    for (_, _, rule) in matched {
        if let Some(value) = rule.declarations.get(property) {
            if rule.declarations.is_important(property) {
                important = fixed_track_pattern_from_value(value);
            } else {
                normal = fixed_track_pattern_from_value(value);
            }
        }
    }

    if let Some(inline) = style_attr.map(parse_inline_style) {
        if let Some(value) = inline.get(property) {
            if inline.is_important(property) {
                important = fixed_track_pattern_from_value(value);
            } else {
                normal = fixed_track_pattern_from_value(value);
            }
        }
    }

    if !important.is_empty() {
        important
    } else if !normal.is_empty() {
        normal
    } else if property == "grid-auto-rows" {
        if !parent_style.grid_auto_rows_pattern.is_empty() {
            parent_style.grid_auto_rows_pattern.clone()
        } else {
            parent_style.grid_auto_rows.into_iter().collect()
        }
    } else {
        Vec::new()
    }
}

fn matched_display_contents(
    el: &ElementNode,
    style_attr: Option<&str>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
) -> bool {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let mut matched: Vec<(u32, usize, &CssRule)> = Vec::new();
    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &classes,
            el.id(),
            &el.attributes,
            &selector_ctx,
        ) {
            matched.push((specificity(&rule.selector), source_idx, rule));
        }
    }
    matched.sort_by_key(|(spec, source_idx, _)| (*spec, *source_idx));

    let mut normal = false;
    let mut important = false;
    for (_, _, rule) in matched {
        if let Some(CssValue::Keyword(value)) = rule.declarations.get("display") {
            let is_contents = value.eq_ignore_ascii_case("contents");
            if rule.declarations.is_important("display") {
                important = is_contents;
            } else {
                normal = is_contents;
            }
        }
    }
    if let Some(inline) = style_attr.map(parse_inline_style) {
        if let Some(CssValue::Keyword(value)) = inline.get("display") {
            let is_contents = value.eq_ignore_ascii_case("contents");
            if inline.is_important("display") {
                important = is_contents;
            } else {
                normal = is_contents;
            }
        }
    }

    important || normal
}

fn anonymous_grid_item_style(parent: &ComputedStyle) -> ComputedStyle {
    let mut style = parent.clone();
    style.display = Display::Block;
    style.margin = Default::default();
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.margin_top_auto = false;
    style.margin_bottom_auto = false;
    style.padding = Default::default();
    style.border = Default::default();
    style.background_color = None;
    style.width = None;
    style.height = None;
    style.position = Position::Static;
    style.order = 0;
    style.grid_column_start = GridLine::Auto;
    style.grid_column_end = GridLine::Auto;
    style.grid_row_start = GridLine::Auto;
    style.grid_row_end = GridLine::Auto;
    style.grid_area_name = None;
    style.grid_justify_self = None;
    style.grid_align_self = None;
    style
}

fn pseudo_element_node(style: &ComputedStyle) -> ElementNode {
    let text = style
        .content
        .iter()
        .filter_map(|item| match item {
            ContentItem::String(value) => Some(value.as_str()),
            _ => None,
        })
        .collect::<String>();
    let children = if text.is_empty() {
        Vec::new()
    } else {
        vec![DomNode::Text(text)]
    };
    ElementNode {
        tag: crate::parser::dom::HtmlTag::Span,
        raw_tag_name: "span".to_string(),
        attributes: Default::default(),
        children,
    }
}

fn shift_absolute_offsets(elements: &mut [LayoutElement], delta_x: f32, delta_y: f32) {
    for element in elements {
        match element {
            LayoutElement::TextBlock {
                position,
                offset_left,
                offset_top,
                ..
            }
            | LayoutElement::Container {
                position,
                offset_left,
                offset_top,
                ..
            } if *position == Position::Absolute => {
                *offset_left += delta_x;
                *offset_top += delta_y;
            }
            LayoutElement::Container { children, .. } => {
                shift_absolute_offsets(children, delta_x, delta_y);
            }
            _ => {}
        }
    }
}

fn shift_nested_flow_up(elements: &mut [LayoutElement], amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let Some(first) = elements.first_mut() else {
        return;
    };
    match first {
        LayoutElement::TextBlock { margin_top, .. }
        | LayoutElement::Container { margin_top, .. }
        | LayoutElement::Image { margin_top, .. }
        | LayoutElement::Svg { margin_top, .. }
        | LayoutElement::FlexRow { margin_top, .. }
        | LayoutElement::TableRow { margin_top, .. }
        | LayoutElement::GridRow { margin_top, .. }
        | LayoutElement::HorizontalRule { margin_top, .. }
        | LayoutElement::ProgressBar { margin_top, .. }
        | LayoutElement::MathBlock { margin_top, .. } => {
            *margin_top -= amount;
        }
        _ => {}
    }
}

fn distribute_tracks(
    sizes: &[f32],
    base_gap: f32,
    available: f32,
    justify: JustifyContent,
) -> (f32, f32) {
    let track_total = sizes.iter().sum::<f32>();
    let gap_count = sizes.len().saturating_sub(1) as f32;
    let natural = track_total + base_gap * gap_count;
    let free = (available - natural).max(0.0);
    match justify {
        JustifyContent::FlexEnd => (free, base_gap),
        JustifyContent::Center => (free / 2.0, base_gap),
        JustifyContent::SpaceBetween if sizes.len() > 1 => (0.0, base_gap + free / gap_count),
        JustifyContent::SpaceAround if !sizes.is_empty() => {
            let extra = free / sizes.len() as f32;
            (extra / 2.0, base_gap + extra)
        }
        JustifyContent::SpaceEvenly if !sizes.is_empty() => {
            let extra = free / (sizes.len() as f32 + 1.0);
            (extra, base_gap + extra)
        }
        _ => (0.0, base_gap),
    }
}

fn distribute_rows(
    heights: &[f32],
    base_gap: f32,
    available: f32,
    align: AlignContent,
) -> (f32, f32) {
    let track_total = heights.iter().sum::<f32>();
    let gap_count = heights.len().saturating_sub(1) as f32;
    let natural = track_total + base_gap * gap_count;
    let free = (available - natural).max(0.0);
    match align {
        AlignContent::FlexEnd => (free, base_gap),
        AlignContent::Center => (free / 2.0, base_gap),
        AlignContent::SpaceBetween if heights.len() > 1 => (0.0, base_gap + free / gap_count),
        AlignContent::SpaceAround if !heights.is_empty() => {
            let extra = free / heights.len() as f32;
            (extra / 2.0, base_gap + extra)
        }
        AlignContent::SpaceEvenly if !heights.is_empty() => {
            let extra = free / (heights.len() as f32 + 1.0);
            (extra, base_gap + extra)
        }
        _ => (0.0, base_gap),
    }
}

fn collect_grid_item_runs(
    cs: &ComputedStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> Vec<super::engine::TextRun> {
    let mut runs = Vec::new();
    FlexTextRunCollector {
        runs: &mut runs,
        rules: env.rules,
        fonts: env.fonts,
    }
    .collect(&child_el.children, cs, None, (0.0, 0.0), ancestors);
    runs
}

fn grid_item_has_block_child(child_el: &ElementNode) -> bool {
    child_el.children.iter().any(|child| match child {
        DomNode::Element(el) => el.tag.is_block(),
        DomNode::Text(_) => false,
    })
}

fn collect_grid_item_leading_runs(
    cs: &ComputedStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> Vec<super::engine::TextRun> {
    let mut leading = child_el.clone();
    leading.children.clear();
    for child in &child_el.children {
        match child {
            DomNode::Element(el) if el.tag.is_block() => break,
            _ => leading.children.push(child.clone()),
        }
    }
    collect_grid_item_runs(cs, env, &leading, ancestors)
}

fn measure_run_text(run: &super::engine::TextRun, text: &str, env: &LayoutEnv) -> f32 {
    if let Some(inline) = run.inline_box.as_deref() {
        return inline.outer_width();
    }
    estimate_word_width(
        text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.italic,
        env.fonts,
    )
}

fn grid_item_intrinsic_widths(
    cs: &ComputedStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> (f32, f32) {
    let runs = if grid_item_has_block_child(child_el) {
        collect_grid_item_leading_runs(cs, env, child_el, ancestors)
    } else {
        collect_grid_item_runs(cs, env, child_el, ancestors)
    };
    let max_content = super::helpers::measure_runs_width(&runs, env.fonts);
    let min_content = if matches!(cs.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre) {
        max_content
    } else {
        runs.iter()
            .map(|run| {
                if run.inline_box.is_some() {
                    return measure_run_text(run, "", env);
                }
                run.text
                    .split_whitespace()
                    .map(|word| measure_run_text(run, word, env))
                    .fold(0.0_f32, f32::max)
            })
            .fold(0.0_f32, f32::max)
    };
    let extras = cs.padding.left
        + cs.padding.right
        + cs.border.left.width
        + cs.border.right.width
        + cs.margin.left
        + cs.margin.right;
    (min_content + extras, max_content + extras)
}

fn is_intrinsic_column_track(track: RuntimeTrack) -> bool {
    matches!(
        track,
        RuntimeTrack::Auto
            | RuntimeTrack::MinContent
            | RuntimeTrack::MaxContent
            | RuntimeTrack::FitContent(_)
            | RuntimeTrack::Minmax(
                TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent,
                _
            )
    )
}

fn add_spanning_contribution(
    widths: &mut [f32],
    tracks: &[RuntimeTrack],
    start: usize,
    span: usize,
    contribution: f32,
) {
    let end = (start + span).min(widths.len()).min(tracks.len());
    if start >= end {
        return;
    }
    let current = widths[start..end].iter().sum::<f32>();
    if contribution <= current {
        return;
    }
    let growable: Vec<usize> = (start..end)
        .filter(|&i| is_intrinsic_column_track(tracks[i]))
        .collect();
    if growable.is_empty() {
        return;
    }
    let empty_growable: Vec<usize> = growable
        .iter()
        .copied()
        .filter(|&i| widths[i] <= 0.01)
        .collect();
    let recipients = if empty_growable.len() > 1 {
        vec![*empty_growable.last().unwrap()]
    } else if empty_growable.is_empty() {
        growable
    } else {
        empty_growable
    };
    let share = (contribution - current) / recipients.len() as f32;
    for i in recipients {
        widths[i] += share;
    }
}

/// The outer height a grid item wants: an explicit `height` (border-box) or
/// the measured text height plus vertical padding.
fn grid_item_outer_height(
    cs: &ComputedStyle,
    ctx: Option<&LayoutContext>,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
    available_width: Option<f32>,
) -> f32 {
    if let Some(h) = cs.height {
        return h;
    }
    let runs = collect_grid_item_runs(cs, env, child_el, ancestors);
    let line_h_factor = resolved_line_height_factor(cs, env.fonts);
    let text_h = if runs.is_empty() {
        0.0
    } else if let Some(width) = available_width {
        let wrap_width = if matches!(cs.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre) {
            f32::MAX
        } else {
            width.max(1.0)
        };
        wrap_text_runs(
            runs,
            TextWrapOptions::new(wrap_width, cs.font_size, line_h_factor, cs.overflow_wrap)
                .with_rtl(cs.direction_rtl)
                .with_bidi_override(cs.bidi_override)
                .with_pre_wrap(matches!(
                    cs.white_space,
                    WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
                ))
                .with_break_spaces(cs.white_space == WhiteSpace::BreakSpaces),
            env.fonts,
        )
        .iter()
        .map(|line| line.height)
        .sum()
    } else {
        cs.font_size * line_h_factor
    };
    let block_h = if let (Some(ctx), Some(width)) = (ctx, available_width) {
        layout_grid_item_children(
            child_el,
            cs,
            ctx,
            ancestors,
            (width - cs.border.horizontal_width()).max(0.0),
            None,
            env,
            None,
        )
        .iter()
        .map(super::paginate::estimate_element_height)
        .sum::<f32>()
    } else {
        0.0
    };
    // Border-box auto height includes the border: an empty bordered item still
    // reserves its border thickness. Without it, the implicit auto track sizes to
    // 0 and a later border stroke emits a negative-height rect.
    text_h
        + block_h
        + cs.padding.top
        + cs.padding.bottom
        + cs.border.top.width
        + cs.border.bottom.width
}

fn grid_item_first_baseline(cs: &ComputedStyle, has_text: bool, env: &LayoutEnv) -> Option<f32> {
    if !has_text {
        return None;
    }
    let line_h = cs.font_size * resolved_line_height_factor(cs, env.fonts);
    let half_leading = ((line_h - cs.font_size) / 2.0).max(0.0);
    Some(cs.border.top.width + cs.padding.top + half_leading + cs.font_size * 0.8)
}

/// Lay out a grid item's block-level children into nested layout elements,
/// sized against the item's content-box width. Returns the flattened layout
/// elements (block children of the item); inline text is handled separately by
/// the caller via `FlexTextRunCollector`. The cell's `overflow` clips these at
/// paint time, so an oversized inner block is painted but cut to the cell.
#[allow(clippy::too_many_arguments)]
fn layout_grid_item_children(
    item_el: &ElementNode,
    item_style: &ComputedStyle,
    ctx: &LayoutContext,
    item_ancestors: &[AncestorInfo],
    content_width: f32,
    content_height: Option<f32>,
    env: &mut LayoutEnv,
    subgrid: Option<SubgridContext>,
) -> Vec<LayoutElement> {
    use crate::parser::css::AncestorInfo;
    use crate::style::computed::Display;

    let mut out: Vec<LayoutElement> = Vec::new();
    // Only block-level element children become nested layout rows; inline text
    // is collected by the caller. A grid item is a block container, so its
    // children flow as a block formatting context inside the item's content box.
    let child_ctx = ctx.with_parent(
        content_width,
        item_style.height.or(content_height),
        item_style.font_size,
    );

    let mut child_ancestors: Vec<AncestorInfo> = item_ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: item_el,
        child_index: 0,
        sibling_count: item_el.children.len(),
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });

    // A grid item that is itself a flex or grid container must arrange its OWN
    // children via that formatting context, not flow them as independent blocks.
    // Lay it out through the matching container path against the item's content
    // box (`content_width`), so e.g. a `display:flex` cell distributes its boxes
    // along the main axis instead of stacking them block-by-block.
    if matches!(item_style.display, Display::Flex | Display::Grid) {
        // Give the inner container exactly the item's content-box width so flex
        // main-axis distribution / grid track sizing resolve correctly.
        let mut inner_style = item_style.clone();
        // The padding/border/background of the grid item are painted by the cell
        // itself; the inner formatting context should not re-apply them or it
        // would double-inset. Use a zero-margin/border/padding clone sized to the
        // content box.
        inner_style.margin = Default::default();
        inner_style.padding = Default::default();
        inner_style.border = Default::default();
        inner_style.background_color = None;
        inner_style.width = Some(content_width);
        // The inner formatting context spans the item's CONTENT box, so a definite
        // item height must be reduced to its content-box height here (the cell's
        // border + padding are stripped above). Otherwise an inner flex/grid would
        // use the full border-box height for cross-axis sizing / `align-items`,
        // pushing centered items down by the padding+border amount.
        if let Some(h) = item_style.height {
            let content_h = if item_style.box_sizing == BoxSizing::BorderBox {
                (h - item_style.border.vertical_width()
                    - item_style.padding.top
                    - item_style.padding.bottom)
                    .max(0.0)
            } else {
                h
            };
            inner_style.height = Some(content_h);
        } else if let Some(content_h) = content_height {
            inner_style.height = Some(content_h);
        }
        if item_style.display == Display::Flex {
            crate::layout::flex::layout_flex_container(
                item_el,
                &inner_style,
                &child_ctx,
                &mut out,
                &child_ancestors,
                None,
                None,
                0,
                env,
            );
        } else {
            layout_grid_container_inner(
                item_el,
                &inner_style,
                &child_ctx,
                &mut out,
                &child_ancestors,
                0,
                env,
                subgrid,
            );
        }
        return out;
    }

    let element_children: Vec<&ElementNode> = item_el
        .children
        .iter()
        .filter_map(|c| match c {
            DomNode::Element(e) => Some(e),
            DomNode::Text(_) => None,
        })
        .collect();
    let sibling_count = element_children.len();
    let mut preceding: Vec<(String, Vec<String>)> = Vec::new();
    let mut element_idx = 0usize;
    let mut after_block = false;
    for child in &item_el.children {
        let DomNode::Element(child_el) = child else {
            if after_block {
                if let DomNode::Text(text) = child {
                    if !text.trim().is_empty() {
                        let mut text_block = ElementNode::new(crate::parser::dom::HtmlTag::Div);
                        text_block.attributes.insert(
                            "style".to_string(),
                            format!(
                                "margin:0; padding:0; background:transparent; font-size:{}pt; line-height:{};",
                                item_style.font_size,
                                resolved_line_height_factor(item_style, env.fonts)
                            ),
                        );
                        text_block.children.push(DomNode::Text(text.clone()));
                        crate::layout::engine::flatten_element(
                            &text_block,
                            item_style,
                            &child_ctx,
                            &mut out,
                            None,
                            &child_ancestors,
                            0,
                            element_idx,
                            sibling_count,
                            &preceding,
                            &[],
                            env,
                        );
                    }
                }
            }
            continue;
        };
        let idx = element_idx;
        element_idx += 1;
        // Skip inline children: their text is already collected for the cell
        // `lines`. Only block / inline-block / flex / grid children need a
        // nested layout element.
        let child_style = compute_style_with_context(
            child_el.tag,
            child_el.style_attr(),
            item_style,
            env.rules,
            child_el.tag_name(),
            &child_el.class_list(),
            child_el.id(),
            &child_el.attributes,
            &SelectorContext {
                ancestors: child_ancestors.clone(),
                child_index: idx,
                sibling_count,
                preceding_siblings: preceding.clone(),
                following_siblings: Vec::new(),
                is_empty: false,
            },
        );
        let is_block = matches!(
            child_style.display,
            Display::Block | Display::InlineBlock | Display::Flex | Display::Grid
        );
        if is_block {
            after_block = true;
            crate::layout::engine::flatten_element(
                child_el,
                item_style,
                &child_ctx,
                &mut out,
                None,
                &child_ancestors,
                0,
                idx,
                sibling_count,
                &preceding,
                &[],
                env,
            );
        }
        preceding.push((
            child_el.tag_name().to_string(),
            child_el
                .class_list()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ));
    }
    out
}

/// An empty filler cell that still occupies the track height so the grid row
/// keeps its geometry when an item is absent in that column.
fn empty_grid_cell(track_h: f32) -> TableCell {
    TableCell {
        lines: Vec::new(),
        nested_rows: Vec::new(),
        bold: false,
        background_color: None,
        padding_top: 0.0,
        padding_right: 0.0,
        padding_bottom: 0.0,
        padding_left: 0.0,
        colspan: 1,
        rowspan: 1,
        border: LayoutBorder::default(),
        text_align: TextAlign::Left,
        vertical_align: VerticalAlign::Baseline,
        min_content_height: track_h,
        hide_if_empty: false,
        grid_inset: None,
        clips: false,
        background_gradient: None,
        background_radial_gradient: None,
        background_conic_gradient: None,
    }
}

/// Compute the painted-box inset of a grid item within its track cell from the
/// per-axis `justify-items` (inline) and `align-items` (block) keywords. Only
/// applies when the item has an explicit size smaller than the track; otherwise
/// the item stretches to fill (returns `None`).
fn compute_grid_inset(
    cs: &ComputedStyle,
    container: &ComputedStyle,
    track_w: f32,
    track_h: f32,
) -> Option<GridInset> {
    let item_w = cs.width;
    let item_h = cs.height;
    // Per-item `justify-self` / `align-self` override the container's
    // `justify-items` / `align-items` (CSS Grid §10.x / box-alignment).
    let justify = cs.grid_justify_self.unwrap_or(container.justify_items);
    let align = cs.grid_align_self.unwrap_or(container.grid_align_items);

    let margin_left = if cs.margin_left_auto {
        0.0
    } else {
        cs.margin.left
    };
    let margin_right = if cs.margin_right_auto {
        0.0
    } else {
        cs.margin.right
    };
    let margin_top = if cs.margin_top_auto {
        0.0
    } else {
        cs.margin.top
    };
    let margin_bottom = if cs.margin_bottom_auto {
        0.0
    } else {
        cs.margin.bottom
    };
    let margin_w = margin_left + margin_right;
    let margin_h = margin_top + margin_bottom;
    let align_w = (track_w - margin_w).max(0.0);
    let align_h = (track_h - margin_h).max(0.0);

    // Stretch on both axes with no explicit size and no margins → fill the
    // track (no inset).
    let stretch_w = item_w.is_none() && justify == GridAlign::Stretch;
    let stretch_h = item_h.is_none() && align == GridAlign::Stretch;
    if stretch_w && stretch_h && margin_w == 0.0 && margin_h == 0.0 {
        return None;
    }

    let box_w = item_w.unwrap_or(align_w).min(align_w);
    let box_h = item_h.unwrap_or(align_h).min(align_h);

    let free_x = (align_w - box_w).max(0.0);
    let free_y = (align_h - box_h).max(0.0);
    let (auto_left, auto_right) = (cs.margin_left_auto, cs.margin_right_auto);
    let (auto_top, auto_bottom) = (cs.margin_top_auto, cs.margin_bottom_auto);

    let offset_x = match justify {
        _ if auto_left && auto_right => free_x / 2.0,
        _ if auto_left => free_x,
        GridAlign::Start | GridAlign::Stretch => 0.0,
        GridAlign::End => free_x,
        GridAlign::Center => free_x / 2.0,
    };
    let offset_y = match align {
        _ if auto_top && auto_bottom => free_y / 2.0,
        _ if auto_top => free_y,
        GridAlign::Start | GridAlign::Stretch => 0.0,
        GridAlign::End => free_y,
        GridAlign::Center => free_y / 2.0,
    };
    // When stretching one axis, use the full track extent on that axis.
    let final_w = if stretch_w { align_w } else { box_w };
    let final_h = if stretch_h { align_h } else { box_h };

    Some(GridInset {
        offset_x: margin_left + offset_x,
        offset_y: margin_top + offset_y,
        width: final_w,
        height: final_h,
    })
}

/// A grid item placed in the integer track grid (0-based track indices).
struct Placed {
    idx: usize,
    col: usize,
    row: usize,
    col_span: usize,
    row_span: usize,
}

/// Result of the grid placement pass: every item placed, plus the final grid
/// dimensions (which may exceed the explicit track count when items reference
/// implicit lines / overflow into implicit tracks).
struct GridPlacement {
    placed: Vec<Placed>,
    num_cols: usize,
    num_rows: usize,
}

/// Build a `name -> first 0-based line index` map for one axis. CSS Grid §8.3:
/// a named line reference resolves to the *first* line bearing that name.
/// `track_line_names[i]` holds the names declared at line `i`. The
/// `grid-template-areas` of the container also generate implicit
/// `<area>-start` / `<area>-end` line names on the relevant axis.
fn build_line_name_map(
    track_line_names: &[Vec<String>],
    area_lines: &[(String, usize)],
    final_line_hint: usize,
) -> std::collections::HashMap<String, usize> {
    let mut map = std::collections::HashMap::new();
    for (line_idx, names) in track_line_names.iter().enumerate() {
        for n in names {
            map.entry(n.clone()).or_insert(line_idx);
        }
    }
    let final_line = final_line_hint.max(track_line_names.len().saturating_sub(1));
    let starts: Vec<String> = map
        .keys()
        .filter_map(|name| name.strip_suffix("-start").map(ToString::to_string))
        .collect();
    for name in starts {
        let end = format!("{name}-end");
        map.entry(end).or_insert(final_line);
    }
    // Implicit area lines fill in any names not already declared explicitly.
    for (name, line_idx) in area_lines {
        map.entry(name.clone()).or_insert(*line_idx);
    }
    map
}

/// Implicit `<area>-start` / `<area>-end` line names for one axis, derived from
/// `grid-template-areas`. For columns, an area spanning columns `c0..=c1`
/// generates `name-start` at line `c0` and `name-end` at line `c1 + 1`.
fn area_lines_for_axis(areas: &[Vec<Option<String>>], axis_columns: bool) -> Vec<(String, usize)> {
    // Compute each area's bounding rectangle (min/max row & col).
    let mut bounds: std::collections::HashMap<&str, (usize, usize, usize, usize)> =
        std::collections::HashMap::new();
    for (r, row) in areas.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(name) = cell {
                let e = bounds.entry(name.as_str()).or_insert((r, r, c, c));
                e.0 = e.0.min(r);
                e.1 = e.1.max(r);
                e.2 = e.2.min(c);
                e.3 = e.3.max(c);
            }
        }
    }
    let mut out = Vec::new();
    for (name, (r0, r1, c0, c1)) in bounds {
        if axis_columns {
            out.push((format!("{name}-start"), c0));
            out.push((format!("{name}-end"), c1 + 1));
        } else {
            out.push((format!("{name}-start"), r0));
            out.push((format!("{name}-end"), r1 + 1));
        }
    }
    out
}

/// Resolve a `GridLine` endpoint to a concrete 0-based line index, given the
/// number of *explicit* tracks on the axis and the axis's name map. Returns
/// `None` for `Auto` / `Span` (the opposite edge is definite) and for
/// unresolved named lines. `negative -1` = the last explicit line.
fn resolve_line(
    line: &GridLine,
    explicit_tracks: usize,
    names: &std::collections::HashMap<String, usize>,
) -> Option<usize> {
    match line {
        GridLine::Line(n) => {
            if *n > 0 {
                Some((*n - 1) as usize)
            } else {
                // Negative: -1 = last explicit line = `explicit_tracks`.
                let from_end = (-*n) as usize; // 1-based from the end
                Some(explicit_tracks.saturating_add(1).saturating_sub(from_end))
            }
        }
        GridLine::Named(name) => names.get(name).copied(),
        GridLine::Auto | GridLine::Span(_) | GridLine::SpanNamed(_) => None,
    }
}

/// Resolve one axis of an item's placement to a definite `(start, span)` in
/// 0-based track coordinates, or `None` when the start is auto (auto-placed).
/// Handles the line/line, line/span, span/line, span-only, and auto cases
/// (§8.3 placement shorthand resolution).
fn resolve_axis(
    start: &GridLine,
    end: &GridLine,
    explicit_tracks: usize,
    names: &std::collections::HashMap<String, usize>,
) -> Option<(usize, usize)> {
    let span_of = |g: &GridLine| -> Option<usize> {
        match g {
            GridLine::Span(n) => Some((*n).max(1)),
            GridLine::SpanNamed(_) => Some(1),
            _ => None,
        }
    };
    let s_line = resolve_line(start, explicit_tracks, names);
    let e_line = resolve_line(end, explicit_tracks, names);

    match (s_line, e_line) {
        (Some(s), Some(e)) => {
            let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
            Some((lo, (hi - lo).max(1)))
        }
        (Some(s), None) => {
            // start definite; end is span or auto (→ span 1).
            let span = match end {
                GridLine::SpanNamed(name) => names
                    .get(name)
                    .copied()
                    .filter(|line| *line > s)
                    .map(|line| line - s)
                    .unwrap_or(1),
                _ => span_of(end).unwrap_or(1),
            };
            Some((s, span))
        }
        (None, Some(e)) => {
            // end definite; start is span (count back) or auto (→ span 1).
            let span = match start {
                GridLine::SpanNamed(name) => names
                    .get(name)
                    .copied()
                    .filter(|line| *line < e)
                    .map(|line| e - line)
                    .unwrap_or(1),
                _ => span_of(start).unwrap_or(1),
            };
            let s = e.saturating_sub(span);
            Some((s, span))
        }
        (None, None) => None,
    }
}

/// Run the CSS Grid placement + §8.5 auto-placement algorithm. Items with a
/// definite position (both edges resolvable on an axis, or a named area) are
/// placed first; the rest are auto-placed by a sparse (or dense) cursor.
fn place_grid_items(
    container: &ComputedStyle,
    child_styles: &[ComputedStyle],
    explicit_cols_hint: usize,
    explicit_cols_override: Option<usize>,
    explicit_rows_override: Option<usize>,
    column_line_names_override: Option<&[Vec<String>]>,
    row_line_names_override: Option<&[Vec<String>]>,
) -> GridPlacement {
    let explicit_cols = explicit_cols_override.unwrap_or(container.grid_template_columns.len());
    let explicit_rows = explicit_rows_override.unwrap_or(container.grid_template_rows.len());
    let areas = &container.grid_template_areas;

    // Area-derived implicit line names per axis.
    let col_area_lines = area_lines_for_axis(areas, true);
    let row_area_lines = area_lines_for_axis(areas, false);
    let merged_line_names = |base: &[Vec<String>], extra: Option<&[Vec<String>]>| {
        let Some(extra) = extra.filter(|names| !names.is_empty()) else {
            return base.to_vec();
        };
        let mut merged = vec![Vec::new(); base.len().max(extra.len())];
        for (i, names) in base.iter().enumerate() {
            merged[i].extend(names.iter().cloned());
        }
        for (i, names) in extra.iter().enumerate() {
            merged[i].extend(names.iter().cloned());
        }
        merged
    };
    let column_line_names = merged_line_names(
        &container.grid_template_column_line_names,
        column_line_names_override,
    );
    let row_line_names = merged_line_names(
        &container.grid_template_row_line_names,
        row_line_names_override,
    );
    let col_names = build_line_name_map(&column_line_names, &col_area_lines, explicit_cols);
    let row_names = build_line_name_map(&row_line_names, &row_area_lines, explicit_rows);

    // The column axis must accommodate the explicit tracks, the area columns,
    // and `grid-template-columns`. Use the widest of these as the wrap width.
    let area_cols = areas.iter().map(|r| r.len()).max().unwrap_or(0);
    let num_cols = explicit_cols.max(area_cols).max(explicit_cols_hint).max(1);
    let column_flow = container.grid_auto_flow_column;

    // Per-item resolved axis placement (None on an axis = auto on that axis).
    struct Resolved {
        idx: usize,
        col: Option<(usize, usize)>,
        row: Option<(usize, usize)>,
    }
    let mut resolved: Vec<Resolved> = Vec::with_capacity(child_styles.len());
    for (idx, cs) in child_styles.iter().enumerate() {
        // grid-area: <name> → resolve against the area's -start/-end lines.
        let (mut cs_col, mut cs_row) = (
            (cs.grid_column_start.clone(), cs.grid_column_end.clone()),
            (cs.grid_row_start.clone(), cs.grid_row_end.clone()),
        );
        if let Some(name) = &cs.grid_area_name {
            let area_exists = areas
                .iter()
                .flatten()
                .any(|cell| cell.as_deref() == Some(name.as_str()));
            let has_implicit_area_lines = col_names.contains_key(&format!("{name}-start"))
                && col_names.contains_key(&format!("{name}-end"))
                && row_names.contains_key(&format!("{name}-start"))
                && row_names.contains_key(&format!("{name}-end"));
            if !area_exists && !has_implicit_area_lines {
                let implicit_col = explicit_cols.max(area_cols) + idx;
                let implicit_row = explicit_rows.max(areas.len()) + idx;
                resolved.push(Resolved {
                    idx,
                    col: Some((implicit_col, 1)),
                    row: Some((implicit_row, 1)),
                });
                continue;
            }
            cs_col = (
                GridLine::Named(format!("{name}-start")),
                GridLine::Named(format!("{name}-end")),
            );
            cs_row = (
                GridLine::Named(format!("{name}-start")),
                GridLine::Named(format!("{name}-end")),
            );
        }
        let col = resolve_axis(
            &cs_col.0,
            &cs_col.1,
            explicit_cols.max(area_cols),
            &col_names,
        );
        let row = resolve_axis(
            &cs_row.0,
            &cs_row.1,
            explicit_rows.max(areas.len()),
            &row_names,
        );
        resolved.push(Resolved { idx, col, row });
    }
    let mut order_modified: Vec<usize> = (0..resolved.len()).collect();
    order_modified.sort_by_key(|&i| (child_styles[resolved[i].idx].order, resolved[i].idx));

    // Occupancy grid (row-major, grown on demand).
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let ensure = |occ: &mut Vec<Vec<bool>>, r: usize, cols: usize| {
        while occ.len() <= r {
            occ.push(vec![false; cols]);
        }
        for row in occ.iter_mut() {
            if row.len() < cols {
                row.resize(cols, false);
            }
        }
    };
    let fits = |occ: &[Vec<bool>], r: usize, c: usize, rs: usize, cs: usize| -> bool {
        for rr in r..r + rs {
            let Some(row) = occ.get(rr) else { continue };
            if row.iter().skip(c).take(cs).any(|&occupied| occupied) {
                return false;
            }
        }
        true
    };
    let mark = |occ: &mut Vec<Vec<bool>>, r: usize, c: usize, rs: usize, cs: usize| {
        let need_cols = c + cs;
        for rr in r..r + rs {
            ensure(occ, rr, need_cols);
            for slot in occ[rr].iter_mut().skip(c).take(cs) {
                *slot = true;
            }
        }
    };

    let mut placed: Vec<Placed> = Vec::with_capacity(child_styles.len());
    let mut max_cols = num_cols;

    // Phase 1: items definite on BOTH axes → fixed position.
    for &resolved_idx in &order_modified {
        let r = &resolved[resolved_idx];
        if let (Some((c, cspan)), Some((rw, rspan))) = (r.col, r.row) {
            mark(&mut occupied, rw, c, rspan, cspan);
            max_cols = max_cols.max(c + cspan);
            placed.push(Placed {
                idx: r.idx,
                col: c,
                row: rw,
                col_span: cspan,
                row_span: rspan,
            });
        }
    }

    // Phase 2: auto-placement of the remaining items, in source order, using a
    // cursor. Sparse (default) never moves the cursor backward; dense restarts
    // the search from the origin for each item.
    let dense = container.grid_auto_flow_dense;
    let mut cursor_major = 0usize; // row (row-flow) or col (column-flow)
    let mut cursor_minor = 0usize; // col (row-flow) or row (column-flow)

    for &resolved_idx in &order_modified {
        let r = &resolved[resolved_idx];
        if placed.iter().any(|p| p.idx == r.idx) {
            continue; // already placed in phase 1
        }
        let cs = &child_styles[r.idx];

        if column_flow {
            // Column-major auto-placement. The wrap bound is the explicit row
            // count (fallback 1).
            let num_rows_bound = explicit_rows.max(1);
            let (col_known, cspan) = match r.col {
                Some((c, s)) => (Some(c), s),
                None => (None, cs.grid_column_span.max(1)),
            };
            let rspan = match r.row {
                Some((_, s)) => s,
                None => cs.grid_row_span.max(1).min(num_rows_bound),
            };
            if dense {
                cursor_major = 0;
                cursor_minor = 0;
            }
            let mut local_major = if r.row.is_some() && r.col.is_none() {
                0
            } else {
                cursor_major
            };
            let mut local_minor = if r.col.is_some() && r.row.is_none() {
                0
            } else {
                cursor_minor
            };
            let mut definite_row_collision = false;
            loop {
                let row_pos = match r.row {
                    Some((rw, _)) => rw,
                    None => local_minor,
                };
                let col_pos = col_known.unwrap_or(local_major);
                // Wrap rows within the bound when row is auto.
                if r.row.is_none() && local_minor + rspan > num_rows_bound {
                    local_minor = 0;
                    local_major += 1;
                    continue;
                }
                ensure(&mut occupied, row_pos + rspan - 1, col_pos + cspan);
                if fits(&occupied, row_pos, col_pos, rspan, cspan) {
                    mark(&mut occupied, row_pos, col_pos, rspan, cspan);
                    max_cols = max_cols.max(col_pos + cspan);
                    placed.push(Placed {
                        idx: r.idx,
                        col: col_pos,
                        row: row_pos,
                        col_span: cspan,
                        row_span: rspan,
                    });
                    if r.row.is_none() && r.col.is_none() {
                        cursor_minor = row_pos + rspan;
                        cursor_major = col_pos;
                    } else if r.row.is_none() && definite_row_collision {
                        cursor_major = cursor_major.max(col_pos + cspan);
                    } else {
                        // A definite row is packed independently and does not
                        // advance the sparse auto-placement cursor.
                    }
                    break;
                }
                if r.row.is_none() {
                    local_minor += 1;
                } else {
                    definite_row_collision = true;
                    local_major += 1;
                }
            }
        } else {
            // Row-major auto-placement (default).
            let cspan = match r.col {
                Some((_, s)) => s,
                None => cs.grid_column_span.max(1),
            }
            .min(num_cols.max(1));
            let rspan = match r.row {
                Some((_, s)) => s,
                None => cs.grid_row_span.max(1),
            };
            if dense {
                cursor_major = 0;
                cursor_minor = 0;
            }
            let mut local_major = if r.col.is_some() && r.row.is_none() {
                0
            } else {
                cursor_major
            };
            let mut local_minor = if r.row.is_some() && r.col.is_none() {
                0
            } else {
                cursor_minor
            };
            loop {
                let col_pos = match r.col {
                    Some((c, _)) => c,
                    None => local_minor,
                };
                let row_pos = match r.row {
                    Some((rw, _)) => rw,
                    None => local_major,
                };
                // Wrap columns when column is auto.
                if r.col.is_none() && col_pos + cspan > num_cols {
                    local_minor = 0;
                    local_major += 1;
                    continue;
                }
                ensure(&mut occupied, row_pos + rspan - 1, num_cols);
                if fits(&occupied, row_pos, col_pos, rspan, cspan) {
                    mark(&mut occupied, row_pos, col_pos, rspan, cspan);
                    max_cols = max_cols.max(col_pos + cspan);
                    placed.push(Placed {
                        idx: r.idx,
                        col: col_pos,
                        row: row_pos,
                        col_span: cspan,
                        row_span: rspan,
                    });
                    if r.col.is_none() && r.row.is_none() {
                        cursor_minor = col_pos + cspan;
                        cursor_major = row_pos;
                    } else if r.col.is_none() {
                        // A definite row is packed independently and does not
                        // advance the sparse auto-placement cursor.
                    } else {
                        cursor_minor = col_pos + cspan;
                        cursor_major = row_pos;
                    }
                    break;
                }
                if r.col.is_none() {
                    local_minor += 1;
                } else {
                    local_major += 1;
                }
            }
        }
    }

    // Restore source order so later per-row emission is deterministic.
    placed.sort_by_key(|p| p.idx);
    let num_rows = placed.iter().map(|p| p.row + p.row_span).max().unwrap_or(0);
    GridPlacement {
        placed,
        num_cols: max_cols,
        num_rows,
    }
}

/// Lay out a CSS Grid container into GridRow layout elements.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_grid_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    positioned_depth: usize,
    env: &mut LayoutEnv,
) {
    layout_grid_container_inner(
        el,
        style,
        ctx,
        output,
        ancestors,
        positioned_depth,
        env,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn layout_grid_container_inner(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    positioned_depth: usize,
    env: &mut LayoutEnv,
    subgrid: Option<SubgridContext>,
) {
    let available_width = ctx.available_width();
    // The track-sizing basis is the container's content-box width. When an
    // explicit `width` is set it wins (resolving box-sizing: a border-box
    // width already includes border+padding, so subtract them; a content-box
    // width is used directly). Otherwise fall back to the available width.
    let border_pad_w = style.border.left.width
        + style.border.right.width
        + style.padding.left
        + style.padding.right;
    let inner_width = match style.width {
        Some(w) => {
            if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
                (w - border_pad_w).max(0.0)
            } else {
                w
            }
        }
        None => {
            let auto_border_adjust = if style.margin.left != 0.0 || style.margin.right != 0.0 {
                style.border.left.width + style.border.right.width
            } else {
                0.0
            };
            (available_width - style.margin.left - style.margin.right)
                - style.padding.left
                - style.padding.right
                - auto_border_adjust
        }
    };
    // The container's border-box width (used for the wrapping Container's
    // block width and to resolve horizontal margin / auto-centering).
    let border_box_w = inner_width + border_pad_w;
    // Horizontal offset of the grid container within the available width:
    // explicit `margin-left`, or centering when both side margins are auto and
    // the box is narrower than the line. Mirrors block-level positioning so the
    // grid lines up with where Chrome paints it.
    let h_offset = if style.width.is_some() && border_box_w < available_width {
        if style.margin_left_auto && style.margin_right_auto {
            (available_width - border_box_w) / 2.0
        } else if style.margin_left_auto {
            available_width - border_box_w
        } else {
            style.margin.left
        }
    } else {
        style.margin.left
    };
    let column_gap = subgrid
        .as_ref()
        .and_then(|ctx| ctx.columns.as_ref().map(|axis| axis.gap))
        .unwrap_or(style.column_gap);
    let row_gap = subgrid
        .as_ref()
        .and_then(|ctx| ctx.rows.as_ref().map(|axis| axis.gap))
        .unwrap_or(style.row_gap);

    // Collect element children (skip text nodes) so we can measure intrinsic
    // widths per column before resolving track sizes.
    let all_element_children: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|child| {
            if let DomNode::Element(child_el) = child {
                Some(child_el)
            } else {
                None
            }
        })
        .collect();

    // Compute each child's style once and remember it alongside the element.
    let child_ancestors_base: Vec<AncestorInfo> = ancestors.to_vec();
    let mut child_ancestors: Vec<AncestorInfo> = child_ancestors_base.clone();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });

    let total_child_count = all_element_children.len();
    let child_siblings: Vec<(String, Vec<String>)> = all_element_children
        .iter()
        .map(|child_el| {
            (
                child_el.tag_name().to_string(),
                child_el.class_list().iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect();
    let all_child_styles: Vec<ComputedStyle> = all_element_children
        .iter()
        .enumerate()
        .map(|(idx, child_el)| {
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: child_ancestors.clone(),
                child_index: idx,
                sibling_count: total_child_count,
                preceding_siblings: child_siblings[..idx].to_vec(),
                following_siblings: child_siblings[idx + 1..].to_vec(),
                is_empty: false,
            };
            compute_style_with_context(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
            )
        })
        .collect();

    // Per CSS Grid §9.1, an absolutely-positioned child of a grid container is
    // NOT a grid item; it is taken out of flow and laid out against the grid
    // container's padding box. Separate such children out so they don't consume
    // grid tracks, then emit them as positioned boxes inside the wrapping
    // Container (which establishes the containing block).
    let abs_child_indices: Vec<usize> = (0..total_child_count)
        .filter(|&i| all_child_styles[i].position == Position::Absolute)
        .collect();
    let mut element_children: Vec<ElementNode> = Vec::new();
    let mut child_styles: Vec<ComputedStyle> = Vec::new();

    let container_classes = el.class_list();
    let container_selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    if let Some(before_style) = compute_pseudo_element_style(
        style,
        env.rules,
        el.tag_name(),
        &container_classes,
        el.id(),
        &el.attributes,
        &container_selector_ctx,
        PseudoElement::Before,
    ) {
        element_children.push(pseudo_element_node(&before_style));
        child_styles.push(before_style);
    }

    let mut element_idx = 0usize;
    for child in &el.children {
        match child {
            DomNode::Text(text) => {
                if !text.trim().is_empty() {
                    let mut node = ElementNode::new(crate::parser::dom::HtmlTag::Span);
                    node.children.push(DomNode::Text(text.clone()));
                    element_children.push(node);
                    child_styles.push(anonymous_grid_item_style(style));
                }
            }
            DomNode::Element(child_el) => {
                let direct_idx = element_idx;
                element_idx += 1;
                let direct_style = &all_child_styles[direct_idx];
                if direct_style.position == Position::Absolute {
                    continue;
                }

                if matched_display_contents(child_el, child_el.style_attr(), env.rules, ancestors) {
                    let mut wrapper_ancestors = child_ancestors.clone();
                    wrapper_ancestors.push(AncestorInfo {
                        element: child_el,
                        child_index: direct_idx,
                        sibling_count: total_child_count,
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    });
                    let flattened: Vec<&ElementNode> = child_el
                        .children
                        .iter()
                        .filter_map(|node| match node {
                            DomNode::Element(el) => Some(el),
                            DomNode::Text(_) => None,
                        })
                        .collect();
                    let flattened_count = flattened.len();
                    for (flat_idx, flat_el) in flattened.into_iter().enumerate() {
                        let flat_classes = flat_el.class_list();
                        let flat_style = compute_style_with_context(
                            flat_el.tag,
                            flat_el.style_attr(),
                            direct_style,
                            env.rules,
                            flat_el.tag_name(),
                            &flat_classes,
                            flat_el.id(),
                            &flat_el.attributes,
                            &SelectorContext {
                                ancestors: wrapper_ancestors.clone(),
                                child_index: flat_idx,
                                sibling_count: flattened_count,
                                preceding_siblings: Vec::new(),
                                following_siblings: Vec::new(),
                                is_empty: false,
                            },
                        );
                        if flat_style.position != Position::Absolute {
                            element_children.push(flat_el.clone());
                            child_styles.push(flat_style);
                        }
                    }
                } else {
                    element_children.push((*child_el).clone());
                    child_styles.push(direct_style.clone());
                }
            }
        }
    }

    if let Some(after_style) = compute_pseudo_element_style(
        style,
        env.rules,
        el.tag_name(),
        &container_classes,
        el.id(),
        &el.attributes,
        &container_selector_ctx,
        PseudoElement::After,
    ) {
        element_children.push(pseudo_element_node(&after_style));
        child_styles.push(after_style);
    }
    let auto_column_pattern = matched_grid_track_pattern(
        el,
        el.style_attr(),
        style,
        env.rules,
        ancestors,
        "grid-auto-columns",
    );
    let auto_row_pattern = matched_grid_track_pattern(
        el,
        el.style_attr(),
        style,
        env.rules,
        ancestors,
        "grid-auto-rows",
    );
    let RuntimeTrackList {
        tracks: mut column_tracks,
        auto_fit: mut column_auto_fit,
        line_names: column_line_names,
    } = runtime_tracks_for_property(
        el,
        el.style_attr(),
        style,
        env.rules,
        ancestors,
        "grid-template-columns",
        inner_width,
        column_gap,
    );
    let RuntimeTrackList {
        tracks: row_tracks,
        auto_fit: _,
        line_names: row_line_names,
    } = runtime_tracks_for_property(
        el,
        el.style_attr(),
        style,
        env.rules,
        ancestors,
        "grid-template-rows",
        inner_width,
        row_gap,
    );
    let subgrid_columns = subgrid.as_ref().and_then(|ctx| ctx.columns.as_ref());
    let subgrid_rows = subgrid.as_ref().and_then(|ctx| ctx.rows.as_ref());
    if let Some(axis) = subgrid_columns {
        column_tracks = axis
            .tracks
            .iter()
            .copied()
            .map(RuntimeTrack::Fixed)
            .collect();
        column_auto_fit = vec![false; column_tracks.len()];
    }
    let mut row_tracks = if let Some(axis) = subgrid_rows {
        axis.tracks
            .iter()
            .copied()
            .map(RuntimeTrack::Fixed)
            .collect::<Vec<_>>()
    } else {
        row_tracks
    };
    let effective_column_line_names = merge_line_name_lists(
        &merge_line_name_lists(&style.grid_template_column_line_names, &column_line_names),
        subgrid_columns
            .map(|axis| axis.line_names.as_slice())
            .unwrap_or(&[]),
    );
    let effective_row_line_names = merge_line_name_lists(
        &merge_line_name_lists(&style.grid_template_row_line_names, &row_line_names),
        subgrid_rows
            .map(|axis| axis.line_names.as_slice())
            .unwrap_or(&[]),
    );
    let explicit_col_count = column_tracks.len().max(1);
    let explicit_row_count_override = subgrid_rows.map(|axis| axis.tracks.len());

    // ---- Item placement (CSS Grid §8) -----------------------------------
    // Resolve each item's definite placement from grid-column / grid-row /
    // grid-area (line numbers, named lines, spans, named areas), then run the
    // §8.5 auto-placement algorithm for items left auto on either axis.
    let placement = place_grid_items(
        style,
        &child_styles,
        explicit_col_count,
        Some(explicit_col_count),
        explicit_row_count_override,
        Some(&effective_column_line_names),
        Some(&effective_row_line_names),
    );
    let mut placed = placement.placed;
    let mut num_cols = placement.num_cols;
    let mut num_rows = placement.num_rows;
    if style.writing_mode == crate::style::computed::WritingMode::VerticalRl {
        let logical_rows = num_rows.max(1);
        for p in &mut placed {
            let logical_col = p.col;
            let logical_row = p.row;
            let logical_col_span = p.col_span;
            let logical_row_span = p.row_span;
            p.col = logical_rows.saturating_sub(logical_row + logical_row_span);
            p.row = logical_col;
            p.col_span = logical_row_span;
            p.row_span = logical_col_span;
        }
        std::mem::swap(&mut column_tracks, &mut row_tracks);
        std::mem::swap(&mut num_cols, &mut num_rows);
    }
    if style.direction_rtl {
        for p in &mut placed {
            p.col = num_cols.saturating_sub(p.col + p.col_span);
        }
    }

    // ---- Track sizing ---------------------------------------------------
    while column_tracks.len() < num_cols {
        let implicit_idx = column_tracks.len().saturating_sub(explicit_col_count);
        let width = if auto_column_pattern.is_empty() {
            RuntimeTrack::Auto
        } else {
            RuntimeTrack::Fixed(auto_column_pattern[implicit_idx % auto_column_pattern.len()])
        };
        column_tracks.push(width);
        column_auto_fit.push(false);
    }
    let collapsed_auto_fit: Vec<bool> = column_auto_fit
        .iter()
        .enumerate()
        .map(|(i, is_auto_fit)| {
            *is_auto_fit
                && !placed
                    .iter()
                    .any(|p| p.col <= i && i < p.col.saturating_add(p.col_span))
        })
        .collect();
    if collapsed_auto_fit.iter().any(|collapsed| *collapsed) {
        let mut old_to_new = vec![0usize; column_tracks.len()];
        let mut kept_before = 0usize;
        for (i, collapsed) in collapsed_auto_fit.iter().copied().enumerate() {
            old_to_new[i] = kept_before;
            if !collapsed {
                kept_before += 1;
            }
        }
        for p in &mut placed {
            let end = (p.col + p.col_span).min(column_tracks.len());
            let kept_span = (p.col..end)
                .filter(|&i| !collapsed_auto_fit.get(i).copied().unwrap_or(false))
                .count();
            p.col = old_to_new.get(p.col).copied().unwrap_or(p.col);
            p.col_span = kept_span.max(1);
        }
        column_tracks = column_tracks
            .into_iter()
            .enumerate()
            .filter_map(|(i, track)| (!collapsed_auto_fit[i]).then_some(track))
            .collect();
        num_cols = column_tracks.len().max(1);
    }

    let mut min_intrinsic_widths = vec![0.0_f32; num_cols];
    let mut max_intrinsic_widths = vec![0.0_f32; num_cols];
    for p in &placed {
        let cs = &child_styles[p.idx];
        let (min_w, max_w) =
            grid_item_intrinsic_widths(cs, env, &element_children[p.idx], &child_ancestors);
        if p.col_span == 1 {
            if p.col < num_cols {
                min_intrinsic_widths[p.col] = min_intrinsic_widths[p.col].max(min_w);
                max_intrinsic_widths[p.col] = max_intrinsic_widths[p.col].max(max_w);
            }
        } else {
            add_spanning_contribution(
                &mut min_intrinsic_widths,
                &column_tracks,
                p.col,
                p.col_span,
                min_w,
            );
            add_spanning_contribution(
                &mut max_intrinsic_widths,
                &column_tracks,
                p.col,
                p.col_span,
                max_w,
            );
        }
    }

    let col_widths = resolve_grid_columns(
        &column_tracks,
        inner_width,
        column_gap,
        &min_intrinsic_widths,
        &max_intrinsic_widths,
    );

    // Rows: explicit template-rows first, then grid-auto-rows for implicit
    // rows, then content height as a final fallback.
    let explicit_content_height = style.height.map(|h| {
        if style.box_sizing == BoxSizing::BorderBox {
            (h - (style.border.top.width + style.border.bottom.width)
                - style.padding.top
                - style.padding.bottom)
                .max(0.0)
        } else {
            h
        }
    });
    let mut row_heights = vec![0.0_f32; num_rows];
    let rows_synthesized_from_areas = !style.grid_template_areas.is_empty()
        && !style.grid_template_rows.is_empty()
        && style
            .grid_template_rows
            .iter()
            .all(|track| matches!(track, GridTrack::Auto));
    for (r, h) in row_heights.iter_mut().enumerate() {
        let explicit = row_tracks
            .get(r)
            .and_then(|track| grid_track_fixed_height(track, explicit_content_height));
        let implicit = if (!style.grid_template_rows.is_empty()
            && rows_synthesized_from_areas
            && !auto_row_pattern.is_empty())
            || (r >= style.grid_template_rows.len() && !auto_row_pattern.is_empty())
        {
            let implicit_idx = if rows_synthesized_from_areas {
                r
            } else {
                r - style.grid_template_rows.len()
            };
            Some(auto_row_pattern[implicit_idx % auto_row_pattern.len()])
        } else {
            None
        };
        *h = explicit.or(implicit).unwrap_or(0.0);
    }
    // Grow rows to fit any item content / explicit item height that exceeds
    // the track height (auto rows, or items taller than their fixed track).
    for p in &placed {
        let r = p.row;
        if p.row_span != 1
            || r >= row_heights.len()
            || row_tracks
                .get(r)
                .and_then(|track| grid_track_fixed_height(track, explicit_content_height))
                .is_some()
        {
            continue;
        }
        let cs = &child_styles[p.idx];
        let track_w = col_widths.iter().skip(p.col).take(p.col_span).sum::<f32>()
            + column_gap * p.col_span.saturating_sub(1) as f32;
        let item_h = grid_item_outer_height(
            cs,
            Some(ctx),
            env,
            &element_children[p.idx],
            &child_ancestors,
            Some((track_w - cs.padding.left - cs.padding.right).max(1.0)),
        );
        if item_h > row_heights[r] {
            row_heights[r] = item_h;
        }
    }

    // Default grid `align-content: normal` resolves to `stretch`: when the grid
    // container has a definite content height larger than the natural row sizes,
    // the surplus is distributed equally among the rows whose track size is NOT a
    // fixed length (auto / implicit / `1fr` tracks). Fixed-length rows keep their
    // size (their surplus, if any, stays as free space — `align-content: start`).
    // Without this, empty cells in a fixed-height container collapse to 0 and
    // vanish, whereas Chrome stretches the single auto row to fill the box.
    if let Some(content_box_target) = explicit_content_height {
        let natural: f32 =
            row_heights.iter().sum::<f32>() + row_gap * num_rows.saturating_sub(1) as f32;
        let surplus = content_box_target - natural;
        if surplus > 0.0 && style.align_content == AlignContent::Stretch {
            // Stretchable rows: those not pinned by a fixed-length template track.
            let stretchable: Vec<usize> = (0..num_rows)
                .filter(|&r| {
                    row_tracks
                        .get(r)
                        .and_then(|track| grid_track_fixed_height(track, explicit_content_height))
                        .is_none()
                })
                .collect();
            if !stretchable.is_empty() {
                let share = surplus / stretchable.len() as f32;
                for &r in &stretchable {
                    row_heights[r] += share;
                }
            }
        }
    }

    let (grid_block_offset, effective_row_gap) = explicit_content_height
        .map(|target| distribute_rows(&row_heights, row_gap, target, style.align_content))
        .unwrap_or((0.0, row_gap));
    let (mut grid_inline_offset, effective_column_gap) =
        distribute_tracks(&col_widths, column_gap, inner_width, style.justify_content);
    if style.writing_mode == crate::style::computed::WritingMode::VerticalRl {
        grid_inline_offset -= style.border.left.width + style.border.right.width;
    }
    if style.direction_rtl {
        let natural_inline = col_widths.iter().sum::<f32>()
            + effective_column_gap * num_cols.saturating_sub(1) as f32;
        grid_inline_offset = match style.justify_content {
            JustifyContent::FlexStart => inner_width - natural_inline,
            JustifyContent::FlexEnd => 0.0,
            _ => grid_inline_offset,
        };
    }

    // Natural content-box height of the grid: the resolved row tracks plus the
    // row gaps between them. With fixed row tracks (no fr/auto growth), this is
    // the height the grid rows actually occupy; any surplus from an explicit
    // container `height` stays as blank free space below the last row (Chrome's
    // default `align-content: start` for definite tracks), rather than being
    // absorbed by stretching the tracks.
    let content_height: f32 = grid_block_offset
        + row_heights.iter().sum::<f32>()
        + effective_row_gap * num_rows.saturating_sub(1) as f32;
    // Honour an explicit container `height` so the container's border-box ends
    // where Chrome paints it (and any free space below the last row is left
    // blank), mirroring the block-level convention where a Container's
    // `block_height` is a border-box value compared against a content height
    // that already includes the border.
    let border_v = style.border.top.width + style.border.bottom.width;
    let block_height = style.height.map(|_| {
        let padding_box_h = super::helpers::resolve_padding_box_height(
            content_height,
            style.height,
            style.padding.top,
            style.padding.bottom,
            border_v,
            style.box_sizing,
        );
        padding_box_h + border_v
    });

    // Helper to compute the x-offset of a column index.
    let col_x = |c: usize| -> f32 {
        col_widths.iter().take(c).sum::<f32>() + effective_column_gap * c as f32
    };
    let span_width = |c: usize, cs: usize| -> f32 {
        let w: f32 = col_widths.iter().skip(c).take(cs).sum();
        w + effective_column_gap * cs.saturating_sub(1) as f32
    };

    // ---- Build one GridRow per grid row --------------------------------
    // Each GridRow holds cells positioned by column (using colspan for the
    // resolved per-cell widths) with min_content_height forcing the row's
    // track height. Items that start on a later row are emitted on that row;
    // multi-row items are approximated by emitting on their starting row with
    // a min height covering the spanned tracks.
    let mut grid_children: Vec<LayoutElement> = Vec::new();
    for row in 0..num_rows {
        let track_h = row_heights[row];
        let mut cells: Vec<TableCell> = Vec::new();
        let mut next_col = 0usize;

        // Items whose top-left lands on this row, in column order.
        let mut row_items: Vec<(&Placed, f32)> = placed
            .iter()
            .filter(|p| {
                p.row == row
                    || (p.row < row
                        && row < p.row + p.row_span
                        && grid_item_has_block_child(&element_children[p.idx]))
            })
            .map(|p| {
                let rows_before = row.saturating_sub(p.row);
                let offset = row_heights
                    .iter()
                    .skip(p.row)
                    .take(rows_before)
                    .sum::<f32>()
                    + effective_row_gap * rows_before as f32;
                (p, offset)
            })
            .collect();
        row_items.sort_by_key(|(p, _)| (p.col, child_styles[p.idx].z_index, p.idx));
        let baseline_offsets: std::collections::HashMap<usize, (f32, f32)> = if style.align_items
            == AlignItems::Baseline
        {
            let mut baselines = Vec::new();
            for (p, _) in &row_items {
                let cs = &child_styles[p.idx];
                let child_el = &element_children[p.idx];
                let has_text = child_el.children.iter().any(|child| match child {
                    DomNode::Text(t) => !t.trim().is_empty(),
                    _ => false,
                });
                if let Some(baseline) = grid_item_first_baseline(cs, has_text, env) {
                    let item_h =
                        grid_item_outer_height(cs, None, env, child_el, &child_ancestors, None);
                    baselines.push((p.idx, baseline, item_h));
                }
            }
            let row_baseline = baselines
                .iter()
                .map(|(_, baseline, _)| *baseline)
                .fold(0.0_f32, f32::max);
            baselines
                .into_iter()
                .map(|(idx, baseline, item_h)| (idx, ((row_baseline - baseline).max(0.0), item_h)))
                .collect()
        } else {
            std::collections::HashMap::new()
        };

        for (p, row_span_offset) in row_items {
            // Definite placements may overlap (two items in one cell). The
            // colspan-based emission cannot represent overlap, so an item whose
            // column was already consumed by an earlier (wider) item on this row
            // is skipped here — it would otherwise shift later columns. (Overlap
            // / z-index stacking is out of scope for the flow model.)
            if p.col < next_col {
                if let Some(cell) = cells.last_mut() {
                    let cs = &child_styles[p.idx];
                    let track_w = span_width(p.col, p.col_span);
                    let spanned_h: f32 = (row..(row + p.row_span).min(row_heights.len()))
                        .map(|r| row_heights[r])
                        .sum::<f32>()
                        + effective_row_gap * (p.row_span.saturating_sub(1)) as f32;
                    let inset =
                        compute_grid_inset(cs, style, track_w, spanned_h).unwrap_or(GridInset {
                            offset_x: 0.0,
                            offset_y: 0.0,
                            width: track_w,
                            height: spanned_h,
                        });
                    let bg = cs
                        .background_color
                        .map(|c: crate::types::Color| c.to_f32_rgba());
                    let BackgroundFields {
                        gradient: background_gradient,
                        radial_gradient: background_radial_gradient,
                        conic_gradient: background_conic_gradient,
                        svg: background_svg,
                        blur_radius: background_blur_radius,
                        size: background_size,
                        position: background_position,
                        repeat: background_repeat,
                        origin: background_origin,
                        clip: background_clip,
                    } = BackgroundFields::from_style(cs);
                    cell.nested_rows.push(LayoutElement::Container {
                        box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
                        children: Vec::new(),
                        background_color: bg,
                        border: LayoutBorder::from_computed(&cs.border),
                        border_radius: cs.border_radius,
                        border_radii: cs.border_radii,
                        border_radii_y: cs.border_radii_y,
                        outline_offset: cs.outline_offset,
                        padding_top: cs.padding.top,
                        padding_bottom: cs.padding.bottom,
                        padding_left: cs.padding.left,
                        padding_right: cs.padding.right,
                        margin_top: inset.offset_y,
                        margin_bottom: 0.0,
                        block_width: Some(inset.width),
                        block_height: Some(inset.height),
                        opacity: cs.opacity,
                        mix_blend_mode: cs.mix_blend_mode,
                        background_blend_mode: cs.background_blend_mode,
                        visible: cs.visibility == Visibility::Visible,
                        float: cs.float,
                        clear: cs.clear,
                        position: cs.position,
                        offset_top: 0.0,
                        offset_left: inset.offset_x,
                        overflow: cs.overflow,
                        overflow_x: cs.overflow_x,
                        overflow_y: cs.overflow_y,
                        transform: cs.transform,
                        transform_origin: cs.transform_origin,
                        clip_path: cs.clip_path.clone(),
                        mask_image: cs.mask_image.clone(),
                        mask_mode: cs.mask_mode,
                        box_shadow: cs.box_shadow.clone(),
                        background_gradient,
                        background_radial_gradient,
                        background_conic_gradient,
                        background_svg,
                        background_blur_radius,
                        background_size,
                        background_position,
                        background_repeat,
                        background_origin,
                        background_clip,
                        outline_width: cs.outline_width,
                        outline_color: cs.outline_color.map(|c| c.to_f32_rgb()),
                        z_index: cs.z_index,
                        positioned_depth: 0,
                        containing_block: None,
                    });
                }
                continue;
            }
            // Pad with empty filler cells up to this item's column.
            while next_col < p.col {
                cells.push(empty_grid_cell(track_h));
                next_col += 1;
            }
            let cs = &child_styles[p.idx];
            let child_el = &element_children[p.idx];

            let track_w = span_width(p.col, p.col_span);
            // Height the item's cell box must occupy in the flow (covers the
            // spanned row tracks plus the gaps between them).
            let spanned_h: f32 = (row..(row + p.row_span).min(row_heights.len()))
                .map(|r| row_heights[r])
                .sum::<f32>()
                + effective_row_gap * (p.row_span.saturating_sub(1)) as f32;

            let cell_inner = (track_w - cs.padding.left - cs.padding.right).max(1.0);
            let runs = if grid_item_has_block_child(child_el) {
                collect_grid_item_leading_runs(cs, env, child_el, &child_ancestors)
            } else {
                collect_grid_item_runs(cs, env, child_el, &child_ancestors)
            };
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner,
                    cs.font_size,
                    resolved_line_height_factor(cs, env.fonts),
                    cs.overflow_wrap,
                )
                .with_rtl(cs.direction_rtl)
                .with_bidi_override(cs.bidi_override),
                env.fonts,
            );

            let bg = cs
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgba());

            // Lay out the grid item's block-level children (e.g. an inner
            // <div>) into nested layout elements so they paint inside the cell,
            // clipped by the cell's `overflow` at paint time. Grid items are
            // block containers; without this, only inline text was collected and
            // a block child (common with `overflow:hidden` to clip it) was
            // dropped entirely.
            let child_column_subgrid = subgrid_track_declaration(
                child_el,
                child_el.style_attr(),
                env.rules,
                &child_ancestors,
                "grid-template-columns",
            )
            .map(|raw| SubgridAxis {
                tracks: col_widths
                    .iter()
                    .skip(p.col)
                    .take(p.col_span)
                    .copied()
                    .collect(),
                gap: effective_column_gap,
                line_names: subgrid_line_names(
                    &effective_column_line_names,
                    p.col,
                    p.col_span,
                    &raw,
                ),
            });
            let child_row_subgrid = subgrid_track_declaration(
                child_el,
                child_el.style_attr(),
                env.rules,
                &child_ancestors,
                "grid-template-rows",
            )
            .map(|raw| SubgridAxis {
                tracks: row_heights
                    .iter()
                    .skip(p.row)
                    .take(p.row_span)
                    .copied()
                    .collect(),
                gap: effective_row_gap,
                line_names: subgrid_line_names(&effective_row_line_names, p.row, p.row_span, &raw),
            });
            let nested_rows = layout_grid_item_children(
                child_el,
                cs,
                ctx,
                &child_ancestors,
                (track_w - cs.padding.left - cs.padding.right - cs.border.horizontal_width())
                    .max(0.0),
                Some(
                    (spanned_h - cs.padding.top - cs.padding.bottom - cs.border.vertical_width())
                        .max(0.0),
                ),
                env,
                Some(SubgridContext {
                    columns: child_column_subgrid,
                    rows: child_row_subgrid,
                }),
            );

            // Per-item alignment: when the item has an explicit smaller size
            // than its track, position the painted box per justify/align-items.
            // A row-spanning item must paint across its spanned tracks without
            // inflating the starting row, so it always carries an explicit
            // inset covering `spanned_h` (the row keeps the single track
            // height via `min_content_height`).
            let mut inset = if p.row_span > 1 && row == p.row {
                Some(
                    compute_grid_inset(cs, style, track_w, spanned_h).unwrap_or(GridInset {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        width: track_w,
                        height: spanned_h,
                    }),
                )
            } else if p.row_span > 1 {
                None
            } else {
                compute_grid_inset(cs, style, track_w, spanned_h)
            };
            if let Some((baseline_offset, item_h)) = baseline_offsets.get(&p.idx).copied() {
                let mut baseline_inset = compute_grid_inset(cs, style, track_w, spanned_h)
                    .unwrap_or(GridInset {
                        offset_x: cs.margin.left,
                        offset_y: 0.0,
                        width: (track_w - cs.margin.left - cs.margin.right).max(0.0),
                        height: item_h.min(spanned_h),
                    });
                baseline_inset.offset_y = cs.margin.top + baseline_offset;
                baseline_inset.height = item_h.min((spanned_h - baseline_offset).max(0.0));
                inset = Some(baseline_inset);
            }
            let cell_min_h = if p.row_span > 1 { track_h } else { spanned_h };

            let mut nested_rows = nested_rows;
            if row_span_offset > 0.0 {
                shift_nested_flow_up(&mut nested_rows, row_span_offset);
            }
            cells.push(TableCell {
                lines,
                nested_rows,
                bold: cs.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: cs.padding.top,
                padding_right: cs.padding.right,
                padding_bottom: cs.padding.bottom,
                padding_left: cs.padding.left,
                colspan: p.col_span.max(1),
                rowspan: 1,
                border: LayoutBorder::from_computed(&cs.border),
                text_align: cs.text_align,
                vertical_align: cs.vertical_align,
                min_content_height: cell_min_h,
                hide_if_empty: false,
                grid_inset: inset,
                clips: cs.overflow.clips() || row_span_offset > 0.0,
                background_gradient: cs.background_gradient.clone(),
                background_radial_gradient: cs.background_radial_gradient.clone(),
                background_conic_gradient: cs.background_conic_gradient.clone(),
            });
            next_col = p.col + p.col_span;
        }

        // Fill trailing columns.
        while next_col < num_cols {
            cells.push(empty_grid_cell(track_h));
            next_col += 1;
        }

        let margin_top = if row == 0 {
            grid_block_offset
        } else {
            effective_row_gap
        };

        grid_children.push(LayoutElement::GridRow {
            cells,
            col_widths: col_widths.clone(),
            gap: effective_column_gap,
            margin_top,
            margin_bottom: 0.0,
            border: LayoutBorder::default(),
            padding_left: grid_inline_offset,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            positioned_depth: 0,
        });
    }
    let _ = col_x;

    // Wrap all grid rows in a Container that carries the border, padding,
    // and background of the grid container element.
    let bg = style
        .background_color
        .map(|c: crate::types::Color| c.to_f32_rgba());
    let BackgroundFields {
        gradient: background_gradient,
        radial_gradient: background_radial_gradient,
        conic_gradient: background_conic_gradient,
        svg: background_svg,
        blur_radius: background_blur_radius,
        size: background_size,
        position: background_position,
        repeat: background_repeat,
        origin: background_origin,
        clip: background_clip,
    } = BackgroundFields::from_style(style);

    // Lay out absolutely-positioned children (out of flow) against the grid
    // container's padding box. The wrapping Container establishes the containing
    // block (recording its padding-box origin under `positioned_depth`), so abs
    // children stamped with this CB anchor correctly via the renderer.
    let establishes_cb = crate::layout::helpers::establishes_containing_block(style);
    let grid_positioned_depth = if establishes_cb { positioned_depth } else { 0 };
    if !abs_child_indices.is_empty() {
        // The containing block for an absolutely-positioned child of a grid
        // container is the grid container's PADDING box (CSS2 §10.1, css-grid-1
        // §9). So `bottom`/`right` insets resolve against the padding-box extent,
        // not the content box — using the content box would place a bottom-anchored
        // box `padding-top + padding-bottom` too high.
        let content_box_height = style
            .height
            .map(|h| {
                if style.box_sizing == BoxSizing::BorderBox {
                    (h - border_v - style.padding.top - style.padding.bottom).max(0.0)
                } else {
                    h
                }
            })
            .unwrap_or(content_height);
        let cb_padding_height = content_box_height + style.padding.top + style.padding.bottom;
        let cb_padding_width = inner_width.max(0.0) + style.padding.left + style.padding.right;
        for &idx in &abs_child_indices {
            let child_el = all_element_children[idx];
            let child_style = &all_child_styles[idx];
            let has_grid_area_cb = child_style.grid_area_name.is_some()
                || child_style.grid_column_start != GridLine::Auto
                || child_style.grid_column_end != GridLine::Auto
                || child_style.grid_row_start != GridLine::Auto
                || child_style.grid_row_end != GridLine::Auto;
            let mut abs_area_inline_offset = 0.0;
            let mut abs_area_block_offset = 0.0;
            let cb = if has_grid_area_cb {
                let abs_placement = place_grid_items(
                    style,
                    std::slice::from_ref(child_style),
                    num_cols,
                    Some(num_cols),
                    None,
                    Some(&effective_column_line_names),
                    Some(&effective_row_line_names),
                );
                if let Some(mut p) = abs_placement.placed.into_iter().next() {
                    if style.direction_rtl {
                        p.col = num_cols.saturating_sub(p.col + p.col_span);
                    }
                    let area_x = grid_inline_offset + col_x(p.col);
                    abs_area_inline_offset = area_x - child_style.left.unwrap_or(0.0) * 2.0;
                    abs_area_block_offset = grid_block_offset
                        + row_heights.iter().take(p.row).sum::<f32>()
                        + effective_row_gap * p.row as f32
                        - child_style.top.unwrap_or(0.0) * 2.0;
                    let area_h = row_heights.iter().skip(p.row).take(p.row_span).sum::<f32>()
                        + effective_row_gap * p.row_span.saturating_sub(1) as f32;
                    ContainingBlock {
                        x: style.padding.left,
                        width: span_width(p.col, p.col_span).max(0.0),
                        height: area_h.max(0.0),
                        depth: grid_positioned_depth,
                    }
                } else {
                    ContainingBlock {
                        x: style.padding.left,
                        width: cb_padding_width,
                        height: cb_padding_height,
                        depth: grid_positioned_depth,
                    }
                }
            } else {
                ContainingBlock {
                    // Padding-box top-left, relative to the wrapping Container's
                    // content origin. The Container seeds abs_origins at its
                    // border-box inner corner (border edge), so an abs child
                    // anchored to the padding box offsets by the container's
                    // padding only (border already folded in).
                    x: style.padding.left,
                    width: cb_padding_width,
                    height: cb_padding_height,
                    depth: grid_positioned_depth,
                }
            };
            let mut abs_ancestors = child_ancestors.clone();
            abs_ancestors.push(AncestorInfo {
                element: child_el,
                child_index: idx,
                sibling_count: total_child_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            let child_ctx = ctx
                .with_parent_and_basis(
                    cb_padding_width.max(0.0),
                    cb_padding_width.max(0.0),
                    Some(cb_padding_height.max(1.0)),
                    style.font_size,
                )
                .with_containing_block(Some(cb));
            let mut buf: Vec<LayoutElement> = Vec::new();
            flatten_element(
                child_el,
                style,
                &child_ctx,
                &mut buf,
                None,
                &abs_ancestors,
                positioned_depth,
                idx,
                total_child_count,
                &[],
                &[],
                env,
            );
            crate::layout::helpers::patch_absolute_children_containing_block(&mut buf, cb);
            if abs_area_inline_offset != 0.0 || abs_area_block_offset != 0.0 {
                shift_absolute_offsets(&mut buf, abs_area_inline_offset, abs_area_block_offset);
            }
            grid_children.extend(buf);
        }
    }

    output.push(LayoutElement::Container {
        box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
        children: grid_children,
        background_color: bg,
        border: LayoutBorder::from_computed(&style.border),
        border_radius: style.border_radius,
        border_radii: style.border_radii,
        border_radii_y: style.border_radii_y,
        outline_offset: style.outline_offset,
        padding_top: style.padding.top,
        padding_bottom: style.padding.bottom,
        padding_left: style.padding.left,
        padding_right: style.padding.right,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
        block_width: Some(border_box_w),
        block_height,
        opacity: style.opacity,
        mix_blend_mode: style.mix_blend_mode,
        background_blend_mode: style.background_blend_mode,
        visible: style.visibility == Visibility::Visible,
        float: style.float,
        clear: style.clear,
        position: style.position,
        offset_top: 0.0,
        offset_left: h_offset,
        overflow: style.overflow,
        overflow_x: style.overflow_x,
        overflow_y: style.overflow_y,
        transform: style.transform,
        transform_origin: style.transform_origin,
        clip_path: style.clip_path.clone(),
        mask_image: style.mask_image.clone(),
        mask_mode: style.mask_mode,
        box_shadow: style.box_shadow.clone(),
        background_gradient,
        background_radial_gradient,
        background_conic_gradient,
        background_svg,
        background_blur_radius,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        background_clip,
        outline_width: style.outline_width,
        outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
        z_index: style.z_index,
        positioned_depth: grid_positioned_depth,
        containing_block: None,
    });
}
