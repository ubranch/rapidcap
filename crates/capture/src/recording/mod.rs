//! Video and GIF recording: one FFmpeg child process per unpaused stretch.
//!
//! Everything that survives a change of platform lives here - the segmenting,
//! the temp-file dance, the finalize with its timeout - and the platform
//! modules supply only what genuinely differs: which source FFmpeg is pointed
//! at and how it is spawned without a console.

use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
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
/// a remux and no re-encode. Both container formats survive it: an MP4 of H.264
/// and the FFV1 MKV a GIF is captured as both concatenate frame-for-frame.
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
        // A GIF is captured as lossless FFV1 and made at stop: on FFmpeg 9 the
        // palette pass never finishes against a live source, so a GIF made in
        // the capture graph came out as its first frame or two.
        let extension = match kind {
            CaptureKind::Video => "mp4",
            _ => "mkv",
        };
        let (temp_stem, final_path) = recording_paths(kind, paths)?;
        let mut session = Self {
            child: None,
            kind,
            source: platform::CaptureSource::resolve(region)?,
            settings: settings.clone(),
            ffmpeg: ffmpeg_path()?,
            segments: Vec::new(),
            temp_stem,
            extension,
            final_path,
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
        let mut joined = self.join_segments()?;
        if self.kind == CaptureKind::Gif {
            let gif = encode_gif(
                &self.ffmpeg,
                &self.settings,
                &joined,
                &self.temp_stem.with_extension("gif"),
            )?;
            if !self.segments.contains(&joined) {
                let _ = fs::remove_file(&joined);
            }
            joined = gif;
        }
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

/// Where a recording started now is written and where it ends up:
/// `<temp>/<stem>.part` without an extension, and
/// `<root>/<yyyy-mm>/<stem>.mp4|gif`. Creates both directories.
///
/// Public for the macOS app, which records video through ScreenCaptureKit
/// rather than FFmpeg and still names and files it the same way.
pub fn recording_paths(
    kind: CaptureKind,
    paths: &AppPaths,
) -> Result<(PathBuf, PathBuf), RecordingError> {
    fs::create_dir_all(&paths.temp_dir).map_err(recording_error)?;
    let now = Local::now();
    let directory = paths
        .capture_root
        .join(format!("{:04}-{:02}", now.year(), now.month()));
    fs::create_dir_all(&directory).map_err(recording_error)?;
    let final_extension = if kind == CaptureKind::Video {
        "mp4"
    } else {
        "gif"
    };
    let stem = OutputNamer::at(now).file_stem("Screen");
    Ok((
        paths.temp_dir.join(format!("{stem}.part")),
        directory.join(format!("{stem}.{final_extension}")),
    ))
}

/// The GIF made from a finished capture file, with the palette the settings
/// name. On a file, unlike a live source, the palette pass finishes.
///
/// One palette for the whole clip (`full`, `diff`) is chosen in a pass of its
/// own, written to a PNG, and then applied in a second pass. Both stream. The
/// single-graph form held every decoded frame in memory until the palette was
/// known: 1.3 GB for ten seconds of 1080p, growing with the length. A palette
/// per frame (`single`) streams in one graph, so it keeps that form.
fn encode_gif(
    ffmpeg: &Path,
    settings: &Settings,
    source: &Path,
    gif: &Path,
) -> Result<PathBuf, RecordingError> {
    let mode = &settings.gif.palette_stats_mode;
    let palette = gif.with_extension("palette.png");
    let result = if mode == "single" {
        run_ffmpeg(
            ffmpeg,
            &[source],
            &format!("[0:v]null{}", gif_palette_chain(settings)),
            &[],
            gif,
        )
    } else {
        // One image: without `-update` the image muxer wants a numbered
        // sequence.
        run_ffmpeg(
            ffmpeg,
            &[source],
            &format!("[0:v]{}[v]", palettegen(mode)),
            &["-update", "1", "-frames:v", "1"],
            &palette,
        )
        .and_then(|()| apply_palette(ffmpeg, settings, source, &palette, gif))
    };
    let _ = fs::remove_file(&palette);
    result.map_err(|status| {
        RecordingError(format!(
            "making the GIF failed: FFmpeg exited {status}; capture kept at {}",
            source.display()
        ))
    })?;
    Ok(gif.to_path_buf())
}

/// How many slices the palette pass is cut into: one per whole second of
/// clip, up to one per core and never more than eight.
fn gif_slices(seconds: f64) -> usize {
    let cores = thread::available_parallelism().map_or(1, |cores| cores.get());
    (seconds as usize).clamp(1, cores.min(8))
}

/// The frames mapped onto `palette`, in time slices side by side.
///
/// Error-diffusion dither runs on one thread per FFmpeg and was most of a
/// GIF's finalise: 5.3 s of the 6.2 s for ten seconds of moving 1080p. Each
/// slice dithers its own stretch and the GIFs are joined byte for byte. With
/// one palette shared and no dither state carried between frames, the join
/// decodes to the same frames one run makes. A short clip, a slice that
/// fails, or a join that does not fit takes that one run instead.
fn apply_palette(
    ffmpeg: &Path,
    settings: &Settings,
    source: &Path,
    palette: &Path,
    gif: &Path,
) -> Result<(), String> {
    let seconds = media_duration(ffmpeg, source).map_or(0.0, |d| d.as_secs_f64());
    let slices = gif_slices(seconds);
    if slices > 1
        && apply_palette_sliced(ffmpeg, settings, source, palette, gif, seconds, slices).is_ok()
    {
        return Ok(());
    }
    run_ffmpeg(
        ffmpeg,
        &[source, palette],
        &format!("[0:v][1:v]{}[v]", paletteuse(settings)),
        &[],
        gif,
    )
}

fn apply_palette_sliced(
    ffmpeg: &Path,
    settings: &Settings,
    source: &Path,
    palette: &Path,
    gif: &Path,
    seconds: f64,
    slices: usize,
) -> Result<(), String> {
    let mapping = paletteuse(settings);
    // One string per cut, so a slice ends on exactly the time the next one
    // starts: `trim` keeps `start` and drops `end`, and no frame is in two.
    let cuts: Vec<String> = (1..slices)
        .map(|cut| format!("{:.6}", seconds * cut as f64 / slices as f64))
        .collect();
    let parts: Vec<PathBuf> = (0..slices)
        .map(|slice| gif.with_extension(format!("{slice}.part.gif")))
        .collect();
    let sliced = thread::scope(|scope| {
        let runs: Vec<_> = parts
            .iter()
            .enumerate()
            .map(|(slice, part)| {
                let start = slice.checked_sub(1).map_or("0", |cut| &cuts[cut]);
                let end = cuts.get(slice).map(|cut| format!(":end={cut}"));
                let graph = format!(
                    "[0:v]trim=start={start}{},setpts=PTS-STARTPTS[s];[s][1:v]{mapping}[v]",
                    end.unwrap_or_default()
                );
                scope.spawn(move || run_ffmpeg(ffmpeg, &[source, palette], &graph, &[], part))
            })
            .collect();
        runs.into_iter()
            .map(|run| {
                run.join()
                    .unwrap_or_else(|_| Err("a GIF slice panicked".into()))
            })
            .collect::<Result<Vec<()>, String>>()
    })
    .and_then(|_| join_gifs(&parts, gif));
    for part in &parts {
        let _ = fs::remove_file(part);
    }
    sliced
}

/// GIFs made with one palette, as one file: the first whole, then the frames
/// of each next one without its header, palette and loop block.
fn join_gifs(parts: &[PathBuf], gif: &Path) -> Result<(), String> {
    let mut joined = Vec::new();
    let mut screen: Option<Vec<u8>> = None;
    for part in parts {
        let data = fs::read(part).map_err(|error| error.to_string())?;
        let (this, frames) = gif_frames(&data).ok_or("a GIF slice is not a GIF")?;
        // An empty slice means the cuts missed the clip's timestamps; one run
        // is the safe answer, not a GIF with a stretch silently missing.
        if frames.is_empty() {
            return Err("a GIF slice has no frames".into());
        }
        match &screen {
            None => {
                joined.extend_from_slice(&data[..data.len() - 1]);
                screen = Some(this.to_vec());
            }
            Some(first) if first == this => joined.extend_from_slice(frames),
            Some(_) => return Err("GIF slices differ in size or palette".into()),
        }
    }
    joined.push(GIF_TRAILER);
    fs::write(gif, joined).map_err(|error| error.to_string())
}

const GIF_TRAILER: u8 = 0x3B;

/// A GIF's logical screen with its global palette, and its frames: from the
/// first frame's control block or image to just before the trailer.
fn gif_frames(data: &[u8]) -> Option<(&[u8], &[u8])> {
    if !data.starts_with(b"GIF8") || data.last() != Some(&GIF_TRAILER) {
        return None;
    }
    let flags = *data.get(10)?;
    let palette = if flags & 0x80 != 0 {
        3 << ((flags & 7) + 1)
    } else {
        0
    };
    let screen_end = 13 + palette;
    // Extensions before the first frame (the loop count) belong to the file.
    let mut at = screen_end;
    while *data.get(at)? == 0x21 && *data.get(at + 1)? != 0xF9 {
        at += 2;
        while *data.get(at)? != 0 {
            at += usize::from(data[at]) + 1;
        }
        at += 1;
    }
    Some((data.get(6..screen_end)?, data.get(at..data.len() - 1)?))
}

/// How long a media file is, from the line FFmpeg prints about its input.
pub fn media_duration(ffmpeg: &Path, media: &Path) -> Option<Duration> {
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(media)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    platform::hide_console(&mut command);
    let output = command.output().ok()?;
    parse_duration(&String::from_utf8_lossy(&output.stderr))
}

/// Pull `Duration: 00:00:12.34` out of what `ffmpeg` prints about its input.
///
/// It is the only place the length appears without shipping a second binary,
/// and the line has had this shape for the whole of ffmpeg 4 through 9. A file
/// whose duration is `N/A` - a stream, or a partial recording - parses to
/// `None` rather than to zero.
pub fn parse_duration(stderr: &str) -> Option<Duration> {
    let after = stderr.split("Duration: ").nth(1)?;
    let stamp = after.split(',').next()?.trim();
    let mut parts = stamp.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(Duration::from_secs_f64(
        (hours * 3600 + minutes * 60) as f64 + seconds,
    ))
}

/// One FFmpeg run: `inputs` through `graph`, whose `[v]` is written to
/// `output` with `extra` output options.
fn run_ffmpeg(
    ffmpeg: &Path,
    inputs: &[&Path],
    graph: &str,
    extra: &[&str],
    output: &Path,
) -> Result<(), String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error"]);
    for input in inputs {
        command.arg("-i").arg(input);
    }
    command
        .args(["-filter_complex", graph, "-map", "[v]"])
        .args(extra)
        .arg("-y")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    platform::hide_console(&mut command);
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(status.to_string())
    }
}

/// How long a segment gets to write its trailer before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Every microphone this machine will let a recording mix in.
///
/// Runs FFmpeg, so it belongs on a background thread rather than in the middle
/// of a frame. An empty list is a machine with no microphone; an error is a
/// machine whose devices could not be asked.
pub fn microphones() -> Result<Vec<String>, RecordingError> {
    platform::microphones()
}

/// The palette half of a GIF filtergraph, appended to whatever produced the
/// frames and ending in the `[v]` the caller maps.
///
/// GIF holds 256 colours. Without this FFmpeg's encoder picks them with a
/// fixed heuristic and no error diffusion, which is what banded every gradient
/// in a recorded window. `palettegen` chooses the 256 from the footage and
/// `paletteuse` maps the frames onto them with the dither the settings name.
///
/// `stats_mode=single` gives every frame its own palette, so `paletteuse` is
/// told to expect a new one each time. The other two modes emit one palette for
/// the whole stream, and passing `new=1` there would make it read the second
/// frame of video as a palette.
///
/// Used live only for `single`, which streams. The whole-clip modes buffer
/// every frame in this form, so [`encode_gif`] runs them in two passes.
fn gif_palette_chain(settings: &Settings) -> String {
    format!(
        ",split[pg][pu];[pg]{}[pal];[pu][pal]{}[v]",
        palettegen(&settings.gif.palette_stats_mode),
        paletteuse(settings)
    )
}

/// The palette chosen, all 256 colours of it for the picture.
///
/// `palettegen` keeps one entry back for transparency by default. A screen
/// capture has none, and with that entry in the palette FFmpeg's GIF encoder
/// wrote every frame whole: ten seconds of a still 1080p desktop came to
/// 11 MB. Without it unchanged frames shrink to almost nothing - 0.08 MB for
/// the same clip - and the extra colour lifted its PSNR from 34 to 40 dB.
fn palettegen(mode: &str) -> String {
    format!("palettegen=stats_mode={mode}:reserve_transparent=0")
}

/// The palette applied, with the settings' dither. `diff_mode=rectangle`
/// re-dithers only the part of each frame that changed: a screen recording
/// is mostly still, and this made the pass about three times faster on one.
fn paletteuse(settings: &Settings) -> String {
    let dither = &settings.gif.dither;
    let new = if settings.gif.palette_stats_mode == "single" {
        ":new=1"
    } else {
        ""
    };
    format!("paletteuse=dither={dither}:diff_mode=rectangle{new}")
}

/// Where the bundled `ffmpeg` is, or the one on `PATH` in a dev checkout.
///
/// Public because the gallery needs it too: an MP4's first frame and its length
/// both come out of one `ffmpeg` call, and there is no second copy of this
/// search worth keeping in sync with this one.
pub fn ffmpeg_path() -> Result<PathBuf, RecordingError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_pass_follows_the_stats_mode() {
        let mut settings = Settings::default();
        // One palette for the whole clip, so `new` stays off: with it,
        // paletteuse reads the next frame of video as a palette.
        assert_eq!(
            paletteuse(&settings),
            "paletteuse=dither=sierra2_4a:diff_mode=rectangle"
        );
        settings.gif.palette_stats_mode = "single".into();
        assert_eq!(
            paletteuse(&settings),
            "paletteuse=dither=sierra2_4a:diff_mode=rectangle:new=1"
        );
    }

    /// The regression: a GIF that came out as one still frame. Needs an FFmpeg
    /// on PATH, as the recorder does, and passes without checking when there
    /// is none.
    #[test]
    fn a_gif_made_at_stop_keeps_every_frame() {
        let Ok(ffmpeg) = ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("rapidcap-gif-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("capture.mkv");
        let made = Command::new(&ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=160x120:rate=10",
                "-t",
                "2",
            ])
            .args(["-c:v", "ffv1", "-y"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());

        // Every palette mode: `full` and `diff` run in two passes, `single`
        // in one graph.
        for mode in ["full", "diff", "single"] {
            let mut settings = Settings::default();
            settings.gif.palette_stats_mode = mode.into();
            let gif = encode_gif(&ffmpeg, &settings, &source, &dir.join("out.gif")).unwrap();
            // Count frames by decoding them: one line per frame.
            let frames = Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&gif)
                .args(["-f", "framemd5", "-"])
                .output()
                .unwrap();
            let count = String::from_utf8_lossy(&frames.stdout)
                .lines()
                .filter(|line| !line.starts_with('#'))
                .count();
            assert_eq!(count, 20, "{mode}: 2 seconds at 10 fps");
            // The palette image is a temporary, not left beside the GIF.
            assert!(!dir.join("out.palette.png").exists(), "{mode}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// The sliced palette pass makes the same frames as one run, and leaves
    /// none of its slices behind. Needs an FFmpeg, like the test above.
    #[test]
    fn a_gif_dithered_in_slices_is_the_gif_one_run_makes() {
        let Ok(ffmpeg) = ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("rapidcap-slices-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("capture.mkv");
        let palette = dir.join("palette.png");
        let ff = |args: &[&str]| {
            let status = Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error"])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "{args:?}");
        };
        let source_arg = source.to_str().unwrap();
        let palette_arg = palette.to_str().unwrap();
        ff(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=10",
            "-t",
            "3",
        ]
        .into_iter()
        .chain(["-c:v", "ffv1", "-y", source_arg])
        .collect::<Vec<_>>());
        ff(&[
            "-i",
            source_arg,
            "-vf",
            "palettegen",
            "-update",
            "1",
            "-frames:v",
            "1",
            "-y",
            palette_arg,
        ]);
        let frames = |gif: &Path| -> Vec<String> {
            let output = Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(gif)
                .args(["-pix_fmt", "rgb24", "-f", "framemd5", "-"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.starts_with('#'))
                .map(|line| line.rsplit(',').next().unwrap().trim().to_owned())
                .collect()
        };
        let settings = Settings::default();
        let one = dir.join("one.gif");
        let sliced = dir.join("sliced.gif");
        run_ffmpeg(
            &ffmpeg,
            &[&source, &palette],
            &format!("[0:v][1:v]{}[v]", paletteuse(&settings)),
            &[],
            &one,
        )
        .unwrap();
        // Called directly: `apply_palette` would fall back to one run and pass
        // this test whatever the slices did.
        apply_palette_sliced(&ffmpeg, &settings, &source, &palette, &sliced, 3.0, 3).unwrap();
        let whole = frames(&one);
        assert_eq!(whole.len(), 30, "3 seconds at 10 fps");
        assert_eq!(frames(&sliced), whole);
        assert!(!dir.join("sliced.0.part.gif").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// The regression: a still screen whose GIF repeated the whole frame
    /// every frame. Desktop Duplication hands back a few pixels with alpha
    /// below opaque (shadows, the cursor), as this source has in one column.
    /// Thirty unchanged frames must cost little more than one.
    #[test]
    fn a_still_screen_does_not_repeat_its_frame() {
        let Ok(ffmpeg) = ffmpeg_path() else { return };
        let dir = std::env::temp_dir().join(format!("rapidcap-still-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("capture.mkv");
        let made = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .args(["testsrc2=size=320x240:rate=10", "-vf"])
            .arg("trim=end_frame=1,loop=-1:1,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(eq(X,5),0,255)'")
            .args(["-t", "3", "-pix_fmt", "bgra", "-c:v", "ffv1", "-y"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let size = |gif: &Path, extra: &[&str]| {
            run_ffmpeg(
                &ffmpeg,
                &[&source],
                &format!("[0:v]null{}", gif_palette_chain(&Settings::default())),
                extra,
                gif,
            )
            .unwrap();
            fs::metadata(gif).unwrap().len()
        };
        let one = size(&dir.join("one.gif"), &["-frames:v", "1"]);
        let all = size(&dir.join("all.gif"), &[]);
        assert!(
            all < one * 3,
            "30 still frames: {all} bytes, one frame: {one}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_length_comes_off_the_line_ffmpeg_prints_about_its_input() {
        let stderr = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'Screen.mp4':
  Metadata:
    encoder         : Lavf61.7.100
  Duration: 00:00:42.53, start: 0.000000, bitrate: 4821 kb/s
  Stream #0:0[0x1](und): Video: h264 (High)
";
        assert_eq!(parse_duration(stderr), Some(Duration::from_secs_f64(42.53)));
        assert_eq!(
            parse_duration("  Duration: 01:02:03.00, start: 0.000000, bitrate: 1 kb/s"),
            Some(Duration::from_secs(3723))
        );
        // A stream, or a recording whose trailer was never written.
        assert_eq!(parse_duration("  Duration: N/A, bitrate: N/A"), None);
        assert_eq!(parse_duration("ffmpeg: no such file"), None);
    }
}
