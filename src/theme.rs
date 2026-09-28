//! Fluent (WinUI-inspired) theming for Notey — dark and light variants.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, Shadow, Stroke, Visuals,
};
use serde_json::Value;
use syntect::highlighting::{
    Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSet,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Title bar, tab strip, menu row, status bar.
    pub chrome: Color32,
    /// The text editing surface.
    pub editor: Color32,
    /// Menus, dialogs, popups.
    pub popup: Color32,
    /// Buttons and inputs at rest.
    pub control: Color32,
    pub control_hover: Color32,
    pub control_active: Color32,
    /// Hairline borders.
    pub stroke: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// Fluent accent (focus rings, toggles, links).
    pub accent: Color32,
    /// Text selection background.
    pub selection: Color32,
    /// Text cursor; always contrasts with `editor`.
    pub caret: Color32,
    /// Tab hover wash.
    pub tab_hover: Color32,
    /// Close-button hover (caption bar).
    pub close_hover: Color32,
}

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

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            chrome: Color32::from_rgb(0x20, 0x20, 0x20),
            editor: Color32::from_rgb(0x28, 0x28, 0x28),
            popup: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            control: Color32::from_rgb(0x2D, 0x2D, 0x2D),
            control_hover: Color32::from_rgb(0x35, 0x35, 0x35),
            control_active: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            stroke: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            text: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            text_weak: Color32::from_rgb(0x9D, 0x9D, 0x9D),
            accent: Color32::from_rgb(0x60, 0xCD, 0xFF),
            selection: Color32::from_rgb(0x26, 0x5C, 0x8C),
            caret: Color32::from_rgb(0x60, 0xCD, 0xFF),
            tab_hover: Color32::from_rgb(0x2A, 0x2A, 0x2A),
            close_hover: Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    } else {
        Palette {
            chrome: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            editor: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            popup: Color32::from_rgb(0xF9, 0xF9, 0xF9),
            control: Color32::from_rgb(0xFB, 0xFB, 0xFB),
            control_hover: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            control_active: Color32::from_rgb(0xE5, 0xE5, 0xE5),
            stroke: Color32::from_rgb(0xE0, 0xE0, 0xE0),
            text: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            text_weak: Color32::from_rgb(0x60, 0x60, 0x60),
            accent: Color32::from_rgb(0x00, 0x67, 0xC0),
            selection: Color32::from_rgb(0xA8, 0xCE, 0xF1),
            caret: Color32::from_rgb(0x00, 0x67, 0xC0),
            tab_hover: Color32::from_rgb(0xEA, 0xEA, 0xEA),
            close_hover: Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    }
}

/// Load a VS Code color-theme JSON/JSONC file or a TextMate `.tmTheme` file.
/// Workbench colors are mapped onto Notey's full application palette, while
/// TextMate token rules are retained for syntax highlighting.
pub fn load_vscode_theme(path: &Path) -> Result<ImportedTheme, String> {
    let mut raw = if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmTheme"))
    {
        RawTheme {
            syntax: load_tm_theme(path)?,
            ..RawTheme::default()
        }
    } else {
        load_json_theme(path, 0)?
    };

    let editor_bg = color_from_map(&raw.colors, &["editor.background"])
        .or_else(|| raw.syntax.settings.background.map(syntect_to_egui));
    let dark = raw
        .kind
        .as_deref()
        .map(|kind| !kind.to_ascii_lowercase().contains("light"))
        .unwrap_or_else(|| editor_bg.map(is_dark_color).unwrap_or(true));
    let mut app_palette = palette(dark);

    app_palette.editor = editor_bg.unwrap_or(app_palette.editor);
    app_palette.text = color_from_map(&raw.colors, &["editor.foreground", "foreground"])
        .or_else(|| raw.syntax.settings.foreground.map(syntect_to_egui))
        .unwrap_or(app_palette.text);
    app_palette.chrome = color_from_map(
        &raw.colors,
        &[
            "titleBar.activeBackground",
            "statusBar.background",
            "sideBar.background",
            "tab.inactiveBackground",
        ],
    )
    .unwrap_or(app_palette.chrome);
    app_palette.popup = color_from_map(
        &raw.colors,
        &["menu.background", "editorWidget.background", "dropdown.background"],
    )
    .unwrap_or(app_palette.popup);
    app_palette.control = color_from_map(
        &raw.colors,
        &["input.background", "dropdown.background", "button.secondaryBackground"],
    )
    .unwrap_or(app_palette.control);
    app_palette.control_hover = color_from_map(
        &raw.colors,
        &[
            "menu.selectionBackground",
            "list.hoverBackground",
            "button.hoverBackground",
        ],
    )
    .unwrap_or(app_palette.control_hover);
    app_palette.control_active = color_from_map(
        &raw.colors,
        &["list.activeSelectionBackground", "button.background"],
    )
    .unwrap_or(app_palette.control_active);
    app_palette.stroke = color_from_map(
        &raw.colors,
        &["menu.border", "editorWidget.border", "input.border", "contrastBorder"],
    )
    .unwrap_or(app_palette.stroke);
    app_palette.text_weak = color_from_map(
        &raw.colors,
        &[
            "descriptionForeground",
            "editorLineNumber.foreground",
            "tab.inactiveForeground",
        ],
    )
    .unwrap_or(app_palette.text_weak);
    app_palette.accent = color_from_map(
        &raw.colors,
        &[
            "focusBorder",
            "activityBarBadge.background",
            "button.background",
        ],
    )
    .unwrap_or(app_palette.accent);
    app_palette.selection = color_from_map(
        &raw.colors,
        &["editor.selectionBackground", "selection.background"],
    )
    .unwrap_or(app_palette.selection);
    app_palette.tab_hover = color_from_map(
        &raw.colors,
        &["tab.hoverBackground", "list.hoverBackground"],
    )
    .unwrap_or(app_palette.tab_hover);
    app_palette.close_hover = color_from_map(
        &raw.colors,
        &[
            "toolbar.hoverBackground",
            "menubar.selectionBackground",
            "statusBarItem.errorBackground",
        ],
    )
    .unwrap_or(app_palette.close_hover);
    // the theme's own cursor color; the accent (focus borders, badges) is
    // often too dim or too close to the editor background to find a caret
    let theme_caret = color_from_map(&raw.colors, &["editorCursor.foreground"])
        .or_else(|| raw.syntax.settings.caret.map(syntect_to_egui));
    app_palette.caret = readable_caret(theme_caret, app_palette.editor, app_palette.text);

    raw.syntax.name = raw
        .name
        .clone()
        .or_else(|| raw.syntax.name.clone())
        .or_else(|| {
            path.file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        });
    raw.syntax.settings.background = Some(egui_to_syntect(app_palette.editor));
    raw.syntax.settings.foreground = Some(egui_to_syntect(app_palette.text));

    let name = raw.syntax.name.clone().unwrap_or_else(|| "Imported theme".into());
    Ok(ImportedTheme {
        name,
        path: path.to_path_buf(),
        dark,
        palette: app_palette,
        syntax: Arc::new(raw.syntax),
    })
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

fn parse_egui_color(value: &str) -> Option<Color32> {
    parse_rgba(value).map(|[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn parse_rgba(value: &str) -> Option<[u8; 4]> {
    let hex = value.trim().strip_prefix('#')?;
    let byte = |pair: &str| u8::from_str_radix(pair, 16).ok();
    match hex.len() {
        3 => Some([
            byte(&hex[0..1].repeat(2))?,
            byte(&hex[1..2].repeat(2))?,
            byte(&hex[2..3].repeat(2))?,
            255,
        ]),
        4 => Some([
            byte(&hex[0..1].repeat(2))?,
            byte(&hex[1..2].repeat(2))?,
            byte(&hex[2..3].repeat(2))?,
            byte(&hex[3..4].repeat(2))?,
        ]),
        6 => Some([byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255]),
        8 => Some([
            byte(&hex[0..2])?,
            byte(&hex[2..4])?,
            byte(&hex[4..6])?,
            byte(&hex[6..8])?,
        ]),
        _ => None,
    }
}

fn color_from_map(colors: &BTreeMap<String, String>, keys: &[&str]) -> Option<Color32> {
    keys.iter()
        .find_map(|key| colors.get(*key).and_then(|value| parse_egui_color(value)))
}

fn is_dark_color(color: Color32) -> bool {
    let luminance = 0.2126 * color.r() as f32
        + 0.7152 * color.g() as f32
        + 0.0722 * color.b() as f32;
    luminance < 128.0
}

/// WCAG contrast ratio between two opaque colors (1.0 to 21.0).
fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The caret color for an editor background: the theme's cursor color
/// (blended over the background if translucent) when it stands out at the
/// 3:1 contrast WCAG asks of UI indicators, otherwise the text color, and
/// black or white as a last resort.
fn readable_caret(theme_caret: Option<Color32>, bg: Color32, text: Color32) -> Color32 {
    const MIN_CONTRAST: f32 = 3.0;
    let bg = Color32::from_rgb(bg.r(), bg.g(), bg.b());
    let over_bg = |c: Color32| {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let a = a as f32 / 255.0;
        let mix = |fg: u8, bg: u8| (fg as f32 * a + bg as f32 * (1.0 - a)).round() as u8;
        Color32::from_rgb(mix(r, bg.r()), mix(g, bg.g()), mix(b, bg.b()))
    };
    theme_caret
        .into_iter()
        .chain([text])
        .map(over_bg)
        .find(|&c| contrast_ratio(c, bg) >= MIN_CONTRAST)
        .unwrap_or(if is_dark_color(bg) { Color32::WHITE } else { Color32::BLACK })
}

fn syntect_to_egui(color: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a)
}

fn egui_to_syntect(color: Color32) -> Color {
    Color {
        r: color.r(),
        g: color.g(),
        b: color.b(),
        a: color.a(),
    }
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

/// Apply a palette to every egui widget and application surface.
pub fn apply_palette(ctx: &egui::Context, dark: bool, p: Palette) {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.panel_fill = p.chrome;
    v.window_fill = p.popup;
    v.extreme_bg_color = p.editor;
    v.faint_bg_color = p.control;

    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;

    let r4 = CornerRadius::same(4);
    v.widgets.noninteractive.bg_fill = p.chrome;
    v.widgets.noninteractive.weak_bg_fill = p.chrome;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = r4;

    v.widgets.inactive.bg_fill = p.control;
    v.widgets.inactive.weak_bg_fill = p.control;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = r4;

    v.widgets.hovered.bg_fill = p.control_hover;
    v.widgets.hovered.weak_bg_fill = p.control_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = r4;

    v.widgets.active.bg_fill = p.control_active;
    v.widgets.active.weak_bg_fill = p.control_active;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.corner_radius = r4;

    v.widgets.open.bg_fill = p.control_active;
    v.widgets.open.weak_bg_fill = p.control_active;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.open.corner_radius = r4;

    v.window_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, p.stroke);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 96 } else { 48 }),
    };
    v.menu_corner_radius = CornerRadius::same(8);
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 40 }),
    };

    v.slider_trailing_fill = true;
    v.striped = false;

    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |style| {
        style.visuals = v;
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(14);
        style.spacing.scroll = egui::style::ScrollStyle {
            floating: true,
            bar_width: 8.0,
            floating_allocated_width: 8.0,
            ..egui::style::ScrollStyle::floating()
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_theme_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notey-theme-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        assert_eq!(imported.palette.editor, Color32::from_rgb(0x10, 0x20, 0x30));
        assert_eq!(imported.palette.chrome, Color32::from_rgb(0x08, 0x10, 0x18));
        assert_eq!(imported.palette.popup, Color32::from_rgb(0x18, 0x28, 0x38));
        assert_eq!(imported.palette.selection.a(), 0xaa);
        assert_eq!(imported.syntax.scopes.len(), 1);
        assert_eq!(
            imported.syntax.settings.foreground,
            Some(Color {
                r: 0xd0,
                g: 0xe0,
                b: 0xf0,
                a: 0xff,
            })
        );
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
        assert_eq!(imported.palette.editor, Color32::from_rgb(0xfa, 0xfa, 0xfa));
        assert_eq!(imported.palette.chrome, Color32::from_rgb(0xee, 0xee, 0xee));
        assert_eq!(imported.syntax.scopes.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn caret_uses_theme_cursor_color_only_when_visible() {
        let bg = Color32::from_rgb(0x1e, 0x1e, 0x1e);
        let text = Color32::from_rgb(0xd4, 0xd4, 0xd4);
        let orange = Color32::from_rgb(0xff, 0x98, 0x00);
        assert_eq!(readable_caret(Some(orange), bg, text), orange);
        // a cursor color nearly the background's falls back to the text color
        let dim = Color32::from_rgb(0x2a, 0x2a, 0x30);
        assert_eq!(readable_caret(Some(dim), bg, text), text);
        assert_eq!(readable_caret(None, bg, text), text);
        // translucent colors are judged as drawn, over the background
        let faint = Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x10);
        assert_eq!(readable_caret(Some(faint), bg, text), text);
        // unreadable text too: plain white on a dark editor
        assert_eq!(readable_caret(None, bg, bg), Color32::WHITE);
        // Notey's own themes keep their accent caret, which is readable
        for dark in [true, false] {
            let p = palette(dark);
            assert!(contrast_ratio(p.caret, p.editor) >= 3.0);
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
