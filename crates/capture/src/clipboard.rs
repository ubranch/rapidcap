use std::{fmt, os::windows::ffi::OsStrExt, path::Path, thread, time::Duration};

use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_DIBV5, CF_HDROP, CF_UNICODETEXT},
        },
    },
    core::w,
};

use crate::SavedCapture;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardError(String);

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ClipboardError {}

/// A screenshot: the pixels *and* the file, so a paste lands as an image in
/// chat and as a file in a folder.
pub fn write_clipboard(capture: &SavedCapture) -> Result<(), ClipboardError> {
    let dib = dibv5_bytes(&capture.rgba, capture.width, capture.height)?;
    write_formats(&capture.path, Some(dib))
}

/// A recording: the file only.
///
/// Video and GIF used to skip the clipboard entirely, so stopping a recording
/// left you with a path in a toast and nothing to paste. There is no `CF_DIBV5`
/// to offer here - a video has no single frame to be - but `CF_HDROP` is what
/// every file target actually reads, and it is what Explorer produces when you
/// copy a file. Paste into chat, into a mail draft, into a folder: all work.
pub fn write_clipboard_file(path: &Path) -> Result<(), ClipboardError> {
    write_formats(path, None)
}

fn write_formats(path: &Path, dib: Option<Vec<u8>>) -> Result<(), ClipboardError> {
    let text = unicode_path_bytes(path);
    let drop = hdrop_bytes(path);
    let effect = 1_u32.to_le_bytes();

    let mut formats = Vec::with_capacity(4);
    if let Some(dib) = &dib {
        formats.push((CF_DIBV5.0 as u32, GlobalBlock::new(dib)?));
    }
    formats.push((CF_UNICODETEXT.0 as u32, GlobalBlock::new(&text)?));
    formats.push((CF_HDROP.0 as u32, GlobalBlock::new(&drop)?));

    let mut opened = false;
    for _ in 0..5 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            opened = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !opened {
        return Err(ClipboardError("clipboard is busy".into()));
    }
    let _close = ClipboardGuard;
    unsafe { EmptyClipboard() }.map_err(win_error)?;
    let drop_effect = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    if drop_effect == 0 {
        return Err(ClipboardError(
            "register Preferred DropEffect failed".into(),
        ));
    }
    formats.push((drop_effect, GlobalBlock::new(&effect)?));

    for (format, mut block) in formats {
        let handle = block.handle();
        unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) }.map_err(win_error)?;
        block.transfer();
    }
    Ok(())
}

fn dibv5_bytes(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ClipboardError> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(ClipboardError(
            "pixel buffer dimensions do not match".into(),
        ));
    }
    // Header plus every pixel, reserved up front. `extend` below feeds this from
    // a `flat_map`, which reports no upper size hint, so the buffer used to grow
    // by doubling and recopy the whole image several times over.
    let mut bytes = Vec::with_capacity(124 + expected);
    bytes.resize(124, 0);
    bytes[0..4].copy_from_slice(&124_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32_u16.to_le_bytes());
    bytes[16..20].copy_from_slice(&3_u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&(expected as u32).to_le_bytes());
    bytes[40..44].copy_from_slice(&0x00ff_0000_u32.to_le_bytes());
    bytes[44..48].copy_from_slice(&0x0000_ff00_u32.to_le_bytes());
    bytes[48..52].copy_from_slice(&0x0000_00ff_u32.to_le_bytes());
    bytes[52..56].copy_from_slice(&0xff00_0000_u32.to_le_bytes());
    bytes[56..60].copy_from_slice(&0x7352_4742_u32.to_le_bytes());
    for row in rgba.chunks_exact(width as usize * 4).rev() {
        for pixel in row.chunks_exact(4) {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Ok(bytes)
}

fn unicode_path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn hdrop_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = vec![0; 20];
    bytes[0..4].copy_from_slice(&20_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes.extend(unicode_path_bytes(path));
    bytes.extend([0, 0]);
    bytes
}

struct GlobalBlock(Option<HGLOBAL>);

impl GlobalBlock {
    fn new(bytes: &[u8]) -> Result<Self, ClipboardError> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(win_error)?;
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            unsafe { drop(GlobalFree(Some(handle))) };
            return Err(ClipboardError("GlobalLock failed".into()));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            drop(GlobalUnlock(handle));
        }
        Ok(Self(Some(handle)))
    }

    fn handle(&self) -> HGLOBAL {
        self.0.expect("clipboard block already transferred")
    }

    fn transfer(&mut self) {
        self.0 = None;
    }
}

impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe { drop(GlobalFree(Some(handle))) };
        }
    }
}

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe { drop(CloseClipboard()) };
    }
}

fn win_error(error: windows::core::Error) -> ClipboardError {
    ClipboardError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dibv5_is_bottom_up_bgra() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        let bytes = dibv5_bytes(&rgba, 1, 2).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 124);
        assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        assert_eq!(&bytes[124..128], &[7, 6, 5, 8]);
    }

    #[test]
    fn file_drop_is_wide_and_double_terminated() {
        let bytes = hdrop_bytes(Path::new(r"C:\Shots\one.png"));
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        assert!(bytes.ends_with(&[0, 0, 0, 0]));
    }
}
