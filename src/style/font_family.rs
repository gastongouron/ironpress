use cssparser::{Parser, ParserInput};

/// One family name parsed from a CSS `font-family` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CssFontFamily {
    name: String,
    quoted: bool,
}

impl CssFontFamily {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn is_quoted(&self) -> bool {
        self.quoted
    }
}

/// A syntactically valid, non-empty CSS `font-family` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CssFontFamilyList(Vec<CssFontFamily>);

impl CssFontFamilyList {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let mut input = ParserInput::new(raw);
        let mut parser = Parser::new(&mut input);
        let families = parser
            .parse_comma_separated(|item| {
                if let Ok(name) = item.try_parse(|input| input.expect_string_cloned()) {
                    if name.is_empty() || !item.is_exhausted() {
                        return Err(item.new_custom_error::<(), ()>(()));
                    }
                    return Ok(CssFontFamily {
                        name: name.to_string(),
                        quoted: true,
                    });
                }

                let mut name = item.expect_ident_cloned()?.to_string();
                while !item.is_exhausted() {
                    name.push(' ');
                    name.push_str(item.expect_ident_cloned()?.as_ref());
                }
                Ok(CssFontFamily {
                    name,
                    quoted: false,
                })
            })
            .ok()?;

        (!families.is_empty()).then_some(Self(families))
    }

    pub(crate) fn families(&self) -> &[CssFontFamily] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::CssFontFamilyList;

    #[test]
    fn parses_quoted_commas_and_css_escapes_without_losing_authored_names() {
        let list = CssFontFamilyList::parse(r#""My, Face", Georg\69 a, serif"#).unwrap();
        let names: Vec<_> = list
            .families()
            .iter()
            .map(|family| (family.name(), family.is_quoted()))
            .collect();

        assert_eq!(
            names,
            vec![("My, Face", true), ("Georgia", false), ("serif", false)]
        );
    }

    #[test]
    fn rejects_empty_or_malformed_lists() {
        assert!(CssFontFamilyList::parse("").is_none());
        assert!(CssFontFamilyList::parse("one,").is_none());
    }
}
