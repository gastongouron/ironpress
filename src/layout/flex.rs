use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    AlignItems, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BoxSizing,
    Clear, ComputedStyle, Display, FlexDirection, FlexWrap, Float, JustifyContent, Overflow,
    Position, TextAlign, VerticalAlign, Visibility, compute_style_with_context,
};

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    BackgroundFields, FlexCell, LayoutBorder, LayoutElement, TextLine, aspect_ratio_height,
    background_svg_for_style, collects_as_inline_text, flatten_element, has_background_paint,
    measure_runs_width, pseudo_is_block_like, push_block_pseudo, resolve_padding_box_height,
};
use super::paginate::estimate_element_height;
use super::text::{
    FlexTextRunCollector, TextWrapOptions, resolved_line_height_factor, wrap_text_runs,
};

/// Each child is laid out as a TextBlock at a computed position. The container
/// emits one TextBlock per flex item with an `offset_left` / `offset_top` that
/// encodes its position inside the flex row/column. The container itself emits
/// a wrapper TextBlock for its background/border first, then the items.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_flex_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    before_style: Option<&ComputedStyle>,
    after_style: Option<&ComputedStyle>,
    positioned_depth: usize,
    env: &mut LayoutEnv,
) {
    let available_width = ctx.available_width();
    let mut block_w = available_width;
    if let Some(w) = style.width {
        block_w = w.min(available_width);
    }
    if let Some(mw) = style.max_width {
        block_w = block_w.min(mw);
    }

    let inner_width = block_w - style.padding.left - style.padding.right;

    // Resolve percentage border-radius for flex containers
    let resolved_border_radius = if let Some(pct) = style.border_radius_pct {
        let dim = style.height.map_or(block_w, |h| block_w.min(h));
        dim * pct / 100.0
    } else {
        style.border_radius
    };

    // Collect child elements and lay each one out into a temporary buffer
    let child_elements: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|c| {
            if let DomNode::Element(e) = c {
                Some(e)
            } else {
                None
            }
        })
        .collect();

    let child_count = child_elements.len();
    if child_count == 0 {
        let before_abs = before_style.is_some_and(|pseudo| {
            pseudo_is_block_like(pseudo) && pseudo.position == Position::Absolute
        });
        let after_abs = after_style.is_some_and(|pseudo| {
            pseudo_is_block_like(pseudo) && pseudo.position == Position::Absolute
        });
        if has_background_paint(style)
            || style.border.has_any()
            || resolved_border_radius > 0.0
            || style.box_shadow.is_some()
            || style.aspect_ratio.is_some()
            || style.height.is_some()
            || before_abs
            || after_abs
        {
            let container_h = style
                .height
                .or_else(|| aspect_ratio_height(block_w, style))
                .unwrap_or(0.0);
            let containing_block = (style.position == Position::Relative
                || style.position == Position::Absolute)
                .then(|| ContainingBlock {
                    x: style.left.unwrap_or(0.0) + style.border.left.width + style.padding.left,
                    width: if style.box_sizing == BoxSizing::BorderBox {
                        block_w - style.border.horizontal_width()
                    } else {
                        block_w + style.padding.left + style.padding.right
                    },
                    height: container_h,
                    depth: positioned_depth,
                });
            let bg = style
                .background_color
                .map(|color: crate::types::Color| color.to_f32_rgba());
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(style);
            output.push(LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                background_color: bg,
                padding_top: style.padding.top,
                padding_bottom: style.padding.bottom,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::from_computed(&style.border),
                block_width: Some(block_w),
                block_height: Some(container_h),
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect: if style.overflow == Overflow::Hidden {
                    Some((0.0, 0.0, block_w, container_h))
                } else {
                    None
                },
                transform: style.transform,
                border_radius: resolved_border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: 0.0,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                vertical_align: style.vertical_align,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth,
                heading_level: None,
                clip_children_count: 0,
            });

            if before_abs {
                push_block_pseudo(
                    output,
                    before_style,
                    el,
                    inner_width.max(0.0),
                    env.fonts,
                    containing_block,
                    positioned_depth,
                    env.counter_state,
                );
            }
            if after_abs {
                push_block_pseudo(
                    output,
                    after_style,
                    el,
                    inner_width.max(0.0),
                    env.fonts,
                    containing_block,
                    positioned_depth,
                    env.counter_state,
                );
            }
        }
        return;
    }

    // Lay out each child into its own set of elements to measure sizes
    #[allow(dead_code)]
    struct FlexItem {
        elements: Vec<LayoutElement>,
        width: f32,
        base_width: f32,
        flex_grow: f32,
        flex_shrink: f32,
        height: f32,
        natural_height: f32,
    }

    let mut items: Vec<FlexItem> = Vec::new();

    // For percentage width resolution, children need the actual container width
    // as the parent reference (not the CSS width which may be None).
    // Subtract total gap space so that percentage widths + gaps fit within the container.
    let total_gaps = style.gap * (child_count.saturating_sub(1)) as f32;
    let width_for_percentages = (inner_width - total_gaps).max(0.0);
    let mut parent_for_children = style.clone();
    if parent_for_children.width.is_none() {
        parent_for_children.width = Some(width_for_percentages);
    }

    for (idx, child_el) in child_elements.iter().enumerate() {
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
            &parent_for_children,
            env.rules,
            child_el.tag_name(),
            &classes,
            child_el.id(),
            &child_el.attributes,
            &selector_ctx,
        );

        if child_style.display == Display::None {
            continue;
        }

        // Determine child width: flex-basis takes priority, then explicit width.
        // Flex base size for grow/shrink distribution:
        // - With flex-basis or width: use that value
        // - flex-grow > 0 without basis/width: use 0 so all space is distributed
        //   proportionally by grow factors
        // - flex-grow == 0 without basis/width: use equal share, then shrink to
        //   natural content width (for justify-content)
        //
        // For `box-sizing: content-box` (the CSS default), the specified width
        // is the *content* width, so the outer box used for flex main-axis
        // layout is `width + padding + border`. For `border-box`, the
        // specified width is already the outer box.
        let has_explicit_width = child_style.flex_basis.is_some() || child_style.width.is_some();
        let inflate_outer = |w: f32| -> f32 {
            if child_style.box_sizing == BoxSizing::ContentBox {
                w + child_style.padding.left
                    + child_style.padding.right
                    + child_style.border.horizontal_width()
            } else {
                w
            }
        };
        let child_w_initial = match child_style.flex_basis.or(child_style.width) {
            Some(w) => inflate_outer(w),
            None => {
                if child_style.flex_grow > 0.0 {
                    0.0
                } else {
                    width_for_percentages / child_count as f32
                }
            }
        };
        // For text wrapping, use equal share as measurement width even when
        // flex base is 0 — text needs a nonzero width to wrap into lines.
        // The actual item width will be set after grow distribution.
        let wrap_width = if child_w_initial < 1.0 && child_style.flex_grow > 0.0 {
            width_for_percentages / child_count as f32
        } else {
            child_w_initial
        };

        // Include the child element itself in the ancestor chain so that
        // descendant selectors like `.card h3` can match.
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(AncestorInfo {
            element: child_el,
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
        });

        // Two widths: child_w_for_flex is the outer main-axis size used for
        // wrapping decisions (content-box + padding + border for content-box),
        // child_w_for_layout is the content width used to lay out children so
        // percentage resolution against the parent content area is correct.
        let child_w_for_flex = match child_style.flex_basis.or(child_style.width) {
            Some(w) => inflate_outer(w),
            None => width_for_percentages / child_count as f32,
        };
        let child_w_for_layout = if child_style.flex_grow > 0.0
            && child_style.flex_basis == Some(0.0)
            && child_style.width.is_none()
        {
            // Use full available width for child percentage resolution,
            // but flex wrapping uses the actual basis (child_w_for_flex).
            width_for_percentages
        } else {
            // Content area for child layout = outer minus padding + border.
            (child_w_for_flex
                - child_style.padding.left
                - child_style.padding.right
                - child_style.border.horizontal_width())
            .max(0.0)
        };

        // Check if this flex item has block-level children that need full layout
        let item_has_block_children = child_el.children.iter().any(|c| {
            matches!(c, DomNode::Element(e) if e.tag.is_block() && !collects_as_inline_text(e.tag))
        });

        // For complex flex items (with block children like <h2>, <p>, <div>),
        // use flatten_element to get a proper list of layout elements with
        // margins and structure preserved.
        if item_has_block_children {
            let mut child_elements_buf = Vec::new();
            let layout_height = 10000.0; // large enough to not constrain
            let child_ctx = ctx
                .with_parent(child_w_for_layout, Some(layout_height), style.font_size)
                .with_containing_block(None);
            flatten_element(
                child_el,
                style,
                &child_ctx,
                &mut child_elements_buf,
                None,
                &child_ancestors,
                positioned_depth,
                idx,
                child_count,
                &[],
                env,
            );
            let child_h = child_elements_buf
                .iter()
                .map(|el| match el {
                    LayoutElement::TextBlock {
                        lines,
                        padding_top,
                        padding_bottom,
                        border,
                        block_height,
                        ..
                    } => {
                        let text_h: f32 = lines.iter().map(|l| l.height).sum();
                        let content =
                            padding_top + text_h + padding_bottom + border.vertical_width();
                        // Don't include margins here — they are added as spacer
                        // lines in the merged FlexCell, so counting them would
                        // double the vertical space.
                        block_height.map_or(content, |h| content.max(h))
                    }
                    LayoutElement::FlexRow {
                        cells,
                        margin_top,
                        margin_bottom,
                        ..
                    } => {
                        let row_h = cells
                            .iter()
                            .map(|c| {
                                let text_h: f32 = c.lines.iter().map(|l| l.height).sum();
                                c.padding_top + text_h + c.padding_bottom
                            })
                            .fold(0.0f32, f32::max);
                        margin_top + row_h + margin_bottom
                    }
                    other => estimate_element_height(other),
                })
                .sum::<f32>();

            items.push(FlexItem {
                elements: child_elements_buf,
                width: child_w_for_flex,
                base_width: child_w_for_flex,
                flex_grow: child_style.flex_grow,
                flex_shrink: child_style.flex_shrink,
                height: child_h,
                natural_height: child_h, // Natural height for align-items flex-start
            });
            continue;
        }

        // Simple flex items: collect text runs and wrap
        let mut runs = Vec::new();
        FlexTextRunCollector {
            runs: &mut runs,
            rules: env.rules,
            fonts: env.fonts,
        }
        .collect(
            &child_el.children,
            &child_style,
            None,
            (0.0, 0.0),
            &child_ancestors,
        );

        // When no explicit width/flex-basis and flex-grow is 0, measure the
        // natural (intrinsic) content width so the item shrinks to fit.
        let child_w = if !has_explicit_width && child_style.flex_grow == 0.0 && !runs.is_empty() {
            let natural_text_w = measure_runs_width(&runs, env.fonts);
            let pad_h = child_style.padding.left + child_style.padding.right;
            let border_h = child_style.border.horizontal_width();
            // Outer width = text + padding + border (capped at container)
            (natural_text_w + pad_h + border_h).min(width_for_percentages)
        } else {
            child_w_initial
        };

        // Use wrap_width for text measurement (nonzero even when flex base is 0)
        let wrap_w = if child_style.flex_grow > 0.0 && !has_explicit_width {
            wrap_width
        } else {
            child_w
        };
        // wrap_w is always the outer box width (after content-box inflation),
        // so the inner content area is outer - padding - border.
        let child_inner_w = (wrap_w
            - child_style.padding.left
            - child_style.padding.right
            - child_style.border.horizontal_width())
        .max(0.0);

        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    child_inner_w.max(1.0),
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, env.fonts),
                    child_style.overflow_wrap,
                ),
                env.fonts,
            )
        } else {
            Vec::new()
        };

        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let aspect_h = child_style
            .height
            .is_none()
            .then(|| aspect_ratio_height(child_w, &child_style))
            .flatten();
        let mut child_h = resolve_padding_box_height(
            text_height,
            child_style.height,
            child_style.padding.top,
            child_style.padding.bottom,
            child_style.border.vertical_width(),
            child_style.box_sizing,
        );
        if let Some(aspect_h) = aspect_h {
            child_h = child_h.max(aspect_h);
        }

        let bg = child_style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgba());
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&child_style);
        let elem = LayoutElement::TextBlock {
            lines,
            margin_top: child_style.margin.top,
            margin_bottom: child_style.margin.bottom,
            text_align: child_style.text_align,
            background_color: bg,
            padding_top: child_style.padding.top,
            padding_bottom: child_style.padding.bottom,
            padding_left: child_style.padding.left,
            padding_right: child_style.padding.right,
            border: LayoutBorder::from_computed(&child_style.border),
            block_width: Some(child_w),
            block_height: child_style
                .height
                .map(|_| child_h)
                .or(aspect_h.map(|_| child_h)),
            opacity: child_style.opacity,
            float: Float::None,
            clear: Clear::None,
            position: child_style.position,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: child_style.box_shadow,
            visible: child_style.visibility == Visibility::Visible,
            clip_rect: if child_style.overflow == Overflow::Hidden {
                Some((0.0, 0.0, child_w, child_h))
            } else {
                None
            },
            transform: child_style.transform,
            border_radius: child_style.border_radius,
            outline_width: child_style.outline_width,
            outline_color: child_style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: child_style.text_indent,
            letter_spacing: child_style.letter_spacing,
            word_spacing: child_style.word_spacing,
            vertical_align: child_style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: child_style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        };

        items.push(FlexItem {
            elements: vec![elem],
            width: child_w,
            base_width: child_w,
            flex_grow: child_style.flex_grow,
            flex_shrink: child_style.flex_shrink,
            height: child_h + child_style.margin.top + child_style.margin.bottom,
            natural_height: child_h + child_style.margin.top + child_style.margin.bottom,
        });
    }

    if items.is_empty() {
        return;
    }

    let direction = style.flex_direction;
    let justify = style.justify_content;
    let align = style.align_items;
    let wrap = style.flex_wrap;
    let gap = style.gap;

    // Group items into lines (for flex-wrap)
    struct FlexLine {
        item_indices: Vec<usize>,
        main_size: f32,
        cross_size: f32,
    }

    let mut lines: Vec<FlexLine> = Vec::new();

    match direction {
        FlexDirection::Row => {
            let max_main = inner_width;
            let mut current_line = FlexLine {
                item_indices: Vec::new(),
                main_size: 0.0,
                cross_size: 0.0,
            };

            for (i, item) in items.iter().enumerate() {
                let item_main = item.width;
                let gap_extra = if current_line.item_indices.is_empty() {
                    0.0
                } else {
                    gap
                };

                if wrap == FlexWrap::Wrap
                    && !current_line.item_indices.is_empty()
                    && current_line.main_size + gap_extra + item_main > max_main
                {
                    lines.push(current_line);
                    current_line = FlexLine {
                        item_indices: Vec::new(),
                        main_size: 0.0,
                        cross_size: 0.0,
                    };
                }

                if !current_line.item_indices.is_empty() {
                    current_line.main_size += gap;
                }
                current_line.main_size += item_main;
                current_line.cross_size = current_line.cross_size.max(item.height);
                current_line.item_indices.push(i);
            }
            if !current_line.item_indices.is_empty() {
                lines.push(current_line);
            }
        }
        FlexDirection::Column => {
            // In column direction, each item is on its own "line" conceptually,
            // but we group them all into one line for simplicity (no column wrap needed yet)
            let mut line = FlexLine {
                item_indices: Vec::new(),
                main_size: 0.0,
                cross_size: 0.0,
            };
            for (i, item) in items.iter().enumerate() {
                if !line.item_indices.is_empty() {
                    line.main_size += gap;
                }
                line.main_size += item.height;
                line.cross_size = line.cross_size.max(item.width);
                line.item_indices.push(i);
            }
            if !line.item_indices.is_empty() {
                lines.push(line);
            }
        }
    }

    // Compute container dimensions
    let total_cross: f32 = match direction {
        FlexDirection::Row => {
            lines.iter().map(|l| l.cross_size).sum::<f32>()
                + if lines.len() > 1 {
                    (lines.len() - 1) as f32 * gap
                } else {
                    0.0
                }
        }
        FlexDirection::Column => lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max),
    };

    let total_main: f32 = match direction {
        FlexDirection::Row => inner_width,
        FlexDirection::Column => lines.iter().map(|l| l.main_size).sum::<f32>(),
    };

    let container_height = match direction {
        FlexDirection::Row => total_cross,
        FlexDirection::Column => total_main,
    };

    // `container_h` is the padding-box height (content + vertical padding).
    // `height` / `min-height` are defined against the content box in
    // `box-sizing: content-box` and against the border box in
    // `box-sizing: border-box`. Translate both to a padding-box comparand so
    // the max() here honors Chrome's semantics.
    let pad_v = style.padding.top + style.padding.bottom;
    let border_v = style.border.vertical_width();
    let container_h = style.padding.top + container_height + style.padding.bottom;
    let container_h = match style.height {
        Some(h) => {
            let target = match style.box_sizing {
                BoxSizing::ContentBox => h + pad_v,
                BoxSizing::BorderBox => (h - border_v).max(0.0),
            };
            container_h.max(target)
        }
        None => container_h,
    };
    let container_h = match style.min_height {
        Some(min_h) => {
            let target = match style.box_sizing {
                BoxSizing::ContentBox => min_h + pad_v,
                BoxSizing::BorderBox => (min_h - border_v).max(0.0),
            };
            container_h.max(target)
        }
        None => container_h,
    };
    // Cross-axis inner size once height/min-height have been honored. For
    // row direction with a single line this is what each item should
    // stretch to (align-items: stretch) and what flex-end/center measure
    // against — otherwise a tall `min-height` container collapses visually
    // to the natural item height.
    let inner_cross_size = (container_h - style.padding.top - style.padding.bottom).max(0.0);
    if direction == FlexDirection::Row && lines.len() == 1 {
        if let Some(line) = lines.first_mut() {
            line.cross_size = line.cross_size.max(inner_cross_size);
        }
    }
    // Recompute total_cross after possibly growing a single line.
    let total_cross: f32 = match direction {
        FlexDirection::Row => {
            lines.iter().map(|l| l.cross_size).sum::<f32>()
                + if lines.len() > 1 {
                    (lines.len() - 1) as f32 * gap
                } else {
                    0.0
                }
        }
        FlexDirection::Column => lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max),
    };
    let bg = style
        .background_color
        .map(|color: crate::types::Color| color.to_f32_rgba());

    // For column direction, emit container background separately
    let emitted_column_bg = direction == FlexDirection::Column
        && (has_background_paint(style) || style.border.has_any() || style.box_shadow.is_some());
    if emitted_column_bg {
        // Emit the container background/border as a visual element.
        // It advances y by its full height in paginate.  We then emit a
        // negative-margin spacer to pull y back so children flow *inside*
        // the background rather than after it.
        let bg_flow_height = container_h + style.border.vertical_width();
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(style);
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: style.margin.top,
            margin_bottom: 0.0,
            text_align: style.text_align,
            background_color: bg,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(block_w),
            block_height: Some(container_h),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: if style.overflow == Overflow::Hidden {
                Some((0.0, 0.0, block_w, container_h))
            } else {
                None
            },
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
        // Pull y back so children flow inside the container background
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::none();
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: -bg_flow_height,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
    }

    // Position items within the flex container and emit them
    let mut cross_offset = 0.0;
    // All flex cells across every line, merged into a single FlexRow for
    // row direction. This keeps container borders/backgrounds around every
    // wrapped line and keeps pagination flow correct.
    let mut all_flex_cells: Vec<FlexCell> = Vec::new();

    for line in &lines {
        let line_items: Vec<usize> = line.item_indices.clone();
        let line_item_count = line_items.len();

        match direction {
            FlexDirection::Row => {
                let total_item_width: f32 = line_items.iter().map(|&i| items[i].width).sum();
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let mut free_space = inner_width - total_item_width - total_gap;

                // Flex grow: distribute positive free space proportionally
                let total_grow: f32 = line_items.iter().map(|&i| items[i].flex_grow).sum();
                if free_space > 0.0 && total_grow > 0.0 {
                    for &i in &line_items {
                        items[i].width += free_space * (items[i].flex_grow / total_grow);
                    }
                    free_space = 0.0;
                }

                // Flex shrink: shrink items when overflowing
                if free_space < 0.0 {
                    let total_shrink_weighted: f32 = line_items
                        .iter()
                        .map(|&i| items[i].flex_shrink * items[i].base_width)
                        .sum();
                    if total_shrink_weighted > 0.0 {
                        let deficit = -free_space;
                        for &i in &line_items {
                            let shrink_ratio =
                                items[i].flex_shrink * items[i].base_width / total_shrink_weighted;
                            items[i].width = (items[i].width - deficit * shrink_ratio).max(0.0);
                        }
                    }
                    free_space = 0.0;
                }

                // Second pass: re-layout flex-grow items whose width changed
                // significantly. This ensures percentage-width children inside
                // flex items resolve against the final cell width, not the
                // initial estimate.
                for &i in &line_items {
                    if items[i].flex_grow > 0.0
                        && (items[i].width - items[i].base_width).abs() > 1.0
                    {
                        let final_w = items[i].width;
                        let child_el = child_elements[i];
                        let has_block_kids = child_el.children.iter().any(|c| {
                            matches!(c, DomNode::Element(e) if e.tag.is_block() && !collects_as_inline_text(e.tag))
                        });
                        if has_block_kids {
                            let mut relayout_buf = Vec::new();
                            let mut relayout_ancestors = ancestors.to_vec();
                            relayout_ancestors.push(AncestorInfo {
                                element: el,
                                child_index: 0,
                                sibling_count: 0,
                                preceding_siblings: Vec::new(),
                            });
                            let relayout_ctx = ctx
                                .with_parent(final_w, Some(10000.0), style.font_size)
                                .with_containing_block(None);
                            flatten_element(
                                child_el,
                                style,
                                &relayout_ctx,
                                &mut relayout_buf,
                                None,
                                &relayout_ancestors,
                                positioned_depth,
                                i,
                                child_count,
                                &[],
                                env,
                            );
                            if !relayout_buf.is_empty() {
                                items[i].elements = relayout_buf;
                                items[i].height =
                                    items[i].elements.iter().map(estimate_element_height).sum();
                            }
                        }
                    }
                }

                let free_space = free_space.max(0.0);

                // Calculate starting x and spacing based on justify-content
                let (mut x, extra_gap) = match justify {
                    JustifyContent::FlexStart => (0.0, 0.0),
                    JustifyContent::FlexEnd => (free_space, 0.0),
                    JustifyContent::Center => (free_space / 2.0, 0.0),
                    JustifyContent::SpaceBetween => {
                        if line_item_count > 1 {
                            (0.0, free_space / (line_item_count - 1) as f32)
                        } else {
                            (0.0, 0.0)
                        }
                    }
                    JustifyContent::SpaceAround => {
                        let around = free_space / line_item_count as f32;
                        (around / 2.0, around)
                    }
                };

                // Build FlexCells for this row line.
                let mut flex_cells = Vec::new();
                for &item_idx in &line_items {
                    let item = &items[item_idx];

                    // Complex items (multiple elements): merge all lines
                    // into a single FlexCell, inserting margin spacing
                    if item.elements.len() > 1 {
                        let mut merged_lines = Vec::new();
                        let mut first_bg = None;
                        let mut first_pt = 0.0f32;
                        let mut first_pb = 0.0f32;
                        let mut first_pl = 0.0f32;
                        let mut first_pr = 0.0f32;
                        let mut first_br = 0.0f32;
                        let mut is_first = true;
                        // Check if all elements are TextBlocks without borders (mergeable).
                        // TextBlocks with borders must go through nested_elements
                        // so the renderer can draw their individual borders.
                        let all_text_blocks = item.elements.iter().all(|e| {
                            matches!(e, LayoutElement::TextBlock { border, .. } if !border.has_any())
                        });

                        if !all_text_blocks {
                            // Mixed elements (e.g. TextBlock + TableRow):
                            // store in nested_elements for the renderer to handle
                            flex_cells.push(FlexCell {
                                lines: Vec::new(),
                                x_offset: x,
                                width: item.width,
                                natural_height: item.height,
                                text_align: TextAlign::Left,
                                background_color: None,
                                padding_top: 0.0,
                                padding_right: 0.0,
                                padding_bottom: 0.0,
                                padding_left: 0.0,
                                border: LayoutBorder::default(),
                                border_radius: 0.0,
                                background_gradient: None,
                                background_radial_gradient: None,
                                background_svg: None,
                                background_blur_radius: 0.0,
                                background_size: BackgroundSize::Auto,
                                background_position: BackgroundPosition::default(),
                                background_repeat: BackgroundRepeat::Repeat,
                                background_origin: BackgroundOrigin::Padding,
                                transform: None,
                                box_shadow: None,
                                nested_elements: item.elements.clone(),
                                y_offset: 0.0,
                                line_cross_size: 0.0,
                            });
                            x += item.width + gap;
                            continue;
                        }

                        for elem in &item.elements {
                            if let LayoutElement::TextBlock {
                                lines: tb_lines,
                                margin_top,
                                background_color: tb_bg,
                                padding_top: tb_pt,
                                padding_bottom: tb_pb,
                                padding_left: tb_pl,
                                padding_right: tb_pr,
                                border_radius: tb_br,
                                ..
                            } = elem
                            {
                                if is_first {
                                    first_bg = *tb_bg;
                                    first_pt = *tb_pt;
                                    first_pb = *tb_pb;
                                    first_pl = *tb_pl;
                                    first_pr = *tb_pr;
                                    first_br = *tb_br;
                                    is_first = false;
                                }
                                // Add margin spacing between sub-elements
                                if !merged_lines.is_empty() && *margin_top > 0.0 {
                                    merged_lines.push(TextLine {
                                        runs: Vec::new(),
                                        height: *margin_top,
                                    });
                                }
                                merged_lines.extend(tb_lines.iter().cloned());
                            }
                        }
                        // Calculate natural height for merged item
                        let natural_h: f32 = merged_lines.iter().map(|l| l.height).sum();
                        flex_cells.push(FlexCell {
                            lines: merged_lines,
                            x_offset: x,
                            width: item.width,
                            natural_height: natural_h,
                            text_align: TextAlign::Left,
                            background_color: first_bg,
                            padding_top: first_pt,
                            padding_right: first_pr,
                            padding_bottom: first_pb,
                            padding_left: first_pl,
                            border: LayoutBorder::default(),
                            border_radius: first_br,
                            background_gradient: None,
                            background_radial_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            transform: None,
                            box_shadow: None,
                            nested_elements: Vec::new(),
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                        });
                        x += item.width + gap;
                        continue;
                    }

                    // Simple items: extract into FlexCell
                    if let Some(LayoutElement::TextBlock {
                        lines: tb_lines,
                        text_align: tb_ta,
                        background_color: tb_bg,
                        padding_top: tb_pt,
                        padding_bottom: tb_pb,
                        padding_left: tb_pl,
                        padding_right: tb_pr,
                        border_radius: tb_br,
                        background_gradient: tb_grad,
                        background_radial_gradient: tb_rgrad,
                        background_svg: tb_bg_svg,
                        background_blur_radius: tb_bg_blur,
                        background_size: tb_bg_size,
                        background_position: tb_bg_pos,
                        background_repeat: tb_bg_repeat,
                        background_origin: tb_bg_origin,
                        box_shadow: tb_bs,
                        border,
                        ..
                    }) = item.elements.first()
                    {
                        // Calculate natural height for this item
                        let text_h: f32 = tb_lines.iter().map(|l| l.height).sum();
                        let natural_h = *tb_pt + text_h + *tb_pb + border.vertical_width();
                        flex_cells.push(FlexCell {
                            lines: tb_lines.clone(),
                            x_offset: x,
                            width: item.width,
                            text_align: *tb_ta,
                            background_color: *tb_bg,
                            padding_top: *tb_pt,
                            padding_right: *tb_pr,
                            padding_bottom: *tb_pb,
                            padding_left: *tb_pl,
                            border: *border,
                            border_radius: *tb_br,
                            background_gradient: tb_grad.clone(),
                            background_radial_gradient: tb_rgrad.clone(),
                            background_svg: tb_bg_svg.clone(),
                            background_blur_radius: *tb_bg_blur,
                            background_size: *tb_bg_size,
                            background_position: *tb_bg_pos,
                            background_repeat: *tb_bg_repeat,
                            background_origin: *tb_bg_origin,
                            transform: None,
                            box_shadow: *tb_bs,
                            nested_elements: Vec::new(),
                            natural_height: natural_h,
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                        });
                    } else {
                        // Single non-TextBlock element (e.g. Container): store
                        // in nested_elements for the renderer to handle.
                        flex_cells.push(FlexCell {
                            lines: Vec::new(),
                            x_offset: x,
                            width: item.width,
                            natural_height: item.height,
                            text_align: TextAlign::Left,
                            background_color: None,
                            padding_top: 0.0,
                            padding_right: 0.0,
                            padding_bottom: 0.0,
                            padding_left: 0.0,
                            border: LayoutBorder::default(),
                            border_radius: 0.0,
                            background_gradient: None,
                            background_radial_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            transform: None,
                            box_shadow: None,
                            nested_elements: item.elements.clone(),
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                        });
                    }

                    x += item.width + gap + extra_gap;
                }

                // Stamp each cell with its cross-axis position within the
                // container so a single FlexRow can span every wrapped line.
                for cell in flex_cells.iter_mut() {
                    cell.y_offset = cross_offset;
                    cell.line_cross_size = line.cross_size;
                }
                all_flex_cells.extend(flex_cells);
            }
            FlexDirection::Column => {
                let _total_item_height: f32 = line_items.iter().map(|&i| items[i].height).sum();
                let _total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let free_space = 0.0f32; // column doesn't constrain main axis to container width
                let _ = free_space;

                let mut y = 0.0;

                for &item_idx in &line_items {
                    let item = &items[item_idx];

                    // Calculate cross-axis (horizontal) alignment
                    let x_offset = match align {
                        AlignItems::FlexStart => 0.0,
                        AlignItems::FlexEnd => inner_width - item.width,
                        AlignItems::Center => (inner_width - item.width) / 2.0,
                        AlignItems::Stretch => 0.0,
                    };

                    let effective_width = if align == AlignItems::Stretch {
                        Some(inner_width)
                    } else {
                        Some(item.width)
                    };

                    for elem in &item.elements {
                        if let LayoutElement::TextBlock {
                            lines: tb_lines,
                            margin_top: tb_mt,
                            margin_bottom: tb_mb,
                            text_align: tb_ta,
                            background_color: tb_bg,
                            padding_top: tb_pt,
                            padding_bottom: tb_pb,
                            padding_left: tb_pl,
                            padding_right: tb_pr,
                            border: tb_border,
                            block_height: tb_bh,
                            opacity: tb_op,
                            position: tb_pos,
                            box_shadow: tb_bs,
                            visible: tb_vis,
                            clip_rect: tb_clip,
                            transform: tb_transform,
                            border_radius: tb_br,
                            outline_width: tb_ow,
                            outline_color: tb_oc,
                            text_indent: tb_ti,
                            letter_spacing: tb_ls,
                            word_spacing: tb_ws,
                            vertical_align: tb_va,
                            background_gradient: tb_grad,
                            background_radial_gradient: tb_rgrad,
                            background_svg: tb_bg_svg,
                            background_blur_radius: tb_bg_blur,
                            background_size: tb_bg_size,
                            background_position: tb_bg_pos,
                            background_repeat: tb_bg_repeat,
                            background_origin: tb_bg_origin,
                            ..
                        } = elem
                        {
                            output.push(LayoutElement::TextBlock {
                                lines: tb_lines.clone(),
                                margin_top: if y == 0.0 && !emitted_column_bg {
                                    style.margin.top + style.padding.top + *tb_mt
                                } else if y == 0.0 {
                                    // Background element already accounts for margin;
                                    // add only the container padding offset.
                                    style.padding.top + *tb_mt
                                } else {
                                    // Apply gap between column-direction flex items.
                                    gap + *tb_mt
                                },
                                margin_bottom: *tb_mb,
                                text_align: *tb_ta,
                                background_color: *tb_bg,
                                padding_top: *tb_pt,
                                padding_bottom: *tb_pb,
                                padding_left: *tb_pl,
                                padding_right: *tb_pr,
                                border: *tb_border,
                                block_width: effective_width,
                                block_height: *tb_bh,
                                opacity: *tb_op,
                                float: Float::None,
                                clear: Clear::None,
                                position: if x_offset > 0.0 || style.padding.left > 0.0 {
                                    Position::Relative
                                } else {
                                    *tb_pos
                                },
                                offset_top: 0.0,
                                offset_left: x_offset + style.padding.left,
                                offset_bottom: 0.0,
                                offset_right: 0.0,
                                containing_block: None,
                                box_shadow: *tb_bs,
                                visible: *tb_vis,
                                clip_rect: *tb_clip,
                                transform: *tb_transform,
                                border_radius: *tb_br,
                                outline_width: *tb_ow,
                                outline_color: *tb_oc,
                                text_indent: *tb_ti,
                                letter_spacing: *tb_ls,
                                word_spacing: *tb_ws,
                                vertical_align: *tb_va,
                                background_gradient: tb_grad.clone(),
                                background_radial_gradient: tb_rgrad.clone(),
                                background_svg: tb_bg_svg.clone(),
                                background_blur_radius: *tb_bg_blur,
                                background_size: *tb_bg_size,
                                background_position: *tb_bg_pos,
                                background_repeat: *tb_bg_repeat,
                                background_origin: *tb_bg_origin,
                                z_index: 0,
                                repeat_on_each_page: false,
                                positioned_depth: 0,
                                heading_level: None,
                                clip_children_count: 0,
                            });
                        }
                    }

                    y += item.height + gap;
                }
            }
        }

        cross_offset += line.cross_size + gap;
    }

    // Emit a single FlexRow carrying every line's cells for row direction.
    // The row's height is the container's inner cross size so pagination and
    // the visual border both include every wrapped line. Each cell's own
    // y_offset and line_cross_size handle per-line alignment internally.
    if direction == FlexDirection::Row && !all_flex_cells.is_empty() {
        let row_height = total_cross.max(inner_cross_size);
        output.push(LayoutElement::FlexRow {
            cells: all_flex_cells,
            row_height,
            margin_top: style.margin.top,
            margin_bottom: 0.0,
            background_color: bg,
            container_width: block_w,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            border_radius: style.border_radius,
            box_shadow: style.box_shadow,
            background_gradient: style.background_gradient.clone(),
            background_radial_gradient: style.background_radial_gradient.clone(),
            background_svg: background_svg_for_style(style),
            background_blur_radius: style.blur_radius,
            background_size: style.background_size,
            background_position: style.background_position,
            background_repeat: style.background_repeat,
            background_origin: style.background_origin,
            align_items: align,
        });
    }

    // Emit trailing margin (include bottom padding when bg spacer shifted y back)
    let trailing = if emitted_column_bg {
        style.padding.bottom + style.margin.bottom
    } else {
        style.margin.bottom
    };
    if trailing > 0.0 {
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: trailing,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
    }
}
