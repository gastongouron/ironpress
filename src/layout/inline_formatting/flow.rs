use crate::layout::context::LayoutEnv;
use crate::layout::engine::TextRun;
use crate::layout::text::{InlineRunCollector, InlineRunContext};
use crate::parser::css::AncestorInfo;
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::ComputedStyle;

use super::{
    InlineContentSequence, InlineFormattingChild, InlineFormattingContext, InlineFormattingRole,
};

/// Context-specific ownership of boxes outside an inline run sequence.
///
/// Implementations lay out only independent boxes. Text, generated content,
/// selector positions, and counter traversal remain owned by
/// [`layout_mixed_flow_children`].
pub(crate) trait IndependentFlowLayout {
    fn inline_run_context(&self) -> InlineRunContext {
        InlineRunContext::Standard
    }

    fn lays_out_independently(&self, element: &ElementNode, child: &InlineFormattingChild) -> bool;

    fn layout_independently(
        &mut self,
        element: &ElementNode,
        child: &InlineFormattingChild,
        env: &mut LayoutEnv<'_>,
    );
}

/// Route one mixed child sequence through the shared inline collector.
///
/// Every DOM item is classified against the same complete sibling sequence.
/// Inline participants are collected by [`InlineRunCollector`]; only a policy-
/// approved independent box escapes to the formatting-context implementation.
pub(crate) fn layout_mixed_flow_children<'dom>(
    nodes: &'dom [DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    ancestors: &[AncestorInfo<'dom>],
    env: &mut LayoutEnv<'_>,
    independent_layout: &mut impl IndependentFlowLayout,
) {
    let sequence = InlineContentSequence::new(nodes);
    let children =
        InlineFormattingContext::new(parent_style, env.rules, ancestors, env.font_metrics())
            .children(sequence);
    let mut element_index = 0;
    let run_context = independent_layout.inline_run_context();

    for (node_index, node) in nodes.iter().enumerate() {
        let DomNode::Element(element) = node else {
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .in_context(run_context)
                .collect(
                    sequence.item(node_index),
                    parent_style,
                    runs,
                    None,
                    ancestors,
                );
            continue;
        };

        let Some(child) = children.get(element_index) else {
            element_index = element_index.saturating_add(1);
            continue;
        };
        element_index = element_index.saturating_add(1);

        if child.role == InlineFormattingRole::Hidden {
            continue;
        }
        if independent_layout.lays_out_independently(element, child) {
            independent_layout.layout_independently(element, child, env);
        } else {
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .in_context(run_context)
                .collect(
                    sequence.item(node_index),
                    parent_style,
                    runs,
                    None,
                    ancestors,
                );
        }
    }
}
