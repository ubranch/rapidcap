use gpui::{
    App, Bounds, Context, DispatchPhase, Entity, FocusHandle, FontWeight, KeyBinding, MouseButton,
    MouseMoveEvent, MouseUpEvent, Render, Role, SharedString, Subscription, Toggled, Window,
    WindowBounds, WindowHandle, WindowOptions, actions, canvas, div, prelude::*, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rapidcap_capture::{
    CaptureCommand, CaptureEvent, CaptureFailure, CaptureKind, CaptureState, CaptureTarget,
    SavedOutput,
};

use crate::controller::AppController;
use crate::icons::Icon;
use crate::platform::{
    drag_main_window, hide_main_window, lock_window_size, open_path, place_main_window,
    remember_main_window, shortcut_label, window_drag_grab,
};
use crate::theme;

actions!(
    rapidcap,
    [
        RegionAction,
        WindowAction,
        VideoAction,
        GifAction,
        OpenOutputAction,
        TabAction,
        TabPrevAction,
        HidePanelAction,
        QuitAction
    ]
);

pub const CONTROL_IDS: [&str; 6] = [
    "capture-region",
    "capture-window",
    "record-video",
    "record-gif",
    "open-output",
    "toggle-audio",
];

/// How long the saved chip sits in the footer before the folder chip returns.
///
/// Long enough to read a filename and reach for it, short enough that it is
/// gone before the next capture. A confirmation that has to be dismissed is
/// worse than no confirmation at all.
const SAVED_CHIP: Duration = Duration::from_secs(6);

/// Characters the saved chip can show. 152px of chip, less its padding, icon
/// and border, leaves about 100px of text, and 12px medium runs near six
/// pixels a character.
const SAVED_LABEL_MAX: usize = 16;

/// How much of the tail a trim keeps. `a7Kq.png` is exactly this long: the
/// extension, plus the guard that separates two captures of the same second.
const SAVED_LABEL_TAIL: usize = 8;

/// What the footer shows for the six seconds after a capture lands.
///
/// The summary is built once, when the event arrives, rather than on every
/// render: it stats the file, and the render path runs on every frame.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Saved {
    path: PathBuf,
    /// Replaces the state text in the status well. `00:42 · 12.4 MB` for a
    /// recording, `Copied` for a screenshot.
    summary: String,
}

impl Saved {
    fn new(output: &SavedOutput) -> Self {
        Self {
            path: output.path.clone(),
            summary: saved_summary(output),
        }
    }
}

pub struct MainWindow {
    controller: Entity<AppController>,
    focus_handle: FocusHandle,
    /// Where the cursor grabbed the panel, while a titlebar drag is in flight.
    drag_grab: Option<(i32, i32)>,
    /// The confirmation the footer is currently showing, if one is up.
    saved: Option<Saved>,
    /// Commands whose global chord another app already owned when RapidCap
    /// started. Empty until the platform runtime reports in, which happens
    /// after this window opens.
    unavailable: Vec<CaptureCommand>,
    _controller_subscription: Subscription,
    _event_subscription: Subscription,
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
        // Alt+F4 and the close button do the same thing: quit. `cx.hide()` used
        // to stand in for "put it back in the tray", but it is a no-op on
        // Windows - the panel stayed put and the app had no way out at all. The
        // tray route is the minimise button, which really does hide the window.
        let close_controller = controller.clone();
        window.on_window_should_close(cx, move |_window, cx| {
            close_on_exit_request(&close_controller, cx);
            false
        });
        let event_subscription = cx.subscribe(&controller, |this, _, event: &CaptureEvent, cx| {
            let CaptureEvent::OutputSaved(output) = event else {
                return;
            };
            this.saved = Some(Saved::new(output));
            cx.notify();
            let path = output.path.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SAVED_CHIP).await;
                let _ = this.update(cx, |this, cx| {
                    // A capture that saved inside the six seconds owns the
                    // chip now, and has its own timer running. Only the one
                    // whose file is still showing may take it down.
                    if this.saved.as_ref().map(|saved| saved.path.as_path()) == Some(path.as_path())
                    {
                        this.saved = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        });
        Self {
            controller,
            focus_handle,
            drag_grab: None,
            saved: None,
            unavailable: Vec::new(),
            _controller_subscription: controller_subscription,
            _event_subscription: event_subscription,
        }
    }

    /// Told once at startup, after `PlatformRuntime` has tried to register
    /// every chord. Before that the panel assumes all five landed, which is the
    /// truth on a machine with nothing else bound.
    pub fn set_unavailable_hotkeys(
        &mut self,
        commands: Vec<CaptureCommand>,
        cx: &mut Context<Self>,
    ) {
        self.unavailable = commands;
        cx.notify();
    }

    /// What a card should print under its name.
    fn shortcut(&self, command: CaptureCommand) -> Shortcut {
        Shortcut {
            label: shortcut_label(command).unwrap_or_default(),
            registered: !self.unavailable.contains(&command),
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
        if let Err(error) = open_path(&self.controller.read(cx).paths().capture_root) {
            tracing::error!(%error, "open output folder");
        }
    }

    /// Open the file the saved chip is offering, and take the chip down.
    ///
    /// It has done its job once it has been pressed, and leaving it up for the
    /// rest of the six seconds invites a second click on a file already open.
    fn open_saved(&mut self, cx: &mut Context<Self>) {
        let Some(saved) = self.saved.take() else {
            return;
        };
        cx.notify();
        if let Err(error) = open_path(&saved.path) {
            tracing::error!(%error, path = %saved.path.display(), "open saved capture");
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controller = self.controller.read(cx);
        let state = controller.state().clone();
        let target = controller.target().cloned();
        let video_fps = controller.settings().video.fps;
        let gif_fps = controller.settings().gif.fps;
        let countdown = controller.settings().countdown_seconds;
        let audio = controller.settings().audio.enabled;
        let output = controller.paths().capture_root.display().to_string();
        let folder_label = folder_label(&output);
        let error = controller.error().cloned();
        let saved = self.saved.clone();

        // Idle is what the controller returns to the moment a capture lands, so
        // the well would read "Ready" over the top of the chip that just
        // appeared. For as long as the chip is up, the well belongs to it.
        let status = match &saved {
            Some(saved) => saved.summary.clone(),
            None => status_text(&state, target.as_ref()),
        };
        let dot = status_dot(&state, target.is_some());
        let video_label = recording_label(&state, CaptureKind::Video);
        let gif_label = recording_label(&state, CaptureKind::Gif);
        let recording_video = matches!(&state, CaptureState::Recording(CaptureKind::Video));
        let recording_gif = matches!(&state, CaptureState::Recording(CaptureKind::Gif));

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
            .bg(theme::bg_body())
            .text_color(theme::text_label())
            .child(self.titlebar(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::u(theme::GAP))
                    .p(theme::u(theme::PAD))
                    .child(self.header_row(video_fps, gif_fps, countdown, cx))
                    .child(
                        div()
                            .flex()
                            .gap(theme::u(theme::GAP))
                            .child(
                                mode_card(
                                    CONTROL_IDS[0],
                                    "rapidcap.capture-region",
                                    "RapidCapRegion",
                                    Icon::Region,
                                    region_label(target.as_ref()),
                                    self.shortcut(CaptureCommand::CaptureRegion),
                                    matches!(target, Some(CaptureTarget::Region(_))),
                                    false,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.dispatch(CaptureCommand::CaptureRegion, cx)
                                    },
                                )),
                            )
                            .child(
                                mode_card(
                                    CONTROL_IDS[1],
                                    "rapidcap.capture-window",
                                    "RapidCapWindow",
                                    Icon::Window,
                                    window_label(target.as_ref()),
                                    self.shortcut(CaptureCommand::CaptureActiveWindow),
                                    matches!(target, Some(CaptureTarget::Window { .. })),
                                    false,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.dispatch(CaptureCommand::CaptureActiveWindow, cx)
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(theme::u(theme::GAP))
                            .child(
                                mode_card(
                                    CONTROL_IDS[2],
                                    "rapidcap.record-video",
                                    "RapidCapVideo",
                                    if recording_video {
                                        Icon::Stop
                                    } else {
                                        Icon::Video
                                    },
                                    video_label.to_string(),
                                    self.shortcut(CaptureCommand::ToggleVideo),
                                    false,
                                    recording_video,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.dispatch(CaptureCommand::ToggleVideo, cx),
                                )),
                            )
                            .child(
                                mode_card(
                                    CONTROL_IDS[3],
                                    "rapidcap.record-gif",
                                    "RapidCapGif",
                                    if recording_gif { Icon::Stop } else { Icon::Gif },
                                    gif_label.to_string(),
                                    self.shortcut(CaptureCommand::ToggleGif),
                                    false,
                                    recording_gif,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.dispatch(CaptureCommand::ToggleGif, cx),
                                )),
                            ),
                    )
                    .child(match error {
                        // A failure takes the whole footer: nothing else there
                        // matters until the message has been read.
                        Some(failure) => self.error_bar(failure, cx).into_any_element(),
                        None => div()
                            .flex()
                            .items_center()
                            .gap(theme::u(6.0))
                            .h(theme::u(theme::CHIP_H))
                            .child(
                                chip(
                                    CONTROL_IDS[5],
                                    "rapidcap.toggle-audio",
                                    "RapidCapAudio",
                                    if audio { Icon::AudioOn } else { Icon::AudioOff },
                                    if audio { "System audio" } else { "No audio" }.to_string(),
                                    if audio {
                                        "System audio is on — click to mute recordings".to_string()
                                    } else {
                                        "System audio is off — click to record sound".to_string()
                                    },
                                    Some(audio),
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.controller.update(cx, |controller, cx| {
                                            controller.toggle_audio(cx)
                                        });
                                    },
                                )),
                            )
                            // The confirmation is a control, not a toast: it
                            // lands where the eye already is, it is pressable,
                            // and it expires on its own. It takes the folder
                            // chip's place rather than adding a slot, so the
                            // footer never reflows under the pointer.
                            .child(match &saved {
                                Some(saved) => chip(
                                    CONTROL_IDS[4],
                                    "rapidcap.open-saved",
                                    "RapidCapSaved",
                                    Icon::Saved,
                                    saved_label(&saved.path),
                                    format!("Saved — open {}", saved.path.display()),
                                    None,
                                )
                                .border_color(theme::accent())
                                .on_click(cx.listener(|this, _, _, cx| this.open_saved(cx)))
                                .into_any_element(),
                                None => chip(
                                    CONTROL_IDS[4],
                                    "rapidcap.open-output",
                                    "RapidCapOutput",
                                    Icon::Folder,
                                    folder_label,
                                    format!("Open output folder {output}"),
                                    None,
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_output(&OpenOutputAction, window, cx)
                                }))
                                .into_any_element(),
                            })
                            .child(div().flex_1())
                            .child(status_well(status, dot))
                            .into_any_element(),
                    }),
            )
    }
}

impl MainWindow {
    /// The header, wired: each countdown slot writes `countdown_seconds`.
    fn header_row(
        &self,
        video_fps: u32,
        gif_fps: u32,
        countdown: u8,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut track = div()
            .flex()
            .gap(theme::u(2.0))
            .p(theme::u(theme::SEG_PAD))
            .rounded(theme::u(theme::RADIUS_PILL))
            .bg(theme::bg_track());
        for choice in AppController::COUNTDOWN_CHOICES {
            track = track.child(
                countdown_slot(choice, choice == countdown).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.controller
                            .update(cx, |controller, cx| controller.set_countdown(choice, cx));
                    },
                )),
            );
        }
        header(video_fps, gif_fps, track)
    }

    /// Amber, not red: red already means a capture is running, and an error bar
    /// in the same colour reads as one more recording indicator.
    fn error_bar(&self, failure: CaptureFailure, cx: &mut Context<Self>) -> impl IntoElement {
        let summary = failure.summary;
        // The raw text, verbatim, on the label a screen reader reads and in the
        // tooltip. The bar is 36px and the summary is written to fit it, so
        // these are the only two surfaces the untruncated error has.
        let detail = SharedString::from(failure.detail);
        div()
            .id("capture-error")
            .accessibility_id("rapidcap.error")
            .role(Role::Alert)
            .aria_label(detail.clone())
            .tooltip({
                let detail = detail.clone();
                move |_, cx| cx.new(|_| ErrorDetail(detail.clone())).into()
            })
            .h(theme::u(theme::CHIP_H))
            .pl(theme::u(12.0))
            .pr(theme::u(6.0))
            .flex()
            .items_center()
            .gap(theme::u(theme::GAP))
            .rounded(theme::u(theme::RADIUS_PILL))
            .border_2()
            .border_color(theme::warn())
            .bg(theme::warn_fill())
            .child(
                div()
                    .size(theme::u(7.0))
                    .flex_none()
                    .rounded(theme::u(theme::RADIUS_PILL))
                    .bg(theme::warn()),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_size(theme::u(theme::TEXT_SMALL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::warn_text())
                    .child(summary),
            )
            .child(
                div()
                    .id("dismiss-error")
                    .accessibility_id("rapidcap.error-dismiss")
                    .role(Role::Button)
                    .aria_label("Dismiss")
                    .size(theme::u(26.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(theme::u(theme::RADIUS_PILL))
                    .border_2()
                    .border_color(theme::warn())
                    .text_color(theme::warn_text())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::warn_fill()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.controller
                            .update(cx, |controller, cx| controller.dismiss_error(cx));
                    }))
                    .child(Icon::Close.element(theme::u(13.0), theme::warn_text())),
            )
    }

    /// Feeds the live drag, from the window rather than from the titlebar.
    ///
    /// An element's `on_mouse_move` only fires while the cursor is inside that
    /// element's own hitbox. The window trails the cursor by a frame, so any
    /// quick flick lands the pointer off the 44px strip, the handler stops
    /// firing, and the window sticks where it was until the pointer wanders
    /// back - then it jumps. That is the drag "catching" partway through a
    /// move. Windows has already handed us the pointer through `SetCapture` for
    /// as long as the button is down, so a window-level listener sees every
    /// move, inside the strip or far outside it.
    ///
    /// `canvas` earns its place by being the one element whose paint callback
    /// runs where [`Window::on_mouse_event`] is legal to call. It draws
    /// nothing and takes no space.
    fn drag_listener(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let moved = cx.entity().downgrade();
        let released = moved.clone();
        canvas(
            |_, _, _| {},
            move |_, (), window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = moved.update(cx, |this, _| match this.drag_grab {
                        // A move with the button already up means the release
                        // happened somewhere we never heard about.
                        Some(_) if !event.dragging() => this.drag_grab = None,
                        Some(grab) => drag_main_window(grab),
                        None => {}
                    });
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = released.update(cx, |this, _| this.drag_grab = None);
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    /// Custom titlebar. The strip is the drag surface; the buttons sit above it
    /// so their clicks are not swallowed by the move.
    ///
    /// The strip moves the window itself, from mouse down to mouse up. Neither
    /// of the two routes GPUI offers works here: `start_window_move` is a no-op
    /// on Windows, and a `WindowControlArea::Drag` hands the job to
    /// `DefWindowProc`'s modal move loop, which gets cancelled - see
    /// [`drag_main_window`]. The strip only records the grab; the moving is
    /// driven from [`Self::drag_listener`], which hears the whole gesture.
    fn titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(theme::u(theme::TITLEBAR_H))
            .flex()
            .items_center()
            .bg(theme::bg_titlebar())
            .child(
                div()
                    .id("titlebar-drag")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(theme::u(theme::TITLEBAR_GAP))
                    .pl(theme::u(theme::TITLEBAR_LEADING))
                    .text_color(theme::text_muted())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag_grab = window_drag_grab()),
                    )
                    .child(self.drag_listener(cx))
                    .child(Icon::Mark.element(theme::u(theme::TITLEBAR_GLYPH), theme::text_muted()))
                    .child(
                        div()
                            .text_size(theme::u(theme::TEXT_SMALL))
                            .font_weight(FontWeight::MEDIUM)
                            .child("RapidCap"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    // The bar's own gap, which runs between these two as well.
                    .gap(theme::u(theme::TITLEBAR_GAP))
                    .child(
                        window_button(
                            "titlebar-minimize",
                            "Minimize to tray",
                            Icon::Minimize,
                            false,
                        )
                        .on_click(|_, _, _| hide_main_window()),
                    )
                    .child(
                        window_button("titlebar-close", "Close", Icon::Close, true).on_click(
                            cx.listener(|this, _, _, cx| {
                                close_on_exit_request(&this.controller, cx);
                            }),
                        ),
                    ),
            )
    }
}

/// Brand row. Mark, wordmark and the frame rate badge on the left; the countdown
/// segmented control on the right, its info dot overhanging the track.
///
/// One badge, both rates, video first — the order the cards sit in below it.
/// The rates are fixed and the badge is the only place they are stated, so it
/// carries both. Two separate badges do not fit: the gap between the badge and
/// the countdown track measures 60px, and a second `15 FPS` badge needs 66px
/// with its gap.
fn header(video_fps: u32, gif_fps: u32, countdown: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .mb(theme::u(theme::HEADER_MB))
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme::u(10.0))
                .child(
                    div()
                        .size(theme::u(theme::MARK))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::u(10.0))
                        .bg(theme::text_primary())
                        .child(
                            div()
                                .size(theme::u(theme::MARK_RING))
                                .rounded(theme::u(theme::RADIUS_PILL))
                                .border_3()
                                .border_color(theme::bg_body()),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::u(theme::TEXT_WORDMARK))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::text_primary())
                        .child("RapidCap"),
                )
                .child(badge(format!("{video_fps} / {gif_fps} FPS"))),
        )
        .child(countdown_control(countdown))
}

/// Three slots: no delay, 3s, 5s. The active one is a 2px accent ring — no fill
/// flood, no sliding pill.
fn countdown_control(track: impl IntoElement) -> impl IntoElement {
    div().relative().flex_none().child(track).child(
        div()
            .absolute()
            .left(theme::u(-3.0))
            .top(theme::u(-7.0))
            .size(theme::u(theme::SEG_INFO))
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::u(theme::RADIUS_PILL))
            .border_2()
            .border_color(theme::border_card())
            .bg(theme::bg_pill_off())
            .text_size(theme::u(9.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::text_badge())
            .child("i"),
    )
}

fn countdown_slot(seconds: u8, active: bool) -> gpui::Stateful<gpui::Div> {
    let colour = if active {
        theme::text_primary()
    } else {
        theme::text_muted()
    };
    let id: &'static str = match seconds {
        0 => "countdown-off",
        3 => "countdown-3",
        _ => "countdown-5",
    };
    let slot = div()
        .id(id)
        .accessibility_id(id)
        .role(Role::RadioButton)
        .aria_label(if seconds == 0 {
            "No countdown".to_string()
        } else {
            format!("{seconds} second countdown")
        })
        .focusable()
        .tab_stop(true)
        .size(theme::u(theme::SEGMENT))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .border_2()
        .border_color(if active {
            theme::accent()
        } else {
            theme::border_card()
        })
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colour)
        .cursor_pointer()
        .hover(|style| style.text_color(theme::text_label()));

    if seconds == 0 {
        slot.child(Icon::Instant.element(theme::u(16.0), colour))
    } else {
        slot.child(format!("{seconds}"))
    }
}

fn badge(label: String) -> impl IntoElement {
    div()
        .h(theme::u(theme::BADGE_H))
        .px(theme::u(9.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::bg_track())
        .text_size(theme::u(theme::TEXT_MICRO))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_badge())
        .child(label)
}

/// A command's global chord, and whether the OS actually handed it over.
///
/// `RegisterHotKey` is first-come-first-served, so a chord that ShareX - or any
/// other running app - claimed first simply fails for RapidCap. The card keeps
/// working, because it is a button and not a key handler, but printing the
/// chord in plain grey beside it promises a key that will never fire.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Shortcut {
    label: String,
    registered: bool,
}

impl Shortcut {
    /// `Alt+E` when the chord is ours, `Alt+E · taken` when it is not.
    /// Amber alone would not say what is wrong, and colour is not a word.
    fn text(&self) -> String {
        if self.registered {
            self.label.clone()
        } else {
            format!("{} · taken", self.label)
        }
    }
}

/// A capture target button. `armed` marks the current selection, `recording`
/// turns the card red — the card is the state indicator, there is no badge.
#[allow(clippy::too_many_arguments)]
fn mode_card(
    id: &'static str,
    accessibility_id: &'static str,
    context: &'static str,
    icon: Icon,
    label: String,
    shortcut: Shortcut,
    armed: bool,
    recording: bool,
) -> gpui::Stateful<gpui::Div> {
    // A chord another app owns must not be announced. A screen reader that
    // reads it out is offering the user a key that does nothing.
    let announced = if shortcut.registered {
        format!("Enter Space {}", shortcut.label)
    } else {
        "Enter Space".to_string()
    };
    let content = if recording {
        theme::rec()
    } else {
        theme::text_label()
    };
    div()
        .id(id)
        .accessibility_id(accessibility_id)
        .key_context(context)
        .focusable()
        .tab_stop(true)
        .role(Role::Button)
        .aria_label(label.clone())
        .aria_keyshortcuts(announced)
        .flex_1()
        .relative()
        .overflow_hidden()
        .h(theme::u(theme::CARD_H))
        // Icon on the left, name over shortcut on the right. Stacking all three
        // down the middle of a 64px card left every row cramped and the card
        // bottom-heavy; a row has width to spare and gives the name and its
        // shortcut a shared left edge to read down.
        .flex()
        .items_center()
        .gap(theme::u(11.0))
        .pl(theme::u(14.0))
        .pr(theme::u(10.0))
        .rounded(theme::u(theme::RADIUS))
        .border_2()
        .border_color(if armed {
            theme::accent()
        } else {
            theme::border_card()
        })
        .bg(if armed {
            theme::accent_fill()
        } else {
            theme::bg_card()
        })
        .text_color(content)
        .cursor_pointer()
        .hover(move |style| style.bg(theme::bg_hover()))
        .focus_visible(move |style| style.border_color(theme::accent()))
        .child(icon.element(theme::u(22.0), content))
        .child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(theme::u(3.0))
                .child(
                    div()
                        .text_size(theme::u(theme::TEXT_BODY))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .child(shortcut_pill(shortcut)),
        )
}

/// The global shortcut, printed under the card's name.
///
/// Left-aligned under the name rather than floating in the card's top right
/// corner: `Ctrl+Shift+W` came within two pixels of the icon up there, and a
/// longer binding would have run straight through it.
fn shortcut_pill(shortcut: Shortcut) -> gpui::Div {
    div()
        .flex_none()
        // Pulled left by its own padding so the shortcut text lines up with the
        // name above it. Aligning the chip's box instead left the text inside it
        // visibly indented.
        .ml(theme::u(-5.0))
        .px(theme::u(5.0))
        .h(theme::u(15.0))
        .flex()
        .items_center()
        .rounded(theme::u(theme::pill_radius(15.0)))
        .bg(theme::bg_pill_off())
        .text_size(theme::u(theme::TEXT_MICRO))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_muted())
        // Amber ring and amber text, the pair the settings mock draws for a
        // clashing binding. Applied after the muted default so it wins.
        .when(!shortcut.registered, |pill| {
            pill.border_1()
                .border_color(theme::warn())
                .text_color(theme::warn_text())
        })
        .child(shortcut.text())
}

/// A raised pill. Raised because it is pressable — see `status_well` for the
/// read-only counterpart.
/// `toggled` is `None` for a chip that performs an action and `Some` for one
/// that carries a state, which is why it is not a plain `bool`: an action chip
/// is neither on nor off, and painting it as "off" would claim it is a toggle
/// nobody has switched on yet.
fn chip(
    id: &'static str,
    accessibility_id: &'static str,
    context: &'static str,
    icon: Icon,
    label: String,
    aria: String,
    toggled: Option<bool>,
) -> gpui::Stateful<gpui::Div> {
    let icon_colour = if toggled == Some(true) {
        theme::accent_text()
    } else {
        theme::text_muted()
    };
    let label_colour = if toggled == Some(false) {
        theme::text_off()
    } else {
        theme::text_label()
    };
    div()
        .id(id)
        .accessibility_id(accessibility_id)
        .key_context(context)
        .focusable()
        .tab_stop(true)
        .role(Role::Button)
        .aria_label(aria)
        .aria_keyshortcuts("Enter Space")
        .when_some(toggled, |this, on| {
            this.aria_toggled(if on { Toggled::True } else { Toggled::False })
        })
        .flex_none()
        .h(theme::u(theme::CHIP_H))
        .max_w(theme::u(theme::CHIP_MAX_W))
        .px(theme::u(12.0))
        .flex()
        .items_center()
        .gap(theme::u(7.0))
        .rounded(theme::u(theme::RADIUS_PILL))
        .border_2()
        .relative()
        .border_color(theme::border_card())
        .bg(theme::bg_card())
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(label_colour)
        .cursor_pointer()
        .hover(|style| style.bg(theme::bg_hover()))
        .focus_visible(|style| style.border_color(theme::accent()))
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .text_color(icon_colour)
                .child(icon.element(theme::u(16.0), icon_colour)),
        )
        .child(div().overflow_hidden().child(label))
}

/// Read-only, so recessed and never focusable: an inner shadow under the shared
/// border, and 8px shorter than the chips beside it.
fn status_well(status: String, dot: gpui::Hsla) -> impl IntoElement {
    div()
        .id("capture-status")
        .role(Role::Status)
        .aria_label(status.clone())
        .flex_none()
        .h(theme::u(theme::STATUS_H))
        .px(theme::u(12.0))
        .flex()
        .items_center()
        .gap(theme::u(7.0))
        .rounded(theme::u(theme::RADIUS_PILL))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::bg_card())
        .shadow(theme::recessed())
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_pill())
        .child(
            div()
                .size(theme::u(7.0))
                .flex_none()
                .rounded(theme::u(theme::RADIUS_PILL))
                .bg(dot),
        )
        .child(status)
}

/// The close fill is deliberately square. Windows 11 already clips the panel to
/// its own corner radius, so rounding the button too only subtracts: our 12px
/// arc sat inside DWM's 8px one, and the lune between them painted neither the
/// hover red nor the desktop - it showed the bare titlebar. Square, the red runs
/// out to DWM's clip and the corner is red all the way into it.
///
/// Borderless: the titlebar already has an edge, and a second one around each
/// glyph reads as a nested frame. Hover fill is what separates the buttons.
fn window_button(
    id: &'static str,
    label: &'static str,
    icon: Icon,
    close: bool,
) -> gpui::Stateful<gpui::Div> {
    let hover_bg = if close {
        theme::danger()
    } else {
        theme::bg_titlebar_hover()
    };
    div()
        .id(id)
        .accessibility_id(id)
        .role(Role::Button)
        .aria_label(label)
        .w(theme::u(theme::WIN_BTN_W))
        .h(theme::u(theme::TITLEBAR_H))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .text_color(theme::text_muted())
        .cursor_pointer()
        // One `hover` call per element: GPUI panics on a second.
        .hover(move |style| style.bg(hover_bg).text_color(theme::text_primary()))
        .child(icon.element(theme::u(theme::TITLEBAR_GLYPH), theme::text_muted()))
}

/// The one place that decides what Alt+F4 and the close button do.
///
/// Both used to call `cx.quit()` outright, which killed a running recording
/// mid-take and left the encoder's part file behind — the tray's Exit item has
/// always refused to quit under a live capture, and these two disagreed with
/// it. Under a live capture, close now does what the tray does when the panel
/// is in the way: hides it. The recording keeps running and the tray icon keeps
/// carrying its state, so there is still a way to stop the take and then a way
/// out.
pub fn close_on_exit_request(controller: &Entity<AppController>, cx: &mut App) {
    if controller.read(cx).state().blocks_exit() {
        hide_main_window();
    } else {
        cx.quit();
    }
}

/// The raw failure text, on hover over the error bar.
///
/// GPUI has the tooltip machinery but ships no tooltip view - Zed's lives in
/// its own crate - so the panel brings its own. Deliberately unbounded in
/// height: an FFmpeg failure is several lines and clipping it here would put
/// the detail nowhere at all.
struct ErrorDetail(SharedString);

impl Render for ErrorDetail {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(theme::u(320.0))
            .px(theme::u(10.0))
            .py(theme::u(7.0))
            .rounded(theme::u(theme::RADIUS))
            .border_1()
            .border_color(theme::border_card())
            .bg(theme::bg_card())
            .shadow(theme::floating())
            .text_size(theme::u(theme::TEXT_SMALL))
            .text_color(theme::text_primary())
            .child(self.0.clone())
    }
}

/// Written strings, one per state. Never `format!("{state:?}")` — a user should
/// not be able to read a Rust type off the panel.
fn status_text(state: &CaptureState, target: Option<&CaptureTarget>) -> String {
    match state {
        CaptureState::Idle => match target {
            Some(CaptureTarget::Region(region)) => {
                format!("Selected {} × {}", region.width, region.height)
            }
            Some(CaptureTarget::Window { process_name, .. }) => {
                format!("Selected {process_name}")
            }
            None => "Ready".to_string(),
        },
        CaptureState::Selecting(_) => "Selecting · Esc to cancel".to_string(),
        CaptureState::Countdown(kind, seconds) => {
            format!("{} starts in {seconds}", kind_noun(*kind))
        }
        CaptureState::Recording(kind) => format!("Recording {}", kind_noun(*kind)),
        CaptureState::Paused(kind) => format!("Paused {}", kind_noun(*kind)),
        CaptureState::Finalizing(kind) => format!("Finalizing {}…", kind_noun(*kind)),
        CaptureState::Error(failure) => failure.summary.clone(),
    }
}

/// Red while capturing, accent once a target is armed, otherwise neutral.
fn status_dot(state: &CaptureState, has_target: bool) -> gpui::Hsla {
    match state {
        CaptureState::Recording(_) | CaptureState::Countdown(_, _) => theme::rec(),
        CaptureState::Paused(_) | CaptureState::Finalizing(_) => theme::text_pill(),
        CaptureState::Error(_) => theme::warn(),
        CaptureState::Selecting(_) => theme::accent(),
        CaptureState::Idle if has_target => theme::accent(),
        CaptureState::Idle => theme::text_muted(),
    }
}

fn kind_noun(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Video => "Video",
        CaptureKind::Gif => "GIF",
        CaptureKind::RegionScreenshot => "Region",
        CaptureKind::ActiveWindowScreenshot => "Window",
    }
}

/// The armed region replaces the label with its size — the card carries the
/// selection instead of a separate readout.
fn region_label(target: Option<&CaptureTarget>) -> String {
    match target {
        Some(CaptureTarget::Region(region)) => format!("{} × {}", region.width, region.height),
        _ => "Region".to_string(),
    }
}

fn window_label(target: Option<&CaptureTarget>) -> String {
    match target {
        Some(CaptureTarget::Window { process_name, .. }) => process_name.clone(),
        _ => "Window".to_string(),
    }
}

/// Last path segment. The chip is 152px wide; a full path never fits and the
/// tail is the part that identifies it.
fn folder_label(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// The filename, for the saved chip. Falls back to the whole path rather than
/// to an empty chip: a path with no final component is not a file we saved, so
/// showing it is more use than showing nothing.
///
/// Names carry a timestamp now, which does not fit a 152px chip. The chip is
/// `overflow_hidden`, so an untrimmed name is clipped mid-character and loses
/// its extension - the one part that says what was captured. Trim from the
/// middle instead: the process name survives at the front, the extension and
/// the collision guard at the back, and the tooltip still carries the whole
/// path.
fn saved_label(path: &Path) -> String {
    let Some(name) = path.file_name() else {
        return path.display().to_string();
    };
    let name = name.to_string_lossy();
    let count = name.chars().count();
    if count <= SAVED_LABEL_MAX {
        return name.into_owned();
    }
    let head: String = name
        .chars()
        .take(SAVED_LABEL_MAX - 1 - SAVED_LABEL_TAIL)
        .collect();
    let tail: String = name.chars().skip(count - SAVED_LABEL_TAIL).collect();
    format!("{head}…{tail}")
}

/// What the status well reads while the chip is up.
///
/// A recording gets its length and its size, the two things checked before a
/// clip is sent anywhere. A screenshot has no length, so it reports the
/// clipboard instead - that is the part of a screenshot a user acts on next.
/// A file that cannot be stat'd reports only what is still known rather than
/// inventing a size.
fn saved_summary(output: &SavedOutput) -> String {
    let size = std::fs::metadata(&output.path)
        .map(|meta| meta.len())
        .ok()
        .map(file_size);
    match (output.recorded, size) {
        (Some(recorded), Some(size)) => format!("{} · {size}", clock(recorded)),
        (Some(recorded), None) => clock(recorded),
        (None, _) if output.copied => "Copied".to_string(),
        (None, Some(size)) => format!("Saved · {size}"),
        (None, None) => "Saved".to_string(),
    }
}

/// `00:42`, and `1:02:07` once a recording passes an hour. Minutes and seconds
/// are always two digits so the well does not reflow while it is being read.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// `12.4 MB`, in the units and to the precision Explorer uses, so the number
/// matches what the user sees when they go looking for the file.
///
/// Three significant digits: `1.00 KB`, `12.4 MB`, `123 MB`. That keeps the
/// well a near-constant width without dropping the digit that distinguishes a
/// clip worth sending from one that is not.
fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    let decimals = if size < 10.0 {
        2
    } else if size < 100.0 {
        1
    } else {
        0
    };
    format!("{size:.decimals$} {}", UNITS[unit])
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
        // Command-W and Command-Q, the two chords every Mac user tries first.
        // They are not Windows chords - Alt+F4 is the close there, and it is
        // handled by the window manager rather than by a binding - so this pair
        // is macOS-only. The menu bar built from these in `main` is what puts
        // the shortcut text next to the items and what makes the keys fire at
        // all: AppKit routes Command chords through the main menu before any
        // window sees them.
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-w", HidePanelAction, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-q", QuitAction, None),
    ]
}

/// The panel's `HWND`, straight from GPUI. The platform helpers need it to
/// place, pin, hide and drag a window GPUI will not do those things to itself.
fn panel_hwnd(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(isize::from(handle.hwnd)),
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as isize),
        _ => None,
    }
}

pub fn open_main_window(
    cx: &mut App,
    controller: Entity<AppController>,
    show: bool,
) -> anyhow::Result<WindowHandle<MainWindow>> {
    let compact_size = size(theme::u(theme::PANEL_W), theme::u(theme::PANEL_H));
    let bounds = Bounds::centered(None, compact_size, cx);
    let handle = cx.open_window(
        WindowOptions {
            focus: show,
            show,
            app_id: Some("com.inspire.rapidcap".into()),
            // `appears_transparent` hides the system titlebar so the custom one
            // can draw. `titlebar: None` does the same on paper, but on Windows
            // it creates a window that never becomes visible.
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("RapidCap".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| MainWindow::new(window, cx, controller)),
    )?;
    handle.update(cx, |_view, window, _cx| window.resize(compact_size))?;
    // Fixed size. Every metric in the design system is absolute, so a resizable
    // panel can only be the right layout at one width - and the error bar takes
    // over the footer slot instead of adding a row, so no state needs more room.
    //
    // The panel is also placed by hand: GPUI's own initial bounds are a coin
    // toss here, see `place_main_window`. Queued rather than called straight
    // out, because `Window::resize` above defers its `SetWindowPos` onto the
    // foreground executor - place the panel before that runs and GPUI's
    // half-screen default wins the race about one launch in five.
    if let Some(hwnd) = handle.update(cx, |_view, window, _cx| panel_hwnd(window))? {
        remember_main_window(hwnd);
        place_main_window(theme::scaled(theme::PANEL_W), theme::scaled(theme::PANEL_H));
        lock_window_size();
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext, VisualTestContext};
    use rapidcap_capture::{
        AppPaths, CaptureKind, CaptureState, PhysicalRegion, SavedCapture, Settings,
    };

    use super::*;
    use crate::controller::AppController;

    fn saved_path(view: &MainWindow) -> Option<PathBuf> {
        view.saved.as_ref().map(|saved| saved.path.clone())
    }

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
                "toggle-audio",
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

    #[test]
    fn no_state_renders_a_rust_debug_string() {
        // The regression this guards shipped for real: every non-idle state fell
        // through to `format!("{state:?}")`, so the panel read `Recording(Video)`.
        let region = PhysicalRegion {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        let states = [
            CaptureState::Idle,
            CaptureState::Selecting(CaptureKind::RegionScreenshot),
            CaptureState::Countdown(CaptureKind::Video, 3),
            CaptureState::Recording(CaptureKind::Video),
            CaptureState::Paused(CaptureKind::Gif),
            CaptureState::Finalizing(CaptureKind::Video),
            CaptureState::Error(CaptureFailure::new("Recording", "disk full")),
        ];
        for state in states {
            for target in [None, Some(CaptureTarget::Region(region.clone()))] {
                let text = status_text(&state, target.as_ref());
                assert!(!text.is_empty(), "{state:?} produced an empty status");
                for debris in ['(', ')', '{', '}', '"'] {
                    assert!(
                        !text.contains(debris),
                        "{state:?} leaked debug syntax: {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn armed_target_replaces_the_card_label() {
        let region = PhysicalRegion {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        assert_eq!(region_label(None), "Region");
        assert_eq!(
            region_label(Some(&CaptureTarget::Region(region))),
            "1280 × 720"
        );
        assert_eq!(window_label(None), "Window");
    }

    #[test]
    fn folder_chip_shows_the_tail_of_the_path() {
        assert_eq!(
            folder_label("C:\\Users\\me\\Documents\\RapidCap"),
            "RapidCap"
        );
        assert_eq!(folder_label("C:/Users/me/Captures/"), "Captures");
        assert_eq!(folder_label("RapidCap"), "RapidCap");
    }

    #[test]
    fn a_chord_another_app_owns_says_so_in_words() {
        // Amber is the signal, but amber is not a word: a user who cannot pick
        // the colour out still has to learn the key is dead.
        let ours = Shortcut {
            label: "Alt+E".to_string(),
            registered: true,
        };
        let theirs = Shortcut {
            registered: false,
            ..ours.clone()
        };
        assert_eq!(ours.text(), "Alt+E");
        assert_eq!(theirs.text(), "Alt+E · taken");
    }

    #[gpui::test]
    fn a_clash_marks_only_the_card_that_lost_its_chord(cx: &mut TestAppContext) {
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

        // Before the runtime reports, every chord is assumed to be ours.
        view.read_with(&cx, |view, _| {
            assert!(view.shortcut(CaptureCommand::CaptureRegion).registered);
        });

        view.update(&mut cx, |view, cx| {
            view.set_unavailable_hotkeys(vec![CaptureCommand::CaptureRegion], cx);
        });

        view.read_with(&cx, |view, _| {
            let lost = view.shortcut(CaptureCommand::CaptureRegion);
            let kept = view.shortcut(CaptureCommand::ToggleVideo);
            assert!(!lost.registered);
            assert!(kept.registered);
            // The chord still prints - the user needs to know which key it is
            // that another app took.
            assert!(lost.text().starts_with(&lost.label));
            assert_ne!(lost.text(), lost.label);
            assert_eq!(kept.text(), kept.label);
        });
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

    #[test]
    fn saved_chip_shows_the_filename() {
        // Joined rather than written out: a backslash separates components on
        // Windows and is an ordinary filename character everywhere else, so a
        // literal Windows path has no final component at all on macOS.
        let nested = Path::new("Documents")
            .join("RapidCap")
            .join("Screen_9EN.png");
        assert_eq!(saved_label(&nested), "Screen_9EN.png");
        assert_eq!(saved_label(Path::new("Screen_JI2.mp4")), "Screen_JI2.mp4");
        // No final component, so there is nothing to shorten to.
        assert_eq!(saved_label(Path::new("/")), "/");
    }

    #[test]
    fn a_timestamped_name_is_trimmed_from_the_middle() {
        // The process name and the extension identify the capture, so both ends
        // survive and the timestamp is what gives way.
        assert_eq!(
            saved_label(Path::new("Screen_2026-08-27_14-32-05_a7Kq.png")),
            "Screen_…a7Kq.png"
        );
        assert_eq!(
            saved_label(Path::new(
                "C:/Captures/2026-08/Microsoft.Photos_2026-08-27_14-32-05_a7Kq.png"
            )),
            "Microso…a7Kq.png"
        );
        assert_eq!(
            saved_label(Path::new("Screen_2026-08-27_14-32-05_a7Kq.png"))
                .chars()
                .count(),
            SAVED_LABEL_MAX
        );
    }

    #[test]
    fn a_recording_reports_its_length_and_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Screen_2026-08-27_14-32-05_Zq3M.mp4");
        std::fs::write(&path, vec![0_u8; 13_002_342]).unwrap();

        assert_eq!(
            saved_summary(&SavedOutput {
                path,
                recorded: Some(Duration::from_secs(42)),
                copied: true,
            }),
            "00:42 · 12.4 MB"
        );
    }

    #[test]
    fn a_screenshot_reports_the_clipboard_instead() {
        let output = |copied| SavedOutput {
            path: PathBuf::from("C:/nowhere/Screen_2026-08-27_14-32-05_a7Kq.png"),
            recorded: None,
            copied,
        };
        assert_eq!(saved_summary(&output(true)), "Copied");
        // Nothing to stat and nothing on the clipboard, so the well says only
        // the one thing still known to be true.
        assert_eq!(saved_summary(&output(false)), "Saved");
    }

    #[test]
    fn a_recording_whose_file_vanished_still_reports_its_length() {
        assert_eq!(
            saved_summary(&SavedOutput {
                path: PathBuf::from("C:/nowhere/gone.mp4"),
                recorded: Some(Duration::from_secs(3727)),
                copied: false,
            }),
            "1:02:07"
        );
    }

    #[test]
    fn the_clock_pads_to_a_stable_width() {
        assert_eq!(clock(Duration::ZERO), "00:00");
        assert_eq!(clock(Duration::from_secs(9)), "00:09");
        assert_eq!(clock(Duration::from_secs(600)), "10:00");
        assert_eq!(clock(Duration::from_secs(3599)), "59:59");
        // The hour is the one field that may widen, because padding it would
        // make every short recording read like a long one.
        assert_eq!(clock(Duration::from_secs(3600)), "1:00:00");
    }

    #[test]
    fn file_sizes_read_the_way_explorer_writes_them() {
        assert_eq!(file_size(0), "0 B");
        assert_eq!(file_size(1023), "1023 B");
        assert_eq!(file_size(1024), "1.00 KB");
        assert_eq!(file_size(13_002_342), "12.4 MB");
        // Three digits throughout, so the decimals give way rather than the
        // number growing wider.
        assert_eq!(file_size(104_857_600), "100 MB");
        assert_eq!(file_size(3_221_225_472), "3.00 GB");
    }

    #[gpui::test]
    fn a_saved_capture_holds_the_chip_for_six_seconds(cx: &mut TestAppContext) {
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
        assert_eq!(view.read_with(&cx, |view, _| saved_path(view)), None);

        let path = PathBuf::from("C:/Documents/RapidCap/Screen_9EN.png");
        controller.update(&mut cx, |controller, cx| {
            controller.finish_screenshot(
                Ok(SavedCapture {
                    path: path.clone(),
                    rgba: Vec::new(),
                    width: 1,
                    height: 1,
                }),
                true,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(&cx, |view, _| saved_path(view)),
            Some(path),
            "a save left no chip behind"
        );

        cx.executor()
            .advance_clock(SAVED_CHIP + Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(
            view.read_with(&cx, |view, _| saved_path(view)),
            None,
            "the chip outlived its timer"
        );
    }
}
