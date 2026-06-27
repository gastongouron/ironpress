use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BackgroundClip, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize,
    BoxSizing, Clear, ComputedStyle, Display, Float, Position, TextAlign, TextOverflow,
    VerticalAlign, Visibility, WhiteSpace, compute_style_with_context,
};
use std::collections::HashMap;

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    LayoutBorder, LayoutElement, PageBreakSide, TextRun, element_sibling_list, flatten_element,
    forward_siblings,
};
use super::helpers::{
    BackgroundFields, append_pseudo_inline_run, aspect_ratio_height, build_pseudo_block,
    collects_as_inline_text, has_background_paint, heading_level,
    patch_absolute_children_containing_block, pseudo_is_block_like, push_block_pseudo,
    recurses_as_layout_child, resolve_abs_containing_block, resolve_content_box_height,
    resolve_inset, resolve_padding_box_height,
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
    first_line_style: Option<ComputedStyle>,
    first_letter_style: Option<ComputedStyle>,
    env: &mut LayoutEnv,
) -> bool {
    let output_start_len = output.len();
    let available_width = ctx.available_width();
    let available_height = ctx.available_height();
    // Basis for percentage `width`/`min-width`/`max-width` (CSS 2.1 § 10.2):
    // the containing block's content width. For normal block flow this equals
    // `available_width`; flex layout hands an item its own resolved width as
    // `available_width` but keeps the container content width as the basis.
    let percent_width_basis = ctx.parent.percent_width_basis;
    let abs_containing_block = ctx.containing_block;
    // Percentage `height` resolves against the parent's content box (CSS 2.1
    // § 10.5), tracked separately from the absolute containing block so the two
    // are not conflated when a child sits inside a static element.
    let percent_height_cb = ctx.percent_height_cb.or(abs_containing_block);
    // Compute effective block width considering CSS width/max-width/min-width.
    // Block elements without explicit width shrink by their horizontal margins.
    let margin_h = style.margin.left + style.margin.right;
    // For `box-sizing: content-box` the declared width is the *content* width,
    // so the outer (border-box) width that `block_w` represents is the declared
    // width plus horizontal padding and border. For `border-box` the declared
    // width already is the border-box width.
    let content_box_extra = if style.box_sizing == BoxSizing::ContentBox {
        style.padding.left + style.padding.right + style.border.horizontal_width()
    } else {
        0.0
    };
    let mut block_w = available_width;
    if let Some(w) = style.width {
        // style.width is the resolved width — for percentages this was already
        // computed against the correct layout parent at style time (in
        // particular, flex children pre-resolve percentages against the flex
        // container inner width, which differs from the per-slot
        // `available_width` passed to this block layout). Prefer it over the
        // late-bound `percentage_sizing.width` hint when both are set.
        //
        // A definite length width (`width: 250px`) is honoured exactly and the
        // box overflows its parent when wider — CSS does not shrink it to fit
        // (that is what `overflow` is for). Only percentage/auto widths clamp to
        // the available width. `percentage_sizing.width` is set when the width
        // came from a `%`, so a pure length has it as `None`.
        block_w = if let Some(pct) = style.percentage_sizing.width {
            // A percentage width resolves against the containing block's
            // *content* width (CSS 2.1 § 10.2). The style cascade pre-resolved
            // `w` against the parent's declared/border-box width, which for a
            // `box-sizing: border-box` (or padded) parent is wider than its
            // content box — recompute from the true basis so e.g. `width: 50%`
            // inside a 400px border-box (396px content) box is 198px, not 200px.
            (pct / 100.0 * percent_width_basis + content_box_extra).min(available_width)
        } else {
            // A definite length width is honoured exactly (overflows when wider).
            w + content_box_extra
        };
    } else if let Some(pct) = style.percentage_sizing.width {
        // Fallback: style.width was not resolved at style time (for example,
        // because the style-time parent width was unknown). Resolve the
        // late-bound percentage against the containing block content width.
        block_w = (pct / 100.0 * percent_width_basis + content_box_extra).min(available_width);
    } else if let Some(keyword) = style.width_keyword {
        // css-sizing-3 § 5.1 intrinsic-sizing keyword (`min-content` /
        // `max-content` / `fit-content`). Size the box from its content rather
        // than filling the available width. `resolve_intrinsic_keyword_width`
        // returns the border-box width (it already adds this box's padding and
        // border, respecting box-sizing) and, for `fit-content`, clamps the
        // stretch-fit term to the available content width less margins. This path
        // is only taken when `width` is `None` and there is no `%` width, so it
        // never perturbs the normal length/percentage/auto behaviour.
        block_w = crate::layout::helpers::resolve_intrinsic_keyword_width(
            el,
            style,
            keyword,
            available_width,
            env.rules,
            env.fonts,
        );
    } else if margin_h > 0.0 {
        block_w = (available_width - margin_h).max(0.0);
    }
    // CSS 2.1 § 10.4: percentage min-/max-width also resolve against the
    // containing block content width. Min wins over max (the floor is applied
    // last) per css-sizing-3 — `max(min, min(value, max))`.
    if let Some(pct) = style.percentage_sizing.max_width {
        block_w = block_w.min(pct / 100.0 * percent_width_basis);
    } else if let Some(mw) = style.max_width {
        block_w = block_w.min(mw);
    }
    if let Some(pct) = style.percentage_sizing.min_width {
        block_w = block_w.max(pct / 100.0 * percent_width_basis);
    } else if let Some(mw) = style.min_width {
        block_w = block_w.max(mw);
    }

    // css-sizing-3 § 5.1: under `box-sizing: border-box` the content width is the
    // declared width minus padding+border, floored at zero. When padding+border
    // exceed the declared border-box width, the content cannot be negative, so
    // the rendered border box grows to the padding+border sum (the box can never
    // be narrower than its own padding and border). `content-box` already keeps
    // padding/border outside `block_w` so this floor is a no-op there.
    if style.box_sizing == BoxSizing::BorderBox {
        let padding_border_w =
            style.padding.left + style.padding.right + style.border.horizontal_width();
        block_w = block_w.max(padding_border_w);
    }

    // CSS 2.1 § 10.3.7 over-constrained absolute width: when `width: auto` and
    // BOTH `left` and `right` are set, the box stretches to fill the containing
    // block, inset by left/right (and horizontal margins). `block_w` is the
    // border-box width, so the stretched border-box width is
    // `cb.width - left - right - margin_h` (padding/border are inside it).
    if style.position == Position::Absolute
        && style.width.is_none()
        && style.percentage_sizing.width.is_none()
        && style.max_width.is_none()
        && style.min_width.is_none()
        && let Some(cb) = abs_containing_block
    {
        let left = resolve_inset(style.left, style.percentage_insets.left, cb.width);
        let right = resolve_inset(style.right, style.percentage_insets.right, cb.width);
        if let (Some(left), Some(right)) = (left, right) {
            block_w = (cb.width - left - right - margin_h).max(0.0);
        }
    }

    // Compute effective height considering CSS height/min-height/max-height
    let mut effective_height = style.height;
    // CSS over-constrained absolute height: `height: auto` with BOTH `top` and
    // `bottom` set stretches the box to fill the containing block, inset by
    // top/bottom. Resolve to the border-box height (`cb.height` is the padding
    // box). Treated as definite so content does not re-expand it.
    if effective_height.is_none()
        && style.position == Position::Absolute
        && style.percentage_sizing.height.is_none()
        && let Some(cb) = abs_containing_block
    {
        let top = resolve_inset(style.top, style.percentage_insets.top, cb.height);
        let bottom = resolve_inset(style.bottom, style.percentage_insets.bottom, cb.height);
        if let (Some(top), Some(bottom)) = (top, bottom) {
            let margin_v = style.margin.top + style.margin.bottom;
            effective_height = Some((cb.height - top - bottom - margin_v).max(0.0));
        }
    }
    if effective_height.is_none() {
        if let Some(pct) = style.percentage_sizing.height {
            // An absolute box's percentage height resolves against its absolute
            // containing block (the positioned ancestor's padding box); an
            // in-flow box's against the parent's content box (CSS 2.1 § 10.5).
            let height_cb = if style.position == Position::Absolute {
                abs_containing_block
            } else {
                percent_height_cb
            };
            if let Some(cb) = height_cb {
                effective_height = Some(pct / 100.0 * cb.height);
            }
        }
    }
    // A *definite* height (`height` / resolvable `height: %`) is a hard size: per
    // CSS, oversized content overflows the box rather than growing it. A
    // `min-height` floor (with no definite height) is NOT definite — the box
    // still grows to fit taller content. Track which case `effective_height`
    // came from so the box height is clamped only for the definite case.
    let has_definite_height = effective_height.is_some();
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

    // `block_w` is now the border-box (outer) width for both box-sizing modes
    // (content-box added padding+border above), so the content area is always
    // the outer width minus horizontal padding and border.
    let inner_width =
        block_w - style.padding.left - style.padding.right - style.border.horizontal_width();
    let inner_width = inner_width.max(0.0);

    // Resolve percentage border-radius. Per CSS Backgrounds §5.1 a horizontal
    // radius percentage resolves against the border-box WIDTH and a vertical one
    // against its HEIGHT, giving elliptical corners on a non-square box. The
    // legacy uniform `border_radius` field stays circular (smaller dimension) for
    // code paths that carry only one radius.
    let height_dim = effective_height.unwrap_or(block_w);
    let radius_dim = block_w.min(height_dim);
    if let Some(pct) = style.border_radius_pct {
        style.border_radius = radius_dim * pct / 100.0;
    }
    // Resolve per-corner radii: turn any percentage corner into an absolute
    // radius (horizontal against width, vertical against height), and seed
    // all-zero radii from the (resolved) uniform value so the renderer's
    // per-corner path matches the uniform path for simple boxes.
    for i in 0..4 {
        if let Some(pct) = style.border_radii_pct[i] {
            style.border_radii[i] = block_w * pct / 100.0;
        }
        if let Some(pct) = style.border_radii_y_pct[i] {
            style.border_radii_y[i] = height_dim * pct / 100.0;
        }
    }
    if style.border_radii.iter().all(|r| *r == 0.0) && style.border_radius > 0.0 {
        style.border_radii = [style.border_radius; 4];
    }
    // Seed vertical radii from the horizontal ones for circular corners (no
    // distinct `/`-group or percentage was given on the vertical axis).
    if style.border_radii_y.iter().all(|r| *r == 0.0) {
        style.border_radii_y = style.border_radii;
    }

    let style = &*style;

    // Parent style handed to block children for *their* percentage-height
    // resolution (CSS 2.1 § 10.5). A child's `height: %` resolves against this
    // box's CONTENT-box height, but `style.height` here is the declared height
    // (the border-box height under `box-sizing: border-box`). When this box has
    // a definite height, hand children a clone whose `.height` is the content
    // box (declared height minus this box's own padding and border) so their
    // `height: 100%` fits inside rather than inflating the parent. Only built
    // when a definite height exists and differs from the content box — otherwise
    // children just see the original `style`.
    let child_parent_owned: Option<ComputedStyle> = effective_height.and_then(|h| {
        let content_h = resolve_content_box_height(
            h,
            style.padding.top,
            style.padding.bottom,
            style.border.vertical_width(),
            style.box_sizing,
        );
        if (content_h - h).abs() < f32::EPSILON {
            None
        } else {
            let mut adjusted = style.clone();
            adjusted.height = Some(content_h);
            Some(adjusted)
        }
    });
    let child_parent_style: &ComputedStyle = child_parent_owned.as_ref().unwrap_or(style);

    let ib_ctx = ctx.with_parent(inner_width, ctx.parent.content_height, style.font_size);

    // An element establishes a containing block for absolute descendants when it
    // is positioned OR carries a `transform` (CSS Transforms § 3). This makes a
    // transformed non-positioned ancestor act as the CB for its absolute kids.
    let positioned_container = crate::layout::helpers::establishes_containing_block(style);
    let make_containing_block = |padding_box_height: f32| {
        if positioned_container {
            // `block_w` is the border-box width for both box-sizing modes, so the
            // padding box (containing block for absolute children) is the
            // border-box width minus horizontal border.
            let cb_width = block_w - style.border.horizontal_width();
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

    // Absolute containing block to forward to this element's descendants.
    //
    // A `position: static` element does NOT establish a containing block, so it
    // forwards the inherited `abs_containing_block` unchanged — this is what lets
    // an absolute box skip static intermediate ancestors and resolve against the
    // nearest *positioned* ancestor (CSS 2.1 § 10.1). A positioned element
    // replaces it with its own padding box. Direct absolute children are later
    // re-patched with the finalized containing block (`patch_absolute_children_…`)
    // once the box height is known; this forwarded value carries the correct
    // origin x and depth to deeper descendants nested inside static intermediates.
    //
    // The forwarded CB height is the box's definite height when known. For an
    // auto-height positioned ancestor it is not yet measured at descent time, so
    // it falls back to 0 — only relevant to a DEEP `bottom`/`right`-anchored
    // descendant nested inside static intermediates (a direct child is re-patched
    // with the real height). top/left descendants (the common case) are exact.
    let forward_abs_cb = if positioned_container {
        let cb_padding_box_h = effective_height.map_or(0.0, |h| {
            resolve_content_box_height(
                h,
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            ) + style.padding.top
                + style.padding.bottom
        });
        make_containing_block(cb_padding_box_h)
    } else {
        abs_containing_block
    };

    // Emit block-level ::before pseudo-element.
    let before_is_abs = before_style
        .as_ref()
        .is_some_and(|s| s.position == Position::Absolute);
    let after_is_abs = after_style
        .as_ref()
        .is_some_and(|s| s.position == Position::Absolute);
    // A non-absolute block-level `::before`/`::after` (e.g.
    // `.card::before { content: "HEADER"; display: block }`) is an in-flow
    // block-level child of the originating element: it must be laid out INSIDE
    // the element's content box as the first/last block, not as a sibling
    // before/after it (css-content-3 §1, css-display-3). It therefore forces the
    // Container wrapper path just like a real block child does.
    let has_block_before = before_style
        .as_ref()
        .is_some_and(|s| pseudo_is_block_like(s) && s.position != Position::Absolute);
    let has_block_after = after_style
        .as_ref()
        .is_some_and(|s| pseudo_is_block_like(s) && s.position != Position::Absolute);
    let has_inflow_block_pseudo = has_block_before || has_block_after;
    // `early_has_visual`/`nesting_depth` are needed both here (to decide whether
    // a block pseudo routes through the Container wrapper) and later (to gate the
    // wrapper itself), so compute them once up front.
    let early_has_visual = has_background_paint(style)
        || style.border.has_any()
        || style.border_radius > 0.0
        || !style.box_shadow.is_empty();
    let nesting_depth = ancestors.len();
    // A block-level `::before` is normally emitted here as the first in-flow
    // block. But when this element takes the Container wrapper path (it has
    // visual box decoration AND in-flow block content), the pseudo must instead
    // be nested INSIDE the wrapper as its first child — handled below — so it
    // sits within the element's padding box rather than as a preceding sibling.
    let has_block_kids_for_wrapper = nesting_depth < 40
        && early_has_visual
        && (has_inflow_block_pseudo
            || el.children.iter().any(|c| {
                matches!(c, DomNode::Element(e)
                if (e.tag.is_block() || e.tag == HtmlTag::Svg)
                    && !collects_as_inline_text(e.tag))
            }));
    let block_pseudo_via_wrapper = has_inflow_block_pseudo && has_block_kids_for_wrapper;
    if let Some(ref ps) = before_style {
        if pseudo_is_block_like(ps) && !before_is_abs && !block_pseudo_via_wrapper {
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
            .with_rtl(style.direction_rtl)
            .with_bidi_override(style.bidi_override)
            .with_text_indent(style.text_indent),
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
            conic_gradient: background_conic_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
            clip: background_clip,
        } = BackgroundFields::from_style(style);
        output.push(LayoutElement::TextBlock {
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: style.text_align,
            writing_mode: style.writing_mode,
            background_color: bg,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: explicit_width,
            block_height: effective_height,
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            background_blend_mode: style.background_blend_mode,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
            offset_bottom: style.bottom.unwrap_or(0.0),
            offset_right: style.right.unwrap_or(0.0),
            containing_block: None,
            box_shadow: style.box_shadow.clone(),
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            transform_origin: style.transform_origin,
            border_radius: style.border_radius,
            border_radii: style.border_radii,
            border_radii_y: style.border_radii_y,
            outline_offset: style.outline_offset,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: style.text_indent,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_conic_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            background_clip,
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
                        env.counter_state,
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
            || !style.box_shadow.is_empty();
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
        // A non-visual block whose padding offsets its children cannot use the
        // flat fast path: that path discards parent padding (it only propagates
        // it for visual containers, lines ~727-740). Route padded blocks through
        // the Container/wrapper path below, which applies the content-box origin
        // (padding + border) to every child type. Without this, a `display:flex`,
        // positioned, or block child of e.g. `<div style="padding:20px">` renders
        // at the parent's border-box origin (padding silently dropped).
        let has_padding_offset = style.padding.left > 0.0
            || style.padding.top > 0.0
            || style.padding.right > 0.0
            || style.padding.bottom > 0.0;
        let has_block_children = !parent_has_visual
            && !has_padding_offset
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
                    conic_gradient: bg_cgrad,
                    svg: bg_svg,
                    blur_radius: bg_blur,
                    size: bg_size,
                    position: bg_pos,
                    repeat: bg_repeat,
                    origin: bg_origin,
                    clip: bg_clip,
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
                    writing_mode: crate::style::computed::WritingMode::HorizontalTb,
                    background_color: bg,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    padding_left: style.padding.left,
                    padding_right: style.padding.right,
                    border: LayoutBorder::from_computed(&style.border),
                    block_width: Some(block_w),
                    block_height: effective_height.map(|_| wrapper_h),
                    opacity: style.opacity,
                    mix_blend_mode: style.mix_blend_mode,
                    background_blend_mode: style.background_blend_mode,
                    float: style.float,
                    clear: style.clear,
                    position: style.position,
                    offset_top: style.top.unwrap_or(0.0),
                    offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
                    offset_bottom: style.bottom.unwrap_or(0.0),
                    offset_right: style.right.unwrap_or(0.0),
                    containing_block: None,
                    clip_children_count: 0,
                    box_shadow: style.box_shadow.clone(),
                    visible: style.visibility == Visibility::Visible,
                    clip_rect: if style.overflow.clips() {
                        Some((0.0, 0.0, block_w, wrapper_h))
                    } else {
                        None
                    },
                    transform: style.transform,
                    transform_origin: style.transform_origin,
                    border_radius: style.border_radius,
                    border_radii: style.border_radii,
                    border_radii_y: style.border_radii_y,
                    outline_offset: style.outline_offset,
                    outline_width: style.outline_width,
                    outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                    text_indent: 0.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    vertical_align: VerticalAlign::Baseline,
                    background_gradient: bg_grad,
                    background_radial_gradient: bg_rgrad,
                    background_conic_gradient: bg_cgrad,
                    background_svg: bg_svg,
                    background_blur_radius: bg_blur,
                    background_size: bg_size,
                    background_position: bg_pos,
                    background_repeat: bg_repeat,
                    background_origin: bg_origin,
                    background_clip: bg_clip,
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
                        writing_mode: crate::style::computed::WritingMode::HorizontalTb,
                        background_color: None,
                        padding_top: 0.0,
                        padding_bottom: 0.0,
                        padding_left: style.padding.left,
                        padding_right: style.padding.right,
                        border: LayoutBorder::default(),
                        block_width: None,
                        block_height: None,
                        opacity: 1.0,
                        mix_blend_mode: crate::style::computed::BlendMode::Normal,
                        background_blend_mode: crate::style::computed::BlendMode::Normal,
                        float: Float::None,
                        clear: Clear::None,
                        position: Position::Static,
                        offset_top: 0.0,
                        offset_left: 0.0,
                        offset_bottom: 0.0,
                        offset_right: 0.0,
                        containing_block: None,
                        clip_children_count: 0,
                        box_shadow: Vec::new(),
                        visible: true,
                        clip_rect: None,
                        transform: None,
                        transform_origin: crate::style::computed::TransformOrigin::default(),
                        border_radius: 0.0,
                        border_radii: [0.0; 4],
                        border_radii_y: [0.0; 4],
                        outline_offset: 0.0,
                        outline_width: 0.0,
                        outline_color: None,
                        text_indent: 0.0,
                        letter_spacing: 0.0,
                        word_spacing: 0.0,
                        vertical_align: VerticalAlign::Baseline,
                        background_gradient: None,
                        background_radial_gradient: None,
                        background_conic_gradient: None,
                        background_svg: None,
                        background_blur_radius: 0.0,
                        background_size: BackgroundSize::Auto,
                        background_position: BackgroundPosition::default(),
                        background_repeat: BackgroundRepeat::Repeat,
                        background_origin: BackgroundOrigin::Padding,
                        background_clip: BackgroundClip::Border,
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
                            env.counter_state,
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
                            child_parent_style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_containing_block(None),
                            target,
                            None,
                            child_ancestors,
                            positioned_depth,
                            0,
                            n_children,
                            &[],
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
                            env.counter_state,
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
                        writing_mode: crate::style::computed::WritingMode::HorizontalTb,
                        background_color: None,
                        padding_top: 0.0,
                        padding_bottom: 0.0,
                        padding_left: 0.0,
                        padding_right: 0.0,
                        border: LayoutBorder::default(),
                        block_width: None,
                        block_height: None,
                        opacity: 1.0,
                        mix_blend_mode: crate::style::computed::BlendMode::Normal,
                        background_blend_mode: crate::style::computed::BlendMode::Normal,
                        float: Float::None,
                        clear: Clear::None,
                        position: Position::Static,
                        offset_top: 0.0,
                        offset_left: 0.0,
                        offset_bottom: 0.0,
                        offset_right: 0.0,
                        containing_block: None,
                        clip_children_count: 0,
                        box_shadow: Vec::new(),
                        visible: true,
                        clip_rect: None,
                        transform: None,
                        transform_origin: crate::style::computed::TransformOrigin::default(),
                        border_radius: 0.0,
                        border_radii: [0.0; 4],
                        border_radii_y: [0.0; 4],
                        outline_offset: 0.0,
                        outline_width: 0.0,
                        outline_color: None,
                        text_indent: 0.0,
                        letter_spacing: 0.0,
                        word_spacing: 0.0,
                        vertical_align: VerticalAlign::Baseline,
                        background_gradient: None,
                        background_radial_gradient: None,
                        background_conic_gradient: None,
                        background_svg: None,
                        background_blur_radius: 0.0,
                        background_size: BackgroundSize::Auto,
                        background_position: BackgroundPosition::default(),
                        background_repeat: BackgroundRepeat::Repeat,
                        background_origin: BackgroundOrigin::Padding,
                        background_clip: BackgroundClip::Border,
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
                output.push(LayoutElement::PageBreak(PageBreakSide::from(
                    style.break_after,
                )));
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
                            env.counter_state,
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
                            env.counter_state,
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
                env.counter_state,
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

    // `::first-letter` (css-pseudo-4 §2.2): split off and restyle the first
    // typographic letter unit before line breaking so its (possibly larger)
    // glyph participates in wrapping. A `float: left` first-letter becomes a drop
    // cap and returns its float-exclusion geometry, applied to the wrapped lines
    // below.
    let mut drop_cap: Option<crate::layout::helpers::DropCap> = None;
    if let Some(ref fl) = first_letter_style {
        let block_line_height = style.font_size * resolved_line_height_factor(style, env.fonts);
        drop_cap = crate::layout::helpers::apply_first_letter_style(
            &mut runs,
            fl,
            env.fonts,
            block_line_height,
        );
    }

    let had_text_runs = runs.iter().any(|r| !r.text.trim().is_empty());
    let has_inline_box_runs = runs.iter().any(|r| r.inline_box.is_some());
    // Inline-block boxes that sit *amongst text* are part of the line and are
    // laid out by the inline TextBlock path; the container must then NOT also
    // re-run `layout_inline_block_group`. But a *visual* container whose children
    // are only inline-blocks (no text) keeps the dedicated group path, which
    // measures shrink-to-fit rows inside the wrapper. When there is no wrapper
    // (a plain non-visual block), the group path never fires, so the inline-box
    // runs must stay and render as a TextBlock.
    let will_use_group_wrapper = has_inline_box_runs
        && !had_text_runs
        && (early_has_visual
            || style.height.is_some()
            || style.aspect_ratio.is_some()
            || style.padding.left > 0.0
            || style.padding.top > 0.0
            || style.padding.right > 0.0
            || style.padding.bottom > 0.0)
        && nesting_depth < 40;
    let had_inline_runs =
        had_text_runs || (has_inline_box_runs && !will_use_group_wrapper) || has_math_children;
    if will_use_group_wrapper {
        // Pure inline-block group inside a wrapper: drop the placeholder runs so
        // `layout_inline_block_group` lays them out (unchanged behaviour).
        runs.clear();
    }
    let mut cb_info = None;

    // has_block_kids_for_wrapper is computed earlier (before has_math_children).
    let mut saved_inline_element: Option<LayoutElement> = None;

    if !runs.is_empty() {
        // `white-space: nowrap` and `pre` never soft-wrap: render with an
        // unbounded width so only explicit newlines break lines. `pre-wrap`
        // keeps spaces but still wraps at the box edge.
        let wrap_width = if matches!(style.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre) {
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
            .with_rtl(style.direction_rtl)
            .with_bidi_override(style.bidi_override)
            .with_pre_wrap(matches!(
                style.white_space,
                WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
            ))
            .with_break_spaces(style.white_space == WhiteSpace::BreakSpaces)
            // text-indent shortens the FIRST formatted line, so the wrapper must
            // reserve that space before breaking — otherwise the first line packs
            // full-width text that then overflows once shifted at paint time
            // (css-text-3 §8).
            .with_text_indent(style.text_indent)
            // A `::first-letter { float: left }` drop cap reserves a left
            // exclusion on the lines it overlaps (css-pseudo-4 §2.2 + css2 §9.5).
            .with_drop_cap(
                drop_cap.map_or(0.0, |d| d.width),
                drop_cap.map_or(0, |d| d.span_lines),
            ),
            env.fonts,
        );

        // `::first-line` (css-pseudo-4 §2.1): restyle the runs that landed on
        // the dynamically-determined first formatted line.
        if let Some(ref fl) = first_line_style {
            crate::layout::helpers::apply_first_line_style(&mut lines, fl, env.fonts);
        }

        // Apply text-overflow: ellipsis when overflow is hidden, white-space
        // is nowrap, and we have a fixed width.
        if style.text_overflow == TextOverflow::Ellipsis
            && style.overflow.clips()
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
        let clip_rect = if style.overflow.clips() {
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
            conic_gradient: background_conic_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
            clip: background_clip,
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

        // Resolve containing block and offsets for absolute elements.
        // `resolve_abs_containing_block` measures bottom/right insets to the box's
        // border-box edge (`cb.height - elem_height - bottom`), so pass the
        // *border-box* height/width — `total_h` is the padding box, so add the
        // vertical border back (width `block_w` is already border-box).
        let (elem_cb, resolved_top, resolved_left) = resolve_abs_containing_block(
            style,
            abs_containing_block,
            total_h + style.border.vertical_width(),
            explicit_width.unwrap_or(block_w),
        );

        // When this block has visual properties AND block children,
        // save the inline text for inclusion inside the wrapper instead
        // of emitting it directly.  The wrapper path will use it. In that case
        // the inline text becomes an anonymous block-level box *inside* the
        // wrapper: the wrapper (Container) paints the element's background,
        // border, padding and offsets, so this inner box must carry none of them
        // (otherwise the border/background/indent would be drawn twice — once on
        // the Container and again around the inline text).
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
            writing_mode: style.writing_mode,
            background_color: if has_block_kids_for_wrapper { None } else { bg },
            padding_top: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.padding.top
            },
            padding_bottom: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.padding.bottom
            },
            padding_left: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.padding.left
            },
            padding_right: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.padding.right
            },
            border: if has_block_kids_for_wrapper {
                LayoutBorder::default()
            } else {
                LayoutBorder::from_computed(&style.border)
            },
            block_width: if has_block_kids_for_wrapper {
                None
            } else {
                explicit_width
            },
            block_height: effective_height.map(|_| total_h),
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            background_blend_mode: style.background_blend_mode,
            float: if has_block_kids_for_wrapper {
                Float::None
            } else {
                style.float
            },
            clear: style.clear,
            position: if has_block_kids_for_wrapper {
                Position::Static
            } else {
                style.position
            },
            offset_top: if has_block_kids_for_wrapper {
                0.0
            } else {
                resolved_top
            },
            offset_left: if has_block_kids_for_wrapper {
                0.0
            } else {
                resolved_left + auto_offset_left
            },
            offset_bottom: style.bottom.unwrap_or(0.0),
            offset_right: style.right.unwrap_or(0.0),
            containing_block: elem_cb,
            box_shadow: if has_block_kids_for_wrapper {
                Vec::new()
            } else {
                style.box_shadow.clone()
            },
            visible: style.visibility == Visibility::Visible,
            clip_rect: if has_block_kids_for_wrapper {
                None
            } else {
                clip_rect
            },
            transform: if has_block_kids_for_wrapper {
                None
            } else {
                style.transform
            },
            transform_origin: style.transform_origin,
            border_radius: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.border_radius
            },
            border_radii: if has_block_kids_for_wrapper {
                [0.0; 4]
            } else {
                style.border_radii
            },
            border_radii_y: if has_block_kids_for_wrapper {
                [0.0; 4]
            } else {
                style.border_radii_y
            },
            outline_offset: style.outline_offset,
            outline_width: if has_block_kids_for_wrapper {
                0.0
            } else {
                style.outline_width
            },
            outline_color: if has_block_kids_for_wrapper {
                None
            } else {
                style.outline_color.map(|c| c.to_f32_rgb())
            },
            text_indent: style.text_indent,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_conic_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            background_clip,
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
            || !style.box_shadow.is_empty();
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
        // When this block routes its block-level pseudos through the Container
        // wrapper (visual box + in-flow block content), skip this sibling emit —
        // the wrapper nests the pseudo inside the padding box instead.
        if !before_is_abs && !block_pseudo_via_wrapper {
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
    // Forward sibling metadata for of-type / sibling-:has() matching.
    let child_sibling_list = element_sibling_list(&el.children);

    // If no inline content but the element has visual properties (background,
    // gradient, border, border-radius), emit a wrapper TextBlock so the visuals
    // are rendered.  Children are then pulled back inside via a negative-margin
    // spacer (same technique as flex column containers).
    // NB: check before runs is moved into wrap_text_runs above.
    let has_visual = has_background_paint(style)
        || style.border.has_any()
        || style.border_radius > 0.0
        || !style.box_shadow.is_empty();
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
        || style.padding.left > 0.0
        || style.padding.top > 0.0
        || style.padding.right > 0.0
        || style.padding.bottom > 0.0
        || (positioned_container && (before_is_abs || after_is_abs))
        || has_abs_children;
    let no_inline_content = !had_inline_runs;

    let has_abs_pseudo = positioned_container && (before_is_abs || after_is_abs);
    if (no_inline_content || has_block_kids_for_wrapper || has_abs_pseudo)
        && needs_wrapper
        && nesting_depth < 40
    {
        // Pre-flatten children to measure total height.
        // A non-absolute block-level `::before` is the element's first in-flow
        // block child, so it is laid out inside the wrapper ahead of the
        // element's own inline content (css-content-3 §1).
        let mut child_elements: Vec<LayoutElement> = Vec::new();
        if has_block_before && !before_is_abs {
            if let Some(ref ps) = before_style {
                child_elements.push(build_pseudo_block(
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
        // If there's saved inline content, include it as the next child.
        if let Some(inline_el) = saved_inline_element.take() {
            child_elements.push(inline_el);
        }
        let mut child_el_idx = 0;
        // Accumulate preceding element siblings so sibling combinators (`+`, `~`)
        // resolve during the cascade (these call sites previously passed `&[]`).
        let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
        let mut ib_group_wrapper: Vec<&ElementNode> = Vec::new();
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                // `element_is_inline_block` checks the *computed* display, so
                // it correctly matches inline tags (e.g. `<span>`) styled with
                // `display: inline-block`. Don't gate on `recurses_as_layout_child`
                // here — that excludes inline tags and is the wrong test for an
                // inline-block box, which lays out as a block regardless of tag.
                if element_is_inline_block(
                    child_el,
                    style,
                    env.rules,
                    child_ancestors,
                    child_el_idx,
                    child_el_count,
                    &preceding_siblings,
                ) {
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
                    if recurses_as_layout_child(child_el.tag)
                        || element_has_css_display_block(
                            child_el,
                            style,
                            env.rules,
                            child_ancestors,
                        )
                    {
                        // An in-flow child's percentage height resolves against
                        // this box's *content-box* height (CSS 2.1 § 10.5), not
                        // its border-box `effective_height`. For `border-box`
                        // that means subtracting this box's own padding and
                        // border; for `content-box` the effective height already
                        // is the content height. (Absolute descendants use the
                        // padding box via `make_containing_block`, kept separate.)
                        let child_cb = effective_height.map(|h| ContainingBlock {
                            x: 0.0,
                            width: inner_width,
                            height: resolve_content_box_height(
                                h,
                                style.padding.top,
                                style.padding.bottom,
                                style.border.vertical_width(),
                                style.box_sizing,
                            ),
                            depth: positioned_depth,
                        });
                        flatten_element(
                            child_el,
                            child_parent_style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_cbs(forward_abs_cb, child_cb),
                            &mut child_elements,
                            None,
                            child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &preceding_siblings,
                            forward_siblings(&child_sibling_list, child_el_idx),
                            env,
                        );
                    }
                }
                preceding_siblings.push((
                    child_el.tag_name().to_string(),
                    child_el
                        .class_list()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ));
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
        // A non-absolute block-level `::after` is the element's last in-flow
        // block child, laid out inside the wrapper after all real children
        // (css-content-3 §1). Appended before height measurement and margin
        // collapsing so it contributes to the box's height.
        if has_block_after && !after_is_abs {
            if let Some(ref ps) = after_style {
                child_elements.push(build_pseudo_block(
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
        // CSS 2.1 § 8.3.1: margins of a block and its first/last in-flow
        // children collapse when no padding/border/line box separates them.
        // Absorb the child margins into the container's own so that flow
        // layout (paginate + render_container_children) doesn't double-count
        // them. Applies only when we're actually building a Container (this
        // wrapper branch); inline/split text blocks are handled by paginate.
        let mut wrapper_margin_top = style.margin.top;
        let mut wrapper_margin_bottom = style.margin.bottom;
        // CSS 2.1 § 8.3.1: collapse-through is suppressed when this box
        // establishes a new BFC (overflow != visible, float, absolute); the
        // *bottom* collapse-through is additionally suppressed when the box has a
        // definite (non-auto) height, which contains the last child's margin.
        let bfc = crate::layout::helpers::establishes_bfc(style);
        crate::layout::helpers::collapse_margins_through_parent(
            &mut child_elements,
            &mut wrapper_margin_top,
            &mut wrapper_margin_bottom,
            style.padding.top,
            style.padding.bottom,
            style.border.top.width,
            style.border.bottom.width,
            bfc,
            bfc || has_definite_height,
        );

        // Measure children total height
        let children_h_raw: f32 = child_elements.iter().map(estimate_element_height).sum();
        // A definite `height` clamps the padding-box to that size (content
        // overflows). A `min-height`-only floor (`effective_height` set but not
        // definite) must still grow to fit taller content — pass `None` so the
        // content height is used, then apply the floor as a `max` below.
        let mut container_h = resolve_padding_box_height(
            children_h_raw,
            effective_height.filter(|_| has_definite_height),
            style.padding.top,
            style.padding.bottom,
            style.border.vertical_width(),
            style.box_sizing,
        );
        if !has_definite_height && let Some(min_h) = effective_height {
            let min_padding_box = resolve_padding_box_height(
                0.0,
                Some(min_h),
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            );
            container_h = container_h.max(min_padding_box);
        }
        if effective_height.is_none()
            && let Some(aspect_h) = aspect_ratio_height(block_w, style)
        {
            container_h = container_h.max(aspect_h);
        }
        // css-sizing-3: `max-height` clamps the used (auto-grown) height even when
        // no `height`/`min-height` is set. Lines ~195 only clamp an already-Some
        // `effective_height`, so an auto box that grows past `max-height` is not
        // caught there — apply the clamp on the measured container padding box.
        // (When `effective_height` is definite, it already incorporates the max.)
        let mut max_height_clamped = false;
        if !has_definite_height && let Some(max_h) = style.max_height {
            let max_padding_box = resolve_padding_box_height(
                0.0,
                Some(max_h),
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            );
            if container_h > max_padding_box {
                container_h = max_padding_box;
                max_height_clamped = true;
            }
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
            conic_gradient: background_conic_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
            clip: background_clip,
        } = BackgroundFields::from_style(style);
        // Resolve containing block and offsets for absolute elements.
        // Pass the border-box height (`container_h` is the padding box) so a
        // bottom-anchored absolute box measures to its border edge, not 1 border
        // width too low.
        let (wrapper_cb, wrapper_top, wrapper_left) = resolve_abs_containing_block(
            style,
            abs_containing_block,
            container_h + style.border.vertical_width(),
            block_w,
        );
        // Emit a Container element with true parent-child nesting.
        // The renderer draws background/border, then renders children inside.
        output.push(LayoutElement::Container {
            children: child_elements,
            background_color: bg,
            border: LayoutBorder::from_computed(&style.border),
            border_radius: style.border_radius,
            border_radii: style.border_radii,
            border_radii_y: style.border_radii_y,
            outline_offset: style.outline_offset,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            margin_top: wrapper_margin_top,
            margin_bottom: wrapper_margin_bottom,
            block_width: Some(block_w),
            // `block_width` is the full border-box width (`block_w` is the
            // declared width, which already includes border under border-box).
            // The renderer and flow estimate both treat a Container's
            // `block_height` as a border-box height too — they compare it against
            // a content height that already includes the border. But
            // `resolve_padding_box_height` returns the *padding-box* height, so
            // add the border back to keep width and height symmetric. (Without
            // this, an explicit-height box with a border rendered short by the
            // border thickness.) The aspect-ratio case is left as-is: its height
            // is derived from the border-box width and is already consistent.
            block_height: if effective_height.is_some() || max_height_clamped {
                Some(container_h + style.border.vertical_width())
            } else if style.aspect_ratio.is_some() {
                Some(container_h)
            } else {
                None
            },
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            background_blend_mode: style.background_blend_mode,
            visible: style.visibility == Visibility::Visible,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: wrapper_top,
            offset_left: wrapper_left + auto_offset_left,
            overflow: style.overflow,
            overflow_x: style.overflow_x,
            overflow_y: style.overflow_y,
            transform: style.transform,
            transform_origin: style.transform_origin,
            clip_path: style.clip_path.clone(),
            mask_image: style.mask_image.clone(),
            mask_mode: style.mask_mode,
            box_shadow: style.box_shadow.clone(),
            background_gradient,
            background_radial_gradient,
            background_conic_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            background_clip,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            z_index: style.z_index,
            positioned_depth,
            containing_block: wrapper_cb,
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
        let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
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
                        &preceding_siblings,
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
                    if recurses_as_layout_child(child_el.tag)
                        || element_has_css_display_block(
                            child_el,
                            style,
                            env.rules,
                            child_ancestors,
                        )
                    {
                        flatten_element(
                            child_el,
                            child_parent_style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                                .with_cbs(forward_abs_cb, cb_info),
                            output,
                            None,
                            child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &preceding_siblings,
                            forward_siblings(&child_sibling_list, child_el_idx),
                            env,
                        );
                    }
                }
                preceding_siblings.push((
                    child_el.tag_name().to_string(),
                    child_el
                        .class_list()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ));
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

    // CSS 2.1 § 8.3.1: a self-collapsing empty box (no in-flow content, zero
    // height/min-height, no padding/border, not a BFC) still contributes its
    // collapsed vertical margin to the surrounding flow — its own top and bottom
    // margins collapse together, and that single margin then collapses with the
    // adjacent siblings. When such a box produced NO layout element above, its
    // margins would otherwise vanish entirely (the gap between its siblings would
    // wrongly close up). Emit a zero-height placeholder carrying the collapsed
    // margin so adjacent-sibling collapse in `paginate` picks it up.
    let produced_nothing = output.len() == output_start_len;
    let self_collapsing = produced_nothing
        && effective_height.is_none()
        && style.min_height.is_none_or(|m| m == 0.0)
        && style.padding.top == 0.0
        && style.padding.bottom == 0.0
        && style.border.top.width == 0.0
        && style.border.bottom.width == 0.0
        && !style.overflow.clips()
        && style.position != Position::Absolute
        && style.float == Float::None;
    if self_collapsing && (style.margin.top != 0.0 || style.margin.bottom != 0.0) {
        // The box's own top and bottom margins collapse together first.
        let collapsed = if style.margin.top >= 0.0 && style.margin.bottom >= 0.0 {
            style.margin.top.max(style.margin.bottom)
        } else if style.margin.top < 0.0 && style.margin.bottom < 0.0 {
            style.margin.top.min(style.margin.bottom)
        } else {
            style.margin.top + style.margin.bottom
        };
        // Carry the collapsed margin as the placeholder's top margin so the
        // preceding-sibling collapse merges it; bottom margin is 0 so the
        // following sibling collapses against this zero-height box's bottom edge,
        // yielding a single collapsed gap rather than the sum of both.
        let mut spacer = LayoutElement::empty_spacer();
        if let LayoutElement::TextBlock { margin_top, .. } = &mut spacer {
            *margin_top = collapsed;
        }
        output.push(spacer);
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
