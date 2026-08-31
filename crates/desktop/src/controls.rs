//! The five settings controls, and the row they sit in.
//!
//! One depth rule runs through all of them: raised means you press it,
//! recessed means you read it. A toggle that is off is a cut track, a path is
//! a cut well, and everything you can actually push has a border and sits on
//! top of the card.
//!
//! Like [`crate::menu`], nothing here owns state. Each function hands back the
//! element and the caller attaches `on_click` and passes in what is currently
//! true, which is what lets one `select` serve the frame rate, the quality and
//! the dithering without three copies of it.

// Built to the design system in one pass, adopted screen by screen - the same
// arrangement `icons` uses. Settings is the first caller and takes most of it.
#![allow(dead_code)]

use gpui::{FontWeight, Role, SharedString, Toggled, div, prelude::*};

use crate::icons::Icon;
use crate::theme;

/// A settings row. 44px, or 54 when the label carries a second line.
const ROW_H: f32 = 44.0;
const ROW_H_TALL: f32 = 54.0;
const ROW_PX: f32 = 10.0;
const ROW_RADIUS: f32 = 6.0;
const ROW_GAP: f32 = 10.0;

/// Every control that is not the label column is this tall. A row is 44 and a
/// control is 30, so the same 7px of air sits above and below all of them.
const CONTROL_H: f32 = 30.0;

const TOGGLE_W: f32 = 38.0;
const TOGGLE_H: f32 = 22.0;
const KNOB: f32 = 16.0;
/// Knob inset. The same 3px on both ends, so the travel is the track minus the
/// knob minus twice this.
const KNOB_INSET: f32 = 3.0;

const STEP_BTN: f32 = 30.0;
/// The value column in a stepper. Wide enough for `Off`, `10 s` and `100`
/// without the buttons moving between them.
const STEP_VALUE_W: f32 = 54.0;
const STEP_DISABLED: f32 = 0.35;

const ACTION_H: f32 = 24.0;
/// The `Change` button inside a path field. Not a system token: this is the
/// only raised thing in the product that sits inside a recessed well, so it
/// needs a fill lighter than the well and darker than a card.
const ACTION_BG: u32 = 0x2a2a2a;
const ACTION_BG_HOVER: u32 = 0x343434;

/// Hotkeys are read as a shape, not as words - `Ctrl + Shift + W` has to line
/// up with the row above it or the modifier column is unreadable.
const MONO: &str = "Cascadia Mono";

/// One row of a settings card.
///
/// `sub` is the consequence of the setting, never a restatement of its name,
/// and taking it here rather than letting callers build their own label column
/// is what keeps the 44/54 split from being decided by eye at each call site.
pub fn row(
    id: &'static str,
    label: impl Into<SharedString>,
    sub: Option<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let tall = sub.is_some();
    div()
        .id(id)
        .h(theme::u(if tall { ROW_H_TALL } else { ROW_H }))
        .px(theme::u(ROW_PX))
        .flex()
        .flex_none()
        .items_center()
        .gap(theme::u(ROW_GAP))
        .rounded(theme::u(ROW_RADIUS))
        .hover(|style| style.bg(theme::bg_hover()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(theme::u(theme::TEXT_BODY))
                .text_color(theme::text_label())
                .child(label.into())
                .children(sub.map(|sub| {
                    div()
                        .mt(theme::u(3.0))
                        .text_size(theme::u(theme::TEXT_MICRO))
                        .text_color(theme::text_muted())
                        .child(sub)
                })),
        )
}

/// A toggle. On is a filled accent track with a white knob; off is a cut one
/// with a grey knob, so an unset toggle reads as an empty slot rather than as
/// a dark button that might be broken.
pub fn toggle(id: &'static str, label: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    let mut track = div()
        .id(id)
        .accessibility_id(id)
        .focusable()
        .tab_stop(true)
        .role(Role::Switch)
        .aria_label(label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .aria_keyshortcuts("Space")
        .relative()
        .flex_none()
        .w(theme::u(TOGGLE_W))
        .h(theme::u(TOGGLE_H))
        .rounded(theme::u(theme::RADIUS_PILL))
        .cursor_pointer()
        // Offset 3 rather than the usual 2: the toggle is the one control with
        // no border of its own, so the ring needs the extra pixel of air to
        // stop reading as one.
        .focus_visible(|style| style.border_color(theme::accent()))
        .border_2()
        .border_color(gpui::transparent_black());
    track = if on {
        track.bg(theme::accent())
    } else {
        track.bg(theme::bg_track()).shadow(theme::recessed())
    };
    track.child(
        div()
            .absolute()
            .top(theme::u(KNOB_INSET))
            .left(theme::u(if on {
                TOGGLE_W - KNOB - KNOB_INSET
            } else {
                KNOB_INSET
            }))
            .size(theme::u(KNOB))
            .rounded(theme::u(theme::RADIUS_PILL))
            .bg(if on {
                gpui::white()
            } else {
                theme::text_muted()
            }),
    )
}

/// A select. The same pill as a footer chip, 30px instead of 36 because it
/// lives inside a row. Shows the value, never the setting's name: the row
/// already carries that.
pub fn select(
    id: &'static str,
    label: &'static str,
    value: impl Into<SharedString>,
    open: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .accessibility_id(id)
        .focusable()
        .tab_stop(true)
        .role(Role::ComboBox)
        .aria_label(label)
        .aria_expanded(open)
        .aria_keyshortcuts("Enter Space Down")
        .flex_none()
        .h(theme::u(CONTROL_H))
        .pl(theme::u(11.0))
        .pr(theme::u(6.0))
        .flex()
        .items_center()
        .gap(theme::u(5.0))
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(theme::bg_card())
        .border_1()
        .border_color(theme::border_card())
        .cursor_pointer()
        .hover(|style| style.bg(theme::bg_hover()))
        .focus_visible(|style| style.border_color(theme::accent()))
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_label())
        .child(value.into())
        .child(Icon::Chevron.element(theme::u(12.0), theme::text_muted()))
}

/// Which end of a stepper a button is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Down,
    Up,
}

/// A stepper track. Caller fills it with [`step_button`], [`step_value`],
/// [`step_button`], in that order.
///
/// For short bounded ranges only - a stepper is how you change a countdown,
/// not how you pick from thirty devices.
pub fn stepper(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .h(theme::u(CONTROL_H))
        .flex()
        .items_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .overflow_hidden()
        .bg(theme::bg_card())
        .border_1()
        .border_color(theme::border_card())
}

/// One end of a stepper.
///
/// Ends disable rather than wrap. A stepper is a number line and you cannot
/// walk off the end of one, which is the whole reason it is not a menu.
pub fn step_button(
    id: &'static str,
    label: &'static str,
    step: Step,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    // The design draws these as text, not as icons: a minus sign and a plus at
    // 14px. Neither is in the icon set, and neither should be - a one-stroke
    // glyph the font already has does not need a 24 viewBox.
    let glyph = match step {
        Step::Down => "\u{2212}",
        Step::Up => "+",
    };
    let mut button = div()
        .id(id)
        .accessibility_id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(theme::u(STEP_BTN))
        .flex()
        .flex_none()
        .items_center()
        .justify_center();
    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(theme::bg_hover()));
    } else {
        button = button.opacity(STEP_DISABLED);
    }
    button
        .text_size(theme::u(theme::TEXT_ROW))
        .text_color(theme::text_pill())
        .child(glyph)
}

/// The number between a stepper's two buttons.
///
/// Tabular digits, so the control is the same width at `9` and at `100` and
/// the button you are clicking does not move out from under the pointer.
pub fn step_value(value: impl Into<SharedString>) -> impl IntoElement {
    div()
        .min_w(theme::u(STEP_VALUE_W))
        .flex()
        .justify_center()
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .font_features(gpui::FontFeatures(std::sync::Arc::new(vec![(
            "tnum".into(),
            1,
        )])))
        .text_color(theme::text_label())
        .child(value.into())
}

/// A path field: recessed, read-only, truncated from the left.
///
/// The tail of a path is the part that identifies it - two capture folders
/// under two different user profiles differ in the middle, and clipping the
/// end would show you the half they have in common. `full` goes to the screen
/// reader, because the visible string is deliberately not the whole one.
pub fn path_field(
    id: &'static str,
    shown: impl Into<SharedString>,
    full: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .aria_label(full.into())
        .flex_1()
        .min_w_0()
        .h(theme::u(CONTROL_H))
        .pl(theme::u(11.0))
        .pr(theme::u(4.0))
        .flex()
        .items_center()
        .gap(theme::u(8.0))
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(theme::bg_well())
        .shadow(theme::recessed())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(theme::u(theme::TEXT_SMALL))
                .text_color(theme::text_pill())
                .child(shown.into()),
        )
}

/// The small button that lives inside a path field.
pub fn action_button(
    id: &'static str,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .accessibility_id(id)
        .focusable()
        .tab_stop(true)
        .role(Role::Button)
        .aria_keyshortcuts("Enter Space")
        .flex_none()
        .h(theme::u(ACTION_H))
        .px(theme::u(10.0))
        .flex()
        .items_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(gpui::rgb(ACTION_BG))
        .cursor_pointer()
        .hover(|style| style.bg(gpui::rgb(ACTION_BG_HOVER)))
        .focus_visible(|style| style.border_color(theme::accent()))
        .text_size(theme::u(theme::TEXT_MICRO))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_label())
        .child(label.into())
}

/// A hotkey field. Recessed like a path, because it displays a binding rather
/// than being the thing you press to fire it.
///
/// Read-only for now: `Settings::hotkeys` is retired and the shortcuts are
/// registered from constants, so there is nothing here to capture into. The
/// accent capturing ring and the amber clash ring the design system draws
/// arrive with the field that can be edited, not before it.
pub fn hotkey_field(binding: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_none()
        .h(theme::u(CONTROL_H))
        .px(theme::u(11.0))
        .flex()
        .items_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(theme::bg_well())
        .shadow(theme::recessed())
        .font_family(MONO)
        .text_size(theme::u(theme::TEXT_MICRO))
        .text_color(theme::text_label())
        .child(binding.into())
}

/// How far a toggle knob travels, for the animation that will drive it.
///
/// Here rather than at the call site so the two ends of the toggle cannot
/// drift apart: the knob is placed from this same arithmetic above.
pub fn knob_travel() -> f32 {
    TOGGLE_W - KNOB - KNOB_INSET * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_knob_sits_the_same_distance_from_both_ends() {
        // Off at 3, on at 38 - 16 - 3 = 19. Symmetric, or the toggle looks
        // wrong in exactly one of its two states and nowhere else.
        assert_eq!(KNOB_INSET, 3.0);
        assert_eq!(TOGGLE_W - KNOB - KNOB_INSET, 19.0);
        assert_eq!(knob_travel(), 16.0);
    }

    #[test]
    fn every_control_clears_the_row_by_the_same_air() {
        // 44 - 30 leaves 7 above and 7 below. A control that is not 30 tall
        // breaks the alignment of every row beside it.
        assert_eq!((ROW_H - CONTROL_H) / 2.0, 7.0);
        assert_eq!(STEP_BTN, CONTROL_H, "a stepper end must fill its track");
    }
}
