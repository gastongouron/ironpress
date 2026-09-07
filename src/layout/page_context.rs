use crate::layout::elements::{
    BackgroundBox, BackgroundBoxGeometry, IntoLayoutNode, PageAreaInFlowSpace,
};
use crate::layout::engine::Page;
use crate::parser::css::{PageRule, PageSelector, PageSelectorContext, StyleMap};
use crate::style::computed::{ComputedStyle, apply_style_map_with_font_metrics};
use crate::style::font_metrics::FontMetrics;
use crate::style::raster_quality::RasterQuality;
use crate::types::{Margin, PageSize, PhysicalEdges, Point, Size};

/// Physical page geometry and the root-owned inset applied to document flow.
///
/// `margin` is selected exclusively by the `@page` cascade. Root/body
/// margin, padding, and centering remain a separate flow inset: they narrow
/// layout but never move the physical page-area clip or page-margin boxes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PageGeometry {
    pub(crate) size: PageSize,
    pub(crate) margin: Margin,
    root_flow_insets: Margin,
}

impl PageGeometry {
    pub(crate) const fn new(size: PageSize, margin: Margin) -> Self {
        Self {
            size,
            margin,
            root_flow_insets: Margin::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    pub(crate) const fn with_root_flow_insets(mut self, insets: Margin) -> Self {
        self.root_flow_insets = insets;
        self
    }

    pub(crate) fn flow_margin(self) -> Margin {
        self.margin + self.root_flow_insets
    }

    pub(crate) fn content_height(self) -> f32 {
        self.size.height - self.flow_margin().vertical()
    }

    pub(crate) fn content_size(self) -> Size {
        let flow_margin = self.flow_margin();
        Size::new(
            self.size.width - flow_margin.horizontal(),
            self.content_height(),
        )
    }

    pub(crate) fn page_area_size(self) -> Size {
        Size::new(
            self.size.width - self.margin.horizontal(),
            self.size.height - self.margin.vertical(),
        )
    }

    pub(crate) fn page_area_in_flow_space(self) -> PageAreaInFlowSpace {
        PageAreaInFlowSpace::new(
            Point::new(-self.root_flow_insets.left, -self.root_flow_insets.top),
            self.page_area_size(),
        )
    }

    /// Convert a right edge measured from the root flow origin into the
    /// physical page area's coordinate space. Print fitting is anchored at
    /// that page-area origin, so a root-start gutter belongs in both the
    /// available page width and the measured document extent.
    pub(crate) fn flow_right_in_page_area(self, flow_right_edge: f32) -> f32 {
        self.root_flow_insets.left + flow_right_edge
    }
}

/// Physical-page-aware cascade for page size and margins.
///
/// The document-wide geometry already contains universal `@page` declarations
/// plus the root/body folds. Selected rules remain specified until pagination
/// knows the page number, blank state, and case-sensitive page type.
#[derive(Debug, Clone)]
pub(crate) struct PageGeometryContext {
    default: PageGeometry,
    rules: Vec<PageGeometryRule>,
}

#[derive(Debug, Clone)]
struct PageGeometryRule {
    selector: PageSelector,
    declarations: PageGeometryDeclarations,
}

#[derive(Debug, Clone, Copy, Default)]
struct PageGeometryDeclarations {
    size: Option<PageSize>,
    margin: PhysicalEdges<Option<f32>>,
}

impl PageGeometryContext {
    pub(crate) fn uniform(size: PageSize, margin: Margin) -> Self {
        Self {
            default: PageGeometry::new(size, margin),
            rules: Vec::new(),
        }
    }

    pub(crate) fn from_rules(
        default_size: PageSize,
        default_margin: Margin,
        page_rules: &[PageRule],
    ) -> Self {
        let rules = page_rules
            .iter()
            .filter(|rule| !rule.selector.is_universal())
            .filter_map(|rule| {
                let declarations = PageGeometryDeclarations {
                    size: match (rule.width, rule.height) {
                        (Some(width), Some(height)) => Some(PageSize::new(width, height)),
                        _ => None,
                    },
                    margin: PhysicalEdges::new(
                        rule.margin_top,
                        rule.margin_right,
                        rule.margin_bottom,
                        rule.margin_left,
                    ),
                };
                (declarations.size.is_some() || declarations.margin != PhysicalEdges::default())
                    .then(|| PageGeometryRule {
                        selector: rule.selector.clone(),
                        declarations,
                    })
            })
            .collect();
        Self {
            default: PageGeometry::new(default_size, default_margin),
            rules,
        }
    }

    pub(crate) fn with_root_flow_insets(mut self, insets: Margin) -> Self {
        self.default = self.default.with_root_flow_insets(insets);
        self
    }

    pub(crate) fn resolve(&self, page: PageSelectorContext<'_>) -> PageGeometry {
        let mut geometry = self.default;
        let mut matching: Vec<_> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.selector.applies_to(page))
            .collect();
        matching.sort_by_key(|(source_order, rule)| (rule.selector.specificity(), *source_order));
        for (_, rule) in matching {
            rule.declarations.apply(&mut geometry);
        }
        geometry
    }
}

impl PageGeometryDeclarations {
    fn apply(self, geometry: &mut PageGeometry) {
        if let Some(size) = self.size {
            geometry.size = size;
        }
        if let Some(value) = self.margin.top {
            geometry.margin.top = value;
        }
        if let Some(value) = self.margin.right {
            geometry.margin.right = value;
        }
        if let Some(value) = self.margin.bottom {
            geometry.margin.bottom = value;
        }
        if let Some(value) = self.margin.left {
            geometry.margin.left = value;
        }
    }
}

/// Deferred `@page` background cascade.
///
/// A physical page does not acquire its `:first`, spread, blank, or named
/// identity until pagination. Retaining specified declarations here prevents
/// selected rules from being flattened into a document-global paint.
#[derive(Debug, Clone)]
pub(crate) struct PageBackgroundContext {
    rules: Vec<PageBackgroundRule>,
    initial_style: ComputedStyle,
    bleed: f32,
}

#[derive(Debug, Clone)]
struct PageBackgroundRule {
    selector: PageSelector,
    declarations: StyleMap,
}

impl PageBackgroundContext {
    pub(crate) fn from_rules(
        page_rules: &[PageRule],
        raster_quality: RasterQuality,
        bleed: f32,
    ) -> Self {
        let rules = page_rules
            .iter()
            .filter_map(|rule| {
                let declarations =
                    crate::parser::css::parse_inline_style(rule.raw_declarations.as_deref()?);
                (!declarations.properties.is_empty()).then(|| PageBackgroundRule {
                    selector: rule.selector.clone(),
                    declarations,
                })
            })
            .collect();
        Self {
            rules,
            initial_style: ComputedStyle::with_raster_quality(raster_quality),
            bleed,
        }
    }

    pub(crate) fn uniform(
        style: Option<&ComputedStyle>,
        bleed: f32,
        raster_quality: RasterQuality,
    ) -> Self {
        let mut context = Self {
            rules: Vec::new(),
            initial_style: ComputedStyle::with_raster_quality(raster_quality),
            bleed,
        };
        if let Some(style) = style {
            context.initial_style = style.clone();
        }
        context
    }

    pub(crate) fn apply(
        &self,
        pages: &mut [Page],
        default_page_size: PageSize,
        default_margin: Margin,
        fonts: &dyn crate::font_registry::FontRegistry,
    ) {
        for (page_index, page) in pages.iter_mut().enumerate() {
            let page_number = page_index + 1;
            let selector_context = PageSelectorContext {
                page_number,
                is_blank: page.is_blank,
                page_name: page.page_name.as_deref(),
            };
            let Some(style) = self.resolve(selector_context, fonts) else {
                continue;
            };
            if !crate::layout::helpers::has_background_paint(&style) {
                continue;
            }

            let geometry = page
                .geometry
                .unwrap_or(PageGeometry::new(default_page_size, default_margin));
            let flow_margin = geometry.flow_margin();
            let origin = Point::new(
                -flow_margin.left - self.bleed,
                -flow_margin.top - self.bleed,
            );
            let geometry = BackgroundBoxGeometry::page_backdrop(
                Size::new(
                    geometry.size.width + 2.0 * self.bleed,
                    geometry.size.height + 2.0 * self.bleed,
                ),
                origin,
                -2,
            );
            let background = BackgroundBox::new(&style, geometry).boxed();
            // This box is attached after pagination, so its absolute block
            // offset has not passed through the paginator's normal
            // `Positioning -> page y` projection. Store that resolved page y
            // beside the node just as pagination does for every other absolute
            // box. The retained Positioning still supplies the inline offset.
            page.elements.insert(0, (origin.y, background));
        }
    }

    fn resolve(
        &self,
        page: PageSelectorContext<'_>,
        fonts: &dyn crate::font_registry::FontRegistry,
    ) -> Option<ComputedStyle> {
        if self.rules.is_empty() {
            return crate::layout::helpers::has_background_paint(&self.initial_style)
                .then(|| self.initial_style.clone());
        }

        let mut matching: Vec<_> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.selector.applies_to(page))
            .collect();
        matching.sort_by_key(|(source_order, rule)| (rule.selector.specificity(), *source_order));
        if matching.is_empty() {
            return None;
        }

        let mut declarations = StyleMap::new();
        for (_, rule) in matching {
            declarations.merge(&rule.declarations);
        }
        let mut style = self.initial_style.clone();
        apply_style_map_with_font_metrics(
            &mut style,
            &declarations,
            &self.initial_style,
            FontMetrics::new(fonts),
        );
        Some(style)
    }
}

#[cfg(test)]
mod tests;
