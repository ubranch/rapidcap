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

/// The mark, as in the app icon (scripts/make-icon.py): two viewfinder
/// corners, top-left and bottom-right, leaning forward round a centre dot.
/// Each corner is two strokes from its elbow, in grid units from the centre.
const CORNERS: [[(f32, f32); 3]; 2] = [
    [(-10.0, -3.5), (-10.0, -9.0), (-2.0, -9.0)],
    [(2.0, 9.0), (10.0, 9.0), (10.0, 3.5)],
];
/// Horizontal shift per unit of height: the top moves right.
const LEAN: f32 = 0.18;
/// Half the stroke, in *device* pixels at the least. Thinner than that and
/// the corners turn into a grey smudge at 16px.
const HALF_STROKE: f32 = 1.25;
/// The centre dot when idle, and the state mark's radius otherwise.
const DOT: f32 = 2.2;
const CORE: f32 = 4.6;

/// RGBA, row-major, straight alpha — the layout `tray_icon::Icon::from_rgba`
/// expects — rasterised `size` square.
///
/// Drawn at the size the tray will actually show rather than resampled from one
/// 32px bitmap. Windows downscales with no regard for a stroke two pixels
/// wide, and at 16px those strokes are all there is.
pub fn rgba(state: TrayState, size: u32) -> Vec<u8> {
    let corners = if state == TrayState::Idle { 1.0 } else { 0.45 };
    let scale = size as f32 / GRID as f32;
    let center = size as f32 / 2.0;
    // Half a device pixel, in grid units: what every coverage function below
    // spreads its edge over, so antialiasing stays one pixel wide at any size
    // instead of one grid unit wide.
    let feather = 0.5 / scale;
    let half = (HALF_STROKE / scale).max(HALF_STROKE);
    let mut buffer = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - center) / scale;
            let py = (y as f32 + 0.5 - center) / scale;
            let distance = (px * px + py * py).sqrt();

            // The corners, always present, dimmed when the centre carries a
            // state. Undo the lean to find the point on the upright shape.
            let upright = px + LEAN * py;
            let stroke = CORNERS
                .iter()
                .flat_map(|corner| corner.windows(2))
                .map(|pair| segment(upright, py, pair[0], pair[1]))
                .fold(f32::MAX, f32::min);
            let mut colour = [252.0, 252.0, 252.0];
            let mut alpha = coverage(half - stroke, feather) * corners;

            if let Some((core_colour, core_alpha)) = core(state, px, py, distance, feather) {
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

/// The mark in the centre: a small dot when idle, the state otherwise.
fn core(
    state: TrayState,
    px: f32,
    py: f32,
    distance: f32,
    feather: f32,
) -> Option<([f32; 3], f32)> {
    match state {
        TrayState::Idle => Some(([252.0, 252.0, 252.0], disc(distance, DOT, feather))),
        TrayState::Recording => Some((rgb(theme::rec()), disc(distance, CORE, feather))),
        TrayState::Finalizing => Some((rgb(theme::accent()), disc(distance, CORE, feather))),
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

/// Distance from a point to the segment `a`-`b`.
fn segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx - px, a.1 + t * dy - py);
    (cx * cx + cy * cy).sqrt()
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
                // The icon's own corners are outside the mark: fully
                // transparent, or it renders as a square block the way an old
                // one did.
                assert_eq!(pixel(&buffer, size, 0, 0)[3], 0, "{state:?} at {size}");
                assert_eq!(
                    pixel(&buffer, size, size - 1, size - 1)[3],
                    0,
                    "{state:?} at {size}"
                );
            }
        }
    }

    /// The reason the rasteriser takes a size at all: a stroke that scales
    /// linearly is a pixel and a bit at 16px, and that much antialiased white
    /// is a smudge.
    #[test]
    fn the_corners_stay_two_pixels_thick_at_every_size() {
        for size in SIZES {
            let buffer = rgba(TrayState::Idle, size);
            // A column through the top-left corner's upright stroke.
            let column = (size as f32 * 0.2) as u32;
            let widest = (0..size)
                .map(|y| {
                    (0..size)
                        .filter(|x| {
                            x.abs_diff(column) < size / 6 && pixel(&buffer, size, *x, y)[3] > 200
                        })
                        .count()
                })
                .max()
                .unwrap_or(0);
            assert!(
                widest >= 2,
                "{size}px corners are only {widest} pixels thick"
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

            // Idle has the mark's small dot there, never red.
            let [r, g, b, a] = pixel(&rgba(TrayState::Idle, size), size, size / 2, size / 2);
            assert!(
                a > 200 && r > 200 && g > 200 && b > 200,
                "idle centre at {size}px"
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
