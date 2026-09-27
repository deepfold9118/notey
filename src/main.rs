#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod editor;
mod ipc;
mod plugins;
mod search;
mod session;
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

fn main() -> eframe::Result {
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
        ..Default::default()
    };
    let inbox: ipc::Inbox = Default::default();
    eframe::run_native(
        "Notey",
        options,
        Box::new(move |cc| {
            // first instance wins the socket; later windows run standalone
            let is_primary = ipc::start_listener(inbox.clone(), cc.egui_ctx.clone());
            Ok(Box::new(app::NoteyApp::new(cc, paths, inbox, is_primary)))
        }),
    )
}
