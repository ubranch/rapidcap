//! Clipboard on macOS, via `NSPasteboard`.
//!
//! One write per capture carrying both representations, the way the Windows
//! backend publishes CF_DIBV5 and CF_HDROP together: PNG so a paste lands as an
//! image in chat, and a file URL so the same paste lands as a file in Finder.
//! `NSPasteboard` keeps both on the one pasteboard and each consumer takes the
//! richest type it understands.

use std::path::Path;

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};

use super::ClipboardError;
use crate::SavedCapture;

pub fn write_clipboard(capture: &SavedCapture) -> Result<(), ClipboardError> {
    // The pixels are re-encoded rather than read back from `capture.path`, so
    // the clipboard write does not depend on the file having landed. PNG rather
    // than the TIFF the pasteboard prefers: it is what `image` already links,
    // and every macOS paste target reads it.
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &capture.rgba,
            capture.width,
            capture.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| ClipboardError(format!("encode clipboard PNG failed: {error}")))?;

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // The file URL goes on first. `writeObjects` appends an item, and an item
    // added after `setData_forType` would land on a pasteboard whose first item
    // already carries the image, leaving the pixels and the path on separate
    // items where a paste sees only one of them.
    // Best effort: a pasteboard that took the pixels but not the path is still
    // a working copy, and the image is the representation the user asked for.
    let _ = write_file_url(&pasteboard, &capture.path);
    let data = NSData::with_bytes(&png);
    if !unsafe { pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG) } {
        return Err(ClipboardError("clipboard rejected the image".into()));
    }
    Ok(())
}

/// Plain text, nothing else - no file URL, so a paste is the string itself.
pub fn write_clipboard_text(text: &str) -> Result<(), ClipboardError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: both arguments are live for the call and the pasteboard copies
    // what it keeps.
    if !unsafe { pasteboard.setString_forType(&NSString::from_str(text), NSPasteboardTypeString) } {
        return Err(ClipboardError("clipboard rejected the text".into()));
    }
    Ok(())
}

pub fn write_clipboard_file(path: &Path) -> Result<(), ClipboardError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    write_file_url(&pasteboard, path)
}

/// Appends the path as a file URL. The caller owns `clearContents`, because a
/// clear here would wipe an image the same write had already published.
fn write_file_url(pasteboard: &NSPasteboard, path: &Path) -> Result<(), ClipboardError> {
    let path = path
        .to_str()
        .ok_or_else(|| ClipboardError("capture path is not valid UTF-8".into()))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let items = NSArray::from_retained_slice(&[ProtocolObject::from_retained(url)]);
    if !pasteboard.writeObjects(&items) {
        return Err(ClipboardError("clipboard rejected the file path".into()));
    }
    Ok(())
}
