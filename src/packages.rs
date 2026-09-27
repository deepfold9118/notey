//! Plugin packages: the official registry (`plugins/index.json` in the Notey
//! repository), the local store of installed plugins, and their
//! enabled/disabled state.
//!
//! Three kinds of package exist:
//! - `feature`: a Rhai script (`script`) and/or a built-in engine that the
//!   package switches on (`native`, e.g. the spellcheck engine),
//! - `language`: a Hunspell dictionary used by the Spellcheck feature,
//! - `filetype`: Sublime grammars that give a file type syntax highlighting.
//!
//! Every downloaded file is checked against the SHA-256 recorded in the
//! registry index before it is installed.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Official registry: the `plugins/` folder of the Notey repository.
pub const DEFAULT_REGISTRY: &str =
    "https://raw.githubusercontent.com/deepfold9118/notey/main/plugins";
/// Per-file download cap (the largest official dictionary is ~9 MB).
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Feature,
    Language,
    Filetype,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Feature => "Features",
            Kind::Language => "Languages",
            Kind::Filetype => "File Types",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    /// Registry-relative folder (e.g. `languages/lang-en-us`).
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    /// Language plugins: Hunspell locale (e.g. `en_US`).
    #[serde(default)]
    pub locale: Option<String>,
    /// File type plugins: file extensions the grammars claim.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Other plugins that improve this one (e.g. code-block languages).
    #[serde(default)]
    pub recommends: Vec<String>,
    /// Feature plugins: built-in engine this package switches on.
    #[serde(default)]
    pub native: Option<String>,
    /// Feature plugins: entry script file.
    #[serde(default)]
    pub script: Option<String>,
    /// Feature plugins: extra permissions the script needs.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

impl Installed {
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    disabled: Vec<String>,
    /// Bundled default plugins have been offered once (first run/upgrade).
    #[serde(default)]
    defaults_seeded: bool,
}

#[derive(Serialize, Deserialize)]
struct Index {
    schema: u32,
    plugins: Vec<Manifest>,
}

/// Installed plugins and their enabled state.
pub struct Store {
    root: PathBuf,
    pub installed: Vec<Installed>,
    disabled: HashSet<String>,
    defaults_seeded: bool,
}

pub fn store_root() -> Option<PathBuf> {
    eframe::storage_dir("Notey").map(|d| d.parent().map(|p| p.join("plugins")).unwrap_or_else(|| d.join("plugins")))
}

impl Store {
    pub fn open() -> Self {
        let root = store_root().unwrap_or_else(|| PathBuf::from("plugins"));
        Self::open_at(root)
    }

    pub fn open_at(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root);
        let state: State = fs::read(root.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut store = Self {
            root,
            installed: Vec::new(),
            disabled: state.disabled.into_iter().collect(),
            defaults_seeded: state.defaults_seeded,
        };
        store.rescan();
        store
    }

    pub fn rescan(&mut self) {
        self.installed.clear();
        let Ok(entries) = fs::read_dir(&self.root) else { return };
        for entry in entries.filter_map(Result::ok) {
            let dir = entry.path();
            if !dir.is_dir() || dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                continue;
            }
            let Some(manifest) = fs::read(dir.join("plugin.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
            else {
                continue;
            };
            self.installed.push(Installed { manifest, dir });
        }
        self.installed.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    }

    fn save_state(&self) {
        let mut disabled: Vec<String> = self.disabled.iter().cloned().collect();
        disabled.sort();
        let state = State {
            disabled,
            defaults_seeded: self.defaults_seeded,
        };
        if let Ok(json) = serde_json::to_vec_pretty(&state) {
            let _ = fs::write(self.root.join("state.json"), json);
        }
    }

    pub fn get(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|p| p.manifest.id == id)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.get(id).is_some() && !self.disabled.contains(id)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled.remove(id);
        } else {
            self.disabled.insert(id.to_string());
        }
        self.save_state();
    }

    /// Enabled plugins of one kind.
    pub fn enabled(&self, kind: Kind) -> impl Iterator<Item = &Installed> {
        self.installed
            .iter()
            .filter(move |p| p.manifest.kind == kind && !self.disabled.contains(&p.manifest.id))
    }

    /// Is a built-in engine switched on by an enabled feature plugin?
    pub fn native_enabled(&self, native: &str) -> bool {
        self.enabled(Kind::Feature)
            .any(|p| p.manifest.native.as_deref() == Some(native))
    }

    pub fn uninstall(&mut self, id: &str) -> Result<(), String> {
        let Some(p) = self.get(id) else { return Ok(()) };
        fs::remove_dir_all(&p.dir).map_err(|e| format!("could not remove {id}: {e}"))?;
        self.disabled.remove(id);
        self.save_state();
        self.rescan();
        Ok(())
    }

    /// Install the installer's bundled default plugins, once. `bundled` holds
    /// an `index.json` plus one folder per plugin; every file is verified
    /// against the index. Skips plugins the user already has, and never
    /// re-adds plugins they later removed.
    pub fn seed_defaults(&mut self, bundled: &Path) -> usize {
        if self.defaults_seeded {
            return 0;
        }
        let index: Option<Index> = fs::read(bundled.join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let Some(index) = index else {
            return 0; // nothing bundled (e.g. a development build)
        };
        let mut added = 0;
        for manifest in &index.plugins {
            let src = bundled.join(&manifest.id);
            if !src.is_dir() || self.get(&manifest.id).is_some() {
                continue;
            }
            let fetch = |f: &FileEntry| fs::read(src.join(&f.path)).map_err(|e| e.to_string());
            if install_files(&self.root, manifest, fetch).is_ok() {
                added += 1;
            }
        }
        self.defaults_seeded = true;
        self.save_state();
        self.rescan();
        added
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Only plain file names are allowed in a package (no folders, no `..`).
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('.')
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Write a package into `root/<id>/`, verifying every file's size and
/// SHA-256 first. Staged in a temp folder and moved into place, so a failed
/// install never leaves a half-written plugin behind.
pub fn install_files(
    root: &Path,
    manifest: &Manifest,
    mut fetch: impl FnMut(&FileEntry) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if !safe_name(&manifest.id) {
        return Err(format!("invalid plugin id {:?}", manifest.id));
    }
    // per-process name: two Notey windows may install the same plugin at once
    let staging = root.join(format!(".staging-{}-{}", manifest.id, std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        for f in &manifest.files {
            if !safe_name(&f.path) {
                return Err(format!("unsafe file name {:?}", f.path));
            }
            let bytes = fetch(f)?;
            if bytes.len() as u64 != f.size || sha256_hex(&bytes) != f.sha256 {
                return Err(format!("{} failed its integrity check", f.path));
            }
            fs::write(staging.join(&f.path), bytes).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(staging.join("plugin.json"), json).map_err(|e| e.to_string())?;
        let dest = root.join(&manifest.id);
        let _ = fs::remove_dir_all(&dest);
        fs::rename(&staging, &dest).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

// ---------- registry ----------

/// Registry base: `NOTEY_PLUGIN_REGISTRY` (a URL or a local `plugins/`
/// folder, for development) or the official repository.
pub fn registry_base() -> String {
    std::env::var("NOTEY_PLUGIN_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_string())
}

fn is_url(base: &str) -> bool {
    base.starts_with("https://") || base.starts_with("http://")
}

fn read_registry(base: &str, rel: &str) -> Result<Vec<u8>, String> {
    if is_url(base) {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("Notey/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(format!("{}/{}", base.trim_end_matches('/'), rel))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("download failed: {e}"))?;
        let mut bytes = Vec::new();
        resp.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("file is too large".into());
        }
        Ok(bytes)
    } else {
        fs::read(Path::new(base).join(rel)).map_err(|e| format!("{rel}: {e}"))
    }
}

/// Fetch the registry's plugin list.
pub fn fetch_index() -> Result<Vec<Manifest>, String> {
    let bytes = read_registry(&registry_base(), "index.json")?;
    let index: Index = serde_json::from_slice(&bytes).map_err(|e| format!("bad plugin index: {e}"))?;
    if index.schema != 1 {
        return Err(format!("unsupported plugin index schema {}", index.schema));
    }
    Ok(index.plugins)
}

/// Download and install one plugin from the registry.
pub fn install_from_registry(root: &Path, manifest: &Manifest) -> Result<(), String> {
    let base = registry_base();
    if manifest.dir.split('/').any(|seg| !safe_name(seg)) {
        return Err(format!("unsafe plugin folder {:?}", manifest.dir));
    }
    install_files(root, manifest, |f| read_registry(&base, &format!("{}/{}", manifest.dir, f.path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("notey-pkg-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(id: &str, files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            id: id.into(),
            kind: Kind::Language,
            name: id.into(),
            version: "1.0.0".into(),
            description: String::new(),
            license: String::new(),
            dir: format!("languages/{id}"),
            files: files
                .iter()
                .map(|(n, b)| FileEntry {
                    path: n.to_string(),
                    sha256: sha256_hex(b),
                    size: b.len() as u64,
                })
                .collect(),
            locale: Some("xx_XX".into()),
            extensions: vec![],
            recommends: vec![],
            native: None,
            script: None,
            capabilities: vec![],
        }
    }

    #[test]
    fn installs_verified_files_and_tracks_state() {
        let root = temp("install");
        let m = manifest("lang-x", &[("x.aff", b"SET UTF-8\n"), ("x.dic", b"1\nword\n")]);
        let files: Vec<(String, Vec<u8>)> =
            vec![("x.aff".into(), b"SET UTF-8\n".to_vec()), ("x.dic".into(), b"1\nword\n".to_vec())];
        install_files(&root, &m, |f| {
            Ok(files.iter().find(|(n, _)| *n == f.path).unwrap().1.clone())
        })
        .unwrap();

        let mut store = Store::open_at(root.clone());
        assert!(store.is_enabled("lang-x"));
        assert_eq!(store.enabled(Kind::Language).count(), 1);
        store.set_enabled("lang-x", false);
        let store2 = Store::open_at(root.clone()); // state persists
        assert!(!store2.is_enabled("lang-x"));
        assert_eq!(store2.enabled(Kind::Language).count(), 0);

        let mut store3 = Store::open_at(root.clone());
        store3.uninstall("lang-x").unwrap();
        assert!(store3.get("lang-x").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_tampered_or_unsafe_files_without_leaving_debris() {
        let root = temp("tamper");
        let m = manifest("lang-y", &[("y.dic", b"good")]);
        let err = install_files(&root, &m, |_| Ok(b"evil".to_vec())).unwrap_err();
        assert!(err.contains("integrity"), "{err}");
        assert!(!root.join("lang-y").exists());
        let leftovers: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
            .collect();
        assert!(leftovers.is_empty(), "staging folder left behind");

        let bad = manifest("lang-z", &[("../escape.txt", b"x")]);
        assert!(install_files(&root, &bad, |_| Ok(b"x".to_vec())).is_err());
        assert!(!root.parent().unwrap().join("escape.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn seeds_defaults_once_and_respects_removals() {
        let bundled = temp("bundled");
        let root = temp("seedroot");
        // the installer ships index.json plus one folder per default plugin
        let m = manifest("lang-d", &[("d.dic", b"1\nd\n")]);
        let other = manifest("lang-not-bundled", &[("x.dic", b"x")]);
        fs::create_dir_all(bundled.join("lang-d")).unwrap();
        fs::write(bundled.join("lang-d/d.dic"), b"1\nd\n").unwrap();
        let index = Index { schema: 1, plugins: vec![m, other] };
        fs::write(bundled.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();

        let mut store = Store::open_at(root.clone());
        assert_eq!(store.seed_defaults(&bundled), 1, "only bundled folders are installed");
        assert!(store.is_enabled("lang-d"));
        store.uninstall("lang-d").unwrap();
        let mut again = Store::open_at(root.clone());
        assert_eq!(again.seed_defaults(&bundled), 0, "removed defaults stay removed");
        assert!(again.get("lang-d").is_none());
        let _ = fs::remove_dir_all(bundled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn official_index_is_consistent() {
        // the repo's generated index must match the files beside it
        let plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
        let index: Index =
            serde_json::from_slice(&fs::read(plugins.join("index.json")).unwrap()).unwrap();
        assert_eq!(index.schema, 1);
        let mut ids = HashSet::new();
        for m in &index.plugins {
            assert!(ids.insert(m.id.clone()), "duplicate id {}", m.id);
            for f in &m.files {
                let bytes = fs::read(plugins.join(&m.dir).join(&f.path)).unwrap();
                assert_eq!(sha256_hex(&bytes), f.sha256, "{}/{}", m.dir, f.path);
            }
        }
    }
}
