/// Page size in points (1 pt = 1/72 inch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    /// Page width in points.
    pub width: f32,
    /// Page height in points.
    pub height: f32,
}

impl PageSize {
    /// ISO A4: 210 mm x 297 mm (595.28 x 841.89 pt).
    pub const A4: Self = Self {
        width: 595.28,
        height: 841.89,
    };
    /// US Letter: 8.5 x 11 in (612 x 792 pt).
    pub const LETTER: Self = Self {
        width: 612.0,
        height: 792.0,
    };
    /// US Legal: 8.5 x 14 in (612 x 1008 pt).
    pub const LEGAL: Self = Self {
        width: 612.0,
        height: 1008.0,
    };

    /// Create a custom page size from width and height in points.
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl Default for PageSize {
    fn default() -> Self {
        Self::A4
    }
}

/// Page margins in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margin {
    /// Top margin in points.
    pub top: f32,
    /// Right margin in points.
    pub right: f32,
    /// Bottom margin in points.
    pub bottom: f32,
    /// Left margin in points.
    pub left: f32,
}

impl Margin {
    /// Create margins with individual values for each side.
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Create margins with the same value on all sides.
    pub fn uniform(v: f32) -> Self {
        Self::new(v, v, v, v)
    }
}

impl Default for Margin {
    fn default() -> Self {
        Self::uniform(72.0) // 1 inch
    }
}

/// Edge sizes for margin, padding, border.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// RGBA color.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[allow(dead_code)]
impl Color {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_f32_rgb(self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_default_is_black() {
        let c = Color::default();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn color_to_f32_rgb() {
        let c = Color::rgb(255, 128, 0);
        let (r, g, b) = c.to_f32_rgb();
        assert!((r - 1.0).abs() < 0.01);
        assert!((g - 0.502).abs() < 0.01);
        assert!((b - 0.0).abs() < 0.01);
    }

    #[test]
    fn page_size_default_is_a4() {
        let ps = PageSize::default();
        assert!((ps.width - 595.28).abs() < 0.01);
    }

    #[test]
    fn margin_uniform() {
        let m = Margin::uniform(36.0);
        assert_eq!(m.top, 36.0);
        assert_eq!(m.right, 36.0);
        assert_eq!(m.bottom, 36.0);
        assert_eq!(m.left, 36.0);
    }
}
