//! WebAssembly modules that plugins ship, for work too heavy for a script
//! (`notey.wasm.run`). A module runs in the wasmi interpreter with no
//! imports at all, so it can only compute: no files, no network, no
//! system calls. Each call has a fuel budget and memory is capped.
//!
//! A module stays loaded for the session, so it can keep state between
//! calls. If a call fails (a trap, or out of fuel), the module is thrown
//! away and the next call starts it fresh.
//!
//! The calling convention, all on the module's own `memory`:
//! - `alloc(len: i32) -> i32` returns room for the input;
//! - `func(ptr: i32, len: i32) -> i64` takes the UTF-8 input and returns
//!   `ptr << 32 | len` of the UTF-8 output;
//! - `dealloc(ptr: i32, len: i32)`, if exported, frees both buffers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wasmi::{Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// Instructions per call: about a second or two of work.
const FUEL: u64 = 3_000_000_000;
const MAX_MEMORY: usize = 512 << 20;
const MAX_MODULE: u64 = 32 << 20;
const MAX_OUTPUT: usize = 64 << 20;

struct Loaded {
    store: Store<StoreLimits>,
    instance: Instance,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    dealloc: Option<TypedFunc<(u32, u32), ()>>,
}

thread_local! {
    static MODULES: RefCell<HashMap<PathBuf, Loaded>> = RefCell::new(HashMap::new());
}

fn load(path: &Path) -> Result<Loaded, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_MODULE {
        return Err("the module is too large".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &bytes).map_err(|e| format!("bad module: {e}"))?;
    if module.imports().next().is_some() {
        return Err("the module may not import anything".into());
    }
    let mut store = Store::new(&engine, StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build());
    store.limiter(|limits| limits);
    store.set_fuel(FUEL).map_err(|e| e.to_string())?;
    let instance = <Linker<StoreLimits>>::new(&engine).instantiate_and_start(&mut store, &module).map_err(|e| e.to_string())?;
    let memory = instance.get_memory(&store, "memory").ok_or("the module exports no memory")?;
    let alloc = instance.get_typed_func::<u32, u32>(&store, "alloc").map_err(|_| "the module exports no alloc(len)")?;
    let dealloc = instance.get_typed_func::<(u32, u32), ()>(&store, "dealloc").ok();
    Ok(Loaded { store, instance, memory, alloc, dealloc })
}

fn invoke(m: &mut Loaded, func: &str, input: &str) -> Result<String, String> {
    m.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
    let f = m
        .instance
        .get_typed_func::<(u32, u32), u64>(&m.store, func)
        .map_err(|_| format!("the module has no function {func}(ptr, len)"))?;
    let len = u32::try_from(input.len()).map_err(|_| "the input is too large")?;
    let ptr = m.alloc.call(&mut m.store, len).map_err(|e| e.to_string())?;
    m.memory.write(&mut m.store, ptr as usize, input.as_bytes()).map_err(|e| e.to_string())?;
    let packed = f.call(&mut m.store, (ptr, len)).map_err(|e| e.to_string())?;
    let (out_ptr, out_len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    if out_len > MAX_OUTPUT {
        return Err("the output is too large".into());
    }
    let mut out = vec![0u8; out_len];
    m.memory.read(&m.store, out_ptr, &mut out).map_err(|e| e.to_string())?;
    if let Some(dealloc) = &m.dealloc {
        let _ = dealloc.call(&mut m.store, (ptr, len));
        let _ = dealloc.call(&mut m.store, (out_ptr as u32, out_len as u32));
    }
    String::from_utf8(out).map_err(|_| "the module returned text that is not UTF-8".into())
}

/// Call `func` of the module at `path` with `input`.
pub fn call(path: &Path, func: &str, input: &str) -> Result<String, String> {
    MODULES.with(|modules| {
        let mut modules = modules.borrow_mut();
        if !modules.contains_key(path) {
            modules.insert(path.to_path_buf(), load(path)?);
        }
        let result = invoke(modules.get_mut(path).unwrap(), func, input);
        if result.is_err() {
            modules.remove(path);
        }
        result
    })
}

/// Forget every loaded module (plugins were reloaded or updated).
pub fn reset() {
    MODULES.with(|m| m.borrow_mut().clear());
}
