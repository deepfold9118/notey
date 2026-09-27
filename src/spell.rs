use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use spellbook::Dictionary;

/// Hunspell dictionary files from one enabled Language plugin.
pub struct LanguageFiles {
    pub locale: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
}

/// Load dictionaries (slow for large languages; call off the UI thread).
/// Returns the loaded dictionaries plus a message per language that failed.
pub fn load_languages(langs: &[LanguageFiles]) -> (Vec<(String, Dictionary)>, Vec<String>) {
    let mut dicts = Vec::new();
    let mut errors = Vec::new();
    for l in langs {
        let read = |p: &Path| {
            std::fs::read_to_string(p)
                .map(|s| s.trim_start_matches('\u{feff}').to_string())
                .map_err(|e| e.to_string())
        };
        match read(&l.aff).and_then(|aff| read(&l.dic).map(|dic| (aff, dic))) {
            Ok((aff, dic)) => match Dictionary::new(&aff, &dic) {
                Ok(d) => dicts.push((l.locale.clone(), d)),
                Err(e) => errors.push(format!("{}: {e:?}", l.locale)),
            },
            Err(e) => errors.push(format!("{}: {e}", l.locale)),
        }
    }
    (dicts, errors)
}

/// Spellchecker over every enabled language: a word is correct if any
/// language accepts it. Keeps a lookup cache so the per-frame layouter
/// stays cheap, and a personal word list that persists on disk.
pub struct Spell {
    dicts: Vec<(String, Dictionary)>,
    cache: RefCell<HashMap<String, bool>>,
    user_words: HashSet<String>,
    user_dict_path: Option<PathBuf>,
}

impl Spell {
    /// No languages: spellcheck is unavailable until plugins provide some.
    pub fn empty() -> Self {
        Self::with_dictionaries(Vec::new(), None)
    }

    pub fn with_dictionaries(dicts: Vec<(String, Dictionary)>, user_dict_path: Option<PathBuf>) -> Self {
        let user_words = user_dict_path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|w| !w.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            dicts,
            cache: RefCell::new(HashMap::new()),
            user_words,
            user_dict_path,
        }
    }

    pub fn available(&self) -> bool {
        !self.dicts.is_empty()
    }

    pub fn locales(&self) -> Vec<&str> {
        self.dicts.iter().map(|(l, _)| l.as_str()).collect()
    }

    /// Returns true if the word is spelled correctly (or can't be judged).
    pub fn check(&self, word: &str) -> bool {
        if self.dicts.is_empty() || self.user_words.contains(word) {
            return true;
        }
        if let Some(&ok) = self.cache.borrow().get(word) {
            return ok;
        }
        let ok = self.dicts.iter().any(|(_, d)| d.check(word));
        self.cache.borrow_mut().insert(word.to_string(), ok);
        ok
    }

    /// Suggestions from every language, best matches first, deduplicated.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, d) in &self.dicts {
            let mut s = Vec::new();
            d.suggest(word, &mut s);
            for w in s {
                if !out.contains(&w) {
                    out.push(w);
                }
            }
        }
        out.truncate(7);
        out
    }

    /// Accept a word from now on; saved to the personal word list.
    pub fn add_word(&mut self, word: &str) {
        if !self.user_words.insert(word.to_string()) {
            return;
        }
        self.cache.borrow_mut().remove(word);
        if let Some(path) = &self.user_dict_path {
            let mut words: Vec<&String> = self.user_words.iter().collect();
            words.sort();
            let text: String = words.iter().map(|w| format!("{w}\n")).collect();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, text);
        }
    }
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '\u{2019}'
}

/// Should this token be spellchecked at all?
pub fn checkable(token: &str, prev_char: Option<char>) -> bool {
    if token.chars().count() < 2 {
        return false;
    }
    if token.chars().any(|c| c.is_numeric()) {
        return false;
    }
    // crude URL/email guard
    if matches!(prev_char, Some('@') | Some('/') | Some(':') | Some('.')) {
        return false;
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn official(code: &str, id: &str) -> LanguageFiles {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/languages").join(id);
        LanguageFiles {
            locale: code.into(),
            aff: dir.join(format!("{code}.aff")),
            dic: dir.join(format!("{code}.dic")),
        }
    }

    /// Spell over the repo's official English (US) language plugin.
    pub(crate) fn english() -> Spell {
        let (dicts, errors) = load_languages(&[official("en_US", "lang-en-us")]);
        assert!(errors.is_empty(), "{errors:?}");
        Spell::with_dictionaries(dicts, None)
    }

    #[test]
    fn no_languages_means_nothing_is_flagged() {
        let s = Spell::empty();
        assert!(!s.available());
        assert!(s.check("qwzxv"));
    }

    #[test]
    fn any_enabled_language_accepts_a_word() {
        let (dicts, errors) =
            load_languages(&[official("en_US", "lang-en-us"), official("es_ES", "lang-es-es")]);
        assert!(errors.is_empty(), "{errors:?}");
        let s = Spell::with_dictionaries(dicts, None);
        assert_eq!(s.locales(), ["en_US", "es_ES"]);
        assert!(s.check("house"));
        assert!(s.check("casa")); // Spanish
        assert!(!s.check("recieve"));
        assert!(s.suggest("recieve").iter().any(|w| w == "receive"));
    }

    #[test]
    fn personal_words_persist() {
        let path = std::env::temp_dir().join(format!("notey-userdict-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut s = Spell::with_dictionaries(english().dicts, Some(path.clone()));
        assert!(!s.check("Notey"));
        s.add_word("Notey");
        assert!(s.check("Notey"));
        let reloaded = Spell::with_dictionaries(english().dicts, Some(path.clone()));
        assert!(reloaded.check("Notey"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn every_official_language_loads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/languages");
        for dir in std::fs::read_dir(root).unwrap().filter_map(Result::ok) {
            let m: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.path().join("plugin.json")).unwrap()).unwrap();
            let code = m["locale"].as_str().unwrap();
            let id = m["id"].as_str().unwrap();
            let (dicts, errors) = load_languages(&[official(code, id)]);
            assert!(errors.is_empty() && dicts.len() == 1, "{id}: {errors:?}");
        }
    }
}
