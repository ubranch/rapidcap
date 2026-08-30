//! The panel comes up at the size it was told to, not at GPUI's default.
//!
//! Both platforms place the window by hand after GPUI has opened it - see
//! `place_main_window` - and the failure being guarded against is that step not
//! running at all, which leaves a 1024x768 window on screen. The window is
//! measured from outside the process, through the window server, because that
//! is the only place the answer is not the one the app believes.

use std::{
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

/// GPUI's own default bounds. The assertion is stated against these rather than
/// against the panel's own size: the panel is drawn in design pixels scaled by
/// the system text size, so a fixed number here fails on a machine whose text
/// slider has been moved rather than on a window that is wrong.
const GPUI_DEFAULT: (i32, i32) = (1024, 768);

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
mod platform {
    use windows::{
        Win32::{
            Foundation::RECT,
            UI::WindowsAndMessaging::{FindWindowW, GetWindowRect, GetWindowThreadProcessId},
        },
        core::{PCWSTR, w},
    };

    /// Found by title and then confirmed by owner, because a RapidCap left
    /// running by hand answers to the same class and title.
    pub fn panel_size(process_id: u32) -> Option<(i32, i32)> {
        let window = unsafe { FindWindowW(PCWSTR::null(), w!("RapidCap")) }.ok()?;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        if owner != process_id {
            return None;
        }
        let mut rect = RECT::default();
        unsafe { GetWindowRect(window, &mut rect) }.ok()?;
        Some((rect.right - rect.left, rect.bottom - rect.top))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{ffi::c_void, ptr::NonNull};

    use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CGRect};
    use objc2_core_graphics::{
        CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
        kCGWindowBounds, kCGWindowLayer, kCGWindowOwnerPID,
    };

    /// The first ordinary window the child owns. There is no lookup by title
    /// here to match the Windows arm - the window server keys on the owner, so
    /// the owner is the whole query and a stray RapidCap cannot answer it.
    pub fn panel_size(process_id: u32) -> Option<(i32, i32)> {
        let theirs = i64::from(process_id);
        // `OptionAll` rather than `OptionOnScreenOnly`: `--silent` opens the
        // panel without ordering it in, so an on-screen query would never see
        // it. The Windows arm has the same reach - `FindWindowW` finds hidden
        // windows too.
        let windows = CGWindowListCopyWindowInfo(
            CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements,
            0,
        )?;
        (0..windows.count()).find_map(|index| {
            // SAFETY: `index` is in range, and every entry in this list is a
            // dictionary by the API's contract.
            let entry = unsafe { &*(windows.value_at_index(index) as *const CFDictionary) };
            // Layer 0 is the ordinary application layer; the status item this
            // app also owns sits above it.
            if number(entry, unsafe { kCGWindowOwnerPID }) != Some(theirs)
                || number(entry, unsafe { kCGWindowLayer }) != Some(0)
            {
                return None;
            }
            let rect = bounds(entry)?;
            Some((rect.size.width as i32, rect.size.height as i32))
        })
    }

    fn value(entry: &CFDictionary, key: &CFString) -> Option<NonNull<c_void>> {
        // SAFETY: a `CFString` key against a `CFDictionary`, which is what every
        // entry in this list is keyed by.
        let value = unsafe { entry.value((key as *const CFString).cast()) };
        NonNull::new(value.cast_mut())
    }

    fn number(entry: &CFDictionary, key: &CFString) -> Option<i64> {
        let value = value(entry, key)?;
        // SAFETY: these keys are documented to carry `CFNumber` values.
        unsafe { &*value.as_ptr().cast::<CFNumber>() }.as_i64()
    }

    fn bounds(entry: &CFDictionary) -> Option<CGRect> {
        let value = value(entry, unsafe { kCGWindowBounds })?;
        let mut rect = CGRect::default();
        // SAFETY: this key is documented to carry the dictionary form of a
        // rect, and `rect` is a live local.
        let ok = unsafe {
            CGRectMakeWithDictionaryRepresentation(
                Some(&*value.as_ptr().cast::<CFDictionary>()),
                &raw mut rect,
            )
        };
        ok.then_some(rect)
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_size = None;

    loop {
        if let Some((width, height)) = platform::panel_size(process_id) {
            last_size = Some((width, height));
            if width < GPUI_DEFAULT.0 && height < GPUI_DEFAULT.1 {
                return;
            }
        }
        // Polled rather than probed once up front: the child takes the
        // single-instance lock a moment after `spawn` returns, so an early
        // check would race it. Without this the same failure surfaces as a
        // timeout that blames the window size.
        assert!(
            guard.0.try_wait().ok().flatten().is_none(),
            "RapidCap exited before showing a window - another instance already \
             holds the single-instance lock, so close the running app first"
        );
        assert!(
            Instant::now() < deadline,
            "hidden RapidCap settled at {last_size:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}
