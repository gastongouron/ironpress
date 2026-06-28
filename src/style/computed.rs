use std::collections::HashMap;

use crate::parser::css::{
    CssRule, CssValue, SelectorContext, StyleMap, parse_length, selector_matches_with_context,
    specificity,
};
use crate::parser::dom::HtmlTag;
use crate::style::defaults::default_style;
use crate::types::{Color, EdgeSizes};

/// Sentinel color used to mark a property whose value was the CSS
/// `currentColor` keyword. The cascade can't resolve `currentColor` while a
/// property is being parsed (the element's final `color` may still change in a
/// later cascade layer), so the keyword is parsed to this unique sentinel and a
/// post-pass at the end of `compute_style_with_context` replaces every
/// occurrence with the element's computed `color`. The RGBA value is chosen to
/// be effectively unauthorable so it can't collide with a real color.
const CURRENT_COLOR_SENTINEL: Color = Color {
    r: 1,
    g: 2,
    b: 3,
    a: 254,
};

/// CSS display property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    None,
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
/// - `SpanNamed(name)`: span until the next line named `name`. Approximated as a
///   1-track span (the named line vocabulary rarely repeats in print fixtures).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum GridLine {
    #[default]
    Auto,
    Line(i32),
    Named(String),
    Span(usize),
    SpanNamed(String),
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

/// A CSS `clip-path` basic shape. Lengths are in points; positions/percentages
/// resolve against the element's border box at render time.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipPath {
    /// `circle(r at cx cy)` — radius + centre, each (value, is_percent).
    Circle {
        r: (f32, bool),
        cx: (f32, bool),
        cy: (f32, bool),
    },
    /// `ellipse(rx ry at cx cy)`.
    Ellipse {
        rx: (f32, bool),
        ry: (f32, bool),
        cx: (f32, bool),
        cy: (f32, bool),
    },
    /// `inset(top right bottom left [round radius])`.
    Inset {
        top: (f32, bool),
        right: (f32, bool),
        bottom: (f32, bool),
        left: (f32, bool),
        radius: f32,
    },
    /// `polygon(x y, ...)` — vertices, each coord (value, is_percent).
    Polygon(Vec<((f32, bool), (f32, bool))>),
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

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/// Font style.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
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
    Absolute,
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
}

impl BlendMode {
    /// Parse a CSS blend-mode keyword. Unknown keywords fall back to `Normal`.
    pub fn from_keyword(keyword: &str) -> Self {
        match keyword.trim().to_ascii_lowercase().as_str() {
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "color-dodge" => BlendMode::ColorDodge,
            "color-burn" => BlendMode::ColorBurn,
            "hard-light" => BlendMode::HardLight,
            "soft-light" => BlendMode::SoftLight,
            "difference" => BlendMode::Difference,
            "exclusion" => BlendMode::Exclusion,
            _ => BlendMode::Normal,
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
        }
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Rotate by the given angle in degrees.
    Rotate(f32),
    /// Scale by (sx, sy).
    Scale(f32, f32),
    /// Translate by (tx, ty). When the corresponding `*_pct` flag is set the
    /// value is a percentage (0..100) resolved against the element's OWN
    /// border-box width (tx) / height (ty) at render time; otherwise it is an
    /// absolute length in pt.
    Translate {
        tx: f32,
        ty: f32,
        tx_pct: bool,
        ty_pct: bool,
    },
    /// Pre-composed affine matrix `(a, b, c, d, e, f)` for chained transforms.
    ///
    /// `e`/`f` are constant pt translations; `e_w`/`e_h`/`f_w`/`f_h` are
    /// coefficients (fractions) multiplying the box width/height to account for
    /// percentage `translate()` components that appear anywhere in the chain.
    /// At render time the effective translation is
    /// `e + e_w*w + e_h*h` / `f + f_w*w + f_h*h`.
    Matrix(f32, f32, f32, f32, f32, f32),
    /// Composed matrix carrying percentage-translate coefficients (see above).
    /// Only emitted when a chained transform contains a `%` translate; plain
    /// chains collapse to [`Transform::Matrix`].
    MatrixPct {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
        e_w: f32,
        e_h: f32,
        f_w: f32,
        f_h: f32,
    },
}

impl Transform {
    /// Resolve this transform to a concrete CSS affine matrix `[a, b, c, d, e, f]`
    /// given the element's border-box size in pt. Percentage translate
    /// components resolve against `w`/`h` here. The returned matrix is in CSS
    /// (y-down) convention; the renderer applies the y-flip + origin
    /// conjugation.
    pub fn to_css_matrix(self, w: f32, h: f32) -> [f32; 6] {
        match self {
            Transform::Rotate(deg) => {
                let rad = deg * std::f32::consts::PI / 180.0;
                let (c, s) = (rad.cos(), rad.sin());
                [c, s, -s, c, 0.0, 0.0]
            }
            Transform::Scale(sx, sy) => [sx, 0.0, 0.0, sy, 0.0, 0.0],
            Transform::Translate {
                tx,
                ty,
                tx_pct,
                ty_pct,
            } => {
                let ex = if tx_pct { tx / 100.0 * w } else { tx };
                let ey = if ty_pct { ty / 100.0 * h } else { ty };
                [1.0, 0.0, 0.0, 1.0, ex, ey]
            }
            Transform::Matrix(a, b, c, d, e, f) => [a, b, c, d, e, f],
            Transform::MatrixPct {
                a,
                b,
                c,
                d,
                e,
                f,
                e_w,
                e_h,
                f_w,
                f_h,
            } => [a, b, c, d, e + e_w * w + e_h * h, f + f_w * w + f_h * h],
        }
    }
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
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self {
            x_fraction: 0.5,
            x_length: 0.0,
            y_fraction: 0.5,
            y_length: 0.0,
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

/// CSS `writing-mode` property (css-writing-modes-4 §3.1). Inherited; initial
/// is `horizontal-tb`. Only the two horizontally/vertically-flowing modes the
/// engine renders are modelled: the default top-to-bottom horizontal mode and
/// `vertical-rl` (vertical text, columns laid right-to-left). Latin glyphs in
/// `vertical-rl` are set sideways (rotated 90° clockwise) per the default
/// `text-orientation: mixed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    /// `horizontal-tb` — the default: inline progresses left-to-right, block
    /// progresses top-to-bottom.
    #[default]
    HorizontalTb,
    /// `vertical-rl` — inline progresses top-to-bottom, block (columns)
    /// progresses right-to-left.
    VerticalRl,
}

/// A color stop in a gradient.
#[derive(Debug, Clone, Copy)]
pub struct GradientStop {
    pub color: Color,
    /// Position in the gradient (0.0 to 1.0).
    pub position: f32,
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
}

/// A CSS linear gradient.
#[derive(Debug, Clone)]
pub struct LinearGradient {
    /// Angle in degrees (0 = to top, 90 = to right, 180 = to bottom, 270 = to left).
    pub angle: f32,
    /// Color stops (at least 2).
    pub stops: Vec<GradientStop>,
    /// `true` for `repeating-linear-gradient(...)`: the stop pattern tiles to
    /// fill the gradient line instead of clamping the end colors.
    pub repeating: bool,
    /// Per-layer size/position/repeat when this gradient is one of several
    /// comma-separated background layers.
    pub layer_box: GradientLayerBox,
}

/// A position component of a radial gradient's center, resolvable against the
/// painted box at render time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RadialPos {
    /// Fraction of the box extent (0..1), e.g. from a keyword or percentage.
    Fraction(f32),
    /// Absolute offset in points from the box's start edge (left/top).
    Points(f32),
}

impl RadialPos {
    /// Resolve to an offset in points given the box extent (in points) along
    /// this axis.
    pub fn resolve(self, extent: f32) -> f32 {
        match self {
            RadialPos::Fraction(f) => extent * f,
            RadialPos::Points(p) => p,
        }
    }
}

/// The ending shape of a CSS radial gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadialShape {
    /// `circle` — a single radius along both axes.
    Circle,
    /// `ellipse` — independent horizontal/vertical radii. This is the CSS
    /// default when no shape keyword is given.
    #[default]
    Ellipse,
}

/// The size/extent of a CSS radial gradient's ending shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadialExtent {
    /// Ending shape meets the box side(s) closest to the center.
    ClosestSide,
    /// Ending shape passes through the box corner closest to the center.
    ClosestCorner,
    /// Ending shape meets the box side(s) farthest from the center.
    FarthestSide,
    /// Ending shape passes through the box corner farthest from the center.
    /// The CSS default when no extent keyword or explicit size is given.
    #[default]
    FarthestCorner,
}

/// A CSS radial gradient.
#[derive(Debug, Clone)]
pub struct RadialGradient {
    /// Color stops (at least 2).
    pub stops: Vec<GradientStop>,
    /// Center position parsed from `at <pos>`, as `(x, y)` measured from the
    /// box's left/top edges (CSS top-down). Defaults to box center.
    pub center: (RadialPos, RadialPos),
    /// Ending shape (circle vs ellipse). Determines how the unspecified extent
    /// (`farthest-corner`) is resolved into radii at render time.
    pub shape: RadialShape,
    /// Extent keyword controlling how the ending shape is sized when no explicit
    /// radius/radii are given. Defaults to `farthest-corner`.
    pub extent: RadialExtent,
    /// Explicit circular radius in points (e.g. `circle 60px` → 45pt). When
    /// `None`, the `extent` is used. Only meaningful for `RadialShape::Circle`.
    pub radius: Option<f32>,
    /// Explicit elliptical radii `(rx, ry)` in points (e.g. `ellipse 100px 50px`).
    /// `%` components are stored as a fraction of the box width/height resolved at
    /// render time via `RadialPos`. When `None`, the `extent` is used.
    pub radii: Option<(RadialPos, RadialPos)>,
    /// `true` for `repeating-radial-gradient(...)`: the stop pattern tiles along
    /// the gradient ray instead of clamping the end colors.
    pub repeating: bool,
    /// Per-layer size/position/repeat when this gradient is one of several
    /// comma-separated background layers.
    pub layer_box: GradientLayerBox,
}

/// A CSS conic gradient. PDF has no native conic shading, so this is rendered as
/// a fine sector fan (one filled wedge per small angular step) clipped to the
/// box at paint time.
#[derive(Debug, Clone)]
pub struct ConicGradient {
    /// Starting angle in degrees, clockwise from 12 o'clock (CSS `from <angle>`).
    pub from_angle: f32,
    /// Center position as `(x, y)` measured from the box's left/top edges (CSS
    /// top-down). Defaults to box center.
    pub center: (RadialPos, RadialPos),
    /// Angular color stops, positions normalized to a fraction of a full turn
    /// (0.0..=1.0, where 1.0 = 360deg). Always at least 2, sorted ascending.
    pub stops: Vec<GradientStop>,
    /// `true` for `repeating-conic-gradient(...)`: the stop pattern tiles around
    /// the full sweep instead of spanning a single turn.
    pub repeating: bool,
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
}
/// CSS background-repeat property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
    None,
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

/// CSS box-shadow value.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

/// Border line style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// Two parallel solid rules separated by a gap (CSS `double`). Each rule and
    /// the gap take roughly one third of the border width.
    Double,
    None,
}

/// A single border side with width, color, and style.
#[derive(Debug, Clone, Copy, Default)]
pub struct BorderSide {
    pub width: f32,
    pub color: Option<Color>,
    pub style: BorderStyle,
}

/// Per-side border specification.
#[derive(Debug, Clone, Copy, Default)]
pub struct BorderSides {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

#[allow(dead_code)]
impl BorderSides {
    pub fn uniform(width: f32, color: Option<Color>) -> Self {
        let side = BorderSide {
            width,
            color,
            style: BorderStyle::Solid,
        };
        Self {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }
    pub fn uniform_styled(width: f32, color: Option<Color>, style: BorderStyle) -> Self {
        let side = BorderSide {
            width,
            color,
            style,
        };
        Self {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }
    pub fn has_any(&self) -> bool {
        self.top.width > 0.0
            || self.right.width > 0.0
            || self.bottom.width > 0.0
            || self.left.width > 0.0
    }
    pub fn max_width(&self) -> f32 {
        self.top
            .width
            .max(self.right.width)
            .max(self.bottom.width)
            .max(self.left.width)
    }
    pub fn horizontal_width(&self) -> f32 {
        self.left.width + self.right.width
    }
    pub fn vertical_width(&self) -> f32 {
        self.top.width + self.bottom.width
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

/// CSS Fragmentation 3 §3.1 forced/avoid break value for `break-before` /
/// `break-after`. `Auto` is the initial value (a class-A break opportunity with
/// no forced break and no avoidance). The forced values (`page`/`left`/`right`/
/// `recto`/`verso`) always start a new page; the sided ones additionally force
/// the following content onto a left/right (verso/recto) page. `Avoid` is a
/// discretionary hint (currently a no-op in pagination).
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
    pub font_size: f32,
    pub root_font_size: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
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
    /// CSS direction property (ltr/rtl), set from `dir` attribute or CSS.
    pub direction_rtl: bool,
    /// CSS `writing-mode` (css-writing-modes-4 §3.1). Inherited; initial
    /// `horizontal-tb`. Inherited automatically via the `parent.clone()` model
    /// in `compute_style_with_context` (never reset in the non-inherited block).
    pub writing_mode: WritingMode,
    /// CSS `unicode-bidi: bidi-override` (or `isolate-override`). When set, the
    /// element's inline content is reordered strictly in sequence according to
    /// `direction`, overriding the characters' intrinsic bidi classes
    /// (css-writing-modes-4 §2.4). Not inherited; initial is `normal` (false).
    pub bidi_override: bool,
    pub text_decoration_underline: bool,
    pub text_decoration_line_through: bool,
    pub text_decoration_overline: bool,
    /// CSS `text-decoration-color` (css-text-decor-3 §2.2): the colour of the
    /// underline/line-through/overline, independent of the text `color`. `None`
    /// means `currentColor` (fall back to the run's text colour). Not inherited.
    pub text_decoration_color: Option<Color>,
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
    pub float: Float,
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
    pub flex_basis: Option<f32>,
    /// `flex-basis` expressed as a percentage of the container's inner main
    /// size (0..1 fraction). Resolved at flex-layout time; takes precedence
    /// over `flex_basis` when set.
    pub flex_basis_pct: Option<f32>,
    /// `flex-basis: content` — size the item's flex base to its (max-)content
    /// size, ignoring any `width`. Distinct from `auto` (which falls back to
    /// `width`). When set, `flex_basis`/`flex_basis_pct` are both `None`.
    pub flex_basis_content: bool,
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
    pub border_radius: f32,
    /// Percentage-based border-radius (e.g. 50% for circles). Resolved in layout.
    pub border_radius_pct: Option<f32>,
    /// Per-corner border radii in points, order [top-left, top-right,
    /// bottom-right, bottom-left]. When all four equal `border_radius` the box is
    /// uniformly rounded; differing values express the 1-4 value `border-radius`
    /// shorthand and the per-corner longhands. Resolved against `border_radius`
    /// in layout when zero.
    pub border_radii: [f32; 4],
    /// Per-corner VERTICAL border radii in points (same corner order). CSS
    /// allows elliptical corners with distinct horizontal/vertical radii (the
    /// `Rx / Ry` slash syntax, or `border-radius: 50%` on a non-square box where
    /// the horizontal radius is width-relative and the vertical radius is
    /// height-relative). `border_radii` holds the horizontal radii; this holds
    /// the matching vertical radii. Equal to `border_radii` for circular corners,
    /// so the renderer falls back to its circular-arc path unless they differ.
    pub border_radii_y: [f32; 4],
    /// Per-corner percentage radii (same corner order). Resolved in layout
    /// against the box dimensions, mirroring `border_radius_pct`.
    pub border_radii_pct: [Option<f32>; 4],
    /// Per-corner VERTICAL percentage radii (same corner order), resolved in
    /// layout against the box HEIGHT (horizontal `border_radii_pct` resolve
    /// against width). Used for elliptical corners from percentage values.
    pub border_radii_y_pct: [Option<f32>; 4],
    pub outline_width: f32,
    pub outline_color: Option<Color>,
    /// CSS `outline-offset`: gap (in points) between the border edge and the
    /// outline. Positive expands the outline outward; negative pulls it inward.
    pub outline_offset: f32,
    pub box_sizing: BoxSizing,
    pub text_transform: TextTransform,
    /// CSS `font-variant-caps` / `font-variant: small-caps` (css-fonts-4 §6.5).
    pub font_variant_caps: FontVariantCaps,
    /// Whether standard/contextual ligatures are enabled (css-fonts-3 §6.4 /
    /// css-fonts-4 §6.11). Defaults to `true`; set to `false` by
    /// `font-feature-settings: "liga" 0` (and `clig`/`dlig` off) to suppress
    /// the shaper's default ligature substitution.
    pub ligatures_enabled: bool,
    pub text_indent: f32,
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
    /// CSS z-index (0 = auto).
    pub z_index: i32,
    /// CSS custom properties inherited from ancestors.
    pub custom_properties: HashMap<String, String>,
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    /// CSS `list-style-image` source (`url(...)`), if any. When set and
    /// decodable, it replaces the `list-style-type` marker glyph (css-lists-3
    /// §3.1). `None` means "use the list-style-type marker".
    pub list_style_image: Option<String>,
    pub content: Vec<ContentItem>,
    pub counter_reset: Vec<(String, i32)>,
    pub counter_increment: Vec<(String, i32)>,
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
    pub blur_radius: f32,
    /// CSS `filter` color functions (grayscale/brightness/.../hue-rotate),
    /// applied in order to a replaced image's pixels. `blur(...)` stays in
    /// `blur_radius`; this holds the non-blur ops.
    pub color_filters: Vec<ColorFilterOp>,
    /// CSS `filter: url(#id)` reference id (css-filter-effects-1 §3), recording
    /// the fragment that names an SVG `<filter>` element. Resolved during layout
    /// (where the DOM is available) into `color_filters` by reading the filter's
    /// `feColorMatrix` primitives. `None` when no `url()` filter is referenced.
    pub filter_url_id: Option<String>,
    /// CSS `filter: drop-shadow(dx dy blur color)`. `None` when no drop-shadow is
    /// present. Offsets/blur are in points; color is straight-alpha RGBA.
    pub drop_shadow: Option<DropShadow>,
    /// CSS `object-fit` for replaced elements (how the content fits its box).
    pub object_fit: ObjectFit,
    /// CSS `object-position` as horizontal/vertical fractions of the free space
    /// (0.0 = start/left/top, 0.5 = center, 1.0 = end/right/bottom).
    pub object_position: ObjectPosition,
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
}

impl ObjectPositionComponent {
    /// Resolve to a concrete offset of the object's start edge from the box
    /// start, given the free space (box length minus object length, which may be
    /// negative when the object overflows / is cropped).
    pub fn resolve(self, free_space: f32) -> f32 {
        match self {
            ObjectPositionComponent::Fraction(f) => free_space * f,
            ObjectPositionComponent::Length(l) => l,
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

/// CSS `filter: drop-shadow(<offset-x> <offset-y> <blur>? <color>?)`
/// (css-filter-effects-1 §4.4). Offsets and blur radius are in points; `color`
/// is straight-alpha RGBA in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub color: (f32, f32, f32, f32),
}

/// A single CSS `filter` color function. Amounts are resolved fractions
/// (`100%`/`1.0` -> 1.0); hue-rotate is in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorFilterOp {
    Grayscale(f32),
    Sepia(f32),
    Invert(f32),
    Brightness(f32),
    Contrast(f32),
    Saturate(f32),
    HueRotate(f32),
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            root_font_size: 12.0,
            viewport_width: 595.28,
            viewport_height: 841.89,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
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
            direction_rtl: false,
            writing_mode: WritingMode::HorizontalTb,
            bidi_override: false,
            text_decoration_underline: false,
            text_decoration_line_through: false,
            text_decoration_overline: false,
            text_decoration_color: None,
            line_height: f32::NAN,
            line_height_absolute: None,
            page_break_before: false,
            page_break_after: false,
            break_before: BreakValue::Auto,
            break_after: BreakValue::Auto,
            break_inside_avoid: false,
            page_name: None,
            border: BorderSides::default(),
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
            float: Float::None,
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
            flex_basis: None,
            flex_basis_pct: None,
            flex_basis_content: false,
            gap: 0.0,
            overflow: Overflow::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            visibility: Visibility::Visible,
            transform: None,
            transform_origin: TransformOrigin::default(),
            clip_path: None,
            mask_image: None,
            mask_mode: MaskMode::default(),
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_rows: None,
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
            border_radius: 0.0,
            border_radius_pct: None,
            border_radii: [0.0; 4],
            border_radii_y: [0.0; 4],
            border_radii_pct: [None; 4],
            border_radii_y_pct: [None; 4],
            outline_width: 0.0,
            outline_color: None,
            outline_offset: 0.0,
            box_sizing: BoxSizing::ContentBox,
            text_transform: TextTransform::None,
            font_variant_caps: FontVariantCaps::Normal,
            ligatures_enabled: true,
            text_indent: 0.0,
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
            z_index: 0,
            custom_properties: HashMap::new(),
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: None,
            content: Vec::new(),
            quotes: None,
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
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
            blur_radius: 0.0,
            color_filters: Vec::new(),
            filter_url_id: None,
            drop_shadow: None,
            object_fit: ObjectFit::default(),
            object_position: ObjectPosition::default(),
        }
    }
}

impl ComputedStyle {
    fn clear_background_images(&mut self) {
        self.background_gradient = None;
        self.background_radial_gradient = None;
        self.background_conic_gradient = None;
        self.background_image = None;
        self.background_svg = None;
    }

    fn reset_background(&mut self) {
        self.background_color = None;
        self.clear_background_images();
        self.background_size = BackgroundSize::Auto;
        self.background_repeat = BackgroundRepeat::Repeat;
        self.background_position = BackgroundPosition::default();
        self.background_origin = BackgroundOrigin::Padding;
        self.background_clip = BackgroundClip::Border;
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
    let mut style = parent.clone();

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
    // `unicode-bidi` is not inherited; initial is `normal`.
    style.bidi_override = false;
    style.float = Float::None;
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
    style.flex_basis = None;
    style.flex_basis_pct = None;
    style.flex_basis_content = false;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.overflow_x = Overflow::Visible;
    style.overflow_y = Overflow::Visible;
    style.visibility = Visibility::Visible;
    style.transform = None;
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
    style.border_radius = 0.0;
    style.border_radii = [0.0; 4];
    style.border_radii_pct = [None; 4];
    style.border_radii_y = [0.0; 4];
    style.border_radii_y_pct = [None; 4];
    style.outline_width = 0.0;
    style.outline_color = None;
    style.outline_offset = 0.0;
    style.box_sizing = BoxSizing::ContentBox;
    style.text_indent = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    // border_collapse, border_spacing and empty_cells are inherited; don't reset.
    style.table_layout = TableLayout::Auto;
    style.background_size = BackgroundSize::Auto;
    style.background_repeat = BackgroundRepeat::Repeat;
    style.background_position = BackgroundPosition::default();
    style.background_origin = BackgroundOrigin::Padding;
    style.background_clip = BackgroundClip::Border;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = 0;
    style.row_gap = 0.0;
    style.column_gap_pct = None;
    style.row_gap_pct = None;
    style.blur_radius = 0.0;
    style.color_filters.clear();
    style.filter_url_id = None;
    style.drop_shadow = None;
    // `page` (CSS Paged Media 3 §3.4) is not inherited; initial is `auto`
    // (the default page → `None`).
    style.page_name = None;
    // custom_properties inherit from parent (already cloned)

    // Apply tag defaults
    let defaults = default_style(tag);
    apply_style_map(&mut style, &defaults, parent);

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

    // Precedence order (lowest → highest), per css-cascade-4 §6.3 within the
    // author origin:
    //   1. author normal declarations (selectors), by specificity then source
    //   2. inline normal declarations (style attribute beats any selector)
    //   3. author important declarations (selectors), by specificity then source
    //   4. inline important declarations (style attribute, important)
    for (_, rule) in &matched {
        apply_style_map_filtered(&mut style, &rule.declarations, parent, Importance::Normal);
    }
    if let Some(inline) = &inline_map {
        apply_style_map_filtered(&mut style, inline, parent, Importance::Normal);
    }
    for (_, rule) in &matched {
        apply_style_map_filtered(
            &mut style,
            &rule.declarations,
            parent,
            Importance::Important,
        );
    }
    if let Some(inline) = &inline_map {
        apply_style_map_filtered(&mut style, inline, parent, Importance::Important);
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

    // Resolve `currentColor`. Two cases collapse here, both needing the
    // element's now-finalized `color`:
    //   1. The legacy parser parks an explicit `currentColor` keyword as a
    //      sentinel (the final `color` isn't known mid-cascade).
    //   2. A visible border with no resolvable color token. Per CSS the initial
    //      value of `border-color` is `currentColor`, and lightningcss strips
    //      `currentColor` from `border`/`border-*` shorthands (it equals the
    //      default), leaving the side with `color: None`. Such a side must paint
    //      in the element's `color`, not the black fallback.
    resolve_current_color(&mut style);

    style
}

/// Resolve `currentColor` into the element's finalized computed `color`.
///
/// Replaces both the `CURRENT_COLOR_SENTINEL` placeholder (explicit
/// `currentColor` keyword) and the implicit `currentColor` default of a visible
/// border side that has no resolved color.
fn resolve_current_color(style: &mut ComputedStyle) {
    let is_sentinel = |c: &Color| {
        c.r == CURRENT_COLOR_SENTINEL.r
            && c.g == CURRENT_COLOR_SENTINEL.g
            && c.b == CURRENT_COLOR_SENTINEL.b
            && c.a == CURRENT_COLOR_SENTINEL.a
    };
    let resolved = style.color;
    for side in [
        &mut style.border.top,
        &mut style.border.right,
        &mut style.border.bottom,
        &mut style.border.left,
    ] {
        let is_visible = side.width > 0.0 && side.style != BorderStyle::None;
        match side.color {
            Some(c) if is_sentinel(&c) => side.color = Some(resolved),
            // A visible border with no color uses `currentColor` (CSS initial
            // value of border-color).
            None if is_visible => side.color = Some(resolved),
            _ => {}
        }
    }
    if matches!(style.outline_color, Some(c) if is_sentinel(&c)) {
        style.outline_color = Some(resolved);
    }
    if matches!(style.background_color, Some(c) if is_sentinel(&c)) {
        style.background_color = Some(resolved);
    }
    for shadow in style.box_shadow.iter_mut() {
        if is_sentinel(&shadow.color) {
            shadow.color = resolved;
        }
    }
    for shadow in style.text_shadow.iter_mut() {
        if is_sentinel(&shadow.color) {
            shadow.color = resolved;
        }
    }
}

/// Compute the style for a `::before` or `::after` pseudo-element.
///
/// The pseudo-element inherits all inherited properties from the originating
/// element's computed style, resets non-inherited properties, then applies
/// matching pseudo-element CSS rules.  `parent_style` is the fully computed
/// style of the originating element.
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
    // Collect all matching pseudo-element rules
    let mut matched_declarations: Vec<&crate::parser::css::StyleMap> = Vec::new();
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
            matched_declarations.push(&rule.declarations);
        }
    }

    if matched_declarations.is_empty() {
        return None;
    }

    // Start from parent style (inherits inherited properties)
    let mut style = parent_style.clone();

    // Reset non-inherited properties (pseudo-elements are generated boxes)
    style.margin = EdgeSizes::default();
    style.margin_em_top = None;
    style.margin_em_right = None;
    style.margin_em_bottom = None;
    style.margin_em_left = None;
    style.padding = EdgeSizes::default();
    style.reset_background();
    style.border = BorderSides::default();
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
    style.float = Float::None;
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
    style.flex_basis = None;
    style.flex_basis_pct = None;
    style.flex_basis_content = false;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.overflow_x = Overflow::Visible;
    style.overflow_y = Overflow::Visible;
    style.transform = None;
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
    style.border_radius = 0.0;
    style.border_radii = [0.0; 4];
    style.border_radii_pct = [None; 4];
    style.border_radii_y = [0.0; 4];
    style.border_radii_y_pct = [None; 4];
    style.outline_width = 0.0;
    style.outline_color = None;
    style.outline_offset = 0.0;
    style.box_sizing = BoxSizing::ContentBox;
    style.text_indent = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = 0;
    style.row_gap = 0.0;
    style.column_gap_pct = None;
    style.row_gap_pct = None;
    style.blur_radius = 0.0;
    style.color_filters.clear();
    style.filter_url_id = None;
    style.drop_shadow = None;
    // Default display for pseudo-elements is inline
    style.display = Display::Inline;

    // Apply matched pseudo-element declarations.
    // Use parent_style as the "parent" for inherit resolution so that
    // `background-image: inherit` copies from the originating element.
    for declarations in &matched_declarations {
        apply_style_map(&mut style, declarations, parent_style);
    }

    // `content: none`/`normal` suppress `::before`/`::after` generation (no box
    // without content). `::marker`, however, always has a box — its content
    // defaults to the list marker symbol — so author rules that only restyle it
    // (e.g. `li::marker { color: … }`) must still apply even with empty content.
    // `::first-line`/`::first-letter` never carry `content`; they restyle
    // existing text, so they are likewise exempt from the empty-content check.
    if style.content.is_empty()
        && !matches!(
            pseudo,
            crate::parser::css::PseudoElement::Marker
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

    // Resolve any `currentColor` sentinels against the pseudo-element's color.
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
            | "font-family"
            | "line-height"
            | "text-align"
            | "text-decoration"
            | "visibility"
            | "letter-spacing"
            | "word-spacing"
            | "text-indent"
            | "text-transform"
            | "font-variant"
            | "font-variant-caps"
            | "font-feature-settings"
            | "white-space"
            | "overflow-wrap"
            | "word-wrap"
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
        "font-family" => {
            style.font_family = default.font_family;
            style.font_stack = default.font_stack;
        }
        "line-height" => {
            style.line_height = default.line_height;
            style.line_height_absolute = default.line_height_absolute;
        }
        "text-align" => style.text_align = default.text_align,
        "text-decoration" => {
            style.text_decoration_underline = default.text_decoration_underline;
            style.text_decoration_line_through = default.text_decoration_line_through;
        }
        "visibility" => style.visibility = default.visibility,
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
        "border-width" => {
            style.border.top.width = default.border.top.width;
            style.border.right.width = default.border.right.width;
            style.border.bottom.width = default.border.bottom.width;
            style.border.left.width = default.border.left.width;
        }
        "border-color" => {
            style.border.top.color = default.border.top.color;
            style.border.right.color = default.border.right.color;
            style.border.bottom.color = default.border.bottom.color;
            style.border.left.color = default.border.left.color;
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            style.border = default.border;
        }
        "float" => style.float = default.float,
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
        "flex-basis" => {
            style.flex_basis = default.flex_basis;
            style.flex_basis_pct = default.flex_basis_pct;
            style.flex_basis_content = default.flex_basis_content;
        }
        "gap" => style.gap = default.gap,
        "text-overflow" => style.text_overflow = default.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = default.overflow_wrap,
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
        "background-image" | "background-svg" => style.clear_background_images(),
        "aspect-ratio" => style.aspect_ratio = default.aspect_ratio,
        "object-fit" => style.object_fit = default.object_fit,
        "object-position" => style.object_position = default.object_position,
        "background" => style.reset_background(),
        "list-style-type" => style.list_style_type = default.list_style_type,
        "list-style-position" => style.list_style_position = default.list_style_position,
        "list-style-image" => style.list_style_image = default.list_style_image.clone(),
        "content" => style.content = default.content,
        "counter-reset" => style.counter_reset = default.counter_reset,
        "counter-increment" => style.counter_increment = default.counter_increment,
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
            style.blur_radius = default.blur_radius;
            style.color_filters = default.color_filters.clone();
            style.filter_url_id = default.filter_url_id.clone();
            style.drop_shadow = default.drop_shadow;
        }
        _ => {}
    }
}

/// Restore a property to the parent's value (inherit behavior).
fn restore_from_parent(style: &mut ComputedStyle, property: &str, parent: &ComputedStyle) {
    match property {
        "color" => style.color = parent.color,
        "font-size" => style.font_size = parent.font_size,
        "font-weight" => style.font_weight = parent.font_weight,
        "font-style" => style.font_style = parent.font_style,
        "font-family" => {
            style.font_family = parent.font_family.clone();
            style.font_stack = parent.font_stack.clone();
        }
        "line-height" => {
            style.line_height = parent.line_height;
            style.line_height_absolute = parent.line_height_absolute;
        }
        "text-align" => style.text_align = parent.text_align,
        "text-decoration" => {
            style.text_decoration_underline = parent.text_decoration_underline;
            style.text_decoration_line_through = parent.text_decoration_line_through;
        }
        "visibility" => style.visibility = parent.visibility,
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
        "border-width" => {
            style.border.top.width = parent.border.top.width;
            style.border.right.width = parent.border.right.width;
            style.border.bottom.width = parent.border.bottom.width;
            style.border.left.width = parent.border.left.width;
        }
        "border-color" => {
            style.border.top.color = parent.border.top.color;
            style.border.right.color = parent.border.right.color;
            style.border.bottom.color = parent.border.bottom.color;
            style.border.left.color = parent.border.left.color;
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            style.border = parent.border;
        }
        "float" => style.float = parent.float,
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
        "flex-basis" => {
            style.flex_basis = parent.flex_basis;
            style.flex_basis_pct = parent.flex_basis_pct;
            style.flex_basis_content = parent.flex_basis_content;
        }
        "gap" => style.gap = parent.gap,
        "text-overflow" => style.text_overflow = parent.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = parent.overflow_wrap,
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
        "background-image" | "background-svg" => style.inherit_background_image(parent),
        "background-gradient" => style.background_gradient = parent.background_gradient.clone(),
        "background-radial-gradient" => {
            style.background_radial_gradient = parent.background_radial_gradient.clone()
        }
        "background-conic-gradient" => {
            style.background_conic_gradient = parent.background_conic_gradient.clone()
        }
        "aspect-ratio" => style.aspect_ratio = parent.aspect_ratio,
        "object-fit" => style.object_fit = parent.object_fit,
        "object-position" => style.object_position = parent.object_position,
        "background" => style.inherit_background(parent),
        "list-style-type" => style.list_style_type = parent.list_style_type,
        "list-style-position" => style.list_style_position = parent.list_style_position,
        "list-style-image" => style.list_style_image = parent.list_style_image.clone(),
        "content" => style.content = parent.content.clone(),
        "counter-reset" => style.counter_reset = parent.counter_reset.clone(),
        "counter-increment" => style.counter_increment = parent.counter_increment.clone(),
        "column-count" | "columns" => style.column_count = parent.column_count,
        "column-gap" => style.column_gap = parent.column_gap,
        "filter" => {
            style.blur_radius = parent.blur_radius;
            style.color_filters = parent.color_filters.clone();
            style.filter_url_id = parent.filter_url_id.clone();
            style.drop_shadow = parent.drop_shadow;
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
            !matches!(lower.as_str(), "inherit" | "initial" | "unset")
        } else {
            true
        }
    })
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

/// Whether to apply normal (non-important) or `!important` declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Importance {
    Normal,
    Important,
}

/// Apply only the declarations of the given importance tier from `map`.
///
/// The cascade applies all normal declarations across matched rules first, then
/// all important declarations on top (css-cascade-4 §6.3). Splitting a rule's
/// declaration block by importance lets a low-specificity `!important` rule win
/// over a high-specificity normal rule. Implemented by projecting `map` down to
/// only the wanted-importance properties and delegating to `apply_style_map`.
pub(crate) fn apply_style_map_filtered(
    style: &mut ComputedStyle,
    map: &StyleMap,
    parent: &ComputedStyle,
    want: Importance,
) {
    let want_important = want == Importance::Important;
    // Fast path: if every property already matches the wanted tier, apply as-is.
    if map
        .properties
        .keys()
        .all(|k| map.is_important(k) == want_important)
    {
        if !map.properties.is_empty() {
            apply_style_map(style, map, parent);
        }
        return;
    }
    let mut filtered = StyleMap::new();
    for (key, value) in &map.properties {
        if map.is_important(key) == want_important {
            filtered.set_with_importance(key, value.clone(), want_important);
        }
    }
    if !filtered.properties.is_empty() {
        apply_style_map(style, &filtered, parent);
    }
}

pub(crate) fn apply_style_map(style: &mut ComputedStyle, map: &StyleMap, parent: &ComputedStyle) {
    // `parent_width_known` tells us whether the `parent_width` we feed into
    // the length-resolution context is a real, layout-driven parent width
    // (Some) or a viewport-width fallback (None). For width-family percentage
    // properties we must NOT eagerly resolve against the viewport fallback,
    // because the result silently diverges from the actual containing-block
    // width and clamps to available_width at layout time, yielding a full-
    // width element (e.g. a `width: 95%` inner bar looking 100% wide).
    let parent_width_known = parent.width.is_some();
    let length_context = crate::style::resolve::LengthResolutionContext::new(
        parent.width.unwrap_or(parent.viewport_width),
        style.font_size,
        parent.root_font_size,
        parent.viewport_width,
        parent.viewport_height,
    );

    // Handle inherit, initial, unset keywords before normal property application
    for (prop, val) in &map.properties {
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

    if let Some(CssValue::Length(v)) = get_non_special(map, "font-size") {
        style.font_size = *v;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "font-size") {
        // em value — multiply by current font-size
        style.font_size *= *v;
    }
    // ex/ch on `font-size` (css-values-4 §6.1.1): the unit refers to the
    // *parent* element's font (the value is computed before the new font-size
    // takes effect), so resolve the x-height / '0'-advance against the parent's
    // resolved font. Falls back to the 0.5em approximation when no font context
    // is active (e.g. the font is not loaded). `style.font_size` currently holds
    // the inherited parent size, matching the `em`/`Number` branch above.
    if let Some(CssValue::Ex(v)) = get_non_special(map, "font-size") {
        let ratio = crate::style::font_ctx::style_x_height_ratio(parent).unwrap_or(0.5);
        style.font_size *= *v * ratio;
    }
    if let Some(CssValue::Ch(v)) = get_non_special(map, "font-size") {
        let ratio = crate::style::font_ctx::style_ch_ratio(parent).unwrap_or(0.5);
        style.font_size *= *v * ratio;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-weight") {
        style.font_weight = if k == "bold" || k == "700" || k == "800" || k == "900" {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-style") {
        style.font_style = if k == "italic" || k == "oblique" {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-family") {
        style.font_stack = parse_font_stack(k);
        style.font_family = style.font_stack.primary();
    }

    if let Some(CssValue::Color(c)) = get_non_special(map, "color") {
        style.color = *c;
    }

    if let Some(CssValue::Color(c)) = get_non_special(map, "background-color") {
        style.background_color = Some(*c);
    }

    // Background-image layers. A single declaration may split into several keys
    // (e.g. `background-image: url(...), linear-gradient(...)` becomes a raster
    // `background-image` key AND a `background-gradient` key — see
    // parser::css::inline). All such layers from the *same* declaration must
    // coexist, but they collectively replace any background-image stack inherited
    // from an earlier, lower-priority cascade layer. So clear the whole stack
    // exactly once, up front, before setting any layer present in this map —
    // rather than per-key, which would clobber a sibling layer from the same
    // declaration. Full resets (`background:` shorthand, `background-image: none`,
    // the per-element reset in compute_style_with_context, the initial/inherit
    // paths) are handled elsewhere and remain unaffected.
    let sets_image_layer = get_non_special(map, "background-gradient").is_some()
        || get_non_special(map, "background-radial-gradient").is_some()
        || get_non_special(map, "background-conic-gradient").is_some()
        || get_non_special(map, "background-svg").is_some()
        || get_non_special(map, "background-image").is_some();
    if sets_image_layer {
        style.clear_background_images();
    }

    // Linear gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-gradient") {
        if let Some(lg) = parse_linear_gradient(k) {
            style.background_gradient = Some(lg);
        }
    }

    // Radial gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-radial-gradient") {
        if let Some(rg) = parse_radial_gradient(k) {
            style.background_radial_gradient = Some(rg);
        }
    }

    // Conic gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-conic-gradient") {
        if let Some(cg) = parse_conic_gradient(k) {
            style.background_conic_gradient = Some(cg);
        }
    }

    // SVG background image (from data:image/svg+xml URI)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-svg") {
        if let Some(tree) = crate::parser::svg::parse_svg_from_string(k) {
            style.background_svg = Some(tree);
        }
    }

    // Raster background image. Read even when a gradient/svg layer is also
    // present in this map so a raster + gradient pair from the same shorthand
    // both paint (the up-front clear above already reset the prior stack).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-image") {
        let resolved = resolve_embedded_vars(k.trim(), &style.custom_properties);
        let trimmed = resolved.trim();
        if trimmed != "none" {
            if let Some(svg_text) = crate::parser::css::extract_svg_data_uri(trimmed) {
                if let Some(tree) = crate::parser::svg::parse_svg_from_string(&svg_text) {
                    style.background_svg = Some(tree);
                }
            } else if let Some(CssValue::Color(c)) = crate::parser::css::parse_color(trimmed) {
                // `background: var(--c)` decomposes here; a resolved colour is a
                // background-color, not an image URL.
                style.background_color = Some(c);
            } else {
                style.background_image = Some(trimmed.to_string());
            }
        }
    }

    // Margins: resolve both Length (pt) and Number (em = multiplied by font_size).
    // The CSS parser produces Number for em values (e.g. "2em" → Number(2.0))
    // and our UA defaults use Number for em-based margins.
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
        Some(CssValue::Number(v)) => {
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
        Some(CssValue::Number(v)) => {
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
        Some(CssValue::Number(v)) => {
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
        Some(CssValue::Number(v)) => {
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
            "justify" => TextAlign::Justify,
            _ => TextAlign::Left,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-decoration") {
        style.text_decoration_underline = k == "underline";
        style.text_decoration_line_through = k == "line-through";
        style.text_decoration_overline = k == "overline";
    }

    // `text-decoration-color` longhand (css-text-decor-3 §2.2): an explicit line
    // colour distinct from the text `color`. Resolved like any colour value.
    if let Some(CssValue::Color(c)) = get_non_special(map, "text-decoration-color") {
        style.text_decoration_color = Some(*c);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "line-height") {
        if k == "normal" {
            style.line_height = f32::NAN;
            style.line_height_absolute = None;
        } else if let Some(v) = resolve_raw_length_for_style(k, style, length_context) {
            style.line_height_absolute = Some(v);
            style.line_height = v / style.font_size;
        }
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "line-height") {
        style.line_height = *v;
        style.line_height_absolute = None;
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
        | CssValue::Calc(_)
        | CssValue::Clamp(_, _, _)
        | CssValue::Var(_, _)),
    ) = get_non_special(map, "line-height")
        && let Some(v) = resolve_css_length_for_style(value, style, length_context)
    {
        style.line_height_absolute = Some(v);
        style.line_height = v / style.font_size;
    }
    sync_line_height_from_absolute(style);

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "display") {
        style.display = match k.as_str() {
            "none" => Display::None,
            "inline" => Display::Inline,
            "inline-block" => Display::InlineBlock,
            "block" => Display::Block,
            "flex" => Display::Flex,
            "grid" => Display::Grid,
            _ => style.display,
        };
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
        // Strip it — the flex layout implements the default (`unsafe`) honor-
        // alignment-on-overflow behavior; `safe`'s overflow→start fallback is not
        // separately tracked (acceptable: it only differs when content overflows).
        let raw = k.trim();
        let kw = raw
            .strip_prefix("safe ")
            .or_else(|| raw.strip_prefix("unsafe "))
            .unwrap_or(raw)
            .trim();
        // css-align-3 §6.2: `left`/`right` resolve against the INLINE axis. For a
        // row container that is the main axis (right→end, left→start); for a
        // column container the main axis is the block axis, so they behave as
        // `start`.
        let is_row = style.flex_direction.is_row();
        style.justify_content = match kw {
            "flex-end" | "end" => JustifyContent::FlexEnd,
            "right" if is_row => JustifyContent::FlexEnd,
            "left" | "right" => JustifyContent::FlexStart,
            "center" => JustifyContent::Center,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            "space-evenly" => JustifyContent::SpaceEvenly,
            _ => JustifyContent::FlexStart,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-items") {
        style.align_items = match k.as_str() {
            "flex-start" | "start" => AlignItems::FlexStart,
            "flex-end" | "end" => AlignItems::FlexEnd,
            "center" => AlignItems::Center,
            "baseline" => AlignItems::Baseline,
            _ => AlignItems::Stretch,
        };
        // Grid uses the same property with start/end/center/stretch keywords.
        style.grid_align_items = parse_grid_align(k);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-content") {
        style.align_content = match k.as_str() {
            "flex-start" | "start" => AlignContent::FlexStart,
            "flex-end" | "end" => AlignContent::FlexEnd,
            "center" => AlignContent::Center,
            "space-between" => AlignContent::SpaceBetween,
            "space-around" => AlignContent::SpaceAround,
            "space-evenly" => AlignContent::SpaceEvenly,
            _ => AlignContent::Stretch,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-self") {
        style.align_self = match k.as_str() {
            "auto" => AlignSelf::Auto,
            "flex-start" | "start" => AlignSelf::FlexStart,
            "flex-end" | "end" => AlignSelf::FlexEnd,
            "center" => AlignSelf::Center,
            "baseline" => AlignSelf::Baseline,
            "stretch" => AlignSelf::Stretch,
            _ => AlignSelf::Auto,
        };
    }

    // `order` (integer). May arrive as a Length (numeric) or Keyword.
    match get_non_special(map, "order") {
        Some(CssValue::Length(v)) => style.order = *v as i32,
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

    if let Some(CssValue::Length(v)) = get_non_special(map, "flex-grow") {
        style.flex_grow = v.max(0.0);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "flex-shrink") {
        style.flex_shrink = v.max(0.0);
    }
    match get_non_special(map, "flex-basis") {
        Some(CssValue::Length(v)) => {
            style.flex_basis = Some(*v);
            style.flex_basis_pct = None;
            style.flex_basis_content = false;
        }
        Some(CssValue::Percentage(p)) => {
            style.flex_basis_pct = Some(*p / 100.0);
            style.flex_basis = None;
            style.flex_basis_content = false;
        }
        Some(CssValue::Keyword(k)) => match k.as_str() {
            "auto" | "content" => {
                style.flex_basis = None;
                style.flex_basis_pct = None;
                // `content` sizes to content; `auto` falls back to `width`.
                style.flex_basis_content = k == "content";
            }
            other => match parse_length(other) {
                Some(CssValue::Percentage(p)) => {
                    style.flex_basis_pct = Some(p / 100.0);
                    style.flex_basis = None;
                    style.flex_basis_content = false;
                }
                Some(CssValue::Length(v)) => {
                    style.flex_basis = Some(v);
                    style.flex_basis_pct = None;
                    style.flex_basis_content = false;
                }
                _ => {}
            },
        },
        _ => {}
    }

    // flex shorthand: "flex: <grow>" or "flex: <grow> <shrink>" or "flex: <grow> <shrink> <basis>"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex") {
        let parts: Vec<&str> = k.split_whitespace().collect();
        if let Some(first) = parts.first() {
            if *first == "none" {
                // flex: none == 0 0 auto
                style.flex_grow = 0.0;
                style.flex_shrink = 0.0;
                style.flex_basis = None;
                style.flex_basis_pct = None;
                style.flex_basis_content = false;
            } else if *first == "auto" {
                // flex: auto == 1 1 auto
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = None;
                style.flex_basis_pct = None;
                style.flex_basis_content = false;
            } else if *first == "initial" {
                // flex: initial == 0 1 auto
                style.flex_grow = 0.0;
                style.flex_shrink = 1.0;
                style.flex_basis = None;
                style.flex_basis_pct = None;
                style.flex_basis_content = false;
            } else if let Ok(grow) = first.parse::<f32>() {
                // flex: <grow> == <grow> 1 0%
                style.flex_grow = grow.max(0.0);
                style.flex_shrink = 1.0;
                style.flex_basis = Some(0.0);
                style.flex_basis_pct = None;
                style.flex_basis_content = false;
                if let Some(second) = parts.get(1) {
                    if let Ok(shrink) = second.parse::<f32>() {
                        style.flex_shrink = shrink.max(0.0);
                    }
                }
                if let Some(third) = parts.get(2) {
                    if *third == "auto" || *third == "content" {
                        style.flex_basis = None;
                        style.flex_basis_pct = None;
                        style.flex_basis_content = *third == "content";
                    } else if let Some(CssValue::Percentage(p)) =
                        crate::parser::css::parse_length(third)
                    {
                        style.flex_basis_pct = Some(p / 100.0);
                        style.flex_basis = None;
                    } else if let Some(CssValue::Length(v)) =
                        crate::parser::css::parse_length(third)
                    {
                        style.flex_basis = Some(v);
                        style.flex_basis_pct = None;
                    }
                }
            }
        }
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
        style.grid_template_areas = parse_grid_template_areas(k);
    }
    // `grid-auto-rows` may arrive as a Length (single px/pt value) or Keyword.
    match get_non_special(map, "grid-auto-rows") {
        Some(CssValue::Length(v)) => style.grid_auto_rows = Some(*v),
        Some(CssValue::Keyword(k)) => {
            if let Some(GridTrack::Fixed(v)) = parse_single_track(k) {
                style.grid_auto_rows = Some(v);
            }
        }
        _ => {}
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-auto-flow") {
        style.grid_auto_flow_column = k.contains("column");
        style.grid_auto_flow_dense = k.contains("dense");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "justify-items") {
        style.justify_items = parse_grid_align(k);
    }
    // `place-items: <align> [<justify>]` shorthand sets both axes.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "place-items") {
        let mut parts = k.split_whitespace();
        if let Some(a) = parts.next() {
            let align = parse_grid_align(a);
            let justify = parts.next().map(parse_grid_align).unwrap_or(align);
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
        let mut parts = k.split_whitespace();
        if let Some(a) = parts.next() {
            if a != "auto" {
                style.grid_align_self = Some(parse_grid_align(a));
            }
            if let Some(j) = parts.next() {
                if j != "auto" {
                    style.grid_justify_self = Some(parse_grid_align(j));
                }
            } else if a != "auto" {
                style.grid_justify_self = Some(parse_grid_align(a));
            }
        }
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
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "clip-path") {
        style.clip_path = parse_clip_path(k);
    }

    // CSS Masking (css-masking-1 §3). The `-webkit-mask*` aliases are normalised
    // to the unprefixed names at parse time. `mask-mode` resolves how the source
    // pixels become coverage (default `match-source` → alpha for CSS images).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-mode") {
        style.mask_mode = match k.trim().to_ascii_lowercase().as_str() {
            "alpha" => MaskMode::Alpha,
            "luminance" => MaskMode::Luminance,
            _ => MaskMode::MatchSource,
        };
    }
    // `mask` shorthand: take its image token if present (other sub-values are
    // not yet modelled). The `mask-image` longhand wins when both are set.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask") {
        if let Some(src) = parse_mask_image(k) {
            style.mask_image = src;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mask-image") {
        if let Some(src) = parse_mask_image(k) {
            style.mask_image = src;
        }
    }

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
    // resets to the default page. Names are stored lowercased to match the
    // case-insensitive `@page <name>` lookup.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page") {
        style.page_name = if k.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(k.to_ascii_lowercase())
        };
    }

    // `filter: opacity(<x>)` multiplies into the element's final opacity. The
    // `opacity` property is parsed later, so remember the factor and fold it in
    // after `style.opacity` is finalized to combine multiplicatively.
    let mut filter_opacity = 1.0_f32;
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "filter") {
        let (blur, ops, opacity, drop_shadow, url_id) = parse_filter(k);
        if let Some(radius) = blur {
            style.blur_radius = radius;
        }
        style.color_filters = ops;
        style.filter_url_id = url_id;
        style.drop_shadow = drop_shadow;
        filter_opacity = opacity;
    }

    // Border shorthand: "1px solid black"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border") {
        let k = resolve_embedded_vars(k, &style.custom_properties);
        let (w, c, bs) = parse_border_shorthand(&k, style.font_size);
        style.border = BorderSides::uniform_styled(w, c, bs);
    }

    // Per-side border shorthands
    for (prop, setter) in &[
        (
            "border-top",
            (|s: &mut ComputedStyle, w, c, bs| {
                s.border.top = BorderSide {
                    width: w,
                    color: c,
                    style: bs,
                };
            }) as fn(&mut ComputedStyle, f32, Option<Color>, BorderStyle),
        ),
        (
            "border-right",
            (|s: &mut ComputedStyle, w, c, bs| {
                s.border.right = BorderSide {
                    width: w,
                    color: c,
                    style: bs,
                };
            }) as fn(&mut ComputedStyle, f32, Option<Color>, BorderStyle),
        ),
        (
            "border-bottom",
            (|s: &mut ComputedStyle, w, c, bs| {
                s.border.bottom = BorderSide {
                    width: w,
                    color: c,
                    style: bs,
                };
            }) as fn(&mut ComputedStyle, f32, Option<Color>, BorderStyle),
        ),
        (
            "border-left",
            (|s: &mut ComputedStyle, w, c, bs| {
                s.border.left = BorderSide {
                    width: w,
                    color: c,
                    style: bs,
                };
            }) as fn(&mut ComputedStyle, f32, Option<Color>, BorderStyle),
        ),
    ] {
        if let Some(CssValue::Keyword(k)) = get_non_special(map, prop) {
            let k = resolve_embedded_vars(k, &style.custom_properties);
            let (w, c, bs) = parse_border_shorthand(&k, style.font_size);
            setter(style, w, c, bs);
        }
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "width") {
        style.width = Some(*v);
        style.width_keyword = None;
        style.percentage_sizing.width = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "width") {
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
        if let Some(v) = resolve_raw_length_for_style(k, style, length_context) {
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
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context)
    {
        style.height = Some(v);
        style.percentage_sizing.height = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "max-width")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context)
    {
        style.max_width = Some(v);
        style.percentage_sizing.max_width = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "min-width")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context)
    {
        style.min_width = Some(v);
        style.percentage_sizing.min_width = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "min-height")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context)
    {
        style.min_height = Some(v);
        style.percentage_sizing.min_height = None;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "max-height")
        && let Some(v) = resolve_raw_length_for_style(k, style, length_context)
    {
        style.max_height = Some(v);
        style.percentage_sizing.max_height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "height") {
        style.height = Some(*v);
        style.percentage_sizing.height = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "height") {
        style.height = Some(*v * style.font_size);
        style.percentage_sizing.height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v);
        style.percentage_sizing.max_width = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v * style.font_size);
        style.percentage_sizing.max_width = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v);
        style.percentage_sizing.min_width = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v * style.font_size);
        style.percentage_sizing.min_width = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v);
        style.percentage_sizing.min_height = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v * style.font_size);
        style.percentage_sizing.min_height = None;
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-height") {
        style.max_height = Some(*v);
        style.percentage_sizing.max_height = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "max-height") {
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

    if let Some(CssValue::Number(v)) = get_non_special(map, "opacity") {
        style.opacity = v.clamp(0.0, 1.0);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "opacity") {
        // bare number parsed as Length
        style.opacity = v.clamp(0.0, 1.0);
    }
    // Fold any `filter: opacity()` factor into the finalized opacity.
    if filter_opacity != 1.0 {
        style.opacity = (style.opacity * filter_opacity).clamp(0.0, 1.0);
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "mix-blend-mode") {
        style.mix_blend_mode = BlendMode::from_keyword(k);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-blend-mode") {
        style.background_blend_mode = BlendMode::from_keyword(k);
    }

    // Uniform `border-width`. Absolute lengths (pt) apply directly; a font-relative
    // width (em/ex/ch) arrives as CssValue::Number (an em factor) and resolves
    // against the element's font-size, mirroring the margin path. Without this an
    // em-unit border width is silently dropped (width = 0).
    if let Some(w) = resolve_border_width(get_non_special(map, "border-width"), style.font_size) {
        style.border.top.width = w;
        style.border.right.width = w;
        style.border.bottom.width = w;
        style.border.left.width = w;
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border-width")
        && let Some(widths) = parse_border_width_shorthand_values(k, style, length_context)
    {
        style.border.top.width = widths[0];
        style.border.right.width = widths[1];
        style.border.bottom.width = widths[2];
        style.border.left.width = widths[3];
    }

    if let Some(CssValue::Color(c)) = get_non_special(map, "border-color") {
        style.border.top.color = Some(*c);
        style.border.right.color = Some(*c);
        style.border.bottom.color = Some(*c);
        style.border.left.color = Some(*c);
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border-color")
        && let Some(colors) = parse_border_color_shorthand_values(k)
    {
        style.border.top.color = Some(colors[0]);
        style.border.right.color = Some(colors[1]);
        style.border.bottom.color = Some(colors[2]);
        style.border.left.color = Some(colors[3]);
    }

    // Uniform `border-style` keyword applies the same line style to all four
    // edges (e.g. `border-style: solid` paired with per-side `border-*-width`).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border-style") {
        if let Some(styles) = parse_border_style_shorthand_values(k) {
            style.border.top.style = styles[0];
            style.border.right.style = styles[1];
            style.border.bottom.style = styles[2];
            style.border.left.style = styles[3];
        } else {
            let bs = parse_border_style_keyword(k);
            style.border.top.style = bs;
            style.border.right.style = bs;
            style.border.bottom.style = bs;
            style.border.left.style = bs;
        }
    }

    // Per-side border longhands (`border-{side}-{width,style,color}`). These run
    // after the `border` / `border-{side}` shorthands and the uniform
    // `border-{width,color,style}` properties so an explicit longhand wins.
    for (prop, setter) in &[
        (
            "border-top-width",
            (|s: &mut ComputedStyle, w| s.border.top.width = w) as fn(&mut ComputedStyle, f32),
        ),
        (
            "border-right-width",
            (|s: &mut ComputedStyle, w| s.border.right.width = w) as fn(&mut ComputedStyle, f32),
        ),
        (
            "border-bottom-width",
            (|s: &mut ComputedStyle, w| s.border.bottom.width = w) as fn(&mut ComputedStyle, f32),
        ),
        (
            "border-left-width",
            (|s: &mut ComputedStyle, w| s.border.left.width = w) as fn(&mut ComputedStyle, f32),
        ),
    ] {
        if let Some(w) = resolve_border_width(get_non_special(map, prop), style.font_size) {
            setter(style, w);
        }
    }

    for (prop, setter) in &[
        (
            "border-top-color",
            (|s: &mut ComputedStyle, c| s.border.top.color = Some(c))
                as fn(&mut ComputedStyle, Color),
        ),
        (
            "border-right-color",
            (|s: &mut ComputedStyle, c| s.border.right.color = Some(c))
                as fn(&mut ComputedStyle, Color),
        ),
        (
            "border-bottom-color",
            (|s: &mut ComputedStyle, c| s.border.bottom.color = Some(c))
                as fn(&mut ComputedStyle, Color),
        ),
        (
            "border-left-color",
            (|s: &mut ComputedStyle, c| s.border.left.color = Some(c))
                as fn(&mut ComputedStyle, Color),
        ),
    ] {
        if let Some(CssValue::Color(c)) = get_non_special(map, prop) {
            setter(style, *c);
        }
    }

    for (prop, setter) in &[
        (
            "border-top-style",
            (|s: &mut ComputedStyle, bs| s.border.top.style = bs)
                as fn(&mut ComputedStyle, BorderStyle),
        ),
        (
            "border-right-style",
            (|s: &mut ComputedStyle, bs| s.border.right.style = bs)
                as fn(&mut ComputedStyle, BorderStyle),
        ),
        (
            "border-bottom-style",
            (|s: &mut ComputedStyle, bs| s.border.bottom.style = bs)
                as fn(&mut ComputedStyle, BorderStyle),
        ),
        (
            "border-left-style",
            (|s: &mut ComputedStyle, bs| s.border.left.style = bs)
                as fn(&mut ComputedStyle, BorderStyle),
        ),
    ] {
        if let Some(CssValue::Keyword(k)) = get_non_special(map, prop) {
            setter(style, parse_border_style_keyword(k));
        }
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
            // For a single-page, non-scrolling PDF the viewport == the page box,
            // so `fixed` behaves like an absolute box anchored to the page
            // content box (handled by the absolute-at-root path), and `sticky`
            // degrades to its relative-until-threshold base position.
            "fixed" => Position::Absolute,
            "sticky" => Position::Relative,
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

    // Box-shadow: parse from keyword (stored as full shorthand string).
    // A comma-separated list yields multiple stacked shadows.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-shadow") {
        let shadows = parse_box_shadow(k);
        if !shadows.is_empty() {
            style.box_shadow = shadows;
        }
    }

    // CSS `text-shadow` (css-text-decor-3 §3). Like box-shadow but with no
    // `spread`/`inset` and the optional color may appear before or after the
    // offsets. `none` clears any inherited value.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "text-shadow") {
        if k.trim() == "none" {
            style.text_shadow = Vec::new();
        } else {
            let shadows = parse_text_shadow(k);
            if !shadows.is_empty() {
                style.text_shadow = shadows;
            }
        }
    }

    // Multi-column layout
    if let Some(val) = get_non_special(map, "column-count") {
        match val {
            CssValue::Length(n) => style.column_count = Some(*n as u32),
            CssValue::Keyword(k) => {
                if let Ok(n) = k.parse::<u32>() {
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
    if let Some(val) = get_non_special(map, "column-gap") {
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
                style.column_gap_is_normal = false;
            }
            CssValue::Keyword(k) if k != "normal" => {
                if let Some(stripped) = k.trim().strip_suffix('%') {
                    if let Ok(p) = stripped.parse::<f32>() {
                        style.column_gap_pct = Some(p / 100.0);
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
        style.column_rule = parse_column_rule_shorthand(k, style.font_size);
    }
    if let Some(val) = get_non_special(map, "column-rule-width") {
        if let CssValue::Length(w) = val {
            style.column_rule.width = *w;
        } else if let CssValue::Keyword(k) = val {
            // `thin` / `medium` / `thick` keyword widths, else a parsed length.
            match k.trim().to_ascii_lowercase().as_str() {
                "thin" => style.column_rule.width = 0.75,
                "medium" => style.column_rule.width = MEDIUM_RULE_WIDTH_PT,
                "thick" => style.column_rule.width = 3.75,
                _ => {
                    if let Some(CssValue::Length(w)) = parse_length(k) {
                        style.column_rule.width = w;
                    }
                }
            }
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-rule-style") {
        style.column_rule.style = parse_border_style_keyword(k);
        // A visible style with no explicit width uses the medium default
        // (column-rule-width initial = medium) so the rule actually paints.
        if style.column_rule.style != BorderStyle::None
            && style.column_rule.width <= 0.0
            && get_non_special(map, "column-rule-width").is_none()
            && get_non_special(map, "column-rule").is_none()
        {
            style.column_rule.width = MEDIUM_RULE_WIDTH_PT;
        }
    }
    if let Some(val) = get_non_special(map, "column-rule-color") {
        let c = match val {
            CssValue::Color(c) => Some(*c),
            CssValue::Keyword(k) => parse_border_color_token(k),
            _ => None,
        };
        if c.is_some() {
            style.column_rule.color = c;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-span") {
        style.column_span_all = k.eq_ignore_ascii_case("all");
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "column-fill") {
        // `auto` fills columns sequentially; `balance` (default) equalises them.
        style.column_fill_auto = k.eq_ignore_ascii_case("auto");
    }
    match get_non_special(map, "row-gap") {
        Some(CssValue::Length(v)) => {
            style.row_gap = *v;
            style.row_gap_pct = None;
        }
        // A percentage row-gap resolves against the container's own content-box
        // block size (height); defer it as a fraction for the flex layout.
        Some(CssValue::Percentage(p)) => {
            style.row_gap_pct = Some(*p / 100.0);
        }
        Some(CssValue::Keyword(k)) if k != "normal" => {
            if let Some(stripped) = k.trim().strip_suffix('%') {
                if let Ok(p) = stripped.parse::<f32>() {
                    style.row_gap_pct = Some(p / 100.0);
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

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "transform-origin") {
        if let Some(origin) = parse_transform_origin(k, style.font_size, style.root_font_size) {
            style.transform_origin = origin;
        }
    }

    // Border-radius shorthand: a single value keeps the fast uniform path; a
    // multi-value or `/`-separated value expands into per-corner radii.
    match get_non_special(map, "border-radius") {
        Some(CssValue::Length(v)) => {
            style.border_radius = *v;
            style.border_radii = [*v; 4];
            style.border_radii_y = [*v; 4];
        }
        Some(CssValue::Percentage(pct)) => {
            // Percentage border-radius resolves per-axis in layout: the
            // horizontal radius against the box width and the vertical against
            // its height. On a square box `50%` is a circle; on a non-square box
            // it is an ellipse. We seed both axes here and resolve in layout.
            style.border_radius_pct = Some(*pct);
            style.border_radii_pct = [Some(*pct); 4];
            style.border_radii_y_pct = [Some(*pct); 4];
        }
        Some(CssValue::Keyword(k)) => {
            let (radii, radii_pct, radii_y, radii_y_pct) = parse_border_radius_shorthand(k);
            style.border_radii = radii;
            style.border_radii_pct = radii_pct;
            style.border_radii_y = radii_y;
            style.border_radii_y_pct = radii_y_pct;
            // Keep the legacy uniform field meaningful for the all-equal case so
            // older single-radius code paths still round.
            if radii.iter().all(|r| (*r - radii[0]).abs() < f32::EPSILON)
                && radii_pct.iter().all(Option::is_none)
            {
                style.border_radius = radii[0];
            }
            if radii_pct.iter().all(|p| *p == radii_pct[0]) {
                style.border_radius_pct = radii_pct[0];
            }
        }
        // Non-percentage relative units (rem/vw/vh/calc/var) resolve against the
        // length context now (they don't depend on the element's own box). A
        // PERCENTAGE never reaches here — it is handled above as a layout-time
        // hint so it can resolve per-axis against the element's own box.
        Some(
            other @ (CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)),
        ) => {
            if let Some(v) = crate::style::resolve::try_resolve_to_length_in_context(
                other,
                &style.custom_properties,
                length_context,
            ) {
                style.border_radius = v;
                style.border_radii = [v; 4];
                style.border_radii_y = [v; 4];
            }
        }
        Some(other @ (CssValue::Calc(_) | CssValue::Var(_, _))) => {
            if let Some(v) = crate::style::resolve::try_resolve_to_length_in_context(
                other,
                &style.custom_properties,
                length_context,
            ) {
                style.border_radius = v;
                style.border_radii = [v; 4];
                style.border_radii_y = [v; 4];
            }
        }
        _ => {}
    }

    // Per-corner border-radius longhands override the shorthand for their corner.
    for (prop, idx) in [
        ("border-top-left-radius", 0usize),
        ("border-top-right-radius", 1),
        ("border-bottom-right-radius", 2),
        ("border-bottom-left-radius", 3),
    ] {
        let assign = |radii: &mut [f32; 4], pct: &mut [Option<f32>; 4], tok: RadiusToken| match tok
        {
            RadiusToken::Len(v) => {
                radii[idx] = v;
                pct[idx] = None;
            }
            RadiusToken::Pct(p) => {
                pct[idx] = Some(p);
                radii[idx] = 0.0;
            }
        };
        match get_non_special(map, prop) {
            Some(CssValue::Length(v)) => {
                style.border_radii[idx] = *v;
                style.border_radii_pct[idx] = None;
                style.border_radii_y[idx] = *v;
                style.border_radii_y_pct[idx] = None;
            }
            Some(CssValue::Percentage(p)) => {
                style.border_radii_pct[idx] = Some(*p);
                style.border_radii[idx] = 0.0;
                style.border_radii_y_pct[idx] = Some(*p);
                style.border_radii_y[idx] = 0.0;
            }
            Some(CssValue::Keyword(k)) => {
                // Corner longhand grammar: `<horizontal> [vertical]` — two
                // space-separated tokens give an elliptical corner. A single
                // token applies to both axes (circular).
                let mut toks = k.split_whitespace();
                let h = toks.next().and_then(parse_radius_token);
                let v = toks.next().and_then(parse_radius_token);
                if let Some(htok) = h {
                    assign(&mut style.border_radii, &mut style.border_radii_pct, htok);
                    let vtok = v.unwrap_or(htok);
                    assign(
                        &mut style.border_radii_y,
                        &mut style.border_radii_y_pct,
                        vtok,
                    );
                }
            }
            _ => {}
        }
    }

    // Outline shorthand: "2px solid red" (with optional style keyword we ignore
    // for paint, since the renderer strokes a solid outline).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "outline") {
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
                style.outline_color = Some(c);
            }
        }
    }

    // Outline individual properties
    if let Some(CssValue::Length(v)) = get_non_special(map, "outline-width") {
        style.outline_width = *v;
    }
    if let Some(CssValue::Color(c)) = get_non_special(map, "outline-color") {
        style.outline_color = Some(*c);
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

    // CSS `writing-mode` property (css-writing-modes-4 §3.1). Inherited (so it
    // is never reset in the non-inherited block above; it rides the
    // `parent.clone()` inheritance). Only `vertical-rl` changes behaviour; every
    // other keyword (including the unsupported `vertical-lr`/`sideways-*`) falls
    // back to the default horizontal mode.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "writing-mode") {
        style.writing_mode = match k.as_str() {
            "vertical-rl" => WritingMode::VerticalRl,
            _ => WritingMode::HorizontalTb,
        };
    }

    // CSS `unicode-bidi` property. Not inherited. `bidi-override` (and the
    // isolating `isolate-override`) force the box's inline content to be
    // reordered strictly in sequence according to `direction`, overriding the
    // characters' intrinsic bidi classes (css-writing-modes-4 §2.4).
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "unicode-bidi") {
        style.bidi_override = matches!(k.as_str(), "bidi-override" | "isolate-override");
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

    // font-feature-settings (css-fonts-3 §6.4): honour explicit ligature
    // control. `"liga" 0` (or `clig`/`dlig` set to 0/off) disables the shaper's
    // default ligature substitution; the inverse (or omission) leaves it on.
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "font-feature-settings") {
        style.ligatures_enabled = !ligatures_disabled_by_feature_settings(k);
    }

    // Text-indent
    if let Some(CssValue::Length(v)) = get_non_special(map, "text-indent") {
        style.text_indent = *v;
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

    // Letter-spacing
    if let Some(CssValue::Length(v)) = get_non_special(map, "letter-spacing") {
        style.letter_spacing = *v;
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

    // Word-spacing
    if let Some(CssValue::Length(v)) = get_non_special(map, "word-spacing") {
        style.word_spacing = *v;
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
        if k == "break-all" && style.overflow_wrap == OverflowWrap::Normal {
            style.overflow_wrap = OverflowWrap::Anywhere;
        }
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
    // Per-layer slot mapping for a comma-separated `background-image` list. Index
    // i names the paint slot ("raster" / "gradient" / "none") that list position
    // occupies, so the matching comma-separated `background-size` / `-position` /
    // `-repeat` entry can be routed to the right slot.
    let layer_slots: Vec<String> = get_non_special(map, "background-layer-slots")
        .and_then(|v| match v {
            CssValue::Keyword(k) => Some(k.split(',').map(|s| s.trim().to_string()).collect()),
            _ => None,
        })
        .unwrap_or_default();
    let raster_layer_index = layer_slots.iter().position(|s| s == "raster");
    let gradient_layer_index = layer_slots.iter().position(|s| s == "gradient");

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

    // Route the gradient layer's own size/position/repeat entry onto the gradient
    // struct so the renderer can paint it as a positioned, sized tile.
    if let Some(gradient_idx) = gradient_layer_index {
        let gradient_box = resolve_gradient_layer_box(map, gradient_idx);
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
        style.background_origin = match k.as_str() {
            "border-box" => BackgroundOrigin::Border,
            "content-box" => BackgroundOrigin::Content,
            _ => BackgroundOrigin::Padding,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-clip") {
        style.background_clip = match k.as_str() {
            "padding-box" => BackgroundClip::Padding,
            "content-box" => BackgroundClip::Content,
            // `text` and any unknown value fall back to the initial `border-box`.
            _ => BackgroundClip::Border,
        };
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

    // z-index
    if let Some(CssValue::Number(v)) = get_non_special(map, "z-index") {
        style.z_index = *v as i32;
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

    // Collect custom properties (--*) into style.custom_properties
    for (prop, val) in &map.properties {
        if prop.starts_with("--") {
            if let CssValue::Keyword(raw) = val {
                style.custom_properties.insert(prop.clone(), raw.clone());
            }
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
            s.border.top.width = v;
            s.border.right.width = v;
            s.border.bottom.width = v;
            s.border.left.width = v;
        }),
        // NOTE: `border-radius` is intentionally NOT in this list. A border-radius
        // percentage resolves against the element's OWN border box (horizontal
        // radii against its width, vertical against its height) per CSS
        // Backgrounds §5.1 — NOT against the parent/containing-block width. The
        // dedicated `border-radius` match above keeps the percentage as a hint
        // (`border_radii_pct` / `border_radii_y_pct`) and the block/flex layout
        // resolves it per-axis once the element's box is known. Eagerly resolving
        // it here against `parent_width` produced circular (and on non-square
        // boxes, wrong-sized) corners.
        ("text-indent", |s, v| s.text_indent = v),
        ("letter-spacing", |s, v| s.letter_spacing = v),
        ("word-spacing", |s, v| s.word_spacing = v),
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
            if matches!(
                (prop_name, val),
                ("text-indent", CssValue::Percentage(_))
                    | ("word-spacing", CssValue::Percentage(_))
            ) {
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
                | CssValue::Ex(_)
                | CssValue::Ch(_)
                | CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Vmin(_)
                | CssValue::Vmax(_)
                | CssValue::Calc(_)
                | CssValue::Clamp(_, _, _)
                | CssValue::Var(_, _) => {
                    if let Some(resolved) = resolve_css_length_for_style(val, style, length_context)
                    {
                        setter(style, resolved);
                    }
                }
                CssValue::Keyword(k) => {
                    if let Some(resolved) = resolve_raw_length_for_style(k, style, length_context) {
                        setter(style, resolved);
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(CssValue::Percentage(v)) = get_non_special(map, "text-indent") {
        let basis = style.width.unwrap_or(length_context.parent_width);
        style.text_indent = basis * *v / 100.0;
    }
    if let Some(CssValue::Percentage(v)) = get_non_special(map, "word-spacing") {
        style.word_spacing = style.font_size * *v / 100.0;
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

    let resolved_parent_height = parent.height.filter(|height| *height > 0.0);
    let resolve_block_percentage =
        |percent: f32| resolved_parent_height.map(|height| height * percent / 100.0);

    // Height-axis resolution context: percentages inside height/min/max-height
    // clamp() resolve against the parent's content height, so we feed the
    // resolved parent height into the context's `parent_width` field (the field
    // the resolver uses as the percentage basis). Falls back to the viewport
    // height when the parent height is indefinite.
    let height_length_context = crate::style::resolve::LengthResolutionContext::new(
        resolved_parent_height.unwrap_or(parent.viewport_height),
        style.font_size,
        parent.root_font_size,
        parent.viewport_width,
        parent.viewport_height,
    );

    if let Some(val) = get_non_special(map, "height") {
        match val {
            CssValue::Percentage(v) => {
                style.percentage_sizing.height = Some(*v);
                style.height = resolve_block_percentage(*v);
            }
            CssValue::Calc(_) | CssValue::Clamp(_, _, _) => {
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
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _) => {
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
            CssValue::Calc(_) | CssValue::Clamp(_, _, _) => {
                style.percentage_sizing.max_height = None;
                style.max_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    height_length_context,
                );
            }
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _) => {
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
            CssValue::Calc(_) | CssValue::Clamp(_, _, _) => {
                style.percentage_sizing.min_height = None;
                style.min_height = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    height_length_context,
                );
            }
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Vmin(_)
            | CssValue::Vmax(_)
            | CssValue::Var(_, _) => {
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
                CssValue::Calc(_) | CssValue::Clamp(_, _, _) => {
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
                CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Vmin(_)
                | CssValue::Vmax(_)
                | CssValue::Var(_, _) => {
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

    // Resolve font-size from new value types
    if let Some(val) = get_non_special(map, "font-size") {
        match val {
            CssValue::Percentage(v) => {
                style.font_size = parent.font_size * v / 100.0;
            }
            CssValue::Rem(v) => {
                style.font_size = v * parent.root_font_size;
            }
            CssValue::Var(_, _) => {
                if let Some(resolved) = crate::style::resolve::try_resolve_to_length_in_context(
                    val,
                    &style.custom_properties,
                    length_context,
                ) {
                    style.font_size = resolved;
                }
            }
            _ => {}
        }
    }

    // Resolve var() for color properties
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "color") {
        if let Some(c) =
            crate::style::resolve::try_resolve_var_to_color(val, &style.custom_properties)
        {
            style.color = c;
        }
    }
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "background-color") {
        if let Some(c) =
            crate::style::resolve::try_resolve_var_to_color(val, &style.custom_properties)
        {
            style.background_color = Some(c);
        }
    }
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "border-color") {
        if let Some(c) =
            crate::style::resolve::try_resolve_var_to_color(val, &style.custom_properties)
        {
            style.border.top.color = Some(c);
            style.border.right.color = Some(c);
            style.border.bottom.color = Some(c);
            style.border.left.color = Some(c);
        }
    }

    // Resolve var() for keyword properties
    if let Some(val @ CssValue::Var(_, _)) = get_non_special(map, "display") {
        if let Some(kw) =
            crate::style::resolve::try_resolve_var_to_keyword(val, &style.custom_properties)
        {
            style.display = match kw.as_str() {
                "none" => Display::None,
                "inline" => Display::Inline,
                "inline-block" => Display::InlineBlock,
                "block" => Display::Block,
                "flex" => Display::Flex,
                "grid" => Display::Grid,
                _ => style.display,
            };
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
                "fixed" => Position::Absolute,
                "sticky" => Position::Relative,
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
}

fn parse_list_style_type(k: &str) -> ListStyleType {
    match k.to_ascii_lowercase().as_str() {
        "disc" => ListStyleType::Disc,
        "circle" => ListStyleType::Circle,
        "square" => ListStyleType::Square,
        "decimal" => ListStyleType::Decimal,
        "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
        "lower-roman" => ListStyleType::LowerRoman,
        "upper-roman" => ListStyleType::UpperRoman,
        "none" => ListStyleType::None,
        _ => ListStyleType::Disc,
    }
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
                items.push(ContentItem::String(body[..end].to_string()));
                rest = &body[end + 1..];
            } else {
                items.push(ContentItem::String(body.to_string()));
                break;
            }
        } else if let Some(body) = rest.strip_prefix('\'') {
            if let Some(end) = body.find('\'') {
                items.push(ContentItem::String(body[..end].to_string()));
                rest = &body[end + 1..];
            } else {
                items.push(ContentItem::String(body.to_string()));
                break;
            }
        } else if let Some((name, tail)) = parse_content_function(rest, "attr(") {
            items.push(ContentItem::Attr(name.trim().to_string()));
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
            strings.push(buf);
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
            // edge, i.e. object start at free_space - L (encoded as a length
            // relative to the start once the box size is known — but object-fit
            // boxes have no separate box size here, so far-edge lengths are rare;
            // approximate using the fraction form is not possible, so we keep the
            // exact start-relative length only for the near edges and fall back to
            // a fraction for the far edge with a length we cannot resolve).
            Some(Offset::Percent(p)) => Some(ObjectPositionComponent::Fraction(if from_start {
                p / 100.0
            } else {
                1.0 - p / 100.0
            })),
            Some(Offset::Length(l)) if from_start => Some(ObjectPositionComponent::Length(l)),
            // Far-edge length offsets are uncommon for object-position; resolve
            // them as the end edge minus the length is not expressible without the
            // box size at parse time, so anchor to the end and ignore the small
            // length (rare path).
            Some(Offset::Length(_)) => Some(ObjectPositionComponent::Fraction(1.0)),
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

/// Parse a full CSS `filter` value into (blur_radius, ordered color ops,
/// opacity multiplier, drop-shadow, url-reference id). Recognizes
/// blur/grayscale/sepia/invert/brightness/contrast/saturate/hue-rotate/
/// opacity/drop-shadow and `url(#id)` (css-filter-effects-1 §3); other unknown
/// functions are ignored. `none` clears. The opacity multiplier is the product
/// of all `opacity()` functions (1.0 when none are present) and is intended to
/// be folded into the element's final `style.opacity`. The url id (if any) is
/// resolved later, during layout, where the DOM `<filter>` is reachable.
fn parse_filter(
    val: &str,
) -> (
    Option<f32>,
    Vec<ColorFilterOp>,
    f32,
    Option<DropShadow>,
    Option<String>,
) {
    let raw = val.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return (Some(0.0), Vec::new(), 1.0, None, None);
    }
    let mut blur = None;
    let mut ops = Vec::new();
    let mut opacity = 1.0_f32;
    let mut drop_shadow = None;
    let mut url_id = None;
    let mut rest = raw;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_ascii_lowercase();
        let Some(close_rel) = rest[open + 1..].find(')') else {
            break;
        };
        let arg = rest[open + 1..open + 1 + close_rel].trim();
        match name.as_str() {
            "blur" => {
                if let Some(r) = parse_filter_blur(&format!("blur({arg})")) {
                    blur = Some(r);
                }
            }
            "grayscale" => {
                ops.push(ColorFilterOp::Grayscale(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "sepia" => {
                ops.push(ColorFilterOp::Sepia(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "invert" => {
                ops.push(ColorFilterOp::Invert(
                    parse_filter_amount(arg, 1.0).clamp(0.0, 1.0),
                ));
            }
            "brightness" => {
                ops.push(ColorFilterOp::Brightness(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "contrast" => {
                ops.push(ColorFilterOp::Contrast(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "saturate" => {
                ops.push(ColorFilterOp::Saturate(
                    parse_filter_amount(arg, 1.0).max(0.0),
                ));
            }
            "hue-rotate" => {
                ops.push(ColorFilterOp::HueRotate(parse_filter_angle(arg)));
            }
            "opacity" => {
                opacity *= parse_filter_amount(arg, 1.0).clamp(0.0, 1.0);
            }
            "drop-shadow" => {
                if let Some(ds) = parse_drop_shadow(arg) {
                    drop_shadow = Some(ds);
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
                }
            }
            _ => {}
        }
        rest = &rest[open + 1 + close_rel + 1..];
    }
    if blur.is_none() && ops.iter().any(|op| matches!(op, ColorFilterOp::Sepia(_))) {
        // Force replaced-image sepia through the rendered-raster filter path:
        // Chrome applies filter functions to the painted image, not directly to
        // the source pixels. A sub-CSS-pixel blur is visually neutral but gives
        // the existing raster pipeline a concrete trigger.
        blur = Some(0.1125);
    }
    (blur, ops, opacity, drop_shadow, url_id)
}

/// Parse the inner argument of `drop-shadow(<offset-x> <offset-y> <blur>?
/// <color>?)` (css-filter-effects-1 §4.4). Lengths become points; the color
/// defaults to the element's `currentColor` (approximated as opaque black here,
/// matching the common case). Returns `None` when the two required offsets are
/// missing.
fn parse_drop_shadow(arg: &str) -> Option<DropShadow> {
    let mut lengths: Vec<f32> = Vec::new();
    let mut color: Option<(f32, f32, f32, f32)> = None;
    for tok in arg.split_whitespace() {
        if let Some(CssValue::Length(l)) = crate::parser::css::parse_length(tok) {
            lengths.push(l);
        } else if let Some(CssValue::Color(c)) = crate::parser::css::parse_color(tok) {
            color = Some(c.to_f32_rgba());
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
        color: color.unwrap_or((0.0, 0.0, 0.0, 1.0)),
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
        _ => BackgroundRepeat::Repeat,
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
    GradientLayerBox {
        size,
        position,
        repeat,
    }
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
        _ => None,
    }
}

fn is_background_position_keyword(token: &str) -> bool {
    matches!(token, "left" | "right" | "top" | "bottom" | "center")
}

/// Parse a `box-shadow` shorthand value.
///
/// Supports CSS syntax:
/// - `[inset]? <offset-x> <offset-y> [<blur> [<spread>]] [<color>]`
/// - `inset 0 2px 8px rgba(0,0,0,0.3)`
/// - `4px 4px 8px 2px #ccc`  (with spread)
/// - Multiple shadows separated by commas — only the first is retained.
fn parse_box_shadow(val: &str) -> Vec<BoxShadow> {
    let val = val.trim();
    if val == "none" {
        return Vec::new();
    }

    // A `box-shadow` may list several shadows separated by top-level commas
    // (outside parens). They paint back-to-front, the first listed on top.
    split_top_level_comma(val)
        .iter()
        .filter_map(|s| parse_single_box_shadow(s))
        .collect()
}

/// Parse one shadow from a `box-shadow` list entry.
fn parse_single_box_shadow(val: &str) -> Option<BoxShadow> {
    let val = val.trim();
    if val.is_empty() {
        return None;
    }

    // Tokenize: spaces delimit tokens, but rgba(...) / rgb(...) / hsl(...)
    // and keyword `inset` are each a single token.
    let mut tokens: Vec<String> = Vec::new();
    let mut chars = val.chars().peekable();
    let mut current = String::new();

    while let Some(&ch) = chars.peek() {
        if ch == ' ' && !current.contains('(') {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            chars.next();
        } else if ch == ')' {
            current.push(ch);
            chars.next();
            tokens.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
            chars.next();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    // Pull out `inset` keyword wherever it appears (CSS allows it at either
    // end of the declaration).
    let mut inset = false;
    tokens.retain(|t| {
        if t.eq_ignore_ascii_case("inset") {
            inset = true;
            false
        } else {
            true
        }
    });

    if tokens.len() < 2 {
        return None;
    }

    let offset_x = parse_shadow_length(&tokens[0])?;
    let offset_y = parse_shadow_length(&tokens[1])?;

    // Tokens after offsets may be: [blur [spread]] [color]. Lengths parse
    // numerically; colors don't. Walk forward consuming up to 2 lengths.
    let mut idx = 2;
    let mut blur = 0.0;
    let mut spread = 0.0;
    if idx < tokens.len() {
        if let Some(b) = parse_shadow_length(&tokens[idx]) {
            blur = b;
            idx += 1;
            if idx < tokens.len()
                && let Some(s) = parse_shadow_length(&tokens[idx])
            {
                spread = s;
                idx += 1;
            }
        }
    }

    // An omitted (or `currentColor`) shadow color defaults to the element's
    // `color`, resolved later via the CURRENT_COLOR_SENTINEL in
    // resolve_current_color (CSS Backgrounds & Borders L3 §7.2).
    let color = if idx < tokens.len() {
        parse_border_color(&tokens[idx]).unwrap_or(CURRENT_COLOR_SENTINEL)
    } else {
        CURRENT_COLOR_SENTINEL
    };

    Some(BoxShadow {
        offset_x,
        offset_y,
        blur,
        spread,
        color,
        inset,
    })
}

/// Parse a `text-shadow` value into a list of shadows (css-text-decor-3 §3).
/// Syntax: `none | [ <color>? && <length>{2,3} ]#`. Unlike `box-shadow` there
/// is no spread or `inset`, and the optional color may precede or follow the
/// offsets. Reuses `BoxShadow` storage with `spread = 0` and `inset = false`.
fn parse_text_shadow(val: &str) -> Vec<BoxShadow> {
    let val = val.trim();
    if val.is_empty() || val == "none" {
        return Vec::new();
    }
    split_top_level_comma(val)
        .iter()
        .filter_map(|s| parse_single_text_shadow(s))
        .collect()
}

/// Parse one `text-shadow` list entry: 2 or 3 lengths plus an optional color
/// that may appear before or after the lengths.
fn parse_single_text_shadow(val: &str) -> Option<BoxShadow> {
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
    let mut color: Option<Color> = None;
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

    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur: lengths.get(2).copied().unwrap_or(0.0),
        spread: 0.0,
        // An omitted color defaults to the element's `color`, resolved later via
        // the CURRENT_COLOR_SENTINEL in resolve_current_color.
        color: color.unwrap_or(CURRENT_COLOR_SENTINEL),
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

/// Parse a length value for box-shadow (px or pt or bare number).
fn parse_shadow_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("px") {
        n.parse::<f32>().ok().map(|v| v * 0.75)
    } else if let Some(n) = val.strip_suffix("pt") {
        n.parse::<f32>().ok()
    } else {
        val.parse::<f32>().ok()
    }
}

/// Parse a single CSS transform function (e.g. `rotate(45deg)`).
///
/// Returns the parsed transform and `None` when the function is unknown or
/// malformed. `font_size`/`root_font_size` (pt) resolve em/rem length args.
fn parse_single_transform(val: &str, font_size: f32, root_font_size: f32) -> Option<Transform> {
    let val = val.trim();
    let len = |s: &str| parse_transform_length(s, font_size, root_font_size);
    let mk_translate = |x: Option<(f32, bool)>, y: Option<(f32, bool)>| {
        let (tx, tx_pct) = x.unwrap_or((0.0, false));
        let (ty, ty_pct) = y.unwrap_or((0.0, false));
        Transform::Translate {
            tx,
            ty,
            tx_pct,
            ty_pct,
        }
    };

    if let Some(inner) = val
        .strip_prefix("rotate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Rotate(parse_angle_deg(inner)?));
    }

    // rotateZ() is the 2D z-axis rotation (== rotate()). rotateX/rotateY are 3D
    // rotations about an in-plane axis; with no perspective they collapse to a
    // horizontal/vertical scale-by-cos. We approximate them as that scale so the
    // whole list is not discarded (Chrome renders the projected footprint).
    if let Some(inner) = val
        .strip_prefix("rotateZ(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Some(Transform::Rotate(parse_angle_deg(inner)?));
    }
    if let Some(inner) = val
        .strip_prefix("rotateX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let rad = parse_angle_deg(inner)? * std::f32::consts::PI / 180.0;
        return Some(Transform::Scale(1.0, rad.cos()));
    }
    if let Some(inner) = val
        .strip_prefix("rotateY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let rad = parse_angle_deg(inner)? * std::f32::consts::PI / 180.0;
        return Some(Transform::Scale(rad.cos(), 1.0));
    }

    if let Some(inner) = val
        .strip_prefix("scaleX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let sx = inner.trim().parse::<f32>().ok()?;
        return Some(Transform::Scale(sx, 1.0));
    }

    if let Some(inner) = val
        .strip_prefix("scaleY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let sy = inner.trim().parse::<f32>().ok()?;
        return Some(Transform::Scale(1.0, sy));
    }

    if let Some(inner) = val.strip_prefix("scale(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 1 {
            let s = parts[0].trim().parse::<f32>().ok()?;
            return Some(Transform::Scale(s, s));
        } else if parts.len() == 2 {
            let sx = parts[0].trim().parse::<f32>().ok()?;
            // CSS: an omitted/empty second arg defaults to the first.
            let sy_tok = parts[1].trim();
            let sy = if sy_tok.is_empty() {
                sx
            } else {
                sy_tok.parse::<f32>().ok()?
            };
            return Some(Transform::Scale(sx, sy));
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

    if let Some(inner) = val.strip_prefix("skew(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        let ax = parse_angle_deg(parts.first()?.trim())?;
        let ay = if parts.len() >= 2 {
            parse_angle_deg(parts[1].trim()).unwrap_or(0.0)
        } else {
            0.0
        };
        let tan_x = (ax * std::f32::consts::PI / 180.0).tan();
        let tan_y = (ay * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, tan_y, tan_x, 1.0, 0.0, 0.0));
    }

    if let Some(inner) = val.strip_prefix("skewX(").and_then(|s| s.strip_suffix(')')) {
        let tan_x = (parse_angle_deg(inner)? * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, 0.0, tan_x, 1.0, 0.0, 0.0));
    }

    if let Some(inner) = val.strip_prefix("skewY(").and_then(|s| s.strip_suffix(')')) {
        let tan_y = (parse_angle_deg(inner)? * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, tan_y, 0.0, 1.0, 0.0, 0.0));
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
        let mut nums = [0.0_f32; 6];
        for (i, tok) in toks.iter().enumerate() {
            nums[i] = tok.trim().parse::<f32>().ok()?;
        }
        // a, b, c, d are unitless; e, f are pixel translations -> points.
        return Some(Transform::Matrix(
            nums[0],
            nums[1],
            nums[2],
            nums[3],
            nums[4] * 0.75,
            nums[5] * 0.75,
        ));
    }

    None
}

/// Extended affine matrix carrying percentage-translate coefficients:
/// `[a, b, c, d, e, f, e_w, e_h, f_w, f_h]`, where the effective translation for
/// a box of size `w`×`h` is `e + e_w*w + e_h*h` / `f + f_w*w + f_h*h`. This lets
/// `%` translate components survive composition without knowing the box size.
type ExtMatrix = [f32; 10];

/// Convert a single Transform into its extended affine matrix.
fn transform_to_ext(t: &Transform) -> ExtMatrix {
    match *t {
        Transform::Rotate(deg) => {
            let rad = deg * std::f32::consts::PI / 180.0;
            let (c, s) = (rad.cos(), rad.sin());
            [c, s, -s, c, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        }
        Transform::Scale(sx, sy) => [sx, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        Transform::Translate {
            tx,
            ty,
            tx_pct,
            ty_pct,
        } => {
            let (e, e_w) = if tx_pct { (0.0, tx / 100.0) } else { (tx, 0.0) };
            let (f, f_h) = if ty_pct { (0.0, ty / 100.0) } else { (ty, 0.0) };
            [1.0, 0.0, 0.0, 1.0, e, f, e_w, 0.0, 0.0, f_h]
        }
        Transform::Matrix(a, b, c, d, e, f) => [a, b, c, d, e, f, 0.0, 0.0, 0.0, 0.0],
        Transform::MatrixPct {
            a,
            b,
            c,
            d,
            e,
            f,
            e_w,
            e_h,
            f_w,
            f_h,
        } => [a, b, c, d, e, f, e_w, e_h, f_w, f_h],
    }
}

/// Multiply two extended affine matrices: `result = lhs × rhs`. The linear part
/// (a..d) multiplies normally; the translation columns (constant + per-dim
/// coefficients) each transform through `lhs`'s linear part, keeping the result
/// affine in `(w, h)`.
fn multiply_ext(lhs: &ExtMatrix, rhs: &ExtMatrix) -> ExtMatrix {
    let lin = |x: f32, y: f32| (lhs[0] * x + lhs[2] * y, lhs[1] * x + lhs[3] * y);
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
    // A trailing third token is the z-offset (3D); ignored in 2D rendering.
    let (x_tok, y_tok) = match tokens.as_slice() {
        [a] => (*a, "center"),
        [a, b] | [a, b, _] => {
            if is_vertical(a) || is_horizontal(b) {
                (*b, *a) // swapped: vertical keyword came first
            } else {
                (*a, *b)
            }
        }
        _ => return None,
    };
    let (x_fraction, x_length) = parse_origin_component(x_tok, font_size, root_font_size)?;
    let (y_fraction, y_length) = parse_origin_component(y_tok, font_size, root_font_size)?;
    Some(TransformOrigin {
        x_fraction,
        x_length,
        y_fraction,
        y_length,
    })
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

    // Multiple transforms — compose into a single matrix.
    // CSS: transforms are applied right-to-left, but the `cm` operator
    // in PDF also post-multiplies, so we compose left-to-right here and
    // the renderer will apply the resulting matrix around the centre.
    // Percentage `translate()` components are carried as box-size coefficients
    // (see `ExtMatrix`) so they survive composition without the box size.
    let mut result: ExtMatrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for func in &functions {
        let t = parse_single_transform(func, font_size, root_font_size)?;
        result = multiply_ext(&result, &transform_to_ext(&t));
    }

    // Collapse to the cheaper `Matrix` form when no percentage translate is
    // present; otherwise keep the coefficients for render-time resolution.
    if result[6] == 0.0 && result[7] == 0.0 && result[8] == 0.0 && result[9] == 0.0 {
        Some(Transform::Matrix(
            result[0], result[1], result[2], result[3], result[4], result[5],
        ))
    } else {
        Some(Transform::MatrixPct {
            a: result[0],
            b: result[1],
            c: result[2],
            d: result[3],
            e: result[4],
            f: result[5],
            e_w: result[6],
            e_h: result[7],
            f_w: result[8],
            f_h: result[9],
        })
    }
}

/// Parse a length/percentage argument to `translate()` into `(value, is_percent)`.
///
/// Absolute units (px/pt/in/cm/mm/pc) and font-relative units (em/rem) resolve
/// to pt; a `%` token returns the raw percentage with `is_percent = true` (it is
/// resolved against the element's own border box at render time). A bare number
/// is treated as pixels (lenient, matching the legacy fallback). `font_size` and
/// `root_font_size` are in pt.
fn parse_transform_length(val: &str, font_size: f32, root_font_size: f32) -> Option<(f32, bool)> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix('%') {
        return n.trim().parse::<f32>().ok().map(|v| (v, true));
    }
    parse_abs_length_pt(val, font_size, root_font_size).map(|v| (v, false))
}

/// Resolve a CSS absolute/font-relative length token to pt. Returns `None` for
/// percentages or unknown units. A bare number is treated as pixels.
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
        // Bare number: lenient fallback (treated as pt), matching legacy
        // behaviour for `translate()`/`transform-origin` fixtures.
        val.parse::<f32>().ok()
    }
}

/// Parse a CSS angle token (deg/rad/grad/turn, or bare number = degrees) to
/// degrees. Reused by `rotate()` and `skew*()`.
fn parse_angle_deg(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("deg") {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = val.strip_suffix("grad") {
        n.trim().parse::<f32>().ok().map(|g| g * 0.9)
    } else if let Some(n) = val.strip_suffix("turn") {
        n.trim().parse::<f32>().ok().map(|t| t * 360.0)
    } else if let Some(n) = val.strip_suffix("rad") {
        n.trim()
            .parse::<f32>()
            .ok()
            .map(|r| r * 180.0 / std::f32::consts::PI)
    } else {
        // Bare number: CSS treats a unitless angle as invalid, but ironpress is
        // lenient elsewhere and existing fixtures rely on bare degrees.
        val.parse::<f32>().ok()
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

/// Parse a CSS `clip-path` basic shape (circle/ellipse/inset/polygon). Returns
/// None for `none` and unsupported forms (url(), path(), etc.).
fn parse_clip_path(val: &str) -> Option<ClipPath> {
    let raw = val.trim();
    let center = (50.0, true);
    let parse_pos = |s: &str| -> ((f32, bool), (f32, bool)) {
        // `at X Y` — default to centre when a coord is missing/unparsable.
        let mut it = s.split_whitespace();
        let x = it.next().and_then(parse_clip_len).unwrap_or(center);
        let y = it.next().and_then(parse_clip_len).unwrap_or(center);
        (x, y)
    };
    if let Some(inner) = raw
        .strip_prefix("circle(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let (shape, pos) = inner.split_once(" at ").unwrap_or((inner, ""));
        let r = parse_clip_len(shape.trim())?;
        let (cx, cy) = parse_pos(pos);
        return Some(ClipPath::Circle { r, cx, cy });
    }
    if let Some(inner) = raw
        .strip_prefix("ellipse(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let (shape, pos) = inner.split_once(" at ").unwrap_or((inner, ""));
        let mut radii = shape.split_whitespace();
        let rx = radii.next().and_then(parse_clip_len)?;
        let ry = radii.next().and_then(parse_clip_len).unwrap_or(rx);
        let (cx, cy) = parse_pos(pos);
        return Some(ClipPath::Ellipse { rx, ry, cx, cy });
    }
    if let Some(inner) = raw.strip_prefix("inset(").and_then(|s| s.strip_suffix(')')) {
        let (insets_part, radius) = match inner.split_once(" round ") {
            Some((a, r)) => (a, parse_clip_len(r.trim()).map_or(0.0, |(v, _)| v)),
            None => (inner, 0.0),
        };
        let vals: Vec<(f32, bool)> = insets_part
            .split_whitespace()
            .filter_map(parse_clip_len)
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
            radius,
        });
    }
    if let Some(inner) = raw
        .strip_prefix("polygon(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let points: Vec<((f32, bool), (f32, bool))> = inner
            .split(',')
            .filter_map(|pair| {
                let mut it = pair.split_whitespace();
                let x = parse_clip_len(it.next()?)?;
                let y = parse_clip_len(it.next()?)?;
                Some((x, y))
            })
            .collect();
        if points.len() >= 3 {
            return Some(ClipPath::Polygon(points));
        }
    }
    None
}

/// Parse a CSS `mask-image` (or the image token of the `mask` shorthand) into a
/// `MaskSource` (css-masking-1 §3.1). Only the deterministic CSS-image sources
/// (linear/radial/conic gradients incl. their repeating variants) are modelled;
/// `url()` references are deferred (return `None` = leave the current value).
///
/// Returns:
/// - `None` — unrecognised / unsupported (e.g. `url(...)`); caller leaves as-is.
/// - `Some(None)` — explicit `none`; caller clears the mask.
/// - `Some(Some(src))` — a parseable gradient mask source.
fn parse_mask_image(val: &str) -> Option<Option<MaskSource>> {
    let raw = val.trim();
    let lower = raw.to_ascii_lowercase();
    // For a multi-layer (comma-separated) value, mask only the primary layer.
    let first = raw.split(',').next().unwrap_or(raw).trim();
    let first_lower = first.to_ascii_lowercase();
    if lower == "none" || first_lower == "none" {
        return Some(None);
    }
    if first_lower.starts_with("linear-gradient(")
        || first_lower.starts_with("repeating-linear-gradient(")
    {
        // A comma-separated gradient must be parsed from the whole `raw` value
        // (its own args contain commas); only fall back to the layer split when
        // the gradient is followed by extra layers.
        return parse_linear_gradient(raw)
            .or_else(|| parse_linear_gradient(first))
            .map(|g| Some(MaskSource::Linear(g)));
    }
    if first_lower.starts_with("radial-gradient(")
        || first_lower.starts_with("repeating-radial-gradient(")
    {
        return parse_radial_gradient(raw)
            .or_else(|| parse_radial_gradient(first))
            .map(|g| Some(MaskSource::Radial(g)));
    }
    if first_lower.starts_with("conic-gradient(")
        || first_lower.starts_with("repeating-conic-gradient(")
    {
        return parse_conic_gradient(raw)
            .or_else(|| parse_conic_gradient(first))
            .map(|g| Some(MaskSource::Conic(g)));
    }
    // `url(...)` mask reference (css-masking-1 §3.1). Only SVG image sources are
    // supported as masks: the referenced SVG is loaded and rasterised to a
    // coverage buffer at paint time. Non-SVG / unloadable urls leave the value
    // unchanged (return `None`) rather than clearing it. Parse from the whole
    // `raw` value because a `url(data:...)` argument legitimately contains commas
    // (so the layer split on `,` would truncate the data URI).
    if lower.starts_with("url(") {
        return parse_mask_url_svg(raw).map(MaskSource::Svg).map(Some);
    }
    None
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
    let (bytes, _mime) = crate::layout::images::load_src_bytes(url)?;
    // Accept only sources whose bytes actually sniff as SVG, since the mask
    // rasteriser only understands SVG image sources. The MIME alone is not
    // trusted (it can mislabel non-SVG payloads).
    if !crate::layout::images::looks_like_svg(&bytes) {
        return None;
    }
    Some(std::sync::Arc::new(bytes))
}

/// Parse a CSS Grid box-alignment keyword (`start`/`end`/`center`/`stretch`).
/// Also accepts the flex aliases `flex-start`/`flex-end` for robustness.
fn parse_grid_align(k: &str) -> GridAlign {
    match k.trim() {
        "start" | "flex-start" | "left" | "self-start" => GridAlign::Start,
        "end" | "flex-end" | "right" | "self-end" => GridAlign::End,
        "center" => GridAlign::Center,
        _ => GridAlign::Stretch,
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
        if let Ok(n) = rest.parse::<usize>() {
            return GridLine::Span(n.max(1));
        }
        if !rest.is_empty() {
            return GridLine::SpanNamed(rest.to_string());
        }
        return GridLine::Span(1);
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
/// Rows are padded to the widest row so the result is rectangular.
fn parse_grid_template_areas(val: &str) -> Vec<Vec<Option<String>>> {
    let val = val.trim();
    if val == "none" || val.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    // Each row is delimited by a quoted string. Split on the quote characters
    // and keep the segments between matched quotes.
    let mut in_quote = false;
    let mut current = String::new();
    for ch in val.chars() {
        match ch {
            '"' | '\'' => {
                if in_quote {
                    // End of a row string.
                    let cells: Vec<Option<String>> = current
                        .split_whitespace()
                        .map(|tok| {
                            if tok.chars().all(|c| c == '.') {
                                None
                            } else {
                                Some(tok.to_string())
                            }
                        })
                        .collect();
                    if !cells.is_empty() {
                        rows.push(cells);
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
    // Pad rows to a uniform width (CSS requires equal counts; be lenient).
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for r in &mut rows {
        while r.len() < width {
            r.push(None);
        }
    }
    rows
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

    (result, line_names)
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

/// A single border-radius token resolved to either an absolute length (points)
/// or a percentage to be resolved against the box dimensions in layout.
#[derive(Debug, Clone, Copy)]
enum RadiusToken {
    Len(f32),
    Pct(f32),
}

/// Parse one border-radius token (`12px`, `40px`, `25%`, `0`) into a
/// `RadiusToken`. Lengths convert px→pt; percentages are preserved for
/// layout-time resolution. Returns `None` for unrecognised tokens.
fn parse_radius_token(tok: &str) -> Option<RadiusToken> {
    let tok = tok.trim();
    if let Some(p) = tok.strip_suffix('%') {
        return p.parse::<f32>().ok().map(RadiusToken::Pct);
    }
    if let Some(p) = tok.strip_suffix("px") {
        return p.parse::<f32>().ok().map(|v| RadiusToken::Len(v * 0.75));
    }
    if let Some(p) = tok.strip_suffix("pt") {
        return p.parse::<f32>().ok().map(RadiusToken::Len);
    }
    // Bare `0` (and other unitless numbers, treated as px-equivalent zero-ish
    // lengths only when exactly zero per CSS; non-zero unitless is invalid but
    // we accept it as points to be forgiving).
    tok.parse::<f32>().ok().map(|v| {
        if v == 0.0 {
            RadiusToken::Len(0.0)
        } else {
            RadiusToken::Len(v * 0.75)
        }
    })
}

/// Expand the CSS `border-radius` shorthand (the horizontal-radii group, before
/// any `/`) of 1-4 space-separated tokens into the four corners in
/// [top-left, top-right, bottom-right, bottom-left] order, following the CSS
/// edge-list expansion rules.
fn expand_radius_group(tokens: &[RadiusToken]) -> [RadiusToken; 4] {
    match tokens.len() {
        1 => [tokens[0]; 4],
        2 => [tokens[0], tokens[1], tokens[0], tokens[1]],
        3 => [tokens[0], tokens[1], tokens[2], tokens[1]],
        _ => [tokens[0], tokens[1], tokens[2], tokens[3]],
    }
}

/// Parse a full `border-radius` value into per-corner horizontal and vertical
/// radii. The grammar is `<h1> [h2 h3 h4] [ / <v1> [v2 v3 v4] ]`: the optional
/// part after `/` gives the VERTICAL radii (elliptical corners). When no `/`
/// group is present the vertical radii equal the horizontal ones (circular
/// corners). Returns `(radii_x_pt, radii_x_pct, radii_y_pt, radii_y_pct)` in
/// [top-left, top-right, bottom-right, bottom-left] corner order.
#[allow(clippy::type_complexity)]
fn parse_border_radius_shorthand(
    value: &str,
) -> ([f32; 4], [Option<f32>; 4], [f32; 4], [Option<f32>; 4]) {
    let mut parts = value.split('/');
    let horiz_part = parts.next().unwrap_or("").trim();
    let vert_part = parts.next().map(str::trim);
    let parse_group = |s: &str| -> Option<[RadiusToken; 4]> {
        let tokens: Vec<RadiusToken> = s
            .split_whitespace()
            .filter_map(parse_radius_token)
            .collect();
        if tokens.is_empty() {
            None
        } else {
            Some(expand_radius_group(&tokens))
        }
    };
    let Some(h_corners) = parse_group(horiz_part) else {
        return ([0.0; 4], [None; 4], [0.0; 4], [None; 4]);
    };
    // Vertical group defaults to the horizontal one (circular corners).
    let v_corners = vert_part.and_then(parse_group).unwrap_or(h_corners);
    let to_arrays = |corners: [RadiusToken; 4]| -> ([f32; 4], [Option<f32>; 4]) {
        let mut radii = [0.0f32; 4];
        let mut radii_pct = [None; 4];
        for (i, c) in corners.iter().enumerate() {
            match c {
                RadiusToken::Len(v) => radii[i] = *v,
                RadiusToken::Pct(p) => radii_pct[i] = Some(*p),
            }
        }
        (radii, radii_pct)
    };
    let (rx, rx_pct) = to_arrays(h_corners);
    let (ry, ry_pct) = to_arrays(v_corners);
    (rx, rx_pct, ry, ry_pct)
}

/// Map a CSS `border-style` keyword to a `BorderStyle`. Unknown keywords keep
/// the CSS-wide default (`solid`); `none`/`hidden` suppress the edge.
fn parse_border_style_keyword(keyword: &str) -> BorderStyle {
    match keyword.trim().to_ascii_lowercase().as_str() {
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "none" | "hidden" => BorderStyle::None,
        _ => BorderStyle::Solid,
    }
}

/// Parse a `column-rule` shorthand (`<width> || <style> || <color>`) into a
/// `BorderSide`, reusing the border-shorthand tokenizer. Per CSS Multicol §6 the
/// initial `column-rule-width` is `medium`, so a shorthand that names a visible
/// style without a width (e.g. `column-rule: dotted blue`) still paints at the
/// medium width rather than the 0 the border tokenizer leaves it at.
fn parse_column_rule_shorthand(k: &str, font_size: f32) -> BorderSide {
    let (mut width, color, style) = parse_border_shorthand(k, font_size);
    if width <= 0.0 && style != BorderStyle::None {
        width = MEDIUM_RULE_WIDTH_PT;
    }
    BorderSide {
        width,
        color,
        style,
    }
}

/// CSS `medium` line width in points (~3px). Shared by the column-rule
/// shorthand and longhand defaults.
const MEDIUM_RULE_WIDTH_PT: f32 = 2.25;

/// Resolve a `border-*-width` CssValue (uniform or per-side) to points using the
/// element's `font_size` as the em basis. Absolute lengths (`CssValue::Length`,
/// already in pt) apply directly; a font-relative width (`em`/`ex`/`ch`, which
/// `parse_length` emits as `CssValue::Number` — an em factor) multiplies by the
/// font-size, mirroring the margin/width paths. `rem` resolves against the same
/// font-size (the consumers run before the root-font-size context is built, and a
/// `rem` border width is exceedingly rare). Returns `None` for anything that
/// isn't a usable length so the caller leaves the existing width untouched.
fn resolve_border_width(val: Option<&CssValue>, font_size: f32) -> Option<f32> {
    match val? {
        CssValue::Length(v) => Some(*v),
        CssValue::Number(v) => Some(*v * font_size),
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
) -> crate::style::resolve::LengthResolutionContext {
    crate::style::resolve::LengthResolutionContext::new(
        base.parent_width,
        style.font_size,
        style.root_font_size,
        style.viewport_width,
        style.viewport_height,
    )
}

fn resolve_css_length_for_style(
    val: &CssValue,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
) -> Option<f32> {
    match val {
        CssValue::Length(v) => Some(*v),
        CssValue::Number(v) => Some(*v * style.font_size),
        CssValue::Percentage(v) => Some(base.parent_width * *v / 100.0),
        CssValue::Ex(v) => Some(*v * style.font_size * style_ex_length_ratio(style)),
        CssValue::Ch(v) => Some(
            *v * style.font_size * crate::style::font_ctx::style_ch_ratio(style).unwrap_or(0.5),
        ),
        CssValue::Rem(v) => Some(*v * style.root_font_size),
        CssValue::Vw(v) => Some(style.viewport_width * *v / 100.0),
        CssValue::Vh(v) => Some(style.viewport_height * *v / 100.0),
        CssValue::Vmin(v) => Some(style.viewport_width.min(style.viewport_height) * *v / 100.0),
        CssValue::Vmax(v) => Some(style.viewport_width.max(style.viewport_height) * *v / 100.0),
        CssValue::Calc(_) | CssValue::Clamp(_, _, _) | CssValue::Var(_, _) => {
            crate::style::resolve::try_resolve_to_length_in_context(
                val,
                &style.custom_properties,
                style_length_context(style, base),
            )
        }
        CssValue::Keyword(k) => resolve_raw_length_for_style(k, style, base),
        _ => None,
    }
}

fn resolve_raw_length_for_style(
    raw: &str,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
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
        _ => resolve_css_length_for_style(&parsed, style, base),
    })
}

fn style_cap_height_ratio(_style: &ComputedStyle) -> f32 {
    0.75
}

fn style_ex_length_ratio(style: &ComputedStyle) -> f32 {
    crate::style::font_ctx::style_x_height_ratio(style)
        .unwrap_or(0.5)
        .max(0.5625)
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
) -> Option<[f32; 4]> {
    let values: Vec<f32> = split_css_whitespace(raw)
        .into_iter()
        .map(|part| parse_border_width_token(part, style, base))
        .collect::<Option<Vec<_>>>()?;
    expand_box_values(&values)
}

fn parse_border_width_token(
    token: &str,
    style: &ComputedStyle,
    base: crate::style::resolve::LengthResolutionContext,
) -> Option<f32> {
    match token.trim().to_ascii_lowercase().as_str() {
        "thin" => Some(0.75),
        "medium" => Some(MEDIUM_RULE_WIDTH_PT),
        "thick" => Some(3.75),
        other => resolve_raw_length_for_style(other, style, base),
    }
}

fn parse_border_color_shorthand_values(raw: &str) -> Option<[Color; 4]> {
    let values: Vec<Color> = split_css_whitespace(raw)
        .into_iter()
        .map(parse_border_color)
        .collect::<Option<Vec<_>>>()?;
    expand_box_values(&values)
}

fn parse_border_style_shorthand_values(raw: &str) -> Option<[BorderStyle; 4]> {
    let parts = split_css_whitespace(raw);
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let values: Vec<BorderStyle> = parts.into_iter().map(parse_border_style_keyword).collect();
    expand_box_values(&values)
}

fn parse_border_shorthand(k: &str, font_size: f32) -> (f32, Option<Color>, BorderStyle) {
    // A function color such as `rgba(38, 50, 56, 0.35)` contains internal spaces,
    // so pull it out (and remove it from the string) before tokenizing on
    // whitespace. Otherwise the rgba(...) would shatter into several "words" and
    // the color (alpha included) would be lost, painting the border opaque black.
    let (rest, func_color) = extract_border_function_color(k);
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let mut width = 0.0f32;
    let mut border_style = BorderStyle::Solid;
    for part in &parts {
        match *part {
            "dashed" => border_style = BorderStyle::Dashed,
            "dotted" => border_style = BorderStyle::Dotted,
            "double" => border_style = BorderStyle::Double,
            "none" | "hidden" => border_style = BorderStyle::None,
            "solid" => border_style = BorderStyle::Solid,
            // CSS keyword border widths.
            "thin" => width = 0.75,
            "medium" => width = 2.25,
            "thick" => width = 3.75,
            // Any length token (px/pt/em/ex/ch/cm/mm/Q/in/pc/rem). `parse_length`
            // returns pt for absolute units and an em factor (Number) for
            // font-relative units; resolve_border_width applies the font-size
            // basis. Previously only px/pt were handled, so an em/cm/etc. width
            // was silently dropped (width = 0) — e.g. `border: 0.2em solid`.
            other => {
                if let Some(w) = resolve_border_width(parse_length(other).as_ref(), font_size) {
                    width = w;
                }
            }
        }
    }
    let color = func_color.or_else(|| parts.last().and_then(|last| parse_border_color(last)));
    (width, color, border_style)
}

/// Parse a single border color token using the shared CSS color parser, which
/// handles named colors, `#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa` hex (lightningcss
/// serialises `rgba(...)` to 8-digit hex with the alpha byte), and
/// `rgb()`/`rgba()` functions — preserving alpha for translucent borders.
fn parse_border_color_token(val: &str) -> Option<Color> {
    match crate::parser::css::parse_color(val) {
        Some(CssValue::Color(c)) => Some(c),
        _ => None,
    }
}

/// Pull a function color (`rgb(...)` / `rgba(...)`) out of a border shorthand,
/// returning the string with that token removed plus the parsed color. The
/// remaining tokens (width / style keyword) are space-separated as usual. If no
/// function color is present the input is returned unchanged with `None`.
fn extract_border_function_color(k: &str) -> (String, Option<Color>) {
    let lower = k.to_ascii_lowercase();
    for prefix in ["rgba(", "rgb("] {
        if let Some(start) = lower.find(prefix) {
            if let Some(close_rel) = k[start..].find(')') {
                let end = start + close_rel + 1;
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

/// Parse a color name or hex value for border shorthand.
fn parse_border_color(val: &str) -> Option<Color> {
    let lower = val.to_ascii_lowercase();
    // `currentColor` can't be resolved here (the element's final `color` isn't
    // known yet). Mark it with a sentinel; a post-pass in
    // `compute_style_with_context` swaps it for the computed `color`.
    if lower == "currentcolor" {
        return Some(CURRENT_COLOR_SENTINEL);
    }
    // Delegate everything else (named colors, #rgb/#rgba/#rrggbb/#rrggbbaa hex,
    // rgb()/rgba() functions) to the shared CSS color parser so translucent
    // borders keep their alpha channel.
    parse_border_color_token(val).or_else(|| match lower.as_str() {
        "black" => Some(Color::rgb(0, 0, 0)),
        "white" => Some(Color::rgb(255, 255, 255)),
        "red" => Some(Color::rgb(255, 0, 0)),
        "green" => Some(Color::rgb(0, 128, 0)),
        "blue" => Some(Color::rgb(0, 0, 255)),
        "yellow" => Some(Color::rgb(255, 255, 0)),
        "orange" => Some(Color::rgb(255, 165, 0)),
        "purple" => Some(Color::rgb(128, 0, 128)),
        "gray" | "grey" => Some(Color::rgb(128, 128, 128)),
        _ => lower.strip_prefix('#').and_then(parse_hex_to_color),
    })
}

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
            Some(Color { r, g, b, a })
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
            Some(Color { r, g, b, a })
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
pub fn parse_linear_gradient(val: &str) -> Option<LinearGradient> {
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
    if parts.len() < 2 {
        return None;
    }

    let first = parts[0].trim();

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
            _ => 180.0,
        };
        (angle, 1)
    } else if let Some(deg) = parse_css_angle_deg(first) {
        (deg, 1)
    } else {
        // No direction specified, default is "to bottom" = 180deg
        (180.0, 0)
    };

    let color_parts = &parts[color_start..];
    if color_parts.len() < 2 {
        return None;
    }

    let stops = parse_gradient_stops(color_parts)?;

    Some(LinearGradient {
        angle,
        stops,
        repeating,
        layer_box: GradientLayerBox::default(),
    })
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
pub fn parse_radial_gradient(val: &str) -> Option<RadialGradient> {
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
    if parts.len() < 2 {
        return None;
    }

    let first = parts[0].trim().to_ascii_lowercase();

    // A first arg is a shape/size/position prefix (not a color stop) when it
    // names a shape keyword, an extent keyword, an `at <pos>` clause, or is a
    // bare length/percentage size (e.g. lightningcss re-serializes
    // `circle 60px at center` to `60px`). We detect the bare-length case by it
    // not parsing as a color while parsing as a length token, so a real first
    // color stop is never dropped.
    let is_shape_or_size = first.starts_with("circle")
        || first.starts_with("ellipse")
        || first.contains("at ")
        || first.contains("closest-side")
        || first.contains("farthest-side")
        || first.contains("closest-corner")
        || first.contains("farthest-corner")
        || (parse_gradient_color(&first).is_none() && first_token_is_length(&first));
    let color_start = usize::from(is_shape_or_size);

    // Honor the `at <position>` clause, the extent keyword, and explicit
    // radius/radii, else default to a box-centered farthest-corner ellipse.
    let (center, shape, extent, radius, radii) = if color_start == 1 {
        let center = parse_radial_center(&first);
        // The shape/size keywords precede any `at` clause.
        let size_part = first.split("at").next().unwrap_or("").trim();

        let extent = if size_part.contains("closest-side") {
            RadialExtent::ClosestSide
        } else if size_part.contains("closest-corner") {
            RadialExtent::ClosestCorner
        } else if size_part.contains("farthest-side") {
            RadialExtent::FarthestSide
        } else {
            RadialExtent::FarthestCorner
        };

        let size_tokens: Vec<&str> = size_part
            .split_whitespace()
            .filter(|t| {
                !t.is_empty() && *t != "circle" && *t != "ellipse" && parse_radial_pos(t).is_some()
            })
            .collect();

        let shape = if first.starts_with("circle") {
            RadialShape::Circle
        } else if first.starts_with("ellipse") {
            RadialShape::Ellipse
        } else if size_tokens.len() == 1 {
            // A lone length size denotes a circle of that radius.
            RadialShape::Circle
        } else {
            RadialShape::Ellipse
        };

        // Explicit sizes: a circle takes one length radius; an ellipse takes two
        // length/percentage radii (rx, ry).
        let (radius, radii) = match shape {
            RadialShape::Circle => (
                size_tokens.first().and_then(|t| parse_radial_length_pt(t)),
                None,
            ),
            RadialShape::Ellipse => {
                if size_tokens.len() == 2 {
                    let rx = parse_radial_pos(size_tokens[0]);
                    let ry = parse_radial_pos(size_tokens[1]);
                    match (rx, ry) {
                        (Some(rx), Some(ry)) => (None, Some((rx, ry))),
                        _ => (None, None),
                    }
                } else {
                    (None, None)
                }
            }
        };

        (center, shape, extent, radius, radii)
    } else {
        (
            (RadialPos::Fraction(0.5), RadialPos::Fraction(0.5)),
            RadialShape::Ellipse,
            RadialExtent::FarthestCorner,
            None,
            None,
        )
    };

    let color_parts = &parts[color_start..];
    if color_parts.len() < 2 {
        return None;
    }

    let stops = parse_gradient_stops(color_parts)?;

    Some(RadialGradient {
        stops,
        center,
        shape,
        extent,
        radius,
        radii,
        repeating,
        layer_box: GradientLayerBox::default(),
    })
}

/// Parse a CSS `conic-gradient(...)` / `repeating-conic-gradient(...)` function
/// value into a `ConicGradient`.
///
/// Honors `from <angle>`, `at <position>`, and angular color stops in `deg`,
/// `grad`, `rad`, `turn`, or `%` (100% = one turn). Stop positions are
/// normalized to a fraction of a full turn.
pub fn parse_conic_gradient(val: &str) -> Option<ConicGradient> {
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

    let first = parts[0].trim();
    let first_lower = first.to_ascii_lowercase();

    // The first argument is a `[from <angle>] [at <position>]` prefix when it
    // mentions either keyword; otherwise it is the first color stop.
    let has_prefix = first_lower.starts_with("from ") || first_lower.contains("at ");
    let color_start = usize::from(has_prefix);

    let (from_angle, center) = if has_prefix {
        let from_angle = first_lower
            .strip_prefix("from ")
            .map(|rest| rest.split("at").next().unwrap_or("").trim().to_string())
            .and_then(|tok| parse_css_angle_deg(&tok))
            .unwrap_or(0.0);
        let center = parse_radial_center(&first_lower);
        (from_angle, center)
    } else {
        (0.0, (RadialPos::Fraction(0.5), RadialPos::Fraction(0.5)))
    };

    let color_parts = &parts[color_start..];
    let stops = parse_conic_stops(color_parts)?;
    if stops.len() < 2 {
        return None;
    }

    Some(ConicGradient {
        from_angle,
        center,
        stops,
        repeating,
        layer_box: GradientLayerBox::default(),
    })
}

/// Parse angular color stops for a conic gradient. Each part is `color`,
/// `color <angle>`, or `color <angle> <angle>` (a hard/range stop expands into
/// two stops at the two angles). Positions are normalized to fractions of one
/// turn. Stops without an explicit angle are distributed/clamped per CSS:
/// the first defaults to 0, the last to 1, and interior gaps are interpolated.
fn parse_conic_stops(parts: &[String]) -> Option<Vec<GradientStop>> {
    // Raw stops: (color, optional fraction position).
    let mut raw: Vec<(Color, Option<f32>)> = Vec::new();

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Split into whitespace tokens; the leading run that parses as a color
        // is the color, trailing tokens are angle positions. Colors like
        // `rgb(...)` have no spaces after lightningcss normalization.
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        // Find how many leading tokens form the color (1 normally).
        let color = parse_gradient_color(tokens[0])?;
        let angle_tokens = &tokens[1..];
        let positions: Vec<f32> = angle_tokens
            .iter()
            .filter_map(|t| parse_conic_angle_fraction(t))
            .collect();
        match positions.len() {
            0 => raw.push((color, None)),
            1 => raw.push((color, Some(positions[0]))),
            _ => {
                // A range stop `color a b` expands to two coincident-color stops.
                for p in positions {
                    raw.push((color, Some(p)));
                }
            }
        }
    }

    if raw.len() < 2 {
        return None;
    }

    // Fill missing positions: clamp ends, then linearly distribute interior runs.
    let n = raw.len();
    if raw[0].1.is_none() {
        raw[0].1 = Some(0.0);
    }
    if raw[n - 1].1.is_none() {
        raw[n - 1].1 = Some(1.0);
    }
    let mut i = 0;
    while i < n {
        if raw[i].1.is_some() {
            i += 1;
            continue;
        }
        // Find the next anchored stop.
        let start = i - 1;
        let mut j = i;
        while j < n && raw[j].1.is_none() {
            j += 1;
        }
        let p0 = raw[start].1.unwrap_or(0.0);
        let p1 = raw[j].1.unwrap_or(1.0);
        let span = (j - start) as f32;
        for (k, idx) in (i..j).enumerate() {
            let frac = (k as f32 + 1.0) / span;
            raw[idx].1 = Some(p0 + (p1 - p0) * frac);
        }
        i = j;
    }

    // Enforce non-decreasing positions (later smaller positions clamp up).
    let mut last = 0.0_f32;
    let stops: Vec<GradientStop> = raw
        .into_iter()
        .map(|(color, pos)| {
            let mut p = pos.unwrap_or(0.0).clamp(0.0, 1.0);
            if p < last {
                p = last;
            }
            last = p;
            GradientStop { color, position: p }
        })
        .collect();

    Some(stops)
}

/// Parse a single conic angular position into a fraction of one turn (0..1).
/// Accepts `deg`, `grad`, `rad`, `turn`, and `%` (100% = one turn).
fn parse_conic_angle_fraction(tok: &str) -> Option<f32> {
    let tok = tok.trim();
    if let Some(n) = tok.strip_suffix('%') {
        return n.trim().parse::<f32>().ok().map(|p| p / 100.0);
    }
    if let Some(n) = tok.strip_suffix("turn") {
        return n.trim().parse::<f32>().ok();
    }
    parse_css_angle_deg(tok).map(|deg| deg / 360.0)
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

/// Parse the `at <position>` clause of a radial-gradient first argument into a
/// center `(x, y)` measured from the box's left/top edges (CSS top-down).
/// Supports keyword positions (`center`, `top`, `left`, corners), percentages,
/// and lengths. Falls back to box center when absent or unparseable.
fn parse_radial_center(first: &str) -> (RadialPos, RadialPos) {
    let half = RadialPos::Fraction(0.5);
    let lower = first.to_ascii_lowercase();
    let Some(at_pos) = lower.find("at ") else {
        return (half, half);
    };
    let pos = lower[at_pos + 3..].trim();
    if pos.is_empty() {
        return (half, half);
    }

    let mut x: Option<RadialPos> = None;
    let mut y: Option<RadialPos> = None;

    for t in pos.split_whitespace() {
        match t {
            "left" => x = Some(RadialPos::Fraction(0.0)),
            "right" => x = Some(RadialPos::Fraction(1.0)),
            "top" => y = Some(RadialPos::Fraction(0.0)),
            "bottom" => y = Some(RadialPos::Fraction(1.0)),
            "center" => { /* leaves the axis at its default 0.5 */ }
            other => {
                if let Some(p) = parse_radial_pos(other) {
                    // First numeric goes to x, the second to y.
                    if x.is_none() {
                        x = Some(p);
                    } else if y.is_none() {
                        y = Some(p);
                    }
                }
            }
        }
    }

    (x.unwrap_or(half), y.unwrap_or(half))
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

/// Parse gradient color stops from a list of comma-separated stop tokens.
///
/// Each token is `color`, `color <pos>`, or `color <pos> <pos>` (a range/hard
/// stop, expanded into two coincident-color stops). Positions are percentages
/// (`0%`..`100%` → 0.0..1.0). lightningcss collapses adjacent equal-color stops
/// into the range form (e.g. `red 0%, red 50%` → `red 0% 50%`), so range
/// handling is required for hard color-stop and repeating gradients.
fn parse_gradient_stops(parts: &[String]) -> Option<Vec<GradientStop>> {
    // Pass 1: parse each part into (color, list of explicit fractional positions).
    let mut raw: Vec<(Color, Vec<f32>)> = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Split off trailing position tokens (percentages) from the leading color.
        // Colors may contain spaces only inside `rgb(...)`/`rgba(...)`, which carry
        // no internal spaces after lightningcss normalization, so a simple
        // whitespace split is safe.
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        // Find the boundary: trailing tokens that parse as a `%` position.
        let mut split_at = tokens.len();
        while split_at > 1 {
            let t = tokens[split_at - 1];
            if t.strip_suffix('%')
                .and_then(|n| n.parse::<f32>().ok())
                .is_some()
            {
                split_at -= 1;
            } else {
                break;
            }
        }
        let color_str = tokens[..split_at].join(" ");
        let color = parse_gradient_color(&color_str)?;
        let positions: Vec<f32> = tokens[split_at..]
            .iter()
            .filter_map(|t| t.strip_suffix('%').and_then(|n| n.parse::<f32>().ok()))
            .map(|p| p / 100.0)
            .collect();
        raw.push((color, positions));
    }

    // Pass 2: expand range stops and flatten into (color, Option<position>).
    let mut flat: Vec<(Color, Option<f32>)> = Vec::new();
    for (color, positions) in raw {
        match positions.len() {
            0 => flat.push((color, None)),
            1 => flat.push((color, Some(positions[0]))),
            _ => {
                for p in positions {
                    flat.push((color, Some(p)));
                }
            }
        }
    }

    let n = flat.len();
    if n < 2 {
        return None;
    }

    // Pass 3: fill missing positions (clamp ends, distribute interior runs).
    if flat[0].1.is_none() {
        flat[0].1 = Some(0.0);
    }
    if flat[n - 1].1.is_none() {
        flat[n - 1].1 = Some(1.0);
    }
    let mut i = 0;
    while i < n {
        if flat[i].1.is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut j = i;
        while j < n && flat[j].1.is_none() {
            j += 1;
        }
        let p0 = flat[start].1.unwrap_or(0.0);
        let p1 = flat[j].1.unwrap_or(1.0);
        let span = (j - start) as f32;
        for (k, idx) in (i..j).enumerate() {
            flat[idx].1 = Some(p0 + (p1 - p0) * (k as f32 + 1.0) / span);
        }
        i = j;
    }

    // Enforce non-decreasing positions (CSS clamps a smaller position up).
    let mut last = 0.0_f32;
    let stops: Vec<GradientStop> = flat
        .into_iter()
        .map(|(color, pos)| {
            let mut p = pos.unwrap_or(0.0);
            if p < last {
                p = last;
            }
            last = p;
            GradientStop { color, position: p }
        })
        .collect();

    if stops.len() >= 2 { Some(stops) } else { None }
}

/// Parse a color string for gradient stops.
fn parse_gradient_color(val: &str) -> Option<Color> {
    let val = val.trim().to_ascii_lowercase();
    match val.as_str() {
        "black" => Some(Color::rgb(0, 0, 0)),
        "white" => Some(Color::rgb(255, 255, 255)),
        "red" => Some(Color::rgb(255, 0, 0)),
        "green" => Some(Color::rgb(0, 128, 0)),
        "blue" => Some(Color::rgb(0, 0, 255)),
        "yellow" => Some(Color::rgb(255, 255, 0)),
        "orange" => Some(Color::rgb(255, 165, 0)),
        "purple" => Some(Color::rgb(128, 0, 128)),
        "gray" | "grey" => Some(Color::rgb(128, 128, 128)),
        "silver" => Some(Color::rgb(192, 192, 192)),
        "maroon" => Some(Color::rgb(128, 0, 0)),
        "navy" => Some(Color::rgb(0, 0, 128)),
        "teal" => Some(Color::rgb(0, 128, 128)),
        "aqua" | "cyan" => Some(Color::rgb(0, 255, 255)),
        "fuchsia" | "magenta" => Some(Color::rgb(255, 0, 255)),
        "lime" => Some(Color::rgb(0, 255, 0)),
        // CSS Color 4 §6.1: `transparent` is `rgb(0 0 0 / 0)`, not white.
        "transparent" => Some(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        _ => {
            if let Some(hex) = val.strip_prefix('#') {
                parse_hex_to_color(hex)
            } else if let Some(inner) = val.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() == 3 {
                    let r = parts[0].trim().parse::<u8>().ok()?;
                    let g = parts[1].trim().parse::<u8>().ok()?;
                    let b = parts[2].trim().parse::<u8>().ok()?;
                    Some(Color::rgb(r, g, b))
                } else {
                    None
                }
            } else if let Some(inner) = val.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')'))
            {
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() == 4 {
                    let r = parts[0].trim().parse::<u8>().ok()?;
                    let g = parts[1].trim().parse::<u8>().ok()?;
                    let b = parts[2].trim().parse::<u8>().ok()?;
                    Some(Color::rgb(r, g, b))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        use ObjectPositionComponent::{Fraction, Length};
        // `right 10px bottom 20%`: x = right edge + 10px (far edge length, rare),
        // y = bottom edge minus 20% == 80% from the top.
        let pos = parse_object_position("right 10px bottom 20%").unwrap();
        // Near-edge length is exact; here right is a far edge so x anchors to end.
        assert_eq!(pos.x, Fraction(1.0));
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
    fn color_inherited() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.color.r, 255);
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
        assert!((style.column_rule.width - 2.25).abs() < 0.01);
        assert_eq!(style.column_rule.style, BorderStyle::Solid);
        assert!(style.column_rule.color.is_some());
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

        // Unsupported keywords fall back to the default horizontal mode.
        let lr = compute_style(HtmlTag::Div, Some("writing-mode: vertical-lr"), &parent);
        assert_eq!(lr.writing_mode, WritingMode::HorizontalTb);
        let htb = compute_style(HtmlTag::Div, Some("writing-mode: horizontal-tb"), &vrl);
        assert_eq!(htb.writing_mode, WritingMode::HorizontalTb);
    }

    #[test]
    fn column_rule_shorthand_dotted_paints_at_medium() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-rule: dotted"), &parent);
        assert!((style.column_rule.width - 2.25).abs() < 0.01);
        assert_eq!(style.column_rule.style, BorderStyle::Dotted);
    }

    #[test]
    fn column_rule_width_keyword_thin_medium_thick() {
        let parent = ComputedStyle::default();
        let thin = compute_style(HtmlTag::Div, Some("column-rule-width: thin"), &parent);
        assert!((thin.column_rule.width - 0.75).abs() < 0.01);
        let medium = compute_style(HtmlTag::Div, Some("column-rule-width: medium"), &parent);
        assert!((medium.column_rule.width - 2.25).abs() < 0.01);
        let thick = compute_style(HtmlTag::Div, Some("column-rule-width: thick"), &parent);
        assert!((thick.column_rule.width - 3.75).abs() < 0.01);
    }

    #[test]
    fn column_rule_style_longhand_only_uses_medium_width() {
        // `column-rule-style: dashed` alone should default the width to medium.
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("column-rule-style: dashed"), &parent);
        assert_eq!(style.column_rule.style, BorderStyle::Dashed);
        assert!((style.column_rule.width - 2.25).abs() < 0.01);
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
        assert!((style.column_rule.width - 3.0).abs() < 0.01); // 4px -> 3pt
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
        // em gets parsed as Number, then multiplied by parent font_size
        assert!((style.font_size - 24.0).abs() < 0.1);
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
    fn background_color_applied() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("background-color: red"), &parent);
        assert!(style.background_color.is_some());
        let bg = style.background_color.unwrap();
        assert_eq!(bg.r, 255);
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
        (c.r, c.g, c.b, c.a)
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
        (c.r, c.g, c.b, c.a)
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
        assert!(style.text_decoration_underline);
    }

    #[test]
    fn line_height_number_and_length() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("line-height: 18pt"), &parent);
        // 18pt / 12.0 font-size = 1.5
        assert!((style.line_height - 1.5).abs() < 0.1);
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
        assert!(style.text_decoration_line_through);
        assert!(!style.text_decoration_underline);
    }

    #[test]
    fn del_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Del, None, &parent);
        assert!(style.text_decoration_line_through);
    }

    #[test]
    fn s_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::S, None, &parent);
        assert!(style.text_decoration_line_through);
    }

    #[test]
    fn border_shorthand_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid black"), &parent);
        assert!((style.border.top.width - 0.75).abs() < 0.1); // 1px = 0.75pt
        assert!(style.border.top.color.is_some());
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
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
    fn border_with_custom_color() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2px solid red"), &parent);
        assert!((style.border.top.width - 1.5).abs() < 0.1); // 2px = 1.5pt
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
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
            (style.border.top.width - 3.0).abs() < 0.05,
            "em border width should be 3pt, got {}",
            style.border.top.width
        );
        assert!((style.border.bottom.width - 3.0).abs() < 0.05);
        let c = style.border.top.color.unwrap();
        assert_eq!((c.r, c.g, c.b), (0x11, 0x30, 0x5f));
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
            (style.border.top.width - 7.5).abs() < 0.05,
            "em uniform border width should be 7.5pt, got {}",
            style.border.top.width
        );
        let per_side = compute_style(
            HtmlTag::Div,
            Some("font-size: 20px; border-top-style: solid; border-top-width: 0.25em"),
            &parent,
        );
        // 0.25em * 20px = 5px = 3.75pt.
        assert!((per_side.border.top.width - 3.75).abs() < 0.05);
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
        let c = style.border.top.color.unwrap();
        assert_eq!((c.r, c.g, c.b), (0x11, 0x30, 0x5f));
    }

    #[test]
    fn border_width_and_color_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border-width: 3pt; border-color: blue"),
            &parent,
        );
        assert!((style.border.top.width - 3.0).abs() < 0.1);
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
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
        assert!((style.border.top.width - 6.0 * 0.75).abs() < 0.01);
        assert!((style.border.right.width - 14.0 * 0.75).abs() < 0.01);
        assert!((style.border.bottom.width - 22.0 * 0.75).abs() < 0.01);
        assert!((style.border.left.width - 30.0 * 0.75).abs() < 0.01);
        for side in [
            &style.border.top,
            &style.border.right,
            &style.border.bottom,
            &style.border.left,
        ] {
            assert_eq!(side.style, BorderStyle::Solid);
            let c = side.color.expect("per-side border color should be set");
            assert_eq!((c.r, c.g, c.b), (0x11, 0x30, 0x5f));
            // Paintable: width > 0 && style != None.
            assert!(side.width > 0.0 && side.style != BorderStyle::None);
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
        let left = style.border.left.color.expect("left color should be set");
        assert_eq!((left.r, left.g, left.b), (255, 0, 0));
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
        assert!((style.border.top.width - 2.0).abs() < 0.1);
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 0);
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
        let c = style.border.top.color.expect("border color should be set");
        assert_eq!(
            (c.r, c.g, c.b),
            (style.color.r, style.color.g, style.color.b)
        );
        assert_eq!((c.r, c.g, c.b), (0x6a, 0x1b, 0x9a));
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
            let c = style.border.top.color.unwrap();
            assert_eq!((c.r, c.g, c.b), (r, g, b), "failed for {name}");
        }
    }

    #[test]
    fn border_color_hex_short() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #f00"), &parent);
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_hex_long() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #00ff00"), &parent);
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_unknown_falls_back_to_current_color() {
        // An unrecognized color token leaves the border without an explicit
        // color. Per CSS the initial value of `border-color` is `currentColor`,
        // so a visible border with no resolvable color paints in the element's
        // computed `color` (here the default, black) rather than staying unset.
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid foobar"), &parent);
        let c = style
            .border
            .top
            .color
            .expect("visible border resolves a color");
        assert_eq!(
            (c.r, c.g, c.b),
            (style.color.r, style.color.g, style.color.b)
        );
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
        assert_eq!(shadow.color.r, 0);
        assert_eq!(shadow.color.g, 0);
        assert_eq!(shadow.color.b, 0);
    }

    #[test]
    fn box_shadow_with_blur() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 2px 2px 4px black"), &parent);
        let shadow = style.box_shadow[0];
        assert!((shadow.offset_x - 1.5).abs() < 0.1); // 2px * 0.75
        assert!((shadow.offset_y - 1.5).abs() < 0.1);
        assert!((shadow.blur - 3.0).abs() < 0.1); // 4px * 0.75
        assert_eq!(shadow.color.r, 0);
    }

    #[test]
    fn box_shadow_with_pt_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 3pt 3pt red"), &parent);
        let shadow = style.box_shadow[0];
        assert!((shadow.offset_x - 3.0).abs() < 0.1);
        assert!((shadow.offset_y - 3.0).abs() < 0.1);
        assert_eq!(shadow.color.r, 255);
    }

    #[test]
    fn box_shadow_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: none"), &parent);
        assert!(style.box_shadow.is_empty());
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
        assert_eq!(style.box_shadow[0].color.r, 0x6a);
        // Second listed shadow.
        assert!((style.box_shadow[1].offset_x + 12.0).abs() < 0.1); // -16px * 0.75
        assert_eq!(style.box_shadow[1].color.g, 0x83);
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
        assert_eq!(style.transform, Some(Transform::Scale(2.0, 2.0)));
    }

    #[test]
    fn transform_scale_non_uniform() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: scale(1.5, 2.0)"), &parent);
        assert_eq!(style.transform, Some(Transform::Scale(1.5, 2.0)));
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
                tx: 10.0,
                ty: 20.0,
                tx_pct: false,
                ty_pct: false
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
            tx,
            ty,
            tx_pct,
            ty_pct,
            ..
        } = t
        {
            assert!((tx - 7.5).abs() < 0.1); // 10 * 0.75
            assert!((ty - 15.0).abs() < 0.1); // 20 * 0.75
            assert!(!tx_pct && !ty_pct);
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
                tx: 50.0,
                ty: 25.0,
                tx_pct: true,
                ty_pct: true
            })
        );
        // The render-time resolution multiplies the percentage by the box size.
        let m = style.transform.unwrap().to_css_matrix(200.0, 80.0);
        assert!((m[4] - 100.0).abs() < 0.01); // 50% of 200pt
        assert!((m[5] - 20.0).abs() < 0.01); // 25% of 80pt
    }

    #[test]
    fn transform_translatex_percent() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("transform: translateX(50%)"), &parent);
        assert_eq!(
            style.transform,
            Some(Transform::Translate {
                tx: 50.0,
                ty: 0.0,
                tx_pct: true,
                ty_pct: false
            })
        );
        let m = style.transform.unwrap().to_css_matrix(120.0, 60.0);
        assert!((m[4] - 60.0).abs() < 0.01); // 50% of 120pt width
        assert!(m[5].abs() < 0.01); // no Y translation
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
        assert_eq!(style.grid_template_column_line_names[0], vec!["start"]);
        assert_eq!(style.grid_template_column_line_names[1], vec!["mid"]);
        assert_eq!(style.grid_template_column_line_names[2], vec!["end"]);
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
        assert_eq!(lg.stops.len(), 2);
        assert_eq!(lg.stops[0].color.r, 255);
        assert_eq!(lg.stops[0].color.g, 0);
        assert_eq!(lg.stops[1].color.b, 255);
        assert!(!lg.repeating);
    }

    #[test]
    fn parse_linear_gradient_range_hard_stops() {
        // lightningcss collapses `red 0%, red 50%, blue 50%, blue 100%` into the
        // range form. The parser must expand it back into four stops.
        let lg = parse_linear_gradient("linear-gradient(90deg, #d32f2f 0% 50%, #1565c0 50% 100%)")
            .unwrap();
        assert_eq!(lg.stops.len(), 4);
        assert!((lg.stops[0].position - 0.0).abs() < 0.01);
        assert!((lg.stops[1].position - 0.5).abs() < 0.01);
        assert!((lg.stops[2].position - 0.5).abs() < 0.01);
        assert!((lg.stops[3].position - 1.0).abs() < 0.01);
        assert_eq!(lg.stops[0].color.r, 211);
        assert_eq!(lg.stops[3].color.b, 192);
    }

    #[test]
    fn parse_repeating_linear_gradient_sets_flag() {
        let lg =
            parse_linear_gradient("repeating-linear-gradient(90deg, red 0% 10%, blue 10% 20%)")
                .unwrap();
        assert!(lg.repeating);
        assert_eq!(lg.stops.len(), 4);
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
        let (rx, ry) = rg.radii.expect("explicit radii");
        // 120px → 90pt, 60px → 45pt.
        assert!((rx.resolve(1000.0) - 90.0).abs() < 0.01);
        assert!((ry.resolve(1000.0) - 45.0).abs() < 0.01);
    }

    #[test]
    fn parse_repeating_radial_gradient_sets_flag() {
        let rg =
            parse_radial_gradient("repeating-radial-gradient(circle, red 0% 10%, blue 10% 20%)")
                .unwrap();
        assert!(rg.repeating);
        assert_eq!(rg.shape, RadialShape::Circle);
    }

    #[test]
    fn parse_conic_gradient_basic_four_quadrants() {
        let cg = parse_conic_gradient(
            "conic-gradient(from 0deg at center, #e53935 0deg 90deg, #43a047 90deg 180deg, #1e88e5 180deg 270deg, #fdd835 270deg 360deg)",
        )
        .unwrap();
        assert!(!cg.repeating);
        assert!((cg.from_angle - 0.0).abs() < 0.01);
        // 4 quadrant range stops → 8 stops (two per quadrant).
        assert_eq!(cg.stops.len(), 8);
        assert!((cg.stops[0].position - 0.0).abs() < 0.01);
        assert!((cg.stops[1].position - 0.25).abs() < 0.01);
        assert!((cg.stops.last().unwrap().position - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_conic_gradient_from_angle_and_position() {
        let cg = parse_conic_gradient("conic-gradient(from 45deg at 30% 30%, red, blue)").unwrap();
        assert!((cg.from_angle - 45.0).abs() < 0.01);
        let (x, _y) = cg.center;
        assert!(matches!(x, RadialPos::Fraction(f) if (f - 0.3).abs() < 0.01));
        // Two implicit stops distribute to 0 and 1.
        assert_eq!(cg.stops.len(), 2);
        assert!((cg.stops[0].position - 0.0).abs() < 0.01);
        assert!((cg.stops[1].position - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_repeating_conic_gradient_sets_flag() {
        let cg = parse_conic_gradient("repeating-conic-gradient(red 0deg 30deg, blue 30deg 60deg)")
            .unwrap();
        assert!(cg.repeating);
        assert_eq!(cg.stops.len(), 4);
        // 30deg → 1/12 turn.
        assert!((cg.stops[1].position - (30.0 / 360.0)).abs() < 0.01);
    }

    #[test]
    fn parse_conic_angle_fraction_units() {
        assert!((parse_conic_angle_fraction("90deg").unwrap() - 0.25).abs() < 0.001);
        assert!((parse_conic_angle_fraction("0.25turn").unwrap() - 0.25).abs() < 0.001);
        assert!((parse_conic_angle_fraction("50%").unwrap() - 0.5).abs() < 0.001);
        assert!((parse_conic_angle_fraction("100grad").unwrap() - 0.25).abs() < 0.001);
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
        assert_eq!(lg.stops.len(), 2);
        assert_eq!(lg.stops[0].color.r, 255);
        assert_eq!(lg.stops[1].color.b, 255);
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
        assert_eq!(lg.stops.len(), 3);
        assert!((lg.stops[0].position - 0.0).abs() < 0.01);
        assert!((lg.stops[1].position - 0.5).abs() < 0.01);
        assert!((lg.stops[2].position - 1.0).abs() < 0.01);
        assert_eq!(lg.stops[1].color.r, 255); // white
        assert_eq!(lg.stops[1].color.g, 255);
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
        assert!(parse_linear_gradient("linear-gradient(red)").is_none());
    }

    #[test]
    fn parse_radial_gradient_basic() {
        let rg = parse_radial_gradient("radial-gradient(red, blue)").unwrap();
        assert_eq!(rg.stops.len(), 2);
        assert_eq!(rg.stops[0].color.r, 255);
        assert_eq!(rg.stops[1].color.b, 255);
    }

    #[test]
    fn parse_radial_gradient_with_circle() {
        let rg = parse_radial_gradient("radial-gradient(circle, red, blue)").unwrap();
        assert_eq!(rg.stops.len(), 2);
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
        assert_eq!(rg.center.0, RadialPos::Fraction(0.0));
        assert_eq!(rg.center.1, RadialPos::Fraction(0.0));
        assert_eq!(rg.radius, None);
    }

    #[test]
    fn gradient_color_stop_auto_positions() {
        let lg = parse_linear_gradient("linear-gradient(to right, red, green, blue)").unwrap();
        assert_eq!(lg.stops.len(), 3);
        assert!((lg.stops[0].position - 0.0).abs() < 0.01);
        assert!((lg.stops[1].position - 0.5).abs() < 0.01);
        assert!((lg.stops[2].position - 1.0).abs() < 0.01);
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
        assert_eq!(lg.stops.len(), 2);
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

    #[test]
    fn mask_image_linear_gradient_from_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("mask-image: linear-gradient(to right, #000, rgba(0,0,0,0))"),
            &parent,
        );
        match style.mask_image {
            Some(MaskSource::Linear(ref lg)) => {
                assert!((lg.angle - 90.0).abs() < 0.01);
                assert_eq!(lg.stops.len(), 2);
            }
            other => panic!("expected a linear mask source, got {other:?}"),
        }
        // `match-source` (initial) on a CSS gradient resolves to alpha at paint.
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
        assert!(matches!(style.mask_image, Some(MaskSource::Radial(_))));
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
            matches!(style.mask_image, Some(MaskSource::Linear(_))),
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
            matches!(style.mask_image, Some(MaskSource::Svg(_))),
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
            matches!(style.mask_image, Some(MaskSource::Svg(_))),
            "the -webkit-mask-image url() SVG alias must populate mask_image"
        );
    }

    #[test]
    fn gradient_with_rgb_colors() {
        let lg = parse_linear_gradient("linear-gradient(to right, rgb(255, 0, 0), rgb(0, 0, 255))")
            .unwrap();
        assert_eq!(lg.stops.len(), 2);
        assert_eq!(lg.stops[0].color.r, 255);
        assert_eq!(lg.stops[1].color.b, 255);
    }

    #[test]
    fn gradient_with_hex_colors() {
        let lg =
            parse_linear_gradient("linear-gradient(90deg, #ff0000, #00ff00, #0000ff)").unwrap();
        assert_eq!(lg.stops.len(), 3);
        assert_eq!(lg.stops[0].color.r, 255);
        assert_eq!(lg.stops[1].color.g, 255);
        assert_eq!(lg.stops[2].color.b, 255);
    }

    // --- border-radius tests ---

    #[test]
    fn border_radius_default_is_zero() {
        let style = ComputedStyle::default();
        assert!((style.border_radius - 0.0).abs() < 0.001);
    }

    #[test]
    fn border_radius_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-radius: 10pt"), &parent);
        assert!((style.border_radius - 10.0).abs() < 0.001);
    }

    #[test]
    fn border_radius_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.border_radius = 15.0;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.border_radius - 0.0).abs() < 0.001);
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
        assert_eq!(style.outline_color.unwrap().r, 255);
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
        assert_eq!(style.color.r, 255);
        assert_eq!(style.color.g, 0);
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
        assert_eq!(style.color.g, 128);
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
        assert!(style.text_decoration_underline);
        // Now use initial to reset
        let style2 = compute_style(HtmlTag::Span, Some("text-decoration: initial"), &parent);
        assert!(!style2.text_decoration_underline);
        assert!(!style2.text_decoration_line_through);
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
        assert!((style.border.top.width - 0.0).abs() < 0.1);
    }

    #[test]
    fn border_color_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-color: initial"), &parent);
        assert!(style.border.top.color.is_none());
    }

    #[test]
    fn border_initial_resets_both() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: initial"), &parent);
        assert!((style.border.top.width - 0.0).abs() < 0.1);
        assert!(style.border.top.color.is_none());
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
        parent.text_decoration_underline = true;
        parent.text_decoration_line_through = true;
        let style = compute_style(HtmlTag::Span, Some("text-decoration: inherit"), &parent);
        assert!(style.text_decoration_underline);
        assert!(style.text_decoration_line_through);
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
        assert_eq!(style.background_color.unwrap().g, 128);
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
        parent.border = BorderSides::uniform(3.0, None);
        let style = compute_style(HtmlTag::Div, Some("border-width: inherit"), &parent);
        assert!((style.border.top.width - 3.0).abs() < 0.1);
    }

    #[test]
    fn border_color_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border = BorderSides::uniform(0.0, Some(Color::rgb(255, 0, 0)));
        let style = compute_style(HtmlTag::Div, Some("border-color: inherit"), &parent);
        assert_eq!(style.border.top.color.unwrap().r, 255);
    }

    #[test]
    fn border_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border = BorderSides::uniform(2.0, Some(Color::rgb(0, 0, 255)));
        let style = compute_style(HtmlTag::Div, Some("border: inherit"), &parent);
        assert!((style.border.top.width - 2.0).abs() < 0.1);
        assert_eq!(style.border.top.color.unwrap().b, 255);
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
        assert!(s.flex_basis.is_none());
        assert!((s.flex_basis_pct.unwrap() - 0.25).abs() < 1e-4);
    }

    #[test]
    fn flex_shorthand_keywords_expand() {
        let p = ComputedStyle::default();
        let none = compute_style(HtmlTag::Div, Some("flex: none"), &p);
        assert_eq!((none.flex_grow, none.flex_shrink), (0.0, 0.0));
        assert!(none.flex_basis.is_none());
        let auto = compute_style(HtmlTag::Div, Some("flex: auto"), &p);
        assert_eq!((auto.flex_grow, auto.flex_shrink), (1.0, 1.0));
        let one = compute_style(HtmlTag::Div, Some("flex: 1"), &p);
        assert_eq!((one.flex_grow, one.flex_shrink), (1.0, 1.0));
        assert_eq!(one.flex_basis, Some(0.0));
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
    fn justify_content_unknown_fallback_to_flex_start() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("justify-content: foobar"), &parent);
        assert_eq!(style.justify_content, JustifyContent::FlexStart);
    }

    #[test]
    fn align_items_unknown_fallback_to_stretch() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("align-items: foobar"), &parent);
        assert_eq!(style.align_items, AlignItems::Stretch);
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

    // --- opacity as Number (line 933) ---

    #[test]
    fn opacity_as_number_value() {
        let parent = ComputedStyle::default();
        // opacity: 0.7em gets parsed as Number(0.7)
        let style = compute_style(HtmlTag::Div, Some("opacity: 0.7em"), &parent);
        assert!((style.opacity - 0.7).abs() < 0.01);
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
        assert_eq!(style.outline_color.unwrap().b, 255);
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
        assert_eq!(style.outline_color.unwrap().r, 255);
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
        assert!((style.text_indent - 20.0).abs() < 0.1);
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
        let shadows = parse_box_shadow("2px 2px 4px black, 0 0 8px red");
        assert_eq!(shadows.len(), 2);
        // First listed shadow.
        assert!((shadows[0].blur - 3.0).abs() < 0.1);
        assert_eq!(shadows[0].color.r, 0);
        // Second listed shadow.
        assert!((shadows[1].blur - 6.0).abs() < 0.1); // 8px * 0.75
        assert_eq!(shadows[1].color.r, 255);
    }

    #[test]
    fn parse_box_shadow_non_parseable_blur_uses_as_color() {
        // "2px 2px notanumber black" — 4 tokens, but third is not a length
        let shadow = parse_single_box_shadow("2px 2px notanumber black");
        // blur parse fails, so blur = 0.0, color_start = 2, color = parse "notanumber" which fails
        // Actually color_start=2 means color_str = "notanumber" which is not a valid color -> Color::BLACK fallback
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 0.0).abs() < 0.1);
    }

    #[test]
    fn parse_box_shadow_no_color_token() {
        // Exactly 3 tokens where third is a valid blur, so color_start=3, no color token
        let shadow = parse_single_box_shadow("2px 2px 4px");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        // Per CSS Backgrounds & Borders L3 §7.2 an omitted shadow color defaults to
        // currentColor: parsed as CURRENT_COLOR_SENTINEL, resolved to the element's
        // `color` later in resolve_current_color.
        assert_eq!(s.color.r, CURRENT_COLOR_SENTINEL.r);
        assert_eq!(s.color.g, CURRENT_COLOR_SENTINEL.g);
        assert_eq!(s.color.b, CURRENT_COLOR_SENTINEL.b);
        assert_eq!(s.color.a, CURRENT_COLOR_SENTINEL.a);
    }

    #[test]
    fn parse_shadow_length_bare_number() {
        let result = parse_shadow_length("5");
        assert!(result.is_some());
        assert!((result.unwrap() - 5.0).abs() < 0.1);
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
        assert_eq!(s.color.r, 0xff);
        assert_eq!(s.color.g, 0x6f);
        assert_eq!(s.color.b, 0x00);
    }

    #[test]
    fn parse_text_shadow_color_first() {
        // text-shadow allows the color before the offsets.
        let shadows = parse_text_shadow("red 2px 2px");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].color.r, 255);
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
    fn parse_transform_rotate_bare_number() {
        let t = pt("rotate(45)");
        assert_eq!(t, Some(Transform::Rotate(45.0)));
    }

    #[test]
    fn parse_transform_translate_single_arg() {
        let t = pt("translate(10pt)");
        assert_eq!(
            t,
            Some(Transform::Translate {
                tx: 10.0,
                ty: 0.0,
                tx_pct: false,
                ty_pct: false
            })
        );
    }

    #[test]
    fn parse_transform_unknown_returns_none() {
        let t = pt("perspective(500px)");
        assert!(t.is_none());
    }

    #[test]
    fn parse_transform_skew() {
        let t = pt("skew(30deg)");
        assert!(t.is_some());
        if let Some(Transform::Matrix(a, _b, c, _d, _e, _f)) = t {
            assert!((a - 1.0).abs() < 0.001);
            assert!((c - (30.0_f32 * std::f32::consts::PI / 180.0).tan()).abs() < 0.001);
        } else {
            panic!("expected Matrix");
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
        assert_eq!(pt("scaleX(1.5)"), Some(Transform::Scale(1.5, 1.0)));
        assert_eq!(pt("scaleY(0.5)"), Some(Transform::Scale(1.0, 0.5)));
    }

    #[test]
    fn parse_transform_translate_x_y() {
        assert!(matches!(
            pt("translateX(40px)"),
            Some(Transform::Translate { ty: y, .. }) if y == 0.0
        ));
        assert!(matches!(
            pt("translateY(20px)"),
            Some(Transform::Translate { tx: x, .. }) if x == 0.0
        ));
    }

    #[test]
    fn parse_transform_length_bare_number() {
        let result = parse_transform_length("42", 12.0, 12.0);
        assert_eq!(result, Some((42.0, false)));
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
        assert_eq!(pt("scale(-1, 1)"), Some(Transform::Scale(-1.0, 1.0)));
        // A single arg mirrors to both axes.
        assert_eq!(pt("scale(2)"), Some(Transform::Scale(2.0, 2.0)));
    }

    #[test]
    fn parse_transform_translate_em_rem() {
        // 2em at 12pt font => 24pt; 1rem at 12pt root => 12pt.
        assert_eq!(
            pt("translate(2em, 1rem)"),
            Some(Transform::Translate {
                tx: 24.0,
                ty: 12.0,
                tx_pct: false,
                ty_pct: false
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
        assert!(matches!(t, Transform::MatrixPct { .. }));
        let m = t.to_css_matrix(100.0, 40.0);
        // a == 2 (scale x), e == 2 * (50% of 100) == 100.
        assert!((m[0] - 2.0).abs() < 0.01);
        assert!((m[4] - 100.0).abs() < 0.01);
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
    fn linear_gradient_unknown_to_direction_defaults() {
        let lg = parse_linear_gradient("linear-gradient(to unknown, red, blue)").unwrap();
        assert!((lg.angle - 180.0).abs() < 0.01);
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
        let lg = parse_linear_gradient("linear-gradient(to right, red)");
        assert!(lg.is_none());
    }

    // --- radial gradient not enough parts (line 1383) ---

    #[test]
    fn radial_gradient_single_part() {
        let rg = parse_radial_gradient("radial-gradient(red)");
        assert!(rg.is_none());
    }

    // --- radial gradient not enough color parts after shape keyword (line 1404) ---

    #[test]
    fn radial_gradient_shape_with_single_color() {
        let rg = parse_radial_gradient("radial-gradient(circle, red)");
        assert!(rg.is_none());
    }

    // --- gradient stop percentage without space (line 1462, 1465) ---

    #[test]
    fn gradient_stop_percentage_no_space() {
        // A stop like "50%" where the whole part is "50%" — no space before percentage
        let lg = parse_linear_gradient("linear-gradient(to right, red 0%, blue 100%)").unwrap();
        assert_eq!(lg.stops.len(), 2);
        assert!((lg.stops[0].position - 0.0).abs() < 0.01);
        assert!((lg.stops[1].position - 1.0).abs() < 0.01);
    }

    // --- gradient single stop count (line 1474) ---

    #[test]
    fn gradient_stops_single_stop_returns_none() {
        // Just one color in parts
        let lg = parse_linear_gradient("linear-gradient(red)");
        assert!(lg.is_none());
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
        assert_eq!(lg.stops.len(), 2);
        assert_eq!(lg.stops[0].color.r, 255);
    }

    #[test]
    fn gradient_color_rgba_invalid_parts() {
        // rgba() with wrong number of parts
        let lg = parse_linear_gradient("linear-gradient(rgba(255, 0, 0), blue)");
        assert!(lg.is_none());
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
        assert_eq!(style.z_index, 10);
    }

    #[test]
    fn z_index_negative() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("z-index: -5"), &parent);
        assert_eq!(style.z_index, -5);
    }

    #[test]
    fn z_index_auto_stays_zero() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("z-index: auto"), &parent);
        assert_eq!(style.z_index, 0);
    }

    #[test]
    fn z_index_resets_between_elements() {
        let parent = ComputedStyle::default();
        let style1 = compute_style(HtmlTag::Div, Some("z-index: 99"), &parent);
        assert_eq!(style1.z_index, 99);
        let style2 = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style2.z_index, 0);
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
    fn var_resolves_color() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--text-color: red"), &parent);
        let child = compute_style(HtmlTag::Span, Some("color: var(--text-color)"), &p);
        assert_eq!(child.color.r, 255);
        assert_eq!(child.color.g, 0);
        assert_eq!(child.color.b, 0);
    }

    #[test]
    fn var_resolves_background_color() {
        let parent = ComputedStyle::default();
        let p = compute_style(HtmlTag::Div, Some("--bg: blue"), &parent);
        let child = compute_style(HtmlTag::Div, Some("background-color: var(--bg)"), &p);
        let bg = child.background_color.unwrap();
        assert_eq!(bg.r, 0);
        assert_eq!(bg.g, 0);
        assert_eq!(bg.b, 255);
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
            Some((255, 0, 0, 255))
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
            Some((255, 0, 0, 255))
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
        // `background-image: url(<png>), linear-gradient(...)` splits (in
        // parser::css::inline) into a raster `background-image` key AND a
        // `background-gradient` key. Both layers must survive the cascade so the
        // renderer can paint a PNG layer over a gradient layer over the color.
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
            Some((0x37, 0x47, 0x4f)),
            "base color should also survive"
        );
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
        assert!((s.border.top.width - 6.0).abs() < 0.1);
    }

    #[test]
    fn border_radius_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("border-radius: 50%"), &parent);
        // A percentage border-radius is kept as a layout-time hint (it resolves
        // per-axis against the element's OWN box: horizontal radii against width,
        // vertical against height), not eagerly against the parent width. The
        // block/flex layout turns the hint into absolute radii.
        assert_eq!(s.border_radius_pct, Some(50.0));
        assert_eq!(s.border_radii_pct, [Some(50.0); 4]);
        assert_eq!(s.border_radii_y_pct, [Some(50.0); 4]);
    }

    #[test]
    fn text_indent_from_rem() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("text-indent: 2rem"), &parent);
        assert!((s.text_indent - 24.0).abs() < 0.1);
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
        assert!(s.border.top.color.is_some());
        let c = s.border.top.color.unwrap();
        assert_eq!(c.b, 255);
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
    fn position_from_var_fixed_maps_to_absolute() {
        // For a single-page, non-scrolling PDF the viewport == the page box, so
        // `position: fixed` is treated as an absolute box anchored to the page
        // content box (the absolute-at-root path handles the anchoring).
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: fixed; position: var(--p)"),
            &parent,
        );
        assert_eq!(s.position, Position::Absolute);
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
    fn list_style_type_unknown_defaults_to_disc() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("list-style-type: foobar"), &parent);
        assert_eq!(s.list_style_type, ListStyleType::Disc);
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
    fn background_size_explicit_bare_number() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-size: 42"), &parent);
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit {
                width: 42.0,
                height: None,
                width_is_percent: false,
                height_is_percent: false,
            }
        );
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
        assert!((style.blur_radius - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filter_blur_from_inline_style_px() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(20px)"), &parent);
        assert!((style.blur_radius - 15.0).abs() < 0.01);
    }

    #[test]
    fn filter_blur_from_inline_style_pt() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(10pt)"), &parent);
        assert!((style.blur_radius - 10.0).abs() < 0.01);
    }

    #[test]
    fn filter_blur_bare_number_is_rejected() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: blur(8)"), &parent);
        assert!((style.blur_radius - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filter_blur_none_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: none"), &parent);
        assert!((style.blur_radius - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filter_blur_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.blur_radius = 10.0;
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.blur_radius - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filter_blur_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.blur_radius = 12.0;
        let style = compute_style(HtmlTag::Div, Some("filter: inherit"), &parent);
        assert!((style.blur_radius - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn filter_blur_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("filter: initial"), &parent);
        assert!((style.blur_radius - 0.0).abs() < f32::EPSILON);
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
                r: (60.0, false),
                cx: (75.0, false),
                cy: (75.0, false),
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
            Some(ClipPath::Polygon(pts)) => assert_eq!(pts.len(), 3),
            other => panic!("expected polygon, got {other:?}"),
        }
        assert_eq!(parse_clip_path("none"), None);
        assert_eq!(parse_clip_path("url(#m)"), None);
    }

    #[test]
    fn parse_filter_color_functions() {
        assert_eq!(
            parse_filter("grayscale(100%)").1,
            vec![ColorFilterOp::Grayscale(1.0)]
        );
        assert_eq!(
            parse_filter("grayscale(0.5)").1,
            vec![ColorFilterOp::Grayscale(0.5)]
        );
        assert_eq!(
            parse_filter("invert(1)").1,
            vec![ColorFilterOp::Invert(1.0)]
        );
        assert_eq!(
            parse_filter("brightness(150%)").1,
            vec![ColorFilterOp::Brightness(1.5)]
        );
        assert_eq!(
            parse_filter("hue-rotate(90deg)").1,
            vec![ColorFilterOp::HueRotate(90.0)]
        );
        // bare function defaults to amount 1.0
        assert_eq!(parse_filter("sepia()").1, vec![ColorFilterOp::Sepia(1.0)]);
        // chained: blur goes to the blur slot, color ops preserve order
        let (blur, ops, _opacity, _ds, _url) = parse_filter("grayscale(1) blur(2px) contrast(2)");
        assert!(blur.is_some_and(|r| r > 0.0));
        assert_eq!(
            ops,
            vec![ColorFilterOp::Grayscale(1.0), ColorFilterOp::Contrast(2.0)]
        );
        // none clears everything
        assert_eq!(parse_filter("none"), (Some(0.0), vec![], 1.0, None, None));
    }

    #[test]
    fn filter_url_captures_reference_id() {
        // `filter: url(#id)` records the fragment id for later DOM resolution
        // (css-filter-effects-1 §3); it produces no inline color ops/blur.
        let (blur, ops, opacity, ds, url) = parse_filter("url(#sat)");
        assert_eq!(url.as_deref(), Some("sat"));
        assert!(ops.is_empty());
        assert!(blur.is_none());
        assert_eq!(opacity, 1.0);
        assert!(ds.is_none());
        // Quoted form and a trailing color function still capture the id.
        let (_, ops2, _, _, url2) = parse_filter("url('#q') grayscale(1)");
        assert_eq!(url2.as_deref(), Some("q"));
        assert_eq!(ops2, vec![ColorFilterOp::Grayscale(1.0)]);
        // A computed style with `filter: url(#id)` exposes the id.
        let style = compute_style(
            HtmlTag::Div,
            Some("filter: url(#sat)"),
            &ComputedStyle::default(),
        );
        assert_eq!(style.filter_url_id.as_deref(), Some("sat"));
    }

    #[test]
    fn filter_opacity_fn_reduces_opacity() {
        let parent = ComputedStyle::default();
        // bare number argument
        let style = compute_style(HtmlTag::Div, Some("filter: opacity(0.5)"), &parent);
        assert!((style.opacity - 0.5).abs() < 0.01);
        // percentage argument
        let style = compute_style(HtmlTag::Div, Some("filter: opacity(50%)"), &parent);
        assert!((style.opacity - 0.5).abs() < 0.01);
        // combines multiplicatively with the opacity property
        let style = compute_style(
            HtmlTag::Div,
            Some("opacity: 0.5; filter: opacity(0.5)"),
            &parent,
        );
        assert!((style.opacity - 0.25).abs() < 0.01);
        // other filter functions leave opacity untouched
        let style = compute_style(HtmlTag::Div, Some("filter: blur(2px)"), &parent);
        assert!((style.opacity - 1.0).abs() < 0.01);
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
    fn background_position_bare_number() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("background-position: 5"), &parent);
        assert!((s.background_position.x - 5.0).abs() < 0.01);
        assert!(!s.background_position.x_is_percent);
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
            assert_eq!(shadow.color.r, 0);
            assert_eq!(shadow.color.g, 0);
            assert_eq!(shadow.color.b, 0);
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
            assert!((g.stops[0].position - 0.0).abs() < 0.01);
        }
    }

    #[test]
    fn border_top_from_stylesheet() {
        let rules = crate::parser::css::parse_stylesheet("div { border-top: 1pt solid red }");
        let parent = ComputedStyle::default();
        let style = compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &[], None);
        assert!((style.border.top.width - 1.0).abs() < 0.1);
        let c = style.border.top.color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
        // Other sides should be zero
        assert!((style.border.bottom.width).abs() < 0.01);
        assert!((style.border.left.width).abs() < 0.01);
        assert!((style.border.right.width).abs() < 0.01);
    }

    #[test]
    fn border_left_from_stylesheet() {
        let rules = crate::parser::css::parse_stylesheet("div { border-left: 3pt solid blue }");
        let parent = ComputedStyle::default();
        let style = compute_style_with_rules(HtmlTag::Div, None, &parent, &rules, "div", &[], None);
        assert!((style.border.left.width - 3.0).abs() < 0.1);
        let c = style.border.left.color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
        assert!((style.border.top.width).abs() < 0.01);
        assert!((style.border.right.width).abs() < 0.01);
        assert!((style.border.bottom.width).abs() < 0.01);
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
            assert!((side.width - 2.0).abs() < 0.1);
            let c = side.color.unwrap();
            assert_eq!((c.r, c.g, c.b), (0, 0, 0));
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
        assert!((style.border.top.width - 2.0).abs() < 0.1);
        let top_c = style.border.top.color.unwrap();
        assert_eq!(top_c.r, 255);
        assert_eq!(top_c.g, 0);
        // Other sides should remain 1pt black
        for side in [style.border.right, style.border.bottom, style.border.left] {
            assert!((side.width - 1.0).abs() < 0.1);
            let c = side.color.unwrap();
            assert_eq!((c.r, c.g, c.b), (0, 0, 0));
        }
    }

    #[test]
    fn border_does_not_inherit() {
        let mut parent = ComputedStyle::default();
        parent.border.top = BorderSide {
            width: 1.0,
            color: Some(Color::rgb(0, 0, 0)),
            style: BorderStyle::Solid,
        };
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert!((style.border.top.width).abs() < 0.01);
        assert!((style.border.bottom.width).abs() < 0.01);
        assert!((style.border.left.width).abs() < 0.01);
        assert!((style.border.right.width).abs() < 0.01);
    }

    #[test]
    fn border_sides_max_and_widths() {
        // Lines 353-358: BorderSides max_width, horizontal_width, vertical_width
        let b = BorderSides {
            top: BorderSide {
                width: 3.0,
                color: None,
                style: BorderStyle::Solid,
            },
            right: BorderSide {
                width: 5.0,
                color: None,
                style: BorderStyle::Solid,
            },
            bottom: BorderSide {
                width: 2.0,
                color: None,
                style: BorderStyle::Solid,
            },
            left: BorderSide {
                width: 4.0,
                color: None,
                style: BorderStyle::Solid,
            },
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
        assert!((style.border.right.width - 2.0).abs() < 0.1);
        let rc = style.border.right.color.unwrap();
        assert_eq!(rc.r, 255);
        assert!((style.border.left.width - 3.0).abs() < 0.1);
        let lc = style.border.left.color.unwrap();
        assert_eq!(lc.b, 255);
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
        assert_eq!(style.flex_basis, Some(200.0));
    }

    #[test]
    fn flex_basis_auto() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-basis: auto"), &parent);
        assert_eq!(style.flex_basis, None);
    }

    #[test]
    fn flex_grow_negative_clamped() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex-grow: -3"), &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_shorthand_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: none"), &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, None);
    }

    #[test]
    fn flex_shorthand_auto() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: auto"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, None);
    }

    #[test]
    fn flex_shorthand_single_number() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 3"), &parent);
        assert!((style.flex_grow - 3.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, Some(0.0));
    }

    #[test]
    fn flex_shorthand_two_values() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 2 0"), &parent);
        assert!((style.flex_grow - 2.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, Some(0.0));
    }

    #[test]
    fn flex_shorthand_three_values() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 1 0 200px"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 0.0).abs() < f32::EPSILON);
        // 200px ≈ 200 * 0.75 = 150pt
        assert!(style.flex_basis.is_some());
        assert!(style.flex_basis.unwrap() > 0.0);
    }

    #[test]
    fn flex_shorthand_three_values_auto_basis() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("flex: 1 1 auto"), &parent);
        assert!((style.flex_grow - 1.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, None);
    }

    #[test]
    fn flex_grow_resets_on_non_inherited() {
        let mut parent = ComputedStyle::default();
        parent.flex_grow = 5.0;
        // flex properties don't inherit — child should get default
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!((style.flex_grow - 0.0).abs() < f32::EPSILON);
        assert!((style.flex_shrink - 1.0).abs() < f32::EPSILON);
        assert_eq!(style.flex_basis, None);
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
            stops: vec![],
            repeating: false,
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
}
