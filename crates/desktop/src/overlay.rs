use gpui::{
    Animation, AnimationExt, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    Role, Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, actions, div, point, prelude::*, px, size,
};
use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState, CaptureTarget, PhysicalRegion};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    controller::AppController,
    icons::Icon,
    motion,
    platform::{
        exclude_from_capture, monitor_containing, monitor_under_cursor, place_window,
        virtual_screen, window_target_at,
    },
    theme,
};

actions!(rapidcap_overlay, [CancelSelection]);

pub struct RegionOverlay {
    controller: Entity<AppController>,
    kind: CaptureKind,
    /// The whole virtual screen, not one display. Every coordinate below is
    /// measured from its top-left, which on a monitor left of the primary one
    /// is a negative number.
    monitor: PhysicalRegion,
    start: Option<Point<Pixels>>,
    current: Option<Point<Pixels>>,
    hovered: Option<CaptureTarget>,
    focus_handle: FocusHandle,
}

impl RegionOverlay {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        controller: Entity<AppController>,
        kind: CaptureKind,
        monitor: PhysicalRegion,
    ) -> Self {
        window.set_window_title("RapidCap Selection");
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            controller,
            kind,
            monitor,
            start: None,
            current: None,
            hovered: None,
            focus_handle,
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.start = Some(event.position);
        self.current = Some(event.position);
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.start.is_some() && event.dragging() {
            self.current = Some(event.position);
        } else {
            self.hovered = window_target_at(physical_point(
                event.position,
                &self.monitor,
                window.scale_factor(),
            ))
            .ok();
        }
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.start else {
            return;
        };
        let scale_factor = window.scale_factor();
        let Some(target) = selected_target(
            self.kind,
            physical_point(start, &self.monitor, scale_factor),
            physical_point(event.position, &self.monitor, scale_factor),
            self.hovered.as_ref(),
        ) else {
            self.start = None;
            self.current = None;
            cx.notify();
            return;
        };
        self.controller
            .update(cx, |controller, cx| controller.set_target(target, cx));
        window.remove_window();
    }

    fn cancel(&mut self, _: &CancelSelection, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, cx| {
            controller.dispatch(CaptureCommand::Cancel, cx)
        });
        window.remove_window();
    }
}

impl Render for RegionOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read live rather than cached: the overlay spans every display, so
        // Windows hands it the DPI of whichever one holds most of it, and that
        // arrives as a `WM_DPICHANGED` after the window is already up.
        let scale_factor = window.scale_factor();
        let mut root = div()
            .id("region-overlay")
            .key_context("RapidCapOverlay")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .relative()
            .size_full()
            .bg(theme::overlay_scrim())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up));

        // A press with no travel is a window pick, not a region. Ask the same
        // question the mouse-up will ask, so pressing on a window keeps its
        // outline instead of swapping it for a 0 x 0 rect that would never
        // become the selection anyway.
        let drag = self.start.zip(self.current).filter(|(start, current)| {
            is_region_drag(
                self.kind,
                physical_point(*start, &self.monitor, scale_factor),
                physical_point(*current, &self.monitor, scale_factor),
            )
        });

        if let Some((start, current)) = drag {
            let left = start.x.min(current.x);
            let top = start.y.min(current.y);
            let width = (start.x - current.x).abs();
            let height = (start.y - current.y).abs();
            root = root.child(
                div()
                    .absolute()
                    .left(left)
                    .top(top)
                    .w(width)
                    .h(height)
                    .border_2()
                    .border_color(theme::accent())
                    .bg(theme::overlay_drag_fill())
                    .child(float_label(format!(
                        "{} × {}",
                        (width.as_f32() * scale_factor).round() as u32,
                        (height.as_f32() * scale_factor).round() as u32
                    )))
                    .child(
                        drag_handle()
                            .left(theme::u(HANDLE_INSET))
                            .top(theme::u(HANDLE_INSET)),
                    )
                    .child(
                        drag_handle()
                            .right(theme::u(HANDLE_INSET))
                            .top(theme::u(HANDLE_INSET)),
                    )
                    .child(
                        drag_handle()
                            .left(theme::u(HANDLE_INSET))
                            .bottom(theme::u(HANDLE_INSET)),
                    )
                    .child(
                        drag_handle()
                            .right(theme::u(HANDLE_INSET))
                            .bottom(theme::u(HANDLE_INSET)),
                    ),
            );
        } else if let Some(target) = &self.hovered {
            let (region, label) = match target {
                CaptureTarget::Window {
                    region,
                    process_name,
                    ..
                } => (region, process_name.as_str()),
                CaptureTarget::Region(_) => unreachable!(),
            };
            root = root.child(
                div()
                    .absolute()
                    .left(px((region.x - self.monitor.x) as f32 / scale_factor))
                    .top(px((region.y - self.monitor.y) as f32 / scale_factor))
                    .w(px(region.width as f32 / scale_factor))
                    .h(px(region.height as f32 / scale_factor))
                    .border_2()
                    .border_color(theme::accent())
                    .bg(theme::overlay_window_fill())
                    .child(float_label(label.to_owned())),
            );
        }
        root = root.child(
            float_label(
                if self.kind == CaptureKind::ActiveWindowScreenshot {
                    "Click window · Esc cancel"
                } else {
                    "Click window or drag region · Esc cancel"
                }
                .to_string(),
            )
            .top(theme::u(18.0))
            .right(theme::u(18.0))
            .left_auto(),
        );
        root
    }
}

/// How long the recording bar waits, pointer away, before it fades.
const HUD_FADE_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

/// One glyph size for every HUD button, whatever the button does.
const HUD_ICON: f32 = 14.0;
/// A HUD button that is present but cannot be pressed yet.
const HUD_DISABLED_OPACITY: f32 = 0.35;

/// Half the grip hangs outside the rect, so it straddles the 2px border rather
/// than sitting inside the selection and covering the pixels being chosen.
const HANDLE_INSET: f32 = -4.0;

/// Corner grip. Only the drag rect gets these - a window hover is not resizable,
/// so grips there would promise an edge the user cannot move.
fn drag_handle() -> gpui::Div {
    div()
        .absolute()
        .size(theme::u(theme::HANDLE))
        .rounded(theme::u(1.0))
        .bg(theme::text_primary())
}

/// One shape for both overlay labels. They share a screen during a drag, so a
/// size badge and a hint pill with different heights sit on different baselines
/// and read as two unrelated things.
fn float_label(text: String) -> gpui::Div {
    div()
        .absolute()
        .top(theme::u(theme::GAP))
        .left(theme::u(theme::GAP))
        .h(theme::u(theme::FLOAT_H))
        .px(theme::u(11.0))
        .flex()
        .items_center()
        .rounded(theme::u(theme::RADIUS))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::overlay_float())
        .text_color(theme::text_primary())
        .text_size(theme::u(13.0))
        .child(text)
}

pub fn overlay_key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "escape",
        CancelSelection,
        Some("RapidCapOverlay"),
    )]
}

pub fn open_region_overlay(
    cx: &mut App,
    controller: Entity<AppController>,
) -> anyhow::Result<WindowHandle<RegionOverlay>> {
    let kind = match controller.read(cx).state() {
        CaptureState::Selecting(kind) => *kind,
        state => anyhow::bail!("selector opened from invalid state: {state:?}"),
    };
    // The window covers every display, not the one under the pointer: a scrim
    // over one monitor leaves the others bright and live, so a modal selection
    // reads as a half-applied effect and a drag that crosses a seam runs off the
    // edge of the only surface taking mouse events.
    let monitor = virtual_screen();
    // The cursor's display still decides which one GPUI creates the window on,
    // which is what sets its initial DPI - starting on the display the user is
    // looking at beats starting on the primary one and being corrected.
    let (display_id, _) = monitor_under_cursor()?;
    let display = cx
        .find_display(display_id)
        .or_else(|| cx.primary_display())
        .ok_or_else(|| anyhow::anyhow!("no display available"))?;
    let bounds: Bounds<Pixels> = display.bounds();
    let placement = monitor.clone();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: Some(display.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("com.inspire.rapidcap.selection".into()),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| RegionOverlay::new(window, cx, controller, kind, monitor)),
    )?;
    // Measured: GPUI treats `window_bounds` as the outer frame, so a window
    // asked to cover a 2560x1440 monitor came out 2576x1448 with its client
    // origin at (0, -4) - the top two pixels of every highlight border were
    // drawn above the screen and clipped. `place_window` strips the frame, which
    // makes client and window rects identical, then positions it in the device
    // pixels the monitor is actually measured in. It is also the only way to
    // reach the virtual screen at all: `window_bounds` is logical and per
    // display, and no logical rectangle describes a desktop whose displays run
    // at different scales.
    if let Some(hwnd) = handle.update(cx, |_view, window, _cx| panel_hwnd(window))? {
        place_window(
            hwnd,
            placement.x,
            placement.y,
            placement.width as i32,
            placement.height as i32,
        );
        // The screenshot is taken 40ms after this window asks to close, which is
        // a bet on how long the compositor takes to stop drawing it. macOS lost
        // that bet: the saved PNG came back with the selection rectangle and the
        // size badge painted into it. Excluding the window settles it instead of
        // lengthening the wait - the overlay is never part of a capture, whether
        // or not it is still on screen when one is taken.
        exclude_from_capture(hwnd);
    }
    Ok(handle)
}

pub struct RecordingHud {
    controller: Entity<AppController>,
    target: CaptureTarget,
    countdown_since: std::time::Instant,
    /// Last time the pointer was over the bar. Drives the idle fade.
    pointer_seen: std::time::Instant,
    /// Whether the bar has ever dimmed. Until it has there is nothing to fade
    /// back from, and a 240ms ramp up to full on the very first frame would be
    /// 240ms of a dimmed bar over something the user is recording right now.
    has_faded: bool,
    _subscription: Subscription,
}

impl RecordingHud {
    fn new(
        cx: &mut Context<Self>,
        controller: Entity<AppController>,
        target: CaptureTarget,
    ) -> Self {
        let subscription = cx.observe(&controller, |_this, _, cx| cx.notify());
        let opened = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            // One repaint per second, landing *on* the second boundary instead of
            // five per second hoping to catch it. Everything this drives - the
            // elapsed clock, the countdown, the idle fade - is read with
            // `as_secs()`, so the other four repaints were pure CPU wakeups for
            // a bar whose pixels had not changed.
            loop {
                let elapsed = opened.elapsed();
                let until_next_second =
                    std::time::Duration::from_secs(elapsed.as_secs() + 1) - elapsed;
                cx.background_executor().timer(until_next_second).await;
                if this.update(cx, |_this, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            controller,
            target,
            countdown_since: opened,
            pointer_seen: opened,
            has_faded: false,
            _subscription: subscription,
        }
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, cx| {
            controller.dispatch(CaptureCommand::TogglePause, cx)
        });
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let command = match self.controller.read(cx).state() {
            CaptureState::Countdown(CaptureKind::Video, _)
            | CaptureState::Recording(CaptureKind::Video)
            | CaptureState::Paused(CaptureKind::Video) => CaptureCommand::ToggleVideo,
            _ => CaptureCommand::ToggleGif,
        };
        let _ = self
            .controller
            .update(cx, |controller, cx| controller.dispatch(command, cx));
    }
}

impl Render for RecordingHud {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.controller.read(cx).state().clone();
        let target = match &self.target {
            CaptureTarget::Region(region) => format!("{} × {}", region.width, region.height),
            CaptureTarget::Window { process_name, .. } => process_name.clone(),
        };
        let HudFace {
            status,
            pause_label,
            pause_icon,
            can_pause,
            dot,
        } = hud_face(
            &state,
            &target,
            self.countdown_since.elapsed().as_secs() as u8,
            self.controller.read(cx).recording_elapsed().as_secs(),
        );
        // Long recordings stop feeling watched: after three quiet seconds the
        // contents drop to 55% and come back on hover. Only while something is
        // running - a countdown is asking a question and has to stay legible.
        // The fill and the border are deliberately not part of it: dimming the
        // whole pill took the background with it, and what was left over a
        // bright desktop was three outlines floating on nothing.
        let faded = matches!(state, CaptureState::Recording(_) | CaptureState::Paused(_))
            && self.pointer_seen.elapsed() >= HUD_FADE_AFTER;
        self.has_faded |= faded;
        let (fade_from, fade_to) = if faded {
            (1.0, theme::HUD_IDLE_OPACITY)
        } else if self.has_faded {
            (theme::HUD_IDLE_OPACITY, 1.0)
        } else {
            (1.0, 1.0)
        };
        let pulsing = matches!(state, CaptureState::Recording(_));
        // The window is a transparent letterbox; the pill inside it is what the
        // user sees, so it can size to its content the way the spec asks.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                this.pointer_seen = std::time::Instant::now();
                cx.notify();
            }))
            .child(
                div()
                    .id("recording-hud")
                    .accessibility_id("rapidcap.hud")
                    .role(Role::Application)
                    .aria_label("RapidCap recording controls")
                    .h(theme::u(theme::HUD_H))
                    .w(theme::u(theme::HUD_W))
                    .flex()
                    .items_center()
                    .rounded(theme::u(theme::RADIUS_PILL))
                    .border_2()
                    .border_color(theme::border_card())
                    .bg(theme::hud_bg())
                    .shadow(theme::floating())
                    .text_color(theme::text_primary())
                    .child(
                        // Everything the idle fade applies to, in one element.
                        // The padding and the gap live here rather than on the
                        // pill so the row still measures the same.
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .gap(theme::u(4.0))
                            .p(theme::u(4.0))
                            .child(
                                div()
                                    .id("recording-hud-status")
                                    .accessibility_id("rapidcap.hud-status")
                                    .role(Role::Status)
                                    .aria_label(status.clone())
                                    // Takes whatever the fixed-width pill has left over,
                                    // and clips rather than pushes: a long window title
                                    // must not be able to move the buttons.
                                    .flex_1()
                                    .min_w(theme::u(0.0))
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    // Centred inside that leftover space. The pill is a
                                    // fixed width so the buttons cannot slide out from
                                    // under the pointer mid-recording, which left a
                                    // running clock - `00:27`, five glyphs - stranded
                                    // against the left edge with the rest of the bar
                                    // empty. A countdown line fills the space and
                                    // ellipsises, so centring is a no-op there.
                                    .justify_center()
                                    .gap(theme::u(7.0))
                                    // Symmetric, or the centre sits 2px left of it.
                                    .px(theme::u(8.0))
                                    .text_size(theme::u(13.0))
                                    .text_ellipsis()
                                    .child(motion::status_dot("hud-pulse", 8.0, dot, pulsing))
                                    .child(status),
                            )
                            // Both buttons exist in every state. Dropping the pause
                            // button during the countdown used to slide the stop button
                            // sideways at the exact moment the pointer was heading for
                            // it; it greys out instead.
                            .child(
                                hud_button(
                                    "rapidcap.hud-pause",
                                    pause_label,
                                    pause_icon,
                                    false,
                                    can_pause,
                                )
                                .when(can_pause, |button| {
                                    button.on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_pause(cx)),
                                    )
                                }),
                            )
                            .child(
                                hud_button(
                                    "rapidcap.hud-stop",
                                    if can_pause { "Stop" } else { "Cancel" },
                                    Icon::Stop,
                                    true,
                                    true,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                            )
                            // Linear, because eased opacity reads as a flicker. The id
                            // carries `faded`: `with_animation` restarts when its id
                            // changes, and that restart is the only thing that runs the
                            // fade back up when the pointer returns.
                            .with_animation(
                                ("hud-fade", faded as usize),
                                Animation::new(motion::HUD_FADE),
                                move |row, delta| {
                                    row.opacity(fade_from + (fade_to - fade_from) * delta)
                                },
                            ),
                    ),
            )
    }
}

/// `MM:SS`, zero-padded so the pill does not resize as the timer rolls over.
fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn kind_noun(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Video => "Video",
        CaptureKind::Gif => "GIF",
        CaptureKind::RegionScreenshot => "Region",
        CaptureKind::ActiveWindowScreenshot => "Window",
    }
}

/// What the pill says and shows, for one capture state.
///
/// Pulled out of `render` so it can be asserted on. The bar is excluded from
/// screen capture - it has to be, or it lands in the recording - which also
/// hides it from the screenshots the interaction suite takes, so its colours
/// cannot be checked from outside the process. This is where that check lives
/// instead.
struct HudFace {
    status: String,
    pause_label: &'static str,
    pause_icon: Icon,
    can_pause: bool,
    dot: gpui::Hsla,
}

/// While a capture runs the only fact worth width is the clock. The kind and the
/// target are shown during the countdown, when there is still a decision to
/// make, and collapse away once recording starts.
fn hud_face(state: &CaptureState, target: &str, countdown_elapsed: u8, recorded: u64) -> HudFace {
    let (status, pause_label, pause_icon, can_pause, dot) = match state {
        CaptureState::Countdown(kind, seconds) => (
            format!(
                "{} in {} · {target}",
                kind_noun(*kind),
                seconds.saturating_sub(countdown_elapsed)
            ),
            "Pause",
            Icon::Pause,
            false,
            theme::text_muted(),
        ),
        CaptureState::Recording(_) => (clock(recorded), "Pause", Icon::Pause, true, theme::rec()),
        CaptureState::Paused(_) => (
            clock(recorded),
            "Resume",
            Icon::Play,
            true,
            theme::text_pill(),
        ),
        CaptureState::Finalizing(kind) => (
            format!("Finalizing {}…", kind_noun(*kind)),
            "Pause",
            Icon::Pause,
            false,
            theme::text_muted(),
        ),
        _ => (
            "Closing…".into(),
            "Pause",
            Icon::Pause,
            false,
            theme::text_muted(),
        ),
    };
    HudFace {
        status,
        pause_label,
        pause_icon,
        can_pause,
        dot,
    }
}

/// Circular icon button, 28px inside a 36px pill. Text labels cost width the
/// HUD does not have — `aria_label` carries the name.
fn hud_button(
    id: &'static str,
    label: &'static str,
    icon: Icon,
    danger: bool,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let (rest_bg, hover_bg, colour) = if danger {
        (
            theme::danger(),
            theme::danger_hover(),
            theme::text_primary(),
        )
    } else {
        (
            gpui::transparent_black(),
            theme::bg_hover(),
            theme::text_pill(),
        )
    };
    div()
        .id(id)
        .accessibility_id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(theme::u(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        // No border. Two outlined circles inside an outlined 36px pill read as
        // three stacked rings rather than as buttons; the stop button carries
        // its fill and the pause button its hover fill, which is enough.
        .bg(rest_bg)
        .text_color(colour)
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg).text_color(theme::text_primary()))
        })
        .when(!enabled, |button| button.opacity(HUD_DISABLED_OPACITY))
        // One size for every HUD button. The stop icon used to render two pixels
        // smaller than the pause icon, which read as the row resizing whenever
        // the state changed.
        .child(icon.element(theme::u(HUD_ICON), colour))
}

pub fn open_recording_hud(
    cx: &mut App,
    controller: Entity<AppController>,
    target: CaptureTarget,
) -> anyhow::Result<WindowHandle<RecordingHud>> {
    let region = match &target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    // Every number below is a device pixel, because the region is. GPUI's
    // display bounds are logical, and mixing the two put the bar above a region
    // that had room under it and left it off centre by half the difference
    // between the pill's logical and physical width.
    // The region's display, not the cursor's. Once a selection can be dragged
    // across a seam the two are different displays, and the bar has to be
    // measured and clamped against the one it is going to sit on.
    let (display_id, monitor) = monitor_containing(region)?;
    let scale = cx
        .find_display(display_id)
        .or_else(|| cx.primary_display())
        .map_or(1.0, |display| {
            monitor.width as f32 / f32::from(display.bounds().size.width)
        });
    let width = theme::scaled(HUD_WINDOW_W) * scale;
    let height = theme::scaled(HUD_WINDOW_H) * scale;
    let gap = theme::scaled(theme::GAP) * scale;
    let (x, y) = hud_origin(region, &monitor, width, height, gap);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                // Logical, which is the only unit GPUI accepts here. The
                // device-pixel placement below is what actually lands.
                origin: point(px(x / scale), px(y / scale)),
                size: size(theme::u(HUD_WINDOW_W), theme::u(HUD_WINDOW_H)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Transparent so the pill inside can be content-sized: an opaque
            // window would paint its own rectangle around it.
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("com.inspire.rapidcap.recording-hud".into()),
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| RecordingHud::new(cx, controller, target)),
    )?;
    // `hud_origin` works in device pixels because the region does; GPUI's
    // window bounds are logical, so on a scaled display the bar landed short of
    // where it was aimed. Moving the HWND afterwards skips the conversion, and
    // strips the Windows 11 border DWM draws around a transparent popup.
    if let Some(hwnd) = handle.update(cx, |_view, window, _cx| panel_hwnd(window))? {
        // Rounded, not truncated. A pill centred at 707.5 truncates to 707 and
        // sits a pixel left of the region it belongs to, which is visible next
        // to a selection rectangle drawn on the exact boundary.
        place_window(
            hwnd,
            x.round() as i32,
            y.round() as i32,
            width.round() as i32,
            height.round() as i32,
        );
        // The bar is up for the whole take, over the region being recorded, so
        // without this it is in the file.
        exclude_from_capture(hwnd);
        // Windows has no equivalent for one rectangle inside a window - the
        // DWM backdrops apply to a whole window - so the pill stays opaque
        // there and this is macOS only.
        #[cfg(target_os = "macos")]
        crate::platform::blur_behind(
            hwnd,
            theme::scaled(theme::HUD_W),
            theme::scaled(theme::HUD_H),
            theme::scaled(theme::pill_radius(theme::HUD_H)),
        );
    }
    Ok(handle)
}

/// A GPUI window's real `HWND`.
fn panel_hwnd(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(isize::from(handle.hwnd)),
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as isize),
        _ => None,
    }
}

/// The transparent letterbox the HUD pill floats in.
const HUD_WINDOW_W: f32 = 360.0;
const HUD_WINDOW_H: f32 = 44.0;

/// Below the region and centred on it — controls belong under the thing they
/// control. Falls back above when the region is near the screen bottom, and
/// inside it when neither fits.
///
/// Everything here is a device pixel, including `gap`, because the region is.
/// The gap arrives already scaled rather than being read from the theme: a
/// design-pixel constant compared against device coordinates draws the bar
/// closer to the region than the design asks for, by exactly the display and
/// text scale.
fn hud_origin(
    region: &PhysicalRegion,
    monitor: &PhysicalRegion,
    width: f32,
    height: f32,
    gap: f32,
) -> (f32, f32) {
    let region_bottom = (region.y + region.height as i32) as f32;
    let below = region_bottom + gap;
    let above = region.y as f32 - gap - height;

    // A display is not a screen at the origin. One placed left of or above the
    // primary display has negative coordinates, so every clamp below is against
    // its own edges - clamping to zero pushed the bar onto the primary display,
    // which is not the one the region is on.
    let left = monitor.x as f32;
    let top = monitor.y as f32;
    let right = (monitor.x + monitor.width as i32) as f32;
    let bottom = (monitor.y + monitor.height as i32) as f32;

    let y = if below + height <= bottom {
        below
    } else if above >= top {
        above
    } else {
        // Region fills the display: sit inside its bottom edge.
        region_bottom - gap - height
    };
    let x = region.x as f32 + (region.width as f32 - width) / 2.0;
    // `max(left)` on the upper bound because a pill wider than the display has
    // no in-bounds position, and the left edge is the less wrong of the two.
    (x.clamp(left, (right - width).max(left)), y.max(top))
}

pub fn close_recording_hud<C: gpui::AppContext>(
    handle: &mut Option<WindowHandle<RecordingHud>>,
    cx: &mut C,
) {
    if let Some(handle) = handle.take() {
        let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
    }
}

fn selected_target(
    kind: CaptureKind,
    start: (i32, i32),
    end: (i32, i32),
    hovered: Option<&CaptureTarget>,
) -> Option<CaptureTarget> {
    if is_region_drag(kind, start, end) {
        PhysicalRegion::from_drag(start, end).map(CaptureTarget::Region)
    } else {
        hovered.cloned()
    }
}

/// Has the pointer travelled far enough for this press to mean a region?
///
/// Six physical pixels of slack, so the hand shake in a click does not turn a
/// window pick into a one-pixel region. A window screenshot never reads a drag
/// at all - its target is always the window under the pointer.
fn is_region_drag(kind: CaptureKind, start: (i32, i32), end: (i32, i32)) -> bool {
    kind != CaptureKind::ActiveWindowScreenshot
        && (start.0.abs_diff(end.0) >= 6 || start.1.abs_diff(end.1) >= 6)
}

fn physical_point(point: Point<Pixels>, monitor: &PhysicalRegion, scale_factor: f32) -> (i32, i32) {
    (
        monitor.x + (point.x.as_f32() * scale_factor).round() as i32,
        monitor.y + (point.y.as_f32() * scale_factor).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};
    use rapidcap_capture::CaptureKind;

    use super::*;

    #[test]
    fn local_logical_point_converts_to_monitor_physical_point() {
        let monitor = PhysicalRegion {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            physical_point(point(px(100.0), px(80.0)), &monitor, 1.5),
            (-1770, 120)
        );
    }

    #[test]
    fn click_selects_hovered_window_for_window_capture() {
        let hovered = CaptureTarget::Window {
            hwnd: 7,
            region: PhysicalRegion {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            },
            process_name: "Code".into(),
        };
        assert_eq!(
            selected_target(
                CaptureKind::ActiveWindowScreenshot,
                (50, 50),
                (50, 50),
                Some(&hovered)
            ),
            Some(hovered)
        );
    }

    #[test]
    fn recording_drag_selects_region_instead_of_hovered_window() {
        let hovered = CaptureTarget::Window {
            hwnd: 7,
            region: PhysicalRegion {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            process_name: "Code".into(),
        };
        assert_eq!(
            selected_target(CaptureKind::Video, (20, 30), (220, 130), Some(&hovered)),
            Some(CaptureTarget::Region(PhysicalRegion {
                x: 20,
                y: 30,
                width: 200,
                height: 100
            }))
        );
    }

    #[test]
    fn a_press_with_no_travel_is_not_a_region_drag() {
        // The bug this guards: mouse-down set start and current to the same
        // point, the render read that as a drag, and the window outline the
        // user was aiming at became a 0 x 0 rect. Mouse-up then picked the
        // window anyway, so the overlay showed one thing and did another.
        assert!(!is_region_drag(CaptureKind::Video, (400, 300), (400, 300)));
        assert!(!is_region_drag(CaptureKind::Video, (400, 300), (405, 304)));
        assert!(is_region_drag(CaptureKind::Video, (400, 300), (406, 300)));
        assert!(!is_region_drag(
            CaptureKind::ActiveWindowScreenshot,
            (400, 300),
            (900, 800)
        ));
    }

    #[test]
    fn hud_sits_below_the_region_unless_there_is_no_room() {
        let screen = PhysicalRegion {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        // Device pixels, the unit the caller works in. Held apart from
        // `theme::GAP` so the assertions read against a number rather than
        // against the same expression the function evaluates.
        let gap = 12.0;
        let region = PhysicalRegion {
            x: 400,
            y: 200,
            width: 800,
            height: 400,
        };

        // Normal: one gap under the bottom edge, centred on the region.
        let (x, y) = hud_origin(&region, &screen, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(y, 600.0 + gap);
        assert_eq!(x, 400.0 + (800.0 - HUD_WINDOW_W) / 2.0);

        // No room below: flip above the top edge.
        let low = PhysicalRegion {
            x: 400,
            y: 600,
            width: 800,
            height: 460,
        };
        let (_, y) = hud_origin(&low, &screen, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(y, 600.0 - gap - HUD_WINDOW_H);

        // Region fills the display: sit inside its bottom edge.
        let full = PhysicalRegion {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let (x, y) = hud_origin(&full, &screen, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(y, 1080.0 - gap - HUD_WINDOW_H);
        assert!(x >= 0.0, "the HUD never starts off the left edge");

        // A region narrower than the pill still clamps on screen.
        let narrow = PhysicalRegion {
            x: 10,
            y: 100,
            width: 120,
            height: 80,
        };
        let (x, _) = hud_origin(&narrow, &screen, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(x, 0.0);
    }

    #[test]
    fn hud_clamps_to_the_display_the_region_is_on_not_to_the_origin() {
        // A second display left of the primary one runs from x = -1920 to 0.
        // The old code clamped x and y to zero, which put the bar on the
        // primary display - the one the user was not recording.
        let left_of_primary = PhysicalRegion {
            x: -1920,
            y: -180,
            width: 1920,
            height: 1080,
        };
        let gap = 12.0;
        let narrow = PhysicalRegion {
            x: -1910,
            y: -100,
            width: 120,
            height: 80,
        };
        let (x, y) = hud_origin(&narrow, &left_of_primary, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(x, -1920.0, "the bar stays on the display it belongs to");
        assert_eq!(y, -20.0 + gap, "a gap under a region whose y is negative");

        // Nothing below and nothing above: the display's own top edge is the
        // floor, not y = 0.
        let full = PhysicalRegion {
            x: -1920,
            y: -180,
            width: 1920,
            height: 1080,
        };
        let (_, y) = hud_origin(&full, &left_of_primary, HUD_WINDOW_W, HUD_WINDOW_H, gap);
        assert_eq!(y, 900.0 - gap - HUD_WINDOW_H);
    }

    #[test]
    fn the_dot_is_red_only_while_something_is_being_recorded() {
        // The dot is the one thing on the bar that says whether frames are
        // being written, so the states around a recording have to be able to
        // tell themselves apart at a glance.
        let face = |state| hud_face(&state, "800 × 600", 0, 27).dot;
        assert_eq!(
            face(CaptureState::Recording(CaptureKind::Video)),
            theme::rec()
        );
        assert_ne!(face(CaptureState::Paused(CaptureKind::Video)), theme::rec());
        assert_ne!(
            face(CaptureState::Countdown(CaptureKind::Video, 3)),
            theme::rec()
        );
        assert_ne!(
            face(CaptureState::Finalizing(CaptureKind::Video)),
            theme::rec()
        );
    }

    #[test]
    fn the_countdown_names_the_target_and_the_recording_only_keeps_the_clock() {
        // The countdown is the last chance to cancel, so it spells out what is
        // about to be recorded. Once it is running the pill is a fixed width
        // and the clock is the only thing worth that width.
        let counting = hud_face(
            &CaptureState::Countdown(CaptureKind::Video, 3),
            "800 × 600",
            1,
            0,
        );
        assert_eq!(counting.status, "Video in 2 · 800 × 600");
        assert!(!counting.can_pause, "nothing to pause before it starts");

        let running = hud_face(
            &CaptureState::Recording(CaptureKind::Video),
            "800 × 600",
            0,
            87,
        );
        assert_eq!(running.status, "01:27");
        assert!(running.can_pause);
    }

    #[test]
    fn the_pause_button_offers_the_action_the_state_is_missing() {
        // Pressing pause has to leave a button that resumes, or a paused
        // recording is a stuck one.
        let running = hud_face(&CaptureState::Recording(CaptureKind::Gif), "app", 0, 0);
        assert_eq!(running.pause_label, "Pause");
        assert_eq!(running.pause_icon, Icon::Pause);

        let paused = hud_face(&CaptureState::Paused(CaptureKind::Gif), "app", 0, 0);
        assert_eq!(paused.pause_label, "Resume");
        assert_eq!(paused.pause_icon, Icon::Play);
        assert!(paused.can_pause, "a paused recording must be resumable");
    }
}
