use std::mem::size_of;

use gpui::{
    App, AppContext as _, Bounds, Context, DisplayId, Entity, FocusHandle, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, actions,
    div, prelude::*, px, rgba,
};
use rapidcap_capture::{CaptureCommand, CaptureTarget, PhysicalRegion};
use windows::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    },
    UI::WindowsAndMessaging::GetCursorPos,
};

use crate::controller::AppController;

actions!(rapidcap_overlay, [CancelSelection]);

pub struct RegionOverlay {
    controller: Entity<AppController>,
    monitor: PhysicalRegion,
    scale_factor: f32,
    start: Option<Point<Pixels>>,
    current: Option<Point<Pixels>>,
    focus_handle: FocusHandle,
}

impl RegionOverlay {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        controller: Entity<AppController>,
        monitor: PhysicalRegion,
    ) -> Self {
        window.set_window_title("RapidCap Selection");
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            controller,
            monitor,
            scale_factor: window.scale_factor(),
            start: None,
            current: None,
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
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(start) = self.start else {
            return;
        };
        let Some(region) = PhysicalRegion::from_drag(
            physical_point(start, &self.monitor, self.scale_factor),
            physical_point(event.position, &self.monitor, self.scale_factor),
        ) else {
            return;
        };
        self.controller.update(cx, |controller, cx| {
            controller.set_target(CaptureTarget::Region(region), cx)
        });
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
        }
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
        |window, cx| cx.new(|cx| RegionOverlay::new(window, cx, controller, monitor)),
    )
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
}
