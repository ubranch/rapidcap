use std::{
    fmt, fs,
    io::Write,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::{AppPaths, CaptureKind, CaptureTarget, OutputNamer, PhysicalRegion, Settings};

pub struct RecordingSession {
    child: Child,
    temp_path: PathBuf,
    final_path: PathBuf,
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
        let ffmpeg = ffmpeg_path()?;
        let child = Command::new(ffmpeg)
            .args(ffmpeg_args(kind, region, settings, &temp_path))
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
        })
    }

    pub fn stop(mut self) -> Result<PathBuf, RecordingError> {
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
    Ok(PathBuf::from("ffmpeg.exe"))
}

fn ffmpeg_args(
    kind: CaptureKind,
    region: &PhysicalRegion,
    settings: &Settings,
    output: &Path,
) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-f".into(),
        "gdigrab".into(),
        "-thread_queue_size".into(),
        "1024".into(),
        "-framerate".into(),
        settings.video.fps.to_string(),
        "-offset_x".into(),
        region.x.to_string(),
        "-offset_y".into(),
        region.y.to_string(),
        "-video_size".into(),
        format!("{}x{}", region.width, region.height),
        "-i".into(),
        "desktop".into(),
    ];
    if kind == CaptureKind::Video {
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-thread_queue_size".into(),
            "1024".into(),
            "-audio_buffer_size".into(),
            "80".into(),
            "-i".into(),
            "audio=virtual-audio-capturer".into(),
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
            "-vf".into(),
            "pad=ceil(iw/2)*2:ceil(ih/2)*2".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-movflags".into(),
            "+faststart".into(),
            "-c:a".into(),
            "aac".into(),
            "-ac".into(),
            settings.audio.channels.to_string(),
            "-b:a".into(),
            format!("{}k", settings.audio.bitrate / 1000),
        ]);
    } else {
        args.extend([
            "-filter_complex".into(),
            format!(
                "split[a][b];[a]fps={},palettegen=stats_mode={}[p];[b]fps={}[x];[x][p]paletteuse=dither={}",
                settings.gif.fps,
                settings.gif.palette_stats_mode,
                settings.gif.fps,
                settings.gif.dither
            ),
        ]);
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
            &PhysicalRegion {
                x: 12,
                y: 34,
                width: 800,
                height: 601,
            },
            &Settings::default(),
            Path::new("out.part.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-framerate 60"));
        assert!(joined.contains("audio=virtual-audio-capturer"));
        assert!(joined.contains("-c:v h264_nvenc -r 60 -preset p7 -tune hq -b:v 3000k"));
        assert!(joined.contains("-c:a aac -ac 2 -b:a 128k"));
    }

    #[test]
    fn gif_command_uses_sharex_palette_settings() {
        let args = ffmpeg_args(
            CaptureKind::Gif,
            &PhysicalRegion {
                x: 0,
                y: 0,
                width: 320,
                height: 200,
            },
            &Settings::default(),
            Path::new("out.part.gif"),
        );
        assert!(args.join(" ").contains(
            "fps=15,palettegen=stats_mode=full[p];[b]fps=15[x];[x][p]paletteuse=dither=sierra2_4a"
        ));
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
