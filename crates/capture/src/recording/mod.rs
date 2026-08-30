//! Video and GIF recording: one FFmpeg child process per session.
//!
//! Everything that survives a change of platform lives here - the pause state,
//! the temp-file dance, the finalize with its timeout - and the platform
//! modules supply only what genuinely differs: which source FFmpeg is pointed
//! at, how its process is suspended, and how it is spawned without a console.

use std::{
    fmt, fs,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use chrono::{Datelike, Local};

use crate::{AppPaths, CaptureKind, CaptureTarget, OutputNamer, Settings};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

pub struct RecordingSession {
    child: Child,
    temp_path: PathBuf,
    final_path: PathBuf,
    paused: bool,
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
        let now = Local::now();
        let directory = paths
            .capture_root
            .join(format!("{:04}-{:02}", now.year(), now.month()));
        fs::create_dir_all(&directory).map_err(recording_error)?;
        let extension = if kind == CaptureKind::Video {
            "mp4"
        } else {
            "gif"
        };
        let stem = OutputNamer::random().file_stem("Screen");
        let final_path = directory.join(format!("{stem}.{extension}"));
        let temp_path = paths.temp_dir.join(format!("{stem}.part.{extension}"));
        let source = platform::CaptureSource::resolve(region)?;
        let ffmpeg = ffmpeg_path()?;
        let mut command = Command::new(ffmpeg);
        command
            .args(platform::ffmpeg_args(kind, &source, settings, &temp_path))
            // `stop` quits FFmpeg by writing "q" to it, so stdin stays a pipe on
            // every platform; only the console hiding below is Windows-shaped.
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        platform::hide_console(&mut command);
        let child = command.spawn().map_err(recording_error)?;
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
        platform::suspend(&self.child)?;
        self.paused = true;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), RecordingError> {
        if !self.paused {
            return Ok(());
        }
        platform::resume(&self.child)?;
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
        .join(platform::FFMPEG_EXE);
    if bundled.is_file() {
        return Ok(bundled);
    }
    // ponytail: PATH fallback is development-only; portable bundle supplies adjacent ffmpeg.
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(platform::FFMPEG_EXE))
        .find(|candidate| candidate.is_file())
        .map(|candidate| platform::resolve_shim(&candidate))
        .ok_or_else(|| RecordingError(format!("{} not found", platform::FFMPEG_EXE)))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingError(pub(crate) String);

impl fmt::Display for RecordingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RecordingError {}

pub(crate) fn recording_error(error: impl fmt::Display) -> RecordingError {
    RecordingError(error.to_string())
}
