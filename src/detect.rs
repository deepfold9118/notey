//! Guess a document's file type from its text, for tabs whose name does not
//! decide it (untitled tabs and files without an extension). The result is
//! the id of an official File Type plugin, so that Notey can switch to it
//! when it is installed, or offer to install it.
//!
//! The guess comes first from unambiguous signs (a shebang, an XML prolog,
//! valid JSON, a diff header), then from scoring the lines against the
//! typical forms of each language. A weak or tied score gives no guess, so
//! ordinary prose stays plain text.

/// Only the start of a long document is looked at.
pub const MAX_BYTES: usize = 32 * 1024;
const MAX_LINES: usize = 400;
/// A signature of a language that has no File Type plugin.
const NONE: &str = "";
/// The lowest score that counts as a guess.
const MIN_SCORE: u32 = 6;

/// The File Type plugin id for `text`, if its language is clear.
pub fn detect(text: &str) -> Option<&'static str> {
    let mut end = text.len().min(MAX_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    let trimmed = head.trim_start_matches('\u{feff}').trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(id) = signature(trimmed, text.len() <= MAX_BYTES) {
        return (id != NONE).then_some(id);
    }
    let lines: Vec<&str> = trimmed.lines().take(MAX_LINES).collect();
    // Section headers ("[package]", "[Setup]") mark INI and TOML files,
    // whose keys and comments would otherwise pass for other languages.
    // TOML's values are typed ("1", "true", "[...]"); INI's are bare text.
    if lines.iter().filter(|l| is_section(l)).count() >= 2 {
        return is_toml(&lines).then_some("filetype-toml");
    }
    let mut scores: Vec<(&'static str, u32)> = RULES.iter().map(|(id, rule)| (*id, rule(&lines))).collect();
    scores.sort_by_key(|s| std::cmp::Reverse(s.1));
    let (best, score) = scores[0];
    let runner_up = scores.get(1).map_or(0, |s| s.1);
    // A clear winner: well above the floor and ahead of the next language.
    (score >= MIN_SCORE && score * 2 > runner_up * 3).then_some(best)
}

/// Signs that settle the file type on their own. `whole` tells whether
/// `text` is the whole document (for the JSON check).
fn signature(text: &str, whole: bool) -> Option<&'static str> {
    let first = text.lines().next().unwrap_or("").trim();
    if let Some(interpreter) = first.strip_prefix("#!") {
        // "#!/usr/bin/python3" or "#!/usr/bin/env -S python3 -u".
        let base = |s: &str| s.rsplit('/').next().unwrap_or("").to_string();
        let mut words = interpreter.split_whitespace();
        let mut name = words.next().map(base).unwrap_or_default();
        if name == "env" {
            name = words.find(|w| !w.starts_with('-')).map(base).unwrap_or_default();
        }
        let name = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        return match name {
            "python" => Some("filetype-python"),
            "node" | "nodejs" | "deno" | "bun" => Some("filetype-javascript"),
            "sh" | "bash" | "zsh" | "ksh" | "dash" => Some("filetype-shellscript"),
            "perl" => Some("filetype-perl"),
            "ruby" => Some("filetype-ruby"),
            "lua" => Some("filetype-lua"),
            "php" => Some("filetype-php"),
            "Rscript" => Some("filetype-r"),
            "tclsh" | "wish" => Some("filetype-tcl"),
            _ => None,
        };
    }
    // PowerShell has no File Type plugin, and its comments and here-strings
    // would pass for Markdown.
    if first == "<#" || text.contains("[CmdletBinding(") || text.contains("$PSScriptRoot") {
        return Some(NONE);
    }
    let lower = first.to_ascii_lowercase();
    if lower.starts_with("<?php") {
        return Some("filetype-php");
    }
    if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
        return Some("filetype-html");
    }
    if lower.starts_with("<?xml") {
        return Some("filetype-xml");
    }
    if lower == "@echo off" || lower == "@echo on" {
        return Some("filetype-batch-file");
    }
    if first.starts_with("diff --git ") || first.starts_with("Index: ") && text.contains("\n+++ ") {
        return Some("filetype-diff");
    }
    let mut lines = text.lines();
    if first.starts_with("--- ") && lines.nth(1).is_some_and(|l| l.starts_with("+++ ")) {
        return Some("filetype-diff");
    }
    if text.lines().any(|l| l.starts_with("\\documentclass") || l.starts_with("\\begin{document}")) {
        return Some("filetype-latex");
    }
    if text.starts_with('{') || text.starts_with('[') {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => {
                let substantial = match &value {
                    serde_json::Value::Object(map) => !map.is_empty(),
                    serde_json::Value::Array(items) => items.iter().any(|v| v.is_object() || v.is_array()),
                    _ => false,
                };
                if substantial {
                    return Some("filetype-json");
                }
            }
            // The start of a longer document, valid up to where it was cut.
            Err(e) if !whole && e.is_eof() => return Some("filetype-json"),
            Err(_) => {}
        }
    }
    if text.starts_with('<') && text.trim_end().ends_with('>') {
        let lower = text.to_ascii_lowercase();
        let html = ["<head", "<body", "<div", "<p>", "<span", "<script", "<table", "<a href"];
        if html.iter().any(|t| lower.contains(t)) {
            return Some("filetype-html");
        }
        if text.contains("</") || text.contains("/>") {
            return Some("filetype-xml");
        }
    }
    None
}

fn is_section(line: &str) -> bool {
    let t = line.trim_end();
    // "[[bin]]" is a TOML array of tables.
    let t = t.strip_prefix('[').and_then(|t| t.strip_suffix(']')).filter(|t| t.starts_with('[')).unwrap_or(t);
    t.len() > 2
        && t.starts_with('[')
        && t.ends_with(']')
        && t[1..t.len() - 1].chars().all(|c| c.is_alphanumeric() || " ._-\"".contains(c))
}

/// Whether the "key = value" lines of a sectioned file are TOML: most
/// values are strings, numbers, booleans, arrays or inline tables.
fn is_toml(lines: &[&str]) -> bool {
    let (mut keys, mut typed) = (0, 0);
    for line in lines {
        let t = line.trim();
        if t.starts_with(['#', ';', '[']) {
            continue;
        }
        let Some((key, value)) = t.split_once('=') else { continue };
        let key = key.trim();
        let bare = |c: char| c.is_alphanumeric() || "_-.\"' ".contains(c);
        if key.is_empty() || !key.chars().all(bare) {
            continue;
        }
        keys += 1;
        let v = value.trim();
        let number = v.split(['#', ' ']).next().is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok());
        if v.starts_with(['"', '\'', '[', '{']) || number || v.starts_with("true") || v.starts_with("false") {
            typed += 1;
        }
    }
    keys >= 2 && typed * 4 >= keys * 3
}

type Rule = fn(&[&str]) -> u32;

/// The scored languages. Each rule adds points for the lines that look
/// like that language.
const RULES: &[(&str, Rule)] = &[
    ("filetype-markdown", markdown),
    ("filetype-python", python),
    ("filetype-rust", rust),
    ("filetype-javascript", javascript),
    ("filetype-cplusplus", cplusplus),
    ("filetype-csharp", csharp),
    ("filetype-java", java),
    ("filetype-go", go),
    ("filetype-sql", sql),
    ("filetype-yaml", yaml),
    ("filetype-css", css),
    ("filetype-shellscript", shell),
    ("filetype-batch-file", batch),
    ("filetype-php", php),
    ("filetype-ruby", ruby),
    ("filetype-lua", lua),
    ("filetype-perl", perl),
    ("filetype-makefile", makefile),
];

/// Sum `weight(line)` over the lines.
fn score(lines: &[&str], mut weight: impl FnMut(&str) -> u32) -> u32 {
    lines.iter().map(|l| weight(l)).sum()
}

fn starts_word(line: &str, word: &str) -> bool {
    line.strip_prefix(word).is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '(', '\t']))
}

fn markdown(lines: &[&str]) -> u32 {
    let mut fence = false;
    let mut headings = 0;
    let blank = |i: Option<usize>| i.and_then(|i| lines.get(i)).is_none_or(|l| l.trim().is_empty());
    let mut i: usize = 0;
    let other = score(lines, |l| {
        i += 1;
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            return 2;
        }
        if fence {
            return 0;
        }
        let hashes = l.chars().take_while(|&c| c == '#').count();
        // A heading stands apart; a run of "#" lines is a comment block.
        let heading = (1..=6).contains(&hashes)
            && l[hashes..].starts_with(' ')
            && l.len() > hashes + 1
            && blank((i - 1).checked_sub(1))
            && blank(Some(i));
        // Any other "#" line is a comment of some other language.
        if hashes > 0 && !heading {
            return 0;
        }
        let list = (t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ")) && t.len() > 2;
        let numbered = t.split_once(". ").is_some_and(|(n, rest)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty());
        let link = l.contains("](") && l.contains('[');
        let emphasis = l.matches("**").count() >= 2 || l.matches('`').count() >= 2;
        let table = t.starts_with('|') && t.ends_with('|') && t.len() > 2;
        let quote = t.starts_with("> ");
        headings += u32::from(heading);
        u32::from(list || numbered || quote) + u32::from(link) * 2 + u32::from(emphasis) + u32::from(table) * 2
    });
    // "# text" is also a comment in many languages: headings count only
    // beside other Markdown.
    if other == 0 {
        0
    } else {
        other + headings * 3
    }
}

/// Type names that Python type hints use.
const PY_TYPES: &[&str] = &[
    "int", "float", "str", "bool", "bytes", "complex", "list", "dict", "tuple", "set", "frozenset", "object", "None",
    "Any", "Optional", "Union", "List", "Dict", "Tuple", "Set", "Callable", "Iterable", "Iterator", "Sequence",
];

/// A Python type hint: `int`, `list[str]`, `Optional[int]`, `int | None`.
fn python_type(s: &str) -> bool {
    let base = s.trim().split(['[', ' ', '|']).next().unwrap_or("");
    PY_TYPES.contains(&base)
}

/// A variable with a type hint: `count: int = 10` or `name: str`.
fn python_annotation(t: &str) -> bool {
    let Some((name, rest)) = t.split_once(':') else { return false };
    let ident = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    let ty = rest.split_once(" = ").map_or(rest, |(ty, _)| ty);
    ident && rest.starts_with(' ') && python_type(ty)
}

/// A `def` whose parameters have type hints: `def f(a: int, b: str = "")`.
fn python_typed_def(t: &str) -> bool {
    let Some(params) = t.strip_prefix("def ").and_then(|r| r.split_once('(')).map(|(_, p)| p) else { return false };
    let params = params.rsplit_once(')').map_or(params, |(p, _)| p);
    params.split(',').any(|p| p.split_once(':').is_some_and(|(_, ty)| python_type(ty.split('=').next().unwrap_or(""))))
}

fn python(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        if t.starts_with("if __name__") {
            return 6;
        }
        if python_annotation(t) {
            return 2;
        }
        let block = ["def ", "class ", "if ", "elif ", "else", "for ", "while ", "with ", "try", "except", "finally"]
            .iter()
            .any(|k| t.starts_with(k));
        let import = (t.starts_with("import ") || t.starts_with("from ") && t.contains(" import ")) && !t.ends_with(';');
        u32::from(t.starts_with("def ") && t.ends_with(':')) * 3
            + u32::from(python_typed_def(t) || t.starts_with("def ") && t.contains(") -> ")) * 3
            + u32::from(t.starts_with("elif ")) * 3
            + u32::from(block && t.ends_with(':')) * 2
            + u32::from(import) * 3
            + u32::from(t.contains("self.") || t.contains("print(") || t.contains("None") || t.starts_with("@"))
    })
}

fn rust(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        let item = ["fn ", "pub fn ", "pub(crate) fn ", "impl ", "impl<", "pub struct ", "struct ", "enum ", "pub enum ", "mod ", "trait "]
            .iter()
            .any(|k| t.starts_with(k));
        u32::from(item) * 2
            + u32::from(t.starts_with("use ") && t.contains("::") && t.ends_with(';')) * 3
            + u32::from(t.starts_with("#[")) * 3
            + u32::from(t.contains("let mut ")) * 3
            + u32::from(t.contains("println!") || t.contains("format!") || t.contains("vec!")) * 2
            + u32::from(t.contains("&mut ") || t.contains("&self") || t.contains("Option<") || t.contains("Result<")) * 2
            + u32::from(t.starts_with("match ") || t.contains(" => ") && t.ends_with(','))
            + u32::from(t.contains("::")) * u32::from(!t.starts_with("//"))
            + u32::from(t.contains(") -> "))
    })
}

fn javascript(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("function ") || t.contains(" function(") || t.contains(" function (")) * 2
            + u32::from(t.starts_with("const ") || t.starts_with("let ") || t.starts_with("var ")) * u32::from(t.contains(" = "))
            + u32::from(t.contains(") => ") || t.contains("=> {")) * 2
            + u32::from(t.contains("console.log(") || t.contains("document.") || t.contains("window.")) * 3
            + u32::from(t.contains("require(") || t.starts_with("import ") && t.contains(" from '") || t.contains(" from \"")) * 3
            + u32::from(t.starts_with("export ") || t.contains("===") || t.contains("!==")) * 2
    })
}

fn cplusplus(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("#include")) * 4
            + u32::from(t.starts_with("#define ") || t.starts_with("#ifndef ") || t.starts_with("#pragma ")) * 3
            + u32::from(t.contains("int main(")) * 3
            + u32::from(t.contains("std::") || t.contains("cout <<") || t.contains("nullptr")) * 3
            + u32::from(t.starts_with("template <") || t.starts_with("template<")) * 3
            + u32::from(t.contains("printf(") || t.contains("malloc(") || t.contains("->"))
    })
}

fn csharp(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("using System") && t.ends_with(';')) * 4
            + u32::from(t.starts_with("namespace ")) * 2
            + u32::from(t.contains("Console.Write")) * 4
            + u32::from(t.contains("{ get; set; }") || t.contains("{ get; }")) * 4
            + u32::from(t.contains(" async Task") || t.contains("var ") && t.ends_with(';'))
    })
}

fn java(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("import java.") || t.starts_with("import javax.")) * 4
            + u32::from(t.starts_with("package ") && t.ends_with(';')) * 3
            + u32::from(t.contains("public static void main(")) * 4
            + u32::from(t.contains("System.out.print")) * 4
            + u32::from(t == "@Override") * 3
    })
}

fn go(lines: &[&str]) -> u32 {
    // Every Go file starts with its package clause.
    if !lines.iter().any(|l| l.starts_with("package ") && !l.trim_end().ends_with(';')) {
        return 0;
    }
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("package ") && !t.ends_with(';')) * 3
            + u32::from(t.starts_with("func ")) * 3
            + u32::from(t == "import (") * 3
            + u32::from(t.contains(" := ")) * 2
            + u32::from(t.contains("fmt.") || t.contains("err != nil")) * 3
    })
}

fn sql(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let u = l.trim().to_ascii_uppercase();
        let statement = [
            "SELECT ", "INSERT INTO ", "UPDATE ", "DELETE FROM ", "CREATE TABLE ", "CREATE INDEX ", "CREATE VIEW ",
            "ALTER TABLE ", "DROP TABLE ", "WITH ",
        ]
        .iter()
        .any(|k| u.starts_with(k));
        u32::from(statement) * 3
            + u32::from(u.starts_with("FROM ") || u.starts_with("WHERE ") || u.contains(" JOIN ") || u.starts_with("GROUP BY ") || u.starts_with("ORDER BY ")) * 2
            + u32::from(u.ends_with(';') && (u.contains(" VALUES") || u.contains(" FROM ")))
    })
}

fn yaml(lines: &[&str]) -> u32 {
    let content: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let key = |l: &str| {
        let t = l.trim_start().trim_start_matches("- ");
        // "count: int = 10" is a Python variable, not a YAML key.
        if python_annotation(t.trim_end()) {
            return false;
        }
        match t.split_once(':') {
            Some((k, rest)) => {
                !k.is_empty()
                    && k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
                    && (rest.is_empty() || rest.starts_with(' '))
            }
            None => false,
        }
    };
    let keys = content.iter().filter(|l| key(l)).count();
    let lists = content.iter().filter(|l| l.trim_start().starts_with("- ")).count();
    let comments = content.iter().filter(|l| l.trim_start().starts_with('#')).count();
    // YAML is mostly keys and list items; prose with a "Note:" is not.
    if keys < 3 || (keys + lists + comments) * 10 < content.len() * 8 {
        return 0;
    }
    let nested = content.iter().any(|l| l.starts_with("  ") && key(l));
    (keys as u32) * 3
        + lists as u32
        + u32::from(lines.first().is_some_and(|l| l.trim() == "---")) * 3
        + u32::from(nested) * 3
}

fn css(lines: &[&str]) -> u32 {
    let property = |l: &str| {
        let t = l.trim();
        t.ends_with(';')
            && t.split_once(':').is_some_and(|(p, _)| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
    };
    if !lines.iter().any(|l| property(l)) {
        return 0;
    }
    score(lines, |l| {
        let t = l.trim();
        let selector = t.ends_with('{') && !t.contains(['(', '=', '"']);
        u32::from(property(l)) * 2 + u32::from(selector) * 2 + u32::from(t.starts_with("@media") || t.starts_with("@import")) * 3
    })
}

fn shell(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t == "fi" || t == "done" || t == "esac" || t.ends_with("; then") || t.ends_with("; do")) * 3
            + u32::from(t.starts_with("if [") || t.starts_with("if [[")) * 3
            + u32::from(t.starts_with("export ") && t.contains('=')) * 2
            + u32::from(starts_word(t, "echo") || t.contains("$(") || t.contains("${")) * 2
    })
}

fn batch(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let u = l.trim().to_ascii_uppercase();
        u32::from(u.starts_with("@ECHO ") || u.starts_with("REM ") || u.starts_with(":: ")) * 3
            + u32::from(u.starts_with("SET ") || u.starts_with("GOTO ") || u.starts_with("CALL ")) * 2
            + u32::from(u.contains("%~DP0") || u.contains("%ERRORLEVEL%")) * 4
            + u32::from(u.starts_with(':') && !u.starts_with("::") && u.len() > 1 && !u.contains(' ')) * 2
    })
}

fn php(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.contains("<?php")) * 6 + u32::from(t.contains("$this->")) * 4 + u32::from(t.starts_with("echo $")) * 3
    })
}

fn ruby(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        // Python's `def` ends in ":" or has type hints.
        let python_def = t.ends_with(':') || python_typed_def(t) || t.contains(") -> ");
        u32::from(t.starts_with("def ") && !python_def && !t.ends_with('{')) * 2
            + u32::from(t.starts_with("require '") || t.starts_with("require \"") || t.starts_with("require_relative ")) * 3
            + u32::from(t.contains(" do |") || t.starts_with("attr_accessor ") || t.starts_with("module ")) * 3
            + u32::from(starts_word(t, "puts") || t.ends_with(".each do")) * 2
    })
}

fn lua(lines: &[&str]) -> u32 {
    if !lines.iter().any(|l| l.trim_start().starts_with("local ")) {
        return 0;
    }
    score(lines, |l| {
        let t = l.trim();
        u32::from(t.starts_with("local function ")) * 4
            + u32::from(t.starts_with("local ") && t.contains(" = ")) * 2
            + u32::from(t.ends_with(" then") && (t.starts_with("if ") || t.starts_with("elseif "))) * 2
            + u32::from(t.starts_with("--") && !t.starts_with("---"))
            + u32::from(t.contains(" ~= ") || t.contains("..")) * u32::from(t.starts_with("local ") || t.starts_with("if "))
    })
}

fn perl(lines: &[&str]) -> u32 {
    score(lines, |l| {
        let t = l.trim();
        u32::from(t == "use strict;" || t == "use warnings;") * 5
            + u32::from(t.starts_with("my $") || t.starts_with("my @") || t.starts_with("my %")) * 3
            + u32::from(t.starts_with("sub ") && t.ends_with('{')) * 2
    })
}

fn makefile(lines: &[&str]) -> u32 {
    let mut score = 0;
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with(".PHONY") {
            score += 5;
        }
        let target = l
            .split_once(':')
            .is_some_and(|(t, rest)| !t.is_empty() && !t.contains(' ') && !rest.starts_with('=') && !t.starts_with('\t'));
        if target && lines.get(i + 1).is_some_and(|n| n.starts_with('\t')) {
            score += 3;
        }
        if l.contains("$(") && !l.contains(':') && l.contains(" = ") {
            score += 1;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::detect;

    #[test]
    fn detects_from_signatures() {
        assert_eq!(detect("#!/usr/bin/env python3\nprint('hi')"), Some("filetype-python"));
        assert_eq!(detect("#!/bin/bash\necho hi"), Some("filetype-shellscript"));
        assert_eq!(detect("<?xml version=\"1.0\"?>\n<a/>"), Some("filetype-xml"));
        assert_eq!(detect("<!DOCTYPE html>\n<html></html>"), Some("filetype-html"));
        assert_eq!(detect("{\"name\": \"notey\", \"version\": 1}"), Some("filetype-json"));
        assert_eq!(detect("[{\"a\": 1}, {\"a\": 2}]"), Some("filetype-json"));
        assert_eq!(detect("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@"), Some("filetype-diff"));
        assert_eq!(detect("@echo off\nset X=1"), Some("filetype-batch-file"));
        assert_eq!(detect("<?php\necho 1;"), Some("filetype-php"));
    }

    #[test]
    fn detects_from_scores() {
        let md = "# Title\n\nSome text with a [link](https://example.com).\n\n- one\n- two\n\n## More\n";
        assert_eq!(detect(md), Some("filetype-markdown"));
        let py = "import os\nfrom pathlib import Path\n\ndef main():\n    for p in Path('.').iterdir():\n        print(p)\n";
        assert_eq!(detect(py), Some("filetype-python"));
        // Type hints, with no imports or colon-ended blocks to go on.
        let typed = "measure: int = 10\nmultiplier: float = 1.5\n\ndef test_script(start: int, multiplier: int): float >\n\treturn start * multiplier\n\ntest_script(measure, multiplier)\n";
        assert_eq!(detect(typed), Some("filetype-python"));
        let typed = "measure: int = 10\nmultiplier: float = 1.5\n\ndef test_script(start: int, multiplier: int) -> float:\n\treturn start * multiplier\n";
        assert_eq!(detect(typed), Some("filetype-python"));
        let rb = "require 'json'\n\ndef greet(name)\n  puts \"hi #{name}\"\nend\n";
        assert_eq!(detect(rb), Some("filetype-ruby"));
        let rs = "use std::fs;\n\nfn main() {\n    let mut x = Vec::new();\n    println!(\"{x:?}\");\n}\n";
        assert_eq!(detect(rs), Some("filetype-rust"));
        let js = "const fs = require('fs');\nfunction go(a) {\n  console.log(a);\n}\n";
        assert_eq!(detect(js), Some("filetype-javascript"));
        let c = "#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\\n\");\n}\n";
        assert_eq!(detect(c), Some("filetype-cplusplus"));
        let sql = "SELECT id, name\nFROM users\nWHERE id = 1;\n";
        assert_eq!(detect(sql), Some("filetype-sql"));
        let yaml = "---\nname: notey\nversion: 1\nauthors:\n  - one\ndeps:\n  gpui: 0.2\n";
        assert_eq!(detect(yaml), Some("filetype-yaml"));
        let css = "body {\n  margin: 0;\n  color: red;\n}\n";
        assert_eq!(detect(css), Some("filetype-css"));
    }

    #[test]
    fn leaves_prose_alone() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("hello"), None);
        assert_eq!(detect("Shopping list\nNote: buy milk\nCall Sam at 5.\n"), None);
        assert_eq!(detect("Dear team,\n\nThe build is green again. Thanks for the quick fixes.\n"), None);
        assert_eq!(detect("[1, 2, 3]"), None);
        // Comments are not headings; INI files have no plugin.
        assert_eq!(detect("# build output
/target
*.pdb
# logs
*.log
"), None);
        let toml = "[package]
name = \"notey\"
# comment

[dependencies]
serde = \"1\"
";
        assert_eq!(detect(toml), Some("filetype-toml"));
        let pyproject = "[project]
name = \"tool\"
version = \"0.1.0\"
dependencies = [
  \"requests>=2\",
]

[[tool.mypy.overrides]]
strict = true
";
        assert_eq!(detect(pyproject), Some("filetype-toml"));
        assert_eq!(detect("<#
.Synopsis
Do things.
#>
param([string]$Path)
"), None);
        let commented = "# the name
name: \"x\"

# list of languages:
#   - rust
#   - go
languages:
  - rust
# Note:
encoding: utf-8
";
        assert_eq!(detect(commented), Some("filetype-yaml"));
        let ini = "[Setup]
AppName=Notey

[Code]
function Init(): Boolean;
begin
  Result := True;
end;
";
        assert_eq!(detect(ini), None);
    }

    #[test]
    fn detects_the_start_of_long_json() {
        let items: Vec<String> = (0..3000).map(|i| format!("{{\"id\": {i}, \"name\": \"item {i}\"}}")).collect();
        let json = format!("{{\"items\": [{}]}}", items.join(",
"));
        assert!(json.len() > super::MAX_BYTES);
        assert_eq!(detect(&json), Some("filetype-json"));
    }

    #[test]
    fn looks_only_at_the_start_on_a_char_boundary() {
        let long = "─".repeat(20_000);
        assert_eq!(detect(&long), None);
    }
}
