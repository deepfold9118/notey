//! Settings that plugins declare in their manifest (`settings: [...]`).
//! Notey shows them in a section of the Settings tab named after the
//! plugin, only while the plugin is enabled, and keeps the values in
//! `plugin_settings.json` in the data folder; a script reads them with
//! `notey.settings.get(id)`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rhai::{Array, Dynamic, Map};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A drop-down list.
    Choice,
    /// A line of text.
    Text,
    /// A file path, with a Browse button.
    Path,
    /// An on/off switch ("true" or "false").
    Toggle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: Kind,
    pub default: String,
    /// The choices of a drop-down: (value, label).
    pub choices: Vec<(String, String)>,
    /// A script function that lists the choices, called when the Settings
    /// section opens (so it can look for what is installed).
    pub choices_fn: Option<String>,
    /// Shown only while another setting has one of these values.
    pub show_if: Option<(String, Vec<String>)>,
}

fn text(m: &Map, key: &str) -> Option<String> {
    let v = m.get(key)?;
    if let Ok(b) = v.as_bool() {
        return Some(b.to_string());
    }
    v.clone().into_string().ok()
}

/// The `settings` of a manifest. Entries without an id are skipped.
pub fn parse(value: Option<&Dynamic>) -> Vec<Setting> {
    let list: Array = value.and_then(|v| v.clone().try_cast()).unwrap_or_default();
    list.into_iter()
        .filter_map(|item| {
            let m: Map = item.try_cast()?;
            let id = text(&m, "id").filter(|id| !id.is_empty())?;
            let kind = match text(&m, "kind").unwrap_or_default().as_str() {
                "choice" => Kind::Choice,
                "path" => Kind::Path,
                "toggle" => Kind::Toggle,
                _ => Kind::Text,
            };
            let (choices, choices_fn) = match m.get("choices") {
                Some(v) if v.is_string() => (Vec::new(), v.clone().into_string().ok()),
                Some(v) => (parse_choices(v.clone()), None),
                None => (Vec::new(), None),
            };
            let show_if = m.get("show_if").and_then(|v| v.clone().try_cast::<Map>()).and_then(|w| {
                let setting = text(&w, "setting")?;
                let values = match w.get("value") {
                    Some(v) if v.is_array() => v.clone().cast::<Array>().into_iter().filter_map(|x| x.into_string().ok()).collect(),
                    Some(v) => v.clone().into_string().ok().into_iter().collect(),
                    None => Vec::new(),
                };
                Some((setting, values))
            });
            let default = text(&m, "initial").unwrap_or_else(|| if kind == Kind::Toggle { "false".into() } else { String::new() });
            Some(Setting {
                title: text(&m, "title").unwrap_or_else(|| id.clone()),
                description: text(&m, "description").unwrap_or_default(),
                id,
                kind,
                default,
                choices,
                choices_fn,
                show_if,
            })
        })
        .collect()
}

/// Choices as a script gives them: `[value, label]` pairs, `#{ value,
/// label }` maps or plain strings.
pub fn parse_choices(v: Dynamic) -> Vec<(String, String)> {
    let list: Array = v.try_cast().unwrap_or_default();
    list.into_iter()
        .filter_map(|c| {
            if let Some(pair) = c.clone().try_cast::<Array>() {
                let value = pair.first()?.clone().into_string().ok()?;
                let label = pair.get(1).and_then(|l| l.clone().into_string().ok()).unwrap_or_else(|| value.clone());
                return Some((value, label));
            }
            if let Some(m) = c.clone().try_cast::<Map>() {
                let value = text(&m, "value")?;
                let label = text(&m, "label").unwrap_or_else(|| value.clone());
                return Some((value, label));
            }
            let s = c.into_string().ok()?;
            Some((s.clone(), s))
        })
        .collect()
}

/// The saved values of every plugin, by plugin name, then setting id.
pub type Store = BTreeMap<String, BTreeMap<String, String>>;

fn file() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("plugin_settings.json"))
}

pub fn load() -> Store {
    file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(store: &Store) {
    let Some(path) = file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(store) {
        let _ = std::fs::write(path, json);
    }
}

/// The values of a plugin's settings: what was saved, else the defaults.
pub fn values(store: &Store, plugin: &str, settings: &[Setting]) -> BTreeMap<String, String> {
    let saved = store.get(plugin);
    settings
        .iter()
        .map(|s| (s.id.clone(), saved.and_then(|v| v.get(&s.id)).cloned().unwrap_or_else(|| s.default.clone())))
        .collect()
}

/// Whether `setting` shows, given the values of the plugin's settings.
pub fn visible(setting: &Setting, values: &BTreeMap<String, String>) -> bool {
    setting.show_if.as_ref().is_none_or(|(other, wanted)| values.get(other).is_some_and(|v| wanted.contains(v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_from_a_manifest() {
        let engine = rhai::Engine::new();
        let v: Dynamic = engine
            .eval(
                r#"[
                #{ id: "terminal", title: "Terminal", kind: "choice", choices: "list_terminals", initial: "auto" },
                #{ id: "path", kind: "path", show_if: #{ setting: "terminal", value: "other" } },
                #{ id: "size", kind: "choice", choices: [["s", "Small"], "large"] },
                #{ id: "on", kind: "toggle", initial: true },
                #{ title: "no id" },
            ]"#,
            )
            .unwrap();
        let s = parse(Some(&v));
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].choices_fn.as_deref(), Some("list_terminals"));
        assert_eq!(s[1].kind, Kind::Path);
        assert_eq!(s[1].title, "path");
        assert_eq!(s[2].choices, vec![("s".into(), "Small".into()), ("large".into(), "large".into())]);
        assert_eq!(s[3].default, "true");
        let mut values = values(&Store::new(), "x", &s);
        assert_eq!(values["terminal"], "auto");
        assert!(!visible(&s[1], &values));
        values.insert("terminal".into(), "other".into());
        assert!(visible(&s[1], &values));
    }
}
