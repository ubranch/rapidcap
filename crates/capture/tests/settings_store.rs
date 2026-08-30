//! Settings persistence and the validation that guards it.
//!
//! The settings file is the one piece of app state that outlives a reinstall
//! and that a user can hand-edit. Every value in it eventually reaches an
//! FFmpeg argument or an encoder call, so a value that survives `load` has to
//! be a value the rest of the app can use.

use rapidcap_capture::{Settings, SettingsError, SettingsStore};

/// One deliberate corruption of an otherwise valid settings file.
type Break = fn(&mut Settings);

fn store() -> (tempfile::TempDir, SettingsStore) {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("settings.json");
    (temp, SettingsStore::new(file))
}

fn write(temp: &tempfile::TempDir, json: &str) {
    std::fs::write(temp.path().join("settings.json"), json).unwrap();
}

#[test]
fn settings_survive_a_save_and_load_round_trip() {
    let (_temp, store) = store();
    let mut settings = Settings {
        countdown_seconds: 3,
        ..Default::default()
    };
    settings.audio.enabled = false;
    settings.video.fps = 60;

    store.save(&settings).unwrap();

    assert_eq!(store.load().unwrap(), settings);
}

#[test]
fn a_save_leaves_no_partial_file_behind() {
    let (temp, store) = store();
    store.save(&Settings::default()).unwrap();
    assert!(!temp.path().join("settings.json.part").exists());
}

#[test]
fn saving_over_an_existing_file_replaces_it_whole() {
    let (temp, store) = store();
    write(&temp, &"x".repeat(64_000));

    store.save(&Settings::default()).unwrap();

    assert_eq!(store.load().unwrap(), Settings::default());
    let raw = std::fs::read(temp.path().join("settings.json")).unwrap();
    assert!(!raw.ends_with(b"xxxx"), "old bytes must not survive the tail");
}

#[test]
fn a_file_from_a_future_schema_is_refused_rather_than_guessed_at() {
    let (temp, store) = store();
    let settings = Settings {
        schema_version: 99,
        ..Default::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    write(&temp, &json);

    let error = store.load().unwrap_err();

    assert!(
        matches!(&error, SettingsError::Invalid(message) if message.contains("99")),
        "the refusal has to name the version it saw: {error:?}"
    );
}

#[test]
fn an_unknown_field_is_refused_rather_than_silently_dropped() {
    // `deny_unknown_fields` is what stops a typo'd key from looking like it
    // took effect. Rewriting the file would erase the user's line without a
    // word, so loading fails and the file is left alone for them to fix.
    let (temp, store) = store();
    let mut value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&Settings::default()).unwrap()).unwrap();
    value["countdown_secondz"] = serde_json::json!(3);
    write(&temp, &value.to_string());

    assert!(matches!(store.load(), Err(SettingsError::Invalid(_))));
    assert!(
        std::fs::read_to_string(temp.path().join("settings.json"))
            .unwrap()
            .contains("countdown_secondz"),
        "the file the user has to fix must still be there"
    );
}

#[test]
fn a_settings_file_written_before_audio_existed_still_loads() {
    // Schema 1 shipped without `audio.enabled`, and back then a recording
    // always carried sound. Its absence has to keep meaning that.
    let (temp, store) = store();
    let mut value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&Settings::default()).unwrap()).unwrap();
    value["audio"].as_object_mut().unwrap().remove("enabled");
    write(&temp, &value.to_string());

    assert!(store.load().unwrap().audio.enabled);
}

#[test]
fn an_out_of_range_jpeg_quality_is_refused_at_both_ends() {
    for quality in [0_u8, 101, 255] {
        let (temp, store) = store();
        let mut settings = Settings::default();
        settings.screenshot.jpeg_quality = quality;
        write(&temp, &serde_json::to_string(&settings).unwrap());

        assert!(
            matches!(store.load(), Err(SettingsError::Invalid(_))),
            "jpeg_quality {quality} must not reach the encoder"
        );
        assert!(
            matches!(store.save(&settings), Err(SettingsError::Invalid(_))),
            "jpeg_quality {quality} must not be writable either"
        );
    }
}

#[test]
fn a_zero_frame_rate_is_refused_for_both_video_and_gif() {
    let cases: [(&str, Break); 2] = [
        ("video", |settings| settings.video.fps = 0),
        ("gif", |settings| settings.gif.fps = 0),
    ];
    for (label, break_it) in cases {
        let mut settings = Settings::default();
        break_it(&mut settings);
        let (temp, store) = store();
        write(&temp, &serde_json::to_string(&settings).unwrap());

        assert!(
            matches!(store.load(), Err(SettingsError::Invalid(_))),
            "{label} fps of 0 must not reach FFmpeg"
        );
    }
}

#[test]
fn a_zero_bitrate_or_channel_count_is_refused() {
    // Every one of these becomes an FFmpeg argument. Zero is not a value FFmpeg
    // accepts, and finding that out at record time costs the take.
    let cases: [(&str, Break); 3] = [
        ("video bitrate", |settings| settings.video.bitrate = 0),
        ("audio bitrate", |settings| settings.audio.bitrate = 0),
        ("audio channels", |settings| settings.audio.channels = 0),
    ];
    for (label, break_it) in cases {
        let mut settings = Settings::default();
        break_it(&mut settings);
        let (temp, store) = store();
        write(&temp, &serde_json::to_string(&settings).unwrap());

        assert!(
            matches!(store.load(), Err(SettingsError::Invalid(_))),
            "{label} of 0 must be refused before a recording starts"
        );
    }
}

#[test]
fn an_empty_encoder_preset_is_refused() {
    // `preset: ""` produces `-preset` followed by the next flag, and FFmpeg
    // reads that flag as the preset name.
    let cases: [(&str, Break); 2] = [
        ("video preset", |settings| settings.video.preset.clear()),
        ("video tune", |settings| settings.video.tune.clear()),
    ];
    for (label, break_it) in cases {
        let mut settings = Settings::default();
        break_it(&mut settings);
        let (temp, store) = store();
        write(&temp, &serde_json::to_string(&settings).unwrap());

        assert!(
            matches!(store.load(), Err(SettingsError::Invalid(_))),
            "an empty {label} must not become an FFmpeg argument"
        );
    }
}

#[test]
fn a_missing_settings_file_is_created_with_defaults_and_reloads_identically() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("nested/dir/settings.json");
    let store = SettingsStore::new(file.clone());

    let created = store.load().unwrap();

    assert_eq!(created, Settings::default());
    assert!(file.is_file(), "load has to write the file it invented");
    assert_eq!(store.load().unwrap(), created);
}

#[test]
fn a_bitrate_that_would_truncate_to_zero_kilobits_is_refused() {
    // The FFmpeg argument is built as `{bitrate / 1000}k`, so 500 bits per
    // second does not become a very low bitrate - it becomes `0k`, and NVENC
    // quietly substitutes its own. The setting looks applied and is not.
    for bitrate in [1_u32, 500, 999] {
        let mut settings = Settings::default();
        settings.video.bitrate = bitrate;
        let (temp, store) = store();
        write(&temp, &serde_json::to_string(&settings).unwrap());

        assert!(
            matches!(store.load(), Err(SettingsError::Invalid(_))),
            "video bitrate {bitrate} truncates to 0k and must be refused"
        );
    }
}

#[test]
fn the_shipped_defaults_pass_their_own_validation() {
    // Every rejection above has to leave the defaults untouched, or the app
    // refuses to start on a clean install.
    let (_temp, store) = store();
    assert!(store.save(&Settings::default()).is_ok());
}
