//! Dedicated Transcribe Live socket, with bounded network waits and no key in
//! the URL or errors. It never pastes or owns the microphone.
use crate::{Error, TIMEOUT};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
const HOST: &str = "generativelanguage.googleapis.com";
const URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const FINAL_TIMEOUT: Duration = Duration::from_millis(1500);
pub fn valid_model(name: &str) -> bool {
    name.starts_with("models/gemini-")
        && name.contains("transcribe")
        && name.contains("live")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/-._".contains(&c))
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
}
impl Live {
    pub fn connect(key: &str, model: &str, words: &[String]) -> Result<Self, Error> {
        if !valid_model(model) {
            return Err(Error::ModelMissing);
        }
        let addr = (HOST, 443)
            .to_socket_addrs()
            .map_err(|_| Error::Offline)?
            .next()
            .ok_or(Error::Offline)?;
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|_| Error::Offline)?;
        tcp.set_read_timeout(Some(TIMEOUT))
            .map_err(|_| Error::Offline)?;
        tcp.set_write_timeout(Some(TIMEOUT))
            .map_err(|_| Error::Offline)?;
        let mut request = URL.into_client_request().map_err(|_| Error::Unavailable)?;
        request
            .headers_mut()
            .insert("x-goog-api-key", key.parse().map_err(|_| Error::Auth)?);
        let (socket, _) = tungstenite::client_tls_with_config(request, tcp, None, None)
            .map_err(|_| Error::Unavailable)?;
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
        };
        this.send(json!({"setup":{"model":model,"generationConfig":{"responseModalities":["TEXT"]},
            "inputAudioTranscription":{"mode":"VERBATIM","customVocabulary":words.iter().take(1000).collect::<Vec<_>>()}}}))?;
        let until = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() >= until {
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
        log::info!(
            "gemini_live setup_complete model={} connect_ms={}",
            model,
            this.since.elapsed().as_millis()
        );
        Ok(this)
    }
    fn send(&mut self, value: Value) -> Result<(), Error> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|_| Error::Offline)
    }
    pub fn audio(&mut self, samples: &[f32]) -> Result<(), Error> {
        let pcm: Vec<u8> = samples
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
            .collect();
        self.text.complete = false;
        self.send(json!({"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=16000","data":STANDARD.encode(pcm)}}}))?;
        self.frames += 1;
        self.samples += samples.len();
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
                    log::warn!(
                        "gemini_live server_error code={}",
                        v["error"]["code"].as_u64().unwrap_or(0)
                    );
                    return Err(if v["error"]["code"] == 429 {
                        Error::Quota
                    } else {
                        Error::Unavailable
                    });
                }
                self.go_away |= v.get("goAway").is_some();
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    log::warn!(
                        "gemini_live server_error code={}",
                        v["error"]["code"].as_u64().unwrap_or(0)
                    );
                    return Err(if v["error"]["code"] == 429 {
                        Error::Quota
                    } else {
                        Error::Unavailable
                    });
                }
                self.go_away |= v.get("goAway").is_some();
                self.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Close(frame)) => {
                log::warn!(
                    "gemini_live socket_closed code={}",
                    frame.map(|f| u16::from(f.code)).unwrap_or(1005)
                );
                Err(Error::Offline)
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
            Err(_) => Err(Error::Offline),
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
        Ok(())
    }
    pub fn finish(&mut self) -> Result<String, Error> {
        self.text.complete = false;
        self.send(json!({"realtimeInput":{"audioStreamEnd":true}}))?;
        let started = Instant::now();
        log::info!(
            "gemini_live audio_end frames={} samples={}",
            self.frames,
            self.samples
        );
        let until = started + FINAL_TIMEOUT;
        let baseline = self.finals;
        let mut previous = self.finals;
        while Instant::now() < until {
            if let Err(e) = self.poll() {
                // A server may close immediately after its authoritative final.
                // There can be nothing more to drain once that socket closes.
                if e == Error::Offline && self.finals > baseline && self.text.tentative.is_empty() {
                    log::info!(
                        "gemini_live finalized_on_close release_ms={}",
                        started.elapsed().as_millis()
                    );
                    return Ok(self.text.committed.clone());
                }
                return Err(e);
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
        if (self.text.complete || self.finals > baseline) && self.text.tentative.is_empty() {
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
        Err(Error::Timeout)
    }
}
#[cfg(test)]
mod tests;
