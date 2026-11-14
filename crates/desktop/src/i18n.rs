//! English and Uzbek (Latin) UI text.
//!
//! English is the source: every call site passes its English text to [`t`],
//! which hands back the Uzbek line when the UI runs in Uzbek. Text with values
//! in it uses named markers, `t("Recording {kind}").replace("{kind}", noun)`,
//! so the Uzbek line can put the value where its grammar wants it.

use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lang {
    En,
    Uz,
}

/// Decided once. `RAPIDCAP_LANG=uz|en` wins; otherwise the OS UI language.
/// Tests always run in English, so their assertions do not depend on the
/// machine they run on.
pub fn lang() -> Lang {
    static LANG: LazyLock<Lang> = LazyLock::new(|| {
        if cfg!(test) {
            return Lang::En;
        }
        match std::env::var("RAPIDCAP_LANG").as_deref() {
            Ok("uz") => Lang::Uz,
            Ok("en") => Lang::En,
            _ => os_lang(),
        }
    });
    *LANG
}

#[cfg(windows)]
fn os_lang() -> Lang {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    const LANG_UZBEK: u16 = 0x43;
    // SAFETY: no arguments, no failure mode; returns a LANGID.
    let id = unsafe { GetUserDefaultUILanguage() };
    if id & 0x3ff == LANG_UZBEK {
        Lang::Uz
    } else {
        Lang::En
    }
}

#[cfg(not(windows))]
fn os_lang() -> Lang {
    Lang::En
}

/// The UI text for `english` in the current language. Unknown text stays English.
pub fn t(english: &'static str) -> &'static str {
    match lang() {
        Lang::En => english,
        Lang::Uz => uz(english).unwrap_or(english),
    }
}

/// The Uzbek line for `english`, if the table has one.
pub fn uz(english: &str) -> Option<&'static str> {
    UZ.iter()
        .find(|(key, _)| *key == english)
        .map(|(_, text)| *text)
}

const UZ: &[(&str, &str)] = &[
    // Commands and capture kinds.
    ("Capture region", "Hududni olish"),
    ("Capture window", "Oynani olish"),
    ("Record video", "Video yozish"),
    ("Record GIF", "GIF yozish"),
    ("Pause · resume", "Pauza · davom ettirish"),
    ("Pause", "Pauza"),
    ("Resume", "Davom ettirish"),
    ("Stop", "To'xtatish"),
    ("Stop recording", "Yozishni to'xtatish"),
    ("Cancel", "Bekor qilish"),
    ("Cancel Video", "Videoni bekor qilish"),
    ("Cancel GIF", "GIF'ni bekor qilish"),
    ("Stop Video", "Videoni to'xtatish"),
    ("Stop GIF", "GIF'ni to'xtatish"),
    ("Region", "Hudud"),
    ("Window", "Oyna"),
    ("region", "hudud"),
    ("window", "oyna"),
    // States.
    ("Ready", "Tayyor"),
    ("Selecting", "Tanlanmoqda"),
    (
        "Selecting · Esc to cancel",
        "Tanlanmoqda · Esc – bekor qilish",
    ),
    ("Selected {w} × {h}", "Tanlandi: {w} × {h}"),
    ("Selected {name}", "Tanlandi: {name}"),
    ("{kind} starts in {n}", "{kind}: {n} soniyadan keyin"),
    (
        "{kind} in {n} · {target}",
        "{kind}: {n} soniyadan keyin · {target}",
    ),
    ("Recording {kind}", "{kind} yozilmoqda"),
    ("Paused {kind}", "{kind} pauzada"),
    ("Finalizing {kind}", "{kind} saqlanmoqda"),
    ("Finalizing {kind}…", "{kind} saqlanmoqda…"),
    ("Capture failed", "Olishda xato"),
    ("Closing…", "Yopilmoqda…"),
    ("Copied", "Nusxalandi"),
    ("Saved", "Saqlandi"),
    ("Saved · {size}", "Saqlandi · {size}"),
    ("Saved — open {path}", "Saqlandi — ochish: {path}"),
    // Panel, titlebar, tray.
    ("RapidCap screen capture", "RapidCap — ekranni olish"),
    ("RapidCap Selection", "RapidCap — tanlash"),
    (
        "RapidCap recording controls",
        "RapidCap — yozish boshqaruvi",
    ),
    ("Settings", "Sozlamalar"),
    ("Recent captures", "So'nggi fayllar"),
    ("Recent captures in {folder}", "So'nggi fayllar: {folder}"),
    ("Back to the panel", "Panelga qaytish"),
    ("Open output folder", "Fayllar papkasini ochish"),
    ("Minimize to tray", "Trayga yashirish"),
    ("Close", "Yopish"),
    ("Dismiss", "Yopish"),
    ("Retry", "Qayta urinish"),
    ("Copy log", "Logni nusxalash"),
    ("Show RapidCap", "RapidCap'ni ochish"),
    ("Hide RapidCap", "RapidCap'ni yashirish"),
    ("Quit RapidCap", "RapidCap'dan chiqish"),
    ("Exit", "Chiqish"),
    ("No countdown", "Taymersiz"),
    ("{n} second countdown", "{n} soniyalik taymer"),
    ("{shortcut} · taken", "{shortcut} · band"),
    // Overlay.
    (
        "Click window · Esc cancel",
        "Oynani tanlang · Esc – bekor qilish",
    ),
    (
        "Click window or drag region · Esc cancel",
        "Oynani tanlang yoki hududni belgilang · Esc – bekor qilish",
    ),
    // Audio.
    ("System audio", "Tizim ovozi"),
    ("Microphone", "Mikrofon"),
    ("Microphone: {device}", "Mikrofon: {device}"),
    ("No audio", "Ovozsiz"),
    ("Audio + mic", "Ovoz + mikrofon"),
    ("System audio and {device}", "Tizim ovozi va {device}"),
    (
        "{sources} — click to choose what a recording picks up",
        "{sources} — yozuv nimani olishini tanlash uchun bosing",
    ),
    ("Default playback device", "Asosiy ovoz qurilmasi"),
    ("Looking for capture devices…", "Qurilmalar qidirilmoqda…"),
    (
        "No capture devices found. Plug one in and reopen this menu.",
        "Qurilma topilmadi. Qurilmani ulang va menyuni qayta oching.",
    ),
    ("Off", "O'chiq"),
    // Gallery.
    ("All", "Hammasi"),
    ("Images", "Rasmlar"),
    ("Copy", "Nusxalash"),
    ("Show in folder", "Papkada ko'rsatish"),
    ("Delete", "O'chirish"),
    // Preview window.
    ("Open with default app", "Standart ilovada ochish"),
    ("Play in default app", "Standart ilovada ijro etish"),
    ("No captures yet", "Hali fayl yo'q"),
    (
        "Every capture is saved to",
        "Barcha fayllar shu yerga saqlanadi:",
    ),
    ("Just now", "Hozirgina"),
    ("{n} min ago", "{n} daqiqa oldin"),
    ("{n} h ago", "{n} soat oldin"),
    ("Yesterday", "Kecha"),
    // Settings.
    ("Capture", "Olish"),
    ("Countdown", "Taymer"),
    ("Shorter countdown", "Qisqaroq taymer"),
    ("Longer countdown", "Uzunroq taymer"),
    ("Copy to clipboard", "Buferga nusxalash"),
    ("Screenshots only", "Faqat skrinshotlar"),
    (
        "Hide panel while capturing",
        "Olish paytida panelni yashirish",
    ),
    ("Audio", "Ovoz"),
    (
        "Applies from the next recording",
        "Keyingi yozuvdan boshlab ishlaydi",
    ),
    ("Hotkeys", "Tezkor tugmalar"),
    ("Taken by another app", "Bu tugmani boshqa ilova ishlatadi"),
    ("Storage", "Saqlash"),
    ("Save to", "Saqlash joyi"),
    ("Open", "Ochish"),
    ("Reset all settings", "Barcha sozlamalarni tiklash"),
    (
        "Press again to reset everything",
        "Hammasini tiklash uchun yana bosing",
    ),
    ("There is no undo", "Buni qaytarib bo'lmaydi"),
    ("Reset", "Tiklash"),
    ("Confirm reset", "Tiklashni tasdiqlash"),
    (
        "RapidCap {version} · settings schema v{schema}",
        "RapidCap {version} · sozlamalar sxemasi v{schema}",
    ),
    ("Video frame rate", "Video kadr tezligi"),
    ("Video quality", "Video sifati"),
    ("GIF frame rate", "GIF kadr tezligi"),
    ("Dithering", "Ditering"),
    ("Audio bitrate", "Ovoz bitreyti"),
    (
        "Convert large PNGs to JPEG",
        "Katta PNG'ni JPEG'ga aylantirish",
    ),
    ("Frame rate", "Kadr tezligi"),
    ("Quality", "Sifat"),
    ("Bitrate", "Bitreyt"),
    ("Above {size}", "{size} dan katta"),
    ("Standard", "Standart"),
    ("High", "Yuqori"),
    ("Maximum", "Maksimal"),
    ("None", "Yo'q"),
    (
        "Smooth gradients at 256 colours",
        "256 rangda silliq gradient",
    ),
    (
        "Wider kernel, a little softer",
        "Kengroq yadro, biroz yumshoqroq",
    ),
    (
        "Classic kernel, a bit grainier",
        "Klassik yadro, biroz donadorroq",
    ),
    (
        "Smallest kernel, coarser grain",
        "Eng kichik yadro, yirikroq don",
    ),
    (
        "Fixed pattern, smaller files",
        "Qat'iy naqsh, kichikroq fayllar",
    ),
    (
        "No dithering, gradients band",
        "Ditering yo'q, gradient pog'onali",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &[&str] = &[
        include_str!("controller.rs"),
        include_str!("controls.rs"),
        include_str!("gallery.rs"),
        include_str!("icons.rs"),
        include_str!("main.rs"),
        include_str!("menu.rs"),
        include_str!("motion.rs"),
        include_str!("overlay.rs"),
        include_str!("platform/mod.rs"),
        include_str!("platform/macos.rs"),
        include_str!("platform/windows.rs"),
        include_str!("settings_view.rs"),
        include_str!("theme.rs"),
        include_str!("thumbs.rs"),
        include_str!("tray.rs"),
        include_str!("window.rs"),
    ];

    /// Every literal passed straight to `t(...)`.
    fn literals(source: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find("t(\"") {
            let before = rest[..at].chars().next_back();
            let after = &rest[at + 3..];
            rest = after;
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            if let Some(end) = after.find('"') {
                found.push(&after[..end]);
            }
        }
        found
    }

    #[test]
    fn every_uzbek_line_is_written() {
        for (key, text) in UZ {
            assert!(!key.is_empty() && !text.trim().is_empty(), "{key:?}");
            let markers = |s: &str| s.matches('{').count();
            assert_eq!(markers(key), markers(text), "markers differ in {key:?}");
        }
        for (index, (key, _)) in UZ.iter().enumerate() {
            assert!(
                UZ[..index].iter().all(|(other, _)| other != key),
                "{key:?} is in the table twice"
            );
        }
    }

    #[test]
    fn every_ui_string_has_an_uzbek_line() {
        let all: Vec<&str> = SOURCES.iter().flat_map(|source| literals(source)).collect();
        assert!(all.len() > 80, "the scan found only {} calls", all.len());
        let missing: Vec<&&str> = all.iter().filter(|key| uz(key).is_none()).collect();
        assert!(missing.is_empty(), "no Uzbek line for {missing:?}");
    }

    #[test]
    fn tests_run_in_english() {
        assert_eq!(lang(), Lang::En);
        assert_eq!(t("Settings"), "Settings");
        assert_eq!(uz("Settings"), Some("Sozlamalar"));
        assert_eq!(uz("not a key"), None);
    }
}
