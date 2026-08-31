mod clipboard;
mod geometry;
mod image_file;
mod naming;
mod recording;
mod screenshot;
mod settings;
mod state;

// One screenshot backend per platform, each exporting the same
// `capture_screenshot`. There is no trait: with exactly one implementation
// compiled in per target, the call sites already pin the shared signature and a
// trait would only add a name to indirect through.
#[cfg(target_os = "macos")]
mod sck;
#[cfg(windows)]
mod wgc;

#[cfg(target_os = "macos")]
pub use sck::capture_screenshot;
#[cfg(windows)]
pub use wgc::capture_screenshot;

pub use clipboard::{ClipboardError, write_clipboard, write_clipboard_file};
pub use geometry::{CaptureError, CaptureTarget, CapturedFrame, PhysicalRegion, RawFrame};
pub use image_file::{ImageFileError, save_screenshot};
pub use naming::{NamingError, OutputNamer};
pub use recording::{RecordingError, RecordingSession};
pub use screenshot::{SavedCapture, ScreenshotError, capture_and_save};
pub use settings::{AppPaths, Settings, SettingsError, SettingsStore};
pub use state::{CaptureCommand, CaptureEvent, CaptureKind, CaptureState, StateError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions_reject_concurrent_capture() {
        assert_eq!(
            CaptureState::Idle.start(CaptureKind::Video),
            Ok(CaptureState::Selecting(CaptureKind::Video))
        );
        assert!(
            CaptureState::Finalizing(CaptureKind::Video)
                .start(CaptureKind::Gif)
                .is_err()
        );
    }

    #[test]
    fn recording_pause_resume_and_stop_are_explicit() {
        assert_eq!(
            CaptureState::Recording(CaptureKind::Video).pause(CaptureKind::Video),
            Ok(CaptureState::Paused(CaptureKind::Video))
        );
        assert_eq!(
            CaptureState::Paused(CaptureKind::Video).resume(CaptureKind::Video),
            Ok(CaptureState::Recording(CaptureKind::Video))
        );
        assert_eq!(
            CaptureState::Paused(CaptureKind::Video).stop(CaptureKind::Video),
            Ok(CaptureState::Finalizing(CaptureKind::Video))
        );
    }

    #[test]
    fn sharex_defaults_are_preserved() {
        let settings = Settings::default();
        assert_eq!(settings.screenshot.png_to_jpeg_threshold_bytes, 2_097_152);
        assert_eq!(settings.screenshot.jpeg_quality, 90);
        assert_eq!(settings.video.fps, 30);
        assert_eq!(settings.video.bitrate, 3_000_000);
        assert_eq!(settings.audio.bitrate, 128_000);
        assert_eq!(settings.gif.fps, 15);
        assert_eq!(settings.countdown_seconds, 5);
    }

    #[test]
    fn output_namer_accepts_injected_suffix() {
        assert_eq!(
            OutputNamer::for_test("0000000000")
                .unwrap()
                .file_stem("Code"),
            "Code_0000000000"
        );
    }

    #[test]
    fn output_namer_random_suffix_has_expected_shape() {
        let stem = OutputNamer::random().file_stem("Screen");
        let suffix = stem.strip_prefix("Screen_").unwrap();
        assert_eq!(suffix.len(), 10);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    #[test]
    fn app_paths_keep_capture_config_log_and_temp_roots_separate() {
        let paths = AppPaths::from_roots("C:/Users/me/Documents", "C:/Roaming", "C:/Local");
        assert_eq!(
            paths.capture_root,
            std::path::PathBuf::from("C:/Users/me/Documents/RapidCap/Screenshots")
        );
        assert_eq!(
            paths.settings_file,
            std::path::PathBuf::from("C:/Roaming/RapidCap/settings.json")
        );
        assert_eq!(
            paths.log_dir,
            std::path::PathBuf::from("C:/Local/RapidCap/Logs")
        );
        assert_eq!(
            paths.temp_dir,
            std::path::PathBuf::from("C:/Local/RapidCap/Temp")
        );
    }

    #[test]
    fn app_paths_discover_existing_windows_known_folders() {
        let paths = AppPaths::discover().unwrap();
        assert!(paths.capture_root.is_absolute());
        assert!(paths.settings_file.is_absolute());
        assert!(paths.log_dir.is_absolute());
        assert!(paths.temp_dir.is_absolute());
    }

    #[test]
    fn missing_settings_file_is_created_with_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("RapidCap/settings.json");
        let store = SettingsStore::new(file.clone());
        assert_eq!(store.load().unwrap(), Settings::default());
        assert!(file.is_file());
        assert!(!file.with_extension("json.part").exists());
    }

    #[test]
    fn invalid_settings_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("settings.json");
        std::fs::write(&file, b"{invalid").unwrap();
        let error = SettingsStore::new(file.clone()).load().unwrap_err();
        assert!(matches!(error, SettingsError::Invalid(_)));
        assert_eq!(std::fs::read(&file).unwrap(), b"{invalid");
    }

    #[test]
    fn screenshot_uses_png_below_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("solid");
        let rgba = vec![255_u8; 16 * 16 * 4];
        let saved = save_screenshot(&rgba, 16, 16, &base, 2_097_152, 90).unwrap();
        assert_eq!(saved.extension().unwrap(), "png");
        assert!(saved.is_file());
        assert!(!base.with_extension("part").exists());
    }

    #[test]
    fn screenshot_uses_jpeg_above_threshold() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("forced-jpeg");
        let rgba = vec![127_u8; 16 * 16 * 4];
        let saved = save_screenshot(&rgba, 16, 16, &base, 1, 90).unwrap();
        assert_eq!(saved.extension().unwrap(), "jpg");
        assert!(saved.is_file());
        assert!(!base.with_extension("part").exists());
    }

    #[test]
    fn failed_screenshot_write_leaves_no_final_file() {
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join("not-a-directory");
        std::fs::write(&blocker, b"block").unwrap();
        let base = blocker.join("capture");
        let rgba = vec![0_u8; 4 * 4 * 4];
        assert!(save_screenshot(&rgba, 4, 4, &base, 2_097_152, 90).is_err());
        assert!(!base.with_extension("png").exists());
        assert!(!base.with_extension("jpg").exists());
    }
}
