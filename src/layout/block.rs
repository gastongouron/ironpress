use crate::layout::elements::{
    BlockSize, Container, FlexRow, GridRow, Image, InlineOffset, InlineSize, IntoLayoutNode,
    LayoutElement, LayoutNode, LayoutVisitor, LayoutVisitorMut, MathBlock, SizeConstraints, Svg,
    TableRow, TextBlock,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{AncestorInfo, CssRule, PseudoElement, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BoxSizing, ComputedStyle, Display, Float, FontFamily, Overflow, Position, TextAlign,
    TextOverflow, WhiteSpace, compute_style_with_context_with_font_metrics,
};
use crate::types::EdgeSizes;
use std::collections::HashMap;

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    ElementSiblingContext, LayoutBorder, LayoutTreeContext, TextLine, TextRun, element_is_empty,
    element_sibling_list, emit_page_break_after, flatten_element, flatten_nodes, forward_siblings,
};
use super::helpers::{
    BackgroundFields, LayoutOverflowKeyword, PseudoBoxContext, aspect_ratio_height,
    authored_intrinsic_width_keyword, authored_keyword_property, authored_line_clamp,
    authored_overflow_axes, authored_overflow_clip_margin, authored_pseudo_keyword_property,
    authored_scrollbar_gutter, build_pseudo_block, establishes_bfc_with_overflow,
    has_background_paint, heading_level, measure_lines_width, pseudo_is_block_like,
    push_block_pseudo, recurses_as_layout_child, resolve_abs_containing_block,
    resolve_absolute_descendants_containing_block, resolve_content_box_height, resolve_inset,
    resolve_padding_box_height, resolve_relative_offsets, selector_attributes_with_has,
    selector_context_from_ancestors,
};
use super::inline::{
    layout_inline_block_group_with_spacing, layout_inline_mixed_sequence_with_env,
};
use super::inline_formatting::{
    AnonymousInlineFormattingContext, AtomicInlineEmission, GeneratedContentStyles,
    InlineContentSequence, InlineFormattingContext, InlineFormattingRole,
};
use super::paginate::estimate_element_height;
use super::text::{
    InlineRunCollector, InlineTextSequence, TextWrapOptions, apply_text_overflow_ellipsis,
    estimate_word_width, has_non_collapsible_text, is_collapsible_space, parent_line_strut,
    resolve_style_font_family, resolved_line_height_factor, text_run_line_height_factor,
    used_font_size, wrap_text_runs,
};
use super::vertical_text::upright_lines;

fn clear_first_backdropless_descendant_blend(elements: &mut [LayoutNode]) -> bool {
    struct BlendClearer(bool);

    impl LayoutVisitorMut for BlendClearer {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            if element
                .paint
                .group
                .effects
                .mix_blend_mode
                .pdf_name()
                .is_some()
            {
                element.paint.group.effects.mix_blend_mode =
                    crate::style::computed::BlendMode::Normal;
                self.0 = true;
            }
        }

        fn visit_container(&mut self, element: &mut Container) {
            if element
                .paint
                .group
                .effects
                .mix_blend_mode
                .pdf_name()
                .is_some()
            {
                element.paint.group.effects.mix_blend_mode =
                    crate::style::computed::BlendMode::Normal;
                self.0 = true;
                return;
            }
            let backdropless = element.paint.background.color.is_none()
                && !element.paint.background.layers.has_image()
                && !element.box_model.border.has_any()
                && element.paint.shadows.is_empty();
            if backdropless {
                self.0 = clear_first_backdropless_descendant_blend(&mut element.children);
            }
        }
    }

    for element in elements {
        let mut clearer = BlendClearer(false);
        element.accept_mut(&mut clearer);
        if clearer.0 {
            return true;
        }
    }
    false
}

fn add_text_horizontal_padding(element: &mut dyn LayoutElement, padding: EdgeSizes) {
    struct PaddingUpdate(EdgeSizes);

    impl LayoutVisitorMut for PaddingUpdate {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.padding.left += self.0.left;
            element.box_model.padding.right += self.0.right;
        }
    }

    element.accept_mut(&mut PaddingUpdate(padding));
}

fn set_text_block_height(element: &mut dyn LayoutElement, height: f32) {
    struct HeightUpdate(f32);

    impl LayoutVisitorMut for HeightUpdate {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.size.height = BlockSize::definite(self.0);
        }
    }

    element.accept_mut(&mut HeightUpdate(height));
}

fn offset_text_block_top(element: &mut dyn LayoutElement, offset: f32) {
    struct TopOffset(f32);

    impl LayoutVisitorMut for TopOffset {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.positioning.insets.top += self.0;
        }
    }

    element.accept_mut(&mut TopOffset(offset));
}

fn cap_height_ratio_for_initial_letter(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let FontFamily::Custom(family) = resolve_style_font_family(style, fonts) else {
        return None;
    };
    let (bold, italic) = (style.font_weight.is_bold(), style.font_style.is_slanted());
    let (_, font) = crate::system_fonts::find_font(fonts, &family, bold, italic)?;
    Some(font.cap_height_ratio()).filter(|ratio| *ratio > 0.0)
}

fn initial_letter_font_size(
    style: &ComputedStyle,
    block_font_size: f32,
    block_line_height: f32,
    size: f32,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let Some(cap_ratio) = cap_height_ratio_for_initial_letter(style, fonts) else {
        return block_line_height * size;
    };
    let normal_cap_height = cap_ratio * block_font_size;
    (((size - 1.0).max(0.0) * block_line_height) + normal_cap_height) / cap_ratio
}

fn restyle_runs_for_first_line_wrap(
    runs: &mut [TextRun],
    fl: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    let family = resolve_style_font_family(fl, fonts);
    let line_height = text_run_line_height_factor(fl, fonts);
    let font_size = used_font_size(fl, fonts);
    for run in runs {
        if run.inline_box.is_some() {
            continue;
        }
        run.font_size = font_size * fl.font_variant_position.glyph_scale();
        run.line_height_basis = font_size;
        run.font_variant_position = fl.font_variant_position;
        run.bold = fl.font_weight == crate::style::computed::FontWeight::Bold;
        run.font_style = fl.font_style;
        run.font_family = family.clone();
        run.line_height_factor = line_height;
        run.metadata = crate::layout::text::text_run_metadata(fl);
        run.shaping = crate::layout::text::text_run_shaping(fl);
    }
}

fn apply_first_line_letter_spacing(lines: &mut [TextLine], letter_spacing: f32) {
    if letter_spacing == 0.0 {
        return;
    }
    let Some(first) = lines.first_mut() else {
        return;
    };
    for run in &mut first.runs {
        if run.inline_box.is_none() {
            run.metadata.spacing.letter = letter_spacing;
            run.shaping.ligatures = false;
        }
    }
}

fn text_len_in_line(line: &TextLine) -> usize {
    line.runs.iter().map(|run| run.text.chars().count()).sum()
}

fn split_runs_at_text_len(runs: Vec<TextRun>, mut count: usize) -> (Vec<TextRun>, Vec<TextRun>) {
    let mut first = Vec::new();
    let mut rest = Vec::new();
    let mut in_rest = false;
    for run in runs {
        if in_rest || run.inline_box.is_some() {
            rest.push(run);
            continue;
        }
        let len = run.text.chars().count();
        if count >= len {
            count -= len;
            first.push(run);
            continue;
        }
        if count == 0 {
            rest.push(run);
            in_rest = true;
            continue;
        }
        let split = run
            .text
            .char_indices()
            .nth(count)
            .map(|(idx, _)| idx)
            .unwrap_or(run.text.len());
        let mut head = run.clone();
        let mut tail = run;
        head.text = head.text[..split].to_string();
        tail.text = tail.text[split..].to_string();
        if !head.text.is_empty() {
            first.push(head);
        }
        if !tail.text.is_empty() {
            rest.push(tail);
        }
        in_rest = true;
    }
    (first, rest)
}

fn trim_leading_collapsible_space(runs: &mut Vec<TextRun>) {
    while let Some(first) = runs.first_mut() {
        if first.inline_box.is_some() {
            return;
        }
        let trimmed = first
            .text
            .trim_start_matches(is_collapsible_space)
            .to_string();
        if trimmed.is_empty() {
            runs.remove(0);
        } else {
            first.text = trimmed;
            return;
        }
    }
}

#[cfg(test)]
mod text_fragment_tests {
    use super::*;

    #[test]
    fn first_line_trimming_preserves_non_breaking_space() {
        let mut runs = vec![TextRun {
            text: "\u{00a0}alpha".to_string(),
            ..Default::default()
        }];

        trim_leading_collapsible_space(&mut runs);

        assert_eq!(runs[0].text, "\u{00a0}alpha");
    }
}

fn wrap_text_runs_with_first_line_style(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    fl: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextLine> {
    let mut styled_runs = runs.clone();
    restyle_runs_for_first_line_wrap(&mut styled_runs, fl, fonts);
    let styled_lines = wrap_text_runs(styled_runs, options, fonts);
    let Some(first_styled) = styled_lines.first() else {
        return Vec::new();
    };
    let first_len = text_len_in_line(first_styled);
    if first_len == 0 {
        return wrap_text_runs(runs, options, fonts);
    }
    let (mut first_runs, mut rest_runs) = split_runs_at_text_len(runs, first_len);
    trim_leading_collapsible_space(&mut rest_runs);
    restyle_runs_for_first_line_wrap(&mut first_runs, fl, fonts);
    let mut first_lines = wrap_text_runs(first_runs, options, fonts);
    first_lines.truncate(1);
    let mut tail_options = options;
    tail_options.text_indent = 0.0;
    tail_options.drop_cap = None;
    let mut tail_lines = wrap_text_runs(rest_runs, tail_options, fonts);
    first_lines.append(&mut tail_lines);
    first_lines
}

fn collect_plain_text_for_dir_auto(nodes: &[DomNode], out: &mut String) {
    for node in nodes {
        match node {
            DomNode::Text(text) => out.push_str(text),
            DomNode::Element(el) => collect_plain_text_for_dir_auto(&el.children, out),
        }
    }
}

fn has_direct_table_cell_child(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    font_metrics: crate::style::font_metrics::FontMetrics<'_>,
) -> bool {
    let siblings = element_sibling_list(nodes);
    let sibling_count = siblings.len();
    let mut child_index = 0usize;

    for node in nodes {
        let DomNode::Element(child) = node else {
            continue;
        };
        let classes = child.class_list();
        let style = compute_style_with_context_with_font_metrics(
            child.tag,
            child.style_attr(),
            parent_style,
            rules,
            child.tag_name(),
            &classes,
            child.id(),
            &selector_attributes_with_has(child),
            &SelectorContext {
                ancestors: ancestors.to_vec(),
                child_index,
                sibling_count,
                preceding_siblings: siblings[..child_index].to_vec(),
                following_siblings: siblings[child_index + 1..].to_vec(),
                is_empty: element_is_empty(child),
            },
            font_metrics,
        );
        child_index += 1;
        if style.display == Display::TableCell {
            return true;
        }
    }
    false
}

/// Lay out a `display: block` or `display: inline-block` element.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_block_element(
    el: &ElementNode,
    style: &mut ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    child_ancestors: &[AncestorInfo],
    positioned_depth: usize,
    generated_styles: &GeneratedContentStyles,
    first_line_style: Option<&ComputedStyle>,
    first_letter_style: Option<&ComputedStyle>,
    env: &mut LayoutEnv,
) {
    let before_style = generated_styles.before();
    let after_style = generated_styles.after();
    let output_start_len = output.len();
    if el.attributes.get("dir").is_some_and(|v| v == "auto") {
        let mut text = String::new();
        collect_plain_text_for_dir_auto(&el.children, &mut text);
        style.direction_rtl = crate::bidi::first_strong_is_rtl(&text);
        style.text_align = if style.direction_rtl {
            TextAlign::Right
        } else {
            TextAlign::Left
        };
    }
    let available_width = ctx.available_width();
    let available_height = ctx.available_height();
    // Basis for percentage `width`/`min-width`/`max-width` (CSS 2.1 § 10.2):
    // the containing block's content width. For normal block flow this equals
    // `available_width`; flex layout hands an item its own resolved width as
    // `available_width` but keeps the container content width as the basis.
    let percent_width_basis = ctx.parent.percent_width_basis;
    let abs_containing_block = if style.position == Position::Fixed {
        Some(ctx.initial_fixed_containing_block())
    } else {
        ctx.containing_block
    };
    // Percentage `height` resolves against the parent's content box (CSS 2.1
    // § 10.5), tracked separately from the absolute containing block so the two
    // are not conflated when a child sits inside a static element.
    let percent_height_cb = ctx.percent_height_cb.or(abs_containing_block);
    // Compute effective block width considering CSS width/max-width/min-width.
    // Block elements without explicit width shrink by their horizontal margins.
    let margin_h = style.margin.horizontal();
    // For `box-sizing: content-box` the declared width is the *content* width,
    // so the outer (border-box) width that `block_w` represents is the declared
    // width plus horizontal padding and border. For `border-box` the declared
    // width already is the border-box width.
    let content_box_extra = if style.box_sizing == BoxSizing::ContentBox {
        style.padding.horizontal() + style.border.horizontal_width()
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
        block_w = crate::layout::intrinsic_width::resolve_intrinsic_keyword_width(
            el,
            style,
            keyword,
            available_width,
            env.rules,
            env.fonts,
        );
    } else if let (Some(ratio), Some(h)) = (style.aspect_ratio, style.height)
        && ratio > 0.0
    {
        block_w = h * ratio + content_box_extra;
    } else if margin_h > 0.0 {
        block_w = (available_width - margin_h).max(0.0);
    }
    let authored_max_width_keyword =
        authored_intrinsic_width_keyword(el, env.rules, child_ancestors, "max-width");
    let authored_min_width_keyword =
        authored_intrinsic_width_keyword(el, env.rules, child_ancestors, "min-width");

    // CSS 2.1 § 10.4: percentage min-/max-width also resolve against the
    // containing block content width. Min wins over max (the floor is applied
    // last) per css-sizing-3 — `max(min, min(value, max))`.
    if let Some(pct) = style.percentage_sizing.max_width {
        block_w = block_w.min(pct / 100.0 * percent_width_basis);
    } else if let Some(keyword) = authored_max_width_keyword {
        block_w = block_w.min(
            crate::layout::intrinsic_width::resolve_intrinsic_keyword_width(
                el,
                style,
                keyword,
                available_width,
                env.rules,
                env.fonts,
            ),
        );
    } else if let Some(mw) = style.max_width {
        block_w = block_w.min(mw);
    }
    if let Some(pct) = style.percentage_sizing.min_width {
        block_w = block_w.max(pct / 100.0 * percent_width_basis);
    } else if let Some(keyword) = authored_min_width_keyword {
        block_w = block_w.max(
            crate::layout::intrinsic_width::resolve_intrinsic_keyword_width(
                el,
                style,
                keyword,
                available_width,
                env.rules,
                env.fonts,
            ),
        );
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
        let padding_border_w = style.padding.horizontal() + style.border.horizontal_width();
        block_w = block_w.max(padding_border_w);
    }

    // CSS 2.1 § 10.3.7 over-constrained absolute width: when `width: auto` and
    // BOTH `left` and `right` are set, the box stretches to fill the containing
    // block, inset by left/right (and horizontal margins). `block_w` is the
    // border-box width, so the stretched border-box width is
    // `cb.width - left - right - margin_h` (padding/border are inside it).
    if style.position.is_absolute()
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
        && style.position.is_absolute()
        && style.percentage_sizing.height.is_none()
        && let Some(cb) = abs_containing_block
    {
        let top = resolve_inset(style.top, style.percentage_insets.top, cb.height);
        let bottom = resolve_inset(style.bottom, style.percentage_insets.bottom, cb.height);
        if let (Some(top), Some(bottom)) = (top, bottom) {
            let margin_v = style.margin.vertical();
            effective_height = Some((cb.height - top - bottom - margin_v).max(0.0));
        }
    }
    if effective_height.is_none() {
        if let Some(pct) = style.percentage_sizing.height {
            // An absolute box's percentage height resolves against its absolute
            // containing block (the positioned ancestor's padding box); an
            // in-flow box's against the parent's content box (CSS 2.1 § 10.5).
            let height_cb = if style.position.is_absolute() {
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
    let authored_height_constraints = SizeConstraints::new(style.min_height, style.max_height);
    effective_height = authored_height_constraints.constrain_preferred(effective_height);

    let has_explicit_width = style.width.is_some()
        || style.width_keyword.is_some()
        || style.max_width.is_some()
        || style.min_width.is_some()
        || authored_max_width_keyword.is_some()
        || authored_min_width_keyword.is_some()
        || style.percentage_sizing.width.is_some();
    let auto_offset_left =
        InlineOffset::resolve_block_start(style, available_width, block_w).value();

    let selector_ctx = selector_context_from_ancestors(child_ancestors, el);
    let mut overflow_axes =
        authored_overflow_axes(el, style, env.rules, child_ancestors, &selector_ctx);
    if authored_keyword_property(
        el,
        env.rules,
        child_ancestors,
        &selector_ctx,
        "scrollbar-width",
    )
    .as_deref()
        == Some("none")
    {
        if matches!(
            overflow_axes.x,
            LayoutOverflowKeyword::Scroll | LayoutOverflowKeyword::Auto
        ) {
            overflow_axes.x = LayoutOverflowKeyword::Hidden;
        }
        if matches!(
            overflow_axes.y,
            LayoutOverflowKeyword::Scroll | LayoutOverflowKeyword::Auto
        ) {
            overflow_axes.y = LayoutOverflowKeyword::Hidden;
        }
    }
    let overflow_clip_margin =
        authored_overflow_clip_margin(el, env.rules, child_ancestors, &selector_ctx);
    let line_clamp = authored_line_clamp(el, env.rules, child_ancestors, &selector_ctx);
    let (scrollbar_gutter_left, scrollbar_gutter_right) = if matches!(
        overflow_axes.y,
        LayoutOverflowKeyword::Hidden | LayoutOverflowKeyword::Scroll | LayoutOverflowKeyword::Auto
    ) {
        authored_scrollbar_gutter(el, env.rules, child_ancestors, &selector_ctx)
    } else {
        (0.0, 0.0)
    };
    let mut layout_padding = style.padding;
    layout_padding.left += scrollbar_gutter_left;
    layout_padding.right += scrollbar_gutter_right;

    // `block_w` is now the border-box (outer) width for both box-sizing modes
    // (content-box added padding+border above), so the content area is always
    // the outer width minus horizontal padding and border.
    let inner_width = block_w - layout_padding.horizontal() - style.border.horizontal_width();
    let inner_width = inner_width.max(0.0);

    // Resolve percentage border-radius once at the style-to-layout boundary.
    // Horizontal percentages use the border-box width and vertical percentages
    // use its height, producing elliptical corners on a non-square box.
    let height_dim = effective_height
        .map(|h| {
            resolve_padding_box_height(
                0.0,
                Some(h),
                style.padding,
                style.border.widths(),
                style.box_sizing,
            ) + style.border.vertical_width()
        })
        .unwrap_or(block_w);
    let border_radii = style.resolve_corner_radii(block_w, height_dim);

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
        let content_h =
            resolve_content_box_height(h, style.padding, style.border.widths(), style.box_sizing);
        if content_h == h {
            None
        } else {
            let mut adjusted = style.clone();
            adjusted.height = Some(content_h);
            Some(adjusted)
        }
    });
    let child_parent_style: &ComputedStyle = child_parent_owned.as_ref().unwrap_or(style);

    let ib_ctx = ctx.with_parent(inner_width, ctx.parent.content_height, style.font_size);

    // A positioned, transformed, or non-`none` filtered element establishes a
    // containing block for absolute descendants. Filter Effects 1 gives a
    // filtered non-positioned ancestor the same containing-block behavior as a
    // transform.
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
                    + style.border.left.used_width()
                    + layout_padding.left,
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
            resolve_content_box_height(h, style.padding, style.border.widths(), style.box_sizing)
                + style.padding.vertical()
        });
        make_containing_block(cb_padding_box_h)
    } else {
        abs_containing_block
    };

    // Emit block-level ::before pseudo-element.
    let before_is_abs = before_style.is_some_and(|s| s.position.is_absolute());
    let after_is_abs = after_style.is_some_and(|s| s.position.is_absolute());
    let pseudo_selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        ..SelectorContext::default()
    };
    let before_is_list_item = authored_pseudo_keyword_property(
        el,
        env.rules,
        &pseudo_selector_ctx,
        PseudoElement::Before,
        "display",
    )
    .is_some_and(|value| value.split_whitespace().any(|part| part == "list-item"));
    let after_is_list_item = authored_pseudo_keyword_property(
        el,
        env.rules,
        &pseudo_selector_ctx,
        PseudoElement::After,
        "display",
    )
    .is_some_and(|value| value.split_whitespace().any(|part| part == "list-item"));
    // A non-absolute block-level `::before`/`::after` (e.g.
    // `.card::before { content: "HEADER"; display: block }`) is an in-flow
    // block-level child of the originating element: it must be laid out INSIDE
    // the element's content box as the first/last block, not as a sibling
    // before/after it (css-content-3 §1, css-display-3). It therefore forces the
    // Container wrapper path just like a real block child does.
    let has_block_before =
        before_style.is_some_and(|s| pseudo_is_block_like(s) && !s.position.is_absolute());
    let has_block_after =
        after_style.is_some_and(|s| pseudo_is_block_like(s) && !s.position.is_absolute());
    let has_any_block_pseudo = before_style.is_some_and(pseudo_is_block_like)
        || after_style.is_some_and(pseudo_is_block_like);
    let has_inflow_block_pseudo = has_block_before || has_block_after;
    let inline_sequence =
        InlineContentSequence::with_generated(&el.children, generated_styles.boxes(el));
    let inline_children =
        InlineFormattingContext::new(style, env.rules, child_ancestors, env.font_metrics())
            .children(inline_sequence);
    let has_out_of_flow_children = inline_children.has_out_of_flow();
    let child_sibling_list = element_sibling_list(&el.children);
    let child_el_count = child_sibling_list.len();
    let has_blended_block_child = inline_children.iter().any(|child| {
        child.style.mix_blend_mode != crate::style::computed::BlendMode::Normal
            || child.style.opacity < 1.0
            || child.style.isolation.isolates()
    });
    // `early_has_visual`/`nesting_depth` are needed both here (to decide whether
    // a block pseudo routes through the Container wrapper) and later (to gate the
    // wrapper itself), so compute them once up front.
    let early_has_visual = has_background_paint(style)
        || style.has_border_decoration()
        || !border_radii.is_zero()
        || style.mask_image.is_some()
        || !style.box_shadow.is_empty()
        || style.opacity < 1.0
        || style.mix_blend_mode != crate::style::computed::BlendMode::Normal
        || style.isolation.isolates()
        || style.filter.establishes_stacking_context
        || has_blended_block_child;
    let nesting_depth = ancestors.len();
    // A block-level `::before` is normally emitted here as the first in-flow
    // block. But when this element takes the Container wrapper path (it has
    // visual box decoration AND in-flow block content), the pseudo must instead
    // be nested INSIDE the wrapper as its first child — handled below — so it
    // sits within the element's padding box rather than as a preceding sibling.
    let has_table_cell_children = has_direct_table_cell_child(
        &el.children,
        style,
        env.rules,
        child_ancestors,
        env.font_metrics(),
    );
    let requires_principal_box = early_has_visual || style.position.is_positioned();
    let has_block_kids_for_wrapper = nesting_depth < 40
        && (has_inflow_block_pseudo
            || early_has_visual && has_out_of_flow_children
            || requires_principal_box && inline_children.has_outside());
    let block_pseudo_via_wrapper = has_inflow_block_pseudo && has_block_kids_for_wrapper;
    if let Some(ps) = before_style {
        if pseudo_is_block_like(ps) && !before_is_abs && !block_pseudo_via_wrapper {
            output.push(build_pseudo_block(
                ps,
                el,
                PseudoBoxContext::new(inner_width, env.fonts, env.filter_defs, &mut *env.resources)
                    .with_positioned_ancestor_depth(positioned_depth),
                env.counter_state,
                before_is_list_item,
            ));
        }
    }

    // When the element has absolute pseudo-elements, skip inline text
    // collection. The wrapper path will handle all children via
    // flatten_element, avoiding double-rendering of text.
    let skip_inline_collection =
        has_table_cell_children || positioned_container && (before_is_abs || after_is_abs);

    // Collect inline content as text runs, splitting at math elements.
    // When a math span is encountered, flush accumulated text runs as a
    // TextBlock, emit a MathBlock, then continue collecting.
    let mut runs = Vec::new();
    let mut mixed_inline_row_emitted = false;
    let mut generated_inline_before_runs = Vec::new();

    // Helper closure: flush accumulated runs as a TextBlock
    let flush_runs = |runs: &mut Vec<TextRun>,
                      inner_width: f32,
                      style: &ComputedStyle,
                      available_width: f32,
                      block_w: f32,
                      effective_height: Option<f32>,
                      auto_offset_left: f32,
                      el: &ElementNode,
                      output: &mut Vec<LayoutNode>,
                      fonts: &HashMap<String, TtfFont>| {
        if runs.is_empty() {
            return;
        }
        runs.as_mut_slice()
            .resolve_unclaimed_boundaries(crate::layout::elements::TextSpacing::from_style(style));
        let shrink_to_fit = style.width.is_none()
            && style.percentage_sizing.width.is_none()
            && (style.display == Display::InlineBlock
                || style.position.is_absolute()
                || matches!(style.float, Float::Left | Float::Right));
        let wrap_width = if style.text_wrap_mode_nowrap || shrink_to_fit {
            f32::MAX
        } else {
            inner_width
        };
        let text_indent = style.text_indent.resolve(inner_width);
        let lines = wrap_text_runs(
            std::mem::take(runs),
            TextWrapOptions::new(
                wrap_width,
                used_font_size(style, fonts),
                text_run_line_height_factor(style, fonts),
                style.overflow_wrap,
            )
            .with_white_space(style.white_space)
            .with_parent_strut(parent_line_strut(style, fonts))
            .with_rtl(style.direction_rtl)
            .with_bidi_override(style.bidi_override)
            .with_bidi_plaintext(style.bidi_plaintext)
            .with_word_break_keep_all(style.word_break_keep_all)
            .with_line_break_anywhere(style.line_break_anywhere)
            .with_hyphens_manual(style.hyphens_manual)
            .with_text_indent(text_indent),
            fonts,
        );
        if lines.is_empty() {
            return;
        }
        // For inline-block without explicit width, shrink-to-fit
        let render_w = if shrink_to_fit {
            (measure_lines_width(&lines, fonts)
                + layout_padding.horizontal()
                + style.border.horizontal_width())
            .min(block_w)
        } else {
            block_w
        };

        let inline_size = InlineSize::from_used(
            render_w,
            available_width,
            style.width.is_some()
                || style.percentage_sizing.width.is_some()
                || style.min_width.is_some()
                || shrink_to_fit,
        );
        let mut block = TextBlock::from_style(
            lines,
            style,
            crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize {
                    width: inline_size,
                    height: effective_height.map_or(BlockSize::AUTO, |height| {
                        if has_definite_height {
                            BlockSize::definite(height)
                        } else {
                            BlockSize::minimum(height)
                        }
                    }),
                },
                margins: BlockMargins::new(style.margin.top, style.margin.bottom),
                padding: layout_padding,
                border: LayoutBorder::from_computed(&style.border, style.color),
            },
        );
        block.paint.border_radii = border_radii;
        block.positioning.insets.left += auto_offset_left;
        block.positioning.containing_block_depth = positioned_depth;
        block.text.indent = text_indent;
        block.semantics.heading_level = heading_level(el.tag);
        output.push(block.boxed());
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
        inline_sequence.append_before(&mut runs, env.fonts, env.counter_state, &mut *env.resources);
        // Split mode: interleave TextBlocks and MathBlocks
        for (child_index, child) in el.children.iter().enumerate() {
            if let DomNode::Element(child_el) = child
                && let Some(tex) = child_el.attributes.get("data-math")
            {
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
                let child_classes = child_el.class_list();
                let is_display = child_classes.contains(&"math-display");
                let ast = crate::parser::math::parse_math(tex);
                let math_layout =
                    crate::layout::math::layout_math(&ast, style.font_size, is_display);
                output.push(
                    MathBlock {
                        layout: math_layout,
                        display: is_display,
                        margins: BlockMargins::ZERO,
                        group: crate::layout::elements::PaintGroup::from_style(style),
                    }
                    .boxed(),
                );
                continue;
            }
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .collect(
                    inline_sequence.item(child_index),
                    style,
                    &mut runs,
                    None,
                    child_ancestors,
                );
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
        let parent_has_visual = has_background_paint(style)
            || style.has_border_decoration()
            || !border_radii.is_zero()
            || style.mask_image.is_some()
            || !style.box_shadow.is_empty()
            || style.opacity < 1.0
            || style.mix_blend_mode != crate::style::computed::BlendMode::Normal
            || style.isolation.isolates()
            || style.filter.establishes_stacking_context
            || has_blended_block_child;
        let early_has_abs_children = has_out_of_flow_children;
        let has_abs_pseudo_early = positioned_container && (before_is_abs || after_is_abs);
        // A non-visual block whose padding offsets its children cannot use the
        // flat fast path: that path discards parent padding (it only propagates
        // it for visual containers, lines ~727-740). Route padded blocks through
        // the Container/wrapper path below, which applies the content-box origin
        // (padding + border) to every child type. Without this, a `display:flex`,
        // positioned, or block child of e.g. `<div style="padding:20px">` renders
        // at the parent's border-box origin (padding silently dropped).
        let has_padding_offset = !layout_padding.is_zero();
        let has_block_children = !has_block_kids_for_wrapper
            && !parent_has_visual
            && !has_padding_offset
            && !early_has_abs_children
            && !has_abs_pseudo_early
            && inline_children
                .iter()
                .any(|child| matches!(child.role, InlineFormattingRole::Outside));

        if skip_inline_collection {
            // All content will be handled by the wrapper path below.
            // Don't collect inline text — the <p> children will be
            // processed via flatten_element in the Container wrapper.
        } else if has_block_children {
            inline_sequence.append_before(
                &mut runs,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            // For visual containers (border, background), emit a wrapper
            // TextBlock first, then a pullback spacer so children render
            // inside the wrapper's padding area.
            let wrapper_output_idx = output.len();
            if parent_has_visual {
                // Wrapper height will be patched after children are processed.
                let wrapper_h = effective_height.map_or(0.0, |height| {
                    resolve_padding_box_height(
                        0.0,
                        Some(height),
                        style.padding,
                        style.border.widths(),
                        style.box_sizing,
                    )
                });
                let mut wrapper = TextBlock::from_style(
                    Vec::new(),
                    style,
                    crate::layout::elements::BoxModel {
                        size: crate::layout::elements::LayoutSize {
                            width: InlineSize::fixed(block_w),
                            height: effective_height.map_or(BlockSize::AUTO, |_| {
                                if has_definite_height {
                                    BlockSize::definite(wrapper_h)
                                } else {
                                    BlockSize::minimum(wrapper_h)
                                }
                            }),
                        },
                        margins: BlockMargins::new(style.margin.top, 0.0),
                        padding: layout_padding.horizontal_only(),
                        border: LayoutBorder::from_computed(&style.border, style.color),
                    },
                );
                wrapper.paint.border_radii = border_radii;
                wrapper.positioning.insets.left += auto_offset_left;
                wrapper.positioning.containing_block_depth = positioned_depth;
                wrapper.clipping.rect = style
                    .overflow
                    .clips()
                    .then(|| crate::types::Rect::from_xywh(0.0, 0.0, block_w, wrapper_h));
                output.push(wrapper.boxed());
                // Pullback spacer
                let pullback = if effective_height.is_some() && wrapper_h > 0.0 {
                    wrapper_h - style.padding.top
                } else {
                    0.0
                };
                if pullback > 0.0 {
                    let mut spacer = TextBlock::empty_spacer();
                    spacer.box_model.margins = BlockMargins::new(-pullback, 0.0);
                    spacer.box_model.padding = layout_padding.horizontal_only();
                    output.push(spacer.boxed());
                }
            }

            // Mixed inline + block children: split at block boundaries.
            let mut block_child_buf: Vec<LayoutNode> = Vec::new();
            let target: &mut Vec<LayoutNode> = if parent_has_visual {
                &mut block_child_buf
            } else {
                output
            };
            // Atomic inlines need one source-ordered row before a block sibling
            // splits the surrounding formatting context.
            let mut atomic_inline_segments =
                InlineFormattingContext::new(style, env.rules, child_ancestors, env.font_metrics())
                    .atomic_layout_segments(inline_sequence)
                    .into_iter()
                    .peekable();
            let mut emitted_atomic_until = 0;
            let mut child_el_idx = 0;
            for (child_index, child) in el.children.iter().enumerate() {
                if atomic_inline_segments
                    .peek()
                    .is_some_and(|segment| segment.start() == child_index)
                {
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
                    if let Some(segment) = atomic_inline_segments.next()
                        && layout_inline_mixed_sequence_with_env(
                            segment,
                            style,
                            &ctx.with_parent(inner_width, Some(available_height), style.font_size),
                            target,
                            child_ancestors,
                            env,
                        )
                    {
                        emitted_atomic_until = segment.end();
                    }
                }
                if child_index < emitted_atomic_until {
                    if matches!(child, DomNode::Element(_)) {
                        child_el_idx += 1;
                    }
                    continue;
                }
                match child {
                    DomNode::Text(_) => {
                        InlineRunCollector::new(
                            env.rules,
                            env.fonts,
                            env.counter_state,
                            &mut *env.resources,
                        )
                        .collect(
                            inline_sequence.item(child_index),
                            style,
                            &mut runs,
                            None,
                            child_ancestors,
                        );
                    }
                    DomNode::Element(child_el)
                        if inline_children.requires_independent_layout(child_el_idx) =>
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
                        let child_percent_cb = effective_height.map(|height| ContainingBlock {
                            x: 0.0,
                            width: inner_width,
                            height: resolve_content_box_height(
                                height,
                                style.padding,
                                style.border.widths(),
                                style.box_sizing,
                            ),
                            depth: positioned_depth,
                        });
                        flatten_element(
                            child_el,
                            LayoutTreeContext::new(
                                child_parent_style,
                                &ctx.with_parent(
                                    inner_width,
                                    Some(available_height),
                                    style.font_size,
                                )
                                .with_cbs(forward_abs_cb, child_percent_cb),
                                child_ancestors,
                            )
                            .with_positioned_ancestor_depth(positioned_depth)
                            .for_element(
                                ElementSiblingContext::new(child_el_idx, child_el_count)
                                    .with_neighbors(
                                        &child_sibling_list[..child_el_idx],
                                        forward_siblings(&child_sibling_list, child_el_idx),
                                    ),
                            ),
                            target,
                            env,
                        );
                    }
                    DomNode::Element(_) => {
                        // Inline element: collect as text runs
                        InlineRunCollector::new(
                            env.rules,
                            env.fonts,
                            env.counter_state,
                            &mut *env.resources,
                        )
                        .collect(
                            inline_sequence.item(child_index),
                            style,
                            &mut runs,
                            None,
                            child_ancestors,
                        );
                    }
                }
                if matches!(child, DomNode::Element(_)) {
                    child_el_idx += 1;
                }
            }
            // Generated `::after` is the final child of the originating
            // element, so it follows the last real block child in source
            // order and forms an anonymous inline box there.
            inline_sequence.append_after(
                &mut runs,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            // Flush remaining inline runs after the last block child.
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
                if layout_padding.horizontal() > 0.0 {
                    for elem in &mut block_child_buf {
                        add_text_horizontal_padding(elem.as_mut(), layout_padding);
                    }
                }
                output.extend(block_child_buf);

                // Patch wrapper block_height to cover all children
                if effective_height.is_none() {
                    let children_total_h: f32 = output[wrapper_output_idx + 1..]
                        .iter()
                        .map(|element| estimate_element_height(element.as_ref()))
                        .sum();
                    let patched_h =
                        children_total_h + style.padding.vertical() + style.border.vertical_width();
                    if let Some(element) = output.get_mut(wrapper_output_idx) {
                        set_text_block_height(element.as_mut(), patched_h);
                    }
                }
            }
            // Add bottom spacer for visual containers
            if parent_has_visual {
                let bottom_space =
                    style.padding.bottom + style.border.vertical_width() + style.margin.bottom;
                if bottom_space > 0.0 {
                    let mut spacer = TextBlock::empty_spacer();
                    spacer.box_model.margins = BlockMargins::new(bottom_space, 0.0);
                    output.push(spacer.boxed());
                }
            }
            // Emit absolute-positioned ::before / ::after pseudo-elements
            if positioned_container && (before_is_abs || after_is_abs) {
                // Compute containing block height from children.
                // Use total element height but strip outer margins of the
                // first/last children — those margins collapse out of the
                // containing block and shouldn't inflate height:100% pseudos.
                let children_slice = &output[wrapper_output_idx..];
                let children_h_raw: f32 = children_slice
                    .iter()
                    .map(|element| estimate_element_height(element.as_ref()))
                    .sum();
                let children_h = crate::layout::helpers::collapse_outer_child_margins(
                    children_slice,
                    children_h_raw,
                    style.padding,
                    style.border.widths(),
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
                        before_style,
                        el,
                        PseudoBoxContext::new(
                            inner_width,
                            env.fonts,
                            env.filter_defs,
                            &mut *env.resources,
                        )
                        .with_containing_block(pseudo_cb)
                        .with_positioned_ancestor_depth(positioned_depth),
                        env.counter_state,
                    );
                }
                if after_is_abs {
                    push_block_pseudo(
                        output,
                        after_style,
                        el,
                        PseudoBoxContext::new(
                            inner_width,
                            env.fonts,
                            env.filter_defs,
                            &mut *env.resources,
                        )
                        .with_containing_block(pseudo_cb)
                        .with_positioned_ancestor_depth(positioned_depth),
                        env.counter_state,
                    );
                }
            }

            emit_page_break_after(style, output);
            return;
        } else if has_block_kids_for_wrapper {
            // Inline generated boundaries live on opposite sides of the real
            // block children. Preserve the leading fragment now; the trailing
            // fragment is resolved after the children so counters and quotes
            // observe source order.
            inline_sequence.append_before(
                &mut generated_inline_before_runs,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            // Only collect inline children's text — block children will
            // be handled by the needs_wrapper path via flatten_element.
            let mut child_el_idx = 0;
            for (child_index, child) in el.children.iter().enumerate() {
                match child {
                    DomNode::Text(_) => {
                        InlineRunCollector::new(
                            env.rules,
                            env.fonts,
                            env.counter_state,
                            &mut *env.resources,
                        )
                        .collect(
                            inline_sequence.item(child_index),
                            style,
                            &mut runs,
                            None,
                            child_ancestors,
                        );
                    }
                    DomNode::Element(_) if inline_children.is_inline_text(child_el_idx) => {
                        InlineRunCollector::new(
                            env.rules,
                            env.fonts,
                            env.counter_state,
                            &mut *env.resources,
                        )
                        .collect(
                            inline_sequence.item(child_index),
                            style,
                            &mut runs,
                            None,
                            child_ancestors,
                        );
                    }
                    _ => {} // Block children handled by needs_wrapper
                }
                if matches!(child, DomNode::Element(_)) {
                    child_el_idx += 1;
                }
            }
        } else if InlineFormattingContext::new(
            style,
            env.rules,
            child_ancestors,
            env.font_metrics(),
        )
        .requires_atomic_layout(inline_sequence)
            && layout_inline_mixed_sequence_with_env(
                inline_sequence,
                style,
                &ctx.with_parent(inner_width, Some(available_height), style.font_size),
                output,
                child_ancestors,
                env,
            )
        {
            // The mixed inline-row path emitted the inline content as one row.
            mixed_inline_row_emitted = true;
        } else {
            inline_sequence.append_before(
                &mut runs,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .collect_box_content(&el.children, style, &mut runs, None, child_ancestors);
        }
    }
    if !skip_inline_collection && !mixed_inline_row_emitted && !has_block_kids_for_wrapper {
        inline_sequence.append_after(&mut runs, env.fonts, env.counter_state, &mut *env.resources);
    }

    // `::first-letter` (css-pseudo-4 §2.2): split off and restyle the first
    // typographic letter unit before line breaking so its (possibly larger)
    // glyph participates in wrapping. A dropped initial returns its complete
    // inline exclusion geometry for the lines it spans.
    let mut drop_cap: Option<crate::layout::helpers::DropCap> = None;
    if let Some(fl) = first_letter_style {
        let block_line_height = style.font_size * resolved_line_height_factor(style, env.fonts);
        let initial_letter_style;
        let fl = if fl.initial_letter > 1.0 {
            initial_letter_style = {
                let mut s = fl.clone();
                s.font_size = initial_letter_font_size(
                    &s,
                    style.font_size,
                    block_line_height,
                    fl.initial_letter,
                    env.fonts,
                );
                s
            };
            &initial_letter_style
        } else {
            fl
        };
        let is_drop_cap = fl.initial_letter > 1.0 || matches!(fl.float, Float::Left);
        // The initial letter's used font size aligns its cap height, while its
        // inline margin-box bearings follow the spanned line metric: one parent
        // em plus every additional line-height (css-inline-3 §7.5–7.6).
        let initial_letter_inline_metric_size = (fl.initial_letter > 1.0)
            .then_some(style.font_size + (fl.initial_letter - 1.0) * block_line_height);
        drop_cap = crate::layout::helpers::apply_first_letter_style(
            &mut runs,
            fl,
            env.fonts,
            block_line_height,
            is_drop_cap,
            initial_letter_inline_metric_size,
        );
    }

    let had_text_runs = runs.iter().any(|run| has_non_collapsible_text(&run.text));
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
            || !layout_padding.is_zero())
        && nesting_depth < 40;
    let had_inline_runs = mixed_inline_row_emitted
        || had_text_runs
        || (has_inline_box_runs && !will_use_group_wrapper)
        || has_math_children;
    let atomic_inline_emission = if mixed_inline_row_emitted {
        AtomicInlineEmission::MixedRow
    } else if has_inline_box_runs && !will_use_group_wrapper {
        AtomicInlineEmission::InlineBlockRuns
    } else {
        AtomicInlineEmission::Independent
    };
    if will_use_group_wrapper {
        // Pure inline-block group inside a wrapper: drop the placeholder runs so
        // `layout_inline_block_group` lays them out (unchanged behaviour).
        runs.clear();
    }
    let mut cb_info = None;

    // has_block_kids_for_wrapper is computed earlier (before has_math_children).
    let mut saved_inline_element: Option<LayoutNode> = None;

    if !runs.is_empty() {
        // `white-space: nowrap` and `pre` never soft-wrap: render with an
        // unbounded width so only explicit newlines break lines. `pre-wrap`
        // keeps spaces but still wraps at the box edge.
        let vertical_content_height = effective_height.map(|h| {
            if style.box_sizing == BoxSizing::BorderBox {
                (h - style.padding.vertical() - style.border.vertical_width()).max(0.0)
            } else {
                h
            }
        });
        let shrink_to_fit = style.width.is_none()
            && style.percentage_sizing.width.is_none()
            && (style.display == Display::InlineBlock
                || style.position.is_absolute()
                || matches!(style.float, Float::Left | Float::Right));
        let wrap_width = if style.text_wrap_mode_nowrap || shrink_to_fit {
            f32::MAX
        } else if style.writing_mode.is_vertical() {
            vertical_content_height.unwrap_or(inner_width)
        } else {
            inner_width
        };
        let text_indent = style
            .text_indent
            .resolve(vertical_content_height.unwrap_or(inner_width));
        let wrap_options = TextWrapOptions::new(
            wrap_width,
            used_font_size(style, env.fonts),
            text_run_line_height_factor(style, env.fonts),
            style.overflow_wrap,
        )
        .with_rtl(style.direction_rtl)
        .with_bidi_override(style.bidi_override)
        .with_bidi_plaintext(style.bidi_plaintext)
        .with_word_break_keep_all(style.word_break_keep_all)
        .with_line_break_anywhere(style.line_break_anywhere)
        .with_hyphens_manual(style.hyphens_manual)
        .with_white_space(style.white_space)
        .with_parent_strut(parent_line_strut(style, env.fonts))
        // text-indent shortens the FIRST formatted line, so the wrapper must
        // reserve that space before breaking — otherwise the first line packs
        // full-width text that then overflows once shifted at paint time
        // (css-text-3 §8).
        .with_text_indent(text_indent)
        // A dropped initial letter reserves its kerned margin-box exclusion on
        // every line it overlaps (css-inline-3 §7.5–7.8).
        .with_drop_cap(drop_cap);
        let mut lines = if let Some(fl) = first_line_style {
            wrap_text_runs_with_first_line_style(runs, wrap_options, fl, env.fonts)
        } else {
            wrap_text_runs(runs, wrap_options, env.fonts)
        };

        // `::first-line` (css-pseudo-4 §2.1): restyle the runs that landed on
        // the dynamically-determined first formatted line.
        if let Some(fl) = first_line_style {
            crate::layout::helpers::apply_first_line_style(&mut lines, fl, env.fonts);
            apply_first_line_letter_spacing(&mut lines, fl.letter_spacing);
        }

        if style.writing_mode.is_vertical() && style.text_orientation_upright {
            lines = upright_lines(&lines);
        }
        if style.writing_mode.is_vertical() {
            for line in &mut lines {
                line.metadata.writing_mode = style.writing_mode;
                line.metadata.text_orientation_upright = style.text_orientation_upright;
            }
        }

        if let Some(max_lines) = line_clamp {
            apply_line_clamp(&mut lines, max_lines, inner_width, env.fonts);
        }
        apply_text_align_last(&mut lines, style, inner_width, env.fonts);
        let scaled_padding = layout_padding;
        let scaled_border = LayoutBorder::from_computed(&style.border, style.color);
        // `white-space: nowrap` suppresses wrapping; it does not scale the box or
        // its contents. With visible overflow the line simply paints beyond the
        // used width and the containing page/ancestor decides whether to clip it.
        let render_block_w = if shrink_to_fit {
            (measure_lines_width(&lines, env.fonts)
                + scaled_padding.horizontal()
                + scaled_border.horizontal_width())
            .min(block_w)
        } else {
            block_w
        };

        // Apply text-overflow: ellipsis when overflow is hidden, white-space
        // is nowrap, and we have a fixed width.
        if style.text_overflow == TextOverflow::Ellipsis
            && style.overflow.clips()
            && (style.white_space == WhiteSpace::NoWrap || style.text_wrap_mode_nowrap)
            && style.width.is_some()
        {
            apply_text_overflow_ellipsis(&mut lines, inner_width, env.fonts, style.direction_rtl);
        }

        let inline_size = InlineSize::from_used(
            render_block_w,
            available_width,
            style.width.is_some()
                || style.percentage_sizing.width.is_some()
                || style.min_width.is_some()
                || shrink_to_fit,
        );

        // Compute clip rect — CSS overflow:hidden clips to the padding box
        // (includes padding, excludes border).
        let clip_rect = if style.overflow.clips() {
            let text_height: f32 = lines.iter().map(|l| l.height).sum();
            let padding_box_h = resolve_padding_box_height(
                text_height,
                effective_height,
                scaled_padding,
                scaled_border.widths(),
                style.box_sizing,
            );
            Some((0.0, 0.0, render_block_w, padding_box_h))
        } else {
            None
        };
        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let total_h = resolve_padding_box_height(
            text_height,
            effective_height,
            scaled_padding,
            scaled_border.widths(),
            style.box_sizing,
        );
        cb_info = make_containing_block(total_h);

        // Resolve containing block and offsets for absolute elements.
        // `resolve_abs_containing_block` measures bottom/right insets to the box's
        // border-box edge (`cb.height - elem_height - bottom`), so pass the
        // *border-box* height/width — `total_h` is the padding box, so add the
        // vertical border back (width `block_w` is already border-box).
        let (elem_cb, mut resolved_top, mut resolved_left) = resolve_abs_containing_block(
            style,
            abs_containing_block,
            total_h + scaled_border.vertical_width(),
            render_block_w,
        );
        if style.position.is_relative() {
            let height_reference = percent_height_cb.map_or(available_height, |cb| cb.height);
            (resolved_top, resolved_left) =
                resolve_relative_offsets(style, percent_width_basis, height_reference);
        }

        // When this block has visual properties AND block children,
        // save the inline text for inclusion inside the wrapper instead
        // of emitting it directly.  The wrapper path will use it. In that case
        // the inline text becomes an anonymous block-level box *inside* the
        // wrapper: the wrapper (Container) paints the element's background,
        // border, padding and offsets, so this inner box must carry none of them
        // (otherwise the border/background/indent would be drawn twice — once on
        // the Container and again around the inline text).
        let anonymous = has_block_kids_for_wrapper || has_out_of_flow_children;
        let mut inline_block = TextBlock::from_style(
            lines,
            style,
            crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize {
                    width: if anonymous {
                        InlineSize::FILL_AVAILABLE
                    } else {
                        inline_size
                    },
                    height: if anonymous {
                        BlockSize::AUTO
                    } else {
                        effective_height.map_or(BlockSize::AUTO, |_| {
                            if has_definite_height {
                                BlockSize::definite(total_h)
                            } else {
                                BlockSize::minimum(total_h)
                            }
                        })
                    },
                },
                margins: if anonymous {
                    BlockMargins::ZERO
                } else {
                    BlockMargins::new(style.margin.top, style.margin.bottom)
                },
                padding: if anonymous {
                    EdgeSizes::ZERO
                } else {
                    scaled_padding
                },
                border: if anonymous {
                    LayoutBorder::default()
                } else {
                    scaled_border
                },
            },
        );
        inline_block.text.indent = text_indent;
        inline_block.semantics.heading_level = heading_level(el.tag);
        if anonymous {
            inline_block.paint = Default::default();
            inline_block.flow.float = Float::None;
            inline_block.positioning = Default::default();
        } else {
            inline_block.paint.border_radii = border_radii;
            inline_block.positioning.insets.top = resolved_top;
            inline_block.positioning.insets.left = resolved_left + auto_offset_left;
            inline_block.positioning.containing_block = elem_cb;
            inline_block.positioning.containing_block_depth = positioned_depth;
            inline_block.clipping.rect = clip_rect
                .map(|(x, y, width, height)| crate::types::Rect::from_xywh(x, y, width, height));
        }
        let inline_tb = inline_block.boxed();
        // Compute needs_wrapper early so we know whether to push the
        // TextBlock or save it for the Container wrapper path.
        let early_has_visual_for_wrapper = has_background_paint(style)
            || style.has_border_decoration()
            || !border_radii.is_zero()
            || style.mask_image.is_some()
            || !style.box_shadow.is_empty()
            || style.opacity < 1.0
            || style.mix_blend_mode != crate::style::computed::BlendMode::Normal
            || style.isolation.isolates()
            || style.filter.establishes_stacking_context
            || has_blended_block_child;
        let early_needs_wrapper = early_has_visual_for_wrapper
            || style.aspect_ratio.is_some()
            || style.height.is_some()
            || (positioned_container && (before_is_abs || after_is_abs))
            || has_out_of_flow_children
            || skip_inline_collection;
        let early_no_inline = !had_inline_runs;

        if has_block_kids_for_wrapper || has_out_of_flow_children {
            saved_inline_element = Some(inline_tb);
        } else if early_no_inline && early_needs_wrapper {
            // Don't push empty TextBlock — the wrapper path will
            // create a Container with the correct block_width.
            saved_inline_element = Some(inline_tb);
        } else {
            output.push(inline_tb);
        }
    }

    // If no inline content but the element has visual properties (background,
    // gradient, border, border-radius), emit a wrapper TextBlock so the visuals
    // are rendered.  Children are then pulled back inside via a negative-margin
    // spacer (same technique as flex column containers).
    // NB: check before runs is moved into wrap_text_runs above.
    let has_visual = has_background_paint(style)
        || style.has_border_decoration()
        || !border_radii.is_zero()
        || style.mask_image.is_some()
        || !style.box_shadow.is_empty()
        || style.opacity < 1.0
        || style.mix_blend_mode != crate::style::computed::BlendMode::Normal
        || style.isolation.isolates()
        || style.filter.establishes_stacking_context
        || has_blended_block_child;
    // A positioned element keeps its principal box even without decoration;
    // its position cannot be transferred to a normal-flow child.
    let has_abs_children = has_out_of_flow_children;
    let needs_wrapper = has_visual
        || style.position.is_positioned()
        || style.aspect_ratio.is_some()
        || style.height.is_some()
        || !layout_padding.is_zero()
        || has_inflow_block_pseudo
        || (positioned_container && (before_is_abs || after_is_abs))
        || has_abs_children;
    let no_inline_content = !had_inline_runs;

    let has_abs_pseudo = positioned_container && (before_is_abs || after_is_abs);
    let mut block_pseudos_nested = false;
    if (no_inline_content || has_block_kids_for_wrapper || has_abs_children || has_abs_pseudo)
        && needs_wrapper
        && nesting_depth < 40
    {
        // Pre-flatten children to measure total height.
        // A non-absolute block-level `::before` is the element's first in-flow
        // block child, so it is laid out inside the wrapper ahead of the
        // element's own inline content (css-content-3 §1).
        let mut child_elements: Vec<LayoutNode> = Vec::new();
        if has_block_before && !before_is_abs {
            if let Some(ps) = before_style {
                child_elements.push(build_pseudo_block(
                    ps,
                    el,
                    PseudoBoxContext::new(
                        inner_width,
                        env.fonts,
                        env.filter_defs,
                        &mut *env.resources,
                    )
                    .with_positioned_ancestor_depth(positioned_depth),
                    env.counter_state,
                    before_is_list_item,
                ));
            }
        }
        if let Some(before) = AnonymousInlineFormattingContext::new(style, inner_width, env.fonts)
            .layout_runs(std::mem::take(&mut generated_inline_before_runs))
        {
            child_elements.push(before);
        }
        // If there's saved inline content, include it as the next child.
        if let Some(inline_el) = saved_inline_element.take() {
            child_elements.push(inline_el);
        }
        let mut child_el_idx = 0;
        // Accumulate preceding element siblings so sibling combinators (`+`, `~`)
        // resolve during the cascade (these call sites previously passed `&[]`).
        let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
        let mut ib_group_wrapper: Vec<(&ElementNode, bool)> = Vec::new();
        let mut pending_inline_space = false;
        let wrapper_children = if has_table_cell_children {
            let child_cb = effective_height.map(|height| ContainingBlock {
                x: 0.0,
                width: inner_width,
                height: resolve_content_box_height(
                    height,
                    style.padding,
                    style.border.widths(),
                    style.box_sizing,
                ),
                depth: positioned_depth,
            });
            flatten_nodes(
                &el.children,
                LayoutTreeContext::new(
                    child_parent_style,
                    &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                        .with_cbs(forward_abs_cb, child_cb),
                    child_ancestors,
                )
                .with_positioned_ancestor_depth(positioned_depth),
                &mut child_elements,
                env,
            );
            &[][..]
        } else {
            el.children.as_slice()
        };
        for child in wrapper_children {
            match child {
                DomNode::Text(text) => {
                    if text.chars().any(char::is_whitespace) {
                        pending_inline_space = true;
                    }
                }
                DomNode::Element(child_el) => {
                    if inline_children.atomic_is_emitted(child_el_idx, atomic_inline_emission) {
                        pending_inline_space = false;
                    } else if inline_children.is_grouped_atomic(child_el_idx) {
                        ib_group_wrapper.push((child_el, pending_inline_space));
                        pending_inline_space = false;
                    } else {
                        // Flush any pending inline-block group
                        if !ib_group_wrapper.is_empty() {
                            #[allow(clippy::drain_collect)]
                            let taken: Vec<(&ElementNode, bool)> =
                                ib_group_wrapper.drain(..).collect();
                            layout_inline_block_group_with_spacing(
                                &taken,
                                style,
                                &ib_ctx,
                                &mut child_elements,
                                env.rules,
                                child_ancestors,
                                env.fonts,
                            );
                        }
                        pending_inline_space = false;
                        if recurses_as_layout_child(child_el.tag)
                            || inline_children.requires_independent_layout(child_el_idx)
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
                                    style.padding,
                                    style.border.widths(),
                                    style.box_sizing,
                                ),
                                depth: positioned_depth,
                            });
                            flatten_element(
                                child_el,
                                LayoutTreeContext::new(
                                    child_parent_style,
                                    &ctx.with_parent(
                                        inner_width,
                                        Some(available_height),
                                        style.font_size,
                                    )
                                    .with_cbs(forward_abs_cb, child_cb),
                                    child_ancestors,
                                )
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(
                                    ElementSiblingContext::new(child_el_idx, child_el_count)
                                        .with_neighbors(
                                            &preceding_siblings,
                                            forward_siblings(&child_sibling_list, child_el_idx),
                                        ),
                                ),
                                &mut child_elements,
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
        }
        // Flush remaining inline-block group
        if !ib_group_wrapper.is_empty() {
            #[allow(clippy::drain_collect)]
            let taken: Vec<(&ElementNode, bool)> = ib_group_wrapper.drain(..).collect();
            layout_inline_block_group_with_spacing(
                &taken,
                style,
                &ib_ctx,
                &mut child_elements,
                env.rules,
                child_ancestors,
                env.fonts,
            );
        }
        let mut generated_inline_after_runs = Vec::new();
        inline_sequence.append_after(
            &mut generated_inline_after_runs,
            env.fonts,
            env.counter_state,
            &mut *env.resources,
        );
        if let Some(after) = AnonymousInlineFormattingContext::new(style, inner_width, env.fonts)
            .layout_runs(generated_inline_after_runs)
        {
            child_elements.push(after);
        }
        // A non-absolute block-level `::after` is the element's last in-flow
        // block child, laid out inside the wrapper after all real children
        // (css-content-3 §1). Appended before height measurement and margin
        // collapsing so it contributes to the box's height.
        if has_block_after && !after_is_abs {
            if let Some(ps) = after_style {
                child_elements.push(build_pseudo_block(
                    ps,
                    el,
                    PseudoBoxContext::new(
                        inner_width,
                        env.fonts,
                        env.filter_defs,
                        &mut *env.resources,
                    )
                    .with_positioned_ancestor_depth(positioned_depth),
                    env.counter_state,
                    after_is_list_item,
                ));
            }
        }
        let child_bounds = child_overflow_bounds(&child_elements);
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
        let bfc = establishes_bfc_with_overflow(style, overflow_axes);
        crate::layout::helpers::collapse_margins_through_parent(
            &mut child_elements,
            &mut wrapper_margin_top,
            &mut wrapper_margin_bottom,
            style.padding,
            style.border.widths(),
            bfc,
            bfc || has_definite_height,
        );

        // Measure children total height. A non-BFC auto-height block does not
        // grow to enclose floated descendants; a BFC does.
        let only_floating_element_children = child_el_count > 0
            && el.children.iter().all(|child| match child {
                DomNode::Element(child_el) => {
                    let cls = child_el.class_list();
                    let cls_refs: Vec<&str> = cls.iter().map(|s| s.as_ref()).collect();
                    let child_selector_ctx = SelectorContext {
                        ancestors: child_ancestors.to_vec(),
                        ..SelectorContext::default()
                    };
                    let child_style = compute_style_with_context_with_font_metrics(
                        child_el.tag,
                        child_el.style_attr(),
                        style,
                        env.rules,
                        child_el.tag_name(),
                        &cls_refs,
                        child_el.id(),
                        &selector_attributes_with_has(child_el),
                        &child_selector_ctx,
                        env.font_metrics(),
                    );
                    child_style.float != Float::None
                }
                DomNode::Text(text) => !has_non_collapsible_text(text),
            });
        let clip_non_bfc_floats = !bfc
            && only_floating_element_children
            && matches!(
                (overflow_axes.x, overflow_axes.y),
                (LayoutOverflowKeyword::Clip, _) | (_, LayoutOverflowKeyword::Clip)
            );
        let children_h_raw: f32 = if clip_non_bfc_floats {
            0.0
        } else if !bfc {
            child_elements
                .iter()
                .filter(|child| {
                    crate::layout::paginate::element_float(child.as_ref()) == Float::None
                        && !layout_element_is_absolute(child.as_ref())
                })
                .map(|child| estimate_element_height(child.as_ref()))
                .sum()
        } else {
            child_elements
                .iter()
                .map(|child| estimate_element_height(child.as_ref()))
                .sum()
        };
        // A definite `height` clamps the padding-box to that size (content
        // overflows). A `min-height`-only floor (`effective_height` set but not
        // definite) must still grow to fit taller content — pass `None` so the
        // content height is used, then apply the floor as a `max` below.
        let mut container_h = resolve_padding_box_height(
            children_h_raw,
            effective_height.filter(|_| has_definite_height),
            style.padding,
            style.border.widths(),
            style.box_sizing,
        );
        if !has_definite_height && let Some(aspect_h) = aspect_ratio_height(block_w, style) {
            container_h = container_h.max(aspect_h);
        }
        let mut max_height_clamped = false;
        if !has_definite_height {
            let padding_box_constraints = authored_height_constraints.map(|height| {
                resolve_padding_box_height(
                    0.0,
                    Some(height),
                    style.padding,
                    style.border.widths(),
                    style.box_sizing,
                )
            });
            let natural_container_h = container_h;
            container_h = padding_box_constraints.constrain(container_h);
            max_height_clamped = container_h < natural_container_h;
        }
        let clip_margin_contains_overflow = overflow_clip_margin > 0.0
            && child_bounds.min_x >= -(layout_padding.left + overflow_clip_margin)
            && child_bounds.max_x <= inner_width + layout_padding.right + overflow_clip_margin
            && child_bounds.max_y <= container_h + overflow_clip_margin;
        let mut emitted_block_w = block_w;
        if overflow_axes.x == LayoutOverflowKeyword::Visible && overflow_axes.y.clips() {
            emitted_block_w = emitted_block_w.max(
                style.border.horizontal_width()
                    + layout_padding.left
                    + child_bounds.max_x.max(inner_width)
                    + layout_padding.right,
            );
        }
        let mut emitted_container_h = container_h;
        if overflow_axes.y == LayoutOverflowKeyword::Visible && overflow_axes.x.clips() {
            emitted_container_h =
                emitted_container_h.max(child_bounds.max_y + style.padding.vertical());
        }
        let emitted_overflow = if clip_margin_contains_overflow || !overflow_axes.clips_any() {
            Overflow::Visible
        } else if overflow_axes.x == LayoutOverflowKeyword::Auto
            && overflow_axes.y == LayoutOverflowKeyword::Auto
        {
            Overflow::Auto
        } else {
            Overflow::Hidden
        };
        let emitted_overflow_x = overflow_keyword_to_computed(overflow_axes.x);
        let emitted_overflow_y = overflow_keyword_to_computed(overflow_axes.y);
        // For pseudo-element containing block sizing (abs children with
        // height:100%), collapse the first/last children's outer margins
        // through the parent when no padding/border blocks them. The
        // rendered container height still uses the raw sum so surrounding
        // flow layout is unchanged.
        let cb_children_h = crate::layout::helpers::collapse_outer_child_margins(
            &child_elements,
            children_h_raw,
            style.padding,
            style.border.widths(),
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
            && style.border.top.used_width() == 0.0
        {
            child_elements.first().map_or(0.0, |child| {
                crate::layout::helpers::outer_margin_top(child.as_ref())
            })
        } else {
            0.0
        };

        // Add absolute-positioned ::before pseudo-element as a Container child.
        if let Some(ps) = before_style {
            if pseudo_is_block_like(ps) && ps.position.is_absolute() {
                let mut pseudo = build_pseudo_block(
                    ps,
                    el,
                    PseudoBoxContext::new(
                        inner_width,
                        env.fonts,
                        env.filter_defs,
                        &mut *env.resources,
                    )
                    .with_containing_block(cb_info)
                    .with_positioned_ancestor_depth(positioned_depth),
                    env.counter_state,
                    before_is_list_item,
                );
                if abs_origin_shift > 0.0 {
                    offset_text_block_top(pseudo.as_mut(), abs_origin_shift);
                }
                child_elements.push(pseudo);
            }
        }
        // Add absolute-positioned ::after pseudo-element as a Container child.
        if let Some(ps) = after_style {
            if pseudo_is_block_like(ps) && ps.position.is_absolute() {
                let mut pseudo = build_pseudo_block(
                    ps,
                    el,
                    PseudoBoxContext::new(
                        inner_width,
                        env.fonts,
                        env.filter_defs,
                        &mut *env.resources,
                    )
                    .with_containing_block(cb_info)
                    .with_positioned_ancestor_depth(positioned_depth),
                    env.counter_state,
                    after_is_list_item,
                );
                if abs_origin_shift > 0.0 {
                    offset_text_block_top(pseudo.as_mut(), abs_origin_shift);
                }
                child_elements.push(pseudo);
            }
        }

        // Patch absolute children with the now-known containing block,
        // and resolve bottom/right offsets into top/left.
        if let Some(cb) = cb_info {
            resolve_absolute_descendants_containing_block(&mut child_elements, cb);
        }

        if (style.opacity < 1.0 || style.isolation.isolates())
            && style.background_color.is_none()
            && !style.has_border_decoration()
            && style.box_shadow.is_empty()
            && !BackgroundFields::from_style(style).has_image()
        {
            clear_first_backdropless_descendant_blend(&mut child_elements);
        }
        // Resolve containing block and offsets for absolute elements.
        // Pass the border-box height (`container_h` is the padding box) so a
        // bottom-anchored absolute box measures to its border edge, not 1 border
        // width too low.
        let (wrapper_cb, mut wrapper_top, mut wrapper_left) = resolve_abs_containing_block(
            style,
            abs_containing_block,
            container_h + style.border.vertical_width(),
            block_w,
        );
        if style.position.is_relative() {
            let height_reference = percent_height_cb.map_or(available_height, |cb| cb.height);
            (wrapper_top, wrapper_left) =
                resolve_relative_offsets(style, percent_width_basis, height_reference);
        }
        // Emit a Container element with true parent-child nesting.
        // The renderer draws background/border, then renders children inside.
        let wrapper_size = crate::layout::elements::LayoutSize {
            width: InlineSize::from_used(
                emitted_block_w,
                available_width,
                has_explicit_width || style.aspect_ratio.is_some() || style.position.is_absolute(),
            ),
            height: if clip_non_bfc_floats || has_definite_height || max_height_clamped {
                BlockSize::definite(emitted_container_h + style.border.vertical_width())
            } else if style.aspect_ratio.is_some() {
                BlockSize::definite(emitted_container_h)
            } else if effective_height.is_some() {
                BlockSize::minimum(emitted_container_h + style.border.vertical_width())
            } else {
                BlockSize::AUTO
            },
        };
        let mut paint = crate::layout::elements::BoxPaint::from_style(style, wrapper_size);
        paint.border_radii = border_radii;
        output.push(
            Container {
                children: child_elements,
                box_model: crate::layout::elements::BoxModel {
                    size: wrapper_size,
                    margins: BlockMargins::new(wrapper_margin_top, wrapper_margin_bottom),
                    padding: layout_padding,
                    border: LayoutBorder::from_computed(&style.border, style.color),
                },
                paint,
                flow: crate::layout::elements::BlockFlow {
                    float: style.float,
                    clear: style.clear,
                },
                positioning: crate::layout::elements::Positioning::from_style(style)
                    .with_resolved_insets(EdgeSizes::new(
                        wrapper_top,
                        style.right.unwrap_or_default(),
                        style.bottom.unwrap_or_default(),
                        wrapper_left + auto_offset_left,
                    ))
                    .with_containing_block(wrapper_cb)
                    .with_containing_block_depth(positioned_depth),
                fragmentation: crate::layout::elements::BoxFragmentation::from_style(style),
                overflow: crate::layout::elements::OverflowBehavior {
                    combined: emitted_overflow,
                    x: emitted_overflow_x,
                    y: emitted_overflow_y,
                },
            }
            .boxed(),
        );
        block_pseudos_nested = has_any_block_pseudo;
    } else {
        // Compute cb_info for positioned containers in the non-wrapper path
        // so that absolute children get a containing block.
        if cb_info.is_none() && positioned_container {
            let h = effective_height.unwrap_or(0.0);
            cb_info = make_containing_block(h);
        }
        let mut child_el_idx = 0;
        let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
        let mut ib_group: Vec<(&ElementNode, bool)> = Vec::new();
        let mut pending_inline_space = false;
        let direct_children = if has_table_cell_children {
            flatten_nodes(
                &el.children,
                LayoutTreeContext::new(
                    child_parent_style,
                    &ctx.with_parent(inner_width, Some(available_height), style.font_size)
                        .with_cbs(forward_abs_cb, cb_info),
                    child_ancestors,
                )
                .with_positioned_ancestor_depth(positioned_depth),
                output,
                env,
            );
            &[][..]
        } else {
            el.children.as_slice()
        };
        for child in direct_children {
            match child {
                DomNode::Text(text) => {
                    if text.chars().any(char::is_whitespace) {
                        pending_inline_space = true;
                    }
                }
                DomNode::Element(child_el) => {
                    if inline_children.atomic_is_emitted(child_el_idx, atomic_inline_emission) {
                        pending_inline_space = false;
                    } else if inline_children.is_grouped_atomic(child_el_idx) {
                        ib_group.push((child_el, pending_inline_space));
                        pending_inline_space = false;
                    } else {
                        // Flush any pending inline-block group
                        if !ib_group.is_empty() {
                            #[allow(clippy::drain_collect)]
                            let taken: Vec<(&ElementNode, bool)> = ib_group.drain(..).collect();
                            layout_inline_block_group_with_spacing(
                                &taken,
                                style,
                                &ib_ctx,
                                output,
                                env.rules,
                                child_ancestors,
                                env.fonts,
                            );
                        }
                        pending_inline_space = false;
                        if recurses_as_layout_child(child_el.tag)
                            || inline_children.requires_independent_layout(child_el_idx)
                        {
                            flatten_element(
                                child_el,
                                LayoutTreeContext::new(
                                    child_parent_style,
                                    &ctx.with_parent(
                                        inner_width,
                                        Some(available_height),
                                        style.font_size,
                                    )
                                    .with_cbs(forward_abs_cb, cb_info),
                                    child_ancestors,
                                )
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(
                                    ElementSiblingContext::new(child_el_idx, child_el_count)
                                        .with_neighbors(
                                            &preceding_siblings,
                                            forward_siblings(&child_sibling_list, child_el_idx),
                                        ),
                                ),
                                output,
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
        }
        // Flush remaining inline-block group
        if !ib_group.is_empty() {
            #[allow(clippy::drain_collect)]
            let taken: Vec<(&ElementNode, bool)> = ib_group.drain(..).collect();
            layout_inline_block_group_with_spacing(
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
        && style.border.top.used_width() == 0.0
        && style.border.bottom.used_width() == 0.0
        && !overflow_axes.clips_any()
        && !style.position.is_absolute()
        && style.float == Float::None;
    if self_collapsing && (style.margin.top != 0.0 || style.margin.bottom != 0.0) {
        // The box's own top and bottom margins collapse together first.
        let collapsed = if style.margin.top >= 0.0 && style.margin.bottom >= 0.0 {
            style.margin.top.max(style.margin.bottom)
        } else if style.margin.top < 0.0 && style.margin.bottom < 0.0 {
            style.margin.top.min(style.margin.bottom)
        } else {
            style.margin.vertical()
        };
        // Carry the collapsed margin as the placeholder's top margin so the
        // preceding-sibling collapse merges it; bottom margin is 0 so the
        // following sibling collapses against this zero-height box's bottom edge,
        // yielding a single collapsed gap rather than the sum of both.
        let mut spacer = TextBlock::empty_spacer();
        spacer.box_model.margins.start = collapsed;
        output.push(spacer.boxed());
    }

    // Emit block-level ::after pseudo-element (inside block path)
    if !block_pseudos_nested {
        push_block_pseudo(
            output,
            after_style,
            el,
            PseudoBoxContext::new(inner_width, env.fonts, env.filter_defs, &mut *env.resources)
                .with_containing_block(cb_info)
                .with_positioned_ancestor_depth(positioned_depth),
            env.counter_state,
        );
    }
}

fn apply_line_clamp(
    lines: &mut Vec<crate::layout::engine::TextLine>,
    max_lines: usize,
    max_width: f32,
    fonts: &HashMap<String, TtfFont>,
) {
    if max_lines == 0 || lines.len() <= max_lines {
        return;
    }
    lines.truncate(max_lines);
    let Some(line) = lines.last_mut() else {
        return;
    };
    if line.runs.is_empty() {
        return;
    }
    let template = line.runs[0].clone();
    let full_text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
    let ellipsis = "...";
    let ellipsis_width = estimate_word_width(
        ellipsis,
        template.font_size,
        &template.font_family,
        template.bold,
        template.font_style.is_slanted(),
        fonts,
    );
    let mut truncated = String::new();
    for ch in full_text.chars() {
        truncated.push(ch);
        let width = estimate_word_width(
            &truncated,
            template.font_size,
            &template.font_family,
            template.bold,
            template.font_style.is_slanted(),
            fonts,
        );
        if width + ellipsis_width > max_width {
            truncated.pop();
            break;
        }
    }
    while truncated.ends_with(char::is_whitespace) {
        truncated.pop();
    }
    truncated.push_str(ellipsis);
    line.runs = vec![TextRun {
        text: truncated,
        ..template
    }];
}

fn apply_text_align_last(
    lines: &mut [crate::layout::engine::TextLine],
    style: &ComputedStyle,
    inner_width: f32,
    fonts: &HashMap<String, TtfFont>,
) {
    let Some(align) = style.text_align_last else {
        return;
    };
    let Some(line) = lines.last_mut() else {
        return;
    };
    let line_width = crate::layout::helpers::measure_runs_width(&line.runs, fonts);
    line.x_offset += match align {
        TextAlign::Center => ((inner_width - line_width) / 2.0).max(0.0),
        TextAlign::Right => (inner_width - line_width).max(0.0),
        TextAlign::Left | TextAlign::Justify => 0.0,
    };
}

#[derive(Debug, Clone, Copy)]
struct ChildOverflowBounds {
    min_x: f32,
    max_x: f32,
    max_y: f32,
}

fn child_overflow_bounds(children: &[LayoutNode]) -> ChildOverflowBounds {
    let mut bounds = ChildOverflowBounds {
        min_x: 0.0,
        max_x: 0.0,
        max_y: 0.0,
    };
    for child in children {
        let (x, width) = child_x_and_width(child.as_ref());
        bounds.min_x = bounds.min_x.min(x);
        bounds.max_x = bounds.max_x.max(x + width);
        bounds.max_y = bounds.max_y.max(estimate_element_height(child.as_ref()));
    }
    bounds
}

fn overflow_keyword_to_computed(value: LayoutOverflowKeyword) -> Overflow {
    match value {
        LayoutOverflowKeyword::Visible => Overflow::Visible,
        LayoutOverflowKeyword::Clip | LayoutOverflowKeyword::Hidden => Overflow::Hidden,
        LayoutOverflowKeyword::Scroll => Overflow::Scroll,
        LayoutOverflowKeyword::Auto => Overflow::Auto,
    }
}

fn layout_element_is_absolute(element: &dyn LayoutElement) -> bool {
    element
        .positioning_owner()
        .is_some_and(|owner| owner.positioning().scheme.is_absolute())
}

fn child_x_and_width(child: &dyn LayoutElement) -> (f32, f32) {
    #[derive(Default)]
    struct InlineGeometry {
        x: f32,
        width: f32,
    }

    impl LayoutVisitor for InlineGeometry {
        fn visit_text_block(&mut self, element: &TextBlock) {
            let line_width = element
                .lines
                .iter()
                .flat_map(|line| &line.runs)
                .map(|run| {
                    crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
                })
                .sum::<f32>();
            self.x = element.positioning.insets.left;
            self.width = element.box_model.size.width.fixed_value().unwrap_or(
                line_width
                    + element.box_model.padding.horizontal()
                    + element.box_model.border.horizontal_width(),
            );
        }

        fn visit_container(&mut self, element: &Container) {
            self.x = element.positioning.insets.left;
            self.width = element
                .box_model
                .size
                .width
                .fixed_value()
                .unwrap_or_default();
        }

        fn visit_image(&mut self, element: &Image) {
            self.width = element.geometry.size.width;
        }

        fn visit_svg(&mut self, element: &Svg) {
            self.width = element.geometry.size.width;
        }

        fn visit_flex_row(&mut self, element: &FlexRow) {
            self.x = element.inline_offset.value();
            self.width = element
                .box_model
                .size
                .width
                .fixed_value()
                .unwrap_or_default();
        }

        fn visit_grid_row(&mut self, element: &GridRow) {
            self.width = element.content.column_widths.iter().sum::<f32>()
                + element.box_model.padding.horizontal()
                + element.box_model.border.horizontal_width();
        }

        fn visit_table_row(&mut self, element: &TableRow) {
            self.x = element.grid_inline_offset();
            self.width = element.content.column_widths.iter().sum();
        }
    }

    let mut geometry = InlineGeometry::default();
    child.accept(&mut geometry);
    (geometry.x, geometry.width)
}
