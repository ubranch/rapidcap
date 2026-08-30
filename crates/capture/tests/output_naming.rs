//! Output paths for processes whose name is not a single bare word.
//!
//! `process_name` strips the `.exe`, but plenty of shipped Windows executables
//! still have a dot in what is left: `Microsoft.Photos`, `Microsoft.CmdPal.UI`,
//! `python3.13`. Those are the names that broke the saved filename, so they are
//! the names the suite captures with.

use std::path::Path;

use rapidcap_capture::{OutputNamer, save_screenshot};

const SUFFIX: &str = "0123456789";

/// 16x16 opaque grey - small enough to stay under any sane PNG threshold.
fn pixels() -> Vec<u8> {
    vec![127_u8; 16 * 16 * 4]
}

fn stem(process_name: &str) -> String {
    OutputNamer::for_test(SUFFIX)
        .unwrap()
        .file_stem(process_name)
}

#[test]
fn dotted_process_name_keeps_its_random_suffix() {
    assert_eq!(stem("Microsoft.Photos"), "Microsoft.Photos_0123456789");
}

#[test]
fn a_dotted_process_name_does_not_collapse_the_saved_file_name() {
    // `Path::with_extension` replaces everything after the *last* dot, and in
    // `Microsoft.Photos_0123456789` that is `Photos_0123456789`. Saving through
    // it produced `Microsoft.png`: the suffix vanished and every capture of the
    // Photos app overwrote the one before it.
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join(stem("Microsoft.Photos"));

    let saved = save_screenshot(&pixels(), 16, 16, &base, 2_097_152, 90).unwrap();

    assert_eq!(
        saved.file_name().unwrap(),
        "Microsoft.Photos_0123456789.png",
        "the whole stem has to survive, dots and all"
    );
}

#[test]
fn two_captures_of_a_dotted_process_do_not_overwrite_each_other() {
    let temp = tempfile::tempdir().unwrap();
    let first = save_screenshot(
        &pixels(),
        16,
        16,
        &temp.path().join("Microsoft.Photos_0123456789"),
        2_097_152,
        90,
    )
    .unwrap();
    let second = save_screenshot(
        &pixels(),
        16,
        16,
        &temp.path().join("Microsoft.Photos_9876543210"),
        2_097_152,
        90,
    )
    .unwrap();

    assert_ne!(first, second, "two captures must not land on one path");
    assert!(first.is_file() && second.is_file());
}

#[test]
fn a_dotted_process_name_survives_the_jpeg_branch_too() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("python3.13_0123456789");

    // Threshold of 1 byte forces the JPEG path.
    let saved = save_screenshot(&pixels(), 16, 16, &base, 1, 90).unwrap();

    assert_eq!(saved.file_name().unwrap(), "python3.13_0123456789.jpg");
}

#[test]
fn the_temporary_file_is_cleaned_up_for_dotted_names() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("Microsoft.CmdPal.UI_0123456789");

    save_screenshot(&pixels(), 16, 16, &base, 2_097_152, 90).unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "part"))
        .collect();
    assert!(leftovers.is_empty(), "left a .part behind: {leftovers:?}");
}
