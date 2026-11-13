// ALGORITHM VERSION 1.0 — 2026-11-13 (Failure classes, retry policy, diagnostics line)
// 1. Every failure is mapped to one class (quota, network, timeout, closed, server,
//    auth, rejected, ...) from the HTTP status, Google status string or websocket close.
// 2. Transient classes (network, timeout, closed, server) retry up to 2 more times
//    after 0.4 s and 1.2 s; 3 attempts in total, counting the first.
// 3. Quota (429 / RESOURCE_EXHAUSTED) retries once, after Google's own "retry in N s"
//    hint plus 1 s, but only if that wait is at most 45 s. Otherwise it fails at once
//    with the wait in the message so the user knows when to try again.
// 4. Auth, rejected, empty and invalid-audio failures never retry; retrying cannot help.
// 5. Each failure or retry writes one `gemini_diag` line: no key, no transcript text,
//    Google's message cleaned of key-like tokens and cut to 200 characters.
// v1.0 — SOP-10790: new; replaces "no automatic retries" in the gemini.8 transport.
//! Failure detail side channel, retry policy and diagnostics line formatting.
use crate::Error;
use serde_json::Value;
use std::{cell::RefCell, time::Duration};

/// Transient failures get this many retries after the first attempt.
pub const TRANSIENT_RETRIES: u32 = 2;
pub const BACKOFF: [Duration; 2] = [Duration::from_millis(400), Duration::from_millis(1200)];
/// A Live session that drops mid-dictation is reconnected for about a minute before
/// anyone gives up on Live (SOP-10811): waits between those attempts.
pub const LIVE_BACKOFF: [Duration; 12] = [
    Duration::from_millis(400),
    Duration::from_millis(1200),
    Duration::from_millis(2500),
    Duration::from_secs(5),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
    Duration::from_secs(8),
];
/// Longest quota wait worth holding a dictation for.
pub const QUOTA_MAX_WAIT: Duration = Duration::from_secs(45);
const QUOTA_MARGIN: Duration = Duration::from_secs(1);
const QUOTA_DEFAULT_WAIT: Duration = Duration::from_secs(15);
const MESSAGE_LIMIT: usize = 200;

/// What the last failing call learned. Kept per thread so the worker that made
/// the call can read it without changing every `Result<_, Error>` signature.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detail {
    pub http: Option<u16>,
    pub ws_close: Option<u16>,
    pub status: Option<String>,
    pub message: Option<String>,
    pub retry_after: Option<Duration>,
}
thread_local! {
    static LAST: RefCell<Detail> = RefCell::new(Detail::default());
}
pub fn note(detail: Detail) {
    // A cleared detail has nothing to say; logging it looked like a failure with every
    // field `None` (SOP-10847).
    if detail != Detail::default() {
        crate::trace::log("failure", || {
            format!(
                "http={:?} ws_close={:?} status={:?} message={:?} retry_after={:?}",
                detail.http, detail.ws_close, detail.status, detail.message, detail.retry_after
            )
        });
    }
    LAST.with(|l| *l.borrow_mut() = detail);
}
pub fn last() -> Detail {
    LAST.with(|l| l.borrow().clone())
}
pub fn clear() {
    note(Detail::default());
}

/// Remove anything key-shaped, flatten whitespace and bound the length.
pub fn sanitize(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let secret = (bare.starts_with("AIza") || bare.starts_with("AQ.")) && bare.len() >= 20;
        let word = if secret || word.contains("key=") {
            "<redacted>"
        } else {
            word
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.chars().count() > MESSAGE_LIMIT {
        out = out.chars().take(MESSAGE_LIMIT).collect::<String>() + "…";
    }
    out
}

fn seconds(text: &str) -> Option<Duration> {
    let number: String = text
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value: f64 = number.parse().ok()?;
    (value.is_finite() && (0.0..=3600.0).contains(&value)).then(|| Duration::from_secs_f64(value))
}

/// Google error bodies: `{"error":{"code","status","message","details":[{"retryDelay":"38s"}]}}`.
/// Also accepts a Live socket error that carries the same `error` object.
pub fn google_error(body: &Value) -> Detail {
    let e = &body["error"];
    let message = e["message"].as_str().unwrap_or("");
    let retry = e["details"]
        .as_array()
        .and_then(|d| {
            d.iter()
                .find_map(|x| x["retryDelay"].as_str().and_then(seconds))
        })
        .or_else(|| {
            message
                .split("retry in ")
                .nth(1)
                .or_else(|| message.split("Retry in ").nth(1))
                .and_then(seconds)
        });
    Detail {
        http: e["code"].as_u64().and_then(|c| u16::try_from(c).ok()),
        ws_close: None,
        status: e["status"].as_str().map(sanitize),
        message: (!message.is_empty()).then(|| sanitize(message)),
        retry_after: retry,
    }
}

/// One mapping for HTTP statuses and Live error objects.
pub fn classify(http: Option<u16>, status: Option<&str>, message: Option<&str>) -> Error {
    let status = status.unwrap_or("");
    let message = message.unwrap_or("").to_ascii_lowercase();
    if http == Some(429) || status == "RESOURCE_EXHAUSTED" {
        Error::Quota
    } else if matches!(http, Some(401 | 403))
        || matches!(status, "UNAUTHENTICATED" | "PERMISSION_DENIED")
        || message.contains("api key")
    {
        Error::Auth
    } else if matches!(http, Some(408 | 504)) || status == "DEADLINE_EXCEEDED" {
        Error::Timeout
    } else if matches!(http, Some(500..=599)) || matches!(status, "UNAVAILABLE" | "INTERNAL") {
        Error::Unavailable
    } else if http.is_some() || status == "INVALID_ARGUMENT" || status == "NOT_FOUND" {
        Error::Rejected
    } else {
        Error::Unavailable
    }
}

/// Websocket close: Google puts the reason in the close text. 1000 is a normal
/// end such as the session-length limit; anything else is an unexpected drop.
pub fn classify_close(code: u16, reason: &str) -> Error {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("quota") || reason.contains("exhausted") || reason.contains("429") {
        Error::Quota
    } else if code == 1008 && (reason.contains("key") || reason.contains("permission")) {
        Error::Auth
    } else if code == 1008 && reason.contains("invalid") {
        Error::Rejected
    } else {
        Error::Closed
    }
}

pub fn retryable(error: &Error) -> bool {
    matches!(
        error,
        Error::Offline | Error::Timeout | Error::Unavailable | Error::Closed
    )
}

/// Tracks what has been spent; one per logical operation.
#[derive(Default, Debug, Clone)]
pub struct RetryState {
    transient: u32,
    quota: bool,
    /// Transient retries allowed; 0 means the default `TRANSIENT_RETRIES`.
    limit: u32,
}
impl RetryState {
    /// The long reconnect policy for a dropped Live session (`LIVE_BACKOFF`).
    pub fn persistent() -> Self {
        Self {
            limit: LIVE_BACKOFF.len() as u32,
            ..Self::default()
        }
    }
    fn limit(&self) -> u32 {
        if self.limit == 0 {
            TRANSIENT_RETRIES
        } else {
            self.limit
        }
    }
    /// Wait before the next attempt, or `None` to give up now.
    pub fn next(&mut self, error: &Error, detail: &Detail) -> Option<Duration> {
        if *error == Error::Quota {
            if self.quota {
                return None;
            }
            let wait = detail.retry_after.unwrap_or(QUOTA_DEFAULT_WAIT) + QUOTA_MARGIN;
            if wait > QUOTA_MAX_WAIT {
                return None;
            }
            self.quota = true;
            return Some(wait);
        }
        if !retryable(error) || self.transient >= self.limit() {
            return None;
        }
        let wait = if self.limit == 0 {
            BACKOFF[self.transient as usize]
        } else {
            LIVE_BACKOFF[self.transient as usize]
        };
        self.transient += 1;
        Some(wait)
    }
    /// Attempts started so far, counting the first (for "2/3" style messages).
    pub fn attempt(&self) -> u32 {
        1 + self.transient + self.quota as u32
    }
    pub fn max_attempts(&self) -> u32 {
        1 + self.limit() + self.quota as u32
    }
}

/// Run `op` until it succeeds or the policy gives up. `before_retry` receives the
/// failure, the wait and the attempt number about to start; it sleeps (checking
/// cancellation) and returns false to stop retrying.
pub fn run<T>(
    mut op: impl FnMut() -> Result<T, Error>,
    mut before_retry: impl FnMut(&Error, &Detail, Duration, &RetryState) -> bool,
) -> Result<T, Error> {
    let mut state = RetryState::default();
    loop {
        clear();
        let error = match op() {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        let detail = last();
        let Some(wait) = state.next(&error, &detail) else {
            return Err(error);
        };
        if !before_retry(&error, &detail, wait, &state) {
            return Err(error);
        }
    }
}

/// Text for the overlay and notification, specific to the failure class.
pub fn describe(error: &Error, detail: &Detail) -> String {
    match error {
        Error::Quota => match detail.retry_after {
            Some(d) => format!(
                "Google: free-tier limit reached. Try again in {} s",
                d.as_secs().max(1)
            ),
            None => "Google: free-tier limit reached. Try again in a minute".into(),
        },
        Error::Offline => "Google: network lost".into(),
        Error::Timeout => "Google: request timed out".into(),
        Error::Closed => "Google: live connection closed".into(),
        Error::Unavailable => match detail.http {
            Some(code) => format!("Google: service unavailable (HTTP {code})"),
            None => "Google: service unavailable".into(),
        },
        Error::Auth => "Google: key rejected. Check the key in Settings".into(),
        Error::Rejected => "Google: request rejected (invalid argument)".into(),
        other => other.to_string(),
    }
}

pub struct Diag<'a> {
    pub mode: &'a str,
    pub error: &'a Error,
    pub detail: &'a Detail,
    pub session_age_ms: Option<u128>,
    pub audio_ms: u64,
    pub attempt: u32,
    pub max_attempts: u32,
    /// `retrying`, `recovered`, `failed_clip_saved`, `failed`, `fallback_pieces`, ...
    pub outcome: &'a str,
    pub wait_ms: Option<u128>,
}
/// One structured line. Never carries the key or any transcript text.
pub fn line(d: &Diag) -> String {
    let dash = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    format!(
        "gemini_diag ts={} outcome={} mode={} class={} http={} ws_close={} status={} session_age_ms={} audio_ms={} attempt={}/{} wait_ms={} msg=\"{}\"",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        d.outcome,
        d.mode,
        d.error.class(),
        dash(d.detail.http.map(|v| v.to_string())),
        dash(d.detail.ws_close.map(|v| v.to_string())),
        d.detail.status.as_deref().unwrap_or("-"),
        dash(d.session_age_ms.map(|v| v.to_string())),
        d.audio_ms,
        d.attempt,
        d.max_attempts,
        dash(d.wait_ms.map(|v| v.to_string())),
        d.detail.message.as_deref().unwrap_or("").replace('"', "'"),
    )
}

#[cfg(test)]
mod tests;
