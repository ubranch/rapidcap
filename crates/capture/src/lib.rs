mod naming;
mod settings;
mod state;

pub use naming::{NamingError, OutputNamer};
pub use settings::{AppPaths, Settings};
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
    fn sharex_defaults_are_preserved() {
        let settings = Settings::default();
        assert_eq!(settings.screenshot.png_to_jpeg_threshold_bytes, 2_097_152);
        assert_eq!(settings.screenshot.jpeg_quality, 90);
        assert_eq!(settings.video.fps, 60);
        assert_eq!(settings.video.bitrate, 3_000_000);
        assert_eq!(settings.audio.bitrate, 128_000);
        assert_eq!(settings.gif.fps, 15);
        assert_eq!(settings.countdown_seconds, 5);
        assert_eq!(settings.hotkeys.video, ["Alt+E", "Shift+Print Screen"]);
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
}
