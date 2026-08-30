//! Clipboard on macOS: not built yet.
//!
//! The shape this owes the caller is one `NSPasteboard` write per capture
//! carrying both representations, the way the Windows backend publishes
//! CF_DIBV5 and CF_HDROP together: `NSPasteboardTypePNG` for the pixels so a
//! paste lands as an image in chat, and `NSPasteboardTypeFileURL` for the path
//! so the same paste lands as a file in Finder.

use std::path::Path;

use super::ClipboardError;
use crate::SavedCapture;

pub fn write_clipboard(_capture: &SavedCapture) -> Result<(), ClipboardError> {
    Err(ClipboardError(
        "clipboard write is not implemented on macOS yet".into(),
    ))
}

pub fn write_clipboard_file(_path: &Path) -> Result<(), ClipboardError> {
    Err(ClipboardError(
        "clipboard file write is not implemented on macOS yet".into(),
    ))
}
