//! The vocabulary every capture backend speaks: screen rectangles, what the
//! user picked, and the pixels that came back. None of it touches an OS API, so
//! Windows Graphics Capture and ScreenCaptureKit both build their frames out of
//! these types and the geometry stays testable off a real desktop.

use std::fmt;

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
        // A row at a time rather than a pixel at a time. Measured at 4K this is
        // no faster than indexing `self.bytes` four times per pixel - the loop
        // is bound by memory bandwidth and LLVM already elided those bounds
        // checks - but the slice makes the in-range access provable at a glance.
        let row_bytes = crop.width as usize * 4;
        let mut rgba = Vec::with_capacity(row_bytes * crop.height as usize);
        for y in crop.y as u32..crop.y as u32 + crop.height {
            let start = y as usize * self.stride as usize + crop.x as usize * 4;
            for pixel in self.bytes[start..start + row_bytes].chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
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
pub struct CaptureError(pub(crate) String);

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CaptureError {}

// Only the Windows backend converts foreign errors today. Drop the gate as soon
// as `sck` has real ScreenCaptureKit failures to funnel through here.
#[cfg(windows)]
pub(crate) fn capture_error(error: impl fmt::Display) -> CaptureError {
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
}
