use crate::parser::css::{CssValue, StyleMap};
use crate::parser::dom::HtmlTag;
use crate::types::Color;

/// Returns the default (user-agent) styles for a given HTML tag.
pub fn default_style(tag: HtmlTag) -> StyleMap {
    let mut style = StyleMap::new();

    match tag {
        HtmlTag::H1 => {
            style.set("font-size", CssValue::Length(24.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(8.0));
            style.set("margin-bottom", CssValue::Length(6.0));
        }
        HtmlTag::H2 => {
            style.set("font-size", CssValue::Length(20.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(6.0));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::H3 => {
            style.set("font-size", CssValue::Length(16.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::H4 => {
            style.set("font-size", CssValue::Length(14.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        HtmlTag::H5 => {
            style.set("font-size", CssValue::Length(12.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(2.0));
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        HtmlTag::H6 => {
            style.set("font-size", CssValue::Length(10.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(2.0));
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        HtmlTag::P => {
            // No default font-size — <p> inherits from parent (matches browser UA behavior)
            style.set("margin-top", CssValue::Length(0.0));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::Strong | HtmlTag::B => {
            style.set("font-weight", CssValue::Keyword("bold".into()));
        }
        HtmlTag::Em | HtmlTag::I => {
            style.set("font-style", CssValue::Keyword("italic".into()));
        }
        HtmlTag::U | HtmlTag::Ins => {
            style.set("text-decoration", CssValue::Keyword("underline".into()));
        }
        HtmlTag::Del | HtmlTag::S => {
            style.set("text-decoration", CssValue::Keyword("line-through".into()));
        }
        HtmlTag::A => {
            style.set("color", CssValue::Color(Color::rgb(0, 0, 238)));
            style.set("text-decoration", CssValue::Keyword("underline".into()));
        }
        HtmlTag::Hr => {
            style.set("margin-top", CssValue::Length(6.0));
            style.set("margin-bottom", CssValue::Length(6.0));
        }
        HtmlTag::Li => {
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        HtmlTag::Ul | HtmlTag::Ol => {
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(8.0));
            style.set("margin-left", CssValue::Length(20.0));
        }
        HtmlTag::Dl => {
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(8.0));
        }
        HtmlTag::Dt => {
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-top", CssValue::Length(4.0));
        }
        HtmlTag::Dd => {
            style.set("margin-left", CssValue::Length(30.0));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::Td => {
            style.set("padding-top", CssValue::Length(4.0));
            style.set("padding-right", CssValue::Length(6.0));
            style.set("padding-bottom", CssValue::Length(4.0));
            style.set("padding-left", CssValue::Length(6.0));
        }
        HtmlTag::Th => {
            style.set("padding-top", CssValue::Length(4.0));
            style.set("padding-right", CssValue::Length(6.0));
            style.set("padding-bottom", CssValue::Length(4.0));
            style.set("padding-left", CssValue::Length(6.0));
            style.set("font-weight", CssValue::Keyword("bold".into()));
        }
        HtmlTag::Blockquote => {
            style.set("margin-top", CssValue::Length(8.0));
            style.set("margin-bottom", CssValue::Length(8.0));
            style.set("margin-left", CssValue::Length(30.0));
            style.set("font-style", CssValue::Keyword("italic".into()));
        }
        HtmlTag::Pre => {
            style.set("font-size", CssValue::Length(10.0));
            style.set("margin-top", CssValue::Length(8.0));
            style.set("margin-bottom", CssValue::Length(8.0));
            style.set("padding-top", CssValue::Length(8.0));
            style.set("padding-bottom", CssValue::Length(8.0));
            style.set("padding-left", CssValue::Length(8.0));
            style.set("padding-right", CssValue::Length(8.0));
            style.set(
                "background-color",
                CssValue::Color(Color::rgb(245, 245, 245)),
            );
        }
        HtmlTag::Code => {
            style.set("font-size", CssValue::Length(10.0));
            style.set(
                "background-color",
                CssValue::Color(Color::rgb(245, 245, 245)),
            );
        }
        HtmlTag::Small => {
            style.set("font-size", CssValue::Length(10.0));
        }
        HtmlTag::Mark => {
            style.set("background-color", CssValue::Color(Color::rgb(255, 255, 0)));
        }
        HtmlTag::Address => {
            style.set("font-style", CssValue::Keyword("italic".into()));
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::Figure => {
            style.set("margin-top", CssValue::Length(8.0));
            style.set("margin-bottom", CssValue::Length(8.0));
            style.set("margin-left", CssValue::Length(30.0));
        }
        HtmlTag::Figcaption => {
            style.set("font-size", CssValue::Length(10.0));
            style.set("font-style", CssValue::Keyword("italic".into()));
        }
        HtmlTag::Caption => {
            style.set("font-weight", CssValue::Keyword("bold".into()));
            style.set("margin-bottom", CssValue::Length(4.0));
        }
        HtmlTag::Summary => {
            style.set("font-weight", CssValue::Keyword("bold".into()));
        }
        HtmlTag::Input => {
            style.set("padding-top", CssValue::Length(2.0));
            style.set("padding-right", CssValue::Length(4.0));
            style.set("padding-bottom", CssValue::Length(2.0));
            style.set("padding-left", CssValue::Length(4.0));
            style.set("border-width", CssValue::Length(1.0));
            style.set("border-color", CssValue::Color(Color::rgb(169, 169, 169)));
        }
        HtmlTag::Select => {
            style.set("padding-top", CssValue::Length(2.0));
            style.set("padding-right", CssValue::Length(4.0));
            style.set("padding-bottom", CssValue::Length(2.0));
            style.set("padding-left", CssValue::Length(4.0));
            style.set("border-width", CssValue::Length(1.0));
            style.set("border-color", CssValue::Color(Color::rgb(169, 169, 169)));
        }
        HtmlTag::Textarea => {
            style.set("padding-top", CssValue::Length(4.0));
            style.set("padding-right", CssValue::Length(4.0));
            style.set("padding-bottom", CssValue::Length(4.0));
            style.set("padding-left", CssValue::Length(4.0));
            style.set("border-width", CssValue::Length(1.0));
            style.set("border-color", CssValue::Color(Color::rgb(169, 169, 169)));
            style.set("font-size", CssValue::Length(10.0));
        }
        HtmlTag::Video => {
            style.set("margin-top", CssValue::Length(4.0));
            style.set("margin-bottom", CssValue::Length(4.0));
            style.set("background-color", CssValue::Color(Color::rgb(0, 0, 0)));
        }
        HtmlTag::Audio => {
            style.set("margin-top", CssValue::Length(2.0));
            style.set("margin-bottom", CssValue::Length(2.0));
            style.set(
                "background-color",
                CssValue::Color(Color::rgb(240, 240, 240)),
            );
        }
        HtmlTag::Progress => {
            style.set("margin-top", CssValue::Length(2.0));
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        HtmlTag::Meter => {
            style.set("margin-top", CssValue::Length(2.0));
            style.set("margin-bottom", CssValue::Length(2.0));
        }
        _ => {}
    }

    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h1_defaults() {
        let s = default_style(HtmlTag::H1);
        assert!(s.get("font-size").is_some());
        assert!(s.get("font-weight").is_some());
    }

    #[test]
    fn all_heading_defaults() {
        for tag in [
            HtmlTag::H1,
            HtmlTag::H2,
            HtmlTag::H3,
            HtmlTag::H4,
            HtmlTag::H5,
            HtmlTag::H6,
        ] {
            let s = default_style(tag);
            assert!(
                s.get("font-weight").is_some(),
                "Missing font-weight for {:?}",
                tag
            );
        }
    }

    #[test]
    fn inline_defaults() {
        assert!(default_style(HtmlTag::Strong).get("font-weight").is_some());
        assert!(default_style(HtmlTag::Em).get("font-style").is_some());
        assert!(default_style(HtmlTag::U).get("text-decoration").is_some());
        assert!(default_style(HtmlTag::A).get("color").is_some());
        assert!(default_style(HtmlTag::Del).get("text-decoration").is_some());
        assert!(default_style(HtmlTag::S).get("text-decoration").is_some());
        assert!(default_style(HtmlTag::Ins).get("text-decoration").is_some());
        assert!(
            default_style(HtmlTag::Mark)
                .get("background-color")
                .is_some()
        );
        assert!(default_style(HtmlTag::Small).get("font-size").is_some());
        assert!(default_style(HtmlTag::Code).get("font-size").is_some());
    }

    #[test]
    fn list_defaults() {
        assert!(default_style(HtmlTag::Ul).get("margin-left").is_some());
        assert!(default_style(HtmlTag::Ol).get("margin-left").is_some());
        assert!(default_style(HtmlTag::Li).get("margin-bottom").is_some());
        assert!(default_style(HtmlTag::Dl).get("margin-top").is_some());
        assert!(default_style(HtmlTag::Dt).get("font-weight").is_some());
        assert!(default_style(HtmlTag::Dd).get("margin-left").is_some());
    }

    #[test]
    fn table_defaults() {
        assert!(default_style(HtmlTag::Td).get("padding-top").is_some());
        assert!(default_style(HtmlTag::Th).get("font-weight").is_some());
        assert!(default_style(HtmlTag::Caption).get("font-weight").is_some());
    }

    #[test]
    fn block_element_defaults() {
        assert!(
            default_style(HtmlTag::Blockquote)
                .get("margin-left")
                .is_some()
        );
        assert!(
            default_style(HtmlTag::Pre)
                .get("background-color")
                .is_some()
        );
        assert!(default_style(HtmlTag::Address).get("font-style").is_some());
        assert!(default_style(HtmlTag::Figure).get("margin-left").is_some());
        assert!(
            default_style(HtmlTag::Figcaption)
                .get("font-style")
                .is_some()
        );
        assert!(default_style(HtmlTag::Summary).get("font-weight").is_some());
    }

    #[test]
    fn form_element_defaults() {
        let input = default_style(HtmlTag::Input);
        assert!(input.get("padding-top").is_some());
        assert!(input.get("border-width").is_some());
        assert!(input.get("border-color").is_some());

        let select = default_style(HtmlTag::Select);
        assert!(select.get("padding-top").is_some());
        assert!(select.get("border-width").is_some());

        let textarea = default_style(HtmlTag::Textarea);
        assert!(textarea.get("padding-top").is_some());
        assert!(textarea.get("font-size").is_some());
        assert!(textarea.get("border-width").is_some());
    }

    #[test]
    fn media_element_defaults() {
        let video = default_style(HtmlTag::Video);
        assert!(video.get("background-color").is_some());
        assert!(video.get("margin-top").is_some());

        let audio = default_style(HtmlTag::Audio);
        assert!(audio.get("background-color").is_some());
        assert!(audio.get("margin-top").is_some());
    }

    #[test]
    fn progress_meter_defaults() {
        let progress = default_style(HtmlTag::Progress);
        assert!(progress.get("margin-top").is_some());

        let meter = default_style(HtmlTag::Meter);
        assert!(meter.get("margin-top").is_some());
    }

    #[test]
    fn unknown_tag_has_no_defaults() {
        let s = default_style(HtmlTag::Unknown);
        assert!(s.properties.is_empty());
    }

    #[test]
    fn p_has_no_default_font_size() {
        let s = default_style(HtmlTag::P);
        assert!(
            s.get("font-size").is_none(),
            "P tag should not have a default font-size (it inherits from parent)"
        );
    }
}
