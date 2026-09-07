use super::*;

/// A link annotation to be placed on a PDF page.
pub(super) struct LinkAnnotation {
    pub(super) rect: PdfRect,
    pub(super) url: String,
}

pub(super) fn text_run_link_annotation(run: &TextRun, rect: PdfRect) -> Option<LinkAnnotation> {
    let url = run.link_url.as_ref()?;
    if decode_footnote_link(url).is_some() {
        return None;
    }
    if is_internal_target_anchor(url) {
        return None;
    }
    Some(LinkAnnotation {
        rect,
        url: url.clone(),
    })
}

/// A bookmark entry for PDF outline (table of contents).
#[allow(dead_code)]
pub(super) struct BookmarkEntry {
    pub(super) title: String,
    pub(super) level: u8,
    pub(super) page_index: usize,
    pub(super) y_pos: f32,
}

/// Render laid-out pages into a PDF byte buffer.
///
/// Uses the PDF built-in Helvetica font family (one of the 14 standard fonts)
/// so no font embedding is needed for the MVP.
#[allow(dead_code)]
pub fn render_pdf(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
) -> Result<Vec<u8>, IronpressError> {
    render_pdf_with_fonts(pages, page_size, margin, &HashMap::new())
}

/// Render laid-out pages into a PDF byte buffer, with custom font embedding.
pub fn render_pdf_with_fonts(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Result<Vec<u8>, IronpressError> {
    let mut buf = Vec::new();
    render_pdf_to_writer_with_fonts(pages, page_size, margin, &mut buf, custom_fonts)?;
    Ok(buf)
}

/// Header, footer, page-margin, and physical-sheet decoration.
#[derive(Default)]
pub struct PageDecoration {
    /// Header text rendered top-center of each page.
    pub header: Option<String>,
    /// Footer text rendered bottom-center of each page.
    /// `{page}` and `{pages}` are replaced with page number and total count.
    pub footer: Option<String>,
    /// CSS `@page` margin boxes (CSS Paged Media 3 §5) — running headers/footers
    /// and page counters declared via `@top-center { content: … }` etc. Rendered
    /// on every page with `counter(page)`/`counter(pages)` resolved per page.
    pub margin_boxes: Vec<crate::parser::css::MarginBox>,
    /// Cascaded page-context text inherited by page-margin boxes.
    pub margin_text: crate::layout::engine::PageMarginTextContext,
    /// Resolved physical-sheet bleed, printer marks, and orientation.
    pub(crate) sheet: PageSheet,
    /// CSS GCPM `@footnote` area declarations.
    pub footnote_area: ResolvedFootnoteAreaStyle,
}

pub(super) fn page_margin_box_applies(
    selector: &crate::parser::css::PageSelector,
    page: &Page,
    page_num: usize,
) -> bool {
    selector.applies_to(crate::parser::css::PageSelectorContext {
        page_number: page_num,
        is_blank: page.is_blank,
        page_name: page.page_name.as_deref(),
    })
}

pub(super) fn page_counter_value(mb: &crate::parser::css::MarginBox, page_num: usize) -> usize {
    mb.page_counter.value_on_page(page_num)
}

/// Whether an applicable page-margin box survives the page-selector cascade.
pub(super) fn page_margin_box_wins(
    boxes: &[crate::parser::css::MarginBox],
    box_index: usize,
    page: &Page,
    page_num: usize,
) -> bool {
    let margin_box = &boxes[box_index];
    page_margin_box_applies(&margin_box.selector, page, page_num)
        && !boxes.iter().enumerate().any(|(other_index, other)| {
            other.position == margin_box.position
                && page_margin_box_applies(&other.selector, page, page_num)
                && (page_selector_specificity(&other.selector)
                    > page_selector_specificity(&margin_box.selector)
                    || (page_selector_specificity(&other.selector)
                        == page_selector_specificity(&margin_box.selector)
                        && other_index > box_index))
        })
}

/// Whether a center margin box receives the whole page-width flex track.
///
/// CSS Paged Media's variable-dimension algorithm gives a generated center
/// box the available track when neither side peer participates. This is the
/// common running-header case; declared widths always take precedence.
pub(super) fn page_margin_box_center_fills_band(
    boxes: &[crate::parser::css::MarginBox],
    box_index: usize,
    page: &Page,
    page_num: usize,
) -> bool {
    use crate::parser::css::MarginBoxPosition;

    let margin_box = &boxes[box_index];
    if margin_box.width.is_some() {
        return false;
    }
    let peers = match margin_box.position {
        MarginBoxPosition::TopCenter => [MarginBoxPosition::TopLeft, MarginBoxPosition::TopRight],
        MarginBoxPosition::BottomCenter => [
            MarginBoxPosition::BottomLeft,
            MarginBoxPosition::BottomRight,
        ],
        _ => return false,
    };
    !boxes.iter().enumerate().any(|(other_index, other)| {
        peers.contains(&other.position) && page_margin_box_wins(boxes, other_index, page, page_num)
    })
}

/// The physical page frame used to position page-margin box decorations.
#[derive(Debug, Clone, Copy)]
pub(super) struct PageMarginBoxFrame {
    page_size: PageSize,
    margin: Margin,
}

/// The inline geometry of one page-margin box and its generated text.
#[derive(Debug, Clone, Copy)]
pub(super) struct PageMarginBoxLayout {
    pub(super) box_x: f32,
    pub(super) box_width: f32,
    pub(super) text_x: f32,
}

impl PageMarginBoxFrame {
    pub(super) const fn new(page_size: PageSize, margin: Margin) -> Self {
        Self { page_size, margin }
    }

    /// Resolve the inline extent of a page-margin box.
    pub(super) fn layout(
        self,
        position: crate::parser::css::MarginBoxPosition,
        declared_width: Option<f32>,
        text_width: f32,
        center_fills_band: bool,
    ) -> PageMarginBoxLayout {
        use crate::parser::css::{MarginBoxAlign, MarginBoxPosition};

        let box_width = declared_width.unwrap_or(match position {
            MarginBoxPosition::TopLeftCorner | MarginBoxPosition::BottomLeftCorner => {
                self.margin.left
            }
            MarginBoxPosition::TopRightCorner | MarginBoxPosition::BottomRightCorner => {
                self.margin.right
            }
            MarginBoxPosition::LeftTop
            | MarginBoxPosition::LeftMiddle
            | MarginBoxPosition::LeftBottom => self.margin.left,
            MarginBoxPosition::RightTop
            | MarginBoxPosition::RightMiddle
            | MarginBoxPosition::RightBottom => self.margin.right,
            MarginBoxPosition::TopCenter | MarginBoxPosition::BottomCenter if center_fills_band => {
                self.page_size.width
            }
            _ => text_width,
        });
        let box_x = match position {
            MarginBoxPosition::TopLeftCorner | MarginBoxPosition::BottomLeftCorner => 0.0,
            MarginBoxPosition::TopLeft | MarginBoxPosition::BottomLeft => self.margin.left,
            MarginBoxPosition::TopCenter | MarginBoxPosition::BottomCenter => {
                (self.page_size.width - box_width) / 2.0
            }
            MarginBoxPosition::TopRight | MarginBoxPosition::BottomRight => {
                self.page_size.width - self.margin.right - box_width
            }
            MarginBoxPosition::TopRightCorner | MarginBoxPosition::BottomRightCorner => {
                self.page_size.width - self.margin.right
            }
            MarginBoxPosition::LeftTop
            | MarginBoxPosition::LeftMiddle
            | MarginBoxPosition::LeftBottom => 0.0,
            MarginBoxPosition::RightTop
            | MarginBoxPosition::RightMiddle
            | MarginBoxPosition::RightBottom => self.page_size.width - self.margin.right,
        };
        let text_x = match position.align() {
            MarginBoxAlign::Left => box_x,
            MarginBoxAlign::Center => box_x + (box_width - text_width) / 2.0,
            MarginBoxAlign::Right => box_x + box_width - text_width,
        };
        PageMarginBoxLayout {
            box_x,
            box_width,
            text_x,
        }
    }

    /// The physical background rectangle for a generated page-margin box.
    pub(super) fn background_rect(
        self,
        position: crate::parser::css::MarginBoxPosition,
        layout: PageMarginBoxLayout,
    ) -> PdfRect {
        use crate::parser::css::MarginBoxPosition;

        match position {
            MarginBoxPosition::TopLeftCorner
            | MarginBoxPosition::TopLeft
            | MarginBoxPosition::TopCenter
            | MarginBoxPosition::TopRight
            | MarginBoxPosition::TopRightCorner => PdfRect::new(
                layout.box_x,
                self.page_size.height - self.margin.top,
                layout.box_width,
                self.margin.top,
            ),
            MarginBoxPosition::BottomLeftCorner
            | MarginBoxPosition::BottomLeft
            | MarginBoxPosition::BottomCenter
            | MarginBoxPosition::BottomRight
            | MarginBoxPosition::BottomRightCorner => {
                PdfRect::new(layout.box_x, 0.0, layout.box_width, self.margin.bottom)
            }
            MarginBoxPosition::LeftTop
            | MarginBoxPosition::LeftMiddle
            | MarginBoxPosition::LeftBottom => {
                PdfRect::new(layout.box_x, 0.0, layout.box_width, self.page_size.height)
            }
            MarginBoxPosition::RightTop
            | MarginBoxPosition::RightMiddle
            | MarginBoxPosition::RightBottom => {
                PdfRect::new(layout.box_x, 0.0, layout.box_width, self.page_size.height)
            }
        }
    }
}

/// The baseline and block extent of one page-margin line.
///
/// Page-margin boxes use the same containing-block strut as ordinary CSS line
/// boxes. Keeping this value together avoids duplicating a second, subtly
/// different font-metrics approximation in page decoration code.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PageMarginLineBox {
    pub(super) baseline_from_top: f32,
    pub(super) height: f32,
}

impl PageMarginLineBox {
    pub(super) fn from_runs(
        runs: &[crate::layout::engine::TextRun],
        custom_fonts: &dyn crate::font_registry::FontRegistry,
    ) -> Self {
        let mut box_metrics = Self::default();
        for run in runs {
            let line_height = run.font_size * run.line_height_factor.max(0.0);
            let strut = crate::layout::text::LineStrut::from_exact_font(
                run.css_font_family(),
                run.font_size,
                run.bold,
                run.font_style.is_slanted(),
                line_height,
                custom_fonts,
            );
            box_metrics.baseline_from_top = box_metrics.baseline_from_top.max(strut.above);
            box_metrics.height = box_metrics.height.max(strut.above + strut.below);
        }
        box_metrics
    }
}

/// Resolve a baseline in a top or bottom page-margin box.
///
/// With `height:auto`, CSS Paged Media resolves the margin box's fixed
/// dimension from the page margin. Its content uses the ordinary line strut;
/// the cell's middle alignment splits positive remaining space between its
/// block margins. That centering remains meaningful when the line is taller
/// than the page-margin box: a zero-height top margin centers the line on the
/// page edge, as required by the page-margin box's table-cell alignment.
pub(super) fn page_margin_text_baseline(
    band: crate::parser::css::MarginBoxBand,
    page_size: PageSize,
    margin: Margin,
    line_box: PageMarginLineBox,
) -> f32 {
    let (margin_extent, from_page_top) = match band {
        crate::parser::css::MarginBoxBand::Top => (margin.top, true),
        crate::parser::css::MarginBoxBand::Bottom => (margin.bottom, false),
    };
    let surplus = (margin_extent - line_box.height) / 2.0;
    let baseline_from_band_top = line_box.baseline_from_top + surplus;

    if from_page_top {
        page_size.height - baseline_from_band_top
    } else {
        margin_extent - baseline_from_band_top
    }
}

#[cfg(test)]
mod page_margin_text_tests {
    use super::*;

    #[test]
    fn overflowing_line_remains_centered_in_the_margin_box() {
        let margin = Margin::new(15.0, 0.0, 0.0, 0.0);
        let baseline = page_margin_text_baseline(
            crate::parser::css::MarginBoxBand::Top,
            PageSize::new(120.0, 78.0),
            margin,
            PageMarginLineBox {
                baseline_from_top: 12.75,
                height: 18.0,
            },
        );
        let expected = 78.0 - 11.25;
        assert!((baseline - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn fitting_line_centers_its_auto_height_box_without_rounding() {
        let margin = Margin::new(21.0, 0.0, 0.0, 0.0);
        let baseline = page_margin_text_baseline(
            crate::parser::css::MarginBoxBand::Top,
            PageSize::new(150.0, 108.0),
            margin,
            PageMarginLineBox {
                baseline_from_top: 15.0,
                height: 20.25,
            },
        );
        let expected = 92.625;
        assert!((baseline - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn line_strut_preserves_fractional_page_margin_font_metrics() {
        let run = crate::layout::engine::TextRun {
            font_size: 12.0,
            line_height_factor: 1.5,
            ..Default::default()
        };
        let line_box = PageMarginLineBox::from_runs(std::slice::from_ref(&run), &HashMap::new());
        let line_height = run.font_size * run.line_height_factor;
        let metrics = crate::fonts::exact_font_line_metrics(
            run.css_font_family(),
            run.font_size,
            run.bold,
            run.font_style.is_slanted(),
            &HashMap::new(),
        );
        let expected = metrics.ascent + (line_height - metrics.ascent - metrics.descent) / 2.0;
        assert!((line_box.baseline_from_top - expected).abs() < f32::EPSILON);
        assert!((line_box.height - 18.0).abs() < f32::EPSILON);
    }

    #[test]
    fn margin_box_font_usage_includes_the_resolved_page_counter_glyphs() {
        let rules = crate::parser::css::parse_page_rules(
            "@page { counter-increment: page 2; @top-center { content: counter(page) } }",
        );
        let margin_box = &rules[0].margin_boxes[0];

        assert_eq!(
            margin_box_font_usage_text(margin_box, &Page::default(), 3, 3),
            "6"
        );
    }

    #[test]
    fn synthetic_weight_running_element_uses_its_registered_font_resource() {
        const FAMILY: &str = "IronpressSyntheticWeightFixture";
        let font = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans fixture font");
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .add_font(FAMILY, font)
            .convert(
                r#"<html><head><style>
                    @page { size: 180px 120px; margin: 22px 0 0;
                        @top-center { content: element(header) } }
                    html { font-family: IronpressSyntheticWeightFixture }
                    h1 { position: running(header); font-size: 12px }
                    div { height: 100px }
                </style></head><body><h1>RUNNING HEAD</h1><div></div></body></html>"#,
            )
            .expect("running-element PDF");
        let syntax = String::from_utf8_lossy(&pdf);

        assert!(
            syntax
                .matches("/ironpresssyntheticweightfixture__synthetic_weight")
                .count()
                >= 2,
            "the synthetic font must appear in both page resources and text operators"
        );
        assert!(
            !syntax.contains("/ironpresssyntheticweightfixture "),
            "running text must not reference the unregistered source resource"
        );
    }

    #[test]
    fn page_margin_background_frame_groups_band_and_side_geometry() {
        use crate::parser::css::MarginBoxPosition;

        let frame = PageMarginBoxFrame::new(
            PageSize::new(150.0, 108.0),
            Margin::new(15.0, 18.0, 21.0, 24.0),
        );

        let top_center = frame.layout(MarginBoxPosition::TopCenter, Some(150.0), 0.0, false);
        assert_eq!(
            frame.background_rect(MarginBoxPosition::TopCenter, top_center),
            PdfRect::new(0.0, 93.0, 150.0, 15.0)
        );
        let auto_center = frame.layout(MarginBoxPosition::TopCenter, None, 12.0, true);
        assert_eq!(auto_center.box_width, 150.0);
        assert_eq!(auto_center.text_x, 69.0);
        let bottom_right = frame.layout(MarginBoxPosition::BottomRight, None, 9.0, false);
        assert_eq!(
            frame.background_rect(MarginBoxPosition::BottomRight, bottom_right),
            PdfRect::new(123.0, 0.0, 9.0, 21.0)
        );
        let left_middle = frame.layout(MarginBoxPosition::LeftMiddle, None, 0.0, false);
        assert_eq!(
            frame.background_rect(MarginBoxPosition::LeftMiddle, left_middle),
            PdfRect::new(0.0, 0.0, 24.0, 108.0)
        );
    }

    #[test]
    fn auto_width_running_element_uses_its_content_width() {
        let element = TextBlock::plain(vec![TextLine {
            runs: vec![TextRun {
                text: "RUN HEAD".into(),
                font_size: 12.0,
                ..Default::default()
            }],
            height: 14.4,
            ..Default::default()
        }]);
        let available_width = 180.0;

        let width = RunningElementInlineSize {
            available_width,
            custom_fonts: &HashMap::new(),
        }
        .resolve(&element);

        assert!(width > 0.0);
        assert!(width < available_width);
    }
}

pub(super) fn page_selector_specificity(
    selector: &crate::parser::css::PageSelector,
) -> (u32, u32, u32) {
    selector.specificity()
}

struct RunningElementInlineSize<'a> {
    available_width: f32,
    custom_fonts: &'a dyn crate::font_registry::FontRegistry,
}

impl RunningElementInlineSize<'_> {
    fn resolve(&self, element: &dyn LayoutElement) -> f32 {
        self.preferred_width(element)
            .unwrap_or(self.available_width)
            .max(0.0)
    }

    fn preferred_width(&self, element: &dyn LayoutElement) -> Option<f32> {
        self.fixed_width(element)
            .or_else(|| self.replaced_width(element))
            .or_else(|| self.text_width(element))
            .or_else(|| {
                element
                    .inline_flow_extent()
                    .and_then(|extent| extent.max_content_outer_extent())
            })
            .or_else(|| self.children_width(element))
    }

    fn fixed_width(&self, element: &dyn LayoutElement) -> Option<f32> {
        element
            .box_fragmentation_owner()?
            .fragmentation_box_model()
            .size
            .width
            .fixed_value()
    }

    fn replaced_width(&self, element: &dyn LayoutElement) -> Option<f32> {
        element
            .replaced_element()
            .map(|replaced| replaced.geometry().size.width)
    }

    fn text_width(&self, element: &dyn LayoutElement) -> Option<f32> {
        struct TextWidth<'a> {
            custom_fonts: &'a dyn crate::font_registry::FontRegistry,
            width: Option<f32>,
        }

        impl LayoutVisitor for TextWidth<'_> {
            fn visit_text_block(&mut self, block: &TextBlock) {
                let content_width = block
                    .lines
                    .iter()
                    .map(|line| estimate_line_width_with_fonts(line, self.custom_fonts))
                    .fold(0.0f32, f32::max);
                self.width = Some(
                    content_width
                        + block.box_model.padding.horizontal()
                        + block.box_model.border.horizontal_width(),
                );
            }
        }

        let mut measurement = TextWidth {
            custom_fonts: self.custom_fonts,
            width: None,
        };
        element.accept(&mut measurement);
        measurement.width
    }

    fn children_width(&self, element: &dyn LayoutElement) -> Option<f32> {
        let mut width: Option<f32> = None;
        element.visit_children(&mut |child| {
            if let Some(child_width) = self.preferred_width(child) {
                width = Some(width.map_or(child_width, |current| current.max(child_width)));
            }
        });
        let content_width = width?;
        let edges = element
            .box_fragmentation_owner()
            .map(|owner| {
                let box_model = owner.fragmentation_box_model();
                box_model.padding.horizontal() + box_model.border.horizontal_width()
            })
            .unwrap_or_default();
        Some(content_width + edges)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_running_margin_element(
    content: &mut String,
    element: &dyn LayoutElement,
    align: crate::parser::css::MarginBoxAlign,
    band: crate::parser::css::MarginBoxBand,
    page_size: PageSize,
    margin: Margin,
    margin_box_background: Option<crate::types::Color>,
    page_index: usize,
    ctx: &mut PageRenderContext<'_>,
) -> bool {
    struct Renderer<'call, 'page> {
        content: &'call mut String,
        align: crate::parser::css::MarginBoxAlign,
        band: crate::parser::css::MarginBoxBand,
        page_size: PageSize,
        margin: Margin,
        margin_box_background: Option<crate::types::Color>,
        page_index: usize,
        ctx: &'call mut PageRenderContext<'page>,
        rendered: bool,
    }

    impl Renderer<'_, '_> {
        fn element_width(&self, element: &dyn LayoutElement) -> f32 {
            RunningElementInlineSize {
                available_width: (self.page_size.width - self.margin.horizontal()).max(0.0),
                custom_fonts: self.ctx.text.custom_fonts,
            }
            .resolve(element)
        }

        fn render_layout_element(&mut self, element: &dyn LayoutElement) {
            if let Some(background) = self.margin_box_background.take() {
                let (r, g, b, alpha) = background.to_f32_rgba();
                if alpha > 0.0 {
                    let (y, height) = match self.band {
                        crate::parser::css::MarginBoxBand::Top => {
                            (self.page_size.height - self.margin.top, self.margin.top)
                        }
                        crate::parser::css::MarginBoxBand::Bottom => (0.0, self.margin.bottom),
                    };
                    self.content.push_str(&format!("{r} {g} {b} rg\n"));
                    self.content
                        .push_str(&format!("0 {y} {} {height} re f\n", self.page_size.width));
                }
            }
            let width = self.element_width(element);
            let height = crate::layout::paginate::estimate_element_height(element).max(0.0);
            let x = match self.align {
                crate::parser::css::MarginBoxAlign::Left => 0.0,
                crate::parser::css::MarginBoxAlign::Center => {
                    self.page_size.width / 2.0 - width / 2.0
                }
                crate::parser::css::MarginBoxAlign::Right => {
                    self.page_size.width - self.margin.right - width
                }
            };
            let band_center_y = match self.band {
                crate::parser::css::MarginBoxBand::Top => {
                    self.page_size.height - self.margin.top / 2.0
                }
                crate::parser::css::MarginBoxBand::Bottom => self.margin.bottom / 2.0,
            };
            let top_inset = self.page_size.height - band_center_y - height / 2.0;
            let frame = PageElementFrame {
                occlusion_coverers: &[],
                page_size: self.page_size,
                margin: Margin::new(top_inset, 0.0, 0.0, x),
                available_width: width,
                y_pos: 0.0,
                element_index: 0,
                page_index: self.page_index,
            };
            let marker = self.ctx.stacking.marker();
            let mut element_content = String::new();
            let mut discarded_bookmarks = Vec::new();
            element.accept(&mut PageElementRenderer {
                content: &mut element_content,
                frame,
                paint_phase: ElementPaintPhase::All,
                bookmarks: &mut discarded_bookmarks,
                ctx: self.ctx,
            });
            let descendants = self.ctx.stacking.take_since(marker);
            let mut plan = StackingPaintPlan::default();
            self.ctx.stacking.commit(
                StackingScope::Local,
                self.content,
                &mut plan,
                layout_element_paint_order(element).with_in_flow_phase(true, true),
                element_content,
                descendants,
            );
            self.ctx.stacking.paint_plan(plan, self.content);
            self.rendered = true;
        }
    }

    impl LayoutVisitor for Renderer<'_, '_> {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.render_layout_element(element);
        }

        fn visit_table_row(&mut self, element: &TableRow) {
            self.render_layout_element(element);
        }

        fn visit_grid_row(&mut self, element: &GridRow) {
            self.render_layout_element(element);
        }

        fn visit_flex_row(&mut self, element: &FlexRow) {
            self.render_layout_element(element);
        }

        fn visit_container(&mut self, element: &Container) {
            self.render_layout_element(element);
        }

        fn visit_image(&mut self, element: &Image) {
            self.render_layout_element(element);
        }

        fn visit_svg(&mut self, element: &Svg) {
            self.render_layout_element(element);
        }

        fn visit_horizontal_rule(&mut self, element: &HorizontalRule) {
            self.render_layout_element(element);
        }

        fn visit_progress_bar(&mut self, element: &ProgressBar) {
            self.render_layout_element(element);
        }

        fn visit_math_block(&mut self, element: &MathBlock) {
            self.render_layout_element(element);
        }

        fn visit_column_rule(&mut self, element: &ColumnRule) {
            self.render_layout_element(element);
        }
    }

    let mut renderer = Renderer {
        content,
        align,
        band,
        page_size,
        margin,
        margin_box_background,
        page_index,
        ctx,
        rendered: false,
    };
    element.accept(&mut renderer);
    renderer.rendered
}

pub(super) fn wrapped_footnote_lines(
    footnotes: &[FootnoteItem],
    available_width: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<TextLine> {
    let available_width = available_width.max(0.0);
    let mut lines = Vec::new();
    let mut compact_runs = Vec::new();
    let flush_compact = |runs: &mut Vec<TextRun>, lines: &mut Vec<TextLine>| {
        if runs.is_empty() {
            return;
        }
        let font_size = runs.first().map(|run| run.font_size).unwrap_or(12.0);
        let line_height = runs
            .first()
            .map(|run| run.line_height_factor)
            .filter(|factor| factor.is_finite())
            .unwrap_or(1.2);
        lines.extend(wrap_text_runs(
            std::mem::take(runs),
            TextWrapOptions::new(
                available_width,
                font_size,
                line_height,
                OverflowWrap::Normal,
            ),
            custom_fonts,
        ));
    };

    for footnote in footnotes {
        let runs = footnote.text_runs();
        if footnote.formatting.display.is_inline_layout() {
            compact_runs.extend(runs);
            continue;
        }
        flush_compact(&mut compact_runs, &mut lines);
        lines.extend(wrap_text_runs(
            runs,
            TextWrapOptions::new(
                available_width,
                footnote.body.font_size,
                if footnote.body.line_height_factor.is_finite() {
                    footnote.body.line_height_factor
                } else {
                    1.2
                },
                OverflowWrap::Normal,
            ),
            custom_fonts,
        ));
    }
    flush_compact(&mut compact_runs, &mut lines);
    lines
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_page_footnotes(
    content: &mut String,
    footnotes: &[FootnoteItem],
    page_size: PageSize,
    margin: Margin,
    area: ResolvedFootnoteAreaStyle,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    if footnotes.is_empty() {
        return;
    }
    let footnote_box_width = (page_size.width - margin.horizontal()).max(0.0);
    let available_width = (footnote_box_width - area.padding.horizontal()).max(0.0);
    let lines = wrapped_footnote_lines(footnotes, available_width, custom_fonts);

    let total_h: f32 = lines.iter().map(|line| line.height).sum();
    let separator_width = area.separator.width.max(0.0);
    if separator_width > 0.0 && footnote_box_width > 0.0 {
        let separator_y = margin.bottom + area.padding.bottom + total_h + area.padding.top;
        let (r, g, b, alpha) = area.separator.color.to_f32_rgba();
        let applied_alpha = begin_border_alpha(content, page_ext_gstates, bg_alpha_counter, alpha);
        content.push_str(&format!(
            "{} {} {} rg\n{} {} {} {} re\nf\n",
            format_pdf_number(r),
            format_pdf_number(g),
            format_pdf_number(b),
            format_pdf_number(margin.left),
            format_pdf_number(separator_y),
            format_pdf_number(footnote_box_width),
            format_pdf_number(separator_width),
        ));
        end_border_alpha(content, applied_alpha);
    }

    let mut baseline_cursor = TextBaselineCursor::new(
        margin.bottom + area.padding.bottom + total_h,
        pdf_writer.page_content_transform,
    );
    for line in &lines {
        // `wrapped_footnote_lines` established this fresh inline formatting
        // context at the page foot. Reuse its resolved baseline instead of
        // resolving the same font metrics a second time during PDF paint.
        let metrics = line_box_metrics(line, custom_fonts);
        let baseline_y = baseline_cursor.next_horizontal(metrics);
        let merged = crate::text::coalesce_text_runs(&line.runs);
        paint_horizontal_line_text(
            content,
            &merged,
            HorizontalLinePaint {
                origin: PdfPoint::new(margin.left + area.padding.left, baseline_y),
                line_ascender: metrics.ascender,
                justification_word_spacing: 0.0,
                text_space: PdfContentSpace::page_css(pdf_writer.page_content_transform),
            },
            custom_fonts,
            prepared_custom_fonts,
            pdf_writer,
            page_images,
        );
    }
}

/// Render laid-out pages as PDF, writing directly to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`render_pdf`]. It writes PDF content incrementally
/// to the provided writer instead of building an in-memory buffer.
#[allow(dead_code)]
pub fn render_pdf_to_writer<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_with_fonts(pages, page_size, margin, writer, &HashMap::new())
}

/// Render laid-out pages as PDF with custom fonts, writing directly to any `std::io::Write` implementation.
pub(super) fn render_pdf_to_writer_with_fonts<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full(pages, page_size, margin, writer, custom_fonts, None)
}

#[derive(Default)]
pub(super) struct MarginBoxFontUsage {
    pub(super) per_page_runs: Vec<Vec<TextRun>>,
}

pub(super) fn margin_box_font_usage(
    pages: &[Page],
    decoration: Option<&PageDecoration>,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> MarginBoxFontUsage {
    let Some(decoration) = decoration else {
        return MarginBoxFontUsage::default();
    };

    let mut usage = MarginBoxFontUsage {
        per_page_runs: (0..pages.len()).map(|_| Vec::new()).collect(),
    };

    for (page_index, (page, runs)) in pages.iter().zip(&mut usage.per_page_runs).enumerate() {
        let page_number = page_index + 1;
        for (margin_box_index, margin_box) in decoration.margin_boxes.iter().enumerate() {
            if !page_margin_box_wins(
                &decoration.margin_boxes,
                margin_box_index,
                page,
                page_number,
            ) {
                continue;
            }
            let style = decoration.margin_text.resolve(
                crate::parser::css::PageSelectorContext {
                    page_number,
                    is_blank: page.is_blank,
                    page_name: page.page_name.as_deref(),
                },
                &margin_box.text_style,
                custom_fonts,
            );
            let text = margin_box_font_usage_text(margin_box, page, page_number, pages.len());
            if !text.is_empty() {
                runs.push(margin_box_font_usage_run(text, style.font_family));
            }
        }
    }

    usage
}

fn margin_box_font_usage_text(
    margin_box: &crate::parser::css::MarginBox,
    page: &Page,
    page_number: usize,
    page_count: usize,
) -> String {
    use crate::parser::css::MarginContentToken;
    let mut text = String::new();
    for token in &margin_box.content {
        match token {
            MarginContentToken::Literal(value) => text.push_str(value),
            MarginContentToken::PageNumber => {
                text.push_str(&page_counter_value(margin_box, page_number).to_string())
            }
            MarginContentToken::PageCount => text.push_str(&page_count.to_string()),
            MarginContentToken::NamedString(reference) => {
                let value = page.generated_content.named_string(reference);
                if let Some(value) = value {
                    text.push_str(value);
                }
            }
            MarginContentToken::Element(_) => {}
        }
    }
    text
}

pub(super) fn margin_box_font_usage_run(text: String, font_family: FontFamily) -> TextRun {
    TextRun {
        text,
        font_family,
        line_height_factor: 1.2,
        ..Default::default()
    }
}
