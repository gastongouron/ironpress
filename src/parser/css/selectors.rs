use std::collections::HashMap;

use crate::parser::dom::ElementNode;

use super::{AncestorInfo, SelectorContext};

#[derive(Clone, Copy)]
enum Combinator {
    GeneralSibling,
    AdjacentSibling,
    Child,
    Descendant,
}

/// Check if a CSS selector matches a given element (backward-compatible, no context).
pub fn selector_matches(selector: &str, tag: &str, classes: &[&str], id: Option<&str>) -> bool {
    selector_matches_with_context(
        selector,
        tag,
        classes,
        id,
        &HashMap::new(),
        &SelectorContext::default(),
    )
}

/// Check if a CSS selector matches a given element with full context.
pub fn selector_matches_with_context(
    selector: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    selector
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| compound_selector_matches(part, tag, classes, id, attributes, ctx))
}

fn compound_selector_matches(
    selector: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    if let Some((combinator, left, right)) = split_rightmost_combinator(selector) {
        return match combinator {
            Combinator::GeneralSibling => {
                simple_selector_matches(right, tag, classes, id, attributes, ctx)
                    && ctx.preceding_siblings.iter().enumerate().any(
                        |(sibling_index, (sibling_tag, sibling_classes))| {
                            let sibling_ctx = sibling_selector_context(ctx, sibling_index);
                            let sibling_class_refs: Vec<&str> =
                                sibling_classes.iter().map(String::as_str).collect();
                            compound_selector_matches(
                                left,
                                sibling_tag,
                                &sibling_class_refs,
                                None,
                                &HashMap::new(),
                                &sibling_ctx,
                            )
                        },
                    )
            }
            Combinator::AdjacentSibling => {
                simple_selector_matches(right, tag, classes, id, attributes, ctx)
                    && ctx
                        .preceding_siblings
                        .iter()
                        .enumerate()
                        .last()
                        .is_some_and(|(sibling_index, (sibling_tag, sibling_classes))| {
                            let sibling_ctx = sibling_selector_context(ctx, sibling_index);
                            let sibling_class_refs: Vec<&str> =
                                sibling_classes.iter().map(String::as_str).collect();
                            compound_selector_matches(
                                left,
                                sibling_tag,
                                &sibling_class_refs,
                                None,
                                &HashMap::new(),
                                &sibling_ctx,
                            )
                        })
            }
            Combinator::Child => {
                if !simple_selector_matches(right, tag, classes, id, attributes, ctx) {
                    return false;
                }

                let Some(parent) = ctx.ancestors.last() else {
                    return false;
                };

                let parent_index = ctx.ancestors.len() - 1;
                let parent_ctx = ancestor_selector_context(ctx, parent_index);
                compound_selector_matches(
                    left,
                    &parent.element.raw_tag_name,
                    &parent.element.class_list(),
                    parent.element.id(),
                    &parent.element.attributes,
                    &parent_ctx,
                )
            }
            Combinator::Descendant => {
                if !simple_selector_matches(right, tag, classes, id, attributes, ctx) {
                    return false;
                }

                for ancestor_index in 0..ctx.ancestors.len() {
                    let ancestor = &ctx.ancestors[ancestor_index];
                    let ancestor_ctx = ancestor_selector_context(ctx, ancestor_index);
                    if compound_selector_matches(
                        left,
                        &ancestor.element.raw_tag_name,
                        &ancestor.element.class_list(),
                        ancestor.element.id(),
                        &ancestor.element.attributes,
                        &ancestor_ctx,
                    ) {
                        return true;
                    }
                }
                false
            }
        };
    }

    simple_selector_matches(selector, tag, classes, id, attributes, ctx)
}

fn sibling_selector_context<'a>(
    ctx: &'a SelectorContext<'a>,
    sibling_index: usize,
) -> SelectorContext<'a> {
    SelectorContext {
        ancestors: ctx.ancestors.clone(),
        child_index: sibling_index,
        sibling_count: ctx.sibling_count,
        preceding_siblings: ctx
            .preceding_siblings
            .iter()
            .take(sibling_index)
            .cloned()
            .collect(),
    }
}

fn ancestor_selector_context<'a>(
    ctx: &'a SelectorContext<'a>,
    ancestor_index: usize,
) -> SelectorContext<'a> {
    let ancestor = &ctx.ancestors[ancestor_index];
    SelectorContext {
        ancestors: ctx.ancestors.iter().take(ancestor_index).cloned().collect(),
        child_index: ancestor.child_index,
        sibling_count: ancestor.sibling_count,
        preceding_siblings: ancestor.preceding_siblings.clone(),
    }
}

pub(crate) fn rfind_descendant_space(selector: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = selector.char_indices().collect();
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for index in (0..chars.len()).rev() {
        let (byte_index, ch) = chars[index];
        match ch {
            ']' => bracket_depth += 1,
            '[' => bracket_depth = bracket_depth.saturating_sub(1),
            ')' => paren_depth += 1,
            '(' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }

        if ch != ' ' || bracket_depth != 0 || paren_depth != 0 {
            continue;
        }

        let prev = index
            .checked_sub(1)
            .and_then(|prev_index| chars.get(prev_index))
            .map(|(_, ch)| *ch);
        let next = chars.get(index + 1).map(|(_, ch)| *ch);
        if matches!(prev, Some('>' | '+' | '~')) || matches!(next, Some('>' | '+' | '~')) {
            continue;
        }

        return Some(byte_index);
    }

    None
}

fn split_rightmost_combinator(selector: &str) -> Option<(Combinator, &str, &str)> {
    let mut candidate = rfind_descendant_space(selector).and_then(|byte_index| {
        let left = selector.get(..byte_index)?.trim();
        let right = selector.get(byte_index + ' '.len_utf8()..)?.trim();
        Some((byte_index, Combinator::Descendant, left, right))
    });

    for (combinator, combinator_char) in [
        (Combinator::GeneralSibling, '~'),
        (Combinator::AdjacentSibling, '+'),
        (Combinator::Child, '>'),
    ] {
        if let Some((byte_index, left, right)) = split_on_combinator(selector, combinator_char) {
            match candidate {
                Some((current_index, _, _, _)) if current_index > byte_index => {}
                _ => candidate = Some((byte_index, combinator, left, right)),
            }
        }
    }

    candidate.map(|(_, combinator, left, right)| (combinator, left, right))
}

fn split_on_combinator(selector: &str, combinator: char) -> Option<(usize, &str, &str)> {
    let chars: Vec<(usize, char)> = selector.char_indices().collect();
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for index in (0..chars.len()).rev() {
        let (_, ch) = chars[index];
        match ch {
            ']' => bracket_depth += 1,
            '[' => bracket_depth = bracket_depth.saturating_sub(1),
            ')' => paren_depth += 1,
            '(' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }

        if bracket_depth != 0 || paren_depth != 0 || ch != combinator {
            continue;
        }

        let mut left_end = index;
        while left_end > 0
            && chars
                .get(left_end - 1)
                .is_some_and(|(_, prev)| prev.is_whitespace())
        {
            left_end -= 1;
        }

        let mut right_start = index + 1;
        while chars
            .get(right_start)
            .is_some_and(|(_, next)| next.is_whitespace())
        {
            right_start += 1;
        }

        let left = selector.get(..left_end)?.trim_end();
        let right = selector.get(right_start..)?.trim_start();
        if left.is_empty() || right.is_empty() {
            continue;
        }
        return Some((left_end, left, right));
    }

    None
}

fn simple_selector_matches(
    selector: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    let selector = selector.trim();
    if selector.is_empty() {
        return false;
    }

    let (base, pseudo) = split_pseudo_class(selector);
    if let Some(pseudo) = pseudo {
        if let Some(inner) = pseudo
            .strip_prefix(":not(")
            .and_then(|value| value.strip_suffix(')'))
        {
            if simple_selector_matches(inner, tag, classes, id, attributes, ctx) {
                return false;
            }
        } else if !pseudo_class_matches(pseudo, ctx) {
            return false;
        }
    }

    if base.is_empty() {
        return true;
    }

    if base.contains('[') {
        if let Some(bracket_index) = base.find('[') {
            let (prefix, attributes_sel) = base.split_at(bracket_index);
            if !prefix.is_empty() && !simple_selector_core_matches(prefix, tag, classes, id) {
                return false;
            }
            return attribute_selector_matches(attributes_sel, attributes);
        }
    }

    simple_selector_core_matches(base, tag, classes, id)
}

fn simple_selector_core_matches(
    selector: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
) -> bool {
    let mut remaining = selector.trim();
    if remaining.is_empty() {
        return true;
    }

    let mut type_checked = false;
    while !remaining.is_empty() {
        match remaining.chars().next() {
            Some('#') => {
                let (name, rest) = consume_simple_selector_name(&remaining[1..]);
                if name.is_empty() || !id.is_some_and(|value| value == name) {
                    return false;
                }
                remaining = rest;
            }
            Some('.') => {
                let (name, rest) = consume_simple_selector_name(&remaining[1..]);
                if name.is_empty() || !classes.iter().any(|class| class == &name) {
                    return false;
                }
                remaining = rest;
            }
            Some('*') => {
                remaining = &remaining[1..];
            }
            Some(_) if !type_checked => {
                let (name, rest) = consume_simple_selector_name(remaining);
                if name.is_empty() || name != tag {
                    return false;
                }
                type_checked = true;
                remaining = rest;
            }
            _ => return false,
        }
    }

    true
}

fn consume_simple_selector_name(selector: &str) -> (&str, &str) {
    let mut end = 0usize;
    for (index, ch) in selector.char_indices() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }

    selector.split_at(end)
}

fn split_pseudo_class(selector: &str) -> (&str, Option<&str>) {
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for (index, ch) in selector.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ':' if bracket_depth == 0 && paren_depth == 0 => {
                return (&selector[..index], Some(&selector[index..]));
            }
            _ => {}
        }
    }

    (selector, None)
}

fn pseudo_class_matches(pseudo: &str, ctx: &SelectorContext) -> bool {
    match pseudo {
        ":first-child" => ctx.child_index == 0,
        ":last-child" => ctx.child_index + 1 == ctx.sibling_count,
        _ if pseudo.starts_with(":nth-child(") && pseudo.ends_with(')') => {
            let arg = pseudo
                .trim_start_matches(":nth-child(")
                .trim_end_matches(')');
            nth_child_matches(arg, ctx.child_index)
        }
        _ => false,
    }
}

fn nth_child_matches(arg: &str, child_index: usize) -> bool {
    let n = child_index as i64 + 1;
    let normalized = arg
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();

    match normalized.as_str() {
        "odd" => return n % 2 == 1,
        "even" => return n % 2 == 0,
        _ => {}
    }

    if let Ok(value) = normalized.parse::<i64>() {
        return n == value;
    }

    let Some((a, b)) = parse_an_plus_b(&normalized) else {
        return false;
    };

    if a == 0 {
        return n == b;
    }

    let diff = n - b;
    if a > 0 {
        diff >= 0 && diff % a == 0
    } else {
        diff <= 0 && diff % a == 0
    }
}

fn parse_an_plus_b(s: &str) -> Option<(i64, i64)> {
    let n_index = s.find('n')?;
    let (a_part, b_part) = s.split_at(n_index);
    let a = match a_part.trim() {
        "" | "+" => 1,
        "-" => -1,
        value => value.parse::<i64>().ok()?,
    };
    let b = match b_part.strip_prefix('n')?.trim() {
        "" => 0,
        value => value.parse::<i64>().ok()?,
    };
    Some((a, b))
}

fn attribute_selector_matches(selector: &str, attributes: &HashMap<String, String>) -> bool {
    selector
        .split('[')
        .filter_map(|part| part.strip_suffix(']'))
        .all(|expr| single_attribute_matches(expr, attributes))
}

fn single_attribute_matches(expr: &str, attributes: &HashMap<String, String>) -> bool {
    if let Some((attr_name, attr_val)) = expr.split_once('=') {
        let attr_name = attr_name.trim();
        let attr_val = attr_val.trim().trim_matches('"').trim_matches('\'');
        return attributes
            .get(attr_name)
            .is_some_and(|value| value == attr_val);
    }
    attributes.contains_key(expr.trim())
}

pub(crate) fn ancestor_info(element: &ElementNode) -> AncestorInfo<'_> {
    AncestorInfo {
        element,
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
    }
}
