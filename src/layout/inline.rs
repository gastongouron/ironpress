use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BoxSizing,
    ComputedStyle, Display, LinearGradient, RadialGradient, TextAlign, Transform,
    compute_style_with_context,
};
use std::collections::HashMap;

use super::context::LayoutContext;
use super::engine::{BackgroundFields, FlexCell, LayoutBorder, LayoutElement, TextLine};
use super::text::{
    FlexTextRunCollector, TextWrapOptions, resolved_line_height_factor, wrap_text_runs,
};

/// Check if an element computes to `display: inline-block` given parent style and CSS rules.
pub(crate) fn element_is_inline_block(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
) -> bool {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
    };
    let style = compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    );
    // SVGs need individual block layout (they use cm operator for viewBox).
    style.display == Display::InlineBlock
        && el.tag != HtmlTag::Svg
        && !el
            .children
            .iter()
            .any(|c| matches!(c, DomNode::Element(e) if e.tag == HtmlTag::Svg))
}

/// Check if a natively-inline element has been styled with `display: block`
/// via CSS rules, making it a block-level element for layout purposes.
pub(crate) fn element_has_css_display_block(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
) -> bool {
    if el.tag.is_block() {
        return false; // already block by default
    }
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
    };
    let style = compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    );
    style.display == Display::Block
}

/// Lay out a group of consecutive `display: inline-block` elements as `FlexRow`s.
///
/// Each element is laid out independently into its own buffer, then positioned
/// horizontally in a row, similar to how `layout_flex_container` works.
pub(crate) fn layout_inline_block_group(
    elements: &[&ElementNode],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
) {
    let available_width = ctx.available_width();
    if elements.is_empty() {
        return;
    }

    // Lay out each inline-block element as a block to measure its size
    struct InlineBlockItem {
        width: f32,
        height: f32,
        lines: Vec<TextLine>,
        background_color: Option<(f32, f32, f32, f32)>,
        padding_top: f32,
        padding_right: f32,
        padding_bottom: f32,
        padding_left: f32,
        border: LayoutBorder,
        border_radius: f32,
        transform: Option<Transform>,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        text_align: TextAlign,
        margin_left: f32,
        margin_right: f32,
        box_shadow: Option<crate::style::computed::BoxShadow>,
    }

    let mut items: Vec<InlineBlockItem> = Vec::new();
    let child_count = elements.len();

    for (idx, child_el) in elements.iter().enumerate() {
        let classes = child_el.class_list();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
        };
        let child_style = compute_style_with_context(
            child_el.tag,
            child_el.style_attr(),
            parent_style,
            rules,
            child_el.tag_name(),
            &classes,
            child_el.id(),
            &child_el.attributes,
            &selector_ctx,
        );

        if child_style.display == Display::None {
            continue;
        }

        // Determine the element width
        let has_explicit_width = child_style.width.is_some();
        let child_w = child_style.width.unwrap_or(0.0);
        let child_h = child_style.height.unwrap_or(0.0);

        let inner_width = if has_explicit_width {
            if child_style.box_sizing == BoxSizing::BorderBox {
                child_w
                    - child_style.padding.left
                    - child_style.padding.right
                    - child_style.border.horizontal_width()
            } else {
                child_w
            }
            .max(0.0)
        } else {
            // No explicit width: use available width for shrink-to-fit
            available_width
        };

        // Collect text runs from the inline-block element's children
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(AncestorInfo {
            element: child_el,
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
        });
        let mut runs = Vec::new();
        FlexTextRunCollector {
            runs: &mut runs,
            rules,
            fonts,
        }
        .collect(
            &child_el.children,
            &child_style,
            None,
            (0.0, 0.0),
            &child_ancestors,
        );

        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    inner_width.max(1.0),
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, fonts),
                    child_style.overflow_wrap,
                )
                .with_rtl(child_style.direction_rtl),
                fonts,
            )
        } else {
            Vec::new()
        };

        // Total element width including padding + border
        let content_w = if has_explicit_width {
            child_w
        } else {
            // Shrink-to-fit: use the widest line
            lines
                .iter()
                .map(|l| {
                    l.runs
                        .iter()
                        .map(|r| {
                            crate::fonts::str_width(&r.text, r.font_size, &r.font_family, r.bold)
                        })
                        .sum::<f32>()
                })
                .fold(0.0f32, f32::max)
        };
        let total_w = if child_style.box_sizing == BoxSizing::BorderBox && has_explicit_width {
            content_w
        } else {
            content_w
                + child_style.padding.left
                + child_style.padding.right
                + child_style.border.horizontal_width()
        };

        // Total element height including padding + border
        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let content_h = if child_h > 0.0 { child_h } else { text_height };
        let total_h = if child_style.box_sizing == BoxSizing::BorderBox {
            content_h.max(child_h)
        } else {
            content_h
                + child_style.padding.top
                + child_style.padding.bottom
                + child_style.border.vertical_width()
        };

        let bg = child_style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgba());
        let bg_fields = BackgroundFields::from_style(&child_style);

        items.push(InlineBlockItem {
            width: total_w,
            height: total_h,
            lines,
            background_color: bg,
            padding_top: child_style.padding.top,
            padding_right: child_style.padding.right,
            padding_bottom: child_style.padding.bottom,
            padding_left: child_style.padding.left,
            border: LayoutBorder::from_computed(&child_style.border),
            border_radius: child_style.border_radius,
            transform: child_style.transform,
            background_gradient: bg_fields.gradient,
            background_radial_gradient: bg_fields.radial_gradient,
            background_svg: bg_fields.svg,
            background_blur_radius: bg_fields.blur_radius,
            background_size: bg_fields.size,
            background_position: bg_fields.position,
            background_repeat: bg_fields.repeat,
            background_origin: bg_fields.origin,
            text_align: child_style.text_align,
            margin_left: child_style.margin.left,
            margin_right: child_style.margin.right,
            box_shadow: child_style.box_shadow,
        });
    }

    if items.is_empty() {
        return;
    }

    // Position items horizontally, wrapping to new rows when they exceed available width
    let mut rows: Vec<(Vec<FlexCell>, f32)> = Vec::new(); // (cells, row_height)
    let mut current_cells: Vec<FlexCell> = Vec::new();
    let mut x = 0.0f32;
    let mut row_height = 0.0f32;

    for item in &items {
        let item_total_w = item.margin_left + item.width + item.margin_right;
        // Wrap to new row if this item would overflow
        if !current_cells.is_empty() && x + item_total_w > available_width + 0.01 {
            rows.push((std::mem::take(&mut current_cells), row_height));
            x = 0.0;
            row_height = 0.0;
        }

        x += item.margin_left;
        let natural_h: f32 = item.lines.iter().map(|l| l.height).sum();
        current_cells.push(FlexCell {
            lines: item.lines.clone(),
            x_offset: x,
            width: item.width,
            natural_height: natural_h,
            text_align: item.text_align,
            background_color: item.background_color,
            padding_top: item.padding_top,
            padding_right: item.padding_right,
            padding_bottom: item.padding_bottom,
            padding_left: item.padding_left,
            border: item.border,
            border_radius: item.border_radius,
            background_gradient: item.background_gradient.clone(),
            background_radial_gradient: item.background_radial_gradient.clone(),
            background_svg: item.background_svg.clone(),
            background_blur_radius: item.background_blur_radius,
            background_size: item.background_size,
            background_position: item.background_position,
            background_repeat: item.background_repeat,
            background_origin: item.background_origin,
            transform: item.transform,
            box_shadow: item.box_shadow,
            nested_elements: Vec::new(),
            y_offset: 0.0,
            line_cross_size: 0.0,
        });
        x += item.width + item.margin_right;
        row_height = row_height.max(item.height);
    }
    // Flush last row
    if !current_cells.is_empty() {
        rows.push((current_cells, row_height));
    }

    for (cells, rh) in rows {
        output.push(LayoutElement::FlexRow {
            cells,
            row_height: rh,
            margin_top: 0.0,
            margin_bottom: 0.0,
            background_color: None,
            container_width: available_width,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            border_radius: 0.0,
            box_shadow: None,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            align_items: crate::style::computed::AlignItems::Stretch,
        });
    }
}
