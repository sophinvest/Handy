use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

fn server(
    status: u16,
    body: &'static str,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", socket.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = socket.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0; 8192];
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let len = header
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + len {
                    break;
                }
            }
        }
        tx.send(String::from_utf8(bytes).unwrap()).unwrap();
        write!(
            stream,
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (addr, rx, handle)
}
#[test]
fn wav_header_and_pcm_are_exact() {
    let wav = encode_wav(&[-1.0, 0.0, 1.0, 2.0]).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16000);
    assert_eq!(&wav[44..], &[1, 128, 0, 0, 255, 127, 255, 127]);
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
    assert_eq!(encode_wav(&[]), Err(Error::InvalidAudio));
    assert_eq!(encode_wav(&[f32::NAN]), Err(Error::InvalidAudio));
    assert_eq!(encode_wav(&vec![0.0; MAX_SAMPLES + 1]), Err(Error::TooLong));
}
#[test]
fn list_models_verifies_method_and_excludes_streaming() {
    let (addr, rx, h) = server(
        200,
        r#"{"models":[{"name":"models/gemini-3.5-transcribe","supportedGenerationMethods":["generateContent"]},{"name":"models/gemini-3.5-transcribe-live","supportedGenerationMethods":["generateContent"]},{"name":"models/gemini-4-transcribe","supportedGenerationMethods":["bidiGenerateContent"]}]}"#,
    );
    assert_eq!(
        Gemini::at(&addr, TIMEOUT)
            .unwrap()
            .models("fixture-key")
            .unwrap(),
        vec!["models/gemini-3.5-transcribe"]
    );
    assert!(rx.recv().unwrap().starts_with("GET /models?"));
    h.join().unwrap();
}
#[test]
fn request_carries_audio_vocabulary_and_verbatim_config() {
    let (addr, rx, h) = server(
        200,
        r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[{"audioTranscription":{"text":"Jean at Sophinvest."}}]}}]}"#,
    );
    let wav = encode_wav(&[0.2; 160]).unwrap();
    let result = Gemini::at(&addr, TIMEOUT)
        .unwrap()
        .transcribe(
            "fixture-key",
            "models/gemini-3.5-transcribe",
            &wav,
            &["Sophinvest".into()],
        )
        .unwrap();
    assert_eq!(result, "Jean at Sophinvest.");
    let request = rx.recv().unwrap();
    h.join().unwrap();
    assert!(request.starts_with("POST /models/gemini-3.5-transcribe:generateContent "));
    let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body["generationConfig"]["audioTranscriptionConfig"]["mode"],
        "VERBATIM"
    );
    assert_eq!(
        body["generationConfig"]["audioTranscriptionConfig"]["customVocabulary"][0],
        "Sophinvest"
    );
    assert_eq!(
        STANDARD
            .decode(
                body["contents"][0]["parts"][0]["inlineData"]["data"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        wav
    );
}
#[test]
fn quota_auth_server_and_offline_fail_once_without_leaking_body() {
    for (status, expected) in [
        (429, Error::Quota),
        (403, Error::Auth),
        (500, Error::Unavailable),
    ] {
        let (addr, rx, h) = server(status, "private-request-material");
        let result = Gemini::at(&addr, TIMEOUT).unwrap().transcribe(
            "fixture-key",
            "models/gemini-3.5-transcribe",
            b"wav",
            &[],
        );
        assert_eq!(result, Err(expected));
        rx.recv().unwrap();
        h.join().unwrap();
    }
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", socket.local_addr().unwrap());
    drop(socket);
    assert_eq!(
        Gemini::at(&addr, TIMEOUT).unwrap().models("fixture-key"),
        Err(Error::Offline)
    );
}
#[test]
fn timeout_does_not_retry() {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", socket.local_addr().unwrap());
    let h = thread::spawn(move || {
        let (_stream, _) = socket.accept().unwrap();
        thread::sleep(Duration::from_millis(150));
    });
    assert_eq!(
        Gemini::at(&addr, Duration::from_millis(25))
            .unwrap()
            .models("fixture-key"),
        Err(Error::Timeout)
    );
    h.join().unwrap();
}
#[test]
fn refuses_empty_blocked_and_truncated_results() {
    for body in [
        json!({}),
        json!({"candidates":[{"finishReason":"MAX_TOKENS","content":{"parts":[{"text":"partial"}]}}]}),
        json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{}]}}]}),
    ] {
        assert_eq!(transcript(&body), Err(Error::Empty));
    }
    assert_eq!(
        transcript(
            &json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"hello"}]}}]})
        )
        .unwrap(),
        "hello"
    );
}
#[test]
fn sixty_seconds_survive_delayed_consumer_without_sample_loss() {
    // Structural buffer test, NOT a physical microphone or recognition test.
    let source: Vec<f32> = (0..960_000).map(|i| (i % 100) as f32 / 100.0).collect();
    let (tx, rx) = mpsc::channel();
    let expected = source.clone();
    let producer = thread::spawn(move || {
        for frame in source.chunks(512) {
            tx.send(frame.to_vec()).unwrap();
        }
    });
    thread::sleep(Duration::from_millis(50));
    let actual: Vec<f32> = rx.into_iter().flatten().collect();
    producer.join().unwrap();
    assert_eq!(actual, expected);
    let wav = encode_wav(&actual).unwrap();
    assert_eq!(wav.len(), 44 + 960_000 * 2);
}

#[test]
fn live_discovery_requires_returned_live_model_and_bidi_method() {
    let (addr, rx, h) = server(
        200,
        r#"{"models":[{"name":"models/gemini-3.5-transcribe-live","supportedGenerationMethods":["bidiGenerateContent"]},{"name":"models/gemini-3.5-transcribe","supportedGenerationMethods":["generateContent"]}]}"#,
    );
    assert_eq!(
        Gemini::at(&addr, TIMEOUT)
            .unwrap()
            .live_models("fixture-key")
            .unwrap(),
        vec!["models/gemini-3.5-transcribe-live"]
    );
    rx.recv().unwrap();
    h.join().unwrap();
}
#[test]
fn previous_sentence_is_context_and_not_part_of_audio() {
    let (addr, rx, h) = server(
        200,
        r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"Next sentence."}]}}]}"#,
    );
    let wav = encode_wav(&[0.2; 160]).unwrap();
    assert_eq!(
        Gemini::at(&addr, TIMEOUT)
            .unwrap()
            .transcribe_context(
                "fixture-key",
                "models/gemini-3.5-transcribe",
                &wav,
                &[],
                "Jean at Sophinvest"
            )
            .unwrap(),
        "Next sentence."
    );
    let r = rx.recv().unwrap();
    h.join().unwrap();
    let v: Value = serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(v["contents"][0]["parts"][1]["text"]
        .as_str()
        .unwrap()
        .contains("Jean at Sophinvest"));
}
