//! Typed PDF geometry across layout and paint coordinate boundaries.

mod boxes;
mod rounded;
#[cfg(test)]
mod tests;

pub(super) use boxes::*;
pub(super) use rounded::RoundedRect;

use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::TransformOrigin;
use crate::types::{CornerRadii, EdgeSizes};
use crate::util::{RasterDimensions, RasterTile};
use std::ops::{Add, Mul, Sub};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct PdfPoint {
    pub(super) x: f32,
    pub(super) y: f32,
}

impl PdfPoint {
    pub(super) const ORIGIN: Self = Self::new(0.0, 0.0);

    pub(super) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(super) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add<PdfVector> for PdfPoint {
    type Output = Self;

    fn add(self, vector: PdfVector) -> Self::Output {
        Self::new(self.x + vector.x, self.y + vector.y)
    }
}

impl Sub for PdfPoint {
    type Output = PdfVector;

    fn sub(self, other: Self) -> Self::Output {
        PdfVector::new(self.x - other.x, self.y - other.y)
    }
}

impl Sub<PdfVector> for PdfPoint {
    type Output = Self;

    fn sub(self, vector: PdfVector) -> Self::Output {
        Self::new(self.x - vector.x, self.y - vector.y)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct PdfVector {
    pub(super) x: f32,
    pub(super) y: f32,
}

impl PdfVector {
    pub(super) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(super) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub(super) fn is_positive(self) -> bool {
        self.is_finite() && self.x > 0.0 && self.y > 0.0
    }

    pub(super) fn component_quotient(self, divisor: Self) -> Option<Self> {
        divisor
            .is_positive()
            .then(|| Self::new(self.x / divisor.x, self.y / divisor.y))
            .filter(|quotient| quotient.is_finite())
    }

    pub(super) const fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }
}

impl Add for PdfVector {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for PdfVector {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Mul<f32> for PdfVector {
    type Output = Self;

    fn mul(self, scale: f32) -> Self::Output {
        Self::new(self.x * scale, self.y * scale)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PdfEllipse {
    pub(super) center: PdfPoint,
    pub(super) radii: PdfVector,
}

impl PdfEllipse {
    pub(super) const fn new(center: PdfPoint, radii: PdfVector) -> Self {
        Self { center, radii }
    }

    pub(super) const fn circle(center: PdfPoint, radius: f32) -> Self {
        Self::new(center, PdfVector::new(radius, radius))
    }

    /// Append the shared conic approximation, starting at the right-hand vertex.
    pub(super) fn push_path(self, content: &mut String) {
        rounded::push_ellipse_path(content, self.center, self.radii);
    }
}

/// PDF affine matrix `[a b c d e f]`, grouped as two basis vectors and a
/// translation instead of six unrelated coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PdfMatrix {
    pub(super) x_axis: PdfVector,
    pub(super) y_axis: PdfVector,
    pub(super) translation: PdfPoint,
}

impl PdfMatrix {
    pub(super) const IDENTITY: Self = Self::new(
        PdfVector::new(1.0, 0.0),
        PdfVector::new(0.0, 1.0),
        PdfPoint::ORIGIN,
    );

    pub(super) const fn new(x_axis: PdfVector, y_axis: PdfVector, translation: PdfPoint) -> Self {
        Self {
            x_axis,
            y_axis,
            translation,
        }
    }

    pub(super) const fn scale(scale: PdfVector) -> Self {
        Self::new(
            PdfVector::new(scale.x, 0.0),
            PdfVector::new(0.0, scale.y),
            PdfPoint::ORIGIN,
        )
    }

    pub(super) const fn translate(translation: PdfPoint) -> Self {
        Self::new(
            PdfVector::new(1.0, 0.0),
            PdfVector::new(0.0, 1.0),
            translation,
        )
    }

    pub(super) const fn rotate_around(pivot: PdfPoint, sin: f32, cos: f32) -> Self {
        let one_minus_cos = 1.0 - cos;
        Self::new(
            PdfVector::new(cos, sin),
            PdfVector::new(-sin, cos),
            PdfPoint::new(
                sin * pivot.y + one_minus_cos * pivot.x,
                -sin * pivot.x + one_minus_cos * pivot.y,
            ),
        )
    }

    pub(super) const fn components(self) -> [f32; 6] {
        [
            self.x_axis.x,
            self.x_axis.y,
            self.y_axis.x,
            self.y_axis.y,
            self.translation.x,
            self.translation.y,
        ]
    }

    pub(super) fn cm_operator(self) -> String {
        let [a, b, c, d, e, f] = self.components();
        format!("{a} {b} {c} {d} {e} {f} cm\n")
    }

    pub(super) fn is_invertible(self) -> bool {
        self.x_axis.is_finite()
            && self.y_axis.is_finite()
            && self.translation.is_finite()
            && self.x_axis.x * self.y_axis.y - self.x_axis.y * self.y_axis.x != 0.0
    }

    pub(super) fn inverse(self) -> Option<Self> {
        if !self.is_invertible() {
            return None;
        }
        let [a, b, c, d, e, f] = self.components().map(f64::from);
        let determinant = a * d - b * c;
        Some(Self::new(
            PdfVector::new((d / determinant) as f32, (-b / determinant) as f32),
            PdfVector::new((-c / determinant) as f32, (a / determinant) as f32),
            PdfPoint::new(
                ((c * f - d * e) / determinant) as f32,
                ((b * e - a * f) / determinant) as f32,
            ),
        ))
    }

    const fn transform_vector(self, vector: PdfVector) -> PdfVector {
        PdfVector::new(
            self.x_axis.x * vector.x + self.y_axis.x * vector.y,
            self.x_axis.y * vector.x + self.y_axis.y * vector.y,
        )
    }

    pub(super) const fn transform_point(self, point: PdfPoint) -> PdfPoint {
        let transformed = self.transform_vector(PdfVector::new(point.x, point.y));
        PdfPoint::new(
            transformed.x + self.translation.x,
            transformed.y + self.translation.y,
        )
    }
}

impl Default for PdfMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mul for PdfMatrix {
    type Output = Self;

    /// Compose transforms so `(left * right)` applies `right` first.
    fn mul(self, right: Self) -> Self::Output {
        Self::new(
            self.transform_vector(right.x_axis),
            self.transform_vector(right.y_axis),
            self.transform_point(right.translation),
        )
    }
}

/// Axis-aligned rectangle in PDF page coordinates: x grows right and y grows
/// up from the bottom-left corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PdfRect {
    pub(super) left: f32,
    pub(super) bottom: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

impl PdfRect {
    pub(super) const fn new(left: f32, bottom: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            bottom,
            width,
            height,
        }
    }

    /// Construct from a top-left position without leaking top-down coordinates
    /// into the rest of the PDF renderer.
    pub(super) const fn from_top(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self::new(left, top - height, width, height)
    }

    pub(super) const fn right(self) -> f32 {
        self.left + self.width
    }

    pub(super) const fn top(self) -> f32 {
        self.bottom + self.height
    }

    pub(super) const fn top_left(self) -> PdfPoint {
        PdfPoint::new(self.left, self.top())
    }

    /// Resolve a CSS top-left transform origin into the PDF rectangle's
    /// bottom-left coordinate system.
    pub(super) fn css_transform_origin(self, origin: TransformOrigin) -> PdfPoint {
        let (x, y) = origin.resolve(self.width, self.height);
        PdfPoint::new(self.left + x, self.top() - y)
    }

    /// PDF function domain order: x-min, x-max, y-min, y-max.
    pub(super) const fn xy_domain(self) -> [f32; 4] {
        [self.left, self.right(), self.bottom, self.top()]
    }

    /// Map a top-down pixel tile onto this PDF rectangle without changing the
    /// requested full-surface sampling grid.
    pub(super) fn raster_tile(self, dimensions: RasterDimensions, tile: RasterTile) -> Self {
        let scale_x = self.width / dimensions.width as f32;
        let scale_y = self.height / dimensions.height as f32;
        Self::new(
            self.left + tile.x as f32 * scale_x,
            self.top() - (tile.y + tile.height) as f32 * scale_y,
            tile.width as f32 * scale_x,
            tile.height as f32 * scale_y,
        )
    }

    pub(super) fn is_empty(self) -> bool {
        ![self.left, self.bottom, self.width, self.height]
            .into_iter()
            .all(f32::is_finite)
            || self.width <= 0.0
            || self.height <= 0.0
    }

    pub(super) fn transformed_bounds(self, transform: PdfMatrix) -> Self {
        let points = [
            PdfPoint::new(self.left, self.bottom),
            PdfPoint::new(self.right(), self.bottom),
            PdfPoint::new(self.left, self.top()),
            PdfPoint::new(self.right(), self.top()),
        ]
        .map(|point| transform.transform_point(point));
        let left = points
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let right = points
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = points
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let top = points
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        Self::new(left, bottom, right - left, top - bottom)
    }

    pub(super) fn covers_with_margin(self, inner: Self, margin: f32) -> bool {
        self.left <= inner.left - margin
            && self.bottom <= inner.bottom - margin
            && self.right() >= inner.right() + margin
            && self.top() >= inner.top() + margin
    }

    pub(super) fn intersection(self, other: Self) -> Option<Self> {
        let left = self.left.max(other.left);
        let bottom = self.bottom.max(other.bottom);
        let right = self.right().min(other.right());
        let top = self.top().min(other.top());
        let intersection = Self::new(left, bottom, right - left, top - bottom);
        (!intersection.is_empty()).then_some(intersection)
    }

    pub(super) const fn translate(self, dx: f32, dy: f32) -> Self {
        Self::new(self.left + dx, self.bottom + dy, self.width, self.height)
    }

    /// Place a top-down raster whose origin is an equal outset from this
    /// rectangle's top-left corner.
    ///
    /// Raster dimensions are already device-quantized. Retaining the top-left
    /// anchor while using their physical size avoids rescaling a finite blur
    /// kernel over the source rectangle's fractional device-pixel remainder.
    pub(super) fn top_left_raster_outset(
        self,
        outset: f32,
        raster_size: crate::types::Size,
    ) -> Self {
        Self::from_top(
            self.left - outset,
            self.top() + outset,
            raster_size.width,
            raster_size.height,
        )
    }

    /// Inset physical edges. The authored origin shift is retained even when
    /// the edges consume the entire rectangle; only the resulting extents are
    /// clamped.
    pub(super) fn inset(self, edges: EdgeSizes) -> Self {
        Self::new(
            self.left + edges.left,
            self.bottom + edges.bottom,
            (self.width - edges.horizontal()).max(0.0),
            (self.height - edges.vertical()).max(0.0),
        )
    }

    pub(super) const fn outset(self, edges: EdgeSizes) -> Self {
        Self::new(
            self.left - edges.left,
            self.bottom - edges.bottom,
            self.width + edges.horizontal(),
            self.height + edges.top + edges.bottom,
        )
    }

    pub(super) const fn outset_uniform(self, amount: f32) -> Self {
        self.outset(EdgeSizes::uniform(amount))
    }

    pub(super) fn rect_path(self) -> String {
        format!(
            "{} {} {} {} re\n",
            self.left, self.bottom, self.width, self.height
        )
    }

    pub(super) const fn rounded(self, radii: CornerRadii) -> RoundedRect {
        RoundedRect::new(self, radii)
    }
}

impl From<PdfRect> for SvgViewportBox {
    fn from(rect: PdfRect) -> Self {
        Self::new(rect.left, rect.bottom, rect.width, rect.height)
    }
}
