use super::support::layout_pages_at;
use crate::layout::elements::{Container, LayoutElementTestExt, LayoutVisitor, TextBlock};
use crate::layout::engine::{
    DocumentGeometry, layout_with_rules_and_fonts_raster_quality, visit_layout_tree,
};
use crate::parser::css::parse_stylesheet;
use crate::parser::html::parse_html_with_styles;
use crate::style::raster_quality::RasterQuality;
use crate::types::{Color, Margin, PageSize, Size};
use std::collections::HashMap;

fn page() -> PageSize {
    PageSize::new(240.0, 120.0)
}

fn root_carrier(display: &str, tracks: &str) -> String {
    format!(
        r#"<style>
            * {{ box-sizing:border-box; margin:0; padding:0; }}
            body {{ display:{display}; {tracks} width:240pt; height:120pt; gap:6pt; padding:12pt; }}
            .item {{ width:60pt; height:60pt; }}
        </style>
        <div class="item"></div><div class="item"></div><div class="item"></div>"#,
    )
}

#[test]
fn root_flex_formatting_context_is_one_page_carrier() {
    let pages = layout_pages_at(&root_carrier("flex", ""), page());
    assert_eq!(pages.len(), 1);
}

#[test]
fn root_grid_formatting_context_is_one_page_carrier() {
    let pages = layout_pages_at(
        &root_carrier(
            "grid",
            "grid-template-columns:repeat(3,60pt); grid-template-rows:60pt;",
        ),
        page(),
    );
    assert_eq!(pages.len(), 1);
}

#[test]
fn fitted_print_content_uses_the_physical_page_edge_not_body_end_padding() {
    let pages = layout_pages_at(
        r#"<style>
            * { box-sizing:border-box; margin:0 }
            body { padding:16.5pt }
            .grid {
                display:grid;
                grid-template-columns:fit-content(30%) minmax(0,1fr);
                column-gap:30pt;
                width:435pt;
                height:69pt;
                margin-bottom:16.5pt;
                padding:13.5pt 22.5pt;
                border:6pt solid #111;
            }
            .content-box { box-sizing:content-box }
            .nested { width:390pt; padding:15pt; border:4.5pt solid #f90 }
            .nested .grid {
                width:100%;
                height:61.5pt;
                margin:0;
                padding:9pt 15pt;
                border-width:4.5pt;
            }
        </style>
        <div class="grid content-box"><div>label</div><div>value</div></div>
        <div class="grid"><div>label</div><div>value</div></div>
        <div class="nested"><div class="grid"><div>label</div><div>value</div></div></div>"#,
        PageSize::new(510.0, 360.0),
    );
    let flow_edges = pages[0]
        .elements
        .iter()
        .filter_map(|(_, element)| element.print_fit_right_edge())
        .collect::<Vec<_>>();

    assert!(
        pages[0].print_content_scale.is_identity(),
        "a border box ending at 508.5pt fits the 510pt page; flow edges: {flow_edges:?}"
    );
}

#[test]
fn projected_body_padding_is_not_subtracted_from_root_percentages_twice() {
    #[derive(Default)]
    struct GreenWidth(Option<f32>);

    impl LayoutVisitor for GreenWidth {
        fn visit_container(&mut self, container: &Container) {
            if container.paint.background.color == Some(Color::rgb(216, 243, 220)) {
                self.0 = container.box_model.size.width.fixed_value();
            }
        }
    }

    let markup = r#"
        <style>
            * { box-sizing:border-box; margin:0 }
            body { padding:0 6pt }
            .stage {
                width:calc(100% + 12pt);
                height:40pt;
                margin-left:-6pt;
                background:#d8f3dc;
            }
        </style>
        <div class="stage"></div>
    "#;
    let document = parse_html_with_styles(markup).expect("valid root percentage fixture");
    let rules = document
        .stylesheets
        .iter()
        .flat_map(|stylesheet| parse_stylesheet(stylesheet))
        .collect::<Vec<_>>();
    let page = PageSize::new(144.0, 150.0);
    let page_margin = Margin::new(24.0, 6.0, 12.0, 18.0);
    let root_insets = Margin::new(0.0, 6.0, 0.0, 6.0);
    let flow_margin = page_margin + root_insets;
    let mut resources = crate::security::resources::ResourceLoader::default();
    let pages = layout_with_rules_and_fonts_raster_quality(
        &document.nodes,
        DocumentGeometry::new(page, flow_margin)
            .with_initial_containing_block(Size::new(120.0, 114.0)),
        &rules,
        &HashMap::new(),
        document.head.font_locale,
        &crate::layout::page_context::PageBackgroundContext::uniform(
            None,
            0.0,
            RasterQuality::default(),
        ),
        crate::layout::paginate::PaginationContext::new(
            crate::layout::page_context::PageGeometryContext::uniform(page, page_margin)
                .with_root_flow_insets(root_insets),
            Default::default(),
            0.0,
        ),
        RasterQuality::default(),
        &mut resources,
    );
    let mut width = GreenWidth::default();
    for (_, element) in &pages[0].elements {
        visit_layout_tree(element.as_ref(), &mut width);
    }
    let flow_edges = pages[0]
        .elements
        .iter()
        .filter_map(|(_, element)| element.print_fit_right_edge())
        .collect::<Vec<_>>();

    assert_eq!(width.0, Some(120.0));
    assert!(
        pages[0].print_content_scale.is_identity(),
        "the negative root-child margin keeps this box inside the page area: {flow_edges:?}"
    );
    assert_eq!(
        pages[0].geometry.map(|geometry| geometry.flow_margin()),
        Some(flow_margin)
    );
}

#[test]
fn viewport_math_uses_page_area_before_projected_body_padding() {
    #[derive(Default)]
    struct BarWidth(Option<f32>);

    impl LayoutVisitor for BarWidth {
        fn visit_text_block(&mut self, block: &TextBlock) {
            if block.paint.background.color == Some(Color::rgb(17, 138, 178)) {
                self.0 = block.box_model.size.width.fixed_value();
            }
        }

        fn visit_container(&mut self, container: &Container) {
            if container.paint.background.color == Some(Color::rgb(17, 138, 178)) {
                self.0 = container.box_model.size.width.fixed_value();
            }
        }
    }

    let markup = r#"
        <style>
            * { box-sizing:border-box; margin:0 }
            body { padding:18px }
            .bar { width:calc(50vmin - 180px); height:20px; background:#118ab2 }
        </style>
        <div class="bar"></div>
    "#;
    let document = parse_html_with_styles(markup).expect("valid viewport fixture");
    let rules = document
        .stylesheets
        .iter()
        .flat_map(|stylesheet| parse_stylesheet(stylesheet))
        .collect::<Vec<_>>();
    let page = PageSize::new(360.0, 468.0);
    let projected_body_padding = Margin::uniform(13.5);
    let page_background = crate::layout::page_context::PageBackgroundContext::uniform(
        None,
        0.0,
        RasterQuality::default(),
    );
    let mut resources = crate::security::resources::ResourceLoader::default();
    let pages = layout_with_rules_and_fonts_raster_quality(
        &document.nodes,
        DocumentGeometry::new(page, projected_body_padding)
            .with_initial_containing_block(Size::new(360.0, 468.0)),
        &rules,
        &HashMap::new(),
        document.head.font_locale,
        &page_background,
        crate::layout::paginate::PaginationContext::new(
            crate::layout::page_context::PageGeometryContext::uniform(page, projected_body_padding),
            Default::default(),
            0.0,
        ),
        RasterQuality::default(),
        &mut resources,
    );
    let mut width = BarWidth::default();
    for (_, element) in &pages[0].elements {
        visit_layout_tree(element.as_ref(), &mut width);
    }

    // 50vmin = 180pt from the 360pt page area; 180px = 135pt.
    assert_eq!(width.0, Some(45.0));
}

#[test]
fn fixed_height_sibling_boxes_keep_their_collapsed_block_margin() {
    let pages = layout_pages_at(
        r#"<style>
            * { box-sizing:border-box; margin:0 }
            .lane { width:100px; height:54px; margin-bottom:10px; border:3px solid #111 }
        </style>
        <div class="lane"></div><div class="lane"></div>"#,
        PageSize::new(360.0, 468.0),
    );
    let lanes = pages[0]
        .elements
        .iter()
        .filter_map(|(y, element)| {
            element.inspect_container(|container| {
                (
                    *y,
                    container.box_model.margins,
                    container.box_model.size.height.used(),
                )
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0].1.end, 7.5);
    assert_eq!(lanes[1].0 - lanes[0].0, 48.0);
}
