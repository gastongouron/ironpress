use super::*;
use crate::layout::cells::TableRowCells;
use crate::layout::elements::{GridRow, LayoutVisitor, TableRow};

mod grid;
mod table;

fn table_row_height(element: &dyn LayoutElement) -> Option<f32> {
    #[derive(Default)]
    struct Height(Option<f32>);

    impl LayoutVisitor for Height {
        fn visit_table_row(&mut self, element: &TableRow) {
            self.0 = Some(element.content.cells.row_block_extent());
        }
    }

    let mut height = Height::default();
    element.accept(&mut height);
    height.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FirstMarginState {
    #[default]
    Pending,
    Resolved,
}

/// CSS table-cell paint phases coordinated across every row of one table grid.
///
/// A cell whose compositing must remain atomic is emitted in `Contents`; an
/// ordinary in-flow cell contributes independently to all three phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TableCellPaintPhase {
    #[default]
    All,
    Backgrounds,
    Borders,
    Contents,
}

impl TableCellPaintPhase {
    const fn paints_backgrounds(self) -> bool {
        matches!(self, Self::All | Self::Backgrounds)
    }

    const fn paints_borders(self) -> bool {
        matches!(self, Self::All | Self::Borders)
    }

    const fn paints_contents(self) -> bool {
        matches!(self, Self::All | Self::Contents)
    }
}

/// Entry state for a contiguous run of internal table/grid rows.
///
/// Reordered painting receives positions from the shared flow planner, where
/// the first table margin has already been collapsed; source-order painting
/// resolves it here. The distinction is explicit so a margin is never applied
/// twice merely because stacking order required a planning pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NestedRowsFlow {
    position: FlowPosition,
    first_margin: FirstMarginState,
}

impl NestedRowsFlow {
    pub(super) const fn pending(position: FlowPosition) -> Self {
        Self {
            position,
            first_margin: FirstMarginState::Pending,
        }
    }

    pub(super) const fn resolved(position: FlowPosition) -> Self {
        Self {
            position,
            first_margin: FirstMarginState::Resolved,
        }
    }
}

struct NestedRowsRenderer<'call, 'fonts> {
    content: &'call mut String,
    paint: bool,
    origin_x: f32,
    cursor_y: f32,
    page_ext_gstates: &'call mut Vec<(String, f32)>,
    bg_alpha_counter: &'call mut usize,
    custom_fonts: &'fonts dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &'fonts PreparedCustomFonts,
    page_shadings: &'call mut Vec<ShadingEntry>,
    shading_counter: &'call mut usize,
    pdf_writer: &'call mut PdfWriter,
    page_images: &'call mut Vec<ImageRef>,
    annotations: &'call mut Vec<LinkAnnotation>,
    stacking: &'call mut StackingTraversal,
    abs_origins: &'call mut HashMap<usize, PdfPoint>,
    page_paint_box: PdfRect,
    initial_fixed_origin: PdfPoint,
    page_height: f32,
    previous_margin_bottom: f32,
    first_margin: FirstMarginState,
    row_heights: Vec<Option<f32>>,
    element_index: usize,
    table_cell_phase: TableCellPaintPhase,
    handled: bool,
}

impl LayoutVisitor for NestedRowsRenderer<'_, '_> {
    fn visit_table_row(&mut self, element: &TableRow) {
        self.render_table_row(element);
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        self.render_grid_row(element);
    }
}

/// Render table and grid rows at a resolved flow origin.
///
/// Page roots, ordinary containers, table cells, and grid/flex descendants all
/// enter this same painter; only the flow origin differs.
pub(super) fn render_rows(
    content: &mut String,
    elements: &[&dyn LayoutElement],
    origin_x: f32,
    flow: NestedRowsFlow,
    paint: bool,
    abs_origins: &mut HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) -> FlowPosition {
    let mut position = flow.position;
    let mut first_margin = flow.first_margin;
    let mut start = 0;
    while start < elements.len() {
        let table_grid = elements[start]
            .table_grid_owner()
            .map(crate::layout::elements::TableGridOwner::table_grid_identity);
        let end = if let Some(table_grid) = table_grid {
            elements[start + 1..]
                .iter()
                .position(|element| {
                    element
                        .table_grid_owner()
                        .map(crate::layout::elements::TableGridOwner::table_grid_identity)
                        != Some(table_grid)
                })
                .map_or(elements.len(), |offset| start + 1 + offset)
        } else {
            start + 1
        };
        let segment_flow = NestedRowsFlow {
            position,
            first_margin,
        };
        if paint && table_grid.is_some() {
            for phase in [
                TableCellPaintPhase::Backgrounds,
                TableCellPaintPhase::Borders,
                TableCellPaintPhase::Contents,
            ] {
                position = render_row_pass(
                    content,
                    &elements[start..end],
                    origin_x,
                    segment_flow,
                    true,
                    phase,
                    abs_origins,
                    ctx,
                );
            }
        } else {
            position = render_row_pass(
                content,
                &elements[start..end],
                origin_x,
                segment_flow,
                paint,
                TableCellPaintPhase::All,
                abs_origins,
                ctx,
            );
        }
        first_margin = FirstMarginState::Pending;
        start = end;
    }
    position
}

#[allow(clippy::too_many_arguments)]
fn render_row_pass(
    content: &mut String,
    elements: &[&dyn LayoutElement],
    origin_x: f32,
    flow: NestedRowsFlow,
    paint: bool,
    table_cell_phase: TableCellPaintPhase,
    abs_origins: &mut HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) -> FlowPosition {
    let row_heights = elements
        .iter()
        .map(|element| table_row_height(*element))
        .collect();
    let mut renderer = NestedRowsRenderer {
        content,
        paint,
        origin_x,
        cursor_y: flow.position.cursor_y,
        page_ext_gstates: ctx.page_ext_gstates,
        bg_alpha_counter: ctx.bg_alpha_counter,
        custom_fonts: ctx.text.custom_fonts,
        prepared_custom_fonts: ctx.text.prepared_custom_fonts,
        page_shadings: ctx.shadings,
        shading_counter: ctx.shading_counter,
        pdf_writer: ctx.text.pdf_writer,
        page_images: ctx.text.page_images,
        annotations: ctx.text.annotations,
        stacking: &mut ctx.stacking,
        abs_origins,
        page_paint_box: ctx.paint_box,
        initial_fixed_origin: ctx.initial_fixed_origin,
        page_height: ctx.text.page_height,
        previous_margin_bottom: flow.position.previous_margin_bottom,
        first_margin: flow.first_margin,
        row_heights,
        element_index: 0,
        table_cell_phase,
        handled: false,
    };
    for (element_index, &element) in elements.iter().enumerate() {
        renderer.element_index = element_index;
        renderer.handled = false;
        element.accept(&mut renderer);
        if !renderer.handled {
            renderer.cursor_y -= crate::layout::paginate::estimate_element_height(element);
            renderer.previous_margin_bottom = 0.0;
        }
    }
    FlowPosition::new(
        renderer.cursor_y,
        renderer.cursor_y,
        renderer.previous_margin_bottom,
    )
}
