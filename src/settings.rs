//! Settings, saved as JSON in the data folder. When there is no settings
//! file yet, Notey imports the settings that the egui version kept in
//! `app.ron`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The folder for the settings and the session: `%APPDATA%\Notey\data`,
/// the folder that the egui version used, so both carry over. The
/// `NOTEY_DATA_DIR` environment variable replaces it, for testing.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("NOTEY_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?;
    Some(base.join("Notey").join("data"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub word_wrap: bool,
    pub show_status: bool,
    /// Show the menu bar row. When off, the menus open from the title-bar icon.
    pub show_menu: bool,
    pub line_numbers: bool,
    /// Underline misspelled words (needs the Spellcheck plugin and a
    /// Language plugin).
    pub spellcheck: bool,
    /// Editor font size in pixels, before zoom.
    pub font_size: f32,
    pub editor_font: String,
    /// Line height multiplier; 1.0 is the default spacing.
    pub line_height: f32,
    /// Tab width in spaces.
    pub tab_size: usize,
    /// Editor zoom in percent.
    pub zoom: f32,
    /// Font ligatures (such as `->` drawn as an arrow) in the editor.
    pub ligatures: bool,
    /// Show the tabs as a vertical list on the left, not in the title bar.
    pub tabs_left: bool,
    /// The width of the tab list on the left.
    pub tabs_left_width: f32,
    /// The imported VS Code or TextMate theme in use, if any.
    pub theme_path: Option<PathBuf>,
    /// Save modified files every `autosave_secs` seconds.
    pub autosave: bool,
    pub autosave_secs: f32,
    pub recent_files: Vec<PathBuf>,
    /// The window position and size when it last closed.
    pub window: Option<WindowGeometry>,
}

/// A window position and size, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            word_wrap: true,
            show_status: true,
            show_menu: true,
            line_numbers: false,
            spellcheck: true,
            font_size: 15.0,
            editor_font: "Consolas".into(),
            line_height: 1.0,
            tab_size: 4,
            zoom: 100.0,
            ligatures: true,
            tabs_left: false,
            tabs_left_width: 200.0,
            theme_path: None,
            autosave: false,
            autosave_secs: 30.0,
            recent_files: Vec::new(),
            window: None,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let Some(dir) = data_dir() else { return Settings::default() };
        if let Ok(bytes) = std::fs::read(dir.join("settings.json")) {
            if let Ok(settings) = serde_json::from_slice(&bytes) {
                return settings;
            }
        }
        std::fs::read_to_string(dir.join("app.ron"))
            .ok()
            .and_then(|s| ron::from_str::<HashMap<String, String>>(&s).ok())
            .map(|map| Settings::from_egui(&map))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(dir) = data_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(dir.join("settings.json"), json);
        }
    }

    /// The editor font size after zoom.
    pub fn zoomed_font_size(&self) -> f32 {
        (self.font_size * self.zoom / 100.0).max(4.0)
    }

    /// Read the string map that eframe stored for the egui version.
    fn from_egui(map: &HashMap<String, String>) -> Settings {
        let mut s = Settings::default();
        let flag = |key: &str, into: &mut bool| {
            if let Some(v) = map.get(key) {
                *into = v == "1";
            }
        };
        flag("dark", &mut s.dark);
        flag("word_wrap", &mut s.word_wrap);
        flag("show_status", &mut s.show_status);
        flag("menu_bar", &mut s.show_menu);
        flag("line_numbers", &mut s.line_numbers);
        flag("spell", &mut s.spellcheck);
        flag("ligatures", &mut s.ligatures);
        flag("tabs_left", &mut s.tabs_left);
        flag("autosave", &mut s.autosave);
        if let Some(v) = map.get("theme_path").filter(|v| !v.is_empty()) {
            s.theme_path = Some(PathBuf::from(v));
        }
        let num = |key: &str| map.get(key).and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = num("font_size") {
            s.font_size = v;
        }
        if let Some(v) = num("zoom") {
            s.zoom = v;
        }
        if let Some(v) = num("line_height") {
            s.line_height = v;
        }
        if let Some(v) = num("autosave_secs") {
            s.autosave_secs = v;
        }
        if let Some(v) = num("tab_size") {
            s.tab_size = v.round().max(1.0) as usize;
        }
        if let Some(v) = map.get("editor_font").filter(|v| !v.is_empty()) {
            s.editor_font = v.clone();
        }
        if let Some(recent) = map.get("recent_files").and_then(|v| serde_json::from_str::<Vec<String>>(v).ok()) {
            s.recent_files = recent.into_iter().map(PathBuf::from).collect();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_the_egui_settings() {
        let ron = r#"{"dark": "0", "font_size": "12", "tab_size": "8", "menu_bar": "0",
            "recent_files": "[\"C:\\\\a.txt\"]", "egui": "(options:())"}"#;
        let map: HashMap<String, String> = ron::from_str(ron).unwrap();
        let s = Settings::from_egui(&map);
        assert!(!s.dark);
        assert!(!s.show_menu);
        assert_eq!(s.font_size, 12.0);
        assert_eq!(s.tab_size, 8);
        assert_eq!(s.recent_files, vec![PathBuf::from(r"C:\a.txt")]);
        assert!(s.word_wrap);
    }
}
