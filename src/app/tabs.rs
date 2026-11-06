//! What tabs can do besides being opened and closed: the tab right-click
//! menu, pinned tabs (in the main window), tab colors (which plugins set),
//! and split view, which shows a second tab beside the active one.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    anchored, canvas, deferred, AnchoredPositionMode, AnyElement, Bounds, ClipboardItem, MouseDownEvent, Pixels, Point,
};

use super::chrome::{Item, Menu};
use super::*;

actions!(notey, [ToggleSplit]);

/// The narrowest a pane of the split view gets, as a share of the width.
const MIN_PANE: f32 = 0.15;

/// Split view: the active tab's pane and another tab's pane, side by side.
/// The active tab is always the one with the keyboard focus; clicking the
/// other pane swaps them.
pub(super) struct Split {
    /// The document in the other pane.
    pub(super) other: u64,
    /// The other pane is the left one.
    pub(super) other_left: bool,
    /// The left pane's share of the width.
    pub(super) ratio: f32,
}

/// Where the split view was drawn, for dragging the divider.
pub(super) type SplitArea = Rc<Cell<Option<Bounds<Pixels>>>>;

/// The tab right-click menu: where it opened and for which document.
pub(super) struct TabMenu {
    position: Point<Pixels>,
    doc_id: u64,
}

impl NoteyApp {
    // ---------- the tab menu ----------

    /// Right-click on a tab: its menu, without switching to it.
    pub(super) fn open_tab_menu(&mut self, idx: usize, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(doc_id) = self.tabs.get(idx).map(|t| t.doc.id) else { return };
        self.close_menus();
        self.tab_menu = Some(TabMenu { position: event.position, doc_id });
        self.open_menu = Some(Menu::Tab);
        cx.notify();
    }

    pub(super) fn tab_menu_items(&self) -> Vec<Item> {
        let Some(id) = self.tab_menu.as_ref().map(|m| m.doc_id) else { return Vec::new() };
        let Some(idx) = self.tab_index(id) else { return Vec::new() };
        let tab = &self.tabs[idx];
        let choice = |label: &str, choose: chrome::Choose| Item::Choice { label: label.to_string().into(), checked: None, choose };
        let mut items = Vec::new();
        if self.is_primary {
            let pin = !tab.pinned;
            items.push(choice(
                if pin { "Pin Tab" } else { "Unpin Tab" },
                Rc::new(move |this, _, cx| this.set_pinned(id, pin, cx)),
            ));
        }
        // Plugin commands for tabs (colors), run on this tab.
        let plugin = self.plugin_items_for(Some(crate::plugins::TAB_MENU), Some(id));
        items.extend(plugin);
        if !items.is_empty() {
            items.push(Item::Separator);
        }
        match &self.split {
            None => items.push(choice("Open in Split View", Rc::new(move |this, window, cx| this.open_split(id, window, cx)))),
            Some(split) => {
                let shown = split.other == id || self.tabs.get(self.active).is_some_and(|t| t.doc.id == id);
                if !shown {
                    items.push(choice("Show in the Other Pane", Rc::new(move |this, window, cx| this.show_in_other_pane(id, window, cx))));
                }
                items.push(choice("Close Split View", Rc::new(|this, window, cx| this.close_split(window, cx))));
            }
        }
        items.push(Item::Separator);
        if !tab.pinned {
            items.push(choice("Close Tab", Rc::new(move |this, window, cx| {
                if let Some(i) = this.tab_index(id) {
                    this.request_close_tab(i, window, cx);
                }
            })));
        }
        let others: Vec<u64> = self.tabs.iter().filter(|t| !t.pinned && t.doc.id != id).map(|t| t.doc.id).collect();
        if !others.is_empty() {
            items.push(choice("Close Other Tabs", Rc::new(move |this, window, cx| this.close_tabs(others.clone(), window, cx))));
        }
        let right: Vec<u64> = self.tabs[idx + 1..].iter().filter(|t| !t.pinned).map(|t| t.doc.id).collect();
        if !right.is_empty() {
            items.push(choice("Close Tabs to the Right", Rc::new(move |this, window, cx| this.close_tabs(right.clone(), window, cx))));
        }
        if let Some(path) = tab.doc.path.clone() {
            items.push(Item::Separator);
            let copy = path.display().to_string();
            items.push(choice("Copy Path", Rc::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))));
            items.push(choice("Show in File Explorer", Rc::new(move |_, _, _| {
                let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
            })));
        }
        items
    }

    /// The tab menu, at the pointer.
    pub(super) fn tab_menu_popup(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.open_menu != Some(Menu::Tab) {
            return None;
        }
        let menu = self.tab_menu.as_ref()?;
        let popup = anchored()
            .position(menu.position)
            .position_mode(AnchoredPositionMode::Window)
            .snap_to_window_with_margin(px(8.))
            .child(self.menu_panel(Menu::Tab, cx));
        Some(deferred(popup).with_priority(1))
    }

    // ---------- closing several tabs ----------

    /// Close the tabs `ids`, one after another; a tab with unsaved changes
    /// asks first, and Cancel stops there.
    pub(super) fn close_tabs(&mut self, ids: Vec<u64>, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            for id in ids {
                let Ok(closing) = this.update_in(cx, |this, window, cx| this.ask_close(id, window, cx)) else { return };
                if !closing.await {
                    return;
                }
            }
        })
        .detach();
    }

    // ---------- pinned tabs ----------

    /// Pin or unpin a tab. Pinned tabs come first, in the order they were
    /// pinned.
    pub(super) fn set_pinned(&mut self, id: u64, pinned: bool, cx: &mut Context<Self>) {
        let Some(idx) = self.tab_index(id) else { return };
        if self.tabs[idx].pinned == pinned || (pinned && !self.is_primary) {
            return;
        }
        let active = self.tabs.get(self.active).map(|t| t.doc.id);
        let mut tab = self.tabs.remove(idx);
        tab.pinned = pinned;
        let at = self.tabs.iter().take_while(|t| t.pinned).count();
        self.tabs.insert(at, tab);
        if let Some(i) = active.and_then(|a| self.tab_index(a)) {
            self.active = i;
        }
        self.session_dirty = true;
        cx.notify();
    }

    // ---------- tab colors ----------

    /// Give a tab a color (a name that follows the theme, or "#rrggbb"),
    /// or none. A file's color is kept for the next time it opens.
    pub(super) fn set_tab_color(&mut self, id: u64, color: Option<String>, cx: &mut Context<Self>) {
        let Some(idx) = self.tab_index(id) else { return };
        let tab = &mut self.tabs[idx];
        tab.color = color.clone();
        if let Some(path) = &tab.doc.path {
            let key = path.display().to_string();
            match color {
                Some(c) => self.settings.tab_colors.insert(key, c),
                None => self.settings.tab_colors.remove(&key),
            };
            self.settings.save();
        }
        self.session_dirty = true;
        cx.notify();
    }

    /// The remembered color of the file at `path`.
    pub(super) fn file_color(&self, path: &Path) -> Option<String> {
        self.settings.tab_colors.get(&path.display().to_string()).cloned()
    }

    // ---------- split view ----------

    /// Show `id` in a pane of its own, on the right, with the keyboard
    /// focus; the left pane keeps the active tab, or when that is `id`,
    /// shows the tab next to it (a new tab when there is none).
    pub(super) fn open_split(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(idx) = self.tab_index(id) else { return };
        let left = match self.tabs.get(self.active).map(|t| t.doc.id) {
            Some(a) if a != id => a,
            _ => match self.tabs.get(if idx > 0 { idx - 1 } else { idx + 1 }) {
                Some(t) => t.doc.id,
                None => {
                    let doc = Document::new(self.new_doc_id(), self.next_untitled);
                    self.next_untitled += 1;
                    let i = self.add_tab(doc, String::new(), cx);
                    self.tabs[i].doc.id
                }
            },
        };
        self.split = Some(Split { other: left, other_left: true, ratio: 0.5 });
        if let Some(i) = self.tab_index(id) {
            self.activate(i, window, cx);
        }
        self.session_dirty = true;
    }

    /// Show `id` in the pane without focus.
    fn show_in_other_pane(&mut self, id: u64, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(split) = &mut self.split {
            split.other = id;
            self.session_dirty = true;
            cx.notify();
        }
    }

    pub(super) fn close_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.split.take().is_some() {
            self.session_dirty = true;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub(super) fn toggle_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.split.is_some() {
            self.close_split(window, cx);
        } else if let Some(id) = self.tabs.get(self.active).map(|t| t.doc.id) {
            self.open_split(id, window, cx);
        }
    }

    /// The tab at `idx` becomes active: when it is in the other pane, the
    /// panes swap roles.
    pub(super) fn swap_if_other(&mut self, idx: usize) {
        let (Some(split), Some(new), Some(old)) =
            (&mut self.split, self.tabs.get(idx).map(|t| t.doc.id), self.tabs.get(self.active).map(|t| t.doc.id))
        else {
            return;
        };
        if new == split.other && new != old {
            split.other = old;
            split.other_left = !split.other_left;
        }
    }

    /// After tabs closed: the split ends when its other tab is gone or is
    /// now the active one.
    pub(super) fn check_split(&mut self) {
        let active = self.tabs.get(self.active).map(|t| t.doc.id);
        if let Some(split) = &self.split {
            if self.tab_index(split.other).is_none() || Some(split.other) == active {
                self.split = None;
            }
        }
    }

    /// The document area in split view: `focused` (the active tab's pane)
    /// and the other tab's editor, with a divider that can be dragged.
    pub(super) fn split_body(&self, focused: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(split) = &self.split else { return focused };
        let Some(other) = self.tab_index(split.other) else { return focused };
        let p = self.palette;
        let other_pane = div()
            .id("other-pane")
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .child(self.tabs[other].view.clone())
            // A click in the other pane gives it the focus (after the editor
            // has put the cursor where the click was).
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, window, cx| {
                if let Some(i) = this.split.as_ref().and_then(|s| this.tab_index(s.other)) {
                    this.activate(i, window, cx);
                }
            }))
            .into_any_element();
        let (left, right) = if split.other_left { (other_pane, focused) } else { (focused, other_pane) };
        let area = self.split_area.clone();
        let divider = div()
            .relative()
            .w(px(1.))
            .h_full()
            .flex_none()
            .bg(p.stroke)
            .child(
                div()
                    .id("split-divider")
                    .absolute()
                    .top_0()
                    .left(px(-3.))
                    .w(px(7.))
                    .h_full()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        this.resizing_split = true;
                        cx.stop_propagation();
                    })),
            );
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(div().w(gpui::relative(split.ratio)).h_full().flex_none().min_w_0().flex().child(left))
            .child(divider)
            .child(div().flex_1().h_full().min_w_0().flex().child(right))
            .child(canvas(move |bounds, _, _| area.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full())
            .into_any_element()
    }

    /// Follow the pointer while the split divider is dragged.
    pub(super) fn resize_split(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if !self.resizing_split {
            return;
        }
        let (Some(split), Some(area)) = (&mut self.split, self.split_area.get()) else { return };
        let width = f32::from(area.size.width).max(1.0);
        split.ratio = (f32::from(x - area.origin.x) / width).clamp(MIN_PANE, 1.0 - MIN_PANE);
        cx.notify();
    }

    pub(super) fn end_split_resize(&mut self) {
        if std::mem::take(&mut self.resizing_split) {
            self.session_dirty = true;
        }
    }
}
