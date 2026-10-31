//! The accessibility tree of the window (see `a11y.rs`): the title bar
//! with the tabs and caption buttons, the menu bar and the open menu, the
//! editor as a multiline text input (its text, a run per line and the
//! selection), the find bar, the Settings tab's sections and controls, the
//! status bar, and an open dialog. Screen reader actions (click, focus,
//! select text, step a value) come back through `a11y_request`.

use std::rc::Rc;

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection, Toggled, Tree, TreeId,
    TreeUpdate,
};
use codemirror_state::EditorSelection;

use super::chrome::{Item, Menu};
use super::settings_tab::Section;
use super::*;
use crate::a11y::Request;

const ROOT: NodeId = NodeId(1);
const TITLE_BAR: NodeId = NodeId(2);
const TAB_LIST: NodeId = NodeId(3);
const NEW_TAB: NodeId = NodeId(4);
const MINIMIZE: NodeId = NodeId(5);
const MAXIMIZE: NodeId = NodeId(6);
const CLOSE: NodeId = NodeId(7);
const MENU_BAR: NodeId = NodeId(8);
const MENU_FILE: NodeId = NodeId(9);
const MENU_EDIT: NodeId = NodeId(10);
const MENU_HELP: NodeId = NodeId(11);
const OPEN_MENU: NodeId = NodeId(12);
const EDITOR: NodeId = NodeId(20);
const FIND_BAR: NodeId = NodeId(21);
const FIND_FIELD: NodeId = NodeId(22);
const REPLACE_FIELD: NodeId = NodeId(23);
const STATUS_BAR: NodeId = NodeId(30);
const GEAR: NodeId = NodeId(31);
const SETTINGS: NodeId = NodeId(50);
const SETTINGS_TAB: NodeId = NodeId(60);
const DIALOG: NodeId = NodeId(70);
const DIALOG_TEXT: NodeId = NodeId(71);
/// Ranges of ids for lists.
const MENU_ITEM: u64 = 100;
const FIND_BUTTON: u64 = 200;
const STATUS_ITEM: u64 = 300;
const SECTION: u64 = 400;
const CONTROL: u64 = 500;
const DIALOG_BUTTON: u64 = 700;
/// The items of open submenus: a block of `SUBMENU_LEVEL` ids per level.
const SUBMENU_ITEM: u64 = 2_000;
const SUBMENU_LEVEL: u64 = 1_000;
const SUBMENU_LEVELS: usize = 1 + ((TAB - SUBMENU_ITEM) / SUBMENU_LEVEL) as usize;
const TAB: u64 = 10_000;
const TEXT_RUN: u64 = 1_000_000;

/// Documents longer than this get their text, but no per-line runs.
const MAX_RUN_CHARS: usize = 200_000;

/// An action a screen reader can take on a Settings control.
type Run = Rc<dyn Fn(&mut NoteyApp, &mut Window, &mut Context<NoteyApp>)>;

/// A Settings control, as the accessibility tree shows it.
pub(super) enum Control {
    Toggle { label: SharedString, on: bool, flip: Run },
    Stepper { label: SharedString, value: SharedString, down: Run, up: Run },
    Text(SharedString),
}

fn node(role: Role, label: impl Into<String>) -> Node {
    let mut n = Node::new(role);
    let label: String = label.into();
    if label.is_empty() {
        return n;
    }
    // AccessKit names a text label by its value.
    if role == Role::Label {
        n.set_value(label);
    } else {
        n.set_label(label);
    }
    n
}

fn button(label: impl Into<String>) -> Node {
    let mut n = node(Role::Button, label);
    n.add_action(Action::Click);
    n
}

impl NoteyApp {
    /// Send the tree, if a screen reader is reading the window.
    pub(super) fn update_accessibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut a11y) = self.a11y.take() else { return };
        a11y.update(|| self.accessibility_tree(window, cx));
        self.a11y = Some(a11y);
    }

    fn accessibility_tree(&self, window: &Window, cx: &App) -> TreeUpdate {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        let mut root = node(Role::Window, self.last_title.clone());
        let mut focus = ROOT;

        // Title bar: tabs, new tab, caption buttons.
        let mut tab_list = node(Role::TabList, "Tabs");
        if !self.settings.tabs_left {
            let mut title_bar = node(Role::TitleBar, "");
            title_bar.set_children(vec![TAB_LIST, NEW_TAB, MINIMIZE, MAXIMIZE, CLOSE]);
            root.push_child(TITLE_BAR);
            nodes.push((TITLE_BAR, title_bar));
        } else {
            root.push_child(TAB_LIST);
            root.push_child(NEW_TAB);
        }
        for (i, tab) in self.tabs.iter().enumerate() {
            let id = NodeId(TAB + tab.doc.id);
            let title = if tab.modified { format!("{} (modified)", tab.doc.title()) } else { tab.doc.title() };
            let mut n = node(Role::Tab, title);
            n.set_selected(!self.settings_active && i == self.active);
            n.add_action(Action::Click);
            tab_list.push_child(id);
            nodes.push((id, n));
        }
        if self.settings_open {
            let mut n = node(Role::Tab, "Settings");
            n.set_selected(self.settings_active);
            n.add_action(Action::Click);
            tab_list.push_child(SETTINGS_TAB);
            nodes.push((SETTINGS_TAB, n));
        }
        nodes.push((TAB_LIST, tab_list));
        nodes.push((NEW_TAB, button("New tab")));
        nodes.push((MINIMIZE, button("Minimize")));
        nodes.push((MAXIMIZE, button(if window.is_maximized() { "Restore" } else { "Maximize" })));
        nodes.push((CLOSE, button("Close")));
        if self.settings.tabs_left {
            root.push_child(MINIMIZE);
            root.push_child(MAXIMIZE);
            root.push_child(CLOSE);
        }

        // Menu bar and the open menu.
        let mut menu_bar = node(Role::MenuBar, "");
        for (id, label, menu) in [(MENU_FILE, "File", Menu::File), (MENU_EDIT, "Edit", Menu::Edit), (MENU_HELP, "Help", Menu::Help)] {
            let mut n = node(Role::MenuItem, label);
            n.add_action(Action::Click);
            n.set_expanded(self.open_menu == Some(menu));
            menu_bar.push_child(id);
            nodes.push((id, n));
        }
        if self.settings.show_menu {
            root.push_child(MENU_BAR);
        } else {
            menu_bar.set_label("Menus (Notey icon)");
            root.push_child(MENU_BAR);
        }
        nodes.push((MENU_BAR, menu_bar));
        if let Some(menu) = self.open_menu.filter(|m| *m != Menu::Language) {
            let mut list = node(Role::Menu, "");
            self.menu_nodes(0, self.menu_items(menu), &mut list, &mut nodes);
            root.push_child(OPEN_MENU);
            nodes.push((OPEN_MENU, list));
        }

        // The document area.
        if self.settings_active {
            root.push_child(SETTINGS);
            nodes.extend(self.settings_nodes());
        } else if let Some(tab) = self.tabs.get(self.active) {
            root.push_child(EDITOR);
            let editor_focused = tab.view.read(cx).has_focus();
            nodes.extend(self.editor_nodes(tab, cx));
            if editor_focused {
                focus = EDITOR;
            }
            if self.find_open(cx) {
                root.push_child(FIND_BAR);
                let mut bar = node(Role::Group, "Find and Replace");
                for (id, label, view) in [(FIND_FIELD, "Find", &self.find.find), (REPLACE_FIELD, "Replace", &self.find.replace)] {
                    let mut n = node(Role::TextInput, label);
                    n.set_value(view.read(cx).state().doc().to_string());
                    n.add_action(Action::Focus);
                    if view.read(cx).has_focus() {
                        focus = id;
                    }
                    bar.push_child(id);
                    nodes.push((id, n));
                }
                for (i, label) in ["Previous match", "Next match", "Replace", "Replace all", "Close"].iter().enumerate() {
                    let id = NodeId(FIND_BUTTON + i as u64);
                    bar.push_child(id);
                    nodes.push((id, button(*label)));
                }
                nodes.push((FIND_BAR, bar));
            }
        }

        // Status bar.
        if self.settings.show_status {
            let mut status = node(Role::Status, "");
            let mut texts: Vec<String> = Vec::new();
            if let Some(msg) = &self.status_msg {
                texts.push(msg.to_string());
            }
            if let (false, Some(tab)) = (self.settings_active, self.tabs.get(self.active)) {
                texts.push(format!("Line {}, column {}", tab.line, tab.col));
                texts.extend(self.scripting.status_items.iter().map(|(_, t, _)| t.clone()));
                texts.push(format!("{} characters", tab.chars));
                texts.push(format!("Zoom {:.0}%", self.settings.zoom));
                texts.push(tab.doc.language.clone().unwrap_or_else(|| "Plain Text".into()));
                texts.push(tab.doc.line_ending.label().into());
                texts.push(tab.doc.encoding.label().into());
            }
            for (i, t) in texts.into_iter().enumerate() {
                let id = NodeId(STATUS_ITEM + i as u64);
                status.push_child(id);
                nodes.push((id, node(Role::Label, t)));
            }
            root.push_child(STATUS_BAR);
            nodes.push((STATUS_BAR, status));
        }
        root.push_child(GEAR);
        nodes.push((GEAR, button("Settings")));

        // An open dialog takes the focus.
        if let Some((message, detail, buttons)) = crate::prompt::current(cx) {
            let mut dialog = node(Role::AlertDialog, message.to_string());
            if let Some(detail) = &detail {
                dialog.set_description(detail.to_string());
                dialog.push_child(DIALOG_TEXT);
                nodes.push((DIALOG_TEXT, node(Role::Label, detail.to_string())));
            }
            for (i, b) in buttons.iter().enumerate() {
                let id = NodeId(DIALOG_BUTTON + i as u64);
                dialog.push_child(id);
                nodes.push((id, button(b.to_string())));
            }
            root.push_child(DIALOG);
            nodes.push((DIALOG, dialog));
            focus = NodeId(DIALOG_BUTTON);
        }

        nodes.push((ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree { root: ROOT, toolkit_name: Some("Notey".into()), toolkit_version: Some(env!("CARGO_PKG_VERSION").into()) }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    /// The editor: its text, a run per line, and the main selection.
    fn editor_nodes(&self, tab: &Tab, cx: &App) -> Vec<(NodeId, Node)> {
        let state = tab.state(cx);
        let doc = state.doc();
        let mut editor = node(Role::MultilineTextInput, tab.doc.title());
        editor.add_action(Action::Focus);
        editor.add_action(Action::SetTextSelection);
        let text = doc.to_string();
        let mut nodes = Vec::new();
        if tab.chars <= MAX_RUN_CHARS {
            let main = state.selection().main();
            let position = |pos: usize| {
                let line = doc.line_at(pos);
                TextPosition {
                    node: NodeId(TEXT_RUN + line.number as u64),
                    character_index: line.text[..pos - line.from].chars().count(),
                }
            };
            for n in 1..=doc.lines() {
                let line = doc.line(n);
                let mut value = line.text.clone();
                if n < doc.lines() {
                    value.push('\n');
                }
                let mut run = Node::new(Role::TextRun);
                run.set_character_lengths(value.chars().map(|c| c.len_utf8() as u8).collect::<Vec<u8>>());
                run.set_value(value);
                let id = NodeId(TEXT_RUN + n as u64);
                editor.push_child(id);
                nodes.push((id, run));
            }
            editor.set_text_selection(TextSelection { anchor: position(main.anchor()), focus: position(main.head()) });
        }
        editor.set_value(text);
        nodes.push((EDITOR, editor));
        nodes
    }

    fn settings_nodes(&self) -> Vec<(NodeId, Node)> {
        let mut nodes = Vec::new();
        let mut settings = node(Role::TabPanel, "Settings");
        for (i, s) in [Section::Editor, Section::Appearance, Section::Spellcheck, Section::Plugins].iter().enumerate() {
            let id = NodeId(SECTION + i as u64);
            let mut n = node(Role::Tab, s.label());
            n.set_selected(self.settings_ui.section == *s);
            n.add_action(Action::Click);
            settings.push_child(id);
            nodes.push((id, n));
        }
        for (i, control) in self.settings_controls().into_iter().enumerate() {
            let id = NodeId(CONTROL + i as u64);
            let n = match control {
                Control::Toggle { label, on, .. } => {
                    let mut n = node(Role::CheckBox, label.to_string());
                    n.set_toggled(Toggled::from(on));
                    n.add_action(Action::Click);
                    n
                }
                Control::Stepper { label, value, .. } => {
                    let mut n = node(Role::SpinButton, label.to_string());
                    n.set_value(value.to_string());
                    n.add_action(Action::Increment);
                    n.add_action(Action::Decrement);
                    n
                }
                Control::Text(t) => node(Role::Label, t.to_string()),
            };
            settings.push_child(id);
            nodes.push((id, n));
        }
        nodes.push((SETTINGS, settings));
        nodes
    }

    /// The controls of the visible Settings section.
    pub(super) fn settings_controls(&self) -> Vec<Control> {
        let s = &self.settings;
        let toggle = |label: &str, on: bool, flip: fn(&mut Settings)| Control::Toggle {
            label: label.to_string().into(),
            on,
            flip: Rc::new(move |this: &mut NoteyApp, _: &mut Window, cx: &mut Context<NoteyApp>| this.update_settings(flip, cx)),
        };
        let stepper = |label: &str, value: String, down: fn(&mut Settings), up: fn(&mut Settings)| Control::Stepper {
            label: label.to_string().into(),
            value: value.into(),
            down: Rc::new(move |this: &mut NoteyApp, _: &mut Window, cx: &mut Context<NoteyApp>| this.update_settings(down, cx)),
            up: Rc::new(move |this: &mut NoteyApp, _: &mut Window, cx: &mut Context<NoteyApp>| this.update_settings(up, cx)),
        };
        match self.settings_ui.section {
            Section::Editor => vec![
                Control::Text(format!("Font: {}", s.editor_font).into()),
                stepper("Font size", format!("{:.0} pixels", s.font_size), |s| s.font_size = (s.font_size - 1.0).max(6.0), |s| {
                    s.font_size = (s.font_size + 1.0).min(72.0)
                }),
                stepper(
                    "Line height",
                    format!("{:.2}", s.line_height),
                    |s| s.line_height = ((s.line_height - 0.05) * 20.0).round().max(16.0) / 20.0,
                    |s| s.line_height = ((s.line_height + 0.05) * 20.0).round().min(60.0) / 20.0,
                ),
                stepper("Tab size", format!("{} spaces", s.tab_size), |s| s.tab_size = (s.tab_size - 1).max(1), |s| {
                    s.tab_size = (s.tab_size + 1).min(16)
                }),
                toggle("Word wrap", s.word_wrap, |s| s.word_wrap = !s.word_wrap),
                toggle("Line numbers", s.line_numbers, |s| s.line_numbers = !s.line_numbers),
                toggle("Ligatures", s.ligatures, |s| s.ligatures = !s.ligatures),
                toggle("Autosave", s.autosave, |s| s.autosave = !s.autosave),
                stepper(
                    "Autosave interval",
                    format!("{:.0} seconds", s.autosave_secs),
                    |s| s.autosave_secs = (s.autosave_secs - 5.0).max(5.0),
                    |s| s.autosave_secs = (s.autosave_secs + 5.0).min(600.0),
                ),
            ],
            Section::Appearance => {
                let builtin = s.theme_path.is_none();
                vec![
                    Control::Toggle {
                        label: "Notey Dark theme".into(),
                        on: builtin && s.dark,
                        flip: Rc::new(|this, _, cx| this.use_builtin_theme(true, cx)),
                    },
                    Control::Toggle {
                        label: "Notey Light theme".into(),
                        on: builtin && !s.dark,
                        flip: Rc::new(|this, _, cx| this.use_builtin_theme(false, cx)),
                    },
                    Control::Text(
                        format!("Theme: {}", self.custom_theme.as_ref().map_or("Notey", |t| t.name.as_str())).into(),
                    ),
                    toggle("Tabs on the left", s.tabs_left, |s| s.tabs_left = !s.tabs_left),
                    toggle("Show menu bar", s.show_menu, |s| s.show_menu = !s.show_menu),
                    toggle("Show status bar", s.show_status, |s| s.show_status = !s.show_status),
                ]
            }
            Section::Spellcheck => {
                let mut v = vec![toggle("Check spelling while you type", s.spellcheck, |s| s.spellcheck = !s.spellcheck)];
                let langs = self.spell_languages().unwrap_or_default();
                v.push(Control::Text(format!("Languages: {}", if langs.is_empty() { "none".into() } else { langs.join(", ") }).into()));
                v
            }
            Section::Plugins => self
                .plugins
                .store
                .installed
                .iter()
                .map(|p| {
                    let id = p.manifest.id.clone();
                    let on = self.plugins.store.is_enabled(&id);
                    Control::Toggle {
                        label: format!("{} ({})", p.manifest.name, p.manifest.kind.label()).into(),
                        on,
                        flip: Rc::new(move |this, _, cx| this.set_plugin_enabled(&id, !on, cx)),
                    }
                })
                .collect(),
        }
    }

    /// Something a screen reader asked for.
    pub(super) fn a11y_request(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        let Request::Action(ActionRequest { action, target_node, data, .. }) = request else {
            // Activation: the next render sends the full tree.
            cx.notify();
            return;
        };
        let id = target_node.0;
        match (target_node, action) {
            (_, Action::Click) if (TAB..TEXT_RUN).contains(&id) => {
                if let Some(i) = self.tab_index(id - TAB) {
                    self.activate(i, window, cx);
                }
            }
            (SETTINGS_TAB | GEAR, Action::Click) => self.open_settings(window, cx),
            (NEW_TAB, Action::Click) => self.new_tab(window, cx),
            (MINIMIZE, Action::Click) => window.minimize_window(),
            (MAXIMIZE, Action::Click) => window.zoom_window(),
            (CLOSE, Action::Click) => window.remove_window(),
            (MENU_FILE | MENU_EDIT | MENU_HELP, Action::Click) => {
                let menu = [Menu::File, Menu::Edit, Menu::Help][(id - MENU_FILE.0) as usize];
                self.open_menu = if self.open_menu == Some(menu) { None } else { Some(menu) };
            }
            (_, Action::Click) if menu_item_at(id).is_some() => {
                let (level, i) = menu_item_at(id).unwrap_or_default();
                let item = self.open_menu.and_then(|menu| self.submenu_items(menu, level)).and_then(|items| items.into_iter().nth(i));
                if let Some(item) = item {
                    self.run_menu_item(item, level, window, cx);
                }
            }
            (EDITOR, Action::Focus) => self.focus_active(window, cx),
            (EDITOR, Action::SetTextSelection) => {
                if let Some(ActionData::SetTextSelection(sel)) = data {
                    self.select_from_a11y(sel, window, cx);
                }
            }
            (FIND_FIELD, Action::Focus) => find::select_field(&self.find.find.clone(), window, cx),
            (REPLACE_FIELD, Action::Focus) => find::select_field(&self.find.replace.clone(), window, cx),
            (_, Action::Click) if (FIND_BUTTON..STATUS_ITEM).contains(&id) => {
                let action: Box<dyn gpui::Action> = match id - FIND_BUTTON {
                    0 => Box::new(find::FindPrevious),
                    1 => Box::new(find::FindNext),
                    _ => Box::new(find::FieldEscape),
                };
                match id - FIND_BUTTON {
                    2 => self.replace_step(false, cx),
                    3 => self.replace_step(true, cx),
                    _ => window.dispatch_action(action, cx),
                }
            }
            (_, Action::Click) if (SECTION..CONTROL).contains(&id) => {
                let sections = [Section::Editor, Section::Appearance, Section::Spellcheck, Section::Plugins];
                if let Some(s) = sections.get((id - SECTION) as usize) {
                    self.settings_ui.section = *s;
                }
            }
            (_, action) if (CONTROL..DIALOG_BUTTON).contains(&id) => {
                if let Some(control) = self.settings_controls().into_iter().nth((id - CONTROL) as usize) {
                    match (control, action) {
                        (Control::Toggle { flip, .. }, Action::Click) => flip(self, window, cx),
                        (Control::Stepper { up, .. }, Action::Increment) => up(self, window, cx),
                        (Control::Stepper { down, .. }, Action::Decrement) => down(self, window, cx),
                        _ => {}
                    }
                }
            }
            (_, Action::Click) if (DIALOG_BUTTON..TAB).contains(&id) => crate::prompt::answer((id - DIALOG_BUTTON) as usize, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Select text in the editor at a screen reader's request.
    fn select_from_a11y(&mut self, sel: TextSelection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let doc = tab.state(cx).doc();
        let offset = |p: TextPosition| -> Option<usize> {
            let n = p.node.0.checked_sub(TEXT_RUN)? as usize;
            if n == 0 || n > doc.lines() {
                return None;
            }
            let line = doc.line(n);
            let bytes: usize = line.text.chars().take(p.character_index).map(char::len_utf8).sum();
            Some(line.from + bytes.min(line.length()))
        };
        let (Some(anchor), Some(head)) = (offset(sel.anchor), offset(sel.focus)) else { return };
        tab.view.update(cx, |view, cx| {
            view.dispatch_spec(TransactionSpec::new().selection(EditorSelection::single(anchor, head)).scroll_into_view(), cx)
        });
        self.focus_active(window, cx);
    }
}

/// The node id of item `i` of the menu at `level` (0 for the menu, 1 for
/// its open submenu, ...).
fn menu_item_id(level: usize, i: usize) -> NodeId {
    if level == 0 {
        NodeId(MENU_ITEM + i as u64)
    } else {
        NodeId(SUBMENU_ITEM + (level as u64 - 1) * SUBMENU_LEVEL + i as u64)
    }
}

/// The level and index of a menu item node, the reverse of `menu_item_id`.
fn menu_item_at(id: u64) -> Option<(usize, usize)> {
    if (MENU_ITEM..FIND_BUTTON).contains(&id) {
        Some((0, (id - MENU_ITEM) as usize))
    } else if (SUBMENU_ITEM..TAB).contains(&id) {
        let n = id - SUBMENU_ITEM;
        Some(((n / SUBMENU_LEVEL) as usize + 1, (n % SUBMENU_LEVEL) as usize))
    } else {
        None
    }
}

impl NoteyApp {
    /// The nodes of `items`, the menu at `level` of the open menu, as
    /// children of `parent`. An open submenu's items are its children.
    fn menu_nodes(&self, level: usize, items: Vec<Item>, parent: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        for (i, item) in items.into_iter().enumerate() {
            let id = menu_item_id(level, i);
            let n = match item {
                Item::Action { label, shortcut, .. } => {
                    let mut n = node(Role::MenuItem, label.to_string());
                    if !shortcut.is_empty() {
                        n.set_keyboard_shortcut(shortcut);
                    }
                    n.add_action(Action::Click);
                    n
                }
                Item::Choice { label, checked, .. } => {
                    let mut n = node(if checked.is_some() { Role::MenuItemRadio } else { Role::MenuItem }, label.to_string());
                    if let Some(c) = checked {
                        n.set_toggled(Toggled::from(c));
                    }
                    n.add_action(Action::Click);
                    n
                }
                Item::Plugin { label, shortcut, enabled, .. } => {
                    let mut n = node(Role::MenuItem, label.to_string());
                    if !shortcut.is_empty() {
                        n.set_keyboard_shortcut(shortcut.to_string());
                    }
                    if enabled {
                        n.add_action(Action::Click);
                    } else {
                        n.set_disabled();
                    }
                    n
                }
                Item::Submenu { label, items } => {
                    let open = self.open_submenus.get(level) == Some(&label);
                    let mut n = node(Role::MenuItem, label.to_string());
                    n.set_expanded(open);
                    n.add_action(Action::Click);
                    if open && level + 1 < SUBMENU_LEVELS {
                        self.menu_nodes(level + 1, items, &mut n, nodes);
                    }
                    n
                }
                Item::Heading(label) => node(Role::Label, label.to_string()),
                Item::Separator => node(Role::Splitter, ""),
            };
            parent.push_child(id);
            nodes.push((id, n));
        }
    }
}
