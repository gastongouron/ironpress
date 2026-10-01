use super::*;
use crate::layout::elements::{CollapsedBorderEdge, CollapsedTableBorders};

/// Page-space geometry for the one collapsed-border grid slice owned by a
/// table row.
///
/// Layout retains exact track positions. Each complete edge band crosses the
/// Chromium-compatible paint boundary once, after joint ownership has adjusted
/// its endpoints.
#[derive(Debug, Clone, Copy)]
pub(in crate::render::pdf) struct CollapsedRowBorderGeometry<'a> {
    column_widths: &'a [f32],
    grid_left: f32,
    row_top: f32,
    row_height: f32,
    page_content: PageContentTransform,
}

impl<'a> CollapsedRowBorderGeometry<'a> {
    pub(in crate::render::pdf) const fn new(
        column_widths: &'a [f32],
        grid_left: f32,
        row_top: f32,
        row_height: f32,
        page_content: PageContentTransform,
    ) -> Self {
        Self {
            column_widths,
            grid_left,
            row_top,
            row_height,
            page_content,
        }
    }

    fn column_boundaries(self) -> Vec<f32> {
        let mut boundaries = Vec::with_capacity(self.column_widths.len().saturating_add(1));
        boundaries.push(self.grid_left);
        let mut boundary = self.grid_left;
        for width in self.column_widths {
            boundary += *width;
            boundaries.push(boundary);
        }
        boundaries
    }

    fn horizontal_band(
        self,
        boundaries: &[f32],
        column: usize,
        y: f32,
        edge: CollapsedBorderEdge,
    ) -> Option<PdfRect> {
        let start = boundaries.get(column).copied()? + edge.joints.start.inset();
        let end = boundaries.get(column.saturating_add(1)).copied()? - edge.joints.end.inset();
        let width = end - start;
        (edge.side.paints() && width > 0.0 && edge.side.width > 0.0).then(|| {
            let thickness = self.page_content.snapped_border_width(edge.side.width);
            self.page_content.snap_layout_box(PdfRect::new(
                start,
                y - thickness / 2.0,
                width,
                thickness,
            ))
        })
    }

    fn vertical_band(
        self,
        boundaries: &[f32],
        column_line: usize,
        edge: CollapsedBorderEdge,
    ) -> Option<PdfRect> {
        let x = boundaries.get(column_line).copied()?;
        let top = self.row_top - edge.joints.start.inset();
        let bottom = self.row_top - self.row_height + edge.joints.end.inset();
        let height = top - bottom;
        (edge.side.paints() && height > 0.0 && edge.side.width > 0.0).then(|| {
            let thickness = self.page_content.snapped_border_width(edge.side.width);
            self.page_content.snap_layout_box(PdfRect::new(
                x - thickness / 2.0,
                bottom,
                thickness,
                height,
            ))
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::render::pdf) fn paint_resolved_collapsed_row_borders(
    content: &mut String,
    borders: &CollapsedTableBorders,
    geometry: CollapsedRowBorderGeometry<'_>,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    let boundaries = geometry.column_boundaries();
    for (column, edge) in borders.block_start.iter().copied().enumerate() {
        if let Some(band) = geometry.horizontal_band(&boundaries, column, geometry.row_top, edge) {
            paint_collapsed_border_band(
                content,
                edge,
                PhysicalSide::Top,
                band,
                page_ext_gstates,
                bg_alpha_counter,
            );
        }
    }
    for (column_line, edge) in borders.block_axis.iter().copied().enumerate() {
        if let Some(band) = geometry.vertical_band(&boundaries, column_line, edge) {
            paint_collapsed_border_band(
                content,
                edge,
                PhysicalSide::Left,
                band,
                page_ext_gstates,
                bg_alpha_counter,
            );
        }
    }
    let block_end = geometry.row_top - geometry.row_height;
    for (column, edge) in borders.block_end.iter().copied().enumerate() {
        if let Some(band) = geometry.horizontal_band(&boundaries, column, block_end, edge) {
            paint_collapsed_border_band(
                content,
                edge,
                PhysicalSide::Top,
                band,
                page_ext_gstates,
                bg_alpha_counter,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_collapsed_border_band(
    content: &mut String,
    edge: CollapsedBorderEdge,
    side: PhysicalSide,
    band: PdfRect,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    if band.is_empty() {
        return;
    }
    let horizontal = matches!(side, PhysicalSide::Top | PhysicalSide::Bottom);
    let mut border = edge.side;
    border.width = if horizontal { band.height } else { band.width };
    let (start, end) = if horizontal {
        (
            PdfPoint::new(band.left, band.bottom + band.height / 2.0),
            PdfPoint::new(band.right(), band.bottom + band.height / 2.0),
        )
    } else {
        (
            PdfPoint::new(band.left + band.width / 2.0, band.top()),
            PdfPoint::new(band.left + band.width / 2.0, band.bottom),
        )
    };
    paint_table_cell_border_line(
        content,
        &border,
        side,
        start.x,
        start.y,
        end.x,
        end.y,
        page_ext_gstates,
        bg_alpha_counter,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::elements::{
        CollapsedBorderJoint, CollapsedBorderJoints, CollapsedBorderLine,
    };
    use crate::layout::engine::LayoutBorderSide;

    fn solid(width: f32) -> LayoutBorderSide {
        LayoutBorderSide {
            width,
            style: BorderStyle::Solid,
            ..Default::default()
        }
    }

    #[test]
    fn stronger_perpendicular_edge_owns_the_joint_frontier() {
        let horizontal = CollapsedBorderEdge::new(
            solid(3.75),
            CollapsedBorderJoints {
                start: CollapsedBorderJoint::resolve(6.0, false),
                end: CollapsedBorderJoint::resolve(6.0, false),
            },
        );
        let borders = CollapsedTableBorders::new(
            CollapsedBorderLine::new(vec![horizontal]),
            CollapsedBorderLine::default(),
            CollapsedBorderLine::default(),
        );
        let geometry = CollapsedRowBorderGeometry::new(
            &[46.5],
            21.0,
            100.0,
            27.0,
            PageContentTransform::default(),
        );
        let boundaries = geometry.column_boundaries();
        let band = geometry
            .horizontal_band(&boundaries, 0, geometry.row_top, horizontal)
            .expect("specified solid edge has positive geometry");

        assert_eq!(band.left, 24.0);
        assert_eq!(band.right(), 64.5);
        assert_eq!(borders.block_start.iter().count(), 1);
    }
    #[test]
    fn sub_pixel_collapsed_edges_never_snap_away_on_a_print_page() {
        // A 0.2mm rule is 0.76 CSS px. Snapping both edges of such a band to
        // the pixel grid can put them on the same line, which erased the edge
        // outright (nested checkbox squares lost their top or left side
        // depending on their fractional position). Chromium paints at least
        // one pixel, so every position must keep a one-pixel band.
        let width = 0.2 * 72.0 / 25.4;
        let edge = CollapsedBorderEdge::new(solid(width), CollapsedBorderJoints::default());
        let page = PageContentTransform::print(PdfVector::new(612.0, 792.0));
        for step in 0..40 {
            let offset = step as f32 * 0.05;
            let geometry = CollapsedRowBorderGeometry::new(
                &[8.22],
                100.0 + offset,
                700.0 + offset,
                9.35,
                page,
            );
            let boundaries = geometry.column_boundaries();
            let top = geometry
                .horizontal_band(&boundaries, 0, geometry.row_top, edge)
                .expect("a painting edge always yields a band");
            assert!(
                (top.height - 0.75).abs() < 1e-4,
                "top band at offset {offset} lost its pixel: {top:?}"
            );
            let left = geometry
                .vertical_band(&boundaries, 0, edge)
                .expect("a painting edge always yields a band");
            assert!(
                (left.width - 0.75).abs() < 1e-4,
                "left band at offset {offset} lost its pixel: {left:?}"
            );
        }
    }

    #[test]
    fn wider_collapsed_edges_keep_their_authored_thickness_on_a_print_page() {
        let edge = CollapsedBorderEdge::new(solid(3.0), CollapsedBorderJoints::default());
        let page = PageContentTransform::print(PdfVector::new(612.0, 792.0));
        let geometry = CollapsedRowBorderGeometry::new(&[46.5], 21.0, 700.0, 27.0, page);
        let boundaries = geometry.column_boundaries();
        let band = geometry
            .horizontal_band(&boundaries, 0, geometry.row_top, edge)
            .expect("a painting edge always yields a band");
        assert_eq!(band.height, 3.0);
    }
}
