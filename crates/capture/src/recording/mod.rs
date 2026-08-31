//! Video and GIF recording: one FFmpeg child process per unpaused stretch.
//!
//! Everything that survives a change of platform lives here - the segmenting,
//! the temp-file dance, the finalize with its timeout - and the platform
//! modules supply only what genuinely differs: which source FFmpeg is pointed
//! at and how it is spawned without a console.

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

/// A recording in progress, made of one FFmpeg run per unpaused stretch.
///
/// Pause ends the run and resume starts another, rather than suspending the
/// process where it stands. Suspending looked like the smaller change and was
/// wrong: `ddagrab` generates frames against the wall clock, so on resume it
/// emitted a duplicate for every frame slot the pause had covered. A four
/// second pause came back as four seconds of frozen picture in a file that ran
/// the full wall time - measurably, 10.2 seconds of output for 10.4 seconds of
/// which 4 were paused. Nothing downstream of the source can take those frames
/// out again, because they are indistinguishable from a still desktop.
///
/// The segments are concatenated at stop with `-c copy`, so the extra runs cost
/// a remux and no re-encode. Both container formats survive it: an MP4 of NVENC
/// H.264 and a GIF both concatenate frame-for-frame.
pub struct RecordingSession {
    /// The run writing the current segment, or `None` between a pause and the
    /// resume that follows it.
    child: Option<Child>,
    /// Enough to start the next segment: the arguments differ only in where the
    /// output goes.
    kind: CaptureKind,
    source: platform::CaptureSource,
    settings: Settings,
    ffmpeg: PathBuf,
    /// Every segment written so far, including the one being written now, in
    /// the order they have to be joined.
    segments: Vec<PathBuf>,
    /// `<temp>/<stem>.part`, without the segment number or the extension.
    temp_stem: PathBuf,
    extension: &'static str,
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
        let stem = OutputNamer::at(now).file_stem("Screen");
        let mut session = Self {
            child: None,
            kind,
            source: platform::CaptureSource::resolve(region)?,
            settings: settings.clone(),
            ffmpeg: ffmpeg_path()?,
            segments: Vec::new(),
            temp_stem: paths.temp_dir.join(format!("{stem}.part")),
            extension,
            final_path: directory.join(format!("{stem}.{extension}")),
        };
        session.spawn_segment()?;
        Ok(session)
    }

    /// Start writing the next segment.
    fn spawn_segment(&mut self) -> Result<(), RecordingError> {
        let path =
            self.temp_stem
                .with_extension(format!("{}.{}", self.segments.len(), self.extension));
        let mut command = Command::new(&self.ffmpeg);
        command
            .args(platform::ffmpeg_args(
                self.kind,
                &self.source,
                &self.settings,
                &path,
            ))
            // FFmpeg is asked to finish by writing "q" to it, so stdin stays a
            // pipe on every platform; only the console hiding below is
            // Windows-shaped.
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        platform::hide_console(&mut command);
        self.child = Some(command.spawn().map_err(recording_error)?);
        self.segments.push(path);
        Ok(())
    }

    /// Ask the current run to finish and wait for it to write its trailer.
    ///
    /// A killed FFmpeg leaves a file with no index - `moov atom not found` for
    /// an MP4 - so the segment has to be quit rather than killed, and the wait
    /// has to be long enough for the muxer to flush.
    fn close_segment(&mut self) -> Result<(), RecordingError> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if child.try_wait().map_err(recording_error)?.is_none() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(
                    b"q
",
                );
            }
            platform::request_stop(&child);
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        let path = self.segments.last().cloned().unwrap_or_default();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(recording_error)? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RecordingError(format!(
                    "FFmpeg stop timed out; temporary output preserved at {}",
                    path.display()
                )));
            }
            thread::sleep(Duration::from_millis(25));
        };
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        if !status.success() || size == 0 {
            return Err(RecordingError(format!(
                "FFmpeg exited {status}; temporary output at {}",
                path.display()
            )));
        }
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), RecordingError> {
        if self.child.is_none() {
            return Ok(());
        }
        self.close_segment()
    }

    pub fn resume(&mut self) -> Result<(), RecordingError> {
        if self.child.is_some() {
            return Ok(());
        }
        self.spawn_segment()
    }

    /// Throw the recording away. Nothing here is worth failing over - the
    /// caller has already decided the take is unwanted - so a segment that
    /// refuses to die is killed and one that will not delete is left behind.
    pub fn cancel(mut self) -> Result<(), RecordingError> {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().map_err(recording_error)?.is_none() {
                child.kill().map_err(recording_error)?;
            }
            child.wait().map_err(recording_error)?;
        }
        for segment in &self.segments {
            match fs::remove_file(segment) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(recording_error(error)),
            }
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<PathBuf, RecordingError> {
        self.close_segment()?;
        let joined = self.join_segments()?;
        fs::rename(&joined, &self.final_path).map_err(|error| {
            RecordingError(format!(
                "finalize {} to {} failed: {error}",
                joined.display(),
                self.final_path.display()
            ))
        })?;
        // The segments have been copied into the joined file by this point. A
        // leftover in the temp directory is not worth failing a finished
        // recording over.
        for segment in &self.segments {
            let _ = fs::remove_file(segment);
        }
        Ok(self.final_path)
    }

    /// The one file holding the whole recording, ready to be moved into place.
    ///
    /// An unpaused recording is a single segment and is already that file. More
    /// than one is remuxed through the concat demuxer, which copies the streams
    /// rather than re-encoding them, so joining costs a pass over the bytes and
    /// no quality.
    fn join_segments(&self) -> Result<PathBuf, RecordingError> {
        let [only] = &self.segments[..] else {
            let list = self.temp_stem.with_extension("segments.txt");
            let mut body = String::new();
            for segment in &self.segments {
                // The concat demuxer's own quoting: single-quoted, with any `'`
                // in the path written as `'\''`. Temp paths run through the
                // user's profile directory, and an apostrophe in a display name
                // is not exotic.
                let path = segment.display().to_string().replace('\'', "'\\''");
                body.push_str(&format!("file '{path}'\n"));
            }
            fs::write(&list, body).map_err(recording_error)?;
            let joined = self
                .temp_stem
                .with_extension(format!("joined.{}", self.extension));
            let mut command = Command::new(&self.ffmpeg);
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "concat",
                    "-safe",
                    "0",
                    "-i",
                ])
                .arg(&list)
                .args(["-c", "copy", "-y"])
                .arg(&joined)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            platform::hide_console(&mut command);
            let status = command.status().map_err(recording_error)?;
            let _ = fs::remove_file(&list);
            if !status.success() {
                return Err(RecordingError(format!(
                    "joining {} recording segments failed: FFmpeg exited {status}",
                    self.segments.len()
                )));
            }
            return Ok(joined);
        };
        Ok(only.clone())
    }
}

/// How long a segment gets to write its trailer before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

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
        .chain(platform::EXTRA_SEARCH_DIRS.iter().map(PathBuf::from))
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
