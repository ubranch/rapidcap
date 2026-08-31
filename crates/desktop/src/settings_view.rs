//! Settings. The same 400px window as the panel, scrolled rather than tabbed.
//!
//! Six sections would be six tabs, and six tabs for a screen this short is
//! furniture. One column that scrolls puts every setting one gesture away and
//! costs nothing to build.
//!
//! Every row here writes a key that something reads. One row the design system
//! draws is not built: **hide panel while capturing** has no key on `Settings`
//! and no behaviour to switch off.
//!
//! The `Change` beside the capture folder is an `Open` instead: `capture_root`
//! lives on `AppPaths`, which is discovered at startup rather than saved, so
//! there is nothing a folder picker could write to.

use gpui::{
    Anchor, Context, Entity, FocusHandle, FontWeight, IntoElement, ParentElement as _, Render,
    Role, ScrollHandle, SharedString, Styled as _, Window, anchored, deferred, div, prelude::*, px,
};
use rapidcap_capture::{CaptureCommand, Settings};

use crate::controller::AppController;
use crate::platform::{hotkey_specs, open_path};
use crate::window::{
    SelectConfirmAction, SelectFirstAction, SelectLastAction, SelectNextAction, SelectPrevAction,
};
use crate::{controls, menu, theme};

/// Frame rates offered for a video recording.
///
/// Three, not a free number: 24 is what film looks like, 30 is the default and
/// what a screen recording is normally shared at, and 60 is for anything with
/// motion in it. Everything between them is a file-size decision nobody makes
/// deliberately.
const VIDEO_FPS: [u32; 3] = [24, 30, 60];

/// GIF frame rates. Lower than video on purpose - a GIF pays for every frame
/// twice, once in palette and once in size.
const GIF_FPS: [u32; 3] = [10, 15, 20];

const AUDIO_BITRATE: [u32; 3] = [96_000, 128_000, 192_000];

/// Where a PNG screenshot stops being worth keeping lossless.
///
/// There is no `off` here because there is no key for one: the threshold is a
/// `usize` compared against the encoded length, so switching conversion off
/// would mean writing `usize::MAX` into the settings file and calling that a
/// setting. Four real sizes instead.
const JPEG_THRESHOLD: [usize; 4] = [1 << 20, 2 << 20, 4 << 20, 8 << 20];

/// A video quality tier: one name over the three keys that actually change.
///
/// The design system shows `High` with `3 Mbps · NVENC preset p7, tune hq`
/// underneath it, which is the whole reason this is one row rather than three -
/// nobody picks a preset string, they pick how good it should look.
struct Quality {
    name: &'static str,
    bitrate: u32,
    preset: &'static str,
    tune: &'static str,
}

const QUALITY: [Quality; 3] = [
    Quality {
        name: "Standard",
        bitrate: 1_500_000,
        preset: "p5",
        tune: "hq",
    },
    Quality {
        name: "High",
        bitrate: 3_000_000,
        preset: "p7",
        tune: "hq",
    },
    Quality {
        name: "Maximum",
        bitrate: 6_000_000,
        preset: "p7",
        tune: "hq",
    },
];

/// The countdown, as a stepper rather than the panel's segmented control.
///
/// The same three delays either way. The panel has one setting to spare a whole
/// track to; a settings row has 30px and a label beside it.
const COUNTDOWN: [u8; 3] = AppController::COUNTDOWN_CHOICES;

/// The dithers, in FFmpeg's spelling, with what to call each one and what it
/// costs. The order is the one `DITHERS` validates against, so a file the
/// loader accepts is always a choice this select can tick.
///
/// The blurb finishes the sentence the label starts, which is how the design
/// draws it: "Sierra 2-4A keeps gradients smooth at 256 colours".
const GIF_DITHER: [(&str, &str, &str); 6] = [
    (
        "sierra2_4a",
        "Sierra 2-4A",
        "keeps gradients smooth at 256 colours",
    ),
    (
        "sierra2",
        "Sierra 2",
        "diffuses over a wider kernel, slower and a little softer",
    ),
    (
        "floyd_steinberg",
        "Floyd-Steinberg",
        "is the classic kernel, slightly grainier than Sierra",
    ),
    (
        "heckbert",
        "Heckbert",
        "uses the smallest kernel, so the grain is coarser",
    ),
    (
        "bayer",
        "Bayer",
        "lays down a fixed pattern instead of diffusing, and compresses smaller",
    ),
    (
        "none",
        "None",
        "maps straight onto the palette, which bands every gradient",
    ),
];

/// Every select on the screen.
///
/// One enum rather than five little widgets, because the difference between
/// them is a table of values and a line that writes it - everything else,
/// including the whole keyboard route through the menu, is identical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Select {
    VideoFps,
    Quality,
    GifFps,
    GifDither,
    AudioBitrate,
    JpegThreshold,
}

impl Select {
    const fn id(self) -> &'static str {
        match self {
            Self::VideoFps => "settings-video-fps",
            Self::Quality => "settings-quality",
            Self::GifFps => "settings-gif-fps",
            Self::GifDither => "settings-gif-dither",
            Self::AudioBitrate => "settings-audio-bitrate",
            Self::JpegThreshold => "settings-jpeg-threshold",
        }
    }

    /// The row the pill sits in. Same reason as [`Self::menu_id`]: the row is
    /// the pill's parent, not its sibling, but an id repeated down a branch is
    /// still one path in the dispatch tree.
    const fn row_id(self) -> &'static str {
        match self {
            Self::VideoFps => "settings-video-fps-row",
            Self::Quality => "settings-quality-row",
            Self::GifFps => "settings-gif-fps-row",
            Self::GifDither => "settings-gif-dither-row",
            Self::AudioBitrate => "settings-audio-bitrate-row",
            Self::JpegThreshold => "settings-jpeg-threshold-row",
        }
    }

    /// The menu's own id. The pill and the menu are siblings under the same
    /// parent, and two siblings sharing an `ElementId` is one hit-test target
    /// for both of them.
    const fn menu_id(self) -> &'static str {
        match self {
            Self::VideoFps => "settings-video-fps-menu",
            Self::Quality => "settings-quality-menu",
            Self::GifFps => "settings-gif-fps-menu",
            Self::GifDither => "settings-gif-dither-menu",
            Self::AudioBitrate => "settings-audio-bitrate-menu",
            Self::JpegThreshold => "settings-jpeg-threshold-menu",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::VideoFps => "Video frame rate",
            Self::Quality => "Video quality",
            Self::GifFps => "GIF frame rate",
            Self::GifDither => "Dithering",
            Self::AudioBitrate => "Audio bitrate",
            Self::JpegThreshold => "Convert large PNGs to JPEG",
        }
    }

    /// How many choices this select offers.
    fn len(self) -> usize {
        match self {
            Self::VideoFps => VIDEO_FPS.len(),
            Self::Quality => QUALITY.len(),
            Self::GifFps => GIF_FPS.len(),
            Self::GifDither => GIF_DITHER.len(),
            Self::AudioBitrate => AUDIO_BITRATE.len(),
            Self::JpegThreshold => JPEG_THRESHOLD.len(),
        }
    }

    /// Which choice is currently set, or `None` when the settings file holds a
    /// value that is not on the list. Hand-edited files do that, and a select
    /// that ticks nothing is the honest way to show it.
    fn current(self, settings: &Settings) -> Option<usize> {
        match self {
            Self::VideoFps => VIDEO_FPS.iter().position(|fps| *fps == settings.video.fps),
            Self::Quality => QUALITY
                .iter()
                .position(|quality| quality.bitrate == settings.video.bitrate),
            Self::GifFps => GIF_FPS.iter().position(|fps| *fps == settings.gif.fps),
            Self::GifDither => GIF_DITHER
                .iter()
                .position(|(name, ..)| *name == settings.gif.dither),
            Self::AudioBitrate => AUDIO_BITRATE
                .iter()
                .position(|rate| *rate == settings.audio.bitrate),
            Self::JpegThreshold => JPEG_THRESHOLD
                .iter()
                .position(|bytes| *bytes == settings.screenshot.png_to_jpeg_threshold_bytes),
        }
    }

    /// What one choice is called.
    fn name(self, index: usize) -> SharedString {
        match self {
            Self::VideoFps => format!("{} FPS", VIDEO_FPS[index]).into(),
            Self::Quality => QUALITY[index].name.into(),
            Self::GifFps => format!("{} FPS", GIF_FPS[index]).into(),
            Self::GifDither => GIF_DITHER[index].1.into(),
            Self::AudioBitrate => format!("{} kbps", AUDIO_BITRATE[index] / 1_000).into(),
            Self::JpegThreshold => format!("Above {}", megabytes(JPEG_THRESHOLD[index])).into(),
        }
    }

    /// The right-hand column: what the choice costs, never what it is called
    /// again. Only quality has one, because a preset string is the only thing
    /// on this screen a name genuinely hides.
    fn detail(self, index: usize) -> Option<SharedString> {
        match self {
            Self::Quality => {
                let quality = &QUALITY[index];
                Some(
                    format!(
                        "{} Mbps · {} {}",
                        quality.bitrate / 1_000_000,
                        quality.preset,
                        quality.tune
                    )
                    .into(),
                )
            }
            _ => None,
        }
    }

    /// The second line under the label, which makes the row tall.
    ///
    /// Only dithering has one. The others are a number in a unit the label
    /// already names, and a row that restates its own value in smaller grey
    /// text is a row that has nothing to say.
    ///
    /// Nothing when the file holds a name that is not on the list: there is no
    /// blurb to show for a value this screen does not know.
    fn sub(self, settings: &Settings) -> Option<SharedString> {
        match self {
            Self::GifDither => self.current(settings).map(|index| {
                let (_, name, blurb) = GIF_DITHER[index];
                format!("{name} {blurb}").into()
            }),
            _ => None,
        }
    }

    /// What the closed pill reads.
    fn value(self, settings: &Settings) -> SharedString {
        match self.current(settings) {
            Some(index) => self.name(index),
            // Whatever is in the file, shown as-is. A select that silently
            // rounded to the nearest offered value would rewrite a deliberate
            // hand edit the first time the screen was opened.
            None => match self {
                Self::VideoFps => format!("{} FPS", settings.video.fps).into(),
                Self::Quality => format!("{} Mbps", settings.video.bitrate / 1_000_000).into(),
                Self::GifFps => format!("{} FPS", settings.gif.fps).into(),
                Self::GifDither => settings.gif.dither.clone().into(),
                Self::AudioBitrate => format!("{} kbps", settings.audio.bitrate / 1_000).into(),
                Self::JpegThreshold => format!(
                    "Above {}",
                    megabytes(settings.screenshot.png_to_jpeg_threshold_bytes)
                )
                .into(),
            },
        }
    }

    fn entries(self, settings: &Settings) -> Vec<menu::Entry> {
        let current = self.current(settings);
        (0..self.len())
            .map(|index| {
                let mut item = menu::Item::new(format!("{}-{index}", self.id()), self.name(index))
                    .selected(current == Some(index));
                if let Some(detail) = self.detail(index) {
                    item = item.detail(detail);
                }
                menu::Entry::Item(item)
            })
            .collect()
    }

    /// Write the choice. Quality moves three keys at once, which is the whole
    /// reason it is a tier and not a bitrate field.
    fn choose(self, index: usize, settings: &mut Settings) {
        match self {
            Self::VideoFps => settings.video.fps = VIDEO_FPS[index],
            Self::Quality => {
                let quality = &QUALITY[index];
                settings.video.bitrate = quality.bitrate;
                settings.video.preset = quality.preset.to_string();
                settings.video.tune = quality.tune.to_string();
            }
            Self::GifFps => settings.gif.fps = GIF_FPS[index],
            Self::GifDither => settings.gif.dither = GIF_DITHER[index].0.to_string(),
            Self::AudioBitrate => settings.audio.bitrate = AUDIO_BITRATE[index],
            Self::JpegThreshold => {
                settings.screenshot.png_to_jpeg_threshold_bytes = JPEG_THRESHOLD[index]
            }
        }
    }
}

/// A byte count as whole megabytes. Every threshold offered is one, so there is
/// no fractional case to get wrong.
fn megabytes(bytes: usize) -> String {
    format!("{} MB", bytes / (1 << 20))
}

pub struct SettingsView {
    controller: Entity<AppController>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// The select showing its menu, and where the arrows are inside it. Only
    /// one can be open: opening a second would leave two menus over a 400px
    /// column with no room for either.
    open: Option<(Select, Option<usize>)>,
    /// Whether Reset has been pressed once already.
    ///
    /// Reset throws away every setting on the screen and there is no undo, so
    /// it asks twice. A modal for one button is furniture; the button changing
    /// its own label is the same question with none of the machinery.
    confirm_reset: bool,
    _controller: gpui::Subscription,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>, controller: Entity<AppController>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            open: None,
            confirm_reset: false,
            // Settings can move without this screen touching them - the panel
            // owns the countdown and the audio chip too.
            _controller: cx.observe(&controller, |_, _, cx| cx.notify()),
            controller,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Close whatever is open. Called by Escape, which the panel handles: one
    /// press shuts the menu, the next leaves the screen.
    ///
    /// Reports whether it had anything to close, so the caller knows whether
    /// the press was already spent.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.open.take().is_some() | std::mem::take(&mut self.confirm_reset);
        if was_open {
            cx.notify();
        }
        was_open
    }

    /// Shut an open menu and say whether there was one.
    ///
    /// Narrower than `dismiss`, which also disarms Reset: this is for Tab,
    /// which moves through the panel without deciding anything about the
    /// button it is moving off.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.open.take().is_some();
        if was_open {
            cx.notify();
        }
        was_open
    }

    fn toggle(&mut self, select: Select, cx: &mut Context<Self>) {
        // Arming Reset and then reaching for a menu is a change of mind.
        self.confirm_reset = false;
        self.open = match self.open {
            Some((open, _)) if open == select => None,
            // Opens on the row it is already set to, so the first arrow press
            // steps from the current value rather than from the top. By entry
            // rather than by choice: they line up on this screen, and would
            // stop lining up the day one of these menus grew a header.
            _ => {
                let entries = select.entries(self.controller.read(cx).settings());
                Some((select, menu::selected(&entries)))
            }
        };
        cx.notify();
    }

    /// Down and Up on a select.
    ///
    /// Closed, either one opens it - that is what the pill's
    /// `aria_keyshortcuts` has been promising, and what every combo box on
    /// both platforms does. Open, they walk the rows, wrapping at both ends
    /// and stepping over anything that cannot be chosen.
    fn menu_step(&mut self, select: Select, delta: isize, cx: &mut Context<Self>) {
        let entries = select.entries(self.controller.read(cx).settings());
        let Some((open, highlighted)) = self.open.filter(|(open, _)| *open == select) else {
            self.toggle(select, cx);
            return;
        };
        self.open = Some((open, menu::step(&entries, highlighted, delta)));
        cx.notify();
    }

    /// Home and End on a select.
    ///
    /// Open, they go to the first and last row that can be chosen. Closed, they
    /// mean what they mean everywhere else in the panel, so the press is handed
    /// on: the pill's binding is the more specific of the two and is offered
    /// the key first whether or not a menu is showing, and GPUI walks the rest
    /// of the matched bindings only for a handler that propagates.
    ///
    /// `step` from nothing already lands on the first row going forward and the
    /// last going back, which is the whole of what these two need.
    fn menu_edge(&mut self, select: Select, delta: isize, cx: &mut Context<Self>) {
        let Some((open, _)) = self.open.filter(|(open, _)| *open == select) else {
            cx.propagate();
            return;
        };
        let entries = select.entries(self.controller.read(cx).settings());
        self.open = Some((open, menu::step(&entries, None, delta)));
        cx.notify();
    }

    /// Enter and Space on a select.
    ///
    /// Closed, they open it. Open on a row, they take that row. Open on
    /// nothing - which only an empty menu can be - they shut it, because there
    /// is nothing there to take.
    fn menu_confirm(&mut self, select: Select, cx: &mut Context<Self>) {
        match self.open {
            Some((open, Some(index))) if open == select => self.choose(select, index, cx),
            Some((open, None)) if open == select => {
                self.open = None;
                cx.notify();
            }
            _ => self.toggle(select, cx),
        }
    }

    fn choose(&mut self, select: Select, index: usize, cx: &mut Context<Self>) {
        self.open = None;
        self.controller.update(cx, |controller, cx| {
            controller.edit_settings(|settings| select.choose(index, settings), cx);
        });
        cx.notify();
    }

    fn section(&self, label: &'static str, rows: Vec<gpui::AnyElement>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                // Outside the card, not in it: a card is one group of related
                // rows, and a heading inside it is a row that cannot be used.
                div()
                    .px(theme::u(6.0))
                    .pb(theme::u(6.0))
                    .text_size(theme::u(theme::TEXT_TINY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_muted())
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme::u(2.0))
                    .p(theme::u(4.0))
                    .rounded(theme::u(theme::RADIUS))
                    .bg(theme::bg_card())
                    .border_1()
                    .border_color(theme::border_card())
                    .children(rows),
            )
    }

    /// A select row: the pill, and the menu hanging under it when it is open.
    ///
    /// The menu is `deferred` so it paints over the rows below rather than
    /// under them, and anchored to the pill's own position rather than to a
    /// measured point - it is rendered where the pill is, which is what
    /// `anchored` uses when it is given no position of its own.
    fn select_row(&self, select: Select, cx: &mut Context<Self>) -> gpui::AnyElement {
        let settings = self.controller.read(cx).settings().clone();
        let open = match self.open {
            Some((open, highlighted)) if open == select => Some(highlighted),
            _ => None,
        };
        let entries = select.entries(&settings);
        controls::row(select.row_id(), select.label(), select.sub(&settings))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .child(
                        controls::select(
                            select.id(),
                            select.label(),
                            select.value(&settings),
                            open.is_some(),
                        )
                        .on_action(cx.listener(move |this, _: &SelectNextAction, _, cx| {
                            this.menu_step(select, 1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &SelectPrevAction, _, cx| {
                            this.menu_step(select, -1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &SelectFirstAction, _, cx| {
                            this.menu_edge(select, 1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &SelectLastAction, _, cx| {
                            this.menu_edge(select, -1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &SelectConfirmAction, _, cx| {
                            this.menu_confirm(select, cx)
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(select, cx))),
                    )
                    .children(open.map(|highlighted| {
                        deferred(
                            anchored()
                                .anchor(Anchor::TopLeft)
                                // Clear of the pill it belongs to, by the same
                                // 4px the menu keeps inside its own border.
                                .offset(gpui::point(px(0.0), px(34.0)))
                                .snap_to_window_with_margin(px(8.0))
                                .child(
                                    menu::surface(select.menu_id(), menu::WIDTH)
                                        // A click anywhere else shuts it, which is
                                        // the half of a menu that Escape does not
                                        // cover: the pointer is already somewhere
                                        // other than the menu by the time anyone
                                        // notices it is still open.
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.open = None;
                                            cx.notify();
                                        }))
                                        .children(entries.iter().enumerate().map(
                                            |(index, entry)| {
                                                let row =
                                                    menu::entry(entry, highlighted == Some(index));
                                                div()
                                                    .id(("menu-row", index))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.choose(select, index, cx)
                                                    }))
                                                    .child(row)
                                            },
                                        )),
                                ),
                        )
                    })),
            )
            .into_any_element()
    }

    /// Reset, which asks twice.
    ///
    /// The second press is the destructive one, so that is the press that turns
    /// the button red - a control that is already red before it does anything
    /// irreversible is a colour, not a warning.
    fn reset_button(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let armed = self.confirm_reset;
        controls::ghost_button(
            "settings-reset-button",
            if armed { "Confirm reset" } else { "Reset" },
            armed,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !armed {
                this.confirm_reset = true;
                cx.notify();
                return;
            }
            this.confirm_reset = false;
            this.controller.update(cx, |controller, cx| {
                controller.edit_settings(|settings| *settings = Settings::default(), cx);
            });
            cx.notify();
        }))
    }

    fn hotkey_rows(&self) -> Vec<gpui::AnyElement> {
        hotkey_specs()
            .into_iter()
            .map(|spec| {
                controls::row(
                    command_id(spec.command()),
                    command_name(spec.command()),
                    None,
                )
                .child(controls::hotkey_field(spec.label()))
                .into_any_element()
            })
            .collect()
    }
}

/// What a command is called on the hotkeys list.
///
/// Written out here rather than reused from the mode cards: a card says
/// `Region` because an icon beside it already says what a region is, and a bare
/// row has nothing to lean on.
const fn command_name(command: CaptureCommand) -> &'static str {
    match command {
        CaptureCommand::CaptureRegion => "Capture region",
        CaptureCommand::CaptureActiveWindow => "Capture window",
        CaptureCommand::ToggleVideo => "Record video",
        CaptureCommand::ToggleGif => "Record GIF",
        CaptureCommand::TogglePause => "Pause · resume",
        CaptureCommand::Cancel => "Cancel",
    }
}

const fn command_id(command: CaptureCommand) -> &'static str {
    match command {
        CaptureCommand::CaptureRegion => "hotkey-region",
        CaptureCommand::CaptureActiveWindow => "hotkey-window",
        CaptureCommand::ToggleVideo => "hotkey-video",
        CaptureCommand::ToggleGif => "hotkey-gif",
        CaptureCommand::TogglePause => "hotkey-pause",
        CaptureCommand::Cancel => "hotkey-cancel",
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.controller.read(cx).settings().clone();
        let paths = self.controller.read(cx).paths().clone();
        let countdown = settings.countdown_seconds;
        let index = COUNTDOWN.iter().position(|slot| *slot == countdown);
        let folder = paths.capture_root.display().to_string();
        let capture_root = paths.capture_root.clone();

        div()
            .id("settings")
            .track_focus(&self.focus_handle)
            .role(Role::Group)
            .aria_label("Settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(theme::u(theme::GAP))
            .p(theme::u(theme::PAD))
            .child(self.section(
                "Capture",
                vec![
                        controls::row("settings-countdown", "Countdown", None)
                            .child(
                                controls::stepper("settings-countdown-stepper")
                                    .child(
                                        controls::step_button(
                                            "countdown-down",
                                            "Shorter countdown",
                                            controls::Step::Down,
                                            index.is_some_and(|index| index > 0),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.controller.update(cx, |controller, cx| {
                                                    controller.set_countdown(
                                                        COUNTDOWN
                                                            [index.unwrap_or(1).saturating_sub(1)],
                                                        cx,
                                                    );
                                                });
                                            }),
                                        ),
                                    )
                                    .child(controls::step_value(countdown_label(countdown)))
                                    .child(
                                        controls::step_button(
                                            "countdown-up",
                                            "Longer countdown",
                                            controls::Step::Up,
                                            index.is_some_and(|index| index + 1 < COUNTDOWN.len()),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.controller.update(cx, |controller, cx| {
                                                    let next = (index.unwrap_or(0) + 1)
                                                        .min(COUNTDOWN.len() - 1);
                                                    controller.set_countdown(COUNTDOWN[next], cx);
                                                });
                                            }),
                                        ),
                                    ),
                            )
                            .into_any_element(),
                        controls::row(
                            "settings-clipboard",
                            "Copy to clipboard",
                            Some(
                                "Screenshots only - recordings are always saved to disk".into(),
                            ),
                        )
                        .child(
                            controls::toggle(
                                "settings-clipboard-toggle",
                                "Copy to clipboard",
                                settings.screenshot.copy_to_clipboard,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.controller.update(cx, |controller, cx| {
                                    controller.toggle_copy_to_clipboard(cx)
                                });
                            })),
                        )
                        .into_any_element(),
                    ],
            ))
            .child(self.section(
                "Video",
                vec![
                    self.select_row(Select::VideoFps, cx),
                    self.select_row(Select::Quality, cx),
                ],
            ))
            .child(self.section(
                "GIF",
                vec![
                    self.select_row(Select::GifFps, cx),
                    self.select_row(Select::GifDither, cx),
                ],
            ))
            .child(self.section(
                "Audio",
                vec![
                        controls::row(
                            "settings-audio",
                            "System audio",
                            Some("Takes effect on the next recording, not the running one".into()),
                        )
                        .child(
                            controls::toggle(
                                "settings-audio-toggle",
                                "System audio",
                                settings.audio.enabled,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.controller
                                    .update(cx, |controller, cx| controller.toggle_audio(cx));
                            })),
                        )
                        .into_any_element(),
                        self.select_row(Select::AudioBitrate, cx),
                    ],
            ))
            .child(self.section("Hotkeys", self.hotkey_rows()))
            .child(self.section(
                "Storage",
                vec![
                    controls::row("settings-folder", "Save to", None)
                        .child(
                            controls::path_field(
                                "settings-folder-path",
                                folder_tail(&folder),
                                folder,
                            )
                            .child(
                                controls::action_button("settings-folder-open", "Open").on_click(
                                    cx.listener(move |_, _, _, _| {
                                        if let Err(error) = open_path(&capture_root) {
                                            tracing::error!(%error, "open capture folder");
                                        }
                                    }),
                                ),
                            ),
                        )
                        .into_any_element(),
                    self.select_row(Select::JpegThreshold, cx),
                    controls::row(
                        "settings-reset",
                        "Reset all settings",
                        Some(if self.confirm_reset {
                            "Press again to put every setting back to its default".into()
                        } else {
                            "There is no undo".into()
                        }),
                    )
                    .child(self.reset_button(cx))
                    .into_any_element(),
                ],
            ))
            .child(
                // The file, named, for the settings this screen deliberately
                // does not show: the schema version, the audio channel count
                // and the raw NVENC strings all live there and nowhere else.
                div()
                    .px(theme::u(6.0))
                    .pt(theme::u(2.0))
                    .text_size(theme::u(theme::TEXT_MICRO))
                    .text_color(theme::text_muted())
                    .child(format!(
                        "RapidCap {} · settings schema v{} · {}",
                        env!("CARGO_PKG_VERSION"),
                        settings.schema_version,
                        paths.settings_file.display()
                    )),
            )
    }
}

/// The countdown as a stepper reads it. `Off` rather than `0 s`, because zero
/// seconds of waiting is not a duration anybody sets.
fn countdown_label(seconds: u8) -> String {
    if seconds == 0 {
        "Off".to_string()
    } else {
        format!("{seconds} s")
    }
}

/// The last two components of a path.
///
/// The head of a capture folder is `C:\Users\<name>\Documents` on every machine
/// that will ever run this, and the tail is the part that identifies it. The
/// full string is on the field itself for anyone who needs it.
fn folder_tail(path: &str) -> String {
    let separator = |c: char| c == '\\' || c == '/';
    let mut parts: Vec<&str> = path.rsplitn(3, separator).collect();
    if parts.len() < 3 {
        return path.to_string();
    }
    parts.truncate(2);
    parts.reverse();
    format!("…\\{}", parts.join("\\"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapidcap_capture::DITHERS;

    #[test]
    fn the_dither_menu_offers_exactly_what_the_loader_accepts() {
        // Two lists of FFmpeg spellings in two crates. If they drift, a file
        // the loader accepts shows up here as an unticked select, or a choice
        // made here is refused on the next load.
        let offered: Vec<&str> = GIF_DITHER.iter().map(|(name, ..)| *name).collect();
        assert_eq!(offered, DITHERS);
    }

    #[test]
    fn the_dither_row_explains_the_choice_that_is_set() {
        let mut settings = Settings::default();
        assert_eq!(
            Select::GifDither.sub(&settings),
            Some("Sierra 2-4A keeps gradients smooth at 256 colours".into())
        );

        Select::GifDither.choose(4, &mut settings);
        assert_eq!(settings.gif.dither, "bayer");
        assert_eq!(Select::GifDither.value(&settings), "Bayer");

        // A hand-edited name the screen does not know: the pill shows it back
        // rather than rounding it, and the row has no blurb to offer.
        settings.gif.dither = "sierra3".into();
        assert_eq!(Select::GifDither.value(&settings), "sierra3");
        assert_eq!(Select::GifDither.sub(&settings), None);
    }

    #[test]
    fn every_select_offers_its_own_default_back() {
        // A screen that cannot show the value the app ships with would tick
        // nothing on first open, which reads as a broken control rather than a
        // deliberate one.
        let settings = Settings::default();
        for select in [
            Select::VideoFps,
            Select::Quality,
            Select::GifFps,
            Select::GifDither,
            Select::AudioBitrate,
            Select::JpegThreshold,
        ] {
            assert!(
                select.current(&settings).is_some(),
                "{} does not offer its default",
                select.label()
            );
        }
    }

    #[test]
    fn choosing_a_quality_moves_all_three_keys_together() {
        let mut settings = Settings::default();
        Select::Quality.choose(0, &mut settings);
        assert_eq!(settings.video.bitrate, QUALITY[0].bitrate);
        assert_eq!(settings.video.preset, QUALITY[0].preset);
        assert_eq!(settings.video.tune, QUALITY[0].tune);
        assert_eq!(Select::Quality.current(&settings), Some(0));
    }

    #[test]
    fn a_hand_edited_value_is_shown_rather_than_rounded() {
        let mut settings = Settings::default();
        settings.video.fps = 47;
        assert_eq!(Select::VideoFps.current(&settings), None);
        assert_eq!(Select::VideoFps.value(&settings), "47 FPS");
        // And nothing is ticked, so the menu does not claim 47 is one of these.
        assert_eq!(menu::selected(&Select::VideoFps.entries(&settings)), None);
    }

    #[gpui::test]
    fn escape_shuts_an_open_menu_before_it_leaves_the_screen(cx: &mut gpui::TestAppContext) {
        let controller = cx.new(|_| {
            AppController::new(
                Settings::default(),
                rapidcap_capture::AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local"),
            )
        });
        let view = cx.new(|cx| SettingsView::new(cx, controller));

        // Nothing open: the press belongs to whoever asked, which is the window
        // and means Back.
        assert!(!view.update(cx, |view, cx| view.dismiss(cx)));

        view.update(cx, |view, cx| view.toggle(Select::VideoFps, cx));
        assert!(view.read_with(cx, |view, _| view.open.is_some()));
        // Open on the value already set, so the first arrow steps from 30 FPS.
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((
                Select::VideoFps,
                Select::VideoFps.current(&Settings::default())
            ))
        );

        assert!(view.update(cx, |view, cx| view.dismiss(cx)));
        assert!(view.read_with(cx, |view, _| view.open.is_none()));
    }

    #[gpui::test]
    fn the_arrows_walk_a_menu_and_enter_takes_the_row_they_are_on(cx: &mut gpui::TestAppContext) {
        let controller = cx.new(|_| {
            AppController::new(
                Settings::default(),
                rapidcap_capture::AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local"),
            )
        });
        let view = cx.new(|cx| SettingsView::new(cx, controller.clone()));
        let entries = Select::VideoFps.entries(&Settings::default());
        let default = menu::selected(&entries).unwrap();

        // Closed, Down opens rather than stepping: there is nothing to step
        // through yet, and a first press that skips the current value would
        // make the row under the pointer disagree with the pill above it.
        view.update(cx, |view, cx| view.menu_step(Select::VideoFps, 1, cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((Select::VideoFps, Some(default)))
        );

        view.update(cx, |view, cx| view.menu_step(Select::VideoFps, 1, cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((Select::VideoFps, Some(default + 1)))
        );

        // Off the bottom and back to the top, which is the half of `step` that
        // a menu of four rows makes reachable in two presses.
        for _ in 0..entries.len() {
            view.update(cx, |view, cx| view.menu_step(Select::VideoFps, 1, cx));
        }
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((Select::VideoFps, Some(default + 1)))
        );

        view.update(cx, |view, cx| view.menu_step(Select::VideoFps, -1, cx));
        view.update(cx, |view, cx| view.menu_confirm(Select::VideoFps, cx));
        assert_eq!(view.read_with(cx, |view, _| view.open), None);
        assert_eq!(
            controller.read_with(cx, |controller, _| controller.settings().video.fps),
            VIDEO_FPS[default],
            "the row Enter was on, written"
        );

        // Enter on a closed select opens it, which is what the pill's
        // `aria_keyshortcuts` has claimed since the screen was built.
        view.update(cx, |view, cx| view.menu_confirm(Select::Quality, cx));
        assert!(view.read_with(cx, |view, _| matches!(
            view.open,
            Some((Select::Quality, _))
        )));

        // And a menu is only ever open on one select: arrowing into another
        // pill moves the menu rather than opening a second one over it.
        view.update(cx, |view, cx| view.menu_step(Select::GifFps, -1, cx));
        assert!(view.read_with(cx, |view, _| matches!(view.open, Some((Select::GifFps, _)))));
    }

    #[gpui::test]
    fn home_and_end_go_to_the_ends_of_an_open_menu_and_tab_shuts_it(cx: &mut gpui::TestAppContext) {
        let controller = cx.new(|_| {
            AppController::new(
                Settings::default(),
                rapidcap_capture::AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local"),
            )
        });
        let view = cx.new(|cx| SettingsView::new(cx, controller.clone()));
        let entries = Select::VideoFps.entries(&Settings::default());
        let last = entries.len() - 1;

        view.update(cx, |view, cx| view.toggle(Select::VideoFps, cx));
        view.update(cx, |view, cx| view.menu_edge(Select::VideoFps, -1, cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((Select::VideoFps, Some(last))),
            "End goes to the bottom row"
        );

        view.update(cx, |view, cx| view.menu_edge(Select::VideoFps, 1, cx));
        assert_eq!(
            view.read_with(cx, |view, _| view.open),
            Some((Select::VideoFps, Some(0))),
            "and Home back to the top, which for this menu is its first row"
        );

        // Tab takes the menu and nothing else. Reset stays armed, which is what
        // separates this from Escape.
        view.update(cx, |view, cx| {
            view.confirm_reset = true;
            assert!(view.close_menu(cx), "there was a menu to close");
            assert_eq!(view.open, None);
            assert!(
                view.confirm_reset,
                "Tab is not a change of mind about Reset"
            );
            assert!(!view.close_menu(cx), "and nothing to close the second time");
        });

        // Closed, Home leaves the menu shut rather than opening one: it is the
        // panel's Home, and the pill only had first refusal on it.
        view.update(cx, |view, cx| view.menu_edge(Select::VideoFps, 1, cx));
        assert_eq!(view.read_with(cx, |view, _| view.open), None);
    }

    /// The keys the handlers above are hung on.
    ///
    /// Without these four lines every one of them is dead code, which is what
    /// they were: `menu::step` was written, tested, and reachable from nothing.
    #[test]
    fn a_select_is_bound_to_the_keys_it_advertises() {
        let bindings = crate::window::key_bindings();
        for key in ["up", "down", "enter", "space", "home", "end"] {
            assert!(
                bindings.iter().any(|binding| {
                    binding
                        .keystrokes()
                        .iter()
                        .any(|stroke| stroke.inner().unparse() == key)
                        && binding.predicate().is_some_and(|predicate| {
                            predicate.to_string().contains("RapidCapSelect")
                        })
                }),
                "{key} does nothing on a select"
            );
        }
    }

    #[gpui::test]
    fn an_open_menu_paints(cx: &mut gpui::TestAppContext) {
        // The open menu is the one branch a switch-and-assert test never
        // reaches, and it is where the crashes live: `deferred`, `anchored`,
        // and a surface that has to carry a different `ElementId` from the pill
        // it hangs off, because the two are siblings.
        let controller = cx.new(|_| {
            AppController::new(
                Settings::default(),
                rapidcap_capture::AppPaths::from_roots("C:/Documents", "C:/Roaming", "C:/Local"),
            )
        });
        let window = cx.update(|cx| {
            let controller = controller.clone();
            cx.open_window(Default::default(), |cx_window, cx| {
                let _ = cx_window;
                cx.new(|cx| SettingsView::new(cx, controller))
            })
            .unwrap()
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut cx).unwrap();

        view.update(&mut cx, |view, cx| view.toggle(Select::Quality, cx));
        // The real draw loop rather than `VisualTestContext::draw`, which
        // paints inside an arena scope it then clears - a `deferred` child is
        // flushed after that scope closes, and reading it back panics on a
        // cleared arena. Nothing wrong with the menu; the shortcut just cannot
        // paint one.
        cx.run_until_parked();
    }

    #[test]
    fn a_folder_shows_the_end_of_its_path() {
        assert_eq!(
            folder_tail(r"C:\Users\inspire\Documents\RapidCap"),
            r"…\Documents\RapidCap"
        );
        // Nothing to trim: shown whole rather than given a leading ellipsis
        // that hides nothing.
        assert_eq!(folder_tail(r"D:\Captures"), r"D:\Captures");
    }
}
