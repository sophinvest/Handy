//! Network worker for cloud dictation. Capture only try-sends frames: network
//! and model-free pause scoring never run on the recorder delivery thread. The
//! recorder's independent buffer is authoritative if this bounded queue fills.
use crate::{
    gemini,
    settings::{get_settings, write_settings},
};
use handy_gemini::{
    live::{append_segment, Live},
    pieces::{Cutter, Limiter, FRAME},
};
use serde_json::{json, Value};
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub offset: usize,
    pub text: String,
}
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_specta::Event;
struct Session {
    tx: SyncSender<Vec<f32>>,
    result: Receiver<(Result<String, String>, gemini::RequestGuard)>,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    received: Arc<AtomicUsize>,
    progress: Arc<Mutex<Progress>>,
}
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
pub fn active() -> bool {
    SESSION.lock().map(|s| s.is_some()).unwrap_or(true)
}
static LAST_PREVIEW: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);
pub fn engine(app: &AppHandle, state: &str) {
    log::info!("dictation_engine state={} local_inference=disabled", state);
    let _ = app.emit("transcription-engine", state);
}
fn preview(app: &AppHandle, committed: &str, tentative: &str) {
    if let Ok(mut last) = LAST_PREVIEW.lock() {
        if let Some((c, t, at)) = last.as_ref() {
            if c == committed && t == tentative {
                return;
            }
            if !committed.is_empty() || !tentative.is_empty() {
                if at.elapsed() < Duration::from_millis(100) {
                    return;
                }
            }
        }
        *last = Some((committed.into(), tentative.into(), Instant::now()));
    }
    let _ = crate::managers::transcription::StreamTextEvent {
        committed: committed.into(),
        tentative: tentative.into(),
    }
    .emit(app);
}
pub fn begin(app: &AppHandle) -> Result<crate::audio_toolkit::audio::AudioFrameCallback, String> {
    let mut slot = SESSION.lock().map_err(|_| "Cloud session unavailable")?;
    if slot.is_some() {
        return Err("Finish the current cloud recording first".into());
    }
    let tm = app.state::<Arc<crate::managers::transcription::TranscriptionManager>>();
    if tm.is_model_loaded() {
        tm.unload_model()
            .map_err(|_| "Cannot unload the local model before Google recording")?;
    }
    log::info!(
        "gemini_capture local_model_loaded={} pause_detector=energy_only",
        tm.is_model_loaded()
    );
    let guard = gemini::request_guard()?;
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(2048); // ~65 s; bounded, no capture wait.
    let (reply, result) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let received = Arc::new(AtomicUsize::new(0));
    let wr = received.clone();
    let progress = Arc::new(Mutex::new(Progress::default()));
    let wp = progress.clone();
    let (ws, wl, wc) = (stop.clone(), lost.clone(), cancel.clone());
    let app = app.clone();
    std::thread::Builder::new()
        .name("gemini-cloud".into())
        .spawn(move || {
            let answer = worker(&app, rx, &ws, &wl, &wc, &wr, &wp);
            if answer.is_err() {
                engine(&app, "google_error");
            }
            let _ = reply.send((answer, guard));
        })
        .map_err(|_| "Cannot start cloud worker")?;
    let sender = tx.clone();
    let overflow = lost.clone();
    *slot = Some(Session {
        tx,
        result,
        stop,
        lost,
        cancel,
        received,
        progress,
    });
    Ok(feed_callback(sender, overflow))
}
fn feed_callback(
    sender: SyncSender<Vec<f32>>,
    overflow: Arc<AtomicBool>,
) -> crate::audio_toolkit::audio::AudioFrameCallback {
    Arc::new(move |frame| {
        if matches!(
            sender.try_send(frame.to_vec()),
            Err(mpsc::TrySendError::Full(_))
        ) {
            overflow.store(true, Ordering::Release);
        }
    })
}

pub fn cancel() {
    if let Ok(mut s) = SESSION.lock() {
        if let Some(s) = s.take() {
            s.cancel.store(true, Ordering::Release);
            s.stop.store(true, Ordering::Release);
        }
    }
}
pub fn finish(
    samples: usize,
) -> Option<(
    Result<String, String>,
    Progress,
    Option<gemini::RequestGuard>,
)> {
    let session = SESSION.lock().ok()?.take()?;
    Some(finish_session(session, samples))
}
fn finish_session(
    session: Session,
    samples: usize,
) -> (
    Result<String, String>,
    Progress,
    Option<gemini::RequestGuard>,
) {
    session.stop.store(true, Ordering::Release);
    drop(session.tx);
    // Each network operation is bounded. The worker must not hold an unbounded
    // finalization queue on release. Deferred quota pieces stay saved for retry.
    let (result, guard) = session
        .result
        .recv_timeout(Duration::from_secs(25))
        .map(|(result, guard)| (result, Some(guard)))
        .unwrap_or_else(|_| {
            session.cancel.store(true, Ordering::Release);
            (Err("Cloud finalization timed out".into()), None)
        });
    let progress = session
        .progress
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    if session.lost.load(Ordering::Acquire) {
        return (
            Err("Cloud preview fell behind; complete recording saved for retry".into()),
            progress,
            guard,
        );
    }
    if result.is_ok() && session.received.load(Ordering::Acquire) != samples {
        return (
            Err(
                "Cloud stream sample count did not match the recording. Complete clip saved".into(),
            ),
            progress,
            guard,
        );
    }
    (result, progress, guard)
}
fn load_usage(app: &AppHandle) -> (String, u32) {
    let day = handy_gemini::pieces::pacific_day();
    let v = crate::portable::app_data_dir(app)
        .ok()
        .and_then(|p| std::fs::read(p.join("gemini-usage.json")).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let n = v
        .filter(|v| v["day"] == day)
        .and_then(|v| v["requests"].as_u64())
        .unwrap_or(0) as u32;
    (day, n)
}
pub fn usage(app: &AppHandle) -> u32 {
    load_usage(app).1
}
fn count(app: &AppHandle) -> Result<u32, String> {
    let (day, n) = load_usage(app);
    let n = n.saturating_add(1);
    let p = crate::portable::app_data_dir(app)
        .map_err(|_| "Cannot save request counter")?
        .join("gemini-usage.json");
    let next = p.with_extension("tmp");
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let mut recent = std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["recent"].as_array().cloned())
        .unwrap_or_default();
    recent.retain(|v| v.as_u64().is_some_and(|t| now.saturating_sub(t) < 60_000));
    recent.push(json!(now));
    std::fs::write(
        &next,
        json!({"day":day,"requests":n,"recent":recent}).to_string(),
    )
    .and_then(|_| std::fs::rename(next, p))
    .map_err(|_| "Cannot save request counter")?;
    let _ = app.emit("gemini-clips-changed", ());
    Ok(n)
}
static QUOTA: Mutex<Option<Limiter>> = Mutex::new(None);
fn restored_limiter(app: &AppHandle) -> Limiter {
    let mut limiter = Limiter::default();
    if let Some(v) = crate::portable::app_data_dir(app)
        .ok()
        .and_then(|p| std::fs::read(p.join("gemini-usage.json")).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        if let Some(values) = v["recent"].as_array() {
            for value in values {
                if let Some(t) = value.as_u64() {
                    limiter.record(t);
                }
            }
        }
    }
    limiter
}
/// Applies to all unary calls, including retry and history. Never infer a cap.
pub fn reserve(app: &AppHandle) -> Result<(), String> {
    let settings = get_settings(app);
    let mut state = QUOTA.lock().map_err(|_| "Quota state unavailable")?;
    let limiter = state.get_or_insert_with(|| restored_limiter(app));
    limiter.minute = settings.gemini_rpm;
    limiter.daily = settings.gemini_rpd;
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    if !limiter.allow(now, usage(app)) {
        return Err(
            "Request cap unknown or reached. Configure project limits or retry later".into(),
        );
    }
    count(app)?;
    limiter.record(now);
    Ok(())
}
fn can_send(app: &AppHandle, finalizing: bool) -> bool {
    let s = get_settings(app);
    let Ok(mut state) = QUOTA.lock() else {
        return false;
    };
    let limiter = state.get_or_insert_with(|| restored_limiter(app));
    limiter.minute = s.gemini_rpm;
    limiter.daily = s.gemini_rpd;
    limiter.allow_with_headroom(
        chrono::Utc::now().timestamp_millis().max(0) as u64,
        usage(app),
        if finalizing { 0 } else { 1 },
    )
}
fn worker(
    app: &AppHandle,
    rx: Receiver<Vec<f32>>,
    stop: &AtomicBool,
    lost: &AtomicBool,
    cancel: &AtomicBool,
    received: &AtomicUsize,
    progress: &Mutex<Progress>,
) -> Result<String, String> {
    let settings = get_settings(app);
    let key = gemini::api_key(app)?;
    let model = settings.gemini_model.ok_or("Verify your key in Settings")?;
    let single = settings.gemini_mode == "single";
    engine(
        app,
        if single {
            "google_uploading"
        } else {
            "google_connecting"
        },
    );
    let mut live = if settings.gemini_mode == "auto" {
        match settings.gemini_live_model.as_deref() {
            Some(m) => match Live::connect(&key, m, &settings.custom_words) {
                Ok(l) => {
                    engine(app, "google_live");
                    Some(l)
                }
                Err(e) => {
                    log::warn!("gemini_live fallback=pause_pieces reason={}", e);
                    None
                }
            },
            None => {
                log::warn!("gemini_live fallback=pause_pieces reason=no_verified_live_model");
                None
            }
        }
    } else {
        None
    };
    if live.is_none() && !single {
        engine(app, "google_pieces");
    }

    let mut audio = Vec::new();
    let mut scanned = 0;
    let mut sent = 0;
    let mut cut_base = 0;
    let mut boundary = 0;
    let mut cutter = Cutter::new(settings.gemini_chunk_seconds);
    let mut text = String::new();
    let mut prefix = String::new();
    let mut silence = 0;
    let mut finalizing = false;
    let mut ended = false;
    let api = handy_gemini::Gemini::new().map_err(|e| e.to_string())?;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Cloud recording cancelled; recording remains recoverable".into());
        }
        if lost.load(Ordering::Acquire) {
            return Err(
                "Cloud worker fell behind; complete recording will be saved for retry".into(),
            );
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                audio.extend_from_slice(&frame);
                received.store(audio.len(), Ordering::Release);
                if let Some(l) = live.as_mut() {
                    if let Err(e) = l.audio(&frame).and_then(|_| l.poll()) {
                        if stop.load(Ordering::Acquire) {
                            return Err(format!("{e}. Clip saved; no release-time backfill"));
                        }
                        log::warn!("gemini_live fallback=pause_pieces reason={}", e);
                        engine(app, "google_pieces");
                        // Never paste an incomplete live prefix. Reprocess this
                        // complete recording in pause pieces, in order.
                        live = None;
                        prefix.clear();
                        text.clear();
                        scanned = 0;
                        sent = 0;
                        cut_base = 0;
                        boundary = 0;
                        cutter = Cutter::new(settings.gemini_chunk_seconds);

                        preview(app, "", "");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => ended = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Drain only through recv above. Never consume a probe frame here.
        if ended {
            finalizing = true;
        }
        while scanned + FRAME <= audio.len() {
            let frame = &audio[scanned..scanned + FRAME];
            let probability = if handy_gemini::pieces::is_speech(frame) {
                1.0
            } else {
                0.0
            };
            silence = if probability <= 0.3 {
                silence + FRAME
            } else {
                0
            };
            scanned += FRAME;
            if let Some(n) = cutter.push(frame, probability > 0.3) {
                cut_base += n;
                boundary = cut_base;
            }
            if let Some(l) = live.as_mut() {
                // Reconnect only at a finalized pause, before the 10 min limit.
                // Audio arriving during finalization remains in the channel.
                if (l.since.elapsed() >= Duration::from_secs(540) || l.go_away) && silence >= 6400 {
                    let next = match l.finish() {
                        Ok(t) => {
                            append_segment(&mut prefix, &t);
                            settings
                                .gemini_live_model
                                .as_deref()
                                .and_then(|m| Live::connect(&key, m, &settings.custom_words).ok())
                        }
                        Err(_) => None,
                    };
                    live = next;
                    if live.is_none() {
                        log::warn!("gemini_live fallback=pause_pieces reason=rollover_failed");
                        engine(app, "google_pieces");
                        prefix.clear();
                        text.clear();
                        scanned = 0;
                        sent = 0;
                        cut_base = 0;
                        boundary = 0;
                        cutter = Cutter::new(settings.gemini_chunk_seconds);

                        preview(app, "", "");
                    }
                }
            }
        }
        if let Some(l) = live.as_mut() {
            if let Err(e) = l.poll() {
                if stop.load(Ordering::Acquire) {
                    return Err(format!("{e}. Clip saved; no release-time backfill"));
                }
                log::warn!("gemini_live fallback=pause_pieces reason={}", e);
                engine(app, "google_pieces");
                live = None;
                prefix.clear();
                text.clear();
                scanned = 0;
                sent = 0;
                cut_base = 0;
                boundary = 0;
                cutter = Cutter::new(settings.gemini_chunk_seconds);

                preview(app, "", "");
                continue;
            }
            let mut shown = prefix.clone();
            append_segment(&mut shown, &l.text.committed);
            preview(app, &shown, &l.text.tentative);
            if finalizing {
                match l.finish() {
                    Ok(t) => {
                        append_segment(&mut prefix, &t);
                        return Ok(prefix);
                    }
                    Err(e) => {
                        return Err(format!(
                            "{e}. Live finalization failed; complete recording saved for retry"
                        ))
                    }
                }
            }
        } else if !single {
            if finalizing {
                boundary = audio.len();
            }
            if boundary > sent && can_send(app, finalizing) {
                let wav =
                    handy_gemini::encode_wav(&audio[sent..boundary]).map_err(|e| e.to_string())?;
                engine(app, "google_pieces");
                log::info!(
                    "gemini_piece audio_out samples={} finalizing={}",
                    boundary - sent,
                    finalizing
                );
                reserve(app)?;
                let part = api
                    .transcribe_context(
                        &key,
                        &model,
                        &wav,
                        &settings.custom_words,
                        &handy_gemini::pieces::last_sentence(&text),
                    )
                    .map_err(|e| e.to_string())?;
                append_segment(&mut text, &part);
                sent = boundary;
                if let Ok(mut p) = progress.lock() {
                    p.offset = sent;
                    p.text = text.clone();
                }
                preview(app, &text, "");
            }
            if finalizing {
                return if sent == audio.len() && !text.is_empty() {
                    Ok(text)
                } else {
                    Err("Request cap unknown or reached. Remaining audio saved; configure quotas or retry later".into())
                };
            }
        } else if finalizing {
            reserve(app)?;
            let wav = handy_gemini::encode_wav(&audio).map_err(|e| e.to_string())?;
            return api
                .transcribe(&key, &model, &wav, &settings.custom_words)
                .map_err(|e| e.to_string());
        }
    }
}
fn validate_stream_config(
    mode: &str,
    chunk_seconds: u32,
    rpm: Option<u32>,
    rpd: Option<u32>,
) -> Result<(), String> {
    if !["auto", "chunks", "single"].contains(&mode)
        || ![30, 60, 90].contains(&chunk_seconds)
        || rpm == Some(0)
        || rpd == Some(0)
    {
        return Err("Choose a mode, a 30/60/90-second target, and positive quota limits, or leave limits unknown".into());
    }
    Ok(())
}
#[tauri::command]
#[specta::specta]
pub fn gemini_configure_stream(
    app: AppHandle,
    mode: String,
    chunk_seconds: u32,
    rpm: Option<u32>,
    rpd: Option<u32>,
) -> Result<(), String> {
    let _guard = gemini::idle_guard()?;
    validate_stream_config(&mode, chunk_seconds, rpm, rpd)?;
    let mut s = get_settings(&app);
    s.gemini_mode = mode;
    s.gemini_chunk_seconds = chunk_seconds;
    s.gemini_rpm = rpm;
    s.gemini_rpd = rpd;
    write_settings(&app, s);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_toolkit::{CaptureSink, CaptureVad, VadPolicy};
    use handy_recorder::{AudioChunk, Sink};
    #[test]
    fn stream_config_accepts_only_supported_targets_and_positive_known_caps() {
        for mode in ["auto", "chunks", "single"] {
            for target in [30, 60, 90] {
                assert!(validate_stream_config(mode, target, None, None).is_ok());
                assert!(validate_stream_config(mode, target, Some(1), Some(1)).is_ok());
            }
        }
        for target in [0, 29, 31, 45, 59, 61, 89, 91, u32::MAX] {
            assert!(validate_stream_config("chunks", target, None, None).is_err());
        }
        assert!(validate_stream_config("invalid", 60, None, None).is_err());
        assert!(validate_stream_config("auto", 60, Some(0), None).is_err());
        assert!(validate_stream_config("auto", 60, None, Some(0)).is_err());
    }
    #[test]
    fn session_drains_after_capture_disconnect_and_keeps_request_ownership() {
        let guard = gemini::request_guard().unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let (reply, result) = mpsc::channel();
        let received = Arc::new(AtomicUsize::new(0));
        let count = received.clone();
        let lost = Arc::new(AtomicBool::new(false));
        let callback = feed_callback(tx.clone(), lost.clone());
        callback(&[0.0, 0.25]);
        callback(&[-0.5]);
        drop(callback); // CaptureSink::finish drops the recorder's sender.
        let worker = std::thread::spawn(move || {
            for frame in rx {
                count.fetch_add(frame.len(), Ordering::Release);
            }
            reply.send((Ok("complete".into()), guard)).ok();
        });
        let session = Session {
            tx,
            result,
            stop: Arc::new(AtomicBool::new(false)),
            lost,
            cancel: Arc::new(AtomicBool::new(false)),
            received,
            progress: Arc::new(Mutex::new(Progress::default())),
        };
        let (answer, _, ownership) = finish_session(session, 3);
        assert_eq!(answer.unwrap(), "complete");
        assert!(gemini::request_guard().is_err());
        drop(ownership);
        let guard = gemini::request_guard().unwrap();
        worker.join().unwrap();
        // Cancellation does not unlock an operation still running on a worker.
        let (release, wait) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            drop(guard);
        });
        assert!(gemini::request_guard().is_err());
        release.send(()).unwrap();
        worker.join().unwrap();
        let guard = gemini::request_guard().unwrap();
        let (tx, _) = mpsc::sync_channel(1);
        let (reply, result) = mpsc::channel();
        reply.send((Ok("incomplete".into()), guard)).ok();
        let session = Session {
            tx,
            result,
            stop: Arc::new(AtomicBool::new(false)),
            lost: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            received: Arc::new(AtomicUsize::new(2)),
            progress: Arc::new(Mutex::new(Progress {
                offset: 2,
                text: "prefix".into(),
            })),
        };
        let (answer, progress, _ownership) = finish_session(session, 3);
        assert!(answer.unwrap_err().contains("sample count"));
        assert_eq!(progress.offset, 2);
        assert_eq!(progress.text, "prefix");
    }
    #[test]
    fn stalled_cloud_consumer_does_not_truncate_the_recording() {
        let (tx, _receiver) = mpsc::sync_channel(1);
        let overflow = Arc::new(AtomicBool::new(false));
        let (ready, _) = mpsc::channel();
        let mut sink = CaptureSink::new(
            CaptureVad::cloud(),
            VadPolicy::Cloud,
            Arc::new(|_| panic!("No FFT in cloud mode")),
            feed_callback(tx, overflow.clone()),
            ready,
        );
        let source: Vec<f32> = (0..960_003).map(|i| (i % 100) as f32 / 100.0).collect();
        for frame in source.chunks(512) {
            let mut padded = [0.0; 512];
            padded[..frame.len()].copy_from_slice(frame);
            sink.process_chunk(AudioChunk {
                samples: &padded,
                valid_frames: frame.len(),
                channels: 1,
                sample_rate: 16000,
            });
        }
        assert!(overflow.load(Ordering::Acquire));
        assert_eq!(sink.finish().samples, source);
    }
}
