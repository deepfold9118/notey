//! Work on a timer: noticing files that another program changed or
//! deleted (with a prompt to reload), autosave, and the window geometry.

use std::time::{Duration, Instant};

use codemirror_state::ChangeSpec;
use gpui::{Bounds, WindowBounds};

use super::*;
use crate::settings::WindowGeometry;

/// How often open files are checked on disk.
pub(super) const WATCH_INTERVAL: Duration = Duration::from_secs(3);

impl NoteyApp {
    /// Runs every [`WATCH_INTERVAL`].
    pub(super) fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.check_disk(window, cx);
        if self.settings.autosave && self.last_autosave.elapsed().as_secs_f32() >= self.settings.autosave_secs.max(5.0) {
            self.last_autosave = Instant::now();
            let mut saved = 0;
            for i in 0..self.tabs.len() {
                if self.tabs[i].doc.path.is_some() && self.tabs[i].modified && self.write_tab_quietly(i, cx) {
                    saved += 1;
                }
            }
            if saved > 0 {
                self.flash(format!("Autosaved {saved} file{}", if saved == 1 { "" } else { "s" }), cx);
            }
        }
    }

    /// Look for files changed or deleted by another program. A changed file
    /// asks whether to reload it (one prompt at a time).
    fn check_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reload_prompt {
            return;
        }
        let mut deleted = None;
        let mut changed = None;
        for tab in &mut self.tabs {
            let (Some(path), Some(known)) = (&tab.doc.path, tab.doc.disk_mtime) else { continue };
            match std::fs::metadata(path).and_then(|m| m.modified()) {
                Ok(m) if m > known => {
                    changed = Some(tab.doc.id);
                    break;
                }
                Ok(_) => {}
                Err(_) => {
                    tab.doc.disk_mtime = None;
                    deleted = Some(tab.doc.title());
                }
            }
        }
        if let Some(title) = deleted {
            self.flash(format!("The file on disk was deleted: {title}"), cx);
        }
        let Some(id) = changed else { return };
        let Some(idx) = self.tab_index(id) else { return };
        let tab = &self.tabs[idx];
        let title = tab.doc.title();
        let detail = if tab.modified {
            "Reloading discards your unsaved changes in this tab."
        } else {
            "Reload it to see the other program's changes."
        };
        self.reload_prompt = true;
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("\u{201c}{title}\u{201d} was changed by another program."),
            Some(detail),
            &["Reload", "Keep This Version"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let reload = answer.await == Ok(0);
            let _ = this.update(cx, |this, cx| {
                this.reload_prompt = false;
                if let Some(idx) = this.tab_index(id) {
                    if reload {
                        this.reload_tab(idx, cx);
                    } else if let Some(path) = this.tabs[idx].doc.path.clone() {
                        // Don't ask again about this change.
                        this.tabs[idx].doc.disk_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                    }
                }
            });
        })
        .detach();
    }

    /// Replace the tab's text with the file on disk. The reload can be
    /// undone.
    fn reload_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        let title = self.tabs[idx].doc.title();
        match self.tabs[idx].doc.reload() {
            Ok(text) => {
                self.tabs[idx].view.update(cx, |view, cx| {
                    let len = view.state().doc().len();
                    view.dispatch_spec(TransactionSpec::new().changes(ChangeSpec::replace(0, len, text)).user_event("reload"), cx);
                });
                let state = self.tabs[idx].state(cx).clone();
                self.tabs[idx].refresh(state, true);
                self.flash(format!("Reloaded {title}"), cx);
            }
            Err(e) => self.flash(format!("Reload failed: {e}"), cx),
        }
        cx.notify();
    }

    /// The window's position and size, to restore next time.
    pub(super) fn remember_window(&mut self, window: &Window) {
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (b, false),
            WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
        };
        self.settings.window = Some(WindowGeometry {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            maximized,
        });
    }
}

/// The bounds to open the window with: the saved ones when they are on a
/// display, else centered on the main display.
pub fn initial_bounds(settings: &Settings, cx: &App) -> WindowBounds {
    let fallback = WindowBounds::Windowed(Bounds::centered(None, gpui::size(px(1100.), px(720.)), cx));
    let Some(g) = settings.window else { return fallback };
    let bounds = Bounds::new(gpui::point(px(g.x), px(g.y)), gpui::size(px(g.width.max(480.)), px(g.height.max(320.))));
    let center = bounds.center();
    if !cx.displays().iter().any(|d| d.bounds().contains(&center)) {
        return fallback;
    }
    if g.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}
