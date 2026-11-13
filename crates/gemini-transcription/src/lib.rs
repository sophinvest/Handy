//! Gemini audio transport. No key discovery, model guesses, local inference,
//! redirects, or billing-project changes. Retries follow `resilience.rs` only.
pub mod live;
pub mod pieces;
pub mod resilience;
pub mod secure_file;

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{fmt, io::Read, time::Duration};

pub const API: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_SAMPLES: usize = 16_000 * 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Quota,
    Offline,
    Timeout,
    Auth,
    Unavailable,
    InvalidAudio,
    TooLong,
    Empty,
    ModelMissing,
    /// Google refused the request itself (HTTP 400/404, INVALID_ARGUMENT). Never retried.
    Rejected,
    /// The Live socket ended: session limit, GoAway, or an unexpected drop.
    Closed,
}
impl Error {
    /// Stable lower-case class for the diagnostics line.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Quota => "quota",
            Self::Offline => "network",
            Self::Timeout => "timeout",
            Self::Auth => "auth",
            Self::Unavailable => "server",
            Self::InvalidAudio => "invalid_audio",
            Self::TooLong => "too_long",
            Self::Empty => "empty",
            Self::ModelMissing => "model_missing",
            Self::Rejected => "rejected",
            Self::Closed => "closed",
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Quota => "Google: free-tier limit reached",
            Self::Offline => "Google: network lost",
            Self::Timeout => "Google: request timed out",
            Self::Auth => "Google: key rejected. Check the key in Settings",
            Self::Unavailable => "Google: service unavailable",
            Self::Rejected => "Google: request rejected (invalid argument)",
            Self::Closed => "Google: live connection closed",
            Self::InvalidAudio => "Recording contains invalid audio",
            Self::TooLong => "Each Gemini upload must be five minutes or shorter",
            Self::Empty => "Gemini returned no complete transcript",
            Self::ModelMissing => {
                "No compatible Gemini Transcribe model was returned by ListModels"
            }
        })
    }
}
impl std::error::Error for Error {}
fn network(e: reqwest::Error) -> Error {
    resilience::note(resilience::Detail {
        message: Some(resilience::sanitize(if e.is_timeout() {
            "timeout"
        } else if e.is_connect() {
            "connect failed"
        } else {
            "request failed"
        })),
        ..Default::default()
    });
    if e.is_timeout() {
        Error::Timeout
    } else {
        Error::Offline
    }
}

/// PCM16 little-endian, mono, 16 kHz. No codec thread or lossy compression.
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, Error> {
    if samples.is_empty() || samples.iter().any(|s| !s.is_finite()) {
        return Err(Error::InvalidAudio);
    }
    if samples.len() > MAX_SAMPLES {
        return Err(Error::TooLong);
    }
    let size = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(size + 36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    out.extend_from_slice(&16000u32.to_le_bytes());
    out.extend_from_slice(&32000u32.to_le_bytes());
    out.extend_from_slice(b"\x02\0\x10\0data");
    out.extend_from_slice(&size.to_le_bytes());
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&pcm.to_le_bytes());
    }
    Ok(out)
}

pub struct Gemini {
    client: Client,
    base: String,
}
impl Gemini {
    pub fn new() -> Result<Self, Error> {
        Self::at(API, TIMEOUT)
    }
    // Endpoint override exists only in this private constructor for mock tests.
    fn at(base: &str, timeout: Duration) -> Result<Self, Error> {
        Ok(Self {
            client: Client::builder()
                .timeout(timeout)
                .connect_timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(network)?,
            base: base.to_string(),
        })
    }
    fn response(response: reqwest::blocking::Response) -> Result<Value, Error> {
        let status = response.status().as_u16();
        // The body is read only for Google's status and message, which pass
        // through `resilience::sanitize`; it is never logged raw.
        if !(200..300).contains(&status) {
            let header = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let mut raw = Vec::new();
            let _ = response.take(16_384).read_to_end(&mut raw);
            let mut detail = serde_json::from_slice::<Value>(&raw)
                .map(|b| resilience::google_error(&b))
                .unwrap_or_default();
            detail.http = Some(status);
            detail.retry_after = detail.retry_after.or(header);
            let error = resilience::classify(
                Some(status),
                detail.status.as_deref(),
                detail.message.as_deref(),
            );
            resilience::note(detail);
            return Err(error);
        }
        let mut bytes = Vec::new();
        response
            .take(2_000_001)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Offline)?;
        if bytes.len() > 2_000_000 {
            return Err(Error::Unavailable);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)
    }
    /// Run only on explicit key save / refresh, never on every dictation.
    pub fn models(&self, key: &str) -> Result<Vec<String>, Error> {
        self.discover(key, false)
    }
    pub fn live_models(&self, key: &str) -> Result<Vec<String>, Error> {
        self.discover(key, true)
    }
    fn discover(&self, key: &str, live: bool) -> Result<Vec<String>, Error> {
        let mut token = String::new();
        let mut found = Vec::new();
        for _ in 0..10 {
            let response = self
                .client
                .get(format!("{}/models", self.base))
                .header("x-goog-api-key", key)
                .query(&[("pageSize", "1000"), ("pageToken", token.as_str())])
                .send()
                .map_err(network)?;
            let body = Self::response(response)?;
            if let Some(models) = body["models"].as_array() {
                for model in models {
                    if let Some(name) = model["name"].as_str() {
                        let compatible = model["supportedGenerationMethods"]
                            .as_array()
                            .is_some_and(|m| {
                                m.iter().any(|v| {
                                    v == if live {
                                        "bidiGenerateContent"
                                    } else {
                                        "generateContent"
                                    }
                                })
                            });
                        if compatible
                            && if live {
                                live::valid_model(name)
                            } else {
                                transcription_model(name)
                            }
                        {
                            found.push(name.to_string());
                        }
                    }
                }
            }
            token = body["nextPageToken"].as_str().unwrap_or("").to_string();
            if token.is_empty() {
                found.sort();
                found.dedup();
                return if found.is_empty() {
                    Err(Error::ModelMissing)
                } else {
                    Ok(found)
                };
            }
        }
        Err(Error::Unavailable)
    }
    pub fn transcribe(
        &self,
        key: &str,
        model: &str,
        wav: &[u8],
        words: &[String],
    ) -> Result<String, Error> {
        self.transcribe_context(key, model, wav, words, "")
    }
    pub fn transcribe_context(
        &self,
        key: &str,
        model: &str,
        wav: &[u8],
        words: &[String],
        previous: &str,
    ) -> Result<String, Error> {
        if !transcription_model(model) {
            return Err(Error::ModelMissing);
        }
        let body = json!({
            "contents": [{"role": "user", "parts": [
                {"inlineData": {"mimeType": "audio/wav", "data": STANDARD.encode(wav)}},
                {"text": format!("Transcribe only this audio verbatim. Previous sentence for context only; do not repeat it: {}", previous)}
            ]}],
            "generationConfig": {"audioTranscriptionConfig": {
                "mode": "VERBATIM",
                "customVocabulary": words.iter().filter(|s| !s.trim().is_empty()).take(1000).collect::<Vec<_>>()
            }}
        });
        let response = self
            .client
            .post(format!("{}/{}:generateContent", self.base, model))
            .header("x-goog-api-key", key)
            .json(&body)
            .send()
            .map_err(network)?;
        transcript(&Self::response(response)?)
    }
}
fn transcription_model(name: &str) -> bool {
    name.starts_with("models/gemini-")
        && name.contains("transcribe")
        && !name.contains("live")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/-._".contains(&c))
}
fn transcript(body: &Value) -> Result<String, Error> {
    let candidate = &body["candidates"][0];
    // A token-limited partial transcript must not look like a complete dictation.
    if candidate["finishReason"].as_str() != Some("STOP") {
        resilience::note(resilience::Detail {
            status: Some(resilience::sanitize(
                candidate["finishReason"].as_str().unwrap_or("NO_CANDIDATE"),
            )),
            ..Default::default()
        });
        return Err(Error::Empty);
    }
    let parts = candidate["content"]["parts"]
        .as_array()
        .ok_or(Error::Empty)?;
    let text: String = parts
        .iter()
        .filter(|p| p["thought"] != true)
        .filter_map(|p| {
            p["audioTranscription"]["text"]
                .as_str()
                .or_else(|| p["text"].as_str())
        })
        .collect();
    if text.trim().is_empty() {
        Err(Error::Empty)
    } else {
        Ok(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests;
