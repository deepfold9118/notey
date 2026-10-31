#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod a11y;
mod app;
mod doc;
mod highlight;
mod ipc;
mod markdown;
// Carried over from the egui version; the plugin manager that uses the
// rest (install, update, enable) is not ported yet.
#[allow(dead_code)]
mod packages;
mod plugins;
mod prompt;
mod search;
mod session;
mod settings;
mod shutdown;
mod spell;
mod spellcheck;
mod styles;
mod syntax;
mod theme;
mod theme_import;
mod theme_marketplace;

use std::path::PathBuf;

use gpui::{px, size, App, AppContext, Application, TitlebarOptions, Window, WindowOptions};

/// Release builds have no console, so a panic would otherwise vanish
/// without a trace. Append it to `%APPDATA%\Notey\crash.log`.
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = settings::data_dir().and_then(|d| d.parent().map(|p| p.to_path_buf())) {
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
            let entry = format!(
                "[{}] Notey {} panicked on thread '{thread}' at {location}: {message}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                env!("CARGO_PKG_VERSION"),
            );
            use std::io::Write;
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(entry.as_bytes());
            }
        }
        default_hook(info);
    }));
}

fn main() {
    install_crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let new_window = args.iter().any(|a| a == "--new-window");
    let paths: Vec<PathBuf> =
        args.iter().filter(|a| !a.starts_with("--")).map(PathBuf::from).filter(|p| p.is_file()).collect();

    // Default behavior: open files as tabs in the running instance, if any.
    // `--new-window` (or launching with no files) always opens a new window.
    if !new_window && !paths.is_empty() && ipc::try_forward(&paths) {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        app::init(cx);
        prompt::init(cx);
        let options = WindowOptions {
            window_bounds: Some(app::initial_bounds(&settings::Settings::load(), cx)),
            // The title bar is Notey's own, with the tabs in it.
            titlebar: Some(TitlebarOptions { title: Some("Notey".into()), appears_transparent: true, ..Default::default() }),
            window_min_size: Some(size(px(480.), px(320.))),
            app_id: Some("Notey".into()),
            // Shown below, once the accessibility adapter is attached (it must
            // be before the window is first shown).
            show: false,
            ..Default::default()
        };
        let (inbox, received) = futures::channel::mpsc::unbounded();
        let handle = cx.open_window(options, move |window, cx| {
            // The first instance owns the socket; later windows run standalone.
            let hwnd = main_hwnd(window);
            let (a11y, a11y_requests) = a11y::install(hwnd);
            shutdown::install(hwnd);
            let is_primary = ipc::start_listener(inbox, hwnd);
            cx.new(|cx| app::NoteyApp::new(paths, is_primary, received, a11y, a11y_requests, window, cx))
        })
        .expect("open the Notey window");
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// The Win32 handle of the window, if the platform gives one.
fn main_hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}
