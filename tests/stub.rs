//! The exports driven in-process against a fake WebDriver server. Nothing
//! here needs a browser; what it pins is the plugin's side of the protocol
//! and its replies to the host.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::{Json, Router};
use serde_json::{Value, json};

pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
/// A 1×1 transparent PNG, base64.
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

#[derive(Clone, Default)]
pub struct StubElement {
    /// Found when the strategy's value *contains* this — good enough for a stub.
    pub key: String,
    pub text: String,
    pub value: String,
    pub displayed: bool,
    pub selected: bool,
    pub attrs: HashMap<String, String>,
}

#[derive(Default)]
pub struct StubState {
    /// `METHOD /path body`, in order.
    pub calls: Vec<String>,
    pub url: String,
    pub title: String,
    pub elements: Vec<StubElement>,
    /// `find` answers "no such element" this many times first.
    pub not_found_first: u32,
    pub script_result: Value,
}

type Shared = Arc<Mutex<StubState>>;

pub struct Stub {
    pub url: String,
    pub state: Shared,
    _rt: tokio::runtime::Runtime,
}

pub fn start_stub(state: StubState) -> Stub {
    let shared: Shared = Arc::new(Mutex::new(state));
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = Router::new()
        .fallback(handle)
        .with_state(Arc::clone(&shared));
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    Stub {
        url: format!("http://{addr}"),
        state: shared,
        _rt: rt,
    }
}

fn reply(v: Value) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({"value": v})))
}

fn error(status: StatusCode, error: &str, message: String) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({"value": {"error": error, "message": message, "stacktrace": ""}})),
    )
}

fn element_index(id: &str) -> Option<usize> {
    id.strip_prefix('e')?.parse().ok()
}

async fn handle(
    State(state): State<Shared>,
    method: Method,
    uri: Uri,
    body: String,
) -> (StatusCode, Json<Value>) {
    let mut st = state.lock().expect("stub state");
    let path = uri.path().to_string();
    st.calls
        .push(format!("{method} {path} {body}").trim().to_string());
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let req: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    match (method.as_str(), segments.as_slice()) {
        ("GET", ["status"]) => reply(json!({"ready": true, "message": "stub"})),
        ("POST", ["session"]) => reply(
            json!({"sessionId": "s1", "capabilities": {"browserName": "chrome", "browserVersion": "131.0"}}),
        ),
        ("DELETE", ["session", "s1"]) => reply(Value::Null),
        ("POST", ["session", "s1", "url"]) => {
            st.url = req["url"].as_str().unwrap_or("").to_string();
            reply(Value::Null)
        }
        ("GET", ["session", "s1", "url"]) => reply(json!(st.url)),
        ("GET", ["session", "s1", "title"]) => reply(json!(st.title)),
        ("POST", ["session", "s1", "refresh"]) => reply(Value::Null),
        ("POST", ["session", "s1", "window", "rect"]) => {
            reply(json!({"width": req["width"], "height": req["height"], "x": 0, "y": 0}))
        }
        ("DELETE", ["session", "s1", "cookie"]) => reply(Value::Null),
        ("POST", ["session", "s1", "execute", "sync"]) => reply(st.script_result.clone()),
        ("GET", ["session", "s1", "screenshot"]) => reply(json!(PNG_1X1)),
        ("POST", ["session", "s1", "element"])
        | ("POST", ["session", "s1", "element", _, "element"]) => {
            if st.not_found_first > 0 {
                st.not_found_first -= 1;
                return error(
                    StatusCode::NOT_FOUND,
                    "no such element",
                    "stub: not yet".to_string(),
                );
            }
            let value = req["value"].as_str().unwrap_or("").to_string();
            match st.elements.iter().position(|e| value.contains(&e.key)) {
                Some(i) => reply(json!({ELEMENT_KEY: format!("e{i}")})),
                None => error(
                    StatusCode::NOT_FOUND,
                    "no such element",
                    format!("nothing matches {value}"),
                ),
            }
        }
        ("POST" | "GET", ["session", "s1", "element", id, rest @ ..]) => {
            let Some(i) = element_index(id) else {
                return error(
                    StatusCode::NOT_FOUND,
                    "stale element reference",
                    id.to_string(),
                );
            };
            let e = &mut st.elements[i];
            match (method.as_str(), rest) {
                ("POST", ["click"]) => {
                    e.selected = !e.selected;
                    reply(Value::Null)
                }
                ("POST", ["clear"]) => {
                    e.value.clear();
                    reply(Value::Null)
                }
                ("POST", ["value"]) => {
                    e.value.push_str(req["text"].as_str().unwrap_or(""));
                    reply(Value::Null)
                }
                ("GET", ["text"]) => reply(json!(e.text)),
                ("GET", ["attribute", name]) => {
                    reply(e.attrs.get(*name).map_or(Value::Null, |v| json!(v)))
                }
                ("GET", ["property", "value"]) => reply(json!(e.value)),
                ("GET", ["displayed"]) => reply(json!(e.displayed)),
                ("GET", ["selected"]) => reply(json!(e.selected)),
                _ => error(StatusCode::NOT_FOUND, "unknown command", path.clone()),
            }
        }
        _ => error(StatusCode::NOT_FOUND, "unknown command", path.clone()),
    }
}

// ---- driving the exports ----

fn take(raw: *mut c_char) -> Value {
    let s = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { bddkit_browser::bddkit_free_string(raw) };
    serde_json::from_str(&s)
        .unwrap_or_else(|e| panic!("the plugin replied with non-JSON {s:?}: {e}"))
}

fn c(s: &str) -> CString {
    CString::new(s).expect("no NUL")
}

pub fn config_for(stub: &Stub, extra: Value) -> Value {
    let mut cfg = json!({"browser": "chrome", "url": stub.url, "base_url": "http://app.test"});
    if let (Some(base), Some(over)) = (cfg.as_object_mut(), extra.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    json!({"group": "browser", "instance": "main", "config": cfg, "options": {}})
}

pub fn init(stub: &Stub, extra: Value) -> u64 {
    let r = take(bddkit_browser::bddkit_init_instance(
        c(&config_for(stub, extra).to_string()).as_ptr(),
    ));
    assert_eq!(r["ok"], true, "init failed: {r}");
    r["handle"].as_u64().expect("handle")
}

pub fn dispatch(
    handle: u64,
    index: u32,
    args: &[&str],
    docstring: Option<&str>,
    artifacts: &Path,
) -> Value {
    let req = json!({
        "args": args,
        "docstring": docstring,
        "table": null,
        "artifacts_dir": artifacts.display().to_string(),
        "workspace_dir": artifacts.display().to_string(),
        "debug": false,
        "options": {"polling": {"timeout_secs": 1, "interval_ms": 100}},
    });
    take(bddkit_browser::bddkit_dispatch(
        handle,
        index,
        c(&req.to_string()).as_ptr(),
    ))
}

pub fn reset(handle: u64) -> Value {
    take(bddkit_browser::bddkit_reset_scenario(handle))
}

pub fn drop_instance(handle: u64) -> Value {
    take(bddkit_browser::bddkit_drop_instance(handle))
}

pub fn calls(stub: &Stub) -> Vec<String> {
    stub.state.lock().expect("state").calls.clone()
}

pub fn artifacts() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

// ---- Task 5 tests ----

const AM_ON: u32 = 0;
const SHOULD_BE_ON: u32 = 15;
const TITLE_SHOULD_BE: u32 = 16;

/// `INSTANCES` (in `src/lib.rs`) is one process-wide table, and cargo runs
/// this binary's `#[test]` functions on several threads by default. Every
/// test below serializes on this lock so a sibling test's `init`/`drop`
/// cannot land inside another test's `live_instances()` before/after window
/// — the delta the module doc promises only holds with that guarantee.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn init_opens_a_headless_session_and_sizes_the_window() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({"window": "1024x600"}));
    let calls = calls(&stub);
    let session = calls
        .iter()
        .find(|c| c.starts_with("POST /session "))
        .expect("a session was opened");
    assert!(session.contains("\"webSocketUrl\":true"), "{session}");
    assert!(session.contains("--headless=new"), "{session}");
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("POST /session/s1/window/rect") && c.contains("\"width\":1024")),
        "{calls:?}"
    );
    assert_eq!(drop_instance(handle)["ok"], true);
}

#[test]
fn i_am_on_joins_a_relative_path_to_base_url_and_keeps_an_absolute_one() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    let dir = artifacts();
    assert_eq!(
        dispatch(handle, AM_ON, &["/login"], None, dir.path())["status"],
        "passed"
    );
    assert!(
        calls(&stub)
            .iter()
            .any(|c| c.contains(r#"{"url":"http://app.test/login"}"#))
    );
    assert_eq!(
        dispatch(handle, AM_ON, &["https://other.test/x"], None, dir.path())["status"],
        "passed"
    );
    assert!(
        calls(&stub)
            .iter()
            .any(|c| c.contains(r#"{"url":"https://other.test/x"}"#))
    );
    drop_instance(handle);
}

#[test]
fn a_relative_path_without_base_url_is_fatal_naming_the_key() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let r = take(bddkit_browser::bddkit_init_instance(
        c(&json!({"group": "browser", "instance": "main", "config": {"browser": "chrome", "url": stub.url}, "options": {}}).to_string()).as_ptr(),
    ));
    let handle = r["handle"].as_u64().expect("handle");
    let dir = artifacts();
    let reply = dispatch(handle, AM_ON, &["/login"], None, dir.path());
    assert_eq!(reply["status"], "fatal");
    assert!(
        reply["error"].as_str().expect("error").contains("base_url"),
        "{reply}"
    );
    drop_instance(handle);
}

#[test]
fn should_be_on_and_title_answer_not_yet_on_a_mismatch() {
    let _guard = serial();
    let stub = start_stub(StubState {
        url: "http://app.test/login?error=1".into(),
        title: "Sign in".into(),
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    assert_eq!(
        dispatch(handle, SHOULD_BE_ON, &["/login?error=1"], None, dir.path())["status"],
        "passed"
    );
    let miss = dispatch(handle, SHOULD_BE_ON, &["/dashboard"], None, dir.path());
    assert_eq!(miss["status"], "not_yet");
    assert!(
        miss["error"]
            .as_str()
            .expect("error")
            .contains("/login?error=1"),
        "{miss}"
    );
    assert_eq!(
        dispatch(handle, TITLE_SHOULD_BE, &["Sign in"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, TITLE_SHOULD_BE, &["Dashboard"], None, dir.path())["status"],
        "not_yet"
    );
    drop_instance(handle);
}

#[test]
fn reset_clears_storage_and_cookies_before_leaving_the_page() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    stub.state.lock().expect("state").calls.clear();
    assert_eq!(reset(handle)["ok"], true);
    let calls = calls(&stub);
    let at = |needle: &str| {
        calls
            .iter()
            .position(|c| c.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing in {calls:?}"))
    };
    let storage = at("execute/sync");
    let cookies = at("DELETE /session/s1/cookie");
    let blank = at("about:blank");
    assert!(
        storage < cookies && cookies < blank,
        "wrong order: {calls:?}"
    );
    assert!(calls[storage].contains("localStorage.clear()"));
    drop_instance(handle);
}

#[test]
fn drop_deletes_the_session_and_forgets_the_handle() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    let before = bddkit_browser::live_instances();
    assert_eq!(drop_instance(handle)["ok"], true);
    assert!(calls(&stub).iter().any(|c| c == "DELETE /session/s1"));
    assert_eq!(bddkit_browser::live_instances(), before - 1);
    let dir = artifacts();
    let after = dispatch(handle, AM_ON, &["/"], None, dir.path());
    assert_eq!(after["status"], "fatal");
    assert_eq!(after["error"], "unknown handle");
}

#[test]
fn a_probe_opens_reads_and_closes_without_keeping_a_handle() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let before = bddkit_browser::live_instances();
    let r = take(bddkit_browser::bddkit_probe_config(
        c(&config_for(&stub, json!({})).to_string()).as_ptr(),
    ));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r.get("handle").is_none());
    assert_eq!(bddkit_browser::live_instances(), before);
    let calls = calls(&stub);
    assert!(calls.iter().any(|c| c.starts_with("POST /session ")));
    assert!(calls.iter().any(|c| c == "DELETE /session/s1"));
}

#[test]
fn an_unreachable_endpoint_fails_init_with_the_url_in_the_message() {
    let _guard = serial();
    let r = take(bddkit_browser::bddkit_init_instance(
        c(&json!({"group": "browser", "instance": "main", "config": {"browser": "chrome", "url": "http://127.0.0.1:1"}, "options": {}}).to_string()).as_ptr(),
    ));
    assert_eq!(r["ok"], false);
    assert!(
        r["error"].as_str().expect("error").contains("127.0.0.1:1"),
        "{r}"
    );
}

#[test]
fn validate_config_refuses_a_typo_through_the_export() {
    let _guard = serial();
    let r = take(bddkit_browser::bddkit_validate_config(
        c(&json!({"group": "browser", "instance": "main", "config": {"browser": "chrome", "url": "http://h:4444", "headles": true}, "options": {}}).to_string()).as_ptr(),
    ));
    assert_eq!(r["ok"], false);
    assert!(
        r["error"].as_str().expect("error").contains("headles"),
        "{r}"
    );
}
