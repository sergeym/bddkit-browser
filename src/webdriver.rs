//! WebDriver classic over blocking HTTP. Knows endpoints and the wire shape,
//! nothing about Mink or steps; `bddkit-appium` takes this module as is.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use url::Url;

/// The W3C element identifier key.
pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

/// Bodies are cut at this many bytes in evidence: enough to read an error,
/// small enough that a dump stays readable.
pub const MAX_BODY: usize = 4 * 1024;

#[derive(Debug, Clone)]
pub struct Exchange {
    /// `METHOD path`, path relative to the endpoint.
    pub request: String,
    pub request_body: String,
    pub status: u16,
    pub response_body: String,
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_BODY {
        return s.to_string();
    }
    let mut end = MAX_BODY;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… truncated", &s[..end])
}

impl Exchange {
    pub fn render(&self) -> String {
        format!(
            "{}\n{}\n\nstatus: {}\n{}",
            self.request,
            truncate(&self.request_body),
            self.status,
            truncate(&self.response_body)
        )
    }
}

#[derive(Debug)]
pub enum Error {
    /// The request never got a WebDriver reply: refused, timed out, not JSON.
    Transport(String),
    /// The driver answered with an error document.
    Protocol {
        status: u16,
        error: String,
        message: String,
    },
}

impl Error {
    pub fn is_no_such_element(&self) -> bool {
        matches!(self, Self::Protocol { error, .. } if error == "no such element")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) => write!(f, "{m}"),
            Self::Protocol {
                status,
                error,
                message,
            } => write!(f, "{error} ({status}): {message}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Css,
    XPath,
}

impl Strategy {
    fn wire(self) -> &'static str {
        match self {
            Self::Css => "css selector",
            Self::XPath => "xpath",
        }
    }
}

pub struct Driver {
    agent: ureq::Agent,
    base: Url,
    last: Mutex<Option<Exchange>>,
    debug: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub browser_name: String,
    pub browser_version: String,
    /// Unused until the BiDi client (`bidi.rs`, a later task) dials it.
    #[allow(dead_code)]
    pub websocket_url: Option<String>,
}

pub struct Session {
    pub driver: Arc<Driver>,
    pub id: String,
}

pub struct Element<'a> {
    session: &'a Session,
    pub id: String,
}

impl Driver {
    pub fn new(mut base: Url, debug: bool) -> Self {
        // `Url::join` treats the last segment as a file unless the path ends
        // with `/`; a Grid at `/wd/hub` would otherwise lose `hub`.
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let config = ureq::Agent::config_builder()
            // A 404 carries `no such element`; the body is the answer.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        Self {
            agent: config.new_agent(),
            base,
            last: Mutex::new(None),
            debug: AtomicBool::new(debug),
        }
    }

    /// `debug` follows the request, not the init: `steps::route` calls this
    /// on every dispatch before running the step.
    pub fn set_debug(&self, on: bool) {
        self.debug.store(on, Ordering::Relaxed);
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base.join(path).unwrap_or_else(|_| self.base.clone())
    }

    pub fn last_exchange(&self) -> Option<Exchange> {
        self.last.lock().ok().and_then(|g| g.clone())
    }

    fn record(&self, exchange: Exchange) {
        if let Ok(mut g) = self.last.lock() {
            *g = Some(exchange);
        }
    }

    /// Interprets one reply. Separate from the I/O so it is unit-testable.
    fn decode(method: &str, path: &str, status: u16, body: &str) -> Result<Value, Error> {
        let mut parsed: Value = serde_json::from_str(body).map_err(|e| {
            Error::Transport(format!(
                "{method} {path}: status {status} with a non-JSON body: {e}"
            ))
        })?;
        if status != 200 {
            return Err(Error::Protocol {
                status,
                error: parsed["value"]["error"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
                message: parsed["value"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            });
        }
        Ok(parsed["value"].take())
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        let url = self.endpoint(path);
        let request_body = body.map(Value::to_string).unwrap_or_default();
        let sent = match method {
            "GET" => self.agent.get(url.as_str()).call(),
            "DELETE" => self.agent.delete(url.as_str()).call(),
            // WebDriver requires a JSON object body on every POST. Sent as
            // compact bytes rather than `send_json` (which pretty-prints):
            // the recorded `request_body` above must match what actually
            // went over the wire, and evidence dumps stay one line per call.
            _ => {
                let compact = body.cloned().unwrap_or_else(|| json!({})).to_string();
                self.agent
                    .post(url.as_str())
                    .content_type("application/json")
                    .send(compact)
            }
        };
        let mut response = match sent {
            Ok(r) => r,
            Err(e) => {
                let message = format!("{method} {url}: {e}");
                self.record(Exchange {
                    request: format!("{method} {path}"),
                    request_body,
                    status: 0,
                    response_body: message.clone(),
                });
                return Err(Error::Transport(message));
            }
        };
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Transport(format!("{method} {url}: reading the reply: {e}")))?;
        if self.debug.load(Ordering::Relaxed) {
            eprintln!("[browser] {method} /{path} {status}");
        }
        self.record(Exchange {
            request: format!("{method} {path}"),
            request_body,
            status,
            response_body: text.clone(),
        });
        Self::decode(method, path, status, &text)
    }

    /// Unused until managed mode (`Mode::Managed`, a later task) probes the
    /// driver before a session exists.
    #[allow(dead_code)]
    pub fn status(&self) -> Result<Value, Error> {
        self.call("GET", "status", None)
    }

    pub fn new_session(self: &Arc<Self>, body: &Value) -> Result<(Session, SessionInfo), Error> {
        let value = self.call("POST", "session", Some(body))?;
        let id = value["sessionId"]
            .as_str()
            .ok_or_else(|| {
                Error::Transport("POST session: the reply carries no sessionId".to_string())
            })?
            .to_string();
        let caps = &value["capabilities"];
        let info = SessionInfo {
            browser_name: caps["browserName"].as_str().unwrap_or("").to_string(),
            browser_version: caps["browserVersion"].as_str().unwrap_or("").to_string(),
            websocket_url: caps["webSocketUrl"].as_str().map(str::to_string),
        };
        Ok((
            Session {
                driver: Arc::clone(self),
                id,
            },
            info,
        ))
    }
}

fn element_id(value: &Value) -> Result<String, Error> {
    value[ELEMENT_KEY]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Transport(format!("an element reply without {ELEMENT_KEY}: {value}")))
}

impl Session {
    fn path(&self, tail: &str) -> String {
        format!("session/{}/{tail}", self.id)
    }

    fn post(&self, tail: &str, body: Value) -> Result<Value, Error> {
        self.driver.call("POST", &self.path(tail), Some(&body))
    }

    fn get(&self, tail: &str) -> Result<Value, Error> {
        self.driver.call("GET", &self.path(tail), None)
    }

    /// `Ok(None)` on `no such element`; every other error is passed up.
    fn find_under(
        &self,
        tail: &str,
        using: Strategy,
        value: &str,
    ) -> Result<Option<Element<'_>>, Error> {
        match self.post(tail, json!({"using": using.wire(), "value": value})) {
            Ok(v) => Ok(Some(Element {
                session: self,
                id: element_id(&v)?,
            })),
            Err(e) if e.is_no_such_element() => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn navigate(&self, url: &str) -> Result<(), Error> {
        self.post("url", json!({"url": url})).map(drop)
    }

    pub fn current_url(&self) -> Result<String, Error> {
        Ok(self.get("url")?.as_str().unwrap_or("").to_string())
    }

    pub fn title(&self) -> Result<String, Error> {
        Ok(self.get("title")?.as_str().unwrap_or("").to_string())
    }

    pub fn refresh(&self) -> Result<(), Error> {
        self.post("refresh", json!({})).map(drop)
    }

    pub fn set_window_rect(&self, width: u32, height: u32) -> Result<(), Error> {
        self.post("window/rect", json!({"width": width, "height": height}))
            .map(drop)
    }

    pub fn find(&self, using: Strategy, value: &str) -> Result<Option<Element<'_>>, Error> {
        self.find_under("element", using, value)
    }

    pub fn execute(&self, script: &str, args: Vec<Value>) -> Result<Value, Error> {
        self.post("execute/sync", json!({"script": script, "args": args}))
    }

    pub fn screenshot(&self) -> Result<Vec<u8>, Error> {
        let encoded = self.get("screenshot")?;
        let encoded = encoded.as_str().unwrap_or("");
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::Transport(format!("the screenshot is not base64: {e}")))
    }

    pub fn delete_all_cookies(&self) -> Result<(), Error> {
        self.driver
            .call("DELETE", &self.path("cookie"), None)
            .map(drop)
    }

    pub fn delete(&self) -> Result<(), Error> {
        self.driver
            .call("DELETE", &format!("session/{}", self.id), None)
            .map(drop)
    }
}

impl Element<'_> {
    fn tail(&self, what: &str) -> String {
        format!("element/{}/{what}", self.id)
    }

    pub fn click(&self) -> Result<(), Error> {
        self.session.post(&self.tail("click"), json!({})).map(drop)
    }

    pub fn clear(&self) -> Result<(), Error> {
        self.session.post(&self.tail("clear"), json!({})).map(drop)
    }

    pub fn send_keys(&self, text: &str) -> Result<(), Error> {
        self.session
            .post(&self.tail("value"), json!({"text": text}))
            .map(drop)
    }

    pub fn text(&self) -> Result<String, Error> {
        Ok(self
            .session
            .get(&self.tail("text"))?
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    pub fn attribute(&self, name: &str) -> Result<Option<String>, Error> {
        Ok(self
            .session
            .get(&self.tail(&format!("attribute/{name}")))?
            .as_str()
            .map(str::to_string))
    }

    pub fn property(&self, name: &str) -> Result<Value, Error> {
        self.session.get(&self.tail(&format!("property/{name}")))
    }

    pub fn displayed(&self) -> Result<bool, Error> {
        Ok(self
            .session
            .get(&self.tail("displayed"))?
            .as_bool()
            .unwrap_or(false))
    }

    pub fn selected(&self) -> Result<bool, Error> {
        Ok(self
            .session
            .get(&self.tail("selected"))?
            .as_bool()
            .unwrap_or(false))
    }

    pub fn find(&self, using: Strategy, value: &str) -> Result<Option<Element<'_>>, Error> {
        self.session.find_under(&self.tail("element"), using, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_without_a_trailing_slash_still_joins_paths_under_it() {
        let d = Driver::new(Url::parse("http://h:4444/wd/hub").expect("url"), false);
        assert_eq!(d.endpoint("status").as_str(), "http://h:4444/wd/hub/status");
        let d = Driver::new(Url::parse("http://h:4444").expect("url"), false);
        assert_eq!(
            d.endpoint("session/s1/url").as_str(),
            "http://h:4444/session/s1/url"
        );
    }

    #[test]
    fn an_error_body_becomes_a_protocol_error() {
        let body =
            r#"{"value":{"error":"no such element","message":"nothing at //a","stacktrace":""}}"#;
        let e =
            Driver::decode("POST", "session/s1/element", 404, body).expect_err("404 is an error");
        assert!(e.is_no_such_element());
        assert!(e.to_string().contains("nothing at //a"), "{e}");
        let ok = Driver::decode("GET", "session/s1/title", 200, r#"{"value":"Hi"}"#).expect("200");
        assert_eq!(ok, serde_json::json!("Hi"));
        let bad = Driver::decode("GET", "status", 200, "<html>").expect_err("HTML is not a reply");
        assert!(matches!(bad, Error::Transport(_)));
    }

    #[test]
    fn set_debug_toggles_what_call_reads() {
        let d = Driver::new(Url::parse("http://h:4444").expect("url"), false);
        assert!(!d.debug.load(Ordering::Relaxed));
        d.set_debug(true);
        assert!(d.debug.load(Ordering::Relaxed));
    }

    #[test]
    fn an_exchange_renders_truncated_bodies() {
        let ex = Exchange {
            request: "POST session/s1/url".into(),
            request_body: "x".repeat(MAX_BODY + 10),
            status: 200,
            response_body: String::new(),
        };
        let rendered = ex.render();
        assert!(rendered.contains("POST session/s1/url"));
        assert!(rendered.contains("… truncated"));
        assert!(!rendered.contains(&"x".repeat(MAX_BODY + 1)));
    }
}
