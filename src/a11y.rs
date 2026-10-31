//! Screen reader support (UI Automation, through AccessKit), as egui gave
//! the egui version. GPUI has no accessibility tree, so Notey describes its
//! window itself (`app/accessibility.rs`) and AccessKit's subclassing
//! adapter answers `WM_GETOBJECT` with it.
//!
//! The adapter asks for the tree inside the window procedure, where the app
//! can't be reached, so it only signals the app, which then sends the full
//! tree. Actions from assistive technology arrive the same way.

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

/// Something an assistive technology asked for.
pub enum Request {
    /// A screen reader started reading the window: send the full tree.
    Activate,
    /// Click, focus or select something.
    Action(ActionRequest),
}

/// The accessibility tree of the main window.
pub struct A11y {
    #[cfg(windows)]
    adapter: accesskit_windows::SubclassingAdapter,
}

impl A11y {
    /// Send a tree update, if a screen reader is reading the window. `build`
    /// runs only then.
    pub fn update(&mut self, build: impl FnOnce() -> TreeUpdate) {
        #[cfg(windows)]
        if let Some(events) = self.adapter.update_if_active(build) {
            events.raise();
        }
        #[cfg(not(windows))]
        let _ = build;
    }
}

struct Activation(UnboundedSender<Request>);

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let _ = self.0.unbounded_send(Request::Activate);
        None
    }
}

struct Actions(UnboundedSender<Request>);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.unbounded_send(Request::Action(request));
    }
}

/// Attach the tree to the window `hwnd`, which must not have been shown yet.
pub fn install(hwnd: Option<isize>) -> (Option<A11y>, UnboundedReceiver<Request>) {
    let (tx, rx) = unbounded();
    #[cfg(windows)]
    if let Some(hwnd) = hwnd {
        let hwnd = accesskit_windows::HWND(hwnd as *mut std::ffi::c_void);
        let adapter = accesskit_windows::SubclassingAdapter::new(hwnd, Activation(tx.clone()), Actions(tx));
        return (Some(A11y { adapter }), rx);
    }
    let _ = (hwnd, Activation(tx.clone()), Actions(tx));
    (None, rx)
}
