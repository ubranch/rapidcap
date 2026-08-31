//! Window manipulation on macOS, via AppKit and the window server.
//!
//! The Windows sibling drives HWNDs directly because GPUI exposes none of this
//! on Windows; the same is true here, so the two files keep the same shape - a
//! remembered panel window, a lazily built recording frame, and free functions
//! that no-op when the window they need is not up yet.
//!
//! Three things differ in every function and are worth reading once rather than
//! rediscovering. AppKit is main-thread-only, so each entry point takes a
//! `MainThreadMarker` and gives up without one. AppKit measures y upwards from
//! the bottom-left of the primary screen while capture rectangles measure it
//! downwards from the top-left, so anything crossing that boundary goes through
//! `flip_y`. And macOS measures every rectangle in points while a
//! `PhysicalRegion` is device pixels, so the same boundary also goes through
//! `display_scale` - together, that is `appkit_frame`.

use std::{
    ffi::c_void,
    os::unix::io::AsRawFd as _,
    path::Path,
    process::Command,
    ptr::NonNull,
    sync::{
        OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
};

use anyhow::Context as _;
use objc2::{MainThreadMarker, MainThreadOnly as _, rc::Retained};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSScreen, NSView, NSWindow, NSWindowButton,
    NSWindowCollectionBehavior, NSWindowLevel, NSWindowSharingType, NSWindowStyleMask,
};
use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CGPoint, CGRect};
use objc2_core_graphics::{
    CGColor, CGDisplayBounds, CGEvent, CGGetActiveDisplayList, CGGetDisplaysWithPoint,
    CGMainDisplayID, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerName,
    kCGWindowOwnerPID,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use rapidcap_capture::{CaptureTarget, PhysicalRegion, display_scale};

use gpui::DisplayId;

/// The panel's `NSView`, handed over by GPUI at startup.
///
/// GPUI hands out the view rather than the window - `raw-window-handle`'s
/// `AppKitWindowHandle` carries `ns_view` and nothing else - so the window is
/// re-derived on each use. That is a pointer chase, and it stays correct if
/// AppKit ever moves the view to a different window.
static PANEL_VIEW: AtomicIsize = AtomicIsize::new(0);

pub fn remember_main_window(handle: isize) {
    PANEL_VIEW.store(handle, Ordering::Relaxed);
}

/// The panel's window, or `None` before GPUI has handed the view over.
fn panel(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    view_window(PANEL_VIEW.load(Ordering::Relaxed), mtm)
}

/// The window owning an `NSView` pointer GPUI handed out.
fn view_window(handle: isize, _mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    if handle == 0 {
        return None;
    }
    // SAFETY: the pointer came from GPUI's `AppKitWindowHandle` for a window
    // this process owns and does not close before exit, and the marker proves
    // we are on the thread that owns it.
    let view: &NSView = unsafe { &*(handle as *const NSView) };
    view.window()
}

/// Converts a top-left-origin y to AppKit's bottom-left-origin y.
///
/// The reference is the *primary* screen, which is what AppKit measures every
/// window frame against, not whichever screen the rectangle lands on.
fn flip_y(mtm: MainThreadMarker, top: f64, height: f64) -> f64 {
    let primary = NSScreen::screens(mtm)
        .firstObject()
        .map(|screen| screen.frame().size.height)
        .unwrap_or_default();
    primary - top - height
}

/// A capture rectangle as an AppKit window frame: pixels to points, and
/// top-left origin to bottom-left.
///
/// Capture rectangles are device pixels and AppKit frames are points, so
/// everything crossing that boundary goes through [`display_scale`] as well as
/// [`flip_y`]. `None` means there is no display to read a scale off, which is
/// also no place to put a window.
fn appkit_frame(mtm: MainThreadMarker, x: f64, y: f64, width: f64, height: f64) -> Option<NSRect> {
    let scale = f64::from(display_scale()?);
    let height = height / scale;
    Some(NSRect::new(
        NSPoint::new(x / scale, flip_y(mtm, y / scale, height)),
        NSSize::new(width / scale, height),
    ))
}

/// `NSFloatingWindowLevel`, which AppKit spells as a plain integer rather than
/// an enum. The recording frame is the only window that wants it.
const FLOATING: NSWindowLevel = 3;

/// Centre the panel at an exact client size.
///
/// Unlike the Windows sibling there is no DPI factor to apply and no invisible
/// resize border to measure out of the frame: AppKit lays out in points and
/// scales to the backing store itself, and a borderless GPUI window has no
/// frame inset to begin with.
pub fn place_main_window(client_width: f32, client_height: f32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = panel(mtm) else { return };
    let size = NSSize::new(f64::from(client_width), f64::from(client_height));
    let Some(screen) = window.screen().or_else(|| NSScreen::mainScreen(mtm)) else {
        return;
    };
    let visible = screen.visibleFrame();
    let origin = NSPoint::new(
        visible.origin.x + (visible.size.width - size.width) / 2.0,
        visible.origin.y + (visible.size.height - size.height) / 2.0,
    );
    window.setFrame_display(NSRect::new(origin, size), true);
}

pub fn lock_window_size() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = panel(mtm) else { return };
    // Dropping the style bit is not enough on its own: a window that is already
    // zoomable keeps a live green button, so the min and max sizes are pinned
    // to the current frame as well.
    window.setStyleMask(window.styleMask() & !NSWindowStyleMask::Resizable);
    let size = window.frame().size;
    window.setMinSize(size);
    window.setMaxSize(size);

    // The traffic lights go with them. GPUI asks for a transparent titlebar so
    // the custom one can draw, but on macOS that leaves the real close,
    // minimise and zoom buttons floating on top of the mark and the wordmark.
    // Hiding rather than disabling, because the panel's own two buttons are not
    // the same actions: minimise sends it to the tray, not the Dock, and close
    // quits. Leaving a greyed-out set of natives beside them would just be two
    // sets of controls for one window.
    for button in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = window.standardWindowButton(button) {
            button.setHidden(true);
        }
    }
}

/// Hands the whole drag to AppKit, and reports no grab to follow up on.
///
/// The Windows sibling has to move the window itself, frame by frame, because
/// neither route Win32 offers survives a custom titlebar. AppKit has the
/// documented one: `performWindowDragWithEvent:` is meant to be called from the
/// mouse-down of a view that stands in for a titlebar, and it runs the drag to
/// mouse-up on its own - with the screen edges, Spaces and Stage Manager that a
/// hand-rolled loop does not get.
///
/// Driving it by hand here was worse than redundant. Every `setFrameOrigin`
/// made AppKit call the window delegate back synchronously, inside the mouse
/// handler GPUI was already borrowing its window from, and each move logged
/// `RefCell already borrowed` and lost the rest of that frame's dispatch.
///
/// `None` is the point: the caller stores it as the grab, so the move handler
/// in `window.rs` has nothing to act on and [`drag_main_window`] never runs.
pub fn window_drag_grab() -> Option<(i32, i32)> {
    let mtm = MainThreadMarker::new()?;
    let window = panel(mtm)?;
    let event = NSApplication::sharedApplication(mtm).currentEvent()?;
    window.performWindowDragWithEvent(&event);
    None
}

/// Unreachable on macOS: [`window_drag_grab`] has already done the whole drag
/// and returned no grab, so `window.rs` never gets as far as calling this.
pub fn drag_main_window(_grab: (i32, i32)) {}

pub fn hide_main_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = panel(mtm) else { return };
    window.orderOut(None);
}

pub fn show_main_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = panel(mtm) else { return };
    // A tray click leaves RapidCap in the background, and a background app
    // cannot raise its own window to the front. Activating first is the
    // supported way to ask - the same problem the Windows sibling solves by
    // bouncing the panel through `HWND_TOPMOST` to beat the foreground lock.
    NSApplication::sharedApplication(mtm).activate();
    window.makeKeyAndOrderFront(None);
}

/// Keeps the recording chrome out of the recording.
///
/// The direct counterpart of the Windows side's `WDA_EXCLUDEFROMCAPTURE`:
/// AppKit's own words for `NSWindowSharingNone` are "the content cannot be
/// captured". The recorder is FFmpeg reading an `avfoundation` screen device,
/// which is a whole display rather than a window list, so this is the only
/// lever - there is nothing to pass a per-window filter to.
///
/// AppKit warns that a non-sharing window drops out of "a number of system
/// services". Both windows this is used on are chrome that nothing else has
/// any business reading: a borderless frame that ignores mouse events, and the
/// HUD that floats over the region for the length of the take.
pub fn exclude_from_capture(handle: isize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = view_window(handle, mtm) else {
        return;
    };
    window.setSharingType(NSWindowSharingType::None);
}

/// Moves an arbitrary GPUI window, given the `NSView` GPUI handed out for it.
///
/// The overlay uses this to cover one monitor exactly, in capture coordinates,
/// so the rectangle is flipped on the way in like every other one.
pub fn place_window(handle: isize, x: i32, y: i32, width: i32, height: i32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = view_window(handle, mtm) else {
        return;
    };
    let Some(frame) = appkit_frame(
        mtm,
        f64::from(x),
        f64::from(y),
        f64::from(width),
        f64::from(height),
    ) else {
        return;
    };
    // AppKit refuses to let a *titled* window cover the menu bar: ordering one
    // on screen quietly slides it down by the menu bar's height, and every
    // later `setFrame:` is constrained the same way. GPUI builds even a
    // titlebar-less window as `Titled | FullSizeContentView`, so an overlay
    // asked to cover the display landed 33 points low and took the highlight
    // drawn inside it along. Measured: dropping the titled bit lands the same
    // frame at the origin, and GPUI overrides `canBecomeKeyWindow`, so a
    // borderless window still receives the Esc that closes the overlay.
    window.setStyleMask(window.styleMask() & !NSWindowStyleMask::Titled);
    // Both windows placed here are transparent chrome, and AppKit draws a
    // shadow around the *window* rather than what is painted inside it: the
    // recording HUD's pill sat inside a rectangular halo the size of its
    // window, and a full-screen overlay has nothing to cast one onto anyway.
    window.setHasShadow(false);
    window.setFrame_display(frame, true);
}

/// The recording frame: one borderless window, hollow so the recorded content
/// stays visible and clickable through the middle.
static FRAME: OnceLock<FrameWindow> = OnceLock::new();

struct FrameWindow(Retained<NSWindow>);

// SAFETY: the window is only ever reached through `frame_window`, which needs a
// `MainThreadMarker` to be called at all. The bounds exist so the `OnceLock`
// can hold it; they are not a claim that `NSWindow` is thread-safe.
unsafe impl Send for FrameWindow {}
unsafe impl Sync for FrameWindow {}

fn frame_window(mtm: MainThreadMarker) -> &'static NSWindow {
    &FRAME
        .get_or_init(|| {
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            window.setOpaque(false);
            // Clear, not `None`. A nil background colour is not "no fill": the
            // window falls back to AppKit's default, and the frame painted a
            // grey rectangle over the whole region it was supposed to outline.
            let clear = NSColor::clearColor();
            window.setBackgroundColor(Some(&clear));
            window.setHasShadow(false);
            // The frame is decoration, not a target: clicks belong to whatever
            // is being recorded underneath it.
            window.setIgnoresMouseEvents(true);
            // Out of the recording, like the HUD. This window is built here
            // rather than by GPUI, so it never passes through
            // `exclude_from_capture` the way the HUD does.
            window.setSharingType(NSWindowSharingType::None);
            window.setLevel(FLOATING);
            // On every Space, so switching desktops mid-recording does not
            // leave the border behind on the old one.
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            FrameWindow(window)
        })
        .0
}

pub fn show_recording_frame(region: &PhysicalRegion, thickness: u32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let window = frame_window(mtm);
    let Some(frame) = appkit_frame(
        mtm,
        f64::from(region.x),
        f64::from(region.y),
        f64::from(region.width),
        f64::from(region.height),
    ) else {
        return;
    };
    window.setFrame_display(frame, false);
    // The hollow middle is a layer border rather than a cut-out shape: it is
    // the effect the Windows sibling gets from `SetWindowRgn` with `RGN_DIFF`,
    // without needing a custom `NSView` subclass to draw it.
    if let Some(view) = window.contentView() {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setBorderWidth(f64::from(thickness));
            layer.setBorderColor(Some(&CGColor::new_srgb(0.90, 0.22, 0.28, 1.0)));
        }
    }
    window.orderFrontRegardless();
}

pub fn hide_recording_frame() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if FRAME.get().is_some() {
        frame_window(mtm).orderOut(None);
    }
}

/// The topmost window under the cursor that RapidCap does not own.
///
/// `CGWindowListCopyWindowInfo` returns front-to-back, so the first hit is the
/// answer - the same order the Windows sibling gets by walking `GW_HWNDNEXT`
/// down from `GetTopWindow`. Reading it needs Screen Recording permission;
/// without it macOS still lists every window but blanks the owner names, which
/// is why a missing name is an error rather than a skipped entry.
pub fn window_target_at(point: (i32, i32)) -> anyhow::Result<CaptureTarget> {
    let ours = i64::from(std::process::id());
    // The cursor arrives in capture pixels and the window list answers in
    // points, so the hit test happens in points and the rectangle that comes
    // back is scaled up. Skipping this drew the highlight at half the size of
    // the window it was tracking, and picked the wrong window entirely once the
    // cursor was past the middle of the screen.
    let scale = f64::from(display_scale().context("read the display backing scale")?);
    let windows = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    )
    .context("list on-screen windows")?;
    for index in 0..windows.count() {
        // SAFETY: `index` is in range, and every entry in this list is a
        // dictionary by the API's contract.
        let entry = unsafe { &*(windows.value_at_index(index) as *const CFDictionary) };
        // Layer 0 is the ordinary application layer. Anything above it is a
        // menu, a dock tile or a status item, none of which is a target the
        // user can mean by pointing at it.
        if number(entry, unsafe { kCGWindowLayer }) != Some(0)
            || number(entry, unsafe { kCGWindowOwnerPID }) == Some(ours)
        {
            continue;
        }
        let Some(bounds) = bounds(entry) else {
            continue;
        };
        let (x, y) = (f64::from(point.0) / scale, f64::from(point.1) / scale);
        if x < bounds.origin.x
            || x >= bounds.origin.x + bounds.size.width
            || y < bounds.origin.y
            || y >= bounds.origin.y + bounds.size.height
        {
            continue;
        }
        return Ok(CaptureTarget::Window {
            hwnd: number(entry, unsafe { kCGWindowNumber }).unwrap_or_default() as isize,
            region: PhysicalRegion {
                x: (bounds.origin.x * scale) as i32,
                y: (bounds.origin.y * scale) as i32,
                width: (bounds.size.width * scale) as u32,
                height: (bounds.size.height * scale) as u32,
            },
            process_name: string(entry, unsafe { kCGWindowOwnerName })
                .context("read window owner name - grant RapidCap Screen Recording permission")?,
        });
    }
    anyhow::bail!("no window under the cursor")
}

/// A value from a window-list entry, by one of the `kCGWindow*` keys.
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

fn string(entry: &CFDictionary, key: &CFString) -> Option<String> {
    let value = value(entry, key)?;
    // SAFETY: this key is documented to carry a `CFString` value.
    Some(unsafe { &*value.as_ptr().cast::<CFString>() }.to_string())
}

/// `kCGWindowBounds` is a rectangle serialised as a dictionary. It comes back
/// in top-left screen coordinates, the same way round as `PhysicalRegion`, so
/// this is one of the few places that does not flip - but it is in points, so
/// the caller still scales it.
fn bounds(entry: &CFDictionary) -> Option<CGRect> {
    let value = value(entry, unsafe { kCGWindowBounds })?;
    let mut rect = CGRect::default();
    // SAFETY: this key is documented to carry the dictionary form of a rect,
    // and `rect` is a live local.
    let ok = unsafe {
        CGRectMakeWithDictionaryRepresentation(
            Some(&*value.as_ptr().cast::<CFDictionary>()),
            &raw mut rect,
        )
    };
    ok.then_some(rect)
}

/// The display under the cursor, as a GPUI id and a capture rectangle.
pub fn monitor_under_cursor() -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    // A synthesised event reports the cursor in that same top-left space.
    // `NSEvent::mouseLocation` would come back bottom-left and relative to the
    // primary screen, which needs a flip and gets it wrong on a display taller
    // than the primary one.
    let event = CGEvent::new(None).context("read cursor position")?;
    display_at(CGEvent::location(Some(&event)))
}

/// The display a capture rectangle sits on, by its centre.
///
/// The cursor is the wrong question once a selection can cross displays: the
/// pointer has moved on by the time the recording bar opens, and a region
/// dragged across a seam belongs to whichever display holds most of it - which
/// is what the centre point picks.
pub fn monitor_containing(region: &PhysicalRegion) -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    let scale = f64::from(display_scale().context("read the display backing scale")?);
    display_at(CGPoint {
        x: f64::from(region.x + region.width as i32 / 2) / scale,
        y: f64::from(region.y + region.height as i32 / 2) / scale,
    })
}

/// The union of every active display, in the same space `display_at` reports.
///
/// The origin is not (0, 0). A display placed left of or above the main one puts
/// it negative, which is why the selection overlay is positioned from this
/// rectangle rather than sized from it.
pub fn virtual_screen() -> PhysicalRegion {
    // Sixteen is more displays than macOS has ever driven from one machine, so
    // the list never truncates in practice; a machine that somehow beats it
    // simply gets an overlay over the first sixteen.
    let mut displays = [0; 16];
    let mut matched = 0;
    // SAFETY: the count matches the array, and both out pointers live.
    unsafe {
        CGGetActiveDisplayList(
            displays.len() as u32,
            displays.as_mut_ptr(),
            &raw mut matched,
        )
    };
    let scale = f64::from(display_scale().unwrap_or(1.0));
    // `CGRectUnion` has no binding in `objc2-core-foundation`, and four `min`
    // and `max` calls are cheaper to read than an extern declaration would be.
    let (mut left, mut top, mut right, mut bottom) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for display in &displays[..matched as usize] {
        let bounds = CGDisplayBounds(*display);
        left = left.min(bounds.origin.x);
        top = top.min(bounds.origin.y);
        right = right.max(bounds.origin.x + bounds.size.width);
        bottom = bottom.max(bounds.origin.y + bounds.size.height);
    }
    if matched == 0 {
        let bounds = CGDisplayBounds(CGMainDisplayID());
        (left, top) = (bounds.origin.x, bounds.origin.y);
        (right, bottom) = (left + bounds.size.width, top + bounds.size.height);
    }
    PhysicalRegion {
        x: (left * scale) as i32,
        y: (top * scale) as i32,
        width: ((right - left) * scale) as u32,
        height: ((bottom - top) * scale) as u32,
    }
}

/// The display containing a point, as a GPUI id and a capture rectangle.
///
/// The id needs no converting - GPUI's macOS `DisplayId` is a
/// `CGDirectDisplayID` - and `CGDisplayBounds` is already the same global
/// top-left space `PhysicalRegion` uses, just measured in points rather than
/// pixels, so the point comes in as points too.
fn display_at(point: CGPoint) -> anyhow::Result<(DisplayId, PhysicalRegion)> {
    let mut display = CGMainDisplayID();
    let mut matched = 0;
    // SAFETY: room for the one display asked for, and both out pointers live.
    unsafe { CGGetDisplaysWithPoint(point, 1, &raw mut display, &raw mut matched) };
    if matched == 0 {
        anyhow::bail!("the point is not on any display");
    }
    let bounds = CGDisplayBounds(display);
    let scale = f64::from(display_scale().context("read the display backing scale")?);
    Ok((
        DisplayId::new(u64::from(display)),
        PhysicalRegion {
            x: (bounds.origin.x * scale) as i32,
            y: (bounds.origin.y * scale) as i32,
            width: (bounds.size.width * scale) as u32,
            height: (bounds.size.height * scale) as u32,
        },
    ))
}

/// One RapidCap at a time.
///
/// The Windows sibling uses a named mutex; the portable equivalent is an
/// exclusive lock on a file in the app's own support directory, which the
/// kernel drops for us however the process ends - including a crash, where a
/// lock file checked by contents would strand every later launch.
pub struct SingleInstance(
    /// Held open for the life of the process: closing it releases the lock.
    #[expect(dead_code, reason = "the open descriptor is the lock")]
    std::fs::File,
);

impl SingleInstance {
    pub fn acquire() -> anyhow::Result<Option<Self>> {
        let path = std::env::temp_dir().join("com.inspire.rapidcap.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .context("open RapidCap instance lock")?;
        // SAFETY: a plain `flock` on a descriptor this function owns.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Ok(None);
        }
        Ok(Some(Self(file)))
    }
}

/// Hand a folder or a file to the OS. `open(1)` decides which is which.
pub fn open_path(path: &Path) -> anyhow::Result<()> {
    // `open` is the documented entry point and handles bundle activation and
    // Finder reuse itself, which is all `NSWorkspace` would do from here.
    let status = Command::new("/usr/bin/open")
        .arg(path)
        .status()
        .context("launch open(1)")?;
    if !status.success() {
        anyhow::bail!("open(1) failed with {status}");
    }
    Ok(())
}

/// The text-scale multiplier the titlebar is drawn at.
///
/// macOS has no global text size the way Windows Settings › Accessibility does
/// — Display › Text Size is per-app and reaches AppKit controls, not a view an
/// app draws for itself. So the bar is drawn at its authored size here.
pub fn text_scale() -> f32 {
    1.0
}
