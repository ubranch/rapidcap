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
    Error(String),
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
    Failed(String),
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
            CaptureState::Error("disk full".into()),
        ] {
            assert!(!state.blocks_exit(), "{state:?} should not block exit");
        }
    }
}
