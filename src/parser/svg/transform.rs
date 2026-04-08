use super::SvgTransform;

pub(crate) fn compose_transform(
    outer: Option<SvgTransform>,
    inner: Option<SvgTransform>,
) -> Option<SvgTransform> {
    match (outer, inner) {
        (
            Some(SvgTransform::Matrix(a1, b1, c1, d1, e1, f1)),
            Some(SvgTransform::Matrix(a2, b2, c2, d2, e2, f2)),
        ) => Some(SvgTransform::Matrix(
            a1 * a2 + c1 * b2,
            b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2,
            b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1,
            b1 * e2 + d1 * f2 + f1,
        )),
        (Some(transform), None) | (None, Some(transform)) => Some(transform),
        (None, None) => None,
    }
}

/// Parse the transform attribute and convert to a Matrix.
/// Supports: translate, scale, rotate, matrix.
pub fn parse_transform(val: &str) -> Option<SvgTransform> {
    let val = val.trim();

    if let Some(inner) = extract_func_args(val, "matrix") {
        let numbers = parse_num_list(inner);
        if let [a, b, c, d, e, f] = numbers.as_slice() {
            return Some(SvgTransform::Matrix(*a, *b, *c, *d, *e, *f));
        }
    }

    if let Some(inner) = extract_func_args(val, "translate") {
        let numbers = parse_num_list(inner);
        let tx = numbers.first().copied().unwrap_or(0.0);
        let ty = numbers.get(1).copied().unwrap_or(0.0);
        return Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, tx, ty));
    }

    if let Some(inner) = extract_func_args(val, "scale") {
        let numbers = parse_num_list(inner);
        let sx = numbers.first().copied().unwrap_or(1.0);
        let sy = numbers.get(1).copied().unwrap_or(sx);
        return Some(SvgTransform::Matrix(sx, 0.0, 0.0, sy, 0.0, 0.0));
    }

    if let Some(inner) = extract_func_args(val, "rotate") {
        let numbers = parse_num_list(inner);
        let angle_deg = numbers.first().copied().unwrap_or(0.0);
        let angle = angle_deg.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        if let [_, cx, cy, ..] = numbers.as_slice() {
            let tx = cx - cos_a * cx + sin_a * cy;
            let ty = cy - sin_a * cx - cos_a * cy;
            return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, tx, ty));
        }

        return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, 0.0, 0.0));
    }

    None
}

fn extract_func_args<'a>(val: &'a str, func_name: &str) -> Option<&'a str> {
    let trimmed = val.trim();
    let (name, args_with_close) = trimmed.split_once('(')?;
    if !name.trim().eq_ignore_ascii_case(func_name) {
        return None;
    }
    args_with_close.strip_suffix(')').map(str::trim)
}

fn parse_num_list(s: &str) -> Vec<f32> {
    s.split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{compose_transform, extract_func_args, parse_num_list, parse_transform};
    use crate::parser::svg::SvgTransform;

    #[test]
    fn parse_transform_translate() {
        let transform = parse_transform("translate(10, 20)").unwrap();
        match transform {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert_eq!((a, b, c, d), (1.0, 0.0, 0.0, 1.0));
                assert_eq!((e, f), (10.0, 20.0));
            }
        }
    }

    #[test]
    fn parse_transform_scale() {
        let transform = parse_transform("scale(2)").unwrap();
        match transform {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert_eq!((a, b, c, d, e, f), (2.0, 0.0, 0.0, 2.0, 0.0, 0.0));
            }
        }
    }

    #[test]
    fn parse_transform_rotate() {
        let transform = parse_transform("rotate(45)").unwrap();
        match transform {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                let cos45 = 45.0_f32.to_radians().cos();
                let sin45 = 45.0_f32.to_radians().sin();
                assert!((a - cos45).abs() < 0.001);
                assert!((b - sin45).abs() < 0.001);
                assert!((c + sin45).abs() < 0.001);
                assert!((d - cos45).abs() < 0.001);
                assert_eq!((e, f), (0.0, 0.0));
            }
        }
    }

    #[test]
    fn parse_transform_matrix() {
        let transform = parse_transform("matrix(1,0,0,1,10,20)").unwrap();
        match transform {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert_eq!((a, b, c, d, e, f), (1.0, 0.0, 0.0, 1.0, 10.0, 20.0));
            }
        }
    }

    #[test]
    fn parse_transform_rotate_with_center() {
        let transform = parse_transform("rotate(90, 50, 50)").unwrap();
        match transform {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                let cos90 = 90.0_f32.to_radians().cos();
                let sin90 = 90.0_f32.to_radians().sin();
                assert!((a - cos90).abs() < 0.01);
                assert!((b - sin90).abs() < 0.01);
                assert!((c + sin90).abs() < 0.01);
                assert!((d - cos90).abs() < 0.01);
                let tx = 50.0 - cos90 * 50.0 + sin90 * 50.0;
                let ty = 50.0 - sin90 * 50.0 - cos90 * 50.0;
                assert!((e - tx).abs() < 0.01);
                assert!((f - ty).abs() < 0.01);
            }
        }
    }

    #[test]
    fn parse_transform_scale_xy() {
        let transform = parse_transform("scale(2, 3)").unwrap();
        match transform {
            SvgTransform::Matrix(a, _b, _c, d, _e, _f) => {
                assert_eq!((a, d), (2.0, 3.0));
            }
        }
    }

    #[test]
    fn parse_transform_translate_single_value() {
        let transform = parse_transform("translate(10)").unwrap();
        match transform {
            SvgTransform::Matrix(_a, _b, _c, _d, e, f) => {
                assert_eq!((e, f), (10.0, 0.0));
            }
        }
    }

    #[test]
    fn parse_transform_unknown() {
        assert!(parse_transform("skewX(30)").is_none());
    }

    #[test]
    fn parse_transform_empty() {
        assert!(parse_transform("").is_none());
    }

    #[test]
    fn compose_transform_multiplies_matrices() {
        let outer = Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 3.0, 4.0));
        let inner = Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 5.0, 6.0));
        assert!(matches!(
            compose_transform(outer, inner),
            Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 8.0, 10.0))
        ));
    }

    #[test]
    fn extract_func_args_basic() {
        assert_eq!(
            extract_func_args("translate(10, 20)", "translate"),
            Some("10, 20")
        );
    }

    #[test]
    fn extract_func_args_not_found() {
        assert_eq!(extract_func_args("translate(10, 20)", "rotate"), None);
    }

    #[test]
    fn extract_func_args_no_parens() {
        assert_eq!(extract_func_args("translate", "translate"), None);
    }

    #[test]
    fn parse_num_list_basic() {
        assert_eq!(parse_num_list("1, 2.5, 3"), vec![1.0, 2.5, 3.0]);
    }

    #[test]
    fn parse_num_list_empty() {
        assert!(parse_num_list("").is_empty());
    }

    #[test]
    fn parse_num_list_with_invalid() {
        assert_eq!(parse_num_list("1, abc, 3"), vec![1.0, 3.0]);
    }
}
