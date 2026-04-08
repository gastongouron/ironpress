//! SVG parser — converts DOM SVG elements into an SvgTree for PDF rendering.

mod length;
mod model;
mod node;
mod path;
mod style;
mod transform;

#[cfg(test)]
mod test_support;

pub(crate) use length::{parse_absolute_length, parse_length, parse_viewbox};
pub use model::{PathCommand, SvgNode, SvgTransform, SvgTree, ViewBox};
#[allow(unused_imports)]
pub use node::parse_svg_from_element;
pub(crate) use node::parse_svg_from_element_with_viewport;
#[allow(unused_imports)]
pub use path::{parse_path_data, parse_points};
pub use style::{SvgPaint, SvgStyle};

#[cfg(test)]
mod node_tests;
#[cfg(test)]
mod path_tests;
