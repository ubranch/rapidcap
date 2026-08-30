use std::{
    fmt, fs,
    io::Write,
    os::windows::io::AsRawHandle,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::HANDLE,
    Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1},
    System::SystemInformation::GetLocalTime,
};

use crate::{AppPaths, CaptureKind, CaptureTarget, OutputNamer, PhysicalRegion, Settings};

pub struct RecordingSession {
    child: Child,
    temp_path: PathBuf,
    final_path: PathBuf,
    paused: bool,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> i32;
    fn NtResumeProcess(process: HANDLE) -> i32;
}

impl RecordingSession {
    pub fn start(
        kind: CaptureKind,
        target: &CaptureTarget,
        settings: &Settings,
        paths: &AppPaths,
    ) -> Result<Self, RecordingError> {
        let region = match target {
            CaptureTarget::Region(region) | CaptureTarget::Window { region, .. } => region,
        };
        if !matches!(kind, CaptureKind::Video | CaptureKind::Gif) {
            return Err(RecordingError("unsupported recording kind".into()));
        }
        fs::create_dir_all(&paths.temp_dir).map_err(recording_error)?;
        let now = unsafe { GetLocalTime() };
        let directory = paths
            .capture_root
            .join(format!("{:04}-{:02}", now.wYear, now.wMonth));
        fs::create_dir_all(&directory).map_err(recording_error)?;
        let extension = if kind == CaptureKind::Video {
            "mp4"
        } else {
            "gif"
        };
        let stem = OutputNamer::random().file_stem("Screen");
        let final_path = directory.join(format!("{stem}.{extension}"));
        let temp_path = paths.temp_dir.join(format!("{stem}.part.{extension}"));
        let source = DdaSource::resolve(region)?;
        let ffmpeg = ffmpeg_path()?;
        let child = Command::new(ffmpeg)
            .args(ffmpeg_args(kind, &source, settings, &temp_path))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(recording_error)?;
        Ok(Self {
            child,
            temp_path,
            final_path,
            paused: false,
        })
    }

    pub fn pause(&mut self) -> Result<(), RecordingError> {
        if self.paused {
            return Ok(());
        }
        let status = unsafe { NtSuspendProcess(HANDLE(self.child.as_raw_handle())) };
        if status < 0 {
            return Err(RecordingError(format!(
                "pause FFmpeg failed: NTSTATUS {status:#x}"
            )));
        }
        self.paused = true;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), RecordingError> {
        if !self.paused {
            return Ok(());
        }
        let status = unsafe { NtResumeProcess(HANDLE(self.child.as_raw_handle())) };
        if status < 0 {
            return Err(RecordingError(format!(
                "resume FFmpeg failed: NTSTATUS {status:#x}"
            )));
        }
        self.paused = false;
        Ok(())
    }

    pub fn cancel(mut self) -> Result<(), RecordingError> {
        self.resume()?;
        if self.child.try_wait().map_err(recording_error)?.is_none() {
            self.child.kill().map_err(recording_error)?;
        }
        self.child.wait().map_err(recording_error)?;
        match fs::remove_file(&self.temp_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(recording_error(error)),
        }
    }

    pub fn stop(mut self) -> Result<PathBuf, RecordingError> {
        self.resume()?;
        if self.child.try_wait().map_err(recording_error)?.is_none()
            && let Some(mut stdin) = self.child.stdin.take()
        {
            let _ = stdin.write_all(b"q\n");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().map_err(recording_error)? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(RecordingError(format!(
                    "FFmpeg stop timed out; temporary output preserved at {}",
                    self.temp_path.display()
                )));
            }
            thread::sleep(Duration::from_millis(25));
        };
        let size = fs::metadata(&self.temp_path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        if !status.success() || size == 0 {
            return Err(RecordingError(format!(
                "FFmpeg exited {status}; temporary output at {}",
                self.temp_path.display()
            )));
        }
        fs::rename(&self.temp_path, &self.final_path).map_err(|error| {
            RecordingError(format!(
                "finalize {} to {} failed: {error}",
                self.temp_path.display(),
                self.final_path.display()
            ))
        })?;
        Ok(self.final_path)
    }
}

fn ffmpeg_path() -> Result<PathBuf, RecordingError> {
    let bundled = std::env::current_exe()
        .map_err(recording_error)?
        .parent()
        .expect("RapidCap executable has a parent")
        .join("ffmpeg.exe");
    if bundled.is_file() {
        return Ok(bundled);
    }
    // ponytail: PATH fallback is development-only; portable bundle supplies adjacent ffmpeg.exe.
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join("ffmpeg.exe"))
        .find(|candidate| candidate.is_file())
        .map(|candidate| resolve_scoop_shim(&candidate))
        .ok_or_else(|| RecordingError("ffmpeg.exe not found".into()))
}

fn resolve_scoop_shim(executable: &Path) -> PathBuf {
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
struct DdaSource {
    output_idx: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl DdaSource {
    fn resolve(region: &PhysicalRegion) -> Result<Self, RecordingError> {
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

fn ffmpeg_args(
    kind: CaptureKind,
    source: &DdaSource,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingError(String);

impl fmt::Display for RecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RecordingError {}

fn recording_error(error: impl fmt::Display) -> RecordingError {
    RecordingError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{CaptureKind, PhysicalRegion, Settings};

    use super::*;

    #[test]
    fn video_command_matches_sharex_encoder_and_audio() {
        let args = ffmpeg_args(
            CaptureKind::Video,
            &DdaSource {
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
            &DdaSource {
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
        assert!(joined.contains("-c:v h264_nvenc"), "video must survive: {joined}");
    }

    #[test]
    fn gif_command_streams_at_sharex_frame_rate() {
        let args = ffmpeg_args(
            CaptureKind::Gif,
            &DdaSource {
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
        assert_eq!(resolve_scoop_shim(&shim_exe), real_exe);
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
}
