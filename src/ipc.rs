//! Single-instance forwarding: a second `notey.exe <file>` hands its paths
//! to the running instance over a local socket (named pipe on Windows) and
//! exits, so files open as tabs in the existing window.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Stream, ToNsName,
};

const SOCKET_NAME: &str = "notey-single-instance.sock";

/// Paths received from other processes, drained by the UI thread each frame.
pub type Inbox = Arc<Mutex<Vec<PathBuf>>>;

/// Strip the `\\?\` verbatim prefix that `canonicalize` adds on Windows.
pub fn clean_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => p,
    }
}

/// Try to hand `paths` to an already-running instance.
/// Returns true if an instance accepted them (caller should exit).
pub fn try_forward(paths: &[PathBuf]) -> bool {
    let Ok(name) = SOCKET_NAME.to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    let Ok(mut conn) = Stream::connect(name) else {
        return false;
    };
    let mut sent_any = false;
    for p in paths {
        let abs = clean_path(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
        if writeln!(conn, "{}", abs.display()).is_ok() {
            sent_any = true;
        }
    }
    let _ = conn.flush();
    sent_any
}

/// Become the primary instance: listen for forwarded paths on a background
/// thread. Returns false if another instance already owns the socket (this
/// process then simply runs standalone).
pub fn start_listener(inbox: Inbox, ctx: eframe::egui::Context) -> bool {
    let Ok(name) = SOCKET_NAME.to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    let Ok(listener) = ListenerOptions::new().name(name).create_sync() else {
        return false;
    };
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let reader = BufReader::new(conn);
            let mut got_any = false;
            for line in reader.lines().map_while(Result::ok) {
                let line = line.trim();
                if !line.is_empty() {
                    inbox.lock().unwrap().push(PathBuf::from(line));
                    got_any = true;
                }
            }
            if got_any {
                ctx.request_repaint();
            }
        }
    });
    true
}
