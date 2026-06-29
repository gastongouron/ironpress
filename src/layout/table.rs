use crate::parser::css::{
    selector_matches_with_context, specificity, AncestorInfo, CssRule, CssValue, SelectorContext,
};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    compute_style_with_context, BorderCollapse, BorderStyle, BoxSizing, ComputedStyle, Display,
    FontStyle, FontWeight, TableLayout, TextAlign, VerticalAlign, Visibility, WhiteSpace,
};
use std::collections::HashMap;

use super::context::{LayoutContext, LayoutEnv, ParentBox, Viewport};
use super::engine::{
    collects_as_inline_text, flatten_element, has_background_paint, recurses_as_layout_child,
    CounterState, LayoutBorder, LayoutBorderSide, LayoutElement, TextLine, TextRun,
};
use super::paginate::{estimate_element_height, table_row_content_width};
use super::text::{
    collapse_whitespace, estimate_word_width, expand_pre_tabs, resolve_style_font_family,
    resolved_line_height_factor, wrap_text_runs, TextWrapOptions,
};

const MAX_COLSPAN: usize = 1000;
const MAX_ROWSPAN: usize = 65_534;

/// A table cell ready for rendering.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TableCell {
    pub lines: Vec<TextLine>,
    pub nested_rows: Vec<LayoutElement>,
    pub bold: bool,
    pub background_color: Option<(f32, f32, f32, f32)>,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    /// Number of columns this cell spans (default 1).
    pub colspan: usize,
    /// Number of rows this cell spans (default 1).
    pub rowspan: usize,
    /// Per-side border specification.
    pub border: LayoutBorder,
    /// Text alignment within the cell.
    pub text_align: TextAlign,
    /// Vertical alignment within the row box.
    pub vertical_align: VerticalAlign,
    /// Minimum content-box height from an explicit `height` on the cell. The
    /// row grows to at least this even when the cell has no text (CSS treats
    /// the cell `height` as a minimum). 0.0 = auto.
    pub min_content_height: f32,
    /// `empty-cells: hide` is in effect and this cell has no content, so its
    /// background and borders must not be painted (the table background shows
    /// through instead).
    pub hide_if_empty: bool,
    /// Grid-only: when set, the cell's painted box (background + border) is
    /// inset from the track cell rather than filling it, to model
    /// `justify-items`/`align-items` with an explicit item size smaller than
    /// the track. `None` for table cells (which always fill). Fields are the
    /// inset from the left/top of the track and the item's own painted size.
    pub grid_inset: Option<GridInset>,
    /// Whether the cell clips its nested content (`overflow: hidden`/`clip`/
    /// `scroll`/`auto` on a grid item). When true, the cell's `nested_rows` are
    /// painted under a clip at the cell's padding box. `false` for table cells
    /// and non-clipping grid items.
    pub clips: bool,
    /// CSS `linear-gradient()` background painted across the cell's box. Grid
    /// items (and table cells) are block containers, so a gradient/image
    /// `background` paints over the cell area exactly like a normal block
    /// (css-backgrounds-3 §3). `None` when the cell has no gradient background.
    pub background_gradient: Option<crate::style::computed::LinearGradient>,
    /// CSS `radial-gradient()` background painted across the cell's box.
    pub background_radial_gradient: Option<crate::style::computed::RadialGradient>,
    /// CSS `conic-gradient()` background painted across the cell's box.
    pub background_conic_gradient: Option<crate::style::computed::ConicGradient>,
}

/// Placement of a grid item's painted box within its (possibly larger) track
/// cell. All values in points relative to the track cell's top-left corner.
#[derive(Debug, Clone, Copy)]
pub struct GridInset {
    pub offset_x: f32,
    pub offset_y: f32,
    pub width: f32,
    pub height: f32,
}

/// Minimum outer width a nested layout element wants inside an auto-sized table
/// cell. Used so shrink-to-fit columns stay at least as wide as fixed-width
/// block descendants, nested tables, and replaced content.
fn nested_element_preferred_width(element: &LayoutElement) -> f32 {
    match element {
        LayoutElement::TableRow { .. } => table_row_content_width(element),
        LayoutElement::TextBlock { block_width, .. }
        | LayoutElement::Container { block_width, .. } => block_width.unwrap_or(0.0),
        LayoutElement::Image { width, .. } | LayoutElement::Svg { width, .. } => *width,
        _ => 0.0,
    }
}

/// Whether a `<td>`/`<th>` has no in-flow content for the purpose of
/// `empty-cells: hide`. A cell is empty only if it has no element children and
/// all its text is ASCII whitespace. A non-breaking space (`&nbsp;`, U+00A0) is
/// content per CSS, so it makes the cell non-empty even though it collapses
/// away during whitespace processing.
fn cell_has_no_content(cell_el: &ElementNode) -> bool {
    cell_el.children.iter().all(|child| match child {
        DomNode::Element(_) => false,
        // A non-breaking space (U+00A0) is content, so only ASCII whitespace
        // counts as "empty" here.
        DomNode::Text(text) => text.chars().all(|c| c.is_ascii_whitespace()),
    })
}

pub(crate) fn table_cell_content_height(cell: &TableCell) -> f32 {
    // An explicit cell height acts as a minimum (CSS): an empty cell with
    // `height:Npx` must still occupy that height rather than collapsing.
    table_cell_intrinsic_content_height(cell).max(cell.min_content_height)
}

/// The cell's *actual* content height (padding + text + nested content), WITHOUT
/// the `min_content_height` floor. Used to position content within a taller cell
/// (e.g. `vertical-align` offset), where the real content extent is needed
/// rather than the cell's full height.
pub(crate) fn table_cell_intrinsic_content_height(cell: &TableCell) -> f32 {
    let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
    let nested_h: f32 = cell.nested_rows.iter().map(estimate_element_height).sum();
    cell.padding_top + text_h + nested_h + cell.padding_bottom
}

/// Parse a width for a `<col>` / `<colgroup>` element.
///
/// Valid inline `width` declarations take precedence. Malformed inline
/// declarations are ignored so the `width` attribute can still act as a
/// fallback. `width: auto` explicitly clears the width.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TableTrackWidth {
    Points(f32),
    Percent(f32),
}

#[derive(Debug, Clone, Default)]
struct TableColumnInfo {
    background_color: Option<(f32, f32, f32, f32)>,
    collapsed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct HiddenBorderSides {
    top: bool,
    right: bool,
    bottom: bool,
    left: bool,
}

#[derive(Debug, Clone, Copy)]
enum BorderSideName {
    Right,
    Left,
}

#[derive(Debug, Clone, Copy)]
struct BorderCandidate {
    side: LayoutBorderSide,
    hidden: bool,
}

fn resolve_table_percentage_width(table_width: f32, percent: f32) -> f32 {
    // Percentage `<col>` and `<colgroup>` widths resolve against the table
    // width itself. Border-spacing is applied later when laying out the cells
    // so it must not shrink the percentage basis.
    table_width * percent
}

fn style_background_rgba(style: &ComputedStyle) -> Option<(f32, f32, f32, f32)> {
    style.background_color.map(|c| c.to_f32_rgba())
}

fn table_cell_is_hidden(style: &ComputedStyle) -> bool {
    style.visibility != Visibility::Visible
}

fn hide_table_cell_paint(cell: &mut TableCell) {
    cell.min_content_height = cell.min_content_height.max(table_cell_content_height(cell));
    cell.lines.clear();
    cell.nested_rows.clear();
    cell.background_color = None;
    cell.border = LayoutBorder::default();
    cell.background_gradient = None;
    cell.background_radial_gradient = None;
    cell.background_conic_gradient = None;
}

fn css_value_contains_hidden(value: &CssValue) -> bool {
    match value {
        CssValue::Keyword(raw) => raw
            .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '/' | ','))
            .any(|token| token.eq_ignore_ascii_case("hidden")),
        _ => false,
    }
}

fn mark_hidden_border_property(sides: &mut HiddenBorderSides, property: &str, value: &CssValue) {
    if !css_value_contains_hidden(value) {
        return;
    }
    match property {
        "border" | "border-style" => {
            sides.top = true;
            sides.right = true;
            sides.bottom = true;
            sides.left = true;
        }
        "border-top" | "border-top-style" => sides.top = true,
        "border-right" | "border-right-style" => sides.right = true,
        "border-bottom" | "border-bottom-style" => sides.bottom = true,
        "border-left" | "border-left-style" => sides.left = true,
        _ => {}
    }
}

fn declared_hidden_borders(
    el: &ElementNode,
    rules: &[CssRule],
    selector_ctx: &SelectorContext,
) -> HiddenBorderSides {
    let classes = el.class_list();
    let mut sides = HiddenBorderSides::default();
    let mut best_specificity: HashMap<&'static str, u32> = HashMap::new();
    for rule in rules.iter().filter(|rule| rule.pseudo_element.is_none()) {
        if !selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &classes,
            el.id(),
            &el.attributes,
            selector_ctx,
        ) {
            continue;
        }
        let rule_specificity = specificity(&rule.selector);
        for (property, value) in &rule.declarations.properties {
            let side_key = match property.as_str() {
                "border" | "border-style" => "all",
                "border-top" | "border-top-style" => "top",
                "border-right" | "border-right-style" => "right",
                "border-bottom" | "border-bottom-style" => "bottom",
                "border-left" | "border-left-style" => "left",
                _ => continue,
            };
            if rule_specificity >= best_specificity.get(side_key).copied().unwrap_or(0) {
                best_specificity.insert(side_key, rule_specificity);
                mark_hidden_border_property(&mut sides, property, value);
            }
        }
    }
    if let Some(inline) = el.style_attr().map(crate::parser::css::parse_inline_style) {
        for (property, value) in &inline.properties {
            mark_hidden_border_property(&mut sides, property, value);
        }
    }
    sides
}

fn hidden_for_side(hidden: HiddenBorderSides, side: BorderSideName) -> bool {
    match side {
        BorderSideName::Right => hidden.right,
        BorderSideName::Left => hidden.left,
    }
}

fn border_side(border: &LayoutBorder, side: BorderSideName) -> LayoutBorderSide {
    match side {
        BorderSideName::Right => border.right,
        BorderSideName::Left => border.left,
    }
}

fn border_side_mut(border: &mut LayoutBorder, side: BorderSideName) -> &mut LayoutBorderSide {
    match side {
        BorderSideName::Right => &mut border.right,
        BorderSideName::Left => &mut border.left,
    }
}

fn collapsed_style_rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::Double => 4,
        BorderStyle::Solid => 3,
        BorderStyle::Dashed => 2,
        BorderStyle::Dotted => 1,
        BorderStyle::None => 0,
    }
}

fn collapsed_border_winner(first: BorderCandidate, second: BorderCandidate) -> Option<usize> {
    if first.hidden || second.hidden {
        return None;
    }
    let first_paints = first.side.width > 0.0 && first.side.style != BorderStyle::None;
    let second_paints = second.side.width > 0.0 && second.side.style != BorderStyle::None;
    match (first_paints, second_paints) {
        (false, false) => return None,
        (true, false) => return Some(0),
        (false, true) => return Some(1),
        (true, true) => {}
    }
    let width_cmp = first
        .side
        .width
        .partial_cmp(&second.side.width)
        .unwrap_or(std::cmp::Ordering::Equal);
    match width_cmp {
        std::cmp::Ordering::Greater => Some(0),
        std::cmp::Ordering::Less => Some(1),
        std::cmp::Ordering::Equal => {
            let first_rank = collapsed_style_rank(first.side.style);
            let second_rank = collapsed_style_rank(second.side.style);
            if first_rank > second_rank {
                Some(0)
            } else if second_rank > first_rank {
                Some(1)
            } else {
                Some(0)
            }
        }
    }
}

fn resolve_collapsed_border_pair(
    first_border: &mut LayoutBorder,
    first_hidden: HiddenBorderSides,
    first_side: BorderSideName,
    second_border: &mut LayoutBorder,
    second_hidden: HiddenBorderSides,
    second_side: BorderSideName,
) {
    let first = BorderCandidate {
        side: border_side(first_border, first_side),
        hidden: hidden_for_side(first_hidden, first_side),
    };
    let second = BorderCandidate {
        side: border_side(second_border, second_side),
        hidden: hidden_for_side(second_hidden, second_side),
    };
    match collapsed_border_winner(first, second) {
        Some(0) => {
            *border_side_mut(first_border, first_side) = LayoutBorderSide::default();
            *border_side_mut(second_border, second_side) = first.side;
        }
        Some(1) => {
            *border_side_mut(first_border, first_side) = LayoutBorderSide::default();
            *border_side_mut(second_border, second_side) = second.side;
        }
        _ => {
            *border_side_mut(first_border, first_side) = LayoutBorderSide::default();
            *border_side_mut(second_border, second_side) = LayoutBorderSide::default();
        }
    }
}

fn apply_table_winning_side(cell_side: &mut LayoutBorderSide, table_side: LayoutBorderSide) {
    if table_side.width > 0.0
        && table_side.style != BorderStyle::None
        && table_side.width >= cell_side.width
    {
        *cell_side = table_side;
        cell_side.width *= 2.0;
        cell_side.alpha = 0.0;
    }
}

fn table_side_wins(cell_side: LayoutBorderSide, table_side: LayoutBorderSide) -> bool {
    table_side.width > 0.0
        && table_side.style != BorderStyle::None
        && table_side.width >= cell_side.width
}

fn resolve_collapsed_outer_table_borders(rows: &mut [LayoutElement], table_border: LayoutBorder) {
    if !table_border.has_any() {
        return;
    }
    let mut first_row_idx = None;
    let mut last_row_idx = None;
    for (idx, row) in rows.iter().enumerate() {
        if matches!(row, LayoutElement::TableRow { cells, .. } if !cells.is_empty()) {
            first_row_idx.get_or_insert(idx);
            last_row_idx = Some(idx);
        }
    }
    for row in rows.iter_mut() {
        let LayoutElement::TableRow { cells, .. } = row else {
            continue;
        };
        if let Some(first_cell) = cells.iter_mut().find(|cell| cell.rowspan != 0) {
            apply_table_winning_side(&mut first_cell.border.left, table_border.left);
        }
        if let Some(last_cell) = cells.iter_mut().rev().find(|cell| cell.rowspan != 0) {
            apply_table_winning_side(&mut last_cell.border.right, table_border.right);
        }
    }
    if let Some(idx) = first_row_idx {
        if let LayoutElement::TableRow { cells, .. } = &mut rows[idx] {
            for cell in cells.iter_mut().filter(|cell| cell.rowspan != 0) {
                let top_wins = table_side_wins(cell.border.top, table_border.top);
                apply_table_winning_side(&mut cell.border.top, table_border.top);
                if top_wins {
                    cell.min_content_height =
                        (cell.min_content_height - table_border.top.width / 2.0).max(0.0);
                }
            }
        }
    }
    if let Some(idx) = last_row_idx {
        if let LayoutElement::TableRow { cells, .. } = &mut rows[idx] {
            for cell in cells.iter_mut().filter(|cell| cell.rowspan != 0) {
                let bottom_wins = table_side_wins(cell.border.bottom, table_border.bottom);
                apply_table_winning_side(&mut cell.border.bottom, table_border.bottom);
                if bottom_wins {
                    cell.min_content_height =
                        (cell.min_content_height - table_border.bottom.width / 2.0).max(0.0);
                }
            }
        }
    }
}

impl TableTrackWidth {
    fn resolve(self, table_width: f32) -> f32 {
        match self {
            Self::Points(width) => width,
            Self::Percent(percent) => resolve_table_percentage_width(table_width, percent),
        }
    }
}

fn compute_column_style(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> ComputedStyle {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    )
}

fn parse_element_width(el: &ElementNode) -> Option<TableTrackWidth> {
    if let Some(inline_width) = parse_element_inline_width(el) {
        return inline_width;
    }
    el.attributes
        .get("width")
        .and_then(|val| parse_table_track_width(val))
}

fn parse_element_inline_width(el: &ElementNode) -> Option<Option<TableTrackWidth>> {
    if let Some(style_str) = el.style_attr() {
        let mut last_inline_width = None;
        for decl in style_str.split(';').map(str::trim) {
            if let Some((prop, val)) = decl.split_once(':') {
                if prop.trim().eq_ignore_ascii_case("width") {
                    let val = strip_important(val).trim();
                    last_inline_width = parse_inline_width_value(val).or(last_inline_width);
                }
            }
        }
        return last_inline_width;
    }
    None
}

fn parse_col_width(
    col_el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> Option<TableTrackWidth> {
    let computed_style = compute_column_style(
        col_el,
        parent_style,
        rules,
        ancestors,
        child_index,
        sibling_count,
    );
    if let Some(inline_width) = parse_column_inline_width(col_el, computed_style.width) {
        return inline_width;
    }
    computed_style
        .width
        .map(TableTrackWidth::Points)
        .or_else(|| {
            col_el
                .attributes
                .get("width")
                .and_then(|val| parse_table_track_width(val))
        })
}

fn parse_column_inline_width(
    el: &ElementNode,
    computed_width: Option<f32>,
) -> Option<Option<TableTrackWidth>> {
    let style_str = el.style_attr()?;
    let inline = crate::parser::css::parse_inline_style(style_str);
    match inline.get("width") {
        Some(CssValue::Keyword(k)) if k.eq_ignore_ascii_case("auto") => Some(None),
        Some(_) => computed_width.map(|width| Some(TableTrackWidth::Points(width))),
        None => None,
    }
}

fn parse_percent_width(val: &str) -> Option<f32> {
    let pct_str = val.trim().strip_suffix('%')?;
    pct_str.trim().parse::<f32>().ok().map(|pct| pct / 100.0)
}

fn parse_table_track_width(val: &str) -> Option<TableTrackWidth> {
    if let Some(percent) = parse_percent_width(val) {
        return Some(TableTrackWidth::Percent(percent));
    }
    match crate::parser::css::parse_length(val) {
        Some(CssValue::Length(width)) => Some(TableTrackWidth::Points(width)),
        _ => None,
    }
}

fn parse_inline_width_value(val: &str) -> Option<Option<TableTrackWidth>> {
    if val.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    parse_table_track_width(val).map(Some).or_else(|| {
        crate::parser::css::parse_length(val)
            .is_some()
            .then_some(None)
    })
}

fn strip_important(val: &str) -> &str {
    val.strip_suffix("!important")
        .map(str::trim_end)
        .unwrap_or(val)
}

fn parse_col_span(el: &ElementNode) -> usize {
    el.attributes
        .get("span")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_COLSPAN)
}

fn parse_cell_colspan(el: &ElementNode) -> usize {
    el.attributes
        .get("colspan")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_COLSPAN)
}

fn parse_cell_rowspan(el: &ElementNode, remaining_rows_in_group: usize) -> usize {
    let parsed = el
        .attributes
        .get("rowspan")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let span = if parsed == 0 {
        remaining_rows_in_group.max(1)
    } else {
        parsed.clamp(1, MAX_ROWSPAN)
    };
    span.min(remaining_rows_in_group.max(1))
}

fn parse_table_attr_length(el: &ElementNode, attr: &str) -> Option<f32> {
    let value = el.attributes.get(attr)?.trim();
    if value.is_empty() {
        return None;
    }
    let parsed = if let Ok(number) = value.parse::<f32>() {
        number * 0.75
    } else {
        match crate::parser::css::parse_length(value) {
            Some(CssValue::Length(length)) => length,
            _ => return None,
        }
    };
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn attr_border(width: f32, light: bool) -> LayoutBorder {
    let light_color = (0.88, 0.88, 0.88);
    let dark_color = (0.55, 0.55, 0.55);
    let side = |color| super::engine::LayoutBorderSide {
        width,
        color,
        alpha: 1.0,
        style: BorderStyle::Solid,
    };
    LayoutBorder {
        top: side(if light { light_color } else { dark_color }),
        right: side(if light { dark_color } else { light_color }),
        bottom: side(if light { dark_color } else { light_color }),
        left: side(if light { light_color } else { dark_color }),
    }
}

fn apply_cell_attribute_hints(
    style: &mut ComputedStyle,
    cellpadding: Option<f32>,
    border_width: Option<f32>,
) {
    if let Some(padding) = cellpadding {
        if style.padding.top <= 1.0
            && style.padding.right <= 1.0
            && style.padding.bottom <= 1.0
            && style.padding.left <= 1.0
        {
            style.padding.top = padding;
            style.padding.right = padding;
            style.padding.bottom = padding;
            style.padding.left = padding;
        }
    }

    if let Some(width) = border_width {
        if style.border.top.width == 0.0
            && style.border.right.width == 0.0
            && style.border.bottom.width == 0.0
            && style.border.left.width == 0.0
        {
            let side = crate::style::computed::BorderSide {
                width: (width / 2.0).max(1.0),
                color: Some(crate::types::Color::rgb(128, 128, 128)),
                style: BorderStyle::Solid,
            };
            style.border.top = side;
            style.border.right = side;
            style.border.bottom = side;
            style.border.left = side;
        }
    }
}

fn table_section_key(el: Option<&ElementNode>, child_index: usize) -> usize {
    el.map_or(child_index, |section| {
        section as *const ElementNode as usize
    })
}

fn distribute_extra_width(widths: &mut [f32], extra: f32) {
    if widths.is_empty() || extra <= 0.0 {
        return;
    }
    let total: f32 = widths.iter().sum();
    if total > 0.0 {
        for width in widths {
            *width += extra * (*width / total);
        }
    } else {
        let per_col = extra / widths.len() as f32;
        for width in widths {
            *width += per_col;
        }
    }
}

fn row_height_from_cells(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(table_cell_content_height)
        .fold(0.0f32, f32::max)
}

fn enforce_row_min_height(cells: &mut [TableCell], min_height: f32) {
    if min_height <= 0.0 {
        return;
    }
    let current = row_height_from_cells(cells);
    if current >= min_height {
        return;
    }
    if let Some(cell) = cells.iter_mut().find(|cell| cell.rowspan != 0) {
        cell.min_content_height = cell.min_content_height.max(min_height);
    } else if let Some(cell) = cells.first_mut() {
        cell.min_content_height = cell.min_content_height.max(min_height);
    }
}

fn stretch_table_rows_to_min_height(
    output: &mut [LayoutElement],
    target_table_height: f32,
    vertical_edge_spacing: f32,
) {
    if target_table_height <= 0.0 {
        return;
    }
    let mut row_indices = Vec::new();
    let mut rows_height = 0.0f32;
    for (idx, elem) in output.iter().enumerate() {
        if let LayoutElement::TableRow { cells, .. } = elem {
            row_indices.push(idx);
            rows_height += row_height_from_cells(cells);
        }
    }
    if row_indices.is_empty() {
        return;
    }
    let target_rows_height =
        (target_table_height - vertical_edge_spacing * (row_indices.len() + 1) as f32).max(0.0);
    if rows_height >= target_rows_height {
        return;
    }
    let extra_per_row = (target_rows_height - rows_height) / row_indices.len() as f32;
    for idx in row_indices {
        if let LayoutElement::TableRow { cells, .. } = &mut output[idx] {
            let target_row_height = row_height_from_cells(cells) + extra_per_row;
            enforce_row_min_height(cells, target_row_height);
        }
    }
}

fn table_cell_vertical_align(value: VerticalAlign) -> VerticalAlign {
    match value {
        VerticalAlign::Middle | VerticalAlign::Bottom | VerticalAlign::Top => value,
        _ => VerticalAlign::Top,
    }
}

fn estimate_run_text_width(run: &TextRun, text: &str, fonts: &HashMap<String, TtfFont>) -> f32 {
    estimate_word_width(
        text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.italic,
        fonts,
    )
}

fn clip_text_lines_to_width(
    lines: &mut [TextLine],
    max_width: f32,
    fonts: &HashMap<String, TtfFont>,
) {
    if max_width <= 0.0 {
        for line in lines {
            line.runs.clear();
        }
        return;
    }

    for line in lines {
        let mut used = 0.0f32;
        let mut clipped_runs = Vec::new();
        for run in &line.runs {
            let run_width = estimate_run_text_width(run, &run.text, fonts);
            if used + run_width <= max_width {
                used += run_width;
                clipped_runs.push(run.clone());
                continue;
            }

            let mut clipped_text = String::new();
            for ch in run.text.chars() {
                let mut candidate = clipped_text.clone();
                candidate.push(ch);
                let candidate_width = estimate_run_text_width(run, &candidate, fonts);
                if used + candidate_width > max_width {
                    if !clipped_text.is_empty() {
                        clipped_text.push(ch);
                    }
                    break;
                }
                clipped_text.push(ch);
            }
            if !clipped_text.is_empty() {
                let mut clipped = run.clone();
                clipped.text = clipped_text;
                clipped_runs.push(clipped);
            }
            break;
        }
        line.runs = clipped_runs;
    }
}

fn compute_table_column_count(
    rows: &[&ElementNode],
    row_section_indices: &[usize],
    row_section_sizes: &[usize],
    row_section_elements: &[Option<&ElementNode>],
    row_section_child_indices: &[usize],
) -> usize {
    let mut max_cols = 0usize;
    let mut occupied: Vec<usize> = Vec::new();
    let mut current_section = None;

    for (row_idx, row) in rows.iter().enumerate() {
        let section_key = table_section_key(
            row_section_elements[row_idx],
            row_section_child_indices[row_idx],
        );
        if current_section != Some(section_key) {
            occupied.clear();
            current_section = Some(section_key);
        }

        let mut next_occupied = vec![0usize; occupied.len()];
        let mut col_pos = 0usize;
        let remaining_rows =
            row_section_sizes[row_idx].saturating_sub(row_section_indices[row_idx]);

        for child in &row.children {
            let DomNode::Element(cell_el) = child else {
                continue;
            };
            if cell_el.tag != HtmlTag::Td && cell_el.tag != HtmlTag::Th {
                continue;
            }
            while occupied.get(col_pos).copied().unwrap_or(0) > 0 {
                if col_pos >= next_occupied.len() {
                    next_occupied.resize(col_pos + 1, 0);
                }
                next_occupied[col_pos] = occupied[col_pos].saturating_sub(1);
                col_pos += 1;
            }

            let colspan = parse_cell_colspan(cell_el);
            let rowspan = parse_cell_rowspan(cell_el, remaining_rows);
            let end = col_pos.saturating_add(colspan);
            if end > next_occupied.len() {
                next_occupied.resize(end, 0);
            }
            if rowspan > 1 {
                for slot in next_occupied.iter_mut().skip(col_pos).take(colspan) {
                    *slot = rowspan - 1;
                }
            }
            col_pos = end;
            max_cols = max_cols.max(col_pos);
        }

        for (col, remaining) in occupied.iter().enumerate().skip(col_pos) {
            if *remaining > 0 {
                if col >= next_occupied.len() {
                    next_occupied.resize(col + 1, 0);
                }
                next_occupied[col] = remaining.saturating_sub(1);
                max_cols = max_cols.max(col + 1);
            }
        }
        occupied = next_occupied;
    }

    max_cols.max(1)
}

fn compute_caption_style(
    caption_el: &ElementNode,
    caption_child_idx: usize,
    section_count: usize,
    table_style: &ComputedStyle,
    table_ancestors: &[AncestorInfo],
    rules: &[CssRule],
) -> ComputedStyle {
    let caption_classes = caption_el.class_list();
    let caption_ctx = SelectorContext {
        ancestors: table_ancestors.to_vec(),
        child_index: caption_child_idx,
        sibling_count: section_count,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    compute_style_with_context(
        caption_el.tag,
        caption_el.style_attr(),
        table_style,
        rules,
        caption_el.tag_name(),
        &caption_classes,
        caption_el.id(),
        &caption_el.attributes,
        &caption_ctx,
    )
}

#[allow(clippy::too_many_arguments)]
fn measure_caption_min_width(
    caption_el: &ElementNode,
    caption_child_idx: usize,
    section_count: usize,
    caption_style: &ComputedStyle,
    table_ancestors: &[AncestorInfo],
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    filter_defs: &HashMap<String, ElementNode>,
    counter_state: &mut CounterState,
    available_width: f32,
) -> f32 {
    let mut caption_ancestors = table_ancestors.to_vec();
    caption_ancestors.push(AncestorInfo {
        element: caption_el,
        child_index: caption_child_idx,
        sibling_count: section_count,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });
    let mut runs = Vec::new();
    let mut nested = Vec::new();
    collect_table_cell_content_inner(
        &caption_el.children,
        caption_style,
        &mut runs,
        &mut nested,
        None,
        rules,
        fonts,
        filter_defs,
        false,
        false,
        true,
        &caption_ancestors,
        available_width.max(1.0),
        counter_state,
    );
    let text_width: f32 = runs
        .iter()
        .map(|run| {
            estimate_word_width(
                &run.text,
                run.font_size,
                &run.font_family,
                run.bold,
                run.italic,
                fonts,
            )
        })
        .sum();
    let nested_width = nested
        .iter()
        .map(nested_element_preferred_width)
        .fold(0.0f32, f32::max);
    let caption_border = LayoutBorder::from_computed(&caption_style.border);
    text_width.max(nested_width)
        + caption_style.padding.left
        + caption_style.padding.right
        + caption_border.horizontal_width()
}

fn assign_explicit_col_widths(
    explicit_col_widths: &mut [Option<TableTrackWidth>],
    col_idx: &mut usize,
    span: usize,
    width: Option<TableTrackWidth>,
) {
    for slot in explicit_col_widths.iter_mut().skip(*col_idx).take(span) {
        *slot = width;
    }
    *col_idx = col_idx.saturating_add(span);
}

fn resolve_table_inner_width(style: &ComputedStyle, available_width: f32) -> f32 {
    let containing_width = (available_width - style.margin.left - style.margin.right).max(0.0);
    style
        .width
        .or_else(|| {
            style
                .percentage_sizing
                .width
                .map(|percent| containing_width * percent / 100.0)
        })
        .map_or(containing_width, |width| {
            width.min(containing_width).max(0.0)
        })
}

fn uses_fixed_table_layout(style: &ComputedStyle) -> bool {
    style.table_layout == TableLayout::Fixed
        && (style.width.is_some() || style.percentage_sizing.width.is_some())
}

fn resolve_cell_track_width(
    cell_el: &ElementNode,
    cell_style: &ComputedStyle,
    table_width: f32,
) -> Option<f32> {
    parse_element_width(cell_el)
        .map(|width| width.resolve(table_width))
        .or(cell_style.width)
}

fn apply_cell_width_to_columns(
    col_widths: &mut [Option<f32>],
    start: usize,
    colspan: usize,
    width: f32,
) {
    if colspan == 0 || start >= col_widths.len() {
        return;
    }
    let per_column_width = width / colspan as f32;
    for slot in col_widths.iter_mut().skip(start).take(colspan) {
        *slot = Some(slot.map_or(per_column_width, |existing| existing.max(per_column_width)));
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_fixed_table_columns(
    table_style: &ComputedStyle,
    table_width: f32,
    rows: &[&ElementNode],
    row_section_indices: &[usize],
    row_section_sizes: &[usize],
    row_section_elements: &[Option<&ElementNode>],
    row_section_child_indices: &[usize],
    row_section_sibling_counts: &[usize],
    table_ancestors: &[AncestorInfo],
    explicit_col_widths: &[Option<TableTrackWidth>],
    num_cols: usize,
    rules: &[CssRule],
) -> Vec<f32> {
    let mut col_widths: Vec<Option<f32>> = explicit_col_widths
        .iter()
        .map(|width| width.map(|specified| specified.resolve(table_width)))
        .collect();

    if let Some(first_row) = rows.first() {
        let mut row_ancestors = table_ancestors.to_vec();
        if let Some(section_el) = row_section_elements.first().copied().flatten() {
            row_ancestors.push(AncestorInfo {
                element: section_el,
                child_index: row_section_child_indices.first().copied().unwrap_or(0),
                sibling_count: row_section_sibling_counts.first().copied().unwrap_or(0),
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
        }
        let row_selector_ctx = SelectorContext {
            ancestors: row_ancestors,
            child_index: row_section_indices.first().copied().unwrap_or(0),
            sibling_count: row_section_sizes.first().copied().unwrap_or(1),
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        };
        let row_classes = first_row.class_list();
        let mut row_style = compute_style_with_context(
            first_row.tag,
            first_row.style_attr(),
            table_style,
            rules,
            first_row.tag_name(),
            &row_classes,
            first_row.id(),
            &first_row.attributes,
            &row_selector_ctx,
        );
        row_style.width = Some(table_width);

        let mut col_pos = 0usize;
        for child in &first_row.children {
            let DomNode::Element(cell_el) = child else {
                continue;
            };
            if cell_el.tag != HtmlTag::Td && cell_el.tag != HtmlTag::Th {
                continue;
            }
            let colspan = parse_cell_colspan(cell_el);

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            cell_ancestors.push(AncestorInfo {
                element: first_row,
                child_index: row_selector_ctx.child_index,
                sibling_count: row_selector_ctx.sibling_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            let cell_selector_ctx = SelectorContext {
                ancestors: cell_ancestors,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let cell_style = compute_style_with_context(
                cell_el.tag,
                cell_el.style_attr(),
                &row_style,
                rules,
                cell_el.tag_name(),
                &cell_classes,
                cell_el.id(),
                &cell_el.attributes,
                &cell_selector_ctx,
            );

            if let Some(width) = resolve_cell_track_width(cell_el, &cell_style, table_width) {
                apply_cell_width_to_columns(&mut col_widths, col_pos, colspan, width);
            }

            col_pos = col_pos.saturating_add(colspan);
            if col_pos >= num_cols {
                break;
            }
        }
    }

    let assigned_width: f32 = col_widths.iter().flatten().copied().sum();
    let unresolved_count = col_widths.iter().filter(|width| width.is_none()).count();
    if unresolved_count > 0 {
        let remaining_width = (table_width - assigned_width).max(0.0);
        let default_width = remaining_width / unresolved_count as f32;
        for width in &mut col_widths {
            if width.is_none() {
                *width = Some(default_width);
            }
        }
    }

    let mut resolved_widths: Vec<f32> = col_widths
        .into_iter()
        .map(|width| width.unwrap_or(0.0))
        .collect();
    let resolved_total: f32 = resolved_widths.iter().sum();
    let used_table_width = table_width.max(resolved_total);
    if used_table_width > resolved_total && !resolved_widths.is_empty() {
        let extra = used_table_width - resolved_total;
        // When every column already has a width but the table is wider than
        // their sum, the surplus is distributed *proportionally* to each
        // column's existing width (Blink's FixedTableLayout). A colspan=2 cell
        // declaring width:120 seeds its two columns with 60 each; a sibling
        // single cell declaring 120 seeds its column with 120. Proportional
        // spreading then yields 90/90/180 from a 360px table — matching Chrome
        // — whereas an equal split would wrongly give 100/100/160. Columns with
        // zero width (no contribution) fall back to an equal split so an empty
        // first row still produces sensible widths.
        if resolved_total > 0.0 {
            for width in &mut resolved_widths {
                *width += extra * (*width / resolved_total);
            }
        } else {
            let extra_per_column = extra / resolved_widths.len() as f32;
            for width in &mut resolved_widths {
                *width += extra_per_column;
            }
        }
    }

    if resolved_widths.iter().all(|width| *width <= 0.0) && num_cols > 0 {
        return vec![table_width / num_cols as f32; num_cols];
    }

    resolved_widths
}

/// The collapsed table's outer left/right border widths, taken from the left
/// border of the first cell and the right border of the last cell in the first
/// row. Returns `(0.0, 0.0)` when there are no cells. Used to shrink the column
/// tracks so the outer borders fit inside the table's declared width
/// (`border-collapse: collapse`).
fn collapse_outer_horizontal_borders(
    rows: &[&ElementNode],
    table_style: &ComputedStyle,
    rules: &[CssRule],
    table_ancestors: &[AncestorInfo],
) -> (f32, f32) {
    if table_style.border_collapse != BorderCollapse::Collapse {
        return (0.0, 0.0);
    }
    let Some(first_row) = rows.first() else {
        return (0.0, 0.0);
    };
    let cells: Vec<&ElementNode> = first_row
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(e) if e.tag == HtmlTag::Td || e.tag == HtmlTag::Th => Some(e),
            _ => None,
        })
        .collect();
    if cells.is_empty() {
        return (0.0, 0.0);
    }
    let row_classes = first_row.class_list();
    let row_style = compute_style_with_context(
        first_row.tag,
        first_row.style_attr(),
        table_style,
        rules,
        first_row.tag_name(),
        &row_classes,
        first_row.id(),
        &first_row.attributes,
        &SelectorContext {
            ancestors: table_ancestors.to_vec(),
            child_index: 0,
            sibling_count: rows.len(),
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        },
    );
    let cell_count = cells.len();
    let cell_border = |idx: usize, cell: &ElementNode| -> ComputedStyle {
        let classes = cell.class_list();
        compute_style_with_context(
            cell.tag,
            cell.style_attr(),
            &row_style,
            rules,
            cell.tag_name(),
            &classes,
            cell.id(),
            &cell.attributes,
            &SelectorContext {
                ancestors: table_ancestors.to_vec(),
                child_index: idx,
                sibling_count: cell_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            },
        )
    };
    let first = cell_border(0, cells[0]);
    let last = cell_border(cell_count - 1, cells[cell_count - 1]);
    (
        first.border.left.width.max(table_style.border.left.width),
        last.border.right.width.max(table_style.border.right.width),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn flatten_table(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    table_child_index: usize,
    table_sibling_count: usize,
    env: &mut LayoutEnv,
) {
    let rules = env.rules;
    let fonts = env.fonts;
    let filter_defs = env.filter_defs;
    let counter_state = &mut *env.counter_state;
    let inner_width = resolve_table_inner_width(style, available_width);
    let table_attr_border_width =
        parse_table_attr_length(el, "border").filter(|width| *width > 0.0);
    let table_attr_cellpadding = parse_table_attr_length(el, "cellpadding");
    let table_attr_cellspacing = parse_table_attr_length(el, "cellspacing");
    let legacy_border_spacing =
        table_attr_cellspacing.or_else(|| table_attr_border_width.is_some().then_some(2.0 * 0.75));
    let effective_border_spacing = if style.border_spacing == 0.0 {
        legacy_border_spacing.unwrap_or(style.border_spacing)
    } else {
        style.border_spacing
    };
    let effective_border_spacing_vertical = if style.border_spacing_vertical == 0.0 {
        legacy_border_spacing.unwrap_or(style.border_spacing_vertical)
    } else {
        style.border_spacing_vertical
    };
    let table_grid_inset = table_attr_border_width.unwrap_or(0.0);

    // Build ancestor chain: everything above + the table element itself.
    let mut table_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    table_ancestors.push(AncestorInfo {
        element: el,
        child_index: table_child_index,
        sibling_count: table_sibling_count,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });

    // Collect all <tr> elements (from direct children, thead, tbody, tfoot).
    // Track section-relative indices so nth-child counts within each section
    // (thead, tbody, tfoot) as browsers do, not globally.
    // Also track the section element so descendant selectors can see it.
    let mut rows: Vec<&ElementNode> = Vec::new();
    let mut row_section_indices: Vec<usize> = Vec::new();
    let mut row_section_sizes: Vec<usize> = Vec::new();
    let mut row_section_elements: Vec<Option<&ElementNode>> = Vec::new();
    let mut row_section_child_indices: Vec<usize> = Vec::new();
    let mut row_section_sibling_counts: Vec<usize> = Vec::new();
    // First `<caption>` (caption-side:top is the only supported placement) and
    // its position among the table's element children, for selector matching.
    let mut caption: Option<(&ElementNode, usize)> = None;
    let section_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();
    for (section_child_idx, child) in el.children.iter().enumerate() {
        if let DomNode::Element(child_el) = child {
            match child_el.tag {
                HtmlTag::Caption => {
                    if caption.is_none() {
                        caption = Some((child_el, section_child_idx));
                    }
                }
                HtmlTag::Tr => {
                    // Direct <tr> child of <table> — standalone section
                    let idx = rows.len();
                    rows.push(child_el);
                    row_section_indices.push(idx);
                    row_section_sizes.push(1);
                    row_section_elements.push(None);
                    row_section_child_indices.push(section_child_idx);
                    row_section_sibling_counts.push(section_count);
                }
                HtmlTag::Thead | HtmlTag::Tbody | HtmlTag::Tfoot => {
                    let section_rows: Vec<&ElementNode> = child_el
                        .children
                        .iter()
                        .filter_map(|gc| {
                            if let DomNode::Element(g) = gc {
                                if g.tag == HtmlTag::Tr {
                                    return Some(g);
                                }
                            }
                            None
                        })
                        .collect();
                    let section_size = section_rows.len();
                    for (i, gc) in section_rows.into_iter().enumerate() {
                        rows.push(gc);
                        row_section_indices.push(i);
                        row_section_sizes.push(section_size);
                        row_section_elements.push(Some(child_el));
                        row_section_child_indices.push(section_child_idx);
                        row_section_sibling_counts.push(section_count);
                    }
                }
                _ => {}
            }
        }
    }

    if rows.is_empty() {
        return;
    }

    // Reorder the collected rows into section order thead -> tbody (and direct
    // `<tr>`) -> tfoot, regardless of DOM source order (CSS 2.1 §17.2.1: the
    // table-header-group always renders first and the table-footer-group last,
    // so a `<tfoot>` written before `<tbody>` in the markup still renders at the
    // bottom — matching Chrome). The sort is stable, so rows within each section
    // keep their order and a table already in thead->tbody->tfoot order is
    // unchanged (no layout/selector difference for the common case). The
    // per-row section metadata travels with each row, so nth-child / descendant
    // selector matching is unaffected.
    let section_rank = |sec: &Option<&ElementNode>| -> u8 {
        match sec {
            Some(s) if s.tag == HtmlTag::Thead => 0,
            Some(s) if s.tag == HtmlTag::Tfoot => 2,
            _ => 1, // tbody and standalone <tr>
        }
    };
    if (0..rows.len()).any(|i| {
        i > 0 && section_rank(&row_section_elements[i]) < section_rank(&row_section_elements[i - 1])
    }) {
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by_key(|&i| section_rank(&row_section_elements[i]));
        let apply =
            |v: &[usize], src: &Vec<usize>| -> Vec<usize> { v.iter().map(|&i| src[i]).collect() };
        rows = order.iter().map(|&i| rows[i]).collect();
        row_section_indices = apply(&order, &row_section_indices);
        row_section_sizes = apply(&order, &row_section_sizes);
        row_section_elements = order.iter().map(|&i| row_section_elements[i]).collect();
        row_section_child_indices = apply(&order, &row_section_child_indices);
        row_section_sibling_counts = apply(&order, &row_section_sibling_counts);
    }

    let caption_style_for_layout = caption.map(|(caption_el, caption_child_idx)| {
        compute_caption_style(
            caption_el,
            caption_child_idx,
            section_count,
            style,
            &table_ancestors,
            rules,
        )
    });
    let caption_min_width = if let (Some((caption_el, caption_child_idx)), Some(caption_style)) =
        (caption, caption_style_for_layout.as_ref())
    {
        measure_caption_min_width(
            caption_el,
            caption_child_idx,
            section_count,
            caption_style,
            &table_ancestors,
            rules,
            fonts,
            filter_defs,
            counter_state,
            inner_width,
        )
    } else {
        0.0
    };

    // Determine the table grid width with rowspan occupancy. A later row can
    // need extra columns when earlier rowspans occupy leading slots.
    let num_cols = compute_table_column_count(
        &rows,
        &row_section_indices,
        &row_section_sizes,
        &row_section_elements,
        &row_section_child_indices,
    );

    let mut column_parent_style = style.clone();
    column_parent_style.width = Some(inner_width);

    // --- Extract explicit column widths/backgrounds from <colgroup>/<col> elements ---
    let mut explicit_col_widths: Vec<Option<TableTrackWidth>> = vec![None; num_cols];
    let mut column_info: Vec<TableColumnInfo> = vec![TableColumnInfo::default(); num_cols];
    {
        let mut col_idx = 0usize;
        for (section_child_idx, child) in el.children.iter().enumerate() {
            if let DomNode::Element(child_el) = child {
                match child_el.tag {
                    HtmlTag::Colgroup => {
                        let cols: Vec<&ElementNode> = child_el
                            .children
                            .iter()
                            .filter_map(|gc| match gc {
                                DomNode::Element(g) if g.tag == HtmlTag::Col => Some(g),
                                _ => None,
                            })
                            .collect();
                        let colgroup_style = compute_column_style(
                            child_el,
                            &column_parent_style,
                            rules,
                            &table_ancestors,
                            section_child_idx,
                            section_count,
                        );
                        let colgroup_bg = style_background_rgba(&colgroup_style);
                        if !cols.is_empty() {
                            let mut colgroup_basis_style = colgroup_style.clone();
                            colgroup_basis_style.width = Some(inner_width);
                            let mut colgroup_ancestors = table_ancestors.clone();
                            colgroup_ancestors.push(AncestorInfo {
                                element: child_el,
                                child_index: section_child_idx,
                                sibling_count: section_count,
                                preceding_siblings: Vec::new(),
                                following_siblings: Vec::new(),
                                is_empty: false,
                            });
                            let col_sibling_count = cols.len();
                            for (col_child_idx, col_el) in cols.into_iter().enumerate() {
                                let span = parse_col_span(col_el);
                                let col_style = compute_column_style(
                                    col_el,
                                    &colgroup_basis_style,
                                    rules,
                                    &colgroup_ancestors,
                                    col_child_idx,
                                    col_sibling_count,
                                );
                                let col_bg = style_background_rgba(&col_style).or(colgroup_bg);
                                let collapsed = col_style.visibility == Visibility::Collapse;
                                for info in column_info.iter_mut().skip(col_idx).take(span) {
                                    info.background_color = col_bg;
                                    info.collapsed = collapsed;
                                }
                                assign_explicit_col_widths(
                                    &mut explicit_col_widths,
                                    &mut col_idx,
                                    span,
                                    parse_column_inline_width(col_el, col_style.width)
                                        .unwrap_or_else(|| {
                                            col_style.width.map(TableTrackWidth::Points).or_else(
                                                || {
                                                    col_el.attributes.get("width").and_then(|val| {
                                                        parse_table_track_width(val)
                                                    })
                                                },
                                            )
                                        }),
                                );
                            }
                            continue;
                        }
                        let span = parse_col_span(child_el);
                        for info in column_info.iter_mut().skip(col_idx).take(span) {
                            info.background_color = colgroup_bg;
                            info.collapsed = colgroup_style.visibility == Visibility::Collapse;
                        }
                        assign_explicit_col_widths(
                            &mut explicit_col_widths,
                            &mut col_idx,
                            span,
                            parse_col_width(
                                child_el,
                                &column_parent_style,
                                rules,
                                &table_ancestors,
                                section_child_idx,
                                section_count,
                            ),
                        );
                    }
                    HtmlTag::Col => {
                        let span = parse_col_span(child_el);
                        let col_style = compute_column_style(
                            child_el,
                            &column_parent_style,
                            rules,
                            &table_ancestors,
                            section_child_idx,
                            section_count,
                        );
                        let col_bg = style_background_rgba(&col_style);
                        let collapsed = col_style.visibility == Visibility::Collapse;
                        for info in column_info.iter_mut().skip(col_idx).take(span) {
                            info.background_color = col_bg;
                            info.collapsed = collapsed;
                        }
                        assign_explicit_col_widths(
                            &mut explicit_col_widths,
                            &mut col_idx,
                            span,
                            parse_column_inline_width(child_el, col_style.width).unwrap_or_else(
                                || {
                                    col_style.width.map(TableTrackWidth::Points).or_else(|| {
                                        child_el
                                            .attributes
                                            .get("width")
                                            .and_then(|val| parse_table_track_width(val))
                                    })
                                },
                            ),
                        );
                    }
                    _ => continue,
                }
            }
        }
    }
    let has_explicit_widths = explicit_col_widths.iter().any(|width| width.is_some());
    // A table with no explicit `width` shrinks to fit its content/column widths
    // (CSS auto table layout) instead of stretching to the containing block.
    let table_has_explicit_width = style.width.is_some() || style.percentage_sizing.width.is_some();
    // When `border-collapse: separate`, horizontal `border-spacing` is drawn between
    // every pair of adjacent cells AND on the outer edges, so the space available for
    // the N columns is `inner_width - (N+1) * border_spacing`. Without this reduction
    // the columns are distributed across the full width and the table overflows by
    // exactly `(N+1) * border_spacing` on the right.
    // For `border-collapse: collapse`, the cell borders are painted INSIDE the
    // table's border box (CSS2 §17.6.2): the table width includes them. ironpress
    // strokes borders centered on cell edges, so the column tracks (measured
    // center-to-center) span `width - outer_left/2 - outer_right/2`; the
    // `collapse_paint_offset` then nudges the painted table right by `outer_left/2`
    // so the outer border's outer pixel lands on the table box edge. Without this
    // reduction the columns sum to the full width and the table renders too wide.
    let (outer_left_border, outer_right_border) =
        collapse_outer_horizontal_borders(&rows, style, rules, &table_ancestors);
    let columns_width = if matches!(
        style.border_collapse,
        crate::style::computed::BorderCollapse::Separate
    ) && effective_border_spacing > 0.0
        && num_cols > 0
    {
        (inner_width - (num_cols as f32 + 1.0) * effective_border_spacing).max(0.0)
    } else if style.border_collapse == BorderCollapse::Collapse {
        (inner_width - outer_left_border / 2.0 - outer_right_border / 2.0).max(0.0)
    } else {
        inner_width
    };
    let mut col_widths: Vec<f32> = if uses_fixed_table_layout(style) {
        resolve_fixed_table_columns(
            style,
            columns_width,
            &rows,
            &row_section_indices,
            &row_section_sizes,
            &row_section_elements,
            &row_section_child_indices,
            &row_section_sibling_counts,
            &table_ancestors,
            &explicit_col_widths,
            num_cols,
            rules,
        )
    } else {
        // --- Auto-sizing pass: measure preferred content width for each column ---
        let min_col_width: f32 = 0.0;
        let mut preferred_widths: Vec<f32> = vec![0.0; num_cols];
        // Per-column MIN-content width (CSS2 §17.5.2): the narrowest the column
        // can be without overflowing its unbreakable content. For normal wrapping
        // that is the longest single word; for `white-space: nowrap`/`pre` the
        // content cannot wrap at all, so it is the full content width. The table's
        // used width is floored at the sum of these, so nowrap content overflows
        // an undersized declared `width` (Chrome) instead of being crushed.
        let mut min_widths: Vec<f32> = vec![0.0; num_cols];
        let mut spanning_widths: Vec<(usize, usize, f32, f32)> = Vec::new();
        let mut sizing_occupied: Vec<usize> = vec![0; num_cols];
        let mut sizing_section = None;

        for (sizing_row_idx, row) in rows.iter().enumerate() {
            let section_key = table_section_key(
                row_section_elements[sizing_row_idx],
                row_section_child_indices[sizing_row_idx],
            );
            if sizing_section != Some(section_key) {
                sizing_occupied.fill(0);
                sizing_section = Some(section_key);
            }
            let row_classes = row.class_list();
            // Build ancestors for the row: table + optional section element
            let mut sizing_row_ancestors = table_ancestors.clone();
            if let Some(section_el) = row_section_elements[sizing_row_idx] {
                sizing_row_ancestors.push(AncestorInfo {
                    element: section_el,
                    child_index: row_section_child_indices[sizing_row_idx],
                    sibling_count: row_section_sibling_counts[sizing_row_idx],
                    preceding_siblings: Vec::new(),
                    following_siblings: Vec::new(),
                    is_empty: false,
                });
            }
            let sizing_row_parent_style = row_section_elements[sizing_row_idx].map(|section_el| {
                compute_column_style(
                    section_el,
                    style,
                    rules,
                    &table_ancestors,
                    row_section_child_indices[sizing_row_idx],
                    row_section_sibling_counts[sizing_row_idx],
                )
            });
            let sizing_section_collapsed = sizing_row_parent_style
                .as_ref()
                .is_some_and(|section_style| section_style.visibility == Visibility::Collapse);
            let sizing_row_ctx = SelectorContext {
                ancestors: sizing_row_ancestors,
                child_index: row_section_indices[sizing_row_idx],
                sibling_count: row_section_sizes[sizing_row_idx],
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let mut row_style = compute_style_with_context(
                row.tag,
                row.style_attr(),
                sizing_row_parent_style.as_ref().unwrap_or(style),
                rules,
                row.tag_name(),
                &row_classes,
                row.id(),
                &row.attributes,
                &sizing_row_ctx,
            );
            // `display: none` and `visibility: collapse` rows are removed from the
            // table entirely (no cells, no reserved height), so skip measuring them.
            if row_style.display == Display::None || row_style.visibility == Visibility::Collapse {
                continue;
            }
            if sizing_section_collapsed {
                continue;
            }
            row_style.width = Some(inner_width);
            let mut col_pos: usize = 0;
            for child in &row.children {
                if let DomNode::Element(cell_el) = child {
                    if cell_el.tag == HtmlTag::Td || cell_el.tag == HtmlTag::Th {
                        while col_pos < num_cols && sizing_occupied[col_pos] > 0 {
                            sizing_occupied[col_pos] -= 1;
                            col_pos += 1;
                        }
                        if col_pos >= num_cols {
                            break;
                        }
                        let colspan = parse_cell_colspan(cell_el);
                        let span = colspan.min(num_cols - col_pos);
                        let cell_classes = cell_el.class_list();
                        let mut cell_sizing_ancestors = sizing_row_ctx.ancestors.clone();
                        cell_sizing_ancestors.push(AncestorInfo {
                            element: row,
                            child_index: row_section_indices[sizing_row_idx],
                            sibling_count: row_section_sizes[sizing_row_idx],
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        });
                        let cell_sizing_ctx = SelectorContext {
                            ancestors: cell_sizing_ancestors,
                            child_index: col_pos,
                            sibling_count: num_cols,
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        };
                        let mut cell_style = compute_style_with_context(
                            cell_el.tag,
                            cell_el.style_attr(),
                            &row_style,
                            rules,
                            cell_el.tag_name(),
                            &cell_classes,
                            cell_el.id(),
                            &cell_el.attributes,
                            &cell_sizing_ctx,
                        );
                        apply_cell_attribute_hints(
                            &mut cell_style,
                            table_attr_cellpadding,
                            table_attr_border_width,
                        );
                        let mut runs = Vec::new();
                        let mut nested_rows = Vec::new();
                        let recurse_descendants = cell_el.children.iter().any(
                            |node| matches!(node, DomNode::Element(e) if recurses_as_layout_child(e.tag)),
                        );
                        let mut text_ancestors = cell_sizing_ctx.ancestors.clone();
                        text_ancestors.push(AncestorInfo {
                            element: cell_el,
                            child_index: col_pos,
                            sibling_count: num_cols,
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        });
                        collect_table_cell_content_inner(
                            &cell_el.children,
                            &cell_style,
                            &mut runs,
                            &mut nested_rows,
                            None,
                            rules,
                            fonts,
                            filter_defs,
                            false,
                            recurse_descendants,
                            recurse_descendants,
                            &text_ancestors,
                            inner_width.max(1.0),
                            counter_state,
                        );
                        // Estimate content width the way the line wrapper measures
                        // it: sum each word's width plus one inter-word space. The
                        // wrapper measures words and spaces separately (losing the
                        // cross-word kerning that a single full-string measurement
                        // folds in), so measuring the full run here would under-size
                        // the column and force max-content text to wrap. Track the
                        // longest single word too, so short headings never hyphenate.
                        // `nowrap`/`pre`: the run cannot break, so its min-content
                        // equals its max-content (the whole run width).
                        let cell_nowrap =
                            matches!(cell_style.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre);
                        let mut content_min_width = 0.0f32;
                        let content_width: f32 = runs
                            .iter()
                            .map(|run| {
                                let space_width = estimate_word_width(
                                    " ",
                                    run.font_size,
                                    &run.font_family,
                                    run.bold,
                                    run.italic,
                                    fonts,
                                );
                                let words: Vec<&str> = run.text.split_whitespace().collect();
                                let mut line_width = 0.0f32;
                                let mut longest_word_width = 0.0f32;
                                for (i, word) in words.iter().enumerate() {
                                    let word_width = estimate_word_width(
                                        word,
                                        run.font_size,
                                        &run.font_family,
                                        run.bold,
                                        run.italic,
                                        fonts,
                                    );
                                    if i > 0 {
                                        line_width += space_width;
                                    }
                                    line_width += word_width;
                                    longest_word_width = longest_word_width.max(word_width);
                                }
                                let run_max = line_width.max(longest_word_width);
                                // Min-content per run: longest unbreakable word, or
                                // the full run when it cannot wrap.
                                content_min_width += if cell_nowrap {
                                    run_max
                                } else {
                                    longest_word_width
                                };
                                run_max
                            })
                            .sum();
                        // The line wrapper accumulates word/space widths in a
                        // different association order than the sum above, so an
                        // exact-fit column can tip into wrapping on a 1-ULP
                        // rounding difference. A sub-point slack keeps max-content
                        // text on one line without visibly widening the column.
                        let content_width = if content_width > 0.0 {
                            content_width + 0.5
                        } else {
                            content_width
                        };
                        // Nested block descendants (e.g. a fixed-width <div>) and
                        // nested tables both contribute a minimum content width so
                        // a shrink-to-fit cell does not crush them narrower than
                        // their own declared/intrinsic width.
                        let nested_width = nested_rows
                            .iter()
                            .map(nested_element_preferred_width)
                            .fold(0.0f32, f32::max);
                        // An explicit width on the cell (CSS or `width` attribute)
                        // seeds the column's preferred width: the column is at least
                        // as wide as the declared width, taken as the track width
                        // (consistent with the fixed-layout path).
                        let explicit_cell_width =
                            resolve_cell_track_width(cell_el, &cell_style, inner_width)
                                .unwrap_or(0.0);
                        // Content-box → border-box: the column track must hold the
                        // content plus the cell's horizontal padding AND border
                        // (borders paint inside the cell box). Under
                        // `border-collapse: collapse` adjacent borders merge onto a
                        // shared grid line and each cell owns only HALF of a
                        // collapsed border, so the track contribution scales the
                        // border by the same half factor used by the paint inset
                        // (see `border_inset_factor` below); `separate` keeps the
                        // full border. Without this the track is sized for the full
                        // border but painted center-to-center on the grid line,
                        // making each collapsed column ~half-a-border too wide.
                        let track_border_factor =
                            if style.border_collapse == BorderCollapse::Collapse {
                                0.5
                            } else {
                                1.0
                            };
                        let cell_padding_x = cell_style.padding.left
                            + cell_style.padding.right
                            + cell_style.border.horizontal_width() * track_border_factor;
                        let total_preferred = (content_width.max(nested_width) + cell_padding_x)
                            .max(explicit_cell_width);
                        // Min-content includes padding and is floored by an explicit
                        // cell width (an explicit `width` makes the column at least
                        // that wide even for shrinking).
                        let total_min = (content_min_width.max(nested_width) + cell_padding_x)
                            .max(explicit_cell_width);
                        if span == 1 {
                            if col_pos < num_cols {
                                preferred_widths[col_pos] =
                                    preferred_widths[col_pos].max(total_preferred);
                                min_widths[col_pos] = min_widths[col_pos].max(total_min);
                            }
                        } else {
                            spanning_widths.push((col_pos, span, total_preferred, total_min));
                        }
                        let rowspan = parse_cell_rowspan(
                            cell_el,
                            row_section_sizes[sizing_row_idx]
                                .saturating_sub(row_section_indices[sizing_row_idx]),
                        );
                        if rowspan > 1 {
                            for i in 0..span {
                                if col_pos + i < num_cols {
                                    sizing_occupied[col_pos + i] = rowspan - 1;
                                }
                            }
                        }
                        col_pos += span;
                    }
                }
            }
        }

        for (start, span, total_preferred, total_min) in spanning_widths {
            let end = (start + span).min(num_cols);
            let preferred_sum: f32 = preferred_widths[start..end].iter().sum();
            if total_preferred > preferred_sum {
                distribute_extra_width(
                    &mut preferred_widths[start..end],
                    total_preferred - preferred_sum,
                );
            }
            let min_sum: f32 = min_widths[start..end].iter().sum();
            if total_min > min_sum {
                distribute_extra_width(&mut min_widths[start..end], total_min - min_sum);
            }
        }

        for (width, min_w) in preferred_widths.iter_mut().zip(min_widths.iter_mut()) {
            if *width < min_col_width {
                *width = min_col_width;
            }
            // A column can never be narrower than its own min-content.
            *min_w = min_w.max(0.0).min(*width);
        }

        if has_explicit_widths {
            preferred_widths
                .iter()
                .zip(explicit_col_widths.iter())
                .map(|(preferred, explicit)| {
                    explicit
                        .map(|width| width.resolve(columns_width).max(min_col_width))
                        .unwrap_or_else(|| preferred.max(min_col_width))
                })
                .collect()
        } else {
            let total_preferred: f32 = preferred_widths.iter().sum();
            if total_preferred <= columns_width {
                let extra = columns_width - total_preferred;
                // Shrink-to-fit: a table with no explicit `width` is only as wide
                // as its columns require, so don't stretch the columns to fill the
                // containing block. Only an explicitly-sized table absorbs the
                // leftover space.
                if table_has_explicit_width && total_preferred > 0.0 && extra > 0.0 {
                    preferred_widths
                        .iter()
                        .map(|width| width + (width / total_preferred) * extra)
                        .collect()
                } else {
                    preferred_widths
                }
            } else {
                // The columns' max-content sum exceeds the available width, so they
                // must shrink. CSS2 §17.5.2.2: distribute the deficit across the
                // columns' shrinkable headroom (max − min), never below each
                // column's min-content. When even the min-content sum exceeds the
                // available width, the table overflows (e.g. `white-space: nowrap`
                // wider than the declared `width`) rather than crushing the text.
                let total_min: f32 = min_widths.iter().sum();
                if total_min >= columns_width {
                    // Cannot fit even at min-content: use min-content widths and let
                    // the table overflow its declared width.
                    min_widths.clone()
                } else {
                    let shrinkable: f32 = total_preferred - total_min;
                    let deficit = total_preferred - columns_width;
                    preferred_widths
                        .iter()
                        .zip(min_widths.iter())
                        .map(|(pref, min_w)| {
                            if shrinkable > 0.0 {
                                let headroom = pref - min_w;
                                pref - deficit * (headroom / shrinkable)
                            } else {
                                *pref
                            }
                        })
                        .collect()
                }
            }
        }
    };
    for (width, info) in col_widths.iter_mut().zip(column_info.iter()) {
        if info.collapsed {
            *width = 0.0;
        }
    }
    let current_table_width = col_widths.iter().sum::<f32>()
        + if style.border_collapse == BorderCollapse::Separate {
            effective_border_spacing * (col_widths.len().saturating_add(1) as f32)
        } else {
            outer_left_border / 2.0 + outer_right_border / 2.0
        };
    if caption_min_width > current_table_width {
        distribute_extra_width(&mut col_widths, caption_min_width - current_table_width);
    }

    // Build layout rows, tracking cells occupied by rowspan from previous rows.
    // Each entry in `occupied` tracks the remaining rowspan count for that column.
    let mut occupied: Vec<usize> = vec![0; num_cols];
    let mut occupied_min_heights: Vec<f32> = vec![0.0; num_cols];
    let mut layout_section = None;
    let mut collapsed_section_spacers = Vec::new();
    // Remember where this table's rows start so a table-level background/border
    // box can be inserted ahead of them once the total height is known.
    let table_output_start = output.len();
    for (row_idx, row) in rows.iter().enumerate() {
        let section_key = table_section_key(
            row_section_elements[row_idx],
            row_section_child_indices[row_idx],
        );
        if layout_section != Some(section_key) {
            occupied.fill(0);
            occupied_min_heights.fill(0.0);
            layout_section = Some(section_key);
        }
        let row_classes = row.class_list();
        // Use section-relative index for nth-child matching (browsers count
        // within thead/tbody/tfoot, not globally across all rows).
        let section_idx = row_section_indices[row_idx];
        let section_size = row_section_sizes[row_idx];
        // Build ancestors for the row: table + optional section element
        let mut row_ancestors = table_ancestors.clone();
        if let Some(section_el) = row_section_elements[row_idx] {
            row_ancestors.push(AncestorInfo {
                element: section_el,
                child_index: row_section_child_indices[row_idx],
                sibling_count: row_section_sibling_counts[row_idx],
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
        }
        let row_parent_style = row_section_elements[row_idx].map(|section_el| {
            compute_column_style(
                section_el,
                style,
                rules,
                &table_ancestors,
                row_section_child_indices[row_idx],
                row_section_sibling_counts[row_idx],
            )
        });
        let section_collapsed = row_parent_style
            .as_ref()
            .is_some_and(|section_style| section_style.visibility == Visibility::Collapse);
        let row_selector_ctx = SelectorContext {
            ancestors: row_ancestors,
            child_index: section_idx,
            sibling_count: section_size,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        };
        let mut row_style = compute_style_with_context(
            row.tag,
            row.style_attr(),
            row_parent_style.as_ref().unwrap_or(style),
            rules,
            row.tag_name(),
            &row_classes,
            row.id(),
            &row.attributes,
            &row_selector_ctx,
        );
        // `display: none` rows are removed from the table entirely.
        if row_style.display == Display::None {
            continue;
        }
        if row_style.visibility == Visibility::Collapse || section_collapsed {
            // CSS Tables collapses the row's content and height, but in the
            // collapsed-border model the row's own cell borders still
            // participate in border conflict resolution. Represent that as a
            // contentless row whose height is only the top+bottom collapsed
            // border thickness, so adjacent visible rows keep the same border
            // geometry as browsers without resurrecting the collapsed content.
            if style.border_collapse == BorderCollapse::Collapse {
                row_style.width = Some(inner_width);
                let mut cells = Vec::new();
                let mut col_pos = 0usize;
                for child in &row.children {
                    let DomNode::Element(cell_el) = child else {
                        continue;
                    };
                    if cell_el.tag != HtmlTag::Td && cell_el.tag != HtmlTag::Th {
                        continue;
                    }
                    if col_pos >= num_cols {
                        break;
                    }
                    let colspan = parse_cell_colspan(cell_el);
                    let span = colspan.min(num_cols - col_pos);

                    let cell_classes = cell_el.class_list();
                    let mut cell_ancestors = row_selector_ctx.ancestors.clone();
                    cell_ancestors.push(AncestorInfo {
                        element: row,
                        child_index: section_idx,
                        sibling_count: section_size,
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    });
                    let cell_selector_ctx = SelectorContext {
                        ancestors: cell_ancestors,
                        child_index: col_pos,
                        sibling_count: num_cols,
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    };
                    let mut cell_style = compute_style_with_context(
                        cell_el.tag,
                        cell_el.style_attr(),
                        &row_style,
                        rules,
                        cell_el.tag_name(),
                        &cell_classes,
                        cell_el.id(),
                        &cell_el.attributes,
                        &cell_selector_ctx,
                    );
                    apply_cell_attribute_hints(
                        &mut cell_style,
                        table_attr_cellpadding,
                        table_attr_border_width,
                    );
                    let cell_border = LayoutBorder::from_computed(&cell_style.border);
                    cells.push(TableCell {
                        lines: Vec::new(),
                        nested_rows: Vec::new(),
                        bold: false,
                        background_color: None,
                        padding_top: 0.0,
                        padding_right: 0.0,
                        padding_bottom: 0.0,
                        padding_left: 0.0,
                        colspan: span,
                        rowspan: 1,
                        border: cell_border,
                        text_align: cell_style.text_align,
                        vertical_align: VerticalAlign::Top,
                        min_content_height: cell_border.top.width + cell_border.bottom.width,
                        hide_if_empty: false,
                        grid_inset: None,
                        clips: false,
                        background_gradient: None,
                        background_radial_gradient: None,
                        background_conic_gradient: None,
                    });
                    col_pos += span;
                }

                if !cells.is_empty() {
                    let first_emitted_row = output.len() == table_output_start;
                    let mut row_cells = cells;
                    let mut row_col_widths = col_widths.clone();
                    if style.direction_rtl {
                        row_cells.reverse();
                        row_col_widths.reverse();
                    }
                    output.push(LayoutElement::TableRow {
                        cells: row_cells,
                        col_widths: row_col_widths,
                        margin_top: if first_emitted_row {
                            table_grid_inset
                        } else {
                            0.0
                        },
                        margin_bottom: 0.0,
                        border_collapse: style.border_collapse,
                        border_spacing: effective_border_spacing,
                        is_header: false,
                        is_footer: false,
                        offset_left: style.margin.left.max(0.0) + table_grid_inset,
                        break_inside_avoid: style.break_inside_avoid,
                    });
                }
            } else {
                let spacer_height = if section_collapsed {
                    let section_key = table_section_key(
                        row_section_elements[row_idx],
                        row_section_child_indices[row_idx],
                    );
                    if collapsed_section_spacers.contains(&section_key) {
                        continue;
                    }
                    collapsed_section_spacers.push(section_key);
                    row_style.font_size * 0.75
                } else {
                    effective_border_spacing_vertical * 0.75
                };
                if spacer_height <= 0.0 {
                    continue;
                }
                let first_emitted_row = output.len() == table_output_start;
                let mut row_col_widths = col_widths.clone();
                if style.direction_rtl {
                    row_col_widths.reverse();
                }
                let is_header = row_section_elements[row_idx]
                    .map(|s| s.tag == HtmlTag::Thead)
                    .unwrap_or(false);
                let is_footer = row_section_elements[row_idx]
                    .map(|s| s.tag == HtmlTag::Tfoot)
                    .unwrap_or(false);
                output.push(LayoutElement::TableRow {
                    cells: Vec::new(),
                    col_widths: row_col_widths,
                    margin_top: spacer_height
                        + if first_emitted_row {
                            table_grid_inset
                        } else {
                            0.0
                        },
                    margin_bottom: 0.0,
                    border_collapse: style.border_collapse,
                    border_spacing: effective_border_spacing,
                    is_header,
                    is_footer,
                    offset_left: style.margin.left.max(0.0) + table_grid_inset,
                    break_inside_avoid: style.break_inside_avoid,
                });
            }
            continue;
        }
        row_style.width = Some(inner_width);
        let mut cells = Vec::new();
        let mut cell_hidden_borders = Vec::new();

        // Current logical column position in the grid
        let mut col_pos: usize = 0;
        let mut child_iter = row.children.iter().filter_map(|child| {
            if let DomNode::Element(cell_el) = child {
                if cell_el.tag == HtmlTag::Td || cell_el.tag == HtmlTag::Th {
                    return Some(cell_el);
                }
            }
            None
        });

        // Process cells, skipping occupied positions and inserting phantom cells
        let mut next_cell = child_iter.next();
        while col_pos < num_cols {
            if occupied[col_pos] > 0 {
                // This position is occupied by a rowspan from a previous row.
                // Insert a phantom cell (rowspan = 0) as a placeholder.
                let span_cols = {
                    // Count how many consecutive occupied columns share this rowspan
                    let remaining = occupied[col_pos];
                    let min_height = occupied_min_heights[col_pos];
                    let mut count = 1;
                    while col_pos + count < num_cols
                        && occupied[col_pos + count] == remaining
                        && (occupied_min_heights[col_pos + count] - min_height).abs() < 0.01
                    {
                        count += 1;
                    }
                    count
                };
                let phantom_min_height = occupied_min_heights[col_pos];
                cells.push(TableCell {
                    lines: Vec::new(),
                    nested_rows: Vec::new(),
                    bold: false,
                    background_color: None,
                    padding_top: 0.0,
                    padding_right: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    colspan: span_cols,
                    rowspan: 0, // phantom cell marker
                    border: LayoutBorder::default(),
                    text_align: TextAlign::Left,
                    vertical_align: VerticalAlign::Baseline,
                    min_content_height: phantom_min_height,
                    hide_if_empty: false,
                    grid_inset: None,
                    clips: false,
                    background_gradient: None,
                    background_radial_gradient: None,
                    background_conic_gradient: None,
                });
                cell_hidden_borders.push(HiddenBorderSides::default());
                for i in 0..span_cols {
                    occupied[col_pos + i] -= 1;
                    if occupied[col_pos + i] == 0 {
                        occupied_min_heights[col_pos + i] = 0.0;
                    }
                }
                col_pos += span_cols;
                continue;
            }

            // Place the next real cell at this position
            let Some(cell_el) = next_cell else { break };
            next_cell = child_iter.next();

            let colspan = parse_cell_colspan(cell_el);
            let rowspan = parse_cell_rowspan(
                cell_el,
                row_section_sizes[row_idx].saturating_sub(row_section_indices[row_idx]),
            );

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            cell_ancestors.push(AncestorInfo {
                element: row,
                child_index: section_idx,
                sibling_count: section_size,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            let cell_selector_ctx = SelectorContext {
                ancestors: cell_ancestors,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let mut cell_style = compute_style_with_context(
                cell_el.tag,
                cell_el.style_attr(),
                &row_style,
                rules,
                cell_el.tag_name(),
                &cell_classes,
                cell_el.id(),
                &cell_el.attributes,
                &cell_selector_ctx,
            );
            apply_cell_attribute_hints(
                &mut cell_style,
                table_attr_cellpadding,
                table_attr_border_width,
            );
            let hidden_borders = declared_hidden_borders(cell_el, rules, &cell_selector_ctx);
            // Compute effective width from auto-sized column widths. Cell borders
            // are painted INSIDE the cell box (CSS2 §17.6: the border-box is the
            // column width), so the content box is inset by the border and the
            // padding on every side — matching Chrome, which positions cell content
            // (text and nested blocks) at the inner border+padding edge.
            //
            // Under `border-collapse: collapse` adjacent borders merge and each cell
            // owns only HALF of a collapsed border (the rest belongs to the shared
            // grid line), so the content box subtracts half the border width;
            // `separate` subtracts the full border.
            let cell_border = LayoutBorder::from_computed(&cell_style.border);
            let border_inset_factor = if style.border_collapse == BorderCollapse::Collapse {
                0.5
            } else {
                1.0
            };
            let inset_left = cell_border.left.width * border_inset_factor + cell_style.padding.left;
            let inset_right =
                cell_border.right.width * border_inset_factor + cell_style.padding.right;
            let inset_top = cell_border.top.width * border_inset_factor + cell_style.padding.top;
            let inset_bottom =
                cell_border.bottom.width * border_inset_factor + cell_style.padding.bottom;
            let effective_width: f32 = col_widths.iter().skip(col_pos).take(colspan).copied().sum();
            let cell_inner = effective_width - inset_left - inset_right;
            let mut cell_content_style = cell_style.clone();
            cell_content_style.width = Some(cell_inner.max(0.0));
            // A cell's `height` is its border-box; a child's `height: %` resolves
            // against the cell's *content* box (height minus the cell's own
            // padding and border). `cell_content_style` is the parent handed to
            // child style resolution, so expose the content-box height here —
            // otherwise `height: 100%` resolved against the full border-box and a
            // padded cell's inner block rendered too tall (overflowing the cell).
            cell_content_style.height = cell_style.height.map(|h| {
                super::helpers::resolve_content_box_height(
                    h,
                    cell_style.padding.top,
                    cell_style.padding.bottom,
                    (cell_border.top.width + cell_border.bottom.width) * border_inset_factor,
                    cell_style.box_sizing,
                )
            });

            let mut runs = Vec::new();
            let mut nested_rows = Vec::new();
            let recurse_descendants = cell_el
                .children
                .iter()
                .any(|node| matches!(node, DomNode::Element(e) if recurses_as_layout_child(e.tag)));
            let mut text_ancestors = cell_selector_ctx.ancestors.clone();
            text_ancestors.push(AncestorInfo {
                element: cell_el,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            let (block_margin_top, block_margin_bottom) = table_cell_edge_block_margins(
                &cell_el.children,
                &cell_content_style,
                rules,
                &text_ancestors,
            );
            collect_table_cell_content_inner(
                &cell_el.children,
                &cell_content_style,
                &mut runs,
                &mut nested_rows,
                None,
                rules,
                fonts,
                filter_defs,
                false,
                recurse_descendants,
                recurse_descendants,
                &text_ancestors,
                cell_inner.max(1.0),
                counter_state,
            );
            let mut lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner.max(1.0),
                    cell_style.font_size,
                    resolved_line_height_factor(&cell_style, fonts),
                    cell_style.overflow_wrap,
                )
                .with_rtl(cell_style.direction_rtl)
                .with_bidi_override(cell_style.bidi_override),
                fonts,
            );
            if cell_style.overflow.clips() {
                clip_text_lines_to_width(&mut lines, cell_inner.max(0.0), fonts);
            }

            let column_bg = column_info
                .iter()
                .skip(col_pos)
                .take(colspan)
                .find_map(|info| info.background_color);
            let bg = style_background_rgba(&cell_style)
                .or_else(|| style_background_rgba(&row_style))
                .or_else(|| row_parent_style.as_ref().and_then(style_background_rgba))
                .or(column_bg);

            // An explicit cell height is a minimum on the cell's rendered box.
            // The intrinsic content height now folds the cell border into its top/
            // bottom inset (see `padding_*` below), so the minimum must match that
            // basis: for content-box, add padding AND border; for border-box, the
            // declared height already includes both, so keep it whole.
            let min_content_height = cell_style.height.map_or(0.0, |h| {
                if cell_style.box_sizing == BoxSizing::BorderBox {
                    h
                } else {
                    h + inset_top + inset_bottom
                }
            });
            // Under `empty-cells: hide`, a cell with no in-flow content has its
            // border and background suppressed so the table background shows
            // through. Emptiness is decided from the DOM, not the collapsed
            // text, because `&nbsp;` is content yet whitespace-collapses away.
            let hide_if_empty = style.border_collapse == BorderCollapse::Separate
                && cell_style.empty_cells == crate::style::computed::EmptyCells::Hide
                && cell_has_no_content(cell_el);
            let row_span_share = if rowspan > 1 {
                (min_content_height / rowspan as f32).max(0.0)
            } else {
                min_content_height
            };
            let mut table_cell = TableCell {
                lines,
                nested_rows,
                bold: cell_style.font_weight == FontWeight::Bold,
                background_color: bg,
                // `padding_*` here is the cell's CONTENT inset (border + padding):
                // borders paint inside the cell box, so content is offset past both.
                padding_top: inset_top + block_margin_top,
                padding_right: inset_right,
                padding_bottom: inset_bottom + block_margin_bottom,
                padding_left: inset_left,
                colspan,
                rowspan,
                border: cell_border,
                text_align: cell_style.text_align,
                vertical_align: table_cell_vertical_align(cell_style.vertical_align),
                min_content_height: row_span_share,
                hide_if_empty,
                grid_inset: None,
                clips: cell_style.overflow.clips(),
                background_gradient: None,
                background_radial_gradient: None,
                background_conic_gradient: None,
            };
            let collapsed_columns = column_info
                .iter()
                .skip(col_pos)
                .take(colspan)
                .all(|info| info.collapsed);
            if table_cell_is_hidden(&cell_style) || collapsed_columns {
                hide_table_cell_paint(&mut table_cell);
            }
            cells.push(table_cell);
            cell_hidden_borders.push(hidden_borders);

            // Mark subsequent rows as occupied if rowspan > 1
            if rowspan > 1 {
                for i in 0..colspan {
                    if col_pos + i < num_cols {
                        occupied[col_pos + i] = rowspan - 1;
                        occupied_min_heights[col_pos + i] = row_span_share;
                    }
                }
            }

            col_pos += colspan;
        }

        if !cells.is_empty() {
            if style.border_collapse == BorderCollapse::Collapse {
                for idx in 0..cells.len().saturating_sub(1) {
                    let (left, right) = cells.split_at_mut(idx + 1);
                    resolve_collapsed_border_pair(
                        &mut left[idx].border,
                        cell_hidden_borders[idx],
                        BorderSideName::Right,
                        &mut right[0].border,
                        cell_hidden_borders[idx + 1],
                        BorderSideName::Left,
                    );
                }
            }
            let first_emitted_row = output.len() == table_output_start;
            if let Some(row_height) = row_style.height.or(row_style.min_height) {
                enforce_row_min_height(&mut cells, row_height);
            }
            let mut row_cells = cells;
            let mut row_col_widths = col_widths.clone();
            if style.direction_rtl {
                row_cells.reverse();
                row_col_widths.reverse();
            }
            let is_header = row_section_elements[row_idx]
                .map(|s| s.tag == HtmlTag::Thead)
                .unwrap_or(false);
            let is_footer = row_section_elements[row_idx]
                .map(|s| s.tag == HtmlTag::Tfoot)
                .unwrap_or(false);
            output.push(LayoutElement::TableRow {
                cells: row_cells,
                col_widths: row_col_widths,
                // The table-level background box (inserted below) carries the
                // table's own `margin-top`. The first row is therefore inset
                // only by the top *vertical* `border-spacing` (zero when
                // collapsed); subsequent rows are separated by the same.
                margin_top: if style.border_collapse == BorderCollapse::Separate {
                    effective_border_spacing_vertical
                } else {
                    0.0
                } + if first_emitted_row {
                    table_grid_inset
                } else {
                    0.0
                },
                margin_bottom: 0.0,
                border_collapse: style.border_collapse,
                border_spacing: effective_border_spacing,
                is_header,
                is_footer,
                // The table's own horizontal start margin shifts every cell (and
                // the table box) right from the containing block's content edge,
                // mirroring how `margin_top` shifts it down.
                offset_left: style.margin.left.max(0.0) + table_grid_inset,
                break_inside_avoid: style.break_inside_avoid,
            });
        }
    }

    // If no rows were actually emitted (e.g. every row was `display:none`),
    // there is no table box to paint.
    if output.len() == table_output_start {
        return;
    }

    let mut table_border = LayoutBorder::from_computed(&style.border);
    if !table_border.has_any() {
        if let Some(width) = table_attr_border_width {
            table_border = attr_border(width, true);
        }
    }

    let separate = matches!(style.border_collapse, BorderCollapse::Separate);
    // Horizontal spacing applies to column gaps + left/right outer edges;
    // vertical spacing applies to row gaps + top/bottom outer edges. They differ
    // only for the two-value `border-spacing: H V` form.
    let edge_spacing_h = if separate {
        effective_border_spacing
    } else {
        0.0
    };
    let edge_spacing_v = if separate {
        effective_border_spacing_vertical
    } else {
        0.0
    };

    if let Some(table_height) = style.height.or(style.min_height) {
        stretch_table_rows_to_min_height(
            &mut output[table_output_start..],
            table_height,
            edge_spacing_v,
        );
    }
    if style.border_collapse == BorderCollapse::Collapse {
        resolve_collapsed_outer_table_borders(&mut output[table_output_start..], table_border);
    }

    // Height of the table content box: the rows plus, for `separate` collapse,
    // the vertical `border-spacing` above the first row, below the last row, and
    // between each adjacent pair. (`compute_row_height` lives in the renderer;
    // mirror it here from each row's cells.)
    let mut emitted_rows = 0usize;
    let mut rows_height = 0.0f32;
    for elem in &output[table_output_start..] {
        if let LayoutElement::TableRow {
            cells, margin_top, ..
        } = elem
        {
            if cells.is_empty() {
                rows_height += *margin_top;
                continue;
            }
            emitted_rows += 1;
            rows_height += cells
                .iter()
                .map(table_cell_content_height)
                .fold(0.0f32, f32::max);
        }
    }
    let box_height = rows_height
        + edge_spacing_v * (emitted_rows.saturating_add(1) as f32)
        + table_grid_inset * 2.0;

    // Width of the table content box: the resolved column widths plus, for
    // `separate` collapse, one horizontal `border-spacing` on each outer edge
    // and between each adjacent pair. For a shrink-to-fit (auto) table this is
    // narrower than `inner_width`, so the background must follow the columns, not
    // the containing block.
    let columns_sum: f32 = col_widths.iter().sum();
    // For collapse the column tracks were shrunk by the outer borders (which paint
    // inside the table box); add them back so the table-level background/border box
    // spans the full border-box width.
    let collapse_outer_w = if style.border_collapse == BorderCollapse::Collapse {
        outer_left_border / 2.0 + outer_right_border / 2.0
    } else {
        0.0
    };
    let box_width = columns_sum
        + edge_spacing_h * (col_widths.len().saturating_add(1) as f32)
        + collapse_outer_w
        + table_grid_inset * 2.0;

    // The last row carries the bottom vertical `border-spacing` gap plus the
    // table's own `margin-bottom`, so the in-flow height below the rows matches
    // the box. A `caption-side: bottom` caption (appended after the rows) takes
    // over the table's `margin-bottom` instead, so the row keeps only the gap.
    let caption_on_top = caption_style_for_layout
        .as_ref()
        .is_none_or(|caption_style| {
            matches!(
                caption_style.caption_side,
                crate::style::computed::CaptionSide::Top
            )
        });
    let bottom_caption = caption.is_some() && !caption_on_top;
    if let Some(LayoutElement::TableRow { margin_bottom, .. }) = output.last_mut() {
        *margin_bottom = edge_spacing_v
            + table_grid_inset
            + if bottom_caption {
                0.0
            } else {
                style.margin.bottom
            };
    }

    let has_top_caption = caption.is_some() && caption_on_top;

    // The table's own `margin-top` is carried by whichever box comes first: a
    // top caption, otherwise the background box (if any), otherwise the first
    // row. Track whether something earlier already claimed it.
    let mut margin_top_claimed = false;

    // Paint the table element's own background/border behind the rows. It is a
    // zero-flow box: its `margin-top` carries the table's own top margin (unless
    // a caption above already does) while a matching negative `margin-bottom`
    // cancels its height so the rows that follow render on top of it.
    if has_background_paint(style) || table_border.has_any() {
        let bg_margin_top = if has_top_caption {
            0.0
        } else {
            margin_top_claimed = true;
            style.margin.top
        };
        let bg_block = LayoutElement::TextBlock {
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines: Vec::new(),
            margin_top: bg_margin_top,
            margin_bottom: -(box_height + table_border.vertical_width()),
            text_align: TextAlign::Left,
            writing_mode: crate::style::computed::WritingMode::HorizontalTb,
            background_color: style.background_color.map(|c| c.to_f32_rgba()),
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: table_border,
            block_width: Some(box_width),
            block_height: Some(box_height),
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            background_blend_mode: style.background_blend_mode,
            float: Default::default(),
            clear: Default::default(),
            position: Default::default(),
            offset_top: 0.0,
            // Shift the table's background/border box right by its own start
            // margin so it aligns with the cells (which carry the same offset).
            offset_left: style.margin.left.max(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            clip_children_count: 0,
            box_shadow: style.box_shadow.clone(),
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: None,
            transform_origin: crate::style::computed::TransformOrigin::default(),
            border_radius: style.border_radius,
            border_radii: style.border_radii,
            border_radii_y: style.border_radii_y,
            outline_offset: style.outline_offset,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: Default::default(),
            background_gradient: style.background_gradient.clone(),
            background_radial_gradient: style.background_radial_gradient.clone(),
            background_conic_gradient: style.background_conic_gradient.clone(),
            background_svg: style.background_svg.clone(),
            background_blur_radius: 0.0,
            background_size: Default::default(),
            background_position: Default::default(),
            background_repeat: Default::default(),
            background_origin: Default::default(),
            background_clip: Default::default(),
            z_index: style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        };
        output.insert(table_output_start, bg_block);
    }

    // `<caption>` (caption-side:top) renders as a full-table-width block above
    // the rows: it carries the table's `margin-top` and pushes the rest of the
    // table down by its own height.
    if let Some((caption_el, caption_child_idx)) = caption {
        let caption_style = caption_style_for_layout.clone().unwrap_or_else(|| {
            compute_caption_style(
                caption_el,
                caption_child_idx,
                section_count,
                style,
                &table_ancestors,
                rules,
            )
        });
        let caption_inner =
            (box_width - caption_style.padding.left - caption_style.padding.right).max(1.0);
        let mut caption_ancestors = table_ancestors.clone();
        caption_ancestors.push(AncestorInfo {
            element: caption_el,
            child_index: caption_child_idx,
            sibling_count: section_count,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        });
        let mut caption_runs = Vec::new();
        let mut caption_nested = Vec::new();
        collect_table_cell_content_inner(
            &caption_el.children,
            &caption_style,
            &mut caption_runs,
            &mut caption_nested,
            None,
            rules,
            fonts,
            filter_defs,
            false,
            false,
            true,
            &caption_ancestors,
            caption_inner,
            counter_state,
        );
        let caption_lines = wrap_text_runs(
            caption_runs,
            TextWrapOptions::new(
                caption_inner,
                caption_style.font_size,
                resolved_line_height_factor(&caption_style, fonts),
                caption_style.overflow_wrap,
            )
            .with_rtl(caption_style.direction_rtl)
            .with_bidi_override(caption_style.bidi_override),
            fonts,
        );
        let caption_border = LayoutBorder::from_computed(&caption_style.border);
        // A top caption sits above the table and carries the table's
        // `margin-top`; a bottom caption is appended after the rows and carries
        // the table's `margin-bottom` instead (the rows already absorbed the
        // bottom border-spacing gap above).
        let (caption_margin_top, caption_margin_bottom) = if caption_on_top {
            (style.margin.top, 0.0)
        } else {
            (0.0, style.margin.bottom)
        };
        let caption_block = LayoutElement::TextBlock {
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines: caption_lines,
            margin_top: caption_margin_top,
            margin_bottom: caption_margin_bottom,
            text_align: caption_style.text_align,
            writing_mode: crate::style::computed::WritingMode::HorizontalTb,
            background_color: caption_style.background_color.map(|c| c.to_f32_rgba()),
            padding_top: caption_style.padding.top,
            padding_bottom: caption_style.padding.bottom,
            padding_left: caption_style.padding.left,
            padding_right: caption_style.padding.right,
            border: caption_border,
            block_width: Some(box_width),
            block_height: caption_style.height,
            opacity: caption_style.opacity,
            mix_blend_mode: caption_style.mix_blend_mode,
            background_blend_mode: caption_style.background_blend_mode,
            float: Default::default(),
            clear: Default::default(),
            position: Default::default(),
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            clip_children_count: 0,
            box_shadow: caption_style.box_shadow.clone(),
            visible: caption_style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: None,
            transform_origin: crate::style::computed::TransformOrigin::default(),
            border_radius: caption_style.border_radius,
            border_radii: caption_style.border_radii,
            border_radii_y: caption_style.border_radii_y,
            outline_offset: caption_style.outline_offset,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: caption_style.letter_spacing,
            word_spacing: caption_style.word_spacing,
            vertical_align: Default::default(),
            background_gradient: caption_style.background_gradient.clone(),
            background_radial_gradient: caption_style.background_radial_gradient.clone(),
            background_conic_gradient: caption_style.background_conic_gradient.clone(),
            background_svg: caption_style.background_svg.clone(),
            background_blur_radius: 0.0,
            background_size: Default::default(),
            background_position: Default::default(),
            background_repeat: Default::default(),
            background_origin: Default::default(),
            background_clip: Default::default(),
            z_index: caption_style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        };
        if caption_on_top {
            output.insert(table_output_start, caption_block);
            margin_top_claimed = true;
        } else {
            output.push(caption_block);
        }
    }

    // If neither a caption nor a background box claimed the table's `margin-top`,
    // fold it into the first emitted row so the table keeps its top margin.
    if !margin_top_claimed && style.margin.top != 0.0 {
        if let Some(LayoutElement::TableRow { margin_top, .. }) = output.get_mut(table_output_start)
        {
            *margin_top += style.margin.top;
        }
    }
}

fn table_cell_edge_block_margins(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
) -> (f32, f32) {
    let element_sibling_count = nodes
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();

    let mut first_margin_top = None;
    let mut last_margin_bottom = None;

    for (node_index, node) in nodes.iter().enumerate() {
        let DomNode::Element(element) = node else {
            continue;
        };
        if element.tag == HtmlTag::Br
            || element.tag == HtmlTag::Table
            || element.children.is_empty()
        {
            continue;
        }

        let child_index = nodes[..node_index]
            .iter()
            .filter(|node| matches!(node, DomNode::Element(_)))
            .count();
        let preceding_siblings = nodes[..node_index]
            .iter()
            .filter_map(|node| match node {
                DomNode::Element(element) => Some((
                    element.tag_name().to_string(),
                    element
                        .class_list()
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                )),
                _ => None,
            })
            .collect();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index,
            sibling_count: element_sibling_count,
            preceding_siblings,
            following_siblings: Vec::new(),
            is_empty: false,
        };
        let child_style = compute_style_with_context(
            element.tag,
            element.style_attr(),
            parent_style,
            rules,
            element.tag_name(),
            &element.class_list(),
            element.id(),
            &element.attributes,
            &selector_ctx,
        );
        if child_style.display == Display::Inline {
            continue;
        }

        first_margin_top.get_or_insert(child_style.margin.top);
        last_margin_bottom = Some(child_style.margin.bottom);
    }

    (
        first_margin_top.unwrap_or(0.0),
        last_margin_bottom.unwrap_or(0.0),
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_table_cell_content_inner(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    nested_rows: &mut Vec<LayoutElement>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    filter_defs: &HashMap<String, ElementNode>,
    inline_parent: bool,
    recurse_blocks: bool,
    suppress_direct_text_padding: bool,
    ancestors: &[AncestorInfo],
    available_width: f32,
    counter_state: &mut CounterState,
) {
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
    );
    let element_sibling_count = nodes
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();

    for (node_index, node) in nodes.iter().enumerate() {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    expand_pre_tabs(text, parent_style, fonts)
                } else {
                    collapse_whitespace(text)
                };
                // Apply CSS text-transform
                let processed = match parent_style.text_transform {
                    crate::style::computed::TextTransform::Uppercase => processed.to_uppercase(),
                    crate::style::computed::TextTransform::Lowercase => processed.to_lowercase(),
                    crate::style::computed::TextTransform::Capitalize => {
                        let mut result = String::with_capacity(processed.len());
                        let mut prev_is_space = true;
                        for c in processed.chars() {
                            if prev_is_space && c.is_alphabetic() {
                                for uc in c.to_uppercase() {
                                    result.push(uc);
                                }
                            } else {
                                result.push(c);
                            }
                            prev_is_space = c.is_whitespace();
                        }
                        result
                    }
                    crate::style::computed::TextTransform::None => processed,
                };
                if !processed.is_empty() {
                    let (bg, pad, br) = if (inline_parent || recurse_blocks) && !preserve_ws {
                        let pad = if suppress_direct_text_padding {
                            (0.0, 0.0)
                        } else {
                            (parent_style.padding.left, parent_style.padding.top)
                        };
                        (
                            parent_style.background_color.map(|c| c.to_f32_rgba()),
                            pad,
                            parent_style.border_radius,
                        )
                    } else {
                        (None, (0.0, 0.0), 0.0)
                    };
                    push_text_run(
                        runs,
                        TextRun {
                            text: processed,
                            font_size: parent_style.font_size,
                            bold: parent_style.font_weight == FontWeight::Bold,
                            italic: parent_style.font_style == FontStyle::Italic,
                            underline: parent_style.text_decoration_underline,
                            line_through: parent_style.text_decoration_line_through,
                            overline: parent_style.text_decoration_overline,
                            decoration_color: parent_style
                                .text_decoration_color
                                .map(|c| c.to_f32_rgb()),
                            color: parent_style.color.to_f32_rgb(),
                            link_url: link_url.map(String::from),
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: bg,
                            padding: pad,
                            border_radius: br,
                            line_height_factor: resolved_line_height_factor(parent_style, fonts),
                            inline_box: None,
                            disable_ligatures: false,
                            vertical_align: parent_style.vertical_align,
                            text_shadow: parent_style.text_shadow.clone(),
                        },
                    );
                }
            }
            DomNode::Element(el) => {
                let child_index = nodes[..node_index]
                    .iter()
                    .filter(|node| matches!(node, DomNode::Element(_)))
                    .count();
                let preceding_siblings = nodes[..node_index]
                    .iter()
                    .filter_map(|node| match node {
                        DomNode::Element(element) => Some((
                            element.tag_name().to_string(),
                            element
                                .class_list()
                                .into_iter()
                                .map(str::to_string)
                                .collect(),
                        )),
                        _ => None,
                    })
                    .collect();
                let classes = el.class_list();
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index,
                    sibling_count: element_sibling_count,
                    preceding_siblings,
                    following_siblings: Vec::new(),
                    is_empty: false,
                };
                let style = compute_style_with_context(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &selector_ctx,
                );
                if style.display == Display::None {
                    continue;
                }
                let url = if el.tag == HtmlTag::A {
                    el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                } else {
                    link_url
                };
                let mut child_ancestors = ancestors.to_vec();
                child_ancestors.push(AncestorInfo {
                    element: el,
                    child_index,
                    sibling_count: element_sibling_count,
                    preceding_siblings: Vec::new(),
                    following_siblings: Vec::new(),
                    is_empty: false,
                });
                if el.tag == HtmlTag::Table {
                    let mut inner_env = LayoutEnv {
                        rules,
                        fonts,
                        counter_state,
                        filter_defs,
                    };
                    flatten_table(
                        el,
                        &style,
                        available_width,
                        nested_rows,
                        &child_ancestors,
                        child_index,
                        element_sibling_count,
                        &mut inner_env,
                    );
                } else if el.tag == HtmlTag::Svg
                    || (recurse_blocks
                        && style.display != Display::Inline
                        && el.tag != HtmlTag::Br
                        && el.children.is_empty()
                        && (has_background_paint(&style)
                            || style.border.has_any()
                            || !style.box_shadow.is_empty()
                            || style.aspect_ratio.is_some()
                            || style.height.is_some()
                            || style.width.is_some()))
                {
                    let cell_ctx = LayoutContext {
                        viewport: Viewport {
                            width: available_width,
                            height: f32::INFINITY,
                        },
                        parent: ParentBox {
                            content_width: available_width,
                            content_height: None,
                            font_size: parent_style.font_size,
                            percent_width_basis: available_width,
                        },
                        containing_block: None,
                        percent_height_cb: None,
                        root_font_size: parent_style.root_font_size,
                    };
                    let mut inner_env = LayoutEnv {
                        rules,
                        fonts,
                        counter_state,
                        filter_defs,
                    };
                    flatten_element(
                        el,
                        parent_style,
                        &cell_ctx,
                        nested_rows,
                        None,
                        ancestors,
                        0,
                        child_index,
                        element_sibling_count,
                        &selector_ctx.preceding_siblings,
                        &[],
                        &mut inner_env,
                    );
                } else if recurse_blocks || collects_as_inline_text(el.tag) || el.tag == HtmlTag::Br
                {
                    if el.tag == HtmlTag::Br {
                        push_line_break_run(runs, parent_style, fonts);
                    } else {
                        collect_table_cell_content_inner(
                            &el.children,
                            &style,
                            runs,
                            nested_rows,
                            url,
                            rules,
                            fonts,
                            filter_defs,
                            collects_as_inline_text(el.tag),
                            recurse_blocks,
                            false,
                            &child_ancestors,
                            available_width,
                            counter_state,
                        );
                        if recurse_blocks && style.display != Display::Inline && !runs.is_empty() {
                            push_line_break_run(runs, &style, fonts);
                        }
                    }
                }
            }
        }
    }
}

fn push_text_run(runs: &mut Vec<TextRun>, run: TextRun) {
    runs.push(run);
}

fn push_line_break_run(
    runs: &mut Vec<TextRun>,
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    push_text_run(
        runs,
        TextRun {
            text: "\n".to_string(),
            font_size: style.font_size,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: resolve_style_font_family(style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: resolved_line_height_factor(style, fonts),
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: style.text_shadow.clone(),
        },
    );
}
