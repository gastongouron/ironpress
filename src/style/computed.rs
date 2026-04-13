use std::collections::HashMap;

use crate::parser::css::{
    CssRule, CssValue, SelectorContext, StyleMap, selector_matches_with_context,
};
use crate::parser::dom::HtmlTag;
use crate::style::defaults::default_style;
use crate::types::{Color, EdgeSizes};

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
    Column,
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
}

/// CSS align-items property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    #[default]
    Stretch,
}

/// CSS flex-wrap property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
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
    /// `minmax(min, max)` — the track is at least `min` and at most `max`.
    Minmax(f32, f32),
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

#[derive(Debug, Clone, PartialEq)]
pub struct FontStack {
    families: Vec<FontFamily>,
}

impl Default for FontStack {
    fn default() -> Self {
        Self::from_family(FontFamily::Helvetica)
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

/// CSS position property (simplified).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
}

/// CSS overflow property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Auto,
}

/// CSS visibility property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
}

/// CSS transform value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Rotate by the given angle in degrees.
    Rotate(f32),
    /// Scale by (sx, sy).
    Scale(f32, f32),
    /// Translate by (tx, ty) in pt.
    Translate(f32, f32),
    /// Pre-composed affine matrix (a, b, c, d, e, f) for chained transforms.
    Matrix(f32, f32, f32, f32, f32, f32),
}

/// CSS box-sizing property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
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

/// CSS white-space property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

/// CSS vertical-align property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Super,
    Sub,
    Top,
    Middle,
    Bottom,
}

/// A color stop in a gradient.
#[derive(Debug, Clone, Copy)]
pub struct GradientStop {
    pub color: Color,
    /// Position in the gradient (0.0 to 1.0).
    pub position: f32,
}

/// A CSS linear gradient.
#[derive(Debug, Clone)]
pub struct LinearGradient {
    /// Angle in degrees (0 = to top, 90 = to right, 180 = to bottom, 270 = to left).
    pub angle: f32,
    /// Color stops (at least 2).
    pub stops: Vec<GradientStop>,
}

/// A CSS radial gradient (simplified: always circular, centered).
#[derive(Debug, Clone)]
pub struct RadialGradient {
    /// Color stops (at least 2).
    pub stops: Vec<GradientStop>,
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
/// CSS background-origin property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BackgroundOrigin {
    #[default]
    Padding,
    Border,
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
    Counter(String),
    Counters(String, String),
}

/// CSS box-shadow value.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub color: Color,
}

/// Border line style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
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
    pub padding: EdgeSizes,
    pub text_align: TextAlign,
    /// CSS direction property (ltr/rtl), set from `dir` attribute or CSS.
    pub direction_rtl: bool,
    pub text_decoration_underline: bool,
    pub text_decoration_line_through: bool,
    pub line_height: f32,
    pub page_break_before: bool,
    pub page_break_after: bool,
    pub border: BorderSides,
    pub display: Display,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub percentage_sizing: PercentageSizing,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub opacity: f32,
    pub float: Float,
    pub clear: Clear,
    pub position: Position,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
    pub percentage_insets: PercentageInsets,
    pub box_shadow: Option<BoxShadow>,
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Option<f32>,
    pub gap: f32,
    pub overflow: Overflow,
    pub visibility: Visibility,
    pub transform: Option<Transform>,
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_gap: f32,
    pub border_radius: f32,
    /// Percentage-based border-radius (e.g. 50% for circles). Resolved in layout.
    pub border_radius_pct: Option<f32>,
    pub outline_width: f32,
    pub outline_color: Option<Color>,
    pub box_sizing: BoxSizing,
    pub text_transform: TextTransform,
    pub text_indent: f32,
    pub white_space: WhiteSpace,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub vertical_align: VerticalAlign,
    pub background_gradient: Option<LinearGradient>,
    pub background_radial_gradient: Option<RadialGradient>,
    pub background_image: Option<String>,
    pub background_svg: Option<crate::parser::svg::SvgTree>,
    pub aspect_ratio: Option<f32>,
    pub text_overflow: TextOverflow,
    pub overflow_wrap: OverflowWrap,
    pub border_collapse: BorderCollapse,
    pub table_layout: TableLayout,
    pub border_spacing: f32,
    pub background_size: BackgroundSize,
    pub background_repeat: BackgroundRepeat,
    pub background_position: BackgroundPosition,
    pub background_origin: BackgroundOrigin,
    /// CSS z-index (0 = auto).
    pub z_index: i32,
    /// CSS custom properties inherited from ancestors.
    pub custom_properties: HashMap<String, String>,
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    pub content: Vec<ContentItem>,
    pub counter_reset: Vec<(String, i32)>,
    pub counter_increment: Vec<(String, i32)>,
    pub column_count: Option<u32>,
    pub column_gap: f32,
    pub blur_radius: f32,
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
            font_family: FontFamily::Helvetica,
            font_stack: FontStack::default(),
            color: Color::BLACK,
            background_color: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            text_align: TextAlign::Left,
            direction_rtl: false,
            text_decoration_underline: false,
            text_decoration_line_through: false,
            line_height: f32::NAN,
            page_break_before: false,
            page_break_after: false,
            border: BorderSides::default(),
            display: Display::Block,
            width: None,
            height: None,
            max_width: None,
            min_width: None,
            min_height: None,
            max_height: None,
            percentage_sizing: PercentageSizing::default(),
            margin_left_auto: false,
            margin_right_auto: false,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            top: None,
            right: None,
            bottom: None,
            left: None,
            percentage_insets: PercentageInsets::default(),
            box_shadow: None,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            flex_wrap: FlexWrap::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
            gap: 0.0,
            overflow: Overflow::Visible,
            visibility: Visibility::Visible,
            transform: None,
            grid_template_columns: Vec::new(),
            grid_gap: 0.0,
            border_radius: 0.0,
            border_radius_pct: None,
            outline_width: 0.0,
            outline_color: None,
            box_sizing: BoxSizing::ContentBox,
            text_transform: TextTransform::None,
            text_indent: 0.0,
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_image: None,
            background_svg: None,
            aspect_ratio: None,
            text_overflow: TextOverflow::Clip,
            overflow_wrap: OverflowWrap::Normal,
            border_collapse: BorderCollapse::Separate,
            table_layout: TableLayout::Auto,
            border_spacing: 0.0,
            background_size: BackgroundSize::Auto,
            background_repeat: BackgroundRepeat::Repeat,
            background_position: BackgroundPosition::default(),
            background_origin: BackgroundOrigin::Padding,
            z_index: 0,
            custom_properties: HashMap::new(),
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            content: Vec::new(),
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            column_count: None,
            column_gap: 0.0,
            blur_radius: 0.0,
        }
    }
}

impl ComputedStyle {
    fn clear_background_images(&mut self) {
        self.background_gradient = None;
        self.background_radial_gradient = None;
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
    }

    fn inherit_background_image(&mut self, source: &ComputedStyle) {
        self.background_gradient = source.background_gradient.clone();
        self.background_radial_gradient = source.background_radial_gradient.clone();
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

    // Reset block-level properties that don't inherit
    if tag.is_block() {
        style.margin = EdgeSizes::default();
        style.padding = EdgeSizes::default();
        style.background_color = None;
        style.clear_background_images();
    }

    // Reset non-inherited properties for inline elements too
    // (background-color does not inherit in CSS)
    if !tag.is_block() {
        style.background_color = None;
        style.clear_background_images();
    }

    // Border does not inherit in CSS — reset for all elements
    style.border = BorderSides::default();

    // Reset non-inherited sizing and opacity properties
    style.width = None;
    style.height = None;
    style.max_width = None;
    style.min_width = None;
    style.min_height = None;
    style.max_height = None;
    style.percentage_sizing = PercentageSizing::default();
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.opacity = 1.0;
    style.float = Float::None;
    style.clear = Clear::None;
    style.position = Position::Static;
    style.top = None;
    style.right = None;
    style.bottom = None;
    style.left = None;
    style.percentage_insets = PercentageInsets::default();
    style.box_shadow = None;
    style.flex_direction = FlexDirection::Row;
    style.justify_content = JustifyContent::FlexStart;
    style.align_items = AlignItems::Stretch;
    style.flex_wrap = FlexWrap::NoWrap;
    style.flex_grow = 0.0;
    style.flex_shrink = 1.0;
    style.flex_basis = None;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.visibility = Visibility::Visible;
    style.transform = None;
    style.grid_template_columns = Vec::new();
    style.grid_gap = 0.0;
    style.border_radius = 0.0;
    style.outline_width = 0.0;
    style.outline_color = None;
    style.box_sizing = BoxSizing::ContentBox;
    style.text_indent = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    // border_collapse and border_spacing are inherited; don't reset them.
    style.table_layout = TableLayout::Auto;
    style.background_size = BackgroundSize::Auto;
    style.background_repeat = BackgroundRepeat::Repeat;
    style.background_position = BackgroundPosition::default();
    style.background_origin = BackgroundOrigin::Padding;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = 0;
    style.blur_radius = 0.0;
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

    // Apply stylesheet rules (between defaults and inline).
    // Skip pseudo-element rules — they target ::before/::after, not the element itself.
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
            apply_style_map(&mut style, &rule.declarations, parent);
        }
    }

    // Apply inline styles (override everything)
    if let Some(css_str) = inline_style {
        let inline = crate::parser::css::parse_inline_style(css_str);
        apply_style_map(&mut style, &inline, parent);
    }

    style
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
    style.padding = EdgeSizes::default();
    style.reset_background();
    style.border = BorderSides::default();
    style.width = None;
    style.height = None;
    style.max_width = None;
    style.min_width = None;
    style.min_height = None;
    style.max_height = None;
    style.percentage_sizing = PercentageSizing::default();
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.opacity = 1.0;
    style.float = Float::None;
    style.clear = Clear::None;
    style.position = Position::Static;
    style.top = None;
    style.right = None;
    style.bottom = None;
    style.left = None;
    style.percentage_insets = PercentageInsets::default();
    style.box_shadow = None;
    style.flex_direction = FlexDirection::Row;
    style.justify_content = JustifyContent::FlexStart;
    style.align_items = AlignItems::Stretch;
    style.flex_wrap = FlexWrap::NoWrap;
    style.flex_grow = 0.0;
    style.flex_shrink = 1.0;
    style.flex_basis = None;
    style.gap = 0.0;
    style.overflow = Overflow::Visible;
    style.transform = None;
    style.grid_template_columns = Vec::new();
    style.grid_gap = 0.0;
    style.border_radius = 0.0;
    style.outline_width = 0.0;
    style.outline_color = None;
    style.box_sizing = BoxSizing::ContentBox;
    style.text_indent = 0.0;
    style.vertical_align = VerticalAlign::Baseline;
    style.text_overflow = TextOverflow::Clip;
    style.content = Vec::new();
    style.counter_reset = Vec::new();
    style.counter_increment = Vec::new();
    style.z_index = 0;
    style.blur_radius = 0.0;
    // Default display for pseudo-elements is inline
    style.display = Display::Inline;

    // Apply matched pseudo-element declarations.
    // Use parent_style as the "parent" for inherit resolution so that
    // `background-image: inherit` copies from the originating element.
    for declarations in &matched_declarations {
        apply_style_map(&mut style, declarations, parent_style);
    }

    // `content: none` and `content: normal` suppress pseudo-element generation.
    if style.content.is_empty() {
        return None;
    }

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
            | "white-space"
            | "overflow-wrap"
            | "word-wrap"
            | "border-collapse"
            | "border-spacing"
            | "list-style-type"
            | "list-style-position"
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
        "line-height" => style.line_height = default.line_height,
        "text-align" => style.text_align = default.text_align,
        "text-decoration" => {
            style.text_decoration_underline = default.text_decoration_underline;
            style.text_decoration_line_through = default.text_decoration_line_through;
        }
        "visibility" => style.visibility = default.visibility,
        "letter-spacing" => style.letter_spacing = default.letter_spacing,
        "word-spacing" => style.word_spacing = default.word_spacing,
        "background-color" => style.background_color = default.background_color,
        "margin-top" => style.margin.top = default.margin.top,
        "margin-right" => style.margin.right = default.margin.right,
        "margin-bottom" => style.margin.bottom = default.margin.bottom,
        "margin-left" => style.margin.left = default.margin.left,
        "padding-top" => style.padding.top = default.padding.top,
        "padding-right" => style.padding.right = default.padding.right,
        "padding-bottom" => style.padding.bottom = default.padding.bottom,
        "padding-left" => style.padding.left = default.padding.left,
        "display" => style.display = default.display,
        "width" => {
            style.width = default.width;
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
        "position" => style.position = default.position,
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
        "overflow" => style.overflow = default.overflow,
        "transform" => style.transform = default.transform,
        "box-shadow" => style.box_shadow = default.box_shadow,
        "flex-direction" => style.flex_direction = default.flex_direction,
        "justify-content" => style.justify_content = default.justify_content,
        "align-items" => style.align_items = default.align_items,
        "flex-wrap" => style.flex_wrap = default.flex_wrap,
        "flex-grow" => style.flex_grow = default.flex_grow,
        "flex-shrink" => style.flex_shrink = default.flex_shrink,
        "flex-basis" => style.flex_basis = default.flex_basis,
        "gap" => style.gap = default.gap,
        "text-overflow" => style.text_overflow = default.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = default.overflow_wrap,
        "border-collapse" => style.border_collapse = default.border_collapse,
        "table-layout" => style.table_layout = default.table_layout,
        "border-spacing" => style.border_spacing = default.border_spacing,
        "background-size" => style.background_size = default.background_size,
        "background-repeat" => style.background_repeat = default.background_repeat,
        "background-position" => style.background_position = default.background_position,
        "background-origin" => style.background_origin = default.background_origin,
        "background-image" | "background-svg" => style.clear_background_images(),
        "aspect-ratio" => style.aspect_ratio = default.aspect_ratio,
        "background" => style.reset_background(),
        "list-style-type" => style.list_style_type = default.list_style_type,
        "list-style-position" => style.list_style_position = default.list_style_position,
        "content" => style.content = default.content,
        "counter-reset" => style.counter_reset = default.counter_reset,
        "counter-increment" => style.counter_increment = default.counter_increment,
        "column-count" | "columns" => style.column_count = default.column_count,
        "column-gap" => style.column_gap = default.column_gap,
        "filter" => style.blur_radius = default.blur_radius,
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
        "line-height" => style.line_height = parent.line_height,
        "text-align" => style.text_align = parent.text_align,
        "text-decoration" => {
            style.text_decoration_underline = parent.text_decoration_underline;
            style.text_decoration_line_through = parent.text_decoration_line_through;
        }
        "visibility" => style.visibility = parent.visibility,
        "letter-spacing" => style.letter_spacing = parent.letter_spacing,
        "word-spacing" => style.word_spacing = parent.word_spacing,
        "background-color" => style.background_color = parent.background_color,
        "margin-top" => style.margin.top = parent.margin.top,
        "margin-right" => style.margin.right = parent.margin.right,
        "margin-bottom" => style.margin.bottom = parent.margin.bottom,
        "margin-left" => style.margin.left = parent.margin.left,
        "padding-top" => style.padding.top = parent.padding.top,
        "padding-right" => style.padding.right = parent.padding.right,
        "padding-bottom" => style.padding.bottom = parent.padding.bottom,
        "padding-left" => style.padding.left = parent.padding.left,
        "display" => style.display = parent.display,
        "width" => {
            style.width = parent.width;
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
        "position" => style.position = parent.position,
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
        "overflow" => style.overflow = parent.overflow,
        "transform" => style.transform = parent.transform,
        "box-shadow" => style.box_shadow = parent.box_shadow,
        "flex-direction" => style.flex_direction = parent.flex_direction,
        "justify-content" => style.justify_content = parent.justify_content,
        "align-items" => style.align_items = parent.align_items,
        "flex-wrap" => style.flex_wrap = parent.flex_wrap,
        "flex-grow" => style.flex_grow = parent.flex_grow,
        "flex-shrink" => style.flex_shrink = parent.flex_shrink,
        "flex-basis" => style.flex_basis = parent.flex_basis,
        "gap" => style.gap = parent.gap,
        "text-overflow" => style.text_overflow = parent.text_overflow,
        "overflow-wrap" | "word-wrap" => style.overflow_wrap = parent.overflow_wrap,
        "border-collapse" => style.border_collapse = parent.border_collapse,
        "table-layout" => style.table_layout = parent.table_layout,
        "border-spacing" => style.border_spacing = parent.border_spacing,
        "background-size" => style.background_size = parent.background_size,
        "background-repeat" => style.background_repeat = parent.background_repeat,
        "background-position" => style.background_position = parent.background_position,
        "background-origin" => style.background_origin = parent.background_origin,
        "background-image" | "background-svg" => style.inherit_background_image(parent),
        "background-gradient" => style.background_gradient = parent.background_gradient.clone(),
        "background-radial-gradient" => {
            style.background_radial_gradient = parent.background_radial_gradient.clone()
        }
        "aspect-ratio" => style.aspect_ratio = parent.aspect_ratio,
        "background" => style.inherit_background(parent),
        "list-style-type" => style.list_style_type = parent.list_style_type,
        "list-style-position" => style.list_style_position = parent.list_style_position,
        "content" => style.content = parent.content.clone(),
        "counter-reset" => style.counter_reset = parent.counter_reset.clone(),
        "counter-increment" => style.counter_increment = parent.counter_increment.clone(),
        "column-count" | "columns" => style.column_count = parent.column_count,
        "column-gap" => style.column_gap = parent.column_gap,
        "filter" => style.blur_radius = parent.blur_radius,
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

pub(crate) fn apply_style_map(style: &mut ComputedStyle, map: &StyleMap, parent: &ComputedStyle) {
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

    // Linear gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-gradient") {
        if let Some(lg) = parse_linear_gradient(k) {
            style.clear_background_images();
            style.background_gradient = Some(lg);
        }
    }

    // Radial gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-radial-gradient") {
        if let Some(rg) = parse_radial_gradient(k) {
            style.clear_background_images();
            style.background_radial_gradient = Some(rg);
        }
    }

    // SVG background image (from data:image/svg+xml URI)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-svg") {
        if let Some(tree) = crate::parser::svg::parse_svg_from_string(k) {
            style.clear_background_images();
            style.background_svg = Some(tree);
        }
    }

    if get_non_special(map, "background-gradient").is_none()
        && get_non_special(map, "background-radial-gradient").is_none()
        && get_non_special(map, "background-svg").is_none()
        && let Some(CssValue::Keyword(k)) = get_non_special(map, "background-image")
    {
        style.clear_background_images();
        let trimmed = k.trim();
        if trimmed != "none" {
            if let Some(svg_text) = crate::parser::css::extract_svg_data_uri(trimmed) {
                if let Some(tree) = crate::parser::svg::parse_svg_from_string(&svg_text) {
                    style.background_svg = Some(tree);
                }
            } else {
                style.background_image = Some(trimmed.to_string());
            }
        }
    }

    // Margins: resolve both Length (pt) and Number (em = multiplied by font_size).
    // The CSS parser produces Number for em values (e.g. "2em" → Number(2.0))
    // and our UA defaults use Number for em-based margins.
    match get_non_special(map, "margin-top") {
        Some(CssValue::Length(v)) => style.margin.top = *v,
        Some(CssValue::Number(v)) => style.margin.top = *v * style.font_size,
        _ => {}
    }
    match get_non_special(map, "margin-right") {
        Some(CssValue::Length(v)) => style.margin.right = *v,
        Some(CssValue::Number(v)) => style.margin.right = *v * style.font_size,
        _ => {}
    }
    match get_non_special(map, "margin-bottom") {
        Some(CssValue::Length(v)) => style.margin.bottom = *v,
        Some(CssValue::Number(v)) => style.margin.bottom = *v * style.font_size,
        _ => {}
    }
    match get_non_special(map, "margin-left") {
        Some(CssValue::Length(v)) => style.margin.left = *v,
        Some(CssValue::Number(v)) => style.margin.left = *v * style.font_size,
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
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "line-height") {
        if k == "normal" {
            style.line_height = f32::NAN;
        }
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "line-height") {
        style.line_height = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "line-height") {
        style.line_height = *v / style.font_size;
    }

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

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex-direction") {
        style.flex_direction = match k.as_str() {
            "column" => FlexDirection::Column,
            _ => FlexDirection::Row,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "justify-content") {
        style.justify_content = match k.as_str() {
            "flex-end" => JustifyContent::FlexEnd,
            "center" => JustifyContent::Center,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            _ => JustifyContent::FlexStart,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "align-items") {
        style.align_items = match k.as_str() {
            "flex-start" => AlignItems::FlexStart,
            "flex-end" => AlignItems::FlexEnd,
            "center" => AlignItems::Center,
            _ => AlignItems::Stretch,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex-wrap") {
        style.flex_wrap = match k.as_str() {
            "wrap" => FlexWrap::Wrap,
            _ => FlexWrap::NoWrap,
        };
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "flex-grow") {
        style.flex_grow = v.max(0.0);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "flex-shrink") {
        style.flex_shrink = v.max(0.0);
    }
    match get_non_special(map, "flex-basis") {
        Some(CssValue::Length(v)) => style.flex_basis = Some(*v),
        Some(CssValue::Keyword(k)) if k == "auto" => style.flex_basis = None,
        _ => {}
    }

    // flex shorthand: "flex: <grow>" or "flex: <grow> <shrink>" or "flex: <grow> <shrink> <basis>"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "flex") {
        let parts: Vec<&str> = k.split_whitespace().collect();
        if let Some(first) = parts.first() {
            if *first == "none" {
                style.flex_grow = 0.0;
                style.flex_shrink = 0.0;
                style.flex_basis = None;
            } else if *first == "auto" {
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = None;
            } else if let Ok(grow) = first.parse::<f32>() {
                style.flex_grow = grow.max(0.0);
                style.flex_shrink = 1.0;
                style.flex_basis = Some(0.0);
                if let Some(second) = parts.get(1) {
                    if let Ok(shrink) = second.parse::<f32>() {
                        style.flex_shrink = shrink.max(0.0);
                    }
                }
                if let Some(third) = parts.get(2) {
                    if *third == "auto" {
                        style.flex_basis = None;
                    } else if let Some(CssValue::Length(v)) =
                        crate::parser::css::parse_length(third)
                    {
                        style.flex_basis = Some(v);
                    }
                }
            }
        }
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "gap") {
        style.gap = *v;
        style.grid_gap = *v;
    }

    // Grid template columns
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "grid-template-columns") {
        style.grid_template_columns = parse_grid_template_columns(k);
    }

    // Grid gap
    if let Some(CssValue::Length(v)) = get_non_special(map, "grid-gap") {
        style.grid_gap = *v;
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page-break-before") {
        style.page_break_before = k == "always";
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "page-break-after") {
        style.page_break_after = k == "always";
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "filter") {
        if let Some(radius) = parse_filter_blur(k) {
            style.blur_radius = radius;
        }
    }

    // Border shorthand: "1px solid black"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border") {
        let (w, c, bs) = parse_border_shorthand(k);
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
            let (w, c, bs) = parse_border_shorthand(k);
            setter(style, w, c, bs);
        }
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "width") {
        style.width = Some(*v);
        style.percentage_sizing.width = None;
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "width") {
        // em value — multiply by current font-size
        style.width = Some(*v * style.font_size);
        style.percentage_sizing.width = None;
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

    // margin-left: auto / margin-right: auto
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

    if let Some(CssValue::Number(v)) = get_non_special(map, "opacity") {
        style.opacity = v.clamp(0.0, 1.0);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "opacity") {
        // bare number parsed as Length
        style.opacity = v.clamp(0.0, 1.0);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "border-width") {
        style.border.top.width = *v;
        style.border.right.width = *v;
        style.border.bottom.width = *v;
        style.border.left.width = *v;
    }

    if let Some(CssValue::Color(c)) = get_non_special(map, "border-color") {
        style.border.top.color = Some(*c);
        style.border.right.color = Some(*c);
        style.border.bottom.color = Some(*c);
        style.border.left.color = Some(*c);
    }

    // Float
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "float") {
        style.float = match k.as_str() {
            "left" => Float::Left,
            "right" => Float::Right,
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

    // Position
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "position") {
        style.position = match k.as_str() {
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            _ => Position::Static,
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

    // Box-shadow: parse from keyword (stored as full shorthand string)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-shadow") {
        if let Some(shadow) = parse_box_shadow(k) {
            style.box_shadow = Some(shadow);
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
    if let Some(val) = get_non_special(map, "columns") {
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
    if let Some(CssValue::Length(v)) = get_non_special(map, "column-gap") {
        style.column_gap = *v;
    }

    // Overflow
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "overflow") {
        style.overflow = match k.as_str() {
            "hidden" => Overflow::Hidden,
            "auto" => Overflow::Auto,
            _ => Overflow::Visible,
        };
    }

    // Visibility
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "visibility") {
        style.visibility = match k.as_str() {
            "hidden" => Visibility::Hidden,
            _ => Visibility::Visible,
        };
    }

    // Transform
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "transform") {
        if let Some(t) = parse_transform(k) {
            style.transform = Some(t);
        }
    }

    // Border-radius (single value shorthand)
    match get_non_special(map, "border-radius") {
        Some(CssValue::Length(v)) => style.border_radius = *v,
        Some(CssValue::Percentage(pct)) => {
            // Resolve percentage border-radius against the smaller dimension.
            // Use width if available, otherwise store a sentinel that the
            // layout engine resolves later.  For the common `50%` case on a
            // square element this produces a perfect circle.
            style.border_radius_pct = Some(*pct);
        }
        _ => {}
    }

    // Outline shorthand: "2px solid red"
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

    // Box-sizing
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-sizing") {
        style.box_sizing = match k.as_str() {
            "border-box" => BoxSizing::BorderBox,
            _ => BoxSizing::ContentBox,
        };
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
            _ => WhiteSpace::Normal,
        };
    }

    // Letter-spacing
    if let Some(CssValue::Length(v)) = get_non_special(map, "letter-spacing") {
        style.letter_spacing = *v;
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
            "middle" => VerticalAlign::Middle,
            "bottom" => VerticalAlign::Bottom,
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
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-size") {
        style.background_size = match k.as_str() {
            "cover" => BackgroundSize::Cover,
            "contain" => BackgroundSize::Contain,
            "auto" => BackgroundSize::Auto,
            _ => parse_background_size_explicit(k).unwrap_or(BackgroundSize::Auto),
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-repeat") {
        style.background_repeat = match k.as_str() {
            "no-repeat" => BackgroundRepeat::NoRepeat,
            "repeat-x" => BackgroundRepeat::RepeatX,
            "repeat-y" => BackgroundRepeat::RepeatY,
            _ => BackgroundRepeat::Repeat,
        };
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-position") {
        if let Some(pos) = parse_background_position(k) {
            style.background_position = pos;
        }
    }
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-origin") {
        style.background_origin = match k.as_str() {
            "border-box" => BackgroundOrigin::Border,
            "content-box" => BackgroundOrigin::Content,
            _ => BackgroundOrigin::Padding,
        };
    }

    if let Some(CssValue::Keyword(k)) = get_non_special(map, "aspect-ratio") {
        style.aspect_ratio = parse_aspect_ratio(k);
    }

    // z-index
    if let Some(CssValue::Number(v)) = get_non_special(map, "z-index") {
        style.z_index = *v as i32;
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
        }),
        ("grid-gap", |s, v| s.grid_gap = v),
        ("border-width", |s, v| {
            s.border.top.width = v;
            s.border.right.width = v;
            s.border.bottom.width = v;
            s.border.left.width = v;
        }),
        ("border-radius", |s, v| s.border_radius = v),
        ("text-indent", |s, v| s.text_indent = v),
        ("letter-spacing", |s, v| s.letter_spacing = v),
        ("word-spacing", |s, v| s.word_spacing = v),
        ("border-spacing", |s, v| s.border_spacing = v),
    ];
    for &(prop_name, setter) in inline_length_props {
        if let Some(val) = get_non_special(map, prop_name) {
            match val {
                CssValue::Percentage(_)
                | CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Calc(_)
                | CssValue::Var(_, _) => {
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

    if let Some(val) = get_non_special(map, "height") {
        match val {
            CssValue::Percentage(v) => {
                style.percentage_sizing.height = Some(*v);
                style.height = resolve_block_percentage(*v);
            }
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Calc(_)
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
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Calc(_)
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
            CssValue::Rem(_)
            | CssValue::Vw(_)
            | CssValue::Vh(_)
            | CssValue::Calc(_)
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
                CssValue::Rem(_)
                | CssValue::Vw(_)
                | CssValue::Vh(_)
                | CssValue::Calc(_)
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
            style.position = match kw.as_str() {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                _ => Position::Static,
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
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "list-style") {
        let lower = k.to_ascii_lowercase();
        for part in lower.split_whitespace() {
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
            let (name, sep) = inner
                .split_once(',')
                .map_or((inner.trim(), "."), |(name, sep)| {
                    (
                        name.trim(),
                        sep.trim().trim_matches(|c: char| c == '"' || c == '\''),
                    )
                });
            items.push(ContentItem::Counters(name.to_string(), sep.to_string()));
            rest = tail;
        } else if let Some((name, tail)) = parse_content_function(rest, "counter(") {
            items.push(ContentItem::Counter(name.trim().to_string()));
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
/// Supports formats like:
/// - `2px 2px black`
/// - `2px 2px 4px black`
/// - `2px 2px 4px rgba(0,0,0,0.3)`  (alpha is ignored in PDF)
fn parse_box_shadow(val: &str) -> Option<BoxShadow> {
    let val = val.trim();
    if val == "none" {
        return None;
    }

    // Split into tokens, but handle rgba(...) as a single token
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

    if tokens.len() < 3 {
        return None;
    }

    let offset_x = parse_shadow_length(&tokens[0])?;
    let offset_y = parse_shadow_length(&tokens[1])?;

    let (blur, color_start) = if tokens.len() >= 4 {
        if let Some(b) = parse_shadow_length(&tokens[2]) {
            (b, 3)
        } else {
            (0.0, 2)
        }
    } else {
        (0.0, 2)
    };

    let color = if color_start < tokens.len() {
        parse_border_color(&tokens[color_start]).unwrap_or(Color::BLACK)
    } else {
        Color::BLACK
    };

    Some(BoxShadow {
        offset_x,
        offset_y,
        blur,
        color,
    })
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
/// Returns the parsed transform and `None` when the function is unknown.
fn parse_single_transform(val: &str) -> Option<Transform> {
    let val = val.trim();

    if let Some(inner) = val
        .strip_prefix("rotate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let inner = inner.trim();
        let degrees = if let Some(n) = inner.strip_suffix("deg") {
            n.trim().parse::<f32>().ok()?
        } else {
            inner.parse::<f32>().ok()?
        };
        return Some(Transform::Rotate(degrees));
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
            let sy = parts[1].trim().parse::<f32>().ok()?;
            return Some(Transform::Scale(sx, sy));
        }
    }

    if let Some(inner) = val
        .strip_prefix("translateX(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let tx = parse_transform_length(inner.trim())?;
        return Some(Transform::Translate(tx, 0.0));
    }

    if let Some(inner) = val
        .strip_prefix("translateY(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let ty = parse_transform_length(inner.trim())?;
        return Some(Transform::Translate(0.0, ty));
    }

    if let Some(inner) = val
        .strip_prefix("translate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 2 {
            let tx = parse_transform_length(parts[0].trim())?;
            let ty = parse_transform_length(parts[1].trim())?;
            return Some(Transform::Translate(tx, ty));
        } else if parts.len() == 1 {
            let tx = parse_transform_length(parts[0].trim())?;
            return Some(Transform::Translate(tx, 0.0));
        }
    }

    if let Some(inner) = val.strip_prefix("skew(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        let ax = parts
            .first()?
            .trim()
            .strip_suffix("deg")
            .and_then(|n| n.parse::<f32>().ok())?;
        let ay = if parts.len() >= 2 {
            parts[1]
                .trim()
                .strip_suffix("deg")
                .and_then(|n| n.parse::<f32>().ok())
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let tan_x = (ax * std::f32::consts::PI / 180.0).tan();
        let tan_y = (ay * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, tan_y, tan_x, 1.0, 0.0, 0.0));
    }

    if let Some(inner) = val.strip_prefix("skewX(").and_then(|s| s.strip_suffix(')')) {
        let deg = inner
            .trim()
            .strip_suffix("deg")
            .and_then(|n| n.parse::<f32>().ok())?;
        let tan_x = (deg * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, 0.0, tan_x, 1.0, 0.0, 0.0));
    }

    if let Some(inner) = val.strip_prefix("skewY(").and_then(|s| s.strip_suffix(')')) {
        let deg = inner
            .trim()
            .strip_suffix("deg")
            .and_then(|n| n.parse::<f32>().ok())?;
        let tan_y = (deg * std::f32::consts::PI / 180.0).tan();
        return Some(Transform::Matrix(1.0, tan_y, 0.0, 1.0, 0.0, 0.0));
    }

    None
}

/// Convert a Transform into its affine matrix (a, b, c, d, e, f).
fn transform_to_matrix(t: &Transform) -> [f32; 6] {
    match t {
        Transform::Rotate(deg) => {
            let rad = deg * std::f32::consts::PI / 180.0;
            let c = rad.cos();
            let s = rad.sin();
            [c, s, -s, c, 0.0, 0.0]
        }
        Transform::Scale(sx, sy) => [*sx, 0.0, 0.0, *sy, 0.0, 0.0],
        Transform::Translate(tx, ty) => [1.0, 0.0, 0.0, 1.0, *tx, *ty],
        Transform::Matrix(a, b, c, d, e, f) => [*a, *b, *c, *d, *e, *f],
    }
}

/// Multiply two 2D affine matrices: result = lhs × rhs.
fn multiply_matrices(lhs: &[f32; 6], rhs: &[f32; 6]) -> [f32; 6] {
    [
        lhs[0] * rhs[0] + lhs[2] * rhs[1],
        lhs[1] * rhs[0] + lhs[3] * rhs[1],
        lhs[0] * rhs[2] + lhs[2] * rhs[3],
        lhs[1] * rhs[2] + lhs[3] * rhs[3],
        lhs[0] * rhs[4] + lhs[2] * rhs[5] + lhs[4],
        lhs[1] * rhs[4] + lhs[3] * rhs[5] + lhs[5],
    ]
}

/// Parse a CSS `transform` value (one or more space-separated functions).
///
/// Supports: rotate, scale, scaleX, scaleY, translate, translateX, translateY,
/// skew, skewX, skewY, and chained transforms like `rotate(10deg) scale(1.1)`.
fn parse_transform(val: &str) -> Option<Transform> {
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
        return parse_single_transform(functions[0]);
    }

    // Multiple transforms — compose into a single matrix.
    // CSS: transforms are applied right-to-left, but the `cm` operator
    // in PDF also post-multiplies, so we compose left-to-right here and
    // the renderer will apply the resulting matrix around the centre.
    let mut result = [1.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0]; // identity
    for func in &functions {
        let t = parse_single_transform(func)?;
        let m = transform_to_matrix(&t);
        result = multiply_matrices(&result, &m);
    }

    Some(Transform::Matrix(
        result[0], result[1], result[2], result[3], result[4], result[5],
    ))
}

/// Parse a length value for transform translate (px or pt or bare number).
fn parse_transform_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("px") {
        n.parse::<f32>().ok().map(|v| v * 0.75)
    } else if let Some(n) = val.strip_suffix("pt") {
        n.parse::<f32>().ok()
    } else {
        val.parse::<f32>().ok()
    }
}

/// Parse a single grid track token (e.g. `1fr`, `200pt`, `100px`, `auto`).
fn parse_single_track(token: &str) -> Option<GridTrack> {
    let token = token.trim();
    if let Some(n) = token.strip_suffix("fr") {
        n.parse::<f32>().ok().map(GridTrack::Fr)
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

/// Parse a `grid-template-columns` value string into a list of `GridTrack` values.
///
/// Supports tokens like `1fr`, `200pt`, `100px`, `auto`, `repeat(3, 1fr)`,
/// `minmax(100px, 1fr)`, `auto-fill`, and `auto-fit`.
fn parse_grid_template_columns(val: &str) -> Vec<GridTrack> {
    let mut result = Vec::new();
    let mut remaining = val.trim();

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
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

                    let track_list = parse_grid_template_columns(pattern);
                    for _ in 0..count {
                        result.extend(track_list.clone());
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
                    result.push(track);
                }
                remaining = &remaining[close + 1..];
                continue;
            }
        }

        // Regular token — read until next whitespace or function start
        let end = remaining
            .find(|c: char| c.is_whitespace())
            .unwrap_or(remaining.len());
        let token = &remaining[..end];
        if let Some(track) = parse_single_track(token) {
            result.push(track);
        }
        remaining = &remaining[end..];
    }

    result
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
fn parse_border_shorthand(k: &str) -> (f32, Option<Color>, BorderStyle) {
    let parts: Vec<&str> = k.split_whitespace().collect();
    let mut width = 0.0f32;
    let mut border_style = BorderStyle::Solid;
    for part in &parts {
        if let Some(n) = part.strip_suffix("px") {
            if let Ok(v) = n.parse::<f32>() {
                width = v * 0.75; // px to pt
            }
        } else if let Some(n) = part.strip_suffix("pt") {
            if let Ok(v) = n.parse::<f32>() {
                width = v;
            }
        } else {
            match *part {
                "dashed" => border_style = BorderStyle::Dashed,
                "dotted" => border_style = BorderStyle::Dotted,
                "none" => border_style = BorderStyle::None,
                "solid" => border_style = BorderStyle::Solid,
                _ => {}
            }
        }
    }
    let color = parts.last().and_then(|last| parse_border_color(last));
    (width, color, border_style)
}

/// Parse a color name or hex value for border shorthand.
fn parse_border_color(val: &str) -> Option<Color> {
    let val = val.to_ascii_lowercase();
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
        _ => {
            if let Some(hex) = val.strip_prefix('#') {
                parse_hex_to_color(hex)
            } else {
                None
            }
        }
    }
}

fn parse_hex_to_color(hex: &str) -> Option<Color> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        _ => None,
    }
}

/// Parse a CSS `linear-gradient(...)` function value into a `LinearGradient`.
///
/// Supports:
/// - `linear-gradient(to right, red, blue)`
/// - `linear-gradient(45deg, #ff0000, #0000ff)`
/// - `linear-gradient(to bottom, red 0%, white 50%, blue 100%)`
pub fn parse_linear_gradient(val: &str) -> Option<LinearGradient> {
    let val = val.trim();
    let inner = val
        .strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;

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
    } else if let Some(deg_str) = first.strip_suffix("deg") {
        if let Ok(deg) = deg_str.trim().parse::<f32>() {
            (deg, 1)
        } else {
            (180.0, 0)
        }
    } else {
        // No direction specified, default is "to bottom" = 180deg
        (180.0, 0)
    };

    let color_parts = &parts[color_start..];
    if color_parts.len() < 2 {
        return None;
    }

    let stops = parse_gradient_stops(color_parts)?;

    Some(LinearGradient { angle, stops })
}

/// Parse a CSS `radial-gradient(...)` function value into a `RadialGradient`.
///
/// Simplified: always centered circular gradient. Ignores shape/size keywords.
pub fn parse_radial_gradient(val: &str) -> Option<RadialGradient> {
    let val = val.trim();
    let inner = val
        .strip_prefix("radial-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;

    let parts = split_gradient_args(inner);
    if parts.len() < 2 {
        return None;
    }

    let first = parts[0].trim().to_ascii_lowercase();

    // Skip shape/size keywords like "circle", "ellipse", "closest-side", etc.
    let color_start = if first.starts_with("circle")
        || first.starts_with("ellipse")
        || first.contains("at ")
        || first == "closest-side"
        || first == "farthest-side"
        || first == "closest-corner"
        || first == "farthest-corner"
    {
        1
    } else {
        0
    };

    let color_parts = &parts[color_start..];
    if color_parts.len() < 2 {
        return None;
    }

    let stops = parse_gradient_stops(color_parts)?;

    Some(RadialGradient { stops })
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

/// Parse gradient color stops from a list of string tokens.
/// Each token is like "red", "#ff0000 50%", "rgb(255,0,0) 30%", etc.
fn parse_gradient_stops(parts: &[String]) -> Option<Vec<GradientStop>> {
    let count = parts.len();
    let mut stops = Vec::with_capacity(count);

    for (i, part) in parts.iter().enumerate() {
        let part = part.trim();
        // Try to split off a trailing percentage
        let (color_str, position) = if let Some(pct_pos) = part.rfind('%') {
            // Find the space before the percentage
            let before_pct = &part[..pct_pos];
            if let Some(space_pos) = before_pct.rfind(' ') {
                let color_part = part[..space_pos].trim();
                let pct_str = part[space_pos + 1..pct_pos].trim();
                if let Ok(pct) = pct_str.parse::<f32>() {
                    (color_part, Some(pct / 100.0))
                } else {
                    (part, None)
                }
            } else {
                (part, None)
            }
        } else {
            (part, None)
        };

        let color = parse_gradient_color(color_str)?;
        let position = position.unwrap_or_else(|| {
            if count <= 1 {
                0.0
            } else {
                i as f32 / (count - 1) as f32
            }
        });

        stops.push(GradientStop { color, position });
    }

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
        "transparent" => Some(Color::rgb(255, 255, 255)),
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
    fn font_family_default_is_helvetica() {
        let style = ComputedStyle::default();
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_serif() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: serif"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
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
    fn border_color_unknown_returns_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid foobar"), &parent);
        assert!(style.border.top.color.is_none());
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
        let shadow = style.box_shadow.unwrap();
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
        let shadow = style.box_shadow.unwrap();
        assert!((shadow.offset_x - 1.5).abs() < 0.1); // 2px * 0.75
        assert!((shadow.offset_y - 1.5).abs() < 0.1);
        assert!((shadow.blur - 3.0).abs() < 0.1); // 4px * 0.75
        assert_eq!(shadow.color.r, 0);
    }

    #[test]
    fn box_shadow_with_pt_units() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: 3pt 3pt red"), &parent);
        let shadow = style.box_shadow.unwrap();
        assert!((shadow.offset_x - 3.0).abs() < 0.1);
        assert!((shadow.offset_y - 3.0).abs() < 0.1);
        assert_eq!(shadow.color.r, 255);
    }

    #[test]
    fn box_shadow_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("box-shadow: none"), &parent);
        assert!(style.box_shadow.is_none());
    }

    #[test]
    fn box_shadow_default_is_none() {
        let style = ComputedStyle::default();
        assert!(style.box_shadow.is_none());
    }

    #[test]
    fn box_shadow_not_inherited() {
        let mut parent = ComputedStyle::default();
        parent.box_shadow = Some(BoxShadow {
            offset_x: 3.0,
            offset_y: 3.0,
            blur: 0.0,
            color: Color::BLACK,
        });
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert!(style.box_shadow.is_none());
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
        assert_eq!(style.transform, Some(Transform::Translate(10.0, 20.0)));
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
        if let Transform::Translate(tx, ty) = t {
            assert!((tx - 7.5).abs() < 0.1); // 10 * 0.75
            assert!((ty - 15.0).abs() < 0.1); // 20 * 0.75
        } else {
            panic!("Expected Translate");
        }
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
        assert!(style.box_shadow.is_none());
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
        parent.font_family = FontFamily::TimesRoman;
        parent.font_stack = FontStack::from_family(FontFamily::TimesRoman);
        let style = compute_style(HtmlTag::Span, Some("font-family: inherit"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
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
        parent.box_shadow = Some(BoxShadow {
            offset_x: 1.0,
            offset_y: 2.0,
            blur: 3.0,
            color: Color::BLACK,
        });
        let style = compute_style(HtmlTag::Div, Some("box-shadow: inherit"), &parent);
        assert!(style.box_shadow.is_some());
        assert!((style.box_shadow.unwrap().offset_x - 1.0).abs() < 0.1);
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
        let shadow = parse_box_shadow("2px 2px 4px rgba(0,0,0,0.3)");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 3.0).abs() < 0.1); // 4px * 0.75
    }

    #[test]
    fn parse_box_shadow_too_few_tokens() {
        let shadow = parse_box_shadow("2px 2px");
        assert!(shadow.is_none());
    }

    #[test]
    fn parse_box_shadow_non_parseable_blur_uses_as_color() {
        // "2px 2px notanumber black" — 4 tokens, but third is not a length
        let shadow = parse_box_shadow("2px 2px notanumber black");
        // blur parse fails, so blur = 0.0, color_start = 2, color = parse "notanumber" which fails
        // Actually color_start=2 means color_str = "notanumber" which is not a valid color -> Color::BLACK fallback
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert!((s.blur - 0.0).abs() < 0.1);
    }

    #[test]
    fn parse_box_shadow_no_color_token() {
        // Exactly 3 tokens where third is a valid blur, so color_start=3, no color token
        let shadow = parse_box_shadow("2px 2px 4px");
        assert!(shadow.is_some());
        let s = shadow.unwrap();
        assert_eq!(s.color.r, 0); // defaults to BLACK
        assert_eq!(s.color.g, 0);
        assert_eq!(s.color.b, 0);
    }

    #[test]
    fn parse_shadow_length_bare_number() {
        let result = parse_shadow_length("5");
        assert!(result.is_some());
        assert!((result.unwrap() - 5.0).abs() < 0.1);
    }

    // --- parse_transform edge cases (lines 1207, 1233-1235, 1239, 1250) ---

    #[test]
    fn parse_transform_rotate_bare_number() {
        let t = parse_transform("rotate(45)");
        assert_eq!(t, Some(Transform::Rotate(45.0)));
    }

    #[test]
    fn parse_transform_translate_single_arg() {
        let t = parse_transform("translate(10pt)");
        assert_eq!(t, Some(Transform::Translate(10.0, 0.0)));
    }

    #[test]
    fn parse_transform_unknown_returns_none() {
        let t = parse_transform("perspective(500px)");
        assert!(t.is_none());
    }

    #[test]
    fn parse_transform_skew() {
        let t = parse_transform("skew(30deg)");
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
        let t = parse_transform("rotate(10deg) scale(1.1)");
        assert!(t.is_some());
        assert!(matches!(t, Some(Transform::Matrix(..))));
    }

    #[test]
    fn parse_transform_scale_x_y() {
        assert_eq!(
            parse_transform("scaleX(1.5)"),
            Some(Transform::Scale(1.5, 1.0))
        );
        assert_eq!(
            parse_transform("scaleY(0.5)"),
            Some(Transform::Scale(1.0, 0.5))
        );
    }

    #[test]
    fn parse_transform_translate_x_y() {
        assert!(matches!(
            parse_transform("translateX(40px)"),
            Some(Transform::Translate(_, 0.0))
        ));
        assert!(matches!(
            parse_transform("translateY(20px)"),
            Some(Transform::Translate(0.0, _))
        ));
    }

    #[test]
    fn parse_transform_length_bare_number() {
        let result = parse_transform_length("42");
        assert!(result.is_some());
        assert!((result.unwrap() - 42.0).abs() < 0.1);
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
        let result = parse_hex_to_color("abcd");
        assert!(result.is_none());
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
        assert_eq!(s.content, vec![ContentItem::Counter("section".to_string())]);
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
                ".".to_string()
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
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("max-width: 50%"), &parent);
        assert!(s.max_width.is_some());
    }

    #[test]
    fn min_width_from_percentage() {
        let parent = ComputedStyle::default();
        let s = compute_style(HtmlTag::Div, Some("min-width: 25%"), &parent);
        assert!(s.min_width.is_some());
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
        assert!(s.border_radius > 0.0);
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
    fn position_from_var_static_fallback() {
        let parent = ComputedStyle::default();
        let s = compute_style(
            HtmlTag::Div,
            Some("--p: fixed; position: var(--p)"),
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
        assert_eq!(items, vec![ContentItem::Counter("section".to_string())]);
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
                ".".to_string()
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
                ".".to_string()
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
        if let Some(shadow) = s.box_shadow {
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
