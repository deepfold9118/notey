//! Close the window normally when Windows ends the session.
//!
//! At shutdown, Windows sends `WM_QUERYENDSESSION` and then `WM_ENDSESSION`
//! to each window. The Restart Manager sends the same messages, for example
//! when the Inno Setup upgrade must replace `notey.exe`. winit does not
//! handle `WM_ENDSESSION`, and Windows stops the process when the window
//! procedure returns. Thus eframe did not close the window and Notey did not
//! save the session.
//!
//! This module puts a small window procedure in front of the winit procedure.
//! On `WM_ENDSESSION`, it sends `WM_CLOSE` to winit. Then it dispatches
//! messages until eframe closes the window (this saves the session) or until
//! the time limit.

/// Put the session-end handler on the main window. `hwnd` is the main window.
pub fn install(hwnd: Option<isize>) {
    if let Some(hwnd) = hwnd {
        win::install(hwnd);
    }
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::time::{Duration, Instant};

    const GWLP_WNDPROC: i32 = -4;
    const WM_CLOSE: u32 = 0x0010;
    const WM_ENDSESSION: u32 = 0x0016;
    const PM_REMOVE: u32 = 0x0001;
    const QS_ALLINPUT: u32 = 0x04FF;
    const MWMO_INPUTAVAILABLE: u32 = 0x0004;

    /// Windows waits approximately 5 s for a window before it shows the
    /// "This app is preventing shutdown" screen.
    const CLOSE_TIMEOUT: Duration = Duration::from_secs(4);

    /// The winit window procedure.
    static PREV_PROC: AtomicIsize = AtomicIsize::new(0);

    #[repr(C)]
    struct Msg {
        hwnd: isize,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
        l_private: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn CallWindowProcW(prev: isize, hwnd: isize, msg: u32, w: usize, l: isize) -> isize;
        fn IsWindow(hwnd: isize) -> i32;
        fn PeekMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn MsgWaitForMultipleObjectsEx(
            count: u32,
            handles: *const isize,
            millis: u32,
            wake_mask: u32,
            flags: u32,
        ) -> u32;
    }

    pub fn install(hwnd: isize) {
        let proc_ptr = window_proc as unsafe extern "system" fn(isize, u32, usize, isize) -> isize;
        let prev = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc_ptr as isize) };
        PREV_PROC.store(prev, Ordering::SeqCst);
    }

    unsafe extern "system" fn window_proc(hwnd: isize, msg: u32, w: usize, l: isize) -> isize {
        let prev = PREV_PROC.load(Ordering::SeqCst);
        if msg == WM_ENDSESSION && w != 0 {
            // The session ends. Ask eframe to close, as for the close button.
            CallWindowProcW(prev, hwnd, WM_CLOSE, 0, 0);
            // eframe closes the window during a frame. Dispatch messages here
            // so that the frame runs before this procedure returns.
            let deadline = Instant::now() + CLOSE_TIMEOUT;
            while IsWindow(hwnd) != 0 {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let mut m: Msg = std::mem::zeroed();
                if PeekMessageW(&mut m, 0, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&m);
                    DispatchMessageW(&m);
                } else {
                    MsgWaitForMultipleObjectsEx(
                        0,
                        std::ptr::null(),
                        left.as_millis().min(u32::MAX as u128) as u32,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                }
            }
            return 0;
        }
        CallWindowProcW(prev, hwnd, msg, w, l)
    }
}

#[cfg(not(windows))]
mod win {
    pub fn install(_hwnd: isize) {}
}
