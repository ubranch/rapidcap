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
    let _guard = ChildGuard(child);
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
                if width <= 500 && height <= 400 {
                    return;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "hidden RapidCap settled at {last_size:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}
