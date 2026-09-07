use unicode_properties::{EmojiStatus, UnicodeEmoji};

/// Unicode emoji-presentation semantics shared by font discovery and fallback.
pub(crate) struct EmojiPresentation;

impl EmojiPresentation {
    /// Return whether `text` asks for emoji presentation.
    ///
    /// Default-emoji scalars need no selector. Text-default emoji scalars need
    /// VS16, while keycap sequences admit their optional selector.
    pub(crate) fn appears_in(text: &str) -> bool {
        let mut preceding = None;

        for character in text.chars() {
            if Self::is_default(character)
                || (unicode_properties::emoji::is_emoji_presentation_selector(character)
                    && preceding.is_some_and(UnicodeEmoji::is_emoji_char))
                || (character == '\u{20e3}' && preceding.is_some_and(Self::is_keycap_base))
            {
                return true;
            }

            preceding = Some(character);
        }

        false
    }

    pub(crate) fn scalar_has_emoji_property(codepoint: u32) -> bool {
        char::from_u32(codepoint).is_some_and(UnicodeEmoji::is_emoji_char)
    }

    fn is_default(character: char) -> bool {
        matches!(
            character.emoji_status(),
            EmojiStatus::EmojiPresentation
                | EmojiStatus::EmojiPresentationAndModifierBase
                | EmojiStatus::EmojiPresentationAndEmojiComponent
                | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
        )
    }

    const fn is_keycap_base(character: char) -> bool {
        matches!(character, '#' | '*' | '0'..='9')
    }
}

#[cfg(test)]
mod tests {
    use super::EmojiPresentation;

    #[test]
    fn recognizes_default_selector_and_keycap_presentations() {
        assert!(EmojiPresentation::appears_in("🀄"));
        assert!(EmojiPresentation::appears_in("©️"));
        assert!(EmojiPresentation::appears_in("1\u{20e3}"));
    }

    #[test]
    fn text_default_symbols_and_plain_numbers_stay_text() {
        assert!(!EmojiPresentation::appears_in("©"));
        assert!(!EmojiPresentation::appears_in("Invoice 123"));
    }
}
