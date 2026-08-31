//! Recording on Windows: Desktop Duplication into NVENC.

use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command},
};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

use super::{RecordingError, recording_error};
use crate::{CaptureKind, PhysicalRegion, Settings};

pub(super) const FFMPEG_EXE: &str = "ffmpeg.exe";

/// The one DirectShow device that hears what the speakers are playing.
///
/// Windows has no loopback capture endpoint of its own that FFmpeg can open, so
/// system audio comes through this virtual driver or not at all. It follows
/// whatever the default playback device is, which is why the menu offers system
/// audio as on or off rather than as a list: there is nothing here to choose
/// between, and offering the speakers by name would offer devices FFmpeg cannot
/// open.
pub(super) const LOOPBACK: &str = "virtual-audio-capturer";

/// Nothing beyond `PATH`: a Windows process inherits the machine and user
/// `PATH` however it was started, so there is no launcher-specific gap to fill.
pub(super) const EXTRA_SEARCH_DIRS: &[&str] = &[];

/// FFmpeg is a console subsystem program, so without this a black window blinks
/// up over whatever is being recorded, in the first frames of the recording.
/// Nothing to add: FFmpeg reads the "q" off its stdin pipe here, and Windows
/// has no signal that asks a process to finish rather than stopping it dead.
pub(super) fn request_stop(_child: &Child) {}

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

/// Every microphone DirectShow will open, by the name it opens under.
///
/// `-list_devices` writes its table to stderr and then exits non-zero, because
/// listing is a side effect of failing to open the dummy input it was given.
/// That exit code is expected; only a listing that cannot be run at all is an
/// error, and an empty one is a machine with no microphone rather than a fault.
///
/// The loopback is filtered out. It is a device in this table like any other,
/// but it is system audio, which the menu offers separately and as a switch.
pub(super) fn microphones() -> Result<Vec<String>, RecordingError> {
    let mut command = Command::new(super::ffmpeg_path()?);
    command.args([
        "-hide_banner",
        "-f",
        "dshow",
        "-list_devices",
        "true",
        "-i",
        "dummy",
    ]);
    hide_console(&mut command);
    let listing = command
        .output()
        .map_err(|error| RecordingError(format!("list DirectShow devices failed: {error}")))?;
    Ok(parse_devices(&String::from_utf8_lossy(&listing.stderr)))
}

/// The audio device names out of an FFmpeg `-list_devices` table.
///
/// Split out so it can be tested against a real listing without a real FFmpeg.
/// Every line that names a device is `... "Name" (audio)` or `(video)`, and the
/// `Alternative name` line under each one is the unique moniker, which is not
/// what `-i audio=` wants and is thrown away. A device name may legitimately
/// contain quotes, so the name runs from the first quote to the last one before
/// the kind rather than to the next quote after the first.
fn parse_devices(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| {
            let head = line.strip_suffix(" (audio)")?;
            let start = head.find('"')? + 1;
            let end = head.rfind('"')?;
            (start <= end).then(|| head[start..end].to_string())
        })
        .filter(|name| name != LOOPBACK)
        .collect()
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
    // A soundtrack is one or two real inputs, so they have to be declared
    // before the filtergraph and their codec after the output options. Every
    // part is skipped together: half an audio pipeline makes FFmpeg map a
    // stream that does not exist. Skipping it is also the only recourse when
    // the loopback driver is not installed, which otherwise fails the whole
    // recording on an input the user never asked for.
    //
    // Loopback first when both are on, so the input indices are stable: a
    // microphone-only recording has its microphone at 0, and a recording with
    // system audio has it at 1.
    let mut sources: Vec<&str> = Vec::new();
    if kind == CaptureKind::Video {
        if settings.audio.enabled {
            sources.push(LOOPBACK);
        }
        sources.extend(settings.audio.microphone.as_deref());
    }
    let with_audio = !sources.is_empty();
    for source in &sources {
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-thread_queue_size".into(),
            "1024".into(),
            "-audio_buffer_size".into(),
            "80".into(),
            "-i".into(),
            format!("audio={source}"),
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
        // GIF has no GPU encoder, so this is the one path that still pays for a
        // readback into system memory.
        chain.push_str(",hwdownload,format=bgra");
        chain.push_str(&super::gif_palette_chain(settings));
    } else {
        chain.push_str("[v]");
    }
    // Two sources are mixed into one track rather than muxed as two. A player
    // that shows only the first stream would play system audio with the
    // narration missing, which is the one combination the user asking for both
    // cannot have meant. `duration=longest` because dshow inputs do not start
    // in the same millisecond and the shorter one must not cut the take.
    if sources.len() > 1 {
        let inputs: String = (0..sources.len())
            .map(|index| format!("[{index}:a]"))
            .collect();
        chain.push_str(&format!(
            ";{inputs}amix=inputs={}:duration=longest[a]",
            sources.len()
        ));
    }
    args.extend(["-filter_complex".into(), chain, "-map".into(), "[v]".into()]);
    if with_audio {
        // `-filter_complex` switches off automatic stream selection, so the
        // audio has to be mapped by hand or it is dropped in silence.
        args.extend([
            "-map".into(),
            if sources.len() > 1 {
                "[a]".into()
            } else {
                "0:a".to_string()
            },
        ]);
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

    /// A real listing, from a machine with a headset, a phone camera, two
    /// virtual cameras and the loopback driver installed.
    const LISTING: &str = r#"[in#0 @ 000002bd] "Ilhamjan's S25 Ultra (Windows Virtual Camera)" (video)
[in#0 @ 000002bd]   Alternative name "@device_pnp_\?\swd#vcamdevapi#b4cf#{65e8773d}\{fcebba03}"
[in#0 @ 000002bd] "screen-capture-recorder" (video)
[in#0 @ 000002bd]   Alternative name "@device_sw_{860BB310}\{4EA69364}"
[in#0 @ 000002bd] "OBS Virtual Camera" (none)
[in#0 @ 000002bd] "Headset (Arctis "Nova" Pro)" (audio)
[in#0 @ 000002bd]   Alternative name "@device_cm_{33D9A762}\wave_{F1D58F97}"
[in#0 @ 000002bd] "virtual-audio-capturer" (audio)
[in#0 @ 000002bd]   Alternative name "@device_sw_{33D9A762}\{8E146464}"
Error opening input file dummy."#;

    #[test]
    fn only_the_audio_devices_come_out_of_a_listing() {
        // The cameras, the `(none)` device, the alternative names and the error
        // line are all skipped. The loopback goes too: it is system audio,
        // which is a switch on its own rather than a row in the microphone
        // list. A name with quotes inside it survives whole, because the name
        // runs to the last quote on the line and not to the second one.
        assert_eq!(
            parse_devices(LISTING),
            vec![r#"Headset (Arctis "Nova" Pro)"#]
        );

        // A machine with no microphone at all, which is a list rather than a
        // failure: nothing here can tell the two apart, and the caller shows
        // "No capture devices found" for the first.
        assert!(parse_devices("Error opening input file dummy.").is_empty());
    }

    #[test]
    fn a_microphone_is_mixed_into_the_system_audio_rather_than_muxed_beside_it() {
        let mut settings = Settings::default();
        settings.audio.microphone = Some("Headset (Arctis Nova Pro)".into());
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
        // Loopback first, so the two inputs are numbered the same way every
        // time this pair is on.
        assert!(
            joined.contains("-i audio=virtual-audio-capturer"),
            "{joined}"
        );
        assert!(
            joined.contains("-i audio=Headset (Arctis Nova Pro)"),
            "{joined}"
        );
        assert!(
            joined.contains("[0:a][1:a]amix=inputs=2:duration=longest[a]"),
            "{joined}"
        );
        assert!(joined.contains("-map [v] -map [a]"), "{joined}");
        assert!(joined.contains("-c:a aac"), "{joined}");

        // System audio off leaves the microphone alone at input 0, mapped
        // straight rather than through a one-input mixer.
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
        assert!(!joined.contains("amix"), "{joined}");
        assert!(joined.contains("-map [v] -map 0:a"), "{joined}");
    }

    #[test]
    fn a_microphone_is_not_recorded_into_a_screenshot_or_a_gif() {
        // The soundtrack is a video-only pipeline, and a microphone chosen for
        // a recording must not add an input to a format that cannot carry one.
        let mut settings = Settings::default();
        settings.audio.microphone = Some("Headset".into());
        for kind in [CaptureKind::Gif, CaptureKind::RegionScreenshot] {
            let args = ffmpeg_args(
                kind,
                &CaptureSource {
                    output_idx: 0,
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                &settings,
                Path::new("out.part"),
            );
            let joined = args.join(" ");
            assert!(!joined.contains("audio="), "{kind:?}: {joined}");
            assert!(!joined.contains("amix"), "{kind:?}: {joined}");
        }
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
        assert!(joined.contains(",hwdownload,format=bgra,split"), "{joined}");
        // The 256 colours are chosen from the footage and the frames are
        // diffused onto them, or a recorded gradient arrives banded.
        assert!(
            joined.contains(
                "palettegen=stats_mode=full[pal];[pu][pal]paletteuse=dither=sierra2_4a[v]"
            ),
            "{joined}"
        );
        // One palette for the whole stream, so `new` stays off: with it,
        // paletteuse reads the next frame of video as a palette.
        assert!(!joined.contains("new=1"), "{joined}");
    }

    #[test]
    fn a_per_frame_palette_tells_paletteuse_to_expect_one() {
        let mut settings = Settings::default();
        settings.gif.palette_stats_mode = "single".into();
        let args = ffmpeg_args(
            CaptureKind::Gif,
            &CaptureSource {
                output_idx: 0,
                x: 0,
                y: 0,
                width: 320,
                height: 200,
            },
            &settings,
            Path::new("out.part.gif"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("palettegen=stats_mode=single"), "{joined}");
        assert!(
            joined.contains("paletteuse=dither=sierra2_4a:new=1[v]"),
            "{joined}"
        );
    }

    #[test]
    fn a_video_is_not_put_through_the_palette_pass() {
        let args = ffmpeg_args(
            CaptureKind::Video,
            &CaptureSource {
                output_idx: 0,
                x: 0,
                y: 0,
                width: 320,
                height: 200,
            },
            &Settings::default(),
            Path::new("out.part.mp4"),
        );
        let joined = args.join(" ");
        assert!(!joined.contains("palettegen"), "{joined}");
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
