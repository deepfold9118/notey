//! The find and replace bar, and the Go To bar, floating over the top
//! right of the editor. Their text fields are small CodeMirror editors.
//! Whether the find bar is open, and the query, live in the active
//! editor's state (codemirror-search); the bar shows and edits them.

use codemirror_search::{
    close_search_panel, find_next, find_previous, get_search_query, goto_line_selection, open_search_panel,
    replace_all, replace_next, search_panel_open, SearchQuery, SearchQueryConfig, SET_SEARCH_QUERY,
};
use codemirror_state::{ChangeSpec, EditorSelection, StateCommand};
use codemirror_view::{scroll_target, ScrollStrategy, ScrollTarget, Theme, SCROLL_INTO_VIEW};
use gpui::AnyElement;

use super::*;

/// The key context of the bar's text fields, for Enter, Escape and Tab.
pub(super) const FIELD_CONTEXT: &str = "FindField";
/// Matches beyond this many are not counted.
const MAX_COUNTED: usize = 10_000;

actions!(notey, [Find, Replace, FindNext, FindPrevious, GoTo, FieldEnter, FieldShiftEnter, FieldEscape, FieldTab]);

pub(super) fn bindings() -> Vec<GpuiKeyBinding> {
    vec![
        GpuiKeyBinding::new("ctrl-f", Find, None),
        GpuiKeyBinding::new("ctrl-h", Replace, None),
        GpuiKeyBinding::new("f3", FindNext, None),
        GpuiKeyBinding::new("shift-f3", FindPrevious, None),
        GpuiKeyBinding::new("ctrl-g", GoTo, None),
        GpuiKeyBinding::new("enter", FieldEnter, Some(FIELD_CONTEXT)),
        GpuiKeyBinding::new("shift-enter", FieldShiftEnter, Some(FIELD_CONTEXT)),
        GpuiKeyBinding::new("escape", FieldEscape, Some(FIELD_CONTEXT)),
        GpuiKeyBinding::new("tab", FieldTab, Some(FIELD_CONTEXT)),
        GpuiKeyBinding::new("shift-tab", FieldTab, Some(FIELD_CONTEXT)),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QueryOption {
    Case,
    Word,
    Regexp,
}

pub(super) struct FindBar {
    find: Entity<EditorView>,
    replace: Entity<EditorView>,
    goto: Entity<EditorView>,
    show_replace: bool,
    goto_open: bool,
    /// Theme of the fields, reconfigured with the settings.
    config: Compartment,
    /// The field texts last seen, to tell typing from caret blinks.
    seen: (String, String),
    /// Matches of the query in the active document, and the document id,
    /// version and query they were found for.
    matches: Vec<(usize, usize)>,
    matches_for: Option<(u64, u64, SearchQuery)>,
    /// Focus the find field at the next render (the panel was opened from
    /// the editor's keys).
    focus_pending: bool,
    _observers: Vec<Subscription>,
}

fn field_theme(p: &Palette) -> Theme {
    let mut unfocused = p.selection;
    unfocused.a = 0.5;
    Theme {
        font_family: UI_FONT.into(),
        font_size: 13.0,
        line_height: 1.4,
        text: p.text,
        background: p.control,
        selection: p.selection,
        selection_unfocused: unfocused,
        caret: p.caret,
        active_line: gpui::transparent_black(),
        gutter_background: p.control,
        scrollbar: gpui::transparent_black(),
        padding_left: 6.0,
        padding_y: 4.0,
        ..Theme::dark()
    }
}

fn field_text(view: &Entity<EditorView>, cx: &App) -> String {
    view.read(cx).state().doc().to_string()
}

/// Replace a field's text and select it.
fn set_field_text(view: &Entity<EditorView>, text: &str, cx: &mut App) {
    view.update(cx, |view, cx| {
        let len = view.state().doc().len();
        view.dispatch_spec(
            TransactionSpec::new()
                .changes(ChangeSpec::replace(0, len, text.to_string()))
                .selection(EditorSelection::single(0, text.len())),
            cx,
        );
    });
}

fn select_field(view: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    view.update(cx, |view, cx| {
        let len = view.state().doc().len();
        view.dispatch_spec(TransactionSpec::new().selection(EditorSelection::single(0, len)), cx);
        view.focus(window);
    });
}

impl FindBar {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> FindBar {
        let config = Compartment::new();
        let make = |cx: &mut Context<NoteyApp>| {
            let mut bindings = default_keymap();
            bindings.extend(history_keymap());
            let extensions = Extension::list([
                history(HistoryConfig::default()),
                keymap(bindings),
                config.of(THEME.of(Arc::new(field_theme(p)))),
            ]);
            let state = EditorState::create(EditorStateConfig::new().extensions(extensions));
            cx.new(|cx| EditorView::new(state, cx))
        };
        let (find, replace, goto) = (make(cx), make(cx), make(cx));
        let observers = vec![
            cx.observe(&find, |this, _, cx| this.field_changed(cx)),
            cx.observe(&replace, |this, _, cx| this.field_changed(cx)),
            cx.observe(&goto, |_, _, cx| cx.notify()),
        ];
        FindBar {
            find,
            replace,
            goto,
            show_replace: false,
            goto_open: false,
            config,
            seen: Default::default(),
            matches: Vec::new(),
            matches_for: None,
            focus_pending: false,
            _observers: observers,
        }
    }

    /// Restyle the fields for a new palette.
    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        for view in [&self.find, &self.replace, &self.goto] {
            let effect = effect.clone();
            view.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
        }
    }

    fn field_focused(&self, cx: &App) -> bool {
        [&self.find, &self.replace, &self.goto].iter().any(|v| v.read(cx).has_focus())
    }
}

impl NoteyApp {
    /// Whether the find bar shows: the active editor has its search panel open.
    pub(super) fn find_open(&self, cx: &App) -> bool {
        !self.settings_active && self.tabs.get(self.active).is_some_and(|t| search_panel_open(t.state(cx)))
    }

    fn run_on_active(&mut self, cmd: StateCommand, cx: &mut Context<Self>) -> bool {
        if self.settings_active {
            return false;
        }
        match self.tabs.get(self.active) {
            Some(tab) => tab.view.update(cx, |view, cx| view.run_state_command(cmd, cx)),
            None => false,
        }
    }

    /// Ctrl+F and Ctrl+H: open the bar and focus the search field. From the
    /// editor, the selected text becomes the search (CodeMirror's
    /// `openSearchPanel`); from the bar itself, the field is selected.
    fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_active || self.tabs.is_empty() {
            return;
        }
        if !self.find.field_focused(cx) || !self.find_open(cx) {
            self.run_on_active(open_search_panel, cx);
        }
        self.find.show_replace = replace;
        self.find.goto_open = false;
        self.sync_find_fields(cx);
        select_field(&self.find.find, window, cx);
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_on_active(close_search_panel, cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.commit_query(cx);
        self.run_on_active(if forward { find_next } else { find_previous }, cx);
    }

    fn replace_step(&mut self, all: bool, cx: &mut Context<Self>) {
        self.commit_query(cx);
        self.run_on_active(if all { replace_all } else { replace_next }, cx);
    }

    /// A field changed: commit typed text to the query.
    fn field_changed(&mut self, cx: &mut Context<Self>) {
        let texts = (field_text(&self.find.find, cx), field_text(&self.find.replace, cx));
        // A pasted line break becomes `\n`, which a search understands.
        for (view, text) in [(&self.find.find, &texts.0), (&self.find.replace, &texts.1)] {
            if text.contains('\n') {
                set_field_text(view, &text.replace('\n', "\\n"), cx);
                return;
            }
        }
        if texts != self.find.seen {
            self.find.seen = texts;
            self.commit_query(cx);
        }
        cx.notify();
    }

    fn query_with(&self, cx: &App, change: impl FnOnce(&mut SearchQueryConfig)) -> Option<(SearchQuery, SearchQuery)> {
        let tab = self.tabs.get(self.active)?;
        let current = get_search_query(tab.state(cx));
        let mut config = current.config();
        config.search = field_text(&self.find.find, cx);
        config.replace = field_text(&self.find.replace, cx);
        change(&mut config);
        Some((current, SearchQuery::new(config)))
    }

    /// Put the fields' text into the active editor's query.
    fn commit_query(&mut self, cx: &mut Context<Self>) {
        self.set_query(|_| {}, cx);
    }

    fn set_query(&mut self, change: impl FnOnce(&mut SearchQueryConfig), cx: &mut Context<Self>) {
        if !self.find_open(cx) {
            return;
        }
        let Some((current, query)) = self.query_with(cx, change) else { return };
        if query != current {
            self.tabs[self.active].view.update(cx, |view, cx| {
                view.dispatch_spec(TransactionSpec::new().effect(SET_SEARCH_QUERY.of(query)), cx)
            });
        }
    }

    fn toggle_option(&mut self, option: QueryOption, cx: &mut Context<Self>) {
        self.set_query(
            |c| match option {
                QueryOption::Case => c.case_sensitive = !c.case_sensitive,
                QueryOption::Word => c.whole_word = !c.whole_word,
                QueryOption::Regexp => c.regexp = !c.regexp,
            },
            cx,
        );
        cx.notify();
    }

    /// Show the active editor's query in the fields, when it changed
    /// elsewhere (such as from the selection).
    fn sync_find_fields(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let query = get_search_query(tab.state(cx));
        if field_text(&self.find.find, cx) != query.search {
            set_field_text(&self.find.find, &query.search, cx);
        }
        if field_text(&self.find.replace, cx) != query.replace {
            set_field_text(&self.find.replace, &query.replace, cx);
        }
        self.find.seen = (query.search, query.replace);
    }

    /// Called when the active editor's state changed while the bar may be
    /// open: keep the fields and the match count up to date.
    pub(super) fn find_editor_changed(&mut self, was_open: bool, cx: &mut Context<Self>) {
        let open = self.find_open(cx);
        if open && !was_open && !self.find.field_focused(cx) {
            // Opened from the editor's keys (F3 with no query).
            self.find.focus_pending = true;
        }
        if open {
            self.sync_find_fields(cx);
        }
    }

    /// The query matches in the active document, counted up to
    /// `MAX_COUNTED`, and the index of the selected one.
    fn match_info(&mut self, cx: &App) -> Option<(Option<usize>, usize)> {
        let tab = self.tabs.get(self.active)?;
        let state = tab.state(cx);
        let query = get_search_query(state);
        if !query.valid {
            return None;
        }
        let key = (tab.doc.id, tab.version, query);
        if self.find.matches_for.as_ref() != Some(&key) {
            self.find.matches = key.2.get_cursor(state, 0, None).take(MAX_COUNTED + 1).map(|m| (m.from, m.to)).collect();
            self.find.matches_for = Some(key);
        }
        let main = state.selection().main();
        let current = self.find.matches.binary_search(&(main.from, main.to)).ok();
        Some((current, self.find.matches.len()))
    }

    /// Hand the find bar over to the newly active tab: open its search with
    /// the same query.
    pub(super) fn carry_find(&mut self, query: Option<SearchQuery>, cx: &mut Context<Self>) {
        let Some(query) = query else { return };
        if self.find_open(cx) || self.settings_active {
            return;
        }
        self.run_on_active(open_search_panel, cx);
        self.tabs[self.active]
            .view
            .update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(SET_SEARCH_QUERY.of(query)), cx));
        self.sync_find_fields(cx);
    }

    // ---------- Go To ----------

    fn open_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active).filter(|_| !self.settings_active) else { return };
        let line = tab.line.to_string();
        self.run_on_active(close_search_panel, cx);
        self.find.goto_open = true;
        set_field_text(&self.find.goto, &line, cx);
        select_field(&self.find.goto, window, cx);
        cx.notify();
    }

    fn submit_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = field_text(&self.find.goto, cx);
        let Some(tab) = self.tabs.get(self.active) else { return };
        let Some(selection) = goto_line_selection(tab.state(cx), input.trim()) else {
            self.flash(format!("\"{}\" is not a line number", input.trim()), cx);
            return;
        };
        let target = ScrollTarget { y: ScrollStrategy::Center, ..scroll_target(selection.main()) };
        tab.view.update(cx, |view, cx| {
            view.dispatch_spec(TransactionSpec::new().selection(selection).effect(SCROLL_INTO_VIEW.of(target)), cx)
        });
        self.close_goto(window, cx);
    }

    fn close_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.goto_open = false;
        self.focus_active(window, cx);
        cx.notify();
    }

    // ---------- actions and drawing ----------

    pub(super) fn register_find_actions(
        &self,
        root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        root.on_action(cx.listener(|this, _: &Find, window, cx| this.open_find(false, window, cx)))
            .on_action(cx.listener(|this, _: &Replace, window, cx| this.open_find(true, window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_step(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.find_step(false, cx)))
            .on_action(cx.listener(|this, _: &GoTo, window, cx| this.open_goto(window, cx)))
            .on_action(cx.listener(|this, _: &FieldEnter, window, cx| {
                if this.find.goto.read(cx).has_focus() {
                    this.submit_goto(window, cx);
                } else if this.find.replace.read(cx).has_focus() {
                    this.replace_step(false, cx);
                } else {
                    this.find_step(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FieldShiftEnter, _, cx| {
                if this.find.find.read(cx).has_focus() {
                    this.find_step(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FieldEscape, window, cx| {
                if this.find.goto.read(cx).has_focus() {
                    this.close_goto(window, cx);
                } else {
                    this.close_find(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FieldTab, window, cx| {
                if !this.find.show_replace {
                    return;
                }
                let target = if this.find.find.read(cx).has_focus() { &this.find.replace } else { &this.find.find };
                select_field(&target.clone(), window, cx);
            }))
    }

    fn field(&self, id: &'static str, view: &Entity<EditorView>, cx: &App) -> impl IntoElement {
        let p = self.palette;
        let focused = view.read(cx).has_focus();
        div()
            .id(id)
            .key_context(FIELD_CONTEXT)
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .rounded(px(4.))
            .border_1()
            .border_color(p.stroke)
            .when(focused, |d| d.border_b_2().border_color(p.accent))
            .overflow_hidden()
            .child(view.clone())
    }

    fn icon_button(&self, id: &'static str, glyph: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
        let p = self.palette;
        div()
            .id(id)
            .flex_none()
            .h(px(28.))
            .min_w(px(28.))
            .px_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(on, |d| d.bg(p.control_active).text_color(p.accent))
            .hover(|s| s.bg(p.control_hover))
            .child(glyph)
    }

    /// The find bar or the Go To bar, if one is open.
    pub(super) fn find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.find.focus_pending {
            self.find.focus_pending = false;
            select_field(&self.find.find, window, cx);
        }
        let p = self.palette;
        let panel = || {
            div()
                .id("find-bar")
                .occlude()
                .absolute()
                .top(px(8.))
                .right(px(20.))
                .w(px(460.))
                .p_1p5()
                .flex()
                .flex_col()
                .gap_1p5()
                .rounded(px(8.))
                .bg(p.popup)
                .border_1()
                .border_color(p.stroke)
                .shadow_lg()
        };
        if self.find.goto_open {
            let bar = panel()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().px_1().flex_none().child("Go to line"))
                        .child(self.field("goto-field", &self.find.goto, cx))
                        .child(self.text_button("goto-go", "Go").on_click(cx.listener(|this, _, window, cx| this.submit_goto(window, cx))))
                        .child(
                            self.icon_button("goto-close", "\u{E8BB}", false)
                                .font_family(self.icon_font.clone())
                                .text_size(px(10.))
                                .on_click(cx.listener(|this, _, window, cx| this.close_goto(window, cx))),
                        ),
                );
            return Some(bar.into_any_element());
        }
        if !self.find_open(cx) {
            return None;
        }
        let query = get_search_query(self.tabs[self.active].state(cx));
        let count: SharedString = if query.search.is_empty() {
            "".into()
        } else if query.regexp && !query.valid {
            "Invalid pattern".into()
        } else {
            match self.match_info(cx) {
                Some((_, 0)) | None => "No results".into(),
                Some((current, total)) => {
                    let total = if total > MAX_COUNTED { format!("{MAX_COUNTED}+") } else { total.to_string() };
                    match current {
                        Some(i) => format!("{} of {total}", i + 1).into(),
                        None => format!("{total} found").into(),
                    }
                }
            }
        };
        let icon_font = self.icon_font.clone();
        let glyph_button = |this: &Self, id, glyph| {
            this.icon_button(id, glyph, false).font_family(icon_font.clone()).text_size(px(11.))
        };
        let option = |this: &Self, id, label, on, which: QueryOption, cx: &mut Context<Self>| {
            this.icon_button(id, label, on)
                .text_size(px(12.))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_option(which, cx)))
        };
        let chevron = if self.find.show_replace { "\u{E70D}" } else { "\u{E76C}" };
        let find_row = div()
            .flex()
            .items_center()
            .gap_1()
            .child(glyph_button(self, "toggle-replace", chevron).on_click(cx.listener(|this, _, _, cx| {
                this.find.show_replace = !this.find.show_replace;
                cx.notify();
            })))
            .child(self.field("find-field", &self.find.find, cx))
            .child(div().flex_none().min_w(px(64.)).px_1().text_size(px(12.)).text_color(p.text_weak).child(count))
            .child(option(self, "opt-case", "Aa", query.case_sensitive, QueryOption::Case, cx))
            .child(option(self, "opt-word", "W", query.whole_word, QueryOption::Word, cx))
            .child(option(self, "opt-regexp", ".*", query.regexp, QueryOption::Regexp, cx))
            .child(glyph_button(self, "find-prev", "\u{E70E}").on_click(cx.listener(|this, _, _, cx| this.find_step(false, cx))))
            .child(glyph_button(self, "find-next", "\u{E70D}").on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))))
            .child(
                glyph_button(self, "find-close", "\u{E8BB}")
                    .text_size(px(10.))
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            );
        let mut bar = panel().child(find_row);
        if self.find.show_replace {
            bar = bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().w(px(28.)).flex_none())
                    .child(self.field("replace-field", &self.find.replace, cx))
                    .child(self.text_button("replace-one", "Replace").on_click(cx.listener(|this, _, _, cx| this.replace_step(false, cx))))
                    .child(self.text_button("replace-all", "Replace all").on_click(cx.listener(|this, _, _, cx| this.replace_step(true, cx)))),
            );
        }
        Some(bar.into_any_element())
    }

    fn text_button(&self, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
        let p = self.palette;
        div()
            .id(id)
            .flex_none()
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(p.control_hover)
            .border_1()
            .border_color(p.stroke)
            .hover(|s| s.bg(p.control_active))
            .child(label)
    }
}

