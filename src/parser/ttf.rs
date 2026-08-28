//! Minimal TrueType font parser.
//!
//! Extracts metrics needed for PDF embedding and layout: font name, character
//! widths, units per em, cmap, bounding box, and vertical metrics.

use std::collections::HashMap;

/// Index of a face inside an OpenType font collection.
///
/// A plain TTF/OTF has [`Self::DEFAULT`]. Keeping the selected TTC face with
/// the parsed metrics is essential: shaping, outline inspection, and PDF
/// subsetting must all operate on that same face.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontFaceIndex(u32);

impl FontFaceIndex {
    pub const DEFAULT: Self = Self(0);

    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Parsed TrueType font data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontVerticalMetrics {
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
}

/// The distinct vertical metric sources a font exposes.
///
/// PDF embedding, ordinary CSS line layout, and explicit OpenType typographic
/// metrics can use different font-table values. Keeping those sources together
/// avoids accidentally applying the primary face's CSS line metrics to a
/// fallback CJK glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontVerticalMetricSet {
    pub pdf: FontVerticalMetrics,
    pub layout: FontVerticalMetrics,
    pub typographic: FontVerticalMetrics,
}

/// Font metrics used to resolve CSS text-relative sizing and initial letters.
///
/// These are optical or advance measurements rather than line-box metrics, so
/// they stay separate from [`FontVerticalMetricSet`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontTextMetrics {
    /// The font's x-height and the source that controls whether it scales
    /// linearly or must be measured from a grid-fitted glyph outline.
    pub x_height: FontXHeightMetric,
    /// The font's cap-height in font units.
    pub cap_height: u16,
    /// Advance width of the ZERO glyph in font units.
    pub zero_advance: u16,
}

/// Source-aware x-height used by CSS font-relative units.
///
/// OpenType's explicit `sxHeight` is a scalable font metric. A measured `x`
/// outline is different: its used height depends on the active grid-fitting
/// mode and point size. Keeping the distinction in the type prevents those
/// two rules from being accidentally collapsed into one ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FontXHeightMetric {
    #[default]
    Unavailable,
    OpenType(u16),
    GlyphOutline(u16),
}

impl FontXHeightMetric {
    const fn open_type(value: u16) -> Self {
        if value == 0 {
            Self::Unavailable
        } else {
            Self::OpenType(value)
        }
    }

    const fn font_units(self) -> Option<u16> {
        match self {
            Self::OpenType(value) | Self::GlyphOutline(value) if value != 0 => Some(value),
            Self::Unavailable | Self::OpenType(_) | Self::GlyphOutline(_) => None,
        }
    }
}

impl FontVerticalMetricSet {
    pub const fn new(
        pdf: FontVerticalMetrics,
        layout: FontVerticalMetrics,
        typographic: FontVerticalMetrics,
    ) -> Self {
        Self {
            pdf,
            layout,
            typographic,
        }
    }

    /// Distance from the vertical line-over edge to a synthesized central
    /// baseline. The `hhea` values used for PDF embedding define the face's
    /// ascender and descender edges for this calculation.
    pub fn central_baseline_from_over_ratio(self, units_per_em: u16) -> f32 {
        self.pdf.central_baseline_from_over_ratio(units_per_em)
    }
}

impl From<FontVerticalMetrics> for FontVerticalMetricSet {
    fn from(metrics: FontVerticalMetrics) -> Self {
        Self::new(metrics, metrics, metrics)
    }
}

impl FontVerticalMetrics {
    pub const fn new(ascent: i16, descent: i16, line_gap: i16) -> Self {
        Self {
            ascent,
            descent,
            line_gap,
        }
    }

    pub fn ascender_ratio(self, units_per_em: u16) -> f32 {
        if units_per_em == 0 {
            return 0.0;
        }
        f32::from(self.ascent).max(0.0) / f32::from(units_per_em)
    }

    pub fn descender_ratio(self, units_per_em: u16) -> f32 {
        if units_per_em == 0 {
            return 0.0;
        }
        (-f32::from(self.descent)).max(0.0) / f32::from(units_per_em)
    }

    pub fn line_gap_ratio(self, units_per_em: u16) -> f32 {
        if units_per_em == 0 {
            return 0.0;
        }
        f32::from(self.line_gap).max(0.0) / f32::from(units_per_em)
    }

    /// Distance from the vertical line-over edge to the midpoint between this
    /// face's ascender and descender edges, expressed in ems.
    ///
    /// This is the central-baseline fallback for a face without an explicit
    /// central-baseline record. `descent` is signed, as it is in OpenType.
    pub fn central_baseline_from_over_ratio(self, units_per_em: u16) -> f32 {
        if units_per_em == 0 {
            return 0.5;
        }
        let central_from_alphabetic = (f32::from(self.ascent) + f32::from(self.descent)) * 0.5;
        1.0 - central_from_alphabetic / f32::from(units_per_em)
    }

    pub fn line_height_ratio(self, units_per_em: u16) -> f32 {
        if units_per_em == 0 {
            return 1.0;
        }
        let height = i32::from(self.ascent) - i32::from(self.descent) + i32::from(self.line_gap);
        (height.max(0) as f32 / f32::from(units_per_em)).max(1.0)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TtfFont {
    /// Font family name from the `name` table.
    pub font_name: String,
    /// Selected face in the raw OpenType data. This is non-zero for a parsed
    /// TTC/OTC face other than the collection's first face.
    pub face_index: FontFaceIndex,
    /// Units per em from the `head` table.
    pub units_per_em: u16,
    /// CSS Fonts `@font-face size-adjust` multiplier for this selected face.
    /// The embedded OpenType program remains byte-for-byte authentic; layout
    /// and PDF text painting apply this scale through the used font size.
    pub size_adjust: f32,
    /// Font bounding box [xMin, yMin, xMax, yMax] from the `head` table.
    pub bbox: [i16; 4],
    /// Metrics selected from the font's PDF, layout, and typographic tables.
    pub vertical_metrics: FontVerticalMetricSet,
    /// Character code (Unicode codepoint) to glyph ID mapping from the `cmap` table.
    /// Uses u32 keys to support astral plane characters (emoji, CJK extensions).
    pub cmap: HashMap<u32, u16>,
    /// Advance width for each glyph ID from the `hmtx` table.
    pub glyph_widths: Vec<u16>,
    /// Number of horizontal metrics entries (from `hhea`).
    pub num_h_metrics: u16,
    /// Font flags for the PDF FontDescriptor.
    pub flags: u32,
    /// Whether this face is itself a bold weight (OS/2 `usWeightClass` >= 600,
    /// or the `head.macStyle` bold bit). Lets the renderer tell a genuine bold
    /// face from a regular face that a font query merely *substituted* for a
    /// missing bold, so it can synthesise (faux) bold only when truly needed.
    pub is_bold: bool,
    /// Whether this face is itself italic/oblique (OS/2 `fsSelection` ITALIC bit
    /// or `head.macStyle` italic bit). Lets the renderer synthesise (faux) italic
    /// — an algorithmic shear — only when an italic request resolves to a face
    /// that is not genuinely italic (CSS Fonts 4 §2.4 `font-synthesis: style`).
    pub is_italic: bool,
    /// Optical and advance measurements used by CSS `ex`, `cap`, `ch`, and
    /// initial-letter sizing.
    pub text_metrics: FontTextMetrics,
    /// Raw TTF data for embedding. Wrapped in Arc so cloning a TtfFont
    /// (e.g. from the bundled font cache) is O(1) instead of copying ~400KB.
    pub data: std::sync::Arc<Vec<u8>>,
}

/// The non-negative inline side bearings of a glyph at a used font size.
///
/// They describe the empty inline space between a glyph's ink bounds and its
/// advance edges. Initial-letter layout uses them to kern its margin box around
/// the glyph ink without inventing a PDF-coordinate adjustment.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlyphSideBearings {
    pub start: f32,
    pub end: f32,
}

impl TtfFont {
    /// The glyph-painting font size after this face's `size-adjust` descriptor.
    pub fn adjusted_font_size(&self, font_size: f32) -> f32 {
        font_size * self.size_adjust_factor()
    }

    /// The valid `@font-face size-adjust` multiplier for this face.
    ///
    /// Invalid descriptors resolve to the descriptor's initial value, `1`.
    /// Keeping that normalization here ensures glyph painting, shaping, and
    /// vertical metrics cannot disagree about a face's effective scale.
    pub fn size_adjust_factor(&self) -> f32 {
        if self.size_adjust.is_finite() && self.size_adjust > 0.0 {
            self.size_adjust
        } else {
            1.0
        }
    }

    pub const fn pdf_vertical_metrics(&self) -> FontVerticalMetrics {
        self.vertical_metrics.pdf
    }

    pub const fn layout_vertical_metrics(&self) -> FontVerticalMetrics {
        self.vertical_metrics.layout
    }

    /// Explicit typographic metrics from the font's `OS/2` table.
    pub const fn typographic_vertical_metrics(&self) -> FontVerticalMetrics {
        self.vertical_metrics.typographic
    }

    /// Get the advance width for a glyph ID, in font units.
    pub fn glyph_width(&self, glyph_id: u16) -> u16 {
        if (glyph_id as usize) < self.glyph_widths.len() {
            self.glyph_widths[glyph_id as usize]
        } else {
            self.glyph_widths.last().copied().unwrap_or(0)
        }
    }

    /// Get the advance width for a character code, in font units.
    #[cfg(test)]
    pub fn char_width(&self, ch: u16) -> u16 {
        let glyph_id = self.cmap.get(&(ch as u32)).copied().unwrap_or(0);
        self.glyph_width(glyph_id)
    }

    /// The actual glyph bounding-box top (`yMax`) for `ch`, as a fraction of the
    /// em. Used to seat a floated `::first-letter` drop cap so its visual cap top
    /// aligns with the surrounding line's text top (css-pseudo-4 §2.2): the font
    /// ascender overshoots the glyph (it reserves accent space), so aligning by
    /// ascender would push the drop cap too low. Returns `None` when the glyph has
    /// no outline/bounds (e.g. whitespace) so the caller can fall back.
    pub fn glyph_top_ratio(&self, ch: char) -> Option<f32> {
        if self.units_per_em == 0 {
            return None;
        }
        let y_max = measure_glyph_y_max(&self.data, self.face_index, ch)?;
        Some(f32::from(y_max) / f32::from(self.units_per_em))
    }

    /// The visual centre of a glyph's outline above its baseline, as a
    /// fraction of the em. This places a glyph used as a mark by its ink
    /// rather than the font's full ascent.
    pub fn glyph_center_ratio(&self, ch: char) -> Option<f32> {
        if self.units_per_em == 0 {
            return None;
        }
        let face = rustybuzz::ttf_parser::Face::parse(&self.data, self.face_index.get()).ok()?;
        let glyph_id = face.glyph_index(ch)?;
        let bounds = face.glyph_bounding_box(glyph_id)?;
        let center = (f32::from(bounds.y_min) + f32::from(bounds.y_max)) / 2.0;
        Some(center / f32::from(self.units_per_em))
    }

    /// Measure the glyph's inline side bearings at `font_size` CSS pixels.
    /// Returns `None` when the font has no measurable outline for `ch`.
    pub fn glyph_side_bearings(&self, ch: char, font_size: f32) -> Option<GlyphSideBearings> {
        if self.units_per_em == 0 || !font_size.is_finite() {
            return None;
        }
        let face = rustybuzz::ttf_parser::Face::parse(&self.data, self.face_index.get()).ok()?;
        let glyph_id = face.glyph_index(ch)?;
        let bounds = face.glyph_bounding_box(glyph_id)?;
        let advance = f32::from(face.glyph_hor_advance(glyph_id)?);
        let scale = font_size.max(0.0) / f32::from(self.units_per_em);
        Some(GlyphSideBearings {
            start: f32::from(bounds.x_min).max(0.0) * scale,
            end: (advance - f32::from(bounds.x_max)).max(0.0) * scale,
        })
    }

    /// x-height as a fraction of the em (css-values-4 §6.1.1 `ex`). Falls back
    /// to the CSS-recommended 0.5em when the metric could not be determined.
    pub fn x_height_ratio(&self) -> f32 {
        let Some(x_height) = self.text_metrics.x_height.font_units() else {
            return 0.5;
        };
        if self.units_per_em == 0 {
            return 0.5;
        }
        f32::from(x_height) / f32::from(self.units_per_em)
    }

    /// X-height exposed to CSS layout at this point size.
    ///
    /// TrueType instructions grid-fit the lowercase `x` differently at
    /// different CSS sizes, so scaling the design-space bounding box is not
    /// the font's used x-height. When the hinted outline cannot be obtained,
    /// retain the metric/fallback ratio required by CSS Values.
    pub fn used_x_height(&self, font_size: f32) -> f32 {
        let scaled_metric = || self.x_height_ratio() * font_size.max(0.0);
        match self.text_metrics.x_height {
            FontXHeightMetric::GlyphOutline(_) => self
                .hinted_x_height(font_size)
                .unwrap_or_else(scaled_metric),
            FontXHeightMetric::OpenType(_) | FontXHeightMetric::Unavailable => scaled_metric(),
        }
    }

    fn hinted_x_height(&self, font_size: f32) -> Option<f32> {
        use skrifa::MetadataProvider as _;
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::{
            Engine, HintingInstance, HintingOptions, SmoothMode, Target, pen::ControlBoundsPen,
        };

        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        let font_size_px = font_size / crate::fonts::PT_PER_CSS_PX;
        let font = skrifa::FontRef::from_index(&self.data, self.face_index.get()).ok()?;
        let outlines = font.outline_glyphs();
        let glyph = outlines.get(font.charmap().map('x')?)?;
        let hinting = HintingInstance::new(
            &outlines,
            Size::new(font_size_px),
            LocationRef::default(),
            HintingOptions {
                engine: Engine::Auto(None),
                target: Target::from(SmoothMode::Light),
            },
        )
        .ok()?;
        let mut bounds = ControlBoundsPen::new();
        glyph.draw(&hinting, &mut bounds).ok()?;
        let bounds = bounds.bounding_box()?;
        let x_height_px = bounds.y_max - bounds.y_min;
        (x_height_px.is_finite() && x_height_px > 0.0)
            .then_some(x_height_px * crate::fonts::PT_PER_CSS_PX)
    }

    /// The x-height aspect Blink exposes to `font-size-adjust` for this CSS
    /// font size. Font metric units become CSS layout geometry before the
    /// property computes its used font size, so preserve the CSS-pixel metric
    /// rather than applying an unbounded outline ratio directly.
    pub fn font_size_adjust_x_height_ratio(&self, computed_font_size: f32) -> f32 {
        if !computed_font_size.is_finite() || computed_font_size <= 0.0 {
            return self.x_height_ratio();
        }
        let used_x_height = self.used_x_height(computed_font_size);
        if used_x_height > 0.0 {
            used_x_height / computed_font_size
        } else {
            self.x_height_ratio()
        }
    }

    /// `ch` advance (the `'0'` glyph) as a fraction of the em (css-values-4
    /// §6.1.1). Falls back to the CSS-recommended 0.5em when undeterminable.
    pub fn ch_ratio(&self) -> f32 {
        if self.units_per_em == 0 || self.text_metrics.zero_advance == 0 {
            return 0.5;
        }
        f32::from(self.text_metrics.zero_advance) / f32::from(self.units_per_em)
    }

    /// CSS Inline's cap-height ratio for initial-letter sizing.
    ///
    /// Prefer the OpenType `OS/2.sCapHeight` metric. If the font omits it,
    /// parsing records the measured `H` glyph height; the CSS Inline 3 fallback
    /// is `.66em` only when neither source is available.
    pub fn cap_height_ratio(&self) -> f32 {
        if self.units_per_em == 0 || self.text_metrics.cap_height == 0 {
            return 0.66;
        }
        f32::from(self.text_metrics.cap_height) / f32::from(self.units_per_em)
    }

    /// Get the advance width for a glyph in PDF units (1/1000 of text space).
    pub fn glyph_width_pdf_value(&self, glyph_id: u16) -> f32 {
        if self.units_per_em == 0 {
            return 0.0;
        }
        self.glyph_width(glyph_id) as f32 * 1000.0 / self.units_per_em as f32
    }

    /// Get the advance width for a glyph scaled to a given font size in points.
    pub fn glyph_width_scaled(&self, glyph_id: u16, font_size: f32) -> f32 {
        if self.units_per_em == 0 {
            return 0.0;
        }
        let width = self.glyph_width(glyph_id) as f32;
        width * font_size / self.units_per_em as f32
    }

    /// Get the advance width for a character in PDF units (1/1000 of text space).
    #[cfg(test)]
    pub fn char_width_pdf(&self, ch: u16) -> u16 {
        self.char_width_pdf_value(ch).trunc() as u16
    }

    /// Get the advance width for a character in PDF units (1/1000 of text space).
    #[cfg(test)]
    pub fn char_width_pdf_value(&self, ch: u16) -> f32 {
        if self.units_per_em == 0 {
            return 0.0;
        }
        self.char_width(ch) as f32 * 1000.0 / self.units_per_em as f32
    }

    /// Get the advance width scaled to a given font size in points.
    #[cfg(test)]
    pub fn char_width_scaled(&self, ch: u16, font_size: f32) -> f32 {
        let glyph_id = self.cmap.get(&(ch as u32)).copied().unwrap_or(0);
        self.glyph_width_scaled(glyph_id, font_size)
    }
}

/// Table directory entry.
#[derive(Debug)]
struct TableRecord {
    offset: u32,
    #[allow(dead_code)]
    length: u32,
}

/// Process-global cache of parsed fonts keyed by a fast hash of the raw TTF
/// bytes. Registering the same font (via `add_font`, `@font-face`, or a bundled
/// asset) re-parses megabytes of TTF data on every `convert()` call otherwise;
/// caching the parse makes the second and later calls in a process near-free.
/// Cloning the cached `TtfFont` shares the Arc'd byte data and only copies the
/// small cmap / glyph-width tables.
static PARSED_FONT_CACHE: std::sync::OnceLock<std::sync::RwLock<HashMap<u64, TtfFont>>> =
    std::sync::OnceLock::new();

fn parsed_font_cache() -> &'static std::sync::RwLock<HashMap<u64, TtfFont>> {
    PARSED_FONT_CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

/// FNV-1a 64-bit hash over the whole byte slice — fast, dependency-free, and
/// collision-safe enough for the handful of distinct fonts a process registers.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Parse a TTF reusing a process-global cache keyed by the byte content.
/// Returns `None` on parse failure (errors are not cached). The stored font's
/// data length is checked on a hash hit to guard against the (astronomically
/// unlikely) FNV collision between two same-length blobs.
pub(crate) fn parse_ttf_cached(data: &[u8]) -> Option<TtfFont> {
    let key = fnv1a_64(data);
    if let Ok(cache) = parsed_font_cache().read()
        && let Some(font) = cache.get(&key)
        && font.data.len() == data.len()
    {
        return Some(font.clone());
    }
    let font = parse_ttf(data.to_vec()).ok()?;
    if let Ok(mut cache) = parsed_font_cache().write() {
        cache.entry(key).or_insert_with(|| font.clone());
    }
    Some(font)
}

/// Parse a TrueType font from raw TTF data.
/// Parse a TTF/TTC font, selecting a specific font by index within a collection.
/// For plain TTF files, `face_index` is ignored (always index 0).
pub fn parse_ttf_with_index(data: Vec<u8>, face_index: usize) -> Result<TtfFont, String> {
    // Handle TrueType Collection (TTC) files
    if data.len() >= 12 && &data[0..4] == b"ttcf" {
        let num_fonts = read_u32(&data, 8) as usize;
        if face_index >= num_fonts {
            return Err(format!(
                "TTC face index {face_index} out of range (collection has {num_fonts} fonts)"
            ));
        }
        let offset_pos = 12 + face_index * 4;
        if data.len() < offset_pos + 4 {
            return Err("TTC offset table too short".to_string());
        }
        let font_offset = read_u32(&data, offset_pos) as usize;
        if font_offset >= data.len() {
            return Err("TTC font offset out of bounds".to_string());
        }
        let selected_face = u32::try_from(face_index)
            .map(FontFaceIndex::new)
            .map_err(|_| "TTC face index exceeds PDF font index range".to_string())?;
        return parse_ttf_at_offset(data, font_offset, selected_face);
    }

    parse_ttf_at_offset(data, 0, FontFaceIndex::DEFAULT)
}

pub fn parse_ttf(data: Vec<u8>) -> Result<TtfFont, String> {
    // Handle TrueType Collection (TTC) files: extract the first font.
    if data.len() >= 12 && &data[0..4] == b"ttcf" {
        let num_fonts = read_u32(&data, 8);
        if num_fonts == 0 || data.len() < 16 {
            return Err("TTC contains no fonts".to_string());
        }
        let first_offset = read_u32(&data, 12) as usize;
        if first_offset >= data.len() {
            return Err("TTC first font offset out of bounds".to_string());
        }
        // Parse using the full data but starting at the first font's offset table.
        return parse_ttf_at_offset(data, first_offset, FontFaceIndex::DEFAULT);
    }

    parse_ttf_at_offset(data, 0, FontFaceIndex::DEFAULT)
}

fn parse_ttf_at_offset(
    data: Vec<u8>,
    base: usize,
    face_index: FontFaceIndex,
) -> Result<TtfFont, String> {
    if data.len() < base + 12 {
        return Err("TTF data too short for offset table".to_string());
    }

    let num_tables = read_u16(&data, base + 4);
    if data.len() < base + 12 + num_tables as usize * 16 {
        return Err("TTF data too short for table directory".to_string());
    }

    // Parse table directory
    let mut tables: HashMap<[u8; 4], TableRecord> = HashMap::new();
    for i in 0..num_tables as usize {
        let offset = base + 12 + i * 16;
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[offset..offset + 4]);
        tables.insert(
            tag,
            TableRecord {
                offset: read_u32(&data, offset + 8),
                length: read_u32(&data, offset + 12),
            },
        );
    }

    // Parse head table
    let head = tables.get(b"head").ok_or("Missing head table")?;
    let head_off = head.offset as usize;
    if data.len() < head_off + 54 {
        return Err("head table too short".to_string());
    }
    let units_per_em = read_u16(&data, head_off + 18);
    if units_per_em == 0 {
        return Err("Invalid units_per_em (0) in head table".to_string());
    }
    let x_min = read_i16(&data, head_off + 36);
    let y_min = read_i16(&data, head_off + 38);
    let x_max = read_i16(&data, head_off + 40);
    let y_max = read_i16(&data, head_off + 42);
    let bbox = [x_min, y_min, x_max, y_max];

    // Parse hhea table
    let hhea = tables.get(b"hhea").ok_or("Missing hhea table")?;
    let hhea_off = hhea.offset as usize;
    if data.len() < hhea_off + 36 {
        return Err("hhea table too short".to_string());
    }
    let hhea_ascent = read_i16(&data, hhea_off + 4);
    let hhea_descent = read_i16(&data, hhea_off + 6);
    let hhea_line_gap = read_i16(&data, hhea_off + 8);
    let num_h_metrics = read_u16(&data, hhea_off + 34);

    let pdf_metrics = FontVerticalMetrics::new(hhea_ascent, hhea_descent, hhea_line_gap);
    let layout_metrics =
        parse_os2_layout_metrics(&data, tables.get(b"OS/2")).unwrap_or(pdf_metrics);
    let typographic_metrics =
        parse_os2_typographic_metrics(&data, tables.get(b"OS/2")).unwrap_or(layout_metrics);

    // Parse maxp table for num_glyphs
    let maxp = tables.get(b"maxp").ok_or("Missing maxp table")?;
    let maxp_off = maxp.offset as usize;
    if data.len() < maxp_off + 6 {
        return Err("maxp table too short".to_string());
    }
    let num_glyphs = read_u16(&data, maxp_off + 4);

    // Parse hmtx table
    let hmtx = tables.get(b"hmtx").ok_or("Missing hmtx table")?;
    let hmtx_off = hmtx.offset as usize;
    let mut glyph_widths = Vec::with_capacity(num_glyphs as usize);
    let mut last_width = 0u16;
    for i in 0..num_glyphs as usize {
        if i < num_h_metrics as usize {
            let entry_off = hmtx_off + i * 4;
            if data.len() < entry_off + 2 {
                break;
            }
            last_width = read_u16(&data, entry_off);
            glyph_widths.push(last_width);
        } else {
            // Glyphs beyond num_h_metrics share the last advance width
            glyph_widths.push(last_width);
        }
    }

    // Parse cmap table
    let cmap_table = tables.get(b"cmap").ok_or("Missing cmap table")?;
    let cmap = parse_cmap(&data, cmap_table.offset as usize)?;

    // Parse name table
    let name_table = tables.get(b"name").ok_or("Missing name table")?;
    let font_name = parse_name_table(&data, name_table.offset as usize)?;

    // Compute flags: bit 5 (Nonsymbolic) = 32 for Latin text
    let flags = 32u32;

    // Detect a genuine bold weight: OS/2 usWeightClass (offset +4) >= 600, or
    // the head.macStyle bold bit (bit 0 of the u16 at head +44).
    let os2_bold = tables.get(b"OS/2").is_some_and(|os2| {
        let off = os2.offset as usize;
        data.len() >= off + 6 && read_u16(&data, off + 4) >= 600
    });
    let mac_style_bold = data.len() >= head_off + 46 && (read_u16(&data, head_off + 44) & 0x1) != 0;
    let is_bold = os2_bold || mac_style_bold;

    // Detect a genuine italic/oblique face: OS/2 fsSelection (offset +62) ITALIC
    // bit, or the head.macStyle italic bit (bit 1 of the u16 at head +44).
    let os2_italic = tables.get(b"OS/2").is_some_and(|os2| {
        let off = os2.offset as usize;
        data.len() >= off + 64 && (read_u16(&data, off + 62) & 0x1) != 0
    });
    let mac_style_italic =
        data.len() >= head_off + 46 && (read_u16(&data, head_off + 44) & 0x2) != 0;
    let is_italic = os2_italic || mac_style_italic;

    // x-height for the CSS `ex` unit. Prefer OS/2.sxHeight (version >= 2,
    // offset +86 as a signed FWORD); when absent or non-positive, measure the
    // 'x' glyph's bounding-box top — this mirrors Chrome's `ex` resolution for
    // fonts (such as the bundled DejaVu-derived faces) whose OS/2 table predates
    // version 2 and therefore carries no sxHeight field.
    let os2 = tables.get(b"OS/2");
    let os2_x_height = parse_os2_positive_metric(&data, os2, 86);
    let x_height = os2_x_height.map_or_else(
        || {
            measure_glyph_y_max(&data, face_index, 'x').map_or(
                FontXHeightMetric::Unavailable,
                FontXHeightMetric::GlyphOutline,
            )
        },
        FontXHeightMetric::open_type,
    );
    // CSS Inline 3 initial-letter sizing uses cap-height rather than a
    // first-letter outline. Prefer OS/2.sCapHeight (version >= 2), then the
    // conventional Latin H measurement when the metric is absent.
    let cap_height = parse_os2_positive_metric(&data, os2, 88)
        .or_else(|| measure_glyph_y_max(&data, face_index, 'H'))
        .unwrap_or(0);
    // ch unit: advance of the '0' (ZERO) glyph.
    let zero_advance = cmap
        .get(&u32::from('0'))
        .map(|&gid| {
            if (gid as usize) < glyph_widths.len() {
                glyph_widths[gid as usize]
            } else {
                0
            }
        })
        .unwrap_or(0);

    Ok(TtfFont {
        font_name,
        face_index,
        units_per_em,
        size_adjust: 1.0,
        bbox,
        vertical_metrics: FontVerticalMetricSet::new(
            pdf_metrics,
            layout_metrics,
            typographic_metrics,
        ),
        cmap,
        glyph_widths,
        num_h_metrics,
        flags,
        is_bold,
        is_italic,
        text_metrics: FontTextMetrics {
            x_height,
            cap_height,
            zero_advance,
        },
        data: std::sync::Arc::new(data),
    })
}

/// Read a positive version-2-or-later signed metric from the OS/2 table.
///
/// `sxHeight` and `sCapHeight` share this shape: both were introduced in
/// version 2 and both are optional in practice even when the table version
/// permits them. A short or older table simply leaves metric selection to the
/// caller's documented fallback.
fn parse_os2_positive_metric(
    data: &[u8],
    os2: Option<&TableRecord>,
    field_offset: usize,
) -> Option<u16> {
    let os2 = os2?;
    let offset = os2.offset as usize;
    if data.len() < offset + field_offset + 2 || read_u16(data, offset) < 2 {
        return None;
    }
    let value = read_i16(data, offset + field_offset);
    (value > 0).then_some(value as u16)
}

/// Measure the bounding-box top (`yMax`, in font units) of the glyph for `ch`.
///
/// Used to derive the x-height for the CSS `ex` unit when `OS/2.sxHeight` is
/// absent. Returns `None` when the character has no glyph or no contour bounds.
fn measure_glyph_y_max(data: &[u8], face_index: FontFaceIndex, ch: char) -> Option<u16> {
    let face = rustybuzz::ttf_parser::Face::parse(data, face_index.get()).ok()?;
    let gid = face.glyph_index(ch)?;
    let bbox = face.glyph_bounding_box(gid)?;
    (bbox.y_max > 0).then_some(bbox.y_max as u16)
}

fn parse_os2_layout_metrics(data: &[u8], os2: Option<&TableRecord>) -> Option<FontVerticalMetrics> {
    let os2 = os2?;
    let os2_off = os2.offset as usize;
    if data.len() < os2_off + 78 {
        return None;
    }

    // Chromium uses usWinAscent/usWinDescent (not sTypo*) for
    // line-height:normal unless USE_TYPO_METRICS bit is set in fsSelection.
    // Match this behavior for visual parity.
    let win_ascent = read_u16(data, os2_off + 74) as i16;
    let win_descent = read_u16(data, os2_off + 76) as i16;
    // Use line_gap=0 with usWin metrics (Chrome doesn't add sTypoLineGap
    // when using usWin metrics).
    Some(FontVerticalMetrics::new(win_ascent, -win_descent, 0))
}

/// Parse the explicit typographic metrics from the `OS/2` table.
///
/// These are distinct from the Windows metrics selected for ordinary
/// `line-height: normal`. When the table is absent or declares no typographic
/// extent, the normal layout metrics are the only safe fallback.
fn parse_os2_typographic_metrics(
    data: &[u8],
    os2: Option<&TableRecord>,
) -> Option<FontVerticalMetrics> {
    let os2 = os2?;
    let os2_off = os2.offset as usize;
    if data.len() < os2_off + 74 {
        return None;
    }
    let metrics = FontVerticalMetrics::new(
        read_i16(data, os2_off + 68),
        read_i16(data, os2_off + 70),
        read_i16(data, os2_off + 72),
    );
    (metrics.ascent != 0 || metrics.descent != 0).then_some(metrics)
}

/// Parse the cmap table. Prefers format 12 (full Unicode including astral
/// plane) over format 4 (BMP only) for emoji and extended CJK support.
fn parse_cmap(data: &[u8], offset: usize) -> Result<HashMap<u32, u16>, String> {
    if data.len() < offset + 4 {
        return Err("cmap table too short".to_string());
    }
    let num_subtables = read_u16(data, offset + 2);

    // Scan all subtables — prefer format 12 (full Unicode) over format 4 (BMP).
    let mut bmp_offset = None;
    let mut full_offset = None;
    for i in 0..num_subtables as usize {
        let record_off = offset + 4 + i * 8;
        if data.len() < record_off + 8 {
            break;
        }
        let platform_id = read_u16(data, record_off);
        let encoding_id = read_u16(data, record_off + 2);
        let sub_offset = read_u32(data, record_off + 4) as usize;
        let abs_off = offset + sub_offset;

        // Platform 3 encoding 10 = Windows full Unicode (format 12)
        if platform_id == 3 && encoding_id == 10 {
            full_offset = Some(abs_off);
        }
        // Platform 3 encoding 1 = Windows BMP, or Platform 0 = Unicode
        if ((platform_id == 3 && encoding_id == 1) || platform_id == 0) && bmp_offset.is_none() {
            bmp_offset = Some(abs_off);
        }
    }

    // Try format 12 first (full Unicode), fall back to format 4 (BMP)
    if let Some(off) = full_offset {
        if data.len() > off + 2 {
            let format = read_u16(data, off);
            if format == 12 {
                return parse_cmap_format12(data, off);
            }
        }
    }

    let sub_off = bmp_offset.ok_or("No suitable cmap subtable found")?;
    if data.len() < sub_off + 2 {
        return Err("cmap subtable too short".to_string());
    }
    let format = read_u16(data, sub_off);

    match format {
        4 => parse_cmap_format4(data, sub_off),
        0 => parse_cmap_format0(data, sub_off),
        _ => Ok(HashMap::new()),
    }
}

/// Parse cmap format 0 (byte encoding table).
fn parse_cmap_format0(data: &[u8], offset: usize) -> Result<HashMap<u32, u16>, String> {
    if data.len() < offset + 262 {
        return Err("cmap format 0 table too short".to_string());
    }
    let mut map = HashMap::new();
    for i in 0..256u32 {
        let glyph_id = data[offset + 6 + i as usize] as u16;
        if glyph_id != 0 {
            map.insert(i, glyph_id);
        }
    }
    Ok(map)
}

/// Parse cmap format 12 (segmented coverage for full Unicode).
fn parse_cmap_format12(data: &[u8], offset: usize) -> Result<HashMap<u32, u16>, String> {
    // Format 12 header: format(2) + reserved(2) + length(4) + language(4) + nGroups(4)
    if data.len() < offset + 16 {
        return Err("cmap format 12 table too short".to_string());
    }
    let n_groups = read_u32(data, offset + 12) as usize;
    let mut map = HashMap::new();
    for i in 0..n_groups {
        let group_off = offset + 16 + i * 12;
        if data.len() < group_off + 12 {
            break;
        }
        let start_char = read_u32(data, group_off);
        let end_char = read_u32(data, group_off + 4);
        let start_glyph = read_u32(data, group_off + 8);
        for ch in start_char..=end_char {
            let glyph_id = (start_glyph + (ch - start_char)) as u16;
            if glyph_id != 0 {
                map.insert(ch, glyph_id);
            }
        }
    }
    Ok(map)
}

/// Parse cmap format 4 (segment mapping to delta values).
fn parse_cmap_format4(data: &[u8], offset: usize) -> Result<HashMap<u32, u16>, String> {
    if data.len() < offset + 14 {
        return Err("cmap format 4 header too short".to_string());
    }

    let seg_count_x2 = read_u16(data, offset + 6);
    let seg_count = seg_count_x2 as usize / 2;

    let end_code_off = offset + 14;
    // +2 for reserved padding after endCode array
    let start_code_off = end_code_off + seg_count * 2 + 2;
    let id_delta_off = start_code_off + seg_count * 2;
    let id_range_offset_off = id_delta_off + seg_count * 2;

    let needed = id_range_offset_off + seg_count * 2;
    if data.len() < needed {
        return Err("cmap format 4 data too short".to_string());
    }

    let mut map = HashMap::new();

    for i in 0..seg_count {
        let end_code = read_u16(data, end_code_off + i * 2);
        let start_code = read_u16(data, start_code_off + i * 2);
        let id_delta = read_i16(data, id_delta_off + i * 2) as i32;
        let id_range_offset = read_u16(data, id_range_offset_off + i * 2);

        if start_code == 0xFFFF {
            break;
        }

        for c in start_code..=end_code {
            let glyph_id = if id_range_offset == 0 {
                ((c as i32 + id_delta) & 0xFFFF) as u16
            } else {
                // idRangeOffset is relative to the current position in the
                // idRangeOffset array
                let range_off = id_range_offset_off + i * 2;
                let glyph_off =
                    range_off + id_range_offset as usize + (c as usize - start_code as usize) * 2;
                if glyph_off + 1 < data.len() {
                    let gid = read_u16(data, glyph_off);
                    if gid != 0 {
                        ((gid as i32 + id_delta) & 0xFFFF) as u16
                    } else {
                        0
                    }
                } else {
                    0
                }
            };
            if glyph_id != 0 {
                map.insert(c as u32, glyph_id);
            }
        }
    }

    Ok(map)
}

/// Parse the name table and extract the best available font face name.
fn parse_name_table(data: &[u8], offset: usize) -> Result<String, String> {
    if data.len() < offset + 6 {
        return Err("name table too short".to_string());
    }

    let count = read_u16(data, offset + 2);
    let string_offset = read_u16(data, offset + 4) as usize;
    let storage_off = offset + string_offset;

    // Prefer the PostScript name (nameID 6), then full name (4), then family (1).
    let mut best_name: Option<String> = None;
    let mut best_priority = 0u8;

    for i in 0..count as usize {
        let rec_off = offset + 6 + i * 12;
        if data.len() < rec_off + 12 {
            break;
        }
        let platform_id = read_u16(data, rec_off);
        let encoding_id = read_u16(data, rec_off + 2);
        let name_id = read_u16(data, rec_off + 6);
        let length = read_u16(data, rec_off + 8) as usize;
        let str_offset = read_u16(data, rec_off + 10) as usize;

        let priority = match name_id {
            6 => 3,
            4 => 2,
            1 => 1,
            _ => continue,
        };
        if priority <= best_priority {
            continue;
        }

        let start = storage_off + str_offset;
        let end = start + length;
        if end > data.len() {
            continue;
        }

        let name_bytes = &data[start..end];
        let name = if platform_id == 3 || (platform_id == 0 && encoding_id > 0) {
            // UTF-16BE
            decode_utf16be(name_bytes)
        } else {
            // Latin-1/ASCII
            String::from_utf8_lossy(name_bytes).to_string()
        };

        if !name.is_empty() {
            best_name = Some(name);
            best_priority = priority;
        }
    }

    best_name.ok_or_else(|| "No font name found in name table".to_string())
}

/// Decode a UTF-16BE byte slice to a String.
fn decode_utf16be(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let code_unit = ((data[i] as u16) << 8) | data[i + 1] as u16;
        if let Some(ch) = char::from_u32(code_unit as u32) {
            result.push(ch);
        }
        i += 2;
    }
    result
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid TTF file for testing.
    /// Contains: head, hhea, maxp, hmtx, cmap (format 4), name tables.
    fn build_test_ttf() -> Vec<u8> {
        let mut buf = Vec::new();

        // We'll build 6 tables: head, hhea, maxp, hmtx, cmap, name
        let num_tables: u16 = 6;

        // Offset table (12 bytes)
        buf.extend_from_slice(&[0, 1, 0, 0]); // sfVersion = 1.0
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes()); // searchRange
        buf.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        buf.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

        // Table directory: 6 entries * 16 bytes = 96 bytes
        // Directory starts at offset 12
        // Table data starts at 12 + 96 = 108

        // We'll fill in table records after building each table's data
        let dir_start = buf.len();
        // Reserve space for directory
        buf.resize(dir_start + num_tables as usize * 16, 0);

        let _data_start = buf.len(); // = 108

        // ---- head table (54 bytes) ----
        let head_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]); // version
        buf.extend_from_slice(&[0, 0, 0, 0]); // fontRevision
        buf.extend_from_slice(&[0, 0, 0, 0]); // checksumAdjustment
        buf.extend_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]); // magicNumber
        buf.extend_from_slice(&0x000Bu16.to_be_bytes()); // flags
        buf.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm = 1000
        buf.extend_from_slice(&[0; 8]); // created
        buf.extend_from_slice(&[0; 8]); // modified
        buf.extend_from_slice(&(-100i16).to_be_bytes()); // xMin
        buf.extend_from_slice(&(-200i16).to_be_bytes()); // yMin
        buf.extend_from_slice(&800i16.to_be_bytes()); // xMax
        buf.extend_from_slice(&900i16.to_be_bytes()); // yMax
        buf.extend_from_slice(&0u16.to_be_bytes()); // macStyle
        buf.extend_from_slice(&8u16.to_be_bytes()); // lowestRecPPEM
        buf.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
        buf.extend_from_slice(&1i16.to_be_bytes()); // indexToLocFormat
        buf.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat
        let head_len = buf.len() - head_offset;

        // ---- hhea table (36 bytes) ----
        let hhea_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]); // version
        buf.extend_from_slice(&800i16.to_be_bytes()); // ascent
        buf.extend_from_slice(&(-200i16).to_be_bytes()); // descent
        buf.extend_from_slice(&0i16.to_be_bytes()); // lineGap
        buf.extend_from_slice(&700u16.to_be_bytes()); // advanceWidthMax
        buf.extend_from_slice(&0i16.to_be_bytes()); // minLeftSideBearing
        buf.extend_from_slice(&0i16.to_be_bytes()); // minRightSideBearing
        buf.extend_from_slice(&700i16.to_be_bytes()); // xMaxExtent
        buf.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
        buf.extend_from_slice(&0i16.to_be_bytes()); // caretSlopeRun
        buf.extend_from_slice(&0i16.to_be_bytes()); // caretOffset
        buf.extend_from_slice(&[0; 8]); // reserved
        buf.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
        buf.extend_from_slice(&3u16.to_be_bytes()); // numOfLongHorMetrics = 3
        let hhea_len = buf.len() - hhea_offset;

        // ---- maxp table (6 bytes for version 0.5, or 32 for 1.0) ----
        let maxp_offset = buf.len();
        buf.extend_from_slice(&[0, 0, 0x50, 0]); // version 0.5 (0x00005000)
        buf.extend_from_slice(&3u16.to_be_bytes()); // numGlyphs = 3
        let maxp_len = buf.len() - maxp_offset;

        // ---- hmtx table ----
        // 3 glyphs: glyph 0 (notdef), glyph 1 (space, char 32), glyph 2 (A, char 65)
        let hmtx_offset = buf.len();
        // Glyph 0: width=500, lsb=0
        buf.extend_from_slice(&500u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        // Glyph 1: width=250, lsb=0 (space)
        buf.extend_from_slice(&250u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        // Glyph 2: width=700, lsb=0 (A)
        buf.extend_from_slice(&700u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        let hmtx_len = buf.len() - hmtx_offset;

        // ---- cmap table (format 4) ----
        let cmap_offset = buf.len();
        // cmap header
        buf.extend_from_slice(&0u16.to_be_bytes()); // version
        buf.extend_from_slice(&1u16.to_be_bytes()); // numSubtables = 1
        // Encoding record: platform 3 (Windows), encoding 1 (Unicode BMP)
        buf.extend_from_slice(&3u16.to_be_bytes()); // platformID
        buf.extend_from_slice(&1u16.to_be_bytes()); // encodingID
        buf.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable (from start of cmap)

        // Format 4 subtable
        // We'll map: char 32 -> glyph 1, char 65 -> glyph 2
        // Two segments: [32..32], [65..65], plus sentinel [0xFFFF..0xFFFF]
        let seg_count = 3u16; // 2 real + 1 sentinel
        let seg_count_x2 = seg_count * 2;

        buf.extend_from_slice(&4u16.to_be_bytes()); // format = 4
        let subtable_len_pos = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes()); // length (placeholder)
        buf.extend_from_slice(&0u16.to_be_bytes()); // language
        buf.extend_from_slice(&seg_count_x2.to_be_bytes()); // segCountX2
        buf.extend_from_slice(&4u16.to_be_bytes()); // searchRange
        buf.extend_from_slice(&1u16.to_be_bytes()); // entrySelector
        buf.extend_from_slice(&2u16.to_be_bytes()); // rangeShift

        // endCode: 32, 65, 0xFFFF
        buf.extend_from_slice(&32u16.to_be_bytes());
        buf.extend_from_slice(&65u16.to_be_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        buf.extend_from_slice(&0u16.to_be_bytes());
        // startCode: 32, 65, 0xFFFF
        buf.extend_from_slice(&32u16.to_be_bytes());
        buf.extend_from_slice(&65u16.to_be_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta: for char 32 -> glyph 1: delta = 1 - 32 = -31
        //          for char 65 -> glyph 2: delta = 2 - 65 = -63
        //          sentinel: 1
        buf.extend_from_slice(&(-31i16).to_be_bytes());
        buf.extend_from_slice(&(-63i16).to_be_bytes());
        buf.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset: 0, 0, 0
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());

        // Fill in subtable length
        let subtable_start = cmap_offset + 12; // after cmap header + encoding record
        let subtable_len = (buf.len() - subtable_start) as u16;
        buf[subtable_len_pos] = (subtable_len >> 8) as u8;
        buf[subtable_len_pos + 1] = subtable_len as u8;

        let cmap_len = buf.len() - cmap_offset;

        // ---- name table ----
        let name_offset = buf.len();
        // name table header
        buf.extend_from_slice(&0u16.to_be_bytes()); // format
        buf.extend_from_slice(&1u16.to_be_bytes()); // count = 1

        // String storage starts after header + 1 name record = 6 + 12 = 18 bytes from name_offset
        let string_storage_offset = 18u16;
        buf.extend_from_slice(&string_storage_offset.to_be_bytes());

        // Name record: platform 1 (Mac), encoding 0, language 0, nameID 1
        let font_name_str = b"TestFont";
        buf.extend_from_slice(&1u16.to_be_bytes()); // platformID
        buf.extend_from_slice(&0u16.to_be_bytes()); // encodingID
        buf.extend_from_slice(&0u16.to_be_bytes()); // languageID
        buf.extend_from_slice(&1u16.to_be_bytes()); // nameID = 1 (family)
        buf.extend_from_slice(&(font_name_str.len() as u16).to_be_bytes()); // length
        buf.extend_from_slice(&0u16.to_be_bytes()); // offset into string storage

        // String storage
        buf.extend_from_slice(font_name_str);

        let name_len = buf.len() - name_offset;

        // Now fill in the table directory
        let tables_info: [(&[u8; 4], usize, usize); 6] = [
            (b"head", head_offset, head_len),
            (b"hhea", hhea_offset, hhea_len),
            (b"maxp", maxp_offset, maxp_len),
            (b"hmtx", hmtx_offset, hmtx_len),
            (b"cmap", cmap_offset, cmap_len),
            (b"name", name_offset, name_len),
        ];

        for (i, (tag, offset, length)) in tables_info.iter().enumerate() {
            let dir_off = dir_start + i * 16;
            buf[dir_off..dir_off + 4].copy_from_slice(*tag);
            buf[dir_off + 4..dir_off + 8].copy_from_slice(&0u32.to_be_bytes()); // checksum
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(*offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16].copy_from_slice(&(*length as u32).to_be_bytes());
        }

        buf
    }

    /// Wrap copies of the minimal TTF in a TTC, fixing each face's table
    /// offsets from face-relative to collection-relative positions.
    fn build_test_ttc(face_count: usize) -> Vec<u8> {
        let face = build_test_ttf();
        let mut collection = Vec::with_capacity(12 + 4 * face_count + face.len() * face_count);
        collection.extend_from_slice(b"ttcf");
        collection.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        collection.extend_from_slice(&(face_count as u32).to_be_bytes());
        let offsets_start = collection.len();
        collection.resize(offsets_start + 4 * face_count, 0);

        for face_number in 0..face_count {
            while collection.len() % 4 != 0 {
                collection.push(0);
            }
            let face_offset = collection.len();
            collection.extend_from_slice(&face);
            collection[offsets_start + 4 * face_number..offsets_start + 4 * (face_number + 1)]
                .copy_from_slice(&(face_offset as u32).to_be_bytes());

            let table_count = read_u16(&collection, face_offset + 4) as usize;
            for table in 0..table_count {
                let table_offset = face_offset + 12 + table * 16 + 8;
                let relative = read_u32(&collection, table_offset);
                collection[table_offset..table_offset + 4]
                    .copy_from_slice(&(relative + face_offset as u32).to_be_bytes());
            }
        }
        collection
    }

    #[test]
    fn parse_ttf_offset_table() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.units_per_em, 1000);
    }

    #[test]
    fn parse_ttf_with_index_preserves_selected_ttc_face() {
        let font = parse_ttf_with_index(build_test_ttc(2), 1).unwrap();
        assert_eq!(font.face_index, FontFaceIndex::new(1));
        assert_eq!(font.font_name, "TestFont");
    }

    #[test]
    fn parse_ttf_head_bbox() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.bbox, [-100, -200, 800, 900]);
    }

    #[test]
    fn parse_ttf_hhea_ascent_descent() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // No OS/2 table, should fall back to hhea values
        assert_eq!(
            font.pdf_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
        assert_eq!(
            font.layout_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
    }

    #[test]
    fn central_baseline_from_over_uses_signed_face_metrics() {
        let metrics = FontVerticalMetrics::new(1160, -288, 0);
        let ratio = metrics.central_baseline_from_over_ratio(1000);
        assert!((ratio - 0.564).abs() < f32::EPSILON, "ratio={ratio}");

        let metric_set = FontVerticalMetricSet::from(metrics);
        assert!((metric_set.central_baseline_from_over_ratio(1000) - ratio).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_ttf_cmap_format4() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // char 32 (space) -> glyph 1
        assert_eq!(font.cmap.get(&32u32), Some(&1));
        // char 65 (A) -> glyph 2
        assert_eq!(font.cmap.get(&65u32), Some(&2));
        // unmapped char should not exist
        assert_eq!(font.cmap.get(&90u32), None);
    }

    #[test]
    fn parse_ttf_char_widths() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.glyph_widths.len(), 3);
        assert_eq!(font.glyph_widths[0], 500); // notdef
        assert_eq!(font.glyph_widths[1], 250); // space
        assert_eq!(font.glyph_widths[2], 700); // A
    }

    #[test]
    fn parse_ttf_char_width_lookup() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // Space (char 32 -> glyph 1 -> width 250)
        assert_eq!(font.char_width(32), 250);
        // A (char 65 -> glyph 2 -> width 700)
        assert_eq!(font.char_width(65), 700);
        // Unknown char -> glyph 0 -> width 500
        assert_eq!(font.char_width(90), 500);
    }

    #[test]
    fn parse_ttf_font_name() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.font_name, "TestFont");
    }

    #[test]
    fn parse_ttf_char_width_scaled() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // A = 700 units at 1000 upm, font_size = 12
        // width = 700 * 12 / 1000 = 8.4
        let w = font.char_width_scaled(65, 12.0);
        assert!((w - 8.4).abs() < 0.01);
    }

    #[test]
    fn parse_ttf_char_width_pdf() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // A = 700 units at 1000 upm -> 700 * 1000 / 1000 = 700
        assert_eq!(font.char_width_pdf(65), 700);
        // Space = 250 -> 250
        assert_eq!(font.char_width_pdf(32), 250);
    }

    #[test]
    fn parse_ttf_too_short() {
        let data = vec![0; 4];
        assert!(parse_ttf(data).is_err());
    }

    #[test]
    fn parse_ttf_num_h_metrics() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.num_h_metrics, 3);
    }

    // --- char_width fallback paths (lines 39, 41, 43) ---

    #[test]
    fn char_width_glyph_beyond_widths_falls_back_to_last() {
        // Line 39/41: glyph_id >= glyph_widths.len() -> use last entry
        let font = TtfFont {
            font_name: String::new(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0; 4],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(0, 0, 0)),
            cmap: {
                let mut m = HashMap::new();
                m.insert(65, 999); // glyph 999 is beyond widths vec
                m
            },
            glyph_widths: vec![500, 700],
            num_h_metrics: 2,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: FontTextMetrics::default(),
            data: std::sync::Arc::new(vec![]),
        };
        assert_eq!(font.char_width(65), 700); // last width
    }

    #[test]
    fn char_width_empty_glyph_widths_returns_zero() {
        // Line 43: empty glyph_widths -> 0
        let font = TtfFont {
            font_name: String::new(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0; 4],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(0, 0, 0)),
            cmap: HashMap::<u32, u16>::new(),
            glyph_widths: vec![],
            num_h_metrics: 0,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: FontTextMetrics::default(),
            data: std::sync::Arc::new(vec![]),
        };
        assert_eq!(font.char_width(65), 0);
    }

    // --- parse_ttf error paths ---

    #[test]
    fn parse_ttf_table_directory_too_short() {
        // Line 76: data too short for table directory
        // 12 bytes offset table with num_tables=1, but no room for 16-byte table record
        let mut data = vec![0u8; 12];
        data[4] = 0;
        data[5] = 1; // num_tables = 1
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("too short for table directory"));
    }

    /// Helper: build a TTF with a given set of tables at specified sizes.
    /// Each table entry is (tag, data_bytes). Constructs offset table + directory + data.
    fn build_ttf_with_tables(tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let num_tables = tables.len() as u16;
        let mut buf = Vec::new();

        // Offset table (12 bytes)
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift

        let dir_start = buf.len();
        buf.resize(dir_start + tables.len() * 16, 0);

        for (i, (tag, table_data)) in tables.iter().enumerate() {
            let offset = buf.len();
            buf.extend_from_slice(table_data);
            let dir_off = dir_start + i * 16;
            buf[dir_off..dir_off + 4].copy_from_slice(*tag);
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16]
                .copy_from_slice(&(table_data.len() as u32).to_be_bytes());
        }

        buf
    }

    fn make_head_table(units_per_em: u16) -> Vec<u8> {
        let mut t = vec![0u8; 54];
        t[18] = (units_per_em >> 8) as u8;
        t[19] = units_per_em as u8;
        // bbox: zeros is fine
        t
    }

    fn make_hhea_table(ascent: i16, descent: i16, num_h_metrics: u16) -> Vec<u8> {
        let mut t = vec![0u8; 36];
        t[4..6].copy_from_slice(&ascent.to_be_bytes());
        t[6..8].copy_from_slice(&descent.to_be_bytes());
        t[34..36].copy_from_slice(&num_h_metrics.to_be_bytes());
        t
    }

    fn make_maxp_table(num_glyphs: u16) -> Vec<u8> {
        let mut t = vec![0u8; 6];
        t[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
        t
    }

    fn make_hmtx_table(widths: &[u16]) -> Vec<u8> {
        let mut t = Vec::new();
        for &w in widths {
            t.extend_from_slice(&w.to_be_bytes());
            t.extend_from_slice(&0i16.to_be_bytes()); // lsb
        }
        t
    }

    /// Build a simple cmap format 4 with one segment mapping start..=end with given delta.
    fn make_cmap_format4(start: u16, end: u16, delta: i16) -> Vec<u8> {
        let mut t = Vec::new();
        // cmap header
        t.extend_from_slice(&0u16.to_be_bytes()); // version
        t.extend_from_slice(&1u16.to_be_bytes()); // numSubtables
        // platform 3, encoding 1
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable

        // Format 4 subtable
        let seg_count: u16 = 2; // 1 real + sentinel
        t.extend_from_slice(&4u16.to_be_bytes()); // format
        t.extend_from_slice(&0u16.to_be_bytes()); // length (unused by parser)
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&(seg_count * 2).to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes()); // searchRange
        t.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        t.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

        // endCode
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        t.extend_from_slice(&0u16.to_be_bytes());
        // startCode
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta
        t.extend_from_slice(&delta.to_be_bytes());
        t.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());

        t
    }

    /// Build a cmap with format 4 that uses idRangeOffset (non-zero).
    fn make_cmap_format4_with_range_offset(start: u16, end: u16, glyph_ids: &[u16]) -> Vec<u8> {
        let mut t = Vec::new();
        // cmap header
        t.extend_from_slice(&0u16.to_be_bytes()); // version
        t.extend_from_slice(&1u16.to_be_bytes()); // numSubtables
        t.extend_from_slice(&3u16.to_be_bytes()); // platform 3
        t.extend_from_slice(&1u16.to_be_bytes()); // encoding 1
        t.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable

        // Format 4 subtable
        let seg_count: u16 = 2; // 1 real + sentinel
        t.extend_from_slice(&4u16.to_be_bytes()); // format
        t.extend_from_slice(&0u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&(seg_count * 2).to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());

        // endCode
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        t.extend_from_slice(&0u16.to_be_bytes());
        // startCode
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta: 0 for both segments
        t.extend_from_slice(&0i16.to_be_bytes());
        t.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset: for segment 0, points to glyphIdArray right after this array
        // The offset is relative to the current position in idRangeOffset array.
        // idRangeOffset[0] is at position P. glyphIdArray starts at P + 4 (2 entries * 2 bytes).
        // So idRangeOffset = 4.
        let range_offset = seg_count * 2; // = 4
        t.extend_from_slice(&range_offset.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // sentinel

        // glyphIdArray
        for &gid in glyph_ids {
            t.extend_from_slice(&gid.to_be_bytes());
        }

        t
    }

    fn make_name_table_ascii(name_id: u16, name: &[u8]) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&1u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 12; // header + 1 record
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // record: platform 1 (Mac), encoding 0, language 0
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&name_id.to_be_bytes());
        t.extend_from_slice(&(name.len() as u16).to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // offset into storage
        t.extend_from_slice(name);
        t
    }

    fn make_name_table_utf16be(
        name_id: u16,
        platform_id: u16,
        encoding_id: u16,
        name: &str,
    ) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&1u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 12;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // record
        t.extend_from_slice(&platform_id.to_be_bytes());
        t.extend_from_slice(&encoding_id.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&name_id.to_be_bytes());
        let name_bytes: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        t.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // offset
        t.extend_from_slice(&name_bytes);
        t
    }

    /// Build a complete valid TTF from individual table data vectors.
    fn build_full_ttf(
        head: &[u8],
        hhea: &[u8],
        maxp: &[u8],
        hmtx: &[u8],
        cmap: &[u8],
        name: &[u8],
        os2: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut table_list: Vec<(&[u8; 4], &[u8])> = vec![
            (b"head", head),
            (b"hhea", hhea),
            (b"maxp", maxp),
            (b"hmtx", hmtx),
            (b"cmap", cmap),
            (b"name", name),
        ];
        if let Some(os2_data) = os2 {
            table_list.push((b"OS/2", os2_data));
        }
        build_ttf_with_tables(&table_list)
    }

    #[test]
    fn parse_ttf_head_too_short() {
        // Line 98
        let data = build_ttf_with_tables(&[(b"head", &[0u8; 10])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("head table too short") || err.contains("Missing"));
    }

    #[test]
    fn parse_ttf_missing_hhea() {
        let head = make_head_table(1000);
        let data = build_ttf_with_tables(&[(b"head", &head)]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("Missing hhea"));
    }

    #[test]
    fn parse_ttf_hhea_too_short() {
        // Line 111
        let head = make_head_table(1000);
        let data = build_ttf_with_tables(&[(b"head", &head), (b"hhea", &[0u8; 10])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("hhea table too short"));
    }

    #[test]
    fn parse_ttf_uses_os2_typographic_metrics_for_layout() {
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64); // char 65 -> glyph 1
        let name = make_name_table_ascii(1, b"Test");

        let mut os2 = vec![0u8; 78];
        // sTypoAscender at offset 68
        os2[68..70].copy_from_slice(&900i16.to_be_bytes());
        // sTypoDescender at offset 70
        os2[70..72].copy_from_slice(&(-300i16).to_be_bytes());
        // sTypoLineGap at offset 72
        os2[72..74].copy_from_slice(&50i16.to_be_bytes());
        // usWinAscent at offset 74
        os2[74..76].copy_from_slice(&950u16.to_be_bytes());
        // usWinDescent at offset 76
        os2[76..78].copy_from_slice(&350u16.to_be_bytes());

        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();
        assert_eq!(
            font.pdf_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
        // layout_metrics uses usWinAscent/usWinDescent for Chrome parity
        assert_eq!(
            font.layout_vertical_metrics(),
            FontVerticalMetrics::new(950, -350, 0)
        );
        assert_eq!(
            font.typographic_vertical_metrics(),
            FontVerticalMetrics::new(900, -300, 50)
        );
    }

    #[test]
    fn parse_ttf_uses_os2_cap_height_for_initial_letters() {
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        let mut os2 = vec![0u8; 90];
        os2[..2].copy_from_slice(&2u16.to_be_bytes());
        os2[88..90].copy_from_slice(&731i16.to_be_bytes());

        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();

        assert_eq!(font.text_metrics.cap_height, 731);
        assert!((font.cap_height_ratio() - 0.731).abs() < 1e-6);
    }

    #[test]
    fn parse_ttf_uses_os2_x_height_as_a_linear_metric() {
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        let mut os2 = vec![0u8; 90];
        os2[..2].copy_from_slice(&2u16.to_be_bytes());
        os2[86..88].copy_from_slice(&537i16.to_be_bytes());

        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();

        assert_eq!(font.text_metrics.x_height, FontXHeightMetric::OpenType(537));
        assert!((font.used_x_height(20.0) - 10.74).abs() < 1e-6);
    }

    #[test]
    fn parse_ttf_os2_table_too_short_falls_back_to_hhea() {
        // Line 125: OS/2 present but too short
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");

        let os2 = vec![0u8; 10]; // too short for ascent/descent fields
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();
        assert_eq!(
            font.pdf_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
        assert_eq!(
            font.layout_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
        assert_eq!(
            font.typographic_vertical_metrics(),
            FontVerticalMetrics::new(800, -200, 0)
        );
    }

    #[test]
    fn parse_ttf_maxp_too_short() {
        // Line 135
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let data =
            build_ttf_with_tables(&[(b"head", &head), (b"hhea", &hhea), (b"maxp", &[0u8; 2])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("maxp table too short"));
    }

    #[test]
    fn parse_ttf_hmtx_break_on_short_data() {
        // Line 148: hmtx data cut short mid-entry.
        // We need hmtx to be the LAST table in the buffer, and truncate
        // the buffer so that reading the 2nd entry fails.
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 3); // claims 3 h_metrics
        let maxp = make_maxp_table(3); // claims 3 glyphs
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        // Only 1 hmtx entry (4 bytes): width=500, lsb=0
        let hmtx = make_hmtx_table(&[500]);

        // Build with hmtx LAST so we can truncate.
        // We build manually to ensure hmtx is at the end.
        let tables: Vec<(&[u8; 4], &[u8])> = vec![
            (b"head", &head),
            (b"hhea", &hhea),
            (b"maxp", &maxp),
            (b"cmap", &cmap),
            (b"name", &name),
            (b"hmtx", &hmtx),
        ];
        let mut data = build_ttf_with_tables(&tables);
        // hmtx is 4 bytes at the end. The parser tries entry_off = hmtx_off + 1*4
        // for i=1, which is hmtx_off + 4 = end of buffer. data.len() < entry_off + 2
        // is data.len() < data.len() + 2 => true => break.
        let font = parse_ttf(data.clone()).unwrap();
        assert_eq!(font.glyph_widths.len(), 1);
        assert_eq!(font.glyph_widths[0], 500);

        // Also test with no hmtx data at all by truncating further
        // to exercise the break at i=0.
        let hmtx_off = data.len() - 4;
        data.truncate(hmtx_off); // remove all hmtx data
        // But we need to keep hmtx table record pointing to the (now past-end) offset.
        // Actually hmtx_off now equals data.len(), so entry_off for i=0 = hmtx_off + 0 = data.len()
        // and data.len() < data.len() + 2 is true => break immediately => 0 widths.
        let font2 = parse_ttf(data).unwrap();
        assert!(font2.glyph_widths.is_empty());
    }

    #[test]
    fn parse_cmap_table_too_short() {
        // Line 186
        let result = parse_cmap(&[0u8; 2], 0);
        assert!(result.unwrap_err().contains("cmap table too short"));
    }

    #[test]
    fn parse_cmap_subtable_record_break() {
        // Line 196: subtable record truncated
        // cmap header says 2 subtables but data only has room for partial second
        let mut data = [0u8; 100];
        // offset 0: version=0, numSubtables=2
        data[2] = 0;
        data[3] = 2;
        // First record at offset 4: platform 1 (not matching), 8 bytes
        // We need the data to be short enough that the second record is truncated
        let _result = parse_cmap(&data[..11], 0);
        // Should find no suitable subtable (first one at platform 0 would match but
        // second record breaks)
        // Actually platform_id=0 matches, so let's set first to non-matching
        let mut data2 = [0u8; 20];
        data2[3] = 2; // 2 subtables
        // First record: platform 5 (no match)
        data2[4] = 0;
        data2[5] = 5;
        // Second record would be at offset 12, but we cut data short
        let result2 = parse_cmap(&data2[..15], 0);
        assert!(result2.unwrap_err().contains("No suitable cmap subtable"));
    }

    #[test]
    fn parse_cmap_subtable_too_short() {
        // Line 213: subtable offset valid but data too short to read format
        let mut data = [0u8; 20];
        data[3] = 1; // 1 subtable
        // platform 3, encoding 1
        data[4] = 0;
        data[5] = 3;
        data[6] = 0;
        data[7] = 1;
        // subtable offset pointing near end of data
        let sub_off = 18u32; // points to offset 0 + 18 = 18
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // data.len() = 20, sub_off=18, need sub_off+2=20 which equals len -> not <, so it's ok
        // Make it 19 so sub_off + 2 > len
        let result = parse_cmap(&data[..19], 0);
        assert!(result.unwrap_err().contains("cmap subtable too short"));
    }

    #[test]
    fn parse_cmap_unsupported_format() {
        // Line 222: unsupported format returns empty map
        let mut data = vec![0u8; 30];
        data[3] = 1; // 1 subtable
        data[5] = 3; // platform 3
        data[7] = 1; // encoding 1
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // At offset 12: format = 6 (unsupported)
        data[12] = 0;
        data[13] = 6;
        let result = parse_cmap(&data, 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_cmap_format0() {
        // Lines 228-239: format 0 parsing
        let mut data = vec![0u8; 300];
        data[3] = 1; // 1 subtable
        data[5] = 3; // platform 3
        data[7] = 1; // encoding 1
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // At offset 12: format = 0
        data[12] = 0;
        data[13] = 0;
        // format 0: 6 bytes header + 256 bytes glyph array = 262 total
        // glyph array starts at offset 12 + 6 = 18
        data[18 + 65] = 5; // char 65 ('A') -> glyph 5
        data[18 + 66] = 6; // char 66 ('B') -> glyph 6
        // char 0 -> glyph 0 (should not be inserted since glyph_id=0)
        let result = parse_cmap(&data, 0).unwrap();
        assert_eq!(result.get(&65), Some(&5));
        assert_eq!(result.get(&66), Some(&6));
        assert_eq!(result.get(&0), None); // glyph 0 skipped
    }

    #[test]
    fn parse_cmap_format0_too_short() {
        // Line 229-230
        let mut data = vec![0u8; 100];
        data[3] = 1;
        data[5] = 3;
        data[7] = 1;
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        data[12] = 0;
        data[13] = 0; // format 0
        // data is only 100 bytes, need offset+262 = 274
        let result = parse_cmap(&data, 0);
        assert!(
            result
                .unwrap_err()
                .contains("cmap format 0 table too short")
        );
    }

    #[test]
    fn parse_cmap_format4_header_too_short() {
        // Line 245
        let result = parse_cmap_format4(&[0u8; 10], 0);
        assert!(
            result
                .unwrap_err()
                .contains("cmap format 4 header too short")
        );
    }

    #[test]
    fn parse_cmap_format4_data_too_short() {
        // Line 259: header ok but segment data truncated
        let mut data = vec![0u8; 20];
        // format 4, segCountX2 = 4 (2 segments)
        data[0] = 0;
        data[1] = 4;
        data[6] = 0;
        data[7] = 4; // segCountX2 = 4
        // Need: offset+14 + seg_count*2*4 + seg_count*2 + 2 = lots, but we only have 20
        let result = parse_cmap_format4(&data, 0);
        assert!(result.unwrap_err().contains("cmap format 4 data too short"));
    }

    #[test]
    fn parse_cmap_format4_with_id_range_offset() {
        // Lines 280-288: idRangeOffset != 0 path
        let cmap_data = make_cmap_format4_with_range_offset(65, 67, &[10, 20, 0]);
        // Parse just the cmap
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 21);
        let maxp = make_maxp_table(21);
        let hmtx = make_hmtx_table(&[
            500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500,
            500, 500, 500, 500,
        ]);
        let name = make_name_table_ascii(1, b"Test");
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap_data, &name, None);
        let font = parse_ttf(data).unwrap();
        // char 65 -> glyph 10, char 66 -> glyph 20, char 67 -> glyph 0 (not inserted)
        assert_eq!(font.cmap.get(&65u32), Some(&10));
        assert_eq!(font.cmap.get(&66u32), Some(&20));
        assert_eq!(font.cmap.get(&67u32), None); // glyph_id 0 not inserted
    }

    #[test]
    fn parse_cmap_format4_id_range_offset_out_of_bounds() {
        // Line 291: glyph_off + 1 >= data.len() -> returns 0
        // Build a cmap format 4 with idRangeOffset pointing beyond data
        let mut data = vec![0u8; 50];
        // format 4
        data[0] = 0;
        data[1] = 4;
        // segCountX2 = 4 (2 segments: 1 real + sentinel)
        data[6] = 0;
        data[7] = 4;
        let seg_count = 2usize;
        // endCode at offset 14
        let end_code_off = 14;
        data[end_code_off] = 0;
        data[end_code_off + 1] = 65; // endCode[0] = 65
        data[end_code_off + 2] = 0xFF;
        data[end_code_off + 3] = 0xFF; // sentinel
        // reserved pad at end_code_off + 4
        let start_code_off = end_code_off + seg_count * 2 + 2;
        data[start_code_off] = 0;
        data[start_code_off + 1] = 65; // startCode[0] = 65
        data[start_code_off + 2] = 0xFF;
        data[start_code_off + 3] = 0xFF;
        let id_delta_off = start_code_off + seg_count * 2;
        // idDelta = 0
        let id_range_off = id_delta_off + seg_count * 2;
        // idRangeOffset = huge value so glyph_off goes out of bounds
        data[id_range_off] = 0xFF;
        data[id_range_off + 1] = 0xFE;
        // sentinel idRangeOffset = 0
        let result = parse_cmap_format4(&data, 0).unwrap();
        // char 65 should not be in map (glyph_id = 0 from out of bounds)
        assert_eq!(result.get(&65), None);
    }

    #[test]
    fn parse_name_table_too_short() {
        // Line 306
        let result = parse_name_table(&[0u8; 4], 0);
        assert!(result.unwrap_err().contains("name table too short"));
    }

    #[test]
    fn parse_name_table_record_break() {
        // Line 320: record data truncated
        let mut data = vec![0u8; 12];
        data[3] = 2; // count = 2
        data[5] = 100; // string storage offset (large)
        // Only room for partial first record, second record will be cut
        let result = parse_name_table(&data, 0);
        assert!(result.unwrap_err().contains("No font name found"));
    }

    #[test]
    fn parse_name_table_skips_non_name_ids() {
        // Line 329: name_id != 1 && name_id != 4 -> continue
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&2u16.to_be_bytes()); // count = 2
        let storage_offset: u16 = 6 + 24; // header + 2 records
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record 0: nameID=2 (should skip)
        t.extend_from_slice(&1u16.to_be_bytes()); // platform
        t.extend_from_slice(&0u16.to_be_bytes()); // encoding
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&2u16.to_be_bytes()); // nameID=2
        t.extend_from_slice(&4u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes()); // offset
        // Record 1: nameID=1 (should use)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // nameID=1
        t.extend_from_slice(&4u16.to_be_bytes());
        t.extend_from_slice(&4u16.to_be_bytes()); // offset=4 (after "Skip")
        // Storage
        t.extend_from_slice(b"SkipGood");
        let result = parse_name_table(&t, 0).unwrap();
        assert_eq!(result, "Good");
    }

    #[test]
    fn parse_name_table_priority_name_id_4_over_1() {
        // Line 334: priority <= best_priority -> continue (nameID 4 > nameID 1)
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&2u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 24;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record 0: nameID=4 (full name, priority 2)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&4u16.to_be_bytes()); // nameID=4
        t.extend_from_slice(&8u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes());
        // Record 1: nameID=1 (family, priority 1 - should be skipped)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // nameID=1
        t.extend_from_slice(&6u16.to_be_bytes());
        t.extend_from_slice(&8u16.to_be_bytes());
        // Storage
        t.extend_from_slice(b"FullNameFamily");
        let result = parse_name_table(&t, 0).unwrap();
        assert_eq!(result, "FullName");
    }

    #[test]
    fn parse_name_table_string_beyond_data() {
        // Line 340: end > data.len() -> continue
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // count=1
        let storage_offset: u16 = 6 + 12;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record: nameID=1, but string extends beyond data
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&100u16.to_be_bytes()); // length=100 (way beyond)
        t.extend_from_slice(&0u16.to_be_bytes());
        // Only 2 bytes of storage
        t.extend_from_slice(b"AB");
        let result = parse_name_table(&t, 0);
        assert!(result.unwrap_err().contains("No font name found"));
    }

    #[test]
    fn parse_name_table_utf16be_windows_platform() {
        // Lines 346, 362-372: platform 3 triggers UTF-16BE decoding
        let name = make_name_table_utf16be(1, 3, 1, "Hello");
        let result = parse_name_table(&name, 0).unwrap();
        assert_eq!(result, "Hello");
    }

    #[test]
    fn parse_name_table_utf16be_unicode_platform_encoding_gt_0() {
        // Line 346: platform 0, encoding > 0 also triggers UTF-16BE
        let name = make_name_table_utf16be(1, 0, 1, "World");
        let result = parse_name_table(&name, 0).unwrap();
        assert_eq!(result, "World");
    }

    #[test]
    fn decode_utf16be_basic() {
        // Lines 362-372
        let data = [0x00, 0x48, 0x00, 0x69]; // "Hi"
        let result = decode_utf16be(&data);
        assert_eq!(result, "Hi");
    }

    #[test]
    fn decode_utf16be_empty() {
        let result = decode_utf16be(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn decode_utf16be_odd_byte_ignored() {
        // Line 365: while i + 1 < data.len() - odd trailing byte ignored
        let data = [0x00, 0x41, 0xFF]; // 'A' + trailing byte
        let result = decode_utf16be(&data);
        assert_eq!(result, "A");
    }

    #[test]
    fn parse_cmap_unicode_platform_fallback() {
        // Line 196 + platform 0 selection: platform 0 (Unicode) is accepted
        let mut data = vec![0u8; 300];
        data[3] = 1; // 1 subtable
        // platform 0, encoding 0
        data[4] = 0;
        data[5] = 0;
        data[6] = 0;
        data[7] = 0;
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // format 0 subtable at offset 12
        data[12] = 0;
        data[13] = 0;
        data[18 + 65] = 3; // char 65 -> glyph 3
        let result = parse_cmap(&data, 0).unwrap();
        assert_eq!(result.get(&65), Some(&3));
    }

    #[test]
    fn parse_ttf_rejects_zero_units_per_em() {
        let head = make_head_table(0); // units_per_em = 0
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, None);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("units_per_em"));
    }

    #[test]
    fn char_width_pdf_large_width_no_overflow() {
        let font = TtfFont {
            font_name: String::new(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0; 4],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(0, 0, 0)),
            cmap: {
                let mut m = HashMap::new();
                m.insert(65, 0);
                m
            },
            glyph_widths: vec![u16::MAX], // 65535 — would overflow u32 with * 1000
            num_h_metrics: 1,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: FontTextMetrics::default(),
            data: std::sync::Arc::new(vec![]),
        };
        // Should not panic; 65535 * 1000 / 1000 = 65535
        let w = font.char_width_pdf(65);
        assert_eq!(w, 65535);
    }

    #[test]
    fn char_width_scaled_zero_upm_returns_zero() {
        let font = TtfFont {
            font_name: String::new(),
            face_index: Default::default(),
            units_per_em: 0,
            size_adjust: 1.0,
            bbox: [0; 4],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(0, 0, 0)),
            cmap: {
                let mut m = HashMap::new();
                m.insert(65, 0);
                m
            },
            glyph_widths: vec![500],
            num_h_metrics: 1,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: FontTextMetrics::default(),
            data: std::sync::Arc::new(vec![]),
        };
        assert_eq!(font.char_width_scaled(65, 12.0), 0.0);
        assert_eq!(font.char_width_pdf(65), 0);
    }

    fn metrics_font(
        units_per_em: u16,
        x_height: u16,
        cap_height: u16,
        zero_advance: u16,
    ) -> TtfFont {
        TtfFont {
            font_name: String::new(),
            face_index: Default::default(),
            units_per_em,
            size_adjust: 1.0,
            bbox: [0; 4],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(0, 0, 0)),
            cmap: HashMap::new(),
            glyph_widths: Vec::new(),
            num_h_metrics: 0,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: FontTextMetrics {
                x_height: FontXHeightMetric::open_type(x_height),
                cap_height,
                zero_advance,
            },
            data: std::sync::Arc::new(vec![]),
        }
    }

    #[test]
    fn x_height_ratio_uses_metric_when_present() {
        let font = metrics_font(2048, 1120, 0, 0);
        assert!((font.x_height_ratio() - 1120.0 / 2048.0).abs() < 1e-6);
    }

    #[test]
    fn font_size_adjust_uses_the_css_x_height_metric() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySerif.ttf"),
        )
        .expect("ParitySerif test font");
        let font = parse_ttf(bytes).expect("valid ParitySerif TTF");
        let font_size = 30.0 * crate::fonts::PT_PER_CSS_PX;

        // Current Chromium/Skia Fontations exposes a 16px hinted contour
        // height at 30 CSS px for this face, which has no OS/2.sxHeight.
        assert!((font.font_size_adjust_x_height_ratio(font_size) - 16.0 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn x_height_ratio_falls_back_to_half_em() {
        // No x-height metric -> css-values-4 fallback of 0.5em.
        assert_eq!(metrics_font(2048, 0, 0, 0).x_height_ratio(), 0.5);
        assert_eq!(metrics_font(0, 1120, 0, 0).x_height_ratio(), 0.5);
    }

    #[test]
    fn ch_ratio_uses_zero_advance_when_present() {
        let font = metrics_font(2048, 0, 0, 1024);
        assert!((font.ch_ratio() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn ch_ratio_falls_back_to_half_em() {
        assert_eq!(metrics_font(2048, 0, 0, 0).ch_ratio(), 0.5);
    }

    #[test]
    fn cap_height_ratio_uses_metric_then_css_fallback() {
        assert!((metrics_font(2048, 0, 1493, 0).cap_height_ratio() - 1493.0 / 2048.0).abs() < 1e-6);
        assert_eq!(metrics_font(2048, 0, 0, 0).cap_height_ratio(), 0.66);
    }

    #[test]
    fn parse_ttf_glyphs_beyond_num_h_metrics_share_last_width() {
        // Line 153-155: glyphs beyond num_h_metrics share last advance width
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 2); // only 2 h metrics
        let maxp = make_maxp_table(4); // but 4 glyphs
        // hmtx: only 2 long entries
        let hmtx = make_hmtx_table(&[500, 700]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, None);
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.glyph_widths.len(), 4);
        assert_eq!(font.glyph_widths[0], 500);
        assert_eq!(font.glyph_widths[1], 700);
        assert_eq!(font.glyph_widths[2], 700); // shares last width
        assert_eq!(font.glyph_widths[3], 700); // shares last width
    }
}
