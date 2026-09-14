use crate::error::IronpressError;
use crate::layout::cells::{CellBox, CellBoxHolder, GridCell};
use crate::layout::elements::{
    ColumnRule, Container, FlexRow, GridRow, HorizontalRule, Image, LayoutElement, LayoutVisitor,
    MathBlock, ProgressBar, StackingParticipant, Svg, TableRow, TextBlock,
};
use crate::layout::engine::{
    FlexCell, FlexLineId, FootnoteItem, ImageFormat, Page, PngMetadata, TableCell, TextLine,
    TextRun, decode_footnote_link, is_internal_target_anchor, layout_element_paint_order,
};
use crate::layout::paginate::ResolvedFootnoteAreaStyle;
use crate::layout::text::{OverflowWrap, TextWrapOptions, wrap_text_runs};
use crate::parser::ttf::TtfFont;
use crate::render::background::BackgroundPaintContext;
use crate::render::borders::{BorderRing, DoubleBorderMetrics, bevel_edge_color, is_bevel_style};
use crate::render::pdf_fonts::{
    PreparedCustomFont, PreparedCustomFonts, Type3GlyphStyle,
    prepare_custom_fonts_with_additional_runs, prepared_font_name_for_run,
};
use crate::render::pdf_syntax::format_pdf_number;
use crate::render::shading::{
    PdfGradientOffset, PdfGradientStops, PdfRgb, PdfShadingKind, ShadingEntry,
    build_shading_function, push_axial_shading, push_radial_shading,
};
#[cfg(test)]
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{
    AlignItems, BackgroundAttachment, BackgroundClip, BackgroundPosition, BackgroundRepeat,
    BackgroundSize, BorderCollapse, BorderStyle, Clear, ConicGradient, Float, FontFamily,
    GradientInterpolation, GradientRamp, LinearGradient, MaskComposite, MaskLayer, MaskLayerSource,
    MaskMode, MaskSource, Overflow, Position, RadialExtent, RadialGradient, RadialShape,
    ResolvedGradientHint, ResolvedGradientRamp, TextAlign, VerticalAlign,
};
use crate::types::{CornerRadii, EdgeSizes, Margin, PageSize, PhysicalEdges, PhysicalSide, Rect};
use crate::util::{AxisRepeatPattern, MAX_RASTER_TILE_EDGE, RasterDimensions, RasterTile};
use std::collections::HashMap;
use std::io::Write as _;

use crate::layout::engine::FlexNestedOrigin;
#[cfg(test)]
use crate::style::computed::{
    BackgroundOrigin, BorderImage, BorderImagePaint, BorderImageSliceValue, BorderImageSlices,
    BorderImageSource, GradientLayerBox,
};
mod affine_solids;
mod background_images;
mod backgrounds;
mod border_geometry;
mod border_images;
mod border_paint;
mod border_support;
mod cell_effects;
mod clipping;
mod compositing;
mod conic_gradients;
mod container;
mod document;
mod flex_cell_shadows;
mod flow_layout;
mod function_gradients;
mod geometry;
mod gradient_rasters;
mod gradient_support;
mod images;
mod layout_elements;
mod line_metrics;
mod linear_gradients;
mod mask_geometry;
mod mask_paint;
mod masks;
mod math;
mod nested_rows;
mod occlusion;
mod page_elements;
mod page_marks;
mod page_paint_plan;
mod patterns;
mod pdf_text;
mod projection;
mod radial_gradients;
mod raster_effects;
mod raster_placement;
mod resources;
mod shadows;
mod stacking;
mod table_borders;
mod text_baselines;
mod text_lines;
mod text_runs;
mod text_shaping;
mod text_support;
mod transforms;
mod type3_fonts;
mod writer;
mod writer_images;
mod writer_masks;
mod writer_output;

use affine_solids::{render_affine_solid_box, render_affine_solid_group};
use background_images::{BlockBackground, render_block_svg_background, render_svg_background};
use backgrounds::{
    PdfBackgroundPaintContext, PdfBackgroundResources, is_device_clippable_box_background,
    paint_device_clipped_css_solid, paint_solid_background,
};
use border_geometry::*;
use border_images::render_border_image;
use border_paint::*;
use border_support::*;
use cell_effects::{paint_box_filter_output, paint_cell_filter_output};
use clipping::ContentClip;
use compositing::*;
use conic_gradients::*;
use container::*;
use document::*;
#[allow(unused_imports)]
pub use document::{PageDecoration, render_pdf, render_pdf_to_writer, render_pdf_with_fonts};
use flex_cell_shadows::FlexCellShadows;
use flow_layout::*;
use function_gradients::*;
use geometry::{
    BoxPaintGeometry, BoxPaintGrid, FragmentPaintGeometry, LayoutBoxGeometry, PaintBoxGeometry,
    PdfEllipse, PdfMatrix, PdfPoint, PdfRect, PdfVector, RoundedRect,
};
use gradient_rasters::*;
use gradient_support::*;
pub(crate) use images::{DEFAULT_JPEG_QUALITY, ImageRef};
use images::{
    ResizedImage, SvgPageImageSink, decode_png_for_pdf, encode_gray_as_jpeg, encode_rgb_as_jpeg,
    flate_compress, should_try_lossy_png_reencode, try_decode_png_as_opaque_rgb,
};
#[cfg(test)]
use layout_elements::NestedLayoutFrame;
use layout_elements::{
    CellRenderBox, PageRenderContext, compute_grid_row_height, render_cell_content,
    row_baseline_shifts,
};
pub(crate) use line_metrics::sanitize_pdf_name;
use line_metrics::*;
use linear_gradients::*;
use mask_geometry::*;
use mask_paint::*;
use masks::*;
use math::*;
use nested_rows::{NestedRowsFlow, render_rows};
use occlusion::*;
use page_elements::*;
pub(crate) use page_marks::PageSheet;
use page_paint_plan::{ElementPaintPhase, plan_page_elements};
#[cfg(test)]
use patterns::PdfTilingPattern;
use patterns::{
    LayerPaintArea, PdfFunctionPattern, PdfPatternEntry, PdfPatternGeometryFormat,
    PdfShadingPattern, PdfTilingPatternEntry, PdfTilingPatternTarget, RepeatModes,
    gradient_layer_pattern, paint_css_box_pattern, paint_css_page_pattern, paint_distributed_tiles,
    paint_shading_pattern, paint_tiling_pattern,
};
use pdf_text::{build_tounicode_cmap, escape_pdf_string};
#[allow(unused_imports)]
pub(crate) use pdf_text::{
    encode_pdf_text, is_winansi_char, is_winansi_encodable, utf8_to_winansi,
};
use projection::*;
use radial_gradients::*;
use raster_effects::*;
use raster_placement::*;
use resources::*;
use shadows::{render_box_shadows, render_box_shadows_inset};
use stacking::{StackingPaintPlan, StackingScope, StackingTraversal};
use table_borders::*;
use text_baselines::*;
use text_lines::*;
use text_runs::*;
pub(crate) use text_runs::{SUB_SHIFT_RATIO, SUPER_SHIFT_RATIO};
pub(crate) use text_shaping::append_pdf_tj_adjustment;
pub(crate) use text_shaping::sfnt_has_cff_outlines;
use text_shaping::*;
pub(crate) use text_support::emphasis_mark_run;
pub(crate) use text_support::encode_pdf_hex_glyph;
use text_support::*;
use transforms::{
    PageContentTransform, PdfContentSpace, PdfPaintSpace, push_resolved_transform_cm,
    resolve_css_transform,
};
pub(crate) use writer::{PdfWriter, RenderOpts};
use writer_images::PdfImageInterpolation;
use writer_output::PagePaintStreams;

#[cfg(test)]
use layout_elements::{
    CellTextPlacement, NestedTextBlock, plan_nested_layout_elements, render_cell_text,
    render_nested_layout_elements, render_nested_text_block, table_row_total_height,
};

fn collect_document_svg_defs(pages: &[Page]) -> crate::parser::svg::SvgDefs {
    pages
        .first()
        .map(|page| page.document_svg_defs.clone())
        .unwrap_or_default()
}

struct PageElementRenderer<'call, 'frame, 'page> {
    content: &'call mut String,
    frame: PageElementFrame<'frame>,
    paint_phase: ElementPaintPhase,
    bookmarks: &'call mut Vec<BookmarkEntry>,
    ctx: &'call mut PageRenderContext<'page>,
}

impl LayoutVisitor for PageElementRenderer<'_, '_, '_> {
    fn visit_column_rule(&mut self, element: &ColumnRule) {
        let offset = element.placement.offset();
        paint_column_rule_line(
            self.content,
            self.frame.margin.left + offset.x,
            self.frame.page_size.height - self.frame.margin.top - self.frame.y_pos - offset.y,
            element.paint.width,
            element.height,
            &element.paint,
            self.ctx.page_ext_gstates,
            self.ctx.bg_alpha_counter,
        );
    }

    fn visit_text_block(&mut self, element: &TextBlock) {
        render_text_block(
            self.content,
            element,
            self.frame,
            self.paint_phase,
            self.bookmarks,
            self.ctx,
        );
    }

    fn visit_table_row(&mut self, element: &TableRow) {
        render_table_row(self.content, element, self.frame, self.ctx);
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        render_grid_row(self.content, element, self.frame, self.ctx);
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        render_flex_row(
            self.content,
            element,
            self.frame,
            self.paint_phase,
            self.ctx,
        );
    }

    fn visit_container(&mut self, element: &Container) {
        render_container(
            self.content,
            element,
            self.frame,
            self.paint_phase,
            self.ctx,
        );
    }

    fn visit_image(&mut self, element: &Image) {
        render_image(self.content, element, self.frame, self.ctx);
    }

    fn visit_svg(&mut self, element: &Svg) {
        render_svg(self.content, element, self.frame, self.ctx);
    }

    fn visit_horizontal_rule(&mut self, element: &HorizontalRule) {
        let y = self.frame.page_size.height - self.frame.margin.top - self.frame.y_pos;
        let layout_geometry = LayoutBoxGeometry::new(
            PdfRect::from_top(self.frame.margin.left, y, self.frame.available_width, 1.0),
            EdgeSizes::ZERO,
            EdgeSizes::ZERO,
        );
        let box_geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry);
        let paint_geometry = box_geometry.painting();
        let geometry = box_geometry.fragment(Default::default());
        let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
        paint_horizontal_rule(
            self.content,
            PdfPoint::new(
                paint_geometry.border_box.left,
                paint_geometry.border_box.top(),
            ),
            paint_geometry.border_box.width,
        );
        group.finish(self.content, self.ctx);
    }

    fn visit_progress_bar(&mut self, element: &ProgressBar) {
        let bar_y = self.frame.page_size.height
            - self.frame.margin.top
            - self.frame.y_pos
            - element.size.height;
        let rect = PdfRect::new(
            self.frame.margin.left,
            bar_y,
            element.size.width,
            element.size.height,
        );
        let layout_geometry = LayoutBoxGeometry::new(rect, EdgeSizes::ZERO, EdgeSizes::ZERO);
        let box_geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry);
        let paint_geometry = box_geometry.painting();
        let geometry = box_geometry.fragment(Default::default());
        let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
        paint_progress_bar(self.content, element, paint_geometry.border_box);
        group.finish(self.content, self.ctx);
    }

    fn visit_math_block(&mut self, element: &MathBlock) {
        let top = self.frame.page_size.height - self.frame.margin.top - self.frame.y_pos;
        let layout_geometry = LayoutBoxGeometry::new(
            PdfRect::from_top(
                self.frame.margin.left,
                top,
                self.frame.available_width,
                element.layout.height(),
            ),
            EdgeSizes::ZERO,
            EdgeSizes::ZERO,
        );
        let geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry)
            .fragment(Default::default());
        let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
        paint_math_block(
            self.content,
            element,
            PdfPoint::new(self.frame.margin.left, top),
            self.frame.available_width,
        );
        group.finish(self.content, self.ctx);
    }
}

/// Low-level render: raw (uncompressed) content streams for deterministic,
/// inspectable output (used by unit tests and the parity harness, which
/// rasterizes the result). The high-level `HtmlConverter` API enables content-
/// stream compression by default for production output; call
/// `render_pdf_to_writer_full_opts(.., opts)` for compression here.
pub(crate) fn render_pdf_to_writer_full<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
    decoration: Option<&PageDecoration>,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full_opts(
        pages,
        page_size,
        margin,
        writer,
        custom_fonts,
        decoration,
        RenderOpts {
            compress: false,
            ..Default::default()
        },
    )
}

pub(crate) fn render_pdf_to_writer_full_opts<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
    decoration: Option<&PageDecoration>,
    opts: RenderOpts,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full_opts_with_resources(
        PdfRenderDocument::new(pages, page_size, margin, custom_fonts, decoration),
        writer,
        opts,
        crate::security::resources::ResourceLoader::default(),
    )
}

pub(crate) struct PdfRenderDocument<'a> {
    pages: &'a [Page],
    page_size: PageSize,
    margin: Margin,
    custom_fonts: &'a HashMap<String, TtfFont>,
    decoration: Option<&'a PageDecoration>,
}

impl<'a> PdfRenderDocument<'a> {
    pub(crate) fn new(
        pages: &'a [Page],
        page_size: PageSize,
        margin: Margin,
        custom_fonts: &'a HashMap<String, TtfFont>,
        decoration: Option<&'a PageDecoration>,
    ) -> Self {
        Self {
            pages,
            page_size,
            margin,
            custom_fonts,
            decoration,
        }
    }
}

pub(crate) fn render_pdf_to_writer_full_opts_with_resources<W: std::io::Write>(
    document: PdfRenderDocument<'_>,
    writer: &mut W,
    opts: RenderOpts,
    resources: crate::security::resources::ResourceLoader,
) -> Result<(), IronpressError> {
    let PdfRenderDocument {
        pages,
        page_size,
        margin,
        custom_fonts,
        decoration,
    } = document;
    let mut pdf_writer = PdfWriter::with_resources(resources);
    pdf_writer.opts = opts;
    pdf_writer.svg_defs = collect_document_svg_defs(pages);
    // `available_width` is derived per page inside the loop below, since a page
    // may carry an `@page :first` margin override that changes its content box.
    let mut bookmarks: Vec<BookmarkEntry> = Vec::new();
    let margin_font_usage = margin_box_font_usage(pages, decoration, custom_fonts);
    let prepared_custom_fonts = prepare_custom_fonts_with_additional_runs(
        pages,
        custom_fonts,
        &margin_font_usage.per_page_runs,
        &[],
    );

    register_used_custom_fonts(&mut pdf_writer, custom_fonts, &prepared_custom_fonts);

    for (page_idx, page) in pages.iter().enumerate() {
        let page_geometry =
            page.geometry
                .unwrap_or(crate::layout::page_context::PageGeometry::new(
                    page_size, margin,
                ));
        // Per-page physical page size selected by the page-context cascade.
        let page_size = page_geometry.size;
        let sheet = decoration.map_or_else(PageSheet::default, |dec| dec.sheet);
        let media_size = sheet.media_size(page_size);
        // Transparency Form XObjects clip to their /BBox. Use the exact visible
        // sheet in the page's pre-orientation coordinate space: transformed
        // descendants may leave their untransformed element box, but anything
        // outside this rectangle cannot contribute to the output page.
        let page_paint_box = sheet.paint_box(page_size);
        let page_margin = page_geometry.margin;
        let flow_margin = page_geometry.flow_margin();
        let physical_page_transform =
            PageContentTransform::print(PdfVector::new(page_size.width, page_size.height));
        let page_content_transform = physical_page_transform.with_content_scale(
            page.print_content_scale,
            PdfPoint::new(page_margin.left, page_size.height - page_margin.top),
        );
        pdf_writer.page_content_transform = page_content_transform;
        let available_width = page_geometry.content_size().width;
        let mut content = String::new();
        let mut page_background_content = String::new();
        let mut document_canvas_content = String::new();
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();
        let mut page_ext_gstates: Vec<(String, f32)> = Vec::new();
        let mut bg_alpha_counter: usize = 0;
        let mut page_shadings: Vec<ShadingEntry> = Vec::new();
        let mut shading_counter: usize = 0;

        // Optional occlusion culling (default off): rectangles of fully-opaque
        // coverers, used to skip rasters that a later opaque element fully hides.
        let occlusion_coverers = if pdf_writer.opts.occlusion_cull {
            collect_opaque_coverers(page, page_size, flow_margin, available_width)
        } else {
            Vec::new()
        };
        let fixed_textblock_flow_adjustments = fixed_textblock_flow_adjustments(&page.elements);
        {
            let mut ctx = PageRenderContext::new(
                &mut pdf_writer,
                &mut page_images,
                custom_fonts,
                &prepared_custom_fonts,
                &mut page_shadings,
                &mut shading_counter,
                &mut page_ext_gstates,
                &mut bg_alpha_counter,
                &mut annotations,
                page_paint_box,
                page_size.height,
            )
            .with_initial_fixed_origin(PdfPoint::new(
                flow_margin.left,
                page_size.height - flow_margin.top,
            ));
            let mut stacking_plan = StackingPaintPlan::default();
            let mut page_background_plan = StackingPaintPlan::default();
            let mut document_canvas_plan = StackingPaintPlan::default();
            for planned in plan_page_elements(&page.elements) {
                let elem_idx = planned.index;
                let (y_pos, element) = &page.elements[elem_idx];
                let adjusted_y_pos = if element_uses_flow_y_adjustment(element) {
                    *y_pos - fixed_textblock_flow_adjustments[elem_idx]
                } else {
                    *y_pos
                };
                let y_pos = &adjusted_y_pos;
                let element_frame = PageElementFrame {
                    occlusion_coverers: &occlusion_coverers,
                    page_size,
                    margin: flow_margin,
                    available_width,
                    y_pos: *y_pos,
                    element_index: elem_idx,
                    page_index: page_idx,
                };
                let marker = ctx.stacking.marker();
                let annotation_marker = ctx.text.annotation_marker();
                let paint_only = element.is_page_paint_continuation();
                let page_area_paint_space = element
                    .page_area_background()
                    .map(crate::layout::elements::PageAreaBackground::paint_space);
                let physical_page_background = page_area_paint_space
                    == Some(crate::layout::elements::PageAreaPaintSpace::PhysicalPage);
                let mut element_content = String::new();
                let mut discarded_bookmarks = Vec::new();
                if physical_page_background {
                    ctx.text.pdf_writer.page_content_transform = physical_page_transform;
                }
                element.accept(&mut PageElementRenderer {
                    content: &mut element_content,
                    frame: element_frame,
                    paint_phase: planned.phase,
                    bookmarks: if paint_only {
                        &mut discarded_bookmarks
                    } else {
                        &mut bookmarks
                    },
                    ctx: &mut ctx,
                });
                if physical_page_background {
                    ctx.text.pdf_writer.page_content_transform = page_content_transform;
                }
                if paint_only {
                    ctx.text.discard_annotations_since(annotation_marker);
                }
                let descendants = ctx.stacking.take_since(marker);
                let (destination, plan) = match page_area_paint_space {
                    Some(crate::layout::elements::PageAreaPaintSpace::PhysicalPage) => {
                        (&mut page_background_content, &mut page_background_plan)
                    }
                    Some(crate::layout::elements::PageAreaPaintSpace::FittedDocumentCanvas) => {
                        (&mut document_canvas_content, &mut document_canvas_plan)
                    }
                    None => (&mut content, &mut stacking_plan),
                };
                ctx.stacking.commit(
                    StackingScope::Local,
                    destination,
                    plan,
                    layout_element_paint_order(element.as_ref()).with_in_flow_phase(
                        planned.phase.paints_decoration(),
                        planned.phase.paints_contents(),
                    ),
                    element_content,
                    descendants,
                );
            }
            ctx.stacking
                .paint_plan(page_background_plan, &mut page_background_content);
            ctx.stacking
                .paint_plan(document_canvas_plan, &mut document_canvas_content);
            ctx.stacking.paint_plan(stacking_plan, &mut content);
        }

        render_page_footnotes(
            &mut content,
            &page.footnotes,
            page_size,
            page_margin,
            decoration.map(|dec| dec.footnote_area).unwrap_or_default(),
            custom_fonts,
            &prepared_custom_fonts,
            &mut pdf_writer,
            &mut page_images,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
        );

        // Page decorations are already expressed in physical page points.
        // Keep them out of the print-device layout transform so exact
        // half-pixel edges retain their authored page geometry.
        let mut decoration_content = String::new();

        // Render page header/footer in margin area.
        if let Some(dec) = decoration {
            let total_pages = pages.len();
            let page_num = page_idx + 1;
            let center_x = page_size.width / 2.0;

            if let Some(ref header_text) = dec.header {
                let text = header_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let header_y = page_size.height - page_margin.top / 2.0;
                decoration_content.push_str("BT\n");
                decoration_content.push_str("/Helvetica 9 Tf\n");
                decoration_content.push_str("0.4 0.4 0.4 rg\n");
                decoration_content.push_str(&format!("{center_x} {header_y} Td\n"));
                decoration_content.push_str(&format!("({encoded}) Tj\n"));
                decoration_content.push_str("ET\n");
            }

            if let Some(ref footer_text) = dec.footer {
                let text = footer_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let footer_y = page_margin.bottom / 2.0;
                decoration_content.push_str("BT\n");
                decoration_content.push_str("/Helvetica 9 Tf\n");
                decoration_content.push_str("0.4 0.4 0.4 rg\n");
                decoration_content.push_str(&format!("{center_x} {footer_y} Td\n"));
                decoration_content.push_str(&format!("({encoded}) Tj\n"));
                decoration_content.push_str("ET\n");
            }

            // CSS `@page` margin boxes (CSS Paged Media 3 §5): running
            // headers/footers + page counters, resolved per page. `@top-*` boxes
            // paint in the top margin band and `@bottom-*` in the bottom band.
            for (mb_idx, mb) in dec.margin_boxes.iter().enumerate() {
                use crate::parser::css::MarginContentToken;
                if !page_margin_box_wins(&dec.margin_boxes, mb_idx, page, page_num) {
                    continue;
                }
                let margin_text = dec.margin_text.resolve(
                    crate::parser::css::PageSelectorContext {
                        page_number: page_num,
                        is_blank: page.is_blank,
                        page_name: page.page_name.as_deref(),
                    },
                    &mb.text_style,
                    custom_fonts,
                );
                let band = mb.position.band();
                let mut running_element: Option<&dyn LayoutElement> = None;
                let mut text_fragments: Vec<(String, FontFamily)> = Vec::new();
                let page_counter = page_counter_value(mb, page_num);
                for tok in &mb.content {
                    match tok {
                        MarginContentToken::Literal(s) => {
                            text_fragments.push((s.clone(), margin_text.font_family.clone()));
                        }
                        MarginContentToken::PageNumber => {
                            text_fragments
                                .push((page_counter.to_string(), margin_text.font_family.clone()));
                        }
                        MarginContentToken::PageCount => {
                            text_fragments
                                .push((total_pages.to_string(), margin_text.font_family.clone()));
                        }
                        MarginContentToken::Element(reference) => {
                            running_element = page.generated_content.running_element(reference);
                        }
                        MarginContentToken::NamedString(reference) => {
                            let value = page.generated_content.named_string(reference);
                            if let Some(value) = value {
                                text_fragments
                                    .push((value.to_string(), margin_text.font_family.clone()));
                            }
                        }
                    }
                }
                if let Some(element) = running_element {
                    let rendered = band.is_some_and(|band| {
                        let document_transform = pdf_writer.page_content_transform;
                        pdf_writer.page_content_transform = physical_page_transform;
                        let rendered = {
                            let mut margin_ctx = PageRenderContext::new(
                                &mut pdf_writer,
                                &mut page_images,
                                custom_fonts,
                                &prepared_custom_fonts,
                                &mut page_shadings,
                                &mut shading_counter,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                &mut annotations,
                                page_paint_box,
                                page_size.height,
                            )
                            .with_initial_fixed_origin(PdfPoint::new(
                                page_margin.left,
                                page_size.height - page_margin.top,
                            ));
                            render_running_margin_element(
                                &mut decoration_content,
                                element,
                                mb.position.align(),
                                band,
                                page_size,
                                page_margin,
                                mb.background_color,
                                page_idx,
                                &mut margin_ctx,
                            )
                        };
                        pdf_writer.page_content_transform = document_transform;
                        rendered
                    });
                    if rendered {
                        continue;
                    }
                }
                text_fragments.retain(|(text, _)| !text.is_empty());
                let mb_font_size = margin_text.font_size;
                let margin_runs: Vec<TextRun> = text_fragments
                    .into_iter()
                    .map(|(text, font_family)| TextRun {
                        text,
                        font_size: mb_font_size,
                        color: mb.color.unwrap_or(crate::types::Color::BLACK),
                        font_family,
                        line_height_factor: margin_text.line_height_factor,
                        ..Default::default()
                    })
                    .collect();
                let text_w: f32 = margin_runs
                    .iter()
                    .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
                    .sum();
                let margin_frame = PageMarginBoxFrame::new(page_size, page_margin);
                let margin_layout = margin_frame.layout(
                    mb.position,
                    mb.width,
                    text_w,
                    page_margin_box_center_fills_band(&dec.margin_boxes, mb_idx, page, page_num),
                );
                let plain_top_center = page_margin.top / 2.0;
                let plain_bottom_center = page_margin.bottom / 2.0;
                let y = match mb.position {
                    crate::parser::css::MarginBoxPosition::TopLeftCorner
                    | crate::parser::css::MarginBoxPosition::TopLeft
                    | crate::parser::css::MarginBoxPosition::TopCenter
                    | crate::parser::css::MarginBoxPosition::TopRight
                    | crate::parser::css::MarginBoxPosition::TopRightCorner => {
                        page_size.height - plain_top_center
                    }
                    crate::parser::css::MarginBoxPosition::BottomLeftCorner
                    | crate::parser::css::MarginBoxPosition::BottomLeft
                    | crate::parser::css::MarginBoxPosition::BottomCenter
                    | crate::parser::css::MarginBoxPosition::BottomRight
                    | crate::parser::css::MarginBoxPosition::BottomRightCorner => {
                        plain_bottom_center
                    }
                    crate::parser::css::MarginBoxPosition::LeftTop
                    | crate::parser::css::MarginBoxPosition::RightTop => {
                        page_size.height - page_margin.top
                    }
                    crate::parser::css::MarginBoxPosition::LeftMiddle
                    | crate::parser::css::MarginBoxPosition::RightMiddle => page_size.height / 2.0,
                    crate::parser::css::MarginBoxPosition::LeftBottom
                    | crate::parser::css::MarginBoxPosition::RightBottom => page_margin.bottom,
                };
                let line_box = crate::render::pdf::document::PageMarginLineBox::from_runs(
                    &margin_runs,
                    custom_fonts,
                );
                let text_y = band.map_or_else(
                    || y - (line_box.baseline_from_top - line_box.height / 2.0),
                    |band| page_margin_text_baseline(band, page_size, page_margin, line_box),
                );
                let text_x =
                    crate::layout::units::LayoutUnit::from_points_floor(margin_layout.text_x)
                        .to_points();
                if let Some(bg) = mb.background_color {
                    let (r, g, b, a) = bg.to_f32_rgba();
                    if a > 0.0 {
                        let bg_rect = margin_frame.background_rect(mb.position, margin_layout);
                        decoration_content.push_str(&format!("{r} {g} {b} rg\n"));
                        decoration_content.push_str(&bg_rect.rect_path());
                        decoration_content.push_str("f\n");
                    }
                }
                if margin_runs.is_empty() {
                    continue;
                }
                paint_horizontal_line_text(
                    &mut decoration_content,
                    &margin_runs,
                    HorizontalLinePaint {
                        origin: PdfPoint::new(text_x, text_y),
                        line_ascender: line_box.baseline_from_top,
                        justification_word_spacing: 0.0,
                        text_space: PdfContentSpace::page_css(pdf_writer.page_content_transform),
                    },
                    custom_fonts,
                    &prepared_custom_fonts,
                    &mut pdf_writer,
                    &mut page_images,
                );
            }
        }

        sheet.paint_marks(&mut decoration_content, page_size);
        let page_matrix = sheet.page_matrix(page_size).cm_operator();
        let page_area = PdfRect::new(
            page_margin.left,
            page_margin.bottom,
            page_geometry.page_area_size().width,
            page_geometry.page_area_size().height,
        );

        // CSS Page paints the physical page backdrop, the propagated document
        // canvas, then document contents. The canvas participates in print
        // fitting but remains outside the page-area clip; its inverse-sized
        // geometry covers the page area with Chromium's device quantization.
        // The clip slices transformed contents only after pagination, so
        // transferred paint reaches its owning page without leaking from
        // adjacent pages.
        let document_content = format!(
            "q 1 0 0 1 0 0 cm\nq {page_matrix}\
             q {}{page_background_content}Q\n\
             q {}{document_canvas_content}Q\n\
             q {}W n\nq {}{content}Q\nQ\nQ\nQ\n",
            physical_page_transform.operator(),
            page_content_transform.operator(),
            page_area.rect_path(),
            page_content_transform.operator(),
        );
        let decoration_stream = (!decoration_content.is_empty())
            .then(|| format!("q 1 0 0 1 0 0 cm\nq {page_matrix}{decoration_content}Q\nQ\n",));

        for annotation in &mut annotations {
            annotation.rect = page_content_transform.transform_rect(annotation.rect);
        }

        let paint_streams = match decoration_stream.as_deref() {
            Some(decorations) => PagePaintStreams::with_decorations(&document_content, decorations),
            None => PagePaintStreams::document_only(&document_content),
        };
        pdf_writer.add_page(
            media_size.width,
            media_size.height,
            paint_streams,
            annotations,
            page_images,
            page_ext_gstates,
            page_shadings,
        );
    }

    pdf_writer.finish_to_writer(writer, &bookmarks)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests;
