//! SVG tree to PDF content stream renderer.

use crate::parser::svg::{
    PathCommand, SvgClipPathUnits, SvgGradientUnits, SvgLinearGradient, SvgNode, SvgPaint,
    SvgStyle, SvgTextContext, SvgTransform, SvgTree,
};
use crate::render::pdf::encode_pdf_text;
use crate::render::shading::{ShadingEntry, push_axial_shading};
use crate::render::svg_geometry::{
    SvgPlacementRequest, SvgViewportBox, compute_raster_placement, compute_svg_placement,
};
use crate::style::computed::{FontFamily, parse_font_stack};
use std::fmt::Write as _;

pub(crate) trait SvgImageObjectSink {
    fn register_raster(&mut self, raw_image: &[u8]) -> Option<String>;
}

pub(crate) struct SvgPdfResources<'a> {
    pub shadings: &'a mut Vec<ShadingEntry>,
    pub shading_counter: &'a mut usize,
    pub image_sink: Option<&'a mut dyn SvgImageObjectSink>,
}

impl<'a> SvgPdfResources<'a> {
    fn without_images(shadings: &'a mut Vec<ShadingEntry>, shading_counter: &'a mut usize) -> Self {
        Self {
            shadings,
            shading_counter,
            image_sink: None,
        }
    }

    fn shading_state(&mut self) -> (&mut Vec<ShadingEntry>, &mut usize) {
        (self.shadings, self.shading_counter)
    }

    fn register_raster(&mut self, raw_image: &[u8]) -> Option<String> {
        self.image_sink.as_deref_mut()?.register_raster(raw_image)
    }
}

/// Render an SVG tree to PDF content stream operators.
///
/// The caller must wrap this in a `q ... Q` block and set up the coordinate
/// transform (position on page + y-axis flip).
#[cfg(test)]
pub fn render_svg_tree(tree: &SvgTree, out: &mut String) {
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    let mut resources = SvgPdfResources::without_images(&mut shadings, &mut shading_counter);
    render_svg_tree_with_resources(tree, out, &mut resources);
}

#[cfg(test)]
pub(crate) fn render_svg_tree_with_shadings(
    tree: &SvgTree,
    out: &mut String,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let mut resources = SvgPdfResources::without_images(shadings, shading_counter);
    render_svg_tree_with_resources(tree, out, &mut resources);
}

pub(crate) fn render_svg_tree_with_resources(
    tree: &SvgTree,
    out: &mut String,
    resources: &mut SvgPdfResources<'_>,
) {
    // SVG initial values: fill=black, stroke=none, stroke-width=1.
    let root_style = ResolvedStyle {
        color: tree.text_ctx.color,
        fill: SvgPaint::Color((0.0, 0.0, 0.0)),
        stroke: SvgPaint::None,
        clip_path: None,
        stroke_width: 1.0,
        font_family: None,
        font_bold: None,
        font_italic: None,
    };
    for node in &tree.children {
        render_node(
            node,
            root_style.clone(),
            &tree.text_ctx,
            &tree.defs,
            resources,
            out,
            RenderMode::Paint,
        );
    }
}

#[derive(Debug, Clone)]
struct ResolvedStyle {
    color: Option<(f32, f32, f32)>,
    fill: SvgPaint,
    stroke: SvgPaint,
    clip_path: Option<String>,
    stroke_width: f32,
    font_family: Option<String>,
    font_bold: Option<bool>,
    font_italic: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Paint,
    PathOnly,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SvgObjectBoundingBox {
    min_x: f32,
    min_y: f32,
    width: f32,
    height: f32,
}

impl SvgObjectBoundingBox {
    fn from_extents(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Option<Self> {
        let width = max_x - min_x;
        let height = max_y - min_y;
        (width >= 0.0 && height >= 0.0).then_some(Self {
            min_x,
            min_y,
            width,
            height,
        })
    }

    fn max_x(self) -> f32 {
        self.min_x + self.width
    }

    fn max_y(self) -> f32 {
        self.min_y + self.height
    }

    fn is_empty(self) -> bool {
        self.width == 0.0 || self.height == 0.0
    }

    fn union(self, other: Self) -> Self {
        let min_x = self.min_x.min(other.min_x);
        let min_y = self.min_y.min(other.min_y);
        let max_x = self.max_x().max(other.max_x());
        let max_y = self.max_y().max(other.max_y());
        Self {
            min_x,
            min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    fn transform(self, transform: &SvgTransform) -> Self {
        let corners = [
            transform_point(transform, self.min_x, self.min_y),
            transform_point(transform, self.max_x(), self.min_y),
            transform_point(transform, self.min_x, self.max_y()),
            transform_point(transform, self.max_x(), self.max_y()),
        ];
        bounding_box_from_points(&corners).unwrap_or(self)
    }

    fn to_object_units_transform(self) -> Option<SvgTransform> {
        (!self.is_empty()).then_some(SvgTransform::Matrix(
            self.width,
            0.0,
            0.0,
            self.height,
            self.min_x,
            self.min_y,
        ))
    }

    fn linear_gradient_coords(self, gradient: &SvgLinearGradient) -> [f32; 4] {
        [
            self.min_x + gradient.x1 * self.width,
            self.min_y + gradient.y1 * self.height,
            self.min_x + gradient.x2 * self.width,
            self.min_y + gradient.y2 * self.height,
        ]
    }
}

fn resolve_style(parent: ResolvedStyle, local: &SvgStyle) -> ResolvedStyle {
    let fill = match &local.fill {
        SvgPaint::Unspecified => parent.fill,
        other => other.clone(),
    };
    let color = local.color.or(parent.color);
    let stroke = match &local.stroke {
        SvgPaint::Unspecified => parent.stroke,
        other => other.clone(),
    };
    let stroke_width = local.stroke_width.unwrap_or(parent.stroke_width);
    ResolvedStyle {
        color,
        fill,
        stroke,
        clip_path: local.clip_path.clone(),
        stroke_width,
        font_family: local.font_family.clone().or(parent.font_family),
        font_bold: local.font_bold.or(parent.font_bold),
        font_italic: local.font_italic.or(parent.font_italic),
    }
}

fn paint_to_rgb(paint: &SvgPaint, color: Option<(f32, f32, f32)>) -> Option<(f32, f32, f32)> {
    match paint {
        SvgPaint::None => None,
        SvgPaint::Color(c) => Some(*c),
        SvgPaint::CurrentColor => Some(color.unwrap_or((0.0, 0.0, 0.0))),
        SvgPaint::Url(_) => None,
        SvgPaint::Unspecified => None,
    }
}

fn render_node(
    node: &SvgNode,
    inherited: ResolvedStyle,
    text_ctx: &SvgTextContext,
    defs: &crate::parser::svg::SvgDefs,
    resources: &mut SvgPdfResources<'_>,
    out: &mut String,
    mode: RenderMode,
) {
    match node {
        SvgNode::Group {
            transform,
            children,
            style,
            ..
        } => {
            let style = resolve_style(inherited, style);
            let clip_path = style.clip_path.clone();
            let mut style = style;
            style.clip_path = None;

            let mut paint_children = |out: &mut String, style: &ResolvedStyle| {
                out.push_str("q\n");
                if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = transform {
                    out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
                }
                for child in children {
                    render_node(child, style.clone(), text_ctx, defs, resources, out, mode);
                }
                out.push_str("Q\n");
            };

            if mode == RenderMode::Paint {
                if let Some(clip_id) = clip_path.as_deref() {
                    out.push_str("q\n");
                    if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = transform {
                        out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
                    }
                    let (shadings, shading_counter) = resources.shading_state();
                    render_clip_path(
                        clip_id,
                        defs,
                        shadings,
                        shading_counter,
                        children_bounding_box(children, text_ctx),
                        out,
                    );
                    for child in children {
                        render_node(child, style.clone(), text_ctx, defs, resources, out, mode);
                    }
                    out.push_str("Q\n");
                } else {
                    paint_children(out, &style);
                }
            } else {
                out.push_str("q\n");
                if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = transform {
                    out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
                }
                for child in children {
                    render_node(child, style.clone(), text_ctx, defs, resources, out, mode);
                }
                out.push_str("Q\n");
            }
        }
        SvgNode::Rect { .. }
        | SvgNode::Circle { .. }
        | SvgNode::Ellipse { .. }
        | SvgNode::Polygon { .. }
        | SvgNode::Path { .. } => {
            let style = match node {
                SvgNode::Rect { style, .. }
                | SvgNode::Circle { style, .. }
                | SvgNode::Ellipse { style, .. }
                | SvgNode::Polygon { style, .. }
                | SvgNode::Path { style, .. } => resolve_style(inherited, style),
                _ => unreachable!(),
            };
            let Some(path) = shape_path_string(node) else {
                return;
            };
            if mode == RenderMode::PathOnly {
                out.push_str(&path);
                return;
            }

            match &style.fill {
                SvgPaint::Url(id) => {
                    out.push_str(&path);
                    if let Some(gradient) = defs.gradients.get(id) {
                        if let Some(coords) = resolve_gradient_coords(
                            gradient,
                            node_object_bounding_box(node, text_ctx),
                        ) {
                            let (shadings, shading_counter) = resources.shading_state();
                            paint_svg_linear_gradient_fill(
                                gradient,
                                coords,
                                out,
                                shadings,
                                shading_counter,
                            );
                        } else {
                            out.push_str("n\n");
                        }
                    } else {
                        out.push_str("n\n");
                    }
                    if has_visible_stroke(&style) {
                        out.push_str(&path);
                        apply_stroke_style(&style, out);
                        paint_stroke_only(&style, out);
                    }
                }
                _ => {
                    apply_style(&style, out);
                    out.push_str(&path);
                    paint(&style, out);
                }
            }
        }
        SvgNode::Line { .. } | SvgNode::Polyline { .. } => {
            let style = match node {
                SvgNode::Line { style, .. } | SvgNode::Polyline { style, .. } => {
                    resolve_style(inherited, style)
                }
                _ => unreachable!(),
            };
            let Some(path) = shape_path_string(node) else {
                return;
            };
            if mode == RenderMode::PathOnly {
                out.push_str(&path);
                return;
            }

            apply_stroke_style(&style, out);
            out.push_str(&path);
            paint_stroke_only(&style, out);
        }
        SvgNode::Image {
            x,
            y,
            width,
            height,
            href,
            preserve_aspect_ratio,
            ..
        } => {
            render_image_with_resources(
                SvgPlacementRequest::from_rect(*x, *y, *width, *height, *preserve_aspect_ratio),
                Some(href.as_str()),
                out,
                resources,
            );
        }
        SvgNode::Text {
            x,
            y,
            font_size,
            font_size_attr,
            fill_specified: _fill_specified,
            fill_raw: _fill_raw,
            font_family,
            font_bold,
            font_italic,
            content,
            style,
        } => {
            if mode == RenderMode::PathOnly {
                return;
            }
            let style = resolve_style(inherited, style);
            let clip_path = style.clip_path.clone();
            let mut style = style;
            style.clip_path = None;
            let size = font_size_attr
                .as_deref()
                .and_then(|raw| resolve_svg_font_size(raw, text_ctx.font_size))
                .or(*font_size)
                .unwrap_or(text_ctx.font_size);
            let font = resolve_svg_text_font(
                style.font_family.as_deref(),
                style.font_bold,
                style.font_italic,
                font_family.as_deref(),
                *font_bold,
                *font_italic,
                text_ctx,
            );
            if let Some(clip_id) = clip_path.as_deref() {
                out.push_str("q\n");
                let (shadings, shading_counter) = resources.shading_state();
                render_clip_path(
                    clip_id,
                    defs,
                    shadings,
                    shading_counter,
                    text_object_bounding_box(*x, *y, size, content, &font),
                    out,
                );
            }

            // Use SVG-explicit font_size if set, otherwise inherit from CSS context
            let fill = paint_to_rgb(&style.fill, style.color);
            let stroke = paint_to_rgb(&style.stroke, style.color);
            let has_stroke = has_visible_stroke(&style);
            let stroke = stroke.filter(|_| has_stroke);
            let text_render_mode = match (fill.is_some(), stroke.is_some()) {
                (true, true) => 2,
                (true, false) => 0,
                (false, true) => 1,
                (false, false) => 3,
            };

            out.push_str("BT\n");
            out.push_str(&format!("/{font} {size} Tf\n"));
            out.push_str(&format!("{text_render_mode} Tr\n"));
            if let Some((r, g, b)) = fill {
                out.push_str(&format!("{r} {g} {b} rg\n"));
            }
            if let Some((r, g, b)) = stroke {
                out.push_str(&format!("{r} {g} {b} RG\n"));
                out.push_str(&format!("{} w\n", style.stroke_width));
            }
            out.push_str(&format!("1 0 0 -1 {x} {y} Tm\n"));
            let encoded = encode_pdf_text(content);
            out.push_str(&format!("({encoded}) Tj\n"));
            out.push_str("ET\n");
            if clip_path.is_some() {
                out.push_str("Q\n");
            }
        }
    }
}

fn shape_path_string(node: &SvgNode) -> Option<String> {
    let mut out = String::new();
    match node {
        SvgNode::Rect {
            x,
            y,
            width,
            height,
            ..
        } => {
            out.push_str(&format!("{x} {y} {width} {height} re\n"));
        }
        SvgNode::Circle { cx, cy, r, .. } => {
            emit_circle(*cx, *cy, *r, &mut out);
        }
        SvgNode::Ellipse { cx, cy, rx, ry, .. } => {
            emit_ellipse(*cx, *cy, *rx, *ry, &mut out);
        }
        SvgNode::Line { x1, y1, x2, y2, .. } => {
            out.push_str(&format!("{x1} {y1} m {x2} {y2} l\n"));
        }
        SvgNode::Polyline { points, .. } => {
            emit_polyline(points, false, &mut out);
        }
        SvgNode::Polygon { points, .. } => {
            emit_polyline(points, true, &mut out);
        }
        SvgNode::Path { commands, .. } => {
            emit_path(commands, &mut out);
        }
        _ => return None,
    }
    Some(out)
}

fn resolve_gradient_coords(
    gradient: &SvgLinearGradient,
    object_bbox: Option<SvgObjectBoundingBox>,
) -> Option<[f32; 4]> {
    match gradient.gradient_units {
        SvgGradientUnits::UserSpaceOnUse => {
            Some([gradient.x1, gradient.y1, gradient.x2, gradient.y2])
        }
        SvgGradientUnits::ObjectBoundingBox => {
            let object_bbox = object_bbox?;
            (!object_bbox.is_empty()).then_some(object_bbox.linear_gradient_coords(gradient))
        }
    }
}

fn render_clip_path(
    clip_path_id: &str,
    defs: &crate::parser::svg::SvgDefs,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    object_bbox: Option<SvgObjectBoundingBox>,
    out: &mut String,
) {
    let Some(clip_path) = defs.clip_paths.get(clip_path_id) else {
        return;
    };

    if clip_path.children.is_empty() {
        emit_empty_clip_path(out);
        return;
    }

    let mut transforms = Vec::new();
    match clip_path.clip_path_units {
        SvgClipPathUnits::UserSpaceOnUse => {}
        SvgClipPathUnits::ObjectBoundingBox => {
            let Some(object_units_transform) =
                object_bbox.and_then(|bbox| bbox.to_object_units_transform())
            else {
                emit_empty_clip_path(out);
                return;
            };
            transforms.push(object_units_transform);
        }
    }
    if let Some(transform) = clip_path.transform.as_ref() {
        if inverse_svg_transform(transform).is_none() {
            emit_empty_clip_path(out);
            return;
        }
        transforms.push(transform.clone());
    }

    let clip_style = ResolvedStyle {
        color: None,
        fill: SvgPaint::None,
        stroke: SvgPaint::None,
        clip_path: None,
        stroke_width: 0.0,
        font_family: None,
        font_bold: None,
        font_italic: None,
    };

    for transform in &transforms {
        write_svg_transform_matrix(transform, out);
    }

    let mut resources = SvgPdfResources::without_images(shadings, shading_counter);
    for child in &clip_path.children {
        render_node(
            child,
            clip_style.clone(),
            &SvgTextContext::default(),
            defs,
            &mut resources,
            out,
            RenderMode::PathOnly,
        );
    }
    out.push_str("W n\n");

    for transform in transforms.iter().rev() {
        if let Some(inverse_transform) = inverse_svg_transform(transform) {
            write_svg_transform_matrix(&inverse_transform, out);
        }
    }
}

fn inverse_svg_transform(transform: &SvgTransform) -> Option<SvgTransform> {
    match transform {
        SvgTransform::Matrix(a, b, c, d, e, f) => {
            let determinant = a * d - b * c;
            if determinant == 0.0 {
                return None;
            }
            let inverse_det = 1.0 / determinant;
            Some(SvgTransform::Matrix(
                d * inverse_det,
                -b * inverse_det,
                -c * inverse_det,
                a * inverse_det,
                (c * f - d * e) * inverse_det,
                (b * e - a * f) * inverse_det,
            ))
        }
    }
}

fn write_svg_transform_matrix(transform: &SvgTransform, out: &mut String) {
    match transform {
        SvgTransform::Matrix(a, b, c, d, e, f) => {
            out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
        }
    }
}

fn emit_empty_clip_path(out: &mut String) {
    out.push_str("0 0 0 0 re\nW n\n");
}

fn paint_svg_linear_gradient_fill(
    gradient: &SvgLinearGradient,
    coords: [f32; 4],
    out: &mut String,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|stop| (stop.offset, stop.color))
        .collect();
    let name = push_axial_shading(shadings, shading_counter, coords, stops);

    out.push_str("q\n");
    out.push_str("W n\n");
    if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = gradient.gradient_transform {
        out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
    }
    out.push_str(&format!("/{name} sh\n"));
    out.push_str("Q\n");
}

fn children_bounding_box(
    children: &[SvgNode],
    text_ctx: &SvgTextContext,
) -> Option<SvgObjectBoundingBox> {
    children
        .iter()
        .filter_map(|child| node_object_bounding_box(child, text_ctx))
        .reduce(SvgObjectBoundingBox::union)
}

fn node_object_bounding_box(
    node: &SvgNode,
    text_ctx: &SvgTextContext,
) -> Option<SvgObjectBoundingBox> {
    match node {
        SvgNode::Group {
            transform,
            children,
            ..
        } => children
            .iter()
            .filter_map(|child| {
                let bbox = node_object_bounding_box(child, text_ctx)?;
                Some(match transform.as_ref() {
                    Some(transform) => bbox.transform(transform),
                    None => bbox,
                })
            })
            .reduce(SvgObjectBoundingBox::union),
        SvgNode::Rect {
            x,
            y,
            width,
            height,
            ..
        }
        | SvgNode::Image {
            x,
            y,
            width,
            height,
            ..
        } => SvgObjectBoundingBox::from_extents(*x, *y, *x + *width, *y + *height),
        SvgNode::Circle { cx, cy, r, .. } => {
            SvgObjectBoundingBox::from_extents(*cx - *r, *cy - *r, *cx + *r, *cy + *r)
        }
        SvgNode::Ellipse { cx, cy, rx, ry, .. } => {
            SvgObjectBoundingBox::from_extents(*cx - *rx, *cy - *ry, *cx + *rx, *cy + *ry)
        }
        SvgNode::Line { x1, y1, x2, y2, .. } => bounding_box_from_points(&[(*x1, *y1), (*x2, *y2)]),
        SvgNode::Polyline { points, .. } | SvgNode::Polygon { points, .. } => {
            bounding_box_from_points(points)
        }
        SvgNode::Path { commands, .. } => path_commands_bounding_box(commands),
        SvgNode::Text {
            x,
            y,
            font_size,
            font_size_attr,
            font_family,
            font_bold,
            font_italic,
            content,
            style,
            ..
        } => {
            let size = font_size_attr
                .as_deref()
                .and_then(|raw| resolve_svg_font_size(raw, text_ctx.font_size))
                .or(*font_size)
                .unwrap_or(text_ctx.font_size);
            let font = resolve_svg_text_font(
                style.font_family.as_deref(),
                style.font_bold,
                style.font_italic,
                font_family.as_deref(),
                *font_bold,
                *font_italic,
                text_ctx,
            );
            text_object_bounding_box(*x, *y, size, content, &font)
        }
    }
}

fn text_object_bounding_box(
    x: f32,
    y: f32,
    font_size: f32,
    content: &str,
    font_name: &str,
) -> Option<SvgObjectBoundingBox> {
    let (font_family, bold) = font_metrics_font(font_name);
    let width = crate::fonts::str_width(content, font_size, &font_family, bold);
    let ascender = crate::fonts::ascender_ratio(&font_family) * font_size;
    let descender = crate::fonts::descender_ratio(&font_family) * font_size;
    SvgObjectBoundingBox::from_extents(x, y - ascender, x + width, y + descender)
}

fn font_metrics_font(font_name: &str) -> (FontFamily, bool) {
    let bold = font_name.contains("Bold");
    let family = parse_font_stack(font_metrics_family_name(font_name))
        .families()
        .iter()
        .find(|family| !matches!(family, FontFamily::Custom(_)))
        .cloned()
        .unwrap_or_else(|| match base_family_from_pdf_name(font_name) {
            "Times-Roman" => FontFamily::TimesRoman,
            "Courier" => FontFamily::Courier,
            _ => FontFamily::Helvetica,
        });
    (family, bold)
}

fn font_metrics_family_name(font_name: &str) -> &str {
    if matches!(
        font_name,
        name if name.starts_with("Times")
            || name.starts_with("Courier")
            || name.starts_with("Helvetica")
    ) {
        base_family_from_pdf_name(font_name)
    } else {
        font_name
    }
}

fn path_commands_bounding_box(commands: &[PathCommand]) -> Option<SvgObjectBoundingBox> {
    let mut bounds = BoundsAccumulator::default();
    let mut current = None;
    let mut subpath_start = None;

    for command in commands {
        match command {
            PathCommand::MoveTo(x, y) => {
                let point = (*x, *y);
                current = Some(point);
                subpath_start = Some(point);
                bounds.include_point(*x, *y);
            }
            PathCommand::LineTo(x, y) => {
                let point = (*x, *y);
                current = Some(point);
                bounds.include_point(*x, *y);
            }
            PathCommand::CubicTo(x1, y1, x2, y2, x, y) => {
                if let Some((x0, y0)) = current {
                    bounds.include_point(x0, y0);
                    bounds.include_point(*x, *y);
                    for t in cubic_bezier_extrema(x0, *x1, *x2, *x) {
                        if (0.0..1.0).contains(&t) {
                            bounds.include_point(
                                cubic_bezier_value(x0, *x1, *x2, *x, t),
                                cubic_bezier_value(y0, *y1, *y2, *y, t),
                            );
                        }
                    }
                    for t in cubic_bezier_extrema(y0, *y1, *y2, *y) {
                        if (0.0..1.0).contains(&t) {
                            bounds.include_point(
                                cubic_bezier_value(x0, *x1, *x2, *x, t),
                                cubic_bezier_value(y0, *y1, *y2, *y, t),
                            );
                        }
                    }
                } else {
                    bounds.include_point(*x1, *y1);
                    bounds.include_point(*x2, *y2);
                    bounds.include_point(*x, *y);
                }
                current = Some((*x, *y));
            }
            PathCommand::QuadTo(cx, cy, x, y) => {
                if let Some((x0, y0)) = current {
                    bounds.include_point(x0, y0);
                    bounds.include_point(*x, *y);
                    if let Some(t) = quadratic_bezier_extremum(x0, *cx, *x) {
                        if (0.0..1.0).contains(&t) {
                            bounds.include_point(
                                quadratic_bezier_value(x0, *cx, *x, t),
                                quadratic_bezier_value(y0, *cy, *y, t),
                            );
                        }
                    }
                    if let Some(t) = quadratic_bezier_extremum(y0, *cy, *y) {
                        if (0.0..1.0).contains(&t) {
                            bounds.include_point(
                                quadratic_bezier_value(x0, *cx, *x, t),
                                quadratic_bezier_value(y0, *cy, *y, t),
                            );
                        }
                    }
                } else {
                    bounds.include_point(*cx, *cy);
                    bounds.include_point(*x, *y);
                }
                current = Some((*x, *y));
            }
            PathCommand::ClosePath => {
                if let Some(start) = subpath_start.or(current) {
                    current = Some(start);
                    bounds.include_point(start.0, start.1);
                }
            }
        }
    }

    bounds.finish()
}

fn bounding_box_from_points(points: &[(f32, f32)]) -> Option<SvgObjectBoundingBox> {
    let mut bounds = BoundsAccumulator::default();
    for (x, y) in points {
        bounds.include_point(*x, *y);
    }
    bounds.finish()
}

fn transform_point(transform: &SvgTransform, x: f32, y: f32) -> (f32, f32) {
    match transform {
        SvgTransform::Matrix(a, b, c, d, e, f) => (a * x + c * y + e, b * x + d * y + f),
    }
}

#[derive(Default)]
struct BoundsAccumulator {
    min_x: Option<f32>,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl BoundsAccumulator {
    fn include_point(&mut self, x: f32, y: f32) {
        if let Some(min_x) = self.min_x {
            self.min_x = Some(min_x.min(x));
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
        } else {
            self.min_x = Some(x);
            self.min_y = y;
            self.max_x = x;
            self.max_y = y;
        }
    }

    fn finish(self) -> Option<SvgObjectBoundingBox> {
        SvgObjectBoundingBox::from_extents(self.min_x?, self.min_y, self.max_x, self.max_y)
    }
}

fn quadratic_bezier_extremum(p0: f32, p1: f32, p2: f32) -> Option<f32> {
    let denominator = p0 - 2.0 * p1 + p2;
    (denominator != 0.0).then_some((p0 - p1) / denominator)
}

fn quadratic_bezier_value(p0: f32, p1: f32, p2: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    mt * mt * p0 + 2.0 * mt * t * p1 + t * t * p2
}

fn cubic_bezier_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let mt = 1.0 - t;
    mt * mt * mt * p0 + 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t * p3
}

fn cubic_bezier_extrema(p0: f32, p1: f32, p2: f32, p3: f32) -> Vec<f32> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = -p0 + p1;

    if a == 0.0 {
        return (b != 0.0).then_some(vec![-c / b]).unwrap_or_default();
    }

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return Vec::new();
    }

    let sqrt_discriminant = discriminant.sqrt();
    vec![
        (-b + sqrt_discriminant) / (2.0 * a),
        (-b - sqrt_discriminant) / (2.0 * a),
    ]
}

fn resolve_svg_font_size(raw: &str, inherited_size: f32) -> Option<f32> {
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        let pct = pct.trim().parse::<f32>().ok()?;
        return Some(inherited_size * pct / 100.0);
    }
    if let Some(em) = raw.strip_suffix("em") {
        let em = em.trim().parse::<f32>().ok()?;
        return Some(inherited_size * em);
    }
    if let Some(px) = raw.strip_suffix("px") {
        let px = px.trim().parse::<f32>().ok()?;
        return Some(px * 0.75);
    }
    if let Some(pt) = raw.strip_suffix("pt") {
        return pt.trim().parse::<f32>().ok();
    }
    raw.parse::<f32>().ok().map(|px| px * 0.75)
}

fn apply_style(style: &ResolvedStyle, out: &mut String) {
    // Fill color
    if let Some((r, g, b)) = paint_to_rgb(&style.fill, style.color) {
        out.push_str(&format!("{r} {g} {b} rg\n"));
    }
    apply_stroke_style(style, out);
}

fn apply_stroke_style(style: &ResolvedStyle, out: &mut String) {
    if let Some((r, g, b)) = paint_to_rgb(&style.stroke, style.color) {
        out.push_str(&format!("{r} {g} {b} RG\n"));
    }
    if style.stroke_width > 0.0 {
        out.push_str(&format!("{} w\n", style.stroke_width));
    }
}

fn paint(style: &ResolvedStyle, out: &mut String) {
    let has_fill = paint_to_rgb(&style.fill, style.color).is_some();
    let has_stroke = has_visible_stroke(style);
    match (has_fill, has_stroke) {
        (true, true) => out.push_str("B\n"),   // fill + stroke
        (true, false) => out.push_str("f\n"),  // fill only
        (false, true) => out.push_str("S\n"),  // stroke only
        (false, false) => out.push_str("n\n"), // no paint
    }
}

fn paint_stroke_only(style: &ResolvedStyle, out: &mut String) {
    let has_stroke = has_visible_stroke(style);
    if has_stroke {
        out.push_str("S\n");
    } else {
        out.push_str("n\n");
    }
}

fn has_visible_stroke(style: &ResolvedStyle) -> bool {
    !matches!(style.stroke, SvgPaint::None | SvgPaint::Unspecified) && style.stroke_width > 0.0
}

/// Resolve the PDF font name for an SVG `<text>` element.
///
/// Precedence is:
/// 1. per-element `<text>` font attributes
/// 2. inherited SVG style from ancestor groups / the text element itself
/// 3. the outer HTML/SVG text context
fn resolve_svg_text_font(
    inherited_font_family: Option<&str>,
    inherited_font_bold: Option<bool>,
    inherited_font_italic: Option<bool>,
    font_family: Option<&str>,
    font_bold: Option<bool>,
    font_italic: Option<bool>,
    text_ctx: &SvgTextContext,
) -> String {
    let bold = font_bold
        .or(inherited_font_bold)
        .unwrap_or(text_ctx.font_bold);
    let italic = font_italic
        .or(inherited_font_italic)
        .unwrap_or(text_ctx.font_italic);

    if let Some(base) = font_family.or(inherited_font_family) {
        crate::fonts::pdf_font_name(base, bold, italic).to_string()
    } else if font_bold.is_some()
        || font_italic.is_some()
        || inherited_font_bold.is_some()
        || inherited_font_italic.is_some()
    {
        let base = base_family_from_pdf_name(&text_ctx.font_family);
        crate::fonts::pdf_font_name(base, bold, italic).to_string()
    } else {
        text_ctx.font_family.clone()
    }
}

/// Extract the base family name from a fully-qualified PDF font name.
fn base_family_from_pdf_name(name: &str) -> &str {
    if name.starts_with("Times") {
        "Times-Roman"
    } else if name.starts_with("Courier") {
        "Courier"
    } else {
        "Helvetica"
    }
}

// Emit a circle approximation using 4 cubic bezier curves
fn emit_circle(cx: f32, cy: f32, r: f32, out: &mut String) {
    emit_ellipse(cx, cy, r, r, out);
}

fn emit_ellipse(cx: f32, cy: f32, rx: f32, ry: f32, out: &mut String) {
    let k = 0.552_284_8_f32;
    let kx = rx * k;
    let ky = ry * k;
    // Start at (cx+rx, cy)
    out.push_str(&format!("{} {} m\n", cx + rx, cy));
    // Top-right quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx + rx,
        cy + ky,
        cx + kx,
        cy + ry,
        cx,
        cy + ry
    ));
    // Top-left quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx - kx,
        cy + ry,
        cx - rx,
        cy + ky,
        cx - rx,
        cy
    ));
    // Bottom-left quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx - rx,
        cy - ky,
        cx - kx,
        cy - ry,
        cx,
        cy - ry
    ));
    // Bottom-right quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx + kx,
        cy - ry,
        cx + rx,
        cy - ky,
        cx + rx,
        cy
    ));
    out.push_str("h\n"); // close path
}

fn emit_polyline(points: &[(f32, f32)], close: bool, out: &mut String) {
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{x} {y} m\n"));
        } else {
            out.push_str(&format!("{x} {y} l\n"));
        }
    }
    if close {
        out.push_str("h\n");
    }
}

fn emit_path(commands: &[PathCommand], out: &mut String) {
    for cmd in commands {
        match cmd {
            PathCommand::MoveTo(x, y) => out.push_str(&format!("{x} {y} m\n")),
            PathCommand::LineTo(x, y) => out.push_str(&format!("{x} {y} l\n")),
            PathCommand::CubicTo(x1, y1, x2, y2, x, y) => {
                out.push_str(&format!("{x1} {y1} {x2} {y2} {x} {y} c\n"));
            }
            PathCommand::QuadTo(_cx, _cy, x, y) => {
                // Convert quadratic to cubic bezier (would need current point tracking)
                // For simplicity, approximate as line to endpoint
                out.push_str(&format!("{x} {y} l\n"));
            }
            PathCommand::ClosePath => out.push_str("h\n"),
        }
    }
}

fn render_image_with_resources(
    request: SvgPlacementRequest,
    href: Option<&str>,
    out: &mut String,
    resources: &mut SvgPdfResources<'_>,
) {
    let Some(href) = href else {
        return;
    };
    let Some((raw, mime)) = crate::layout::engine::load_src_bytes(href) else {
        return;
    };

    if is_svg_image_resource(mime.as_deref(), &raw) {
        if let Some(tree) = parse_svg_image_tree(&raw) {
            render_svg_image_tree(&tree, request, out, resources);
        }
        return;
    }

    if let Some(raster) = parse_raster_image(&raw) {
        let (image_width, image_height) = raster.source_size();
        if let Some(name) = resources.register_raster(&raw) {
            render_registered_raster_image(&name, image_width, image_height, request, out);
            return;
        }
        raster.render_inline(&raw, request, out);
        return;
    }

    if let Some(tree) = parse_svg_image_tree(&raw) {
        render_svg_image_tree(&tree, request, out, resources);
    }
}

fn is_svg_image_resource(mime: Option<&str>, raw: &[u8]) -> bool {
    mime.is_some_and(|m| m.contains("svg") || m.contains("xml"))
        || raw.starts_with(b"<svg")
        || raw.starts_with(b"<?xml")
}

fn parse_svg_image_tree(raw: &[u8]) -> Option<SvgTree> {
    let svg_str = String::from_utf8_lossy(raw);
    let nodes = crate::parser::html::parse_html(&svg_str).ok()?;
    let svg_el = find_svg_element(&nodes)?;
    crate::parser::svg::parse_svg_from_element_with_ctx_and_viewport(
        svg_el,
        SvgTextContext::default(),
        None,
    )
}

fn find_svg_element(
    nodes: &[crate::parser::dom::DomNode],
) -> Option<&crate::parser::dom::ElementNode> {
    for node in nodes {
        if let Some(svg_el) = find_svg_element_in_node(node) {
            return Some(svg_el);
        }
    }
    None
}

fn find_svg_element_in_node(
    node: &crate::parser::dom::DomNode,
) -> Option<&crate::parser::dom::ElementNode> {
    match node {
        crate::parser::dom::DomNode::Element(el) if el.raw_tag_name == "svg" => Some(el),
        crate::parser::dom::DomNode::Element(el) => find_svg_element(&el.children),
        _ => None,
    }
}

fn render_svg_image_tree(
    tree: &SvgTree,
    request: SvgPlacementRequest,
    out: &mut String,
    resources: &mut SvgPdfResources<'_>,
) {
    let Some(placement) = compute_svg_placement(tree, request) else {
        return;
    };

    out.push_str("q\n");
    out.push_str(&placement.viewport.clip_path());
    out.push_str(&format!(
        "{sx} 0 0 {sy} {tx} {ty} cm\n",
        sx = placement.scale_x,
        sy = placement.scale_y,
        tx = placement.translate_x,
        ty = placement.translate_y,
    ));
    render_svg_tree_with_resources(tree, out, resources);
    out.push_str("Q\n");
}

fn render_registered_raster_image(
    name: &str,
    source_width: u32,
    source_height: u32,
    request: SvgPlacementRequest,
    out: &mut String,
) {
    let Some(placement) = compute_raster_placement(source_width, source_height, request) else {
        return;
    };
    emit_raster_draw_prefix(placement.draw_box, out);
    out.push_str(&format!("/{name} Do\n"));
    out.push_str("Q\n");
}

fn render_raster_image(
    data: &[u8],
    source_width: u32,
    source_height: u32,
    kind: RasterImageKind,
    request: SvgPlacementRequest,
    out: &mut String,
) {
    let Some(placement) = compute_raster_placement(source_width, source_height, request) else {
        return;
    };
    emit_raster_draw_prefix(placement.draw_box, out);
    emit_inline_image(data, source_width, source_height, kind, out);
    out.push_str("Q\n");
}

enum ParsedRasterImage {
    Png(crate::parser::png::PngInfo),
    Jpeg { width: u32, height: u32 },
}

impl ParsedRasterImage {
    fn source_size(&self) -> (u32, u32) {
        match self {
            Self::Png(png_info) => (png_info.width, png_info.height),
            Self::Jpeg { width, height } => (*width, *height),
        }
    }

    fn render_inline(self, raw: &[u8], request: SvgPlacementRequest, out: &mut String) {
        match self {
            Self::Png(png_info) => {
                if png_info.has_alpha() {
                    return;
                }
                render_raster_image(
                    &png_info.idat_data,
                    png_info.width,
                    png_info.height,
                    RasterImageKind::Png {
                        channels: png_info.channels,
                        bit_depth: png_info.bit_depth,
                    },
                    request,
                    out,
                );
            }
            Self::Jpeg { width, height } => {
                render_raster_image(raw, width, height, RasterImageKind::Jpeg, request, out);
            }
        }
    }
}

fn parse_raster_image(raw: &[u8]) -> Option<ParsedRasterImage> {
    if let Some(png_info) = crate::parser::png::parse_png(raw) {
        return Some(ParsedRasterImage::Png(png_info));
    }

    let (width, height) = crate::parser::jpeg::parse_jpeg_dimensions(raw)?;
    Some(ParsedRasterImage::Jpeg { width, height })
}

fn emit_raster_draw_prefix(draw_box: SvgViewportBox, out: &mut String) {
    out.push_str("q\n");
    out.push_str(&draw_box.clip_path());
    out.push_str(&format!(
        "{width} 0 0 -{height} {x} {y} cm\n",
        width = draw_box.width,
        height = draw_box.height,
        x = draw_box.x,
        y = draw_box.y + draw_box.height,
    ));
}

enum RasterImageKind {
    Png { channels: u8, bit_depth: u8 },
    Jpeg,
}

fn emit_inline_image(
    data: &[u8],
    source_width: u32,
    source_height: u32,
    kind: RasterImageKind,
    out: &mut String,
) {
    out.push_str("BI\n");
    match kind {
        RasterImageKind::Png {
            channels,
            bit_depth,
        } => {
            let color_space = if channels == 1 || channels == 2 {
                "/DeviceGray"
            } else {
                "/DeviceRGB"
            };
            out.push_str(&format!(
                "/W {source_width}\n/H {source_height}\n/CS {color_space}\n/BPC {bit_depth}\n/F [/ASCIIHexDecode /FlateDecode]\n/DP << /Predictor 15 /Colors {channels} /BitsPerComponent {bit_depth} /Columns {source_width} >>\n"
            ));
        }
        RasterImageKind::Jpeg => {
            out.push_str(&format!(
                "/W {source_width}\n/H {source_height}\n/CS /DeviceRGB\n/BPC 8\n/F [/ASCIIHexDecode /DCTDecode]\n"
            ));
        }
    }
    out.push_str("ID\n");
    out.push_str(&hex_encode(data));
    out.push_str(">\nEI\n");
}

fn hex_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        let _ = write!(&mut out, "{byte:02X}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::svg::{
        PathCommand, SvgClipPath, SvgClipPathUnits, SvgGradientStop, SvgGradientUnits,
        SvgLinearGradient, SvgNode, SvgPaint, SvgPreserveAspectRatio, SvgStyle, SvgTextContext,
        SvgTransform, SvgTree,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn style_fill(r: f32, g: f32, b: f32) -> SvgStyle {
        SvgStyle {
            color: None,
            fill: SvgPaint::Color((r, g, b)),
            stroke: SvgPaint::Unspecified,
            clip_path: None,
            stroke_width: None,
            font_family: None,
            font_bold: None,
            font_italic: None,
            opacity: 1.0,
        }
    }

    fn style_stroke(r: f32, g: f32, b: f32, w: f32) -> SvgStyle {
        SvgStyle {
            color: None,
            fill: SvgPaint::None,
            stroke: SvgPaint::Color((r, g, b)),
            clip_path: None,
            stroke_width: Some(w),
            font_family: None,
            font_bold: None,
            font_italic: None,
            opacity: 1.0,
        }
    }

    fn style_fill_and_stroke() -> SvgStyle {
        SvgStyle {
            color: None,
            fill: SvgPaint::Color((1.0, 0.0, 0.0)),
            stroke: SvgPaint::Color((0.0, 0.0, 1.0)),
            clip_path: None,
            stroke_width: Some(2.0),
            font_family: None,
            font_bold: None,
            font_italic: None,
            opacity: 1.0,
        }
    }

    fn style_none() -> SvgStyle {
        SvgStyle {
            color: None,
            fill: SvgPaint::None,
            stroke: SvgPaint::None,
            clip_path: None,
            stroke_width: None,
            font_family: None,
            font_bold: None,
            font_italic: None,
            opacity: 1.0,
        }
    }

    fn tree_with(children: Vec<SvgNode>) -> SvgTree {
        SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children,
            text_ctx: SvgTextContext::default(),
            source_markup: None,
        }
    }

    fn unique_temp_path(extension: &str) -> std::path::PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ironpress-svg-image-node-{}-{}.{}",
            std::process::id(),
            id,
            extension
        ))
    }

    fn unique_temp_svg_path() -> std::path::PathBuf {
        unique_temp_path("svg")
    }

    fn unique_temp_png_path() -> std::path::PathBuf {
        unique_temp_path("png")
    }

    struct TestImageSink {
        next_id: usize,
    }

    impl SvgImageObjectSink for TestImageSink {
        fn register_raster(&mut self, _raw_image: &[u8]) -> Option<String> {
            self.next_id += 1;
            Some(format!("Im{}", self.next_id))
        }
    }

    // ---- Rect tests ----

    #[test]
    fn render_rect_with_fill() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 60.0,
            rx: 0.0,
            ry: 0.0,
            style: style_fill(1.0, 0.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 rg\n"), "should set red fill");
        assert!(out.contains("10 20 80 60 re\n"), "should emit rect");
        assert!(out.contains("f\n"), "should paint fill only");
    }

    #[test]
    fn render_rect_fill_url_uses_svg_shading() {
        let mut tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 20.0,
            rx: 0.0,
            ry: 0.0,
            style: SvgStyle {
                fill: SvgPaint::Url("grad".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.gradients.insert(
            "grad".to_string(),
            SvgLinearGradient {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 0.0,
                gradient_units: SvgGradientUnits::UserSpaceOnUse,
                gradient_transform: None,
                stops: vec![
                    SvgGradientStop {
                        offset: 0.0,
                        color: (1.0, 0.0, 0.0),
                        opacity: 1.0,
                    },
                    SvgGradientStop {
                        offset: 1.0,
                        color: (0.0, 0.0, 1.0),
                        opacity: 1.0,
                    },
                ],
            },
        );

        let mut out = String::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_tree_with_shadings(&tree, &mut out, &mut shadings, &mut shading_counter);

        assert!(out.contains("/SH0 sh\n"));
        assert_eq!(shadings.len(), 1);
    }

    #[test]
    fn render_rect_object_bbox_gradient_maps_to_shape_bounds() {
        let mut tree = tree_with(vec![SvgNode::Rect {
            x: 10.0,
            y: 20.0,
            width: 40.0,
            height: 20.0,
            rx: 0.0,
            ry: 0.0,
            style: SvgStyle {
                fill: SvgPaint::Url("grad".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.gradients.insert(
            "grad".to_string(),
            SvgLinearGradient {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 0.0,
                gradient_units: SvgGradientUnits::ObjectBoundingBox,
                gradient_transform: None,
                stops: vec![
                    SvgGradientStop {
                        offset: 0.0,
                        color: (1.0, 0.0, 0.0),
                        opacity: 1.0,
                    },
                    SvgGradientStop {
                        offset: 1.0,
                        color: (0.0, 0.0, 1.0),
                        opacity: 1.0,
                    },
                ],
            },
        );

        let mut out = String::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_tree_with_shadings(&tree, &mut out, &mut shadings, &mut shading_counter);

        assert!(out.contains("/SH0 sh\n"));
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].coords[..4], [10.0, 20.0, 50.0, 20.0]);
    }

    #[test]
    fn render_path_object_bbox_gradient_uses_curve_extrema() {
        let mut tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::CubicTo(0.0, 10.0, 10.0, 10.0, 10.0, 0.0),
            ],
            style: SvgStyle {
                fill: SvgPaint::Url("grad".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.gradients.insert(
            "grad".to_string(),
            SvgLinearGradient {
                x1: 0.0,
                y1: 0.0,
                x2: 0.0,
                y2: 1.0,
                gradient_units: SvgGradientUnits::ObjectBoundingBox,
                gradient_transform: None,
                stops: vec![
                    SvgGradientStop {
                        offset: 0.0,
                        color: (1.0, 0.0, 0.0),
                        opacity: 1.0,
                    },
                    SvgGradientStop {
                        offset: 1.0,
                        color: (0.0, 0.0, 1.0),
                        opacity: 1.0,
                    },
                ],
            },
        );

        let mut out = String::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_tree_with_shadings(&tree, &mut out, &mut shadings, &mut shading_counter);

        assert!(out.contains("/SH0 sh\n"));
        assert_eq!(shadings.len(), 1);
        assert!((shadings[0].coords[3] - 7.5).abs() < 1e-6);
    }

    #[test]
    fn render_text_object_bbox_clip_path_uses_font_metrics() {
        let text = "WI";
        let font_size = 12.0;
        let bbox = text_object_bounding_box(10.0, 20.0, font_size, text, "Helvetica").unwrap();
        let mut tree = tree_with(vec![SvgNode::Text {
            x: 10.0,
            y: 20.0,
            font_size: Some(font_size),
            font_size_attr: None,
            fill_specified: false,
            fill_raw: None,
            font_family: Some("Helvetica".to_string()),
            font_bold: Some(false),
            font_italic: Some(false),
            content: text.to_string(),
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::ObjectBoundingBox,
                transform: None,
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains(&format!(
            "{} 0 0 {} {} {} cm\n",
            bbox.width, bbox.height, bbox.min_x, bbox.min_y
        )));
    }

    #[test]
    fn text_object_bounding_box_uses_css_font_family_mapping() {
        let font_size = 12.0;
        let bbox = text_object_bounding_box(10.0, 20.0, font_size, "WI", "Georgia").unwrap();
        let expected_width =
            crate::fonts::str_width("WI", font_size, &FontFamily::TimesRoman, false);
        let expected_top = 20.0 - crate::fonts::ascender_ratio(&FontFamily::TimesRoman) * font_size;
        let expected_bottom =
            20.0 + crate::fonts::descender_ratio(&FontFamily::TimesRoman) * font_size;

        assert!((bbox.width - expected_width).abs() < 1e-6);
        assert!((bbox.min_y - expected_top).abs() < 1e-6);
        assert!((bbox.max_y() - expected_bottom).abs() < 1e-6);
    }

    #[test]
    fn render_rect_with_stroke_only() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            rx: 0.0,
            ry: 0.0,
            style: style_stroke(0.0, 1.0, 0.0, 3.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 1 0 RG\n"), "should set green stroke");
        assert!(out.contains("3 w\n"), "should set stroke width");
        assert!(out.contains("0 0 50 50 re\n"), "should emit rect");
        assert!(out.contains("S\n"), "should paint stroke only");
    }

    #[test]
    fn render_rect_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            rx: 0.0,
            ry: 0.0,
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 rg\n"), "should set fill color");
        assert!(out.contains("0 0 1 RG\n"), "should set stroke color");
        assert!(out.contains("2 w\n"), "should set stroke width");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    #[test]
    fn render_rect_no_paint() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
            style: style_none(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"), "should emit no-paint operator");
    }

    #[test]
    fn render_group_clip_path_emits_clip_operator() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::UserSpaceOnUse,
                transform: None,
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("W n\n"), "should emit a clip path");
        assert!(out.contains("Q\n"), "should restore after clipping");
    }

    #[test]
    fn render_group_object_bbox_clip_path_scales_to_target_bounds() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 10.0,
                y: 20.0,
                width: 20.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::ObjectBoundingBox,
                transform: None,
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("20 0 0 10 10 20 cm\n"));
        assert!(out.contains("0 0 0.5 1 re\nW n\n"));
    }

    #[test]
    fn render_transformed_group_object_bbox_clip_path_uses_local_bounds() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 3.0, 5.0, 7.0)),
            children: vec![SvgNode::Rect {
                x: 10.0,
                y: 20.0,
                width: 20.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::ObjectBoundingBox,
                transform: None,
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("2 0 0 3 5 7 cm\n"));
        assert!(out.contains("20 0 0 10 10 20 cm\n"));
        assert!(!out.contains("40 0 0 30 25 67 cm\n"));
    }

    #[test]
    fn group_object_bounding_box_unions_transformed_children_tightly() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let group = SvgNode::Group {
            transform: Some(SvgTransform::Matrix(s, s, -s, s, 0.0, 0.0)),
            children: vec![
                SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                },
                SvgNode::Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 10.0,
                    height: 10.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                },
            ],
            style: SvgStyle::default(),
        };

        let bbox = node_object_bounding_box(&group, &SvgTextContext::default()).unwrap();
        assert!((bbox.min_x + 7.071_068).abs() < 1e-5);
        assert!((bbox.max_x() - 7.071_068).abs() < 1e-5);
    }

    #[test]
    fn render_group_transformed_clip_path_keeps_clip_active() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::UserSpaceOnUse,
                transform: Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 10.0, 0.0)),
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 1 10 0 cm\n"));
        let clip_index = out.find("W n\n").expect("should emit clip operator");
        let child_paint_index = out[clip_index + 4..]
            .find("0 1 0 rg\n")
            .map(|offset| clip_index + 4 + offset)
            .expect("child paint should follow clip");
        let between_clip_and_paint = &out[clip_index + 4..child_paint_index];
        assert!(
            between_clip_and_paint.contains(" cm\n"),
            "should restore the clip transform before painting children"
        );
        assert!(
            !between_clip_and_paint.contains("Q\n"),
            "clip should remain active for child painting instead of being dropped by Q"
        );
    }

    #[test]
    fn render_group_singular_clip_path_emits_empty_clip_without_ctm_leak() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::UserSpaceOnUse,
                transform: Some(SvgTransform::Matrix(0.0, 0.0, 0.0, 1.0, 10.0, 0.0)),
                children: vec![SvgNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                    rx: 0.0,
                    ry: 0.0,
                    style: style_fill(0.0, 0.0, 0.0),
                }],
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 0 0 re\nW n\n"));
        assert!(
            !out.contains("0 0 0 1 10 0 cm\n"),
            "singular clip transforms should not leak into child rendering"
        );
        assert!(out.contains("0 1 0 rg\n"));
    }

    #[test]
    fn render_group_empty_transformed_clip_path_does_not_emit_transform() {
        let mut tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 1.0, 0.0),
            }],
            style: SvgStyle {
                clip_path: Some("clip".to_string()),
                ..SvgStyle::default()
            },
        }]);
        tree.defs.clip_paths.insert(
            "clip".to_string(),
            SvgClipPath {
                clip_path_units: SvgClipPathUnits::UserSpaceOnUse,
                transform: Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 10.0, 0.0)),
                children: Vec::new(),
            },
        );

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 0 0 re\nW n\n"));
        assert!(
            !out.contains("1 0 0 1 10 0 cm\n"),
            "empty clip paths should not emit a transform that leaks into child rendering"
        );
    }

    #[test]
    fn render_missing_url_fill_consumes_path() {
        let tree = tree_with(vec![
            SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 5.0,
                rx: 0.0,
                ry: 0.0,
                style: SvgStyle {
                    fill: SvgPaint::Url("missing".to_string()),
                    ..SvgStyle::default()
                },
            },
            SvgNode::Rect {
                x: 10.0,
                y: 0.0,
                width: 5.0,
                height: 5.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(1.0, 0.0, 0.0),
            },
        ]);

        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 5 5 re\nn\n"));
        assert!(out.contains("1 0 0 rg\n1 w\n10 0 5 5 re\nf\n"));
    }

    // ---- Circle tests ----

    #[test]
    fn render_circle_with_fill() {
        let tree = tree_with(vec![SvgNode::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 25.0,
            style: style_fill(0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 1 rg\n"), "should set blue fill");
        // Circle start point: cx+r = 75, cy = 50
        assert!(out.contains("75 50 m\n"), "should move to circle start");
        // Should have 4 cubic bezier curves
        assert_eq!(out.matches(" c\n").count(), 4, "should have 4 cubic curves");
        assert!(out.contains("h\n"), "should close path");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_circle_with_stroke() {
        let tree = tree_with(vec![SvgNode::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 10.0,
            style: style_stroke(1.0, 0.0, 0.0, 1.5),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 RG\n"), "should set stroke color");
        assert!(out.contains("1.5 w\n"), "should set stroke width");
        assert!(out.contains("S\n"), "should stroke only");
    }

    // ---- Ellipse tests ----

    #[test]
    fn render_ellipse_with_fill() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 50.0,
            cy: 50.0,
            rx: 30.0,
            ry: 20.0,
            style: style_fill(0.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 1 0 rg\n"), "should set green fill");
        // Ellipse start point: cx+rx = 80, cy = 50
        assert!(out.contains("80 50 m\n"), "should move to ellipse start");
        assert_eq!(out.matches(" c\n").count(), 4, "should have 4 cubic curves");
        assert!(out.contains("h\n"), "should close path");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_ellipse_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 10.0,
            ry: 5.0,
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    // ---- Line tests ----

    #[test]
    fn render_line() {
        let tree = tree_with(vec![SvgNode::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 100.0,
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("0 0 m 100 100 l\nS\n"),
            "should emit line with stroke"
        );
    }

    #[test]
    fn render_line_with_fill_style() {
        // Fill does not apply to <line>; without a stroke, the line is not painted.
        let tree = tree_with(vec![SvgNode::Line {
            x1: 5.0,
            y1: 10.0,
            x2: 50.0,
            y2: 60.0,
            style: style_fill(1.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            !out.contains(" rg\n"),
            "should not set fill color for <line>"
        );
        assert!(out.contains("5 10 m 50 60 l\n"), "should emit line path");
        assert!(
            out.contains("n\n"),
            "should not stroke without a stroke paint"
        );
    }

    #[test]
    fn render_line_without_stroke_is_not_painted() {
        let tree = tree_with(vec![SvgNode::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"));
        assert!(!out.contains("S\n"));
    }

    // ---- Polyline tests ----

    #[test]
    fn render_polyline() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![(0.0, 0.0), (10.0, 20.0), (30.0, 40.0)],
            style: style_stroke(1.0, 0.0, 0.0, 2.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "first point should be moveto");
        assert!(out.contains("10 20 l\n"), "second point should be lineto");
        assert!(out.contains("30 40 l\n"), "third point should be lineto");
        assert!(!out.contains("h\n"), "polyline should not close path");
        assert!(out.contains("S\n"), "polyline should stroke");
    }

    #[test]
    fn render_polyline_empty() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // Should still emit S even with no points
        assert!(out.contains("S\n"));
    }

    #[test]
    fn render_polyline_without_stroke_is_not_painted() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![(0.0, 0.0), (10.0, 10.0)],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"));
        assert!(!out.contains("S\n"));
    }

    #[test]
    fn group_fill_is_inherited_by_children() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: SvgStyle::default(),
            }],
            style: SvgStyle {
                color: None,
                fill: SvgPaint::Color((1.0, 0.0, 0.0)),
                stroke: SvgPaint::Unspecified,
                clip_path: None,
                stroke_width: None,
                font_family: None,
                font_bold: None,
                font_italic: None,
                ..SvgStyle::default()
            },
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("1 0 0 rg\n"),
            "child should inherit group fill"
        );
        assert!(out.contains("f\n"), "rect should be filled");
    }

    // ---- Polygon tests ----

    #[test]
    fn render_polygon_with_fill() {
        let tree = tree_with(vec![SvgNode::Polygon {
            points: vec![(0.0, 0.0), (50.0, 0.0), (25.0, 50.0)],
            style: style_fill(0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "first point should be moveto");
        assert!(out.contains("50 0 l\n"), "second point should be lineto");
        assert!(out.contains("25 50 l\n"), "third point should be lineto");
        assert!(out.contains("h\n"), "polygon should close path");
        assert!(out.contains("f\n"), "polygon should paint fill");
    }

    #[test]
    fn render_polygon_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Polygon {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("h\n"), "polygon should close path");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    // ---- Path tests ----

    #[test]
    fn render_path_moveto_lineto() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
            ],
            style: style_fill(1.0, 0.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "should emit moveto");
        assert!(out.contains("10 10 l\n"), "should emit lineto");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_path_cubic_to() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::CubicTo(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
            ],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 2 3 4 5 6 c\n"), "should emit cubic bezier");
        assert!(out.contains("S\n"), "should stroke");
    }

    #[test]
    fn render_path_quad_to() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::QuadTo(5.0, 5.0, 10.0, 10.0),
            ],
            style: style_fill(0.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // QuadTo is approximated as lineto to endpoint
        assert!(
            out.contains("10 10 l\n"),
            "QuadTo should approximate as lineto"
        );
    }

    #[test]
    fn render_path_close() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
                PathCommand::ClosePath,
            ],
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("h\n"), "should emit close path");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    #[test]
    fn render_path_all_commands() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::CubicTo(20.0, 0.0, 20.0, 10.0, 10.0, 10.0),
                PathCommand::QuadTo(5.0, 15.0, 0.0, 10.0),
                PathCommand::ClosePath,
            ],
            style: style_fill(0.5, 0.5, 0.5),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"));
        assert!(out.contains("10 0 l\n"));
        assert!(out.contains("20 0 20 10 10 10 c\n"));
        assert!(out.contains("0 10 l\n")); // QuadTo approximated
        assert!(out.contains("h\n"));
        assert!(out.contains("f\n"));
    }

    // ---- Group tests ----

    #[test]
    fn render_group_without_transform() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(1.0, 0.0, 0.0),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.starts_with("q\n"), "should save graphics state");
        assert!(out.contains("0 0 10 10 re\n"), "should render child rect");
        assert!(out.ends_with("Q\n"), "should restore graphics state");
        // No cm operator since no transform
        assert!(
            !out.contains(" cm\n"),
            "should not have cm without transform"
        );
    }

    #[test]
    fn render_group_with_transform() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 10.0, 20.0)),
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 5.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 0.0, 0.0),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("q\n"), "should save state");
        assert!(out.contains("1 0 0 1 10 20 cm\n"), "should apply transform");
        assert!(out.contains("0 0 5 5 re\n"), "should render child");
        assert!(out.contains("Q\n"), "should restore state");
    }

    #[test]
    fn render_nested_groups() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Group {
                transform: Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 0.0, 0.0)),
                children: vec![SvgNode::Circle {
                    cx: 10.0,
                    cy: 10.0,
                    r: 5.0,
                    style: style_fill(1.0, 1.0, 0.0),
                }],
                style: SvgStyle::default(),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // Should have two q/Q pairs (nested groups)
        assert_eq!(out.matches("q\n").count(), 2, "two nested save states");
        assert_eq!(out.matches("Q\n").count(), 2, "two nested restore states");
        assert!(out.contains("2 0 0 2 0 0 cm\n"), "inner transform");
    }

    // ---- Empty tree ----

    #[test]
    fn render_empty_tree() {
        let tree = tree_with(vec![]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.is_empty(), "empty tree should produce no output");
    }

    #[test]
    fn render_image_png_data_uri_uses_inline_image() {
        let png_path = unique_temp_png_path();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0])))
            .save_with_format(&png_path, image::ImageFormat::Png)
            .unwrap();
        let tree = tree_with(vec![SvgNode::Image {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 20.0,
            href: png_path.to_string_lossy().into_owned(),
            preserve_aspect_ratio: SvgPreserveAspectRatio::None,
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        let _ = std::fs::remove_file(png_path);
        assert!(out.contains("BI\n"), "inline image should use BI");
        assert!(out.contains("/W 1\n"), "PNG width should be embedded");
        assert!(out.contains("/H 1\n"), "PNG height should be embedded");
        assert!(
            out.contains("/F [/ASCIIHexDecode /FlateDecode]\n"),
            "PNG should use hex + Flate filters"
        );
        assert!(
            out.contains("30 0 0 -20 0 20 cm\n"),
            "preserveAspectRatio=none should stretch to the target box"
        );
    }

    #[test]
    fn render_image_alpha_png_uses_xobject_when_sink_is_available() {
        let png_path = unique_temp_png_path();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 128]),
        ))
        .save_with_format(&png_path, image::ImageFormat::Png)
        .unwrap();

        let tree = tree_with(vec![SvgNode::Image {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 20.0,
            href: png_path.to_string_lossy().into_owned(),
            preserve_aspect_ratio: SvgPreserveAspectRatio::None,
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut image_sink = TestImageSink { next_id: 0 };
        let mut resources = SvgPdfResources {
            shadings: &mut shadings,
            shading_counter: &mut shading_counter,
            image_sink: Some(&mut image_sink),
        };

        render_svg_tree_with_resources(&tree, &mut out, &mut resources);
        let _ = std::fs::remove_file(png_path);

        assert!(
            !out.contains("BI\n"),
            "alpha PNGs should not use BI with a sink"
        );
        assert!(
            out.contains("/Im1 Do\n"),
            "registered image XObject should be drawn"
        );
        assert!(
            out.contains("30 0 0 -20 0 20 cm\n"),
            "registered image should still use the target box"
        );
    }

    #[test]
    fn render_raster_image_uses_target_box_not_source_pixels() {
        let mut out = String::new();
        render_raster_image(
            &[0xFF, 0xD8, 0xFF, 0xD9],
            2048,
            2048,
            RasterImageKind::Jpeg,
            SvgPlacementRequest::from_rect(10.0, 20.0, 100.0, 50.0, SvgPreserveAspectRatio::None),
            &mut out,
        );

        assert!(
            out.contains("10 20 100 50 re W n\n"),
            "inline image clip should match the target draw box"
        );
        assert!(
            out.contains("100 0 0 -50 10 70 cm\n"),
            "inline image matrix should use the target size, not source pixel dimensions"
        );
    }

    #[test]
    fn render_image_svg_file_renders_nested_svg() {
        let svg_path = unique_temp_svg_path();
        std::fs::write(
            &svg_path,
            r#"<svg width="10" height="5"><rect width="10" height="5"/></svg>"#,
        )
        .unwrap();

        let tree = tree_with(vec![SvgNode::Image {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
            href: svg_path.to_string_lossy().into_owned(),
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        let _ = std::fs::remove_file(svg_path);

        assert!(
            out.contains("2 0 0 2"),
            "nested SVG should be scaled into the target viewport"
        );
        assert!(
            out.contains("0 0 10 5 re"),
            "nested SVG content should be rendered"
        );
    }

    // ---- Multiple children ----

    #[test]
    fn render_multiple_children() {
        let tree = tree_with(vec![
            SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(1.0, 0.0, 0.0),
            },
            SvgNode::Circle {
                cx: 50.0,
                cy: 50.0,
                r: 10.0,
                style: style_fill(0.0, 1.0, 0.0),
            },
        ]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 10 10 re\n"), "should render rect");
        assert!(out.contains("60 50 m\n"), "should render circle start");
    }

    // ---- apply_style edge cases ----

    #[test]
    fn apply_style_stroke_with_zero_width_not_emitted_in_paint() {
        // stroke is Some but stroke_width is 0 => paint treats as no stroke
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
            style: SvgStyle {
                color: None,
                fill: SvgPaint::Color((1.0, 0.0, 0.0)),
                stroke: SvgPaint::Color((0.0, 0.0, 0.0)),
                clip_path: None,
                stroke_width: Some(0.0),
                font_family: None,
                font_bold: None,
                font_italic: None,
                opacity: 1.0,
            },
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // stroke color is emitted by apply_style (it doesn't check width)
        assert!(out.contains("0 0 0 RG\n"), "stroke color still applied");
        // but paint should be fill-only because stroke_width is 0
        assert!(out.contains("f\n"), "paint should be fill only");
        assert!(!out.contains("B\n"), "should not be fill+stroke");
    }

    // ---- paint edge: stroke present but no fill ----

    #[test]
    fn paint_stroke_only_no_fill() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
            ],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("S\n"), "should stroke only");
        assert!(!out.contains("f\n"), "should not fill");
        assert!(!out.contains("B\n"), "should not fill+stroke");
    }

    // ---- paint edge: neither fill nor stroke ----

    #[test]
    fn paint_no_fill_no_stroke() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 10.0,
            ry: 10.0,
            style: style_none(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"), "should emit no-paint");
    }

    #[test]
    fn text_fill_none_does_not_fallback_to_context_color() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("none".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    color: None,
                    fill: SvgPaint::None,
                    stroke: SvgPaint::Unspecified,
                    clip_path: None,
                    stroke_width: None,
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("3 Tr\n"),
            "fill:none should disable text painting"
        );
        assert!(
            !out.contains(" rg\n"),
            "explicit fill:none should not fall back to inherited text color"
        );
        assert!(out.contains("(Hello) Tj\n"));
    }

    #[test]
    fn text_fill_none_with_stroke_renders_stroked_glyphs() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("none".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    color: None,
                    fill: SvgPaint::None,
                    stroke: SvgPaint::Color((1.0, 0.0, 0.0)),
                    clip_path: None,
                    stroke_width: Some(1.5),
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext::default(),
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("1 Tr\n"),
            "stroke-only text should use stroke render mode"
        );
        assert!(
            out.contains("1 0 0 RG\n"),
            "stroke-only text should set the stroke color"
        );
        assert!(
            out.contains("1.5 w\n"),
            "stroke-only text should set the stroke width"
        );
        assert!(
            !out.contains("3 Tr\n"),
            "stroke-only text must not be invisible"
        );
    }

    #[test]
    fn text_fill_defaults_to_black_when_unspecified() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: false,
                fill_raw: None,
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle::default(),
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("0 0 0 rg\n"),
            "unspecified SVG text fill should default to black"
        );
    }

    #[test]
    fn text_font_size_percent_scales_from_context() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: Some("150%".to_string()),
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    color: None,
                    fill: SvgPaint::Color((0.0, 0.0, 0.0)),
                    stroke: SvgPaint::Unspecified,
                    clip_path: None,
                    stroke_width: None,
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                font_size: 12.0,
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("/Helvetica 18 Tf\n"),
            "150% font-size should resolve from the inherited SVG text size"
        );
    }

    #[test]
    fn text_font_size_unitless_number_treated_as_px() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: Some("12".to_string()),
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    color: None,
                    fill: SvgPaint::Color((0.0, 0.0, 0.0)),
                    stroke: SvgPaint::Unspecified,
                    clip_path: None,
                    stroke_width: None,
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext::default(),
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("/Helvetica 9 Tf\n"),
            "unitless SVG font-size should resolve like px"
        );
    }

    #[test]
    fn text_inherits_group_font_family_and_style() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Group {
                transform: None,
                style: SvgStyle {
                    font_family: Some("Courier".to_string()),
                    font_bold: Some(true),
                    font_italic: Some(true),
                    ..SvgStyle::default()
                },
                children: vec![SvgNode::Text {
                    x: 10.0,
                    y: 20.0,
                    font_size: None,
                    font_size_attr: None,
                    fill_specified: true,
                    fill_raw: Some("currentColor".to_string()),
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    content: "Hello".to_string(),
                    style: SvgStyle {
                        fill: SvgPaint::CurrentColor,
                        ..SvgStyle::default()
                    },
                }],
            }],
            text_ctx: SvgTextContext::default(),
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("/Courier-BoldOblique 12 Tf\n"),
            "group font inheritance should reach SVG text rendering"
        );
    }

    #[test]
    fn text_fill_current_color_uses_context_color() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    color: None,
                    fill: SvgPaint::CurrentColor,
                    stroke: SvgPaint::Unspecified,
                    clip_path: None,
                    stroke_width: None,
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                color: Some((0.0, 0.5, 1.0)),
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0.5 1 rg\n"));
    }

    #[test]
    fn text_fill_current_color_inherits_nested_svg_color() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Group {
                transform: None,
                style: SvgStyle {
                    color: Some((1.0, 0.0, 0.0)),
                    ..SvgStyle::default()
                },
                children: vec![SvgNode::Text {
                    x: 10.0,
                    y: 20.0,
                    font_size: None,
                    font_size_attr: None,
                    fill_specified: true,
                    fill_raw: Some("currentColor".to_string()),
                    font_family: None,
                    font_bold: None,
                    font_italic: None,
                    content: "Hello".to_string(),
                    style: SvgStyle {
                        fill: SvgPaint::CurrentColor,
                        ..SvgStyle::default()
                    },
                }],
            }],
            text_ctx: SvgTextContext {
                color: Some((0.0, 0.5, 1.0)),
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 rg\n"));
    }

    #[test]
    fn text_invalid_fill_defaults_to_black() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("bogus".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle::default(),
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
            source_markup: None,
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 0 rg\n"));
    }
}
