use super::*;

#[derive(Clone, Copy)]
pub(super) struct PagePaintStreams<'a> {
    document: &'a str,
    decorations: Option<&'a str>,
}

impl<'a> PagePaintStreams<'a> {
    pub(super) fn document_only(document: &'a str) -> Self {
        Self {
            document,
            decorations: None,
        }
    }

    pub(super) fn with_decorations(document: &'a str, decorations: &'a str) -> Self {
        Self {
            document,
            decorations: (!decorations.is_empty()).then_some(decorations),
        }
    }
}

impl PdfWriter {
    /// Embed a TrueType font and return the PDF resource name to reference it.
    pub(super) fn add_ttf_font(
        &mut self,
        name: &str,
        ttf: &TtfFont,
        prepared_font: &PreparedCustomFont,
    ) -> String {
        if prepared_font.uses_type3_embedding() {
            return self.add_type3_font(name, ttf, prepared_font);
        }
        let resource_name = sanitize_pdf_name(name);
        let base_font_name = &prepared_font.base_font_name;
        let cff_outlines = sfnt_has_cff_outlines(&prepared_font.font_data);
        let font_file_key = if cff_outlines {
            "FontFile3"
        } else {
            "FontFile2"
        };
        let font_stream_subtype = if cff_outlines {
            " /Subtype /OpenType"
        } else {
            ""
        };
        let cid_font_subtype = if cff_outlines {
            "CIDFontType0"
        } else {
            "CIDFontType2"
        };
        let cid_to_gid_map = if cff_outlines {
            ""
        } else {
            " /CIDToGIDMap /Identity"
        };

        // 1. Font stream: embed the prepared font data and compress the stream
        // to avoid paying the full raw TTF size in the PDF.
        let stream_id = self.next_id();
        let compressed_data = flate_compress(&prepared_font.font_data);
        let header = if let Some(ref compressed_data) = compressed_data {
            format!(
                "{stream_id} 0 obj\n<<{font_stream_subtype} /Filter /FlateDecode /Length {} /Length1 {} >>\nstream\n",
                compressed_data.len(),
                prepared_font.font_data.len(),
            )
        } else {
            format!(
                "{stream_id} 0 obj\n<<{font_stream_subtype} /Length {} /Length1 {} >>\nstream\n",
                prepared_font.font_data.len(),
                prepared_font.font_data.len(),
            )
        };
        self.objects.push(header);
        self.binary_objects.insert(
            stream_id,
            compressed_data.unwrap_or_else(|| prepared_font.font_data.clone()),
        );

        // 2. FontDescriptor
        let descriptor_id = self.next_id();
        let pdf_metrics = ttf.pdf_vertical_metrics();
        let ascent_pdf = (pdf_metrics.ascent as i32 * 1000) / ttf.units_per_em as i32;
        let descent_pdf = (pdf_metrics.descent as i32 * 1000) / ttf.units_per_em as i32;
        let bbox_pdf = [
            (ttf.bbox[0] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[1] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[2] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[3] as i32 * 1000) / ttf.units_per_em as i32,
        ];
        self.objects.push(format!(
            "{descriptor_id} 0 obj\n<< /Type /FontDescriptor /FontName /{base_font_name} /Flags {flags} /FontBBox [{b0} {b1} {b2} {b3}] /Ascent {ascent} /Descent {descent} /ItalicAngle 0 /CapHeight {ascent} /StemV 80 /{font_file_key} {stream_id} 0 R >>\nendobj",
            flags = ttf.flags,
            b0 = bbox_pdf[0],
            b1 = bbox_pdf[1],
            b2 = bbox_pdf[2],
            b3 = bbox_pdf[3],
            ascent = ascent_pdf,
            descent = descent_pdf,
        ));

        // 3. CID widths array keyed by glyph ID so shaped glyph IDs can be
        // emitted directly with Identity-H.
        let widths_str = serialize_cid_widths(&prepared_font.widths);

        // 4. CID descendant font object
        let cid_font_id = self.next_id();
        self.objects.push(format!(
            "{cid_font_id} 0 obj\n<< /Type /Font /Subtype /{cid_font_subtype} /BaseFont /{base_font_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor_id} 0 R{cid_to_gid_map} /W [0 [{widths_str}]] >>\nendobj",
        ));

        // 5. ToUnicode CMap so text stays searchable/selectable.
        let to_unicode_id = self.next_id();
        let to_unicode = build_tounicode_cmap(&prepared_font.to_unicode_map);
        self.objects.push(format!(
            "{to_unicode_id} 0 obj\n<< /Length {} >>\nstream\n{to_unicode}endstream\nendobj",
            to_unicode.len(),
        ));

        // 6. Type0 wrapper font object
        let font_id = self.next_id();
        self.objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /{base_font_name} /Encoding /Identity-H /DescendantFonts [{cid_font_id} 0 R] /ToUnicode {to_unicode_id} 0 R >>\nendobj",
        ));

        self.custom_font_entries.push(CustomFontEntry {
            resource_name: resource_name.clone(),
            font_obj_id: font_id,
        });

        resource_name
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_page(
        &mut self,
        width: f32,
        height: f32,
        paint: PagePaintStreams<'_>,
        annotations: Vec<LinkAnnotation>,
        images: Vec<ImageRef>,
        ext_gstates: Vec<(String, f32)>,
        shadings: Vec<ShadingEntry>,
    ) {
        let document_id = self.add_page_content_stream(paint.document);
        let page_contents = match paint.decorations {
            Some(decorations) => {
                let decorations_id = self.add_page_content_stream(decorations);
                format!("[{document_id} 0 R {decorations_id} 0 R]")
            }
            None => format!("{document_id} 0 R"),
        };
        let page_id = self.objects.len() + annotations.len() + 1;

        // Annotation objects
        let mut annot_ids = Vec::new();
        for annot in &annotations {
            let annot_id = self.next_id();
            self.objects.push(format!(
                "{annot_id} 0 obj\n<< /Type /Annot /Subtype /Link /P {page_id} 0 R /Rect [{x1} {y1} {x2} {y2}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({uri}) >> >>\nendobj",
                page_id = page_id,
                x1 = annot.rect.left,
                y1 = annot.rect.bottom,
                x2 = annot.rect.right(),
                y2 = annot.rect.top(),
                uri = escape_pdf_string(&annot.url),
            ));
            annot_ids.push(annot_id);
        }

        // Page object (placeholder — will be updated in finish())
        self.objects.push(format!(
            "{page_id} 0 obj\n<< /Type /Page /MediaBox [0 0 {width} {height}] /Contents {page_contents} >>\nendobj",
        ));

        self.page_ids.push(page_id);
        self.page_annotations.push(annot_ids);
        self.page_images.push(images);
        self.page_ext_gstates.push(ext_gstates);
        self.page_shadings.push(shadings);
    }

    fn add_page_content_stream(&mut self, content: &str) -> usize {
        let content_id = self.next_id();
        match self
            .opts
            .compress
            .then(|| flate_compress(content.as_bytes()))
            .flatten()
        {
            Some(compressed) => {
                self.objects.push(format!(
                    "{content_id} 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n",
                    compressed.len(),
                ));
                self.binary_objects.insert(content_id, compressed);
            }
            None => {
                self.objects.push(format!(
                    "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj",
                    content.len(),
                ));
            }
        }
        content_id
    }

    pub(super) fn finish_to_writer<W: std::io::Write>(
        self,
        out: &mut W,
        bookmarks: &[BookmarkEntry],
    ) -> Result<(), IronpressError> {
        let mut bytes_written: usize = 0;
        out.write_all(b"%PDF-1.4\n")?;
        bytes_written += b"%PDF-1.4\n".len();

        // Font objects
        let font_base_id = self.objects.len() + 1;
        let font_names = [
            // Helvetica (sans-serif)
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            // Times Roman (serif)
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            // Courier (monospace)
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
            // Symbol (math/Greek)
            "Symbol",
        ];

        // `finish_to_writer` owns the writer, so retain that ownership all the
        // way through final assembly instead of duplicating every textual PDF
        // object. The remaining fields can still be read after this partial
        // move, and object order/IDs are unchanged.
        let mut all_objects = self.objects;

        for (i, name) in font_names.iter().enumerate() {
            let id = font_base_id + i;
            if name == &"Symbol" {
                // Symbol font uses its own built-in encoding, not WinAnsiEncoding
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>\nendobj",
                ));
            } else {
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>\nendobj",
                ));
            }
        }

        // Font dictionary (standard + custom fonts)
        let font_dict_id = font_base_id + font_names.len();
        let mut font_entries: Vec<String> = font_names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} {} 0 R", font_base_id + i))
            .collect();
        // Add custom font entries
        for entry in &self.custom_font_entries {
            font_entries.push(format!(
                "/{} {} 0 R",
                entry.resource_name, entry.font_obj_id
            ));
        }
        let font_entries_str = font_entries.join(" ");
        all_objects.push(format!(
            "{font_dict_id} 0 obj\n<< {font_entries_str} >>\nendobj",
        ));

        // Collect all image object IDs used across all pages
        let mut all_image_refs: Vec<(String, usize)> = Vec::new();
        for page_imgs in &self.page_images {
            for img in page_imgs {
                if !all_image_refs.iter().any(|(_, id)| *id == img.obj_id) {
                    all_image_refs.push((img.name.clone(), img.obj_id));
                }
            }
        }

        // Collect unique ExtGState entries across all pages
        let mut gs_entries: Vec<(String, f32)> = Vec::new();
        for page_gs in &self.page_ext_gstates {
            for (name, opacity) in page_gs {
                if !gs_entries.iter().any(|(n, _)| n == name) {
                    gs_entries.push((name.clone(), *opacity));
                }
            }
        }
        gs_entries.extend(
            self.conic_shadings
                .iter()
                .filter(|entry| entry.function.opacity < 1.0)
                .map(|entry| (format!("GS{}", entry.name), entry.function.opacity)),
        );
        let has_opacity = !gs_entries.is_empty();
        // CSS `mask-image` soft-mask graphics states (css-masking-1 §3) — emitted
        // alongside the opacity/blend gstates into the shared resource dict.
        let has_soft_masks = !self.soft_mask_gstates.is_empty();
        let has_gstates = has_opacity || has_soft_masks;

        // Add ExtGState objects if needed
        let mut gs_obj_refs: Vec<(String, usize)> = Vec::new();
        if has_gstates {
            // GSDefault (opacity 1.0)
            let default_gs_id = all_objects.len() + 1;
            all_objects.push(format!(
                "{default_gs_id} 0 obj\n<< /Type /ExtGState /ca 1 /CA 1 >>\nendobj"
            ));
            gs_obj_refs.push(("GSDefault".to_string(), default_gs_id));

            // Per-element ExtGState objects. Names prefixed `GSbm` carry a blend
            // mode (e.g. `GSbmMultiply` → `/BM /Multiply`); all others are alpha
            // (`/ca` / `/CA`) gstates whose float value is the opacity.
            for (name, opacity) in &gs_entries {
                let gs_id = all_objects.len() + 1;
                let body = match name.strip_prefix("GSbm") {
                    Some(mode) => format!("/Type /ExtGState /BM /{mode}"),
                    None => {
                        let opacity = format_pdf_number(*opacity);
                        format!("/Type /ExtGState /ca {opacity} /CA {opacity}")
                    }
                };
                all_objects.push(format!("{gs_id} 0 obj\n<< {body} >>\nendobj"));
                gs_obj_refs.push((name.clone(), gs_id));
            }

            // Soft-mask gstates: the form already exists, so its object id is
            // stable while the shared resource dictionary is assembled.
            for mask in &self.soft_mask_gstates {
                let gs_id = all_objects.len() + 1;
                all_objects.push(format!(
                    "{gs_id} 0 obj\n<< /Type /ExtGState /SMask << /Type /Mask /S {} /G {} 0 R >> >>\nendobj",
                    mask.mode.pdf_name(),
                    mask.form_id,
                ));
                gs_obj_refs.push((mask.name.clone(), gs_id));
            }
        }

        // Add Shading objects
        let mut shading_obj_refs: Vec<(String, usize)> = Vec::new();
        for page_sh in &self.page_shadings {
            for entry in page_sh {
                let sh_id = all_objects.len() + 1;
                let function_str = build_shading_function(&entry.stops);
                let coords_str = if entry.kind == PdfShadingKind::Axial {
                    // Axial: only first 4 coords
                    format!(
                        "{} {} {} {}",
                        entry.coords[0], entry.coords[1], entry.coords[2], entry.coords[3]
                    )
                } else {
                    // Radial: all 6 coords
                    format!(
                        "{} {} {} {} {} {}",
                        entry.coords[0],
                        entry.coords[1],
                        entry.coords[2],
                        entry.coords[3],
                        entry.coords[4],
                        entry.coords[5]
                    )
                };
                all_objects.push(format!(
                    "{sh_id} 0 obj\n<< /ShadingType {} /ColorSpace /DeviceRGB /Coords [{coords_str}] /Function {function_str} /Extend [true true] >>\nendobj",
                    entry.kind.pdf_type(),
                ));
                shading_obj_refs.push((entry.name.clone(), sh_id));
            }
        }
        for entry in &self.conic_shadings {
            let domain = entry.domain.xy_domain().map(format_pdf_number).join(" ");
            let function_id = all_objects.len() + 1;
            let stream = format!("{}\n", entry.function.calculator);
            all_objects.push(format!(
                "{function_id} 0 obj\n<< /FunctionType 4 /Domain [{domain}] /Range [0 1 0 1 0 1] /Length {length} >>\nstream\n{stream}endstream\nendobj",
                length = stream.len(),
            ));
            let shading_id = all_objects.len() + 1;
            all_objects.push(format!(
                "{shading_id} 0 obj\n<< /ShadingType 1 /ColorSpace /DeviceRGB /Domain [{domain}] /Function {function_id} 0 R >>\nendobj"
            ));
            shading_obj_refs.push((entry.name.clone(), shading_id));
        }

        let mut pattern_obj_refs = self
            .pdf_patterns
            .iter()
            .map(|entry| (entry.name.clone(), entry.object_id))
            .collect::<Vec<_>>();
        pattern_obj_refs.extend(self.tiling_patterns.iter().filter_map(
            |entry| match &entry.target {
                PdfTilingPatternTarget::Page { name } => Some((name.clone(), entry.pattern_id)),
                PdfTilingPatternTarget::Form { .. } => None,
            },
        ));

        // Local Forms must not inherit the page resource dictionary: the page's
        // /XObject map names those same Forms, which would create a self-cycle.
        // Resolve only the resources actually named by each Form stream.
        for entry in &self.local_forms {
            let resources = entry.resources.dictionary(
                font_dict_id,
                &all_image_refs,
                &gs_obj_refs,
                &shading_obj_refs,
                &pattern_obj_refs,
                Some(entry.form_id),
            )?;
            let form_object = &mut all_objects[entry.form_id - 1];
            *form_object = form_object.replace("__IP_LOCAL_FORM_RESOURCES__", &resources);
        }

        // Pattern cells get only the resources named by their own streams.
        // Form-contained patterns reject a reference back to that form; direct
        // page patterns have no containing XObject to exclude.
        for entry in &self.tiling_patterns {
            let forbidden_form = match entry.target {
                PdfTilingPatternTarget::Form { object_id } => Some(object_id),
                PdfTilingPatternTarget::Page { .. } => None,
            };
            let resources = entry.resources.dictionary(
                font_dict_id,
                &all_image_refs,
                &gs_obj_refs,
                &shading_obj_refs,
                &pattern_obj_refs,
                forbidden_form,
            )?;
            let pattern_object = &mut all_objects[entry.pattern_id - 1];
            *pattern_object = pattern_object.replace("__IP_PATTERN_CELL_RESOURCES__", &resources);
        }

        // Resources dictionary
        let resources_id = all_objects.len() + 1;
        let mut resource_parts = format!("/Font {font_dict_id} 0 R");

        if !all_image_refs.is_empty() {
            let xobj_entries: String = all_image_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /XObject << {xobj_entries} >>"));
        }

        if has_gstates {
            let gs_dict: String = gs_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /ExtGState << {gs_dict} >>"));
        }

        if !shading_obj_refs.is_empty() {
            let shading_dict: String = shading_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /Shading << {shading_dict} >>"));
        }

        if !self.pdf_patterns.is_empty()
            || self
                .tiling_patterns
                .iter()
                .any(|entry| matches!(entry.target, PdfTilingPatternTarget::Page { .. }))
        {
            let patterns = pattern_obj_refs
                .iter()
                .map(|(name, object_id)| format!("/{name} {object_id} 0 R"))
                .collect::<Vec<_>>();
            resource_parts.push_str(&format!(" /Pattern << {} >>", patterns.join(" ")));
        }

        all_objects.push(format!(
            "{resources_id} 0 obj\n<< {resource_parts} >>\nendobj",
        ));

        // Update page objects to include parent, resources, and annotations
        let pages_id = resources_id + 1;
        for (idx, &page_id) in self.page_ids.iter().enumerate() {
            let obj = &mut all_objects[page_id - 1];
            let annot_ids = &self.page_annotations[idx];
            let mut extra = format!("/Parent {pages_id} 0 R /Resources {resources_id} 0 R");
            if !annot_ids.is_empty() {
                let annots_str: String = annot_ids
                    .iter()
                    .map(|id| format!("{id} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ");
                extra.push_str(&format!(" /Annots [{annots_str}]"));
            }
            *obj = obj.replace("/Contents", &format!("{extra} /Contents"));
        }

        // Pages object
        let kids: String = self
            .page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        all_objects.push(format!(
            "{pages_id} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {} >>\nendobj",
            self.page_ids.len(),
        ));

        // Outlines (PDF bookmarks from headings)
        let outlines_ref = if bookmarks.is_empty() {
            String::new()
        } else {
            let count = bookmarks.len();
            // Outline root object
            let root_id = all_objects.len() + 1;
            let first_entry_id = root_id + 1;
            let last_entry_id = first_entry_id + count - 1;
            all_objects.push(format!(
                "{root_id} 0 obj\n<< /Type /Outlines /First {first_entry_id} 0 R /Last {last_entry_id} 0 R /Count {count} >>\nendobj",
            ));

            // Outline entry objects (flat list, linked via Prev/Next)
            for (i, bm) in bookmarks.iter().enumerate() {
                let entry_id = first_entry_id + i;
                let page_obj_id = self.page_ids.get(bm.page_index).copied().unwrap_or(1);

                let mut entry = format!(
                    "{entry_id} 0 obj\n<< /Title {title} /Parent {root_id} 0 R /Dest [{page_obj_id} 0 R /XYZ 0 {dest_y} 0]",
                    title = pdf_text_string(&bm.title),
                    dest_y = bm.y_pos,
                );
                if i > 0 {
                    entry.push_str(&format!(" /Prev {} 0 R", first_entry_id + i - 1));
                }
                if i + 1 < count {
                    entry.push_str(&format!(" /Next {} 0 R", first_entry_id + i + 1));
                }
                entry.push_str(" >>\nendobj");
                all_objects.push(entry);
            }

            format!(" /Outlines {root_id} 0 R /PageMode /UseOutlines")
        };

        // Catalog
        let catalog_id = all_objects.len() + 1;
        all_objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R{outlines_ref} >>\nendobj",
        ));

        // Write objects and track offsets for xref
        // Binary objects (images) need special handling
        let mut offsets = Vec::new();
        for (idx, obj_str) in all_objects.iter().enumerate() {
            offsets.push(bytes_written);
            let obj_id = idx + 1;
            if let Some(bin_data) = self.binary_objects.get(&obj_id) {
                // Write the header (stored in obj_str), then binary data, then endstream/endobj
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(bin_data)?;
                bytes_written += bin_data.len();
                out.write_all(b"\nendstream\nendobj\n")?;
                bytes_written += b"\nendstream\nendobj\n".len();
            } else {
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(b"\n")?;
                bytes_written += 1;
            }
        }

        // Cross-reference table
        let xref_offset = bytes_written;
        let xref_header = format!("xref\n0 {}\n", all_objects.len() + 1);
        out.write_all(xref_header.as_bytes())?;
        out.write_all(b"0000000000 65535 f \n")?;
        for offset in &offsets {
            let entry = format!("{offset:010} 00000 n \n");
            out.write_all(entry.as_bytes())?;
        }

        // Trailer
        let trailer = format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            all_objects.len() + 1,
        );
        out.write_all(trailer.as_bytes())?;

        Ok(())
    }
}

/// Serialize CID widths without losing source-font advance precision.
///
/// A Type 0 `TJ` array advances from the embedded `/W` table. Rounding that
/// table while calculating kerning corrections from the original font metrics
/// makes every unkerned glyph drift by the rounding remainder. Keep the
/// fractional text-space widths so shaping, emitted metrics, and rasterized
/// glyph origins describe the same font geometry.
fn serialize_cid_widths(widths: &[f32]) -> String {
    widths
        .iter()
        .map(|width| format_pdf_number(*width))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{PagePaintStreams, serialize_cid_widths};
    use crate::render::pdf::PdfWriter;

    #[test]
    fn cid_widths_preserve_fractional_text_space_units() {
        assert_eq!(
            serialize_cid_widths(&[0.0, 333.25, 666.75, 1_000.0]),
            "0 333.25 666.75 1000"
        );
    }

    #[test]
    fn page_decorations_are_a_separate_final_content_stream() {
        let mut writer = PdfWriter::new();
        writer.add_page(
            100.0,
            100.0,
            PagePaintStreams::with_decorations("document", "margin boxes"),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let page = writer
            .objects
            .iter()
            .find(|object| object.contains("/Type /Page "))
            .expect("page object");
        assert!(page.contains("/Contents [1 0 R 2 0 R]"), "{page}");
    }
}
