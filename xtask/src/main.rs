//! Build tooling for Notey's official plugins.
//!
//! `cargo xtask plugins` regenerates the generated plugin families
//! (spellcheck languages and file types) from pinned upstream sources,
//! validates every package with the same engines the app uses, and rewrites
//! `plugins/index.json`.
//!
//! `cargo xtask index` only rewrites the index, for edits to hand-written
//! plugins under `plugins/features/`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Sublime Text's grammar repository, at the commit bat pins as syntect-
/// compatible.
const SUBLIME_REPO: &str = "sublimehq/Packages";
const SUBLIME_COMMIT: &str = "759d6eed9b4beed87e602a23303a121c3a6c2fb3";
/// bat, whose patches make Sublime's grammars parse correctly in syntect.
const BAT_REPO: &str = "sharkdp/bat";
const BAT_COMMIT: &str = "4987f76709aae3a1c4db723c53874c9ddcb0c4fd";
const DICT_REPO: &str = "LibreOffice/dictionaries";
const DICT_COMMIT: &str = "32b006a2c22a4ac7e8ed3f03346f7b3d85a970a4";

/// (locale, display name, path of .aff/.dic without extension)
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en_US", "English (US)", "en/en_US"),
    ("en_GB", "English (UK)", "en/en_GB"),
    ("en_CA", "English (Canada)", "en/en_CA"),
    ("en_AU", "English (Australia)", "en/en_AU"),
    ("es_ES", "Spanish", "es/es_ES"),
    ("fr_FR", "French", "fr_FR/dictionaries/fr"),
    ("de_DE", "German", "de/de_DE_frami"),
    ("it_IT", "Italian", "it_IT/it_IT"),
    ("pt_BR", "Portuguese (Brazil)", "pt_BR/pt_BR"),
    ("pt_PT", "Portuguese (Portugal)", "pt_PT/pt_PT"),
    ("nl_NL", "Dutch", "nl_NL/nl_NL"),
    ("pl_PL", "Polish", "pl_PL/pl_PL"),
    ("sv_SE", "Swedish", "sv_SE/dictionaries/sv_SE"),
    ("da_DK", "Danish", "da_DK/da_DK"),
    ("ru_RU", "Russian", "ru_RU/ru_RU"),
    ("uk_UA", "Ukrainian", "uk_UA/uk_UA"),
];

type Res<T> = Result<T, String>;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let result = match cmd.as_str() {
        "plugins" => build_plugins(),
        "index" => write_index(&repo_root()),
        _ => Err("usage: cargo xtask <plugins|index>".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

// ---------- networking ----------

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent("notey-xtask")
        .build()
        .expect("http client")
}

fn get_bytes(url: &str) -> Res<Vec<u8>> {
    let resp = client()
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("GET {url}: {e}"))?;
    Ok(resp.bytes().map_err(|e| e.to_string())?.to_vec())
}

fn raw_url(repo: &str, commit: &str, path: &str) -> String {
    let encoded: String = path
        .split('/')
        .map(|seg| {
            seg.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("https://raw.githubusercontent.com/{repo}/{commit}/{encoded}")
}

/// All blob paths in a repo tree at a commit (one GitHub API call).
fn tree_paths(repo: &str, commit: &str) -> Res<Vec<String>> {
    let url = format!("https://api.github.com/repos/{repo}/git/trees/{commit}?recursive=1");
    let mut req = client().get(&url);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let v: Value = req
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("tree {repo}: {e}"))?;
    if v["truncated"].as_bool() == Some(true) {
        return Err(format!("tree listing for {repo} was truncated"));
    }
    Ok(v["tree"]
        .as_array()
        .ok_or("tree missing")?
        .iter()
        .filter(|e| e["type"] == "blob")
        .filter_map(|e| e["path"].as_str().map(String::from))
        .collect())
}

// ---------- shared helpers ----------

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        match c {
            'a'..='z' | '0'..='9' => out.push(c),
            '+' => out.push_str("plus"),
            '#' => out.push_str("sharp"),
            _ => {
                if !out.ends_with('-') && !out.is_empty() {
                    out.push('-');
                }
            }
        }
    }
    out.trim_end_matches('-').to_string()
}

fn reset_dir(dir: &Path) -> Res<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("clear {}: {e}", dir.display()))?;
    }
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))
}

fn write_manifest(dir: &Path, manifest: &Value) -> Res<()> {
    let text = serde_json::to_string_pretty(manifest).unwrap() + "\n";
    fs::write(dir.join("plugin.json"), text).map_err(|e| e.to_string())
}

// ---------- spellcheck languages ----------

/// Decode a Hunspell file to UTF-8 using the .aff's SET declaration, and
/// rewrite that declaration to UTF-8.
fn to_utf8(bytes: &[u8], set: &str) -> Res<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let label = set.replace("ISO8859-", "ISO-8859-");
    let enc = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| format!("unknown dictionary encoding {set}"))?;
    let (text, _, bad) = enc.decode(bytes);
    if bad {
        return Err(format!("invalid {set} data"));
    }
    Ok(text.into_owned())
}

/// In `FLAG num` dictionaries, a word such as `A/S` would have `S` read as
/// its (invalid) flags. Escape the slash (`A\/S`) when the text after it
/// cannot be numeric flags. Returns the fixed text and the number of words
/// changed.
fn escape_word_slashes(dic: &str) -> (String, usize) {
    let mut changed = 0;
    let lines: Vec<String> = dic
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                return line.to_string(); // word count header
            }
            let split = line.find(|c: char| c == ' ' || c == '\t').unwrap_or(line.len());
            let (word, rest) = line.split_at(split);
            let bad = word
                .split_once('/')
                .is_some_and(|(_, flags)| !flags.chars().all(|c| c.is_ascii_digit() || c == ','));
            if bad {
                changed += 1;
                format!("{}{rest}", word.replace('/', "\\/"))
            } else {
                line.to_string()
            }
        })
        .collect();
    (lines.join("\n") + "\n", changed)
}

fn build_languages(root: &Path) -> Res<()> {
    let out_root = root.join("plugins/languages");
    reset_dir(&out_root)?;
    let tree = tree_paths(DICT_REPO, DICT_COMMIT)?;
    for (locale, name, path) in LANGUAGES {
        let id = format!("lang-{}", slug(locale));
        let dir = out_root.join(&id);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let aff_raw = get_bytes(&raw_url(DICT_REPO, DICT_COMMIT, &format!("{path}.aff")))?;
        let dic_raw = get_bytes(&raw_url(DICT_REPO, DICT_COMMIT, &format!("{path}.dic")))?;
        let aff_head = String::from_utf8_lossy(&aff_raw[..aff_raw.len().min(4096)]).into_owned();
        let set = aff_head
            .lines()
            .map(|l| l.trim_start_matches('\u{feff}').trim())
            .find_map(|l| l.strip_prefix("SET "))
            .unwrap_or("UTF-8")
            .trim()
            .to_string();
        let mut aff = to_utf8(&aff_raw, &set)?;
        let mut dic = to_utf8(&dic_raw, &set)?;
        let numeric_flags = aff.lines().any(|l| l.trim() == "FLAG num");
        if numeric_flags {
            let (fixed, n) = escape_word_slashes(&dic);
            if n > 0 {
                println!("  {locale}: escaped '/' inside {n} word(s)");
                dic = fixed;
            }
        }
        if set != "UTF-8" {
            aff = aff.replacen(&format!("SET {set}"), "SET UTF-8", 1);
        }

        // validate with the same engine the app uses
        let dict = spellbook::Dictionary::new(&aff, &dic)
            .map_err(|e| format!("{locale}: dictionary does not load: {e:?}"))?;
        let _ = dict.check("test");

        let aff_name = format!("{locale}.aff");
        let dic_name = format!("{locale}.dic");
        fs::write(dir.join(&aff_name), &aff).map_err(|e| e.to_string())?;
        fs::write(dir.join(&dic_name), &dic).map_err(|e| e.to_string())?;
        let mut files = vec![aff_name, dic_name];

        // licenses and readmes travel with the dictionary; some packages keep
        // them one level up from the .aff/.dic (e.g. sv_SE/dictionaries/)
        let src_dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let parent_dir = src_dir.rsplit_once('/').map(|(d, _)| d);
        for search in std::iter::once(src_dir).chain(parent_dir) {
            for p in &tree {
                let Some(file) = p.strip_prefix(&format!("{search}/")) else { continue };
                if file.contains('/') {
                    continue;
                }
                let lower = file.to_lowercase();
                let is_doc = lower.contains("licen") || lower.contains("copying") || lower.starts_with("readme");
                // skip docs for hyphenation, thesaurus, grammar checking, and
                // for other locales that share this upstream folder
                let other_locale = LANGUAGES
                    .iter()
                    .any(|(l, _, _)| *l != *locale && lower.contains(&l.to_lowercase()))
                    || lower.contains("en_za");
                let is_other = lower.contains("hyph")
                    || lower.contains("thes")
                    || lower.contains("th_")
                    || lower.contains("lightproof")
                    || lower.contains("wordnet")
                    || other_locale;
                if is_doc && !is_other {
                    let bytes = get_bytes(&raw_url(DICT_REPO, DICT_COMMIT, p))?;
                    fs::write(dir.join(file), bytes).map_err(|e| e.to_string())?;
                    files.push(file.to_string());
                }
            }
            if files.len() > 2 {
                break;
            }
        }
        if files.len() == 2 {
            return Err(format!("{locale}: no license file found beside the dictionary"));
        }

        write_manifest(
            &dir,
            &json!({
                "id": id,
                "kind": "language",
                "name": name,
                "version": "1.0.0",
                "description": format!("{name} spellcheck dictionary (Hunspell, via LibreOffice). Needs the Spellcheck plugin."),
                "license": "See the bundled README/license files",
                "locale": locale,
                "files": files,
                "source": format!("https://github.com/{DICT_REPO}/tree/{DICT_COMMIT}/{src_dir}"),
            }),
        )?;
        println!("  language {locale:<6} ({} set) ok", set);
    }
    Ok(())
}

// ---------- file types ----------

/// Fetch bat's syntect-compatibility patches for Sublime's packages:
/// package-relative path -> patch text.
fn bat_patches() -> Res<BTreeMap<String, String>> {
    let tree = tree_paths(BAT_REPO, BAT_COMMIT)?;
    let mut out = BTreeMap::new();
    for p in tree.iter().filter(|p| p.starts_with("assets/patches/") && p.ends_with(".sublime-syntax.patch")) {
        let text = String::from_utf8(get_bytes(&raw_url(BAT_REPO, BAT_COMMIT, p))?).map_err(|e| e.to_string())?;
        // only patches that target Sublime's packages (01_Packages)
        for line in text.lines() {
            if let Some(target) = line.strip_prefix("+++ syntaxes/01_Packages/") {
                out.insert(target.trim().to_string(), text.clone());
                break;
            }
        }
    }
    Ok(out)
}

/// Apply a bat patch (paths relative to bat's `assets/`) to one file.
fn apply_patch(original: &str, patch: &str, rel: &str) -> Res<String> {
    let work = std::env::temp_dir().join(format!("notey-xtask-patch-{}", std::process::id()));
    reset_dir(&work)?;
    let target = work.join("syntaxes/01_Packages").join(rel);
    fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&target, original).map_err(|e| e.to_string())?;
    let patch_file = work.join("change.patch");
    fs::write(&patch_file, patch).map_err(|e| e.to_string())?;
    let status = Command::new("git")
        .args(["apply", "-p0", "--whitespace=nowarn", "change.patch"])
        .current_dir(&work)
        .status()
        .map_err(|e| format!("git apply: {e}"))?;
    if !status.success() {
        return Err(format!("patch for {rel} did not apply"));
    }
    let out = fs::read_to_string(&target).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&work);
    Ok(out)
}

#[derive(Default)]
struct FileTypeInfo {
    id: String,
    name: String,
    files: Vec<String>,
    extensions: BTreeSet<String>,
    provides: BTreeSet<String>,
    references: BTreeSet<String>,
}

fn build_filetypes(root: &Path) -> Res<()> {
    let out_root = root.join("plugins/filetypes");
    reset_dir(&out_root)?;
    let tree = tree_paths(SUBLIME_REPO, SUBLIME_COMMIT)?;
    let patches = bat_patches()?;
    let general_license = get_bytes(&raw_url(SUBLIME_REPO, SUBLIME_COMMIT, "LICENSE"))?;

    // group grammars by package directory
    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in tree.iter().filter(|p| p.ends_with(".sublime-syntax")) {
        if let Some((pkg, _)) = p.split_once('/') {
            packages.entry(pkg.to_string()).or_default().push(p.clone());
        }
    }

    let scope_re = regex_lite_scopes();
    let mut infos: Vec<FileTypeInfo> = Vec::new();
    let mut all_defs = Vec::new();
    for (pkg, grammars) in &packages {
        let id = format!("filetype-{}", slug(pkg));
        let dir = out_root.join(&id);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut info = FileTypeInfo {
            id: id.clone(),
            name: pkg.clone(),
            ..Default::default()
        };
        for g in grammars {
            let rel = g.as_str();
            let mut text = String::from_utf8(get_bytes(&raw_url(SUBLIME_REPO, SUBLIME_COMMIT, rel))?)
                .map_err(|e| format!("{rel}: {e}"))?;
            if let Some(patch) = patches.get(rel) {
                text = apply_patch(&text, patch, rel)?;
                println!("  patched {rel}");
            }
            let def = syntect::parsing::SyntaxDefinition::load_from_str(&text, true, None)
                .map_err(|e| format!("{rel}: grammar does not parse: {e}"))?;
            if !def.hidden {
                info.extensions.extend(def.file_extensions.iter().cloned());
            }
            info.provides.insert(def.scope.build_string());
            for cap in scope_re(&text) {
                info.references.insert(cap);
            }
            all_defs.push(def);
            let file = rel.rsplit('/').next().unwrap().to_string();
            fs::write(dir.join(&file), &text).map_err(|e| e.to_string())?;
            info.files.push(file);
        }
        // license: the repository-wide grant, plus any package-specific one
        fs::write(dir.join("LICENSE-sublime-packages.txt"), &general_license).map_err(|e| e.to_string())?;
        info.files.push("LICENSE-sublime-packages.txt".into());
        for p in tree.iter().filter(|p| p.starts_with(&format!("{pkg}/"))) {
            let file = &p[pkg.len() + 1..];
            if !file.contains('/') && file.to_lowercase().starts_with("license") {
                let bytes = get_bytes(&raw_url(SUBLIME_REPO, SUBLIME_COMMIT, p))?;
                fs::write(dir.join(file), bytes).map_err(|e| e.to_string())?;
                info.files.push(file.to_string());
            }
        }
        infos.push(info);
    }

    // the whole family must also build together
    let mut builder = syntect::parsing::SyntaxSetBuilder::new();
    builder.add_plain_text_syntax();
    for def in all_defs {
        builder.add(def);
    }
    let set = builder.build();
    println!("  full file-type set builds: {} syntaxes", set.syntaxes().len());

    // recommendations: other packages providing scopes this one embeds
    let provider: BTreeMap<String, String> = infos
        .iter()
        .flat_map(|i| i.provides.iter().map(move |s| (s.clone(), i.id.clone())))
        .collect();
    for info in &infos {
        let recommends: BTreeSet<String> = info
            .references
            .iter()
            .filter_map(|s| provider.get(s))
            .filter(|id| **id != info.id)
            .cloned()
            .collect();
        write_manifest(
            &out_root.join(&info.id),
            &json!({
                "id": info.id,
                "kind": "filetype",
                "name": info.name,
                "version": "1.0.0",
                "description": format!("Syntax highlighting for {}.", info.name),
                "license": "Sublime Text Packages license (see bundled license files)",
                "extensions": info.extensions,
                "recommends": recommends,
                "files": info.files,
                "source": format!("https://github.com/{SUBLIME_REPO}/tree/{SUBLIME_COMMIT}/{}", info.name),
            }),
        )?;
        println!("  filetype {:<22} {} ext, {} recommended", info.name, info.extensions.len(), recommends.len());
    }
    Ok(())
}

/// Extract `scope:foo.bar` references from grammar text.
fn regex_lite_scopes() -> impl Fn(&str) -> Vec<String> {
    |text: &str| {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find("scope:") {
            rest = &rest[i + 6..];
            let scope: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
                .collect();
            if !scope.is_empty() {
                out.push(scope.trim_end_matches('.').to_string());
            }
        }
        out
    }
}

// ---------- index ----------

#[derive(Serialize, Deserialize)]
struct IndexFile {
    path: String,
    sha256: String,
    size: u64,
}

/// Rewrite plugins/index.json from every plugin.json under plugins/.
fn write_index(root: &Path) -> Res<()> {
    let plugins_dir = root.join("plugins");
    let mut entries = Vec::new();
    for family in ["features", "languages", "filetypes"] {
        let fam = plugins_dir.join(family);
        let Ok(read) = fs::read_dir(&fam) else { continue };
        let mut dirs: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for dir in dirs {
            let manifest_path = dir.join("plugin.json");
            let text = fs::read_to_string(&manifest_path)
                .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
            let mut manifest: Value = serde_json::from_str(&text)
                .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
            let names: Vec<String> = manifest["files"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let mut files = Vec::new();
            for name in &names {
                let bytes = fs::read(dir.join(name)).map_err(|e| format!("{}/{name}: {e}", dir.display()))?;
                files.push(IndexFile {
                    path: name.clone(),
                    sha256: hex(&Sha256::digest(&bytes)),
                    size: bytes.len() as u64,
                });
            }
            let rel = dir.strip_prefix(&plugins_dir).unwrap().to_string_lossy().replace('\\', "/");
            manifest["dir"] = json!(rel);
            manifest["files"] = serde_json::to_value(files).unwrap();
            entries.push(manifest);
        }
    }
    let index = json!({ "schema": 1, "plugins": entries });
    let out = serde_json::to_string_pretty(&index).unwrap() + "\n";
    fs::write(plugins_dir.join("index.json"), out).map_err(|e| e.to_string())?;
    println!("wrote plugins/index.json ({} plugins)", index["plugins"].as_array().unwrap().len());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_plugins() -> Res<()> {
    let root = repo_root();
    println!("languages (LibreOffice {DICT_COMMIT:.7}):");
    build_languages(&root)?;
    println!("file types (Sublime Packages {SUBLIME_COMMIT:.7} + bat {BAT_COMMIT:.7} patches):");
    build_filetypes(&root)?;
    write_index(&root)
}
