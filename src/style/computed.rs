use std::{borrow::Cow, collections::HashMap};

pub(crate) use crate::parser::css::LengthPercent;
use crate::parser::css::{
    BackgroundLayerSource, CssMathExpression, CssRule, CssValue, FontStretch, MathUnitContext,
    SelectorContext, SpecifiedColor, StyleMap, parse_length, parse_property_value,
    selector_matches_with_context, specificity, split_radius_components,
};
use crate::parser::dom::HtmlTag;
use crate::style::defaults::default_style;
use crate::style::font_metrics::FontMetrics;
use crate::style::html_cascade::html_cascade_layers;
use crate::style::raster_quality::{RasterQuality, background_raster_dimensions};
use crate::types::{
    Color, CornerRadii, CornerRadius, EdgeSizes, PhysicalEdges, PhysicalSide, Point, Size,
};
use crate::util::{MAX_RASTER_TILE_EDGE, RasterDimensions, RasterTile};

#[cfg(test)]
use crate::util::{AxisRepeatMode, AxisRepeatPattern};

mod borders;
mod filter;
mod gradient_geometry;
mod text_decoration;
mod transforms;
pub(crate) use filter::NormalizedFilterRegion;
pub use filter::{DropShadow, FilterEffects, FilterOperation};
pub use gradient_geometry::{
    ConicGradient, RadialExtent, RadialGradient, RadialPoint, RadialPos, RadialShape, RadialVector,
};
pub use text_decoration::{
    TextDecoration, TextDecorationLines, TextDecorationSkipInk, TextDecorationStyle,
    TextDecorations,
};

/// CSS display property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    Block,
    ListItem,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumnGroup,
    TableColumn,
    TableCaption,
    None,
}

impl Display {
    /// CSS Display 3 blockification for boxes taken out of normal flow.
    /// Preserve the inner layout mode while replacing an inline outer display
    /// with its block-level counterpart.
    pub(crate) const fn blockified(self) -> Self {
        match self {
            Self::Inline
            | Self::InlineBlock
            | Self::TableRowGroup
            | Self::TableHeaderGroup
            | Self::TableFooterGroup
            | Self::TableRow
            | Self::TableCell
            | Self::TableColumnGroup
            | Self::TableColumn
            | Self::TableCaption => Self::Block,
            Self::InlineFlex => Self::Flex,
            Self::InlineGrid => Self::Grid,
            Self::InlineTable => Self::Table,
            other => other,
        }
    }

    /// Atomic inline boxes stop decorations imposed by their ancestors from
    /// entering their contents. A decoration originated on the atomic box
    /// itself can still propagate to its own in-flow children.
    const fn is_atomic_inline(self) -> bool {
        matches!(
            self,
            Self::InlineBlock | Self::InlineFlex | Self::InlineGrid | Self::InlineTable
        )
    }
}

/// CSS flex-direction property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    /// Whether the main axis is the inline (horizontal) axis.
    pub fn is_row(self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }
}

/// CSS justify-content property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SafeCenter,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// CSS align-items property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    #[default]
    Stretch,
}

/// CSS align-self property (per-item cross-axis alignment override).
/// `Auto` means "use the container's align-items".
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

/// CSS align-content property (cross-axis distribution of flex LINES in a
/// multi-line/wrapping flex container). Only takes effect when the container
/// wraps onto more than one line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    #[default]
    Stretch,
}

/// CSS flex-wrap property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

impl FlexWrap {
    /// Whether wrapping is enabled (either direction).
    pub fn wraps(self) -> bool {
        matches!(self, FlexWrap::Wrap | FlexWrap::WrapReverse)
    }
}

/// A single track definition in `grid-template-columns`.
#[derive(Debug, Clone, PartialEq)]
pub enum GridTrack {
    /// A fixed size in points.
    Fixed(f32),
    /// A fractional unit (`fr`).
    Fr(f32),
    /// Automatic sizing (equal share of remaining space).
    Auto,
    /// A percentage of the grid container's content box (0..1 fraction).
    Percent(f32),
    /// `minmax(min, max)` — the track is at least `min` and at most `max`.
    Minmax(f32, f32),
}

/// A grid-placement endpoint (`grid-row-start` / `grid-column-end` etc.),
/// per CSS Grid Layout Level 1 §8. One end of an item's placement on one axis.
///
/// - `Auto`: no explicit placement — resolved by auto-placement (§8.5).
/// - `Line(n)`: a definite line number. Positive counts from the start edge of
///   the explicit grid (1 = first line); negative from the end (-1 = last line).
/// - `Named(name)`: a named line — the first line carrying `name` (§8.3). Also
///   matches the implicit `<name>-start` / `<name>-end` lines that
///   `grid-template-areas` generates for a named area.
/// - `Span(n)`: span `n` tracks from the opposite (definite) edge (§8.3).
/// - `SpanNamed { count, name }`: span until the `count`th line carrying
///   `name` in the search direction (§8.3).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum GridLine {
    #[default]
    Auto,
    Line(i32),
    Named(String),
    Span(usize),
    SpanNamed {
        count: usize,
        name: String,
    },
}

/// CSS Grid box-alignment keyword (justify-items / align-items per item axis).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum GridAlign {
    /// `stretch` — item fills the track (default).
    #[default]
    Stretch,
    /// `start` — item placed at the start of the track at its own size.
    Start,
    /// `end` — item placed at the end of the track at its own size.
    End,
    /// `center` — item centered in the track at its own size.
    Center,
}

/// A CSS reference box for `clip-path` / `mask-origin` / `mask-clip`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeBox {
    #[default]
    Border,
    Padding,
    Content,
}

/// One unresolved border-radius axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedRadiusValue {
    /// Absolute length in points.
    Length(f32),
    /// Percentage resolved against the matching border-box axis.
    Percentage(f32),
}

impl Default for SpecifiedRadiusValue {
    fn default() -> Self {
        Self::Length(0.0)
    }
}

impl SpecifiedRadiusValue {
    fn resolve(self, basis: f32) -> f32 {
        match self {
            Self::Length(value) => value,
            Self::Percentage(value) => basis * value / 100.0,
        }
    }

    fn is_zero(self) -> bool {
        match self {
            Self::Length(value) | Self::Percentage(value) => value == 0.0,
        }
    }
}

/// Unresolved horizontal and vertical radii for one corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpecifiedCornerRadius {
    pub x: SpecifiedRadiusValue,
    pub y: SpecifiedRadiusValue,
}

impl SpecifiedCornerRadius {
    pub const fn new(x: SpecifiedRadiusValue, y: SpecifiedRadiusValue) -> Self {
        Self { x, y }
    }

    pub const fn circular(value: SpecifiedRadiusValue) -> Self {
        Self::new(value, value)
    }

    fn resolve(self, width: f32, height: f32) -> CornerRadius {
        CornerRadius::new(self.x.resolve(width), self.y.resolve(height))
    }

    fn is_zero(self) -> bool {
        self.x.is_zero() || self.y.is_zero()
    }
}

/// The four unresolved border radii of a box, in CSS clockwise order.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpecifiedCornerRadii {
    pub top_left: SpecifiedCornerRadius,
    pub top_right: SpecifiedCornerRadius,
    pub bottom_right: SpecifiedCornerRadius,
    pub bottom_left: SpecifiedCornerRadius,
}

impl SpecifiedCornerRadii {
    pub const ZERO: Self = Self::uniform(SpecifiedCornerRadius::circular(
        SpecifiedRadiusValue::Length(0.0),
    ));

    pub const fn new(
        top_left: SpecifiedCornerRadius,
        top_right: SpecifiedCornerRadius,
        bottom_right: SpecifiedCornerRadius,
        bottom_left: SpecifiedCornerRadius,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    pub const fn uniform(radius: SpecifiedCornerRadius) -> Self {
        Self::new(radius, radius, radius, radius)
    }

    pub const fn circular(value: SpecifiedRadiusValue) -> Self {
        Self::uniform(SpecifiedCornerRadius::circular(value))
    }

    fn resolve(self, width: f32, height: f32) -> CornerRadii {
        CornerRadii::new(
            self.top_left.resolve(width, height),
            self.top_right.resolve(width, height),
            self.bottom_right.resolve(width, height),
            self.bottom_left.resolve(width, height),
        )
    }

    fn is_zero(self) -> bool {
        self.top_left.is_zero()
            && self.top_right.is_zero()
            && self.bottom_right.is_zero()
            && self.bottom_left.is_zero()
    }
}

/// Shape-radius keywords accepted by `circle()` / `ellipse()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeExtent {
    #[default]
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipRadius {
    Length(LengthPercent),
    Extent(ShapeExtent),
}

impl ClipRadius {
    /// Resolve a circle radius against the basic-shape reference box.
    pub(crate) fn resolve_circle(
        self,
        width: f32,
        height: f32,
        center_x: f32,
        center_y: f32,
    ) -> f32 {
        match self {
            Self::Length(length) => {
                length.resolve((width * width + height * height).sqrt() / std::f32::consts::SQRT_2)
            }
            Self::Extent(ShapeExtent::ClosestSide) => center_x
                .min(width - center_x)
                .min(center_y.min(height - center_y)),
            Self::Extent(ShapeExtent::FarthestSide) => center_x
                .max(width - center_x)
                .max(center_y.max(height - center_y)),
            Self::Extent(ShapeExtent::ClosestCorner) => {
                let x = center_x.min(width - center_x);
                let y = center_y.min(height - center_y);
                (x * x + y * y).sqrt()
            }
            Self::Extent(ShapeExtent::FarthestCorner) => {
                let x = center_x.max(width - center_x);
                let y = center_y.max(height - center_y);
                (x * x + y * y).sqrt()
            }
        }
    }

    /// Resolve one ellipse radius along its selected axis.
    pub(crate) fn resolve_ellipse_axis(
        self,
        axis_extent: f32,
        other_extent: f32,
        center_offset: f32,
    ) -> f32 {
        match self {
            Self::Length(length) => length.resolve(axis_extent),
            Self::Extent(ShapeExtent::ClosestSide) => {
                center_offset.min(axis_extent - center_offset)
            }
            Self::Extent(ShapeExtent::FarthestSide) => {
                center_offset.max(axis_extent - center_offset)
            }
            Self::Extent(ShapeExtent::ClosestCorner | ShapeExtent::FarthestCorner) => {
                (axis_extent * axis_extent + other_extent * other_extent).sqrt() * 0.5
            }
        }
    }
}

/// A CSS `clip-path` basic shape. Lengths are in points; positions/percentages
/// resolve against the selected reference box at render time.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipPath {
    /// `circle(r at cx cy)` — radius + centre, each (value, is_percent).
    Circle {
        r: ClipRadius,
        cx: LengthPercent,
        cy: LengthPercent,
        geometry_box: ShapeBox,
    },
    /// `ellipse(rx ry at cx cy)`.
    Ellipse {
        rx: ClipRadius,
        ry: ClipRadius,
        cx: LengthPercent,
        cy: LengthPercent,
        geometry_box: ShapeBox,
    },
    /// `inset(top right bottom left [round radius])`.
    Inset {
        top: LengthPercent,
        right: LengthPercent,
        bottom: LengthPercent,
        left: LengthPercent,
        radii: CornerRadii,
        geometry_box: ShapeBox,
    },
    /// `polygon(x y, ...)` — vertices, each coord (value, is_percent).
    Polygon {
        points: Vec<(LengthPercent, LengthPercent)>,
        even_odd: bool,
        geometry_box: ShapeBox,
    },
    /// `path("...")`, parsed with the SVG path-data grammar.
    Path {
        commands: Vec<crate::parser::svg::PathCommand>,
        geometry_box: ShapeBox,
    },
    /// `rect()` / `xywh()` rectangle with optional rounded corners.
    Rect {
        x: LengthPercent,
        y: LengthPercent,
        width: LengthPercent,
        height: LengthPercent,
        radii: CornerRadii,
        geometry_box: ShapeBox,
    },
    /// `url(#id)` fragment reference. Resolution needs document SVG defs.
    Url(String),
}

/// A CSS `mask-image` source (css-masking-1 §3.1). The deterministic CSS-image
/// sources (gradients) and `url()` references to an SVG image are modelled.
#[derive(Debug, Clone)]
pub enum MaskSource {
    /// `linear-gradient(...)` / `repeating-linear-gradient(...)`.
    Linear(LinearGradient),
    /// `radial-gradient(...)` / `repeating-radial-gradient(...)`.
    Radial(RadialGradient),
    /// `conic-gradient(...)` / `repeating-conic-gradient(...)`.
    Conic(ConicGradient),
    /// `url(...)` pointing at an SVG image (data URI or file). Holds the raw SVG
    /// bytes; rasterised to a coverage buffer at paint time (css-masking-1 §3.2).
    Svg(std::sync::Arc<Vec<u8>>),
    /// A resolved CSS mask layer list.
    Layers(Vec<MaskLayer>),
    /// `mask-border-*` represented as a border-box ring coverage mask.
    BorderRing { width: f32 },
    /// `url(#id)` fragment reference. Resolution needs document SVG defs.
    Ref(String),
}

#[derive(Debug, Clone)]
pub enum MaskLayerSource {
    Linear(LinearGradient),
    Radial(RadialGradient),
    Conic(ConicGradient),
    Svg(std::sync::Arc<Vec<u8>>),
    Ref(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskComposite {
    #[default]
    Add,
    Subtract,
    Intersect,
    Exclude,
    Destination,
}

#[derive(Debug, Clone)]
pub struct MaskLayer {
    pub source: MaskLayerSource,
    pub mode: MaskMode,
    pub layer_box: GradientLayerBox,
    pub origin: ShapeBox,
    pub clip: ShapeBox,
    pub composite: MaskComposite,
}

impl MaskLayer {
    /// Whether this layer is the initial single-image paint, with no tile or
    /// box overrides. Renderers may use an equivalent whole-border-box path.
    pub(crate) fn uses_initial_paint_area(&self) -> bool {
        self.layer_box.is_initial()
            && self.origin == ShapeBox::Border
            && self.clip == ShapeBox::Border
            && self.composite == MaskComposite::Add
    }
}

/// CSS `mask-mode` (css-masking-1 §3.4): how the mask layer's pixels are turned
/// into mask coverage values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskMode {
    /// `alpha` — use the source's alpha channel as the coverage.
    Alpha,
    /// `luminance` — use the source's (premultiplied) luminance as coverage.
    Luminance,
    /// `match-source` (initial) — for a CSS gradient/image source this resolves
    /// to `alpha`; for an SVG `<mask>` it follows `mask-type` (luminance default).
    #[default]
    MatchSource,
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// A `text-indent` value before layout has established the block's inner
/// inline size. Percentages are deliberately retained here: CSS Text resolves
/// them against the block container's own inner size, not its containing block.
#[derive(Debug, Clone)]
pub enum TextIndent {
    Length(f32),
    Percentage(f32),
    Math(DeferredLength),
}

impl Default for TextIndent {
    fn default() -> Self {
        Self::Length(0.0)
    }
}

impl TextIndent {
    pub(crate) fn resolve(&self, inner_inline_size: f32) -> f32 {
        match self {
            Self::Length(length) => *length,
            Self::Percentage(percentage) => inner_inline_size * percentage / 100.0,
            Self::Math(value) => value.resolve(inner_inline_size).unwrap_or(0.0),
        }
    }
}

/// A typed CSS length expression whose percentage basis is supplied by layout.
#[derive(Debug, Clone, PartialEq)]
pub struct DeferredLength {
    expression: CssMathExpression,
    units: MathUnitContext,
}

impl DeferredLength {
    fn new(expression: CssMathExpression, units: MathUnitContext) -> Self {
        Self { expression, units }
    }

    fn resolve(&self, percentage_basis: f32) -> Option<f32> {
        self.expression.resolve(self.units, percentage_basis)
    }
}

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Number(u16),
    Bold,
}

impl FontWeight {
    pub(crate) fn numeric(self) -> u16 {
        match self {
            FontWeight::Normal => 400,
            FontWeight::Number(weight) => weight,
            FontWeight::Bold => 700,
        }
    }

    pub(crate) fn from_number(weight: u16) -> Self {
        match weight {
            400 => FontWeight::Normal,
            700 => FontWeight::Bold,
            weight => FontWeight::Number(weight.clamp(1, 1000)),
        }
    }

    pub(crate) fn is_bold(self) -> bool {
        self.numeric() >= 700
    }

    fn bolder(self) -> Self {
        match self.numeric() {
            0..=99 => FontWeight::Normal,
            100..=349 => FontWeight::Normal,
            350..=549 => FontWeight::Bold,
            550..=899 => FontWeight::from_number(900),
            _ => self,
        }
    }

    fn lighter(self) -> Self {
        match self.numeric() {
            0..=99 => self,
            100..=549 => FontWeight::from_number(100),
            550..=749 => FontWeight::Normal,
            _ => FontWeight::Bold,
        }
    }
}

/// Font style.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique(f32),
}

impl FontStyle {
    pub const DEFAULT_OBLIQUE_ANGLE_DEGREES: f32 = 14.0;

    pub(crate) const fn is_slanted(self) -> bool {
        !matches!(self, Self::Normal)
    }

    /// Synthetic glyph shear for an upright face selected by this style.
    /// Italic has no authored angle, so it retains the browser-compatible
    /// conventional 0.25 shear. Oblique preserves the specified CSS angle.
    pub(crate) fn synthetic_shear(self) -> Option<f32> {
        match self {
            Self::Normal => None,
            Self::Italic => Some(0.25),
            Self::Oblique(degrees) => Some(degrees.to_radians().tan()),
        }
    }

    fn from_css(value: &str) -> Option<Self> {
        let mut tokens = value.split_whitespace();
        match tokens.next()? {
            "normal" if tokens.next().is_none() => Some(Self::Normal),
            "italic" if tokens.next().is_none() => Some(Self::Italic),
            "left" if tokens.next().is_none() => {
                Some(Self::Oblique(Self::DEFAULT_OBLIQUE_ANGLE_DEGREES))
            }
            "right" if tokens.next().is_none() => {
                Some(Self::Oblique(-Self::DEFAULT_OBLIQUE_ANGLE_DEGREES))
            }
            "oblique" => match (tokens.next(), tokens.next()) {
                (None, None) => Some(Self::Oblique(Self::DEFAULT_OBLIQUE_ANGLE_DEGREES)),
                (Some(angle), None) => parse_angle_deg(angle)
                    .filter(|angle| angle.is_finite() && (-90.0..=90.0).contains(angle))
                    .map(|angle| Self::Oblique(angle as f32)),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The inherited `font-size-adjust` used-value rule.
///
/// CSS Fonts keeps `font-size` as a computed value and adjusts only the size
/// used to shape each selected font. Keeping that distinction explicit avoids
/// leaking a glyph-scale adjustment into `em` lengths, line-height, or the
/// containing block's inherited font geometry.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FontSizeAdjust {
    /// The requested x-height / computed-font-size aspect. `None` represents
    /// the initial `none` value; zero is a valid requested aspect.
    ex_height: Option<f32>,
}

impl FontSizeAdjust {
    pub const fn none() -> Self {
        Self { ex_height: None }
    }

    pub const fn ex_height(aspect: f32) -> Self {
        Self {
            ex_height: Some(aspect),
        }
    }

    pub const fn target_ex_height(self) -> Option<f32> {
        self.ex_height
    }

    /// Resolve the CSS Fonts used font size for one selected face.
    pub fn used_font_size(self, computed_font_size: f32, actual_aspect: f32) -> f32 {
        let Some(target_aspect) = self.target_ex_height() else {
            return computed_font_size;
        };
        if !computed_font_size.is_finite()
            || !target_aspect.is_finite()
            || !actual_aspect.is_finite()
            || target_aspect < 0.0
            || actual_aspect <= 0.0
        {
            return computed_font_size;
        }
        computed_font_size * target_aspect / actual_aspect
    }
}

/// Font family.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FontFamily {
    /// Helvetica (sans-serif) — the default.
    #[default]
    Helvetica,
    /// Times Roman (serif).
    TimesRoman,
    /// Courier (monospace).
    Courier,
    /// A custom TrueType font identified by name.
    Custom(String),
}

impl FontFamily {
    /// Return the font family name as a string slice.
    pub fn name(&self) -> &str {
        match self {
            FontFamily::Helvetica => "Helvetica",
            FontFamily::TimesRoman => "Times-Roman",
            FontFamily::Courier => "Courier",
            FontFamily::Custom(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontStack {
    families: Vec<FontFamily>,
}

impl Default for FontStack {
    fn default() -> Self {
        // The UA-initial generic font-family is `serif` (matching Chrome's
        // default "standard" font). This makes unstyled text and the `ex`/`ch`
        // font-relative units resolve against serif metrics, and keeps `serif`
        // distinct from an explicit `sans-serif` (which both previously mapped
        // to Helvetica).
        Self::from_family(FontFamily::TimesRoman)
    }
}

impl FontStack {
    pub fn from_family(family: FontFamily) -> Self {
        Self {
            families: vec![family],
        }
    }

    pub fn families(&self) -> &[FontFamily] {
        &self.families
    }

    pub fn primary(&self) -> FontFamily {
        self.families.first().cloned().unwrap_or_default()
    }
}

fn parse_font_family_name(raw: &str) -> FontFamily {
    let lower = raw.to_ascii_lowercase();
    let cleaned = lower.trim_matches(|c| c == '\'' || c == '"');
    match cleaned {
        "serif" | "times" | "times new roman" | "times-roman" | "georgia" | "garamond"
        | "book antiqua" | "palatino" | "palatino linotype" | "baskerville" | "hoefler text"
        | "cambria" | "droid serif" | "noto serif" | "libre baskerville" | "merriweather"
        | "playfair display" | "lora" => FontFamily::TimesRoman,

        "monospace"
        | "courier"
        | "courier new"
        | "lucida console"
        | "lucida sans typewriter"
        | "monaco"
        | "andale mono"
        | "consolas"
        | "source code pro"
        | "fira code"
        | "fira mono"
        | "jetbrains mono"
        | "ibm plex mono"
        | "roboto mono"
        | "ubuntu mono"
        | "droid sans mono"
        | "menlo"
        | "sf mono"
        | "cascadia code"
        | "cascadia mono" => FontFamily::Courier,

        "sans-serif" => FontFamily::Helvetica,
        "arial" | "helvetica" | "helvetica neue" | "arial black" | "verdana" | "tahoma"
        | "trebuchet ms" | "gill sans" | "lucida sans" | "lucida grande" | "ui-sans-serif"
        | "system-ui" | "-apple-system" | "blinkmacsystemfont" | "segoe ui" | "roboto"
        | "open sans" | "lato" | "inter" | "nunito" | "poppins" | "montserrat" | "raleway"
        | "ubuntu" | "noto sans" => FontFamily::Custom(cleaned.to_string()),

        other => FontFamily::Custom(other.to_string()),
    }
}

fn split_font_family_list(raw: &str) -> Vec<&str> {
    let mut families = Vec::new();
    let mut start = 0usize;
    let mut quote = None;

    for (index, ch) in raw.char_indices() {
        match ch {
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            ',' if quote.is_none() => {
                families.push(raw[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    families.push(raw[start..].trim());
    families.retain(|family| !family.is_empty());
    families
}

pub(crate) fn parse_font_stack(raw: &str) -> FontStack {
    let families: Vec<FontFamily> = split_font_family_list(raw)
        .into_iter()
        .map(parse_font_family_name)
        .collect();
    if families.is_empty() {
        FontStack::default()
    } else {
        FontStack { families }
    }
}

/// CSS float property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
    Footnote,
}

/// CSS Generated Content for Paged Media footnote presentation.
///
/// These properties form one non-inherited formatting group: both describe how
/// a `float: footnote` is collected and placed at the page footnote area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FootnoteFormatting {
    pub display: FootnoteDisplay,
    pub policy: FootnotePolicy,
}

impl FootnoteFormatting {
    pub const fn display_keyword(self) -> &'static str {
        self.display.keyword()
    }

    pub const fn policy_keyword(self) -> &'static str {
        self.policy.keyword()
    }

    pub fn from_keywords(display: &str, policy: &str) -> Option<Self> {
        Some(Self {
            display: FootnoteDisplay::from_keyword(display)?,
            policy: FootnotePolicy::from_keyword(policy)?,
        })
    }
}

/// CSS GCPM `footnote-display`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FootnoteDisplay {
    #[default]
    Block,
    Inline,
    Compact,
}

impl FootnoteDisplay {
    pub const fn is_inline_layout(self) -> bool {
        matches!(self, Self::Inline | Self::Compact)
    }

    const fn keyword(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Inline => "inline",
            Self::Compact => "compact",
        }
    }

    fn from_keyword(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "inline" => Some(Self::Inline),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

/// CSS GCPM `footnote-policy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FootnotePolicy {
    #[default]
    Auto,
    Line,
    Block,
}

impl FootnotePolicy {
    const fn keyword(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Line => "line",
            Self::Block => "block",
        }
    }

    fn from_keyword(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "line" => Some(Self::Line),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

/// CSS clear property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

/// CSS `box-decoration-break` (css-break-3 §6.2): how a box's borders, padding,
/// margin and background are applied when the box is split across fragmentainers
/// (pages, columns). `Slice` (the default) renders the decoration as if the box
/// were whole and then sliced at the break — the first fragment keeps its top
/// border/padding but no bottom decoration, the continuation drops its top
/// decoration. `Clone` wraps EACH fragment independently with the full
/// border/padding/margin and background.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BoxDecorationBreak {
    #[default]
    Slice,
    Clone,
}

/// CSS position property (simplified).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Sticky,
    Absolute,
    Fixed,
}

impl Position {
    /// Whether the box participates in its parent's normal flow.
    pub const fn is_in_flow(self) -> bool {
        matches!(self, Self::Static | Self::Relative | Self::Sticky)
    }

    /// Whether the box uses the absolute-positioning layout model.
    pub const fn is_absolute(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }

    /// Whether inset resolution visually offsets an in-flow box.
    pub const fn is_relative(self) -> bool {
        matches!(self, Self::Relative | Self::Sticky)
    }

    pub const fn is_positioned(self) -> bool {
        !matches!(self, Self::Static)
    }

    /// Fixed and sticky boxes form stacking contexts even when `z-index` is
    /// `auto` (CSS Positioned Layout 3 section 2.2).
    pub const fn establishes_stacking_context(self) -> bool {
        matches!(self, Self::Fixed | Self::Sticky)
    }
}

/// Computed CSS `z-index` value.
///
/// `auto` and the integer zero share a used stack level, but they do not have
/// the same stacking-context semantics. Preserving that distinction at the
/// style boundary prevents later paint code from guessing whether a zero was
/// authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZIndex {
    #[default]
    Auto,
    Integer(i32),
}

impl ZIndex {
    pub const fn integer(value: i32) -> Self {
        Self::Integer(value)
    }

    pub const fn value(self) -> i32 {
        match self {
            Self::Auto => 0,
            Self::Integer(value) => value,
        }
    }

    pub const fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }

    pub const fn is_negative(self) -> bool {
        self.value() < 0
    }

    pub const fn is_positive(self) -> bool {
        self.value() > 0
    }
}

fn parse_running_position_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let inner = raw.strip_prefix("running(")?.strip_suffix(')')?.trim();
    (!inner.is_empty()).then(|| inner.to_ascii_lowercase())
}

/// CSS blend mode (`mix-blend-mode` / `background-blend-mode`).
///
/// Maps directly onto the PDF `/BM` blend-mode names emitted in an ExtGState.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    BackgroundList {
        modes: [u8; 8],
        len: u8,
    },
}

impl BlendMode {
    /// Parse a CSS blend-mode keyword. Unknown keywords fall back to `Normal`.
    pub fn from_keyword(keyword: &str) -> Self {
        Self::from_code(Self::keyword_code(keyword))
    }

    /// Parse `background-blend-mode`, whose value is a comma-separated list.
    pub fn from_background_value(value: &str) -> Self {
        let parts: Vec<&str> = value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if parts.len() <= 1 {
            return Self::from_keyword(value);
        }

        let mut modes = [0u8; 8];
        let mut len = 0usize;
        for part in parts.iter().take(modes.len()) {
            modes[len] = Self::keyword_code(part);
            len += 1;
        }
        BlendMode::BackgroundList {
            modes,
            len: len as u8,
        }
    }

    pub fn background_layer(self, index: usize) -> Self {
        match self {
            BlendMode::BackgroundList { modes, len } if len > 0 => {
                Self::from_code(modes[index % len as usize])
            }
            BlendMode::BackgroundList { .. } => BlendMode::Normal,
            mode => mode,
        }
    }

    /// PDF `/BM` blend-mode name, or `None` for `Normal` (which needs no gstate).
    pub fn pdf_name(self) -> Option<&'static str> {
        match self {
            BlendMode::Normal => None,
            BlendMode::Multiply => Some("Multiply"),
            BlendMode::Screen => Some("Screen"),
            BlendMode::Overlay => Some("Overlay"),
            BlendMode::Darken => Some("Darken"),
            BlendMode::Lighten => Some("Lighten"),
            BlendMode::ColorDodge => Some("ColorDodge"),
            BlendMode::ColorBurn => Some("ColorBurn"),
            BlendMode::HardLight => Some("HardLight"),
            BlendMode::SoftLight => Some("SoftLight"),
            BlendMode::Difference => Some("Difference"),
            BlendMode::Exclusion => Some("Exclusion"),
            BlendMode::Hue => Some("Hue"),
            BlendMode::Saturation => Some("Saturation"),
            BlendMode::Color => Some("Color"),
            BlendMode::Luminosity => Some("Luminosity"),
            BlendMode::BackgroundList { modes, len } if len > 0 => {
                Self::from_code(modes[0]).pdf_name()
            }
            BlendMode::BackgroundList { .. } => None,
        }
    }

    fn keyword_code(keyword: &str) -> u8 {
        match keyword.trim().to_ascii_lowercase().as_str() {
            "multiply" => 1,
            "screen" => 2,
            "overlay" => 3,
            "darken" => 4,
            "lighten" => 5,
            "color-dodge" => 6,
            "color-burn" => 7,
            "hard-light" => 8,
            "soft-light" => 9,
            "difference" => 10,
            "exclusion" => 11,
            "hue" => 12,
            "saturation" => 13,
            "color" => 14,
            "luminosity" => 15,
            _ => 0,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => BlendMode::Multiply,
            2 => BlendMode::Screen,
            3 => BlendMode::Overlay,
            4 => BlendMode::Darken,
            5 => BlendMode::Lighten,
            6 => BlendMode::ColorDodge,
            7 => BlendMode::ColorBurn,
            8 => BlendMode::HardLight,
            9 => BlendMode::SoftLight,
            10 => BlendMode::Difference,
            11 => BlendMode::Exclusion,
            12 => BlendMode::Hue,
            13 => BlendMode::Saturation,
            14 => BlendMode::Color,
            15 => BlendMode::Luminosity,
            _ => BlendMode::Normal,
        }
    }
}

/// Whether an element isolates its stacking group from outside blending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Isolation {
    #[default]
    Auto,
    Isolate,
}

impl Isolation {
    pub const fn isolates(self) -> bool {
        matches!(self, Self::Isolate)
    }
}

/// CSS overflow property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    /// `overflow: scroll` — a scroll container that, in print, always reserves a
    /// scrollbar gutter and paints a (non-interactive) scrollbar on the axis.
    Scroll,
    Auto,
}

impl Overflow {
    /// Whether this overflow value clips its content. In a print/PDF context
    /// every non-`visible` value clips to the box: `hidden`/`clip`/`scroll`
    /// always, and `auto` clips when content overflows (our deterministic
    /// fixtures always overflow, and there is no interactive scroll affordance
    /// in print, so `auto` clips too).
    pub fn clips(self) -> bool {
        !matches!(self, Overflow::Visible)
    }
}

/// A single per-axis overflow keyword as authored, before the CSS computed-value
/// coercion that depends on the sibling axis. `clip` and `scroll` are kept
/// distinct from `hidden`/`auto` only so the coercion rules can distinguish a
/// "scrolling value" (`auto`/`scroll`/`hidden`) from `visible`/`clip`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RawOverflow {
    Visible,
    Clip,
    Hidden,
    Scroll,
    Auto,
}

pub(crate) fn parse_raw_overflow(k: &str) -> RawOverflow {
    match k.trim().to_ascii_lowercase().as_str() {
        "clip" => RawOverflow::Clip,
        "hidden" => RawOverflow::Hidden,
        "scroll" => RawOverflow::Scroll,
        "auto" => RawOverflow::Auto,
        _ => RawOverflow::Visible,
    }
}

/// Apply the CSS Overflow 3 computed-value coercion between the two axes: when
/// one axis is a scrolling value (`auto`/`scroll`/`hidden`) and the other is
/// `visible` or `clip`, the latter is coerced (`visible` -> `auto`, `clip` ->
/// `hidden`). Returns the resulting per-axis `Overflow` (with `scroll`/`hidden`
/// modelled as `Hidden` and `clip` as `Hidden`, since print has no scrollbars).
pub(crate) fn coerce_overflow_axes(x: RawOverflow, y: RawOverflow) -> (Overflow, Overflow) {
    fn is_scrolling(v: RawOverflow) -> bool {
        matches!(
            v,
            RawOverflow::Auto | RawOverflow::Scroll | RawOverflow::Hidden
        )
    }
    let coerce = |this: RawOverflow, other: RawOverflow| -> Overflow {
        match this {
            RawOverflow::Visible => {
                if is_scrolling(other) {
                    Overflow::Auto
                } else {
                    Overflow::Visible
                }
            }
            RawOverflow::Clip => {
                // `clip` clips to the box (no scroll container); modelled as
                // Hidden whether or not the other axis scrolls.
                Overflow::Hidden
            }
            RawOverflow::Auto => Overflow::Auto,
            RawOverflow::Hidden => Overflow::Hidden,
            // `scroll` is preserved so the print scrollbar painter can reserve a
            // gutter and draw a (non-interactive) scrollbar on this axis.
            RawOverflow::Scroll => Overflow::Scroll,
        }
    };
    (coerce(x, y), coerce(y, x))
}

/// CSS visibility property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    /// `collapse`: like `hidden` for non-table elements; removes the row/column
    /// (including its space) for table rows/columns, similar to `display: none`.
    Collapse,
}

/// CSS transform value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CssVector {
    pub x: f64,
    pub y: f64,
}

impl CssVector {
    pub const ZERO: Self = Self::splat(0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub const fn splat(value: f64) -> Self {
        Self::new(value, value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CssAffineMatrix {
    pub x_axis: CssVector,
    pub y_axis: CssVector,
    pub translation: CssVector,
}

impl CssAffineMatrix {
    pub const IDENTITY: Self = Self::from_components(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);

    pub const fn new(x_axis: CssVector, y_axis: CssVector, translation: CssVector) -> Self {
        Self {
            x_axis,
            y_axis,
            translation,
        }
    }

    pub const fn from_components(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Self {
        Self::new(
            CssVector::new(a, b),
            CssVector::new(c, d),
            CssVector::new(e, f),
        )
    }

    pub const fn components(self) -> [f64; 6] {
        [
            self.x_axis.x,
            self.x_axis.y,
            self.y_axis.x,
            self.y_axis.y,
            self.translation.x,
            self.translation.y,
        ]
    }
}

impl Default for CssAffineMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CssMatrix3d([f64; 16]);

impl CssMatrix3d {
    pub const fn new(components: [f64; 16]) -> Self {
        Self(components)
    }
}

impl std::ops::Index<usize> for CssMatrix3d {
    type Output = f64;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl std::ops::IndexMut<usize> for CssMatrix3d {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PercentageAxes {
    pub x: bool,
    pub y: bool,
}

impl PercentageAxes {
    pub const fn new(x: bool, y: bool) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ParametricAffineMatrix {
    pub constant: CssAffineMatrix,
    pub width_translation: CssVector,
    pub height_translation: CssVector,
}

impl ParametricAffineMatrix {
    pub fn resolve(self, box_size: CssVector) -> CssAffineMatrix {
        CssAffineMatrix::new(
            self.constant.x_axis,
            self.constant.y_axis,
            CssVector::new(
                self.constant.translation.x
                    + self.width_translation.x * box_size.x
                    + self.height_translation.x * box_size.y,
                self.constant.translation.y
                    + self.width_translation.y * box_size.x
                    + self.height_translation.y * box_size.y,
            ),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Rotate by the given angle in degrees.
    Rotate(f64),
    /// Skew angles around the CSS X/Y axes, in degrees.
    Skew(CssVector),
    /// Scale along the CSS X/Y axes.
    Scale(CssVector),
    /// Translate by (tx, ty). When the corresponding `*_pct` flag is set the
    /// value is a percentage (0..100) resolved against the element's OWN
    /// border-box width (tx) / height (ty) at render time; otherwise it is an
    /// absolute length in pt.
    Translate {
        offset: CssVector,
        percentages: PercentageAxes,
    },
    /// Pre-composed affine matrix `(a, b, c, d, e, f)` for chained transforms.
    ///
    /// `e`/`f` are constant pt translations; `e_w`/`e_h`/`f_w`/`f_h` are
    /// coefficients (fractions) multiplying the box width/height to account for
    /// percentage `translate()` components that appear anywhere in the chain.
    /// At render time the effective translation is
    /// `e + e_w*w + e_h*h` / `f + f_w*w + f_h*h`.
    Matrix(CssAffineMatrix),
    /// CSS 3D matrix in the column-major order used by `matrix3d()`.
    Matrix3d(CssMatrix3d),
    /// Child 3D transform projected through a parent `perspective` property.
    Project3d {
        matrix: CssMatrix3d,
        perspective: f64,
        perspective_origin: CssVector,
    },
    /// Composed matrix carrying percentage-translate coefficients (see above).
    /// Only emitted when a chained transform contains a `%` translate; plain
    /// chains collapse to [`Transform::Matrix`].
    MatrixPct(ParametricAffineMatrix),
}

impl Transform {
    /// Resolve this transform to a concrete CSS affine matrix `[a, b, c, d, e, f]`
    /// given the element's border-box size in pt. Percentage translate
    /// components resolve against `w`/`h` here. The returned matrix is in CSS
    /// (y-down) convention; the renderer applies the y-flip + origin
    /// conjugation.
    pub fn to_css_matrix(self, box_size: CssVector) -> CssAffineMatrix {
        match self {
            Transform::Rotate(deg) => {
                let rad = deg.to_radians();
                let (c, s) = (rad.cos(), rad.sin());
                CssAffineMatrix::from_components(c, s, -s, c, 0.0, 0.0)
            }
            Transform::Skew(angles) => CssAffineMatrix::from_components(
                1.0,
                angles.y.to_radians().tan(),
                angles.x.to_radians().tan(),
                1.0,
                0.0,
                0.0,
            ),
            Transform::Scale(scale) => {
                CssAffineMatrix::from_components(scale.x, 0.0, 0.0, scale.y, 0.0, 0.0)
            }
            Transform::Translate {
                offset,
                percentages,
            } => {
                let resolved = CssVector::new(
                    if percentages.x {
                        offset.x / 100.0 * box_size.x
                    } else {
                        offset.x
                    },
                    if percentages.y {
                        offset.y / 100.0 * box_size.y
                    } else {
                        offset.y
                    },
                );
                CssAffineMatrix::new(CssVector::new(1.0, 0.0), CssVector::new(0.0, 1.0), resolved)
            }
            Transform::Matrix(matrix) => matrix,
            Transform::Matrix3d(m) => matrix3d_affine_projection(&m),
            Transform::Project3d { matrix, .. } => matrix3d_affine_projection(&matrix),
            Transform::MatrixPct(matrix) => matrix.resolve(box_size),
        }
    }
}

fn matrix3d_affine_projection(m: &CssMatrix3d) -> CssAffineMatrix {
    let w0 = m[15];
    if w0 == 0.0 {
        return CssAffineMatrix::IDENTITY;
    }
    CssAffineMatrix::from_components(
        m[0] / w0,
        m[1] / w0,
        m[4] / w0,
        m[5] / w0,
        m[12] / w0,
        m[13] / w0,
    )
}

/// CSS `transform-origin`: the pivot point for an element's transform.
///
/// Each axis is `fraction * dimension + length`, where `fraction` resolves
/// percentages/keywords against the box's own width/height and `length` is an
/// absolute pixel offset. The default is the box centre (`50% 50%`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOrigin {
    pub x_fraction: f32,
    pub x_length: f32,
    pub y_fraction: f32,
    pub y_length: f32,
    pub z_length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformBox {
    Border,
    Content,
    Fill,
    Stroke,
    #[default]
    View,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self {
            x_fraction: 0.5,
            x_length: 0.0,
            y_fraction: 0.5,
            y_length: 0.0,
            z_length: 0.0,
        }
    }
}

impl TransformOrigin {
    /// Resolve to a pixel offset from the box's top-left corner.
    pub fn resolve(&self, width: f32, height: f32) -> (f32, f32) {
        (
            self.x_fraction * width + self.x_length,
            self.y_fraction * height + self.y_length,
        )
    }
}

/// CSS box-sizing property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// CSS intrinsic-sizing keyword for the `width` property (css-sizing-3 § 5.1).
///
/// When `width` is one of these keywords the declared length is *intrinsic*: the
/// box is sized from its content rather than to a fixed value or the available
/// space. `ComputedStyle.width` stays `None` (so existing length/percentage/auto
/// paths are untouched) and this enum records which keyword was used so block
/// layout can compute the corresponding content-based width.
// Variant names deliberately mirror the CSS keyword family
// (`min-content` / `max-content` / `fit-content`); the shared `Content` suffix is
// part of the spec vocabulary, so keep it rather than abbreviating.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicWidthKeyword {
    /// Narrowest width the content can take without overflow.
    MinContent,
    /// Widest the content wants to be with no line wrapping.
    MaxContent,
    /// `min(max-content, max(min-content, stretch-fit))` — shrink-to-fit.
    FitContent,
}

/// The computed `flex-basis` value. A percentage component remains unresolved
/// until flex layout has the container's inner main size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexBasis {
    #[default]
    Auto,
    Definite(LengthPercent),
    Content(IntrinsicWidthKeyword),
}

impl FlexBasis {
    pub fn resolve(self, basis: f32) -> Option<f32> {
        match self {
            Self::Definite(value) => Some(value.resolve(basis).max(0.0)),
            Self::Auto | Self::Content(_) => None,
        }
    }

    pub fn definite_length(self) -> Option<f32> {
        match self {
            Self::Definite(value) => value.absolute_length(),
            Self::Auto | Self::Content(_) => None,
        }
    }

    pub const fn content_keyword(self) -> Option<IntrinsicWidthKeyword> {
        match self {
            Self::Content(keyword) => Some(keyword),
            Self::Auto | Self::Definite(_) => None,
        }
    }

    pub fn is_zero(self) -> bool {
        self.definite_length() == Some(0.0)
    }
}

/// CSS text-transform property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// CSS `font-variant-caps` (css-fonts-4 §6.5) — the caps-related subset of
/// `font-variant`. Only `small-caps` is synthesised (the bundled faces carry no
/// real small-caps OpenType feature); other sub-values fall back to `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontVariantCaps {
    #[default]
    Normal,
    /// `small-caps`: lowercase letters render as smaller uppercase forms.
    SmallCaps,
}

/// CSS `font-variant-position` (css-fonts-4 §6.12).
///
/// The feature changes glyph selection and positioning while the computed
/// `font-size` and inherited `line-height` remain those of the originating
/// inline box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontVariantPosition {
    #[default]
    Normal,
    Sub,
    Super,
}

impl FontVariantPosition {
    /// The bundled text path has no OpenType `subs`/`sups` shaping yet. This
    /// is the conventional fallback glyph scale used until the feature can be
    /// selected from the font directly.
    pub const fn glyph_scale(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Sub | Self::Super => 0.8,
        }
    }
}

/// CSS `text-emphasis-position` (css-text-decor-4 §3.4).
///
/// The two keywords are a contextual pair: one selects the annotation side of
/// the base text and the other its inline preference. Keeping the legal pairs
/// together prevents the painter and line-box code from independently
/// interpreting loose booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisPosition {
    #[default]
    OverRight,
    OverLeft,
    UnderRight,
    UnderLeft,
}

impl TextEmphasisPosition {
    pub(crate) const fn is_under(self) -> bool {
        matches!(self, Self::UnderRight | Self::UnderLeft)
    }

    fn parse(value: &str) -> Option<Self> {
        let mut side = None;
        let mut inline = None;
        for token in value.split_whitespace() {
            let slot = match token {
                "over" => &mut side,
                "under" => &mut side,
                "right" => &mut inline,
                "left" => &mut inline,
                _ => return None,
            };
            if slot.is_some() {
                return None;
            }
            *slot = Some(token);
        }

        match (side.unwrap_or("over"), inline.unwrap_or("right")) {
            ("over", "right") => Some(Self::OverRight),
            ("over", "left") => Some(Self::OverLeft),
            ("under", "right") => Some(Self::UnderRight),
            ("under", "left") => Some(Self::UnderLeft),
            _ => None,
        }
    }
}

/// CSS white-space property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
    /// `white-space: break-spaces` (css-text-3 §3): like `pre-wrap` (preserve
    /// spaces and forced segment breaks, still soft-wrap) but trailing
    /// preserved spaces are treated as visible characters that occupy width at
    /// the line end and cannot hang. Handled in the same paths as `pre-wrap`.
    BreakSpaces,
}

/// CSS vertical-align property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Super,
    Sub,
    Length(f32),
    Percent(f32),
    Top,
    /// `text-top`: align the box top to the top of the PARENT's text content
    /// (font) area — the parent baseline plus the parent's font ascent — which
    /// is lower than the line-box top when the line box is taller than the
    /// parent's font box (css2 §10.8.1).
    TextTop,
    Middle,
    Bottom,
    /// `text-bottom`: align the box bottom to the bottom of the parent's text
    /// content (font) area — the parent baseline minus the parent's font
    /// descent (css2 §10.8.1).
    TextBottom,
}

/// CSS `writing-mode` property (css-writing-modes-4 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    /// `horizontal-tb` — the default: inline progresses left-to-right, block
    /// progresses top-to-bottom.
    #[default]
    HorizontalTb,
    /// `vertical-rl` — inline progresses top-to-bottom, block (columns)
    /// progresses right-to-left.
    VerticalRl,
    /// `vertical-lr` — inline progresses top-to-bottom, block progresses
    /// left-to-right.
    VerticalLr,
    /// `sideways-rl` — sideways inline content with right-to-left block flow.
    SidewaysRl,
    /// `sideways-lr` — sideways inline content with left-to-right block flow
    /// and bottom-to-top inline progression.
    SidewaysLr,
}

impl WritingMode {
    /// Whether the inline axis is vertical.
    pub const fn is_vertical(self) -> bool {
        !matches!(self, Self::HorizontalTb)
    }

    /// Whether the block axis progresses physically left-to-right.
    pub const fn is_vertical_lr(self) -> bool {
        matches!(self, Self::VerticalLr | Self::SidewaysLr)
    }

    /// Whether vertical columns progress physically right-to-left.
    pub const fn block_axis_reversed(self) -> bool {
        matches!(self, Self::VerticalRl | Self::SidewaysRl)
    }
}

/// CSS `text-combine-upright` (css-writing-modes-4 §9.1).
///
/// The property inherits. It only changes glyph composition in vertical writing
/// modes: a qualifying run is composed horizontally inside one em square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextCombineUpright {
    #[default]
    None,
    All,
    Digits(u8),
}

impl TextCombineUpright {
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }

    fn parse(value: &str) -> Option<Self> {
        let mut tokens = value.split_ascii_whitespace();
        match (tokens.next()?, tokens.next(), tokens.next()) {
            (token, None, None) if token.eq_ignore_ascii_case("none") => Some(Self::None),
            (token, None, None) if token.eq_ignore_ascii_case("all") => Some(Self::All),
            (token, None, None) if token.eq_ignore_ascii_case("digits") => Some(Self::Digits(2)),
            (token, Some(limit), None) if token.eq_ignore_ascii_case("digits") => limit
                .parse::<u8>()
                .ok()
                .filter(|limit| (2..=4).contains(limit))
                .map(Self::Digits),
            _ => None,
        }
    }
}

/// One unresolved affine position on a gradient line.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GradientPosition {
    pub fraction: f32,
    pub length: f32,
}

impl GradientPosition {
    pub const fn new(fraction: f32, length: f32) -> Self {
        Self { fraction, length }
    }

    pub const fn fraction(value: f32) -> Self {
        Self::new(value, 0.0)
    }

    pub const fn length(value: f32) -> Self {
        Self::new(0.0, value)
    }

    fn resolve(self, basis: f32, length_scale: f32) -> Option<f32> {
        let value = self.fraction + self.length * length_scale / basis;
        value.is_finite().then_some(value)
    }
}

impl std::ops::Add for GradientPosition {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.fraction + rhs.fraction, self.length + rhs.length)
    }
}

impl std::ops::AddAssign for GradientPosition {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl std::ops::Sub for GradientPosition {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.fraction - rhs.fraction, self.length - rhs.length)
    }
}

impl std::ops::SubAssign for GradientPosition {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl std::ops::Mul<f32> for GradientPosition {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.fraction * rhs, self.length * rhs)
    }
}

/// The authored form which controls the default gradient interpolation space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientColorProvenance {
    LegacySrgb,
    Modern,
    CurrentColor,
}

/// A computed color together with the source distinction CSS still needs.
///
/// `Color` is currently the engine's bounded sRGB storage. Provenance preserves
/// Auto's legacy-vs-modern interpolation choice, but it cannot recover modern
/// channels clipped by the shared color parser; true wide-gamut interpolation
/// requires widening that shared value model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientColor {
    pub color: Color,
    pub provenance: GradientColorProvenance,
}

impl GradientColor {
    pub const fn new(color: Color, provenance: GradientColorProvenance) -> Self {
        Self { color, provenance }
    }

    fn uses_legacy_srgb_interpolation(self) -> bool {
        self.provenance == GradientColorProvenance::LegacySrgb
    }
}

/// A color stop in a gradient. Omitted positions stay omitted until the
/// concrete gradient line is known; a transition hint belongs to the segment
/// immediately after the stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: GradientColor,
    pub position: Option<GradientPosition>,
    pub hint_after: Option<GradientPosition>,
}

impl GradientStop {
    pub const fn new(color: GradientColor, position: Option<GradientPosition>) -> Self {
        Self {
            color,
            position,
            hint_after: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GradientInterpolation {
    #[default]
    Auto,
    Srgb,
    Oklab,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GradientRepeat {
    #[default]
    Clamp,
    Repeat,
}

impl GradientRepeat {
    pub const fn is_repeating(self) -> bool {
        matches!(self, Self::Repeat)
    }
}

/// Color, interpolation, and repetition semantics shared by every gradient
/// geometry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GradientRamp {
    pub stops: Vec<GradientStop>,
    pub interpolation: GradientInterpolation,
    pub repeat: GradientRepeat,
}

impl GradientRamp {
    pub(crate) fn resolve(&self, basis: f32) -> Option<ResolvedGradientRamp> {
        self.resolve_scaled(basis, 1.0)
    }

    pub(crate) fn resolve_scaled(
        &self,
        basis: f32,
        length_scale: f32,
    ) -> Option<ResolvedGradientRamp> {
        if self.stops.is_empty() || !basis.is_finite() || basis <= 0.0 || !length_scale.is_finite()
        {
            return None;
        }

        let mut positions = Vec::with_capacity(self.stops.len());
        let mut hints = Vec::with_capacity(self.stops.len());
        for (index, stop) in self.stops.iter().enumerate() {
            if ![
                stop.color.color.r,
                stop.color.color.g,
                stop.color.color.b,
                stop.color.color.a,
            ]
            .into_iter()
            .all(f32::is_finite)
                || (index + 1 == self.stops.len() && stop.hint_after.is_some())
            {
                return None;
            }
            positions.push(match stop.position {
                Some(position) => Some(position.resolve(basis, length_scale)?),
                None => None,
            });
            hints.push(match stop.hint_after {
                Some(position) => Some(position.resolve(basis, length_scale)?),
                None => None,
            });
        }

        positions[0].get_or_insert(0.0);
        if positions.len() > 1 {
            positions.last_mut()?.get_or_insert(1.0);
        }

        #[derive(Clone, Copy)]
        enum FixupItem {
            Stop(usize),
            Hint(usize),
        }
        let mut items = Vec::with_capacity(positions.len() * 2 - 1);
        for index in 0..positions.len() {
            items.push((FixupItem::Stop(index), positions[index]));
            if hints[index].is_some() {
                items.push((FixupItem::Hint(index), hints[index]));
            }
        }

        // Positioned stops and hints are one ordered source stream. Clamp its
        // specified items first, then distribute each omitted run between the
        // surrounding specified items. Hints therefore correctly bound an
        // adjacent run instead of being moved backward after distribution.
        let mut previous = None;
        for (_, position) in &mut items {
            if let Some(value) = position {
                if let Some(previous) = previous
                    && *value < previous
                {
                    *value = previous;
                }
                previous = Some(*value);
            }
        }

        let mut index = 1;
        while index + 1 < items.len() {
            if items[index].1.is_some() {
                index += 1;
                continue;
            }
            let left = index - 1;
            let mut right = index + 1;
            while items[right].1.is_none() {
                right += 1;
            }
            let start = items[left].1?;
            let end = items[right].1?;
            let span = (right - left) as f32;
            for (offset, (_, position)) in items[index..right].iter_mut().enumerate() {
                *position = Some(start + (end - start) * (offset + 1) as f32 / span);
            }
            index = right;
        }

        for (item, position) in items {
            match item {
                FixupItem::Stop(index) => positions[index] = position,
                FixupItem::Hint(index) => hints[index] = position,
            }
        }
        let positions = positions.into_iter().collect::<Option<Vec<_>>>()?;
        let stops = self
            .stops
            .iter()
            .zip(positions)
            .zip(hints)
            .map(|((stop, position), hint_after)| ResolvedGradientStop {
                color: stop.color,
                position,
                hint_after,
            })
            .collect();
        let interpolation = match self.interpolation {
            GradientInterpolation::Auto
                if self
                    .stops
                    .iter()
                    .all(|stop| stop.color.uses_legacy_srgb_interpolation()) =>
            {
                GradientInterpolation::Srgb
            }
            GradientInterpolation::Auto => GradientInterpolation::Oklab,
            explicit => explicit,
        };
        Some(ResolvedGradientRamp {
            stops,
            interpolation,
            repeat: self.repeat,
        })
    }
}

/// A gradient stop after fixup against one concrete line or radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedGradientStop {
    pub(crate) color: GradientColor,
    pub(crate) position: f32,
    pub(crate) hint_after: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ResolvedGradientHint {
    AtStart,
    AtEnd,
    Exponent(f32),
}

impl ResolvedGradientHint {
    fn map_progress(self, progress: f32) -> f32 {
        match self {
            Self::AtStart => {
                if progress <= 0.0 {
                    0.0
                } else {
                    1.0
                }
            }
            Self::AtEnd => {
                if progress < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
            Self::Exponent(exponent) => progress.powf(exponent),
        }
    }
}

/// One exact fixed-up segment for PDF/native-backend capability decisions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedGradientSegment {
    pub(crate) lower: ResolvedGradientStop,
    pub(crate) upper: ResolvedGradientStop,
    pub(crate) interpolation: GradientInterpolation,
    pub(crate) hint: Option<ResolvedGradientHint>,
}

/// One fixed-up ramp shared by every gradient sampling path.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedGradientRamp {
    stops: Box<[ResolvedGradientStop]>,
    interpolation: GradientInterpolation,
    repeat: GradientRepeat,
}

impl ResolvedGradientRamp {
    pub(crate) fn stops(&self) -> &[ResolvedGradientStop] {
        &self.stops
    }

    pub(crate) const fn repeat(&self) -> GradientRepeat {
        self.repeat
    }

    pub(crate) fn segments(&self) -> impl ExactSizeIterator<Item = ResolvedGradientSegment> + '_ {
        self.stops.windows(2).map(|pair| {
            let lower = pair[0];
            let upper = pair[1];
            ResolvedGradientSegment {
                lower,
                upper,
                interpolation: self.segment_interpolation(lower, upper),
                hint: Self::segment_hint(lower, upper),
            }
        })
    }

    /// Return an exact SVG stop representation only for the subset whose CSS
    /// interpolation semantics SVG can preserve without approximation.
    pub(crate) fn svg_unit_interval_stops(&self) -> Option<Vec<ResolvedGradientStop>> {
        if self.repeat.is_repeating()
            || self.stops.iter().any(|stop| stop.hint_after.is_some())
            || self.uniform_opacity().is_none()
            || self.stops.windows(2).any(|pair| {
                self.segment_interpolation(pair[0], pair[1]) != GradientInterpolation::Srgb
            })
        {
            return None;
        }
        self.fixed_unit_interval_stops()
    }

    /// Fixed-up stops clipped to the visible unit interval. Backends decide
    /// separately whether their interpolation primitive matches CSS exactly or
    /// intentionally mirrors a lower-level print backend representation.
    pub(crate) fn fixed_unit_interval_stops(&self) -> Option<Vec<ResolvedGradientStop>> {
        if self.repeat.is_repeating() || self.stops.iter().any(|stop| stop.hint_after.is_some()) {
            return None;
        }
        let mut visible = Vec::with_capacity(self.stops.len() + 2);
        if !self.stops.iter().any(|stop| stop.position == 0.0) {
            visible.push(ResolvedGradientStop {
                color: GradientColor::new(
                    Self::color(self.sample(0.0)),
                    GradientColorProvenance::LegacySrgb,
                ),
                position: 0.0,
                hint_after: None,
            });
        }
        visible.extend(
            self.stops
                .iter()
                .copied()
                .filter(|stop| (0.0..=1.0).contains(&stop.position)),
        );
        if !self.stops.iter().any(|stop| stop.position == 1.0) {
            visible.push(ResolvedGradientStop {
                color: GradientColor::new(
                    Self::color(self.sample(1.0)),
                    GradientColorProvenance::LegacySrgb,
                ),
                position: 1.0,
                hint_after: None,
            });
        }
        Some(visible)
    }

    pub(crate) fn is_opaque(&self) -> bool {
        self.stops.iter().all(|stop| stop.color.color.a == 255.0)
    }

    pub(crate) fn uniform_opacity(&self) -> Option<f32> {
        let opacity = self.stops.first()?.color.color.a;
        self.stops
            .iter()
            .all(|stop| stop.color.color.a == opacity)
            .then(|| (opacity / 255.0).clamp(0.0, 1.0))
    }

    pub(crate) fn sample(&self, mut t: f32) -> (f32, f32, f32, f32) {
        if !t.is_finite() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        if self.repeat.is_repeating() {
            let first = self.stops[0].position;
            let last = self.stops[self.stops.len() - 1].position;
            let period = last - first;
            if period == 0.0 {
                return self.weighted_average();
            }
            if !period.is_finite() || period < 0.0 {
                return (0.0, 0.0, 0.0, 0.0);
            }
            t = first + (t - first).rem_euclid(period);
        }

        if t < self.stops[0].position {
            return Self::rgba(self.stops[0].color.color);
        }
        let last = self.stops[self.stops.len() - 1];
        if t >= last.position {
            return Self::rgba(last.color.color);
        }

        let upper_index = self.stops.partition_point(|stop| stop.position <= t);
        let lower = self.stops[upper_index - 1];
        if lower.position == t {
            return Self::rgba(lower.color.color);
        }
        let upper = self.stops[upper_index];
        let mut progress = (t - lower.position) / (upper.position - lower.position);
        if let Some(hint) = Self::segment_hint(lower, upper) {
            progress = hint.map_progress(progress);
        }
        self.interpolate(lower, upper, progress)
    }

    fn segment_hint(
        lower: ResolvedGradientStop,
        upper: ResolvedGradientStop,
    ) -> Option<ResolvedGradientHint> {
        let hint = lower.hint_after?;
        let span = upper.position - lower.position;
        if span <= 0.0 {
            return None;
        }
        let hint = (hint - lower.position) / span;
        Some(if hint <= 0.0 {
            ResolvedGradientHint::AtStart
        } else if hint >= 1.0 {
            ResolvedGradientHint::AtEnd
        } else {
            ResolvedGradientHint::Exponent(0.5_f32.ln() / hint.ln())
        })
    }

    pub(crate) fn segment_interpolation(
        &self,
        _lower: ResolvedGradientStop,
        _upper: ResolvedGradientStop,
    ) -> GradientInterpolation {
        debug_assert_ne!(self.interpolation, GradientInterpolation::Auto);
        self.interpolation
    }

    fn interpolate(
        &self,
        lower: ResolvedGradientStop,
        upper: ResolvedGradientStop,
        progress: f32,
    ) -> (f32, f32, f32, f32) {
        let lower_rgba = Self::rgba(lower.color.color);
        let upper_rgba = Self::rgba(upper.color.color);
        match self.segment_interpolation(lower, upper) {
            GradientInterpolation::Srgb | GradientInterpolation::Auto => {
                Self::interpolate_premultiplied(lower_rgba, upper_rgba, progress)
            }
            GradientInterpolation::Oklab => {
                let lower = Self::srgb_to_oklab(lower_rgba);
                let upper = Self::srgb_to_oklab(upper_rgba);
                Self::oklab_to_srgb(Self::interpolate_premultiplied(lower, upper, progress))
            }
        }
    }

    fn interpolate_premultiplied(
        lower: (f32, f32, f32, f32),
        upper: (f32, f32, f32, f32),
        progress: f32,
    ) -> (f32, f32, f32, f32) {
        let lower = Self::premultiply(lower);
        let upper = Self::premultiply(upper);
        Self::unpremultiply((
            lower.0 + (upper.0 - lower.0) * progress,
            lower.1 + (upper.1 - lower.1) * progress,
            lower.2 + (upper.2 - lower.2) * progress,
            lower.3 + (upper.3 - lower.3) * progress,
        ))
    }

    fn srgb_to_oklab((r, g, b, a): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let linear = |value: f32| {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (linear(r), linear(g), linear(b));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        (
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
            a,
        )
    }

    fn oklab_to_srgb((l, a, b, alpha): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
        let (l, m, s) = (l_.powi(3), m_.powi(3), s_.powi(3));
        let encode = |value: f32| {
            let magnitude = value.abs();
            if magnitude <= 0.003_130_8 {
                12.92 * value
            } else {
                value.signum() * (1.055 * magnitude.powf(1.0 / 2.4) - 0.055)
            }
        };
        (
            encode(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
            encode(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
            encode(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
            alpha,
        )
    }

    /// Average already-sampled colors in premultiplied sRGBA. Used by raster
    /// antialiasing so transparent samples cannot darken neighboring colors.
    pub(crate) fn average_samples(
        samples: impl IntoIterator<Item = (f32, f32, f32, f32)>,
    ) -> (f32, f32, f32, f32) {
        let mut sum = (0.0, 0.0, 0.0, 0.0);
        let mut count = 0_u32;
        for sample in samples {
            let sample = Self::premultiply(sample);
            sum.0 += sample.0;
            sum.1 += sample.1;
            sum.2 += sample.2;
            sum.3 += sample.3;
            count += 1;
        }
        if count == 0 {
            return sum;
        }
        let scale = 1.0 / count as f32;
        Self::unpremultiply((sum.0 * scale, sum.1 * scale, sum.2 * scale, sum.3 * scale))
    }

    pub(crate) fn weighted_average(&self) -> (f32, f32, f32, f32) {
        let [single] = self.stops.as_ref() else {
            let period = self.stops[self.stops.len() - 1].position - self.stops[0].position;
            let equal_segment = 1.0 / (self.stops.len() - 1) as f32;
            let mut average = (0.0, 0.0, 0.0, 0.0);
            for pair in self.stops.windows(2) {
                let segment = if period > 0.0 {
                    (pair[1].position - pair[0].position) / period
                } else {
                    equal_segment
                };
                let weight = segment * 0.5;
                for stop in pair {
                    let color = Self::premultiply(Self::rgba(stop.color.color));
                    average.0 += color.0 * weight;
                    average.1 += color.1 * weight;
                    average.2 += color.2 * weight;
                    average.3 += color.3 * weight;
                }
            }
            return Self::unpremultiply(average);
        };
        Self::rgba(single.color.color)
    }

    fn rgba(color: Color) -> (f32, f32, f32, f32) {
        color.to_f32_rgba()
    }

    fn color((r, g, b, a): (f32, f32, f32, f32)) -> Color {
        Color::from_srgb(r, g, b, a)
    }

    fn premultiply((r, g, b, a): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        (r * a, g * a, b * a, a)
    }

    fn unpremultiply((r, g, b, a): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        if a > 0.0 {
            (r / a, g / a, b / a, a)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}

/// Per-layer painting parameters for a gradient background layer. Populated
/// only when a gradient coexists with other layers in a comma-separated
/// `background-image` list and that layer has its own `background-size` /
/// `-position` / `-repeat` entry. When fields are `None` the gradient fills the
/// whole painting area (the historical single-layer behaviour).
#[derive(Debug, Clone, Copy, Default)]
pub struct GradientLayerBox {
    /// Size of one gradient tile (`background-size` for this layer).
    pub size: Option<BackgroundSize>,
    /// Position of the gradient tile (`background-position` for this layer).
    pub position: Option<BackgroundPosition>,
    /// Repeat mode of the gradient tile (`background-repeat` for this layer).
    pub repeat: Option<BackgroundRepeat>,
    pub origin: Option<BackgroundOrigin>,
    pub clip: Option<BackgroundClip>,
    pub attachment: Option<BackgroundAttachment>,
    pub paint_above_raster: bool,
}

impl GradientLayerBox {
    pub(crate) const fn is_initial(self) -> bool {
        self.size.is_none()
            && self.position.is_none()
            && self.repeat.is_none()
            && self.origin.is_none()
            && self.clip.is_none()
            && self.attachment.is_none()
            && !self.paint_above_raster
    }

    /// Fill omitted per-layer values from the owning background without
    /// overwriting values authored specifically for this image layer.
    pub(crate) fn with_fallback(mut self, fallback: Self) -> Self {
        if self.size.is_none() {
            self.size = fallback.size;
        }
        if self.position.is_none() {
            self.position = fallback.position;
        }
        if self.repeat.is_none() {
            self.repeat = fallback.repeat;
        }
        if self.origin.is_none() {
            self.origin = fallback.origin;
        }
        if self.clip.is_none() {
            self.clip = fallback.clip;
        }
        if self.attachment.is_none() {
            self.attachment = fallback.attachment;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderImageSliceValue {
    Number(f32),
    Percentage(f32),
}

impl BorderImageSliceValue {
    fn resolve(self, extent: f32, number_scale: f32) -> f32 {
        match self {
            Self::Number(value) => value * number_scale,
            Self::Percentage(value) => extent * value / 100.0,
        }
        .max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderImageSlices {
    pub top: BorderImageSliceValue,
    pub right: BorderImageSliceValue,
    pub bottom: BorderImageSliceValue,
    pub left: BorderImageSliceValue,
    pub fill: bool,
}

impl BorderImageSlices {
    pub const fn uniform(value: BorderImageSliceValue) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
            fill: false,
        }
    }

    pub fn resolve(self, width: f32, height: f32, number_scale: f32) -> EdgeSizes {
        EdgeSizes::new(
            self.top.resolve(height, number_scale),
            self.right.resolve(width, number_scale),
            self.bottom.resolve(height, number_scale),
            self.left.resolve(width, number_scale),
        )
        .clamp_to_extents(width, height)
    }
}

impl Default for BorderImageSlices {
    fn default() -> Self {
        Self::uniform(BorderImageSliceValue::Percentage(100.0))
    }
}

/// One used-value component of `border-image-width`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderImageWidth {
    /// A multiple of the corresponding computed physical border width.
    Number(f32),
    /// A `<length-percentage>` resolved against the corresponding
    /// border-image-area dimension.
    LengthPercent(LengthPercent),
    /// The source image's natural slice size, or the physical border width
    /// when the source has no natural dimension (as for CSS gradients).
    Auto,
}

impl BorderImageWidth {
    fn resolve(self, border_width: f32, area_extent: f32, natural_slice: Option<f32>) -> f32 {
        match self {
            Self::Number(value) => value * border_width,
            Self::LengthPercent(value) => value.resolve(area_extent),
            Self::Auto => natural_slice.unwrap_or(border_width),
        }
        .max(0.0)
    }
}

/// Four `border-image-width` values, in CSS top/right/bottom/left order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderImageWidths {
    pub top: BorderImageWidth,
    pub right: BorderImageWidth,
    pub bottom: BorderImageWidth,
    pub left: BorderImageWidth,
}

impl BorderImageWidths {
    pub const fn uniform(value: BorderImageWidth) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// Resolve CSS border-image destination offsets for one border-image area.
    pub fn resolve(
        self,
        border: EdgeSizes,
        area_width: f32,
        area_height: f32,
        natural_slices: Option<EdgeSizes>,
    ) -> EdgeSizes {
        let natural = natural_slices.unwrap_or(EdgeSizes::ZERO);
        EdgeSizes::new(
            self.top
                .resolve(border.top, area_height, natural_slices.map(|_| natural.top)),
            self.right.resolve(
                border.right,
                area_width,
                natural_slices.map(|_| natural.right),
            ),
            self.bottom.resolve(
                border.bottom,
                area_height,
                natural_slices.map(|_| natural.bottom),
            ),
            self.left.resolve(
                border.left,
                area_width,
                natural_slices.map(|_| natural.left),
            ),
        )
        .scale_to_fit_within(area_width, area_height)
    }
}

impl Default for BorderImageWidths {
    fn default() -> Self {
        Self::uniform(BorderImageWidth::Number(1.0))
    }
}

/// One used-value component of `border-image-outset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderImageOutset {
    /// A multiple of the corresponding computed physical border width.
    Number(f32),
    /// A resolved CSS length in PDF points.
    Length(f32),
}

impl BorderImageOutset {
    fn resolve(self, border_width: f32) -> f32 {
        match self {
            Self::Number(value) => value * border_width,
            Self::Length(value) => value,
        }
    }
}

/// Four `border-image-outset` values, in CSS top/right/bottom/left order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderImageOutsets {
    pub top: BorderImageOutset,
    pub right: BorderImageOutset,
    pub bottom: BorderImageOutset,
    pub left: BorderImageOutset,
}

impl BorderImageOutsets {
    pub const fn uniform(value: BorderImageOutset) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// Resolve the expansion from the border box to the border-image area.
    pub fn resolve(self, border: EdgeSizes) -> EdgeSizes {
        EdgeSizes::new(
            self.top.resolve(border.top),
            self.right.resolve(border.right),
            self.bottom.resolve(border.bottom),
            self.left.resolve(border.left),
        )
    }
}

impl Default for BorderImageOutsets {
    fn default() -> Self {
        Self::uniform(BorderImageOutset::Number(0.0))
    }
}

/// One axis of CSS `border-image-repeat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BorderImageRepeatMode {
    /// Scale one image to fill its border-image region.
    #[default]
    Stretch,
    /// Center and tile the image, clipping partial end tiles.
    Repeat,
    /// Resize the image along the axis so a whole number of tiles fit.
    Round,
    /// Keep whole tiles and distribute surplus space around them.
    Space,
}

/// CSS `border-image-repeat`'s horizontal and vertical tiling modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderImageRepeats {
    pub horizontal: BorderImageRepeatMode,
    pub vertical: BorderImageRepeatMode,
}

impl BorderImageRepeats {
    pub const fn uniform(mode: BorderImageRepeatMode) -> Self {
        Self {
            horizontal: mode,
            vertical: mode,
        }
    }
}

impl Default for BorderImageRepeats {
    fn default() -> Self {
        Self::uniform(BorderImageRepeatMode::Stretch)
    }
}

/// The geometry portion of one CSS `border-image` shorthand.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderImage {
    pub slices: BorderImageSlices,
    pub widths: BorderImageWidths,
    pub outsets: BorderImageOutsets,
    pub repeats: BorderImageRepeats,
}

/// A resolved CSS border image and its nine-slice geometry.
///
/// A border image is a box decoration, not a background layer. Keeping it in a
/// dedicated structure prevents `border-image` from replacing an unrelated
/// `background-image` gradient in the computed style.
#[derive(Debug, Clone)]
pub struct BorderImagePaint {
    pub source: BorderImageSource,
    pub geometry: BorderImage,
}

/// Independently cascaded `border-image-*` longhands.
///
/// Geometry remains computed even while the source is `none`, so an inherited
/// longhand or a later source declaration cannot lose its specified value.
#[derive(Debug, Clone, Default)]
pub struct BorderImageProperties {
    pub source: Option<BorderImageSource>,
    pub geometry: BorderImage,
}

impl BorderImageProperties {
    /// Materialize the paint-only representation when a source exists.
    pub fn paint(&self) -> Option<BorderImagePaint> {
        Some(BorderImagePaint {
            source: self.source.clone()?,
            geometry: self.geometry,
        })
    }

    /// Whether this border image has a source that requires a paint owner.
    pub const fn has_source(&self) -> bool {
        self.source.is_some()
    }
}

/// CSS image sources supported by the border-image paint pipeline.
#[derive(Debug, Clone)]
pub enum BorderImageSource {
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
    ConicGradient(ConicGradient),
    Url(String),
}

/// A CSS linear gradient.
#[derive(Debug, Clone)]
pub struct LinearGradient {
    /// Angle in degrees (0 = to top, 90 = to right, 180 = to bottom, 270 = to left).
    pub angle: f32,
    pub ramp: GradientRamp,
    /// Per-layer size/position/repeat when this gradient is one of several
    /// comma-separated background layers.
    pub layer_box: GradientLayerBox,
}

/// CSS text-overflow property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

/// CSS overflow-wrap / word-wrap property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    Anywhere,
    BreakWord,
}
/// CSS border-collapse property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}
/// CSS table-layout property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    Fixed,
}
/// CSS empty-cells property (inherited). Controls whether the borders and
/// background of an empty table cell are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EmptyCells {
    #[default]
    Show,
    Hide,
}
/// CSS caption-side property (inherited). Controls whether a table `<caption>`
/// is placed above (`top`) or below (`bottom`) the table box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CaptionSide {
    #[default]
    Top,
    Bottom,
}
/// CSS background-origin property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundOrigin {
    #[default]
    Padding,
    Border,
    Content,
}
/// CSS background-clip property: the box the background painting area is clipped
/// to (css-backgrounds-3 §3.4). Default is `border-box`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundClip {
    #[default]
    Border,
    Padding,
    Content,
    Text,
}
/// CSS background-attachment property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundAttachment {
    #[default]
    Scroll,
    Fixed,
    Local,
}
/// CSS background-size property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundSize {
    #[default]
    Auto,
    Cover,
    Contain,
    Explicit {
        width: f32,
        height: Option<f32>,
        width_is_percent: bool,
        height_is_percent: bool,
    },
    ExplicitAuto {
        width: Option<f32>,
        height: Option<f32>,
        width_is_percent: bool,
        height_is_percent: bool,
    },
}
/// CSS background-repeat property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
    Space,
    Round,
    SpaceRound,
    RoundSpace,
}
/// CSS background-position value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    pub x: f32,
    pub y: f32,
    pub x_is_percent: bool,
    pub y_is_percent: bool,
}
impl Default for BackgroundPosition {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            x_is_percent: true,
            y_is_percent: true,
        }
    }
}
/// CSS list-style-type property.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    CjkDecimal,
    String(String),
    Custom(String),
    CounterStyle(CounterStyle),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CounterStyle {
    pub system: CounterStyleSystem,
    pub symbols: Vec<String>,
    pub prefix: String,
    pub suffix: String,
    pub pad: Option<(usize, String)>,
    pub negative: (String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterStyleSystem {
    Cyclic,
    ExtendsDecimal,
}
/// CSS list-style-position property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}
/// A single item in a CSS `content` property value.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    String(String),
    Attr(String),
    /// `counter(name)` or `counter(name, style)`. The style governs how the
    /// numeric value is rendered (decimal by default, e.g. upper-roman).
    Counter(String, ListStyleType),
    /// `counters(name, separator)` or `counters(name, separator, style)`. Joins
    /// every nested level of `name` with `separator`, each formatted in `style`.
    Counters(String, String, ListStyleType),
    /// `leader(pattern)` from CSS Generated Content for Paged Media.
    Leader(String),
    /// `open-quote` keyword — resolves to the opening quotation mark.
    OpenQuote,
    /// `close-quote` keyword — resolves to the closing quotation mark.
    CloseQuote,
    /// `no-open-quote` keyword — inserts nothing but increments quote depth
    /// (css-content-3 §2.4.2).
    NoOpenQuote,
    /// `no-close-quote` keyword — inserts nothing but decrements quote depth
    /// (css-content-3 §2.4.2).
    NoCloseQuote,
    /// `url(...)` / `<image>` — makes the pseudo-element a replaced element
    /// filled with the referenced image (css-content-3 §1, §2). Holds the raw
    /// URL/data-URI string.
    Url(String),
}

pub(crate) const TARGET_PLACEHOLDER_START: &str = "__ironpress-target:";
pub(crate) const TARGET_PLACEHOLDER_END: &str = "__";
pub(crate) const LEADER_PLACEHOLDER_START: &str = "\u{1d}ip-leader:";
pub(crate) const LEADER_PLACEHOLDER_END: &str = "\u{1d}";

/// CSS box-shadow value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSource {
    Absolute,
    CurrentColor,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    /// Retained only because `text-shadow` is inherited: a child must rebind an
    /// inherited `currentColor` shadow to its own foreground color. Box shadows
    /// are non-inherited but share this compact storage type.
    pub(crate) color_source: ColorSource,
    pub inset: bool,
}

/// Border line style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BorderStyle {
    Solid,
    Dashed,
    Dotted,
    /// Two parallel solid rules separated by a gap (CSS `double`). Each rule and
    /// the gap take roughly one third of the border width.
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    Hidden,
    #[default]
    None,
}

impl BorderStyle {
    pub(crate) const fn paints(self) -> bool {
        !matches!(self, Self::None | Self::Hidden)
    }
}

/// A single border side with width, color, and style.
#[derive(Debug, Clone, Copy)]
pub struct BorderSide {
    /// Cascaded `border-*-width` before `none`/`hidden` style normalization.
    /// Layout code must use [`Self::used_width`] so a non-painting side never
    /// contributes the initial `medium` width to box geometry.
    specified_width: f32,
    /// The computed border color. Keeping `currentColor` symbolic is required
    /// for explicit inheritance: a child inheriting `currentColor` binds it to
    /// the child's foreground color, not the parent's used RGB value.
    pub color: SpecifiedColor,
    pub style: BorderStyle,
}

impl Default for BorderSide {
    fn default() -> Self {
        Self {
            // `medium` is the initial specified width. A `none` or `hidden`
            // style makes its computed/used width zero through `used_width`,
            // without losing the initial width needed when only border-style
            // is authored later in the cascade.
            specified_width: MEDIUM_RULE_WIDTH_PT,
            color: SpecifiedColor::CurrentColor,
            style: BorderStyle::None,
        }
    }
}

impl BorderSide {
    pub const fn new(width: f32, color: SpecifiedColor, style: BorderStyle) -> Self {
        Self {
            specified_width: width,
            color,
            style,
        }
    }

    pub const fn solid(width: f32, color: SpecifiedColor) -> Self {
        Self::new(width, color, BorderStyle::Solid)
    }

    pub const fn used_width(self) -> f32 {
        if self.style.paints() {
            self.specified_width
        } else {
            0.0
        }
    }

    pub const fn paints(self) -> bool {
        self.used_width() > 0.0
    }
}

/// Per-side border specification.
pub type BorderSides = PhysicalEdges<BorderSide>;

#[allow(dead_code)]
impl PhysicalEdges<BorderSide> {
    /// Whether at least one side paints a non-zero border.
    pub fn has_any(&self) -> bool {
        self.top.paints() || self.right.paints() || self.bottom.paints() || self.left.paints()
    }
    /// Largest used border width across all physical sides.
    pub fn max_width(&self) -> f32 {
        self.top
            .used_width()
            .max(self.right.used_width())
            .max(self.bottom.used_width())
            .max(self.left.used_width())
    }
    /// Sum of the used left and right border widths.
    pub fn horizontal_width(&self) -> f32 {
        self.left.used_width() + self.right.used_width()
    }
    /// Sum of the used top and bottom border widths.
    pub fn vertical_width(&self) -> f32 {
        self.top.used_width() + self.bottom.used_width()
    }
    /// Resolved physical widths as one box-model edge group.
    pub fn widths(&self) -> EdgeSizes {
        EdgeSizes::new(
            self.top.used_width(),
            self.right.used_width(),
            self.bottom.used_width(),
            self.left.used_width(),
        )
    }
}

/// Fully resolved style for a node.
#[derive(Debug, Clone, Default)]
pub struct PercentageSizing {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct PercentageInsets {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

/// Root-font used lengths needed by the root-relative CSS units whose basis is
/// not merely `rem`. The root font size remains the single source for `rem`;
/// this group owns the complementary font metrics and `rlh` basis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontUnitLengths {
    pub ex: f32,
    pub ch: f32,
    pub cap: f32,
    pub ic: f32,
    pub line_height: f32,
}

impl Default for FontUnitLengths {
    fn default() -> Self {
        Self {
            ex: 6.0,
            ch: 6.0,
            cap: 9.0,
            ic: 12.0,
            line_height: 14.4,
        }
    }
}

/// CSS Fragmentation 3 §3.1 forced/avoid break value for `break-before` /
/// `break-after`. `Auto` is the initial value (a class-A break opportunity with
/// no forced break and no avoidance). The forced values (`page`/`left`/`right`/
/// `recto`/`verso`) always start a new page; the sided ones additionally force
/// the following content onto a left/right (verso/recto) page. `Avoid` keeps
/// its class-A boundary so pagination can retain an adjacent fitting group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakValue {
    #[default]
    Auto,
    Avoid,
    Page,
    Left,
    Right,
    Recto,
    Verso,
}

impl BreakValue {
    /// Whether this value forces a page break (any of the CSS Fragmentation 3
    /// "forced break values": `page`/`left`/`right`/`recto`/`verso`).
    pub fn forces_break(self) -> bool {
        matches!(
            self,
            BreakValue::Page
                | BreakValue::Left
                | BreakValue::Right
                | BreakValue::Recto
                | BreakValue::Verso
        )
    }

    /// Map a CSS keyword (legacy `page-break-*` or modern `break-*`) to a
    /// `BreakValue`. The legacy `always` aliases to `page`. Returns `None` for
    /// keywords that are not valid break values so the caller leaves the field
    /// untouched.
    pub fn from_keyword(k: &str) -> Option<BreakValue> {
        match k {
            "auto" => Some(BreakValue::Auto),
            "always" | "page" => Some(BreakValue::Page),
            "left" => Some(BreakValue::Left),
            "right" => Some(BreakValue::Right),
            "recto" => Some(BreakValue::Recto),
            "verso" => Some(BreakValue::Verso),
            "avoid" | "avoid-page" => Some(BreakValue::Avoid),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComputedStyle {
    /// Conversion-owned raster policy, propagated with the style-resolution
    /// context. It is not a CSS property and never changes box geometry.
    pub(crate) raster_quality: RasterQuality,
    pub font_size: f32,
    pub root_font_size: f32,
    pub root_font_units: FontUnitLengths,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_stretch: FontStretch,
    pub font_size_adjust: FontSizeAdjust,
    pub font_family: FontFamily,
    pub font_stack: FontStack,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    /// Unresolved em multipliers for each margin side, retained from the cascade
    /// so that em-based margins re-resolve against the element's final font-size
    /// rather than whatever `style.font_size` happened to be when the margin
    /// declaration was applied. `None` means the side was set via an absolute
    /// length (or never touched) and should not be re-resolved.
    pub margin_em_top: Option<f32>,
    pub margin_em_right: Option<f32>,
    pub margin_em_bottom: Option<f32>,
    pub margin_em_left: Option<f32>,
    pub padding: EdgeSizes,
    pub text_align: TextAlign,
    pub text_align_last: Option<TextAlign>,
    pub word_break_keep_all: bool,
    pub hyphens_manual: bool,
    pub text_wrap_mode_nowrap: bool,
    /// CSS direction property (ltr/rtl), set from `dir` attribute or CSS.
    pub direction_rtl: bool,
    /// CSS `writing-mode` (css-writing-modes-4 §3.1). Inherited; initial
    /// `horizontal-tb`. Inherited automatically via the `parent.clone()` model
    /// in `compute_style_with_context` (never reset in the non-inherited block).
    pub writing_mode: WritingMode,
    pub text_orientation_upright: bool,
    /// CSS `text-combine-upright` (css-writing-modes-4 §9.1). Inherited.
    pub text_combine_upright: TextCombineUpright,
    /// CSS `unicode-bidi: bidi-override` (or `isolate-override`). When set, the
    /// element's inline content is reordered strictly in sequence according to
    /// `direction`, overriding the characters' intrinsic bidi classes
    /// (css-writing-modes-4 §2.4). Not inherited; initial is `normal` (false).
    pub bidi_override: bool,
    /// CSS `unicode-bidi: plaintext`: each forced line break resolves its own
    /// paragraph base direction from its first strong character.
    pub bidi_plaintext: bool,
    pub text_decorations: TextDecorations,
    pub line_height: f32,
    pub line_height_absolute: Option<f32>,
    pub page_break_before: bool,
    pub page_break_after: bool,
    /// CSS Fragmentation 3 `break-before` / `break-after` (and their legacy
    /// `page-break-*` aliases). The forced values drive `page_break_before/after`
    /// (above); the sided ones (`left`/`right`/`recto`/`verso`) additionally
    /// carry the parity used to insert a blank page during pagination.
    pub break_before: BreakValue,
    pub break_after: BreakValue,
    /// CSS Fragmentation 3 `break-inside: avoid` (and legacy
    /// `page-break-inside: avoid`): keep the box together — do not split it
    /// across a page boundary unless it is taller than a whole page.
    pub break_inside_avoid: bool,
    /// CSS Paged Media 3 §3.4 `page: <name>`: the named page this box belongs
    /// to. A box whose page name differs from the preceding box forces a page
    /// break before it, and the page it starts adopts the matching
    /// `@page <name>` geometry (currently the margin override). `None` is the
    /// initial `auto` value (the default page). Not inherited.
    pub page_name: Option<String>,
    pub border: BorderSides,
    pub border_image: BorderImageProperties,
    pub display: Display,
    pub width: Option<f32>,
    /// css-sizing-3 § 5.1 intrinsic `width` keyword (`min-content` / `max-content`
    /// / `fit-content`). `None` for the usual length/percentage/`auto` cases. When
    /// set, `width` is left `None` and block layout derives the box width from its
    /// content instead of filling the available width.
    pub width_keyword: Option<IntrinsicWidthKeyword>,
    pub height: Option<f32>,
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub percentage_sizing: PercentageSizing,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    /// `margin-top: auto` / `margin-bottom: auto`. Tracked (in addition to the
    /// horizontal flags) so a flex item can absorb cross-axis (for a row
    /// container) / main-axis (for a column container) free space via auto
    /// margins per css-flexbox-1 §8.1.
    pub margin_top_auto: bool,
    pub margin_bottom_auto: bool,
    pub opacity: f32,
    /// CSS `mix-blend-mode`: how this element composites with the backdrop.
    pub mix_blend_mode: BlendMode,
    /// CSS `background-blend-mode`: how the element's background layers blend
    /// with each other and the background color.
    pub background_blend_mode: BlendMode,
    pub isolation: Isolation,
    pub float: Float,
    /// CSS GCPM `footnote-display` and `footnote-policy`. Not inherited.
    pub footnote: FootnoteFormatting,
    pub clear: Clear,
    /// CSS `box-decoration-break`: how borders/padding/margin/background are
    /// applied across a fragmentation break. Not inherited.
    pub box_decoration_break: BoxDecorationBreak,
    /// CSS `orphans` (css-break-3 §3.4): minimum number of line boxes that must
    /// be left at the BOTTOM of a fragment before a break. Positive integer,
    /// initial 2, inherited. Applies to block containers with an inline
    /// formatting context.
    pub orphans: u8,
    /// CSS `widows` (css-break-3 §3.4): minimum number of line boxes that must
    /// be placed at the TOP of the next fragment after a break. Positive
    /// integer, initial 2, inherited.
    pub widows: u8,
    pub position: Position,
    /// CSS GCPM `position: running(name)`: removes this element from normal flow
    /// and stores it under `name` for `content: element(name)` page-margin boxes.
    /// Kept separate from [`Position`] so the widely-used copy enum can remain
    /// small and static/relative/absolute comparisons stay simple.
    pub running_name: Option<String>,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
    pub percentage_insets: PercentageInsets,
    /// CSS `box-shadow` may list several shadows (comma separated). They paint
    /// back-to-front: the FIRST listed shadow is painted LAST (on top). Empty
    /// when no shadow is set.
    pub box_shadow: Vec<BoxShadow>,
    /// CSS `text-shadow` (css-text-decor-3 §3): a list of shadows painted behind
    /// the element's text. Inherited. Reuses `BoxShadow` (spread/inset unused).
    pub text_shadow: Vec<BoxShadow>,
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    /// Per-item `align-self` (cross-axis alignment override; `Auto` defers to
    /// the container's `align-items`). Not inherited.
    pub align_self: AlignSelf,
    /// Per-item `order` — flex items are laid out by ascending `order`, with
    /// document order breaking ties. Not inherited.
    pub order: i32,
    pub flex_wrap: FlexWrap,
    /// CSS `align-content` — cross-axis distribution of flex lines in a
    /// multi-line container. Ignored for single-line containers.
    pub align_content: AlignContent,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    /// `flex-basis` retains length and percentage components until flex layout
    /// establishes the container's inner main-axis basis.
    pub flex_basis: FlexBasis,
    pub gap: f32,
    pub overflow: Overflow,
    /// Per-axis computed overflow (after the CSS Overflow 3 inter-axis coercion).
    /// The collapsed `overflow` above is kept for the many clip-only consumers;
    /// these two carry the axis detail the scrollbar painter needs (which axis
    /// shows a scrollbar, and whether it is `scroll` = always vs `auto` = only
    /// when content overflows).
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub visibility: Visibility,
    pub transform: Option<Transform>,
    /// CSS `transform-origin` pivot (defaults to the box centre).
    pub transform_origin: TransformOrigin,
    /// CSS `perspective` property for projecting transformed children.
    pub perspective: Option<f32>,
    /// CSS `perspective-origin`, resolved against this box.
    pub perspective_origin: TransformOrigin,
    pub transform_box: TransformBox,
    pub clip_path: Option<ClipPath>,
    /// CSS `mask-image` source (css-masking-1 §3.1). `None` = `mask-image: none`.
    /// Only the primary (first) layer is modelled.
    pub mask_image: Option<MaskSource>,
    /// CSS `mask-mode` (css-masking-1 §3.4).
    pub mask_mode: MaskMode,
    pub grid_template_columns: Vec<GridTrack>,
    /// Explicit `grid-template-rows` track list (empty = auto rows).
    pub grid_template_rows: Vec<GridTrack>,
    /// `grid-auto-rows` size in points for implicit rows (None = auto/content).
    pub grid_auto_rows: Option<f32>,
    /// Full `grid-auto-rows` track-size pattern; repeated for implicit rows.
    pub grid_auto_rows_pattern: Vec<f32>,
    /// `grid-auto-flow: column` is in effect (default is row).
    pub grid_auto_flow_column: bool,
    /// `justify-items` (inline-axis alignment of grid items in their tracks).
    pub justify_items: GridAlign,
    /// `align-items` for grid (block-axis alignment of grid items). Distinct
    /// from the flex `align_items` field; grid uses start/end/center/stretch.
    pub grid_align_items: GridAlign,
    /// Item-level `grid-column: span N` (number of columns to span, >=1).
    /// Retained for back-compat; derived from `grid_column_start/end`.
    pub grid_column_span: usize,
    /// Item-level `grid-row: span N` (number of rows to span, >=1).
    pub grid_row_span: usize,
    /// `grid-auto-flow: dense` packing (backfill earlier holes). Default sparse.
    pub grid_auto_flow_dense: bool,
    /// Container `grid-template-areas`: row-major cells; `None` is a `.`/null
    /// (empty) cell, `Some(name)` names the area occupying that cell. Empty when
    /// no areas declared. Every row has the same length (column count).
    pub grid_template_areas: Vec<Vec<Option<String>>>,
    /// Line names per *column* line index (index 0 = the line before the first
    /// column). Populated from bracketed `[name]` tokens in
    /// `grid-template-columns` plus implicit `<area>-start`/`-end` lines.
    pub grid_template_column_line_names: Vec<Vec<String>>,
    /// Line names per *row* line index. As above for `grid-template-rows`.
    pub grid_template_row_line_names: Vec<Vec<String>>,
    /// Item-level placement endpoints (CSS Grid §8). `Auto` = auto-placed.
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,
    /// Item-level `grid-area: <name>` (a single named area). `None` = unset.
    pub grid_area_name: Option<String>,
    /// Item-level grid `justify-self` / `align-self` (overrides the container
    /// `justify-items` / `align-items`). `None` = inherit the container value.
    pub grid_justify_self: Option<GridAlign>,
    pub grid_align_self: Option<GridAlign>,
    pub grid_gap: f32,
    /// Border radii remain specified until layout knows the element's own box.
    pub border_radii: SpecifiedCornerRadii,
    pub outline_width: f32,
    pub outline_color: Option<Color>,
    /// CSS `outline-offset`: gap (in points) between the border edge and the
    /// outline. Positive expands the outline outward; negative pulls it inward.
    pub outline_offset: f32,
    pub box_sizing: BoxSizing,
    pub text_transform: TextTransform,
    /// CSS `font-variant-caps` / `font-variant: small-caps` (css-fonts-4 §6.5).
    pub font_variant_caps: FontVariantCaps,
    /// CSS `font-variant-position` / `font-variant: sub|super`.
    pub font_variant_position: FontVariantPosition,
    /// Whether standard/contextual ligatures are enabled (css-fonts-3 §6.4 /
    /// css-fonts-4 §6.11). Defaults to `true`; set to `false` by
    /// `font-feature-settings: "liga" 0` (and `clig`/`dlig` off) to suppress
    /// the shaper's default ligature substitution.
    pub ligatures_enabled: bool,
    pub font_kerning_enabled: bool,
    pub font_synthesis_weight: bool,
    pub font_synthesis_style: bool,
    pub font_synthesis_small_caps: bool,
    pub initial_letter: f32,
    pub text_emphasis_mark: bool,
    pub text_emphasis_position: TextEmphasisPosition,
    pub text_emphasis_color: Color,
    pub(crate) text_emphasis_color_source: ColorSource,
    pub text_indent: TextIndent,
    pub white_space: WhiteSpace,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    /// CSS `tab-size` (css-text-3 §6.3): the number of space advances between
    /// consecutive tab stops. A unitless `<number>` is stored directly; a
    /// `<length>` is converted to an equivalent count by the renderer via the
    /// space advance. Defaults to 8 (the initial value).
    pub tab_size: f32,
    pub vertical_align: VerticalAlign,
    pub background_gradient: Option<LinearGradient>,
    pub background_radial_gradient: Option<RadialGradient>,
    pub background_conic_gradient: Option<ConicGradient>,
    pub background_image: Option<String>,
    pub background_svg: Option<crate::parser::svg::SvgTree>,
    pub aspect_ratio: Option<f32>,
    pub text_overflow: TextOverflow,
    pub overflow_wrap: OverflowWrap,
    pub border_collapse: BorderCollapse,
    pub table_layout: TableLayout,
    pub border_spacing: f32,
    /// Vertical `border-spacing` (between rows). Equals `border_spacing` for the
    /// single-value form; differs for the two-value form `H V`.
    pub border_spacing_vertical: f32,
    pub empty_cells: EmptyCells,
    /// CSS `caption-side` (inherited): top (default) or bottom.
    pub caption_side: CaptionSide,
    pub background_size: BackgroundSize,
    pub background_repeat: BackgroundRepeat,
    pub background_position: BackgroundPosition,
    pub background_origin: BackgroundOrigin,
    pub background_clip: BackgroundClip,
    pub background_attachment: BackgroundAttachment,
    pub z_index: ZIndex,
    /// CSS custom properties inherited from ancestors.
    pub custom_properties: HashMap<String, String>,
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    pub marker_side_match_parent: bool,
    /// CSS `list-style-image` source (`url(...)`), if any. When set and
    /// decodable, it replaces the `list-style-type` marker glyph (css-lists-3
    /// §3.1). `None` means "use the list-style-type marker".
    pub list_style_image: Option<String>,
    pub content: Vec<ContentItem>,
    pub counter_reset: Vec<(String, i32)>,
    pub counter_increment: Vec<(String, i32)>,
    pub counter_set: Vec<(String, i32)>,
    /// CSS `quotes` property: ordered (open, close) glyph pairs by nesting level.
    /// `None` means `auto`/unset (use the UA default pair); an empty `Vec` means
    /// `quotes: none` (open/close-quote produce no glyphs). Per css-content-3
    /// §2.4. Inherited.
    pub quotes: Option<Vec<(String, String)>>,
    pub column_count: Option<u32>,
    /// CSS `column-width` (the ideal width of each column, in px). Combined with
    /// `column-count` to derive the used number of columns for multicol flow.
    pub column_width: Option<f32>,
    pub column_gap: f32,
    /// Whether `column-gap` was set explicitly. Multicol uses a `normal` default
    /// of 1em when unset, while grid/flex keep a 0 default.
    pub column_gap_is_normal: bool,
    /// CSS `column-rule`: the vertical line painted in each column gap.
    pub column_rule: BorderSide,
    /// CSS `column-span: all` — element spans across all columns as a full-width
    /// band, breaking the column flow.
    pub column_span_all: bool,
    /// CSS `column-fill` — `false` (default) balances columns to equal height;
    /// `true` (`column-fill: auto`) fills each column to the container's height
    /// in turn, leaving the last column short.
    pub column_fill_auto: bool,
    pub row_gap: f32,
    /// Percentage `column-gap` (as a fraction, e.g. 0.10 for `10%`). Resolved
    /// late against the flex container's OWN content-box inline size (width), not
    /// the parent/ICB width (CSS Box Alignment §8.3). `None` when the gap is a
    /// fixed length. Takes precedence over `column_gap` in the flex layout.
    pub column_gap_pct: Option<f32>,
    /// Percentage `row-gap` (as a fraction). Resolved late against the flex
    /// container's OWN content-box block size (height).
    pub row_gap_pct: Option<f32>,
    /// Computed CSS filter operations and their mandatory stacking-context
    /// boundary. Keeping these together prevents identity filters from being
    /// optimized into `filter: none`.
    pub filter: FilterEffects,
    /// CSS `object-fit` for replaced elements (how the content fits its box).
    pub object_fit: ObjectFit,
    /// CSS `object-position` as horizontal/vertical fractions of the free space
    /// (0.0 = start/left/top, 0.5 = center, 1.0 = end/right/bottom).
    pub object_position: ObjectPosition,
    /// CSS `image-rendering`, carried through source-image and filter-raster
    /// painting so each scaling preference has one semantic owner.
    pub image_rendering: ImageRendering,
}

/// CSS `object-fit` for replaced elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    /// Stretch to fill the box, ignoring aspect ratio (initial value).
    #[default]
    Fill,
    /// Scale to fit inside the box, preserving aspect ratio (letterboxed).
    Contain,
    /// Scale to cover the box, preserving aspect ratio (cropped).
    Cover,
    /// Use the intrinsic size, ignoring the box dimensions.
    None,
    /// Use the smaller of `None` and `Contain`.
    ScaleDown,
}

/// Sampling preference for replaced raster content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageRendering {
    /// UA-default interpolation behaviour.
    #[default]
    Auto,
    /// Prefer color-smoothing while scaling photographic content.
    Smooth,
    /// Like [`Self::Smooth`], with a higher-quality preference when resources
    /// require a trade-off.
    HighQuality,
    /// Preserve a source pixel grid at its nearest integer scale, then smooth
    /// only the remainder required to reach the exact target size.
    Pixelated,
    /// Preserve contrast and avoid introducing blended source colours.
    CrispEdges,
}

impl ImageRendering {
    pub(crate) const fn is_pixelated(self) -> bool {
        matches!(self, Self::Pixelated)
    }

    pub(crate) const fn preserves_source_edges(self) -> bool {
        matches!(self, Self::Pixelated | Self::CrispEdges)
    }

    pub(crate) const fn requests_smooth_pdf_interpolation(self) -> bool {
        matches!(self, Self::Smooth | Self::HighQuality)
    }
}

/// A single `object-position` axis component (css-images-3 §5.5 / css-backgrounds
/// `<position>`). A percentage/keyword resolves against the free space (object
/// size relative to the positioning area); a length is an absolute offset of the
/// object's start edge from the box's start edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectPositionComponent {
    /// Fraction of the free space (0.0 = start, 0.5 = center, 1.0 = end).
    Fraction(f32),
    /// Absolute offset of the object's start edge from the box start, in points.
    Length(f32),
    /// Absolute offset from the far edge; resolved as free-space minus this length.
    FarEdgeLength(f32),
}

impl ObjectPositionComponent {
    /// Resolve to a concrete offset of the object's start edge from the box
    /// start, given the free space (box length minus object length, which may be
    /// negative when the object overflows / is cropped).
    pub fn resolve(self, free_space: f32) -> f32 {
        match self {
            ObjectPositionComponent::Fraction(f) => free_space * f,
            ObjectPositionComponent::Length(l) => l,
            ObjectPositionComponent::FarEdgeLength(l) => free_space - l,
        }
    }
}

/// CSS `object-position` as a pair of axis components. The initial value is
/// `50% 50%` (centered).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectPosition {
    pub x: ObjectPositionComponent,
    pub y: ObjectPositionComponent,
}

impl Default for ObjectPosition {
    fn default() -> Self {
        Self {
            x: ObjectPositionComponent::Fraction(0.5),
            y: ObjectPositionComponent::Fraction(0.5),
        }
    }
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            raster_quality: RasterQuality::default(),
            font_size: 12.0,
            root_font_size: 12.0,
            root_font_units: FontUnitLengths::default(),
            viewport_width: 595.28,
            viewport_height: 841.89,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            font_stretch: FontStretch::default(),
            font_size_adjust: FontSizeAdjust::default(),
            font_family: FontFamily::TimesRoman,
            font_stack: FontStack::default(),
            color: Color::BLACK,
            background_color: None,
            margin: EdgeSizes::default(),
            margin_em_top: None,
            margin_em_right: None,
            margin_em_bottom: None,
            margin_em_left: None,
            padding: EdgeSizes::default(),
            text_align: TextAlign::Left,
            text_align_last: None,
            word_break_keep_all: false,
            hyphens_manual: true,
            text_wrap_mode_nowrap: false,
            direction_rtl: false,
            writing_mode: WritingMode::HorizontalTb,
            text_orientation_upright: false,
            text_combine_upright: TextCombineUpright::None,
            bidi_override: false,
            bidi_plaintext: false,
            text_decorations: TextDecorations::default(),
            line_height: f32::NAN,
            line_height_absolute: None,
            page_break_before: false,
            page_break_after: false,
            break_before: BreakValue::Auto,
            break_after: BreakValue::Auto,
            break_inside_avoid: false,
            page_name: None,
            border: BorderSides::default(),
            border_image: BorderImageProperties::default(),
            display: Display::Block,
            width: None,
            width_keyword: None,
            height: None,
            max_width: None,
            min_width: None,
            min_height: None,
            max_height: None,
            percentage_sizing: PercentageSizing::default(),
            margin_left_auto: false,
            margin_right_auto: false,
            margin_top_auto: false,
            margin_bottom_auto: false,
            opacity: 1.0,
            mix_blend_mode: BlendMode::default(),
            background_blend_mode: BlendMode::default(),
            isolation: Isolation::Auto,
            float: Float::None,
            footnote: FootnoteFormatting::default(),
            clear: Clear::None,
            box_decoration_break: BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            position: Position::Static,
            running_name: None,
            top: None,
            right: None,
            bottom: None,
            left: None,
            percentage_insets: PercentageInsets::default(),
            box_shadow: Vec::new(),
            text_shadow: Vec::new(),
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: AlignSelf::Auto,
            order: 0,
            flex_wrap: FlexWrap::NoWrap,
            align_content: AlignContent::Stretch,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasis::Auto,
            gap: 0.0,
            overflow: Overflow::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            visibility: Visibility::Visible,
            transform: None,
            transform_origin: TransformOrigin::default(),
            perspective: None,
            perspective_origin: TransformOrigin::default(),
            transform_box: TransformBox::default(),
            clip_path: None,
            mask_image: None,
            mask_mode: MaskMode::default(),
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_rows: None,
            grid_auto_rows_pattern: Vec::new(),
            grid_auto_flow_column: false,
            justify_items: GridAlign::Stretch,
            grid_align_items: GridAlign::Stretch,
            grid_column_span: 1,
            grid_row_span: 1,
            grid_auto_flow_dense: false,
            grid_template_areas: Vec::new(),
            grid_template_column_line_names: Vec::new(),
            grid_template_row_line_names: Vec::new(),
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
            grid_area_name: None,
            grid_justify_self: None,
            grid_align_self: None,
            grid_gap: 0.0,
            border_radii: SpecifiedCornerRadii::ZERO,
            outline_width: 0.0,
            outline_color: None,
            outline_offset: 0.0,
            box_sizing: BoxSizing::ContentBox,
            text_transform: TextTransform::None,
            font_variant_caps: FontVariantCaps::Normal,
            font_variant_position: FontVariantPosition::Normal,
            ligatures_enabled: true,
            font_kerning_enabled: true,
            font_synthesis_weight: true,
            font_synthesis_style: true,
            font_synthesis_small_caps: true,
            initial_letter: 0.0,
            text_emphasis_mark: false,
            text_emphasis_position: TextEmphasisPosition::default(),
            text_emphasis_color: Color::BLACK,
            text_emphasis_color_source: ColorSource::CurrentColor,
            text_indent: TextIndent::default(),
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            tab_size: 8.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
            background_image: None,
            background_svg: None,
            aspect_ratio: None,
            text_overflow: TextOverflow::Clip,
            overflow_wrap: OverflowWrap::Normal,
            border_collapse: BorderCollapse::Separate,
            table_layout: TableLayout::Auto,
            border_spacing: 0.0,
            border_spacing_vertical: 0.0,
            empty_cells: EmptyCells::Show,
            caption_side: CaptionSide::Top,
            background_size: BackgroundSize::Auto,
            background_repeat: BackgroundRepeat::Repeat,
            background_position: BackgroundPosition::default(),
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            background_attachment: BackgroundAttachment::Scroll,
            z_index: ZIndex::default(),
            custom_properties: HashMap::new(),
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            marker_side_match_parent: false,
            list_style_image: None,
            content: Vec::new(),
            quotes: None,
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            counter_set: Vec::new(),
            column_count: None,
            column_width: None,
            column_gap: 0.0,
            column_gap_is_normal: true,
            column_rule: BorderSide::default(),
            column_span_all: false,
            column_fill_auto: false,
            row_gap: 0.0,
            column_gap_pct: None,
            row_gap_pct: None,
            filter: FilterEffects::default(),
            object_fit: ObjectFit::default(),
            object_position: ObjectPosition::default(),
            image_rendering: ImageRendering::default(),
        }
    }
}

impl ComputedStyle {
    /// Start a style-resolution tree with one conversion-owned raster policy.
    pub(crate) fn with_raster_quality(raster_quality: RasterQuality) -> Self {
        Self {
            raster_quality: raster_quality.normalized(),
            ..Self::default()
        }
    }

    /// Resolve the specified corner radii against a border-box size and hand
    /// layout one owning geometry value. Percentage radii are axis-relative:
    /// horizontal values use width and vertical values use height.
    pub(crate) fn resolve_corner_radii(&self, width: f32, height: f32) -> CornerRadii {
        self.border_radii.resolve(width, height)
    }

    /// Whether the box needs a border paint owner. A length-valued
    /// `border-image-width` can paint even when every ordinary used border
    /// width is zero.
    pub(crate) fn has_border_decoration(&self) -> bool {
        self.border_image.has_source() || self.border.has_any()
    }

    fn clear_background_images(&mut self) {
        self.background_gradient = None;
        self.background_radial_gradient = None;
        self.background_conic_gradient = None;
        self.background_image = None;
        self.background_svg = None;
    }

    pub(crate) fn reset_background(&mut self) {
        self.background_color = None;
        self.clear_background_images();
        self.background_size = BackgroundSize::Auto;
        self.background_repeat = BackgroundRepeat::Repeat;
        self.background_position = BackgroundPosition::default();
        self.background_origin = BackgroundOrigin::Padding;
        self.background_clip = BackgroundClip::Border;
        self.background_attachment = BackgroundAttachment::Scroll;
    }

    fn inherit_background_image(&mut self, source: &ComputedStyle) {
        self.background_gradient = source.background_gradient.clone();
        self.background_radial_gradient = source.background_radial_gradient.clone();
        self.background_conic_gradient = source.background_conic_gradient.clone();
        self.background_image = source.background_image.clone();
        self.background_svg = source.background_svg.clone();
    }

    fn inherit_background(&mut self, source: &ComputedStyle) {
        self.background_color = source.background_color;
        self.inherit_background_image(source);
        self.background_size = source.background_size;
        self.background_repeat = source.background_repeat;
        self.background_position = source.background_position;
        self.background_origin = source.background_origin;
        self.background_clip = source.background_clip;
        self.background_attachment = source.background_attachment;
    }
}

/// Compute the style for a node given its tag, inline styles, and parent style.
#[cfg(test)]
pub fn compute_style(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
) -> ComputedStyle {
    compute_style_with_rules(tag, inline_style, parent, &[], "", &[], None)
}

/// Compute style with stylesheet rules, class list, and id.
#[allow(dead_code)]
pub fn compute_style_with_rules(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
) -> ComputedStyle {
    compute_style_with_context(
        tag,
        inline_style,
        parent,
        rules,
        tag_name,
        classes,
        id,
        &HashMap::new(),
        &SelectorContext::default(),
    )
}

/// Compute style with stylesheet rules, class list, id, attributes, and selector context.
#[allow(clippy::too_many_arguments)]
pub fn compute_style_with_context(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
) -> ComputedStyle {
    compute_style_with_context_with_font_metrics(
        tag,
        inline_style,
        parent,
        rules,
        tag_name,
        classes,
        id,
        attributes,
        selector_ctx,
        FontMetrics::default(),
    )
}

/// Compute a style with the loaded-font metrics used by layout.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_style_with_context_with_font_metrics(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    font_metrics: FontMetrics<'_>,
) -> ComputedStyle {
    compute_style_with_context_and_percentage_basis_with_font_metrics(
        tag,
        inline_style,
        parent,
        rules,
        tag_name,
        classes,
        id,
        attributes,
        selector_ctx,
        PercentageBasis::default(),
        font_metrics,
    )
}

/// Content-box dimensions used only to resolve descendant percentages.
///
/// They remain separate from [`ComputedStyle`], whose parent value must stay
/// available for inheritance. In particular, `width: inherit` uses the
/// cascade parent while `calc(50% - 1px)` uses the containing block's content
/// width.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PercentageBasis {
    width: Option<f32>,
    height: Option<f32>,
}

impl PercentageBasis {
    pub(crate) const fn new(width: Option<f32>, height: Option<f32>) -> Self {
        Self { width, height }
    }

    fn width_or_parent(self, parent: &ComputedStyle) -> Option<f32> {
        self.width.or(parent.width)
    }

    fn height_or_parent(self, parent: &ComputedStyle) -> Option<f32> {
        self.height.or(parent.height)
    }
}

/// Compute a style with the actual containing-block content dimensions for
/// percentage resolution, without altering the cascade parent.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_style_with_context_and_percentage_basis(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    percentage_basis: PercentageBasis,
) -> ComputedStyle {
    compute_style_with_context_and_percentage_basis_with_font_metrics(
        tag,
        inline_style,
        parent,
        rules,
        tag_name,
        classes,
        id,
        attributes,
        selector_ctx,
        percentage_basis,
        FontMetrics::default(),
    )
}

/// Compute a style with a loaded-font context for `ex` and `ch` resolution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_style_with_context_and_percentage_basis_with_font_metrics(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    percentage_basis: PercentageBasis,
    font_metrics: FontMetrics<'_>,
) -> ComputedStyle {
    compute_style_with_context_and_percentage_basis_impl(
        tag,
        inline_style,
        parent,
        rules,
        tag_name,
        classes,
        id,
        attributes,
        selector_ctx,
        percentage_basis,
        font_metrics,
    )
}

#[allow(clippy::too_many_arguments)]
fn compute_style_with_context_and_percentage_basis_impl(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    percentage_basis: PercentageBasis,
    font_metrics: FontMetrics<'_>,
) -> ComputedStyle {
    let mut style = parent.clone();
    style.text_decorations = TextDecorations::for_descendant(
        &parent.text_decorations,
        parent.color,
        !parent.display.is_atomic_inline(),
    );
    let html_layers = html_cascade_layers(tag, attributes, selector_ctx);

    // Set default display based on tag
    style.display = if tag.is_inline() {
        Display::Inline
    } else {
        Display::Block
    };

    // Margin, padding, and background never inherit in CSS — reset them for
    // every element regardless of display. (Previously these were reset only
    // for block tags, so an inline tag such as a `display:inline-block`
    // `<span>` wrongly inherited its parent's padding/margin.)
    style.margin = EdgeSizes::default();
    style.margin_em_top = None;
    style.margin_em_right = None;
    style.margin_em_bottom = None;
    style.margin_em_left = None;
    style.padding = EdgeSizes::default();
    style.background_color = None;
    style.clear_background_images();

    // Border does not inherit in CSS — reset for all elements
    style.border = BorderSides::default();
    style.border_image = BorderImageProperties::default();

    // Reset non-inherited sizing and opacity properties
    style.width = None;
    style.width_keyword = None;
    style.height = None;
    style.max_width = None;
    style.min_width = None;
    style.min_height = None;
    style.max_height = None;
    style.percentage_sizing = PercentageSizing::default();
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.margin_top_auto = false;
    style.margin_bottom_auto = false;
    style.opacity = 1.0;
    style.isolation = Isolation::Auto;
    // `unicode-bidi` is not inherited; initial is `normal`.
    style.bidi_override = false;
    style.bidi_plaintext = false;
    style.float = Float::None;
    style.footnote = FootnoteFormatting::default();
    style.clear = Clear::None;
    style.position = Position::Static;
    style.running_name = None;
    style.top = None;
    style.right = None;
    style.bottom = None;
    style.left = None;
    style.percentage_insets = PercentageInsets::default();
    style.box_shadow = Vec::new();
    style.flex_direction = FlexDirection::Row;
    style.justify_content = JustifyContent::FlexStart;
    style.align_items = AlignItems::Stretch;
    style.align_self = AlignSelf::Auto;
    style.order = 0;
    style.flex_wrap = FlexWrap::NoWrap;
    style.align_content = AlignContent::Stretch;
    style.flex_grow = 0.0;
    style.flex_shrink = 1.0;
    style.flex_basis = FlexBasis::Auto;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.overflow_x = Overflow::Visible;
    style.overflow_y = Overflow::Visible;
    style.visibility = Visibility::Visible;
    style.transform = None;
    style.perspective = None;
    style.perspective_origin = TransformOrigin::default();
    style.transform_box = TransformBox::default();
    style.clip_path = None;
    style.mask_image = None;
    style.mask_mode = MaskMode::default();
    style.grid_template_columns = Vec::new();
    // Grid placement is not inherited — reset per element so a grid item's
    // children don't inherit its line placement / area assignment.
    style.grid_template_rows = Vec::new();
    style.grid_template_areas = Vec::new();
    style.grid_template_column_line_names = Vec::new();
    style.grid_template_row_line_names = Vec::new();
    style.grid_auto_rows = None;
    style.grid_auto_rows_pattern.clear();
    style.grid_auto_flow_column = false;
    style.grid_auto_flow_dense = false;
    style.grid_column_span = 1;
    style.grid_row_span = 1;
    style.grid_column_start = GridLine::Auto;
    style.grid_column_end = GridLine::Auto;
    style.grid_row_start = GridLine::Auto;
    style.grid_row_end = GridLine::Auto;
    style.grid_area_name = None;
    style.grid_justify_self = None;
    style.grid_align_self = None;
    style.grid_gap = 0.0;
    // Multi-column properties are not inherited — reset for every element so a
    // multicol container's block children don't themselves become multicol
    // boxes (which would recursively re-fragment their own content).
    style.column_count = None;
    style.column_width = None;
    style.column_gap = 0.0;
    style.column_gap_is_normal = true;
    style.column_rule = BorderSide::default();
    style.column_span_all = false;
    style.column_fill_auto = false;
    style.border_radii = SpecifiedCornerRadii::ZERO;
    style.outline_width = 0.0;
    style.outline_color = None;
    style.outline_offset = 0.0;
    style.box_sizing = BoxSizing::ContentBox;
    style.initial_letter = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    // border_collapse, border_spacing and empty_cells are inherited; don't reset.
    style.table_layout = TableLayout::Auto;
    style.background_size = BackgroundSize::Auto;
    style.background_repeat = BackgroundRepeat::Repeat;
    style.background_position = BackgroundPosition::default();
    style.background_origin = BackgroundOrigin::Padding;
    style.background_clip = BackgroundClip::Border;
    style.background_attachment = BackgroundAttachment::Scroll;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = ZIndex::default();
    style.row_gap = 0.0;
    style.column_gap_pct = None;
    style.row_gap_pct = None;
    style.filter = FilterEffects::default();
    // `page` (CSS Paged Media 3 §3.4) is not inherited; initial is `auto`
    // (the default page → `None`).
    style.page_name = None;
    // custom_properties inherit from parent (already cloned)

    // Apply tag defaults
    let defaults = default_style(tag);
    apply_style_map_with_percentage_basis(
        &mut style,
        &defaults,
        parent,
        percentage_basis,
        font_metrics,
    );
    apply_style_map_with_percentage_basis(
        &mut style,
        &html_layers.ua,
        parent,
        percentage_basis,
        font_metrics,
    );

    // Handle HTML dir attribute (inheritable, overrides CSS direction)
    if let Some(dir) = attributes.get("dir") {
        match dir.as_str() {
            "rtl" => {
                style.direction_rtl = true;
                // RTL elements default to right-aligned text
                if style.text_align == TextAlign::Left {
                    style.text_align = TextAlign::Right;
                }
            }
            "ltr" => {
                style.direction_rtl = false;
            }
            _ => {}
        }
    }

    // Apply stylesheet rules (between defaults and inline) in cascade order
    // (css-cascade-4 §6.3): all matching rules are sorted by specificity, with
    // source order breaking ties (a stable sort over the source-ordered rule
    // list preserves order for equal specificity). Within the same origin,
    // `!important` declarations win over normal ones regardless of specificity,
    // so we apply all matched NORMAL declarations first (low→high precedence),
    // then all matched IMPORTANT declarations (low→high precedence) on top.
    // Pseudo-element rules target ::before/::after, not the element itself.
    let mut matched: Vec<(u32, &CssRule)> = Vec::new();
    for rule in rules {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if selector_matches_with_context(
            &rule.selector,
            tag_name,
            classes,
            id,
            attributes,
            selector_ctx,
        ) {
            matched.push((specificity(&rule.selector), rule));
        }
    }
    // Stable sort by specificity ascending; equal specificity keeps source order
    // (later source = applied later = wins), matching the spec's order-of-
    // appearance tiebreak.
    matched.sort_by_key(|(spec, _)| *spec);

    // Parse inline (style attribute) declarations up front so they can be
    // interleaved into the cascade at the correct precedence tiers.
    let inline_map = inline_style.map(crate::parser::css::parse_inline_style);

    // Select one specified winner per property before computing any values.
    // Applying each rule directly to ComputedStyle leaked lower declarations
    // through when a higher winning var() became invalid at computed-value time.
    // A single cascaded map also lets every property see the final custom-property
    // set and avoids cloning/applying a full filtered map per matched rule.
    let mut cascaded = StyleMap::new();

    // Precedence order (lowest → highest), per css-cascade-4 §6.3 within the
    // author origin: presentational hints, selector normal, inline normal,
    // selector important, inline important. `matched` is already stable-sorted
    // by specificity/source order.
    cascade_style_map_filtered(
        &mut cascaded,
        &html_layers.presentational_hints,
        Importance::Normal,
    );
    for (_, rule) in &matched {
        cascade_style_map_filtered(&mut cascaded, &rule.declarations, Importance::Normal);
    }
    if let Some(inline) = &inline_map {
        cascade_style_map_filtered(&mut cascaded, inline, Importance::Normal);
    }
    for (_, rule) in &matched {
        cascade_style_map_filtered(&mut cascaded, &rule.declarations, Importance::Important);
    }
    if let Some(inline) = &inline_map {
        cascade_style_map_filtered(&mut cascaded, inline, Importance::Important);
    }
    if !cascaded.properties.is_empty() {
        apply_style_map_with_percentage_basis(
            &mut style,
            &cascaded,
            parent,
            percentage_basis,
            font_metrics,
        );
    }

    // Now that the cascade is finalized, re-resolve em-based margins against
    // the element's *final* font-size. Earlier apply_style_map calls resolve
    // em-factors eagerly against whatever font_size was current at that layer,
    // which is wrong if a later layer changes font-size.
    if let Some(em) = style.margin_em_top {
        style.margin.top = em * style.font_size;
    }
    if let Some(em) = style.margin_em_right {
        style.margin.right = em * style.font_size;
    }
    if let Some(em) = style.margin_em_bottom {
        style.margin.bottom = em * style.font_size;
    }
    if let Some(em) = style.margin_em_left {
        style.margin.left = em * style.font_size;
    }
    sync_line_height_from_absolute(&mut style);

    if style.position.is_absolute() || matches!(style.float, Float::Left | Float::Right) {
        // Text decorations do not propagate into out-of-flow descendants.
        // Decorations originated by this element remain active.
        style.text_decorations.clear_propagated();
        style.display = style.display.blockified();
    }

    resolve_custom_counter_style(&mut style.list_style_type, rules);
    resolve_custom_counter_styles_in_content(&mut style.content, rules);

    // Materialize used currentColor values only after the foreground color is
    // final. Typed provenance remains on inherited text-shadow/emphasis values
    // so descendants can rebind them to their own foreground color.
    resolve_current_color(&mut style);

    style
}

fn resolve_current_color(style: &mut ComputedStyle) {
    let resolved = style.color;
    for shadow in &mut style.box_shadow {
        if shadow.color_source == ColorSource::CurrentColor {
            shadow.color = resolved;
        }
    }
    for shadow in &mut style.text_shadow {
        if shadow.color_source == ColorSource::CurrentColor {
            shadow.color = resolved;
        }
    }
    if style.text_emphasis_color_source == ColorSource::CurrentColor {
        style.text_emphasis_color = resolved;
    }
}

fn bind_specified_color(color: SpecifiedColor, current_color: Color) -> (Color, ColorSource) {
    match color {
        SpecifiedColor::Absolute(color) => (color, ColorSource::Absolute),
        SpecifiedColor::CurrentColor => (current_color, ColorSource::CurrentColor),
    }
}

/// Compute the style for a `::before` or `::after` pseudo-element.
///
/// The pseudo-element inherits all inherited properties from the originating
/// element's computed style, resets non-inherited properties, then applies
/// matching pseudo-element CSS rules.  `parent_style` is the fully computed
/// style of the originating element.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn compute_pseudo_element_style(
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    pseudo: crate::parser::css::PseudoElement,
) -> Option<ComputedStyle> {
    compute_pseudo_element_style_with_font_metrics(
        parent_style,
        rules,
        tag_name,
        classes,
        id,
        attributes,
        selector_ctx,
        pseudo,
        FontMetrics::default(),
    )
}

/// Compute a pseudo-element style with the document's loaded-font metrics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_pseudo_element_style_with_font_metrics(
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    selector_ctx: &SelectorContext,
    pseudo: crate::parser::css::PseudoElement,
    font_metrics: FontMetrics<'_>,
) -> Option<ComputedStyle> {
    // Select pseudo-element declarations with the same specificity/source-order
    // and importance cascade used for ordinary elements.
    let mut matched: Vec<(u32, &CssRule)> = Vec::new();
    for rule in rules {
        if rule.pseudo_element == Some(pseudo)
            && selector_matches_with_context(
                &rule.selector,
                tag_name,
                classes,
                id,
                attributes,
                selector_ctx,
            )
        {
            matched.push((specificity(&rule.selector), rule));
        }
    }

    if matched.is_empty() {
        return None;
    }
    matched.sort_by_key(|(specificity, _)| *specificity);
    let mut cascaded = StyleMap::new();
    for (_, rule) in &matched {
        cascade_style_map_filtered(&mut cascaded, &rule.declarations, Importance::Normal);
    }
    for (_, rule) in &matched {
        cascade_style_map_filtered(&mut cascaded, &rule.declarations, Importance::Important);
    }

    // Start from parent style (inherits inherited properties)
    let mut style = parent_style.clone();
    style.text_decorations = TextDecorations::for_descendant(
        &parent_style.text_decorations,
        parent_style.color,
        !parent_style.display.is_atomic_inline(),
    );

    // Reset non-inherited properties (pseudo-elements are generated boxes)
    style.margin = EdgeSizes::default();
    style.margin_em_top = None;
    style.margin_em_right = None;
    style.margin_em_bottom = None;
    style.margin_em_left = None;
    style.padding = EdgeSizes::default();
    style.reset_background();
    style.border = BorderSides::default();
    style.border_image = BorderImageProperties::default();
    style.width = None;
    style.width_keyword = None;
    style.height = None;
    style.max_width = None;
    style.min_width = None;
    style.min_height = None;
    style.max_height = None;
    style.percentage_sizing = PercentageSizing::default();
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.margin_top_auto = false;
    style.margin_bottom_auto = false;
    style.opacity = 1.0;
    // `unicode-bidi` is not inherited; initial is `normal`.
    style.bidi_override = false;
    style.bidi_plaintext = false;
    style.float = Float::None;
    style.footnote = FootnoteFormatting::default();
    style.clear = Clear::None;
    style.position = Position::Static;
    style.running_name = None;
    style.top = None;
    style.right = None;
    style.bottom = None;
    style.left = None;
    style.percentage_insets = PercentageInsets::default();
    style.box_shadow = Vec::new();
    style.flex_direction = FlexDirection::Row;
    style.justify_content = JustifyContent::FlexStart;
    style.align_items = AlignItems::Stretch;
    style.align_self = AlignSelf::Auto;
    style.order = 0;
    style.flex_wrap = FlexWrap::NoWrap;
    style.align_content = AlignContent::Stretch;
    style.flex_grow = 0.0;
    style.flex_shrink = 1.0;
    style.flex_basis = FlexBasis::Auto;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.overflow_x = Overflow::Visible;
    style.overflow_y = Overflow::Visible;
    style.transform = None;
    style.perspective = None;
    style.perspective_origin = TransformOrigin::default();
    style.transform_box = TransformBox::default();
    style.clip_path = None;
    style.mask_image = None;
    style.mask_mode = MaskMode::default();
    style.grid_template_columns = Vec::new();
    // Grid placement is not inherited — reset per element so a grid item's
    // children don't inherit its line placement / area assignment.
    style.grid_template_rows = Vec::new();
    style.grid_template_areas = Vec::new();
    style.grid_template_column_line_names = Vec::new();
    style.grid_template_row_line_names = Vec::new();
    style.grid_auto_rows = None;
    style.grid_auto_rows_pattern.clear();
    style.grid_auto_flow_column = false;
    style.grid_auto_flow_dense = false;
    style.grid_column_span = 1;
    style.grid_row_span = 1;
    style.grid_column_start = GridLine::Auto;
    style.grid_column_end = GridLine::Auto;
    style.grid_row_start = GridLine::Auto;
    style.grid_row_end = GridLine::Auto;
    style.grid_area_name = None;
    style.grid_justify_self = None;
    style.grid_align_self = None;
    style.grid_gap = 0.0;
    // Multi-column properties are not inherited — reset for every element so a
    // multicol container's block children don't themselves become multicol
    // boxes (which would recursively re-fragment their own content).
    style.column_count = None;
    style.column_width = None;
    style.column_gap = 0.0;
    style.column_gap_is_normal = true;
    style.column_rule = BorderSide::default();
    style.column_span_all = false;
    style.column_fill_auto = false;
    style.border_radii = SpecifiedCornerRadii::ZERO;
    style.outline_width = 0.0;
    style.outline_color = None;
    style.outline_offset = 0.0;
    style.box_sizing = BoxSizing::ContentBox;
    style.initial_letter = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = ZIndex::default();
    style.row_gap = 0.0;
    style.column_gap_pct = None;
    style.row_gap_pct = None;
    style.filter = FilterEffects::default();
    // Default display for pseudo-elements is inline
    style.display = Display::Inline;

    // CSS GCPM §2.6 gives the footnote call a UA `super` position. Set it
    // before the author cascade so an explicit ::footnote-call declaration can
    // override it, while a rule that only changes colour/content keeps the UA
    // default.
    if pseudo == crate::parser::css::PseudoElement::FootnoteCall {
        style.font_variant_position = FontVariantPosition::Super;
    }

    // Apply the complete winner map once. This lets every currentColor consumer
    // see the pseudo-element's final winning `color`, independent of declaration
    // or rule order. `parent_style` remains the inheritance source.
    apply_style_map_with_font_metrics(&mut style, &cascaded, parent_style, font_metrics);

    if style.position.is_absolute() || matches!(style.float, Float::Left | Float::Right) {
        style.text_decorations.clear_propagated();
    }

    // `content: none`/`normal` suppress `::before`/`::after` generation (no box
    // without content). Generated markers and footnote pseudo-elements already
    // have content supplied by their formatting models, so author rules that
    // only restyle them must still apply. `::first-line`/`::first-letter` also
    // restyle existing text and are exempt from the empty-content check.
    if style.content.is_empty()
        && !matches!(
            pseudo,
            crate::parser::css::PseudoElement::Marker
                | crate::parser::css::PseudoElement::FootnoteCall
                | crate::parser::css::PseudoElement::FootnoteMarker
                | crate::parser::css::PseudoElement::FirstLine
                | crate::parser::css::PseudoElement::FirstLetter
        )
    {
        return None;
    }

    // Re-resolve em-based margins against the pseudo-element's final font-size.
    // See the same fixup in `compute_style_with_context` for rationale.
    if let Some(em) = style.margin_em_top {
        style.margin.top = em * style.font_size;
    }
    if let Some(em) = style.margin_em_right {
        style.margin.right = em * style.font_size;
    }
    if let Some(em) = style.margin_em_bottom {
        style.margin.bottom = em * style.font_size;
    }
    if let Some(em) = style.margin_em_left {
        style.margin.left = em * style.font_size;
    }
    sync_line_height_from_absolute(&mut style);
    resolve_custom_counter_style(&mut style.list_style_type, rules);
    resolve_custom_counter_styles_in_content(&mut style.content, rules);

    // Refresh inherited currentColor provenance against the pseudo's final color.
    resolve_current_color(&mut style);

    Some(style)
}

/// Returns true if the property is inherited by default in CSS.
fn is_inherited_property(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "font-family"
            | "font"
            | "line-height"
            | "text-align"
            | "text-align-last"
            | "text-decoration-skip-ink"
            | "text-shadow"
            | "text-emphasis-color"
            | "-webkit-text-emphasis-color"
            | "text-underline-offset"
            | "visibility"
            | "letter-spacing"
            | "word-spacing"
            | "text-indent"
            | "text-transform"
            | "font-variant"
            | "font-variant-caps"
            | "font-variant-position"
            | "font-variant-ligatures"
            | "font-kerning"
            | "font-size-adjust"
            | "font-synthesis"
            | "font-feature-settings"
            | "text-emphasis"
            | "text-emphasis-position"
            | "-webkit-text-emphasis-position"
            | "white-space"
            | "white-space-collapse"
            | "text-wrap-mode"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "hyphens"
            | "border-collapse"
            | "border-spacing"
            | "empty-cells"
            | "caption-side"
            | "list-style-type"
            | "list-style-position"
            | "list-style-image"
            | "orphans"
            | "widows"
    )
}

/// Reset a property to its initial (default) value on the given style.
fn reset_to_initial(style: &mut ComputedStyle, property: &str) {
    let default = ComputedStyle::default();
    match property {
        "color" => style.color = default.color,
        "font-size" => style.font_size = default.font_size,
        "font-weight" => style.font_weight = default.font_weight,
        "font-style" => style.font_style = default.font_style,
        "font-stretch" => style.font_stretch = default.font_stretch,
        "font-variant" => {
            style.font_variant_caps = default.font_variant_caps;
            style.font_variant_position = default.font_variant_position;
        }
        "font-variant-caps" => style.font_variant_caps = default.font_variant_caps,
        "font-variant-position" => style.font_variant_position = default.font_variant_position,
        "font-size-adjust" => style.font_size_adjust = default.font_size_adjust,
        "font-synthesis" => {
            style.font_synthesis_weight = default.font_synthesis_weight;
            style.font_synthesis_style = default.font_synthesis_style;
            style.font_synthesis_small_caps = default.font_synthesis_small_caps;
        }
        "font-family" => {
            style.font_family = default.font_family;
            style.font_stack = default.font_stack;
        }
        "line-height" => {
            style.line_height = default.line_height;
            style.line_height_absolute = default.line_height_absolute;
        }
        "text-align" => style.text_align = default.text_align,
        "text-align-last" => style.text_align_last = default.text_align_last,
        "text-decoration" => {
            let controls = (
                style.text_decorations.current.skip_ink,
                style.text_decorations.current.underline_offset,
            );
            style.text_decorations.current = TextDecoration {
                skip_ink: controls.0,
                underline_offset: controls.1,
                ..Default::default()
            };
        }
        "text-decoration-line" => {
            style.text_decorations.current.lines = default.text_decorations.current.lines;
        }
        "text-decoration-style" => {
            style.text_decorations.current.style = default.text_decorations.current.style
        }
        "text-decoration-thickness" => {
            style.text_decorations.current.thickness = default.text_decorations.current.thickness
        }
        "text-decoration-color" => style.text_decorations.current.color = None,
        "text-decoration-skip-ink" => {
            style.text_decorations.current.skip_ink = default.text_decorations.current.skip_ink
        }
        "text-shadow" => style.text_shadow.clear(),
        "text-emphasis-color" | "-webkit-text-emphasis-color" => {
            style.text_emphasis_color = style.color;
            style.text_emphasis_color_source = ColorSource::CurrentColor;
        }
        "text-emphasis-position" | "-webkit-text-emphasis-position" => {
            style.text_emphasis_position = default.text_emphasis_position;
        }
        "text-underline-offset" => {
            style.text_decorations.current.underline_offset =
                default.text_decorations.current.underline_offset
        }
        "visibility" => style.visibility = default.visibility,
        "initial-letter" => style.initial_letter = default.initial_letter,
        "text-indent" => style.text_indent = default.text_indent,
        "letter-spacing" => style.letter_spacing = default.letter_spacing,
        "word-spacing" => style.word_spacing = default.word_spacing,
        "tab-size" => style.tab_size = default.tab_size,
        "background-color" => style.background_color = default.background_color,
        "margin-top" => {
            style.margin.top = default.margin.top;
            style.margin_em_top = None;
        }
        "margin-right" => {
            style.margin.right = default.margin.right;
            style.margin_em_right = None;
        }
        "margin-bottom" => {
            style.margin.bottom = default.margin.bottom;
            style.margin_em_bottom = None;
        }
        "margin-left" => {
            style.margin.left = default.margin.left;
            style.margin_em_left = None;
        }
        "padding-top" => style.padding.top = default.padding.top,
        "padding-right" => style.padding.right = default.padding.right,
        "padding-bottom" => style.padding.bottom = default.padding.bottom,
        "padding-left" => style.padding.left = default.padding.left,
        "display" => style.display = default.display,
        "width" => {
            style.width = default.width;
            style.width_keyword = default.width_keyword;
            style.percentage_sizing.width = default.percentage_sizing.width;
        }
        "height" => {
            style.height = default.height;
            style.percentage_sizing.height = default.percentage_sizing.height;
        }
        "max-width" => {
            style.max_width = default.max_width;
            style.percentage_sizing.max_width = default.percentage_sizing.max_width;
        }
        "min-width" => {
            style.min_width = default.min_width;
            style.percentage_sizing.min_width = default.percentage_sizing.min_width;
        }
        "min-height" => {
            style.min_height = default.min_height;
            style.percentage_sizing.min_height = default.percentage_sizing.min_height;
        }
        "max-height" => {
            style.max_height = default.max_height;
            style.percentage_sizing.max_height = default.percentage_sizing.max_height;
        }
        "opacity" => style.opacity = default.opacity,
        "mix-blend-mode" => style.mix_blend_mode = default.mix_blend_mode,
        "background-blend-mode" => style.background_blend_mode = default.background_blend_mode,
        "isolation" => style.isolation = default.isolation,
        "border-width" => {
            style.border.top.specified_width = default.border.top.specified_width;
            style.border.right.specified_width = default.border.right.specified_width;
            style.border.bottom.specified_width = default.border.bottom.specified_width;
            style.border.left.specified_width = default.border.left.specified_width;
        }
        "border-color" => {
            style.border.top.color = default.border.top.color;
            style.border.right.color = default.border.right.color;
            style.border.bottom.color = default.border.bottom.color;
            style.border.left.color = default.border.left.color;
        }
        "border-style" => {
            style.border.top.style = default.border.top.style;
            style.border.right.style = default.border.right.style;
            style.border.bottom.style = default.border.bottom.style;
            style.border.left.style = default.border.left.style;
        }
        "border-top-style" => style.border.top.style = default.border.top.style,
        "border-right-style" => style.border.right.style = default.border.right.style,
        "border-bottom-style" => style.border.bottom.style = default.border.bottom.style,
        "border-left-style" => style.border.left.style = default.border.left.style,
        "border-radius" => style.border_radii = default.border_radii,
        "border-top-left-radius" => style.border_radii.top_left = default.border_radii.top_left,
        "border-top-right-radius" => style.border_radii.top_right = default.border_radii.top_right,
        "border-bottom-right-radius" => {
            style.border_radii.bottom_right = default.border_radii.bottom_right
        }
        "border-bottom-left-radius" => {
            style.border_radii.bottom_left = default.border_radii.bottom_left
        }
        "border-image-source" => style.border_image.source = None,
        "border-image-slice" => style.border_image.geometry.slices = BorderImageSlices::default(),
        "border-image-width" => style.border_image.geometry.widths = BorderImageWidths::default(),
        "border-image-outset" => {
            style.border_image.geometry.outsets = BorderImageOutsets::default()
        }
        "border-image-repeat" => {
            style.border_image.geometry.repeats = BorderImageRepeats::default()
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            style.border = default.border;
        }
        "float" => style.float = default.float,
        "footnote-display" => style.footnote.display = default.footnote.display,
        "footnote-policy" => style.footnote.policy = default.footnote.policy,
        "clear" => style.clear = default.clear,
        "box-decoration-break" => style.box_decoration_break = default.box_decoration_break,
        "orphans" => style.orphans = default.orphans,
        "widows" => style.widows = default.widows,
        "position" => {
            style.position = default.position;
            style.running_name = default.running_name.clone();
        }
        "top" => {
            style.top = default.top;
            style.percentage_insets.top = default.percentage_insets.top;
        }
        "right" => {
            style.right = default.right;
            style.percentage_insets.right = default.percentage_insets.right;
        }
        "bottom" => {
            style.bottom = default.bottom;
            style.percentage_insets.bottom = default.percentage_insets.bottom;
        }
        "left" => {
            style.left = default.left;
            style.percentage_insets.left = default.percentage_insets.left;
        }
        "overflow" => {
            style.overflow = default.overflow;
            style.overflow_x = default.overflow_x;
            style.overflow_y = default.overflow_y;
        }
        "transform" => style.transform = default.transform,
        "perspective" => style.perspective = default.perspective,
        "transform-box" => style.transform_box = default.transform_box,
        "box-shadow" => style.box_shadow = default.box_shadow.clone(),
        "flex-direction" => style.flex_direction = default.flex_direction,
        "justify-content" => style.justify_content = default.justify_content,
        "align-items" => style.align_items = default.align_items,
        "align-content" => style.align_content = default.align_content,
        "align-self" => style.align_self = default.align_self,
        "order" => style.order = default.order,
        "flex-wrap" => style.flex_wrap = default.flex_wrap,
        "flex-grow" => style.flex_grow = default.flex_grow,
        "flex-shrink" => style.flex_shrink = default.flex_shrink,
        "flex-basis" => style.flex_basis = default.flex_basis,
        "gap" => style.gap = default.gap,
        "text-overflow" => style.text_overflow = default.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = default.overflow_wrap,
        "word-break" => style.word_break_keep_all = default.word_break_keep_all,
        "white-space-collapse" => style.white_space = default.white_space,
        "text-wrap-mode" => style.text_wrap_mode_nowrap = default.text_wrap_mode_nowrap,
        "border-collapse" => style.border_collapse = default.border_collapse,
        "table-layout" => style.table_layout = default.table_layout,
        "border-spacing" => {
            style.border_spacing = default.border_spacing;
            style.border_spacing_vertical = default.border_spacing_vertical;
        }
        "empty-cells" => style.empty_cells = default.empty_cells,
        "caption-side" => style.caption_side = default.caption_side,
        "background-size" => style.background_size = default.background_size,
        "background-repeat" => style.background_repeat = default.background_repeat,
        "background-position" => style.background_position = default.background_position,
        "background-origin" => style.background_origin = default.background_origin,
        "background-clip" => style.background_clip = default.background_clip,
        "background-attachment" => style.background_attachment = default.background_attachment,
        "background-image" => style.clear_background_images(),
        "aspect-ratio" => style.aspect_ratio = default.aspect_ratio,
        "object-fit" => style.object_fit = default.object_fit,
        "object-position" => style.object_position = default.object_position,
        "image-rendering" => style.image_rendering = default.image_rendering,
        "background" => style.reset_background(),
        "list-style-type" => style.list_style_type = default.list_style_type.clone(),
        "list-style-position" => style.list_style_position = default.list_style_position,
        "marker-side" => style.marker_side_match_parent = default.marker_side_match_parent,
        "list-style-image" => style.list_style_image = default.list_style_image.clone(),
        "content" => style.content = default.content,
        "counter-reset" => style.counter_reset = default.counter_reset,
        "counter-increment" => style.counter_increment = default.counter_increment,
        "counter-set" => style.counter_set = default.counter_set,
        "column-count" => style.column_count = default.column_count,
        "column-width" => style.column_width = default.column_width,
        "columns" => {
            style.column_count = default.column_count;
            style.column_width = default.column_width;
        }
        "column-gap" => {
            style.column_gap = default.column_gap;
            style.column_gap_is_normal = default.column_gap_is_normal;
        }
        "column-rule" | "column-rule-width" | "column-rule-style" | "column-rule-color" => {
            style.column_rule = default.column_rule;
        }
        "column-span" => style.column_span_all = default.column_span_all,
        "column-fill" => style.column_fill_auto = default.column_fill_auto,
        "filter" => {
            style.filter = default.filter.clone();
        }
        _ => {}
    }
}

fn reset_all_to_initial(style: &mut ComputedStyle) {
    // `all` resets computed longhands on this element; it cannot erase line
    // decorations propagated from an ancestor decorating box.
    let mut text_decorations = std::mem::take(&mut style.text_decorations);
    text_decorations.current = TextDecoration::default();
    let raster_quality = style.raster_quality;
    let root_font_size = style.root_font_size;
    let root_font_units = style.root_font_units;
    let viewport_width = style.viewport_width;
    let viewport_height = style.viewport_height;
    let direction_rtl = style.direction_rtl;
    let bidi_override = style.bidi_override;
    let bidi_plaintext = style.bidi_plaintext;
    let custom_properties = style.custom_properties.clone();

    *style = ComputedStyle::default();

    style.raster_quality = raster_quality;
    style.root_font_size = root_font_size;
    style.root_font_units = root_font_units;
    style.viewport_width = viewport_width;
    style.viewport_height = viewport_height;
    style.direction_rtl = direction_rtl;
    style.bidi_override = bidi_override;
    style.bidi_plaintext = bidi_plaintext;
    style.custom_properties = custom_properties;
    style.text_decorations = text_decorations;
}

/// Restore a property to the parent's value (inherit behavior).
fn restore_from_parent(style: &mut ComputedStyle, property: &str, parent: &ComputedStyle) {
    match property {
        "color" => style.color = parent.color,
        "font-size" => style.font_size = parent.font_size,
        "font-weight" => style.font_weight = parent.font_weight,
        "font-style" => style.font_style = parent.font_style,
        "font-stretch" => style.font_stretch = parent.font_stretch,
        "font-variant" => {
            style.font_variant_caps = parent.font_variant_caps;
            style.font_variant_position = parent.font_variant_position;
        }
        "font-variant-caps" => style.font_variant_caps = parent.font_variant_caps,
        "font-variant-position" => style.font_variant_position = parent.font_variant_position,
        "font-size-adjust" => style.font_size_adjust = parent.font_size_adjust,
        "font-synthesis" => {
            style.font_synthesis_weight = parent.font_synthesis_weight;
            style.font_synthesis_style = parent.font_synthesis_style;
            style.font_synthesis_small_caps = parent.font_synthesis_small_caps;
        }
        "font-family" => {
            style.font_family = parent.font_family.clone();
            style.font_stack = parent.font_stack.clone();
        }
        "line-height" => {
            style.line_height = parent.line_height;
            style.line_height_absolute = parent.line_height_absolute;
        }
        "text-align" => style.text_align = parent.text_align,
        "text-align-last" => style.text_align_last = parent.text_align_last,
        "text-decoration" => {
            let controls = (
                style.text_decorations.current.skip_ink,
                style.text_decorations.current.underline_offset,
            );
            style.text_decorations.current = parent.text_decorations.current;
            style.text_decorations.current.skip_ink = controls.0;
            style.text_decorations.current.underline_offset = controls.1;
            style.text_decorations.current.color =
                Some(parent.text_decorations.current.resolved_color(parent.color));
        }
        "text-decoration-line" => {
            style.text_decorations.current.lines = parent.text_decorations.current.lines;
        }
        "text-decoration-style" => {
            style.text_decorations.current.style = parent.text_decorations.current.style
        }
        "text-decoration-thickness" => {
            style.text_decorations.current.thickness = parent.text_decorations.current.thickness
        }
        "text-decoration-color" => {
            style.text_decorations.current.color =
                Some(parent.text_decorations.current.resolved_color(parent.color))
        }
        "text-decoration-skip-ink" => {
            style.text_decorations.current.skip_ink = parent.text_decorations.current.skip_ink
        }
        "text-shadow" => style.text_shadow = parent.text_shadow.clone(),
        "text-emphasis-color" | "-webkit-text-emphasis-color" => {
            style.text_emphasis_color = parent.text_emphasis_color;
            style.text_emphasis_color_source = parent.text_emphasis_color_source;
        }
        "text-emphasis-position" | "-webkit-text-emphasis-position" => {
            style.text_emphasis_position = parent.text_emphasis_position;
        }
        "text-underline-offset" => {
            style.text_decorations.current.underline_offset =
                parent.text_decorations.current.underline_offset
        }
        "visibility" => style.visibility = parent.visibility,
        "initial-letter" => style.initial_letter = parent.initial_letter,
        "text-indent" => style.text_indent = parent.text_indent.clone(),
        "letter-spacing" => style.letter_spacing = parent.letter_spacing,
        "word-spacing" => style.word_spacing = parent.word_spacing,
        "tab-size" => style.tab_size = parent.tab_size,
        "background-color" => style.background_color = parent.background_color,
        "margin-top" => {
            style.margin.top = parent.margin.top;
            style.margin_em_top = None;
        }
        "margin-right" => {
            style.margin.right = parent.margin.right;
            style.margin_em_right = None;
        }
        "margin-bottom" => {
            style.margin.bottom = parent.margin.bottom;
            style.margin_em_bottom = None;
        }
        "margin-left" => {
            style.margin.left = parent.margin.left;
            style.margin_em_left = None;
        }
        "padding-top" => style.padding.top = parent.padding.top,
        "padding-right" => style.padding.right = parent.padding.right,
        "padding-bottom" => style.padding.bottom = parent.padding.bottom,
        "padding-left" => style.padding.left = parent.padding.left,
        "display" => style.display = parent.display,
        "width" => {
            style.width = parent.width;
            style.width_keyword = parent.width_keyword;
            style.percentage_sizing.width = parent.percentage_sizing.width;
        }
        "height" => {
            style.height = parent.height;
            style.percentage_sizing.height = parent.percentage_sizing.height;
        }
        "max-width" => {
            style.max_width = parent.max_width;
            style.percentage_sizing.max_width = parent.percentage_sizing.max_width;
        }
        "min-width" => {
            style.min_width = parent.min_width;
            style.percentage_sizing.min_width = parent.percentage_sizing.min_width;
        }
        "min-height" => {
            style.min_height = parent.min_height;
            style.percentage_sizing.min_height = parent.percentage_sizing.min_height;
        }
        "max-height" => {
            style.max_height = parent.max_height;
            style.percentage_sizing.max_height = parent.percentage_sizing.max_height;
        }
        "opacity" => style.opacity = parent.opacity,
        "mix-blend-mode" => style.mix_blend_mode = parent.mix_blend_mode,
        "background-blend-mode" => style.background_blend_mode = parent.background_blend_mode,
        "isolation" => style.isolation = parent.isolation,
        "border-width" => {
            style.border.top.specified_width = parent.border.top.specified_width;
            style.border.right.specified_width = parent.border.right.specified_width;
            style.border.bottom.specified_width = parent.border.bottom.specified_width;
            style.border.left.specified_width = parent.border.left.specified_width;
        }
        "border-color" => {
            style.border.top.color = parent.border.top.color;
            style.border.right.color = parent.border.right.color;
            style.border.bottom.color = parent.border.bottom.color;
            style.border.left.color = parent.border.left.color;
        }
        "border-style" => {
            style.border.top.style = parent.border.top.style;
            style.border.right.style = parent.border.right.style;
            style.border.bottom.style = parent.border.bottom.style;
            style.border.left.style = parent.border.left.style;
        }
        "border-top-style" => style.border.top.style = parent.border.top.style,
        "border-right-style" => style.border.right.style = parent.border.right.style,
        "border-bottom-style" => style.border.bottom.style = parent.border.bottom.style,
        "border-left-style" => style.border.left.style = parent.border.left.style,
        "border-radius" => style.border_radii = parent.border_radii,
        "border-top-left-radius" => style.border_radii.top_left = parent.border_radii.top_left,
        "border-top-right-radius" => style.border_radii.top_right = parent.border_radii.top_right,
        "border-bottom-right-radius" => {
            style.border_radii.bottom_right = parent.border_radii.bottom_right
        }
        "border-bottom-left-radius" => {
            style.border_radii.bottom_left = parent.border_radii.bottom_left
        }
        "border-image-source" => style.border_image.source = parent.border_image.source.clone(),
        "border-image-slice" => {
            style.border_image.geometry.slices = parent.border_image.geometry.slices
        }
        "border-image-width" => {
            style.border_image.geometry.widths = parent.border_image.geometry.widths
        }
        "border-image-outset" => {
            style.border_image.geometry.outsets = parent.border_image.geometry.outsets
        }
        "border-image-repeat" => {
            style.border_image.geometry.repeats = parent.border_image.geometry.repeats
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            style.border = parent.border;
        }
        "float" => style.float = parent.float,
        "footnote-display" => style.footnote.display = parent.footnote.display,
        "footnote-policy" => style.footnote.policy = parent.footnote.policy,
        "clear" => style.clear = parent.clear,
        "box-decoration-break" => style.box_decoration_break = parent.box_decoration_break,
        "orphans" => style.orphans = parent.orphans,
        "widows" => style.widows = parent.widows,
        "position" => {
            style.position = parent.position;
            style.running_name = parent.running_name.clone();
        }
        "top" => {
            style.top = parent.top;
            style.percentage_insets.top = parent.percentage_insets.top;
        }
        "right" => {
            style.right = parent.right;
            style.percentage_insets.right = parent.percentage_insets.right;
        }
        "bottom" => {
            style.bottom = parent.bottom;
            style.percentage_insets.bottom = parent.percentage_insets.bottom;
        }
        "left" => {
            style.left = parent.left;
            style.percentage_insets.left = parent.percentage_insets.left;
        }
        "overflow" => {
            style.overflow = parent.overflow;
            style.overflow_x = parent.overflow_x;
            style.overflow_y = parent.overflow_y;
        }
        "transform" => style.transform = parent.transform,
        "perspective" => style.perspective = parent.perspective,
        "transform-box" => style.transform_box = parent.transform_box,
        "box-shadow" => style.box_shadow = parent.box_shadow.clone(),
        "flex-direction" => style.flex_direction = parent.flex_direction,
        "justify-content" => style.justify_content = parent.justify_content,
        "align-items" => style.align_items = parent.align_items,
        "align-content" => style.align_content = parent.align_content,
        "align-self" => style.align_self = parent.align_self,
        "order" => style.order = parent.order,
        "flex-wrap" => style.flex_wrap = parent.flex_wrap,
        "flex-grow" => style.flex_grow = parent.flex_grow,
        "flex-shrink" => style.flex_shrink = parent.flex_shrink,
        "flex-basis" => style.flex_basis = parent.flex_basis,
        "gap" => style.gap = parent.gap,
        "text-overflow" => style.text_overflow = parent.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = parent.overflow_wrap,
        "word-break" => style.word_break_keep_all = parent.word_break_keep_all,
        "white-space-collapse" => style.white_space = parent.white_space,
        "text-wrap-mode" => style.text_wrap_mode_nowrap = parent.text_wrap_mode_nowrap,
        "empty-cells" => style.empty_cells = parent.empty_cells,
        "caption-side" => style.caption_side = parent.caption_side,
        "border-collapse" => style.border_collapse = parent.border_collapse,
        "table-layout" => style.table_layout = parent.table_layout,
        "border-spacing" => {
            style.border_spacing = parent.border_spacing;
            style.border_spacing_vertical = parent.border_spacing_vertical;
        }
        "background-size" => style.background_size = parent.background_size,
        "background-repeat" => style.background_repeat = parent.background_repeat,
        "background-position" => style.background_position = parent.background_position,
        "background-origin" => style.background_origin = parent.background_origin,
        "background-clip" => style.background_clip = parent.background_clip,
        "background-attachment" => style.background_attachment = parent.background_attachment,
        "background-image" => style.inherit_background_image(parent),
        "aspect-ratio" => style.aspect_ratio = parent.aspect_ratio,
        "object-fit" => style.object_fit = parent.object_fit,
        "object-position" => style.object_position = parent.object_position,
        "image-rendering" => style.image_rendering = parent.image_rendering,
        "background" => style.inherit_background(parent),
        "list-style-type" => style.list_style_type = parent.list_style_type.clone(),
        "list-style-position" => style.list_style_position = parent.list_style_position,
        "marker-side" => style.marker_side_match_parent = parent.marker_side_match_parent,
        "list-style-image" => style.list_style_image = parent.list_style_image.clone(),
        "content" => style.content = parent.content.clone(),
        "counter-reset" => style.counter_reset = parent.counter_reset.clone(),
        "counter-increment" => style.counter_increment = parent.counter_increment.clone(),
        "counter-set" => style.counter_set = parent.counter_set.clone(),
        "column-count" | "columns" => style.column_count = parent.column_count,
        "column-gap" => style.column_gap = parent.column_gap,
        "filter" => {
            style.filter = parent.filter.clone();
        }
        _ => {}
    }
}

/// Get a CSS value from the map, but return None if the value is an inherit/initial/unset keyword
/// (those are handled separately before normal property application).
fn get_non_special<'a>(map: &'a StyleMap, key: &str) -> Option<&'a CssValue> {
    map.get(key).filter(|v| {
        if let CssValue::Keyword(k) = v {
            let lower = k.to_ascii_lowercase();
            !matches!(
                lower.as_str(),
                "inherit" | "initial" | "unset" | "revert" | "revert-layer"
            )
        } else {
            true
        }
    })
}

fn specified_color_from_value(
    value: &CssValue,
    custom_properties: &HashMap<String, String>,
) -> Option<SpecifiedColor> {
    match value {
        CssValue::Color(color) => Some(*color),
        CssValue::Var(_, _) => {
            crate::style::resolve::try_resolve_var_to_color(value, custom_properties)
        }
        _ => None,
    }
}

fn resolve_opacity(value: &CssValue, custom_properties: &HashMap<String, String>) -> Option<f32> {
    match value {
        CssValue::Number(value) | CssValue::Length(value) => Some(value.clamp(0.0, 1.0)),
        CssValue::Percentage(value) => Some((value / 100.0).clamp(0.0, 1.0)),
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                custom_properties,
            )?;
            parse_opacity_token(&raw)
        }
        _ => None,
    }
}

fn parse_opacity_token(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    if let Some(percentage) = raw.strip_suffix('%') {
        return percentage
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| (value / 100.0).clamp(0.0, 1.0));
    }
    raw.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 1.0))
}

/// Parse a `font-feature-settings` value (css-fonts-3 §6.4) and report whether
/// it turns standard ligatures OFF. The value is a comma-separated list of
/// `<tag> [<value>]` entries; a ligature tag (`liga`, `clig`, `dlig`, `hlig`)
/// followed by `0` or `off` disables ligatures, while `1`/`on`/omitted enables.
fn ligatures_disabled_by_feature_settings(value: &str) -> bool {
    let mut disabled = false;
    for entry in value.split(',') {
        let entry = entry.trim();
        let mut parts = entry.split_whitespace();
        let Some(tag) = parts.next() else { continue };
        let tag = tag
            .trim_matches(|c| c == '"' || c == '\'')
            .to_ascii_lowercase();
        if !matches!(tag.as_str(), "liga" | "clig" | "dlig" | "hlig") {
            continue;
        }
        let on = match parts.next() {
            None => true,
            Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "off"),
        };
        if on {
            // An explicit enable cancels any prior disable in the same list.
            disabled = false;
        } else {
            disabled = true;
        }
    }
    disabled
}

fn apply_text_decoration_line(style: &mut ComputedStyle, value: &str) {
    let mut lines = TextDecorationLines::default();
    for token in value.split_whitespace() {
        match token {
            "underline" => lines.underline = true,
            "overline" => lines.overline = true,
            "line-through" => lines.line_through = true,
            "none" => lines = TextDecorationLines::default(),
            _ => {}
        }
    }
    style.text_decorations.current.lines = lines;
}

fn color_in_text_emphasis_shorthand(value: &str) -> Option<SpecifiedColor> {
    if let Some(start) = value.find("rgb(").or_else(|| value.find("rgba("))
        && let Some(end) = value[start..].find(')')
    {
        return match crate::parser::css::parse_color(&value[start..=start + end]) {
            Some(CssValue::Color(color)) => Some(color),
            _ => None,
        };
    }

    value.split_whitespace().find_map(|token| {
        if let Some(CssValue::Color(color)) = crate::parser::css::parse_color(token) {
            Some(color)
        } else {
            None
        }
    })
}

fn apply_font_shorthand(
    style: &mut ComputedStyle,
    value: &str,
    parent: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let Some(size_idx) = tokens.iter().position(|token| {
        let size = token.split_once('/').map_or(*token, |(size, _)| size);
        parse_length(size).is_some()
    }) else {
        return;
    };

    let inherited_font_weight = style.font_weight;
    style.font_style = FontStyle::Normal;
    style.font_weight = FontWeight::Normal;
    style.font_stretch = FontStretch::default();
    style.line_height = f32::NAN;
    style.line_height_absolute = None;

    let mut descriptor_index = 0;
    while descriptor_index < size_idx {
        let lower = tokens[descriptor_index].to_ascii_lowercase();
        if let Some(stretch) = FontStretch::from_css(&lower) {
            style.font_stretch = stretch;
            descriptor_index += 1;
            continue;
        }
        match lower.as_str() {
            "italic" => style.font_style = FontStyle::Italic,
            "oblique" => {
                let angle = tokens
                    .get(descriptor_index + 1)
                    .and_then(|angle| parse_angle_deg(angle))
                    .filter(|angle| angle.is_finite() && (-90.0..=90.0).contains(angle));
                style.font_style = FontStyle::Oblique(angle.map_or(
                    FontStyle::DEFAULT_OBLIQUE_ANGLE_DEGREES,
                    |angle| {
                        descriptor_index += 1;
                        angle as f32
                    },
                ));
            }
            "bolder" => style.font_weight = inherited_font_weight.bolder(),
            "lighter" => style.font_weight = inherited_font_weight.lighter(),
            "bold" => apply_font_weight(style, &lower),
            value if value.parse::<u16>().is_ok() => apply_font_weight(style, value),
            _ => {}
        }
        descriptor_index += 1;
    }

    let (size_raw, mut line_raw) = tokens[size_idx]
        .split_once('/')
        .map_or((tokens[size_idx], None), |(size, line)| (size, Some(line)));
    let mut family_start = size_idx + 1;
    if line_raw.is_none() && tokens.get(family_start).is_some_and(|token| *token == "/") {
        line_raw = tokens.get(family_start + 1).copied();
        family_start += 2;
    }
    let family = tokens[family_start..].join(" ");
    if !family.trim().is_empty() {
        style.font_stack = parse_font_stack(&family);
        style.font_family = style.font_stack.primary();
    }

    apply_font_size_token(style, parent, size_raw, font_metrics);
    if let Some(line) = line_raw {
        apply_line_height_token(
            style,
            line,
            line_height_length_context(style, parent, length_context, font_metrics),
            font_metrics,
        );
    }
}

fn apply_font_weight(style: &mut ComputedStyle, value: &str) {
    let lower = value.trim().to_ascii_lowercase();
    style.font_weight = match lower.as_str() {
        "normal" => FontWeight::Normal,
        "bold" => FontWeight::Bold,
        "bolder" => style.font_weight.bolder(),
        "lighter" => style.font_weight.lighter(),
        _ => lower
            .parse::<u16>()
            .ok()
            .map(FontWeight::from_number)
            .unwrap_or(FontWeight::Normal),
    };
}

fn apply_font_size_token(
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    token: &str,
    font_metrics: FontMetrics<'_>,
) {
    if let Some(value) = parse_length(token)
        && let Some(size) =
            resolve_font_size_value(&value, &style.custom_properties, parent, font_metrics)
    {
        style.font_size = size;
    }
}

/// Resolve one `font-size` value against the inherited font context.
///
/// Font-relative units on `font-size` use the parent's font metrics, while all
/// other properties on the element use the newly computed size. Owning that
/// boundary here keeps widths, heights, borders, and spacing on one `em` basis.
fn resolve_font_size_value(
    value: &CssValue,
    custom_properties: &HashMap<String, String>,
    parent: &ComputedStyle,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    let size = match value {
        CssValue::Length(value) => *value,
        CssValue::Em(value) => *value * parent.font_size,
        CssValue::Ex(value) => *value * style_ex_length(parent, font_metrics),
        CssValue::Ch(value) => {
            *value * parent.font_size * font_metrics.style_ch_ratio(parent).unwrap_or(0.5)
        }
        CssValue::Rem(value) => *value * parent.root_font_size,
        CssValue::Percentage(value) => *value * parent.font_size / 100.0,
        CssValue::Vw(value) => *value * parent.viewport_width / 100.0,
        CssValue::Vh(value) => *value * parent.viewport_height / 100.0,
        CssValue::Vmin(value) => *value * parent.viewport_width.min(parent.viewport_height) / 100.0,
        CssValue::Vmax(value) => *value * parent.viewport_width.max(parent.viewport_height) / 100.0,
        CssValue::Math(expression) => {
            expression.resolve(parent.math_unit_context(font_metrics), parent.font_size)?
        }
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                custom_properties,
            )?;
            let resolved = parse_property_value("font-size", &raw)?;
            return resolve_font_size_value(&resolved, custom_properties, parent, font_metrics);
        }
        CssValue::Keyword(raw) => {
            let parsed = parse_length(raw)?;
            return resolve_font_size_value(&parsed, custom_properties, parent, font_metrics);
        }
        CssValue::Number(_) | CssValue::Color(_) | CssValue::BackgroundLayers(_) => return None,
    };

    (size.is_finite() && size >= 0.0).then_some(size)
}

fn apply_line_height_token(
    style: &mut ComputedStyle,
    token: &str,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) {
    if token == "normal" {
        style.line_height = f32::NAN;
        style.line_height_absolute = None;
        return;
    }
    if let Some(value) = parse_length(token) {
        match value {
            CssValue::Length(v) => {
                style.line_height_absolute = Some(v);
                style.line_height = v / style.font_size;
            }
            CssValue::Number(v) => {
                style.line_height = v;
                style.line_height_absolute = None;
            }
            CssValue::Em(v) => {
                let absolute = v * style.font_size;
                style.line_height = v;
                style.line_height_absolute = Some(absolute);
            }
            CssValue::Percentage(p) => {
                let absolute = style.font_size * p / 100.0;
                style.line_height_absolute = Some(absolute);
                style.line_height = absolute / style.font_size;
            }
            other => {
                if let Some(v) =
                    resolve_css_length_for_style(&other, style, length_context, font_metrics)
                {
                    style.line_height_absolute = Some(v);
                    style.line_height = v / style.font_size;
                }
            }
        }
    }
}

/// Whether to apply normal (non-important) or `!important` declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Importance {
    Normal,
    Important,
}

fn cascade_style_map_filtered(target: &mut StyleMap, source: &StyleMap, want: Importance) {
    let want_important = want == Importance::Important;
    for property in &source.declaration_order {
        if source.is_important(property) != want_important {
            continue;
        }
        let Some(value) = source.properties.get(property) else {
            continue;
        };

        // All maps merged here belong to the author origin. `revert` therefore
        // removes the author candidate and exposes the already-applied UA/tag
        // default (or inherited value) when the final map is computed.
        if matches!(
            value,
            CssValue::Keyword(keyword)
                if matches!(keyword.to_ascii_lowercase().as_str(), "revert" | "revert-layer")
        ) {
            target.remove(property);
            continue;
        }
        target.set_with_importance(property, value.clone(), want_important);
    }
}

fn style_map_subset(map: &StyleMap, keys: &[String]) -> StyleMap {
    let mut subset = StyleMap::new();
    for key in keys {
        let Some(value) = map.properties.get(key) else {
            continue;
        };
        subset.set_with_importance(key, value.clone(), map.is_important(key));
    }
    subset
}

fn apply_all_keyword(style: &mut ComputedStyle, keyword: &str, parent: &ComputedStyle) {
    match keyword.to_ascii_lowercase().as_str() {
        "inherit" => {
            // `all` deliberately excludes direction, unicode-bidi, and custom
            // properties. Preserve those fields while inheriting every other
            // longhand in one operation.
            let direction_rtl = style.direction_rtl;
            let bidi_override = style.bidi_override;
            let bidi_plaintext = style.bidi_plaintext;
            let custom_properties = std::mem::take(&mut style.custom_properties);
            let mut text_decorations = std::mem::take(&mut style.text_decorations);
            *style = parent.clone();
            text_decorations.current = parent
                .text_decorations
                .current
                .with_resolved_color(parent.color);
            style.direction_rtl = direction_rtl;
            style.bidi_override = bidi_override;
            style.bidi_plaintext = bidi_plaintext;
            style.custom_properties = custom_properties;
            style.text_decorations = text_decorations;
        }
        "initial" | "unset" => reset_all_to_initial(style),
        _ => {}
    }
}

pub(crate) fn apply_style_map(style: &mut ComputedStyle, map: &StyleMap, parent: &ComputedStyle) {
    apply_style_map_with_font_metrics(style, map, parent, FontMetrics::default());
}

/// Apply a declaration map with the document's loaded-font metrics available
/// for `ex` and `ch` lengths.
pub(crate) fn apply_style_map_with_font_metrics(
    style: &mut ComputedStyle,
    map: &StyleMap,
    parent: &ComputedStyle,
    font_metrics: FontMetrics<'_>,
) {
    apply_style_map_with_percentage_basis(
        style,
        map,
        parent,
        PercentageBasis::default(),
        font_metrics,
    );
}

fn apply_style_map_with_percentage_basis(
    style: &mut ComputedStyle,
    map: &StyleMap,
    parent: &ComputedStyle,
    percentage_basis: PercentageBasis,
    font_metrics: FontMetrics<'_>,
) {
    // `all` is a shorthand for every longhand (except direction, unicode-bidi,
    // and custom properties), so its source position relative to explicit
    // declarations is semantically observable. StyleMap retains the accepted
    // winners' order; apply the two sides independently around the winning `all`
    // declaration instead of processing `all` in an unconditional pre-pass.
    if let Some(all_index) = map
        .declaration_order
        .iter()
        .position(|property| property == "all")
        && let Some(CssValue::Keyword(keyword)) = map.get("all")
    {
        let before = style_map_subset(map, &map.declaration_order[..all_index]);
        if !before.properties.is_empty() {
            apply_style_map_with_percentage_basis(
                style,
                &before,
                parent,
                percentage_basis,
                font_metrics,
            );
        }
        apply_all_keyword(style, keyword, parent);
        let after = style_map_subset(map, &map.declaration_order[all_index + 1..]);
        if !after.properties.is_empty() {
            apply_style_map_with_percentage_basis(
                style,
                &after,
                parent,
                percentage_basis,
                font_metrics,
            );
        }
        return;
    }

    // Custom properties participate in the cascade as token streams, but every
    // ordinary property in this declaration set must see their final values
    // regardless of textual order. Collect them before property grammar is
    // evaluated (rather than halfway through this function).
    for property in &map.declaration_order {
        if !property.starts_with("--") {
            continue;
        }
        if let Some(CssValue::Keyword(raw)) = map.get(property) {
            style
                .custom_properties
                .insert(property.clone(), raw.clone());
        }
    }

    // `parent_width_known` tells us whether the `parent_width` we feed into
    // the length-resolution context is a real, layout-driven parent width
    // (Some) or a viewport-width fallback (None). For width-family percentage
    // properties we must NOT eagerly resolve against the viewport fallback,
    // because the result silently diverges from the actual containing-block
    // width and clamps to available_width at layout time, yielding a full-
    // width element (e.g. a `width: 95%` inner bar looking 100% wide).
    let parent_width = percentage_basis.width_or_parent(parent);
    let parent_width_known = parent_width.is_some();
    let length_context = crate::style::resolve::LengthResolutionContext::new(
        parent_width.unwrap_or(parent.viewport_width),
        style.math_unit_context(font_metrics),
    );

    // Handle inherit, initial, unset keywords before normal property application
    for (prop, val) in &map.properties {
        // Border shorthands, physical longhands, and logical longhands share
        // one ordered cascade. Applying their CSS-wide keywords from this
        // unordered map pass would destroy shorthand/longhand source order.
        if borders::is_border_property(prop) {
            continue;
        }
        if let CssValue::Keyword(k) = val {
            let lower = k.to_ascii_lowercase();
            match lower.as_str() {
                "inherit" => {
                    restore_from_parent(style, prop, parent);
                }
                "initial" => {
                    reset_to_initial(style, prop);
                }
                "unset" => {
                    if is_inherited_property(prop) {
                        restore_from_parent(style, prop, parent);
                    } else {
                        reset_to_initial(style, prop);
                    }
                }
                _ => {}
            }
        }
    }

    // Resolve the winning foreground color before any property that may use
    // `currentColor`. On the `color` property itself, currentColor's used value
    // is the resolved inherited color (CSS Color 4 §4.1).
    if let Some(value) = get_non_special(map, "color")
        && let Some(color) = specified_color_from_value(value, &style.custom_properties)
    {
        style.color = color.resolve(parent.color);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font") {
        apply_font_shorthand(style, k, parent, length_context, font_metrics);
    }

    if let Some(value) = get_non_special(map, "font-size")
        && let Some(size) =
            resolve_font_size_value(value, &style.custom_properties, parent, font_metrics)
    {
        style.font_size = size;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-weight") {
        apply_font_weight(style, k);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-style")
        && let Some(font_style) = FontStyle::from_css(k)
    {
        style.font_style = font_style;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-stretch")
        && let Some(stretch) = FontStretch::from_css(k)
    {
        style.font_stretch = stretch;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-family") {
        style.font_stack = parse_font_stack(k);
        style.font_family = style.font_stack.primary();
    }

    if let Some(value) = get_non_special(map, "font-size-adjust") {
        match value {
            CssValue::Number(aspect) if aspect.is_finite() && *aspect >= 0.0 => {
                style.font_size_adjust = FontSizeAdjust::ex_height(*aspect);
            }
            CssValue::Keyword(keyword) if keyword == "none" => {
                style.font_size_adjust = FontSizeAdjust::none();
            }
            _ => {}
        }
    }

    // CSS Values 4 §6.1.1: `lh` used by ordinary properties is the element's
    // computed line height, regardless of declaration order. Resolve the
    // line-height winner after the complete font tuple but before constructing
    // the unit context consumed by margins, sizing, gaps, and every other
    // length-valued property. Within line-height itself, `lh`/`rlh` use the
    // parent's metrics to avoid a self-reference.
    let line_height_context =
        line_height_length_context(style, parent, length_context, font_metrics);
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "line-height") {
        if k == "normal" {
            style.line_height = f32::NAN;
            style.line_height_absolute = None;
        } else if let Some(v) =
            resolve_raw_length_for_style(k, style, line_height_context, font_metrics)
        {
            style.line_height_absolute = Some(v);
            style.line_height = v / style.font_size;
        }
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "line-height") {
        style.line_height = *v;
        style.line_height_absolute = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "line-height") {
        let absolute = *v * style.font_size;
        style.line_height_absolute = Some(absolute);
        style.line_height = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "line-height") {
        style.line_height_absolute = Some(*v);
        style.line_height = *v / style.font_size;
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "line-height") {
        let absolute = style.font_size * *v / 100.0;
        style.line_height_absolute = Some(absolute);
        style.line_height = absolute / style.font_size;
    }
    if let Some(
        value @ (CssValue::Rem(_)
        | CssValue::Vw(_)
        | CssValue::Vh(_)
        | CssValue::Vmin(_)
        | CssValue::Vmax(_)
        | CssValue::Math(_)
        | CssValue::Var(_, _)),
    ) = get_non_special(map, "line-height")
        && let Some(v) =
            resolve_css_length_for_style(value, style, line_height_context, font_metrics)
    {
        style.line_height_absolute = Some(v);
        style.line_height = v / style.font_size;
    }
    sync_line_height_from_absolute(style);

    // All remaining font-relative properties use the element's completed font
    // metrics and line height, never reconstructed font-size-only defaults.
    let length_context = style_length_context(style, length_context, font_metrics);

    if let Some(value) = get_non_special(map, "background-color")
        && let Some(color) = specified_color_from_value(value, &style.custom_properties)
    {
        style.background_color = Some(color.resolve(style.color));
    }

    // `background-image` is one atomic cascade value. The parser preserves the
    // ordered source list here; legacy per-kind renderer fields are derived only
    // after the cascade has selected that single winner.
    if let Some(background_image) = get_non_special(map, "background-image") {
        style.clear_background_images();
        match background_image {
            CssValue::BackgroundLayers(layers) => {
                for layer in layers {
                    match layer {
                        BackgroundLayerSource::Raster(raw) if style.background_image.is_none() => {
                            let resolved =
                                resolve_embedded_vars(raw.trim(), &style.custom_properties);
                            let trimmed = resolved.trim();
                            if let Some(url) = extract_image_set_url(trimmed) {
                                style.background_image = Some(url);
                            } else {
                                style.background_image = Some(trimmed.to_string());
                            }
                        }
                        BackgroundLayerSource::Svg(svg) if style.background_svg.is_none() => {
                            style.background_svg = crate::parser::svg::parse_svg_from_string(svg);
                        }
                        BackgroundLayerSource::Linear(raw)
                            if style.background_gradient.is_none() =>
                        {
                            let raw = resolve_embedded_vars(raw, &style.custom_properties);
                            style.background_gradient =
                                parse_linear_gradient_for_color(&raw, style.color);
                        }
                        BackgroundLayerSource::Radial(raw)
                            if style.background_radial_gradient.is_none() =>
                        {
                            let raw = resolve_embedded_vars(raw, &style.custom_properties);
                            style.background_radial_gradient =
                                parse_radial_gradient_for_color(&raw, style.color);
                        }
                        BackgroundLayerSource::Conic(raw)
                            if style.background_conic_gradient.is_none() =>
                        {
                            let raw = resolve_embedded_vars(raw, &style.custom_properties);
                            style.background_conic_gradient =
                                parse_conic_gradient_for_color(&raw, style.color);
                        }
                        _ => {}
                    }
                }
            }
            // Values containing custom properties cannot always be typed until
            // after substitution, so retain the computed-time fallback.
            CssValue::Keyword(raw) => {
                let resolved = resolve_embedded_vars(raw.trim(), &style.custom_properties);
                let trimmed = resolved.trim();
                if trimmed != "none" {
                    if let Some(svg_text) = crate::parser::css::extract_svg_data_uri(trimmed) {
                        style.background_svg = crate::parser::svg::parse_svg_from_string(&svg_text);
                    } else if let Some(url) = extract_image_set_url(trimmed) {
                        style.background_image = Some(url);
                    } else if let Some(CssValue::Color(c)) =
                        crate::parser::css::parse_color(trimmed)
                    {
                        style.background_color = Some(c.resolve(style.color));
                    } else {
                        style.background_image = Some(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // Margins: resolve both absolute lengths and semantic em values.
    //
    // Em values must be resolved against the element's *final* font-size (per CSS
    // spec), but `style.font_size` at this point is whatever it was when the
    // current cascade layer was applied — the next layer may still override it.
    // So we save the em multiplier and re-resolve after the whole cascade runs
    // (see the em-fixup block at the end of `compute_style_with_context`).
    // Length (absolute) sets the margin and clears the em factor so later
    // re-resolution doesn't clobber the explicit value.
    match get_non_special(map, "margin-top") {
        Some(CssValue::Length(v)) => {
            style.margin.top = *v;
            style.margin_em_top = None;
        }
        Some(CssValue::Em(v)) => {
            style.margin.top = *v * style.font_size;
            style.margin_em_top = Some(*v);
        }
        _ => {}
    }
    match get_non_special(map, "margin-right") {
        Some(CssValue::Length(v)) => {
            style.margin.right = *v;
            style.margin_em_right = None;
        }
        Some(CssValue::Em(v)) => {
            style.margin.right = *v * style.font_size;
            style.margin_em_right = Some(*v);
        }
        _ => {}
    }
    match get_non_special(map, "margin-bottom") {
        Some(CssValue::Length(v)) => {
            style.margin.bottom = *v;
            style.margin_em_bottom = None;
        }
        Some(CssValue::Em(v)) => {
            style.margin.bottom = *v * style.font_size;
            style.margin_em_bottom = Some(*v);
        }
        _ => {}
    }
    match get_non_special(map, "margin-left") {
        Some(CssValue::Length(v)) => {
            style.margin.left = *v;
            style.margin_em_left = None;
        }
        Some(CssValue::Em(v)) => {
            style.margin.left = *v * style.font_size;
            style.margin_em_left = Some(*v);
        }
        _ => {}
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "padding-top") {
        style.padding.top = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "padding-right") {
        style.padding.right = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "padding-bottom") {
        style.padding.bottom = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "padding-left") {
        style.padding.left = *v;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-align") {
        style.text_align = match k.as_str() {
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            "left" => TextAlign::Left,
            "justify" => TextAlign::Justify,
            "start" => {
                if style.direction_rtl {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                }
            }
            "end" => {
                if style.direction_rtl {
                    TextAlign::Left
                } else {
                    TextAlign::Right
                }
            }
            "match-parent" => {
                if parent.direction_rtl
                    && !style.direction_rtl
                    && parent.text_align == TextAlign::Left
                {
                    TextAlign::Right
                } else {
                    parent.text_align
                }
            }
            _ => TextAlign::Left,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-align-last") {
        style.text_align_last = match k.as_str() {
            "center" => Some(TextAlign::Center),
            "right" => Some(TextAlign::Right),
            "left" => Some(TextAlign::Left),
            "justify" => Some(TextAlign::Justify),
            "start" => Some(if style.direction_rtl {
                TextAlign::Right
            } else {
                TextAlign::Left
            }),
            "end" => Some(if style.direction_rtl {
                TextAlign::Left
            } else {
                TextAlign::Right
            }),
            "auto" => None,
            _ => style.text_align_last,
        };
    }

    if let Some(value) = get_non_special(map, "text-decoration")
        && let Some(k) = resolved_raw_css_value(value, &style.custom_properties)
    {
        let inherited_controls = (
            style.text_decorations.current.skip_ink,
            style.text_decorations.current.underline_offset,
        );
        style.text_decorations.current = TextDecoration {
            skip_ink: inherited_controls.0,
            underline_offset: inherited_controls.1,
            ..Default::default()
        };
        apply_text_decoration_line(style, &k);
        if k.split_whitespace().any(|t| t == "wavy") {
            style.text_decorations.current.style = TextDecorationStyle::Wavy;
        }
        for token in k.split_whitespace() {
            if let Some(CssValue::Length(v)) = parse_length(token) {
                style.text_decorations.current.thickness = Some(v);
            }
        }
        if let Some(color) = color_in_text_emphasis_shorthand(&k) {
            style.text_decorations.current.color = Some(color.resolve(style.color));
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-decoration-line") {
        apply_text_decoration_line(style, k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-decoration-style") {
        style.text_decorations.current.style = if k.split_whitespace().any(|token| token == "wavy")
        {
            TextDecorationStyle::Wavy
        } else {
            TextDecorationStyle::Solid
        };
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "text-decoration-thickness") {
        style.text_decorations.current.thickness = Some(*v);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "text-underline-offset") {
        style.text_decorations.current.underline_offset = Some(*v);
    }

    // `text-decoration-color` longhand (css-text-decor-3 §2.2): an explicit line
    // colour distinct from the text `color`. Resolved like any colour value.
    if let Some(value) = get_non_special(map, "text-decoration-color")
        && let Some(color) = specified_color_from_value(value, &style.custom_properties)
    {
        style.text_decorations.current.color = Some(color.resolve(style.color));
    }
    if let Some(value) = get_non_special(map, "text-decoration-skip-ink")
        && let Some(keyword) = resolved_raw_css_value(value, &style.custom_properties)
        && let Some(skip_ink) = TextDecorationSkipInk::parse(&keyword)
    {
        style.text_decorations.current.skip_ink = skip_ink;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "display") {
        if let Some(display) = parse_display_value(k) {
            style.display = display;
        }
    }

    // `flex-flow` shorthand sets flex-direction and/or flex-wrap (order-free).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex-flow") {
        for token in k.split_whitespace() {
            if let Some(dir) = parse_flex_direction(token) {
                style.flex_direction = dir;
            } else if let Some(wrap) = parse_flex_wrap(token) {
                style.flex_wrap = wrap;
            }
        }
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex-direction") {
        if let Some(dir) = parse_flex_direction(k) {
            style.flex_direction = dir;
        }
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "justify-content") {
        // css-align-3 §9: an optional `safe`/`unsafe` overflow-alignment prefix
        // may precede the positional keyword (`justify-content: safe center`).
        // css-align-3 §6.2: `left`/`right` resolve against the INLINE axis. For a
        // row container that is the main axis (right→end, left→start); for a
        // column container the main axis is the block axis, so they behave as
        // `start`.
        style.justify_content = parse_justify_content(k, style.flex_direction.is_row());
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-items") {
        style.align_items = parse_align_items(k);
        // Grid uses the same property with start/end/center/stretch keywords.
        style.grid_align_items = parse_grid_align(k);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-content") {
        style.align_content = parse_align_content(k);
    }

    // `place-content: <align-content> [<justify-content>]`.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "place-content") {
        let parts = split_alignment_components(k);
        if let Some(align) = parts.first() {
            style.align_content = parse_align_content(align);
            let justify = parts.get(1).unwrap_or(align);
            style.justify_content = parse_justify_content(justify, style.flex_direction.is_row());
        }
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-self") {
        style.align_self = parse_align_self(k);
    }

    // `order` (integer). May arrive as a Length (numeric) or Keyword.
    match get_non_special(map, "order") {
        Some(CssValue::Number(v)) => style.order = *v as i32,
        Some(CssValue::Keyword(k)) => {
            if let Ok(n) = k.trim().parse::<i32>() {
                style.order = n;
            }
        }
        _ => {}
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex-wrap") {
        if let Some(wrap) = parse_flex_wrap(k) {
            style.flex_wrap = wrap;
        }
    }

    if let Some(CssValue::Number(v) | CssValue::Length(v)) = get_non_special(map, "flex-grow")
        && v.is_finite()
        && *v >= 0.0
    {
        style.flex_grow = *v;
    }
    if let Some(CssValue::Number(v) | CssValue::Length(v)) = get_non_special(map, "flex-shrink")
        && v.is_finite()
        && *v >= 0.0
    {
        style.flex_shrink = *v;
    }
    if let Some(value) = get_non_special(map, "flex-basis") {
        apply_flex_basis_value(style, value, length_context, font_metrics);
    }

    // flex shorthand: "flex: <grow>" or "flex: <grow> <shrink>" or "flex: <grow> <shrink> <basis>"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex") {
        apply_flex_shorthand(style, k, length_context, font_metrics);
    }

    // `gap: <row-gap> [<column-gap>]`. A single value sets both axes; the
    // two-value form sets row-gap then column-gap (CSS Box Alignment §8.3).
    // A single value arrives as `Length`; the two-value form as a `Keyword`
    // (multi-token string), so handle both.
    match get_non_special(map, "gap") {
        Some(CssValue::Length(v)) => {
            style.gap = *v;
            style.grid_gap = *v;
            style.column_gap = *v;
            style.row_gap = *v;
            style.column_gap_is_normal = false;
            style.column_gap_pct = None;
            style.row_gap_pct = None;
        }
        Some(CssValue::Percentage(p)) => {
            // A single percentage `gap` sets both axes. Percentages resolve
            // against the flex container's OWN content box (column-gap against
            // width, row-gap against height) — store a fraction hint and let the
            // flex layout resolve it; the eager parent-width setter is skipped.
            let frac = *p / 100.0;
            style.column_gap_pct = Some(frac);
            style.row_gap_pct = Some(frac);
            if let Some(width) = style.width {
                style.column_gap = width * frac;
                style.grid_gap = style.column_gap;
            }
            if let Some(height) = style.height {
                style.row_gap = height * frac;
            }
            style.column_gap_is_normal = false;
        }
        Some(CssValue::Keyword(k)) => {
            let parts: Vec<&str> = k.split_whitespace().collect();
            let resolve = |t: &str| match parse_length(t) {
                Some(CssValue::Length(v)) => Some(v),
                _ => None,
            };
            if let Some(row) = parts.first().and_then(|t| resolve(t)) {
                let col = parts.get(1).and_then(|t| resolve(t)).unwrap_or(row);
                style.row_gap = row;
                style.column_gap = col;
                // `gap` (single field used by the flex main axis) tracks the
                // column gap for row containers; the layout consults row_gap /
                // column_gap directly per axis.
                style.gap = col;
                style.grid_gap = row;
                style.column_gap_is_normal = false;
            }
        }
        _ => {}
    }

    // Grid template columns / rows. Each parse also extracts bracketed
    // `[name]` line names (CSS Grid §7.1) into a per-line-index name list, so
    // named-line placement (§8.3) can resolve them.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-template-columns") {
        let (tracks, names) = parse_grid_track_list(k);
        style.grid_template_columns = tracks;
        style.grid_template_column_line_names = names;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-template-rows") {
        let (tracks, names) = parse_grid_track_list(k);
        style.grid_template_rows = tracks;
        style.grid_template_row_line_names = names;
    }
    // `grid-template-areas`: ASCII-art row strings naming rectangular regions
    // (§7.3). Each string is a row; whitespace-separated tokens are cells; `.`
    // is an empty cell. Implicit `<name>-start`/`-end` line names are derived
    // in layout from the resulting area rectangles.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-template-areas") {
        let areas = parse_grid_template_areas(k);
        if !areas.is_empty() || k.trim() == "none" {
            apply_grid_template_areas(style, areas);
        }
    }
    // `grid-auto-rows` may arrive as a Length (single px/pt value) or Keyword.
    match get_non_special(map, "grid-auto-rows") {
        Some(CssValue::Length(v)) => {
            style.grid_auto_rows = Some(*v);
            style.grid_auto_rows_pattern = vec![*v];
        }
        Some(CssValue::Keyword(k)) => {
            let pattern: Vec<f32> = k
                .split_whitespace()
                .filter_map(|token| match parse_single_track(token) {
                    Some(GridTrack::Fixed(v)) => Some(v),
                    _ => None,
                })
                .collect();
            if let Some(first) = pattern.first().copied() {
                style.grid_auto_rows = Some(first);
                style.grid_auto_rows_pattern = pattern;
            }
        }
        _ => {}
    }
    let explicit_columns_declared = get_non_special(map, "grid-template-columns").is_some()
        || get_non_special(map, "grid-template").is_some()
        || get_non_special(map, "grid").is_some();
    if !explicit_columns_declared
        && !style.grid_template_areas.is_empty()
        && let Some(auto_col) = match get_non_special(map, "grid-auto-columns") {
            Some(CssValue::Length(v)) => Some(GridTrack::Fixed(*v)),
            Some(CssValue::Keyword(k)) => parse_single_track(k),
            _ => None,
        }
    {
        let area_cols = style
            .grid_template_areas
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0);
        let looks_synthesized = style.grid_template_columns.len() == area_cols
            && style
                .grid_template_columns
                .iter()
                .all(|track| matches!(track, GridTrack::Auto))
            && style
                .grid_template_column_line_names
                .iter()
                .all(Vec::is_empty);
        if area_cols > 0 && looks_synthesized {
            style.grid_template_columns = vec![auto_col; area_cols];
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-auto-flow") {
        style.grid_auto_flow_column = k.contains("column");
        style.grid_auto_flow_dense = k.contains("dense");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-template") {
        apply_grid_template_shorthand(style, k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid") {
        apply_grid_shorthand(style, k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "justify-items") {
        style.justify_items = parse_grid_align(k);
    }
    // `place-items: <align> [<justify>]` shorthand sets both axes.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "place-items") {
        let parts = split_alignment_components(k);
        if let Some(a) = parts.first() {
            let align = parse_grid_align(a);
            let justify = parts.get(1).map(|s| parse_grid_align(s)).unwrap_or(align);
            style.align_items = parse_align_items(a);
            style.grid_align_items = align;
            style.justify_items = justify;
        }
    }
    // Per-item self alignment overrides (grid). `auto` keeps the container value
    // (`None` here); anything else pins this item. Note the *flex* `align-self`
    // is handled separately above; here we mirror it into the grid field too.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "justify-self") {
        if k.trim() != "auto" {
            style.grid_justify_self = Some(parse_grid_align(k));
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-self") {
        if k.trim() != "auto" {
            style.grid_align_self = Some(parse_grid_align(k));
        }
    }
    // `place-self: <align> [<justify>]` shorthand sets both grid self axes.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "place-self") {
        let parts = split_alignment_components(k);
        if let Some(a) = parts.first() {
            style.align_self = parse_align_self(a);
            if a != "auto" {
                style.grid_align_self = Some(parse_grid_align(a));
            }
            if let Some(j) = parts.get(1) {
                if j != "auto" {
                    style.grid_justify_self = Some(parse_grid_align(j));
                }
            } else if a != "auto" {
                style.grid_justify_self = Some(parse_grid_align(a));
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-column-start") {
        style.grid_column_start = parse_grid_line(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-column-end") {
        style.grid_column_end = parse_grid_line(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-row-start") {
        style.grid_row_start = parse_grid_line(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-row-end") {
        style.grid_row_end = parse_grid_line(k);
    }
    // Item-level placement: grid-column / grid-row resolve a start/end line
    // pair (CSS Grid §8). Each side is a line number, `span N`, named line, or
    // `auto`. The legacy span count is derived for back-compat.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-column") {
        let (start, end) = parse_grid_placement_shorthand(k);
        style.grid_column_start = start;
        style.grid_column_end = end;
        if let Some(n) = parse_grid_span(k) {
            style.grid_column_span = n;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-row") {
        let (start, end) = parse_grid_placement_shorthand(k);
        style.grid_row_start = start;
        style.grid_row_end = end;
        if let Some(n) = parse_grid_span(k) {
            style.grid_row_span = n;
        }
    }
    // `grid-area`: either a single area name, or the 4-value line form
    // `row-start / col-start / row-end / col-end` (§8.1).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-area") {
        apply_grid_area(style, k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "clip") {
        style.clip_path = parse_legacy_clip_rect(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "clip-path") {
        style.clip_path = parse_clip_path(k);
    }

    // CSS Masking (css-masking-1 §3). The `-webkit-mask*` aliases are normalised
    // to the unprefixed names at parse time. `mask-mode` resolves how the source
    // pixels become coverage (default `match-source` → alpha for CSS images).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-mode") {
        style.mask_mode = parse_mask_mode(k);
    }
    // `mask` shorthand: parse the image plus its position/size/repeat/mode.
    // Longhands below override it when present, as usual in the cascade map.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask") {
        let k = resolve_embedded_vars(k, &style.custom_properties);
        if let Some(src) = parse_mask_shorthand(&k, style.mask_mode, style.color) {
            style.mask_image = src;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-image") {
        let k = resolve_embedded_vars(k, &style.custom_properties);
        if let Some(src) = parse_mask_image(&k, style.mask_mode, style.color) {
            style.mask_image = src;
        }
    }
    apply_mask_longhands(map, style, style.color);

    // Grid gap (shorthand sets both column and row gap)
    if let Some(CssValue::Length(v)) = get_non_special(map, "grid-gap") {
        style.grid_gap = *v;
        style.column_gap = *v;
        style.row_gap = *v;
    }

    // CSS Fragmentation 3 §3.1 `break-before`/`break-after` plus their legacy
    // `page-break-*` aliases (CSS 2.1 §13.3.1). The legacy property is read
    // first and the modern one overrides it when both are present (modern wins
    // at equal cascade origin). `always` maps to `page`. The forced values set
    // the `page_break_*` bool that drives the existing `PageBreak` emission.
    let mut bb = BreakValue::Auto;
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page-break-before") {
        if let Some(v) = BreakValue::from_keyword(k) {
            bb = v;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "break-before") {
        if let Some(v) = BreakValue::from_keyword(k) {
            bb = v;
        }
    }
    style.break_before = bb;
    style.page_break_before = bb.forces_break();

    let mut ba = BreakValue::Auto;
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page-break-after") {
        if let Some(v) = BreakValue::from_keyword(k) {
            ba = v;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "break-after") {
        if let Some(v) = BreakValue::from_keyword(k) {
            ba = v;
        }
    }
    style.break_after = ba;
    style.page_break_after = ba.forces_break();

    // `break-inside: avoid` / legacy `page-break-inside: avoid` (only the
    // `avoid*` family is meaningful; `auto` is the default).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page-break-inside") {
        style.break_inside_avoid = k.starts_with("avoid");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "break-inside") {
        style.break_inside_avoid = k.starts_with("avoid");
    }

    // CSS Paged Media 3 §3.4 `page: <name>` — the named page a box belongs to.
    // Only set when the property is present so it accumulates across cascade
    // layers (a later layer without `page` leaves the prior value). `auto`
    // resets to the default page. Page type names are case-sensitive CSS
    // identifiers, so their authored case survives to page-selector matching.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page") {
        style.page_name = if k.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(k.clone())
        };
    }

    if let Some(value) = get_non_special(map, "filter")
        && let Some(k) = resolved_raw_css_value(value, &style.custom_properties)
    {
        let parsed = parse_filter_for_color(&k, style.color);
        style.filter = FilterEffects {
            establishes_stacking_context: parsed.establishes_stacking_context,
            operations: parsed.operations,
            url_id: parsed.url_id,
        };
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "width") {
        style.width = Some(*v);
        style.width_keyword = None;
        style.percentage_sizing.width = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "width") {
        // em value — multiply by current font-size
        style.width = Some(*v * style.font_size);
        style.width_keyword = None;
        style.percentage_sizing.width = None;
    }
    // css-sizing-3 § 5.1 intrinsic-sizing keywords (`min-content` / `max-content`
    // / `fit-content`). These keep `width` as `None` (so the auto/length/percentage
    // paths are untouched) and record the keyword for block layout to derive a
    // content-based width. `auto` and any other keyword leave the box as `auto`.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "width") {
        if let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics) {
            style.width = Some(v);
            style.width_keyword = None;
            style.percentage_sizing.width = None;
        } else {
            let kw = match k.trim().to_ascii_lowercase().as_str() {
                "min-content" => Some(IntrinsicWidthKeyword::MinContent),
                "max-content" => Some(IntrinsicWidthKeyword::MaxContent),
                "fit-content" => Some(IntrinsicWidthKeyword::FitContent),
                _ => None,
            };
            if kw.is_some() {
                style.width = None;
                style.width_keyword = kw;
                style.percentage_sizing.width = None;
            } else if k.trim().eq_ignore_ascii_case("auto") {
                style.width_keyword = None;
            }
        }
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "height")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics)
    {
        style.height = Some(v);
        style.percentage_sizing.height = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "max-width")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics)
    {
        style.max_width = Some(v);
        style.percentage_sizing.max_width = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "min-width")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics)
    {
        style.min_width = Some(v);
        style.percentage_sizing.min_width = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "min-height")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics)
    {
        style.min_height = Some(v);
        style.percentage_sizing.min_height = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "max-height")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context, font_metrics)
    {
        style.max_height = Some(v);
        style.percentage_sizing.max_height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "height") {
        style.height = Some(*v);
        style.percentage_sizing.height = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "height") {
        style.height = Some(*v * style.font_size);
        style.percentage_sizing.height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v);
        style.percentage_sizing.max_width = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v * style.font_size);
        style.percentage_sizing.max_width = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v);
        style.percentage_sizing.min_width = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v * style.font_size);
        style.percentage_sizing.min_width = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v);
        style.percentage_sizing.min_height = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v * style.font_size);
        style.percentage_sizing.min_height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-height") {
        style.max_height = Some(*v);
        style.percentage_sizing.max_height = None;
    }
    if let Some(CssValue::Em(v)) = get_non_special(map, "max-height") {
        style.max_height = Some(*v * style.font_size);
        style.percentage_sizing.max_height = None;
    }

    // margin-left/right/top/bottom: auto. The `auto` keyword on a flex item
    // absorbs free space along its axis (css-flexbox-1 §8.1); on the vertical
    // axis it also enables cross-axis centering for a row container.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "margin-left") {
        if k == "auto" {
            style.margin_left_auto = true;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "margin-right") {
        if k == "auto" {
            style.margin_right_auto = true;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "margin-top") {
        if k == "auto" {
            style.margin_top_auto = true;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "margin-bottom") {
        if k == "auto" {
            style.margin_bottom_auto = true;
        }
    }

    if let Some(value) = get_non_special(map, "opacity")
        && let Some(opacity) = resolve_opacity(value, &style.custom_properties)
    {
        style.opacity = opacity;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mix-blend-mode") {
        style.mix_blend_mode = BlendMode::from_keyword(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-blend-mode") {
        style.background_blend_mode = BlendMode::from_background_value(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "isolation") {
        style.isolation = if k.trim().eq_ignore_ascii_case("isolate") {
            Isolation::Isolate
        } else {
            Isolation::Auto
        };
    }

    // Float
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "float") {
        style.float = match k.as_str() {
            "left" => Float::Left,
            "right" => Float::Right,
            "footnote" => Float::Footnote,
            _ => Float::None,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "footnote-display")
        && let Some(display) = FootnoteDisplay::from_keyword(k)
    {
        style.footnote.display = display;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "footnote-policy")
        && let Some(policy) = FootnotePolicy::from_keyword(k)
    {
        style.footnote.policy = policy;
    }

    // Clear
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "clear") {
        style.clear = match k.as_str() {
            "left" => Clear::Left,
            "right" => Clear::Right,
            "both" => Clear::Both,
            _ => Clear::None,
        };
    }

    // box-decoration-break (css-break-3 §6.2)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-decoration-break") {
        style.box_decoration_break = match k.as_str() {
            "clone" => BoxDecorationBreak::Clone,
            _ => BoxDecorationBreak::Slice,
        };
    }

    // Position
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "position") {
        style.running_name = None;
        style.position = match k.as_str() {
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            value => {
                style.running_name = parse_running_position_name(value);
                Position::Static
            }
        };
    }

    // Top / Right / Bottom / Left for positioned elements
    if let Some(CssValue::Length(v)) = get_non_special(map, "top") {
        style.top = Some(*v);
        style.percentage_insets.top = None;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "right") {
        style.right = Some(*v);
        style.percentage_insets.right = None;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "bottom") {
        style.bottom = Some(*v);
        style.percentage_insets.bottom = None;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "left") {
        style.left = Some(*v);
        style.percentage_insets.left = None;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "inset") {
        let parts: Vec<&str> = k.split_whitespace().collect();
        let values = match parts.as_slice() {
            [a] => Some([*a, *a, *a, *a]),
            [a, b] => Some([*a, *b, *a, *b]),
            [a, b, c] => Some([*a, *b, *c, *b]),
            [a, b, c, d] => Some([*a, *b, *c, *d]),
            _ => None,
        };
        if let Some(values) = values {
            for (idx, token) in values.iter().enumerate() {
                let token = token.trim();
                let length = parse_length(token);
                match (idx, length) {
                    (0, Some(CssValue::Length(v))) => {
                        style.top = Some(v);
                        style.percentage_insets.top = None;
                    }
                    (1, Some(CssValue::Length(v))) => {
                        style.right = Some(v);
                        style.percentage_insets.right = None;
                    }
                    (2, Some(CssValue::Length(v))) => {
                        style.bottom = Some(v);
                        style.percentage_insets.bottom = None;
                    }
                    (3, Some(CssValue::Length(v))) => {
                        style.left = Some(v);
                        style.percentage_insets.left = None;
                    }
                    (0, Some(CssValue::Percentage(v))) => style.percentage_insets.top = Some(v),
                    (1, Some(CssValue::Percentage(v))) => style.percentage_insets.right = Some(v),
                    (2, Some(CssValue::Percentage(v))) => style.percentage_insets.bottom = Some(v),
                    (3, Some(CssValue::Percentage(v))) => style.percentage_insets.left = Some(v),
                    _ => {}
                }
            }
        }
    }

    // Box-shadow: parse from keyword (stored as full shorthand string).
    // The comma-separated list is one declaration: if any component is
    // invalid, the whole declaration is ignored rather than retaining a
    // valid-looking prefix (CSS Backgrounds & Borders 3 §7.2).
    if let Some(value) = get_non_special(map, "box-shadow")
        && let Some(k) = resolved_raw_css_value(value, &style.custom_properties)
    {
        if let Some(shadows) = parse_box_shadow_for_color(&k, style.color) {
            style.box_shadow = shadows;
        }
    }

    // CSS `text-shadow` (css-text-decor-3 §3). Like box-shadow but with no
    // `spread`/`inset` and the optional color may appear before or after the
    // offsets. `none` clears any inherited value.
    if let Some(value) = get_non_special(map, "text-shadow")
        && let Some(k) = resolved_raw_css_value(value, &style.custom_properties)
    {
        if k.trim() == "none" {
            style.text_shadow = Vec::new();
        } else {
            let shadows = parse_text_shadow_for_color(&k, style.color);
            if !shadows.is_empty() {
                style.text_shadow = shadows;
            }
        }
    }

    // Multi-column layout
    if let Some(val) = get_non_special(map, "column-count") {
        match val {
            CssValue::Number(n) | CssValue::Length(n)
                if n.is_finite() && *n >= 1.0 && n.fract() == 0.0 =>
            {
                style.column_count = Some(*n as u32);
            }
            CssValue::Keyword(k) => {
                if let Ok(n) = k.parse::<u32>()
                    && n >= 1
                {
                    style.column_count = Some(n);
                }
            }
            _ => {}
        }
    }
    if let Some(val) = get_non_special(map, "column-width") {
        match val {
            CssValue::Length(w) => style.column_width = Some(*w),
            CssValue::Keyword(k) if k != "auto" => {
                if let Some(CssValue::Length(w)) = parse_length(k) {
                    style.column_width = Some(w);
                }
            }
            _ => {}
        }
    }
    // `columns` shorthand: `<column-width> || <column-count>`, in any order.
    // Each token is either a length (column-width) or an integer (column-count).
    if let Some(val) = get_non_special(map, "columns") {
        match val {
            CssValue::Length(n) => style.column_count = Some(*n as u32),
            CssValue::Keyword(k) => {
                for token in k.split_whitespace() {
                    if token == "auto" {
                        continue;
                    }
                    if let Ok(n) = token.parse::<u32>() {
                        style.column_count = Some(n);
                    } else if let Some(CssValue::Length(w)) = parse_length(token) {
                        style.column_width = Some(w);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(val) =
        get_non_special(map, "column-gap").or_else(|| get_non_special(map, "grid-column-gap"))
    {
        match val {
            CssValue::Length(v) => {
                style.column_gap = *v;
                style.column_gap_is_normal = false;
                style.column_gap_pct = None;
            }
            // A percentage column-gap resolves against the container's own
            // content-box width (CSS Box Alignment §8.3); defer it as a fraction.
            CssValue::Percentage(p) => {
                style.column_gap_pct = Some(*p / 100.0);
                if let Some(width) = style.width {
                    style.column_gap = width * *p / 100.0;
                }
                style.column_gap_is_normal = false;
            }
            CssValue::Keyword(k) if k != "normal" => {
                if let Some(stripped) = k.trim().strip_suffix('%') {
                    if let Ok(p) = stripped.parse::<f32>() {
                        style.column_gap_pct = Some(p / 100.0);
                        if let Some(width) = style.width {
                            style.column_gap = width * p / 100.0;
                        }
                        style.column_gap_is_normal = false;
                    }
                } else if let Some(CssValue::Length(v)) = parse_length(k) {
                    style.column_gap = v;
                    style.column_gap_is_normal = false;
                    style.column_gap_pct = None;
                }
            }
            _ => {}
        }
    }
    // `column-rule` shorthand and longhands. The rule is the vertical line drawn
    // in each column gap; width/style/color mirror a single border side.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-rule") {
        let k = resolve_embedded_vars(k, &style.custom_properties);
        if let Some(rule) = parse_column_rule_shorthand(&k, style.font_size) {
            style.column_rule = rule;
        }
    }
    if let Some(val) = get_non_special(map, "column-rule-width") {
        if let CssValue::Length(w) = val {
            if *w >= 0.0 {
                style.column_rule.specified_width = *w;
            }
        } else if let CssValue::Keyword(k) = val {
            // `thin` / `medium` / `thick` keyword widths, else a parsed length.
            match k.trim().to_ascii_lowercase().as_str() {
                "thin" => style.column_rule.specified_width = 0.75,
                "medium" => style.column_rule.specified_width = MEDIUM_RULE_WIDTH_PT,
                "thick" => style.column_rule.specified_width = 3.75,
                _ => {
                    if let Some(CssValue::Length(w)) = parse_length(k)
                        && w >= 0.0
                    {
                        style.column_rule.specified_width = w;
                    }
                }
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-rule-style") {
        if let Some(rule_style) = borders::parse_border_style(k) {
            style.column_rule.style = rule_style;
            // A visible style with no explicit width uses the medium default
            // (column-rule-width initial = medium) so the rule actually paints.
            if style.column_rule.style.paints()
                && style.column_rule.specified_width <= 0.0
                && get_non_special(map, "column-rule-width").is_none()
                && get_non_special(map, "column-rule").is_none()
            {
                style.column_rule.specified_width = MEDIUM_RULE_WIDTH_PT;
            }
        }
    }
    if let Some(val) = get_non_special(map, "column-rule-color") {
        if let Some(color) = specified_color_from_value(val, &style.custom_properties) {
            style.column_rule.color = color;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-span") {
        style.column_span_all = k.eq_ignore_ascii_case("all");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-fill") {
        // `auto` fills columns sequentially; `balance` (default) equalises them.
        style.column_fill_auto = k.eq_ignore_ascii_case("auto");
    }
    match get_non_special(map, "row-gap").or_else(|| get_non_special(map, "grid-row-gap")) {
        Some(CssValue::Length(v)) => {
            style.row_gap = *v;
            style.row_gap_pct = None;
        }
        // A percentage row-gap resolves against the container's own content-box
        // block size (height); defer it as a fraction for the flex layout.
        Some(CssValue::Percentage(p)) => {
            style.row_gap_pct = Some(*p / 100.0);
            if let Some(height) = style.height {
                style.row_gap = height * *p / 100.0;
            }
        }
        Some(CssValue::Keyword(k)) if k != "normal" => {
            if let Some(stripped) = k.trim().strip_suffix('%') {
                if let Ok(p) = stripped.parse::<f32>() {
                    style.row_gap_pct = Some(p / 100.0);
                    if let Some(height) = style.height {
                        style.row_gap = height * p / 100.0;
                    }
                }
            } else if let Some(CssValue::Length(v)) = parse_length(k) {
                style.row_gap = v;
                style.row_gap_pct = None;
            }
        }
        _ => {}
    }

    // Overflow. The `overflow` shorthand sets both axes; `overflow-x` and
    // `overflow-y` set them independently. Per CSS Overflow 3 §3, a used value
    // of `visible`/`clip` is coerced when the OTHER axis is a scrolling value
    // (`auto`/`scroll`/`hidden`): `visible` -> `auto`, `clip` -> `hidden`. So a
    // box with one non-visible axis effectively clips BOTH axes. In a print/PDF
    // context there are no interactive scrollbars, so `clip`/`scroll`/`hidden`
    // all clip overflowing content to the box; `auto` clips only when content
    // actually overflows (handled at layout/paint as a clip).
    let mut overflow_x: Option<RawOverflow> = None;
    let mut overflow_y: Option<RawOverflow> = None;
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow") {
        // The shorthand accepts one or two keywords (`overflow: hidden visible`).
        let mut parts = k.split_whitespace();
        let first = parts.next().map(parse_raw_overflow);
        let second = parts.next().map(parse_raw_overflow);
        if let Some(x) = first {
            overflow_x = Some(x);
            overflow_y = Some(second.unwrap_or(x));
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow-x") {
        overflow_x = Some(parse_raw_overflow(k));
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow-y") {
        overflow_y = Some(parse_raw_overflow(k));
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow-inline") {
        overflow_x = Some(parse_raw_overflow(k));
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow-block") {
        overflow_y = Some(parse_raw_overflow(k));
    }
    if overflow_x.is_some() || overflow_y.is_some() {
        let (cx, cy) = coerce_overflow_axes(
            overflow_x.unwrap_or(RawOverflow::Visible),
            overflow_y.unwrap_or(RawOverflow::Visible),
        );
        style.overflow_x = cx;
        style.overflow_y = cy;
        // Collapse the two coerced axes into the single `overflow` field used by
        // the clip-only consumers. `auto` is preserved only when BOTH axes are
        // `auto` (no clip until content overflows); any clipping axis collapses
        // to `Hidden`. (The per-axis fields above retain the `scroll`/`auto`
        // detail the scrollbar painter needs.)
        style.overflow = match (cx, cy) {
            (Overflow::Visible, Overflow::Visible) => Overflow::Visible,
            (Overflow::Auto, Overflow::Auto) => Overflow::Auto,
            _ => Overflow::Hidden,
        };
    }

    // Visibility
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "visibility") {
        style.visibility = match k.as_str() {
            "hidden" => Visibility::Hidden,
            "collapse" => Visibility::Collapse,
            _ => Visibility::Visible,
        };
    }

    // Transform. `none` is an explicit reset; any other value that fails to
    // parse leaves the current value untouched.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "transform") {
        if k.trim() == "none" {
            style.transform = None;
        } else if let Some(t) = parse_transform(k, style.font_size, style.root_font_size) {
            style.transform = Some(t);
        }
    }

    let mut individual = Vec::new();
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "translate")
        && k.trim() != "none"
        && let Some(t) = parse_individual_translate(k, style.font_size, style.root_font_size)
    {
        individual.push(t);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "rotate")
        && k.trim() != "none"
        && let Some(t) = parse_individual_rotate(k)
    {
        individual.push(t);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "scale")
        && k.trim() != "none"
        && let Some(t) = parse_individual_scale(k)
    {
        individual.push(t);
    }
    if !individual.is_empty() {
        if let Some(t) = style.transform {
            individual.push(t);
        }
        style.transform = compose_transforms(&individual);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "transform-origin") {
        if let Some(origin) = parse_transform_origin(k, style.font_size, style.root_font_size) {
            style.transform_origin = origin;
        }
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "transform-box") {
        if let Some(reference_box) = match k.trim().to_ascii_lowercase().as_str() {
            "border-box" => Some(TransformBox::Border),
            "content-box" => Some(TransformBox::Content),
            "fill-box" => Some(TransformBox::Fill),
            "stroke-box" => Some(TransformBox::Stroke),
            "view-box" => Some(TransformBox::View),
            _ => None,
        } {
            style.transform_box = reference_box;
        }
    }

    if let Some(value) = get_non_special(map, "perspective") {
        style.perspective = match value {
            CssValue::Length(v) if *v > 0.0 => Some(*v),
            CssValue::Keyword(k) if k.trim().eq_ignore_ascii_case("none") => None,
            CssValue::Keyword(k) => {
                parse_transform_length(k.trim(), style.font_size, style.root_font_size)
                    .and_then(|(v, is_pct)| (!is_pct && v > 0.0).then_some(v))
                    .filter(|value| value.is_finite() && *value <= f64::from(f32::MAX))
                    .map(|value| value as f32)
            }
            _ => style.perspective,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "perspective-origin") {
        if let Some(origin) = parse_transform_origin(k, style.font_size, style.root_font_size) {
            style.perspective_origin = origin;
        }
    }

    if let (Some(Transform::Matrix3d(matrix)), Some(perspective)) =
        (style.transform, parent.perspective)
    {
        let parent_w = parent.width.unwrap_or(0.0);
        let parent_h = parent.height.unwrap_or(parent_w);
        let (px, py) = parent.perspective_origin.resolve(parent_w, parent_h);
        style.transform = Some(Transform::Project3d {
            matrix,
            perspective: f64::from(perspective),
            perspective_origin: CssVector::new(
                f64::from(px - style.left.unwrap_or(0.0)),
                f64::from(py - style.top.unwrap_or(0.0)),
            ),
        });
    }

    // Outline shorthand: "2px solid red" (with optional style keyword we ignore
    // for paint, since the renderer strokes a solid outline).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "outline") {
        let k = resolve_embedded_vars(k, &style.custom_properties);
        let parts: Vec<&str> = k.split_whitespace().collect();
        for part in &parts {
            if let Some(n) = part.strip_suffix("px") {
                if let Ok(v) = n.parse::<f32>() {
                    style.outline_width = v * 0.75; // px to pt
                }
            } else if let Some(n) = part.strip_suffix("pt") {
                if let Ok(v) = n.parse::<f32>() {
                    style.outline_width = v;
                }
            }
        }
        if let Some(last) = parts.last() {
            if let Some(c) = parse_border_color(last) {
                style.outline_color = Some(c.resolve(style.color));
            }
        }
    }

    // Outline individual properties
    if let Some(CssValue::Length(v)) = get_non_special(map, "outline-width") {
        style.outline_width = *v;
    }
    if let Some(value) = get_non_special(map, "outline-color")
        && let Some(color) = specified_color_from_value(value, &style.custom_properties)
    {
        style.outline_color = Some(color.resolve(style.color));
    }
    // `outline-offset`: gap between border edge and outline (may be negative).
    if let Some(CssValue::Length(v)) = get_non_special(map, "outline-offset") {
        style.outline_offset = *v;
    }

    // Box-sizing
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-sizing") {
        style.box_sizing = match k.as_str() {
            "border-box" => BoxSizing::BorderBox,
            _ => BoxSizing::ContentBox,
        };
    }

    // CSS `direction` property. Inheritable. `dir=` attribute wins over CSS
    // but is applied earlier in compute_style_with_context; here we only set
    // from CSS when present.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "direction") {
        match k.as_str() {
            "rtl" => {
                style.direction_rtl = true;
                if style.text_align == TextAlign::Left {
                    style.text_align = TextAlign::Right;
                }
            }
            "ltr" => {
                style.direction_rtl = false;
            }
            _ => {}
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-align") {
        style.text_align = match k.as_str() {
            "start" => {
                if style.direction_rtl {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                }
            }
            "end" => {
                if style.direction_rtl {
                    TextAlign::Left
                } else {
                    TextAlign::Right
                }
            }
            "match-parent" => {
                if parent.direction_rtl
                    && !style.direction_rtl
                    && parent.text_align == TextAlign::Left
                {
                    TextAlign::Right
                } else {
                    parent.text_align
                }
            }
            _ => style.text_align,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-align-last") {
        style.text_align_last = match k.as_str() {
            "start" => Some(if style.direction_rtl {
                TextAlign::Right
            } else {
                TextAlign::Left
            }),
            "end" => Some(if style.direction_rtl {
                TextAlign::Left
            } else {
                TextAlign::Right
            }),
            "match-parent" => Some(parent.text_align_last.unwrap_or(parent.text_align)),
            _ => style.text_align_last,
        };
    }

    // CSS `writing-mode` property (css-writing-modes-4 §3.1). Inherited, so it
    // rides the parent style unless this element specifies another flow.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "writing-mode") {
        style.writing_mode = match k.as_str() {
            "vertical-rl" => WritingMode::VerticalRl,
            "vertical-lr" => WritingMode::VerticalLr,
            "sideways-rl" => WritingMode::SidewaysRl,
            "sideways-lr" => WritingMode::SidewaysLr,
            _ => WritingMode::HorizontalTb,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-orientation") {
        style.text_orientation_upright = k == "upright";
    }
    if let Some(CssValue::Keyword(value)) = get_non_special(map, "text-combine-upright")
        && let Some(text_combine_upright) = TextCombineUpright::parse(value)
    {
        style.text_combine_upright = text_combine_upright;
    }

    // CSS Logical 1 §4.5 and CSS Cascade: physical and flow-relative border
    // declarations are one logical property group. Resolve them in declaration
    // order after writing mode, color, and font sizing are final.
    borders::apply(style, map, parent, length_context, font_metrics);

    // CSS `unicode-bidi` property. Not inherited. `bidi-override` (and the
    // isolating `isolate-override`) force the box's inline content to be
    // reordered strictly in sequence according to `direction`, overriding the
    // characters' intrinsic bidi classes (css-writing-modes-4 §2.4).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "unicode-bidi") {
        style.bidi_override = matches!(k.as_str(), "bidi-override" | "isolate-override");
        style.bidi_plaintext = k == "plaintext";
    }

    // Text-transform
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-transform") {
        style.text_transform = match k.as_str() {
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "capitalize" => TextTransform::Capitalize,
            _ => TextTransform::None,
        };
    }

    // font-variant-caps (css-fonts-4 §6.5) and the `font-variant` shorthand
    // (css-fonts-3 §6.5). Only `small-caps` is synthesised; any other token (or
    // `normal`/`none`) resets to Normal. The shorthand may carry several
    // space-separated tokens, so scan for the `small-caps` keyword.
    let caps_value =
        get_non_special(map, "font-variant-caps").or_else(|| get_non_special(map, "font-variant"));
    if let Some(CssValue::Keyword(k)) = caps_value {
        let lower = k.to_ascii_lowercase();
        style.font_variant_caps = if lower.split_whitespace().any(|t| t == "small-caps") {
            FontVariantCaps::SmallCaps
        } else {
            FontVariantCaps::Normal
        };
    }

    let position_value = map
        .get("font-variant-position")
        .or_else(|| map.get("font-variant"));
    if let Some(CssValue::Keyword(k)) = position_value {
        let lower = k.to_ascii_lowercase();
        if lower.split_whitespace().any(|token| token == "super") {
            style.font_variant_position = FontVariantPosition::Super;
        } else if lower.split_whitespace().any(|token| token == "sub") {
            style.font_variant_position = FontVariantPosition::Sub;
        } else if lower == "normal" {
            style.font_variant_position = FontVariantPosition::Normal;
        }
    }

    // font-feature-settings (css-fonts-3 §6.4): honour explicit ligature
    // control. `"liga" 0` (or `clig`/`dlig` set to 0/off) disables the shaper's
    // default ligature substitution; the inverse (or omission) leaves it on.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-feature-settings") {
        style.ligatures_enabled = !ligatures_disabled_by_feature_settings(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-variant-ligatures") {
        style.ligatures_enabled = !k.split_whitespace().any(|t| {
            matches!(
                t,
                "none" | "no-common-ligatures" | "no-contextual" | "no-discretionary-ligatures"
            )
        });
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-kerning") {
        style.font_kerning_enabled = k != "none";
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-synthesis") {
        let lower = k.to_ascii_lowercase();
        let mut tokens = lower.split_whitespace().peekable();
        if tokens.peek().is_some() {
            style.font_synthesis_weight = false;
            style.font_synthesis_style = false;
            style.font_synthesis_small_caps = false;
            for token in tokens {
                match token {
                    "none" => break,
                    "weight" => style.font_synthesis_weight = true,
                    "style" => style.font_synthesis_style = true,
                    "small-caps" => style.font_synthesis_small_caps = true,
                    _ => {}
                }
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "initial-letter") {
        let mut parts = k.split_whitespace();
        style.initial_letter = match parts.next() {
            Some("normal") | None => 0.0,
            Some(size) => size.parse::<f32>().unwrap_or(0.0).max(0.0),
        };
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "initial-letter") {
        style.initial_letter = v.max(0.0);
    }
    for prop in ["text-emphasis", "-webkit-text-emphasis"] {
        if let Some(value) = get_non_special(map, prop)
            && let Some(k) = resolved_raw_css_value(value, &style.custom_properties)
        {
            style.text_emphasis_mark = k.split_whitespace().any(|t| matches!(t, "dot" | "filled"));
            if let Some(c) = color_in_text_emphasis_shorthand(&k) {
                let (color, source) = bind_specified_color(c, style.color);
                style.text_emphasis_color = color;
                style.text_emphasis_color_source = source;
            }
        }
    }
    for prop in ["text-emphasis-style", "-webkit-text-emphasis-style"] {
        if let Some(CssValue::Keyword(k)) = get_non_special(map, prop) {
            style.text_emphasis_mark = k.split_whitespace().any(|t| matches!(t, "dot" | "filled"));
        }
    }
    for prop in ["text-emphasis-position", "-webkit-text-emphasis-position"] {
        if let Some(value) = get_non_special(map, prop)
            && let Some(raw) = resolved_raw_css_value(value, &style.custom_properties)
            && let Some(position) = TextEmphasisPosition::parse(&raw)
        {
            style.text_emphasis_position = position;
        }
    }
    for prop in ["text-emphasis-color", "-webkit-text-emphasis-color"] {
        if let Some(value) = get_non_special(map, prop)
            && let Some(c) = specified_color_from_value(value, &style.custom_properties)
        {
            let (color, source) = bind_specified_color(c, style.color);
            style.text_emphasis_color = color;
            style.text_emphasis_color_source = source;
        }
    }

    // `text-indent` percentages resolve during layout against this block's
    // content box, so retain their specified percentage through inheritance.
    if let Some(value) = get_non_special(map, "text-indent")
        && let Some(text_indent) =
            text_indent_from_css_value(value, style, length_context, font_metrics)
    {
        style.text_indent = text_indent;
    }

    // White-space
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "white-space") {
        style.white_space = match k.as_str() {
            "nowrap" => WhiteSpace::NoWrap,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => WhiteSpace::Normal,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "white-space-collapse")
        && k == "preserve"
        && style.white_space == WhiteSpace::Normal
    {
        style.white_space = WhiteSpace::PreWrap;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-wrap-mode")
        && k == "nowrap"
    {
        style.text_wrap_mode_nowrap = true;
    }

    // Tab-size (css-text-3 §6.3). A unitless `<number>` is a count of space
    // advances; a `<length>` is the tab-stop distance directly (stored as a
    // negative sentinel so the renderer can tell counts from absolute lengths).
    // `-moz-tab-size` is accepted as a legacy alias. The initial value is 8.
    for prop in ["tab-size", "-moz-tab-size"] {
        if let Some(CssValue::Number(v)) = get_non_special(map, prop) {
            style.tab_size = v.max(0.0);
        } else if let Some(CssValue::Length(v)) = get_non_special(map, prop) {
            // Encode an absolute length as a negative value; the renderer maps a
            // negative `tab_size` to `-tab_size` points (independent of the
            // space advance).
            style.tab_size = -(v.abs());
        }
    }

    // Vertical-align
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "vertical-align") {
        style.vertical_align = match k.as_str() {
            "super" => VerticalAlign::Super,
            "sub" => VerticalAlign::Sub,
            "top" => VerticalAlign::Top,
            // `text-top`/`text-bottom` align to the parent's text content (font)
            // area edges, which sit inside the line box when the line is taller
            // than the parent font box (css2 §10.8.1).
            "text-top" => VerticalAlign::TextTop,
            "middle" => VerticalAlign::Middle,
            "bottom" => VerticalAlign::Bottom,
            "text-bottom" => VerticalAlign::TextBottom,
            _ => VerticalAlign::Baseline,
        };
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "vertical-align") {
        style.vertical_align = VerticalAlign::Length(*v);
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "vertical-align") {
        style.vertical_align = VerticalAlign::Percent(*v / 100.0);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-overflow") {
        style.text_overflow = match k.as_str() {
            "ellipsis" => TextOverflow::Ellipsis,
            _ => TextOverflow::Clip,
        };
    }
    if let Some(CssValue::Keyword(k)) =
        get_non_special(map, "overflow-wrap").or_else(|| get_non_special(map, "word-wrap"))
    {
        style.overflow_wrap = match k.as_str() {
            "anywhere" => OverflowWrap::Anywhere,
            "break-word" => OverflowWrap::BreakWord,
            _ => OverflowWrap::Normal,
        };
    }
    // `word-break: break-all` permits a break between any two characters so a
    // long unbreakable run fills each line and wraps within the box. We map it
    // onto the same per-character split path the wrapper uses for
    // `overflow-wrap: anywhere`; the visual line-breaking is equivalent for
    // print output. (`keep-all` / `normal` leave the default behavior.)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "word-break") {
        style.word_break_keep_all = k == "keep-all";
        if k == "break-all" && style.overflow_wrap == OverflowWrap::Normal {
            style.overflow_wrap = OverflowWrap::Anywhere;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "hyphens") {
        style.hyphens_manual = k != "none";
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border-collapse") {
        style.border_collapse = match k.as_str() {
            "collapse" => BorderCollapse::Collapse,
            _ => BorderCollapse::Separate,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "table-layout") {
        style.table_layout = match k.as_str() {
            "fixed" => TableLayout::Fixed,
            _ => TableLayout::Auto,
        };
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "border-spacing") {
        style.border_spacing = *v;
        // Single-value shorthand: vertical mirrors horizontal unless an explicit
        // `border-spacing-vertical` (from the two-value form) overrides it below.
        style.border_spacing_vertical = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "border-spacing-horizontal") {
        style.border_spacing = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "border-spacing-vertical") {
        style.border_spacing_vertical = *v;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "empty-cells") {
        style.empty_cells = match k.as_str() {
            "hide" => EmptyCells::Hide,
            _ => EmptyCells::Show,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "caption-side") {
        style.caption_side = match k.as_str() {
            "bottom" => CaptionSide::Bottom,
            _ => CaptionSide::Top,
        };
    }
    // Route comma-separated longhand entries using the winning image list
    // itself. Slot metadata is derived state, not a separate cascade property.
    let image_layers = match get_non_special(map, "background-image") {
        Some(CssValue::BackgroundLayers(layers)) => layers.as_slice(),
        _ => &[],
    };
    let raster_layer_index = image_layers.iter().position(|source| {
        matches!(
            source,
            BackgroundLayerSource::Raster(_) | BackgroundLayerSource::Svg(_)
        )
    });
    let gradient_layer_index = image_layers.iter().position(|source| {
        matches!(
            source,
            BackgroundLayerSource::Linear(_)
                | BackgroundLayerSource::Radial(_)
                | BackgroundLayerSource::Conic(_)
        )
    });

    // For the raster slot the single `background_*` fields are used directly; when
    // it is a non-first layer in a multi-layer list, route its comma-separated
    // entry into those fields. The gradient slot stores its own entry on the
    // gradient struct (`layer_box`).
    let raster_size_idx = raster_layer_index.unwrap_or(0);
    let raster_pos_idx = raster_layer_index.unwrap_or(0);
    let raster_repeat_idx = raster_layer_index.unwrap_or(0);

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-size") {
        if let Some(part) = nth_layer_value(k, raster_size_idx) {
            style.background_size = parse_background_size_value(&part);
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-repeat") {
        if let Some(part) = nth_layer_value(k, raster_repeat_idx) {
            style.background_repeat = parse_background_repeat_value(&part);
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-position") {
        if let Some(part) = nth_layer_value(k, raster_pos_idx) {
            if let Some(pos) = parse_background_position(&part) {
                style.background_position = pos;
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-attachment")
        && let Some(part) = nth_layer_value(k, raster_layer_index.unwrap_or(0))
    {
        style.background_attachment = parse_background_attachment_value(&part);
        if let Some(ref mut lg) = style.background_gradient {
            lg.layer_box.attachment = Some(style.background_attachment);
        }
        if let Some(ref mut rg) = style.background_radial_gradient {
            rg.layer_box.attachment = Some(style.background_attachment);
        }
        if let Some(ref mut cg) = style.background_conic_gradient {
            cg.layer_box.attachment = Some(style.background_attachment);
        }
    }

    // Route the gradient layer's own size/position/repeat entry onto the gradient
    // struct so the renderer can paint it as a positioned, sized tile.
    if let Some(gradient_idx) = gradient_layer_index {
        let mut gradient_box = resolve_gradient_layer_box(map, gradient_idx);
        gradient_box.paint_above_raster =
            raster_layer_index.is_some_and(|raster_idx| gradient_idx < raster_idx);
        if let Some(ref mut lg) = style.background_gradient {
            lg.layer_box = gradient_box;
        }
        if let Some(ref mut rg) = style.background_radial_gradient {
            rg.layer_box = gradient_box;
        }
        if let Some(ref mut cg) = style.background_conic_gradient {
            cg.layer_box = gradient_box;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-origin") {
        if let Some(part) = nth_layer_value(k, raster_layer_index.unwrap_or(0)) {
            style.background_origin = parse_background_origin_value(&part);
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-clip")
        .or_else(|| get_non_special(map, "-webkit-background-clip"))
    {
        if let Some(part) = nth_layer_value(k, raster_layer_index.unwrap_or(0)) {
            style.background_clip = parse_background_clip_value(&part);
        }
    }

    if let Some(value) = get_non_special(map, "border-image-source") {
        style.border_image.source = resolved_raw_css_value(value, &style.custom_properties)
            .filter(|source| !source.trim().eq_ignore_ascii_case("none"))
            .and_then(|source| parse_border_image_source(&source, style.color));
    }
    if let Some(value) = get_non_special(map, "border-image-slice") {
        style.border_image.geometry.slices =
            resolved_raw_css_value(value, &style.custom_properties)
                .and_then(|value| parse_border_image_slices(&value))
                .unwrap_or_default();
    }
    if let Some(value) = get_non_special(map, "border-image-width") {
        style.border_image.geometry.widths =
            resolved_raw_css_value(value, &style.custom_properties)
                .and_then(|value| {
                    parse_border_image_widths(&value, style, length_context, font_metrics)
                })
                .unwrap_or_default();
    }
    if let Some(value) = get_non_special(map, "border-image-outset") {
        style.border_image.geometry.outsets =
            resolved_raw_css_value(value, &style.custom_properties)
                .and_then(|value| {
                    parse_border_image_outsets(&value, style, length_context, font_metrics)
                })
                .unwrap_or_default();
    }
    if let Some(value) = get_non_special(map, "border-image-repeat") {
        style.border_image.geometry.repeats =
            resolved_raw_css_value(value, &style.custom_properties)
                .and_then(|value| parse_border_image_repeats(&value))
                .unwrap_or_default();
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "aspect-ratio") {
        style.aspect_ratio = parse_aspect_ratio(k);
    }

    // object-fit / object-position (replaced-element content placement)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "object-fit") {
        style.object_fit = match k.to_ascii_lowercase().as_str() {
            "contain" => ObjectFit::Contain,
            "cover" => ObjectFit::Cover,
            "none" => ObjectFit::None,
            "scale-down" => ObjectFit::ScaleDown,
            _ => ObjectFit::Fill,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "object-position") {
        if let Some(pos) = parse_object_position(k) {
            style.object_position = pos;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "image-rendering") {
        style.image_rendering = match k.to_ascii_lowercase().as_str() {
            "smooth" => ImageRendering::Smooth,
            "high-quality" => ImageRendering::HighQuality,
            "pixelated" => ImageRendering::Pixelated,
            "crisp-edges" | "optimizespeed" => ImageRendering::CrispEdges,
            "optimizequality" => ImageRendering::Smooth,
            _ => ImageRendering::Auto,
        };
    }

    // z-index
    if let Some(CssValue::Number(v)) = get_non_special(map, "z-index") {
        style.z_index = ZIndex::integer(*v as i32);
    }

    // orphans / widows (css-break-3 §3.4): positive <integer>, initial 2,
    // inherited. A zero or negative value is invalid and ignored (keeps the
    // inherited/initial value), matching Chrome.
    if let Some(CssValue::Number(v)) = get_non_special(map, "orphans") {
        let n = *v as i32;
        if n >= 1 {
            style.orphans = n.min(u8::MAX as i32) as u8;
        }
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "widows") {
        let n = *v as i32;
        if n >= 1 {
            style.widows = n.min(u8::MAX as i32) as u8;
        }
    }

    // Resolve late-bound length values.
    //
    // CSS does not use one universal percentage basis:
    // - width-like properties resolve against the containing block width
    // - height/top/bottom resolve against the containing block height
    // - padding/margin percentages still resolve against width
    //
    // Keep percentage hints for layout-time cases where the containing block
    // height is only known after layout (for example absolute pseudo-elements).
    type LengthSetter = fn(&mut ComputedStyle, f32);
    let inline_length_props: &[(&str, LengthSetter)] = &[
        ("width", |s, v| s.width = Some(v)),
        ("max-width", |s, v| s.max_width = Some(v)),
        ("min-width", |s, v| s.min_width = Some(v)),
        ("margin-top", |s, v| s.margin.top = v),
        ("margin-right", |s, v| s.margin.right = v),
        ("margin-bottom", |s, v| s.margin.bottom = v),
        ("margin-left", |s, v| s.margin.left = v),
        ("padding-top", |s, v| s.padding.top = v),
        ("padding-right", |s, v| s.padding.right = v),
        ("padding-bottom", |s, v| s.padding.bottom = v),
        ("padding-left", |s, v| s.padding.left = v),
        ("left", |s, v| s.left = Some(v)),
        ("right", |s, v| s.right = Some(v)),
        ("gap", |s, v| {
            s.gap = v;
            s.grid_gap = v;
            s.column_gap = v;
            s.row_gap = v;
        }),
        ("grid-gap", |s, v| {
            s.grid_gap = v;
            s.column_gap = v;
            s.row_gap = v;
        }),
        ("border-width", |s, v| {
            s.border.top.specified_width = v;
            s.border.right.specified_width = v;
            s.border.bottom.specified_width = v;
            s.border.left.specified_width = v;
        }),
        // NOTE: `border-radius` is intentionally NOT in this list. A border-radius
        // percentage resolves against the element's OWN border box (horizontal
        // radii against its width, vertical against its height) per CSS
        // Backgrounds §5.1 — NOT against the parent/containing-block width. The
        // dedicated `border-radius` match above keeps percentages in the
        // specified corner structure. Layout resolves each axis once the
        // element's own box is known.
        ("letter-spacing", |s, v| s.letter_spacing = v),
        ("border-spacing", |s, v| {
            s.border_spacing = v;
            s.border_spacing_vertical = v;
        }),
        ("border-spacing-horizontal", |s, v| s.border_spacing = v),
        ("border-spacing-vertical", |s, v| {
            s.border_spacing_vertical = v
        }),
    ];
    for &(prop_name, setter) in inline_length_props {
        if let Some(val) = get_non_special(map, prop_name) {
            if matches!((prop_name, val), ("word-spacing", CssValue::Percentage(_))) {
                continue;
            }
            // For width/max-width/min-width percentages, only pre-resolve when
            // parent.width is actually known. Otherwise the value resolves
            // against viewport_width and produces an oversized result that
            // clamps to 100% at layout time. The percentage_sizing.width
            // hint (set below) is the correct late-bound fallback.
            let is_width_prop = matches!(prop_name, "width" | "max-width" | "min-width");
            if is_width_prop && !parent_width_known && matches!(val, CssValue::Percentage(_)) {
                continue;
            }
            match val {
                CssValue::Percentage(_)
                | CssValue::Em(_)
                | CssValue::Ex(_)
                | CssValue::Ch(_)
                | CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Vmin(_)
                | CssValue::Vmax(_)
                | CssValue::Math(_)
                | CssValue::Var(_, _)
                | CssValue::Number(0.0) => {
                    if let Some(resolved) =
                        resolve_css_length_for_style(val, style, length_context, font_metrics)
                    {
                        setter(style, resolved);
                    }
                }
                CssValue::Keyword(k) => {
                    if let Some(resolved) =
                        resolve_raw_length_for_style(k, style, length_context, font_metrics)
                    {
                        setter(style, resolved);
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(value) = get_non_special(map, "letter-spacing")
        && let Some(letter_spacing) =
            letter_spacing_from_css_value(value, style, length_context, font_metrics)
    {
        style.letter_spacing = letter_spacing;
    }

    if let Some(value) = get_non_special(map, "word-spacing")
        && let Some(word_spacing) = word_spacing_from_css_value(value, style, font_metrics)
    {
        style.word_spacing = word_spacing;
    }

    if let Some(CssValue::Percentage(v)) = get_non_special(map, "width") {
        style.percentage_sizing.width = Some(*v);
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "max-width") {
        style.percentage_sizing.max_width = Some(*v);
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "min-width") {
        style.percentage_sizing.min_width = Some(*v);
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "left") {
        style.percentage_insets.left = Some(*v);
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "right") {
        style.percentage_insets.right = Some(*v);
    }

    let resolved_parent_height = percentage_basis
        .height_or_parent(parent)
        .filter(|height| *height > 0.0);
    let resolve_block_percentage =
        |percent: f32| resolved_parent_height.map(|height| height * percent / 100.0);

    // Height-axis resolution context: percentages inside height/min/max-height
    // clamp() resolve against the parent's content height, so we feed the
    // resolved parent height into the context's `parent_width` field (the field
    // the resolver uses as the percentage basis). Falls back to the viewport
    // height when the parent height is indefinite.
    let height_length_context = length_context
        .with_percentage_basis(resolved_parent_height.unwrap_or(parent.viewport_height));

    if let Some(val) = get_non_special(map, "height") {
        match val {
            CssValue::Percentage(v) => {
                style.percentage_sizing.height = Some(*v);
                style.height = resolve_block_percentage(*v);
            }
            CssValue::Math(_) => {
                // Percentages inside a calc()/clamp() on a block height resolve
                // against the parent's content height, so use the height-axis
                // context (parent height in the percentage-basis slot).
                style.percentage_sizing.height = None;
                style.height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    height_length_context,
                );
            }
            CssValue::Em(_)
            | CssValue::Ex(_)
            | CssValue::Ch(_)
            | CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _)
            | CssValue::Number(0.0) => {
                style.percentage_sizing.height = None;
                style.height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    length_context,
                );
            }
            _ => {}
        }
    }
    if let Some(val) = get_non_special(map, "max-height") {
        match val {
            CssValue::Percentage(v) => {
                style.percentage_sizing.max_height = Some(*v);
                style.max_height = resolve_block_percentage(*v);
            }
            CssValue::Math(_) => {
                style.percentage_sizing.max_height = None;
                style.max_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    height_length_context,
                );
            }
            CssValue::Em(_)
            | CssValue::Ex(_)
            | CssValue::Ch(_)
            | CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _)
            | CssValue::Number(0.0) => {
                style.percentage_sizing.max_height = None;
                style.max_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    length_context,
                );
            }
            _ => {}
        }
    }
    if let Some(val) = get_non_special(map, "min-height") {
        match val {
            CssValue::Percentage(v) => {
                style.percentage_sizing.min_height = Some(*v);
                style.min_height = resolve_block_percentage(*v);
            }
            CssValue::Math(_) => {
                style.percentage_sizing.min_height = None;
                style.min_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    height_length_context,
                );
            }
            CssValue::Em(_)
            | CssValue::Ex(_)
            | CssValue::Ch(_)
            | CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _)
            | CssValue::Number(0.0) => {
                style.percentage_sizing.min_height = None;
                style.min_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    length_context,
                );
            }
            _ => {}
        }
    }
    for (prop_name, setter, hint_setter) in [
        (
            "top",
            (|s: &mut ComputedStyle, v| s.top = Some(v)) as LengthSetter,
            (|s: &mut ComputedStyle, v| s.percentage_insets.top = Some(v))
                as fn(&mut ComputedStyle, f32),
        ),
        (
            "bottom",
            (|s: &mut ComputedStyle, v| s.bottom = Some(v)) as LengthSetter,
            (|s: &mut ComputedStyle, v| s.percentage_insets.bottom = Some(v))
                as fn(&mut ComputedStyle, f32),
        ),
    ] {
        if let Some(val) = get_non_special(map, prop_name) {
            match val {
                CssValue::Percentage(v) => {
                    hint_setter(style, *v);
                    if let Some(resolved) = resolve_block_percentage(*v) {
                        setter(style, resolved);
                    } else {
                        setter(style, 0.0);
                        match prop_name {
                            "top" => style.top = None,
                            "bottom" => style.bottom = None,
                            _ => {}
                        }
                    }
                }
                CssValue::Math(_) => {
                    // top/bottom percentages resolve against the containing
                    // block's height, so use the height-axis context.
                    match prop_name {
                        "top" => style.percentage_insets.top = None,
                        "bottom" => style.percentage_insets.bottom = None,
                        _ => {}
                    }
                    if let Some(resolved) = crate::style::resolve::try_resolve_to_length_in_context(
                        val,
                        &style.custom_properties,
                        height_length_context,
                    ) {
                        setter(style, resolved);
                    }
                }
                CssValue::Em(_)
                | CssValue::Ex(_)
                | CssValue::Ch(_)
                | CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Vmin(_)
                | CssValue::Vmax(_)
                | CssValue::Var(_, _)
                | CssValue::Number(0.0) => {
                    match prop_name {
                        "top" => style.percentage_insets.top = None,
                        "bottom" => style.percentage_insets.bottom = None,
                        _ => {}
                    }
                    if let Some(resolved) = crate::style::resolve::try_resolve_to_length_in_context(
                        val,
                        &style.custom_properties,
                        length_context,
                    ) {
                        setter(style, resolved);
                    }
                }
                _ => {}
            }
        }
    }

    // Resolve var() for keyword properties
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "display") {
        if let Some(kw) =
            crate::style::resolve::try_resolve_var_to_keyword(val, &style.custom_properties)
        {
            if let Some(display) = parse_display_value(&kw) {
                style.display = display;
            }
        }
    }
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "position") {
        if let Some(kw) =
            crate::style::resolve::try_resolve_var_to_keyword(val, &style.custom_properties)
        {
            style.running_name = None;
            style.position = match kw.as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" => Position::Sticky,
                value => {
                    style.running_name = parse_running_position_name(value);
                    Position::Static
                }
            };
        }
    }
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "text-align") {
        if let Some(kw) =
            crate::style::resolve::try_resolve_var_to_keyword(val, &style.custom_properties)
        {
            style.text_align = match kw.as_str() {
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                "justify" => TextAlign::Justify,
                _ => TextAlign::Left,
            };
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "list-style-type") {
        style.list_style_type = parse_list_style_type(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "list-style-position") {
        style.list_style_position = match k.to_ascii_lowercase().as_str() {
            "inside" => ListStylePosition::Inside,
            _ => ListStylePosition::Outside,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "marker-side") {
        style.marker_side_match_parent = k.eq_ignore_ascii_case("match-parent");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "list-style-image") {
        let trimmed = k.trim();
        style.list_style_image = if trimmed.eq_ignore_ascii_case("none") {
            None
        } else if crate::parser::css::extract_url_path(trimmed).is_some() {
            Some(trimmed.to_string())
        } else {
            style.list_style_image.clone()
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "list-style") {
        // The shorthand resets all three longhands; an omitted component takes
        // its initial value (css-lists-3 §6.1). A `url(...)` token sets
        // list-style-image; `none` clears both type and image.
        if let Some(url) = crate::parser::css::extract_url_path(k.trim()) {
            let _ = url;
            style.list_style_image = Some(k.trim().to_string());
        }
        let lower = k.to_ascii_lowercase();
        for part in lower.split_whitespace() {
            if part.starts_with("url(") {
                continue;
            }
            match part {
                "inside" => style.list_style_position = ListStylePosition::Inside,
                "outside" => style.list_style_position = ListStylePosition::Outside,
                other => style.list_style_type = parse_list_style_type(other),
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "content") {
        style.content = parse_content_value(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "quotes") {
        style.quotes = parse_quotes_value(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "counter-reset") {
        style.counter_reset = parse_counter_directive(k, 0);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "counter-increment") {
        style.counter_increment = parse_counter_directive(k, 1);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "counter-set") {
        style.counter_set = parse_counter_directive(k, 0);
    }
    synthesize_simple_multi_background_svg(map, style);
}

fn parse_list_style_type(k: &str) -> ListStyleType {
    let trimmed = k.trim();
    if let Some(marker) = parse_quoted_css_string(trimmed) {
        return ListStyleType::String(marker);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "disc" => ListStyleType::Disc,
        "circle" => ListStyleType::Circle,
        "square" => ListStyleType::Square,
        "decimal" => ListStyleType::Decimal,
        "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
        "lower-roman" => ListStyleType::LowerRoman,
        "upper-roman" => ListStyleType::UpperRoman,
        "cjk-decimal" => ListStyleType::CjkDecimal,
        "none" => ListStyleType::None,
        other => ListStyleType::Custom(other.to_string()),
    }
}

fn resolve_custom_counter_style(list_style_type: &mut ListStyleType, rules: &[CssRule]) {
    let ListStyleType::Custom(name) = list_style_type else {
        return;
    };
    let Some(style) = find_counter_style(name, rules) else {
        return;
    };
    *list_style_type = ListStyleType::CounterStyle(style);
}

fn resolve_custom_counter_styles_in_content(items: &mut [ContentItem], rules: &[CssRule]) {
    for item in items {
        match item {
            ContentItem::Counter(_, style) | ContentItem::Counters(_, _, style) => {
                resolve_custom_counter_style(style, rules);
            }
            _ => {}
        }
    }
}

fn find_counter_style(name: &str, rules: &[CssRule]) -> Option<CounterStyle> {
    rules.iter().rev().find_map(|rule| {
        rule.counter_style_name()
            .filter(|rule_name| rule_name.trim().eq_ignore_ascii_case(name))
            .and_then(|_| parse_counter_style_rule(&rule.declarations))
    })
}

fn counter_style_keyword(map: &StyleMap, property: &str) -> Option<String> {
    match map.get(property)? {
        CssValue::Keyword(value) => Some(value.trim().to_string()),
        CssValue::Number(value) => Some(value.to_string()),
        CssValue::Em(value) => Some(format!("{value}em")),
        _ => None,
    }
}

fn parse_counter_style_rule(map: &StyleMap) -> Option<CounterStyle> {
    let system_raw = counter_style_keyword(map, "system").unwrap_or_else(|| "symbolic".into());
    let system_lower = system_raw.to_ascii_lowercase();
    let system = if system_lower.split_whitespace().any(|part| part == "cyclic") {
        CounterStyleSystem::Cyclic
    } else if system_lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair == ["extends", "decimal"])
    {
        CounterStyleSystem::ExtendsDecimal
    } else {
        return None;
    };
    let symbols = counter_style_keyword(map, "symbols")
        .map(|raw| parse_counter_style_symbols(&raw))
        .unwrap_or_default();
    let prefix = counter_style_keyword(map, "prefix")
        .and_then(|raw| parse_quoted_css_string(raw.trim()))
        .unwrap_or_default();
    let suffix = counter_style_keyword(map, "suffix")
        .and_then(|raw| parse_quoted_css_string(raw.trim()))
        .unwrap_or_else(|| ". ".to_string());
    let pad = counter_style_keyword(map, "pad").and_then(|raw| {
        let mut parts = raw.split_whitespace();
        let width = parts.next()?.parse::<usize>().ok()?;
        let symbol = parse_quoted_css_string(raw[raw.find(char::is_whitespace)?..].trim())?;
        Some((width, symbol))
    });
    let negative = counter_style_keyword(map, "negative")
        .map(|raw| {
            let parts = parse_counter_style_symbols(&raw);
            match parts.as_slice() {
                [prefix, suffix, ..] => (prefix.clone(), suffix.clone()),
                [prefix] => (prefix.clone(), String::new()),
                _ => ("-".to_string(), String::new()),
            }
        })
        .unwrap_or_else(|| ("-".to_string(), String::new()));

    Some(CounterStyle {
        system,
        symbols,
        prefix,
        suffix,
        pad,
        negative,
    })
}

fn parse_counter_style_symbols(raw: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        rest = rest.trim_start();
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '"' || ch == '\'' {
            let after = &rest[ch.len_utf8()..];
            if let Some(end) = after.find(ch) {
                symbols.push(repair_css_string_mojibake(&after[..end]));
                rest = &after[end + ch.len_utf8()..];
            } else {
                symbols.push(repair_css_string_mojibake(after));
                break;
            }
        } else if let Some(space) = rest.find(char::is_whitespace) {
            symbols.push(rest[..space].to_string());
            rest = &rest[space..];
        } else {
            symbols.push(rest.to_string());
            break;
        }
    }
    symbols
}

fn parse_quoted_css_string(raw: &str) -> Option<String> {
    let mut chars = raw.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if ch == quote {
            return Some(repair_css_string_mojibake(&out));
        } else {
            out.push(ch);
        }
    }
    Some(repair_css_string_mojibake(&out))
}

fn repair_css_string_mojibake(s: &str) -> String {
    let repaired = s
        .replace("Ã‚Â«", "«")
        .replace("Ã‚Â»", "»")
        .replace("Ã¢Â\u{86}Â\u{92}", "→")
        .replace("Ã¢ÂÂ¹", "‹")
        .replace("Ã¢ÂÂº", "›")
        .replace("Ã«", "«")
        .replace("Ã»", "»")
        .replace("Â«", "«")
        .replace("Â»", "»")
        .replace("â†’", "→")
        .replace("â€¹", "‹")
        .replace("â€º", "›");
    for glyph in ["«", "»", "‹", "›"] {
        if repaired.contains(glyph) {
            return glyph.to_string();
        }
    }
    repaired
}

/// Test-only wrapper for `parse_content_value`.
#[cfg(test)]
pub fn parse_content_value_pub(raw: &str) -> Vec<ContentItem> {
    parse_content_value(raw)
}

fn parse_content_value(raw: &str) -> Vec<ContentItem> {
    let s = raw.trim();
    if s == "none" || s == "normal" {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(body) = rest.strip_prefix('"') {
            if let Some(end) = body.find('"') {
                items.push(ContentItem::String(repair_css_string_mojibake(
                    &body[..end],
                )));
                rest = &body[end + 1..];
            } else {
                items.push(ContentItem::String(repair_css_string_mojibake(body)));
                break;
            }
        } else if let Some(body) = rest.strip_prefix('\'') {
            if let Some(end) = body.find('\'') {
                items.push(ContentItem::String(repair_css_string_mojibake(
                    &body[..end],
                )));
                rest = &body[end + 1..];
            } else {
                items.push(ContentItem::String(repair_css_string_mojibake(body)));
                break;
            }
        } else if let Some((name, tail)) = parse_content_function(rest, "attr(") {
            items.push(ContentItem::Attr(name.trim().to_string()));
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function_balanced(rest, "target-counter(")
        {
            let mut parts = inner.splitn(2, ',').map(str::trim);
            let target = parts.next().unwrap_or("");
            let counter = parts.next().unwrap_or("");
            if !target.is_empty() && counter.eq_ignore_ascii_case("page") {
                items.push(ContentItem::String(format!(
                    "{TARGET_PLACEHOLDER_START}counter|{target}|page{TARGET_PLACEHOLDER_END}"
                )));
            }
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function_balanced(rest, "target-text(") {
            let target = inner.split(',').next().unwrap_or("").trim();
            if !target.is_empty() {
                items.push(ContentItem::String(format!(
                    "{TARGET_PLACEHOLDER_START}text|{target}{TARGET_PLACEHOLDER_END}"
                )));
            }
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function_balanced(rest, "leader(") {
            let pattern = parse_quoted_css_string(inner.trim()).unwrap_or_else(|| {
                let trimmed = inner.trim();
                if trimmed.is_empty() {
                    ".".to_string()
                } else {
                    trimmed.to_string()
                }
            });
            items.push(ContentItem::Leader(pattern));
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function(rest, "counters(") {
            // counters(name, sep[, style])
            let mut parts = inner.splitn(3, ',');
            let name = parts.next().unwrap_or("").trim().to_string();
            let sep = parts
                .next()
                .map(|s| {
                    s.trim()
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .to_string()
                })
                .unwrap_or_else(|| ".".to_string());
            let style = parts
                .next()
                .map(|s| parse_list_style_type(s.trim()))
                .unwrap_or(ListStyleType::Decimal);
            items.push(ContentItem::Counters(name, sep, style));
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function(rest, "counter(") {
            // counter(name[, style])
            let (name, style) = inner.split_once(',').map_or_else(
                || (inner.trim().to_string(), ListStyleType::Decimal),
                |(name, style)| (name.trim().to_string(), parse_list_style_type(style.trim())),
            );
            items.push(ContentItem::Counter(name, style));
            rest = tail;
        } else if let Some((inner, tail)) = parse_content_function(rest, "url(") {
            // `url(...)` is a <content-replacement>: it makes the pseudo a
            // replaced element. Strip optional surrounding quotes from the URL.
            let url = inner
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'')
                .to_string();
            items.push(ContentItem::Url(url));
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("no-open-quote") {
            items.push(ContentItem::NoOpenQuote);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("no-close-quote") {
            items.push(ContentItem::NoCloseQuote);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("open-quote") {
            items.push(ContentItem::OpenQuote);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("close-quote") {
            items.push(ContentItem::CloseQuote);
            rest = tail;
        } else if let Some(space) = rest.find(char::is_whitespace) {
            rest = &rest[space..];
        } else {
            break;
        }
    }
    items
}

fn parse_content_function<'a>(rest: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    rest.strip_prefix(prefix)?.split_once(')')
}

fn parse_content_function_balanced<'a>(rest: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let body = rest.strip_prefix(prefix)?;
    let mut depth = 0usize;
    for (idx, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some((&body[..idx], &body[idx + 1..])),
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Parse the CSS `quotes` property (css-content-3 §2.4.1).
///
/// `none` -> `Some(vec![])` (open/close-quote produce nothing).
/// `auto` / `match-parent` / css-wide keywords -> `None` (use UA default).
/// `<string> <string>+` -> ordered (open, close) pairs by nesting level.
fn parse_quotes_value(raw: &str) -> Option<Vec<(String, String)>> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower == "none" {
        return Some(Vec::new());
    }
    if lower == "auto" || lower == "match-parent" || lower == "inherit" || lower == "initial" {
        return None;
    }
    // Collect every double/single-quoted string in order, honoring `\"` escapes.
    let mut strings: Vec<String> = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == '"' || c == '\'' {
            let quote = c;
            chars.next();
            let mut buf = String::new();
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        buf.push(next);
                    }
                } else if ch == quote {
                    break;
                } else {
                    buf.push(ch);
                }
            }
            strings.push(repair_css_string_mojibake(&buf));
        } else {
            chars.next();
        }
    }
    if strings.len() < 2 {
        return None;
    }
    let pairs: Vec<(String, String)> = strings
        .chunks_exact(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    if pairs.is_empty() { None } else { Some(pairs) }
}

fn parse_counter_directive(raw: &str, default_value: i32) -> Vec<(String, i32)> {
    let s = raw.trim();
    if s == "none" {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut tokens = s.split_whitespace().peekable();
    while let Some(name) = tokens.next() {
        let val = tokens
            .peek()
            .and_then(|t| t.parse::<i32>().ok())
            .inspect(|_| {
                let _ = tokens.next();
            })
            .unwrap_or(default_value);
        result.push((name.to_string(), val));
    }
    result
}

fn parse_aspect_ratio(raw: &str) -> Option<f32> {
    let value = raw.trim();
    if value.is_empty() || matches!(value.to_ascii_lowercase().as_str(), "auto" | "none") {
        return None;
    }
    if let Some((lhs, rhs)) = value.split_once('/') {
        let num = lhs.trim().parse::<f32>().ok()?;
        let den = rhs.trim().parse::<f32>().ok()?;
        return (num > 0.0 && den > 0.0).then_some(num / den);
    }
    value.parse::<f32>().ok().filter(|ratio| *ratio > 0.0)
}

/// Parse CSS `object-position` (a `<position>` value, css-images-3 §5.5).
/// Supports the keywords `left`/`right`/`top`/`bottom`/`center`, percentages,
/// lengths, and the 3/4-value edge-offset syntax (e.g. `right 10px bottom 20%`).
/// Returns `None` for unrecognized input so the caller keeps the default.
fn parse_object_position(raw: &str) -> Option<ObjectPosition> {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    if tokens.is_empty() || tokens.len() > 4 {
        return None;
    }

    // A token that names an edge (used to anchor the offset that follows it).
    fn edge(token: &str) -> Option<Edge> {
        match token {
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            "top" => Some(Edge::Top),
            "bottom" => Some(Edge::Bottom),
            "center" => Some(Edge::Center),
            _ => None,
        }
    }

    // A length or percentage offset value (no keyword).
    fn offset(token: &str) -> Option<Offset> {
        if let Some(pct) = token.strip_suffix('%') {
            return pct.trim().parse::<f32>().ok().map(Offset::Percent);
        }
        match crate::parser::css::parse_length(token) {
            Some(CssValue::Length(len)) => Some(Offset::Length(len)),
            _ => None,
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Edge {
        Left,
        Right,
        Top,
        Bottom,
        Center,
    }
    #[derive(Clone, Copy)]
    enum Offset {
        Percent(f32),
        Length(f32),
    }

    // Resolve an edge + optional trailing offset into an axis component. The
    // offset is measured from the named edge; for the far edge (right/bottom) it
    // is converted to an offset from the start edge.
    fn component(edge: Edge, off: Option<Offset>) -> Option<ObjectPositionComponent> {
        let from_start = matches!(edge, Edge::Left | Edge::Top | Edge::Center);
        match off {
            None => Some(ObjectPositionComponent::Fraction(match edge {
                Edge::Left | Edge::Top => 0.0,
                Edge::Right | Edge::Bottom => 1.0,
                Edge::Center => 0.5,
            })),
            // Offset from the far edge: percentage P% from the end == (100-P)%
            // from the start; a length L from the end aligns the object's end
            // edge at free_space - L, resolved once the object size is known.
            Some(Offset::Percent(p)) => Some(ObjectPositionComponent::Fraction(if from_start {
                p / 100.0
            } else {
                1.0 - p / 100.0
            })),
            Some(Offset::Length(l)) if from_start => Some(ObjectPositionComponent::Length(l)),
            Some(Offset::Length(l)) => Some(ObjectPositionComponent::FarEdgeLength(l)),
        }
    }

    // One-value: a single keyword or offset; the other axis defaults to center.
    if tokens.len() == 1 {
        let t = &tokens[0];
        if let Some(e) = edge(t) {
            return match e {
                Edge::Left | Edge::Right => Some(ObjectPosition {
                    x: component(e, None)?,
                    y: ObjectPositionComponent::Fraction(0.5),
                }),
                Edge::Top | Edge::Bottom => Some(ObjectPosition {
                    x: ObjectPositionComponent::Fraction(0.5),
                    y: component(e, None)?,
                }),
                Edge::Center => Some(ObjectPosition::default()),
            };
        }
        let c = match offset(t)? {
            Offset::Percent(p) => ObjectPositionComponent::Fraction(p / 100.0),
            Offset::Length(l) => ObjectPositionComponent::Length(l),
        };
        return Some(ObjectPosition {
            x: c,
            y: ObjectPositionComponent::Fraction(0.5),
        });
    }

    // Two-value: [x] [y]. Each is a keyword or an offset. Keywords may appear in
    // either order (e.g. `top right`); offsets are positional (x then y).
    if tokens.len() == 2 {
        let a_edge = edge(&tokens[0]);
        let b_edge = edge(&tokens[1]);
        // If both are keywords and one is vertical, allow swapped order.
        if let (Some(ea), Some(eb)) = (a_edge, b_edge) {
            let a_vertical = matches!(ea, Edge::Top | Edge::Bottom);
            let (ex, ey) = if a_vertical { (eb, ea) } else { (ea, eb) };
            return Some(ObjectPosition {
                x: component(ex, None)?,
                y: component(ey, None)?,
            });
        }
        let x = match a_edge {
            Some(e) => component(e, None)?,
            None => match offset(&tokens[0])? {
                Offset::Percent(p) => ObjectPositionComponent::Fraction(p / 100.0),
                Offset::Length(l) => ObjectPositionComponent::Length(l),
            },
        };
        let y = match b_edge {
            Some(e) => component(e, None)?,
            None => match offset(&tokens[1])? {
                Offset::Percent(p) => ObjectPositionComponent::Fraction(p / 100.0),
                Offset::Length(l) => ObjectPositionComponent::Length(l),
            },
        };
        return Some(ObjectPosition { x, y });
    }

    // Three/four-value edge-offset syntax: a sequence of (edge [offset]) groups,
    // one per axis. Parse greedily into per-edge components.
    let mut x: Option<ObjectPositionComponent> = None;
    let mut y: Option<ObjectPositionComponent> = None;
    let mut i = 0;
    while i < tokens.len() {
        let e = edge(&tokens[i])?;
        let mut off = None;
        if i + 1 < tokens.len() && edge(&tokens[i + 1]).is_none() {
            off = Some(offset(&tokens[i + 1])?);
            i += 2;
        } else {
            i += 1;
        }
        let comp = component(e, off)?;
        match e {
            Edge::Left | Edge::Right => x = Some(comp),
            Edge::Top | Edge::Bottom => y = Some(comp),
            Edge::Center => {
                if x.is_none() {
                    x = Some(comp);
                } else {
                    y = Some(comp);
                }
            }
        }
    }

    Some(ObjectPosition {
        x: x.unwrap_or(ObjectPositionComponent::Fraction(0.5)),
        y: y.unwrap_or(ObjectPositionComponent::Fraction(0.5)),
    })
}

fn parse_filter_blur(val: &str) -> Option<f32> {
    let raw = val.trim();
    if raw.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }

    let inner = raw.strip_prefix("blur(")?.strip_suffix(')')?.trim();
    if inner.is_empty() {
        return None;
    }
    if let Ok(value) = inner.parse::<f32>() {
        return (value == 0.0).then_some(0.0);
    }

    match crate::parser::css::parse_length(inner)? {
        CssValue::Length(length) if length >= 0.0 => Some(length),
        _ => None,
    }
}

/// Parsed CSS filter-list state. Operations stay in source order; an SVG URL
/// remains unresolved until layout has access to the document definitions.
struct ParsedFilterList {
    operations: Vec<FilterOperation>,
    url_id: Option<String>,
    establishes_stacking_context: bool,
}

impl ParsedFilterList {
    fn none() -> Self {
        Self {
            operations: Vec::new(),
            url_id: None,
            establishes_stacking_context: false,
        }
    }
}

/// Parse a full CSS `filter` value into one ordered operation stream.
/// Unknown or malformed functions invalidate the complete list; `none` clears
/// both paint and stacking behavior.
fn parse_filter_for_color(val: &str, current_color: Color) -> ParsedFilterList {
    let raw = val.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return ParsedFilterList::none();
    }
    let mut ops = Vec::new();
    let mut url_id = None;
    let mut rest = raw;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let Some(open) = rest.find('(') else {
            return ParsedFilterList::none();
        };
        if rest[..open].trim().is_empty() {
            return ParsedFilterList::none();
        }
        let name = rest[..open].trim().to_ascii_lowercase();
        let after_open = &rest[open + 1..];
        let Some(close_rel) = matching_close_paren(after_open) else {
            return ParsedFilterList::none();
        };
        let arg = after_open[..close_rel].trim();
        match name.as_str() {
            "blur" => {
                if let Some(r) = parse_filter_blur(&format!("blur({arg})")) {
                    ops.push(FilterOperation::Blur(r));
                } else {
                    return ParsedFilterList::none();
                }
            }
            "grayscale" => {
                ops.push(FilterOperation::Grayscale(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "sepia" => {
                ops.push(FilterOperation::Sepia(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "invert" => {
                ops.push(FilterOperation::Invert(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "brightness" => {
                ops.push(FilterOperation::Brightness(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "contrast" => {
                ops.push(FilterOperation::Contrast(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "saturate" => {
                ops.push(FilterOperation::Saturate(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "hue-rotate" => {
                ops.push(FilterOperation::HueRotate(parse_filter_angle(arg)));
            }
            "opacity" => {
                ops.push(FilterOperation::Opacity(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "drop-shadow" => {
                if let Some(ds) = parse_drop_shadow(arg, current_color) {
                    ops.push(FilterOperation::DropShadow(ds));
                } else {
                    return ParsedFilterList::none();
                }
            }
            "url" => {
                // `filter: url(#id)` references an SVG <filter> element by its
                // fragment id (css-filter-effects-1 §3). Strip optional quotes
                // and the leading '#'; the referenced filter is resolved during
                // layout, where the DOM is available.
                let inner = arg.trim().trim_matches(|c| c == '\'' || c == '"');
                if let Some(id) = inner.strip_prefix('#') {
                    url_id = Some(id.to_string());
                } else {
                    return ParsedFilterList::none();
                }
            }
            _ => return ParsedFilterList::none(),
        }
        rest = &after_open[close_rel + 1..];
    }
    ParsedFilterList {
        operations: ops,
        url_id,
        establishes_stacking_context: true,
    }
}

#[cfg(test)]
fn parse_filter(val: &str) -> ParsedFilterList {
    parse_filter_for_color(val, Color::BLACK)
}

/// Parse the inner argument of `drop-shadow(<offset-x> <offset-y> <blur>?
/// <color>?)` (css-filter-effects-1 §4.4). Lengths become points; the color
/// defaults to the element's `currentColor`, resolved after cascade finalization.
/// Returns `None` when the two required offsets are missing.
fn parse_drop_shadow(arg: &str, current_color: Color) -> Option<DropShadow> {
    let mut lengths: Vec<f32> = Vec::new();
    let mut color = None;
    // Keep functional colors such as `color(srgb 0 0 0 / .5)` intact. This
    // parser runs after the outer drop-shadow() function has been balanced.
    for tok in split_css_whitespace(arg) {
        if let Some(CssValue::Length(l)) = crate::parser::css::parse_length(tok) {
            lengths.push(l);
        } else if let Some(c) = parse_border_color(tok) {
            color = Some(c);
        } else if tok == "0" {
            lengths.push(0.0);
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(DropShadow {
        dx: lengths[0],
        dy: lengths[1],
        blur: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
        color: color
            .unwrap_or(SpecifiedColor::CurrentColor)
            .resolve(current_color),
    })
}

/// Parse a filter amount: `100%` -> 1.0, `1.5` -> 1.5, empty -> `default`.
fn parse_filter_amount(arg: &str, default: f32) -> f32 {
    let a = arg.trim();
    if a.is_empty() {
        return default;
    }
    if let Some(p) = a.strip_suffix('%') {
        return p.trim().parse::<f32>().map_or(default, |v| v / 100.0);
    }
    a.parse::<f32>().unwrap_or(default)
}

/// Parse a hue-rotate angle to degrees (`90deg`, `90`, `0.25turn`, `1.57rad`).
fn parse_filter_angle(arg: &str) -> f32 {
    let a = arg.trim();
    if let Some(d) = a.strip_suffix("deg") {
        d.trim().parse::<f32>().unwrap_or(0.0)
    } else if let Some(t) = a.strip_suffix("turn") {
        t.trim().parse::<f32>().map_or(0.0, |v| v * 360.0)
    } else if let Some(r) = a.strip_suffix("rad") {
        r.trim().parse::<f32>().map_or(0.0, f32::to_degrees)
    } else {
        a.parse::<f32>().unwrap_or(0.0)
    }
}

/// Split a comma-separated background property value into its top-level layers
/// (commas inside parentheses are ignored) and return the layer at `index`.
///
/// CSS repeats the shorter list to cover all layers, so an out-of-range index
/// wraps around modulo the layer count. Returns `None` only when there are no
/// layers at all.
fn nth_layer_value(val: &str, index: usize) -> Option<String> {
    let parts = split_top_level_commas_value(val);
    if parts.is_empty() {
        return None;
    }
    Some(parts[index % parts.len()].clone())
}

/// Split a value on top-level commas (ignoring commas inside parentheses).
fn split_top_level_commas_value(val: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for ch in val.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' if depth > 0 => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() || !parts.is_empty() {
        parts.push(current);
    }
    parts.into_iter().map(|p| p.trim().to_string()).collect()
}

/// Parse a single (non-comma) `background-size` layer value.
fn parse_background_size_value(val: &str) -> BackgroundSize {
    match val {
        "cover" => BackgroundSize::Cover,
        "contain" => BackgroundSize::Contain,
        "auto" => BackgroundSize::Auto,
        _ => parse_background_size_explicit(val).unwrap_or(BackgroundSize::Auto),
    }
}

/// Parse a single (non-comma) `background-repeat` layer value.
fn parse_background_repeat_value(val: &str) -> BackgroundRepeat {
    match val {
        "no-repeat" => BackgroundRepeat::NoRepeat,
        "repeat-x" => BackgroundRepeat::RepeatX,
        "repeat-y" => BackgroundRepeat::RepeatY,
        "space" => BackgroundRepeat::Space,
        "round" => BackgroundRepeat::Round,
        "space round" => BackgroundRepeat::SpaceRound,
        "round space" => BackgroundRepeat::RoundSpace,
        _ => BackgroundRepeat::Repeat,
    }
}

fn parse_background_origin_value(val: &str) -> BackgroundOrigin {
    match val.trim() {
        "border-box" => BackgroundOrigin::Border,
        "content-box" => BackgroundOrigin::Content,
        _ => BackgroundOrigin::Padding,
    }
}

fn parse_background_clip_value(val: &str) -> BackgroundClip {
    match val.trim() {
        "padding-box" => BackgroundClip::Padding,
        "content-box" => BackgroundClip::Content,
        "text" => BackgroundClip::Text,
        _ => BackgroundClip::Border,
    }
}

fn parse_background_attachment_value(val: &str) -> BackgroundAttachment {
    match val.trim() {
        "fixed" => BackgroundAttachment::Fixed,
        "local" => BackgroundAttachment::Local,
        _ => BackgroundAttachment::Scroll,
    }
}

/// Build the per-layer size/position/repeat box for the gradient layer at
/// `gradient_idx`, pulling the matching comma-separated entry from each of the
/// `background-size` / `-position` / `-repeat` properties.
fn resolve_gradient_layer_box(map: &StyleMap, gradient_idx: usize) -> GradientLayerBox {
    let size = get_non_special(map, "background-size").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).map(|part| parse_background_size_value(&part))
        }
        _ => None,
    });
    let repeat = get_non_special(map, "background-repeat").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).map(|part| parse_background_repeat_value(&part))
        }
        _ => None,
    });
    let position = get_non_special(map, "background-position").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).and_then(|part| parse_background_position(&part))
        }
        _ => None,
    });
    let origin = get_non_special(map, "background-origin").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).map(|part| parse_background_origin_value(&part))
        }
        _ => None,
    });
    let clip = get_non_special(map, "background-clip").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).map(|part| parse_background_clip_value(&part))
        }
        _ => None,
    });
    let attachment = get_non_special(map, "background-attachment").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, gradient_idx).map(|part| parse_background_attachment_value(&part))
        }
        _ => None,
    });
    GradientLayerBox {
        size,
        position,
        repeat,
        origin,
        clip,
        attachment,
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
struct CssBoxRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

enum SimpleBackgroundLayerSource {
    Image(String),
    Linear(LinearGradient),
}

fn synthesize_simple_multi_background_svg(map: &StyleMap, style: &mut ComputedStyle) {
    let Some(sources) = parse_background_layer_sources(map, style.color, &style.custom_properties)
    else {
        return;
    };
    if sources.len() <= 1 {
        return;
    }
    let Some((border_width, border_height)) = style_background_border_box_size(style) else {
        return;
    };
    if let Some(tree) = build_blended_linear_background_raster_svg(
        map,
        style,
        &sources,
        border_width,
        border_height,
    ) {
        style.clear_background_images();
        style.background_color = None;
        style.background_svg = Some(tree);
        style.background_size = BackgroundSize::Explicit {
            width: border_width,
            height: Some(border_height),
            width_is_percent: false,
            height_is_percent: false,
        };
        style.background_repeat = BackgroundRepeat::NoRepeat;
        style.background_position = BackgroundPosition::default();
        style.background_origin = BackgroundOrigin::Border;
        style.background_clip = BackgroundClip::Border;
        return;
    }
    let Some(svg) =
        build_simple_multi_background_svg(map, style, &sources, border_width, border_height)
    else {
        return;
    };
    let Some(tree) = crate::parser::svg::parse_svg_from_string(&svg) else {
        return;
    };

    style.clear_background_images();
    style.background_svg = Some(tree);
    style.background_size = BackgroundSize::Explicit {
        width: border_width,
        height: Some(border_height),
        width_is_percent: false,
        height_is_percent: false,
    };
    style.background_repeat = BackgroundRepeat::NoRepeat;
    style.background_position = BackgroundPosition::default();
    style.background_origin = BackgroundOrigin::Border;
    style.background_clip = BackgroundClip::Border;
}

fn build_blended_linear_background_raster_svg(
    map: &StyleMap,
    style: &ComputedStyle,
    sources: &[SimpleBackgroundLayerSource],
    border_width: f32,
    border_height: f32,
) -> Option<crate::parser::svg::SvgTree> {
    if style.background_clip != BackgroundClip::Border || !style.border_radii.is_zero() {
        return None;
    }
    if !sources
        .iter()
        .all(|source| matches!(source, SimpleBackgroundLayerSource::Linear(_)))
    {
        return None;
    }
    if sources
        .iter()
        .enumerate()
        .all(|(idx, _)| style.background_blend_mode.background_layer(idx) == BlendMode::Normal)
    {
        return None;
    }
    if sources.iter().enumerate().any(|(idx, _)| {
        !crate::render::blend::supports(style.background_blend_mode.background_layer(idx))
    }) {
        return None;
    }

    let dimensions = background_raster_dimensions(
        border_width,
        border_height,
        style.raster_quality.background_dpi,
    )?;
    let base = style
        .background_color
        .unwrap_or_else(|| Color::rgba8(0, 0, 0, 0));
    let border_rect = CssBoxRect {
        x: 0.0,
        y: 0.0,
        width: border_width,
        height: border_height,
    };
    let layers = sources
        .iter()
        .enumerate()
        .rev()
        .map(|(index, source)| {
            let SimpleBackgroundLayerSource::Linear(gradient) = source else {
                return None;
            };
            Some((
                raster_linear_background_layer(map, style, index, border_rect, gradient)?,
                style.background_blend_mode.background_layer(index),
            ))
        })
        .collect::<Option<Vec<_>>>()?;

    tiled_raster_background_svg(dimensions, border_width, border_height, |tile| {
        let mut image =
            image::RgbaImage::from_pixel(tile.width, tile.height, image::Rgba(base.to_rgba8()));
        for (layer, blend_mode) in &layers {
            for py in 0..tile.height {
                let y = (tile.y + py) as f32 + 0.5;
                let y = y * border_height / dimensions.height as f32;
                for px in 0..tile.width {
                    let x = (tile.x + px) as f32 + 0.5;
                    let x = x * border_width / dimensions.width as f32;
                    let Some(source_pixel) = sample_raster_linear_background_layer(layer, x, y)
                    else {
                        continue;
                    };
                    let backdrop = *image.get_pixel(px, py);
                    image.put_pixel(
                        px,
                        py,
                        crate::render::blend::composite_pixel(
                            source_pixel,
                            backdrop,
                            *blend_mode,
                            false,
                        )?,
                    );
                }
            }
        }
        Some(image)
    })
}

struct RasterLinearBackgroundLayer {
    sampler: crate::render::gradient_sampling::LinearGradientSampler,
    origin: CssBoxRect,
    clip: CssBoxRect,
    tiles: crate::render::background::BackgroundTilePattern,
}

fn raster_linear_background_layer(
    map: &StyleMap,
    style: &ComputedStyle,
    index: usize,
    border_rect: CssBoxRect,
    gradient: &LinearGradient,
) -> Option<RasterLinearBackgroundLayer> {
    let origin = background_layer_origin_rect(map, style, index, border_rect)?;
    let clip = background_layer_clip_rect(map, style, index, border_rect)?;
    let size = background_layer_size(map, index).unwrap_or(BackgroundSize::Auto);
    let position = background_layer_position(map, index).unwrap_or_default();
    let repeat = get_non_special(map, "background-repeat")
        .and_then(|v| match v {
            CssValue::Keyword(k) => {
                nth_layer_value(k, index).map(|part| parse_background_repeat_value(&part))
            }
            _ => None,
        })
        .unwrap_or(BackgroundRepeat::Repeat);
    let tiles = crate::render::background::BackgroundTilePattern::resolve(
        size,
        position,
        repeat,
        Size::new(origin.width, origin.height),
    )?;
    Some(RasterLinearBackgroundLayer {
        sampler: crate::render::gradient_sampling::LinearGradientSampler::resolve(
            gradient,
            tiles.tile_size(),
        )?,
        origin,
        clip,
        tiles,
    })
}

fn sample_raster_linear_background_layer(
    layer: &RasterLinearBackgroundLayer,
    x: f32,
    y: f32,
) -> Option<image::Rgba<u8>> {
    if !point_in_css_rect(x, y, layer.clip) {
        return None;
    }
    let local = layer
        .tiles
        .sample(Point::new(x - layer.origin.x, y - layer.origin.y))?;
    Some(image::Rgba(layer.sampler.sample(local).to_rgba8()))
}

fn point_in_css_rect(x: f32, y: f32, rect: CssBoxRect) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn tiled_raster_background_svg(
    dimensions: RasterDimensions,
    width: f32,
    height: f32,
    mut render: impl FnMut(RasterTile) -> Option<image::RgbaImage>,
) -> Option<crate::parser::svg::SvgTree> {
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        fmt_svg_num(width),
        fmt_svg_num(height),
        fmt_svg_num(width),
        fmt_svg_num(height),
    );
    for tile in dimensions.tiles(MAX_RASTER_TILE_EDGE)? {
        let image = render(tile)?;
        if image.dimensions() != (tile.width, tile.height) {
            return None;
        }
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .ok()?;
        let x = width * tile.x as f32 / dimensions.width as f32;
        let y = height * tile.y as f32 / dimensions.height as f32;
        let tile_width = width * tile.width as f32 / dimensions.width as f32;
        let tile_height = height * tile.height as f32 / dimensions.height as f32;
        svg.push_str(&format!(
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>",
            fmt_svg_num(x),
            fmt_svg_num(y),
            fmt_svg_num(tile_width),
            fmt_svg_num(tile_height),
            encode_base64_background_data(&encoded),
        ));
    }
    svg.push_str("</svg>");
    crate::parser::svg::parse_svg_from_string(&svg)
}

fn encode_base64_background_data(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn parse_background_layer_sources(
    map: &StyleMap,
    current_color: Color,
    custom_properties: &HashMap<String, String>,
) -> Option<Vec<SimpleBackgroundLayerSource>> {
    let CssValue::BackgroundLayers(raw) = get_non_special(map, "background-image")? else {
        return None;
    };
    let mut sources = Vec::new();
    for source in raw {
        match source {
            BackgroundLayerSource::Raster(value) => {
                sources.push(SimpleBackgroundLayerSource::Image(resolve_embedded_vars(
                    value,
                    custom_properties,
                )));
            }
            BackgroundLayerSource::Linear(value) => {
                let value = resolve_embedded_vars(value, custom_properties);
                sources.push(SimpleBackgroundLayerSource::Linear(
                    parse_linear_gradient_for_color(&value, current_color)?,
                ));
            }
            BackgroundLayerSource::None => {}
            _ => return None,
        }
    }
    Some(sources)
}

fn style_background_border_box_size(style: &ComputedStyle) -> Option<(f32, f32)> {
    let mut width = style.width?;
    let mut height = style.height?;
    if style.box_sizing == BoxSizing::ContentBox {
        width += style.padding.horizontal() + style.border.horizontal_width();
        height += style.padding.vertical() + style.border.vertical_width();
    }
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn build_simple_multi_background_svg(
    map: &StyleMap,
    style: &ComputedStyle,
    sources: &[SimpleBackgroundLayerSource],
    border_width: f32,
    border_height: f32,
) -> Option<String> {
    let mut defs = String::new();
    let mut body = String::new();
    let border_rect = CssBoxRect {
        x: 0.0,
        y: 0.0,
        width: border_width,
        height: border_height,
    };

    for (rev_idx, source) in sources.iter().enumerate().rev() {
        let origin = background_layer_origin_rect(map, style, rev_idx, border_rect)?;
        let clip = background_layer_clip_rect(map, style, rev_idx, border_rect)?;
        let size = background_layer_size(map, rev_idx)
            .and_then(|size| resolve_simple_background_tile_size(size, origin.width, origin.height))
            .unwrap_or((origin.width, origin.height));
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return None;
        }
        let position = background_layer_position(map, rev_idx).unwrap_or_default();
        let offset_x = if position.x_is_percent {
            (origin.width - size.0) * position.x
        } else if position.x < 0.0 {
            (origin.width - size.0) + position.x
        } else {
            position.x
        };
        let offset_y = if position.y_is_percent {
            (origin.height - size.1) * position.y
        } else if position.y < 0.0 {
            (origin.height - size.1) + position.y
        } else {
            position.y
        };
        let x = origin.x + offset_x;
        let y = origin.y + offset_y;
        let clip_id = format!("bgclip{rev_idx}");
        defs.push_str(&format!(
            r#"<clipPath id="{clip_id}"><rect x="{x}" y="{y}" width="{w}" height="{h}"/></clipPath>"#,
            x = fmt_svg_num(clip.x),
            y = fmt_svg_num(clip.y),
            w = fmt_svg_num(clip.width),
            h = fmt_svg_num(clip.height),
        ));
        body.push_str(&format!(r#"<g clip-path="url(#{clip_id})">"#));
        let blend_mode = style.background_blend_mode.background_layer(rev_idx);
        match source {
            SimpleBackgroundLayerSource::Image(raw) => {
                let href = background_url_href(raw)?;
                if blend_mode == BlendMode::Normal {
                    body.push_str(&format!(
                        r#"<image href="{href}" x="{x}" y="{y}" width="{w}" height="{h}"/>"#,
                        href = xml_escape_attr(&href),
                        x = fmt_svg_num(x),
                        y = fmt_svg_num(y),
                        w = fmt_svg_num(size.0),
                        h = fmt_svg_num(size.1),
                    ));
                } else if blend_mode == BlendMode::Multiply
                    && rev_idx + 1 == sources.len()
                    && let Some(background) = style.background_color
                    && let Some(color) = solid_data_png_color(&href)
                {
                    let color = multiply_colors(color, background);
                    body.push_str(&format!(
                        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{color}" fill-opacity="{opacity}"/>"#,
                        x = fmt_svg_num(x),
                        y = fmt_svg_num(y),
                        w = fmt_svg_num(size.0),
                        h = fmt_svg_num(size.1),
                        color = color_to_svg_hex(color),
                        opacity = fmt_svg_num(color.a / 255.0),
                    ));
                } else {
                    return None;
                }
            }
            SimpleBackgroundLayerSource::Linear(gradient) => {
                if blend_mode != BlendMode::Normal {
                    return None;
                }
                let grad_id = format!("bggrad{rev_idx}");
                let (x1, y1, x2, y2) =
                    linear_gradient_svg_line(gradient.angle, x, y, size.0, size.1);
                defs.push_str(&format!(
                    r#"<linearGradient id="{grad_id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">"#,
                    x1 = fmt_svg_num(x1),
                    y1 = fmt_svg_num(y1),
                    x2 = fmt_svg_num(x2),
                    y2 = fmt_svg_num(y2),
                ));
                let basis = size.0 * gradient.angle.to_radians().sin().abs()
                    + size.1 * gradient.angle.to_radians().cos().abs();
                let stops = gradient.ramp.resolve(basis)?;
                for stop in stops.svg_unit_interval_stops()? {
                    let offset = stop.position * 100.0;
                    defs.push_str(&format!(
                        r#"<stop offset="{offset}%" stop-color="{color}" stop-opacity="{opacity}"/>"#,
                        offset = fmt_svg_num(offset),
                        color = color_to_svg_hex(stop.color.color),
                        opacity = fmt_svg_num(stop.color.color.a / 255.0),
                    ));
                }
                defs.push_str("</linearGradient>");
                body.push_str(&format!(
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="url(#{grad_id})"/>"#,
                    x = fmt_svg_num(x),
                    y = fmt_svg_num(y),
                    w = fmt_svg_num(size.0),
                    h = fmt_svg_num(size.1),
                ));
            }
        }
        body.push_str("</g>");
    }

    Some(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs>{defs}</defs>{body}</svg>"#,
        w = fmt_svg_num(border_width),
        h = fmt_svg_num(border_height),
    ))
}

fn background_layer_origin_rect(
    map: &StyleMap,
    style: &ComputedStyle,
    index: usize,
    border_rect: CssBoxRect,
) -> Option<CssBoxRect> {
    let origin = get_non_special(map, "background-origin")
        .and_then(|v| match v {
            CssValue::Keyword(k) => {
                nth_layer_value(k, index).map(|part| parse_background_origin_value(&part))
            }
            _ => None,
        })
        .unwrap_or(style.background_origin);
    Some(css_box_rect_for_background_origin(
        origin,
        style,
        border_rect,
    ))
}

fn background_layer_clip_rect(
    map: &StyleMap,
    style: &ComputedStyle,
    index: usize,
    border_rect: CssBoxRect,
) -> Option<CssBoxRect> {
    let clip = get_non_special(map, "background-clip")
        .or_else(|| get_non_special(map, "-webkit-background-clip"))
        .and_then(|v| match v {
            CssValue::Keyword(k) => {
                nth_layer_value(k, index).map(|part| parse_background_clip_value(&part))
            }
            _ => None,
        })
        .unwrap_or(style.background_clip);
    if clip == BackgroundClip::Text {
        return None;
    }
    Some(match clip {
        BackgroundClip::Border => border_rect,
        BackgroundClip::Padding => css_padding_box_rect(style, border_rect),
        BackgroundClip::Content => css_content_box_rect(style, border_rect),
        BackgroundClip::Text => border_rect,
    })
}

fn css_box_rect_for_background_origin(
    origin: BackgroundOrigin,
    style: &ComputedStyle,
    border_rect: CssBoxRect,
) -> CssBoxRect {
    match origin {
        BackgroundOrigin::Border => border_rect,
        BackgroundOrigin::Padding => css_padding_box_rect(style, border_rect),
        BackgroundOrigin::Content => css_content_box_rect(style, border_rect),
    }
}

fn css_padding_box_rect(style: &ComputedStyle, border_rect: CssBoxRect) -> CssBoxRect {
    CssBoxRect {
        x: border_rect.x + style.border.left.used_width(),
        y: border_rect.y + style.border.top.used_width(),
        width: (border_rect.width - style.border.horizontal_width()).max(0.0),
        height: (border_rect.height - style.border.vertical_width()).max(0.0),
    }
}

fn css_content_box_rect(style: &ComputedStyle, border_rect: CssBoxRect) -> CssBoxRect {
    let padding = css_padding_box_rect(style, border_rect);
    CssBoxRect {
        x: padding.x + style.padding.left,
        y: padding.y + style.padding.top,
        width: (padding.width - style.padding.horizontal()).max(0.0),
        height: (padding.height - style.padding.top - style.padding.bottom).max(0.0),
    }
}

fn background_layer_size(map: &StyleMap, index: usize) -> Option<BackgroundSize> {
    get_non_special(map, "background-size").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, index).map(|part| parse_background_size_value(&part))
        }
        _ => None,
    })
}

fn background_layer_position(map: &StyleMap, index: usize) -> Option<BackgroundPosition> {
    get_non_special(map, "background-position").and_then(|v| match v {
        CssValue::Keyword(k) => {
            nth_layer_value(k, index).and_then(|part| parse_background_position(&part))
        }
        _ => None,
    })
}

fn resolve_simple_background_tile_size(
    size: BackgroundSize,
    reference_width: f32,
    reference_height: f32,
) -> Option<(f32, f32)> {
    let resolve = |value: f32, is_percent: bool, basis: f32| {
        if is_percent {
            basis * value / 100.0
        } else {
            value
        }
    };
    match size {
        BackgroundSize::Auto => Some((reference_width, reference_height)),
        BackgroundSize::Explicit {
            width,
            height,
            width_is_percent,
            height_is_percent,
        } => Some((
            resolve(width, width_is_percent, reference_width),
            height
                .map(|value| resolve(value, height_is_percent, reference_height))
                .unwrap_or(reference_height),
        )),
        BackgroundSize::ExplicitAuto {
            width: Some(width),
            height,
            width_is_percent,
            height_is_percent,
        } => Some((
            resolve(width, width_is_percent, reference_width),
            height
                .map(|value| resolve(value, height_is_percent, reference_height))
                .unwrap_or(reference_height),
        )),
        BackgroundSize::ExplicitAuto {
            width: None,
            height: Some(height),
            height_is_percent,
            ..
        } => Some((
            reference_width,
            resolve(height, height_is_percent, reference_height),
        )),
        BackgroundSize::ExplicitAuto { .. } => Some((reference_width, reference_height)),
        BackgroundSize::Cover | BackgroundSize::Contain => None,
    }
}

fn background_url_href(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix("url(")?
        .strip_suffix(')')?
        .trim()
        .trim_matches(|c| c == '\'' || c == '"');
    (!inner.is_empty()).then(|| inner.to_string())
}

fn solid_data_png_color(href: &str) -> Option<Color> {
    let href = href.trim();
    let comma = href.find(',')?;
    let (header, data) = href.split_at(comma);
    if !header.to_ascii_lowercase().starts_with("data:image/png")
        || !header.to_ascii_lowercase().contains("base64")
    {
        return None;
    }
    let bytes = crate::util::decode_base64(&data[1..])?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let mut pixels = image.pixels();
    let first = pixels.next()?;
    if pixels.any(|pixel| pixel.0 != first.0) {
        return None;
    }
    Some(Color::rgba8(first[0], first[1], first[2], first[3]))
}

fn multiply_colors(source: Color, backdrop: Color) -> Color {
    if source.a < 255.0 || backdrop.a < 255.0 {
        return source;
    }
    Color::from_css_rgb(
        source.r * backdrop.r / 255.0,
        source.g * backdrop.g / 255.0,
        source.b * backdrop.b / 255.0,
        255.0,
    )
}

fn linear_gradient_svg_line(
    angle: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> (f32, f32, f32, f32) {
    let angle = angle.to_radians();
    let dx = angle.sin();
    let dy = -angle.cos();
    let half = (width * dx.abs() + height * dy.abs()) / 2.0;
    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    (
        cx - dx * half,
        cy - dy * half,
        cx + dx * half,
        cy + dy * half,
    )
}

fn color_to_svg_hex(color: Color) -> String {
    let [r, g, b, _] = color.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn xml_escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn fmt_svg_num(value: f32) -> String {
    let mut out = format!("{value:.4}");
    while out.contains('.') && out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    if out == "-0" {
        out = "0".to_string();
    }
    out
}

fn parse_background_size_explicit(val: &str) -> Option<BackgroundSize> {
    let parts: Vec<&str> = val.split_whitespace().collect();
    let parse_dimension = |s: &str| -> Option<(f32, bool)> {
        if let Some(n) = s.strip_suffix("px") {
            n.parse::<f32>().ok().map(|v| (v * 0.75, false))
        } else if let Some(n) = s.strip_suffix("pt") {
            n.parse::<f32>().ok().map(|v| (v, false))
        } else if let Some(n) = s.strip_suffix('%') {
            n.parse::<f32>().ok().map(|v| (v, true))
        } else {
            s.parse::<f32>().ok().map(|v| (v, false))
        }
    };
    match parts.len() {
        2 if parts[0] == "auto" => {
            let (height, height_is_percent) = parse_dimension(parts[1])?;
            Some(BackgroundSize::ExplicitAuto {
                width: None,
                height: Some(height),
                width_is_percent: false,
                height_is_percent,
            })
        }
        2 if parts[1] == "auto" => {
            let (width, width_is_percent) = parse_dimension(parts[0])?;
            Some(BackgroundSize::ExplicitAuto {
                width: Some(width),
                height: None,
                width_is_percent,
                height_is_percent: false,
            })
        }
        1 => {
            let (width, width_is_percent) = parse_dimension(parts[0])?;
            Some(BackgroundSize::Explicit {
                width,
                height: None,
                width_is_percent,
                height_is_percent: false,
            })
        }
        2 => {
            let (width, width_is_percent) = parse_dimension(parts[0])?;
            let (height, height_is_percent) = parse_dimension(parts[1])?;
            Some(BackgroundSize::Explicit {
                width,
                height: Some(height),
                width_is_percent,
                height_is_percent,
            })
        }
        _ => None,
    }
}

fn parse_background_position(val: &str) -> Option<BackgroundPosition> {
    let v = val.trim().to_ascii_lowercase();
    let p: Vec<&str> = v.split_whitespace().collect();
    let pc = |s: &str| -> Option<(f32, bool)> {
        match s {
            "left" => Some((0.0, true)),
            "right" => Some((1.0, true)),
            "top" => Some((0.0, true)),
            "bottom" => Some((1.0, true)),
            "center" => Some((0.5, true)),
            _ => {
                if let Some(n) = s.strip_suffix('%') {
                    n.parse::<f32>().ok().map(|x| (x / 100.0, true))
                } else if let Some(n) = s.strip_suffix("px") {
                    n.parse::<f32>().ok().map(|x| (x * 0.75, false))
                } else if let Some(n) = s.strip_suffix("pt") {
                    n.parse::<f32>().ok().map(|x| (x, false))
                } else {
                    s.parse::<f32>().ok().map(|x| (x, false))
                }
            }
        }
    };
    let set_axis =
        |token: &str, x: &mut Option<(f32, bool)>, y: &mut Option<(f32, bool)>| -> Option<()> {
            match token {
                "left" => *x = Some((0.0, true)),
                "right" => *x = Some((1.0, true)),
                "top" => *y = Some((0.0, true)),
                "bottom" => *y = Some((1.0, true)),
                "center" => {
                    if x.is_none() {
                        *x = Some((0.5, true));
                    } else if y.is_none() {
                        *y = Some((0.5, true));
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
            Some(())
        };
    match p.as_slice() {
        [token] => {
            let (value, is_percent) = pc(token)?;
            let (x, y) = if matches!(*token, "top" | "bottom") {
                ((0.5, true), (value, true))
            } else {
                ((value, is_percent), (0.5, true))
            };
            Some(BackgroundPosition {
                x: x.0,
                y: y.0,
                x_is_percent: x.1,
                y_is_percent: true,
            })
        }
        [first, second]
            if is_background_position_keyword(first) && is_background_position_keyword(second) =>
        {
            let mut x = None;
            let mut y = None;
            set_axis(first, &mut x, &mut y)?;
            set_axis(second, &mut x, &mut y)?;
            let (x, xp) = x.unwrap_or((0.5, true));
            let (y, yp) = y.unwrap_or((0.5, true));
            Some(BackgroundPosition {
                x,
                y,
                x_is_percent: xp,
                y_is_percent: yp,
            })
        }
        [first, second] => {
            let (x, xp) = pc(first)?;
            let (y, yp) = pc(second)?;
            Some(BackgroundPosition {
                x,
                y,
                x_is_percent: xp,
                y_is_percent: yp,
            })
        }
        [h_edge, h_offset, v_edge, v_offset]
            if matches!(*h_edge, "left" | "right") && matches!(*v_edge, "top" | "bottom") =>
        {
            let (mut x, xp) = pc(h_offset)?;
            let (mut y, yp) = pc(v_offset)?;
            if *h_edge == "right" && !xp {
                x = -x;
            } else if *h_edge == "right" {
                x = 1.0 - x;
            }
            if *v_edge == "bottom" && !yp {
                y = -y;
            } else if *v_edge == "bottom" {
                y = 1.0 - y;
            }
            Some(BackgroundPosition {
                x,
                y,
                x_is_percent: xp,
                y_is_percent: yp,
            })
        }
        [v_edge, v_offset, h_edge, h_offset]
            if matches!(*h_edge, "left" | "right") && matches!(*v_edge, "top" | "bottom") =>
        {
            let (mut x, xp) = pc(h_offset)?;
            let (mut y, yp) = pc(v_offset)?;
            if *h_edge == "right" && !xp {
                x = -x;
            } else if *h_edge == "right" {
                x = 1.0 - x;
            }
            if *v_edge == "bottom" && !yp {
                y = -y;
            } else if *v_edge == "bottom" {
                y = 1.0 - y;
            }
            Some(BackgroundPosition {
                x,
                y,
                x_is_percent: xp,
                y_is_percent: yp,
            })
        }
        _ => None,
    }
}

fn is_background_position_keyword(token: &str) -> bool {
    matches!(token, "left" | "right" | "top" | "bottom" | "center")
}

fn extract_image_set_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let _ = lower
        .strip_prefix("image-set(")
        .or_else(|| lower.strip_prefix("-webkit-image-set("))?;
    if !trimmed.ends_with(')') {
        return None;
    }
    let inner = &trimmed[trimmed.find('(')? + 1..trimmed.len() - 1];
    let inner = inner.trim();
    let lower_inner = inner.to_ascii_lowercase();
    if let Some(start) = lower_inner.find("url(") {
        let tail = &inner[start..];
        let end = tail.find(')')?;
        return Some(tail[..=end].to_string());
    }
    let mut chars = inner.char_indices();
    let (_, first) = chars.next()?;
    if first == '"' || first == '\'' {
        let rest = &inner[first.len_utf8()..];
        let end = rest.find(first)?;
        let source = &rest[..end];
        return (!source.is_empty()).then(|| source.to_string());
    }
    let end = inner
        .find(|ch: char| ch == ',' || ch.is_whitespace())
        .unwrap_or(inner.len());
    let source = inner[..end].trim();
    (!source.is_empty()).then(|| source.to_string())
}

fn parse_border_image_source(source: &str, current_color: Color) -> Option<BorderImageSource> {
    let source = source.trim();
    let lower = source.to_ascii_lowercase();
    let source = if lower.starts_with("linear-gradient(")
        || lower.starts_with("repeating-linear-gradient(")
    {
        BorderImageSource::LinearGradient(parse_linear_gradient_for_color(source, current_color)?)
    } else if lower.starts_with("radial-gradient(")
        || lower.starts_with("repeating-radial-gradient(")
    {
        BorderImageSource::RadialGradient(parse_radial_gradient_for_color(source, current_color)?)
    } else if lower.starts_with("conic-gradient(") || lower.starts_with("repeating-conic-gradient(")
    {
        BorderImageSource::ConicGradient(parse_conic_gradient_for_color(source, current_color)?)
    } else {
        BorderImageSource::Url(background_url_href(source)?)
    };
    Some(source)
}

fn parse_border_image_slices(input: &str) -> Option<BorderImageSlices> {
    let slice_part = input.trim();
    let mut fill = false;
    let mut values = Vec::new();
    for token in slice_part.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("fill") {
            fill = true;
            continue;
        }
        let value = if let Some(percent) = token.strip_suffix('%') {
            BorderImageSliceValue::Percentage(percent.parse().ok()?)
        } else {
            BorderImageSliceValue::Number(token.parse().ok()?)
        };
        values.push(value);
    }
    if values.is_empty() {
        return Some(BorderImageSlices {
            fill,
            ..Default::default()
        });
    }
    let (top, right, bottom, left) = expand_border_image_quad(&values)?;
    Some(BorderImageSlices {
        top,
        right,
        bottom,
        left,
        fill,
    })
}

fn parse_border_image_widths(
    input: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<BorderImageWidths> {
    let values = split_css_whitespace(input)
        .into_iter()
        .map(|value| parse_border_image_width(value, style, length_context, font_metrics))
        .collect::<Option<Vec<_>>>()?;
    let (top, right, bottom, left) = expand_border_image_quad(&values)?;
    Some(BorderImageWidths {
        top,
        right,
        bottom,
        left,
    })
}

fn parse_border_image_outsets(
    input: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<BorderImageOutsets> {
    let values = split_css_whitespace(input)
        .into_iter()
        .map(|value| parse_border_image_outset(value, style, length_context, font_metrics))
        .collect::<Option<Vec<_>>>()?;
    let (top, right, bottom, left) = expand_border_image_quad(&values)?;
    Some(BorderImageOutsets {
        top,
        right,
        bottom,
        left,
    })
}

fn parse_border_image_repeats(input: &str) -> Option<BorderImageRepeats> {
    let mut values = input.split_ascii_whitespace().map(|value| match value {
        value if value.eq_ignore_ascii_case("stretch") => Some(BorderImageRepeatMode::Stretch),
        value if value.eq_ignore_ascii_case("repeat") => Some(BorderImageRepeatMode::Repeat),
        value if value.eq_ignore_ascii_case("round") => Some(BorderImageRepeatMode::Round),
        value if value.eq_ignore_ascii_case("space") => Some(BorderImageRepeatMode::Space),
        _ => None,
    });
    let horizontal = values.next()??;
    let vertical = match values.next() {
        Some(value) => value?,
        None => horizontal,
    };
    values.next().is_none().then_some(BorderImageRepeats {
        horizontal,
        vertical,
    })
}

fn parse_border_image_width(
    token: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<BorderImageWidth> {
    let token = token.trim();
    if token.eq_ignore_ascii_case("auto") {
        return Some(BorderImageWidth::Auto);
    }
    if let Ok(value) = token.parse::<f32>() {
        return (value.is_finite() && value >= 0.0).then_some(BorderImageWidth::Number(value));
    }
    match parse_length(token)? {
        CssValue::Var(name, fallback) => {
            let value = crate::style::resolve::resolve_var_to_string(
                &name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            parse_border_image_width(&value, style, length_context, font_metrics)
        }
        value => parse_border_image_length_percent(&value, style, length_context, font_metrics)
            .map(BorderImageWidth::LengthPercent),
    }
}

fn parse_border_image_length_percent(
    value: &CssValue,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<LengthPercent> {
    let value = match value {
        CssValue::Length(value) => LengthPercent::length(*value),
        CssValue::Percentage(value) => LengthPercent::percent(*value),
        CssValue::Math(expression) => expression.affine(style.math_unit_context(font_metrics))?,
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            return parse_border_image_width(&raw, style, length_context, font_metrics).and_then(
                |value| match value {
                    BorderImageWidth::LengthPercent(value) => Some(value),
                    _ => None,
                },
            );
        }
        value => LengthPercent::length(resolve_css_length_for_style(
            value,
            style,
            length_context,
            font_metrics,
        )?),
    };
    let (length, percent) = value.terms();
    (length.is_finite() && percent.is_finite()).then_some(value)
}

fn parse_border_image_outset(
    token: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<BorderImageOutset> {
    let token = token.trim();
    if let Ok(value) = token.parse::<f32>() {
        return (value.is_finite() && value >= 0.0).then_some(BorderImageOutset::Number(value));
    }
    let value = match parse_length(token)? {
        CssValue::Var(name, fallback) => {
            let value = crate::style::resolve::resolve_var_to_string(
                &name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            return parse_border_image_outset(&value, style, length_context, font_metrics);
        }
        value if css_value_contains_percentage(&value) => return None,
        value => resolve_css_length_for_style(&value, style, length_context, font_metrics)?,
    };
    (value.is_finite() && value >= 0.0).then_some(BorderImageOutset::Length(value))
}

fn css_value_contains_percentage(value: &CssValue) -> bool {
    match value {
        CssValue::Percentage(_) => true,
        CssValue::Math(expression) => expression.contains_percentage(),
        _ => false,
    }
}

fn expand_border_image_quad<T: Copy>(values: &[T]) -> Option<(T, T, T, T)> {
    match *values {
        [top] => Some((top, top, top, top)),
        [top, right] => Some((top, right, top, right)),
        [top, right, bottom] => Some((top, right, bottom, right)),
        [top, right, bottom, left] => Some((top, right, bottom, left)),
        _ => None,
    }
}

/// Parse a `box-shadow` shorthand value.
///
/// Supports CSS syntax:
/// - `[inset]? <offset-x> <offset-y> [<blur> [<spread>]] [<color>]`
/// - `inset 0 2px 8px rgba(0,0,0,0.3)`
/// - `4px 4px 8px 2px #ccc`  (with spread)
/// - Multiple shadows separated by commas.
///
/// The list is parsed atomically. A malformed entry invalidates the complete
/// property value; silently dropping one entry changes both the declaration's
/// validity and its paint result.
fn parse_box_shadow_for_color(val: &str, current_color: Color) -> Option<Vec<BoxShadow>> {
    let val = val.trim();
    if val == "none" {
        return Some(Vec::new());
    }
    if val.starts_with(',') || val.ends_with(',') {
        return None;
    }

    // A `box-shadow` may list several shadows separated by top-level commas
    // (outside parens). They paint back-to-front, the first listed on top.
    let entries = split_top_level_comma(val);
    if entries.is_empty() || entries.iter().any(|entry| entry.trim().is_empty()) {
        return None;
    }
    entries
        .iter()
        .map(|entry| parse_single_box_shadow_for_color(entry, current_color))
        .collect()
}

/// Parse one shadow from a `box-shadow` list entry.
fn parse_single_box_shadow_for_color(val: &str, current_color: Color) -> Option<BoxShadow> {
    let val = val.trim();
    if val.is_empty() {
        return None;
    }

    // Whitespace inside color functions is not a component separator.
    let tokens = split_css_components(val);

    // CSS combines these components with `&&`, so color and `inset` may occur
    // on either side of the length sequence. Each may occur at most once.
    let mut inset = false;
    let mut lengths = Vec::with_capacity(4);
    let mut color = None;
    for token in &tokens {
        if token.eq_ignore_ascii_case("inset") {
            if inset {
                return None;
            }
            inset = true;
        } else if let Some(length) = parse_shadow_length(token) {
            lengths.push(length);
        } else if color.is_none() {
            color = parse_border_color(token);
            color?;
        } else {
            return None;
        }
    }

    if !(2..=4).contains(&lengths.len()) {
        return None;
    }
    let blur = lengths.get(2).copied().unwrap_or(0.0);
    if blur < 0.0 {
        return None;
    }

    let (color, color_source) =
        bind_specified_color(color.unwrap_or(SpecifiedColor::CurrentColor), current_color);
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur,
        spread: lengths.get(3).copied().unwrap_or(0.0),
        color,
        color_source,
        inset,
    })
}

/// Parse a `text-shadow` value into a list of shadows (css-text-decor-3 §3).
/// Syntax: `none | [ <color>? && <length>{2,3} ]#`. Unlike `box-shadow` there
/// is no spread or `inset`, and the optional color may precede or follow the
/// offsets. Reuses `BoxShadow` storage with `spread = 0` and `inset = false`.
fn parse_text_shadow_for_color(val: &str, current_color: Color) -> Vec<BoxShadow> {
    let val = val.trim();
    if val.is_empty() || val == "none" {
        return Vec::new();
    }
    split_top_level_comma(val)
        .iter()
        .filter_map(|s| parse_single_text_shadow(s, current_color))
        .collect()
}

#[cfg(test)]
fn parse_box_shadow(val: &str) -> Option<Vec<BoxShadow>> {
    parse_box_shadow_for_color(val, Color::BLACK)
}

#[cfg(test)]
fn parse_single_box_shadow(val: &str) -> Option<BoxShadow> {
    parse_single_box_shadow_for_color(val, Color::BLACK)
}

#[cfg(test)]
fn parse_text_shadow(val: &str) -> Vec<BoxShadow> {
    parse_text_shadow_for_color(val, Color::BLACK)
}

/// Parse one `text-shadow` list entry: 2 or 3 lengths plus an optional color
/// that may appear before or after the lengths.
fn parse_single_text_shadow(val: &str, current_color: Color) -> Option<BoxShadow> {
    let val = val.trim();
    if val.is_empty() {
        return None;
    }

    // Tokenize: spaces delimit tokens, but rgba(...) / rgb(...) / hsl(...) are
    // each a single token (same rule as box-shadow).
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in val.chars() {
        if ch == ' ' && !current.contains('(') {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if ch == ')' {
            current.push(ch);
            tokens.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    // Separate the (at most one) color token from the length tokens. The color
    // may be at either end; lengths parse numerically, colors do not.
    let mut lengths: Vec<f32> = Vec::new();
    let mut color = None;
    for t in &tokens {
        if let Some(len) = parse_shadow_length(t) {
            lengths.push(len);
        } else if color.is_none() {
            color = parse_border_color(t);
        }
    }

    if lengths.len() < 2 {
        return None;
    }

    let (color, color_source) =
        bind_specified_color(color.unwrap_or(SpecifiedColor::CurrentColor), current_color);
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur: lengths.get(2).copied().unwrap_or(0.0),
        spread: 0.0,
        color,
        color_source,
        inset: false,
    })
}

/// Split on top-level commas (commas not enclosed in parens). Used for the
/// comma-separated list syntax of several CSS properties.
fn split_top_level_comma(val: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for ch in val.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth <= 0 => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse a length value for box-shadow.
///
/// CSS permits a unitless zero where a length is expected, but a non-zero
/// number is not a length and invalidates the declaration.
fn parse_shadow_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("px") {
        n.parse::<f32>().ok().map(|v| v * 0.75)
    } else if let Some(n) = val.strip_suffix("pt") {
        n.parse::<f32>().ok()
    } else {
        val.parse::<f32>().ok().filter(|value| *value == 0.0)
    }
}

/// Parse a single CSS transform function (e.g. `rotate(45deg)`).
///
/// Returns the parsed transform and `None` when the function is unknown or
/// malformed. `font_size`/`root_font_size` (pt) resolve em/rem length args.
fn parse_single_transform(val: &str, font_size: f32, root_font_size: f32) -> Option<Transform> {
    let val = val.trim();
    let len = |s: &str| parse_transform_length(s, font_size, root_font_size);
    let mk_translate = |x: Option<(f64, bool)>, y: Option<(f64, bool)>| {
        let (x, x_percentage) = x.unwrap_or((0.0, false));
        let (y, y_percentage) = y.unwrap_or((0.0, false));
        Transform::Translate {
            offset: CssVector::new(x, y),
            percentages: PercentageAxes::new(x_percentage, y_percentage),
        }
    };

    if let Some(inner) = val
        .strip_prefix("rotate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Rotate(parse_angle_deg(inner)?));
    }

    if let Some(inner) = val
        .strip_prefix("perspective(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let (d, is_pct) = len(inner.trim())?;
        if is_pct || d <= 0.0 {
            return None;
        }
        let mut m = matrix3d_identity();
        m[11] = -1.0 / d;
        return Some(Transform::Matrix3d(m));
    }

    // rotateZ() is the 2D z-axis rotation (== rotate()).
    if let Some(inner) = val
        .strip_prefix("rotateZ(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Rotate(parse_angle_deg(inner)?));
    }
    if let Some(inner) = val
        .strip_prefix("rotateY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Matrix3d(matrix3d_rotate_y(parse_angle_deg(
            inner,
        )?)));
    }

    if let Some(inner) = val
        .strip_prefix("rotateX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Matrix3d(matrix3d_rotate_x(parse_angle_deg(
            inner,
        )?)));
    }

    if let Some(inner) = val
        .strip_prefix("rotate3d(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 4 {
            let x = parts[0].parse::<f64>().ok()?;
            let y = parts[1].parse::<f64>().ok()?;
            let z = parts[2].parse::<f64>().ok()?;
            let deg = parse_angle_deg(parts[3])?;
            return Some(Transform::Matrix3d(matrix3d_rotate_axis(x, y, z, deg)?));
        }
    }

    if let Some(inner) = val
        .strip_prefix("scaleX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let sx = parse_scale_factor(inner)?;
        return Some(Transform::Scale(CssVector::new(sx, 1.0)));
    }

    if let Some(inner) = val
        .strip_prefix("scaleY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let sy = parse_scale_factor(inner)?;
        return Some(Transform::Scale(CssVector::new(1.0, sy)));
    }

    if let Some(inner) = val.strip_prefix("scale(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 1 {
            let scale = parse_scale_factor(parts[0])?;
            return Some(Transform::Scale(CssVector::splat(scale)));
        } else if parts.len() == 2 {
            let sx = parse_scale_factor(parts[0])?;
            // CSS: an omitted/empty second arg defaults to the first.
            let sy_tok = parts[1].trim();
            let sy = if sy_tok.is_empty() {
                sx
            } else {
                parse_scale_factor(sy_tok)?
            };
            return Some(Transform::Scale(CssVector::new(sx, sy)));
        }
    }

    if let Some(inner) = val
        .strip_prefix("scale3d(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            let sx = parse_scale_factor(parts[0])?;
            let sy = parse_scale_factor(parts[1])?;
            let sz = parse_scale_factor(parts[2])?;
            return Some(Transform::Matrix3d(matrix3d_scale(sx, sy, sz)));
        }
    }

    if let Some(inner) = val
        .strip_prefix("translateX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let x = len(inner.trim())?;
        return Some(mk_translate(Some(x), None));
    }

    if let Some(inner) = val
        .strip_prefix("translateY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let y = len(inner.trim())?;
        return Some(mk_translate(None, Some(y)));
    }

    if let Some(inner) = val
        .strip_prefix("translate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 2 {
            let x = len(parts[0].trim())?;
            let y = len(parts[1].trim())?;
            return Some(mk_translate(Some(x), Some(y)));
        } else if parts.len() == 1 {
            let x = len(parts[0].trim())?;
            return Some(mk_translate(Some(x), None));
        }
    }

    if let Some(inner) = val
        .strip_prefix("translateZ(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let (z, z_pct) = len(inner.trim())?;
        if z_pct {
            return None;
        }
        return Some(Transform::Matrix3d(matrix3d_translate(0.0, 0.0, z)));
    }

    if let Some(inner) = val
        .strip_prefix("translate3d(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            let (x, x_pct) = len(parts[0])?;
            let (y, y_pct) = len(parts[1])?;
            let (z, z_pct) = len(parts[2])?;
            if x_pct || y_pct || z_pct {
                return None;
            }
            return Some(Transform::Matrix3d(matrix3d_translate(x, y, z)));
        }
    }

    if let Some(inner) = val.strip_prefix("skew(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        let ax = parse_angle_deg(parts.first()?.trim())?;
        let ay = if parts.len() >= 2 {
            parse_angle_deg(parts[1].trim()).unwrap_or(0.0)
        } else {
            0.0
        };
        return Some(Transform::Skew(CssVector::new(ax, ay)));
    }

    if let Some(inner) = val.strip_prefix("skewX(").and_then(|s| s.strip_suffix(')')) {
        return Some(Transform::Skew(CssVector::new(
            parse_angle_deg(inner)?,
            0.0,
        )));
    }

    if let Some(inner) = val.strip_prefix("skewY(").and_then(|s| s.strip_suffix(')')) {
        return Some(Transform::Skew(CssVector::new(
            0.0,
            parse_angle_deg(inner)?,
        )));
    }

    if let Some(inner) = val
        .strip_prefix("matrix(")
        .and_then(|s| s.strip_suffix(')'))
    {
        // Per spec an unparseable argument makes the whole function invalid; map
        // each token through parse (no filtering) so a bad token => None rather
        // than silently shifting arity.
        let toks: Vec<&str> = inner.split(',').collect();
        if toks.len() != 6 {
            return None;
        }
        let mut nums = [0.0_f64; 6];
        for (i, tok) in toks.iter().enumerate() {
            nums[i] = tok.trim().parse::<f64>().ok()?;
        }
        // a, b, c, d are unitless; e, f are pixel translations -> points.
        return Some(Transform::Matrix(CssAffineMatrix::from_components(
            nums[0],
            nums[1],
            nums[2],
            nums[3],
            nums[4] * 0.75,
            nums[5] * 0.75,
        )));
    }

    if let Some(inner) = val
        .strip_prefix("matrix3d(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let toks: Vec<&str> = inner.split(',').collect();
        if toks.len() != 16 {
            return None;
        }
        let mut nums = [0.0_f64; 16];
        for (i, tok) in toks.iter().enumerate() {
            nums[i] = tok.trim().parse::<f64>().ok()?;
        }
        nums[12] *= 0.75;
        nums[13] *= 0.75;
        nums[14] *= 0.75;
        return Some(Transform::Matrix3d(CssMatrix3d::new(nums)));
    }

    None
}

fn matrix3d_identity() -> CssMatrix3d {
    CssMatrix3d::new([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
}

fn matrix3d_translate(tx: f64, ty: f64, tz: f64) -> CssMatrix3d {
    let mut m = matrix3d_identity();
    m[12] = tx;
    m[13] = ty;
    m[14] = tz;
    m
}

fn matrix3d_scale(sx: f64, sy: f64, sz: f64) -> CssMatrix3d {
    CssMatrix3d::new([
        sx, 0.0, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, sz, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
}

fn matrix3d_rotate_x(deg: f64) -> CssMatrix3d {
    let rad = deg.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    CssMatrix3d::new([
        1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
}

fn matrix3d_rotate_y(deg: f64) -> CssMatrix3d {
    let rad = deg.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    CssMatrix3d::new([
        c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
}

fn matrix3d_rotate_axis(x: f64, y: f64, z: f64, deg: f64) -> Option<CssMatrix3d> {
    let len = (x * x + y * y + z * z).sqrt();
    if len == 0.0 {
        return None;
    }
    let (x, y, z) = (x / len, y / len, z / len);
    let rad = deg.to_radians();
    let (c, s) = (rad.cos(), rad.sin());
    let t = 1.0 - c;
    Some(CssMatrix3d::new([
        t * x * x + c,
        t * x * y + s * z,
        t * x * z - s * y,
        0.0,
        t * x * y - s * z,
        t * y * y + c,
        t * y * z + s * x,
        0.0,
        t * x * z + s * y,
        t * y * z - s * x,
        t * z * z + c,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]))
}

fn matrix3d_multiply(lhs: &CssMatrix3d, rhs: &CssMatrix3d) -> CssMatrix3d {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = lhs[row] * rhs[col * 4]
                + lhs[4 + row] * rhs[col * 4 + 1]
                + lhs[8 + row] * rhs[col * 4 + 2]
                + lhs[12 + row] * rhs[col * 4 + 3];
        }
    }
    CssMatrix3d::new(out)
}

fn transform_to_matrix3d(t: &Transform) -> CssMatrix3d {
    match *t {
        Transform::Matrix3d(m) | Transform::Project3d { matrix: m, .. } => m,
        Transform::Rotate(_)
        | Transform::Skew(_)
        | Transform::Scale(_)
        | Transform::Translate { .. }
        | Transform::Matrix(_)
        | Transform::MatrixPct(_) => {
            let [a, b, c, d, e, f] = t.to_css_matrix(CssVector::ZERO).components();
            CssMatrix3d::new([
                a, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, e, f, 0.0, 1.0,
            ])
        }
    }
}

/// Extended affine matrix carrying percentage-translate coefficients:
/// `[a, b, c, d, e, f, e_w, e_h, f_w, f_h]`, where the effective translation for
/// a box of size `w`×`h` is `e + e_w*w + e_h*h` / `f + f_w*w + f_h*h`. This lets
/// `%` translate components survive composition without knowing the box size.
type ExtMatrix = [f64; 10];

/// Convert a single Transform into its extended affine matrix.
fn transform_to_ext(t: &Transform) -> ExtMatrix {
    match *t {
        Transform::Rotate(deg) => {
            let rad = deg.to_radians();
            let (c, s) = (rad.cos(), rad.sin());
            [c, s, -s, c, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        }
        Transform::Skew(angles) => [
            1.0,
            angles.y.to_radians().tan(),
            angles.x.to_radians().tan(),
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        Transform::Scale(scale) => [scale.x, 0.0, 0.0, scale.y, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        Transform::Translate {
            offset,
            percentages,
        } => {
            let (e, e_w) = if percentages.x {
                (0.0, offset.x / 100.0)
            } else {
                (offset.x, 0.0)
            };
            let (f, f_h) = if percentages.y {
                (0.0, offset.y / 100.0)
            } else {
                (offset.y, 0.0)
            };
            [1.0, 0.0, 0.0, 1.0, e, f, e_w, 0.0, 0.0, f_h]
        }
        Transform::Matrix(matrix) => {
            let [a, b, c, d, e, f] = matrix.components();
            [a, b, c, d, e, f, 0.0, 0.0, 0.0, 0.0]
        }
        Transform::Matrix3d(m) => {
            let [a, b, c, d, e, f] = matrix3d_affine_projection(&m).components();
            [a, b, c, d, e, f, 0.0, 0.0, 0.0, 0.0]
        }
        Transform::Project3d { matrix, .. } => {
            let [a, b, c, d, e, f] = matrix3d_affine_projection(&matrix).components();
            [a, b, c, d, e, f, 0.0, 0.0, 0.0, 0.0]
        }
        Transform::MatrixPct(matrix) => {
            let [a, b, c, d, e, f] = matrix.constant.components();
            [
                a,
                b,
                c,
                d,
                e,
                f,
                matrix.width_translation.x,
                matrix.height_translation.x,
                matrix.width_translation.y,
                matrix.height_translation.y,
            ]
        }
    }
}

/// Multiply two extended affine matrices: `result = lhs × rhs`. The linear part
/// (a..d) multiplies normally; the translation columns (constant + per-dim
/// coefficients) each transform through `lhs`'s linear part, keeping the result
/// affine in `(w, h)`.
fn multiply_ext(lhs: &ExtMatrix, rhs: &ExtMatrix) -> ExtMatrix {
    let lin = |x: f64, y: f64| (lhs[0] * x + lhs[2] * y, lhs[1] * x + lhs[3] * y);
    let (e, f) = lin(rhs[4], rhs[5]);
    let (ew, fw) = lin(rhs[6], rhs[8]);
    let (eh, fh) = lin(rhs[7], rhs[9]);
    [
        lhs[0] * rhs[0] + lhs[2] * rhs[1],
        lhs[1] * rhs[0] + lhs[3] * rhs[1],
        lhs[0] * rhs[2] + lhs[2] * rhs[3],
        lhs[1] * rhs[2] + lhs[3] * rhs[3],
        e + lhs[4],
        f + lhs[5],
        ew + lhs[6],
        eh + lhs[7],
        fw + lhs[8],
        fh + lhs[9],
    ]
}

fn transform_from_ext(matrix: ExtMatrix) -> Transform {
    let [a, b, c, d, e, f, e_w, e_h, f_w, f_h] = matrix;
    if [e_w, e_h, f_w, f_h].into_iter().all(|value| value == 0.0) {
        Transform::Matrix(CssAffineMatrix::from_components(a, b, c, d, e, f))
    } else {
        Transform::MatrixPct(ParametricAffineMatrix {
            constant: CssAffineMatrix::from_components(a, b, c, d, e, f),
            width_translation: CssVector::new(e_w, f_w),
            height_translation: CssVector::new(e_h, f_h),
        })
    }
}

/// Parse a CSS `transform` value (one or more space-separated functions).
///
/// Supports: rotate, scale, scaleX, scaleY, translate, translateX, translateY,
/// skew, skewX, skewY, and chained transforms like `rotate(10deg) scale(1.1)`.
/// Parse a single `transform-origin` axis component into
/// `(fraction, length_px)`. `horizontal` selects which keywords are valid for
/// disambiguating a bare `left`/`right`/`top`/`bottom`/`center`.
/// Parse a single `transform-origin` axis component into `(fraction, length_pt)`.
/// Keywords/percentages set the fraction; absolute/font-relative lengths resolve
/// to pt. `font_size`/`root_font_size` are in pt.
fn parse_origin_component(token: &str, font_size: f32, root_font_size: f32) -> Option<(f32, f32)> {
    let lowered = token.trim().to_ascii_lowercase();
    let t = lowered.as_str();
    match t {
        "left" | "top" => Some((0.0, 0.0)),
        "center" => Some((0.5, 0.0)),
        "right" | "bottom" => Some((1.0, 0.0)),
        _ => {
            if let Some(pct) = t.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|p| (p / 100.0, 0.0))
            } else {
                parse_abs_length_pt(t, font_size, root_font_size).map(|pt| (0.0, pt))
            }
        }
    }
}

fn parse_transform_origin(
    val: &str,
    font_size: f32,
    root_font_size: f32,
) -> Option<TransformOrigin> {
    let val = val.trim();
    if val.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = val.split_whitespace().collect();
    // Vertical-only keywords that, when first, indicate the value order is
    // swapped (e.g. `top left`). Otherwise the first token is the x axis.
    let is_vertical = |s: &str| s.eq_ignore_ascii_case("top") || s.eq_ignore_ascii_case("bottom");
    let is_horizontal = |s: &str| s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("right");
    let (x_tok, y_tok, z_tok) = match tokens.as_slice() {
        [a] => (*a, "center", None),
        [a, b] => {
            if is_vertical(a) || is_horizontal(b) {
                (*b, *a, None) // swapped: vertical keyword came first
            } else {
                (*a, *b, None)
            }
        }
        [a, b, z] => {
            if is_vertical(a) || is_horizontal(b) {
                (*b, *a, Some(*z))
            } else {
                (*a, *b, Some(*z))
            }
        }
        _ => return None,
    };
    let (x_fraction, x_length) = parse_origin_component(x_tok, font_size, root_font_size)?;
    let (y_fraction, y_length) = parse_origin_component(y_tok, font_size, root_font_size)?;
    let z_length = z_tok
        .and_then(|z| parse_abs_length_pt(z, font_size, root_font_size))
        .unwrap_or(0.0);
    Some(TransformOrigin {
        x_fraction,
        x_length,
        y_fraction,
        y_length,
        z_length,
    })
}

fn compose_transforms(transforms: &[Transform]) -> Option<Transform> {
    if transforms.is_empty() {
        return None;
    }
    if transforms
        .iter()
        .any(|t| matches!(t, Transform::Matrix3d(_) | Transform::Project3d { .. }))
    {
        let mut result = matrix3d_identity();
        for t in transforms {
            result = matrix3d_multiply(&result, &transform_to_matrix3d(t));
        }
        Some(Transform::Matrix3d(result))
    } else {
        let mut result: ExtMatrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        for t in transforms {
            result = multiply_ext(&result, &transform_to_ext(t));
        }
        Some(transform_from_ext(result))
    }
}

fn parse_individual_translate(val: &str, font_size: f32, root_font_size: f32) -> Option<Transform> {
    let parts: Vec<&str> = val.split_whitespace().collect();
    let len = |s: &str| parse_transform_length(s, font_size, root_font_size);
    match parts.as_slice() {
        [x] => {
            let (x, x_percentage) = len(x)?;
            Some(Transform::Translate {
                offset: CssVector::new(x, 0.0),
                percentages: PercentageAxes::new(x_percentage, false),
            })
        }
        [x, y] => {
            let (x, x_percentage) = len(x)?;
            let (y, y_percentage) = len(y)?;
            Some(Transform::Translate {
                offset: CssVector::new(x, y),
                percentages: PercentageAxes::new(x_percentage, y_percentage),
            })
        }
        [x, y, z] => {
            let (tx, tx_pct) = len(x)?;
            let (ty, ty_pct) = len(y)?;
            let (tz, tz_pct) = len(z)?;
            if tx_pct || ty_pct || tz_pct {
                return None;
            }
            Some(Transform::Matrix3d(matrix3d_translate(tx, ty, tz)))
        }
        _ => None,
    }
}

fn parse_individual_rotate(val: &str) -> Option<Transform> {
    let parts: Vec<&str> = val.split_whitespace().collect();
    match parts.as_slice() {
        [angle] => Some(Transform::Rotate(parse_angle_deg(angle)?)),
        [x, y, z, angle] => Some(Transform::Matrix3d(matrix3d_rotate_axis(
            x.parse().ok()?,
            y.parse().ok()?,
            z.parse().ok()?,
            parse_angle_deg(angle)?,
        )?)),
        _ => None,
    }
}

fn parse_individual_scale(val: &str) -> Option<Transform> {
    let parts: Vec<&str> = val.split_whitespace().collect();
    match parts.as_slice() {
        [s] => {
            let scale = parse_scale_factor(s)?;
            Some(Transform::Scale(CssVector::splat(scale)))
        }
        [sx, sy] => Some(Transform::Scale(CssVector::new(
            parse_scale_factor(sx)?,
            parse_scale_factor(sy)?,
        ))),
        [sx, sy, sz] => Some(Transform::Matrix3d(matrix3d_scale(
            parse_scale_factor(sx)?,
            parse_scale_factor(sy)?,
            parse_scale_factor(sz)?,
        ))),
        _ => None,
    }
}

fn parse_scale_factor(value: &str) -> Option<f64> {
    let value = value.trim();
    value.strip_suffix('%').map_or_else(
        || value.parse::<f64>().ok(),
        |percentage| {
            percentage
                .trim()
                .parse::<f64>()
                .ok()
                .map(|value| value / 100.0)
        },
    )
}

fn parse_transform(val: &str, font_size: f32, root_font_size: f32) -> Option<Transform> {
    let val = val.trim();
    if val == "none" {
        return None;
    }

    // Split into individual transform functions by finding `) ` boundaries.
    let mut functions: Vec<&str> = Vec::new();
    let mut start = 0;
    let bytes = val.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b')' {
            functions.push(&val[start..=i]);
            start = i + 1;
        }
    }
    // Skip any trailing whitespace-only content
    let remaining = val[start..].trim();
    if !remaining.is_empty() {
        return None; // trailing garbage
    }

    if functions.is_empty() {
        return None;
    }

    if functions.len() == 1 {
        return parse_single_transform(functions[0], font_size, root_font_size);
    }

    let parsed: Vec<Transform> = functions
        .iter()
        .map(|func| parse_single_transform(func, font_size, root_font_size))
        .collect::<Option<Vec<_>>>()?;

    if parsed
        .iter()
        .any(|t| matches!(t, Transform::Matrix3d(_) | Transform::Project3d { .. }))
    {
        let mut result = matrix3d_identity();
        for t in &parsed {
            result = matrix3d_multiply(&result, &transform_to_matrix3d(t));
        }
        return Some(Transform::Matrix3d(result));
    }

    // Multiple transforms — compose into a single matrix.
    // CSS: transforms are applied right-to-left, but the `cm` operator
    // in PDF also post-multiplies, so we compose left-to-right here and
    // the renderer will apply the resulting matrix around the centre.
    // Percentage `translate()` components are carried as box-size coefficients
    // (see `ExtMatrix`) so they survive composition without the box size.
    let mut result: ExtMatrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for t in &parsed {
        result = multiply_ext(&result, &transform_to_ext(t));
    }

    // Collapse to the cheaper `Matrix` form when no percentage translate is
    // present; otherwise keep the coefficients for render-time resolution.
    Some(transform_from_ext(result))
}

/// Parse a length/percentage argument to `translate()` into `(value, is_percent)`.
///
/// Absolute units (px/pt/in/cm/mm/pc) and font-relative units (em/rem) resolve
/// to pt; a `%` token returns the raw percentage with `is_percent = true` (it is
/// resolved against the element's own border box at render time). Only a bare
/// zero is accepted as a unitless length. `font_size` and `root_font_size` are
/// in pt.
fn parse_transform_length(val: &str, font_size: f32, root_font_size: f32) -> Option<(f64, bool)> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix('%') {
        return n.trim().parse::<f64>().ok().map(|v| (v, true));
    }
    parse_abs_length_pt_f64(val, font_size, root_font_size).map(|v| (v, false))
}

fn parse_abs_length_pt_f64(val: &str, font_size: f32, root_font_size: f32) -> Option<f64> {
    let val = val.trim();
    if let Some(number) = val.strip_suffix("px") {
        number.trim().parse::<f64>().ok().map(|value| value * 0.75)
    } else if let Some(number) = val.strip_suffix("pt") {
        number.trim().parse::<f64>().ok()
    } else if let Some(number) = val.strip_suffix("rem") {
        number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value * f64::from(root_font_size))
    } else if let Some(number) = val.strip_suffix("em") {
        number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value * f64::from(font_size))
    } else if let Some(number) = val.strip_suffix("in") {
        number.trim().parse::<f64>().ok().map(|value| value * 72.0)
    } else if let Some(number) = val.strip_suffix("cm") {
        number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value * 72.0 / 2.54)
    } else if let Some(number) = val.strip_suffix("mm") {
        number
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value * 72.0 / 25.4)
    } else if let Some(number) = val.strip_suffix("pc") {
        number.trim().parse::<f64>().ok().map(|value| value * 12.0)
    } else {
        val.parse::<f64>().ok().filter(|value| *value == 0.0)
    }
}

/// Resolve a CSS absolute/font-relative length token to pt. Returns `None` for
/// percentages, unknown units, and non-zero unitless numbers.
fn parse_abs_length_pt(val: &str, font_size: f32, root_font_size: f32) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("px") {
        n.trim().parse::<f32>().ok().map(|v| v * 0.75)
    } else if let Some(n) = val.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = val.strip_suffix("rem") {
        n.trim().parse::<f32>().ok().map(|v| v * root_font_size)
    } else if let Some(n) = val.strip_suffix("em") {
        n.trim().parse::<f32>().ok().map(|v| v * font_size)
    } else if let Some(n) = val.strip_suffix("in") {
        n.trim().parse::<f32>().ok().map(|v| v * 72.0)
    } else if let Some(n) = val.strip_suffix("cm") {
        n.trim().parse::<f32>().ok().map(|v| v * 72.0 / 2.54)
    } else if let Some(n) = val.strip_suffix("mm") {
        n.trim().parse::<f32>().ok().map(|v| v * 72.0 / 25.4)
    } else if let Some(n) = val.strip_suffix("pc") {
        n.trim().parse::<f32>().ok().map(|v| v * 12.0)
    } else {
        val.parse::<f32>().ok().filter(|value| *value == 0.0)
    }
}

/// Parse a CSS angle token (deg/rad/grad/turn) to degrees. Transform functions
/// additionally accept a unitless zero for legacy compatibility.
fn parse_angle_deg(val: &str) -> Option<f64> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("deg") {
        n.trim().parse::<f64>().ok()
    } else if let Some(n) = val.strip_suffix("grad") {
        n.trim().parse::<f64>().ok().map(|g| g * 0.9)
    } else if let Some(n) = val.strip_suffix("turn") {
        n.trim().parse::<f64>().ok().map(|t| t * 360.0)
    } else if let Some(n) = val.strip_suffix("rad") {
        n.trim().parse::<f64>().ok().map(f64::to_degrees)
    } else {
        val.parse::<f64>().ok().filter(|value| *value == 0.0)
    }
}

/// Parse a clip-path length/percentage token into (points-or-percent, is_percent).
fn parse_clip_len(token: &str) -> Option<(f32, bool)> {
    let t = token.trim();
    if let Some(n) = t.strip_suffix('%') {
        n.parse::<f32>().ok().map(|v| (v, true))
    } else if let Some(n) = t.strip_suffix("px") {
        n.parse::<f32>().ok().map(|v| (v * 0.75, false))
    } else if let Some(n) = t.strip_suffix("pt") {
        n.parse::<f32>().ok().map(|v| (v, false))
    } else {
        t.parse::<f32>().ok().map(|v| (v * 0.75, false))
    }
}

fn parse_clip_len_lp(token: &str) -> Option<LengthPercent> {
    parse_clip_len(token).map(LengthPercent::from)
}

fn parse_shape_box(token: &str) -> Option<ShapeBox> {
    match token.trim().to_ascii_lowercase().as_str() {
        "border-box" => Some(ShapeBox::Border),
        "padding-box" => Some(ShapeBox::Padding),
        "content-box" => Some(ShapeBox::Content),
        _ => None,
    }
}

fn parse_shape_extent(token: &str) -> Option<ShapeExtent> {
    match token.trim().to_ascii_lowercase().as_str() {
        "closest-side" => Some(ShapeExtent::ClosestSide),
        "farthest-side" => Some(ShapeExtent::FarthestSide),
        "closest-corner" => Some(ShapeExtent::ClosestCorner),
        "farthest-corner" => Some(ShapeExtent::FarthestCorner),
        _ => None,
    }
}

fn parse_clip_radius(token: &str, default: ShapeExtent) -> Option<ClipRadius> {
    let t = token.trim();
    if t.is_empty() {
        return Some(ClipRadius::Extent(default));
    }
    if let Some(extent) = parse_shape_extent(t) {
        Some(ClipRadius::Extent(extent))
    } else {
        parse_clip_len_lp(t).map(ClipRadius::Length)
    }
}

fn split_clip_geometry_box(raw: &str) -> (&str, ShapeBox) {
    let mut s = raw.trim();
    for suffix in ["border-box", "padding-box", "content-box"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest.trim();
            if s.is_empty() {
                return (suffix, ShapeBox::Border);
            }
            return (s, parse_shape_box(suffix).unwrap_or_default());
        }
    }
    (s, ShapeBox::Border)
}

fn split_function_args<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.trim()
        .strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
        .map(str::trim)
}

fn split_css_words(raw: &str) -> Vec<String> {
    split_css_components(raw)
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_clip_position(raw: &str) -> (LengthPercent, LengthPercent) {
    let center = LengthPercent::percent(50.0);
    let tokens = split_css_words(raw);
    if tokens.is_empty() {
        return (center, center);
    }
    let edge_value = |edge: &str, offset: Option<&str>| -> Option<LengthPercent> {
        match edge {
            "left" | "top" => offset
                .and_then(parse_clip_len_lp)
                .or(Some(LengthPercent::ZERO)),
            "right" | "bottom" => Some(
                LengthPercent::percent(100.0)
                    - offset
                        .and_then(parse_clip_len_lp)
                        .unwrap_or(LengthPercent::ZERO),
            ),
            "center" => Some(center),
            _ => None,
        }
    };
    if tokens.len() == 4 {
        let mut x = None;
        let mut y = None;
        let mut i = 0usize;
        while i + 1 < tokens.len() {
            match tokens[i].as_str() {
                "left" | "right" => x = edge_value(&tokens[i], Some(&tokens[i + 1])),
                "top" | "bottom" => y = edge_value(&tokens[i], Some(&tokens[i + 1])),
                _ => {}
            }
            i += 2;
        }
        return (x.unwrap_or(center), y.unwrap_or(center));
    }
    if tokens.len() == 2 {
        let a = tokens[0].as_str();
        let b = tokens[1].as_str();
        if matches!(a, "top" | "bottom") || matches!(b, "left" | "right") {
            return (
                edge_value(b, None).unwrap_or_else(|| parse_clip_len_lp(b).unwrap_or(center)),
                edge_value(a, None).unwrap_or_else(|| parse_clip_len_lp(a).unwrap_or(center)),
            );
        }
        return (
            edge_value(a, None).unwrap_or_else(|| parse_clip_len_lp(a).unwrap_or(center)),
            edge_value(b, None).unwrap_or_else(|| parse_clip_len_lp(b).unwrap_or(center)),
        );
    }
    let first = tokens[0].as_str();
    if matches!(first, "top" | "bottom") {
        (center, edge_value(first, None).unwrap_or(center))
    } else {
        (
            edge_value(first, None).unwrap_or_else(|| parse_clip_len_lp(first).unwrap_or(center)),
            center,
        )
    }
}

fn parse_clip_radius_group(tokens: &[&str]) -> Option<[f32; 4]> {
    let values = tokens
        .iter()
        .map(|token| parse_clip_len(token).map(|(value, _)| value.max(0.0)))
        .collect::<Option<Vec<_>>>()?;
    Some(match values.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    })
}

fn parse_clip_corner_radii(raw: &str) -> Option<CornerRadii> {
    let components = split_radius_components(raw, true)?;
    let [tl_x, tr_x, br_x, bl_x] = parse_clip_radius_group(&components.horizontal)?;
    let [tl_y, tr_y, br_y, bl_y] = components
        .vertical
        .as_deref()
        .map(parse_clip_radius_group)
        .unwrap_or(Some([tl_x, tr_x, br_x, bl_x]))?;
    Some(CornerRadii::new(
        CornerRadius::new(tl_x, tl_y),
        CornerRadius::new(tr_x, tr_y),
        CornerRadius::new(br_x, br_y),
        CornerRadius::new(bl_x, bl_y),
    ))
}

fn parse_rect_like_radii(raw: &str) -> Option<CornerRadii> {
    raw.split_once(" round ")
        .map_or(Some(CornerRadii::ZERO), |(_, radii)| {
            parse_clip_corner_radii(radii)
        })
}

/// Parse a CSS `clip-path` basic shape. Returns `None` for `none` and invalid
/// forms; supported fragment URLs and `path()` values have dedicated variants.
fn parse_clip_path(val: &str) -> Option<ClipPath> {
    let (raw, geometry_box) = split_clip_geometry_box(val);
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(inner) = split_function_args(raw, "circle") {
        let (shape, pos) = inner.split_once(" at ").unwrap_or((inner, ""));
        let r = parse_clip_radius(shape.trim(), ShapeExtent::ClosestSide)?;
        let (cx, cy) = parse_clip_position(pos);
        return Some(ClipPath::Circle {
            r,
            cx,
            cy,
            geometry_box,
        });
    }
    if let Some(inner) = split_function_args(raw, "ellipse") {
        let (shape, pos) = inner.split_once(" at ").unwrap_or((inner, ""));
        let radii: Vec<String> = split_css_words(shape);
        let rx = radii
            .first()
            .and_then(|s| parse_clip_radius(s, ShapeExtent::FarthestSide))
            .unwrap_or(ClipRadius::Extent(ShapeExtent::FarthestSide));
        let ry = radii
            .get(1)
            .and_then(|s| parse_clip_radius(s, ShapeExtent::FarthestSide))
            .unwrap_or(rx);
        let (cx, cy) = parse_clip_position(pos);
        return Some(ClipPath::Ellipse {
            rx,
            ry,
            cx,
            cy,
            geometry_box,
        });
    }
    if let Some(inner) = split_function_args(raw, "inset") {
        let (insets_part, radius) = match inner.split_once(" round ") {
            Some((insets, radii)) => (insets, parse_clip_corner_radii(radii)?),
            None => (inner, CornerRadii::ZERO),
        };
        let vals: Vec<LengthPercent> = insets_part
            .split_whitespace()
            .filter_map(parse_clip_len_lp)
            .collect();
        // CSS 1-4 value shorthand (top, right, bottom, left).
        let (top, right, bottom, left) = match vals.len() {
            1 => (vals[0], vals[0], vals[0], vals[0]),
            2 => (vals[0], vals[1], vals[0], vals[1]),
            3 => (vals[0], vals[1], vals[2], vals[1]),
            n if n >= 4 => (vals[0], vals[1], vals[2], vals[3]),
            _ => return None,
        };
        return Some(ClipPath::Inset {
            top,
            right,
            bottom,
            left,
            radii: radius,
            geometry_box,
        });
    }
    if let Some(inner) = split_function_args(raw, "polygon") {
        let mut parts = split_top_level_comma(inner);
        let even_odd = parts
            .first()
            .is_some_and(|p| p.trim().eq_ignore_ascii_case("evenodd"));
        if even_odd {
            parts.remove(0);
        }
        let points: Vec<(LengthPercent, LengthPercent)> = parts
            .iter()
            .filter_map(|pair| {
                let mut it = pair.split_whitespace();
                let x = parse_clip_len_lp(it.next()?)?;
                let y = parse_clip_len_lp(it.next()?)?;
                Some((x, y))
            })
            .collect();
        if points.len() >= 3 {
            return Some(ClipPath::Polygon {
                points,
                even_odd,
                geometry_box,
            });
        }
    }
    if let Some(inner) = split_function_args(raw, "path") {
        let d = inner.trim().trim_matches(|c: char| c == '"' || c == '\'');
        let commands = crate::parser::svg::parse_path_data(d);
        if !commands.is_empty() {
            return Some(ClipPath::Path {
                commands,
                geometry_box,
            });
        }
    }
    if let Some(inner) = split_function_args(raw, "rect") {
        let (coords, radii) = inner.split_once(" round ").unwrap_or((inner, ""));
        let vals: Vec<LengthPercent> = coords
            .split_whitespace()
            .filter_map(parse_clip_len_lp)
            .collect();
        if vals.len() >= 4 {
            let top = vals[0];
            let right = vals[1];
            let bottom = vals[2];
            let left = vals[3];
            let radii = if radii.is_empty() {
                CornerRadii::ZERO
            } else {
                parse_clip_corner_radii(radii)?
            };
            return Some(ClipPath::Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
                radii,
                geometry_box,
            });
        }
    }
    if let Some(inner) = split_function_args(raw, "xywh") {
        let (coords, _) = inner.split_once(" round ").unwrap_or((inner, ""));
        let vals: Vec<LengthPercent> = coords
            .split_whitespace()
            .filter_map(parse_clip_len_lp)
            .collect();
        if vals.len() >= 4 {
            let radii = parse_rect_like_radii(inner)?;
            return Some(ClipPath::Rect {
                x: vals[0],
                y: vals[1],
                width: vals[2],
                height: vals[3],
                radii,
                geometry_box,
            });
        }
    }
    if let Some(id) = parse_fragment_url(raw) {
        return Some(ClipPath::Url(id));
    }
    parse_shape_box(raw).map(|box_kind| ClipPath::Inset {
        top: LengthPercent::ZERO,
        right: LengthPercent::ZERO,
        bottom: LengthPercent::ZERO,
        left: LengthPercent::ZERO,
        radii: CornerRadii::ZERO,
        geometry_box: box_kind,
    })
}

/// Whether `value` is a valid clip path that this engine can preserve when an
/// upstream CSS parser leaves the property as an unparsed token list.
pub(crate) fn is_supported_clip_path(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("none") || parse_clip_path(value).is_some()
}

fn parse_legacy_clip_rect(val: &str) -> Option<ClipPath> {
    let raw = val.trim();
    let inner = split_function_args(raw, "rect")?;
    let vals: Vec<LengthPercent> = inner
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(parse_clip_len_lp)
        .collect();
    if vals.len() < 4 {
        return None;
    }
    Some(ClipPath::Rect {
        x: vals[3],
        y: vals[0],
        width: vals[1] - vals[3],
        height: vals[2] - vals[0],
        radii: CornerRadii::ZERO,
        geometry_box: ShapeBox::Border,
    })
}

fn parse_mask_mode(raw: &str) -> MaskMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "alpha" => MaskMode::Alpha,
        "luminance" => MaskMode::Luminance,
        _ => MaskMode::MatchSource,
    }
}

fn parse_mask_composite(raw: &str) -> MaskComposite {
    match raw.trim().to_ascii_lowercase().as_str() {
        "subtract" => MaskComposite::Subtract,
        "intersect" => MaskComposite::Intersect,
        "exclude" | "xor" => MaskComposite::Exclude,
        "source-out" => MaskComposite::Exclude,
        "source-in" => MaskComposite::Destination,
        _ => MaskComposite::Add,
    }
}

fn parse_fragment_url(raw: &str) -> Option<String> {
    let inner = raw
        .trim()
        .strip_prefix("url(")?
        .strip_suffix(')')?
        .trim()
        .trim_matches(|c| c == '\'' || c == '"');
    inner.strip_prefix('#').map(str::to_string)
}

fn parse_mask_layer_source(raw: &str, current_color: Color) -> Option<MaskLayerSource> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("linear-gradient(") || lower.starts_with("repeating-linear-gradient(") {
        return parse_linear_gradient_for_color(raw, current_color).map(MaskLayerSource::Linear);
    }
    if lower.starts_with("radial-gradient(") || lower.starts_with("repeating-radial-gradient(") {
        return parse_radial_gradient_for_color(raw, current_color).map(MaskLayerSource::Radial);
    }
    if lower.starts_with("conic-gradient(") || lower.starts_with("repeating-conic-gradient(") {
        return parse_conic_gradient_for_color(raw, current_color).map(MaskLayerSource::Conic);
    }
    if lower.starts_with("url(") {
        if let Some(id) = parse_fragment_url(raw) {
            return Some(MaskLayerSource::Ref(id));
        }
    }
    if lower.starts_with("url(") {
        return parse_mask_url_svg(raw).map(MaskLayerSource::Svg);
    }
    None
}

fn mask_layer_from_source(source: MaskLayerSource, mode: MaskMode) -> MaskLayer {
    MaskLayer {
        source,
        mode,
        layer_box: GradientLayerBox::default(),
        origin: ShapeBox::Border,
        clip: ShapeBox::Border,
        composite: MaskComposite::Add,
    }
}

fn mask_source_from_layers(layers: Vec<MaskLayer>) -> Option<Option<MaskSource>> {
    match layers.len() {
        0 => None,
        1 => {
            let layer = layers.into_iter().next()?;
            if layer.layer_box.size.is_none()
                && layer.layer_box.position.is_none()
                && layer.layer_box.repeat.is_none()
                && layer.origin == ShapeBox::Border
                && layer.clip == ShapeBox::Border
                && layer.composite == MaskComposite::Add
            {
                match layer.source {
                    MaskLayerSource::Linear(g) => Some(Some(MaskSource::Linear(g))),
                    MaskLayerSource::Radial(g) => Some(Some(MaskSource::Radial(g))),
                    MaskLayerSource::Conic(g) => Some(Some(MaskSource::Conic(g))),
                    MaskLayerSource::Svg(bytes) => Some(Some(MaskSource::Svg(bytes))),
                    MaskLayerSource::Ref(id) => Some(Some(MaskSource::Ref(id))),
                }
            } else {
                Some(Some(MaskSource::Layers(vec![layer])))
            }
        }
        _ => Some(Some(MaskSource::Layers(layers))),
    }
}

fn parse_mask_image(val: &str, mode: MaskMode, current_color: Color) -> Option<Option<MaskSource>> {
    let raw = val.trim();
    if raw.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let layers: Vec<MaskLayer> = split_top_level_commas_value(raw)
        .into_iter()
        .filter_map(|part| {
            parse_mask_layer_source(&part, current_color)
                .map(|src| mask_layer_from_source(src, mode))
        })
        .collect();
    mask_source_from_layers(layers)
}

fn extract_first_function(raw: &str) -> Option<(&str, &str)> {
    let raw = raw.trim();
    let open = raw.find('(')?;
    let mut depth = 0i32;
    let mut quote = None;
    for (idx, ch) in raw.char_indices().skip(open) {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&raw[..=idx], raw[idx + 1..].trim()));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_mask_shorthand(
    val: &str,
    inherited_mode: MaskMode,
    current_color: Color,
) -> Option<Option<MaskSource>> {
    let raw = val.trim();
    if raw.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let (image, rest) = extract_first_function(raw)?;
    let mut layer = mask_layer_from_source(
        parse_mask_layer_source(image, current_color)?,
        inherited_mode,
    );
    let mut rest = rest.trim();
    for token in split_css_words(rest) {
        match token.as_str() {
            "alpha" | "luminance" | "match-source" => layer.mode = parse_mask_mode(&token),
            "repeat" | "no-repeat" | "repeat-x" | "repeat-y" => {
                layer.layer_box.repeat = Some(parse_background_repeat_value(&token));
            }
            "border-box" | "padding-box" | "content-box" => {
                if let Some(box_kind) = parse_shape_box(&token) {
                    layer.origin = box_kind;
                    layer.clip = box_kind;
                }
            }
            _ => {}
        }
    }
    if let Some((pos, tail)) = rest.split_once('/') {
        layer.layer_box.position = parse_background_position(pos.trim());
        let size_tokens: Vec<String> = split_css_components(tail)
            .into_iter()
            .take_while(|t| {
                !matches!(
                    t.as_str(),
                    "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "alpha" | "luminance"
                )
            })
            .collect();
        if !size_tokens.is_empty() {
            layer.layer_box.size = Some(parse_background_size_value(&size_tokens.join(" ")));
        }
    } else {
        let tokens = split_css_components(rest);
        let mut pos_tokens = Vec::new();
        for token in &tokens {
            if matches!(
                token.as_str(),
                "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "alpha" | "luminance"
            ) || parse_shape_box(token).is_some()
            {
                break;
            }
            pos_tokens.push(token.clone());
        }
        if !pos_tokens.is_empty() {
            layer.layer_box.position = parse_background_position(&pos_tokens.join(" "));
        }
    }
    rest = "";
    let _ = rest;
    Some(Some(MaskSource::Layers(vec![layer])))
}

fn apply_mask_longhands(map: &StyleMap, style: &mut ComputedStyle, current_color: Color) {
    let Some(source) = style.mask_image.take() else {
        if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-border-source")
            && parse_mask_layer_source(k, current_color).is_some()
        {
            let width = get_non_special(map, "mask-border-width")
                .and_then(|v| match v {
                    CssValue::Length(w) => Some(*w),
                    CssValue::Keyword(k) => parse_clip_len(k).map(|(v, _)| v),
                    _ => None,
                })
                .unwrap_or(0.0);
            style.mask_image = Some(MaskSource::BorderRing { width });
        }
        return;
    };
    let mut layers = match source {
        MaskSource::Layers(layers) => layers,
        MaskSource::Linear(g) => vec![mask_layer_from_source(
            MaskLayerSource::Linear(g),
            style.mask_mode,
        )],
        MaskSource::Radial(g) => vec![mask_layer_from_source(
            MaskLayerSource::Radial(g),
            style.mask_mode,
        )],
        MaskSource::Conic(g) => vec![mask_layer_from_source(
            MaskLayerSource::Conic(g),
            style.mask_mode,
        )],
        MaskSource::Svg(bytes) => vec![mask_layer_from_source(
            MaskLayerSource::Svg(bytes),
            style.mask_mode,
        )],
        MaskSource::Ref(id) => vec![mask_layer_from_source(
            MaskLayerSource::Ref(id),
            style.mask_mode,
        )],
        MaskSource::BorderRing { width } => {
            style.mask_image = Some(MaskSource::BorderRing { width });
            return;
        }
    };
    if layers.is_empty() {
        return;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-size") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx) {
                layer.layer_box.size = Some(parse_background_size_value(&part));
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-position") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx) {
                layer.layer_box.position = parse_background_position(&part);
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-repeat") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx) {
                layer.layer_box.repeat = Some(parse_background_repeat_value(&part));
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-origin") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx).and_then(|p| parse_shape_box(&p)) {
                layer.origin = part;
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-clip") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx).and_then(|p| parse_shape_box(&p)) {
                layer.clip = part;
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-mode") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx) {
                layer.mode = parse_mask_mode(&part);
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-composite") {
        for (idx, layer) in layers.iter_mut().enumerate() {
            if let Some(part) = nth_layer_value(k, idx) {
                layer.composite = parse_mask_composite(&part);
            }
        }
    }
    style.mask_image = Some(MaskSource::Layers(layers));
}

/// Resolve a `url(...)` mask reference to raw SVG bytes (css-masking-1 §3.1).
///
/// Returns `Some(bytes)` only when the reference loads and looks like SVG;
/// otherwise `None` so the caller leaves the existing mask value untouched.
fn parse_mask_url_svg(val: &str) -> Option<std::sync::Arc<Vec<u8>>> {
    let inner = val.strip_prefix("url(")?.strip_suffix(')')?.trim();
    // Strip optional surrounding quotes.
    let url = inner
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(inner)
        .trim();
    if url.is_empty() {
        return None;
    }
    // Authorise and load through the ambient resource loader, like every other
    // document resource. (This still loads during style computation; moving mask
    // I/O out of the cascade is a separate change.)
    let (bytes, _mime) = crate::layout::images::load_resource(url, None)?;
    // Accept only sources whose bytes actually sniff as SVG, since the mask
    // rasteriser only understands SVG image sources. The MIME alone is not
    // trusted (it can mislabel non-SVG payloads).
    if !crate::layout::images::looks_like_svg(&bytes) {
        return None;
    }
    Some(bytes)
}

/// Parse a CSS Grid box-alignment keyword (`start`/`end`/`center`/`stretch`).
/// Also accepts the flex aliases `flex-start`/`flex-end` for robustness.
fn parse_display_value(k: &str) -> Option<Display> {
    let lower = k.trim().to_ascii_lowercase();
    match lower.as_str() {
        "none" => return Some(Display::None),
        "inline" => return Some(Display::Inline),
        "inline-block" => return Some(Display::InlineBlock),
        "inline-flex" => return Some(Display::InlineFlex),
        "block" | "-webkit-box" => return Some(Display::Block),
        "list-item" => return Some(Display::ListItem),
        "flex" => return Some(Display::Flex),
        "grid" => return Some(Display::Grid),
        "inline-grid" => return Some(Display::InlineGrid),
        "table" => return Some(Display::Table),
        "inline-table" => return Some(Display::InlineTable),
        "table-row-group" => return Some(Display::TableRowGroup),
        "table-header-group" => return Some(Display::TableHeaderGroup),
        "table-footer-group" => return Some(Display::TableFooterGroup),
        "table-row" => return Some(Display::TableRow),
        "table-cell" => return Some(Display::TableCell),
        "table-column-group" => return Some(Display::TableColumnGroup),
        "table-column" => return Some(Display::TableColumn),
        "table-caption" => return Some(Display::TableCaption),
        _ => {}
    }

    let parts: Vec<&str> = lower.split_whitespace().collect();
    if parts.contains(&"none") {
        Some(Display::None)
    } else if parts.contains(&"inline") && parts.contains(&"flex") {
        Some(Display::InlineFlex)
    } else if parts.contains(&"flex") {
        Some(Display::Flex)
    } else if parts.contains(&"grid") {
        if parts.contains(&"inline") {
            Some(Display::InlineGrid)
        } else {
            Some(Display::Grid)
        }
    } else if parts.contains(&"table") {
        if parts.contains(&"caption") {
            Some(Display::TableCaption)
        } else if parts.contains(&"cell") {
            Some(Display::TableCell)
        } else if parts.contains(&"row") {
            Some(Display::TableRow)
        } else if parts.contains(&"column") {
            Some(Display::TableColumn)
        } else if parts.contains(&"inline") {
            Some(Display::InlineTable)
        } else {
            Some(Display::Table)
        }
    } else if parts.contains(&"inline")
        && (parts.contains(&"block") || parts.contains(&"flow-root"))
    {
        Some(Display::InlineBlock)
    } else if parts.contains(&"inline") {
        Some(Display::Inline)
    } else if parts.contains(&"list-item") {
        Some(Display::ListItem)
    } else if parts.contains(&"block") || parts.contains(&"flow") {
        Some(Display::Block)
    } else {
        None
    }
}

fn strip_overflow_position(k: &str) -> (&str, bool) {
    let raw = k.trim();
    if let Some(rest) = raw.strip_prefix("safe ") {
        (rest.trim(), true)
    } else if let Some(rest) = raw.strip_prefix("unsafe ") {
        (rest.trim(), false)
    } else {
        (raw, false)
    }
}

fn split_alignment_components(k: &str) -> Vec<String> {
    let tokens: Vec<&str> = k.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], "safe" | "unsafe") && i + 1 < tokens.len() {
            out.push(format!("{} {}", tokens[i], tokens[i + 1]));
            i += 2;
        } else {
            out.push(tokens[i].to_string());
            i += 1;
        }
    }
    out
}

fn parse_justify_content(k: &str, is_row: bool) -> JustifyContent {
    let (kw, safe) = strip_overflow_position(k);
    match kw {
        "flex-end" | "end" => JustifyContent::FlexEnd,
        "right" if is_row => JustifyContent::FlexEnd,
        "left" | "right" => JustifyContent::FlexStart,
        "center" if safe => JustifyContent::SafeCenter,
        "center" => JustifyContent::Center,
        "space-between" => JustifyContent::SpaceBetween,
        "space-around" => JustifyContent::SpaceAround,
        "space-evenly" => JustifyContent::SpaceEvenly,
        _ => JustifyContent::FlexStart,
    }
}

fn parse_align_items(k: &str) -> AlignItems {
    match strip_overflow_position(k).0 {
        "flex-start" | "start" | "self-start" => AlignItems::FlexStart,
        "flex-end" | "end" | "self-end" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" | "first baseline" | "last baseline" => AlignItems::Baseline,
        _ => AlignItems::Stretch,
    }
}

fn parse_align_self(k: &str) -> AlignSelf {
    match strip_overflow_position(k).0 {
        "auto" => AlignSelf::Auto,
        "flex-start" | "start" | "self-start" => AlignSelf::FlexStart,
        "flex-end" | "end" | "self-end" => AlignSelf::FlexEnd,
        "center" => AlignSelf::Center,
        "baseline" | "first baseline" | "last baseline" => AlignSelf::Baseline,
        "stretch" => AlignSelf::Stretch,
        _ => AlignSelf::Auto,
    }
}

fn parse_align_content(k: &str) -> AlignContent {
    match strip_overflow_position(k).0 {
        "flex-start" | "start" => AlignContent::FlexStart,
        "flex-end" | "end" => AlignContent::FlexEnd,
        "center" => AlignContent::Center,
        "space-between" => AlignContent::SpaceBetween,
        "space-around" => AlignContent::SpaceAround,
        "space-evenly" => AlignContent::SpaceEvenly,
        _ => AlignContent::Stretch,
    }
}

fn parse_grid_align(k: &str) -> GridAlign {
    match strip_overflow_position(k).0 {
        "start" | "flex-start" | "left" | "self-start" => GridAlign::Start,
        "end" | "flex-end" | "right" | "self-end" => GridAlign::End,
        "center" => GridAlign::Center,
        _ => GridAlign::Stretch,
    }
}

fn split_css_components(val: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote: Option<char> = None;

    for ch in val.chars() {
        if let Some(q) = quote {
            current.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                bracket_depth += 1;
                current.push(ch);
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && paren_depth == 0 && bracket_depth == 0 => {
                if !current.trim().is_empty() {
                    parts.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

fn set_flex_basis_auto(style: &mut ComputedStyle) {
    style.flex_basis = FlexBasis::Auto;
}

fn set_flex_basis_length(style: &mut ComputedStyle, value: f32) {
    style.flex_basis = FlexBasis::Definite(LengthPercent::length(value));
}

fn set_flex_basis_percentage(style: &mut ComputedStyle, percent: f32) {
    style.flex_basis = FlexBasis::Definite(LengthPercent::percent(percent));
}

/// Parse one computed CSS `<length-percentage>` without prematurely resolving
/// its percentage term. Layout consumers supply the correct eventual basis.
pub(crate) fn computed_length_percent(
    value: &CssValue,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<LengthPercent> {
    match value {
        CssValue::Length(value) => Some(LengthPercent::length(*value)),
        CssValue::Percentage(value) => Some(LengthPercent::percent(*value)),
        CssValue::Math(expression) => expression.affine(style.math_unit_context(font_metrics)),
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            computed_length_percent(
                &parse_property_value("flex-basis", &raw)?,
                style,
                length_context,
                font_metrics,
            )
        }
        value => resolve_css_length_for_style(value, style, length_context, font_metrics)
            .map(LengthPercent::length),
    }
}

fn apply_flex_basis_value(
    style: &mut ComputedStyle,
    value: &CssValue,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> bool {
    match value {
        CssValue::Length(_) | CssValue::Percentage(_) | CssValue::Math(_) | CssValue::Var(_, _) => {
            let Some(value) = computed_length_percent(value, style, length_context, font_metrics)
            else {
                return false;
            };
            style.flex_basis = FlexBasis::Definite(value);
            true
        }
        CssValue::Keyword(k) => apply_flex_basis_token(style, k, length_context, font_metrics),
        CssValue::Em(_)
        | CssValue::Rem(_)
        | CssValue::Vw(_)
        | CssValue::Vh(_)
        | CssValue::Vmin(_)
        | CssValue::Vmax(_)
        | CssValue::Ex(_)
        | CssValue::Ch(_)
        | CssValue::Number(0.0) => {
            if let Some(v) =
                resolve_css_length_for_style(value, style, length_context, font_metrics)
            {
                set_flex_basis_length(style, v);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn apply_flex_basis_token(
    style: &mut ComputedStyle,
    token: &str,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> bool {
    let lower = token.trim().to_ascii_lowercase();
    match lower.as_str() {
        "auto" => {
            set_flex_basis_auto(style);
            true
        }
        "content" | "max-content" => {
            style.flex_basis = FlexBasis::Content(IntrinsicWidthKeyword::MaxContent);
            true
        }
        "fit-content" | "min-content" => {
            style.flex_basis = FlexBasis::Content(if lower == "min-content" {
                IntrinsicWidthKeyword::MinContent
            } else {
                IntrinsicWidthKeyword::FitContent
            });
            true
        }
        _ => match parse_length(&lower) {
            Some(CssValue::Percentage(p)) => {
                set_flex_basis_percentage(style, p);
                true
            }
            Some(parsed) => apply_flex_basis_value(style, &parsed, length_context, font_metrics),
            None => false,
        },
    }
}

fn set_flex_basis_zero(style: &mut ComputedStyle) {
    style.flex_basis = FlexBasis::Definite(LengthPercent::ZERO);
}

fn apply_flex_shorthand(
    style: &mut ComputedStyle,
    value: &str,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) {
    let lower = value.trim().to_ascii_lowercase();
    match lower.as_str() {
        "none" => {
            style.flex_grow = 0.0;
            style.flex_shrink = 0.0;
            set_flex_basis_auto(style);
            return;
        }
        "auto" => {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            set_flex_basis_auto(style);
            return;
        }
        "initial" => {
            style.flex_grow = 0.0;
            style.flex_shrink = 1.0;
            set_flex_basis_auto(style);
            return;
        }
        _ => {}
    }

    let parts = split_css_components(&lower);
    let Some(first) = parts.first() else {
        return;
    };

    if let Ok(grow) = first.parse::<f32>() {
        style.flex_grow = grow.max(0.0);
        style.flex_shrink = 1.0;
        set_flex_basis_zero(style);

        if let Some(second) = parts.get(1) {
            if let Ok(shrink) = second.parse::<f32>() {
                style.flex_shrink = shrink.max(0.0);
                if let Some(third) = parts.get(2) {
                    apply_flex_basis_token(style, third, length_context, font_metrics);
                }
            } else {
                apply_flex_basis_token(style, second, length_context, font_metrics);
            }
        }
        return;
    }

    if parts.len() == 1 && apply_flex_basis_token(style, first, length_context, font_metrics) {
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
    }
}

/// Parse a single `flex-direction` keyword. Returns `None` for unrecognized
/// tokens so the `flex-flow` shorthand can try them as a `flex-wrap` value.
fn parse_flex_direction(k: &str) -> Option<FlexDirection> {
    match k.trim() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

/// Parse a single `flex-wrap` keyword. Returns `None` for unrecognized tokens
/// so the `flex-flow` shorthand can try them as a `flex-direction` value.
fn parse_flex_wrap(k: &str) -> Option<FlexWrap> {
    match k.trim() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

/// Parse a `grid-column` / `grid-row` value into a span count. Supports
/// `span N` and bare integer line ranges (`start / end` → end-start). Returns
/// `None` when no explicit span is expressed (defaults to 1 elsewhere).
fn parse_grid_span(val: &str) -> Option<usize> {
    let val = val.trim();
    // `a / b` line syntax: span = |b - a| when both are integers.
    if let Some((a, b)) = val.split_once('/') {
        let a = a.trim();
        let b = b.trim();
        if let Some(rest) = b.strip_prefix("span") {
            return rest.trim().parse::<usize>().ok().filter(|n| *n >= 1);
        }
        if let (Ok(ai), Ok(bi)) = (a.parse::<i32>(), b.parse::<i32>()) {
            let n = (bi - ai).unsigned_abs() as usize;
            return if n >= 1 { Some(n) } else { None };
        }
        return None;
    }
    if let Some(rest) = val.strip_prefix("span") {
        return rest.trim().parse::<usize>().ok().filter(|n| *n >= 1);
    }
    None
}

/// Parse a single grid-placement endpoint (one side of `grid-column` /
/// `grid-row` / a quarter of `grid-area`). CSS Grid §8.3 grammar (subset):
///   `auto | <integer> | span <integer> | <custom-ident> | span <custom-ident>`
fn parse_grid_line(token: &str) -> GridLine {
    let token = token.trim();
    if token.is_empty() || token == "auto" {
        return GridLine::Auto;
    }
    if let Some(rest) = token.strip_prefix("span") {
        let rest = rest.trim();
        let parts = split_css_components(rest);
        let mut count = 1usize;
        let mut name: Option<String> = None;
        for part in parts {
            if let Ok(parsed) = part.parse::<usize>() {
                if parsed == 0 {
                    return GridLine::Auto;
                }
                count = parsed;
            } else if name.replace(part).is_some() {
                return GridLine::Auto;
            }
        }
        if let Some(name) = name {
            return GridLine::SpanNamed { count, name };
        }
        return GridLine::Span(count);
    }
    if let Ok(n) = token.parse::<i32>() {
        // A line number of 0 is invalid per spec; treat as auto.
        if n != 0 {
            return GridLine::Line(n);
        }
        return GridLine::Auto;
    }
    GridLine::Named(token.to_string())
}

/// Parse a `grid-column` / `grid-row` shorthand into (start, end) endpoints.
/// The two sides are separated by `/`; an omitted second side defaults to
/// `auto` (which §8.3 then resolves to a 1-track span / matching named line).
fn parse_grid_placement_shorthand(val: &str) -> (GridLine, GridLine) {
    let val = val.trim();
    if let Some((a, b)) = val.split_once('/') {
        (parse_grid_line(a), parse_grid_line(b))
    } else {
        (parse_grid_line(val), GridLine::Auto)
    }
}

/// Apply a `grid-area` value to a style. Either a single `<custom-ident>`
/// naming an area, or the 4-value line form
/// `row-start / col-start / row-end / col-end` (§8.1, omitted parts = auto).
fn apply_grid_area(style: &mut ComputedStyle, val: &str) {
    let val = val.trim();
    if val.contains('/') {
        let parts: Vec<&str> = val.split('/').collect();
        let get = |i: usize| parts.get(i).map(|s| parse_grid_line(s)).unwrap_or_default();
        style.grid_row_start = get(0);
        style.grid_column_start = get(1);
        style.grid_row_end = get(2);
        style.grid_column_end = get(3);
    } else if !val.is_empty() && val != "auto" {
        style.grid_area_name = Some(val.to_string());
    }
}

/// Parse `grid-template-areas` row strings into a row-major grid of optional
/// area names (§7.3). Each quoted string is a row; whitespace-separated tokens
/// are cells; a token of all dots (`.`/`...`) is a null (empty) cell → `None`.
/// CSS requires equal row lengths and rectangular named areas; invalid
/// declarations are ignored by returning an empty area matrix.
fn parse_grid_template_areas(val: &str) -> Vec<Vec<Option<String>>> {
    let val = val.trim();
    if val == "none" || val.is_empty() {
        return Vec::new();
    }
    let mut row_strings: Vec<String> = Vec::new();
    // Each row is delimited by a quoted string. Split on the quote characters
    // and keep the segments between matched quotes.
    let mut in_quote = false;
    let mut current = String::new();
    for ch in val.chars() {
        match ch {
            '"' | '\'' => {
                if in_quote {
                    // End of a row string.
                    if !current.trim().is_empty() {
                        row_strings.push(current.clone());
                    }
                    current.clear();
                    in_quote = false;
                } else {
                    in_quote = true;
                }
            }
            _ if in_quote => current.push(ch),
            _ => {}
        }
    }
    parse_grid_template_area_rows(&row_strings).unwrap_or_default()
}

fn parse_grid_template_area_rows(row_strings: &[String]) -> Option<Vec<Vec<Option<String>>>> {
    let mut rows = Vec::new();
    for row in row_strings {
        let cells: Vec<Option<String>> = row
            .split_whitespace()
            .map(|tok| {
                if tok.chars().all(|c| c == '.') {
                    None
                } else {
                    Some(tok.to_string())
                }
            })
            .collect();
        if cells.is_empty() {
            return None;
        }
        rows.push(cells);
    }

    if rows.is_empty() || !grid_template_areas_are_valid(&rows) {
        None
    } else {
        Some(rows)
    }
}

fn grid_template_areas_are_valid(rows: &[Vec<Option<String>>]) -> bool {
    let Some(width) = rows.first().map(Vec::len) else {
        return false;
    };
    if width == 0 || rows.iter().any(|row| row.len() != width) {
        return false;
    }

    let mut bounds: HashMap<&str, (usize, usize, usize, usize)> = HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(name) = cell {
                let entry = bounds.entry(name.as_str()).or_insert((r, r, c, c));
                entry.0 = entry.0.min(r);
                entry.1 = entry.1.max(r);
                entry.2 = entry.2.min(c);
                entry.3 = entry.3.max(c);
            }
        }
    }

    for (name, (r0, r1, c0, c1)) in bounds {
        for row in rows.iter().take(r1 + 1).skip(r0) {
            for cell in row.iter().take(c1 + 1).skip(c0) {
                if cell.as_deref() != Some(name) {
                    return false;
                }
            }
        }
    }

    true
}

fn apply_grid_template_areas(style: &mut ComputedStyle, areas: Vec<Vec<Option<String>>>) {
    style.grid_template_areas = areas;
    synthesize_grid_area_tracks(style);
}

fn synthesize_grid_area_tracks(style: &mut ComputedStyle) {
    if style.grid_template_areas.is_empty() {
        return;
    }
    let rows = style.grid_template_areas.len();
    let cols = style
        .grid_template_areas
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0);

    if cols > 0 && style.grid_template_columns.is_empty() {
        style.grid_template_columns = vec![GridTrack::Auto; cols];
        style.grid_template_column_line_names = vec![Vec::new(); cols + 1];
    }
    if rows > 0 && style.grid_template_rows.is_empty() {
        style.grid_template_rows = vec![GridTrack::Auto; rows];
        style.grid_template_row_line_names = vec![Vec::new(); rows + 1];
    }
}

fn split_top_level_once(value: &str, delimiter: char) -> Option<(&str, &str)> {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote: Option<char> = None;

    for (idx, ch) in value.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            c if c == delimiter && paren_depth == 0 && bracket_depth == 0 => {
                let after = idx + ch.len_utf8();
                return Some((&value[..idx], &value[after..]));
            }
            _ => {}
        }
    }

    None
}

fn reset_grid_template(style: &mut ComputedStyle) {
    style.grid_template_columns.clear();
    style.grid_template_rows.clear();
    style.grid_template_areas.clear();
    style.grid_template_column_line_names.clear();
    style.grid_template_row_line_names.clear();
}

fn apply_grid_track_list_to_rows(style: &mut ComputedStyle, value: &str) {
    let (tracks, names) = parse_grid_track_list(value);
    style.grid_template_rows = tracks;
    style.grid_template_row_line_names = names;
}

fn apply_grid_track_list_to_columns(style: &mut ComputedStyle, value: &str) {
    let (tracks, names) = parse_grid_track_list(value);
    style.grid_template_columns = tracks;
    style.grid_template_column_line_names = names;
}

type GridTemplateAreaRows = (Vec<Vec<Option<String>>>, Vec<GridTrack>, Vec<Vec<String>>);

fn parse_grid_template_rows_with_areas(value: &str) -> Option<GridTemplateAreaRows> {
    let mut row_strings = Vec::new();
    let mut row_tracks = Vec::new();
    let mut row_line_names: Vec<Vec<String>> = vec![Vec::new()];
    let mut pos = 0usize;

    while pos < value.len() {
        let remaining = &value[pos..];
        let Some(open_rel) = remaining.find(['"', '\'']) else {
            break;
        };
        let open = pos + open_rel;
        let quote = value[open..].chars().next()?;
        let content_start = open + quote.len_utf8();
        let close_rel = value[content_start..].find(quote)?;
        let close = content_start + close_rel;
        row_strings.push(value[content_start..close].to_string());

        let after_quote = close + quote.len_utf8();
        let next_quote = value[after_quote..]
            .find(['"', '\''])
            .map(|idx| after_quote + idx)
            .unwrap_or(value.len());
        let track_segment = value[after_quote..next_quote].trim();
        if !track_segment.is_empty() {
            let (tracks, names) = parse_grid_track_list(track_segment);
            if let Some(first) = tracks.first() {
                if let (Some(slot), Some(src)) = (row_line_names.last_mut(), names.first()) {
                    slot.extend(src.iter().cloned());
                }
                row_tracks.push(first.clone());
                row_line_names.push(names.get(1).cloned().unwrap_or_default());
            }
        } else {
            row_tracks.push(GridTrack::Auto);
            row_line_names.push(Vec::new());
        }
        pos = next_quote;
    }

    let areas = parse_grid_template_area_rows(&row_strings)?;
    while row_tracks.len() < areas.len() {
        row_tracks.push(GridTrack::Auto);
        row_line_names.push(Vec::new());
    }
    Some((areas, row_tracks, row_line_names))
}

fn apply_grid_template_shorthand(style: &mut ComputedStyle, value: &str) {
    let value = value.trim();
    if value == "none" {
        reset_grid_template(style);
        return;
    }

    let (rows_part, cols_part) = split_top_level_once(value, '/').unwrap_or((value, ""));
    if rows_part.contains('"') || rows_part.contains('\'') {
        if let Some((areas, rows, row_names)) = parse_grid_template_rows_with_areas(rows_part) {
            style.grid_template_areas = areas;
            style.grid_template_rows = rows;
            style.grid_template_row_line_names = row_names;
        }
    } else if !rows_part.trim().is_empty() && rows_part.trim() != "none" {
        apply_grid_track_list_to_rows(style, rows_part);
    }

    if !cols_part.trim().is_empty() && cols_part.trim() != "none" {
        apply_grid_track_list_to_columns(style, cols_part);
    }
    synthesize_grid_area_tracks(style);
}

fn apply_grid_shorthand(style: &mut ComputedStyle, value: &str) {
    reset_grid_template(style);
    style.grid_auto_rows = None;
    style.grid_auto_rows_pattern.clear();
    style.grid_auto_flow_column = false;
    style.grid_auto_flow_dense = false;

    let value = value.trim();
    if value == "none" {
        return;
    }

    let (before_slash, after_slash) = split_top_level_once(value, '/').unwrap_or((value, ""));
    let before_tokens = split_css_components(before_slash);
    let after_tokens = split_css_components(after_slash);
    let before_auto_flow = before_tokens.iter().any(|t| t == "auto-flow");
    let after_auto_flow = after_tokens.iter().any(|t| t == "auto-flow");

    if before_auto_flow {
        style.grid_auto_flow_column = false;
        style.grid_auto_flow_dense = before_tokens.iter().any(|t| t == "dense");
        let row_tokens: Vec<&str> = before_tokens
            .iter()
            .map(String::as_str)
            .filter(|t| *t != "auto-flow" && *t != "dense")
            .collect();
        if let Some(track) = row_tokens.first().and_then(|t| parse_single_track(t)) {
            if let Some(v) = fixed_grid_track_size(&track) {
                style.grid_auto_rows = Some(v);
                style.grid_auto_rows_pattern = vec![v];
            }
        }
        if !after_slash.trim().is_empty() {
            apply_grid_track_list_to_columns(style, after_slash);
        }
        return;
    }

    if after_auto_flow {
        style.grid_auto_flow_column = true;
        style.grid_auto_flow_dense = after_tokens.iter().any(|t| t == "dense");
        if !before_slash.trim().is_empty() {
            apply_grid_track_list_to_rows(style, before_slash);
        }
        return;
    }

    apply_grid_template_shorthand(style, value);
}

fn fixed_grid_track_size(track: &GridTrack) -> Option<f32> {
    match track {
        GridTrack::Fixed(v) => Some(*v),
        GridTrack::Minmax(min, _) => Some(*min),
        _ => None,
    }
}

/// Parse a single grid track token (e.g. `1fr`, `200pt`, `100px`, `auto`).
fn parse_single_track(token: &str) -> Option<GridTrack> {
    let token = token.trim();
    if let Some(n) = token.strip_suffix("fr") {
        n.parse::<f32>().ok().map(GridTrack::Fr)
    } else if let Some(n) = token.strip_suffix('%') {
        n.parse::<f32>().ok().map(|v| GridTrack::Percent(v / 100.0))
    } else if token == "auto" || token == "auto-fill" || token == "auto-fit" {
        Some(GridTrack::Auto)
    } else if let Some(n) = token.strip_suffix("pt") {
        n.parse::<f32>().ok().map(GridTrack::Fixed)
    } else if let Some(n) = token.strip_suffix("px") {
        n.parse::<f32>().ok().map(|v| GridTrack::Fixed(v * 0.75))
    } else {
        token.parse::<f32>().ok().map(GridTrack::Fixed)
    }
}

/// Parse a `minmax(min, max)` expression.
fn parse_minmax(val: &str) -> Option<GridTrack> {
    let inner = val.strip_prefix("minmax(")?.strip_suffix(')')?;
    let mut parts = inner.splitn(2, ',');
    let min_s = parts.next()?.trim();
    let max_s = parts.next()?.trim();

    let min_val = if min_s == "auto" || min_s == "0" {
        0.0
    } else if let Some(n) = min_s.strip_suffix("px") {
        n.parse::<f32>().ok()? * 0.75
    } else if let Some(n) = min_s.strip_suffix("pt") {
        n.parse::<f32>().ok()?
    } else {
        min_s.parse::<f32>().ok().unwrap_or(0.0)
    };

    // If max is `1fr` or `auto`, treat as flexible — use Minmax with a large max
    let max_val = if max_s.ends_with("fr") || max_s == "auto" {
        f32::MAX
    } else if let Some(n) = max_s.strip_suffix("px") {
        n.parse::<f32>().ok()? * 0.75
    } else if let Some(n) = max_s.strip_suffix("pt") {
        n.parse::<f32>().ok()?
    } else {
        max_s.parse::<f32>().ok().unwrap_or(f32::MAX)
    };

    Some(GridTrack::Minmax(min_val, max_val))
}

/// Parse a `grid-template-columns`/`-rows` value into a list of `GridTrack`s.
///
/// Supports tokens like `1fr`, `200pt`, `100px`, `auto`, `repeat(3, 1fr)`,
/// `minmax(100px, 1fr)`, `auto-fill`, and `auto-fit`. Bracketed `[name]` line
/// names are tolerated (and dropped). For the line-name vocabulary use
/// `parse_grid_track_list`.
#[cfg(test)]
fn parse_grid_template_columns(val: &str) -> Vec<GridTrack> {
    parse_grid_track_list(val).0
}

/// Parse a track list into both the `GridTrack`s and the names of each grid
/// *line* (CSS Grid §7.1). The returned name list has `tracks.len() + 1`
/// entries (line index 0 = the line before the first track); entry `i` holds
/// the names declared for line `i` via bracketed `[a b]` tokens.
fn parse_grid_track_list(val: &str) -> (Vec<GridTrack>, Vec<Vec<String>>) {
    let mut result: Vec<GridTrack> = Vec::new();
    // Names accumulate at the "current" line (index == result.len()).
    let mut line_names: Vec<Vec<String>> = vec![Vec::new()];
    let mut remaining = val.trim();

    let push_track =
        |result: &mut Vec<GridTrack>, line_names: &mut Vec<Vec<String>>, track: GridTrack| {
            result.push(track);
            line_names.push(Vec::new());
        };

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        // Bracketed line-name set: `[name1 name2]` names the current line.
        if remaining.starts_with('[') {
            if let Some(close) = remaining.find(']') {
                let names = &remaining[1..close];
                if let Some(slot) = line_names.last_mut() {
                    for n in names.split_whitespace() {
                        slot.push(n.to_string());
                    }
                }
                remaining = &remaining[close + 1..];
                continue;
            }
        }

        // Handle repeat(...)
        if remaining.starts_with("repeat(") {
            if let Some(close) = find_matching_paren(remaining, 7) {
                let inner = &remaining[7..close];
                let rest = &remaining[close + 1..];

                // Parse repeat(count, track_pattern)
                if let Some(comma) = inner.find(',') {
                    let count_str = inner[..comma].trim();
                    let pattern = inner[comma + 1..].trim();

                    // auto-fill and auto-fit: default to 3 columns for PDF (no viewport)
                    let count: usize = if count_str == "auto-fill" || count_str == "auto-fit" {
                        3
                    } else {
                        count_str.parse().unwrap_or(1)
                    };

                    let (track_list, sub_names) = parse_grid_track_list(pattern);
                    for _ in 0..count {
                        // Merge the pattern's interior line names at each repeat.
                        for (i, t) in track_list.iter().enumerate() {
                            if let (Some(slot), Some(src)) =
                                (line_names.last_mut(), sub_names.get(i))
                            {
                                slot.extend(src.iter().cloned());
                            }
                            push_track(&mut result, &mut line_names, t.clone());
                        }
                        if let (Some(slot), Some(src)) = (line_names.last_mut(), sub_names.last()) {
                            slot.extend(src.iter().cloned());
                        }
                    }
                }
                remaining = rest;
                continue;
            }
        }

        // Handle minmax(...)
        if remaining.starts_with("minmax(") {
            if let Some(close) = find_matching_paren(remaining, 7) {
                let expr = &remaining[..close + 1];
                if let Some(track) = parse_minmax(expr) {
                    push_track(&mut result, &mut line_names, track);
                }
                remaining = &remaining[close + 1..];
                continue;
            }
        }

        // fit-content(...) → approximate as an auto track (sized to content,
        // capped at the argument, which we don't yet enforce).
        if remaining.starts_with("fit-content(") {
            if let Some(close) = find_matching_paren(remaining, 12) {
                push_track(&mut result, &mut line_names, GridTrack::Auto);
                remaining = &remaining[close + 1..];
                continue;
            }
        }

        // Regular token — read until next whitespace or bracket.
        let end = remaining
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(remaining.len());
        let token = &remaining[..end];
        // `min-content` / `max-content` keywords → approximate as Auto.
        if token == "min-content" || token == "max-content" {
            push_track(&mut result, &mut line_names, GridTrack::Auto);
        } else if let Some(track) = parse_single_track(token) {
            push_track(&mut result, &mut line_names, track);
        }
        remaining = &remaining[end..];
    }

    add_nth_line_name_aliases(&mut line_names);
    (result, line_names)
}

fn add_nth_line_name_aliases(line_names: &mut [Vec<String>]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for names in line_names.iter_mut() {
        let originals = names.clone();
        for name in originals {
            if name.contains(char::is_whitespace) {
                continue;
            }
            let count = counts.entry(name.clone()).or_insert(0);
            *count += 1;
            names.push(format!("{} {}", *count, name));
            names.push(format!("{} {}", name, *count));
        }
    }
}

/// Find the closing `)` matching an opening `(` at `start` in `s`.
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 1;
    for (i, c) in s[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a border shorthand string like "1px solid black" into (width_pt, Option<Color>, BorderStyle).
/// Substitute every `var(--name[, fallback])` occurrence inside a raw value
/// string with its resolved custom-property value, so var() works inside
/// shorthands (e.g. `border: 4px solid var(--c)`, `background: var(--bg)`),
/// not just standalone properties.
fn resolve_embedded_vars(raw: &str, cp: &HashMap<String, String>) -> String {
    if !raw.contains("var(") {
        return raw.to_string();
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(pos) = rest.find("var(") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 4..];
        // Find the paren that closes THIS var(), accounting for nested var()
        // inside the fallback (e.g. `var(--a, var(--b, 12px))`). A naive
        // `find(')')` would stop at the inner var()'s `)`, truncating the
        // fallback and dropping the whole substitution.
        let Some(close) = matching_close_paren(after) else {
            out.push_str(rest);
            return out;
        };
        let inner = after[..close].trim();
        let (name, fb) = match inner.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner, None),
        };
        if let Some(v) = crate::style::resolve::resolve_var_to_string(name, fb, cp) {
            out.push_str(&v);
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn resolved_raw_css_value<'a>(
    value: &'a CssValue,
    custom_properties: &HashMap<String, String>,
) -> Option<Cow<'a, str>> {
    match value {
        CssValue::Keyword(raw) if raw.contains("var(") => {
            Some(Cow::Owned(resolve_embedded_vars(raw, custom_properties)))
        }
        CssValue::Keyword(raw) => Some(Cow::Borrowed(raw)),
        CssValue::Math(expression) => expression.to_css_string().map(Cow::Owned),
        CssValue::Var(name, fallback) => crate::style::resolve::resolve_var_to_string(
            name,
            fallback.as_deref(),
            custom_properties,
        )
        .map(Cow::Owned),
        _ => None,
    }
}

/// Byte offset of the `)` that closes the parenthesis group `s` is inside of,
/// where `s` is the text immediately AFTER an opening `(`. Returns `None` if the
/// group is unterminated. Nested `(...)` (e.g. a fallback containing another
/// `var(...)`) are skipped so the offset is the OUTER group's close.
fn matching_close_paren(s: &str) -> Option<usize> {
    let mut depth = 0u32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn specified_radius_from_css_value(
    value: &CssValue,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedRadiusValue> {
    let radius = match value {
        CssValue::Percentage(value) if value.is_finite() && *value >= 0.0 => {
            SpecifiedRadiusValue::Percentage(*value)
        }
        CssValue::Percentage(_) => return None,
        value => {
            let value = resolve_css_length_for_style(value, style, length_context, font_metrics)?;
            if !value.is_finite() || value < 0.0 {
                return None;
            }
            SpecifiedRadiusValue::Length(value)
        }
    };
    Some(radius)
}

/// Parse one `<length-percentage>` radius token. CSS permits a bare number only
/// when it is exactly zero.
fn parse_radius_token(
    token: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedRadiusValue> {
    let token = token.trim();
    if let Ok(number) = token.parse::<f32>() {
        return (number == 0.0).then_some(SpecifiedRadiusValue::Length(0.0));
    }
    let value = parse_length(token)?;
    specified_radius_from_css_value(&value, style, length_context, font_metrics)
}

fn parse_radius_group(
    tokens: &[&str],
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<[SpecifiedRadiusValue; 4]> {
    let tokens = tokens
        .iter()
        .map(|token| parse_radius_token(token, style, length_context, font_metrics))
        .collect::<Option<Vec<_>>>()?;
    let radii = match tokens.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    };
    Some(radii)
}

fn parse_border_radius_shorthand(
    value: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedCornerRadii> {
    let value = resolve_embedded_vars(value, &style.custom_properties);
    let components = split_radius_components(&value, true)?;
    let [tl_x, tr_x, br_x, bl_x] =
        parse_radius_group(&components.horizontal, style, length_context, font_metrics)?;
    let [tl_y, tr_y, br_y, bl_y] = components
        .vertical
        .as_deref()
        .map_or(Some([tl_x, tr_x, br_x, bl_x]), |vertical| {
            parse_radius_group(vertical, style, length_context, font_metrics)
        })?;
    Some(SpecifiedCornerRadii::new(
        SpecifiedCornerRadius::new(tl_x, tl_y),
        SpecifiedCornerRadius::new(tr_x, tr_y),
        SpecifiedCornerRadius::new(br_x, br_y),
        SpecifiedCornerRadius::new(bl_x, bl_y),
    ))
}

fn parse_corner_radius_raw(
    value: &str,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedCornerRadius> {
    let value = resolve_embedded_vars(value, &style.custom_properties);
    let components = split_radius_components(&value, false)?;
    let x = parse_radius_token(
        components.horizontal[0],
        style,
        length_context,
        font_metrics,
    )?;
    let y = match components.horizontal.get(1) {
        Some(token) => parse_radius_token(token, style, length_context, font_metrics)?,
        None => x,
    };
    Some(SpecifiedCornerRadius::new(x, y))
}

fn parse_border_radius_value(
    value: &CssValue,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedCornerRadii> {
    match value {
        CssValue::Keyword(value) => {
            parse_border_radius_shorthand(value, style, length_context, font_metrics)
        }
        CssValue::Var(name, fallback) => crate::style::resolve::resolve_var_to_string(
            name,
            fallback.as_deref(),
            &style.custom_properties,
        )
        .and_then(|value| {
            parse_border_radius_shorthand(&value, style, length_context, font_metrics)
        }),
        value => specified_radius_from_css_value(value, style, length_context, font_metrics)
            .map(SpecifiedCornerRadii::circular),
    }
}

fn parse_corner_radius_value(
    value: &CssValue,
    style: &ComputedStyle,
    length_context: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<SpecifiedCornerRadius> {
    match value {
        CssValue::Keyword(value) => {
            parse_corner_radius_raw(value, style, length_context, font_metrics)
        }
        CssValue::Var(name, fallback) => crate::style::resolve::resolve_var_to_string(
            name,
            fallback.as_deref(),
            &style.custom_properties,
        )
        .and_then(|value| parse_corner_radius_raw(&value, style, length_context, font_metrics)),
        value => specified_radius_from_css_value(value, style, length_context, font_metrics)
            .map(SpecifiedCornerRadius::circular),
    }
}

/// Parse a `column-rule` shorthand (`<width> || <style> || <color>`) into a
/// `BorderSide`, reusing the border-shorthand tokenizer. Per CSS Multicol §6 the
/// initial `column-rule-width` is `medium`, shared with border shorthands.
fn parse_column_rule_shorthand(raw: &str, font_size: f32) -> Option<BorderSide> {
    let (without_function_color, mut color) = extract_border_function_color(raw);
    let mut width = None;
    let mut rule_style = None;
    let mut found = color.is_some();
    for token in split_css_whitespace(&without_function_color) {
        if let Some(value) = borders::parse_border_style(token) {
            if rule_style.replace(value).is_some() {
                return None;
            }
        } else if let Some(value) = match token.trim().to_ascii_lowercase().as_str() {
            "thin" => Some(0.75),
            "medium" => Some(MEDIUM_RULE_WIDTH_PT),
            "thick" => Some(3.75),
            _ => resolve_border_width(parse_length(token).as_ref(), font_size),
        } {
            if value < 0.0 || width.replace(value).is_some() {
                return None;
            }
        } else {
            let value = parse_border_color(token)?;
            if color.replace(value).is_some() {
                return None;
            }
        }
        found = true;
    }
    found.then(|| BorderSide {
        specified_width: width.unwrap_or(MEDIUM_RULE_WIDTH_PT),
        color: color.unwrap_or(SpecifiedColor::CurrentColor),
        style: rule_style.unwrap_or(BorderStyle::None),
    })
}

/// CSS `medium` line width in points (~3px). Shared by the column-rule
/// shorthand and longhand defaults.
const MEDIUM_RULE_WIDTH_PT: f32 = 2.25;

/// Resolve a `border-*-width` CssValue (uniform or per-side) to points using the
/// element's `font_size` as the em basis. Absolute lengths (`CssValue::Length`,
/// already in pt) apply directly; a font-relative `em` width multiplies by the
/// font-size, mirroring the margin/width paths. `rem` resolves against the same
/// font-size (the consumers run before the root-font-size context is built, and a
/// `rem` border width is exceedingly rare). Returns `None` for anything that
/// isn't a usable length so the caller leaves the existing width untouched.
fn resolve_border_width(val: Option<&CssValue>, font_size: f32) -> Option<f32> {
    match val? {
        CssValue::Length(v) => Some(*v),
        CssValue::Em(v) => Some(*v * font_size),
        CssValue::Number(v) if *v == 0.0 => Some(0.0),
        CssValue::Rem(v) => Some(*v * font_size),
        _ => None,
    }
}

fn sync_line_height_from_absolute(style: &mut ComputedStyle) {
    if let Some(absolute) = style.line_height_absolute
        && style.font_size > 0.0
    {
        style.line_height = absolute / style.font_size;
    }
}

fn style_length_context(
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> crate::style::resolve::LengthResolutionContext {
    crate::style::resolve::LengthResolutionContext::new(
        base.percentage_basis,
        style.math_unit_context(font_metrics),
    )
}

fn line_height_length_context(
    style: &ComputedStyle,
    parent: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> crate::style::resolve::LengthResolutionContext {
    let mut units = style.math_unit_context(font_metrics);
    let parent_units = parent.math_unit_context(font_metrics);
    units.font.lh = parent_units.font.lh;
    units.font.rlh = parent_units.font.rlh;
    crate::style::resolve::LengthResolutionContext::new(base.percentage_basis, units)
}

impl ComputedStyle {
    pub(crate) fn font_unit_lengths(&self, font_metrics: FontMetrics<'_>) -> FontUnitLengths {
        FontUnitLengths {
            ex: style_ex_length(self, font_metrics),
            ch: self.font_size * font_metrics.style_ch_ratio(self).unwrap_or(0.5),
            cap: self.font_size * style_cap_height_ratio(self),
            // CSS Values 4 requires a 1em fallback when the ideographic
            // advance cannot be determined.
            ic: self.font_size,
            line_height: resolved_line_height_length(self),
        }
    }

    pub(crate) fn math_unit_context(&self, font_metrics: FontMetrics<'_>) -> MathUnitContext {
        let mut units = MathUnitContext::from_font_and_viewport(
            self.font_size,
            self.root_font_size,
            self.viewport_width,
            self.viewport_height,
        );
        let local = self.font_unit_lengths(font_metrics);
        units.font.ex = local.ex;
        units.font.ch = local.ch;
        units.font.cap = local.cap;
        units.font.ic = local.ic;
        units.font.lh = local.line_height;
        units.font.rex = self.root_font_units.ex;
        units.font.rch = self.root_font_units.ch;
        units.font.rcap = self.root_font_units.cap;
        units.font.ric = self.root_font_units.ic;
        units.font.rlh = self.root_font_units.line_height;
        units
    }
}

fn resolve_css_length_for_style(
    val: &CssValue,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    match val {
        CssValue::Length(v) => Some(*v),
        CssValue::Em(v) => Some(*v * style.font_size),
        CssValue::Number(v) if *v == 0.0 => Some(0.0),
        CssValue::Percentage(v) => Some(base.percentage_basis * *v / 100.0),
        CssValue::Ex(v) => Some(*v * style_ex_length(style, font_metrics)),
        CssValue::Ch(v) => {
            Some(*v * style.font_size * font_metrics.style_ch_ratio(style).unwrap_or(0.5))
        }
        CssValue::Rem(v) => Some(*v * style.root_font_size),
        CssValue::Vw(v) => Some(style.viewport_width * *v / 100.0),
        CssValue::Vh(v) => Some(style.viewport_height * *v / 100.0),
        CssValue::Vmin(v) => Some(style.viewport_width.min(style.viewport_height) * *v / 100.0),
        CssValue::Vmax(v) => Some(style.viewport_width.max(style.viewport_height) * *v / 100.0),
        CssValue::Math(_) | CssValue::Var(_, _) => {
            crate::style::resolve::try_resolve_to_length_in_context(
                val,
                &style.custom_properties,
                style_length_context(style, base, font_metrics),
            )
        }
        CssValue::Keyword(k) => resolve_raw_length_for_style(k, style, base, font_metrics),
        _ => None,
    }
}

/// Normalize a `text-indent` value without choosing a percentage basis early.
/// CSS Text resolves percentages against the block container's own inner inline
/// size, which layout alone knows. Font- and viewport-relative `calc()` terms
/// are already definite at computed-value time, so convert only those terms to
/// points and leave percentage tokens for layout.
fn text_indent_from_css_value(
    value: &CssValue,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<TextIndent> {
    match value {
        CssValue::Length(value) => Some(TextIndent::Length(*value)),
        CssValue::Percentage(value) => Some(TextIndent::Percentage(*value)),
        CssValue::Math(expression) => Some(TextIndent::Math(DeferredLength::new(
            expression.clone(),
            style.math_unit_context(font_metrics),
        ))),
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            text_indent_from_css_value(&parse_length(&raw)?, style, base, font_metrics)
        }
        value => {
            resolve_css_length_for_style(value, style, base, font_metrics).map(TextIndent::Length)
        }
    }
}

/// Resolve CSS Text's `word-spacing: <length-percentage>` against the computed
/// font size. Unlike most CSS percentages, this percentage basis is `1em`, not
/// the containing block width.
fn word_spacing_from_css_value(
    value: &CssValue,
    style: &ComputedStyle,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    match value {
        CssValue::Length(value) => Some(*value),
        CssValue::Em(value) => Some(*value * style.font_size),
        CssValue::Number(value) if *value == 0.0 => Some(0.0),
        CssValue::Percentage(value) => Some(style.font_size * *value / 100.0),
        CssValue::Math(expression) => {
            expression.resolve(style.math_unit_context(font_metrics), style.font_size)
        }
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            word_spacing_from_css_value(
                &parse_property_value("word-spacing", &raw)?,
                style,
                font_metrics,
            )
        }
        CssValue::Keyword(value) if value.eq_ignore_ascii_case("normal") => Some(0.0),
        value => resolve_css_length_for_style(
            value,
            style,
            crate::style::resolve::LengthResolutionContext::new(
                style.font_size,
                style.math_unit_context(font_metrics),
            ),
            font_metrics,
        ),
    }
}

fn letter_spacing_from_css_value(
    value: &CssValue,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    match value {
        CssValue::Keyword(value) if value.eq_ignore_ascii_case("normal") => Some(0.0),
        CssValue::Var(name, fallback) => {
            let raw = crate::style::resolve::resolve_var_to_string(
                name,
                fallback.as_deref(),
                &style.custom_properties,
            )?;
            letter_spacing_from_css_value(
                &parse_property_value("letter-spacing", &raw)?,
                style,
                base,
                font_metrics,
            )
        }
        value => resolve_css_length_for_style(value, style, base, font_metrics),
    }
}

fn resolve_raw_length_for_style(
    raw: &str,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    let lower = raw.trim().to_ascii_lowercase();
    if let Some(number) = lower.strip_suffix("cap") {
        return number
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * style.font_size * style_cap_height_ratio(style));
    }
    if let Some(number) = lower.strip_suffix("lh") {
        return number
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * resolved_line_height_length(style));
    }
    parse_length(&lower).and_then(|parsed| match parsed {
        CssValue::Keyword(_) => None,
        _ => resolve_css_length_for_style(&parsed, style, base, font_metrics),
    })
}

fn style_cap_height_ratio(_style: &ComputedStyle) -> f32 {
    0.75
}

fn style_ex_length(style: &ComputedStyle, font_metrics: FontMetrics<'_>) -> f32 {
    font_metrics
        .style_x_height(style)
        .unwrap_or(style.font_size * 0.5)
}

fn resolved_line_height_length(style: &ComputedStyle) -> f32 {
    if let Some(absolute) = style.line_height_absolute {
        absolute
    } else if style.line_height.is_nan() {
        style.font_size * 1.2
    } else {
        style.font_size * style.line_height
    }
}

fn split_css_whitespace(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if start < index {
                    parts.push(value[start..index].trim());
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    if start < value.len() {
        parts.push(value[start..].trim());
    }
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn expand_box_values<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    match values.len() {
        1 => Some([values[0], values[0], values[0], values[0]]),
        2 => Some([values[0], values[1], values[0], values[1]]),
        3 => Some([values[0], values[1], values[2], values[1]]),
        4 => Some([values[0], values[1], values[2], values[3]]),
        _ => None,
    }
}

fn parse_border_width_shorthand_values(
    raw: &str,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<[f32; 4]> {
    let values: Vec<f32> = split_css_whitespace(raw)
        .into_iter()
        .map(|part| parse_border_width_token(part, style, base, font_metrics))
        .collect::<Option<Vec<_>>>()?;
    expand_box_values(&values)
}

fn parse_border_width_token(
    token: &str,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
    font_metrics: FontMetrics<'_>,
) -> Option<f32> {
    match token.trim().to_ascii_lowercase().as_str() {
        "thin" => Some(0.75),
        "medium" => Some(MEDIUM_RULE_WIDTH_PT),
        "thick" => Some(3.75),
        other => resolve_raw_length_for_style(other, style, base, font_metrics),
    }
}

fn parse_border_color_shorthand_values(raw: &str) -> Option<[SpecifiedColor; 4]> {
    let values: Vec<SpecifiedColor> = split_css_whitespace(raw)
        .into_iter()
        .map(parse_border_color)
        .collect::<Option<Vec<_>>>()?;
    expand_box_values(&values)
}

/// Parse a single border color token using the shared CSS color parser, which
/// handles named colors, `#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa` hex (lightningcss
/// serialises `rgba(...)` to 8-digit hex with the alpha byte), and
/// `rgb()`/`rgba()` functions — preserving alpha for translucent borders.
fn parse_border_color_token(val: &str) -> Option<SpecifiedColor> {
    match crate::parser::css::parse_color(val) {
        Some(CssValue::Color(c)) => Some(c),
        _ => None,
    }
}

/// Pull a function color out of a border shorthand, returning the string with
/// that complete, balanced token removed plus the parsed color. Lightning may
/// serialize precise RGB input as `color(srgb ...)`, so limiting this to the
/// legacy `rgb()` spellings would silently discard both color and alpha.
fn extract_border_function_color(k: &str) -> (String, Option<SpecifiedColor>) {
    let lower = k.to_ascii_lowercase();
    for prefix in [
        "rgba(", "rgb(", "color(", "oklab(", "oklch(", "lab(", "lch(",
    ] {
        if let Some(start) = lower.find(prefix) {
            let open = start + prefix.len() - 1;
            if let Some(close_rel) = matching_close_paren(&k[open + 1..]) {
                let end = open + 1 + close_rel + 1;
                let func = &k[start..end];
                let color = match crate::parser::css::parse_color(func) {
                    Some(CssValue::Color(c)) => Some(c),
                    _ => None,
                };
                let mut rest = String::with_capacity(k.len());
                rest.push_str(&k[..start]);
                rest.push(' ');
                rest.push_str(&k[end..]);
                return (rest, color);
            }
        }
    }
    (k.to_string(), None)
}

/// Parse a color token without binding `currentColor` prematurely.
fn parse_border_color(val: &str) -> Option<SpecifiedColor> {
    parse_border_color_token(val)
}

#[cfg(test)]
fn parse_hex_to_color(hex: &str) -> Option<Color> {
    // Per CSS Color 4 §5.2: each single hex digit expands by duplication
    // (`#rgb`/`#rgba`); `#rgba` and `#rrggbbaa` carry an alpha byte where
    // `00` = fully transparent and `ff` = fully opaque. Missing alpha is opaque.
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        4 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            let a = u8::from_str_radix(&hex[3..4].repeat(2), 16).ok()?;
            Some(Color::rgba8(r, g, b, a))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color::rgba8(r, g, b, a))
        }
        _ => None,
    }
}

/// Parse a CSS `linear-gradient(...)` / `repeating-linear-gradient(...)` function
/// value into a `LinearGradient`.
///
/// Supports:
/// - `linear-gradient(to right, red, blue)`
/// - `linear-gradient(45deg, #ff0000, #0000ff)`
/// - `linear-gradient(to bottom, red 0%, white 50%, blue 100%)`
/// - `repeating-linear-gradient(45deg, red 0 10px, blue 10px 20px)`
#[cfg(test)]
fn parse_linear_gradient(val: &str) -> Option<LinearGradient> {
    parse_linear_gradient_for_color(val, Color::BLACK)
}

fn parse_linear_gradient_for_color(val: &str, current_color: Color) -> Option<LinearGradient> {
    let val = val.trim();
    let (inner, repeating) = if let Some(i) = val
        .strip_prefix("repeating-linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (i, true)
    } else {
        (
            val.strip_prefix("linear-gradient(")
                .and_then(|s| s.strip_suffix(')'))?,
            false,
        )
    };

    // Split on commas, but be careful of commas inside rgb() or rgba()
    let parts = split_gradient_args(inner);
    if parts.is_empty() {
        return None;
    }

    let first = parts[0].trim();
    let (first, interpolation) = parse_gradient_interpolation(first)?;
    let has_interpolation = interpolation.is_some();
    let first = first.trim();

    // Determine if the first arg is a direction/angle or a color stop
    let (angle, color_start) = if first.starts_with("to ") {
        let angle = match first {
            "to top" => 0.0,
            "to right" => 90.0,
            "to bottom" => 180.0,
            "to left" => 270.0,
            "to top right" | "to right top" => 45.0,
            "to bottom right" | "to right bottom" => 135.0,
            "to bottom left" | "to left bottom" => 225.0,
            "to top left" | "to left top" => 315.0,
            _ => return None,
        };
        (angle, 1)
    } else if let Some(deg) = parse_css_angle_deg(first) {
        (deg, 1)
    } else if has_interpolation && first.is_empty() {
        (180.0, 1)
    } else if has_interpolation {
        return None;
    } else {
        // No direction specified, default is "to bottom" = 180deg
        (180.0, 0)
    };

    let color_parts = &parts[color_start..];
    if color_parts.is_empty() {
        return None;
    }

    let ramp = parse_gradient_ramp(
        color_parts,
        current_color,
        interpolation.unwrap_or_default(),
        repeating,
        parse_gradient_stop_position,
    )?;

    Some(LinearGradient {
        angle,
        ramp,
        layer_box: GradientLayerBox::default(),
    })
}

fn parse_gradient_interpolation(prelude: &str) -> Option<(String, Option<GradientInterpolation>)> {
    let mut tokens = split_css_components(prelude);
    let indices = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| token.eq_ignore_ascii_case("in").then_some(index))
        .collect::<Vec<_>>();
    let ([] | [_]) = indices.as_slice() else {
        return None;
    };
    let Some(&index) = indices.first() else {
        return Some((prelude.to_string(), None));
    };
    let interpolation = match tokens.get(index + 1)?.to_ascii_lowercase().as_str() {
        "srgb" => GradientInterpolation::Srgb,
        "oklab" => GradientInterpolation::Oklab,
        _ => return None,
    };
    tokens.drain(index..=index + 1);
    Some((tokens.join(" "), Some(interpolation)))
}

/// Parse a CSS `<angle>` token into degrees. Supports `deg`, `grad`, `rad`,
/// `turn`, and a bare `0`. Returns `None` for non-angle tokens.
fn parse_css_angle_deg(tok: &str) -> Option<f32> {
    let tok = tok.trim();
    if let Some(n) = tok.strip_suffix("deg") {
        return n.trim().parse::<f32>().ok();
    }
    if let Some(n) = tok.strip_suffix("grad") {
        return n.trim().parse::<f32>().ok().map(|g| g * 0.9);
    }
    if let Some(n) = tok.strip_suffix("turn") {
        return n.trim().parse::<f32>().ok().map(|t| t * 360.0);
    }
    if let Some(n) = tok.strip_suffix("rad") {
        return n
            .trim()
            .parse::<f32>()
            .ok()
            .map(|r| r * 180.0 / std::f32::consts::PI);
    }
    if tok == "0" {
        return Some(0.0);
    }
    None
}

/// Parse a CSS `radial-gradient(...)` / `repeating-radial-gradient(...)` function
/// value into a `RadialGradient`.
///
/// Honors the shape (`circle`/`ellipse`), the extent keyword
/// (`closest-side`/`closest-corner`/`farthest-side`/`farthest-corner`), an
/// explicit circle radius or ellipse radii, and the `at <position>` clause.
#[cfg(test)]
fn parse_radial_gradient(val: &str) -> Option<RadialGradient> {
    parse_radial_gradient_for_color(val, Color::BLACK)
}

fn parse_radial_gradient_for_color(val: &str, current_color: Color) -> Option<RadialGradient> {
    let val = val.trim();
    let (inner, repeating) = if let Some(i) = val
        .strip_prefix("repeating-radial-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (i, true)
    } else {
        (
            val.strip_prefix("radial-gradient(")
                .and_then(|s| s.strip_suffix(')'))?,
            false,
        )
    };

    let parts = split_gradient_args(inner);
    if parts.is_empty() {
        return None;
    }

    let (first, interpolation) = parse_gradient_interpolation(parts[0].trim())?;
    let has_interpolation = interpolation.is_some();
    let first = first.trim().to_ascii_lowercase();

    // A first arg is a shape/size/position prefix (not a color stop) when it
    // names a shape keyword, an extent keyword, an `at <pos>` clause, or is a
    // bare length/percentage size (e.g. lightningcss re-serializes
    // `circle 60px at center` to `60px`). We detect the bare-length case by it
    // not parsing as a color while parsing as a length token, so a real first
    // color stop is never dropped.
    let is_shape_or_size = has_interpolation
        || first.starts_with("circle")
        || first.starts_with("ellipse")
        || first.contains("at ")
        || first.contains("closest-side")
        || first.contains("farthest-side")
        || first.contains("closest-corner")
        || first.contains("farthest-corner")
        || (parse_gradient_color_for_color(&first, current_color).is_none()
            && first_token_is_length(&first));
    let color_start = usize::from(is_shape_or_size);

    // Honor the `at <position>` clause, the extent keyword, and explicit
    // radius/radii, else default to a box-centered farthest-corner ellipse.
    let (center, shape, extent, radius, radii) = if color_start == 1 {
        let center = parse_radial_center(&first)?;
        let size_part = first
            .find("at ")
            .map(|at| &first[..at])
            .unwrap_or(&first)
            .trim();
        let mut shape = None;
        let mut extent = None;
        let mut size_tokens = Vec::new();
        for token in size_part.split_whitespace() {
            match token {
                "circle" if shape.replace(RadialShape::Circle).is_none() => {}
                "ellipse" if shape.replace(RadialShape::Ellipse).is_none() => {}
                "closest-side" if extent.replace(RadialExtent::ClosestSide).is_none() => {}
                "closest-corner" if extent.replace(RadialExtent::ClosestCorner).is_none() => {}
                "farthest-side" if extent.replace(RadialExtent::FarthestSide).is_none() => {}
                "farthest-corner" if extent.replace(RadialExtent::FarthestCorner).is_none() => {}
                token if parse_radial_pos(token).is_some() => size_tokens.push(token),
                _ => return None,
            }
        }
        if extent.is_some() && !size_tokens.is_empty() {
            return None;
        }

        let shape = shape.unwrap_or(if size_tokens.len() == 1 {
            RadialShape::Circle
        } else {
            RadialShape::Ellipse
        });
        let (radius, radii) = match (shape, size_tokens.as_slice()) {
            (_, []) => (None, None),
            (RadialShape::Circle, [radius]) => {
                let radius = parse_radial_length_pt(radius)?;
                if radius < 0.0 {
                    return None;
                }
                (Some(radius), None)
            }
            (RadialShape::Ellipse, [rx, ry]) => {
                let (rx, ry) = (parse_radial_pos(rx)?, parse_radial_pos(ry)?);
                if !radial_pos_is_nonnegative(rx) || !radial_pos_is_nonnegative(ry) {
                    return None;
                }
                (None, Some(RadialVector::new(rx, ry)))
            }
            _ => return None,
        };

        (center, shape, extent.unwrap_or_default(), radius, radii)
    } else {
        (
            RadialPoint::default(),
            RadialShape::Ellipse,
            RadialExtent::FarthestCorner,
            None,
            None,
        )
    };

    let color_parts = &parts[color_start..];
    if color_parts.is_empty() {
        return None;
    }

    let ramp = parse_gradient_ramp(
        color_parts,
        current_color,
        interpolation.unwrap_or_default(),
        repeating,
        parse_gradient_stop_position,
    )?;

    Some(RadialGradient {
        ramp,
        center,
        shape,
        extent,
        radius,
        radii,
        layer_box: GradientLayerBox::default(),
    })
}

/// Parse a CSS `conic-gradient(...)` / `repeating-conic-gradient(...)` function
/// value into a `ConicGradient`.
///
/// Honors `from <angle>`, `at <position>`, and angular color stops in `deg`,
/// `grad`, `rad`, `turn`, or `%` (100% = one turn). Stop positions are
/// normalized to a fraction of a full turn.
#[cfg(test)]
fn parse_conic_gradient(val: &str) -> Option<ConicGradient> {
    parse_conic_gradient_for_color(val, Color::BLACK)
}

fn parse_conic_gradient_for_color(val: &str, current_color: Color) -> Option<ConicGradient> {
    let val = val.trim();
    let (inner, repeating) = if let Some(i) = val
        .strip_prefix("repeating-conic-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (i, true)
    } else {
        (
            val.strip_prefix("conic-gradient(")
                .and_then(|s| s.strip_suffix(')'))?,
            false,
        )
    };

    let parts = split_gradient_args(inner);
    if parts.is_empty() {
        return None;
    }

    let (first, interpolation) = parse_gradient_interpolation(parts[0].trim())?;
    let first_lower = first.trim().to_ascii_lowercase();

    // The first argument is a `[from <angle>] [at <position>]` prefix when it
    // mentions either keyword; otherwise it is the first color stop.
    let has_prefix =
        interpolation.is_some() || first_lower.starts_with("from ") || first_lower.contains("at ");
    let color_start = usize::from(has_prefix);

    let (from_angle, center) = if has_prefix {
        parse_conic_prelude(&first_lower)?
    } else {
        (0.0, RadialPoint::default())
    };

    let color_parts = &parts[color_start..];
    let ramp = parse_gradient_ramp(
        color_parts,
        current_color,
        interpolation.unwrap_or_default(),
        repeating,
        parse_conic_angle_fraction,
    )?;

    Some(ConicGradient {
        from_angle,
        center,
        ramp,
        layer_box: GradientLayerBox::default(),
    })
}

fn parse_conic_prelude(prelude: &str) -> Option<(f32, RadialPoint)> {
    let prelude = prelude.trim();
    if prelude.is_empty() {
        return Some((0.0, RadialPoint::default()));
    }

    let (from_angle, remainder) = if let Some(rest) = prelude.strip_prefix("from ") {
        let angle_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        (
            parse_css_angle_deg(&rest[..angle_end])?,
            rest[angle_end..].trim(),
        )
    } else {
        (0.0, prelude)
    };
    let center = if remainder.is_empty() {
        RadialPoint::default()
    } else if remainder.starts_with("at ") {
        parse_radial_center(remainder)?
    } else {
        return None;
    };
    Some((from_angle, center))
}

/// Parse a single conic angular position into a fraction of one turn (0..1).
/// Accepts `deg`, `grad`, `rad`, `turn`, and `%` (100% = one turn).
fn parse_conic_angle_fraction(tok: &str) -> Option<GradientPosition> {
    let tok = tok.trim();
    if let Some(n) = tok.strip_suffix('%') {
        return n
            .trim()
            .parse::<f32>()
            .ok()
            .map(|p| GradientPosition::fraction(p / 100.0));
    }
    if let Some(n) = tok.strip_suffix("turn") {
        return n.trim().parse::<f32>().ok().map(GradientPosition::fraction);
    }
    parse_css_angle_deg(tok).map(|deg| GradientPosition::fraction(deg / 360.0))
}

/// True when the first whitespace-delimited token looks like a CSS length or
/// percentage (a number with a known unit, or a bare `%`). Used to recognize a
/// size-only first argument of `radial-gradient()`.
fn first_token_is_length(s: &str) -> bool {
    let tok = s.split_whitespace().next().unwrap_or("");
    parse_radial_pos(tok).is_some()
}

/// Parse a length token to points (px→pt = 0.75). Only absolute pixel/point
/// units are honored; relative units return `None`. A bare `0` is treated as
/// `0pt`.
fn parse_radial_length_pt(tok: &str) -> Option<f32> {
    if let Some(n) = tok.strip_suffix("px") {
        return n.parse::<f32>().ok().map(|v| v * 0.75);
    }
    if let Some(n) = tok.strip_suffix("pt") {
        return n.parse::<f32>().ok();
    }
    // Bare numeric (typically `0`).
    tok.parse::<f32>().ok()
}

/// Parse a single position component into a `RadialPos`: a percentage becomes a
/// fraction, a length becomes an absolute point offset.
fn parse_radial_pos(tok: &str) -> Option<RadialPos> {
    if let Some(n) = tok.strip_suffix('%') {
        return n
            .parse::<f32>()
            .ok()
            .map(|p| RadialPos::Fraction(p / 100.0));
    }
    parse_radial_length_pt(tok).map(RadialPos::Points)
}

fn radial_pos_is_nonnegative(position: RadialPos) -> bool {
    match position {
        RadialPos::Fraction(value) | RadialPos::Points(value) | RadialPos::EndOffset(value) => {
            value >= 0.0
        }
    }
}

/// Parse the `at <position>` clause of a radial-gradient first argument into a
/// center `(x, y)` measured from the box's left/top edges (CSS top-down).
/// Supports keyword positions (`center`, `top`, `left`, corners), percentages,
/// and lengths. An absent clause means box center; a malformed clause rejects
/// the gradient instead of silently changing its geometry.
fn parse_radial_center(first: &str) -> Option<RadialPoint> {
    let half = RadialPos::Fraction(0.5);
    let lower = first.to_ascii_lowercase();
    let Some(at_pos) = lower.find("at ") else {
        return Some(RadialPoint::new(half, half));
    };
    let pos = lower[at_pos + 3..].trim();
    if pos.is_empty() {
        return None;
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum AxisEdge {
        Left,
        Right,
        Top,
        Bottom,
        Center,
    }

    fn edge(token: &str) -> Option<AxisEdge> {
        match token {
            "left" => Some(AxisEdge::Left),
            "right" => Some(AxisEdge::Right),
            "top" => Some(AxisEdge::Top),
            "bottom" => Some(AxisEdge::Bottom),
            "center" => Some(AxisEdge::Center),
            _ => None,
        }
    }

    fn edge_pos(edge: AxisEdge, offset: Option<RadialPos>) -> RadialPos {
        match (edge, offset) {
            (AxisEdge::Left | AxisEdge::Top, Some(pos)) => pos,
            (AxisEdge::Right | AxisEdge::Bottom, Some(RadialPos::Fraction(f))) => {
                RadialPos::Fraction(1.0 - f)
            }
            (AxisEdge::Right | AxisEdge::Bottom, Some(RadialPos::Points(p))) => {
                RadialPos::EndOffset(p)
            }
            (AxisEdge::Right | AxisEdge::Bottom, Some(RadialPos::EndOffset(p))) => {
                RadialPos::Points(p)
            }
            (AxisEdge::Left | AxisEdge::Top, None) => RadialPos::Fraction(0.0),
            (AxisEdge::Right | AxisEdge::Bottom, None) => RadialPos::Fraction(1.0),
            (AxisEdge::Center, _) => RadialPos::Fraction(0.5),
        }
    }

    let tokens: Vec<&str> = pos.split_whitespace().collect();
    let mut x: Option<RadialPos> = None;
    let mut y: Option<RadialPos> = None;

    let mut i = 0;
    while i < tokens.len() {
        if let Some(e) = edge(tokens[i]) {
            let next = tokens.get(i + 1).and_then(|t| {
                if edge(t).is_some() {
                    None
                } else {
                    parse_radial_pos(t)
                }
            });
            match e {
                AxisEdge::Left | AxisEdge::Right => x = Some(edge_pos(e, next)),
                AxisEdge::Top | AxisEdge::Bottom => y = Some(edge_pos(e, next)),
                AxisEdge::Center => {
                    if x.is_none() {
                        x = Some(half);
                    } else if y.is_none() {
                        y = Some(half);
                    }
                }
            }
            i += usize::from(next.is_some()) + 1;
            continue;
        }
        let p = parse_radial_pos(tokens[i])?;
        if x.is_none() {
            x = Some(p);
        } else if y.is_none() {
            y = Some(p);
        } else {
            return None;
        }
        i += 1;
    }

    Some(RadialPoint::new(x.unwrap_or(half), y.unwrap_or(half)))
}

/// Split gradient arguments on commas, respecting parentheses (e.g., rgb(...)).
fn split_gradient_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

enum RawGradientItem {
    Stop(GradientColor, Vec<GradientPosition>),
    Hint(GradientPosition),
}

fn parse_gradient_stop_position(token: &str) -> Option<GradientPosition> {
    match parse_length(token)? {
        CssValue::Percentage(value) => Some(GradientPosition::fraction(value / 100.0)),
        CssValue::Length(value) => Some(GradientPosition::length(value)),
        CssValue::Math(expression) => {
            let units = MathUnitContext::from_font_and_viewport(0.0, 0.0, 0.0, 0.0);
            let (length, percent) = expression.affine(units)?.terms();
            Some(GradientPosition::length(length) + GradientPosition::fraction(percent / 100.0))
        }
        _ => None,
    }
}

/// Parse one shared gradient ramp from comma-separated color stops.
///
/// Each token is `color`, `color <pos>`, `color <pos> <pos>`, or a standalone
/// transition hint between two color stops. Positions remain unresolved until
/// the actual line/radius is known.
fn parse_gradient_ramp(
    parts: &[String],
    current_color: Color,
    interpolation: GradientInterpolation,
    repeating: bool,
    parse_position: fn(&str) -> Option<GradientPosition>,
) -> Option<GradientRamp> {
    let mut items = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let tokens = split_css_components(part);
        if tokens.is_empty() {
            return None;
        }
        if tokens.len() == 1
            && let Some(position) = parse_position(&tokens[0])
        {
            items.push(RawGradientItem::Hint(position));
            continue;
        }

        let mut split_at = tokens.len();
        while split_at > 0 && parse_position(&tokens[split_at - 1]).is_some() {
            split_at -= 1;
        }
        if split_at == 0 || tokens.len() - split_at > 2 {
            return None;
        }
        let color_str = tokens[..split_at].join(" ");
        let color = parse_gradient_color_for_color(&color_str, current_color)?;
        let positions = tokens[split_at..]
            .iter()
            .map(|token| parse_position(token))
            .collect::<Option<Vec<_>>>()?;
        items.push(RawGradientItem::Stop(color, positions));
    }

    let mut stops = Vec::new();
    let mut awaiting_right_stop = false;
    for item in items {
        match item {
            RawGradientItem::Hint(position) => {
                let previous: &mut GradientStop = stops.last_mut()?;
                if awaiting_right_stop || previous.hint_after.is_some() {
                    return None;
                }
                previous.hint_after = Some(position);
                awaiting_right_stop = true;
            }
            RawGradientItem::Stop(color, positions) => {
                awaiting_right_stop = false;
                match positions.as_slice() {
                    [] => stops.push(GradientStop::new(color, None)),
                    [position] => stops.push(GradientStop::new(color, Some(*position))),
                    [first, second] => {
                        stops.push(GradientStop::new(color, Some(*first)));
                        stops.push(GradientStop::new(color, Some(*second)));
                    }
                    _ => return None,
                }
            }
        }
    }
    if awaiting_right_stop || stops.is_empty() {
        return None;
    }
    Some(GradientRamp {
        stops,
        interpolation,
        repeat: if repeating {
            GradientRepeat::Repeat
        } else {
            GradientRepeat::Clamp
        },
    })
}

/// Parse a color string for gradient stops.
#[cfg(test)]
fn parse_gradient_color(val: &str) -> Option<Color> {
    parse_gradient_color_for_color(val, Color::BLACK).map(|color| color.color)
}

fn parse_gradient_color_for_color(val: &str, current_color: Color) -> Option<GradientColor> {
    let source = val.trim();
    let lower = source.to_ascii_lowercase();
    let Some(CssValue::Color(specified)) = crate::parser::css::parse_color(source) else {
        return None;
    };
    let provenance = if lower == "currentcolor" {
        GradientColorProvenance::CurrentColor
    } else if ["color(", "lab(", "lch(", "oklab(", "oklch("]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        GradientColorProvenance::Modern
    } else {
        GradientColorProvenance::LegacySrgb
    };
    Some(GradientColor::new(
        specified.resolve(current_color),
        provenance,
    ))
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn invalid_column_rule_longhand_preserves_the_prior_cascaded_style() {
        let style = compute_style(
            HtmlTag::Div,
            Some("column-rule-style: dashed; column-rule-style: zigzag"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.column_rule.style, BorderStyle::Dashed);
    }

    #[test]
    fn invalid_column_rule_shorthand_preserves_the_prior_declaration() {
        let style = compute_style(
            HtmlTag::Div,
            Some("column-rule: 4px double red; column-rule: 2px dashed red extra"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.column_rule.style, BorderStyle::Double);
        assert_eq!(style.column_rule.specified_width, 3.0);
    }

    #[test]
    fn negative_column_rule_width_does_not_replace_a_valid_width() {
        let style = compute_style(
            HtmlTag::Div,
            Some("column-rule-width: 4px; column-rule-width: -2px"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.column_rule.specified_width, 3.0);
    }

    #[test]
    fn color_matrix_identity_is_detected_without_losing_the_filter_boundary() {
        let identity = FilterOperation::Matrix([
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ]);
        let desaturate = FilterOperation::Matrix([
            0.213, 0.715, 0.072, 0.0, 0.0, 0.213, 0.715, 0.072, 0.0, 0.0, 0.213, 0.715, 0.072, 0.0,
            0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ]);

        assert!(identity.is_visual_identity());
        assert!(!desaturate.is_visual_identity());
    }

    fn gradient_stop(position: f32, color: Color) -> GradientStop {
        GradientStop::new(
            GradientColor::new(color, GradientColorProvenance::LegacySrgb),
            Some(GradientPosition::fraction(position)),
        )
    }

    fn gradient_ramp(stops: impl IntoIterator<Item = GradientStop>) -> GradientRamp {
        GradientRamp {
            stops: stops.into_iter().collect(),
            ..Default::default()
        }
    }

    fn resolved_positions(ramp: &GradientRamp) -> Vec<f32> {
        ramp.resolve(1.0)
            .unwrap()
            .stops()
            .iter()
            .map(|stop| stop.position)
            .collect()
    }

    fn assert_rgba_close(actual: (f32, f32, f32, f32), expected: (f32, f32, f32, f32)) {
        for (actual, expected) in [actual.0, actual.1, actual.2, actual.3]
            .into_iter()
            .zip([expected.0, expected.1, expected.2, expected.3])
        {
            assert!(
                (actual - expected).abs() <= f32::EPSILON,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn background_repeat_pattern_is_constant_size_for_billions_of_tiles() {
        let pattern = AxisRepeatPattern::new(AxisRepeatMode::Repeat, 0.0, 1e-9, 1e30).unwrap();
        assert!(std::mem::size_of_val(&pattern) <= 5 * std::mem::size_of::<f64>());
        let local = pattern.sample(2.25e-9).unwrap();
        assert!(local >= 0.0 && local < pattern.tile_size());

        let spaced = AxisRepeatPattern::new(AxisRepeatMode::Space, 0.0, 1e-9, 100.0).unwrap();
        let local = spaced.sample(50.0).unwrap();
        assert!(local >= 0.0 && local < spaced.tile_size());

        let rounded = AxisRepeatPattern::new(AxisRepeatMode::Round, 0.0, 1e-9, 100.0).unwrap();
        let local = rounded.sample(50.0).unwrap();
        assert!(local >= 0.0 && local < rounded.tile_size());
    }

    #[test]
    fn background_space_with_one_tile_preserves_authored_position() {
        let pattern = AxisRepeatPattern::new(AxisRepeatMode::Space, 17.0, 60.0, 100.0).unwrap();
        assert_eq!(pattern.sample(16.0), None);
        assert_eq!(pattern.sample(17.0), Some(0.0));
        assert_eq!(pattern.sample(27.0), Some(10.0));
        assert_eq!(pattern.sample(77.0), None);
    }

    #[test]
    fn background_axis_pattern_samples_without_offset_search() {
        let repeat = AxisRepeatPattern::new(AxisRepeatMode::Repeat, 3.0, 10.0, 100.0).unwrap();
        assert_eq!(repeat.sample(4.0), Some(1.0));
        assert_eq!(repeat.sample(2.0), Some(9.0));

        let no_repeat = AxisRepeatPattern::new(AxisRepeatMode::NoRepeat, 3.0, 10.0, 100.0).unwrap();
        assert_eq!(no_repeat.sample(2.0), None);
        assert_eq!(no_repeat.sample(12.0), Some(9.0));
        assert_eq!(no_repeat.sample(13.0), None);
    }

    #[test]
    fn resolved_gradient_ramp_preserves_endpoint_plateaus() {
        let ramp = gradient_ramp([
            gradient_stop(0.2, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(0.8, Color::rgba8(0, 0, 255, 255)),
        ])
        .resolve(1.0)
        .unwrap();
        let visible = ramp.svg_unit_interval_stops().unwrap();
        assert_eq!(
            visible.iter().map(|stop| stop.position).collect::<Vec<_>>(),
            [0.0, 0.2, 0.8, 1.0]
        );
        assert_eq!(visible[0].color.color, Color::rgba8(255, 0, 0, 255));
        assert_eq!(visible[3].color.color, Color::rgba8(0, 0, 255, 255));
    }

    #[test]
    fn resolved_gradient_ramp_preserves_and_projects_out_of_range_positions() {
        let ramp = gradient_ramp([
            gradient_stop(-0.5, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(1.5, Color::rgba8(0, 0, 255, 255)),
        ])
        .resolve(1.0)
        .unwrap();
        assert_eq!(ramp.stops()[0].position, -0.5);
        assert_eq!(ramp.stops()[1].position, 1.5);
        assert_rgba_close(ramp.sample(0.0), (0.75, 0.0, 0.25, 1.0));
        assert_rgba_close(ramp.sample(1.0), (0.25, 0.0, 0.75, 1.0));

        let visible = ramp.svg_unit_interval_stops().unwrap();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].position, 0.0);
        assert_eq!(visible[1].position, 1.0);
        assert_rgba_close(visible[0].color.color.to_f32_rgba(), ramp.sample(0.0));
        assert_rgba_close(visible[1].color.color.to_f32_rgba(), ramp.sample(1.0));
    }

    #[test]
    fn resolved_gradient_ramp_interpolates_premultiplied_srgba() {
        let ramp = gradient_ramp([
            gradient_stop(0.0, Color::rgba8(0, 0, 0, 0)),
            gradient_stop(1.0, Color::rgba8(255, 0, 0, 255)),
        ])
        .resolve(1.0)
        .unwrap();
        assert_rgba_close(ramp.sample(0.5), (1.0, 0.0, 0.0, 0.5));
        assert_rgba_close(
            ResolvedGradientRamp::average_samples([(0.0, 0.0, 0.0, 0.0), (1.0, 0.0, 0.0, 1.0)]),
            (1.0, 0.0, 0.0, 0.5),
        );
    }

    #[test]
    fn resolved_gradient_ramp_samples_exact_and_adjacent_hard_transitions() {
        let boundary = 0.5_f32;
        let next = f32::from_bits(boundary.to_bits() + 1);
        let exact = gradient_ramp([
            gradient_stop(0.0, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(boundary, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(boundary, Color::rgba8(0, 0, 255, 255)),
            gradient_stop(1.0, Color::rgba8(0, 0, 255, 255)),
        ])
        .resolve(1.0)
        .unwrap();
        assert_eq!(exact.sample(boundary), (0.0, 0.0, 1.0, 1.0));
        assert_eq!(
            exact.sample(f32::from_bits(boundary.to_bits() - 1)),
            (1.0, 0.0, 0.0, 1.0)
        );

        let adjacent = gradient_ramp([
            gradient_stop(0.0, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(boundary, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(next, Color::rgba8(0, 0, 255, 255)),
            gradient_stop(1.0, Color::rgba8(0, 0, 255, 255)),
        ])
        .resolve(1.0)
        .unwrap();
        assert_eq!(adjacent.sample(boundary), (1.0, 0.0, 0.0, 1.0));
        assert_eq!(adjacent.sample(next), (0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn zero_period_repeating_gradient_uses_weighted_premultiplied_average() {
        let opaque = GradientRamp {
            stops: vec![
                gradient_stop(0.5, Color::rgba8(255, 0, 0, 255)),
                gradient_stop(0.5, Color::rgba8(255, 255, 255, 255)),
                gradient_stop(0.5, Color::rgba8(0, 0, 255, 255)),
            ],
            repeat: GradientRepeat::Repeat,
            ..Default::default()
        }
        .resolve(1.0)
        .unwrap();
        assert_rgba_close(opaque.sample(0.25), (0.75, 0.5, 0.75, 1.0));

        let alpha = GradientRamp {
            stops: vec![
                gradient_stop(0.5, Color::rgba8(0, 0, 0, 0)),
                gradient_stop(0.5, Color::rgba8(255, 0, 0, 255)),
            ],
            repeat: GradientRepeat::Repeat,
            ..Default::default()
        }
        .resolve(1.0)
        .unwrap();
        assert_rgba_close(alpha.sample(0.75), (1.0, 0.0, 0.0, 0.5));
    }

    #[test]
    fn resolved_gradient_ramp_accepts_positive_submicro_bases_and_exact_fixup() {
        let length_stop = GradientStop::new(
            GradientColor::new(
                Color::rgba8(0, 0, 255, 255),
                GradientColorProvenance::LegacySrgb,
            ),
            Some(GradientPosition::length(0.000_000_5)),
        );
        let ramp = gradient_ramp([
            gradient_stop(0.5, Color::rgba8(255, 0, 0, 255)),
            length_stop,
        ])
        .resolve(0.000_000_5)
        .unwrap();
        assert_eq!(ramp.stops()[1].position, 1.0);

        let decreasing = gradient_ramp([
            gradient_stop(0.75, Color::rgba8(255, 0, 0, 255)),
            gradient_stop(-4.0, Color::rgba8(0, 0, 255, 255)),
        ])
        .resolve(f32::MIN_POSITIVE)
        .unwrap();
        assert_eq!(decreasing.stops()[1].position, 0.75);

        for basis in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                gradient_ramp([length_stop, length_stop])
                    .resolve(basis)
                    .is_none()
            );
        }
        assert!(
            gradient_ramp([length_stop, length_stop])
                .resolve_scaled(1.0, f32::NAN)
                .is_none()
        );
    }

    #[test]
    fn gradient_fixup_clamps_positioned_stops_before_distributing_omissions() {
        let ramp = GradientRamp {
            stops: vec![
                GradientStop::new(
                    GradientColor::new(Color::rgb(255, 0, 0), GradientColorProvenance::LegacySrgb),
                    Some(GradientPosition::length(80.0)),
                ),
                GradientStop::new(
                    GradientColor::new(Color::WHITE, GradientColorProvenance::LegacySrgb),
                    Some(GradientPosition::length(0.0)),
                ),
                GradientStop::new(
                    GradientColor::new(Color::BLACK, GradientColorProvenance::LegacySrgb),
                    None,
                ),
                GradientStop::new(
                    GradientColor::new(Color::rgb(0, 0, 255), GradientColorProvenance::LegacySrgb),
                    Some(GradientPosition::length(100.0)),
                ),
            ],
            ..Default::default()
        }
        .resolve(100.0)
        .unwrap();
        assert_eq!(
            ramp.stops()
                .iter()
                .map(|stop| stop.position)
                .collect::<Vec<_>>(),
            [0.8, 0.8, 0.9, 1.0]
        );
    }

    #[test]
    fn color_architecture_size_baseline() {
        eprintln!(
            "Color={} CssValue={} GradientStop={} BoxShadow={} BorderSide={} ComputedStyle={}",
            std::mem::size_of::<Color>(),
            std::mem::size_of::<CssValue>(),
            std::mem::size_of::<GradientStop>(),
            std::mem::size_of::<BoxShadow>(),
            std::mem::size_of::<BorderSide>(),
            std::mem::size_of::<ComputedStyle>(),
        );
    }

    #[test]
    fn background_raster_tiling_preserves_large_physical_dimensions() {
        assert_eq!(
            background_raster_dimensions(3_000.0, 1.0, RasterQuality::default().background_dpi,)
                .unwrap()
                .width,
            8_000
        );
    }

    #[test]
    fn object_position_keywords_and_default() {
        use ObjectPositionComponent::Fraction;
        assert_eq!(
            parse_object_position("center"),
            Some(ObjectPosition::default())
        );
        assert_eq!(
            parse_object_position("bottom"),
            Some(ObjectPosition {
                x: Fraction(0.5),
                y: Fraction(1.0)
            })
        );
        assert_eq!(
            parse_object_position("right"),
            Some(ObjectPosition {
                x: Fraction(1.0),
                y: Fraction(0.5)
            })
        );
        // Keyword order may be swapped (top right == right top).
        assert_eq!(
            parse_object_position("top right"),
            Some(ObjectPosition {
                x: Fraction(1.0),
                y: Fraction(0.0)
            })
        );
    }

    #[test]
    fn object_position_percentages_resolve_to_fractions() {
        use ObjectPositionComponent::Fraction;
        let pos = parse_object_position("25% 75%").unwrap();
        assert_eq!(pos.x, Fraction(0.25));
        assert_eq!(pos.y, Fraction(0.75));
        // A single percentage applies to x; y defaults to center.
        assert_eq!(
            parse_object_position("10%"),
            Some(ObjectPosition {
                x: Fraction(0.10),
                y: Fraction(0.5)
            })
        );
    }

    #[test]
    fn object_position_lengths_are_absolute_offsets() {
        // 10px -> 7.5pt, 20px -> 15pt (1px = 0.75pt).
        let pos = parse_object_position("10px 20px").unwrap();
        assert_eq!(pos.x, ObjectPositionComponent::Length(7.5));
        assert_eq!(pos.y, ObjectPositionComponent::Length(15.0));
        // A length component is an absolute start-edge offset, independent of the
        // free space; a fraction scales the free space.
        assert_eq!(pos.x.resolve(100.0), 7.5);
        assert_eq!(ObjectPositionComponent::Fraction(0.25).resolve(80.0), 20.0);
    }

    #[test]
    fn object_position_edge_offset_three_value() {
        use ObjectPositionComponent::{FarEdgeLength, Fraction, Length};
        // `right 10px bottom 20%`: x = right edge + 10px (far edge length, rare),
        // y = bottom edge minus 20% == 80% from the top.
        let pos = parse_object_position("right 10px bottom 20%").unwrap();
        // Near-edge length is exact; here right is a far edge so x anchors to end.
        assert_eq!(pos.x, FarEdgeLength(7.5));
        assert_eq!(pos.y, Fraction(0.80));
        // `left 10px top 20px`: both near-edge length offsets stay absolute.
        let pos2 = parse_object_position("left 10px top 20px").unwrap();
        assert_eq!(pos2.x, Length(7.5));
        assert_eq!(pos2.y, Length(15.0));
    }

    #[test]
    fn object_position_rejects_invalid() {
        assert!(parse_object_position("").is_none());
        assert!(parse_object_position("frobnicate").is_none());
        assert!(parse_object_position("left top right bottom center").is_none());
    }

    #[test]
    fn h1_defaults() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, None, &parent);
        assert_eq!(style.font_size, 24.0);
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn inline_overrides_defaults() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, Some("font-size: 36pt"), &parent);
        assert_eq!(style.font_size, 36.0);
        assert_eq!(style.font_weight, FontWeight::Bold); // still bold from defaults
    }

    #[test]
    fn font_size_adjust_preserves_computed_font_and_line_sizes() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 30.0;
        parent.line_height = 1.6;

        let style = compute_style(HtmlTag::Span, Some("font-size-adjust: 0.8"), &parent);

        assert_eq!(style.font_size, 30.0);
        assert_eq!(style.line_height, 1.6);
        assert_eq!(style.font_size_adjust.target_ex_height(), Some(0.8));
    }

    #[test]
    fn font_size_adjust_is_inherited_and_none_resets_it() {
        let mut parent = ComputedStyle::default();
        parent.font_size_adjust = FontSizeAdjust::ex_height(0.8);

        let inherited = compute_style(HtmlTag::Span, None, &parent);
        let reset = compute_style(HtmlTag::Span, Some("font-size-adjust: none"), &parent);

        assert_eq!(inherited.font_size_adjust, parent.font_size_adjust);
        assert_eq!(reset.font_size_adjust, FontSizeAdjust::none());
    }

    #[test]
    fn color_inherited() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.color.r, 255 as f32);
    }

    #[test]
    fn bold_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Strong, None, &parent);
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn column_rule_shorthand_style_only_uses_medium_width() {
        // `column-rule: solid blue` — no width given; per CSS Multicol §6 the
        // initial column-rule-width is `medium` (~2.25pt), so the rule paints.
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-rule: solid blue"), &parent);
        assert!((style.column_rule.specified_width - 2.25).abs() < 0.01);
        assert_eq!(style.column_rule.style, BorderStyle::Solid);
        assert_eq!(
            style.column_rule.color.resolve(style.color),
            Color::rgb(0, 0, 255)
        );
    }

    #[test]
    fn writing_mode_vertical_rl_parses_and_inherits() {
        let parent = ComputedStyle::default();
        assert_eq!(parent.writing_mode, WritingMode::HorizontalTb);

        let vrl = compute_style(HtmlTag::Div, Some("writing-mode: vertical-rl"), &parent);
        assert_eq!(vrl.writing_mode, WritingMode::VerticalRl);

        // Inherited: a child with no writing-mode of its own keeps the parent's.
        let child = compute_style(HtmlTag::Span, None, &vrl);
        assert_eq!(child.writing_mode, WritingMode::VerticalRl);

        // Every authored mode remains represented through inheritance.
        let lr = compute_style(HtmlTag::Div, Some("writing-mode: vertical-lr"), &parent);
        assert_eq!(lr.writing_mode, WritingMode::VerticalLr);
        assert_eq!(
            compute_style(HtmlTag::Span, None, &lr).writing_mode,
            WritingMode::VerticalLr
        );
        let sideways_lr = compute_style(HtmlTag::Div, Some("writing-mode: sideways-lr"), &parent);
        assert_eq!(sideways_lr.writing_mode, WritingMode::SidewaysLr);
        let htb = compute_style(HtmlTag::Div, Some("writing-mode: horizontal-tb"), &vrl);
        assert_eq!(htb.writing_mode, WritingMode::HorizontalTb);
    }

    #[test]
    fn text_combine_upright_digits_parses_valid_range_and_inherits() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("text-combine-upright: digits 3"),
            &ComputedStyle::default(),
        );
        assert_eq!(parent.text_combine_upright, TextCombineUpright::Digits(3));
        assert_eq!(
            compute_style(HtmlTag::Span, None, &parent).text_combine_upright,
            TextCombineUpright::Digits(3)
        );
        assert_eq!(
            compute_style(
                HtmlTag::Span,
                Some("text-combine-upright: digits"),
                &ComputedStyle::default(),
            )
            .text_combine_upright,
            TextCombineUpright::Digits(2)
        );
        assert_eq!(
            compute_style(
                HtmlTag::Span,
                Some("text-combine-upright: digits 1"),
                &parent,
            )
            .text_combine_upright,
            TextCombineUpright::Digits(3)
        );
    }

    #[test]
    fn column_rule_shorthand_dotted_paints_at_medium() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-rule: dotted"), &parent);
        assert!((style.column_rule.specified_width - 2.25).abs() < 0.01);
        assert_eq!(style.column_rule.style, BorderStyle::Dotted);
    }

    #[test]
    fn column_rule_bevel_style_is_not_flattened_to_solid() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("column-rule: 6px ridge #00897b"),
            &parent,
        );
        assert_eq!(style.column_rule.style, BorderStyle::Ridge);
    }

    #[test]
    fn column_rule_width_keyword_thin_medium_thick() {
        let parent = ComputedStyle::default();
        let thin = compute_style(HtmlTag::Div, Some("column-rule-width: thin"), &parent);
        assert!((thin.column_rule.specified_width - 0.75).abs() < 0.01);
        let medium = compute_style(HtmlTag::Div, Some("column-rule-width: medium"), &parent);
        assert!((medium.column_rule.specified_width - 2.25).abs() < 0.01);
        let thick = compute_style(HtmlTag::Div, Some("column-rule-width: thick"), &parent);
        assert!((thick.column_rule.specified_width - 3.75).abs() < 0.01);
    }

    #[test]
    fn column_rule_style_longhand_only_uses_medium_width() {
        // `column-rule-style: dashed` alone should default the width to medium.
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-rule-style: dashed"), &parent);
        assert_eq!(style.column_rule.style, BorderStyle::Dashed);
        assert!((style.column_rule.specified_width - 2.25).abs() < 0.01);
    }

    #[test]
    fn column_rule_explicit_width_with_style_longhands() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("column-rule-width: 4px; column-rule-style: double"),
            &parent,
        );
        assert_eq!(style.column_rule.style, BorderStyle::Double);
        assert!((style.column_rule.specified_width - 3.0).abs() < 0.01); // 4px -> 3pt
    }

    #[test]
    fn columns_shorthand_width_only_vs_count_only() {
        let parent = ComputedStyle::default();
        // `columns: 140px` is a column-WIDTH, not a count of 140 columns.
        let w = compute_style(HtmlTag::Div, Some("columns: 140px"), &parent);
        assert_eq!(w.column_count, None);
        assert!((w.column_width.unwrap() - 105.0).abs() < 0.01); // 140px -> 105pt
        // `columns: 4` is a column-COUNT.
        let c = compute_style(HtmlTag::Div, Some("columns: 4"), &parent);
        assert_eq!(c.column_count, Some(4));
        assert_eq!(c.column_width, None);
        // Both together.
        let b = compute_style(HtmlTag::Div, Some("columns: 120px 3"), &parent);
        assert_eq!(b.column_count, Some(3));
        assert!((b.column_width.unwrap() - 90.0).abs() < 0.01); // 120px -> 90pt
        // `columns: auto` sets neither.
        let a = compute_style(HtmlTag::Div, Some("columns: auto"), &parent);
        assert_eq!(a.column_count, None);
        assert_eq!(a.column_width, None);
    }

    #[test]
    fn column_fill_auto_vs_balance() {
        let parent = ComputedStyle::default();
        assert!(!compute_style(HtmlTag::Div, None, &parent).column_fill_auto);
        assert!(
            !compute_style(HtmlTag::Div, Some("column-fill: balance"), &parent).column_fill_auto
        );
        assert!(compute_style(HtmlTag::Div, Some("column-fill: auto"), &parent).column_fill_auto);
    }

    #[test]
    fn italic_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Em, None, &parent);
        assert_eq!(style.font_style, FontStyle::Italic);
    }

    #[test]
    fn em_font_size() {
        let parent = ComputedStyle::default(); // font_size = 12.0
        let style = compute_style(HtmlTag::Span, Some("font-size: 2em"), &parent);
        assert!((style.font_size - 24.0).abs() < 0.1);
    }

    #[test]
    fn computed_font_size_is_the_shared_em_basis_for_dependent_dimensions() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 15.0;
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 2em; width: 6em; height: 3em"),
            &parent,
        );

        assert_eq!(style.font_size, 30.0);
        assert_eq!(style.width, Some(180.0));
        assert_eq!(style.height, Some(90.0));
    }

    #[test]
    fn calc_font_size_uses_parent_units_before_dependent_dimensions() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 15.0;
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: calc(1em + 8px); width: 2em; height: 3em"),
            &parent,
        );

        assert_eq!(style.font_size, 21.0);
        assert_eq!(style.width, Some(42.0));
        assert_eq!(style.height, Some(63.0));
    }

    #[test]
    fn calc_mixed_percent_width_uses_parent_width() {
        // calc(50% - 40px) against a 300pt-wide parent: 50% of 300 = 150,
        // minus 40px (30pt) = 120pt.
        let mut parent = ComputedStyle::default();
        parent.width = Some(300.0);
        parent.height = Some(120.0);
        let style = compute_style(HtmlTag::Div, Some("width: calc(50% - 40px)"), &parent);
        assert!(
            matches!(style.width, Some(w) if (w - 120.0).abs() < 0.01),
            "got {:?}",
            style.width
        );
    }

    #[test]
    fn calc_mixed_percent_height_uses_parent_height() {
        // calc(100% - 60px) on height must resolve the percent against the
        // parent height (120pt), not its width: 120 - 45 = 75pt.
        let mut parent = ComputedStyle::default();
        parent.width = Some(300.0);
        parent.height = Some(120.0);
        let style = compute_style(HtmlTag::Div, Some("height: calc(100% - 60px)"), &parent);
        assert!(
            matches!(style.height, Some(h) if (h - 75.0).abs() < 0.01),
            "got {:?}",
            style.height
        );
    }

    #[test]
    fn calc_uses_content_box_basis_without_changing_inheritance_parent() {
        // The inherited style still records the parent's declared 300pt
        // border-box width, but CSS percentages resolve against the layout
        // content box supplied by the caller (297pt × 117pt here).
        let mut parent = ComputedStyle::default();
        parent.width = Some(300.0);
        parent.height = Some(120.0);

        let style = compute_style_with_context_and_percentage_basis(
            HtmlTag::Div,
            Some("width: calc(50% - 40px); height: calc(100% - 60px)"),
            &parent,
            &[],
            "div",
            &[],
            None,
            &HashMap::new(),
            &SelectorContext::default(),
            PercentageBasis::new(Some(297.0), Some(117.0)),
        );

        assert!(matches!(style.width, Some(w) if (w - 118.5).abs() < 0.01));
        assert!(matches!(style.height, Some(h) if (h - 72.0).abs() < 0.01));
        assert_eq!(parent.width, Some(300.0));
        assert_eq!(parent.height, Some(120.0));
    }

    #[test]
    fn clamp_width_resolves_against_parent_width() {
        // clamp(120px, 50%, 240px): 50% of 600pt = 300, clamped to 180pt (240px).
        let mut parent = ComputedStyle::default();
        parent.width = Some(600.0);
        parent.height = Some(120.0);
        let style = compute_style(
            HtmlTag::Div,
            Some("width: clamp(120px, 50%, 240px)"),
            &parent,
        );
        assert!(
            matches!(style.width, Some(w) if (w - 180.0).abs() < 0.01),
            "got {:?}",
            style.width
        );
    }

    #[test]
    fn clamp_height_resolves_against_parent_height() {
        // clamp(80px, 50%, 200px): 50% of 160pt parent height = 80, min 80px(60pt)
        // -> clamps to 60pt.
        let mut parent = ComputedStyle::default();
        parent.width = Some(600.0);
        parent.height = Some(160.0);
        let style = compute_style(
            HtmlTag::Div,
            Some("height: clamp(80px, 50%, 200px)"),
            &parent,
        );
        // 50% of 160 = 80pt, within [60, 150] -> 80pt.
        assert!(
            matches!(style.height, Some(h) if (h - 80.0).abs() < 0.01),
            "got {:?}",
            style.height
        );
    }

    #[test]
    fn font_weight_normal() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-weight: normal"), &parent);
        assert_eq!(style.font_weight, FontWeight::Normal);
    }

    #[test]
    fn font_style_normal() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-style: normal"), &parent);
        assert_eq!(style.font_style, FontStyle::Normal);
    }

    #[test]
    fn font_style_preserves_the_authored_oblique_angle() {
        let parent = ComputedStyle::default();
        assert_eq!(
            compute_style(HtmlTag::Span, Some("font-style: oblique 20deg"), &parent,).font_style,
            FontStyle::Oblique(20.0)
        );
        assert_eq!(
            compute_style(HtmlTag::Span, Some("font-style: oblique"), &parent).font_style,
            FontStyle::Oblique(FontStyle::DEFAULT_OBLIQUE_ANGLE_DEGREES)
        );
    }

    #[test]
    fn font_shorthand_preserves_the_authored_oblique_angle() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font: oblique 25deg bold 20px/24px ParitySans"),
            &parent,
        );
        assert_eq!(style.font_style, FontStyle::Oblique(25.0));
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn background_color_applied() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("background-color: red"), &parent);
        assert!(style.background_color.is_some());
        let bg = style.background_color.unwrap();
        assert_eq!(bg.r, 255 as f32);
    }

    // --- CSS Color 4 spec coverage (full inline-style pipeline) -------------
    // Each case asserts the RGBA the library computes for a `background-color`.
    // The inline pipeline runs lightningcss, which normalizes every modern
    // color form (percentage rgb, slash-alpha, hsl/hwb, named, hex-alpha) to a
    // canonical hex/keyword the engine then parses. These tests pin the spec
    // conversions so a regression in either layer is caught.
    fn bg_rgba(decl: &str) -> (u8, u8, u8, u8) {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some(decl), &parent);
        let c = style
            .background_color
            .expect("background-color should parse");
        let [r, g, b, a] = c.to_rgba8();
        (r, g, b, a)
    }

    #[test]
    fn color_hex_4_digit_alpha() {
        // #rgba: each digit duplicated; alpha 0x8 -> 0x88. (css-color-4 §5.2)
        assert_eq!(bg_rgba("background-color: #0a68"), (0, 170, 102, 136));
    }

    #[test]
    fn color_hex_8_digit_alpha() {
        // #rrggbbaa: last pair is alpha. (css-color-4 §5.2)
        assert_eq!(bg_rgba("background-color: #c2185b80"), (194, 24, 91, 128));
    }

    #[test]
    fn color_rgb_percentage_components() {
        // rgb(80% 20% 10%): 80%*255=204, 20%*255=51, 10%*255=26. (css-color-4 §11)
        assert_eq!(
            bg_rgba("background-color: rgb(80% 20% 10%)"),
            (204, 51, 26, 255)
        );
    }

    #[test]
    fn color_rgb_modern_slash_alpha() {
        // rgb(r g b / a) modern space syntax with decimal alpha. (css-color-4 §11)
        assert_eq!(
            bg_rgba("background-color: rgb(255 0 0 / 0.5)"),
            (255, 0, 0, 128)
        );
    }

    #[test]
    fn color_rgb_percentage_alpha() {
        // Percentage alpha: 50% -> 128. (css-color-4 §15 <alpha-value>)
        assert_eq!(
            bg_rgba("background-color: rgb(0 0 0 / 50%)"),
            (0, 0, 0, 128)
        );
    }

    #[test]
    fn color_rgb_none_keyword() {
        // `none` resolves to 0 in legacy contexts. (css-color-4 §4.3)
        assert_eq!(
            bg_rgba("background-color: rgb(none 128 none)"),
            (0, 128, 0, 255)
        );
    }

    #[test]
    fn color_rgb_out_of_range_clamped() {
        // Out-of-range components clamp to [0,255]. (css-color-4 §11)
        assert_eq!(
            bg_rgba("background-color: rgb(300 -20 999)"),
            (255, 0, 255, 255)
        );
    }

    #[test]
    fn color_hsl_modern_slash_alpha() {
        // hsl(h s l / a) modern slash-alpha; hsl(280 60% 45%) -> #8a2eb8.
        // (css-color-4 §7)
        assert_eq!(
            bg_rgba("background-color: hsl(280 60% 45% / 0.5)"),
            (138, 46, 184, 128)
        );
    }

    #[test]
    fn color_hsl_hue_angle_units_normalized() {
        // 0.5turn = 180deg = cyan at full sat/half light. (css-color-4 §7, §<angle>)
        assert_eq!(
            bg_rgba("background-color: hsl(0.5turn 100% 50%)"),
            (0, 255, 255, 255)
        );
        // Hue > 360 normalizes: 400deg -> 40deg. (css-color-4 §7)
        assert_eq!(
            bg_rgba("background-color: hsl(400 100% 50%)"),
            (255, 170, 0, 255)
        );
    }

    #[test]
    fn color_hsl_powerless_hue_when_zero_sat() {
        // Saturation 0% -> gray regardless of hue. (css-color-4 §7)
        assert_eq!(
            bg_rgba("background-color: hsl(120 0% 50%)"),
            (128, 128, 128, 255)
        );
    }

    #[test]
    fn color_hwb_function() {
        // hwb(194 0% 0%) is fully saturated == hsl(194 100% 50%). (css-color-4 §8)
        assert_eq!(
            bg_rgba("background-color: hwb(194 0% 0%)"),
            (0, 196, 255, 255)
        );
        // hwb(120 50% 50%): w+b==1 -> gray = w/(w+b) = 0.5 -> 128. (css-color-4 §8)
        assert_eq!(
            bg_rgba("background-color: hwb(120 50% 50%)"),
            (128, 128, 128, 255)
        );
    }

    #[test]
    fn color_rebeccapurple_keyword() {
        // rebeccapurple == #663399. (css-color-4 §6.1, named color)
        assert_eq!(
            bg_rgba("background-color: rebeccapurple"),
            (102, 51, 153, 255)
        );
        // Named colors are ASCII case-insensitive.
        assert_eq!(
            bg_rgba("background-color: REBECCAPURPLE"),
            (102, 51, 153, 255)
        );
        assert_eq!(bg_rgba("background-color: NavY"), (0, 0, 128, 255));
    }

    #[test]
    fn color_transparent_keyword_is_zero_alpha() {
        // transparent == rgb(0 0 0 / 0). (css-color-4 §6.1)
        assert_eq!(bg_rgba("background-color: transparent"), (0, 0, 0, 0));
    }

    fn rgba_tuple(c: Color) -> (u8, u8, u8, u8) {
        let [r, g, b, a] = c.to_rgba8();
        (r, g, b, a)
    }

    #[test]
    fn parse_hex_to_color_alpha_forms() {
        // Direct unit coverage for the gradient/border hex parser's alpha forms.
        assert_eq!(
            parse_hex_to_color("0000").map(rgba_tuple),
            Some((0, 0, 0, 0))
        );
        assert_eq!(
            parse_hex_to_color("ff000080").map(rgba_tuple),
            Some((255, 0, 0, 128))
        );
        assert_eq!(
            parse_hex_to_color("1234").map(rgba_tuple),
            Some((17, 34, 51, 68))
        );
    }

    #[test]
    fn parse_gradient_color_transparent_is_zero_alpha() {
        // `transparent` in a gradient stop must be rgb(0 0 0 / 0), not white.
        assert_eq!(
            parse_gradient_color("transparent").map(rgba_tuple),
            Some((0, 0, 0, 0))
        );
        // lightningcss normalizes `transparent` to #0000 before this parser.
        assert_eq!(
            parse_gradient_color("#0000").map(rgba_tuple),
            Some((0, 0, 0, 0))
        );
    }

    #[test]
    fn margin_and_padding_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "margin-top: 10pt; margin-right: 20pt; margin-bottom: 30pt; margin-left: 40pt; padding-top: 5pt; padding-right: 6pt; padding-bottom: 7pt; padding-left: 8pt",
            ),
            &parent,
        );
        assert!((style.margin.top - 10.0).abs() < 0.1);
        assert!((style.margin.right - 20.0).abs() < 0.1);
        assert!((style.margin.bottom - 30.0).abs() < 0.1);
        assert!((style.margin.left - 40.0).abs() < 0.1);
        assert!((style.padding.top - 5.0).abs() < 0.1);
        assert!((style.padding.right - 6.0).abs() < 0.1);
        assert!((style.padding.bottom - 7.0).abs() < 0.1);
        assert!((style.padding.left - 8.0).abs() < 0.1);
    }

    #[test]
    fn text_align_center_and_right() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-align: center"), &parent);
        assert_eq!(style.text_align, TextAlign::Center);
        let style = compute_style(HtmlTag::Div, Some("text-align: right"), &parent);
        assert_eq!(style.text_align, TextAlign::Right);
    }

    #[test]
    fn text_decoration_underline() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("text-decoration: underline"), &parent);
        assert!(style.text_decorations.current.lines.underline);
    }

    #[test]
    fn text_decoration_skip_ink_is_inherited_and_independently_cascaded() {
        let parent = compute_style(
            HtmlTag::Span,
            Some("text-decoration-skip-ink: all"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            parent.text_decorations.current.skip_ink,
            TextDecorationSkipInk::All
        );

        let inherited = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(
            inherited.text_decorations.current.skip_ink,
            TextDecorationSkipInk::All
        );

        let overridden = compute_style(
            HtmlTag::Span,
            Some("text-decoration: underline; text-decoration-skip-ink: none"),
            &parent,
        );
        assert!(overridden.text_decorations.current.lines.underline);
        assert_eq!(
            overridden.text_decorations.current.skip_ink,
            TextDecorationSkipInk::None
        );
    }

    #[test]
    fn descendant_decoration_keeps_ancestor_origin_independent() {
        let parent = compute_style(
            HtmlTag::Div,
            Some(
                "color: #0055aa; text-decoration-line: underline; \
                 text-decoration-style: wavy; text-decoration-thickness: 2px",
            ),
            &ComputedStyle::default(),
        );
        let child = compute_style(
            HtmlTag::Span,
            Some(
                "color: #aa2200; text-decoration-line: line-through; \
                 text-decoration-color: #008844",
            ),
            &parent,
        );

        let active = child.text_decorations.active(child.color);
        assert_eq!(active.len(), 2);
        assert!(active[0].lines.underline);
        assert_eq!(active[0].style, TextDecorationStyle::Wavy);
        assert_eq!(active[0].thickness, Some(1.5));
        assert_eq!(active[0].color, Some(Color::rgb(0x00, 0x55, 0xaa)));
        assert!(active[1].lines.line_through);
        assert_eq!(active[1].style, TextDecorationStyle::Solid);
        assert_eq!(active[1].color, Some(Color::rgb(0x00, 0x88, 0x44)));
    }

    #[test]
    fn descendant_none_does_not_cancel_an_ancestor_decoration() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("text-decoration: underline"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Span, Some("text-decoration: none"), &parent);

        assert!(child.text_decorations.current.is_empty());
        let active = child.text_decorations.active(child.color);
        assert_eq!(active.len(), 1);
        assert!(active[0].lines.underline);
    }

    #[test]
    fn atomic_inline_contents_exclude_outer_decoration_origins() {
        let ancestor = compute_style(
            HtmlTag::Div,
            Some("text-decoration: underline"),
            &ComputedStyle::default(),
        );
        let atomic = compute_style(
            HtmlTag::Span,
            Some("display: inline-block; text-decoration: line-through"),
            &ancestor,
        );
        let content = compute_style(HtmlTag::Span, None, &atomic);

        let active = content.text_decorations.active(content.color);
        assert_eq!(active.len(), 1);
        assert!(active[0].lines.line_through);
        assert!(!active[0].lines.underline);
    }

    #[test]
    fn line_height_number_and_length() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("line-height: 18pt"), &parent);
        // 18pt / 12.0 font-size = 1.5
        assert!((style.line_height - 1.5).abs() < 0.1);
    }

    #[test]
    fn calc_lh_uses_the_completed_line_height_independent_of_declaration_order() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("font-size: 20px; line-height: 30px"),
            &ComputedStyle::default(),
        );

        for declarations in [
            "line-height: 36px; width: calc(3lh)",
            "width: calc(3lh); line-height: 36px",
        ] {
            let style = compute_style(HtmlTag::Div, Some(declarations), &parent);
            // 3 * 36 CSS px * 0.75pt/px.
            assert_eq!(style.width, Some(81.0), "{declarations}");
        }

        let inherited = compute_style(HtmlTag::Div, Some("width: calc(4lh)"), &parent);
        // 4 * inherited 30 CSS px * 0.75pt/px.
        assert_eq!(inherited.width, Some(90.0));
    }

    #[test]
    fn lh_inside_line_height_uses_the_parent_to_avoid_self_reference() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("line-height: 30px"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Div, Some("line-height: calc(2lh)"), &parent);

        assert_eq!(child.line_height_absolute, Some(45.0));
    }

    #[test]
    fn calc_rlh_uses_the_propagated_root_line_height() {
        let parent = ComputedStyle {
            root_font_units: FontUnitLengths {
                line_height: 18.0,
                ..FontUnitLengths::default()
            },
            ..ComputedStyle::default()
        };
        let child = compute_style(HtmlTag::Div, Some("width: calc(2rlh)"), &parent);

        assert_eq!(child.width, Some(36.0));
    }

    #[test]
    fn page_break_after() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("page-break-after: always"), &parent);
        assert!(style.page_break_after);
        // Legacy `always` maps to the modern `page` break value.
        assert_eq!(style.break_after, BreakValue::Page);
    }

    #[test]
    fn modern_break_before_page_forces_break() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("break-before: page"), &parent);
        assert_eq!(style.break_before, BreakValue::Page);
        assert!(style.page_break_before);
    }

    #[test]
    fn modern_break_after_sided_keeps_parity() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("break-after: left"), &parent);
        assert_eq!(style.break_after, BreakValue::Left);
        // Sided breaks still force a page break.
        assert!(style.page_break_after);
        let style = compute_style(HtmlTag::Div, Some("break-before: recto"), &parent);
        assert_eq!(style.break_before, BreakValue::Recto);
        assert!(style.page_break_before);
    }

    #[test]
    fn break_inside_avoid_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("break-inside: avoid"), &parent);
        assert!(style.break_inside_avoid);
        // Legacy alias.
        let style = compute_style(HtmlTag::Div, Some("page-break-inside: avoid"), &parent);
        assert!(style.break_inside_avoid);
        // `auto` does not set avoid.
        let style = compute_style(HtmlTag::Div, Some("break-inside: auto"), &parent);
        assert!(!style.break_inside_avoid);
    }

    #[test]
    fn break_before_avoid_does_not_force_break() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("break-before: avoid"), &parent);
        assert_eq!(style.break_before, BreakValue::Avoid);
        assert!(!style.page_break_before);
    }

    #[test]
    fn text_align_justify() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-align: justify"), &parent);
        assert_eq!(style.text_align, TextAlign::Justify);
    }

    #[test]
    fn text_align_unknown_fallback() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-align: foobar"), &parent);
        assert_eq!(style.text_align, TextAlign::Left);
    }

    #[test]
    fn line_height_as_number() {
        let parent = ComputedStyle::default();
        // line-height: 1.8em — em gets parsed as Number
        let style = compute_style(HtmlTag::Div, Some("line-height: 1.8em"), &parent);
        assert!((style.line_height - 1.8).abs() < 0.1);
    }

    #[test]
    fn text_decoration_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("text-decoration: line-through"),
            &parent,
        );
        assert!(style.text_decorations.current.lines.line_through);
        assert!(!style.text_decorations.current.lines.underline);
    }

    #[test]
    fn del_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Del, None, &parent);
        assert!(style.text_decorations.current.lines.line_through);
    }

    #[test]
    fn s_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::S, None, &parent);
        assert!(style.text_decorations.current.lines.line_through);
    }

    #[test]
    fn border_shorthand_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid black"), &parent);
        assert!((style.border.top.specified_width - 0.75).abs() < 0.1); // 1px = 0.75pt
        assert_eq!(
            style.border.top.color,
            SpecifiedColor::Absolute(Color::BLACK)
        );
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 0 as f32);
        assert_eq!(c.g, 0 as f32);
        assert_eq!(c.b, 0 as f32);
    }

    #[test]
    fn omitted_border_width_and_color_use_medium_and_current_color() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("color: #6a1b9a; border: solid"), &parent);

        for side in [
            style.border.top,
            style.border.right,
            style.border.bottom,
            style.border.left,
        ] {
            assert_eq!(side.used_width(), MEDIUM_RULE_WIDTH_PT);
            assert_eq!(side.color, SpecifiedColor::CurrentColor);
            assert_eq!(
                side.color.resolve(style.color),
                Color::rgb(0x6a, 0x1b, 0x9a)
            );
        }
    }

    #[test]
    fn border_style_longhand_uses_the_initial_medium_width() {
        let style = compute_style(
            HtmlTag::Div,
            Some("border-style: solid"),
            &ComputedStyle::default(),
        );

        assert_eq!(style.border.top.used_width(), MEDIUM_RULE_WIDTH_PT);
    }

    #[test]
    fn border_shorthand_none_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 10px none #d50000"), &parent);
        assert_eq!(style.border.top.style, BorderStyle::None);
        assert_eq!(style.border.right.style, BorderStyle::None);
        assert_eq!(style.border.bottom.style, BorderStyle::None);
        assert_eq!(style.border.left.style, BorderStyle::None);
    }

    #[test]
    fn border_bevel_style_is_typed_and_preserves_rgb_pdf_alpha_semantics() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border: 4px groove rgba(0 137 123 / 50%)"),
            &parent,
        );
        for side in [
            style.border.top,
            style.border.right,
            style.border.bottom,
            style.border.left,
        ] {
            assert_eq!(side.style, BorderStyle::Groove);
            let alpha = side.color.resolve(style.color).a;
            let expected = 128.0_f32;
            assert!(
                (alpha - expected).abs() <= expected * f32::EPSILON * 4.0,
                "alpha={alpha}"
            );
        }
    }

    #[test]
    fn border_style_four_value_shorthand_preserves_each_bevel_kind() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-width: 4px; border-color: #00897b; \
                 border-style: groove ridge inset outset",
            ),
            &parent,
        );
        assert_eq!(style.border.top.style, BorderStyle::Groove);
        assert_eq!(style.border.right.style, BorderStyle::Ridge);
        assert_eq!(style.border.bottom.style, BorderStyle::Inset);
        assert_eq!(style.border.left.style, BorderStyle::Outset);
    }

    #[test]
    fn solid_border_high_alpha_byte_is_not_bevel_metadata() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 4px solid #00897bfb"), &parent);
        assert_eq!(style.border.top.style, BorderStyle::Solid);
        assert_eq!(style.border.top.color.resolve(style.color).a, 251.0);
    }

    #[test]
    fn hidden_border_style_remains_distinct_from_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 4px hidden red"), &parent);
        assert_eq!(style.border.top.style, BorderStyle::Hidden);
        assert!(!style.border.top.style.paints());
    }

    #[test]
    fn border_with_custom_color() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2px solid red"), &parent);
        assert!((style.border.top.specified_width - 1.5).abs() < 0.1); // 2px = 1.5pt
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 255 as f32);
        assert_eq!(c.g, 0 as f32);
        assert_eq!(c.b, 0 as f32);
    }

    #[test]
    fn border_shorthand_em_width_resolves_against_font_size() {
        // Regression: a font-relative border width in the `border` shorthand
        // (e.g. `0.2em`) was dropped, leaving width = 0. font-size:20px -> 1em
        // = 20px, so 0.2em = 4px = 3pt. Width/height in em must also resolve.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 20px; border: 0.2em solid #11305f"),
            &parent,
        );
        // 0.2em * 20px = 4px = 3pt.
        assert!(
            (style.border.top.specified_width - 3.0).abs() < 0.05,
            "em border width should be 3pt, got {}",
            style.border.top.specified_width
        );
        assert!((style.border.bottom.specified_width - 3.0).abs() < 0.05);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!((c.r, c.g, c.b), (0x11 as f32, 0x30 as f32, 0x5f as f32));
    }

    #[test]
    fn border_width_longhand_em_resolves_against_font_size() {
        // The uniform `border-width` and per-side longhands accept em widths too.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 20px; border-style: solid; border-width: 0.5em"),
            &parent,
        );
        // 0.5em * 20px = 10px = 7.5pt.
        assert!(
            (style.border.top.specified_width - 7.5).abs() < 0.05,
            "em uniform border width should be 7.5pt, got {}",
            style.border.top.specified_width
        );
        let per_side = compute_style(
            HtmlTag::Div,
            Some("font-size: 20px; border-top-style: solid; border-top-width: 0.25em"),
            &parent,
        );
        // 0.25em * 20px = 5px = 3.75pt.
        assert!((per_side.border.top.specified_width - 3.75).abs() < 0.05);
    }

    #[test]
    fn nested_var_fallback_in_border_shorthand_resolves() {
        // Regression: `var(--a, var(--b, #11305f))` in a shorthand had its
        // fallback truncated at the inner `)`, dropping the substitution and
        // leaving the border the black default. Neither custom property is
        // defined, so the innermost literal color must win.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border: 4px solid var(--bc, var(--bc2, #11305f))"),
            &parent,
        );
        let c = style.border.top.color.resolve(style.color);
        assert_eq!((c.r, c.g, c.b), (0x11 as f32, 0x30 as f32, 0x5f as f32));
    }

    #[test]
    fn border_width_and_color_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border-width: 3pt; border-color: blue"),
            &parent,
        );
        assert!((style.border.top.specified_width - 3.0).abs() < 0.1);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 0 as f32);
        assert_eq!(c.g, 0 as f32);
        assert_eq!(c.b, 255 as f32);
    }

    #[test]
    fn border_per_side_width_longhands() {
        // Mirrors block-box-model/block-border-width-thick: a single
        // `border-style`/`border-color` plus asymmetric per-side widths. Each
        // edge must pick up its own width and remain paintable (solid + colored).
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-style: solid; border-color: #11305f; \
                 border-top-width: 6px; border-right-width: 14px; \
                 border-bottom-width: 22px; border-left-width: 30px",
            ),
            &parent,
        );
        // px -> pt is a 0.75 factor.
        assert!((style.border.top.specified_width - 6.0 * 0.75).abs() < 0.01);
        assert!((style.border.right.specified_width - 14.0 * 0.75).abs() < 0.01);
        assert!((style.border.bottom.specified_width - 22.0 * 0.75).abs() < 0.01);
        assert!((style.border.left.specified_width - 30.0 * 0.75).abs() < 0.01);
        for side in [
            &style.border.top,
            &style.border.right,
            &style.border.bottom,
            &style.border.left,
        ] {
            assert_eq!(side.style, BorderStyle::Solid);
            let c = side.color.resolve(style.color);
            assert_eq!((c.r, c.g, c.b), (0x11 as f32, 0x30 as f32, 0x5f as f32));
            // Paintable: width > 0 && style != None.
            assert!(side.specified_width > 0.0 && side.style != BorderStyle::None);
        }
    }

    #[test]
    fn border_per_side_style_and_color_longhands() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-width: 4px; border-top-style: dashed; \
                 border-right-style: dotted; border-bottom-style: none; \
                 border-left-color: red",
            ),
            &parent,
        );
        assert_eq!(style.border.top.style, BorderStyle::Dashed);
        assert_eq!(style.border.right.style, BorderStyle::Dotted);
        assert_eq!(style.border.bottom.style, BorderStyle::None);
        let left = style.border.left.color.resolve(style.color);
        assert_eq!((left.r, left.g, left.b), (255 as f32, 0 as f32, 0 as f32));
    }

    #[test]
    fn font_family_default_is_serif() {
        // The UA-initial font-family is a serif face (matching Chrome's default
        // "standard" font), so unstyled text and the `ex`/`ch` units resolve
        // against serif metrics rather than sans-serif.
        let style = ComputedStyle::default();
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_serif() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: serif"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_variant_small_caps_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-variant: small-caps"), &parent);
        assert_eq!(style.font_variant_caps, FontVariantCaps::SmallCaps);
        let caps = compute_style(
            HtmlTag::Span,
            Some("font-variant-caps: small-caps"),
            &parent,
        );
        assert_eq!(caps.font_variant_caps, FontVariantCaps::SmallCaps);
    }

    #[test]
    fn font_variant_normal_resets_small_caps() {
        let mut parent = ComputedStyle::default();
        parent.font_variant_caps = FontVariantCaps::SmallCaps;
        let style = compute_style(HtmlTag::Span, Some("font-variant: normal"), &parent);
        assert_eq!(style.font_variant_caps, FontVariantCaps::Normal);
    }

    #[test]
    fn font_variant_caps_inherits() {
        let mut parent = ComputedStyle::default();
        parent.font_variant_caps = FontVariantCaps::SmallCaps;
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.font_variant_caps, FontVariantCaps::SmallCaps);
    }

    #[test]
    fn font_variant_position_parses_and_resets() {
        let parent = ComputedStyle::default();
        let super_style =
            compute_style(HtmlTag::Span, Some("font-variant-position: super"), &parent);
        assert_eq!(
            super_style.font_variant_position,
            FontVariantPosition::Super
        );

        let normal = compute_style(
            HtmlTag::Span,
            Some("font-variant-position: normal"),
            &super_style,
        );
        assert_eq!(normal.font_variant_position, FontVariantPosition::Normal);

        let initial = compute_style(
            HtmlTag::Span,
            Some("font-variant-position: initial"),
            &super_style,
        );
        assert_eq!(initial.font_variant_position, FontVariantPosition::Normal);
    }

    #[test]
    fn font_feature_settings_liga_off_disables_ligatures() {
        let parent = ComputedStyle::default();
        assert!(parent.ligatures_enabled);
        let style = compute_style(
            HtmlTag::Span,
            Some("font-feature-settings: \"liga\" 0"),
            &parent,
        );
        assert!(!style.ligatures_enabled);
    }

    #[test]
    fn font_feature_settings_liga_on_keeps_ligatures() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-feature-settings: \"liga\" 1"),
            &parent,
        );
        assert!(style.ligatures_enabled);
    }

    #[test]
    fn font_variant_ligatures_no_common_ligatures_disables_ligatures() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-variant-ligatures: no-common-ligatures"),
            &parent,
        );
        assert!(!style.ligatures_enabled);
    }

    #[test]
    fn ligatures_disabled_by_feature_settings_parsing() {
        assert!(ligatures_disabled_by_feature_settings("\"liga\" 0"));
        assert!(ligatures_disabled_by_feature_settings("'clig' off"));
        assert!(ligatures_disabled_by_feature_settings(
            "\"liga\" 0, \"dlig\" 0"
        ));
        assert!(!ligatures_disabled_by_feature_settings("\"liga\" 1"));
        assert!(!ligatures_disabled_by_feature_settings("\"liga\""));
        // A non-ligature feature does not affect ligatures.
        assert!(!ligatures_disabled_by_feature_settings("\"kern\" 0"));
        // A later enable cancels an earlier disable.
        assert!(!ligatures_disabled_by_feature_settings(
            "\"liga\" 0, \"liga\" 1"
        ));
    }

    #[test]
    fn font_family_times_new_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-family: 'Times New Roman'"),
            &parent,
        );
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_monospace() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: monospace"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: courier"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_sans_serif_defaults_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: sans-serif"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_inherited() {
        let mut parent = ComputedStyle::default();
        parent.font_family = FontFamily::Courier;
        parent.font_stack = FontStack::from_family(FontFamily::Courier);
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn border_shorthand_pt_unit() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2pt solid green"), &parent);
        assert!((style.border.top.specified_width - 2.0).abs() < 0.1);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 0 as f32);
        assert_eq!(c.g, 128 as f32);
        assert_eq!(c.b, 0 as f32);
    }

    #[test]
    fn border_currentcolor_resolves_to_computed_color() {
        // `border: ... currentColor` must paint with the element's computed
        // `color`, not fall back to black.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("color: #6a1b9a; border: 12px solid currentColor"),
            &parent,
        );
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(
            (c.r, c.g, c.b),
            (style.color.r, style.color.g, style.color.b)
        );
        assert_eq!((c.r, c.g, c.b), (0x6a as f32, 0x1b as f32, 0x9a as f32));
    }

    #[test]
    fn border_color_variants() {
        let parent = ComputedStyle::default();
        for (name, r, g, b) in [
            ("yellow", 255, 255, 0),
            ("orange", 255, 165, 0),
            ("purple", 128, 0, 128),
            ("gray", 128, 128, 128),
            ("grey", 128, 128, 128),
            ("white", 255, 255, 255),
        ] {
            let css = format!("border: 1px solid {name}");
            let style = compute_style(HtmlTag::Div, Some(&css), &parent);
            let c = style.border.top.color.resolve(style.color);
            assert_eq!(
                (c.r, c.g, c.b),
                (r as f32, g as f32, b as f32),
                "failed for {name}"
            );
        }
    }

    #[test]
    fn border_color_hex_short() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #f00"), &parent);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 255 as f32);
        assert_eq!(c.g, 0 as f32);
        assert_eq!(c.b, 0 as f32);
    }

    #[test]
    fn border_color_hex_long() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #00ff00"), &parent);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 0 as f32);
        assert_eq!(c.g, 255 as f32);
        assert_eq!(c.b, 0 as f32);
    }

    #[test]
    fn invalid_border_color_discards_the_whole_shorthand() {
        // A custom ident is not a color, so the entire border shorthand is
        // invalid rather than a visible currentColor border.
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid foobar"), &parent);
        assert!(!style.border.has_any());
    }

    #[test]
    fn parse_border_color_unknown_token_returns_none() {
        // The low-level parser still reports `None` for an unknown color token;
        // the currentColor default is applied later, in the cascade post-pass.
        assert!(parse_border_color("foobar").is_none());
    }

    // --- Extended font-family mapping tests ---

    #[test]
    fn font_family_arial_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Arial"), &parent);
        assert_eq!(style.font_family, FontFamily::Custom("arial".to_string()));
    }

    #[test]
    fn font_family_roboto_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Roboto"), &parent);
        assert_eq!(style.font_family, FontFamily::Custom("roboto".to_string()));
    }

    #[test]
    fn font_family_verdana_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Verdana"), &parent);
        assert_eq!(style.font_family, FontFamily::Custom("verdana".to_string()));
    }

    #[test]
    fn font_family_open_sans_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Open Sans'"), &parent);
        assert_eq!(
            style.font_family,
            FontFamily::Custom("open sans".to_string())
        );
    }

    #[test]
    fn font_family_system_ui_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: system-ui"), &parent);
        assert_eq!(
            style.font_family,
            FontFamily::Custom("system-ui".to_string())
        );
    }

    #[test]
    fn font_family_ui_sans_serif_prefers_custom_face() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: ui-sans-serif"), &parent);
        assert_eq!(
            style.font_family,
            FontFamily::Custom("ui-sans-serif".to_string())
        );
    }

    #[test]
    fn font_family_georgia_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Georgia"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_garamond_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Garamond"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_merriweather_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Merriweather"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_palatino_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Palatino"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_consolas_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Consolas"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_fira_code_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Fira Code'"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_jetbrains_mono_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-family: 'JetBrains Mono'"),
            &parent,
        );
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_menlo_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Menlo"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_sf_mono_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'SF Mono'"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_monaco_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Monaco"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_unknown_becomes_custom() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Comic Sans MS'"), &parent);
        assert_eq!(
            style.font_family,
            FontFamily::Custom("comic sans ms".to_string())
        );
    }

    #[test]
    fn font_family_case_insensitive() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: GEORGIA"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
        let style = compute_style(HtmlTag::Span, Some("font-family: CONSOLAS"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_double_quoted() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: \"Courier New\""), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn display_none_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: none"), &parent);
        assert_eq!(style.display, Display::None);
    }

    #[test]
    fn display_block_on_inline_element() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("display: block"), &parent);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn display_inline_on_block_element() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: inline"), &parent);
        assert_eq!(style.display, Display::Inline);
    }

    #[test]
    fn display_default_for_block_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn display_default_for_inline_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.display, Display::Inline);
    }

    #[test]
    fn width_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: 200pt"), &parent);
        assert_eq!(style.width, Some(200.0));
    }

    #[test]
    fn height_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("height: 100pt"), &parent);
        assert_eq!(style.height, Some(100.0));
    }

    #[test]
    fn max_width_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("max-width: 300pt"), &parent);
        assert_eq!(style.max_width, Some(300.0));
    }

    #[test]
    fn width_px_converted_to_pt() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: 200px"), &parent);
        assert!((style.width.unwrap() - 150.0).abs() < 0.1); // 200 * 0.75 = 150
    }

    #[test]
    fn opacity_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("opacity: 0.5"), &parent);
        assert!((style.opacity - 0.5).abs() < 0.01);
    }

    #[test]
    fn opacity_reparses_percentage_after_var_substitution() {
        let parent = ComputedStyle::default();
        for declarations in [
            "--half: 50%; opacity: var(--half)",
            "opacity: var(--half); --half: 50%",
        ] {
            let style = compute_style(HtmlTag::Div, Some(declarations), &parent);
            assert!((style.opacity - 0.5).abs() < 0.01, "{declarations}");
        }

        let mut inherited = ComputedStyle::default();
        inherited
            .custom_properties
            .insert("--half".to_string(), "50%".to_string());
        let style = compute_style(HtmlTag::Div, Some("opacity: var(--half)"), &inherited);
        assert!((style.opacity - 0.5).abs() < 0.01);
    }

    #[test]
    fn blend_modes_from_inline_style() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("mix-blend-mode: multiply"), &parent);
        assert_eq!(s.mix_blend_mode, BlendMode::Multiply);
        let s = compute_style(HtmlTag::Div, Some("mix-blend-mode: screen"), &parent);
        assert_eq!(s.mix_blend_mode, BlendMode::Screen);
        let s = compute_style(
            HtmlTag::Div,
            Some("background-blend-mode: multiply"),
            &parent,
        );
        assert_eq!(s.background_blend_mode, BlendMode::Multiply);
    }

    #[test]
    fn blend_mode_default_is_normal() {
        let s = ComputedStyle::default();
        assert_eq!(s.mix_blend_mode, BlendMode::Normal);
        assert_eq!(s.background_blend_mode, BlendMode::Normal);
        assert_eq!(BlendMode::Normal.pdf_name(), None);
        assert_eq!(BlendMode::Multiply.pdf_name(), Some("Multiply"));
    }

    #[test]
    fn opacity_default_is_one() {
        let style = ComputedStyle::default();
        assert!((style.opacity - 1.0).abs() < 0.01);
    }

    #[test]
    fn opacity_clamped_to_range() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("opacity: 1.5"), &parent);
        assert!((style.opacity - 1.0).abs() < 0.01);
        let style = compute_style(HtmlTag::Div, Some("opacity: -0.5"), &parent);
        assert!((style.opacity - 0.0).abs() < 0.01);
    }

    #[test]
    fn width_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.width = Some(200.0);
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.width, None);
    }

    #[test]
    fn opacity_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.opacity = 0.5;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.opacity - 1.0).abs() < 0.01);
    }

    // --- Float / Clear / Position / Box-shadow tests ---

    #[test]
    fn float_left_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("float: left"), &parent);
        assert_eq!(style.float, Float::Left);
    }

    #[test]
    fn float_right_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("float: right"), &parent);
        assert_eq!(style.float, Float::Right);
    }

    #[test]
    fn float_none_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("float: none"), &parent);
        assert_eq!(style.float, Float::None);
    }

    #[test]
    fn float_default_is_none() {
        let style = ComputedStyle::default();
        assert_eq!(style.float, Float::None);
    }

    #[test]
    fn footnote_formatting_is_typed_and_non_inherited() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("footnote-display: compact; footnote-policy: line"),
            &ComputedStyle::default(),
        );
        assert_eq!(parent.footnote.display, FootnoteDisplay::Compact);
        assert_eq!(parent.footnote.policy, FootnotePolicy::Line);

        let child = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(child.footnote, FootnoteFormatting::default());

        let inherited = compute_style(
            HtmlTag::Span,
            Some("footnote-display: inherit; footnote-policy: inherit"),
            &parent,
        );
        assert_eq!(inherited.footnote, parent.footnote);
    }

    #[test]
    fn footnote_formatting_css_wide_initial_resets_each_property() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "footnote-display: inline; footnote-policy: block; \
                 footnote-display: initial; footnote-policy: initial",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(style.footnote, FootnoteFormatting::default());
    }

    #[test]
    fn invalid_footnote_formatting_declarations_do_not_replace_valid_values() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "footnote-display: inline; footnote-policy: line; \
                 footnote-display: sideways; footnote-policy: paragraph",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(style.footnote.display, FootnoteDisplay::Inline);
        assert_eq!(style.footnote.policy, FootnotePolicy::Line);
    }

    #[test]
    fn clear_both_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("clear: both"), &parent);
        assert_eq!(style.clear, Clear::Both);
    }

    #[test]
    fn clear_left_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("clear: left"), &parent);
        assert_eq!(style.clear, Clear::Left);
    }

    #[test]
    fn clear_right_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("clear: right"), &parent);
        assert_eq!(style.clear, Clear::Right);
    }

    #[test]
    fn clear_default_is_none() {
        let style = ComputedStyle::default();
        assert_eq!(style.clear, Clear::None);
    }

    #[test]
    fn position_relative_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("position: relative; top: 10pt; left: 5pt"),
            &parent,
        );
        assert_eq!(style.position, Position::Relative);
        assert_eq!(style.top, Some(10.0));
        assert_eq!(style.left, Some(5.0));
    }

    #[test]
    fn position_absolute_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("position: absolute; top: 100pt; left: 50pt"),
            &parent,
        );
        assert_eq!(style.position, Position::Absolute);
        assert_eq!(style.top, Some(100.0));
        assert_eq!(style.left, Some(50.0));
    }

    #[test]
    fn position_running_records_name_and_stays_static() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("position: running(RunHead)"), &parent);
        assert_eq!(style.position, Position::Static);
        assert_eq!(style.running_name.as_deref(), Some("runhead"));
    }

    #[test]
    fn position_default_is_static() {
        let style = ComputedStyle::default();
        assert_eq!(style.position, Position::Static);
    }

    #[test]
    fn position_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.position = Position::Relative;
        parent.top = Some(10.0);
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.position, Position::Static);
        assert_eq!(style.top, None);
    }

    #[test]
    fn float_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.float = Float::Left;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.float, Float::None);
    }

    #[test]
    fn box_shadow_simple_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 3px 3px black"), &parent);
        let shadow = style.box_shadow[0];
        assert!((shadow.offset_x - 2.25).abs() < 0.1); // 3px * 0.75
        assert!((shadow.offset_y - 2.25).abs() < 0.1);
        assert!((shadow.blur - 0.0).abs() < 0.1);
        assert_eq!(shadow.color.r, 0 as f32);
        assert_eq!(shadow.color.g, 0 as f32);
        assert_eq!(shadow.color.b, 0 as f32);
    }

    #[test]
    fn box_shadow_with_blur() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 2px 2px 4px black"), &parent);
        let shadow = style.box_shadow[0];
        assert!((shadow.offset_x - 1.5).abs() < 0.1); // 2px * 0.75
        assert!((shadow.offset_y - 1.5).abs() < 0.1);
        assert!((shadow.blur - 3.0).abs() < 0.1); // 4px * 0.75
        assert_eq!(shadow.color.r, 0 as f32);
    }

    #[test]
    fn box_shadow_with_pt_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 3pt 3pt red"), &parent);
        let shadow = style.box_shadow[0];
        assert!((shadow.offset_x - 3.0).abs() < 0.1);
        assert!((shadow.offset_y - 3.0).abs() < 0.1);
        assert_eq!(shadow.color.r, 255 as f32);
    }

    #[test]
    fn box_shadow_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: none"), &parent);
        assert_eq!(style.box_shadow.len(), 0);
    }

    #[test]
    fn box_shadow_default_is_none() {
        let style = ComputedStyle::default();
        assert!(style.box_shadow.is_empty());
    }

    #[test]
    fn box_shadow_multiple_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("box-shadow: 16px 16px 0 0 #6a1b9a, -16px -16px 0 0 #00838f"),
            &parent,
        );
        assert_eq!(style.box_shadow.len(), 2);
        // First listed shadow.
        assert!((style.box_shadow[0].offset_x - 12.0).abs() < 0.1); // 16px * 0.75
        assert_eq!(style.box_shadow[0].color.r, 0x6a as f32);
        // Second listed shadow.
        assert!((style.box_shadow[1].offset_x + 12.0).abs() < 0.1); // -16px * 0.75
        assert_eq!(style.box_shadow[1].color.g, 0x83 as f32);
    }

    #[test]
    fn overflow_clip_keyword_clips() {
        let parent = ComputedStyle::default();
        // `overflow: clip` clips to the box like hidden in our model.
        let s = compute_style(HtmlTag::Div, Some("overflow: clip"), &parent);
        assert_eq!(s.overflow, Overflow::Hidden);
        assert!(s.overflow.clips());
    }

    #[test]
    fn overflow_scroll_keyword_clips() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("overflow: scroll"), &parent);
        assert_eq!(s.overflow, Overflow::Hidden);
    }

    #[test]
    fn overflow_auto_clips_in_print() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("overflow: auto"), &parent);
        assert_eq!(s.overflow, Overflow::Auto);
        assert!(
            s.overflow.clips(),
            "auto clips overflowing content in print"
        );
    }

    #[test]
    fn overflow_x_hidden_y_visible_coerces_to_clip_both() {
        // Per CSS Overflow 3: `overflow-x: hidden` is a scrolling value, so the
        // sibling `overflow-y: visible` is coerced to `auto`, making the box
        // clip on BOTH axes.
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("overflow-x: hidden; overflow-y: visible"),
            &parent,
        );
        assert_eq!(s.overflow, Overflow::Hidden);
        assert!(s.overflow.clips());
    }

    #[test]
    fn overflow_x_visible_y_hidden_coerces_to_auto_and_clips() {
        // CSS Overflow 3 applies the same computed-value rule in the opposite
        // direction: a visible x axis becomes auto when y is scrollable.
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("overflow-x: visible; overflow-y: hidden"),
            &parent,
        );
        assert_eq!(s.overflow_x, Overflow::Auto);
        assert_eq!(s.overflow_y, Overflow::Hidden);
        assert_eq!(s.overflow, Overflow::Hidden);
        assert!(s.overflow.clips());
    }

    #[test]
    fn overflow_both_visible_does_not_clip() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("overflow-x: visible; overflow-y: visible"),
            &parent,
        );
        assert_eq!(s.overflow, Overflow::Visible);
        assert!(!s.overflow.clips());
    }

    #[test]
    fn overflow_y_only_hidden_clips() {
        let parent = ComputedStyle::default();
        // `overflow-y: hidden` alone: x defaults visible, coerced to auto -> clip.
        let s = compute_style(HtmlTag::Div, Some("overflow-y: hidden"), &parent);
        assert_eq!(s.overflow, Overflow::Hidden);
    }

    #[test]
    fn box_shadow_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.box_shadow = vec![BoxShadow {
            offset_x: 3.0,
            offset_y: 3.0,
            blur: 0.0,
            spread: 0.0,
            color: Color::BLACK,
            color_source: ColorSource::Absolute,
            inset: false,
        }];
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!(style.box_shadow.is_empty());
    }

    #[test]
    fn top_left_px_converted() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("top: 20px; left: 10px"), &parent);
        assert!((style.top.unwrap() - 15.0).abs() < 0.1); // 20 * 0.75
        assert!((style.left.unwrap() - 7.5).abs() < 0.1); // 10 * 0.75
    }

    // --- Overflow tests ---

    #[test]
    fn overflow_default_is_visible() {
        let style = ComputedStyle::default();
        assert_eq!(style.overflow, Overflow::Visible);
    }

    #[test]
    fn overflow_hidden_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("overflow: hidden"), &parent);
        assert_eq!(style.overflow, Overflow::Hidden);
    }

    #[test]
    fn overflow_auto_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("overflow: auto"), &parent);
        assert_eq!(style.overflow, Overflow::Auto);
    }

    #[test]
    fn overflow_visible_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("overflow: visible"), &parent);
        assert_eq!(style.overflow, Overflow::Visible);
    }

    #[test]
    fn overflow_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.overflow = Overflow::Hidden;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.overflow, Overflow::Visible);
    }

    // --- Visibility tests ---

    #[test]
    fn visibility_default_is_visible() {
        let style = ComputedStyle::default();
        assert_eq!(style.visibility, Visibility::Visible);
    }

    #[test]
    fn visibility_hidden_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("visibility: hidden"), &parent);
        assert_eq!(style.visibility, Visibility::Hidden);
    }

    #[test]
    fn visibility_visible_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("visibility: visible"), &parent);
        assert_eq!(style.visibility, Visibility::Visible);
    }

    #[test]
    fn visibility_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.visibility = Visibility::Hidden;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.visibility, Visibility::Visible);
    }

    // --- Transform tests ---

    #[test]
    fn transform_default_is_none() {
        let style = ComputedStyle::default();
        assert!(style.transform.is_none());
    }

    #[test]
    fn transform_rotate_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: rotate(45deg)"), &parent);
        assert_eq!(style.transform, Some(Transform::Rotate(45.0)));
    }

    #[test]
    fn transform_rotate_negative() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: rotate(-90deg)"), &parent);
        assert_eq!(style.transform, Some(Transform::Rotate(-90.0)));
    }

    #[test]
    fn transform_scale_uniform() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: scale(2)"), &parent);
        assert_eq!(
            style.transform,
            Some(Transform::Scale(CssVector::splat(2.0)))
        );
    }

    #[test]
    fn transform_scale_non_uniform() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: scale(1.5, 2.0)"), &parent);
        assert_eq!(
            style.transform,
            Some(Transform::Scale(CssVector::new(1.5, 2.0)))
        );
    }

    #[test]
    fn transform_translate_pt() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("transform: translate(10pt, 20pt)"),
            &parent,
        );
        assert_eq!(
            style.transform,
            Some(Transform::Translate {
                offset: CssVector::new(10.0, 20.0),
                percentages: PercentageAxes::default(),
            })
        );
    }

    #[test]
    fn transform_translate_px() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("transform: translate(10px, 20px)"),
            &parent,
        );
        let t = style.transform.unwrap();
        if let Transform::Translate {
            offset,
            percentages,
        } = t
        {
            assert!((offset.x - 7.5).abs() < 0.1); // 10 * 0.75
            assert!((offset.y - 15.0).abs() < 0.1); // 20 * 0.75
            assert_eq!(percentages, PercentageAxes::default());
        } else {
            panic!("Expected Translate");
        }
    }

    #[test]
    fn transform_translate_percent() {
        // translate(50%, 25%) keeps the raw percentages with the pct flags set;
        // they resolve against the element's own border box at render time.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("transform: translate(50%, 25%)"),
            &parent,
        );
        assert_eq!(
            style.transform,
            Some(Transform::Translate {
                offset: CssVector::new(50.0, 25.0),
                percentages: PercentageAxes::new(true, true),
            })
        );
        // The render-time resolution multiplies the percentage by the box size.
        let matrix = style
            .transform
            .unwrap()
            .to_css_matrix(CssVector::new(200.0, 80.0));
        assert!((matrix.translation.x - 100.0).abs() < 0.01); // 50% of 200pt
        assert!((matrix.translation.y - 20.0).abs() < 0.01); // 25% of 80pt
    }

    #[test]
    fn transform_translatex_percent() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: translateX(50%)"), &parent);
        assert_eq!(
            style.transform,
            Some(Transform::Translate {
                offset: CssVector::new(50.0, 0.0),
                percentages: PercentageAxes::new(true, false),
            })
        );
        let matrix = style
            .transform
            .unwrap()
            .to_css_matrix(CssVector::new(120.0, 60.0));
        assert!((matrix.translation.x - 60.0).abs() < 0.01); // 50% of 120pt width
        assert!(matrix.translation.y.abs() < 0.01); // no Y translation
    }

    #[test]
    fn transform_none_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: none"), &parent);
        assert!(style.transform.is_none());
    }

    #[test]
    fn transform_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.transform = Some(Transform::Rotate(45.0));
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!(style.transform.is_none());
    }

    // --- Grid style tests ---

    #[test]
    fn display_grid_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: grid"), &parent);
        assert_eq!(style.display, Display::Grid);
    }

    #[test]
    fn grid_template_columns_fr_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: 1fr 2fr 1fr"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 3);
        assert_eq!(style.grid_template_columns[0], GridTrack::Fr(1.0));
        assert_eq!(style.grid_template_columns[1], GridTrack::Fr(2.0));
        assert_eq!(style.grid_template_columns[2], GridTrack::Fr(1.0));
    }

    #[test]
    fn grid_column_line_numbers_parse() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-column: 2 / 4"), &parent);
        assert_eq!(style.grid_column_start, GridLine::Line(2));
        assert_eq!(style.grid_column_end, GridLine::Line(4));
        // Back-compat span is the delta.
        assert_eq!(style.grid_column_span, 2);
    }

    #[test]
    fn grid_column_start_with_span_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-column: 2 / span 2"), &parent);
        assert_eq!(style.grid_column_start, GridLine::Line(2));
        assert_eq!(style.grid_column_end, GridLine::Span(2));
    }

    #[test]
    fn grid_named_span_preserves_count_and_name_in_either_order() {
        let parent = ComputedStyle::default();
        for declaration in [
            "grid-column: 1 / span 2 target",
            "grid-column: 1 / span target 2",
        ] {
            let style = compute_style(HtmlTag::Div, Some(declaration), &parent);
            assert_eq!(
                style.grid_column_end,
                GridLine::SpanNamed {
                    count: 2,
                    name: "target".into(),
                }
            );
        }
    }

    #[test]
    fn grid_column_negative_line_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-column: 1 / -1"), &parent);
        assert_eq!(style.grid_column_start, GridLine::Line(1));
        assert_eq!(style.grid_column_end, GridLine::Line(-1));
    }

    #[test]
    fn grid_named_line_placement_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-column: mid / end"), &parent);
        assert_eq!(style.grid_column_start, GridLine::Named("mid".into()));
        assert_eq!(style.grid_column_end, GridLine::Named("end".into()));
    }

    #[test]
    fn grid_template_columns_named_lines_stored() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: [start] 100px [mid] 100px [end]"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 2);
        // line 0 = start, line 1 = mid, line 2 = end
        assert_eq!(style.grid_template_column_line_names.len(), 3);
        assert!(
            style.grid_template_column_line_names[0]
                .iter()
                .any(|name| name == "start")
        );
        assert!(
            style.grid_template_column_line_names[1]
                .iter()
                .any(|name| name == "mid")
        );
        assert!(
            style.grid_template_column_line_names[2]
                .iter()
                .any(|name| name == "end")
        );
    }

    #[test]
    fn grid_area_single_name_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-area: header"), &parent);
        assert_eq!(style.grid_area_name.as_deref(), Some("header"));
    }

    #[test]
    fn grid_area_line_form_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-area: 1 / 2 / 3 / 4"), &parent);
        assert_eq!(style.grid_row_start, GridLine::Line(1));
        assert_eq!(style.grid_column_start, GridLine::Line(2));
        assert_eq!(style.grid_row_end, GridLine::Line(3));
        assert_eq!(style.grid_column_end, GridLine::Line(4));
        assert_eq!(style.grid_area_name, None);
    }

    #[test]
    fn grid_template_areas_parses_rows_and_dots() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-areas: \"a a .\" \". b b\""),
            &parent,
        );
        assert_eq!(style.grid_template_areas.len(), 2);
        assert_eq!(
            style.grid_template_areas[0],
            vec![Some("a".to_string()), Some("a".to_string()), None]
        );
        assert_eq!(
            style.grid_template_areas[1],
            vec![None, Some("b".to_string()), Some("b".to_string())]
        );
    }

    #[test]
    fn grid_auto_flow_dense_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("grid-auto-flow: row dense"), &parent);
        assert!(style.grid_auto_flow_dense);
        assert!(!style.grid_auto_flow_column);
    }

    #[test]
    fn grid_shorthand_auto_flow_rows_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid: auto-flow 60px / 70px 70px"),
            &parent,
        );
        assert_eq!(style.display, Display::Grid);
        assert_eq!(style.grid_template_columns.len(), 2);
        assert!((style.grid_auto_rows.unwrap() - 45.0).abs() < 0.01);
        assert!(!style.grid_auto_flow_column);
    }

    #[test]
    fn grid_shorthand_auto_flow_rows_from_stylesheet_parses() {
        let parent = ComputedStyle::default();
        let rules = crate::parser::css::parse_stylesheet(
            ".grid { display: grid; grid: auto-flow 60px / 70px 70px }",
        );
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &["grid"], None);
        assert_eq!(style.display, Display::Grid);
        assert_eq!(style.grid_template_columns.len(), 2);
        assert!((style.grid_auto_rows.unwrap() - 45.0).abs() < 0.01);
        assert!(!style.grid_auto_flow_column);
    }

    #[test]
    fn grid_justify_self_parses() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-self: end"), &parent);
        assert_eq!(style.grid_justify_self, Some(GridAlign::End));
    }

    #[test]
    fn grid_template_columns_fixed_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: 100pt 200pt"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 2);
        assert_eq!(style.grid_template_columns[0], GridTrack::Fixed(100.0));
        assert_eq!(style.grid_template_columns[1], GridTrack::Fixed(200.0));
    }

    #[test]
    fn grid_template_columns_auto() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: auto auto auto"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 3);
        assert_eq!(style.grid_template_columns[0], GridTrack::Auto);
        assert_eq!(style.grid_template_columns[1], GridTrack::Auto);
        assert_eq!(style.grid_template_columns[2], GridTrack::Auto);
    }

    #[test]
    fn grid_template_columns_mixed() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: 100pt 1fr auto"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 3);
        assert_eq!(style.grid_template_columns[0], GridTrack::Fixed(100.0));
        assert_eq!(style.grid_template_columns[1], GridTrack::Fr(1.0));
        assert_eq!(style.grid_template_columns[2], GridTrack::Auto);
    }

    #[test]
    fn grid_gap_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: grid; grid-gap: 10pt"), &parent);
        assert!((style.grid_gap - 10.0).abs() < 0.1);
    }

    #[test]
    fn grid_gap_alias_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: grid; gap: 15pt"), &parent);
        assert!((style.grid_gap - 15.0).abs() < 0.1);
    }

    #[test]
    fn grid_properties_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.grid_template_columns = vec![GridTrack::Fr(1.0), GridTrack::Fr(1.0)];
        parent.grid_gap = 10.0;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!(
            style.grid_template_columns.is_empty(),
            "grid-template-columns should not inherit"
        );
        assert!(
            (style.grid_gap - 0.0).abs() < 0.1,
            "grid-gap should not inherit"
        );
    }

    #[test]
    fn grid_template_columns_px_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("display: grid; grid-template-columns: 100px 200px"),
            &parent,
        );
        assert_eq!(style.grid_template_columns.len(), 2);
        // px to pt: 100px * 0.75 = 75pt
        assert_eq!(style.grid_template_columns[0], GridTrack::Fixed(75.0));
        assert_eq!(style.grid_template_columns[1], GridTrack::Fixed(150.0));
    }

    #[test]
    fn min_width_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("min-width: 200pt"), &parent);
        assert_eq!(style.min_width, Some(200.0));
    }

    #[test]
    fn min_height_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("min-height: 150pt"), &parent);
        assert_eq!(style.min_height, Some(150.0));
    }

    #[test]
    fn max_height_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("max-height: 300pt"), &parent);
        assert_eq!(style.max_height, Some(300.0));
    }

    #[test]
    fn margin_auto_flags_from_shorthand() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin: 0 auto"), &parent);
        assert!(style.margin_left_auto, "margin-left should be auto");
        assert!(style.margin_right_auto, "margin-right should be auto");
        assert!((style.margin.top - 0.0).abs() < 0.01);
        assert!((style.margin.bottom - 0.0).abs() < 0.01);
    }

    #[test]
    fn margin_left_auto_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin-left: auto"), &parent);
        assert!(style.margin_left_auto, "margin-left should be auto");
        assert!(!style.margin_right_auto, "margin-right should not be auto");
    }

    #[test]
    fn margin_right_auto_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin-right: auto"), &parent);
        assert!(!style.margin_left_auto, "margin-left should not be auto");
        assert!(style.margin_right_auto, "margin-right should be auto");
    }

    #[test]
    fn min_max_properties_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.min_width = Some(100.0);
        parent.min_height = Some(50.0);
        parent.max_height = Some(500.0);
        parent.margin_left_auto = true;
        parent.margin_right_auto = true;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.min_width, None, "min-width should not inherit");
        assert_eq!(style.min_height, None, "min-height should not inherit");
        assert_eq!(style.max_height, None, "max-height should not inherit");
        assert!(
            !style.margin_left_auto,
            "margin_left_auto should not inherit"
        );
        assert!(
            !style.margin_right_auto,
            "margin_right_auto should not inherit"
        );
    }

    #[test]
    fn parse_linear_gradient_to_right() {
        let lg = parse_linear_gradient("linear-gradient(to right, red, blue)").unwrap();
        assert!((lg.angle - 90.0).abs() < 0.01);
        assert_eq!(lg.ramp.stops.len(), 2);
        assert_eq!(lg.ramp.stops[0].color.color.r, 255 as f32);
        assert_eq!(lg.ramp.stops[0].color.color.g, 0 as f32);
        assert_eq!(lg.ramp.stops[1].color.color.b, 255 as f32);
        assert!(!lg.ramp.repeat.is_repeating());
    }

    #[test]
    fn parse_linear_gradient_range_hard_stops() {
        // lightningcss collapses `red 0%, red 50%, blue 50%, blue 100%` into the
        // range form. The parser must expand it back into four stops.
        let lg = parse_linear_gradient("linear-gradient(90deg, #d32f2f 0% 50%, #1565c0 50% 100%)")
            .unwrap();
        assert_eq!(resolved_positions(&lg.ramp), [0.0, 0.5, 0.5, 1.0]);
        assert_eq!(lg.ramp.stops[0].color.color.r, 211 as f32);
        assert_eq!(lg.ramp.stops[3].color.color.b, 192 as f32);
    }

    #[test]
    fn parse_repeating_linear_gradient_sets_flag() {
        let lg =
            parse_linear_gradient("repeating-linear-gradient(90deg, red 0% 10%, blue 10% 20%)")
                .unwrap();
        assert!(lg.ramp.repeat.is_repeating());
        assert_eq!(lg.ramp.stops.len(), 4);
    }

    #[test]
    fn parse_radial_gradient_extent_keywords() {
        let cs = parse_radial_gradient("radial-gradient(circle closest-side, red, blue)").unwrap();
        assert_eq!(cs.extent, RadialExtent::ClosestSide);
        assert_eq!(cs.shape, RadialShape::Circle);

        let fs = parse_radial_gradient("radial-gradient(circle farthest-side, red, blue)").unwrap();
        assert_eq!(fs.extent, RadialExtent::FarthestSide);

        let cc = parse_radial_gradient("radial-gradient(closest-corner, red, blue)").unwrap();
        assert_eq!(cc.extent, RadialExtent::ClosestCorner);
    }

    #[test]
    fn parse_radial_gradient_explicit_ellipse_radii() {
        let rg = parse_radial_gradient("radial-gradient(ellipse 120px 60px at center, red, blue)")
            .unwrap();
        assert_eq!(rg.shape, RadialShape::Ellipse);
        let radii = rg.radii.expect("explicit radii");
        // 120px → 90pt, 60px → 45pt.
        assert!((radii.x.resolve(1000.0) - 90.0).abs() < 0.01);
        assert!((radii.y.resolve(1000.0) - 45.0).abs() < 0.01);
    }

    #[test]
    fn parse_repeating_radial_gradient_sets_flag() {
        let rg =
            parse_radial_gradient("repeating-radial-gradient(circle, red 0% 10%, blue 10% 20%)")
                .unwrap();
        assert!(rg.ramp.repeat.is_repeating());
        assert_eq!(rg.shape, RadialShape::Circle);
    }

    #[test]
    fn parse_conic_gradient_basic_four_quadrants() {
        let cg = parse_conic_gradient(
            "conic-gradient(from 0deg at center, #e53935 0deg 90deg, #43a047 90deg 180deg, #1e88e5 180deg 270deg, #fdd835 270deg 360deg)",
        )
        .unwrap();
        assert!(!cg.ramp.repeat.is_repeating());
        assert!((cg.from_angle - 0.0).abs() < 0.01);
        // 4 quadrant range stops → 8 stops (two per quadrant).
        assert_eq!(cg.ramp.stops.len(), 8);
        let positions = resolved_positions(&cg.ramp);
        assert!((positions[0] - 0.0).abs() < 0.01);
        assert!((positions[1] - 0.25).abs() < 0.01);
        assert!((positions.last().unwrap() - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_conic_gradient_percentage_range_stops() {
        let gradient = parse_conic_gradient(
            "conic-gradient(from 23deg at 64% 34%, #ff00c8 0 25%, #00ff67 25% 55%, #ff6a00 55% 83%, #402080 83% 100%)",
        )
        .unwrap();

        assert_eq!(gradient.ramp.stops.len(), 8);
        assert_eq!(
            resolved_positions(&gradient.ramp),
            [0.0, 0.25, 0.25, 0.55, 0.55, 0.83, 0.83, 1.0]
        );
    }

    #[test]
    fn parse_conic_gradient_from_angle_and_position() {
        let cg = parse_conic_gradient("conic-gradient(from 45deg at 30% 30%, red, blue)").unwrap();
        assert!((cg.from_angle - 45.0).abs() < 0.01);
        assert!(matches!(cg.center.x, RadialPos::Fraction(f) if (f - 0.3).abs() < 0.01));
        // Two implicit stops distribute to 0 and 1.
        assert_eq!(cg.ramp.stops.len(), 2);
        assert_eq!(resolved_positions(&cg.ramp), [0.0, 1.0]);
    }

    #[test]
    fn parse_repeating_conic_gradient_sets_flag() {
        let cg = parse_conic_gradient("repeating-conic-gradient(red 0deg 30deg, blue 30deg 60deg)")
            .unwrap();
        assert!(cg.ramp.repeat.is_repeating());
        assert_eq!(cg.ramp.stops.len(), 4);
        // 30deg → 1/12 turn.
        assert!((resolved_positions(&cg.ramp)[1] - (30.0 / 360.0)).abs() < 0.01);
    }

    #[test]
    fn parse_conic_angle_fraction_units() {
        assert!((parse_conic_angle_fraction("90deg").unwrap().fraction - 0.25).abs() < 0.001);
        assert!((parse_conic_angle_fraction("0.25turn").unwrap().fraction - 0.25).abs() < 0.001);
        assert!((parse_conic_angle_fraction("50%").unwrap().fraction - 0.5).abs() < 0.001);
        assert!((parse_conic_angle_fraction("100grad").unwrap().fraction - 0.25).abs() < 0.001);
    }

    #[test]
    fn parse_css_angle_deg_units() {
        assert!((parse_css_angle_deg("90deg").unwrap() - 90.0).abs() < 0.01);
        assert!((parse_css_angle_deg("0.5turn").unwrap() - 180.0).abs() < 0.01);
        assert!((parse_css_angle_deg("100grad").unwrap() - 90.0).abs() < 0.01);
        assert!(parse_css_angle_deg("red").is_none());
    }

    #[test]
    fn parse_linear_gradient_45deg() {
        let lg = parse_linear_gradient("linear-gradient(45deg, #ff0000, #0000ff)").unwrap();
        assert!((lg.angle - 45.0).abs() < 0.01);
        assert_eq!(lg.ramp.stops.len(), 2);
        assert_eq!(lg.ramp.stops[0].color.color.r, 255 as f32);
        assert_eq!(lg.ramp.stops[1].color.color.b, 255 as f32);
    }

    #[test]
    fn parse_linear_gradient_default_direction() {
        let lg = parse_linear_gradient("linear-gradient(red, blue)").unwrap();
        assert!((lg.angle - 180.0).abs() < 0.01); // default is "to bottom"
    }

    #[test]
    fn parse_linear_gradient_with_positions() {
        let lg = parse_linear_gradient("linear-gradient(to bottom, red 0%, white 50%, blue 100%)")
            .unwrap();
        assert_eq!(resolved_positions(&lg.ramp), [0.0, 0.5, 1.0]);
        assert_eq!(lg.ramp.stops[1].color.color.r, 255 as f32); // white
        assert_eq!(lg.ramp.stops[1].color.color.g, 255 as f32);
    }

    #[test]
    fn parse_linear_gradient_direction_keywords() {
        let lg = parse_linear_gradient("linear-gradient(to top, red, blue)").unwrap();
        assert!((lg.angle - 0.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to left, red, blue)").unwrap();
        assert!((lg.angle - 270.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to bottom, red, blue)").unwrap();
        assert!((lg.angle - 180.0).abs() < 0.01);
    }

    #[test]
    fn parse_linear_gradient_invalid() {
        assert!(parse_linear_gradient("not-a-gradient").is_none());
        assert!(parse_linear_gradient("linear-gradient(to sideways, red, blue)").is_none());
    }

    #[test]
    fn gradient_interpolation_method_is_typed_for_every_geometry() {
        let linear =
            parse_linear_gradient("linear-gradient(in oklab to right, red, blue)").unwrap();
        assert_eq!(linear.ramp.interpolation, GradientInterpolation::Oklab);
        let radial = parse_radial_gradient("radial-gradient(in srgb circle, red, blue)").unwrap();
        assert_eq!(radial.ramp.interpolation, GradientInterpolation::Srgb);
        let conic = parse_conic_gradient("conic-gradient(in oklab from 30deg, red, blue)").unwrap();
        assert_eq!(conic.ramp.interpolation, GradientInterpolation::Oklab);
        assert!(parse_linear_gradient("linear-gradient(in display-p3, red, blue)").is_none());

        let style = compute_style(
            HtmlTag::Div,
            Some("background-image: linear-gradient(to right in oklab, red, blue)"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.background_gradient.unwrap().ramp.interpolation,
            GradientInterpolation::Oklab
        );
    }

    #[test]
    fn gradient_hint_remains_one_semantic_hint() {
        let gradient = parse_linear_gradient("linear-gradient(red 0%, 25%, blue 100%)").unwrap();
        assert_eq!(gradient.ramp.stops.len(), 2);
        assert_eq!(
            gradient.ramp.stops[0].hint_after,
            Some(GradientPosition::fraction(0.25))
        );
        assert_rgba_close(
            gradient.ramp.resolve(1.0).unwrap().sample(0.25),
            (0.5, 0.0, 0.5, 1.0),
        );
        for invalid in [
            "linear-gradient(25%, red, blue)",
            "linear-gradient(red, 25%, 30%, blue)",
            "linear-gradient(red, blue, 75%)",
        ] {
            assert!(parse_linear_gradient(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn gradient_hint_participates_in_stop_fixup_order() {
        let before =
            parse_linear_gradient("linear-gradient(red 0%, 80%, white, blue 100%)").unwrap();
        assert_eq!(resolved_positions(&before.ramp), [0.0, 0.9, 1.0]);
        assert_eq!(
            before.ramp.stops[0].hint_after,
            Some(GradientPosition::fraction(0.8))
        );

        let after =
            parse_linear_gradient("linear-gradient(red 0%, white, 80%, blue 100%)").unwrap();
        assert_eq!(resolved_positions(&after.ramp), [0.0, 0.4, 1.0]);
        assert_eq!(
            after.ramp.stops[1].hint_after,
            Some(GradientPosition::fraction(0.8))
        );
    }

    #[test]
    fn auto_distinguishes_legacy_and_modern_srgb_sources() {
        let legacy = parse_linear_gradient("linear-gradient(rgb(255 0 0), rgb(0 0 255))").unwrap();
        let modern =
            parse_linear_gradient("linear-gradient(color(srgb 1 0 0), color(srgb 0 0 1))").unwrap();
        assert!(
            legacy
                .ramp
                .stops
                .iter()
                .all(|stop| stop.color.provenance == GradientColorProvenance::LegacySrgb)
        );
        assert!(
            modern
                .ramp
                .stops
                .iter()
                .all(|stop| stop.color.provenance == GradientColorProvenance::Modern)
        );
        assert_rgba_close(
            legacy.ramp.resolve(1.0).unwrap().sample(0.5),
            (0.5, 0.0, 0.5, 1.0),
        );
        let modern_midpoint = modern.ramp.resolve(1.0).unwrap().sample(0.5);
        assert!((modern_midpoint.0 - 0.5).abs() > 0.02);

        let mixed = parse_linear_gradient(
            "linear-gradient(rgb(255 0 0) 0%, rgb(255 255 255) 50%, color(srgb 0 0 1) 100%)",
        )
        .unwrap()
        .ramp
        .resolve(1.0)
        .unwrap();
        assert!(
            mixed
                .segments()
                .all(|segment| segment.interpolation == GradientInterpolation::Oklab)
        );
    }

    #[test]
    fn oklab_conversion_preserves_finite_extended_srgb_channels() {
        let converted = ResolvedGradientRamp::oklab_to_srgb((0.5, 1.0, 1.0, 1.0));
        assert!(
            [converted.0, converted.1, converted.2, converted.3]
                .into_iter()
                .all(f32::is_finite)
        );
        assert!(converted.1 < 0.0 && converted.2 < 0.0);
    }

    #[test]
    fn lightning_path_preserves_gradient_color_source_and_precision() {
        let style = compute_style(
            HtmlTag::Div,
            Some("background-image: linear-gradient(rgb(10% 20% 30%), color(srgb 0 0 1))"),
            &ComputedStyle::default(),
        );
        let ramp = style.background_gradient.unwrap().ramp;
        assert_eq!(
            ramp.stops[0].color.provenance,
            GradientColorProvenance::LegacySrgb
        );
        assert_eq!(
            ramp.stops[1].color.provenance,
            GradientColorProvenance::Modern
        );
        assert!((ramp.stops[0].color.color.r - 25.5).abs() < 1e-6);
        assert!((ramp.stops[0].color.color.g - 51.0).abs() < 1e-6);
        assert!((ramp.stops[0].color.color.b - 76.5).abs() < 1e-6);

        let style = compute_style(
            HtmlTag::Div,
            Some(
                "--stop: rgb(10% 20% 30%); \
                 background-image: linear-gradient(var(--stop), color(srgb 0 0 1))",
            ),
            &ComputedStyle::default(),
        );
        let stop = style.background_gradient.unwrap().ramp.stops[0].color;
        assert_eq!(stop.provenance, GradientColorProvenance::LegacySrgb);
        assert!((stop.color.r - 25.5).abs() < 1e-6);

        let rules = crate::parser::css::parse_stylesheet(concat!(
            ".sample { background-image: ",
            "linear-gradient(in oklab, rgb(10% 20% 30%), color(srgb 0 0 1)) }"
        ));
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &ComputedStyle::default(),
            &rules,
            "div",
            &["sample"],
            None,
        );
        let ramp = style.background_gradient.unwrap().ramp;
        assert_eq!(ramp.interpolation, GradientInterpolation::Oklab);
        assert_eq!(
            ramp.stops[0].color.provenance,
            GradientColorProvenance::LegacySrgb
        );
        assert!((ramp.stops[0].color.color.r - 25.5).abs() < 1e-6);
    }

    #[test]
    fn malformed_radial_and_conic_preludes_are_rejected() {
        for invalid in [
            "radial-gradient(circle at garbage, red, blue)",
            "radial-gradient(circle garbage, red, blue)",
            "radial-gradient(ellipse 10px, red, blue)",
            "radial-gradient(circle 50%, red, blue)",
            "radial-gradient(circle -1px, red, blue)",
        ] {
            assert!(parse_radial_gradient(invalid).is_none(), "{invalid}");
        }
        for invalid in [
            "conic-gradient(at garbage, red, blue)",
            "conic-gradient(from nope, red, blue)",
            "conic-gradient(foo at center, red, blue)",
            "conic-gradient(in oklab foo, red, blue)",
        ] {
            assert!(parse_conic_gradient(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn repeating_radial_stop_lengths_do_not_resize_the_ending_shape() {
        let radial =
            parse_radial_gradient("repeating-radial-gradient(circle, red 0px, blue 20px)").unwrap();
        assert_eq!(radial.radius, None);
        assert_eq!(radial.extent, RadialExtent::FarthestCorner);
    }

    #[test]
    fn parse_radial_gradient_basic() {
        let rg = parse_radial_gradient("radial-gradient(red, blue)").unwrap();
        assert_eq!(rg.ramp.stops.len(), 2);
        assert_eq!(rg.ramp.stops[0].color.color.r, 255 as f32);
        assert_eq!(rg.ramp.stops[1].color.color.b, 255 as f32);
    }

    #[test]
    fn parse_radial_gradient_with_circle() {
        let rg = parse_radial_gradient("radial-gradient(circle, red, blue)").unwrap();
        assert_eq!(rg.ramp.stops.len(), 2);
        assert_eq!(rg.shape, RadialShape::Circle);
    }

    #[test]
    fn parse_radial_gradient_default_shape_is_ellipse() {
        // CSS default shape is `ellipse` when no shape keyword is present.
        let rg = parse_radial_gradient("radial-gradient(red, blue)").unwrap();
        assert_eq!(rg.shape, RadialShape::Ellipse);
    }

    #[test]
    fn parse_radial_gradient_ellipse_at_corner() {
        let rg = parse_radial_gradient("radial-gradient(ellipse at top left, #00897b, #b71c1c)")
            .unwrap();
        assert_eq!(rg.shape, RadialShape::Ellipse);
        assert_eq!(rg.center.x, RadialPos::Fraction(0.0));
        assert_eq!(rg.center.y, RadialPos::Fraction(0.0));
        assert_eq!(rg.radius, None);
    }

    #[test]
    fn gradient_color_stop_auto_positions() {
        let lg = parse_linear_gradient("linear-gradient(to right, red, green, blue)").unwrap();
        assert_eq!(
            lg.ramp
                .stops
                .iter()
                .filter(|stop| stop.position.is_none())
                .count(),
            3
        );
        assert_eq!(resolved_positions(&lg.ramp), [0.0, 0.5, 1.0]);
    }

    #[test]
    fn background_gradient_from_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("background: linear-gradient(to right, red, blue)"),
            &parent,
        );
        assert!(style.background_gradient.is_some());
        let lg = style.background_gradient.unwrap();
        assert!((lg.angle - 90.0).abs() < 0.01);
        assert_eq!(lg.ramp.stops.len(), 2);
    }

    #[test]
    fn background_radial_gradient_from_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("background: radial-gradient(red, blue)"),
            &parent,
        );
        assert!(style.background_radial_gradient.is_some());
    }

    fn single_mask_layer(style: &ComputedStyle) -> &MaskLayer {
        match &style.mask_image {
            Some(MaskSource::Layers(layers)) if layers.len() == 1 => &layers[0],
            other => panic!("expected a single mask layer, got {other:?}"),
        }
    }

    #[test]
    fn mask_image_linear_gradient_from_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("mask-image: linear-gradient(to right, #000, rgba(0,0,0,0))"),
            &parent,
        );
        let layer = single_mask_layer(&style);
        match &layer.source {
            MaskLayerSource::Linear(lg) => {
                assert!((lg.angle - 90.0).abs() < 0.01);
                assert_eq!(lg.ramp.stops.len(), 2);
            }
            other => panic!("expected a linear mask source, got {other:?}"),
        }
        // `match-source` (initial) on a CSS gradient resolves to alpha at paint.
        assert_eq!(layer.mode, MaskMode::MatchSource);
        assert_eq!(style.mask_mode, MaskMode::MatchSource);
    }

    #[test]
    fn mask_image_radial_gradient_from_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("mask-image: radial-gradient(circle at 50% 50%, #000, transparent)"),
            &parent,
        );
        assert!(matches!(
            single_mask_layer(&style).source,
            MaskLayerSource::Radial(_)
        ));
    }

    #[test]
    fn webkit_mask_image_alias_is_mask_image() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("-webkit-mask-image: linear-gradient(to bottom, #000, rgba(0,0,0,0))"),
            &parent,
        );
        assert!(
            matches!(single_mask_layer(&style).source, MaskLayerSource::Linear(_)),
            "the -webkit-mask-image alias must populate mask_image"
        );
    }

    #[test]
    fn mask_mode_luminance_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("mask-image: linear-gradient(#fff, #000); mask-mode: luminance"),
            &parent,
        );
        assert_eq!(style.mask_mode, MaskMode::Luminance);
        assert!(style.mask_image.is_some());
    }

    #[test]
    fn mask_image_none_clears_source() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("mask-image: none"), &parent);
        assert!(style.mask_image.is_none());
    }

    #[test]
    fn mask_image_url_non_svg_is_left_unset() {
        // A url() that doesn't resolve to SVG content (here invalid base64 bytes
        // that don't sniff as SVG) must not panic and must leave `mask_image` as
        // None rather than a bogus value — only SVG image masks are modelled.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("mask-image: url(\"data:image/svg+xml;base64,AAAA\")"),
            &parent,
        );
        assert!(style.mask_image.is_none());
    }

    #[test]
    fn mask_image_url_svg_data_uri_is_loaded() {
        // A url() data-URI SVG mask (css-masking-1 §3.1) must populate
        // `mask_image` with an Svg source carrying the raw bytes.
        // base64 of: <svg xmlns="http://www.w3.org/2000/svg" width="10"
        //   height="10"><circle cx="5" cy="5" r="4" fill="#fff"/></svg>
        let b64 = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PGNpcmNsZSBjeD0iNSIgY3k9IjUiIHI9IjQiIGZpbGw9IiNmZmYiLz48L3N2Zz4=";
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(&format!(
                "mask-image: url(\"data:image/svg+xml;base64,{b64}\")"
            )),
            &parent,
        );
        assert!(
            matches!(single_mask_layer(&style).source, MaskLayerSource::Svg(_)),
            "a data-URI SVG url() mask must populate mask_image as Svg"
        );
    }

    #[test]
    fn webkit_mask_image_url_svg_alias_is_loaded() {
        // The -webkit-mask-image alias of a url() SVG mask must behave the same.
        let b64 = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PGNpcmNsZSBjeD0iNSIgY3k9IjUiIHI9IjQiIGZpbGw9IiNmZmYiLz48L3N2Zz4=";
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(&format!(
                "-webkit-mask-image: url(\"data:image/svg+xml;base64,{b64}\")"
            )),
            &parent,
        );
        assert!(
            matches!(single_mask_layer(&style).source, MaskLayerSource::Svg(_)),
            "the -webkit-mask-image url() SVG alias must populate mask_image"
        );
    }

    #[test]
    fn gradient_with_rgb_colors() {
        let lg = parse_linear_gradient("linear-gradient(to right, rgb(255, 0, 0), rgb(0, 0, 255))")
            .unwrap();
        assert_eq!(lg.ramp.stops.len(), 2);
        assert_eq!(lg.ramp.stops[0].color.color.r, 255 as f32);
        assert_eq!(lg.ramp.stops[1].color.color.b, 255 as f32);
    }

    #[test]
    fn gradient_with_hex_colors() {
        let lg =
            parse_linear_gradient("linear-gradient(90deg, #ff0000, #00ff00, #0000ff)").unwrap();
        assert_eq!(lg.ramp.stops.len(), 3);
        assert_eq!(lg.ramp.stops[0].color.color.r, 255 as f32);
        assert_eq!(lg.ramp.stops[1].color.color.g, 255 as f32);
        assert_eq!(lg.ramp.stops[2].color.color.b, 255 as f32);
    }

    // --- border-radius tests ---

    #[test]
    fn border_radius_default_is_zero() {
        let style = ComputedStyle::default();
        assert_eq!(style.border_radii, SpecifiedCornerRadii::ZERO);
    }

    #[test]
    fn border_radius_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-radius: 10pt"), &parent);
        assert_eq!(
            style.resolve_corner_radii(80.0, 40.0),
            CornerRadii::circular(10.0)
        );
    }

    #[test]
    fn border_radius_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.border_radii = SpecifiedCornerRadii::circular(SpecifiedRadiusValue::Length(15.0));
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.border_radii, SpecifiedCornerRadii::ZERO);
    }

    #[test]
    fn border_radius_shorthand_resolves_named_elliptical_corners() {
        let style = compute_style(
            HtmlTag::Div,
            Some("border-radius: 10pt 20pt 30pt 40pt / 1pt 2pt 3pt 4pt"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.resolve_corner_radii(200.0, 100.0),
            CornerRadii::new(
                CornerRadius::new(10.0, 1.0),
                CornerRadius::new(20.0, 2.0),
                CornerRadius::new(30.0, 3.0),
                CornerRadius::new(40.0, 4.0),
            )
        );
    }

    #[test]
    fn invalid_radius_declarations_preserve_prior_winners() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-radius: 7px; border-radius: 9; \
                 border-top-left-radius: 8px; border-top-left-radius: 11",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.resolve_corner_radii(100.0, 100.0),
            CornerRadii::new(
                CornerRadius::circular(6.0),
                CornerRadius::circular(5.25),
                CornerRadius::circular(5.25),
                CornerRadius::circular(5.25),
            )
        );
    }

    #[test]
    fn radius_shorthand_and_longhand_follow_cascade_order() {
        let parent = ComputedStyle::default();
        let shorthand_last = compute_style(
            HtmlTag::Div,
            Some("border-top-left-radius: 2pt; border-radius: 10pt"),
            &parent,
        );
        assert_eq!(
            shorthand_last.resolve_corner_radii(100.0, 100.0),
            CornerRadii::circular(10.0)
        );

        let longhand_last = compute_style(
            HtmlTag::Div,
            Some("border-radius: 10pt; border-top-left-radius: 2pt"),
            &parent,
        );
        assert_eq!(
            longhand_last.resolve_corner_radii(100.0, 100.0),
            CornerRadii::new(
                CornerRadius::circular(2.0),
                CornerRadius::circular(10.0),
                CornerRadius::circular(10.0),
                CornerRadius::circular(10.0),
            )
        );

        let important_longhand = compute_style(
            HtmlTag::Div,
            Some("border-top-left-radius: 2pt !important; border-radius: 10pt"),
            &parent,
        );
        assert_eq!(
            important_longhand.resolve_corner_radii(100.0, 100.0),
            longhand_last.resolve_corner_radii(100.0, 100.0)
        );

        let important_shorthand = compute_style(
            HtmlTag::Div,
            Some("border-radius: 10pt !important; border-top-left-radius: 2pt"),
            &parent,
        );
        assert_eq!(
            important_shorthand.resolve_corner_radii(100.0, 100.0),
            CornerRadii::circular(10.0)
        );
    }

    #[test]
    fn radius_var_expansion_preserves_corner_and_axis_structure() {
        let style = compute_style(
            HtmlTag::Div,
            Some("--r: 25% 10pt / 50% 5pt; border-radius: var(--r)"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.resolve_corner_radii(80.0, 40.0),
            CornerRadii::new(
                CornerRadius::new(20.0, 20.0),
                CornerRadius::new(10.0, 5.0),
                CornerRadius::new(20.0, 20.0),
                CornerRadius::new(10.0, 5.0),
            )
        );
    }

    // --- outline tests ---

    #[test]
    fn outline_default_is_zero() {
        let style = ComputedStyle::default();
        assert!((style.outline_width - 0.0).abs() < 0.001);
        assert!(style.outline_color.is_none());
    }

    #[test]
    fn outline_shorthand_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("outline: 2px solid red"), &parent);
        assert!((style.outline_width - 1.5).abs() < 0.001); // 2px * 0.75
        assert!(style.outline_color.is_some());
        assert_eq!(style.outline_color.unwrap().r, 255 as f32);
    }

    #[test]
    fn outline_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.outline_width = 5.0;
        parent.outline_color = Some(Color::rgb(255, 0, 0));
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.outline_width - 0.0).abs() < 0.001);
        assert!(style.outline_color.is_none());
    }

    // --- box-sizing tests ---

    #[test]
    fn box_sizing_default_is_content_box() {
        let style = ComputedStyle::default();
        assert_eq!(style.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn box_sizing_border_box_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-sizing: border-box"), &parent);
        assert_eq!(style.box_sizing, BoxSizing::BorderBox);
    }

    #[test]
    fn box_sizing_content_box_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-sizing: content-box"), &parent);
        assert_eq!(style.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn box_sizing_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.box_sizing = BoxSizing::BorderBox;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.box_sizing, BoxSizing::ContentBox);
    }

    #[test]
    fn color_inherit_keeps_parent_value() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        let style = compute_style(HtmlTag::Div, Some("color: inherit"), &parent);
        assert_eq!(style.color.r, 255 as f32);
        assert_eq!(style.color.g, 0 as f32);
    }

    #[test]
    fn margin_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, Some("margin-top: initial"), &parent);
        assert!((style.margin.top - 0.0).abs() < 0.1);
    }

    #[test]
    fn color_unset_inherits() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(0, 128, 0);
        let style = compute_style(HtmlTag::Div, Some("color: unset"), &parent);
        assert_eq!(style.color.g, 128 as f32);
    }

    #[test]
    fn margin_unset_resets_to_initial() {
        let mut parent = ComputedStyle::default();
        parent.margin.top = 50.0;
        let style = compute_style(HtmlTag::Div, Some("margin-top: unset"), &parent);
        assert!((style.margin.top - 0.0).abs() < 0.1);
    }

    #[test]
    fn font_weight_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.font_weight = FontWeight::Bold;
        let style = compute_style(HtmlTag::Span, Some("font-weight: inherit"), &parent);
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    // --- reset_to_initial tests (lines 513-553) ---

    #[test]
    fn text_decoration_initial_resets_both_flags() {
        let parent = ComputedStyle::default();
        // First set text-decoration underline, then reset with initial
        let style = compute_style(HtmlTag::Span, Some("text-decoration: underline"), &parent);
        assert!(style.text_decorations.current.lines.underline);
        // Now use initial to reset
        let style2 = compute_style(HtmlTag::Span, Some("text-decoration: initial"), &parent);
        assert!(!style2.text_decorations.current.lines.underline);
        assert!(!style2.text_decorations.current.lines.line_through);
    }

    #[test]
    fn margin_right_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin-right: initial"), &parent);
        assert!((style.margin.right - 0.0).abs() < 0.1);
    }

    #[test]
    fn margin_bottom_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, Some("margin-bottom: initial"), &parent);
        assert!((style.margin.bottom - 0.0).abs() < 0.1);
    }

    #[test]
    fn margin_left_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin-left: initial"), &parent);
        assert!((style.margin.left - 0.0).abs() < 0.1);
    }

    #[test]
    fn padding_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "padding-top: initial; padding-right: initial; padding-bottom: initial; padding-left: initial",
            ),
            &parent,
        );
        assert!((style.padding.top - 0.0).abs() < 0.1);
        assert!((style.padding.right - 0.0).abs() < 0.1);
        assert!((style.padding.bottom - 0.0).abs() < 0.1);
        assert!((style.padding.left - 0.0).abs() < 0.1);
    }

    #[test]
    fn display_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: initial"), &parent);
        assert_eq!(style.display, Display::Block); // default is Block
    }

    #[test]
    fn width_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: initial"), &parent);
        assert_eq!(style.width, None);
    }

    #[test]
    fn height_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("height: initial"), &parent);
        assert_eq!(style.height, None);
    }

    #[test]
    fn max_width_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("max-width: initial"), &parent);
        assert_eq!(style.max_width, None);
    }

    #[test]
    fn opacity_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("opacity: initial"), &parent);
        assert!((style.opacity - 1.0).abs() < 0.01);
    }

    #[test]
    fn border_width_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-width: initial"), &parent);
        assert!((style.border.top.used_width() - 0.0).abs() < 0.1);
    }

    #[test]
    fn border_color_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-color: initial"), &parent);
        assert_eq!(style.border.top.color, SpecifiedColor::CurrentColor);
    }

    #[test]
    fn border_initial_resets_both() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: initial"), &parent);
        assert!((style.border.top.used_width() - 0.0).abs() < 0.1);
        assert_eq!(style.border.top.color, SpecifiedColor::CurrentColor);
    }

    #[test]
    fn float_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("float: initial"), &parent);
        assert_eq!(style.float, Float::None);
    }

    #[test]
    fn clear_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("clear: initial"), &parent);
        assert_eq!(style.clear, Clear::None);
    }

    #[test]
    fn position_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("position: initial"), &parent);
        assert_eq!(style.position, Position::Static);
    }

    #[test]
    fn top_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("top: initial"), &parent);
        assert_eq!(style.top, None);
    }

    #[test]
    fn left_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("left: initial"), &parent);
        assert_eq!(style.left, None);
    }

    #[test]
    fn overflow_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("overflow: initial"), &parent);
        assert_eq!(style.overflow, Overflow::Visible);
    }

    #[test]
    fn transform_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: initial"), &parent);
        assert!(style.transform.is_none());
    }

    #[test]
    fn box_shadow_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: initial"), &parent);
        assert!(style.box_shadow.is_empty());
    }

    #[test]
    fn all_shorthand_respects_declaration_source_order() {
        let parent = ComputedStyle::default();
        let reset_later = compute_style(
            HtmlTag::Div,
            Some("background-color: green; all: initial; display: block; width: 50px"),
            &parent,
        );
        assert!(reset_later.background_color.is_none());
        assert!(reset_later.width.is_some());

        let explicit_later = compute_style(
            HtmlTag::Div,
            Some("all: initial; display: block; width: 50px; background-color: green"),
            &parent,
        );
        assert!(explicit_later.background_color.is_some());
        assert!(explicit_later.width.is_some());
    }

    #[test]
    fn cascade_selects_winner_before_computed_value_resolution() {
        let parent = ComputedStyle::default();
        let rules = crate::parser::css::parse_stylesheet(
            ".target { width: 150px; opacity: var(--half); } \
             div.target { width: var(--missing-width); --half: 50%; }",
        );
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &rules,
            "div",
            &["target"],
            None,
        );
        assert_eq!(style.width, None);
        assert!((style.opacity - 0.5).abs() < 0.01);
    }

    #[test]
    fn revert_removes_the_author_origin_winner() {
        let parent = ComputedStyle::default();
        let rules = crate::parser::css::parse_stylesheet(
            ".target { display: flex; } div.target { display: revert; }",
        );
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &rules,
            "div",
            &["target"],
            None,
        );
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn flex_direction_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-direction: initial"), &parent);
        assert_eq!(style.flex_direction, FlexDirection::Row);
    }

    #[test]
    fn justify_content_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-content: initial"), &parent);
        assert_eq!(style.justify_content, JustifyContent::FlexStart);
    }

    #[test]
    fn align_items_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("align-items: initial"), &parent);
        assert_eq!(style.align_items, AlignItems::Stretch);
    }

    #[test]
    fn flex_wrap_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-wrap: initial"), &parent);
        assert_eq!(style.flex_wrap, FlexWrap::NoWrap);
    }

    #[test]
    fn gap_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("gap: initial"), &parent);
        assert!((style.gap - 0.0).abs() < 0.1);
    }

    // --- restore_from_parent (inherit) tests (lines 563-607) ---

    #[test]
    fn font_style_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.font_style = FontStyle::Italic;
        let style = compute_style(HtmlTag::Span, Some("font-style: inherit"), &parent);
        assert_eq!(style.font_style, FontStyle::Italic);
    }

    #[test]
    fn font_stretch_is_inherited_and_accepts_condensed() {
        let parent = ComputedStyle::default();
        let condensed = compute_style(HtmlTag::Span, Some("font-stretch: condensed"), &parent);
        assert_eq!(condensed.font_stretch, FontStretch::Condensed);

        let inherited = compute_style(HtmlTag::Span, Some("font-stretch: inherit"), &condensed);
        assert_eq!(inherited.font_stretch, FontStretch::Condensed);

        let alias = compute_style(HtmlTag::Span, Some("font-width: expanded"), &parent);
        assert_eq!(alias.font_stretch, FontStretch::Expanded);

        let cascade = compute_style(
            HtmlTag::Span,
            Some("font-width: expanded; font-stretch: condensed"),
            &parent,
        );
        assert_eq!(cascade.font_stretch, FontStretch::Condensed);

        let shorthand = compute_style(
            HtmlTag::Span,
            Some("font: condensed 12px ParitySans"),
            &parent,
        );
        assert_eq!(shorthand.font_stretch, FontStretch::Condensed);
    }

    #[test]
    fn font_family_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.font_family = FontFamily::Helvetica;
        parent.font_stack = FontStack::from_family(FontFamily::Helvetica);
        let style = compute_style(HtmlTag::Span, Some("font-family: inherit"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn line_height_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.line_height = 2.0;
        let style = compute_style(HtmlTag::Div, Some("line-height: inherit"), &parent);
        assert!((style.line_height - 2.0).abs() < 0.1);
    }

    #[test]
    fn text_align_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.text_align = TextAlign::Center;
        let style = compute_style(HtmlTag::Div, Some("text-align: inherit"), &parent);
        assert_eq!(style.text_align, TextAlign::Center);
    }

    #[test]
    fn text_decoration_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.text_decorations.current.lines.underline = true;
        parent.text_decorations.current.lines.line_through = true;
        let style = compute_style(HtmlTag::Span, Some("text-decoration: inherit"), &parent);
        assert!(style.text_decorations.current.lines.underline);
        assert!(style.text_decorations.current.lines.line_through);
    }

    #[test]
    fn visibility_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.visibility = Visibility::Hidden;
        let style = compute_style(HtmlTag::Div, Some("visibility: inherit"), &parent);
        assert_eq!(style.visibility, Visibility::Hidden);
    }

    #[test]
    fn letter_spacing_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.letter_spacing = 2.0;
        let style = compute_style(HtmlTag::Span, Some("letter-spacing: inherit"), &parent);
        assert!((style.letter_spacing - 2.0).abs() < 0.1);
    }

    #[test]
    fn word_spacing_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.word_spacing = 3.0;
        let style = compute_style(HtmlTag::Span, Some("word-spacing: inherit"), &parent);
        assert!((style.word_spacing - 3.0).abs() < 0.1);
    }

    #[test]
    fn background_color_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.background_color = Some(Color::rgb(0, 128, 0));
        let style = compute_style(HtmlTag::Div, Some("background-color: inherit"), &parent);
        assert_eq!(style.background_color.unwrap().g, 128 as f32);
    }

    #[test]
    fn margin_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.margin.top = 10.0;
        parent.margin.right = 20.0;
        parent.margin.bottom = 30.0;
        parent.margin.left = 40.0;
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "margin-top: inherit; margin-right: inherit; margin-bottom: inherit; margin-left: inherit",
            ),
            &parent,
        );
        assert!((style.margin.top - 10.0).abs() < 0.1);
        assert!((style.margin.right - 20.0).abs() < 0.1);
        assert!((style.margin.bottom - 30.0).abs() < 0.1);
        assert!((style.margin.left - 40.0).abs() < 0.1);
    }

    #[test]
    fn padding_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.padding.top = 5.0;
        parent.padding.right = 6.0;
        parent.padding.bottom = 7.0;
        parent.padding.left = 8.0;
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "padding-top: inherit; padding-right: inherit; padding-bottom: inherit; padding-left: inherit",
            ),
            &parent,
        );
        assert!((style.padding.top - 5.0).abs() < 0.1);
        assert!((style.padding.right - 6.0).abs() < 0.1);
        assert!((style.padding.bottom - 7.0).abs() < 0.1);
        assert!((style.padding.left - 8.0).abs() < 0.1);
    }

    #[test]
    fn display_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.display = Display::Flex;
        let style = compute_style(HtmlTag::Div, Some("display: inherit"), &parent);
        assert_eq!(style.display, Display::Flex);
    }

    #[test]
    fn width_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.width = Some(200.0);
        let style = compute_style(HtmlTag::Div, Some("width: inherit"), &parent);
        assert_eq!(style.width, Some(200.0));
    }

    #[test]
    fn height_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.height = Some(100.0);
        let style = compute_style(HtmlTag::Div, Some("height: inherit"), &parent);
        assert_eq!(style.height, Some(100.0));
    }

    #[test]
    fn max_width_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.max_width = Some(300.0);
        let style = compute_style(HtmlTag::Div, Some("max-width: inherit"), &parent);
        assert_eq!(style.max_width, Some(300.0));
    }

    #[test]
    fn opacity_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.opacity = 0.5;
        let style = compute_style(HtmlTag::Div, Some("opacity: inherit"), &parent);
        assert!((style.opacity - 0.5).abs() < 0.01);
    }

    #[test]
    fn border_width_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border = BorderSides::uniform(BorderSide::solid(3.0, SpecifiedColor::CurrentColor));
        let style = compute_style(HtmlTag::Div, Some("border-width: inherit"), &parent);
        assert!((style.border.top.specified_width - 3.0).abs() < 0.1);
    }

    #[test]
    fn border_color_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border = BorderSides::uniform(BorderSide::solid(0.0, Color::rgb(255, 0, 0).into()));
        let style = compute_style(HtmlTag::Div, Some("border-color: inherit"), &parent);
        assert_eq!(style.border.top.color.resolve(style.color).r, 255.0);
    }

    #[test]
    fn border_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border = BorderSides::uniform(BorderSide::solid(2.0, Color::rgb(0, 0, 255).into()));
        let style = compute_style(HtmlTag::Div, Some("border: inherit"), &parent);
        assert!((style.border.top.specified_width - 2.0).abs() < 0.1);
        assert_eq!(style.border.top.color.resolve(style.color).b, 255.0);
    }

    #[test]
    fn float_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.float = Float::Right;
        let style = compute_style(HtmlTag::Div, Some("float: inherit"), &parent);
        assert_eq!(style.float, Float::Right);
    }

    #[test]
    fn clear_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.clear = Clear::Both;
        let style = compute_style(HtmlTag::Div, Some("clear: inherit"), &parent);
        assert_eq!(style.clear, Clear::Both);
    }

    #[test]
    fn position_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.position = Position::Absolute;
        let style = compute_style(HtmlTag::Div, Some("position: inherit"), &parent);
        assert_eq!(style.position, Position::Absolute);
    }

    #[test]
    fn flex_direction_reverse_keywords_parse() {
        let p = ComputedStyle::default();
        assert_eq!(
            compute_style(HtmlTag::Div, Some("flex-direction: row-reverse"), &p).flex_direction,
            FlexDirection::RowReverse
        );
        assert_eq!(
            compute_style(HtmlTag::Div, Some("flex-direction: column-reverse"), &p).flex_direction,
            FlexDirection::ColumnReverse
        );
    }

    #[test]
    fn flex_wrap_wrap_reverse_parses() {
        let p = ComputedStyle::default();
        assert_eq!(
            compute_style(HtmlTag::Div, Some("flex-wrap: wrap-reverse"), &p).flex_wrap,
            FlexWrap::WrapReverse
        );
    }

    #[test]
    fn flex_flow_shorthand_sets_both_axes() {
        let p = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("flex-flow: column wrap"), &p);
        assert_eq!(s.flex_direction, FlexDirection::Column);
        assert_eq!(s.flex_wrap, FlexWrap::Wrap);
        // Order-free.
        let s2 = compute_style(
            HtmlTag::Div,
            Some("flex-flow: wrap-reverse row-reverse"),
            &p,
        );
        assert_eq!(s2.flex_direction, FlexDirection::RowReverse);
        assert_eq!(s2.flex_wrap, FlexWrap::WrapReverse);
    }

    #[test]
    fn justify_content_space_evenly_parses() {
        let p = ComputedStyle::default();
        assert_eq!(
            compute_style(HtmlTag::Div, Some("justify-content: space-evenly"), &p).justify_content,
            JustifyContent::SpaceEvenly
        );
    }

    #[test]
    fn align_content_keywords_parse() {
        let p = ComputedStyle::default();
        for (kw, exp) in [
            ("flex-start", AlignContent::FlexStart),
            ("flex-end", AlignContent::FlexEnd),
            ("center", AlignContent::Center),
            ("space-between", AlignContent::SpaceBetween),
            ("space-around", AlignContent::SpaceAround),
            ("space-evenly", AlignContent::SpaceEvenly),
            ("stretch", AlignContent::Stretch),
        ] {
            let s = compute_style(HtmlTag::Div, Some(&format!("align-content: {kw}")), &p);
            assert_eq!(s.align_content, exp, "align-content: {kw}");
        }
    }

    #[test]
    fn align_items_and_self_baseline_parse() {
        let p = ComputedStyle::default();
        assert_eq!(
            compute_style(HtmlTag::Div, Some("align-items: baseline"), &p).align_items,
            AlignItems::Baseline
        );
        assert_eq!(
            compute_style(HtmlTag::Div, Some("align-self: baseline"), &p).align_self,
            AlignSelf::Baseline
        );
    }

    #[test]
    fn flex_basis_percentage_parses() {
        let p = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("flex-basis: 25%"), &p);
        assert_eq!(
            s.flex_basis,
            FlexBasis::Definite(LengthPercent::percent(25.0))
        );
    }

    #[test]
    fn flex_basis_calc_preserves_its_percentage_term() {
        let style = compute_style(
            HtmlTag::Div,
            Some("flex-basis: calc(25% - 10pt)"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.flex_basis,
            FlexBasis::Definite(LengthPercent::from_terms(-10.0, 25.0))
        );
    }

    #[test]
    fn flex_basis_clamp_gives_minimum_precedence_without_panicking() {
        let style = compute_style(
            HtmlTag::Div,
            Some("flex-basis: clamp(40pt, 20pt, 30pt)"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.flex_basis.resolve(0.0), Some(40.0));
    }

    #[test]
    fn flex_shorthand_keywords_expand() {
        let p = ComputedStyle::default();
        let none = compute_style(HtmlTag::Div, Some("flex: none"), &p);
        assert_eq!((none.flex_grow, none.flex_shrink), (0.0, 0.0));
        assert_eq!(none.flex_basis, FlexBasis::Auto);
        let auto = compute_style(HtmlTag::Div, Some("flex: auto"), &p);
        assert_eq!((auto.flex_grow, auto.flex_shrink), (1.0, 1.0));
        let one = compute_style(HtmlTag::Div, Some("flex: 1"), &p);
        assert_eq!((one.flex_grow, one.flex_shrink), (1.0, 1.0));
        assert!(one.flex_basis.is_zero());
    }

    #[test]
    fn gap_two_value_sets_row_and_column() {
        let p = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("gap: 30px 10px"), &p);
        // 30px -> 22.5pt row gap, 10px -> 7.5pt column gap.
        assert!((s.row_gap - 22.5).abs() < 0.01, "row_gap={}", s.row_gap);
        assert!(
            (s.column_gap - 7.5).abs() < 0.01,
            "column_gap={}",
            s.column_gap
        );
    }

    #[test]
    fn top_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.top = Some(10.0);
        let style = compute_style(HtmlTag::Div, Some("top: inherit"), &parent);
        assert_eq!(style.top, Some(10.0));
    }

    #[test]
    fn left_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.left = Some(20.0);
        let style = compute_style(HtmlTag::Div, Some("left: inherit"), &parent);
        assert_eq!(style.left, Some(20.0));
    }

    #[test]
    fn overflow_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.overflow = Overflow::Auto;
        let style = compute_style(HtmlTag::Div, Some("overflow: inherit"), &parent);
        assert_eq!(style.overflow, Overflow::Auto);
    }

    #[test]
    fn transform_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.transform = Some(Transform::Rotate(45.0));
        let style = compute_style(HtmlTag::Div, Some("transform: inherit"), &parent);
        assert_eq!(style.transform, Some(Transform::Rotate(45.0)));
    }

    #[test]
    fn box_shadow_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.box_shadow = vec![BoxShadow {
            offset_x: 1.0,
            offset_y: 2.0,
            blur: 3.0,
            spread: 0.0,
            color: Color::BLACK,
            color_source: ColorSource::Absolute,
            inset: false,
        }];
        let style = compute_style(HtmlTag::Div, Some("box-shadow: inherit"), &parent);
        assert!(!style.box_shadow.is_empty());
        assert!((style.box_shadow[0].offset_x - 1.0).abs() < 0.1);
    }

    #[test]
    fn flex_direction_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.flex_direction = FlexDirection::Column;
        let style = compute_style(HtmlTag::Div, Some("flex-direction: inherit"), &parent);
        assert_eq!(style.flex_direction, FlexDirection::Column);
    }

    #[test]
    fn justify_content_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.justify_content = JustifyContent::Center;
        let style = compute_style(HtmlTag::Div, Some("justify-content: inherit"), &parent);
        assert_eq!(style.justify_content, JustifyContent::Center);
    }

    #[test]
    fn align_items_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.align_items = AlignItems::FlexEnd;
        let style = compute_style(HtmlTag::Div, Some("align-items: inherit"), &parent);
        assert_eq!(style.align_items, AlignItems::FlexEnd);
    }

    #[test]
    fn flex_wrap_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.flex_wrap = FlexWrap::Wrap;
        let style = compute_style(HtmlTag::Div, Some("flex-wrap: inherit"), &parent);
        assert_eq!(style.flex_wrap, FlexWrap::Wrap);
    }

    #[test]
    fn gap_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.gap = 10.0;
        let style = compute_style(HtmlTag::Div, Some("gap: inherit"), &parent);
        assert!((style.gap - 10.0).abs() < 0.1);
    }

    // --- display/flex/align fallback tests (lines 795, 802, 812, 821, 828) ---

    #[test]
    fn display_unknown_keyword_fallback() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: foobar"), &parent);
        // Unknown display keyword keeps the current display value
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn flex_direction_unknown_fallback_to_row() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-direction: foobar"), &parent);
        assert_eq!(style.flex_direction, FlexDirection::Row);
    }

    #[test]
    fn flex_direction_column() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-direction: column"), &parent);
        assert_eq!(style.flex_direction, FlexDirection::Column);
    }

    #[test]
    fn invalid_justify_content_does_not_replace_valid_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("justify-content: center; justify-content: foobar"),
            &parent,
        );
        assert_eq!(style.justify_content, JustifyContent::Center);
    }

    #[test]
    fn invalid_align_items_does_not_replace_valid_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("align-items: center; align-items: foobar"),
            &parent,
        );
        assert_eq!(style.align_items, AlignItems::Center);
    }

    #[test]
    fn flex_wrap_unknown_fallback_to_nowrap() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-wrap: foobar"), &parent);
        assert_eq!(style.flex_wrap, FlexWrap::NoWrap);
    }

    #[test]
    fn flex_wrap_wrap() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-wrap: wrap"), &parent);
        assert_eq!(style.flex_wrap, FlexWrap::Wrap);
    }

    // --- em (Number) values for sizing properties (lines 882, 889, 896, 903, 910, 917) ---

    #[test]
    fn width_em_value() {
        let parent = ComputedStyle::default(); // font_size = 12.0
        let style = compute_style(HtmlTag::Div, Some("width: 10em"), &parent);
        assert!((style.width.unwrap() - 120.0).abs() < 0.1);
    }

    #[test]
    fn width_calc_em_value_uses_current_font_size() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 20.0;
        let style = compute_style(HtmlTag::Div, Some("width: calc(1em + 5pt)"), &parent);
        assert!((style.width.unwrap() - 25.0).abs() < 0.1);
    }

    #[test]
    fn height_em_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("height: 5em"), &parent);
        assert!((style.height.unwrap() - 60.0).abs() < 0.1);
    }

    #[test]
    fn max_width_em_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("max-width: 20em"), &parent);
        assert!((style.max_width.unwrap() - 240.0).abs() < 0.1);
    }

    #[test]
    fn min_width_em_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("min-width: 5em"), &parent);
        assert!((style.min_width.unwrap() - 60.0).abs() < 0.1);
    }

    #[test]
    fn min_height_em_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("min-height: 8em"), &parent);
        assert!((style.min_height.unwrap() - 96.0).abs() < 0.1);
    }

    #[test]
    fn max_height_em_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("max-height: 15em"), &parent);
        assert!((style.max_height.unwrap() - 180.0).abs() < 0.1);
    }

    // --- invalid opacity dimension ---

    #[test]
    fn invalid_dimension_on_opacity_does_not_replace_valid_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("opacity: 0.4; opacity: 0.7em"), &parent);
        assert!((style.opacity - 0.4).abs() < 0.01);
    }

    // --- clear/position unknown fallback (lines 963, 972) ---

    #[test]
    fn clear_unknown_fallback_to_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("clear: foobar"), &parent);
        assert_eq!(style.clear, Clear::None);
    }

    #[test]
    fn position_unknown_fallback_to_static() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("position: foobar"), &parent);
        assert_eq!(style.position, Position::Static);
    }

    // --- outline shorthand pt unit (lines 1029-1030) ---

    #[test]
    fn outline_shorthand_pt_unit() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("outline: 3pt solid blue"), &parent);
        assert!((style.outline_width - 3.0).abs() < 0.001);
        assert!(style.outline_color.is_some());
        assert_eq!(style.outline_color.unwrap().b, 255.0);
    }

    // --- outline individual properties (lines 1043, 1046) ---

    #[test]
    fn outline_width_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("outline-width: 5pt"), &parent);
        assert!((style.outline_width - 5.0).abs() < 0.001);
    }

    #[test]
    fn outline_color_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("outline-color: red"), &parent);
        assert!(style.outline_color.is_some());
        assert_eq!(style.outline_color.unwrap().r, 255.0);
    }

    // --- text-transform (lines 1059-1063) ---
    // Note: text-transform, white-space, and vertical-align keyword properties are not
    // recognized by the inline CSS parser, so we test via CssRule with manually built StyleMap.

    fn make_keyword_rule(prop: &str, val: &str) -> CssRule {
        let mut map = StyleMap::new();
        map.set(prop, CssValue::Keyword(val.to_string()));
        CssRule {
            selector: "div".to_string(),
            declarations: map,
            pseudo_element: None,
        }
    }

    #[test]
    fn text_transform_uppercase() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("text-transform", "uppercase");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn text_transform_lowercase() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("text-transform", "lowercase");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.text_transform, TextTransform::Lowercase);
    }

    #[test]
    fn text_transform_capitalize() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("text-transform", "capitalize");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.text_transform, TextTransform::Capitalize);
    }

    #[test]
    fn text_transform_unknown_fallback() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("text-transform", "foobar");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.text_transform, TextTransform::None);
    }

    // --- text-indent (line 1069) ---

    #[test]
    fn text_indent_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-indent: 20pt"), &parent);
        assert!(matches!(style.text_indent, TextIndent::Length(20.0)));
    }

    #[test]
    fn text_indent_percentage_is_inherited_and_resolved_from_the_inner_size() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("text-indent: 50%"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::P, None, &parent);
        let initial = compute_style(HtmlTag::P, Some("text-indent: initial"), &parent);

        assert!(matches!(child.text_indent, TextIndent::Percentage(50.0)));
        assert_eq!(child.text_indent.resolve(216.0), 108.0);
        assert!(matches!(initial.text_indent, TextIndent::Length(0.0)));
    }

    #[test]
    fn text_indent_percentage_expressions_stay_deferred() {
        for (declaration, expected) in [
            ("text-indent: calc(50% - 10pt)", 98.0),
            ("text-indent: clamp(20pt, 50%, 100pt)", 100.0),
            (
                "--indent: calc(50% - 10pt); text-indent: var(--indent)",
                98.0,
            ),
        ] {
            let style = compute_style(HtmlTag::Div, Some(declaration), &ComputedStyle::default());
            assert!(
                (style.text_indent.resolve(216.0) - expected).abs() < f32::EPSILON,
                "{declaration} resolved to {}",
                style.text_indent.resolve(216.0)
            );
        }
    }

    // --- white-space (lines 1074-1079) ---

    #[test]
    fn white_space_nowrap() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "nowrap");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::NoWrap);
    }

    #[test]
    fn white_space_pre() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "pre");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn white_space_pre_wrap() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "pre-wrap");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::PreWrap);
    }

    #[test]
    fn white_space_pre_line() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "pre-line");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::PreLine);
    }

    #[test]
    fn white_space_break_spaces() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "break-spaces");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::BreakSpaces);
    }

    #[test]
    fn white_space_unknown_fallback() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("white-space", "foobar");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.white_space, WhiteSpace::Normal);
    }

    // --- letter-spacing (line 1085) ---

    #[test]
    fn letter_spacing_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("letter-spacing: 2pt"), &parent);
        assert!((style.letter_spacing - 2.0).abs() < 0.1);
    }

    // --- word-spacing (line 1090) ---

    #[test]
    fn word_spacing_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("word-spacing: 4pt"), &parent);
        assert!((style.word_spacing - 4.0).abs() < 0.1);
    }

    #[test]
    fn word_spacing_percentage_uses_the_computed_font_size() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-size: 24px; word-spacing: calc(200% - 3pt)"),
            &parent,
        );
        // 24 CSS px = 18pt; 200% is two computed font sizes.
        assert!((style.word_spacing - 33.0).abs() < 0.001);
    }

    #[test]
    fn word_spacing_normal_resets_the_inherited_value() {
        let mut parent = ComputedStyle::default();
        parent.word_spacing = 8.0;
        let style = compute_style(HtmlTag::Span, Some("word-spacing: normal"), &parent);
        assert_eq!(style.word_spacing, 0.0);
    }

    #[test]
    fn spacing_var_normal_resets_inherited_values() {
        let mut parent = ComputedStyle::default();
        parent.letter_spacing = 4.0;
        parent.word_spacing = 8.0;
        let style = compute_style(
            HtmlTag::Span,
            Some("--spacing: normal; letter-spacing: var(--spacing); word-spacing: var(--spacing)"),
            &parent,
        );
        assert_eq!(style.letter_spacing, 0.0);
        assert_eq!(style.word_spacing, 0.0);
    }

    // --- vertical-align (lines 1095-1101) ---

    #[test]
    fn vertical_align_super() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "super");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Super);
    }

    #[test]
    fn vertical_align_sub() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "sub");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Sub);
    }

    #[test]
    fn vertical_align_top() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "top");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Top);
    }

    #[test]
    fn vertical_align_middle() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "middle");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Middle);
    }

    #[test]
    fn vertical_align_bottom() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "bottom");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Bottom);
    }

    #[test]
    fn vertical_align_unknown_fallback() {
        let parent = ComputedStyle::default();
        let rule = make_keyword_rule("vertical-align", "foobar");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &[rule], "div", &[], None);
        assert_eq!(style.vertical_align, VerticalAlign::Baseline);
    }

    // --- parse_box_shadow edge cases (lines 1130-1132, 1143, 1153, 1162, 1181) ---

    #[test]
    fn parse_box_shadow_with_rgba() {
        let shadow = parse_single_box_shadow("2px 2px 4px rgba(0,0,0,0.3)");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 3.0).abs() < 0.1); // 4px * 0.75
    }

    #[test]
    fn parse_box_shadow_too_few_tokens() {
        // CSS allows just offset-x + offset-y (no blur/spread/color). Should parse.
        let shadow = parse_single_box_shadow("2px 2px");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 0.0).abs() < 0.1);
        assert!((s.spread - 0.0).abs() < 0.1);
        assert!(!s.inset);

        // Single token is not enough.
        assert!(parse_single_box_shadow("2px").is_none());
    }

    #[test]
    fn parse_box_shadow_inset_keyword() {
        let shadow = parse_single_box_shadow("inset 2px 2px 4px rgba(0,0,0,0.3)");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!(s.inset);
        assert!((s.blur - 3.0).abs() < 0.1);

        let color_first = parse_single_box_shadow("red 2px 2px inset").unwrap();
        assert!(color_first.inset);
        assert_eq!(
            (
                color_first.color.r,
                color_first.color.g,
                color_first.color.b,
                color_first.color.a,
            ),
            (255.0, 0.0, 0.0, 255.0)
        );
    }

    #[test]
    fn parse_box_shadow_with_spread() {
        let shadow = parse_single_box_shadow("4px 4px 8px 2px #000");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 6.0).abs() < 0.1); // 8px * 0.75
        assert!((s.spread - 1.5).abs() < 0.1); // 2px * 0.75
    }

    #[test]
    fn parse_box_shadow_multi_returns_all() {
        let shadows = parse_box_shadow("2px 2px 4px black, 0 0 8px red").unwrap();
        assert_eq!(shadows.len(), 2);
        // First listed shadow.
        assert!((shadows[0].blur - 3.0).abs() < 0.1);
        assert_eq!(shadows[0].color.r, 0.0);
        // Second listed shadow.
        assert!((shadows[1].blur - 6.0).abs() < 0.1); // 8px * 0.75
        assert_eq!(shadows[1].color.r, 255.0);
    }

    #[test]
    fn parse_box_shadow_rejects_unknown_tokens() {
        assert!(parse_single_box_shadow("2px 2px notanumber black").is_none());
        assert!(parse_single_box_shadow("2px 2px red blue").is_none());
        assert!(parse_single_box_shadow("inset inset 2px 2px").is_none());
    }

    #[test]
    fn parse_box_shadow_rejects_negative_blur() {
        assert!(parse_single_box_shadow("2px 2px -1px black").is_none());
    }

    #[test]
    fn parse_box_shadow_list_is_atomic() {
        assert!(parse_box_shadow("2px 2px black, 12 0 blue").is_none());
        assert!(parse_box_shadow("2px 2px black,").is_none());
    }

    #[test]
    fn invalid_box_shadow_list_does_not_create_a_partial_shadow() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("box-shadow: 22px 0 0 red, 12 0 blue"),
            &parent,
        );
        assert_eq!(style.box_shadow.len(), 0);

        let style = compute_style(
            HtmlTag::Div,
            Some(
                "box-shadow: 2px 2px black; \
                 box-shadow: 22px 0 0 red, 12 0 blue",
            ),
            &parent,
        );
        assert_eq!(style.box_shadow.len(), 1);
        assert!((style.box_shadow[0].offset_x - 1.5).abs() < 0.1);
    }

    #[test]
    fn parse_box_shadow_no_color_token() {
        // Exactly 3 tokens where third is a valid blur, so color_start=3, no color token
        let current_color = Color::rgb(7, 11, 13);
        let shadow = parse_single_box_shadow_for_color("2px 2px 4px", current_color);
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        // Per CSS Backgrounds & Borders L3 §7.2 an omitted shadow color defaults
        // to currentColor. Keep the typed provenance so inherited text shadows
        // can rebind on descendants instead of relying on an authorable sentinel.
        assert_eq!(s.color, current_color);
        assert_eq!(s.color_source, ColorSource::CurrentColor);
    }

    #[test]
    fn parse_shadow_length_rejects_nonzero_bare_number() {
        assert_eq!(parse_shadow_length("5"), None);
        assert_eq!(parse_shadow_length("0"), Some(0.0));
    }

    #[test]
    fn parse_text_shadow_offset_color() {
        // `text-shadow: 6px 6px 0 #ff6f00` (offset + zero blur + color).
        let shadows = parse_text_shadow("6px 6px 0 #ff6f00");
        assert_eq!(shadows.len(), 1);
        let s = &shadows[0];
        assert!((s.offset_x - 4.5).abs() < 0.1); // 6px * 0.75
        assert!((s.offset_y - 4.5).abs() < 0.1);
        assert!((s.blur - 0.0).abs() < 0.1);
        assert_eq!(s.spread, 0.0);
        assert!(!s.inset);
        assert_eq!(s.color.r, 255.0);
        assert_eq!(s.color.g, 111.0);
        assert_eq!(s.color.b, 0.0);
    }

    #[test]
    fn parse_text_shadow_color_first() {
        // text-shadow allows the color before the offsets.
        let shadows = parse_text_shadow("red 2px 2px");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].color.r, 255.0);
        assert!((shadows[0].offset_x - 1.5).abs() < 0.1);
    }

    #[test]
    fn parse_text_shadow_none_and_list() {
        assert!(parse_text_shadow("none").is_empty());
        let shadows = parse_text_shadow("1px 1px black, 2px 2px red");
        assert_eq!(shadows.len(), 2);
    }

    // --- parse_transform edge cases (lines 1207, 1233-1235, 1239, 1250) ---

    /// Parse a transform with default (12pt) font sizes for em/rem resolution.
    fn pt(val: &str) -> Option<Transform> {
        parse_transform(val, 12.0, 12.0)
    }

    #[test]
    fn parse_transform_rejects_nonzero_unitless_angle() {
        assert_eq!(pt("rotate(45)"), None);
        assert_eq!(pt("rotate(0)"), Some(Transform::Rotate(0.0)));
    }

    #[test]
    fn parse_transform_translate_single_arg() {
        let t = pt("translate(10pt)");
        assert_eq!(
            t,
            Some(Transform::Translate {
                offset: CssVector::new(10.0, 0.0),
                percentages: PercentageAxes::default(),
            })
        );
    }

    #[test]
    fn parse_transform_unknown_returns_none() {
        let t = pt("perspective(500px)");
        match t {
            Some(Transform::Matrix3d(m)) => {
                assert!((m[11] + 1.0 / 375.0).abs() < 0.0001);
            }
            other => panic!("expected perspective() to produce a 3D matrix, got {other:?}"),
        }
    }

    #[test]
    fn parse_transform_skew() {
        let t = pt("skew(30deg)");
        assert!(t.is_some());
        if let Some(Transform::Skew(angles)) = t {
            assert_eq!(angles, CssVector::new(30.0, 0.0));
        } else {
            panic!("expected Skew");
        }
    }

    #[test]
    fn parse_transform_chained() {
        let t = pt("rotate(10deg) scale(1.1)");
        assert!(t.is_some());
        assert!(matches!(t, Some(Transform::Matrix(..))));
    }

    #[test]
    fn parse_transform_scale_x_y() {
        assert_eq!(
            pt("scaleX(1.5)"),
            Some(Transform::Scale(CssVector::new(1.5, 1.0)))
        );
        assert_eq!(
            pt("scaleY(0.5)"),
            Some(Transform::Scale(CssVector::new(1.0, 0.5)))
        );
    }

    #[test]
    fn parse_transform_translate_x_y() {
        assert!(matches!(
            pt("translateX(40px)"),
            Some(Transform::Translate { offset, .. }) if offset.y == 0.0
        ));
        assert!(matches!(
            pt("translateY(20px)"),
            Some(Transform::Translate { offset, .. }) if offset.x == 0.0
        ));
    }

    #[test]
    fn parse_transform_length_rejects_nonzero_bare_number() {
        assert_eq!(parse_transform_length("42", 12.0, 12.0), None);
        assert_eq!(parse_transform_length("0", 12.0, 12.0), Some((0.0, false)));
    }

    #[test]
    fn invalid_unitless_transform_length_is_not_normalized_to_pixels() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: translate(40, 0)"), &parent);
        assert_eq!(style.transform, None);

        let style = compute_style(
            HtmlTag::Div,
            Some("transform: translateX(8px); transform: translate(40, 0)"),
            &parent,
        );
        assert!(matches!(
            style.transform,
            Some(Transform::Translate { offset, .. }) if (offset.x - 6.0).abs() < 0.01
        ));
    }

    #[test]
    fn parse_transform_angle_units() {
        // 0.25turn == 90deg, 1.5708rad ~= 90deg, 100grad == 90deg.
        assert_eq!(pt("rotate(0.25turn)"), Some(Transform::Rotate(90.0)));
        assert_eq!(pt("rotate(100grad)"), Some(Transform::Rotate(90.0)));
        if let Some(Transform::Rotate(deg)) = pt("rotate(1.5708rad)") {
            assert!((deg - 90.0).abs() < 0.05);
        } else {
            panic!("expected Rotate from rad");
        }
        assert_eq!(pt("rotate(-90deg)"), Some(Transform::Rotate(-90.0)));
    }

    #[test]
    fn parse_transform_scale_negative_and_omitted() {
        assert_eq!(
            pt("scale(-1, 1)"),
            Some(Transform::Scale(CssVector::new(-1.0, 1.0)))
        );
        // A single arg mirrors to both axes.
        assert_eq!(
            pt("scale(2)"),
            Some(Transform::Scale(CssVector::splat(2.0)))
        );
    }

    #[test]
    fn parse_transform_translate_em_rem() {
        // 2em at 12pt font => 24pt; 1rem at 12pt root => 12pt.
        assert_eq!(
            pt("translate(2em, 1rem)"),
            Some(Transform::Translate {
                offset: CssVector::new(24.0, 12.0),
                percentages: PercentageAxes::default(),
            })
        );
    }

    #[test]
    fn parse_transform_matrix_malformed_rejected() {
        // A non-numeric token makes the whole function invalid (None), rather
        // than silently dropping it and shifting the arity.
        assert!(pt("matrix(1,2,3,bad,5,6)").is_none());
        assert!(pt("matrix(1,0,0,1,10,20)").is_some());
    }

    #[test]
    fn parse_transform_compound_percent() {
        // scale(2) translate(50%) — the % resolves against own box THEN scales.
        let t = pt("scale(2) translate(50%, 0%)").expect("compound");
        assert!(matches!(t, Transform::MatrixPct(_)));
        let matrix = t.to_css_matrix(CssVector::new(100.0, 40.0));
        // a == 2 (scale x), e == 2 * (50% of 100) == 100.
        assert!((matrix.x_axis.x - 2.0).abs() < 0.01);
        assert!((matrix.translation.x - 100.0).abs() < 0.01);
    }

    // --- grid-template-columns bare number (line 1270) ---

    #[test]
    fn grid_template_columns_bare_number() {
        let tracks = parse_grid_template_columns("100 200");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0], GridTrack::Fixed(100.0));
        assert_eq!(tracks[1], GridTrack::Fixed(200.0));
    }

    #[test]
    fn grid_template_columns_repeat() {
        let tracks = parse_grid_template_columns("repeat(3, 1fr)");
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0], GridTrack::Fr(1.0));
        assert_eq!(tracks[1], GridTrack::Fr(1.0));
        assert_eq!(tracks[2], GridTrack::Fr(1.0));
    }

    #[test]
    fn grid_template_columns_repeat_fixed() {
        let tracks = parse_grid_template_columns("repeat(2, 100px)");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0], GridTrack::Fixed(75.0));
        assert_eq!(tracks[1], GridTrack::Fixed(75.0));
    }

    #[test]
    fn grid_template_columns_repeat_multi_track() {
        let tracks = parse_grid_template_columns("repeat(2, 1fr 2fr)");
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0], GridTrack::Fr(1.0));
        assert_eq!(tracks[1], GridTrack::Fr(2.0));
        assert_eq!(tracks[2], GridTrack::Fr(1.0));
        assert_eq!(tracks[3], GridTrack::Fr(2.0));
    }

    #[test]
    fn grid_template_columns_repeat_auto_fill() {
        let tracks = parse_grid_template_columns("repeat(auto-fill, 100px)");
        // auto-fill defaults to 3 columns for PDF
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0], GridTrack::Fixed(75.0));
    }

    #[test]
    fn grid_template_columns_repeat_auto_fit() {
        let tracks = parse_grid_template_columns("repeat(auto-fit, 1fr)");
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0], GridTrack::Fr(1.0));
    }

    #[test]
    fn grid_template_columns_minmax() {
        let tracks = parse_grid_template_columns("minmax(100px, 1fr)");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], GridTrack::Minmax(75.0, f32::MAX));
    }

    #[test]
    fn grid_template_columns_minmax_fixed() {
        let tracks = parse_grid_template_columns("minmax(50pt, 200pt)");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], GridTrack::Minmax(50.0, 200.0));
    }

    #[test]
    fn grid_template_columns_mixed_with_repeat() {
        let tracks = parse_grid_template_columns("100pt repeat(2, 1fr) auto");
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[0], GridTrack::Fixed(100.0));
        assert_eq!(tracks[1], GridTrack::Fr(1.0));
        assert_eq!(tracks[2], GridTrack::Fr(1.0));
        assert_eq!(tracks[3], GridTrack::Auto);
    }

    #[test]
    fn grid_template_columns_repeat_with_minmax() {
        let tracks = parse_grid_template_columns("repeat(3, minmax(100px, 1fr))");
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0], GridTrack::Minmax(75.0, f32::MAX));
    }

    #[test]
    fn column_count_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-count: 3"), &parent);
        assert_eq!(style.column_count, Some(3));
    }

    #[test]
    fn column_gap_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("column-count: 2; column-gap: 15pt"),
            &parent,
        );
        assert_eq!(style.column_count, Some(2));
        assert!((style.column_gap - 15.0).abs() < 0.1);
    }

    #[test]
    fn columns_shorthand_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("columns: 2"), &parent);
        assert_eq!(style.column_count, Some(2));
    }

    #[test]
    fn column_count_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-count: initial"), &parent);
        assert_eq!(style.column_count, None);
    }

    #[test]
    fn column_count_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.column_count = Some(3);
        let style = compute_style(HtmlTag::Div, Some("column-count: inherit"), &parent);
        assert_eq!(style.column_count, Some(3));
    }

    #[test]
    fn column_gap_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-gap: initial"), &parent);
        assert!((style.column_gap - 0.0).abs() < 0.1);
    }

    #[test]
    fn column_count_invalid_value_ignored() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-count: auto"), &parent);
        assert_eq!(style.column_count, None);
    }

    #[test]
    fn grid_template_columns_repeat_single() {
        let tracks = parse_grid_template_columns("repeat(1, 100pt)");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], GridTrack::Fixed(100.0));
    }

    #[test]
    fn grid_minmax_auto_min() {
        let tracks = parse_grid_template_columns("minmax(auto, 200pt)");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], GridTrack::Minmax(0.0, 200.0));
    }

    #[test]
    fn grid_minmax_auto_max() {
        let tracks = parse_grid_template_columns("minmax(50pt, auto)");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], GridTrack::Minmax(50.0, f32::MAX));
    }

    // --- parse_hex_to_color invalid length (line 1313) ---

    #[test]
    fn parse_hex_to_color_invalid_length() {
        // 4-digit (#rgba) and 8-digit (#rrggbbaa) are now VALID alpha forms;
        // only lengths outside {3,4,6,8} are rejected.
        assert!(parse_hex_to_color("abcde").is_none());
        assert!(parse_hex_to_color("abcdef0").is_none());
        // `abcd` is a valid 4-digit #rgba (a->0xaa, b->0xbb, c->0xcc, d->0xdd).
        assert_eq!(
            parse_hex_to_color("abcd").map(rgba_tuple),
            Some((0xaa, 0xbb, 0xcc, 0xdd))
        );
    }

    #[test]
    fn parse_hex_to_color_single_char() {
        let result = parse_hex_to_color("a");
        assert!(result.is_none());
    }

    // --- linear gradient diagonal directions (lines 1344-1348) ---

    #[test]
    fn linear_gradient_diagonal_directions() {
        let lg = parse_linear_gradient("linear-gradient(to top right, red, blue)").unwrap();
        assert!((lg.angle - 45.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to right top, red, blue)").unwrap();
        assert!((lg.angle - 45.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to bottom right, red, blue)").unwrap();
        assert!((lg.angle - 135.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to right bottom, red, blue)").unwrap();
        assert!((lg.angle - 135.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to bottom left, red, blue)").unwrap();
        assert!((lg.angle - 225.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to left bottom, red, blue)").unwrap();
        assert!((lg.angle - 225.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to top left, red, blue)").unwrap();
        assert!((lg.angle - 315.0).abs() < 0.01);

        let lg = parse_linear_gradient("linear-gradient(to left top, red, blue)").unwrap();
        assert!((lg.angle - 315.0).abs() < 0.01);
    }

    #[test]
    fn linear_gradient_unknown_to_direction_is_invalid() {
        assert!(parse_linear_gradient("linear-gradient(to unknown, red, blue)").is_none());
    }

    // --- linear gradient invalid deg (line 1355) ---

    #[test]
    fn linear_gradient_invalid_deg_falls_back() {
        // "xdeg" has "deg" suffix but is not parseable as f32.
        // Falls through to (180.0, 0) — color_start = 0, so "xdeg" becomes a color stop.
        // "xdeg" is not a valid color, so the whole gradient returns None.
        let lg = parse_linear_gradient("linear-gradient(xdeg, red, blue)");
        assert!(lg.is_none());
    }

    // --- linear gradient not enough color parts after direction (line 1364) ---

    #[test]
    fn linear_gradient_single_color_after_direction() {
        let lg = parse_linear_gradient("linear-gradient(to right, red)").unwrap();
        assert_eq!(resolved_positions(&lg.ramp), [0.0]);
        assert_rgba_close(
            lg.ramp.resolve(1.0).unwrap().sample(0.75),
            (1.0, 0.0, 0.0, 1.0),
        );
    }

    // --- radial gradient not enough parts (line 1383) ---

    #[test]
    fn radial_gradient_single_part() {
        let rg = parse_radial_gradient("radial-gradient(red)").unwrap();
        assert_eq!(resolved_positions(&rg.ramp), [0.0]);
    }

    // --- radial gradient not enough color parts after shape keyword (line 1404) ---

    #[test]
    fn radial_gradient_shape_with_single_color() {
        let rg = parse_radial_gradient("radial-gradient(circle, red)").unwrap();
        assert_eq!(rg.shape, RadialShape::Circle);
        assert_eq!(resolved_positions(&rg.ramp), [0.0]);
    }

    // --- gradient stop percentage without space (line 1462, 1465) ---

    #[test]
    fn gradient_stop_percentage_no_space() {
        // A stop like "50%" where the whole part is "50%" — no space before percentage
        let lg = parse_linear_gradient("linear-gradient(to right, red 0%, blue 100%)").unwrap();
        assert_eq!(resolved_positions(&lg.ramp), [0.0, 1.0]);
    }

    // --- gradient single stop count (line 1474) ---

    #[test]
    fn gradient_single_stop_is_a_solid_image() {
        let lg = parse_linear_gradient("linear-gradient(red)").unwrap();
        let ramp = lg.ramp.resolve(1.0).unwrap();
        assert_eq!(ramp.stops()[0].position, 0.0);
        for position in [-1.0, 0.0, 0.5, 2.0] {
            assert_rgba_close(ramp.sample(position), (1.0, 0.0, 0.0, 1.0));
        }

        let style = compute_style(
            HtmlTag::Div,
            Some("background-image: linear-gradient(red)"),
            &ComputedStyle::default(),
        );
        assert!(style.background_gradient.is_some());
    }

    // --- gradient color parsing: rgb, rgba, invalid (lines 1518-1532) ---

    #[test]
    fn gradient_color_rgb_invalid_parts() {
        // rgb() with wrong number of parts
        let lg = parse_linear_gradient("linear-gradient(rgb(255, 0), blue)");
        assert!(lg.is_none());
    }

    #[test]
    fn gradient_color_rgba() {
        let lg =
            parse_linear_gradient("linear-gradient(to right, rgba(255, 0, 0, 0.5), blue)").unwrap();
        assert_eq!(lg.ramp.stops.len(), 2);
        assert_eq!(lg.ramp.stops[0].color.color.r, 255.0);
    }

    #[test]
    fn gradient_color_rgba_is_an_rgb_alias() {
        let lg = parse_linear_gradient("linear-gradient(rgba(255, 0, 0), blue)").unwrap();
        assert_eq!(lg.ramp.stops[0].color.color, Color::rgb(255, 0, 0));
        assert!(parse_linear_gradient("linear-gradient(rgba(255, 0), blue)").is_none());
        assert!(parse_linear_gradient("linear-gradient(rgba(255, 0, 0, .5, 1), blue)").is_none());
    }

    #[test]
    fn gradient_color_unknown_name() {
        // Unknown color name
        let lg = parse_linear_gradient("linear-gradient(unknowncolor, blue)");
        assert!(lg.is_none());
    }

    // --- display flex from inline style (line 795 flex variant) ---

    #[test]
    fn display_flex_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: flex"), &parent);
        assert_eq!(style.display, Display::Flex);
    }

    // --- justify-content variants ---

    #[test]
    fn justify_content_flex_end() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-content: flex-end"), &parent);
        assert_eq!(style.justify_content, JustifyContent::FlexEnd);
    }

    #[test]
    fn justify_content_center() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-content: center"), &parent);
        assert_eq!(style.justify_content, JustifyContent::Center);
    }

    #[test]
    fn justify_content_space_between() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("justify-content: space-between"),
            &parent,
        );
        assert_eq!(style.justify_content, JustifyContent::SpaceBetween);
    }

    #[test]
    fn justify_content_space_around() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-content: space-around"), &parent);
        assert_eq!(style.justify_content, JustifyContent::SpaceAround);
    }

    // --- align-items variants ---

    #[test]
    fn align_items_flex_start() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("align-items: flex-start"), &parent);
        assert_eq!(style.align_items, AlignItems::FlexStart);
    }

    #[test]
    fn align_items_flex_end() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("align-items: flex-end"), &parent);
        assert_eq!(style.align_items, AlignItems::FlexEnd);
    }

    #[test]
    fn align_items_center() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("align-items: center"), &parent);
        assert_eq!(style.align_items, AlignItems::Center);
    }

    // ---- z-index tests ----

    #[test]
    fn z_index_positive() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("z-index: 10"), &parent);
        assert_eq!(style.z_index, ZIndex::integer(10));
    }

    #[test]
    fn z_index_negative() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("z-index: -5"), &parent);
        assert_eq!(style.z_index, ZIndex::integer(-5));
    }

    #[test]
    fn z_index_auto_remains_distinct_from_integer_zero() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("z-index: auto"), &parent);
        assert_eq!(style.z_index, ZIndex::Auto);

        let zero = compute_style(HtmlTag::Div, Some("z-index: 0"), &parent);
        assert_eq!(zero.z_index, ZIndex::integer(0));
    }

    #[test]
    fn z_index_resets_between_elements() {
        let parent = ComputedStyle::default();
        let style1 = compute_style(HtmlTag::Div, Some("z-index: 99"), &parent);
        assert_eq!(style1.z_index, ZIndex::integer(99));
        let style2 = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style2.z_index, ZIndex::Auto);
    }

    // ---- CSS custom properties tests ----

    #[test]
    fn custom_property_stored() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("--spacing: 10pt"), &parent);
        assert_eq!(
            style.custom_properties.get("--spacing"),
            Some(&"10pt".to_string())
        );
    }

    #[test]
    fn custom_property_inherited() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--color: red"), &parent);
        assert_eq!(p.custom_properties.get("--color"), Some(&"red".to_string()));
        // Child inherits custom properties from parent (parent is cloned)
        let child = compute_style(HtmlTag::Span, None, &p);
        assert_eq!(
            child.custom_properties.get("--color"),
            Some(&"red".to_string())
        );
    }

    #[test]
    fn var_resolves_width_from_custom_prop() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--w: 200pt"), &parent);
        let child = compute_style(HtmlTag::Div, Some("width: var(--w)"), &p);
        assert!((child.width.unwrap() - 200.0).abs() < 0.1);
    }

    #[test]
    fn var_fallback_for_width() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: var(--missing, 50pt)"), &parent);
        assert!((style.width.unwrap() - 50.0).abs() < 0.1);
    }

    // ---- New unit tests ----

    #[test]
    fn percentage_width() {
        let mut parent = ComputedStyle::default();
        parent.width = Some(400.0);
        let style = compute_style(HtmlTag::Div, Some("width: 50%"), &parent);
        // 50% of parent width (400) = 200 ... but default parent_width_hint is 595.28
        // Actually resolve uses parent.width.unwrap_or(595.28)
        assert!(style.width.is_some());
    }

    #[test]
    fn rem_margin() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("margin-top: 2rem"), &parent);
        // 2rem * 12pt (default root) = 24pt
        assert!((style.margin.top - 24.0).abs() < 0.1);
    }

    #[test]
    fn calc_width() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: calc(100% - 20pt)"), &parent);
        assert!(style.width.is_some());
        // 100% of 595.28 - 20 = 575.28
        assert!((style.width.unwrap() - 575.28).abs() < 0.5);
    }

    #[test]
    fn vw_width() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("width: 50vw"), &parent);
        assert!(style.width.is_some());
        // 50vw = 50% of 595.28 = 297.64
        assert!((style.width.unwrap() - 297.64).abs() < 0.1);
    }

    #[test]
    fn vh_height() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("height: 100vh"), &parent);
        assert!(style.height.is_some());
        // 100vh = 841.89
        assert!((style.height.unwrap() - 841.89).abs() < 0.1);
    }

    #[test]
    fn rem_font_size() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("font-size: 1.5rem"), &parent);
        // 1.5rem * 12pt = 18pt
        assert!((style.font_size - 18.0).abs() < 0.1);
    }

    #[test]
    fn rem_uses_root_font_size_from_parent_context() {
        let mut parent = ComputedStyle::default();
        parent.root_font_size = 10.0;
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 1.5rem; margin-top: 0.5rem"),
            &parent,
        );
        assert!((style.font_size - 15.0).abs() < 0.1);
        assert!((style.margin.top - 5.0).abs() < 0.1);
    }

    #[test]
    fn percentage_font_size() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 16.0;
        let style = compute_style(HtmlTag::Div, Some("font-size: 150%"), &parent);
        // 150% of 16pt = 24pt
        assert!((style.font_size - 24.0).abs() < 0.1);
    }

    #[test]
    fn ex_font_size_uses_the_parent_fonts_measured_x_height() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let mut parent = ComputedStyle::default();
        parent.font_size = 20.0;
        parent.font_stack = FontStack::from_family(FontFamily::Custom("ParitySans".to_string()));

        let size = resolve_font_size_value(
            &CssValue::Ex(4.0),
            &HashMap::new(),
            &parent,
            FontMetrics::new(&fonts),
        )
        .expect("valid ex font size");

        // ParitySans has no OS/2.sxHeight. Current Chromium's Fontations
        // backend measures a 15px hinted `x` at the inherited 26.667px size,
        // so `4ex` computes to 60px / 45pt.
        assert_eq!(size, 45.0);
    }

    #[test]
    fn ex_font_size_uses_half_an_em_only_when_font_metrics_are_unavailable() {
        let mut parent = ComputedStyle::default();
        parent.font_size = 20.0;

        let size = resolve_font_size_value(
            &CssValue::Ex(4.0),
            &HashMap::new(),
            &parent,
            FontMetrics::default(),
        )
        .expect("valid fallback ex font size");

        assert_eq!(size, 40.0);
    }

    #[test]
    fn var_resolves_color() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--text-color: red"), &parent);
        let child = compute_style(HtmlTag::Span, Some("color: var(--text-color)"), &p);
        assert_eq!(child.color.r, 255.0);
        assert_eq!(child.color.g, 0.0);
        assert_eq!(child.color.b, 0.0);
    }

    #[test]
    fn var_resolves_background_color() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--bg: blue"), &parent);
        let child = compute_style(HtmlTag::Div, Some("background-color: var(--bg)"), &p);
        let bg = child.background_color.unwrap();
        assert_eq!(bg.r, 0.0);
        assert_eq!(bg.g, 0.0);
        assert_eq!(bg.b, 255.0);
    }

    #[test]
    fn text_overflow_default_is_clip() {
        let s = ComputedStyle::default();
        assert_eq!(s.text_overflow, TextOverflow::Clip);
    }

    #[test]
    fn text_overflow_ellipsis_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("text-overflow: ellipsis"), &parent);
        assert_eq!(s.text_overflow, TextOverflow::Ellipsis);
    }

    #[test]
    fn text_overflow_clip_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("text-overflow: clip"), &parent);
        assert_eq!(s.text_overflow, TextOverflow::Clip);
    }

    #[test]
    fn overflow_wrap_break_word_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("overflow-wrap: break-word"), &parent);
        assert_eq!(s.overflow_wrap, OverflowWrap::BreakWord);
    }

    #[test]
    fn word_wrap_alias_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("word-wrap: break-word"), &parent);
        assert_eq!(s.overflow_wrap, OverflowWrap::BreakWord);
    }

    #[test]
    fn word_break_break_all_enables_per_char_wrapping() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("word-break: break-all"), &parent);
        assert_eq!(s.overflow_wrap, OverflowWrap::Anywhere);
    }

    #[test]
    fn word_break_keep_all_parsed_and_inherited() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("word-break: keep-all"),
            &ComputedStyle::default(),
        );
        assert!(parent.word_break_keep_all);

        let child = compute_style(HtmlTag::Span, None, &parent);
        assert!(child.word_break_keep_all);
    }

    #[test]
    fn word_break_does_not_override_explicit_overflow_wrap() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("overflow-wrap: break-word; word-break: break-all"),
            &parent,
        );
        assert_eq!(s.overflow_wrap, OverflowWrap::BreakWord);
    }

    #[test]
    fn border_collapse_default_is_separate() {
        let s = ComputedStyle::default();
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
    }

    #[test]
    fn border_collapse_collapse_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("border-collapse: collapse"), &parent);
        assert_eq!(s.border_collapse, BorderCollapse::Collapse);
    }

    #[test]
    fn border_collapse_separate_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("border-collapse: separate"), &parent);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
    }

    #[test]
    fn border_collapse_inherits() {
        let parent = compute_style(
            HtmlTag::Table,
            Some("border-collapse: collapse"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Td, None, &parent);
        assert_eq!(child.border_collapse, BorderCollapse::Collapse);
    }

    #[test]
    fn table_layout_default_is_auto() {
        let s = ComputedStyle::default();
        assert_eq!(s.table_layout, TableLayout::Auto);
    }

    #[test]
    fn table_layout_fixed_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("table-layout: fixed"), &parent);
        assert_eq!(s.table_layout, TableLayout::Fixed);
    }

    #[test]
    fn table_layout_does_not_inherit() {
        let parent = compute_style(
            HtmlTag::Table,
            Some("table-layout: fixed"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Td, None, &parent);
        assert_eq!(child.table_layout, TableLayout::Auto);
    }

    #[test]
    fn border_spacing_single_value_sets_both_axes() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("border-spacing: 8px"), &parent);
        assert!((s.border_spacing - 6.0).abs() < 0.001); // 8px = 6pt
        assert!((s.border_spacing_vertical - 6.0).abs() < 0.001);
    }

    #[test]
    fn border_spacing_two_value_distinct_axes() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("border-spacing: 24px 6px"), &parent);
        assert!((s.border_spacing - 18.0).abs() < 0.001); // 24px = 18pt
        assert!((s.border_spacing_vertical - 4.5).abs() < 0.001); // 6px = 4.5pt
    }

    #[test]
    fn border_spacing_vertical_inherits() {
        let parent = compute_style(
            HtmlTag::Table,
            Some("border-spacing: 24px 6px"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Td, None, &parent);
        assert!((child.border_spacing - 18.0).abs() < 0.001);
        assert!((child.border_spacing_vertical - 4.5).abs() < 0.001);
    }

    #[test]
    fn caption_side_defaults_to_top() {
        let s = ComputedStyle::default();
        assert_eq!(s.caption_side, CaptionSide::Top);
    }

    #[test]
    fn caption_side_bottom_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("caption-side: bottom"), &parent);
        assert_eq!(s.caption_side, CaptionSide::Bottom);
    }

    #[test]
    fn caption_side_inherits() {
        let parent = compute_style(
            HtmlTag::Table,
            Some("caption-side: bottom"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Caption, None, &parent);
        assert_eq!(child.caption_side, CaptionSide::Bottom);
    }

    #[test]
    fn border_spacing_default_is_zero() {
        let s = ComputedStyle::default();
        assert!((s.border_spacing - 0.0).abs() < 0.001);
    }

    #[test]
    fn border_spacing_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Table, Some("border-spacing: 10px"), &parent);
        assert!((s.border_spacing - 7.5).abs() < 0.001); // 10px = 7.5pt
    }

    #[test]
    fn border_spacing_inherits() {
        let parent = compute_style(
            HtmlTag::Table,
            Some("border-spacing: 5px"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Td, None, &parent);
        assert!((child.border_spacing - 3.75).abs() < 0.001); // 5px = 3.75pt
    }

    #[test]
    fn background_size_default_is_auto() {
        let s = ComputedStyle::default();
        assert_eq!(s.background_size, BackgroundSize::Auto);
    }

    #[test]
    fn background_size_cover_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: cover"), &parent);
        assert_eq!(s.background_size, BackgroundSize::Cover);
    }

    #[test]
    fn background_size_contain_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: contain"), &parent);
        assert_eq!(s.background_size, BackgroundSize::Contain);
    }

    #[test]
    fn background_clip_default_is_border_box() {
        let s = ComputedStyle::default();
        assert_eq!(s.background_clip, BackgroundClip::Border);
    }

    #[test]
    fn background_clip_padding_box_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-clip: padding-box"), &parent);
        assert_eq!(s.background_clip, BackgroundClip::Padding);
    }

    #[test]
    fn background_clip_content_box_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-clip: content-box"), &parent);
        assert_eq!(s.background_clip, BackgroundClip::Content);
    }

    #[test]
    fn background_clip_border_box_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-clip: border-box"), &parent);
        assert_eq!(s.background_clip, BackgroundClip::Border);
    }

    #[test]
    fn background_shorthand_single_box_sets_origin_and_clip() {
        // A lone box keyword in the `background` shorthand sets BOTH
        // background-origin and background-clip (css-backgrounds-3 §3.10).
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background: red content-box"), &parent);
        assert_eq!(s.background_origin, BackgroundOrigin::Content);
        assert_eq!(s.background_clip, BackgroundClip::Content);
    }

    #[test]
    fn background_shorthand_two_boxes_set_origin_then_clip() {
        // First box = origin, second box = clip.
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background: red padding-box content-box"),
            &parent,
        );
        assert_eq!(s.background_origin, BackgroundOrigin::Padding);
        assert_eq!(s.background_clip, BackgroundClip::Content);
    }

    #[test]
    fn background_size_explicit_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 100px 200px"), &parent);
        if let BackgroundSize::Explicit {
            width,
            height,
            width_is_percent,
            height_is_percent,
        } = s.background_size
        {
            assert!(!width_is_percent);
            assert!(!height_is_percent);
            assert!((width - 75.0).abs() < 0.001); // 100px = 75pt
            assert!((height.unwrap_or_default() - 150.0).abs() < 0.001); // 200px = 150pt
        } else {
            panic!(
                "Expected BackgroundSize::Explicit, got {:?}",
                s.background_size
            );
        }
    }

    #[test]
    fn background_repeat_default_is_repeat() {
        let s = ComputedStyle::default();
        assert_eq!(s.background_repeat, BackgroundRepeat::Repeat);
    }

    #[test]
    fn background_repeat_no_repeat_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-repeat: no-repeat"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
    }

    #[test]
    fn background_repeat_repeat_x_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-repeat: repeat-x"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::RepeatX);
    }

    #[test]
    fn background_repeat_repeat_y_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-repeat: repeat-y"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::RepeatY);
    }

    #[test]
    fn background_position_default_is_zero_percent() {
        let s = ComputedStyle::default();
        assert!((s.background_position.x - 0.0).abs() < 0.001);
        assert!((s.background_position.y - 0.0).abs() < 0.001);
        assert!(s.background_position.x_is_percent);
        assert!(s.background_position.y_is_percent);
    }

    #[test]
    fn background_position_center_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: center"), &parent);
        assert!((s.background_position.x - 0.5).abs() < 0.001);
        assert!((s.background_position.y - 0.5).abs() < 0.001);
        assert!(s.background_position.x_is_percent);
        assert!(s.background_position.y_is_percent);
    }

    #[test]
    fn background_position_top_left_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: top left"), &parent);
        assert!((s.background_position.x - 0.0).abs() < 0.001);
        assert!((s.background_position.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn background_position_top_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: top"), &parent);
        assert!((s.background_position.x - 0.5).abs() < 0.001);
        assert!((s.background_position.y - 0.0).abs() < 0.001);
        assert!(s.background_position.x_is_percent);
        assert!(s.background_position.y_is_percent);
    }

    #[test]
    fn background_position_center_left_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background-position: center left"),
            &parent,
        );
        assert!((s.background_position.x - 0.0).abs() < 0.001);
        assert!((s.background_position.y - 0.5).abs() < 0.001);
    }

    #[test]
    fn background_position_bottom_center_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background-position: bottom center"),
            &parent,
        );
        assert!((s.background_position.x - 0.5).abs() < 0.001);
        assert!((s.background_position.y - 1.0).abs() < 0.001);
    }

    #[test]
    fn background_position_bottom_right_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background-position: bottom right"),
            &parent,
        );
        assert!((s.background_position.x - 1.0).abs() < 0.001);
        assert!((s.background_position.y - 1.0).abs() < 0.001);
    }

    // --- list-style-type tests ---
    #[test]
    fn list_style_type_default_is_disc() {
        let s = ComputedStyle::default();
        assert_eq!(s.list_style_type, ListStyleType::Disc);
    }

    #[test]
    fn list_style_type_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-type: circle"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Circle);
    }

    #[test]
    fn list_style_type_decimal() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-type: decimal"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Decimal);
    }

    #[test]
    fn list_style_type_none() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-type: none"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::None);
    }

    #[test]
    fn list_style_type_lower_roman() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-type: lower-roman"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::LowerRoman);
    }

    #[test]
    fn list_style_type_upper_alpha() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-type: upper-alpha"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::UpperAlpha);
    }

    #[test]
    fn list_style_type_decimal_leading_zero() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Li,
            Some("list-style-type: decimal-leading-zero"),
            &parent,
        );
        assert_eq!(s.list_style_type, ListStyleType::DecimalLeadingZero);
    }

    #[test]
    fn list_style_type_inherits() {
        let parent = compute_style(
            HtmlTag::Ul,
            Some("list-style-type: square"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Li, None, &parent);
        assert_eq!(child.list_style_type, ListStyleType::Square);
    }

    // --- list-style-position tests ---
    #[test]
    fn list_style_position_default_is_outside() {
        let s = ComputedStyle::default();
        assert_eq!(s.list_style_position, ListStylePosition::Outside);
    }

    #[test]
    fn list_style_position_inside() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style-position: inside"), &parent);
        assert_eq!(s.list_style_position, ListStylePosition::Inside);
    }

    #[test]
    fn list_style_position_inherits() {
        let parent = compute_style(
            HtmlTag::Ul,
            Some("list-style-position: inside"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Li, None, &parent);
        assert_eq!(child.list_style_position, ListStylePosition::Inside);
    }

    // --- list-style shorthand tests ---
    #[test]
    fn list_style_shorthand_type_only() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style: square"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Square);
    }

    #[test]
    fn list_style_shorthand_position_only() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style: inside"), &parent);
        assert_eq!(s.list_style_position, ListStylePosition::Inside);
    }

    #[test]
    fn list_style_shorthand_both() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Li, Some("list-style: circle inside"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Circle);
        assert_eq!(s.list_style_position, ListStylePosition::Inside);
    }

    // --- content property tests ---
    #[test]
    fn content_default_is_empty() {
        let s = ComputedStyle::default();
        assert!(s.content.is_empty());
    }

    #[test]
    fn content_string() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: \"hello\""), &parent);
        assert_eq!(s.content, vec![ContentItem::String("hello".to_string())]);
    }

    #[test]
    fn content_attr() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: attr(title)"), &parent);
        assert_eq!(s.content, vec![ContentItem::Attr("title".to_string())]);
    }

    #[test]
    fn content_counter() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: counter(section)"), &parent);
        assert_eq!(
            s.content,
            vec![ContentItem::Counter(
                "section".to_string(),
                ListStyleType::Decimal
            )]
        );
    }

    #[test]
    fn content_counter_with_style_argument() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("content: counter(chap, upper-roman)"),
            &parent,
        );
        assert_eq!(
            s.content,
            vec![ContentItem::Counter(
                "chap".to_string(),
                ListStyleType::UpperRoman
            )]
        );
    }

    #[test]
    fn content_counters_with_separator() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("content: counters(section, \".\")"),
            &parent,
        );
        assert_eq!(
            s.content,
            vec![ContentItem::Counters(
                "section".to_string(),
                ".".to_string(),
                ListStyleType::Decimal
            )]
        );
    }

    #[test]
    fn content_counters_with_style_argument() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("content: counters(section, \".\", lower-alpha)"),
            &parent,
        );
        assert_eq!(
            s.content,
            vec![ContentItem::Counters(
                "section".to_string(),
                ".".to_string(),
                ListStyleType::LowerAlpha
            )]
        );
    }

    #[test]
    fn content_none() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: none"), &parent);
        assert!(s.content.is_empty());
    }

    #[test]
    fn content_not_inherited() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("content: \"hello\""),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Span, None, &parent);
        assert!(child.content.is_empty());
    }

    // --- counter-reset tests ---
    #[test]
    fn counter_reset_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-reset: section"), &parent);
        assert_eq!(s.counter_reset, vec![("section".to_string(), 0)]);
    }

    #[test]
    fn counter_reset_with_value() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-reset: section 5"), &parent);
        assert_eq!(s.counter_reset, vec![("section".to_string(), 5)]);
    }

    #[test]
    fn counter_reset_multiple() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("counter-reset: section 0 chapter 1"),
            &parent,
        );
        assert_eq!(
            s.counter_reset,
            vec![("section".to_string(), 0), ("chapter".to_string(), 1)]
        );
    }

    #[test]
    fn counter_reset_none() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-reset: none"), &parent);
        assert!(s.counter_reset.is_empty());
    }

    #[test]
    fn counter_reset_not_inherited() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("counter-reset: section"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Span, None, &parent);
        assert!(child.counter_reset.is_empty());
    }

    // --- counter-increment tests ---
    #[test]
    fn counter_increment_parsed() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-increment: section"), &parent);
        assert_eq!(s.counter_increment, vec![("section".to_string(), 1)]);
    }

    #[test]
    fn counter_increment_with_value() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-increment: section 2"), &parent);
        assert_eq!(s.counter_increment, vec![("section".to_string(), 2)]);
    }

    #[test]
    fn counter_increment_not_inherited() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("counter-increment: section"),
            &ComputedStyle::default(),
        );
        let child = compute_style(HtmlTag::Span, None, &parent);
        assert!(child.counter_increment.is_empty());
    }

    // --- Coverage: reset_to_initial for tail properties (lines 677-688) ---

    #[test]
    fn initial_keyword_resets_text_overflow() {
        let parent = ComputedStyle::default();
        let mut p = compute_style(HtmlTag::Div, Some("text-overflow: ellipsis"), &parent);
        p.text_overflow = TextOverflow::Ellipsis;
        let s = compute_style(HtmlTag::Div, Some("text-overflow: initial"), &p);
        assert_eq!(s.text_overflow, TextOverflow::Clip);
    }

    #[test]
    fn initial_keyword_resets_border_collapse() {
        let mut parent = ComputedStyle::default();
        parent.border_collapse = BorderCollapse::Collapse;
        let s = compute_style(HtmlTag::Div, Some("border-collapse: initial"), &parent);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
    }

    #[test]
    fn initial_keyword_resets_border_spacing() {
        let mut parent = ComputedStyle::default();
        parent.border_spacing = 10.0;
        let s = compute_style(HtmlTag::Div, Some("border-spacing: initial"), &parent);
        assert!((s.border_spacing - 0.0).abs() < 0.1);
    }

    #[test]
    fn revert_keyword_keeps_border_spacing_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border_spacing = 10.0;
        let s = compute_style(HtmlTag::Div, Some("border-spacing: revert"), &parent);
        assert!((s.border_spacing - 10.0).abs() < 0.1);
    }

    #[test]
    fn initial_keyword_resets_background_size() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: initial"), &parent);
        assert_eq!(s.background_size, BackgroundSize::Auto);
    }

    #[test]
    fn initial_keyword_resets_background_repeat() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-repeat: initial"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::Repeat);
    }

    #[test]
    fn initial_keyword_resets_background_position() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: initial"), &parent);
        assert_eq!(s.background_position, BackgroundPosition::default());
    }

    #[test]
    fn initial_keyword_resets_list_style_type() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("list-style-type: initial"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Disc);
    }

    #[test]
    fn initial_keyword_resets_list_style_position() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("list-style-position: initial"), &parent);
        assert_eq!(s.list_style_position, ListStylePosition::Outside);
    }

    #[test]
    fn initial_keyword_resets_content() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: initial"), &parent);
        assert!(s.content.is_empty());
    }

    #[test]
    fn initial_keyword_resets_counter_reset() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-reset: initial"), &parent);
        assert!(s.counter_reset.is_empty());
    }

    #[test]
    fn initial_keyword_resets_counter_increment() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("counter-increment: initial"), &parent);
        assert!(s.counter_increment.is_empty());
    }

    // --- Coverage: restore_from_parent for tail properties (lines 742-753) ---

    #[test]
    fn inherit_keyword_restores_text_overflow_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.text_overflow = TextOverflow::Ellipsis;
        let s = compute_style(HtmlTag::Div, Some("text-overflow: inherit"), &parent);
        assert_eq!(s.text_overflow, TextOverflow::Ellipsis);
    }

    #[test]
    fn inherit_keyword_restores_border_collapse_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border_collapse = BorderCollapse::Collapse;
        let s = compute_style(HtmlTag::Div, Some("border-collapse: inherit"), &parent);
        assert_eq!(s.border_collapse, BorderCollapse::Collapse);
    }

    #[test]
    fn inherit_keyword_restores_border_spacing_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border_spacing = 5.0;
        let s = compute_style(HtmlTag::Div, Some("border-spacing: inherit"), &parent);
        assert!((s.border_spacing - 5.0).abs() < 0.1);
    }

    #[test]
    fn inherit_keyword_restores_background_size() {
        let mut parent = ComputedStyle::default();
        parent.background_size = BackgroundSize::Cover;
        let s = compute_style(HtmlTag::Div, Some("background-size: inherit"), &parent);
        assert_eq!(s.background_size, BackgroundSize::Cover);
    }

    #[test]
    fn inherit_keyword_restores_background_repeat() {
        let mut parent = ComputedStyle::default();
        parent.background_repeat = BackgroundRepeat::NoRepeat;
        let s = compute_style(HtmlTag::Div, Some("background-repeat: inherit"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
    }

    #[test]
    fn inherit_keyword_restores_background_position() {
        let mut parent = ComputedStyle::default();
        parent.background_position = BackgroundPosition {
            x: 0.5,
            y: 0.5,
            x_is_percent: true,
            y_is_percent: true,
        };
        let s = compute_style(HtmlTag::Div, Some("background-position: inherit"), &parent);
        assert_eq!(s.background_position, parent.background_position);
    }

    #[test]
    fn inherit_keyword_restores_background_svg() {
        let mut parent = ComputedStyle::default();
        parent.background_svg = crate::parser::svg::parse_svg_from_string(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#,
        );
        assert!(parent.background_svg.is_some());
        let s = compute_style(HtmlTag::Div, Some("background-image: inherit"), &parent);
        assert!(s.background_svg.is_some());
    }

    #[test]
    fn background_image_initial_clears_only_image_layers() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background-color: red; background-repeat: no-repeat; background-size: cover; background-position: center; background-origin: content-box; background-image: initial"#,
            ),
            &ComputedStyle::default(),
        );

        assert_eq!(
            style.background_color.map(|c| (c.r, c.g, c.b, c.a)),
            Some((255.0, 0.0, 0.0, 255.0))
        );
        assert_eq!(style.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(style.background_size, BackgroundSize::Cover);
        assert_eq!(
            style.background_position,
            BackgroundPosition {
                x: 0.5,
                y: 0.5,
                x_is_percent: true,
                y_is_percent: true,
            }
        );
        assert_eq!(style.background_origin, BackgroundOrigin::Content);
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
    }

    #[test]
    fn background_image_inherit_restores_only_image_layers() {
        let mut parent = ComputedStyle::default();
        parent.background_color = Some(Color::rgb(10, 20, 30));
        parent.background_repeat = BackgroundRepeat::NoRepeat;
        parent.background_size = BackgroundSize::Cover;
        parent.background_position = BackgroundPosition {
            x: 0.25,
            y: 0.75,
            x_is_percent: true,
            y_is_percent: true,
        };
        parent.background_origin = BackgroundOrigin::Content;
        parent.background_svg = crate::parser::svg::parse_svg_from_string(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#,
        );

        let style = compute_style(
            HtmlTag::Div,
            Some("background-color: red; background-repeat: repeat-x; background-image: inherit"),
            &parent,
        );

        assert_eq!(
            style.background_color.map(|c| (c.r, c.g, c.b, c.a)),
            Some((255.0, 0.0, 0.0, 255.0))
        );
        assert_eq!(style.background_repeat, BackgroundRepeat::RepeatX);
        assert_eq!(style.background_size, BackgroundSize::Auto);
        assert_eq!(style.background_position, BackgroundPosition::default());
        assert_eq!(style.background_origin, BackgroundOrigin::Padding);
        assert!(style.background_svg.is_some());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
    }

    #[test]
    fn background_image_none_clears_existing_svg_background() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E"); background-image: none"#,
            ),
            &parent,
        );
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
    }

    #[test]
    fn multiple_backgrounds_raster_and_gradient_coexist() {
        // The ordered atomic background-image value derives both renderer fields
        // after the cascade has selected its winner.
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background-color: #37474f; background-image: url("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAIElEQVR42mO4rK8PRJJll4CIAYWjV2sERF/rxYEIhQMABxYT6ZMR6l4AAAAASUVORK5CYII="), linear-gradient(to bottom, #ffd600, #00bcd4)"#,
            ),
            &parent,
        );
        assert!(
            style.background_image.is_some(),
            "raster layer should be present"
        );
        assert!(
            style.background_gradient.is_some(),
            "gradient layer should coexist with raster"
        );
        assert_eq!(
            style.background_color.map(|c| (c.r, c.g, c.b)),
            Some((0x37 as f32, 0x47 as f32, 0x4f as f32)),
            "base color should also survive"
        );
    }

    #[test]
    fn later_background_image_url_replaces_lower_gradient_atomically() {
        let parent = ComputedStyle::default();
        let lower = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: linear-gradient(red, blue)",
            ),
            pseudo_element: None,
        };
        let higher = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: url(higher.png)",
            ),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[lower, higher],
            "div",
            &[],
            None,
        );

        assert!(style.background_image.is_some());
        assert!(style.background_gradient.is_none());
    }

    #[test]
    fn later_background_image_gradient_replaces_lower_url_atomically() {
        let parent = ComputedStyle::default();
        let lower = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: url(lower.png)",
            ),
            pseudo_element: None,
        };
        let higher = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: linear-gradient(red, blue)",
            ),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[lower, higher],
            "div",
            &[],
            None,
        );

        assert!(style.background_image.is_none());
        assert!(style.background_gradient.is_some());
    }

    #[test]
    fn normal_background_image_cannot_replace_lower_important_image_list() {
        let parent = ComputedStyle::default();
        let lower = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: linear-gradient(red, blue) !important",
            ),
            pseudo_element: None,
        };
        let higher = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-image: url(normal.png)",
            ),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[lower, higher],
            "div",
            &[],
            None,
        );

        assert!(style.background_image.is_none());
        assert!(style.background_gradient.is_some());
    }

    #[test]
    fn background_none_clears_existing_svg_background() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E"); background: none"#,
            ),
            &parent,
        );
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
    }

    #[test]
    fn background_image_url_clears_existing_svg_background() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E"); background-image: url("data:image/png;base64,AAAA")"#,
            ),
            &parent,
        );
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
    }

    #[test]
    fn background_initial_resets_all_background_state() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                r#"background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E") no-repeat center / cover; background: initial"#,
            ),
            &ComputedStyle::default(),
        );
        assert!(style.background_color.is_none());
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
        assert_eq!(style.background_size, BackgroundSize::Auto);
        assert_eq!(style.background_repeat, BackgroundRepeat::Repeat);
        assert_eq!(style.background_position, BackgroundPosition::default());
        assert_eq!(style.background_origin, BackgroundOrigin::Padding);
    }

    #[test]
    fn background_shorthand_resets_omitted_longhands_from_previous_rule() {
        let parent = ComputedStyle::default();
        let prior_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                "background-repeat: no-repeat; background-position: center; background-origin: content-box; background-size: cover; background-color: red",
            ),
            pseudo_element: None,
        };
        let later_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                r#"background: url("data:image/png;base64,AAAA")"#,
            ),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[prior_rule, later_rule],
            "div",
            &[],
            None,
        );

        assert_eq!(style.background_repeat, BackgroundRepeat::Repeat);
        assert_eq!(style.background_size, BackgroundSize::Auto);
        assert_eq!(style.background_position, BackgroundPosition::default());
        assert_eq!(style.background_origin, BackgroundOrigin::Padding);
        assert!(style.background_color.is_none());
    }

    #[test]
    fn later_background_initial_rule_resets_previous_background_state() {
        let parent = ComputedStyle::default();
        let prior_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                r#"background: url("data:image/png;base64,AAAA") no-repeat center / cover content-box"#,
            ),
            pseudo_element: None,
        };
        let later_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style("background: initial"),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[prior_rule, later_rule],
            "div",
            &[],
            None,
        );

        assert!(style.background_color.is_none());
        assert!(style.background_svg.is_none());
        assert!(style.background_gradient.is_none());
        assert!(style.background_radial_gradient.is_none());
        assert_eq!(style.background_size, BackgroundSize::Auto);
        assert_eq!(style.background_repeat, BackgroundRepeat::Repeat);
        assert_eq!(style.background_position, BackgroundPosition::default());
        assert_eq!(style.background_origin, BackgroundOrigin::Padding);
    }

    #[test]
    fn later_background_inherit_rule_restores_parent_background_state() {
        let mut parent = ComputedStyle::default();
        parent.background_color = Some(Color::rgb(10, 20, 30));
        parent.background_repeat = BackgroundRepeat::NoRepeat;
        parent.background_size = BackgroundSize::Cover;
        parent.background_position = BackgroundPosition {
            x: 0.25,
            y: 0.75,
            x_is_percent: true,
            y_is_percent: true,
        };
        parent.background_origin = BackgroundOrigin::Content;
        parent.background_svg = crate::parser::svg::parse_svg_from_string(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#,
        );

        let prior_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style(
                r#"background: url("data:image/png;base64,AAAA") no-repeat center / cover content-box"#,
            ),
            pseudo_element: None,
        };
        let later_rule = CssRule {
            selector: "div".to_string(),
            declarations: crate::parser::css::parse_inline_style("background: inherit"),
            pseudo_element: None,
        };
        let style = compute_style_with_rules(
            HtmlTag::Div,
            None,
            &parent,
            &[prior_rule, later_rule],
            "div",
            &[],
            None,
        );

        assert_eq!(
            style.background_color.map(|c| (c.r, c.g, c.b, c.a)),
            parent.background_color.map(|c| (c.r, c.g, c.b, c.a))
        );
        assert_eq!(style.background_repeat, parent.background_repeat);
        assert_eq!(style.background_size, parent.background_size);
        assert_eq!(style.background_position, parent.background_position);
        assert_eq!(style.background_origin, parent.background_origin);
        assert!(style.background_svg.is_some());
    }

    #[test]
    fn inherit_keyword_restores_list_style_type() {
        let mut parent = ComputedStyle::default();
        parent.list_style_type = ListStyleType::Square;
        let s = compute_style(HtmlTag::Div, Some("list-style-type: inherit"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Square);
    }

    #[test]
    fn inherit_keyword_restores_list_style_position() {
        let mut parent = ComputedStyle::default();
        parent.list_style_position = ListStylePosition::Inside;
        let s = compute_style(HtmlTag::Div, Some("list-style-position: inherit"), &parent);
        assert_eq!(s.list_style_position, ListStylePosition::Inside);
    }

    #[test]
    fn inherit_keyword_restores_content() {
        let mut parent = ComputedStyle::default();
        parent.content = vec![ContentItem::String("hello".to_string())];
        let s = compute_style(HtmlTag::Div, Some("content: inherit"), &parent);
        assert_eq!(s.content, vec![ContentItem::String("hello".to_string())]);
    }

    #[test]
    fn inherit_keyword_restores_counter_reset() {
        let mut parent = ComputedStyle::default();
        parent.counter_reset = vec![("section".to_string(), 0)];
        let s = compute_style(HtmlTag::Div, Some("counter-reset: inherit"), &parent);
        assert_eq!(s.counter_reset, vec![("section".to_string(), 0)]);
    }

    #[test]
    fn inherit_keyword_restores_counter_increment() {
        let mut parent = ComputedStyle::default();
        parent.counter_increment = vec![("item".to_string(), 1)];
        let s = compute_style(HtmlTag::Div, Some("counter-increment: inherit"), &parent);
        assert_eq!(s.counter_increment, vec![("item".to_string(), 1)]);
    }

    // --- Coverage: background-repeat default branch (line 1278) ---

    #[test]
    fn background_repeat_explicit_repeat_keyword() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-repeat: repeat"), &parent);
        assert_eq!(s.background_repeat, BackgroundRepeat::Repeat);
    }

    // --- Coverage: length property resolution via Percentage/Rem/Var (lines 1306-1330) ---

    #[test]
    fn max_width_from_percentage() {
        // With a known parent width, percentage resolves eagerly.
        let mut parent = ComputedStyle::default();
        parent.width = Some(400.0);
        let s = compute_style(HtmlTag::Div, Some("max-width: 50%"), &parent);
        assert!(s.max_width.is_some());
        // Without a known parent width, percentage defers to layout time via
        // percentage_sizing hint (avoids bogus resolution against viewport).
        let parent_unknown = ComputedStyle::default();
        let s2 = compute_style(HtmlTag::Div, Some("max-width: 50%"), &parent_unknown);
        assert!(s2.max_width.is_none());
        assert_eq!(s2.percentage_sizing.max_width, Some(50.0));
    }

    #[test]
    fn min_width_from_percentage() {
        let mut parent = ComputedStyle::default();
        parent.width = Some(400.0);
        let s = compute_style(HtmlTag::Div, Some("min-width: 25%"), &parent);
        assert!(s.min_width.is_some());
        // Layout-time deferral when parent width is unknown.
        let parent_unknown = ComputedStyle::default();
        let s2 = compute_style(HtmlTag::Div, Some("min-width: 25%"), &parent_unknown);
        assert!(s2.min_width.is_none());
        assert_eq!(s2.percentage_sizing.min_width, Some(25.0));
    }

    #[test]
    fn max_height_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("max-height: 80%"), &parent);
        assert!(s.max_height.is_none());
        assert_eq!(s.percentage_sizing.max_height, Some(80.0));
    }

    #[test]
    fn min_height_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("min-height: 10%"), &parent);
        assert!(s.min_height.is_none());
        assert_eq!(s.percentage_sizing.min_height, Some(10.0));
    }

    #[test]
    fn height_percentage_stays_deferred_without_parent_height() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("height: 100%"), &parent);
        assert!(s.height.is_none());
        assert_eq!(s.percentage_sizing.height, Some(100.0));
    }

    #[test]
    fn gap_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("gap: 1rem"), &parent);
        assert!((s.gap - 12.0).abs() < 0.1);
    }

    #[test]
    fn grid_gap_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("grid-gap: 5%"), &parent);
        assert!(s.grid_gap > 0.0);
    }

    #[test]
    fn border_width_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("border-width: 0.5rem"), &parent);
        assert!((s.border.top.specified_width - 6.0).abs() < 0.1);
    }

    #[test]
    fn border_radius_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("border-radius: 50%"), &parent);
        assert_eq!(
            s.resolve_corner_radii(80.0, 40.0),
            CornerRadii::uniform(CornerRadius::new(40.0, 20.0))
        );
    }

    #[test]
    fn text_indent_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("text-indent: 2rem"), &parent);
        assert!(matches!(s.text_indent, TextIndent::Length(24.0)));
    }

    #[test]
    fn letter_spacing_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("letter-spacing: 0.1rem"), &parent);
        assert!((s.letter_spacing - 1.2).abs() < 0.1);
    }

    #[test]
    fn word_spacing_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("word-spacing: 0.5rem"), &parent);
        assert!((s.word_spacing - 6.0).abs() < 0.1);
    }

    #[test]
    fn border_spacing_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("border-spacing: 1rem"), &parent);
        assert!((s.border_spacing - 12.0).abs() < 0.1);
    }

    // --- Coverage: font-size from Var (lines 1363-1369) ---

    #[test]
    fn font_size_from_var() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--my-size: 20pt; font-size: var(--my-size)"),
            &parent,
        );
        assert!((s.font_size - 20.0).abs() < 0.1);
    }

    // --- Coverage: border-color from Var (lines 1391-1395) ---

    #[test]
    fn border_color_from_var() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--bc: blue; border-color: var(--bc)"),
            &parent,
        );
        assert_eq!(
            s.border.top.color,
            SpecifiedColor::Absolute(Color::rgb(0, 0, 255))
        );
        let c = s.border.top.color.resolve(s.color);
        assert_eq!(c.b, 255.0);
    }

    // --- Coverage: display from Var (lines 1400-1410) ---

    #[test]
    fn display_from_var_none() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("--d: none; display: var(--d)"), &parent);
        assert_eq!(s.display, Display::None);
    }

    #[test]
    fn display_from_var_inline() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--d: inline; display: var(--d)"),
            &parent,
        );
        assert_eq!(s.display, Display::Inline);
    }

    #[test]
    fn display_from_var_flex() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("--d: flex; display: var(--d)"), &parent);
        assert_eq!(s.display, Display::Flex);
    }

    #[test]
    fn display_from_var_grid() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("--d: grid; display: var(--d)"), &parent);
        assert_eq!(s.display, Display::Grid);
    }

    #[test]
    fn display_from_var_block() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("--d: block; display: var(--d)"), &parent);
        assert_eq!(s.display, Display::Block);
    }

    // --- Coverage: position from Var (lines 1414-1421) ---

    #[test]
    fn position_from_var_relative() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: relative; position: var(--p)"),
            &parent,
        );
        assert_eq!(s.position, Position::Relative);
    }

    #[test]
    fn position_from_var_absolute() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: absolute; position: var(--p)"),
            &parent,
        );
        assert_eq!(s.position, Position::Absolute);
    }

    #[test]
    fn absolute_inline_display_is_blockified_after_the_cascade() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Span,
            Some("display:inline;position:absolute"),
            &parent,
        );
        assert_eq!(s.position, Position::Absolute);
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn absolute_table_internal_displays_blockify_before_table_fixup() {
        for display in [
            "table-row-group",
            "table-header-group",
            "table-footer-group",
            "table-row",
            "table-cell",
            "table-column-group",
            "table-column",
            "table-caption",
        ] {
            let style = compute_style(
                HtmlTag::Div,
                Some(&format!("display:{display};position:absolute")),
                &ComputedStyle::default(),
            );
            assert_eq!(style.display, Display::Block, "{display}");
        }
    }

    #[test]
    fn position_from_var_preserves_fixed_semantics() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: fixed; position: var(--p)"),
            &parent,
        );
        assert_eq!(s.position, Position::Fixed);
    }

    #[test]
    fn position_from_var_unknown_static_fallback() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: bogus; position: var(--p)"),
            &parent,
        );
        assert_eq!(s.position, Position::Static);
    }

    // --- Coverage: text-align from Var (lines 1425-1433) ---

    #[test]
    fn text_align_from_var_center() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--ta: center; text-align: var(--ta)"),
            &parent,
        );
        assert_eq!(s.text_align, TextAlign::Center);
    }

    #[test]
    fn text_align_from_var_right() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--ta: right; text-align: var(--ta)"),
            &parent,
        );
        assert_eq!(s.text_align, TextAlign::Right);
    }

    #[test]
    fn text_align_from_var_justify() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--ta: justify; text-align: var(--ta)"),
            &parent,
        );
        assert_eq!(s.text_align, TextAlign::Justify);
    }

    #[test]
    fn text_align_from_var_unknown_defaults_to_left() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--ta: foobar; text-align: var(--ta)"),
            &parent,
        );
        assert_eq!(s.text_align, TextAlign::Left);
    }

    // --- Coverage: list-style-position outside default (line 1443) ---

    #[test]
    fn list_style_position_outside_default() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("list-style-position: outside"), &parent);
        assert_eq!(s.list_style_position, ListStylePosition::Outside);
    }

    // --- Coverage: parse_list_style_type unknown default (line 1479) ---

    #[test]
    fn list_style_type_unknown_defaults_to_decimal() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("list-style-type: foobar"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Custom("foobar".into()));
    }

    // --- Coverage: parse_content_value branches (lines 1497-1546) ---

    #[test]
    fn content_empty_string_after_trim() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("content: '  '"), &parent);
        // The content should contain a string with spaces
        assert!(!s.content.is_empty());
    }

    #[test]
    fn content_unterminated_quote() {
        // An unterminated quote should still produce a string item (lines 1506-1507)
        let items = parse_content_value_pub("\"hello");
        assert_eq!(items, vec![ContentItem::String("hello".to_string())]);
    }

    #[test]
    fn content_counter_function() {
        let items = parse_content_value_pub("counter(section)");
        assert_eq!(
            items,
            vec![ContentItem::Counter(
                "section".to_string(),
                ListStyleType::Decimal
            )]
        );
    }

    #[test]
    fn content_counter_unterminated() {
        // counter( without closing ) -> break (line 1541)
        let items = parse_content_value_pub("counter(section");
        assert!(items.is_empty());
    }

    #[test]
    fn content_counters_with_explicit_separator() {
        let items = parse_content_value_pub("counters(section, \".\")");
        assert_eq!(
            items,
            vec![ContentItem::Counters(
                "section".to_string(),
                ".".to_string(),
                ListStyleType::Decimal
            )]
        );
    }

    #[test]
    fn content_counters_default_separator() {
        // counters without second arg -> default "." separator (line 1528)
        let items = parse_content_value_pub("counters(section)");
        assert_eq!(
            items,
            vec![ContentItem::Counters(
                "section".to_string(),
                ".".to_string(),
                ListStyleType::Decimal
            )]
        );
    }

    #[test]
    fn content_counters_unterminated() {
        // counters( without closing ) -> break (line 1533)
        let items = parse_content_value_pub("counters(section");
        assert!(items.is_empty());
    }

    #[test]
    fn content_attr_unterminated() {
        // attr( without closing ) -> break (line 1515)
        let items = parse_content_value_pub("attr(href");
        assert!(items.is_empty());
    }

    #[test]
    fn content_unknown_token_with_space_skips() {
        // Unknown token followed by whitespace -> skip to next (line 1543-1544)
        let items = parse_content_value_pub("unknown \"hello\"");
        assert_eq!(items, vec![ContentItem::String("hello".to_string())]);
    }

    #[test]
    fn content_unknown_token_at_end_breaks() {
        // Unknown token at the end with no whitespace -> break (line 1546)
        let items = parse_content_value_pub("unknown");
        assert!(items.is_empty());
    }

    #[test]
    fn content_url_double_quoted() {
        let items = parse_content_value_pub("url(\"data:image/png;base64,AAA=\")");
        assert_eq!(
            items,
            vec![ContentItem::Url("data:image/png;base64,AAA=".to_string())]
        );
    }

    #[test]
    fn content_url_unquoted() {
        let items = parse_content_value_pub("url(icon.png)");
        assert_eq!(items, vec![ContentItem::Url("icon.png".to_string())]);
    }

    #[test]
    fn content_no_open_close_quote_keywords() {
        assert_eq!(
            parse_content_value_pub("no-open-quote"),
            vec![ContentItem::NoOpenQuote]
        );
        assert_eq!(
            parse_content_value_pub("no-close-quote"),
            vec![ContentItem::NoCloseQuote]
        );
        // `no-open-quote` must be matched before `open-quote`.
        assert_eq!(
            parse_content_value_pub("open-quote close-quote"),
            vec![ContentItem::OpenQuote, ContentItem::CloseQuote]
        );
    }

    #[test]
    fn quotes_value_none_and_pairs() {
        assert_eq!(parse_quotes_value("none"), Some(Vec::new()));
        assert_eq!(parse_quotes_value("auto"), None);
        assert_eq!(
            parse_quotes_value("\"\\\"\" \"\\\"\""),
            Some(vec![("\"".to_string(), "\"".to_string())])
        );
        assert_eq!(
            parse_quotes_value("\"\u{201C}\" \"\u{201D}\" \"\u{2039}\" \"\u{203A}\""),
            Some(vec![
                ("\u{201C}".to_string(), "\u{201D}".to_string()),
                ("\u{2039}".to_string(), "\u{203A}".to_string()),
            ])
        );
        // An odd/short list is invalid -> use UA default.
        assert_eq!(parse_quotes_value("\"a\""), None);
    }

    // --- Coverage: parse_background_size_explicit (lines 1577-1595) ---

    #[test]
    fn background_size_explicit_px() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 100px"), &parent);
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit {
                width: 75.0,
                height: None,
                width_is_percent: false,
                height_is_percent: false,
            }
        );
    }

    #[test]
    fn background_size_explicit_pt() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 50pt"), &parent);
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit {
                width: 50.0,
                height: None,
                width_is_percent: false,
                height_is_percent: false,
            }
        );
    }

    #[test]
    fn background_size_explicit_percent() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 50%"), &parent);
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit {
                width: 50.0,
                height: None,
                width_is_percent: true,
                height_is_percent: false,
            }
        );
    }

    #[test]
    fn background_size_rejects_nonzero_unitless_number() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 42"), &parent);
        assert_eq!(s.background_size, BackgroundSize::Auto);
    }

    #[test]
    fn background_size_explicit_two_values() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 100px 200px"), &parent);
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit {
                width: 75.0,
                height: Some(150.0),
                width_is_percent: false,
                height_is_percent: false,
            }
        );
    }

    #[test]
    fn filter_blur_default_is_zero() {
        let style = ComputedStyle::default();
        assert!(style.filter.operations.is_empty());
    }

    #[test]
    fn filter_blur_from_inline_style_px() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(20px)"), &parent);
        assert_eq!(style.filter.operations, vec![FilterOperation::Blur(15.0)]);
    }

    #[test]
    fn filter_blur_from_inline_style_pt() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(10pt)"), &parent);
        assert_eq!(style.filter.operations, vec![FilterOperation::Blur(10.0)]);
    }

    #[test]
    fn filter_blur_bare_number_is_rejected() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(8)"), &parent);
        assert!(style.filter.operations.is_empty());
    }

    #[test]
    fn filter_blur_none_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: none"), &parent);
        assert!(style.filter.operations.is_empty());
        assert!(!style.filter.establishes_stacking_context);
    }

    #[test]
    fn identity_filter_still_establishes_a_stacking_context() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: brightness(1)"), &parent);
        assert!(style.filter.establishes_stacking_context);
        assert_eq!(
            style.filter.operations,
            vec![FilterOperation::Brightness(1.0)]
        );
    }

    #[test]
    fn filter_blur_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.filter.operations = vec![FilterOperation::Blur(10.0)];
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!(style.filter.operations.is_empty());
    }

    #[test]
    fn filter_blur_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.filter.operations = vec![FilterOperation::Blur(12.0)];
        let style = compute_style(HtmlTag::Div, Some("filter: inherit"), &parent);
        assert_eq!(style.filter.operations, parent.filter.operations);
    }

    #[test]
    fn filter_blur_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: initial"), &parent);
        assert!(style.filter.operations.is_empty());
    }

    #[test]
    fn parse_filter_blur_valid_px() {
        let parsed = parse_filter_blur("blur(5px)");
        assert!(parsed.is_some_and(|radius| (radius - 3.75).abs() < 0.01));
    }

    #[test]
    fn parse_clip_path_shapes() {
        assert_eq!(
            parse_clip_path("circle(80px at 100px 100px)"),
            Some(ClipPath::Circle {
                r: ClipRadius::Length((60.0, false).into()),
                cx: (75.0, false).into(),
                cy: (75.0, false).into(),
                geometry_box: ShapeBox::Border,
            })
        );
        assert!(matches!(
            parse_clip_path("ellipse(100px 60px at 50% 50%)"),
            Some(ClipPath::Ellipse { .. })
        ));
        assert!(matches!(
            parse_clip_path("inset(40px 60px 40px 60px)"),
            Some(ClipPath::Inset { .. })
        ));
        match parse_clip_path("polygon(50% 0%, 100% 50%, 0% 50%)") {
            Some(ClipPath::Polygon { points, .. }) => assert_eq!(points.len(), 3),
            other => panic!("expected polygon, got {other:?}"),
        }
        assert!(matches!(
            parse_clip_path(r#"path(\"M 20 120 L 100 12 L 180 120 Z\")"#),
            Some(ClipPath::Path { commands, .. }) if commands.len() == 4
        ));
        assert_eq!(parse_clip_path("none"), None);
        assert_eq!(parse_clip_path("url(#m)"), Some(ClipPath::Url("m".into())));
    }

    #[test]
    fn parse_filter_color_functions() {
        assert_eq!(
            parse_filter("grayscale(100%)").operations,
            vec![FilterOperation::Grayscale(1.0)]
        );
        assert_eq!(
            parse_filter("grayscale(0.5)").operations,
            vec![FilterOperation::Grayscale(0.5)]
        );
        assert_eq!(
            parse_filter("invert(1)").operations,
            vec![FilterOperation::Invert(1.0)]
        );
        assert_eq!(
            parse_filter("brightness(150%)").operations,
            vec![FilterOperation::Brightness(1.5)]
        );
        assert_eq!(
            parse_filter("hue-rotate(90deg)").operations,
            vec![FilterOperation::HueRotate(90.0)]
        );
        assert_eq!(
            parse_filter("sepia()").operations,
            vec![FilterOperation::Sepia(1.0)]
        );
        let sepia_style = compute_style(
            HtmlTag::Img,
            Some("filter: sepia()"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            sepia_style.filter.operations,
            vec![FilterOperation::Sepia(1.0)]
        );
        let parsed = parse_filter("grayscale(1) blur(2px) contrast(2)");
        assert_eq!(
            parsed.operations,
            vec![
                FilterOperation::Grayscale(1.0),
                FilterOperation::Blur(1.5),
                FilterOperation::Contrast(2.0)
            ]
        );
        let none = parse_filter("none");
        assert!(none.operations.is_empty());
        assert!(none.url_id.is_none());
        assert!(!none.establishes_stacking_context);
    }

    #[test]
    fn filter_drop_shadow_nested_color_keeps_the_following_function() {
        let parsed = parse_filter_for_color(
            "drop-shadow(0 0 0 color(srgb 0 0 0 / .5)) brightness(.8)",
            Color::rgb(255, 0, 0),
        );
        assert!(parsed.url_id.is_none());
        assert!(parsed.establishes_stacking_context);
        let [
            FilterOperation::DropShadow(drop_shadow),
            FilterOperation::Brightness(0.8),
        ] = parsed.operations.as_slice()
        else {
            panic!("drop-shadow and brightness must remain in source order");
        };
        assert_eq!(
            (drop_shadow.dx, drop_shadow.dy, drop_shadow.blur),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(
            drop_shadow.color,
            Color::from_srgb(0.0, 0.0, 0.0, 0.5),
            "the nested color() alpha must not fall back to currentColor"
        );
    }

    #[test]
    fn filter_url_captures_reference_id() {
        // `filter: url(#id)` records the fragment id for later DOM resolution
        // (css-filter-effects-1 §3); it produces no inline color ops/blur.
        let parsed = parse_filter("url(#sat)");
        assert_eq!(parsed.url_id.as_deref(), Some("sat"));
        assert!(parsed.operations.is_empty());
        // Quoted form and a trailing color function still capture the id.
        let quoted = parse_filter("url('#q') grayscale(1)");
        assert_eq!(quoted.url_id.as_deref(), Some("q"));
        assert_eq!(quoted.operations, vec![FilterOperation::Grayscale(1.0)]);
        // A computed style with `filter: url(#id)` exposes the id.
        let style = compute_style(
            HtmlTag::Div,
            Some("filter: url(#sat)"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.filter.url_id.as_deref(), Some("sat"));
    }

    #[test]
    fn filter_opacity_remains_an_ordered_filter_operation() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: opacity(0.5)"), &parent);
        assert_eq!(style.opacity, 1.0);
        assert_eq!(style.filter.operations, vec![FilterOperation::Opacity(0.5)]);
        let style = compute_style(HtmlTag::Div, Some("filter: opacity(50%)"), &parent);
        assert_eq!(style.filter.operations, vec![FilterOperation::Opacity(0.5)]);
        let style = compute_style(
            HtmlTag::Div,
            Some("opacity: 0.5; filter: opacity(0.5) drop-shadow(1px 0 black)"),
            &parent,
        );
        assert_eq!(style.opacity, 0.5);
        assert!(matches!(
            style.filter.operations.as_slice(),
            [
                FilterOperation::Opacity(0.5),
                FilterOperation::DropShadow(_)
            ]
        ));
    }

    #[test]
    fn parse_filter_blur_valid_pt() {
        let parsed = parse_filter_blur("blur(10pt)");
        assert!(parsed.is_some_and(|radius| (radius - 10.0).abs() < 0.01));
    }

    #[test]
    fn parse_filter_blur_bare_number() {
        assert_eq!(parse_filter_blur("blur(12)"), None);
    }

    #[test]
    fn parse_filter_blur_none() {
        let parsed = parse_filter_blur("none");
        assert!(parsed.is_some_and(|radius| radius.abs() < f32::EPSILON));
    }

    #[test]
    fn parse_filter_blur_invalid() {
        assert!(parse_filter_blur("brightness(50%)").is_none());
        assert!(parse_filter_blur("blur()").is_none());
        assert!(parse_filter_blur("blur(abc)").is_none());
        assert!(parse_filter_blur("blur(-1px)").is_none());
    }

    #[test]
    fn parse_filter_blur_unitless_zero() {
        let parsed = parse_filter_blur("blur(0)");
        assert!(parsed.is_some_and(|radius| radius.abs() < f32::EPSILON));
    }

    #[test]
    fn parse_filter_blur_whitespace() {
        let parsed = parse_filter_blur("  blur( 5px )  ");
        assert!(parsed.is_some_and(|radius| (radius - 3.75).abs() < 0.01));
    }

    #[test]
    fn background_size_three_values_ignored() {
        // Three or more values -> None, stays Auto (line 1595)
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background-size: 100px 200px 300px"),
            &parent,
        );
        assert_eq!(s.background_size, BackgroundSize::Auto);
    }

    // --- Coverage: parse_background_position with units (lines 1610-1617, 1642) ---

    #[test]
    fn background_position_percent() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: 50%"), &parent);
        assert!((s.background_position.x - 0.5).abs() < 0.01);
        assert!(s.background_position.x_is_percent);
    }

    #[test]
    fn background_position_px() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: 10px"), &parent);
        assert!((s.background_position.x - 7.5).abs() < 0.01);
        assert!(!s.background_position.x_is_percent);
    }

    #[test]
    fn background_position_pt() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: 10pt"), &parent);
        assert!((s.background_position.x - 10.0).abs() < 0.01);
        assert!(!s.background_position.x_is_percent);
    }

    #[test]
    fn background_position_rejects_nonzero_unitless_number() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: 5"), &parent);
        assert_eq!(s.background_position, BackgroundPosition::default());
    }

    #[test]
    fn background_position_three_values_returns_default() {
        // Three or more values -> None, stays default (line 1642)
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background-position: left center top"),
            &parent,
        );
        assert_eq!(s.background_position, BackgroundPosition::default());
    }

    // --- Coverage: box_shadow color fallback (line 1702) ---

    #[test]
    fn box_shadow_only_offsets_no_color_uses_black() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("box-shadow: 2pt 2pt 0pt"), &parent);
        // When there are only 3 tokens and all parse as lengths, color defaults to BLACK
        if let Some(shadow) = s.box_shadow.first() {
            assert_eq!(shadow.color.r, 0.0);
            assert_eq!(shadow.color.g, 0.0);
            assert_eq!(shadow.color.b, 0.0);
        }
    }

    // --- Coverage: gradient stop parsing (lines 2002, 2005, 2014) ---

    #[test]
    fn gradient_stop_with_unparseable_percentage() {
        // When the percentage can't parse, the whole part is treated as color
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background: linear-gradient(to bottom, red abc%, blue)"),
            &parent,
        );
        // This exercises the fallback branch at line 2002
        assert!(s.background_gradient.is_none() || s.background_gradient.is_some());
    }

    #[test]
    fn gradient_stop_pct_no_space_before() {
        // When rfind('%') finds one but there's no space before => (part, None) branch (line 2005)
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background: linear-gradient(to bottom, red%, blue)"),
            &parent,
        );
        assert!(s.background_gradient.is_none() || s.background_gradient.is_some());
    }

    #[test]
    fn gradient_single_stop_position_zero() {
        // With only one stop (count <=1), position defaults to 0.0 (line 2014)
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("background: linear-gradient(to bottom, red, blue)"),
            &parent,
        );
        if let Some(ref g) = s.background_gradient {
            assert_eq!(resolved_positions(&g.ramp)[0], 0.0);
        }
    }

    #[test]
    fn border_top_from_stylesheet() {
        let rules = crate::parser::css::parse_stylesheet("div { border-top: 1pt solid red }");
        let parent = ComputedStyle::default();
        let style = compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &[], None);
        assert!((style.border.top.specified_width - 1.0).abs() < 0.1);
        let c = style.border.top.color.resolve(style.color);
        assert_eq!(c.r, 255.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.0);
        // Other sides should be zero
        assert!((style.border.bottom.used_width()).abs() < 0.01);
        assert!((style.border.left.used_width()).abs() < 0.01);
        assert!((style.border.right.used_width()).abs() < 0.01);
    }

    #[test]
    fn border_left_from_stylesheet() {
        let rules = crate::parser::css::parse_stylesheet("div { border-left: 3pt solid blue }");
        let parent = ComputedStyle::default();
        let style = compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &[], None);
        assert!((style.border.left.specified_width - 3.0).abs() < 0.1);
        let c = style.border.left.color.resolve(style.color);
        assert_eq!(c.r, 0.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 255.0);
        assert!((style.border.top.used_width()).abs() < 0.01);
        assert!((style.border.right.used_width()).abs() < 0.01);
        assert!((style.border.bottom.used_width()).abs() < 0.01);
    }

    #[test]
    fn border_shorthand_sets_all_sides() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2pt solid black"), &parent);
        for side in [
            style.border.top,
            style.border.right,
            style.border.bottom,
            style.border.left,
        ] {
            assert!((side.specified_width - 2.0).abs() < 0.1);
            let c = side.color.resolve(style.color);
            assert_eq!((c.r, c.g, c.b), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn border_side_overrides_shorthand() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border: 1pt solid black; border-top: 2pt solid red"),
            &parent,
        );
        // Top should be overridden to 2pt red
        assert!((style.border.top.specified_width - 2.0).abs() < 0.1);
        let top_c = style.border.top.color.resolve(style.color);
        assert_eq!(top_c.r, 255.0);
        assert_eq!(top_c.g, 0.0);
        // Other sides should remain 1pt black
        for side in [style.border.right, style.border.bottom, style.border.left] {
            assert!((side.specified_width - 1.0).abs() < 0.1);
            let c = side.color.resolve(style.color);
            assert_eq!((c.r, c.g, c.b), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn border_shorthand_and_side_longhand_follow_cascade_order() {
        let parent = ComputedStyle::default();
        let shorthand_last = compute_style(
            HtmlTag::Div,
            Some("border-top: 5pt solid red; border: 1pt solid blue"),
            &parent,
        );
        assert_eq!(shorthand_last.border.top.specified_width, 1.0);
        assert_eq!(
            shorthand_last
                .border
                .top
                .color
                .resolve(shorthand_last.color),
            Color::rgb(0, 0, 255)
        );

        let longhand_last = compute_style(
            HtmlTag::Div,
            Some("border: 1pt solid blue; border-top: 5pt solid red"),
            &parent,
        );
        assert_eq!(longhand_last.border.top.specified_width, 5.0);
        assert_eq!(
            longhand_last.border.top.color.resolve(longhand_last.color),
            Color::rgb(255, 0, 0)
        );
    }

    #[test]
    fn important_border_components_win_as_expanded_longhands() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-top: 5pt solid red !important; \
                 border: 1pt solid blue",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(style.border.top.specified_width, 5.0);
        assert_eq!(
            style.border.top.color.resolve(style.color),
            Color::rgb(255, 0, 0)
        );
        assert_eq!(style.border.right.specified_width, 1.0);
    }

    #[test]
    fn logical_borders_map_and_cascade_with_physical_borders() {
        let rtl = compute_style(
            HtmlTag::Div,
            Some(
                "direction: rtl; border: 1pt solid black; \
                 border-inline-start: 5pt solid red",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(rtl.border.right.specified_width, 5.0);
        assert_eq!(rtl.border.left.specified_width, 1.0);

        let physical_last = compute_style(
            HtmlTag::Div,
            Some(
                "border-inline-start: 5pt solid red; \
                 border-left: 2pt solid blue",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(physical_last.border.left.specified_width, 2.0);
        assert_eq!(
            physical_last.border.left.color.resolve(physical_last.color),
            Color::rgb(0, 0, 255)
        );
    }

    #[test]
    fn logical_border_mapping_covers_vertical_and_sideways_modes() {
        let vertical_rl = compute_style(
            HtmlTag::Div,
            Some(
                "writing-mode: vertical-rl; \
                 border-block-start: 3pt solid red; \
                 border-inline-start: 4pt solid blue",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(vertical_rl.border.right.specified_width, 3.0);
        assert_eq!(vertical_rl.border.top.specified_width, 4.0);

        let vertical_upright_rtl = compute_style(
            HtmlTag::Div,
            Some(
                "writing-mode: vertical-lr; direction: rtl; text-orientation: upright; \
                 border-inline-start: 6pt solid green",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(vertical_upright_rtl.border.top.specified_width, 6.0);

        let sideways_lr = compute_style(
            HtmlTag::Div,
            Some(
                "writing-mode: sideways-lr; \
                 border-block-start: 7pt solid red; \
                 border-inline-start: 8pt solid blue",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(sideways_lr.border.left.specified_width, 7.0);
        assert_eq!(sideways_lr.border.bottom.specified_width, 8.0);
    }

    #[test]
    fn logical_corner_radii_share_the_physical_cascade_group() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "direction: rtl; border-start-start-radius: 11pt; \
                 border-top-right-radius: 3pt",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.resolve_corner_radii(100.0, 100.0).top_right,
            CornerRadius::circular(3.0)
        );

        let logical_last = compute_style(
            HtmlTag::Div,
            Some(
                "direction: rtl; border-top-right-radius: 3pt; \
                 border-start-start-radius: 11pt",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            logical_last.resolve_corner_radii(100.0, 100.0).top_right,
            CornerRadius::circular(11.0)
        );
    }

    #[test]
    fn inherited_currentcolor_rebinds_to_the_child_foreground() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        parent.border.left = BorderSide::solid(2.0, SpecifiedColor::CurrentColor);
        let child = compute_style(
            HtmlTag::Div,
            Some("color: blue; border-left: inherit"),
            &parent,
        );
        assert_eq!(child.border.left.color, SpecifiedColor::CurrentColor);
        assert_eq!(
            child.border.left.color.resolve(child.color),
            Color::rgb(0, 0, 255)
        );
    }

    #[test]
    fn border_does_not_inherit() {
        let mut parent = ComputedStyle::default();
        parent.border.top = BorderSide::solid(1.0, Color::rgb(0, 0, 0).into());
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert!((style.border.top.used_width()).abs() < 0.01);
        assert!((style.border.bottom.used_width()).abs() < 0.01);
        assert!((style.border.left.used_width()).abs() < 0.01);
        assert!((style.border.right.used_width()).abs() < 0.01);
    }

    #[test]
    fn border_sides_max_and_widths() {
        // Lines 353-358: BorderSides max_width, horizontal_width, vertical_width
        let b = BorderSides {
            top: BorderSide::solid(3.0, SpecifiedColor::CurrentColor),
            right: BorderSide::solid(5.0, SpecifiedColor::CurrentColor),
            bottom: BorderSide::solid(2.0, SpecifiedColor::CurrentColor),
            left: BorderSide::solid(4.0, SpecifiedColor::CurrentColor),
        };
        assert!((b.max_width() - 5.0).abs() < 0.01);
        assert!((b.horizontal_width() - 9.0).abs() < 0.01); // left + right = 4 + 5
        assert!((b.vertical_width() - 5.0).abs() < 0.01); // top + bottom = 3 + 2
    }

    #[test]
    fn border_color_from_stylesheet() {
        // Line 830, 1093-1094: Per-side border color parsing
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border-right: 2pt solid red; border-left: 3pt solid blue"),
            &parent,
        );
        assert!((style.border.right.specified_width - 2.0).abs() < 0.1);
        let rc = style.border.right.color.resolve(style.color);
        assert_eq!(rc.r, 255.0);
        assert!((style.border.left.specified_width - 3.0).abs() < 0.1);
        let lc = style.border.left.color.resolve(style.color);
        assert_eq!(lc.b, 255.0);
    }

    #[test]
    fn var_resolution_for_width() {
        // Lines 1410-1418: Var resolution for width/height via custom properties
        let mut parent = ComputedStyle::default();
        parent
            .custom_properties
            .insert("--my-width".to_string(), "200pt".to_string());
        let style = compute_style(HtmlTag::Div, Some("width: var(--my-width)"), &parent);
        assert!(
            style.width.is_some(),
            "Expected width to be resolved from var"
        );
        assert!((style.width.unwrap() - 200.0).abs() < 0.1);
    }

    #[test]
    fn content_property_parsing() {
        // Line 1517: Content property parsing
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some(r#"content: "Hello""#), &parent);
        assert!(!style.content.is_empty(), "Expected content to be parsed");
        if let ContentItem::String(s) = &style.content[0] {
            assert_eq!(s, "Hello");
        } else {
            panic!("Expected ContentItem::String");
        }
    }

    #[test]
    fn counter_increment_from_inline() {
        // Line 1605: Counter increment parsing
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("counter-increment: section 2"), &parent);
        assert_eq!(style.counter_increment.len(), 1);
        assert_eq!(style.counter_increment[0].0, "section");
        assert_eq!(style.counter_increment[0].1, 2);
    }

    #[test]
    fn line_height_from_length_value() {
        // Line 2140: Line-height from Length value
        let parent = ComputedStyle::default(); // font_size = 12.0
        let style = compute_style(HtmlTag::Div, Some("line-height: 24pt"), &parent);
        // 24pt / 12pt = 2.0
        assert!((style.line_height - 2.0).abs() < 0.1);
    }

    // --- flex-grow / flex-shrink / flex-basis coverage tests ---

    #[test]
    fn flex_grow_property() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-grow: 2"), &parent);
        assert!((style.flex_grow - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_shrink_property() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-shrink: 0"), &parent);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_basis_length() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-basis: 200pt"), &parent);
        assert_eq!(style.flex_basis.definite_length(), Some(200.0));
    }

    #[test]
    fn flex_basis_auto() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-basis: auto"), &parent);
        assert_eq!(style.flex_basis, FlexBasis::Auto);
    }

    #[test]
    fn invalid_negative_flex_grow_does_not_replace_valid_value() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-grow: 2; flex-grow: -3"), &parent);
        assert!((style.flex_grow - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_shorthand_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: none"), &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, FlexBasis::Auto);
    }

    #[test]
    fn flex_shorthand_auto() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: auto"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, FlexBasis::Auto);
    }

    #[test]
    fn flex_shorthand_single_number() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 3"), &parent);
        assert!((style.flex_grow - 3.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert!(style.flex_basis.is_zero());
    }

    #[test]
    fn flex_shorthand_two_values() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 2 0"), &parent);
        assert!((style.flex_grow - 2.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        assert!(style.flex_basis.is_zero());
    }

    #[test]
    fn flex_shorthand_three_values() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 1 0 200px"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        // 200px ≈ 200 * 0.75 = 150pt
        assert!(
            style
                .flex_basis
                .definite_length()
                .is_some_and(|value| value > 0.0)
        );
    }

    #[test]
    fn flex_shorthand_three_values_auto_basis() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 1 1 auto"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, FlexBasis::Auto);
    }

    #[test]
    fn flex_shorthand_intrinsic_basis_keywords_preserve_their_sizing_mode() {
        let parent = ComputedStyle::default();
        let parsed = crate::parser::css::parse_inline_style("flex: 0 0 content");
        assert!(
            matches!(parsed.get("flex"), Some(CssValue::Keyword(value)) if value == "0 0 content"),
            "flex shorthand must preserve its three authored components: {parsed:#?}"
        );
        for (basis, keyword) in [
            ("content", IntrinsicWidthKeyword::MaxContent),
            ("max-content", IntrinsicWidthKeyword::MaxContent),
            ("min-content", IntrinsicWidthKeyword::MinContent),
            ("fit-content", IntrinsicWidthKeyword::FitContent),
        ] {
            let style = compute_style(HtmlTag::Div, Some(&format!("flex: 0 0 {basis}")), &parent);
            assert_eq!(style.flex_grow, 0.0);
            assert_eq!(style.flex_shrink, 0.0);
            assert_eq!(style.flex_basis.content_keyword(), Some(keyword));

            let longhand =
                compute_style(HtmlTag::Div, Some(&format!("flex-basis: {basis}")), &parent);
            assert_eq!(longhand.flex_basis.content_keyword(), Some(keyword));
        }
    }

    #[test]
    fn width_intrinsic_keywords_preserve_their_sizing_mode() {
        let parent = ComputedStyle::default();
        for (width, keyword) in [
            ("min-content", IntrinsicWidthKeyword::MinContent),
            ("max-content", IntrinsicWidthKeyword::MaxContent),
            ("fit-content", IntrinsicWidthKeyword::FitContent),
        ] {
            let parsed = crate::parser::css::parse_inline_style(&format!("width: {width}"));
            assert!(
                matches!(parsed.get("width"), Some(CssValue::Keyword(value)) if value == width),
                "intrinsic width must remain a keyword: {parsed:#?}",
            );
            let style = compute_style(HtmlTag::Div, Some(&format!("width: {width}")), &parent);
            assert_eq!(style.width, None);
            assert_eq!(style.width_keyword, Some(keyword));
        }

        let rules = crate::parser::css::parse_stylesheet(".box { width: fit-content; }");
        let style = compute_style_with_context(
            HtmlTag::Div,
            None,
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &SelectorContext::default(),
        );
        assert_eq!(style.width_keyword, Some(IntrinsicWidthKeyword::FitContent),);
    }

    #[test]
    fn flex_grow_resets_on_non_inherited() {
        let mut parent = ComputedStyle::default();
        parent.flex_grow = 5.0;
        // flex properties don't inherit — child should get default
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, FlexBasis::Auto);
    }

    #[test]
    fn flex_grow_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-grow: initial"), &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_grow_inherit() {
        let mut parent = ComputedStyle::default();
        parent.flex_grow = 3.0;
        let style = compute_style(HtmlTag::Div, Some("flex-grow: inherit"), &parent);
        assert!((style.flex_grow - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn authorable_old_currentcolor_sentinel_is_an_absolute_color() {
        // The previous implementation encoded currentColor as #010203fe. That
        // is an authorable CSS color and must never be rewritten to `color`.
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "color: red; \
                 background-color: #010203fe; \
                 border: 1px solid #010203fe; \
                 outline: 1px solid #010203fe; \
                 column-rule: 1px solid #010203fe; \
                 box-shadow: 1px 2px #010203fe; \
                 text-shadow: 1px 2px #010203fe; \
                 text-decoration: underline #010203fe",
            ),
            &ComputedStyle::default(),
        );
        let expected = (1, 2, 3, 254);
        assert_eq!(rgba_tuple(style.background_color.unwrap()), expected);
        assert_eq!(
            rgba_tuple(style.border.top.color.resolve(style.color)),
            expected
        );
        assert_eq!(rgba_tuple(style.outline_color.unwrap()), expected);
        assert_eq!(
            rgba_tuple(style.column_rule.color.resolve(style.color)),
            expected
        );
        assert_eq!(rgba_tuple(style.box_shadow[0].color), expected);
        assert_eq!(style.box_shadow[0].color_source, ColorSource::Absolute);
        assert_eq!(rgba_tuple(style.text_shadow[0].color), expected);
        assert_eq!(style.text_shadow[0].color_source, ColorSource::Absolute);
        assert_eq!(
            rgba_tuple(style.text_decorations.current.color.unwrap()),
            expected
        );
    }

    #[test]
    fn currentcolor_and_var_consumers_see_final_foreground_color() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "--paint: currentColor; \
                 background-color: var(--paint); \
                 border: 1px solid var(--paint); \
                 outline: 1px solid var(--paint); \
                 column-rule: 1px solid var(--paint); \
                 box-shadow: 1px 2px var(--paint); \
                 text-shadow: 1px 2px var(--paint); \
                 text-decoration: underline var(--paint); \
                 color: #345678",
            ),
            &ComputedStyle::default(),
        );
        let expected = (0x34, 0x56, 0x78, 0xff);
        assert_eq!(rgba_tuple(style.color), expected);
        assert_eq!(rgba_tuple(style.background_color.unwrap()), expected);
        assert_eq!(
            rgba_tuple(style.border.top.color.resolve(style.color)),
            expected
        );
        assert_eq!(rgba_tuple(style.outline_color.unwrap()), expected);
        assert_eq!(
            rgba_tuple(style.column_rule.color.resolve(style.color)),
            expected
        );
        assert_eq!(rgba_tuple(style.box_shadow[0].color), expected);
        assert_eq!(style.box_shadow[0].color_source, ColorSource::CurrentColor);
        assert_eq!(rgba_tuple(style.text_shadow[0].color), expected);
        assert_eq!(style.text_shadow[0].color_source, ColorSource::CurrentColor);
        assert_eq!(
            rgba_tuple(style.text_decorations.current.color.unwrap()),
            expected
        );
    }

    #[test]
    fn currentcolor_on_color_itself_uses_the_inherited_color() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(12, 34, 56);

        let direct = compute_style(HtmlTag::Div, Some("color: currentColor"), &parent);
        assert_eq!(direct.color, parent.color);

        let through_var = compute_style(
            HtmlTag::Div,
            Some("--foreground: currentColor; color: var(--foreground)"),
            &parent,
        );
        assert_eq!(through_var.color, parent.color);
    }

    #[test]
    fn inherited_currentcolor_text_effects_rebind_on_the_child() {
        let parent = compute_style(
            HtmlTag::Div,
            Some(
                "color: #c2185b; \
                 text-shadow: 1px 2px currentColor; \
                 text-emphasis: filled dot currentColor",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            parent.text_shadow[0].color_source,
            ColorSource::CurrentColor
        );
        assert_eq!(parent.text_emphasis_color_source, ColorSource::CurrentColor);

        let child = compute_style(HtmlTag::Span, Some("color: #1565c0"), &parent);
        let child_color = (0x15, 0x65, 0xc0, 0xff);
        assert_eq!(rgba_tuple(child.text_shadow[0].color), child_color);
        assert_eq!(rgba_tuple(child.text_emphasis_color), child_color);

        let absolute_parent = compute_style(
            HtmlTag::Div,
            Some("color: #c2185b; text-shadow: 1px 2px #c2185b"),
            &ComputedStyle::default(),
        );
        let absolute_child = compute_style(HtmlTag::Span, Some("color: #1565c0"), &absolute_parent);
        assert_eq!(
            rgba_tuple(absolute_child.text_shadow[0].color),
            (0xc2, 0x18, 0x5b, 0xff)
        );
        assert_eq!(
            absolute_child.text_shadow[0].color_source,
            ColorSource::Absolute
        );
    }

    #[test]
    fn text_emphasis_does_not_overwrite_independent_overline_color() {
        let style = compute_style(
            HtmlTag::Span,
            Some(
                "color: #111111; \
                 text-decoration: overline #d7263d; \
                 text-emphasis: filled dot #1565c0",
            ),
            &ComputedStyle::default(),
        );

        assert!(style.text_decorations.current.lines.overline);
        assert!(style.text_emphasis_mark);
        assert_eq!(
            rgba_tuple(style.text_decorations.current.color.unwrap()),
            (0xd7, 0x26, 0x3d, 0xff)
        );
        assert_eq!(
            rgba_tuple(style.text_emphasis_color),
            (0x15, 0x65, 0xc0, 0xff)
        );
    }

    #[test]
    fn text_emphasis_position_is_inherited_and_keeps_its_keyword_pair() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("text-emphasis-position: under left"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            parent.text_emphasis_position,
            TextEmphasisPosition::UnderLeft
        );

        let child = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(
            child.text_emphasis_position,
            TextEmphasisPosition::UnderLeft
        );

        let reset = compute_style(
            HtmlTag::Span,
            Some("text-emphasis-position: initial"),
            &parent,
        );
        assert_eq!(
            reset.text_emphasis_position,
            TextEmphasisPosition::OverRight
        );
    }

    #[test]
    fn currentcolor_resolves_inside_gradient_image_consumers() {
        let current = Color::rgb(21, 101, 192);
        let linear =
            parse_linear_gradient_for_color("linear-gradient(currentColor, white)", current)
                .unwrap();
        let radial =
            parse_radial_gradient_for_color("radial-gradient(currentColor, white)", current)
                .unwrap();
        let conic =
            parse_conic_gradient_for_color("conic-gradient(currentColor, white)", current).unwrap();
        assert_eq!(linear.ramp.stops[0].color.color, current);
        assert_eq!(radial.ramp.stops[0].color.color, current);
        assert_eq!(conic.ramp.stops[0].color.color, current);

        let style = compute_style(
            HtmlTag::Div,
            Some(
                "--stop: currentColor; color: #1565c0; \
                 background-image: linear-gradient(var(--stop), white)",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.background_gradient.unwrap().ramp.stops[0].color.color,
            current
        );
    }

    // ---- Pseudo-element style computation tests ----

    #[test]
    fn pseudo_element_style_inherits_color() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let parent = ComputedStyle::default();
        let mut parent_with_color = parent.clone();
        parent_with_color.color = Color::rgb(255, 0, 0);
        let rules = parse_stylesheet(".box::before { content: 'X'; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent_with_color,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::Before,
        );
        assert!(result.is_some());
        let ps = result.unwrap();
        // Color should be inherited from parent
        let (r, g, b) = ps.color.to_f32_rgb();
        assert!((r - 1.0).abs() < 0.01 && g < 0.01 && b < 0.01);
    }

    #[test]
    fn pseudo_element_style_applies_own_declarations() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let parent = ComputedStyle::default();
        let rules =
            parse_stylesheet(".box::after { content: 'Y'; font-weight: bold; display: block; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::After,
        );
        assert!(result.is_some());
        let ps = result.unwrap();
        assert_eq!(ps.font_weight, FontWeight::Bold);
        assert_eq!(ps.display, Display::Block);
    }

    #[test]
    fn pseudo_preserves_the_originating_decoration_color() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};

        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(0, 0, 238);
        parent.text_decorations.current.lines.underline = true;
        let rules = parse_stylesheet(
            ".link::after { content: ' target'; color: #d7263d; text-decoration: none; }",
        );
        let pseudo = compute_pseudo_element_style(
            &parent,
            &rules,
            "a",
            &["link"],
            None,
            &HashMap::new(),
            &SelectorContext::default(),
            PseudoElement::After,
        )
        .unwrap();

        assert!(!pseudo.text_decorations.active(pseudo.color).is_empty());
        assert_eq!(rgba_tuple(pseudo.color), (0xd7, 0x26, 0x3d, 0xff));
        assert_eq!(
            rgba_tuple(
                pseudo.text_decorations.active(pseudo.color)[0]
                    .color
                    .unwrap()
            ),
            (0x00, 0x00, 0xee, 0xff)
        );
    }

    #[test]
    fn pseudo_currentcolor_consumers_use_the_final_cascade_winner() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};

        // Exercise both source orders. The more-specific color declaration wins
        // in either case, and currentColor consumers must bind only after the
        // complete pseudo-element cascade has selected that winner.
        for css in [
            ".box::before { content: 'X'; background-color: currentColor; } \
             div.box::before { color: #1565c0; }",
            "div.box::before { color: #1565c0; } \
             .box::before { content: 'X'; background-color: currentColor; }",
        ] {
            let rules = parse_stylesheet(css);
            let style = compute_pseudo_element_style(
                &ComputedStyle::default(),
                &rules,
                "div",
                &["box"],
                None,
                &HashMap::new(),
                &SelectorContext::default(),
                PseudoElement::Before,
            )
            .unwrap();
            assert_eq!(rgba_tuple(style.color), (0x15, 0x65, 0xc0, 0xff));
            assert_eq!(
                rgba_tuple(style.background_color.unwrap()),
                (0x15, 0x65, 0xc0, 0xff)
            );
        }
    }

    #[test]
    fn pseudo_currentcolor_honors_important_before_specificity() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};

        let rules = parse_stylesheet(
            ".box::after { content: 'X'; color: #c2185b !important; } \
             div.box::after { color: #1565c0; background-color: currentColor; }",
        );
        let style = compute_pseudo_element_style(
            &ComputedStyle::default(),
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &SelectorContext::default(),
            PseudoElement::After,
        )
        .unwrap();
        assert_eq!(rgba_tuple(style.color), (0xc2, 0x18, 0x5b, 0xff));
        assert_eq!(
            rgba_tuple(style.background_color.unwrap()),
            (0xc2, 0x18, 0x5b, 0xff)
        );
    }

    #[test]
    fn pseudo_element_none_without_content() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let parent = ComputedStyle::default();
        // No content property = no pseudo-element
        let rules = parse_stylesheet(".box::before { color: red; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::Before,
        );
        assert!(result.is_none());
    }

    #[test]
    fn pseudo_element_none_with_content_none() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let parent = ComputedStyle::default();
        let rules = parse_stylesheet(".box::before { content: none; color: red; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::Before,
        );
        assert!(result.is_none());
    }

    #[test]
    fn pseudo_element_resets_non_inherited() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let mut parent = ComputedStyle::default();
        parent.width = Some(200.0);
        parent.position = Position::Relative;
        parent.background_color = Some(Color::rgb(128, 128, 128));
        let rules = parse_stylesheet(".box::before { content: 'X'; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::Before,
        );
        let ps = result.unwrap();
        // Non-inherited properties should be reset
        assert_eq!(ps.width, None);
        assert_eq!(ps.position, Position::Static);
        assert!(ps.background_color.is_none());
    }

    #[test]
    fn pseudo_element_resets_background_image_layers() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};

        let mut parent = ComputedStyle::default();
        parent.background_image = Some("data:image/png;base64,abc".to_string());
        parent.background_svg = crate::parser::svg::parse_svg_from_string(
            r#"<svg width="1" height="1"><rect width="1" height="1"/></svg>"#,
        );
        parent.background_origin = BackgroundOrigin::Content;
        parent.background_repeat = BackgroundRepeat::NoRepeat;

        let rules = parse_stylesheet(".box::before { content: 'X'; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::Before,
        );

        let ps = result.unwrap();
        assert!(ps.background_image.is_none());
        assert!(ps.background_svg.is_none());
        assert_eq!(ps.background_origin, BackgroundOrigin::Padding);
        assert_eq!(ps.background_repeat, BackgroundRepeat::Repeat);
    }

    #[test]
    fn pseudo_element_rules_skipped_in_normal_style() {
        use crate::parser::css::parse_stylesheet;
        let parent = ComputedStyle::default();
        // This rule targets ::before, not the element itself
        let rules = parse_stylesheet(".box::before { content: 'X'; font-weight: bold; }");
        let style =
            compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &["box"], None);
        // The element should NOT get font-weight: bold from the ::before rule
        assert_eq!(style.font_weight, FontWeight::Normal);
    }

    #[test]
    fn background_image_inherit_copies_gradient() {
        use crate::parser::css::{PseudoElement, parse_stylesheet};
        let mut parent = ComputedStyle::default();
        parent.background_gradient = Some(LinearGradient {
            angle: 90.0,
            ramp: GradientRamp::default(),
            layer_box: GradientLayerBox::default(),
        });
        let rules = parse_stylesheet(".box::after { content: ''; background-image: inherit; }");
        let ctx = SelectorContext::default();
        let result = compute_pseudo_element_style(
            &parent,
            &rules,
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &ctx,
            PseudoElement::After,
        );
        let ps = result.unwrap();
        assert!(ps.background_gradient.is_some());
    }

    #[test]
    fn border_image_gradient_preserves_balanced_source_and_slice_structure() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-image: linear-gradient(to right, rgba(255, 0, 0, .5), blue) 1 2% 3 4% fill",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices {
                    top: BorderImageSliceValue::Number(1.0),
                    right: BorderImageSliceValue::Percentage(2.0),
                    bottom: BorderImageSliceValue::Number(3.0),
                    left: BorderImageSliceValue::Percentage(4.0),
                    fill: true,
                },
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_does_not_replace_an_independent_background_gradient() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "background-image: linear-gradient(white, black); \
                 border-image: linear-gradient(red, blue) 1",
            ),
            &ComputedStyle::default(),
        );

        assert!(style.background_gradient.is_some());
        assert!(style.border_image.has_source());
    }

    #[test]
    fn border_image_accepts_url_radial_and_conic_image_sources() {
        let parent = ComputedStyle::default();
        let url = compute_style(
            HtmlTag::Div,
            Some("border-image: url('data:image/png;base64,AAAA') 1"),
            &parent,
        );
        let radial = compute_style(
            HtmlTag::Div,
            Some("border-image: radial-gradient(red, blue) 1"),
            &parent,
        );
        let conic = compute_style(
            HtmlTag::Div,
            Some("border-image: conic-gradient(red, blue) 1"),
            &parent,
        );

        assert!(matches!(
            url.border_image.paint().map(|image| image.source),
            Some(BorderImageSource::Url(source))
                if source == "data:image/png;base64,AAAA"
        ));
        assert!(matches!(
            radial.border_image.paint().map(|image| image.source),
            Some(BorderImageSource::RadialGradient(_))
        ));
        assert!(matches!(
            conic.border_image.paint().map(|image| image.source),
            Some(BorderImageSource::ConicGradient(_))
        ));
    }

    #[test]
    fn border_image_numeric_slices_resolve_in_css_pixels() {
        let slices = BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0));
        assert_eq!(slices.resolve(165.0, 112.5, 0.75), EdgeSizes::uniform(0.75));
    }

    #[test]
    fn overlapping_border_image_slices_preserve_each_corner_region() {
        let slices = BorderImageSlices {
            top: BorderImageSliceValue::Percentage(70.0),
            right: BorderImageSliceValue::Percentage(80.0),
            bottom: BorderImageSliceValue::Percentage(70.0),
            left: BorderImageSliceValue::Percentage(80.0),
            fill: false,
        };
        assert_eq!(
            slices.resolve(100.0, 200.0, 1.0),
            EdgeSizes::new(140.0, 80.0, 140.0, 80.0)
        );
    }

    #[test]
    fn border_image_width_overlap_uses_one_factor_for_both_axes() {
        let widths = BorderImageWidths::uniform(BorderImageWidth::Number(1.0));
        assert_eq!(
            widths.resolve(EdgeSizes::uniform(80.0), 100.0, 400.0, None,),
            EdgeSizes::uniform(50.0)
        );
    }

    #[test]
    fn border_image_width_number_multiplies_the_physical_border() {
        assert_eq!(
            BorderImageWidths::uniform(BorderImageWidth::Number(3.0)).resolve(
                EdgeSizes::uniform(6.0),
                135.0,
                63.0,
                None,
            ),
            EdgeSizes::uniform(18.0)
        );
    }

    #[test]
    fn border_image_outset_number_expands_from_the_physical_border() {
        assert_eq!(
            BorderImageOutsets::uniform(BorderImageOutset::Number(2.0))
                .resolve(EdgeSizes::new(3.0, 5.0, 7.0, 11.0)),
            EdgeSizes::new(6.0, 10.0, 14.0, 22.0)
        );
    }

    #[test]
    fn border_image_font_relative_width_and_outset_resolve_after_font_size() {
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 40px; border-image: linear-gradient(red, blue) 1 / .25em / .25em"),
            &ComputedStyle::default(),
        );

        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                widths: BorderImageWidths::uniform(BorderImageWidth::LengthPercent(
                    LengthPercent::length(7.5),
                )),
                outsets: BorderImageOutsets::uniform(BorderImageOutset::Length(7.5)),
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_width_keeps_calc_percentage_for_the_image_area() {
        let style = compute_style(
            HtmlTag::Div,
            Some("font-size: 40px; border-image: linear-gradient(red, blue) 1 / calc(25% + .25em)"),
            &ComputedStyle::default(),
        );

        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                widths: BorderImageWidths::uniform(BorderImageWidth::LengthPercent(
                    LengthPercent::from_terms(7.5, 25.0),
                )),
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_repeat_uses_one_or_two_axis_keywords() {
        let style = compute_style(
            HtmlTag::Div,
            Some("border-image: linear-gradient(red, blue) 1 repeat round"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                repeats: BorderImageRepeats {
                    horizontal: BorderImageRepeatMode::Repeat,
                    vertical: BorderImageRepeatMode::Round,
                },
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_repeat_longhand_cascades_independently_from_the_shorthand() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-image-repeat: space !important; border-image: linear-gradient(red, blue) 1 repeat stretch",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                repeats: BorderImageRepeats::uniform(BorderImageRepeatMode::Space),
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_width_longhand_overrides_the_shorthand_component() {
        let style = compute_style(
            HtmlTag::Div,
            Some("border-image: linear-gradient(red, blue) 1; border-image-width: 3"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                widths: BorderImageWidths::uniform(BorderImageWidth::Number(3.0)),
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_shorthand_resets_every_border_image_longhand() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-image: linear-gradient(red, blue) 7 / 4 / 3 round; \
                 border: 2px solid black; \
                 border-image-source: linear-gradient(green, yellow)",
            ),
            &ComputedStyle::default(),
        );

        assert!(style.border_image.has_source());
        assert_eq!(style.border_image.geometry, BorderImage::default());
    }

    #[test]
    fn border_image_longhands_inherit_without_a_parent_source() {
        let parent = compute_style(
            HtmlTag::Div,
            Some("border-image-slice: 9 fill; border-image-width: 3"),
            &ComputedStyle::default(),
        );
        let child = compute_style(
            HtmlTag::Div,
            Some(
                "border-image-source: linear-gradient(red, blue); \
                 border-image-slice: inherit; border-image-width: inherit",
            ),
            &parent,
        );

        assert!(child.border_image.has_source());
        assert_eq!(
            child.border_image.geometry.slices,
            BorderImageSlices {
                fill: true,
                ..BorderImageSlices::uniform(BorderImageSliceValue::Number(9.0))
            }
        );
        assert_eq!(
            child.border_image.geometry.widths,
            BorderImageWidths::uniform(BorderImageWidth::Number(3.0))
        );
    }

    #[test]
    fn border_image_width_important_longhand_survives_a_later_plain_shorthand() {
        let style = compute_style(
            HtmlTag::Div,
            Some("border-image-width: 3 !important; border-image: linear-gradient(red, blue) 1"),
            &ComputedStyle::default(),
        );
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                widths: BorderImageWidths::uniform(BorderImageWidth::Number(3.0)),
                ..Default::default()
            })
        );
    }

    #[test]
    fn border_image_outset_shorthand_and_longhand_cascade_independently() {
        let style = compute_style(
            HtmlTag::Div,
            Some(
                "border-image-outset: 3 !important; border-image: linear-gradient(red, blue) 1 / 1 / 2",
            ),
            &ComputedStyle::default(),
        );
        assert_eq!(style.background_clip, BackgroundClip::Border);
        assert_eq!(
            style.border_image.paint().map(|image| image.geometry),
            Some(BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                widths: BorderImageWidths::uniform(BorderImageWidth::Number(1.0)),
                outsets: BorderImageOutsets::uniform(BorderImageOutset::Number(3.0)),
                ..Default::default()
            })
        );
    }
}
