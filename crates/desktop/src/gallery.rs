//! Recent captures.
//!
//! A view of a folder, not a database. It reads `AppPaths::capture_root`,
//! sorts by modified time and draws the first [`RECENT`] entries whose
//! extension it recognises. Nothing is indexed, nothing needs migrating, and a
//! file dropped into that folder by hand shows up like any other - which is
//! what a capture tool should do.
//!
//! Thumbnails arrive after the listing does, not with it: a first look at a
//! folder of sixty captures has to decode sixty images and run `ffmpeg` over
//! every clip, and the grid is drawn and scrollable throughout. Until each one
//! lands its tile paints the placeholder tone the design specifies, which is
//! also what a tile whose file cannot be read keeps forever - see [`crate::thumbs`].
//!
//! Delete goes to the Recycle Bin, and that is what makes a single click with
//! no confirmation the right shape: the undo is the shell's, it is one keypress
//! away, and a modal in front of every delete is a modal in front of the one
//! action this screen exists to make quick.
//!
//! Copy, Show in folder and Delete are mouse-only. They appear on hover, which
//! is what the design asks for, and the alternative - four tab stops per tile
//! across thirty tiles - would be a worse screen for the keyboard than the one
//! that sends it to the folder. The tile itself is a tab stop and opens the
//! file, and Delete removes the focused one.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike as _, Local};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, FocusHandle, FontWeight, IntoElement, ObjectFit,
    ParentElement as _, Render, Role, ScrollHandle, SharedString, StyledImage as _, Window, div,
    img, linear_color_stop, linear_gradient, prelude::*,
};
use rapidcap_capture::{CaptureCommand, CaptureEvent, write_clipboard_file};

use crate::controller::AppController;
use crate::icons::Icon;
use crate::platform::{hotkey_specs, move_to_trash, open_path, reveal_path, shortcut_label};
use crate::theme;
use crate::thumbs::{self, Thumb};
use crate::window::middle_trim;

/// How many tiles the grid will draw.
///
/// Twenty rows of three. Past that this would need to recycle its rows, and a
/// screen called Recent captures that reaches back two hundred captures is not
/// the screen anyone wanted - the folder is one click away in the titlebar.
const RECENT: usize = 60;

const COLUMNS: usize = 3;

/// Filter chips. Same height as a select in settings, and the same pill.
const CHIP_H: f32 = 30.0;
const CHIP_PX: f32 = 12.0;
const CHIP_GAP: f32 = 6.0;

const THUMB_H: f32 = 72.0;
/// A tile, edge to edge, including its border.
///
/// Pinned rather than left to the two text lines, because the window's height
/// is three of these plus the chrome: if a font substitution grew the name line
/// by two pixels, the third row would hang six pixels below the bottom edge of
/// a window that cannot resize. Each line gets an explicit line height for the
/// same reason, and the two agree - see [`crate::theme::GALLERY_H`].
pub const TILE_H: f32 = 118.0;
/// The name line, and the timestamp under it. 11px and 10px text at the ratio a
/// browser would have given the mockup.
const NAME_LINE: f32 = 13.0;
const WHEN_LINE: f32 = 12.0;
/// The gradient the design specifies for an unloaded tile, in degrees.
const THUMB_ANGLE: f32 = 140.0;
/// Tile radius, and the inset of the two badges from the thumbnail's corners.
const BADGE_INSET: f32 = 6.0;
const BADGE_H: f32 = 18.0;
const BADGE_PX: f32 = 6.0;
const META_PT: f32 = 7.0;
const META_PX: f32 = 8.0;
const META_PB: f32 = 8.0;
const META_GAP: f32 = 4.0;

const ACTION: f32 = 28.0;
const ACTION_GLYPH: f32 = 14.0;
const ACTION_GAP: f32 = 5.0;

/// Characters a tile name can show. A tile is 119px wide, less its border and
/// 8px of padding on each side, and 11px regular runs a shade under six pixels
/// a character.
const NAME_MAX: usize = 18;
/// What a trim keeps of the tail: `a7Kq.png`, the extension plus the guard.
const NAME_TAIL: usize = 8;

/// The empty state's ring, and the inset of the hotkey rows inside it.
const RING: f32 = 44.0;
const KEYS_PX: f32 = 20.0;
const KBD: &str = "Cascadia Mono";

/// What the gallery knows how to show.
///
/// The extension is the whole of it, which is what makes the filters free: the
/// screenshot pipeline switches PNG to JPEG above its threshold, so both
/// extensions land under Images without either one being recorded anywhere.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Png,
    Jpg,
    Mp4,
    Gif,
}

impl Kind {
    fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpg),
            "mp4" => Some(Self::Mp4),
            "gif" => Some(Self::Gif),
            _ => None,
        }
    }

    /// The badge in the thumbnail's corner. Upper case in the source rather
    /// than uppercased at the call site: it is a file format, not a label.
    fn badge(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpg => "JPG",
            Self::Mp4 => "MP4",
            Self::Gif => "GIF",
        }
    }
}

/// The four chips above the grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Filter {
    All,
    Images,
    Video,
    Gif,
}

impl Filter {
    const ALL: [Self; 4] = [Self::All, Self::Images, Self::Video, Self::Gif];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Images => "Images",
            Self::Video => "Video",
            Self::Gif => "GIF",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::All => "filter-all",
            Self::Images => "filter-images",
            Self::Video => "filter-video",
            Self::Gif => "filter-gif",
        }
    }

    fn accepts(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Images => matches!(kind, Kind::Png | Kind::Jpg),
            Self::Video => kind == Kind::Mp4,
            Self::Gif => kind == Kind::Gif,
        }
    }
}

/// One file in the folder.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Capture {
    path: PathBuf,
    name: SharedString,
    kind: Kind,
    modified: SystemTime,
    /// `None` while the cache is still working through the folder, and forever
    /// for a file that could not be read.
    thumb: Option<Thumb>,
}

pub struct GalleryView {
    controller: Entity<AppController>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    filter: Filter,
    captures: Vec<Capture>,
    /// Which tile the pointer is over, and therefore which one is showing its
    /// three actions.
    ///
    /// State rather than `group_hover` because the actions have to be gone, not
    /// transparent: an invisible Delete sitting over a thumbnail that opens on
    /// click is the one bug on this screen that cannot be undone by the person
    /// who hit it.
    hovered: Option<usize>,
    /// False until the first listing lands. The empty state is a claim about
    /// the folder, and claiming it before anything has been read would flash
    /// "No captures yet" over a folder with two hundred files in it.
    loaded: bool,
    /// Bumped by every [`Self::reload`], so the thumbnails of a listing that
    /// has already been replaced land on nothing instead of on the new one.
    generation: u64,
    _controller: gpui::Subscription,
    _saved: gpui::Subscription,
}

impl GalleryView {
    pub fn new(cx: &mut Context<Self>, controller: Entity<AppController>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            filter: Filter::All,
            captures: Vec::new(),
            hovered: None,
            loaded: false,
            generation: 0,
            _controller: cx.observe(&controller, |_, _, cx| cx.notify()),
            // A capture that lands while the grid is up belongs at the front of
            // it. Guarded on `loaded`, so a folder nobody has opened the
            // gallery on is not re-read once per screenshot.
            _saved: cx.subscribe(&controller, |this, _, event: &CaptureEvent, cx| {
                if this.loaded && matches!(event, CaptureEvent::OutputSaved(_)) {
                    this.reload(cx);
                }
            }),
            controller,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Re-read the folder, then fill in its thumbnails.
    ///
    /// Off the main thread, every time. A `read_dir` over a capture folder is
    /// microseconds on a local disk and a stall on a mapped drive, and the
    /// difference is not something this screen gets to find out on the frame it
    /// is drawing.
    ///
    /// The listing is published on its own, before a single thumbnail has been
    /// read, and each thumbnail is published as it arrives. A first look at a
    /// folder of sixty clips is a minute of `ffmpeg`; the grid is drawn,
    /// scrollable, and clickable for all of it.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let paths = self.controller.read(cx).paths().clone();
        let root = paths.capture_root.clone();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let captures = cx
                .background_spawn(async move { read_folder(&root, RECENT) })
                .await;
            let wanted: Vec<(PathBuf, SystemTime, Kind)> = captures
                .iter()
                .map(|capture| (capture.path.clone(), capture.modified, capture.kind))
                .collect();
            let live: Vec<u64> = wanted
                .iter()
                .map(|(path, modified, _)| thumbs::key(path, *modified))
                .collect();
            this.update(cx, |this, cx| {
                this.captures = captures;
                this.loaded = true;
                cx.notify();
            })?;

            // The cache is handed to the background and handed back, one
            // capture at a time, so that the main thread gets a turn between
            // any two of them.
            let dir = paths.thumb_dir.clone();
            let mut cache = cx
                .background_spawn(async move { thumbs::Cache::open(&dir) })
                .await;
            for (path, modified, kind) in wanted {
                let (thumb, path, returned) = cx
                    .background_spawn(async move {
                        let thumb = cache.thumb(&path, modified, kind);
                        (thumb, path, cache)
                    })
                    .await;
                cache = returned;
                let Some(thumb) = thumb else { continue };
                this.update(cx, |this, cx| this.fill(generation, &path, thumb, cx))?;
            }

            // Whatever is left over belongs to a capture that has been deleted,
            // renamed, or written over since the last time the gallery ran.
            cx.background_spawn(async move { cache.sweep(&live) }).await;
            anyhow::Ok(())
        })
        .detach();
    }

    /// Hang a finished thumbnail on its tile.
    ///
    /// By path rather than by index: a delete during the pass shifts every
    /// index behind it, and hanging a thumbnail on the wrong tile is worse
    /// than hanging it on none.
    fn fill(&mut self, generation: u64, path: &Path, thumb: Thumb, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        let Some(capture) = self.captures.iter_mut().find(|c| c.path == path) else {
            return;
        };
        capture.thumb = Some(thumb);
        cx.notify();
    }

    fn choose(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        // The tile under the pointer is a different tile now, or no tile at
        // all. Leaving the index behind would open the actions on whatever
        // happens to land in that slot.
        self.hovered = None;
        self.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    fn hover(&mut self, index: usize, hovered: bool, cx: &mut Context<Self>) {
        // Leaving one tile and entering the next arrive in that order, so a
        // leave that names a tile we are no longer on has already been
        // superseded and must not clear the new one.
        let next = if hovered {
            Some(index)
        } else if self.hovered == Some(index) {
            None
        } else {
            return;
        };
        if self.hovered != next {
            self.hovered = next;
            cx.notify();
        }
    }

    fn open(&self, path: &Path) {
        if let Err(error) = open_path(path) {
            tracing::error!(%error, path = %path.display(), "open capture");
        }
    }

    /// Put the file itself on the clipboard, not its path: a capture is pasted
    /// into a chat window far more often than into a shell.
    fn copy(&self, path: &Path) {
        if let Err(error) = write_clipboard_file(path) {
            tracing::error!(%error, path = %path.display(), "copy capture");
        }
    }

    fn reveal(&self, path: &Path) {
        if let Err(error) = reveal_path(path) {
            tracing::error!(%error, path = %path.display(), "reveal capture");
        }
    }

    /// Delete to the Recycle Bin, and drop the tile.
    ///
    /// The tile goes without waiting for a re-listing: the grid reflows under
    /// the pointer either way, and doing it on the next frame rather than after
    /// a round trip to the disk is the difference between a delete and a delay.
    /// A failed delete keeps its tile, which is the only honest answer.
    fn delete(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(capture) = self.captures.get(index) else {
            return;
        };
        if let Err(error) = move_to_trash(&capture.path) {
            tracing::error!(%error, path = %capture.path.display(), "delete capture");
            return;
        }
        self.captures.remove(index);
        self.hovered = None;
        cx.notify();
    }
}

/// Everything in `root` the gallery knows how to draw, newest first.
///
/// A file that cannot be stat'd is dropped rather than sorted to one end: it is
/// mid-write, or gone between the listing and the stat, and either way it has
/// no place in a list ordered by when it was written.
fn read_folder(root: &Path, limit: usize) -> Vec<Capture> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut captures: Vec<Capture> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let kind = Kind::of(&path)?;
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some(Capture {
                name: SharedString::from(path.file_name()?.to_string_lossy().into_owned()),
                modified: metadata.modified().ok()?,
                thumb: None,
                kind,
                path,
            })
        })
        .collect();
    captures.sort_by_key(|capture| std::cmp::Reverse(capture.modified));
    captures.truncate(limit);
    captures
}

/// How long ago a capture landed, in the units someone reads at a glance.
///
/// The switch from hours to a date is the calendar day, not 24 elapsed hours: a
/// capture taken at 23:50 reads "Yesterday" at 00:10, which is what it is, and
/// "0 h ago" is what a duration would have called it.
///
/// A file dated in the future - a clock correction, or a copy off a machine
/// running fast - reads "Just now" rather than a negative count.
fn when_label(modified: SystemTime, now: DateTime<Local>) -> String {
    let at: DateTime<Local> = modified.into();
    let elapsed = now.signed_duration_since(at);
    if elapsed.num_minutes() < 1 {
        return "Just now".to_string();
    }
    if elapsed.num_minutes() < 60 {
        return format!("{} min ago", elapsed.num_minutes());
    }
    let days = now
        .date_naive()
        .signed_duration_since(at.date_naive())
        .num_days();
    match days {
        0 => format!("{} h ago", elapsed.num_hours()),
        1 => "Yesterday".to_string(),
        _ if at.year() == now.year() => at.format("%-d %b").to_string(),
        _ => at.format("%-d %b %Y").to_string(),
    }
}

impl Render for GalleryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Local::now();
        let filter = self.filter;
        let visible: Vec<(usize, &Capture)> = self
            .captures
            .iter()
            .enumerate()
            .filter(|(_, capture)| filter.accepts(capture.kind))
            .collect();
        let empty = self.loaded && visible.is_empty();

        div()
            .id("gallery")
            .accessibility_id("rapidcap.gallery")
            .role(Role::Group)
            .aria_label("Recent captures")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(theme::u(theme::GAP))
            .p(theme::u(theme::PAD))
            .child(div().flex().flex_none().gap(theme::u(CHIP_GAP)).children(
                Filter::ALL.into_iter().map(|choice| {
                    filter_chip(choice, choice == filter)
                        .on_click(cx.listener(move |this, _, _, cx| this.choose(choice, cx)))
                }),
            ))
            .children(empty.then(|| self.empty_state(cx)))
            .children((!empty).then(|| {
                div()
                    .id("gallery-grid")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(theme::u(theme::GAP))
                    .children(visible.chunks(COLUMNS).map(|row| {
                        div()
                            .flex()
                            .flex_none()
                            .gap(theme::u(theme::GAP))
                            .children(
                                row.iter()
                                    .map(|(index, capture)| self.tile(*index, capture, now, cx)),
                            )
                            // A last row of one must not draw one tile three
                            // columns wide. The spacers hold the other two.
                            .children((row.len()..COLUMNS).map(|_| div().flex_1().min_w_0().h_0()))
                    }))
            }))
    }
}

impl GalleryView {
    fn tile(
        &self,
        index: usize,
        capture: &Capture,
        now: DateTime<Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = capture.path.clone();
        let name = capture.name.clone();
        let hovered = self.hovered == Some(index);
        div()
            .id(("tile", index))
            .accessibility_id(format!("rapidcap.gallery.{index}"))
            .key_context("RapidCapTile")
            .focusable()
            .tab_stop(true)
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_keyshortcuts("Enter Space")
            .relative()
            .flex_1()
            .min_w_0()
            .h(theme::u(TILE_H))
            .overflow_hidden()
            .rounded(theme::u(theme::RADIUS))
            .bg(theme::bg_card())
            .border_1()
            .border_color(theme::border_card())
            .cursor_pointer()
            .focus_visible(|style| style.border_color(theme::accent()))
            .on_hover(
                cx.listener(move |this, hovered: &bool, _, cx| this.hover(index, *hovered, cx)),
            )
            .on_click({
                let path = path.clone();
                cx.listener(move |this, _, _, _| this.open(&path))
            })
            .child(
                div()
                    .relative()
                    .h(theme::u(THUMB_H))
                    // The gradient is the background, not a fallback: it is
                    // what a tile shows while its thumbnail is still being
                    // made, and what it keeps if the file cannot be read.
                    .bg(linear_gradient(
                        THUMB_ANGLE,
                        linear_color_stop(theme::thumb_from(), 0.0),
                        linear_color_stop(theme::thumb_to(), 1.0),
                    ))
                    .children(capture.thumb.as_ref().map(|thumb| {
                        img(thumb.image.clone())
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                    }))
                    .children(capture.thumb.as_ref().and_then(|thumb| thumb.duration).map(
                        |duration| {
                            div()
                                .absolute()
                                .right(theme::u(BADGE_INSET))
                                .bottom(theme::u(BADGE_INSET))
                                .h(theme::u(BADGE_H))
                                .px(theme::u(BADGE_PX))
                                .flex()
                                .items_center()
                                .rounded(theme::u(theme::RADIUS_PILL))
                                .bg(theme::thumb_badge())
                                .text_size(theme::u(10.0))
                                .text_color(theme::text_primary())
                                .child(thumbs::duration_label(duration))
                        },
                    ))
                    .child(
                        div()
                            .absolute()
                            .left(theme::u(BADGE_INSET))
                            .top(theme::u(BADGE_INSET))
                            .h(theme::u(BADGE_H))
                            .px(theme::u(BADGE_PX))
                            .flex()
                            .items_center()
                            .rounded(theme::u(theme::RADIUS_PILL))
                            .bg(theme::thumb_badge())
                            .text_size(theme::u(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_primary())
                            .child(capture.kind.badge()),
                    )
                    .children(hovered.then(|| self.actions(index, &path, cx))),
            )
            .child(
                div()
                    .pt(theme::u(META_PT))
                    .px(theme::u(META_PX))
                    .pb(theme::u(META_PB))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_size(theme::u(theme::TEXT_MICRO))
                            .line_height(theme::u(NAME_LINE))
                            .text_color(theme::text_label())
                            .child(middle_trim(&name, NAME_MAX, NAME_TAIL)),
                    )
                    .child(
                        div()
                            .mt(theme::u(META_GAP))
                            .text_size(theme::u(theme::TEXT_TINY))
                            .line_height(theme::u(WHEN_LINE))
                            .text_color(theme::text_muted())
                            .child(when_label(capture.modified, now)),
                    ),
            )
            .into_any_element()
    }

    /// Copy, Show in folder and Delete, over the thumbnail.
    fn actions(&self, index: usize, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let copy = path.to_path_buf();
        let reveal = path.to_path_buf();
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(theme::u(THUMB_H))
            .flex()
            .items_center()
            .justify_center()
            .gap(theme::u(ACTION_GAP))
            .bg(theme::thumb_scrim())
            .child(
                tile_action(("tile-copy", index), "Copy", Icon::Copy, false).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.copy(&copy);
                    },
                )),
            )
            .child(
                tile_action(
                    ("tile-reveal", index),
                    "Show in folder",
                    Icon::Folder,
                    false,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.reveal(&reveal);
                })),
            )
            .child(
                tile_action(("tile-delete", index), "Delete", Icon::Delete, true).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.delete(index, cx);
                    }),
                ),
            )
            .into_any_element()
    }

    /// What the folder says before anything has been captured into it.
    ///
    /// Recessed, because it is a container with nothing in it rather than a
    /// card that can be pressed, and it spends its space teaching the three
    /// hotkeys rather than apologising for being empty.
    fn empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let folder = self
            .controller
            .read(cx)
            .paths()
            .capture_root
            .display()
            .to_string();
        // The three the design lists, in its order: the two screenshots and the
        // video. Straight off the same table the panel reads, so a chord the
        // machine could not register is not taught here as if it worked.
        let taught = [
            CaptureCommand::CaptureRegion,
            CaptureCommand::CaptureActiveWindow,
            CaptureCommand::ToggleVideo,
        ];
        div()
            .flex_none()
            .px(theme::u(18.0))
            .pt(theme::u(26.0))
            .pb(theme::u(22.0))
            .rounded(theme::u(theme::RADIUS))
            .bg(theme::bg_well())
            .shadow(theme::recessed())
            .child(
                div()
                    .size(theme::u(RING))
                    .mx_auto()
                    .mb(theme::u(12.0))
                    .rounded(theme::u(theme::RADIUS_PILL))
                    .border_3()
                    .border_color(theme::bg_pill_off()),
            )
            .child(
                div()
                    .mb(theme::u(6.0))
                    .text_center()
                    .text_size(theme::u(theme::TEXT_BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_label())
                    .child("No captures yet"),
            )
            .child(
                div()
                    .max_w(theme::u(250.0))
                    .mx_auto()
                    .text_center()
                    .text_size(theme::u(theme::TEXT_MICRO))
                    .line_height(theme::u(theme::TEXT_MICRO * 1.6))
                    .text_color(theme::text_muted())
                    .child(format!(
                        "Everything you capture lands in {folder} and shows up here."
                    )),
            )
            .child(
                div()
                    .mt(theme::u(14.0))
                    .flex()
                    .flex_col()
                    .gap(theme::u(4.0))
                    .children(taught.into_iter().filter_map(|command| {
                        let binding = shortcut_label(command)?;
                        let label = hotkey_specs()
                            .into_iter()
                            .find(|spec| spec.command() == command)?
                            .label();
                        Some(key_row(label, binding))
                    })),
            )
            .into_any_element()
    }
}

fn filter_chip(filter: Filter, on: bool) -> gpui::Stateful<gpui::Div> {
    let mut chip = div()
        .id(filter.id())
        .accessibility_id(format!("rapidcap.{}", filter.id()))
        .key_context("RapidCapFilter")
        .focusable()
        .tab_stop(true)
        .role(Role::Tab)
        .aria_label(filter.label())
        .aria_selected(on)
        .aria_keyshortcuts("Enter Space")
        .h(theme::u(CHIP_H))
        .px(theme::u(CHIP_PX))
        .flex()
        .flex_none()
        .items_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .border_1()
        .text_size(theme::u(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .focus_visible(|style| style.border_color(theme::accent()));
    // One `hover` call per element: GPUI panics on a second, so the on state
    // picks its own rather than layering a second rule over the off one.
    if on {
        chip = chip
            .bg(theme::accent_fill())
            .border_color(theme::accent_edge())
            .text_color(theme::accent_text())
            .hover(|style| style.bg(theme::accent_pill()));
    } else {
        chip = chip
            .bg(theme::bg_card())
            .border_color(theme::border_card())
            .text_color(theme::text_pill())
            .hover(|style| style.bg(theme::bg_hover()));
    }
    chip.child(filter.label())
}

fn tile_action(
    id: (&'static str, usize),
    label: &'static str,
    icon: Icon,
    danger: bool,
) -> gpui::Stateful<gpui::Div> {
    let hover_bg = if danger {
        theme::danger()
    } else {
        theme::thumb_action_hover()
    };
    div()
        .id(id)
        .accessibility_id(format!("rapidcap.{}.{}", id.0, id.1))
        .role(Role::Button)
        .aria_label(label)
        .size(theme::u(ACTION))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::u(theme::RADIUS_PILL))
        .bg(theme::thumb_action())
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .child(icon.element(theme::u(ACTION_GLYPH), theme::text_label()))
}

/// One `Capture region — Alt + E` line in the empty state.
fn key_row(label: String, binding: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(theme::u(10.0))
        .px(theme::u(KEYS_PX))
        .text_size(theme::u(theme::TEXT_MICRO))
        .text_color(theme::text_muted())
        .child(div().child(label))
        .child(
            div()
                .px(theme::u(6.0))
                .py(theme::u(3.0))
                .rounded(theme::u(4.0))
                .bg(theme::bg_hover())
                .font_family(KBD)
                .text_size(theme::u(theme::TEXT_TINY))
                .text_color(theme::text_label())
                .child(binding),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use std::time::Duration;

    fn write(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn a_folder_lists_newest_first_and_ignores_what_it_cannot_show() {
        let root = tempfile::tempdir().unwrap();
        // Written oldest to newest, so the listing has to reverse them rather
        // than happening to agree with the directory order.
        for name in [
            "old.png",
            "notes.txt",
            "clip.mp4",
            "loop.gif",
            "shot.jpeg",
            "new.png",
        ] {
            write(root.path(), name);
            // Windows file times are coarse enough that six writes in a row can
            // share one timestamp, which would make the order arbitrary.
            std::thread::sleep(Duration::from_millis(12));
        }
        let captures = read_folder(root.path(), RECENT);
        let names: Vec<&str> = captures
            .iter()
            .map(|capture| capture.name.as_ref())
            .collect();
        assert_eq!(
            names,
            ["new.png", "shot.jpeg", "loop.gif", "clip.mp4", "old.png"],
            "notes.txt has no kind, so it is not a capture"
        );
        assert_eq!(
            captures[1].kind,
            Kind::Jpg,
            ".jpeg is a JPG, not a stranger"
        );
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_empty_gallery() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("nope");
        assert!(read_folder(&missing, RECENT).is_empty());
    }

    #[test]
    fn the_grid_never_draws_more_than_it_promised() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..8 {
            write(root.path(), &format!("shot-{index}.png"));
        }
        assert_eq!(read_folder(root.path(), 3).len(), 3);
    }

    /// A gallery over a real folder, in a real window.
    ///
    /// Returns the folder the gallery will actually read, which is a pair of
    /// subdirectories under the documents root rather than the root itself.
    fn open(
        cx: &mut gpui::TestAppContext,
        documents: &Path,
    ) -> (gpui::VisualTestContext, Entity<GalleryView>, PathBuf) {
        // Every root is the tempdir, including the one the thumbnail cache
        // hangs off: a test that writes outside the directory it cleans up is
        // a test that litters the machine it runs on.
        let paths = rapidcap_capture::AppPaths::from_roots(documents, documents, documents);
        let root = paths.capture_root.clone();
        std::fs::create_dir_all(&root).unwrap();
        let controller =
            cx.new(|_| AppController::new(rapidcap_capture::Settings::default(), paths));
        let window = cx.update(|cx| {
            let controller = controller.clone();
            cx.open_window(Default::default(), |cx_window, cx| {
                let _ = cx_window;
                cx.new(|cx| GalleryView::new(cx, controller))
            })
            .unwrap()
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let view = window.root(&mut visual).unwrap();
        (visual, view, root)
    }

    #[gpui::test]
    fn a_folder_of_captures_paints_its_grid(cx: &mut gpui::TestAppContext) {
        let documents = tempfile::tempdir().unwrap();
        let (mut cx, view, root) = open(cx, documents.path());
        // Four, so the last row is short and has to be padded rather than
        // stretched - and one of each kind, so every badge and every filter is
        // exercised by the same draw.
        for name in ["a.png", "b.jpg", "c.mp4", "d.gif"] {
            write(&root, name);
        }

        view.update(&mut cx, |view, cx| view.reload(cx));
        cx.run_until_parked();
        assert_eq!(view.read_with(&cx, |view, _| view.captures.len()), 4);

        // Hovering paints the scrim and its three buttons over the thumbnail,
        // which is the branch that carries every id built from an index.
        view.update(&mut cx, |view, cx| view.hover(0, true, cx));
        cx.run_until_parked();
        assert_eq!(view.read_with(&cx, |view, _| view.hovered), Some(0));

        // Moving to the next tile: the enter arrives before the leave it
        // replaced, so the stale leave has to be dropped rather than obeyed or
        // the new tile loses the actions it just gained.
        view.update(&mut cx, |view, cx| {
            view.hover(1, true, cx);
            view.hover(0, false, cx);
        });
        cx.run_until_parked();
        assert_eq!(view.read_with(&cx, |view, _| view.hovered), Some(1));

        view.update(&mut cx, |view, cx| view.hover(1, false, cx));
        assert_eq!(view.read_with(&cx, |view, _| view.hovered), None);

        // And a filter change drops it: the tile in that slot is a different
        // file now, or gone.
        view.update(&mut cx, |view, cx| {
            view.hover(0, true, cx);
            view.choose(Filter::Video, cx);
        });
        assert_eq!(view.read_with(&cx, |view, _| view.hovered), None);

        // Images takes both extensions the screenshot pipeline writes; the
        // other two chips take exactly one kind each.
        for (filter, expected) in [
            (Filter::All, 4),
            (Filter::Images, 2),
            (Filter::Video, 1),
            (Filter::Gif, 1),
        ] {
            view.update(&mut cx, |view, cx| view.choose(filter, cx));
            cx.run_until_parked();
            let shown = view.read_with(&cx, |view, _| {
                view.captures
                    .iter()
                    .filter(|capture| view.filter.accepts(capture.kind))
                    .count()
            });
            assert_eq!(shown, expected, "{filter:?}");
        }
    }

    #[gpui::test]
    async fn a_readable_capture_gets_a_thumbnail_and_a_stale_one_is_dropped(
        cx: &mut gpui::TestAppContext,
    ) {
        let documents = tempfile::tempdir().unwrap();
        let (mut cx, view, root) = open(cx, documents.path());
        // A real image beside a file that only has the extension of one: the
        // first ends up with a thumbnail, the second keeps the gradient.
        let shot = root.join("shot.png");
        image::RgbImage::new(320, 240)
            .save_with_format(&shot, image::ImageFormat::Png)
            .unwrap();
        write(&root, "broken.png");

        view.update(&mut cx, |view, cx| view.reload(cx));
        cx.run_until_parked();

        let thumbs: Vec<Option<PathBuf>> = view.read_with(&cx, |view, _| {
            view.captures
                .iter()
                .map(|capture| capture.thumb.as_ref().map(|thumb| thumb.image.clone()))
                .collect()
        });
        let made: Vec<&PathBuf> = thumbs.iter().flatten().collect();
        assert_eq!(made.len(), 1, "only the readable one, {thumbs:?}");
        assert!(made[0].is_file(), "the tile points at a file that exists");

        // A thumbnail from a listing that has already been replaced belongs to
        // nothing. It has to be dropped, not hung on whichever tile now sits
        // where its own used to.
        let stale = Thumb {
            image: made[0].clone(),
            duration: None,
        };
        view.update(&mut cx, |view, cx| {
            let generation = view.generation;
            view.captures[0].thumb = None;
            view.fill(generation.wrapping_sub(1), &shot, stale, cx);
        });
        assert!(
            view.read_with(&cx, |view, _| view.captures[0].thumb.is_none()),
            "a thumbnail from the previous listing"
        );
    }

    #[gpui::test]
    fn an_empty_folder_paints_the_empty_state(cx: &mut gpui::TestAppContext) {
        let documents = tempfile::tempdir().unwrap();
        let (mut cx, view, _root) = open(cx, documents.path());

        // Before the listing lands the screen must not claim the folder is
        // empty, because it has not looked yet.
        assert!(!view.read_with(&cx, |view, _| view.loaded));
        view.update(&mut cx, |view, cx| view.reload(cx));
        cx.run_until_parked();
        assert!(view.read_with(&cx, |view, _| view.loaded));
        assert!(view.read_with(&cx, |view, _| view.captures.is_empty()));
    }

    #[test]
    fn three_rows_of_tiles_fit_the_window_exactly() {
        // The window cannot resize, so this is not a layout preference: a tile
        // one pixel taller than its parts is a third row hanging over the
        // bottom edge with nothing under it.
        let meta = META_PT + NAME_LINE + META_GAP + WHEN_LINE + META_PB;
        assert_eq!(THUMB_H + meta + 2.0, TILE_H, "thumbnail, meta, and border");

        let rows = 3.0 * TILE_H + 2.0 * theme::GAP;
        let body = theme::PAD + CHIP_H + theme::GAP + rows + theme::PAD;
        assert_eq!(
            theme::TITLEBAR_H + body,
            theme::GALLERY_H,
            "titlebar, filters, three rows"
        );
    }

    #[test]
    fn images_covers_both_extensions_the_pipeline_writes() {
        assert!(Filter::Images.accepts(Kind::Png));
        assert!(Filter::Images.accepts(Kind::Jpg));
        assert!(!Filter::Images.accepts(Kind::Gif), "GIF has its own chip");
        assert!(!Filter::Video.accepts(Kind::Gif), "and it is not video");
        for kind in [Kind::Png, Kind::Jpg, Kind::Mp4, Kind::Gif] {
            assert!(Filter::All.accepts(kind));
        }
    }

    #[test]
    fn the_day_boundary_is_a_calendar_day_not_a_duration() {
        let now = Local.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap();
        let ago = |hours: i64, minutes: i64| {
            SystemTime::from(
                now - chrono::Duration::hours(hours) - chrono::Duration::minutes(minutes),
            )
        };
        assert_eq!(when_label(ago(0, 0), now), "Just now");
        assert_eq!(when_label(ago(0, 18), now), "18 min ago");
        assert_eq!(when_label(ago(3, 0), now), "3 h ago");
        // Last night, and a person calls it yesterday. An elapsed-hours rule
        // would have said "13 h ago" here and "36 h ago" on the line below.
        assert_eq!(when_label(ago(13, 0), now), "Yesterday");
        assert_eq!(when_label(ago(36, 0), now), "Yesterday");
        assert_eq!(when_label(ago(60, 0), now), "25 Aug");
    }

    #[test]
    fn minutes_still_read_as_minutes_across_midnight() {
        // Ten past midnight, looking at a capture from a quarter to. The
        // calendar says yesterday and the clock says twenty-five minutes; the
        // clock is the one that answers "when was this".
        let now = Local.with_ymd_and_hms(2026, 8, 27, 0, 10, 0).unwrap();
        let quarter_to = SystemTime::from(now - chrono::Duration::minutes(25));
        assert_eq!(when_label(quarter_to, now), "25 min ago");
    }

    #[test]
    fn an_older_year_says_which_year() {
        let now = Local.with_ymd_and_hms(2026, 1, 4, 12, 0, 0).unwrap();
        let then = SystemTime::from(Local.with_ymd_and_hms(2025, 12, 26, 9, 0, 0).unwrap());
        assert_eq!(when_label(then, now), "26 Dec 2025");
    }

    #[test]
    fn a_clock_that_ran_backwards_does_not_print_a_negative_age() {
        let now = Local.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap();
        let ahead = SystemTime::from(now + chrono::Duration::hours(3));
        assert_eq!(when_label(ahead, now), "Just now");
    }
}
