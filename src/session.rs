//! Session persistence: unsaved tabs survive closing the app (like Windows 11
//! Notepad). Only the primary window reads and writes the session.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SessionTab {
    /// Backing file, if any.
    pub path: Option<PathBuf>,
    /// Untitled-tab number ("Untitled 3"); 0 for file-backed tabs.
    #[serde(default)]
    pub untitled_n: usize,
    /// Unsaved buffer contents. `None` means the tab was saved (reopen from
    /// `path`).
    pub dirty_text: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    pub active: usize,
    pub tabs: Vec<SessionTab>,
}

fn session_file() -> Option<PathBuf> {
    eframe::storage_dir("Notey").map(|d| d.join("session.json"))
}

pub fn load() -> Option<Session> {
    let path = session_file()?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save(session: &Session) {
    let Some(path) = session_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec(session) {
        let _ = std::fs::write(path, json);
    }
}
