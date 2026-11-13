// ALGORITHM VERSION 1.4 — 2026-11-13 (Live turn cadence; pause/energy rules live in pieces.rs)
// 1. Recording owns the Google turn (manual activity); send every sample exactly once.
// 2. After at least 3 s of turn audio and 1.5 s of detected quiet, send activityEnd.
// 3. Queue all audio (speech and silence) until Google's final arrives, then activityStart
//    and flush in order; bounded to 1.5 s, release shares that deadline.
// 4. Finals therefore arrive phrase by phrase and only the current phrase stays provisional.
// v1.0 — SOP-10742: 30 s minimum / 0.4 s quiet (whole 30 s stayed underlined).
// v1.1 — SOP-10774: 3 s minimum / 0.7 s quiet for Live; pieces mode keeps 30/60/90 s.
// 5. Resume point: when a turn's final arrives, remember how many samples and how much
//    committed text that final covers. If the socket then fails, the caller reconnects,
//    keeps only that text and replays only the audio after those samples (no word is
//    dropped or repeated). Failure classes and retries live in resilience.rs.
// v1.2 — SOP-10790: resume point, specific failure classes, connect/close detail.
// 6. A turn that held no detected speech is never ended: Google sends no final for it, so
//    the 1.5 s wait would time out and Live would drop on any pause over ~3 s.
// v1.3 — SOP-10793: silence-only turns stay open; fallback keeps the overlay text.
// v1.4 — SOP-10793: quiet needed to end a phrase raised 0.7 s -> 1.5 s (mid-sentence thinking pauses).
//! Dedicated Transcribe Live socket, with bounded network waits and no key in
//! the URL or errors. It never pastes or owns the microphone.
use crate::{resilience, Error, TIMEOUT};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
const HOST: &str = "generativelanguage.googleapis.com";
const URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
/// Live phrase cadence (SOP-10774): minimum turn length and required quiet.
pub const LIVE_MIN_TURN_MS: u32 = 3000;
/// 1.5 s of quiet: people pause mid-sentence to think, and a shorter cut punctuates
/// each fragment as a sentence (SOP-10793). Unit tests keep the 0.7 s timing their
/// fixed frame counts were written for.
#[cfg(not(test))]
pub const LIVE_QUIET_MS: u32 = 1500;
#[cfg(test)]
pub const LIVE_QUIET_MS: u32 = 700;
const FINAL_TIMEOUT: Duration = Duration::from_millis(1500);
/// Pause between replayed 32 ms frames when resuming a session.
const REPLAY_PACE: Duration = Duration::from_millis(4);
pub fn valid_model(name: &str) -> bool {
    name.starts_with("models/gemini-")
        && name.contains("transcribe")
        && name.contains("live")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/-._".contains(&c))
}
fn offline(message: &str) -> Error {
    resilience::note(resilience::Detail {
        message: Some(message.into()),
        ..Default::default()
    });
    Error::Offline
}
fn io_failure(what: &str, e: &std::io::Error) -> Error {
    if matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        resilience::note(resilience::Detail {
            message: Some(format!("{what}: timed out")),
            ..Default::default()
        });
        return Error::Timeout;
    }
    offline(&format!("{what}: {:?}", e.kind()))
}
fn handshake_failure(e: tungstenite::Error) -> Error {
    match e {
        tungstenite::Error::Http(response) => {
            let status = response.status().as_u16();
            let body = response
                .body()
                .as_deref()
                .and_then(|b| serde_json::from_slice::<Value>(b).ok())
                .map(|b| resilience::google_error(&b))
                .unwrap_or_default();
            let error = resilience::classify(
                Some(status),
                body.status.as_deref(),
                body.message.as_deref(),
            );
            resilience::note(resilience::Detail {
                http: Some(status),
                ..body
            });
            error
        }
        tungstenite::Error::Io(e) => io_failure("websocket handshake failed", &e),
        _ => offline("websocket handshake failed"),
    }
}
/// A Live error object (`{"error":{...}}`) delivered as a socket message.
fn server_error(v: &Value) -> Error {
    let detail = resilience::google_error(v);
    let error = resilience::classify(
        detail.http,
        detail.status.as_deref(),
        detail.message.as_deref(),
    );
    log::warn!(
        "gemini_live server_error code={} status={}",
        detail.http.unwrap_or(0),
        detail.status.as_deref().unwrap_or("-")
    );
    resilience::note(detail);
    error
}
fn socket_failure(e: tungstenite::Error) -> Error {
    match e {
        tungstenite::Error::Io(e) => io_failure("socket error", &e),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            resilience::note(resilience::Detail {
                ws_close: Some(1006),
                message: Some("socket closed without a close frame".into()),
                ..Default::default()
            });
            Error::Closed
        }
        _ => offline("websocket protocol error"),
    }
}
fn final_timeout() -> Error {
    resilience::note(resilience::Detail {
        message: Some("no final transcript within 1.5 s".into()),
        ..Default::default()
    });
    Error::Timeout
}
#[derive(Default, Clone, Debug)]
pub struct Text {
    pub committed: String,
    pub tentative: String,
    pub complete: bool,
}
/// Join authoritative utterances/pieces, never speculative partial tokens.
/// Preserve intentional repetitions and punctuation; only supply missing whitespace.
pub fn append_segment(text: &mut String, part: &str) {
    if !text.is_empty()
        && !part.is_empty()
        && !text.ends_with(char::is_whitespace)
        && !part.starts_with(char::is_whitespace)
        && !part.starts_with(|c: char| ",.;:!?)]}，。！？".contains(c))
    {
        text.push(' ');
    }
    text.push_str(part);
}
impl Text {
    pub fn accept(&mut self, body: &Value) {
        let c = &body["serverContent"];
        if let Some(t) = c["interimInputTranscription"]["text"].as_str() {
            self.tentative = t.to_owned();
        }
        if let Some(t) = c["inputTranscription"]["text"].as_str() {
            append_segment(&mut self.committed, t);
            self.tentative.clear();
        }
        if c["turnComplete"] == true {
            self.complete = true;
        }
    }
}
pub struct Live {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    pub text: Text,
    pub since: Instant,
    pub go_away: bool,
    frames: usize,
    samples: usize,
    partials: usize,
    finals: usize,
    pause_cutter: crate::pieces::Cutter,
    boundary_wait: Option<(usize, Instant, usize)>,
    safe_samples: usize,
    safe_text: usize,
    turn_needs_final: bool,
    pending_audio: VecDeque<Vec<f32>>,
}
impl Live {
    pub fn connect(key: &str, model: &str, words: &[String]) -> Result<Self, Error> {
        if !valid_model(model) {
            return Err(Error::ModelMissing);
        }
        resilience::clear();
        let addr = (HOST, 443)
            .to_socket_addrs()
            .map_err(|e| io_failure("dns lookup failed", &e))?
            .next()
            .ok_or_else(|| offline("dns returned no address"))?;
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)
            .map_err(|e| io_failure("tcp connect failed", &e))?;
        tcp.set_read_timeout(Some(TIMEOUT))
            .map_err(|e| io_failure("socket setup failed", &e))?;
        tcp.set_write_timeout(Some(TIMEOUT))
            .map_err(|e| io_failure("socket setup failed", &e))?;
        let mut request = URL.into_client_request().map_err(|_| Error::Unavailable)?;
        request
            .headers_mut()
            .insert("x-goog-api-key", key.parse().map_err(|_| Error::Auth)?);
        let (socket, _) =
            tungstenite::client_tls_with_config(request, tcp, None, None).map_err(|e| match e {
                tungstenite::HandshakeError::Failure(e) => handshake_failure(e),
                tungstenite::HandshakeError::Interrupted(_) => {
                    offline("websocket handshake interrupted")
                }
            })?;
        Self::setup(socket, model, words)
    }
    fn setup(
        socket: WebSocket<MaybeTlsStream<TcpStream>>,
        model: &str,
        words: &[String],
    ) -> Result<Self, Error> {
        let mut this = Self {
            socket,
            text: Text::default(),
            since: Instant::now(),
            go_away: false,
            frames: 0,
            samples: 0,
            partials: 0,
            finals: 0,
            pause_cutter: crate::pieces::Cutter::with_millis(LIVE_MIN_TURN_MS, LIVE_QUIET_MS),
            boundary_wait: None,
            safe_samples: 0,
            safe_text: 0,
            turn_needs_final: false,
            pending_audio: VecDeque::new(),
        };
        this.send(json!({"setup":{"model":model,"generationConfig":{"responseModalities":["TEXT"]},
            "realtimeInputConfig":{"automaticActivityDetection":{"disabled":true},"activityHandling":"NO_INTERRUPTION"},
            "inputAudioTranscription":{"mode":"VERBATIM","customVocabulary":words.iter().take(1000).collect::<Vec<_>>()}}}))?;
        let until = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() >= until {
                resilience::note(resilience::Detail {
                    message: Some("setupComplete not received within 10 s".into()),
                    ..Default::default()
                });
                return Err(Error::Timeout);
            }
            let Some(body) = this.receive()? else {
                continue;
            };
            if body.get("setupComplete").is_some() {
                break;
            }
        }
        match this.socket.get_mut() {
            MaybeTlsStream::Plain(t) => {
                t.set_write_timeout(Some(Duration::from_millis(300)))
                    .map_err(|_| Error::Offline)?;
                t.set_read_timeout(Some(Duration::from_millis(2)))
            }
            MaybeTlsStream::Rustls(t) => {
                t.sock
                    .set_write_timeout(Some(Duration::from_millis(300)))
                    .map_err(|_| Error::Offline)?;
                t.sock.set_read_timeout(Some(Duration::from_millis(2)))
            }
            _ => return Err(Error::Unavailable),
        }
        .map_err(|_| Error::Offline)?;
        // Recording, not Google's pause detector, owns this push-to-talk turn.
        // Automatic endpointing truncated the public speech fixture mid-sentence.
        this.send(json!({"realtimeInput":{"activityStart":{}}}))?;
        log::info!(
            "gemini_live setup_complete remote_manual_activity=true model={} connect_ms={}",
            model,
            this.since.elapsed().as_millis()
        );
        Ok(this)
    }
    fn send(&mut self, value: Value) -> Result<(), Error> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(socket_failure)
    }
    pub fn audio(&mut self, samples: &[f32]) -> Result<(), Error> {
        if let Some((_, deadline, _)) = self.boundary_wait {
            if Instant::now() >= deadline {
                return Err(final_timeout());
            }
            // Keep onset and silence alike while the previous turn finalizes.
            // Starting the next turn earlier caused real Google audio loss.
            self.pending_audio.push_back(samples.to_vec());
            return Ok(());
        }
        let speech = crate::pieces::is_speech(samples);
        self.turn_needs_final |= speech;
        let pcm: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
            .collect();
        self.text.complete = false;
        self.send(json!({"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=16000","data":STANDARD.encode(pcm)}}}))?;
        self.frames += 1;
        self.samples += samples.len();
        // Reuse the narrated model-free pause algorithm in pieces.rs. A long
        // recording must not become one enormous unfinished server utterance.
        // Every sample is sent exactly once before a boundary at a real pause.
        // A silence-only turn has nothing to finalize: Google sends no final for it,
        // so ending it would time out and drop Live (SOP-10793). Keep it open.
        if self.pause_cutter.push(samples, speech).is_some() && self.turn_needs_final {
            self.send(json!({"realtimeInput":{"activityEnd":{}}}))?;
            self.boundary_wait = Some((self.finals, Instant::now() + FINAL_TIMEOUT, self.samples));
            log::info!(
                "gemini_live pause_boundary frames={} samples={}",
                self.frames,
                self.samples
            );
        }
        if self.frames == 1 || self.frames.is_multiple_of(100) {
            log::info!(
                "gemini_live audio_out frames={} samples={} elapsed_ms={}",
                self.frames,
                self.samples,
                self.since.elapsed().as_millis()
            );
        }
        Ok(())
    }
    fn receive(&mut self) -> Result<Option<Value>, Error> {
        match self.socket.read() {
            Ok(Message::Binary(t)) => {
                let v: Value = serde_json::from_slice(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    return Err(server_error(&v));
                }
                self.go_away |= v.get("goAway").is_some();
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    return Err(server_error(&v));
                }
                self.go_away |= v.get("goAway").is_some();
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Close(frame)) => {
                let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(1005);
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                log::warn!("gemini_live socket_closed code={}", code);
                let error = resilience::classify_close(code, &reason);
                resilience::note(resilience::Detail {
                    ws_close: Some(code),
                    message: (!reason.is_empty()).then(|| resilience::sanitize(&reason)),
                    ..Default::default()
                });
                Err(error)
            }
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(socket_failure(e)),
        }
    }
    fn accept(&mut self, v: &Value) {
        let content = &v["serverContent"];
        if content["interimInputTranscription"]["text"].is_string() {
            self.partials += 1;
            log::info!(
                "gemini_live partial_in count={} audio_frames={} elapsed_ms={}",
                self.partials,
                self.frames,
                self.since.elapsed().as_millis()
            );
        }
        if content["inputTranscription"]["text"].is_string() {
            self.finals += 1;
            self.turn_needs_final = false;
            log::info!(
                "gemini_live final_in count={} audio_frames={} elapsed_ms={}",
                self.finals,
                self.frames,
                self.since.elapsed().as_millis()
            );
        }
        self.text.accept(v);
    }
    pub fn poll(&mut self) -> Result<(), Error> {
        // Bounded even when the server floods events; audio remains buffered.
        for _ in 0..16 {
            if self.receive()?.is_none() {
                break;
            }
        }
        if let Some((baseline, deadline, cut)) = self.boundary_wait {
            if self.finals > baseline && self.text.tentative.is_empty() {
                self.boundary_wait = None;
                // Everything up to this cut now has authoritative text.
                self.safe_samples = cut;
                self.safe_text = self.text.committed.len();
                self.send(json!({"realtimeInput":{"activityStart":{}}}))?;
                while let Some(frame) = self.pending_audio.pop_front() {
                    if Instant::now() >= deadline {
                        return Err(final_timeout());
                    }
                    self.audio(&frame)?;
                }
                log::info!(
                    "gemini_live resumed_after_final frames={} samples={}",
                    self.frames,
                    self.samples
                );
            } else if Instant::now() >= deadline {
                return Err(final_timeout());
            }
        }
        Ok(())
    }
    /// Samples (counted from this connection's start) and committed text that are
    /// final. Audio after `.0` is the only audio a replacement session must replay.
    pub fn resume_point(&self) -> (usize, &str) {
        (self.safe_samples, &self.text.committed[..self.safe_text])
    }
    /// Open a replacement session and replay `tail` (the audio after the old
    /// session's resume point). Fails the same way a live session would.
    pub fn resume(key: &str, model: &str, words: &[String], tail: &[f32]) -> Result<Self, Error> {
        let mut live = Self::connect(key, model, words)?;
        for frame in tail.chunks(crate::pieces::FRAME) {
            live.audio(frame)?;
            live.poll()?;
            // Real Google finals fall behind audio sent with no pacing (measured:
            // an 11 s clip replayed in 1.3 s never finalized in time; at about
            // 4 ms per 32 ms frame, roughly 5x real time, it always did).
            std::thread::sleep(REPLAY_PACE);
        }
        Ok(live)
    }
    pub fn finish(&mut self) -> Result<String, Error> {
        let started = Instant::now();
        self.text.complete = false;
        let mut end_sent = self.boundary_wait.is_none();
        if end_sent {
            self.send(json!({"realtimeInput":{"activityEnd":{}}}))?;
        }
        log::info!(
            "gemini_live audio_end frames={} samples={}",
            self.frames,
            self.samples
        );
        let until = started + FINAL_TIMEOUT;
        let mut previous = self.finals;
        while Instant::now() < until {
            if let Err(e) = self.poll() {
                // A server may close immediately after its authoritative final.
                // There can be nothing more to drain once that socket closes.
                if matches!(e, Error::Offline | Error::Closed)
                    && self.finals > 0
                    && self.text.tentative.is_empty()
                    && self.boundary_wait.is_none()
                    && self.pending_audio.is_empty()
                    && !self.turn_needs_final
                {
                    log::info!(
                        "gemini_live finalized_on_close release_ms={}",
                        started.elapsed().as_millis()
                    );
                    return Ok(self.text.committed.clone());
                }
                return Err(e);
            }
            if !end_sent && self.boundary_wait.is_none() {
                self.send(json!({"realtimeInput":{"activityEnd":{}}}))?;
                end_sent = true;
            }
            if self.finals != previous {
                previous = self.finals;
                log::info!(
                    "gemini_live finalization_received complete={} tentative_chars={}",
                    self.text.complete,
                    self.text.tentative.len()
                );
            }
            // Input transcripts have no guaranteed ordering with turnComplete.
            // A quiet gap (even after one final) cannot prove the last packet
            // arrived. Keep draining the existing bounded window unless the
            // peer ends the socket after authoritative final text.
            std::thread::sleep(Duration::from_millis(1));
        }
        // Transcribe Live may finalize the last utterance before release and
        // send neither another final nor turnComplete after the recording-end signal.
        // After the full drain, retain authoritative text already received;
        // never promote a pending interim or return before delayed finals.
        if end_sent
            && self.finals > 0
            && self.text.tentative.is_empty()
            && self.boundary_wait.is_none()
            && self.pending_audio.is_empty()
            && !self.turn_needs_final
        {
            log::info!(
                "gemini_live finalized_after_drain release_ms={} frames={} partials={} finals={}",
                started.elapsed().as_millis(),
                self.frames,
                self.partials,
                self.finals
            );
            return if self.text.committed.trim().is_empty() {
                Err(Error::Empty)
            } else {
                Ok(self.text.committed.clone())
            };
        }
        log::warn!(
            "gemini_live finalization_timeout release_ms={}; clip preserved, no whole-file upload",
            started.elapsed().as_millis()
        );
        Err(final_timeout())
    }
}
#[cfg(test)]
mod tests;
