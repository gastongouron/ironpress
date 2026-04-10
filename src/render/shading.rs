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
