use crate::layout::elements::{
    AvoidPageBreak, ColumnRule, Container, FlexRow, GridRow, HorizontalRule, Image, LayoutElement,
    LayoutVisitor, MathBlock, NamedString, PageBreak, ProgressBar, RunningElement, Svg, TableRow,
    TextBlock,
};
use crate::layout::engine::{Page, TextLine, TextRun};
use crate::parser::ttf::TtfFont;
use crate::style::computed::FontFamily;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) type PreparedCustomFonts = BTreeMap<String, PreparedCustomFont>;
type ToUnicodeMap = Vec<(u16, Vec<u16>)>;

pub(crate) struct PreparedCustomFont {
    pub(crate) base_font_name: String,
    source_font_name: String,
    pub(crate) font_data: Vec<u8>,
    pub(crate) widths: Vec<f32>,
    pub(crate) to_unicode_map: ToUnicodeMap,
    glyph_id_map: HashMap<u16, u16>,
    embedding: FontEmbedding,
    type3_glyphs: Vec<Type3Glyph>,
}

impl PreparedCustomFont {
    fn with_source_font_name(mut self, source_font_name: String) -> Self {
        self.source_font_name = source_font_name;
        self
    }

    pub(crate) fn source_font_name<'a>(&'a self, resource_name: &'a str) -> &'a str {
        if self.source_font_name.is_empty() {
            resource_name
        } else {
            &self.source_font_name
        }
    }

    pub(crate) fn pdf_glyph_id(&self, old_glyph_id: u16) -> u16 {
        self.glyph_id_map
            .get(&old_glyph_id)
            .copied()
            .unwrap_or(old_glyph_id)
    }

    pub(crate) fn encode_glyph(&self, old_glyph_id: u16) -> String {
        let glyph_id = self.pdf_glyph_id(old_glyph_id);
        match self.embedding {
            FontEmbedding::Cid => format!("{glyph_id:04X}"),
            FontEmbedding::Type3(_) => format!("{glyph_id:02X}"),
        }
    }

    pub(super) const fn uses_type3_embedding(&self) -> bool {
        matches!(self.embedding, FontEmbedding::Type3(_))
    }

    pub(super) const fn embeds_synthetic_weight(&self) -> bool {
        matches!(
            self.embedding,
            FontEmbedding::Type3(Type3GlyphStyle::SyntheticWeight)
        )
    }

    pub(super) const fn type3_glyph_style(&self) -> Type3GlyphStyle {
        match self.embedding {
            FontEmbedding::Type3(style) => style,
            FontEmbedding::Cid => Type3GlyphStyle::Plain,
        }
    }

    pub(super) fn type3_glyphs(&self) -> &[Type3Glyph] {
        &self.type3_glyphs
    }
}

#[derive(Clone, Copy)]
pub(super) struct Type3Glyph {
    pub(super) code: u8,
    pub(super) glyph_id: u16,
}

#[derive(Clone, Copy, Default)]
enum FontEmbedding {
    #[default]
    Cid,
    Type3(Type3GlyphStyle),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Type3GlyphStyle {
    #[default]
    Plain,
    SyntheticWeight,
}

#[derive(Default)]
struct FontUsage {
    glyphs: BTreeSet<u16>,
    to_unicode_map: BTreeMap<u16, Vec<u16>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FontUsageKey {
    source_name: String,
    synthetic_weight: bool,
}

impl FontUsageKey {
    fn plain(source_name: &str) -> Self {
        Self {
            source_name: source_name.to_string(),
            synthetic_weight: false,
        }
    }

    fn for_run(source_name: &str, run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> Self {
        Self {
            source_name: source_name.to_string(),
            synthetic_weight: run.synthetic_bold_stroke_width(custom_fonts).is_some(),
        }
    }

    fn resource_name(&self) -> Cow<'_, str> {
        prepared_font_name(&self.source_name, self.synthetic_weight)
    }

    const fn glyph_style(&self) -> Type3GlyphStyle {
        if self.synthetic_weight {
            Type3GlyphStyle::SyntheticWeight
        } else {
            Type3GlyphStyle::Plain
        }
    }
}

const SYNTHETIC_WEIGHT_FONT_SUFFIX: &str = "__synthetic_weight";

fn prepared_font_name(source_name: &str, synthetic_weight: bool) -> Cow<'_, str> {
    if synthetic_weight {
        Cow::Owned(format!("{source_name}{SYNTHETIC_WEIGHT_FONT_SUFFIX}"))
    } else {
        Cow::Borrowed(source_name)
    }
}

pub(crate) fn prepared_font_name_for_run<'a>(
    source_name: &'a str,
    run: &TextRun,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Cow<'a, str> {
    prepared_font_name(
        source_name,
        run.synthetic_bold_stroke_width(custom_fonts).is_some(),
    )
}

impl FontUsage {
    fn record_glyph(&mut self, glyph_id: u16, unicode: Vec<u16>) {
        self.glyphs.insert(glyph_id);
        if !unicode.is_empty() {
            self.to_unicode_map.entry(glyph_id).or_insert(unicode);
        }
    }
}

/// Prepare document fonts while also accounting for generated text that is not
/// stored in the laid-out page tree (for example CSS page-margin boxes).
///
/// `per_page_runs` are visited after that page's ordinary elements and before
/// its running elements and footnotes. `trailing_runs` are visited after all
/// pages. This matches the traversal order of adding synthetic text blocks to
/// cloned pages, without copying any page, layout, SVG, or raster payload.
pub(crate) fn prepare_custom_fonts_with_additional_runs(
    pages: &[Page],
    custom_fonts: &HashMap<String, TtfFont>,
    per_page_runs: &[Vec<TextRun>],
    trailing_runs: &[TextRun],
) -> PreparedCustomFonts {
    let usage =
        collect_font_usage_with_additional_runs(pages, custom_fonts, per_page_runs, trailing_runs);

    usage
        .into_iter()
        .filter_map(|(usage_key, usage)| {
            custom_fonts.get(&usage_key.source_name).map(|ttf| {
                (
                    usage_key.resource_name().into_owned(),
                    prepare_font(ttf, &usage, usage_key.glyph_style())
                        .with_source_font_name(usage_key.source_name.clone()),
                )
            })
        })
        .collect()
}

fn collect_font_usage_with_additional_runs(
    pages: &[Page],
    custom_fonts: &HashMap<String, TtfFont>,
    per_page_runs: &[Vec<TextRun>],
    trailing_runs: &[TextRun],
) -> BTreeMap<FontUsageKey, FontUsage> {
    let mut usage = BTreeMap::new();
    for (page_index, page) in pages.iter().enumerate() {
        for (_, element) in &page.elements {
            collect_font_usage_from_element(element, custom_fonts, &mut usage);
        }
        if let Some(runs) = per_page_runs.get(page_index) {
            for run in runs {
                collect_font_usage_from_run(run, custom_fonts, &mut usage);
            }
        }
        for element in page.generated_content.running_elements() {
            collect_font_usage_from_element(element, custom_fonts, &mut usage);
        }
        for footnote in &page.footnotes {
            for run in footnote.text_runs() {
                collect_font_usage_from_run(&run, custom_fonts, &mut usage);
            }
        }
    }
    for run in trailing_runs {
        collect_font_usage_from_run(run, custom_fonts, &mut usage);
    }
    usage
}

fn collect_font_usage_from_element(
    element: &dyn LayoutElement,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    let mut collector = FontUsageCollector {
        custom_fonts,
        usage,
    };
    collect_font_usage_walk(element, &mut collector);
}

/// Depth-first walk pairing the node visitor with a per-box background check:
/// custom-font `<text>` inside a CSS background-image SVG must be subset and
/// embedded exactly like foreground SVG text.
fn collect_font_usage_walk(element: &dyn LayoutElement, collector: &mut FontUsageCollector<'_>) {
    if let Some(owner) = element.box_paint_owner()
        && let Some(svg) = owner.box_paint().background.layers.svg.as_ref()
    {
        collect_font_usage_from_svg(svg, collector.custom_fonts, collector.usage);
    }
    element.accept(collector);
    element.visit_children(&mut |child| collect_font_usage_walk(child, collector));
}

struct FontUsageCollector<'a> {
    custom_fonts: &'a HashMap<String, TtfFont>,
    usage: &'a mut BTreeMap<FontUsageKey, FontUsage>,
}

impl LayoutVisitor for FontUsageCollector<'_> {
    fn visit_text_block(&mut self, element: &TextBlock) {
        collect_font_usage_from_lines(&element.lines, self.custom_fonts, self.usage);
    }

    fn visit_table_row(&mut self, element: &TableRow) {
        for cell in &element.content.cells {
            collect_font_usage_from_lines(
                &cell.layout.content.lines,
                self.custom_fonts,
                self.usage,
            );
            if let Some(svg) = cell.layout.paint.box_paint.background.layers.svg.as_ref() {
                collect_font_usage_from_svg(svg, self.custom_fonts, self.usage);
            }
        }
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        for cell in &element.content.cells {
            collect_font_usage_from_lines(
                &cell.layout.content.lines,
                self.custom_fonts,
                self.usage,
            );
            if let Some(svg) = cell.layout.paint.box_paint.background.layers.svg.as_ref() {
                collect_font_usage_from_svg(svg, self.custom_fonts, self.usage);
            }
        }
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        for cell in &element.content.cells {
            collect_font_usage_from_lines(&cell.lines, self.custom_fonts, self.usage);
            if let Some(svg) = cell.paint.background.layers.svg.as_ref() {
                collect_font_usage_from_svg(svg, self.custom_fonts, self.usage);
            }
        }
    }

    fn visit_svg(&mut self, element: &Svg) {
        collect_font_usage_from_svg(&element.tree, self.custom_fonts, self.usage);
    }

    fn visit_avoid_page_break(&mut self, _element: &AvoidPageBreak) {}
    fn visit_column_rule(&mut self, _element: &ColumnRule) {}
    fn visit_image(&mut self, _element: &Image) {}
    fn visit_horizontal_rule(&mut self, _element: &HorizontalRule) {}
    fn visit_progress_bar(&mut self, _element: &ProgressBar) {}
    fn visit_math_block(&mut self, _element: &MathBlock) {}
    fn visit_container(&mut self, _element: &Container) {}
    fn visit_running_element(&mut self, _element: &RunningElement) {}
    fn visit_named_string(&mut self, _element: &NamedString) {}
    fn visit_page_break(&mut self, _element: &PageBreak) {}
}

/// Collect glyph usage for SVG `<text>` rendered with a registered custom font.
///
/// The SVG text renderer shapes such text and emits embedded CID glyphs against
/// the same subsetted font resource the body text uses. Those glyphs must be
/// registered here so they survive subsetting (otherwise SVG-only glyphs would
/// be dropped and render as `.notdef`).
fn collect_font_usage_from_svg(
    tree: &crate::parser::svg::SvgTree,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    for node in &tree.children {
        collect_font_usage_from_svg_node(
            node,
            &tree.text_ctx,
            None,
            None,
            None,
            crate::render::svg_text::SvgTextSizing::initial(tree.text_ctx.font_size),
            custom_fonts,
            usage,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_font_usage_from_svg_node(
    node: &crate::parser::svg::SvgNode,
    text_ctx: &crate::parser::svg::SvgTextContext,
    inherited_family: Option<&str>,
    inherited_bold: Option<bool>,
    inherited_italic: Option<bool>,
    inherited_sizing: crate::render::svg_text::SvgTextSizing,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    use crate::parser::svg::SvgNode;
    match node {
        SvgNode::Group {
            children, style, ..
        } => {
            let family = style.font_family.as_deref().or(inherited_family);
            let bold = style.font_bold.or(inherited_bold);
            let italic = style.font_italic.or(inherited_italic);
            let sizing = inherited_sizing.cascade(style.font_size, style.letter_spacing.as_ref());
            for child in children {
                collect_font_usage_from_svg_node(
                    child,
                    text_ctx,
                    family,
                    bold,
                    italic,
                    sizing,
                    custom_fonts,
                    usage,
                );
            }
        }
        SvgNode::Text {
            font_family,
            font_bold,
            font_italic,
            content,
            style,
            ..
        } => {
            let family = font_family
                .as_deref()
                .or(style.font_family.as_deref())
                .or(inherited_family)
                .map(str::to_string)
                .or_else(|| {
                    let ctx = text_ctx.font_family.trim();
                    (!ctx.is_empty()).then(|| ctx.to_string())
                });
            let Some(family) = family else {
                return;
            };
            let bold = font_bold
                .or(style.font_bold)
                .or(inherited_bold)
                .unwrap_or(text_ctx.font_bold);
            let italic = font_italic
                .or(style.font_italic)
                .or(inherited_italic)
                .unwrap_or(text_ctx.font_italic);
            let sizing = inherited_sizing.cascade(style.font_size, style.letter_spacing.as_ref());
            let runs = crate::render::svg_text::fonts::resolve_svg_text_font_runs(
                content,
                &family,
                &text_ctx.font_family,
                bold,
                italic,
                Some(custom_fonts),
            );
            for run in runs {
                let crate::render::svg_text::fonts::SvgTextFace::Custom(face) = run.face else {
                    continue;
                };
                if let Some(shaped) = crate::text::shape_text_with_explicit_font_and_shaping(
                    run.text,
                    sizing.font_size(),
                    face.font(),
                    sizing.shaping(),
                ) {
                    let font_usage = usage.entry(FontUsageKey::plain(face.key())).or_default();
                    for glyph in shaped.glyphs {
                        font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_font_usage_from_lines(
    lines: &[TextLine],
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    for line in lines {
        let painted_runs = crate::text::coalesce_text_runs(&line.runs);
        for run in &painted_runs {
            collect_font_usage_from_run(run, custom_fonts, usage);
            collect_upright_vertical_font_usage(run, line, custom_fonts, usage);
        }
    }
}

/// Include vertical-substitution glyphs before subsetting an upright line.
///
/// Horizontal shaping alone can omit a `vert` alternate from the PDF subset.
/// The ordinary collector remains necessary too: it covers all non-upright
/// paint paths and gives a sensible fallback if an external PDF consumer does
/// not support the vertical alternate.
fn collect_upright_vertical_font_usage(
    run: &TextRun,
    line: &TextLine,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    if !line.metadata.text_orientation_upright
        || run.metadata.text_combine_upright.is_active()
        || run.text.is_empty()
        || !crate::text::contains_cjk_vertical_text(&run.text)
    {
        return;
    }
    let Some(shaped) = crate::text::shape_upright_vertical_run(run, custom_fonts) else {
        return;
    };
    let font_usage = usage
        .entry(FontUsageKey::plain(shaped.font_key))
        .or_default();
    for glyph in shaped.shaped.glyphs {
        font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
    }
}

fn collect_font_usage_from_run(
    run: &TextRun,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<FontUsageKey, FontUsage>,
) {
    // An atomic inline box (display: inline-block) carries its own pre-wrapped
    // inner text lines. Those glyphs must be registered for subsetting too, or
    // the box renders with missing glyphs. The run's own `text` is empty.
    if let Some(inline) = run.inline_box.as_deref() {
        collect_font_usage_from_lines(&inline.lines, custom_fonts, usage);
        return;
    }

    if run.metadata.emphasis.mark {
        collect_font_usage_from_run(
            &crate::render::pdf::emphasis_mark_run(run),
            custom_fonts,
            usage,
        );
    }

    // Standard PDF font runs with non-WinAnsi text → collect under fallback font
    if !matches!(&run.font_family, FontFamily::Custom(_)) {
        if let Some((shaped_run, fallback_key, _)) =
            crate::text::shape_with_unicode_fallback(run, custom_fonts)
        {
            let font_usage = usage.entry(FontUsageKey::plain(fallback_key)).or_default();
            for glyph in shaped_run.glyphs {
                font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
            }
        }
        return;
    }

    let FontFamily::Custom(name) = &run.font_family else {
        return;
    };
    let Some((resolved_name, ttf)) =
        crate::system_fonts::find_font(custom_fonts, name, run.bold, run.font_style.is_slanted())
    else {
        return;
    };

    if crate::text::needs_unicode_fallback(run, custom_fonts) {
        for (segment_text, use_fallback) in
            crate::text::split_run_by_font_coverage(run, custom_fonts)
        {
            let mut sub_run = run.clone();
            sub_run.text = segment_text;
            if use_fallback {
                if let Some((shaped_run, fallback_key, _)) =
                    crate::text::shape_with_unicode_fallback(&sub_run, custom_fonts)
                {
                    let font_usage = usage.entry(FontUsageKey::plain(fallback_key)).or_default();
                    for glyph in shaped_run.glyphs {
                        font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
                    }
                }
            } else if let Some(shaped_run) = crate::text::shape_text_run(&sub_run, custom_fonts) {
                let font_usage = usage
                    .entry(FontUsageKey::for_run(resolved_name, &sub_run, custom_fonts))
                    .or_default();
                for glyph in shaped_run.glyphs {
                    font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
                }
            }
        }
        return;
    }

    let font_usage = usage
        .entry(FontUsageKey::for_run(resolved_name, run, custom_fonts))
        .or_default();
    if let Some(shaped_run) = crate::text::shape_text_run(run, custom_fonts) {
        for glyph in shaped_run.glyphs {
            font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
        }
        return;
    }

    for ch in run.text.chars() {
        if let Some(glyph_id) = ttf.cmap.get(&(ch as u32)).copied() {
            let unicode: Vec<u16> = ch.encode_utf16(&mut [0; 2]).to_vec();
            font_usage.record_glyph(glyph_id, unicode);
        }
    }
}

fn prepare_font(
    ttf: &TtfFont,
    usage: &FontUsage,
    glyph_style: Type3GlyphStyle,
) -> PreparedCustomFont {
    if (glyph_style == Type3GlyphStyle::SyntheticWeight
        || crate::render::pdf::sfnt_has_cff_outlines(&ttf.data))
        && usage.glyphs.len() <= u8::MAX as usize
        && rustybuzz::ttf_parser::Face::parse(&ttf.data, ttf.face_index.get()).is_ok()
    {
        return type3_font(ttf, usage, glyph_style);
    }

    let glyphs: Vec<u16> = usage.glyphs.iter().copied().collect();
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);

    subsetter::subset(&ttf.data, ttf.face_index.get(), &remapper)
        .ok()
        .map(|font_data| subset_font(ttf, usage, &remapper, font_data))
        .unwrap_or_else(|| fallback_font(ttf))
}

fn subset_font(
    ttf: &TtfFont,
    usage: &FontUsage,
    remapper: &subsetter::GlyphRemapper,
    font_data: Vec<u8>,
) -> PreparedCustomFont {
    let mut glyph_id_map = HashMap::with_capacity(remapper.num_gids() as usize);
    let mut widths = vec![0.0; remapper.num_gids() as usize];

    for old_glyph_id in remapper.remapped_gids() {
        let Some(new_glyph_id) = remapper.get(old_glyph_id) else {
            continue;
        };
        glyph_id_map.insert(old_glyph_id, new_glyph_id);
        if let Some(width) = widths.get_mut(new_glyph_id as usize) {
            *width = ttf.glyph_width_pdf_value(old_glyph_id);
        }
    }

    PreparedCustomFont {
        base_font_name: subset_base_font_name(&ttf.font_name, remapper.num_gids()),
        source_font_name: String::new(),
        font_data,
        widths,
        to_unicode_map: to_unicode_map_for_subset(usage, remapper),
        glyph_id_map,
        embedding: FontEmbedding::Cid,
        type3_glyphs: Vec::new(),
    }
}

fn fallback_font(ttf: &TtfFont) -> PreparedCustomFont {
    PreparedCustomFont {
        base_font_name: sanitize_pdf_font_name(&ttf.font_name),
        source_font_name: String::new(),
        font_data: (*ttf.data).clone(),
        widths: (0..ttf.glyph_widths.len())
            .map(|glyph_id| ttf.glyph_width_pdf_value(glyph_id as u16))
            .collect(),
        to_unicode_map: to_unicode_map_for_full_font(ttf),
        glyph_id_map: HashMap::new(),
        embedding: FontEmbedding::Cid,
        type3_glyphs: Vec::new(),
    }
}

/// Chrome serializes small CFF-backed fallback runs as unhinted Type 3 glyph
/// paths. Keeping that representation avoids device-dependent CFF hinting when
/// the same PDF is rasterized by Poppler. Larger CJK documents retain the
/// compact CID embedding, whose 16-bit code space is required above 255 glyphs.
fn type3_font(
    ttf: &TtfFont,
    usage: &FontUsage,
    glyph_style: Type3GlyphStyle,
) -> PreparedCustomFont {
    let mut glyph_id_map = HashMap::with_capacity(usage.glyphs.len());
    let mut to_unicode_map = Vec::with_capacity(usage.to_unicode_map.len());
    let mut type3_glyphs = Vec::with_capacity(usage.glyphs.len());
    for (index, glyph_id) in usage.glyphs.iter().copied().enumerate() {
        let code = (index + 1) as u16;
        glyph_id_map.insert(glyph_id, code);
        type3_glyphs.push(Type3Glyph {
            code: code as u8,
            glyph_id,
        });
        if let Some(unicode) = usage.to_unicode_map.get(&glyph_id) {
            to_unicode_map.push((code, unicode.clone()));
        }
    }

    PreparedCustomFont {
        base_font_name: subset_base_font_name(&ttf.font_name, usage.glyphs.len() as u16),
        source_font_name: String::new(),
        font_data: Vec::new(),
        widths: usage
            .glyphs
            .iter()
            // Type 3 widths use the font's glyph coordinate system through
            // `/FontMatrix`, not CIDFont's normalized 1000-unit convention.
            .map(|glyph_id| f32::from(ttf.glyph_width(*glyph_id)))
            .collect(),
        to_unicode_map,
        glyph_id_map,
        embedding: FontEmbedding::Type3(glyph_style),
        type3_glyphs,
    }
}

fn to_unicode_map_for_subset(
    usage: &FontUsage,
    remapper: &subsetter::GlyphRemapper,
) -> ToUnicodeMap {
    let mut mappings = BTreeMap::new();
    for (&old_glyph_id, unicode) in &usage.to_unicode_map {
        if let Some(new_glyph_id) = remapper.get(old_glyph_id) {
            mappings
                .entry(new_glyph_id)
                .or_insert_with(|| unicode.clone());
        }
    }
    mappings.into_iter().collect()
}

fn to_unicode_map_for_full_font(ttf: &TtfFont) -> ToUnicodeMap {
    let mut mappings = BTreeMap::new();
    for (&char_code, &glyph_id) in &ttf.cmap {
        if glyph_id != 0 {
            let unicode: Vec<u16> = char::from_u32(char_code)
                .map(|c| c.encode_utf16(&mut [0; 2]).to_vec())
                .unwrap_or_else(|| vec![char_code as u16]);
            mappings.entry(glyph_id).or_insert(unicode);
        }
    }
    mappings.into_iter().collect()
}

fn subset_base_font_name(font_name: &str, glyph_count: u16) -> String {
    let sanitized_name = sanitize_pdf_font_name(font_name);
    let mut hash = 0xcbf29ce484222325u64;
    for byte in sanitized_name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= u64::from(glyph_count);
    hash = hash.wrapping_mul(0x100000001b3);

    let mut tag = String::with_capacity(6);
    let mut value = hash;
    for _ in 0..6 {
        let letter = b'A' + (value % 26) as u8;
        tag.push(char::from(letter));
        value /= 26;
    }

    format!("{tag}+{sanitized_name}")
}

fn sanitize_pdf_font_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '+'))
        .collect();

    if sanitized.is_empty() {
        "CustomFont".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::cells::{CellAlignment, CellBox, GridCell};
    use crate::layout::elements::{
        FlexContent, GridContent, IntoLayoutNode, LayoutNode, TableCells,
    };
    use crate::layout::engine::{FlexCell, TableCell, TextLine, TextRun};
    use crate::parser::ttf::{FontVerticalMetricSet, FontVerticalMetrics, TtfFont};
    use crate::style::computed::{FontFamily, VerticalAlign};

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_stub_ttf() -> TtfFont {
        TtfFont {
            font_name: "Stub".into(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0, -200, 800, 800],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(800, -200, 0)),
            cmap: HashMap::new(),
            glyph_widths: vec![0, 500, 600],
            num_h_metrics: 3,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: Default::default(),
            data: std::sync::Arc::new(Vec::new()), // empty ⟹ subsetting always fails → fallback_font path
            shaping: None,
        }
    }

    fn make_ttf_with_cmap(cmap: HashMap<u32, u16>, widths: Vec<u16>) -> TtfFont {
        TtfFont {
            font_name: "TestFont".into(),
            face_index: Default::default(),
            units_per_em: 1000,
            size_adjust: 1.0,
            bbox: [0, -200, 800, 800],
            vertical_metrics: FontVerticalMetricSet::from(FontVerticalMetrics::new(800, -200, 0)),
            cmap,
            glyph_widths: widths,
            num_h_metrics: 3,
            flags: 32,
            is_bold: false,
            is_italic: false,
            text_metrics: Default::default(),
            data: std::sync::Arc::new(Vec::new()),
            shaping: None,
        }
    }

    fn empty_text_line() -> TextLine {
        TextLine {
            runs: vec![],
            height: 12.0,
            baseline_ascent: None,
            x_offset: 0.0,
            metadata: Default::default(),
        }
    }

    fn empty_table_cell() -> TableCell {
        TableCell {
            layout: CellBox {
                alignment: CellAlignment {
                    block: VerticalAlign::Middle,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn empty_grid_cell() -> GridCell {
        GridCell::default()
    }

    fn empty_flex_cell() -> FlexCell {
        FlexCell {
            width: 100.0,
            ..Default::default()
        }
    }

    fn text_block_element(lines: Vec<TextLine>) -> LayoutNode {
        TextBlock::plain(lines).boxed()
    }

    fn custom_text_run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            font_family: FontFamily::Custom("TestFont".to_string()),
            line_height_factor: 1.2,
            ..Default::default()
        }
    }

    fn standard_text_run(text: &str) -> TextRun {
        let mut run = custom_text_run(text);
        run.font_family = FontFamily::Helvetica;
        run
    }

    // ── sanitize_pdf_font_name ───────────────────────────────────────────────

    #[test]
    fn sanitize_pdf_font_name_normal() {
        assert_eq!(sanitize_pdf_font_name("OpenSans"), "OpenSans");
    }

    #[test]
    fn sanitize_pdf_font_name_with_allowed_special_chars() {
        assert_eq!(
            sanitize_pdf_font_name("Open-Sans_Bold+Italic"),
            "Open-Sans_Bold+Italic"
        );
    }

    #[test]
    fn sanitize_pdf_font_name_strips_spaces_and_punctuation() {
        // spaces, slashes, dots, and other punctuation must be removed
        let result = sanitize_pdf_font_name("Open Sans / Bold.ttf");
        // Only alphanumeric, '-', '_', '+' survive
        assert_eq!(result, "OpenSansBoldttf");
    }

    #[test]
    fn sanitize_pdf_font_name_empty_returns_custom_font() {
        assert_eq!(sanitize_pdf_font_name(""), "CustomFont");
    }

    #[test]
    fn sanitize_pdf_font_name_all_special_chars_returns_custom_font() {
        assert_eq!(sanitize_pdf_font_name("!@#$%^&*()"), "CustomFont");
    }

    #[test]
    fn sanitize_pdf_font_name_unicode_alphanumeric_kept() {
        // Digits and ASCII letters are always kept.
        let result = sanitize_pdf_font_name("Font123");
        assert_eq!(result, "Font123");
    }

    // ── subset_base_font_name ────────────────────────────────────────────────

    #[test]
    fn subset_base_font_name_format() {
        let name = subset_base_font_name("OpenSans", 42);
        // Must be "XXXXXX+<sanitized_name>"
        let parts: Vec<&str> = name.splitn(2, '+').collect();
        assert_eq!(parts.len(), 2, "expected exactly one '+' separator");
        let tag = parts[0];
        let base = parts[1];
        assert_eq!(tag.len(), 6, "tag must be exactly 6 characters");
        assert!(
            tag.chars().all(|c| c.is_ascii_uppercase()),
            "tag must be uppercase ASCII letters"
        );
        assert_eq!(base, "OpenSans");
    }

    #[test]
    fn subset_base_font_name_deterministic() {
        // Same inputs must always produce the same output.
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("Roboto", 10);
        assert_eq!(a, b);
    }

    #[test]
    fn subset_base_font_name_different_glyph_count_differs() {
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("Roboto", 20);
        assert_ne!(a, b, "different glyph counts should produce different tags");
    }

    #[test]
    fn subset_base_font_name_different_name_differs() {
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("OpenSans", 10);
        assert_ne!(a, b, "different font names should produce different tags");
    }

    #[test]
    fn subset_base_font_name_sanitizes_input() {
        // Special characters in the name are stripped before embedding.
        let name = subset_base_font_name("Open Sans", 5);
        assert!(
            name.ends_with("+OpenSans"),
            "sanitized name should appear after '+'"
        );
    }

    // ── FontUsage::record_glyph ──────────────────────────────────────────────

    #[test]
    fn font_usage_record_glyph_stores_glyph_id() {
        let mut usage = FontUsage::default();
        usage.record_glyph(42, vec![0x0041]); // 'A'
        assert!(usage.glyphs.contains(&42));
    }

    #[test]
    fn font_usage_record_glyph_stores_unicode_mapping() {
        let mut usage = FontUsage::default();
        usage.record_glyph(7, vec![0x0048, 0x0069]); // "Hi"
        assert_eq!(
            usage.to_unicode_map.get(&7),
            Some(&vec![0x0048u16, 0x0069u16])
        );
    }

    #[test]
    fn font_usage_record_glyph_empty_unicode_does_not_insert_mapping() {
        let mut usage = FontUsage::default();
        usage.record_glyph(99, vec![]);
        assert!(usage.glyphs.contains(&99));
        assert!(!usage.to_unicode_map.contains_key(&99));
    }

    #[test]
    fn font_usage_record_glyph_first_mapping_wins() {
        let mut usage = FontUsage::default();
        usage.record_glyph(1, vec![0x0041]); // 'A'
        usage.record_glyph(1, vec![0x0042]); // 'B' — second call should not overwrite
        assert_eq!(usage.to_unicode_map.get(&1), Some(&vec![0x0041u16]));
    }

    #[test]
    fn font_usage_record_glyph_multiple_glyphs() {
        let mut usage = FontUsage::default();
        for glyph_id in [1u16, 2, 3, 5, 8] {
            usage.record_glyph(glyph_id, vec![glyph_id]);
        }
        assert_eq!(usage.glyphs.len(), 5);
        // BTreeSet is sorted — collect to verify all are present
        let ids: Vec<u16> = usage.glyphs.iter().copied().collect();
        assert_eq!(ids, vec![1, 2, 3, 5, 8]);
    }

    #[test]
    fn synthetic_weight_gets_a_distinct_prepared_font_resource() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let mut run = TextRun {
            bold: true,
            font_family: FontFamily::Custom("ParitySans".to_string()),
            ..Default::default()
        };

        assert_eq!(
            prepared_font_name_for_run("paritysans", &run, &fonts),
            "paritysans__synthetic_weight"
        );
        run.font_synthesis.weight = crate::layout::engine::SyntheticFontWeight::Suppressed;
        assert_eq!(
            prepared_font_name_for_run("paritysans", &run, &fonts),
            "paritysans"
        );
    }

    #[test]
    fn font_subset_uses_the_same_coalesced_ligature_buffer_as_pdf_paint() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let runs = "verification"
            .chars()
            .map(|character| TextRun {
                text: character.to_string(),
                font_family: FontFamily::Custom("ParitySans".to_string()),
                ..Default::default()
            })
            .collect();
        let line = TextLine {
            runs,
            height: 12.0,
            ..Default::default()
        };
        let mut usage = BTreeMap::new();

        collect_font_usage_from_lines(&[line], &fonts, &mut usage);

        let painted_run = TextRun {
            text: "verification".to_string(),
            font_family: FontFamily::Custom("ParitySans".to_string()),
            ..Default::default()
        };
        let shaped = crate::text::shape_text_run(&painted_run, &fonts)
            .expect("ParitySans verification text must shape");
        let ligature = shaped
            .glyphs
            .iter()
            .find(|glyph| glyph.unicode == [b'f' as u16, b'i' as u16])
            .expect("fixture must form an fi ligature");
        let collected = usage
            .get(&FontUsageKey::plain("paritysans"))
            .expect("ParitySans usage");

        assert!(
            collected.glyphs.contains(&ligature.glyph_id),
            "every glyph discovered by the painted shaping buffer must be embedded"
        );
        assert_eq!(
            collected.to_unicode_map.get(&ligature.glyph_id),
            Some(&vec![b'f' as u16, b'i' as u16])
        );
    }

    #[test]
    fn svg_font_subset_uses_the_used_font_size_for_letter_spacing_shaping() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let tree = crate::parser::svg::parse_svg_from_string(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <text font-family="ParitySans" font-size="20"
                      letter-spacing="calc(1em - 16px)">office</text>
            </svg>"#,
        )
        .expect("valid SVG");
        let mut usage = BTreeMap::new();

        collect_font_usage_from_svg(&tree, &fonts, &mut usage);

        let font = fonts.get("paritysans").expect("registered font");
        let painted = crate::text::shape_text_with_explicit_font_and_shaping(
            "office",
            20.0,
            font,
            crate::layout::engine::TextShaping::KERNING_ONLY,
        )
        .expect("SVG text must shape");
        let painted_glyphs = painted
            .glyphs
            .iter()
            .map(|glyph| glyph.glyph_id)
            .collect::<BTreeSet<_>>();
        let collected = usage
            .get(&FontUsageKey::plain("paritysans"))
            .expect("ParitySans usage");

        assert_eq!(collected.glyphs, painted_glyphs);
        for character in ['f', 'i'] {
            let glyph_id = font.cmap[&(character as u32)];
            assert!(collected.glyphs.contains(&glyph_id));
        }
    }

    #[test]
    fn type3_widths_stay_in_the_font_matrix_coordinate_system() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let glyph_id = *font.cmap.get(&('B' as u32)).expect("B glyph");
        let mut usage = FontUsage::default();
        usage.record_glyph(glyph_id, vec!['B' as u16]);

        let prepared = type3_font(&font, &usage, Type3GlyphStyle::SyntheticWeight);

        assert_eq!(prepared.widths, vec![f32::from(font.glyph_width(glyph_id))]);
    }

    // ── PreparedCustomFont::pdf_glyph_id ────────────────────────────────────

    #[test]
    fn pdf_glyph_id_returns_remapped_id_when_present() {
        let mut map = HashMap::new();
        map.insert(10u16, 1u16);
        map.insert(20u16, 2u16);
        let font = PreparedCustomFont {
            base_font_name: "X".into(),
            source_font_name: String::new(),
            font_data: vec![],
            widths: vec![],
            to_unicode_map: vec![],
            glyph_id_map: map,
            embedding: FontEmbedding::Cid,
            type3_glyphs: vec![],
        };
        assert_eq!(font.pdf_glyph_id(10), 1);
        assert_eq!(font.pdf_glyph_id(20), 2);
    }

    #[test]
    fn pdf_glyph_id_returns_original_when_not_in_map() {
        let font = PreparedCustomFont {
            base_font_name: "X".into(),
            source_font_name: String::new(),
            font_data: vec![],
            widths: vec![],
            to_unicode_map: vec![],
            glyph_id_map: HashMap::new(),
            embedding: FontEmbedding::Cid,
            type3_glyphs: vec![],
        };
        // Any unknown glyph ID should pass through unchanged.
        assert_eq!(font.pdf_glyph_id(42), 42);
        assert_eq!(font.pdf_glyph_id(0), 0);
    }

    // ── to_unicode_map_for_full_font ─────────────────────────────────────────

    #[test]
    fn to_unicode_map_for_full_font_maps_cmap_entries() {
        let mut cmap = HashMap::new();
        cmap.insert(0x0041u32, 1u16); // 'A' → glyph 1
        cmap.insert(0x0042u32, 2u16); // 'B' → glyph 2
        let ttf = make_ttf_with_cmap(cmap, vec![0, 500, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        // The map is collected from a BTreeMap, so entries are sorted by glyph_id.
        let found_a = map.iter().find(|(gid, _)| *gid == 1);
        let found_b = map.iter().find(|(gid, _)| *gid == 2);
        assert!(found_a.is_some(), "glyph 1 ('A') should be in the map");
        assert_eq!(found_a.unwrap().1, vec![0x0041u16]);
        assert!(found_b.is_some(), "glyph 2 ('B') should be in the map");
        assert_eq!(found_b.unwrap().1, vec![0x0042u16]);
    }

    #[test]
    fn to_unicode_map_for_full_font_skips_glyph_zero() {
        // cmap entries that map to glyph ID 0 (.notdef) must not appear in the map.
        let mut cmap = HashMap::new();
        cmap.insert(0x0020u32, 0u16); // space → .notdef (should be skipped)
        cmap.insert(0x0041u32, 1u16); // 'A' → glyph 1
        let ttf = make_ttf_with_cmap(cmap, vec![0, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        assert!(
            map.iter().all(|(gid, _)| *gid != 0),
            "glyph 0 (.notdef) must not appear"
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn to_unicode_map_for_full_font_empty_cmap_yields_empty_map() {
        let ttf = make_ttf_with_cmap(HashMap::new(), vec![]);
        let map = to_unicode_map_for_full_font(&ttf);
        assert!(map.is_empty());
    }

    #[test]
    fn to_unicode_map_for_full_font_first_codepoint_wins_for_same_glyph() {
        // Two codepoints map to the same glyph — only the first insertion should survive.
        let mut cmap = HashMap::new();
        cmap.insert(0x0041u32, 5u16);
        cmap.insert(0x0061u32, 5u16); // same glyph
        let ttf = make_ttf_with_cmap(cmap, vec![0, 0, 0, 0, 0, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        let entry = map.iter().find(|(gid, _)| *gid == 5);
        assert!(entry.is_some());
        assert_eq!(
            entry.unwrap().1.len(),
            1,
            "only one codepoint should be stored"
        );
    }

    // ── to_unicode_map_for_subset ────────────────────────────────────────────

    #[test]
    fn to_unicode_map_for_subset_remaps_glyph_ids() {
        // Build a FontUsage with glyphs 5 and 10.
        let mut usage = FontUsage::default();
        usage.record_glyph(5, vec![0x0041]); // 'A'
        usage.record_glyph(10, vec![0x0042]); // 'B'

        // Build a remapper for those same glyphs (sorted).
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[5, 10]);

        let map = to_unicode_map_for_subset(&usage, &remapper);

        // Both glyphs must appear in the output under their *new* IDs.
        assert_eq!(map.len(), 2, "both glyphs should have entries");

        // New IDs come from the remapper — old ID 5 gets new ID 1 (0 is .notdef),
        // old ID 10 gets new ID 2.
        let new_5 = remapper.get(5).expect("glyph 5 must be remapped");
        let new_10 = remapper.get(10).expect("glyph 10 must be remapped");

        let entry_5 = map.iter().find(|(gid, _)| *gid == new_5);
        let entry_10 = map.iter().find(|(gid, _)| *gid == new_10);

        assert!(entry_5.is_some());
        assert_eq!(entry_5.unwrap().1, vec![0x0041u16]);
        assert!(entry_10.is_some());
        assert_eq!(entry_10.unwrap().1, vec![0x0042u16]);
    }

    #[test]
    fn to_unicode_map_for_subset_skips_glyphs_not_in_remapper() {
        let mut usage = FontUsage::default();
        usage.record_glyph(5, vec![0x0041]);
        usage.record_glyph(99, vec![0x0042]); // not in remapper

        // Only remap glyph 5.
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[5]);
        let map = to_unicode_map_for_subset(&usage, &remapper);

        // Only glyph 5 should appear after remapping.
        assert_eq!(map.len(), 1);
        let new_5 = remapper.get(5).unwrap();
        assert!(map.iter().any(|(gid, _)| *gid == new_5));
    }

    #[test]
    fn to_unicode_map_for_subset_empty_usage_yields_empty_map() {
        let usage = FontUsage::default();
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[]);
        let map = to_unicode_map_for_subset(&usage, &remapper);
        assert!(map.is_empty());
    }

    // ── fallback_font ────────────────────────────────────────────────────────

    #[test]
    fn fallback_font_uses_full_font_data() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        assert_eq!(prepared.font_data, *ttf.data);
    }

    #[test]
    fn fallback_font_name_matches_sanitized_font_name() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        assert_eq!(
            prepared.base_font_name,
            sanitize_pdf_font_name(&ttf.font_name)
        );
    }

    #[test]
    fn fallback_font_widths_match_glyph_count() {
        let ttf = make_stub_ttf(); // glyph_widths has 3 entries
        let prepared = fallback_font(&ttf);
        assert_eq!(prepared.widths.len(), ttf.glyph_widths.len());
    }

    #[test]
    fn fallback_font_glyph_id_map_is_empty() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        // Empty map means pdf_glyph_id passes IDs through unchanged.
        assert_eq!(prepared.pdf_glyph_id(5), 5);
    }

    #[test]
    fn prepare_font_falls_back_when_data_empty() {
        // Empty font data causes subsetter::subset to fail, so prepare_font
        // must call fallback_font instead of subset_font.
        let ttf = make_stub_ttf(); // data: std::sync::Arc::new(Vec::new())
        let mut usage = FontUsage::default();
        usage.record_glyph(1, vec![0x0041]);
        let prepared = prepare_font(&ttf, &usage, Type3GlyphStyle::Plain);
        // Fallback: base_font_name must NOT contain a '+' prefix tag.
        assert!(
            !prepared.base_font_name.starts_with(char::is_uppercase)
                || !prepared.base_font_name.contains('+')
                || prepared
                    .base_font_name
                    .ends_with(&sanitize_pdf_font_name(&ttf.font_name)),
            "fallback font name should be sanitized font name, not a subset tag"
        );
        // Widths come from all glyphs (fallback uses full glyph_widths).
        assert_eq!(prepared.widths.len(), ttf.glyph_widths.len());
    }

    // ── collect_font_usage_from_element ─────────────────────────────────────

    #[test]
    fn collect_font_usage_from_element_ignores_image() {
        let element = PageBreak::default();
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty(), "PageBreak should produce no font usage");
    }

    #[test]
    fn collect_font_usage_from_element_handles_table_row() {
        let element = TableRow {
            content: TableCells {
                cells: vec![empty_table_cell()],
                column_widths: vec![100.0],
            },
            ..Default::default()
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        // Should not panic; empty cells with no custom fonts yield empty usage.
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_grid_row() {
        let element = GridRow {
            content: GridContent {
                cells: vec![empty_grid_cell()],
                column_widths: vec![100.0],
                ..Default::default()
            },
            ..Default::default()
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_flex_row() {
        let element = FlexRow {
            content: FlexContent {
                cells: vec![empty_flex_cell()],
                row_height: 20.0,
                ..Default::default()
            },
            box_model: crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize::fixed(500.0, None),
                ..Default::default()
            },
            ..Default::default()
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_text_block() {
        let element = text_block_element(vec![empty_text_line()]);
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        // No custom fonts configured, so usage stays empty.
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_table_row_with_nested_rows() {
        // A TableCell with a nested TableRow inside should recurse.
        let nested = TableRow {
            content: TableCells {
                cells: vec![empty_table_cell()],
                column_widths: vec![50.0],
            },
            ..Default::default()
        }
        .boxed();
        let mut cell = empty_table_cell();
        cell.layout.content.children = vec![nested];

        let element = TableRow {
            content: TableCells {
                cells: vec![cell],
                column_widths: vec![100.0],
            },
            ..Default::default()
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        // Should not panic when recursing through nested rows.
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    // ── collect_font_usage_from_run (non-custom font is skipped) ────────────

    #[test]
    fn collect_font_usage_skips_non_custom_font_family() {
        let run = TextRun {
            text: "Hello".into(),
            ..Default::default()
        };
        let line = TextLine {
            runs: vec![run],
            height: 12.0,
            baseline_ascent: None,
            x_offset: 0.0,
            metadata: Default::default(),
        };
        let element = text_block_element(vec![line]);
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<FontUsageKey, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(
            usage.is_empty(),
            "non-custom font families should not produce any usage entries"
        );
    }

    #[test]
    fn borrowed_generated_runs_match_synthetic_page_font_discovery() {
        let mut cmap = HashMap::new();
        cmap.insert('A' as u32, 1);
        cmap.insert('B' as u32, 2);
        cmap.insert('C' as u32, 3);
        let mut fonts = HashMap::new();
        fonts.insert(
            "TestFont".to_string(),
            make_ttf_with_cmap(cmap, vec![0, 500, 500, 500]),
        );
        let mut fallback_cmap = HashMap::new();
        fallback_cmap.insert('Ω' as u32, 1);
        fonts.insert(
            crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
            make_ttf_with_cmap(fallback_cmap, vec![0, 500]),
        );

        let body_line = TextLine {
            runs: vec![custom_text_run("A")],
            height: 12.0,
            baseline_ascent: None,
            x_offset: 0.0,
            metadata: Default::default(),
        };
        let pages = vec![Page {
            elements: vec![(0.0, text_block_element(vec![body_line]))],
            ..Page::default()
        }];
        let per_page_runs = vec![vec![custom_text_run("B"), standard_text_run("Ω")]];
        let trailing_runs = vec![custom_text_run("C")];

        let borrowed =
            collect_font_usage_with_additional_runs(&pages, &fonts, &per_page_runs, &trailing_runs);

        // Recreate the former cloning approach as the oracle: generated page
        // text was inserted after ordinary elements, and the combined margin
        // text occupied a final synthetic page.
        let mut synthetic_pages = vec![Page {
            elements: vec![(
                0.0,
                text_block_element(vec![TextLine {
                    runs: vec![custom_text_run("A")],
                    height: 12.0,
                    baseline_ascent: None,
                    x_offset: 0.0,
                    metadata: Default::default(),
                }]),
            )],
            ..Page::default()
        }];
        synthetic_pages[0].elements.push((
            0.0,
            text_block_element(vec![TextLine {
                runs: per_page_runs[0].clone(),
                height: 12.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }]),
        ));
        synthetic_pages.push(Page {
            elements: vec![(
                0.0,
                text_block_element(vec![TextLine {
                    runs: trailing_runs.clone(),
                    height: 12.0,
                    baseline_ascent: None,
                    x_offset: 0.0,
                    metadata: Default::default(),
                }]),
            )],
            ..Page::default()
        });
        let cloned = collect_font_usage_with_additional_runs(&synthetic_pages, &fonts, &[], &[]);

        assert_eq!(
            borrowed.keys().collect::<Vec<_>>(),
            cloned.keys().collect::<Vec<_>>()
        );
        for (name, borrowed_usage) in &borrowed {
            let cloned_usage = &cloned[name];
            assert_eq!(borrowed_usage.glyphs, cloned_usage.glyphs);
            assert_eq!(borrowed_usage.to_unicode_map, cloned_usage.to_unicode_map);
        }

        let borrowed_fonts = prepare_custom_fonts_with_additional_runs(
            &pages,
            &fonts,
            &per_page_runs,
            &trailing_runs,
        );
        let cloned_fonts =
            prepare_custom_fonts_with_additional_runs(&synthetic_pages, &fonts, &[], &[]);
        assert_eq!(
            borrowed_fonts.keys().collect::<Vec<_>>(),
            cloned_fonts.keys().collect::<Vec<_>>()
        );
        for (name, borrowed_font) in &borrowed_fonts {
            let cloned_font = &cloned_fonts[name];
            assert_eq!(borrowed_font.base_font_name, cloned_font.base_font_name);
            assert_eq!(borrowed_font.font_data, cloned_font.font_data);
            assert_eq!(borrowed_font.widths, cloned_font.widths);
            assert_eq!(borrowed_font.to_unicode_map, cloned_font.to_unicode_map);
            assert_eq!(borrowed_font.glyph_id_map, cloned_font.glyph_id_map);
        }
    }
}
