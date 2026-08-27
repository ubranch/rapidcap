use std::mem::size_of;

use gpui::{
    App, AppContext as _, Bounds, Context, DisplayId, Entity, FocusHandle, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Role, Subscription,
    Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    actions, div, point, prelude::*, px, rgba, size,
};
use rapidcap_capture::{CaptureCommand, CaptureKind, CaptureState, CaptureTarget, PhysicalRegion};
use windows::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    },
    UI::WindowsAndMessaging::GetCursorPos,
};

use crate::{controller::AppController, platform::window_target_at};

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
            .bg(rgba(0x00000066))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up));

        if let (Some(start), Some(current)) = (self.start, self.current) {
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
                    .border_color(rgba(0x4d8dffff))
                    .bg(rgba(0x00000022))
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .left(px(8.0))
                            .px(px(7.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .bg(rgba(0x111318dd))
                            .text_color(rgba(0xffffffff))
                            .text_size(px(12.0))
                            .child(format!(
                                "{} × {}",
                                (width.as_f32() * self.scale_factor).round() as u32,
                                (height.as_f32() * self.scale_factor).round() as u32
                            )),
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
                    .border_color(rgba(0x4d8dffff))
                    .bg(rgba(0x4d8dff22))
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .left(px(8.0))
                            .px(px(7.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .bg(rgba(0x111318dd))
                            .text_color(rgba(0xffffffff))
                            .text_size(px(12.0))
                            .child(label.to_owned()),
                    ),
            );
        }
        root = root.child(
            div()
                .absolute()
                .top(px(16.0))
                .right(px(16.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(rgba(0x111318dd))
                .text_color(rgba(0xffffffff))
                .text_size(px(13.0))
                .child(if self.kind == CaptureKind::ActiveWindowScreenshot {
                    "Click window · Esc cancel"
                } else {
                    "Click window or drag region · Esc cancel"
                }),
        );
        root
    }
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
    cx.open_window(
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
    )
}

pub struct RecordingBorder;

impl Render for RecordingBorder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgba(0xff2d2dff))
    }
}

pub struct RecordingHud {
    controller: Entity<AppController>,
    target: CaptureTarget,
    countdown_since: std::time::Instant,
    _subscription: Subscription,
}

impl RecordingHud {
    fn new(
        cx: &mut Context<Self>,
        controller: Entity<AppController>,
        target: CaptureTarget,
    ) -> Self {
        let subscription = cx.observe(&controller, |_this, _, cx| cx.notify());
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                if this.update(cx, |_this, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            controller,
            target,
            countdown_since: std::time::Instant::now(),
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
        let (status, pause_label, can_pause) = match state {
            CaptureState::Countdown(kind, seconds) => (
                format!(
                    "{} starts in {} · {target}",
                    if kind == CaptureKind::Video {
                        "Video"
                    } else {
                        "GIF"
                    },
                    seconds.saturating_sub(self.countdown_since.elapsed().as_secs() as u8)
                ),
                "Pause",
                false,
            ),
            CaptureState::Recording(kind) => {
                let elapsed = self.controller.read(cx).recording_elapsed().as_secs();
                (
                    format!(
                        "● REC {} {:02}:{:02} · {target}",
                        if kind == CaptureKind::Video {
                            "Video"
                        } else {
                            "GIF"
                        },
                        elapsed / 60,
                        elapsed % 60
                    ),
                    "Pause",
                    true,
                )
            }
            CaptureState::Paused(kind) => {
                let elapsed = self.controller.read(cx).recording_elapsed().as_secs();
                (
                    format!(
                        "Ⅱ PAUSED {} {:02}:{:02} · {target}",
                        if kind == CaptureKind::Video {
                            "Video"
                        } else {
                            "GIF"
                        },
                        elapsed / 60,
                        elapsed % 60
                    ),
                    "Resume",
                    true,
                )
            }
            CaptureState::Finalizing(kind) => (
                format!(
                    "Finalizing {}…",
                    if kind == CaptureKind::Video {
                        "Video"
                    } else {
                        "GIF"
                    }
                ),
                "Pause",
                false,
            ),
            _ => ("Closing…".into(), "Pause", false),
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .accessibility_id(id)
                .role(Role::Button)
                .aria_label(label)
                .px(px(12.0))
                .h(px(34.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(rgba(0x5a6070ff))
                .bg(rgba(0x242832ff))
                .text_color(rgba(0xffffffff))
                .cursor_pointer()
                .child(label)
        };
        div()
            .id("recording-hud")
            .accessibility_id("rapidcap.hud")
            .role(Role::Application)
            .aria_label("RapidCap recording controls")
            .size_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .border_2()
            .border_color(rgba(0xff2d2dff))
            .bg(rgba(0x111318f5))
            .text_color(rgba(0xffffffff))
            .child(
                div()
                    .id("recording-hud-status")
                    .accessibility_id("rapidcap.hud-status")
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .flex_1()
                    .text_size(px(13.0))
                    .child(status),
            )
            .when(can_pause, |root| {
                root.child(
                    button("rapidcap.hud-pause", pause_label)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
                )
            })
            .child(
                button(
                    "rapidcap.hud-stop",
                    if can_pause { "Stop" } else { "Cancel" },
                )
                .bg(rgba(0xc92a2aff))
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            )
    }
}

pub fn open_recording_hud(
    cx: &mut App,
    controller: Entity<AppController>,
    target: CaptureTarget,
) -> anyhow::Result<WindowHandle<RecordingHud>> {
    let region = match &target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    let width = 420.0;
    let height = 58.0;
    let x = region.x as f32 + (region.width as f32 - width) / 2.0;
    let y = if region.y >= 70 {
        region.y as f32 - 66.0
    } else {
        region.y as f32 + 8.0
    };
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(x), px(y)),
                size: size(px(width), px(height)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Opaque,
            app_id: Some("com.inspire.rapidcap.recording-hud".into()),
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| RecordingHud::new(cx, controller, target)),
    )
}

pub fn close_recording_hud<C: gpui::AppContext>(
    handle: &mut Option<WindowHandle<RecordingHud>>,
    cx: &mut C,
) {
    if let Some(handle) = handle.take() {
        let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
    }
}

pub fn open_recording_border(
    cx: &mut App,
    target: &CaptureTarget,
) -> anyhow::Result<Vec<WindowHandle<RecordingBorder>>> {
    let region = match target {
        CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
    };
    recording_border_regions(region, 4)
        .into_iter()
        .map(|edge| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(edge.x as f32), px(edge.y as f32)),
                        size: size(px(edge.width as f32), px(edge.height as f32)),
                    })),
                    titlebar: None,
                    focus: false,
                    show: true,
                    kind: WindowKind::PopUp,
                    is_movable: false,
                    is_resizable: false,
                    is_minimizable: false,
                    window_background: WindowBackgroundAppearance::Opaque,
                    app_id: Some("com.inspire.rapidcap.recording-border".into()),
                    ..Default::default()
                },
                |_window, cx| cx.new(|_| RecordingBorder),
            )
        })
        .collect()
}

pub fn close_recording_border<C: gpui::AppContext>(
    handles: &mut Vec<WindowHandle<RecordingBorder>>,
    cx: &mut C,
) {
    for handle in handles.drain(..) {
        let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
    }
}

fn recording_border_regions(region: &PhysicalRegion, thickness: u32) -> [PhysicalRegion; 4] {
    let x = region.x - thickness as i32;
    let y = region.y - thickness as i32;
    let width = region.width + thickness * 2;
    [
        PhysicalRegion {
            x,
            y,
            width,
            height: thickness,
        },
        PhysicalRegion {
            x,
            y: region.y + region.height as i32,
            width,
            height: thickness,
        },
        PhysicalRegion {
            x,
            y: region.y,
            width: thickness,
            height: region.height,
        },
        PhysicalRegion {
            x: region.x + region.width as i32,
            y: region.y,
            width: thickness,
            height: region.height,
        },
    ]
}

fn selected_target(
    kind: CaptureKind,
    start: (i32, i32),
    end: (i32, i32),
    hovered: Option<&CaptureTarget>,
) -> Option<CaptureTarget> {
    let dragged = start.0.abs_diff(end.0) >= 6 || start.1.abs_diff(end.1) >= 6;
    if kind != CaptureKind::ActiveWindowScreenshot && dragged {
        PhysicalRegion::from_drag(start, end).map(CaptureTarget::Region)
    } else {
        hovered.cloned()
    }
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
    fn recording_boundary_is_four_thin_edges() {
        let region = PhysicalRegion {
            x: 100,
            y: 200,
            width: 640,
            height: 480,
        };
        assert_eq!(
            recording_border_regions(&region, 4),
            [
                PhysicalRegion {
                    x: 96,
                    y: 196,
                    width: 648,
                    height: 4
                },
                PhysicalRegion {
                    x: 96,
                    y: 680,
                    width: 648,
                    height: 4
                },
                PhysicalRegion {
                    x: 96,
                    y: 200,
                    width: 4,
                    height: 480
                },
                PhysicalRegion {
                    x: 740,
                    y: 200,
                    width: 4,
                    height: 480
                },
            ]
        );
    }
}
