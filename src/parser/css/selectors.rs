use std::collections::HashMap;

use crate::parser::dom::{ElementNode, HtmlTag};

use super::{AncestorInfo, SelectorContext};

#[derive(Clone, Copy)]
enum Combinator {
    GeneralSibling,
    AdjacentSibling,
    Child,
    Descendant,
}

/// Check if a CSS selector matches a given element (backward-compatible, no context).
#[cfg(test)]
pub(crate) fn selector_matches(
    selector: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
) -> bool {
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
    split_selector_list(selector)
        .into_iter()
        .any(|part| compound_selector_matches(part, tag, classes, id, attributes, ctx))
}

/// CSS Selectors-4 §17 specificity, packed as `(a << 20) | (b << 10) | c`
/// where `a` = id count, `b` = class/attr/pseudo-class count, `c` =
/// type/pseudo-element count. The packed form gives a single sortable integer
/// for the cascade (css-cascade-4 §6.3). For a selector list (commas) the
/// highest specificity among the alternatives is returned.
pub fn specificity(selector: &str) -> u32 {
    split_selector_list(selector)
        .into_iter()
        .map(|part| {
            let (a, b, c) = complex_specificity(part);
            (a.min(1023) << 20) | (b.min(1023) << 10) | c.min(1023)
        })
        .max()
        .unwrap_or(0)
}

/// Compute the raw (a, b, c) triple for a single complex selector.
fn complex_specificity(selector: &str) -> (u32, u32, u32) {
    let mut total = (0u32, 0u32, 0u32);
    // Walk each compound separated by combinators; combinators add nothing.
    for compound in split_into_compounds(selector) {
        let (a, b, c) = compound_specificity(compound);
        total.0 += a;
        total.1 += b;
        total.2 += c;
    }
    total
}

/// Split a complex selector into its compound pieces, dropping the combinators
/// (which contribute zero specificity). Respects bracket/paren nesting so that
/// combinator characters inside `[...]`/`(...)` are not treated as separators.
fn split_into_compounds(selector: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut last_was_sep = false;
    for (byte_index, ch) in selector.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
        let is_sep = bracket_depth == 0
            && paren_depth == 0
            && (ch.is_whitespace() || ch == '>' || ch == '+' || ch == '~');
        if is_sep {
            if !last_was_sep {
                let piece = selector[start..byte_index].trim();
                if !piece.is_empty() {
                    pieces.push(piece);
                }
            }
            // Advance start past this separator char.
            start = byte_index + ch.len_utf8();
            last_was_sep = true;
        } else {
            last_was_sep = false;
        }
    }
    let tail = selector[start..].trim();
    if !tail.is_empty() {
        pieces.push(tail);
    }
    pieces
}

/// Specificity (a, b, c) of a single compound selector (no combinators).
fn compound_specificity(compound: &str) -> (u32, u32, u32) {
    let (head, pseudos) = split_compound(compound);
    let (mut a, mut b, mut c) = head_specificity(head);

    for pseudo in pseudos {
        // Functional logical pseudos contribute the specificity of their
        // argument list (most specific item); :where() contributes zero.
        if let Some(arg) = functional_arg(pseudo, ":where(") {
            let _ = arg; // zero contribution
        } else if let Some(arg) = functional_arg(pseudo, ":is(")
            .or_else(|| functional_arg(pseudo, ":not("))
            .or_else(|| functional_arg(pseudo, ":has("))
        {
            let (na, nb, nc) = split_selector_list(arg)
                .into_iter()
                .map(complex_specificity)
                .max_by_key(|(a, b, c)| (*a, *b, *c))
                .unwrap_or((0, 0, 0));
            a += na;
            b += nb;
            c += nc;
        } else if let Some(arg) = functional_arg(pseudo, ":nth-child(")
            .or_else(|| functional_arg(pseudo, ":nth-last-child("))
        {
            b += 1;
            if let Some((_, selector_list)) = split_nth_of(arg) {
                let (na, nb, nc) = split_selector_list(selector_list)
                    .into_iter()
                    .map(complex_specificity)
                    .max_by_key(|(a, b, c)| (*a, *b, *c))
                    .unwrap_or((0, 0, 0));
                a += na;
                b += nb;
                c += nc;
            }
        } else {
            // Plain (or functional structural) pseudo-class: counts as a class.
            b += 1;
        }
    }

    (a, b, c)
}

/// Specificity of the non-pseudo head (tag/universal + ids + classes + attrs).
fn head_specificity(head: &str) -> (u32, u32, u32) {
    // Splice out every `[...]` attribute selector, counting each toward `b`,
    // then score the remaining tag/class/id part.
    let mut attr_count = 0u32;
    let mut simple = String::with_capacity(head.len());
    let mut chars = head.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '[' {
            attr_count += 1;
            // Skip until the matching `]`.
            for (_, c) in chars.by_ref() {
                if c == ']' {
                    break;
                }
            }
        } else {
            simple.push(ch);
            let _ = i;
        }
    }
    let (a, b, c) = simple_part_specificity(&simple);
    (a, b + attr_count, c)
}

/// Specificity of a simple compound with no attribute selectors: an optional
/// type/universal part plus `.class` / `#id` segments.
fn simple_part_specificity(part: &str) -> (u32, u32, u32) {
    let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
    if part.is_empty() {
        return (a, b, c);
    }
    let first_delim = part.find(['.', '#']);
    let (tag_part, segs) = match first_delim {
        Some(i) => part.split_at(i),
        None => (part, ""),
    };
    // A type selector adds to c; `*` and an empty type part add nothing.
    if !tag_part.is_empty() && tag_part != "*" {
        c += 1;
    }
    let mut chars = segs.char_indices().peekable();
    while let Some((start, marker)) = chars.next() {
        let mut end = segs.len();
        while let Some(&(idx, ch)) = chars.peek() {
            if ch == '.' || ch == '#' {
                end = idx;
                break;
            }
            chars.next();
        }
        let _name = &segs[start + 1..end];
        match marker {
            '#' => a += 1,
            '.' => b += 1,
            _ => {}
        }
    }
    (a, b, c)
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
                        .next_back()
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

                if let Some((parent_index, parent)) = ctx.ancestors.iter().enumerate().next_back() {
                    let parent_ctx = ancestor_selector_context(ctx, parent_index);
                    compound_selector_matches(
                        left,
                        &parent.element.raw_tag_name,
                        &parent.element.class_list(),
                        parent.element.id(),
                        &parent.element.attributes,
                        &parent_ctx,
                    )
                } else {
                    selector_matches_virtual_body(left)
                }
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
                selector_matches_virtual_document_ancestor(left)
            }
        };
    }

    simple_selector_matches(selector, tag, classes, id, attributes, ctx)
}

fn sibling_selector_context<'a>(
    ctx: &'a SelectorContext<'a>,
    sibling_index: usize,
) -> SelectorContext<'a> {
    // Following siblings of the matched left-hand sibling: every sibling after
    // it, i.e. the remaining preceding siblings of the anchor plus the anchor's
    // own following siblings. We approximate with the preceding-sibling slice
    // beyond `sibling_index` (the part of the anchor's preceding list that comes
    // after this sibling); forward siblings past the anchor are not needed for
    // sibling-combinator left-hand matching.
    let following_siblings: Vec<(String, Vec<String>)> = ctx
        .preceding_siblings
        .iter()
        .skip(sibling_index + 1)
        .cloned()
        .collect();
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
        following_siblings,
        is_empty: false,
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
        following_siblings: ancestor.following_siblings.clone(),
        is_empty: ancestor.is_empty,
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
        if let Some((byte_index, left, right)) =
            split_on_spaced_combinator(selector, combinator_char)
        {
            match candidate {
                Some((current_index, _, _, _)) if current_index > byte_index => {}
                _ => candidate = Some((byte_index, combinator, left, right)),
            }
        }
    }

    candidate.map(|(_, combinator, left, right)| (combinator, left, right))
}

fn split_on_spaced_combinator(selector: &str, combinator: char) -> Option<(usize, &str, &str)> {
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

        let Some((left_space_index, ' ')) = index
            .checked_sub(1)
            .and_then(|prev_index| chars.get(prev_index).copied())
        else {
            continue;
        };
        let Some((right_space_index, ' ')) = chars.get(index + 1).copied() else {
            continue;
        };
        let right_start = right_space_index + ' '.len_utf8();
        let left = selector.get(..left_space_index)?.trim();
        let right = selector.get(right_start..)?.trim();
        return Some((left_space_index, left, right));
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

    // Split the compound selector into its non-pseudo head (tag/id/class/attr)
    // and the list of trailing pseudo-class tokens. A compound may carry several
    // pseudos (e.g. `.box:nth-child(2):not(.skip)`); every one must match.
    let (base, pseudos) = split_compound(selector);

    for pseudo in &pseudos {
        if !pseudo_matches(pseudo, tag, classes, id, attributes, ctx) {
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
    if selector.is_empty() {
        return true;
    }

    // A compound selector is an optional tag part followed by any number of
    // `.class` and `#id` segments, all of which must match (e.g. `.card.alt`,
    // `div.box#hero`). Split at each `.`/`#` boundary, keeping the delimiter so
    // we know whether each segment is a class or an id. The leading run (before
    // the first delimiter) is the type/universal part.
    let first_delim = selector.find(['.', '#']);
    let (tag_part, rest) = match first_delim {
        Some(i) => selector.split_at(i),
        None => (selector, ""),
    };

    // A `*` tag part matches any element (universal selector), just like an
    // empty tag part. This applies to bare `*`, `*#id`, and `*.class`.
    if !(tag_part.is_empty() || tag_part == "*" || tag_part == tag) {
        return false;
    }

    if rest.is_empty() {
        return true;
    }

    // Walk each `.class` / `#id` segment; every one must match.
    let mut chars = rest.char_indices().peekable();
    while let Some((start, marker)) = chars.next() {
        // Find the end of this segment (next `.`/`#` or end of string).
        let mut end = rest.len();
        while let Some(&(idx, c)) = chars.peek() {
            if c == '.' || c == '#' {
                end = idx;
                break;
            }
            chars.next();
        }
        let name = &rest[start + 1..end];
        match marker {
            '.' => {
                if !classes.contains(&name) {
                    return false;
                }
            }
            '#' => {
                if id != Some(name) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Split a compound selector into its non-pseudo head and the list of trailing
/// pseudo-class tokens. Each pseudo token retains its leading `:` and, for
/// functional pseudos, its parenthesised argument (e.g. `:nth-child(2n+1)`,
/// `:not(.a, .b)`). Brackets and parentheses are tracked so a `:` inside an
/// attribute value (`[x=":"]`) or a functional argument (`:not(:first-child)`)
/// does not terminate the head.
fn split_compound(selector: &str) -> (&str, Vec<&str>) {
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut head_end = selector.len();
    let mut pseudo_starts: Vec<usize> = Vec::new();

    for (byte_index, ch) in selector.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ':' if bracket_depth == 0 && paren_depth == 0 => {
                // A `::` introduces a pseudo-element, which is stripped earlier;
                // treat any `:` at top level as a pseudo-class boundary.
                if pseudo_starts.is_empty() {
                    head_end = byte_index;
                }
                pseudo_starts.push(byte_index);
            }
            _ => {}
        }
    }

    let head = &selector[..head_end];
    let mut pseudos = Vec::with_capacity(pseudo_starts.len());
    for (i, &start) in pseudo_starts.iter().enumerate() {
        let end = pseudo_starts.get(i + 1).copied().unwrap_or(selector.len());
        // Skip a leading `:` belonging to a `::pseudo-element` written with a
        // single colon split — only collect real pseudo-class tokens.
        let token = &selector[start..end];
        if token.starts_with("::") {
            continue;
        }
        pseudos.push(token);
    }
    (head, pseudos)
}

/// Match a single pseudo-class token (with its `:` prefix) against the element.
fn pseudo_matches(
    pseudo: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    // Functional logical pseudo-classes operate on a selector list argument.
    if let Some(arg) = functional_arg(pseudo, ":not(") {
        // :not() matches when NONE of the listed compound selectors match.
        return !selector_list_matches(arg, tag, classes, id, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":is(") {
        return selector_list_matches(arg, tag, classes, id, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":where(") {
        return selector_list_matches(arg, tag, classes, id, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":has(") {
        return has_matches(arg, tag, classes, id, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":dir(") {
        return dir_matches(arg, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":lang(") {
        return lang_matches(arg, attributes, ctx);
    }
    if let Some(arg) = functional_arg(pseudo, ":nth-child(") {
        if let Some((formula, selector_list)) = split_nth_of(arg) {
            return nth_child_of_matches(
                formula,
                selector_list,
                tag,
                classes,
                id,
                attributes,
                ctx,
                false,
            );
        }
        return nth_child_matches(arg, ctx.child_index);
    }
    if let Some(arg) = functional_arg(pseudo, ":nth-last-child(") {
        if let Some((formula, selector_list)) = split_nth_of(arg) {
            return nth_child_of_matches(
                formula,
                selector_list,
                tag,
                classes,
                id,
                attributes,
                ctx,
                true,
            );
        }
        let from_end = ctx.sibling_count.saturating_sub(ctx.child_index + 1);
        return nth_child_matches(arg, from_end);
    }
    if let Some(arg) = functional_arg(pseudo, ":nth-of-type(") {
        return nth_child_matches(arg, type_index_from_start(tag, ctx));
    }
    if let Some(arg) = functional_arg(pseudo, ":nth-last-of-type(") {
        return nth_child_matches(arg, type_index_from_end(tag, ctx));
    }

    match pseudo {
        ":first-child" => ctx.child_index == 0,
        ":last-child" => ctx.child_index + 1 == ctx.sibling_count,
        ":only-child" => ctx.sibling_count == 1,
        ":empty" => ctx.is_empty,
        ":root" => ctx.ancestors.is_empty() && tag.eq_ignore_ascii_case("html"),
        // :scope without an explicit scoping root matches the document root in
        // print contexts (no :scope attribute is set), mirroring :root.
        ":scope" => ctx.ancestors.is_empty(),
        // HTML built-in elements are always defined. Unknown/custom elements
        // would require a custom-element registry, which this static renderer
        // does not model.
        ":defined" => tag != "unknown",
        ":first-of-type" => type_index_from_start(tag, ctx) == 0,
        ":last-of-type" => type_index_from_end(tag, ctx) == 0,
        ":only-of-type" => {
            type_index_from_start(tag, ctx) == 0 && type_index_from_end(tag, ctx) == 0
        }
        // Dynamic / UI pseudo-classes never match in a static print context.
        _ => false,
    }
}

/// If `pseudo` is the functional pseudo named by `prefix` (e.g. `:not(`),
/// return its inner argument with the trailing `)` stripped.
fn functional_arg<'a>(pseudo: &'a str, prefix: &str) -> Option<&'a str> {
    pseudo
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(')'))
}

/// Match a comma-separated selector list as the argument of logical and
/// structural pseudo-classes. Returns true if ANY item matches this element.
fn selector_list_matches(
    list: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    split_selector_list(list)
        .into_iter()
        .any(|item| compound_selector_matches(item, tag, classes, id, attributes, ctx))
}

/// Split a selector list on top-level commas (ignoring commas inside `[]`/`()`).
fn split_selector_list(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in list.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ',' if bracket_depth == 0 && paren_depth == 0 => {
                let piece = list[start..index].trim();
                if !piece.is_empty() {
                    parts.push(piece);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = list[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

/// Evaluate `:has(<relative-selector-list>)` against the element's known
/// context. Sibling forms use the sibling lists in `SelectorContext`; child and
/// descendant class selectors use synthetic class summaries supplied by layout
/// in the element attributes map.
fn has_matches(
    arg: &str,
    _tag: &str,
    _classes: &[&str],
    _id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    split_selector_list(arg)
        .into_iter()
        .any(|relative| has_relative_selector_matches(relative, attributes, ctx))
}

fn has_relative_selector_matches(
    relative: &str,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    let arg = relative.trim();
    let (combinator, target) = if let Some(rest) = arg.strip_prefix('+') {
        ('+', rest.trim())
    } else if let Some(rest) = arg.strip_prefix('~') {
        ('~', rest.trim())
    } else if let Some(rest) = arg.strip_prefix('>') {
        ('>', rest.trim())
    } else {
        (' ', arg)
    };

    match combinator {
        // Adjacent following sibling matches the target.
        '+' => ctx.following_siblings.first().is_some_and(|(t, c)| {
            let refs: Vec<&str> = c.iter().map(String::as_str).collect();
            compound_selector_matches(target, t, &refs, None, &HashMap::new(), &empty_ctx())
        }),
        // Any following sibling matches the target.
        '~' => ctx.following_siblings.iter().any(|(t, c)| {
            let refs: Vec<&str> = c.iter().map(String::as_str).collect();
            compound_selector_matches(target, t, &refs, None, &HashMap::new(), &empty_ctx())
        }),
        '>' => synthetic_has_class(attributes, "__ironpress_has_child_classes", target),
        ' ' => synthetic_has_class(attributes, "__ironpress_has_descendant_classes", target),
        _ => false,
    }
}

fn synthetic_has_class(attributes: &HashMap<String, String>, key: &str, selector: &str) -> bool {
    let Some(classes) = attributes.get(key) else {
        return false;
    };
    let Some(required) = required_classes(selector) else {
        return false;
    };
    required
        .iter()
        .all(|class| classes.split_whitespace().any(|present| present == class))
}

fn required_classes(selector: &str) -> Option<Vec<String>> {
    let selector = selector.trim();
    if selector.is_empty() || selector.contains([' ', '>', '+', '~', '[', ':']) {
        return None;
    }
    let mut classes = Vec::new();
    for part in selector.split('.') {
        if part.is_empty() {
            continue;
        }
        if part.contains('#') {
            return None;
        }
        if classes.is_empty() && !selector.starts_with('.') {
            continue;
        }
        classes.push(part.to_string());
    }
    (!classes.is_empty()).then_some(classes)
}

fn dir_matches(arg: &str, attributes: &HashMap<String, String>, ctx: &SelectorContext) -> bool {
    let want = unquote(arg.trim()).to_ascii_lowercase();
    if !matches!(want.as_str(), "ltr" | "rtl") {
        return false;
    }

    match inherited_attribute_value(attributes, ctx, &["dir"])
        .as_deref()
        .map(|dir| dir.to_ascii_lowercase())
        .as_deref()
    {
        Some("ltr") => want == "ltr",
        Some("rtl") => want == "rtl",
        // `auto` needs text-direction analysis, which is outside this matcher.
        Some("auto") => false,
        // HTML's default directionality is left-to-right.
        _ => want == "ltr",
    }
}

fn lang_matches(arg: &str, attributes: &HashMap<String, String>, ctx: &SelectorContext) -> bool {
    let lang = inherited_attribute_value(attributes, ctx, &["lang", "xml:lang"])
        .unwrap_or_else(|| "en".to_string())
        .to_ascii_lowercase();

    split_selector_list(arg).into_iter().any(|range| {
        let range = unquote(range.trim()).to_ascii_lowercase();
        if range == "*" {
            return true;
        }
        lang == range
            || lang
                .strip_prefix(&range)
                .is_some_and(|suffix| suffix.starts_with('-'))
    })
}

fn inherited_attribute_value(
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
    names: &[&str],
) -> Option<String> {
    find_attribute_value(attributes, names)
        .map(str::to_string)
        .or_else(|| {
            ctx.ancestors.iter().rev().find_map(|ancestor| {
                find_attribute_value(&ancestor.element.attributes, names).map(str::to_string)
            })
        })
}

fn find_attribute_value<'a>(
    attributes: &'a HashMap<String, String>,
    names: &[&str],
) -> Option<&'a str> {
    attributes.iter().find_map(|(name, value)| {
        names
            .iter()
            .any(|want| name.eq_ignore_ascii_case(want))
            .then_some(value.as_str())
    })
}

#[allow(clippy::too_many_arguments)]
fn nth_child_of_matches(
    formula: &str,
    selector_list: &str,
    tag: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
    from_end: bool,
) -> bool {
    if !selector_list_matches(selector_list, tag, classes, id, attributes, ctx) {
        return false;
    }

    let preceding = matching_sibling_count(&ctx.preceding_siblings, selector_list);
    let following = matching_sibling_count(&ctx.following_siblings, selector_list);
    let filtered_index = if from_end { following } else { preceding };
    nth_child_matches(formula, filtered_index)
}

fn matching_sibling_count(siblings: &[(String, Vec<String>)], selector_list: &str) -> usize {
    siblings
        .iter()
        .filter(|(tag, classes)| {
            let refs: Vec<&str> = classes.iter().map(String::as_str).collect();
            selector_list_matches(
                selector_list,
                tag,
                &refs,
                None,
                &HashMap::new(),
                &empty_ctx(),
            )
        })
        .count()
}

fn split_nth_of(arg: &str) -> Option<(&str, &str)> {
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for (index, ch) in arg.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            'o' | 'O' if bracket_depth == 0 && paren_depth == 0 => {
                let rest = &arg[index..];
                if !rest
                    .get(..2)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("of"))
                {
                    continue;
                }
                let before_is_space = arg[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
                let after_is_space = arg[index + 2..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace);
                if before_is_space && after_is_space {
                    let formula = arg[..index].trim();
                    let selector_list = arg[index + 2..].trim();
                    if !formula.is_empty() && !selector_list.is_empty() {
                        return Some((formula, selector_list));
                    }
                }
            }
            _ => {}
        }
    }

    None
}

fn selector_matches_virtual_document_ancestor(selector: &str) -> bool {
    selector_matches_virtual_body(selector) || selector_matches_virtual_html(selector)
}

fn selector_matches_virtual_html(selector: &str) -> bool {
    let attributes = HashMap::new();
    compound_selector_matches(
        selector,
        "html",
        &[],
        None,
        &attributes,
        &SelectorContext::default(),
    )
}

fn selector_matches_virtual_body(selector: &str) -> bool {
    let html = ElementNode {
        tag: HtmlTag::Html,
        raw_tag_name: "html".to_string(),
        attributes: HashMap::new(),
        children: Vec::new(),
    };
    let attributes = HashMap::new();
    let ctx = SelectorContext {
        ancestors: vec![AncestorInfo {
            element: &html,
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        }],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };

    compound_selector_matches(selector, "body", &[], None, &attributes, &ctx)
}

fn empty_ctx<'a>() -> SelectorContext<'a> {
    SelectorContext::default()
}

/// Zero-based index of this element among siblings of the SAME element type,
/// counting from the start (for `:nth-of-type` / `:first-of-type`).
fn type_index_from_start(tag: &str, ctx: &SelectorContext) -> usize {
    ctx.preceding_siblings
        .iter()
        .filter(|(t, _)| t.eq_ignore_ascii_case(tag))
        .count()
}

/// Zero-based index of this element among siblings of the SAME element type,
/// counting from the end (for `:nth-last-of-type` / `:last-of-type`).
fn type_index_from_end(tag: &str, ctx: &SelectorContext) -> usize {
    ctx.following_siblings
        .iter()
        .filter(|(t, _)| t.eq_ignore_ascii_case(tag))
        .count()
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

/// Match a single attribute selector expression (the text between `[` and `]`)
/// per Selectors-4 §6: presence and the six value operators (`=`, `~=`, `|=`,
/// `^=`, `$=`, `*=`), plus the trailing case-sensitivity flag (`i` = ASCII
/// case-insensitive, `s` = case-sensitive).
fn single_attribute_matches(expr: &str, attributes: &HashMap<String, String>) -> bool {
    let expr = expr.trim();

    // Strip a trailing case-sensitivity flag: `attr=val i` / `attr=val s`.
    let (expr, case_insensitive) = if let Some(stripped) = strip_case_flag(expr, 'i') {
        (stripped, true)
    } else if let Some(stripped) = strip_case_flag(expr, 's') {
        (stripped, false)
    } else {
        (expr, false)
    };

    // Locate the operator: one of `~= |= ^= $= *=` or a bare `=`.
    let op_index = expr.find(['=', '~', '|', '^', '$', '*']);
    let Some(op_index) = op_index else {
        // Presence selector `[attr]`.
        return attributes
            .keys()
            .any(|k| k.eq_ignore_ascii_case(expr.trim()));
    };

    let (name_part, op_and_value) = expr.split_at(op_index);
    let (op, value_part): (&str, &str) = if let Some(rest) = op_and_value.strip_prefix("~=") {
        ("~=", rest)
    } else if let Some(rest) = op_and_value.strip_prefix("|=") {
        ("|=", rest)
    } else if let Some(rest) = op_and_value.strip_prefix("^=") {
        ("^=", rest)
    } else if let Some(rest) = op_and_value.strip_prefix("$=") {
        ("$=", rest)
    } else if let Some(rest) = op_and_value.strip_prefix("*=") {
        ("*=", rest)
    } else if let Some(rest) = op_and_value.strip_prefix('=') {
        ("=", rest)
    } else {
        return false;
    };

    let attr_name = name_part.trim();
    let want = unquote(value_part.trim());

    // Attribute names are ASCII case-insensitive in HTML.
    let Some(have) = attributes
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(attr_name))
        .map(|(_, v)| v.as_str())
    else {
        return false;
    };

    // An empty value never matches the substring/prefix/suffix/word operators.
    let eq = |a: &str, b: &str| {
        if case_insensitive {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let contains = |hay: &str, needle: &str| {
        if case_insensitive {
            hay.to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        } else {
            hay.contains(needle)
        }
    };

    match op {
        "=" => eq(have, want),
        "~=" => !want.is_empty() && have.split_ascii_whitespace().any(|word| eq(word, want)),
        "|=" => {
            eq(have, want) || {
                let prefix = format!("{want}-");
                if case_insensitive {
                    have.to_ascii_lowercase()
                        .starts_with(&prefix.to_ascii_lowercase())
                } else {
                    have.starts_with(&prefix)
                }
            }
        }
        "^=" => {
            !want.is_empty()
                && if case_insensitive {
                    have.to_ascii_lowercase()
                        .starts_with(&want.to_ascii_lowercase())
                } else {
                    have.starts_with(want)
                }
        }
        "$=" => {
            !want.is_empty()
                && if case_insensitive {
                    have.to_ascii_lowercase()
                        .ends_with(&want.to_ascii_lowercase())
                } else {
                    have.ends_with(want)
                }
        }
        "*=" => !want.is_empty() && contains(have, want),
        _ => false,
    }
}

/// Strip a trailing ` i` / ` s` case flag (Selectors-4 §6.3) from an attribute
/// expression, returning the expression without the flag if present.
fn strip_case_flag(expr: &str, flag: char) -> Option<&str> {
    let trimmed = expr.trim_end();
    let without = trimmed.strip_suffix(flag)?;
    // The flag must be a standalone token: preceded by whitespace and the value
    // must contain an operator (so a bare `[attr]` ending in `i` isn't stripped).
    if without.ends_with(char::is_whitespace) && without.contains('=') {
        Some(without.trim_end())
    } else {
        None
    }
}

/// Remove surrounding single or double quotes from an attribute value token.
fn unquote(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

#[cfg(test)]
pub(crate) fn ancestor_info(element: &ElementNode) -> AncestorInfo<'_> {
    AncestorInfo {
        element,
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    }
}
