use super::{ImageRef, PageRenderContext, PaintBoxGeometry, push_raster_xobject};
use crate::layout::cells::{CellBox, CellPaint};
use crate::render::pdf::structure::MarkKind;

/// Paint a cell's already-composited filter surface over its source border box.
/// Returns `true` when the filtered output replaced ordinary cell paint.
pub(super) fn paint_cell_filter_output(
    content: &mut String,
    paint: &CellPaint,
    source_geometry: PaintBoxGeometry,
    ctx: &mut PageRenderContext<'_>,
) -> bool {
    let Some(output) = &paint.filter_output else {
        return false;
    };
    let image_id = ctx.text.pdf_writer.add_image_object(
        &output.asset.data,
        output.asset.source_width,
        output.asset.source_height,
        output.asset.format,
        output.asset.png_metadata.as_ref(),
    );
    let image_name = format!("Im{image_id}");
    let painted_box = source_geometry.border_box.outset(output.raster_overflow);
    let mut picture = String::new();
    push_raster_xobject(
        &mut picture,
        &image_name,
        painted_box,
        &output.asset,
        ctx.text.pdf_writer,
    );
    let kind = MarkKind::picture(
        &output.meaning,
        ctx.text
            .pdf_writer
            .page_content_transform
            .transform_rect(painted_box),
    );
    ctx.text
        .pdf_writer
        .structure
        .append_content(content, paint.group.owner, kind, &picture);
    ctx.text.page_images.push(ImageRef {
        name: image_name,
        obj_id: image_id,
    });
    true
}

pub(super) fn paint_box_filter_output(
    content: &mut String,
    cell: &CellBox,
    source_geometry: PaintBoxGeometry,
    ctx: &mut PageRenderContext<'_>,
) -> bool {
    paint_cell_filter_output(content, &cell.paint, source_geometry, ctx)
}
