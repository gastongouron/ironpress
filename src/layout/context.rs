use crate::parser::css::CssRule;
use crate::parser::dom::ElementNode;
use crate::parser::ttf::TtfFont;
use crate::style::font_metrics::FontMetrics;
use std::collections::HashMap;

use super::elements::TableSourcePath;
use super::engine::CounterState;
use super::table::TableCellSizingContext;

/// Shared mutable environment for the layout traversal.
///
/// Bundles the CSS rules, font map, counter state, and raster options that flow through
/// every layout function unchanged in shape.
pub(crate) struct LayoutEnv<'a> {
    pub rules: &'a [CssRule],
    pub fonts: &'a HashMap<String, TtfFont>,
    pub counter_state: &'a mut CounterState,
    pub resources: &'a mut crate::security::resources::ResourceLoader,
    /// Document-wide `id -> element` map used to resolve `filter: url(#id)`
    /// (css-filter-effects-1 §3) to the inline SVG `<filter>` element. Built
    /// once over the whole DOM before the traversal begins.
    pub filter_defs: &'a HashMap<String, ElementNode>,
    /// Rasterization DPI for layout-time filter bitmaps such as replaced-image
    /// `filter: blur()` and `filter: drop-shadow()`.
    pub filter_dpi: f32,
    /// Measurements already taken by the table auto-sizing pass, owned by the
    /// current top-level layout. Auto table layout re-measures a cell once per
    /// sizing pass at every nesting level, so a deeply nested cell is otherwise
    /// measured exponentially often; borrowing the memo from here keeps it alive
    /// exactly as long as the DOM it describes.
    pub table_cell_sizing: TableCellSizingContext<'a>,
}

impl<'a> LayoutEnv<'a> {
    /// Font-relative CSS units resolve through the same borrowed font map that
    /// text layout and shaping use.
    pub(crate) const fn font_metrics(&self) -> FontMetrics<'a> {
        FontMetrics::new(self.fonts)
    }

    /// Reborrow this traversal inside a formatting-context source path without
    /// changing the selector ancestry seen by descendants.
    pub(crate) fn for_table_source<'scope>(
        &'scope mut self,
        source_path: &TableSourcePath,
    ) -> LayoutEnv<'scope> {
        LayoutEnv {
            rules: self.rules,
            fonts: self.fonts,
            counter_state: &mut *self.counter_state,
            resources: &mut *self.resources,
            filter_defs: self.filter_defs,
            filter_dpi: self.filter_dpi,
            table_cell_sizing: self.table_cell_sizing.source_descendants(source_path),
        }
    }
}

/// Containing block information for `position: absolute` elements.
/// Stores the containing block's position and dimensions so the renderer
/// can resolve offsets relative to the nearest positioned ancestor.
#[derive(Debug, Clone, Copy)]
pub struct ContainingBlock {
    /// X-offset of the containing block's left edge from the page left margin.
    pub x: f32,
    /// Width of the containing block.
    pub width: f32,
    /// Height of the containing block.
    pub height: f32,
    /// Depth of the positioned ancestor in the layout stack.
    pub depth: usize,
}

/// Page content-area dimensions (after margins).
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// Width, height, and font-size inherited from the parent box.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct ParentBox {
    pub content_width: f32,
    pub content_height: Option<f32>,
    pub font_size: f32,
    /// Width against which a child's percentage `width`/`min-width`/`max-width`
    /// resolves: the containing block's **content** width (CSS 2.1 § 10.2). For
    /// normal block flow this equals `content_width`. It is tracked separately
    /// because some layout modes (notably flex) hand a child an `available_width`
    /// equal to the child's own resolved size, while percentages must still
    /// resolve against the container's content width — the percentage basis the
    /// style cascade pre-resolved against.
    pub percent_width_basis: f32,
}

/// Contextual information that flows through the layout tree.
///
/// Replaces scattered `available_width` / `available_height` /
/// `abs_containing_block` parameters with a single struct.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct LayoutContext {
    pub viewport: Viewport,
    pub parent: ParentBox,
    /// Containing block for `position: absolute` descendants: the padding box of
    /// the nearest *positioned* (relative/absolute/fixed) ancestor. It is
    /// forwarded unchanged through `position: static` elements (which do NOT
    /// establish a containing block) and replaced only by positioned elements.
    pub containing_block: Option<ContainingBlock>,
    /// Containing block used to resolve a child's percentage `height` (CSS 2.1
    /// § 10.5): the parent's content box. Unlike `containing_block`, every block
    /// element replaces this with its own content box for its children.
    pub percent_height_cb: Option<ContainingBlock>,
    pub root_font_size: f32,
}

#[allow(dead_code)]
impl LayoutContext {
    /// Width available for the current element (parent content width).
    pub fn available_width(&self) -> f32 {
        self.parent.content_width
    }

    /// Height available for the current element, falling back to viewport.
    pub fn available_height(&self) -> f32 {
        self.parent.content_height.unwrap_or(self.viewport.height)
    }

    /// Initial fixed containing block. In paged media this is the page area,
    /// independent of ordinary positioned ancestors.
    pub const fn initial_fixed_containing_block(&self) -> ContainingBlock {
        ContainingBlock {
            x: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
            depth: 0,
        }
    }

    /// Return a child context with updated parent dimensions.
    pub fn with_parent(
        &self,
        content_width: f32,
        content_height: Option<f32>,
        font_size: f32,
    ) -> Self {
        LayoutContext {
            parent: ParentBox {
                content_width,
                content_height,
                font_size,
                // Normal block flow: percentages resolve against the parent's
                // content width, which is the `content_width` passed here.
                percent_width_basis: content_width,
            },
            ..*self
        }
    }

    /// Like [`with_parent`], but lets the caller specify a percentage-width
    /// basis that differs from `content_width`. Used by flex layout, which hands
    /// each item an `available_width` equal to the item's own resolved size while
    /// percentage widths must still resolve against the flex container's content
    /// width (the basis the style cascade pre-resolved against).
    pub fn with_parent_and_basis(
        &self,
        content_width: f32,
        percent_width_basis: f32,
        content_height: Option<f32>,
        font_size: f32,
    ) -> Self {
        LayoutContext {
            parent: ParentBox {
                content_width,
                content_height,
                font_size,
                percent_width_basis,
            },
            ..*self
        }
    }

    /// Return a child context with an updated absolute containing block.
    pub fn with_containing_block(&self, cb: Option<ContainingBlock>) -> Self {
        LayoutContext {
            containing_block: cb,
            ..*self
        }
    }

    /// Return a child context with both the absolute containing block and the
    /// percentage-height containing block set. A `position: static` block keeps
    /// the inherited `abs_cb` (it does not establish a containing block) while
    /// supplying its own content box as `percent_height_cb`.
    pub fn with_cbs(
        &self,
        abs_cb: Option<ContainingBlock>,
        percent_height_cb: Option<ContainingBlock>,
    ) -> Self {
        LayoutContext {
            containing_block: abs_cb,
            percent_height_cb,
            ..*self
        }
    }
}
