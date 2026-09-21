//! bddkit plugin serving the `browser` resource group: a real browser driven
//! over WebDriver — classic HTTP for commands, BiDi for events. Written
//! against docs/plugin-authoring.md; it must never need the host's source.

mod config;
mod find;
mod instance;
mod reply;
mod steps;
mod webdriver;

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Outside `guard`, and safe there only because it provably cannot panic:
/// `CString::new` returns a `Result` and the fallback literal has no interior
/// NUL. Anything added here must keep that property.
fn out(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| {
            CString::new("{\"ok\":false,\"error\":\"NUL in reply\"}").expect("literal")
        })
        .into_raw()
}

/// Inside `guard` at every call site.
fn input(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// A panic must never unwind across the FFI boundary. Every export is
/// guarded, including the trivial ones, so "every export is guarded" is an
/// invariant a reader checks in one pass.
fn guard(envelope_kind: &str, body: impl FnOnce() -> String) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(reply) => out(reply),
        Err(_) => out(match envelope_kind {
            "dispatch" => r#"{"status":"fatal","error":"the plugin panicked"}"#.to_string(),
            _ => r#"{"ok":false,"error":"the plugin panicked"}"#.to_string(),
        }),
    }
}

pub fn manifest_json() -> String {
    serde_json::json!({
        "name": "browser",
        "version": env!("CARGO_PKG_VERSION"),
        "groups": ["browser"],
        // One browser per feature file, reset per scenario: per-scenario state
        // means per_worker, by the contract's own rule.
        "concurrency": "per_worker",
        "fields": { "browser": config::fields_json() },
    })
    .to_string()
}

#[unsafe(no_mangle)]
pub extern "C" fn bddkit_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn bddkit_manifest() -> *mut c_char {
    guard("envelope", manifest_json)
}

/// # Safety
/// `s` must be a pointer this library returned and has not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bddkit_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Eager, at startup, for every declared instance, with nothing connected.
/// Rejecting here is what turns a config typo into exit 2 before the first
/// request instead of a failure halfway through the suite.
#[unsafe(no_mangle)]
pub extern "C" fn bddkit_validate_config(request: *const c_char) -> *mut c_char {
    guard("envelope", move || {
        let value: serde_json::Value = match serde_json::from_str(&input(request)) {
            Ok(v) => v,
            Err(e) => return reply::err(e.to_string()),
        };
        match config::InstanceConfig::parse(&value["config"]) {
            Ok(_) => reply::ok(),
            Err(error) => reply::err(error),
        }
    })
}

/// A handle is an index into this table, never a pointer. Instances sit
/// behind `Arc` so a step never runs while this lock is held.
static INSTANCES: Mutex<Option<HashMap<u64, Arc<instance::Instance>>>> = Mutex::new(None);
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn take_instance(handle: u64) -> Option<Arc<instance::Instance>> {
    INSTANCES
        .lock()
        .expect("instances")
        .get_or_insert_with(HashMap::new)
        .get(&handle)
        .cloned()
}

/// For tests: how many handles are live.
pub fn live_instances() -> usize {
    INSTANCES
        .lock()
        .expect("instances")
        .as_ref()
        .map_or(0, HashMap::len)
}

fn parse_request(
    request: *const c_char,
) -> Result<(config::InstanceConfig, serde_json::Value), String> {
    let value: serde_json::Value =
        serde_json::from_str(&input(request)).map_err(|e| e.to_string())?;
    let config = config::InstanceConfig::parse(&value["config"])?;
    Ok((config, value))
}

#[unsafe(no_mangle)]
pub extern "C" fn bddkit_list_steps() -> *mut c_char {
    guard("envelope", steps::steps_json)
}

/// Stateless: opens a session, reads the browser's name, closes it. The one
/// export besides `init_instance` that may be slow and may need the network.
#[unsafe(no_mangle)]
pub extern "C" fn bddkit_probe_config(request: *const c_char) -> *mut c_char {
    guard("envelope", move || match parse_request(request) {
        Ok((config, _)) => match instance::Instance::probe(&config) {
            Ok(()) => reply::ok(),
            Err(error) => reply::err(error),
        },
        Err(error) => reply::err(error),
    })
}

/// Lazy: the first browser step of a feature file. Every call returns a
/// handle distinct from every live one.
#[unsafe(no_mangle)]
pub extern "C" fn bddkit_init_instance(request: *const c_char) -> *mut c_char {
    guard("envelope", move || {
        let (config, _) = match parse_request(request) {
            Ok(x) => x,
            Err(error) => return reply::err(error),
        };
        // `debug` is per dispatch; an instance opened outside one traces
        // nothing until a step under `I am in debug mode` asks.
        let instance = match instance::Instance::open(config, false) {
            Ok(i) => i,
            Err(error) => return reply::err(error),
        };
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
        INSTANCES
            .lock()
            .expect("instances")
            .get_or_insert_with(HashMap::new)
            .insert(handle, Arc::new(instance));
        serde_json::json!({"ok": true, "handle": handle}).to_string()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn bddkit_dispatch(
    handle: u64,
    step_index: u32,
    request: *const c_char,
) -> *mut c_char {
    guard("dispatch", move || {
        let value: serde_json::Value = match serde_json::from_str(&input(request)) {
            Ok(v) => v,
            Err(e) => return reply::fatal(&e.to_string(), &[]),
        };
        let Some(instance) = take_instance(handle) else {
            return reply::fatal("unknown handle", &[]);
        };
        steps::route(&instance, step_index, &steps::Request::parse(&value))
    })
}

/// Called at the scenario boundary for every instance this file has used.
#[unsafe(no_mangle)]
pub extern "C" fn bddkit_reset_scenario(handle: u64) -> *mut c_char {
    guard("envelope", move || match take_instance(handle) {
        Some(instance) => match instance.reset() {
            Ok(()) => reply::ok(),
            Err(error) => reply::err(error),
        },
        None => reply::err("unknown handle"),
    })
}

/// When the feature file ends, and swept at the end of the run for a file
/// that panicked. Closes the session; the table entry goes either way.
#[unsafe(no_mangle)]
pub extern "C" fn bddkit_drop_instance(handle: u64) -> *mut c_char {
    guard("envelope", move || {
        let removed = INSTANCES
            .lock()
            .expect("instances")
            .get_or_insert_with(HashMap::new)
            .remove(&handle);
        match removed {
            Some(instance) => match instance.close() {
                Ok(()) => reply::ok(),
                Err(error) => reply::err(error),
            },
            None => reply::err("unknown handle"),
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_manifest_claims_the_browser_group_and_is_per_worker() {
        let v: serde_json::Value = serde_json::from_str(&crate::manifest_json()).expect("JSON");
        assert_eq!(
            v["name"], "browser",
            "the lock entry must name this plugin `browser`"
        );
        assert_eq!(v["groups"], serde_json::json!(["browser"]));
        assert_eq!(
            v["concurrency"], "per_worker",
            "the plugin keeps per-scenario state"
        );
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    }
}
