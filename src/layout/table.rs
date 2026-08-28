use crate::layout::cells::{
    CellAlignment, CellBox, CellBoxModel, CellContent, CellFragmentation, CellPaint, TableCell,
    TableCellHeightConstraint, TableCellSpan, TableCellState,
};
use crate::layout::elements::{
    BoxModel, BoxPaint, CollapsedTableBorders, Container, Image, InlineOffset, IntoLayoutNode,
    LayoutElement, LayoutNode, LayoutSize, LayoutVisitor, LayoutVisitorMut, PageBreak, PaintGroup,
    Positioning, SizeConstraints, Svg, Table, TableBoxDecoration, TableCells, TableFormatting,
    TableFragmentGroup, TableFragmentation, TableGridIdentity, TableInlineGeometry, TableRow,
    TextBlock,
};
use crate::layout::flow_metrics::{BlockFlowSpacing, BlockMargins};
use crate::parser::css::{AncestorInfo, CssRule, CssValue, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BorderCollapse, BoxSizing, ComputedStyle, Display, TableLayout, VerticalAlign, Visibility,
    WhiteSpace, compute_style_with_context, compute_style_with_context_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::EdgeSizes;
use std::collections::HashMap;

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    CounterState, ElementSiblingContext, LayoutBorder, LayoutTreeContext, PageBreakSide, TextRun,
    collects_as_inline_text, element_is_empty, element_sibling_list, flatten_element,
    forward_siblings, has_background_paint, recurses_as_layout_child,
};
use super::helpers::{PseudoBoxContext, build_pseudo_block, pseudo_is_block_like};
use super::inline_formatting::{
    AnonymousInlineFormattingContext, GeneratedBox, GeneratedContentStyles, GeneratedInlineContent,
    IndependentFlowLayout, InlineFormattingChild, layout_mixed_flow_children,
};
#[cfg(test)]
use super::paginate::estimate_element_height;
use super::text::{
    InlineTextSequence, TextWrapOptions, estimate_word_width, has_non_collapsible_text,
    measure_text_intrinsic_widths, parent_line_strut, required_outer_width,
    text_run_line_height_factor, used_font_size, wrap_text_runs,
};

mod collapsed_borders;
use collapsed_borders::{
    CollapsedBorderSources, CollapsedBorderTrack, resolve_collapsed_border_grid,
};

const MAX_COLSPAN: usize = 1000;
const MAX_ROWSPAN: usize = 65_534;

/// Traversal state owned by one table formatting context.
///
/// Table fixup replaces the ordinary DOM walk with anonymous row/cell boxes,
/// but it must not replace the layout viewport, containing-block ancestry, or
/// positioned depth. Keeping those values together prevents table content from
/// falling into a synthetic root context.
#[derive(Clone, Copy)]
pub(crate) struct TableLayoutContext<'context, 'dom> {
    layout: &'context LayoutContext,
    ancestors: &'context [AncestorInfo<'dom>],
    source_index: usize,
    sibling_count: usize,
    positioned_depth: usize,
}

impl<'context, 'dom> TableLayoutContext<'context, 'dom> {
    pub(crate) const fn new(
        layout: &'context LayoutContext,
        ancestors: &'context [AncestorInfo<'dom>],
        source: ElementSiblingContext<'_>,
        positioned_depth: usize,
    ) -> Self {
        Self {
            layout,
            ancestors,
            source_index: source.child_index(),
            sibling_count: source.sibling_count(),
            positioned_depth,
        }
    }
}

#[derive(Clone, Copy)]
struct TableDescendantLayout {
    layout: LayoutContext,
    positioned_depth: usize,
}

impl TableDescendantLayout {
    fn for_table(context: TableLayoutContext<'_, '_>, style: &ComputedStyle) -> Self {
        let layout = if crate::layout::helpers::establishes_containing_block(style) {
            // The exact table-wrapper extent is known only after track sizing.
            // Descendants remain unresolved until the completed principal box
            // supplies that one authoritative containing block below.
            context.layout.with_containing_block(None)
        } else {
            *context.layout
        };
        Self {
            layout,
            positioned_depth: context.positioned_depth,
        }
    }

    fn child_context(self, width: f32, parent: &ComputedStyle) -> LayoutContext {
        self.layout
            .with_parent(width, parent.height, parent.font_size)
    }
}

struct TableRowQuery<F, R> {
    query: Option<F>,
    result: Option<R>,
}

impl<F, R> LayoutVisitor for TableRowQuery<F, R>
where
    F: FnOnce(&TableRow) -> R,
{
    fn visit_table_row(&mut self, element: &TableRow) {
        if let Some(query) = self.query.take() {
            self.result = Some(query(element));
        }
    }
}

fn query_table_row<R>(
    element: &dyn LayoutElement,
    query: impl FnOnce(&TableRow) -> R,
) -> Option<R> {
    let mut visitor = TableRowQuery {
        query: Some(query),
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

struct TableRowUpdate<F>(Option<F>);

impl<F> LayoutVisitorMut for TableRowUpdate<F>
where
    F: FnOnce(&mut TableRow),
{
    fn visit_table_row(&mut self, element: &mut TableRow) {
        if let Some(update) = self.0.take() {
            update(element);
        }
    }
}

fn update_table_row(element: &mut dyn LayoutElement, update: impl FnOnce(&mut TableRow)) {
    element.accept_mut(&mut TableRowUpdate(Some(update)));
}

fn table_row_node(
    grid: TableGridIdentity,
    content: TableCells,
    flow: BlockFlowSpacing,
    formatting: TableFormatting,
    fragmentation: TableFragmentation,
    inline: TableInlineGeometry,
) -> LayoutNode {
    TableRow {
        grid,
        content,
        collapsed_borders: CollapsedTableBorders::default(),
        flow,
        formatting,
        fragmentation,
        inline,
    }
    .boxed()
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowspanId(usize);

impl RowspanId {
    fn allocate(counter: &mut usize) -> Self {
        let id = *counter;
        *counter = counter.saturating_add(1);
        Self(id)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RowspanSlot {
    span_id: Option<RowspanId>,
    remaining_rows: u16,
    min_height: f32,
}

impl RowspanSlot {
    fn occupied(span_id: RowspanId, remaining_rows: usize, min_height: f32) -> Self {
        Self {
            span_id: Some(span_id),
            remaining_rows: remaining_rows.min(MAX_ROWSPAN) as u16,
            min_height,
        }
    }

    fn is_occupied(self) -> bool {
        self.span_id.is_some()
    }

    fn consume_row(&mut self) {
        debug_assert!(self.is_occupied() && self.remaining_rows > 0);
        self.remaining_rows -= 1;
        if self.remaining_rows == 0 {
            *self = Self::default();
        }
    }
}

fn table_cell_allows_soft_wrap(style: &ComputedStyle) -> bool {
    !matches!(style.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre)
        && !style.text_wrap_mode_nowrap
}

/// Keep table intrinsic sizing and final line wrapping on the same set of CSS
/// text controls.  In particular, run tokenization must agree about preserved
/// whitespace, hyphen/CJK opportunities, bidi, and the first-line indent.
fn table_cell_text_wrap_options(
    style: &ComputedStyle,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
) -> TextWrapOptions {
    let wrap_width = if table_cell_allows_soft_wrap(style) {
        available_width.max(0.0)
    } else {
        f32::MAX
    };
    TextWrapOptions::new(
        wrap_width,
        used_font_size(style, fonts),
        text_run_line_height_factor(style, fonts),
        style.overflow_wrap,
    )
    .with_parent_strut(parent_line_strut(style, fonts))
    .with_rtl(style.direction_rtl)
    .with_bidi_override(style.bidi_override)
    .with_bidi_plaintext(style.bidi_plaintext)
    .with_word_break_keep_all(style.word_break_keep_all)
    .with_hyphens_manual(style.hyphens_manual)
    .with_white_space(style.white_space)
    .with_text_indent(style.text_indent.resolve(available_width))
}

/// Padding plus the cell-owned share of each border, grouped as one reusable
/// edge value so sizing, wrapping, and painting cannot swap or omit a side.
fn table_cell_border_inset_factor(border_collapse: BorderCollapse) -> f32 {
    if border_collapse == BorderCollapse::Collapse {
        0.5
    } else {
        1.0
    }
}

fn table_cell_content_insets(
    style: &ComputedStyle,
    border: &LayoutBorder,
    border_collapse: BorderCollapse,
) -> EdgeSizes {
    style.padding + border.widths() * table_cell_border_inset_factor(border_collapse)
}

/// The table root has a different box model in collapsed-border mode: its
/// authored padding is ignored and winning outer borders, not the authored
/// border widths, determine the border-box extent. Centralizing those rules
/// prevents width, height, row offsets, and paint from each approximating the
/// model independently.
#[derive(Debug, Clone, Copy)]
struct TableRootBoxModel {
    formatting: TableFormatting,
    box_sizing: BoxSizing,
    padding: EdgeSizes,
    authored_border: EdgeSizes,
}

impl TableRootBoxModel {
    fn new(
        formatting: TableFormatting,
        box_sizing: BoxSizing,
        authored_padding: EdgeSizes,
        authored_border: EdgeSizes,
    ) -> Self {
        Self {
            formatting,
            box_sizing,
            padding: formatting.root_padding(authored_padding),
            authored_border,
        }
    }

    fn grid_insets(self) -> EdgeSizes {
        if self.formatting.is_collapsed() {
            EdgeSizes::ZERO
        } else {
            self.padding + self.authored_border
        }
    }

    fn resolve_inline_extent(self, specified: f32) -> f32 {
        if self.box_sizing == BoxSizing::BorderBox {
            (specified - self.grid_insets().horizontal()).max(0.0)
        } else {
            specified.max(0.0)
        }
    }

    fn resolve_block_extent(self, specified: f32) -> f32 {
        if self.box_sizing == BoxSizing::BorderBox {
            (specified - self.grid_insets().vertical()).max(0.0)
        } else {
            specified.max(0.0)
        }
    }

    /// Convert the resolved table width/height into the span between the outer
    /// collapsed grid lines. With `border-box`, the specified size already
    /// includes the two half-border overhangs; with `content-box`, they extend
    /// beyond the specified content extent.
    fn collapsed_grid_extent(self, resolved: f32, outer_border_extent: f32) -> f32 {
        if self.formatting.is_collapsed() && self.box_sizing == BoxSizing::BorderBox {
            (resolved - outer_border_extent).max(0.0)
        } else {
            resolved.max(0.0)
        }
    }
}

/// Minimum outer width a nested layout element wants inside an auto-sized table
/// cell. Used so shrink-to-fit columns stay at least as wide as fixed-width
/// block descendants, nested tables, and replaced content.
fn nested_element_preferred_width(element: &dyn LayoutElement) -> f32 {
    #[derive(Default)]
    struct PreferredWidth(f32);

    impl LayoutVisitor for PreferredWidth {
        fn visit_table_row(&mut self, element: &TableRow) {
            self.0 = element.box_inline_extent();
        }

        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element
                .box_model
                .size
                .width
                .fixed_value()
                .unwrap_or_default();
        }

        fn visit_container(&mut self, element: &Container) {
            self.0 = element
                .box_model
                .size
                .width
                .fixed_value()
                .unwrap_or_default();
        }

        fn visit_image(&mut self, element: &Image) {
            self.0 = element.geometry.size.width;
        }

        fn visit_svg(&mut self, element: &Svg) {
            self.0 = element.geometry.size.width;
        }
    }

    let mut width = PreferredWidth::default();
    element.accept(&mut width);
    width.0
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
    background_color: Option<crate::types::Color>,
    collapsed: bool,
    column_border: LayoutBorder,
    column_group_border: LayoutBorder,
}

fn assign_column_border(
    columns: &mut [TableColumnInfo],
    start: usize,
    span: usize,
    border: LayoutBorder,
) {
    for column in columns.iter_mut().skip(start).take(span) {
        column.column_border = border;
    }
}

fn assign_column_group_border(
    columns: &mut [TableColumnInfo],
    start: usize,
    span: usize,
    border: LayoutBorder,
) {
    let end = start.saturating_add(span).min(columns.len());
    for (index, column) in columns.iter_mut().enumerate().take(end).skip(start) {
        column.column_group_border.top = border.top;
        column.column_group_border.bottom = border.bottom;
        if index == start {
            column.column_group_border.left = border.left;
        }
        if index + 1 == end {
            column.column_group_border.right = border.right;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableBoxRole {
    Table,
    Caption,
    ColumnGroup,
    Column,
    HeaderGroup,
    RowGroup,
    FooterGroup,
    Row,
    Cell,
}

const ANONYMOUS_TABLE: &str = "#anonymous-table";
const ANONYMOUS_TABLE_ROW: &str = "#anonymous-table-row";
const ANONYMOUS_TABLE_CELL: &str = "#anonymous-table-cell";

struct AnonymousTableRow<'a> {
    element: ElementNode,
    generated_cells: Vec<GeneratedInlineContent<'a>>,
}

enum TableRowNode<'a> {
    Element(&'a ElementNode),
    Anonymous(AnonymousTableRow<'a>),
}

impl TableRowNode<'_> {
    fn element(&self) -> &ElementNode {
        match self {
            Self::Element(element) => element,
            Self::Anonymous(row) => &row.element,
        }
    }
}

struct GeneratedCellLayout<'a> {
    runs: &'a mut Vec<TextRun>,
    blocks: &'a mut Vec<LayoutNode>,
    parent_style: &'a ComputedStyle,
    available_width: f32,
    fonts: &'a HashMap<String, TtfFont>,
    filter_defs: &'a HashMap<String, ElementNode>,
    counter_state: &'a mut CounterState,
    resources: &'a mut crate::security::resources::ResourceLoader,
}

fn append_generated_cell_layout(
    generated: Option<GeneratedBox<'_>>,
    output: GeneratedCellLayout<'_>,
) {
    let Some(generated) = generated else {
        return;
    };
    if pseudo_is_block_like(generated.style()) {
        if let Some(block) = generated_table_cell_boundary(
            generated,
            output.parent_style,
            output.available_width,
            output.fonts,
            output.filter_defs,
            output.counter_state,
            &mut *output.resources,
        ) {
            output.blocks.push(block);
        }
    } else {
        generated.append_inline(
            output.runs,
            output.fonts,
            output.counter_state,
            output.resources,
        );
    }
}

struct TableRowSource<'a> {
    node: TableRowNode<'a>,
    section_index: usize,
    section_size: usize,
    section: Option<&'a ElementNode>,
    section_child_index: usize,
    section_sibling_count: usize,
    section_role: EffectiveTableSectionRole,
}

impl<'a> TableRowSource<'a> {
    fn generated_cell_content(&self, cell_index: usize) -> GeneratedInlineContent<'a> {
        match &self.node {
            TableRowNode::Element(_) => GeneratedInlineContent::default(),
            TableRowNode::Anonymous(row) => row
                .generated_cells
                .get(cell_index)
                .copied()
                .unwrap_or_default(),
        }
    }
}

/// The role a row group actually has after CSS Tables' first-header/first-footer
/// normalization. Declared `table-header-group` and `table-footer-group` values
/// are not sufficient: only the first owned group of each kind is special and
/// every later one participates in the ordinary body lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveTableSectionRole {
    Header,
    Body,
    Footer,
}

impl EffectiveTableSectionRole {
    fn claim(declared: TableBoxRole, header_claimed: &mut bool, footer_claimed: &mut bool) -> Self {
        match declared {
            TableBoxRole::HeaderGroup if !*header_claimed => {
                *header_claimed = true;
                Self::Header
            }
            TableBoxRole::FooterGroup if !*footer_claimed => {
                *footer_claimed = true;
                Self::Footer
            }
            TableBoxRole::HeaderGroup | TableBoxRole::RowGroup | TableBoxRole::FooterGroup => {
                Self::Body
            }
            _ => Self::Body,
        }
    }

    const fn paint_order(self) -> u8 {
        match self {
            Self::Header => 0,
            Self::Body => 1,
            Self::Footer => 2,
        }
    }

    const fn fragmentation(
        self,
        avoid_inside: bool,
        avoid_group: Option<TableFragmentGroup>,
    ) -> TableFragmentation {
        TableFragmentation {
            repeats_as_header: matches!(self, Self::Header),
            repeats_as_footer: matches!(self, Self::Footer),
            avoid_inside,
            avoid_group,
        }
    }
}

fn anonymous_table_box(role: TableBoxRole, children: Vec<DomNode>) -> ElementNode {
    let mut element = ElementNode::new(HtmlTag::Unknown);
    element.raw_tag_name = match role {
        TableBoxRole::Table => ANONYMOUS_TABLE,
        TableBoxRole::Row => ANONYMOUS_TABLE_ROW,
        TableBoxRole::Cell => ANONYMOUS_TABLE_CELL,
        _ => "#anonymous-table-box",
    }
    .to_string();
    element.children = children;
    element
}

pub(crate) fn anonymous_table_from_cells(cells: &[&ElementNode]) -> ElementNode {
    let row = anonymous_table_box(
        TableBoxRole::Row,
        cells
            .iter()
            .map(|cell| DomNode::Element((*cell).clone()))
            .collect(),
    );
    anonymous_table_box(TableBoxRole::Table, vec![DomNode::Element(row)])
}

fn anonymous_table_row<'a>(
    children: Vec<(DomNode, Option<TableBoxRole>)>,
    generated: GeneratedInlineContent<'a>,
) -> AnonymousTableRow<'a> {
    let mut row_children = Vec::new();
    let mut generated_cells = Vec::new();
    let mut anonymous_cell_children = Vec::new();
    let mut anonymous_cell_generated = GeneratedInlineContent::from_boxes(generated.before(), None);
    let flush_anonymous_cell =
        |row_children: &mut Vec<DomNode>,
         generated_cells: &mut Vec<GeneratedInlineContent<'a>>,
         anonymous_cell_children: &mut Vec<DomNode>,
         anonymous_cell_generated: &mut GeneratedInlineContent<'a>| {
            if anonymous_cell_children.is_empty() && anonymous_cell_generated.is_empty() {
                return;
            }
            row_children.push(DomNode::Element(anonymous_table_box(
                TableBoxRole::Cell,
                std::mem::take(anonymous_cell_children),
            )));
            generated_cells.push(std::mem::take(anonymous_cell_generated));
        };

    for (child, role) in children {
        if role == Some(TableBoxRole::Cell) {
            flush_anonymous_cell(
                &mut row_children,
                &mut generated_cells,
                &mut anonymous_cell_children,
                &mut anonymous_cell_generated,
            );
            row_children.push(child);
            generated_cells.push(GeneratedInlineContent::default());
        } else {
            anonymous_cell_children.push(child);
        }
    }
    anonymous_cell_generated =
        GeneratedInlineContent::from_boxes(anonymous_cell_generated.before(), generated.after());
    flush_anonymous_cell(
        &mut row_children,
        &mut generated_cells,
        &mut anonymous_cell_children,
        &mut anonymous_cell_generated,
    );
    AnonymousTableRow {
        element: anonymous_table_box(TableBoxRole::Row, row_children),
        generated_cells,
    }
}

fn push_anonymous_table_row<'a>(
    row_sources: &mut Vec<TableRowSource<'a>>,
    improper_children: &mut Vec<(DomNode, Option<TableBoxRole>)>,
    generated: &mut GeneratedInlineContent<'a>,
    section_child_index: usize,
    section_sibling_count: usize,
) {
    if improper_children.is_empty() && generated.is_empty() {
        return;
    }
    let section_index = row_sources.len();
    row_sources.push(TableRowSource {
        node: TableRowNode::Anonymous(anonymous_table_row(
            std::mem::take(improper_children),
            std::mem::take(generated),
        )),
        section_index,
        section_size: 1,
        section: None,
        section_child_index,
        section_sibling_count,
        section_role: EffectiveTableSectionRole::Body,
    });
}

fn generated_table_cell_boundary(
    generated: GeneratedBox<'_>,
    parent_style: &ComputedStyle,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    filter_defs: &HashMap<String, ElementNode>,
    counter_state: &mut CounterState,
    resources: &mut crate::security::resources::ResourceLoader,
) -> Option<LayoutNode> {
    if pseudo_is_block_like(generated.style()) {
        return Some(build_pseudo_block(
            generated.style(),
            generated.originating_element(),
            PseudoBoxContext::new(available_width, fonts, filter_defs, resources),
            counter_state,
            generated.style().display == Display::ListItem,
        ));
    }

    let mut runs = Vec::new();
    generated.append_inline(&mut runs, fonts, counter_state, resources);
    AnonymousInlineFormattingContext::new(parent_style, available_width, fonts).layout_runs(runs)
}

fn anonymous_table_box_role(element: &ElementNode) -> Option<TableBoxRole> {
    match element.raw_tag_name.as_str() {
        ANONYMOUS_TABLE => Some(TableBoxRole::Table),
        ANONYMOUS_TABLE_ROW => Some(TableBoxRole::Row),
        ANONYMOUS_TABLE_CELL => Some(TableBoxRole::Cell),
        _ => None,
    }
}

fn is_anonymous_table_box(element: &ElementNode) -> bool {
    anonymous_table_box_role(element).is_some()
}

fn push_table_dom_ancestor<'a>(
    ancestors: &mut Vec<AncestorInfo<'a>>,
    element: &'a ElementNode,
    siblings: ElementSiblingContext<'_>,
) {
    if !is_anonymous_table_box(element) {
        ancestors.push(siblings.ancestor(element, element_is_empty(element)));
    }
}

pub(crate) fn anonymous_table_box_style(
    element: &ElementNode,
    parent: &ComputedStyle,
) -> Option<ComputedStyle> {
    let role = anonymous_table_box_role(element)?;
    let mut style = compute_style_with_context(
        HtmlTag::Unknown,
        None,
        parent,
        &[],
        "",
        &[],
        None,
        &HashMap::new(),
        &SelectorContext::default(),
    );
    style.display = match role {
        TableBoxRole::Table => Display::Table,
        TableBoxRole::Row => Display::TableRow,
        TableBoxRole::Cell => Display::TableCell,
        _ => style.display,
    };
    Some(style)
}

fn is_proper_table_child(role: Option<TableBoxRole>) -> bool {
    matches!(
        role,
        Some(
            TableBoxRole::Caption
                | TableBoxRole::ColumnGroup
                | TableBoxRole::Column
                | TableBoxRole::HeaderGroup
                | TableBoxRole::RowGroup
                | TableBoxRole::FooterGroup
                | TableBoxRole::Row
        )
    )
}

fn table_box_role(el: &ElementNode, style: &ComputedStyle) -> Option<TableBoxRole> {
    if let Some(role) = anonymous_table_box_role(el) {
        return Some(role);
    }
    match el.tag {
        HtmlTag::Table => return Some(TableBoxRole::Table),
        HtmlTag::Caption => return Some(TableBoxRole::Caption),
        HtmlTag::Colgroup => return Some(TableBoxRole::ColumnGroup),
        HtmlTag::Col => return Some(TableBoxRole::Column),
        HtmlTag::Thead => return Some(TableBoxRole::HeaderGroup),
        HtmlTag::Tbody => return Some(TableBoxRole::RowGroup),
        HtmlTag::Tfoot => return Some(TableBoxRole::FooterGroup),
        HtmlTag::Tr => return Some(TableBoxRole::Row),
        HtmlTag::Td | HtmlTag::Th => return Some(TableBoxRole::Cell),
        _ => {}
    }

    match style.display {
        Display::Table | Display::InlineTable => Some(TableBoxRole::Table),
        Display::TableCaption => Some(TableBoxRole::Caption),
        Display::TableColumnGroup => Some(TableBoxRole::ColumnGroup),
        Display::TableColumn => Some(TableBoxRole::Column),
        Display::TableHeaderGroup => Some(TableBoxRole::HeaderGroup),
        Display::TableRowGroup => Some(TableBoxRole::RowGroup),
        Display::TableFooterGroup => Some(TableBoxRole::FooterGroup),
        Display::TableRow => Some(TableBoxRole::Row),
        Display::TableCell => Some(TableBoxRole::Cell),
        _ => None,
    }
}

fn table_child_role(
    child_el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> (ComputedStyle, Option<TableBoxRole>) {
    let child_style = compute_column_style(
        child_el,
        parent_style,
        rules,
        ancestors,
        child_index,
        sibling_count,
    );
    let role = table_box_role(child_el, &child_style);
    (child_style, role)
}

fn row_child_is_table_cell(
    child_el: &ElementNode,
    row_style: &ComputedStyle,
    rules: &[CssRule],
    row_ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> bool {
    let (_, role) = table_child_role(
        child_el,
        row_style,
        rules,
        row_ancestors,
        child_index,
        sibling_count,
    );
    role.is_none_or(|role| role == TableBoxRole::Cell)
}

#[derive(Clone, Copy)]
struct TableCellSource<'a> {
    element: &'a ElementNode,
    authored_child_index: usize,
}

impl TableCellSource<'_> {
    fn siblings<'a>(
        self,
        authored_siblings: &'a [(String, Vec<String>)],
    ) -> ElementSiblingContext<'a> {
        ElementSiblingContext::new(self.authored_child_index, authored_siblings.len())
            .with_neighbors(
                authored_siblings
                    .get(..self.authored_child_index)
                    .unwrap_or(&[]),
                forward_siblings(authored_siblings, self.authored_child_index),
            )
    }
}

fn table_row_cell_elements<'a>(
    row: &'a ElementNode,
    row_style: &ComputedStyle,
    rules: &[CssRule],
    row_ancestors: &[AncestorInfo],
) -> Vec<TableCellSource<'a>> {
    if table_box_role(row, row_style) != Some(TableBoxRole::Row) {
        return vec![TableCellSource {
            element: row,
            authored_child_index: 0,
        }];
    }

    let row_child_count = row
        .children
        .iter()
        .filter(|child| matches!(child, DomNode::Element(_)))
        .count();
    row.children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(cell_el) => Some(cell_el),
            _ => None,
        })
        .enumerate()
        .filter_map(|(row_child_idx, cell_el)| {
            row_child_is_table_cell(
                cell_el,
                row_style,
                rules,
                row_ancestors,
                row_child_idx,
                row_child_count,
            )
            .then_some(TableCellSource {
                element: cell_el,
                authored_child_index: row_child_idx,
            })
        })
        .collect()
}

fn resolve_table_percentage_width(table_width: f32, percent: f32) -> f32 {
    // Percentage `<col>` and `<colgroup>` widths resolve against the table
    // width itself. Border-spacing is applied later when laying out the cells
    // so it must not shrink the percentage basis.
    table_width * percent
}

fn style_background_rgba(style: &ComputedStyle) -> Option<crate::types::Color> {
    style.background_color
}

fn table_cell_is_hidden(style: &ComputedStyle) -> bool {
    style.visibility != Visibility::Visible
}

fn hide_table_cell_paint(cell: &mut TableCell) {
    cell.layout.box_model.minimum_block_size = cell
        .layout
        .box_model
        .minimum_block_size
        .max(cell.row_block_extent());
    cell.layout.content.lines.clear();
    cell.layout.content.children.clear();
    cell.layout.paint.background = Default::default();
    cell.layout.box_model.border = LayoutBorder::default();
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
    if let Some(style) = anonymous_table_box_style(el, parent_style) {
        return style;
    }
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
    crate::layout::cells::TableRowCells::row_block_extent(cells)
}

fn enforce_row_min_height(cells: &mut [TableCell], min_height: f32) {
    if min_height <= 0.0 {
        return;
    }
    let current = row_height_from_cells(cells);
    if current >= min_height {
        return;
    }
    if let Some(cell) = cells.iter_mut().find(|cell| cell.span.rows != 0) {
        cell.layout.box_model.minimum_block_size =
            cell.layout.box_model.minimum_block_size.max(min_height);
    } else if let Some(cell) = cells.first_mut() {
        cell.layout.box_model.minimum_block_size =
            cell.layout.box_model.minimum_block_size.max(min_height);
    }
}

fn stretch_table_rows_to_min_height(
    output: &mut [LayoutNode],
    target_table_height: f32,
    vertical_edge_spacing: f32,
) {
    if target_table_height <= 0.0 {
        return;
    }
    let mut row_indices = Vec::new();
    let mut rows_height = 0.0f32;
    for (idx, elem) in output.iter().enumerate() {
        if let Some(row_height) = query_table_row(elem.as_ref(), |row| {
            row_height_from_cells(&row.content.cells)
        }) {
            row_indices.push(idx);
            rows_height += row_height;
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
        update_table_row(output[idx].as_mut(), |row| {
            let target_row_height = row_height_from_cells(&row.content.cells) + extra_per_row;
            enforce_row_min_height(&mut row.content.cells, target_row_height);
        });
    }
}

fn collapsed_outer_vertical_border_extent(rows: &[LayoutNode]) -> f32 {
    let first_top = rows.iter().find_map(|row| {
        query_table_row(row.as_ref(), |row| {
            row.content
                .cells
                .iter()
                .find(|cell| cell.span.rows != 0)
                .map(|cell| cell.layout.box_model.border.top.width / 2.0)
        })
        .flatten()
    });
    let last_bottom = rows.iter().rev().find_map(|row| {
        query_table_row(row.as_ref(), |row| {
            row.content
                .cells
                .iter()
                .rev()
                .find(|cell| cell.span.rows != 0)
                .map(|cell| cell.layout.box_model.border.bottom.width / 2.0)
        })
        .flatten()
    });
    first_top.unwrap_or(0.0) + last_bottom.unwrap_or(0.0)
}

fn table_cell_vertical_align(value: VerticalAlign) -> VerticalAlign {
    match value {
        VerticalAlign::Middle | VerticalAlign::Bottom | VerticalAlign::Top => value,
        _ => VerticalAlign::Top,
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
            if table_box_role(cell_el, &ComputedStyle::default()) != Some(TableBoxRole::Cell)
                && !matches!(cell_el.tag, HtmlTag::Div | HtmlTag::Span)
            {
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
    filter_dpi: f32,
    counter_state: &mut CounterState,
    resources: &mut crate::security::resources::ResourceLoader,
    available_width: f32,
    descendant_layout: TableDescendantLayout,
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
    {
        let mut flow_env = LayoutEnv {
            rules,
            fonts,
            counter_state,
            resources,
            filter_defs,
            filter_dpi,
        };
        layout_table_cell_flow(
            &caption_el.children,
            &mut runs,
            &mut nested,
            TableCellFlowContext {
                style: caption_style,
                ancestors: &caption_ancestors,
                available_width: available_width.max(0.0),
                descendant_layout,
            },
            &mut flow_env,
        );
    }
    let text_width: f32 = runs
        .iter()
        .map(|run| {
            estimate_word_width(
                &run.text,
                run.font_size,
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                fonts,
            )
        })
        .sum();
    let nested_width = nested
        .iter()
        .map(|element| nested_element_preferred_width(element.as_ref()))
        .fold(0.0f32, f32::max);
    let caption_border = LayoutBorder::from_computed(&caption_style.border, caption_style.color);
    text_width.max(nested_width)
        + caption_style.padding.horizontal()
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

fn resolve_table_inner_width(
    style: &ComputedStyle,
    available_width: f32,
    box_model: TableRootBoxModel,
) -> f32 {
    let containing_width = (available_width - style.margin.horizontal()).max(0.0);
    let specified = style.width.or_else(|| {
        style
            .percentage_sizing
            .width
            .map(|percent| containing_width * percent / 100.0)
    });
    specified.map_or(containing_width, |width| {
        box_model.resolve_inline_extent(width)
    })
}

fn resolve_table_inner_height(style: &ComputedStyle, box_model: TableRootBoxModel) -> Option<f32> {
    SizeConstraints::new(style.min_height, style.max_height)
        .constrain_preferred(style.height)
        .map(|height| box_model.resolve_block_extent(height))
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
        let mut row_style =
            anonymous_table_box_style(first_row, table_style).unwrap_or_else(|| {
                compute_style_with_context(
                    first_row.tag,
                    first_row.style_attr(),
                    table_style,
                    rules,
                    first_row.tag_name(),
                    &row_classes,
                    first_row.id(),
                    &first_row.attributes,
                    &row_selector_ctx,
                )
            });
        row_style.width = Some(table_width);

        let mut col_pos = 0usize;
        let authored_siblings = element_sibling_list(&first_row.children);
        let mut authored_child_index = 0usize;
        for child in &first_row.children {
            let DomNode::Element(cell_el) = child else {
                continue;
            };
            let cell_siblings =
                ElementSiblingContext::new(authored_child_index, authored_siblings.len())
                    .with_neighbors(
                        authored_siblings.get(..authored_child_index).unwrap_or(&[]),
                        forward_siblings(&authored_siblings, authored_child_index),
                    );
            authored_child_index += 1;
            if table_box_role(cell_el, &ComputedStyle::default()) != Some(TableBoxRole::Cell)
                && !matches!(cell_el.tag, HtmlTag::Div | HtmlTag::Span)
            {
                continue;
            }
            let colspan = parse_cell_colspan(cell_el);

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            push_table_dom_ancestor(
                &mut cell_ancestors,
                first_row,
                ElementSiblingContext::new(
                    row_selector_ctx.child_index,
                    row_selector_ctx.sibling_count,
                ),
            );
            let cell_selector_ctx =
                cell_siblings.selector_context(&cell_ancestors, element_is_empty(cell_el));
            let cell_style = anonymous_table_box_style(cell_el, &row_style).unwrap_or_else(|| {
                compute_style_with_context(
                    cell_el.tag,
                    cell_el.style_attr(),
                    &row_style,
                    rules,
                    cell_el.tag_name(),
                    &cell_classes,
                    cell_el.id(),
                    &cell_el.attributes,
                    &cell_selector_ctx,
                )
            });

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
        let mut distributed = (unresolved_count > 1)
            .then(|| {
                crate::layout::units::LayoutUnitDiffuser::new(
                    crate::layout::units::LayoutUnit::from_points(remaining_width),
                    unresolved_count,
                )
            })
            .flatten();
        for width in &mut col_widths {
            if width.is_none() {
                *width = Some(if unresolved_count == 1 {
                    remaining_width
                } else {
                    distributed
                        .as_mut()
                        .and_then(Iterator::next)
                        .unwrap_or_default()
                        .to_points()
                });
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
#[allow(clippy::too_many_arguments)]
fn collapse_outer_horizontal_borders(
    rows: &[&ElementNode],
    row_section_indices: &[usize],
    row_section_sizes: &[usize],
    row_section_elements: &[Option<&ElementNode>],
    row_section_child_indices: &[usize],
    row_section_sibling_counts: &[usize],
    table_style: &ComputedStyle,
    rules: &[CssRule],
    table_ancestors: &[AncestorInfo],
) -> (f32, f32) {
    if table_style.border_collapse != BorderCollapse::Collapse {
        return (0.0, 0.0);
    }
    let Some((row_idx, first_row)) = rows.iter().enumerate().next() else {
        return (0.0, 0.0);
    };
    let mut row_ancestors = table_ancestors.to_vec();
    if let Some(section_el) = row_section_elements
        .get(row_idx)
        .and_then(|section| *section)
    {
        row_ancestors.push(AncestorInfo {
            element: section_el,
            child_index: row_section_child_indices.get(row_idx).copied().unwrap_or(0),
            sibling_count: row_section_sibling_counts
                .get(row_idx)
                .copied()
                .unwrap_or(0),
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        });
    }
    let row_parent_style = row_section_elements
        .get(row_idx)
        .and_then(|section| *section)
        .map(|section_el| {
            compute_column_style(
                section_el,
                table_style,
                rules,
                table_ancestors,
                row_section_child_indices.get(row_idx).copied().unwrap_or(0),
                row_section_sibling_counts
                    .get(row_idx)
                    .copied()
                    .unwrap_or(0),
            )
        });
    let row_classes = first_row.class_list();
    let row_selector_ctx = SelectorContext {
        ancestors: row_ancestors,
        child_index: row_section_indices.get(row_idx).copied().unwrap_or(0),
        sibling_count: row_section_sizes.get(row_idx).copied().unwrap_or(1),
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let row_style =
        anonymous_table_box_style(first_row, row_parent_style.as_ref().unwrap_or(table_style))
            .unwrap_or_else(|| {
                compute_style_with_context(
                    first_row.tag,
                    first_row.style_attr(),
                    row_parent_style.as_ref().unwrap_or(table_style),
                    rules,
                    first_row.tag_name(),
                    &row_classes,
                    first_row.id(),
                    &first_row.attributes,
                    &row_selector_ctx,
                )
            });
    let cells = table_row_cell_elements(first_row, &row_style, rules, &row_selector_ctx.ancestors);
    if cells.is_empty() {
        return (0.0, 0.0);
    }
    let cell_count = cells.len();
    let authored_siblings = element_sibling_list(&first_row.children);
    let mut cell_ancestors = row_selector_ctx.ancestors.clone();
    push_table_dom_ancestor(
        &mut cell_ancestors,
        first_row,
        ElementSiblingContext::new(row_selector_ctx.child_index, row_selector_ctx.sibling_count),
    );
    let cell_border = |source: TableCellSource<'_>| -> ComputedStyle {
        let cell = source.element;
        let classes = cell.class_list();
        anonymous_table_box_style(cell, &row_style).unwrap_or_else(|| {
            compute_style_with_context(
                cell.tag,
                cell.style_attr(),
                &row_style,
                rules,
                cell.tag_name(),
                &classes,
                cell.id(),
                &cell.attributes,
                &source
                    .siblings(&authored_siblings)
                    .selector_context(&cell_ancestors, element_is_empty(cell)),
            )
        })
    };
    let first = cell_border(cells[0]);
    let last = cell_border(cells[cell_count - 1]);
    (
        first
            .border
            .left
            .used_width()
            .max(table_style.border.left.used_width()),
        last.border
            .right
            .used_width()
            .max(table_style.border.right.used_width()),
    )
}

thread_local! {
    /// Per-document memo of the table auto-sizing pass's per-cell preferred
    /// and minimum content widths, keyed by `(cell element pointer, table
    /// inner-width bits)`.
    ///
    /// The sizing pass measures every cell — including flattening any nested
    /// tables just to read their width — and an ancestor table is re-flattened
    /// once per pass (sizing + placement) at every nesting level, so nested
    /// cells are otherwise re-measured `2^depth` times. Caching the reduced
    /// widths collapses that to once per `(cell, width)`.
    ///
    /// Only populated for cells whose measurement touches no CSS counter or
    /// quote state (see the sizing pass in [`flatten_table`]), so a cached
    /// width never depends on counter context. Cleared at the start of every
    /// top-level layout via [`reset_table_sizing_cache`], so pointers from a
    /// freed DOM are never reused as keys.
    static TABLE_CELL_SIZING_CACHE: std::cell::RefCell<HashMap<(usize, u32), (f32, f32)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Clear the per-document table auto-sizing memo. Called once at the start of
/// each top-level layout.
pub(crate) fn reset_table_sizing_cache() {
    TABLE_CELL_SIZING_CACHE.with(|c| c.borrow_mut().clear());
}

fn table_cell_sizing_get(key: (usize, u32)) -> Option<(f32, f32)> {
    TABLE_CELL_SIZING_CACHE.with(|c| c.borrow().get(&key).copied())
}

fn table_cell_sizing_insert(key: (usize, u32), widths: (f32, f32)) {
    TABLE_CELL_SIZING_CACHE.with(|c| {
        c.borrow_mut().insert(key, widths);
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn flatten_table(
    el: &ElementNode,
    style: &ComputedStyle,
    output: &mut Vec<LayoutNode>,
    generated_content: GeneratedInlineContent<'_>,
    env: &mut LayoutEnv,
    context: TableLayoutContext<'_, '_>,
) {
    let available_width = context.layout.available_width();
    let ancestors = context.ancestors;
    let table_child_index = context.source_index;
    let table_sibling_count = context.sibling_count;
    let table_grid = TableGridIdentity::from_source_path(
        context
            .ancestors
            .iter()
            .map(|ancestor| ancestor.child_index)
            .chain(std::iter::once(table_child_index)),
    );
    let descendant_layout = TableDescendantLayout::for_table(context, style);
    let rules = env.rules;
    let fonts = env.fonts;
    let filter_defs = env.filter_defs;
    let filter_dpi = env.filter_dpi;
    let counter_state = &mut *env.counter_state;
    let resources = &mut *env.resources;
    let mut measurement_counter_state = counter_state.clone();
    let table_border = LayoutBorder::from_computed(&style.border, style.color);
    let effective_border_spacing = style.border_spacing;
    let effective_border_spacing_vertical = style.border_spacing_vertical;
    let table_formatting = TableFormatting::new(style.border_collapse, effective_border_spacing);
    let table_box_model = TableRootBoxModel::new(
        table_formatting,
        style.box_sizing,
        style.padding,
        table_border.widths(),
    );
    let inner_width = resolve_table_inner_width(style, available_width, table_box_model);
    let table_grid_box_insets = table_box_model.grid_insets();
    // Row construction needs the table grid's local inset, but the table
    // wrapper's final inline position cannot be resolved until auto layout has
    // produced the used table width below.
    let table_inline_geometry = TableInlineGeometry::new(
        InlineOffset::ZERO,
        InlineOffset::new(table_grid_box_insets.left),
    );

    // Build ancestor chain: everything above + the table element itself.
    let mut table_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    push_table_dom_ancestor(
        &mut table_ancestors,
        el,
        ElementSiblingContext::new(table_child_index, table_sibling_count),
    );

    // Collect all <tr> elements (from direct children, thead, tbody, tfoot).
    // Track section-relative indices so nth-child counts within each section
    // (thead, tbody, tfoot) as browsers do, not globally.
    // Also track the section element so descendant selectors can see it.
    let mut row_sources = Vec::new();
    let mut header_section_claimed = false;
    let mut footer_section_claimed = false;
    let mut improper_children = Vec::new();
    let mut improper_generated =
        GeneratedInlineContent::from_boxes(generated_content.before(), None);
    // `<caption>` / `display: table-caption` boxes and their positions among
    // the table's element children, for selector matching.
    let mut captions: Vec<(&ElementNode, usize)> = Vec::new();
    let section_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();
    for (section_child_idx, child) in el.children.iter().enumerate() {
        let (child_el, child_role) = match child {
            DomNode::Element(child_el) => {
                let (_, role) = table_child_role(
                    child_el,
                    style,
                    rules,
                    &table_ancestors,
                    section_child_idx,
                    section_count,
                );
                if !is_proper_table_child(role) {
                    improper_children.push((DomNode::Element(child_el.clone()), role));
                    continue;
                }
                (child_el, role)
            }
            DomNode::Text(text) if has_non_collapsible_text(text) => {
                improper_children.push((DomNode::Text(text.clone()), None));
                continue;
            }
            DomNode::Text(_) => continue,
        };

        push_anonymous_table_row(
            &mut row_sources,
            &mut improper_children,
            &mut improper_generated,
            section_child_idx,
            section_count,
        );
        match child_role {
            Some(TableBoxRole::Caption) => {
                captions.push((child_el, section_child_idx));
            }
            Some(TableBoxRole::Row) => {
                // Direct <tr> child of <table> — standalone section
                let section_index = row_sources.len();
                row_sources.push(TableRowSource {
                    node: TableRowNode::Element(child_el),
                    section_index,
                    section_size: 1,
                    section: None,
                    section_child_index: section_child_idx,
                    section_sibling_count: section_count,
                    section_role: EffectiveTableSectionRole::Body,
                });
            }
            Some(
                declared_role @ (TableBoxRole::HeaderGroup
                | TableBoxRole::RowGroup
                | TableBoxRole::FooterGroup),
            ) => {
                let section_role = EffectiveTableSectionRole::claim(
                    declared_role,
                    &mut header_section_claimed,
                    &mut footer_section_claimed,
                );
                let mut section_ancestors = table_ancestors.clone();
                section_ancestors.push(AncestorInfo {
                    element: child_el,
                    child_index: section_child_idx,
                    sibling_count: section_count,
                    preceding_siblings: Vec::new(),
                    following_siblings: Vec::new(),
                    is_empty: false,
                });
                let group_style = compute_column_style(
                    child_el,
                    style,
                    rules,
                    &table_ancestors,
                    section_child_idx,
                    section_count,
                );
                let group_child_count = child_el
                    .children
                    .iter()
                    .filter(|gc| matches!(gc, DomNode::Element(_)))
                    .count();
                let section_rows: Vec<&ElementNode> = child_el
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(group_child_idx, gc)| {
                        let DomNode::Element(g) = gc else {
                            return None;
                        };
                        let (_, role) = table_child_role(
                            g,
                            &group_style,
                            rules,
                            &section_ancestors,
                            group_child_idx,
                            group_child_count,
                        );
                        matches!(role, Some(TableBoxRole::Row) | None).then_some(g)
                    })
                    .collect();
                let section_size = section_rows.len();
                for (i, gc) in section_rows.into_iter().enumerate() {
                    row_sources.push(TableRowSource {
                        node: TableRowNode::Element(gc),
                        section_index: i,
                        section_size,
                        section: Some(child_el),
                        section_child_index: section_child_idx,
                        section_sibling_count: section_count,
                        section_role,
                    });
                }
            }
            _ => {}
        }
    }
    improper_generated =
        GeneratedInlineContent::from_boxes(improper_generated.before(), generated_content.after());
    push_anonymous_table_row(
        &mut row_sources,
        &mut improper_children,
        &mut improper_generated,
        section_count,
        section_count,
    );

    if row_sources.is_empty() {
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
    let section_rank = |row: &TableRowSource<'_>| row.section_role.paint_order();
    if row_sources
        .windows(2)
        .any(|rows| section_rank(&rows[1]) < section_rank(&rows[0]))
    {
        row_sources.sort_by_key(section_rank);
    }

    let rows: Vec<&ElementNode> = row_sources.iter().map(|row| row.node.element()).collect();
    let row_section_indices: Vec<usize> = row_sources.iter().map(|row| row.section_index).collect();
    let row_section_sizes: Vec<usize> = row_sources.iter().map(|row| row.section_size).collect();
    let row_section_elements: Vec<Option<&ElementNode>> =
        row_sources.iter().map(|row| row.section).collect();
    let row_section_child_indices: Vec<usize> = row_sources
        .iter()
        .map(|row| row.section_child_index)
        .collect();
    let row_section_sibling_counts: Vec<usize> = row_sources
        .iter()
        .map(|row| row.section_sibling_count)
        .collect();

    let caption_styles_for_layout: Vec<ComputedStyle> = captions
        .iter()
        .map(|(caption_el, caption_child_idx)| {
            compute_caption_style(
                caption_el,
                *caption_child_idx,
                section_count,
                style,
                &table_ancestors,
                rules,
            )
        })
        .collect();
    let caption_min_width = captions
        .iter()
        .zip(&caption_styles_for_layout)
        .map(|((caption_el, caption_child_idx), caption_style)| {
            measure_caption_min_width(
                caption_el,
                *caption_child_idx,
                section_count,
                caption_style,
                &table_ancestors,
                rules,
                fonts,
                filter_defs,
                filter_dpi,
                &mut measurement_counter_state,
                &mut *resources,
                inner_width,
                descendant_layout,
            )
        })
        .fold(0.0f32, f32::max);

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
                let (child_style, child_role) = table_child_role(
                    child_el,
                    &column_parent_style,
                    rules,
                    &table_ancestors,
                    section_child_idx,
                    section_count,
                );
                match child_role {
                    Some(TableBoxRole::ColumnGroup) => {
                        let mut colgroup_ancestors = table_ancestors.clone();
                        colgroup_ancestors.push(AncestorInfo {
                            element: child_el,
                            child_index: section_child_idx,
                            sibling_count: section_count,
                            preceding_siblings: Vec::new(),
                            following_siblings: Vec::new(),
                            is_empty: false,
                        });
                        let colgroup_basis_style = {
                            let mut basis = child_style.clone();
                            basis.width = Some(inner_width);
                            basis
                        };
                        let col_child_count = child_el
                            .children
                            .iter()
                            .filter(|gc| matches!(gc, DomNode::Element(_)))
                            .count();
                        let cols: Vec<&ElementNode> = child_el
                            .children
                            .iter()
                            .enumerate()
                            .filter_map(|(col_child_idx, gc)| {
                                let DomNode::Element(g) = gc else {
                                    return None;
                                };
                                let (_, role) = table_child_role(
                                    g,
                                    &colgroup_basis_style,
                                    rules,
                                    &colgroup_ancestors,
                                    col_child_idx,
                                    col_child_count,
                                );
                                (role == Some(TableBoxRole::Column)).then_some(g)
                            })
                            .collect();
                        let colgroup_style = child_style;
                        let colgroup_bg = style_background_rgba(&colgroup_style);
                        if !cols.is_empty() {
                            let colgroup_start = col_idx;
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
                                assign_column_border(
                                    &mut column_info,
                                    col_idx,
                                    span,
                                    LayoutBorder::from_computed(&col_style.border, col_style.color),
                                );
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
                            assign_column_group_border(
                                &mut column_info,
                                colgroup_start,
                                col_idx.saturating_sub(colgroup_start),
                                LayoutBorder::from_computed(
                                    &colgroup_style.border,
                                    colgroup_style.color,
                                ),
                            );
                            continue;
                        }
                        let span = parse_col_span(child_el);
                        assign_column_group_border(
                            &mut column_info,
                            col_idx,
                            span,
                            LayoutBorder::from_computed(
                                &colgroup_style.border,
                                colgroup_style.color,
                            ),
                        );
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
                    Some(TableBoxRole::Column) => {
                        let span = parse_col_span(child_el);
                        let col_style = child_style;
                        let col_bg = style_background_rgba(&col_style);
                        let collapsed = col_style.visibility == Visibility::Collapse;
                        assign_column_border(
                            &mut column_info,
                            col_idx,
                            span,
                            LayoutBorder::from_computed(&col_style.border, col_style.color),
                        );
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
    // For `border-collapse: collapse`, CSS 2.2 §17.6.2 defines the table width
    // from the outer border edges while collapsed borders are centered on grid
    // lines.  Therefore the column tracks (grid-line to grid-line) span the
    // table width minus half of each winning outer border.  Painting starts the
    // first grid line half an outer border inward from the table origin.
    let (outer_left_border, outer_right_border) = collapse_outer_horizontal_borders(
        &rows,
        &row_section_indices,
        &row_section_sizes,
        &row_section_elements,
        &row_section_child_indices,
        &row_section_sibling_counts,
        style,
        rules,
        &table_ancestors,
    );
    let columns_width = if matches!(
        style.border_collapse,
        crate::style::computed::BorderCollapse::Separate
    ) && effective_border_spacing > 0.0
        && num_cols > 0
    {
        (inner_width - (num_cols as f32 + 1.0) * effective_border_spacing).max(0.0)
    } else if table_formatting.is_collapsed() {
        table_box_model.collapsed_grid_extent(
            inner_width,
            outer_left_border / 2.0 + outer_right_border / 2.0,
        )
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
            let mut row_style =
                anonymous_table_box_style(row, sizing_row_parent_style.as_ref().unwrap_or(style))
                    .unwrap_or_else(|| {
                        compute_style_with_context_with_font_metrics(
                            row.tag,
                            row.style_attr(),
                            sizing_row_parent_style.as_ref().unwrap_or(style),
                            rules,
                            row.tag_name(),
                            &row_classes,
                            row.id(),
                            &row.attributes,
                            &sizing_row_ctx,
                            FontMetrics::new(fonts),
                        )
                    });
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
            let row_child_count = row
                .children
                .iter()
                .filter(|child| matches!(child, DomNode::Element(_)))
                .count();
            let authored_siblings = element_sibling_list(&row.children);
            let mut row_child_idx = 0usize;
            for child in &row.children {
                if let DomNode::Element(cell_el) = child {
                    let cell_siblings = ElementSiblingContext::new(row_child_idx, row_child_count)
                        .with_neighbors(
                            authored_siblings.get(..row_child_idx).unwrap_or(&[]),
                            forward_siblings(&authored_siblings, row_child_idx),
                        );
                    let is_cell = row_child_is_table_cell(
                        cell_el,
                        &row_style,
                        rules,
                        &sizing_row_ctx.ancestors,
                        row_child_idx,
                        row_child_count,
                    );
                    let generated_cell_content =
                        row_sources[sizing_row_idx].generated_cell_content(row_child_idx);
                    row_child_idx += 1;
                    if is_cell {
                        while col_pos < num_cols && sizing_occupied[col_pos] > 0 {
                            sizing_occupied[col_pos] -= 1;
                            col_pos += 1;
                        }
                        if col_pos >= num_cols {
                            break;
                        }
                        let colspan = parse_cell_colspan(cell_el);
                        let span = colspan.min(num_cols - col_pos);
                        // Memoize this cell's measured widths. On a hit the
                        // cell's compute_style and the entire nested flatten
                        // below are skipped; a stored entry is only trusted
                        // when no counter/quote context is live (an empty
                        // state cannot influence the measurement, and an
                        // empty-to-empty measurement cannot have leaked any).
                        let cache_key =
                            (std::ptr::from_ref(cell_el) as usize, inner_width.to_bits());
                        let counters_empty_before = measurement_counter_state.stacks.is_empty()
                            && measurement_counter_state.quote_depth == 0;
                        let (total_preferred, total_min) = 'cell_sizing: {
                            if counters_empty_before
                                && let Some(widths) = table_cell_sizing_get(cache_key)
                            {
                                break 'cell_sizing widths;
                            }
                            let cell_classes = cell_el.class_list();
                            let mut cell_sizing_ancestors = sizing_row_ctx.ancestors.clone();
                            push_table_dom_ancestor(
                                &mut cell_sizing_ancestors,
                                row,
                                ElementSiblingContext::new(
                                    row_section_indices[sizing_row_idx],
                                    row_section_sizes[sizing_row_idx],
                                ),
                            );
                            let cell_sizing_ctx = cell_siblings.selector_context(
                                &cell_sizing_ancestors,
                                element_is_empty(cell_el),
                            );
                            let cell_style = anonymous_table_box_style(cell_el, &row_style)
                                .unwrap_or_else(|| {
                                    compute_style_with_context_with_font_metrics(
                                        cell_el.tag,
                                        cell_el.style_attr(),
                                        &row_style,
                                        rules,
                                        cell_el.tag_name(),
                                        &cell_classes,
                                        cell_el.id(),
                                        &cell_el.attributes,
                                        &cell_sizing_ctx,
                                        FontMetrics::new(fonts),
                                    )
                                });
                            let cell_counter_scope =
                                measurement_counter_state.enter_element(&cell_style);
                            let authored_generated = GeneratedContentStyles::resolve(
                                cell_el,
                                &cell_style,
                                rules,
                                &cell_sizing_ctx,
                                fonts,
                            );
                            let authored_generated = authored_generated.boxes(cell_el);
                            let mut runs = Vec::new();
                            let mut nested_rows = Vec::new();
                            let mut text_ancestors = cell_sizing_ctx.ancestors.clone();
                            push_table_dom_ancestor(&mut text_ancestors, cell_el, cell_siblings);
                            generated_cell_content.append_before_measurement(
                                &mut runs,
                                fonts,
                                &mut measurement_counter_state,
                                &mut *resources,
                            );
                            authored_generated.append_before_measurement(
                                &mut runs,
                                fonts,
                                &mut measurement_counter_state,
                                &mut *resources,
                            );
                            {
                                let mut measurement_env = LayoutEnv {
                                    rules,
                                    fonts,
                                    counter_state: &mut measurement_counter_state,
                                    resources: &mut *resources,
                                    filter_defs,
                                    filter_dpi,
                                };
                                layout_table_cell_flow(
                                    &cell_el.children,
                                    &mut runs,
                                    &mut nested_rows,
                                    TableCellFlowContext {
                                        style: &cell_style,
                                        ancestors: &text_ancestors,
                                        available_width: inner_width,
                                        descendant_layout,
                                    },
                                    &mut measurement_env,
                                );
                            }
                            generated_cell_content.append_after_measurement(
                                &mut runs,
                                fonts,
                                &mut measurement_counter_state,
                                &mut *resources,
                            );
                            authored_generated.append_after_measurement(
                                &mut runs,
                                fonts,
                                &mut measurement_counter_state,
                                &mut *resources,
                            );
                            runs.as_mut_slice().resolve_unclaimed_boundaries(
                                crate::layout::elements::TextSpacing::from_style(&cell_style),
                            );
                            measurement_counter_state.leave_element(cell_counter_scope);
                            // Measure the same prepared styled tokens and the same
                            // ordered advances that the final line wrapper consumes.
                            // This makes max-content an actual exact fit: no point- or
                            // pixel-based slack is added to table geometry.
                            let text_options =
                                table_cell_text_wrap_options(&cell_style, inner_width, fonts);
                            let intrinsic = measure_text_intrinsic_widths(
                                runs,
                                text_options,
                                table_cell_allows_soft_wrap(&cell_style),
                                fonts,
                            );
                            let content_width = intrinsic.max_content;
                            let content_min_width = intrinsic.min_content;
                            // Nested block descendants (e.g. a fixed-width <div>) and
                            // nested tables both contribute a minimum content width so
                            // a shrink-to-fit cell does not crush them narrower than
                            // their own declared/intrinsic width.
                            let nested_width = nested_rows
                                .iter()
                                .map(|element| nested_element_preferred_width(element.as_ref()))
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
                            let cell_border =
                                LayoutBorder::from_computed(&cell_style.border, cell_style.color);
                            let cell_insets = table_cell_content_insets(
                                &cell_style,
                                &cell_border,
                                style.border_collapse,
                            );
                            let cell_padding_x = cell_insets.horizontal();
                            let total_preferred = required_outer_width(
                                content_width.max(nested_width),
                                cell_padding_x,
                            )
                            .max(explicit_cell_width);
                            // Min-content includes padding and is floored by an explicit
                            // cell width (an explicit `width` makes the column at least
                            // that wide even for shrinking).
                            let total_min = required_outer_width(
                                content_min_width.max(nested_width),
                                cell_padding_x,
                            )
                            .max(explicit_cell_width);
                            if counters_empty_before
                                && measurement_counter_state.stacks.is_empty()
                                && measurement_counter_state.quote_depth == 0
                            {
                                table_cell_sizing_insert(cache_key, (total_preferred, total_min));
                            }
                            (total_preferred, total_min)
                        };
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
                if table_has_explicit_width && extra > 0.0 {
                    if total_preferred > 0.0 {
                        preferred_widths
                            .iter()
                            .map(|width| width + (width / total_preferred) * extra)
                            .collect()
                    } else {
                        vec![columns_width / preferred_widths.len() as f32; preferred_widths.len()]
                    }
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

    let mut collapsed_border_sources = CollapsedBorderSources::new(
        table_border,
        column_info.iter().map(|column| CollapsedBorderTrack {
            border: column.column_border,
            group_border: column.column_group_border,
        }),
        style.direction_rtl,
    );

    // Build layout rows, tracking cells occupied by rowspans from previous rows.
    // Every column covered by one originating cell carries the same semantic ID,
    // so adjacent but independent rowspans can never be merged by coincidental
    // geometry.
    let mut occupied = vec![RowspanSlot::default(); num_cols];
    let mut rowspan_id_counter = 0;
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
            occupied.fill(RowspanSlot::default());
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
        let section_starts_here = row_section_indices[row_idx] == 0;
        let section_break_before = section_starts_here
            && row_parent_style
                .as_ref()
                .is_some_and(|section_style| section_style.page_break_before);
        let section_break_side = row_parent_style
            .as_ref()
            .map(|section_style| PageBreakSide::from(section_style.break_before))
            .unwrap_or_default();
        let section_break_inside_avoid = row_parent_style
            .as_ref()
            .is_some_and(|section_style| section_style.break_inside_avoid);
        let section_avoid_group = section_break_inside_avoid.then(|| {
            TableFragmentGroup::new(table_section_key(
                row_section_elements[row_idx],
                row_section_child_indices[row_idx],
            ))
        });
        let row_selector_ctx = SelectorContext {
            ancestors: row_ancestors,
            child_index: section_idx,
            sibling_count: section_size,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        };
        let mut row_style =
            anonymous_table_box_style(row, row_parent_style.as_ref().unwrap_or(style))
                .unwrap_or_else(|| {
                    compute_style_with_context_with_font_metrics(
                        row.tag,
                        row.style_attr(),
                        row_parent_style.as_ref().unwrap_or(style),
                        rules,
                        row.tag_name(),
                        &row_classes,
                        row.id(),
                        &row.attributes,
                        &row_selector_ctx,
                        FontMetrics::new(fonts),
                    )
                });
        // `display: none` rows are removed from the table entirely.
        if row_style.display == Display::None {
            continue;
        }
        let row_border = LayoutBorder::from_computed(&row_style.border, row_style.color);
        let row_group_border = row_parent_style.as_ref().map(|section_style| {
            LayoutBorder::from_computed(&section_style.border, section_style.color)
        });
        if row_style.page_break_before || section_break_before {
            let side = if row_style.page_break_before {
                PageBreakSide::from(row_style.break_before)
            } else {
                section_break_side
            };
            output.push(
                PageBreak {
                    side,
                    page_name: None,
                }
                .boxed(),
            );
        }
        let row_break_inside_avoid = row_style.break_inside_avoid;
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
                let row_child_count = row
                    .children
                    .iter()
                    .filter(|child| matches!(child, DomNode::Element(_)))
                    .count();
                let authored_siblings = element_sibling_list(&row.children);
                let mut row_child_idx = 0usize;
                for child in &row.children {
                    let DomNode::Element(cell_el) = child else {
                        continue;
                    };
                    let cell_siblings = ElementSiblingContext::new(row_child_idx, row_child_count)
                        .with_neighbors(
                            authored_siblings.get(..row_child_idx).unwrap_or(&[]),
                            forward_siblings(&authored_siblings, row_child_idx),
                        );
                    let is_cell = row_child_is_table_cell(
                        cell_el,
                        &row_style,
                        rules,
                        &row_selector_ctx.ancestors,
                        row_child_idx,
                        row_child_count,
                    );
                    row_child_idx += 1;
                    if !is_cell {
                        continue;
                    }
                    if col_pos >= num_cols {
                        break;
                    }
                    let colspan = parse_cell_colspan(cell_el);
                    let span = colspan.min(num_cols - col_pos);

                    let cell_classes = cell_el.class_list();
                    let mut cell_ancestors = row_selector_ctx.ancestors.clone();
                    push_table_dom_ancestor(
                        &mut cell_ancestors,
                        row,
                        ElementSiblingContext::new(section_idx, section_size),
                    );
                    let cell_selector_ctx =
                        cell_siblings.selector_context(&cell_ancestors, element_is_empty(cell_el));
                    let cell_style =
                        anonymous_table_box_style(cell_el, &row_style).unwrap_or_else(|| {
                            compute_style_with_context_with_font_metrics(
                                cell_el.tag,
                                cell_el.style_attr(),
                                &row_style,
                                rules,
                                cell_el.tag_name(),
                                &cell_classes,
                                cell_el.id(),
                                &cell_el.attributes,
                                &cell_selector_ctx,
                                FontMetrics::new(fonts),
                            )
                        });
                    let cell_border =
                        LayoutBorder::from_computed(&cell_style.border, cell_style.color);
                    cells.push(TableCell {
                        layout: CellBox {
                            box_model: CellBoxModel {
                                border: cell_border,
                                minimum_block_size: cell_border.vertical_width(),
                                ..Default::default()
                            },
                            alignment: CellAlignment {
                                inline: cell_style.text_align,
                                block: VerticalAlign::Top,
                            },
                            fragmentation: CellFragmentation::from_style(&cell_style),
                            ..Default::default()
                        },
                        span: TableCellSpan {
                            columns: span,
                            ..Default::default()
                        },
                        table: TableCellState::default(),
                    });
                    col_pos += span;
                }

                if !cells.is_empty() {
                    collapsed_border_sources.push_row(CollapsedBorderTrack::row(
                        row_border,
                        row_group_border,
                        section_idx,
                        section_size,
                    ));
                    let first_emitted_row = output.len() == table_output_start;
                    let mut row_cells = cells;
                    let mut row_col_widths = col_widths.clone();
                    if style.direction_rtl {
                        row_cells.reverse();
                        row_col_widths.reverse();
                    }
                    output.push(table_row_node(
                        table_grid.clone(),
                        TableCells {
                            cells: row_cells,
                            column_widths: row_col_widths,
                        },
                        BlockFlowSpacing::from_internal_start(if first_emitted_row {
                            table_grid_box_insets.top
                        } else {
                            0.0
                        }),
                        table_formatting,
                        row_sources[row_idx]
                            .section_role
                            .fragmentation(row_break_inside_avoid, section_avoid_group),
                        table_inline_geometry,
                    ));
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
                output.push(table_row_node(
                    table_grid.clone(),
                    TableCells {
                        cells: Vec::new(),
                        column_widths: row_col_widths,
                    },
                    BlockFlowSpacing::from_internal_start(
                        spacer_height
                            + if first_emitted_row {
                                table_grid_box_insets.top
                            } else {
                                0.0
                            },
                    ),
                    table_formatting,
                    row_sources[row_idx]
                        .section_role
                        .fragmentation(row_break_inside_avoid, section_avoid_group),
                    table_inline_geometry,
                ));
            }
            continue;
        }
        row_style.width = Some(inner_width);
        let mut cells = Vec::new();

        // Current logical column position in the grid
        let mut col_pos: usize = 0;
        let authored_siblings = element_sibling_list(&row.children);
        let row_cell_elements =
            table_row_cell_elements(row, &row_style, rules, &row_selector_ctx.ancestors);
        let mut child_iter = row_cell_elements.into_iter().enumerate();

        // Process cells, skipping occupied positions and inserting phantom cells
        let mut next_cell = child_iter.next();
        while col_pos < num_cols {
            if occupied[col_pos].is_occupied() {
                // This position is occupied by a rowspan from a previous row.
                // Insert a phantom cell (rowspan = 0) as a placeholder.
                let span_id = occupied[col_pos].span_id;
                let span_cols = occupied[col_pos..]
                    .iter()
                    .take_while(|slot| slot.span_id == span_id)
                    .count();
                cells.push(TableCell {
                    layout: CellBox {
                        box_model: CellBoxModel {
                            minimum_block_size: occupied[col_pos].min_height,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    span: TableCellSpan {
                        columns: span_cols,
                        rows: 0,
                    },
                    ..Default::default()
                });
                for slot in &mut occupied[col_pos..col_pos + span_cols] {
                    slot.consume_row();
                }
                col_pos += span_cols;
                continue;
            }

            // Place the next real cell at this position
            let Some((cell_source_index, cell_source)) = next_cell else {
                break;
            };
            next_cell = child_iter.next();
            let cell_el = cell_source.element;
            let cell_siblings = cell_source.siblings(&authored_siblings);
            let generated_cell_content =
                row_sources[row_idx].generated_cell_content(cell_source_index);

            let colspan = parse_cell_colspan(cell_el);
            let rowspan = parse_cell_rowspan(
                cell_el,
                row_section_sizes[row_idx].saturating_sub(row_section_indices[row_idx]),
            );

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            push_table_dom_ancestor(
                &mut cell_ancestors,
                row,
                ElementSiblingContext::new(section_idx, section_size),
            );
            let cell_selector_ctx =
                cell_siblings.selector_context(&cell_ancestors, element_is_empty(cell_el));
            let cell_style = anonymous_table_box_style(cell_el, &row_style).unwrap_or_else(|| {
                compute_style_with_context_with_font_metrics(
                    cell_el.tag,
                    cell_el.style_attr(),
                    &row_style,
                    rules,
                    cell_el.tag_name(),
                    &cell_classes,
                    cell_el.id(),
                    &cell_el.attributes,
                    &cell_selector_ctx,
                    FontMetrics::new(fonts),
                )
            });
            let cell_counter_scope = counter_state.enter_element(&cell_style);
            let authored_generated = GeneratedContentStyles::resolve(
                cell_el,
                &cell_style,
                rules,
                &cell_selector_ctx,
                fonts,
            );
            let authored_generated = authored_generated.boxes(cell_el);
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
            let cell_border = LayoutBorder::from_computed(&cell_style.border, cell_style.color);
            let border_inset_factor = table_cell_border_inset_factor(style.border_collapse);
            let cell_border_insets = cell_border.widths() * border_inset_factor;
            let cell_insets =
                table_cell_content_insets(&cell_style, &cell_border, style.border_collapse);
            let effective_width: f32 = col_widths.iter().skip(col_pos).take(colspan).copied().sum();
            let cell_inner = (effective_width - cell_insets.horizontal()).max(0.0);
            let mut cell_content_style = cell_style.clone();
            cell_content_style.width = Some(cell_inner);
            // A cell's `height` is its border-box; a child's `height: %` resolves
            // against the cell's *content* box (height minus the cell's own
            // padding and border). `cell_content_style` is the parent handed to
            // child style resolution, so expose the content-box height here —
            // otherwise `height: 100%` resolved against the full border-box and a
            // padded cell's inner block rendered too tall (overflowing the cell).
            cell_content_style.height = cell_style.height.map(|h| {
                super::helpers::resolve_content_box_height(
                    h,
                    cell_style.padding,
                    cell_border.widths() * border_inset_factor,
                    cell_style.box_sizing,
                )
            });

            let mut runs = Vec::new();
            let mut nested_rows = Vec::new();
            let mut text_ancestors = cell_selector_ctx.ancestors.clone();
            push_table_dom_ancestor(&mut text_ancestors, cell_el, cell_siblings);
            let (block_margin_top, block_margin_bottom) = table_cell_edge_block_margins(
                &cell_el.children,
                &cell_content_style,
                rules,
                &text_ancestors,
                FontMetrics::new(fonts),
            );
            if let Some(before) = generated_cell_content.before()
                && let Some(boundary) = generated_table_cell_boundary(
                    before,
                    &cell_content_style,
                    cell_inner,
                    fonts,
                    filter_defs,
                    counter_state,
                    &mut *resources,
                )
            {
                nested_rows.push(boundary);
            }
            append_generated_cell_layout(
                authored_generated.before(),
                GeneratedCellLayout {
                    runs: &mut runs,
                    blocks: &mut nested_rows,
                    parent_style: &cell_content_style,
                    available_width: cell_inner,
                    fonts,
                    filter_defs,
                    counter_state,
                    resources: &mut *resources,
                },
            );
            {
                let mut flow_env = LayoutEnv {
                    rules,
                    fonts,
                    counter_state: &mut *counter_state,
                    resources: &mut *resources,
                    filter_defs,
                    filter_dpi,
                };
                layout_table_cell_flow(
                    &cell_el.children,
                    &mut runs,
                    &mut nested_rows,
                    TableCellFlowContext {
                        style: &cell_content_style,
                        ancestors: &text_ancestors,
                        available_width: cell_inner,
                        descendant_layout,
                    },
                    &mut flow_env,
                );
            }
            if let Some(after) = generated_cell_content.after()
                && let Some(boundary) = generated_table_cell_boundary(
                    after,
                    &cell_content_style,
                    cell_inner,
                    fonts,
                    filter_defs,
                    counter_state,
                    &mut *resources,
                )
            {
                nested_rows.push(boundary);
            }
            append_generated_cell_layout(
                authored_generated.after(),
                GeneratedCellLayout {
                    runs: &mut runs,
                    blocks: &mut nested_rows,
                    parent_style: &cell_content_style,
                    available_width: cell_inner,
                    fonts,
                    filter_defs,
                    counter_state,
                    resources: &mut *resources,
                },
            );
            runs.as_mut_slice().resolve_unclaimed_boundaries(
                crate::layout::elements::TextSpacing::from_style(&cell_content_style),
            );
            counter_state.leave_element(cell_counter_scope);
            let lines = wrap_text_runs(
                runs,
                table_cell_text_wrap_options(&cell_style, cell_inner, fonts),
                fonts,
            );

            let column_bg = column_info
                .iter()
                .skip(col_pos)
                .take(colspan)
                .find_map(|info| info.background_color);
            let bg = style_background_rgba(&cell_style)
                .or_else(|| style_background_rgba(&row_style))
                .or_else(|| row_parent_style.as_ref().and_then(style_background_rgba))
                .or(column_bg);

            // CSS Tables defines the row minimum from the cell's definite
            // computed height. Collapsed borders extend around the row grid;
            // conflict resolution must not shrink this track contribution.
            let height_constraint = cell_style
                .height
                .map(|specified| TableCellHeightConstraint { specified, rowspan });
            let min_content_height = height_constraint
                .map(TableCellHeightConstraint::minimum_row_height)
                .unwrap_or(0.0);
            // Under `empty-cells: hide`, a cell with no in-flow content has its
            // border and background suppressed so the table background shows
            // through. Emptiness is decided from the DOM, not the collapsed
            // text, because `&nbsp;` is content yet whitespace-collapses away.
            let hide_if_empty = style.border_collapse == BorderCollapse::Separate
                && cell_style.empty_cells == crate::style::computed::EmptyCells::Hide
                && cell_has_no_content(cell_el)
                && generated_cell_content.is_empty();
            let row_span_share = min_content_height;
            let mut box_paint = BoxPaint::from_style(
                &cell_style,
                LayoutSize::fixed(
                    col_widths.get(col_pos).copied().unwrap_or_default(),
                    Some(row_span_share),
                ),
            );
            table_formatting.constrain_internal_decoration(&mut box_paint);
            box_paint.background.color = bg;
            let mut table_cell = TableCell {
                layout: CellBox {
                    content: CellContent {
                        lines,
                        children: nested_rows,
                    },
                    box_model: CellBoxModel {
                        content_insets: EdgeSizes::new(
                            cell_insets.top + block_margin_top,
                            cell_insets.right,
                            cell_insets.bottom + block_margin_bottom,
                            cell_insets.left,
                        ),
                        border_insets: cell_border_insets,
                        border: cell_border,
                        minimum_block_size: row_span_share,
                    },
                    paint: CellPaint {
                        box_paint,
                        ..Default::default()
                    },
                    positioning: Positioning::from_style(&cell_style),
                    alignment: CellAlignment {
                        inline: cell_style.text_align,
                        block: table_cell_vertical_align(cell_style.vertical_align),
                    },
                    fragmentation: CellFragmentation::from_style(&cell_style),
                },
                span: TableCellSpan {
                    columns: colspan,
                    rows: rowspan,
                },
                table: TableCellState {
                    hide_if_empty,
                    clips: cell_style.overflow.clips(),
                },
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

            // Mark subsequent rows as occupied if rowspan > 1
            if rowspan > 1 {
                let slot = RowspanSlot::occupied(
                    RowspanId::allocate(&mut rowspan_id_counter),
                    rowspan - 1,
                    row_span_share,
                );
                occupied[col_pos..col_pos.saturating_add(colspan).min(num_cols)].fill(slot);
            }

            col_pos += colspan;
        }

        if !cells.is_empty() {
            collapsed_border_sources.push_row(CollapsedBorderTrack::row(
                row_border,
                row_group_border,
                section_idx,
                section_size,
            ));
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
            output.push(table_row_node(
                table_grid.clone(),
                TableCells {
                    cells: row_cells,
                    column_widths: row_col_widths,
                },
                // The table-level background box (inserted below) carries the
                // table's own `margin-top`. The first row is therefore inset
                // only by the top *vertical* `border-spacing` (zero when
                // collapsed); subsequent rows are separated by the same.
                BlockFlowSpacing::from_internal_start(
                    if style.border_collapse == BorderCollapse::Separate {
                        effective_border_spacing_vertical
                    } else {
                        0.0
                    } + if first_emitted_row {
                        table_grid_box_insets.top
                    } else {
                        0.0
                    },
                ),
                table_formatting,
                row_sources[row_idx]
                    .section_role
                    .fragmentation(row_break_inside_avoid, section_avoid_group),
                // The table's own horizontal start margin shifts every cell (and
                // the table box) right from the containing block's content edge,
                // mirroring how `margin_top` shifts it down.
                table_inline_geometry,
            ));
        }
    }

    // If no rows were actually emitted (e.g. every row was `display:none`),
    // there is no table box to paint.
    if output.len() == table_output_start {
        return;
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

    let resolved_table_grid_insets = if style.border_collapse == BorderCollapse::Collapse {
        resolve_collapsed_border_grid(
            &mut output[table_output_start..],
            &collapsed_border_sources,
            style.direction_rtl,
        )
    } else {
        table_grid_box_insets
    };
    if let Some(table_height) = resolve_table_inner_height(style, table_box_model) {
        let stretch_target = table_box_model.collapsed_grid_extent(
            table_height,
            collapsed_outer_vertical_border_extent(&output[table_output_start..]),
        );
        stretch_table_rows_to_min_height(
            &mut output[table_output_start..],
            stretch_target,
            edge_spacing_v,
        );
    }
    // Height of the table padding box: the rows plus, for `separate` collapse,
    // the vertical `border-spacing` above the first row, below the last row, and
    // between each adjacent pair, then the table padding. `TextBlock` height is
    // a padding-box height; its renderer adds the border exactly once.
    let mut emitted_rows = 0usize;
    let mut rows_height = 0.0f32;
    for elem in &output[table_output_start..] {
        if let Some((cell_count, row_height, margin_start)) =
            query_table_row(elem.as_ref(), |row| {
                (
                    row.content.cells.len(),
                    row_height_from_cells(&row.content.cells),
                    row.flow.internal.start,
                )
            })
        {
            if cell_count == 0 {
                rows_height += margin_start;
                continue;
            }
            emitted_rows += 1;
            rows_height += row_height;
        }
    }
    let collapsed_outer_height = if style.border_collapse == BorderCollapse::Collapse {
        collapsed_outer_vertical_border_extent(&output[table_output_start..])
    } else {
        0.0
    };
    let collapsed = style.border_collapse == BorderCollapse::Collapse;
    let box_height = rows_height
        + if collapsed {
            collapsed_outer_height
        } else {
            0.0
        }
        + edge_spacing_v * (emitted_rows.saturating_add(1) as f32)
        + table_box_model.padding.vertical();

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
        resolved_table_grid_insets.left + resolved_table_grid_insets.right
    } else {
        0.0
    };
    let table_level_border = if collapsed {
        LayoutBorder::default()
    } else {
        table_border
    };
    let box_width = columns_sum
        + edge_spacing_h * (col_widths.len().saturating_add(1) as f32)
        + collapse_outer_w
        + table_box_model.padding.horizontal()
        + table_level_border.horizontal_width();
    let table_offset = InlineOffset::resolve_block_start(style, available_width, box_width);
    let resolved_grid_left = if collapsed {
        resolved_table_grid_insets.left
    } else {
        table_grid_box_insets.left
    };
    let table_inline_geometry =
        TableInlineGeometry::new(table_offset, table_offset + resolved_grid_left)
            .with_box_extent(box_width);

    for row in &mut output[table_output_start..] {
        update_table_row(row.as_mut(), |row| {
            row.inline = table_inline_geometry.relative_to(table_offset);
        });
    }

    // The last row carries the bottom vertical `border-spacing` gap plus the
    // table's own `margin-bottom`, so the in-flow height below the rows matches
    // the box. A `caption-side: bottom` caption (appended after the rows) takes
    // over the table's `margin-bottom` instead, so the row keeps only the gap.
    let caption_on_top = caption_styles_for_layout
        .first()
        .is_none_or(|caption_style| {
            matches!(
                caption_style.caption_side,
                crate::style::computed::CaptionSide::Top
            )
        });
    let bottom_caption = !captions.is_empty() && !caption_on_top;
    if let Some(last) = output.last_mut() {
        // A collapsed table extends half of its winning top and bottom borders
        // beyond the row grid.  Row heights span grid line to grid line, so carry
        // that one table-level outer extent after the last row.  Charging it to
        // every row would incorrectly grow multi-row tables; omitting it places
        // the following sibling one border-width too high.
        update_table_row(last.as_mut(), |row| {
            row.flow.internal.end = edge_spacing_v + table_grid_box_insets.bottom;
            row.flow.margins.end = if bottom_caption {
                0.0
            } else {
                style.margin.bottom
            };
            row.flow.extra_end = if collapsed {
                resolved_table_grid_insets.bottom
            } else {
                0.0
            };
        });
    }

    let has_top_caption = !captions.is_empty() && caption_on_top;

    // The table's own `margin-top` is carried by whichever box comes first: a
    // top caption, otherwise the background box (if any), otherwise the first
    // row. Track whether something earlier already claimed it.
    let mut margin_top_claimed = false;

    // Paint the table element's own background/border behind the rows. It is a
    // zero-flow box: its `margin-top` carries the table's own top margin (unless
    // a caption above already does) while a matching negative `margin-bottom`
    // cancels its height so the rows that follow render on top of it.
    if has_background_paint(style) || style.has_border_decoration() {
        let bg_margin_top = if has_top_caption {
            0.0
        } else {
            margin_top_claimed = true;
            style.margin.top
        };
        let mut bg_block = TextBlock::from_style(
            Vec::new(),
            style,
            crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize::fixed(box_width, Some(box_height)),
                margins: BlockMargins::new(
                    bg_margin_top,
                    -(box_height + table_level_border.vertical_width()),
                ),
                padding: EdgeSizes::ZERO,
                border: table_level_border,
            },
        );
        bg_block.flow = Default::default();
        bg_block.positioning = Default::default();
        bg_block.paint.group = Default::default();
        bg_block.paint.border_radii = table_formatting.table_corner_radii(
            style.resolve_corner_radii(box_width, box_height + table_level_border.vertical_width()),
        );
        bg_block.text = Default::default();
        let bg_block = TableBoxDecoration::new(bg_block).boxed();
        output.insert(table_output_start, bg_block);
    }

    // `<caption>` (caption-side:top) renders as a full-table-width block above
    // the rows: it carries the table's `margin-top` and pushes the rest of the
    // table down by its own height.
    for (caption_idx, ((caption_el, caption_child_idx), caption_style)) in captions
        .iter()
        .zip(caption_styles_for_layout.iter())
        .enumerate()
    {
        let caption_style = caption_style.clone();
        let caption_inner = (box_width - caption_style.padding.horizontal()).max(0.0);
        let mut caption_ancestors = table_ancestors.clone();
        caption_ancestors.push(AncestorInfo {
            element: caption_el,
            child_index: *caption_child_idx,
            sibling_count: section_count,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        });
        let mut caption_runs = Vec::new();
        let mut caption_nested = Vec::new();
        {
            let mut flow_env = LayoutEnv {
                rules,
                fonts,
                counter_state: &mut *counter_state,
                resources: &mut *resources,
                filter_defs,
                filter_dpi,
            };
            layout_table_cell_flow(
                &caption_el.children,
                &mut caption_runs,
                &mut caption_nested,
                TableCellFlowContext {
                    style: &caption_style,
                    ancestors: &caption_ancestors,
                    available_width: caption_inner,
                    descendant_layout,
                },
                &mut flow_env,
            );
        }
        let caption_lines = wrap_text_runs(
            caption_runs,
            TextWrapOptions::new(
                caption_inner,
                used_font_size(&caption_style, fonts),
                text_run_line_height_factor(&caption_style, fonts),
                caption_style.overflow_wrap,
            )
            .with_white_space(caption_style.white_space)
            .with_parent_strut(parent_line_strut(&caption_style, fonts))
            .with_rtl(caption_style.direction_rtl)
            .with_bidi_override(caption_style.bidi_override),
            fonts,
        );
        let caption_border =
            LayoutBorder::from_computed(&caption_style.border, caption_style.color);
        // A top caption sits above the table and carries the table's
        // `margin-top`; a bottom caption is appended after the rows and carries
        // the table's `margin-bottom` instead (the rows already absorbed the
        // bottom border-spacing gap above).
        let (caption_margin_top, caption_margin_bottom) = if caption_on_top {
            (
                if caption_idx == 0 {
                    style.margin.top
                } else {
                    0.0
                },
                0.0,
            )
        } else {
            (
                0.0,
                if caption_idx + 1 == captions.len() {
                    style.margin.bottom
                } else {
                    0.0
                },
            )
        };
        let mut caption_block = TextBlock::from_style(
            caption_lines,
            &caption_style,
            crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize::fixed(box_width, caption_style.height),
                margins: BlockMargins::new(caption_margin_top, caption_margin_bottom),
                padding: caption_style.padding,
                border: caption_border,
            },
        );
        caption_block.flow = Default::default();
        caption_block.positioning =
            crate::layout::elements::Positioning::from_style(&caption_style);
        caption_block.paint.border_radii = caption_style
            .resolve_corner_radii(box_width, caption_style.height.unwrap_or(box_width));
        caption_block.text.indent = 0.0;
        let caption_block = caption_block.boxed();
        if caption_on_top {
            output.insert(table_output_start + caption_idx, caption_block);
            margin_top_claimed = true;
        } else {
            output.push(caption_block);
        }
    }

    // If neither a caption nor a background box claimed the table's `margin-top`,
    // fold it into the first emitted row so the table keeps its top margin.
    if !margin_top_claimed && style.margin.top != 0.0 {
        if let Some(first_row) = output.get_mut(table_output_start) {
            update_table_row(first_row.as_mut(), |row| {
                row.flow.margins.start += style.margin.top;
            });
        }
    }

    // A table is one principal CSS box, not a coincidental run of paint leaves.
    // Move its external flow spacing and positioning onto that principal while
    // keeping rows, captions, and decoration as fragmentable children.
    let mut parts = output.drain(table_output_start..).collect::<Vec<_>>();
    if let Some(first) = parts.first_mut().and_then(|part| part.margin_holder_mut()) {
        first.margins_mut().start = 0.0;
    }
    if let Some(last) = parts.last_mut().and_then(|part| part.margin_holder_mut()) {
        last.margins_mut().end = 0.0;
    }

    // CSS Tables treats `height` on a table-root as a minimum for the table
    // grid. `box_height` is measured after row expansion and therefore is the
    // authoritative used padding-box extent even when content exceeds that
    // authored minimum. Preserve the distinction in `BlockSize`: marking the
    // authored value definite here made flex alignment and containing blocks
    // use a shorter box than the rows that were actually painted.
    let table_border_box_height = box_height + table_level_border.vertical_width();
    let establishes_containing_block = crate::layout::helpers::establishes_containing_block(style);
    if establishes_containing_block {
        crate::layout::helpers::resolve_absolute_descendants_containing_block(
            &mut parts,
            ContainingBlock {
                x: 0.0,
                width: box_width,
                height: table_border_box_height,
                depth: context.positioned_depth,
            },
        );
    }

    let mut principal = Container::from_style(
        parts,
        style,
        BoxModel {
            size: LayoutSize::fixed_inline(
                box_width,
                crate::layout::elements::BlockSize::minimum(table_border_box_height),
            ),
            margins: BlockMargins::new(style.margin.top, style.margin.bottom),
            ..Default::default()
        },
    );
    principal.paint = BoxPaint {
        group: PaintGroup::from_style(style),
        visible: style.visibility == Visibility::Visible,
        ..Default::default()
    };
    principal.positioning.insets.left += table_offset.value();
    principal.positioning.containing_block_depth = if establishes_containing_block {
        context.positioned_depth
    } else {
        0
    };
    output.push(Table::new(principal).boxed());
}

fn table_cell_edge_block_margins(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    font_metrics: FontMetrics<'_>,
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
        let child_style = compute_style_with_context_with_font_metrics(
            element.tag,
            element.style_attr(),
            parent_style,
            rules,
            element.tag_name(),
            &element.class_list(),
            element.id(),
            &element.attributes,
            &selector_ctx,
            font_metrics,
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

fn table_cell_child_should_flatten(el: &ElementNode, style: &ComputedStyle) -> bool {
    el.tag == HtmlTag::Table
        || el.tag == HtmlTag::Img
        || el.tag == HtmlTag::Svg
        || (recurses_as_layout_child(el.tag) && !collects_as_inline_text(el.tag))
        || style.display != Display::Inline
        || style.position.is_absolute()
}

#[derive(Clone, Copy)]
struct TableCellFlowContext<'style, 'ancestors, 'dom> {
    style: &'style ComputedStyle,
    ancestors: &'ancestors [AncestorInfo<'dom>],
    available_width: f32,
    descendant_layout: TableDescendantLayout,
}

struct TableCellChildLayout<'output, 'style, 'ancestors, 'dom> {
    context: TableCellFlowContext<'style, 'ancestors, 'dom>,
    output: &'output mut Vec<LayoutNode>,
}

impl IndependentFlowLayout for TableCellChildLayout<'_, '_, '_, '_> {
    fn lays_out_independently(&self, element: &ElementNode, child: &InlineFormattingChild) -> bool {
        table_cell_child_should_flatten(element, &child.style) && element.tag != HtmlTag::Br
    }

    fn layout_independently(
        &mut self,
        element: &ElementNode,
        child: &InlineFormattingChild,
        env: &mut LayoutEnv<'_>,
    ) {
        let cell_context = self
            .context
            .descendant_layout
            .child_context(self.context.available_width, self.context.style);
        flatten_element(
            element,
            LayoutTreeContext::new(self.context.style, &cell_context, self.context.ancestors)
                .with_positioned_ancestor_depth(self.context.descendant_layout.positioned_depth)
                .for_element(child.source().as_context()),
            self.output,
            env,
        );
    }
}

fn layout_table_cell_flow(
    nodes: &[DomNode],
    runs: &mut Vec<TextRun>,
    nested_rows: &mut Vec<LayoutNode>,
    context: TableCellFlowContext<'_, '_, '_>,
    env: &mut LayoutEnv,
) {
    let mut child_layout = TableCellChildLayout {
        context,
        output: nested_rows,
    };
    layout_mixed_flow_children(
        nodes,
        context.style,
        runs,
        context.ancestors,
        env,
        &mut child_layout,
    );
}

#[cfg(test)]
mod subpoint_width_tests {
    use super::*;
    use crate::layout::cells::TableRowCells;
    use crate::layout::elements::{LayoutElementTestExt, visit_layout_tree};
    use crate::layout::engine::{
        SyntheticFontWeight, layout, layout_with_rules, layout_with_rules_and_fonts,
    };
    use crate::parser::css::parse_stylesheet;
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::types::{Margin, PageSize};

    fn table_rows(page: &crate::layout::engine::Page) -> Vec<TableRow> {
        #[derive(Default)]
        struct Rows(Vec<TableRow>);

        impl LayoutVisitor for Rows {
            fn visit_table_row(&mut self, row: &TableRow) {
                self.0.push(row.clone());
            }
        }

        let mut rows = Rows::default();
        for (_, element) in &page.elements {
            visit_layout_tree(element.as_ref(), &mut rows);
        }
        rows.0
    }

    #[test]
    fn positioned_table_root_owns_blockified_absolute_descendants() {
        #[derive(Default)]
        struct AbsoluteText(Option<(String, Positioning)>);

        impl LayoutVisitor for AbsoluteText {
            fn visit_text_block(&mut self, block: &TextBlock) {
                if block.positioning.scheme == crate::style::computed::Position::Absolute {
                    let text = block
                        .lines
                        .iter()
                        .flat_map(|line| &line.runs)
                        .map(|run| run.text.as_str())
                        .collect::<String>();
                    if text == "Bb" {
                        self.0 = Some((text, block.positioning.clone()));
                    }
                }
            }
        }

        let document = parse_html_with_styles(
            r#"<style>
                * { box-sizing:border-box; margin:0 }
                .table { display:table; position:relative; width:120px; height:60px;
                         padding:7px; border:2px solid black; border-spacing:3px }
                .cell { display:table-cell }
                .cell:last-child { position:absolute; right:4px; bottom:5px;
                                   width:10px; height:8px }
            </style>
            <div class="table"><div><span class="cell">Ag</span><span class="cell">Bb</span></div></div>"#,
        )
        .expect("valid positioned table fixture");
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(300.0, 180.0),
            Margin::uniform(0.0),
            &rules,
        );
        let mut absolute = AbsoluteText::default();
        for (_, element) in &pages[0].elements {
            visit_layout_tree(element.as_ref(), &mut absolute);
        }
        let (_, positioning) = absolute.0.expect("blockified absolute table child");
        let containing_block = positioning
            .containing_block
            .expect("table wrapper containing block");

        assert_eq!(containing_block.depth, 1);
        assert!((containing_block.width - 90.0).abs() < 0.01);
        assert!((containing_block.height - 45.0).abs() < 0.01);
        assert!(positioning.insets.left > 70.0);
        assert!(positioning.insets.top > 25.0);
    }

    fn first_cell(width: f32) -> (f32, usize) {
        let nodes = parse_html(&format!(
            r#"<table style="table-layout:fixed;width:{width}pt;border-spacing:0;font-size:0.5pt;line-height:1"><tr><td style="padding:0;overflow-wrap:anywhere">i i i i</td></tr></table>"#
        ))
        .expect("valid table fixture");
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_table(|row| {
                    (
                        row.content.column_widths[0],
                        row.content.cells[0].layout.content.lines.len(),
                    )
                })
            })
            .expect("one table row")
    }

    #[test]
    fn css_table_normalizes_only_the_first_header_and_footer_groups() {
        let nodes = parse_html(
            r#"<div style="display:table">
                <div style="display:table-row-group"><div style="display:table-row"><div style="display:table-cell">body</div></div></div>
                <div style="display:table-footer-group"><div style="display:table-row"><div style="display:table-cell">first footer</div></div></div>
                <div style="display:table-footer-group"><div style="display:table-row"><div style="display:table-cell">second footer</div></div></div>
                <div style="display:table-header-group"><div style="display:table-row"><div style="display:table-cell">first header</div></div></div>
                <div style="display:table-header-group"><div style="display:table-row"><div style="display:table-cell">second header</div></div></div>
            </div>"#,
        )
        .expect("valid CSS table fixture");
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let roles = table_rows(&pages[0])
            .into_iter()
            .map(|row| {
                (
                    row.fragmentation.repeats_as_header,
                    row.fragmentation.repeats_as_footer,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            roles,
            vec![
                (true, false),
                (false, false),
                (false, false),
                (false, false),
                (false, true),
            ]
        );
    }

    #[test]
    fn adjacent_avoided_row_groups_keep_distinct_fragmentation_identity() {
        let nodes = parse_html(
            r#"<div style="display:table">
                <div style="display:table-row-group;break-inside:avoid"><div style="display:table-row"><div style="display:table-cell">first</div></div></div>
                <div style="display:table-row-group;break-inside:avoid"><div style="display:table-row"><div style="display:table-cell">second</div></div></div>
            </div>"#,
        )
        .expect("valid CSS table fixture");
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let groups = table_rows(&pages[0])
            .into_iter()
            .map(|row| row.fragmentation.avoid_group)
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(Option::is_some));
        assert_ne!(groups[0], groups[1]);
    }

    #[test]
    fn table_wrap_width_preserves_half_point_and_thousandth_point_cells() {
        let fonts = HashMap::new();
        let style = ComputedStyle::default();
        assert_eq!(
            table_cell_text_wrap_options(&style, 0.5, &fonts).max_width,
            0.5
        );
        assert_eq!(
            table_cell_text_wrap_options(&style, 0.001, &fonts).max_width,
            0.001
        );

        let (half_width, half_lines) = first_cell(0.5);
        let (thousandth_width, thousandth_lines) = first_cell(0.001);
        assert_eq!(half_width, 0.5);
        assert_eq!(thousandth_width, 0.001);
        assert!(
            half_lines > 1,
            "0.5pt cell unexpectedly used a wider wrap width"
        );
        assert!(
            thousandth_lines > half_lines,
            "0.001pt cell must remain distinct from the 0.5pt and 1pt layout lanes: {thousandth_lines} vs {half_lines} lines"
        );
    }

    #[test]
    fn collapsed_table_carries_its_outer_border_extent_once_after_the_last_row() {
        let nodes = parse_html(
            r#"<table style="border-collapse:collapse;margin-bottom:4pt">
                <tr><td style="border:1pt solid">a</td></tr>
                <tr><td style="border:1pt solid">b</td></tr>
                <tr><td style="border:1pt solid">c</td></tr>
            </table>"#,
        )
        .expect("valid collapsed table fixture");
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        let row_flow = rows
            .iter()
            .map(|row| {
                (
                    row.flow.internal.start,
                    row.flow.margins.start,
                    row.flow.margins.end,
                    row.flow.extra_end,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            row_flow,
            vec![
                (0.5, 0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0, 0.5),
            ]
        );

        let last_row = rows.last().expect("last collapsed-table row");
        let margin_top = last_row.flow.margins.start;
        let margin_bottom = last_row.flow.margins.end;
        let flow_extra_bottom = last_row.flow.extra_end;
        let row_height = last_row.content.cells.as_slice().row_block_extent();
        assert_eq!(
            estimate_element_height(last_row),
            margin_top + row_height + flow_extra_bottom + margin_bottom,
            "nested layout sizing must reserve the collapsed table's outer border"
        );
        let table_margin_bottom = pages[0].elements[0]
            .1
            .inspect_container(|table| table.box_model.margins.end)
            .expect("table principal box");
        assert_eq!(table_margin_bottom, 4.0);
    }

    #[test]
    fn table_wrapper_auto_margins_center_rows_and_decoration() {
        let parsed = parse_html_with_styles(
            r#"<style>
                * { box-sizing: border-box; margin: 0; }
                table {
                    width: 126px;
                    margin: 0 auto;
                    padding: 7px;
                    border: 2px solid;
                    border-spacing: 3px;
                    background: white;
                }
                td { padding: 0; }
            </style>
            <table><tr><td>Ag Bb</td></tr></table>"#,
        )
        .expect("valid centered table fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|stylesheet| parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(144.0, 150.0),
            Margin::uniform(0.0),
            &rules,
        );
        let row = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_table(Clone::clone))
            .expect("one table row");

        let expected_box_width = 94.5;
        let expected_box_offset = (144.0 - expected_box_width) / 2.0;
        let expected_grid_offset = 6.75;
        assert!((row.box_inline_extent() - expected_box_width).abs() < 0.001);
        assert!((row.grid_inline_offset() - expected_grid_offset).abs() < 0.001);

        let principal_offset = pages[0].elements[0]
            .1
            .inspect_container(|table| table.positioning.insets.left)
            .expect("table principal box");
        assert!((principal_offset - expected_box_offset).abs() < 0.001);
    }

    #[test]
    fn fixed_unspecified_columns_keep_the_declared_table_width() {
        let parsed = parse_html_with_styles(
            r#"<style>
                * { margin: 0; box-sizing: border-box; }
                body { padding: 16px; }
                table { width: 240px; table-layout: fixed; border-collapse: collapse; }
                td { height: 64px; }
                .large { font-size: 64px; background: #f57c00; }
                .small { font-size: 8px; background: #1565c0; }
            </style>
            <table><tr><td class="large"></td><td class="small"></td></tr></table>"#,
        )
        .expect("valid fixed-table fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|stylesheet| parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(204.0, 72.0),
            Margin::uniform(0.0),
            &rules,
        );
        let row = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_table(Clone::clone))
            .expect("one table row");

        assert_eq!(row.content.column_widths, vec![90.0, 90.0]);
        assert_eq!(row.box_inline_extent(), 180.0);
        assert!(pages[0].print_content_scale.is_identity());
    }

    #[test]
    fn table_cell_text_preserves_font_synthesis_none() {
        let parsed = parse_html_with_styles(
            r#"<style>
                td { font-family: ParitySans; font-weight: bold; font-synthesis: none; }
            </style><table><tr><td>Cell</td></tr></table>"#,
        )
        .expect("valid table markup");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|stylesheet| parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let font = crate::parser::ttf::parse_ttf(
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/parity/fonts/ParitySans.ttf"),
            )
            .expect("ParitySans test font"),
        )
        .expect("valid ParitySans test font");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
            &fonts,
            None,
            0.0,
            Default::default(),
        );
        let run = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_table(|row| {
                    row.content
                        .cells
                        .first()
                        .and_then(|cell| cell.layout.content.lines.first())
                        .and_then(|line| line.runs.first())
                        .cloned()
                })?
            })
            .expect("table cell text run");

        assert!(!run.bold);
        assert_eq!(run.font_synthesis.weight, SyntheticFontWeight::Suppressed);
    }

    #[test]
    fn table_fixup_keeps_generated_boundaries_around_improper_block_children() {
        let parsed = parse_html_with_styles(
            r#"<style>
                * { margin: 0; }
                .table { display: table; width: 126px; }
                .table::before { content: 'before'; }
                .table::after { content: 'after'; }
                .child { display: block; height: 20px; }
            </style><div class="table"><div class="child">body</div></div>"#,
        )
        .expect("valid generated table fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|stylesheet| parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(180.0, 120.0),
            Margin::uniform(0.0),
            &rules,
        );
        let generated_boundaries = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_table(|row| {
                    let children = &row.content.cells.first()?.layout.content.children;
                    let first = children.first()?.inspect_text(|block| {
                        block
                            .lines
                            .iter()
                            .flat_map(|line| &line.runs)
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })?;
                    let last = children.last()?.inspect_text(|block| {
                        block
                            .lines
                            .iter()
                            .flat_map(|line| &line.runs)
                            .map(|run| run.text.as_str())
                            .collect::<String>()
                    })?;
                    Some((children.len(), first, last))
                })?
            })
            .expect("anonymous table cell with generated boundary children");

        assert!(generated_boundaries.0 >= 3);
        assert_eq!(generated_boundaries.1, "before");
        assert_eq!(generated_boundaries.2, "after");
    }
}

#[cfg(test)]
mod cell_attribute_tests {
    use super::*;
    use crate::parser::css::parse_stylesheet;

    const CELLPADDING: f32 = 6.0;

    fn first_element(nodes: &[DomNode], tag: HtmlTag) -> Option<&ElementNode> {
        for node in nodes {
            let DomNode::Element(element) = node else {
                continue;
            };
            if element.tag == tag {
                return Some(element);
            }
            if let Some(found) = first_element(&element.children, tag) {
                return Some(found);
            }
        }
        None
    }

    fn cell_padding(
        inline_style: Option<&str>,
        stylesheet: &str,
        parent: &ComputedStyle,
    ) -> EdgeSizes {
        let inline = inline_style
            .map(|style| format!(r#" style="{style}""#))
            .unwrap_or_default();
        let parsed = crate::parser::html::parse_html(&format!(
            r#"<table cellpadding="8"><tr><td{inline}></td></tr></table>"#
        ))
        .expect("valid table fixture");
        let table = first_element(&parsed, HtmlTag::Table).expect("table element");
        let row = first_element(&table.children, HtmlTag::Tr).expect("table row");
        let cell = first_element(&row.children, HtmlTag::Td).expect("table cell");
        let rules = parse_stylesheet(stylesheet);
        compute_style_with_context(
            HtmlTag::Td,
            cell.style_attr(),
            parent,
            &rules,
            "td",
            &[],
            None,
            &cell.attributes,
            &SelectorContext {
                ancestors: vec![
                    AncestorInfo {
                        element: table,
                        child_index: 0,
                        sibling_count: 1,
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    },
                    AncestorInfo {
                        element: row,
                        child_index: 0,
                        sibling_count: 1,
                        preceding_siblings: Vec::new(),
                        following_siblings: Vec::new(),
                        is_empty: false,
                    },
                ],
                ..SelectorContext::default()
            },
        )
        .padding
    }

    #[test]
    fn cellpadding_supplies_all_sides_when_css_is_absent() {
        assert_eq!(
            cell_padding(None, "", &ComputedStyle::default()),
            EdgeSizes::uniform(CELLPADDING)
        );
    }

    #[test]
    fn authored_subpoint_padding_shorthand_wins_by_cascade_not_magnitude() {
        assert_eq!(
            cell_padding(Some("padding: 0.5pt"), "", &ComputedStyle::default()),
            EdgeSizes::uniform(0.5)
        );
    }

    #[test]
    fn authored_padding_sides_override_only_their_presentational_hints() {
        assert_eq!(
            cell_padding(
                Some("padding-top: 0.5pt; padding-left: 2pt"),
                "",
                &ComputedStyle::default()
            ),
            EdgeSizes::new(0.5, CELLPADDING, CELLPADDING, 2.0)
        );
    }

    #[test]
    fn authored_padding_inherit_wins_over_cellpadding() {
        let parent = ComputedStyle {
            padding: EdgeSizes::new(1.0, 2.0, 3.0, 4.0),
            ..ComputedStyle::default()
        };
        assert_eq!(
            cell_padding(Some("padding: inherit"), "", &parent),
            parent.padding
        );
    }

    #[test]
    fn authored_padding_initial_is_not_mistaken_for_an_absent_declaration() {
        assert_eq!(
            cell_padding(Some("padding: initial"), "", &ComputedStyle::default()),
            EdgeSizes::ZERO
        );
    }

    #[test]
    fn stylesheet_inline_and_important_padding_share_one_cascade() {
        assert_eq!(
            cell_padding(
                Some("padding: 2pt"),
                "td { padding: 4pt; padding-right: 0.5pt !important; }",
                &ComputedStyle::default()
            ),
            EdgeSizes::new(2.0, 0.5, 2.0, 2.0)
        );
    }

    #[test]
    fn padding_revert_removes_the_author_origin_hint() {
        assert_eq!(
            cell_padding(Some("padding-top: revert"), "", &ComputedStyle::default()),
            EdgeSizes::new(0.75, CELLPADDING, CELLPADDING, CELLPADDING)
        );
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;

    #[test]
    fn collapsed_column_borders_preserve_definite_cell_track_heights() {
        use crate::layout::elements::LayoutElementTestExt;
        use crate::layout::engine::layout_with_rules;
        use crate::parser::css::parse_stylesheet;
        use crate::parser::html::parse_html_with_styles;
        use crate::types::{Margin, PageSize};

        let parsed = parse_html_with_styles(
            r#"<style>
                * { margin:0; padding:0; box-sizing:border-box }
                table { border-collapse:collapse; border:2px solid }
                col { border:8px solid blue }
                td { width:58px; height:36px; border:none }
            </style><table><colgroup><col><col></colgroup>
            <tr><td></td><td></td></tr><tr><td></td><td></td></tr></table>"#,
        )
        .expect("valid collapsed column-border fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|sheet| parse_stylesheet(sheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(312.0, 168.0),
            Margin::uniform(0.0),
            &rules,
        );
        let heights = pages[0]
            .elements
            .iter()
            .filter_map(|(_, element)| {
                element.inspect_table(|row| row_height_from_cells(&row.content.cells))
            })
            .collect::<Vec<_>>();

        assert!(!heights.is_empty());
        assert!(heights.into_iter().all(|height| height == 27.0));
    }

    #[test]
    fn contextual_table_cells_and_replaced_sibling_keep_table_fixup_boundaries() {
        use crate::layout::elements::LayoutElementTestExt;
        use crate::layout::engine::layout_with_rules;
        use crate::parser::html::parse_html_with_styles;
        use crate::types::{Margin, PageSize};

        let document = parse_html_with_styles(
            r#"<style>
                * { box-sizing:border-box; margin:0; }
                .table { display:table; width:126px; height:68px; padding:7px;
                    border:2px solid #577590; border-spacing:3px; }
                .own { height:22px; white-space:nowrap; }
                .table > .own > .token { display:table-cell; vertical-align:middle; }
                .asset { display:inline-block; width:34px; height:24px; }
            </style>
            <div class="table"><div class="own"><span class="token">Ag</span><span class="token">Bb</span><img class="asset" alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII="></div></div>"#,
        )
        .expect("table fixup fixture should parse");
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(200.0, 120.0),
            Margin::uniform(0.0),
            &rules,
        );

        fn has_fixup_boundary(element: &dyn LayoutElement) -> bool {
            if element
                .inspect_container(|container| {
                    let [table, replaced] = container.children.as_slice() else {
                        return false;
                    };
                    let two_cells = table
                        .inspect_table(|row| row.content.cells.len() == 2)
                        .unwrap_or(false);
                    let one_replaced_cell = replaced
                        .inspect_flex(|row| row.content.cells.len() == 1)
                        .unwrap_or(false);
                    two_cells && one_replaced_cell
                })
                .unwrap_or(false)
            {
                return true;
            }
            let mut found = false;
            element.visit_children(&mut |child| found |= has_fixup_boundary(child));
            found
        }
        assert!(
            pages[0]
                .elements
                .iter()
                .any(|(_, element)| has_fixup_boundary(element.as_ref())),
            "computed table-cell children form an anonymous table before the replaced sibling"
        );
    }

    #[test]
    fn generated_table_rows_expand_the_table_height_minimum() {
        use crate::layout::elements::LayoutElementTestExt;
        use crate::layout::engine::layout_with_rules;
        use crate::parser::html::parse_html_with_styles;
        use crate::types::{Margin, PageSize};

        let document = parse_html_with_styles(
            r#"<style>
                * { box-sizing:border-box; margin:0; }
                .table { display:table; width:126px; height:68px; padding:7px;
                    border:2px solid; border-spacing:3px; }
                .table::before { content:'before'; }
                .table::after { content:'after'; }
                .own { height:22px; white-space:nowrap; }
                .table > .own > span { display:table-cell; }
            </style>
            <div class="table"><div class="own"><span>Ag</span><span>Bb</span></div></div>"#,
        )
        .expect("generated table fixture should parse");
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(200.0, 120.0),
            Margin::uniform(0.0),
            &rules,
        );
        let table_root = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_table(|_| ()).map(|()| element.as_ref()))
            .expect("table principal box");
        let (used_height, is_definite) = table_root
            .inspect_container(|principal| {
                (
                    principal.box_model.size.height.used().unwrap_or_default(),
                    principal.box_model.size.height.is_definite(),
                )
            })
            .expect("table principal geometry");

        assert!(used_height > 68.0 * 0.75);
        assert!(!is_definite, "table height is a minimum, not a hard cap");
        assert!(estimate_element_height(table_root) >= used_height);
    }
}
