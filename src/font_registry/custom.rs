use std::collections::HashMap;

use crate::parser::ttf::{TtfFont, parse_ttf};

/// A normalized CSS family name for one caller-provided face.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CustomFontFamily(String);

impl CustomFontFamily {
    fn from_css_name(name: &str) -> Self {
        Self(name.to_ascii_lowercase())
    }
}

/// Parsed custom fonts owned by one converter.
#[derive(Debug, Clone, Default)]
pub(crate) struct CustomFontCatalog {
    fonts: HashMap<CustomFontFamily, TtfFont>,
}

impl CustomFontCatalog {
    /// Replace one family after parsing its bytes at the registration boundary.
    /// Invalid replacement bytes leave that family unregistered.
    pub(crate) fn replace(&mut self, name: &str, data: Vec<u8>) {
        let family = CustomFontFamily::from_css_name(name);
        self.fonts.remove(&family);
        if let Ok(font) = parse_ttf(data) {
            self.fonts.insert(family, font);
        }
    }

    #[cfg(test)]
    pub(crate) fn get(&self, name: &str) -> Option<&TtfFont> {
        self.fonts.get(&CustomFontFamily::from_css_name(name))
    }

    pub(crate) fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont)) {
        for (family, font) in &self.fonts {
            visitor(&family.0, font);
        }
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, name: &str) -> bool {
        self.fonts
            .contains_key(&CustomFontFamily::from_css_name(name))
    }
}
