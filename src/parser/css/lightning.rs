use lightningcss::rules::CssRule as LightningRule;
use lightningcss::stylesheet::{
    ParserFlags, ParserOptions, PrinterOptions, StyleAttribute, StyleSheet,
};
use lightningcss::traits::ToCss;

use super::{CssRule, StyleMap, inline::apply_declaration, rules::extract_pseudo_element};

pub(super) fn parse_inline_style_with_lightning(style: &str) -> Option<StyleMap> {
    let attribute = StyleAttribute::parse(style, parser_options(style)).ok()?;
    Some(declaration_block_to_style_map(&attribute.declarations))
}

pub(super) fn parse_stylesheet_rules_with_lightning(css: &str) -> Option<Vec<CssRule>> {
    let stylesheet = StyleSheet::parse(css, parser_options(css)).ok()?;
    let mut rules = Vec::new();

    for rule in &stylesheet.rules.0 {
        if let LightningRule::Style(style_rule) = rule {
            let declarations = declaration_block_to_style_map(&style_rule.declarations);
            if declarations.properties.is_empty() {
                continue;
            }

            for selector in &style_rule.selectors.0 {
                let selector = selector.to_css_string(PrinterOptions::default()).ok()?;
                let (selector, pseudo_element) = extract_pseudo_element(&selector);
                rules.push(CssRule {
                    selector,
                    declarations: declarations.clone(),
                    pseudo_element,
                });
            }
        }
    }

    Some(rules)
}

fn declaration_block_to_style_map(
    declarations: &lightningcss::declaration::DeclarationBlock<'_>,
) -> StyleMap {
    let mut map = StyleMap::new();

    for (property, is_important) in declarations.iter() {
        let property_id = property.property_id();
        let property_name = property_id.name().to_string();
        let value = match property.value_to_css_string(PrinterOptions::default()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        apply_declaration(&mut map, &property_name, &value, is_important);
    }

    map
}

fn parser_options<'i>(input: &'i str) -> ParserOptions<'static, 'i> {
    let _ = input;
    ParserOptions {
        filename: String::new(),
        css_modules: None,
        source_index: 0,
        error_recovery: true,
        warnings: None,
        flags: ParserFlags::empty(),
    }
}
