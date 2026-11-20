use super::*;

#[derive(Clone, Copy)]
pub(crate) struct RenderOpts {
    pub compress: bool,
    pub jpeg_quality: u8,
    pub auto_resize_images: bool,
    /// One conversion-owned policy for all raster fallbacks. Keeping this
    /// grouped prevents render paths from silently choosing their own density.
    pub raster_quality: crate::style::raster_quality::RasterQuality,
    /// Skip embedding raster images fully covered by a later fully-opaque
    /// rectangular element (default false). Conservative; zero visual change.
    pub occlusion_cull: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            compress: true,
            jpeg_quality: DEFAULT_JPEG_QUALITY,
            auto_resize_images: true,
            raster_quality: crate::style::raster_quality::RasterQuality::default(),
            occlusion_cull: false,
        }
    }
}

/// Minimal PDF writer that produces valid PDF files.
#[derive(Default)]
pub(crate) struct PdfWriter {
    pub(super) resources: crate::security::resources::ResourceLoader,
    pub(super) objects: Vec<String>,
    /// Raw binary objects stored separately (index corresponds to objects slot).
    pub(super) binary_objects: std::collections::HashMap<usize, Vec<u8>>,
    /// Document images keyed by source identity and final display sampling.
    pub(super) layout_image_objects:
        std::collections::HashMap<super::writer_images::LayoutImageCacheKey, usize>,
    pub(super) page_ids: Vec<usize>,
    /// Annotation object IDs grouped by page index.
    pub(super) page_annotations: Vec<Vec<usize>>,
    /// Image references grouped by page index.
    pub(super) page_images: Vec<Vec<ImageRef>>,
    /// ExtGState entries (name, opacity) grouped by page index.
    pub(super) page_ext_gstates: Vec<Vec<(String, f32)>>,
    /// Shading dictionary entries grouped by page index.
    pub(super) page_shadings: Vec<Vec<ShadingEntry>>,
    /// Function-based conic shadings are document-global resources, like the shared
    /// page resource dictionary that names them.
    pub(super) conic_shadings: Vec<ConicShadingEntry>,
    /// Custom TrueType font entries.
    pub(super) custom_font_entries: Vec<CustomFontEntry>,
    /// CSS `mask-image` transparency-group masks. Names are global because PDF
    /// page resource dictionaries share the entries.
    pub(super) soft_mask_gstates: Vec<PdfSoftMaskGState>,
    /// Repeated-background pattern cells and their page-visible local forms.
    pub(super) tiling_patterns: Vec<PdfTilingPatternEntry>,
    /// Direct CSS gradients whose Pattern matrix lives in default page space.
    pub(super) pdf_patterns: Vec<PdfPatternEntry>,
    /// Page-visible Forms whose content needs an exact, acyclic resource map.
    pub(super) local_forms: Vec<PdfLocalFormEntry>,
    pub(super) svg_defs: crate::parser::svg::SvgDefs,
    pub(super) opts: RenderOpts,
    pub(super) page_content_transform: PageContentTransform,
    pub(super) graphics_state: PdfGraphicsState,
    /// Marked-content recording for accessible output; disabled by default.
    pub(crate) structure: super::structure::StructureRecorder,
    /// Targets of `#id` links, resolved in accessible output only.
    pub(super) destinations: super::destinations::NamedDestinations,
    /// The standard fonts the content selects; accessible output lists only
    /// those, historical output always lists all of them.
    pub(super) standard_fonts_in_use: std::collections::BTreeSet<&'static str>,
}

/// Renderer-owned subset of the PDF graphics state which cannot be recovered
/// from a resource after that resource has been registered.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PdfGraphicsState {
    /// CSS paint-space transform from local layout coordinates into page layout
    /// space. PDF shading-pattern matrices are defined in default user space,
    /// so they must explicitly inherit this matrix from every ancestor.
    local_to_layout: PdfMatrix,
    box_paint_grid: BoxPaintGrid,
}

impl PdfWriter {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Note the standard fonts a stream selects.
    pub(super) fn note_standard_fonts(&mut self, usage: &PdfResourceUsage) {
        self.standard_fonts_in_use
            .extend(usage.standard_fonts.iter().copied());
    }

    pub(super) fn with_resources(resources: crate::security::resources::ResourceLoader) -> Self {
        Self {
            resources,
            ..Self::default()
        }
    }

    pub(crate) fn load_document_resource(
        &mut self,
        source: &str,
    ) -> Option<crate::security::resources::LoadedResource> {
        self.resources.load_document_resource(source)
    }

    pub(super) fn resolve_background_svg(
        &mut self,
        background: &crate::layout::helpers::BackgroundFields,
    ) -> Option<crate::parser::svg::SvgTree> {
        background.svg.clone().or_else(|| {
            background.raster_source.as_deref().and_then(|source| {
                crate::layout::images::build_raster_background_tree(&mut self.resources, source)
            })
        })
    }

    pub(super) fn paint_matrix(&self, local: PdfMatrix) -> PdfMatrix {
        self.graphics_state.local_to_layout * local
    }

    pub(super) fn has_active_paint_transform(&self) -> bool {
        self.graphics_state.local_to_layout != PdfMatrix::IDENTITY
    }

    /// Select the browser-like serialization space for the current page.
    ///
    /// CSS transforms operate on the same top-down CSS coordinates as the
    /// content they transform. Keep that coordinate system inside an active
    /// transform as well; falling back to bottom-up points would reverse the
    /// box's block axis before the transform is applied.
    pub(super) fn current_content_space(&self) -> PdfContentSpace {
        PdfContentSpace::page_css(self.page_content_transform)
    }

    pub(super) fn resolve_box_geometry(&self, layout: LayoutBoxGeometry) -> BoxPaintGeometry {
        layout.for_paint(
            self.page_content_transform,
            self.graphics_state.box_paint_grid,
        )
    }

    /// Root an outer atomic inline subtree at its authored layout position.
    ///
    /// Nested atomic boxes inherit the existing grid: the subtree is one paint
    /// unit in the parent inline formatting context, not a sequence of
    /// independently shifted descendants.
    pub(super) fn enter_atomic_inline_paint_grid(
        &mut self,
        origin: PdfPoint,
    ) -> Option<BoxPaintGrid> {
        matches!(self.graphics_state.box_paint_grid, BoxPaintGrid::Page).then(|| {
            std::mem::replace(
                &mut self.graphics_state.box_paint_grid,
                BoxPaintGrid::AtomicInline(origin),
            )
        })
    }

    pub(super) fn restore_box_paint_grid(&mut self, grid: BoxPaintGrid) {
        self.graphics_state.box_paint_grid = grid;
    }

    pub(super) fn transformed_paint_space(&self, page_bounds: PdfRect) -> Option<PdfPaintSpace> {
        let local_to_layout = self.graphics_state.local_to_layout;
        (local_to_layout != PdfMatrix::IDENTITY)
            .then(|| PdfPaintSpace::new(local_to_layout, self.page_content_transform, page_bounds))
    }

    pub(super) fn enter_paint_transform(&mut self, local: PdfMatrix) -> PdfMatrix {
        let prior = self.graphics_state.local_to_layout;
        self.graphics_state.local_to_layout = prior * local;
        prior
    }

    pub(super) fn restore_paint_transform(&mut self, prior: PdfMatrix) {
        self.graphics_state.local_to_layout = prior;
    }

    pub(super) fn next_id(&self) -> usize {
        self.objects.len() + 1
    }

    pub(crate) fn image_dimensions(&self, object_id: usize) -> Option<RasterDimensions> {
        let object = self.objects.get(object_id.checked_sub(1)?)?;
        let value_after = |name| {
            object
                .split_ascii_whitespace()
                .zip(object.split_ascii_whitespace().skip(1))
                .find_map(|(key, value)| (key == name).then(|| value.parse().ok()).flatten())
        };
        Some(RasterDimensions {
            width: value_after("/Width")?,
            height: value_after("/Height")?,
        })
    }

    pub(super) fn add_conic_shading(
        &mut self,
        domain: PdfRect,
        function: ConicShadingFunction,
    ) -> String {
        let name = format!("ShConic{}", self.conic_shadings.len());
        self.conic_shadings.push(ConicShadingEntry {
            name: name.clone(),
            domain,
            function,
        });
        name
    }

    pub(super) fn add_transparency_group_form(
        &mut self,
        stream: String,
        paint_box: PdfRect,
    ) -> ImageRef {
        self.add_local_form(
            stream,
            paint_box,
            Some("/Group << /Type /Group /S /Transparency /CS /DeviceRGB /I true /K false >>"),
        )
    }

    pub(super) fn register_soft_mask(&mut self, name: String, form_id: usize) {
        self.soft_mask_gstates.push(PdfSoftMaskGState {
            name,
            form_id,
            mode: PdfSoftMaskMode::Luminosity,
        });
    }

    /// Register a transparency form whose composited alpha is the mask.
    ///
    /// Layered CSS masks must retain alpha while their sources are composed;
    /// reducing the final group to luminosity would lose that compositing
    /// result for overlapping translucent coverage.
    pub(super) fn register_alpha_soft_mask(&mut self, name: String, form_id: usize) {
        self.soft_mask_gstates.push(PdfSoftMaskGState {
            name,
            form_id,
            mode: PdfSoftMaskMode::Alpha,
        });
    }

    pub(super) fn add_plain_local_form(&mut self, stream: String, paint_box: PdfRect) -> ImageRef {
        self.add_local_form(stream, paint_box, None)
    }

    fn add_local_form(
        &mut self,
        stream: String,
        paint_box: PdfRect,
        group: Option<&str>,
    ) -> ImageRef {
        let form_id = self.next_id();
        let [x, x1, y, y1] = paint_box.xy_domain();
        let resources = PdfResourceUsage::from_stream(&stream);
        self.note_standard_fonts(&resources);
        let bytes = stream.into_bytes();
        let group = group.map_or_else(String::new, |group| format!(" {group}"));
        self.objects.push(format!(
            "{form_id} 0 obj\n<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{x} {y} {x1} {y1}]{group} /Resources __IP_LOCAL_FORM_RESOURCES__ /Length {len} >>\nstream\n",
            len = bytes.len(),
        ));
        self.binary_objects.insert(form_id, bytes);
        self.local_forms
            .push(PdfLocalFormEntry { form_id, resources });
        ImageRef {
            name: format!("Fm{form_id}"),
            obj_id: form_id,
        }
    }
}

/// The PDF soft-mask channel extracted from its transparency group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PdfSoftMaskMode {
    #[default]
    Luminosity,
    Alpha,
}

impl PdfSoftMaskMode {
    pub(super) const fn pdf_name(self) -> &'static str {
        match self {
            Self::Luminosity => "/Luminosity",
            Self::Alpha => "/Alpha",
        }
    }
}

/// One named PDF transparency-group mask.
pub(super) struct PdfSoftMaskGState {
    pub(super) name: String,
    pub(super) form_id: usize,
    pub(super) mode: PdfSoftMaskMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_paint_scopes_compose_and_restore_resource_matrices() {
        let mut writer = PdfWriter::new();
        let parent = PdfMatrix::rotate_around(PdfPoint::new(40.0, 30.0), 0.5, 0.866_025_4);
        let child = PdfMatrix::translate(PdfPoint::new(7.0, -3.0));
        let resource = PdfMatrix::scale(PdfVector::new(0.75, -0.75));

        let root = writer.enter_paint_transform(parent);
        let parent_state = writer.enter_paint_transform(child);
        assert_eq!(writer.paint_matrix(resource), parent * child * resource);

        writer.restore_paint_transform(parent_state);
        assert_eq!(writer.paint_matrix(resource), parent * resource);

        writer.restore_paint_transform(root);
        assert_eq!(writer.paint_matrix(resource), resource);
    }
}
