//! Imported color themes: VS Code color-theme JSON/JSONC files and TextMate
//! `.tmTheme` files. Workbench colors are mapped onto Notey's application
//! palette, while TextMate token rules drive syntax highlighting. Carried
//! over from the egui version, with GPUI colors.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use gpui::{Hsla, Rgba};
use serde_json::Value;
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSet};

use crate::theme::{palette, Palette};

#[derive(Clone)]
pub struct ImportedTheme {
    pub name: String,
    pub path: PathBuf,
    pub dark: bool,
    pub palette: Palette,
    pub syntax: Arc<Theme>,
}

#[derive(Default)]
struct RawTheme {
    name: Option<String>,
    kind: Option<String>,
    colors: BTreeMap<String, String>,
    syntax: Theme,
}

/// An sRGB color with straight alpha, as theme files write them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgba8([u8; 4]);

impl Rgba8 {
    const WHITE: Rgba8 = Rgba8([255, 255, 255, 255]);
    const BLACK: Rgba8 = Rgba8([0, 0, 0, 255]);

    fn rgb(r: u8, g: u8, b: u8) -> Rgba8 {
        Rgba8([r, g, b, 255])
    }

    fn from_hsla(c: Hsla) -> Rgba8 {
        let c = Rgba::from(c);
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba8([b(c.r), b(c.g), b(c.b), b(c.a)])
    }

    fn hsla(self) -> Hsla {
        let [r, g, b, a] = self.0;
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }.into()
    }
}

/// Load a VS Code color-theme JSON/JSONC file or a TextMate `.tmTheme` file.
pub fn load_vscode_theme(path: &Path) -> Result<ImportedTheme, String> {
    let tm = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"));
    let mut raw = if tm { RawTheme { syntax: load_tm_theme(path)?, ..RawTheme::default() } } else { load_json_theme(path, 0)? };

    let editor_bg =
        color_from_map(&raw.colors, &["editor.background"]).or_else(|| raw.syntax.settings.background.map(from_syntect));
    let dark = raw
        .kind
        .as_deref()
        .map(|kind| !kind.to_ascii_lowercase().contains("light"))
        .unwrap_or_else(|| editor_bg.map(is_dark_color).unwrap_or(true));
    let base = palette(dark);
    let colors = &raw.colors;
    let pick = |keys: &[&str], fallback: Hsla| color_from_map(colors, keys).map(Rgba8::hsla).unwrap_or(fallback);

    let mut p = base;
    p.editor = editor_bg.map(Rgba8::hsla).unwrap_or(base.editor);
    p.text = color_from_map(colors, &["editor.foreground", "foreground"])
        .or_else(|| raw.syntax.settings.foreground.map(from_syntect))
        .map(Rgba8::hsla)
        .unwrap_or(base.text);
    p.chrome = pick(
        &["titleBar.activeBackground", "statusBar.background", "sideBar.background", "tab.inactiveBackground"],
        base.chrome,
    );
    p.popup = pick(&["menu.background", "editorWidget.background", "dropdown.background"], base.popup);
    p.control = pick(&["input.background", "dropdown.background", "button.secondaryBackground"], base.control);
    p.control_hover =
        pick(&["menu.selectionBackground", "list.hoverBackground", "button.hoverBackground"], base.control_hover);
    p.control_active = pick(&["list.activeSelectionBackground", "button.background"], base.control_active);
    p.stroke = pick(&["menu.border", "editorWidget.border", "input.border", "contrastBorder"], base.stroke);
    p.text_weak =
        pick(&["descriptionForeground", "editorLineNumber.foreground", "tab.inactiveForeground"], base.text_weak);
    p.accent = pick(&["focusBorder", "activityBarBadge.background", "button.background"], base.accent);
    p.selection = pick(&["editor.selectionBackground", "selection.background"], base.selection);
    p.tab_hover = pick(&["tab.hoverBackground", "list.hoverBackground"], base.tab_hover);
    p.close_hover = pick(
        &["toolbar.hoverBackground", "menubar.selectionBackground", "statusBarItem.errorBackground"],
        base.close_hover,
    );
    p.find_match = pick(&["editor.findMatchHighlightBackground"], base.find_match);
    p.find_match_selected = pick(&["editor.findMatchBackground"], base.find_match_selected);
    // The theme's own cursor color; the accent (focus borders, badges) is
    // often too dim or too close to the editor background to find a caret.
    let theme_caret =
        color_from_map(colors, &["editorCursor.foreground"]).or_else(|| raw.syntax.settings.caret.map(from_syntect));
    p.caret = readable_caret(theme_caret, Rgba8::from_hsla(p.editor), Rgba8::from_hsla(p.text)).hsla();

    raw.syntax.name = raw
        .name
        .clone()
        .or_else(|| raw.syntax.name.clone())
        .or_else(|| path.file_stem().map(|name| name.to_string_lossy().into_owned()));
    raw.syntax.settings.background = Some(to_syntect(Rgba8::from_hsla(p.editor)));
    raw.syntax.settings.foreground = Some(to_syntect(Rgba8::from_hsla(p.text)));

    let name = raw.syntax.name.clone().unwrap_or_else(|| "Imported theme".into());
    Ok(ImportedTheme { name, path: path.to_path_buf(), dark, palette: p, syntax: Arc::new(raw.syntax) })
}

fn load_json_theme(path: &Path, depth: usize) -> Result<RawTheme, String> {
    if depth > 12 {
        return Err("theme include nesting is too deep".into());
    }
    let source = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    let clean = strip_jsonc(&source);
    let value: Value = serde_json::from_str(&clean)
        .map_err(|err| format!("invalid VS Code theme {}: {err}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| "theme file must contain a JSON object".to_string())?;

    let mut raw = if let Some(include) = object.get("include").and_then(Value::as_str) {
        let included = path.parent().unwrap_or_else(|| Path::new(".")).join(include);
        load_json_theme(&included, depth + 1)?
    } else {
        RawTheme::default()
    };

    if let Some(name) = object.get("name").and_then(Value::as_str) {
        raw.name = Some(name.to_string());
    }
    if let Some(kind) = object.get("type").and_then(Value::as_str) {
        raw.kind = Some(kind.to_string());
    }
    if let Some(colors) = object.get("colors").and_then(Value::as_object) {
        for (key, value) in colors {
            if let Some(value) = value.as_str() {
                raw.colors.insert(key.clone(), value.to_string());
            }
        }
    }
    if let Some(token_colors) = object.get("tokenColors") {
        match token_colors {
            Value::Array(rules) => append_token_rules(&mut raw.syntax, rules),
            Value::String(relative) => {
                let token_path = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(relative);
                let mut tokens = if token_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"))
                {
                    load_tm_theme(&token_path)?
                } else {
                    load_json_theme(&token_path, depth + 1)?.syntax
                };
                merge_theme(&mut raw.syntax, &mut tokens);
            }
            _ => {}
        }
    }
    Ok(raw)
}

fn load_tm_theme(path: &Path) -> Result<Theme, String> {
    let file = std::fs::File::open(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    let mut reader = BufReader::new(file);
    ThemeSet::load_from_reader(&mut reader)
        .map_err(|err| format!("invalid TextMate theme {}: {err}", path.display()))
}

fn merge_theme(base: &mut Theme, overlay: &mut Theme) {
    if overlay.settings.foreground.is_some() {
        base.settings.foreground = overlay.settings.foreground;
    }
    if overlay.settings.background.is_some() {
        base.settings.background = overlay.settings.background;
    }
    base.scopes.append(&mut overlay.scopes);
}

fn append_token_rules(theme: &mut Theme, rules: &[Value]) {
    for rule in rules {
        let Some(object) = rule.as_object() else {
            continue;
        };
        let Some(settings) = object.get("settings").and_then(Value::as_object) else {
            continue;
        };
        let foreground = settings
            .get("foreground")
            .and_then(Value::as_str)
            .and_then(parse_syntect_color);
        let background = settings
            .get("background")
            .and_then(Value::as_str)
            .and_then(parse_syntect_color);
        let font_style = settings
            .get("fontStyle")
            .and_then(Value::as_str)
            .map(parse_font_style);
        let scopes = match object.get("scope") {
            Some(Value::String(scope)) => scope.clone(),
            Some(Value::Array(scopes)) => scopes
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        };
        if scopes.trim().is_empty() {
            if foreground.is_some() {
                theme.settings.foreground = foreground;
            }
            if background.is_some() {
                theme.settings.background = background;
            }
            continue;
        }
        if let Ok(scope) = ScopeSelectors::from_str(&scopes) {
            theme.scopes.push(ThemeItem {
                scope,
                style: StyleModifier {
                    foreground,
                    background,
                    font_style,
                },
            });
        }
    }
}

fn parse_font_style(value: &str) -> FontStyle {
    let mut style = FontStyle::empty();
    for part in value.split_whitespace() {
        match part {
            "bold" => style.insert(FontStyle::BOLD),
            "italic" => style.insert(FontStyle::ITALIC),
            "underline" => style.insert(FontStyle::UNDERLINE),
            _ => {}
        }
    }
    style
}


fn parse_syntect_color(value: &str) -> Option<Color> {
    parse_rgba(value).map(|[r, g, b, a]| Color { r, g, b, a })
}

fn parse_rgba(value: &str) -> Option<[u8; 4]> {
    let hex = value.trim().strip_prefix('#')?;
    let byte = |pair: &str| u8::from_str_radix(pair, 16).ok();
    match hex.len() {
        3 => Some([byte(&hex[0..1].repeat(2))?, byte(&hex[1..2].repeat(2))?, byte(&hex[2..3].repeat(2))?, 255]),
        4 => Some([
            byte(&hex[0..1].repeat(2))?,
            byte(&hex[1..2].repeat(2))?,
            byte(&hex[2..3].repeat(2))?,
            byte(&hex[3..4].repeat(2))?,
        ]),
        6 => Some([byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255]),
        8 => Some([byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, byte(&hex[6..8])?]),
        _ => None,
    }
}

fn color_from_map(colors: &BTreeMap<String, String>, keys: &[&str]) -> Option<Rgba8> {
    keys.iter().find_map(|key| colors.get(*key).and_then(|value| parse_rgba(value).map(Rgba8)))
}

fn is_dark_color(color: Rgba8) -> bool {
    let [r, g, b, _] = color.0;
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) < 128.0
}

/// WCAG contrast ratio between two opaque colors (1.0 to 21.0).
fn contrast_ratio(a: Rgba8, b: Rgba8) -> f32 {
    fn luminance(c: Rgba8) -> f32 {
        let channel = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b, _] = c.0;
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The caret color for an editor background: the theme's cursor color
/// (blended over the background if translucent) when it stands out at the
/// 3:1 contrast WCAG asks of UI indicators, otherwise the text color, and
/// black or white as a last resort.
fn readable_caret(theme_caret: Option<Rgba8>, bg: Rgba8, text: Rgba8) -> Rgba8 {
    const MIN_CONTRAST: f32 = 3.0;
    let [br, bgr, bb, _] = bg.0;
    let bg = Rgba8::rgb(br, bgr, bb);
    let over_bg = |c: Rgba8| {
        let [r, g, b, a] = c.0;
        let a = a as f32 / 255.0;
        let mix = |fg: u8, bg: u8| (fg as f32 * a + bg as f32 * (1.0 - a)).round() as u8;
        Rgba8::rgb(mix(r, br), mix(g, bgr), mix(b, bb))
    };
    theme_caret
        .into_iter()
        .chain([text])
        .map(over_bg)
        .find(|&c| contrast_ratio(c, bg) >= MIN_CONTRAST)
        .unwrap_or(if is_dark_color(bg) { Rgba8::WHITE } else { Rgba8::BLACK })
}

fn from_syntect(color: Color) -> Rgba8 {
    Rgba8([color.r, color.g, color.b, color.a])
}

fn to_syntect(color: Rgba8) -> Color {
    let [r, g, b, a] = color.0;
    Color { r, g, b, a }
}

/// Remove JavaScript-style comments and trailing commas accepted by VS Code's
/// JSON-with-comments parser while preserving string contents.
fn strip_jsonc(source: &str) -> String {
    let mut without_comments = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            without_comments.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            without_comments.push(ch);
        } else if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    without_comments.push('\n');
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for next in chars.by_ref() {
                if next == '\n' {
                    without_comments.push('\n');
                }
                if previous == '*' && next == '/' {
                    break;
                }
                previous = next;
            }
        } else {
            without_comments.push(ch);
        }
    }

    let chars: Vec<char> = without_comments.chars().collect();
    let mut out = String::with_capacity(without_comments.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, &ch) in chars.iter().enumerate() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == ',' {
            let next = chars[index + 1..]
                .iter()
                .copied()
                .find(|next| !next.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_theme_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("notey-theme-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn hex(v: u32) -> Hsla {
        Rgba8::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8).hsla()
    }

    #[test]
    fn parses_vscode_jsonc_into_app_and_syntax_colors() {
        let dir = temp_theme_dir();
        let path = dir.join("ocean-color-theme.json");
        std::fs::write(
            &path,
            r##"{
                // VS Code themes allow comments and trailing commas.
                "name": "Ocean Test",
                "type": "dark",
                "colors": {
                    "editor.background": "#102030",
                    "editor.foreground": "#d0e0f0",
                    "titleBar.activeBackground": "#081018",
                    "menu.background": "#182838",
                    "editor.selectionBackground": "#336699aa",
                },
                "tokenColors": [
                    { "settings": { "foreground": "#d0e0f0", "background": "#102030" } },
                    {
                        "scope": ["comment", "punctuation.definition.comment"],
                        "settings": { "foreground": "#55aa77", "fontStyle": "italic" },
                    },
                ],
            }"##,
        )
        .unwrap();

        let imported = load_vscode_theme(&path).unwrap();
        assert_eq!(imported.name, "Ocean Test");
        assert!(imported.dark);
        assert_eq!(imported.palette.editor, hex(0x102030));
        assert_eq!(imported.palette.chrome, hex(0x081018));
        assert_eq!(imported.palette.popup, hex(0x182838));
        assert_eq!(Rgba8::from_hsla(imported.palette.selection).0[3], 0xaa);
        assert_eq!(imported.syntax.scopes.len(), 1);
        assert_eq!(imported.syntax.settings.foreground, Some(Color { r: 0xd0, g: 0xe0, b: 0xf0, a: 0xff }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn included_theme_colors_are_inherited_and_overridden() {
        let dir = temp_theme_dir();
        std::fs::write(
            dir.join("base.json"),
            r##"{
                "type": "light",
                "colors": {
                    "editor.background": "#ffffff",
                    "titleBar.activeBackground": "#eeeeee"
                },
                "tokenColors": [
                    { "scope": "comment", "settings": { "foreground": "#008000" } }
                ]
            }"##,
        )
        .unwrap();
        let child = dir.join("child.json");
        std::fs::write(
            &child,
            r##"{
                "name": "Child",
                "include": "./base.json",
                "colors": { "editor.background": "#fafafa" },
                "tokenColors": [
                    { "scope": "string", "settings": { "foreground": "#a31515" } }
                ]
            }"##,
        )
        .unwrap();

        let imported = load_vscode_theme(&child).unwrap();
        assert!(!imported.dark);
        assert_eq!(imported.palette.editor, hex(0xfafafa));
        assert_eq!(imported.palette.chrome, hex(0xeeeeee));
        assert_eq!(imported.syntax.scopes.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn caret_uses_theme_cursor_color_only_when_visible() {
        let bg = Rgba8::rgb(0x1e, 0x1e, 0x1e);
        let text = Rgba8::rgb(0xd4, 0xd4, 0xd4);
        let orange = Rgba8::rgb(0xff, 0x98, 0x00);
        assert_eq!(readable_caret(Some(orange), bg, text), orange);
        // A cursor color nearly the background's falls back to the text color.
        let dim = Rgba8::rgb(0x2a, 0x2a, 0x30);
        assert_eq!(readable_caret(Some(dim), bg, text), text);
        assert_eq!(readable_caret(None, bg, text), text);
        // Translucent colors are judged as drawn, over the background.
        let faint = Rgba8([0xff, 0xff, 0xff, 0x10]);
        assert_eq!(readable_caret(Some(faint), bg, text), text);
        // Unreadable text too: plain white on a dark editor.
        assert_eq!(readable_caret(None, bg, bg), Rgba8::WHITE);
        // Notey's own themes keep their accent caret, which is readable.
        for dark in [true, false] {
            let p = palette(dark);
            assert!(contrast_ratio(Rgba8::from_hsla(p.caret), Rgba8::from_hsla(p.editor)) >= 3.0);
        }
    }

    #[test]
    fn parses_short_and_alpha_hex_colors() {
        assert_eq!(parse_rgba("#abc"), Some([0xaa, 0xbb, 0xcc, 0xff]));
        assert_eq!(parse_rgba("#abcd"), Some([0xaa, 0xbb, 0xcc, 0xdd]));
        assert_eq!(parse_rgba("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
        assert_eq!(parse_rgba("red"), None);
    }
}
