//! Screenshots on macOS, via Core Graphics window-list capture.
//!
//! The geometry and the frame types live in `geometry`; this file is only the
//! part that talks to the OS, so it presents the same `capture_screenshot` the
//! Windows backend in `wgc` does.
//!
//! Unlike Windows Graphics Capture, Core Graphics composites the requested
//! rectangle itself, in global display coordinates and across as many displays
//! as it spans. There is no monitor to pick and no offset to subtract, so the
//! whole "which output does this region belong to" dance the Windows backend
//! performs has no counterpart here.

use core_graphics::{
    display::{CGDisplay, CGPoint, CGRect, CGSize},
    window::{kCGNullWindowID, kCGWindowImageBestResolution, kCGWindowListOptionAll},
};

use crate::geometry::{CaptureError, CaptureTarget, CapturedFrame, PhysicalRegion, RawFrame};

/// Device pixels per point, the factor that turns Core Graphics geometry into
/// the pixels [`PhysicalRegion`] is named for.
///
/// Every rectangle macOS hands out - `CGDisplayBounds`, `kCGWindowBounds`, an
/// `NSWindow` frame - is measured in points, the resolution-independent unit
/// AppKit lays out in. `PhysicalRegion` means device pixels, because that is
/// what it means on Windows and what the overlay divides by the window's
/// backing scale before drawing. On a Retina display the two differ by two, so
/// every macOS rectangle is multiplied by this on the way into a region and
/// divided by it on the way back out to an OS call.
///
/// ponytail: one scale for the whole desktop, read off the main display. A
/// Retina laptop beside a non-Retina monitor has no single global pixel space
/// to convert into at all - the two displays' point origins would need
/// different factors, and their scaled rectangles would overlap - so the honest
/// fix is carrying the display id alongside every rectangle. Not worth it until
/// someone captures on a mixed-DPI desktop.
pub fn display_scale() -> Option<f32> {
    let mode = CGDisplay::main().display_mode()?;
    let points = mode.width();
    (points != 0).then(|| mode.pixel_width() as f32 / points as f32)
}

// Both are Core Graphics, macOS 10.15 and later, and neither is bound by the
// `core-graphics` crate.
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Check the Screen Recording grant before anything tries to read the screen.
///
/// Neither of the two capture paths fails usefully without it. Core Graphics
/// answers `CGDisplay::screenshot` with a picture of the desktop and no windows
/// in it, and FFmpeg's AVFoundation input blocks inside `avformat_open_input`
/// until the grant arrives - measured on macOS 26 as a recording that never
/// wrote a single byte, sat out the ten second stop timeout, and reported
/// "FFmpeg stop timed out" with nothing to show for it.
///
/// The request is not a dialog to wait on: `CGRequestScreenCaptureAccess` posts
/// the system prompt and returns immediately, so this attempt is lost either
/// way. Asking anyway is what puts the prompt on screen the first time, and the
/// next attempt goes through once the box is ticked.
///
/// RapidCap is signed ad hoc, so its code hash changes with every build and
/// macOS treats each new build as a different application that has to be
/// granted again. Until the app ships with a stable signing identity this will
/// fire after every update rather than only once.
pub fn ensure_screen_access() -> Result<(), CaptureError> {
    // SAFETY: neither call takes an argument or returns anything to free.
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    unsafe { CGRequestScreenCaptureAccess() };
    Err(CaptureError(
        "RapidCap has no Screen Recording permission - allow it in System Settings > Privacy &          Security > Screen & System Audio Recording, then try again"
            .into(),
    ))
}

pub fn capture_screenshot(target: &CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    ensure_screen_access()?;
    let region = match target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    // The window server is asked in points, so the region's pixels convert back
    // before it is handed over.
    let scale = f64::from(
        display_scale().ok_or_else(|| CaptureError("no display to capture from".into()))?,
    );
    let bounds = CGRect::new(
        &CGPoint::new(f64::from(region.x) / scale, f64::from(region.y) / scale),
        &CGSize::new(
            f64::from(region.width) / scale,
            f64::from(region.height) / scale,
        ),
    );
    let image = CGDisplay::screenshot(
        bounds,
        kCGWindowListOptionAll,
        kCGNullWindowID,
        kCGWindowImageBestResolution,
    )
    .ok_or_else(|| {
        CaptureError(
            "screen capture returned no image - grant RapidCap Screen Recording permission in \
             System Settings > Privacy & Security"
                .into(),
        )
    })?;
    // `kCGWindowImageBestResolution` answers in native pixels, which is what the
    // region asked for in the first place - but the point rectangle above was
    // rounded on the way in, so the image can come back a pixel either side of
    // it. The frame is built from the image's own size rather than the region's
    // so those pixels are described accurately instead of being trusted away.
    let width = image.width() as u32;
    let height = image.height() as u32;
    if image.bits_per_pixel() != 32 {
        return Err(CaptureError(format!(
            "screen capture returned {} bits per pixel, expected 32",
            image.bits_per_pixel()
        )));
    }
    let frame = RawFrame {
        bytes: image.data().bytes().to_vec(),
        width,
        height,
        stride: image.bytes_per_row() as u32,
    };
    // Core Graphics hands back BGRA on every little-endian Mac, which is the
    // same order Windows Graphics Capture uses, so `crop_rgba`'s swizzle serves
    // both backends.
    frame
        .crop_rgba(PhysicalRegion {
            x: 0,
            y: 0,
            width,
            height,
        })
        .ok_or_else(|| CaptureError("captured frame did not contain requested crop".into()))
}
