//! Unicode `Default_Ignorable_Code_Point` membership.

/// Whether `ch` is a Unicode default-ignorable code point (UAX #44,
/// `DerivedCoreProperties.txt`): an invisible formatting character such as
/// ZERO WIDTH SPACE, the joiners, the bidi controls, variation selectors, or
/// tag characters, which renders nothing on its own.
///
/// CSS Text 3 §8.2 requires letter-spacing to be added as if such characters
/// did not exist in the document, and browsers skip them when spacing glyphs.
pub(crate) fn is_default_ignorable(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x00AD
            | 0x034F
            | 0x061C
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF0..=0xFFF8
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

#[cfg(test)]
mod tests {
    use super::is_default_ignorable;

    #[test]
    fn zero_width_formatting_characters_are_ignorable() {
        for ch in [
            '\u{200B}', '\u{200C}', '\u{200D}', '\u{00AD}', '\u{FEFF}', '\u{FE0F}', '\u{2066}',
        ] {
            assert!(is_default_ignorable(ch), "U+{:04X}", u32::from(ch));
        }
    }

    #[test]
    fn visible_and_combining_characters_are_not_ignorable() {
        for ch in ['A', ' ', '\u{00A0}', '\u{0301}', '\u{4E2D}', '\u{1F600}'] {
            assert!(!is_default_ignorable(ch), "U+{:04X}", u32::from(ch));
        }
    }
}
