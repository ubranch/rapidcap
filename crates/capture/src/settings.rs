use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
            SHGetKnownFolderPath,
        },
    },
    core::GUID,
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
                fps: 60,
                bitrate: 3_000_000,
                preset: "p7".into(),
                tune: "hq".into(),
            },
            audio: AudioSettings {
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
    pub bitrate: u32,
    pub channels: u16,
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
    pub fn discover() -> Result<Self, PathDiscoveryError> {
        Ok(Self::from_roots(
            known_folder(&FOLDERID_Documents)?,
            known_folder(&FOLDERID_RoamingAppData)?,
            known_folder(&FOLDERID_LocalAppData)?,
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
        write!(
            formatter,
            "failed to resolve Windows known folder: {}",
            self.0
        )
    }
}

impl std::error::Error for PathDiscoveryError {}
