//! The engine of Notey's Syntax Check plugin, built to WebAssembly and run
//! by `notey.wasm.run` (see docs/plugin-api.md).
//!
//! `check` takes `{ text, language, path, brackets }` and returns
//! `{ problems: [{ start, end, message, severity }] }`, in character
//! offsets of the text:
//! - JSON, YAML, TOML and XML go through a real parser, so a problem there
//!   is an error: the text is not valid.
//! - Code gets a bracket and quote check (`brackets`): an opening bracket
//!   that never closes, a closing one without an opener, and a string or
//!   comment left open. It knows each language's comments and strings, but
//!   it is not a parser, so its problems are hints: the code might be wrong.

mod brackets;
mod data;

use serde::{Deserialize, Serialize};

/// No more problems than this are reported.
pub const MAX_PROBLEMS: usize = 50;
/// Longer texts are not checked (the check runs as you type).
pub const MAX_TEXT: usize = 4 << 20;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Request {
    text: String,
    language: String,
    path: String,
    brackets: bool,
}

#[derive(Serialize, Default)]
struct Reply {
    problems: Vec<Problem>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Problem {
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub severity: &'static str,
}

/// A problem in byte offsets, before conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub from: usize,
    pub to: usize,
    pub message: String,
    pub error: bool,
}

/// The problems in `text`, in character offsets.
pub fn check(text: &str, language: &str, path: &str, brackets: bool) -> Vec<Problem> {
    if text.len() > MAX_TEXT {
        return Vec::new();
    }
    let language = if language.is_empty() || language == "Plain Text" { by_extension(path) } else { language };
    let mut found = match data::check(text, language, path) {
        Some(found) => found,
        None if brackets => brackets::check(text, language),
        None => Vec::new(),
    };
    found.sort_by_key(|f| f.from);
    found.truncate(MAX_PROBLEMS);
    to_chars(text, found)
}

/// The language of a file by its extension, for a tab whose language
/// Notey doesn't know yet (the grammars load after the file opens).
fn by_extension(path: &str) -> &'static str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "json" | "jsonc" | "json5" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" | "xsd" | "xsl" | "xslt" | "svg" | "csproj" | "vcxproj" | "props" | "targets" | "plist" => "XML",
        "rs" => "Rust",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => "JavaScript",
        "go" => "Go",
        "py" | "pyw" => "Python",
        "lua" => "Lua",
        "css" => "CSS",
        "sql" => "SQL",
        "scala" => "Scala",
        "groovy" | "gradle" => "Groovy",
        "d" => "D",
        "r" => "R",
        "hs" => "Haskell",
        "ml" | "mli" => "OCaml",
        "clj" | "cljs" | "cljc" | "edn" => "Clojure",
        "lisp" | "lsp" | "el" | "cl" => "Lisp",
        "m" => "Objective-C",
        "mm" => "Objective-C++",
        _ => "",
    }
}

/// Convert byte offsets (sorted by start) to character offsets.
fn to_chars(text: &str, found: Vec<Found>) -> Vec<Problem> {
    let char_at = |b: usize| {
        let mut b = b.min(text.len());
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        text[..b].chars().count()
    };
    found
        .into_iter()
        .map(|f| Problem {
            start: char_at(f.from),
            end: char_at(f.to.max(f.from)),
            // the bar shows plain text, so "{" rather than `{`
            message: f.message.replace('`', "\""),
            severity: if f.error { "error" } else { "hint" },
        })
        .collect()
}

fn check_json(input: &[u8]) -> String {
    let reply = match serde_json::from_slice::<Request>(input) {
        Ok(r) => Reply { problems: check(&r.text, &r.language, &r.path, r.brackets) },
        Err(_) => Reply::default(),
    };
    serde_json::to_string(&reply).unwrap_or_default()
}

// ---------- the WebAssembly calling convention ----------

#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from `alloc` or a reply.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: u32) {
    drop(Vec::from_raw_parts(ptr, 0, len as usize));
}

/// # Safety
/// The input must be memory from `alloc`.
#[no_mangle]
pub unsafe extern "C" fn check_text(ptr: *mut u8, len: u32) -> u64 {
    let input = std::slice::from_raw_parts(ptr, len as usize);
    let out = check_json(input).into_bytes().into_boxed_slice();
    let n = out.len() as u64;
    let p = Box::into_raw(out) as *mut u8 as usize as u64;
    (p << 32) | n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_are_characters() {
        let p = check("{\"é\": 1,}", "JSON", "a.json", true);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].severity, "error");
        // the "}" after the comma, at character 8
        assert_eq!(p[0].start, 8);
    }

    #[test]
    fn plain_text_tabs_go_by_the_extension() {
        assert_eq!(check("{\"a\": 1,}", "Plain Text", r"C:\work\a.json", true).len(), 1);
        assert_eq!(check("fn f() {", "Plain Text", "/src/main.rs", true).len(), 1);
        assert_eq!(check("fn f() {", "Plain Text", "notes.txt", true).len(), 0);
    }

    #[test]
    fn the_request_round_trips() {
        let out = check_json(br#"{"text":"(","language":"Rust","path":"","brackets":true}"#);
        assert!(out.contains("never closed"), "{out}");
        let out = check_json(br#"{"text":"(","language":"Rust","path":"","brackets":false}"#);
        assert_eq!(out, r#"{"problems":[]}"#);
    }
}
