//! Spellcheck: loading the dictionaries of the installed Language plugins,
//! and the editor's right-click menu with suggestions.

use std::rc::Rc;

use codemirror_state::{ChangeSpec, EditorSelection};
use gpui::{anchored, deferred, AnchoredPositionMode, MouseDownEvent, Pixels, Point};

use super::chrome::{Item, Menu};
use super::*;
use crate::packages::{Kind, Store};
use crate::spell::{self, LanguageFiles, Spell};
use crate::spellcheck::SpellHandle;

/// The engine that the Spellcheck feature plugin switches on.
const NATIVE_SPELLCHECK: &str = "spellcheck";

/// The editor's right-click menu: where it opened, and the misspelled word
/// under the pointer with its suggestions.
pub(super) struct ContextMenu {
    position: Point<Pixels>,
    doc_id: u64,
    word: Option<(usize, usize, String)>,
    suggestions: Vec<String>,
}

impl NoteyApp {
    /// Load the dictionaries of the enabled Language plugins in the
    /// background, when the Spellcheck plugin is installed and enabled.
    pub(super) fn start_spell_load(&mut self, cx: &mut Context<Self>) {
        let store = Store::open();
        let langs: Vec<LanguageFiles> = if store.native_enabled(NATIVE_SPELLCHECK) {
            store
                .enabled(Kind::Language)
                .filter_map(|p| {
                    let find = |ext: &str| p.manifest.files.iter().find(|f| f.path.ends_with(ext)).map(|f| p.file(&f.path));
                    Some(LanguageFiles { locale: p.manifest.locale.clone()?, aff: find(".aff")?, dic: find(".dic")? })
                })
                .collect()
        } else {
            Vec::new()
        };
        let user_dict = store.root().parent().map(|p| p.join("user_dictionary.txt"));
        let loading = cx.background_executor().spawn(async move { spell::load_languages(&langs) });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let (dicts, errors) = loading.await;
            let _ = this.update(cx, |this, cx| {
                if !errors.is_empty() {
                    this.flash(format!("Could not load a dictionary: {}", errors.join("; ")), cx);
                }
                this.spell = Some(SpellHandle::new(Spell::with_dictionaries(dicts, user_dict)));
                this.update_settings(|_| {}, cx);
            });
        }));
    }

    /// The languages spellcheck uses, for the Settings tab.
    pub(super) fn spell_languages(&self) -> Option<Vec<String>> {
        let spell = self.spell.as_ref()?.spell.read().unwrap();
        Some(spell.locales().into_iter().map(String::from).collect())
    }

    /// Right-click in the editor: open the menu, with suggestions when the
    /// word under the pointer is misspelled.
    pub(super) fn open_context_menu(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let view = tab.view.read(cx);
        let pos = view.pos_at_coords(window, event.position).pos;
        let doc = view.state().doc();
        let line = doc.line_at(pos);
        let mut word = None;
        let mut suggestions = Vec::new();
        if let (true, Some(handle)) = (self.settings.spellcheck, &self.spell) {
            if let Some((a, b)) = spell::word_at(&line.text, pos - line.from) {
                let text = &line.text[a..b];
                let prev = line.text[..a].chars().next_back();
                let checker = handle.spell.read().unwrap();
                if spell::checkable(text, prev) && !checker.check(text) {
                    suggestions = checker.suggest(text);
                    word = Some((line.from + a, line.from + b, text.to_string()));
                }
            }
        }
        self.context_menu = Some(ContextMenu { position: event.position, doc_id: tab.doc.id, word, suggestions });
        self.open_menu = Some(Menu::Context);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn context_menu_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(ContextMenu { word: Some((from, to, word)), suggestions, doc_id, .. }) = &self.context_menu {
            items.push(Item::Heading(format!("\u{201c}{word}\u{201d} is not in the dictionary").into()));
            if suggestions.is_empty() {
                items.push(Item::Heading("No suggestions".into()));
            }
            for s in suggestions {
                let (from, to, doc_id, word, s) = (*from, *to, *doc_id, word.clone(), s.clone());
                items.push(Item::Choice {
                    label: s.clone().into(),
                    checked: None,
                    choose: Rc::new(move |this, _, cx| this.replace_word(doc_id, from, to, &word, &s, cx)),
                });
            }
            let word = word.clone();
            items.push(Item::Choice {
                label: "Add to dictionary".into(),
                checked: None,
                choose: Rc::new(move |this, _, cx| this.add_to_dictionary(&word, cx)),
            });
            items.push(Item::Separator);
        }
        items.extend([
            chrome::action("Cut", "Ctrl+X", Cut),
            chrome::action("Copy", "Ctrl+C", Copy),
            chrome::action("Paste", "Ctrl+V", Paste),
            Item::Separator,
            chrome::action("Select All", "Ctrl+A", SelectAll),
        ]);
        items
    }

    /// Replace the misspelled `word` at `from`-`to` with `with`, if the
    /// text there hasn't changed.
    fn replace_word(&mut self, doc_id: u64, from: usize, to: usize, word: &str, with: &str, cx: &mut Context<Self>) {
        let Some(idx) = self.tab_index(doc_id) else { return };
        self.tabs[idx].view.update(cx, |view, cx| {
            if view.state().slice_doc(from, to) != word {
                return;
            }
            let end = from + with.len();
            view.dispatch_spec(
                TransactionSpec::new()
                    .changes(ChangeSpec::replace(from, to, with.to_string()))
                    .selection(EditorSelection::single(end, end))
                    .user_event("input"),
                cx,
            );
        });
    }

    fn add_to_dictionary(&mut self, word: &str, cx: &mut Context<Self>) {
        if let Some(handle) = &self.spell {
            self.spell = Some(handle.add_word(word));
            self.update_settings(|_| {}, cx);
        }
    }

    /// The right-click menu, at the pointer.
    pub(super) fn context_popup(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.open_menu != Some(Menu::Context) {
            return None;
        }
        let menu = self.context_menu.as_ref()?;
        let popup = anchored()
            .position(menu.position)
            .position_mode(AnchoredPositionMode::Window)
            .snap_to_window_with_margin(px(8.))
            .child(self.menu_panel(Menu::Context, cx));
        Some(deferred(popup).with_priority(1))
    }
}
