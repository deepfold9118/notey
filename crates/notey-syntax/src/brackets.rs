//! Unmatched brackets and quotes in code. The scanner skips each
//! language's comments and strings, so brackets inside them don't count,
//! and reports:
//! - an opening bracket that is never closed, or a closing one that has no
//!   opener or closes the wrong kind;
//! - a string that runs past the end of its line, where the language's
//!   strings can't, and a string or comment still open at the end.
//!
//! It is not a parser, so the problems are hints.

use crate::Found;

/// A kind of string.
struct Quote {
    open: &'static str,
    close: &'static str,
    /// A backslash escapes the next character.
    escape: bool,
    /// The string can span lines.
    multiline: bool,
}

/// A `Quote` literal (so the lists below are constants).
macro_rules! q {
    ($open:expr, $close:expr, $escape:expr, $multiline:expr) => {
        Quote { open: $open, close: $close, escape: $escape, multiline: $multiline }
    };
}

/// What the scanner knows about a language.
struct Lang {
    line_comments: &'static [&'static str],
    block_comments: &'static [(&'static str, &'static str)],
    /// Block comments nest (Haskell, OCaml, D's `/+ +/`, Lisp).
    nested: bool,
    /// The strings, longest opening first.
    quotes: &'static [Quote],
    /// `'` is a character literal only when it closes right away (`'a'`,
    /// `'\n'`); otherwise it is a lifetime, a prime or a digit separator.
    char_quote: bool,
    /// A backslash outside strings is a character literal (`\(` in Lisp).
    char_backslash: bool,
    /// `/.../` regular expression literals (JavaScript).
    regex: bool,
    /// Rust's raw strings, `r#"..."#`.
    rust_raw: bool,
    /// C++'s raw strings, `R"x(...)x"`.
    cpp_raw: bool,
    /// Lua's long strings and comments, `[[...]]` and `--[[...]]`.
    lua_long: bool,
}

const LANG: Lang = Lang {
    line_comments: &[],
    block_comments: &[],
    nested: false,
    quotes: &[],
    char_quote: false,
    char_backslash: false,
    regex: false,
    rust_raw: false,
    cpp_raw: false,
    lua_long: false,
};

const C_COMMENTS: &[(&str, &str)] = &[("/*", "*/")];
const C_QUOTES: &[Quote] = &[q!("\"", "\"", true, false)];

fn lang(language: &str) -> Option<Lang> {
    let c_like = Lang { line_comments: &["//"], block_comments: C_COMMENTS, quotes: C_QUOTES, char_quote: true, ..LANG };
    Some(match language {
        "C" | "Objective-C" => c_like,
        "C++" | "Objective-C++" => Lang { cpp_raw: true, ..c_like },
        "C#" => Lang { quotes: &[q!("@\"", "\"", false, true), q!("\"\"\"", "\"\"\"", false, true), q!("\"", "\"", true, false)], ..c_like },
        "Java" => Lang { quotes: &[q!("\"\"\"", "\"\"\"", true, true), q!("\"", "\"", true, false)], ..c_like },
        "Scala" => Lang { quotes: &[q!("\"\"\"", "\"\"\"", false, true), q!("\"", "\"", true, false)], ..c_like },
        "Groovy" => Lang {
            quotes: &[q!("\"\"\"", "\"\"\"", true, true), q!("'''", "'''", true, true), q!("\"", "\"", true, false), q!("'", "'", true, false)],
            char_quote: false,
            regex: true,
            ..c_like
        },
        "D" => Lang {
            block_comments: &[("/+", "+/"), ("/*", "*/")],
            nested: true,
            quotes: &[q!("r\"", "\"", false, true), q!("`", "`", false, true), q!("\"", "\"", true, true)],
            ..c_like
        },
        "Go" => Lang { quotes: &[q!("`", "`", false, true), q!("\"", "\"", true, false)], ..c_like },
        "Rust" => Lang { quotes: &[q!("\"", "\"", true, true)], rust_raw: true, ..c_like },
        "JavaScript" | "ActionScript" => Lang {
            quotes: &[q!("`", "`", true, true), q!("\"", "\"", true, false), q!("'", "'", true, false)],
            char_quote: false,
            regex: true,
            ..c_like
        },
        "CSS" => Lang { block_comments: C_COMMENTS, quotes: &[q!("\"", "\"", true, false), q!("'", "'", true, false)], ..LANG },
        "Python" => Lang {
            line_comments: &["#"],
            quotes: &[q!("\"\"\"", "\"\"\"", true, true), q!("'''", "'''", true, true), q!("\"", "\"", true, false), q!("'", "'", true, false)],
            ..LANG
        },
        "R" => Lang { line_comments: &["#"], quotes: &[q!("\"", "\"", true, true), q!("'", "'", true, true), q!("`", "`", false, false)], ..LANG },
        "Lua" => Lang { line_comments: &["--"], quotes: &[q!("\"", "\"", true, false), q!("'", "'", true, false)], lua_long: true, ..LANG },
        "SQL" => Lang {
            line_comments: &["--"],
            block_comments: C_COMMENTS,
            quotes: &[q!("'", "'", false, true), q!("\"", "\"", false, true), q!("`", "`", false, true)],
            ..LANG
        },
        "Haskell" => Lang {
            line_comments: &["--"],
            block_comments: &[("{-", "-}")],
            nested: true,
            quotes: &[q!("\"", "\"", true, false)],
            char_quote: true,
            ..LANG
        },
        "OCaml" => Lang { block_comments: &[("(*", "*)")], nested: true, quotes: &[q!("\"", "\"", true, true)], char_quote: true, ..LANG },
        "Clojure" => Lang { line_comments: &[";"], quotes: &[q!("\"", "\"", true, true)], char_backslash: true, ..LANG },
        "Lisp" => Lang {
            line_comments: &[";"],
            block_comments: &[("#|", "|#")],
            nested: true,
            quotes: &[q!("\"", "\"", true, true)],
            char_backslash: true,
            ..LANG
        },
        _ => return None,
    })
}

fn opener_of(close: u8) -> u8 {
    match close {
        b')' => b'(',
        b']' => b'[',
        _ => b'{',
    }
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The length of the UTF-8 character at `i`.
fn char_len(b: &[u8], i: usize) -> usize {
    match b.get(i) {
        Some(&c) if c >= 0xF0 => 4,
        Some(&c) if c >= 0xE0 => 3,
        Some(&c) if c >= 0xC0 => 2,
        _ => 1,
    }
}

pub fn check(text: &str, language: &str) -> Vec<Found> {
    let Some(lang) = lang(language) else { return Vec::new() };
    Scanner { b: text.as_bytes(), text, lang, found: Vec::new() }.run()
}

struct Scanner<'a> {
    b: &'a [u8],
    text: &'a str,
    lang: Lang,
    found: Vec<Found>,
}

impl Scanner<'_> {
    fn at(&self, i: usize, s: &str) -> bool {
        self.b[i..].starts_with(s.as_bytes())
    }

    fn hint(&mut self, from: usize, to: usize, message: String) {
        self.found.push(Found { from, to, message, error: false });
    }

    fn line_of(&self, i: usize) -> usize {
        self.b[..i].iter().filter(|&&c| c == b'\n').count() + 1
    }

    fn run(mut self) -> Vec<Found> {
        let b = self.b;
        let mut stack: Vec<(u8, usize)> = Vec::new();
        // the last byte of code that is not a space, for regex literals
        let mut last_code: Option<usize> = None;
        let mut i = 0;
        'scan: while i < b.len() {
            let c = b[i];
            if c.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            // comments
            if self.lang.lua_long && self.at(i, "--") {
                if let Some(end) = self.lua_long_end(i + 2) {
                    i = end;
                    continue;
                }
            }
            for (open, close) in self.lang.block_comments {
                if self.at(i, open) {
                    i = self.block_comment(i, open, close);
                    continue 'scan;
                }
            }
            for open in self.lang.line_comments {
                if self.at(i, open) {
                    i = self.text[i..].find('\n').map_or(b.len(), |n| i + n);
                    continue 'scan;
                }
            }
            // strings and literals
            let after_ident = i > 0 && is_ident(b[i - 1]);
            if self.lang.rust_raw && !after_ident {
                if let Some(end) = self.rust_raw(i) {
                    i = end;
                    last_code = Some(i - 1);
                    continue;
                }
            }
            if self.lang.cpp_raw && !after_ident {
                if let Some(end) = self.cpp_raw(i) {
                    i = end;
                    last_code = Some(i - 1);
                    continue;
                }
            }
            if self.lang.lua_long {
                if let Some(end) = self.lua_long_end(i) {
                    i = end;
                    last_code = Some(i - 1);
                    continue;
                }
            }
            if c == b'\'' && self.lang.char_quote {
                i = self.char_literal(i);
                last_code = Some(i - 1);
                continue;
            }
            if c == b'\\' && self.lang.char_backslash {
                i += 1 + char_len(b, i + 1);
                last_code = Some(i - 1);
                continue;
            }
            if c == b'/' && self.lang.regex && self.regex_allowed(last_code) {
                if let Some(end) = self.regex(i) {
                    i = end;
                    last_code = Some(i - 1);
                    continue;
                }
            }
            for quote in self.lang.quotes {
                if self.at(i, quote.open) {
                    i = self.string(i, quote);
                    last_code = Some(i.saturating_sub(1));
                    continue 'scan;
                }
            }
            // brackets
            match c {
                b'(' | b'[' | b'{' => stack.push((c, i)),
                b')' | b']' | b'}' => {
                    let want = opener_of(c);
                    match stack.iter().rposition(|&(o, _)| o == want) {
                        Some(k) if k + 1 == stack.len() => {
                            stack.pop();
                        }
                        Some(k) => {
                            // the brackets opened after the match were never closed
                            for (o, at) in stack.drain(k..).skip(1).collect::<Vec<_>>() {
                                let close = match o {
                                    b'(' => ')',
                                    b'[' => ']',
                                    _ => '}',
                                };
                                self.hint(at, at + 1, format!("`{}` is not closed before `{}` (missing `{close}`?)", o as char, c as char));
                            }
                        }
                        None => {
                            let open = want as char;
                            let message = match stack.last() {
                                Some(&(o, at)) => format!(
                                    "`{}` doesn't match the `{}` on line {}",
                                    c as char,
                                    o as char,
                                    self.line_of(at)
                                ),
                                None => format!("`{}` has no opening `{open}`", c as char),
                            };
                            self.hint(i, i + 1, message);
                        }
                    }
                }
                _ => {}
            }
            last_code = Some(i);
            i += char_len(b, i);
        }
        for (o, at) in stack {
            self.hint(at, at + 1, format!("`{}` is never closed", o as char));
        }
        self.found
    }

    /// Skip a block comment that starts at `i`; returns the end.
    fn block_comment(&mut self, i: usize, open: &str, close: &str) -> usize {
        let mut depth = 1;
        let mut j = i + open.len();
        while j < self.b.len() {
            if self.at(j, close) {
                depth -= 1;
                j += close.len();
                if depth == 0 || !self.lang.nested {
                    return j;
                }
            } else if self.lang.nested && self.at(j, open) {
                depth += 1;
                j += open.len();
            } else {
                j += 1;
            }
        }
        self.hint(i, i + open.len(), format!("This comment is never closed (missing `{close}`)"));
        self.b.len()
    }

    /// Skip a string that starts at `i`; returns the end.
    fn string(&mut self, i: usize, quote: &Quote) -> usize {
        let b = self.b;
        let mut j = i + quote.open.len();
        while j < b.len() {
            if quote.escape && b[j] == b'\\' {
                j += 1 + char_len(b, j + 1);
                continue;
            }
            if self.at(j, quote.close) {
                return j + quote.close.len();
            }
            if b[j] == b'\n' && !quote.multiline {
                let to = if j > 0 && b[j - 1] == b'\r' { j - 1 } else { j };
                self.hint(i, i + quote.open.len(), format!("This string is not closed on its line (missing `{}`)", quote.close));
                return to;
            }
            j += 1;
        }
        self.hint(i, i + quote.open.len(), format!("This string is never closed (missing `{}`)", quote.close));
        b.len()
    }

    /// A `'` character literal, or a lone `'` (a lifetime or a prime).
    fn char_literal(&self, i: usize) -> usize {
        let b = self.b;
        if b.get(i + 1) == Some(&b'\\') {
            // '\n', '\x41', '\u{1F600}': up to the next ' on the line
            let end = (i + 2..b.len().min(i + 14)).find(|&j| b[j] == b'\'' || b[j] == b'\n');
            return match end {
                Some(j) if b[j] == b'\'' => j + 1,
                _ => i + 1,
            };
        }
        let n = char_len(b, i + 1);
        if i + 1 < b.len() && b[i + 1] != b'\n' && b.get(i + 1 + n) == Some(&b'\'') {
            return i + 2 + n;
        }
        i + 1
    }

    /// Rust's `r"..."`, `r#"..."#`, `br"..."`, starting at `i`.
    fn rust_raw(&mut self, i: usize) -> Option<usize> {
        let b = self.b;
        let mut j = i;
        if b.get(j) == Some(&b'b') || b.get(j) == Some(&b'c') {
            j += 1;
        }
        if b.get(j) != Some(&b'r') {
            return None;
        }
        j += 1;
        let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
        j += hashes;
        if b.get(j) != Some(&b'"') {
            return None;
        }
        let close = format!("\"{}", "#".repeat(hashes));
        match self.text[j + 1..].find(&close) {
            Some(n) => Some(j + 1 + n + close.len()),
            None => {
                self.hint(i, j + 1, format!("This string is never closed (missing `{close}`)"));
                Some(b.len())
            }
        }
    }

    /// C++'s `R"delim(...)delim"`, starting at `i`.
    fn cpp_raw(&mut self, i: usize) -> Option<usize> {
        let b = self.b;
        let mut j = i;
        while j < b.len() && matches!(b[j], b'u' | b'U' | b'L' | b'8') && j - i < 2 {
            j += 1;
        }
        if !self.at(j, "R\"") {
            return None;
        }
        let start = j + 2;
        let paren = self.text[start..].find('(').filter(|&n| n <= 16)?;
        let delim = &self.text[start..start + paren];
        let close = format!("){delim}\"");
        match self.text[start + paren..].find(&close) {
            Some(n) => Some(start + paren + n + close.len()),
            None => {
                self.hint(i, start, format!("This string is never closed (missing `{close}`)"));
                Some(b.len())
            }
        }
    }

    /// Lua's long bracket `[[`, `[==[` at `i`: the end of its `]]`.
    fn lua_long_end(&mut self, i: usize) -> Option<usize> {
        let b = self.b;
        if b.get(i) != Some(&b'[') {
            return None;
        }
        let eq = b[i + 1..].iter().take_while(|&&c| c == b'=').count();
        if b.get(i + 1 + eq) != Some(&b'[') {
            return None;
        }
        let close = format!("]{}]", "=".repeat(eq));
        match self.text[i + 2 + eq..].find(&close) {
            Some(n) => Some(i + 2 + eq + n + close.len()),
            None => {
                self.hint(i, i + 2 + eq, format!("This is never closed (missing `{close}`)"));
                Some(b.len())
            }
        }
    }

    /// Whether a `/` after `last` starts a regular expression rather than
    /// dividing.
    fn regex_allowed(&self, last: Option<usize>) -> bool {
        let Some(at) = last else { return true };
        let c = self.b[at];
        if b"(,=:[!&|?{};+-*%<>~^".contains(&c) {
            return true;
        }
        if is_ident(c) {
            let start = self.b[..=at].iter().rposition(|&x| !is_ident(x)).map_or(0, |p| p + 1);
            let word = &self.text[start..=at];
            return matches!(word, "return" | "typeof" | "case" | "in" | "of" | "new" | "delete" | "void" | "throw" | "yield" | "await");
        }
        false
    }

    /// A `/.../flags` literal at `i`, on one line; `None` if it is not one.
    fn regex(&self, i: usize) -> Option<usize> {
        let b = self.b;
        if matches!(b.get(i + 1), Some(b'/') | Some(b'*')) {
            return None;
        }
        let mut j = i + 1;
        let mut class = false;
        while j < b.len() {
            match b[j] {
                b'\\' => j += 1,
                b'\n' => return None,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    return Some(j);
                }
                _ => {}
            }
            j += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(text: &str, language: &str) -> Vec<(usize, String)> {
        check(text, language).into_iter().map(|f| (f.from, f.message)).collect()
    }

    #[test]
    fn balanced_code_has_no_hints() {
        let rust = "fn f<'a>(x: &'a str) -> char {\n    let s = \"(\"; // )\n    /* { */ let c = '}';\n    let r = r#\"\"]\"#;\n    let n = 1_000; '\\n'\n}\n";
        assert_eq!(hints(rust, "Rust"), []);
        let py = "def f(x):\n    s = '''(\n    '''\n    # ]\n    return [x, \"{\"]\n";
        assert_eq!(hints(py, "Python"), []);
        let js = "const r = /[(]/g;\nconst t = `a ${b}`;\nlet x = a / b / (c);\n";
        assert_eq!(hints(js, "JavaScript"), []);
        let cpp = "auto s = R\"x(a ) b)x\";\nint n = 1'000'000;\n";
        assert_eq!(hints(cpp, "C++"), []);
        let lua = "local s = [[ ( ]]\n--[[ { ]]\nprint(s) -- )\n";
        assert_eq!(hints(lua, "Lua"), []);
        let lisp = "(defun f (x) #| ( |# (list #\\( x))\n";
        assert_eq!(hints(lisp, "Lisp"), []);
        let hs = "f x' = x' -- (\n{- { {- } -} -}\n";
        assert_eq!(hints(hs, "Haskell"), []);
    }

    #[test]
    fn unmatched_brackets_are_hints() {
        assert_eq!(hints("fn main() {\n", "Rust"), [(10, "`{` is never closed".to_string())]);
        assert_eq!(hints("x = f(1))\n", "Python"), [(8, "`)` has no opening `(`".to_string())]);
        let h = hints("call(a, [b);\n", "JavaScript");
        assert_eq!(h, [(8, "`[` is not closed before `)` (missing `]`?)".to_string())]);
        let h = hints("{\n  x]\n}", "C");
        assert_eq!(h, [(5, "`]` doesn't match the `{` on line 1".to_string())]);
    }

    #[test]
    fn open_strings_and_comments_are_hints() {
        let h = hints("let s = \"abc;\nlet t = 1;\n", "JavaScript");
        assert_eq!(h, [(8, "This string is not closed on its line (missing `\"`)".to_string())]);
        // Rust strings can span lines, so only the end of the text counts.
        let h = hints("let s = \"abc;\nlet t = 1;\n", "Rust");
        assert_eq!(h, [(8, "This string is never closed (missing `\"`)".to_string())]);
        let h = hints("int x; /* note\n", "C");
        assert_eq!(h, [(7, "This comment is never closed (missing `*/`)".to_string())]);
    }

    #[test]
    fn other_languages_are_not_checked() {
        assert_eq!(hints("case $x in a) ;; esac", "Bourne Again Shell (bash)"), []);
        assert_eq!(hints("1) first (", "Markdown"), []);
    }
}
