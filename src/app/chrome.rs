//! The window chrome: the title bar with the tabs and caption buttons, the
//! menu bar and its menus, and the status bar.

use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    anchored, deferred, div, img, prelude::*, px, relative, Action, AnyElement, Context, Corner, Image, ImageFormat,
    MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window, WindowControlArea,
};

use super::*;
use crate::doc::{Encoding, LineEnding};

pub(super) const TITLE_H: f32 = 40.0;
const MENU_H: f32 = 30.0;
const STATUS_H: f32 = 26.0;
const CAPTION_W: f32 = 46.0;
const ICON_W: f32 = 44.0;
/// The width of the menu bar's buttons, for telling a click on them apart
/// from a click elsewhere.
const MENU_BAR_W: f32 = 200.0;

static ICON: LazyLock<Arc<Image>> =
    LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../notey_icon.png").to_vec())));

/// The menus that can be open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    Help,
    /// All three, from the title-bar icon when the menu bar is hidden.
    All,
    Encoding,
    LineEnding,
    /// The language picker.
    Language,
    /// The editor's right-click menu.
    Context,
}

pub(super) type Choose = Rc<dyn Fn(&mut NoteyApp, &mut Window, &mut Context<NoteyApp>)>;

pub(super) enum Item {
    Action { label: SharedString, shortcut: &'static str, action: Box<dyn Action> },
    /// `checked` is `None` for an item without a check mark column.
    Choice { label: SharedString, checked: Option<bool>, choose: Choose },
    Heading(SharedString),
    Separator,
}

pub(super) fn action(label: &'static str, shortcut: &'static str, action: impl Action) -> Item {
    Item::Action { label: label.into(), shortcut, action: Box::new(action) }
}

impl NoteyApp {
    fn menu_items(&self, menu: Menu) -> Vec<Item> {
        match menu {
            Menu::File => {
                let mut items = vec![
                    action("New Tab", "Ctrl+N", NewTab),
                    action("New Window", "Ctrl+Shift+N", NewWindow),
                    action("Open…", "Ctrl+O", Open),
                ];
                if !self.settings.recent_files.is_empty() {
                    items.push(Item::Separator);
                    items.push(Item::Heading("Open Recent".into()));
                    for path in &self.settings.recent_files {
                        let path = path.clone();
                        let label = path.display().to_string().into();
                        items.push(Item::Choice {
                            label,
                            checked: None,
                            choose: Rc::new(move |this, window, cx| this.open_path(&path, window, cx)),
                        });
                    }
                }
                items.extend([
                    Item::Separator,
                    action("Save", "Ctrl+S", Save),
                    action("Save As…", "Ctrl+Shift+S", SaveAs),
                    action("Print…", "Ctrl+P", Print),
                    Item::Separator,
                    action("Close Tab", "Ctrl+W", CloseTab),
                    action("Exit", "", Exit),
                ]);
                items
            }
            Menu::Edit => vec![
                action("Undo", "Ctrl+Z", Undo),
                action("Redo", "Ctrl+Y", Redo),
                Item::Separator,
                action("Cut", "Ctrl+X", Cut),
                action("Copy", "Ctrl+C", Copy),
                action("Paste", "Ctrl+V", Paste),
                action("Delete", "Del", Delete),
                Item::Separator,
                action("Select All", "Ctrl+A", SelectAll),
                action("Insert Time/Date", "F5", InsertTimeDate),
                Item::Separator,
                action("Find…", "Ctrl+F", find::Find),
                action("Replace…", "Ctrl+H", find::Replace),
                action("Find Next", "F3", find::FindNext),
                action("Find Previous", "Shift+F3", find::FindPrevious),
                action("Find in Files…", "Ctrl+Shift+F", super::find_in_files::FindInFiles),
                action("Go To…", "Ctrl+G", find::GoTo),
                Item::Separator,
                action("Settings", "Ctrl+,", OpenSettings),
            ],
            Menu::Help => vec![action("About Notey", "", About)],
            Menu::Context => self.context_menu_items(),
            Menu::All => {
                let mut items = self.menu_items(Menu::File);
                items.push(Item::Separator);
                items.extend(self.menu_items(Menu::Edit));
                items.push(Item::Separator);
                items.extend(self.menu_items(Menu::Help));
                items
            }
            Menu::Encoding => {
                let current = self.tabs[self.active].doc.encoding;
                Encoding::ALL
                    .into_iter()
                    .map(|enc| Item::Choice {
                        label: enc.label().into(),
                        checked: Some(enc == current),
                        choose: Rc::new(move |this, _, cx| {
                            let tab = &mut this.tabs[this.active];
                            tab.doc.encoding = enc;
                            cx.notify();
                        }),
                    })
                    .collect()
            }
            Menu::Language => self.language_menu_items(),
            Menu::LineEnding => {
                let current = self.tabs[self.active].doc.line_ending;
                LineEnding::ALL
                    .into_iter()
                    .map(|le| Item::Choice {
                        label: le.label().into(),
                        checked: Some(le == current),
                        choose: Rc::new(move |this, _, cx| {
                            let tab = &mut this.tabs[this.active];
                            tab.doc.line_ending = le;
                            cx.notify();
                        }),
                    })
                    .collect()
            }
        }
    }

    /// Whether `pos` is on something that opens a menu. A click there is
    /// left to that control, so it can switch or close the menu.
    fn in_menu_trigger(&self, pos: Point<Pixels>, window: &Window) -> bool {
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        let height = f32::from(window.viewport_size().height);
        (self.settings.show_menu && (TITLE_H..TITLE_H + MENU_H).contains(&y) && x < MENU_BAR_W)
            || (!self.settings.show_menu && y < TITLE_H && x < ICON_W)
            || (self.settings.show_status && y >= height - STATUS_H)
    }

    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.open_menu = if self.open_menu == Some(menu) { None } else { Some(menu) };
        cx.notify();
    }

    pub(super) fn menu_panel(&self, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let rows = self.menu_items(menu).into_iter().enumerate().map(|(i, item)| -> AnyElement {
            let row = || {
                div()
                    .id(("menu-item", i))
                    .mx_1()
                    .px_3()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap_6()
                    .rounded(px(4.))
                    .hover(|s| s.bg(p.control_hover))
            };
            match item {
                Item::Action { label, shortcut, action } => row()
                    .justify_between()
                    .child(label)
                    .child(div().text_color(p.text_weak).child(shortcut))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_menu = None;
                        this.focus_active(window, cx);
                        window.dispatch_action(action.boxed_clone(), cx);
                        cx.notify();
                    }))
                    .into_any_element(),
                Item::Choice { label, checked, choose } => row()
                    .when_some(checked, |d, checked| {
                        d.child(div().w(px(12.)).text_color(p.accent).child(if checked { "✓" } else { "" }))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_menu = None;
                        choose(this, window, cx);
                        this.focus_active(window, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
                Item::Heading(label) => {
                    div().px_4().pt_1().pb_0p5().text_size(px(11.)).text_color(p.text_weak).child(label).into_any_element()
                }
                Item::Separator => div().my_1().mx_2().h(px(1.)).bg(p.stroke).into_any_element(),
            }
        });
        div()
            .id("menu")
            .occlude()
            .min_w(px(200.))
            .max_w(px(520.))
            .max_h(px(440.))
            .overflow_y_scroll()
            .py_1()
            .bg(p.popup)
            .border_1()
            .border_color(p.stroke)
            .rounded(px(8.))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                if !this.in_menu_trigger(ev.position, window) {
                    this.open_menu = None;
                    cx.notify();
                }
            }))
            .children(rows)
    }

    /// The menu `menu`, below its button when `below`, else above it.
    fn menu_popup(&self, menu: Menu, below: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.open_menu != Some(menu) {
            return None;
        }
        let place = div().absolute().left_0();
        let place = if below { place.top(relative(1.)) } else { place.bottom(relative(1.)) };
        let popup = anchored()
            .anchor(if below { Corner::TopLeft } else { Corner::BottomLeft })
            .snap_to_window_with_margin(px(8.))
            .child(self.menu_panel(menu, cx));
        Some(place.child(deferred(popup).with_priority(1)))
    }

    pub(super) fn title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let icon = div()
            .id("app-icon")
            .relative()
            .w(px(ICON_W))
            .h_full()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .child(div().size(px(28.)).flex().items_center().justify_center().rounded(px(4.)).when(
                !self.settings.show_menu,
                |d| d.hover(|s| s.bg(p.tab_hover)),
            ).child(img(ICON.clone()).size(px(16.))))
            .when(!self.settings.show_menu, |d| {
                d.on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.toggle_menu(Menu::All, cx)))
                    .children(self.menu_popup(Menu::All, true, cx))
            });

        let mut tabs: Vec<AnyElement> = Vec::new();
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = !self.settings_active && i == self.active;
            let title: SharedString = tab.doc.title().into();
            tabs.push(
                self.tab_element(("tab", i), title, tab.modified, active, cx)
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.request_close_tab(i, window, cx)),
                    )
                    .child(self.tab_close(("close", i), cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_close_tab(i, window, cx)
                    })))
                    .into_any_element(),
            );
        }
        if self.settings_open {
            tabs.push(
                self.tab_element("settings-tab", "Settings".into(), false, self.settings_active, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
                    .child(self.tab_close("settings-close", cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_settings(window, cx)
                    })))
                    .into_any_element(),
            );
        }
        let new_tab = div()
            .id("new-tab")
            .ml_1()
            .mb(px(4.))
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .font_family(self.icon_font.clone())
            .text_size(px(12.))
            .hover(|s| s.bg(p.tab_hover))
            .child("\u{E710}")
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)));

        let caption = |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
            div()
                .id(id)
                .w(px(CAPTION_W))
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .font_family(self.icon_font.clone())
                .text_size(px(10.))
                .window_control_area(area)
                .when(close, |d| d.hover(|s| s.bg(p.close_hover).text_color(gpui::white())))
                .when(!close, |d| d.hover(|s| s.bg(p.control_hover)))
                .child(glyph)
        };
        let max_glyph = if window.is_maximized() { "\u{E923}" } else { "\u{E922}" };

        div()
            .id("titlebar")
            .h(px(TITLE_H))
            .w_full()
            .flex()
            .flex_none()
            .items_end()
            .child(icon)
            .child(div().flex().flex_row().items_end().min_w_0().overflow_hidden().children(tabs))
            .child(new_tab)
            .child(div().flex_1().h_full().window_control_area(WindowControlArea::Drag))
            .child(
                div()
                    .flex()
                    .h_full()
                    .child(caption("min", "\u{E921}", WindowControlArea::Min, false))
                    .child(caption("max", max_glyph, WindowControlArea::Max, false))
                    .child(caption("close", "\u{E8BB}", WindowControlArea::Close, true)),
            )
    }

    fn tab_element(
        &self,
        id: impl Into<gpui::ElementId>,
        title: SharedString,
        modified: bool,
        active: bool,
        _cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.palette;
        div()
            .id(id)
            .h(px(32.))
            .min_w(px(80.))
            .max_w(px(220.))
            .pl_3()
            .pr_1()
            .flex()
            .flex_shrink()
            .items_center()
            .gap_2()
            .rounded_t(px(6.))
            .when(active, |d| d.bg(p.editor))
            .when(!active, |d| d.text_color(p.text_weak).hover(|s| s.bg(p.tab_hover)))
            .when(modified, |d| d.child(div().flex_none().text_color(p.accent).child("●")))
            .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(title))
    }

    fn tab_close(
        &self,
        id: impl Into<gpui::ElementId>,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        let p = self.palette;
        div()
            .id(id)
            .size(px(22.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .font_family(self.icon_font.clone())
            .text_size(px(9.))
            .hover(|s| s.bg(p.control_hover))
            .child("\u{E8BB}")
            .on_click(on_click)
    }

    pub(super) fn menu_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let mut button = |menu: Menu, label: &'static str| {
            let open = self.open_menu == Some(menu);
            div()
                .relative()
                .child(
                    div()
                        .id(label)
                        .h(px(26.))
                        .px_2p5()
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .when(open, |d| d.bg(p.control_active))
                        .hover(|s| s.bg(p.control_hover))
                        .child(label)
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| this.toggle_menu(menu, cx)))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.open_menu.is_some_and(|m| m != menu) {
                                this.open_menu = Some(menu);
                                cx.notify();
                            }
                        })),
                )
                .children(self.menu_popup(menu, true, cx))
        };
        div()
            .h(px(MENU_H))
            .flex()
            .flex_none()
            .items_center()
            .px_1()
            .child(button(Menu::File, "File"))
            .child(button(Menu::Edit, "Edit"))
            .child(button(Menu::Help, "Help"))
    }

    pub(super) fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let sep = || div().w(px(1.)).h(px(14.)).bg(p.stroke);
        let item = |id: &'static str, label: SharedString, menu: Option<Menu>, cx: &mut Context<Self>| {
            div().relative().child(
                div()
                    .id(id)
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .child(label)
                    .when_some(menu, |d, menu| {
                        d.hover(|s| s.bg(p.control_hover))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| this.toggle_menu(menu, cx)))
                    }),
            )
        };
        let mut right = div().flex().items_center().gap_1();
        let mut left = div().flex_1().min_w_0().flex().items_center().gap_1();
        if !self.settings_active {
            if let Some(tab) = self.tabs.get(self.active) {
                let pos = match &self.status_msg {
                    Some(msg) => msg.clone(),
                    None => format!("Ln {}, Col {}", tab.line, tab.col).into(),
                };
                left = left.child(item("pos", pos, None, cx));
                right = right
                    .child(item("chars", format!("{} characters", tab.chars).into(), None, cx))
                    .child(sep())
                    .child(item("zoom", format!("{:.0}%", self.settings.zoom).into(), None, cx))
                    .child(sep())
                    .child(
                        item(
                            "language",
                            tab.doc.language.clone().unwrap_or_else(|| "Plain Text".into()).into(),
                            Some(Menu::Language),
                            cx,
                        )
                        .children(self.menu_popup(Menu::Language, false, cx)),
                    )
                    .child(sep())
                    .child(
                        item("line-ending", tab.doc.line_ending.label().into(), Some(Menu::LineEnding), cx)
                            .children(self.menu_popup(Menu::LineEnding, false, cx)),
                    )
                    .child(sep())
                    .child(
                        item("encoding", tab.doc.encoding.label().into(), Some(Menu::Encoding), cx)
                            .children(self.menu_popup(Menu::Encoding, false, cx)),
                    );
            }
        } else if let Some(msg) = &self.status_msg {
            left = left.child(item("pos", msg.clone(), None, cx));
        }
        div()
            .h(px(STATUS_H))
            .flex()
            .flex_none()
            .items_center()
            .px_1()
            .border_t_1()
            .border_color(p.stroke)
            .text_size(px(12.))
            .text_color(p.text_weak)
            .child(left)
            .child(right)
    }
}
