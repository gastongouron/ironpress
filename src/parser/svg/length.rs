use super::ViewBox;

/// Parse a length value (strip px/em/etc suffix, parse number).
pub(crate) fn parse_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    let number = trimmed.trim_end_matches(|ch: char| ch.is_ascii_alphabetic() || ch == '%');
    number.trim().parse::<f32>().ok()
}

pub(crate) fn parse_absolute_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    if trimmed.ends_with('%') {
        return None;
    }
    parse_length(trimmed)
}

/// Parse a viewBox attribute: "min-x min-y width height".
pub(crate) fn parse_viewbox(val: &str) -> Option<ViewBox> {
    let parts: Vec<f32> = val
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();

    match parts.as_slice() {
        [min_x, min_y, width, height] => Some(ViewBox {
            min_x: *min_x,
            min_y: *min_y,
            width: *width,
            height: *height,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_absolute_length, parse_length, parse_viewbox};

    #[test]
    fn parse_length_plain_number() {
        assert_eq!(parse_length("42"), Some(42.0));
    }

    #[test]
    fn parse_length_with_px_suffix() {
        assert_eq!(parse_length("100px"), Some(100.0));
    }

    #[test]
    fn parse_length_with_em_suffix() {
        assert_eq!(parse_length("1.5em"), Some(1.5));
    }

    #[test]
    fn parse_length_with_percent() {
        assert_eq!(parse_length("50%"), Some(50.0));
    }

    #[test]
    fn parse_length_with_whitespace() {
        assert_eq!(parse_length("  200  "), Some(200.0));
    }

    #[test]
    fn parse_length_invalid() {
        assert_eq!(parse_length("abc"), None);
    }

    #[test]
    fn parse_length_empty() {
        assert_eq!(parse_length(""), None);
    }

    #[test]
    fn parse_absolute_length_rejects_percent() {
        assert_eq!(parse_absolute_length("50%"), None);
    }

    #[test]
    fn parse_viewbox_comma_separated() {
        let view_box = parse_viewbox("0,0,100,200").unwrap();
        assert_eq!(
            (
                view_box.min_x,
                view_box.min_y,
                view_box.width,
                view_box.height
            ),
            (0.0, 0.0, 100.0, 200.0)
        );
    }

    #[test]
    fn parse_viewbox_space_separated() {
        let view_box = parse_viewbox("10 20 300 400").unwrap();
        assert_eq!(
            (
                view_box.min_x,
                view_box.min_y,
                view_box.width,
                view_box.height
            ),
            (10.0, 20.0, 300.0, 400.0)
        );
    }

    #[test]
    fn parse_viewbox_mixed_separators() {
        let view_box = parse_viewbox("5, 10  200, 300").unwrap();
        assert_eq!(
            (
                view_box.min_x,
                view_box.min_y,
                view_box.width,
                view_box.height
            ),
            (5.0, 10.0, 200.0, 300.0)
        );
    }

    #[test]
    fn parse_viewbox_too_few_values() {
        assert!(parse_viewbox("0 0 100").is_none());
    }

    #[test]
    fn parse_viewbox_too_many_values() {
        assert!(parse_viewbox("0 0 100 200 300").is_none());
    }

    #[test]
    fn parse_viewbox_invalid_number() {
        assert!(parse_viewbox("0 abc 100 200").is_none());
    }
}
