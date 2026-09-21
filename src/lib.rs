//! bddkit plugin serving the `browser` resource group: a real browser driven
//! over WebDriver — classic HTTP for commands, BiDi for events. Written
//! against docs/plugin-authoring.md; it must never need the host's source.

mod config;
mod reply;

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

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
