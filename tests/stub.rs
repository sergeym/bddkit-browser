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
    /// When true the session reply advertises `webSocketUrl` = the stub's `/bidi`.
    pub bidi: bool,
    /// Sent on the socket right after the subscribe is acknowledged.
    pub events: Vec<Value>,
    /// Filled by `start_stub`.
    pub ws_url: String,
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
    shared.lock().expect("state").ws_url = format!("ws://{addr}/bidi");
    let app = Router::new()
        .route("/bidi", axum::routing::get(bidi_socket))
        .fallback(handle)
        .with_state(Arc::clone(&shared));
    rt.spawn(async move { axum::serve(listener, app).await.expect("serve") });
    Stub {
        url: format!("http://{addr}"),
        state: shared,
        _rt: rt,
    }
}

async fn bidi_socket(
    State(state): State<Shared>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    upgrade.on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        // The subscribe comes first; acknowledge it by id.
        if let Some(Ok(Message::Text(text))) = socket.recv().await {
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let ack = json!({"id": v["id"], "type": "success", "result": {}}).to_string();
            let _ = socket.send(Message::Text(ack.into())).await;
        }
        let events = state.lock().expect("state").events.clone();
        for event in events {
            let _ = socket.send(Message::Text(event.to_string().into())).await;
        }
        // Stay open until the plugin closes.
        while let Some(Ok(message)) = socket.recv().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
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
        ("POST", ["session"]) => {
            let mut caps = json!({"browserName": "chrome", "browserVersion": "131.0"});
            if st.bidi {
                caps["webSocketUrl"] = json!(st.ws_url);
            }
            reply(json!({"sessionId": "s1", "capabilities": caps}))
        }
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

// ---- Task 6 tests ----

const FOLLOW: u32 = 2;
const PRESS: u32 = 3;
const CLICK_ON: u32 = 4;
const FILL_IN: u32 = 5;
const SELECT: u32 = 6;
const CHECK: u32 = 7;
const UNCHECK: u32 = 8;
const ATTACH: u32 = 9;
const READ_TEXT: u32 = 12;
const READ_ATTR: u32 = 13;
const PAGE_CONTAINS: u32 = 17;
const PAGE_NOT_CONTAINS: u32 = 18;
const ELEMENT_CONTAINS: u32 = 19;
const ELEMENT_NOT_CONTAINS: u32 = 20;
const VISIBLE: u32 = 21;
const NOT_VISIBLE: u32 = 22;
const FIELD_CONTAINS: u32 = 23;
const CHECKED: u32 = 24;
const UNCHECKED: u32 = 25;

fn element(key: &str, text: &str) -> StubElement {
    StubElement {
        key: key.into(),
        text: text.into(),
        displayed: true,
        ..Default::default()
    }
}

#[test]
fn follow_press_and_click_find_by_text_and_click() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![
            element("'New order'", "New order"),
            element("'Pay'", "Pay"),
            element("#cart", ""),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0}));
    let dir = artifacts();
    assert_eq!(
        dispatch(handle, FOLLOW, &["New order"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, PRESS, &["Pay"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, CLICK_ON, &["#cart"], None, dir.path())["status"],
        "passed"
    );
    let calls = calls(&stub);
    assert!(
        calls.iter().any(|c| c.contains("\"using\":\"xpath\"")
            && c.contains("//a[normalize-space(.)='New order'")),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("\"using\":\"css selector\",\"value\":\"#cart\"")),
        "{calls:?}"
    );
    assert_eq!(calls.iter().filter(|c| c.ends_with("/click {}")).count(), 3);
    drop_instance(handle);
}

#[test]
fn a_missing_element_is_fatal_for_an_action_and_names_the_lookup() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({"find_timeout_secs": 0, "on_failure": "none"}));
    let dir = artifacts();
    let r = dispatch(handle, PRESS, &["Nope"], None, dir.path());
    assert_eq!(r["status"], "fatal");
    assert!(
        r["error"]
            .as_str()
            .expect("error")
            .contains("button \"Nope\""),
        "{r}"
    );
    drop_instance(handle);
}

#[test]
fn fill_in_clears_then_types_and_select_picks_the_option() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![
            element("'email'", ""),
            element("'country'", ""),
            element("option[normalize-space(.)='Latvia'", "Latvia"),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0}));
    let dir = artifacts();
    assert_eq!(
        dispatch(
            handle,
            FILL_IN,
            &["email", "ann@example.test"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    assert_eq!(
        stub.state.lock().expect("state").elements[0].value,
        "ann@example.test"
    );
    let calls_before_select = calls(&stub);
    let clear = calls_before_select
        .iter()
        .position(|c| c.ends_with("/e0/clear {}"))
        .expect("clear");
    let typed = calls_before_select
        .iter()
        .position(|c| c.ends_with("/e0/value {\"text\":\"ann@example.test\"}"))
        .expect("value");
    assert!(clear < typed);
    assert_eq!(
        dispatch(handle, SELECT, &["Latvia", "country"], None, dir.path())["status"],
        "passed"
    );
    assert!(
        calls(&stub)
            .iter()
            .any(|c| c.starts_with("POST /session/s1/element/e1/element") && c.contains("option"))
    );
    drop_instance(handle);
}

#[test]
fn check_and_uncheck_click_only_when_the_state_differs() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![element("'newsletter'", "")],
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0}));
    let dir = artifacts();
    assert_eq!(
        dispatch(handle, CHECK, &["newsletter"], None, dir.path())["status"],
        "passed"
    );
    assert!(stub.state.lock().expect("state").elements[0].selected);
    assert_eq!(
        dispatch(handle, CHECK, &["newsletter"], None, dir.path())["status"],
        "passed"
    );
    assert!(
        stub.state.lock().expect("state").elements[0].selected,
        "a second check must not toggle it back"
    );
    assert_eq!(
        dispatch(handle, UNCHECK, &["newsletter"], None, dir.path())["status"],
        "passed"
    );
    assert!(!stub.state.lock().expect("state").elements[0].selected);
    assert_eq!(
        calls(&stub)
            .iter()
            .filter(|c| c.ends_with("/click {}"))
            .count(),
        2
    );
    drop_instance(handle);
}

#[test]
fn attach_sends_the_absolute_path_and_refuses_a_missing_file() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![element("'avatar'", "")],
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0, "on_failure": "none"}));
    let dir = artifacts();
    std::fs::write(dir.path().join("me.png"), b"png").expect("write");
    assert_eq!(
        dispatch(handle, ATTACH, &["me.png", "avatar"], None, dir.path())["status"],
        "passed"
    );
    let expected = dir.path().join("me.png").display().to_string();
    assert!(
        calls(&stub)
            .iter()
            .any(|c| c.ends_with(&format!("/e0/value {{\"text\":\"{expected}\"}}"))),
        "{:?}",
        calls(&stub)
    );
    let missing = dispatch(handle, ATTACH, &["nope.png", "avatar"], None, dir.path());
    assert_eq!(missing["status"], "fatal");
    assert!(
        missing["error"]
            .as_str()
            .expect("error")
            .contains("nope.png")
    );
    drop_instance(handle);
}

#[test]
fn read_text_and_attribute_publish_variables_and_a_missing_attribute_is_fatal() {
    let _guard = serial();
    let mut e = element("[data-test=order-id]", "  ord-42 \n");
    e.attrs.insert("data-id".into(), "42".into());
    let stub = start_stub(StubState {
        elements: vec![e],
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0, "on_failure": "none"}));
    let dir = artifacts();
    let r = dispatch(
        handle,
        READ_TEXT,
        &["[data-test=order-id]", "orderId"],
        None,
        dir.path(),
    );
    assert_eq!(r["status"], "passed");
    assert_eq!(r["vars"]["orderId"], "ord-42");
    let r = dispatch(
        handle,
        READ_ATTR,
        &["data-id", "[data-test=order-id]", "id"],
        None,
        dir.path(),
    );
    assert_eq!(r["vars"]["id"], "42");
    let r = dispatch(
        handle,
        READ_ATTR,
        &["data-nope", "[data-test=order-id]", "id"],
        None,
        dir.path(),
    );
    assert_eq!(r["status"], "fatal");
    assert!(r["error"].as_str().expect("error").contains("data-nope"));
    drop_instance(handle);
}

#[test]
fn page_and_element_text_assertions_collapse_whitespace_and_answer_not_yet() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![
            element("body", "Welcome,\n   ann@example.test  \n Sign out"),
            element("h1", "Welcome,   ann"),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    assert_eq!(
        dispatch(
            handle,
            PAGE_CONTAINS,
            &["Welcome, ann@example.test"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, PAGE_CONTAINS, &["Goodbye"], None, dir.path())["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(handle, PAGE_NOT_CONTAINS, &["Goodbye"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, PAGE_NOT_CONTAINS, &["Sign out"], None, dir.path())["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(
            handle,
            ELEMENT_CONTAINS,
            &["h1", "Welcome, ann"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    assert_eq!(
        dispatch(
            handle,
            ELEMENT_NOT_CONTAINS,
            &["h1", "bob"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    let missing = dispatch(
        handle,
        ELEMENT_NOT_CONTAINS,
        &["h2", "bob"],
        None,
        dir.path(),
    );
    assert_eq!(
        missing["status"], "not_yet",
        "an absent element is expected to appear, for both directions"
    );
    drop_instance(handle);
}

#[test]
fn visibility_field_and_checkbox_assertions() {
    let _guard = serial();
    let mut hidden = element("#hidden", "");
    hidden.displayed = false;
    let mut email = element("'email'", "");
    email.value = "ann@example.test".into();
    let mut news = element("'newsletter'", "");
    news.selected = true;
    let stub = start_stub(StubState {
        elements: vec![element("#shown", ""), hidden, email, news],
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    assert_eq!(
        dispatch(handle, VISIBLE, &["#shown"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, VISIBLE, &["#hidden"], None, dir.path())["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(handle, NOT_VISIBLE, &["#hidden"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, NOT_VISIBLE, &["#absent"], None, dir.path())["status"],
        "passed",
        "not found counts as not visible"
    );
    assert_eq!(
        dispatch(handle, VISIBLE, &["#absent"], None, dir.path())["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(
            handle,
            FIELD_CONTAINS,
            &["email", "ann@example.test"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, FIELD_CONTAINS, &["email", "bob"], None, dir.path())["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(handle, CHECKED, &["newsletter"], None, dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, UNCHECKED, &["newsletter"], None, dir.path())["status"],
        "not_yet"
    );
    drop_instance(handle);
}

#[test]
fn a_null_argument_is_refused_before_anything_is_sent() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    stub.state.lock().expect("state").calls.clear();
    let dir = artifacts();
    let r = dispatch(
        handle,
        FILL_IN,
        &["email", "\u{0}__bddkit_null__\u{0}"],
        None,
        dir.path(),
    );
    assert_eq!(r["status"], "fatal");
    assert!(r["error"].as_str().expect("error").contains("<<null>>"));
    assert!(calls(&stub).is_empty(), "nothing must reach the driver");
    drop_instance(handle);
}

// ---- Task 7 tests ----

const SCRIPT: u32 = 10;
const SCRIPT_DOC: u32 = 11;
const SCREENSHOT: u32 = 14;

#[test]
fn scripts_publish_their_result_as_text() {
    let _guard = serial();
    let stub = start_stub(StubState {
        script_result: json!({"a": 1}),
        ..Default::default()
    });
    let handle = init(&stub, json!({}));
    let dir = artifacts();
    let r = dispatch(handle, SCRIPT, &["return document.title"], None, dir.path());
    assert_eq!(r["vars"]["script_result"], "{\"a\":1}");
    stub.state.lock().expect("state").script_result = Value::Null;
    let r = dispatch(handle, SCRIPT_DOC, &[], Some("return null"), dir.path());
    assert_eq!(r["status"], "passed");
    assert_eq!(r["vars"]["script_result"], "");
    assert!(
        calls(&stub)
            .iter()
            .any(|c| c.contains("\"script\":\"return null\""))
    );
    let r = dispatch(handle, SCRIPT_DOC, &[], None, dir.path());
    assert_eq!(
        r["status"], "fatal",
        "the doc string form without a doc string"
    );
    drop_instance(handle);
}

#[test]
fn i_take_a_screenshot_writes_a_png_into_artifacts_dir() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    let dir = artifacts();
    let target = dir.path().join("000001");
    assert_eq!(
        dispatch(handle, SCREENSHOT, &[], None, &target)["status"],
        "passed"
    );
    let png = std::fs::read(target.join("screenshot.png"))
        .expect("the plugin creates artifacts_dir and writes the PNG");
    assert_eq!(&png[..4], b"\x89PNG");
    drop_instance(handle);
}

#[test]
fn an_action_waits_for_its_element_and_gives_up_after_find_timeout() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![element("'Pay'", "Pay")],
        not_found_first: 3,
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 2, "on_failure": "none"}));
    let dir = artifacts();
    let started = std::time::Instant::now();
    assert_eq!(
        dispatch(handle, PRESS, &["Pay"], None, dir.path())["status"],
        "passed"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(100),
        "three misses mean at least one sleep"
    );
    stub.state.lock().expect("state").not_found_first = 1000;
    let started = std::time::Instant::now();
    let r = dispatch(handle, PRESS, &["Pay"], None, dir.path());
    assert_eq!(r["status"], "fatal");
    assert!(
        r["error"].as_str().expect("error").contains("within 2s"),
        "{r}"
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(2));
    drop_instance(handle);
}

#[test]
fn an_assertion_looks_once_and_never_waits() {
    let _guard = serial();
    let stub = start_stub(StubState {
        elements: vec![element("h1", "x")],
        not_found_first: 1,
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 5, "on_failure": "none"}));
    let dir = artifacts();
    let started = std::time::Instant::now();
    assert_eq!(
        dispatch(handle, VISIBLE, &["h1"], None, dir.path())["status"],
        "not_yet"
    );
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    drop_instance(handle);
}

#[test]
fn a_failure_carries_the_page_a_screenshot_and_the_last_exchange() {
    let _guard = serial();
    let stub = start_stub(StubState {
        url: "http://app.test/orders/new".into(),
        title: "New order".into(),
        ..Default::default()
    });
    let handle = init(&stub, json!({"find_timeout_secs": 0}));
    let dir = artifacts();
    let target = dir.path().join("000007");
    let r = dispatch(handle, PRESS, &["Pay"], None, &target);
    assert_eq!(r["status"], "fatal");
    let d = r["diagnostics"].as_array().expect("diagnostics");
    let titles: Vec<&str> = d
        .iter()
        .map(|x| x["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, vec!["Page", "Screenshot", "WebDriver"], "{r}");
    assert!(
        d[0]["content"]
            .as_str()
            .expect("page")
            .contains("http://app.test/orders/new\nNew order")
    );
    assert!(Path::new(d[1]["path"].as_str().expect("path")).is_file());
    let http = d[2]["content"].as_str().expect("http");
    assert!(
        http.contains("POST session/s1/element") && http.contains("no such element"),
        "the last exchange is the failed find, not the evidence calls: {http}"
    );
    drop_instance(handle);
}

#[test]
fn on_failure_none_writes_nothing_to_disk() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({"find_timeout_secs": 0, "on_failure": "none"}));
    let dir = artifacts();
    let target = dir.path().join("000008");
    let r = dispatch(handle, PRESS, &["Pay"], None, &target);
    assert_eq!(r["status"], "fatal");
    assert!(!target.exists(), "artifacts_dir must not even be created");
    let titles: Vec<&str> = r["diagnostics"]
        .as_array()
        .expect("d")
        .iter()
        .map(|x| x["title"].as_str().expect("t"))
        .collect();
    assert_eq!(titles, vec!["Page", "WebDriver"]);
    drop_instance(handle);
}

#[test]
fn debug_true_then_false_traces_without_failing_the_dispatch() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({}));
    let dir = artifacts();
    let req = |debug: bool| {
        json!({
            "args": ["/login"],
            "docstring": null,
            "table": null,
            "artifacts_dir": dir.path().display().to_string(),
            "workspace_dir": dir.path().display().to_string(),
            "debug": debug,
            "options": {"polling": {"timeout_secs": 1, "interval_ms": 100}},
        })
    };
    let r = take(bddkit_browser::bddkit_dispatch(
        handle,
        AM_ON,
        c(&req(true).to_string()).as_ptr(),
    ));
    assert_eq!(r["status"], "passed");
    let r = take(bddkit_browser::bddkit_dispatch(
        handle,
        AM_ON,
        c(&req(false).to_string()).as_ptr(),
    ));
    assert_eq!(r["status"], "passed");
    drop_instance(handle);
}

// ---- Task 11 tests ----

const DUMP_CONSOLE: u32 = 26;
const DUMP_NETWORK: u32 = 27;
const READ_STATUS: u32 = 28;
const NO_CONSOLE_ERRORS: u32 = 29;
const SENT_REQUEST: u32 = 30;
const LAST_STATUS: u32 = 31;

fn console_event(level: &str, text: &str) -> Value {
    json!({"type": "event", "method": "log.entryAdded", "params": {"type": "console", "level": level, "text": text, "timestamp": 1}})
}

fn request_event(id: &str, method: &str, url: &str) -> Value {
    json!({"type": "event", "method": "network.beforeRequestSent", "params": {"timestamp": 1, "request": {"request": id, "method": method, "url": url, "headers": []}}})
}

fn response_event(id: &str, status: u16) -> Value {
    json!({"type": "event", "method": "network.responseCompleted", "params": {"timestamp": 2, "request": {"request": id}, "response": {"status": status, "headers": []}}})
}

/// Events arrive on another thread; an assertion answers `not_yet` until
/// they have — exactly what the host's polling would do.
fn eventually_passes(handle: u64, index: u32, args: &[&str], dir: &Path) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let r = dispatch(handle, index, args, None, dir);
        if r["status"] == "passed" || std::time::Instant::now() > deadline {
            return r;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn without_bidi_the_console_and_network_steps_fail_naming_the_capability() {
    let _guard = serial();
    let stub = start_stub(StubState::default());
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    for index in [DUMP_CONSOLE, DUMP_NETWORK, NO_CONSOLE_ERRORS] {
        let r = dispatch(handle, index, &[], None, dir.path());
        assert_eq!(r["status"], "fatal", "{index}");
        assert!(
            r["error"].as_str().expect("error").contains("webSocketUrl"),
            "{r}"
        );
    }
    drop_instance(handle);
}

#[test]
fn console_errors_including_uncaught_exceptions_are_fatal_with_the_entries() {
    let _guard = serial();
    let stub = start_stub(StubState {
        bidi: true,
        events: vec![
            console_event("info", "fine"),
            console_event("error", "boom: the widget failed"),
            json!({"type": "event", "method": "log.entryAdded", "params": {"type": "javascript", "level": "error", "text": "Error: uncaught", "timestamp": 3}}),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "console"}));
    let dir = artifacts();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let r = dispatch(handle, NO_CONSOLE_ERRORS, &[], None, dir.path());
    assert_eq!(r["status"], "fatal", "errors never disappear: {r}");
    assert!(
        r["error"].as_str().expect("error").contains("2 error"),
        "{r}"
    );
    let console = r["diagnostics"]
        .as_array()
        .expect("d")
        .iter()
        .find(|d| d["title"] == "Console")
        .expect("console evidence");
    assert!(
        console["content"]
            .as_str()
            .expect("content")
            .contains("boom: the widget failed")
    );
    drop_instance(handle);
}

#[test]
fn network_assertions_match_method_and_path_prefix_and_read_the_status() {
    let _guard = serial();
    let stub = start_stub(StubState {
        bidi: true,
        events: vec![
            request_event("r1", "GET", "http://app.test/orders/new"),
            request_event("r2", "POST", "http://app.test/api/orders?x=1"),
            response_event("r2", 201),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    assert_eq!(
        eventually_passes(handle, SENT_REQUEST, &["POST", "/api/orders"], dir.path())["status"],
        "passed"
    );
    assert_eq!(
        dispatch(
            handle,
            SENT_REQUEST,
            &["DELETE", "/api/orders"],
            None,
            dir.path()
        )["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(handle, SENT_REQUEST, &["GET", "/api"], None, dir.path())["status"],
        "not_yet",
        "prefix on the path, method exact"
    );
    assert_eq!(
        dispatch(
            handle,
            LAST_STATUS,
            &["/api/orders", "201"],
            None,
            dir.path()
        )["status"],
        "passed"
    );
    assert_eq!(
        dispatch(
            handle,
            LAST_STATUS,
            &["/api/orders", "200"],
            None,
            dir.path()
        )["status"],
        "not_yet"
    );
    assert_eq!(
        dispatch(
            handle,
            LAST_STATUS,
            &["/orders/new", "200"],
            None,
            dir.path()
        )["status"],
        "not_yet",
        "no response yet"
    );
    let r = dispatch(
        handle,
        READ_STATUS,
        &["/api/orders", "code"],
        None,
        dir.path(),
    );
    assert_eq!(r["vars"]["code"], "201");
    let none = dispatch(handle, READ_STATUS, &["/nope", "code"], None, dir.path());
    assert_eq!(none["status"], "fatal");
    assert_eq!(
        dispatch(handle, NO_CONSOLE_ERRORS, &[], None, dir.path())["status"],
        "passed"
    );
    drop_instance(handle);
}

#[test]
fn dumps_write_json_files_and_a_reset_empties_the_buffers() {
    let _guard = serial();
    let stub = start_stub(StubState {
        bidi: true,
        events: vec![
            console_event("warn", "w"),
            request_event("r1", "GET", "http://app.test/x"),
        ],
        ..Default::default()
    });
    let handle = init(&stub, json!({"on_failure": "none"}));
    let dir = artifacts();
    assert_eq!(
        eventually_passes(handle, SENT_REQUEST, &["GET", "/x"], dir.path())["status"],
        "passed"
    );
    let target = dir.path().join("000009");
    assert_eq!(
        dispatch(handle, DUMP_CONSOLE, &[], None, &target)["status"],
        "passed"
    );
    assert_eq!(
        dispatch(handle, DUMP_NETWORK, &[], None, &target)["status"],
        "passed"
    );
    let console: Value =
        serde_json::from_slice(&std::fs::read(target.join("console.json")).expect("console.json"))
            .expect("json");
    assert_eq!(console[0]["text"], "w");
    let network: Value =
        serde_json::from_slice(&std::fs::read(target.join("network.json")).expect("network.json"))
            .expect("json");
    assert_eq!(network[0]["url"], "http://app.test/x");
    assert_eq!(reset(handle)["ok"], true);
    assert_eq!(
        dispatch(handle, SENT_REQUEST, &["GET", "/x"], None, dir.path())["status"],
        "not_yet",
        "the reset emptied the network log"
    );
    drop_instance(handle);
}

#[test]
fn a_failure_attaches_console_and_network_when_on_failure_asks() {
    let _guard = serial();
    let stub = start_stub(StubState {
        bidi: true,
        events: vec![
            console_event("error", "e"),
            request_event("r1", "GET", "http://app.test/x"),
        ],
        ..Default::default()
    });
    let handle = init(
        &stub,
        json!({"find_timeout_secs": 0, "on_failure": "console,network"}),
    );
    let dir = artifacts();
    assert_eq!(
        eventually_passes(handle, SENT_REQUEST, &["GET", "/x"], dir.path())["status"],
        "passed"
    );
    let r = dispatch(handle, PRESS, &["Pay"], None, dir.path());
    let titles: Vec<&str> = r["diagnostics"]
        .as_array()
        .expect("d")
        .iter()
        .map(|x| x["title"].as_str().expect("t"))
        .collect();
    assert_eq!(titles, vec!["Page", "Console", "Network", "WebDriver"]);
    drop_instance(handle);
}
