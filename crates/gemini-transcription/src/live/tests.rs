use super::*;
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
};
fn client(addr: std::net::SocketAddr) -> Live {
    let tcp = TcpStream::connect(addr).unwrap();
    tcp.set_read_timeout(Some(TIMEOUT)).unwrap();
    let (socket, _) =
        tungstenite::client(format!("ws://{addr}"), MaybeTlsStream::Plain(tcp)).unwrap();
    Live::setup(
        socket,
        "models/gemini-3.5-transcribe-live",
        &["Whatagraph".into()],
    )
    .unwrap()
}
fn read(ws: &mut WebSocket<TcpStream>) -> Value {
    loop {
        if let Message::Text(t) = ws.read().unwrap() {
            return serde_json::from_str(&t).unwrap();
        }
    }
}
fn send(ws: &mut WebSocket<TcpStream>, v: Value) {
    ws.send(Message::Text(v.to_string().into())).unwrap();
}
fn accept(listener: TcpListener) -> WebSocket<TcpStream> {
    let (tcp, _) = listener.accept().unwrap();
    let mut ws = tungstenite::accept(tcp).unwrap();
    let setup = read(&mut ws);
    assert_eq!(setup["setup"]["model"], "models/gemini-3.5-transcribe-live");
    assert_eq!(
        setup["setup"]["inputAudioTranscription"]["customVocabulary"][0],
        "Whatagraph"
    );
    assert_eq!(
        setup["setup"]["realtimeInputConfig"]["automaticActivityDetection"]["disabled"],
        true
    );
    ws.send(Message::Ping(vec![1].into())).unwrap();
    send(&mut ws, json!({"setupComplete":{}}));
    assert!(read(&mut ws)["realtimeInput"]["activityStart"].is_object());
    ws
}
#[test]
fn final_segments_have_sentence_spaces_without_losing_repetitions() {
    let mut t = Text::default();
    for partial in ["water", "Whatagraph"] {
        t.accept(&json!({"serverContent":{"interimInputTranscription":{"text":partial}}}));
    }
    assert_eq!(t.tentative, "Whatagraph");
    assert!(t.committed.is_empty());
    for part in ["Hello.", "Next sentence!", "Next sentence!", " Yes."] {
        t.accept(&json!({"serverContent":{"inputTranscription":{"text":part}}}));
    }
    assert_eq!(t.committed, "Hello. Next sentence! Next sentence! Yes.");
    assert!(t.tentative.is_empty());
    let mut text = "Hello".to_owned();
    append_segment(&mut text, ".");
    append_segment(&mut text, "World");
    assert_eq!(text, "Hello. World");
}
#[test]
fn final_after_turn_complete_and_final_without_turn_complete_are_drained() {
    for turn_first in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut ws = accept(listener);
            assert!(read(&mut ws)["realtimeInput"]["audio"].is_object());
            send(
                &mut ws,
                json!({"serverContent":{"interimInputTranscription":{"text":"Next"}}}),
            );
            assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
            if turn_first {
                send(&mut ws, json!({"serverContent":{"turnComplete":true}}));
                thread::sleep(Duration::from_millis(60));
            }
            send(
                &mut ws,
                json!({"serverContent":{"inputTranscription":{"text":"Next sentence."}}}),
            );
            thread::sleep(Duration::from_millis(350));
        });
        let mut live = client(addr);
        live.audio(&[0.25; 512]).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while live.text.tentative.is_empty() && Instant::now() < until {
            live.poll().unwrap();
        }
        assert_eq!(live.text.tentative, "Next");
        let start = Instant::now();
        assert_eq!(live.finish().unwrap(), "Next sentence.");
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}
#[test]
fn missing_final_is_bounded_and_never_promotes_tentative_text() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        read(&mut ws);
        send(
            &mut ws,
            json!({"serverContent":{"interimInputTranscription":{"text":"unfinished"}}}),
        );
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        thread::sleep(Duration::from_millis(1800));
    });
    let mut live = client(addr);
    live.audio(&[0.25; 512]).unwrap();
    let start = Instant::now();
    assert_eq!(live.finish(), Err(Error::Timeout));
    assert!(start.elapsed() < Duration::from_secs(2));
    server.join().unwrap();
}
#[test]
fn final_received_before_release_does_not_need_another_final() {
    for close_after_end in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut ws = accept(listener);
            assert!(read(&mut ws)["realtimeInput"]["audio"].is_object());
            send(
                &mut ws,
                json!({"serverContent":{"inputTranscription":{"text":"Already finished."}}}),
            );
            assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
            if close_after_end {
                ws.close(None).unwrap();
            } else {
                thread::sleep(Duration::from_millis(1800));
            }
        });
        let mut live = client(addr);
        live.audio(&[0.25; 512]).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while live.text.committed.is_empty() && Instant::now() < until {
            live.poll().unwrap();
        }
        assert_eq!(live.text.committed, "Already finished.");
        let start = Instant::now();
        assert_eq!(live.finish().unwrap(), "Already finished.");
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}
#[test]
fn prior_final_never_promotes_a_new_unfinished_tail() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        read(&mut ws);
        send(
            &mut ws,
            json!({"serverContent":{"inputTranscription":{"text":"First sentence."}}}),
        );
        send(
            &mut ws,
            json!({"serverContent":{"interimInputTranscription":{"text":"unfinished tail"}}}),
        );
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        thread::sleep(Duration::from_millis(1800));
    });
    let mut live = client(addr);
    live.audio(&[0.25; 512]).unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    while live.text.tentative.is_empty() && Instant::now() < until {
        live.poll().unwrap();
    }
    assert_eq!(live.text.committed, "First sentence.");
    assert_eq!(live.finish(), Err(Error::Timeout));
    assert_eq!(live.text.tentative, "unfinished tail");
    server.join().unwrap();
}
#[test]
fn pause_boundary_finalizes_during_capture_without_losing_or_repeating_samples() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        let mut frames = 0;
        let mut ends = 0;
        loop {
            let v = read(&mut ws);
            if v["realtimeInput"]["activityEnd"].is_object() {
                ends += 1;
                if ends == 1 {
                    assert_eq!(frames, 951);
                    thread::sleep(Duration::from_millis(300));
                    send(
                        &mut ws,
                        json!({"serverContent":{"inputTranscription":{"text":"First sentence."}}}),
                    );
                    assert!(read(&mut ws)["realtimeInput"]["activityStart"].is_object());
                } else {
                    assert_eq!(ends, 2);
                    assert_eq!(frames, 982);
                    send(
                        &mut ws,
                        json!({"serverContent":{"inputTranscription":{"text":"Second sentence."}}}),
                    );
                    ws.close(None).unwrap();
                    break;
                }
                continue;
            }
            let pcm = STANDARD
                .decode(v["realtimeInput"]["audio"]["data"].as_str().unwrap())
                .unwrap();
            let value: f32 = if frames < 938 {
                0.25
            } else if frames < 951 {
                0.0
            } else {
                0.5
            };
            let sample = ((value * 32767.0).round() as i16).to_le_bytes();
            assert_eq!(pcm.len(), 1024);
            assert!(pcm.as_chunks::<2>().0.iter().all(|bytes| *bytes == sample));
            frames += 1;
        }
    });
    let mut live = client(addr);
    for i in 0..982 {
        let value = if i < 938 {
            0.25
        } else if i < 951 {
            0.0
        } else {
            0.5
        };
        live.audio(&[value; 512]).unwrap();
        live.poll().unwrap();
    }
    assert!(!live.pending_audio.is_empty());
    assert!(live.text.committed.is_empty());
    assert_eq!(live.finish().unwrap(), "First sentence. Second sentence.");
    server.join().unwrap();
}
#[test]
fn pause_next_turn_missing_final_never_pastes_the_previous_turn() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        for _ in 0..951 {
            assert!(read(&mut ws)["realtimeInput"]["audio"].is_object());
        }
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        send(
            &mut ws,
            json!({"serverContent":{"inputTranscription":{"text":"Previous turn."}}}),
        );
        assert!(read(&mut ws)["realtimeInput"]["activityStart"].is_object());
        for _ in 0..31 {
            assert!(read(&mut ws)["realtimeInput"]["audio"].is_object());
        }
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        thread::sleep(Duration::from_millis(1800));
    });
    let mut live = client(addr);
    for i in 0..982 {
        live.audio(&[if (938..951).contains(&i) { 0.0 } else { 0.25 }; 512])
            .unwrap();
        live.poll().unwrap();
    }
    assert_eq!(live.finish(), Err(Error::Timeout));
    server.join().unwrap();
}
// Production setup/audio/poll/finish with a real-time mock socket. No physical mic/paste.
struct TestLogger;
impl log::Log for TestLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        println!("{}", r.args());
    }
    fn flush(&self) {}
}
static LOGGER: TestLogger = TestLogger;
fn paced_mock(seconds: usize) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let recording = Arc::new(AtomicBool::new(true));
    let flag = recording.clone();
    let frames = seconds * 16000 / 512;
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        let mut n: usize = 0;
        let mut samples = 0;
        let mut before = false;
        loop {
            let v = read(&mut ws);
            if v["realtimeInput"]["activityEnd"].is_object() {
                break;
            }
            let pcm = STANDARD
                .decode(v["realtimeInput"]["audio"]["data"].as_str().unwrap())
                .unwrap();
            assert_eq!(pcm.len(), 1024);
            let expected = (((n % 100) as f32 / 100.0 * 32767.0).round() as i16).to_le_bytes();
            assert_eq!(&pcm[..2], &expected);
            n += 1;
            samples += pcm.len() / 2;
            if n.is_multiple_of(32) {
                assert!(flag.load(Ordering::Acquire));
                before = true;
                send(
                    &mut ws,
                    json!({"serverContent":{"interimInputTranscription":{"text":"Streaming now"}}}),
                );
            }
            if n.is_multiple_of(320) {
                send(
                    &mut ws,
                    json!({"serverContent":{"inputTranscription":{"text":"Sentence."}}}),
                );
            }
        }
        assert_eq!(n, frames);
        assert!(before);
        send(
            &mut ws,
            json!({"serverContent":{"inputTranscription":{"text":"Last sentence."},"turnComplete":true}}),
        );
        thread::sleep(Duration::from_millis(350));
        samples
    });
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(2048);
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        let mut live = client(addr);
        let mut preview = false;
        for frame in rx {
            live.audio(&frame).unwrap();
            live.poll().unwrap();
            preview |= !live.text.tentative.is_empty();
        }
        assert!(preview);
        let start = Instant::now();
        let text = live.finish().unwrap();
        (
            text,
            start.elapsed(),
            live.samples,
            live.frames,
            live.partials,
        )
    });
    let start = Instant::now();
    for n in 0..frames {
        tx.try_send(vec![(n % 100) as f32 / 100.0; 512]).unwrap();
        let due = start + Duration::from_micros((n as u64 + 1) * 32000);
        thread::sleep(due.saturating_duration_since(Instant::now()));
    }
    recording.store(false, Ordering::Release);
    let release = Instant::now();
    drop(tx);
    let (text, finalization, samples, frames, partials) = worker.join().unwrap();
    let delivery = release.elapsed();
    assert_eq!(samples, seconds * 16000);
    assert!(text.ends_with("Last sentence."));
    assert!(delivery < Duration::from_secs(2));
    assert_eq!(server.join().unwrap(), samples);
    println!("MOCK seconds={seconds} frames={frames} samples={samples} partials_before_release={partials} finalization_ms={} release_to_transcript_ms={} physical_paste=UNMEASURED",finalization.as_millis(),delivery.as_millis());
}
#[test]
#[ignore = "four minute real-time mock benchmark; run explicitly"]
fn realtime_60_and_180_second_streams() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    paced_mock(60);
    paced_mock(180);
}

#[test]
fn short_paced_stream() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    paced_mock(4);
}

#[test]
fn authoritative_final_is_not_discarded_when_server_closes_immediately() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        read(&mut ws);
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        send(
            &mut ws,
            json!({"serverContent":{"inputTranscription":{"text":"Complete sentence."},"turnComplete":true}}),
        );
        ws.close(None).unwrap();
    });
    let mut live = client(addr);
    live.audio(&[0.1; 512]).unwrap();
    assert_eq!(live.finish().unwrap(), "Complete sentence.");
    server.join().unwrap();
}

#[test]
fn delayed_final_without_interim_after_completion_keeps_the_last_words() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut ws = accept(listener);
        read(&mut ws);
        send(
            &mut ws,
            json!({"serverContent":{"inputTranscription":{"text":"First sentence."}}}),
        );
        read(&mut ws);
        assert!(read(&mut ws)["realtimeInput"]["activityEnd"].is_object());
        send(&mut ws, json!({"serverContent":{"turnComplete":true}}));
        for part in ["The last", "words."] {
            thread::sleep(Duration::from_millis(300));
            send(
                &mut ws,
                json!({"serverContent":{"inputTranscription":{"text":part}}}),
            );
        }
        thread::sleep(Duration::from_millis(100));
    });
    let mut live = client(addr);
    live.audio(&[0.25; 512]).unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    while live.text.committed.is_empty() && Instant::now() < until {
        live.poll().unwrap();
    }
    assert_eq!(live.text.committed, "First sentence.");
    live.audio(&[0.25; 512]).unwrap();
    let result = live.finish();
    server.join().unwrap();
    assert_eq!(result.unwrap(), "First sentence. The last words.");
}
