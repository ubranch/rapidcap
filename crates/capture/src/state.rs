use std::{fmt, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureCommand {
    CaptureRegion,
    CaptureActiveWindow,
    ToggleVideo,
    ToggleGif,
    TogglePause,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureKind {
    RegionScreenshot,
    ActiveWindowScreenshot,
    Video,
    Gif,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureState {
    Idle,
    Selecting(CaptureKind),
    Countdown(CaptureKind, u8),
    Recording(CaptureKind),
    Paused(CaptureKind),
    Finalizing(CaptureKind),
    Error(CaptureFailure),
}

/// A failure, split the way it is read.
///
/// `summary` is a written sentence for the status well and the error bar;
/// `detail` is whatever the failing call actually said. One blob could only
/// ever be truncated to fit the well, and truncating an FFmpeg line yields
/// thirty-nine characters of nothing useful.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureFailure {
    pub summary: String,
    pub detail: String,
}

impl CaptureFailure {
    /// What the status well fits, in characters.
    ///
    /// The well sizes to its content and shares a 400px row with two chips, so
    /// a longer summary pushes that row wider than the panel. The error bar has
    /// more room, but the same string lands in both.
    pub const SUMMARY_MAX: usize = 40;

    /// `operation` is the noun the user recognises — "Recording",
    /// "Screenshot". It is passed in because only the caller knows it: reading
    /// it back out of the message text would be a guess that fails silently.
    pub fn new(operation: &str, detail: impl fmt::Display) -> Self {
        let detail = detail.to_string();
        let first = detail.lines().next().unwrap_or_default().trim();
        let inline = format!("{operation} failed — {first}");
        // Not truncated: thirty-nine characters of an FFmpeg line say nothing
        // the user can act on, and the whole string is one hover away.
        let summary = if first.is_empty() || inline.chars().count() > Self::SUMMARY_MAX {
            format!("{operation} failed — see details")
        } else {
            inline
        };
        Self { summary, detail }
    }

    /// Whether the summary only points at the detail instead of carrying it.
    ///
    /// This is what decides that Copy log appears: a bar already showing the
    /// whole error has nothing left to copy.
    pub fn is_summarised(&self) -> bool {
        !self.summary.ends_with(self.detail.trim())
    }
}

impl fmt::Display for CaptureFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.summary)
    }
}

impl CaptureState {
    pub fn start(self, kind: CaptureKind) -> Result<Self, StateError> {
        match self {
            Self::Idle => Ok(Self::Selecting(kind)),
            state => Err(StateError::Busy(state)),
        }
    }

    pub fn cancel(self) -> Result<Self, StateError> {
        match self {
            Self::Selecting(_) | Self::Countdown(_, _) => Ok(Self::Idle),
            state => Err(StateError::InvalidTransition(state)),
        }
    }

    pub fn stop(self, kind: CaptureKind) -> Result<Self, StateError> {
        match self {
            Self::Recording(active) | Self::Paused(active) if active == kind => {
                Ok(Self::Finalizing(kind))
            }
            state => Err(StateError::InvalidTransition(state)),
        }
    }

    pub fn pause(self, kind: CaptureKind) -> Result<Self, StateError> {
        match self {
            Self::Recording(active) if active == kind => Ok(Self::Paused(kind)),
            state => Err(StateError::InvalidTransition(state)),
        }
    }

    pub fn resume(self, kind: CaptureKind) -> Result<Self, StateError> {
        match self {
            Self::Paused(active) if active == kind => Ok(Self::Recording(kind)),
            state => Err(StateError::InvalidTransition(state)),
        }
    }

    /// Whether quitting right now would destroy a capture.
    ///
    /// `Recording` and `Paused` have an encoder holding an open part file,
    /// `Finalizing` is still writing the real one, and `Countdown` is a take
    /// the user has already committed to. `Selecting` only has an overlay on
    /// screen and `Error` has nothing at all, so both are safe to quit from —
    /// which matters, because refusing to exit from the error state leaves the
    /// app with no way out.
    pub fn blocks_exit(&self) -> bool {
        matches!(
            self,
            Self::Countdown(_, _) | Self::Recording(_) | Self::Paused(_) | Self::Finalizing(_)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    Busy(CaptureState),
    InvalidTransition(CaptureState),
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy(state) => write!(formatter, "capture already active: {state:?}"),
            Self::InvalidTransition(state) => {
                write!(formatter, "invalid capture transition from {state:?}")
            }
        }
    }
}

impl std::error::Error for StateError {}

/// A capture that reached disk.
///
/// `recorded` is the elapsed time of a video or GIF and `None` for a
/// screenshot, which is what tells the two apart afterwards - a screenshot has
/// no duration to report and a recording always does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedOutput {
    pub path: PathBuf,
    pub recorded: Option<Duration>,
    /// Whether the capture also reached the clipboard. A failed clipboard write
    /// does not fail the capture - the file is still saved - so it travels as a
    /// flag rather than as an error.
    pub copied: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureEvent {
    StateChanged(CaptureState),
    OutputSaved(SavedOutput),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_states_holding_capture_data_block_exit() {
        for state in [
            CaptureState::Countdown(CaptureKind::Video, 3),
            CaptureState::Recording(CaptureKind::Video),
            CaptureState::Paused(CaptureKind::Gif),
            CaptureState::Finalizing(CaptureKind::Video),
        ] {
            assert!(state.blocks_exit(), "{state:?} should block exit");
        }

        for state in [
            CaptureState::Idle,
            CaptureState::Selecting(CaptureKind::RegionScreenshot),
            CaptureState::Error(CaptureFailure::new("Recording", "disk full")),
        ] {
            assert!(!state.blocks_exit(), "{state:?} should not block exit");
        }
    }

    #[test]
    fn a_long_detail_is_replaced_by_a_pointer_to_it_not_truncated() {
        let short = CaptureFailure::new("Recording", "disk full");
        assert_eq!(short.summary, "Recording failed — disk full");
        assert_eq!(short.detail, "disk full");

        let raw =
            "ffmpeg: Error initializing output stream 0:0 -- opening encoder\nffmpeg exited 1";
        let long = CaptureFailure::new("Recording", raw);
        assert_eq!(long.summary, "Recording failed — see details");
        assert_eq!(long.detail, raw, "the tooltip still gets every character");

        assert!(!short.is_summarised(), "the bar already shows all of it");
        assert!(long.is_summarised(), "so Copy log has something to offer");

        for failure in [short, long, CaptureFailure::new("Screenshot", "")] {
            assert!(
                failure.summary.chars().count() <= CaptureFailure::SUMMARY_MAX,
                "{} does not fit the status well",
                failure.summary
            );
        }
    }
}
