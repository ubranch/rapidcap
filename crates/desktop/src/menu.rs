//! One menu shape for every menu in the product.
//!
//! Not a widget that owns state: the surface, the rows and the separators are
//! built here to one set of numbers, and whoever opens a menu keeps track of
//! which row is highlighted and what each row does. That is the same division
//! the rest of the panel uses - `mode_card`, `chip` and `countdown_slot` all
//! hand a `Stateful<Div>` back for the caller to attach `on_click` to - and it
//! is what keeps a menu from needing a bespoke entity per place it appears.
//!
//! The tray menu is the one exception, and always will be. It is a native
//! `tray_icon::menu::Menu` built in `platform/mod.rs`, drawn by the shell, and
//! nothing here applies to it.

// Built to the design system in one pass, adopted screen by screen - the same
// arrangement `icons` uses. Settings is the first caller and takes most of it.
#![allow(dead_code)]

use gpui::{FontWeight, Role, SharedString, div, prelude::*};

use crate::icons::Icon;
use crate::theme;

/// The default menu width. Fixed, never content-sized: a menu that grows when
/// a label is long moves the row under the pointer between openings.
pub const WIDTH: f32 = 212.0;

/// The wide variant, for menus that list device names.
pub const WIDTH_WIDE: f32 = 248.0;

const PADDING: f32 = 5.0;
const RADIUS: f32 = 10.0;
/// Item radius. Smaller than the surface it sits inside, so the hover fill
/// stays clear of the corner rather than fighting it.
const ITEM_RADIUS: f32 = 6.0;
const ITEM_H: f32 = 32.0;
const ITEM_PX: f32 = 10.0;
const ITEM_GAP: f32 = 9.0;
/// The tick column. Reserved on every row, ticked or not, so a label does not
/// shift sideways when the selection moves onto it.
const TICK_W: f32 = 15.0;
const TICK_GLYPH: f32 = 14.0;
/// What a disabled row drops to. It stays on screen: an option that vanishes
/// when it does not apply teaches nothing, and makes the menu a different
/// height each time it opens.
const DISABLED: f32 = 0.4;

/// One row of a menu.
///
/// Built by value rather than by a chain of `Div` calls so that `step` can walk
/// the same list the renderer draws.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    /// A group label. Same 10px uppercase used everywhere else in the system.
    Header(SharedString),
    Item(Item),
    Separator,
    /// What a menu says instead of being empty.
    Empty(SharedString),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub id: SharedString,
    pub label: SharedString,
    /// The right-hand value. A bitrate beside a quality name, a shortcut
    /// beside a command.
    pub detail: Option<SharedString>,
    /// Replaces the tick column when the row is not a choice - "More settings"
    /// is a destination, not something that can be on.
    pub icon: Option<Icon>,
    pub selected: bool,
    pub enabled: bool,
    /// The whole string, when `label` is the truncated one. A device name that
    /// does not fit the menu still has to be readable somewhere.
    pub tooltip: Option<SharedString>,
}

impl Item {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            icon: None,
            selected: false,
            enabled: true,
            tooltip: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

/// The row the arrows should land on, `delta` steps from `from`.
///
/// Headers, separators, the empty sentence and disabled rows are all skipped:
/// none of them can be chosen, so stopping on one would be a keypress that
/// looks like it did nothing. Wraps, because a menu is short and walking off
/// the bottom of a six-item list to find nothing there is worse than arriving
/// back at the top.
///
/// `None` back means there is nothing selectable at all, which is exactly the
/// case an `Empty` menu is for.
pub fn step(entries: &[Entry], from: Option<usize>, delta: isize) -> Option<usize> {
    let selectable: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches!(entry, Entry::Item(item) if item.enabled))
        .map(|(index, _)| index)
        .collect();
    if selectable.is_empty() {
        return None;
    }
    // Where `from` sits among the selectable rows, not among all of them. A
    // highlight parked on a header - which nothing puts it on, but a caller
    // could - counts as before the start.
    let position = from.and_then(|from| selectable.iter().position(|index| *index == from));
    let next = match position {
        Some(position) => (position as isize + delta).rem_euclid(selectable.len() as isize),
        // Opening with nothing highlighted: Down lands on the first row and Up
        // on the last, which is what every menu on both platforms does.
        None if delta >= 0 => 0,
        None => selectable.len() as isize - 1,
    };
    Some(selectable[next as usize])
}

/// The row that carries the current value, for a menu opening fresh.
pub fn selected(entries: &[Entry]) -> Option<usize> {
    entries
        .iter()
        .position(|entry| matches!(entry, Entry::Item(item) if item.selected && item.enabled))
}

/// The surface. Caller fills it with `entry` and owns where it is placed.
pub fn surface(id: &'static str, width: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .accessibility_id(id)
        .role(Role::Menu)
        .w(theme::u(width))
        .flex()
        .flex_col()
        .p(theme::u(PADDING))
        .rounded(theme::u(RADIUS))
        .bg(theme::bg_card())
        .border_1()
        .border_color(theme::border_card())
        .shadow(theme::floating())
}

/// One entry, drawn. `highlighted` is the row the arrow keys are on, which is
/// not the same thing as the row that is selected: selection is a tick and
/// survives the menu closing, highlight is a fill and does not.
pub fn entry(entry: &Entry, highlighted: bool) -> gpui::AnyElement {
    match entry {
        Entry::Header(label) => header(label.clone()).into_any_element(),
        Entry::Separator => separator().into_any_element(),
        Entry::Empty(sentence) => empty(sentence.clone()).into_any_element(),
        Entry::Item(spec) => item(spec, highlighted).into_any_element(),
    }
}

fn header(label: SharedString) -> impl IntoElement {
    div()
        .pt(theme::u(7.0))
        .px(theme::u(ITEM_PX))
        .pb(theme::u(5.0))
        .text_size(theme::u(theme::TEXT_TINY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_muted())
        // GPUI has no letter-spacing, so the 0.06em tracking the design asks
        // for is not expressible. Uppercasing here rather than leaving it to
        // the caller keeps every group label in the product one call away from
        // being wrong.
        .child(label.to_uppercase())
}

fn separator() -> impl IntoElement {
    div()
        .h(theme::u(1.0))
        .mx(theme::u(6.0))
        .my(theme::u(PADDING))
        .bg(theme::border_card())
}

/// The sentence a menu shows instead of being empty.
///
/// Not an item: it cannot be chosen, it is not focusable, and it wraps. A menu
/// with nothing in it is a bug that looks like a rendering failure, and this is
/// what stops one ever being drawn.
fn empty(sentence: SharedString) -> impl IntoElement {
    div()
        .px(theme::u(ITEM_PX))
        .py(theme::u(7.0))
        .text_size(theme::u(theme::TEXT_MICRO))
        .text_color(theme::text_muted())
        .child(sentence)
}

fn item(spec: &Item, highlighted: bool) -> gpui::Stateful<gpui::Div> {
    let text = if spec.selected {
        theme::text_primary()
    } else {
        theme::text_label()
    };
    let mut row = div()
        .id(SharedString::from(format!("menu-{}", spec.id)))
        .accessibility_id(format!("rapidcap.menu.{}", spec.id))
        .role(Role::MenuItem)
        .aria_label(spec.tooltip.clone().unwrap_or_else(|| spec.label.clone()))
        .aria_selected(spec.selected)
        .h(theme::u(ITEM_H))
        .px(theme::u(ITEM_PX))
        .flex()
        .flex_none()
        .items_center()
        .gap(theme::u(ITEM_GAP))
        .rounded(theme::u(ITEM_RADIUS))
        .text_size(theme::u(theme::TEXT_SMALL))
        .text_color(text)
        .child(
            div()
                .w(theme::u(TICK_W))
                .flex()
                .flex_none()
                .children(mark(spec)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(spec.label.clone()),
        )
        .children(spec.detail.clone().map(|detail| {
            div()
                .flex_none()
                .text_size(theme::u(theme::TEXT_MICRO))
                .text_color(theme::text_muted())
                .child(detail)
        }));
    if spec.enabled {
        row = row
            .cursor_pointer()
            // One `hover` call per element: GPUI panics on a second. The
            // keyboard highlight therefore has to be the same fill applied
            // directly, not a second hover rule.
            .hover(|style| style.bg(theme::bg_hover()))
            .when(highlighted, |row| row.bg(theme::bg_hover()));
    } else {
        row = row.opacity(DISABLED);
    }
    row
}

/// The tick, or the icon that replaces it on a row that is not a choice.
fn mark(spec: &Item) -> Option<gpui::AnyElement> {
    match (spec.icon, spec.selected) {
        (Some(icon), _) => Some(icon.element(theme::u(TICK_GLYPH), theme::text_muted())),
        (None, true) => Some(Icon::Check.element(theme::u(TICK_GLYPH), theme::accent())),
        // Selection is a tick in this column, never a highlighted row: a fill
        // already means hover, and one treatment carries one meaning.
        (None, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Entry> {
        vec![
            Entry::Header("Frame rate".into()),
            Entry::Item(Item::new("30", "30 FPS")),
            Entry::Item(Item::new("60", "60 FPS").selected(true)),
            Entry::Separator,
            Entry::Item(Item::new("loop", "Loop forever").disabled()),
            Entry::Item(Item::new("more", "More settings").icon(Icon::Settings)),
        ]
    }

    #[test]
    fn arrows_land_only_on_rows_that_can_be_chosen() {
        let entries = fixture();
        // 1, 2 and 5 are the enabled items; 0 is a header, 3 a separator and 4
        // is disabled.
        assert_eq!(step(&entries, None, 1), Some(1));
        assert_eq!(step(&entries, Some(1), 1), Some(2));
        assert_eq!(step(&entries, Some(2), 1), Some(5));
        assert_eq!(step(&entries, Some(5), 1), Some(1), "down wraps to the top");

        assert_eq!(step(&entries, None, -1), Some(5));
        assert_eq!(step(&entries, Some(5), -1), Some(2));
        assert_eq!(
            step(&entries, Some(1), -1),
            Some(5),
            "up wraps to the bottom"
        );
    }

    #[test]
    fn a_menu_with_nothing_to_choose_has_nowhere_to_go() {
        let entries = vec![
            Entry::Header("Microphone".into()),
            Entry::Empty("No capture devices found.".into()),
            Entry::Item(Item::new("off", "Off").disabled()),
        ];
        assert_eq!(step(&entries, None, 1), None);
        assert_eq!(step(&entries, None, -1), None);
        assert_eq!(selected(&entries), None);
    }

    #[test]
    fn a_menu_opens_on_the_row_it_is_already_set_to() {
        assert_eq!(selected(&fixture()), Some(2));
    }
}
