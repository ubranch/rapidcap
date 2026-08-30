use std::mem::size_of;

use gpui::{
    App, AppContext as _, Bounds, Context, DisplayId, Entity, FocusHandle, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Role, Subscription,
    Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    actions, div, point, prelude::*, px, size,
};
use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState, CaptureTarget, PhysicalRegion};
use windows::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    },
    UI::WindowsAndMessaging::GetCursorPos,
};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{
    controller::AppController,
    icons::Icon,
    platform::{place_window, window_target_at},
    theme,
};

actions!(rapidcap_overlay, [CancelSelection]);

pub struct RegionOverlay {
    controller: Entity<AppController>,
    kind: CaptureKind,
    monitor: PhysicalRegion,
    scale_factor: f32,
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
            scale_factor: window.scale_factor(),
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

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.start.is_some() && event.dragging() {
            self.current = Some(event.position);
        } else {
            self.hovered = window_target_at(physical_point(
                event.position,
                &self.monitor,
                self.scale_factor,
            ))
            .ok();
        }
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.start else {
            return;
        };
        let Some(target) = selected_target(
            self.kind,
            physical_point(start, &self.monitor, self.scale_factor),
            physical_point(event.position, &self.monitor, self.scale_factor),
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                physical_point(*start, &self.monitor, self.scale_factor),
                physical_point(*current, &self.monitor, self.scale_factor),
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
                        (width.as_f32() * self.scale_factor).round() as u32,
                        (height.as_f32() * self.scale_factor).round() as u32
                    )))
                    .child(drag_handle().left(px(HANDLE_INSET)).top(px(HANDLE_INSET)))
                    .child(drag_handle().right(px(HANDLE_INSET)).top(px(HANDLE_INSET)))
                    .child(
                        drag_handle()
                            .left(px(HANDLE_INSET))
                            .bottom(px(HANDLE_INSET)),
                    )
                    .child(
                        drag_handle()
                            .right(px(HANDLE_INSET))
                            .bottom(px(HANDLE_INSET)),
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
                    .left(px((region.x - self.monitor.x) as f32 / self.scale_factor))
                    .top(px((region.y - self.monitor.y) as f32 / self.scale_factor))
                    .w(px(region.width as f32 / self.scale_factor))
                    .h(px(region.height as f32 / self.scale_factor))
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
            .top(px(18.0))
            .right(px(18.0))
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
        .size(px(theme::HANDLE))
        .rounded(px(1.0))
        .bg(theme::text_primary())
}

/// One shape for both overlay labels. They share a screen during a drag, so a
/// size badge and a hint pill with different heights sit on different baselines
/// and read as two unrelated things.
fn float_label(text: String) -> gpui::Div {
    div()
        .absolute()
        .top(px(theme::GAP))
        .left(px(theme::GAP))
        .h(px(theme::FLOAT_H))
        .px(px(11.0))
        .flex()
        .items_center()
        .rounded(px(theme::RADIUS))
        .border_2()
        .border_color(theme::border_card())
        .bg(theme::overlay_float())
        .text_color(theme::text_primary())
        .text_size(px(13.0))
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
    let (display_id, monitor) = monitor_under_cursor()?;
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
    // pixels the monitor is actually measured in.
    if let Some(hwnd) = handle.update(cx, |_view, window, _cx| panel_hwnd(window))? {
        place_window(
            hwnd,
            placement.x,
            placement.y,
            placement.width as i32,
            placement.height as i32,
        );
    }
    Ok(handle)
}

pub struct RecordingHud {
    controller: Entity<AppController>,
    target: CaptureTarget,
    countdown_since: std::time::Instant,
    /// Last time the pointer was over the bar. Drives the idle fade.
    pointer_seen: std::time::Instant,
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
        // While a capture runs the only fact worth width is the clock. The kind
        // and the target are shown during the countdown, when there is still a
        // decision to make, and collapse away once recording starts.
        let (status, pause_label, pause_icon, can_pause, dot) = match state {
            CaptureState::Countdown(kind, seconds) => (
                format!(
                    "{} in {} · {target}",
                    kind_noun(kind),
                    seconds.saturating_sub(self.countdown_since.elapsed().as_secs() as u8)
                ),
                "Pause",
                Icon::Pause,
                false,
                theme::text_muted(),
            ),
            CaptureState::Recording(_) => (
                clock(self.controller.read(cx).recording_elapsed().as_secs()),
                "Pause",
                Icon::Pause,
                true,
                theme::rec(),
            ),
            CaptureState::Paused(_) => (
                clock(self.controller.read(cx).recording_elapsed().as_secs()),
                "Resume",
                Icon::Play,
                true,
                theme::text_pill(),
            ),
            CaptureState::Finalizing(kind) => (
                format!("Finalizing {}…", kind_noun(kind)),
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
        // Long recordings stop feeling watched: after three quiet seconds the bar
        // drops to 55% and comes back on hover. Only while something is running -
        // a countdown is asking a question and has to stay legible.
        let faded = matches!(state, CaptureState::Recording(_) | CaptureState::Paused(_))
            && self.pointer_seen.elapsed() >= HUD_FADE_AFTER;
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
                    .when(faded, |pill| pill.opacity(theme::HUD_IDLE_OPACITY))
                    .h(px(theme::HUD_H))
                    .w(px(theme::HUD_W))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .p(px(4.0))
                    .rounded(px(theme::RADIUS_PILL))
                    .border_2()
                    .border_color(theme::border_card())
                    .bg(theme::hud_bg())
                    .shadow(theme::floating())
                    .text_color(theme::text_primary())
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
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .pl(px(8.0))
                            .pr(px(4.0))
                            .text_size(px(13.0))
                            .text_ellipsis()
                            .child(
                                div()
                                    .size(px(8.0))
                                    .flex_none()
                                    .rounded(px(theme::RADIUS_PILL))
                                    .bg(dot),
                            )
                            .child(status),
                    )
                    .child(
                        div()
                            .w(px(theme::BORDER))
                            .h(px(18.0))
                            .flex_none()
                            .bg(theme::border_divider()),
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
                            button.on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx)))
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
        .size(px(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_PILL))
        .border_2()
        .border_color(if danger {
            theme::danger_hover()
        } else {
            theme::border_card()
        })
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
        .child(icon.element(px(HUD_ICON), colour))
}

pub fn open_recording_hud(
    cx: &mut App,
    controller: Entity<AppController>,
    target: CaptureTarget,
) -> anyhow::Result<WindowHandle<RecordingHud>> {
    let region = match &target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    let (x, y) = hud_origin(region, cx.primary_display().map(|display| display.bounds()));
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(x), px(y)),
                size: size(px(HUD_WINDOW_W), px(HUD_WINDOW_H)),
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
        let scale = handle.update(cx, |_view, window, _cx| window.scale_factor())?;
        place_window(
            hwnd,
            x as i32,
            y as i32,
            (HUD_WINDOW_W * scale) as i32,
            (HUD_WINDOW_H * scale) as i32,
        );
    }
    Ok(handle)
}

/// A GPUI window's real `HWND`.
fn panel_hwnd(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(isize::from(handle.hwnd)),
        _ => None,
    }
}

/// The transparent letterbox the HUD pill floats in.
const HUD_WINDOW_W: f32 = 360.0;
const HUD_WINDOW_H: f32 = 44.0;

/// Below the region and centred on it — controls belong under the thing they
/// control. Falls back above when the region is near the screen bottom, and
/// inside it when neither fits.
fn hud_origin(region: &PhysicalRegion, screen: Option<Bounds<Pixels>>) -> (f32, f32) {
    let gap = theme::GAP;
    let region_bottom = (region.y + region.height as i32) as f32;
    let below = region_bottom + gap;
    let above = region.y as f32 - gap - HUD_WINDOW_H;

    let screen_bottom = screen
        .map(|bounds| f32::from(bounds.origin.y + bounds.size.height))
        .unwrap_or(f32::MAX);

    let y = if below + HUD_WINDOW_H <= screen_bottom {
        below
    } else if above >= 0.0 {
        above
    } else {
        // Region fills the display: sit inside its bottom edge.
        region_bottom - gap - HUD_WINDOW_H
    };
    let x = region.x as f32 + (region.width as f32 - HUD_WINDOW_W) / 2.0;
    (x.max(0.0), y.max(0.0))
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

fn monitor_under_cursor() -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }?;
    let monitor: HMONITOR = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.ok()?;
    let bounds = info.rcMonitor;
    Ok((
        DisplayId::new(monitor.0 as isize as u64),
        PhysicalRegion {
            x: bounds.left,
            y: bounds.top,
            width: (bounds.right - bounds.left) as u32,
            height: (bounds.bottom - bounds.top) as u32,
        },
    ))
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
        let screen = Some(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(1920.0), px(1080.0)),
        });
        let region = PhysicalRegion {
            x: 400,
            y: 200,
            width: 800,
            height: 400,
        };

        // Normal: 9px under the bottom edge, centred on the region.
        let (x, y) = hud_origin(&region, screen);
        assert_eq!(y, 600.0 + theme::GAP);
        assert_eq!(x, 400.0 + (800.0 - HUD_WINDOW_W) / 2.0);

        // No room below: flip above the top edge.
        let low = PhysicalRegion {
            x: 400,
            y: 600,
            width: 800,
            height: 460,
        };
        let (_, y) = hud_origin(&low, screen);
        assert_eq!(y, 600.0 - theme::GAP - HUD_WINDOW_H);

        // Region fills the display: sit inside its bottom edge.
        let full = PhysicalRegion {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let (x, y) = hud_origin(&full, screen);
        assert_eq!(y, 1080.0 - theme::GAP - HUD_WINDOW_H);
        assert!(x >= 0.0, "the HUD never starts off the left edge");

        // A region narrower than the pill still clamps on screen.
        let narrow = PhysicalRegion {
            x: 10,
            y: 100,
            width: 120,
            height: 80,
        };
        let (x, _) = hud_origin(&narrow, screen);
        assert_eq!(x, 0.0);
    }
}
