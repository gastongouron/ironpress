//! Reusable semantic properties carried by concrete layout nodes.

mod paint_group;

pub(crate) use paint_group::*;

use crate::layout::engine::{ContainingBlock, LayoutBorder};
use crate::layout::flow_metrics::BlockMargins;
use crate::layout::helpers::BackgroundFields;
use crate::style::computed::{
    BlendMode, BoxShadow, Clear, Float, Overflow, Position, TextAlign, Visibility, WritingMode,
};
use crate::types::{Color, CornerRadii, EdgeSizes, PhysicalEdges, Point, Rect, Size};

/// Inline-axis sizing carried by a laid-out box.
///
/// A box either fills the inline space offered by its containing layout frame,
/// or carries a fixed used width resolved by layout. Keeping that distinction
/// out of a raw `Option<f32>` prevents authored `width: auto` from being
/// confused with unresolved geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct InlineSize {
    used: Option<f32>,
    /// The used width is the offered space less margins, not a width the box
    /// asks for, so it must not feed back into intrinsic sizing.
    derived_from_available: bool,
}

impl InlineSize {
    pub(crate) const FILL_AVAILABLE: Self = Self {
        used: None,
        derived_from_available: false,
    };

    pub(crate) const fn fixed(width: f32) -> Self {
        Self {
            used: Some(width),
            derived_from_available: false,
        }
    }

    pub(crate) fn from_fixed_value(width: Option<f32>) -> Self {
        width.map_or(Self::FILL_AVAILABLE, Self::fixed)
    }

    pub(crate) fn from_used(width: f32, available_width: f32, fixed_by_layout: bool) -> Self {
        if fixed_by_layout {
            Self::fixed(width)
        } else if width != available_width {
            Self {
                used: Some(width),
                derived_from_available: true,
            }
        } else {
            Self::FILL_AVAILABLE
        }
    }

    pub(crate) const fn fixed_value(self) -> Option<f32> {
        self.used
    }

    /// The width this box asks for independently of the space it was offered.
    pub(crate) const fn preferred_value(self) -> Option<f32> {
        if self.derived_from_available {
            None
        } else {
            self.used
        }
    }

    pub(crate) const fn is_fill_available(self) -> bool {
        self.used.is_none()
    }

    pub(crate) fn resolve(self, available_width: f32) -> f32 {
        self.used.unwrap_or(available_width)
    }
}

/// Minimum and maximum constraints for one physical axis.
///
/// CSS sizing does not define these as a conventional ordered range: an
/// authored minimum may exceed the maximum, in which case the minimum wins.
/// Keeping the pair together makes the required `max(minimum, min(size,
/// maximum))` ordering impossible to reverse at individual layout call sites.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SizeConstraints {
    minimum: Option<f32>,
    maximum: Option<f32>,
}

impl SizeConstraints {
    pub(crate) const fn new(minimum: Option<f32>, maximum: Option<f32>) -> Self {
        Self { minimum, maximum }
    }

    pub(crate) fn map(self, mut convert: impl FnMut(f32) -> f32) -> Self {
        Self::new(self.minimum.map(&mut convert), self.maximum.map(convert))
    }

    pub(crate) fn constrain(self, size: f32) -> f32 {
        let capped = self.maximum.map_or(size, |maximum| size.min(maximum));
        self.minimum.map_or(capped, |minimum| capped.max(minimum))
    }

    pub(crate) const fn minimum(self) -> Option<f32> {
        self.minimum
    }

    pub(crate) const fn maximum(self) -> Option<f32> {
        self.maximum
    }

    /// Constrain a definite preferred size, or expose an authored minimum as
    /// the initial used floor for an otherwise automatic size. A lone maximum
    /// cannot determine an automatic size before content has been measured.
    pub(crate) fn constrain_preferred(self, preferred: Option<f32>) -> Option<f32> {
        preferred.map(|size| self.constrain(size)).or(self.minimum)
    }
}

/// A laid-out block-axis size without conflating an already-resolved used
/// extent with an authored definite height.
///
/// Auto boxes can acquire a used extent from `min-height` or fragmentation and
/// still remain fragmentable. Definite boxes instead establish a hard extent;
/// overflowing descendants do not enlarge or split that principal box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BlockSize {
    used: Option<f32>,
    definite: bool,
}

impl BlockSize {
    pub(crate) const AUTO: Self = Self {
        used: None,
        definite: false,
    };

    pub(crate) const fn definite(used: f32) -> Self {
        Self {
            used: Some(used),
            definite: true,
        }
    }

    pub(crate) const fn from_definite(used: Option<f32>) -> Self {
        match used {
            Some(used) => Self::definite(used),
            None => Self::AUTO,
        }
    }

    /// Record a used extent whose final size may still grow with its contents.
    ///
    /// This is the shared representation for intrinsic auto sizing,
    /// `min-height`, and fragment geometry: all three supply a real used floor
    /// without turning the box into a fixed-height overflow container.
    pub(crate) const fn content_dependent(used: f32) -> Self {
        Self {
            used: Some(used),
            definite: false,
        }
    }

    pub(crate) const fn minimum(used: f32) -> Self {
        Self::content_dependent(used)
    }

    /// Record the used extent of one slice without turning the source box into
    /// a fixed-height overflow container.
    pub(crate) const fn fragment(used: f32) -> Self {
        Self::content_dependent(used)
    }

    pub(crate) const fn used(self) -> Option<f32> {
        self.used
    }

    pub(crate) const fn is_definite(self) -> bool {
        self.definite
    }

    /// Split one used principal-box extent without losing its sizing semantics.
    /// A definite height stays definite in both fragments; a content-dependent
    /// minimum becomes one painted slice plus the remaining composite floor.
    pub(crate) fn split_fragment_at(self, consumed: f32) -> Option<(Self, Self)> {
        let total = self.used?;
        let consumed = consumed.clamp(0.0, total);
        let remainder = total - consumed;
        if !crate::layout::roundoff::is_positive_with_roundoff(consumed)
            || !crate::layout::roundoff::is_positive_with_roundoff(remainder)
        {
            return None;
        }

        Some(if self.definite {
            (Self::definite(consumed), Self::definite(remainder))
        } else {
            (Self::fragment(consumed), Self::minimum(remainder))
        })
    }

    /// Remaining composite minimum after one fragment consumes a block-axis
    /// extent. A content-dependent floor belongs to the unfragmented principal
    /// box; copying it to every continuation multiplies `min-height`, while
    /// dropping it shortens the composite box.
    pub(crate) fn remaining_fragment_floor(self, consumed: f32) -> Self {
        match (self.used, self.definite) {
            (Some(minimum), false) if minimum > consumed => {
                Self::minimum(minimum - consumed.max(0.0))
            }
            _ => Self::AUTO,
        }
    }

    pub(crate) fn resolve(self, content_height: f32) -> f32 {
        match (self.used, self.definite) {
            (Some(used), true) => used,
            (Some(minimum), false) => content_height.max(minimum),
            (None, _) => content_height,
        }
    }

    /// Resolve the authored block-axis constraints against a natural border
    /// box without losing whether the resulting extent is a floor or a hard
    /// cap. `min-height` remains fragmentable; an explicit `height`, or a
    /// `max-height` that actually clamps content, is definite.
    pub(crate) fn from_style(
        style: &crate::style::computed::ComputedStyle,
        natural_border_box_height: f32,
    ) -> Self {
        let edges = style.padding.vertical() + style.border.vertical_width();
        let border_box = |height: f32| match style.box_sizing {
            crate::style::computed::BoxSizing::BorderBox => height.max(0.0),
            crate::style::computed::BoxSizing::ContentBox => height.max(0.0) + edges,
        };
        let constraints = SizeConstraints::new(style.min_height, style.max_height).map(border_box);

        if let Some(height) = style.height {
            return Self::definite(constraints.constrain(border_box(height)));
        }

        let used = constraints.constrain(natural_border_box_height);
        if used < natural_border_box_height {
            return Self::definite(used);
        }

        Self::content_dependent(used)
    }
}

/// Physical dimensions of a laid-out border box. Inline and block sizing both
/// retain the constraint semantics needed by pagination.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct LayoutSize {
    pub(crate) width: InlineSize,
    pub(crate) height: BlockSize,
}

impl LayoutSize {
    /// A fixed inline extent paired with explicit block-axis sizing semantics.
    ///
    /// This is the preferred constructor for layout modes that compute an
    /// auto/minimum fragment height: their used extent must not accidentally
    /// become a definite CSS height merely because layout has measured it.
    pub(crate) const fn fixed_inline(width: f32, height: BlockSize) -> Self {
        Self {
            width: InlineSize::fixed(width),
            height,
        }
    }

    pub(crate) const fn fixed(width: f32, height: Option<f32>) -> Self {
        Self::fixed_inline(width, BlockSize::from_definite(height))
    }

    pub(crate) fn resolve_width(self, available_width: f32) -> f32 {
        self.width.resolve(available_width)
    }
}

/// Geometry shared by ordinary CSS boxes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BoxModel {
    pub(crate) size: LayoutSize,
    pub(crate) margins: BlockMargins,
    pub(crate) padding: EdgeSizes,
    pub(crate) border: LayoutBorder,
}

impl BoxModel {
    pub(crate) fn from_style(
        style: &crate::style::computed::ComputedStyle,
        margins: BlockMargins,
    ) -> Self {
        Self {
            size: LayoutSize {
                width: InlineSize::from_fixed_value(style.width),
                height: BlockSize::from_definite(style.height),
            },
            margins,
            padding: style.padding,
            border: LayoutBorder::from_computed(&style.border, style.color),
        }
    }
}

impl super::BoxReferenceGeometry for BoxModel {
    fn border_insets(&self) -> EdgeSizes {
        self.border.widths()
    }

    fn content_insets(&self) -> EdgeSizes {
        self.border.widths() + self.padding
    }
}

/// Resolved background layers and their local compositing rule.
#[derive(Debug, Clone)]
pub(crate) struct BackgroundPaint {
    pub(crate) color: Option<Color>,
    pub(crate) layers: BackgroundFields,
    pub(crate) blend_mode: BlendMode,
}

impl Default for BackgroundPaint {
    fn default() -> Self {
        Self {
            color: None,
            layers: BackgroundFields::default(),
            blend_mode: BlendMode::Normal,
        }
    }
}

impl BackgroundPaint {
    pub(crate) fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            color: style.background_color,
            layers: BackgroundFields::from_style(style),
            blend_mode: style.background_blend_mode,
        }
    }
}

/// Paint outside the border edge.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OutlinePaint {
    pub(crate) width: f32,
    pub(crate) color: Option<Color>,
    pub(crate) offset: f32,
}

impl OutlinePaint {
    pub(crate) fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            width: style.outline_width,
            color: style.outline_color,
            offset: style.outline_offset,
        }
    }
}

/// Visual decoration and compositing shared by box nodes.
#[derive(Debug, Clone)]
pub(crate) struct BoxPaint {
    pub(crate) background: BackgroundPaint,
    pub(crate) border_image: Option<crate::style::computed::BorderImagePaint>,
    pub(crate) border_radii: CornerRadii,
    pub(crate) shadows: Vec<BoxShadow>,
    pub(crate) outline: OutlinePaint,
    pub(crate) group: PaintGroup,
    pub(crate) visible: bool,
}

impl Default for BoxPaint {
    fn default() -> Self {
        Self {
            background: BackgroundPaint::default(),
            border_image: None,
            border_radii: CornerRadii::ZERO,
            shadows: Vec::new(),
            outline: OutlinePaint::default(),
            group: PaintGroup::default(),
            visible: true,
        }
    }
}

impl BoxPaint {
    pub(crate) fn from_style(
        style: &crate::style::computed::ComputedStyle,
        size: LayoutSize,
    ) -> Self {
        let width = size.width.fixed_value().unwrap_or_default();
        let height = size.height.resolve(width);
        Self {
            background: BackgroundPaint::from_style(style),
            border_image: style.border_image.paint(),
            border_radii: style.resolve_corner_radii(width, height),
            shadows: style.box_shadow.clone(),
            outline: OutlinePaint::from_style(style),
            group: PaintGroup::from_style(style),
            visible: style.visibility == Visibility::Visible,
        }
    }

    /// Whether paint owned by this box can escape its fragment border box.
    ///
    /// The exact overflow bounds are renderer geometry, but ownership of an
    /// outset effect is semantic layout state. Page separation uses this
    /// conservative predicate to retain complete graphical output without
    /// teaching fragmentation about individual effect implementations.
    pub(crate) fn has_outset_graphical_effect(&self) -> bool {
        self.group.transform.establishes_stacking_context()
            || self.shadows.iter().any(|shadow| !shadow.inset)
            || self.outline.width > 0.0
            || self.background.layers.blur_radius > 0.0
            || self
                .group
                .filter
                .as_ref()
                .is_some_and(crate::layout::filter::ResolvedFilter::requires_source_surface)
    }
}

pub(crate) fn text_lines_have_outset_shadows(lines: &[crate::layout::engine::TextLine]) -> bool {
    lines
        .iter()
        .flat_map(|line| &line.runs)
        .any(|run| run.text_shadow.iter().any(|shadow| !shadow.inset))
}

/// Normal-flow participation of a block-level box.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BlockFlow {
    pub(crate) float: Float,
    pub(crate) clear: Clear,
}

/// Block-axis spacing and non-painting trailing flow extent.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FlowSpacing {
    pub(crate) margins: BlockMargins,
    pub(crate) extra_end: f32,
}

impl FlowSpacing {
    pub(crate) const fn content_extent(self, block_size: f32) -> f32 {
        block_size + self.extra_end
    }

    pub(crate) const fn outer_extent(self, block_size: f32) -> f32 {
        self.margins.total() + self.content_extent(block_size)
    }
}

/// An inline-axis displacement from the containing content edge.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct InlineOffset(f32);

impl InlineOffset {
    pub(crate) const ZERO: Self = Self(0.0);

    pub(crate) const fn new(value: f32) -> Self {
        Self(value)
    }

    pub(crate) const fn value(self) -> f32 {
        self.0
    }

    /// Resolve the physical start edge of a normal-flow block-level box.
    ///
    /// Auto inline margins absorb positive free space after the used border-box
    /// width and the non-auto margin have been removed. This applies equally to
    /// ordinary blocks and to table wrappers after their fit-content width has
    /// been resolved.
    pub(crate) fn resolve_block_start(
        style: &crate::style::computed::ComputedStyle,
        containing_width: f32,
        border_box_width: f32,
    ) -> Self {
        let fixed_start = if !style.margin_left_auto {
            style.margin.left
        } else {
            Default::default()
        };
        let fixed_end = if !style.margin_right_auto {
            style.margin.right
        } else {
            Default::default()
        };
        let free_space = (containing_width - border_box_width - fixed_start - fixed_end).max(0.0);

        let start = match (style.margin_left_auto, style.margin_right_auto) {
            (true, true) => free_space / 2.0,
            (true, false) => free_space,
            (false, _) => fixed_start,
        };
        Self(start)
    }
}

impl std::ops::Add<f32> for InlineOffset {
    type Output = Self;

    fn add(self, rhs: f32) -> Self::Output {
        Self(self.0 + rhs)
    }
}

/// One authored CSS inset, retained past initial layout so fragmented
/// containing blocks can resolve the same constraint against their continuous
/// reference box instead of treating an early top/left result as authored.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum PositionInset {
    #[default]
    Auto,
    Length(f32),
    Percentage(f32),
}

impl PositionInset {
    const fn from_style(length: Option<f32>, percentage: Option<f32>) -> Self {
        match (percentage, length) {
            (Some(value), _) => Self::Percentage(value),
            (None, Some(value)) => Self::Length(value),
            (None, None) => Self::Auto,
        }
    }

    fn resolve(self, reference: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Length(value) => Some(value),
            Self::Percentage(value) => Some(reference * value / 100.0),
        }
    }
}

/// Authored absolute-position constraints on both physical axes.
///
/// [`Positioning::insets`] is the resolved top-left placement consumed by
/// layout and paint. This structure is the source constraint that can be
/// resolved again when fragmentation changes the containing block's composite
/// block size.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct PositionConstraints {
    edges: PhysicalEdges<PositionInset>,
}

impl PositionConstraints {
    fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            edges: PhysicalEdges::new(
                PositionInset::from_style(style.top, style.percentage_insets.top),
                PositionInset::from_style(style.right, style.percentage_insets.right),
                PositionInset::from_style(style.bottom, style.percentage_insets.bottom),
                PositionInset::from_style(style.left, style.percentage_insets.left),
            ),
        }
    }

    #[cfg(test)]
    fn from_lengths(edges: PhysicalEdges<Option<f32>>) -> Self {
        Self {
            edges: edges.map(|value| value.map_or(PositionInset::Auto, PositionInset::Length)),
        }
    }

    fn resolve_axis(
        start: PositionInset,
        end: PositionInset,
        reference: f32,
        extent: f32,
        fallback: f32,
    ) -> f32 {
        start.resolve(reference).unwrap_or_else(|| {
            end.resolve(reference)
                .map_or(fallback, |end| reference - extent - end)
        })
    }

    fn resolve_origin(
        self,
        reference: crate::types::Size,
        extent: crate::types::Size,
        fallback: Point,
    ) -> Point {
        Point::new(
            Self::resolve_axis(
                self.edges.left,
                self.edges.right,
                reference.width,
                extent.width,
                fallback.x,
            ),
            Self::resolve_axis(
                self.edges.top,
                self.edges.bottom,
                reference.height,
                extent.height,
                fallback.y,
            ),
        )
    }

    fn resolved_edges(self, reference: Size) -> PhysicalEdges<Option<f32>> {
        PhysicalEdges::new(
            self.edges.top.resolve(reference.height),
            self.edges.right.resolve(reference.width),
            self.edges.bottom.resolve(reference.height),
            self.edges.left.resolve(reference.width),
        )
    }
}

/// Resolve one sticky-positioned axis at the initial scroll position.
///
/// `normal_start` is the border edge produced by ordinary flow. Insets define
/// the sticky view rectangle; they are constraints, never unconditional
/// translations. The end inset is reduced when necessary so the view rectangle
/// can contain the border box, as required by CSS Positioned Layout 3 section
/// 3.4.
fn resolve_sticky_axis(
    normal_start: f32,
    extent: f32,
    scrollport_extent: f32,
    start: Option<f32>,
    end: Option<f32>,
) -> f32 {
    if !normal_start.is_finite()
        || !extent.is_finite()
        || !scrollport_extent.is_finite()
        || scrollport_extent <= 0.0
    {
        return normal_start;
    }

    let view_start = start.unwrap_or_default();
    let mut view_end = scrollport_extent - end.unwrap_or_default();
    if view_end - view_start < extent {
        view_end = view_start + extent;
    }

    let mut used_start = normal_start;
    if start.is_some() && used_start < view_start {
        used_start = view_start;
    }
    if end.is_some() && used_start + extent > view_end {
        used_start = view_end - extent;
    }

    // Sticky positioning may move the box only insofar as its position box
    // remains in its containing block. At this stage the nearest scrollport is
    // also the local containing frame; preserve already-overflowing normal
    // positions instead of pulling ordinary overflow back into the box.
    if used_start > normal_start {
        used_start.min((scrollport_extent - extent).max(normal_start))
    } else if used_start < normal_start {
        used_start.max(0.0f32.min(normal_start))
    } else {
        used_start
    }
}

/// Physical positioned-layout state. The resolved placement and authored
/// constraints stay together so pagination never has to infer whether a
/// top-left coordinate came from `top`/`left`, `bottom`/`right`, or `auto`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Positioning {
    pub(crate) scheme: Position,
    pub(crate) insets: EdgeSizes,
    constraints: PositionConstraints,
    pub(crate) containing_block: Option<ContainingBlock>,
    pub(crate) containing_block_depth: usize,
}

impl Positioning {
    pub(crate) fn with_scheme(mut self, scheme: Position) -> Self {
        self.scheme = scheme;
        self
    }

    pub(crate) fn with_resolved_insets(mut self, insets: EdgeSizes) -> Self {
        self.insets = insets;
        self
    }

    pub(crate) fn with_containing_block(
        mut self,
        containing_block: Option<ContainingBlock>,
    ) -> Self {
        self.containing_block = containing_block;
        self
    }

    pub(crate) fn with_containing_block_depth(mut self, depth: usize) -> Self {
        self.containing_block_depth = depth;
        self
    }

    /// Position an implementation-owned child from its containing padding box.
    ///
    /// Anonymous layout boxes such as multicolumn columns and rules use the
    /// same containing-block contract as authored absolute descendants. Keeping
    /// that placement in [`Positioning`] lets every recursive painter resolve
    /// it through the ordinary positioned-child path.
    pub(crate) const fn absolute_at(origin: Point) -> Self {
        Self {
            scheme: Position::Absolute,
            insets: EdgeSizes::new(origin.y, 0.0, 0.0, origin.x),
            constraints: PositionConstraints {
                edges: PhysicalEdges::new(
                    PositionInset::Length(origin.y),
                    PositionInset::Auto,
                    PositionInset::Auto,
                    PositionInset::Length(origin.x),
                ),
            },
            containing_block: None,
            containing_block_depth: 0,
        }
    }

    /// Physical top-left inset represented as a point.
    pub(crate) const fn origin(&self) -> Point {
        Point::new(self.insets.left, self.insets.top)
    }

    /// Resolve the painted origin of an in-flow box in its nearest scrollport.
    ///
    /// `normal_origin` is the box's ordinary-flow border-box origin before any
    /// relative/sticky adjustment. For historical layout nodes the resolved
    /// inline margin/centering contribution is already folded into `insets`;
    /// subtracting the authored sticky inset recovers that ordinary position.
    /// Keeping this interpretation in one method prevents concrete renderers
    /// from implementing incompatible `relative`/`sticky` special cases.
    pub(crate) fn resolve_in_flow_origin(
        &self,
        normal_origin: Point,
        extent: Size,
        scrollport: Size,
    ) -> Point {
        if self.scheme != Position::Sticky {
            return Point::new(
                normal_origin.x + self.insets.left,
                normal_origin.y + self.insets.top,
            );
        }

        let resolved = self.constraints.resolved_edges(scrollport);
        let ordinary = Point::new(
            normal_origin.x + self.insets.left - resolved.left.unwrap_or_default(),
            normal_origin.y,
        );
        Point::new(
            resolve_sticky_axis(
                ordinary.x,
                extent.width,
                scrollport.width,
                resolved.left,
                resolved.right,
            ),
            resolve_sticky_axis(
                ordinary.y,
                extent.height,
                scrollport.height,
                resolved.top,
                resolved.bottom,
            ),
        )
    }

    pub(crate) fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            scheme: style.position,
            insets: EdgeSizes::new(
                style.top.unwrap_or_default(),
                style.right.unwrap_or_default(),
                style.bottom.unwrap_or_default(),
                style.left.unwrap_or_default(),
            ),
            constraints: PositionConstraints::from_style(style),
            ..Default::default()
        }
    }

    /// Build an authored absolute position before a containing block is known.
    /// `None` is CSS `auto`; a present zero remains distinguishable from it.
    #[cfg(test)]
    pub(crate) fn absolute_from_lengths(edges: PhysicalEdges<Option<f32>>) -> Self {
        Self {
            scheme: Position::Absolute,
            insets: edges.map(Option::unwrap_or_default),
            constraints: PositionConstraints::from_lengths(edges),
            ..Default::default()
        }
    }

    /// Resolve authored constraints against one containing block.
    pub(crate) fn resolve_against(
        &mut self,
        containing_block: ContainingBlock,
        extent: crate::types::Size,
    ) {
        let reference = crate::types::Size::new(containing_block.width, containing_block.height);
        let origin = self
            .constraints
            .resolve_origin(reference, extent, self.origin());
        self.insets.top = origin.y;
        self.insets.left = origin.x;
        self.containing_block = Some(containing_block);
    }

    /// Re-resolve an absolute box when its owning containing block reaches its
    /// final used size.
    ///
    /// A provisional block at the same positioned depth is replaceable.
    /// Geometry from a different depth belongs to a nearer positioned ancestor
    /// and must remain intact.
    pub(crate) fn resolve_final_containing_block(
        &mut self,
        containing_block: ContainingBlock,
        extent: crate::types::Size,
    ) {
        if !self.scheme.is_absolute()
            || self
                .containing_block
                .is_some_and(|current| current.depth != containing_block.depth)
        {
            return;
        }
        self.resolve_against(containing_block, extent);
    }

    /// Re-resolve the block coordinate against a fragmented containing block's
    /// continuous padding box, then express it in one fragment's local space.
    /// Returns the continuous block-start coordinate for fragment selection.
    pub(crate) fn resolve_fragmented_block_offset(
        &mut self,
        composite_block_size: f32,
        extent: f32,
        fragment_offset: f32,
    ) -> f32 {
        let continuous = PositionConstraints::resolve_axis(
            self.constraints.edges.top,
            self.constraints.edges.bottom,
            composite_block_size,
            extent,
            self.insets.top,
        );
        self.insets.top = continuous - fragment_offset;
        if let Some(containing_block) = &mut self.containing_block {
            containing_block.height = composite_block_size;
        }
        continuous
    }

    pub(crate) const fn is_in_normal_flow(&self) -> bool {
        self.scheme.is_in_flow()
    }
}

/// Spacing that affects text advances rather than the surrounding box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct TextSpacing {
    pub(crate) letter: f32,
    pub(crate) word: f32,
}

impl TextSpacing {
    pub(crate) const fn new(letter: f32, word: f32) -> Self {
        Self { letter, word }
    }

    pub(crate) const fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self::new(style.letter_spacing, style.word_spacing)
    }

    fn internal_letter_advance(self, unit_count: usize) -> f32 {
        self.letter * unit_count.saturating_sub(1) as f32
    }

    /// Add tracking between the extended grapheme clusters in one paint run.
    ///
    /// CSS Text requires extended grapheme clusters as the baseline
    /// typographic-character unit. The outgoing boundary is deliberately not
    /// included here: it belongs to the nearest inline ancestor containing the
    /// adjacent units and is carried separately by `TextRun`.
    pub(crate) fn add_internal_advance(self, raw_width: f32, text: &str) -> f32 {
        let unit_count = unicode_segmentation::UnicodeSegmentation::graphemes(text, true).count();
        let spaces = text.chars().filter(|character| *character == ' ').count();
        raw_width + self.internal_letter_advance(unit_count) + self.word * spaces as f32
    }
}

/// Block-level text formatting kept separate from glyph-run styles.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextBlockStyle {
    pub(crate) alignment: TextAlign,
    pub(crate) writing_mode: WritingMode,
    pub(crate) indent: f32,
}

impl Default for TextBlockStyle {
    fn default() -> Self {
        Self {
            alignment: TextAlign::Left,
            writing_mode: WritingMode::HorizontalTb,
            indent: 0.0,
        }
    }
}

/// Overflow clip owned by a text box.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DescendantClip {
    pub(crate) rect: Option<Rect>,
}

/// Document-level meaning carried by a laid-out text block.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TextSemantics {
    pub(crate) heading_level: Option<u8>,
}

/// Per-axis overflow behavior of a container.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OverflowBehavior {
    pub(crate) combined: Overflow,
    pub(crate) x: Overflow,
    pub(crate) y: Overflow,
}

#[cfg(test)]
mod tests {
    use super::{BlockSize, BoxModel, InlineSize, Positioning, SizeConstraints};
    use crate::layout::elements::BoxFragmentation;
    use crate::style::computed::{BoxDecorationBreak, ComputedStyle, Position};
    use crate::types::{EdgeSizes, Point, Size};

    #[test]
    fn sticky_insets_constrain_normal_flow_instead_of_translating_it() {
        let style = ComputedStyle {
            position: Position::Sticky,
            top: Some(12.0),
            left: Some(12.0),
            ..ComputedStyle::default()
        };
        // Layout has folded the 12px normal-flow inline margin and the authored
        // 12px left inset into the legacy used inline offset.
        let positioning = Positioning::from_style(&style)
            .with_resolved_insets(EdgeSizes::new(12.0, 0.0, 0.0, 24.0));

        assert_eq!(
            positioning.resolve_in_flow_origin(
                Point::new(0.0, 12.0),
                Size::new(104.0, 96.0),
                Size::new(152.0, 136.0),
            ),
            Point::new(12.0, 12.0),
        );
    }

    #[test]
    fn sticky_start_and_end_constraints_move_only_outside_edges_inward() {
        let start = Positioning::from_style(&ComputedStyle {
            position: Position::Sticky,
            top: Some(12.0),
            left: Some(10.0),
            ..ComputedStyle::default()
        });
        assert_eq!(
            start.resolve_in_flow_origin(
                Point::ORIGIN,
                Size::new(30.0, 20.0),
                Size::new(120.0, 100.0),
            ),
            Point::new(10.0, 12.0),
        );

        let end = Positioning::from_style(&ComputedStyle {
            position: Position::Sticky,
            right: Some(10.0),
            bottom: Some(8.0),
            ..ComputedStyle::default()
        });
        assert_eq!(
            end.resolve_in_flow_origin(
                Point::new(100.0, 90.0),
                Size::new(30.0, 20.0),
                Size::new(120.0, 100.0),
            ),
            Point::new(80.0, 72.0),
        );
    }

    #[test]
    fn relative_insets_remain_visual_translations() {
        let positioning = Positioning::from_style(&ComputedStyle {
            position: Position::Relative,
            top: Some(8.0),
            left: Some(12.0),
            ..ComputedStyle::default()
        });

        assert_eq!(
            positioning.resolve_in_flow_origin(
                Point::new(4.0, 6.0),
                Size::new(30.0, 20.0),
                Size::new(120.0, 100.0),
            ),
            Point::new(16.0, 14.0),
        );
    }

    #[test]
    fn inline_size_distinguishes_fill_available_from_fixed_geometry() {
        assert_eq!(InlineSize::FILL_AVAILABLE.resolve(320.0), 320.0);
        assert_eq!(InlineSize::fixed(180.0).resolve(320.0), 180.0);
        assert!(InlineSize::from_used(320.0, 320.0, false).is_fill_available());
        assert_eq!(
            InlineSize::from_used(320.0, 320.0, true).fixed_value(),
            Some(320.0)
        );
        assert!(InlineSize::FILL_AVAILABLE.is_fill_available());
        assert_eq!(InlineSize::fixed(180.0).fixed_value(), Some(180.0));
    }

    #[test]
    fn margin_narrowed_width_is_used_but_not_preferred() {
        let narrowed = InlineSize::from_used(315.5, 320.0, false);
        assert_eq!(narrowed.fixed_value(), Some(315.5));
        assert_eq!(narrowed.resolve(320.0), 315.5);
        assert_eq!(narrowed.preferred_value(), None);
        assert_eq!(
            InlineSize::from_used(180.0, 320.0, true).preferred_value(),
            Some(180.0)
        );
        assert_eq!(InlineSize::fixed(180.0).preferred_value(), Some(180.0));
    }

    #[test]
    fn minimum_block_size_is_a_floor_not_a_content_cap() {
        let size = BlockSize::minimum(50.0);

        assert_eq!(size.resolve(20.0), 50.0);
        assert_eq!(size.resolve(80.0), 80.0);
        assert!(!size.is_definite());
    }

    #[test]
    fn fragment_continuation_keeps_only_the_unconsumed_minimum() {
        let minimum = BlockSize::minimum(680.0);

        assert_eq!(
            minimum.remaining_fragment_floor(312.0),
            BlockSize::minimum(368.0)
        );
        assert_eq!(
            minimum
                .remaining_fragment_floor(312.0)
                .remaining_fragment_floor(312.0),
            BlockSize::minimum(56.0)
        );
        assert_eq!(minimum.remaining_fragment_floor(680.0), BlockSize::AUTO);
        assert_eq!(
            BlockSize::AUTO.remaining_fragment_floor(10.0),
            BlockSize::AUTO
        );
    }

    #[test]
    fn splitting_used_extents_preserves_definite_and_minimum_semantics() {
        assert_eq!(
            BlockSize::definite(120.0).split_fragment_at(70.0),
            Some((BlockSize::definite(70.0), BlockSize::definite(50.0)))
        );
        assert_eq!(
            BlockSize::minimum(120.0).split_fragment_at(70.0),
            Some((BlockSize::fragment(70.0), BlockSize::minimum(50.0)))
        );
        assert_eq!(BlockSize::minimum(120.0).split_fragment_at(120.0), None);
    }

    #[test]
    fn minimum_size_wins_when_constraints_conflict() {
        let constraints = SizeConstraints::new(Some(68.0), Some(58.0));

        assert_eq!(constraints.constrain(40.0), 68.0);
        assert_eq!(constraints.constrain(80.0), 68.0);
        assert_eq!(constraints.constrain_preferred(None), Some(68.0));
    }

    #[test]
    fn maximum_alone_waits_for_automatic_content_size() {
        let constraints = SizeConstraints::new(None, Some(58.0));

        assert_eq!(constraints.constrain_preferred(None), None);
        assert_eq!(constraints.constrain(40.0), 40.0);
        assert_eq!(constraints.constrain(80.0), 58.0);
    }

    #[test]
    fn authored_minimum_remains_fragmentable_while_height_is_definite() {
        let minimum = BlockSize::from_style(
            &ComputedStyle {
                min_height: Some(50.0),
                ..ComputedStyle::default()
            },
            20.0,
        );
        let definite = BlockSize::from_style(
            &ComputedStyle {
                height: Some(50.0),
                ..ComputedStyle::default()
            },
            20.0,
        );

        assert_eq!(minimum.resolve(80.0), 80.0);
        assert!(!minimum.is_definite());
        assert_eq!(definite.resolve(80.0), 50.0);
        assert!(definite.is_definite());
    }

    #[test]
    fn auto_block_size_retains_its_intrinsic_used_extent() {
        let size = BlockSize::from_style(&ComputedStyle::default(), 72.0);

        assert_eq!(size.used(), Some(72.0));
        assert_eq!(size.resolve(0.0), 72.0);
        assert_eq!(size.resolve(96.0), 96.0);
        assert!(!size.is_definite());
    }

    #[test]
    fn repeated_slice_keeps_one_composite_reference_box() {
        let fragmentation = BoxFragmentation {
            decoration: BoxDecorationBreak::Slice,
            ..Default::default()
        };
        let (first, continuation) = fragmentation
            .split_reference_box(80.0, 120.0, &BoxModel::default())
            .expect("slice decoration has reference geometry");
        let continuation = BoxFragmentation {
            decoration: BoxDecorationBreak::Slice,
            reference_slice: Some(continuation),
            ..Default::default()
        };
        let (second, third) = continuation
            .split_reference_box(50.0, 70.0, &BoxModel::default())
            .expect("a continuation retains reference geometry");

        assert_eq!(first.block_offset(), 0.0);
        assert_eq!(second.block_offset(), 80.0);
        assert_eq!(third.block_offset(), 130.0);
        assert_eq!(first.composite_block_size(), 200.0);
        assert_eq!(second.composite_block_size(), 200.0);
        assert_eq!(third.composite_block_size(), 200.0);
    }
}
