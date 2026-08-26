use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureCommand {
    CaptureRegion,
    CaptureActiveWindow,
    ToggleVideo,
    ToggleGif,
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
            Self::Recording(active) if active == kind => Ok(Self::Finalizing(kind)),
            state => Err(StateError::InvalidTransition(state)),
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureEvent {
    StateChanged(CaptureState),
    OutputSaved(PathBuf),
    ClipboardFailed(String),
    Failed(String),
}
