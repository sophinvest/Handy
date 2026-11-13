// ALGORITHM VERSION 1.6 — 2026-11-13 (Live turn cadence; pause/energy rules live in pieces.rs)
// 1. Recording owns the Google turn (manual activity); send every sample exactly once.
// 2. After at least 3 s of turn audio and 1.5 s of detected quiet, send activityEnd, EXCEPT
//    while Google's running text for the turn has no sentence end yet (no . ? ! at its end):
//    then the speaker is most likely thinking mid-sentence, so wait for 4.5 s of quiet instead.
// 3. Queue all audio (speech and silence) until Google answers the turn, then activityStart
//    and flush in order. "Answers" means ONE of: a final transcript; or Google's
//    voiceActivity ACTIVITY_END followed by 3 s with no transcript (Google heard no speech:
//    an empty turn, not an error). Release shares the same rules.
// 4. Finals therefore arrive phrase by phrase and only the current phrase stays provisional.
// v1.0 — SOP-10742: 30 s minimum / 0.4 s quiet (whole 30 s stayed underlined).
// v1.1 — SOP-10774: 3 s minimum / 0.7 s quiet for Live; pieces mode keeps 30/60/90 s.
// 5. Resume point: when a turn's final arrives, remember how many samples and how much
//    committed text that final covers. If the socket then fails, the caller reconnects,
//    keeps only that text and replays only the audio after those samples (no word is
//    dropped or repeated). Failure classes and retries live in resilience.rs.
// v1.2 — SOP-10790: resume point, specific failure classes, connect/close detail.
// 6. A turn that held no detected speech is never ended: Google sends no final for it, so
//    waiting would time out and Live would drop on any pause over ~3 s.
// v1.3 — SOP-10793: silence-only turns stay open; fallback keeps the overlay text.
// v1.4 — SOP-10793: quiet needed to end a phrase raised 0.7 s -> 1.5 s (mid-sentence thinking pauses).
// 7. Waiting for Google is never a fixed short timer. The wait fails only if Google sends
//    nothing at all for 20 s (any message, partial or not, restarts that clock) or the wait
//    passes 60 s. Sends may block up to 5 s. A slow reply with the socket alive is not a failure.
// 8. Release returns when the last spoken turn has its final and Google reports the
//    generation complete (0.7 s quiet) or stays quiet for 1.5 s; a spoken turn Google
//    closed without any transcript for 3 s counts as empty, so release never fails on it.
// v1.5 — SOP-10811: the 1.5 s final wait (and 0.3 s send limit) removed; Google's empty
//    turns no longer count as timeouts; release settles on generationComplete.
// 9. Joining finals (append_segment): when the text so far does not end a sentence, the next
//    piece continues it: its first letter is lower-cased (never "I", "I'm", acronyms, words
//    like iPhone, months/days, or a name already seen capitalised mid-sentence), and a full
//    stop Google put after a dangling word ("... on the.") is dropped.
// v1.6 — SOP-10847: mid-sentence pauses. 4.5 s quiet while the phrase is unfinished; join
//    rule 9. Measured on Jean's 11-13 log: unfinished-phrase pauses were 1.4-3.8 s.
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
/// Quiet needed to end a phrase whose running text has no sentence end yet (SOP-10847).
/// Jean's mid-sentence thinking pauses measured 1.4-3.8 s; Google already ends a
/// finished sentence with a stop, which keeps the short 1.5 s cut for those.
#[cfg(not(test))]
pub const LIVE_QUIET_OPEN_MS: u32 = 4500;
#[cfg(test)]
pub const LIVE_QUIET_OPEN_MS: u32 = 2500;
/// A final transcript is overdue only when Google has been silent this long. Any
/// server message (partial, final, turnComplete) restarts the clock (SOP-10811).
pub const FINAL_IDLE: Duration = Duration::from_secs(20);
/// Hard ceiling for one wait even while messages keep arriving.
pub const FINAL_CEILING: Duration = Duration::from_secs(60);
/// After the last turn's final, how long the socket must stay quiet before release
/// returns without a turnComplete.
const SETTLE_QUIET: Duration = Duration::from_millis(1500);
/// Quiet needed after `generationComplete`/`turnComplete` before release returns.
const SETTLE_AFTER_DONE: Duration = Duration::from_millis(700);
/// Google answers our activityEnd with `voiceActivity ACTIVITY_END`. If a turn we
/// thought was speech gets no transcript this long after that, Google heard none:
/// the turn is empty, not a failure (measured on Jean's recordings, SOP-10811).
#[cfg(not(test))]
const EMPTY_TURN_GRACE: Duration = Duration::from_secs(3);
#[cfg(test)]
const EMPTY_TURN_GRACE: Duration = Duration::from_millis(300);
/// Longest a single send may block. It was 300 ms, which a brief network stall turned
/// into a "timed out" failure although the connection was fine (SOP-10811).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
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
fn socket_failure(e: tungstenite::Error, op: &str) -> Error {
    match e {
        tungstenite::Error::Io(e) => io_failure(&format!("socket {op}"), &e),
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
fn final_timeout(waited: Duration, idle: Duration) -> Error {
    let message = format!(
        "no transcript from Google for {:.1} s after the phrase ended (silent for {:.1} s)",
        waited.as_secs_f32(),
        idle.as_secs_f32()
    );
    crate::trace::log("timer", || format!("final_wait FIRED {message}"));
    resilience::note(resilience::Detail {
        message: Some(message),
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
/// True when `text` (ignoring trailing spaces and closing quotes/brackets) ends a sentence.
pub fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(|c: char| "\"')]}”’»".contains(c))
        .ends_with(|c: char| ".?!…。！？".contains(c))
}
/// Words a speaker cannot end a sentence on. A stop after one of these is Google guessing.
const DANGLING: [&str; 40] = [
    "and", "or", "but", "the", "a", "an", "to", "of", "in", "on", "at", "for", "with", "by",
    "from", "if", "because", "also", "is", "are", "was", "were", "be", "been", "as", "than",
    "into", "about", "any", "my", "your", "our", "their", "his", "her", "its", "which", "who",
    "when", "while",
];
const ALWAYS_CAPITAL: [&str; 19] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "January",
    "February",
    "March",
    "April",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
    "Google",
];
fn last_word(text: &str) -> &str {
    text.trim_end()
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '\''))
        .next()
        .unwrap_or("")
}
/// The text so far stops on a word no sentence ends on, even if Google added a stop.
fn dangling_stop(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with('.')
        && !t.ends_with("..")
        && DANGLING.contains(&last_word(t).to_lowercase().as_str())
}
/// A capitalised word that already appears capitalised in the middle of `text` is a name.
fn seen_as_name(text: &str, word: &str) -> bool {
    let mut previous: Option<&str> = None;
    for token in text.split_whitespace() {
        if let Some(p) = previous {
            let bare = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
            if bare == word && !ends_sentence(p) {
                return true;
            }
        }
        previous = Some(token);
    }
    false
}
/// `part` continues an unfinished sentence: lower-case its first letter when that
/// letter is only there because Google started a new turn.
fn lower_first(text: &str, part: &str) -> String {
    let start = part.len() - part.trim_start().len();
    let rest = &part[start..];
    let Some(first) = rest.chars().next() else {
        return part.to_owned();
    };
    let word: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '\'')
        .collect();
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    let keep = !first.is_uppercase()
        || word == "I"
        || word.starts_with("I'")
        || letters.iter().skip(1).any(|c| c.is_uppercase())
        || ALWAYS_CAPITAL.contains(&word.as_str())
        || seen_as_name(text, &word);
    let lower: String = first.to_lowercase().collect();
    if keep || lower.chars().count() != 1 {
        return part.to_owned();
    }
    format!("{}{}{}", &part[..start], lower, &rest[first.len_utf8()..])
}
/// Join authoritative utterances/pieces, never speculative partial tokens.
/// Preserve intentional repetitions and punctuation; supply missing whitespace.
/// When the text so far is an unfinished sentence (no stop, or a stop after a word like
/// "the"), the next piece continues it: no capital letter, no stray stop (SOP-10847).
pub fn append_segment(text: &mut String, part: &str) {
    let joined;
    let mut part = part;
    if !text.trim().is_empty() && !part.trim().is_empty() {
        let stray = dangling_stop(text);
        if stray || !ends_sentence(text) {
            if stray {
                let keep = text.trim_end().len() - 1;
                text.truncate(keep);
            }
            joined = lower_first(text, part);
            part = &joined;
        }
    }
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
    /// (finals before activityEnd, when it was sent, samples it covers)
    boundary_wait: Option<(usize, Instant, usize)>,
    last_message: Instant,
    activity_end_at: Option<Instant>,
    /// When Google's `voiceActivity ACTIVITY_END` answered our activityEnd.
    end_acked: Option<Instant>,
    /// `generationComplete` seen since our activityEnd.
    generation_done: bool,
    final_idle: Duration,
    final_ceiling: Duration,
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
        crate::trace::log("ws", || format!("connect start model={model}"));
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
            last_message: Instant::now(),
            activity_end_at: None,
            end_acked: None,
            generation_done: false,
            final_idle: FINAL_IDLE,
            final_ceiling: FINAL_CEILING,
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
                t.set_write_timeout(Some(WRITE_TIMEOUT))
                    .map_err(|_| Error::Offline)?;
                t.set_read_timeout(Some(Duration::from_millis(2)))
            }
            MaybeTlsStream::Rustls(t) => {
                t.sock
                    .set_write_timeout(Some(WRITE_TIMEOUT))
                    .map_err(|_| Error::Offline)?;
                t.sock.set_read_timeout(Some(Duration::from_millis(2)))
            }
            _ => return Err(Error::Unavailable),
        }
        .map_err(|_| Error::Offline)?;
        // Recording, not Google's pause detector, owns this push-to-talk turn.
        // Automatic endpointing truncated the public speech fixture mid-sentence.
        this.send(json!({"realtimeInput":{"activityStart":{}}}))?;
        crate::trace::log("ws", || {
            format!(
                "setup complete, activityStart sent connect_ms={}",
                this.since.elapsed().as_millis()
            )
        });
        log::info!(
            "gemini_live setup_complete remote_manual_activity=true model={} connect_ms={}",
            model,
            this.since.elapsed().as_millis()
        );
        Ok(this)
    }
    /// Tell Google the current turn is over and start timing its answer.
    fn send_end(&mut self, why: &str) -> Result<(), Error> {
        self.send(json!({"realtimeInput":{"activityEnd":{}}}))?;
        self.activity_end_at = Some(Instant::now());
        self.end_acked = None;
        self.generation_done = false;
        self.text.complete = false;
        crate::trace::log("turn", || {
            format!(
                "end sent ({why}) frames={} samples={}",
                self.frames, self.samples
            )
        });
        Ok(())
    }
    fn send(&mut self, value: Value) -> Result<(), Error> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|e| socket_failure(e, "send"))
    }
    pub fn audio(&mut self, samples: &[f32]) -> Result<(), Error> {
        if let Some((_, started, _)) = self.boundary_wait {
            if let Some(e) = self.final_overdue(started) {
                return Err(e);
            }
            // Keep onset and silence alike while the previous turn finalizes.
            // Starting the next turn earlier caused real Google audio loss.
            self.pending_audio.push_back(samples.to_vec());
            return Ok(());
        }
        let speech = crate::pieces::is_speech(samples);
        self.turn_needs_final |= speech;
        // A phrase Google has not punctuated yet is probably a thinking pause, not an
        // ending: give it longer before the turn is cut (SOP-10847).
        let open = !self.text.tentative.trim().is_empty() && !ends_sentence(&self.text.tentative);
        self.pause_cutter.set_quiet_ms(if open {
            LIVE_QUIET_OPEN_MS
        } else {
            LIVE_QUIET_MS
        });
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
            self.send_end("pause boundary")?;
            self.boundary_wait = Some((self.finals, Instant::now(), self.samples));
            log::info!(
                "gemini_live pause_boundary frames={} samples={}",
                self.frames,
                self.samples
            );
        }
        if self.frames == 1 || self.frames.is_multiple_of(50) {
            crate::trace::log("ws_out", || {
                format!(
                    "audio frames={} samples={} bytes_this_frame={} t_ms={}",
                    self.frames,
                    self.samples,
                    samples.len() * 2,
                    self.since.elapsed().as_millis()
                )
            });
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
                self.trace_in(&v, t.len());
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    return Err(server_error(&v));
                }
                self.go_away |= v.get("goAway").is_some();
                self.trace_in(&v, t.len());
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Close(frame)) => {
                let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(1005);
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                crate::trace::log("ws_close", || {
                    format!(
                        "code={code} reason={:?} t_ms={}",
                        reason,
                        self.since.elapsed().as_millis()
                    )
                });
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
            Err(e) => Err(socket_failure(e, "read")),
        }
    }
    /// Detailed-log line for one server message: kinds, size, timing; transcript text only.
    fn trace_in(&mut self, v: &Value, size: usize) {
        self.last_message = Instant::now();
        crate::trace::log("ws_in", || {
            let c = &v["serverContent"];
            let mut kinds: Vec<String> = v
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            if let Some(o) = c.as_object() {
                kinds.extend(o.keys().map(|k| format!("serverContent.{k}")));
            }
            let text = |k: &str| c[k]["text"].as_str().map(|t| format!(" {k}=\"{t}\""));
            format!(
                "bytes={size} kinds={} t_ms={} since_end_ms={}{}{}",
                kinds.join(","),
                self.since.elapsed().as_millis(),
                self.activity_end_at
                    .map(|t| t.elapsed().as_millis().to_string())
                    .unwrap_or_else(|| "-".into()),
                text("interimInputTranscription").unwrap_or_default(),
                text("inputTranscription").unwrap_or_default(),
            )
        });
    }
    fn accept(&mut self, v: &Value) {
        let content = &v["serverContent"];
        if v["voiceActivity"]["type"] == "ACTIVITY_END" && self.activity_end_at.is_some() {
            self.end_acked = Some(Instant::now());
        }
        if content["generationComplete"] == true {
            self.generation_done = true;
        }
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
        if let Some((baseline, started, cut)) = self.boundary_wait {
            let final_in = self.finals > baseline && self.text.tentative.is_empty();
            let empty = self.finals == baseline
                && self.text.tentative.is_empty()
                && self
                    .end_acked
                    .is_some_and(|t| t.elapsed() >= EMPTY_TURN_GRACE);
            if final_in || empty {
                self.boundary_wait = None;
                if empty {
                    self.turn_needs_final = false;
                    log::info!(
                        "gemini_live empty_turn samples={} (no transcript after ACTIVITY_END)",
                        cut
                    );
                }
                crate::trace::log("turn", || {
                    format!(
                        "{} after {} ms; next turn starts",
                        if empty {
                            "Google heard no speech (ACTIVITY_END, no transcript)"
                        } else {
                            "final received"
                        },
                        started.elapsed().as_millis()
                    )
                });
                // Everything up to this cut now has authoritative text.
                self.safe_samples = cut;
                self.safe_text = self.text.committed.len();
                self.send(json!({"realtimeInput":{"activityStart":{}}}))?;
                self.activity_end_at = None;
                crate::trace::log("turn", || "start sent (next turn)".into());
                while let Some(frame) = self.pending_audio.pop_front() {
                    self.audio(&frame)?;
                }
                log::info!(
                    "gemini_live resumed_after_final frames={} samples={}",
                    self.frames,
                    self.samples
                );
            } else if let Some(e) = self.final_overdue(started) {
                return Err(e);
            }
        }
        Ok(())
    }
    /// Some(error) when Google has gone quiet for `final_idle` or the wait passed the ceiling.
    fn final_overdue(&self, started: Instant) -> Option<Error> {
        let waited = started.elapsed();
        let idle = self.last_message.elapsed().min(waited);
        (idle >= self.final_idle || waited >= self.final_ceiling)
            .then(|| final_timeout(waited, idle))
    }
    /// Experiments only: override the wait limits.
    pub fn set_final_wait(&mut self, idle: Duration, ceiling: Duration) {
        self.final_idle = idle;
        self.final_ceiling = ceiling;
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
            self.send_end("release")?;
        }
        log::info!(
            "gemini_live audio_end frames={} samples={}",
            self.frames,
            self.samples
        );
        let mut previous = self.finals;
        loop {
            if let Err(e) = self.poll() {
                // A server may close immediately after its authoritative final.
                // There can be nothing more to drain once that socket closes.
                if matches!(e, Error::Offline | Error::Closed) && self.drained() {
                    log::info!(
                        "gemini_live finalized_on_close release_ms={}",
                        started.elapsed().as_millis()
                    );
                    return self.committed_or_empty();
                }
                return Err(e);
            }
            if !end_sent && self.boundary_wait.is_none() {
                self.send_end("release, after boundary")?;
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
            if end_sent && self.settled() {
                log::info!(
                    "gemini_live finalized release_ms={} frames={} partials={} finals={} turn_complete={}",
                    started.elapsed().as_millis(),
                    self.frames,
                    self.partials,
                    self.finals,
                    self.text.complete
                );
                crate::trace::log("turn", || {
                    format!("release settled after {} ms", started.elapsed().as_millis())
                });
                return self.committed_or_empty();
            }
            if end_sent {
                let from = self.activity_end_at.unwrap_or(started);
                if let Some(e) = self.final_overdue(from) {
                    log::warn!(
                        "gemini_live finalization_timeout release_ms={}; clip preserved, no whole-file upload",
                        started.elapsed().as_millis()
                    );
                    return Err(e);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    /// Nothing left to wait for: every spoken turn has its final text.
    fn drained(&self) -> bool {
        self.finals > 0
            && self.text.tentative.is_empty()
            && self.boundary_wait.is_none()
            && self.pending_audio.is_empty()
            && !self.turn_needs_final
    }
    /// Release can return. Input transcripts have no guaranteed order with
    /// turnComplete, so a spoken turn needs its final; then turnComplete, or a short
    /// quiet spell, shows nothing else is coming. A turnComplete with no final for a
    /// turn our energy gate called speech means Google heard none; wait 3 s, then move on.
    fn settled(&self) -> bool {
        if self.boundary_wait.is_some()
            || !self.pending_audio.is_empty()
            || !self.text.tentative.is_empty()
        {
            return false;
        }
        if self.turn_needs_final {
            // A spoken turn with no final yet: done only once Google has closed the
            // turn (ACTIVITY_END) and stayed silent past the grace period.
            return self
                .end_acked
                .is_some_and(|t| t.elapsed() >= EMPTY_TURN_GRACE);
        }
        // Finals can trail a completion marker (mock evidence), so even then stay open briefly.
        let quiet = self.last_message.elapsed();
        quiet >= SETTLE_QUIET
            || ((self.generation_done || self.text.complete) && quiet >= SETTLE_AFTER_DONE)
    }
    fn committed_or_empty(&self) -> Result<String, Error> {
        if self.text.committed.trim().is_empty() {
            Err(Error::Empty)
        } else {
            Ok(self.text.committed.clone())
        }
    }
}
#[cfg(test)]
mod tests;
