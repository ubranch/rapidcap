//! Screenshots on Windows, via Windows Graphics Capture.
//!
//! The geometry and the frame types live in `geometry`; this file is only the
//! part that talks to the OS, so the macOS backend in `sck` can present the
//! same `capture_screenshot` without duplicating any of the cropping.

use std::{
    mem::size_of,
    sync::mpsc::{SyncSender, sync_channel},
    time::Duration,
};

use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GetMonitorInfoW, HMONITOR, InvalidateRect, MONITORINFO},
};
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

use crate::geometry::{
    CaptureError, CaptureTarget, CapturedFrame, PhysicalRegion, RawFrame, capture_error,
};

struct OneFrameCapture {
    sender: SyncSender<RawFrame>,
}

impl GraphicsCaptureApiHandler for OneFrameCapture {
    type Flags = SyncSender<RawFrame>;
    type Error = String;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { sender: ctx.flags })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let width = frame.width();
        let height = frame.height();
        let mut buffer = frame.buffer().map_err(|error| error.to_string())?;
        let stride = buffer.row_pitch();
        self.sender
            .send(RawFrame {
                bytes: buffer.as_raw_buffer().to_vec(),
                width,
                height,
                stride,
            })
            .map_err(|error| error.to_string())?;
        control.stop();
        Ok(())
    }
}

pub fn capture_screenshot(target: &CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    let target_region = match target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    let (monitor, monitor_region, crop) = Monitor::enumerate()
        .map_err(capture_error)?
        .into_iter()
        .filter_map(|monitor| {
            let bounds = monitor_region(monitor).ok()?;
            let crop = target_region.intersection(bounds.clone())?;
            Some((monitor, bounds, crop))
        })
        .max_by_key(|(_, _, crop)| u64::from(crop.width) * u64::from(crop.height))
        .ok_or_else(|| CaptureError("capture region does not intersect a monitor".into()))?;
    let local_crop = PhysicalRegion {
        x: crop.x - monitor_region.x,
        y: crop.y - monitor_region.y,
        width: crop.width,
        height: crop.height,
    };
    // ponytail: one monitor per capture; split/stitch only when cross-monitor selection is demanded.
    let (sender, receiver) = sync_channel(1);
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Exclude,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        sender,
    );
    let control = OneFrameCapture::start_free_threaded(settings).map_err(capture_error)?;
    // Windows Graphics Capture hands over a frame only when the monitor's
    // content changes, and we ask for no cursor, so an idle desktop produces
    // nothing at all and the wait below expires on a screen that is perfectly
    // fine. A null window invalidates every window on the desktop, which makes
    // DWM present once and gives the session the frame it is waiting for.
    let _ = unsafe { InvalidateRect(None, None, false) };
    let frame = match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = control.stop();
            return Err(CaptureError(format!("screen capture timed out: {error}")));
        }
    };
    control.wait().map_err(capture_error)?;
    frame
        .crop_rgba(local_crop)
        .ok_or_else(|| CaptureError("captured frame did not contain requested crop".into()))
}

fn monitor_region(monitor: Monitor) -> Result<PhysicalRegion, CaptureError> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &raw mut info) }
        .ok()
        .map_err(capture_error)?;
    let rect = info.rcMonitor;
    Ok(PhysicalRegion {
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left) as u32,
        height: (rect.bottom - rect.top) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an idle interactive Windows desktop"]
    fn idle_desktop_still_gives_up_a_frame() {
        // The bug this guards: WGC only pushes a frame when the monitor changes,
        // so on a desktop nobody is touching the capture waited two seconds and
        // reported a timeout on a screen that was working perfectly. Let the
        // desktop go quiet between attempts, which is exactly the state that
        // used to fail.
        for _ in 0..5 {
            std::thread::sleep(Duration::from_secs(3));
            capture_screenshot(&CaptureTarget::Region(PhysicalRegion {
                x: 0,
                y: 0,
                width: 16,
                height: 16,
            }))
            .unwrap();
        }
    }

    #[test]
    #[ignore = "requires interactive Windows desktop"]
    fn real_wgc_captures_primary_pixels() {
        let frame = capture_screenshot(&CaptureTarget::Region(PhysicalRegion {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        }))
        .unwrap();
        assert_eq!(
            (frame.width, frame.height, frame.rgba.len()),
            (16, 16, 1024)
        );
    }
}
