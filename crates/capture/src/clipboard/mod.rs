//! Putting a capture on the system clipboard.
//!
//! The error type lives here so both backends report failures the same way; the
//! two `write_clipboard*` entry points come from whichever platform module is
//! compiled in.

use std::fmt;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{write_clipboard, write_clipboard_file};
#[cfg(windows)]
pub use windows::{write_clipboard, write_clipboard_file};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardError(pub(crate) String);

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ClipboardError {}
