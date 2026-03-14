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

/// CSS box-shadow value.
#[derive(Debug, Clone, Copy)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub color: Color,
}

/// Fully resolved style for a node.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_family: FontFamily,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub text_align: TextAlign,
    pub text_decoration_underline: bool,
    pub text_decoration_line_through: bool,
    pub line_height: f32,
    pub page_break_before: bool,
    pub page_break_after: bool,
    pub border_width: f32,
    pub border_color: Option<Color>,
    pub display: Display,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub max_width: Option<f32>,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub opacity: f32,
    pub float: Float,
    pub clear: Clear,
    pub position: Position,
    pub top: Option<f32>,
    pub left: Option<f32>,
    pub box_shadow: Option<BoxShadow>,
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub flex_wrap: FlexWrap,
    pub gap: f32,
    pub overflow: Overflow,
    pub visibility: Visibility,
    pub transform: Option<Transform>,
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_gap: f32,
    pub border_radius: f32,
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
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            font_family: FontFamily::Helvetica,
            color: Color::BLACK,
            background_color: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            text_align: TextAlign::Left,
            text_decoration_underline: false,
            text_decoration_line_through: false,
            line_height: 1.4,
            page_break_before: false,
            page_break_after: false,
            border_width: 0.0,
            border_color: None,
            display: Display::Block,
            width: None,
            height: None,
            max_width: None,
            min_width: None,
            min_height: None,
            max_height: None,
            margin_left_auto: false,
            margin_right_auto: false,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            top: None,
            left: None,
            box_shadow: None,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            flex_wrap: FlexWrap::NoWrap,
            gap: 0.0,
            overflow: Overflow::Visible,
            visibility: Visibility::Visible,
            transform: None,
            grid_template_columns: Vec::new(),
            grid_gap: 0.0,
            border_radius: 0.0,
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
        }
    }
}

/// Compute the style for a node given its tag, inline styles, and parent style.
pub fn compute_style(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
) -> ComputedStyle {
    compute_style_with_rules(tag, inline_style, parent, &[], "", &[], None)
}

/// Compute style with stylesheet rules, class list, and id.
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
        style.background_gradient = None;
        style.background_radial_gradient = None;
    }

    // Reset non-inherited sizing and opacity properties
    style.width = None;
    style.height = None;
    style.max_width = None;
    style.min_width = None;
    style.min_height = None;
    style.max_height = None;
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.opacity = 1.0;
    style.float = Float::None;
    style.clear = Clear::None;
    style.position = Position::Static;
    style.top = None;
    style.left = None;
    style.box_shadow = None;
    style.flex_direction = FlexDirection::Row;
    style.justify_content = JustifyContent::FlexStart;
    style.align_items = AlignItems::Stretch;
    style.flex_wrap = FlexWrap::NoWrap;
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

    // Apply tag defaults
    let defaults = default_style(tag);
    apply_style_map(&mut style, &defaults, parent);

    // Apply stylesheet rules (between defaults and inline)
    for rule in rules {
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
        "font-family" => style.font_family = default.font_family,
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
        "width" => style.width = default.width,
        "height" => style.height = default.height,
        "max-width" => style.max_width = default.max_width,
        "opacity" => style.opacity = default.opacity,
        "border-width" => style.border_width = default.border_width,
        "border-color" => style.border_color = default.border_color,
        "border" => {
            style.border_width = default.border_width;
            style.border_color = default.border_color;
        }
        "float" => style.float = default.float,
        "clear" => style.clear = default.clear,
        "position" => style.position = default.position,
        "top" => style.top = default.top,
        "left" => style.left = default.left,
        "overflow" => style.overflow = default.overflow,
        "transform" => style.transform = default.transform,
        "box-shadow" => style.box_shadow = default.box_shadow,
        "flex-direction" => style.flex_direction = default.flex_direction,
        "justify-content" => style.justify_content = default.justify_content,
        "align-items" => style.align_items = default.align_items,
        "flex-wrap" => style.flex_wrap = default.flex_wrap,
        "gap" => style.gap = default.gap,
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
        "font-family" => style.font_family = parent.font_family.clone(),
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
        "width" => style.width = parent.width,
        "height" => style.height = parent.height,
        "max-width" => style.max_width = parent.max_width,
        "opacity" => style.opacity = parent.opacity,
        "border-width" => style.border_width = parent.border_width,
        "border-color" => style.border_color = parent.border_color,
        "border" => {
            style.border_width = parent.border_width;
            style.border_color = parent.border_color;
        }
        "float" => style.float = parent.float,
        "clear" => style.clear = parent.clear,
        "position" => style.position = parent.position,
        "top" => style.top = parent.top,
        "left" => style.left = parent.left,
        "overflow" => style.overflow = parent.overflow,
        "transform" => style.transform = parent.transform,
        "box-shadow" => style.box_shadow = parent.box_shadow,
        "flex-direction" => style.flex_direction = parent.flex_direction,
        "justify-content" => style.justify_content = parent.justify_content,
        "align-items" => style.align_items = parent.align_items,
        "flex-wrap" => style.flex_wrap = parent.flex_wrap,
        "gap" => style.gap = parent.gap,
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

fn apply_style_map(style: &mut ComputedStyle, map: &StyleMap, parent: &ComputedStyle) {
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
        let lower = k.to_ascii_lowercase();
        // Strip quotes from font names like "'Times New Roman'" or "\"Courier New\""
        let cleaned = lower.trim_matches(|c| c == '\'' || c == '"');
        style.font_family = match cleaned {
            // Serif → TimesRoman
            "serif" | "times" | "times new roman" | "times-roman" | "georgia" | "garamond"
            | "book antiqua" | "palatino" | "palatino linotype" | "baskerville"
            | "hoefler text" | "cambria" | "droid serif" | "noto serif" | "libre baskerville"
            | "merriweather" | "playfair display" | "lora" => FontFamily::TimesRoman,

            // Monospace → Courier
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

            // Explicit sans-serif mappings
            "sans-serif" | "arial" | "helvetica" | "helvetica neue" | "arial black" | "verdana"
            | "tahoma" | "trebuchet ms" | "gill sans" | "lucida sans" | "lucida grande"
            | "system-ui" | "-apple-system" | "segoe ui" | "roboto" | "open sans" | "lato"
            | "inter" | "nunito" | "poppins" | "montserrat" | "raleway" | "ubuntu" => {
                FontFamily::Helvetica
            }

            // Unknown font name — treat as custom; renderer will fall back to
            // Helvetica if no matching TTF is registered.
            other => FontFamily::Custom(other.to_string()),
        };
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
            style.background_gradient = Some(lg);
        }
    }

    // Radial gradient (from background or background-image)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "background-radial-gradient") {
        if let Some(rg) = parse_radial_gradient(k) {
            style.background_radial_gradient = Some(rg);
        }
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "margin-top") {
        style.margin.top = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "margin-right") {
        style.margin.right = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "margin-bottom") {
        style.margin.bottom = *v;
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "margin-left") {
        style.margin.left = *v;
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

    // Border shorthand: "1px solid black"
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "border") {
        let parts: Vec<&str> = k.split_whitespace().collect();
        // Extract width from first token
        for part in &parts {
            if let Some(n) = part.strip_suffix("px") {
                if let Ok(v) = n.parse::<f32>() {
                    style.border_width = v * 0.75; // px to pt
                }
            } else if let Some(n) = part.strip_suffix("pt") {
                if let Ok(v) = n.parse::<f32>() {
                    style.border_width = v;
                }
            }
        }
        // Extract color from last token
        if let Some(last) = parts.last() {
            if let Some(c) = parse_border_color(last) {
                style.border_color = Some(c);
            }
        }
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "width") {
        style.width = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "width") {
        // em value — multiply by current font-size
        style.width = Some(*v * style.font_size);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "height") {
        style.height = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "height") {
        style.height = Some(*v * style.font_size);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "max-width") {
        style.max_width = Some(*v * style.font_size);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "min-width") {
        style.min_width = Some(*v * style.font_size);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "min-height") {
        style.min_height = Some(*v * style.font_size);
    }

    if let Some(CssValue::Length(v)) = get_non_special(map, "max-height") {
        style.max_height = Some(*v);
    }
    if let Some(CssValue::Number(v)) = get_non_special(map, "max-height") {
        style.max_height = Some(*v * style.font_size);
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
        style.border_width = *v;
    }

    if let Some(CssValue::Color(c)) = get_non_special(map, "border-color") {
        style.border_color = Some(*c);
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

    // Top / Left for positioned elements
    if let Some(CssValue::Length(v)) = get_non_special(map, "top") {
        style.top = Some(*v);
    }
    if let Some(CssValue::Length(v)) = get_non_special(map, "left") {
        style.left = Some(*v);
    }

    // Box-shadow: parse from keyword (stored as full shorthand string)
    if let Some(CssValue::Keyword(k)) = get_non_special(map, "box-shadow") {
        if let Some(shadow) = parse_box_shadow(k) {
            style.box_shadow = Some(shadow);
        }
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
    if let Some(CssValue::Length(v)) = get_non_special(map, "border-radius") {
        style.border_radius = *v;
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

/// Parse a CSS `transform` value.
///
/// Supports:
/// - `rotate(45deg)`
/// - `scale(2)` or `scale(1.5, 2.0)`
/// - `translate(10pt, 20pt)` or `translate(10px, 20px)`
/// - `none`
fn parse_transform(val: &str) -> Option<Transform> {
    let val = val.trim();
    if val == "none" {
        return None;
    }

    if let Some(inner) = val
        .strip_prefix("rotate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let inner = inner.trim();
        let degrees = if let Some(n) = inner.strip_suffix("deg") {
            n.trim().parse::<f32>().ok()?
        } else {
            // bare number treated as degrees
            inner.parse::<f32>().ok()?
        };
        return Some(Transform::Rotate(degrees));
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

    None
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

/// Parse a `grid-template-columns` value string into a list of `GridTrack` values.
///
/// Supports tokens like `1fr`, `200pt`, `100px`, and `auto`.
fn parse_grid_template_columns(val: &str) -> Vec<GridTrack> {
    val.split_whitespace()
        .filter_map(|token| {
            if let Some(n) = token.strip_suffix("fr") {
                n.parse::<f32>().ok().map(GridTrack::Fr)
            } else if token == "auto" {
                Some(GridTrack::Auto)
            } else if let Some(n) = token.strip_suffix("pt") {
                n.parse::<f32>().ok().map(GridTrack::Fixed)
            } else if let Some(n) = token.strip_suffix("px") {
                n.parse::<f32>().ok().map(|v| GridTrack::Fixed(v * 0.75))
            } else {
                // Try bare number as pt
                token.parse::<f32>().ok().map(GridTrack::Fixed)
            }
        })
        .collect()
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
        assert!((style.border_width - 0.75).abs() < 0.1); // 1px = 0.75pt
        assert!(style.border_color.is_some());
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_with_custom_color() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2px solid red"), &parent);
        assert!((style.border_width - 1.5).abs() < 0.1); // 2px = 1.5pt
        let c = style.border_color.unwrap();
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
        assert!((style.border_width - 3.0).abs() < 0.1);
        let c = style.border_color.unwrap();
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
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn border_shorthand_pt_unit() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2pt solid green"), &parent);
        assert!((style.border_width - 2.0).abs() < 0.1);
        let c = style.border_color.unwrap();
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
            let c = style.border_color.unwrap();
            assert_eq!((c.r, c.g, c.b), (r, g, b), "failed for {name}");
        }
    }

    #[test]
    fn border_color_hex_short() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #f00"), &parent);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_hex_long() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #00ff00"), &parent);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_unknown_returns_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid foobar"), &parent);
        assert!(style.border_color.is_none());
    }

    // --- Extended font-family mapping tests ---

    #[test]
    fn font_family_arial_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Arial"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_roboto_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Roboto"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_verdana_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Verdana"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_open_sans_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Open Sans'"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_system_ui_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: system-ui"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
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
        assert!((style.border_width - 0.0).abs() < 0.1);
    }

    #[test]
    fn border_color_initial_resets() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border-color: initial"), &parent);
        assert!(style.border_color.is_none());
    }

    #[test]
    fn border_initial_resets_both() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: initial"), &parent);
        assert!((style.border_width - 0.0).abs() < 0.1);
        assert!(style.border_color.is_none());
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
        parent.border_width = 3.0;
        let style = compute_style(HtmlTag::Div, Some("border-width: inherit"), &parent);
        assert!((style.border_width - 3.0).abs() < 0.1);
    }

    #[test]
    fn border_color_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border_color = Some(Color::rgb(255, 0, 0));
        let style = compute_style(HtmlTag::Div, Some("border-color: inherit"), &parent);
        assert_eq!(style.border_color.unwrap().r, 255);
    }

    #[test]
    fn border_inherit_from_parent() {
        let mut parent = ComputedStyle::default();
        parent.border_width = 2.0;
        parent.border_color = Some(Color::rgb(0, 0, 255));
        let style = compute_style(HtmlTag::Div, Some("border: inherit"), &parent);
        assert!((style.border_width - 2.0).abs() < 0.1);
        assert_eq!(style.border_color.unwrap().b, 255);
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
        let t = parse_transform("skew(30deg)");
        assert!(t.is_none());
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
}
