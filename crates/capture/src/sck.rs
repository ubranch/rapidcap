//! Screenshots on macOS, via ScreenCaptureKit.
//!
//! Not built yet. `gpui` already locks `screencapturekit 0.2.8` into the tree,
//! so the real implementation has a vetted crate waiting for it; what it owes
//! this module is one `CapturedFrame` in the same top-left-origin coordinates
//! `geometry` uses, which is `SCShareableContent` for the display list and a
//! one-frame `SCStream` for the pixels.
//!
//! Note this needs the Screen Recording TCC grant, which only persists for a
//! signed bundle - a bare `cargo run` binary re-prompts on every launch.

use crate::geometry::{CaptureError, CaptureTarget, CapturedFrame};

pub fn capture_screenshot(_target: &CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    Err(CaptureError(
        "screen capture is not implemented on macOS yet".into(),
    ))
}
