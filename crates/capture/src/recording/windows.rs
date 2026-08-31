//! Recording on Windows: Desktop Duplication into NVENC.

use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

use super::{RecordingError, recording_error};
use crate::{CaptureKind, PhysicalRegion, Settings};

pub(super) const FFMPEG_EXE: &str = "ffmpeg.exe";

/// Nothing beyond `PATH`: a Windows process inherits the machine and user
/// `PATH` however it was started, so there is no launcher-specific gap to fill.
pub(super) const EXTRA_SEARCH_DIRS: &[&str] = &[];

/// FFmpeg is a console subsystem program, so without this a black window blinks
/// up over whatever is being recorded, in the first frames of the recording.
pub(super) fn hide_console(command: &mut Command) {
    command.creation_flags(0x0800_0000);
}

pub(super) fn resolve_shim(executable: &Path) -> PathBuf {
    let Some(target) = fs::read_to_string(executable.with_extension("shim"))
        .ok()
        .and_then(|line| {
            line.trim()
                .strip_prefix("path = \"")
                .and_then(|path| path.strip_suffix('"'))
                .map(PathBuf::from)
        })
    else {
        return executable.to_owned();
    };
    if target.is_file() {
        target
    } else {
        executable.to_owned()
    }
}

/// Where Desktop Duplication has to be pointed: which DXGI output, and the
/// crop within it.
///
/// `ddagrab` reads one adapter output at a time and its offsets are local to
/// that output, where gdigrab took virtual-desktop coordinates. A region is
/// therefore resolved against the output it overlaps most, which is also how
/// `capture_screenshot` picks a monitor for stills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CaptureSource {
    output_idx: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl CaptureSource {
    pub(super) fn resolve(region: &PhysicalRegion) -> Result<Self, RecordingError> {
        // Adapter 0 is the one `-init_hw_device d3d11va` opens by default, so
        // enumerating its outputs here yields the same indices FFmpeg's
        // `output_idx` counts through.
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(recording_error)?;
        let adapter = unsafe { factory.EnumAdapters1(0) }.map_err(recording_error)?;
        let mut best: Option<(u32, PhysicalRegion, PhysicalRegion)> = None;
        for index in 0.. {
            let Ok(output) = (unsafe { adapter.EnumOutputs(index) }) else {
                break;
            };
            let rect = unsafe { output.GetDesc() }
                .map_err(recording_error)?
                .DesktopCoordinates;
            let bounds = PhysicalRegion {
                x: rect.left,
                y: rect.top,
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            };
            let Some(crop) = region.intersection(bounds.clone()) else {
                continue;
            };
            let area = u64::from(crop.width) * u64::from(crop.height);
            if best
                .as_ref()
                .is_none_or(|(_, _, best)| u64::from(best.width) * u64::from(best.height) < area)
            {
                best = Some((index, bounds, crop));
            }
        }
        let (output_idx, bounds, crop) = best.ok_or_else(|| {
            RecordingError("recording region does not intersect a display".into())
        })?;
        Ok(Self {
            output_idx,
            x: crop.x - bounds.x,
            y: crop.y - bounds.y,
            // H.264 has no odd dimensions, and the `pad` filter that used to
            // round them up cannot run on D3D11 frames. Rounding down loses at
            // most one pixel per axis and never reaches past the output edge.
            width: crop.width & !1,
            height: crop.height & !1,
        })
    }
}

pub(super) fn ffmpeg_args(
    kind: CaptureKind,
    source: &CaptureSource,
    settings: &Settings,
    output: &Path,
) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        // Desktop Duplication hands NVENC frames that never leave the GPU.
        // gdigrab used to BitBlt every frame into system memory on the CPU and
        // could not keep up: a 1080p30 recording arrived at roughly 20 fps with
        // the rest duplicated, for ~4.6s of CPU per 8s of wall clock. The same
        // capture through DDA costs ~0.17s and drops nothing.
        "-init_hw_device".into(),
        "d3d11va=dx".into(),
    ];
    // The soundtrack is a real input, so it has to be declared before the
    // filtergraph and its codec after the output options. Both halves are
    // skipped together: half an audio pipeline makes FFmpeg map a stream that
    // does not exist. Skipping it is also the only recourse when
    // `virtual-audio-capturer` is not installed, which otherwise fails the
    // whole recording on an input the user never asked for.
    let with_audio = kind == CaptureKind::Video && settings.audio.enabled;
    if with_audio {
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-thread_queue_size".into(),
            "1024".into(),
            "-audio_buffer_size".into(),
            "80".into(),
            "-i".into(),
            "audio=virtual-audio-capturer".into(),
        ]);
    }
    // DDA decimates at the source, so a GIF never captures the 15 frames per
    // second it is about to throw away.
    let framerate = if kind == CaptureKind::Gif {
        settings.gif.fps
    } else {
        settings.video.fps
    };
    let mut chain = format!(
        "ddagrab=output_idx={}:framerate={framerate}:video_size={}x{}:offset_x={}:offset_y={}",
        source.output_idx, source.width, source.height, source.x, source.y
    );
    if kind == CaptureKind::Gif {
        // ponytail: direct GIF streams safely; full palette generation requires a second pass.
        // GIF has no GPU encoder, so this is the one path that still pays for a
        // readback into system memory.
        chain.push_str(",hwdownload,format=bgra");
    }
    chain.push_str("[v]");
    args.extend(["-filter_complex".into(), chain, "-map".into(), "[v]".into()]);
    if with_audio {
        // `-filter_complex` switches off automatic stream selection, so the
        // audio input has to be mapped by hand or it is dropped in silence.
        args.extend(["-map".into(), "0:a".into()]);
    }
    if kind == CaptureKind::Video {
        args.extend([
            "-c:v".into(),
            "h264_nvenc".into(),
            "-r".into(),
            settings.video.fps.to_string(),
            "-preset".into(),
            settings.video.preset.clone(),
            "-tune".into(),
            settings.video.tune.clone(),
            "-b:v".into(),
            format!("{}k", settings.video.bitrate / 1000),
            "-movflags".into(),
            "+faststart".into(),
        ]);
        if with_audio {
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-ac".into(),
                settings.audio.channels.to_string(),
                "-b:a".into(),
                format!("{}k", settings.audio.bitrate / 1000),
            ]);
        }
    }
    args.extend(["-y".into(), output.display().to_string()]);
    args
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use crate::{CaptureKind, PhysicalRegion, RecordingSession, Settings};

    use super::*;

    #[test]
    fn video_command_matches_sharex_encoder_and_audio() {
        let args = ffmpeg_args(
            CaptureKind::Video,
            &CaptureSource {
                output_idx: 1,
                x: 12,
                y: 34,
                width: 800,
                height: 601,
            },
            &Settings::default(),
            Path::new("out.part.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-init_hw_device d3d11va=dx"));
        assert!(joined.contains(
            "ddagrab=output_idx=1:framerate=30:video_size=800x601:offset_x=12:offset_y=34[v]"
        ));
        // `-filter_complex` disables automatic stream selection, so both halves
        // of the pipeline have to be mapped explicitly.
        assert!(joined.contains("-map [v] -map 0:a"));
        assert!(joined.contains("audio=virtual-audio-capturer"));
        assert!(joined.contains("-c:v h264_nvenc -r 30 -preset p7 -tune hq -b:v 3000k"));
        assert!(joined.contains("-c:a aac -ac 2 -b:a 128k"));
    }

    #[test]
    fn muting_audio_drops_the_input_and_the_codec_together() {
        let mut settings = Settings::default();
        settings.audio.enabled = false;
        let args = ffmpeg_args(
            CaptureKind::Video,
            &CaptureSource {
                output_idx: 0,
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            },
            &settings,
            Path::new("out.part.mp4"),
        );
        let joined = args.join(" ");
        assert!(!joined.contains("virtual-audio-capturer"), "{joined}");
        assert!(!joined.contains("dshow"), "{joined}");
        // The codec half matters as much as the input half - an `-c:a` with no
        // audio input is what makes FFmpeg abort instead of recording silently.
        assert!(!joined.contains("-c:a"), "{joined}");
        assert!(!joined.contains("0:a"), "nothing left to map: {joined}");
        assert!(
            joined.contains("-c:v h264_nvenc"),
            "video must survive: {joined}"
        );
    }

    #[test]
    fn gif_command_streams_at_sharex_frame_rate() {
        let args = ffmpeg_args(
            CaptureKind::Gif,
            &CaptureSource {
                output_idx: 0,
                x: 0,
                y: 0,
                width: 320,
                height: 200,
            },
            &Settings::default(),
            Path::new("out.part.gif"),
        );
        let joined = args.join(" ");
        // Decimated at the source, not after a readback of frames nobody keeps.
        assert!(joined.contains("framerate=15"), "{joined}");
        assert!(joined.contains(",hwdownload,format=bgra[v]"), "{joined}");
        assert!(!joined.contains("palettegen"));
    }

    #[test]
    fn scoop_shim_resolves_real_ffmpeg_process() {
        let temp = tempfile::tempdir().unwrap();
        let shim_exe = temp.path().join("ffmpeg.exe");
        let real_exe = temp.path().join("apps/ffmpeg/bin/ffmpeg.exe");
        std::fs::create_dir_all(real_exe.parent().unwrap()).unwrap();
        std::fs::write(&shim_exe, b"shim").unwrap();
        std::fs::write(&real_exe, b"real").unwrap();
        std::fs::write(
            temp.path().join("ffmpeg.shim"),
            format!("path = \"{}\"", real_exe.display()),
        )
        .unwrap();
        assert_eq!(resolve_shim(&shim_exe), real_exe);
    }

    #[test]
    #[ignore = "requires interactive desktop, NVENC, FFmpeg, and virtual-audio-capturer"]
    fn real_video_records_and_finalizes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::from_roots(
            temp.path().join("Documents"),
            temp.path().join("Roaming"),
            temp.path().join("Local"),
        );
        let session = RecordingSession::start(
            CaptureKind::Video,
            &crate::CaptureTarget::Region(PhysicalRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            }),
            &Settings::default(),
            &paths,
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let output = session.stop().unwrap();
        assert!(output.is_file());
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
        let probe = Command::new("ffprobe.exe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,codec_type",
                "-of",
                "json",
            ])
            .arg(output)
            .output()
            .unwrap();
        let streams = String::from_utf8(probe.stdout).unwrap();
        assert!(probe.status.success(), "{streams}");
        assert!(streams.contains("h264"), "{streams}");
        assert!(streams.contains("aac"), "{streams}");
    }

    #[test]
    #[ignore = "requires interactive desktop and FFmpeg"]
    fn real_gif_records_and_finalizes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::from_roots(
            temp.path().join("Documents"),
            temp.path().join("Roaming"),
            temp.path().join("Local"),
        );
        let session = RecordingSession::start(
            CaptureKind::Gif,
            &crate::CaptureTarget::Region(PhysicalRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            }),
            &Settings::default(),
            &paths,
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let output = session.stop().unwrap();
        let probe = Command::new("ffprobe.exe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,avg_frame_rate",
                "-of",
                "json",
            ])
            .arg(output)
            .output()
            .unwrap();
        let stream = String::from_utf8(probe.stdout).unwrap();
        assert!(probe.status.success(), "{stream}");
        assert!(stream.contains("gif"), "{stream}");
    }

    #[test]
    #[ignore = "requires interactive desktop, NVENC and FFmpeg"]
    fn a_paused_recording_is_missing_the_paused_seconds() {
        // The bug this guards: pause used to suspend the FFmpeg process where
        // it stood. `ddagrab` generates frames against the wall clock, so on
        // resume it emitted a duplicate for every frame slot the pause had
        // covered, and a paused recording came back running its full wall time
        // with the picture frozen through the middle of it.
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::from_roots(
            temp.path().join("Documents"),
            temp.path().join("Roaming"),
            temp.path().join("Local"),
        );
        let mut session = RecordingSession::start(
            CaptureKind::Video,
            &crate::CaptureTarget::Region(PhysicalRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            }),
            &Settings::default(),
            &paths,
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(2));
        session.pause().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        session.resume().unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let output = session.stop().unwrap();

        let probe = Command::new("ffprobe.exe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&output)
            .output()
            .unwrap();
        let text = String::from_utf8(probe.stdout).unwrap();
        assert!(probe.status.success(), "{text}");
        let seconds: f64 = text.trim().parse().unwrap();
        // Four seconds of frames around a four second pause. Spawning a second
        // segment costs a moment, so the floor is generous; the ceiling is what
        // the assertion is for, and the broken behaviour sat above 8.
        assert!(
            (1.0..6.0).contains(&seconds),
            "a 4s pause around 4s of frames produced {seconds}s of video"
        );
    }
}
