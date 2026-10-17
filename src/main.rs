#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod editor;
mod ipc;
mod packages;
mod plugin_manager;
mod plugins;
mod search;
mod session;
mod shutdown;
mod spell;
mod syntax;
mod theme;
mod theme_marketplace;

use std::path::PathBuf;

use eframe::egui;

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../notey_icon.ico");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// Release builds have no console, so a panic would otherwise vanish
/// without a trace. Append it to `%APPDATA%\Notey\crash.log`.
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = eframe::storage_dir("Notey").and_then(|d| d.parent().map(|p| p.to_path_buf())) {
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
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crash.log"))
            {
                let _ = f.write_all(entry.as_bytes());
            }
        }
        default_hook(info);
    }));
}

fn main() -> eframe::Result {
    install_crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let new_window = args.iter().any(|a| a == "--new-window");
    let paths: Vec<PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();

    // Default behavior: open files as tabs in the running instance, if any.
    // `--new-window` (or launching with no files) always opens a new window.
    if !new_window && !paths.is_empty() && ipc::try_forward(&paths) {
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([480.0, 320.0])
        .with_decorations(false) // custom Fluent title bar
        .with_title("Notey");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    let inbox: ipc::Inbox = Default::default();
    eframe::run_native(
        "Notey",
        options,
        Box::new(move |cc| {
            // first instance wins the socket; later windows run standalone
            let hwnd = main_hwnd(cc);
            shutdown::install(hwnd);
            let is_primary = ipc::start_listener(inbox.clone(), cc.egui_ctx.clone(), hwnd);
            Ok(Box::new(app::NoteyApp::new(cc, paths, inbox, is_primary)))
        }),
    )
}

/// The wgpu settings. On Windows, Notey does not use the OpenGL backend.
/// That backend makes a hidden window on a thread that does not read its
/// messages. Windows then reports the window as "not responding". As a
/// result, the Restart Manager (Inno Setup upgrade) does not close Notey,
/// and the shutdown waits for that window. DirectX 12 and Vulkan do not make
/// such a window. The `WGPU_BACKEND` environment variable still selects a
/// backend.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    #[allow(unused_mut)]
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    #[cfg(windows)]
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::from_env().unwrap_or(eframe::wgpu::Backends::PRIMARY);
    }
    options
}

/// The Win32 handle of the main window, if the platform gives one.
fn main_hwnd(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}
