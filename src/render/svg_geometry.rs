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
