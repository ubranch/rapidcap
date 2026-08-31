//! Scaled-down copies of the captures, for the gallery grid.
//!
//! A tile is 118px wide on a 1x display, and a 4K screenshot is 33 MB of pixels
//! decoded. Sixty of those is the only real cost the gallery has, so nothing
//! full-size ever reaches the renderer: each capture gets one [`WIDTH`] by
//! [`HEIGHT`] PNG on disk, and the grid draws that.
//!
//! There is no index file. The cache directory *is* the index: every entry is
//! named for the capture it came from and the length of that capture, so one
//! `read_dir` answers both "have I seen this file" and "how long is it". That
//! means no schema, no migration, and a cache that heals by being deleted.
//!
//! Two ways in, split on what each kind costs. A PNG or a JPEG is decoded in
//! process, because the `image` crate is already here and a process spawn per
//! screenshot is the slower half of the job. An MP4 or a GIF goes to `ffmpeg`,
//! because their duration has to be read anyway and one invocation returns the
//! first frame and the length together.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use rapidcap_capture::ffmpeg_path;

use crate::gallery::Kind;

/// Twice the width of a tile, so the grid stays sharp on a 2x display.
pub const WIDTH: u32 = 236;
/// Twice the 72px thumbnail strip. Every entry is cropped to exactly this, so
/// the grid never letterboxes and the renderer never scales one axis alone.
pub const HEIGHT: u32 = 144;

/// What the grid needs about one capture beyond its name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumb {
    pub image: PathBuf,
    /// `None` for a still, and for a video whose length `ffmpeg` would not
    /// report - a partial file, or one still being written.
    pub duration: Option<Duration>,
}

/// The thumbnail directory, read once and then written through.
pub struct Cache {
    dir: PathBuf,
    entries: HashMap<u64, Thumb>,
}

impl Cache {
    /// One `read_dir`, and every name parsed back into what it stands for.
    ///
    /// A directory that cannot be created or read is not an error worth
    /// reporting: the gallery draws its placeholder tone and the next open
    /// tries again. The one thing it must not do is fail to draw.
    pub fn open(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let entries = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let (key, duration) = parse_name(path.file_name()?.to_str()?)?;
                Some((
                    key,
                    Thumb {
                        image: path,
                        duration,
                    },
                ))
            })
            .collect();
        Self {
            dir: dir.to_path_buf(),
            entries,
        }
    }

    /// The thumbnail for one capture, generating it if this is the first sight
    /// of that file.
    ///
    /// `modified` is part of the key rather than a field to compare, so a file
    /// overwritten in place gets a new entry instead of a stale one - the old
    /// entry is simply never asked for again, and [`Self::sweep`] collects it.
    pub fn thumb(&mut self, source: &Path, modified: SystemTime, kind: Kind) -> Option<Thumb> {
        let key = key(source, modified);
        if let Some(thumb) = self.entries.get(&key) {
            return Some(thumb.clone());
        }
        let thumb = generate(source, kind, &self.dir, key)?;
        self.entries.insert(key, thumb.clone());
        Some(thumb)
    }

    /// Drop every entry the folder no longer has a capture for.
    ///
    /// Called once after a listing, with the keys that listing produced. A
    /// capture deleted from the gallery leaves its thumbnail behind otherwise,
    /// and a folder that turns over daily would grow this directory forever.
    pub fn sweep(&mut self, live: &[u64]) {
        self.entries.retain(|key, thumb| {
            if live.contains(key) {
                return true;
            }
            let _ = std::fs::remove_file(&thumb.image);
            false
        });
    }
}

/// Identity of a capture, as far as the cache is concerned.
///
/// FNV-1a rather than [`std::hash::DefaultHasher`], whose output is explicitly
/// not stable between Rust releases - which would silently orphan the whole
/// directory on every toolchain upgrade.
pub fn key(source: &Path, modified: SystemTime) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_01b3);
        }
    };
    eat(source.to_string_lossy().as_bytes());
    let nanos = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or_default();
    eat(&nanos.to_le_bytes());
    hash
}

/// `{key}-{milliseconds}.png`, or `{key}-x.png` for a capture with no length.
fn name(key: u64, duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("{key:016x}-{}.png", duration.as_millis()),
        None => format!("{key:016x}-x.png"),
    }
}

fn parse_name(name: &str) -> Option<(u64, Option<Duration>)> {
    let stem = name.strip_suffix(".png")?;
    let (key, duration) = stem.split_once('-')?;
    let key = u64::from_str_radix(key, 16).ok()?;
    let duration = match duration {
        "x" => None,
        millis => Some(Duration::from_millis(millis.parse().ok()?)),
    };
    Some((key, duration))
}

/// Write the entry, and only then let anyone see it.
///
/// Through a temporary and a rename: a thumbnail half-written when the process
/// is killed would otherwise sit in the cache as a valid name over a truncated
/// PNG, and nothing would ever regenerate it.
fn publish(temp: &Path, dir: &Path, key: u64, duration: Option<Duration>) -> Option<Thumb> {
    let image = dir.join(name(key, duration));
    if std::fs::rename(temp, &image).is_err() {
        let _ = std::fs::remove_file(temp);
        return None;
    }
    Some(Thumb { image, duration })
}

fn generate(source: &Path, kind: Kind, dir: &Path, key: u64) -> Option<Thumb> {
    let temp = dir.join(format!("{key:016x}.part"));
    let result = match kind {
        Kind::Png | Kind::Jpg => still(source, &temp).map(|()| None),
        Kind::Mp4 | Kind::Gif => frame_and_length(source, &temp).map(Some),
    };
    match result {
        Ok(duration) => publish(&temp, dir, key, duration.flatten()),
        Err(error) => {
            tracing::warn!(%error, source = %source.display(), "thumbnail");
            let _ = std::fs::remove_file(&temp);
            None
        }
    }
}

/// A screenshot, decoded and cropped in process.
///
/// `resize_to_fill` rather than a fit: the tile shows a strip of a 16:9 screen,
/// and letterboxing it inside a 236x144 box would spend two thirds of the tile
/// on the background colour. Cropping to the middle is what the design draws.
fn still(source: &Path, temp: &Path) -> anyhow::Result<()> {
    let image = image::open(source)?;
    image
        .resize_to_fill(WIDTH, HEIGHT, image::imageops::FilterType::Triangle)
        .save_with_format(temp, image::ImageFormat::Png)?;
    Ok(())
}

/// A video or an animation: one frame out, and the length off the same run.
///
/// `-frames:v 1` with no seek, because a RapidCap recording starts on the
/// content it was pointed at - there is no title card to skip, and a seek into
/// a file still being finalised is the reliable way to get nothing back.
fn frame_and_length(source: &Path, temp: &Path) -> anyhow::Result<Option<Duration>> {
    let output = Command::new(ffmpeg_path()?)
        .args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(source)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!(
                "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=increase,crop={WIDTH}:{HEIGHT}"
            ),
            "-f",
            "image2",
        ])
        .arg(temp)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("ffmpeg exited with {}", output.status);
    }
    Ok(parse_duration(&String::from_utf8_lossy(&output.stderr)))
}

/// Pull `Duration: 00:00:12.34` out of what `ffmpeg` prints about its input.
///
/// It is the only place the length appears without shipping a second binary,
/// and the line has had this shape for the whole of ffmpeg 4 through 8. A file
/// whose duration is `N/A` - a stream, or a partial recording - parses to
/// `None` rather than to zero, so the badge is left off instead of claiming the
/// clip is empty.
fn parse_duration(stderr: &str) -> Option<Duration> {
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

/// The badge in the thumbnail's corner: `00:42`, or `1:02:03` past an hour.
///
/// Rounded up, not down, so a clip of 900 milliseconds reads `00:01` rather
/// than `00:00` - a recording that exists is never zero seconds long.
pub fn duration_label(duration: Duration) -> String {
    let total = duration.as_millis().div_ceil(1000) as u64;
    let (hours, minutes, seconds) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_round_trips_through_the_directory() {
        for duration in [
            None,
            Some(Duration::from_millis(42_500)),
            Some(Duration::ZERO),
        ] {
            let encoded = name(0xdead_beef_0000_0001, duration);
            assert_eq!(
                parse_name(&encoded),
                Some((0xdead_beef_0000_0001, duration)),
                "{encoded}"
            );
        }
    }

    #[test]
    fn a_stranger_in_the_cache_directory_is_ignored() {
        // Anything the user or the shell dropped in here. It must not be read
        // back as a thumbnail for some capture it has nothing to do with.
        for stranger in [
            "thumbs.db",
            "notes.txt",
            "0011.part",
            "nothex-500.png",
            "0000000000000001-.png",
            "0000000000000001-later.png",
        ] {
            assert_eq!(parse_name(stranger), None, "{stranger}");
        }
    }

    #[test]
    fn a_key_follows_the_file_and_the_time_it_was_written() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let one = key(Path::new("C:/shots/a.png"), now);
        assert_eq!(one, key(Path::new("C:/shots/a.png"), now));
        assert_ne!(one, key(Path::new("C:/shots/b.png"), now));
        assert_ne!(
            one,
            key(Path::new("C:/shots/a.png"), now + Duration::from_secs(1)),
            "a file overwritten in place is a different thumbnail"
        );
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

    #[test]
    fn the_badge_never_says_a_recording_lasted_no_time() {
        assert_eq!(duration_label(Duration::from_millis(900)), "00:01");
        assert_eq!(duration_label(Duration::from_millis(1)), "00:01");
        assert_eq!(duration_label(Duration::ZERO), "00:00");
        assert_eq!(duration_label(Duration::from_secs(42)), "00:42");
        assert_eq!(duration_label(Duration::from_secs(8)), "00:08");
        assert_eq!(duration_label(Duration::from_secs(600)), "10:00");
        assert_eq!(duration_label(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn a_screenshot_is_cropped_to_the_tile_rather_than_letterboxed_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wide.png");
        // 16:9, which is wider than the tile's 236:144 - so the crop takes the
        // sides, and the result is exactly the tile either way.
        image::RgbImage::new(1920, 1080)
            .save_with_format(&source, image::ImageFormat::Png)
            .unwrap();

        let mut cache = Cache::open(&dir.path().join("cache"));
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let thumb = cache.thumb(&source, modified, Kind::Png).unwrap();

        assert_eq!(thumb.duration, None, "a screenshot has no length");
        let written = image::open(&thumb.image).unwrap();
        assert_eq!(
            (written.width(), written.height()),
            (WIDTH, HEIGHT),
            "every entry is exactly one tile"
        );

        // Second time round it comes off the directory rather than the decoder,
        // and a fresh Cache over the same directory finds it too.
        assert_eq!(cache.thumb(&source, modified, Kind::Png), Some(thumb));
        let mut reopened = Cache::open(&dir.path().join("cache"));
        assert!(reopened.thumb(&source, modified, Kind::Png).is_some());
    }

    #[test]
    fn a_capture_that_is_gone_takes_its_thumbnail_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let mut sources = Vec::new();
        for name in ["a.png", "b.png"] {
            let path = dir.path().join(name);
            image::RgbImage::new(40, 30)
                .save_with_format(&path, image::ImageFormat::Png)
                .unwrap();
            sources.push(path);
        }

        let mut cache = Cache::open(&cache_dir);
        let keys: Vec<u64> = sources
            .iter()
            .map(|source| {
                let modified = std::fs::metadata(source).unwrap().modified().unwrap();
                cache.thumb(source, modified, Kind::Png).unwrap();
                key(source, modified)
            })
            .collect();
        assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 2);

        cache.sweep(&keys[..1]);
        assert_eq!(
            std::fs::read_dir(&cache_dir).unwrap().count(),
            1,
            "the entry for the deleted capture is gone from the disk too"
        );
    }

    #[test]
    fn an_unreadable_capture_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        let source = dir.path().join("truncated.png");
        std::fs::write(&source, b"not a png").unwrap();

        let mut cache = Cache::open(&cache_dir);
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        assert_eq!(cache.thumb(&source, modified, Kind::Png), None);
        // No `.part` left over: the grid would keep the tile blank forever and
        // the file would never be retried.
        assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 0);
    }
}
