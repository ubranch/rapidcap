//! The capture state machine, transition by transition.
//!
//! The existing tests cover the happy path. This one covers the matrix: every
//! transition from every state, so a future edit that quietly widens one of
//! them has to say so here first.

use rapidcap_capture::{CaptureKind, CaptureState, StateError};

const KINDS: [CaptureKind; 4] = [
    CaptureKind::RegionScreenshot,
    CaptureKind::ActiveWindowScreenshot,
    CaptureKind::Video,
    CaptureKind::Gif,
];

fn every_state() -> Vec<CaptureState> {
    let mut states = vec![CaptureState::Idle, CaptureState::Error("boom".into())];
    for kind in KINDS {
        states.push(CaptureState::Selecting(kind));
        states.push(CaptureState::Countdown(kind, 3));
        states.push(CaptureState::Recording(kind));
        states.push(CaptureState::Paused(kind));
        states.push(CaptureState::Finalizing(kind));
    }
    states
}

#[test]
fn only_idle_starts_a_capture() {
    for state in every_state() {
        let result = state.clone().start(CaptureKind::Video);
        match state {
            CaptureState::Idle => {
                assert_eq!(result, Ok(CaptureState::Selecting(CaptureKind::Video)));
            }
            busy => assert_eq!(
                result,
                Err(StateError::Busy(busy.clone())),
                "{busy:?} must refuse a second capture"
            ),
        }
    }
}

#[test]
fn cancel_only_backs_out_of_the_phases_before_the_recorder_runs() {
    for state in every_state() {
        let result = state.clone().cancel();
        match state {
            CaptureState::Selecting(_) | CaptureState::Countdown(_, _) => {
                assert_eq!(result, Ok(CaptureState::Idle), "{state:?} should cancel");
            }
            other => assert_eq!(
                result,
                Err(StateError::InvalidTransition(other.clone())),
                // Once FFmpeg holds a file, throwing the state away would strand
                // it. Recording is left by `stop`, which finalises the file.
                "{other:?} has no cancel: it must be stopped, not abandoned"
            ),
        }
    }
}

#[test]
fn stop_requires_a_live_recording_of_the_same_kind() {
    for state in every_state() {
        for kind in KINDS {
            let result = state.clone().stop(kind);
            let expected = match state {
                CaptureState::Recording(active) | CaptureState::Paused(active)
                    if active == kind =>
                {
                    Ok(CaptureState::Finalizing(kind))
                }
                ref other => Err(StateError::InvalidTransition(other.clone())),
            };
            assert_eq!(result, expected, "stop({kind:?}) from {state:?}");
        }
    }
}

#[test]
fn a_recording_cannot_be_stopped_by_the_other_recording_button() {
    // Pressing GIF while a video runs must not finalise the video.
    assert_eq!(
        CaptureState::Recording(CaptureKind::Video).stop(CaptureKind::Gif),
        Err(StateError::InvalidTransition(CaptureState::Recording(
            CaptureKind::Video
        )))
    );
}

#[test]
fn pause_and_resume_are_mirror_images() {
    for kind in KINDS {
        let recording = CaptureState::Recording(kind);
        let paused = recording.clone().pause(kind).unwrap();
        assert_eq!(paused, CaptureState::Paused(kind));
        assert_eq!(paused.resume(kind).unwrap(), recording);
    }
}

#[test]
fn pause_is_not_idempotent_and_says_so() {
    let paused = CaptureState::Paused(CaptureKind::Video);
    assert_eq!(
        paused.clone().pause(CaptureKind::Video),
        Err(StateError::InvalidTransition(paused))
    );
    let recording = CaptureState::Recording(CaptureKind::Video);
    assert_eq!(
        recording.clone().resume(CaptureKind::Video),
        Err(StateError::InvalidTransition(recording))
    );
}

#[test]
fn finalizing_and_error_are_left_by_the_controller_not_by_a_transition() {
    // Neither state has a method back to `Idle`: the controller resets them
    // when the capture task reports in (`finish_recording` /
    // `finish_screenshot`) or when the next command arrives. If a transition
    // ever grows that job, this assertion is the place to notice.
    for kind in KINDS {
        let finalizing = CaptureState::Finalizing(kind);
        assert!(finalizing.clone().cancel().is_err());
        assert!(finalizing.clone().stop(kind).is_err());
        assert!(finalizing.start(kind).is_err());
    }
    let error = CaptureState::Error("disk full".into());
    assert!(error.clone().cancel().is_err());
    assert!(error.start(CaptureKind::Video).is_err());
}

#[test]
fn the_error_message_rides_along_in_the_state_error() {
    let state = CaptureState::Recording(CaptureKind::Gif);
    let error = state.clone().start(CaptureKind::Video).unwrap_err();
    assert_eq!(error, StateError::Busy(state));
    assert!(
        error.to_string().contains("Recording"),
        "the message has to name the state it refused: {error}"
    );
}
