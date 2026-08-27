use gpui::{
    App, Bounds, Context, Entity, FocusHandle, KeyBinding, Render, Role, Subscription, Window,
    WindowAppearance, WindowBounds, WindowHandle, WindowOptions, actions, div, prelude::*, px, rgb,
    size,
};
use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState, CaptureTarget};

use crate::controller::AppController;
use crate::platform::open_folder;

actions!(
    rapidcap,
    [
        RegionAction,
        WindowAction,
        VideoAction,
        GifAction,
        OpenOutputAction,
        TabAction,
        TabPrevAction
    ]
);

pub const CONTROL_IDS: [&str; 5] = [
    "capture-region",
    "capture-window",
    "record-video",
    "record-gif",
    "open-output",
];

pub struct MainWindow {
    controller: Entity<AppController>,
    focus_handle: FocusHandle,
    _controller_subscription: Subscription,
}

impl MainWindow {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        controller: Entity<AppController>,
    ) -> Self {
        window.set_window_title("RapidCap");
        let controller_subscription = cx.observe(&controller, |_, _, cx| cx.notify());
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        window.on_window_should_close(cx, |_window, cx| {
            cx.hide();
            false
        });
        Self {
            controller,
            focus_handle,
            _controller_subscription: controller_subscription,
        }
    }

    #[cfg(test)]
    fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    fn dispatch(&mut self, command: CaptureCommand, cx: &mut Context<Self>) {
        let _ = self
            .controller
            .update(cx, |controller, cx| controller.dispatch(command, cx));
    }

    fn capture_region(&mut self, _: &RegionAction, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(CaptureCommand::CaptureRegion, cx);
    }

    fn capture_window(&mut self, _: &WindowAction, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(CaptureCommand::CaptureActiveWindow, cx);
    }

    fn toggle_video(&mut self, _: &VideoAction, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(CaptureCommand::ToggleVideo, cx);
    }

    fn toggle_gif(&mut self, _: &GifAction, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(CaptureCommand::ToggleGif, cx);
    }

    fn open_output(&mut self, _: &OpenOutputAction, _: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = open_folder(&self.controller.read(cx).paths().capture_root) {
            tracing::error!(%error, "open output folder");
        }
    }

    fn focus_next(&mut self, _: &TabAction, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    fn focus_previous(&mut self, _: &TabPrevAction, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let canvas = if dark { rgb(0x111318) } else { rgb(0xf3f4f6) };
        let surface = if dark { rgb(0x23262d) } else { rgb(0xffffff) };
        let border = if dark { rgb(0x444954) } else { rgb(0xc8ccd4) };
        let text = if dark { rgb(0xf4f6fb) } else { rgb(0x17191f) };
        let secondary = if dark { rgb(0xa9b0bf) } else { rgb(0x5f6673) };
        let accent = if dark { rgb(0x3478f6) } else { rgb(0x225cc5) };
        let focus = if dark { rgb(0xa8c6ff) } else { rgb(0x174fae) };

        let button = |id: &'static str,
                      accessibility_id: &'static str,
                      context: &'static str,
                      label: &'static str| {
            div()
                .id(id)
                .accessibility_id(accessibility_id)
                .key_context(context)
                .focusable()
                .tab_stop(true)
                .role(Role::Button)
                .aria_label(label)
                .aria_keyshortcuts("Enter Space")
                .flex_1()
                .h(px(54.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.0))
                .border_1()
                .border_color(border)
                .bg(surface)
                .text_color(text)
                .cursor_pointer()
                .hover(move |style| style.bg(accent))
                .focus_visible(move |style| style.border_color(focus))
                .child(label)
        };

        let state = self.controller.read(cx).state().clone();
        let status = match self.controller.read(cx).target() {
            Some(CaptureTarget::Region(region)) => {
                format!("Selected {} × {}", region.width, region.height)
            }
            Some(CaptureTarget::Window { process_name, .. }) => {
                format!("Selected {process_name}")
            }
            None => match &state {
                CaptureState::Idle => "Ready".to_string(),
                other => format!("{other:?}"),
            },
        };
        let output = self
            .controller
            .read(cx)
            .paths()
            .capture_root
            .display()
            .to_string();
        let video_fps = self.controller.read(cx).settings().video.fps;
        let gif_fps = self.controller.read(cx).settings().gif.fps;
        let video_label = recording_label(&state, CaptureKind::Video);
        let gif_label = recording_label(&state, CaptureKind::Gif);

        div()
            .id("rapidcap-root")
            .accessibility_id("rapidcap.application")
            .role(Role::Application)
            .aria_label("RapidCap screen capture")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::capture_region))
            .on_action(cx.listener(Self::capture_window))
            .on_action(cx.listener(Self::toggle_video))
            .on_action(cx.listener(Self::toggle_gif))
            .on_action(cx.listener(Self::open_output))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .bg(canvas)
            .text_color(text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("RapidCap"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(secondary)
                            .child(format!("{video_fps} FPS · GIF {gif_fps} FPS")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        button(
                            CONTROL_IDS[0],
                            "rapidcap.capture-region",
                            "RapidCapRegion",
                            "Region",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(CaptureCommand::CaptureRegion, cx)
                        })),
                    )
                    .child(
                        button(
                            CONTROL_IDS[1],
                            "rapidcap.capture-window",
                            "RapidCapWindow",
                            "Window",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(CaptureCommand::CaptureActiveWindow, cx)
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        button(
                            CONTROL_IDS[2],
                            "rapidcap.record-video",
                            "RapidCapVideo",
                            video_label,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(CaptureCommand::ToggleVideo, cx)
                        })),
                    )
                    .child(
                        button(
                            "record-gif",
                            "rapidcap.record-gif",
                            "RapidCapGif",
                            gif_label,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(CaptureCommand::ToggleGif, cx)
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id("capture-status")
                            .role(Role::Status)
                            .aria_label(status.clone())
                            .text_size(px(12.0))
                            .text_color(secondary)
                            .child(status),
                    )
                    .child(
                        button(
                            CONTROL_IDS[4],
                            "rapidcap.open-output",
                            "RapidCapOutput",
                            "Output folder",
                        )
                        .flex_none()
                        .h(px(32.0))
                        .px(px(10.0))
                        .aria_label(format!("Open output folder {output}"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_output(&OpenOutputAction, window, cx)
                        })),
                    ),
            )
    }
}

fn recording_label(state: &CaptureState, kind: CaptureKind) -> &'static str {
    match state {
        CaptureState::Countdown(active, _) if *active == kind => {
            if kind == CaptureKind::Video {
                "Cancel Video"
            } else {
                "Cancel GIF"
            }
        }
        CaptureState::Recording(active)
        | CaptureState::Paused(active)
        | CaptureState::Finalizing(active)
            if *active == kind =>
        {
            if kind == CaptureKind::Video {
                "Stop Video"
            } else {
                "Stop GIF"
            }
        }
        _ if kind == CaptureKind::Video => "Video",
        _ => "GIF",
    }
}

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", TabAction, None),
        KeyBinding::new("shift-tab", TabPrevAction, None),
        KeyBinding::new("enter", RegionAction, Some("RapidCapRegion")),
        KeyBinding::new("space", RegionAction, Some("RapidCapRegion")),
        KeyBinding::new("enter", WindowAction, Some("RapidCapWindow")),
        KeyBinding::new("space", WindowAction, Some("RapidCapWindow")),
        KeyBinding::new("enter", VideoAction, Some("RapidCapVideo")),
        KeyBinding::new("space", VideoAction, Some("RapidCapVideo")),
        KeyBinding::new("enter", GifAction, Some("RapidCapGif")),
        KeyBinding::new("space", GifAction, Some("RapidCapGif")),
        KeyBinding::new("enter", OpenOutputAction, Some("RapidCapOutput")),
        KeyBinding::new("space", OpenOutputAction, Some("RapidCapOutput")),
    ]
}

pub fn open_main_window(
    cx: &mut App,
    controller: Entity<AppController>,
    show: bool,
) -> anyhow::Result<WindowHandle<MainWindow>> {
    let compact_size = size(px(360.0), px(240.0));
    let bounds = Bounds::centered(None, compact_size, cx);
    let handle = cx.open_window(
        WindowOptions {
            focus: show,
            show,
            app_id: Some("com.inspire.rapidcap".into()),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(320.0), px(220.0))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| MainWindow::new(window, cx, controller)),
    )?;
    handle.update(cx, |_view, window, _cx| window.resize(compact_size))?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext, VisualTestContext};
    use rapidcap_capture::{AppPaths, CaptureKind, CaptureState, Settings};

    use super::*;
    use crate::controller::AppController;

    #[test]
    fn primary_controls_have_stable_ids() {
        assert_eq!(
            CONTROL_IDS,
            [
                "capture-region",
                "capture-window",
                "record-video",
                CONTROL_IDS[3],
                CONTROL_IDS[4],
            ]
        );
    }

    #[test]
    fn active_recording_button_becomes_stop() {
        assert_eq!(
            recording_label(
                &CaptureState::Recording(CaptureKind::Video),
                CaptureKind::Video
            ),
            "Stop Video"
        );
        assert_eq!(
            recording_label(&CaptureState::Idle, CaptureKind::Gif),
            "GIF"
        );
    }

    #[gpui::test]
    fn region_action_updates_controller(cx: &mut TestAppContext) {
        let controller = cx.new(|_| {
            AppController::new(
                Settings::default(),
                AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local"),
            )
        });
        let window = cx.update(|cx| {
            let controller = controller.clone();
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| MainWindow::new(window, cx, controller))
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut cx).unwrap();
        let focus = view.read_with(&cx, |view, _| view.focus_handle().clone());

        cx.update(|window, cx| focus.dispatch_action(&RegionAction, window, cx));

        assert_eq!(
            controller.read_with(&cx, |controller, _| controller.state().clone()),
            CaptureState::Selecting(CaptureKind::RegionScreenshot)
        );
    }
}
