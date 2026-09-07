use super::support::layout_pages_with_fonts;
use crate::layout::elements::{FlexRow, GridRow, LayoutVisitor};
use crate::layout::engine::visit_layout_tree;

#[derive(Default)]
struct FilteredGridCells(usize);

impl LayoutVisitor for FilteredGridCells {
    fn visit_grid_row(&mut self, row: &GridRow) {
        self.0 += row
            .content
            .cells
            .iter()
            .filter(|cell| cell.layout.paint.filter_output.is_some())
            .count();
    }
}

#[test]
fn direct_grid_item_owns_its_composited_filter_output() {
    let font = crate::parser::ttf::parse_ttf(
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font"),
    )
    .expect("valid ParitySans test font");
    let fonts = std::collections::HashMap::from([("paritysans".to_string(), font)]);
    let pages = layout_pages_with_fonts(
        r#"
        <style>
            .outer { display:grid; grid-template-columns:1fr 1fr; width:126px; height:96px; }
            .inner {
                width:58px; height:48px; padding:5px;
                border:2px solid #577590; background:#e7f5ff;
                font-family:ParitySans;
                filter:grayscale(.18) contrast(1.08) drop-shadow(2px 1px 0 #90a4ae);
            }
            .own { height:22px; white-space:nowrap; }
        </style>
        <div class="outer"><div>A</div><div class="inner"><div class="own"><span>A</span><span>B</span></div></div></div>
        "#,
        &fonts,
    );
    let mut filtered = FilteredGridCells::default();
    for page in pages {
        for (_, element) in page.elements {
            visit_layout_tree(element.as_ref(), &mut filtered);
        }
    }
    assert_eq!(
        filtered.0, 1,
        "the filtered grid item remains one paint group"
    );
}

#[derive(Default)]
struct FlexFilterOwnership(Vec<bool>);

impl LayoutVisitor for FlexFilterOwnership {
    fn visit_flex_row(&mut self, row: &FlexRow) {
        self.0.extend(
            row.content
                .cells
                .iter()
                .map(|cell| cell.paint.filter_output.is_some()),
        );
    }
}

#[test]
fn filtered_flex_item_keeps_document_order_ownership() {
    let fonts = std::collections::HashMap::<String, crate::parser::ttf::TtfFont>::new();
    let pages = layout_pages_with_fonts(
        r#"
        <style>
            .row { display:flex; width:260px; height:120px; }
            .item { width:70px; height:60px; }
            .first { background:#d32f2f; filter:blur(7px); }
            .second { background:#2e7d32; }
        </style>
        <div class="row">
            <div class="item first"></div>
            <div class="item second"></div>
        </div>
        "#,
        &fonts,
    );
    let mut ownership = FlexFilterOwnership::default();
    for page in pages {
        for (_, element) in page.elements {
            visit_layout_tree(element.as_ref(), &mut ownership);
        }
    }
    assert_eq!(ownership.0, [true, false]);
}
