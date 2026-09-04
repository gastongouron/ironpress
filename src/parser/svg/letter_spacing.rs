//! `letter-spacing` as authored on SVG `<text>`.

use std::collections::HashMap;

use crate::parser::css::{CssValue, MathUnitContext, parse_length};
use crate::style::resolve::{LengthResolutionContext, resolve_length_value_in_context};

/// CSS points per SVG user unit: user units are CSS pixels (SVG 2 §8.2) and
/// the canonical `CssValue::Length` is kept in points.
const POINTS_PER_USER_UNIT: f32 = 0.75;

/// The UA initial `font-size` (`medium`, 16px) in points. SVG text reaches the
/// renderer without its document's root element, so `rem` resolves against
/// this initial value.
const INITIAL_ROOT_FONT_SIZE_POINTS: f32 = 12.0;

/// Where an SVG length was written; the two syntaxes disagree on bare numbers.
#[derive(Debug, Clone, Copy)]
enum SvgLengthSyntax {
    /// A presentation attribute (`letter-spacing="4"`): an SVG length accepts a
    /// unitless number as user units (SVG 1.1 §4.2).
    PresentationAttribute,
    /// An inline `style` declaration: CSS syntax, where a non-zero bare number
    /// is not a `<length>` and the declaration is invalid (CSS Values 4 §5.1).
    StyleDeclaration,
}

/// `letter-spacing` on an SVG `<text>` element, kept as specified until the
/// text's used font size is known.
///
/// SVG exposes the CSS property both as a presentation attribute and as an
/// inline style declaration (SVG 1.1 §10.11; CSS Text 3 §8.2). A valid style
/// declaration wins over the attribute, and an invalid one is dropped so the
/// attribute still applies, as in the CSS cascade. Font-relative units resolve
/// against the element's own used font size, which the renderer only knows
/// once the inherited text context is applied, so the value stays in the
/// canonical CSS value model until [`resolve`](Self::resolve).
#[derive(Debug, Clone, Default)]
pub enum SvgLetterSpacing {
    /// `normal`, the initial value: no tracking.
    #[default]
    Normal,
    /// A specified `<length>`. CSS Text 3 §8.2 defines no percentage basis for
    /// the property, so percentages never reach this variant.
    Tracked(CssValue),
}

impl SvgLetterSpacing {
    pub(crate) const fn normal() -> Self {
        Self::Normal
    }

    pub(crate) fn parse_presentation_attribute(raw: &str) -> Option<Self> {
        Self::parse(raw, SvgLengthSyntax::PresentationAttribute)
    }

    pub(crate) fn from_css_value(value: &CssValue) -> Option<Self> {
        match value {
            CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("normal") => {
                Some(Self::Normal)
            }
            CssValue::Number(value) if *value == 0.0 => Some(Self::Tracked(CssValue::Length(0.0))),
            CssValue::Number(_) | CssValue::Percentage(_) | CssValue::Keyword(_) => None,
            value => Some(Self::Tracked(value.clone())),
        }
    }

    /// Parse the cascaded value from one element's inline `style` declaration
    /// and presentation attribute.
    pub(crate) fn from_declarations(style: Option<&str>, attribute: Option<&str>) -> Self {
        style
            .and_then(|raw| Self::parse(raw, SvgLengthSyntax::StyleDeclaration))
            .or_else(|| {
                attribute.and_then(|raw| Self::parse(raw, SvgLengthSyntax::PresentationAttribute))
            })
            .unwrap_or_default()
    }

    fn parse(raw: &str, syntax: SvgLengthSyntax) -> Option<Self> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("normal") {
            return Some(Self::Normal);
        }
        match parse_length(raw)? {
            CssValue::Number(user_units) => match syntax {
                SvgLengthSyntax::PresentationAttribute => Some(Self::Tracked(CssValue::Length(
                    user_units * POINTS_PER_USER_UNIT,
                ))),
                SvgLengthSyntax::StyleDeclaration => None,
            },
            CssValue::Percentage(_) | CssValue::Keyword(_) => None,
            length => Some(Self::Tracked(length)),
        }
    }

    /// The tracking in SVG user units for text set at `font_size` user units.
    ///
    /// Resolution goes through the shared CSS length resolver so `em`, `ex`,
    /// `ch`, `rem`, the absolute units, and `calc()` follow the same rules as
    /// HTML text. SVG text has no page viewport of its own, so viewport
    /// units resolve to zero, and `rem` uses the UA initial font size.
    pub(crate) fn resolve(&self, font_size: f32) -> f32 {
        let Self::Tracked(value) = self else {
            return 0.0;
        };
        let font_size_points = font_size * POINTS_PER_USER_UNIT;
        let context = LengthResolutionContext::new(
            font_size_points,
            MathUnitContext::from_font_and_viewport(
                font_size_points,
                INITIAL_ROOT_FONT_SIZE_POINTS,
                0.0,
                0.0,
            ),
        );
        resolve_length_value_in_context(value, context, &HashMap::new())
            .map(|points| points / POINTS_PER_USER_UNIT)
            .filter(|tracking| tracking.is_finite())
            .unwrap_or(0.0)
    }

    pub(crate) fn resolve_user_units(&self, font_size: f32) -> f32 {
        self.resolve(font_size)
    }
}

#[cfg(test)]
mod tests {
    use super::SvgLetterSpacing;

    fn resolve(style: Option<&str>, attribute: Option<&str>, font_size: f32) -> f32 {
        SvgLetterSpacing::from_declarations(style, attribute).resolve(font_size)
    }

    #[test]
    fn absent_and_normal_resolve_to_zero() {
        assert_eq!(resolve(None, None, 32.0), 0.0);
        assert_eq!(resolve(None, Some("normal"), 32.0), 0.0);
        assert_eq!(resolve(Some(" NORMAL "), Some("8"), 32.0), 0.0);
    }

    #[test]
    fn unitless_attribute_is_user_units() {
        assert_eq!(resolve(None, Some("8"), 32.0), 8.0);
        assert_eq!(resolve(None, Some("-2"), 32.0), -2.0);
    }

    #[test]
    fn unitless_style_declaration_is_invalid_and_falls_back_to_the_attribute() {
        assert_eq!(resolve(Some("8"), Some("4"), 32.0), 4.0);
        assert_eq!(resolve(Some("8"), None, 32.0), 0.0);
        assert_eq!(resolve(Some("0"), Some("4"), 32.0), 0.0);
    }

    #[test]
    fn em_resolves_against_the_used_font_size() {
        assert_eq!(resolve(None, Some("0.25em"), 32.0), 8.0);
        assert_eq!(resolve(Some("0.25em"), None, 40.0), 10.0);
    }

    #[test]
    fn absolute_units_convert_to_user_units() {
        assert!((resolve(None, Some("6pt"), 32.0) - 8.0).abs() < 1e-4);
        assert!((resolve(None, Some("8px"), 32.0) - 8.0).abs() < 1e-4);
        assert!((resolve(None, Some("0.1in"), 32.0) - 9.6).abs() < 1e-3);
    }

    #[test]
    fn calc_and_ex_use_the_shared_resolver() {
        assert!((resolve(None, Some("calc(0.25em + 4px)"), 32.0) - 12.0).abs() < 1e-3);
        assert!((resolve(None, Some("1ex"), 32.0) - 16.0).abs() < 1e-3);
    }

    #[test]
    fn percentages_and_non_finite_values_are_rejected() {
        assert_eq!(resolve(None, Some("50%"), 32.0), 0.0);
        assert_eq!(resolve(Some("50%"), Some("4"), 32.0), 4.0);
        assert_eq!(resolve(None, Some("NaNpx"), 32.0), 0.0);
        assert_eq!(resolve(None, Some("infpx"), 32.0), 0.0);
    }

    #[test]
    fn style_declaration_wins_over_the_attribute() {
        assert_eq!(resolve(Some("0.5em"), Some("2"), 20.0), 10.0);
    }
}
