//! Syntax highlighting: building the grammar set from the enabled File Type
//! plugins, and the language picker in the status bar.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use super::chrome::Item;
use super::*;
use crate::packages::{Kind, Store};
use crate::syntax;

impl NoteyApp {
    /// Build the grammar set from the enabled File Type plugins in the
    /// background, with an on-disk cache keyed to exactly those plugins
    /// (the same cache the egui version kept).
    pub(super) fn start_syntax_build(&mut self, cx: &mut Context<Self>) {
        let store = Store::open();
        let mut grammars = Vec::new();
        let mut hasher = DefaultHasher::new();
        let mut enabled: Vec<_> = store.enabled(Kind::Filetype).collect();
        enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        for p in &enabled {
            (&p.manifest.id, &p.manifest.version).hash(&mut hasher);
            for f in &p.manifest.files {
                if f.path.ends_with(".sublime-syntax") {
                    f.sha256.hash(&mut hasher);
                    grammars.push(p.file(&f.path));
                }
            }
        }
        let cache_dir = store.root().join(".cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let cache = cache_dir.join(format!("syntaxes-{:016x}.bin", hasher.finish()));
        // Drop the caches of other plugin combinations.
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for e in entries.filter_map(Result::ok) {
                if e.path() != cache && e.file_name().to_string_lossy().starts_with("syntaxes-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let building = cx.background_executor().spawn(async move {
            let cache = if grammars.is_empty() { None } else { Some(cache) };
            syntax::build(&grammars, cache.as_deref())
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let (set, errors) = building.await;
            syntax::install(set);
            let _ = this.update(cx, |this, cx| {
                if let Some(e) = errors.first() {
                    this.flash(format!("A file type failed to load: {e}"), cx);
                }
                // Files opened before their file type was available.
                for tab in &mut this.tabs {
                    if tab.doc.language.is_none() {
                        if let Some(path) = &tab.doc.path {
                            tab.doc.language = syntax::detect(path);
                        }
                    }
                }
                this.update_settings(|_| {}, cx);
            });
        }));
    }

    /// The language picker: Plain Text and the installed file types.
    pub(super) fn language_menu_items(&self) -> Vec<Item> {
        let current = self.tabs[self.active].doc.language.clone();
        let names = syntax::global().names.clone();
        std::iter::once(None)
            .chain(names.into_iter().map(Some))
            .map(|language| Item::Choice {
                label: language.clone().unwrap_or_else(|| "Plain Text".into()).into(),
                checked: Some(language == current),
                choose: Rc::new(move |this, _, cx| this.set_language(language.clone(), cx)),
            })
            .collect()
    }

    fn set_language(&mut self, language: Option<String>, cx: &mut Context<Self>) {
        let idx = self.active;
        self.tabs[idx].doc.language = language;
        self.reconfigure_tab(idx, cx);
        self.session_dirty = true;
        cx.notify();
    }
}
