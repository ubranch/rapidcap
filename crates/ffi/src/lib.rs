//! C surface over `rapidcap-capture`, for the native macOS app.
//!
//! Every call that can fail answers with JSON on the heap: `{"error": "..."}`
//! on failure, a call-specific object on success. Strings returned from here
//! belong to Rust and go back through [`rc_free`]. JSON rather than a C struct
//! per call keeps the header a page long and lets the Swift side decode with
//! `Codable`, which it already does for the settings file itself.
//!
//! Stateless apart from a recording handle: settings are read from disk on
//! every capture, so whatever the Swift side last saved is what the capture
//! uses, with no second copy in here to drift.

use std::{
    ffi::{CStr, CString, c_char},
    fs,
    path::Path,
    process::Command,
    ptr,
};

use rapidcap_capture::{
    AppPaths, CaptureKind, CaptureTarget, PhysicalRegion, RecordingSession, Settings,
    SettingsStore, capture_and_save, encode_gif, ffmpeg_path, media_duration, microphones,
    recording_paths, write_clipboard, write_clipboard_file,
};
use serde_json::{Value, json};

fn into_c(value: Value) -> *mut c_char {
    // JSON never contains an interior NUL - serde escapes it as \u0000.
    CString::new(value.to_string()).map_or(ptr::null_mut(), CString::into_raw)
}

fn error(message: impl ToString) -> *mut c_char {
    into_c(json!({ "error": message.to_string() }))
}

/// # Safety
/// `text` is null or a NUL-terminated string valid for the call.
unsafe fn read(text: *const c_char) -> Option<String> {
    (!text.is_null()).then(|| {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    })
}

fn context() -> Result<(AppPaths, Settings), String> {
    let paths = AppPaths::discover().map_err(|error| error.to_string())?;
    let settings = SettingsStore::new(paths.settings_file.clone())
        .load()
        .map_err(|error| error.to_string())?;
    Ok((paths, settings))
}

fn region(x: i32, y: i32, width: u32, height: u32) -> PhysicalRegion {
    PhysicalRegion {
        x,
        y,
        width,
        height,
    }
}

/// Free a string this library returned. Null is accepted and ignored.
///
/// # Safety
/// `text` came from this library and has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_free(text: *mut c_char) {
    if !text.is_null() {
        drop(unsafe { CString::from_raw(text) });
    }
}

/// `{"settings": {...}, "paths": {...}, "version": "..."}`.
///
/// Creates the settings file with defaults when there is none, which is what
/// the Windows app does on first launch too.
#[unsafe(no_mangle)]
pub extern "C" fn rc_load() -> *mut c_char {
    match context() {
        Ok((paths, settings)) => into_c(json!({
            "settings": settings,
            "paths": {
                "capture_root": paths.capture_root,
                "settings_file": paths.settings_file,
                "temp_dir": paths.temp_dir,
                "thumb_dir": paths.thumb_dir,
            },
            "version": env!("CARGO_PKG_VERSION"),
        })),
        Err(message) => error(message),
    }
}

/// Validate and write the settings file. `{}` on success.
///
/// # Safety
/// `json` is a NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_save_settings(json: *const c_char) -> *mut c_char {
    let Some(text) = (unsafe { read(json) }) else {
        return error("no settings given");
    };
    let settings: Settings = match serde_json::from_str(&text) {
        Ok(settings) => settings,
        Err(problem) => return error(problem),
    };
    let paths = match AppPaths::discover() {
        Ok(paths) => paths,
        Err(problem) => return error(problem),
    };
    match SettingsStore::new(paths.settings_file).save(&settings) {
        Ok(()) => into_c(json!({})),
        Err(problem) => error(problem),
    }
}

/// Capture a region of the desktop, save it, and copy it when the settings
/// say so. The region is in global Core Graphics device pixels: points from
/// the top-left of the main display, times the backing scale.
///
/// `process_name` names the file (`Safari_...png`); null means a region.
/// `{"path": "...", "width": n, "height": n}`.
///
/// # Safety
/// `process_name` is null or a NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_screenshot(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    process_name: *const c_char,
) -> *mut c_char {
    let (paths, settings) = match context() {
        Ok(context) => context,
        Err(message) => return error(message),
    };
    let region = region(x, y, width, height);
    let target = match unsafe { read(process_name) } {
        Some(process_name) => CaptureTarget::Window {
            hwnd: 0,
            region,
            process_name,
        },
        None => CaptureTarget::Region(region),
    };
    let saved = match capture_and_save(&target, &settings, &paths) {
        Ok(saved) => saved,
        Err(problem) => return error(problem),
    };
    // A failed copy does not undo a saved file: the capture is on disk, which
    // is the half that matters, and the Swift side says so.
    let clipboard = if settings.screenshot.copy_to_clipboard {
        write_clipboard(&saved)
            .err()
            .map(|problem| problem.to_string())
    } else {
        None
    };
    into_c(json!({
        "path": saved.path,
        "width": saved.width,
        "height": saved.height,
        "clipboard_error": clipboard,
    }))
}

/// An FFmpeg recording in progress. Opaque to C.
pub struct Recording(RecordingSession);

/// Start recording a region. `kind` 0 is MP4 video, 1 is GIF.
///
/// Returns the handle, or null with `*error_out` set to a JSON error the
/// caller frees.
///
/// # Safety
/// `error_out` is null or points at writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_record_start(
    kind: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    error_out: *mut *mut c_char,
) -> *mut Recording {
    let fail = |message: String| {
        if !error_out.is_null() {
            unsafe { *error_out = error(message) };
        }
        ptr::null_mut()
    };
    let (paths, settings) = match context() {
        Ok(context) => context,
        Err(message) => return fail(message),
    };
    let kind = if kind == 1 {
        CaptureKind::Gif
    } else {
        CaptureKind::Video
    };
    let target = CaptureTarget::Region(region(x, y, width, height));
    match RecordingSession::start(kind, &target, &settings, &paths) {
        Ok(session) => Box::into_raw(Box::new(Recording(session))),
        Err(problem) => fail(problem.to_string()),
    }
}

/// `{}` or an error.
///
/// # Safety
/// `recording` came from [`rc_record_start`] and has not been stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_record_pause(recording: *mut Recording) -> *mut c_char {
    match unsafe { recording.as_mut() } {
        Some(Recording(session)) => match session.pause() {
            Ok(()) => into_c(json!({})),
            Err(problem) => error(problem),
        },
        None => error("no recording"),
    }
}

/// `{}` or an error.
///
/// # Safety
/// `recording` came from [`rc_record_start`] and has not been stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_record_resume(recording: *mut Recording) -> *mut c_char {
    match unsafe { recording.as_mut() } {
        Some(Recording(session)) => match session.resume() {
            Ok(()) => into_c(json!({})),
            Err(problem) => error(problem),
        },
        None => error("no recording"),
    }
}

/// Finish the file and put a reference to it on the clipboard.
/// `{"path": "..."}`. Consumes the handle either way.
///
/// # Safety
/// `recording` came from [`rc_record_start`] and has not been stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_record_stop(recording: *mut Recording) -> *mut c_char {
    if recording.is_null() {
        return error("no recording");
    }
    let Recording(session) = *unsafe { Box::from_raw(recording) };
    match session.stop() {
        Ok(path) => {
            let clipboard = write_clipboard_file(&path)
                .err()
                .map(|problem| problem.to_string());
            into_c(json!({ "path": path, "clipboard_error": clipboard }))
        }
        Err(problem) => error(problem),
    }
}

/// Throw the recording away. Consumes the handle.
///
/// # Safety
/// `recording` came from [`rc_record_start`] and has not been stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_record_cancel(recording: *mut Recording) {
    if !recording.is_null() {
        let Recording(session) = *unsafe { Box::from_raw(recording) };
        let _ = session.cancel();
    }
}

/// Where a recording made outside FFmpeg goes: the macOS app records through
/// ScreenCaptureKit and files it exactly as [`rc_record_start`] would.
/// Creates both directories. A video is written straight to its MP4; a GIF's
/// frames go to a ProRes `.mov` that [`rc_encode_gif`] turns into the GIF.
/// `{"temp": "<temp>/<stem>.part.mp4|mov", "final": "<root>/<yyyy-mm>/<stem>.mp4|gif"}`.
#[unsafe(no_mangle)]
pub extern "C" fn rc_recording_paths(gif: bool) -> *mut c_char {
    let (paths, _) = match context() {
        Ok(context) => context,
        Err(message) => return error(message),
    };
    let (kind, temp) = if gif {
        (CaptureKind::Gif, "part.mov")
    } else {
        (CaptureKind::Video, "part.mp4")
    };
    match recording_paths(kind, &paths) {
        Ok((temp_stem, final_path)) => into_c(json!({
            "temp": temp_stem.with_extension(temp),
            "final": final_path,
        })),
        Err(problem) => error(problem),
    }
}

/// Make `gif` from a finished capture file, with the palette the saved
/// settings name. `{}` on success; on failure the capture file is kept.
///
/// # Safety
/// `source` and `gif` are NUL-terminated strings valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_encode_gif(source: *const c_char, gif: *const c_char) -> *mut c_char {
    let (Some(source), Some(gif)) = (unsafe { read(source) }, unsafe { read(gif) }) else {
        return error("no file given");
    };
    let (_, settings) = match context() {
        Ok(context) => context,
        Err(message) => return error(message),
    };
    let made = ffmpeg_path()
        .and_then(|ffmpeg| encode_gif(&ffmpeg, &settings, Path::new(&source), Path::new(&gif)));
    match made {
        Ok(_) => into_c(json!({})),
        Err(problem) => error(problem),
    }
}

/// `{"microphones": ["..."]}` or an error.
#[unsafe(no_mangle)]
pub extern "C" fn rc_microphones() -> *mut c_char {
    match microphones() {
        Ok(names) => into_c(json!({ "microphones": names })),
        Err(problem) => error(problem),
    }
}

/// Copy a file reference to the clipboard: the gallery's Copy action.
///
/// # Safety
/// `path` is a NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_copy_file(path: *const c_char) -> *mut c_char {
    let Some(path) = (unsafe { read(path) }) else {
        return error("no path given");
    };
    match write_clipboard_file(Path::new(&path)) {
        Ok(()) => into_c(json!({})),
        Err(problem) => error(problem),
    }
}

/// Put a separately recorded audio track into a finished video, in place.
///
/// The macOS recorder takes the picture through FFmpeg and the system audio
/// through ScreenCaptureKit, so the two meet here after both have stopped.
/// They stop together but do not start together - FFmpeg's first frame comes
/// a moment after its launch - so the track is lined up by its end: trimmed at
/// the front when it runs longer than the video, delayed when it runs shorter.
/// `{}` on success; on failure the video is left as it was, without sound.
///
/// # Safety
/// `video` and `audio` are NUL-terminated strings valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rc_mux_audio(
    video: *const c_char,
    audio: *const c_char,
    audio_seconds: f64,
) -> *mut c_char {
    let (Some(video), Some(audio)) = (unsafe { read(video) }, unsafe { read(audio) }) else {
        return error("no file given");
    };
    match mux_audio(Path::new(&video), Path::new(&audio), audio_seconds) {
        Ok(()) => into_c(json!({})),
        Err(problem) => error(problem),
    }
}

fn mux_audio(video: &Path, audio: &Path, audio_seconds: f64) -> Result<(), String> {
    let ffmpeg = ffmpeg_path().map_err(|problem| problem.to_string())?;
    let late =
        media_duration(&ffmpeg, video).map_or(0.0, |video| video.as_secs_f64() - audio_seconds);
    // Positive: the audio began after the first frame and waits that long.
    // Negative: it began before, and that much is cut from its front.
    let align = if late >= 0.0 {
        ["-itsoffset".to_string(), format!("{late:.3}")]
    } else {
        ["-ss".to_string(), format!("{:.3}", -late)]
    };
    let joined = video.with_extension("audio.mp4");
    let output = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args(align)
        .arg("-i")
        .arg(audio)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c",
            "copy",
            "-movflags",
            "+faststart",
        ])
        .arg(&joined)
        .output()
        .map_err(|problem| format!("ffmpeg did not start: {problem}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&joined);
        return Err(format!(
            "adding audio failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    fs::rename(&joined, video).map_err(|problem| format!("adding audio failed: {problem}"))?;
    let _ = fs::remove_file(audio);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(text: *mut c_char) -> Value {
        let value = serde_json::from_str(&unsafe { CStr::from_ptr(text) }.to_string_lossy())
            .expect("valid JSON");
        unsafe { rc_free(text) };
        value
    }

    #[test]
    fn errors_are_json_objects() {
        assert_eq!(take(error("boom")), json!({ "error": "boom" }));
    }

    #[test]
    fn settings_that_fail_to_parse_are_reported_not_saved() {
        let input = CString::new("{\"schema_version\": 1}").unwrap();
        let answer = take(unsafe { rc_save_settings(input.as_ptr()) });
        assert!(answer["error"].is_string(), "{answer}");
    }

    #[test]
    fn a_null_recording_is_an_error_not_a_crash() {
        assert!(take(unsafe { rc_record_stop(ptr::null_mut()) })["error"].is_string());
        assert!(take(unsafe { rc_record_pause(ptr::null_mut()) })["error"].is_string());
        unsafe { rc_record_cancel(ptr::null_mut()) };
    }
}
