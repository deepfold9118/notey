use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use spellbook::Dictionary;

/// Hunspell-based spellchecker with a lookup cache so the per-frame
/// layouter stays cheap.
pub struct Spell {
    dict: Option<Dictionary>,
    cache: RefCell<HashMap<String, bool>>,
    session_words: HashSet<String>,
}

impl Spell {
    pub fn new() -> Self {
        let aff = include_str!("../assets/en_US.aff");
        let dic = include_str!("../assets/en_US.dic");
        Self {
            dict: Dictionary::new(aff, dic).ok(),
            cache: RefCell::new(HashMap::new()),
            session_words: HashSet::new(),
        }
    }

    pub fn available(&self) -> bool {
        self.dict.is_some()
    }

    /// Returns true if the word is spelled correctly (or can't be judged).
    pub fn check(&self, word: &str) -> bool {
        let Some(dict) = &self.dict else { return true };
        if self.session_words.contains(word) {
            return true;
        }
        if let Some(&ok) = self.cache.borrow().get(word) {
            return ok;
        }
        let ok = dict.check(word);
        self.cache.borrow_mut().insert(word.to_string(), ok);
        ok
    }

    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(dict) = &self.dict {
            dict.suggest(word, &mut out);
        }
        out.truncate(7);
        out
    }

    pub fn add_word(&mut self, word: &str) {
        self.session_words.insert(word.to_string());
        self.cache.borrow_mut().remove(word);
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
