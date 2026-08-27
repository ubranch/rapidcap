use gpui::Context;
use rapidcap_capture::{AppPaths, CaptureCommand, CaptureKind, CaptureState, Settings, StateError};

pub struct AppController {
    state: CaptureState,
    settings: Settings,
    paths: AppPaths,
    generation: u64,
}

impl AppController {
    pub fn new(settings: Settings, paths: AppPaths) -> Self {
        Self {
            state: CaptureState::Idle,
            settings,
            paths,
            generation: 0,
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

    pub fn dispatch(
        &mut self,
        command: CaptureCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), CommandError> {
        let next = match command {
            CaptureCommand::CaptureRegion => self.start(CaptureKind::RegionScreenshot),
            CaptureCommand::CaptureActiveWindow => self.start(CaptureKind::ActiveWindowScreenshot),
            CaptureCommand::ToggleVideo => self.toggle_recording(CaptureKind::Video),
            CaptureCommand::ToggleGif => self.toggle_recording(CaptureKind::Gif),
            CaptureCommand::Cancel => self.state.clone().cancel().map_err(CommandError::from),
        }?;
        self.state = next;
        self.generation = self.generation.wrapping_add(1);
        cx.notify();
        Ok(())
    }

    fn start(&self, kind: CaptureKind) -> Result<CaptureState, CommandError> {
        self.state.clone().start(kind).map_err(CommandError::from)
    }

    fn toggle_recording(&self, kind: CaptureKind) -> Result<CaptureState, CommandError> {
        match self.state {
            CaptureState::Idle => self.start(kind),
            CaptureState::Recording(active) if active == kind => {
                self.state.clone().stop(kind).map_err(CommandError::from)
            }
            _ => Err(CommandError::Busy),
        }
    }

    #[cfg(test)]
    fn set_state_for_test(&mut self, state: CaptureState) {
        self.state = state;
    }
}

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
}
