//! Post-compositing effects shared by every box-like layout node.

use crate::layout::engine::StackingContext;
use crate::style::computed::{
    BlendMode, ClipPath, CssAffineMatrix, CssVector, Isolation, MaskMode, MaskSource, Transform,
    TransformBox, TransformOrigin,
};
use crate::types::{EdgeSizes, Point, Rect};

use super::super::Stacking;

/// Clipping and masking sources that apply to one paint group.
#[derive(Debug, Clone, Default)]
pub(crate) struct Masking {
    pub(crate) clip_path: Option<ClipPath>,
    pub(crate) image: Option<MaskSource>,
    pub(crate) mode: MaskMode,
}

impl Masking {
    fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            clip_path: style.clip_path.clone(),
            image: style.mask_image.clone(),
            mode: style.mask_mode,
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        self.clip_path.is_none() && self.image.is_none()
    }
}

/// Effects applied to a box after its contents have been composited.
///
/// The raster replacing a filtered subtree is only its source graphic.
/// Clipping, masking, opacity, and blending remain on this group so their
/// ordering cannot drift between ordinary, replaced, and cell paint paths.
#[derive(Debug, Clone)]
pub(crate) struct GroupEffects {
    pub(crate) opacity: f32,
    pub(crate) mix_blend_mode: BlendMode,
    pub(crate) isolation: Isolation,
    pub(crate) stacking_context: StackingContext,
    pub(crate) masking: Masking,
}

impl Default for GroupEffects {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            mix_blend_mode: BlendMode::Normal,
            isolation: Isolation::Auto,
            stacking_context: StackingContext::None,
            masking: Masking::default(),
        }
    }
}

impl GroupEffects {
    fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            opacity: style.opacity,
            mix_blend_mode: style.mix_blend_mode,
            isolation: style.isolation,
            stacking_context: (&style.filter).into(),
            masking: Masking::from_style(style),
        }
    }

    /// Whether painting this source needs no post-compositing wrapper.
    pub(crate) fn is_identity(&self) -> bool {
        self.opacity >= 1.0
            && self.mix_blend_mode == BlendMode::Normal
            && !self.isolation.isolates()
            && !self.stacking_context.establishes()
            && self.masking.is_none()
    }

    pub(crate) fn needs_source_isolation(&self) -> bool {
        self.opacity < 1.0
            || self.mix_blend_mode != BlendMode::Normal
            || self.isolation.isolates()
            || self.stacking_context.needs_source_isolation()
    }
}

/// One CSS transform and the reference-box policy used to resolve it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BoxTransform {
    pub(crate) value: Option<Transform>,
    pub(crate) origin: TransformOrigin,
    pub(crate) reference_box: TransformBox,
    /// Parent perspective establishes a spatial stacking group even when its
    /// projection is resolved on transformed descendants.
    pub(crate) perspective: Option<f32>,
}

impl BoxTransform {
    pub(crate) const fn establishes_stacking_context(self) -> bool {
        self.value.is_some() || self.perspective.is_some()
    }

    /// Resolve percentages and the transform origin against one fragment.
    pub(crate) fn resolve(
        self,
        border_box: Rect,
        content_insets: EdgeSizes,
    ) -> Option<CssAffineMatrix> {
        let transform = self.value?;
        let reference_box = match self.reference_box {
            TransformBox::Content | TransformBox::Fill => border_box.inset(content_insets),
            TransformBox::Border | TransformBox::Stroke | TransformBox::View => border_box,
        };
        let (origin_x, origin_y) = self
            .origin
            .resolve(reference_box.size.width, reference_box.size.height);
        let pivot = Point::new(
            reference_box.origin.x + origin_x,
            reference_box.origin.y + origin_y,
        );
        Some(
            transform
                .to_css_matrix(CssVector::new(
                    f64::from(reference_box.size.width),
                    f64::from(reference_box.size.height),
                ))
                .around(pivot),
        )
    }
}

/// The complete graphical group applied to a box and all descendants.
#[derive(Debug, Clone, Default)]
pub(crate) struct PaintGroup {
    /// The element this box belongs to, for the accessibility structure tree.
    #[allow(dead_code)] // read by the renderer when marking content
    pub(crate) owner: crate::accessibility::StructureRef,
    pub(crate) stacking: Stacking,
    pub(crate) transform: BoxTransform,
    pub(crate) effects: GroupEffects,
    /// A filter retained until fragmentation establishes its device-space
    /// anchor.
    pub(crate) filter: Option<crate::layout::filter::ResolvedFilter>,
}

impl PaintGroup {
    pub(crate) fn from_style(style: &crate::style::computed::ComputedStyle) -> Self {
        Self {
            owner: style.structure,
            stacking: Stacking::from_style(style),
            transform: BoxTransform {
                value: style.transform,
                origin: style.transform_origin,
                reference_box: style.transform_box,
                perspective: style.perspective,
            },
            effects: GroupEffects::from_style(style),
            filter: None,
        }
    }

    pub(crate) fn is_identity(&self) -> bool {
        !self.transform.establishes_stacking_context() && self.effects.is_identity()
    }

    pub(crate) fn establishes_stacking_context(&self) -> bool {
        self.transform.establishes_stacking_context()
            || self.effects.opacity < 1.0
            || self.effects.mix_blend_mode != BlendMode::Normal
            || self.effects.isolation.isolates()
            || self.effects.stacking_context.establishes()
            || !self.effects.masking.is_none()
    }

    pub(crate) fn with_materialized_filter(mut self) -> Self {
        self.effects.stacking_context = self.effects.stacking_context.materialized();
        self.filter = None;
        self
    }
}

/// Ownership of a filter retained until fragmentation produces its concrete
/// box fragments.
pub(crate) trait FilterHolder {
    fn filter_slot_mut(&mut self) -> &mut Option<crate::layout::filter::ResolvedFilter>;

    fn take_filter(&mut self) -> Option<crate::layout::filter::ResolvedFilter> {
        self.filter_slot_mut().take()
    }
}

impl FilterHolder for PaintGroup {
    fn filter_slot_mut(&mut self) -> &mut Option<crate::layout::filter::ResolvedFilter> {
        &mut self.filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::computed::PercentageAxes;
    use crate::types::Size;

    #[test]
    fn percentage_translation_uses_the_selected_content_box() {
        let transform = BoxTransform {
            value: Some(Transform::Translate {
                offset: CssVector::new(100.0, 0.0),
                percentages: PercentageAxes::new(true, false),
            }),
            reference_box: TransformBox::Content,
            ..Default::default()
        };

        let resolved = transform
            .resolve(
                Rect::new(Point::new(10.0, 20.0), Size::new(20.0, 10.0)),
                EdgeSizes::uniform(2.0),
            )
            .expect("the test transform is present");

        assert_eq!(
            resolved.transform_point(Point::ORIGIN),
            Point::new(16.0, 0.0)
        );
    }
}
