//! Shared PDF shading helpers for CSS and SVG gradient rendering.

/// A PDF shading dictionary entry.
#[derive(Debug, Clone)]
pub(crate) struct ShadingEntry {
    pub name: String,
    pub shading_type: u8, // 2 = axial (linear), 3 = radial
    pub coords: [f32; 6],
    pub stops: Vec<(f32, (f32, f32, f32))>,
}

/// Reserve a shading name and store an axial shading entry for the current page.
pub(crate) fn push_axial_shading(
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    coords: [f32; 4],
    stops: Vec<(f32, (f32, f32, f32))>,
) -> String {
    let name = format!("SH{}", *shading_counter);
    *shading_counter += 1;
    shadings.push(ShadingEntry {
        name: name.clone(),
        shading_type: 2,
        coords: [coords[0], coords[1], coords[2], coords[3], 0.0, 0.0],
        stops,
    });
    name
}

/// Reserve a shading name and store a radial shading entry for the current page.
pub(crate) fn push_radial_shading(
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    coords: [f32; 6],
    stops: Vec<(f32, (f32, f32, f32))>,
) -> String {
    let name = format!("SH{}", *shading_counter);
    *shading_counter += 1;
    shadings.push(ShadingEntry {
        name: name.clone(),
        shading_type: 3,
        coords,
        stops,
    });
    name
}

/// Build an inline PDF Function dictionary string for a gradient's color stops.
pub(crate) fn build_shading_function(stops: &[(f32, (f32, f32, f32))]) -> String {
    if stops.len() < 2 {
        let (r, g, b) = stops.first().map(|s| s.1).unwrap_or((0.0, 0.0, 0.0));
        return format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r} {g} {b}] /C1 [{r} {g} {b}] /N 1 >>"
        );
    }

    if stops.len() == 2 {
        let (r0, g0, b0) = stops[0].1;
        let (r1, g1, b1) = stops[1].1;
        return format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r0} {g0} {b0}] /C1 [{r1} {g1} {b1}] /N 1 >>"
        );
    }

    let mut functions = Vec::new();
    let mut bounds = Vec::new();
    let mut encode = Vec::new();

    for i in 0..stops.len() - 1 {
        let (r0, g0, b0) = stops[i].1;
        let (r1, g1, b1) = stops[i + 1].1;
        functions.push(format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{r0} {g0} {b0}] /C1 [{r1} {g1} {b1}] /N 1 >>"
        ));
        if i < stops.len() - 2 {
            bounds.push(format!("{}", stops[i + 1].0));
        }
        encode.push("0 1".to_string());
    }

    let functions_str = functions.join(" ");
    let bounds_str = bounds.join(" ");
    let encode_str = encode.join(" ");

    format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{functions_str}] /Bounds [{bounds_str}] /Encode [{encode_str}] >>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // build_shading_function
    // -------------------------------------------------------------------------

    #[test]
    fn shading_function_zero_stops_uses_black() {
        let result = build_shading_function(&[]);
        assert!(
            result.contains("/FunctionType 2"),
            "should be FunctionType 2"
        );
        assert!(result.contains("/C0 [0 0 0]"), "C0 should be black");
        assert!(result.contains("/C1 [0 0 0]"), "C1 should be black");
    }

    #[test]
    fn shading_function_one_stop_repeats_color() {
        let stops = [(0.0_f32, (1.0_f32, 0.5_f32, 0.0_f32))];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        // Both C0 and C1 must carry the same colour.
        assert!(result.contains("/C0 [1 0.5 0]"));
        assert!(result.contains("/C1 [1 0.5 0]"));
    }

    #[test]
    fn shading_function_two_stops_produces_type2() {
        let stops = [
            (0.0_f32, (0.0_f32, 0.0_f32, 0.0_f32)),
            (1.0_f32, (1.0_f32, 1.0_f32, 1.0_f32)),
        ];
        let result = build_shading_function(&stops);
        assert!(
            result.contains("/FunctionType 2"),
            "two stops → FunctionType 2"
        );
        assert!(result.contains("/C0 [0 0 0]"));
        assert!(result.contains("/C1 [1 1 1]"));
        // Must NOT be a stitching function.
        assert!(!result.contains("/FunctionType 3"));
    }

    #[test]
    fn shading_function_three_stops_produces_type3_stitching() {
        let stops = [
            (0.0_f32, (0.0_f32, 0.0_f32, 0.0_f32)),
            (0.5_f32, (0.5_f32, 0.5_f32, 0.5_f32)),
            (1.0_f32, (1.0_f32, 1.0_f32, 1.0_f32)),
        ];
        let result = build_shading_function(&stops);
        assert!(
            result.contains("/FunctionType 3"),
            "three stops → FunctionType 3 (stitching)"
        );
        // The inner functions are FunctionType 2.
        assert!(result.contains("/FunctionType 2"));
        // Bounds should list the middle stop position.
        assert!(
            result.contains("/Bounds [0.5]"),
            "bounds should contain middle stop offset"
        );
        // Two sub-functions → two "0 1" encode entries.
        assert!(result.contains("/Encode [0 1 0 1]"));
    }

    #[test]
    fn shading_function_four_stops_has_three_bounds() {
        let stops = [
            (0.0_f32, (0.0_f32, 0.0_f32, 0.0_f32)),
            (0.25_f32, (0.25_f32, 0.0_f32, 0.0_f32)),
            (0.75_f32, (0.75_f32, 0.0_f32, 0.0_f32)),
            (1.0_f32, (1.0_f32, 0.0_f32, 0.0_f32)),
        ];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 3"));
        // Three sub-functions → three "0 1" entries.
        assert!(result.contains("/Encode [0 1 0 1 0 1]"));
        // Two interior boundaries.
        assert!(result.contains("/Bounds [0.25 0.75]"));
    }

    // -------------------------------------------------------------------------
    // push_axial_shading
    // -------------------------------------------------------------------------

    #[test]
    fn push_axial_shading_returns_correct_name_and_increments_counter() {
        let mut shadings: Vec<ShadingEntry> = Vec::new();
        let mut counter = 0usize;
        let stops = vec![(0.0_f32, (0.0_f32, 0.0_f32, 0.0_f32))];

        let name = push_axial_shading(
            &mut shadings,
            &mut counter,
            [0.0, 0.0, 100.0, 100.0],
            stops.clone(),
        );
        assert_eq!(name, "SH0");
        assert_eq!(counter, 1);
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].shading_type, 2);
        assert_eq!(shadings[0].name, "SH0");

        let name2 =
            push_axial_shading(&mut shadings, &mut counter, [10.0, 20.0, 30.0, 40.0], stops);
        assert_eq!(name2, "SH1");
        assert_eq!(counter, 2);
        assert_eq!(shadings.len(), 2);
    }

    #[test]
    fn push_axial_shading_stores_correct_coords() {
        let mut shadings: Vec<ShadingEntry> = Vec::new();
        let mut counter = 0usize;
        push_axial_shading(&mut shadings, &mut counter, [1.0, 2.0, 3.0, 4.0], vec![]);

        let entry = &shadings[0];
        // Axial uses only the first four coords; last two are padded with 0.
        assert_eq!(entry.coords[0], 1.0);
        assert_eq!(entry.coords[1], 2.0);
        assert_eq!(entry.coords[2], 3.0);
        assert_eq!(entry.coords[3], 4.0);
        assert_eq!(entry.coords[4], 0.0);
        assert_eq!(entry.coords[5], 0.0);
    }

    // -------------------------------------------------------------------------
    // push_radial_shading
    // -------------------------------------------------------------------------

    #[test]
    fn push_radial_shading_returns_correct_name_and_increments_counter() {
        let mut shadings: Vec<ShadingEntry> = Vec::new();
        let mut counter = 5usize; // start at non-zero to verify offset
        let coords = [10.0_f32, 20.0, 5.0, 30.0, 40.0, 15.0];
        let stops = vec![(0.0_f32, (1.0_f32, 0.0_f32, 0.0_f32))];

        let name = push_radial_shading(&mut shadings, &mut counter, coords, stops);
        assert_eq!(name, "SH5");
        assert_eq!(counter, 6);
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].shading_type, 3);
    }

    #[test]
    fn push_radial_shading_stores_all_six_coords() {
        let mut shadings: Vec<ShadingEntry> = Vec::new();
        let mut counter = 0usize;
        let coords = [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        push_radial_shading(&mut shadings, &mut counter, coords, vec![]);

        assert_eq!(shadings[0].coords, coords);
    }
}
