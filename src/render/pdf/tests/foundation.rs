use super::*;
use crate::layout::elements::{
    IntoLayoutNode, LayoutElementTestExt, LayoutElementTestMutExt, LayoutNode, PageBreak,
    ReplacedGeometry, Svg, SvgPaint, TableCells, TableRow, TextBlock,
};
use crate::layout::engine::{LayoutBorder, layout};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::html::parse_html;
use crate::style::computed::AlignSelf;
use crate::types::{Color, Size};

const TEST_PAGE_PAINT_BOX: PdfRect = PdfRect::new(0.0, 0.0, 612.0, 792.0);

const TEST_JPEG_DATA_URI: &str = concat!(
    "data:image/jpeg;base64,",
    "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
    "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
    "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
);

fn filled_rect_heights(content: &str) -> Vec<f32> {
    let lines: Vec<_> = content.lines().collect();
    lines
        .windows(2)
        .filter_map(|pair| {
            if pair[1] != "f" || !pair[0].ends_with(" re") {
                return None;
            }
            let parts: Vec<_> = pair[0].split_whitespace().collect();
            parts
                .get(parts.len().saturating_sub(2))
                .and_then(|value| value.parse::<f32>().ok())
        })
        .collect()
}

fn filled_rect_count(content: &str) -> usize {
    filled_rect_heights(content).len()
}

fn test_text_run(text: impl Into<String>) -> TextRun {
    TextRun {
        text: text.into(),
        ..Default::default()
    }
}

fn gradient_stop(
    position: f32,
    color: crate::types::Color,
) -> crate::style::computed::GradientStop {
    crate::style::computed::GradientStop::new(
        crate::style::computed::GradientColor::new(
            color,
            crate::style::computed::GradientColorProvenance::LegacySrgb,
        ),
        Some(crate::style::computed::GradientPosition::fraction(position)),
    )
}

fn gradient_ramp(
    stops: impl IntoIterator<Item = crate::style::computed::GradientStop>,
    repeating: bool,
) -> GradientRamp {
    GradientRamp {
        stops: stops.into_iter().collect(),
        repeat: if repeating {
            crate::style::computed::GradientRepeat::Repeat
        } else {
            crate::style::computed::GradientRepeat::Clamp
        },
        ..Default::default()
    }
}

fn test_conic_gradient(
    stops: impl IntoIterator<Item = crate::style::computed::GradientStop>,
    repeating: bool,
) -> crate::style::computed::ConicGradient {
    crate::style::computed::ConicGradient {
        from_angle: 0.0,
        center: crate::style::computed::RadialPoint::default(),
        ramp: gradient_ramp(stops, repeating),
        layer_box: crate::style::computed::GradientLayerBox::default(),
    }
}

fn assert_rgba_close(actual: (f32, f32, f32, f32), expected: (f32, f32, f32, f32)) {
    for (actual, expected) in [actual.0, actual.1, actual.2, actual.3]
        .into_iter()
        .zip([expected.0, expected.1, expected.2, expected.3])
    {
        assert!(
            (actual - expected).abs() <= 1e-6,
            "expected {expected}, got {actual}"
        );
    }
}

fn rasterize_mask_grid_by_tiles(
    grid: MaskRasterGrid,
    max_edge: u32,
    mut rasterize: impl FnMut(MaskRasterWindow) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let mut coverage = vec![0; grid.full_window().len()?];
    let grid_width = usize::try_from(grid.pixels.width).ok()?;
    for tile in grid.pixels.tiles(max_edge)? {
        let window = grid.window(tile)?;
        let source = rasterize(window)?;
        if source.len() != window.len()? {
            return None;
        }
        let tile_width = usize::try_from(tile.width).ok()?;
        for row in 0..usize::try_from(tile.height).ok()? {
            let source_start = row.checked_mul(tile_width)?;
            let destination_start = usize::try_from(tile.y)
                .ok()?
                .checked_add(row)?
                .checked_mul(grid_width)?
                .checked_add(usize::try_from(tile.x).ok()?)?;
            coverage[destination_start..destination_start + tile_width]
                .copy_from_slice(&source[source_start..source_start + tile_width]);
        }
    }
    Some(coverage)
}

fn layout_svg_clip_reference_document(rect: Option<[u32; 4]>) -> Vec<Page> {
    let defs = rect.map_or_else(String::new, |[x, y, width, height]| {
            format!(
                r#"<svg style="display:none"><defs><clipPath id="same-id" clipPathUnits="userSpaceOnUse"><rect x="{x}" y="{y}" width="{width}" height="{height}"/></clipPath></defs></svg>"#
            )
        });
    let html = format!(
        r#"{defs}<div style="width:80pt;height:80pt;background:red;clip-path:url(#same-id)"></div>"#
    );
    let nodes = parse_html(&html).unwrap();
    layout(&nodes, PageSize::A4, Margin::default())
}

fn render_svg_clip_reference_document(pages: &[Page]) -> String {
    String::from_utf8_lossy(&render_pdf(pages, PageSize::A4, Margin::default()).unwrap())
        .into_owned()
}

#[test]
fn svg_fragment_defs_are_isolated_between_sequential_documents() {
    const A_RECT: &str = "11 13 17 19 re\nW n";
    const B_RECT: &str = "23 29 31 37 re\nW n";

    let document_a = layout_svg_clip_reference_document(Some([11, 13, 17, 19]));
    assert!(
        document_a[0]
            .document_svg_defs
            .clip_paths
            .contains_key("same-id")
    );
    let pdf_a = render_svg_clip_reference_document(&document_a);
    assert!(pdf_a.contains(A_RECT));

    let document_b = layout_svg_clip_reference_document(Some([23, 29, 31, 37]));
    let pdf_b = render_svg_clip_reference_document(&document_b);
    assert!(pdf_b.contains(B_RECT));
    assert!(!pdf_b.contains(A_RECT));

    let missing_definition = layout_svg_clip_reference_document(None);
    assert!(
        missing_definition[0]
            .document_svg_defs
            .clip_paths
            .is_empty()
    );
    let missing_pdf = render_svg_clip_reference_document(&missing_definition);
    assert!(!missing_pdf.contains(A_RECT));
    assert!(!missing_pdf.contains(B_RECT));
}

#[test]
fn svg_fragment_defs_are_isolated_between_parallel_documents() {
    const A_RECT: &str = "41 43 47 53 re\nW n";
    const B_RECT: &str = "59 61 67 71 re\nW n";
    let barrier = std::sync::Barrier::new(2);

    let (pdf_a, pdf_b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            let pages = layout_svg_clip_reference_document(Some([41, 43, 47, 53]));
            barrier.wait();
            render_svg_clip_reference_document(&pages)
        });
        let b = scope.spawn(|| {
            let pages = layout_svg_clip_reference_document(Some([59, 61, 67, 71]));
            barrier.wait();
            render_svg_clip_reference_document(&pages)
        });
        (a.join().unwrap(), b.join().unwrap())
    });

    assert!(pdf_a.contains(A_RECT));
    assert!(!pdf_a.contains(B_RECT));
    assert!(pdf_b.contains(B_RECT));
    assert!(!pdf_b.contains(A_RECT));
}

fn test_footnote(text: impl Into<String>) -> FootnoteItem {
    FootnoteItem {
        marker: String::new(),
        text: text.into(),
        body: crate::layout::engine::FootnoteBodyStyle {
            font_size: 10.0,
            line_height_factor: 1.0,
            ..Default::default()
        },
        marker_color: Color::BLACK,
        marker_prefix: String::new(),
        formatting: Default::default(),
        structure: Default::default(),
    }
}

#[test]
fn footnote_padding_positions_text_and_separator_paint() {
    let footnote = test_footnote("note");
    let footnotes = std::slice::from_ref(&footnote);
    let page_size = PageSize::new(100.0, 100.0);
    let margin = Margin::uniform(10.0);
    let area = ResolvedFootnoteAreaStyle {
        padding: EdgeSizes::new(2.0, 3.0, 4.0, 5.0),
        separator: crate::layout::paginate::FootnoteSeparator {
            width: 1.5,
            color: crate::types::Color::rgb(255, 0, 0),
        },
    };
    let lines = wrapped_footnote_lines(footnotes, 72.0, &HashMap::new());
    let total_height = lines.iter().map(|line| line.height).sum::<f32>();
    let metrics = line_box_metrics(&lines[0], &HashMap::new());
    let expected_baseline = margin.bottom + area.padding.bottom + total_height
        - metrics.half_leading
        - metrics.ascender;
    let expected_separator_y =
        margin.bottom + area.padding.bottom + total_height + area.padding.top;
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut page_ext_gstates = Vec::new();
    let mut alpha_counter = 0;

    render_page_footnotes(
        &mut content,
        footnotes,
        page_size,
        margin,
        area,
        &HashMap::new(),
        &PreparedCustomFonts::new(),
        &mut pdf_writer,
        &mut page_images,
        &mut page_ext_gstates,
        &mut alpha_counter,
    );

    assert!(content.contains(&format!(
        "1 0 0 rg\n10 {} 80 1.5 re\nf\n",
        format_pdf_number(expected_separator_y)
    )));
    assert!(content.contains(&format!("15 {} Td\n", format_pdf_number(expected_baseline))));
}

fn synthetic_weight_test_fonts() -> HashMap<String, TtfFont> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fonts/ParitySans.ttf"),
    )
    .expect("ParitySans test font");
    let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
    HashMap::from([("paritysans".to_string(), font)])
}

fn render_synthetic_weight(weight: crate::layout::engine::SyntheticFontWeight) -> String {
    let fonts = synthetic_weight_test_fonts();
    let run = TextRun {
        text: "Weight".to_string(),
        font_size: 20.0,
        bold: true,
        font_family: FontFamily::Custom("ParitySans".to_string()),
        font_synthesis: crate::layout::engine::FontSynthesisState {
            weight,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();

    render_run_glyphs(
        &mut content,
        &run,
        10.0,
        20.0,
        run.font_size,
        &fonts,
        &PreparedCustomFonts::new(),
        0.0,
        &mut pdf_writer,
        &mut Vec::new(),
    );
    content
}

#[test]
fn pdf_text_uses_typed_synthetic_weight_stroke() {
    use crate::layout::engine::SyntheticFontWeight;

    let content = render_synthetic_weight(SyntheticFontWeight::Auto);
    let expected = SyntheticFontWeight::Auto
        .stroke_width(20.0)
        .expect("automatic synthetic weight has a finite stroke");
    assert!(content.contains(&format!("{} w\n2 Tr\n", format_pdf_number(expected))));
    assert!(content.contains("0 Tr\n"));

    let suppressed = render_synthetic_weight(SyntheticFontWeight::Suppressed);
    assert!(!suppressed.contains("2 Tr\n"));
    assert!(!suppressed.contains("0 Tr\n"));
}

#[test]
fn emphasis_mark_uses_the_css_family_after_glyph_fallback() {
    let run = TextRun {
        font_family: FontFamily::Helvetica,
        ..test_text_run("重要")
    }
    .with_glyph_fallback(FontFamily::Custom("japanese".to_string()));

    assert_eq!(emphasis_mark_run(&run).font_family, FontFamily::Helvetica);
}

#[test]
fn explicit_run_metadata_drives_decoration_and_drop_cap_state() {
    let mut run = test_text_run("Decorated");
    run.border_radii = CornerRadii::circular(6.0);
    run.line_height_factor = 1.25;
    run.decorations
        .push(crate::style::computed::TextDecoration {
            style: crate::style::computed::TextDecorationStyle::Wavy,
            thickness: Some(1.25),
            underline_offset: Some(2.5),
            ..Default::default()
        });
    run.metadata.emphasis.mark = true;
    run.metadata.spacing.letter = 0.375;
    run.metadata.is_drop_cap = true;

    let decoration = &run.decorations[0];
    assert!(decoration_is_wavy(decoration));
    assert!(decoration_is_emphasis(&run));
    assert_eq!(decoration_thickness(&run, decoration), 1.25);
    assert_eq!(underline_center_y(&run, decoration, 10.0), 6.875);
    assert_eq!(text_run_letter_spacing(&run), 0.375);
    assert!(is_drop_cap_run(&run));
    assert_eq!(run.border_radii, CornerRadii::circular(6.0));
    assert_eq!(run.line_height_factor, 1.25);
}

#[test]
fn decoration_uses_the_css_device_pixel_floor_without_rescaling_offset() {
    let mut run = test_text_run("thin");
    run.font_size = 1.0;
    run.decorations
        .push(crate::style::computed::TextDecoration {
            thickness: Some(0.075),
            underline_offset: Some(-0.125),
            ..Default::default()
        });

    assert_eq!(
        decoration_thickness(&run, &run.decorations[0]),
        crate::fonts::PT_PER_CSS_PX
    );
    assert_eq!(underline_center_y(&run, &run.decorations[0], 3.0), 2.75);

    let mut solid = String::new();
    push_decoration_stroke(
        &mut solid,
        &run,
        &run.decorations[0],
        DecorationStroke::new(
            (0.0, 0.0, 0.0),
            DecorationLine::Underline,
            1.0,
            2.0,
            3.0,
            0.0,
        ),
    );
    assert!(solid.contains("1 2.625 1 0.75 re"));

    run.decorations[0].style = crate::style::computed::TextDecorationStyle::Wavy;
    let mut wavy = String::new();
    push_decoration_stroke(
        &mut wavy,
        &run,
        &run.decorations[0],
        DecorationStroke::new(
            (0.0, 0.0, 0.0),
            DecorationLine::Underline,
            1.0,
            2.0,
            3.0,
            0.0,
        ),
    );
    assert!(wavy.contains("0.75 w"));
    assert!(wavy.contains("-6.5 1.5 m"));

    run.decorations[0].style = crate::style::computed::TextDecorationStyle::Solid;
    run.decorations[0].thickness = Some(0.0);
    let mut zero = String::new();
    push_decoration_stroke(
        &mut zero,
        &run,
        &run.decorations[0],
        DecorationStroke::new(
            (0.0, 0.0, 0.0),
            DecorationLine::Underline,
            1.0,
            2.0,
            3.0,
            0.0,
        ),
    );
    assert!(zero.contains("1 2.625 1 0.75 re"));
}

#[test]
fn shared_horizontal_decoration_painter_emits_wavy_line_and_shadow_layers() {
    let mut run = test_text_run("wave");
    run.decorations
        .push(crate::style::computed::TextDecoration {
            lines: crate::style::computed::TextDecorationLines {
                underline: true,
                ..Default::default()
            },
            color: Some(Color::rgb(239, 71, 111)),
            style: crate::style::computed::TextDecorationStyle::Wavy,
            ..Default::default()
        });
    run.text_shadow.push(crate::style::computed::BoxShadow {
        offset_x: 1.0,
        offset_y: 1.0,
        blur: 0.0,
        spread: 0.0,
        color: Color::rgb(255, 255, 255),
        color_source: crate::style::computed::ColorSource::Absolute,
        inset: false,
    });

    let custom_fonts = HashMap::new();
    let decoration = HorizontalRunDecorations::new(&run, 2.0, 20.0, 10.0, &custom_fonts);
    let mut content = String::new();
    decoration.paint_shadows(&mut content);
    decoration.paint_below_text(&mut content);
    decoration.paint_above_text(&mut content);

    assert!(content.contains("1 1 1 RG"), "{content}");
    assert!(content.contains("0.9373 0.2784 0.4353 RG"), "{content}");
    assert_eq!(content.matches(" m\n").count(), 2, "{content}");
}

#[test]
fn horizontal_line_paints_underlines_below_glyphs_and_line_through_above() {
    let mut run = test_text_run("Decorated");
    run.decorations
        .push(crate::style::computed::TextDecoration {
            lines: crate::style::computed::TextDecorationLines {
                underline: true,
                line_through: true,
                ..Default::default()
            },
            color: Some(Color::rgb(239, 71, 111)),
            ..Default::default()
        });
    let mut content = String::new();
    let mut writer = PdfWriter::new();
    let mut images = Vec::new();

    paint_horizontal_line_text(
        &mut content,
        &[run],
        HorizontalLinePaint {
            origin: PdfPoint::new(2.0, 10.0),
            line_ascender: 9.0,
            justification_word_spacing: 0.0,
            text_space: PdfContentSpace::Points,
        },
        &HashMap::new(),
        &PreparedCustomFonts::new(),
        &mut writer,
        &mut images,
    );

    let decoration_color = PdfRgb::from(Color::rgb(239, 71, 111)).fill_operator();
    let color_positions = content
        .match_indices(&decoration_color)
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let glyph = content.find("(Decorated) Tj").expect("text glyph operator");
    assert_eq!(color_positions.len(), 2, "{content}");
    assert!(color_positions[0] < glyph, "{content}");
    assert!(glyph < color_positions[1], "{content}");
}

#[test]
fn horizontal_line_preserves_independent_decoration_origins() {
    let mut run = test_text_run("Decorated");
    run.decorations = vec![
        crate::style::computed::TextDecoration {
            lines: crate::style::computed::TextDecorationLines {
                underline: true,
                ..Default::default()
            },
            color: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        },
        crate::style::computed::TextDecoration {
            lines: crate::style::computed::TextDecorationLines {
                line_through: true,
                ..Default::default()
            },
            color: Some(Color::rgb(0, 0, 255)),
            ..Default::default()
        },
    ];
    let mut content = String::new();

    paint_horizontal_line_text(
        &mut content,
        &[run],
        HorizontalLinePaint {
            origin: PdfPoint::new(2.0, 10.0),
            line_ascender: 9.0,
            justification_word_spacing: 0.0,
            text_space: PdfContentSpace::Points,
        },
        &HashMap::new(),
        &PreparedCustomFonts::new(),
        &mut PdfWriter::new(),
        &mut Vec::new(),
    );

    let underline = content.find("1 0 0 rg").expect("red underline paint");
    let glyph = content.find("(Decorated) Tj").expect("text glyph operator");
    let line_through = content.find("0 0 1 rg").expect("blue line-through paint");
    assert!(underline < glyph, "{content}");
    assert!(glyph < line_through, "{content}");
}

#[test]
fn automatic_decoration_thickness_uses_the_same_device_floor() {
    let mut run = test_text_run("auto");
    run.font_size = 1.0;
    run.decorations.push(Default::default());
    assert_eq!(
        decoration_thickness(&run, &run.decorations[0]),
        crate::fonts::PT_PER_CSS_PX
    );
    assert_eq!(underline_center_y(&run, &run.decorations[0], 3.0), 1.875);

    run.font_size = 25.5;
    run.decorations[0].thickness = Some(4.5);
    assert_eq!(underline_center_y(&run, &run.decorations[0], 30.0), 25.5);

    run.decorations[0].underline_offset = Some(0.0);
    assert_eq!(underline_center_y(&run, &run.decorations[0], 30.0), 27.75);
    run.decorations[0].style = crate::style::computed::TextDecorationStyle::Wavy;
    assert_eq!(decoration_thickness(&run, &run.decorations[0]), 4.5);
}

#[test]
fn emphasis_marks_keep_a_color_distinct_from_overline() {
    let mut run = test_text_run("AB");
    run.decorations
        .push(crate::style::computed::TextDecoration {
            lines: crate::style::computed::TextDecorationLines {
                overline: true,
                ..Default::default()
            },
            color: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        });
    run.metadata.emphasis.mark = true;
    run.metadata.emphasis.color = Color::rgb(0, 0, 255);

    let page = test_page(vec![(0.0, test_text_block_from_runs(vec![run]))]);
    let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);

    let red_overline = content
        .find("1 0 0 rg")
        .expect("overline must retain its red paint");
    let blue_emphasis = content
        .find("0 0 1 rg")
        .expect("emphasis marks must retain their blue paint");
    assert!(red_overline < blue_emphasis);
}

fn test_text_line(runs: Vec<TextRun>) -> TextLine {
    TextLine {
        runs,
        height: 14.0,
        baseline_ascent: None,
        x_offset: 0.0,
        metadata: Default::default(),
    }
}

fn baseline_flex_cell(line_id: FlexLineId, y_offset: f32, baseline: f32) -> FlexCell {
    FlexCell {
        lines: vec![TextLine {
            runs: vec![test_text_run("baseline")],
            height: baseline + 1.0,
            baseline_ascent: Some(baseline),
            ..Default::default()
        }],
        align_self: AlignSelf::Baseline,
        line_id,
        y_offset,
        ..Default::default()
    }
}

#[test]
fn flex_baseline_grouping_uses_line_identity_not_subpoint_offsets() {
    let first = FlexLineId::from_index(0);
    let second = FlexLineId::from_index(1);
    let cells = vec![
        baseline_flex_cell(first, 10.0, 4.0),
        baseline_flex_cell(first, 10.0, 7.0),
        baseline_flex_cell(second, 10.005, 19.0),
    ];
    let fonts = HashMap::new();

    assert_eq!(
        flex_line_max_baseline(&cells, first, AlignItems::Baseline, &fonts),
        Some(7.0),
        "cells owned by one flex line must share its maximum baseline"
    );
    assert_eq!(
        flex_line_max_baseline(&cells, second, AlignItems::Baseline, &fonts),
        Some(19.0),
        "a distinct line only 0.005pt away must remain independent"
    );
}

fn test_text_block(lines: Vec<TextLine>) -> LayoutNode {
    TextBlock::plain(lines).boxed()
}

fn test_text_block_from_runs(runs: Vec<TextRun>) -> LayoutNode {
    test_text_block(vec![test_text_line(runs)])
}

fn test_table_row(cells: Vec<TableCell>, column_widths: Vec<f32>) -> TableRow {
    TableRow {
        content: TableCells {
            cells,
            column_widths,
        },
        ..Default::default()
    }
}

fn test_page(elements: Vec<(f32, LayoutNode)>) -> Page {
    Page {
        elements,
        ..Default::default()
    }
}

fn has_axial_gradient_pattern(pdf: &str) -> bool {
    pdf.contains("/PatternType 2")
        && pdf.contains("/ShadingType 2")
        && pdf.contains("/Pattern cs")
        && pdf.contains(" scn\n")
}

#[test]
fn overflowing_page_does_not_add_an_overflow_scale() {
    let page_size = PageSize::new(100.0, 100.0);
    let mut overflowing = test_text_block_from_runs(vec![test_text_run("overflow")]);
    overflowing.update_text(|block| {
        block.box_model.size.width = crate::layout::elements::InlineSize::fixed(200.0);
        block.positioning.insets.left = 20.0;
    });
    let pdf = render_pdf(
        &[test_page(vec![(0.0, overflowing)])],
        page_size,
        Margin::uniform(0.0),
    )
    .unwrap();
    let content = String::from_utf8_lossy(&pdf);

    assert!(content.contains("q 1 0 0 1 0 0 cm\n"));
    assert!(!content.contains("0.5 0 0 0.5 0 0 cm"));
}

#[test]
fn text_shadow_does_not_move_foreground_text() {
    fn foreground_td(pdf: &[u8], text: &str) -> String {
        let content = String::from_utf8_lossy(pdf);
        let marker = format!("({text}) Tj");
        let marker_pos = content.rfind(&marker).expect("foreground text");
        content[..marker_pos]
            .lines()
            .rev()
            .find(|line| line.ends_with(" Td") || line.ends_with(" Tm"))
            .expect("foreground text position")
            .to_string()
    }

    let plain = test_text_run("shadow-invariant");
    let mut shadowed = plain.clone();
    shadowed
        .text_shadow
        .push(crate::style::computed::BoxShadow {
            offset_x: 0.0,
            offset_y: 0.0,
            blur: 0.0,
            spread: 0.0,
            color: crate::types::Color::rgba8(0, 0, 0, 0),
            color_source: crate::style::computed::ColorSource::Absolute,
            inset: false,
        });
    let page_size = PageSize::new(200.0, 100.0);
    let margin = Margin::uniform(10.0);
    let plain_pdf = render_pdf(
        &[test_page(vec![(
            0.0,
            test_text_block_from_runs(vec![plain]),
        )])],
        page_size,
        margin,
    )
    .unwrap();
    let shadowed_pdf = render_pdf(
        &[test_page(vec![(
            0.0,
            test_text_block_from_runs(vec![shadowed]),
        )])],
        page_size,
        margin,
    )
    .unwrap();

    assert_eq!(
        foreground_td(&plain_pdf, "shadow-invariant"),
        foreground_td(&shadowed_pdf, "shadow-invariant")
    );
}
