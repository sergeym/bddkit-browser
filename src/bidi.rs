//! WebDriver BiDi: the event side of the session. One WebSocket, one reader
//! thread, two buffers a scenario reads and the reset empties.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use url::Url;

const EVENTS: [&str; 3] = [
    "log.entryAdded",
    "network.beforeRequestSent",
    "network.responseCompleted",
];

#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
    /// `console` for `console.*` calls, `javascript` for uncaught exceptions.
    pub source: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEntry {
    pub request_id: String,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub started_at: u64,
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub completed_at: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Buffers {
    pub console: Vec<ConsoleEntry>,
    pub network: Vec<NetworkEntry>,
}

fn headers(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|h| {
                    (
                        h["name"].as_str().unwrap_or("").to_string(),
                        h["value"]["value"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Buffers {
    pub fn clear(&mut self) {
        self.console.clear();
        self.network.clear();
    }

    /// Folds one BiDi message in. Anything that is not one of the three
    /// subscribed events is ignored — command replies included.
    pub fn apply(&mut self, message: &Value) {
        let params = &message["params"];
        match message["method"].as_str() {
            Some("log.entryAdded") => self.console.push(ConsoleEntry {
                level: params["level"].as_str().unwrap_or("").to_string(),
                text: params["text"].as_str().unwrap_or("").to_string(),
                source: params["type"].as_str().unwrap_or("console").to_string(),
                timestamp: params["timestamp"].as_u64().unwrap_or(0),
            }),
            Some("network.beforeRequestSent") => self.network.push(NetworkEntry {
                request_id: params["request"]["request"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                method: params["request"]["method"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                url: params["request"]["url"].as_str().unwrap_or("").to_string(),
                request_headers: headers(&params["request"]["headers"]),
                started_at: params["timestamp"].as_u64().unwrap_or(0),
                status: None,
                response_headers: Vec::new(),
                completed_at: None,
            }),
            Some("network.responseCompleted") => {
                let id = params["request"]["request"].as_str().unwrap_or("");
                if let Some(entry) = self.network.iter_mut().rev().find(|e| e.request_id == id) {
                    entry.status = params["response"]["status"]
                        .as_u64()
                        .and_then(|s| u16::try_from(s).ok());
                    entry.response_headers = headers(&params["response"]["headers"]);
                    entry.completed_at = params["timestamp"].as_u64();
                }
            }
            _ => {}
        }
    }

    pub fn errors(&self) -> impl Iterator<Item = &ConsoleEntry> {
        self.console.iter().filter(|e| e.level == "error")
    }

    /// The most recent request whose URL path starts with `prefix`, any method.
    pub fn last_to(&self, prefix: &str) -> Option<&NetworkEntry> {
        self.network
            .iter()
            .rev()
            .find(|e| path_starts_with(&e.url, prefix))
    }
}

pub fn path_starts_with(url: &str, prefix: &str) -> bool {
    Url::parse(url)
        .map(|u| u.path().starts_with(prefix))
        .unwrap_or(false)
}

pub struct Bidi {
    buffers: Arc<Mutex<Buffers>>,
    stop: Arc<AtomicBool>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

impl Bidi {
    pub fn connect(ws_url: &str, debug: bool) -> Result<Self, String> {
        let (mut socket, _) =
            tungstenite::connect(ws_url).map_err(|e| format!("BiDi socket {ws_url}: {e}"))?;
        socket
            .send(Message::Text(
                json!({"id": 1, "method": "session.subscribe", "params": {"events": EVENTS}})
                    .to_string()
                    .into(),
            ))
            .map_err(|e| format!("BiDi subscribe: {e}"))?;
        // The subscribe reply comes before any event of interest; read it
        // here, synchronously, so the reader thread never has to route
        // command replies.
        loop {
            let message = socket
                .read()
                .map_err(|e| format!("BiDi subscribe reply: {e}"))?;
            let Message::Text(text) = message else {
                continue;
            };
            let v: Value =
                serde_json::from_str(&text).map_err(|e| format!("BiDi reply is not JSON: {e}"))?;
            if v["id"] == 1 {
                if v["type"] != "success" {
                    return Err(format!(
                        "BiDi subscribe refused: {}",
                        v["message"].as_str().unwrap_or(&text)
                    ));
                }
                break;
            }
        }
        if let MaybeTlsStream::Plain(tcp) = socket.get_mut() {
            tcp.set_read_timeout(Some(Duration::from_millis(200)))
                .map_err(|e| format!("BiDi socket timeout: {e}"))?;
        }
        let buffers = Arc::new(Mutex::new(Buffers::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let reader = std::thread::spawn({
            let buffers = Arc::clone(&buffers);
            let stop = Arc::clone(&stop);
            move || read_loop(socket, &buffers, &stop, debug)
        });
        Ok(Self {
            buffers,
            stop,
            reader: Mutex::new(Some(reader)),
        })
    }

    pub fn buffers(&self) -> Arc<Mutex<Buffers>> {
        Arc::clone(&self.buffers)
    }

    /// Stops the reader and closes the socket. Idempotent.
    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.reader.lock().ok().and_then(|mut g| g.take()) {
            let _ = handle.join();
        }
    }
}

impl Drop for Bidi {
    fn drop(&mut self) {
        self.close();
    }
}

fn read_loop(mut socket: Socket, buffers: &Mutex<Buffers>, stop: &AtomicBool, debug: bool) {
    while !stop.load(Ordering::SeqCst) {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if debug {
                    eprintln!("[browser] bidi {text}");
                }
                if let Ok(v) = serde_json::from_str::<Value>(&text)
                    && let Ok(mut b) = buffers.lock()
                {
                    b.apply(&v);
                }
            }
            Ok(Message::Close(_))
            | Err(tungstenite::Error::ConnectionClosed)
            | Err(tungstenite::Error::AlreadyClosed) => return,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Ok(_) => continue,
            Err(e) => {
                if debug {
                    eprintln!("[browser] bidi read: {e}");
                }
                return;
            }
        }
    }
    let _ = socket.close(None);
    // Drain the close handshake, bounded by the read timeout.
    let _ = socket.read();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request_sent(id: &str, method: &str, url: &str, ts: u64) -> Value {
        json!({"type": "event", "method": "network.beforeRequestSent", "params": {
            "timestamp": ts,
            "request": {"request": id, "method": method, "url": url,
                        "headers": [{"name": "accept", "value": {"type": "string", "value": "*/*"}}]}
        }})
    }

    fn response_completed(id: &str, status: u16, ts: u64) -> Value {
        json!({"type": "event", "method": "network.responseCompleted", "params": {
            "timestamp": ts,
            "request": {"request": id},
            "response": {"status": status, "headers": [{"name": "content-type", "value": {"type": "string", "value": "application/json"}}]}
        }})
    }

    #[test]
    fn console_entries_keep_level_text_and_source() {
        let mut b = Buffers::default();
        b.apply(&json!({"type": "event", "method": "log.entryAdded", "params": {"type": "console", "level": "error", "text": "boom", "timestamp": 5}}));
        b.apply(&json!({"type": "event", "method": "log.entryAdded", "params": {"type": "javascript", "level": "error", "text": "Error: uncaught", "timestamp": 6}}));
        b.apply(&json!({"type": "event", "method": "log.entryAdded", "params": {"type": "console", "level": "info", "text": "fine", "timestamp": 7}}));
        assert_eq!(b.console.len(), 3);
        assert_eq!(b.console[1].source, "javascript");
        assert_eq!(b.errors().count(), 2);
    }

    #[test]
    fn a_response_is_joined_onto_its_request_by_id() {
        let mut b = Buffers::default();
        b.apply(&request_sent(
            "r1",
            "POST",
            "http://app.test/api/orders",
            10,
        ));
        b.apply(&request_sent("r2", "GET", "http://app.test/dashboard", 11));
        b.apply(&response_completed("r1", 201, 12));
        assert_eq!(b.network.len(), 2);
        assert_eq!(b.network[0].status, Some(201));
        assert_eq!(
            b.network[0].response_headers[0],
            ("content-type".to_string(), "application/json".to_string())
        );
        assert_eq!(b.network[1].status, None);
        assert_eq!(
            b.last_to("/api/orders").map(|e| e.method.as_str()),
            Some("POST")
        );
        assert!(b.last_to("/nope").is_none());
    }

    #[test]
    fn unknown_events_and_command_replies_are_ignored() {
        let mut b = Buffers::default();
        b.apply(&json!({"id": 1, "type": "success", "result": {}}));
        b.apply(&json!({"type": "event", "method": "browsingContext.load", "params": {}}));
        assert!(b.console.is_empty() && b.network.is_empty());
        b.apply(&request_sent("r1", "GET", "http://a/b", 1));
        b.clear();
        assert!(b.network.is_empty());
    }
}
