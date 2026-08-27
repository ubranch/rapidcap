use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{Context, EventEmitter};
use rapidcap_capture::{
    AppPaths, CaptureCommand, CaptureEvent, CaptureKind, CaptureState, CaptureTarget,
    RecordingError, SavedCapture, ScreenshotError, Settings, StateError,
};

pub struct AppController {
    state: CaptureState,
    settings: Settings,
    paths: AppPaths,
    target: Option<CaptureTarget>,
    generation: u64,
    recorded: Duration,
    recording_since: Option<Instant>,
}

impl AppController {
    pub fn new(settings: Settings, paths: AppPaths) -> Self {
        Self {
            state: CaptureState::Idle,
            settings,
            paths,
            target: None,
            generation: 0,
            recorded: Duration::ZERO,
            recording_since: None,
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

    pub fn finish_recording(
        &mut self,
        result: Result<PathBuf, RecordingError>,
        cx: &mut Context<Self>,
    ) {
        self.target = None;
        self.recorded = Duration::ZERO;
        self.recording_since = None;
        match result {
            Ok(path) => {
                self.state = CaptureState::Idle;
                cx.emit(CaptureEvent::OutputSaved(path));
            }
            Err(error) => {
                let message = error.to_string();
                self.state = CaptureState::Error(message.clone());
                cx.emit(CaptureEvent::Failed(message));
            }
        }
        cx.notify();
    }

    pub fn finish_screenshot(
        &mut self,
        result: Result<SavedCapture, ScreenshotError>,
        cx: &mut Context<Self>,
    ) {
        self.target = None;
        match result {
            Ok(saved) => {
                self.state = CaptureState::Idle;
                cx.emit(CaptureEvent::OutputSaved(saved.path));
            }
            Err(error) => {
                let message = error.to_string();
                self.state = CaptureState::Error(message.clone());
                cx.emit(CaptureEvent::Failed(message));
            }
        }
        cx.notify();
    }

    pub fn dispatch(
        &mut self,
        command: CaptureCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), CommandError> {
        if matches!(self.state, CaptureState::Error(_)) {
            self.state = CaptureState::Idle;
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
        if matches!(command, CaptureCommand::Cancel) {
            self.target = None;
        }
        self.generation = self.generation.wrapping_add(1);
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
            CaptureState::Countdown(active, _) if active == kind => {
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
    use rapidcap_capture::{AppPaths, CaptureCommand, CaptureKind, CaptureState, Settings};

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
    fn next_command_recovers_from_previous_error(cx: &mut TestAppContext) {
        let controller = cx.new(|_| AppController::new(Settings::default(), paths()));
        controller.update(cx, |controller, _| {
            controller.set_state_for_test(CaptureState::Error("previous failure".into()));
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
