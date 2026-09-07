//! CSS filter compositing over laid-out element subtrees.
//!
//! Layout supplies one semantic source tree. Filter painting turns that tree
//! into `SourceGraphic` once, then applies the ordered filter list to the
//! resulting surface. Individual boxes, glyphs, and replaced descendants must
//! never be filtered independently when CSS requires a group.

pub(crate) mod cells;
mod fallback;
mod materialize;
mod paint_space;
mod raster_frame;
pub(crate) mod surface;
mod vector_source;

pub(crate) use materialize::materialize_page_filters;
pub(crate) use vector_source::ExactVectorFilterSource;

use std::collections::HashMap;

use crate::layout::elements::{
    Image, ImagePaint, ImageSampling, IntoLayoutNode, LayoutNode, ReplacedGeometry,
};
use crate::layout::engine::{LayoutBorder, RasterImageAsset};
use crate::parser::dom::ElementNode;
use crate::style::computed::{ComputedStyle, FilterOperation, NormalizedFilterRegion, ObjectFit};
use crate::types::EdgeSizes;

/// An owned filter surface kept with the semantic box that produced it.
///
/// The raster is already rendered at the configured filter DPI. Its overflow
/// records the portion added outside the source border box by operations such
/// as blur and drop-shadow.
#[derive(Debug, Clone)]
pub(crate) struct FilterRasterOutput {
    pub(crate) asset: RasterImageAsset,
    pub(crate) raster_overflow: EdgeSizes,
}

/// A resolved filter list together with its color-interpolation space.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedFilter {
    pub(crate) operations: Vec<FilterOperation>,
    pub(crate) linear_rgb: bool,
    pub(crate) svg_region: Option<NormalizedFilterRegion>,
    pub(crate) isolates_source: bool,
}

/// Most image filters can consume only a scale-and-translate parameter matrix.
/// Colour-only graphs are affine-local and can consume the complete matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilterMatrixCapability {
    ScaleTranslate,
    Complex,
}

/// Effects applied after an element's filter has produced its composited
/// output. CSS applies these to the filtered group rather than to each source
/// primitive independently.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FilterCompositing {
    output_clip: FilterOutputClip,
}

impl FilterCompositing {
    fn from_group(group: &crate::layout::elements::PaintGroup) -> Self {
        Self {
            output_clip: if group.effects.masking.image.is_some() {
                FilterOutputClip::BorderBox
            } else {
                FilterOutputClip::None
            },
        }
    }
}

/// Conservative finite bound imposed after filter evaluation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FilterOutputClip {
    #[default]
    None,
    /// Every supported `mask-clip` geometry box is contained by the border box.
    BorderBox,
}

impl ResolvedFilter {
    pub(crate) fn from_style(
        style: &mut ComputedStyle,
        definitions: &HashMap<String, ElementNode>,
    ) -> Self {
        let mut linear_rgb = false;
        let mut svg_region = None;
        let isolates_source = style.filter.establishes_stacking_context;
        if let Some(id) = style.filter.url_id.clone() {
            let Some(filter) = definitions.get(&id) else {
                style.filter = Default::default();
                return Self::default();
            };
            let Some(definition) = crate::parser::svg::filter_element_definition(filter) else {
                style.filter = Default::default();
                return Self::default();
            };
            if !definition.operations.is_empty() {
                linear_rgb = definition.linear_rgb;
            }
            svg_region = Some(definition.region);
            style.filter.operations.extend(definition.operations);
        }
        Self {
            operations: style.filter.operations.clone(),
            linear_rgb,
            svg_region,
            isolates_source,
        }
    }

    pub(crate) const fn requires_source_surface(&self) -> bool {
        self.isolates_source
    }

    pub(super) fn matrix_capability(&self) -> FilterMatrixCapability {
        let is_color_operation = |operation: &FilterOperation| {
            matches!(
                operation,
                FilterOperation::Grayscale(_)
                    | FilterOperation::Sepia(_)
                    | FilterOperation::Invert(_)
                    | FilterOperation::Brightness(_)
                    | FilterOperation::Contrast(_)
                    | FilterOperation::Saturate(_)
                    | FilterOperation::HueRotate(_)
                    | FilterOperation::Opacity(_)
                    | FilterOperation::Matrix(_)
            )
        };
        if self.operations.iter().all(is_color_operation) {
            FilterMatrixCapability::Complex
        } else {
            FilterMatrixCapability::ScaleTranslate
        }
    }
}

/// One renderer-owned filtered SourceGraphic, before it is reinserted into
/// normal flow as an atomic image.
pub(crate) struct FilteredGraphic {
    asset: RasterImageAsset,
    geometry: surface::SourceGeometry,
    overflow: EdgeSizes,
    group: crate::layout::elements::PaintGroup,
}

impl FilteredGraphic {
    pub(crate) fn into_layout_node(self) -> LayoutNode {
        Image {
            source: self.asset,
            geometry: ReplacedGeometry::new(
                self.geometry.size,
                self.geometry.flow.margins,
                LayoutBorder::default(),
            ),
            positioning: self.geometry.positioning,
            sampling: ImageSampling {
                replaced: crate::layout::engine::ReplacedContent {
                    object_fit: ObjectFit::Fill,
                    ..Default::default()
                },
                ..Default::default()
            },
            paint: ImagePaint {
                raster_overflow: self.overflow,
                group: self.group,
                ..Default::default()
            },
        }
        .boxed()
    }
}

/// Retain a resolved filter on its semantic box until pagination has split the
/// box into the fragments to which graphical effects are applied.
pub(crate) fn retain_for_fragmentation(
    element: &mut dyn crate::layout::elements::LayoutElement,
    filter: &ResolvedFilter,
) -> bool {
    let Some(holder) = element.filter_holder_mut() else {
        return false;
    };
    *holder.filter_slot_mut() = Some(filter.clone());
    true
}

pub(crate) fn composite_source(
    element: &dyn crate::layout::elements::LayoutElement,
    filter: &ResolvedFilter,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
    raster_space: surface::SourceRasterSpace,
) -> Option<FilteredGraphic> {
    let exact_vector = element.exact_vector_filter_source().is_some_and(|source| {
        !filter.linear_rgb && source.supports_exact_vector_filter(&filter.operations)
    });
    if !filter.requires_source_surface() || exact_vector {
        return None;
    }
    let group = element
        .paint_group_owner()
        .map(crate::layout::elements::PaintGroupOwner::paint_group);
    let source = surface::paint_source_graphic(element, fonts, filter_dpi, raster_space)?;
    let (output, geometry) = composite_source_graphic(
        source,
        filter,
        group.map(FilterCompositing::from_group).unwrap_or_default(),
        filter_dpi,
    )?;
    Some(FilteredGraphic {
        asset: output.asset,
        geometry,
        overflow: output.raster_overflow,
        group: group
            .cloned()
            .unwrap_or_default()
            .with_materialized_filter(),
    })
}

/// Apply a resolved filter to a source surface while retaining the source box
/// geometry separately from filter overflow.
pub(crate) fn composite_source_graphic(
    source: surface::SourceGraphic,
    filter: &ResolvedFilter,
    compositing: FilterCompositing,
    filter_dpi: f32,
) -> Option<(FilterRasterOutput, surface::SourceGeometry)> {
    if !filter.requires_source_surface() {
        return None;
    }
    let filter_geometry = source.geometry.filter_geometry()?;
    let filtered = crate::render::filter::apply_operations_to_surface(
        &source.pixels,
        filter_geometry,
        &filter.operations,
        filter.linear_rgb,
        filter.svg_region,
        filter_dpi,
    )?;
    let filter_bounds = filtered.bounds;
    let frame = raster_frame::FilterRasterFrame::new(
        filtered.pixels,
        source.geometry.paint_overflow() + filter_bounds.raster_overflow,
    );
    let retain_complete_region = filter.svg_region.is_some()
        && filter
            .operations
            .iter()
            .any(FilterOperation::requires_complete_svg_region);
    let frame = if retain_complete_region {
        frame
    } else {
        frame.subset_to_paint_bounds(
            source.paint_bounds,
            filter_bounds.raster_overflow,
            filter_bounds.effect_support,
            filter_dpi,
        )
    };
    let frame = match compositing.output_clip {
        FilterOutputClip::None => frame,
        FilterOutputClip::BorderBox => {
            frame.subset_to_border_box(source.geometry.layout.size, filter_dpi)
        }
    };
    let layout_geometry = source.geometry.layout;
    Some((
        FilterRasterOutput {
            asset: crate::render::blur::rgba_to_png_alpha_asset(frame.pixels, filter_dpi)?,
            raster_overflow: frame.overflow,
        },
        layout_geometry,
    ))
}
