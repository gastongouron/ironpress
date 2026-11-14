use crate::render::pdf::border_images::BorderImageShadings;

include!("foundation.rs");
mod accessibility;
mod flex_cells;
mod rich_page_margins;
mod stacking;
include!("conic_and_effects.rs");
include!("document_basics.rs");
include!("images.rs");
include!("paint_and_transforms.rs");
include!("gradient_masks.rs");
include!("text_encoding.rs");
include!("backgrounds.rs");
include!("layout.rs");
include!("fonts_and_metrics.rs");
include!("nested_boxes.rs");
include!("nested_tables.rs");
include!("math_and_writer.rs");
