use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    AlignContent, AlignItems, AlignSelf, BackgroundClip, BackgroundOrigin, BackgroundPosition,
    BackgroundRepeat, BackgroundSize, BoxSizing, Clear, ComputedStyle, Display, FlexDirection,
    FlexWrap, Float, JustifyContent, Overflow, OverflowWrap, Position, TextAlign, VerticalAlign,
    Visibility, WhiteSpace, compute_style_with_context,
};

use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    BackgroundFields, FlexCell, LayoutBorder, LayoutElement, TextLine, TextRun,
    aspect_ratio_height, background_svg_for_style, collects_as_inline_text, flatten_element,
    has_background_paint, measure_runs_width, pseudo_is_block_like, push_block_pseudo,
    resolve_padding_box_height,
};
use super::paginate::estimate_element_height;
use super::text::{
    FlexTextRunCollector, TextWrapOptions, estimate_word_width, resolved_line_height_factor,
    wrap_text_runs,
};

/// Each child is laid out as a TextBlock at a computed position. The container
/// emits one TextBlock per flex item with an `offset_left` / `offset_top` that
/// encodes its position inside the flex row/column. The container itself emits
/// a wrapper TextBlock for its background/border first, then the items.
#[allow(clippy::too_many_arguments)]
/// Max-content border-box width of any `<table>` laid out inside a flex item's
/// flattened content (recursing through the `Container` the item flattens to).
/// Used to shrink-wrap a `flex: 0 0 auto` item around a nested table's intrinsic
/// grid (Chrome max-content-sizes such items). Returns 0 when there is no table.
fn flex_probe_table_extent(elements: &[LayoutElement]) -> f32 {
    let mut max_w = 0.0f32;
    for e in elements {
        match e {
            LayoutElement::TableRow {
                offset_left, cells, ..
            } => {
                // Collapsed outer borders paint half inside the table box, so add
                // them back for the painted border-box width (table.rs box_width).
                let outer = cells.first().map_or(0.0, |c| c.border.left.width) / 2.0
                    + cells.last().map_or(0.0, |c| c.border.right.width) / 2.0;
                let w = *offset_left + crate::layout::paginate::table_row_content_width(e) + outer;
                max_w = max_w.max(w);
            }
            LayoutElement::Container { children, .. } => {
                max_w = max_w.max(flex_probe_table_extent(children));
            }
            _ => {}
        }
    }
    max_w
}

/// Content-based minimum main-axis (inline) size of a run of inline text — the
/// width of the longest unbreakable piece. For wrappable text that is the widest
/// single word; for `white-space: nowrap` / `pre` the run cannot soft-wrap so its
/// whole width is unbreakable. This is the "content size suggestion" used to
/// resolve a flex item's automatic minimum size (css-flexbox-1 §4.5), so a
/// shrinking item never collapses below its content.
fn flex_text_min_content(
    runs: &[TextRun],
    nowrap: bool,
    fonts: &std::collections::HashMap<String, crate::parser::ttf::TtfFont>,
) -> f32 {
    let mut total = 0.0f32;
    for run in runs {
        // Atomic inline boxes (images) are unbreakable; use their outer width.
        if let Some(inline) = run.inline_box.as_deref() {
            total += inline.outer_width();
            continue;
        }
        let space_w = estimate_word_width(
            " ",
            run.font_size,
            &run.font_family,
            run.bold,
            run.italic,
            fonts,
        );
        let mut whole = 0.0f32;
        let mut longest = 0.0f32;
        for (i, word) in run.text.split_whitespace().enumerate() {
            let ww = estimate_word_width(
                word,
                run.font_size,
                &run.font_family,
                run.bold,
                run.italic,
                fonts,
            );
            if i > 0 {
                whole += space_w;
            }
            whole += ww;
            longest = longest.max(ww);
        }
        total += if nowrap { whole } else { longest };
    }
    total
}

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

    // Horizontal offset of the flex container's border box from the containing
    // block's content-left edge. A flex container is a block-level box, so it
    // honours its own `margin-left` (and `margin: 0 auto` centering) exactly
    // like `block.rs` does for normal blocks. Without this the top-level
    // renderer painted every flex container flush at the page content-left,
    // dropping its horizontal margin (vertical margin was already applied via
    // `margin_top`/`margin_bottom`). Centering only applies when the container
    // has a definite width narrower than the available space.
    let has_explicit_width = style.width.is_some()
        || style.max_width.is_some()
        || style.min_width.is_some()
        || style.percentage_sizing.width.is_some();
    let h_offset = if has_explicit_width && block_w < available_width {
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

    // Content width for flex main-axis distribution. `block_w` is the BORDER-box
    // width under `box-sizing: border-box` (so subtract border AND padding to reach
    // the content box); under `content-box` it already excludes the border.
    let inner_width = if style.box_sizing == BoxSizing::BorderBox {
        block_w - style.border.horizontal_width() - style.padding.left - style.padding.right
    } else {
        block_w - style.padding.left - style.padding.right
    };

    // Resolve percentage border-radius for flex containers
    let resolved_border_radius = if let Some(pct) = style.border_radius_pct {
        let dim = style.height.map_or(block_w, |h| block_w.min(h));
        dim * pct / 100.0
    } else {
        style.border_radius
    };

    // Collect child elements and lay each one out into a temporary buffer.
    // Per CSS Flexbox §4.1, an absolutely-positioned child of a flex container
    // does NOT participate in flex layout (it is taken out of flow). We collect
    // such children separately and emit them as positioned boxes anchored to the
    // flex container's padding box, while the in-flow children become flex items.
    let all_child_elements: Vec<&ElementNode> = el
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
    let total_child_count = all_child_elements.len();
    // Identify which children are absolutely/fixed positioned (out of flow). We
    // compute each child's position against the container style here; full styles
    // for in-flow items are recomputed in the item loop below.
    let child_is_abs: Vec<bool> = all_child_elements
        .iter()
        .enumerate()
        .map(|(idx, child_el)| {
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: ancestors.to_vec(),
                child_index: idx,
                sibling_count: total_child_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let cs = compute_style_with_context(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
            );
            cs.position == Position::Absolute
        })
        .collect();
    let has_abs_children = child_is_abs.iter().any(|&b| b);
    // In-flow flex items (abs children excluded).
    let child_elements: Vec<&ElementNode> = all_child_elements
        .iter()
        .zip(child_is_abs.iter())
        .filter(|(_, is_abs)| !**is_abs)
        .map(|(e, _)| *e)
        .collect();

    let child_count = child_elements.len();

    // Lay out absolutely-positioned children (out of flow) against this flex
    // container's padding box. The container establishes a containing block when
    // it is positioned or transformed; otherwise the abs child resolves against
    // an ancestor and we leave its CB unstamped (forwarded by the renderer).
    let establishes_cb = crate::layout::helpers::establishes_containing_block(style);
    let abs_cb_depth = if establishes_cb { positioned_depth } else { 0 };
    let mut abs_output: Vec<LayoutElement> = Vec::new();
    if has_abs_children {
        // Containing block = the flex container's PADDING box (CSS abs CB). Its
        // height (for `bottom` resolution) is the padding-box height: content
        // height + vertical padding. Unknown auto heights resolve to 0.
        let content_h = style
            .height
            .map(|h| {
                if style.box_sizing == BoxSizing::BorderBox {
                    (h - style.border.vertical_width() - style.padding.top - style.padding.bottom)
                        .max(0.0)
                } else {
                    h
                }
            })
            .unwrap_or(0.0);
        let cb_padding_box_height = content_h + style.padding.top + style.padding.bottom;
        let cb_padding_box_width = inner_width.max(0.0) + style.padding.left + style.padding.right;
        let cb = ContainingBlock {
            // PADDING-box of the flex container (the CSS containing block for abs
            // children): `top/left/right/bottom` and percentages resolve against
            // it. x = padding-box left relative to the page content-left =
            // container margin/centering + border (NOT padding); the renderer
            // anchors abs children at this x and adds their resolved left offset.
            x: h_offset + style.border.left.width,
            width: cb_padding_box_width,
            height: cb_padding_box_height,
            depth: abs_cb_depth,
        };
        for (idx, child_el) in all_child_elements.iter().enumerate() {
            if !child_is_abs[idx] {
                continue;
            }
            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(AncestorInfo {
                element: child_el,
                child_index: idx,
                sibling_count: total_child_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            let child_ctx = ctx
                .with_parent_and_basis(
                    inner_width.max(0.0),
                    inner_width.max(0.0),
                    Some(content_h.max(1.0)),
                    style.font_size,
                )
                .with_containing_block(Some(cb));
            let mut buf: Vec<LayoutElement> = Vec::new();
            flatten_element(
                child_el,
                style,
                &child_ctx,
                &mut buf,
                None,
                &child_ancestors,
                positioned_depth,
                idx,
                total_child_count,
                &[],
                &[],
                env,
            );
            crate::layout::helpers::patch_absolute_children_containing_block(&mut buf, cb);
            // The flex container is emitted at the top level (a sibling of its
            // FlexRow), so its abs children render through the top-level path,
            // which positions an abs box at `page_content_left + offset_left`
            // (Containers) / `page_content_left + cb.x + offset_left` (TextBlocks).
            // To anchor to the flex container's padding box regardless of element
            // type, bake the padding-box left (`cb.x`) into each child's
            // `offset_left` and zero the stamped `cb.x` so both paths agree.
            for el in &mut buf {
                match el {
                    LayoutElement::Container {
                        offset_left,
                        containing_block,
                        ..
                    }
                    | LayoutElement::TextBlock {
                        offset_left,
                        containing_block,
                        ..
                    } => {
                        *offset_left += cb.x;
                        if let Some(c) = containing_block {
                            c.x = 0.0;
                        }
                    }
                    _ => {}
                }
            }
            abs_output.extend(buf);
        }
    }

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
            || !style.box_shadow.is_empty()
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
                box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
                orphans: 2,
                widows: 2,
                lines: Vec::new(),
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                writing_mode: crate::style::computed::WritingMode::HorizontalTb,
                background_color: bg,
                padding_top: style.padding.top,
                padding_bottom: style.padding.bottom,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::from_computed(&style.border),
                block_width: Some(block_w),
                block_height: Some(container_h),
                opacity: style.opacity,
                mix_blend_mode: style.mix_blend_mode,
                background_blend_mode: style.background_blend_mode,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow.clone(),
                visible: style.visibility == Visibility::Visible,
                clip_rect: if style.overflow.clips() {
                    Some((0.0, 0.0, block_w, container_h))
                } else {
                    None
                },
                transform: style.transform,
                transform_origin: style.transform_origin,
                border_radius: resolved_border_radius,
                border_radii: [resolved_border_radius; 4],
                border_radii_y: [resolved_border_radius; 4],
                outline_offset: 0.0,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: 0.0,
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
        output.append(&mut abs_output);
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
        /// Whether the item has an explicit cross-axis size (width for a column
        /// container). `align-items: stretch` must NOT stretch such items.
        has_explicit_width: bool,
        /// Whether the item has an explicit `height`. For a row container the
        /// cross axis is the block axis, so `align-items: stretch` must NOT
        /// stretch an item that already has a definite height.
        has_explicit_height: bool,
        /// Per-item `align-self` (cross-axis override; `Auto` defers to the
        /// container's `align-items`).
        align_self: AlignSelf,
        /// CSS `order`. Items are placed by ascending order, document order
        /// breaking ties.
        order: i32,
        /// Index of the originating child element (document order). Used to map
        /// a (possibly reordered) item back to its `child_elements` entry.
        child_idx: usize,
        /// Min/max clamps on the OUTER (border-box) main size. `min-width` /
        /// `max-width` for a row container, `min-height` / `max-height` for a
        /// column container. `max_main` is `f32::INFINITY` when unconstrained.
        /// Grow and shrink both clamp each item to `[min_main, max_main]`.
        min_main: f32,
        max_main: f32,
        /// Min/max clamps on the OUTER (border-box) CROSS size. `min-height` /
        /// `max-height` for a row container, `min-width` / `max-width` for a
        /// column container. A stretched item's cross size is clamped to
        /// `[cross_min, cross_max]` (css-flexbox-1 §9.4 step 11); a non-stretch
        /// item's used cross size also honors them. `cross_max` is
        /// `f32::INFINITY` when unconstrained.
        cross_min: f32,
        cross_max: f32,
        /// Whether this item is itself a flex container (`display: flex`). Such an
        /// item establishes an independent formatting context and its sub-layout
        /// (`elements`) carries each inner box's own geometry/background. It must
        /// therefore be routed through `nested_elements` for the renderer to paint
        /// every inner box, NOT collapsed by the lossy text-merge path (which kept
        /// only the first box's background and dropped the rest).
        is_flex_container: bool,
        /// `auto` on the item's main-axis leading / trailing margin
        /// (margin-left/right for a row container). Per css-flexbox-1 §8.1 these
        /// absorb positive free space equally and override `justify-content`.
        margin_main_start_auto: bool,
        margin_main_end_auto: bool,
        /// Fixed (non-auto) main-axis leading / trailing margin in px and the
        /// cross-axis leading margin (margin-left/right/top for a row
        /// container). Flex-item margins are honored during placement: the
        /// main-axis margins offset the cursor, the cross-axis leading margin
        /// shifts the item within its line.
        margin_main_start: f32,
        margin_main_end: f32,
        margin_cross_start: f32,
        /// `position: relative` paint-time offset resolved to physical
        /// left/top px (right/bottom mapped to negatives). The item still takes
        /// part in flex layout at its static position; only its painted cell is
        /// shifted, and it is flagged positioned so it paints above in-flow
        /// siblings (css-flexbox-1 §4 / CSS 2.1 §9.9.1).
        rel_left: f32,
        rel_top: f32,
        is_relative: bool,
        z_index: i32,
    }

    // Resolve an item's outer (border-box) main-axis min/max clamps from its
    // computed min/max width-or-height for the container's main axis. Content-
    // box values are inflated by the item's padding+border so the clamp applies
    // to the border-box main size used throughout flex resolution.
    let main_min_max = |child_style: &ComputedStyle| -> (f32, f32) {
        let extra = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.left
                + child_style.padding.right
                + child_style.border.horizontal_width()
        } else {
            0.0
        };
        let extra_v = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.top
                + child_style.padding.bottom
                + child_style.border.vertical_width()
        } else {
            0.0
        };
        if style.flex_direction.is_row() {
            let min = child_style.min_width.map_or(0.0, |v| v + extra);
            let max = child_style.max_width.map_or(f32::INFINITY, |v| v + extra);
            (min, max)
        } else {
            let min = child_style.min_height.map_or(0.0, |v| v + extra_v);
            let max = child_style
                .max_height
                .map_or(f32::INFINITY, |v| v + extra_v);
            (min, max)
        }
    };

    // Resolve an item's outer (border-box) CROSS-axis min/max clamps: the
    // opposite axis from `main_min_max`. For a row container the cross axis is
    // the block axis (min/max-height); for a column container it is the inline
    // axis (min/max-width). These clamp the used cross size — both the stretched
    // size (css-flexbox-1 §9.4 step 11) and a non-stretch item's cross size.
    let cross_min_max = |child_style: &ComputedStyle| -> (f32, f32) {
        let extra_h = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.left
                + child_style.padding.right
                + child_style.border.horizontal_width()
        } else {
            0.0
        };
        let extra_v = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.top
                + child_style.padding.bottom
                + child_style.border.vertical_width()
        } else {
            0.0
        };
        if style.flex_direction.is_row() {
            let min = child_style.min_height.map_or(0.0, |v| v + extra_v);
            let max = child_style
                .max_height
                .map_or(f32::INFINITY, |v| v + extra_v);
            (min, max)
        } else {
            let min = child_style.min_width.map_or(0.0, |v| v + extra_h);
            let max = child_style.max_width.map_or(f32::INFINITY, |v| v + extra_h);
            (min, max)
        }
    };

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
            following_siblings: Vec::new(),
            is_empty: false,
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

        // Auto margins on a flex item (css-flexbox-1 §8.1). Map the four physical
        // `auto` flags onto the container's main/cross axes. Main-axis autos are
        // carried on the `FlexItem` and absorb main free space during placement;
        // cross-axis autos override `align-self` here: both → center, a single
        // leading auto → push to the cross-end, a single trailing auto → cross-start.
        // (Per §8.3 a cross auto margin also suppresses `align-items: stretch`,
        // which the Center/FlexEnd/FlexStart mapping does implicitly.)
        let (m_main_start_auto, m_main_end_auto, m_cross_start_auto, m_cross_end_auto) =
            if style.flex_direction.is_row() {
                (
                    child_style.margin_left_auto,
                    child_style.margin_right_auto,
                    child_style.margin_top_auto,
                    child_style.margin_bottom_auto,
                )
            } else {
                (
                    child_style.margin_top_auto,
                    child_style.margin_bottom_auto,
                    child_style.margin_left_auto,
                    child_style.margin_right_auto,
                )
            };
        let item_align_self = if m_cross_start_auto && m_cross_end_auto {
            AlignSelf::Center
        } else if m_cross_start_auto {
            AlignSelf::FlexEnd
        } else if m_cross_end_auto {
            AlignSelf::FlexStart
        } else {
            child_style.align_self
        };

        // Fixed (non-auto) flex-item margins mapped onto the container's axes.
        // An `auto` margin contributes 0 here (the auto flag drives it instead).
        let (m_main_start, m_main_end, m_cross_start) = if style.flex_direction.is_row() {
            (
                child_style.margin.left,
                child_style.margin.right,
                child_style.margin.top,
            )
        } else {
            (
                child_style.margin.top,
                child_style.margin.bottom,
                child_style.margin.left,
            )
        };
        // `position: relative` offsets on a flex item. `left`/`top` win over
        // `right`/`bottom`; an unset axis is 0. The item lays out statically and
        // is painted shifted by these deltas.
        let item_is_relative = child_style.position == Position::Relative;
        let (item_rel_left, item_rel_top) = if item_is_relative {
            (
                child_style
                    .left
                    .or_else(|| child_style.right.map(|r| -r))
                    .unwrap_or(0.0),
                child_style
                    .top
                    .or_else(|| child_style.bottom.map(|b| -b))
                    .unwrap_or(0.0),
            )
        } else {
            (0.0, 0.0)
        };

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
        // Resolve a percentage `flex-basis` against the container's main-axis
        // content size. For a row container that is `inner_width`; for a column
        // container the main axis is the (often indefinite) height, where a
        // percentage basis behaves like `auto`, so we only resolve it for row
        // direction. The resolved length then feeds the same path as an explicit
        // `flex-basis` length.
        // `flex-basis` (and a percentage basis) is a MAIN-axis base size. For a
        // ROW container the main axis is inline, so the basis feeds the item's
        // width. For a COLUMN container the main axis is the block (height) axis,
        // so the basis must NOT leak into the item's cross-axis WIDTH — doing so
        // defeated `align-items: stretch` (a `flex: 1 1 0` column item rendered
        // width 0, a `flex: 0 0 40px` item rendered 40px wide instead of filling
        // the column). The column main-axis basis is applied to the item height
        // further below (see the `!is_row()` `item_border_box_h` branch).
        let resolved_basis = if style.flex_direction.is_row() {
            match child_style.flex_basis_pct {
                Some(pct) => Some((inner_width * pct).max(0.0)),
                None => child_style.flex_basis,
            }
        } else {
            None
        };
        let has_explicit_width = resolved_basis.is_some() || child_style.width.is_some();
        let has_explicit_height = child_style.height.is_some();
        let inflate_outer = |w: f32| -> f32 {
            if child_style.box_sizing == BoxSizing::ContentBox {
                w + child_style.padding.left
                    + child_style.padding.right
                    + child_style.border.horizontal_width()
            } else {
                w
            }
        };
        // An item's outer (border-box) main size can never be smaller than its
        // own border + padding — the content box floors at 0, not the border
        // box. Under `box-sizing: border-box` a `flex-basis: 0` therefore yields
        // an outer width equal to the horizontal border + padding, NOT 0; the
        // grow free space is then `inner - Σ(these floors)` and each item's
        // final width = its floor + its share. Without this floor a bordered
        // `flex-basis: 0` item lost its border thickness from the distribution
        // (e.g. widths 78/156/78 instead of Chrome's 78.75/154.5/78.75).
        let item_box_floor = child_style.border.horizontal_width()
            + child_style.padding.left
            + child_style.padding.right;
        let child_w_initial = match resolved_basis.or(child_style.width) {
            Some(w) => inflate_outer(w).max(item_box_floor),
            None => {
                if child_style.flex_grow > 0.0 {
                    item_box_floor
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
            following_siblings: Vec::new(),
            is_empty: false,
        });

        // Two widths: child_w_for_flex is the outer main-axis size used for
        // wrapping decisions (content-box + padding + border for content-box),
        // child_w_for_layout is the content width used to lay out children so
        // percentage resolution against the parent content area is correct.
        let child_w_for_flex = match resolved_basis.or(child_style.width) {
            Some(w) => inflate_outer(w).max(item_box_floor),
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

        // flex: 0 0 auto wrapping a nested <table>: Chrome sizes the item to the
        // table's max-content (intrinsic grid), not the equal-share fallback.
        // Probe the table's intrinsic border-box width with a throwaway layout at
        // the full container width (a table doesn't stretch, so it settles at its
        // grid width), then hug it — only ever shrinking below the equal-share
        // width, never growing. With grow:0 the base width is also the final
        // width, so shrinking it here is the resolved size. Guarded to nested-
        // table flex items to keep the blast radius minimal.
        let mut hugged_item_width: Option<f32> = None;
        let (child_w_for_flex, child_w_for_layout) = if item_has_block_children
            && !has_explicit_width
            && child_style.flex_grow == 0.0
            && resolved_basis.is_none()
        {
            let mut probe_buf = Vec::new();
            let probe_ctx = ctx
                .with_parent_and_basis(
                    width_for_percentages,
                    width_for_percentages,
                    Some(10000.0),
                    style.font_size,
                )
                .with_containing_block(None);
            flatten_element(
                child_el,
                style,
                &probe_ctx,
                &mut probe_buf,
                None,
                &child_ancestors,
                positioned_depth,
                idx,
                child_count,
                &[],
                &[],
                env,
            );
            let table_w = flex_probe_table_extent(&probe_buf);
            if table_w > 0.0 {
                let pad_border = child_style.padding.left
                    + child_style.padding.right
                    + child_style.border.horizontal_width();
                let hugged = (table_w + pad_border).max(item_box_floor);
                if hugged < child_w_for_flex {
                    hugged_item_width = Some(hugged);
                    let inner = (hugged
                        - child_style.padding.left
                        - child_style.padding.right
                        - child_style.border.horizontal_width())
                    .max(0.0);
                    (hugged, inner)
                } else {
                    (child_w_for_flex, child_w_for_layout)
                }
            } else {
                (child_w_for_flex, child_w_for_layout)
            }
        } else {
            (child_w_for_flex, child_w_for_layout)
        };

        // For complex flex items (with block children like <h2>, <p>, <div>),
        // use flatten_element to get a proper list of layout elements with
        // margins and structure preserved.
        if item_has_block_children {
            let mut child_elements_buf = Vec::new();
            // Percentage-height children resolve against the item's OWN definite
            // height. A height-less item has an indefinite block size during this
            // intrinsic-measurement pass, so percentage heights resolve to `auto`
            // (not against an arbitrary placeholder that would balloon the item
            // and poison the container cross size). When the item later stretches
            // (`align-items: stretch`) to a definite cross size, the percentage
            // children are re-resolved against that size (see the stretch loop).
            let item_content_height_basis: Option<f32> = if has_explicit_height {
                child_style.height.map(|h| match child_style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => (h
                        - child_style.padding.top
                        - child_style.padding.bottom
                        - child_style.border.vertical_width())
                    .max(0.0),
                })
            } else {
                None
            };
            let child_ctx = ctx
                .with_parent_and_basis(
                    child_w_for_layout,
                    width_for_percentages,
                    item_content_height_basis,
                    style.font_size,
                )
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
                &[],
                env,
            );
            // For a shrink-wrapped table item the leading Container paints the
            // item's own background/border; stamp the hugged border-box width on it
            // so it paints at the item width (flex base size), not the laid-out
            // content width. The nested table is left-aligned and intrinsic, so its
            // position is unaffected.
            if let Some(hw) = hugged_item_width {
                if let Some(LayoutElement::Container { block_width, .. }) =
                    child_elements_buf.first_mut()
                {
                    *block_width = Some(hw);
                }
            }
            // A nested flex/block container that paints its own background emits
            // a leading background TextBlock (carrying the container's full
            // padding-box `block_height`) immediately followed by a
            // negative-margin spacer that pulls the flowed children back *inside*
            // that background. In that layout the background block already
            // accounts for the children's vertical extent, so summing the
            // pulled-back children as well double-counts the column's height.
            // Detect that pattern and take the background block's border-box
            // height as the item's natural height instead.
            let self_bg_natural = match child_elements_buf.as_slice() {
                [
                    LayoutElement::TextBlock {
                        block_height: Some(bg_h),
                        border: bg_border,
                        ..
                    },
                    LayoutElement::TextBlock {
                        margin_top: spacer_mt,
                        lines: spacer_lines,
                        ..
                    },
                    ..,
                ] if *spacer_mt < 0.0 && spacer_lines.is_empty() => {
                    Some(bg_h + bg_border.vertical_width())
                }
                _ => None,
            };
            let mut child_h = self_bg_natural.unwrap_or_else(|| {
                child_elements_buf
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
                    .sum::<f32>()
            });
            if hugged_item_width.is_some() {
                child_h += child_style.border.bottom.width;
                if let Some(LayoutElement::Container { block_height, .. }) =
                    child_elements_buf.first_mut()
                {
                    *block_height = Some(child_h);
                }
            }

            items.push(FlexItem {
                elements: child_elements_buf,
                width: child_w_for_flex,
                base_width: child_w_for_flex,
                flex_grow: child_style.flex_grow,
                flex_shrink: child_style.flex_shrink,
                height: child_h,
                natural_height: child_h, // Natural height for align-items flex-start
                has_explicit_width,
                has_explicit_height,
                align_self: item_align_self,
                order: child_style.order,
                child_idx: idx,
                min_main: main_min_max(&child_style).0,
                max_main: main_min_max(&child_style).1,
                cross_min: cross_min_max(&child_style).0,
                cross_max: cross_min_max(&child_style).1,
                is_flex_container: child_style.display == Display::Flex,
                margin_main_start_auto: m_main_start_auto,
                margin_main_end_auto: m_main_end_auto,
                margin_main_start: m_main_start,
                margin_main_end: m_main_end,
                margin_cross_start: m_cross_start,
                rel_left: item_rel_left,
                rel_top: item_rel_top,
                is_relative: item_is_relative,
                z_index: child_style.z_index,
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

        // `flex-basis: content` sizes the flex base to the item's max-content
        // size, ignoring any `width` (css-flexbox-1 §7.2.3). Measure the run
        // width and inflate by padding/border, capped at the container — this
        // overrides the explicit `width` that `has_explicit_width` reflects.
        // When no explicit width/flex-basis and flex-grow is 0, measure the
        // natural (intrinsic) content width so the item shrinks to fit.
        let child_w = if child_style.flex_basis_content && !runs.is_empty() {
            let natural_text_w = measure_runs_width(&runs, env.fonts);
            let pad_h = child_style.padding.left + child_style.padding.right;
            let border_h = child_style.border.horizontal_width();
            (natural_text_w + pad_h + border_h).min(width_for_percentages)
        } else if !has_explicit_width && child_style.flex_grow == 0.0 && !runs.is_empty() {
            let natural_text_w = measure_runs_width(&runs, env.fonts);
            let pad_h = child_style.padding.left + child_style.padding.right;
            let border_h = child_style.border.horizontal_width();
            // Outer width = text + padding + border (capped at container)
            (natural_text_w + pad_h + border_h).min(width_for_percentages)
        } else {
            child_w_initial
        };

        // Automatic minimum size (css-flexbox-1 §4.5). For a row container the
        // main axis is inline, and a flex item whose `min-width` is `auto` (the
        // default) and that is not a scroll container (overflow:visible) must not
        // shrink below its content-based minimum. The used automatic minimum is
        // min(content size suggestion, specified size suggestion) clamped by the
        // item's max main size — so it never exceeds the item's own specified
        // width, and items with `min-width:0`/clipped overflow keep collapsing.
        // Only the row main axis is handled here (column main = block height is
        // left at 0 to avoid disturbing column sizing).
        let (resolved_min_main, resolved_max_main) = main_min_max(&child_style);
        let auto_min_main = if style.flex_direction.is_row()
            && child_style.min_width.is_none()
            && child_style.overflow_x == Overflow::Visible
            && child_style.overflow_y == Overflow::Visible
            && child_style.overflow_wrap != OverflowWrap::Anywhere
            && !runs.is_empty()
        {
            let nowrap = matches!(
                child_style.white_space,
                WhiteSpace::NoWrap | WhiteSpace::Pre
            );
            let content_min = flex_text_min_content(&runs, nowrap, env.fonts)
                + child_style.padding.left
                + child_style.padding.right
                + child_style.border.horizontal_width();
            let specified = if has_explicit_width {
                child_w_initial
            } else {
                f32::INFINITY
            };
            content_min.min(specified).min(resolved_max_main)
        } else {
            resolved_min_main
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
                )
                .with_rtl(child_style.direction_rtl)
                .with_bidi_override(child_style.bidi_override),
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
            conic_gradient: background_conic_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
            clip: background_clip,
        } = BackgroundFields::from_style(&child_style);
        let elem = LayoutElement::TextBlock {
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines,
            margin_top: child_style.margin.top,
            margin_bottom: child_style.margin.bottom,
            text_align: child_style.text_align,
            writing_mode: crate::style::computed::WritingMode::HorizontalTb,
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
            mix_blend_mode: child_style.mix_blend_mode,
            background_blend_mode: child_style.background_blend_mode,
            float: Float::None,
            clear: Clear::None,
            position: child_style.position,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: child_style.box_shadow.clone(),
            visible: child_style.visibility == Visibility::Visible,
            clip_rect: if child_style.overflow.clips() {
                Some((0.0, 0.0, child_w, child_h))
            } else {
                None
            },
            transform: child_style.transform,
            transform_origin: child_style.transform_origin,
            border_radius: child_style.border_radius,
            border_radii: child_style.border_radii,
            border_radii_y: child_style.border_radii_y,
            outline_offset: child_style.outline_offset,
            outline_width: child_style.outline_width,
            outline_color: child_style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: child_style.text_indent,
            letter_spacing: child_style.letter_spacing,
            word_spacing: child_style.word_spacing,
            vertical_align: child_style.vertical_align,
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
            z_index: child_style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        };

        // `child_h` is the item's *padding-box* height (the TextBlock
        // convention used for `block_height`). The flex *item*'s main- and
        // cross-axis extent is its border box, so add the border back here —
        // otherwise a `box-sizing: border-box` item with an explicit height
        // measured short by its border, collapsing wrapped-line cross sizes and
        // column main-axis spacing.
        let mut item_border_box_h = child_h + child_style.border.vertical_width();
        // For a column container the main axis is the block axis, so `flex-basis`
        // (a main-size) sets the item's height when no explicit `height` is
        // given. Without this an empty `flex-basis: 150px` column item measured
        // its content height (~0) and collapsed. A percentage basis resolves
        // against the container's main (cross_size already folds the height) —
        // we approximate it against `inner_cross_size` which is the resolved
        // content height, computed later, so only the length basis is used here.
        if !style.flex_direction.is_row() && !has_explicit_height {
            // A percentage `flex-basis` resolves against the container's inner
            // main (block) size when that size is DEFINITE (css-flexbox-1 §9.2).
            // For a column container the main size is the container's content
            // height; fall back to the length basis / content size when the
            // height is indefinite.
            let container_main_content: Option<f32> =
                style.height.map(|h| match style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => (h
                        - style.padding.top
                        - style.padding.bottom
                        - style.border.vertical_width())
                    .max(0.0),
                });
            let basis_len = child_style.flex_basis.or_else(|| {
                child_style
                    .flex_basis_pct
                    .zip(container_main_content)
                    .map(|(pct, main)| (main * pct).max(0.0))
            });
            if let Some(basis) = basis_len {
                let bb = if child_style.box_sizing == BoxSizing::ContentBox {
                    basis
                        + child_style.padding.top
                        + child_style.padding.bottom
                        + child_style.border.vertical_width()
                } else {
                    basis
                };
                item_border_box_h = bb;
            }
        }
        items.push(FlexItem {
            elements: vec![elem],
            width: child_w,
            base_width: child_w,
            flex_grow: child_style.flex_grow,
            flex_shrink: child_style.flex_shrink,
            height: item_border_box_h + child_style.margin.top + child_style.margin.bottom,
            natural_height: item_border_box_h + child_style.margin.top + child_style.margin.bottom,
            has_explicit_width,
            has_explicit_height,
            align_self: item_align_self,
            order: child_style.order,
            child_idx: idx,
            min_main: auto_min_main,
            max_main: resolved_max_main,
            cross_min: cross_min_max(&child_style).0,
            cross_max: cross_min_max(&child_style).1,
            is_flex_container: child_style.display == Display::Flex,
            margin_main_start_auto: m_main_start_auto,
            margin_main_end_auto: m_main_end_auto,
            margin_main_start: m_main_start,
            margin_main_end: m_main_end,
            margin_cross_start: m_cross_start,
            rel_left: item_rel_left,
            rel_top: item_rel_top,
            is_relative: item_is_relative,
            z_index: child_style.z_index,
        });
    }

    if items.is_empty() {
        output.append(&mut abs_output);
        return;
    }

    // Reorder items by CSS `order` (ascending), with document order breaking
    // ties. Layout/placement and visual paint order both follow `order`.
    if items.iter().any(|it| it.order != 0) {
        items.sort_by_key(|it| (it.order, it.child_idx));
    }

    let direction = style.flex_direction;
    let justify = style.justify_content;
    let align = style.align_items;
    let wrap = style.flex_wrap;
    // Resolve percentage gaps against the flex container's OWN content box (CSS
    // Box Alignment §8.3): column-gap% against the content-box inline size
    // (width), row-gap% against the content-box block size (height). The parser
    // stores these as fraction hints (`column_gap_pct`/`row_gap_pct`) precisely
    // so they bind to this box, not the parent/ICB width.
    let resolved_column_gap = match style.column_gap_pct {
        Some(frac) => (inner_width * frac).max(0.0),
        None => style.column_gap,
    };
    let resolved_row_gap = match style.row_gap_pct {
        Some(frac) => {
            let content_h = match style.height {
                Some(h) => match style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => (h
                        - style.padding.top
                        - style.padding.bottom
                        - style.border.vertical_width())
                    .max(0.0),
                },
                // Indefinite block size => percentage row-gap resolves to 0.
                None => 0.0,
            };
            (content_h * frac).max(0.0)
        }
        None => style.row_gap,
    };
    // Per-axis gaps. `column_gap` separates items along the inline axis,
    // `row_gap` along the block axis. For a row container the main-axis gap is
    // the column gap and the line (cross) gap is the row gap; for a column
    // container they swap. `style.gap` is kept as the legacy single value.
    let (main_gap, line_gap) = if direction.is_row() {
        (resolved_column_gap, resolved_row_gap)
    } else {
        (resolved_row_gap, resolved_column_gap)
    };
    // `gap` is the main-axis gap used throughout the per-line packing math.
    let gap = main_gap;
    let column_wrap_limit = if direction.is_row() {
        None
    } else {
        style.height.map(|h| match style.box_sizing {
            BoxSizing::ContentBox => h,
            BoxSizing::BorderBox => {
                (h - style.padding.top - style.padding.bottom - style.border.vertical_width())
                    .max(0.0)
            }
        })
    };

    // Group items into lines (for flex-wrap)
    struct FlexLine {
        item_indices: Vec<usize>,
        main_size: f32,
        cross_size: f32,
    }

    let mut lines: Vec<FlexLine> = Vec::new();

    match direction {
        FlexDirection::Row | FlexDirection::RowReverse => {
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

                if wrap.wraps()
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
        FlexDirection::Column | FlexDirection::ColumnReverse => {
            // In column direction the main axis is vertical. With `flex-wrap:
            // wrap` and a definite container height, items that overflow that
            // height start a new column (a new flex line on the horizontal
            // cross axis).
            let mut line = FlexLine {
                item_indices: Vec::new(),
                main_size: 0.0,
                cross_size: 0.0,
            };
            for (i, item) in items.iter().enumerate() {
                let gap_extra = if line.item_indices.is_empty() {
                    0.0
                } else {
                    gap
                };
                if wrap.wraps()
                    && !line.item_indices.is_empty()
                    && column_wrap_limit
                        .is_some_and(|max_main| line.main_size + gap_extra + item.height > max_main)
                {
                    lines.push(line);
                    line = FlexLine {
                        item_indices: Vec::new(),
                        main_size: 0.0,
                        cross_size: 0.0,
                    };
                }
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
    let total_cross: f32 = if direction.is_row() {
        lines.iter().map(|l| l.cross_size).sum::<f32>()
            + if lines.len() > 1 {
                (lines.len() - 1) as f32 * line_gap
            } else {
                0.0
            }
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.cross_size).sum::<f32>() + (lines.len() - 1) as f32 * line_gap
    } else {
        lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max)
    };

    let total_main: f32 = if direction.is_row() {
        inner_width
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.main_size).fold(0.0f32, f32::max)
    } else {
        lines.iter().map(|l| l.main_size).sum::<f32>()
    };

    let container_height = if direction.is_row() {
        total_cross
    } else {
        total_main
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
            // For a column container a definite height is the main-axis size:
            // it caps the content so flex-shrink can compress overflowing items
            // into it (use the height directly, not max with the natural sum).
            // For a row container the height is the cross size, where a taller
            // explicit height must still contain the items (keep the max).
            if direction.is_row() {
                container_h.max(target)
            } else {
                target
            }
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

    // Cross-axis stretch for nested flex containers (row direction).
    //
    // A flex item with the default `align-items: stretch` and no definite cross
    // size (here `height`) must stretch to the container's content cross size.
    // For a *nested flex container* item, that stretched height is also its main
    // size when laid out as its own column flex, so its internal
    // `justify-content` (e.g. `space-between`) distributes against the stretched
    // height — not its natural content height. The first flatten produced the
    // item at natural height; re-flatten it with the stretched height forced so
    // its inner layout (and its painted background/border) fill the cross axis.
    if direction.is_row() && lines.len() == 1 && inner_cross_size > 0.0 {
        for item in items.iter_mut() {
            let stretches = match item.align_self {
                AlignSelf::Stretch => true,
                AlignSelf::Auto => align == AlignItems::Stretch,
                _ => false,
            };
            if !stretches || item.has_explicit_height || item.height >= inner_cross_size - 0.01 {
                continue;
            }
            let child_el = child_elements[item.child_idx];
            // Only flex containers carry their own main-axis distribution that
            // depends on the stretched height. Simple items are stretched purely
            // visually by the renderer (cell_render_h = line_cross).
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: ancestors.to_vec(),
                child_index: item.child_idx,
                sibling_count: child_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let mut child_style = compute_style_with_context(
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
            // Force the item's cross size (its main size as a column flex) to the
            // stretched height. Translate the padding-box `inner_cross_size` to a
            // value the container's box-sizing interprets as that border-box.
            let forced_h = match child_style.box_sizing {
                BoxSizing::BorderBox => inner_cross_size,
                BoxSizing::ContentBox => (inner_cross_size
                    - child_style.border.vertical_width()
                    - child_style.padding.top
                    - child_style.padding.bottom)
                    .max(0.0),
            };
            child_style.height = Some(forced_h);

            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(AncestorInfo {
                element: child_el,
                child_index: item.child_idx,
                sibling_count: child_count,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            });
            // The item's own content-box dimensions once stretched: percentage
            // children resolve their heights against this definite cross size.
            let item_content_w = (item.width
                - child_style.padding.left
                - child_style.padding.right
                - child_style.border.horizontal_width())
            .max(0.0);
            let mut buf = Vec::new();
            if child_style.display == Display::Flex {
                // A nested flex container carries its own main-axis distribution
                // that depends on the stretched height; re-layout it as a flex.
                let child_ctx = ctx
                    .with_parent_and_basis(
                        item.width,
                        width_for_percentages,
                        Some(inner_cross_size),
                        style.font_size,
                    )
                    .with_containing_block(None);
                layout_flex_container(
                    child_el,
                    &child_style,
                    &child_ctx,
                    &mut buf,
                    &child_ancestors,
                    None,
                    None,
                    positioned_depth,
                    env,
                );
            } else {
                // A stretched plain block item whose block children include a
                // percentage-height box: the first (intrinsic) pass treated those
                // percentages as `auto` because the item was height-less. Now that
                // align-items:stretch gives the item a definite height, re-flatten
                // it with that height so `height: 50%` descendants resolve against
                // it. Items with only inline/text content need no re-flatten — the
                // renderer stretches their cell visually (cell_render_h = line_cross).
                let has_block_kids = child_el.children.iter().any(|c| {
                    matches!(c, DomNode::Element(e) if e.tag.is_block() && !collects_as_inline_text(e.tag))
                });
                if !has_block_kids {
                    continue;
                }
                // `flatten_element` recomputes the item's own style from its
                // attributes, so the stretched height must be injected there.
                // Clone the item and append `height:<forced_h>pt` to its inline
                // style (inline declarations win the cascade, and layout units are
                // points). `forced_h` is already expressed in the item's own
                // box-sizing. With a now-definite block size, `height:50%`
                // descendants resolve against it.
                let mut forced_el = child_el.clone();
                let mut style_decl = forced_el
                    .attributes
                    .get("style")
                    .cloned()
                    .unwrap_or_default();
                if !style_decl.trim_end().is_empty() && !style_decl.trim_end().ends_with(';') {
                    style_decl.push(';');
                }
                style_decl.push_str(&format!("height:{forced_h}pt"));
                forced_el.attributes.insert("style".to_string(), style_decl);
                let child_ctx = ctx
                    .with_parent_and_basis(
                        item_content_w,
                        width_for_percentages,
                        Some(forced_h),
                        style.font_size,
                    )
                    .with_containing_block(None);
                flatten_element(
                    &forced_el,
                    style,
                    &child_ctx,
                    &mut buf,
                    None,
                    &child_ancestors,
                    positioned_depth,
                    item.child_idx,
                    child_count,
                    &[],
                    &[],
                    env,
                );
            }
            if !buf.is_empty() {
                item.elements = buf;
                item.height = inner_cross_size;
                item.natural_height = inner_cross_size;
            }
        }
    }

    if direction.is_row() && lines.len() == 1 {
        if let Some(line) = lines.first_mut() {
            line.cross_size = line.cross_size.max(inner_cross_size);
        }
    }
    // Recompute total_cross after possibly growing a single line.
    let total_cross: f32 = if direction.is_row() {
        lines.iter().map(|l| l.cross_size).sum::<f32>()
            + if lines.len() > 1 {
                (lines.len() - 1) as f32 * line_gap
            } else {
                0.0
            }
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.cross_size).sum::<f32>() + (lines.len() - 1) as f32 * line_gap
    } else {
        lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max)
    };

    // align-content distributes wrapped flex LINES along the cross axis when the
    // container has more than one line and spare cross space. For rows the cross
    // axis is vertical; for column-wrap it is horizontal.
    let line_count = lines.len();
    let cross_axis_extent = if direction.is_row() {
        inner_cross_size
    } else {
        inner_width
    };
    let (ac_lead, ac_between, ac_line_stretch) = if line_count > 1 {
        let lines_cross: f32 = lines.iter().map(|l| l.cross_size).sum::<f32>();
        let base_gaps = (line_count - 1) as f32 * line_gap;
        // Signed cross free space — kept negative on overflow so center/flex-end
        // honor alignment past the edge (css-flexbox-1 §8.4 + css-align-3 §9).
        let ac_free = cross_axis_extent - lines_cross - base_gaps;
        let neg = ac_free < 0.0;
        // flex-wrap:wrap-reverse swaps the cross-start/cross-end edges
        // (css-flexbox-1 §5.3), so flex-start/flex-end exchange leads. The line
        // *order* is reversed separately at placement time (`line_order`).
        let effective_ac = if wrap == FlexWrap::WrapReverse {
            match style.align_content {
                AlignContent::FlexStart => AlignContent::FlexEnd,
                AlignContent::FlexEnd => AlignContent::FlexStart,
                other => other,
            }
        } else {
            style.align_content
        };
        match effective_ac {
            AlignContent::FlexStart => (0.0, 0.0, 0.0),
            AlignContent::FlexEnd => (ac_free, 0.0, 0.0),
            AlignContent::Center => (ac_free / 2.0, 0.0, 0.0),
            AlignContent::SpaceBetween => {
                // Negative free space behaves as flex-start (§8.4).
                if neg || line_count <= 1 {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.0, ac_free / (line_count - 1) as f32, 0.0)
                }
            }
            AlignContent::SpaceAround => {
                // Negative free space falls back to center (§8.4).
                if neg {
                    (ac_free / 2.0, 0.0, 0.0)
                } else {
                    let around = ac_free / line_count as f32;
                    (around / 2.0, around, 0.0)
                }
            }
            AlignContent::SpaceEvenly => {
                // Negative free space falls back to center (§8.4).
                if neg {
                    (ac_free / 2.0, 0.0, 0.0)
                } else {
                    let ev = ac_free / (line_count + 1) as f32;
                    (ev, ev, 0.0)
                }
            }
            // stretch grows each line equally to fill the spare cross space, but
            // never shrinks lines when the space is negative.
            AlignContent::Stretch => {
                if neg {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.0, 0.0, ac_free / line_count as f32)
                }
            }
        }
    } else {
        (0.0, 0.0, 0.0)
    };
    if ac_line_stretch > 0.0 {
        for line in lines.iter_mut() {
            line.cross_size += ac_line_stretch;
        }
    }
    let bg = style
        .background_color
        .map(|color: crate::types::Color| color.to_f32_rgba());

    let column_wrap_lines = !direction.is_row() && lines.len() > 1;

    // For single-line column direction, emit container background separately.
    // Multi-line column-wrap uses a FlexRow wrapper below so the items can be
    // positioned in additional columns while the container remains one flow box.
    let emitted_column_bg = !direction.is_row()
        && !column_wrap_lines
        && (has_background_paint(style) || style.border.has_any() || !style.box_shadow.is_empty());
    if emitted_column_bg {
        // Emit the container background/border as a visual element.
        // It advances y by its full height in paginate.  We then emit a
        // negative-margin spacer to pull y back so children flow *inside*
        // the background rather than after it.
        // The background block is a bordered TextBlock, so it advances the
        // cursor by its *border-box* height (`block_height` + vertical border)
        // in the flow. The pull-back spacer undoes that whole advance back to
        // the border-box top; the first item then re-adds the container's
        // top border + padding in its own leading to flow inside the box.
        let bg_flow_height = container_h + style.border.vertical_width();
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
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines: Vec::new(),
            margin_top: style.margin.top,
            margin_bottom: 0.0,
            text_align: style.text_align,
            writing_mode: crate::style::computed::WritingMode::HorizontalTb,
            background_color: bg,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(block_w),
            block_height: Some(container_h),
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            background_blend_mode: style.background_blend_mode,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow.clone(),
            visible: style.visibility == Visibility::Visible,
            clip_rect: if style.overflow.clips() {
                Some((0.0, 0.0, block_w, container_h))
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
            conic_gradient: background_conic_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
            clip: background_clip,
        } = BackgroundFields::none();
        output.push(LayoutElement::TextBlock {
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines: Vec::new(),
            margin_top: -bg_flow_height,
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
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        });
    }

    // Position items within the flex container and emit them. align-content
    // leading bumps the first line away from the cross-start edge.
    let mut cross_offset = ac_lead;
    // All flex cells across every line, merged into a single FlexRow for
    // row direction. This keeps container borders/backgrounds around every
    // wrapped line and keeps pagination flow correct.
    let mut all_flex_cells: Vec<FlexCell> = Vec::new();

    // `flex-wrap: wrap-reverse` stacks the wrapped lines from the cross-end
    // toward the cross-start, i.e. the visual line order is reversed. We keep
    // the cross_offset accumulation forward (cross-start downward) but feed the
    // lines in reversed order so the last source line lands at the top.
    let line_order: Vec<usize> = if wrap == FlexWrap::WrapReverse {
        (0..lines.len()).rev().collect()
    } else {
        (0..lines.len()).collect()
    };

    for (visual_pos, &line_idx) in line_order.iter().enumerate() {
        let line = &lines[line_idx];
        if visual_pos > 0 {
            cross_offset += ac_between;
        }
        let line_items: Vec<usize> = line.item_indices.clone();
        let line_item_count = line_items.len();

        match direction {
            FlexDirection::Row | FlexDirection::RowReverse => {
                let total_item_width: f32 = line_items.iter().map(|&i| items[i].width).sum();
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let mut free_space = inner_width - total_item_width - total_gap;

                // Flex grow: distribute positive free space proportionally,
                // iterating so items that hit their `max-main` clamp are frozen
                // and their unused share is redistributed to the rest (CSS
                // Flexbox §9.7 "Resolving Flexible Lengths").
                let total_grow: f32 = line_items.iter().map(|&i| items[i].flex_grow).sum();
                if free_space > 0.0 && total_grow > 0.0 {
                    let mut frozen = vec![false; line_items.len()];
                    // css-flexbox-1 §9.7 step 4.b: when the unfrozen items' flex
                    // factors sum to less than 1, only that fraction of the free
                    // space is distributed; the remainder stays as free space for
                    // `justify-content` instead of over-growing the items.
                    let grow_fraction = total_grow < 1.0;
                    let pool = if grow_fraction {
                        free_space * total_grow
                    } else {
                        free_space
                    };
                    let mut remaining = pool;
                    // Bounded iteration count: at most one item freezes per pass.
                    for _ in 0..=line_items.len() {
                        let active_grow: f32 = line_items
                            .iter()
                            .enumerate()
                            .filter(|(li, _)| !frozen[*li])
                            .map(|(_, &i)| items[i].flex_grow)
                            .sum();
                        if active_grow <= 0.0 || remaining <= 0.01 {
                            break;
                        }
                        let mut newly_frozen = false;
                        let mut consumed = 0.0;
                        for (li, &i) in line_items.iter().enumerate() {
                            if frozen[li] {
                                continue;
                            }
                            let share = remaining * (items[i].flex_grow / active_grow);
                            let target = items[i].width + share;
                            if target >= items[i].max_main {
                                consumed += items[i].max_main - items[i].width;
                                items[i].width = items[i].max_main;
                                frozen[li] = true;
                                newly_frozen = true;
                            } else {
                                items[i].width = target;
                                consumed += share;
                            }
                        }
                        remaining -= consumed;
                        if !newly_frozen {
                            break;
                        }
                    }
                    // Leave any undistributed space (the sum<1 remainder, plus a
                    // pool left over when every item hit its max) for justify.
                    free_space = if grow_fraction {
                        (free_space - (pool - remaining)).max(0.0)
                    } else {
                        0.0
                    };
                }

                // Flex shrink: remove overflow weighted by shrink×base, freezing
                // items that hit their `min-main` clamp and redistributing.
                if free_space < 0.0 {
                    let mut frozen = vec![false; line_items.len()];
                    // css-flexbox-1 §9.7 step 4.b (shrink): when the unfrozen
                    // items' flex-shrink factors sum to less than 1, only that
                    // fraction of the deficit is absorbed; the rest overflows.
                    let total_shrink: f32 = line_items.iter().map(|&i| items[i].flex_shrink).sum();
                    let initial_deficit = -free_space;
                    let mut deficit = if total_shrink < 1.0 {
                        initial_deficit * total_shrink
                    } else {
                        initial_deficit
                    };
                    for _ in 0..=line_items.len() {
                        let total_weight: f32 = line_items
                            .iter()
                            .enumerate()
                            .filter(|(li, _)| !frozen[*li])
                            .map(|(_, &i)| items[i].flex_shrink * items[i].base_width)
                            .sum();
                        if total_weight <= 0.0 || deficit <= 0.01 {
                            break;
                        }
                        let mut newly_frozen = false;
                        let mut removed = 0.0;
                        for (li, &i) in line_items.iter().enumerate() {
                            if frozen[li] {
                                continue;
                            }
                            let weight = items[i].flex_shrink * items[i].base_width;
                            let reduce = deficit * (weight / total_weight);
                            let target = items[i].width - reduce;
                            let floor = items[i].min_main.max(0.0);
                            if target <= floor {
                                removed += items[i].width - floor;
                                items[i].width = floor;
                                frozen[li] = true;
                                newly_frozen = true;
                            } else {
                                items[i].width = target;
                                removed += reduce;
                            }
                        }
                        deficit -= removed;
                        if !newly_frozen {
                            break;
                        }
                    }
                    // NB: the real remaining free space is recomputed from the
                    // final item widths below (for justify-content overflow
                    // handling), so we deliberately do NOT zero `free_space` here.
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
                        let child_idx = items[i].child_idx;
                        let child_el = child_elements[child_idx];
                        let has_block_kids = child_el.children.iter().any(|c| {
                            matches!(c, DomNode::Element(e) if e.tag.is_block() && !collects_as_inline_text(e.tag))
                        });
                        if has_block_kids {
                            // `final_w` is the item's resolved BORDER-box main size.
                            // `flatten_element` derives the child's block (border-box)
                            // width by adding the child's own horizontal border to the
                            // available width it is handed. Passing the border-box
                            // width verbatim therefore double-counted the child's
                            // border: a bordered auto-width child (e.g. a nested grid
                            // host) rendered `final_w + its border` wide, overflowing
                            // the flex item. Subtract the child's own horizontal
                            // border so its border-box lands exactly on `final_w`.
                            let relayout_classes = child_el.class_list();
                            let relayout_selector_ctx = SelectorContext {
                                ancestors: ancestors.to_vec(),
                                child_index: child_idx,
                                sibling_count: child_count,
                                preceding_siblings: Vec::new(),
                                following_siblings: Vec::new(),
                                is_empty: false,
                            };
                            let relayout_child_style = compute_style_with_context(
                                child_el.tag,
                                child_el.style_attr(),
                                &parent_for_children,
                                env.rules,
                                child_el.tag_name(),
                                &relayout_classes,
                                child_el.id(),
                                &child_el.attributes,
                                &relayout_selector_ctx,
                            );
                            // Only auto-width children fill the available width (and
                            // thus need the border-deduction); an explicit width
                            // resolves the child's box itself, so leave `final_w`.
                            let relayout_avail = if relayout_child_style.width.is_some() {
                                final_w
                            } else {
                                (final_w - relayout_child_style.border.horizontal_width()).max(0.0)
                            };
                            // A nested flex container must re-run its OWN flex
                            // algorithm at the final (grown) main-axis width, and —
                            // when it stretches — with the line's cross size forced
                            // as its definite height, so its flex-grow children
                            // distribute against a real main size. The generic
                            // `flatten_element` re-flatten below lays a flex item out
                            // at an indefinite height, collapsing its grow children
                            // to zero (the pre-grow stretch pass above used the
                            // ungrown width, which the grow re-flatten then clobbered).
                            if relayout_child_style.display == Display::Flex
                                && direction.is_row()
                                && inner_cross_size > 0.0
                            {
                                let mut fstyle = relayout_child_style.clone();
                                let stretches = matches!(items[i].align_self, AlignSelf::Stretch)
                                    || (matches!(items[i].align_self, AlignSelf::Auto)
                                        && align == AlignItems::Stretch);
                                if stretches && fstyle.height.is_none() {
                                    fstyle.height = Some(match fstyle.box_sizing {
                                        BoxSizing::BorderBox => inner_cross_size,
                                        BoxSizing::ContentBox => (inner_cross_size
                                            - fstyle.border.vertical_width()
                                            - fstyle.padding.top
                                            - fstyle.padding.bottom)
                                            .max(0.0),
                                    });
                                }
                                let mut fbuf = Vec::new();
                                let mut fancestors = ancestors.to_vec();
                                fancestors.push(AncestorInfo {
                                    element: child_el,
                                    child_index: child_idx,
                                    sibling_count: child_count,
                                    preceding_siblings: Vec::new(),
                                    following_siblings: Vec::new(),
                                    is_empty: false,
                                });
                                let fctx = ctx
                                    .with_parent_and_basis(
                                        final_w,
                                        width_for_percentages,
                                        Some(inner_cross_size),
                                        style.font_size,
                                    )
                                    .with_containing_block(None);
                                layout_flex_container(
                                    child_el,
                                    &fstyle,
                                    &fctx,
                                    &mut fbuf,
                                    &fancestors,
                                    None,
                                    None,
                                    positioned_depth,
                                    env,
                                );
                                if !fbuf.is_empty() {
                                    items[i].elements = fbuf;
                                    items[i].height = if stretches {
                                        inner_cross_size
                                    } else {
                                        items[i].elements.iter().map(estimate_element_height).sum()
                                    };
                                }
                                continue;
                            }
                            let mut relayout_buf = Vec::new();
                            let mut relayout_ancestors = ancestors.to_vec();
                            relayout_ancestors.push(AncestorInfo {
                                element: el,
                                child_index: 0,
                                sibling_count: 0,
                                preceding_siblings: Vec::new(),
                                following_siblings: Vec::new(),
                                is_empty: false,
                            });
                            let relayout_ctx = ctx
                                .with_parent_and_basis(
                                    relayout_avail,
                                    width_for_percentages,
                                    Some(10000.0),
                                    style.font_size,
                                )
                                .with_containing_block(None);
                            flatten_element(
                                child_el,
                                style,
                                &relayout_ctx,
                                &mut relayout_buf,
                                None,
                                &relayout_ancestors,
                                positioned_depth,
                                items[i].child_idx,
                                child_count,
                                &[],
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

                // Recompute the true remaining main free space from the FINAL
                // item widths after grow/shrink. With `flex-shrink:0` items that
                // overflow the line this stays NEGATIVE, so `justify-content`
                // positions from the proper edge (center/flex-end/space-*) instead
                // of collapsing to flex-start. The earlier `free_space` was forced
                // to 0 by the shrink pass, masking real overflow.
                let final_item_width: f32 = line_items.iter().map(|&i| items[i].width).sum();
                // Fixed (non-auto) main-axis item margins consume free space too,
                // so subtract them before justify-content / auto-margin packing.
                let total_main_margin: f32 = line_items
                    .iter()
                    .map(|&i| items[i].margin_main_start + items[i].margin_main_end)
                    .sum();
                let free_space = inner_width - final_item_width - total_gap - total_main_margin;

                // css-flexbox-1 §8.1: before justify-content runs, positive main
                // free space is split equally among the line's `auto` main-axis
                // margins, which then override justify-content. With no auto
                // margins this is inert and justify-content distributes normally.
                let auto_main_count: u32 = line_items
                    .iter()
                    .map(|&i| {
                        items[i].margin_main_start_auto as u32
                            + items[i].margin_main_end_auto as u32
                    })
                    .sum();
                let use_auto_margins = auto_main_count > 0 && free_space > 0.0;
                let auto_share = if use_auto_margins {
                    free_space / auto_main_count as f32
                } else {
                    0.0
                };
                let justify_free = if use_auto_margins {
                    0.0
                } else {
                    free_space.max(0.0)
                };

                // Calculate starting x and spacing based on justify-content. On
                // overflow (negative free space, half-pixel epsilon to ignore
                // rounding noise from a full grow) css-flexbox-1 §8.2 degrades
                // space-between -> flex-start and space-around/space-evenly ->
                // center, while center/flex-end honor alignment and overflow past
                // the edge (css-align-3 §9 Overflow Alignment, unsafe default).
                let (mut x, extra_gap) = if free_space < -0.5 && !use_auto_margins {
                    match justify {
                        JustifyContent::FlexStart | JustifyContent::SpaceBetween => (0.0, 0.0),
                        JustifyContent::FlexEnd => (free_space, 0.0),
                        JustifyContent::Center
                        | JustifyContent::SpaceAround
                        | JustifyContent::SpaceEvenly => (free_space / 2.0, 0.0),
                    }
                } else {
                    match justify {
                        JustifyContent::FlexStart => (0.0, 0.0),
                        JustifyContent::FlexEnd => (justify_free, 0.0),
                        JustifyContent::Center => (justify_free / 2.0, 0.0),
                        JustifyContent::SpaceBetween => {
                            if line_item_count > 1 {
                                (0.0, justify_free / (line_item_count - 1) as f32)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        JustifyContent::SpaceAround => {
                            let around = justify_free / line_item_count as f32;
                            (around / 2.0, around)
                        }
                        JustifyContent::SpaceEvenly => {
                            let ev = justify_free / (line_item_count + 1) as f32;
                            (ev, ev)
                        }
                    }
                };

                // Build FlexCells for this row line.
                let mut flex_cells = Vec::new();
                // A trailing `auto` main margin on a prior item pushes the next
                // item along the main axis; carry it forward into the cursor.
                let mut pending_trailing_auto = 0.0_f32;
                for &item_idx in &line_items {
                    // Apply the previous item's trailing auto margin, then this
                    // item's leading auto margin, before placing its cell.
                    x += pending_trailing_auto;
                    if items[item_idx].margin_main_start_auto {
                        x += auto_share;
                    }
                    pending_trailing_auto = if items[item_idx].margin_main_end_auto {
                        auto_share
                    } else {
                        0.0
                    };
                    // Honor the item's fixed leading main-axis margin: it offsets
                    // the cursor so the cell sits after the margin, and the
                    // trailing margin is added to the advance below.
                    x += items[item_idx].margin_main_start;
                    let item = &items[item_idx];

                    // A flex item that is itself a flex container establishes an
                    // independent formatting context: its `elements` already carry
                    // every inner box's own background/width/height/x-offset (a
                    // nested `FlexRow`, or a column's per-child TextBlocks). The
                    // text-merge path below would keep only the first box's
                    // background and drop the rest (blank nested rows, vanished
                    // column children), so route the whole sub-layout through
                    // `nested_elements` for the renderer to paint each inner box.
                    if item.is_flex_container {
                        flex_cells.push(FlexCell {
                            lines: Vec::new(),
                            x_offset: x,
                            width: item.width,
                            natural_height: item.height,
                            has_explicit_height: item.has_explicit_height,
                            cross_min: item.cross_min,
                            cross_max: item.cross_max,
                            align_self: item.align_self,
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
                            background_conic_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            background_clip: BackgroundClip::Border,
                            transform: None,
                            transform_origin: crate::style::computed::TransformOrigin::default(),
                            box_shadow: Vec::new(),
                            nested_elements: item.elements.clone(),
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                            is_positioned: false,
                            z_index: item.z_index,
                        });
                        // Match the x-advance of the pre-existing nested_elements
                        // branch this guard supersedes (no `extra_gap`), so the
                        // already-correct bordered-nested-flex layout is unchanged.
                        x += item.width + gap + item.margin_main_end;
                        continue;
                    }

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
                                has_explicit_height: item.has_explicit_height,
                                cross_min: item.cross_min,
                                cross_max: item.cross_max,
                                align_self: item.align_self,
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
                                background_conic_gradient: None,
                                background_svg: None,
                                background_blur_radius: 0.0,
                                background_size: BackgroundSize::Auto,
                                background_position: BackgroundPosition::default(),
                                background_repeat: BackgroundRepeat::Repeat,
                                background_origin: BackgroundOrigin::Padding,
                                background_clip: BackgroundClip::Border,
                                transform: None,
                                transform_origin: crate::style::computed::TransformOrigin::default(
                                ),
                                box_shadow: Vec::new(),
                                nested_elements: item.elements.clone(),
                                y_offset: 0.0,
                                line_cross_size: 0.0,
                                is_positioned: false,
                                z_index: item.z_index,
                            });
                            x += item.width + gap + item.margin_main_end;
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
                                        x_offset: 0.0,
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
                            has_explicit_height: item.has_explicit_height,
                            cross_min: item.cross_min,
                            cross_max: item.cross_max,
                            align_self: item.align_self,
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
                            background_conic_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            background_clip: BackgroundClip::Border,
                            transform: None,
                            transform_origin: crate::style::computed::TransformOrigin::default(),
                            box_shadow: Vec::new(),
                            nested_elements: Vec::new(),
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                            is_positioned: false,
                            z_index: item.z_index,
                        });
                        x += item.width + gap + item.margin_main_end;
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
                        background_conic_gradient: tb_cgrad,
                        background_svg: tb_bg_svg,
                        background_blur_radius: tb_bg_blur,
                        background_size: tb_bg_size,
                        background_position: tb_bg_pos,
                        background_repeat: tb_bg_repeat,
                        background_origin: tb_bg_origin,
                        background_clip: tb_bg_clip,
                        box_shadow: tb_bs,
                        border,
                        block_height: tb_bh,
                        transform: tb_transform,
                        transform_origin: tb_transform_origin,
                        ..
                    }) = item.elements.first()
                    {
                        // Natural cross size: an explicit height defines it;
                        // otherwise derive from content (text + padding + border).
                        // Without honoring block_height, an empty box with an
                        // explicit height collapses to ~border height under any
                        // non-stretch align-items (it vanished entirely).
                        let text_h: f32 = tb_lines.iter().map(|l| l.height).sum();
                        let content_natural = *tb_pt + text_h + *tb_pb + border.vertical_width();
                        // `natural_height` is the cell's border-box (the renderer
                        // paints the border inside it). `content_natural` already
                        // includes the border, but `block_height` is a padding-box
                        // height (TextBlock convention), so add the border back to
                        // keep the two cases consistent — otherwise an explicit
                        // height rendered the box short by its border thickness.
                        let natural_h = tb_bh
                            .map(|h| h + border.vertical_width())
                            .unwrap_or(content_natural);
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
                            background_conic_gradient: tb_cgrad.clone(),
                            background_svg: tb_bg_svg.clone(),
                            background_blur_radius: *tb_bg_blur,
                            background_size: *tb_bg_size,
                            background_position: *tb_bg_pos,
                            background_repeat: *tb_bg_repeat,
                            background_origin: *tb_bg_origin,
                            background_clip: *tb_bg_clip,
                            transform: *tb_transform,
                            transform_origin: *tb_transform_origin,
                            box_shadow: tb_bs.clone(),
                            nested_elements: Vec::new(),
                            natural_height: natural_h,
                            has_explicit_height: item.has_explicit_height,
                            cross_min: item.cross_min,
                            cross_max: item.cross_max,
                            align_self: item.align_self,
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                            is_positioned: false,
                            z_index: item.z_index,
                        });
                    } else {
                        // Single non-TextBlock element (e.g. Container): store
                        // in nested_elements for the renderer to handle.
                        flex_cells.push(FlexCell {
                            lines: Vec::new(),
                            x_offset: x,
                            width: item.width,
                            natural_height: item.height,
                            has_explicit_height: item.has_explicit_height,
                            cross_min: item.cross_min,
                            cross_max: item.cross_max,
                            align_self: item.align_self,
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
                            background_conic_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::Padding,
                            background_clip: BackgroundClip::Border,
                            transform: None,
                            transform_origin: crate::style::computed::TransformOrigin::default(),
                            box_shadow: Vec::new(),
                            nested_elements: item.elements.clone(),
                            y_offset: 0.0,
                            line_cross_size: 0.0,
                            is_positioned: false,
                            z_index: item.z_index,
                        });
                    }

                    x += item.width + gap + extra_gap + item.margin_main_end;
                }

                // `flex-direction: row-reverse` flips the main axis: main-start
                // is the right edge. Mirror each cell's x within the content box
                // so the first source item sits at the right and items run
                // right-to-left (gaps and justify packing are preserved by the
                // mirror because they are symmetric about the content box).
                if direction == FlexDirection::RowReverse {
                    for cell in flex_cells.iter_mut() {
                        cell.x_offset = inner_width - cell.x_offset - cell.width;
                    }
                }

                // `flex-wrap: wrap-reverse` flips the cross-start edge to the
                // cross-end. Items that would anchor to a line's top (the
                // default for non-stretched items) instead anchor to the line
                // bottom. The renderer positions each cell within its line by
                // `align`, so for a flex-start-anchored, non-stretching item we
                // pre-shift its y by the slack inside the line to land it at the
                // cross-end. (Stretch items already fill the line.)
                let wrap_reversed = wrap == FlexWrap::WrapReverse;

                // Stamp each cell with its cross-axis position within the
                // container so a single FlexRow can span every wrapped line.
                for cell in flex_cells.iter_mut() {
                    let anchor_start = matches!(
                        cell.align_self,
                        AlignSelf::Auto | AlignSelf::FlexStart | AlignSelf::Baseline
                    ) && (align == AlignItems::FlexStart
                        || align == AlignItems::Baseline
                        || cell.align_self == AlignSelf::FlexStart
                        || cell.align_self == AlignSelf::Baseline
                        || (matches!(cell.align_self, AlignSelf::Auto)
                            && align == AlignItems::Stretch
                            && cell.has_explicit_height));
                    let cross_pad = if wrap_reversed && anchor_start {
                        (line.cross_size - cell.natural_height).max(0.0)
                    } else {
                        0.0
                    };
                    cell.y_offset = cross_offset + cross_pad;
                    cell.line_cross_size = line.cross_size;
                }

                // Apply each item's fixed cross-axis leading margin (margin-top
                // for a row container) and its `position: relative` paint offset.
                // Cells are 1:1 with `line_items` in placement order, so zip them.
                // Relative offsets are physical and applied after the row-reverse
                // mirror; a relatively-offset item is flagged positioned so it
                // paints above its in-flow siblings.
                for (cell, &item_idx) in flex_cells.iter_mut().zip(line_items.iter()) {
                    let it = &items[item_idx];
                    cell.y_offset += it.margin_cross_start;
                    if it.is_relative {
                        cell.x_offset += it.rel_left;
                        cell.y_offset += it.rel_top;
                    }
                    cell.is_positioned = it.is_relative || it.z_index > 0;
                }
                all_flex_cells.extend(flex_cells);
            }
            FlexDirection::Column | FlexDirection::ColumnReverse => {
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };

                // Column main-axis flex grow/shrink: the main axis is the block
                // (vertical) axis, so distribute/absorb the container's spare
                // height across the items, clamped to each item's min/max main
                // (min-height / max-height). Only run when the container has a
                // definite main size (`inner_cross_size > 0`). Mirrors the row
                // resolution but along the height.
                if inner_cross_size > 0.0 {
                    let sum_h: f32 = line_items.iter().map(|&i| items[i].height).sum();
                    let mut col_free = inner_cross_size - sum_h - total_gap;
                    let total_grow: f32 = line_items.iter().map(|&i| items[i].flex_grow).sum();
                    if col_free > 0.0 && total_grow > 0.0 {
                        let mut frozen = vec![false; line_items.len()];
                        // §9.7 step 4.b: cap the distributed space to the flex
                        // factor sum when it is below 1 (the rest stays free).
                        let mut remaining = if total_grow < 1.0 {
                            col_free * total_grow
                        } else {
                            col_free
                        };
                        for _ in 0..=line_items.len() {
                            let active: f32 = line_items
                                .iter()
                                .enumerate()
                                .filter(|(li, _)| !frozen[*li])
                                .map(|(_, &i)| items[i].flex_grow)
                                .sum();
                            if active <= 0.0 || remaining <= 0.01 {
                                break;
                            }
                            let mut froze = false;
                            let mut consumed = 0.0;
                            for (li, &i) in line_items.iter().enumerate() {
                                if frozen[li] {
                                    continue;
                                }
                                let share = remaining * (items[i].flex_grow / active);
                                let target = items[i].height + share;
                                if target >= items[i].max_main {
                                    consumed += items[i].max_main - items[i].height;
                                    items[i].height = items[i].max_main;
                                    frozen[li] = true;
                                    froze = true;
                                } else {
                                    items[i].height = target;
                                    consumed += share;
                                }
                            }
                            remaining -= consumed;
                            if !froze {
                                break;
                            }
                        }
                        col_free = 0.0;
                    }
                    if col_free < 0.0 {
                        let mut frozen = vec![false; line_items.len()];
                        // §9.7 step 4.b (shrink): absorb only the flex-shrink
                        // factor sum's fraction of the deficit when it is below 1.
                        let total_shrink: f32 =
                            line_items.iter().map(|&i| items[i].flex_shrink).sum();
                        let mut deficit = if total_shrink < 1.0 {
                            -col_free * total_shrink
                        } else {
                            -col_free
                        };
                        for _ in 0..=line_items.len() {
                            let weight_sum: f32 = line_items
                                .iter()
                                .enumerate()
                                .filter(|(li, _)| !frozen[*li])
                                .map(|(_, &i)| items[i].flex_shrink * items[i].height)
                                .sum();
                            if weight_sum <= 0.0 || deficit <= 0.01 {
                                break;
                            }
                            let mut froze = false;
                            let mut removed = 0.0;
                            for (li, &i) in line_items.iter().enumerate() {
                                if frozen[li] {
                                    continue;
                                }
                                let weight = items[i].flex_shrink * items[i].height;
                                let reduce = deficit * (weight / weight_sum);
                                let target = items[i].height - reduce;
                                let floor = items[i].min_main.max(0.0);
                                if target <= floor {
                                    removed += items[i].height - floor;
                                    items[i].height = floor;
                                    frozen[li] = true;
                                    froze = true;
                                } else {
                                    items[i].height = target;
                                    removed += reduce;
                                }
                            }
                            deficit -= removed;
                            if !froze {
                                break;
                            }
                        }
                    }
                    // Keep natural_height in sync so cross-axis emission uses the
                    // resolved main size.
                    for &i in &line_items {
                        items[i].natural_height = items[i].height;
                    }
                }

                let total_item_height: f32 = line_items.iter().map(|&i| items[i].height).sum();
                // Main-axis (vertical) free space within the container's content
                // box. `justify-content` distributes it as leading before the
                // first item and extra spacing between items. `inner_cross_size`
                // is the resolved content height once an explicit `height` /
                // `min-height` has been honored.
                // Real (signed) main-axis free space: keep it negative when the
                // items overflow a definite container height so justify-content
                // packs from the proper edge instead of collapsing to flex-start
                // (css-align-3 §9 Overflow Alignment).
                let main_free_space = inner_cross_size - total_item_height - total_gap;
                // For column-reverse the main axis points up (main-start is the
                // bottom). We lay items out in reverse source order (top to
                // bottom = last to first); swapping flex-start/flex-end then
                // packs the free space on the correct (top) side so the visual
                // result matches a bottom-anchored start edge.
                let effective_justify = if direction == FlexDirection::ColumnReverse {
                    match justify {
                        JustifyContent::FlexStart => JustifyContent::FlexEnd,
                        JustifyContent::FlexEnd => JustifyContent::FlexStart,
                        other => other,
                    }
                } else {
                    justify
                };
                let (leading, extra_gap) = if main_free_space < -0.5 {
                    // Overflow: §8.2 degradation — space-between -> flex-start,
                    // space-around/space-evenly -> center; center/flex-end honor
                    // alignment and overflow past the edge.
                    match effective_justify {
                        JustifyContent::FlexStart | JustifyContent::SpaceBetween => (0.0, 0.0),
                        JustifyContent::FlexEnd => (main_free_space, 0.0),
                        JustifyContent::Center
                        | JustifyContent::SpaceAround
                        | JustifyContent::SpaceEvenly => (main_free_space / 2.0, 0.0),
                    }
                } else {
                    let main_free_space = main_free_space.max(0.0);
                    match effective_justify {
                        JustifyContent::FlexStart => (0.0, 0.0),
                        JustifyContent::FlexEnd => (main_free_space, 0.0),
                        JustifyContent::Center => (main_free_space / 2.0, 0.0),
                        JustifyContent::SpaceBetween => {
                            if line_item_count > 1 {
                                (0.0, main_free_space / (line_item_count - 1) as f32)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        JustifyContent::SpaceAround => {
                            let around = main_free_space / line_item_count as f32;
                            (around / 2.0, around)
                        }
                        JustifyContent::SpaceEvenly => {
                            let ev = main_free_space / (line_item_count + 1) as f32;
                            (ev, ev)
                        }
                    }
                };

                let mut y = 0.0;
                // Leading is applied as part of the first item's top spacing
                // (which already folds in the container's border + padding); a
                // nonzero leading bumps `y` so subsequent gap math stays correct.
                let mut pending_leading = leading;
                // Per css-flexbox-1 § 6, flex-item margins never collapse — not
                // with each other, nor with the container. The downstream block
                // flow *does* collapse adjacent sibling margins, so we fold the
                // previous item's bottom margin into the next item's leading and
                // emit each item with `margin_bottom: 0`. That keeps the full
                // `prev.margin_bottom + next.margin_top` gap (e.g. 40 + 30 = 70px)
                // instead of the collapsed `max(40, 30) = 40px` of block flow.
                let mut prev_item_margin_bottom = 0.0_f32;

                // `flex-direction: column-reverse` flips the main axis: the
                // first source item is placed at the bottom. Iterating the line
                // in reverse source order packs them bottom-to-top.
                let column_order: Vec<usize> = if direction == FlexDirection::ColumnReverse {
                    line_items.iter().rev().copied().collect()
                } else {
                    line_items.clone()
                };

                if column_wrap_lines {
                    let mut y = leading;
                    for (item_pos, &item_idx) in column_order.iter().enumerate() {
                        let item = &items[item_idx];
                        if item_pos > 0 {
                            y += gap + extra_gap;
                        }

                        let effective_align = match item.align_self {
                            AlignSelf::Auto => align,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            AlignSelf::Baseline => AlignItems::FlexStart,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };
                        let used_width =
                            if effective_align == AlignItems::Stretch && !item.has_explicit_width {
                                line.cross_size
                            } else {
                                item.width
                            };
                        let mut x_offset = cross_offset
                            + match effective_align {
                                AlignItems::FlexStart | AlignItems::Baseline => 0.0,
                                AlignItems::FlexEnd => line.cross_size - used_width,
                                AlignItems::Center => (line.cross_size - used_width) / 2.0,
                                AlignItems::Stretch => 0.0,
                            };
                        let mut y_offset = y + item.margin_main_start;
                        if item.is_relative {
                            x_offset += item.rel_left;
                            y_offset += item.rel_top;
                        }

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
                            background_conic_gradient: tb_cgrad,
                            background_svg: tb_bg_svg,
                            background_blur_radius: tb_bg_blur,
                            background_size: tb_bg_size,
                            background_position: tb_bg_pos,
                            background_repeat: tb_bg_repeat,
                            background_origin: tb_bg_origin,
                            background_clip: tb_bg_clip,
                            box_shadow: tb_bs,
                            border,
                            block_height: tb_bh,
                            transform: tb_transform,
                            transform_origin: tb_transform_origin,
                            ..
                        }) = item.elements.first()
                        {
                            let text_h: f32 = tb_lines.iter().map(|l| l.height).sum();
                            let content_natural =
                                *tb_pt + text_h + *tb_pb + border.vertical_width();
                            let natural_h = tb_bh
                                .map(|h| h + border.vertical_width())
                                .unwrap_or(content_natural);
                            all_flex_cells.push(FlexCell {
                                lines: tb_lines.clone(),
                                x_offset,
                                width: used_width,
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
                                background_conic_gradient: tb_cgrad.clone(),
                                background_svg: tb_bg_svg.clone(),
                                background_blur_radius: *tb_bg_blur,
                                background_size: *tb_bg_size,
                                background_position: *tb_bg_pos,
                                background_repeat: *tb_bg_repeat,
                                background_origin: *tb_bg_origin,
                                background_clip: *tb_bg_clip,
                                transform: *tb_transform,
                                transform_origin: *tb_transform_origin,
                                box_shadow: tb_bs.clone(),
                                nested_elements: Vec::new(),
                                natural_height: natural_h,
                                has_explicit_height: true,
                                cross_min: 0.0,
                                cross_max: f32::INFINITY,
                                align_self: AlignSelf::FlexStart,
                                y_offset,
                                line_cross_size: natural_h,
                                is_positioned: item.is_relative || item.z_index > 0,
                                z_index: item.z_index,
                            });
                        } else {
                            all_flex_cells.push(FlexCell {
                                lines: Vec::new(),
                                x_offset,
                                width: used_width,
                                natural_height: item.height,
                                has_explicit_height: true,
                                cross_min: 0.0,
                                cross_max: f32::INFINITY,
                                align_self: AlignSelf::FlexStart,
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
                                background_conic_gradient: None,
                                background_svg: None,
                                background_blur_radius: 0.0,
                                background_size: BackgroundSize::Auto,
                                background_position: BackgroundPosition::default(),
                                background_repeat: BackgroundRepeat::Repeat,
                                background_origin: BackgroundOrigin::Padding,
                                background_clip: BackgroundClip::Border,
                                transform: None,
                                transform_origin: crate::style::computed::TransformOrigin::default(
                                ),
                                box_shadow: Vec::new(),
                                nested_elements: item.elements.clone(),
                                y_offset,
                                line_cross_size: item.height,
                                is_positioned: item.is_relative || item.z_index > 0,
                                z_index: item.z_index,
                            });
                        }
                        y += item.height;
                    }
                } else {
                    for (item_pos, &item_idx) in column_order.iter().enumerate() {
                        let item = &items[item_idx];

                        // `align-self` overrides the container's `align-items` on the
                        // cross axis (horizontal, for a column container).
                        let effective_align = match item.align_self {
                            AlignSelf::Auto => align,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            // Baseline has no first-baseline notion on the cross axis
                            // of a column container; fall back to flex-start (the
                            // cross-start edge), matching browser behaviour for
                            // baseline alignment of empty boxes.
                            AlignSelf::Baseline => AlignItems::FlexStart,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };

                        // Calculate cross-axis (horizontal) alignment
                        let x_offset = match effective_align {
                            AlignItems::FlexStart | AlignItems::Baseline => 0.0,
                            AlignItems::FlexEnd => inner_width - item.width,
                            AlignItems::Center => (inner_width - item.width) / 2.0,
                            AlignItems::Stretch => 0.0,
                        };

                        // align-items: stretch only stretches items whose cross size
                        // (width, for a column container) is auto. An item with an
                        // explicit width keeps it.
                        let effective_width =
                            if effective_align == AlignItems::Stretch && !item.has_explicit_width {
                                Some(inner_width)
                            } else {
                                Some(item.width)
                            };

                        // Extra main-axis spacing this item contributes from
                        // `justify-content`: the leading for the first item, an
                        // even slice between items otherwise. Applied only to the
                        // item's first emitted element so multi-element items aren't
                        // over-spaced.
                        let item_justify_lead = if item_pos == 0 {
                            std::mem::take(&mut pending_leading)
                        } else {
                            extra_gap
                        };
                        let mut item_first_elem = true;
                        // The bottom margin of this item's last emitted element,
                        // folded into the next item's leading (flex margins don't
                        // collapse). Reset per item.
                        let mut item_last_margin_bottom = 0.0_f32;

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
                                mix_blend_mode: tb_mix_blend,
                                background_blend_mode: tb_bg_blend,
                                position: tb_pos,
                                box_shadow: tb_bs,
                                visible: tb_vis,
                                clip_rect: tb_clip,
                                transform: tb_transform,
                                transform_origin: tb_transform_origin,
                                border_radius: tb_br,
                                outline_width: tb_ow,
                                outline_color: tb_oc,
                                text_indent: tb_ti,
                                letter_spacing: tb_ls,
                                word_spacing: tb_ws,
                                vertical_align: tb_va,
                                background_gradient: tb_grad,
                                background_radial_gradient: tb_rgrad,
                                background_conic_gradient: tb_cgrad,
                                background_svg: tb_bg_svg,
                                background_blur_radius: tb_bg_blur,
                                background_size: tb_bg_size,
                                background_position: tb_bg_pos,
                                background_repeat: tb_bg_repeat,
                                background_origin: tb_bg_origin,
                                background_clip: tb_bg_clip,
                                ..
                            } = elem
                            {
                                // `justify-content` leading/spacing applies once per
                                // item, to its first emitted element.
                                let justify_lead = if item_first_elem {
                                    item_first_elem = false;
                                    item_justify_lead
                                } else {
                                    0.0
                                };
                                // Carry this element's bottom margin to the next
                                // item's leading (flex margins don't collapse).
                                item_last_margin_bottom = *tb_mb;
                                // When the column flex resolution changed the item's
                                // main (block) size (grow/shrink against the
                                // container height, or a `flex-basis` height on an
                                // empty box), paint the box at that resolved height.
                                // `block_height` is a padding-box height (TextBlock
                                // convention), so subtract the element's border. Only
                                // applies to single-element items.
                                let resolved_bh = if item.elements.len() == 1 {
                                    // item.height is the border-box main size + item
                                    // margins; block_height is a padding-box height,
                                    // so strip the element's own margins and border.
                                    let pad_box = (item.height
                                        - *tb_mt
                                        - *tb_mb
                                        - tb_border.vertical_width())
                                    .max(0.0);
                                    Some(pad_box)
                                } else {
                                    *tb_bh
                                };
                                output.push(LayoutElement::TextBlock {
                                    box_decoration_break:
                                        crate::style::computed::BoxDecorationBreak::Slice,
                                    orphans: 2,
                                    widows: 2,
                                    lines: tb_lines.clone(),
                                    margin_top: if y == 0.0 && !emitted_column_bg {
                                        style.margin.top
                                            + style.border.top.width
                                            + style.padding.top
                                            + justify_lead
                                            + *tb_mt
                                    } else if y == 0.0 {
                                        // Background element already accounts for margin;
                                        // add the container's top border + padding so the
                                        // first item flows inside the container's border box.
                                        style.border.top.width
                                            + style.padding.top
                                            + justify_lead
                                            + *tb_mt
                                    } else {
                                        // Apply gap between column-direction flex
                                        // items, plus the previous item's bottom
                                        // margin (flex margins don't collapse, so we
                                        // sum rather than let the flow collapse them).
                                        gap + justify_lead + prev_item_margin_bottom + *tb_mt
                                    },
                                    // Flex-item margins never collapse; the prior
                                    // item's bottom margin is folded into this item's
                                    // leading above, so emit 0 here to avoid the
                                    // downstream block flow collapsing them.
                                    margin_bottom: 0.0,
                                    text_align: *tb_ta,
                                    writing_mode: crate::style::computed::WritingMode::HorizontalTb,
                                    background_color: *tb_bg,
                                    padding_top: *tb_pt,
                                    padding_bottom: *tb_pb,
                                    padding_left: *tb_pl,
                                    padding_right: *tb_pr,
                                    border: *tb_border,
                                    block_width: effective_width,
                                    block_height: resolved_bh,
                                    opacity: *tb_op,
                                    mix_blend_mode: *tb_mix_blend,
                                    background_blend_mode: *tb_bg_blend,
                                    float: Float::None,
                                    clear: Clear::None,
                                    position: if x_offset > 0.0
                                        || style.padding.left > 0.0
                                        || style.border.left.width > 0.0
                                    {
                                        Position::Relative
                                    } else {
                                        *tb_pos
                                    },
                                    offset_top: 0.0,
                                    offset_left: x_offset
                                        + style.padding.left
                                        + style.border.left.width,
                                    offset_bottom: 0.0,
                                    offset_right: 0.0,
                                    containing_block: None,
                                    box_shadow: tb_bs.clone(),
                                    visible: *tb_vis,
                                    clip_rect: *tb_clip,
                                    transform: *tb_transform,
                                    transform_origin: *tb_transform_origin,
                                    border_radius: *tb_br,
                                    border_radii: [*tb_br; 4],
                                    border_radii_y: [*tb_br; 4],
                                    outline_offset: 0.0,
                                    outline_width: *tb_ow,
                                    outline_color: *tb_oc,
                                    text_indent: *tb_ti,
                                    letter_spacing: *tb_ls,
                                    word_spacing: *tb_ws,
                                    vertical_align: *tb_va,
                                    background_gradient: tb_grad.clone(),
                                    background_radial_gradient: tb_rgrad.clone(),
                                    background_conic_gradient: tb_cgrad.clone(),
                                    background_svg: tb_bg_svg.clone(),
                                    background_blur_radius: *tb_bg_blur,
                                    background_size: *tb_bg_size,
                                    background_position: *tb_bg_pos,
                                    background_repeat: *tb_bg_repeat,
                                    background_origin: *tb_bg_origin,
                                    background_clip: *tb_bg_clip,
                                    z_index: 0,
                                    repeat_on_each_page: false,
                                    positioned_depth: 0,
                                    heading_level: None,
                                    clip_children_count: 0,
                                });
                            } else {
                                // Non-TextBlock flex item (e.g. a Container emitted
                                // for a padded child). Wrap it so the column's
                                // main-axis (vertical) leading and cross-axis
                                // (horizontal) alignment are applied; otherwise the
                                // element would be silently dropped by this loop.
                                let justify_lead = if item_first_elem {
                                    item_first_elem = false;
                                    item_justify_lead
                                } else {
                                    0.0
                                };
                                let leading = if y == 0.0 && !emitted_column_bg {
                                    style.margin.top
                                        + style.border.top.width
                                        + style.padding.top
                                        + justify_lead
                                } else if y == 0.0 {
                                    style.border.top.width + style.padding.top + justify_lead
                                } else {
                                    gap + justify_lead + prev_item_margin_bottom
                                };
                                output.push(LayoutElement::Container {
                                    box_decoration_break:
                                        crate::style::computed::BoxDecorationBreak::Slice,
                                    children: vec![elem.clone()],
                                    background_color: None,
                                    border: LayoutBorder::default(),
                                    border_radius: 0.0,
                                    border_radii: [0.0; 4],
                                    border_radii_y: [0.0; 4],
                                    outline_offset: 0.0,
                                    padding_top: 0.0,
                                    padding_bottom: 0.0,
                                    padding_left: 0.0,
                                    padding_right: 0.0,
                                    margin_top: leading,
                                    margin_bottom: 0.0,
                                    block_width: effective_width,
                                    block_height: None,
                                    opacity: 1.0,
                                    mix_blend_mode: crate::style::computed::BlendMode::Normal,
                                    background_blend_mode:
                                        crate::style::computed::BlendMode::Normal,
                                    visible: true,
                                    float: Float::None,
                                    clear: Clear::None,
                                    position: if x_offset > 0.0
                                        || style.padding.left > 0.0
                                        || style.border.left.width > 0.0
                                    {
                                        Position::Relative
                                    } else {
                                        Position::Static
                                    },
                                    offset_top: 0.0,
                                    offset_left: x_offset
                                        + style.padding.left
                                        + style.border.left.width,
                                    overflow: Overflow::Visible,
                                    overflow_x: Overflow::Visible,
                                    overflow_y: Overflow::Visible,
                                    transform: None,
                                    transform_origin:
                                        crate::style::computed::TransformOrigin::default(),
                                    clip_path: None,
                                    mask_image: None,
                                    mask_mode: crate::style::computed::MaskMode::default(),
                                    box_shadow: Vec::new(),
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
                                    outline_width: 0.0,
                                    outline_color: None,
                                    z_index: 0,
                                    positioned_depth: 0,
                                    containing_block: None,
                                });
                            }
                        }

                        y += item.height + gap;
                        prev_item_margin_bottom = item_last_margin_bottom;
                    }
                }
            }
        }

        cross_offset += line.cross_size + line_gap;
    }

    // Emit a single FlexRow carrying every line's cells for row direction.
    // The row's height is the container's inner cross size so pagination and
    // the visual border both include every wrapped line. Each cell's own
    // y_offset and line_cross_size handle per-line alignment internally.
    if (direction.is_row() || column_wrap_lines) && !all_flex_cells.is_empty() {
        all_flex_cells.sort_by_key(|cell| cell.z_index);
        let row_height = if column_wrap_lines {
            inner_cross_size
        } else {
            total_cross.max(inner_cross_size)
        };
        output.push(LayoutElement::FlexRow {
            cells: all_flex_cells,
            row_height,
            margin_top: style.margin.top,
            margin_bottom: 0.0,
            offset_left: h_offset,
            background_color: bg,
            container_width: block_w,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            border_radius: style.border_radius,
            box_shadow: style.box_shadow.clone(),
            background_gradient: style.background_gradient.clone(),
            background_radial_gradient: style.background_radial_gradient.clone(),
            background_conic_gradient: style.background_conic_gradient.clone(),
            background_svg: background_svg_for_style(style),
            background_blur_radius: style.blur_radius,
            background_size: style.background_size,
            background_position: style.background_position,
            background_repeat: style.background_repeat,
            background_origin: style.background_origin,
            background_clip: style.background_clip,
            align_items: if column_wrap_lines {
                AlignItems::FlexStart
            } else {
                align
            },
            positioned_depth: abs_cb_depth,
        });
    }

    // Emit out-of-flow absolute children after the in-flow flex content so they
    // paint above it (CSS painting order) and anchor to the container's padding
    // box via the containing block stamped above.
    output.append(&mut abs_output);

    // Emit trailing margin (include bottom padding when bg spacer shifted y back)
    let trailing = if emitted_column_bg {
        style.padding.bottom + style.margin.bottom
    } else {
        style.margin.bottom
    };
    if trailing > 0.0 {
        output.push(LayoutElement::TextBlock {
            box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
            orphans: 2,
            widows: 2,
            lines: Vec::new(),
            margin_top: trailing,
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
            clip_children_count: 0,
        });
    }
}
