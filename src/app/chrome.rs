//! The window chrome: the title bar with the tabs and caption buttons, the
//! menu bar and its menus, and the status bar.

use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    anchored, canvas, deferred, div, img, prelude::*, px, relative, Action, AnyElement, Context, Corner, Image, ImageFormat,
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

/// The element ids of a menu's panel and of its submenus' panels, by level.
const PANEL_IDS: [&str; 4] = ["menu", "submenu", "submenu2", "submenu3"];

/// Where each submenu row of the open menu was drawn, by level and label.
pub(super) type SubmenuRows = Rc<std::cell::RefCell<std::collections::HashMap<(usize, SharedString), gpui::Bounds<Pixels>>>>;

type HoverListener = Box<dyn Fn(&bool, &mut Window, &mut App)>;

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
    /// A plugin command.
    Plugin {
        label: SharedString,
        shortcut: SharedString,
        swatch: Option<gpui::Hsla>,
        enabled: bool,
        script: usize,
        cmd: String,
        /// The script behind the command, for the tooltip.
        source: SharedString,
    },
    /// A submenu (a plugin command group).
    Submenu { label: SharedString, items: Vec<Item> },
}

pub(super) fn action(label: &'static str, shortcut: &'static str, action: impl Action) -> Item {
    Item::Action { label: label.into(), shortcut, action: Box::new(action) }
}

impl NoteyApp {
    pub(super) fn menu_items(&self, menu: Menu) -> Vec<Item> {
        match menu {
            Menu::File => {
                let mut items = vec![
                    action("New Tab", "Ctrl+N", NewTab),
                    action("New Window", "Ctrl+Shift+N", NewWindow),
                    action("Open…", "Ctrl+O", Open),
                ];
                if !self.settings.recent_files.is_empty() {
                    let recent = self
                        .settings
                        .recent_files
                        .iter()
                        .map(|path| {
                            let path = path.clone();
                            Item::Choice {
                                label: path.display().to_string().into(),
                                checked: None,
                                choose: Rc::new(move |this, window, cx| this.open_path(&path, window, cx)),
                            }
                        })
                        .collect();
                    items.push(Item::Submenu { label: "Open Recent".into(), items: recent });
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
            // The plugin commands follow, a section for each plugin menu.
            Menu::Edit => self
                .edit_items()
                .into_iter()
                .chain(crate::plugins::MENUS.iter().flat_map(|menu| {
                    let items = self.plugin_items(Some(menu));
                    let sep = (!items.is_empty()).then_some(Item::Separator);
                    sep.into_iter().chain(items)
                }))
                .collect(),
            Menu::Help => vec![action("About Notey", "", About)],
            Menu::Context => self.context_menu_items(),
            // A submenu for each menu of the menu bar, and for the commands
            // of each plugin menu.
            Menu::All => {
                let mut items = vec![
                    Item::Submenu { label: "File".into(), items: self.menu_items(Menu::File) },
                    Item::Submenu { label: "Edit".into(), items: self.edit_items() },
                ];
                for menu in crate::plugins::MENUS {
                    let commands = self.plugin_items(Some(menu));
                    if !commands.is_empty() {
                        items.push(Item::Submenu { label: menu.into(), items: commands });
                    }
                }
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
            // The language picker is its own popup (`language_popup`).
            Menu::Language => Vec::new(),
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

    /// The Edit menu's own commands, without those of plugins.
    fn edit_items(&self) -> Vec<Item> {
        vec![
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
            action("Find in Files…", "Ctrl+Shift+F", super::find_in_files::FindInFiles),
            action("Go To…", "Ctrl+G", find::GoTo),
            Item::Separator,
            action("Settings", "Ctrl+,", OpenSettings),
        ]
    }

    /// The items of the submenus open at `levels` (the first `levels` of
    /// `open_submenus`) inside `menu`; `None` when one of them is gone.
    pub(super) fn submenu_items(&self, menu: Menu, levels: usize) -> Option<Vec<Item>> {
        let mut items = self.menu_items(menu);
        for label in self.open_submenus.iter().take(levels) {
            items = items.into_iter().find_map(|item| match item {
                Item::Submenu { label: l, items } if &l == label => Some(items),
                _ => None,
            })?;
        }
        Some(items)
    }

    /// Open the submenu `label` of the menu at `level`, closing any deeper ones.
    fn open_submenu(&mut self, level: usize, label: SharedString) {
        self.open_submenus.truncate(level);
        self.open_submenus.push(label);
    }

    /// Close the open menu and its submenus.
    fn close_menus(&mut self) {
        self.open_menu = None;
        self.open_submenus.clear();
        self.submenu_hovered = 0;
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
        let open = self.open_menu != Some(menu);
        self.close_menus();
        self.open_menu = open.then_some(menu);
        self.languages.focus_pending = self.open_menu == Some(Menu::Language);
        cx.notify();
    }

    /// The plugin commands for one menu (a name from `plugins::MENUS`), or
    /// with `None`, the commands for the editor's right-click menu. Commands
    /// with a group go in a submenu, before the other commands.
    pub(super) fn plugin_items(&self, menu: Option<&str>) -> Vec<Item> {
        let scripts = &self.scripting.host.scripts;
        let language = self.tabs.get(self.active).and_then(|t| t.doc.language.clone());
        let wanted = |c: &crate::plugins::PluginCommand| match menu {
            Some(m) => c.menu == m,
            None => c.context,
        };
        let item = |i: usize, c: &crate::plugins::PluginCommand| Item::Plugin {
            label: c.title.clone().into(),
            shortcut: c.shortcut_text.clone().into(),
            swatch: c.swatch,
            // Focus conditions do not apply to menu clicks.
            enabled: c.when.matches(language.as_deref(), true),
            script: i,
            cmd: c.id.clone(),
            source: format!("{} \u{2014} {}", scripts[i].name, scripts[i].path.display()).into(),
        };
        let mut groups: Vec<String> = Vec::new();
        for c in scripts.iter().flat_map(|s| &s.commands).filter(|c| wanted(c)) {
            if let Some(g) = &c.group {
                if !groups.contains(g) {
                    groups.push(g.clone());
                }
            }
        }
        let mut items: Vec<Item> = groups
            .into_iter()
            .map(|g| Item::Submenu {
                items: scripts
                    .iter()
                    .enumerate()
                    .flat_map(|(i, s)| s.commands.iter().map(move |c| (i, c)))
                    .filter(|(_, c)| wanted(c) && c.group.as_deref() == Some(g.as_str()))
                    .map(|(i, c)| item(i, c))
                    .collect(),
                label: g.into(),
            })
            .collect();
        for (i, s) in scripts.iter().enumerate() {
            items.extend(s.commands.iter().filter(|c| wanted(c) && c.group.is_none()).map(|c| item(i, c)));
        }
        items
    }

    pub(super) fn menu_panel(&self, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        self.item_list(0, self.menu_items(menu), cx)
            // Menus scroll when taller than the window; submenus are drawn
            // outside them (`submenu_popups`).
            .max_h(px(self.viewport_h - 16.))
            .overflow_y_scroll()
            .on_mouse_down_out(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                if !this.in_menu_trigger(ev.position, window) && this.submenu_hovered == 0 {
                    this.close_menus();
                    cx.notify();
                }
            }))
            .bg(p.popup)
    }

    /// Pointing at an item of the menu at `level` closes the submenus open
    /// below it.
    fn close_deeper(level: usize, cx: &mut Context<Self>) -> HoverListener {
        Box::new(cx.listener(move |this, hovered: &bool, _, cx| {
            if *hovered && this.open_submenus.len() > level {
                this.open_submenus.truncate(level);
                cx.notify();
            }
        }))
    }

    /// The panel of the menu at `level` (0 for the menu, 1 for its open
    /// submenu, ...), with `items`; submenus open beside their row on hover.
    fn item_list(&self, level: usize, items: Vec<Item>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.palette;
        let id = PANEL_IDS[level.min(PANEL_IDS.len() - 1)];
        let rows = items.into_iter().enumerate().map(|(i, item)| -> AnyElement {
            let row = || {
                div()
                    .id((id, i))
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
                Item::Plugin { label, shortcut, swatch, enabled, script, cmd, source } => row()
                    .tooltip(crate::prompt::tooltip(source))
                    .on_hover(Self::close_deeper(level, cx))
                    .justify_between()
                    .when(!enabled, |d| d.text_color(p.text_weak))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(swatch, |d, c| {
                                d.child(div().size(px(12.)).rounded(px(2.)).bg(c).border_1().border_color(p.stroke))
                            })
                            .child(label),
                    )
                    .child(div().text_color(p.text_weak).child(shortcut))
                    .when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.close_menus();
                            this.focus_active(window, cx);
                            this.run_plugin_command(script, &cmd, window, cx);
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
                Item::Submenu { label, items: _ } => {
                    let open = self.open_submenus.get(level) == Some(&label);
                    let hover_label = label.clone();
                    let click_label = label.clone();
                    // The row's bounds, where `submenu_popups` puts the submenu.
                    let rows = self.submenu_rows.clone();
                    let bounds_key = (level, label.clone());
                    row()
                        .relative()
                        .justify_between()
                        .when(open, |d| d.bg(p.control_hover))
                        .child(label.clone())
                        .child(div().font_family(self.icon_font.clone()).text_size(px(9.)).child("\u{E76C}"))
                        .child(
                            canvas(move |bounds, _, _| rows.borrow_mut().insert(bounds_key, bounds), |_, _, _, _| {})
                                .absolute()
                                .size_full(),
                        )
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.open_submenus.get(level) != Some(&hover_label) {
                                this.open_submenu(level, hover_label.clone());
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_submenu(level, click_label.clone());
                            cx.notify();
                        }))
                        .into_any_element()
                }
                Item::Action { label, shortcut, action } => row()
                    .on_hover(Self::close_deeper(level, cx))
                    .justify_between()
                    .child(label)
                    .child(div().text_color(p.text_weak).child(shortcut))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menus();
                        this.focus_active(window, cx);
                        window.dispatch_action(action.boxed_clone(), cx);
                        cx.notify();
                    }))
                    .into_any_element(),
                Item::Choice { label, checked, choose } => row()
                    .on_hover(Self::close_deeper(level, cx))
                    .when_some(checked, |d, checked| {
                        d.child(div().w(px(12.)).text_color(p.accent).child(if checked { "✓" } else { "" }))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menus();
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
            .id(id)
            .occlude()
            .min_w(px(200.))
            .max_w(px(520.))
            .py_1()
            .border_1()
            .border_color(p.stroke)
            .rounded(px(8.))
            .shadow_lg()
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
            .tooltip(crate::prompt::tooltip("New tab (Ctrl+N)"))
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
            .when(!self.settings.tabs_left, |d| {
                d.child(div().flex().flex_row().items_end().min_w_0().overflow_hidden().children(tabs)).child(new_tab)
            })
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

    pub(super) fn status_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language_popup = self.language_popup(window, cx);
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
                    })
                    .when_some(
                        match id {
                            "language" => Some("Syntax highlighting language"),
                            "line-ending" => Some("Line endings used when saving"),
                            "encoding" => Some("Encoding used when saving"),
                            _ => None,
                        },
                        |d, tip| d.when(menu.is_none_or(|m| self.open_menu != Some(m)), |d| d.tooltip(crate::prompt::tooltip(tip))),
                    ),
            )
        };
        let mut right = div().flex().items_center().gap_1();
        let mut left = div().flex_1().min_w_0().flex().items_center().gap_1();
        // With the tabs on the left, the gear is at the bottom of the tab list.
        if !self.settings.tabs_left {
            left = left.child(self.gear_button(cx)).child(sep());
        }
        if !self.settings_active {
            if let Some(tab) = self.tabs.get(self.active) {
                let pos = match &self.status_msg {
                    Some(msg) => msg.clone(),
                    None => format!("Ln {}, Col {}", tab.line, tab.col).into(),
                };
                left = left.child(item("pos", pos, None, cx));
                let hint = match (&tab.doc.path, &tab.doc.language, &self.status_msg) {
                    (Some(path), None, None) => self.plugins.suggestion_for(path).map(|m| (m.id.clone(), m.name.clone())),
                    _ => None,
                };
                if let Some((id, name)) = hint {
                    left = left.child(
                        div()
                            .id("install-filetype")
                            .h(px(20.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(p.control_hover)
                            .border_1()
                            .border_color(p.stroke)
                            .hover(|s| s.bg(p.control_active))
                            .text_color(p.text)
                            .child(format!("Install {name} highlighting"))
                            .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(&id, cx))),
                    );
                }
                for (i, (_, text, tip)) in self.scripting.status_items.iter().enumerate() {
                    let item = div().id(("plugin-status", i)).px_2().child(text.clone());
                    let item = if tip.is_empty() { item } else { item.tooltip(crate::prompt::tooltip(tip.clone())) };
                    right = right.child(item).child(sep());
                }
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
                        .children(language_popup),
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

impl NoteyApp {
    /// The gear icon that opens the Settings tab.
    pub(super) fn gear_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        div()
            .id("gear")
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .font_family(self.icon_font.clone())
            .text_size(px(13.))
            .text_color(if self.settings_active { p.text } else { p.text_weak })
            .hover(|s| s.bg(p.tab_hover))
            .child("\u{E713}")
            .tooltip(crate::prompt::tooltip("Settings (Ctrl+,)"))
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
    }

    /// The tabs as a vertical list on the left (Settings > Appearance > Tabs).
    pub(super) fn left_tab_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let entry = |id: gpui::ElementId, group: SharedString, title: SharedString, modified: bool, active: bool, gear: bool| {
            div()
                .id(id)
                .group(group.clone())
                .h(px(26.))
                .pl_2()
                .pr_1()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .rounded(px(4.))
                .when(active, |d| d.bg(p.control_active))
                .when(!active, |d| d.text_color(p.text_weak).hover(|s| s.bg(p.tab_hover)))
                .when(gear, |d| d.child(div().font_family(self.icon_font.clone()).text_size(px(11.)).child("\u{E713}")))
                .when(modified, |d| d.child(div().flex_none().text_color(p.accent).child("●")))
                .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(title))
        };
        let mut list = div().id("tab-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_0p5();
        for (i, tab) in self.tabs.iter().enumerate() {
            let group: SharedString = format!("left-tab-{i}").into();
            let active = !self.settings_active && i == self.active;
            let close = self.tab_close(("left-close", i), cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.request_close_tab(i, window, cx)
            }));
            list = list.child(
                entry(("left-tab", i).into(), group.clone(), tab.doc.title().into(), tab.modified, active, false)
                    .child(div().when(!active, |d| d.invisible().group_hover(group, |s| s.visible())).child(close))
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(i, window, cx)))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.request_close_tab(i, window, cx))),
            );
        }
        if self.settings_open {
            let close = self.tab_close("left-settings-close", cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.close_settings(window, cx)
            }));
            list = list.child(
                entry("left-settings".into(), "left-settings".into(), "Settings".into(), false, self.settings_active, true)
                    .child(close)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            );
        }
        let new_tab = div()
            .id("left-new-tab")
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .font_family(self.icon_font.clone())
            .text_size(px(11.))
            .hover(|s| s.bg(p.tab_hover))
            .child("\u{E710}")
            .tooltip(crate::prompt::tooltip("New tab (Ctrl+N)"))
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)));
        // Dragging the right edge resizes the list.
        let handle = div()
            .id("tab-list-resize")
            .absolute()
            .top_0()
            .right_0()
            .w(px(5.))
            .h_full()
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.resizing_tab_list = true;
                cx.stop_propagation();
            }));
        div()
            .relative()
            .w(px(self.settings.tabs_left_width))
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .p_1p5()
            .bg(p.chrome)
            .border_r_1()
            .border_color(p.stroke)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl_1()
                    .child(div().text_size(px(11.)).text_color(p.text_weak).child("OPEN TABS"))
                    .child(new_tab),
            )
            .child(list)
            .child(self.gear_button(cx))
            .child(handle)
    }

    /// Follow the pointer while the tab list is being resized.
    pub(super) fn resize_tab_list(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if self.resizing_tab_list {
            self.settings.tabs_left_width = f32::from(x).clamp(120.0, 480.0);
            cx.notify();
        }
    }

    pub(super) fn end_tab_list_resize(&mut self) {
        if std::mem::take(&mut self.resizing_tab_list) {
            self.settings.save();
        }
    }
}

impl NoteyApp {
    /// The open submenus, each beside its row in the menu or submenu before
    /// it. They are drawn at the window level, outside the menus, which may
    /// scroll.
    pub(super) fn submenu_popups(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(menu) = self.open_menu else { return Vec::new() };
        let mut popups = Vec::new();
        for (i, label) in self.open_submenus.clone().into_iter().enumerate() {
            let level = i + 1;
            let Some(row) = self.submenu_rows.borrow().get(&(i, label)).copied() else { break };
            let Some(items) = self.submenu_items(menu, level) else { break };
            let p = self.palette;
            let bit = 1u32 << level.min(31);
            let panel = self
                .item_list(level, items, cx)
                .bg(p.popup)
                .max_h(px(self.viewport_h - 16.))
                .overflow_y_scroll()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.submenu_hovered |= bit;
                    } else {
                        this.submenu_hovered &= !bit;
                    }
                    cx.notify();
                }));
            let at = gpui::point(row.right(), row.top() - px(5.));
            popups.push(
                deferred(anchored().position(at).snap_to_window_with_margin(px(8.)).child(panel))
                    .with_priority(3 + level)
                    .into_any_element(),
            );
        }
        popups
    }
}

impl NoteyApp {
    /// Do what clicking an item of the menu at `level` does (for screen
    /// readers).
    pub(super) fn run_menu_item(&mut self, item: Item, level: usize, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            Item::Action { action, .. } => {
                self.close_menus();
                self.focus_active(window, cx);
                window.dispatch_action(action, cx);
            }
            Item::Choice { choose, .. } => {
                self.close_menus();
                choose(self, window, cx);
                self.focus_active(window, cx);
            }
            Item::Plugin { script, cmd, enabled: true, .. } => {
                self.close_menus();
                self.focus_active(window, cx);
                self.run_plugin_command(script, &cmd, window, cx);
            }
            Item::Submenu { label, .. } => {
                if self.open_submenus.get(level) == Some(&label) {
                    self.open_submenus.truncate(level);
                } else {
                    self.open_submenu(level, label);
                }
            }
            _ => {}
        }
        cx.notify();
    }
}
