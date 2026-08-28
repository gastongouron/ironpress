use crate::layout::engine::{TextRun, TextShaping};
use crate::parser::ttf::{FontVerticalMetricSet, FontVerticalMetrics, TtfFont};
use crate::style::computed::FontFamily;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

mod run_coalescing;
pub(crate) use run_coalescing::{coalesce_text_runs, text_runs_share_shaping_buffer};

#[derive(Debug, Clone)]
pub(crate) struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub unicode: Vec<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

/// A run shaped in the font's top-to-bottom direction.
///
/// Its glyph positions use vertical OpenType metrics (`vmtx`/`VORG`) and
/// vertical substitutions.  It deliberately does not expose a horizontal
/// width: callers use it only to paint one upright vertical typographic unit.
#[derive(Debug, Clone)]
pub(crate) struct VerticalShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
}

/// Typographic metrics together with the unit scale of the resolved font.
#[derive(Clone, Copy)]
pub(crate) struct UprightVerticalFontMetrics {
    line_metrics: FontVerticalMetrics,
    central_baseline_metrics: FontVerticalMetricSet,
    units_per_em: u16,
}

impl UprightVerticalFontMetrics {
    pub(crate) fn line_ratios(self) -> (f32, f32) {
        (
            self.line_metrics.ascender_ratio(self.units_per_em),
            self.line_metrics.descender_ratio(self.units_per_em),
        )
    }

    pub(crate) fn central_baseline_from_over_ratio(self) -> f32 {
        self.central_baseline_metrics
            .central_baseline_from_over_ratio(self.units_per_em)
    }
}

/// The resolved face and vertical shaping result for one upright run.
///
/// This keeps font fallback and paint on one exact face, which matters for CJK
/// vertical origin metrics as well as glyph coverage.
pub(crate) struct VerticalShapedFontRun<'a> {
    pub font_key: &'a str,
    pub shaped: VerticalShapedRun,
}

pub(crate) fn resolve_custom_font<'a>(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(&'a str, &'a TtfFont)> {
    let FontFamily::Custom(name) = font_family else {
        return None;
    };

    crate::system_fonts::find_font(fonts, name, bold, italic)
}

pub(crate) fn measure_text_width(
    text: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    measure_text_width_with_shaping(
        text,
        font_size,
        font_family,
        bold,
        italic,
        TextShaping::default(),
        fonts,
    )
}

/// Measure text with the exact OpenType feature selection carried by a run.
///
/// Layout and PDF painting call this shared entry point so a disabled kerning
/// or ligature feature cannot make wrapping disagree with the painted advance.
pub(crate) fn measure_text_width_with_shaping(
    text: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    shaping: TextShaping,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    if let Some(width) = measure_text_width_by_font_face_ranges(
        text,
        font_size,
        font_family,
        bold,
        italic,
        shaping,
        fonts,
    ) {
        return Some(width);
    }
    if let Some((_, font)) = resolve_custom_font(font_family, bold, italic, fonts) {
        let shaped = shape_text_with_font(text, font_size, font, shaping)?;
        if shaped_has_no_missing_glyphs(&shaped) {
            return Some(shaped.width);
        }
    } else if crate::render::pdf::is_winansi_encodable(text) {
        return None;
    }

    shape_with_fallback_font(
        text,
        font_size,
        shaping,
        crate::font_pack::FontLocale::Unspecified,
        fonts,
    )
    .map(|(shaped, _, _)| shaped.width)
}

pub(crate) fn custom_font_line_height(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let (_, font) = resolve_custom_font(font_family, bold, italic, fonts)?;
    Some(
        font.layout_vertical_metrics()
            .line_height_ratio(font.units_per_em),
    )
}

pub(crate) fn shape_text_run(run: &TextRun, fonts: &HashMap<String, TtfFont>) -> Option<ShapedRun> {
    let (_, font) = resolve_custom_font(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        fonts,
    )?;
    shape_text_with_font(&run.text, run.font_size, font, run.shaping)
}

/// Shape one already-expanded upright vertical run using the exact custom face
/// that ordinary painting would select.
///
/// `upright_lines` splits regular vertical text into individual typographic
/// units, so this intentionally handles one resolved run at a time. Horizontal
/// `text-combine-upright` is painted through its own path and never reaches
/// here.
pub(crate) fn shape_upright_vertical_run<'a>(
    run: &TextRun,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<VerticalShapedFontRun<'a>> {
    let (font_key, font) = resolve_upright_vertical_font(run, fonts)?;
    let shaped = shape_vertical_text_with_font(&run.text, run.font_size, font, run.shaping)?;
    shaped
        .glyphs
        .iter()
        .all(|glyph| glyph.glyph_id != 0)
        .then_some(VerticalShapedFontRun { font_key, shaped })
}

/// Metrics of the exact face selected for an upright vertical run.
///
/// A CJK fallback does not share the primary Latin face's typographic metrics,
/// so callers must not substitute the nominal `font-family` when calculating
/// the line geometry that surrounds its vertical glyphs.
pub(crate) fn upright_vertical_font_metrics(
    run: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> Option<UprightVerticalFontMetrics> {
    resolve_upright_vertical_font(run, fonts).map(|(_, font)| UprightVerticalFontMetrics {
        line_metrics: font.typographic_vertical_metrics(),
        central_baseline_metrics: font.vertical_metrics,
        units_per_em: font.units_per_em,
    })
}

/// Resolve the one face used by a vertical typographic unit without shaping it.
///
/// Upright layout expands ordinary text into individual units, so a single face
/// must cover every character in `run`. This mirrors the paint-time fallback
/// order while allowing line geometry to use the same metrics before painting.
fn resolve_upright_vertical_font<'a>(
    run: &TextRun,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(&'a str, &'a TtfFont)> {
    if !needs_unicode_fallback(run, fonts) {
        return resolve_custom_font(
            &run.font_family,
            run.bold,
            run.font_style.is_slanted(),
            fonts,
        );
    }

    if let FontFamily::Custom(name) = &run.font_family {
        if let Some((key, font)) =
            font_face_range_fonts(fonts, name, run.bold, run.font_style.is_slanted())
                .into_iter()
                .find(|(_, font)| font_covers_text(font, &run.text))
        {
            return Some((key, font));
        }
    }

    crate::font_pack::fallback_keys(run.metadata.font_locale)
        .into_iter()
        .find_map(|key| {
            fonts
                .get_key_value(key)
                .filter(|(_, font)| font_covers_text(font, &run.text))
                .map(|(key, font)| (key.as_str(), font))
        })
}

fn font_covers_text(font: &TtfFont, text: &str) -> bool {
    text.chars().all(|ch| {
        font.cmap
            .get(&(ch as u32))
            .is_some_and(|glyph_id| *glyph_id != 0)
    })
}

/// Faces authored for one resolved CSS family and style.
///
/// Preparing this once keeps grapheme-level fallback from repeatedly scanning
/// the registry. A family may contain several `@font-face` rules partitioned by
/// `unicode-range`; every such face retains priority over optional packs.
pub(crate) struct AuthoredFontFaces<'a> {
    /// Custom faces, or `None` when coverage follows PDF WinAnsi encoding.
    custom: Option<Vec<&'a TtfFont>>,
}

impl<'a> AuthoredFontFaces<'a> {
    /// Resolve all authored faces that may serve the requested family variant.
    pub(crate) fn resolve(
        font_family: &FontFamily,
        bold: bool,
        italic: bool,
        fonts: &'a HashMap<String, TtfFont>,
    ) -> Self {
        let FontFamily::Custom(name) = font_family else {
            return Self { custom: None };
        };

        let mut custom = Vec::new();
        if let Some((_, primary)) = resolve_custom_font(font_family, bold, italic, fonts) {
            custom.push(primary);
        }
        custom.extend(
            font_face_range_fonts(fonts, name, bold, italic)
                .into_iter()
                .map(|(_, font)| font),
        );
        Self {
            custom: Some(custom),
        }
    }

    /// Return whether one authored face covers the complete text unit.
    pub(crate) fn covers(&self, text: &str) -> bool {
        self.custom.as_ref().map_or_else(
            || crate::render::pdf::is_winansi_encodable(text),
            |fonts| fonts.iter().any(|font| font_covers_text(font, text)),
        )
    }
}

/// Shape arbitrary text with an explicit `TtfFont` face.
///
/// Used by the SVG `<text>` renderer, which resolves its own face (via
/// `find_font`) rather than going through a layout `TextRun`.
pub(crate) fn shape_text_with_explicit_font(
    text: &str,
    font_size: f32,
    font: &TtfFont,
) -> Option<ShapedRun> {
    shape_text_with_font(text, font_size, font, TextShaping::default())
}

/// Pair-positioning advance retained across separately painted inline runs.
///
/// CSS Text requires shaping to continue when a boundary has no effective
/// glyph-formatting change, and asks engines to preserve feasible shaping for
/// other boundaries. We retain only the two-glyph kerning adjustment here:
/// the glyphs remain in their individual runs, so a ligature can never cross a
/// colour, background, or pseudo-element boundary.
pub(crate) fn inline_boundary_kerning_advance(
    left: &TextRun,
    right: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if !left.shaping.kerning
        || !right.shaping.kerning
        || left.shaping != right.shaping
        || left.font_family != right.font_family
        || left.font_size != right.font_size
        || left.bold != right.bold
        || left.font_style != right.font_style
        || left.font_synthesis != right.font_synthesis
        || left.font_variant_position != right.font_variant_position
        || left.metadata.spacing.letter != 0.0
        || right.metadata.spacing.letter != 0.0
        || left.padding != crate::types::EdgeSizes::ZERO
        || right.padding != crate::types::EdgeSizes::ZERO
        || !matches!(
            left.vertical_align,
            crate::style::computed::VerticalAlign::Baseline
        )
        || !matches!(
            right.vertical_align,
            crate::style::computed::VerticalAlign::Baseline
        )
        || left.inline_box.is_some()
        || right.inline_box.is_some()
    {
        return 0.0;
    }

    let (Some(left_char), Some(right_char)) =
        (left.text.chars().next_back(), right.text.chars().next())
    else {
        return 0.0;
    };
    if left_char.is_whitespace() || right_char.is_whitespace() {
        return 0.0;
    }

    let (Some((_, left_font)), Some((_, right_font))) = (
        resolve_custom_font(
            &left.font_family,
            left.bold,
            left.font_style.is_slanted(),
            fonts,
        ),
        resolve_custom_font(
            &right.font_family,
            right.bold,
            right.font_style.is_slanted(),
            fonts,
        ),
    ) else {
        return 0.0;
    };
    if !std::ptr::eq(left_font, right_font) {
        return 0.0;
    }

    let left_text = left_char.to_string();
    let right_text = right_char.to_string();
    let pair_text = format!("{left_text}{right_text}");
    let shaping = TextShaping::KERNING_ONLY;
    let Some(left_width) = shape_text_with_font(&left_text, left.font_size, left_font, shaping)
        .map(|shaped| shaped.width)
    else {
        return 0.0;
    };
    let Some(right_width) = shape_text_with_font(&right_text, right.font_size, right_font, shaping)
        .map(|shaped| shaped.width)
    else {
        return 0.0;
    };
    let Some(pair_width) = shape_text_with_font(&pair_text, left.font_size, left_font, shaping)
        .map(|shaped| shaped.width)
    else {
        return 0.0;
    };
    let advance = pair_width - left_width - right_width;
    if advance.is_finite() {
        advance
    } else {
        Default::default()
    }
}

/// Try to shape `run` with the Unicode fallback font.
///
/// Returns `Some((shaped_run, font_key))` when the run uses a standard PDF font,
/// contains non-WinAnsi characters, and the fallback font is loaded and can shape
/// the text.  The returned `font_key` is the key into the custom fonts map.
pub(crate) fn shape_with_unicode_fallback<'a>(
    run: &TextRun,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(ShapedRun, &'a str, &'a TtfFont)> {
    if let Some((shaped_run, font_key, font)) = shape_with_font_face_range(run, fonts) {
        return Some((shaped_run, font_key, font));
    }

    // For standard PDF fonts, fall back when text has non-WinAnsi characters.
    // For custom fonts (including bundled Liberation), fall back when the
    // primary font cannot shape the text (missing glyphs for CJK, Arabic, etc.).
    if matches!(run.font_family, FontFamily::Custom(_)) {
        // Check if all characters in the run have glyphs in the primary font's
        // cmap table. If any character is missing, fall back to the unicode font.
        let all_covered = if let Some((_, primary_font)) = crate::system_fonts::find_font(
            fonts,
            run.font_family.name(),
            run.bold,
            run.font_style.is_slanted(),
        ) {
            run.text.chars().all(|ch| {
                let cp = ch as u32;
                primary_font.cmap.contains_key(&cp)
            })
        } else {
            false
        };
        if all_covered {
            return None;
        }
        // Font doesn't cover all characters — try unicode fallback
    } else if crate::render::pdf::is_winansi_encodable(&run.text) {
        return None;
    }
    shape_with_fallback_font(
        &run.text,
        run.font_size,
        run.shaping,
        run.metadata.font_locale,
        fonts,
    )
}

/// Shape text through the registered fallback chain, accepting only a face that
/// resolves every glyph. Width calculation and PDF painting must make the same
/// choice: measuring a primary face's `.notdef` while painting a CJK fallback
/// mis-centres inline content.
fn shape_with_fallback_font<'a>(
    text: &str,
    font_size: f32,
    shaping: TextShaping,
    locale: crate::font_pack::FontLocale,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(ShapedRun, &'a str, &'a TtfFont)> {
    let fallback_keys = crate::font_pack::fallback_keys(locale);
    for fallback_key in fallback_keys {
        if let Some((key, font)) = fonts.get_key_value(fallback_key)
            && let Some(shaped) = shape_text_with_font(text, font_size, font, shaping)
            && shaped_has_no_missing_glyphs(&shaped)
        {
            return Some((shaped, key.as_str(), font));
        }
    }
    None
}

fn shaped_has_no_missing_glyphs(shaped: &ShapedRun) -> bool {
    shaped.glyphs.iter().all(|glyph| glyph.glyph_id != 0)
}

fn measure_text_width_by_font_face_ranges(
    text: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    shaping: TextShaping,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let FontFamily::Custom(name) = font_family else {
        return None;
    };
    let (_, primary_font) = crate::system_fonts::find_font(fonts, name, bold, italic)?;
    if text
        .chars()
        .all(|ch| primary_font.cmap.contains_key(&(ch as u32)))
    {
        return None;
    }

    let mut width = 0.0;
    for segment in split_text_by_font_face_ranges(text, name, bold, italic, fonts)? {
        let shaped = shape_text_with_font(&segment.text, font_size, segment.font, shaping)?;
        width += shaped.width;
    }
    Some(width)
}

fn shape_with_font_face_range<'a>(
    run: &TextRun,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(ShapedRun, &'a str, &'a TtfFont)> {
    let FontFamily::Custom(name) = &run.font_family else {
        return None;
    };
    for (key, font) in font_face_range_fonts(fonts, name, run.bold, run.font_style.is_slanted()) {
        if !run
            .text
            .chars()
            .all(|ch| font.cmap.contains_key(&(ch as u32)))
        {
            continue;
        }
        if let Some(shaped) = shape_text_with_font(&run.text, run.font_size, font, run.shaping)
            && !shaped.glyphs.is_empty()
            && shaped.glyphs.iter().all(|g| g.glyph_id != 0)
        {
            return Some((shaped, key, font));
        }
    }
    None
}

struct FontFaceRangeSegment<'a> {
    text: String,
    font: &'a TtfFont,
}

fn split_text_by_font_face_ranges<'a>(
    text: &str,
    family: &str,
    bold: bool,
    italic: bool,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<Vec<FontFaceRangeSegment<'a>>> {
    let (_, primary_font) = crate::system_fonts::find_font(fonts, family, bold, italic)?;
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_font: Option<&TtfFont> = None;

    for ch in text.chars() {
        let font = if primary_font.cmap.contains_key(&(ch as u32)) {
            primary_font
        } else {
            font_face_range_fonts(fonts, family, bold, italic)
                .into_iter()
                .map(|(_, font)| font)
                .find(|font| font.cmap.contains_key(&(ch as u32)))?
        };

        if let Some(active) = current_font {
            if std::ptr::eq(active, font) {
                current.push(ch);
                continue;
            }
            segments.push(FontFaceRangeSegment {
                text: std::mem::take(&mut current),
                font: active,
            });
        }
        current_font = Some(font);
        current.push(ch);
    }

    if let Some(font) = current_font {
        segments.push(FontFaceRangeSegment {
            text: current,
            font,
        });
    }
    Some(segments)
}

fn font_face_range_fonts<'a>(
    fonts: &'a HashMap<String, TtfFont>,
    family: &str,
    bold: bool,
    italic: bool,
) -> Vec<(&'a str, &'a TtfFont)> {
    let prefix = format!(
        "{}__fontface_",
        crate::system_fonts::font_variant_key(family, bold, italic)
    );
    let mut matches: Vec<_> = fonts
        .iter()
        .filter_map(|(key, font)| {
            key.strip_prefix(&prefix)
                .and_then(|suffix| suffix.parse::<usize>().ok())
                .map(|index| (index, key.as_str(), font))
        })
        .collect();
    matches.sort_by_key(|(index, _, _)| *index);
    matches
        .into_iter()
        .map(|(_, key, font)| (key, font))
        .collect()
}

/// Check if a run needs unicode fallback (has characters the primary font can't cover).
pub(crate) fn needs_unicode_fallback(run: &TextRun, fonts: &HashMap<String, TtfFont>) -> bool {
    if let FontFamily::Custom(name) = &run.font_family {
        if let Some((_, font)) =
            crate::system_fonts::find_font(fonts, name, run.bold, run.font_style.is_slanted())
        {
            return run.text.chars().any(|ch| {
                let cp = ch as u32;
                !font.cmap.contains_key(&cp)
            });
        }
    }
    !crate::render::pdf::is_winansi_encodable(&run.text)
}

/// Whether an upright run contains a script whose font normally carries native
/// vertical metrics and alternate glyph positioning.
///
/// Western faces commonly omit those tables, and the established horizontal
/// fallback already synthesizes their upright placement. CJK uses the actual
/// vertical OpenType path when available.
pub(crate) fn contains_cjk_vertical_text(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            u32::from(ch),
            0x3040..=0x30ff
                | 0x3400..=0x4dbf
                | 0x4e00..=0x9fff
                | 0xf900..=0xfaff
                | 0xac00..=0xd7af
                | 0xff01..=0xff60
        )
    })
}

/// Split a text run into segments at font-coverage boundaries.
/// Returns `(text, use_fallback)` pairs — `use_fallback=true` means the
/// segment should be rendered with the unicode fallback font.
pub(crate) fn split_run_by_font_coverage(
    run: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<(String, bool)> {
    let primary_font = if let FontFamily::Custom(name) = &run.font_family {
        crate::system_fonts::find_font(fonts, name, run.bold, run.font_style.is_slanted())
            .map(|(_, f)| f)
    } else {
        None
    };

    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_is_fallback = false;

    for ch in run.text.chars() {
        let needs_fallback = if let Some(font) = primary_font {
            let cp = ch as u32;
            !font.cmap.contains_key(&cp)
        } else {
            !crate::render::pdf::is_winansi_char(ch)
        };

        if current.is_empty() {
            current_is_fallback = needs_fallback;
        } else if needs_fallback != current_is_fallback {
            segments.push((std::mem::take(&mut current), current_is_fallback));
            current_is_fallback = needs_fallback;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        segments.push((current, current_is_fallback));
    }
    segments
}

fn shape_text_with_font(
    text: &str,
    font_size: f32,
    font: &TtfFont,
    shaping: TextShaping,
) -> Option<ShapedRun> {
    let glyphs = shape_text_glyphs(
        text,
        font_size,
        font,
        shaping,
        TextShapingDirection::Horizontal,
    )?;
    let width = glyphs
        .iter()
        .fold(
            crate::layout::units::InlineLayoutUnit::default(),
            |width, glyph| {
                width + crate::layout::units::InlineLayoutUnit::from_points(glyph.x_advance)
            },
        )
        .to_points();
    Some(ShapedRun { glyphs, width })
}

fn shape_vertical_text_with_font(
    text: &str,
    font_size: f32,
    font: &TtfFont,
    shaping: TextShaping,
) -> Option<VerticalShapedRun> {
    Some(VerticalShapedRun {
        glyphs: shape_text_glyphs(
            text,
            font_size,
            font,
            shaping,
            TextShapingDirection::Vertical,
        )?,
    })
}

#[derive(Clone, Copy)]
enum TextShapingDirection {
    Horizontal,
    Vertical,
}

/// A `rustybuzz::Face` kept for the whole process. Building a face parses the
/// font's shaping tables; doing it on every shaped run (layout measures then
/// paint re-shapes, thousands of times per document) is the dominant text cost.
///
/// SAFETY: the face is an immutable, read-only view — shaping borrows it by
/// shared reference and never mutates it — so sharing `&SharedFace` across
/// threads is sound. Its `'static` byte view is backed by a leaked Arc
/// ref-count (see [`face_for_font`]) that pins the font buffer for the process.
struct SharedFace(rustybuzz::Face<'static>);
// SAFETY: see the type doc — read-only, bytes pinned for 'static.
unsafe impl Send for SharedFace {}
unsafe impl Sync for SharedFace {}

/// Process-global cache of shaping faces keyed by the font's shared byte-buffer
/// address (`Arc<Vec<u8>>` inner pointer) plus the selected face index (a
/// TTC/OTC collection serves several faces from one buffer). All clones of a
/// cached font share one Arc, so the key is stable; the first shape of a font
/// leaks one Arc ref-count to pin that buffer for the process, so the address
/// is never reused and the `'static` face view stays valid. `None` is cached
/// for un-parseable fonts.
type FaceCache = HashMap<(usize, u32), Option<Arc<SharedFace>>>;
static FACE_CACHE: OnceLock<RwLock<FaceCache>> = OnceLock::new();

fn face_cache() -> &'static RwLock<FaceCache> {
    FACE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Return a cached shaping face for `font`, building (and caching) it on first
/// use. Identical bytes to
/// `rustybuzz::Face::from_slice(&font.data, font.face_index.get())`, so
/// shaping results are unchanged.
fn face_for_font(font: &TtfFont) -> Option<Arc<SharedFace>> {
    let key = (Arc::as_ptr(&font.data) as usize, font.face_index.get());
    if let Ok(cache) = face_cache().read()
        && let Some(entry) = cache.get(&key)
    {
        return entry.clone();
    }
    // Miss: leak one Arc ref-count so the byte buffer lives for the whole
    // process, then take a 'static view of it to build the face.
    let arc = font.data.clone();
    let buf: &Vec<u8> = &arc;
    let bytes: &'static [u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr(), buf.len()) };
    std::mem::forget(arc);
    let face = rustybuzz::Face::from_slice(bytes, font.face_index.get())
        .map(|face| Arc::new(SharedFace(face)));
    if let Ok(mut cache) = face_cache().write() {
        cache.entry(key).or_insert_with(|| face.clone());
    }
    face
}

fn shape_text_glyphs(
    text: &str,
    font_size: f32,
    font: &TtfFont,
    shaping: TextShaping,
    direction: TextShapingDirection,
) -> Option<Vec<ShapedGlyph>> {
    if text.is_empty() {
        return Some(Vec::new());
    }

    let shared_face = face_for_font(font)?;
    let face = &shared_face.0;
    let scale = font.adjusted_font_size(font_size) / (face.units_per_em() as f32).max(1.0);

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    if matches!(direction, TextShapingDirection::Vertical) {
        // HarfBuzz/Rustybuzz enables vertical OpenType features for TTB. This
        // also obtains per-glyph vertical advances and origins from `vmtx` and
        // `VORG` rather than approximating them with horizontal metrics.
        buffer.set_direction(rustybuzz::Direction::TopToBottom);
    }

    let mut features = Vec::new();
    if !shaping.ligatures {
        // `font-feature-settings: "liga" 0` (css-fonts-3 §6.4): turn off
        // standard and contextual ligatures while leaving pair positioning to
        // the independently-controlled `font-kerning` property.
        features.extend([
            rustybuzz::Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"liga"), 0, ..),
            rustybuzz::Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"clig"), 0, ..),
        ]);
    }
    if !shaping.kerning {
        features.push(rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"kern"),
            0,
            ..,
        ));
    }

    let shaped = rustybuzz::shape(face, &features, buffer);
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    if infos.len() != positions.len() {
        return None;
    }
    let clusters = infos
        .iter()
        .map(|info| usize::try_from(info.cluster).ok())
        .collect::<Option<Vec<_>>>()?;
    let cluster_unicode = glyph_cluster_unicode(text, &clusters)?;
    let resolve_position = |position: i32| {
        crate::layout::units::TextRunLayoutUnit::from_points(position as f32 * scale).to_points()
    };

    infos
        .iter()
        .zip(positions.iter())
        .zip(cluster_unicode)
        .map(|((info, position), unicode)| {
            Some(ShapedGlyph {
                glyph_id: u16::try_from(info.glyph_id).ok()?,
                x_advance: resolve_position(position.x_advance),
                y_advance: resolve_position(position.y_advance),
                x_offset: resolve_position(position.x_offset),
                y_offset: resolve_position(position.y_offset),
                unicode,
            })
        })
        .collect()
}

fn glyph_cluster_unicode(text: &str, clusters: &[usize]) -> Option<Vec<Vec<u16>>> {
    let mut cluster_starts = clusters.to_vec();
    cluster_starts.push(text.len());
    cluster_starts.sort_unstable();
    cluster_starts.dedup();

    let mut cluster_text = HashMap::with_capacity(cluster_starts.len());
    for window in cluster_starts.windows(2) {
        let start = window[0];
        let end = window[1];
        let slice = text.get(start..end)?;
        cluster_text.insert(start, slice.encode_utf16().collect());
    }

    let mut seen_clusters = HashSet::with_capacity(clusters.len());
    clusters
        .iter()
        .map(|cluster| {
            if seen_clusters.insert(*cluster) {
                cluster_text.get(cluster).cloned()
            } else {
                Some(Vec::new())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ShapedGlyph, ShapedRun, custom_font_line_height, glyph_cluster_unicode,
        inline_boundary_kerning_advance, measure_text_width, resolve_custom_font, shape_text_run,
        shape_text_with_font, shape_vertical_text_with_font,
    };
    use crate::layout::engine::{TextRun, TextShaping};
    use crate::style::computed::FontFamily;
    use std::collections::HashMap;

    #[test]
    fn glyph_cluster_unicode_emits_cluster_text_once_per_cluster() {
        let unicode = glyph_cluster_unicode("fi", &[0, 0]).unwrap();
        assert_eq!(unicode, vec![vec![0x0066, 0x0069], Vec::new()]);
    }

    #[test]
    fn glyph_cluster_unicode_handles_reordered_clusters() {
        let unicode = glyph_cluster_unicode("ab", &[1, 0]).unwrap();
        assert_eq!(unicode, vec![vec![0x0062], vec![0x0061]]);
    }

    // --- shape_text_with_font ---

    // shape_text_with_font is private; we need a real TtfFont to call it with a
    // non-empty string.  For the empty-string branch we can verify the fast path
    // without any font data by constructing a minimal stub.
    fn make_stub_font() -> crate::parser::ttf::TtfFont {
        use crate::parser::ttf::{FontVerticalMetricSet, FontVerticalMetrics, TtfFont};
        TtfFont {
            font_name: "Stub".into(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0, 0, 0, 0],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(800, -200, 0)),
            cmap: HashMap::new(),
            glyph_widths: Vec::new(),
            num_h_metrics: 0,
            flags: 0,
            is_bold: false,
            is_italic: false,
            text_metrics: Default::default(),
            data: std::sync::Arc::new(Vec::new()),
        }
    }

    #[test]
    fn shape_text_with_font_empty_string_returns_zero_width() {
        let font = make_stub_font();
        let run = shape_text_with_font("", 12.0, &font, TextShaping::default()).unwrap();
        assert_eq!(run.width, 0.0);
        assert!(run.glyphs.is_empty());
    }

    #[test]
    fn custom_font_advances_retain_text_run_fixed_point_precision() {
        let fonts = parity_sans_fonts();
        let (_, font) = resolve_custom_font(
            &FontFamily::Custom("ParitySans".to_string()),
            false,
            false,
            &fonts,
        )
        .expect("ParitySans must resolve");
        let shaped = shape_text_with_font("AgBb", 12.0, font, TextShaping::default())
            .expect("ParitySans text shapes");

        for glyph in &shaped.glyphs {
            let text_unit = crate::layout::units::TextRunLayoutUnit::from_points(glyph.x_advance);
            assert_eq!(text_unit.to_points(), glyph.x_advance);
        }
        assert!(
            (shaped.width - 31.675_781).abs() < 0.001,
            "shaped width was {}",
            shaped.width
        );
    }

    // --- resolve_custom_font ---

    #[test]
    fn resolve_custom_font_returns_none_for_helvetica() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::Helvetica, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_for_times_roman() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::TimesRoman, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_for_courier() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::Courier, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_when_custom_font_not_in_map() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("MyFont".into());
        assert!(resolve_custom_font(&family, false, false, &fonts).is_none());
    }

    // --- measure_text_width ---

    #[test]
    fn measure_text_width_returns_none_for_standard_font() {
        let fonts = HashMap::new();
        let result =
            measure_text_width("hello", 12.0, &FontFamily::Helvetica, false, false, &fonts);
        assert!(result.is_none());
    }

    #[test]
    fn measure_text_width_returns_none_when_custom_font_not_found() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("Missing".into());
        let result = measure_text_width("hello", 12.0, &family, false, false, &fonts);
        assert!(result.is_none());
    }

    #[test]
    fn measure_text_width_uses_the_same_unicode_fallback_as_painting() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let primary = crate::parser::ttf::parse_ttf(
            std::fs::read(root.join("tests/parity/fonts/ParitySans.ttf")).unwrap(),
        )
        .unwrap();
        let fallback = crate::parser::ttf::parse_ttf(
            include_bytes!("../tests/fonts/IronpressCjkVertical.ttf").to_vec(),
        )
        .unwrap();
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::font_variant_key("ParitySans", false, false),
            primary,
        );
        fonts.insert(crate::system_fonts::UNICODE_FALLBACK_KEY.into(), fallback);

        let width = measure_text_width(
            "第",
            18.0,
            &FontFamily::Custom("ParitySans".into()),
            false,
            false,
            &fonts,
        )
        .unwrap();
        assert!((width - 18.0).abs() < 0.01, "width={width}");
    }

    fn parity_sans_fonts() -> HashMap<String, crate::parser::ttf::TtfFont> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let font = crate::parser::ttf::parse_ttf(
            std::fs::read(root.join("tests/parity/fonts/ParitySans.ttf"))
                .expect("ParitySans test font"),
        )
        .expect("valid ParitySans TTF");
        HashMap::from([(
            crate::system_fonts::font_variant_key("ParitySans", false, false),
            font,
        )])
    }

    fn parity_sans_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_size: 18.0,
            font_family: FontFamily::Custom("ParitySans".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn styled_inline_boundary_retains_pair_kerning_without_a_cross_run_ligature() {
        let fonts = parity_sans_fonts();
        let left = parity_sans_run("F");
        let right = parity_sans_run("irst");
        let advance = inline_boundary_kerning_advance(&left, &right, &fonts);
        assert!(
            advance < 0.0,
            "expected negative F/i kerning, got {advance}"
        );

        let (_, font) = resolve_custom_font(&left.font_family, false, false, &fonts)
            .expect("ParitySans must resolve");
        let shaping = TextShaping::KERNING_ONLY;
        let pair_width = shape_text_with_font("Fi", left.font_size, font, shaping)
            .expect("Fi shapes")
            .width;
        let separate_width = shape_text_with_font("F", left.font_size, font, shaping)
            .expect("F shapes")
            .width
            + shape_text_with_font("i", left.font_size, font, shaping)
                .expect("i shapes")
                .width;
        assert!(((separate_width + advance) - pair_width).abs() < 0.0001);
    }

    #[test]
    fn vertical_shaping_uses_the_font_vertical_origin_and_advance() {
        let font = crate::parser::ttf::parse_ttf(
            include_bytes!("../tests/fonts/IronpressCjkVertical.ttf").to_vec(),
        )
        .unwrap();

        let shaped =
            shape_vertical_text_with_font("第", 18.0, &font, TextShaping::default()).unwrap();
        assert_eq!(shaped.glyphs.len(), 1);
        let glyph = &shaped.glyphs[0];
        assert_eq!(glyph.x_advance, 0.0);
        assert!(glyph.y_advance < 0.0, "y_advance={}", glyph.y_advance);
        assert!(glyph.x_offset < 0.0, "x_offset={}", glyph.x_offset);
        assert!(glyph.y_offset < 0.0, "y_offset={}", glyph.y_offset);
    }

    // --- custom_font_line_height ---

    #[test]
    fn custom_font_line_height_returns_none_for_helvetica() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::Helvetica, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_for_times_roman() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::TimesRoman, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_for_courier() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::Courier, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_when_custom_font_not_found() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("Ghost".into());
        assert!(custom_font_line_height(&family, false, false, &fonts).is_none());
    }

    // -----------------------------------------------------------------------
    // ShapedGlyph / ShapedRun – struct field access, Clone, Debug
    // -----------------------------------------------------------------------

    #[test]
    fn shaped_glyph_fields_and_clone() {
        let g = ShapedGlyph {
            glyph_id: 42,
            x_advance: 10.5,
            y_advance: 0.0,
            x_offset: 1.0,
            y_offset: -2.0,
            unicode: vec![0x0041],
        };
        let g2 = g.clone();
        assert_eq!(g2.glyph_id, 42);
        assert_eq!(g2.x_advance, 10.5);
        assert_eq!(g2.y_advance, 0.0);
        assert_eq!(g2.x_offset, 1.0);
        assert_eq!(g2.y_offset, -2.0);
        assert_eq!(g2.unicode, vec![0x0041u16]);
        // Debug must not panic
        let _ = format!("{:?}", g);
    }

    #[test]
    fn shaped_run_fields_and_clone() {
        let run = ShapedRun {
            glyphs: vec![ShapedGlyph {
                glyph_id: 1,
                x_advance: 5.0,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: vec![0x0061],
            }],
            width: 5.0,
        };
        let run2 = run.clone();
        assert_eq!(run2.width, 5.0);
        assert_eq!(run2.glyphs.len(), 1);
        assert_eq!(run2.glyphs[0].glyph_id, 1);
        let _ = format!("{:?}", run);
    }

    // -----------------------------------------------------------------------
    // shape_text_run – None when font is missing from map
    // -----------------------------------------------------------------------

    #[test]
    fn shape_text_run_returns_none_when_font_not_found() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "hello".into(),
            font_family: FontFamily::Custom("Missing".into()),
            ..Default::default()
        };
        assert!(shape_text_run(&run, &fonts).is_none());
    }

    #[test]
    fn shape_text_run_returns_none_for_standard_font_family() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "hello".into(),
            ..Default::default()
        };
        assert!(shape_text_run(&run, &fonts).is_none());
    }

    // -----------------------------------------------------------------------
    // shape_text_with_font – returns None when font.data is not a valid face
    // -----------------------------------------------------------------------

    #[test]
    fn shape_text_with_font_returns_none_for_invalid_font_data() {
        let font = make_stub_font(); // data is Vec::new(), rustybuzz can't parse it
        assert!(shape_text_with_font("hello", 12.0, &font, TextShaping::default()).is_none());
    }

    // -----------------------------------------------------------------------
    // Helper to load a real system font so we can test the shaping hot path.
    // The font path is macOS-specific; the tests are gated accordingly.
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    fn load_real_font() -> Option<crate::parser::ttf::TtfFont> {
        let data = std::fs::read("/System/Library/Fonts/Geneva.ttf").ok()?;
        crate::parser::ttf::parse_ttf(data).ok()
    }

    #[cfg(target_os = "macos")]
    fn make_real_font_map() -> HashMap<String, crate::parser::ttf::TtfFont> {
        let font = match load_real_font() {
            Some(f) => f,
            None => return HashMap::new(),
        };
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::font_variant_key("Geneva", false, false),
            font,
        );
        fonts
    }

    // -----------------------------------------------------------------------
    // shape_text_with_font – full shaping path with a real font
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_with_font_shapes_ascii_text_with_real_font() {
        let font = match load_real_font() {
            Some(f) => f,
            None => return, // font not available on this machine, skip
        };
        let result = shape_text_with_font("Hi", 12.0, &font, TextShaping::default());
        let run = result.expect("shaping should succeed with a real font");
        assert_eq!(run.glyphs.len(), 2, "two glyphs for two-character input");
        assert!(run.width > 0.0, "shaped width must be positive");
        // Each glyph should carry the right character
        assert!(!run.glyphs[0].unicode.is_empty());
        assert!(!run.glyphs[1].unicode.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_with_font_glyph_fields_are_populated() {
        let font = match load_real_font() {
            Some(f) => f,
            None => return,
        };
        let run = shape_text_with_font("A", 10.0, &font, TextShaping::default()).unwrap();
        assert_eq!(run.glyphs.len(), 1);
        let g = &run.glyphs[0];
        // x_advance should be a non-negative scaled value for a normal glyph
        assert!(g.x_advance >= 0.0);
        assert_eq!(run.width, g.x_advance);
    }

    // -----------------------------------------------------------------------
    // shape_text_run – full path with a real font
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_run_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return; // font not available, skip
        }
        let run = TextRun {
            text: "Hi".into(),
            font_size: 14.0,
            font_family: FontFamily::Custom("Geneva".into()),
            ..Default::default()
        };
        let result = shape_text_run(&run, &fonts);
        let shaped = result.expect("shape_text_run must return Some for a found font");
        assert_eq!(shaped.glyphs.len(), 2);
        assert!(shaped.width > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_run_empty_text_returns_zero_width_run() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let run = TextRun {
            text: String::new(),
            font_family: FontFamily::Custom("Geneva".into()),
            ..Default::default()
        };
        let shaped = shape_text_run(&run, &fonts).expect("empty text still returns Some");
        assert_eq!(shaped.width, 0.0);
        assert!(shaped.glyphs.is_empty());
    }

    // -----------------------------------------------------------------------
    // measure_text_width – returns Some when font is present
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn measure_text_width_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = measure_text_width("hello", 12.0, &family, false, false, &fonts);
        let width = result.expect("must return Some for a found custom font");
        assert!(width > 0.0, "width of non-empty text must be positive");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn measure_text_width_empty_string_returns_zero() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = measure_text_width("", 12.0, &family, false, false, &fonts);
        assert_eq!(result, Some(0.0));
    }

    // -----------------------------------------------------------------------
    // custom_font_line_height – returns Some when font is present
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_font_line_height_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = custom_font_line_height(&family, false, false, &fonts);
        let ratio = result.expect("must return Some for a found custom font");
        // line_height_ratio is clamped to at least 1.0
        assert!(
            ratio >= 1.0,
            "line height ratio must be >= 1.0, got {}",
            ratio
        );
    }
}
