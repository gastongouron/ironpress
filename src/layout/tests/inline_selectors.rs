use crate::layout::elements::{LayoutVisitor, TextBlock};
use crate::layout::engine::visit_layout_tree;
use crate::style::computed::{Position, VerticalAlign};
use crate::types::Color;

use super::support::{layout_pages, visible_inline_box_runs, visible_runs};

fn positioned_text(markup: &str) -> Vec<(String, crate::layout::elements::Positioning)> {
    #[derive(Default)]
    struct Collector(Vec<(String, crate::layout::elements::Positioning)>);

    impl LayoutVisitor for Collector {
        fn visit_text_block(&mut self, block: &TextBlock) {
            let text = block
                .lines
                .iter()
                .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
                .collect::<String>();
            if !text.is_empty() {
                self.0.push((text, block.positioning.clone()));
            }
        }
    }

    let mut collector = Collector::default();
    for page in layout_pages(markup) {
        for (_, element) in page.elements {
            visit_layout_tree(element.as_ref(), &mut collector);
        }
    }
    collector.0
}

#[test]
fn nested_inline_runs_keep_their_real_sibling_selector_positions() {
    let runs = visible_runs(
        r#"<style>
            .node { font-size: 16px; }
            .node > .own > .token:first-of-type {
                font-size: .72em;
                vertical-align: super;
            }
            .node > .own > .token:nth-of-type(2) { color: #c1121f; }
        </style>
        <div class="node"><div class="own"><span class="token">Ag</span><span class="token">Bb</span></div></div>"#,
    );

    assert_eq!(runs.len(), 2, "runs: {runs:#?}");
    assert_eq!(runs[0].text, "Ag");
    assert_eq!(runs[0].vertical_align, VerticalAlign::Super);
    assert!(runs[0].font_size < runs[1].font_size, "runs: {runs:#?}");
    assert_eq!(runs[1].text, "Bb");
    assert_eq!(runs[1].vertical_align, VerticalAlign::Baseline);
    assert_eq!(runs[1].color, Color::rgb(193, 18, 31));
}

#[test]
fn anonymous_table_fixup_keeps_authored_ancestor_and_type_position() {
    let runs = visible_runs(
        r#"<style>
            .node { display: table; }
            .node > .own > .token { display: table-cell; }
            .node > .own > .token:nth-of-type(2) { color: #c1121f; }
        </style>
        <div class="node"><div class="own"><span class="token">Ag</span><span class="token">Bb</span></div></div>"#,
    );

    assert_eq!(runs.len(), 2, "runs: {runs:#?}");
    assert_eq!(runs[0].text, "Ag");
    assert_eq!(runs[1].text, "Bb");
    assert_eq!(runs[1].color, Color::rgb(193, 18, 31));
}

#[test]
fn table_cell_mixed_flow_uses_the_shared_complete_sibling_model() {
    let runs = visible_runs(
        r#"<style>
            * { margin: 0; padding: 0; }
            table { border-collapse: collapse; }
            .host > .token:last-of-type { color: #c1121f; }
        </style>
        <table><tr><td class="host">
            <span class="token">FIRST</span>
            <div>BLOCK</div>
            <span class="token">LAST</span>
        </td></tr></table>"#,
    );
    let first = runs
        .iter()
        .find(|run| run.text.contains("FIRST"))
        .expect("first table-cell inline run");
    let last = runs
        .iter()
        .find(|run| run.text.contains("LAST"))
        .expect("last table-cell inline run");

    assert_ne!(first.color, Color::rgb(193, 18, 31), "runs: {runs:#?}");
    assert_eq!(last.color, Color::rgb(193, 18, 31), "runs: {runs:#?}");
}

#[test]
fn inline_block_mixed_flow_uses_the_shared_complete_sibling_model() {
    let runs = visible_inline_box_runs(
        r#"<style>
            * { margin: 0; padding: 0; }
            .host { display: inline-block; }
            .host > .break { display: block; }
            .host > .token:last-of-type { color: #c1121f; }
        </style>
        <div><span class="host">
            <span class="token">FIRST</span>
            <span class="break">BLOCK</span>
            <span class="token">LAST</span>
        </span></div>"#,
    );
    let first = runs
        .iter()
        .find(|run| run.text.contains("FIRST"))
        .unwrap_or_else(|| panic!("missing first inline-block run: {runs:#?}"));
    let last = runs
        .iter()
        .find(|run| run.text.contains("LAST"))
        .unwrap_or_else(|| panic!("missing last inline-block run: {runs:#?}"));

    assert_ne!(first.color, Color::rgb(193, 18, 31), "runs: {runs:#?}");
    assert_eq!(last.color, Color::rgb(193, 18, 31), "runs: {runs:#?}");
}

#[test]
fn positioned_inline_is_out_of_flow_and_resolves_against_its_ancestor() {
    let snapshots = positioned_text(
        r#"<style>
            .cb { position: relative; width: 126px; height: 68px; }
            .own { height: 22px; }
            .token:last-of-type { position: absolute; right: 4px; bottom: 4px; }
        </style>
        <div class="cb"><div class="own"><span class="token">Ag</span><span class="token">Bb</span></div></div>"#,
    );
    assert_eq!(snapshots.len(), 2, "snapshots: {snapshots:#?}");

    let (text, in_flow) = &snapshots[0];
    assert_eq!(text, "Ag");
    assert_eq!(in_flow.scheme, Position::Static);

    let (text, positioned) = &snapshots[1];
    assert_eq!(text, "Bb");
    assert_eq!(positioned.scheme, Position::Absolute);
    let containing_block = positioned
        .containing_block
        .expect("absolute inline containing block");
    assert!((containing_block.width - 94.5).abs() < 0.001);
    assert!((containing_block.height - 51.0).abs() < 0.001);
    assert!((positioned.insets.left - 77.496).abs() < 0.001);
    assert!((positioned.insets.top - 33.0).abs() < 0.001);
}

#[test]
fn nested_absolute_bottom_uses_final_min_height_containing_block() {
    let snapshots = positioned_text(
        r#"<style>
            * { box-sizing: border-box; margin: 0; }
            .cb {
                position: relative;
                width: 152px;
                min-height: 96px;
                padding: 7px;
                border: 2px solid;
            }
            .own { height: 22px; }
            .inner { height: 48px; }
            .token:last-of-type {
                position: absolute;
                right: 4px;
                bottom: 4px;
            }
        </style>
        <div class="cb">
            <div class="own"><span class="token">Ag</span><span class="token">Bb</span></div>
            <div class="inner"></div>
        </div>"#,
    );
    let (_, positioned) = snapshots
        .iter()
        .find(|(text, _)| text == "Bb")
        .expect("nested absolute text");
    let containing_block = positioned.containing_block.expect("final containing block");

    // CSS Positioned Layout: the containing block is the positioned
    // ancestor's used padding box. 96px border-box minus two 2px borders.
    assert!((containing_block.height - 69.0).abs() < 0.001);
    assert!(positioned.insets.top > 50.0, "positioning: {positioned:#?}");
}

#[test]
fn positioned_inline_inside_flex_item_uses_the_general_item_context() {
    let snapshots = positioned_text(
        r#"<style>
            .cb {
                display: flex;
                position: relative;
                width: 126px;
                height: 68px;
                align-items: center;
                justify-content: center;
            }
            .own { height: 22px; }
            .cb > .own > .token:last-of-type {
                position: absolute;
                right: 4px;
                bottom: 4px;
            }
        </style>
        <div class="cb"><div class="own"><span class="token">Ag</span><span class="token">Bb</span></div></div>"#,
    );

    let in_flow = snapshots
        .iter()
        .find(|(text, _)| text == "Ag")
        .expect("in-flow flex item text");
    let positioned = snapshots
        .iter()
        .find(|(text, _)| text == "Bb")
        .expect("positioned flex descendant");
    assert_eq!(in_flow.1.scheme, Position::Static);
    assert_eq!(positioned.1.scheme, Position::Absolute);
    assert!(
        positioned.1.insets.left > in_flow.1.insets.left + 40.0,
        "snapshots: {snapshots:#?}"
    );
    assert!(
        positioned.1.insets.top > in_flow.1.insets.top + 20.0,
        "snapshots: {snapshots:#?}"
    );
}

#[test]
fn a_float_inside_an_inline_box_does_not_break_it() {
    // Only an in-flow block-level descendant breaks an inline box (CSS 2.1
    // § 9.2.1.1); a float is out of flow, so the line stays whole.
    #[derive(Default)]
    struct Lines(Vec<String>);

    impl LayoutVisitor for Lines {
        fn visit_text_block(&mut self, block: &TextBlock) {
            for line in &block.lines {
                let text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
                if !text.trim().is_empty() {
                    self.0.push(text);
                }
            }
        }
    }

    let mut lines = Lines::default();
    for page in layout_pages(
        r#"<style>* { margin: 0 }</style>
        <div>a <span>b<div style="float:left">F</div>d</span> e</div>"#,
    ) {
        for (_, element) in page.elements {
            visit_layout_tree(element.as_ref(), &mut lines);
        }
    }
    assert!(
        lines.0.iter().any(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            words.contains(&"a") && words.contains(&"e")
        }),
        "{:?}",
        lines.0
    );
}
