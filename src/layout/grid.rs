use crate::layout::cells::{
    CellAlignment, CellBox, CellBoxModel, CellContent, CellFragmentation, CellPaint, GridCell,
    GridCellPlacement, GridInset, GridPaintOrder,
};
use crate::layout::elements::{
    BlockSize, BoxModel, BoxPaint, Container, GridContent, GridRow, GridRowStartSpace,
    IntoLayoutNode, LayoutElement, LayoutNode, LayoutSize, Positioning, TableSourcePath,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{
    AncestorInfo, CssRule, CssValue, SelectorContext, parse_inline_style, parse_length,
    selector_matches_with_context, specificity,
};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    AlignContent, AlignItems, BoxSizing, ComputedStyle, ContentItem, Display, GridAlign, GridLine,
    GridTrack, JustifyContent, LengthPercent, Overflow, Position, WhiteSpace,
    compute_style_with_context_with_font_metrics, computed_length_percent,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::{EdgeSizes, Point, Size};

use super::box_model::ResolvedBoxDimensions;
use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    ElementSiblingContext, ElementSiblingPosition, LayoutBorder, LayoutTreeContext,
    element_is_empty, flatten_element,
};
use super::inline::layout_inline_mixed_sequence_with_env;
use super::inline_formatting::{
    AnonymousInlineFormattingContext, GeneratedContentStyles, GeneratedInlineContent,
    InlineContentSequence, InlineFormattingContext, InlineFormattingRole,
};
use super::table::TableLayoutContext;
use super::text::{
    InlineRunCollector, TextWrapOptions, has_non_collapsible_text, measure_text_intrinsic_widths,
    parent_line_strut, resolved_line_height_factor, text_run_line_height_factor, used_font_size,
    wrap_text_runs,
};

mod fragmentation;

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
    FitContent(LengthPercent),
    Minmax(TrackBreadth, TrackBreadth),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum IntrinsicContribution {
    #[default]
    Empty,
    Sized(f32),
}

impl IntrinsicContribution {
    const fn size(self) -> f32 {
        match self {
            Self::Empty => 0.0,
            Self::Sized(size) => size,
        }
    }

    const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }

    fn include(&mut self, size: f32) {
        if size <= 0.0 {
            return;
        }
        *self = Self::Sized(self.size().max(size));
    }

    fn grow(&mut self, amount: f32) {
        if amount > 0.0 {
            *self = Self::Sized(self.size() + amount);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct TrackIntrinsicContributions {
    minimum: IntrinsicContribution,
    min_content: IntrinsicContribution,
    max_content: IntrinsicContribution,
}

#[derive(Debug, Clone, Copy)]
enum IntrinsicAxis {
    Minimum,
    MinContent,
    MaxContent,
}

impl TrackIntrinsicContributions {
    const fn get(self, axis: IntrinsicAxis) -> IntrinsicContribution {
        match axis {
            IntrinsicAxis::Minimum => self.minimum,
            IntrinsicAxis::MinContent => self.min_content,
            IntrinsicAxis::MaxContent => self.max_content,
        }
    }

    fn get_mut(&mut self, axis: IntrinsicAxis) -> &mut IntrinsicContribution {
        match axis {
            IntrinsicAxis::Minimum => &mut self.minimum,
            IntrinsicAxis::MinContent => &mut self.min_content,
            IntrinsicAxis::MaxContent => &mut self.max_content,
        }
    }
}

/// The three distinct inline-axis contributions a grid item supplies to track
/// sizing. CSS Grid's `auto` minimum is not interchangeable with min-content;
/// in particular, an explicit `min-width` can be much smaller than a no-wrap
/// item's min-content size.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct GridItemInlineContributions {
    minimum: f32,
    min_content: f32,
    max_content: f32,
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

/// Complete computed style state for one grid item.
///
/// Generated boxes are children of the originating item's principal box. They
/// must therefore travel with the principal style through intrinsic sizing and
/// cell-content layout; retaining only the principal style makes a grid-specific
/// child path silently discard `::before` and `::after`.
#[derive(Debug, Clone)]
struct GridItemStyle {
    principal: ComputedStyle,
    generated: GeneratedContentStyles,
    source_position: ElementSiblingPosition,
    table_source: Option<TableSourcePath>,
}

impl GridItemStyle {
    fn principal_only(principal: ComputedStyle) -> Self {
        Self {
            principal,
            generated: GeneratedContentStyles::default(),
            source_position: ElementSiblingPosition::default(),
            table_source: None,
        }
    }

    fn from_element(
        principal: ComputedStyle,
        element: &ElementNode,
        selector_context: &SelectorContext<'_>,
        env: &LayoutEnv,
    ) -> Self {
        Self {
            generated: GeneratedContentStyles::resolve(
                element,
                &principal,
                env.rules,
                selector_context,
                env.fonts,
            ),
            principal,
            source_position: ElementSiblingPosition::from_selector_context(selector_context),
            table_source: None,
        }
    }

    fn from_flattened_element(
        principal: ComputedStyle,
        element: &ElementNode,
        selector_context: &SelectorContext<'_>,
        table_source: TableSourcePath,
        env: &LayoutEnv,
    ) -> Self {
        let mut item = Self::from_element(principal, element, selector_context, env);
        item.table_source = Some(table_source);
        item
    }

    fn descendant_ancestors<'dom>(
        &self,
        element: &'dom ElementNode,
        ancestors: &[AncestorInfo<'dom>],
    ) -> Vec<AncestorInfo<'dom>> {
        let mut descendants = ancestors.to_vec();
        descendants.push(
            self.source_position
                .ancestor(element, element_is_empty(element)),
        );
        descendants
    }

    fn generated_content<'a>(
        &'a self,
        element: &'a ElementNode,
    ) -> super::inline_formatting::GeneratedInlineContent<'a> {
        self.generated.boxes(element)
    }
}

impl std::ops::Deref for GridItemStyle {
    type Target = ComputedStyle;

    fn deref(&self) -> &Self::Target {
        &self.principal
    }
}

/// Absolute-positioning state inherited by descendants while grid replaces
/// the ordinary DOM traversal with track and cell layout.
///
/// Grid containers and grid items are still principal CSS boxes. Their
/// padding boxes therefore participate in the same positioned-ancestor chain
/// as ordinary blocks, even though neither box is flattened through the block
/// layout path.
#[derive(Debug, Clone, Copy)]
struct GridDescendantPositioning {
    containing_block: Option<ContainingBlock>,
    positioned_depth: usize,
}

impl GridDescendantPositioning {
    fn inherited(ctx: &LayoutContext, positioned_depth: usize) -> Self {
        Self {
            containing_block: ctx.containing_block,
            positioned_depth,
        }
    }

    fn for_container(
        style: &ComputedStyle,
        ctx: &LayoutContext,
        positioned_depth: usize,
        padding_box: Size,
    ) -> Self {
        let inherited = Self::inherited(ctx, positioned_depth);
        if !crate::layout::helpers::establishes_containing_block(style) {
            return inherited;
        }

        Self {
            containing_block: Some(ContainingBlock {
                // Descendants are rendered through a depth-keyed padding-box
                // origin. The local x coordinate is deliberately zero: it is
                // not a second copy of the eventual page-space origin.
                x: 0.0,
                width: padding_box.width,
                height: padding_box.height,
                depth: positioned_depth,
            }),
            positioned_depth,
        }
    }

    fn for_item(self, style: &ComputedStyle, padding_box: Size) -> GridItemPositioning {
        if !crate::layout::helpers::establishes_containing_block(style) {
            return GridItemPositioning {
                descendants: self,
                established_depth: 0,
            };
        }

        let established_depth = self.positioned_depth + 1;
        GridItemPositioning {
            descendants: Self {
                containing_block: Some(ContainingBlock {
                    x: 0.0,
                    width: padding_box.width,
                    height: padding_box.height,
                    depth: established_depth,
                }),
                positioned_depth: established_depth,
            },
            established_depth,
        }
    }
}

/// Positioning state for one concrete grid-item principal box.
#[derive(Debug, Clone, Copy)]
struct GridItemPositioning {
    descendants: GridDescendantPositioning,
    /// Zero means that the item forwards an ancestor's containing block and
    /// must not register its own padding-box origin in the renderer.
    established_depth: usize,
}

/// Geometry and ancestor state offered to a grid item's formatting context.
#[derive(Debug, Clone, Copy)]
struct GridItemContentFrame {
    width: f32,
    height: Option<f32>,
    descendants: GridDescendantPositioning,
}

impl GridItemContentFrame {
    fn inherited(width: f32, height: Option<f32>, ctx: &LayoutContext) -> Self {
        let positioned_depth = ctx
            .containing_block
            .map_or(0, |containing_block| containing_block.depth);
        Self {
            width,
            height,
            descendants: GridDescendantPositioning::inherited(ctx, positioned_depth),
        }
    }

    const fn positioned(content_box: Size, descendants: GridDescendantPositioning) -> Self {
        Self {
            width: content_box.width,
            height: Some(content_box.height),
            descendants,
        }
    }

    fn child_context(self, ctx: &LayoutContext, font_size: f32) -> LayoutContext {
        ctx.with_parent(self.width, self.height, font_size)
            .with_containing_block(self.descendants.containing_block)
    }
}

/// Block-axis constraints that affect grid track sizing.
///
/// A definite height provides a percentage basis. A definite `min-height`
/// does not, but it still supplies the minimum grid area into which auto tracks
/// stretch. Keeping these roles separate prevents a min-height from becoming a
/// hard principal-box height merely to size its tracks.
#[derive(Debug, Clone, Copy, Default)]
struct GridBlockSizing {
    definite_content_height: Option<f32>,
    minimum_content_height: Option<f32>,
}

impl GridBlockSizing {
    fn from_style(style: &ComputedStyle) -> Self {
        let content_height = |height: f32| match style.box_sizing {
            BoxSizing::BorderBox => {
                (height - style.padding.vertical() - style.border.vertical_width()).max(0.0)
            }
            BoxSizing::ContentBox => height.max(0.0),
        };
        Self {
            // A definite preferred size is not the used size until min/max
            // constraints have resolved. Track sizing, content alignment, and
            // the final principal box must all observe the same constrained
            // content box; reusing the authored height here and constraining
            // only the final box makes centered tracks drift by half the clamp.
            definite_content_height: style.height.map(|_| {
                ResolvedBoxDimensions::from_style(style, Size::default())
                    .content
                    .height
            }),
            minimum_content_height: style.min_height.map(content_height),
        }
    }

    fn percentage_basis(self) -> Option<f32> {
        self.definite_content_height.map(|height| {
            self.minimum_content_height
                .map_or(height, |minimum| height.max(minimum))
        })
    }

    fn track_extent(self) -> Option<f32> {
        self.percentage_basis().or(self.minimum_content_height)
    }
}

/// Translation from a grid area's containing-block origin to the grid
/// container's containing-block origin.
///
/// The child's `offset_left` and `offset_top` already contain the resolved CSS
/// insets. Applying this translation must therefore add only the area origin.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct ContainingBlockTranslation {
    inline: f32,
    block: f32,
}

/// Used border/content boxes and alignment placement for one grid item.
///
/// Grid track sizing offers an alignment area; the item's authored preferred,
/// minimum, and maximum sizes resolve inside that area before self-alignment is
/// applied. Keeping the resolved boxes with their placement prevents paint,
/// descendant layout, and row measurement from independently interpreting the
/// same width constraints.
#[derive(Debug, Clone, Copy)]
struct GridItemGeometry {
    dimensions: ResolvedBoxDimensions,
    inset: GridInset,
    fills_track: bool,
}

impl GridItemGeometry {
    fn placement(self) -> Option<GridInset> {
        (!self.fills_track).then_some(self.inset)
    }
}

impl ContainingBlockTranslation {
    const fn new(inline: f32, block: f32) -> Self {
        Self { inline, block }
    }

    const fn is_identity(self) -> bool {
        self.inline == 0.0 && self.block == 0.0
    }
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
/// - Auto: sized to the column's recorded max-content contribution. When the
///   sum of fixed + auto exceeds the available space, auto columns shrink
///   proportionally.
/// - Fr(v) / Minmax(min, max): flexible tracks. The space left after the
///   fixed/percent/auto tracks is divided among them by the CSS Grid
///   "find the size of an fr" algorithm — each flexible track resolves to
///   `flex_size × flex_factor`, floored at its base (the automatic minimum for
///   a bare `fr`, the authored `min` for a `minmax`) and capped at its `max`, with `flex_size` found by
///   iteratively freezing clamped tracks. Equal `fr` peers therefore resolve
///   to equal widths even when their `minmax` minimums differ. If no flexible
///   tracks exist and slack remains, Auto columns absorb it (so `auto auto`
///   fills the row like Chrome does).
///
/// `intrinsic` records both min-content and max-content contributions for each
/// track. Its explicit `Empty` state is distinct from every positive subpoint
/// size; geometry is never used as an occupancy flag.
fn resolve_grid_columns(
    tracks: &[RuntimeTrack],
    available_width: f32,
    gap: f32,
    intrinsic: &[TrackIntrinsicContributions],
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

    let minimum = |i: usize| -> f32 { intrinsic.get(i).map_or(0.0, |track| track.minimum.size()) };
    let min_content = |i: usize| -> f32 {
        intrinsic
            .get(i)
            .map_or(0.0, |track| track.min_content.size())
    };
    let max_content = |i: usize| -> f32 {
        intrinsic
            .get(i)
            .map_or(0.0, |track| track.max_content.size())
    };
    let breadth = |b: TrackBreadth, i: usize, percent_basis: f32| -> f32 {
        match b {
            TrackBreadth::Fixed(v) => v,
            TrackBreadth::Percent(p) => p * percent_basis,
            TrackBreadth::Fr(_) => 0.0,
            TrackBreadth::Auto => minimum(i),
            TrackBreadth::MinContent => min_content(i),
            TrackBreadth::MaxContent => max_content(i),
        }
    };
    let max_breadth = |b: TrackBreadth, i: usize, percent_basis: f32| -> f32 {
        match b {
            TrackBreadth::Fixed(v) => v,
            TrackBreadth::Percent(p) => p * percent_basis,
            TrackBreadth::Fr(_) => f32::MAX,
            TrackBreadth::Auto | TrackBreadth::MaxContent => max_content(i),
            TrackBreadth::MinContent => min_content(i),
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
                auto_total += max_content(i);
                auto_count += 1;
            }
            RuntimeTrack::MinContent => fixed_total += min_content(i),
            RuntimeTrack::MaxContent => fixed_total += max_content(i),
            RuntimeTrack::FitContent(limit) => {
                fixed_total += max_content(i)
                    .min(limit.resolve(available_width))
                    .max(minimum(i));
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
        // minimum (the automatic minimum for a bare `fr`, the `min` for a
        // `minmax`) and no larger
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
                    // A flex value outside minmax is defined as
                    // `minmax(auto, <flex>)`, not `minmax(0, <flex>)`.
                    // Its automatic minimum is the track's min-content
                    // contribution (css-grid-1 §7.2.4).
                    base: minimum(i),
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
                RuntimeTrack::Auto => max_content(i) * auto_shrink_scale,
                RuntimeTrack::MinContent => min_content(i),
                RuntimeTrack::MaxContent => max_content(i),
                RuntimeTrack::FitContent(limit) => max_content(i)
                    .min(limit.resolve(available_width))
                    .max(minimum(i)),
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
            RuntimeTrack::Auto => max_content(i) * auto_shrink_scale + auto_extra,
            RuntimeTrack::MinContent => min_content(i),
            RuntimeTrack::MaxContent => max_content(i),
            RuntimeTrack::FitContent(limit) => max_content(i)
                .min(limit.resolve(available_width))
                .max(minimum(i)),
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
        Some(CssValue::Number(0.0)) => Some(0.0),
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

struct RuntimeTrackParser<'a> {
    style: &'a ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'a>,
}

impl<'a> RuntimeTrackParser<'a> {
    fn new(style: &'a ComputedStyle, available_width: f32, font_metrics: FontMetrics<'a>) -> Self {
        Self {
            style,
            length_context: crate::style::resolve::LengthResolutionContext::new(
                available_width,
                style.math_unit_context(font_metrics),
            ),
            font_metrics,
        }
    }

    fn length_percent(&self, token: &str) -> Option<LengthPercent> {
        let value = parse_length(token.trim())?;
        let length =
            computed_length_percent(&value, self.style, self.length_context, self.font_metrics)?;
        (length.resolve(self.length_context.percentage_basis) >= 0.0).then_some(length)
    }
}

fn parse_runtime_track(token: &str, parser: &RuntimeTrackParser<'_>) -> Option<RuntimeTrack> {
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
        parser.length_percent(inner).map(RuntimeTrack::FitContent)
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

fn track_min_for_auto_repeat(track: RuntimeTrack, available_width: f32) -> f32 {
    match track {
        RuntimeTrack::Fixed(v) => v,
        RuntimeTrack::Percent(_) => 0.0,
        RuntimeTrack::Fr(_) | RuntimeTrack::Auto => 0.0,
        RuntimeTrack::MinContent | RuntimeTrack::MaxContent => 0.0,
        RuntimeTrack::FitContent(limit) => limit.resolve(available_width),
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
        .map(|t| track_min_for_auto_repeat(*t, available_width))
        .sum::<f32>()
        + gap * pattern.len().saturating_sub(1) as f32;
    let repeat_stride = pattern_width + gap;
    if repeat_stride <= 0.0 {
        1
    } else {
        ((available_width + gap) / repeat_stride).floor().max(1.0) as usize
    }
}

fn parse_runtime_track_list(
    value: &str,
    available_width: f32,
    gap: f32,
    parser: &RuntimeTrackParser<'_>,
) -> RuntimeTrackList {
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
                let pattern = parse_runtime_track_list(&parts[1], available_width, gap, parser);
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
        } else if let Some(track) = parse_runtime_track(token, parser) {
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
    ancestors: &[AncestorInfo],
    property: &str,
    available_width: f32,
    gap: f32,
    env: &LayoutEnv,
) -> RuntimeTrackList {
    if let Some(raw) =
        winning_grid_track_declaration(el, style_attr, env.rules, ancestors, property)
    {
        let parser = RuntimeTrackParser::new(style, available_width, env.font_metrics());
        let parsed = parse_runtime_track_list(&raw, available_width, gap, &parser);
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

fn translate_absolute_offsets(
    elements: &mut [LayoutNode],
    translation: ContainingBlockTranslation,
) {
    for element in elements {
        translate_absolute_offset(element.as_mut(), translation);
    }
}

fn translate_absolute_offset(
    element: &mut dyn LayoutElement,
    translation: ContainingBlockTranslation,
) {
    if let Some(owner) = element.positioning_owner_mut() {
        let positioning = owner.positioning_mut();
        if positioning.scheme.is_absolute() {
            positioning.insets.left += translation.inline;
            positioning.insets.top += translation.block;
        }
    }
    element.visit_children_mut(&mut |child| translate_absolute_offset(child, translation));
}

fn shift_nested_flow_up(elements: &mut [LayoutNode], amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let Some(first) = elements.first_mut() else {
        return;
    };
    if let Some(holder) = first.margin_holder_mut() {
        holder.margins_mut().start -= amount;
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
    cs: &GridItemStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> Vec<super::engine::TextRun> {
    let mut runs = Vec::new();
    let descendant_ancestors = cs.descendant_ancestors(child_el, ancestors);
    InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources).collect(
        InlineContentSequence::with_generated(&child_el.children, cs.generated_content(child_el)),
        &cs.principal,
        &mut runs,
        None,
        &descendant_ancestors,
    );
    runs
}

fn grid_item_has_block_child(child_el: &ElementNode) -> bool {
    child_el.children.iter().any(|child| match child {
        DomNode::Element(el) => el.tag.is_block(),
        DomNode::Text(_) => false,
    })
}

fn collect_grid_item_leading_runs(
    cs: &GridItemStyle,
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
    let mut runs = Vec::new();
    let descendant_ancestors = cs.descendant_ancestors(child_el, ancestors);
    InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources).collect(
        InlineContentSequence::with_generated(
            &leading.children,
            GeneratedInlineContent::from_boxes(cs.generated_content(child_el).before(), None),
        ),
        &cs.principal,
        &mut runs,
        None,
        &descendant_ancestors,
    );
    runs
}

fn grid_item_intrinsic_widths(
    cs: &GridItemStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> GridItemInlineContributions {
    if let Some(width) = grid_item_definite_outer_width(cs) {
        return GridItemInlineContributions {
            minimum: width,
            min_content: width,
            max_content: width,
        };
    }
    let counter_checkpoint = env.counter_state.clone();
    let counter_scope = env.counter_state.enter_element(&cs.principal);
    let runs = if grid_item_has_block_child(child_el) {
        collect_grid_item_leading_runs(cs, env, child_el, ancestors)
    } else {
        collect_grid_item_runs(cs, env, child_el, ancestors)
    };
    env.counter_state.leave_element(counter_scope);
    *env.counter_state = counter_checkpoint;
    let intrinsic = measure_text_intrinsic_widths(
        runs,
        TextWrapOptions::new(
            f32::MAX,
            used_font_size(cs, env.fonts),
            text_run_line_height_factor(cs, env.fonts),
            cs.overflow_wrap,
        )
        .with_white_space(cs.white_space)
        .with_parent_strut(parent_line_strut(cs, env.fonts))
        .with_rtl(cs.direction_rtl)
        .with_bidi_override(cs.bidi_override)
        .with_bidi_plaintext(cs.bidi_plaintext)
        .with_word_break_keep_all(cs.word_break_keep_all)
        .with_hyphens_manual(cs.hyphens_manual),
        !matches!(cs.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre),
        env.fonts,
    );
    let extras = cs.padding.horizontal() + cs.border.horizontal_width() + cs.margin.horizontal();
    let min_content = intrinsic.min_content + extras;
    let max_content = (intrinsic.max_content + extras).max(min_content);
    let minimum = cs.min_width.map_or_else(
        || {
            if matches!(cs.overflow_x, Overflow::Scroll | Overflow::Auto) {
                extras
            } else {
                min_content
            }
        },
        |width| grid_item_outer_width_for_specified_size(cs, width),
    );

    GridItemInlineContributions {
        minimum,
        min_content: min_content.max(minimum),
        max_content: max_content.max(minimum),
    }
}

fn grid_item_outer_width_for_specified_size(style: &ComputedStyle, width: f32) -> f32 {
    let decorations = style.padding.horizontal() + style.border.horizontal_width();
    let border_box = match style.box_sizing {
        BoxSizing::ContentBox => width + decorations,
        BoxSizing::BorderBox => width.max(decorations),
    };
    border_box + used_grid_item_margins(style).horizontal()
}

fn grid_item_definite_outer_width(style: &ComputedStyle) -> Option<f32> {
    style.width?;
    Some(
        ResolvedBoxDimensions::from_style(style, Size::default())
            .border_box
            .width
            + used_grid_item_margins(style).horizontal(),
    )
}

/// Physical margins that participate in a grid item's outer contribution.
/// Auto margins absorb free alignment space and therefore contribute zero to
/// intrinsic track sizing.
fn used_grid_item_margins(style: &ComputedStyle) -> EdgeSizes {
    EdgeSizes::new(
        if style.margin_top_auto {
            0.0
        } else {
            style.margin.top
        },
        if style.margin_right_auto {
            0.0
        } else {
            style.margin.right
        },
        if style.margin_bottom_auto {
            0.0
        } else {
            style.margin.bottom
        },
        if style.margin_left_auto {
            0.0
        } else {
            style.margin.left
        },
    )
}

fn is_intrinsic_column_track(track: RuntimeTrack) -> bool {
    matches!(
        track,
        RuntimeTrack::Fr(_) // implicit `minmax(auto, <flex>)`
            | RuntimeTrack::Auto
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
    intrinsic: &mut [TrackIntrinsicContributions],
    tracks: &[RuntimeTrack],
    start: usize,
    span: usize,
    axis: IntrinsicAxis,
    contribution: f32,
) {
    let end = (start + span).min(intrinsic.len()).min(tracks.len());
    if start >= end {
        return;
    }
    let current = intrinsic[start..end]
        .iter()
        .map(|track| track.get(axis).size())
        .sum::<f32>();
    if contribution <= current {
        return;
    }
    let growable_count = (start..end)
        .filter(|&i| is_intrinsic_column_track(tracks[i]))
        .count();
    if growable_count == 0 {
        return;
    }
    let empty_count = (start..end)
        .filter(|&i| is_intrinsic_column_track(tracks[i]) && intrinsic[i].get(axis).is_empty())
        .count();
    let last_empty = (start..end)
        .rfind(|&i| is_intrinsic_column_track(tracks[i]) && intrinsic[i].get(axis).is_empty());
    let recipient_count = if empty_count == 0 { growable_count } else { 1 };
    let share = (contribution - current) / recipient_count as f32;
    for i in start..end {
        if !is_intrinsic_column_track(tracks[i]) {
            continue;
        }
        let is_empty = intrinsic[i].get(axis).is_empty();
        let receives = match empty_count {
            0 => true,
            1 => is_empty,
            _ => Some(i) == last_empty,
        };
        if receives {
            intrinsic[i].get_mut(axis).grow(share);
        }
    }
}

/// The outer block-axis contribution of a grid item after preferred, minimum,
/// and maximum sizing constraints have resolved against its natural border box.
fn grid_item_outer_height(
    cs: &GridItemStyle,
    ctx: Option<&LayoutContext>,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
    content_width: Option<f32>,
) -> f32 {
    let content_h = if cs.height.is_some() {
        0.0
    } else if let (Some(ctx), Some(width)) = (ctx, content_width) {
        let content =
            measure_grid_item_content(child_el, cs, ctx, ancestors, width, None, env, None);
        content.lines.iter().map(|line| line.height).sum::<f32>()
            + content
                .children
                .iter()
                .map(|element| super::paginate::estimate_element_height(element.as_ref()))
                .sum::<f32>()
    } else {
        let counter_checkpoint = env.counter_state.clone();
        let counter_scope = env.counter_state.enter_element(&cs.principal);
        let runs = collect_grid_item_runs(cs, env, child_el, ancestors);
        env.counter_state.leave_element(counter_scope);
        *env.counter_state = counter_checkpoint;
        if runs.is_empty() {
            0.0
        } else {
            cs.font_size * text_run_line_height_factor(cs, env.fonts)
        }
    };
    // Border-box auto height includes the border: an empty bordered item still
    // reserves its border thickness. Without it, the implicit auto track sizes to
    // 0 and a later border stroke emits a negative-height rect.
    let natural_border_box = content_h + cs.padding.vertical() + cs.border.vertical_width();
    ResolvedBoxDimensions::from_style(cs, Size::new(0.0, natural_border_box))
        .border_box
        .height
        + used_grid_item_margins(cs).vertical()
}

fn grid_item_first_baseline(cs: &ComputedStyle, has_text: bool, env: &LayoutEnv) -> Option<f32> {
    if !has_text {
        return None;
    }
    // Grid first-baseline alignment must use the same resolved strut that text
    // layout uses to paint the line. A font-size ratio is neither font-specific
    // nor CSS line-height aware, and makes differently sized grid items miss a
    // shared baseline.
    Some(cs.border.top.used_width() + cs.padding.top + parent_line_strut(cs, env.fonts).above)
}

/// Lay out a grid item's complete principal content.
///
/// A grid item establishes an independent formatting context. Its text and
/// atomic inline children therefore have to be classified together before the
/// result is split into the cell's line and nested-child storage. Returning one
/// [`CellContent`] prevents the text collector and block-child collector from
/// assigning the same source sequence to incompatible formatting contexts.
#[allow(clippy::too_many_arguments)]
fn layout_grid_item_content(
    item_el: &ElementNode,
    item_style: &GridItemStyle,
    ctx: &LayoutContext,
    item_ancestors: &[AncestorInfo],
    frame: GridItemContentFrame,
    env: &mut LayoutEnv,
    subgrid: Option<SubgridContext>,
) -> CellContent {
    let counter_scope = env.counter_state.enter_element(&item_style.principal);
    let content = if let Some(source_path) = &item_style.table_source {
        let mut scoped_env = env.for_table_source(source_path);
        layout_grid_item_content_inner(
            item_el,
            item_style,
            ctx,
            item_ancestors,
            frame,
            &mut scoped_env,
            subgrid,
        )
    } else {
        layout_grid_item_content_inner(
            item_el,
            item_style,
            ctx,
            item_ancestors,
            frame,
            env,
            subgrid,
        )
    };
    env.counter_state.leave_element(counter_scope);
    content
}

/// Intrinsic measurement must observe generated counters without consuming
/// them. The actual cell-content pass owns the source-order counter mutation.
#[allow(clippy::too_many_arguments)]
fn measure_grid_item_content(
    item_el: &ElementNode,
    item_style: &GridItemStyle,
    ctx: &LayoutContext,
    item_ancestors: &[AncestorInfo],
    content_width: f32,
    content_height: Option<f32>,
    env: &mut LayoutEnv,
    subgrid: Option<SubgridContext>,
) -> CellContent {
    let counter_checkpoint = env.counter_state.clone();
    let content = layout_grid_item_content(
        item_el,
        item_style,
        ctx,
        item_ancestors,
        GridItemContentFrame::inherited(content_width, content_height, ctx),
        env,
        subgrid,
    );
    *env.counter_state = counter_checkpoint;
    content
}

#[allow(clippy::too_many_arguments)]
fn layout_grid_item_content_inner(
    item_el: &ElementNode,
    item_style: &GridItemStyle,
    ctx: &LayoutContext,
    item_ancestors: &[AncestorInfo],
    frame: GridItemContentFrame,
    env: &mut LayoutEnv,
    subgrid: Option<SubgridContext>,
) -> CellContent {
    use crate::style::computed::Display;

    let mut out: Vec<LayoutNode> = Vec::new();
    // A grid item is a block container. This child context owns both its inline
    // formatting sequence and any nested block formatting contexts.
    let content_width = frame.width;
    let content_height = frame.height;
    let child_ctx = frame.child_context(ctx, item_style.font_size);

    let child_ancestors = item_style.descendant_ancestors(item_el, item_ancestors);

    // A grid item that is itself a flex or grid container must arrange its OWN
    // children via that formatting context, not flow them as independent blocks.
    // Lay it out through the matching container path against the item's content
    // box (`content_width`), so e.g. a `display:flex` cell distributes its boxes
    // along the main axis instead of stacking them block-by-block.
    if matches!(
        item_style.display,
        Display::Flex
            | Display::InlineFlex
            | Display::Grid
            | Display::InlineGrid
            | Display::Table
            | Display::InlineTable
    ) {
        // Give the inner container exactly the item's content-box width so flex
        // main-axis distribution / grid track sizing resolve correctly.
        let mut inner_style = item_style.principal.clone();
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
                (h - item_style.border.vertical_width() - item_style.padding.vertical()).max(0.0)
            } else {
                h
            };
            inner_style.height = Some(content_h);
        } else if let Some(content_h) = content_height {
            inner_style.height = Some(content_h);
        }
        match item_style.display {
            Display::Flex | Display::InlineFlex => {
                crate::layout::flex::layout_flex_container(
                    item_el,
                    &inner_style,
                    &child_ctx,
                    &mut out,
                    &child_ancestors,
                    item_style.generated_content(item_el),
                    frame.descendants.positioned_depth,
                    env,
                );
            }
            Display::Grid | Display::InlineGrid => {
                layout_grid_container_inner(
                    item_el,
                    &inner_style,
                    &child_ctx,
                    &mut out,
                    &child_ancestors,
                    frame.descendants.positioned_depth,
                    env,
                    subgrid,
                );
            }
            Display::Table | Display::InlineTable => {
                inner_style.display = Display::Table;
                let source = item_style.source_position.as_context();
                crate::layout::table::flatten_table(
                    item_el,
                    &inner_style,
                    &mut out,
                    GeneratedInlineContent::new(
                        item_el,
                        item_style.generated.before(),
                        item_style.generated.after(),
                    ),
                    env,
                    TableLayoutContext::new(
                        &child_ctx,
                        item_ancestors,
                        source,
                        frame.descendants.positioned_depth,
                    ),
                );
            }
            _ => {}
        }
        return CellContent {
            lines: Vec::new(),
            children: out,
        };
    }

    let inline_sequence = InlineContentSequence::with_generated(
        &item_el.children,
        item_style.generated_content(item_el),
    );
    if InlineFormattingContext::new(item_style, env.rules, &child_ancestors, env.font_metrics())
        .requires_atomic_layout(inline_sequence)
        && layout_inline_mixed_sequence_with_env(
            inline_sequence,
            item_style,
            &child_ctx,
            &mut out,
            &child_ancestors,
            env,
        )
    {
        return CellContent {
            lines: Vec::new(),
            children: out,
        };
    }

    let runs = if grid_item_has_block_child(item_el) {
        collect_grid_item_leading_runs(item_style, env, item_el, item_ancestors)
    } else {
        collect_grid_item_runs(item_style, env, item_el, item_ancestors)
    };
    let lines = wrap_text_runs(
        runs,
        TextWrapOptions::new(
            content_width,
            used_font_size(item_style, env.fonts),
            text_run_line_height_factor(item_style, env.fonts),
            item_style.overflow_wrap,
        )
        .with_white_space(item_style.white_space)
        .with_parent_strut(parent_line_strut(item_style, env.fonts))
        .with_rtl(item_style.direction_rtl)
        .with_bidi_override(item_style.bidi_override),
        env.fonts,
    );

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
                    if has_non_collapsible_text(text) {
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
                            LayoutTreeContext::new(item_style, &child_ctx, &child_ancestors)
                                .with_positioned_ancestor_depth(frame.descendants.positioned_depth)
                                .for_element(
                                    ElementSiblingContext::new(element_idx, sibling_count)
                                        .with_neighbors(&preceding, &[]),
                                ),
                            &mut out,
                            env,
                        );
                    }
                }
            }
            continue;
        };
        let idx = element_idx;
        element_idx += 1;
        // The shared inline-formatting classifier is authoritative here too:
        // text and atomic inline boxes belong to the cell's line content;
        // only an outside (block-level) participant becomes a nested row.
        let child_style = compute_style_with_context_with_font_metrics(
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
            env.font_metrics(),
        );
        let role = InlineFormattingRole::of(child_el, &child_style);
        if matches!(
            role,
            InlineFormattingRole::Outside | InlineFormattingRole::OutOfFlow
        ) {
            after_block |= role == InlineFormattingRole::Outside;
            crate::layout::engine::flatten_element(
                child_el,
                LayoutTreeContext::new(item_style, &child_ctx, &child_ancestors)
                    .with_positioned_ancestor_depth(frame.descendants.positioned_depth)
                    .for_element(
                        ElementSiblingContext::new(idx, sibling_count)
                            .with_neighbors(&preceding, &[]),
                    ),
                &mut out,
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
    if grid_item_has_block_child(item_el) {
        let mut after_runs = Vec::new();
        item_style.generated_content(item_el).append_after(
            &mut after_runs,
            env.fonts,
            env.counter_state,
            &mut *env.resources,
        );
        if let Some(after) =
            AnonymousInlineFormattingContext::new(&item_style.principal, content_width, env.fonts)
                .layout_runs(after_runs)
        {
            out.push(after);
        }
    }
    CellContent {
        lines,
        children: out,
    }
}

/// An empty filler cell that still occupies the track height so the grid row
/// keeps its geometry when an item is absent in that column.
fn empty_grid_cell(column_start: usize, track_h: f32) -> GridCell {
    GridCell {
        layout: CellBox {
            box_model: CellBoxModel {
                minimum_block_size: track_h,
                ..Default::default()
            },
            ..Default::default()
        },
        placement: GridCellPlacement {
            column_start,
            ..Default::default()
        },
    }
}

/// Resolve a grid item's used boxes and its alignment within a track cell.
fn compute_grid_item_geometry(
    cs: &ComputedStyle,
    container: &ComputedStyle,
    track_w: f32,
    track_h: f32,
) -> GridItemGeometry {
    // Per-item `justify-self` / `align-self` override the container's
    // `justify-items` / `align-items` (CSS Grid §10.x / box-alignment).
    let justify = cs.grid_justify_self.unwrap_or(container.justify_items);
    let align = cs.grid_align_self.unwrap_or(container.grid_align_items);

    let margins = used_grid_item_margins(cs);
    let margin_w = margins.horizontal();
    let margin_h = margins.vertical();
    let align_w = (track_w - margin_w).max(0.0);
    let align_h = (track_h - margin_h).max(0.0);
    let dimensions = ResolvedBoxDimensions::from_style(cs, Size::new(align_w, align_h));
    let box_w = dimensions.border_box.width;
    let box_h = dimensions.border_box.height;

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
    let inset = GridInset {
        offset: Point::new(margins.left + offset_x, margins.top + offset_y),
        size: dimensions.border_box,
    };
    GridItemGeometry {
        dimensions,
        inset,
        fills_track: margin_w == 0.0
            && margin_h == 0.0
            && inset.offset == Point::default()
            && inset.size == Size::new(track_w, track_h),
    }
}

#[cfg(test)]
fn compute_grid_inset(
    cs: &ComputedStyle,
    container: &ComputedStyle,
    track_w: f32,
    track_h: f32,
) -> Option<GridInset> {
    compute_grid_item_geometry(cs, container, track_w, track_h).placement()
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

/// Build a `name -> ordered 0-based line indices` map for one axis. A plain
/// named-line reference resolves to the first entry, while a named span must
/// count every matching line in the search direction (CSS Grid §8.3).
/// `track_line_names[i]` holds the names declared at line `i`. The
/// `grid-template-areas` of the container also generate implicit
/// `<area>-start` / `<area>-end` line names on the relevant axis.
fn build_line_name_map(
    track_line_names: &[Vec<String>],
    area_lines: &[(String, usize)],
    final_line_hint: usize,
) -> std::collections::HashMap<String, Vec<usize>> {
    let mut map: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for (line_idx, names) in track_line_names.iter().enumerate() {
        for n in names {
            let matching_lines = map.entry(n.clone()).or_default();
            // A named span counts grid lines, not duplicate occurrences of the
            // same custom-ident on one line. Duplicate metadata can also arise
            // when computed and runtime track sources describe the same line.
            if matching_lines.last().copied() != Some(line_idx) {
                matching_lines.push(line_idx);
            }
        }
    }
    let final_line = final_line_hint.max(track_line_names.len().saturating_sub(1));
    let starts: Vec<String> = map
        .keys()
        .filter_map(|name| name.strip_suffix("-start").map(ToString::to_string))
        .collect();
    for name in starts {
        let end = format!("{name}-end");
        map.entry(end).or_insert_with(|| vec![final_line]);
    }
    // Implicit area lines fill in any names not already declared explicitly.
    for (name, line_idx) in area_lines {
        map.entry(name.clone()).or_insert_with(|| vec![*line_idx]);
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
    names: &std::collections::HashMap<String, Vec<usize>>,
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
        GridLine::Named(name) => names.get(name).and_then(|lines| lines.first().copied()),
        GridLine::Auto | GridLine::Span(_) | GridLine::SpanNamed { .. } => None,
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
    names: &std::collections::HashMap<String, Vec<usize>>,
) -> Option<(usize, usize)> {
    let span_of = |g: &GridLine| -> Option<usize> {
        match g {
            GridLine::Span(n) => Some((*n).max(1)),
            GridLine::SpanNamed { count, .. } => Some((*count).max(1)),
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
                GridLine::SpanNamed { count, name } => {
                    let lines = names.get(name).map(Vec::as_slice).unwrap_or(&[]);
                    if let Some(line) = lines
                        .iter()
                        .copied()
                        .filter(|line| *line > s)
                        .nth(count.saturating_sub(1))
                    {
                        line - s
                    } else {
                        let found = lines.iter().filter(|line| **line > s).count();
                        let remaining = count.saturating_sub(found).max(1);
                        explicit_tracks.max(s) + remaining - s
                    }
                }
                _ => span_of(end).unwrap_or(1),
            };
            Some((s, span))
        }
        (None, Some(e)) => {
            // end definite; start is span (count back) or auto (→ span 1).
            let span = match start {
                GridLine::SpanNamed { count, name } => names
                    .get(name)
                    .and_then(|lines| {
                        lines
                            .iter()
                            .rev()
                            .copied()
                            .filter(|line| *line < e)
                            .nth(count.saturating_sub(1))
                    })
                    .map(|line| e - line)
                    .unwrap_or_else(|| (*count).max(1).min(e.max(1))),
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
    child_styles: &[GridItemStyle],
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
    // Callers pass the effective (already merged) runtime line-name lists.
    // Treat them as true overrides: merging them with the computed lists again
    // duplicates every name at the same line, so `span 2 <name>` can mistake a
    // duplicate of the first matching line for the second matching line.
    let column_line_names =
        column_line_names_override.unwrap_or(&container.grid_template_column_line_names);
    let row_line_names = row_line_names_override.unwrap_or(&container.grid_template_row_line_names);
    let col_names = build_line_name_map(column_line_names, &col_area_lines, explicit_cols);
    let row_names = build_line_name_map(row_line_names, &row_area_lines, explicit_rows);

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
    output: &mut Vec<LayoutNode>,
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
    output: &mut Vec<LayoutNode>,
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
    let border_pad_w = style.border.horizontal_width() + style.padding.horizontal();
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
                style.border.horizontal_width()
            } else {
                0.0
            };
            available_width
                - style.margin.horizontal()
                - style.padding.horizontal()
                - auto_border_adjust
        }
    };
    // The container's border-box width (used for the wrapping Container's
    // block width and to resolve horizontal margin / auto-centering).
    let border_box_w = inner_width + border_pad_w;
    let h_offset = crate::layout::elements::InlineOffset::resolve_block_start(
        style,
        available_width,
        border_box_w,
    )
    .value();
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
                child_el
                    .class_list()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
        })
        .collect();
    let all_child_styles: Vec<GridItemStyle> = all_element_children
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
            let principal = compute_style_with_context_with_font_metrics(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
                env.font_metrics(),
            );
            GridItemStyle::from_element(principal, child_el, &selector_ctx, env)
        })
        .collect();

    // Per CSS Grid §9.1, an absolutely-positioned child of a grid container is
    // NOT a grid item; it is taken out of flow and laid out against the grid
    // container's padding box. Separate such children out so they don't consume
    // grid tracks, then emit them as positioned boxes inside the wrapping
    // Container (which establishes the containing block).
    let abs_child_indices: Vec<usize> = (0..total_child_count)
        .filter(|&i| all_child_styles[i].position.is_absolute())
        .collect();
    let mut element_children: Vec<ElementNode> = Vec::new();
    let mut child_styles: Vec<GridItemStyle> = Vec::new();

    let container_selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let generated_styles =
        GeneratedContentStyles::resolve(el, style, env.rules, &container_selector_ctx, env.fonts);
    if let Some(before_style) = generated_styles.before() {
        element_children.push(pseudo_element_node(before_style));
        child_styles.push(GridItemStyle::principal_only(before_style.clone()));
    }

    let mut element_idx = 0usize;
    for child in &el.children {
        match child {
            DomNode::Text(text) => {
                if has_non_collapsible_text(text) {
                    let mut node = ElementNode::new(crate::parser::dom::HtmlTag::Span);
                    node.children.push(DomNode::Text(text.clone()));
                    element_children.push(node);
                    child_styles.push(GridItemStyle::principal_only(anonymous_grid_item_style(
                        style,
                    )));
                }
            }
            DomNode::Element(child_el) => {
                let direct_idx = element_idx;
                element_idx += 1;
                let direct_style = &all_child_styles[direct_idx];
                if direct_style.position.is_absolute() {
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
                        let flat_selector_context = SelectorContext {
                            ancestors: wrapper_ancestors.clone(),
                            child_index: flat_idx,
                            sibling_count: flattened_count,
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        };
                        let flat_style = compute_style_with_context_with_font_metrics(
                            flat_el.tag,
                            flat_el.style_attr(),
                            direct_style,
                            env.rules,
                            flat_el.tag_name(),
                            &flat_classes,
                            flat_el.id(),
                            &flat_el.attributes,
                            &flat_selector_context,
                            env.font_metrics(),
                        );
                        if !flat_style.position.is_absolute() {
                            element_children.push(flat_el.clone());
                            let table_source = TableSourcePath::new(
                                flat_selector_context
                                    .ancestors
                                    .iter()
                                    .map(|ancestor| ancestor.child_index)
                                    .chain(std::iter::once(flat_idx)),
                            );
                            child_styles.push(GridItemStyle::from_flattened_element(
                                flat_style,
                                flat_el,
                                &flat_selector_context,
                                table_source,
                                env,
                            ));
                        }
                    }
                } else {
                    element_children.push((*child_el).clone());
                    child_styles.push(direct_style.clone());
                }
            }
        }
    }

    if let Some(after_style) = generated_styles.after() {
        element_children.push(pseudo_element_node(after_style));
        child_styles.push(GridItemStyle::principal_only(after_style.clone()));
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
        ancestors,
        "grid-template-columns",
        inner_width,
        column_gap,
        env,
    );
    let RuntimeTrackList {
        tracks: row_tracks,
        auto_fit: _,
        line_names: row_line_names,
    } = runtime_tracks_for_property(
        el,
        el.style_attr(),
        style,
        ancestors,
        "grid-template-rows",
        inner_width,
        row_gap,
        env,
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
    if style.writing_mode.is_vertical() {
        let logical_rows = num_rows.max(1);
        for p in &mut placed {
            let logical_col = p.col;
            let logical_row = p.row;
            let logical_col_span = p.col_span;
            let logical_row_span = p.row_span;
            p.col = if style.writing_mode.block_axis_reversed() {
                logical_rows.saturating_sub(logical_row + logical_row_span)
            } else {
                logical_row
            };
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

    let mut intrinsic = vec![TrackIntrinsicContributions::default(); num_cols];
    for p in &placed {
        let cs = &child_styles[p.idx];
        let contributions =
            grid_item_intrinsic_widths(cs, env, &element_children[p.idx], &child_ancestors);
        if p.col_span == 1 {
            if p.col < num_cols {
                intrinsic[p.col].minimum.include(contributions.minimum);
                intrinsic[p.col]
                    .min_content
                    .include(contributions.min_content);
                intrinsic[p.col]
                    .max_content
                    .include(contributions.max_content);
            }
        } else {
            add_spanning_contribution(
                &mut intrinsic,
                &column_tracks,
                p.col,
                p.col_span,
                IntrinsicAxis::Minimum,
                contributions.minimum,
            );
            add_spanning_contribution(
                &mut intrinsic,
                &column_tracks,
                p.col,
                p.col_span,
                IntrinsicAxis::MinContent,
                contributions.min_content,
            );
            add_spanning_contribution(
                &mut intrinsic,
                &column_tracks,
                p.col,
                p.col_span,
                IntrinsicAxis::MaxContent,
                contributions.max_content,
            );
        }
    }

    let col_widths = resolve_grid_columns(&column_tracks, inner_width, column_gap, &intrinsic);

    // Rows: explicit template-rows first, then grid-auto-rows for implicit
    // rows, then content height as a final fallback.
    let block_sizing = GridBlockSizing::from_style(style);
    let percentage_block_basis = block_sizing.percentage_basis();
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
            .and_then(|track| grid_track_fixed_height(track, percentage_block_basis));
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
                .and_then(|track| grid_track_fixed_height(track, percentage_block_basis))
                .is_some()
        {
            continue;
        }
        let cs = &child_styles[p.idx];
        let track_w = col_widths.iter().skip(p.col).take(p.col_span).sum::<f32>()
            + column_gap * p.col_span.saturating_sub(1) as f32;
        let item_width = compute_grid_item_geometry(cs, style, track_w, 0.0)
            .dimensions
            .content
            .width;
        let item_h = grid_item_outer_height(
            cs,
            Some(ctx),
            env,
            &element_children[p.idx],
            &child_ancestors,
            Some(item_width),
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
    if let Some(content_box_target) = block_sizing.track_extent() {
        let natural: f32 =
            row_heights.iter().sum::<f32>() + row_gap * num_rows.saturating_sub(1) as f32;
        let surplus = content_box_target - natural;
        if surplus > 0.0 && style.align_content == AlignContent::Stretch {
            // Stretchable rows: those not pinned by a fixed-length template track.
            let stretchable: Vec<usize> = (0..num_rows)
                .filter(|&r| {
                    row_tracks
                        .get(r)
                        .and_then(|track| grid_track_fixed_height(track, percentage_block_basis))
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

    let (grid_block_offset, effective_row_gap) = block_sizing
        .track_extent()
        .map(|target| distribute_rows(&row_heights, row_gap, target, style.align_content))
        .unwrap_or((0.0, row_gap));
    let (mut grid_inline_offset, effective_column_gap) =
        distribute_tracks(&col_widths, column_gap, inner_width, style.justify_content);
    if style.writing_mode.is_vertical() {
        grid_inline_offset -= style.border.horizontal_width();
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
    // Retain the distinction between a hard authored height and a min-height
    // floor. Both are resolved to border-box geometry, but only the former
    // makes the grid monolithic to descendant fragmentation.
    let border = style.border.widths();
    let natural_border_box_height = content_height + style.padding.vertical() + border.vertical();
    let block_size = BlockSize::from_style(style, natural_border_box_height);
    let grid_descendants = GridDescendantPositioning::for_container(
        style,
        ctx,
        positioned_depth,
        Size::new(
            (border_box_w - border.horizontal()).max(0.0),
            (block_size.resolve(natural_border_box_height) - border.vertical()).max(0.0),
        ),
    );

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
    let mut grid_children: Vec<LayoutNode> = Vec::new();
    for row in 0..num_rows {
        let row_breaks = fragmentation::forced_row_breaks(row, &placed, &child_styles);
        row_breaks.push_before(&mut grid_children);
        let track_h = row_heights[row];
        let mut cells: Vec<GridCell> = Vec::new();
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
        row_items.sort_by_key(|(p, _)| (p.col, p.idx));
        let baseline_offsets: std::collections::HashMap<usize, (f32, f32)> = if style.align_items
            == AlignItems::Baseline
        {
            let mut baselines = Vec::new();
            for (p, _) in &row_items {
                let cs = &child_styles[p.idx];
                let child_el = &element_children[p.idx];
                let has_text = child_el.children.iter().any(|child| match child {
                    DomNode::Text(text) => has_non_collapsible_text(text),
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
            // Pad with empty filler cells up to this item's column.
            while next_col < p.col {
                cells.push(empty_grid_cell(next_col, track_h));
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

            let bg = cs.background_color;

            // Resolve authored preferred/minimum/maximum sizes once. The same
            // border box drives paint and placement, and its content box is the
            // containing block offered to descendants.
            let geometry = compute_grid_item_geometry(cs, style, track_w, spanned_h);
            let mut inset = if p.row_span > 1 && row == p.row {
                Some(geometry.inset)
            } else if p.row_span > 1 {
                None
            } else {
                geometry.placement()
            };
            if let Some((baseline_offset, item_h)) = baseline_offsets.get(&p.idx).copied() {
                let mut baseline_inset = geometry.inset;
                baseline_inset.offset.y = cs.margin.top + baseline_offset;
                baseline_inset.size.height = item_h.min((spanned_h - baseline_offset).max(0.0));
                inset = Some(baseline_inset);
            }
            let painted_size = inset
                .map(|inset| inset.size)
                .unwrap_or_else(|| Size::new(track_w, spanned_h));
            let content_size = Size::new(
                (painted_size.width - cs.padding.horizontal() - cs.border.horizontal_width())
                    .max(0.0),
                (painted_size.height - cs.padding.vertical() - cs.border.vertical_width()).max(0.0),
            );
            let item_positioning = grid_descendants.for_item(
                cs,
                Size::new(
                    (painted_size.width - cs.border.horizontal_width()).max(0.0),
                    (painted_size.height - cs.border.vertical_width()).max(0.0),
                ),
            );

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
            let item_content = layout_grid_item_content(
                child_el,
                cs,
                ctx,
                &child_ancestors,
                GridItemContentFrame::positioned(content_size, item_positioning.descendants),
                env,
                Some(SubgridContext {
                    columns: child_column_subgrid,
                    rows: child_row_subgrid,
                }),
            );
            let cell_min_h = if p.row_span > 1 { track_h } else { spanned_h };

            let CellContent {
                lines,
                children: mut nested_rows,
            } = item_content;
            if row_span_offset > 0.0 {
                shift_nested_flow_up(&mut nested_rows, row_span_offset);
            }
            let border = LayoutBorder::from_computed(&cs.border, cs.color);
            let mut box_paint = BoxPaint::from_style(
                cs,
                LayoutSize::fixed(painted_size.width, Some(painted_size.height)),
            );
            box_paint.group.stacking = box_paint
                .group
                .stacking
                .with_role(crate::layout::elements::StackingRole::GridItem);
            box_paint.background.color = bg;
            let mut cell = GridCell {
                layout: CellBox {
                    content: CellContent {
                        lines,
                        children: nested_rows,
                    },
                    box_model: CellBoxModel {
                        content_insets: cs.padding + border.widths(),
                        border_insets: border.widths(),
                        border,
                        minimum_block_size: cell_min_h,
                    },
                    paint: CellPaint {
                        box_paint,
                        ..Default::default()
                    },
                    positioning: Positioning::from_style(cs)
                        .with_containing_block_depth(item_positioning.established_depth),
                    alignment: CellAlignment {
                        inline: cs.text_align,
                        block: cs.vertical_align,
                    },
                    fragmentation: CellFragmentation::from_style(cs),
                },
                placement: GridCellPlacement {
                    inset,
                    clips: cs.overflow.clips() || row_span_offset > 0.0,
                    column_start: p.col,
                    column_span: p.col_span.max(1),
                    row_span: p.row_span.max(1),
                    paint_order: GridPaintOrder::new(cs.order, p.idx),
                },
            };
            let mut filter_style = cs.principal.clone();
            let filter =
                super::filter::ResolvedFilter::from_style(&mut filter_style, env.filter_defs);
            super::filter::cells::retain_grid_cell_filter(&mut cell, filter);
            cells.push(cell);
            next_col = next_col.max(p.col + p.col_span);
        }

        // Fill trailing columns.
        while next_col < num_cols {
            cells.push(empty_grid_cell(next_col, track_h));
            next_col += 1;
        }

        let margin_top = if row == 0 {
            grid_block_offset
        } else {
            effective_row_gap
        };

        grid_children.push(
            GridRow {
                content: GridContent {
                    cells,
                    column_widths: col_widths.clone(),
                    gap: effective_column_gap,
                },
                box_model: BoxModel {
                    margins: BlockMargins::new(margin_top, 0.0),
                    padding: EdgeSizes {
                        left: grid_inline_offset,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                start_space: if row == 0 {
                    GridRowStartSpace::Alignment
                } else {
                    GridRowStartSpace::Gutter
                },
            }
            .boxed(),
        );
        row_breaks.push_after(&mut grid_children);
    }
    let _ = col_x;

    // Wrap all grid rows in a Container that carries the border, padding,
    // and background of the grid container element.
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
        let cb_padding_height =
            (block_size.resolve(natural_border_box_height) - border.vertical()).max(0.0);
        let cb_padding_width = inner_width.max(0.0) + style.padding.horizontal();
        for &idx in &abs_child_indices {
            let child_el = all_element_children[idx];
            let child_style = &all_child_styles[idx];
            let has_grid_area_cb = child_style.grid_area_name.is_some()
                || child_style.grid_column_start != GridLine::Auto
                || child_style.grid_column_end != GridLine::Auto
                || child_style.grid_row_start != GridLine::Auto
                || child_style.grid_row_end != GridLine::Auto;
            let mut abs_area_translation = ContainingBlockTranslation::default();
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
                    let area_y = grid_block_offset
                        + row_heights.iter().take(p.row).sum::<f32>()
                        + effective_row_gap * p.row as f32;
                    abs_area_translation = ContainingBlockTranslation::new(area_x, area_y);
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
                    Some(cb_padding_height.max(0.0)),
                    style.font_size,
                )
                .with_containing_block(Some(cb));
            let mut buf: Vec<LayoutNode> = Vec::new();
            flatten_element(
                child_el,
                LayoutTreeContext::new(style, &child_ctx, &abs_ancestors)
                    .with_positioned_ancestor_depth(positioned_depth)
                    .for_element(ElementSiblingContext::new(idx, total_child_count)),
                &mut buf,
                env,
            );
            crate::layout::helpers::resolve_absolute_descendants_containing_block(&mut buf, cb);
            if !abs_area_translation.is_identity() {
                translate_absolute_offsets(&mut buf, abs_area_translation);
            }
            grid_children.extend(buf);
        }
    }

    let box_model = BoxModel {
        size: LayoutSize {
            width: crate::layout::elements::InlineSize::fixed(border_box_w),
            height: block_size,
        },
        margins: BlockMargins::new(style.margin.top, style.margin.bottom),
        padding: style.padding,
        border: LayoutBorder::from_computed(&style.border, style.color),
    };
    let mut grid = Container::from_style(grid_children, style, box_model);
    // The grid formatting path must retain the same authored positioning state
    // as an ordinary block. `h_offset` is the box-model/static-position
    // contribution; it augments, rather than replaces, `left`.
    grid.positioning.insets.left += h_offset;
    grid.positioning.containing_block_depth = grid_positioned_depth;
    output.push(grid.boxed());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::elements::LayoutElementTestExt;

    fn find_colored_grid_row(elements: &[LayoutNode]) -> Option<(Vec<GridCell>, Vec<f32>)> {
        fn find(element: &dyn LayoutElement) -> Option<(Vec<GridCell>, Vec<f32>)> {
            if let Some(Some(row)) = element.inspect_grid(|row| {
                (row.content.cells.len() == 2
                    && row
                        .content
                        .cells
                        .iter()
                        .all(|cell| cell.layout.paint.background.color.is_some()))
                .then(|| (row.content.cells.clone(), row.content.column_widths.clone()))
            }) {
                return Some(row);
            }
            let mut nested = None;
            element.visit_children(&mut |child| nested = nested.take().or_else(|| find(child)));
            nested
        }

        elements.iter().find_map(|element| find(element.as_ref()))
    }

    #[test]
    fn subgrid_columns_inherit_parent_tracks() {
        let html = include_str!("../../tests/parity/cases/grid/grid-subgrid-columns.html");
        let parsed = crate::parser::html::parse_html_with_styles(html)
            .expect("subgrid fixture should parse");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| crate::parser::css::parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = crate::layout::engine::layout_with_rules(
            &parsed.nodes,
            crate::types::PageSize::new(288.0, 114.0),
            crate::types::Margin::uniform(0.0),
            &rules,
        );
        let (cells, widths) = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| find_colored_grid_row(std::slice::from_ref(element)))
            .expect("subgrid children should share one inherited row");

        assert_eq!(cells.len(), 2);
        assert_eq!(widths, [75.0, 150.0]);
    }

    fn find_absolute_offsets(elements: &[LayoutNode]) -> Option<(f32, f32)> {
        fn find(element: &dyn LayoutElement) -> Option<(f32, f32)> {
            if let Some(positioning) = element.positioning_owner()
                && positioning.positioning().scheme == Position::Absolute
            {
                return Some((
                    positioning.positioning().insets.left,
                    positioning.positioning().insets.top,
                ));
            }
            let mut nested = None;
            element.visit_children(&mut |child| nested = nested.take().or_else(|| find(child)));
            nested
        }

        elements.iter().find_map(|element| find(element.as_ref()))
    }

    fn find_page_absolute_offsets(elements: &[(f32, LayoutNode)]) -> Option<(f32, f32)> {
        elements
            .iter()
            .find_map(|(_, element)| find_absolute_offsets(std::slice::from_ref(element)))
    }

    fn absolute_grid_area_offsets(left: f32, top: f32) -> (f32, f32) {
        let html = format!(
            r#"<div style="display:grid; position:relative; width:120pt; height:80pt;
                    grid-template-columns:40pt 80pt; grid-template-rows:30pt 50pt;
                    justify-content:start; align-content:start">
                    <div style="position:absolute; grid-column-start:2; grid-column-end:3;
                        grid-row-start:1; grid-row-end:2;
                        left:{left}pt; top:{top}pt; width:5pt; height:5pt"></div>
                </div>"#
        );
        let nodes = crate::parser::html::parse_html(&html).expect("grid fixture should parse");
        let pages = crate::layout::engine::layout(
            &nodes,
            crate::types::PageSize::A4,
            crate::types::Margin::uniform(0.0),
        );
        let page = pages.first().expect("grid fixture should produce a page");
        find_page_absolute_offsets(&page.elements)
            .expect("grid fixture should produce an absolute child")
    }

    fn repeated_target_lines() -> Vec<Vec<String>> {
        vec![
            vec!["start".into()],
            vec!["target".into()],
            vec!["other".into()],
            vec!["target".into()],
            Vec::new(),
        ]
    }

    #[test]
    fn spanning_contribution_does_not_reclassify_a_subpoint_track_as_empty() {
        let tracks = [RuntimeTrack::Auto, RuntimeTrack::Auto];
        let mut intrinsic = [TrackIntrinsicContributions::default(); 2];
        intrinsic[1].minimum.include(0.005);

        add_spanning_contribution(&mut intrinsic, &tracks, 0, 2, IntrinsicAxis::Minimum, 1.005);

        assert_eq!(intrinsic[0].minimum.size(), 1.005_f32 - 0.005);
        assert_eq!(intrinsic[1].minimum.size(), 0.005);
    }

    #[test]
    fn fit_content_percentages_resolve_and_clamp_across_the_full_range() {
        let style = ComputedStyle::default();
        let fonts = std::collections::HashMap::new();
        let parser = RuntimeTrackParser::new(&style, 300.0, FontMetrics::new(&fonts));
        let intrinsic = [TrackIntrinsicContributions {
            minimum: IntrinsicContribution::Sized(45.0),
            max_content: IntrinsicContribution::Sized(135.0),
            ..Default::default()
        }];

        for (argument, expected) in [
            ("0%", 45.0),
            ("10%", 45.0),
            ("30%", 90.0),
            ("50%", 135.0),
            ("100%", 135.0),
            ("150%", 135.0),
        ] {
            let track = parse_runtime_track(&format!("fit-content({argument})"), &parser)
                .expect("valid fit-content percentage");
            assert_eq!(
                resolve_grid_columns(&[track], 300.0, 0.0, &intrinsic),
                [expected],
                "argument {argument}"
            );
        }
    }

    #[test]
    fn fit_content_percentage_basis_is_the_content_box_before_gaps() {
        let intrinsic = [
            TrackIntrinsicContributions {
                minimum: IntrinsicContribution::Sized(45.0),
                max_content: IntrinsicContribution::Sized(135.0),
                ..Default::default()
            },
            TrackIntrinsicContributions::default(),
        ];
        let tracks = [
            RuntimeTrack::FitContent(LengthPercent::percent(30.0)),
            RuntimeTrack::Fr(1.0),
        ];

        assert_eq!(
            resolve_grid_columns(&tracks, 300.0, 30.0, &intrinsic),
            [90.0, 180.0]
        );
    }

    #[test]
    fn definite_grid_block_size_is_constrained_before_track_alignment() {
        let style = ComputedStyle {
            height: Some(68.0),
            max_height: Some(58.0),
            padding: EdgeSizes::uniform(7.0),
            border: crate::style::computed::BorderSides::uniform(
                crate::style::computed::BorderSide::solid(
                    2.0,
                    crate::parser::css::SpecifiedColor::CurrentColor,
                ),
            ),
            box_sizing: BoxSizing::BorderBox,
            ..Default::default()
        };

        let sizing = GridBlockSizing::from_style(&style);

        assert_eq!(sizing.definite_content_height, Some(40.0));
        assert_eq!(sizing.track_extent(), Some(40.0));
    }

    #[test]
    fn fit_content_uses_auto_minimum_not_no_wrap_min_content() {
        let intrinsic = [
            TrackIntrinsicContributions {
                minimum: IntrinsicContribution::Sized(39.0),
                min_content: IntrinsicContribution::Sized(240.0),
                max_content: IntrinsicContribution::Sized(240.0),
            },
            TrackIntrinsicContributions::default(),
        ];

        assert_eq!(
            resolve_grid_columns(
                &[
                    RuntimeTrack::FitContent(LengthPercent::percent(0.0)),
                    RuntimeTrack::Fr(1.0),
                ],
                300.0,
                0.0,
                &intrinsic,
            ),
            [39.0, 261.0]
        );
        assert_eq!(
            resolve_grid_columns(
                &[
                    RuntimeTrack::FitContent(LengthPercent::percent(30.0)),
                    RuntimeTrack::Fr(1.0),
                ],
                300.0,
                0.0,
                &intrinsic,
            ),
            [90.0, 210.0]
        );
    }

    #[test]
    fn fit_content_accepts_mixed_calc_and_rejects_negative_literal() {
        let style = ComputedStyle::default();
        let fonts = std::collections::HashMap::new();
        let parser = RuntimeTrackParser::new(&style, 300.0, FontMetrics::new(&fonts));

        let mixed = parse_runtime_track("fit-content(calc(30% - 10px))", &parser)
            .expect("mixed length-percentage is valid");
        let RuntimeTrack::FitContent(limit) = mixed else {
            panic!("fit-content must retain its semantic limit");
        };
        assert!((limit.resolve(300.0) - 82.5).abs() < 0.001);
        assert!(parse_runtime_track("fit-content(-1px)", &parser).is_none());
    }

    #[test]
    fn spanning_contribution_still_targets_an_exactly_empty_track() {
        let tracks = [RuntimeTrack::Auto, RuntimeTrack::Auto];
        let mut intrinsic = [TrackIntrinsicContributions::default(); 2];
        intrinsic[0].minimum.include(0.5);
        intrinsic[1].minimum.include(0.0);
        assert!(intrinsic[1].minimum.is_empty());

        add_spanning_contribution(&mut intrinsic, &tracks, 0, 2, IntrinsicAxis::Minimum, 1.5);

        assert_eq!(intrinsic[0].minimum.size(), 0.5);
        assert_eq!(intrinsic[1].minimum.size(), 1.0);
    }

    #[test]
    fn grid_item_geometry_preserves_subpoint_content_width_and_exact_zero() {
        let style = ComputedStyle {
            padding: EdgeSizes::new(0.0, 0.001, 0.0, 0.001),
            ..Default::default()
        };

        let subpoint_inner =
            compute_grid_item_geometry(&style, &ComputedStyle::default(), 0.005, 0.0)
                .dimensions
                .content
                .width;
        assert_eq!(subpoint_inner, 0.005_f32 - style.padding.horizontal());
        assert!(subpoint_inner > 0.0);
        assert_eq!(
            compute_grid_item_geometry(&style, &ComputedStyle::default(), 0.002, 0.0)
                .dimensions
                .content
                .width,
            0.0
        );
    }

    #[test]
    fn explicit_grid_item_size_overflows_instead_of_shrinking_to_its_track() {
        let item = ComputedStyle {
            width: Some(43.5),
            height: Some(36.0),
            ..Default::default()
        };
        let inset = compute_grid_inset(&item, &ComputedStyle::default(), 39.375, 30.0)
            .expect("an explicitly sized item has concrete alignment geometry");

        assert_eq!(inset.offset, Point::default());
        assert_eq!(inset.size, Size::new(43.5, 36.0));
    }

    #[test]
    fn grid_item_minimum_overflows_a_narrower_track() {
        let item = ComputedStyle {
            width: Some(58.0),
            min_width: Some(92.0),
            height: Some(48.0),
            padding: EdgeSizes::uniform(5.0),
            border: crate::style::computed::BorderSides::uniform(
                crate::style::computed::BorderSide::solid(
                    2.0,
                    crate::parser::css::SpecifiedColor::CurrentColor,
                ),
            ),
            box_sizing: BoxSizing::BorderBox,
            ..Default::default()
        };
        let geometry = compute_grid_item_geometry(&item, &ComputedStyle::default(), 52.5, 48.0);

        assert_eq!(geometry.inset.offset, Point::default());
        assert_eq!(geometry.inset.size, Size::new(92.0, 48.0));
        assert_eq!(geometry.dimensions.content.width, 78.0);
    }

    #[test]
    fn bare_fraction_track_honors_its_automatic_minimum() {
        let tracks = [RuntimeTrack::Fr(1.0), RuntimeTrack::Fr(1.0)];
        let mut intrinsic = [TrackIntrinsicContributions::default(); 2];
        intrinsic[0].minimum.include(31.5);
        intrinsic[1].minimum.include(43.5);

        let widths = resolve_grid_columns(&tracks, 81.0, 2.25, &intrinsic);

        assert_eq!(widths, [35.25, 43.5]);
    }

    #[test]
    fn definite_grid_item_contribution_uses_its_outer_border_box() {
        let item = ComputedStyle {
            width: Some(43.5),
            padding: EdgeSizes::uniform(5.25),
            box_sizing: BoxSizing::BorderBox,
            margin: EdgeSizes::new(0.0, 1.5, 0.0, 2.25),
            ..Default::default()
        };

        assert_eq!(grid_item_definite_outer_width(&item), Some(47.25));
    }

    #[test]
    fn named_span_counts_only_matching_lines_in_search_direction() {
        let mut line_names = repeated_target_lines();
        line_names[1].push("target".into());
        let names = build_line_name_map(&line_names, &[], 4);
        assert_eq!(names["target"], vec![1, 3]);
        assert_eq!(
            resolve_axis(
                &GridLine::Named("start".into()),
                &GridLine::SpanNamed {
                    count: 2,
                    name: "target".into(),
                },
                4,
                &names,
            ),
            Some((0, 3))
        );
        assert_eq!(
            resolve_axis(
                &GridLine::SpanNamed {
                    count: 2,
                    name: "target".into(),
                },
                &GridLine::Line(5),
                4,
                &names,
            ),
            Some((1, 3))
        );
    }

    #[test]
    fn effective_line_name_override_is_not_merged_twice() {
        let mut container = ComputedStyle::default();
        container.grid_template_columns = vec![GridTrack::Fixed(50.0); 4];
        container.grid_template_column_line_names = repeated_target_lines();

        let mut child = ComputedStyle::default();
        child.grid_column_start = GridLine::Named("start".into());
        child.grid_column_end = GridLine::SpanNamed {
            count: 2,
            name: "target".into(),
        };

        let effective_names = repeated_target_lines();
        let placement = place_grid_items(
            &container,
            &[GridItemStyle::principal_only(child)],
            4,
            Some(4),
            None,
            Some(&effective_names),
            None,
        );
        assert_eq!(placement.placed.len(), 1);
        assert_eq!(placement.placed[0].col, 0);
        assert_eq!(placement.placed[0].col_span, 3);
    }

    #[test]
    fn absolute_grid_area_applies_fractional_insets_once() {
        assert_eq!(
            absolute_grid_area_offsets(2.125, 3.375),
            (40.0_f32 + 2.125, 3.375)
        );
    }

    #[test]
    fn absolute_grid_area_preserves_a_point_zero_zero_five_inset_separation() {
        let first = absolute_grid_area_offsets(2.125, 3.375);
        let second = absolute_grid_area_offsets(2.130, 3.380);

        assert_eq!(first, (40.0_f32 + 2.125, 3.375));
        assert_eq!(second, (40.0_f32 + 2.130, 3.380));
        assert_ne!(first, second);
    }
}
