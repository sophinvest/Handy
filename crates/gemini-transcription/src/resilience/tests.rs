use super::*;
use serde_json::json;

fn quota_body() -> Value {
    json!({"error":{"code":429,"status":"RESOURCE_EXHAUSTED",
        "message":"You exceeded your current quota. Please retry in 38.327515988s.",
        "details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"38s"}]}})
}
#[test]
fn google_error_reads_status_message_and_retry_delay() {
    let d = google_error(&quota_body());
    assert_eq!(d.http, Some(429));
    assert_eq!(d.status.as_deref(), Some("RESOURCE_EXHAUSTED"));
    assert_eq!(d.retry_after, Some(Duration::from_secs(38)));
    // Without details the hint is read from the message text.
    let d =
        google_error(&json!({"error":{"code":429,"message":"slow down. Please retry in 12.5s."}}));
    assert_eq!(d.retry_after, Some(Duration::from_millis(12_500)));
    assert_eq!(google_error(&json!({})), Detail::default());
}
#[test]
fn http_and_close_failures_map_to_the_documented_classes() {
    assert_eq!(classify(Some(429), None, None), Error::Quota);
    assert_eq!(
        classify(None, Some("RESOURCE_EXHAUSTED"), None),
        Error::Quota
    );
    assert_eq!(classify(Some(401), None, None), Error::Auth);
    assert_eq!(classify(Some(403), None, None), Error::Auth);
    assert_eq!(
        classify(
            Some(400),
            Some("INVALID_ARGUMENT"),
            Some("API key not valid. Please pass a valid API key.")
        ),
        Error::Auth
    );
    assert_eq!(
        classify(Some(400), Some("INVALID_ARGUMENT"), Some("bad audio")),
        Error::Rejected
    );
    assert_eq!(classify(Some(404), None, None), Error::Rejected);
    assert_eq!(
        classify(Some(503), Some("UNAVAILABLE"), None),
        Error::Unavailable
    );
    assert_eq!(classify(Some(500), None, None), Error::Unavailable);
    assert_eq!(classify(Some(504), None, None), Error::Timeout);
    assert_eq!(classify_close(1011, "Internal error"), Error::Closed);
    assert_eq!(classify_close(1000, ""), Error::Closed);
    assert_eq!(classify_close(1008, "API key not valid"), Error::Auth);
    assert_eq!(classify_close(1011, "Quota exceeded"), Error::Quota);
    assert!(retryable(&Error::Closed) && retryable(&Error::Offline));
    assert!(!retryable(&Error::Auth) && !retryable(&Error::Rejected) && !retryable(&Error::Quota));
    assert!(!retryable(&Error::Empty) && !retryable(&Error::InvalidAudio));
}
#[test]
fn transient_failures_retry_twice_with_short_backoff_then_stop() {
    let mut s = RetryState::default();
    let d = Detail::default();
    assert_eq!(
        s.next(&Error::Offline, &d),
        Some(Duration::from_millis(400))
    );
    assert_eq!((s.attempt(), s.max_attempts()), (2, 3));
    assert_eq!(
        s.next(&Error::Closed, &d),
        Some(Duration::from_millis(1200))
    );
    assert_eq!(s.next(&Error::Timeout, &d), None);
}
#[test]
fn quota_retries_once_after_googles_hint_and_only_if_short() {
    let mut s = RetryState::default();
    let d = Detail {
        retry_after: Some(Duration::from_secs(38)),
        ..Default::default()
    };
    assert_eq!(s.next(&Error::Quota, &d), Some(Duration::from_secs(39)));
    assert_eq!(s.next(&Error::Quota, &d), None);
    let long = Detail {
        retry_after: Some(Duration::from_secs(120)),
        ..Default::default()
    };
    assert_eq!(RetryState::default().next(&Error::Quota, &long), None);
    // No hint: a modest default wait.
    assert_eq!(
        RetryState::default().next(&Error::Quota, &Detail::default()),
        Some(Duration::from_secs(16))
    );
}
#[test]
fn never_retry_auth_rejected_or_empty() {
    for e in [
        Error::Auth,
        Error::Rejected,
        Error::Empty,
        Error::InvalidAudio,
        Error::TooLong,
        Error::ModelMissing,
    ] {
        assert_eq!(
            RetryState::default().next(&e, &Detail::default()),
            None,
            "{e:?}"
        );
    }
}
#[test]
fn run_recovers_after_transient_failures_and_reports_each_wait() {
    let mut calls = 0;
    let mut waits = Vec::new();
    let out = run(
        || {
            calls += 1;
            if calls < 3 {
                Err(Error::Offline)
            } else {
                Ok("text")
            }
        },
        |e, _, wait, state| {
            assert_eq!(*e, Error::Offline);
            waits.push((wait, state.attempt()));
            true
        },
    );
    assert_eq!(out, Ok("text"));
    assert_eq!(calls, 3);
    assert_eq!(
        waits,
        vec![
            (Duration::from_millis(400), 2),
            (Duration::from_millis(1200), 3)
        ]
    );
}
#[test]
fn run_gives_up_after_three_attempts_and_honours_cancel() {
    let mut calls = 0;
    let out: Result<(), _> = run(
        || {
            calls += 1;
            Err(Error::Unavailable)
        },
        |_, _, _, _| true,
    );
    assert_eq!((out, calls), (Err(Error::Unavailable), 3));
    let mut calls = 0;
    let out: Result<(), _> = run(
        || {
            calls += 1;
            Err(Error::Offline)
        },
        |_, _, _, _| false,
    );
    assert_eq!((out, calls), (Err(Error::Offline), 1));
    let mut calls = 0;
    let out: Result<(), _> = run(
        || {
            calls += 1;
            Err(Error::Auth)
        },
        |_, _, _, _| panic!("no retry"),
    );
    assert_eq!((out, calls), (Err(Error::Auth), 1));
}
#[test]
fn run_reads_the_quota_hint_noted_by_the_failing_call() {
    let mut calls = 0;
    let mut wait_seen = None;
    let out = run(
        || {
            calls += 1;
            if calls == 1 {
                note(Detail {
                    retry_after: Some(Duration::from_secs(5)),
                    ..Default::default()
                });
                Err(Error::Quota)
            } else {
                Ok(1)
            }
        },
        |_, _, wait, _| {
            wait_seen = Some(wait);
            true
        },
    );
    assert_eq!(out, Ok(1));
    assert_eq!(wait_seen, Some(Duration::from_secs(6)));
}
#[test]
fn sanitize_removes_key_shaped_tokens_and_bounds_length() {
    let s = sanitize("bad key AIzaSyA1234567890abcdefghij and AQ.Ab8RN6K_mAfBXUImrKuY0b41\nnext");
    assert!(!s.contains("AIza") && !s.contains("AQ.Ab8"), "{s}");
    assert!(s.contains("<redacted>") && !s.contains('\n'));
    assert!(sanitize("see https://x/?key=SECRET now").contains("<redacted>"));
    assert!(!sanitize("see https://x/?key=SECRET now").contains("SECRET"));
    assert!(sanitize(&"x ".repeat(500)).chars().count() <= 201);
}
#[test]
fn diagnostics_line_has_every_field_and_no_secret_or_text() {
    let detail = Detail {
        http: Some(503),
        ws_close: Some(1011),
        status: Some("UNAVAILABLE".into()),
        message: Some(sanitize("The model is overloaded.")),
        retry_after: None,
    };
    let l = line(&Diag {
        mode: "live",
        error: &Error::Unavailable,
        detail: &detail,
        session_age_ms: Some(65_000),
        audio_ms: 41_000,
        attempt: 2,
        max_attempts: 3,
        outcome: "retrying",
        wait_ms: Some(400),
    });
    for part in [
        "gemini_diag ts=",
        "outcome=retrying",
        "mode=live",
        "class=server",
        "http=503",
        "ws_close=1011",
        "status=UNAVAILABLE",
        "session_age_ms=65000",
        "audio_ms=41000",
        "attempt=2/3",
        "wait_ms=400",
        "msg=\"The model is overloaded.\"",
    ] {
        assert!(l.contains(part), "missing {part} in {l}");
    }
    assert!(!l.contains('\n'));
    let bare = line(&Diag {
        mode: "pieces",
        error: &Error::Offline,
        detail: &Detail::default(),
        session_age_ms: None,
        audio_ms: 0,
        attempt: 1,
        max_attempts: 3,
        outcome: "failed_clip_saved",
        wait_ms: None,
    });
    assert!(bare
        .contains("http=- ws_close=- status=- session_age_ms=- audio_ms=0 attempt=1/3 wait_ms=-"));
}
#[test]
fn user_text_names_the_failure_and_the_wait() {
    let q = Detail {
        retry_after: Some(Duration::from_secs(38)),
        ..Default::default()
    };
    assert_eq!(
        describe(&Error::Quota, &q),
        "Google: free-tier limit reached. Try again in 38 s"
    );
    assert!(describe(&Error::Offline, &Detail::default()).contains("network lost"));
    assert!(describe(&Error::Auth, &Detail::default()).contains("key rejected"));
    let d = Detail {
        http: Some(503),
        ..Default::default()
    };
    assert_eq!(
        describe(&Error::Unavailable, &d),
        "Google: service unavailable (HTTP 503)"
    );
}
