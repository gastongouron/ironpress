use super::*;

/// The standard 14 fonts the writer lists without embedding them, in the
/// order their objects are written.
pub(super) const STANDARD_FONTS: [&str; 13] = [
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

#[derive(Debug, Default)]
pub(super) struct PdfResourceUsage {
    pub(super) uses_font: bool,
    /// The standard fonts the stream selects.
    pub(super) standard_fonts: Vec<&'static str>,
    pub(super) xobjects: Vec<String>,
    pub(super) ext_gstates: Vec<String>,
    pub(super) shadings: Vec<String>,
    pub(super) patterns: Vec<String>,
}

impl PdfResourceUsage {
    pub(super) fn from_stream(stream: &str) -> Self {
        fn name(token: &str) -> Option<&str> {
            token.strip_prefix('/').filter(|name| !name.is_empty())
        }

        fn push_unique(names: &mut Vec<String>, name: &str) {
            if !names.iter().any(|existing| existing == name) {
                names.push(name.to_owned());
            }
        }

        let tokens: Vec<_> = stream.split_ascii_whitespace().collect();
        let mut resources = Self::default();
        for (index, token) in tokens.iter().enumerate() {
            match *token {
                "Do" => {
                    if let Some(name) = index.checked_sub(1).and_then(|index| name(tokens[index])) {
                        push_unique(&mut resources.xobjects, name);
                    }
                }
                "gs" => {
                    if let Some(name) = index.checked_sub(1).and_then(|index| name(tokens[index])) {
                        push_unique(&mut resources.ext_gstates, name);
                    }
                }
                "sh" => {
                    if let Some(name) = index.checked_sub(1).and_then(|index| name(tokens[index])) {
                        push_unique(&mut resources.shadings, name);
                    }
                }
                "scn" | "SCN" => {
                    if let Some(name) = index.checked_sub(1).and_then(|index| name(tokens[index])) {
                        push_unique(&mut resources.patterns, name);
                    }
                }
                "Tf" => {
                    let font = index.checked_sub(2).and_then(|index| name(tokens[index]));
                    resources.uses_font |= font.is_some();
                    if let Some(standard) = STANDARD_FONTS
                        .iter()
                        .find(|standard| font == Some(standard))
                        && !resources.standard_fonts.contains(standard)
                    {
                        resources.standard_fonts.push(standard);
                    }
                }
                _ => {}
            }
        }
        resources
    }

    pub(super) fn dictionary(
        &self,
        font_dict_id: usize,
        xobjects: &[(String, usize)],
        ext_gstates: &[(String, usize)],
        shadings: &[(String, usize)],
        patterns: &[(String, usize)],
        forbidden_xobject_id: Option<usize>,
    ) -> Result<String, IronpressError> {
        fn entries(
            kind: &str,
            names: &[String],
            available: &[(String, usize)],
        ) -> Result<String, IronpressError> {
            names
                .iter()
                .map(|name| {
                    available
                        .iter()
                        .find(|(available, _)| available == name)
                        .map(|(_, id)| format!("/{name} {id} 0 R"))
                        .ok_or_else(|| {
                            IronpressError::RenderError(format!(
                                "local PDF content references missing {kind} resource /{name}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|entries| entries.join(" "))
        }

        let mut sections = Vec::new();
        if self.uses_font {
            sections.push(format!("/Font {font_dict_id} 0 R"));
        }
        if !self.xobjects.is_empty() {
            let entries = entries("XObject", &self.xobjects, xobjects)?;
            if forbidden_xobject_id.is_some_and(|forbidden| {
                self.xobjects.iter().any(|name| {
                    xobjects
                        .iter()
                        .any(|(available, id)| available == name && *id == forbidden)
                })
            }) {
                return Err(IronpressError::RenderError(
                    "local PDF content references its containing form".to_owned(),
                ));
            }
            sections.push(format!("/XObject << {entries} >>"));
        }
        if !self.ext_gstates.is_empty() {
            sections.push(format!(
                "/ExtGState << {} >>",
                entries("ExtGState", &self.ext_gstates, ext_gstates)?
            ));
        }
        if !self.shadings.is_empty() {
            sections.push(format!(
                "/Shading << {} >>",
                entries("Shading", &self.shadings, shadings)?
            ));
        }
        if !self.patterns.is_empty() {
            sections.push(format!(
                "/Pattern << {} >>",
                entries("Pattern", &self.patterns, patterns)?
            ));
        }
        Ok(format!("<< {} >>", sections.join(" ")))
    }
}

#[derive(Debug)]
pub(super) struct PdfLocalFormEntry {
    pub(super) form_id: usize,
    pub(super) resources: PdfResourceUsage,
}

/// A custom TrueType font entry for the PDF font dictionary.
pub(super) struct CustomFontEntry {
    /// Sanitized PDF resource key used from page content streams.
    pub(super) resource_name: String,
    /// Object ID of the font object.
    pub(super) font_obj_id: usize,
}

pub(super) struct ConicShadingEntry {
    pub(super) name: String,
    pub(super) domain: PdfRect,
    pub(super) function: ConicShadingFunction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_resources_include_nested_patterns_once() {
        let usage = PdfResourceUsage::from_stream(
            "/Mask gs\n/Pattern CS/Pattern cs\n/Color SCN/Color scn\n/Color scn\n",
        );

        assert_eq!(usage.ext_gstates, ["Mask"]);
        assert_eq!(usage.patterns, ["Color"]);
        let dictionary = usage
            .dictionary(
                1,
                &[],
                &[("Mask".to_owned(), 2)],
                &[],
                &[("Color".to_owned(), 3)],
                None,
            )
            .unwrap();
        assert_eq!(
            dictionary,
            "<< /ExtGState << /Mask 2 0 R >> /Pattern << /Color 3 0 R >> >>"
        );
    }

    /// Standard fonts are told apart from embedded ones by name, once each.
    #[test]
    fn standard_fonts_selected_are_listed_once() {
        let usage = PdfResourceUsage::from_stream(
            "BT\n/Symbol 12 Tf\n(a) Tj\n/liberationsans 12 Tf\n(b) Tj\n/Symbol 9 Tf\nET\n",
        );
        assert!(usage.uses_font);
        assert_eq!(usage.standard_fonts, ["Symbol"]);
    }

    #[test]
    fn missing_nested_pattern_fails_closed() {
        let usage = PdfResourceUsage::from_stream("/Pattern cs /Missing scn");
        let error = usage.dictionary(1, &[], &[], &[], &[], None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing Pattern resource /Missing")
        );
    }
}
