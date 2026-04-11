use crate::parser::svg::{SvgAlign, SvgMeetOrSlice, SvgPreserveAspectRatio, SvgTree};

#[derive(Debug, Clone, Copy)]
pub(crate) struct SvgViewportBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SvgViewportBox {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn clip_path(self) -> String {
        format!(
            "{x} {y} {width} {height} re W n\n",
            x = self.x,
            y = self.y,
            width = self.width,
            height = self.height,
        )
    }

    pub const fn translate(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self::new(left, top, right - left, bottom - top)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SvgPlacementRequest {
    pub viewport: SvgViewportBox,
    pub preserve_aspect_ratio: SvgPreserveAspectRatio,
}

impl SvgPlacementRequest {
    pub const fn new(
        viewport: SvgViewportBox,
        preserve_aspect_ratio: SvgPreserveAspectRatio,
    ) -> Self {
        Self {
            viewport,
            preserve_aspect_ratio,
        }
    }

    pub const fn from_rect(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        preserve_aspect_ratio: SvgPreserveAspectRatio,
    ) -> Self {
        Self::new(
            SvgViewportBox::new(x, y, width, height),
            preserve_aspect_ratio,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SvgPlacement {
    pub viewport: SvgViewportBox,
    pub draw_box: SvgViewportBox,
    pub scale_x: f32,
    pub scale_y: f32,
    pub translate_x: f32,
    pub translate_y: f32,
}

#[derive(Debug, Clone, Copy)]
struct SvgSourceBox {
    min_x: f32,
    min_y: f32,
    width: f32,
    height: f32,
}

impl SvgSourceBox {
    const fn new(min_x: f32, min_y: f32, width: f32, height: f32) -> Self {
        Self {
            min_x,
            min_y,
            width,
            height,
        }
    }

    fn from_tree(tree: &SvgTree) -> Option<Self> {
        if let Some(view_box) = tree.view_box.as_ref() {
            if view_box.width > 0.0 && view_box.height > 0.0 {
                return Some(Self::new(
                    view_box.min_x,
                    view_box.min_y,
                    view_box.width,
                    view_box.height,
                ));
            }
        }

        let width = tree.width.max(0.0);
        let height = tree.height.max(0.0);
        if width > 0.0 && height > 0.0 {
            Some(Self::new(0.0, 0.0, width, height))
        } else {
            None
        }
    }

    fn from_raster(width: f32, height: f32) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            None
        } else {
            Some(Self::new(0.0, 0.0, width, height))
        }
    }

    fn placement(self, request: SvgPlacementRequest) -> Option<SvgPlacement> {
        let draw_box = self.fit(request)?;
        let scale_x = draw_box.width / self.width.max(f32::EPSILON);
        let scale_y = draw_box.height / self.height.max(f32::EPSILON);

        Some(SvgPlacement {
            viewport: request.viewport,
            draw_box,
            scale_x,
            scale_y,
            translate_x: draw_box.x - self.min_x * scale_x,
            translate_y: draw_box.y - self.min_y * scale_y,
        })
    }

    fn fit(self, request: SvgPlacementRequest) -> Option<SvgViewportBox> {
        if request.viewport.width < 0.0 || request.viewport.height < 0.0 {
            return None;
        }

        match request.preserve_aspect_ratio {
            SvgPreserveAspectRatio::None => Some(request.viewport),
            SvgPreserveAspectRatio::Align {
                align,
                meet_or_slice,
            } => {
                let scale_x = request.viewport.width / self.width;
                let scale_y = request.viewport.height / self.height;
                let scale = match meet_or_slice {
                    SvgMeetOrSlice::Meet => scale_x.min(scale_y),
                    SvgMeetOrSlice::Slice => scale_x.max(scale_y),
                };
                let draw_width = self.width * scale;
                let draw_height = self.height * scale;
                let offset_x = match align {
                    SvgAlign::TopLeft | SvgAlign::CenterLeft | SvgAlign::BottomLeft => 0.0,
                    SvgAlign::TopCenter | SvgAlign::Center | SvgAlign::BottomCenter => {
                        (request.viewport.width - draw_width) / 2.0
                    }
                    SvgAlign::TopRight | SvgAlign::CenterRight | SvgAlign::BottomRight => {
                        request.viewport.width - draw_width
                    }
                };
                let offset_y = match align {
                    SvgAlign::TopLeft | SvgAlign::TopCenter | SvgAlign::TopRight => 0.0,
                    SvgAlign::CenterLeft | SvgAlign::Center | SvgAlign::CenterRight => {
                        (request.viewport.height - draw_height) / 2.0
                    }
                    SvgAlign::BottomLeft | SvgAlign::BottomCenter | SvgAlign::BottomRight => {
                        request.viewport.height - draw_height
                    }
                };

                Some(SvgViewportBox::new(
                    request.viewport.x + offset_x,
                    request.viewport.y + offset_y,
                    draw_width,
                    draw_height,
                ))
            }
        }
    }
}

pub(crate) fn compute_svg_placement(
    tree: &SvgTree,
    request: SvgPlacementRequest,
) -> Option<SvgPlacement> {
    SvgSourceBox::from_tree(tree)?.placement(request)
}

pub(crate) fn compute_raster_placement(
    source_width: u32,
    source_height: u32,
    request: SvgPlacementRequest,
) -> Option<SvgPlacement> {
    SvgSourceBox::from_raster(source_width as f32, source_height as f32)?.placement(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::svg::{
        SvgAlign, SvgDefs, SvgMeetOrSlice, SvgPreserveAspectRatio, SvgTextContext, SvgTree, ViewBox,
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_par_none() -> SvgPreserveAspectRatio {
        SvgPreserveAspectRatio::None
    }

    fn make_par_align(align: SvgAlign, meet_or_slice: SvgMeetOrSlice) -> SvgPreserveAspectRatio {
        SvgPreserveAspectRatio::Align {
            align,
            meet_or_slice,
        }
    }

    /// Build a minimal `SvgTree` using explicit dimensions (no view-box).
    fn make_tree(width: f32, height: f32) -> SvgTree {
        SvgTree {
            width,
            height,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: SvgPreserveAspectRatio::None,
            view_box: None,
            defs: SvgDefs::default(),
            children: vec![],
            text_ctx: SvgTextContext::default(),
            source_markup: None,
        }
    }

    /// Build a minimal `SvgTree` with an explicit view-box.
    fn make_tree_with_viewbox(vb_min_x: f32, vb_min_y: f32, vb_w: f32, vb_h: f32) -> SvgTree {
        let mut tree = make_tree(0.0, 0.0);
        tree.view_box = Some(ViewBox {
            min_x: vb_min_x,
            min_y: vb_min_y,
            width: vb_w,
            height: vb_h,
        });
        tree
    }

    // -----------------------------------------------------------------------
    // SvgViewportBox helpers
    // -----------------------------------------------------------------------

    #[test]
    fn viewport_clip_path_format() {
        let vp = SvgViewportBox::new(10.0, 20.0, 100.0, 50.0);
        let clip = vp.clip_path();
        assert_eq!(clip, "10 20 100 50 re W n\n");
    }

    #[test]
    fn viewport_translate_shifts_origin() {
        let vp = SvgViewportBox::new(5.0, 10.0, 40.0, 30.0);
        let translated = vp.translate(3.0, -2.0);
        assert_eq!(translated.x, 8.0);
        assert_eq!(translated.y, 8.0);
        // Dimensions must be unchanged.
        assert_eq!(translated.width, 40.0);
        assert_eq!(translated.height, 30.0);
    }

    #[test]
    fn viewport_translate_zero_is_identity() {
        let vp = SvgViewportBox::new(1.0, 2.0, 3.0, 4.0);
        let t = vp.translate(0.0, 0.0);
        assert_eq!(t.x, vp.x);
        assert_eq!(t.y, vp.y);
        assert_eq!(t.width, vp.width);
        assert_eq!(t.height, vp.height);
    }

    #[test]
    fn viewport_union_encloses_both_boxes() {
        let a = SvgViewportBox::new(0.0, 0.0, 10.0, 10.0);
        let b = SvgViewportBox::new(5.0, 5.0, 10.0, 10.0);
        let u = a.union(b);
        assert_eq!(u.x, 0.0);
        assert_eq!(u.y, 0.0);
        assert_eq!(u.width, 15.0);
        assert_eq!(u.height, 15.0);
    }

    #[test]
    fn viewport_union_non_overlapping() {
        let a = SvgViewportBox::new(0.0, 0.0, 10.0, 5.0);
        let b = SvgViewportBox::new(20.0, 10.0, 10.0, 5.0);
        let u = a.union(b);
        assert_eq!(u.x, 0.0);
        assert_eq!(u.y, 0.0);
        assert_eq!(u.width, 30.0);
        assert_eq!(u.height, 15.0);
    }

    // -----------------------------------------------------------------------
    // compute_raster_placement
    // -----------------------------------------------------------------------

    #[test]
    fn raster_placement_fills_viewport_with_par_none() {
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 200.0, 100.0, make_par_none());
        let p = compute_raster_placement(400, 200, request).expect("should return a placement");
        // With PAR=None the draw_box equals the viewport.
        assert_eq!(p.draw_box.x, 0.0);
        assert_eq!(p.draw_box.y, 0.0);
        assert_eq!(p.draw_box.width, 200.0);
        assert_eq!(p.draw_box.height, 100.0);
        // Scales are viewport/source.
        assert!(
            (p.scale_x - 0.5).abs() < 1e-5,
            "scale_x should be 0.5, got {}",
            p.scale_x
        );
        assert!(
            (p.scale_y - 0.5).abs() < 1e-5,
            "scale_y should be 0.5, got {}",
            p.scale_y
        );
    }

    #[test]
    fn raster_placement_zero_width_returns_none() {
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, make_par_none());
        assert!(compute_raster_placement(0, 100, request).is_none());
    }

    #[test]
    fn raster_placement_zero_height_returns_none() {
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, make_par_none());
        assert!(compute_raster_placement(100, 0, request).is_none());
    }

    #[test]
    fn raster_placement_zero_both_returns_none() {
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, make_par_none());
        assert!(compute_raster_placement(0, 0, request).is_none());
    }

    #[test]
    fn raster_placement_meet_preserves_aspect_ratio() {
        // Source is 200×100 (2:1), viewport is 100×100 (1:1).
        // Meet → use the smaller scale (0.5), so draw box is 100×50.
        let par = make_par_align(SvgAlign::Center, SvgMeetOrSlice::Meet);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, par);
        let p = compute_raster_placement(200, 100, request).unwrap();
        assert_eq!(p.draw_box.width, 100.0);
        assert_eq!(p.draw_box.height, 50.0);
        // Centered vertically → y offset = (100 - 50) / 2 = 25.
        assert!(
            (p.draw_box.y - 25.0).abs() < 1e-5,
            "y should be 25, got {}",
            p.draw_box.y
        );
    }

    // -----------------------------------------------------------------------
    // compute_svg_placement
    // -----------------------------------------------------------------------

    #[test]
    fn svg_placement_from_tree_dimensions() {
        let tree = make_tree(80.0, 40.0);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 160.0, 80.0, make_par_none());
        let p = compute_svg_placement(&tree, request).expect("should return placement");
        // PAR=None → draw_box = viewport.
        assert_eq!(p.draw_box.width, 160.0);
        assert_eq!(p.draw_box.height, 80.0);
        assert!((p.scale_x - 2.0).abs() < 1e-5);
        assert!((p.scale_y - 2.0).abs() < 1e-5);
    }

    #[test]
    fn svg_placement_from_tree_zero_dimensions_returns_none() {
        let tree = make_tree(0.0, 0.0);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, make_par_none());
        assert!(compute_svg_placement(&tree, request).is_none());
    }

    #[test]
    fn svg_placement_uses_viewbox_over_tree_dimensions() {
        // Tree has no intrinsic size but a valid viewBox.
        let tree = make_tree_with_viewbox(10.0, 5.0, 100.0, 50.0);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 200.0, 100.0, make_par_none());
        let p = compute_svg_placement(&tree, request).expect("should return placement");
        // PAR=None → draw_box spans full viewport.
        assert_eq!(p.draw_box.width, 200.0);
        assert_eq!(p.draw_box.height, 100.0);
        // Scales are viewport / viewbox dimensions.
        assert!((p.scale_x - 2.0).abs() < 1e-5);
        assert!((p.scale_y - 2.0).abs() < 1e-5);
        // translate_x = draw_box.x - min_x * scale_x = 0 - 10*2 = -20.
        assert!(
            (p.translate_x - (-20.0)).abs() < 1e-5,
            "translate_x={}",
            p.translate_x
        );
        assert!(
            (p.translate_y - (-10.0)).abs() < 1e-5,
            "translate_y={}",
            p.translate_y
        );
    }

    #[test]
    fn svg_placement_viewbox_zero_dimensions_returns_none() {
        let tree = make_tree_with_viewbox(0.0, 0.0, 0.0, 0.0);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, make_par_none());
        // viewBox is 0×0, tree dims are also 0 → should fall back to None.
        assert!(compute_svg_placement(&tree, request).is_none());
    }

    // -----------------------------------------------------------------------
    // fit() via SvgSourceBox – tested through compute_raster_placement
    // -----------------------------------------------------------------------

    #[test]
    fn fit_par_none_returns_viewport_unchanged() {
        let request = SvgPlacementRequest::from_rect(5.0, 10.0, 80.0, 40.0, make_par_none());
        let p = compute_raster_placement(200, 100, request).unwrap();
        assert_eq!(p.draw_box.x, 5.0);
        assert_eq!(p.draw_box.y, 10.0);
        assert_eq!(p.draw_box.width, 80.0);
        assert_eq!(p.draw_box.height, 40.0);
    }

    #[test]
    fn fit_align_top_left_meet() {
        // Source 100×200 (1:2), viewport 100×100 (1:1).
        // Meet → scale = min(1.0, 0.5) = 0.5 → draw 50×100.
        // TopLeft → no offset; x=0, y=0.
        let par = make_par_align(SvgAlign::TopLeft, SvgMeetOrSlice::Meet);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, par);
        let p = compute_raster_placement(100, 200, request).unwrap();
        assert_eq!(p.draw_box.x, 0.0);
        assert_eq!(p.draw_box.y, 0.0);
        assert_eq!(p.draw_box.width, 50.0);
        assert_eq!(p.draw_box.height, 100.0);
    }

    #[test]
    fn fit_align_bottom_right_meet() {
        // Source 100×200, viewport 100×100.
        // Meet → scale=0.5 → draw 50×100.
        // BottomRight → offset_x = 100-50=50, offset_y = 100-100=0.
        let par = make_par_align(SvgAlign::BottomRight, SvgMeetOrSlice::Meet);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, par);
        let p = compute_raster_placement(100, 200, request).unwrap();
        assert_eq!(p.draw_box.x, 50.0);
        assert_eq!(p.draw_box.y, 0.0);
    }

    #[test]
    fn fit_align_center_slice() {
        // Source 100×200 (1:2), viewport 100×100.
        // Slice → scale = max(1.0, 0.5) = 1.0 → draw 100×200.
        // Centered → offset_x=0, offset_y=(100-200)/2=-50.
        let par = make_par_align(SvgAlign::Center, SvgMeetOrSlice::Slice);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, par);
        let p = compute_raster_placement(100, 200, request).unwrap();
        assert_eq!(p.draw_box.width, 100.0);
        assert_eq!(p.draw_box.height, 200.0);
        assert!((p.draw_box.y - (-50.0)).abs() < 1e-5, "y={}", p.draw_box.y);
    }

    #[test]
    fn fit_align_center_right_meet() {
        // Source 200×100, viewport 100×100.
        // Meet → scale = min(0.5, 1.0) = 0.5 → draw 100×50.
        // CenterRight → offset_x = 100-100=0, offset_y = (100-50)/2=25.
        let par = make_par_align(SvgAlign::CenterRight, SvgMeetOrSlice::Meet);
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, 100.0, 100.0, par);
        let p = compute_raster_placement(200, 100, request).unwrap();
        assert_eq!(p.draw_box.width, 100.0);
        assert_eq!(p.draw_box.height, 50.0);
        assert!((p.draw_box.x - 0.0).abs() < 1e-5);
        assert!((p.draw_box.y - 25.0).abs() < 1e-5);
    }

    #[test]
    fn fit_negative_viewport_dimension_returns_none() {
        let par = make_par_none();
        let request = SvgPlacementRequest::from_rect(0.0, 0.0, -10.0, 100.0, par);
        assert!(compute_raster_placement(100, 100, request).is_none());
    }
}
