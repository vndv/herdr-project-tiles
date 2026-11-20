//! NDJSON client for the herdr Unix socket (`HERDR_SOCKET_PATH`).
//!
//! Requests use short connections (one request, one response line). Subscriptions keep their own
//! connection open: the first line is the `subscription_started` ack, later lines are events.

use crate::api::{EventEnvelope, SessionSnapshot};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            code: code.into(),
            message: message.into(),
        }
    }
    fn io(e: std::io::Error) -> ApiError {
        ApiError::new("io", e.to_string())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

static SEQ: AtomicU64 = AtomicU64::new(1);

fn next_id(prefix: &str) -> String {
    format!(
        "pt-{prefix}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug, Clone)]
pub struct Client {
    pub path: PathBuf,
    pub timeout: Duration,
}

impl Client {
    pub fn new(path: impl Into<PathBuf>) -> Client {
        Client {
            path: path.into(),
            timeout: Duration::from_secs(10),
        }
    }

    pub fn from_env() -> Option<Client> {
        std::env::var_os("HERDR_SOCKET_PATH")
            .filter(|s| !s.is_empty())
            .map(Client::new)
    }

    fn connect(&self) -> Result<UnixStream, ApiError> {
        UnixStream::connect(&self.path).map_err(ApiError::io)
    }

    /// Sends one request and returns its `result` object.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        let mut s = self.connect()?;
        s.set_read_timeout(Some(self.timeout))
            .map_err(ApiError::io)?;
        s.set_write_timeout(Some(self.timeout))
            .map_err(ApiError::io)?;
        let id = next_id("req");
        let line = json!({ "id": id, "method": method, "params": params }).to_string();
        s.write_all(line.as_bytes()).map_err(ApiError::io)?;
        s.write_all(b"\n").map_err(ApiError::io)?;
        s.flush().map_err(ApiError::io)?;
        let mut r = BufReader::new(s);
        loop {
            let mut buf = String::new();
            let n = r.read_line(&mut buf).map_err(ApiError::io)?;
            if n == 0 {
                return Err(ApiError::new("closed", "herdr closed the connection"));
            }
            let v: Value = match serde_json::from_str(buf.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            return parse_response(v);
        }
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot, ApiError> {
        let r = self.request("session.snapshot", json!({}))?;
        serde_json::from_value(r.get("snapshot").cloned().unwrap_or(Value::Null))
            .map_err(|e| ApiError::new("decode", e.to_string()))
    }

    /// Opens a subscription and waits for its ack.
    pub fn subscribe(&self, subscriptions: Vec<Value>) -> Result<Subscription, ApiError> {
        let mut s = self.connect()?;
        s.set_read_timeout(Some(self.timeout))
            .map_err(ApiError::io)?;
        let id = next_id("sub");
        let line = json!({ "id": id, "method": "events.subscribe",
                            "params": { "subscriptions": subscriptions } })
        .to_string();
        s.write_all(line.as_bytes()).map_err(ApiError::io)?;
        s.write_all(b"\n").map_err(ApiError::io)?;
        let control = s.try_clone().map_err(ApiError::io)?;
        let mut sub = Subscription {
            id,
            reader: BufReader::new(s),
            control,
        };
        loop {
            match sub.read()? {
                Line::Ack => break,
                Line::Event(_) => continue,
            }
        }
        // Events can be far apart: no read timeout after the ack.
        sub.reader
            .get_ref()
            .set_read_timeout(None)
            .map_err(ApiError::io)?;
        Ok(sub)
    }
}

fn parse_response(v: Value) -> Result<Value, ApiError> {
    if let Some(e) = v.get("error") {
        return Err(ApiError::new(
            e.get("code").and_then(Value::as_str).unwrap_or("error"),
            e.get("message").and_then(Value::as_str).unwrap_or(""),
        ));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

enum Line {
    Ack,
    Event(EventEnvelope),
}

pub struct Subscription {
    id: String,
    reader: BufReader<UnixStream>,
    control: UnixStream,
}

impl Subscription {
    fn read(&mut self) -> Result<Line, ApiError> {
        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf).map_err(ApiError::io)?;
            if n == 0 {
                return Err(ApiError::new("closed", "subscription closed"));
            }
            let Ok(v) = serde_json::from_str::<Value>(buf.trim()) else {
                continue;
            };
            if v.get("event").is_some()
                && let Ok(e) = serde_json::from_value::<EventEnvelope>(v.clone())
            {
                return Ok(Line::Event(e));
            }
            let id = v.get("id").and_then(Value::as_str);
            if id.is_some() && id != Some(self.id.as_str()) {
                continue;
            }
            match parse_response(v) {
                Err(e) => return Err(e),
                Ok(r) => {
                    if r.get("type").and_then(Value::as_str) == Some("subscription_started") {
                        return Ok(Line::Ack);
                    }
                }
            }
        }
    }

    /// Next pushed event. `events_lost`, other errors and EOF end the subscription.
    pub fn next_event(&mut self) -> Result<EventEnvelope, ApiError> {
        loop {
            if let Line::Event(e) = self.read()? {
                return Ok(e);
            }
        }
    }

    /// A handle that can close the connection from another thread.
    pub fn closer(&self) -> Closer {
        Closer(self.control.try_clone().ok())
    }
}

pub struct Closer(Option<UnixStream>);

impl Closer {
    pub fn close(&self) {
        if let Some(s) = &self.0 {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}
