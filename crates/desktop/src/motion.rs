//! Motion.
//!
//! GPUI has no CSS transitions, so hover and press fills land instantly. That
//! is deliberate and it is what ships: a capture tool is judged on whether it
//! got out of the way in time, and nothing here may move while the user is
//! timing something.
//!
//! What does animate is animated through [`gpui::AnimationExt`], which already
//! honours the system reduced-motion setting - a repeating animation renders in
//! its start state and schedules no frames at all. There is no tween engine
//! here and there is not going to be one.

use std::time::Duration;

use gpui::{Animation, AnimationExt, AnyElement, Hsla, bounce, div, ease_in_out, prelude::*};

use crate::theme;

/// The saved chip arriving and leaving.
pub const CHIP_FADE: Duration = Duration::from_millis(160);

/// The HUD's idle fade, in each direction.
pub const HUD_FADE: Duration = Duration::from_millis(240);

/// One cycle of the recording dot.
const PULSE: Duration = Duration::from_secs(2);

/// How far the pulse dips. Not to zero: a dot that disappears reads as a dot
/// that stopped, which is the opposite of what it is saying.
const PULSE_MIN: f32 = 0.45;

/// The dot in a status well or on the HUD.
///
/// It pulses only while recording, and then it is the only thing in the product
/// that loops. `repeat_synced` phase-locks it to a clock the whole app shares,
/// so the panel's dot and the HUD's dot dip together rather than beating
/// against each other when both are on screen.
pub fn status_dot(id: &'static str, size: f32, colour: Hsla, pulsing: bool) -> AnyElement {
    let dot = div()
        .size(theme::u(size))
        .flex_none()
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(colour);
    if !pulsing {
        return dot.into_any_element();
    }
    dot.with_animation(
        id,
        Animation::new(PULSE)
            .repeat_synced()
            // `bounce` runs the curve forwards then backwards, which is the
            // 0%/50%/100% keyframe the design asks for without a second
            // animation to bring the opacity back up.
            .with_easing(bounce(ease_in_out)),
        |dot, delta| dot.opacity(1.0 - delta * (1.0 - PULSE_MIN)),
    )
    .into_any_element()
}
