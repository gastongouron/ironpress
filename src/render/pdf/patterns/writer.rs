use super::{
    PdfFunctionPattern, PdfPatternEntry, PdfShadingPattern, PdfTilingPattern,
    PdfTilingPatternEntry, PdfTilingPatternTarget,
};
use crate::render::pdf::geometry::{PdfRect, PdfVector};
use crate::render::pdf::{ImageRef, PdfResourceUsage, PdfWriter};
use crate::render::pdf_syntax::{format_pdf_number, format_pdf_number_fixed};
use crate::render::shading::build_shading_function;

impl PdfWriter {
    /// Wrap one masked solid in a page-space pattern cell. PDF soft-mask
    /// coverage at the painted box edge depends on the cell and its consumer
    /// being composited as one transparency primitive; applying the mask
    /// directly to the consumer path produces a second, observably different
    /// edge rasterization.
    pub(in crate::render::pdf) fn add_masked_solid_page_pattern(
        &mut self,
        paint_box: PdfRect,
        mask: &str,
        color: (f32, f32, f32),
    ) -> Option<String> {
        const CELL_GAP: f32 = 2.0;

        let cell_box = self
            .page_content_transform
            .page_bounds()
            .unwrap_or(paint_box);
        let color_pattern = self.add_premultiplied_solid_pattern(cell_box, color)?;
        let stream = format!(
            "/{mask} gs\n/Pattern CS/Pattern cs\n/{color_pattern} SCN/{color_pattern} scn\n{}f*\n",
            cell_box.rect_path(),
        );
        self.add_page_tiling_pattern(
            stream,
            PdfTilingPattern {
                bbox: cell_box,
                paint_box: cell_box,
                step: PdfVector::new(cell_box.width + CELL_GAP, cell_box.height + CELL_GAP),
                ..Default::default()
            },
        )
    }

    /// Emit the function-based color half of a premultiplied transparent-to-
    /// solid gradient. The companion luminosity mask carries alpha; this
    /// calculator reconstructs straight RGB from the same premultiplied ramp,
    /// matching the PDF transparency model at every sample.
    fn add_premultiplied_solid_pattern(
        &mut self,
        page: PdfRect,
        color: (f32, f32, f32),
    ) -> Option<String> {
        if page.is_empty() {
            return None;
        }
        let (red, green, blue) = color;
        let exact = [red, green, blue].map(|value| format_pdf_number_fixed(f64::from(value), 8));
        let terminal = [red, green, blue].map(|value| format_pdf_number_fixed(f64::from(value), 4));
        let calculator = format!(
            "{{pop\n\
dup 0 le {{pop 0 0 0 0 0}} if\n\
dup dup 0 gt exch 1 le and {{\n\
0 sub dup {} mul exch dup {} mul exch dup {} mul exch \n\
0}} if\n\
0 gt {{{} {} {} 1}} if\n\
dup abs 0.00001 lt{{ pop pop pop pop 0 0 0 }}{{ dup 3 1 roll div 4 1 roll dup 3 1 roll div 4 1 roll div 3 1 roll}} ifelse\n\
}}\n",
            exact[0], exact[1], exact[2], terminal[0], terminal[1], terminal[2],
        );
        let domain_y = format_pdf_number_fixed(f64::from(page.height / page.width), 8);
        let function_id = self.next_id();
        self.objects.push(format!(
            "{function_id} 0 obj\n<< /FunctionType 4 /Domain [0 1 0 {domain_y}] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n",
            calculator.len(),
        ));
        self.binary_objects
            .insert(function_id, calculator.into_bytes());

        let object_id = self.next_id();
        let name = format!("PSh{}", self.pdf_patterns.len());
        self.objects.push(format!(
            "{object_id} 0 obj\n<< /Type /Pattern /PatternType 2 /Matrix [{width} 0 0 -{width} {left} {top}] /Shading << /Domain [0 1 0 {domain_y}] /Function {function_id} 0 R /ShadingType 1 /ColorSpace /DeviceRGB >> >>\nendobj",
            width = page.width,
            left = page.left,
            top = page.top(),
        ));
        self.pdf_patterns.push(PdfPatternEntry {
            name: name.clone(),
            object_id,
        });
        Some(name)
    }

    pub(in crate::render::pdf) fn add_shading_pattern(
        &mut self,
        pattern: PdfShadingPattern,
    ) -> String {
        let (kind, coordinates, coordinate_count, transform, stops, geometry_format) =
            pattern.into_pdf_parts();
        let object_id = self.next_id();
        let name = format!("PSh{}", self.pdf_patterns.len());
        let coordinates = coordinates[..coordinate_count]
            .iter()
            .copied()
            .map(|value| geometry_format.number(value))
            .collect::<Vec<_>>()
            .join(" ");
        let matrix = transform
            .components()
            .into_iter()
            .map(|value| geometry_format.number(value))
            .collect::<Vec<_>>()
            .join(" ");
        let function = build_shading_function(&stops);
        self.objects.push(format!(
            "{object_id} 0 obj\n<< /Type /Pattern /PatternType 2 /Matrix [{matrix}] /Shading << /ShadingType {} /ColorSpace /DeviceRGB /Coords [{coordinates}] /Function {function} /Extend [true true] >> >>\nendobj",
            kind.pdf_type(),
        ));
        self.pdf_patterns.push(PdfPatternEntry {
            name: name.clone(),
            object_id,
        });
        name
    }

    pub(in crate::render::pdf) fn add_function_pattern(
        &mut self,
        pattern: PdfFunctionPattern,
    ) -> String {
        let (transform, domain, calculator) = pattern.into_parts();
        let function_id = self.next_id();
        self.objects.push(format!(
            "{function_id} 0 obj\n<< /FunctionType 4 /Domain [{}] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n",
            domain
                .xy_domain()
                .into_iter()
                .map(format_pdf_number)
                .collect::<Vec<_>>()
                .join(" "),
            calculator.len(),
        ));
        self.binary_objects
            .insert(function_id, calculator.into_bytes());

        let object_id = self.next_id();
        let name = format!("PSh{}", self.pdf_patterns.len());
        let matrix = transform
            .components()
            .into_iter()
            .map(format_pdf_number)
            .collect::<Vec<_>>()
            .join(" ");
        let domain = domain
            .xy_domain()
            .into_iter()
            .map(format_pdf_number)
            .collect::<Vec<_>>()
            .join(" ");
        self.objects.push(format!(
            "{object_id} 0 obj\n<< /Type /Pattern /PatternType 2 /Matrix [{matrix}] /Shading << /Domain [{domain}] /Function {function_id} 0 R /ShadingType 1 /ColorSpace /DeviceRGB >> >>\nendobj",
        ));
        self.pdf_patterns.push(PdfPatternEntry {
            name: name.clone(),
            object_id,
        });
        name
    }

    pub(in crate::render::pdf) fn add_tiling_pattern(
        &mut self,
        stream: String,
        pattern: PdfTilingPattern,
    ) -> Option<ImageRef> {
        let (pattern_id, resources) = self.add_tiling_pattern_object(stream, pattern)?;
        let form_id = self.next_id();
        let form_name = format!("Fm{form_id}");
        let form_stream = format!(
            "q\n/Pattern cs\n/Cell scn\n0 0 {width} {height} re\nf\nQ\n",
            width = pattern.paint_box.width,
            height = pattern.paint_box.height,
        );
        self.objects.push(format!(
            "{form_id} 0 obj\n<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {width} {height}] /Resources << /Pattern << /Cell {pattern_id} 0 R >> >> /Length {len} >>\nstream\n",
            width = pattern.paint_box.width,
            height = pattern.paint_box.height,
            len = form_stream.len(),
        ));
        self.binary_objects
            .insert(form_id, form_stream.into_bytes());
        self.tiling_patterns.push(PdfTilingPatternEntry {
            pattern_id,
            target: PdfTilingPatternTarget::Form { object_id: form_id },
            resources,
        });
        Some(ImageRef {
            name: form_name,
            obj_id: form_id,
        })
    }

    pub(in crate::render::pdf) fn add_page_tiling_pattern(
        &mut self,
        stream: String,
        pattern: PdfTilingPattern,
    ) -> Option<String> {
        let (pattern_id, resources) = self.add_tiling_pattern_object(stream, pattern)?;
        let name = format!("PT{}", self.tiling_patterns.len());
        self.tiling_patterns.push(PdfTilingPatternEntry {
            pattern_id,
            target: PdfTilingPatternTarget::Page { name: name.clone() },
            resources,
        });
        Some(name)
    }

    fn add_tiling_pattern_object(
        &mut self,
        stream: String,
        pattern: PdfTilingPattern,
    ) -> Option<(usize, PdfResourceUsage)> {
        if pattern.bbox.is_empty()
            || pattern.paint_box.is_empty()
            || !pattern.step.is_positive()
            || !pattern.transform.is_invertible()
        {
            return None;
        }
        let PdfVector {
            x: step_x,
            y: step_y,
        } = pattern.step;
        let matrix = pattern.matrix_dictionary_entry();
        let resources = PdfResourceUsage::from_stream(&stream);
        self.note_standard_fonts(&resources);
        let pattern_id = self.next_id();
        let bytes = stream.into_bytes();
        self.objects.push(format!(
            "{pattern_id} 0 obj\n<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [{left} {bottom} {right} {top}] /XStep {step_x} /YStep {step_y}{matrix} /Resources __IP_PATTERN_CELL_RESOURCES__ /Length {len} >>\nstream\n",
            left = pattern.bbox.left,
            bottom = pattern.bbox.bottom,
            right = pattern.bbox.right(),
            top = pattern.bbox.top(),
            matrix = matrix,
            len = bytes.len(),
        ));
        self.binary_objects.insert(pattern_id, bytes);
        Some((pattern_id, resources))
    }
}
