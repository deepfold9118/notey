//! Single-instance forwarding: a second `notey.exe <file>` hands its paths
//! to the running instance over a local socket (named pipe on Windows) and
//! exits, so files open as tabs in the existing window.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use futures::channel::mpsc::UnboundedSender;
use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Stream, ToNsName,
};

const SOCKET_NAME: &str = "notey-single-instance.sock";

/// The socket name. A test run with its own data folder (`NOTEY_DATA_DIR`)
/// gets its own socket, so it neither forwards files to nor takes over from
/// the Notey in everyday use.
fn socket_name() -> String {
    match std::env::var_os("NOTEY_DATA_DIR") {
        Some(_) => format!("test-{SOCKET_NAME}"),
        None => SOCKET_NAME.to_string(),
    }
}

/// Paths received from other processes, for the UI thread.
pub type Inbox = UnboundedSender<PathBuf>;

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
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    let Ok(mut conn) = Stream::connect(name) else {
        return false;
    };
    // Explorer gives this process the right to set the foreground window.
    // Give that right to the running instance, so that it can come to the front.
    win::allow_foreground();
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
/// process then simply runs standalone). `hwnd` is the main window. The
/// listener restores it and brings it to the front when files arrive.
pub fn start_listener(inbox: Inbox, hwnd: Option<isize>) -> bool {
    let Ok(name) = socket_name().to_ns_name::<GenericNamespaced>() else {
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
                    let _ = inbox.unbounded_send(PathBuf::from(line));
                    got_any = true;
                }
            }
            if got_any {
                if let Some(hwnd) = hwnd {
                    win::activate(hwnd);
                }
            }
        }
    });
    true
}

/// Win32 calls that put the window in front, as Notepad++ does.
#[cfg(windows)]
mod win {
    const ASFW_ANY: u32 = u32::MAX;
    const SW_RESTORE: i32 = 9;

    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn ShowWindowAsync(hwnd: isize, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
    }

    pub fn allow_foreground() {
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }

    /// Restore the window if it is minimized, then make it the foreground window.
    pub fn activate(hwnd: isize) {
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindowAsync(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub fn allow_foreground() {}
    pub fn activate(_hwnd: isize) {}
}
