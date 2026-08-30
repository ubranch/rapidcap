//! The icon set.
//!
//! One family: a `24` viewBox, `1.5` stroke, round caps and joins, no fills.
//! Four are filled by definition — the recording dot and the three transport
//! controls — because a hollow stop button reads as disabled.
//!
//! Paths are embedded rather than shipped as files: thirty tiny SVGs in an
//! assets folder is thirty chances for a path bug at runtime.

// The set is complete; surfaces adopt it phase by phase.
#![allow(dead_code)]

use std::borrow::Cow;

use gpui::{AnyElement, AssetSource, Hsla, IntoElement, Pixels, SharedString, Styled, svg};

/// An icon, identified by name. [`Icon::element`] renders it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    // capture
    Region,
    Window,
    Video,
    Gif,
    Gallery,
    Folder,
    // transport
    Pause,
    Play,
    Stop,
    Dot,
    Clock,
    Instant,
    // audio
    AudioOff,
    AudioLow,
    AudioOn,
    Microphone,
    // chrome
    Mark,
    KeepOnTop,
    Settings,
    Minimize,
    Close,
    Cycle,
    Back,
    Check,
    // files
    Copy,
    Delete,
    Saved,
    Open,
    Exit,
    Warning,
}

impl Icon {
    /// The SVG body for this icon: everything inside the `<svg>` element.
    const fn body(self) -> &'static str {
        match self {
            Self::Region => {
                r#"<path d="M4 8V5a1 1 0 0 1 1-1h3M20 8V5a1 1 0 0 0-1-1h-3M4 16v3a1 1 0 0 0 1 1h3m12-4v3a1 1 0 0 1-1 1h-3"/>"#
            }
            Self::Window => {
                r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/>"#
            }
            Self::Video => {
                r#"<rect x="2.5" y="6" width="13" height="12" rx="2"/><path d="m21.5 8-6 4 6 4z"/>"#
            }
            Self::Gif => {
                r#"<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 5v14M17 5v14"/>"#
            }
            Self::Gallery => {
                r#"<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8.5" cy="9.5" r="1.5"/><path d="m4 17 5-5 4 4 3-2 4 4"/>"#
            }
            Self::Folder => {
                r#"<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#
            }
            Self::Pause => {
                r#"<rect x="7" y="5" width="3.4" height="14" rx="1.2" fill="black" stroke="none"/><rect x="13.6" y="5" width="3.4" height="14" rx="1.2" fill="black" stroke="none"/>"#
            }
            Self::Play => r#"<path d="M8 5.5v13l11-6.5z" fill="black" stroke="none"/>"#,
            Self::Stop => {
                r#"<rect x="5" y="5" width="14" height="14" rx="2.4" fill="black" stroke="none"/>"#
            }
            Self::Dot => r#"<circle cx="12" cy="12" r="5" fill="black" stroke="none"/>"#,
            Self::Clock => r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#,
            Self::Instant => r#"<path d="M13 2 4.5 13H11l-1 9 8.5-11H12z"/>"#,
            Self::AudioOff => r#"<path d="M4 9h3l5-4v14l-5-4H4z"/><path d="m17 10 4 4m0-4-4 4"/>"#,
            Self::AudioLow => {
                r#"<path d="M4 9h3l5-4v14l-5-4H4z"/><path d="M17 9.5a3.5 3.5 0 0 1 0 5"/>"#
            }
            Self::AudioOn => {
                r#"<path d="M4 9h3l5-4v14l-5-4H4z"/><path d="M17 9.5a3.5 3.5 0 0 1 0 5M20 7a7 7 0 0 1 0 10"/>"#
            }
            Self::Microphone => {
                r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>"#
            }
            Self::Mark => r#"<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="3.2"/>"#,
            Self::KeepOnTop => r#"<path d="M9 4h6l-1 6 4 3v2H6v-2l4-3z"/><path d="M12 15v5"/>"#,
            Self::Settings => {
                r#"<circle cx="12" cy="12" r="3"/><path d="M12 3v2m0 14v2M3 12h2m14 0h2M5.6 5.6l1.4 1.4m10 10 1.4 1.4m0-12.8-1.4 1.4m-10 10-1.4 1.4"/>"#
            }
            Self::Minimize => r#"<path d="M5 12h14"/>"#,
            Self::Close => r#"<path d="M6 6l12 12M18 6 6 18"/>"#,
            Self::Cycle => {
                r#"<path d="M20 12a8 8 0 1 1-2.6-5.9L20 8"/><path d="M20 3v5h-5"/>"#
            }
            Self::Back => r#"<path d="M15 5l-7 7 7 7"/>"#,
            Self::Check => r#"<path d="m5 12 5 5 9-10"/>"#,
            Self::Copy => {
                r#"<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a1 1 0 0 1 1-1h9"/>"#
            }
            Self::Delete => r#"<path d="M5 7h14M9 7V5h6v2M7 7l1 12h8l1-12"/>"#,
            Self::Saved => {
                r#"<rect x="4" y="3" width="16" height="18" rx="2"/><path d="m8.5 12 2.5 2.5 4.5-5"/>"#
            }
            Self::Open => {
                r#"<path d="M14.5 9.5 21 3M21 3h-5m5 0v5"/><path d="M20 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h5"/>"#
            }
            Self::Exit => {
                r#"<path d="M9 5H6a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h3"/><path d="m15 8 4 4-4 4M19 12h-9"/>"#
            }
            Self::Warning => {
                r#"<circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><circle cx="12" cy="16.5" r=".9" fill="black" stroke="none"/>"#
            }
        }
    }

    /// Minimize and close are single lines and read thin at 1.5.
    const fn stroke(self) -> &'static str {
        match self {
            Self::Minimize | Self::Close => "1.6",
            Self::Check => "2",
            _ => "1.5",
        }
    }

    /// Stable asset name, without directory or extension.
    const fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Window => "window",
            Self::Video => "video",
            Self::Gif => "gif",
            Self::Gallery => "gallery",
            Self::Folder => "folder",
            Self::Pause => "pause",
            Self::Play => "play",
            Self::Stop => "stop",
            Self::Dot => "dot",
            Self::Clock => "clock",
            Self::Instant => "instant",
            Self::AudioOff => "audio-off",
            Self::AudioLow => "audio-low",
            Self::AudioOn => "audio-on",
            Self::Microphone => "microphone",
            Self::Mark => "mark",
            Self::KeepOnTop => "keep-on-top",
            Self::Settings => "settings",
            Self::Minimize => "minimize",
            Self::Close => "close",
            Self::Cycle => "cycle",
            Self::Back => "back",
            Self::Check => "check",
            Self::Copy => "copy",
            Self::Delete => "delete",
            Self::Saved => "saved",
            Self::Open => "open",
            Self::Exit => "exit",
            Self::Warning => "warning",
        }
    }

    /// A complete SVG document.
    ///
    /// Painted black on purpose: GPUI rasterises an SVG and then keeps only the
    /// alpha channel, tinting the mask with the element's text colour. A
    /// `currentColor` paint has no context to resolve against here and renders
    /// nothing at all — the icon silently disappears.
    pub fn to_svg(self) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"##,
            self.stroke(),
            self.body()
        )
    }

    /// The asset path GPUI resolves through [`IconAssets`].
    pub fn source(self) -> SharedString {
        SharedString::from(format!("icons/{}.svg", self.name()))
    }

    /// Render at `size` in `colour`.
    ///
    /// The colour has to be set *on the svg element itself*: GPUI reads
    /// `style.text.color` from the element being painted, and a parent's
    /// `text_color` does not populate it. Leave it unset and the icon paints
    /// nothing at all, with no warning.
    pub fn element(self, size: Pixels, colour: Hsla) -> AnyElement {
        svg()
            .path(self.source())
            .size(size)
            .flex_none()
            .text_color(colour)
            .into_any_element()
    }
}

/// Serves the embedded icon set to GPUI's SVG renderer.
///
/// Registered once with `Application::with_assets`. Without it every `svg()`
/// element silently paints nothing — the renderer resolves paths through the
/// asset source, and the default source returns `None` for everything.
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let Some(name) = path
            .strip_prefix("icons/")
            .and_then(|rest| rest.strip_suffix(".svg"))
        else {
            return Ok(None);
        };
        Ok(ALL_ICONS
            .iter()
            .find(|icon| icon.name() == name)
            .map(|icon| Cow::Owned(icon.to_svg().into_bytes())))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        if path.trim_end_matches('/') != "icons" {
            return Ok(Vec::new());
        }
        Ok(ALL_ICONS.iter().map(|icon| icon.source()).collect())
    }
}

/// Every icon, in declaration order. The asset source and the tests both walk
/// this, so a new variant that is not added here fails to resolve at runtime.
pub const ALL_ICONS: [Icon; 30] = [
    Icon::Region,
    Icon::Window,
    Icon::Video,
    Icon::Gif,
    Icon::Gallery,
    Icon::Folder,
    Icon::Pause,
    Icon::Play,
    Icon::Stop,
    Icon::Dot,
    Icon::Clock,
    Icon::Instant,
    Icon::AudioOff,
    Icon::AudioLow,
    Icon::AudioOn,
    Icon::Microphone,
    Icon::Mark,
    Icon::KeepOnTop,
    Icon::Settings,
    Icon::Minimize,
    Icon::Close,
    Icon::Cycle,
    Icon::Back,
    Icon::Check,
    Icon::Copy,
    Icon::Delete,
    Icon::Saved,
    Icon::Open,
    Icon::Exit,
    Icon::Warning,
];

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 30] = ALL_ICONS;

    #[test]
    fn every_icon_is_a_closed_svg_on_the_same_grid() {
        for icon in ALL {
            let svg = icon.to_svg();
            assert!(svg.starts_with("<svg"), "{icon:?} is not an svg");
            assert!(svg.ends_with("</svg>"), "{icon:?} is not closed");
            assert!(
                svg.contains(r#"viewBox="0 0 24 24""#),
                "{icon:?} is off the 24 grid"
            );
            assert!(!icon.body().is_empty(), "{icon:?} has no geometry");
        }
    }

    #[test]
    fn only_the_four_filled_icons_carry_a_fill() {
        // Filled geometry is the one exception to the outline family, so it is
        // worth failing a build over: a stray `fill` turns an outline icon into
        // a blob at 16px.
        for icon in ALL {
            let filled = icon.body().contains("fill=\"black\"");
            let expected = matches!(
                icon,
                Icon::Pause | Icon::Play | Icon::Stop | Icon::Dot | Icon::Warning
            );
            assert_eq!(filled, expected, "{icon:?} fill does not match the spec");
        }
    }

    #[test]
    fn every_icon_resolves_through_the_asset_source() {
        // The failure this guards is silent: an unresolved path paints nothing
        // and logs nothing, so a missing arm here shows up as a blank button.
        let assets = IconAssets;
        for icon in ALL {
            let loaded = assets
                .load(&icon.source())
                .expect("asset source must not error")
                .unwrap_or_else(|| panic!("{icon:?} did not resolve at {}", icon.source()));
            assert_eq!(loaded.as_ref(), icon.to_svg().as_bytes());
        }
        assert_eq!(assets.list("icons").unwrap().len(), ALL.len());
        assert!(assets.load("icons/nope.svg").unwrap().is_none());
        assert!(assets.load("fonts/thing.ttf").unwrap().is_none());
    }

    #[test]
    fn icon_names_are_unique() {
        let mut names: Vec<_> = ALL.iter().map(|icon| icon.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two icons share an asset name");
    }
}
