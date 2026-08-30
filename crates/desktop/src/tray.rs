//! The tray icon.
//!
//! RapidCap minimises to tray, so for most of its life this square *is* the
//! product. It carries state: a static icon cannot tell a running capture from
//! an idle app, and during a recording the tray is often the only RapidCap UI
//! on screen.
//!
//! Rasterised in code rather than shipped as .ico files — five states times
//! four DPI sizes is twenty files to keep in sync with one palette.

use rapidcap_capture::{CaptureKind, CaptureState};

use crate::theme;

/// What the icon shows. Derived from `CaptureState`, not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayState {
    Idle,
    Recording,
    Paused,
    Finalizing,
    Error,
}

impl TrayState {
    pub fn from_capture(state: &CaptureState) -> Self {
        match state {
            CaptureState::Recording(_) | CaptureState::Countdown(_, _) => Self::Recording,
            CaptureState::Paused(_) => Self::Paused,
            CaptureState::Finalizing(_) => Self::Finalizing,
            CaptureState::Error(_) => Self::Error,
            CaptureState::Idle | CaptureState::Selecting(_) => Self::Idle,
        }
    }

    /// Hover text. Mirrors the panel's status well — same words, same order.
    pub fn tooltip(self, state: &CaptureState) -> String {
        let detail = match state {
            CaptureState::Countdown(kind, seconds) => {
                format!("{} starts in {seconds}", kind_noun(*kind))
            }
            CaptureState::Recording(kind) => format!("Recording {}", kind_noun(*kind)),
            CaptureState::Paused(kind) => format!("Paused {}", kind_noun(*kind)),
            CaptureState::Finalizing(kind) => format!("Finalizing {}", kind_noun(*kind)),
            CaptureState::Selecting(_) => "Selecting".to_string(),
            CaptureState::Error(_) => "Capture failed".to_string(),
            CaptureState::Idle => "Ready".to_string(),
        };
        format!("RapidCap — {detail}")
    }
}

fn kind_noun(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Video => "video",
        CaptureKind::Gif => "GIF",
        CaptureKind::RegionScreenshot => "region",
        CaptureKind::ActiveWindowScreenshot => "window",
    }
}

/// Rendered at 32 so Windows has pixels to downscale from at 100% DPI and
/// something honest to show at 200%.
pub const SIZE: u32 = 32;

const CENTER: f32 = SIZE as f32 / 2.0;
const RING_OUTER: f32 = 13.0;
const RING_INNER: f32 = 10.0;
const CORE: f32 = 6.5;

/// RGBA, row-major, straight alpha — the layout `tray_icon::Icon::from_rgba`
/// expects.
pub fn rgba(state: TrayState) -> Vec<u8> {
    let ring = if state == TrayState::Idle { 1.0 } else { 0.45 };
    let mut buffer = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5 - CENTER;
            let py = y as f32 + 0.5 - CENTER;
            let distance = (px * px + py * py).sqrt();

            // The mark: a ring, always present, dimmed when it is carrying a
            // state in its centre.
            let mut colour = [252.0, 252.0, 252.0];
            let mut alpha = band(distance, RING_INNER, RING_OUTER) * ring;

            if let Some((core_colour, core_alpha)) = core(state, px, py, distance) {
                // Straight `over`: the core is opaque where it covers.
                colour = core_colour;
                alpha = alpha.max(core_alpha);
            }

            let index = ((y * SIZE + x) * 4) as usize;
            buffer[index] = colour[0] as u8;
            buffer[index + 1] = colour[1] as u8;
            buffer[index + 2] = colour[2] as u8;
            buffer[index + 3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    buffer
}

/// The state mark inside the ring.
fn core(state: TrayState, px: f32, py: f32, distance: f32) -> Option<([f32; 3], f32)> {
    match state {
        TrayState::Idle => None,
        TrayState::Recording => Some((rgb(theme::rec()), disc(distance, CORE))),
        TrayState::Finalizing => Some((rgb(theme::accent()), disc(distance, CORE))),
        TrayState::Paused => {
            // Two bars, 3 wide, 10 tall, 2 apart.
            let bar = |offset: f32| rect(px - offset, py, 1.5, 5.0);
            Some(([252.0, 252.0, 252.0], bar(-2.5).max(bar(2.5))))
        }
        TrayState::Error => {
            let stem = rect(px, py + 1.5, 1.5, 4.0);
            let dot = disc((px * px + (py - 5.0) * (py - 5.0)).sqrt(), 1.8);
            Some((rgb(theme::warn()), stem.max(dot)))
        }
    }
}

/// Antialiased disc coverage.
fn disc(distance: f32, radius: f32) -> f32 {
    (radius + 0.5 - distance).clamp(0.0, 1.0)
}

/// Antialiased annulus coverage.
fn band(distance: f32, inner: f32, outer: f32) -> f32 {
    let outside = (outer + 0.5 - distance).clamp(0.0, 1.0);
    let inside = (distance - (inner - 0.5)).clamp(0.0, 1.0);
    outside.min(inside)
}

/// Antialiased axis-aligned rectangle coverage, centred on the origin.
fn rect(px: f32, py: f32, half_width: f32, half_height: f32) -> f32 {
    let x = (half_width + 0.5 - px.abs()).clamp(0.0, 1.0);
    let y = (half_height + 0.5 - py.abs()).clamp(0.0, 1.0);
    x.min(y)
}

fn rgb(colour: gpui::Hsla) -> [f32; 3] {
    let rgba = gpui::Rgba::from(colour);
    [rgba.r * 255.0, rgba.g * 255.0, rgba.b * 255.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(buffer: &[u8], x: u32, y: u32) -> [u8; 4] {
        let index = ((y * SIZE + x) * 4) as usize;
        [
            buffer[index],
            buffer[index + 1],
            buffer[index + 2],
            buffer[index + 3],
        ]
    }

    #[test]
    fn every_state_produces_a_full_rgba_buffer() {
        for state in [
            TrayState::Idle,
            TrayState::Recording,
            TrayState::Paused,
            TrayState::Finalizing,
            TrayState::Error,
        ] {
            let buffer = rgba(state);
            assert_eq!(buffer.len(), (SIZE * SIZE * 4) as usize, "{state:?}");
            // Corners are outside the ring: fully transparent, or the icon
            // renders as a square block the way the old one did.
            assert_eq!(pixel(&buffer, 0, 0)[3], 0, "{state:?} corner is opaque");
            assert_eq!(pixel(&buffer, 31, 31)[3], 0, "{state:?} corner is opaque");
        }
    }

    #[test]
    fn states_are_visually_distinct() {
        // The regression this guards is the one that shipped: an icon that
        // never changes, so a running capture looks exactly like an idle app.
        let buffers: Vec<_> = [
            TrayState::Idle,
            TrayState::Recording,
            TrayState::Paused,
            TrayState::Finalizing,
            TrayState::Error,
        ]
        .map(rgba)
        .into();
        for (a, first) in buffers.iter().enumerate() {
            for (b, second) in buffers.iter().enumerate().skip(a + 1) {
                assert_ne!(first, second, "tray states {a} and {b} render identically");
            }
        }
    }

    #[test]
    fn recording_puts_red_at_the_centre() {
        let buffer = rgba(TrayState::Recording);
        let [r, g, b, a] = pixel(&buffer, SIZE / 2, SIZE / 2);
        assert_eq!(a, 255, "the core must be opaque");
        assert!(r > 200 && g < 90 && b < 90, "expected red, got {r},{g},{b}");

        // Idle has nothing at the centre at all.
        assert_eq!(pixel(&rgba(TrayState::Idle), SIZE / 2, SIZE / 2)[3], 0);
    }

    #[test]
    fn tray_state_follows_capture_state() {
        assert_eq!(
            TrayState::from_capture(&CaptureState::Recording(CaptureKind::Video)),
            TrayState::Recording
        );
        assert_eq!(
            TrayState::from_capture(&CaptureState::Countdown(CaptureKind::Gif, 3)),
            TrayState::Recording,
            "a countdown is already committed — the icon should not read as idle"
        );
        assert_eq!(
            TrayState::from_capture(&CaptureState::Error("boom".into())),
            TrayState::Error
        );
        assert_eq!(
            TrayState::from_capture(&CaptureState::Idle),
            TrayState::Idle
        );
    }

    #[test]
    fn tooltip_never_leaks_a_rust_type() {
        for state in [
            CaptureState::Idle,
            CaptureState::Selecting(CaptureKind::RegionScreenshot),
            CaptureState::Countdown(CaptureKind::Video, 3),
            CaptureState::Recording(CaptureKind::Gif),
            CaptureState::Paused(CaptureKind::Video),
            CaptureState::Finalizing(CaptureKind::Video),
            CaptureState::Error("disk full".into()),
        ] {
            let text = TrayState::from_capture(&state).tooltip(&state);
            assert!(text.starts_with("RapidCap — "));
            for debris in ['(', ')', '{', '}'] {
                assert!(!text.contains(debris), "{state:?} leaked: {text}");
            }
        }
    }
}
