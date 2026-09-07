mod custom;

use std::collections::HashMap;

use crate::font_pack::FontCatalog;
use crate::parser::ttf::TtfFont;

pub(crate) use custom::CustomFontCatalog;

/// Read-only font capabilities shared by layout, shaping, and rendering.
pub(crate) trait FontRegistry {
    fn get(&self, name: &str) -> Option<&TtfFont>;

    fn get_key_value(&self, name: &str) -> Option<(&str, &TtfFont)>;

    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont));

    fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

impl FontRegistry for HashMap<String, TtfFont> {
    fn get(&self, name: &str) -> Option<&TtfFont> {
        HashMap::get(self, name)
    }

    fn get_key_value(&self, name: &str) -> Option<(&str, &TtfFont)> {
        HashMap::get_key_value(self, name).map(|(key, font)| (key.as_str(), font))
    }

    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont)) {
        for (name, font) in self {
            visitor(name, font);
        }
    }
}

/// A font inserted into the owned overlay. Its identifier stays private so an
/// alias can only refer to a face already owned by this registry.
#[derive(Clone, Copy, PartialEq, Eq)]
struct OwnedFontId(usize);

/// Names and programs owned only for the lifetime of one conversion.
///
/// Replacing the last name for a program drops it immediately and reuses its
/// vacant slot. Aliases keep their shared program alive without reference
/// counting.
#[derive(Default)]
struct OwnedFontOverlay {
    names: HashMap<String, OwnedFontId>,
    programs: Vec<Option<TtfFont>>,
}

impl OwnedFontOverlay {
    fn replace(&mut self, name: String, font: TtfFont) -> OwnedFontId {
        if let Some(displaced) = self.names.remove(&name) {
            self.release_if_unbound(displaced);
        }
        let id = self.store(font);
        self.names.insert(name, id);
        id
    }

    fn bind_if_absent(&mut self, name: String, id: OwnedFontId) {
        self.names.entry(name).or_insert(id);
    }

    fn store(&mut self, font: TtfFont) -> OwnedFontId {
        if let Some((index, slot)) = self
            .programs
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(font);
            return OwnedFontId(index);
        }

        let id = OwnedFontId(self.programs.len());
        self.programs.push(Some(font));
        id
    }

    fn release_if_unbound(&mut self, id: OwnedFontId) {
        if self.names.values().all(|bound| *bound != id)
            && let Some(slot) = self.programs.get_mut(id.0)
        {
            *slot = None;
        }
    }

    fn get(&self, name: &str) -> Option<&TtfFont> {
        self.names
            .get(name)
            .and_then(|id| self.programs.get(id.0))
            .and_then(Option::as_ref)
    }

    fn get_key_value(&self, name: &str) -> Option<(&str, &TtfFont)> {
        let (stored, id) = self.names.get_key_value(name)?;
        let font = self.programs.get(id.0)?.as_ref()?;
        Some((stored.as_str(), font))
    }

    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont)) {
        for (name, id) in &self.names {
            if let Some(font) = self.programs.get(id.0).and_then(Option::as_ref) {
                visitor(name, font);
            }
        }
    }

    fn contains_name(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    #[cfg(test)]
    fn retained_program_count(&self) -> usize {
        self.programs.iter().flatten().count()
    }

    #[cfg(test)]
    fn storage_slot_count(&self) -> usize {
        self.programs.len()
    }
}

/// Fonts visible during one conversion.
///
/// API registrations and font packs stay owned by the converter and are
/// borrowed here alongside immutable defaults. Document and dynamically
/// resolved host faces live in the owned overlay.
pub(crate) struct ConversionFontRegistry<'a> {
    borrowed: HashMap<String, &'a TtfFont>,
    owned: OwnedFontOverlay,
}

impl<'a> ConversionFontRegistry<'a> {
    pub(crate) fn new(custom: &'a CustomFontCatalog, packs: &'a FontCatalog) -> Self {
        let mut borrowed = HashMap::new();
        custom.visit(&mut |name, font| {
            borrowed.insert(name.to_string(), font);
        });
        packs.visit(&mut |name, font| {
            borrowed.insert(name.to_string(), font);
        });
        Self {
            borrowed,
            owned: OwnedFontOverlay::default(),
        }
    }

    /// Insert an owned face, replacing the visible binding for `name`.
    pub(crate) fn insert(&mut self, name: String, font: TtfFont) {
        self.owned.replace(name, font);
    }

    /// Insert one owned face under `name`, then bind `alias` to the same face
    /// unless an earlier layer already defines the alias.
    pub(crate) fn insert_with_alias_if_absent(
        &mut self,
        name: String,
        alias: String,
        font: TtfFont,
    ) {
        let id = self.owned.replace(name, font);
        if !self.contains_key(&alias) {
            self.owned.bind_if_absent(alias, id);
        }
    }

    /// Insert a conversion-owned face unless any layer already defines `name`.
    pub(crate) fn insert_if_absent(&mut self, name: String, font: TtfFont) {
        if !self.contains_key(&name) {
            self.insert(name, font);
        }
    }

    /// Borrow a process-owned parsed face unless any higher-priority layer
    /// already defines `name`.
    pub(crate) fn borrow_if_absent(&mut self, name: String, font: &'a TtfFont) {
        if !self.contains_key(&name) {
            self.borrowed.insert(name, font);
        }
    }

    pub(crate) fn contains_key(&self, name: &str) -> bool {
        FontRegistry::contains_key(self, name)
    }

    #[cfg(test)]
    fn retained_owned_font_count(&self) -> usize {
        self.owned.retained_program_count()
    }

    #[cfg(test)]
    fn owned_storage_slot_count(&self) -> usize {
        self.owned.storage_slot_count()
    }
}

impl FontRegistry for ConversionFontRegistry<'_> {
    fn get(&self, name: &str) -> Option<&TtfFont> {
        if self.owned.is_empty() {
            return self.borrowed.get(name).copied();
        }
        if let Some(font) = self.owned.get(name) {
            return Some(font);
        }
        self.borrowed.get(name).copied()
    }

    fn get_key_value(&self, name: &str) -> Option<(&str, &TtfFont)> {
        if self.owned.is_empty() {
            return self
                .borrowed
                .get_key_value(name)
                .map(|(stored, font)| (stored.as_str(), *font));
        }
        if let Some(found) = self.owned.get_key_value(name) {
            return Some(found);
        }
        if let Some((stored, font)) = self.borrowed.get_key_value(name) {
            return Some((stored, *font));
        }
        None
    }

    fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont)) {
        self.owned.visit(visitor);
        for (name, font) in &self.borrowed {
            if !self.owned.contains_name(name) {
                visitor(name, font);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_pack::{FontPack, FontPackKind};
    use crate::parser::ttf::parse_ttf;

    const LIBERATION_SANS: &[u8] = include_bytes!("../../assets/LiberationSans-Regular.ttf");

    #[test]
    fn conversion_borrows_the_converter_owned_custom_font() {
        let mut custom = CustomFontCatalog::default();
        custom.replace("Bench Font", LIBERATION_SANS.to_vec());
        let packs = FontCatalog::default();
        let registry = ConversionFontRegistry::new(&custom, &packs);

        let registered = custom.get("bench font").expect("registered custom font");
        let visible = registry.get("bench font").expect("visible custom font");

        assert!(std::ptr::eq(registered, visible));
    }

    #[test]
    fn conversion_borrows_the_converter_owned_font_pack() {
        let mut custom = CustomFontCatalog::default();
        custom.replace(
            crate::font_pack::CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            include_bytes!("../../assets/NotoSans-Regular.ttf").to_vec(),
        );
        let mut packs = FontCatalog::default();
        packs.install(
            FontPack::parse(FontPackKind::CjkSimplifiedChinese, LIBERATION_SANS.to_vec())
                .expect("valid font pack"),
        );
        let registry = ConversionFontRegistry::new(&custom, &packs);

        let registered = packs
            .get(crate::font_pack::CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY)
            .expect("registered pack");
        let visible = registry
            .get(crate::font_pack::CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY)
            .expect("visible pack");

        assert!(std::ptr::eq(registered, visible));
    }

    #[test]
    fn document_font_overrides_a_registration_and_aliases_without_copying() {
        let mut custom = CustomFontCatalog::default();
        custom.replace("Document Font", LIBERATION_SANS.to_vec());
        let packs = FontCatalog::default();
        let mut registry = ConversionFontRegistry::new(&custom, &packs);
        let document = parse_ttf(LIBERATION_SANS.to_vec()).expect("valid document font");

        registry.insert_with_alias_if_absent(
            "document font".to_string(),
            "document font alias".to_string(),
            document,
        );

        let document = registry.get("document font").expect("document face");
        let alias = registry
            .get("document font alias")
            .expect("document face alias");
        assert!(std::ptr::eq(document, alias));
        assert!(!std::ptr::eq(
            custom.get("document font").expect("registered custom font"),
            alias
        ));
    }

    #[test]
    fn system_discovery_does_not_replace_an_api_registration() {
        let mut custom = CustomFontCatalog::default();
        custom.replace("Registered Font", LIBERATION_SANS.to_vec());
        let packs = FontCatalog::default();
        let mut registry = ConversionFontRegistry::new(&custom, &packs);
        let discovered = parse_ttf(include_bytes!("../../assets/NotoSans-Regular.ttf").to_vec())
            .expect("valid discovered font");

        registry.insert_if_absent("registered font".to_string(), discovered);

        let registered = custom
            .get("registered font")
            .expect("registered custom font");
        let visible = registry
            .get("registered font")
            .expect("visible custom font");
        assert!(std::ptr::eq(registered, visible));
    }

    #[test]
    fn replacing_one_owned_name_keeps_only_its_live_program() {
        let custom = CustomFontCatalog::default();
        let packs = FontCatalog::default();
        let mut registry = ConversionFontRegistry::new(&custom, &packs);

        for _ in 0..128 {
            let font = parse_ttf(LIBERATION_SANS.to_vec()).expect("valid replacement font");
            registry.insert("document font".to_string(), font);
        }

        assert_eq!(registry.retained_owned_font_count(), 1);
        assert_eq!(registry.owned_storage_slot_count(), 1);
    }

    #[test]
    fn replacing_a_primary_name_keeps_its_existing_alias_alive() {
        let custom = CustomFontCatalog::default();
        let packs = FontCatalog::default();
        let mut registry = ConversionFontRegistry::new(&custom, &packs);
        let first = parse_ttf(LIBERATION_SANS.to_vec()).expect("valid aliased font");
        registry.insert_with_alias_if_absent(
            "document font".to_string(),
            "document alias".to_string(),
            first,
        );
        let alias_name = registry
            .get("document alias")
            .expect("visible alias")
            .font_name
            .clone();

        let replacement = parse_ttf(include_bytes!("../../assets/NotoSans-Regular.ttf").to_vec())
            .expect("valid replacement font");
        registry.insert("document font".to_string(), replacement);

        assert_eq!(
            registry
                .get("document alias")
                .expect("retained alias")
                .font_name,
            alias_name
        );
        assert_ne!(
            registry
                .get("document font")
                .expect("replacement")
                .font_name,
            alias_name
        );
        assert_eq!(registry.retained_owned_font_count(), 2);
        assert_eq!(registry.owned_storage_slot_count(), 2);
    }
}
