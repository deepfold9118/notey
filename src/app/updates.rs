//! Looking for a newer release of Notey, offering it, and installing it.

use super::*;
use crate::update::{self, Release};

/// How often the main window looks for a release.
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;
/// After a failed check, try again this much sooner.
const RETRY_SECS: i64 = 60 * 60;
/// No check right after startup, while Notey is busy loading.
const STARTUP_DELAY: Duration = Duration::from_secs(10);

/// The release notes in a few plain lines, for the prompt.
fn summary(notes: &str) -> String {
    let lines: Vec<&str> = notes
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.contains("NoteySetup.exe") && !l.contains("notey_setup_"))
        .map(|l| l.trim_start_matches("- "))
        .collect();
    let mut text = lines.join("\n");
    if text.chars().count() > 400 {
        text = text.chars().take(400).collect::<String>().trim_end().to_string() + "\u{2026}";
    }
    text
}

/// An error message as a sentence: "Could not ..." for "could not ...".
fn sentence(e: &str) -> String {
    let mut chars = e.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The number of other running Notey processes (other windows).
fn other_notey_windows() -> usize {
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq notey.exe", "/FO", "CSV", "/NH"])
        .output()
    else {
        return 0;
    };
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.to_ascii_lowercase().starts_with("\"notey.exe\""))
        .filter(|l| l.split(',').nth(1).map(|p| p.trim_matches('"')) != Some(me.as_str()))
        .count()
}

impl NoteyApp {
    /// From the timer: look for a release when automatic checks are on
    /// and the last check is a day old. Only the main window looks.
    pub(super) fn maybe_check_for_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_primary || !self.settings.check_updates || self.update_busy || self.started.elapsed() < STARTUP_DELAY {
            return;
        }
        if chrono::Utc::now().timestamp() - self.settings.last_update_check < CHECK_INTERVAL_SECS {
            return;
        }
        self.check_for_update(false, window, cx);
    }

    /// Look for a release. A `manual` check (Help > Check for Updates)
    /// also says when Notey is up to date or the check failed, and offers
    /// a release that was skipped.
    pub(super) fn check_for_update(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        self.update_busy = true;
        if manual {
            self.flash("Checking for updates\u{2026}".into(), cx);
        }
        let job = cx.background_executor().spawn(async { update::latest() });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.update_busy = false;
                let now = chrono::Utc::now().timestamp();
                this.settings.last_update_check =
                    if result.is_ok() { now } else { now - CHECK_INTERVAL_SECS + RETRY_SECS };
                this.settings.save();
                let current = update::current_version();
                match result {
                    Ok(release) if update::is_newer(&release.version, current) => {
                        if manual || this.settings.skipped_version.as_deref() != Some(release.version.as_str()) {
                            this.offer_update(release, window, cx);
                        }
                    }
                    Ok(_) if manual => {
                        let detail = format!("Version {current} is the latest version.");
                        drop(window.prompt(PromptLevel::Info, "Notey is up to date", Some(&detail), &["OK"], cx));
                    }
                    Err(e) if manual => {
                        drop(window.prompt(PromptLevel::Warning, "Notey could not check for updates", Some(&sentence(&e)), &["OK"], cx));
                    }
                    _ => {}
                }
            });
        })
        .detach();
    }

    fn offer_update(&mut self, release: Release, window: &mut Window, cx: &mut Context<Self>) {
        let mut detail = format!("You have version {}.", update::current_version());
        let notes = summary(&release.notes);
        if !notes.is_empty() {
            detail = format!("{detail}\n\n{notes}");
        }
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Notey {} is available", release.version),
            Some(&detail),
            &["Update Now", "Later", "Skip This Version"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, window, cx| match answer {
                Ok(0) => this.confirm_update(release, window, cx),
                Ok(2) => this.update_settings(|s| s.skipped_version = Some(release.version.clone()), cx),
                _ => {}
            });
        })
        .detach();
    }

    /// The installer closes every Notey window. Other windows keep no
    /// session, so ask before their unsaved changes could be lost.
    fn confirm_update(&mut self, release: Release, window: &mut Window, cx: &mut Context<Self>) {
        let others = other_notey_windows();
        if others == 0 {
            self.install_update(release, window, cx);
            return;
        }
        let detail = format!(
            "The update closes {}. Save your changes there first. This window keeps its tabs.",
            if others == 1 { "the other Notey window".to_string() } else { format!("the other {others} Notey windows") }
        );
        let answer = window.prompt(PromptLevel::Warning, "Other Notey windows are open", Some(&detail), &["Update Now", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update_in(cx, |this, window, cx| this.install_update(release, window, cx));
            }
        })
        .detach();
    }

    /// Download and check the installer, then run it and quit. The
    /// installer upgrades Notey and starts it again, with this window's
    /// tabs from the session.
    fn install_update(&mut self, release: Release, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_busy {
            return;
        }
        self.update_busy = true;
        self.flash(format!("Downloading Notey {}\u{2026}", release.version), cx);
        let job = cx.background_executor().spawn(async move { update::download(&release) });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await.and_then(|path| update::run_installer(&path));
            let _ = this.update_in(cx, |this, window, cx| {
                this.update_busy = false;
                match result {
                    Ok(()) => {
                        this.remember_window(window);
                        this.settings.save();
                        this.save_session(cx);
                        cx.quit();
                    }
                    Err(e) => {
                        drop(window.prompt(PromptLevel::Warning, "Notey could not update", Some(&sentence(&e)), &["OK"], cx));
                    }
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn summarizes_release_notes() {
        let notes = "### Fixed\n- One thing.\n- Another.\n\nDownload `NoteySetup.exe` below.";
        assert_eq!(summary(notes), "One thing.\nAnother.");
        assert!(summary(&"word ".repeat(200)).ends_with('\u{2026}'));
    }
}
