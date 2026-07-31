use super::*;
use crate::style::computed::{BorderImageSource, ConicGradient, LinearGradient, RadialGradient};
use crate::types::Size;

/// Coordinate space of one CSS image before nine-slice mapping.
#[derive(Debug, Clone, Copy)]
pub(super) struct BorderImageSourceGeometry {
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) number_scale: f32,
    pub(super) natural_slice_scale: Option<f32>,
}

impl BorderImageSourceGeometry {
    pub(super) fn generated(image_area: PdfRect) -> Self {
        Self {
            width: image_area.width,
            height: image_area.height,
            number_scale: crate::fonts::PT_PER_CSS_PX,
            natural_slice_scale: None,
        }
    }

    pub(super) fn natural(width: f32, height: f32) -> Option<Self> {
        ([width, height].into_iter().all(f32::is_finite) && width > 0.0 && height > 0.0).then_some(
            Self {
                width,
                height,
                number_scale: 1.0,
                natural_slice_scale: Some(crate::fonts::PT_PER_CSS_PX),
            },
        )
    }
}

pub(super) enum ResolvedBorderImageSource<'a> {
    Linear(&'a LinearGradient),
    Radial(&'a RadialGradient),
    Conic(&'a ConicGradient),
    Raster(crate::layout::engine::RasterImageAsset),
    Svg(Box<crate::parser::svg::SvgTree>),
}

impl ResolvedBorderImageSource<'_> {
    pub(super) fn prepare(&mut self, image_area: PdfRect) -> Option<BorderImageSourceGeometry> {
        match self {
            Self::Linear(_) | Self::Radial(_) | Self::Conic(_) => {
                Some(BorderImageSourceGeometry::generated(image_area))
            }
            Self::Raster(image) => BorderImageSourceGeometry::natural(
                image.source_width as f32,
                image.source_height as f32,
            ),
            Self::Svg(tree) => prepare_svg_source(tree, image_area),
        }
    }

    pub(super) const fn needs_slice_edge_clamp(&self) -> bool {
        matches!(self, Self::Raster(_) | Self::Svg(_))
    }
}

fn prepare_svg_source(
    tree: &mut crate::parser::svg::SvgTree,
    image_area: PdfRect,
) -> Option<BorderImageSourceGeometry> {
    let concrete = crate::layout::images::resolve_svg_image_size(
        tree,
        Size::new(image_area.width, image_area.height),
    );
    let css_pixel = crate::fonts::PT_PER_CSS_PX;
    let source_width = concrete.width / css_pixel;
    let source_height = concrete.height / css_pixel;
    let geometry = BorderImageSourceGeometry::natural(source_width, source_height)?;

    if let Some(markup) = tree.source_markup.clone()
        && let Some(reparsed) = crate::parser::svg::parse_svg_from_string_with_viewport(
            &markup,
            Some((source_width, source_height)),
        )
    {
        *tree = reparsed;
    }
    tree.width = source_width;
    tree.height = source_height;
    Some(geometry)
}

pub(super) fn resolve_border_image_source(
    source: &BorderImageSource,
) -> Option<ResolvedBorderImageSource<'_>> {
    match source {
        BorderImageSource::LinearGradient(gradient) => {
            Some(ResolvedBorderImageSource::Linear(gradient))
        }
        BorderImageSource::RadialGradient(gradient) => {
            Some(ResolvedBorderImageSource::Radial(gradient))
        }
        BorderImageSource::ConicGradient(gradient) => {
            Some(ResolvedBorderImageSource::Conic(gradient))
        }
        BorderImageSource::Url(url) => resolve_url_source(url),
    }
}

fn resolve_url_source(url: &str) -> Option<ResolvedBorderImageSource<'static>> {
    let (bytes, mime) = crate::layout::images::load_resource(url, None)?;
    let skip_svg = mime
        .as_deref()
        .is_some_and(|mime| !mime.contains("svg") && !mime.contains("xml"));
    if !skip_svg && let Some(tree) = crate::layout::images::try_parse_svg_bytes(&bytes) {
        return Some(ResolvedBorderImageSource::Svg(Box::new(tree)));
    }
    crate::layout::images::load_image_bytes(bytes.to_vec()).map(ResolvedBorderImageSource::Raster)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn register_border_image_source(
    source: &ResolvedBorderImageSource<'_>,
    source_box: PdfRect,
    image_area: PdfRect,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    page_ext_gstates: &mut Vec<(String, f32)>,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> ImageRef {
    let mut stream = String::new();
    match source {
        ResolvedBorderImageSource::Linear(gradient) => render_linear_gradient_layer_tile(
            &mut stream,
            gradient.source(),
            GradientBackdrop::default(),
            source_box,
            PageContentTransform::default(),
            shadings,
            shading_counter,
            pdf_writer,
            page_images,
        ),
        ResolvedBorderImageSource::Radial(gradient) => render_radial_gradient_layer_tile(
            &mut stream,
            gradient,
            source_box,
            PageContentTransform::default(),
            shadings,
            shading_counter,
            pdf_writer,
            page_images,
        ),
        ResolvedBorderImageSource::Conic(gradient) => render_conic_gradient_layer_tile(
            &mut stream,
            gradient,
            source_box,
            pdf_writer,
            page_images,
        ),
        ResolvedBorderImageSource::Raster(image) => {
            let object_id = pdf_writer.add_layout_image_object(
                image,
                image_area.width,
                image_area.height,
                crate::style::computed::ImageRendering::Auto,
            );
            let image = ImageRef {
                name: format!("Im{object_id}"),
                obj_id: object_id,
            };
            stream.push_str(&format!(
                "{} 0 0 {} 0 0 cm\n/{} Do\n",
                source_box.width, source_box.height, image.name
            ));
            page_images.push(image);
        }
        ResolvedBorderImageSource::Svg(tree) => render_svg_source(
            &mut stream,
            tree,
            source_box,
            image_area,
            shadings,
            shading_counter,
            page_ext_gstates,
            pdf_writer,
            page_images,
        ),
    }
    pdf_writer.add_plain_local_form(stream, source_box)
}

/// Materialize one of the nine source regions as its own image.
///
/// CSS slices the source image before scaling and tiling. Repeatedly clipping
/// the unsliced source at the destination lets antialiasing sample adjacent
/// source regions, producing visible coloured seams between tiles. A nested
/// form gives the slice an independent bounding box, matching the specified
/// image-slicing model and allowing every repeated tile to reuse it.
pub(super) fn register_border_image_slice(
    resolved_source: &ResolvedBorderImageSource<'_>,
    registered_source: &ImageRef,
    source_region: PdfRect,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> ImageRef {
    if let ResolvedBorderImageSource::Raster(asset) = resolved_source
        && let Some(slice) =
            register_raster_border_image_slice(asset, source_region, pdf_writer, page_images)
    {
        return slice;
    }

    // PDF clipping is antialiased after painting. Move the sample boundary a
    // negligible distance into the selected image region, then expand that
    // sample back to the exact slice bounds. This implements edge clamping:
    // neighbouring regions cannot contribute colour to the sliced image.
    let clamp = (source_region.width.min(source_region.height) * 0.01).min(0.125);
    let sample_region = source_region.inset(EdgeSizes::uniform(clamp));
    let sample_to_slice =
        PdfMatrix::translate(PdfPoint::new(source_region.left, source_region.bottom))
            * PdfMatrix::scale(PdfVector::new(
                source_region.width / sample_region.width,
                source_region.height / sample_region.height,
            ))
            * PdfMatrix::translate(PdfPoint::new(-sample_region.left, -sample_region.bottom));
    let mut stream = String::from("q\n");
    stream.push_str(&source_region.rect_path());
    stream.push_str("W n\n");
    stream.push_str(&sample_to_slice.cm_operator());
    stream.push_str(&format!("/{} Do\nQ\n", registered_source.name));
    pdf_writer.add_plain_local_form(stream, source_region)
}

/// Register an integer-aligned raster slice as an independent image XObject.
///
/// A PDF clip limits coverage, but it does not limit the interpolation kernel
/// of the image being sampled. Keeping a whole nine-patch source behind nine
/// clips therefore lets neighbouring source pixels colour repeated seams.
/// Browsers isolate raster source rectangles before repetition; for the common
/// integer-pixel case, embedding the cropped pixels expresses that model
/// directly and losslessly. Fractional slices retain the vector clamp fallback
/// above because resampling them here would change the authored source.
fn register_raster_border_image_slice(
    asset: &crate::layout::engine::RasterImageAsset,
    source_region: PdfRect,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> Option<ImageRef> {
    // Raster rows are top-down while border-image source coordinates are
    // bottom-up PDF coordinates.
    let crop_y = asset.source_height as f32 - source_region.top();
    let crop = crate::layout::images::RasterCrop::aligned(
        crate::types::Rect::from_xywh(
            source_region.left,
            crop_y,
            source_region.width,
            source_region.height,
        ),
        crate::util::RasterDimensions {
            width: asset.source_width,
            height: asset.source_height,
        },
    )?;
    let cropped = crate::layout::images::crop_raster_asset(asset, crop)?;
    let object_id = pdf_writer.add_layout_image_object(
        &cropped,
        source_region.width * crate::fonts::PT_PER_CSS_PX,
        source_region.height * crate::fonts::PT_PER_CSS_PX,
        crate::style::computed::ImageRendering::Auto,
    );
    let image = ImageRef {
        name: format!("Im{object_id}"),
        obj_id: object_id,
    };
    page_images.push(image.clone());

    let mut stream = String::from("q\n");
    stream.push_str(&source_region.rect_path());
    stream.push_str("W n\n");
    stream.push_str(
        &PdfMatrix::translate(PdfPoint::new(source_region.left, source_region.bottom))
            .cm_operator(),
    );
    stream.push_str(
        &PdfMatrix::scale(PdfVector::new(source_region.width, source_region.height)).cm_operator(),
    );
    stream.push_str(&format!("/{} Do\nQ\n", image.name));
    Some(pdf_writer.add_plain_local_form(stream, source_region))
}

#[allow(clippy::too_many_arguments)]
fn render_svg_source(
    stream: &mut String,
    tree: &crate::parser::svg::SvgTree,
    source_box: PdfRect,
    image_area: PdfRect,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    page_ext_gstates: &mut Vec<(String, f32)>,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    stream.push_str(&format!("1 0 0 -1 0 {} cm\n", source_box.height));
    let mut image_sink = SvgPageImageSink {
        pdf_writer,
        page_images,
    };
    let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
        shadings,
        shading_counter,
        ext_gstates: Some(page_ext_gstates),
        image_sink: Some(&mut image_sink),
        raster_scale_x: image_area.width / source_box.width,
        raster_scale_y: image_area.height / source_box.height,
        custom_fonts: None,
        prepared_custom_fonts: None,
    };
    crate::render::svg_to_pdf::render_svg_tree_in_viewport(
        tree,
        source_box.width,
        source_box.height,
        stream,
        &mut resources,
    );
}
