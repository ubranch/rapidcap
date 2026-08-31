use std::{
    fmt,
    path::{Path, PathBuf},
};

use chrono::{Datelike, Local};

use crate::{AppPaths, CaptureTarget, OutputNamer, Settings, capture_screenshot, save_screenshot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedCapture {
    pub path: PathBuf,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenshotError(String);

impl fmt::Display for ScreenshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ScreenshotError {}

pub fn capture_and_save(
    target: &CaptureTarget,
    settings: &Settings,
    paths: &AppPaths,
) -> Result<SavedCapture, ScreenshotError> {
    let frame = capture_screenshot(target).map_err(screenshot_error)?;
    let process_name = match target {
        CaptureTarget::Window { process_name, .. } => process_name.as_str(),
        CaptureTarget::Region(_) => "Screen",
    };
    let now = Local::now();
    let base = output_base(
        &paths.capture_root,
        now.year(),
        now.month(),
        process_name,
        &OutputNamer::at(now),
    );
    let path = save_screenshot(
        &frame.rgba,
        frame.width,
        frame.height,
        &base,
        settings.screenshot.png_to_jpeg_threshold_bytes,
        settings.screenshot.jpeg_quality,
    )
    .map_err(screenshot_error)?;
    Ok(SavedCapture {
        path,
        rgba: frame.rgba,
        width: frame.width,
        height: frame.height,
    })
}

fn output_base(
    root: &Path,
    year: i32,
    month: u32,
    process_name: &str,
    namer: &OutputNamer,
) -> PathBuf {
    root.join(format!("{year:04}-{month:02}"))
        .join(namer.file_stem(process_name))
}

fn screenshot_error(error: impl fmt::Display) -> ScreenshotError {
    ScreenshotError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::OutputNamer;

    use super::*;

    #[test]
    fn output_base_uses_year_month_process_and_stamp() {
        let base = output_base(
            Path::new("C:/Captures"),
            2026,
            8,
            "Code",
            &OutputNamer::for_test("2026-08-27_14-32-05_a7Kq"),
        );
        assert_eq!(
            base,
            Path::new("C:/Captures/2026-08/Code_2026-08-27_14-32-05_a7Kq")
        );
    }
}
