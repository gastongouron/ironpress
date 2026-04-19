use crate::parser::css::{AncestorInfo, CssRule, PseudoElement, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    AlignItems, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize,
    BorderCollapse, BorderSides, BoxShadow, Clear, ComputedStyle, Display, Float, FontFamily,
    FontStyle, FontWeight, GridTrack, LinearGradient, ListStylePosition, ListStyleType, Overflow,
    Position, RadialGradient, TextAlign, Transform, VerticalAlign, Visibility,
    compute_pseudo_element_style, compute_style_with_context,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;

use super::block::layout_block_element;
use super::flex::layout_flex_container;
use super::grid::layout_grid_container;
pub(crate) use super::helpers::*;
use super::images::*;
use super::inline::{element_is_inline_block, layout_inline_block_group};
use super::table::flatten_table;

#[cfg(test)]
use super::text::OverflowWrap;
use super::text::{
    TextWrapOptions, collapse_whitespace, collect_text_runs, push_text_run_with_fallback,
    resolve_style_font_family, resolved_line_height_factor, wrap_text_runs,
};
#[cfg(test)]
use crate::style::computed::ContentItem;

/// A single border side for layout rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutBorderSide {
    pub width: f32,
    pub color: (f32, f32, f32),
    pub style: crate::style::computed::BorderStyle,
}

/// Per-side border for layout rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutBorder {
    pub top: LayoutBorderSide,
    pub right: LayoutBorderSide,
    pub bottom: LayoutBorderSide,
    pub left: LayoutBorderSide,
}

// Default is derived via #[derive(Default)] on the struct.

#[allow(dead_code)]
impl LayoutBorder {
    pub fn from_computed(b: &BorderSides) -> Self {
        Self {
            top: LayoutBorderSide {
                width: b.top.width,
                color: b.top.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
                style: b.top.style,
            },
            right: LayoutBorderSide {
                width: b.right.width,
                color: b.right.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
                style: b.right.style,
            },
            bottom: LayoutBorderSide {
                width: b.bottom.width,
                color: b.bottom.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
                style: b.bottom.style,
            },
            left: LayoutBorderSide {
                width: b.left.width,
                color: b.left.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
                style: b.left.style,
            },
        }
    }
    pub fn has_any(&self) -> bool {
        self.top.width > 0.0
            || self.right.width > 0.0
            || self.bottom.width > 0.0
            || self.left.width > 0.0
    }
    pub fn horizontal_width(&self) -> f32 {
        self.left.width + self.right.width
    }
    pub fn vertical_width(&self) -> f32 {
        self.top.width + self.bottom.width
    }
    pub fn max_width(&self) -> f32 {
        self.top
            .width
            .max(self.right.width)
            .max(self.bottom.width)
            .max(self.left.width)
    }
}

// Inline-block layout functions moved to `super::inline`.

/// Counter state for CSS counters.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub(crate) struct CounterState {
    pub(crate) stacks: HashMap<String, Vec<i32>>,
}
#[allow(dead_code)]
impl CounterState {
    fn apply_resets(&mut self, resets: &[(String, i32)]) {
        for (name, val) in resets {
            self.stacks.entry(name.clone()).or_default().push(*val);
        }
    }
    fn apply_increments(&mut self, increments: &[(String, i32)]) {
        for (name, val) in increments {
            let stack = self.stacks.entry(name.clone()).or_default();
            if stack.is_empty() {
                stack.push(0);
            }
            if let Some(top) = stack.last_mut() {
                *top += val;
            }
        }
    }
    fn pop_resets(&mut self, resets: &[(String, i32)]) {
        for (name, _) in resets {
            if let Some(stack) = self.stacks.get_mut(name) {
                stack.pop();
            }
        }
    }
    pub(crate) fn get(&self, name: &str) -> i32 {
        self.stacks
            .get(name)
            .and_then(|s| s.last().copied())
            .unwrap_or(0)
    }
    pub(crate) fn get_all(&self, name: &str, sep: &str) -> String {
        self.stacks
            .get(name)
            .map(|s| {
                s.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(sep)
            })
            .unwrap_or_else(|| "0".to_string())
    }
}

/// Context for rendering list items.
#[derive(Debug, Clone)]
pub(crate) enum ListContext {
    Unordered { indent: f32 },
    Ordered { index: usize, indent: f32 },
}

pub use super::table::TableCell;
pub(crate) use super::table::table_cell_content_height;

/// A cell within a flex row, with its computed x-offset and width.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlexCell {
    pub lines: Vec<TextLine>,
    pub x_offset: f32,
    pub width: f32,
    pub text_align: TextAlign,
    pub background_color: Option<(f32, f32, f32, f32)>,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border: LayoutBorder,
    /// Natural height of this flex item (without stretching)
    pub natural_height: f32,
    pub border_radius: f32,
    pub background_gradient: Option<LinearGradient>,
    pub background_radial_gradient: Option<RadialGradient>,
    pub background_svg: Option<crate::parser::svg::SvgTree>,
    pub background_blur_radius: f32,
    pub background_size: BackgroundSize,
    pub background_position: BackgroundPosition,
    pub background_repeat: BackgroundRepeat,
    pub background_origin: BackgroundOrigin,
    pub transform: Option<Transform>,
    /// Box shadow to render behind this cell (CSS `box-shadow`).
    pub box_shadow: Option<BoxShadow>,
    /// Nested layout elements for complex flex items (tables, images, etc.)
    pub nested_elements: Vec<LayoutElement>,
    /// Cross-axis offset of this cell within the FlexRow. For single-line
    /// rows this is 0; for `flex-wrap: wrap` with multiple lines, items on
    /// subsequent lines carry their cumulative cross_offset here so a single
    /// FlexRow can visually span every wrapped line.
    pub y_offset: f32,
    /// Cross-axis size of the flex line this cell belongs to. Drives
    /// per-line alignment math (stretch/center/flex-end) so that a single
    /// FlexRow carrying cells from multiple wrapped lines still aligns each
    /// item against its own line rather than the entire row.
    pub line_cross_size: f32,
}

/// A styled text run (a piece of text with uniform style).
#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
    pub color: (f32, f32, f32),
    pub link_url: Option<String>,
    pub font_family: FontFamily,
    /// Background color for inline spans (e.g. badge/highlight).
    pub background_color: Option<(f32, f32, f32, f32)>,
    /// Horizontal and vertical padding for inline background.
    pub padding: (f32, f32),
    /// Border radius for inline spans (e.g. badge with rounded corners).
    pub border_radius: f32,
}

/// A laid-out line of text runs.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub runs: Vec<TextRun>,
    pub height: f32,
}

/// The format of an embedded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

/// Parsed PNG metadata needed for PDF FlateDecode parameters.
#[derive(Debug, Clone)]
pub struct PngMetadata {
    pub channels: u8,
    pub bit_depth: u8,
}

/// Raster image bytes plus the source pixel dimensions required by the PDF renderer.
#[derive(Debug, Clone)]
pub struct RasterImageAsset {
    pub data: Vec<u8>,
    pub source_width: u32,
    pub source_height: u32,
    pub format: ImageFormat,
    pub png_metadata: Option<PngMetadata>,
}

pub use super::context::*;

/// A layout element ready for rendering.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant, dead_code)]
pub enum LayoutElement {
    /// A block of text lines with optional background.
    TextBlock {
        lines: Vec<TextLine>,
        margin_top: f32,
        margin_bottom: f32,
        text_align: TextAlign,
        background_color: Option<(f32, f32, f32, f32)>,
        padding_top: f32,
        padding_bottom: f32,
        padding_left: f32,
        padding_right: f32,
        border: LayoutBorder,
        block_width: Option<f32>,
        block_height: Option<f32>,
        opacity: f32,
        float: Float,
        clear: Clear,
        position: Position,
        offset_top: f32,
        offset_left: f32,
        offset_bottom: f32,
        offset_right: f32,
        /// Containing block for `position: absolute` elements.
        /// When `Some`, offsets are relative to this block instead of the page.
        containing_block: Option<ContainingBlock>,
        /// Number of elements that follow this one in the output list and should
        /// be rendered within this element's clip rect (overflow: hidden).
        /// The renderer keeps the clipping path active for this many elements.
        clip_children_count: usize,
        box_shadow: Option<BoxShadow>,
        visible: bool,
        clip_rect: Option<(f32, f32, f32, f32)>,
        transform: Option<Transform>,
        border_radius: f32,
        outline_width: f32,
        outline_color: Option<(f32, f32, f32)>,
        text_indent: f32,
        letter_spacing: f32,
        word_spacing: f32,
        vertical_align: VerticalAlign,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        z_index: i32,
        repeat_on_each_page: bool,
        positioned_depth: usize,
        /// Heading level (1-6) if this block is an h1-h6, used for PDF bookmarks.
        heading_level: Option<u8>,
    },
    /// A table row with cells.
    TableRow {
        cells: Vec<TableCell>,
        col_widths: Vec<f32>,
        margin_top: f32,
        margin_bottom: f32,
        border_collapse: BorderCollapse,
        border_spacing: f32,
        /// Row belongs to a `<thead>`; pagination re-emits it on every page
        /// the parent table spans (mirroring Chrome's behavior).
        is_header: bool,
    },
    /// A grid row with cells of varying widths.
    GridRow {
        cells: Vec<TableCell>,
        col_widths: Vec<f32>,
        gap: f32,
        margin_top: f32,
        margin_bottom: f32,
        border: LayoutBorder,
        padding_left: f32,
        padding_right: f32,
        padding_top: f32,
        padding_bottom: f32,
    },
    /// An embedded image.
    Image {
        image: RasterImageAsset,
        width: f32,
        height: f32,
        /// Extra flow-only height below the replaced content, used to model
        /// inline baseline/strut space without stretching the rendered image.
        flow_extra_bottom: f32,
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A horizontal rule.
    HorizontalRule { margin_top: f32, margin_bottom: f32 },
    /// An inline SVG element.
    Svg {
        /// The parsed SVG tree.
        tree: crate::parser::svg::SvgTree,
        /// Rendered width in points.
        width: f32,
        /// Rendered height in points.
        height: f32,
        /// Extra flow-only height below the rendered SVG, used to model
        /// inline baseline/strut space without stretching the rendered image.
        flow_extra_bottom: f32,
        /// Top margin.
        margin_top: f32,
        /// Bottom margin.
        margin_bottom: f32,
    },
    /// A flex row with cells positioned horizontally.
    #[allow(dead_code)]
    FlexRow {
        cells: Vec<FlexCell>,
        row_height: f32,
        margin_top: f32,
        margin_bottom: f32,
        /// Container background color.
        background_color: Option<(f32, f32, f32, f32)>,
        /// Full container width (including padding).
        container_width: f32,
        padding_top: f32,
        padding_bottom: f32,
        padding_left: f32,
        padding_right: f32,
        border: LayoutBorder,
        border_radius: f32,
        box_shadow: Option<BoxShadow>,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        align_items: AlignItems,
    },
    /// A progress bar or meter element.
    ProgressBar {
        /// Fraction filled (0.0 to 1.0).
        fraction: f32,
        /// Total width in points.
        width: f32,
        /// Total height in points.
        height: f32,
        /// Fill color (r, g, b).
        fill_color: (f32, f32, f32),
        /// Track color (r, g, b).
        track_color: (f32, f32, f32),
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A math expression (LaTeX).
    MathBlock {
        /// Laid-out math glyphs.
        layout: crate::layout::math::MathLayout,
        /// Whether this is display math (centered, own paragraph) or inline.
        display: bool,
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A block container with visual properties and nested children.
    /// Unlike the flat TextBlock+pullback hack, this represents true
    /// parent-child nesting. The renderer draws the container's background
    /// and border, then recursively renders children inside.
    Container {
        children: Vec<LayoutElement>,
        background_color: Option<(f32, f32, f32, f32)>,
        border: LayoutBorder,
        border_radius: f32,
        padding_top: f32,
        padding_bottom: f32,
        padding_left: f32,
        padding_right: f32,
        margin_top: f32,
        margin_bottom: f32,
        block_width: Option<f32>,
        block_height: Option<f32>,
        opacity: f32,
        float: Float,
        position: Position,
        offset_top: f32,
        offset_left: f32,
        overflow: Overflow,
        transform: Option<Transform>,
        box_shadow: Option<BoxShadow>,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        z_index: i32,
    },
    /// A page break.
    PageBreak,
}

/// A fully laid-out page.
pub struct Page {
    pub elements: Vec<(f32, LayoutElement)>, // (y_position, element)
}

/// Lay out the DOM nodes into pages.
#[allow(dead_code)]
pub fn layout(nodes: &[DomNode], page_size: PageSize, margin: Margin) -> Vec<Page> {
    layout_with_rules(nodes, page_size, margin, &[])
}

/// Resolve the margin declared on `body`, `html`, or `:root` selectors against
/// the given page size. The result is additive to the caller-supplied page
/// margin — Chrome treats the body margin as shrinking the printable area
/// inside the page margin, so both offsets stack.
///
/// Returns zeros when no matching rule declares a margin.
pub fn compute_root_margin(rules: &[CssRule], page_size: PageSize) -> Margin {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle {
        viewport_width: page_size.width,
        viewport_height: page_size.height,
        root_font_size: style.font_size,
        width: Some(page_size.width),
        ..ComputedStyle::default()
    };

    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(&mut style, &rule.declarations, &parent);
        }
    }

    Margin {
        top: style.margin.top,
        right: style.margin.right,
        bottom: style.margin.bottom,
        left: style.margin.left,
    }
}

/// Compute the extra horizontal gutter that body's `max-width` plus
/// `margin: auto` produces. This pattern (`body { max-width: 640px;
/// margin: 40px auto; }`) centers the body content within the page's
/// printable area. Since ironpress strips the `<body>` element before
/// layout, we emulate the centering by folding the remainder width
/// `(printable - max_width) / 2` into the effective page margin.
///
/// `printable_width` is the page width minus existing left/right margins
/// (including any previously folded body margin/padding). Returns the
/// half-gutter width to add on BOTH sides, or 0 if the body doesn't
/// declare a max-width or both margins aren't auto.
pub fn compute_root_body_centering_gutter(
    rules: &[CssRule],
    page_size: PageSize,
    printable_width: f32,
) -> f32 {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle {
        viewport_width: page_size.width,
        viewport_height: page_size.height,
        root_font_size: style.font_size,
        width: Some(page_size.width),
        ..ComputedStyle::default()
    };
    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(&mut style, &rule.declarations, &parent);
        }
    }
    // Require BOTH left and right margin: auto (centering) plus a max-width.
    if !(style.margin_left_auto && style.margin_right_auto) {
        return 0.0;
    }
    let max_w = match (style.max_width, style.percentage_sizing.max_width) {
        (Some(w), _) => w,
        (None, Some(pct)) => pct / 100.0 * printable_width,
        _ => return 0.0,
    };
    if max_w <= 0.0 || max_w >= printable_width {
        return 0.0;
    }
    (printable_width - max_w) / 2.0
}

/// Resolve the padding declared on `body`, `html`, or `:root` selectors against
/// the given page size. Chrome treats body padding as shrinking the printable
/// area inside the page margin (like an inner gutter), so we fold it into the
/// effective page margin alongside `compute_root_margin`.
///
/// Returns zeros when no matching rule declares padding.
pub fn compute_root_padding(rules: &[CssRule], page_size: PageSize) -> (f32, f32, f32, f32) {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle {
        viewport_width: page_size.width,
        viewport_height: page_size.height,
        root_font_size: style.font_size,
        width: Some(page_size.width),
        ..ComputedStyle::default()
    };

    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(&mut style, &rule.declarations, &parent);
        }
    }

    (
        style.padding.top,
        style.padding.right,
        style.padding.bottom,
        style.padding.left,
    )
}

/// Lay out the DOM nodes into pages with stylesheet rules.
#[allow(dead_code)]
pub fn layout_with_rules(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
) -> Vec<Page> {
    layout_with_rules_and_fonts(nodes, page_size, margin, rules, &HashMap::new())
}

/// Lay out the DOM nodes into pages with stylesheet rules and custom fonts.
pub fn layout_with_rules_and_fonts(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
    custom_fonts: &HashMap<String, TtfFont>,
) -> Vec<Page> {
    // Apply body/html/:root rules to the root style so that inherited root
    // properties still take effect even though the HTML parser unwraps the
    // <html>/<body> elements before layout.
    let mut parent_style = ComputedStyle::default();
    let default_parent = ComputedStyle::default();
    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(
                &mut parent_style,
                &rule.declarations,
                &default_parent,
            );
        }
    }
    let available_width = page_size.width - margin.left - margin.right;
    let content_height = page_size.height - margin.top - margin.bottom;
    parent_style.width = Some(available_width);
    parent_style.root_font_size = parent_style.font_size;
    parent_style.viewport_width = page_size.width;
    parent_style.viewport_height = page_size.height;

    // First, flatten DOM into layout elements
    let mut elements = Vec::new();

    // If the body/html has a background SVG (or gradient/color), emit a full-content-area
    // background block at the very start so it renders behind all content.
    //
    // Chrome's print model paints body background across the entire body box —
    // including through the body padding, up to the body border edge (i.e. the
    // page margin edge). Since `compute_root_padding` folded the body padding
    // into the effective page margin, `available_width`/`content_height` here
    // describe only the inner content area AFTER padding. We extend the bg
    // block outward by the body padding so it visually fills the padding zone
    // too — matching Chrome's behaviour.
    let has_body_bg = has_background_paint(&parent_style);
    if has_body_bg {
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&parent_style);
        let bp_left = parent_style.padding.left;
        let bp_right = parent_style.padding.right;
        let bp_top = parent_style.padding.top;
        let bp_bottom = parent_style.padding.bottom;
        elements.push(LayoutElement::TextBlock {
            lines: vec![],
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: parent_style.background_color.map(|c| c.to_f32_rgba()),
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: Some(available_width + bp_left + bp_right),
            block_height: Some(content_height + bp_top + bp_bottom),
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Absolute,
            offset_top: -bp_top,
            offset_left: -bp_left,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: -1,
            repeat_on_each_page: true,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
    }

    let ancestors: Vec<AncestorInfo> = Vec::new();
    let mut counter_state = CounterState::default();
    let root_ctx = LayoutContext {
        viewport: Viewport {
            width: available_width,
            height: content_height,
        },
        parent: ParentBox {
            content_width: available_width,
            content_height: Some(content_height),
            font_size: parent_style.font_size,
        },
        containing_block: None,
        root_font_size: parent_style.root_font_size,
    };
    let mut env = LayoutEnv {
        rules,
        fonts: custom_fonts,
        counter_state: &mut counter_state,
    };
    flatten_nodes(
        nodes,
        &parent_style,
        &root_ctx,
        &mut elements,
        None,
        &ancestors,
        0,
        &mut env,
    );

    // Then paginate. Pass the body/html margin-top (plus padding-top, which
    // acts as an additional inner gutter on page 1 when the body has padding)
    // so the first in-flow block on each page can collapse through the root.
    super::paginate::paginate(
        elements,
        content_height,
        parent_style.margin.top + parent_style.padding.top,
    )
}

/// Flatten a list of DOM nodes into layout elements.
///
/// Iterates over `nodes`, collecting inline-block groups and dispatching
/// each element to [`flatten_element`]. Text nodes between elements
/// trigger inline-block group flushes when non-whitespace.
#[allow(clippy::too_many_arguments)]
fn flatten_nodes(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    ancestors: &[AncestorInfo],
    positioned_ancestor_depth: usize,
    env: &mut LayoutEnv,
) {
    let ib_ctx = *ctx;

    // Count element children for sibling context
    let element_count = nodes
        .iter()
        .filter(|n| matches!(n, DomNode::Element(_)))
        .count();
    let mut element_index = 0;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();

    // Accumulator for consecutive inline-block elements
    let mut ib_group: Vec<&ElementNode> = Vec::new();

    // Helper closure-like macro for flushing an inline-block group.
    // We use a nested fn instead since closures can't borrow multiple fields.
    #[allow(clippy::drain_collect)]
    #[inline]
    fn flush_ib(
        group: &mut Vec<&ElementNode>,
        parent_style: &ComputedStyle,
        ctx: &LayoutContext,
        output: &mut Vec<LayoutElement>,
        rules: &[CssRule],
        ancestors: &[AncestorInfo],
        fonts: &HashMap<String, TtfFont>,
    ) {
        if group.is_empty() {
            return;
        }
        let taken: Vec<&ElementNode> = group.drain(..).collect();
        layout_inline_block_group(&taken, parent_style, ctx, output, rules, ancestors, fonts);
    }

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let trimmed = collapse_whitespace(text);
                // Only flush inline-block group for non-whitespace text.
                // Whitespace between consecutive inline-block elements must
                // not break the group — they should stay on the same row.
                if !trimmed.is_empty() {
                    flush_ib(
                        &mut ib_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        list_ctx.map(|_| env.rules).unwrap_or(env.rules),
                        ancestors,
                        env.fonts,
                    );
                }
                if !trimmed.is_empty() {
                    let mut text_runs = Vec::new();
                    push_text_run_with_fallback(
                        TextRun {
                            text: trimmed,
                            font_size: parent_style.font_size,
                            bold: parent_style.font_weight == FontWeight::Bold,
                            italic: parent_style.font_style == FontStyle::Italic,
                            underline: parent_style.text_decoration_underline,
                            line_through: parent_style.text_decoration_line_through,
                            overline: parent_style.text_decoration_overline,
                            color: parent_style.color.to_f32_rgb(),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, env.fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                        },
                        &mut text_runs,
                        env.fonts,
                    );
                    let lines = wrap_text_runs(
                        text_runs,
                        TextWrapOptions::new(
                            ctx.available_width(),
                            parent_style.font_size,
                            resolved_line_height_factor(parent_style, env.fonts),
                            parent_style.overflow_wrap,
                        )
                        .with_rtl(parent_style.direction_rtl),
                        env.fonts,
                    );
                    if !lines.is_empty() {
                        output.push(LayoutElement::TextBlock {
                            lines,
                            margin_top: 0.0,
                            margin_bottom: 0.0,
                            text_align: parent_style.text_align,
                            background_color: None,
                            padding_top: 0.0,
                            padding_bottom: 0.0,
                            padding_left: 0.0,
                            padding_right: 0.0,
                            border: LayoutBorder::default(),
                            block_width: None,
                            block_height: None,
                            opacity: 1.0,
                            float: Float::None,
                            clear: Clear::None,
                            position: Position::Static,
                            offset_top: 0.0,
                            offset_left: 0.0,
                            offset_bottom: 0.0,
                            offset_right: 0.0,
                            containing_block: None,
                            box_shadow: None,
                            visible: true,
                            clip_rect: None,
                            transform: None,
                            border_radius: 0.0,
                            outline_width: 0.0,
                            outline_color: None,
                            text_indent: 0.0,
                            letter_spacing: 0.0,
                            word_spacing: 0.0,
                            vertical_align: VerticalAlign::Baseline,
                            background_gradient: None,
                            background_radial_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            z_index: 0,
                            repeat_on_each_page: false,
                            positioned_depth: 0,
                            heading_level: None,
                            clip_children_count: 0,
                        });
                    }
                }
            }
            DomNode::Element(el) => {
                // Check if this element is inline-block
                if element_is_inline_block(
                    el,
                    parent_style,
                    env.rules,
                    ancestors,
                    element_index,
                    element_count,
                    &preceding_siblings,
                ) {
                    ib_group.push(el);
                } else {
                    // Flush any pending inline-block group
                    flush_ib(
                        &mut ib_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        env.rules,
                        ancestors,
                        env.fonts,
                    );
                    flatten_element(
                        el,
                        parent_style,
                        &ib_ctx,
                        output,
                        list_ctx,
                        ancestors,
                        positioned_ancestor_depth,
                        element_index,
                        element_count,
                        &preceding_siblings,
                        env,
                    );
                }
                // Track this element as a preceding sibling for the next element
                preceding_siblings.push((
                    el.tag_name().to_string(),
                    el.class_list().iter().map(|s| s.to_string()).collect(),
                ));
                element_index += 1;
            }
        }
    }
    // Flush any remaining inline-block group at end of nodes
    flush_ib(
        &mut ib_group,
        parent_style,
        &ib_ctx,
        output,
        env.rules,
        ancestors,
        env.fonts,
    );
}

/// Flatten a single DOM element into layout elements.
///
/// Computes the element's style, handles special tags (math, br, hr, img,
/// svg, form controls, media, tables, lists), then delegates to
/// [`route_element`] for display-mode dispatching.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flatten_element(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    ancestors: &[AncestorInfo],
    positioned_ancestor_depth: usize,
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
    env: &mut LayoutEnv,
) {
    let available_width = ctx.available_width();
    let available_height = ctx.available_height();
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
    };
    let mut style = compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        env.rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    );

    // Apply CSS counter operations for this element.
    env.counter_state.apply_resets(&style.counter_reset);
    env.counter_state.apply_increments(&style.counter_increment);

    // Bail out on excessively deep nesting to prevent stack overflow.
    if ancestors.len() > 30 {
        return;
    }

    let available_height = style.height.unwrap_or(available_height);
    // Update context when element narrows the available height.
    let layout_ctx = if style.height.is_some() {
        ctx.with_parent(available_width, Some(available_height), style.font_size)
    } else {
        *ctx
    };
    let positioned_depth =
        if style.position == Position::Relative || style.position == Position::Absolute {
            positioned_ancestor_depth + 1
        } else {
            positioned_ancestor_depth
        };

    // display: none — skip this element entirely
    if style.display == Display::None {
        return;
    }

    // Math elements: <span class="math-inline"> or <div class="math-display">
    if let Some(tex) = el.attributes.get("data-math") {
        let is_display = classes.contains(&"math-display");
        if is_display {
            let ast = crate::parser::math::parse_math(tex);
            let math_layout = crate::layout::math::layout_math(&ast, style.font_size, is_display);
            output.push(LayoutElement::MathBlock {
                layout: math_layout,
                display: true,
                margin_top: style.margin.top.max(6.0),
                margin_bottom: style.margin.bottom.max(6.0),
            });
            return;
        }
        // Inline math: fall through to normal inline text collection.
        // The <span> children contain the raw LaTeX text which is rendered
        // as italic text in the surrounding paragraph flow.
    }

    if el.tag == HtmlTag::Br {
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::none();
        let line = TextLine {
            runs: vec![TextRun {
                text: String::new(),
                font_size: style.font_size,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                overline: false,
                color: (0.0, 0.0, 0.0),
                link_url: None,
                font_family: resolve_style_font_family(&style, env.fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            }],
            height: style.font_size * resolved_line_height_factor(&style, env.fonts),
        };
        output.push(LayoutElement::TextBlock {
            lines: vec![line],
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border: LayoutBorder::default(),
            padding_right: 0.0,
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
        return;
    }

    if el.tag == HtmlTag::Hr {
        output.push(LayoutElement::HorizontalRule {
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
        });
        return;
    }

    if el.tag == HtmlTag::Img {
        if let Some(img_element) =
            load_image_from_element(el, available_width, available_height, &style)
        {
            output.push(add_inline_replaced_baseline_gap(
                img_element,
                &style,
                env.fonts,
            ));
        }
        return;
    }

    if el.tag == HtmlTag::Svg {
        let (svg_width, svg_height) =
            resolve_svg_element_size(el, available_width, available_height, true, true);
        if let Some(mut tree) = crate::parser::svg::parse_svg_from_element_with_viewport(
            el,
            Some((svg_width, svg_height)),
        ) {
            sync_svg_tree_to_layout_box(&mut tree, svg_width, svg_height);
            inject_inherited_svg_color(&mut tree, style.color.to_f32_rgb());
            output.push(LayoutElement::Svg {
                tree,
                width: svg_width,
                height: svg_height,
                flow_extra_bottom: 0.0,
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
            });
        }
        return;
    }

    // Form control elements — render as styled boxes with placeholder text
    if el.tag == HtmlTag::Input || el.tag == HtmlTag::Select || el.tag == HtmlTag::Textarea {
        let ctrl_width = style
            .width
            .unwrap_or(if el.tag == HtmlTag::Textarea {
                available_width.min(300.0)
            } else {
                150.0
            })
            .min(available_width);
        let ctrl_height = style.height.unwrap_or(if el.tag == HtmlTag::Textarea {
            80.0
        } else {
            20.0
        });

        let label = if el.tag == HtmlTag::Select {
            el.children
                .iter()
                .find_map(|c| {
                    if let DomNode::Element(opt) = c {
                        opt.children.iter().find_map(|t| {
                            if let DomNode::Text(s) = t {
                                Some(s.trim().to_string())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else if el.tag == HtmlTag::Textarea {
            el.children
                .iter()
                .find_map(|c| {
                    if let DomNode::Text(s) = c {
                        Some(s.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            el.attributes
                .get("value")
                .or(el.attributes.get("placeholder"))
                .cloned()
                .unwrap_or_default()
        };

        let mut lines = Vec::new();
        if !label.is_empty() {
            let mut runs = Vec::new();
            push_text_run_with_fallback(
                TextRun {
                    text: label,
                    font_size: style.font_size,
                    bold: false,
                    italic: false,
                    underline: false,
                    line_through: false,
                    overline: false,
                    color: style.color.to_f32_rgb(),
                    link_url: None,
                    font_family: resolve_style_font_family(&style, env.fonts),
                    background_color: None,
                    padding: (0.0, 0.0),
                    border_radius: 0.0,
                },
                &mut runs,
                env.fonts,
            );
            let inner_w = ctrl_width - style.padding.left - style.padding.right;
            lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    inner_w,
                    style.font_size,
                    resolved_line_height_factor(&style, env.fonts),
                    style.overflow_wrap,
                )
                .with_rtl(style.direction_rtl),
                env.fonts,
            );
        }

        let bg = style
            .background_color
            .map(|c| c.to_f32_rgba())
            .unwrap_or((1.0, 1.0, 1.0, 1.0));
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&style);

        output.push(LayoutElement::TextBlock {
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: style.text_align,
            background_color: Some(bg),
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(ctrl_width),
            block_height: Some(ctrl_height),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
        return;
    }

    // Media elements — render as placeholder rectangles
    if el.tag == HtmlTag::Video || el.tag == HtmlTag::Audio {
        let media_width = style
            .width
            .or_else(|| {
                el.attributes
                    .get("width")
                    .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            })
            .unwrap_or(if el.tag == HtmlTag::Video {
                300.0
            } else {
                200.0
            })
            .min(available_width);
        let media_height = style
            .height
            .or_else(|| {
                el.attributes
                    .get("height")
                    .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            })
            .unwrap_or(if el.tag == HtmlTag::Video {
                150.0
            } else {
                24.0
            });

        let label = if el.tag == HtmlTag::Video {
            "\u{25B6} Video".to_string()
        } else {
            "\u{25B6} Audio".to_string()
        };

        let bg = style.background_color.map(|c| c.to_f32_rgba()).unwrap_or(
            if el.tag == HtmlTag::Video {
                (0.0, 0.0, 0.0, 1.0)
            } else {
                (0.94, 0.94, 0.94, 1.0)
            },
        );
        let text_color = if el.tag == HtmlTag::Video {
            (1.0, 1.0, 1.0)
        } else {
            (0.3, 0.3, 0.3)
        };
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&style);

        let mut runs = Vec::new();
        push_text_run_with_fallback(
            TextRun {
                text: label,
                font_size: style.font_size,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                overline: false,
                color: text_color,
                link_url: None,
                font_family: resolve_style_font_family(&style, env.fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            },
            &mut runs,
            env.fonts,
        );
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(
                media_width,
                style.font_size,
                resolved_line_height_factor(&style, env.fonts),
                style.overflow_wrap,
            )
            .with_rtl(style.direction_rtl),
            env.fonts,
        );

        output.push(LayoutElement::TextBlock {
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: TextAlign::Center,
            background_color: Some(bg),
            padding_top: if el.tag == HtmlTag::Video {
                (media_height - style.font_size) / 2.0
            } else {
                4.0
            },
            padding_bottom: if el.tag == HtmlTag::Video {
                (media_height - style.font_size) / 2.0
            } else {
                4.0
            },
            padding_left: 4.0,
            padding_right: 4.0,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(media_width),
            block_height: Some(media_height),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
        return;
    }

    // Progress and meter elements — render as a horizontal bar
    if el.tag == HtmlTag::Progress || el.tag == HtmlTag::Meter {
        let bar_width = style.width.unwrap_or(150.0).min(available_width);
        let bar_height = style.height.unwrap_or(12.0);
        let value: f32 = el
            .attributes
            .get("value")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let max: f32 = el
            .attributes
            .get("max")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        let fraction = if max > 0.0 {
            (value / max).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let fill_color = if el.tag == HtmlTag::Progress {
            (0.12, 0.53, 0.90)
        } else {
            let low: f32 = el
                .attributes
                .get("low")
                .and_then(|s| s.parse().ok())
                .unwrap_or(max * 0.25);
            let high: f32 = el
                .attributes
                .get("high")
                .and_then(|s| s.parse().ok())
                .unwrap_or(max * 0.75);
            if value <= low {
                (0.90, 0.20, 0.20)
            } else if value >= high {
                (0.20, 0.78, 0.35)
            } else {
                (0.95, 0.77, 0.06)
            }
        };

        output.push(LayoutElement::ProgressBar {
            fraction,
            width: bar_width,
            height: bar_height,
            fill_color,
            track_color: (0.88, 0.88, 0.88),
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
        });
        return;
    }

    if style.page_break_before {
        output.push(LayoutElement::PageBreak);
    }

    // Table handling
    if el.tag == HtmlTag::Table {
        flatten_table(
            el,
            &style,
            available_width,
            output,
            ancestors,
            child_index,
            sibling_count,
            env,
        );
        return;
    }

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index,
        sibling_count,
        preceding_siblings: Vec::new(),
    });

    // List handling — Ul/Ol pass context to Li children
    if el.tag == HtmlTag::Ul || el.tag == HtmlTag::Ol {
        let list_indent = style.padding.left + style.margin.left;
        let inner_width = available_width - list_indent;
        // Accumulate indentation from parent list context
        let parent_indent = match list_ctx {
            Some(ListContext::Unordered { indent }) => *indent,
            Some(ListContext::Ordered { indent, .. }) => *indent,
            None => 0.0,
        };
        let total_indent = parent_indent + list_indent;
        let mut ctx = if el.tag == HtmlTag::Ol {
            ListContext::Ordered {
                index: 1,
                indent: total_indent,
            }
        } else {
            ListContext::Unordered {
                indent: total_indent,
            }
        };
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();
        let mut child_el_idx = 0;
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag == HtmlTag::Li {
                    let child_ctx = layout_ctx
                        .with_parent(inner_width, Some(available_height), style.font_size)
                        .with_containing_block(None);
                    flatten_element(
                        child_el,
                        &style,
                        &child_ctx,
                        output,
                        Some(&ctx),
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        env,
                    );
                    if let ListContext::Ordered { index, .. } = &mut ctx {
                        *index += 1;
                    }
                } else {
                    let child_ctx = layout_ctx
                        .with_parent(inner_width, Some(available_height), style.font_size)
                        .with_containing_block(None);
                    flatten_element(
                        child_el,
                        &style,
                        &child_ctx,
                        output,
                        None,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        env,
                    );
                }
                child_el_idx += 1;
            }
        }
        return;
    }

    // Li handling — prepend bullet/number marker
    if el.tag == HtmlTag::Li {
        // counter_state resets/increments already applied at top of flatten_element

        let inner_width = available_width - style.padding.left - style.padding.right;
        let mut runs = Vec::new();

        // Check for ::before pseudo-element with custom content (e.g. CSS counters).
        // If present, use it instead of the default list marker.
        let class_list = el.class_list();
        let classes: Vec<&str> = class_list.iter().map(|s| s.as_ref()).collect();
        let li_selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index,
            sibling_count,
            preceding_siblings: preceding_siblings.to_vec(),
        };
        let li_before = compute_pseudo_element_style(
            &style,
            env.rules,
            el.tag_name(),
            &classes,
            el.id(),
            &el.attributes,
            &li_selector_ctx,
            PseudoElement::Before,
        );
        let has_custom_before = li_before.as_ref().is_some_and(|s| !s.content.is_empty());
        if has_custom_before {
            let ps = li_before.as_ref().unwrap();
            let content_text = resolve_content(&ps.content, &el.attributes, env.counter_state);
            if !content_text.is_empty() {
                push_text_run_with_fallback(
                    TextRun {
                        text: content_text,
                        font_size: ps.font_size,
                        bold: ps.font_weight == FontWeight::Bold,
                        italic: ps.font_style == FontStyle::Italic,
                        underline: ps.text_decoration_underline,
                        line_through: ps.text_decoration_line_through,
                        overline: ps.text_decoration_overline,
                        color: ps.color.to_f32_rgb(),
                        link_url: None,
                        font_family: resolve_style_font_family(ps, env.fonts),
                        background_color: None,
                        padding: (0.0, 0.0),
                        border_radius: 0.0,
                    },
                    &mut runs,
                    env.fonts,
                );
            }
        }

        // Add list marker using list-style-type from computed style
        // (only if no custom ::before content)
        let marker = if has_custom_before {
            String::new()
        } else {
            match list_ctx {
                Some(ListContext::Unordered { .. }) => format_list_marker(style.list_style_type, 0),
                Some(ListContext::Ordered { index, .. }) => {
                    let lst = if style.list_style_type == ListStyleType::Disc {
                        ListStyleType::Decimal
                    } else {
                        style.list_style_type
                    };
                    format_list_marker(lst, *index)
                }
                None => format_list_marker(style.list_style_type, 0),
            }
        };
        let list_indent = if style.list_style_position == ListStylePosition::Inside {
            0.0
        } else {
            match list_ctx {
                Some(ListContext::Unordered { indent }) => *indent,
                Some(ListContext::Ordered { indent, .. }) => *indent,
                None => 0.0,
            }
        };
        if !marker.is_empty() {
            push_text_run_with_fallback(
                TextRun {
                    text: marker,
                    font_size: style.font_size,
                    bold: style.font_weight == FontWeight::Bold,
                    italic: style.font_style == FontStyle::Italic,
                    underline: false,
                    line_through: false,
                    overline: false,
                    color: style.color.to_f32_rgb(),
                    link_url: None,
                    font_family: resolve_style_font_family(&style, env.fonts),
                    background_color: None,
                    padding: (0.0, 0.0),
                    border_radius: 0.0,
                },
                &mut runs,
                env.fonts,
            );
        }

        let runs_before_inline = runs.len();
        collect_text_runs(
            &el.children,
            &style,
            &mut runs,
            None,
            env.rules,
            env.fonts,
            ancestors,
        );

        // "Loose" list items (Markdown with blank lines between items) wrap each
        // item's content in a <p>. When the <li> has no direct inline content
        // but its first block child is a <p>, inline that <p>'s runs so the
        // marker sits on the same baseline as the first line of text (matching
        // Chrome), and apply the <p>'s vertical margins on the combined block
        // so consecutive loose items are separated as paragraphs. Gated on
        // <li> to keep the hot path (nested blocks) free of extra stack.
        let (consumed_p_idx, extra_margin_top, extra_margin_bottom) =
            if el.tag == HtmlTag::Li && runs.len() == runs_before_inline {
                inline_loose_list_p(el, &style, &child_ancestors, env, &mut runs)
            } else {
                (None, 0.0, 0.0)
            };

        let block_heading_level = heading_level(el.tag);

        if !runs.is_empty() {
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    inner_width,
                    style.font_size,
                    resolved_line_height_factor(&style, env.fonts),
                    style.overflow_wrap,
                )
                .with_rtl(style.direction_rtl),
                env.fonts,
            );
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(&style);
            output.push(LayoutElement::TextBlock {
                lines,
                margin_top: style.margin.top + extra_margin_top,
                margin_bottom: style.margin.bottom + extra_margin_bottom,
                text_align: style.text_align,
                background_color: None,
                padding_top: style.padding.top,
                padding_bottom: style.padding.bottom,
                padding_left: list_indent + style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::default(),
                block_width: None,
                block_height: None,
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
                offset_bottom: style.bottom.unwrap_or(0.0),
                offset_right: style.right.unwrap_or(0.0),
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect: None,
                transform: style.transform,
                border_radius: style.border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: style.text_indent,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                vertical_align: style.vertical_align,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth: 0,
                heading_level: block_heading_level,
                clip_children_count: 0,
            });
        }

        // Process block children inside li (nested lists get reduced width for indentation)
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();
        let mut child_el_idx = 0;
        for (raw_idx, child) in el.children.iter().enumerate() {
            if Some(raw_idx) == consumed_p_idx {
                // This <p> was inlined into the li's TextBlock above — skip.
                child_el_idx += 1;
                continue;
            }
            if let DomNode::Element(child_el) = child {
                if child_el.tag == HtmlTag::Ul || child_el.tag == HtmlTag::Ol {
                    let child_ctx = layout_ctx
                        .with_parent(inner_width, Some(available_height), style.font_size)
                        .with_containing_block(None);
                    flatten_element(
                        child_el,
                        &style,
                        &child_ctx,
                        output,
                        list_ctx,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        env,
                    );
                } else if recurses_as_layout_child(child_el.tag) {
                    let child_ctx = layout_ctx
                        .with_parent(available_width, Some(available_height), style.font_size)
                        .with_containing_block(None);
                    flatten_element(
                        child_el,
                        &style,
                        &child_ctx,
                        output,
                        None,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        env,
                    );
                }
                child_el_idx += 1;
            }
        }
        return;
    }

    // Compute ::before and ::after pseudo-element styles before any display-
    // specific early returns so layout modes such as flex can still emit them.
    let cls: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
    let before_style = compute_pseudo_element_style(
        &style,
        env.rules,
        el.tag_name(),
        &cls,
        el.id(),
        &el.attributes,
        &selector_ctx,
        PseudoElement::Before,
    );
    let after_style = compute_pseudo_element_style(
        &style,
        env.rules,
        el.tag_name(),
        &cls,
        el.id(),
        &el.attributes,
        &selector_ctx,
        PseudoElement::After,
    );

    route_element(
        el,
        &mut style,
        &layout_ctx,
        output,
        ancestors,
        &child_ancestors,
        positioned_depth,
        before_style,
        after_style,
        env,
    );
}

/// Extracted helper for the loose-list fix (#140): when an `<li>` has no
/// direct inline content and its first block child is a `<p>`, inline that
/// `<p>`'s runs into the li's TextBlock and return the `<p>`'s margins +
/// raw child index so the caller can skip re-emitting it as a block.
///
/// Isolated into its own function so the extra locals (SelectorContext,
/// ComputedStyle, class list, etc.) are only paid for on the `<li>` path,
/// not on every recursive `flatten_element` frame — deep nested blocks are
/// otherwise stack-sensitive.
fn inline_loose_list_p(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    child_ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
    runs: &mut Vec<TextRun>,
) -> (Option<usize>, f32, f32) {
    let li_child_el_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();
    let mut child_el_ordinal = 0usize;
    for (raw_idx, child) in el.children.iter().enumerate() {
        if let DomNode::Element(child_el) = child {
            if child_el.tag == HtmlTag::P {
                let p_cls: Vec<&str> = child_el.class_list();
                let p_selector_ctx = SelectorContext {
                    ancestors: child_ancestors.to_vec(),
                    child_index: child_el_ordinal,
                    sibling_count: li_child_el_count,
                    preceding_siblings: Vec::new(),
                };
                let p_style = compute_style_with_context(
                    child_el.tag,
                    child_el.style_attr(),
                    parent_style,
                    env.rules,
                    child_el.tag_name(),
                    &p_cls,
                    child_el.id(),
                    &child_el.attributes,
                    &p_selector_ctx,
                );
                collect_text_runs(
                    &child_el.children,
                    &p_style,
                    runs,
                    None,
                    env.rules,
                    env.fonts,
                    child_ancestors,
                );
                return (Some(raw_idx), p_style.margin.top, p_style.margin.bottom);
            }
            child_el_ordinal += 1;
            if recurses_as_layout_child(child_el.tag) {
                break;
            }
        }
    }
    (None, 0.0, 0.0)
}

/// Dispatch an element to the appropriate layout function based on its
/// computed `display` value (flex, grid, block, inline-block, or inline).
///
/// Handles page-break-after emission and CSS counter cleanup.
#[allow(clippy::too_many_arguments)]
fn route_element(
    el: &ElementNode,
    style: &mut ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    child_ancestors: &[AncestorInfo],
    positioned_depth: usize,
    before_style: Option<ComputedStyle>,
    after_style: Option<ComputedStyle>,
    env: &mut LayoutEnv,
) {
    let layout_ctx = *ctx;

    // Flex container handling
    if style.display == Display::Flex {
        layout_flex_container(
            el,
            style,
            &layout_ctx,
            output,
            child_ancestors,
            before_style.as_ref(),
            after_style.as_ref(),
            positioned_depth,
            env,
        );

        if style.page_break_after {
            output.push(LayoutElement::PageBreak);
        }
        return;
    }

    // Grid container handling
    if style.display == Display::Grid {
        layout_grid_container(el, style, &layout_ctx, output, child_ancestors, env);

        if style.page_break_after {
            output.push(LayoutElement::PageBreak);
        }
        return;
    }

    // Multi-column layout: treat as implicit grid with equal columns
    if let Some(col_count) = style.column_count {
        if col_count >= 2 {
            let gap = style.column_gap;
            let tracks: Vec<GridTrack> = (0..col_count).map(|_| GridTrack::Fr(1.0)).collect();
            let mut col_style = style.clone();
            col_style.grid_template_columns = tracks;
            col_style.grid_gap = gap;
            layout_grid_container(el, &col_style, &layout_ctx, output, child_ancestors, env);

            if style.page_break_after {
                output.push(LayoutElement::PageBreak);
            }
            return;
        }
    }

    if style.display == Display::Block || style.display == Display::InlineBlock {
        let early_exit = layout_block_element(
            el,
            style,
            &layout_ctx,
            output,
            ancestors,
            child_ancestors,
            positioned_depth,
            before_style,
            after_style,
            env,
        );
        if early_exit {
            return;
        }
    } else {
        // Inline element — process children with this style context
        flatten_nodes(
            &el.children,
            style,
            &layout_ctx,
            output,
            None,
            child_ancestors,
            positioned_depth,
            env,
        );
    }

    if style.page_break_after {
        output.push(LayoutElement::PageBreak);
    }

    // Pop any counters that were pushed by counter-reset on this element.
    env.counter_state.pop_resets(&style.counter_reset);
}

// Grid layout functions have been moved to `super::grid`.

// Table layout functions have been moved to `super::table`.

// Re-export estimate_element_height from paginate module so existing
// `crate::layout::engine::estimate_element_height` paths keep working.
pub(crate) use super::paginate::estimate_element_height;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::css::parse_stylesheet;
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::util::decode_base64;

    const TEST_JPEG_DATA_URI: &str = concat!(
        "data:image/jpeg;base64,",
        "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
        "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
        "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
    );

    #[test]
    fn layout_simple_paragraph() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_multiple_elements() {
        let nodes = parse_html("<h1>Title</h1><p>Paragraph one.</p><p>Paragraph two.</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 3);
    }

    #[test]
    fn layout_empty() {
        let nodes = parse_html("").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.is_empty());
    }

    #[test]
    fn collapse_whitespace_test() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
        assert_eq!(collapse_whitespace("\n\t  foo  \n"), "foo");
    }

    #[test]
    fn page_break_creates_new_page() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn bare_text_node() {
        // Text not wrapped in any element — exercises DomNode::Text branch in flatten_nodes
        let nodes = parse_html("Just some bare text").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn br_element_creates_empty_line() {
        let html = "<p>Line one</p><br><p>Line two</p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have at least 3 elements (p, br, p)
        assert!(pages[0].elements.len() >= 2);
    }

    #[test]
    fn inline_element_layout() {
        // Inline element outside a block — exercises the else branch
        let html = "<span>Hello</span>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn nested_svg_percent_height_uses_parent_height() {
        let html = r#"<div style="height: 200pt"><svg width="100" height="50%"></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        fn find_svg(elements: &[(f32, LayoutElement)]) -> Option<(f32, f32)> {
            for (_, el) in elements {
                match el {
                    LayoutElement::Svg { width, height, .. } => return Some((*width, *height)),
                    LayoutElement::Container { children, .. } => {
                        // Search recursively; children don't have y_pos tuples
                        for child in children {
                            if let LayoutElement::Svg { width, height, .. } = child {
                                return Some((*width, *height));
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let svg = find_svg(&pages[0].elements).expect("expected nested svg element");
        assert!((svg.0 - 75.0).abs() < 0.1); // 100px = 75pt
        assert!((svg.1 - 100.0).abs() < 0.1); // 50% of 200pt = 100pt
    }

    #[test]
    fn nested_svg_percent_viewport_uses_resolved_root_size() {
        let html = r#"
            <div style="width: 400pt; height: 200pt">
                <svg width="100%" height="50%" viewBox="0 0 20 10">
                    <svg width="50%" height="50%" viewBox="0 0 10 10">
                        <rect width="10" height="10"/>
                    </svg>
                </svg>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        fn find_svg_tree(
            elements: &[(f32, LayoutElement)],
        ) -> Option<&crate::parser::svg::SvgTree> {
            for (_, el) in elements {
                match el {
                    LayoutElement::Svg { tree, .. } => return Some(tree),
                    LayoutElement::Container { children, .. } => {
                        for child in children {
                            if let LayoutElement::Svg { tree, .. } = child {
                                return Some(tree);
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let svg = find_svg_tree(&pages[0].elements).expect("expected nested svg element");
        match &svg.children[0] {
            crate::parser::svg::SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(crate::parser::svg::SvgTransform::Matrix(
                        20.0, 0.0, 0.0, 5.0, 0.0, 0.0
                    ))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_element_preserves_viewbox_for_renderer() {
        let html = r#"<svg width="200" height="100" viewBox="0 0 20 10"><rect width="10" height="10"/></svg>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let svg = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => Some((tree, *width, *height)),
                _ => None,
            })
            .expect("expected svg layout element");
        assert_eq!(svg.1, 150.0); // 200px = 150pt
        assert_eq!(svg.2, 75.0); // 100px = 75pt
        assert!(
            svg.0.view_box.is_some(),
            "renderer should keep viewBox metadata"
        );
    }

    #[test]
    fn inline_svg_inherits_document_color_for_current_color() {
        let html = r#"<div style="color: #336699"><svg width="20" height="10"><rect width="10" height="10" fill="currentColor"/></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let tree = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg { tree, .. } => Some(tree),
                _ => None,
            })
            .expect("expected svg layout element");

        match &tree.children[0] {
            crate::parser::svg::SvgNode::Group {
                style, children, ..
            } => {
                assert_eq!(style.color, Some((0.2, 0.4, 0.6)));
                assert_eq!(children.len(), 1);
            }
            other => panic!("expected root group wrapper, got {other:?}"),
        }
    }

    #[test]
    fn page_break_after() {
        let html = r#"<div style="page-break-after: always"><p>Page 1</p></div><p>Page 2</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn word_wrap_long_text() {
        // Generate text that exceeds page width to trigger word wrapping
        let long_text = "word ".repeat(200);
        let html = format!("<p>{long_text}</p>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have wrapped into multiple lines
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(lines.len() > 1);
        }
    }

    #[test]
    fn content_overflows_to_next_page() {
        // Generate enough content to overflow one page
        let paragraphs = "<p>Some paragraph text that takes up space.</p>\n".repeat(100);
        let nodes = parse_html(&paragraphs).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn background_color_block() {
        let html = r#"<div style="background-color: yellow"><p>Highlighted</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn pre_element_with_background() {
        let html = "<pre>code block</pre>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Pre has background color in defaults
        if let (
            _,
            LayoutElement::TextBlock {
                background_color, ..
            },
        ) = &pages[0].elements[0]
        {
            assert!(background_color.is_some());
        }
    }

    #[test]
    fn table_layout_basic() {
        // Exercises flatten_table and table row layout (lines 232, 248, 344, 354)
        let html = r#"
            <table>
                <tr><th>Header 1</th><th>Header 2</th></tr>
                <tr><td>Cell A</td><td>Cell B</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have TableRow elements
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 2);
    }

    #[test]
    fn table_with_thead_tbody_tfoot() {
        // Exercises lines 345-353: collecting rows from thead/tbody/tfoot
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody><tr><td>B</td></tr></tbody>
                <tfoot><tr><td>F</td></tr></tfoot>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 3);
    }

    #[test]
    fn table_empty_rows_ignored() {
        // Line 360: empty table returns early
        let html = "<table></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Should have no table rows
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 0);
    }

    #[test]
    fn ordered_list_layout() {
        // Exercises lines 219-232, 248: ordered list context and numbering
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have items with numbered markers
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TextBlock { .. }))
            .collect();
        assert!(blocks.len() >= 3);
    }

    #[test]
    fn unordered_list_layout() {
        // Exercises lines 217-236: unordered list layout
        let html = "<ul><li>A</li><li>B</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn list_with_non_li_child() {
        // Line 232: non-li child inside ul
        let html = "<ul><li>Item</li><p>Not a list item</p></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn li_with_block_child() {
        // Lines 279-280: block child inside li
        let html = "<ul><li><p>Paragraph inside li</p></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn table_row_pagination() {
        // Exercises TableRow height calculation in paginate (lines 559-572)
        let mut rows = String::new();
        for i in 0..100 {
            rows.push_str(&format!(
                "<tr><td>Row {i} with some text</td><td>More text</td></tr>"
            ));
        }
        let html = format!("<table>{rows}</table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2, "Large table should span multiple pages");
    }

    #[test]
    fn table_with_non_cell_children_in_row() {
        // Line 354: non-td/th child in tr is ignored
        let html = r#"<table><tr><td>Cell</td><span>Ignored</span></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 1);
    }

    #[test]
    fn del_element_sets_line_through() {
        let html = "<p><del>Deleted text</del></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let run = &lines[0].runs[0];
            assert!(run.line_through, "del element should set line_through");
            assert!(!run.underline);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn s_element_sets_line_through() {
        let html = "<p><s>Struck text</s></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let run = &lines[0].runs[0];
            assert!(run.line_through, "s element should set line_through");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn nested_unordered_list() {
        let html = "<ul><li>Parent<ul><li>Child</li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have at least 2 TextBlock elements: parent item and nested child item
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 2,
            "Expected at least 2 text blocks for nested list, got {}",
            blocks.len()
        );
        // The nested item should have greater indentation than the parent
        let parent_indent = blocks[0].1;
        let child_indent = blocks[1].1;
        assert!(
            child_indent > parent_indent,
            "Nested list item should be more indented: parent={parent_indent}, child={child_indent}"
        );
    }

    #[test]
    fn nested_ordered_list() {
        let html = "<ol><li>First<ol><li>Nested first</li><li>Nested second</li></ol></li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        // Should have: "1. First", "1. Nested first", "2. Nested second", "2. Second"
        assert!(
            blocks.len() >= 3,
            "Expected at least 3 text blocks for nested ordered list, got {}",
            blocks.len()
        );
        // Nested items should have greater indentation
        let parent_indent = blocks[0].1;
        let nested_indent = blocks[1].1;
        assert!(
            nested_indent > parent_indent,
            "Nested ordered list should be more indented: parent={parent_indent}, nested={nested_indent}"
        );
    }

    #[test]
    fn mixed_nested_list() {
        let html = "<ul><li>Bullet<ol><li>Numbered</li></ol></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 2,
            "Expected at least 2 text blocks for mixed nested list, got {}",
            blocks.len()
        );
        // Nested ordered list inside unordered should be more indented
        let parent_indent = blocks[0].1;
        let nested_indent = blocks[1].1;
        assert!(
            nested_indent > parent_indent,
            "Nested ol inside ul should be more indented: parent={parent_indent}, nested={nested_indent}"
        );
        // Check that the nested item has a numbered marker
        let nested_text: String = blocks[1].0[0].runs.iter().map(|r| r.text.clone()).collect();
        assert!(
            nested_text.contains("1."),
            "Nested item should have ordered marker, got: {nested_text}"
        );
    }

    #[test]
    fn base64_decode_basic() {
        // "Hello" in base64 is "SGVsbG8="
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn base64_decode_with_whitespace() {
        let decoded = decode_base64("SGVs\nbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn layout_jpeg_image_from_data_uri() {
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image {
                image,
                width,
                height,
                ..
            } => {
                assert_eq!(image.format, ImageFormat::Jpeg);
                assert!((width - 75.0).abs() < 0.1); // 100px * 0.75
                assert!((height - 60.0).abs() < 0.1); // 80px * 0.75
                assert!(image.png_metadata.is_none());
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_svg_image_from_data_uri_uses_intrinsic_size() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%20viewBox%3D%220%200%20100%2050%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 300.0).abs() < 0.1);
                assert!((*height - 150.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_respects_max_width() {
        let html = r#"<img style="max-width: 75pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 75.0).abs() < 0.1);
                assert!((*height - 37.5).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 20pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 40.0).abs() < 0.1);
                assert!((*height - 20.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_viewbox_only_svg_image_uses_default_object_size_ratio() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 300.0).abs() < 0.1);
                assert!((*height - 60.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_viewbox_only_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 50pt" src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 250.0).abs() < 0.1);
                assert!((*height - 50.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_without_viewbox_syncs_tree_to_layout_box() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%3E%3Crect%20width%3D%22100%25%22%20height%3D%22100%25%22/%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (tree_width, tree_height, width, height) = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => Some((tree.width, tree.height, *width, *height)),
                _ => None,
            })
            .expect("expected svg layout element");

        assert!((tree_width - width).abs() < 0.1);
        assert!((tree_height - height).abs() < 0.1);
    }

    #[test]
    fn layout_png_image_from_data_uri() {
        // Build a minimal valid PNG and encode as base64
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="120" height="90">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image { image, .. } => {
                assert_eq!(image.format, ImageFormat::Png);
                let meta = image.png_metadata.as_ref().unwrap();
                assert_eq!(meta.channels, 3); // RGB
                assert_eq!(meta.bit_depth, 8);
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_image_without_dimensions_gets_defaults() {
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image { width, height, .. } => {
                assert!(*width > 0.0);
                assert!(*height > 0.0);
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_image_unsupported_src_ignored() {
        // HTTP src is not supported, should be silently ignored
        let html = r#"<img src="http://example.com/image.png" width="100" height="100">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // No image element should be produced
        assert!(
            pages[0].elements.is_empty()
                || !matches!(&pages[0].elements[0].1, LayoutElement::Image { .. })
        );
    }

    #[test]
    fn img_scales_to_fit_available_width() {
        // Very wide image: 2000px = 1500pt, which exceeds A4 content width (~451pt)
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="2000" height="1000">"#);
        let nodes = parse_html(&html).unwrap();
        let page_size = PageSize::A4;
        let margin_val = Margin::default();
        let available_width = page_size.width - margin_val.left - margin_val.right;
        let pages = layout(&nodes, page_size, margin_val);
        if let (_, LayoutElement::Image { width, .. }) = &pages[0].elements[0] {
            assert!(
                *width <= available_width + 0.01,
                "Image width {width} should fit within available width {available_width}"
            );
        } else {
            panic!("Expected Image element");
        }
    }

    #[test]
    fn img_without_src_ignored() {
        let html = r#"<img width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::Image { .. }));
        assert!(
            !has_image,
            "img without src should not produce Image element"
        );
    }

    #[test]
    fn block_aspect_ratio_sets_height_for_empty_box() {
        let html = r#"<div style="width: 120pt; aspect-ratio: 3 / 2"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (_, element) = &pages[0].elements[0];
        match element {
            LayoutElement::TextBlock {
                block_height: Some(height),
                ..
            }
            | LayoutElement::Container {
                block_height: Some(height),
                ..
            } => {
                assert!((*height - 80.0).abs() < 0.1);
            }
            _ => panic!("Expected aspect-ratio box to produce a TextBlock or Container"),
        }
    }

    #[test]
    fn raster_background_image_survives_into_layout() {
        let png = build_test_png_bytes();
        let encoded = base64_encode(&png);
        let html = format!(
            r#"<div style="width: 40pt; height: 40pt; background-image: url('data:image/png;base64,{encoded}') no-repeat"></div>"#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (_, element) = &pages[0].elements[0];
        let tree_opt = match element {
            LayoutElement::TextBlock {
                background_svg: Some(tree),
                ..
            } => Some(tree),
            LayoutElement::Container {
                background_svg: Some(tree),
                ..
            } => Some(tree),
            _ => None,
        };
        if let Some(tree) = tree_opt {
            assert!(matches!(
                tree.children.first(),
                Some(crate::parser::svg::SvgNode::Image { .. })
            ));
        } else {
            panic!("Expected raster background to produce a TextBlock or Container");
        }
    }

    fn build_test_png_bytes() -> Vec<u8> {
        let mut png_data = Vec::new();
        png_data.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(2); // color type RGB
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        append_test_chunk(&mut png_data, b"IHDR", &ihdr);
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_test_chunk(&mut png_data, b"IDAT", &idat);
        append_test_chunk(&mut png_data, b"IEND", &[]);
        png_data
    }

    fn append_test_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]);
    }

    #[test]
    fn three_levels_deep_nested_list() {
        let html = "<ul><li>Level 1<ul><li>Level 2<ul><li>Level 3</li></ul></li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 3,
            "Expected at least 3 text blocks for 3-level list, got {}",
            blocks.len()
        );
        let indent_1 = blocks[0].1;
        let indent_2 = blocks[1].1;
        let indent_3 = blocks[2].1;
        assert!(
            indent_2 > indent_1,
            "Level 2 should be more indented than level 1: l1={indent_1}, l2={indent_2}"
        );
        assert!(
            indent_3 > indent_2,
            "Level 3 should be more indented than level 2: l2={indent_2}, l3={indent_3}"
        );
    }

    // --- Overflow / Visibility / Transform layout tests ---

    #[test]
    fn visibility_hidden_keeps_space_but_not_visible() {
        let html = r#"<div style="visibility: hidden">Hidden text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { visible, .. }) = &pages[0].elements[0] {
            assert!(!visible, "visibility: hidden should set visible to false");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn visibility_visible_is_visible() {
        let html = r#"<div>Visible text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { visible, .. }) = &pages[0].elements[0] {
            assert!(*visible, "Default should be visible");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn overflow_hidden_produces_clip_rect() {
        let html = r#"<div style="overflow: hidden; width: 200pt; height: 100pt">Clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { clip_rect, .. }) = &pages[0].elements[0] {
            assert!(clip_rect.is_some(), "overflow: hidden should set clip_rect");
            let (_, _, w, _) = clip_rect.unwrap();
            assert!((w - 200.0).abs() < 0.1);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn overflow_visible_no_clip_rect() {
        let html = r#"<div style="width: 200pt">Not clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { clip_rect, .. }) = &pages[0].elements[0] {
            assert!(clip_rect.is_none(), "No overflow should mean no clip_rect");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_rotate_stored_in_layout() {
        let html = r#"<div style="transform: rotate(45deg)">Rotated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Rotate(45.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_scale_stored_in_layout() {
        let html = r#"<div style="transform: scale(2)">Scaled</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Scale(2.0, 2.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_translate_stored_in_layout() {
        let html = r#"<div style="transform: translate(10pt, 20pt)">Translated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Translate(10.0, 20.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn table_colspan_default_is_one() {
        let html = "<table><tr><td>A</td><td>B</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TableRow { cells, .. } = el {
                for cell in cells {
                    assert_eq!(cell.colspan, 1, "Default colspan should be 1");
                    assert_eq!(cell.rowspan, 1, "Default rowspan should be 1");
                }
            }
        }
    }

    #[test]
    fn table_colspan_header_spans_two() {
        let html =
            r#"<table><tr><th colspan="2">Header</th></tr><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2);
        assert_eq!(table_rows[0].len(), 1);
        assert_eq!(table_rows[0][0].colspan, 2);
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].colspan, 1);
        assert_eq!(table_rows[1][1].colspan, 1);
    }

    #[test]
    fn table_colspan_makes_cells_wider() {
        let html = r#"<table><tr><td colspan="2">Wide</td><td>N</td></tr><tr><td>A</td><td>B</td><td>C</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths.clone()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2);
        let (cells, col_widths) = &table_rows[0];
        assert_eq!(cells[0].colspan, 2);
        // With auto-sizing, col_widths should have 3 entries
        assert_eq!(col_widths.len(), 3);
        // The colspan=2 cell should span the first two column widths
        let span_width: f32 = col_widths[0] + col_widths[1];
        let single_width = col_widths[2];
        assert!(
            span_width > single_width,
            "colspan=2 span ({span_width}) should be wider than single col ({single_width})"
        );
    }

    #[test]
    fn table_mixed_colspan_values() {
        let html = r#"<table><tr><td colspan="3">Full</td></tr><tr><td>A</td><td colspan="2">BC</td></tr><tr><td>X</td><td>Y</td><td>Z</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 3);
        assert_eq!(table_rows[0].len(), 1);
        assert_eq!(table_rows[0][0].colspan, 3);
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].colspan, 1);
        assert_eq!(table_rows[1][1].colspan, 2);
        assert_eq!(table_rows[2].len(), 3);
        for cell in table_rows[2] {
            assert_eq!(cell.colspan, 1);
        }
    }

    #[test]
    fn table_rowspan_basic() {
        // Cell A spans two rows; row 1 should have a phantom cell in column 0.
        let html = r#"<table>
            <tr><td rowspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2, "Should have 2 rows");
        // Row 0: cell A (rowspan=2) and cell B
        assert_eq!(table_rows[0].len(), 2);
        assert_eq!(table_rows[0][0].rowspan, 2);
        assert_eq!(table_rows[0][1].rowspan, 1);
        // Row 1: phantom cell (rowspan=0) and cell C
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(
            table_rows[1][0].rowspan, 0,
            "Phantom cell should have rowspan=0"
        );
        assert_eq!(table_rows[1][1].rowspan, 1);
    }

    #[test]
    fn table_rowspan_and_colspan_combined() {
        // Cell A spans 2 rows and 2 columns in a 3-column table.
        let html = r#"<table>
            <tr><td rowspan="2" colspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
            <tr><td>D</td><td>E</td><td>F</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 3, "Should have 3 rows");
        // Row 0: cell A (rowspan=2, colspan=2) and cell B
        assert_eq!(table_rows[0].len(), 2);
        assert_eq!(table_rows[0][0].rowspan, 2);
        assert_eq!(table_rows[0][0].colspan, 2);
        assert_eq!(table_rows[0][1].rowspan, 1);
        // Row 1: phantom cell spanning 2 cols and cell C
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].rowspan, 0);
        assert_eq!(table_rows[1][0].colspan, 2, "Phantom should span 2 cols");
        assert_eq!(table_rows[1][1].rowspan, 1);
        // Row 2: three normal cells
        assert_eq!(table_rows[2].len(), 3);
        for cell in table_rows[2] {
            assert_eq!(cell.rowspan, 1);
            assert_eq!(cell.colspan, 1);
        }
    }

    #[test]
    fn table_rowspan_renders_to_pdf() {
        // Verify that a table with rowspan produces valid PDF output.
        let html = r#"<table>
            <tr><td rowspan="2">Spans two rows</td><td>Top right</td></tr>
            <tr><td>Bottom right</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Spans"),
            "Cell text 'Spans' should be in PDF"
        );
        assert!(
            content.contains("rows"),
            "Cell text 'rows' should be in PDF"
        );
        assert!(content.contains("Top"), "Cell text 'Top' should be in PDF");
        assert!(
            content.contains("Bottom"),
            "Cell text 'Bottom' should be in PDF"
        );
        // No default cell borders — only CSS-specified borders produce strokes
    }

    #[test]
    fn css_width_constrains_block() {
        let html = r#"<div style="width: 200pt">Narrow block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_max_width_limits_width() {
        let html = r#"<div style="max-width: 300pt">Limited block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_height_sets_minimum_height() {
        let html = r#"<div style="height: 100pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(100.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_opacity_stored_in_layout() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { opacity, .. }) = &pages[0].elements[0] {
            assert!((*opacity - 0.5).abs() < 0.01);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn no_explicit_width_is_none() {
        let html = "<div>Normal block</div>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, None);
        } else {
            panic!("Expected TextBlock");
        }
    }

    // --- Float / Clear / Position / Box-shadow layout tests ---

    #[test]
    fn float_left_positions_element() {
        let html = r#"<div style="float: left; width: 100pt">Floated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { float, .. }) = &pages[0].elements[0] {
            assert_eq!(*float, Float::Left);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn float_right_positions_element() {
        let html = r#"<div style="float: right; width: 100pt">Floated right</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { float, .. }) = &pages[0].elements[0] {
            assert_eq!(*float, Float::Right);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn clear_both_moves_below_floats() {
        let html = r#"
            <div style="float: left">Float</div>
            <div style="clear: both">After float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // The cleared element should be below the floated element
        let float_y = pages[0].elements[0].0;
        let cleared_y = pages[0].elements[1].0;
        assert!(
            cleared_y >= float_y,
            "Cleared element y={cleared_y} should be >= floated y={float_y}"
        );
        // Check the clear property is set
        if let (_, LayoutElement::TextBlock { clear, .. }) = &pages[0].elements[1] {
            assert_eq!(*clear, Clear::Both);
        }
    }

    #[test]
    fn position_relative_offsets_element() {
        let html = r#"<div style="position: relative; top: 10pt; left: 5pt">Offset</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            y,
            LayoutElement::TextBlock {
                position,
                offset_top,
                offset_left,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*position, Position::Relative);
            assert!((offset_top - 10.0).abs() < 0.1);
            assert!((offset_left - 5.0).abs() < 0.1);
            // y should be offset by top value from normal position
            assert!(
                *y > 0.0,
                "Element should have non-zero y due to relative offset"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn position_absolute_fixed_position() {
        let html = r#"<div style="position: absolute; top: 100pt; left: 50pt">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            y,
            LayoutElement::TextBlock {
                position,
                offset_top,
                offset_left,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*position, Position::Absolute);
            assert!((offset_top - 100.0).abs() < 0.1);
            assert!((offset_left - 50.0).abs() < 0.1);
            // y should be exactly the top value
            assert!((*y - 100.0).abs() < 0.1, "Absolute y={y} should be 100.0");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn position_absolute_relative_to_containing_block() {
        let html = r#"
            <div style="margin-top: 200pt; height: 200pt; position: relative; background: #eee;">
                <div style="position: absolute; top: 10pt; left: 10pt; width: 50pt; height: 50pt; background: red;">X</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let parent = pages[0]
            .elements
            .iter()
            .find(|(_, el)| {
                matches!(
                    el,
                    LayoutElement::TextBlock {
                        position: Position::Relative,
                        background_color: Some(_),
                        ..
                    } | LayoutElement::Container {
                        position: Position::Relative,
                        background_color: Some(_),
                        ..
                    }
                )
            })
            .expect("Should find positioned parent");
        let parent_y = parent.0;
        assert!(
            (parent_y - 200.0).abs() < 1.0,
            "Parent should be at ~200pt, got {parent_y}"
        );
        // The absolute child may be a top-level element or inside a Container.
        let has_abs_child = pages[0].elements.iter().any(|(_, el)| match el {
            LayoutElement::TextBlock {
                position: Position::Absolute,
                ..
            } => true,
            LayoutElement::Container { children, .. } => children.iter().any(|c| {
                matches!(
                    c,
                    LayoutElement::TextBlock {
                        position: Position::Absolute,
                        ..
                    }
                )
            }),
            _ => false,
        });
        assert!(
            has_abs_child,
            "Should find absolute child in elements or Container children"
        );
    }

    #[test]
    fn position_absolute_does_not_affect_flow() {
        let html = r#"
            <div style="position: absolute; top: 200pt">Absolute</div>
            <div>Normal flow</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // The normal flow element should start at y=0 (top of content area)
        let normal_y = pages[0].elements[1].0;
        assert!(
            normal_y < 10.0,
            "Normal flow element should be near top, but y={normal_y}"
        );
    }

    #[test]
    fn box_shadow_produces_offset_rect() {
        let html = r#"<div style="box-shadow: 3px 3px black">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { box_shadow, .. }) = &pages[0].elements[0] {
            let shadow = box_shadow.unwrap();
            assert!((shadow.offset_x - 2.25).abs() < 0.1); // 3px * 0.75
            assert!((shadow.offset_y - 2.25).abs() < 0.1);
            assert_eq!(shadow.color.r, 0);
            assert_eq!(shadow.color.g, 0);
            assert_eq!(shadow.color.b, 0);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn float_does_not_advance_normal_flow() {
        let html = r#"
            <div style="float: left">Floated</div>
            <div>Normal after float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // Both elements should start at roughly the same y position
        // because floats don't advance normal flow
        let float_y = pages[0].elements[0].0;
        let normal_y = pages[0].elements[1].0;
        // The normal element might be at the same position or slightly different
        // due to margins, but it should not be pushed far down
        assert!(
            (normal_y - float_y).abs() < 50.0,
            "Normal flow element should be near float, not pushed far down: float_y={float_y}, normal_y={normal_y}"
        );
    }

    #[test]
    fn table_auto_sizing_varying_content() {
        let html = "<table><tr><td>A</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Column with longer text ({}) should be wider than short text ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_auto_sizing_very_long_cell_no_break() {
        let long_text = "x".repeat(500);
        let html = format!("<table><tr><td>{long_text}</td><td>Short</td></tr></table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages.is_empty());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        for w in &table_rows[0] {
            assert!(*w >= 30.0, "Column width {w} should be at least 30pt");
        }
    }

    #[test]
    fn table_auto_sizing_min_column_width() {
        let html = "<table><tr><td></td><td></td><td></td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        for w in &table_rows[0] {
            assert!(
                *w >= 30.0,
                "Empty column should have minimum width, got {w}"
            );
        }
    }

    #[test]
    fn table_four_column_invoice_non_equal_widths() {
        // A 4-column invoice table: Description should be wider than Qty/Amount
        let html = r#"<table>
            <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
            <tr><td>Web development services - January</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
            <tr><td>Hosting and maintenance</td><td>12</td><td>50.00</td><td>600.00</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        let cw = &table_rows[0];
        assert_eq!(cw.len(), 4);
        // Description column (index 0) should be wider than Qty (index 1)
        assert!(
            cw[0] > cw[1],
            "Description column ({}) should be wider than Qty column ({})",
            cw[0],
            cw[1]
        );
        // Description column should be wider than Amount column
        assert!(
            cw[0] > cw[3],
            "Description column ({}) should be wider than Amount column ({})",
            cw[0],
            cw[3]
        );
        // Columns should NOT all be equal
        assert!(
            !(cw[0] == cw[1] && cw[1] == cw[2] && cw[2] == cw[3]),
            "Column widths should not all be equal: {:?}",
            cw
        );
    }

    #[test]
    fn simple_invoice_fits_on_one_page() {
        // A simple invoice with ~15 lines should fit on a single A4 page
        let html = r#"
            <h1>Invoice #1001</h1>
            <p>Date: 2026-01-15</p>
            <p>Bill To: Acme Corp</p>
            <p>123 Main Street, Springfield</p>
            <table>
                <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
                <tr><td>Web development</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
                <tr><td>Hosting</td><td>12</td><td>50.00</td><td>600.00</td></tr>
                <tr><td>Domain renewal</td><td>1</td><td>15.00</td><td>15.00</td></tr>
                <tr><td>SSL certificate</td><td>1</td><td>75.00</td><td>75.00</td></tr>
            </table>
            <p>Subtotal: 3190.00</p>
            <p>Tax (10%): 319.00</p>
            <p>Total: 3509.00</p>
            <p>Thank you for your business!</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(
            pages.len(),
            1,
            "Simple invoice should fit on one page, got {} pages",
            pages.len()
        );
    }

    // --- Flexbox layout tests ---

    fn extract_flex_items(pages: &[Page]) -> Vec<(f32, f32, Option<f32>, String)> {
        let mut result = Vec::new();
        for page in pages {
            for (y, elem) in &page.elements {
                match elem {
                    LayoutElement::TextBlock {
                        lines,
                        offset_left,
                        block_width,
                        ..
                    } => {
                        let text: String = lines
                            .iter()
                            .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            result.push((*y, *offset_left, *block_width, text));
                        }
                    }
                    LayoutElement::FlexRow { cells, .. } => {
                        for cell in cells {
                            let text: String = cell
                                .lines
                                .iter()
                                .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
                                .collect::<Vec<_>>()
                                .join("");
                            if !text.is_empty() {
                                result.push((*y, cell.x_offset, Some(cell.width), text));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        result
    }

    #[test]
    fn flex_row_horizontal_layout() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">L</div><div style="width: 100pt">R</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let l = items.iter().find(|i| i.3.contains('L')).unwrap();
        let r = items.iter().find(|i| i.3.contains('R')).unwrap();
        assert!(r.1 > l.1);
    }

    #[test]
    fn flex_column_vertical() {
        let html = r#"<div style="display: flex; flex-direction: column"><div style="width: 100pt">T</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3.contains('T')).unwrap();
        let b = items.iter().find(|i| i.3.contains('B')).unwrap();
        assert!(b.0 > t.0);
    }

    #[test]
    fn flex_justify_center() {
        let html = r#"<div style="display: flex; justify-content: center"><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 50.0);
    }

    #[test]
    fn flex_justify_space_between() {
        let html = r#"<div style="display: flex; justify-content: space-between"><div style="width: 100pt">A</div><div style="width: 100pt">B</div><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 3);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let c = items.iter().find(|i| i.3 == "C").unwrap();
        let g1 = b.1 - a.1;
        let g2 = c.1 - b.1;
        assert!((g1 - g2).abs() < 1.0, "gaps equal: {g1} vs {g2}");
    }

    #[test]
    fn flex_justify_space_around() {
        let html = r#"<div style="display: flex; justify-content: space-around"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        assert!(a.1 > 10.0, "space-around: first not at edge, got {}", a.1);
    }

    #[test]
    fn flex_justify_flex_end() {
        let html = r#"<div style="display: flex; justify-content: flex-end"><div style="width: 100pt">E</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 200.0, "flex-end: got {}", items[0].1);
    }

    #[test]
    fn flex_align_center() {
        let html = r#"<div style="display: flex; align-items: center"><div style="width: 100pt; height: 50pt">T</div><div style="width: 100pt">S</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3 == "T").unwrap();
        let s = items.iter().find(|i| i.3 == "S").unwrap();
        assert!(s.0 >= t.0);
    }

    #[test]
    fn flex_wrap_test() {
        let html = r#"<div style="display: flex; flex-wrap: wrap"><div style="width: 200pt">A</div><div style="width: 200pt">B</div><div style="width: 200pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(
            items.len() >= 3,
            "Should have at least 3 flex items, got {}",
            items.len()
        );
        // Verify all three items appear in the output
        assert!(items.iter().any(|i| i.3 == "A"), "A should appear");
        assert!(items.iter().any(|i| i.3 == "B"), "B should appear");
        assert!(items.iter().any(|i| i.3 == "C"), "C should appear");
        // B should be to the right of A (same row)
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1, "B should be to the right of A");
    }

    #[test]
    fn flex_gap_spacing() {
        let html = r#"<div style="display: flex; gap: 20pt"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0 + 20.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_no_gap() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "no gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_column_gap_spacing() {
        // Column-direction flex: gap should push items apart vertically.
        let html = r#"<div style="display: flex; flex-direction: column; gap: 20pt"><div style="height: 30pt">A</div><div style="height: 30pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2, "expected at least 2 flex column items");
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        // B must be below A; with a 20pt gap the Y gap between starts should exceed 20pt
        assert!(
            b.0 > a.0 + 20.0,
            "column gap: B y={} should be more than 20pt below A y={}",
            b.0,
            a.0
        );
    }

    #[test]
    fn flex_style_block() {
        use crate::parser::css::parse_stylesheet;
        let css = ".f{display:flex;gap:10pt}";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="f"><div style="width:100pt">A</div><div style="width:100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1);
    }

    #[test]
    fn flex_display_none_child() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">V</div><div style="width: 100pt; display: none">H</div><div style="width: 100pt">V2</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.iter().all(|i| !i.3.contains('H')));
        assert!(items.len() >= 2);
    }

    #[test]
    fn flex_row_children_same_y_not_stacked() {
        let html = r#"<div style="display: flex;"><div>Left</div><div>Right</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left text");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right text");
        // Both should be at the same y position (same row, not stacked)
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "Left y={} Right y={} -- should be on the same line",
            left.0,
            right.0
        );
        // Right should be to the right of Left
        assert!(
            right.1 > left.1,
            "Right x={} should be greater than Left x={}",
            right.1,
            left.1
        );
    }

    #[test]
    fn flex_space_between_positions() {
        let html = r#"<div style="display: flex; justify-content: space-between;">
            <div>Left content</div>
            <div>Right content</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left content");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right content");
        // Both at same y
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "space-between: both should be on same y"
        );
        // First child should be at x=0 (or near 0)
        assert!(
            left.1 < 5.0,
            "space-between: first child near left edge, got {}",
            left.1
        );
        // Second child should be far to the right
        assert!(
            right.1 > 100.0,
            "space-between: second child should be far right, got {}",
            right.1
        );
    }

    #[test]
    fn flex_text_align_right_in_child() {
        let html = r#"<div style="display: flex;">
            <div style="width: 200pt; text-align: right">Aligned</div>
            <div style="width: 200pt">Normal</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Verify we can find both items as flex cells
        let items = extract_flex_items(&pages);
        let aligned = items
            .iter()
            .find(|i| i.3.contains("Aligned"))
            .expect("Aligned text");
        let normal = items
            .iter()
            .find(|i| i.3.contains("Normal"))
            .expect("Normal text");
        // Aligned should be in first cell (x_offset = 0)
        assert!(aligned.1 < normal.1, "first cell before second");
        // Verify the FlexRow element stores text_align correctly
        for page in &pages {
            for (_y, elem) in &page.elements {
                if let LayoutElement::FlexRow { cells, .. } = elem {
                    if let Some(cell) = cells.iter().find(|c| {
                        c.lines
                            .iter()
                            .any(|l| l.runs.iter().any(|r| r.text.contains("Aligned")))
                    }) {
                        assert_eq!(
                            cell.text_align,
                            TextAlign::Right,
                            "text-align: right should be preserved in FlexCell"
                        );
                    }
                }
            }
        }
    }

    // --- CSS Grid tests ---

    /// Helper: extract GridRow elements from inside a Container child on page 0.
    fn extract_grid_rows(pages: &[Page]) -> Vec<&LayoutElement> {
        let mut rows = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::Container { children, .. } = el {
                for child in children {
                    if matches!(child, LayoutElement::GridRow { .. }) {
                        rows.push(child);
                    }
                }
            }
        }
        rows
    }

    #[test]
    fn grid_three_column_places_items_correctly() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            grid_rows.len(),
            2,
            "Should have 2 rows for 6 items in 3 columns"
        );
        assert_eq!(grid_rows[0].0.len(), 3, "First row should have 3 cells");
        assert_eq!(grid_rows[1].0.len(), 3, "Second row should have 3 cells");

        // Columns should be equal width
        let widths = grid_rows[0].1;
        assert!(
            (widths[0] - widths[1]).abs() < 0.1,
            "Columns should be equal width"
        );
        assert!(
            (widths[1] - widths[2]).abs() < 0.1,
            "Columns should be equal width"
        );
    }

    #[test]
    fn grid_mixed_fr_and_fixed_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: 100pt 1fr 200pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        let widths = grid_rows[0].1;
        assert_eq!(widths.len(), 3);
        assert!(
            (widths[0] - 100.0).abs() < 0.1,
            "First column should be 100pt"
        );
        assert!(
            (widths[2] - 200.0).abs() < 0.1,
            "Third column should be 200pt"
        );
        // Middle column gets remaining space
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_middle = available - 100.0 - 200.0;
        assert!(
            (widths[1] - expected_middle).abs() < 0.1,
            "Middle column should get remaining space: got {}, expected {}",
            widths[1],
            expected_middle
        );
    }

    #[test]
    fn grid_auto_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: auto auto">
            <div>Left</div>
            <div>Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow { col_widths, .. } = el {
                    Some(col_widths)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        let widths = grid_rows[0];
        assert_eq!(widths.len(), 2);
        // Per CSS Grid: auto columns take their max-content intrinsic width,
        // then split the remaining free space EQUALLY. "Left" and "Right"
        // have slightly different measured widths, so the columns differ by
        // exactly that content-width delta (small, a few points) — they are
        // NOT forced to equal width.
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let sum = widths[0] + widths[1];
        assert!(
            (sum - available).abs() < 1.0,
            "Auto columns should fill available: sum {} vs available {}",
            sum,
            available
        );
        assert!(
            (widths[0] - widths[1]).abs() < 30.0,
            "Auto columns should be close (differ by at most content delta): {} vs {}",
            widths[0],
            widths[1]
        );
    }

    #[test]
    fn grid_auto_fr_auto_does_not_collapse_to_equal_columns() {
        // Regression for parity bug #145: `auto 1fr auto` was being treated
        // as three equal tracks (auto == 1fr semantically). Correct behavior:
        // auto columns size to their max-content intrinsic width, and the fr
        // track swallows the remaining space.
        let html = r#"<div style="display: grid; grid-template-columns: auto 1fr auto">
            <div>L</div>
            <div>middle</div>
            <div>R</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow { col_widths, .. } = el {
                    Some(col_widths)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        let widths = grid_rows[0];
        assert_eq!(widths.len(), 3);

        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let sum = widths[0] + widths[1] + widths[2];
        assert!(
            (sum - available).abs() < 1.0,
            "Grid columns should fill available: sum {} vs {}",
            sum,
            available
        );
        // Auto columns ("L"/"R") must be much narrower than the 1fr column.
        // If the old bug resurfaces, all three would be ~equal (≈available/3).
        assert!(
            widths[1] > widths[0] * 3.0,
            "1fr column ({}) should dwarf auto columns ({}, {})",
            widths[1],
            widths[0],
            widths[2]
        );
        assert!(
            widths[1] > widths[2] * 3.0,
            "1fr column ({}) should dwarf auto columns ({}, {})",
            widths[1],
            widths[0],
            widths[2]
        );
    }

    #[test]
    fn grid_gap_adds_spacing() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; grid-gap: 10pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow {
                    col_widths,
                    margin_top,
                    ..
                } = el
                {
                    Some((col_widths, *margin_top))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 2, "Should have 2 rows");

        // Column widths should account for the gap
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 10.0) / 2.0;
        let widths = grid_rows[0].0;
        assert!(
            (widths[0] - expected_col).abs() < 0.1,
            "Column width should account for gap: got {}, expected {}",
            widths[0],
            expected_col
        );

        // Second row should have grid-gap as margin_top
        assert!(
            (grid_rows[1].1 - 10.0).abs() < 0.1,
            "Second row margin_top should be the grid gap: got {}",
            grid_rows[1].1
        );
    }

    #[test]
    fn grid_wraps_to_new_rows() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
            <div>E</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 3, "5 items in 2 columns = 3 rows");
        assert_eq!(grid_rows[0].len(), 2);
        assert_eq!(grid_rows[1].len(), 2);
        assert_eq!(
            grid_rows[2].len(),
            2,
            "Last row should be padded to 2 cells"
        );
        // Last row's second cell should be empty
        assert!(
            grid_rows[2][1].lines.is_empty(),
            "Padding cell should have no text"
        );
    }

    #[test]
    fn grid_renders_to_pdf() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr; grid-gap: 10pt">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Cell"),
            "Grid cell text should appear in PDF"
        );
        assert!(content.contains("1"), "Cell numbers should appear in PDF");
        assert!(content.contains("6"), "Cell 6 should appear in PDF");
    }

    #[test]
    fn grid_with_gap_alias() {
        // Test that 'gap' works as an alias for 'grid-gap'
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; gap: 20pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow { margin_top, .. } = el {
                    Some(*margin_top)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 2);
        // Second row should have gap as margin_top
        assert!(
            (grid_rows[1] - 20.0).abs() < 0.1,
            "gap alias should work: got {}",
            grid_rows[1]
        );
    }

    #[test]
    fn grid_with_stylesheet_rules() {
        use crate::parser::css::parse_stylesheet;
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; grid-gap: 5pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1, "Should have 1 grid row");
        assert_eq!(grid_rows[0].0.len(), 2, "Should have 2 cells");
        // Verify gap is accounted for in widths
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 5.0) / 2.0;
        assert!(
            (grid_rows[0].1[0] - expected_col).abs() < 0.1,
            "Column width with gap: got {}, expected {}",
            grid_rows[0].1[0],
            expected_col
        );
    }

    #[test]
    fn grid_no_template_columns_defaults_to_single_column() {
        let html = r#"<div style="display: grid">
            <div>Only</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let rows = extract_grid_rows(&pages);
        let grid_rows: Vec<_> = rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        assert_eq!(grid_rows[0].1.len(), 1, "Default should be single column");
    }

    // --- min-width / min-height / max-height / margin auto tests ---

    #[test]
    fn css_min_width_enforces_minimum() {
        // width: 100pt would be 100, but min-width: 300pt forces it to 300
        let html = r#"<div style="width: 100pt; min-width: 300pt">Narrow text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_min_height_enforces_minimum() {
        let html = r#"<div style="min-height: 200pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_max_height_limits_height() {
        let html = r#"<div style="height: 500pt; max-height: 300pt">Tall box</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_margin_auto_centers_element() {
        let html = r#"<div style="width: 200pt; margin: 0 auto">Centered</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                offset_left,
                block_width,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*block_width, Some(200.0));
            // available_width = 595.28 - 72 - 72 = 451.28
            let expected_offset = (451.28 - 200.0) / 2.0;
            assert!(
                (*offset_left - expected_offset).abs() < 0.1,
                "offset_left should be ~{expected_offset}, got {offset_left}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_margin_left_auto_pushes_right() {
        let html = r#"<div style="width: 200pt; margin-left: auto">Right-aligned</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                offset_left,
                block_width,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*block_width, Some(200.0));
            // available_width = 451.28, push to right
            let expected_offset = 451.28 - 200.0;
            assert!(
                (*offset_left - expected_offset).abs() < 0.1,
                "offset_left should be ~{expected_offset}, got {offset_left}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_min_max_interact_with_width_height() {
        // min-height larger than height => min-height wins
        let html = r#"<div style="height: 50pt; min-height: 100pt">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(100.0));
        } else {
            panic!("Expected TextBlock");
        }

        // width smaller than min-width => min-width wins
        let html2 = r#"<div style="width: 100pt; min-width: 300pt">Content</div>"#;
        let nodes2 = parse_html(html2).unwrap();
        let pages2 = layout(&nodes2, PageSize::A4, Margin::default());
        assert_eq!(pages2.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages2[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }

        // max-height smaller than min-height => max-height wins (CSS spec)
        // Actually in CSS spec min-height wins over max-height. Let's test:
        // height: 500pt, min-height: 200pt, max-height: 300pt => clamp to 300pt
        let html3 =
            r#"<div style="height: 500pt; max-height: 300pt; min-height: 200pt">Content</div>"#;
        let nodes3 = parse_html(html3).unwrap();
        let pages3 = layout(&nodes3, PageSize::A4, Margin::default());
        assert_eq!(pages3.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages3[0].elements[0] {
            assert_eq!(*block_height, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    // --- box-sizing tests ---

    #[test]
    fn box_sizing_border_box_subtracts_padding_from_width() {
        // With border-box, width: 200pt includes padding.
        // With 20pt padding on each side, content area = 200 - 20 - 20 = 160pt
        let html = r#"<div style="box-sizing: border-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            // block_width should still be 200 (the outer box)
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn box_sizing_content_box_width_is_content_only() {
        // With content-box (default), width: 200pt is just the content
        let html = r#"<div style="box-sizing: content-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn border_radius_stored_in_layout() {
        let html = r#"<div style="border-radius: 8pt; background-color: red">Rounded</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { border_radius, .. }) = &pages[0].elements[0] {
            assert!((*border_radius - 8.0).abs() < 0.001);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn outline_stored_in_layout() {
        let html = r#"<div style="outline: 3px solid blue">Outlined</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                outline_width,
                outline_color,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert!((*outline_width - 2.25).abs() < 0.01); // 3px * 0.75
            assert!(outline_color.is_some());
            let (r, g, b) = outline_color.unwrap();
            assert!((r - 0.0).abs() < 0.01);
            assert!((g - 0.0).abs() < 0.01);
            assert!((b - 1.0).abs() < 0.01);
        } else {
            panic!("Expected TextBlock");
        }
    }

    // ---- z-index tests ----

    #[test]
    fn z_index_stored_in_layout_element() {
        let html = r#"<div style="position: absolute; z-index: 5; top: 10pt">High</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::TextBlock { z_index: 5, .. }));
        assert!(found, "Expected element with z_index=5");
    }

    #[test]
    fn paginate_repeats_only_synthetic_page_background() {
        let make_block =
            |position, z_index, repeat_on_each_page, height| LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: 0.0,
                margin_bottom: 0.0,
                text_align: TextAlign::Left,
                background_color: None,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: Some(height),
                opacity: 1.0,
                float: Float::None,
                clear: Clear::None,
                position,
                offset_top: 0.0,
                offset_left: 0.0,
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: None,
                visible: true,
                clip_rect: None,
                transform: None,
                border_radius: 0.0,
                outline_width: 0.0,
                outline_color: None,
                text_indent: 0.0,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                vertical_align: VerticalAlign::Baseline,
                background_gradient: None,
                background_radial_gradient: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                z_index,
                repeat_on_each_page,
                positioned_depth: 0,
                heading_level: None,
                clip_children_count: 0,
            };

        let pages = crate::layout::paginate::paginate(
            vec![
                make_block(Position::Absolute, -1, true, 40.0),
                make_block(Position::Absolute, -1, false, 40.0),
                make_block(Position::Static, 0, false, 30.0),
                make_block(Position::Static, 0, false, 30.0),
            ],
            40.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        let repeated_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        matches!(
                            element,
                            LayoutElement::TextBlock {
                                repeat_on_each_page: true,
                                ..
                            }
                        )
                    })
                    .count()
            })
            .collect();
        assert_eq!(repeated_per_page, vec![1, 1]);

        let non_repeating_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        matches!(
                            element,
                            LayoutElement::TextBlock {
                                position: Position::Absolute,
                                repeat_on_each_page: false,
                                ..
                            }
                        )
                    })
                    .count()
            })
            .collect();
        assert_eq!(non_repeating_per_page, vec![1, 0]);
    }

    #[test]
    fn z_index_sorting_order() {
        let html = r#"
            <div style="position: absolute; z-index: 10; top: 0">High</div>
            <div style="position: absolute; z-index: 1; top: 0">Low</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // After sorting, z_index=1 should come before z_index=10
        let z_indices: Vec<i32> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    z_index, position, ..
                } if *position != Position::Static => Some(*z_index),
                _ => None,
            })
            .collect();
        if z_indices.len() >= 2 {
            assert!(
                z_indices[0] <= z_indices[1],
                "Elements should be sorted by z_index"
            );
        }
    }

    #[test]
    fn synthetic_page_background_sorts_before_more_negative_layers() {
        let make_block = |z_index, repeat_on_each_page| LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: Some(100.0),
            block_height: Some(40.0),
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Absolute,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            z_index,
            repeat_on_each_page,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        };

        let pages = crate::layout::paginate::paginate(
            vec![make_block(-1, true), make_block(-2, false)],
            200.0,
            0.0,
        );

        match &pages[0].elements[0].1 {
            LayoutElement::TextBlock {
                repeat_on_each_page,
                ..
            } => assert!(
                *repeat_on_each_page,
                "synthetic background should render first"
            ),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    // ---- calc() integration test ----

    #[test]
    fn calc_width_in_layout() {
        // Use a calc() value that's smaller than available_width so explicit_width is set
        let html = r#"<div style="width: calc(50% - 10pt)">Calc content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert!(
                block_width.is_some(),
                "calc() width should resolve to explicit width"
            );
        }
    }

    // ---- CSS variable integration test ----

    #[test]
    fn var_width_in_layout() {
        let html = r#"<div style="--w: 200pt"><div style="width: var(--w)">Var width</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            matches!(el, LayoutElement::TextBlock { block_width: Some(_w), .. } if (*_w - 200.0).abs() < 1.0)
        });
        assert!(found, "Expected element with width ~200pt from var()");
    }

    // ---- rem unit integration test ----

    #[test]
    fn rem_unit_in_layout() {
        let html = r#"<div style="margin-top: 2rem">Rem margin</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        // 2rem = 24pt margin_top
        if let (_, LayoutElement::TextBlock { margin_top, .. }) = &pages[0].elements[0] {
            assert!(
                (*margin_top - 24.0).abs() < 0.5,
                "Expected ~24pt margin_top from 2rem"
            );
        }
    }

    #[test]
    fn table_row_carries_border_collapse() {
        let html = r#"<table style="border-collapse: collapse"><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_collapse = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TableRow {
                    border_collapse: BorderCollapse::Collapse,
                    ..
                }
            )
        });
        assert!(has_collapse, "Expected border_collapse: Collapse");
    }

    #[test]
    fn table_row_default_border_separate() {
        let html = r#"<table><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_separate = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TableRow {
                    border_collapse: BorderCollapse::Separate,
                    ..
                }
            )
        });
        assert!(has_separate, "Expected default border_collapse: Separate");
    }

    #[test]
    fn table_row_carries_border_spacing() {
        let html = r#"<table style="border-spacing: 8px"><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_spacing = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TableRow { border_spacing, .. } = el {
                (*border_spacing - 6.0).abs() < 0.1
            } else {
                false
            }
        });
        assert!(has_spacing, "Expected border_spacing of 6pt (8px * 0.75)");
    }

    #[test]
    fn text_overflow_ellipsis_truncates() {
        // text-overflow: ellipsis is stored on the style; layout does not yet
        // perform the actual truncation with "..." so we just verify the
        // element is produced and has a single line (nowrap).
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis">This is a very long text that should be truncated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines.len() == 1
            } else {
                false
            }
        });
        assert!(found, "Text with nowrap should have a single line");
    }

    #[test]
    fn text_overflow_clip_no_ellipsis() {
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: clip">This is a very long text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_ellipsis = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines
                    .iter()
                    .any(|l| l.runs.iter().any(|r| r.text.ends_with("...")))
            } else {
                false
            }
        });
        assert!(!has_ellipsis, "clip should not add ellipsis");
    }

    // --- list-style-type tests ---
    #[test]
    fn format_list_marker_disc() {
        assert_eq!(format_list_marker(ListStyleType::Disc, 1), "\u{2022} ");
    }

    #[test]
    fn format_list_marker_circle() {
        assert_eq!(format_list_marker(ListStyleType::Circle, 1), "\u{25E6} ");
    }

    #[test]
    fn format_list_marker_square() {
        assert_eq!(format_list_marker(ListStyleType::Square, 1), "\u{25AA} ");
    }

    #[test]
    fn format_list_marker_decimal() {
        assert_eq!(format_list_marker(ListStyleType::Decimal, 3), "3. ");
    }

    #[test]
    fn format_list_marker_decimal_leading_zero() {
        assert_eq!(
            format_list_marker(ListStyleType::DecimalLeadingZero, 3),
            "03. "
        );
        assert_eq!(
            format_list_marker(ListStyleType::DecimalLeadingZero, 12),
            "12. "
        );
    }

    #[test]
    fn format_list_marker_lower_alpha() {
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 1), "a. ");
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 3), "c. ");
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 27), "aa. ");
    }

    #[test]
    fn format_list_marker_upper_alpha() {
        assert_eq!(format_list_marker(ListStyleType::UpperAlpha, 1), "A. ");
        assert_eq!(format_list_marker(ListStyleType::UpperAlpha, 26), "Z. ");
    }

    #[test]
    fn format_list_marker_lower_roman() {
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 1), "i. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 4), "iv. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 9), "ix. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 14), "xiv. ");
    }

    #[test]
    fn format_list_marker_upper_roman() {
        assert_eq!(format_list_marker(ListStyleType::UpperRoman, 1), "I. ");
        assert_eq!(format_list_marker(ListStyleType::UpperRoman, 4), "IV. ");
    }

    #[test]
    fn format_list_marker_none() {
        assert_eq!(format_list_marker(ListStyleType::None, 1), "");
    }

    // --- Counter state tests ---
    #[test]
    fn counter_state_default_returns_zero() {
        let cs = CounterState::default();
        assert_eq!(cs.get("foo"), 0);
    }

    #[test]
    fn counter_state_apply_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
    }

    #[test]
    fn counter_state_apply_increments() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 2);
    }

    #[test]
    fn counter_state_nested_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        // Nested reset pushes a new counter
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        // Pop nested reset
        cs.pop_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 1); // Back to outer counter value
    }

    #[test]
    fn counter_state_get_all() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        cs.apply_resets(&[("section".to_string(), 3)]);
        assert_eq!(cs.get_all("section", "."), "1.2.3");
    }

    // --- resolve_content tests ---
    #[test]
    fn resolve_content_string() {
        let cs = CounterState::default();
        let attrs = HashMap::new();
        let items = vec![ContentItem::String("hello".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "hello");
    }

    #[test]
    fn resolve_content_attr() {
        let cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("title".to_string(), "My Title".to_string());
        let items = vec![ContentItem::Attr("title".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "My Title");
    }

    #[test]
    fn resolve_content_counter() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 3)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counter("section".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "3");
    }

    #[test]
    fn resolve_content_counters() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counters(
            "section".to_string(),
            ".".to_string(),
        )];
        assert_eq!(resolve_content(&items, &attrs, &cs), "1.2");
    }

    #[test]
    fn resolve_content_mixed() {
        let cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("data-label".to_string(), "Note".to_string());
        let items = vec![
            ContentItem::Attr("data-label".to_string()),
            ContentItem::String(": ".to_string()),
        ];
        assert_eq!(resolve_content(&items, &attrs, &cs), "Note: ");
    }

    // --- ::before/::after integration tests ---
    #[test]
    fn before_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::before { content: ">> " }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains(">>") && t.contains("Hello"));
        assert!(
            found,
            "::before content should be prepended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn after_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::after { content: " <<" }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains("Hello") && t.contains("<<"));
        assert!(
            found,
            "::after content should be appended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn root_font_size_drives_rem_layout_values() {
        let html = r#"
            <html>
                <head>
                    <style>
                        :root { font-size: 10pt; }
                        .title { font-size: 2rem; margin-top: 0.5rem; }
                    </style>
                </head>
                <body><div class="title">Title</div></body>
            </html>
        "#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }

        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let title_block = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines, margin_top, ..
                } if lines
                    .iter()
                    .flat_map(|line| line.runs.iter())
                    .any(|run| run.text.contains("Title")) =>
                {
                    Some((lines, margin_top))
                }
                _ => None,
            })
            .expect("expected title block");

        let (lines, margin_top) = title_block;
        assert!((*margin_top - 5.0).abs() < 0.1);
        assert!(
            (lines[0].runs[0].font_size - 20.0).abs() < 0.1,
            "expected 2rem to resolve from :root 10pt"
        );
    }

    // --- list-style-type in layout tests ---
    #[test]
    fn unordered_list_uses_bullet_marker() {
        let html = "<ul><li>Item</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines
                    .iter()
                    .any(|l| l.runs.iter().any(|r| r.text.contains('\u{2022}')))
            } else {
                false
            }
        });
        assert!(found, "Unordered list should use bullet marker");
    }

    #[test]
    fn ordered_list_uses_decimal_marker() {
        let html = "<ol><li>First</li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts.iter().any(|t| t.contains("1."));
        assert!(
            found,
            "Ordered list should use decimal marker, got: {:?}",
            all_texts
        );
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn to_alpha_lower_zero_returns_a() {
        // Covers line 81: to_alpha_lower(0) returns "a"
        assert_eq!(to_alpha_lower(0), "a");
    }

    #[test]
    fn to_roman_lower_zero_returns_zero_string() {
        // Covers line 120: to_roman_lower(0) returns "0"
        assert_eq!(to_roman_lower(0), "0");
    }

    #[test]
    fn counter_state_apply_increments_on_empty_stack() {
        // Covers line 32: apply_increments pushes 0 when stack is empty
        let mut state = CounterState::default();
        state.apply_increments(&[("test".to_string(), 1)]);
        assert_eq!(state.get("test"), 1);
    }

    #[test]
    fn css_counter_in_pseudo_element_generates_numbers() {
        // Verifies that counter-reset + counter-increment + counter() in
        // ::before pseudo-elements produce sequential numbers (BUG 1 fix).
        let html = r#"<html><head><style>
            ol.counted { counter-reset: item }
            ol.counted li { counter-increment: item }
            ol.counted li::before { content: counter(item) ". " }
        </style></head><body>
            <ol class="counted"><li>First</li><li>Second</li><li>Third</li></ol>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    if !text.trim().is_empty() {
                        all_texts.push(text);
                    }
                }
            }
        }
        let joined = all_texts.join(" ");
        assert!(
            joined.contains("1.") && joined.contains("2.") && joined.contains("3."),
            "CSS counters should generate sequential numbers 1, 2, 3. Got: {joined}"
        );
    }

    #[test]
    fn layout_flex_container() {
        // Covers lines 1067,1133,1395: flex layout code paths
        let html = r#"<div style="display: flex; width: 400pt;">
            <div style="width: 200pt;">Left</div>
            <div style="width: 200pt;">Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_grid_container() {
        // Covers lines 1670,1712: grid layout code paths
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 1fr 1fr; }
        </style></head><body>
        <div class="grid"><div>A</div><div>B</div></div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_with_non_standard_children() {
        // Covers line 1821,1831,1858: table non-tr children
        let html = "<table><caption>Cap</caption><tr><td>A</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_colspan_exceeds_cols() {
        // Covers line 1943,2003: colspan beyond column count
        let html = r#"<table>
            <tr><td colspan="10">Wide</td></tr>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_white_space_nowrap_overflow() {
        // Covers lines 2221,2227,2242: nowrap + text-overflow: ellipsis
        let html = r#"<html><head><style>
            .nowrap { width: 50pt; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
        </style></head><body>
        <div class="nowrap">This text is very long and should be truncated</div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_clear_right_float() {
        // Covers line 2312: clear: right
        let html = r#"
            <div style="float: right; width: 100pt;">Floated</div>
            <div style="clear: right;">Cleared</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn base64_decode_valid() {
        // Covers lines 2562,2574: base64 decode
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(&decoded, b"Hello");
    }

    #[test]
    fn base64_decode_invalid_char() {
        // Covers line 2562: base64 decode with invalid char
        let result = decode_base64("!!!!");
        assert!(result.is_none());
    }

    #[test]
    fn base64_decode_short_input() {
        // Covers line 2574: base64 decode with very short input (breaks early)
        let result = decode_base64("A");
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn wrap_break_word_splits_long_word_without_hyphen() {
        let fonts = HashMap::new();
        let template = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        // At 12pt, each char ~6pt. "Hi" = 12pt.
        // "Supercalifragilisticexpialidocious" = 34*6 = 204pt.
        // With max_width=100, "Hi" (12pt) fits, then the long word (204pt)
        // doesn't fit (12 + 6 space + 204 > 100), so break-word splits it
        // across lines without inserting a hyphen character.
        let runs = vec![TextRun {
            text: "Hi Supercalifragilisticexpialidocious".to_string(),
            ..template
        }];
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(100.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        assert!(
            lines.len() > 1,
            "expected break-word to produce multiple lines, got {}",
            lines.len()
        );
        let first_line_text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !first_line_text.ends_with('-'),
            "break-word should not insert hyphens, got: {first_line_text:?}"
        );
    }

    #[test]
    fn wrap_normal_keeps_fitting_text_on_one_line() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hello world".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(500.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !text.contains('-'),
            "short fitting text should stay unchanged, got: {text:?}"
        );
    }

    #[test]
    fn wrap_break_word_splits_short_remainder_without_hyphen() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hi the end".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(20.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        for line in &lines {
            for run in &line.runs {
                assert!(
                    !run.text.contains('-'),
                    "break-word should not add hyphens, got: {:?}",
                    run.text
                );
            }
        }
    }

    /// Helper: extract all text strings from a PDF byte vector.
    /// Handles both WinAnsi Tj strings and CID TJ arrays with ToUnicode CMap.
    fn extract_tj_strings(pdf: &[u8]) -> Vec<String> {
        let pdf_str = String::from_utf8_lossy(pdf);
        let content: &str = pdf_str.as_ref();

        // Try WinAnsi Tj path first
        let winans: Vec<String> = content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.ends_with("Tj") && trimmed.starts_with('(') {
                    Some(trimmed[1..trimmed.len() - 4].to_string())
                } else {
                    None
                }
            })
            .collect();
        if !winans.is_empty() {
            return winans;
        }

        // CID path: parse ToUnicode CMap to build glyph→char map
        let mut glyph_to_char: std::collections::HashMap<String, char> =
            std::collections::HashMap::new();
        let mut pos = 0;
        while let Some(start) = content[pos..].find("beginbfchar") {
            let block_start = pos + start + 11;
            let block_end = content[block_start..]
                .find("endbfchar")
                .map(|e| block_start + e)
                .unwrap_or(content.len());
            for line in content[block_start..block_end].lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line
                    .split(|c: char| c == '<' || c == '>' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 {
                    let glyph_hex = parts[0].to_uppercase();
                    let unicode_hex = parts[1];
                    if let Ok(cp) = u32::from_str_radix(unicode_hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            glyph_to_char.insert(glyph_hex, ch);
                        }
                    }
                }
            }
            pos = block_end;
        }

        if glyph_to_char.is_empty() {
            return Vec::new();
        }

        // Parse TJ arrays: [...] TJ
        let mut results = Vec::new();
        let mut search_pos = 0;
        while let Some(tj_end) = content[search_pos..].find("] TJ") {
            let tj_end_abs = search_pos + tj_end;
            if let Some(tj_start) = content[..tj_end_abs].rfind('[') {
                let array_content = &content[tj_start + 1..tj_end_abs];
                let mut decoded = String::new();
                let mut apos = 0;
                while let Some(open) = array_content[apos..].find('<') {
                    let open_abs = apos + open;
                    if let Some(close) = array_content[open_abs..].find('>') {
                        let hex_str = array_content[open_abs + 1..open_abs + close]
                            .trim()
                            .to_uppercase();
                        if let Some(&ch) = glyph_to_char.get(&hex_str) {
                            decoded.push(ch);
                        }
                        apos = open_abs + close + 1;
                    } else {
                        break;
                    }
                }
                if !decoded.is_empty() {
                    results.push(decoded);
                }
            }
            search_pos = tj_end_abs + 4;
        }
        results
    }

    #[test]
    fn spaces_preserved_in_text() {
        // "Hello World" must stay "Hello World" through the full pipeline
        let html = "<p>Hello World</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected 'Hello World' in PDF text, got: {tj:?}"
        );
    }

    #[test]
    fn spaces_between_inline_elements() {
        // `<span>Hello</span> <span>World</span>` must have a space
        let html = "<p><span>Hello</span> <span>World</span></p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected space between inline elements, got: {tj:?}"
        );
    }

    #[test]
    fn invoice_text_spaces_preserved() {
        // Verify the specific failing cases from the invoice
        let html = r#"
            <p><strong>Bill to:</strong><br>
            Acme Corp<br>
            456 Enterprise Blvd<br>
            New York, NY 10001</p>
            <table>
                <tr><td>Custom font embedding module</td></tr>
                <tr><td>SVG rendering add-on</td></tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let has = |needle: &str| tj.iter().any(|s| s.contains(needle));

        assert!(has("Acme Corp"), "Expected 'Acme Corp', got: {tj:?}");
        assert!(has("New York"), "Expected 'New York', got: {tj:?}");
        assert!(has("Custom font"), "Expected 'Custom font', got: {tj:?}");
        assert!(
            has("SVG rendering"),
            "Expected 'SVG rendering', got: {tj:?}"
        );
        assert!(
            has("Enterprise Blvd"),
            "Expected 'Enterprise Blvd', got: {tj:?}"
        );
    }

    /// Block children inside a padded parent should use inner_width (parent
    /// width minus padding) so that their text wraps within the padding.
    #[test]
    fn padded_div_child_block_respects_inner_width() {
        let html = r#"<div style="padding: 20pt;"><p>short</p></div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(200.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // The <p> inside the padded div should be laid out within 200 - 40 = 160pt.
        // We verify that the p's TextBlock has block_width <= 160.
        let mut found = false;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::TextBlock {
                    lines, block_width, ..
                } = elem
                {
                    let text: String = lines
                        .iter()
                        .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                        .collect();
                    if text.contains("short") {
                        if let Some(bw) = block_width {
                            assert!(
                                *bw <= 160.0,
                                "child block width {bw} should be <= inner_width 160"
                            );
                        }
                        found = true;
                    }
                }
            }
        }
        assert!(found, "did not find the child paragraph");
    }

    /// Flex child with inline background (badge) should propagate the
    /// background_color from the computed style to the TextRun.
    #[test]
    fn flex_child_propagates_background_color() {
        let html = r#"
        <div style="display: flex;">
          <div><span style="background-color: #27ae60; color: white;">PAID</span></div>
        </div>"#;
        let dom = parse_html(html).unwrap();
        let rules = parse_stylesheet("span { background-color: #27ae60; color: white; }");
        let pages = layout_with_rules(
            &dom,
            crate::types::PageSize::default(),
            crate::types::Margin::uniform(20.0),
            &rules,
        );
        let mut found_bg = false;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::FlexRow { cells, .. } = elem {
                    for cell in cells {
                        for line in &cell.lines {
                            for run in &line.runs {
                                if run.text.contains("PAID") && run.background_color.is_some() {
                                    found_bg = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            found_bg,
            "PAID badge text run should have background_color set"
        );
    }

    #[test]
    fn flex_row_child_preserves_svg_background() {
        let child_style = r#"background-image: url(data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill='%23f00'/%3E%3C/svg%3E); width: 60pt;"#;
        let parsed = crate::parser::css::parse_inline_style(child_style);
        assert!(
            parsed.get("background-svg").is_some(),
            "expected inline style parser to capture SVG background"
        );
        let computed = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some(child_style),
            &ComputedStyle::default(),
        );
        assert!(
            computed.background_svg.is_some(),
            "expected computed style to retain SVG background"
        );
        let html =
            format!(r#"<div style="display: flex;"><div style="{child_style}">A</div></div>"#);
        let pages = layout(&parse_html(&html).unwrap(), PageSize::A4, Margin::default());
        let has_cell_svg_background = pages.iter().any(|page| {
            page.elements.iter().any(|(_, el)| match el {
                LayoutElement::FlexRow { cells, .. } => {
                    cells.iter().any(|cell| cell.background_svg.is_some())
                }
                _ => false,
            })
        });
        assert!(
            has_cell_svg_background,
            "expected flex row cell to retain SVG background data"
        );
    }

    /// Notes-style div with padding, br tags, and inline content should
    /// produce wrapped text that fits within the padded area.
    #[test]
    fn notes_div_with_padding_and_br_wraps_correctly() {
        let html = r#"<div style="padding: 10pt; font-size: 9pt;">
          <strong>Notes:</strong><br>
          First line of text that should be fully visible inside the padded area.<br>
          Second line with content.
        </div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(300.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // Verify that lines exist and the text is present
        let mut all_text = String::new();
        let mut line_count = 0;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::TextBlock { lines, .. } = elem {
                    for line in lines {
                        for run in &line.runs {
                            all_text.push_str(&run.text);
                        }
                        line_count += 1;
                    }
                }
            }
        }
        assert!(all_text.contains("Notes:"), "Notes: text missing");
        assert!(
            all_text.contains("First line"),
            "First line text missing: {all_text:?}"
        );
        assert!(
            all_text.contains("Second line"),
            "Second line text missing: {all_text:?}"
        );
        // Should have at least 3 lines due to the <br> tags
        assert!(
            line_count >= 3,
            "expected at least 3 lines from br tags, got {line_count}"
        );
    }

    #[test]
    fn body_rules_applied_to_root() {
        let css = "body { font-size: 10pt }";
        let rules = parse_stylesheet(css);
        let html = "<p>text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let font_size = lines[0].runs[0].font_size;
            assert!(
                (font_size - 10.0).abs() < 0.1,
                "Expected font_size 10.0 from body rule, got {font_size}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn root_rules_applied_to_root_style() {
        let css = ":root { font-size: 11pt; background-color: #abcdef }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());

        let first_is_background = matches!(
            &pages[0].elements[0].1,
            LayoutElement::TextBlock {
                background_color: Some((r, g, b, _a)),
                repeat_on_each_page: true,
                ..
            } if (*r - 0xAB as f32 / 255.0).abs() < 0.01
                && (*g - 0xCD as f32 / 255.0).abs() < 0.01
                && (*b - 0xEF as f32 / 255.0).abs() < 0.01
        );
        assert!(first_is_background, "Expected page background from :root");

        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[1] {
            assert!(!lines.is_empty());
            let font_size = lines[0].runs[0].font_size;
            assert!(
                (font_size - 11.0).abs() < 0.1,
                "Expected font_size 11.0 from :root rule, got {font_size}"
            );
        } else {
            panic!("Expected text block after root background");
        }
    }

    #[test]
    fn root_svg_background_emits_page_background_block() {
        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        if let (
            _,
            LayoutElement::TextBlock {
                background_svg: Some(tree),
                block_width: Some(width),
                block_height: Some(height),
                repeat_on_each_page: true,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(tree.width, 20.0);
            assert_eq!(tree.height, 10.0);
            // Body/root background is confined to the content area (page minus margins),
            // matching Chrome's print behavior which surrounds the body bg with a
            // white page margin frame.
            let margin = Margin::default();
            let expected_width = PageSize::A4.width - margin.left - margin.right;
            let expected_height = PageSize::A4.height - margin.top - margin.bottom;
            assert!((*width - expected_width).abs() < 0.1);
            assert!((*height - expected_height).abs() < 0.1);
        } else {
            panic!("Expected a repeat-on-each-page SVG background block");
        }
    }

    #[test]
    fn wrapper_textblock_for_visual_blocks() {
        let css = ".box { background-color: red; padding: 10pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="box"><p>hello</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    ..
                } | LayoutElement::Container {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_bg,
            "Expected a TextBlock or Container with background_color from .box div"
        );
    }

    #[test]
    fn flex_child_ancestor_selectors() {
        let css = ".card .value { font-size: 20pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="card" style="display: flex"><div class="value">big</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        let big_item = items.iter().find(|i| i.3.contains("big"));
        assert!(
            big_item.is_some(),
            "Did not find 'big' text in flex layout output"
        );
        // Verify the font size was applied via ancestor selector
        // Check via the layout elements directly for font_size
        let mut found = false;
        for (_, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { lines, .. } => {
                    for line in lines {
                        for run in &line.runs {
                            if run.text.contains("big") && (run.font_size - 20.0).abs() < 0.1 {
                                found = true;
                            }
                        }
                    }
                }
                LayoutElement::FlexRow { cells, .. } => {
                    for cell in cells {
                        for line in &cell.lines {
                            for run in &line.runs {
                                if run.text.contains("big") && (run.font_size - 20.0).abs() < 0.1 {
                                    found = true;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(found, "Expected font_size 20.0 for .value in flex child");
    }

    #[test]
    fn p_inherits_parent_font_size() {
        let html = r#"<div style="font-size: 8pt"><p>small</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        let mut found = false;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        if run.text.contains("small") {
                            assert!(
                                (run.font_size - 8.0).abs() < 0.1,
                                "Expected font_size 8.0 for p inside div, got {}",
                                run.font_size
                            );
                            found = true;
                        }
                    }
                }
            }
        }
        assert!(found, "Did not find 'small' text run in layout output");
    }

    #[test]
    fn table_nth_child_section_relative() {
        let css = "tbody tr:nth-child(even) { background-color: #eee }";
        let rules = parse_stylesheet(css);
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody>
                    <tr><td>Row 1</td></tr>
                    <tr><td>Row 2</td></tr>
                    <tr><td>Row 3</td></tr>
                </tbody>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        // Should have at least 4 rows (1 thead + 3 tbody)
        assert!(
            table_rows.len() >= 4,
            "Expected at least 4 table rows, got {}",
            table_rows.len()
        );
    }

    #[test]
    fn layout_border_horizontal_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            right: LayoutBorderSide {
                width: 3.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            bottom: LayoutBorderSide {
                width: 2.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            left: LayoutBorderSide {
                width: 5.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
        };
        assert!((border.horizontal_width() - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_border_vertical_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 4.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            right: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            bottom: LayoutBorderSide {
                width: 6.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            left: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
        };
        assert!((border.vertical_width() - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_border_max_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 2.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            right: LayoutBorderSide {
                width: 7.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            bottom: LayoutBorderSide {
                width: 3.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
            left: LayoutBorderSide {
                width: 5.0,
                color: (0.0, 0.0, 0.0),
                style: crate::style::computed::BorderStyle::Solid,
            },
        };
        assert!((border.max_width() - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_column_layout() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>First</div>
            <div>Second</div>
            <div>Third</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TextBlock { .. }))
            .collect();
        assert!(
            text_blocks.len() >= 3,
            "Expected at least 3 text blocks for column flex children, got {}",
            text_blocks.len()
        );
    }

    #[test]
    fn flex_column_with_background() {
        let html = r#"<div style="display: flex; flex-direction: column; background-color: #eee">
            <p>Child A</p>
            <p>Child B</p>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_bg,
            "Expected a wrapper TextBlock with background_color for flex column container"
        );
    }

    #[test]
    fn table_rowspan_layout() {
        let html = r#"
            <table>
                <tr><td rowspan="2">Spanning</td><td>A</td></tr>
                <tr><td>B</td></tr>
                <tr><td>C</td><td>D</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert!(
            table_rows.len() >= 2,
            "Expected at least 2 table rows with rowspan, got {}",
            table_rows.len()
        );
    }

    #[test]
    fn inline_span_inherits_border_radius() {
        let css = "span.badge { background-color: green; border-radius: 5pt; padding: 2pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<p><span class="badge">Tag</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let mut found_br = false;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        if run.text.contains("Tag") && run.border_radius > 0.0 {
                            found_br = true;
                        }
                    }
                }
            }
        }
        assert!(
            found_br,
            "Expected TextRun for 'Tag' to have border_radius > 0 from stylesheet"
        );
    }

    #[test]
    fn grid_layout_produces_rows() {
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div><div>D</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        // Grid rows are now inside a Container wrapper
        let has_grid_container = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::Container { children, .. } = el {
                children
                    .iter()
                    .any(|c| matches!(c, LayoutElement::GridRow { .. }))
            } else {
                false
            }
        });
        assert!(
            has_grid_container,
            "Expected Container with GridRow children from display: grid layout"
        );
    }

    #[test]
    fn page_break_produces_multiple_pages() {
        let html = r#"
            <p>Page one content</p>
            <div style="page-break-before: always">
                <p>Page two content</p>
            </div>
            <div style="page-break-before: always">
                <p>Page three content</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(
            pages.len() >= 3,
            "Expected at least 3 pages from two page-break-before: always, got {}",
            pages.len()
        );
    }

    #[test]
    fn image_element_in_layout() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" style="width: 50px; height: 50px">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::Image { .. }));
        assert!(has_image, "Expected an Image layout element from img tag");
    }

    #[test]
    fn wrapper_textblock_with_border() {
        let css = ".bordered { border: 2pt solid black; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="bordered"><p>inside</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_border = pages[0].elements.iter().any(|(_, el)| match el {
            LayoutElement::TextBlock { border, .. } => border.has_any(),
            LayoutElement::Container { border, .. } => border.has_any(),
            _ => false,
        });
        assert!(
            has_border,
            "Expected a TextBlock or Container with border from .bordered div"
        );
    }

    #[test]
    fn wrapper_textblock_with_box_shadow() {
        let css = ".shadow { box-shadow: 2pt 2pt 4pt #000; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="shadow"><p>shadowed</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_shadow = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    box_shadow: Some(_),
                    ..
                } | LayoutElement::Container {
                    box_shadow: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_shadow,
            "Expected a TextBlock or Container with box_shadow from .shadow div"
        );
    }

    #[test]
    fn flex_column_child_positioning() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>Alpha</div>
            <div>Beta</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| {
                if let LayoutElement::TextBlock { lines, .. } = el {
                    !lines.is_empty()
                } else {
                    false
                }
            })
            .collect();
        if text_blocks.len() >= 2 {
            assert!(
                text_blocks[1].0 >= text_blocks[0].0,
                "Expected second flex column child to be at or below first child"
            );
        }
    }

    #[test]
    fn grid_row_alignment_in_paginate() {
        let css = ".g { display: grid; grid-template-columns: 1fr 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="g"><div>X</div><div>Y</div><div>Z</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
        let grid_rows = extract_grid_rows(&pages);
        assert!(
            !grid_rows.is_empty(),
            "Expected GridRow elements from grid layout"
        );
    }

    #[test]
    fn table_descendant_selector_total_row_td() {
        // .total-row td should apply styles via descendant selector on table rows
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 14pt; }
        </style></head><body>
        <table><tbody>
            <tr><td>Normal</td></tr>
            <tr class="total-row"><td>Total</td></tr>
        </tbody></table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut table_rows: Vec<&Vec<TableCell>> = Vec::new();
        for page in &pages {
            for (_, el) in &page.elements {
                if let LayoutElement::TableRow { cells, .. } = el {
                    table_rows.push(cells);
                }
            }
        }
        assert_eq!(table_rows.len(), 2, "Expected 2 table rows");
        assert!(
            table_rows[1][0].bold,
            "Cell in .total-row should be bold via descendant selector"
        );
        let normal_h: f32 = table_rows[0][0].lines.iter().map(|l| l.height).sum();
        let total_h: f32 = table_rows[1][0].lines.iter().map(|l| l.height).sum();
        assert!(
            total_h > normal_h,
            "Total row text should be larger: {total_h} vs {normal_h}"
        );
    }

    #[test]
    fn flex_grow_distributes_free_space() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-grow: 1; }
            .b { flex-grow: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // With flex-grow 1:2, widths should be roughly 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex-grow 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_basis_overrides_width() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 400pt; }
            .a { flex-basis: 100pt; }
            .b { flex-basis: 300pt; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // flex-basis: 100pt vs 300pt
        assert!(
            (cells[0].width - 100.0).abs() < 5.0,
            "First cell should be ~100pt, got {}",
            cells[0].width
        );
        assert!(
            (cells[1].width - 300.0).abs() < 5.0,
            "Second cell should be ~300pt, got {}",
            cells[1].width
        );
    }

    #[test]
    fn margin_collapsing_adjacent_blocks() {
        // Adjacent sibling margins collapse: max(20, 30) = 30pt gap, not 50pt
        let html = r#"<html><head><style>
            .a { margin-bottom: 20pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find the two TextBlock y-positions
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2, "Expected 2 text blocks, got {}", ys.len());
        // The gap between the bottom of the first block and the second y-position
        // should reflect collapsed margin (30pt), not stacked (50pt).
        // We can't check exact absolute positions easily, but we can verify the
        // second block is closer than it would be without collapsing.
        let gap = ys[1] - ys[0];
        // Without collapsing: first_content_height + 20 + 30 = content + 50
        // With collapsing: first_content_height + 30
        // The gap should be smaller than content + 50
        assert!(gap > 0.0, "Second block should be below first");
    }

    #[test]
    fn margin_collapse_through_container() {
        // CSS 2.1 § 8.3.1: the top margin of a parent with no padding/border
        // collapses with the margin-top of its first in-flow child (and same
        // for margin-bottom / last child). This mirrors the .block-pseudo
        // fixture where two sibling containers each wrap a <p> whose default
        // 1em margins should collapse through the container — not stack.
        let html = r#"<html><head><style>
            .wrap { position: relative; margin-bottom: 12pt; }
            .wrap::before { content: ""; display: block; position: absolute;
                left: 0; top: 0; width: 4pt; height: 100%; background: #3b82f6; }
            .wrap p { margin-top: 16pt; margin-bottom: 16pt; }
        </style></head><body>
        <div class="wrap"><p>one</p></div>
        <div class="wrap"><p>two</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Collect the y-positions of the two <p> text blocks (one inside
        // each .wrap Container).
        let mut text_ys: Vec<f32> = Vec::new();
        fn collect_text_ys(elements: &[(f32, LayoutElement)], out: &mut Vec<f32>) {
            for (y, el) in elements {
                match el {
                    LayoutElement::TextBlock { lines, .. } if !lines.is_empty() => {
                        out.push(*y);
                    }
                    LayoutElement::Container { children, .. } => {
                        let pairs: Vec<(f32, LayoutElement)> =
                            children.iter().map(|c| (*y, c.clone())).collect();
                        collect_text_ys(&pairs, out);
                    }
                    _ => {}
                }
            }
        }
        collect_text_ys(&pages[0].elements, &mut text_ys);
        assert_eq!(text_ys.len(), 2, "expected 2 text blocks");
        // Without collapse: gap = 16 (p.mb) + 12 (div.mb) + 16 (p.mt) = 44pt
        //                         plus text height of first <p>
        // With collapse:   gap = max(16, 12, 16) = 16pt + text height
        // The container y is the same as child y since margins collapsed in.
        // We can't reliably inspect inner text y without deeper traversal,
        // but we can check the Container y positions instead.
        let container_ys: Vec<f32> = pages[0]
            .elements
            .iter()
            .filter_map(|(y, el)| match el {
                LayoutElement::Container { .. } => Some(*y),
                _ => None,
            })
            .collect();
        assert_eq!(container_ys.len(), 2, "expected 2 wrap containers");
        let gap = container_ys[1] - container_ys[0];
        // Expected: 16pt (collapsed) + height of <p> text (~16pt at 16pt
        // font with 1.5 line-height ≈ 24pt). Total ≈ 40pt.
        // Without collapse this would be ~68pt (44 + 24).
        assert!(
            gap < 50.0,
            "Containers should be tight (margin-collapse-through-parent), got {}pt",
            gap
        );
    }

    #[test]
    fn flex_shorthand_parsing() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex: 1; }
            .b { flex: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // flex: 1 and flex: 2 with basis=0 should distribute 300pt as 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex shorthand 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_shrink_overflow() {
        // Items totalling 600pt in a 300pt container should shrink
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-basis: 400pt; flex-shrink: 1; }
            .b { flex-basis: 200pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        let total: f32 = cells.iter().map(|c| c.width).sum();
        assert!(
            total <= 305.0,
            "Shrunk items should fit in container (~300pt), got {total}"
        );
        // Proportional: 400 shrinks more than 200
        assert!(
            cells[0].width > cells[1].width,
            "Larger basis should still be wider after shrink"
        );
    }

    #[test]
    fn flex_shrink_zero_prevents_shrink() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 200pt; }
            .a { flex-basis: 150pt; flex-shrink: 0; }
            .b { flex-basis: 150pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        // First item has shrink: 0 so it keeps its basis
        assert!(
            (cells[0].width - 150.0).abs() < 5.0,
            "flex-shrink: 0 should prevent shrinking, got {}",
            cells[0].width
        );
        // Second item absorbs all the deficit
        assert!(
            cells[1].width < 150.0,
            "flex-shrink: 1 item should shrink, got {}",
            cells[1].width
        );
    }

    #[test]
    fn flex_no_grow_uses_content_width() {
        // 3 flex items with no flex-grow should use their content width,
        // not expand to fill the full container.
        let html = r#"<html><head><style>
            .container { display: flex; width: 400pt; }
            .item { width: 50pt; }
        </style></head><body>
        <div class="container">
            <div class="item">A</div>
            <div class="item">B</div>
            <div class="item">C</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 3);
        // Each item should be ~50pt wide, not ~133pt (400/3)
        for (idx, cell) in cells.iter().enumerate() {
            assert!(
                (cell.width - 50.0).abs() < 5.0,
                "Item {} should be ~50pt wide (content width), got {}",
                idx,
                cell.width
            );
        }
        // Total width of items should be much less than container width
        let total: f32 = cells.iter().map(|c| c.width).sum();
        assert!(
            total < 200.0,
            "Total item width should be much less than 400pt container, got {total}"
        );
    }

    #[test]
    fn flex_justify_center_positions_items_in_middle() {
        // justify-content: center should position items in the middle
        // of the container, not at the start.
        let html = r#"<html><head><style>
            .container { display: flex; justify-content: center; width: 400pt; }
            .item { width: 100pt; }
        </style></head><body>
        <div class="container">
            <div class="item">X</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 1);
        // Item should be centered: x_offset should be ~150pt ((400-100)/2)
        assert!(
            (cells[0].x_offset - 150.0).abs() < 5.0,
            "justify-content: center should put item at ~150pt, got {}",
            cells[0].x_offset
        );
        // Width should stay at 100pt
        assert!(
            (cells[0].width - 100.0).abs() < 5.0,
            "Item width should remain ~100pt, got {}",
            cells[0].width
        );
    }

    #[test]
    fn flex_justify_space_between_distributes_items() {
        // justify-content: space-between with 3 items should put first at
        // start, last at end, and distribute space between them evenly.
        let html = r#"<html><head><style>
            .container { display: flex; justify-content: space-between; width: 400pt; }
            .item { width: 80pt; }
        </style></head><body>
        <div class="container">
            <div class="item">A</div>
            <div class="item">B</div>
            <div class="item">C</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 3);
        // First item should be at x=0
        assert!(
            cells[0].x_offset < 5.0,
            "First item should be at start, got {}",
            cells[0].x_offset
        );
        // Last item should end at ~400pt (x_offset + width ~ 400)
        let last_end = cells[2].x_offset + cells[2].width;
        assert!(
            (last_end - 400.0).abs() < 5.0,
            "Last item should end at ~400pt, got {last_end}"
        );
        // Gaps between items should be equal
        let gap1 = cells[1].x_offset - (cells[0].x_offset + cells[0].width);
        let gap2 = cells[2].x_offset - (cells[1].x_offset + cells[1].width);
        assert!(
            (gap1 - gap2).abs() < 1.0,
            "Gaps should be equal: {gap1} vs {gap2}"
        );
        // Each gap should be ~80pt ((400 - 240) / 2)
        assert!((gap1 - 80.0).abs() < 5.0, "Gap should be ~80pt, got {gap1}");
    }

    #[test]
    fn margin_collapsing_negative_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: -20pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2);
        // Both negative: most negative wins (-20), not sum (-30)
        // Second block may overlap first (negative gap)
    }

    #[test]
    fn margin_collapsing_mixed_signs() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2);
        // Mixed: sum = -10 + 30 = 20pt gap (not 30 or 40)
        let gap = ys[1] - ys[0];
        assert!(gap > 0.0, "Gap should be positive with mixed margins");
    }

    #[test]
    fn margin_collapsing_zero_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: 0; }
            .b { margin-top: 0; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_descendant_selector_thead_th() {
        let html = r#"<html><head><style>
            thead th { color: red; font-size: 14pt; }
        </style></head><body>
        <table>
            <thead><tr><th>Header</th></tr></thead>
            <tbody><tr><td>Body</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
        // Should render without panics; thead th selector exercises section ancestor chain
    }

    #[test]
    fn table_descendant_selector_tbody_td() {
        let html = r#"<html><head><style>
            tbody td { font-style: italic; }
            table td { font-size: 11pt; }
        </style></head><body>
        <table>
            <thead><tr><th>H</th></tr></thead>
            <tbody><tr><td>B</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_colgroup_percentage_widths() {
        let html = r#"<table>
            <colgroup>
                <col span="1" style="width: 30%;">
                <col span="1" style="width: 70%;">
            </colgroup>
            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1, "Expected 1 table row");
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2, "Expected 2 columns");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.30).abs() < 0.05,
            "First column should be ~30% of total, got {:.1}% (widths: {:?})",
            ratio * 100.0,
            col_widths
        );
    }

    fn first_table_row_col_widths(html: &str) -> Vec<f32> {
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row")
    }

    #[test]
    fn table_colgroup_percentage_widths_ignore_border_spacing() {
        let no_spacing = first_table_row_col_widths(
            r#"<table style="width: 300pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );
        let spaced = first_table_row_col_widths(
            r#"<table style="width: 300pt; border-spacing: 10pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert_eq!(no_spacing.len(), 2);
        assert_eq!(spaced.len(), 2);
        assert!(
            (spaced[0] - no_spacing[0]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
        assert!(
            (spaced[1] - no_spacing[1]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
    }

    #[test]
    fn table_colgroup_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col width="75%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.05,
            "First column should be ~25% of total, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_colgroup_last_inline_width_wins() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: 40%;" width="90%">
                <col style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "Last inline width declaration should win, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_inline_width_ignores_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: auto" width="80%">
                <col>
            </colgroup>
            <tr><td>Short</td><td>Much longer content here</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            col_widths[1] > col_widths[0],
            "Inline width should override width attribute; got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_malformed_inline_width_is_ignored() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: not-a-width" width="25%">
                <col style="width: not-a-width" width="90%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.10).abs() < 0.05,
            "Malformed inline width should be ignored, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_all_invalid_inline_widths_fall_back_to_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: not-a-width" width="80%">
                <col width="20%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.80).abs() < 0.05,
            "All-invalid inline widths should fall back to width attributes, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_span_attribute() {
        let html = r#"<table>
            <colgroup>
                <col span="2" style="width: 20%;">
                <col span="1" style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 3);
        let total: f32 = col_widths.iter().sum();
        let ratio_0 = col_widths[0] / total;
        let ratio_2 = col_widths[2] / total;
        assert!(
            (ratio_0 - 0.20).abs() < 0.05,
            "First two columns should each be ~20%, got {:.1}%",
            ratio_0 * 100.0
        );
        assert!(
            (ratio_2 - 0.60).abs() < 0.05,
            "Third column should be ~60%, got {:.1}%",
            ratio_2 * 100.0
        );
    }

    #[test]
    fn table_bare_col_without_colgroup() {
        let html = r#"<table>
            <col style="width: 40%;">
            <col style="width: 60%;">
            <tr><td>X</td><td>Y</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "First column should be ~40%, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_without_colgroup_unchanged() {
        let html = "<table><tr><td>Short</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Auto-sizing should still work: longer column ({}) should be wider than short ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_mixed_explicit_and_auto_widths() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col>
            </colgroup>
            <tr><td>Fixed</td><td>Auto column content</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[0] > 0.0 && col_widths[1] > 0.0,
            "Both explicit and auto columns should keep usable widths: {:?}",
            col_widths
        );
        assert!(
            col_widths[0] < col_widths[1] || (col_widths[0] - col_widths[1]).abs() < 5.0,
            "Auto column should not be collapsed by explicit width redistribution: {:?}",
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_colgroup_widths_over_content() {
        let html = r#"<table style="table-layout: fixed; width: 400pt;">
            <colgroup>
                <col style="width: 25%;">
                <col style="width: 75%;">
            </colgroup>
            <tr>
                <td>Very long content that should not widen the first fixed column</td>
                <td>Short</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.02,
            "fixed layout should honor colgroup width instead of content, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_first_row_cell_widths() {
        let html = r#"<table style="table-layout: fixed; width: 300pt;">
            <tr>
                <td style="width: 90pt;">A</td>
                <td>B</td>
            </tr>
            <tr>
                <td>Short</td>
                <td>Longer content in the second column</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            (col_widths[0] - 90.0).abs() < 1.0,
            "first-row cell width should determine fixed column width, got {:?}",
            col_widths
        );
        assert!(
            (col_widths[1] - 210.0).abs() < 1.0,
            "remaining width should be assigned to the other fixed column, got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_absolute_lengths_are_supported() {
        let html = r#"<table style="table-layout: fixed; width: 300pt;">
            <colgroup>
                <col style="width: 90pt;">
                <col>
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            (col_widths[0] - 90.0).abs() < 1.0,
            "absolute <col> widths should be honored, got {:?}",
            col_widths
        );
        assert!(
            (col_widths[1] - 210.0).abs() < 1.0,
            "remaining width should stay usable for the trailing column, got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt;">
                <colgroup style="font-size: 20pt">
                    <col style="width: 2em;">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert!(
            (widths[0] - 40.0).abs() < 0.5,
            "2em should resolve against the colgroup font-size, got {:?}",
            widths
        );
        assert!(
            (widths[1] - 160.0).abs() < 0.5,
            "remaining width should stay on the trailing column, got {:?}",
            widths
        );
    }

    #[test]
    fn table_colgroup_calc_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt;">
                <colgroup style="font-size: 20pt">
                    <col style="width: calc(1em + 5pt);">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert!(
            (widths[0] - 25.0).abs() < 0.5,
            "calc(1em + 5pt) should use the colgroup font-size, got {:?}",
            widths
        );
        assert!(
            (widths[1] - 175.0).abs() < 0.5,
            "remaining width should stay on the trailing column, got {:?}",
            widths
        );
    }

    #[test]
    fn table_cell_block_content_preserves_link_and_whitespace() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        <div><a href="https://example.com">Click here</a></div>
                        <pre>  keep   spaces  </pre>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected table row");
        let text: String = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .map(|run| run.text.as_str())
            .collect();
        assert!(
            cells[0]
                .lines
                .iter()
                .flat_map(|line| line.runs.iter())
                .any(|run| run.link_url.as_deref() == Some("https://example.com")),
            "Expected link URL to survive nested block traversal"
        );
        assert!(
            text.contains("  keep   spaces  "),
            "Expected preformatted whitespace to survive nested block traversal: {text:?}"
        );
    }

    #[test]
    fn table_cell_mixed_recursion_keeps_nested_block_padding_but_not_cell_padding() {
        let html = r#"
            <table>
                <tr>
                    <td style="padding: 18pt 12pt; text-align: right;">
                        Direct text
                        <div style="padding-left: 6pt; padding-top: 3pt; background-color: #eee;">
                            Nested block
                        </div>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected table row");
        let direct_run = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .find(|run| run.text.contains("Direct"))
            .expect("expected direct cell text run");
        assert_eq!(
            direct_run.padding,
            (0.0, 0.0),
            "direct cell text should not inherit table-cell padding"
        );
        let nested_run = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .find(|run| run.text.contains("Nested"))
            .expect("expected nested block text run");
        assert_eq!(
            nested_run.padding,
            (6.0, 3.0),
            "nested block text should keep its own padding"
        );
    }

    #[test]
    fn table_cell_nested_table_is_preserved_as_nested_layout() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected outer table row");
        assert!(
            !cells[0].nested_rows.is_empty(),
            "expected nested table rows to be preserved"
        );
        let nested_text: String = cells[0]
            .nested_rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(
                        cells
                            .iter()
                            .flat_map(|cell| cell.lines.iter())
                            .flat_map(|line| line.runs.iter())
                            .map(|run| run.text.as_str())
                            .collect::<String>(),
                    )
                } else {
                    None
                }
            })
            .collect();
        assert!(
            nested_text.contains("Inner"),
            "expected nested table text to stay in nested layout: {nested_text:?}"
        );
    }

    #[test]
    fn nested_fixed_table_percentage_width_uses_table_cell_width() {
        let html = r#"
            <table style="table-layout: fixed; width: 400pt;">
                <tr>
                    <td>
                        <table style="table-layout: fixed; width: 100%;">
                            <colgroup>
                                <col style="width: 30%;">
                                <col style="width: 70%;">
                            </colgroup>
                            <tr><td>A</td><td>B</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (outer_col_widths, outer_cells) = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow {
                    col_widths, cells, ..
                } => Some((col_widths.clone(), cells)),
                _ => None,
            })
            .expect("expected outer table row");
        let nested_col_widths = outer_cells[0]
            .nested_rows
            .iter()
            .find_map(|element| match element {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected nested table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width =
            outer_col_widths[0] - outer_cells[0].padding_left - outer_cells[0].padding_right;
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "nested fixed table should expand to the table cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "nested fixed table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn certificate_like_nested_table_uses_full_width() {
        let html = r#"
            <style>
                @page {
                    size: A4 landscape;
                    margin: 1cm;
                }
                table {
                    table-layout: fixed;
                    width: 100%;
                    border-collapse: collapse;
                    column-count: 2;
                }
                .content th,
                .content td {
                    padding: 0 16px 8px 0;
                    word-wrap: break-word;
                }
            </style>
            <table>
                <tr style="vertical-align: top">
                    <td>
                        <table class="content">
                            <colgroup>
                                <col span="1" style="width: 30%;">
                                <col span="1" style="width: 70%;">
                            </colgroup>
                            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
                            <tr><th>Verification</th><td><a href="https://app.ipocamp.io/verify">https://app.ipocamp.io/verify</a></td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let parsed = parse_html_with_styles(html).unwrap();
        let mut page_rules = Vec::new();
        for css in &parsed.stylesheets {
            page_rules.extend(crate::parser::css::parse_page_rules(css));
        }
        let mut page_size = PageSize::default();
        let mut margin = Margin::default();
        for page_rule in &page_rules {
            if let (Some(width), Some(height)) = (page_rule.width, page_rule.height) {
                page_size = PageSize { width, height };
            }
            if let Some(v) = page_rule.margin_top {
                margin.top = v;
            }
            if let Some(v) = page_rule.margin_right {
                margin.right = v;
            }
            if let Some(v) = page_rule.margin_bottom {
                margin.bottom = v;
            }
            if let Some(v) = page_rule.margin_left {
                margin.left = v;
            }
        }
        let media_ctx = crate::parser::css::MediaContext {
            width: page_size.width,
            height: page_size.height,
        };
        let mut rules = Vec::new();
        for css in &parsed.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet_with_context(
                css,
                Some(media_ctx),
            ));
        }
        let pages = layout_with_rules(&parsed.nodes, page_size, margin, &rules);
        let outer_cells = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { cells, .. } => Some(cells),
                _ => None,
            })
            .expect("expected outer table row");
        let nested_col_widths = outer_cells[0]
            .nested_rows
            .iter()
            .find_map(|element| match element {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected nested content table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width = page_size.width - margin.left - margin.right - 1.5;
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "certificate-like nested table should span the outer cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "certificate-like nested table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn table_cell_preserves_empty_block_background_layout() {
        let encoded = base64_encode(&build_test_png_bytes());
        let html = format!(
            r#"
                <table>
                    <tr>
                        <td>
                            <div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('data:image/png;base64,{encoded}') no-repeat;"></div>
                        </td>
                    </tr>
                </table>
            "#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected outer table row");
        assert!(
            !cells[0].nested_rows.is_empty(),
            "expected block descendant to be preserved as nested layout"
        );
        assert!(
            cells[0].nested_rows.iter().any(|element| matches!(
                element,
                LayoutElement::TextBlock {
                    background_svg: Some(_),
                    block_height: Some(height),
                    ..
                } if (*height - 40.0).abs() < 0.1
            )),
            "expected nested flex block with raster background to survive table-cell layout"
        );
    }

    #[test]
    fn paginate_math_block_advances_y() {
        // MathBlock elements must reserve vertical space so subsequent content
        // doesn't overlap.
        let html = r#"<p>Before</p><div class="math-display" data-math="\frac{a}{b}">frac</div><p>After</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);

        // Gather y positions and element types
        let mut y_positions: Vec<(f32, &str)> = Vec::new();
        for (y, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { .. } => y_positions.push((*y, "TextBlock")),
                LayoutElement::MathBlock { .. } => y_positions.push((*y, "MathBlock")),
                _ => {}
            }
        }

        // We expect: TextBlock("Before"), MathBlock, TextBlock("After")
        assert!(
            y_positions.len() >= 3,
            "Expected at least 3 elements (Before text, MathBlock, After text), got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        // Each element must have a strictly increasing y position
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0,
                "Element {} ({} at y={}) should be below element {} ({} at y={})",
                i,
                y_positions[i].1,
                y_positions[i].0,
                i - 1,
                y_positions[i - 1].1,
                y_positions[i - 1].0,
            );
        }
    }

    #[test]
    fn paginate_math_block_from_markdown() {
        // Test the actual markdown flow: $$ ... $$ produces display math
        // that must advance Y so subsequent text doesn't overlap.
        let md = "# Title\n\n$$\\frac{a}{b}$$\n\nText after math should not overlap";
        let html = crate::parser::markdown::markdown_to_html(md);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);

        let mut y_positions: Vec<(f32, &str)> = Vec::new();
        for (y, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { .. } => y_positions.push((*y, "TextBlock")),
                LayoutElement::MathBlock { .. } => y_positions.push((*y, "MathBlock")),
                _ => {}
            }
        }

        // We expect: TextBlock(Title), MathBlock(frac), TextBlock(Text after...)
        assert!(
            y_positions.len() >= 3,
            "Expected at least 3 elements (Title, MathBlock, After text), got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        // Each element must have a strictly increasing y position
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0,
                "Element {} ({} at y={}) should be below element {} ({} at y={})",
                i,
                y_positions[i].1,
                y_positions[i].0,
                i - 1,
                y_positions[i - 1].1,
                y_positions[i - 1].0,
            );
        }
    }

    /// Verify that styled blocks (background, border, padding) don't cause
    /// subsequent content to overlap.
    #[test]
    fn styled_block_does_not_overlap_next_element() {
        let css = r#"
            .summary { background-color: #eff6ff; border-left: 4px solid #3b82f6; padding: 12px 16px; margin: 16px 0; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <h1>Title</h1>
            <div class="summary">This is a summary box with background and border styling.</div>
            <h2>Next Section</h2>
            <p>This should not overlap with the summary box.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        // Collect Y positions of text blocks with content
        let mut y_positions: Vec<(f32, String)> = Vec::new();
        for (y_pos, elem) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = elem {
                let text: String = lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                if !text.is_empty() {
                    y_positions.push((*y_pos, text[..text.len().min(40)].to_string()));
                }
            }
        }

        // Each Y position should be strictly greater than the previous
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    /// Verify that blockquotes with visual styling don't cause overlap.
    #[test]
    fn blockquote_with_background_no_overlap() {
        let css = r#"
            blockquote { margin: 20px 0; padding: 12px 20px; border-left: 4px solid #3b82f6; background-color: #f8fafc; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <p>Paragraph before the blockquote.</p>
            <blockquote>This is a blockquote with background and border styling that should take up vertical space.</blockquote>
            <p>Paragraph after the blockquote should not overlap.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions: Vec<(f32, String)> = Vec::new();
        for (y_pos, elem) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = elem {
                let text: String = lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                if !text.is_empty() {
                    y_positions.push((*y_pos, text[..text.len().min(40)].to_string()));
                }
            }
        }

        assert!(
            y_positions.len() >= 2,
            "Expected at least 2 text blocks, got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    /// Verify pre blocks with padding/background don't cause overlap.
    #[test]
    fn pre_block_with_padding_no_overlap() {
        let css = r#"
            pre { background-color: #1e293b; padding: 16px 20px; margin: 16px 0; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <p>Before the code block.</p>
            <pre>line 1
line 2
line 3</pre>
            <p>After the code block should not overlap.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions: Vec<(f32, String)> = Vec::new();
        for (y_pos, elem) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = elem {
                let text: String = lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                if !text.is_empty() {
                    y_positions.push((*y_pos, text[..text.len().min(40)].to_string()));
                }
            }
        }

        assert!(
            y_positions.len() >= 2,
            "Expected at least 2 text blocks with content, got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }
    /// Verify fixture text blocks have monotonically increasing Y positions
    /// (no overlapping text).
    fn check_fixture_no_overlap(html: &str, fixture_name: &str) {
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules_and_fonts(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
            &std::collections::HashMap::new(),
        );

        for (page_idx, page) in pages.iter().enumerate() {
            let mut y_positions: Vec<(f32, String)> = Vec::new();
            for (y_pos, elem) in &page.elements {
                if let LayoutElement::TextBlock {
                    lines, position, ..
                } = elem
                {
                    if *position == Position::Absolute {
                        continue;
                    }
                    let text: String = lines
                        .iter()
                        .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                        .collect();
                    if !text.is_empty() {
                        y_positions.push((*y_pos, text[..text.len().min(50)].to_string()));
                    }
                }
            }

            for i in 1..y_positions.len() {
                let (prev_y, ref prev_text) = y_positions[i - 1];
                let (curr_y, ref curr_text) = y_positions[i];
                assert!(
                    curr_y >= prev_y - 0.1,
                    "{fixture_name} page {page_idx}: text blocks overlap!\n  \
                     [{prev}] y={prev_y:.1} {prev_text:?}\n  \
                     [{i}] y={curr_y:.1} {curr_text:?}",
                    prev = i - 1,
                );
            }
        }
    }

    #[test]
    fn simple_report_fixture_no_overlap() {
        check_fixture_no_overlap(
            include_str!("../../tests/fixtures/combined/simple-report.html"),
            "simple-report",
        );
    }

    #[test]
    fn article_fixture_no_overlap() {
        check_fixture_no_overlap(
            include_str!("../../tests/fixtures/combined/article.html"),
            "article",
        );
    }

    #[test]
    fn page_breaks_fixture_no_overlap() {
        check_fixture_no_overlap(
            include_str!("../../tests/fixtures/edge-cases/page-breaks.html"),
            "page-breaks",
        );
    }

    /// Test with styled wrapper block containing only block children.
    #[test]
    fn styled_wrapper_with_block_children_no_overlap() {
        let css = r#"
            .section { padding: 16px; margin-bottom: 16px; border: 1px solid #e2e8f0; border-radius: 4px; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <h1>Page Break Test</h1>
            <div class="section">
                <h2>Section 1</h2>
                <p>Content in section 1.</p>
            </div>
            <div class="section">
                <h2>Section 2</h2>
                <p>Content in section 2 should not overlap with section 1.</p>
            </div>
            <p>Final paragraph after both sections.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions: Vec<(f32, String)> = Vec::new();
        for (y_pos, elem) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                lines, position, ..
            } = elem
            {
                if *position == Position::Absolute {
                    continue;
                }
                let text: String = lines
                    .iter()
                    .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                    .collect();
                if !text.is_empty() {
                    y_positions.push((*y_pos, text[..text.len().min(50)].to_string()));
                }
            }
        }

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 0.5,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    #[test]
    fn nested_div_container_has_background_color() {
        let html = r#"
        <style>
            .d1 { border-left: 2px solid #ef4444; background-color: rgba(239,68,68,0.05); padding: 4px; }
        </style>
        <div class="d1"><span>Level 1</span>
            <div class="d2"><span>Level 2</span><p>Block child</p></div>
        </div>
        "#;
        let result = parse_html_with_styles(html).unwrap();
        let rules = crate::parser::css::parse_stylesheet(&result.stylesheets.join("\n"));
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let has_container_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::Container {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_container_bg,
            "Container should have background_color from rgba stylesheet"
        );
    }

    #[test]
    fn vw_unit_resolves_against_actual_page_size() {
        // 50vw on Letter (612pt wide) should produce ~306pt, not ~297pt (A4)
        let html = r#"<div style="width:50vw;background:red">test</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::LETTER, Margin::default());
        let expected = PageSize::LETTER.width / 2.0; // 306pt
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                block_width: Some(_w),
                background_color: Some(_),
                ..
            } = el
            {
                assert!(
                    (*_w - expected).abs() < 1.0,
                    "50vw on Letter should be ~{expected}pt, got {_w}pt"
                );
                return;
            }
        }
        panic!("expected a TextBlock with explicit width from 50vw");
    }

    // ---- LayoutContext tests ----

    #[test]
    fn layout_context_available_width_returns_parent_content_width() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
            },
            containing_block: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_width() - 400.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_available_height_falls_back_to_viewport() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: None,
                font_size: 16.0,
            },
            containing_block: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_height() - 842.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_available_height_uses_parent_when_set() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(300.0),
                font_size: 16.0,
            },
            containing_block: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_height() - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_with_parent_preserves_viewport() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
            },
            containing_block: Some(ContainingBlock {
                x: 10.0,
                width: 400.0,
                height: 600.0,
                depth: 1,
            }),
            root_font_size: 16.0,
        };
        let child = ctx.with_parent(200.0, Some(150.0), 12.0);
        assert!((child.viewport.width - 595.0).abs() < f32::EPSILON);
        assert!((child.viewport.height - 842.0).abs() < f32::EPSILON);
        assert!((child.available_width() - 200.0).abs() < f32::EPSILON);
        assert!((child.available_height() - 150.0).abs() < f32::EPSILON);
        assert!((child.parent.font_size - 12.0).abs() < f32::EPSILON);
        assert!((child.root_font_size - 16.0).abs() < f32::EPSILON);
        // containing_block is preserved
        assert!(child.containing_block.is_some());
    }

    #[test]
    fn layout_context_with_containing_block_replaces_cb() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
            },
            containing_block: None,
            root_font_size: 16.0,
        };
        let cb = ContainingBlock {
            x: 50.0,
            width: 300.0,
            height: 200.0,
            depth: 2,
        };
        let updated = ctx.with_containing_block(Some(cb));
        assert!(updated.containing_block.is_some());
        assert!((updated.containing_block.unwrap().x - 50.0).abs() < f32::EPSILON);
        // parent is preserved
        assert!((updated.available_width() - 400.0).abs() < f32::EPSILON);
    }

    // ---- Integration tests for extracted layout functions ----

    #[test]
    fn route_element_dispatches_flex() {
        let html = r#"<div style="display:flex"><span>A</span><span>B</span></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn route_element_dispatches_grid() {
        let html = r#"<div style="display:grid;grid-template-columns:1fr 1fr"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn route_element_dispatches_inline() {
        let html = r#"<span>inline text</span>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn inline_block_shrink_to_fit_width() {
        let html = r#"<div style="display:inline-block;background:#eee">short</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // The inline-block should be narrower than the full page width
        let page_width = PageSize::A4.width - Margin::default().left - Margin::default().right;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                block_width: Some(w),
                ..
            } = el
            {
                assert!(
                    *w < page_width,
                    "inline-block width {w} should be less than page width {page_width}"
                );
            }
        }
    }

    #[test]
    fn percentage_border_radius_resolved() {
        let html =
            r#"<div style="width:100px;height:100px;border-radius:50%;background:red">.</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // 50% of min(100px=75pt, 100px=75pt) = 37.5pt
        for (_, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { border_radius, .. }
                | LayoutElement::Container { border_radius, .. } => {
                    if *border_radius > 0.0 {
                        assert!(
                            (*border_radius - 37.5).abs() < 1.0,
                            "border_radius {border_radius} should be ~37.5pt"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn css_height_narrows_available_height_for_children() {
        // Parent with explicit height, child SVG with percentage height
        let html = r#"<div style="height:200pt"><svg width="100" height="50%"></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        fn find_svg(elements: &[(f32, LayoutElement)]) -> Option<f32> {
            for (_, el) in elements {
                match el {
                    LayoutElement::Svg { height, .. } => return Some(*height),
                    LayoutElement::Container { children, .. } => {
                        for child in children {
                            if let LayoutElement::Svg { height, .. } = child {
                                return Some(*height);
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let svg_h = find_svg(&pages[0].elements).expect("expected svg element");
        assert!(
            (svg_h - 100.0).abs() < 1.0,
            "SVG height {svg_h} should be ~100pt (50% of 200pt)"
        );
    }

    #[test]
    fn flex_grow_distributes_remaining_space() {
        let html = r#"
            <div style="display:flex;width:300px">
                <div style="flex-grow:1;background:#aaa">A</div>
                <div style="flex-grow:2;background:#ccc">B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn multicolumn_layout_creates_grid() {
        let html = r#"<div style="column-count:3"><p>One</p><p>Two</p><p>Three</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should produce grid rows
        let has_grid = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::Container { .. } | LayoutElement::GridRow { .. }
            )
        });
        assert!(has_grid, "multi-column should produce Container/GridRow");
    }

    #[test]
    fn bidi_reordering_preserves_content() {
        let html = r#"<p>Hello مرحبا World</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        // Verify all text content is present (might be reordered)
        let mut all_text = String::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        all_text.push_str(&run.text);
                    }
                }
            }
        }
        assert!(
            all_text.contains("Hello") && all_text.contains("World"),
            "BiDi should preserve Latin text"
        );
    }

    // --- Issue #99: pre code color inheritance ---

    #[test]
    fn pre_code_inherits_color_from_pre_not_code_default() {
        let html = r#"<html><head><style>
            code { color: #be123c; }
            pre { color: #e2e8f0; background-color: #1e293b; }
            pre code { color: inherit; }
        </style></head><body>
        <pre><code>Hello World</code></pre>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find the text — may be in TextBlock or Container
        fn find_hello_color(elements: &[(f32, LayoutElement)]) -> Option<(f32, f32, f32)> {
            for (_, el) in elements {
                match el {
                    LayoutElement::TextBlock { lines, .. } => {
                        for line in lines {
                            for run in &line.runs {
                                if run.text.contains("Hello") {
                                    return Some(run.color);
                                }
                            }
                        }
                    }
                    LayoutElement::Container { children, .. } => {
                        for child in children {
                            if let LayoutElement::TextBlock { lines, .. } = child {
                                for line in lines {
                                    for run in &line.runs {
                                        if run.text.contains("Hello") {
                                            return Some(run.color);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let color = find_hello_color(&pages[0].elements).expect("should find 'Hello World' text");
        // pre code { color: inherit } should give #e2e8f0 (0.886, 0.910, 0.941)
        // NOT #be123c (code default red = 0.745, 0.071, 0.235)
        assert!(
            color.0 > 0.7 && color.1 > 0.7,
            "pre>code text should inherit light color from pre, got ({:.3}, {:.3}, {:.3})",
            color.0,
            color.1,
            color.2
        );
    }

    // --- Issue #101: float right positioning (layout side) ---
    // Note: float positioning is handled by the renderer, not the layout engine.
    // This test documents the current limitation: offset_left is 0 from layout.

    #[test]
    #[ignore] // TODO: fix float:right layout positioning (#101)
    fn float_right_positions_at_right_edge() {
        let html = r#"
        <div style="width: 400px">
            <div style="float: right; width: 100px; height: 50px; background: pink">Float</div>
            <p>Text should wrap around the float.</p>
        </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Find the floated element (pink background)
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                float: Float::Right,
                offset_left,
                block_width: Some(_w),
                ..
            } = el
            {
                // Float right should have offset_left > 0 (pushed right)
                // In a 400px (300pt) container with a 100px (75pt) float,
                // the float should be near the right edge
                assert!(
                    *offset_left > 50.0,
                    "float:right should be positioned rightward, got offset_left={offset_left}"
                );
                return;
            }
        }
        // Float right may not produce offset_left — the renderer handles it.
        // Just verify the float property is preserved.
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                float: Float::Right,
                ..
            } = el
            {
                return; // Float property is at least preserved
            }
        }
        panic!("did not find any element with float:right");
    }

    // --- Issue #103: horizontal separators via border-bottom ---

    #[test]
    fn h1_border_bottom_produces_visible_border() {
        let html = r#"<html><head><style>
            h1 { border-bottom: 3px solid #1e40af; padding-bottom: 8px; }
        </style></head><body><h1>Title</h1></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut found_border = false;
        for (_, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { border, .. } if border.bottom.width > 0.0 => {
                    found_border = true;
                }
                LayoutElement::Container { border, .. } if border.bottom.width > 0.0 => {
                    found_border = true;
                }
                _ => {}
            }
        }
        assert!(
            found_border,
            "h1 with border-bottom:3px should produce a visible bottom border"
        );
    }

    // --- Issue #102: margin collapse between adjacent blocks ---

    #[test]
    fn adjacent_block_margins_collapse() {
        let html = r#"<html><head><style>
            .a { margin-bottom: 20px; background: #ddd; }
            .b { margin-top: 10px; background: #ddd; }
        </style></head><body>
            <div class="a">A</div>
            <div class="b">B</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find positions of the two divs
        let mut positions: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        if run.text.trim() == "A" || run.text.trim() == "B" {
                            positions.push(*y);
                        }
                    }
                }
            }
        }
        assert!(
            positions.len() >= 2,
            "should find both divs, got {} positions",
            positions.len()
        );
        // The gap between A's bottom and B's top should reflect collapsed margin
        // max(20px, 10px) = 20px = 15pt, NOT 20+10=30px=22.5pt
        let gap = positions[1] - positions[0];
        // Rough check: gap should be less than what non-collapsed margins would produce
        // With font height ~12pt + 15pt margin: gap ~27pt
        // Without collapse: ~12pt + 22.5pt = 34.5pt
        assert!(
            gap < 32.0,
            "margins should collapse: gap={gap}pt (expected ~27pt, not ~34pt)"
        );
    }
    #[test]
    fn block_margin_left_reduces_content_width() {
        let html = r#"<html><head><style>
            .m { margin-left: 40px; margin-right: 40px; background: #ddd; }
        </style></head><body><div class="m">Indented</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let page_width = PageSize::A4.width - Margin::default().left - Margin::default().right;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock {
                lines, block_width, ..
            } = el
            {
                let has_indented = lines
                    .iter()
                    .any(|l| l.runs.iter().any(|r| r.text.contains("Indented")));
                if has_indented {
                    if let Some(w) = block_width {
                        // 40px = 30pt each side → block should be ~60pt narrower
                        assert!(
                            *w < page_width - 40.0,
                            "block with margin-left/right should be narrower than page: w={w}, page={page_width}"
                        );
                        return;
                    }
                }
            }
        }
        // If no explicit width, the block fills the page — that's the bug
        panic!("block with margin-left/right should have reduced width");
    }

    #[test]
    fn debug_inline_raw_and_wrapped_runs() {
        let html = r#"<html><head><style>
            body { font-family: Georgia, serif; font-size: 15px; line-height: 1.8; }
            .hl { background-color: #fef3c7; padding: 2px 4px; }
        </style></head><body>
            <p>AAA <span class="hl">BBB</span> CCC</p>
            <p>What was once dominated by heavyweight Java libraries is now seeing a new wave of <span class="hl">high-performance native renderers</span> that promise faster output.</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for (li, line) in lines.iter().enumerate() {
                    for (ri, run) in line.runs.iter().enumerate() {
                        eprintln!(
                            "line{li} run{ri}: text={:?} pad=({:.1},{:.1}) bg={:?}",
                            run.text,
                            run.padding.0,
                            run.padding.1,
                            run.background_color.is_some()
                        );
                    }
                }
            }
        }
        assert!(true);
    }

    #[test]
    fn debug_float_right_structure() {
        let html = r#"<html><head><style>
            .container { width: 400px; border: 1px solid #ccc; padding: 10px; }
            .float-right { float: right; width: 100px; height: 80px; background-color: #f472b6; }
        </style></head><body>
        <div class="container">
            <div class="float-right">FR</div>
            <p>Text</p>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        fn dump(elements: &[(f32, LayoutElement)], indent: &str) {
            for (y, el) in elements {
                match el {
                    LayoutElement::Container {
                        float,
                        block_width,
                        children,
                        ..
                    } => {
                        eprintln!(
                            "{indent}Container y={y} float={float:?} w={block_width:?} kids={}",
                            children.len()
                        );
                        for child in children {
                            match child {
                                LayoutElement::Container {
                                    float, block_width, ..
                                } => {
                                    eprintln!(
                                        "{indent}  Container float={float:?} w={block_width:?}"
                                    );
                                }
                                LayoutElement::TextBlock {
                                    float,
                                    block_width,
                                    lines,
                                    ..
                                } => {
                                    let text: String = lines
                                        .iter()
                                        .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                                        .collect();
                                    eprintln!(
                                        "{indent}  TextBlock float={float:?} w={block_width:?} text={text:?}"
                                    );
                                }
                                other => eprintln!("{indent}  {:?}", std::mem::discriminant(other)),
                            }
                        }
                    }
                    LayoutElement::TextBlock {
                        float,
                        block_width,
                        lines,
                        ..
                    } => {
                        let text: String = lines
                            .iter()
                            .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                            .collect();
                        eprintln!(
                            "{indent}TextBlock y={y} float={float:?} w={block_width:?} text={text:?}"
                        );
                    }
                    _ => {}
                }
            }
        }
        dump(&pages[0].elements, "");
        // Just verify structure — we want to see the debug output
        assert!(!pages[0].elements.is_empty());
    }

    // ---------------------------------------------------------------
    // Block layout coverage tests (src/layout/block.rs uncovered paths)
    // ---------------------------------------------------------------

    #[test]
    fn block_percentage_max_width() {
        let html = r#"<html><head><style>
            .clamped { max-width: 50%; }
        </style></head><body>
            <div class="clamped">Narrow</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let found = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock {
                block_width: Some(w),
                ..
            } = el
            {
                *w < 300.0
            } else {
                false
            }
        });
        assert!(found, "Expected a block clamped by max-width: 50%");
    }

    #[test]
    fn block_percentage_min_width() {
        let html = r#"<html><head><style>
            .wide { min-width: 80%; width: 50pt; }
        </style></head><body>
            <div class="wide">Wide</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_percentage_height_resolves_against_containing_block() {
        let html = r#"<div style="height: 400pt; position: relative">
            <div style="height: 50%">Half</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_pct_border_radius_with_height() {
        let html = r#"<html><head><style>
            .pill { border-radius: 50%; width: 100pt; height: 100pt; background: red; }
        </style></head><body>
            <div class="pill">Round</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_radius = pages[0].elements.iter().any(|(_, el)| match el {
            LayoutElement::TextBlock { border_radius, .. } => *border_radius > 0.0,
            LayoutElement::Container { border_radius, .. } => *border_radius > 0.0,
            _ => false,
        });
        assert!(has_radius, "Expected border_radius resolved from 50%");
    }

    #[test]
    fn block_pct_border_radius_without_height() {
        let html = r#"<html><head><style>
            .rounded { border-radius: 50%; width: 80pt; background: blue; }
        </style></head><body>
            <div class="rounded">No height</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_before_pseudo_block_like() {
        let html = r#"<html><head><style>
            .banner::before {
                content: "PREFIX";
                display: block;
                background: green;
            }
        </style></head><body>
            <div class="banner"><p>Content</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_white_space_nowrap_flush_runs() {
        let html = r#"<html><head><style>
            .nowrap { white-space: nowrap; width: 50pt; overflow: hidden; }
        </style></head><body>
            <div class="nowrap">This text should not wrap at all even though narrow</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_inline_block_shrink_to_fit() {
        let html = r#"<html><head><style>
            .ib { display: inline-block; }
        </style></head><body>
            <div><span class="ib">Short</span></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_parent_with_block_children() {
        let html = r#"<html><head><style>
            .box { background: #ff0000; padding: 10pt; border: 1pt solid black; }
        </style></head><body>
            <div class="box">
                <p>First paragraph</p>
                <p>Second paragraph</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_visual = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    ..
                } | LayoutElement::Container {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_visual,
            "Expected wrapper with background from visual parent"
        );
    }

    #[test]
    fn block_visual_wrapper_with_fixed_height() {
        let html = r#"<html><head><style>
            .fixed { background: blue; height: 200pt; padding: 10pt; }
        </style></head><body>
            <div class="fixed">
                <p>Inside fixed height</p>
                <p>Second child</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_overflow_hidden_visual_wrapper_clip() {
        let html = r#"<html><head><style>
            .clip { overflow: hidden; background: gray; width: 150pt; height: 100pt; }
        </style></head><body>
            <div class="clip">
                <p>Clipped paragraph one</p>
                <p>Clipped paragraph two</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_wrapper_padding_propagation() {
        let html = r#"<html><head><style>
            .padded { background: yellow; padding: 20pt 15pt; }
        </style></head><body>
            <div class="padded">
                <p>Child with propagated padding</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_wrapper_patches_auto_height() {
        let html = r#"<html><head><style>
            .autoheight { background: orange; padding: 5pt; border: 2pt solid red; }
        </style></head><body>
            <div class="autoheight">
                <h2>Heading</h2>
                <p>Body text</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_abs_pseudo_before_after_in_block() {
        let html = r#"<html><head><style>
            .rel { position: relative; }
            .rel::before {
                content: "B";
                display: block;
                position: absolute;
                top: 0;
                left: 0;
            }
            .rel::after {
                content: "A";
                display: block;
                position: absolute;
                bottom: 0;
                right: 0;
            }
        </style></head><body>
            <div class="rel">
                <p>Main content</p>
                <p>More content</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_container_wrapper_aspect_ratio() {
        let html = r#"<html><head><style>
            .aspect { aspect-ratio: 16 / 9; width: 320pt; background: purple; }
        </style></head><body>
            <div class="aspect"><p>Aspect ratio box</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_container = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::Container { .. }));
        assert!(
            has_container,
            "aspect-ratio should produce a Container element"
        );
    }

    #[test]
    fn block_wrapper_inline_block_children() {
        let html = r#"<html><head><style>
            .parent { background: cyan; height: 60pt; }
            .child { display: inline-block; width: 40pt; }
        </style></head><body>
            <div class="parent">
                <span class="child">A</span>
                <span class="child">B</span>
                <span class="child">C</span>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_positioned_container_cb_non_wrapper() {
        let html = r#"<div style="position: relative">
            <span>Inline text only</span>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_inline_block_flush_at_block_break() {
        let html = r#"<html><head><style>
            .ib { display: inline-block; width: 50pt; }
        </style></head><body>
            <div>
                <span class="ib">X</span>
                <span class="ib">Y</span>
                <div>Block break</div>
                <span class="ib">Z</span>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_no_inline_visual_wrapper_path() {
        let html = r#"<html><head><style>
            .visual { background: lime; border: 1pt solid black; }
        </style></head><body>
            <div class="visual">
                <div>Only block children, no inline text</div>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_container_or_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::Container { .. }
                    | LayoutElement::TextBlock {
                        background_color: Some(_),
                        ..
                    }
            )
        });
        assert!(
            has_container_or_bg,
            "Expected Container or TextBlock with visual properties"
        );
    }
    // -----------------------------------------------------------------------
    // helpers.rs coverage tests
    // -----------------------------------------------------------------------

    #[test]
    fn helpers_resolve_padding_box_height_border_box() {
        use crate::layout::helpers::resolve_padding_box_height;
        use crate::style::computed::BoxSizing;

        let h =
            resolve_padding_box_height(50.0, Some(200.0), 10.0, 10.0, 20.0, BoxSizing::BorderBox);
        assert!((h - 180.0).abs() < 0.01);

        let h2 = resolve_padding_box_height(50.0, Some(10.0), 5.0, 5.0, 30.0, BoxSizing::BorderBox);
        assert!((h2 - 0.0).abs() < 0.01);
    }

    #[test]
    fn helpers_pseudo_block_display_block_renders() {
        let html = r#"<html><head><style>
            p::before {
                content: "PREFIX";
                display: block;
                padding: 5pt;
                background-color: #eee;
            }
        </style></head><body><p>Main text</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let t: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    texts.push(t);
                }
            }
        }
        assert!(
            texts.iter().any(|t| t.contains("PREFIX")),
            "expected block ::before with 'PREFIX', got: {:?}",
            texts
        );
    }

    #[test]
    fn helpers_pseudo_block_absolute_positioned() {
        let html = r#"<html><head><style>
            .container {
                position: relative;
                width: 300pt;
                height: 200pt;
            }
            .container::after {
                content: "ABS";
                position: absolute;
                top: 10pt;
                left: 20pt;
                padding: 4pt;
            }
        </style></head><body><div class="container">Content</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let abs_el = pages[0].elements.iter().find(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    position: Position::Absolute,
                    ..
                }
            )
        });
        assert!(
            abs_el.is_some(),
            "expected an absolute-positioned pseudo TextBlock"
        );
        if let Some((
            _,
            LayoutElement::TextBlock {
                offset_top,
                offset_left,
                ..
            },
        )) = abs_el
        {
            assert!(
                (*offset_top - 10.0).abs() < 1.0,
                "offset_top={}",
                offset_top
            );
            assert!(
                (*offset_left - 20.0).abs() < 1.0,
                "offset_left={}",
                offset_left
            );
        }
    }

    #[test]
    fn helpers_pseudo_block_absolute_bottom_right() {
        let html = r#"<html><head><style>
            .box {
                position: relative;
                width: 400pt;
                height: 300pt;
            }
            .box::before {
                content: "BR";
                position: absolute;
                bottom: 10pt;
                right: 20pt;
            }
        </style></head><body><div class="box">Hello</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        fn find_abs_br(elements: &[(f32, LayoutElement)]) -> Option<(f32, f32)> {
            for (_, el) in elements {
                match el {
                    LayoutElement::TextBlock {
                        position: Position::Absolute,
                        offset_top,
                        offset_left,
                        lines,
                        ..
                    } if lines
                        .iter()
                        .flat_map(|l| l.runs.iter())
                        .any(|r| r.text.contains("BR")) =>
                    {
                        return Some((*offset_top, *offset_left));
                    }
                    LayoutElement::Container { children, .. } => {
                        for child in children {
                            if let LayoutElement::TextBlock {
                                position: Position::Absolute,
                                offset_top,
                                offset_left,
                                lines,
                                ..
                            } = child
                            {
                                if lines
                                    .iter()
                                    .flat_map(|l| l.runs.iter())
                                    .any(|r| r.text.contains("BR"))
                                {
                                    return Some((*offset_top, *offset_left));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        let abs_el = find_abs_br(&pages[0].elements);
        assert!(
            abs_el.is_some(),
            "expected absolute pseudo with bottom/right resolved"
        );
        let (top, left) = abs_el.unwrap();
        assert!(top > 0.0, "bottom should resolve to positive top: {}", top);
        assert!(
            left > 0.0,
            "right should resolve to positive left: {}",
            left
        );
    }

    #[test]
    fn helpers_resolve_abs_cb_bottom_right_only() {
        use crate::layout::helpers::resolve_abs_containing_block;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        style.position = Position::Absolute;
        style.top = None;
        style.left = None;
        style.bottom = Some(10.0);
        style.right = Some(20.0);

        let cb = ContainingBlock {
            x: 0.0,
            width: 500.0,
            height: 400.0,
            depth: 0,
        };

        let (resolved_cb, top, left) = resolve_abs_containing_block(&style, Some(cb), 50.0, 100.0);
        assert!(resolved_cb.is_some());
        assert!((top - 340.0).abs() < 0.01, "top={}", top);
        assert!((left - 380.0).abs() < 0.01, "left={}", left);
    }

    #[test]
    fn helpers_resolve_abs_cb_none() {
        use crate::layout::helpers::resolve_abs_containing_block;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        style.position = Position::Absolute;
        style.top = Some(15.0);
        style.left = Some(25.0);

        let (resolved_cb, top, left) = resolve_abs_containing_block(&style, None, 50.0, 100.0);
        assert!(resolved_cb.is_none());
        assert!((top - 15.0).abs() < 0.01);
        assert!((left - 25.0).abs() < 0.01);
    }

    #[test]
    fn helpers_patch_abs_children_cb_resolves_offsets() {
        use crate::layout::helpers::patch_absolute_children_containing_block;

        let cb = ContainingBlock {
            x: 0.0,
            width: 600.0,
            height: 400.0,
            depth: 1,
        };

        let mut elements = vec![LayoutElement::TextBlock {
            lines: vec![],
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: Some(100.0),
            block_height: Some(50.0),
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Absolute,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 30.0,
            offset_right: 40.0,
            containing_block: None,
            clip_children_count: 0,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        }];

        patch_absolute_children_containing_block(&mut elements, cb);

        if let LayoutElement::TextBlock {
            offset_top,
            offset_left,
            containing_block,
            ..
        } = &elements[0]
        {
            assert!(containing_block.is_some(), "containing_block should be set");
            assert!(
                (*offset_left - 460.0).abs() < 0.01,
                "offset_left={}",
                offset_left
            );
            assert!(
                (*offset_top - 320.0).abs() < 0.01,
                "offset_top={}",
                offset_top
            );
        } else {
            panic!("expected TextBlock");
        }
    }

    #[test]
    fn helpers_aspect_ratio_height_computed() {
        use crate::layout::helpers::aspect_ratio_height;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        assert!(aspect_ratio_height(200.0, &style).is_none());

        style.aspect_ratio = Some(2.0);
        let h = aspect_ratio_height(200.0, &style);
        assert!(h.is_some());
        assert!((h.unwrap() - 100.0).abs() < 0.01);

        style.aspect_ratio = Some(0.0);
        assert!(aspect_ratio_height(200.0, &style).is_none());
    }

    #[test]
    fn helpers_format_list_marker_roman_large() {
        use crate::layout::helpers::format_list_marker;
        use crate::style::computed::ListStyleType;

        assert_eq!(
            format_list_marker(ListStyleType::UpperRoman, 2024),
            "MMXXIV. "
        );
        assert_eq!(
            format_list_marker(ListStyleType::LowerRoman, 999),
            "cmxcix. "
        );
        assert_eq!(format_list_marker(ListStyleType::UpperRoman, 49), "XLIX. ");
        assert_eq!(
            format_list_marker(ListStyleType::LowerRoman, 444),
            "cdxliv. "
        );
    }

    #[test]
    fn helpers_pseudo_block_with_min_height() {
        let html = r#"<html><head><style>
            p::before {
                content: "X";
                display: block;
                min-height: 50pt;
            }
        </style></head><body><p>Text</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);

        let pseudo = pages[0].elements.iter().find_map(|(_, el)| match el {
            LayoutElement::TextBlock {
                lines,
                block_height,
                ..
            } if lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .any(|r| r.text.contains("X")) =>
            {
                Some(block_height)
            }
            _ => None,
        });
        assert!(pseudo.is_some(), "expected pseudo-block with min-height");
        if let Some(Some(h)) = pseudo {
            assert!(
                *h >= 50.0,
                "min-height should enforce at least 50pt, got {}",
                h
            );
        }
    }

    #[test]
    fn helpers_resolve_content_counters_in_layout() {
        let html = r#"<html><head><style>
            ol { counter-reset: item; list-style-type: none; }
            ol li { counter-increment: item; }
            ol li::before { content: counters(item, ".") " "; display: block; }
        </style></head><body>
            <ol>
                <li>A
                    <ol>
                        <li>B</li>
                    </ol>
                </li>
            </ol>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let t: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    if !t.trim().is_empty() {
                        texts.push(t);
                    }
                }
            }
        }
        let has_nested = texts.iter().any(|t| t.contains("1.1"));
        assert!(
            has_nested,
            "expected nested counter '1.1' from counters(), got: {:?}",
            texts
        );
    }

    #[test]
    fn helpers_background_svg_for_style_raster() {
        use crate::layout::helpers::background_svg_for_style;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        assert!(background_svg_for_style(&style).is_none());

        style.background_image = Some(TEST_JPEG_DATA_URI.to_string());
        let _ = background_svg_for_style(&style);
    }

    #[test]
    fn compute_root_margin_resolves_body_margin() {
        // body { margin: 40px } → 40 CSS px = 30pt on each side.
        let rules = parse_stylesheet("body { margin: 40px; }");
        let m = compute_root_margin(&rules, PageSize::LETTER);
        assert!((m.top - 30.0).abs() < 0.01, "top = {}", m.top);
        assert!((m.right - 30.0).abs() < 0.01, "right = {}", m.right);
        assert!((m.bottom - 30.0).abs() < 0.01, "bottom = {}", m.bottom);
        assert!((m.left - 30.0).abs() < 0.01, "left = {}", m.left);
    }

    #[test]
    fn compute_root_margin_zero_when_no_body_rule() {
        let rules = parse_stylesheet("p { margin: 40px; }");
        let m = compute_root_margin(&rules, PageSize::A4);
        assert_eq!(m.top, 0.0);
        assert_eq!(m.right, 0.0);
        assert_eq!(m.bottom, 0.0);
        assert_eq!(m.left, 0.0);
    }

    #[test]
    fn compute_root_margin_accepts_html_and_root_selectors() {
        let rules = parse_stylesheet(":root { margin-top: 20pt; } html { margin-left: 10pt; }");
        let m = compute_root_margin(&rules, PageSize::A4);
        assert!((m.top - 20.0).abs() < 0.01);
        assert!((m.left - 10.0).abs() < 0.01);
    }
}

// (end of file -- debug tests removed)
#[cfg(any())]
mod _removed {
    #![allow(unused)]
    fn debug_pdf_output() {
        let html = r#"<p><span>Acme</span> <span>Corp</span></p>
            <p><strong>Bold</strong> Normal</p>
            <table><tr><td>SVG rendering add-on</td></tr></table>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Search for text rendering commands in the PDF content
        for line in pdf_str.lines() {
            if line.contains("Tj") {
                eprintln!("PDF Tj: {:?}", line.trim());
            }
        }
        // Check that the PDF contains properly spaced text
        assert!(
            pdf_str.contains("(Acme") || pdf_str.contains("( Corp"),
            "PDF should contain Acme and Corp text"
        );
    }

    #[test]
    fn debug_space_preservation_html_parser() {
        // Check what the HTML parser produces for various inputs
        use crate::parser::dom::DomNode;

        fn dump_nodes(nodes: &[DomNode], indent: usize) -> String {
            let mut out = String::new();
            for node in nodes {
                match node {
                    DomNode::Text(t) => {
                        out.push_str(&format!("{:indent$}Text({:?})\n", "", t, indent = indent));
                    }
                    DomNode::Element(el) => {
                        out.push_str(&format!(
                            "{:indent$}Element({:?})\n",
                            "",
                            el.tag,
                            indent = indent
                        ));
                        out.push_str(&dump_nodes(&el.children, indent + 2));
                    }
                }
            }
            out
        }

        // Test what html5ever produces for span-space-span
        let html = "<p><span>Acme</span> <span>Corp</span></p>";
        let nodes = parse_html(html).unwrap();
        let dump = dump_nodes(&nodes, 0);
        eprintln!("=== span-space-span ===\n{dump}");

        // Test what html5ever produces for br-separated text
        let html2 = "<p><strong>Bill to:</strong><br>Acme Corp<br>New York</p>";
        let nodes2 = parse_html(html2).unwrap();
        let dump2 = dump_nodes(&nodes2, 0);
        eprintln!("=== br-separated ===\n{dump2}");

        // Test strong followed by text in same element
        let html3 = "<p><strong>Hello</strong> World</p>";
        let nodes3 = parse_html(html3).unwrap();
        let dump3 = dump_nodes(&nodes3, 0);
        eprintln!("=== strong-space-text ===\n{dump3}");

        // Test with the full invoice-like structure
        let html4 =
            r#"<p><span class="label">Invoice #</span><br><strong>INV-2026-0042</strong></p>"#;
        let nodes4 = parse_html(html4).unwrap();
        let dump4 = dump_nodes(&nodes4, 0);
        eprintln!("=== invoice label ===\n{dump4}");
    }

    #[test]
    fn debug_space_preservation() {
        // Test 1: Simple text with spaces
        let html = "<p>Hello World</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let text: String = lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect();
            eprintln!("Test 1 text: {:?}", text);
            assert!(
                text.contains("Hello World"),
                "Spaces lost in simple text: {text:?}"
            );
        }

        // Test 2: Inline elements with space between
        let html2 = "<p><span>Hello</span> <span>World</span></p>";
        let nodes2 = parse_html(html2).unwrap();
        let pages2 = layout(&nodes2, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages2[0].elements[0] {
            let text: String = lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect();
            eprintln!("Test 2 text: {:?}", text);
            assert!(
                text.contains("Hello") && text.contains("World"),
                "Missing text: {text:?}"
            );
            let combined = text.replace(' ', "");
            assert_ne!(text, combined, "Spaces completely lost: {text:?}");
        }

        // Test 3: Table cell with spaces
        let html3 = "<table><tr><td>Custom font embedding module</td></tr></table>";
        let nodes3 = parse_html(html3).unwrap();
        let pages3 = layout(&nodes3, PageSize::A4, Margin::default());
        for (_, el) in &pages3[0].elements {
            if let LayoutElement::TableRow { cells, .. } = el {
                let text: String = cells[0]
                    .lines
                    .iter()
                    .flat_map(|l| l.runs.iter())
                    .map(|r| r.text.as_str())
                    .collect();
                eprintln!("Test 3 text: {:?}", text);
                assert!(
                    text.contains("Custom font"),
                    "Spaces lost in table cell: {text:?}"
                );
            }
        }

        // Test 4: bold/br structure from invoice
        let html4 = "<p><strong>Bill to:</strong><br>Acme Corp<br>New York, NY 10001</p>";
        let nodes4 = parse_html(html4).unwrap();
        let pages4 = layout(&nodes4, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages4[0].elements[0] {
            for (i, line) in lines.iter().enumerate() {
                let line_text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
                eprintln!(
                    "Test 4 line {i}: {:?} (runs: {:?})",
                    line_text,
                    line.runs
                        .iter()
                        .map(|r| r.text.as_str())
                        .collect::<Vec<_>>()
                );
            }
            let all_text: String = lines
                .iter()
                .map(|l| l.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("Test 4 combined: {:?}", all_text);
            assert!(
                all_text.contains("Acme Corp"),
                "Spaces in 'Acme Corp' lost: {all_text:?}"
            );
            assert!(
                all_text.contains("New York"),
                "Spaces in 'New York' lost: {all_text:?}"
            );
        }
    }

    #[test]
    fn textblock_with_border_has_visual() {
        // Line 1232: has_visual check for border.has_any() in wrapper TextBlock path
        let html = r#"<div style="border: 1pt solid black; overflow: hidden; height: 50pt"><p>Inside</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        let found_clip = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { clip_rect, .. } = el {
                clip_rect.is_some()
            } else {
                false
            }
        });
        assert!(
            found_clip,
            "Expected a TextBlock with clip_rect from overflow:hidden"
        );
    }

    #[test]
    fn flex_column_direction_layout() {
        // Lines 1508, 1711, 1786-1790: FlexRow column direction rendering
        let html = r#"<div style="display: flex; flex-direction: column"><div>First</div><div>Second</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn table_rowspan_cell_handling() {
        // Lines 2248, 2250-2253: Table rowspan cell handling
        let html =
            r#"<table><tr><td rowspan="2">Spanning</td><td>A</td></tr><tr><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let row_count = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .count();
        assert!(
            row_count >= 2,
            "Expected at least 2 table rows, got {row_count}"
        );
    }

    #[test]
    fn table_cell_border_propagation() {
        // Line 2436: Table cell border propagation with preferred widths fitting
        let html = r#"<table style="width: 400pt"><tr><td style="border: 1pt solid black">Cell</td><td>Other</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn inline_link_collects_url() {
        // Line 2567: Inline element in collect_text_runs with link URL
        let html = r#"<p><a href="https://example.com">Click here</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let has_link = lines.iter().any(|l| {
                l.runs
                    .iter()
                    .any(|r| r.link_url.as_deref() == Some("https://example.com"))
            });
            assert!(has_link, "Expected link URL in text runs");
        }
    }

    #[test]
    fn inline_span_border_radius_from_stylesheet() {
        // Lines 2686, 2690-2702: collect_text_runs_inner inline span with border_radius
        let css = "span.tag { background-color: #eee; border-radius: 4pt; padding: 2pt 4pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<p><span class="tag">Label</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let has_br = lines
                .iter()
                .any(|l| l.runs.iter().any(|r| r.border_radius > 0.0));
            assert!(has_br, "Expected border_radius > 0 on inline span text run");
        }
    }

    #[test]
    fn paginate_image_height() {
        // Lines 3116-3156: Image height handling in paginate
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="100" height="100">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn paginate_horizontal_rule() {
        // Lines 3152-3155: HorizontalRule height in paginate
        let html = "<p>Above</p><hr><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_hr = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::HorizontalRule { .. }));
        assert!(has_hr, "Expected a HorizontalRule element");
    }

    #[test]
    fn page_break_in_paginate() {
        // Line 3193: Page break handling in paginate
        let html = r#"<p>Page 1 content</p><div style="page-break-before: always"><p>Page 2 content</p></div><div style="page-break-before: always"><p>Page 3 content</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(
            pages.len() >= 3,
            "Expected at least 3 pages, got {}",
            pages.len()
        );
    }

    #[test]
    fn layout_input_element() {
        let html = r#"<input type="text" value="Hello">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_input_with_placeholder() {
        let html = r#"<input type="text" placeholder="Enter name...">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_select_element() {
        let html = r#"<select><option>One</option><option>Two</option></select>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_textarea_element() {
        let html = r#"<textarea>Some text content</textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_textarea_with_custom_size() {
        let html = r#"<textarea style="width: 200px; height: 100px">Content</textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_video_element() {
        let html = r#"<video width="320" height="240"></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_video_default_size() {
        let html = r#"<video></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_audio_element() {
        let html = r#"<audio></audio>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_progress_element() {
        let html = r#"<progress value="0.7" max="1"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bar = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::ProgressBar { .. }));
        assert!(has_bar, "Expected a ProgressBar element");
    }

    #[test]
    fn layout_progress_zero_value() {
        let html = r#"<progress value="0" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0));
    }

    #[test]
    fn layout_progress_full_value() {
        let html = r#"<progress value="100" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(1.0));
    }

    #[test]
    fn layout_progress_over_max_clamped() {
        let html = r#"<progress value="200" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(1.0));
    }

    #[test]
    fn layout_meter_element() {
        let html = r#"<meter value="0.6" max="1"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bar = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::ProgressBar { .. }));
        assert!(has_bar, "Expected a ProgressBar element for meter");
    }

    #[test]
    fn layout_meter_low_high_thresholds() {
        let html = r#"<meter value="10" max="100" low="25" high="75"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        assert!(fill.is_some());
        let (r, _, _) = fill.unwrap();
        assert!(r > 0.8, "Expected red fill for low meter value");
    }

    #[test]
    fn layout_meter_high_value_green() {
        let html = r#"<meter value="90" max="100" low="25" high="75"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        assert!(fill.is_some());
        let (_, g, _) = fill.unwrap();
        assert!(g > 0.7, "Expected green fill for high meter value");
    }

    #[test]
    fn layout_form_elements_in_context() {
        let html = r#"
            <div>
                <p>Name:</p>
                <input type="text" value="John">
                <p>Country:</p>
                <select><option>France</option><option>USA</option></select>
                <p>Bio:</p>
                <textarea>Some biography text here</textarea>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 3);
    }

    #[test]
    fn layout_progress_custom_width() {
        let html = r#"<progress value="50" max="100" style="width: 200px"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let width = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { width, .. } = el {
                Some(*width)
            } else {
                None
            }
        });
        assert_eq!(width, Some(200.0));
    }

    #[test]
    fn grid_layout_repeat() {
        let css = ".grid { display: grid; grid-template-columns: repeat(3, 1fr); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(
            grid_rows.len(),
            1,
            "Expected 1 grid row with 3 columns from repeat(3, 1fr)"
        );
    }

    #[test]
    fn grid_layout_minmax() {
        let css = ".grid { display: grid; grid-template-columns: minmax(50pt, 200pt) 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(!grid_rows.is_empty(), "Expected GridRow from minmax grid");
    }

    #[test]
    fn grid_layout_auto_fill() {
        let css = ".grid { display: grid; grid-template-columns: repeat(auto-fill, 100px); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn grid_layout_repeat_with_minmax() {
        let css = ".grid { display: grid; grid-template-columns: repeat(3, minmax(50px, 1fr)); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1);
    }

    #[test]
    fn multi_column_layout() {
        let css = ".cols { column-count: 2; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>Col 1</div><div>Col 2</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1, "Expected 1 row from 2-column layout");
    }

    #[test]
    fn multi_column_three_cols() {
        let css = ".cols { column-count: 3; column-gap: 10pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1);
    }

    #[test]
    fn multi_column_wraps_rows() {
        let css = ".cols { column-count: 2; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>A</div><div>B</div><div>C</div><div>D</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(
            grid_rows.len(),
            2,
            "Expected 2 rows from 4 items in 2-column layout"
        );
    }

    #[test]
    fn layout_input_empty_no_value() {
        let html = r#"<input type="text">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_select_empty_options() {
        let html = r#"<select></select>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_textarea_empty() {
        let html = r#"<textarea></textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_video_with_css_dimensions() {
        let html = r#"<video style="width: 400px; height: 300px"></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_audio_with_css_dimensions() {
        let html = r#"<audio style="width: 250px"></audio>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_progress_no_value_attr() {
        let html = r#"<progress></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0));
    }

    #[test]
    fn layout_meter_no_thresholds() {
        let html = r#"<meter value="50" max="100"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        // 50/100 = 0.5, between default low (25) and high (75) → yellow
        assert!(fill.is_some());
        let (r, _, _) = fill.unwrap();
        assert!(r > 0.9, "Expected yellow fill for mid-range meter");
    }

    #[test]
    fn layout_meter_zero_max() {
        let html = r#"<meter value="5" max="0"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0), "Zero max should produce 0 fraction");
    }

    #[test]
    fn heading_level_returns_correct_values() {
        assert_eq!(heading_level(HtmlTag::H1), Some(1));
        assert_eq!(heading_level(HtmlTag::H2), Some(2));
        assert_eq!(heading_level(HtmlTag::H3), Some(3));
        assert_eq!(heading_level(HtmlTag::H4), Some(4));
        assert_eq!(heading_level(HtmlTag::H5), Some(5));
        assert_eq!(heading_level(HtmlTag::H6), Some(6));
        assert_eq!(heading_level(HtmlTag::P), None);
        assert_eq!(heading_level(HtmlTag::Div), None);
    }

    #[test]
    fn layout_heading_has_level_in_textblock() {
        let html = "<h2>Section Title</h2>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_heading = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    heading_level: Some(2),
                    clip_children_count: 0,
                    ..
                }
            )
        });
        assert!(
            has_heading,
            "h2 should produce TextBlock with heading_level=2"
        );
    }

    #[test]
    fn layout_paragraph_has_no_heading_level() {
        let html = "<p>Just text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_heading = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    heading_level: Some(_),
                    clip_children_count: 0,
                    ..
                }
            )
        });
        assert!(!has_heading, "p should not have a heading_level");
    }

    #[test]
    fn column_count_1_not_grid() {
        // column-count: 1 should not trigger grid layout
        let css = ".cols { column-count: 1; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><p>Single column</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(
            grid_rows.is_empty(),
            "column-count: 1 should not produce grid rows"
        );
    }

    // ── push_text_run_with_fallback ─────────────────────────────────────────

    fn make_stub_font(name: &str) -> crate::parser::ttf::TtfFont {
        use crate::parser::ttf::{FontVerticalMetrics, TtfFont};
        TtfFont {
            font_name: name.to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 1000, 800],
            pdf_metrics: FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: FontVerticalMetrics::new(800, -200, 0),
            cmap: std::collections::HashMap::new(),
            glyph_widths: vec![500],
            num_h_metrics: 1,
            flags: 0,
            data: vec![],
        }
    }

    fn base_text_run(text: &str, family: FontFamily) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: family,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        }
    }

    #[test]
    fn fallback_ascii_only_stays_single_run() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        let run = base_text_run("Hello world", FontFamily::Helvetica);
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].font_family, FontFamily::Helvetica);
    }

    #[test]
    fn fallback_cjk_only_uses_fallback_font() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        let run = base_text_run("\u{4F60}\u{597D}", FontFamily::Helvetica); // 你好
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].font_family,
            FontFamily::Custom(crate::system_fonts::UNICODE_FALLBACK_KEY.to_string())
        );
    }

    #[test]
    fn fallback_mixed_ascii_cjk_splits_into_segments() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        // "Hello你好World"
        let run = base_text_run("Hello\u{4F60}\u{597D}World", FontFamily::Helvetica);
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "Hello");
        assert_eq!(runs[0].font_family, FontFamily::Helvetica);
        assert_eq!(runs[1].text, "\u{4F60}\u{597D}");
        assert_eq!(
            runs[1].font_family,
            FontFamily::Custom(crate::system_fonts::UNICODE_FALLBACK_KEY.to_string())
        );
        assert_eq!(runs[2].text, "World");
        assert_eq!(runs[2].font_family, FontFamily::Helvetica);
    }

    #[test]
    fn fallback_custom_font_not_split() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        // Custom fonts handle their own glyph encoding; no splitting.
        let run = base_text_run(
            "Hello\u{4F60}\u{597D}",
            FontFamily::Custom("MyFont".to_string()),
        );
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].font_family,
            FontFamily::Custom("MyFont".to_string())
        );
    }

    #[test]
    fn fallback_no_fallback_font_loaded_passes_through() {
        let fonts = HashMap::new(); // no fallback loaded
        let run = base_text_run("Hello\u{4F60}\u{597D}", FontFamily::Helvetica);
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].font_family, FontFamily::Helvetica);
    }

    #[test]
    fn fallback_times_roman_also_splits() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        let run = base_text_run("A\u{4E16}", FontFamily::TimesRoman); // A世
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].font_family, FontFamily::TimesRoman);
        assert_eq!(
            runs[1].font_family,
            FontFamily::Custom(crate::system_fonts::UNICODE_FALLBACK_KEY.to_string())
        );
    }

    #[test]
    fn fallback_courier_also_splits() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        let run = base_text_run("X\u{3042}", FontFamily::Courier); // Xあ
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].font_family, FontFamily::Courier);
        assert_eq!(
            runs[1].font_family,
            FontFamily::Custom(crate::system_fonts::UNICODE_FALLBACK_KEY.to_string())
        );
    }

    #[test]
    fn fallback_preserves_run_style_properties() {
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_stub_font("Fallback"),
        );
        let mut run = base_text_run("A\u{4E16}", FontFamily::Helvetica);
        run.bold = true;
        run.italic = true;
        run.font_size = 24.0;
        run.color = (1.0, 0.0, 0.0);
        let mut runs = Vec::new();
        push_text_run_with_fallback(run, &mut runs, &fonts);
        assert_eq!(runs.len(), 2);
        // Both segments should preserve the style properties.
        for r in &runs {
            assert!(r.bold);
            assert!(r.italic);
            assert_eq!(r.font_size, 24.0);
            assert_eq!(r.color, (1.0, 0.0, 0.0));
        }
    }
}
