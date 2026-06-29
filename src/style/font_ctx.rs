//! Thread-local access to the active document's loaded fonts during style
//! resolution.
//!
//! The CSS font-relative `ex` and `ch` units (css-values-4 §6.1.1) resolve
//! against the **resolved font's** metrics — `ex` to the font's x-height and
//! `ch` to the advance of the `'0'` glyph. Style resolution (`apply_style_map`)
//! runs deep inside the layout/render call tree and is not threaded the parsed
//! fonts map, so this module exposes the active fonts through a thread-local
//! pointer installed for the duration of a single (synchronous, single-threaded)
//! layout or render pass via the RAII [`FontCtxGuard`].
//!
//! Safety: the pointer is only ever read while the guard that installed it is
//! alive, and each `convert` runs layout and render synchronously on one thread
//! (no internal parallelism in the layout/render code), so the borrow the guard
//! captures outlives every read.

use std::cell::Cell;
use std::collections::HashMap;

use crate::parser::ttf::TtfFont;
use crate::style::computed::{ComputedStyle, FontStack, FontStyle, FontWeight};

thread_local! {
    /// Raw pointer to the active fonts map, valid only while a [`FontCtxGuard`]
    /// is alive. `None`/null when no pass is active (e.g. unit tests).
    static ACTIVE_FONTS: Cell<*const HashMap<String, TtfFont>> = const { Cell::new(std::ptr::null()) };
}

/// RAII guard that installs `fonts` as the active font context and restores the
/// previous context on drop. Hold it for the whole layout/render pass.
pub struct FontCtxGuard {
    previous: *const HashMap<String, TtfFont>,
}

impl FontCtxGuard {
    /// Install `fonts` as the active context. The borrow must outlive the guard.
    pub fn new(fonts: &HashMap<String, TtfFont>) -> Self {
        let ptr = fonts as *const HashMap<String, TtfFont>;
        let previous = ACTIVE_FONTS.with(|c| c.replace(ptr));
        FontCtxGuard { previous }
    }
}

impl Drop for FontCtxGuard {
    fn drop(&mut self) {
        ACTIVE_FONTS.with(|c| c.set(self.previous));
    }
}

/// Resolve the x-height of the font selected by `stack`/`bold`/`italic`, as a
/// fraction of the em. Returns `None` when no font context is active or the
/// stack resolves to no loaded font (caller falls back to the 0.5em default).
pub fn resolved_x_height_ratio(stack: &FontStack, bold: bool, italic: bool) -> Option<f32> {
    with_resolved_font(stack, bold, italic, TtfFont::x_height_ratio)
}

/// Resolve the `ch` advance (the `'0'` glyph) of the font selected by
/// `stack`/`bold`/`italic`, as a fraction of the em.
pub fn resolved_ch_ratio(stack: &FontStack, bold: bool, italic: bool) -> Option<f32> {
    with_resolved_font(stack, bold, italic, TtfFont::ch_ratio)
}

/// x-height ratio for the font a [`ComputedStyle`] already selects. Used when
/// resolving `ex` on the `font-size` property, where the unit refers to the
/// **parent** element's font (css-values-4 §6.1.1: the value is computed before
/// the new font-size establishes a new font).
pub fn style_x_height_ratio(style: &ComputedStyle) -> Option<f32> {
    resolved_x_height_ratio(
        &style.font_stack,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
    )
}

/// `ch` ratio for the font a [`ComputedStyle`] already selects (parent font for
/// the `font-size` property; see [`style_x_height_ratio`]).
pub fn style_ch_ratio(style: &ComputedStyle) -> Option<f32> {
    resolved_ch_ratio(
        &style.font_stack,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
    )
}

/// X-height ratio for `font-size-adjust`.
///
/// Chromium applies the adjustment from the hinted/scaled font metrics. For
/// bundled fonts whose OS/2 table has no `sxHeight`, our raw outline fallback is
/// slightly below Chrome because it is unhinted. Round the resolved x-height to
/// CSS pixels at the current computed font size before deriving the ratio; this
/// leaves `ex` unit resolution on the unhinted metric path above.
pub fn style_font_size_adjust_x_height_ratio(style: &ComputedStyle) -> Option<f32> {
    with_resolved_font(
        &style.font_stack,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
        |font| {
            let ratio = font.x_height_ratio();
            let font_size_px = style.font_size / 0.75;
            if font_size_px.is_finite() && font_size_px > 0.0 {
                let hinted_px = (ratio * font_size_px).round();
                if hinted_px > 0.0 {
                    return hinted_px / font_size_px;
                }
            }
            ratio
        },
    )
}

fn with_resolved_font(
    stack: &FontStack,
    bold: bool,
    italic: bool,
    extract: impl Fn(&TtfFont) -> f32,
) -> Option<f32> {
    let ptr = ACTIVE_FONTS.with(|c| c.get());
    if ptr.is_null() {
        return None;
    }
    // SAFETY: a live `FontCtxGuard` installed this pointer from a borrow that
    // outlives every read on this synchronous single-threaded pass.
    let fonts = unsafe { &*ptr };
    let resolved = crate::system_fonts::resolve_font_family(stack, fonts, bold, italic);
    let name = match &resolved {
        crate::style::computed::FontFamily::Custom(n) => n.as_str(),
        // Standard PDF fonts have no embedded TTF metrics here; fall back.
        _ => return None,
    };
    crate::system_fonts::find_font(fonts, name, bold, italic).map(|(_, font)| extract(font))
}
