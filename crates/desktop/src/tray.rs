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

/// The grid the mark is drawn on.
///
/// Not the size that ships: that is whatever the OS says it will draw the icon
/// at, and every length below is expressed in grid units and scaled into it. So
/// the geometry is written once and read at 32 regardless of the raster.
pub const GRID: u32 = 32;

const RING_OUTER: f32 = 13.0;
/// How thick the ring is, in *device* pixels, never grid units. Below three the
/// ring stops being a ring and becomes a grey smudge, which at 16px is the whole
/// icon - so at small sizes it takes a bigger share of the grid rather than
/// scaling down with everything else.
const RING_STROKE: f32 = 3.0;
/// The state mark, as a fraction of the hole the ring leaves. Follows the ring
/// inwards: a fixed radius would collide with a thickened ring at 16px.
const CORE_RATIO: f32 = 0.65;

/// RGBA, row-major, straight alpha — the layout `tray_icon::Icon::from_rgba`
/// expects — rasterised `size` square.
///
/// Drawn at the size the tray will actually show rather than resampled from one
/// 32px bitmap. Windows downscales with no regard for a three-pixel ring, and at
/// 16px that ring is all there is.
pub fn rgba(state: TrayState, size: u32) -> Vec<u8> {
    let ring = if state == TrayState::Idle { 1.0 } else { 0.45 };
    let scale = size as f32 / GRID as f32;
    let center = size as f32 / 2.0;
    // Half a device pixel, in grid units: what every coverage function below
    // spreads its edge over, so antialiasing stays one pixel wide at any size
    // instead of one grid unit wide.
    let feather = 0.5 / scale;
    let ring_inner = RING_OUTER - (RING_STROKE / scale).max(RING_STROKE);
    let mut buffer = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - center) / scale;
            let py = (y as f32 + 0.5 - center) / scale;
            let distance = (px * px + py * py).sqrt();

            // The mark: a ring, always present, dimmed when it is carrying a
            // state in its centre.
            let mut colour = [252.0, 252.0, 252.0];
            let mut alpha = band(distance, ring_inner, RING_OUTER, feather) * ring;

            if let Some((core_colour, core_alpha)) =
                core(state, px, py, distance, ring_inner, feather)
            {
                // Straight `over`: the core is opaque where it covers.
                colour = core_colour;
                alpha = alpha.max(core_alpha);
            }

            let index = ((y * size + x) * 4) as usize;
            buffer[index] = colour[0] as u8;
            buffer[index + 1] = colour[1] as u8;
            buffer[index + 2] = colour[2] as u8;
            buffer[index + 3] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    buffer
}

/// The state mark inside the ring.
fn core(
    state: TrayState,
    px: f32,
    py: f32,
    distance: f32,
    ring_inner: f32,
    feather: f32,
) -> Option<([f32; 3], f32)> {
    let core = ring_inner * CORE_RATIO;
    match state {
        TrayState::Idle => None,
        TrayState::Recording => Some((rgb(theme::rec()), disc(distance, core, feather))),
        TrayState::Finalizing => Some((rgb(theme::accent()), disc(distance, core, feather))),
        TrayState::Paused => {
            // Two bars, 3 wide, 10 tall, 2 apart.
            let bar = |offset: f32| rect(px - offset, py, 1.5, 5.0, feather);
            Some(([252.0, 252.0, 252.0], bar(-2.5).max(bar(2.5))))
        }
        TrayState::Error => {
            let stem = rect(px, py + 1.5, 1.5, 4.0, feather);
            let dot = disc((px * px + (py - 5.0) * (py - 5.0)).sqrt(), 1.8, feather);
            Some((rgb(theme::warn()), stem.max(dot)))
        }
    }
}

/// Coverage for a distance that is positive inside the shape, feathered across
/// exactly one device pixel however many grid units that comes to.
fn coverage(inside: f32, feather: f32) -> f32 {
    ((inside + feather) / (2.0 * feather)).clamp(0.0, 1.0)
}

/// Antialiased disc coverage.
fn disc(distance: f32, radius: f32, feather: f32) -> f32 {
    coverage(radius - distance, feather)
}

/// Antialiased annulus coverage.
fn band(distance: f32, inner: f32, outer: f32, feather: f32) -> f32 {
    coverage(outer - distance, feather).min(coverage(distance - inner, feather))
}

/// Antialiased axis-aligned rectangle coverage, centred on the origin.
fn rect(px: f32, py: f32, half_width: f32, half_height: f32, feather: f32) -> f32 {
    coverage(half_width - px.abs(), feather).min(coverage(half_height - py.abs(), feather))
}

fn rgb(colour: gpui::Hsla) -> [f32; 3] {
    let rgba = gpui::Rgba::from(colour);
    [rgba.r * 255.0, rgba.g * 255.0, rgba.b * 255.0]
}

#[cfg(test)]
mod tests {
    use rapidcap_capture::CaptureFailure;

    use super::*;

    /// The four sizes a Windows tray asks for, at 100%, 125%, 150% and 200%.
    const SIZES: [u32; 4] = [16, 20, 24, 32];

    const STATES: [TrayState; 5] = [
        TrayState::Idle,
        TrayState::Recording,
        TrayState::Paused,
        TrayState::Finalizing,
        TrayState::Error,
    ];

    fn pixel(buffer: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * size + x) * 4) as usize;
        [
            buffer[index],
            buffer[index + 1],
            buffer[index + 2],
            buffer[index + 3],
        ]
    }

    #[test]
    fn every_state_produces_a_full_rgba_buffer_at_every_size() {
        for size in SIZES {
            for state in STATES {
                let buffer = rgba(state, size);
                assert_eq!(
                    buffer.len(),
                    (size * size * 4) as usize,
                    "{state:?} at {size}"
                );
                // Corners are outside the ring: fully transparent, or the icon
                // renders as a square block the way the old one did.
                assert_eq!(pixel(&buffer, size, 0, 0)[3], 0, "{state:?} at {size}");
                assert_eq!(
                    pixel(&buffer, size, size - 1, size - 1)[3],
                    0,
                    "{state:?} at {size}"
                );
            }
        }
    }

    /// The reason the rasteriser takes a size at all: a ring that scales
    /// linearly is 1.5px at 16px, and 1.5px of antialiased white is a smudge.
    #[test]
    fn the_ring_stays_three_pixels_thick_at_every_size() {
        for size in SIZES {
            let buffer = rgba(TrayState::Idle, size);
            let row = size / 2;
            let lit = (0..size)
                .filter(|x| pixel(&buffer, size, *x, row)[3] > 0)
                .count();
            // Two crossings of the ring on the row through the centre, each
            // at least three pixels of it.
            assert!(
                lit >= 6,
                "{size}px icon lights only {lit} pixels across the middle"
            );
            assert_eq!(
                pixel(&buffer, size, size / 2, row)[3],
                0,
                "{size}px icon filled the hole in the ring"
            );
        }
    }

    #[test]
    fn states_are_visually_distinct() {
        // The regression this guards is the one that shipped: an icon that
        // never changes, so a running capture looks exactly like an idle app.
        for size in SIZES {
            let buffers: Vec<_> = STATES.map(|state| rgba(state, size)).into();
            for (a, first) in buffers.iter().enumerate() {
                for (b, second) in buffers.iter().enumerate().skip(a + 1) {
                    assert_ne!(
                        first, second,
                        "tray states {a} and {b} render identically at {size}px"
                    );
                }
            }
        }
    }

    #[test]
    fn recording_puts_red_at_the_centre() {
        for size in SIZES {
            let buffer = rgba(TrayState::Recording, size);
            let [r, g, b, a] = pixel(&buffer, size, size / 2, size / 2);
            assert_eq!(a, 255, "the core must be opaque at {size}px");
            assert!(
                r > 200 && g < 90 && b < 90,
                "expected red at {size}px, got {r},{g},{b}"
            );

            // Idle has nothing at the centre at all.
            assert_eq!(
                pixel(&rgba(TrayState::Idle, size), size, size / 2, size / 2)[3],
                0
            );
        }
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
            TrayState::from_capture(&CaptureState::Error(CaptureFailure::new(
                "Screenshot",
                "boom",
            ))),
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
            CaptureState::Error(CaptureFailure::new("Recording", "disk full")),
        ] {
            let text = TrayState::from_capture(&state).tooltip(&state);
            assert!(text.starts_with("RapidCap — "));
            for debris in ['(', ')', '{', '}'] {
                assert!(!text.contains(debris), "{state:?} leaked: {text}");
            }
        }
    }
}
