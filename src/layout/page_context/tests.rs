use super::*;
use crate::parser::css::parse_page_rules;
use std::collections::HashMap;

fn color_on_page(css: &str, page: PageSelectorContext<'_>) -> Option<crate::types::Color> {
    PageBackgroundContext::from_rules(&parse_page_rules(css), RasterQuality::default(), 0.0)
        .resolve(page, &HashMap::new())
        .and_then(|style| style.background_color)
}

fn geometry_on_page(css: &str, page: PageSelectorContext<'_>) -> PageGeometry {
    PageGeometryContext::from_rules(
        PageSize::new(200.0, 300.0),
        Margin::uniform(10.0),
        &parse_page_rules(css),
    )
    .resolve(page)
}

#[test]
fn page_geometry_combines_matching_selectors_by_specificity() {
    let geometry = geometry_on_page(
        "@page :right { size: 320pt 480pt; margin-left: 30pt }\
         @page :first { margin-top: 40pt }",
        PageSelectorContext {
            page_number: 1,
            is_blank: false,
            page_name: None,
        },
    );

    assert_eq!(geometry.size, PageSize::new(320.0, 480.0));
    assert_eq!(geometry.margin, Margin::new(40.0, 10.0, 10.0, 30.0));
    assert_eq!(geometry.content_height(), 430.0);
}

#[test]
fn page_geometry_uses_source_order_with_equal_specificity() {
    let geometry = geometry_on_page(
        "@page :left { margin-right: 20pt }\
         @page :left { margin-right: 35pt }",
        PageSelectorContext {
            page_number: 2,
            is_blank: false,
            page_name: None,
        },
    );

    assert_eq!(geometry.margin.right, 35.0);
}

#[test]
fn root_flow_insets_do_not_change_the_physical_page_area() {
    let geometry = PageGeometry::new(
        PageSize::new(200.0, 300.0),
        Margin::new(10.0, 20.0, 30.0, 40.0),
    )
    .with_root_flow_insets(Margin::new(2.0, 4.0, 6.0, 8.0));

    assert_eq!(geometry.flow_margin(), Margin::new(12.0, 24.0, 36.0, 48.0));
    assert_eq!(geometry.content_size(), Size::new(128.0, 252.0));
    assert_eq!(geometry.page_area_size(), Size::new(140.0, 260.0));
    assert_eq!(
        geometry.page_area_in_flow_space(),
        PageAreaInFlowSpace::new(Point::new(-8.0, -2.0), Size::new(140.0, 260.0))
    );
}

#[test]
fn page_geometry_supports_named_compounds_lists_and_case_sensitive_names() {
    let css = "@page Chapter:left, Appendix:right { margin-bottom: 45pt }";
    let chapter_left = geometry_on_page(
        css,
        PageSelectorContext {
            page_number: 2,
            is_blank: false,
            page_name: Some("Chapter"),
        },
    );
    let appendix_right = geometry_on_page(
        css,
        PageSelectorContext {
            page_number: 3,
            is_blank: false,
            page_name: Some("Appendix"),
        },
    );
    let wrong_case = geometry_on_page(
        css,
        PageSelectorContext {
            page_number: 2,
            is_blank: false,
            page_name: Some("chapter"),
        },
    );

    assert_eq!(chapter_left.margin.bottom, 45.0);
    assert_eq!(appendix_right.margin.bottom, 45.0);
    assert_eq!(wrong_case.margin.bottom, 10.0);
}

#[test]
fn blank_specificity_overrides_spread_regardless_of_source_order() {
    let geometry = geometry_on_page(
        "@page :blank { margin-left: 50pt }\
         @page :left { margin-left: 25pt }",
        PageSelectorContext {
            page_number: 2,
            is_blank: true,
            page_name: None,
        },
    );

    assert_eq!(geometry.margin.left, 50.0);
}

#[test]
fn page_background_resolves_each_physical_page_selector() {
    let css = "@page { background: red }\
               @page :left { background: blue }\
               @page :first { background: green }";
    assert_eq!(
        color_on_page(
            css,
            PageSelectorContext {
                page_number: 1,
                is_blank: false,
                page_name: None,
            }
        ),
        Some(crate::types::Color::rgb(0, 128, 0))
    );
    assert_eq!(
        color_on_page(
            css,
            PageSelectorContext {
                page_number: 2,
                is_blank: false,
                page_name: None,
            }
        ),
        Some(crate::types::Color::rgb(0, 0, 255))
    );
    assert_eq!(
        color_on_page(
            css,
            PageSelectorContext {
                page_number: 3,
                is_blank: false,
                page_name: None,
            }
        ),
        Some(crate::types::Color::rgb(255, 0, 0))
    );
}

#[test]
fn compound_named_selector_requires_every_component() {
    let css = "@page { background: red }\
               @page Chapter:left { background: blue }";
    let named_left = PageSelectorContext {
        page_number: 2,
        is_blank: false,
        page_name: Some("Chapter"),
    };
    let wrong_case = PageSelectorContext {
        page_name: Some("chapter"),
        ..named_left
    };
    assert_eq!(
        color_on_page(css, named_left),
        Some(crate::types::Color::rgb(0, 0, 255))
    );
    assert_eq!(
        color_on_page(css, wrong_case),
        Some(crate::types::Color::rgb(255, 0, 0))
    );
}
