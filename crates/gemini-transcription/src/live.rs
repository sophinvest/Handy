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
impl Text {
    pub fn accept(&mut self, body: &Value) {
        let c = &body["serverContent"];
        if let Some(t) = c["interimInputTranscription"]["text"].as_str() {
            self.tentative = t.to_owned();
        }
        if let Some(t) = c["inputTranscription"]["text"].as_str() {
            self.committed.push_str(t);
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
        let mut this = Self {
            socket,
            text: Text::default(),
            since: Instant::now(),
            go_away: false,
        };
        this.send(json!({"setup":{"model":model,"generationConfig":{"responseModalities":["TEXT"]},
            "inputAudioTranscription":{"mode":"VERBATIM","customVocabulary":words.iter().take(1000).collect::<Vec<_>>()}}}))?;
        let until = Instant::now() + TIMEOUT;
        loop {
            if Instant::now() >= until {
                return Err(Error::Timeout);
            }
            let body = this.receive()?.ok_or(Error::Unavailable)?;
            if body.get("setupComplete").is_some() {
                break;
            }
        }
        match this.socket.get_mut() {
            MaybeTlsStream::Plain(t) => t.set_read_timeout(Some(Duration::from_millis(2))),
            MaybeTlsStream::Rustls(t) => t.sock.set_read_timeout(Some(Duration::from_millis(2))),
            _ => return Err(Error::Unavailable),
        }
        .map_err(|_| Error::Offline)?;
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
        self.send(json!({"realtimeInput":{"audio":{"mimeType":"audio/pcm;rate=16000","data":STANDARD.encode(pcm)}}}))
    }
    fn receive(&mut self) -> Result<Option<Value>, Error> {
        match self.socket.read() {
            Ok(Message::Binary(t)) => {
                let v: Value = serde_json::from_slice(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    return Err(if v["error"]["code"] == 429 {
                        Error::Quota
                    } else {
                        Error::Unavailable
                    });
                }
                self.go_away |= v.get("goAway").is_some();
                self.text.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).map_err(|_| Error::Unavailable)?;
                if v.get("error").is_some() {
                    return Err(if v["error"]["code"] == 429 {
                        Error::Quota
                    } else {
                        Error::Unavailable
                    });
                }
                self.go_away |= v.get("goAway").is_some();
                self.text.accept(&v);
                Ok(Some(v))
            }
            Ok(Message::Close(_)) => Err(Error::Offline),
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
        let until = Instant::now() + TIMEOUT;
        while Instant::now() < until {
            self.poll()?;
            if self.text.complete && self.text.tentative.is_empty() {
                return if self.text.committed.trim().is_empty() {
                    Err(Error::Empty)
                } else {
                    Ok(self.text.committed.clone())
                };
            }
        }
        Err(Error::Timeout)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interim_replaces_and_final_appends_without_pasting_partial() {
        let mut t = Text::default();
        t.accept(&json!({"serverContent":{"interimInputTranscription":{"text":"water"}}}));
        t.accept(&json!({"serverContent":{"interimInputTranscription":{"text":"Whatagraph"}}}));
        assert_eq!(t.tentative, "Whatagraph");
        assert!(t.committed.is_empty());
        t.accept(&json!({"serverContent":{"inputTranscription":{"text":"Whatagraph "}}}));
        t.accept(&json!({"serverContent":{"inputTranscription":{"text":"Whatagraph"},"turnComplete":true}}));
        assert_eq!(t.committed, "Whatagraph Whatagraph");
        assert!(t.tentative.is_empty());
        assert!(t.complete);
    }
    #[test]
    fn socket_audio_preview_and_release_finish_use_one_ordered_stream() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(tcp).unwrap();
            let audio: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(
                audio["realtimeInput"]["audio"]["mimeType"],
                "audio/pcm;rate=16000"
            );
            assert_eq!(
                STANDARD
                    .decode(audio["realtimeInput"]["audio"]["data"].as_str().unwrap())
                    .unwrap()
                    .len(),
                6
            );
            ws.send(Message::Text(
                json!({"serverContent":{"interimInputTranscription":{"text":"Jean"}}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
            let end: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(end["realtimeInput"]["audioStreamEnd"], true);
            ws.send(Message::Binary(json!({"serverContent":{"inputTranscription":{"text":"Jean Gottschalk"},"turnComplete":true}}).to_string().into_bytes().into())).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        });
        let tcp = TcpStream::connect(addr).unwrap();
        tcp.set_read_timeout(Some(Duration::from_millis(2)))
            .unwrap();
        let (socket, _) =
            tungstenite::client(format!("ws://{addr}"), MaybeTlsStream::Plain(tcp)).unwrap();
        let mut live = Live {
            socket,
            text: Text::default(),
            since: Instant::now(),
            go_away: false,
        };
        live.audio(&[0.0, 0.25, -0.5]).unwrap();
        assert_eq!(live.finish().unwrap(), "Jean Gottschalk");
        server.join().unwrap();
    }
}
