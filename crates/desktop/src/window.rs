use gpui::{
    App, Bounds, Context, DispatchPhase, Entity, FocusHandle, FontWeight, KeyBinding, MouseButton,
    MouseMoveEvent, MouseUpEvent, Render, Role, Subscription, Toggled, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, canvas, div, prelude::*, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState, CaptureTarget};

use crate::controller::AppController;
use crate::icons::Icon;
use crate::platform::{
    drag_main_window, hide_main_window, lock_window_size, open_folder, place_main_window,
    remember_main_window, set_keep_on_top, shortcut_label, window_drag_grab,
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
        TabPrevAction
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

pub struct MainWindow {
    controller: Entity<AppController>,
    focus_handle: FocusHandle,
    keep_on_top: bool,
    /// Where the cursor grabbed the panel, while a titlebar drag is in flight.
    drag_grab: Option<(i32, i32)>,
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
        // Alt+F4 and the close button do the same thing: quit. `cx.hide()` used
        // to stand in for "put it back in the tray", but it is a no-op on
        // Windows - the panel stayed put and the app had no way out at all. The
        // tray route is the minimise button, which really does hide the window.
        window.on_window_should_close(cx, |_window, cx| {
            cx.quit();
            false
        });
        Self {
            controller,
            focus_handle,
            keep_on_top: false,
            drag_grab: None,
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
        let error = controller.error().map(str::to_owned);

        let status = status_text(&state, target.as_ref());
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
                    .gap(px(theme::GAP))
                    .p(px(theme::PAD))
                    .child(self.header_row(video_fps, gif_fps, countdown, cx))
                    .child(
                        div()
                            .flex()
                            .gap(px(theme::GAP))
                            .child(
                                mode_card(
                                    CONTROL_IDS[0],
                                    "rapidcap.capture-region",
                                    "RapidCapRegion",
                                    Icon::Region,
                                    region_label(target.as_ref()),
                                    shortcut_label(CaptureCommand::CaptureRegion)
                                        .unwrap_or_default(),
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
                                    shortcut_label(CaptureCommand::CaptureActiveWindow)
                                        .unwrap_or_default(),
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
                            .gap(px(theme::GAP))
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
                                    shortcut_label(CaptureCommand::ToggleVideo).unwrap_or_default(),
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
                                    shortcut_label(CaptureCommand::ToggleGif).unwrap_or_default(),
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
                        Some(message) => self.error_bar(message, cx).into_any_element(),
                        None => div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .h(px(theme::CHIP_H))
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
                            .child(
                                chip(
                                    CONTROL_IDS[4],
                                    "rapidcap.open-output",
                                    "RapidCapOutput",
                                    Icon::Folder,
                                    folder_label,
                                    format!("Open output folder {output}"),
                                    None,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_output(&OpenOutputAction, window, cx)
                                    },
                                )),
                            )
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
            .gap(px(2.0))
            .p(px(theme::SEG_PAD))
            .rounded(px(theme::RADIUS_PILL))
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
    fn error_bar(&self, message: String, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("capture-error")
            .accessibility_id("rapidcap.error")
            .role(Role::Alert)
            .aria_label(message.clone())
            .h(px(theme::CHIP_H))
            .pl(px(12.0))
            .pr(px(6.0))
            .flex()
            .items_center()
            .gap(px(theme::GAP))
            .rounded(px(theme::RADIUS_PILL))
            .border_2()
            .border_color(theme::warn())
            .bg(theme::warn_fill())
            .child(
                div()
                    .size(px(7.0))
                    .flex_none()
                    .rounded(px(theme::RADIUS_PILL))
                    .bg(theme::warn()),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_size(theme::TEXT_SMALL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::warn_text())
                    .child(error_summary(&message)),
            )
            .child(
                div()
                    .id("dismiss-error")
                    .accessibility_id("rapidcap.error-dismiss")
                    .role(Role::Button)
                    .aria_label("Dismiss")
                    .size(px(26.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme::RADIUS_PILL))
                    .border_2()
                    .border_color(theme::warn())
                    .text_color(theme::warn_text())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::warn_fill()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.controller
                            .update(cx, |controller, cx| controller.dismiss_error(cx));
                    }))
                    .child(Icon::Close.element(px(13.0), theme::warn_text())),
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
            .h(px(theme::TITLEBAR_H))
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
                    .gap(px(8.0))
                    .pl(px(10.0))
                    .text_color(theme::text_muted())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag_grab = window_drag_grab()),
                    )
                    .child(self.drag_listener(cx))
                    .child(Icon::Mark.element(px(16.0), theme::text_muted()))
                    .child(
                        div()
                            .text_size(theme::TEXT_SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .child("RapidCap"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        action_button(
                            "titlebar-keep-on-top",
                            "Keep on top",
                            Icon::KeepOnTop,
                            self.keep_on_top,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.keep_on_top = !this.keep_on_top;
                            set_keep_on_top(this.keep_on_top);
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .w(px(1.0))
                            .h(px(20.0))
                            .flex_none()
                            .mx(px(theme::TITLEBAR_GAP))
                            .bg(theme::border_titlebar()),
                    )
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
                        window_button("titlebar-close", "Close", Icon::Close, true)
                            .on_click(cx.listener(|_, _, _, cx| cx.quit())),
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
        .mb(px(theme::HEADER_MB))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .size(px(theme::MARK))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.0))
                        .bg(theme::text_primary())
                        .child(
                            div()
                                .size(px(theme::MARK_RING))
                                .rounded(px(theme::RADIUS_PILL))
                                .border_3()
                                .border_color(theme::bg_body()),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::TEXT_WORDMARK)
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
            .left(px(-3.0))
            .top(px(-7.0))
            .size(px(theme::SEG_INFO))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS_PILL))
            .border_2()
            .border_color(theme::border_card())
            .bg(theme::bg_pill_off())
            .text_size(px(9.0))
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
        .size(px(theme::SEGMENT))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_PILL))
        .border_2()
        .border_color(if active {
            theme::accent()
        } else {
            theme::border_card()
        })
        .text_size(theme::TEXT_SMALL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colour)
        .cursor_pointer()
        .hover(|style| style.text_color(theme::text_label()));

    if seconds == 0 {
        slot.child(Icon::Instant.element(px(16.0), colour))
    } else {
        slot.child(format!("{seconds}"))
    }
}

fn badge(label: String) -> impl IntoElement {
    div()
        .h(px(theme::BADGE_H))
        .px(px(9.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(theme::RADIUS_PILL))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::bg_track())
        .text_size(theme::TEXT_MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_badge())
        .child(label)
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
    shortcut: String,
    armed: bool,
    recording: bool,
) -> gpui::Stateful<gpui::Div> {
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
        .aria_keyshortcuts(format!("Enter Space {shortcut}"))
        .flex_1()
        .relative()
        .overflow_hidden()
        .h(px(theme::CARD_H))
        // Icon on the left, name over shortcut on the right. Stacking all three
        // down the middle of a 64px card left every row cramped and the card
        // bottom-heavy; a row has width to spare and gives the name and its
        // shortcut a shared left edge to read down.
        .flex()
        .items_center()
        .gap(px(11.0))
        .pl(px(14.0))
        .pr(px(10.0))
        .rounded(px(theme::RADIUS))
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
        .child(icon.element(px(22.0), content))
        .child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(theme::TEXT_BODY)
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
fn shortcut_pill(shortcut: String) -> gpui::Div {
    div()
        .flex_none()
        // Pulled left by its own padding so the shortcut text lines up with the
        // name above it. Aligning the chip's box instead left the text inside it
        // visibly indented.
        .ml(px(-5.0))
        .px(px(5.0))
        .h(px(15.0))
        .flex()
        .items_center()
        .rounded(px(theme::pill_radius(15.0)))
        .bg(theme::bg_pill_off())
        .text_size(theme::TEXT_MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_muted())
        .child(shortcut)
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
        .h(px(theme::CHIP_H))
        .max_w(px(theme::CHIP_MAX_W))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(theme::RADIUS_PILL))
        .border_2()
        .relative()
        .border_color(theme::border_card())
        .bg(theme::bg_card())
        .text_size(theme::TEXT_SMALL)
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
                .child(icon.element(px(16.0), icon_colour)),
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
        .h(px(theme::STATUS_H))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(theme::RADIUS_PILL))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::bg_card())
        .shadow(theme::recessed())
        .text_size(theme::TEXT_SMALL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_pill())
        .child(
            div()
                .size(px(7.0))
                .flex_none()
                .rounded(px(theme::RADIUS_PILL))
                .bg(dot),
        )
        .child(status)
}

/// Titlebar app action: 36 x 36 with a rounded hover pill, 4px from its
/// neighbour — the same 4px a 36px button leaves above and below it in a 44px
/// bar. Borderless on purpose: a box around a titlebar glyph reads as a second
/// window frame. Pressed state is an accent tint on the fill instead.
fn action_button(
    id: &'static str,
    label: &'static str,
    icon: Icon,
    pressed: bool,
) -> gpui::Stateful<gpui::Div> {
    let colour = if pressed {
        theme::accent_text()
    } else {
        theme::text_muted()
    };
    div()
        .id(id)
        .accessibility_id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(theme::TITLEBAR_BTN))
        .ml(px(theme::TITLEBAR_GAP))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS))
        .when(pressed, |this| this.bg(theme::accent_fill()))
        .text_color(colour)
        .cursor_pointer()
        .hover(move |style| {
            style
                .bg(theme::bg_titlebar_hover())
                .text_color(theme::text_primary())
        })
        .child(icon.element(px(18.0), colour))
}

/// Full bar height so the top screen edge — and, for close, the corner — is a
/// valid click.
///
/// The close fill is deliberately square. Windows 11 already clips the panel to
/// its own corner radius, so rounding the button too only subtracts: our 12px
/// arc sat inside DWM's 8px one, and the lune between them painted neither the
/// hover red nor the desktop - it showed the bare titlebar. Square, the red runs
/// out to DWM's clip and the corner is red all the way into it.
///
/// Borderless, like the app actions beside it: the titlebar already has an
/// edge, and a second one around each glyph reads as a nested frame. Hover fill
/// is what separates the buttons.
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
        .w(px(theme::WIN_BTN_W))
        .h(px(theme::TITLEBAR_H))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .text_color(theme::text_muted())
        .cursor_pointer()
        // One `hover` call per element: GPUI panics on a second.
        .hover(move |style| style.bg(hover_bg).text_color(theme::text_primary()))
        .child(icon.element(px(16.0), theme::text_muted()))
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
        CaptureState::Error(message) => error_summary(message),
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

/// First line only, capped. The full text belongs in a detail surface, not in a
/// 28px well — see the Errors card.
fn error_summary(message: &str) -> String {
    let first = message.lines().next().unwrap_or(message).trim();
    if first.chars().count() <= 40 {
        first.to_string()
    } else {
        let cut: String = first.chars().take(39).collect();
        format!("{}…", cut.trim_end())
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

/// The panel's `HWND`, straight from GPUI. The platform helpers need it to
/// place, pin, hide and drag a window GPUI will not do those things to itself.
fn panel_hwnd(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(isize::from(handle.hwnd)),
        _ => None,
    }
}

pub fn open_main_window(
    cx: &mut App,
    controller: Entity<AppController>,
    show: bool,
) -> anyhow::Result<WindowHandle<MainWindow>> {
    let compact_size = size(px(theme::PANEL_W), px(theme::PANEL_H));
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
        place_main_window(theme::PANEL_W, theme::PANEL_H);
        lock_window_size();
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext, VisualTestContext};
    use rapidcap_capture::{AppPaths, CaptureKind, CaptureState, PhysicalRegion, Settings};

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
            CaptureState::Error("disk full".into()),
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
    fn long_errors_are_summarised_to_fit_the_well() {
        assert_eq!(error_summary("disk full"), "disk full");
        let long = error_summary(
            "Encoder error: nvenc session limit reached on device 0, cannot continue",
        );
        assert!(
            long.chars().count() <= 40,
            "{long} is too long for the well"
        );
        assert!(long.ends_with('…'));
        assert_eq!(error_summary("first line\nsecond line"), "first line");
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
