//! Download VS Code color themes referenced by vscodethemes.com URLs.
//!
//! vscodethemes.com supplies the useful publisher, extension, and theme slug.
//! The complete color theme lives in the extension's Marketplace VSIX package.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use url::Url;
use zip::ZipArchive;

const MAX_VSIX_BYTES: u64 = 100 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_THEME_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXTRACTED_THEME_BYTES: u64 = 40 * 1024 * 1024;
const MAX_THEME_FILES: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledTheme {
    pub label: String,
    pub package: String,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct ThemeReference {
    publisher: String,
    extension: String,
    theme_slug: Option<String>,
}

#[derive(Deserialize)]
struct ExtensionManifest {
    name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    contributes: Option<Contributions>,
}

#[derive(Deserialize)]
struct Contributions {
    themes: Option<Vec<ThemeContribution>>,
}

#[derive(Deserialize)]
struct ThemeContribution {
    id: Option<String>,
    label: Option<String>,
    path: String,
}

/// Download the Marketplace extension named by a vscodethemes.com URL and
/// extract its selected color theme. Returns a local file suitable for
/// `theme::load_vscode_theme`.
pub fn download_theme(url: &str) -> Result<PathBuf, String> {
    let storage = eframe::storage_dir("Notey")
        .ok_or_else(|| "Notey could not locate its settings folder".to_string())?;
    download_theme_to(url, &storage.join("themes"))
}

/// List every color-theme variant in the Marketplace extensions that Notey
/// has already downloaded. Broken or incomplete packages are skipped so one
/// bad install cannot hide the rest of the menu.
pub fn installed_themes() -> Vec<InstalledTheme> {
    let Some(storage) = eframe::storage_dir("Notey") else {
        return Vec::new();
    };
    installed_themes_in(&storage.join("themes"))
}

fn installed_themes_in(theme_root: &Path) -> Vec<InstalledTheme> {
    let Ok(packages) = fs::read_dir(theme_root) else {
        return Vec::new();
    };
    let mut installed = Vec::new();
    for package_dir in packages.filter_map(Result::ok) {
        let Ok(file_type) = package_dir.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let package_path = package_dir.path();
        let Ok(metadata) = fs::metadata(package_path.join("package.json")) else {
            continue;
        };
        if metadata.len() > MAX_THEME_ENTRY_BYTES {
            continue;
        }
        let Ok(source) = fs::read(package_path.join("package.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<ExtensionManifest>(&source) else {
            continue;
        };
        let package = manifest
            .display_name
            .or(manifest.name)
            .unwrap_or_else(|| package_dir.file_name().to_string_lossy().into_owned());
        let Some(themes) = manifest.contributes.and_then(|value| value.themes) else {
            continue;
        };
        for theme in themes {
            let Ok(archive_path) = extension_archive_path(&theme.path) else {
                continue;
            };
            let Some(relative) = safe_extension_relative_path(&archive_path) else {
                continue;
            };
            let path = package_path.join(relative);
            if !path.is_file() {
                continue;
            }
            let label = theme
                .label
                .or(theme.id)
                .or_else(|| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "Unnamed theme".into());
            installed.push(InstalledTheme {
                label,
                package: package.clone(),
                path,
            });
        }
    }
    installed.sort_by(|left, right| {
        left.package
            .to_ascii_lowercase()
            .cmp(&right.package.to_ascii_lowercase())
            .then_with(|| {
                left.label
                    .to_ascii_lowercase()
                    .cmp(&right.label.to_ascii_lowercase())
            })
    });
    installed
}

fn download_theme_to(url: &str, theme_root: &Path) -> Result<PathBuf, String> {
    let reference = parse_theme_url(url)?;
    let package_url = format!(
        "https://{}.gallery.vsassets.io/_apis/public/gallery/publisher/{}/extension/{}/latest/assetbyname/Microsoft.VisualStudio.Services.VSIXPackage",
        reference.publisher, reference.publisher, reference.extension
    );

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|err| format!("could not create the download client: {err}"))?;
    let response = client
        .get(package_url)
        .send()
        .map_err(|err| format!("could not download the VS Code extension: {err}"))?
        .error_for_status()
        .map_err(|err| format!("the VS Code Marketplace rejected the download: {err}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_VSIX_BYTES)
    {
        return Err("the VS Code extension is larger than 100 MB".into());
    }

    let mut bytes = Vec::new();
    response
        .take(MAX_VSIX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("could not read the VS Code extension: {err}"))?;
    if bytes.len() as u64 > MAX_VSIX_BYTES {
        return Err("the VS Code extension is larger than 100 MB".into());
    }

    extract_theme_package(&bytes, &reference, theme_root)
}

fn parse_theme_url(input: &str) -> Result<ThemeReference, String> {
    let url = Url::parse(input.trim()).map_err(|_| {
        "enter a URL like https://vscodethemes.com/e/publisher.extension/theme".to_string()
    })?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("vscodethemes.com" | "www.vscodethemes.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("only https://vscodethemes.com theme URLs are accepted".into());
    }

    let segments: Vec<_> = url
        .path_segments()
        .ok_or_else(|| "the theme URL has no path".to_string())?
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 || segments.len() > 3 || segments[0] != "e" {
        return Err(
            "enter a theme URL like https://vscodethemes.com/e/publisher.extension/theme"
                .into(),
        );
    }
    let (publisher, extension) = segments[1]
        .split_once('.')
        .ok_or_else(|| "the URL does not contain a publisher and extension".to_string())?;
    if !valid_identifier(publisher) || !valid_identifier(extension) {
        return Err("the URL contains an invalid publisher or extension name".into());
    }
    let theme_slug = segments.get(2).map(|value| value.to_string());
    if theme_slug
        .as_deref()
        .is_some_and(|slug| !valid_identifier(slug))
    {
        return Err("the URL contains an invalid theme name".into());
    }

    Ok(ThemeReference {
        publisher: publisher.to_string(),
        extension: extension.to_string(),
        theme_slug,
    })
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn extract_theme_package(
    bytes: &[u8],
    reference: &ThemeReference,
    theme_root: &Path,
) -> Result<PathBuf, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| format!("the downloaded extension is not a valid VSIX: {err}"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("the VS Code extension contains too many files".into());
    }

    let manifest_source = read_archive_entry(
        &mut archive,
        "extension/package.json",
        MAX_THEME_ENTRY_BYTES,
    )?;
    let manifest: ExtensionManifest = serde_json::from_slice(&manifest_source)
        .map_err(|err| format!("the extension has an invalid package.json: {err}"))?;
    let themes = manifest
        .contributes
        .and_then(|value| value.themes)
        .filter(|themes| !themes.is_empty())
        .ok_or_else(|| "the extension does not contain any color themes".to_string())?;
    let selected = select_theme(&themes, reference.theme_slug.as_deref())?;
    let selected_archive_path = extension_archive_path(&selected.path)?;

    let folder_name = format!("{}.{}", reference.publisher, reference.extension);
    let destination = theme_root.join(folder_name);
    fs::create_dir_all(&destination)
        .map_err(|err| format!("could not create the theme folder: {err}"))?;

    let mut extracted_bytes = 0u64;
    let mut extracted_files = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("could not read the extension archive: {err}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = safe_extension_relative_path(entry.name()) else {
            continue;
        };
        let is_selected = extension_archive_path_from_relative(&relative) == selected_archive_path;
        if !is_selected && !is_theme_resource(&relative) {
            continue;
        }
        if entry.size() > MAX_THEME_ENTRY_BYTES {
            return Err(format!("theme file {} is larger than 8 MB", relative.display()));
        }
        extracted_files += 1;
        extracted_bytes = extracted_bytes.saturating_add(entry.size());
        if extracted_files > MAX_THEME_FILES || extracted_bytes > MAX_EXTRACTED_THEME_BYTES {
            return Err("the extension contains too much theme data".into());
        }

        let output = destination.join(&relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("could not create a theme folder: {err}"))?;
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take(MAX_THEME_ENTRY_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|err| format!("could not extract {}: {err}", relative.display()))?;
        if contents.len() as u64 > MAX_THEME_ENTRY_BYTES {
            return Err(format!("theme file {} is larger than 8 MB", relative.display()));
        }
        fs::write(&output, contents)
            .map_err(|err| format!("could not save {}: {err}", relative.display()))?;
    }

    let selected_relative = safe_extension_relative_path(&selected_archive_path)
        .ok_or_else(|| "the selected theme path is unsafe".to_string())?;
    let selected_file = destination.join(selected_relative);
    if !selected_file.is_file() {
        return Err("the selected theme file was not present in the extension".into());
    }
    Ok(selected_file)
}

fn read_archive_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| format!("the extension does not contain {name}"))?;
    if entry.size() > limit {
        return Err(format!("{name} is too large"));
    }
    let mut contents = Vec::with_capacity(entry.size() as usize);
    entry
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut contents)
        .map_err(|err| format!("could not read {name}: {err}"))?;
    if contents.len() as u64 > limit {
        return Err(format!("{name} is too large"));
    }
    Ok(contents)
}

fn select_theme<'a>(
    themes: &'a [ThemeContribution],
    requested_slug: Option<&str>,
) -> Result<&'a ThemeContribution, String> {
    let Some(requested) = requested_slug else {
        return Ok(&themes[0]);
    };
    let requested = requested.to_ascii_lowercase();
    themes
        .iter()
        .find(|theme| {
            theme
                .label
                .as_deref()
                .into_iter()
                .chain(theme.id.as_deref())
                .any(|name| slugify(name) == requested)
                || theme_path_slug(&theme.path).is_some_and(|slug| slug == requested)
        })
        .ok_or_else(|| format!("theme '{requested}' was not found in the extension"))
}

fn slugify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn theme_path_slug(value: &str) -> Option<String> {
    Path::new(value)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify)
}

fn extension_archive_path(value: &str) -> Result<String, String> {
    let trimmed = value.trim_start_matches("./").replace('\\', "/");
    if trimmed.is_empty() {
        return Err("the selected theme has an empty file path".into());
    }
    Ok(format!("extension/{trimmed}"))
}

fn extension_archive_path_from_relative(value: &Path) -> String {
    format!("extension/{}", value.to_string_lossy().replace('\\', "/"))
}

fn safe_extension_relative_path(value: &str) -> Option<PathBuf> {
    let normalized = value.replace('\\', "/");
    let relative = normalized.strip_prefix("extension/")?;
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn is_theme_resource(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("json")
                || extension.eq_ignore_ascii_case("jsonc")
                || extension.eq_ignore_ascii_case("tmTheme")
                || extension.eq_ignore_ascii_case("plist")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vscodethemes_theme_url() {
        assert_eq!(
            parse_theme_url(
                "https://vscodethemes.com/e/GitHub.github-vscode-theme/github-dark?language=rust"
            )
            .unwrap(),
            ThemeReference {
                publisher: "GitHub".into(),
                extension: "github-vscode-theme".into(),
                theme_slug: Some("github-dark".into()),
            }
        );
    }

    #[test]
    fn rejects_non_vscodethemes_hosts_and_unsafe_names() {
        assert!(parse_theme_url(
            "https://vscodethemes.com.evil.example/e/GitHub.github-vscode-theme/github-dark"
        )
        .is_err());
        assert!(parse_theme_url("https://vscodethemes.com/e/GitHub.github-vscode-theme/..")
            .is_err());
    }

    #[test]
    fn selects_theme_by_label_id_or_file_name() {
        let themes = vec![
            ThemeContribution {
                id: Some("one".into()),
                label: Some("First Theme".into()),
                path: "./themes/first.json".into(),
            },
            ThemeContribution {
                id: Some("github-dark".into()),
                label: Some("GitHub Dark".into()),
                path: "./themes/dark.json".into(),
            },
        ];
        assert_eq!(
            select_theme(&themes, Some("github-dark")).unwrap().path,
            "./themes/dark.json"
        );
        assert_eq!(select_theme(&themes, None).unwrap().path, "./themes/first.json");
    }

    #[test]
    fn archive_paths_cannot_escape_the_extension_folder() {
        assert_eq!(
            safe_extension_relative_path("extension/themes/dark.json"),
            Some(PathBuf::from("themes/dark.json"))
        );
        assert_eq!(
            safe_extension_relative_path("extension/themes/../../evil.json"),
            None
        );
        assert_eq!(safe_extension_relative_path("outside/theme.json"), None);
    }

    #[test]
    fn scans_installed_packages_and_ignores_unsafe_or_missing_themes() {
        let root = std::env::temp_dir().join(format!(
            "notey-installed-theme-test-{}",
            std::process::id()
        ));
        let package = root.join("publisher.extension");
        fs::create_dir_all(package.join("themes")).unwrap();
        fs::write(package.join("themes/dark.json"), "{}").unwrap();
        fs::write(
            package.join("package.json"),
            r#"{
                "name": "extension",
                "displayName": "Example Themes",
                "contributes": { "themes": [
                    { "label": "Example Dark", "path": "./themes/dark.json" },
                    { "label": "Missing", "path": "./themes/missing.json" },
                    { "label": "Unsafe", "path": "../outside.json" }
                ] }
            }"#,
        )
        .unwrap();

        let installed = installed_themes_in(&root);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].label, "Example Dark");
        assert_eq!(installed[0].package, "Example Themes");
        assert_eq!(installed[0].path, package.join("themes/dark.json"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "downloads a live Visual Studio Marketplace extension"]
    fn downloads_and_loads_a_live_theme() {
        let root = std::env::temp_dir().join(format!(
            "notey-live-theme-test-{}",
            std::process::id()
        ));
        let path = download_theme_to(
            "https://vscodethemes.com/e/GitHub.github-vscode-theme/github-dark",
            &root,
        )
        .unwrap();
        let imported = crate::theme::load_vscode_theme(&path).unwrap();
        assert!(imported.dark);
        assert!(path.is_file());
        let installed = installed_themes_in(&root);
        assert!(installed.iter().any(|theme| theme.label == "GitHub Dark"));
        let _ = fs::remove_dir_all(root);
    }
}
