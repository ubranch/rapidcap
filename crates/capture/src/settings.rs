use std::{
    fmt,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows::{
    Win32::{
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
            SHGetKnownFolderPath,
        },
    },
    core::{GUID, PCWSTR},
};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub screenshot: ScreenshotSettings,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub gif: GifSettings,
    pub countdown_seconds: u8,
    pub hotkeys: HotkeySettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            screenshot: ScreenshotSettings {
                png_to_jpeg_threshold_bytes: 2_097_152,
                jpeg_quality: 90,
            },
            video: VideoSettings {
                fps: 30,
                bitrate: 3_000_000,
                preset: "p7".into(),
                tune: "hq".into(),
            },
            audio: AudioSettings {
                enabled: true,
                bitrate: 128_000,
                channels: 2,
            },
            gif: GifSettings {
                fps: 15,
                palette_stats_mode: "full".into(),
                dither: "sierra2_4a".into(),
            },
            countdown_seconds: 5,
            hotkeys: HotkeySettings {
                region: "Alt+Q".into(),
                window: "Alt+Print Screen".into(),
                video: ["Alt+E".into(), "Shift+Print Screen".into()],
                gif: "Ctrl+Shift+Print Screen".into(),
            },
        }
    }
}

impl Settings {
    fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::Invalid(format!(
                "unsupported settings schema {}",
                self.schema_version
            )));
        }
        if !(1..=100).contains(&self.screenshot.jpeg_quality)
            || self.video.fps == 0
            || self.gif.fps == 0
        {
            return Err(SettingsError::Invalid(
                "settings contain zero or out-of-range values".into(),
            ));
        }
        // FFmpeg is handed `-b:v {bitrate / 1000}k`, so anything under 1000
        // truncates to `0k` and the encoder picks its own rate - the setting
        // reads as if it applied and did not. Zero channels produces `-ac 0`,
        // and an empty preset produces `-preset` followed by the next flag,
        // which FFmpeg then reads as the preset name. All three fail at spawn,
        // which is the moment the take is lost, so they fail at load instead.
        if self.video.bitrate < 1000 || self.audio.bitrate < 1000 {
            return Err(SettingsError::Invalid(
                "bitrates are given in bits per second and must be at least 1000".into(),
            ));
        }
        if self.audio.channels == 0 {
            return Err(SettingsError::Invalid(
                "a recording needs at least one audio channel".into(),
            ));
        }
        if self.video.preset.trim().is_empty() || self.video.tune.trim().is_empty() {
            return Err(SettingsError::Invalid(
                "encoder preset and tune must be named".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    file: PathBuf,
}

impl SettingsStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    pub fn load(&self) -> Result<Settings, SettingsError> {
        if !self.file.exists() {
            let settings = Settings::default();
            self.save(&settings)?;
            return Ok(settings);
        }
        let raw = fs::read(&self.file).map_err(SettingsError::io)?;
        let settings: Settings = serde_json::from_slice(&raw)
            .map_err(|error| SettingsError::Invalid(error.to_string()))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        settings.validate()?;
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|error| SettingsError::Invalid(error.to_string()))?;
        let temp = self.file.with_extension("json.part");
        write_atomic(&temp, &self.file, &bytes).map_err(SettingsError::io)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsError {
    Io(String),
    Invalid(String),
}

impl SettingsError {
    fn io(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "settings I/O failed: {message}"),
            Self::Invalid(message) => write!(formatter, "settings invalid: {message}"),
        }
    }
}

impl std::error::Error for SettingsError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotSettings {
    pub png_to_jpeg_threshold_bytes: usize,
    pub jpeg_quality: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoSettings {
    pub fps: u32,
    pub bitrate: u32,
    pub preset: String,
    pub tune: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSettings {
    /// Whether a video recording carries a soundtrack.
    ///
    /// Added after schema version 1 shipped, so it has to tolerate its own
    /// absence: a settings file written before this field existed is still a
    /// valid file, and back then the recorder always captured audio. The
    /// default therefore reproduces the old behaviour rather than inventing a
    /// new one.
    #[serde(default = "audio_enabled_default")]
    pub enabled: bool,
    pub bitrate: u32,
    pub channels: u16,
}

fn audio_enabled_default() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GifSettings {
    pub fps: u32,
    pub palette_stats_mode: String,
    pub dither: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeySettings {
    pub region: String,
    pub window: String,
    pub video: [String; 2],
    pub gif: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    pub capture_root: PathBuf,
    pub settings_file: PathBuf,
    pub log_dir: PathBuf,
    pub temp_dir: PathBuf,
}

impl AppPaths {
    #[cfg(windows)]
    pub fn discover() -> Result<Self, PathDiscoveryError> {
        Ok(Self::from_roots(
            known_folder(&FOLDERID_Documents)?,
            known_folder(&FOLDERID_RoamingAppData)?,
            known_folder(&FOLDERID_LocalAppData)?,
        ))
    }

    /// macOS has no registry of relocatable known folders - all three of these
    /// are fixed by the filesystem layout - so `$HOME` and three literals cover
    /// what `SHGetKnownFolderPath` is needed for on Windows.
    ///
    /// `Caches` stands in for LocalAppData, which puts the logs under
    /// `~/Library/Caches/RapidCap/Logs` rather than the `~/Library/Logs` a mac
    /// user would look in first. Worth revisiting once `from_roots` stops
    /// naming its arguments after Windows folders.
    #[cfg(target_os = "macos")]
    pub fn discover() -> Result<Self, PathDiscoveryError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| PathDiscoveryError("HOME is not set".into()))?;
        Ok(Self::from_roots(
            home.join("Documents"),
            home.join("Library/Application Support"),
            home.join("Library/Caches"),
        ))
    }

    pub fn from_roots(
        documents: impl AsRef<Path>,
        roaming_app_data: impl AsRef<Path>,
        local_app_data: impl AsRef<Path>,
    ) -> Self {
        Self {
            capture_root: documents.as_ref().join("RapidCap").join("Screenshots"),
            settings_file: roaming_app_data
                .as_ref()
                .join("RapidCap")
                .join("settings.json"),
            log_dir: local_app_data.as_ref().join("RapidCap").join("Logs"),
            temp_dir: local_app_data.as_ref().join("RapidCap").join("Temp"),
        }
    }
}

pub(crate) fn write_atomic(temp: &Path, final_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = final_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "output path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let result = (|| {
        let mut file = File::create(temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        move_replace(temp, final_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(windows)]
fn move_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both vectors are live NUL-terminated UTF-16 paths for the call duration.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))
    }
}

#[cfg(target_os = "macos")]
fn move_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    // `rename` is atomic and replaces on APFS, but on its own it only orders the
    // metadata write - the new directory entry can still be lost to a power cut.
    // Syncing the parent afterwards is what buys back the durability that
    // MOVEFILE_WRITE_THROUGH gives the Windows path.
    fs::rename(source, destination)?;
    match destination.parent() {
        Some(parent) => File::open(parent)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn known_folder(id: &GUID) -> Result<PathBuf, PathDiscoveryError> {
    // SAFETY: SHGetKnownFolderPath owns returned NUL-terminated allocation. We copy it,
    // then release exactly once with CoTaskMemFree on every decode path.
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)
            .map_err(|error| PathDiscoveryError(error.to_string()))?;
        let decoded = raw
            .to_string()
            .map(PathBuf::from)
            .map_err(|error| PathDiscoveryError(error.to_string()));
        CoTaskMemFree(Some(raw.as_ptr().cast()));
        decoded
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathDiscoveryError(String);

impl fmt::Display for PathDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed to resolve a platform folder: {}", self.0)
    }
}

impl std::error::Error for PathDiscoveryError {}
