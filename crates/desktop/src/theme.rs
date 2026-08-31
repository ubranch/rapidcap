//! Design tokens.
//!
//! Every colour, size and radius the UI uses lives here. Nothing outside this
//! module writes a literal `rgb()` / `px()` for a themed value.
//!
//! Every element carries the same [`BORDER`]-pixel [`border_card`] on all four
//! sides. There used to be a 1px top-only highlight instead, which read as an
//! element with a border along one edge and nothing along the other three.
//!
//! Dark only. The panel floats over arbitrary desktop content, the overlay and
//! HUD are dark by necessity, and the recessed shadow does not invert.

// The palette is defined as a whole; surfaces adopt it phase by phase. Warning
// on a token the overlay has not reached yet would only invite deleting it and
// re-adding it later.
#![allow(dead_code)]

use gpui::{BoxShadow, Hsla, Pixels, point, px, rgb, rgba};

// ---------------------------------------------------------------- surfaces

/// Panel body, below the titlebar.
pub fn bg_body() -> Hsla {
    rgb(0x111111).into()
}

/// Titlebar strip. Deliberately *lighter* than the body — the tone step is the
/// separator, there is no border.
pub fn bg_titlebar() -> Hsla {
    rgb(0x1c1c1c).into()
}

/// Cards, chips, menus, the status well.
pub fn bg_card() -> Hsla {
    rgb(0x1c1c1c).into()
}

/// Hover fill for cards, chips and rows.
pub fn bg_hover() -> Hsla {
    rgb(0x242424).into()
}

/// Hover fill for titlebar buttons, which sit on the lighter strip.
pub fn bg_titlebar_hover() -> Hsla {
    rgb(0x2a2a2a).into()
}

/// Segmented control track, header badge.
pub fn bg_track() -> Hsla {
    rgb(0x282828).into()
}

// ---------------------------------------------------------------- borders

/// Outer border on every element. [`BORDER`] pixels, all four sides.
///
/// Was `0x202020`, which is 1.1:1 on [`bg_card`] - a border nobody could see.
/// This reads on both the card and the body fill without becoming a grid.
pub fn border_card() -> Hsla {
    rgb(0x333333).into()
}

/// The HUD's hairline, between the status pill and the transport buttons. Was
/// the split-button divider too, until the split button came off the mode cards.
pub fn border_divider() -> Hsla {
    rgb(0x444444).into()
}

// ---------------------------------------------------------------- text

/// Wordmark, logo mark, hovered icons.
pub fn text_primary() -> Hsla {
    rgb(0xfcfcfc).into()
}

/// Card and row labels.
pub fn text_label() -> Hsla {
    rgb(0xdddddd).into()
}

/// Header badge.
pub fn text_badge() -> Hsla {
    rgb(0xe2e2e2).into()
}

/// Pill text. 4.6:1 on [`bg_pill_off`] — `0x7a7a7a` measured 2.4:1 and failed AA.
pub fn text_pill() -> Hsla {
    rgb(0xa9a9a9).into()
}

/// Titlebar icons and titles at rest. 3.6:1 on the titlebar strip.
pub fn text_muted() -> Hsla {
    rgb(0x8a8a8a).into()
}

/// A toggle chip's label while the toggle is off. Between [`text_label`] and
/// [`text_muted`]: still readable, but visibly not the on state.
pub fn text_off() -> Hsla {
    rgb(0xa9a9a9).into()
}

/// Row icons at rest. Never used for text.
pub fn text_dim() -> Hsla {
    rgb(0x7a7a7a).into()
}

// ---------------------------------------------------------------- state

/// Off-state pill background.
pub fn bg_pill_off() -> Hsla {
    rgb(0x3a3a3a).into()
}

/// Selection, focus, active segment.
pub fn accent() -> Hsla {
    rgb(0x3478f6).into()
}

/// Accent at 16% — selected card fill.
pub fn accent_fill() -> Hsla {
    rgba(0x3478f629).into()
}

/// Accent at 20% — enabled pill fill.
pub fn accent_pill() -> Hsla {
    rgba(0x3478f633).into()
}

/// Accent text on an accent fill.
pub fn accent_text() -> Hsla {
    rgb(0x8fb6fb).into()
}

/// A capture is running. The recording dot — nothing else.
pub fn rec() -> Hsla {
    rgb(0xff2d2d).into()
}

/// Recording pill fill.
pub fn rec_pill() -> Hsla {
    rgba(0xff2d2d2e).into()
}

/// Recording pill text.
pub fn rec_text() -> Hsla {
    rgb(0xff8a8a).into()
}

/// Destructive buttons: Stop, Cancel, Close hover.
pub fn danger() -> Hsla {
    rgb(0xc92a2a).into()
}

/// Hovered destructive button.
pub fn danger_hover() -> Hsla {
    rgb(0xd93b3b).into()
}

/// Failure. Amber rather than red, so an error never reads as a live capture.
pub fn warn() -> Hsla {
    rgb(0xe8a33d).into()
}

/// Error bar background.
pub fn warn_fill() -> Hsla {
    rgba(0xe8a33d1f).into()
}

/// Error message text. 8.1:1 on [`warn_fill`].
pub fn warn_text() -> Hsla {
    rgb(0xf0c07a).into()
}

// ---------------------------------------------------------------- overlay

/// Full-screen dim while selecting.
pub fn overlay_scrim() -> Hsla {
    rgba(0x0000008c).into()
}

/// Inside the drag rect.
pub fn overlay_drag_fill() -> Hsla {
    rgba(0x0000002e).into()
}

/// Tint over a hovered window.
pub fn overlay_window_fill() -> Hsla {
    rgba(0x3478f624).into()
}

/// Floating size badge and hint pill. The card tone at 92%.
pub fn overlay_float() -> Hsla {
    rgba(0x1c1c1ceb).into()
}

/// HUD background. The card tone at 97%.
pub fn hud_bg() -> Hsla {
    rgba(0x1c1c1cf7).into()
}

// ---------------------------------------------------------------- metrics

/// The one spacing value. Grid gaps, row gaps, group gaps — no exceptions.
pub const GAP: f32 = 9.0;
/// Panel body padding.
pub const PAD: f32 = 12.0;
/// The one rectangular radius.
pub const RADIUS: f32 = 8.0;
/// Pills, chips, tracks.
pub const RADIUS_PILL: f32 = 999.0;

/// Fixed panel width. Long names truncate; this never moves.
pub const PANEL_W: f32 = 400.0;
/// Panel height. Fixed, like the width: the error bar replaces the footer row
/// rather than adding one, so no state is taller than any other. 258 of body
/// under a [`TITLEBAR_H`] bar.
pub const PANEL_H: f32 = 288.0;

/// The titlebar, and the panel it sits on.
///
/// Every number in this group is a *design* pixel and reaches the screen through
/// [`u`], because the bar is the one part of the window that has to keep step
/// with the native titlebars around it — see [`u`] for why.
///
/// The heights are cumcord's, measured off its source rather than its pixels:
/// `TITLEBAR_HEIGHT` 30, window buttons 34 wide by the full bar height, 13px
/// glyphs, a 9px leading inset and 8px between everything. Matching it by eye
/// is what produced the 22px bar this replaces.
pub const TITLEBAR_H: f32 = 30.0;
/// Minimize and close. Wider than they are tall, which is what a titlebar
/// control is; full bar height so the screen corner stays clickable.
pub const WIN_BTN_W: f32 = 34.0;
/// Mark, wordmark and window-control glyphs. One size, the whole bar.
pub const TITLEBAR_GLYPH: f32 = 13.0;
/// Leading inset before the mark.
pub const TITLEBAR_LEADING: f32 = 9.0;
/// Between the mark and the wordmark, and between the two window controls.
pub const TITLEBAR_GAP: f32 = 8.0;

/// A design measurement, in the unit the interface is drawn in.
///
/// Every number in this module is a *design* pixel and reaches the screen
/// through here. Windows applies Settings > Accessibility > Text size to its own
/// chrome and to nothing an app draws for itself, so an interface authored in
/// raw pixels stays put while every native window on the machine grows - at 130%
/// this panel was drawing a titlebar close to half the height of the one on the
/// window beside it. One multiply here scales the whole panel with the system
/// instead.
///
/// This is *not* the display's DPI scale. GPUI already applies that underneath,
/// which is why the unit stays logical pixels rather than physical ones.
pub fn u(value: f32) -> Pixels {
    px(value * crate::platform::text_scale())
}

/// [`u`], for the callers that need the number rather than the type: window
/// bounds and the Win32 placement behind them.
pub fn scaled(value: f32) -> f32 {
    value * crate::platform::text_scale()
}

pub const CARD_H: f32 = 64.0;
pub const CHIP_H: f32 = 36.0;
pub const CHIP_MAX_W: f32 = 152.0;
pub const STATUS_H: f32 = 28.0;
pub const SEGMENT: f32 = 32.0;
pub const MARK: f32 = 34.0;
/// Ring inside the logo mark: 20px circle, 3px cut.
pub const MARK_RING: f32 = 20.0;
pub const BADGE_H: f32 = 22.0;
/// Segmented control: 4px track padding around 32px slots.
pub const SEG_PAD: f32 = 4.0;
pub const SEG_INFO: f32 = 15.0;
/// Gap between the header row and the grid, on top of the 9px column gap.
pub const HEADER_MB: f32 = 3.0;

/// The smallest target in the body of the panel that is not a segment slot.
/// The titlebar's window controls sit under it at 34 x 30 - what a titlebar
/// control is - because the screen corner makes close clickable regardless.
pub const TARGET_MIN: f32 = 36.0;

/// Recording HUD pill.
pub const HUD_H: f32 = 36.0;
/// Fixed, because the status text changes length on every state change and a
/// pill that sizes to its content drags the buttons sideways underneath the
/// pointer. The status column absorbs the difference instead.
pub const HUD_W: f32 = 248.0;
/// The HUD dims to this once the pointer has been away for a few seconds.
pub const HUD_IDLE_OPACITY: f32 = 0.55;
/// Overlay size badge and hint pill. One height, so they share a baseline.
pub const FLOAT_H: f32 = 30.0;
/// Corner grip on the drag rect.
pub const HANDLE: f32 = 7.0;
/// Recording frame thickness, in physical pixels.
pub const FRAME: u32 = 3;
/// Element border, in logical pixels. One width, every element.
pub const BORDER: f32 = 2.0;

// ---------------------------------------------------------------- depth

/// A pill's radius is clamped to half its height, whatever `RADIUS_PILL` says.
pub fn pill_radius(height: f32) -> f32 {
    height / 2.0
}

/// Recessed: the element is read-only. Cut into the panel rather than sitting
/// on it. Never combined with a border.
pub fn recessed() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(0x0000008c).into(),
        offset: point(u(0.0), u(1.0)),
        blur_radius: u(3.0),
        spread_radius: u(0.0),
        inset: true,
    }]
}

/// Floating surfaces: menus, the HUD.
pub fn floating() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(0x00000099).into(),
        offset: point(u(0.0), u(10.0)),
        blur_radius: u(28.0),
        spread_radius: u(0.0),
        inset: false,
    }]
}

// ---------------------------------------------------------------- type

pub const TEXT_WORDMARK: f32 = 22.0;
pub const TEXT_ROW: f32 = 14.0;
pub const TEXT_BODY: f32 = 13.0;
pub const TEXT_SMALL: f32 = 12.0;
pub const TEXT_MICRO: f32 = 11.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surface_is_neutral_gray() {
        // R == G == B on every surface token. A blue-gray tint here is the one
        // regression that would be invisible in review and obvious on screen.
        for (name, colour) in [
            ("bg_body", bg_body()),
            ("bg_titlebar", bg_titlebar()),
            ("bg_card", bg_card()),
            ("bg_hover", bg_hover()),
            ("bg_titlebar_hover", bg_titlebar_hover()),
            ("bg_track", bg_track()),
            ("bg_pill_off", bg_pill_off()),
            ("border_card", border_card()),
            ("border_divider", border_divider()),
            ("text_primary", text_primary()),
            ("text_label", text_label()),
            ("text_muted", text_muted()),
            ("text_off", text_off()),
            ("text_pill", text_pill()),
        ] {
            assert_eq!(
                colour.s, 0.0,
                "{name} is not neutral: saturation {}",
                colour.s
            );
        }
    }

    #[test]
    fn titlebar_is_lighter_than_body() {
        assert!(
            bg_titlebar().l > bg_body().l,
            "the titlebar must read as a step up from the body — that step is the separator"
        );
    }

    #[test]
    fn a_pill_is_capped_not_rounded() {
        // `RADIUS_PILL` is larger than any pill is tall, so a pill has to clamp
        // to half its own height or the caps stop being semicircles.
        assert_eq!(pill_radius(CHIP_H), 18.0);
        assert!(
            pill_radius(CHIP_H) > RADIUS,
            "a pill is rounder than a card"
        );
    }

    #[test]
    fn recessed_is_an_inset_shadow_with_blur() {
        let recessed = recessed();
        assert!(
            recessed[0].inset,
            "recessed must be inset or it is a drop shadow"
        );
        assert!(
            recessed[0].blur_radius > px(0.0),
            "GPUI renders nothing at blur 0 - that is why the raised edge is an element"
        );
    }
}
