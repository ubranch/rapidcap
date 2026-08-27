use std::{
    fmt,
    mem::size_of,
    sync::mpsc::{SyncSender, sync_channel},
    time::Duration,
};

use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalRegion {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRegion {
    pub fn from_drag(start: (i32, i32), end: (i32, i32)) -> Option<Self> {
        let x = start.0.min(end.0);
        let y = start.1.min(end.1);
        let width = start.0.abs_diff(end.0);
        let height = start.1.abs_diff(end.1);
        (width >= 2 && height >= 2).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn intersection(&self, other: Self) -> Option<Self> {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        (right - left >= 2 && bottom - top >= 2).then_some(Self {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureTarget {
    Region(PhysicalRegion),
    Window {
        hwnd: isize,
        region: PhysicalRegion,
        process_name: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawFrame {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl RawFrame {
    pub fn crop_rgba(&self, crop: PhysicalRegion) -> Option<CapturedFrame> {
        if crop.x < 0
            || crop.y < 0
            || crop.x as u32 + crop.width > self.width
            || crop.y as u32 + crop.height > self.height
            || self.stride < self.width * 4
            || self.bytes.len() < self.stride as usize * self.height as usize
        {
            return None;
        }
        let mut rgba = Vec::with_capacity(crop.width as usize * crop.height as usize * 4);
        for y in crop.y as u32..crop.y as u32 + crop.height {
            let row = y as usize * self.stride as usize;
            for x in crop.x as u32..crop.x as u32 + crop.width {
                let pixel = row + x as usize * 4;
                rgba.extend_from_slice(&[
                    self.bytes[pixel + 2],
                    self.bytes[pixel + 1],
                    self.bytes[pixel],
                    self.bytes[pixel + 3],
                ]);
            }
        }
        Some(CapturedFrame {
            rgba,
            width: crop.width,
            height: crop.height,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureError(String);

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CaptureError {}

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

fn capture_error(error: impl fmt::Display) -> CaptureError {
    CaptureError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_drag_normalizes_negative_virtual_coordinates() {
        assert_eq!(
            PhysicalRegion::from_drag((-100, 50), (-500, 350)).unwrap(),
            PhysicalRegion {
                x: -500,
                y: 50,
                width: 400,
                height: 300,
            }
        );
    }

    #[test]
    fn tiny_drag_is_rejected() {
        assert_eq!(PhysicalRegion::from_drag((10, 10), (11, 30)), None);
    }

    #[test]
    fn region_clamps_to_monitor() {
        let region = PhysicalRegion {
            x: -20,
            y: 10,
            width: 100,
            height: 80,
        };
        let monitor = PhysicalRegion {
            x: 0,
            y: 0,
            width: 50,
            height: 50,
        };
        assert_eq!(
            region.intersection(monitor),
            Some(PhysicalRegion {
                x: 0,
                y: 10,
                width: 50,
                height: 40
            })
        );
    }

    #[test]
    fn padded_bgra_crop_becomes_tight_rgba() {
        let frame = RawFrame {
            bytes: vec![
                1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
            ],
            width: 2,
            height: 2,
            stride: 12,
        };
        let cropped = frame
            .crop_rgba(PhysicalRegion {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            })
            .unwrap();
        assert_eq!(cropped.rgba, [7, 6, 5, 8, 15, 14, 13, 16]);
        assert_eq!((cropped.width, cropped.height), (1, 2));
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
