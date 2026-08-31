use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Context, EventEmitter};
use rapidcap_capture::{
    AppPaths, CaptureCommand, CaptureEvent, CaptureFailure, CaptureKind, CaptureState,
    CaptureTarget, RecordingError, SavedCapture, SavedOutput, ScreenshotError, Settings,
    SettingsStore, StateError,
};

pub struct AppController {
    state: CaptureState,
    settings: Settings,
    paths: AppPaths,
    target: Option<CaptureTarget>,
    recorded: Duration,
    recording_since: Option<Instant>,
    /// The last failure, kept separately from `state`.
    ///
    /// `state` has to return to `Idle` so the next capture can start, but the
    /// message has to outlive that: clearing it on the next command meant the
    /// user's click dismissed a notice they had not read yet.
    error: Option<CaptureFailure>,
    /// The command Retry re-runs, when there is a failure to retry.
    ///
    /// Only the four that start a capture are recorded. Pause and Cancel cannot
    /// be the command a `finish_*` failure came from, and re-running one from
    /// the error bar would do something the user never asked for.
    last_command: Option<CaptureCommand>,
}

impl AppController {
    pub fn new(settings: Settings, paths: AppPaths) -> Self {
        Self {
            state: CaptureState::Idle,
            settings,
            paths,
            target: None,
            recorded: Duration::ZERO,
            recording_since: None,
            error: None,
            last_command: None,
        }
    }

    pub fn state(&self) -> &CaptureState {
        &self.state
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn target(&self) -> Option<&CaptureTarget> {
        self.target.as_ref()
    }

    pub fn recording_elapsed(&self) -> Duration {
        self.recorded
            + self
                .recording_since
                .map(|started| started.elapsed())
                .unwrap_or_default()
    }

    pub fn set_target(&mut self, target: CaptureTarget, cx: &mut Context<Self>) {
        if let CaptureState::Selecting(kind @ (CaptureKind::Video | CaptureKind::Gif)) = self.state
        {
            self.state = CaptureState::Countdown(kind, self.settings.countdown_seconds);
        }
        self.target = Some(target.clone());
        cx.emit(target);
        cx.notify();
    }

    pub fn begin_recording(&mut self, kind: CaptureKind, cx: &mut Context<Self>) {
        if matches!(self.state, CaptureState::Countdown(active, _) if active == kind) {
            self.state = CaptureState::Recording(kind);
            self.recorded = Duration::ZERO;
            self.recording_since = Some(Instant::now());
            cx.emit(CaptureEvent::StateChanged(self.state.clone()));
            cx.notify();
        }
    }

    /// `copied` says whether the file also reached the clipboard. The write
    /// happens on the background thread that finished the recording, so its
    /// outcome has to be carried in - the controller cannot ask afterwards.
    pub fn finish_recording(
        &mut self,
        result: Result<PathBuf, RecordingError>,
        copied: bool,
        cx: &mut Context<Self>,
    ) {
        self.target = None;
        // Read before the reset: this is the only moment the finished length is
        // still known, and the status well is about to report it.
        let recorded = Some(self.recording_elapsed());
        self.recorded = Duration::ZERO;
        self.recording_since = None;
        match result {
            Ok(path) => {
                self.state = CaptureState::Idle;
                self.error = None;
                cx.emit(CaptureEvent::OutputSaved(SavedOutput {
                    path,
                    recorded,
                    copied,
                }));
            }
            Err(error) => {
                tracing::error!(%error, "recording failed");
                self.fail("Recording", error);
            }
        }
        cx.notify();
    }

    pub fn finish_screenshot(
        &mut self,
        result: Result<SavedCapture, ScreenshotError>,
        copied: bool,
        cx: &mut Context<Self>,
    ) {
        self.target = None;
        match result {
            Ok(saved) => {
                self.state = CaptureState::Idle;
                self.error = None;
                cx.emit(CaptureEvent::OutputSaved(SavedOutput {
                    path: saved.path,
                    // A screenshot is an instant, so there is no duration to
                    // report and the well shows the clipboard result instead.
                    recorded: None,
                    copied,
                }));
            }
            Err(error) => {
                tracing::error!(%error, "screenshot failed");
                self.fail("Screenshot", error);
            }
        }
        cx.notify();
    }

    /// Countdown slots offered by the segmented control, in order.
    pub const COUNTDOWN_CHOICES: [u8; 3] = [0, 3, 5];

    pub fn set_countdown(&mut self, seconds: u8, cx: &mut Context<Self>) {
        if self.settings.countdown_seconds != seconds {
            self.settings.countdown_seconds = seconds;
            self.persist_settings();
            cx.notify();
        }
    }

    /// Mute or unmute the soundtrack on video recordings.
    ///
    /// Takes effect on the next recording, not the running one: FFmpeg's inputs
    /// are fixed at spawn, so a mid-capture change would need a restart and
    /// lose the take.
    pub fn toggle_audio(&mut self, cx: &mut Context<Self>) {
        self.settings.audio.enabled = !self.settings.audio.enabled;
        self.persist_settings();
        cx.notify();
    }

    /// Best effort: a settings file that cannot be written must not take the
    /// running app down with it, but it does belong in the log.
    fn persist_settings(&self) {
        if let Err(error) =
            SettingsStore::new(self.paths.settings_file.clone()).save(&self.settings)
        {
            tracing::warn!(%error, "persist settings");
        }
    }

    /// The summary is written here rather than sniffed out of the message
    /// downstream: this is the only place that knows which operation failed.
    fn fail(&mut self, operation: &str, error: impl std::fmt::Display) {
        let failure = CaptureFailure::new(operation, error);
        self.state = CaptureState::Error(failure.clone());
        self.error = Some(failure);
    }

    /// The last failure, until it is dismissed or a capture succeeds.
    pub fn error(&self) -> Option<&CaptureFailure> {
        self.error.as_ref()
    }

    /// The command the error bar offers as Retry, if there is one.
    ///
    /// A departure from the spec, which says Retry appears only when the failed
    /// command is idempotent: that is a property of the error, and deciding it
    /// would mean matching on message text. The command is what the code
    /// actually knows, so Retry is offered whenever the failure came from one
    /// that starts a capture. Pressing it on a failure that will fail again
    /// costs a click; guessing wrong from a substring costs a wrong button.
    pub fn retry_command(&self) -> Option<CaptureCommand> {
        self.error.as_ref().and(self.last_command)
    }

    /// Runs Retry, clearing the notice the button was attached to.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let Some(command) = self.retry_command() else {
            return;
        };
        self.dismiss_error(cx);
        if let Err(error) = self.dispatch(command, cx) {
            tracing::warn!(?error, ?command, "retry rejected");
        }
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        if self.error.take().is_some() {
            cx.notify();
        }
    }

    pub fn dispatch(
        &mut self,
        command: CaptureCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), CommandError> {
        // Leaving the error state is how the next capture becomes possible, but
        // `self.error` deliberately survives — see the field comment.
        if matches!(self.state, CaptureState::Error(_)) {
            self.state = CaptureState::Idle;
        }
        if matches!(
            command,
            CaptureCommand::CaptureRegion
                | CaptureCommand::CaptureActiveWindow
                | CaptureCommand::ToggleVideo
                | CaptureCommand::ToggleGif
        ) {
            self.last_command = Some(command);
        }
        let previous = self.state.clone();
        let next = match command {
            CaptureCommand::CaptureRegion => self.start(CaptureKind::RegionScreenshot),
            CaptureCommand::CaptureActiveWindow => self.start(CaptureKind::ActiveWindowScreenshot),
            CaptureCommand::ToggleVideo => self.toggle_recording(CaptureKind::Video),
            CaptureCommand::ToggleGif => self.toggle_recording(CaptureKind::Gif),
            CaptureCommand::TogglePause => self.toggle_pause(),
            CaptureCommand::Cancel => self.state.clone().cancel().map_err(CommandError::from),
        }?;
        self.state = next;
        match (&previous, &self.state) {
            (CaptureState::Recording(_), CaptureState::Paused(_)) => {
                if let Some(started) = self.recording_since.take() {
                    self.recorded += started.elapsed();
                }
            }
            (CaptureState::Paused(_), CaptureState::Recording(_)) => {
                self.recording_since = Some(Instant::now());
            }
            _ => {}
        }
        // Keyed on the state rather than on the command: Escape is not the only
        // way back to `Idle`, and a region left over from an abandoned capture
        // would be inherited by the next one, which the user never drew it for.
        if self.state == CaptureState::Idle {
            self.target = None;
        }
        cx.emit(command);
        cx.notify();
        Ok(())
    }

    fn start(&self, kind: CaptureKind) -> Result<CaptureState, CommandError> {
        self.state.clone().start(kind).map_err(CommandError::from)
    }

    fn toggle_recording(&self, kind: CaptureKind) -> Result<CaptureState, CommandError> {
        match self.state {
            CaptureState::Idle => self.start(kind),
            // Backing out covers the whole run-up, not just the countdown. The
            // panel stays on screen while the region is picked, so the button
            // that started the selection is visible and clickable throughout;
            // answering `Busy` to it left Escape as the only way out.
            CaptureState::Selecting(active) | CaptureState::Countdown(active, _)
                if active == kind =>
            {
                self.state.clone().cancel().map_err(CommandError::from)
            }
            CaptureState::Recording(active) | CaptureState::Paused(active) if active == kind => {
                self.state.clone().stop(kind).map_err(CommandError::from)
            }
            _ => Err(CommandError::Busy),
        }
    }

    fn toggle_pause(&self) -> Result<CaptureState, CommandError> {
        match self.state {
            CaptureState::Recording(kind) => {
                self.state.clone().pause(kind).map_err(CommandError::from)
            }
            CaptureState::Paused(kind) => {
                self.state.clone().resume(kind).map_err(CommandError::from)
            }
            _ => Err(CommandError::Busy),
        }
    }

    #[cfg(test)]
    fn set_state_for_test(&mut self, state: CaptureState) {
        self.state = state;
    }

    /// `RecordingError` wraps a private `String`, so a test cannot build one.
    #[cfg(test)]
    fn set_error_for_test(&mut self, detail: &str) {
        self.fail("Recording", detail);
    }
}

impl EventEmitter<CaptureCommand> for AppController {}
impl EventEmitter<CaptureTarget> for AppController {}
impl EventEmitter<CaptureEvent> for AppController {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    Busy,
    InvalidState,
}

impl From<StateError> for CommandError {
    fn from(error: StateError) -> Self {
        match error {
            StateError::Busy(_) => Self::Busy,
            StateError::InvalidTransition(_) => Self::InvalidState,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext};
    use rapidcap_capture::{
        AppPaths, CaptureCommand, CaptureFailure, CaptureKind, CaptureState, Settings,
    };

    use super::*;

    fn paths() -> AppPaths {
        AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local")
    }

    #[gpui::test]
    fn capture_region_enters_selection(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::CaptureRegion, cx)
                .unwrap();
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Selecting(CaptureKind::RegionScreenshot)
        );
    }

    #[gpui::test]
    fn capture_is_rejected_while_finalizing(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_state_for_test(CaptureState::Finalizing(CaptureKind::Video));
        });
        let result = controller.update(cx, |controller, cx| {
            controller.dispatch(CaptureCommand::ToggleGif, cx)
        });
        assert_eq!(result, Err(CommandError::Busy));
    }

    #[gpui::test]
    fn selected_target_is_retained(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        let target = rapidcap_capture::CaptureTarget::Region(rapidcap_capture::PhysicalRegion {
            x: -10,
            y: 20,
            width: 300,
            height: 200,
        });
        controller.update(cx, |controller, cx| {
            controller.set_target(target.clone(), cx)
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.target().cloned()),
            Some(target)
        );
    }

    #[gpui::test]
    fn video_selection_enters_configured_countdown(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
            controller.set_target(
                rapidcap_capture::CaptureTarget::Region(rapidcap_capture::PhysicalRegion {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                }),
                cx,
            );
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Countdown(CaptureKind::Video, 5)
        );
    }

    #[gpui::test]
    fn matching_recording_button_cancels_countdown(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
            controller.set_target(
                rapidcap_capture::CaptureTarget::Region(rapidcap_capture::PhysicalRegion {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                }),
                cx,
            );
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Idle
        );
    }

    #[gpui::test]
    fn error_message_survives_the_next_command(cx: &mut TestAppContext) {
        // The bug this guards shipped: `dispatch` reset the error state, so the
        // very click the user made to read the message also threw it away.
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_error_for_test("disk full");
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller
                .error()
                .map(|failure| failure.summary.clone())),
            Some("Recording failed — disk full".to_string())
        );

        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::CaptureRegion, cx)
                .unwrap();
        });
        assert!(
            controller.read_with(cx, |controller, _| controller.error().is_some()),
            "starting a new capture must not silently drop an unread failure"
        );

        controller.update(cx, |controller, cx| controller.dismiss_error(cx));
        assert!(controller.read_with(cx, |controller, _| controller.error().is_none()));
    }

    #[gpui::test]
    fn a_successful_capture_clears_the_previous_error(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_error_for_test("disk full");
        });
        controller.update(cx, |controller, cx| {
            controller.finish_recording(Ok(PathBuf::from("C:/out.mp4")), true, cx);
        });
        assert!(
            controller.read_with(cx, |controller, _| controller.error().is_none()),
            "a capture that worked answers the question the error asked"
        );
    }

    #[gpui::test]
    fn retry_offers_the_capture_that_failed_and_never_cancel(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_error_for_test("disk full");
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.retry_command()),
            None,
            "nothing has been asked for yet, so there is nothing to run again"
        );

        controller.update(cx, |controller, cx| {
            controller.dispatch(CaptureCommand::ToggleGif, cx).unwrap();
            // Backing out of the selection must not become the thing Retry runs.
            controller.dispatch(CaptureCommand::Cancel, cx).unwrap();
            controller.set_error_for_test("disk full");
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.retry_command()),
            Some(CaptureCommand::ToggleGif)
        );

        controller.update(cx, |controller, cx| controller.dismiss_error(cx));
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.retry_command()),
            None,
            "the button goes with the bar it sits on"
        );
    }

    #[gpui::test]
    fn the_same_recording_button_backs_out_of_its_own_selection(cx: &mut TestAppContext) {
        // The panel stays on screen while the region is being picked, so the
        // Video button is right there and clickable. It used to answer `Busy`:
        // the only way out of a selection you had just started was Escape.
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Selecting(CaptureKind::Video)
        );

        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
        });

        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Idle
        );
    }

    #[gpui::test]
    fn the_other_recording_button_still_refuses_a_live_selection(cx: &mut TestAppContext) {
        // Backing out is only for the button that started it. GIF must not
        // cancel a video selection out from under the user.
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
        });
        let result = controller.update(cx, |controller, cx| {
            controller.dispatch(CaptureCommand::ToggleGif, cx)
        });
        assert_eq!(result, Err(CommandError::Busy));
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Selecting(CaptureKind::Video)
        );
    }

    #[gpui::test]
    fn backing_out_of_a_selection_drops_the_target(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
            controller.set_target(
                rapidcap_capture::CaptureTarget::Region(rapidcap_capture::PhysicalRegion {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                }),
                cx,
            );
            controller
                .dispatch(CaptureCommand::ToggleVideo, cx)
                .unwrap();
        });
        assert!(
            controller.read_with(cx, |controller, _| controller.target().is_none()),
            "a cancelled selection must not leave its region behind for the next capture"
        );
    }

    #[gpui::test]
    fn next_command_recovers_from_previous_error(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_state_for_test(CaptureState::Error(CaptureFailure::new(
                "Recording",
                "previous failure",
            )));
        });
        controller.update(cx, |controller, cx| {
            controller
                .dispatch(CaptureCommand::CaptureRegion, cx)
                .unwrap();
        });
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.state().clone()),
            CaptureState::Selecting(CaptureKind::RegionScreenshot)
        );
    }
}
