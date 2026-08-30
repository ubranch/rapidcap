//! Recording on macOS: AVFoundation into VideoToolbox.
//!
//! Only `CaptureSource::resolve` and `ffmpeg_args` are still owed a real
//! implementation. Pause and resume below are complete - a POSIX signal is the
//! whole of what ntdll's NtSuspendProcess buys the Windows backend.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
};

use super::RecordingError;
use crate::{CaptureKind, PhysicalRegion, Settings};

pub(super) const FFMPEG_EXE: &str = "ffmpeg";

/// FFmpeg spawned from a bundle never gets a console to hide, so unlike the
/// Windows backend there is nothing to suppress here.
pub(super) fn hide_console(_command: &mut Command) {}

/// macOS has no equivalent of scoop's shim files - a `ffmpeg` found on PATH is
/// already the real binary, or a symlink the kernel resolves for us.
pub(super) fn resolve_shim(executable: &Path) -> PathBuf {
    executable.to_owned()
}

pub(super) fn suspend(child: &Child) -> Result<(), RecordingError> {
    signal(child, libc::SIGSTOP, "pause")
}

pub(super) fn resume(child: &Child) -> Result<(), RecordingError> {
    signal(child, libc::SIGCONT, "resume")
}

fn signal(child: &Child, signal: libc::c_int, verb: &str) -> Result<(), RecordingError> {
    // SAFETY: `kill` only reads the pid. The child is still owned by the caller,
    // so the pid cannot have been reaped and reused underneath this call.
    if unsafe { libc::kill(child.id() as libc::pid_t, signal) } != 0 {
        return Err(RecordingError(format!(
            "{verb} FFmpeg failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Which AVFoundation capture device FFmpeg is pointed at, and the crop within
/// it - the counterpart to the Windows backend's DXGI output index.
pub(super) struct CaptureSource {
    _region: PhysicalRegion,
}

impl CaptureSource {
    pub(super) fn resolve(_region: &PhysicalRegion) -> Result<Self, RecordingError> {
        Err(RecordingError(
            "recording is not implemented on macOS yet".into(),
        ))
    }
}

/// The shape this owes the caller, once `resolve` can name a device: swap
/// `ddagrab` for `-f avfoundation -i "<index>:"`, `d3d11va` for `videotoolbox`,
/// and `h264_nvenc` for `h264_videotoolbox`. The audio half has no direct
/// counterpart - macOS exposes no system-audio device without either a loopback
/// driver installed or ScreenCaptureKit capturing the audio itself.
pub(super) fn ffmpeg_args(
    _kind: CaptureKind,
    _source: &CaptureSource,
    _settings: &Settings,
    _output: &Path,
) -> Vec<String> {
    unreachable!("CaptureSource::resolve fails before any arguments are built")
}
