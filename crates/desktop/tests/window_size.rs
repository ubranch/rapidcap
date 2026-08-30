#![cfg(windows)]

use std::{
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{FindWindowW, GetWindowRect, GetWindowThreadProcessId},
    },
    core::{PCWSTR, w},
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn silent_window_keeps_compact_bounds_before_first_show() {
    let child = Command::new(env!("CARGO_BIN_EXE_RapidCap"))
        .arg("--silent")
        .spawn()
        .unwrap();
    let process_id = child.id();
    let mut guard = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_size = None;

    loop {
        if let Ok(window) = unsafe { FindWindowW(PCWSTR::null(), w!("RapidCap")) } {
            let mut owner = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
            if owner == process_id {
                let mut rect = RECT::default();
                unsafe { GetWindowRect(window, &mut rect) }.unwrap();
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                last_size = Some((width, height));
                // The bug this catches is the panel never being resized at all,
                // which leaves GPUI's own 1024x768 default on screen. Stated
                // against that rather than against the panel's own size: the
                // panel is drawn in design pixels scaled by the system text
                // size, so a fixed number here fails on a machine whose slider
                // has been moved rather than on a window that is wrong.
                if width < 1024 && height < 768 {
                    return;
                }
            }
        }
        // Polled rather than probed once up front: the child takes the
        // single-instance mutex a moment after `spawn` returns, so an early
        // check would race it. Without this the same failure surfaces as a
        // three second timeout that blames the window size.
        assert!(
            guard.0.try_wait().ok().flatten().is_none(),
            "RapidCap exited before showing a window - another instance already \
             holds the single-instance mutex, so close the running app first"
        );
        assert!(
            Instant::now() < deadline,
            "hidden RapidCap settled at {last_size:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}
