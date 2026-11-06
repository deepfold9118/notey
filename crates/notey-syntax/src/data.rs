//! Data formats with a real parser: JSON, YAML, TOML and XML. A problem
//! here is an error (the text is not valid), at the parser's position.

use crate::Found;

/// The parser's problem in `text` for a data language, or `None` when the
/// language is not one of them.
pub fn check(text: &str, language: &str, path: &str) -> Option<Vec<Found>> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let found = match language {
        "JSON" => {
            // JSON5 is another format; JSONC allows comments and trailing commas.
            if ext == "json5" {
                return Some(Vec::new());
            }
            json(text, is_jsonc(&name, ext))
        }
        "YAML" => yaml(text),
        "TOML" => toml_check(text),
        "XML" => xml(text),
        _ => return None,
    };
    Some(found.into_iter().collect())
}

/// Files that are JSON with comments, by name.
fn is_jsonc(name: &str, ext: &str) -> bool {
    const NAMES: [&str; 9] = [
        "settings.json",
        "keybindings.json",
        "launch.json",
        "tasks.json",
        "extensions.json",
        "devcontainer.json",
        ".devcontainer.json",
        ".eslintrc.json",
        ".babelrc",
    ];
    ext == "jsonc" || NAMES.contains(&name) || name.starts_with("tsconfig") || name.starts_with("jsconfig")
}

/// The byte offset of a 1-based line and a 0-based column, counted in
/// bytes (`chars` false) or characters.
fn offset(text: &str, line: usize, col: usize, chars: bool) -> usize {
    let start: usize = text.split_inclusive('\n').take(line.saturating_sub(1)).map(str::len).sum();
    let start = start.min(text.len());
    let rest = &text[start..];
    let line_len = rest.find('\n').unwrap_or(rest.len());
    let at = if chars { rest.char_indices().nth(col).map_or(rest.len(), |(i, _)| i) } else { col };
    let mut b = (start + at.min(line_len)).min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

fn error(at: usize, message: String) -> Found {
    Found { from: at, to: at, message, error: true }
}

fn json(text: &str, jsonc: bool) -> Option<Found> {
    let cleaned;
    let text = if jsonc {
        cleaned = strip_jsonc(text);
        cleaned.as_str()
    } else {
        text
    };
    if text.trim().is_empty() {
        return None;
    }
    let e = serde_json::from_str::<serde::de::IgnoredAny>(text).err()?;
    let message = e.to_string();
    let message = message.rsplit_once(" at line ").map_or(message.as_str(), |(m, _)| m);
    let message = message.replace("EOF while parsing", "the text ends inside");
    // serde_json's column is 1-based, in bytes; 0 means the end of the line
    let col = e.column().saturating_sub(1);
    Some(error(offset(text, e.line(), col, false), format!("Invalid JSON: {message}")))
}

/// JSONC as JSON: comments and trailing commas become spaces, so offsets
/// stay the same.
fn strip_jsonc(text: &str) -> String {
    let mut out = text.as_bytes().to_vec();
    // first the comments, then the commas (a comment may follow a comma)
    for commas in [false, true] {
        let b = out.clone();
        let mut i = 0;
        let mut in_string = false;
        while i < b.len() {
            let c = b[i];
            if in_string {
                match c {
                    b'\\' => i += 1,
                    b'"' => in_string = false,
                    _ => {}
                }
                i += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'/' if !commas && b.get(i + 1) == Some(&b'/') => {
                    while i < b.len() && b[i] != b'\n' {
                        out[i] = b' ';
                        i += 1;
                    }
                    continue;
                }
                b'/' if !commas && b.get(i + 1) == Some(&b'*') => {
                    let end = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len(), |e| i + 2 + e + 2);
                    for o in out.iter_mut().take(end).skip(i) {
                        if *o != b'\n' {
                            *o = b' ';
                        }
                    }
                    i = end;
                    continue;
                }
                b',' if commas => {
                    let next = b[i + 1..].iter().find(|c| !c.is_ascii_whitespace());
                    if matches!(next, Some(b']') | Some(b'}')) {
                        out[i] = b' ';
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    // only ASCII bytes were replaced with ASCII, so this stays UTF-8
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

fn yaml(text: &str) -> Option<Found> {
    let e = yaml_rust2::YamlLoader::load_from_str(text).err()?;
    // the marker's index counts characters
    let at = text.char_indices().nth(e.marker().index()).map_or(text.len(), |(i, _)| i);
    Some(error(at, format!("Invalid YAML: {}", e.info())))
}

fn toml_check(text: &str) -> Option<Found> {
    let e = text.parse::<toml::Table>().err()?;
    let span = e.span().unwrap_or(0..0);
    let message = e.message().trim().trim_end_matches('.').to_string();
    Some(Found { from: span.start, to: span.end, message: format!("Invalid TOML: {message}"), error: true })
}

fn xml(text: &str) -> Option<Found> {
    if text.trim().is_empty() {
        return None;
    }
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let e = roxmltree::Document::parse_with_options(text, opts).err()?;
    let pos = e.pos();
    let message = e.to_string();
    let message = message.rsplit_once(" at ").map_or(message.as_str(), |(m, _)| m).to_string();
    let at = offset(text, pos.row as usize, (pos.col as usize).saturating_sub(1), true);
    Some(error(at, format!("Invalid XML: {message}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(text: &str, language: &str, path: &str) -> Option<(usize, String)> {
        check(text, language, path).unwrap().into_iter().next().map(|f| (f.from, f.message))
    }

    #[test]
    fn valid_data_has_no_problems() {
        assert_eq!(first("{\"a\": [1, 2], \"b\": null}", "JSON", "a.json"), None);
        assert_eq!(first("name: x\nlist:\n  - 1\n  - 2\n", "YAML", "a.yaml"), None);
        assert_eq!(first("[package]\nname = \"x\"\n\n[[bin]]\nname = \"y\"\n", "TOML", "Cargo.toml"), None);
        assert_eq!(first("<?xml version=\"1.0\"?>\n<a><b x=\"1\"/></a>\n", "XML", "a.xml"), None);
        assert_eq!(first("", "JSON", "a.json"), None);
        assert!(check("fn main() {}", "Rust", "a.rs").is_none());
    }

    #[test]
    fn json_errors_point_at_the_problem() {
        let (at, msg) = first("{\n  \"a\": 1\n  \"b\": 2\n}", "JSON", "a.json").unwrap();
        // line 3, at the "b"
        assert_eq!(at, 13, "{msg}");
        assert!(msg.starts_with("Invalid JSON: expected `,` or `}`"), "{msg}");
        let (_, msg) = first("{\"a\": [1, 2", "JSON", "a.json").unwrap();
        assert!(msg.contains("the text ends inside"), "{msg}");
    }

    #[test]
    fn jsonc_allows_comments_and_trailing_commas() {
        let text = "{\n  // the theme\n  \"a\": 1, /* x */\n}\n";
        assert_eq!(first(text, "JSON", "C:\\Users\\x\\settings.json"), None);
        assert!(first(text, "JSON", "data.json").is_some());
        assert_eq!(first("{a: 1}", "JSON", "x.json5"), None);
    }

    #[test]
    fn yaml_toml_and_xml_errors() {
        let (at, msg) = first("a: 1\nb: [1, 2\nc: 3\n", "YAML", "a.yaml").unwrap();
        assert!(msg.starts_with("Invalid YAML"), "{msg}");
        assert!(at > 5, "{at}");
        let (at, msg) = first("[package]\nname = \"x\nversion = 1\n", "TOML", "a.toml").unwrap();
        assert!(msg.starts_with("Invalid TOML"), "{msg}");
        assert!((10..30).contains(&at), "{at}");
        let (at, msg) = first("<a>\n  <b>\n</a>\n", "XML", "a.xml").unwrap();
        assert!(msg.starts_with("Invalid XML"), "{msg}");
        assert_eq!(&"<a>\n  <b>\n</a>\n"[at..at + 1], "<", "{msg}");
    }
}
