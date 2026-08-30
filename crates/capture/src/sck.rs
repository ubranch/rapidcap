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

pub fn capture_screenshot(target: &CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    let region = match target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    let bounds = CGRect::new(
        &CGPoint::new(f64::from(region.x), f64::from(region.y)),
        &CGSize::new(f64::from(region.width), f64::from(region.height)),
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
    // `bounds` is in points but `kCGWindowImageBestResolution` answers in native
    // pixels, so on a Retina display the image comes back at twice the width the
    // region named. Cropping to the image's own size keeps those extra pixels:
    // a screenshot tool that threw away half the resolution it was handed would
    // be worse than one that never asked for it.
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
