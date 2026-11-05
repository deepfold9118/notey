//! Updates: find the latest release of Notey on GitHub, download its
//! installer, check it against the release's SHA-256 file, and run it.
//!
//! `NOTEY_UPDATE_FEED` replaces the GitHub address, for testing: a URL, or
//! a local JSON file in the shape of GitHub's release API whose asset URLs
//! may be local paths.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

const LATEST_RELEASE: &str = "https://api.github.com/repos/deepfold9118/notey/releases/latest";
const INSTALLER: &str = "NoteySetup.exe";
const CHECKSUM: &str = "NoteySetup.exe.sha256";
/// The installer is a few megabytes; refuse anything absurd.
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;

/// A release that can be installed.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.8.8", without the tag's "v".
    pub version: String,
    /// The release notes (Markdown).
    pub notes: String,
    pub installer_url: String,
    pub checksum_url: String,
}

fn feed() -> String {
    std::env::var("NOTEY_UPDATE_FEED").unwrap_or_else(|_| LATEST_RELEASE.to_string())
}

fn is_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

/// Read a URL or a local file, up to `limit` bytes.
fn fetch(source: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    if is_url(source) {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .user_agent(concat!("Notey/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(source)
            .header("Accept", "application/vnd.github+json")
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        resp.take(limit + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    } else {
        std::fs::File::open(source)
            .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
            .map_err(|e| format!("{source}: {e}"))?;
    }
    if bytes.len() as u64 > limit {
        return Err("the download is too large".into());
    }
    Ok(bytes)
}

/// The release described by a GitHub release API response.
pub fn parse_release(json: &[u8]) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("bad release data: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("the release has no version")?;
    let asset = |name: &str| {
        v["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == name))
            .and_then(|x| x["browser_download_url"].as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("the release has no {name}"))
    };
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        installer_url: asset(INSTALLER)?,
        checksum_url: asset(CHECKSUM)?,
    })
}

/// Look up the latest release.
pub fn latest() -> Result<Release, String> {
    parse_release(&fetch(&feed(), 1024 * 1024)?)
}

/// "0.8.10" as numbers; anything after a "-" (a pre-release) is ignored.
fn version_parts(v: &str) -> Option<Vec<u64>> {
    let core = v.trim_start_matches('v').split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

/// Whether `latest` is a later version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (version_parts(latest), version_parts(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The current version of Notey.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Download the installer of `release` into the temporary folder and check
/// it against the release's SHA-256. Returns the installer's path.
pub fn download(release: &Release) -> Result<PathBuf, String> {
    let sums = String::from_utf8(fetch(&release.checksum_url, 4096)?).map_err(|_| "bad checksum file")?;
    let expected = sums.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bad checksum file".into());
    }
    let bytes = fetch(&release.installer_url, MAX_INSTALLER_BYTES)?;
    let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err("the download does not match the release's checksum".into());
    }
    let dir = std::env::temp_dir().join("Notey");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("NoteySetup-{}.exe", release.version));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Start the installer: it closes Notey, upgrades it in place and starts
/// it again (`/RELAUNCH=1`, see installer/notey.iss).
pub fn run_installer(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/RELAUNCH=1"])
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not start the installer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.8.8", "0.8.7"));
        assert!(is_newer("0.9.0", "0.8.10"));
        assert!(is_newer("0.8.10", "0.8.9"));
        assert!(!is_newer("0.8.7", "0.8.7"));
        assert!(!is_newer("0.8.6", "0.8.7"));
        assert!(!is_newer("nonsense", "0.8.7"));
        assert!(!is_newer("0.8.8-beta", "0.8.8"));
    }

    #[test]
    fn reads_a_github_release() {
        let json = br#"{"tag_name": "v0.8.8", "body": "Fixes", "assets": [
            {"name": "NoteySetup.exe", "browser_download_url": "https://x/NoteySetup.exe"},
            {"name": "NoteySetup.exe.sha256", "browser_download_url": "https://x/NoteySetup.exe.sha256"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.8.8");
        assert_eq!(r.notes, "Fixes");
        assert_eq!(r.installer_url, "https://x/NoteySetup.exe");
        assert!(parse_release(br#"{"tag_name": "v1", "assets": []}"#).is_err());
    }

    #[test]
    fn downloads_and_checks_the_installer() {
        let dir = std::env::temp_dir().join(format!("notey-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("NoteySetup.exe");
        std::fs::write(&exe, b"installer").unwrap();
        let sum: String = Sha256::digest(b"installer").iter().map(|b| format!("{b:02x}")).collect();
        let sums = dir.join("NoteySetup.exe.sha256");
        std::fs::write(&sums, format!("{sum}  NoteySetup.exe\n")).unwrap();
        let release = Release {
            version: "0.0.0-test".into(),
            notes: String::new(),
            installer_url: exe.to_string_lossy().into(),
            checksum_url: sums.to_string_lossy().into(),
        };
        let path = download(&release).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"installer");
        std::fs::remove_file(path).unwrap();
        std::fs::write(&exe, b"tampered").unwrap();
        assert!(download(&release).unwrap_err().contains("checksum"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
