//! Root formatting-context construction after page geometry projection.
//!
//! The HTML parser exposes the body children directly, while the conversion
//! boundary projects the body margin and padding into page geometry. A body
//! with a non-block inner display still needs one real flex/grid formatting
//! context around those children; inheriting its computed style into each child
//! cannot represent that relationship.

use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::style::computed::{
    BoxSizing, ComputedStyle, Display, PercentageBasis, SpecifiedCornerRadii,
    compute_style_with_context_and_percentage_basis_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::EdgeSizes;

/// Independently cascaded root-element and body styles.
///
/// Keeping these boxes distinct prevents non-inherited HTML properties from
/// leaking into body layout and lets the ordinary selector cascade apply
/// universal rules such as `* { box-sizing: border-box }` to the body.
pub(crate) struct DocumentRootStyles {
    pub(crate) html: ComputedStyle,
    pub(crate) body: ComputedStyle,
}

impl DocumentRootStyles {
    pub(crate) fn resolve(
        children: &[DomNode],
        rules: &[CssRule],
        raster_quality: crate::style::raster_quality::RasterQuality,
        viewport_width: f32,
        viewport_height: f32,
        font_metrics: FontMetrics<'_>,
    ) -> Self {
        let mut initial = ComputedStyle::with_raster_quality(raster_quality);
        initial.viewport_width = viewport_width;
        initial.viewport_height = viewport_height;
        initial.width = Some(viewport_width);

        let html_element = ElementNode::new(HtmlTag::Html);
        let basis = PercentageBasis::new(Some(viewport_width), Some(viewport_height));
        let mut html = compute_style_with_context_and_percentage_basis_with_font_metrics(
            HtmlTag::Html,
            None,
            &initial,
            rules,
            "html",
            &[],
            None,
            &html_element.attributes,
            &SelectorContext {
                is_empty: children.is_empty(),
                ..Default::default()
            },
            basis,
            font_metrics,
        );
        html.root_font_size = html.font_size;
        html.root_font_units = html.font_unit_lengths(font_metrics);

        let body_element = ElementNode::new(HtmlTag::Body);
        let body = compute_style_with_context_and_percentage_basis_with_font_metrics(
            HtmlTag::Body,
            None,
            &html,
            rules,
            "body",
            &[],
            None,
            &body_element.attributes,
            &SelectorContext {
                ancestors: vec![AncestorInfo {
                    element: &html_element,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Default::default(),
                    following_siblings: Default::default(),
                    is_empty: false,
                }],
                sibling_count: 1,
                is_empty: children.is_empty(),
                ..Default::default()
            },
            basis,
            font_metrics,
        );

        Self { html, body }
    }

    pub(crate) fn start_page_name(&self) -> Option<&str> {
        self.body
            .page_name
            .as_deref()
            .or(self.html.page_name.as_deref())
    }
}

/// One synthetic body box used only to establish its authored inner formatting
/// context. Page geometry and canvas paint remain owned by the root projection.
pub(crate) struct RootFormattingContext {
    element: ElementNode,
    style: ComputedStyle,
}

impl RootFormattingContext {
    /// Build a root container only for a display mode whose children cannot be
    /// represented by ordinary block flattening.
    pub(crate) fn from_projected_body(
        children: &[DomNode],
        body_style: &ComputedStyle,
        content_width: f32,
    ) -> Option<Self> {
        matches!(body_style.display, Display::Flex | Display::Grid).then(|| {
            let mut element = ElementNode::new(HtmlTag::Body);
            element.children = children.to_vec();

            let mut style = body_style.clone();
            let content_height = style.height.map(|height| match style.box_sizing {
                BoxSizing::BorderBox => {
                    (height - style.padding.vertical() - style.border.vertical_width()).max(0.0)
                }
                BoxSizing::ContentBox => height,
            });
            // Horizontal body margins and padding are already part of the page
            // content geometry, and root background propagation owns canvas
            // paint. Keep every inner-layout property while removing only that
            // already-projected outer box state.
            style.margin = EdgeSizes::ZERO;
            style.padding = EdgeSizes::ZERO;
            style.border = Default::default();
            style.border_image = Default::default();
            style.border_radii = SpecifiedCornerRadii::ZERO;
            style.outline_width = 0.0;
            style.outline_color = None;
            style.outline_offset = 0.0;
            style.reset_background();
            style.box_shadow.clear();
            style.width = Some(content_width);
            style.height = content_height;
            style.min_width = None;
            style.max_width = None;

            Self { element, style }
        })
    }

    pub(crate) const fn element(&self) -> &ElementNode {
        &self.element
    }

    pub(crate) const fn style(&self) -> &ComputedStyle {
        &self.style
    }

    /// Selector ancestry seen by every direct body child.
    pub(crate) fn descendant_ancestors(&self) -> [AncestorInfo<'_>; 1] {
        [AncestorInfo {
            element: &self.element,
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Default::default(),
            following_siblings: Default::default(),
            is_empty: self.element.children.is_empty(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_root_retains_inner_layout_and_removes_outer_projection() {
        let style = ComputedStyle {
            display: Display::Grid,
            margin: EdgeSizes::uniform(9.0),
            padding: EdgeSizes::uniform(7.0),
            width: Some(123.0),
            ..Default::default()
        };

        let root = RootFormattingContext::from_projected_body(&[], &style, 240.0)
            .expect("grid establishes a root formatting context");

        assert_eq!(root.style.display, Display::Grid);
        assert_eq!(root.style.margin, EdgeSizes::ZERO);
        assert_eq!(root.style.padding, EdgeSizes::ZERO);
        assert_eq!(root.style.width, Some(240.0));
        assert_eq!(root.element.tag, HtmlTag::Body);
    }

    #[test]
    fn body_page_name_supplies_the_root_start_page_value() {
        let styles = DocumentRootStyles {
            html: ComputedStyle {
                page_name: Some("volume".to_string()),
                ..Default::default()
            },
            body: ComputedStyle {
                page_name: Some("chapter".to_string()),
                ..Default::default()
            },
        };

        assert_eq!(styles.start_page_name(), Some("chapter"));
    }

    #[test]
    fn html_page_name_supplies_the_root_start_page_value() {
        let styles = DocumentRootStyles {
            html: ComputedStyle {
                page_name: Some("volume".to_string()),
                ..Default::default()
            },
            body: ComputedStyle::default(),
        };

        assert_eq!(styles.start_page_name(), Some("volume"));
    }
}
