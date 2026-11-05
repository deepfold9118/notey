//! Notey's Math plugin engine: Calca-style math in any text. A line such
//! as `price * 3 =>` gets its result written after the `=>`, with
//! definitions, functions, symbolic algebra, solving, derivatives,
//! matrices, complex numbers and money (see the plugin's README).
//!
//! Built to WebAssembly (`wasm32-unknown-unknown`), the plugin's Rhai script
//! calls it through `notey.wasm.run`: `alloc` makes room for the input,
//! and `evaluate`/`toggle` take a JSON request and return a pointer and
//! length (packed in a u64) to a JSON reply, freed with `dealloc`.

pub mod builtins;
pub mod calculus;
pub mod currency;
pub mod doc;
pub mod eval;
pub mod expr;
pub mod format;
pub mod matrix;
pub mod num;
pub mod parse;
pub mod solve;

use std::cell::RefCell;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use doc::{BufState, Mode};

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    buffer: i64,
    #[serde(default)]
    text: String,
    /// The tab's language ("Plain Text", "Markdown", "Rust", ...).
    #[serde(default)]
    language: String,
    #[serde(default)]
    path: String,
    /// The exchange-rate source's JSON, if fetched.
    #[serde(default)]
    rates: String,
}

#[derive(Serialize, Default)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Serialize, Default)]
struct Reply {
    edits: Vec<Edit>,
    /// Money was used, so the exchange rates should be fresh.
    currency: bool,
    /// A conversion waits for the exchange rates.
    needs_rates: bool,
    paused: bool,
}

thread_local! {
    static TABS: RefCell<HashMap<i64, BufState>> = RefCell::new(HashMap::new());
}

/// How carefully a tab is treated (see doc.rs).
pub fn mode_for(language: &str, path: &str) -> Mode {
    if path.to_ascii_lowercase().ends_with(".calca") {
        return Mode::Calca;
    }
    match language {
        "" | "Plain Text" | "Markdown" | "MultiMarkdown" | "Text" => Mode::Prose,
        _ => Mode::Code,
    }
}

fn evaluate_json(input: &[u8]) -> String {
    let Ok(req) = serde_json::from_slice::<Request>(input) else {
        return "{\"edits\":[],\"error\":\"bad request\"}".into();
    };
    let rates = (!req.rates.is_empty()).then(|| currency::Rates::parse(&req.rates)).flatten();
    let mode = mode_for(&req.language, &req.path);
    let out = TABS.with(|t| {
        let mut tabs = t.borrow_mut();
        let state = tabs.entry(req.buffer).or_default();
        let out = doc::process(&req.text, mode, rates, state);
        (out, state.paused)
    });
    let (out, paused) = out;
    let reply = Reply {
        edits: out.edits.into_iter().map(|(start, end, text)| Edit { start, end, text }).collect(),
        currency: out.currency,
        needs_rates: out.needs_rates,
        paused,
    };
    serde_json::to_string(&reply).unwrap_or_default()
}

fn toggle_json(input: &[u8]) -> String {
    let buffer = serde_json::from_slice::<Request>(input).map(|r| r.buffer).unwrap_or_default();
    let paused = TABS.with(|t| {
        let mut tabs = t.borrow_mut();
        let state = tabs.entry(buffer).or_default();
        state.paused = !state.paused;
        state.paused
    });
    serde_json::to_string(&Reply { paused, ..Reply::default() }).unwrap_or_default()
}

/// Run a request from the host against our memory.
///
/// # Safety
/// `ptr..ptr+len` must be memory from `alloc`.
unsafe fn run(ptr: *mut u8, len: u32, f: fn(&[u8]) -> String) -> u64 {
    let input = std::slice::from_raw_parts(ptr, len as usize);
    let out = f(input).into_bytes().into_boxed_slice();
    let n = out.len() as u64;
    let p = Box::into_raw(out) as *mut u8 as usize as u64;
    (p << 32) | n
}

#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from `alloc` or a reply.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: u32) {
    drop(Vec::from_raw_parts(ptr, 0, len as usize));
}

/// # Safety
/// The input must be memory from `alloc`.
#[no_mangle]
pub unsafe extern "C" fn evaluate(ptr: *mut u8, len: u32) -> u64 {
    run(ptr, len, evaluate_json)
}

/// Pause or resume a tab.
///
/// # Safety
/// The input must be memory from `alloc`.
#[no_mangle]
pub unsafe extern "C" fn toggle(ptr: *mut u8, len: u32) -> u64 {
    run(ptr, len, toggle_json)
}
