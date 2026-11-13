// ALGORITHM VERSION 1.0 — 2026-11-13 (Detailed Google log)
// 1. Off by default. When on, every call to `log` appends one timestamped line to
//    `google-trace-YYYY-MM-DD.log` (one file per local day) in the chosen folder.
// 2. Lines name what happened (key, mic, socket, message, timer, turn, engine, overlay,
//    retry, paste) and carry sizes, timings and states. Never audio bytes, never the key.
//    Transcript text is allowed: Jean asked for it, and the Settings toggle says so.
// 3. A file stops growing at 50 MB; files older than 7 days are removed when tracing starts.
// v1.0 — SOP-10811: new, for the gemini.11 timeout investigation.
//! Opt-in detailed trace of everything the Google path does.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime},
};
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
static ON: AtomicBool = AtomicBool::new(false);
struct Sink {
    dir: PathBuf,
    day: String,
    file: Option<File>,
    bytes: u64,
}
static SINK: Mutex<Option<Sink>> = Mutex::new(None);
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}
fn day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
pub fn file_name(day: &str) -> String {
    format!("google-trace-{day}.log")
}
/// Turn tracing on, writing into `dir`. Removes trace files older than 7 days.
pub fn enable(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .is_some_and(|age| age > KEEP);
            if old && e.file_name().to_string_lossy().starts_with("google-trace-") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sink {
        dir: dir.to_path_buf(),
        day: String::new(),
        file: None,
        bytes: 0,
    });
    ON.store(true, Ordering::Relaxed);
    log("trace", || "detailed Google log enabled".into());
    Ok(())
}
pub fn disable() {
    log("trace", || "detailed Google log disabled".into());
    ON.store(false, Ordering::Relaxed);
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
/// Append `tag: message`. The closure runs only when tracing is on.
pub fn log(tag: &str, message: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let line = format!(
        "{} [{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        tag,
        message().replace('\n', " ")
    );
    let Ok(mut guard) = SINK.lock() else { return };
    let Some(sink) = guard.as_mut() else { return };
    let today = day();
    if sink.day != today || sink.file.is_none() {
        let path = sink.dir.join(file_name(&today));
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        sink.bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        sink.file = options.open(path).ok();
        sink.day = today;
    }
    if sink.bytes >= MAX_BYTES {
        return;
    }
    if let Some(f) = sink.file.as_mut() {
        if f.write_all(line.as_bytes()).is_ok() {
            sink.bytes += line.len() as u64;
        }
    }
}
/// Path of today's trace file, if tracing has a folder.
pub fn current_path() -> Option<PathBuf> {
    SINK.lock()
        .ok()?
        .as_ref()
        .map(|s| s.dir.join(file_name(&day())))
}
/// Last `max_bytes` of today's file as text (for the Copy button).
pub fn tail(max_bytes: u64) -> String {
    let Some(path) = current_path() else {
        return String::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(max_bytes as usize);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}
