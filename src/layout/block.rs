use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BoxSizing, Clear,
    ComputedStyle, Display, Float, Overflow, Position, TextAlign, TextOverflow, VerticalAlign,
    Visibility, WhiteSpace, compute_style_with_context,
};
use std::collections::HashMap;

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{LayoutBorder, LayoutElement, TextRun, flatten_element};
use super::helpers::{
    BackgroundFields, append_pseudo_inline_run, aspect_ratio_height, build_pseudo_block,
    collects_as_inline_text, has_background_paint, heading_level,
    patch_absolute_children_containing_block, pseudo_is_block_like, push_block_pseudo,
    recurses_as_layout_child, resolve_abs_containing_block, resolve_padding_box_height,
};
use super::inline::{
    element_has_css_display_block, element_is_inline_block, layout_inline_block_group,
};
use super::paginate::estimate_element_height;
use super::text::{
    TextWrapOptions, apply_text_overflow_ellipsis, collect_text_runs, resolved_line_height_factor,
    wrap_text_runs,
};

/// Lay out a `display: block` or `display: inline-block` element.
///
/// Returns `true` when the layout completed via the mixed-block-children
/// early-exit path (page-break-after already emitted), meaning the caller
/// should return immediately without further post-processing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_block_element(
    el: &ElementNode,
    style: &mut ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    child_ancestors: &[AncestorInfo],
    positioned_depth: usize,
    before_style: Option<ComputedStyle>,
    after_style: Option<ComputedStyle>,
    env: &mut LayoutEnv,
) -> bool {
    let available_width = ctx.available_width();
    let available_height = ctx.available_height();
    let abs_containing_block = ctx.containing_block;
    // Compute effective block width considering CSS width/max-width/min-width.
    // Block elements without explicit width shrink by their horizontal margins.
    let margin_h = style.margin.left + style.margin.right;
    let mut block_w = available_width;
    if let Some(w) = style.width {
        // style.width is the resolved width — for percentages this was already
        // computed against the correct layout parent at style time (in
        // particular, flex children pre-resolve percentages against the flex
        // container inner width, which differs from the per-slot
        // `available_width` passed to this block layout). Prefer it over the
        // late-bound `percentage_sizing.width` hint when both are set.
        block_w = w.min(available_width);
    } else if let Some(pct) = style.percentage_sizing.width {
        // Fallback: style.width was not resolved at style time (for example,
        // because the style-time parent width was unknown). Resolve the
        // late-bound percentage against the actual layout parent width.
        block_w = (pct / 100.0 * available_width).min(available_width);
    } else if margin_h > 0.0 {
        block_w = (available_width - margin_h).max(0.0);
    }
    if let Some(pct) = style.percentage_sizing.max_width {
        block_w = block_w.min(pct / 100.0 * available_width);
    } else if let Some(mw) = style.max_width {
        block_w = block_w.min(mw);
    }
    if let Some(pct) = style.percentage_sizing.min_width {
        block_w = block_w.max(pct / 100.0 * available_width);
    } else if let Some(mw) = style.min_width {
        block_w = block_w.max(mw);
    }

    // Compute effective height considering CSS height/min-height/max-height
    let mut effective_height = style.height;
    if effective_height.is_none() {
        if let Some(pct) = style.percentage_sizing.height {
            // Resolve percentage height against containing block if available
            if let Some(cb) = abs_containing_block {
                effective_height = Some(pct / 100.0 * cb.height);
            }
        }
    }
    if let Some(min_h) = style.min_height {
        effective_height = Some(effective_height.map_or(min_h, |h| h.max(min_h)));
    }
    if let Some(max_h) = style.max_height {
        effective_height = effective_height.map(|h| h.min(max_h));
    }

    // Compute margin auto offset for horizontal centering
    let has_explicit_width = style.width.is_some()
        || style.max_width.is_some()
        || style.min_width.is_some()
        || style.percentage_sizing.width.is_some();
    let auto_offset_left = if has_explicit_width && block_w < available_width {
        if style.margin_left_auto && style.margin_right_auto {
            (available_width - block_w) / 2.0
        } else if style.margin_left_auto {
            available_width - block_w
        } else {
            style.margin.left
        }
    } else {
        style.margin.left
    };

    // Adjust for box-sizing: border-box
    // When border-box, the specified width includes padding and border,
    // so the content area is width minus padding and border.
    let inner_width = if style.box_sizing == BoxSizing::BorderBox {
        block_w - style.padding.left - style.padding.right - style.border.horizontal_width()
    } else {
        block_w - style.padding.left - style.padding.right
    };
    let inner_width = inner_width.max(0.0);

    // Resolve percentage border-radius against element dimensions
    if let Some(pct) = style.border_radius_pct {
        let dim = if let Some(h) = effective_height {
            block_w.min(h)
        } else {
            block_w
        };
        style.border_radius = dim * pct / 100.0;
    }

    let style = &*style;

    let ib_ctx = ctx.with_parent(inner_width, ctx.parent.content_height, style.font_size);

    let positioned_container =
        style.position == Position::Relative || style.position == Position::Absolute;
    let make_containing_block = |padding_box_height: f32| {
        if positioned_container {
            let cb_width = if style.box_sizing == BoxSizing::BorderBox {
                block_w - style.border.horizontal_width()
            } else {
                block_w + style.padding.left + style.padding.right
            };
            Some(ContainingBlock {
                x: style.left.unwrap_or(0.0)
                    + auto_offset_left
                    + style.border.left.width
                    + style.padding.left,
                width: cb_width,
                height: padding_box_height,
                depth: positioned_depth,
            })
        } else {
            None
        }
    };

    // Emit block-level ::before pseudo-element.
    let before_is_abs = before_style
        .as_ref()
        .is_some_and(|s| s.position == Position::Absolute);
    let after_is_abs = after_style
        .as_ref()
        .is_some_and(|s| s.position == Position::Absolute);
    if let Some(ref ps) = before_style {
        if pseudo_is_block_like(ps) && !before_is_abs {
            output.push(build_pseudo_block(
                ps,
                el,
                inner_width,
                env.fonts,
                None,
                positioned_depth,
                env.counter_state,
            ));
        }
    }

    // When the element has absolute pseudo-elements, skip inline text
    // collection. The wrapper path will handle all children via
    // flatten_element, avoiding double-rendering of text.
    let skip_inline_collection = positioned_container && (before_is_abs || after_is_abs);

    // Collect inline content as text runs, splitting at math elements.
    // When a math span is encountered, flush accumulated text runs as a
    // TextBlock, emit a MathBlock, then continue collecting.
    let mut runs = Vec::new();
    if !skip_inline_collection {
        append_pseudo_inline_run(
            &mut runs,
            before_style.as_ref(),
            el,
            env.fonts,
            env.counter_state,
        );
    }

    // Helper closure: flush accumulated runs as a TextBlock
    let flush_runs = |runs: &mut Vec<TextRun>,
                      inner_width: f32,
                      style: &ComputedStyle,
                      available_width: f32,
                      block_w: f32,
                      effective_height: Option<f32>,
                      auto_offset_left: f32,
                      el: &ElementNode,
                      output: &mut Vec<LayoutElement>,
                      fonts: &HashMap<String, TtfFont>| {
        if runs.is_empty() {
            return;
        }
        let wrap_width = if style.white_space == WhiteSpace::NoWrap {
            f32::MAX
        } else {
            inner_width
        };
        let lines = wrap_text_runs(
            std::mem::take(runs),
            TextWrapOptions::new(
                wrap_width,
                style.font_size,
                resolved_line_height_factor(style, fonts),
                style.overflow_wrap,
            )
            .with_rtl(style.direction_rtl),
            fonts,
        );
        if lines.is_empty() {
            return;
        }
        // For inline-block without explicit width, shrink-to-fit
        let render_w = if style.display == Display::InlineBlock
            && style.width.is_none()
            && style.percentage_sizing.width.is_none()
        {
            let max_line_w: f32 = lines
                .iter()
                .map(|l| {
                    l.runs
                        .iter()
                        .map(|r| {
                            crate::fonts::str_width(&r.text, r.font_size, &r.font_family, r.bold)
                        })
                        .sum::<f32>()
                })
                .fold(0.0f32, f32::max);
            let shrink_w = max_line_w
                + style.padding.left
                + style.padding.right
                + style.border.horizontal_width();
            shrink_w.min(block_w)
        } else {
            block_w
        };

        let bg = style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgba());
        let explicit_width = if render_w < available_width
            || style.min_width.is_some()
            || style.display == Display::InlineBlock
        {
            Some(render_w)
        } else {
            None
        };
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
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: style.text_align,
            background_color: bg,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: explicit_width,
            block_height: effective_height,
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
            offset_bottom: style.bottom.unwrap_or(0.0),
            offset_right: style.right.unwrap_or(0.0),
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: style.text_indent,
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
            heading_level: heading_level(el.tag),
            clip_children_count: 0,
        });
    };

    // Check if any child is a math element — if so, split at boundaries
    let has_math_children = el.children.iter().any(|c| {
        if let DomNode::Element(child) = c {
            child.attributes.contains_key("data-math")
        } else {
            false
        }
    });

    // Check if this block has visual properties AND block children.
    // When true, inline text is collected separately and block children
    // are processed via the needs_wrapper path for correct nesting.
    let early_has_visual = has_background_paint(style)
        || style.border.has_any()
        || style.border_radius > 0.0
        || style.box_shadow.is_some();
    // Limit nesting depth to prevent stack overflow on deeply nested HTML.
    // Beyond depth 40, fall back to flat text collection instead of Containers.
    let nesting_depth = ancestors.len();
    let has_block_kids_for_wrapper = nesting_depth < 40
        && early_has_visual
        && el.children.iter().any(|c| {
            matches!(c, DomNode::Element(e)
                if (e.tag.is_block() || e.tag == HtmlTag::Svg)
                    && !collects_as_inline_text(e.tag))
        });

    if has_math_children {
        // Split mode: interleave TextBlocks and MathBlocks
        for child in &el.children {
            match child {
                DomNode::Element(child_el) if child_el.attributes.contains_key("data-math") => {
                    // Flush accumulated text runs before math
                    flush_runs(
                        &mut runs,
                        inner_width,
                        style,
                        available_width,
                        block_w,
                        effective_height,
                        auto_offset_left,
                        el,
                        output,
                        env.fonts,
                    );
                    // Emit math block
                    let tex = child_el.attributes.get("data-math").unwrap();
                    let child_classes = child_el.class_list();
                    let is_display = child_classes.contains(&"math-display");
                    let ast = crate::parser::math::parse_math(tex);
                    let math_layout =
                        crate::layout::math::layout_math(&ast, style.font_size, is_display);
                    output.push(LayoutElement::MathBlock {
                        layout: math_layout,
                        display: is_display,
                        margin_top: 0.0,
                        margin_bottom: 0.0,
                    });
                }
                _ => {
                    // Collect text from this child
                    collect_text_runs(
                        std::slice::from_ref(child),
                        style,
                        &mut runs,
                        None,
                        env.rules,
                        env.fonts,
                        child_ancestors,
                    );
                }
            }
        }
        // Flush remaining text runs after math
        flush_runs(
            &mut runs,
            inner_width,
            style,
            available_width,
            block_w,
            effective_height,
            auto_offset_left,
            el,
            output,
            env.fonts,
        );
    } else {
        // Check if children contain block-level elements that have their own
        // margins (e.g. <p>, <h1>-<h6>, <ul>, <ol>, <blockquote>).
        // These need individual layout via flatten_element to preserve
        // their margins. Generic containers (<div>) are not included to
        // avoid expensive recursion on deeply nested structures.
        fn has_own_margins(tag: HtmlTag) -> bool {
            matches!(
                tag,
                HtmlTag::P
                    | HtmlTag::H1
                    | HtmlTag::H2
                    | HtmlTag::H3
                    | HtmlTag::H4
                    | HtmlTag::H5
                    | HtmlTag::H6
                    | HtmlTag::Ul
                    | HtmlTag::Ol
                    | HtmlTag::Li
                    | HtmlTag::Blockquote
                    | HtmlTag::Pre
                    | HtmlTag::Hr
                    | HtmlTag::Dl
                    | HtmlTag::Dt
                    | HtmlTag::Dd
                    | HtmlTag::Figure
                    | HtmlTag::Table
            )
        }
        let parent_has_visual = has_background_paint(style)
            || style.border.has_any()
            || style.border_radius > 0.0
            || style.box_shadow.is_some();
        // Check early if this positioned container has absolute children.
        // When true, skip the has_block_children fast path so we use the
        // Container/wrapper path instead, preserving the containing block.
        let early_has_abs_children = positioned_container
            && el.children.iter().any(|c| {
                if let DomNode::Element(e) = c {
                    // Quick inline style check
                    let s = e.style_attr().unwrap_or("");
                    if s.contains("absolute") {
                        return true;
                    }
                    // Check stylesheet rules
                    let cls = e.class_list();
                    let cls_refs: Vec<&str> = cls.iter().map(|s| s.as_ref()).collect();
                    let cs = compute_style_with_context(
                        e.tag,
                        e.style_attr(),
                        style,
                        env.rules,
                        e.tag_name(),
                        &cls_refs,
                        e.id(),
                        &e.attributes,
                        &SelectorContext::default(),
                    );
                    cs.position == Position::Absolute
                } else {
                    false
                }
            });
        let has_abs_pseudo_early = positioned_container && (before_is_abs || after_is_abs);
        let has_block_children = !parent_has_visual
            && !early_has_abs_children
            && !has_abs_pseudo_early
            && el.children.iter().any(|c| {
                matches!(c, DomNode::Element(e)
                    if (has_own_margins(e.tag)
                        || (e.tag.is_block() && !collects_as_inline_text(e.tag))
                        || element_has_css_display_block(e, style, env.rules, child_ancestors))
                        && !element_is_inline_block(
                            e, style, env.rules, child_ancestors, 0, 0, &[]))
            });

        if skip_inline_collection {
            // All content will be handled by the wrapper path below.
            // Don't collect inline text — the <p> children will be
            // processed via flatten_element in the Container wrapper.
        } else if has_block_children {
            // For visual containers (border, background), emit a wrapper
            // TextBlock first, then a pullback spacer so children render
            // inside the wrapper's padding area.
            let wrapper_output_idx = output.len();
            if parent_has_visual {
                let bg = style
                    .background_color
                    .map(|c: crate::types::Color| c.to_f32_rgba());
                let BackgroundFields {
                    gradient: bg_grad,
                    radial_gradient: bg_rgrad,
                    svg: bg_svg,
                    blur_radius: bg_blur,
                    size: bg_size,
                    position: bg_pos,
                    repeat: bg_repeat,
                    origin: bg_origin,
                } = BackgroundFields::from_style(style);
                // Wrapper height will be patched after children are processed.
                let wrapper_h = effective_height.map_or(0.0, |h| {
                    resolve_padding_box_height(
                        0.0,
                        Some(h),
                        style.padding.top,
                        style.padding.bottom,
                        style.border.vertical_width(),
                        style.box_sizing,
                    )
                });
                output.push(LayoutElement::TextBlock {
                    lines: Vec::new(),
                    margin_top: style.margin.top,
                    margin_bottom: 0.0,
                    text_align: style.text_align,
                    background_color: bg,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    padding_left: style.padding.left,
                    padding_right: style.padding.right,
                    border: LayoutBorder::from_computed(&style.border),
                    block_width: Some(block_w),
                    block_height: effective_height.map(|_| wrapper_h),
                    opacity: style.opacity,
                    float: style.float,
                    clear: style.clear,
                    position: style.position,
                    offset_top: style.top.unwrap_or(0.0),
                    offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
                    offset_bottom: style.bottom.unwrap_or(0.0),
                    offset_right: style.right.unwrap_or(0.0),
                    containing_block: None,
                    clip_children_count: 0,
                    box_shadow: style.box_shadow,
                    visible: style.visibility == Visibility::Visible,
                    clip_rect: if style.overflow == Overflow::Hidden {
                        Some((0.0, 0.0, block_w, wrapper_h))
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
                    background_gradient: bg_grad,
                    background_radial_gradient: bg_rgrad,
                    background_svg: bg_svg,
                    background_blur_radius: bg_blur,
                    background_size: bg_size,
                    background_position: bg_pos,
                    background_repeat: bg_repeat,
                    background_origin: bg_origin,
                    z_index: style.z_index,
                    repeat_on_each_page: false,
                    positioned_depth,
                    heading_level: None,
                });
                // Pullback spacer
                let pullback = if effective_height.is_some() && wrapper_h > 0.0 {
                    wrapper_h - style.padding.top
                } else {
                    0.0
                };
                if pullback > 0.0 {
                    output.push(LayoutElement::TextBlock {
                        lines: Vec::new(),
                        margin_top: -pullback,
                        margin_bottom: 0.0,
                        text_align: TextAlign::Left,
                        background_color: None,
                        padding_top: 0.0,
                        padding_bottom: 0.0,
                        padding_left: style.padding.left,
                        padding_right: style.padding.right,
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
                        clip_children_count: 0,
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
                    });
                }
            }

            // Mixed inline + block children: split at block boundaries.
            let mut block_child_buf: Vec<LayoutElement> = Vec::new();
            let target: &mut Vec<LayoutElement> = if parent_has_visual {
                &mut block_child_buf
            } else {
                output
            };
            for child in &el.children {
                match child {
                    DomNode::Text(_) => {
                        collect_text_runs(
                            std::slice::from_ref(child),
                            style,
                            &mut runs,
                            None,
                            env.rules,
                            env.fonts,
                            child_ancestors,
                        );
                    }
                    DomNode::Element(child_el)
                        if (child_el.tag.is_block()
                            || child_el.tag == HtmlTag::Svg
                            || element_has_css_display_block(
                                child_el,
                                style,
                                env.rules,
                                child_ancestors,
                            ))
                            && !collects_as_inline_text(child_el.tag) =>
                    {
                        // Flush inline runs before block child
                        flush_runs(
                            &mut runs,
                            inner_width,
                            style,
                            available_width,
                            block_w,
                            effective_height,
                            auto_offset_left,
                            el,
                            target,
                            env.fonts,
                        );
                        // Recurse into block child
                        let n_children = el
                            .children
                            .iter()
                            .filter(|c| matches!(c, DomNode::Element(_)))
                            .count();
                        flatten_element(
                            child_el,
                            style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_containing_block(None),
                            target,
                            None,
                            child_ancestors,
                            positioned_depth,
                            0,
                            n_children,
                            &[],
                            env,
                        );
                    }
                    DomNode::Element(_) => {
                        // Inline element: collect as text runs
                        collect_text_runs(
                            std::slice::from_ref(child),
                            style,
                            &mut runs,
                            None,
                            env.rules,
                            env.fonts,
                            child_ancestors,
                        );
                    }
                }
            }
            // Flush remaining inline runs after the last block child
            flush_runs(
                &mut runs,
                inner_width,
                style,
                available_width,
                block_w,
                effective_height,
                auto_offset_left,
                el,
                target,
                env.fonts,
            );
            // For visual containers, propagate parent padding to children
            // so they render inside the padded area.
            if parent_has_visual {
                if style.padding.left > 0.0 || style.padding.right > 0.0 {
                    for elem in &mut block_child_buf {
                        if let LayoutElement::TextBlock {
                            padding_left,
                            padding_right,
                            ..
                        } = elem
                        {
                            *padding_left += style.padding.left;
                            *padding_right += style.padding.right;
                        }
                    }
                }
                output.extend(block_child_buf);

                // Patch wrapper block_height to cover all children
                if effective_height.is_none() {
                    let children_total_h: f32 = output[wrapper_output_idx + 1..]
                        .iter()
                        .map(estimate_element_height)
                        .sum();
                    let patched_h = style.padding.top
                        + children_total_h
                        + style.padding.bottom
                        + style.border.vertical_width();
                    if let Some(LayoutElement::TextBlock { block_height, .. }) =
                        output.get_mut(wrapper_output_idx)
                    {
                        *block_height = Some(patched_h);
                    }
                }
            }
            // Add bottom spacer for visual containers
            if parent_has_visual {
                let bottom_space =
                    style.padding.bottom + style.border.vertical_width() + style.margin.bottom;
                if bottom_space > 0.0 {
                    output.push(LayoutElement::TextBlock {
                        lines: Vec::new(),
                        margin_top: bottom_space,
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
                        clip_children_count: 0,
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
                    });
                }
            }
            // Emit absolute-positioned ::before / ::after pseudo-elements
            if positioned_container && (before_is_abs || after_is_abs) {
                // Compute containing block height from children.
                // Use total element height but strip outer margins of the
                // first/last children — those margins collapse out of the
                // containing block and shouldn't inflate height:100% pseudos.
                let children_slice = &output[wrapper_output_idx..];
                let children_h_raw: f32 = children_slice.iter().map(estimate_element_height).sum();
                let children_h = crate::layout::helpers::collapse_outer_child_margins(
                    children_slice,
                    children_h_raw,
                    style.padding.top,
                    style.padding.bottom,
                    style.border.top.width,
                    style.border.bottom.width,
                );
                let pseudo_cb = Some(ContainingBlock {
                    x: 0.0,
                    width: block_w,
                    height: children_h,
                    depth: positioned_depth,
                });
                if before_is_abs {
                    push_block_pseudo(
                        output,
                        before_style.as_ref(),
                        el,
                        inner_width,
                        env.fonts,
                        pseudo_cb,
                        positioned_depth,
                        env.counter_state,
                    );
                }
                if after_is_abs {
                    push_block_pseudo(
                        output,
                        after_style.as_ref(),
                        el,
                        inner_width,
                        env.fonts,
                        pseudo_cb,
                        positioned_depth,
                        env.counter_state,
                    );
                }
            }

            if style.page_break_after {
                output.push(LayoutElement::PageBreak);
            }
            return true;
        } else if has_block_kids_for_wrapper {
            // Only collect inline children's text — block children will
            // be handled by the needs_wrapper path via flatten_element.
            for child in &el.children {
                match child {
                    DomNode::Text(_) => {
                        collect_text_runs(
                            std::slice::from_ref(child),
                            style,
                            &mut runs,
                            None,
                            env.rules,
                            env.fonts,
                            child_ancestors,
                        );
                    }
                    DomNode::Element(child_el)
                        if collects_as_inline_text(child_el.tag)
                            && !element_has_css_display_block(
                                child_el,
                                style,
                                env.rules,
                                child_ancestors,
                            ) =>
                    {
                        collect_text_runs(
                            std::slice::from_ref(child),
                            style,
                            &mut runs,
                            None,
                            env.rules,
                            env.fonts,
                            child_ancestors,
                        );
                    }
                    _ => {} // Block children handled by needs_wrapper
                }
            }
        } else {
            collect_text_runs(
                &el.children,
                style,
                &mut runs,
                None,
                env.rules,
                env.fonts,
                child_ancestors,
            );
        }
    }
    if !skip_inline_collection {
        append_pseudo_inline_run(
            &mut runs,
            after_style.as_ref(),
            el,
            env.fonts,
            env.counter_state,
        );
    }

    let had_inline_runs = runs.iter().any(|r| !r.text.trim().is_empty()) || has_math_children;
    let mut cb_info = None;

    // has_block_kids_for_wrapper is computed earlier (before has_math_children).
    let mut saved_inline_element: Option<LayoutElement> = None;

    if !runs.is_empty() {
        // When white-space: nowrap, prevent wrapping by using a huge width
        let wrap_width = if style.white_space == WhiteSpace::NoWrap {
            f32::MAX
        } else {
            inner_width
        };
        let mut lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(
                wrap_width,
                style.font_size,
                resolved_line_height_factor(style, env.fonts),
                style.overflow_wrap,
            )
            .with_rtl(style.direction_rtl),
            env.fonts,
        );

        // Apply text-overflow: ellipsis when overflow is hidden, white-space
        // is nowrap, and we have a fixed width.
        if style.text_overflow == TextOverflow::Ellipsis
            && style.overflow == Overflow::Hidden
            && style.white_space == WhiteSpace::NoWrap
            && style.width.is_some()
        {
            apply_text_overflow_ellipsis(&mut lines, inner_width, env.fonts);
        }

        let bg = style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgba());

        let explicit_width = if block_w < available_width || style.min_width.is_some() {
            Some(block_w)
        } else {
            None
        };

        // Compute clip rect — CSS overflow:hidden clips to the padding box
        // (includes padding, excludes border).
        let clip_rect = if style.overflow == Overflow::Hidden {
            let text_height: f32 = lines.iter().map(|l| l.height).sum();
            let padding_box_h = resolve_padding_box_height(
                text_height,
                effective_height,
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            );
            Some((0.0, 0.0, block_w, padding_box_h))
        } else {
            None
        };
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
        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let total_h = resolve_padding_box_height(
            text_height,
            effective_height,
            style.padding.top,
            style.padding.bottom,
            style.border.vertical_width(),
            style.box_sizing,
        );
        cb_info = make_containing_block(total_h);

        // Resolve containing block and offsets for absolute elements
        let (elem_cb, resolved_top, resolved_left) = resolve_abs_containing_block(
            style,
            abs_containing_block,
            total_h,
            explicit_width.unwrap_or(block_w),
        );

        // When this block has visual properties AND block children,
        // save the inline text for inclusion inside the wrapper instead
        // of emitting it directly.  The wrapper path will use it.
        let inline_tb = LayoutElement::TextBlock {
            lines,
            margin_top: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.margin.top
            },
            margin_bottom: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.margin.bottom
            },
            text_align: style.text_align,
            background_color: if has_block_kids_for_wrapper { None } else { bg },
            padding_top: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.padding.top
            },
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: explicit_width,
            block_height: effective_height.map(|_| total_h),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: resolved_top,
            offset_left: resolved_left + auto_offset_left,
            offset_bottom: style.bottom.unwrap_or(0.0),
            offset_right: style.right.unwrap_or(0.0),
            containing_block: elem_cb,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: style.text_indent,
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
            heading_level: heading_level(el.tag),
            clip_children_count: 0,
        };
        // Compute needs_wrapper early so we know whether to push the
        // TextBlock or save it for the Container wrapper path.
        let early_has_visual_for_wrapper = has_background_paint(style)
            || style.border.has_any()
            || style.border_radius > 0.0
            || style.box_shadow.is_some();
        let early_needs_wrapper = early_has_visual_for_wrapper
            || style.aspect_ratio.is_some()
            || style.height.is_some()
            || (positioned_container && (before_is_abs || after_is_abs))
            || skip_inline_collection;
        let early_no_inline = !had_inline_runs;

        if has_block_kids_for_wrapper {
            saved_inline_element = Some(inline_tb);
        } else if early_no_inline && early_needs_wrapper {
            // Don't push empty TextBlock — the wrapper path will
            // create a Container with the correct block_width.
            saved_inline_element = Some(inline_tb);
        } else {
            output.push(inline_tb);
        }
        // Only emit non-absolute before pseudo-elements here.
        // Absolute positioned ::before will be emitted after children processing.
        if !before_is_abs {
            push_block_pseudo(
                output,
                before_style.as_ref(),
                el,
                inner_width,
                env.fonts,
                cb_info,
                positioned_depth,
                env.counter_state,
            );
        }
    }

    // Also process block children recursively, using inner_width
    // so children respect the parent's padding boundaries.
    let child_el_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();

    // If no inline content but the element has visual properties (background,
    // gradient, border, border-radius), emit a wrapper TextBlock so the visuals
    // are rendered.  Children are then pulled back inside via a negative-margin
    // spacer (same technique as flex column containers).
    // NB: check before runs is moved into wrap_text_runs above.
    let has_visual = has_background_paint(style)
        || style.border.has_any()
        || style.border_radius > 0.0
        || style.box_shadow.is_some();
    // A positioned container (position: relative/absolute) needs the
    // Container element to establish a containing block for absolute children.
    let has_abs_children = positioned_container
        && el.children.iter().any(|c| {
            if let DomNode::Element(e) = c {
                let cls = e.class_list();
                let cls_refs: Vec<&str> = cls.iter().map(|s| s.as_ref()).collect();
                let child_style = compute_style_with_context(
                    e.tag,
                    e.style_attr(),
                    style,
                    env.rules,
                    e.tag_name(),
                    &cls_refs,
                    e.id(),
                    &e.attributes,
                    &SelectorContext::default(),
                );
                child_style.position == Position::Absolute
            } else {
                false
            }
        });
    let needs_wrapper = has_visual
        || style.aspect_ratio.is_some()
        || style.height.is_some()
        || (positioned_container && (before_is_abs || after_is_abs))
        || has_abs_children;
    let no_inline_content = !had_inline_runs;

    let has_abs_pseudo = positioned_container && (before_is_abs || after_is_abs);
    if (no_inline_content || has_block_kids_for_wrapper || has_abs_pseudo)
        && needs_wrapper
        && nesting_depth < 40
    {
        // Pre-flatten children to measure total height.
        // If there's saved inline content, include it as the first child.
        let mut child_elements: Vec<LayoutElement> = Vec::new();
        if let Some(inline_el) = saved_inline_element.take() {
            child_elements.push(inline_el);
        }
        let mut child_el_idx = 0;
        let mut ib_group_wrapper: Vec<&ElementNode> = Vec::new();
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if recurses_as_layout_child(child_el.tag)
                    && element_is_inline_block(
                        child_el,
                        style,
                        env.rules,
                        child_ancestors,
                        child_el_idx,
                        child_el_count,
                        &[],
                    )
                {
                    ib_group_wrapper.push(child_el);
                } else {
                    // Flush any pending inline-block group
                    if !ib_group_wrapper.is_empty() {
                        #[allow(clippy::drain_collect)]
                        let taken: Vec<&ElementNode> = ib_group_wrapper.drain(..).collect();
                        layout_inline_block_group(
                            &taken,
                            style,
                            &ib_ctx,
                            &mut child_elements,
                            env.rules,
                            child_ancestors,
                            env.fonts,
                        );
                    }
                    if recurses_as_layout_child(child_el.tag) {
                        let child_cb = if effective_height.is_some() {
                            Some(ContainingBlock {
                                x: 0.0,
                                width: inner_width,
                                height: effective_height.unwrap_or(0.0),
                                depth: positioned_depth,
                            })
                        } else {
                            None
                        };
                        flatten_element(
                            child_el,
                            style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_containing_block(child_cb),
                            &mut child_elements,
                            None,
                            child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &[],
                            env,
                        );
                    }
                }
                child_el_idx += 1;
            }
        }
        // Flush remaining inline-block group
        if !ib_group_wrapper.is_empty() {
            #[allow(clippy::drain_collect)]
            let taken: Vec<&ElementNode> = ib_group_wrapper.drain(..).collect();
            layout_inline_block_group(
                &taken,
                style,
                &ib_ctx,
                &mut child_elements,
                env.rules,
                child_ancestors,
                env.fonts,
            );
        }
        // Measure children total height
        let children_h_raw: f32 = child_elements.iter().map(estimate_element_height).sum();
        let mut container_h = resolve_padding_box_height(
            children_h_raw,
            effective_height,
            style.padding.top,
            style.padding.bottom,
            style.border.vertical_width(),
            style.box_sizing,
        );
        if effective_height.is_none()
            && let Some(aspect_h) = aspect_ratio_height(block_w, style)
        {
            container_h = container_h.max(aspect_h);
        }
        // For pseudo-element containing block sizing (abs children with
        // height:100%), collapse the first/last children's outer margins
        // through the parent when no padding/border blocks them. The
        // rendered container height still uses the raw sum so surrounding
        // flow layout is unchanged.
        let cb_children_h = crate::layout::helpers::collapse_outer_child_margins(
            &child_elements,
            children_h_raw,
            style.padding.top,
            style.padding.bottom,
            style.border.top.width,
            style.border.bottom.width,
        );
        let cb_height = if effective_height.is_some() {
            container_h
        } else {
            cb_children_h.max(aspect_ratio_height(block_w, style).unwrap_or(0.0))
        };
        cb_info = make_containing_block(cb_height);

        // When the first/last child's outer margins collapse through this
        // container (no padding/border blocks them), the containing-block
        // origin used for abs pseudos shifts down by the first child's
        // margin-top so `top:0` aligns with the child's content top — matching
        // Chrome's margin-collapse-through behavior.
        let abs_origin_shift = if effective_height.is_none()
            && style.padding.top == 0.0
            && style.border.top.width == 0.0
        {
            child_elements
                .first()
                .map_or(0.0, crate::layout::helpers::outer_margin_top)
        } else {
            0.0
        };

        // Add absolute-positioned ::before pseudo-element as a Container child.
        if let Some(ref ps) = before_style {
            if pseudo_is_block_like(ps) && ps.position == Position::Absolute {
                let mut pseudo = build_pseudo_block(
                    ps,
                    el,
                    inner_width,
                    env.fonts,
                    cb_info,
                    positioned_depth,
                    env.counter_state,
                );
                if abs_origin_shift > 0.0
                    && let LayoutElement::TextBlock { offset_top, .. } = &mut pseudo
                {
                    *offset_top += abs_origin_shift;
                }
                child_elements.push(pseudo);
            }
        }
        // Add absolute-positioned ::after pseudo-element as a Container child.
        if let Some(ref ps) = after_style {
            if pseudo_is_block_like(ps) && ps.position == Position::Absolute {
                let mut pseudo = build_pseudo_block(
                    ps,
                    el,
                    inner_width,
                    env.fonts,
                    cb_info,
                    positioned_depth,
                    env.counter_state,
                );
                if abs_origin_shift > 0.0
                    && let LayoutElement::TextBlock { offset_top, .. } = &mut pseudo
                {
                    *offset_top += abs_origin_shift;
                }
                child_elements.push(pseudo);
            }
        }

        // Patch absolute children with the now-known containing block,
        // and resolve bottom/right offsets into top/left.
        if let Some(cb) = cb_info {
            patch_absolute_children_containing_block(&mut child_elements, cb);
        }

        let bg = style
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
        } = BackgroundFields::from_style(style);
        // Resolve containing block and offsets for absolute elements
        let (_wrapper_cb, wrapper_top, wrapper_left) =
            resolve_abs_containing_block(style, abs_containing_block, container_h, block_w);
        // Emit a Container element with true parent-child nesting.
        // The renderer draws background/border, then renders children inside.
        output.push(LayoutElement::Container {
            children: child_elements,
            background_color: bg,
            border: LayoutBorder::from_computed(&style.border),
            border_radius: style.border_radius,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            block_width: Some(block_w),
            block_height: if effective_height.is_some() || style.aspect_ratio.is_some() {
                Some(container_h)
            } else {
                None
            },
            opacity: style.opacity,
            float: style.float,
            position: style.position,
            offset_top: wrapper_top,
            offset_left: wrapper_left + auto_offset_left,
            overflow: style.overflow,
            transform: style.transform,
            box_shadow: style.box_shadow,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: style.z_index,
        });
    } else {
        if no_inline_content {
            push_block_pseudo(
                output,
                before_style.as_ref(),
                el,
                inner_width,
                env.fonts,
                cb_info,
                positioned_depth,
                env.counter_state,
            );
        }
        // Compute cb_info for positioned containers in the non-wrapper path
        // so that absolute children get a containing block.
        if cb_info.is_none() && positioned_container {
            let h = effective_height.unwrap_or(0.0);
            cb_info = make_containing_block(h);
        }
        let mut child_el_idx = 0;
        let mut ib_group: Vec<&ElementNode> = Vec::new();
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if recurses_as_layout_child(child_el.tag)
                    && element_is_inline_block(
                        child_el,
                        style,
                        env.rules,
                        child_ancestors,
                        child_el_idx,
                        child_el_count,
                        &[],
                    )
                {
                    ib_group.push(child_el);
                } else {
                    // Flush any pending inline-block group
                    if !ib_group.is_empty() {
                        #[allow(clippy::drain_collect)]
                        let taken: Vec<&ElementNode> = ib_group.drain(..).collect();
                        layout_inline_block_group(
                            &taken,
                            style,
                            &ib_ctx,
                            output,
                            env.rules,
                            child_ancestors,
                            env.fonts,
                        );
                    }
                    if recurses_as_layout_child(child_el.tag) {
                        flatten_element(
                            child_el,
                            style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_containing_block(cb_info),
                            output,
                            None,
                            child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &[],
                            env,
                        );
                    }
                }
                child_el_idx += 1;
            }
        }
        // Flush remaining inline-block group
        if !ib_group.is_empty() {
            #[allow(clippy::drain_collect)]
            let taken: Vec<&ElementNode> = ib_group.drain(..).collect();
            layout_inline_block_group(
                &taken,
                style,
                &ib_ctx,
                output,
                env.rules,
                child_ancestors,
                env.fonts,
            );
        }
    }

    // Emit block-level ::after pseudo-element (inside block path)
    push_block_pseudo(
        output,
        after_style.as_ref(),
        el,
        inner_width,
        env.fonts,
        cb_info,
        positioned_depth,
        env.counter_state,
    );
    false
}
