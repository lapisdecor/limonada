//! Errors produced by the API layer.
//!
//! Every variant carries a message that is safe and useful to show in the UI:
//! the app renders `Display` directly in its error state, so these strings are
//! written for a human, not a log.

use std::fmt;

/// Anything that can go wrong talking to a Lemmy instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The instance could not be reached at all (DNS, connection refused, TLS,
    /// timeout). The string is the underlying transport message.
    Unreachable(String),
    /// The instance answered with a non-success HTTP status.
    Status { status: u16, body: String },
    /// The instance answered 200 with Lemmy's `{"error": "..."}` envelope.
    Lemmy(String),
    /// The instance refused a request because it needs a valid session.
    ///
    /// Verified against a live lemmy.ml 0.19.20: `post/like`, `comment/like`
    /// and `comment` without a token answer HTTP 401 with
    /// `{"error":"incorrect_login"}`, and the same answer comes back once a
    /// saved token has expired. Kept distinct from [`Self::Status`] because it
    /// has one specific remedy — sign in again — which the UI offers directly,
    /// where a bare 401 would only say "the instance returned HTTP 401".
    Unauthorized { detail: String },
    /// A successful response could not be decoded into our types.
    Decode { context: String, detail: String },
    /// The request was refused before it left the process (bad instance URL,
    /// or a write attempted in read-only demo mode).
    Refused(String),
    /// The background thread for the request could not be started.
    Worker(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(msg) => write!(f, "Could not reach the instance: {msg}"),
            Self::Status { status, body } => {
                write!(f, "The instance returned HTTP {status}")?;
                if body.trim().is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {}", body.trim())
                }
            }
            Self::Lemmy(msg) => write!(f, "The instance rejected the request: {msg}"),
            Self::Unauthorized { detail } => {
                write!(f, "This needs a signed-in session")?;
                if detail.trim().is_empty() {
                    Ok(())
                } else {
                    write!(f, " ({})", detail.trim())
                }
            }
            Self::Decode { context, detail } => {
                write!(f, "Unreadable response while {context}: {detail}")
            }
            Self::Refused(msg) => write!(f, "{msg}"),
            Self::Worker(msg) => write!(f, "Could not start a background request: {msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// True when retrying the same request could plausibly succeed, which the
    /// UI uses to decide whether to offer a Retry button.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Unreachable(_) | Self::Worker(_) => true,
            Self::Status { status, .. } => *status >= 500 || *status == 429,
            // Retrying is pointless until the session is replaced, which is a
            // different action from reloading the same request.
            Self::Lemmy(_) | Self::Decode { .. } | Self::Refused(_) | Self::Unauthorized { .. } => {
                false
            }
        }
    }
}
