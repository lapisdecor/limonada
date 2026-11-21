//! The transport seam.
//!
//! Everything above this trait is synchronous and therefore directly testable.
//! Two implementations exist: [`HttpTransport`] talks to a live instance, and
//! [`FixtureTransport`] answers from files on disk, which powers both the tests
//! and `LIMONADA_DEMO=1`.

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use serde_json::Value;

use super::error::ApiError;

/// Identifies the client to instances. Lemmy administrators expect a
/// descriptive agent string; anonymous browsing is unaffected by it.
pub const USER_AGENT: &str = concat!(
    "Limonada/",
    env!("CARGO_PKG_VERSION"),
    " (Lemmy desktop client)"
);

/// A synchronous source of Lemmy API responses.
///
/// Implementations must be safe to share across the request worker threads:
/// every method takes `&self` and the trait is `Send + Sync`.
pub trait Transport: Send + Sync {
    fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, ApiError>;
    fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError>;
    /// One-line description of where responses come from, for the status bar.
    fn description(&self) -> String;
    /// True when this transport never touches the network.
    fn is_offline(&self) -> bool {
        false
    }
    /// Install (or clear) the session token used to authenticate requests.
    ///
    /// Default: ignored. Only a transport that can actually authenticate keeps
    /// it, so [`FixtureTransport`] and [`RefusingTransport`] behave exactly as
    /// they did before a session existed.
    fn set_auth(&self, _token: Option<String>) {}
    /// The session token currently installed, if any.
    fn auth(&self) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Live HTTP
// ---------------------------------------------------------------------------

pub struct HttpTransport {
    http: reqwest::blocking::Client,
    base: String,
    /// The session token sent as `Authorization: Bearer <token>`.
    ///
    /// In a `RwLock` rather than a constructor argument because signing in
    /// happens long after the transport is built, and because every request
    /// worker thread reads it while the UI thread may replace it. The token is
    /// never logged.
    token: RwLock<Option<String>>,
}

impl HttpTransport {
    /// `base` is an instance root such as `https://lemmy.ml`, with or without a
    /// trailing slash.
    pub fn new(base: &str) -> Result<Self, ApiError> {
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(ApiError::Refused("No instance URL is configured.".into()));
        }
        let parsed = reqwest::Url::parse(&base)
            .map_err(|e| ApiError::Refused(format!("{base:?} is not a valid URL: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ApiError::Refused(format!(
                "{base:?} must be an http:// or https:// URL."
            )));
        }
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ApiError::Worker(e.to_string()))?;
        Ok(Self {
            http,
            base,
            token: RwLock::new(None),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Attach the session token, when there is one.
    fn authorise(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match self.token.read().expect("token lock").as_deref() {
            Some(token) if !token.is_empty() => request.bearer_auth(token),
            _ => request,
        }
    }
}

impl Transport for HttpTransport {
    fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, ApiError> {
        let response = self
            .authorise(self.http.get(format!("{}{path}", self.base)).query(query))
            .send()
            .map_err(unreachable)?;
        read_response(response)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let response = self
            .authorise(self.http.post(format!("{}{path}", self.base)))
            .json(body)
            .send()
            .map_err(unreachable)?;
        read_response(response)
    }

    fn description(&self) -> String {
        self.base.clone()
    }

    fn set_auth(&self, token: Option<String>) {
        *self.token.write().expect("token lock") = token;
    }

    fn auth(&self) -> Option<String> {
        self.token.read().expect("token lock").clone()
    }
}

/// Map a reqwest failure onto our error type, distinguishing "the instance
/// never answered" from "we could not even build the request".
fn unreachable(err: reqwest::Error) -> ApiError {
    if err.is_connect() || err.is_timeout() || err.is_request() {
        ApiError::Unreachable(err.to_string())
    } else {
        ApiError::Worker(err.to_string())
    }
}

fn read_response(response: reqwest::blocking::Response) -> Result<Value, ApiError> {
    let status = response.status();
    let body = response.text().map_err(unreachable)?;

    // A 401 is the instance's authentication answer, not a generic HTTP
    // failure: lemmy.ml 0.19.20 returns `{"error":"incorrect_login"}` with HTTP
    // 401 for a write with no token or an expired one. Reporting it as its own
    // error lets the UI offer "sign in" instead of "retry".
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized {
            detail: error_detail(&body),
        });
    }

    if !status.is_success() {
        return Err(ApiError::Status {
            status: status.as_u16(),
            body: preview(&body),
        });
    }

    let value: Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        context: format!("reading the HTTP {} body", status.as_u16()),
        detail: e.to_string(),
    })?;

    // Lemmy signals application-level failures with a 200 plus an `error` key.
    if let Some(message) = value
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        return Err(ApiError::Lemmy(message.to_string()));
    }

    Ok(value)
}

/// The `error` string from a Lemmy error envelope, when the body is one, so a
/// message reads as `incorrect_login` rather than as raw JSON.
fn error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
        .unwrap_or_else(|| preview(body))
}

/// Char-boundary-safe truncation for error bodies.
fn preview(body: &str) -> String {
    const MAX: usize = 300;
    if body.len() <= MAX {
        return body.trim().to_string();
    }
    let mut end = MAX;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &body[..end])
}

// ---------------------------------------------------------------------------
// Unusable configuration
// ---------------------------------------------------------------------------

/// A transport that refuses every request with a fixed reason.
///
/// Used when the configured instance URL cannot be turned into a transport. It
/// keeps the error flowing through the ordinary failure path, so the UI needs no
/// `Option<Client>` branches everywhere — it shows the reason like any other
/// request failure.
pub struct RefusingTransport {
    reason: String,
}

impl RefusingTransport {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Transport for RefusingTransport {
    fn get(&self, _path: &str, _query: &[(&str, String)]) -> Result<Value, ApiError> {
        Err(ApiError::Refused(self.reason.clone()))
    }

    fn post(&self, _path: &str, _body: &Value) -> Result<Value, ApiError> {
        Err(ApiError::Refused(self.reason.clone()))
    }

    fn description(&self) -> String {
        format!("unavailable — {}", self.reason)
    }
}

// ---------------------------------------------------------------------------
// Offline fixtures
// ---------------------------------------------------------------------------

/// Answers requests from `fixtures/*.json`.
///
/// This is what makes the interesting claims testable without an account and
/// without network access — comment trees, Markdown bodies, empty threads, and
/// error states all get exercised deterministically.
pub struct FixtureTransport {
    dir: PathBuf,
    latency: Duration,
    /// Substring of a path that should answer with HTTP 503 instead, for
    /// exercising the error state.
    fail_matching: Mutex<Option<String>>,
}

impl FixtureTransport {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            latency: Duration::ZERO,
            fail_matching: Mutex::new(None),
        }
    }

    /// Simulate network latency so loading states are visible in demo mode.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Make any request whose path contains `needle` fail with HTTP 503.
    pub fn failing(&self, needle: &str) {
        *self.fail_matching.lock().expect("fixture lock") = Some(needle.to_string());
    }

    pub fn stop_failing(&self) {
        *self.fail_matching.lock().expect("fixture lock") = None;
    }

    /// The fixture file that answers a given API path.
    fn file_for(path: &str) -> Option<&'static str> {
        match path {
            "/api/v3/site" => Some("site.json"),
            "/api/v3/post/list" => Some("post_list.json"),
            "/api/v3/post" => Some("post_detail.json"),
            "/api/v3/comment/list" => Some("comment_list.json"),
            "/api/v3/community/list" => Some("community_list.json"),
            "/api/v3/search" => Some("search.json"),
            _ => None,
        }
    }
}

impl Transport for FixtureTransport {
    fn get(&self, path: &str, _query: &[(&str, String)]) -> Result<Value, ApiError> {
        if !self.latency.is_zero() {
            std::thread::sleep(self.latency);
        }

        if let Some(needle) = self.fail_matching.lock().expect("fixture lock").as_ref()
            && path.contains(needle.as_str())
        {
            return Err(ApiError::Status {
                status: 503,
                body: "service unavailable (simulated)".into(),
            });
        }

        let Some(file) = Self::file_for(path) else {
            return Err(ApiError::Refused(format!(
                "Demo mode has no fixture for {path}."
            )));
        };
        let full = self.dir.join(file);
        let text = std::fs::read_to_string(&full).map_err(|e| {
            ApiError::Refused(format!("Could not read fixture {}: {e}", full.display()))
        })?;
        serde_json::from_str(&text).map_err(|e| ApiError::Decode {
            context: format!("reading fixture {file}"),
            detail: e.to_string(),
        })
    }

    fn post(&self, path: &str, _body: &Value) -> Result<Value, ApiError> {
        // Deliberate, and still deliberate now that writes exist: demo mode has
        // no instance to authenticate against or write to, so it refuses every
        // POST — sign-in included — rather than pretending to accept one. The
        // read path is separately asserted never to POST at all
        // (`tests/api_shapes.rs`), so reaching here from browsing is a
        // regression too.
        Err(ApiError::Refused(format!(
            "Demo mode is read-only; refusing to POST {path}."
        )))
    }

    fn description(&self) -> String {
        format!("demo fixtures ({})", self.dir.display())
    }

    fn is_offline(&self) -> bool {
        true
    }
}
