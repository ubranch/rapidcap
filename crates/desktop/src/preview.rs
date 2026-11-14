//! The capture preview window.
//!
//! Opening a capture used to hand it to whatever app the system had for the
//! extension - Photos, a video player - which put the file in a window that
//! knows nothing about RapidCap: no Copy, no Delete to the Recycle Bin, and a
//! different app to close every time. This window shows the capture itself and
//! keeps the gallery's actions one click away, with the system app still on
//! the toolbar for anything it does better.
//!
//! There is one preview window, reused. Opening a second capture swaps the
//! content of the one already up and brings it to the front, so a run of
//! clicks through the gallery does not leave a stack of windows behind it.
//!
//! GPUI has no video player, so a video shows its first frame, full size, with
//! a play button that hands the file to the system app. GIFs animate: GPUI's
//! `img` decodes every frame and loops them, as long as the element has an id
//! to keep its frame counter in.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    Global, KeyBinding, ObjectFit, Render, Role, StyledImage as _, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, div, img, prelude::*, px, size,
};

use crate::gallery::{Kind, copy_capture, reveal_capture, tile_action};
use crate::i18n::t;
use crate::icons::Icon;
use crate::platform::{move_to_trash, open_path};
use crate::theme;
use crate::thumbs;

actions!(rapidcap_preview, [ClosePreview]);

/// The window's first size, in logical pixels. Big enough that a screenshot is
/// worth looking at, small enough that it never covers a laptop screen.
const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 640.0;

/// The play button over a video's poster.
const PLAY: f32 = 64.0;
const PLAY_GLYPH: f32 = 28.0;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "escape",
        ClosePreview,
        Some("RapidCapPreview"),
    )]
}

/// What the gallery needs to hear from the preview.
pub enum PreviewEvent {
    /// The capture is in the Recycle Bin and the window is closing.
    Deleted(PathBuf),
}

/// The one preview window, if it has been opened. A handle to a window that
/// has since closed is not cleared: updating it fails, and that is the signal
/// to open a fresh one.
#[derive(Default)]
struct Slot(Option<WindowHandle<PreviewView>>);

impl Global for Slot {}

/// Show `path` in the preview window, opening it if none is up.
pub fn open_preview(
    cx: &mut App,
    path: &Path,
    thumb_dir: &Path,
) -> anyhow::Result<Entity<PreviewView>> {
    if let Some(handle) = cx.try_global::<Slot>().and_then(|slot| slot.0)
        && let Ok(view) = handle.update(cx, |view, window, cx| {
            view.show(path, window, cx);
            window.activate_window();
            cx.entity()
        })
    {
        return Ok(view);
    }
    let bounds = Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
    let path = path.to_path_buf();
    let thumb_dir = thumb_dir.to_path_buf();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            focus: true,
            show: true,
            app_id: Some("com.inspire.rapidcap.preview".into()),
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let mut view = PreviewView {
                    path: PathBuf::new(),
                    kind: None,
                    thumb: None,
                    thumb_dir,
                    generation: 0,
                    focus_handle: cx.focus_handle(),
                };
                view.show(&path, window, cx);
                view
            })
        },
    )?;
    cx.set_global(Slot(Some(handle)));
    handle.update(cx, |_, _, cx| cx.entity())
}

pub struct PreviewView {
    path: PathBuf,
    kind: Option<Kind>,
    /// A video's full-size first frame, once FFmpeg has written it.
    thumb: Option<PathBuf>,
    thumb_dir: PathBuf,
    /// Bumped by every [`Self::show`], so a thumbnail for a video that has
    /// since been replaced in the window does not land on the new one.
    generation: u64,
    focus_handle: FocusHandle,
}

impl EventEmitter<PreviewEvent> for PreviewView {}

impl PreviewView {
    fn show(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.path = path.to_path_buf();
        self.kind = Kind::of(path);
        self.thumb = None;
        self.generation = self.generation.wrapping_add(1);
        window.set_window_title(&file_name(path));
        self.focus_handle.focus(window, cx);
        cx.notify();
        if self.kind != Some(Kind::Mp4) {
            return;
        }
        // Not the gallery's thumbnail: that is a 144px crop and turns to mush
        // stretched over the window. A re-open of the same file is one stat.
        let generation = self.generation;
        let source = self.path.clone();
        let dir = self.thumb_dir.clone();
        cx.spawn(async move |this, cx| {
            let thumb = cx
                .background_spawn(async move {
                    let modified = std::fs::metadata(&source).ok()?.modified().ok()?;
                    thumbs::poster(&source, modified, &dir)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.thumb = thumb;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_external(&self) {
        if let Err(error) = open_path(&self.path) {
            tracing::error!(%error, path = %self.path.display(), "open capture");
        }
    }

    /// Recycle Bin, then close. A failed delete leaves the window up on the
    /// file that is still there, which is the only honest answer.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = move_to_trash(&self.path) {
            tracing::error!(%error, path = %self.path.display(), "delete capture");
            return;
        }
        cx.emit(PreviewEvent::Deleted(self.path.clone()));
        window.remove_window();
    }

    fn close(&mut self, _: &ClosePreview, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.kind != Some(Kind::Mp4) {
            // The id is what lets a GIF animate: GPUI keeps the frame counter
            // in element state, and an element without an id has none.
            return img(self.path.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .id("rapidcap-preview-image")
                .into_any_element();
        }
        div()
            .size_full()
            .relative()
            .children(self.thumb.clone().map(|thumb| {
                img(thumb)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Contain)
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id("preview-play")
                            .accessibility_id("rapidcap.preview.play")
                            .role(Role::Button)
                            .aria_label(t("Play in default app"))
                            .size(theme::u(PLAY))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::u(theme::RADIUS_PILL))
                            .bg(theme::thumb_action())
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::thumb_action_hover()))
                            .child(Icon::Play.element(theme::u(PLAY_GLYPH), theme::text_primary()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, _| {
                                this.open_external();
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for PreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("RapidCapPreview")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_body())
            .text_color(theme::text_primary())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme::u(theme::GAP))
                    .px(theme::u(theme::PAD))
                    .py(theme::u(8.0))
                    .border_b_1()
                    .border_color(theme::border_card())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(theme::u(theme::TEXT_SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(file_name(&self.path)),
                    )
                    .child(
                        tile_action(("preview-copy", 0), t("Copy"), Icon::Copy, false).on_click(
                            cx.listener(|this, _: &ClickEvent, _, _| copy_capture(&this.path)),
                        ),
                    )
                    .child(
                        tile_action(
                            ("preview-reveal", 0),
                            t("Show in folder"),
                            Icon::Folder,
                            false,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, _| reveal_capture(&this.path)),
                        ),
                    )
                    .child(
                        tile_action(
                            ("preview-open", 0),
                            t("Open with default app"),
                            Icon::Open,
                            false,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.open_external())),
                    )
                    .child(
                        tile_action(("preview-delete", 0), t("Delete"), Icon::Delete, true)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.delete(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p(theme::u(theme::PAD))
                    .child(self.content(cx)),
            )
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    #[gpui::test]
    fn one_preview_is_reused_and_escape_closes_it(cx: &mut TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.png");
        let second = dir.path().join("b.png");
        std::fs::write(&first, b"").unwrap();
        std::fs::write(&second, b"").unwrap();
        let thumbs = dir.path().join("thumbs");

        let a = cx.update(|cx| open_preview(cx, &first, &thumbs)).unwrap();
        let b = cx.update(|cx| open_preview(cx, &second, &thumbs)).unwrap();
        cx.run_until_parked();
        assert_eq!(a.entity_id(), b.entity_id(), "same view, not a second one");
        assert_eq!(cx.windows().len(), 1, "one window");
        assert_eq!(b.read_with(cx, |view, _| view.path.clone()), second);

        let window = cx.windows()[0];
        let mut visual = VisualTestContext::from_window(window, cx);
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(cx.windows().is_empty(), "escape closed the preview");

        // A closed preview is not reused: the next open makes a new window.
        cx.update(|cx| open_preview(cx, &first, &thumbs)).unwrap();
        assert_eq!(cx.windows().len(), 1);
    }
}
