//! Font-relative lengths measured once per font rather than once per style.
//!
//! `ex` and `ch` come from the face a style selects: choosing the face walks
//! the font stack, and the used x-height hints its outlines. Every computed
//! style needs them, but nearly every element selects its parent's font. A
//! style therefore keeps the lengths it measured together with the font they
//! belong to, and a child, which starts as a copy of its parent, reuses them
//! while its font stays the same.

use crate::parser::css::FontStretch;
use crate::style::computed::{ComputedStyle, FontStack, FontStyle, FontWeight};
use crate::style::font_metrics::FontMetrics;

/// Everything [`FontMetrics`] reads from a style to choose and measure a face,
/// kept whole so that two selections are equal only when the face and the
/// size are certainly the same.
#[derive(Debug, Clone, PartialEq)]
struct FontSelection {
    stack: FontStack,
    weight: FontWeight,
    style: FontStyle,
    stretch: FontStretch,
    size: f32,
}

impl FontSelection {
    fn of(style: &ComputedStyle) -> Self {
        Self {
            stack: style.font_stack.clone(),
            weight: style.font_weight,
            style: style.font_style,
            stretch: style.font_stretch,
            size: style.font_size,
        }
    }

    fn is_selected_by(&self, style: &ComputedStyle) -> bool {
        self.size.to_bits() == style.font_size.to_bits()
            && self.weight == style.font_weight
            && self.style == style.font_style
            && self.stretch == style.font_stretch
            && self.stack == style.font_stack
    }
}

/// The `ex` length and `ch` advance ratio of one font selection, measured
/// against loaded fonts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasuredFontUnits {
    font: FontSelection,
    ex: Option<f32>,
    ch_ratio: Option<f32>,
}

/// A style's `ex` length and `ch` advance ratio, each `None` when no loaded
/// face provides it and the CSS fallback applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FontUnitMetrics {
    pub(crate) ex: Option<f32>,
    pub(crate) ch_ratio: Option<f32>,
}

impl MeasuredFontUnits {
    /// Measure the font `style` selects. Without loaded fonts nothing is
    /// measured: every length takes its CSS fallback, which costs nothing.
    pub(crate) fn measure(style: &ComputedStyle, font_metrics: FontMetrics<'_>) -> Option<Self> {
        font_metrics.has_fonts().then(|| Self {
            font: FontSelection::of(style),
            ex: font_metrics.style_x_height(style),
            ch_ratio: font_metrics.style_ch_ratio(style),
        })
    }

    /// The metrics of the font `style` selects: these measurements when they
    /// belong to that font, otherwise measured now.
    pub(crate) fn metrics_for(
        measured: Option<&Self>,
        style: &ComputedStyle,
        font_metrics: FontMetrics<'_>,
    ) -> FontUnitMetrics {
        match measured {
            Some(units) if font_metrics.has_fonts() && units.font.is_selected_by(style) => {
                FontUnitMetrics {
                    ex: units.ex,
                    ch_ratio: units.ch_ratio,
                }
            }
            _ => FontUnitMetrics {
                ex: font_metrics.style_x_height(style),
                ch_ratio: font_metrics.style_ch_ratio(style),
            },
        }
    }

    /// Whether these measurements belong to the font `style` selects.
    pub(crate) fn is_for(&self, style: &ComputedStyle) -> bool {
        self.font.is_selected_by(style)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::style::computed::FontFamily;

    fn parity_serif() -> HashMap<String, crate::parser::ttf::TtfFont> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySerif.ttf"),
        )
        .expect("ParitySerif test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySerif TTF");
        HashMap::from([("parityserif".to_string(), font)])
    }

    fn direct(style: &ComputedStyle, font_metrics: FontMetrics<'_>) -> FontUnitMetrics {
        FontUnitMetrics {
            ex: font_metrics.style_x_height(style),
            ch_ratio: font_metrics.style_ch_ratio(style),
        }
    }

    #[test]
    fn a_child_reuses_its_parents_measurements_only_for_the_same_font() {
        use crate::parser::dom::HtmlTag;
        use crate::style::computed::compute_style_with_context_with_font_metrics;

        let fonts = parity_serif();
        let font_metrics = FontMetrics::new(&fonts);
        let mut parent = ComputedStyle {
            font_stack: FontStack::from_family(FontFamily::Custom("ParitySerif".to_string())),
            font_size: 15.0,
            ..Default::default()
        };
        parent.measured_font_units = MeasuredFontUnits::measure(&parent, font_metrics);
        let child = |declarations: &str| {
            compute_style_with_context_with_font_metrics(
                HtmlTag::Div,
                Some(declarations),
                &parent,
                &[],
                "div",
                &[],
                None,
                &Default::default(),
                &Default::default(),
                font_metrics,
            )
        };

        let same_font = child("height: 3ex");
        assert_eq!(same_font.measured_font_units, parent.measured_font_units);
        let ex = direct(&same_font, font_metrics).ex.expect("x-height");
        assert_eq!(same_font.height, Some(3.0 * ex));

        let larger = child("font-size: 30pt; height: 3ex");
        assert_ne!(larger.measured_font_units, parent.measured_font_units);
        assert!(
            larger
                .measured_font_units
                .as_ref()
                .is_some_and(|units| units.is_for(&larger))
        );
        let ex = direct(&larger, font_metrics).ex.expect("x-height");
        assert_eq!(larger.height, Some(3.0 * ex));
    }

    #[test]
    fn reused_measurements_equal_measuring_the_selected_font_again() {
        let fonts = parity_serif();
        let font_metrics = FontMetrics::new(&fonts);
        let mut style = ComputedStyle {
            font_stack: FontStack::from_family(FontFamily::Custom("ParitySerif".to_string())),
            font_size: 12.0,
            ..Default::default()
        };
        let measured = MeasuredFontUnits::measure(&style, font_metrics).expect("fonts are loaded");
        assert!(measured.is_for(&style));
        assert_eq!(
            MeasuredFontUnits::metrics_for(Some(&measured), &style, font_metrics),
            direct(&style, font_metrics)
        );

        // Another size is another font selection: its x-height is measured,
        // not taken from the 12pt measurement.
        style.font_size = 30.0;
        assert!(!measured.is_for(&style));
        let at_30 = MeasuredFontUnits::metrics_for(Some(&measured), &style, font_metrics);
        assert_eq!(at_30, direct(&style, font_metrics));
        assert_ne!(at_30.ex, measured.ex);
    }

    #[test]
    fn font_relative_lengths_resolve_against_the_measured_font() {
        // `ex` is the font's used x-height and `ch` the advance of its `0`
        // (CSS Values 4 § 6.1.1), whether a declaration uses them directly,
        // inside math, or alongside other units.
        use crate::parser::dom::HtmlTag;
        use crate::style::computed::compute_style_with_context_with_font_metrics;

        let fonts = parity_serif();
        let font_metrics = FontMetrics::new(&fonts);
        let parent = ComputedStyle {
            font_stack: FontStack::from_family(FontFamily::Custom("ParitySerif".to_string())),
            font_size: 15.0,
            ..Default::default()
        };
        let style = compute_style_with_context_with_font_metrics(
            HtmlTag::Div,
            Some(
                "width: 4ex; padding-left: 2ch; padding-top: 10px; \
                 padding-right: calc(1ex + 2pt); letter-spacing: 0.5ex; \
                 height: 3ex; min-height: 2ch; max-height: calc(10ex - 1ch)",
            ),
            &parent,
            &[],
            "div",
            &[],
            None,
            &Default::default(),
            &Default::default(),
            font_metrics,
        );
        let direct = direct(&style, font_metrics);
        let ex = direct.ex.expect("ParitySerif has an x-height");
        let ch = style.font_size * direct.ch_ratio.expect("ParitySerif has a 0 glyph");

        assert_eq!(style.width, Some(4.0 * ex));
        assert_eq!(style.padding.left, 2.0 * ch);
        assert_eq!(style.padding.right, ex + 2.0);
        assert_eq!(style.letter_spacing, 0.5 * ex);
        // These resolve through the declaration set's length context.
        assert_eq!(style.height, Some(3.0 * ex));
        assert_eq!(style.min_height, Some(2.0 * ch));
        assert_eq!(style.max_height, Some(10.0 * ex - ch));
        assert!(
            ex != style.font_size * 0.5,
            "the fallback would hide a wrong context"
        );
    }

    #[test]
    fn only_values_that_cannot_hold_ex_or_ch_skip_font_measurement() {
        use crate::parser::css::CssValue;

        for value in [
            CssValue::Ex(1.0),
            CssValue::Ch(1.0),
            CssValue::Var("--gap".to_string(), None),
            // Raw text parsed later, such as a shorthand's list of lengths.
            CssValue::Keyword("1ex 2EX".to_string()),
            CssValue::Keyword("0 0 1ch black".to_string()),
        ] {
            assert!(value.may_use_font_metric_units(), "{value:?}");
        }
        for value in [
            CssValue::Length(4.0),
            CssValue::Em(1.5),
            CssValue::Percentage(50.0),
            CssValue::Rem(2.0),
            CssValue::Keyword("solid".to_string()),
        ] {
            assert!(!value.may_use_font_metric_units(), "{value:?}");
        }
    }

    #[test]
    fn without_loaded_fonts_nothing_is_measured_or_reused() {
        let fonts = parity_serif();
        let style = ComputedStyle {
            font_stack: FontStack::from_family(FontFamily::Custom("ParitySerif".to_string())),
            ..Default::default()
        };
        let measured =
            MeasuredFontUnits::measure(&style, FontMetrics::new(&fonts)).expect("fonts are loaded");
        assert_eq!(
            MeasuredFontUnits::measure(&style, FontMetrics::default()),
            None
        );
        assert_eq!(
            MeasuredFontUnits::metrics_for(Some(&measured), &style, FontMetrics::default()),
            FontUnitMetrics {
                ex: None,
                ch_ratio: None
            }
        );
    }
}
