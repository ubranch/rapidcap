//! Recording on macOS: AVFoundation into VideoToolbox.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use core_graphics::display::CGDisplay;

use super::RecordingError;
use crate::{CaptureKind, PhysicalRegion, Settings, display_scale};

pub(super) const FFMPEG_EXE: &str = "ffmpeg";

/// FFmpeg spawned from a bundle never gets a console to hide, so unlike the
/// Windows backend there is nothing to suppress here.
pub(super) fn hide_console(_command: &mut Command) {}

/// macOS has no equivalent of scoop's shim files - a `ffmpeg` found on PATH is
/// already the real binary, or a symlink the kernel resolves for us.
pub(super) fn resolve_shim(executable: &Path) -> PathBuf {
    executable.to_owned()
}

/// Which AVFoundation screen device FFmpeg is pointed at, and the crop within
/// it - the counterpart to the Windows backend's DXGI output index.
///
/// AVFoundation numbers its screen devices from the same list Core Graphics
/// enumerates, but its `-i` index counts *cameras first*: on a MacBook the
/// built-in FaceTime camera is device 0 and "Capture screen 0" is device 1.
/// The offset is discovered rather than assumed, by asking FFmpeg to list the
/// devices it can see, so an external webcam cannot shift the screens out from
/// under us.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CaptureSource {
    device_index: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl CaptureSource {
    pub(super) fn resolve(region: &PhysicalRegion) -> Result<Self, RecordingError> {
        let displays = CGDisplay::active_displays()
            .map_err(|error| RecordingError(format!("enumerate displays failed: {error}")))?;
        // AVFoundation hands FFmpeg the display at its native resolution, so
        // the crop filter counts pixels. `CGDisplay::bounds` counts points, and
        // on a Retina display an unconverted crop would be half the size the
        // user selected, in the top-left quarter of what they picked.
        let scale = f64::from(
            display_scale().ok_or_else(|| RecordingError("no display to record from".into()))?,
        );
        let mut best: Option<(usize, PhysicalRegion, PhysicalRegion)> = None;
        for (screen_index, id) in displays.into_iter().enumerate() {
            let rect = CGDisplay::new(id).bounds();
            let bounds = PhysicalRegion {
                x: (rect.origin.x * scale) as i32,
                y: (rect.origin.y * scale) as i32,
                width: (rect.size.width * scale) as u32,
                height: (rect.size.height * scale) as u32,
            };
            let Some(crop) = region.intersection(bounds.clone()) else {
                continue;
            };
            let area = u64::from(crop.width) * u64::from(crop.height);
            if best
                .as_ref()
                .is_none_or(|(_, _, best)| u64::from(best.width) * u64::from(best.height) < area)
            {
                best = Some((screen_index, bounds, crop));
            }
        }
        let (screen_index, bounds, crop) = best.ok_or_else(|| {
            RecordingError("recording region does not intersect a display".into())
        })?;
        Ok(Self {
            device_index: first_screen_device()? + screen_index as u32,
            x: crop.x - bounds.x,
            y: crop.y - bounds.y,
            // H.264 has no odd dimensions, and rounding down loses at most one
            // pixel per axis and never reaches past the display edge.
            width: crop.width & !1,
            height: crop.height & !1,
        })
    }
}

/// The AVFoundation device index of "Capture screen 0".
///
/// `-list_devices` writes its table to stderr and then exits non-zero, because
/// listing is a side effect of failing to open the empty input it was given.
/// That exit code is expected, so only a missing table is an error.
fn first_screen_device() -> Result<u32, RecordingError> {
    let listing = Command::new(super::ffmpeg_path()?)
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .output()
        .map_err(|error| RecordingError(format!("list AVFoundation devices failed: {error}")))?;
    let listing = String::from_utf8_lossy(&listing.stderr);
    listing
        .lines()
        .find(|line| line.contains("Capture screen 0"))
        .and_then(|line| {
            let index = line.split_once("] [")?.1;
            index.split_once(']')?.0.parse().ok()
        })
        .ok_or_else(|| {
            RecordingError(
                "AVFoundation listed no screen device - grant RapidCap Screen Recording                  permission in System Settings > Privacy & Security"
                    .into(),
            )
        })
}

pub(super) fn ffmpeg_args(
    kind: CaptureKind,
    source: &CaptureSource,
    settings: &Settings,
    output: &Path,
) -> Vec<String> {
    let framerate = if kind == CaptureKind::Gif {
        settings.gif.fps
    } else {
        settings.video.fps
    };
    // There is no audio half. macOS exposes no system-audio input device at
    // all: `-i "<screen>:<audio>"` can only name a *microphone*, and recording
    // the room instead of the app is not what `settings.audio.enabled` asks
    // for. Capturing system sound needs either a loopback driver the user
    // installs or ScreenCaptureKit taking the audio itself, so the setting is
    // deliberately inert here rather than quietly recording the wrong source.
    //
    // AVFoundation captures a whole display, so unlike `ddagrab` the crop is a
    // filter rather than an input option.
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-f".into(),
        "avfoundation".into(),
        "-capture_cursor".into(),
        "0".into(),
        "-framerate".into(),
        framerate.to_string(),
        "-i".into(),
        format!("{}:", source.device_index),
    ];
    args.extend([
        "-filter_complex".into(),
        format!(
            "[0:v]crop={}:{}:{}:{}[v]",
            source.width, source.height, source.x, source.y
        ),
        "-map".into(),
        "[v]".into(),
    ]);
    if kind == CaptureKind::Video {
        args.extend([
            "-c:v".into(),
            "h264_videotoolbox".into(),
            "-r".into(),
            settings.video.fps.to_string(),
            "-b:v".into(),
            format!("{}k", settings.video.bitrate / 1000),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    }
    args.extend(["-y".into(), output.display().to_string()]);
    args
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::Settings;

    fn source() -> CaptureSource {
        CaptureSource {
            device_index: 1,
            x: 12,
            y: 34,
            width: 800,
            height: 600,
        }
    }

    #[test]
    fn video_encodes_on_videotoolbox_and_crops_in_the_filtergraph() {
        let args = ffmpeg_args(
            CaptureKind::Video,
            &source(),
            &Settings::default(),
            Path::new("out.part.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-f avfoundation"));
        // The device index is the input, and the crop is a filter, because
        // AVFoundation only ever hands over a whole display.
        assert!(joined.contains("-i 1:"));
        assert!(joined.contains("[0:v]crop=800:600:12:34[v]"));
        assert!(joined.contains("-map [v]"));
        assert!(joined.contains("-c:v h264_videotoolbox"));
    }

    #[test]
    fn a_gif_captures_at_its_own_framerate_and_names_no_encoder() {
        let settings = Settings::default();
        let args = ffmpeg_args(
            CaptureKind::Gif,
            &source(),
            &settings,
            Path::new("out.part.gif"),
        );
        let joined = args.join(" ");
        assert!(joined.contains(&format!("-framerate {}", settings.gif.fps)));
        assert!(!joined.contains("-c:v"));
    }

    #[test]
    fn no_audio_input_is_declared_even_when_the_setting_is_on() {
        // macOS exposes no system-audio device, so `audio.enabled` cannot be
        // honoured here. The guarantee worth testing is that it never silently
        // records the microphone instead.
        let mut settings = Settings::default();
        settings.audio.enabled = true;
        let joined = ffmpeg_args(
            CaptureKind::Video,
            &source(),
            &settings,
            Path::new("out.part.mp4"),
        )
        .join(" ");
        assert!(!joined.contains("-map 0:a"));
        assert!(!joined.contains("-c:a"));
    }
}
