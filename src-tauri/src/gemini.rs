//! Google dictation integration. Keys use Secret Service on Linux, with an
//! explicitly disclosed mode-600 file fallback if the service cannot save. Failed
//! clips live outside history retention until the user retries or discards them.
use crate::managers::{
    audio::AudioRecordingManager,
    model::{ModelInfo, ModelSource},
    transcription::TranscriptionManager,
};
use crate::settings::{get_settings, write_settings};
use handy_gemini::Gemini;
use serde::Serialize;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, Manager};

pub const MODEL_ID: &str = "google-gemini-cloud";
static REQUEST: AtomicBool = AtomicBool::new(false);
/// Exclusive cloud operation ownership, transferable from capture setup to the
/// worker and back to finalization. Cancellation releases it only on worker exit.
pub struct RequestGuard;
impl Drop for RequestGuard {
    fn drop(&mut self) {
        REQUEST.store(false, Ordering::Release);
    }
}
pub(crate) fn idle_guard() -> Result<RequestGuard, String> {
    let guard = request_guard()?;
    if crate::gemini_stream::active() {
        return Err("Finish the cloud recording first".into());
    }
    Ok(guard)
}
pub(crate) fn request_guard() -> Result<RequestGuard, String> {
    REQUEST
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .map(|_| RequestGuard)
        .map_err(|_| "Finish the current Gemini request first".into())
}
fn key_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new("com.sophinvest.handy.gemini", "api-key")
        .map_err(|_| "OS keychain is unavailable".to_string())
}
#[cfg(target_os = "linux")]
fn private_key_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(crate::portable::app_data_dir(app)
        .map_err(|_| "Cannot locate Gemini key directory")?
        .join("gemini-credentials")
        .join("api-key"))
}
pub(crate) fn api_key(app: &AppHandle) -> Result<String, String> {
    // A private file represents the last save when Secret Service was locked or
    // missing. Prefer it to an older keychain entry until a later successful save.
    #[cfg(target_os = "linux")]
    {
        let path = private_key_path(app)?;
        if fs::symlink_metadata(&path).is_ok() {
            return handy_gemini::secure_file::load(&path).map_err(str::to_string);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
    key_entry()?
        .get_password()
        .map_err(|_| "Save your Gemini API key in Settings first".into())
}
fn store_key(app: &AppHandle, key: &str) -> Result<(), String> {
    if key_entry()
        .and_then(|entry| {
            entry
                .set_password(key)
                .map_err(|_| "OS keychain could not save the key".to_string())
        })
        .is_ok()
    {
        #[cfg(target_os = "linux")]
        {
            let path = private_key_path(app)?;
            if path.exists() {
                fs::remove_file(path).map_err(|_| "New key saved to Secret Service, but the old private key file could not be removed")?;
            }
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        handy_gemini::secure_file::save(&private_key_path(app)?, key).map_err(str::to_string)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Err("The OS keychain could not save the key".into())
    }
}
fn credential_storage(app: &AppHandle) -> &'static str {
    #[cfg(target_os = "linux")]
    if private_key_path(app).is_ok_and(|p| fs::symlink_metadata(p).is_ok()) {
        return "file";
    }
    let _ = app;
    "keyring"
}
pub fn model_info() -> ModelInfo {
    ModelInfo {
        id: MODEL_ID.into(), name: "Google Gemini (cloud)".into(),
        description: "Audio sent to Google. No local speech model. Free tier requires a project without billing.".into(),
        filename: String::new(), source: ModelSource::Local, size_mb: 0,
        is_downloaded: true, is_downloading: false, partial_size: 0,
        accuracy_score: 0.0, speed_score: 0.0, supports_translation: false,
        is_recommended: false, supported_languages: vec![], supports_language_selection: false,
        is_custom: false, supports_streaming: true, supports_language_detection: false,
    }
}
fn pending_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = crate::portable::app_data_dir(app)
        .map_err(|_| "Cannot locate saved recordings")?
        .join("gemini-pending");
    fs::create_dir_all(&dir).map_err(|_| "Cannot create saved recordings folder")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect saved recordings")?;
    }
    Ok(dir)
}
fn pending(app: &AppHandle) -> Result<Vec<crate::saved_clips::SavedClip>, String> {
    crate::saved_clips::list(&pending_dir(app)?)
}
fn save(app: &AppHandle, samples: &[f32]) -> Result<PathBuf, String> {
    // Save before network/encoding limits. The ordinary history retention job
    // never touches this folder, including for overlong clips.
    let dir = pending_dir(app)?;
    let name = format!(
        "{}.wav",
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .ok_or("Cannot timestamp recording")?
    );
    let path = dir.join(name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| "Recording could not be saved. Check available disk space")?;
    let mut writer = hound::WavWriter::new(
        file,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|_| "Cannot save recording")?;
    for s in samples {
        writer
            .write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .map_err(|_| "Cannot save recording")?;
    }
    writer
        .finalize()
        .map_err(|_| "Recording could not be saved completely")?;
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|f| f.sync_all())
        .map_err(|_| "Cannot finish saving recording")?;
    let _ = app.emit("gemini-clips-changed", ());
    Ok(path)
}
fn failed(app: &AppHandle, message: impl std::fmt::Display) -> String {
    let settings = get_settings(app);
    let shortcut = settings
        .bindings
        .get("gemini_retry")
        .map(|b| b.current_binding.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("the retry shortcut in Settings");
    format!("{message}. Clip saved. Press {shortcut} to retry, or open Google Gemini settings to recover it.")
}
fn upload(app: &AppHandle, wav: &[u8]) -> Result<String, String> {
    let s = get_settings(app);
    let model = s
        .gemini_model
        .ok_or("Save your key to verify an available Gemini Transcribe model")?;
    let key = api_key(app)?;
    crate::gemini_stream::reserve(app)?;
    let api = Gemini::new().map_err(|e| e.to_string())?;
    crate::gemini_stream::transcribe_retrying(
        app,
        None,
        "single_clip",
        wav.len() / 2,
        "google_uploading",
        || api.transcribe(&key, &model, wav, &s.custom_words),
    )
}
/// Returns the saved path as well as the upload result so opt-in local fallback
/// can remove exactly this pending clip only after producing its transcript.
pub fn transcribe(
    app: &AppHandle,
    samples: &[f32],
    recording: bool,
) -> Result<(Option<RequestGuard>, PathBuf, Result<String, String>), String> {
    let mut streamed = if recording {
        crate::gemini_stream::finish(samples.len())
    } else {
        None
    };
    let guard = streamed
        .as_mut()
        .and_then(|(_, _, guard)| guard.take())
        .or_else(|| request_guard().ok());
    let path = save(app, samples)?;
    let result = (|| {
        if let Some((result, progress, _)) = streamed {
            if result.is_err() && progress.offset > 0 {
                let body = serde_json::to_vec(&progress)
                    .map_err(|_| "Cannot save piece recovery progress")?;
                let sidecar = path.with_extension("json");
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                use std::io::Write;
                let mut f = options
                    .open(sidecar)
                    .map_err(|_| "Cannot save piece recovery progress")?;
                f.write_all(&body)
                    .and_then(|_| f.sync_all())
                    .map_err(|_| "Cannot save piece recovery progress")?;
            }
            return result;
        }
        if guard.is_none() {
            return Err("Another Gemini request is running".to_string());
        }
        let wav = handy_gemini::encode_wav(samples).map_err(|e| e.to_string())?;
        upload(app, &wav)
    })()
    .map_err(|e: String| failed(app, e));
    if result.is_ok() {
        // Keep the audio of every dictation that worked (SOP-10847); never leave it in
        // the retry list, where it would be offered again as a failure.
        let kept = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|id| crate::saved_clips::keep(&pending_dir(app)?, id))
            .unwrap_or_else(|| Err("Clip has no name".into()));
        if kept.is_err() {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(path.with_extension("json"));
        }
        let _ = app.emit("gemini-clips-changed", ());
    }
    Ok((guard, path, result))
}

/// Recent Google failure/retry lines from Handy's own log files, newest last.
/// Only metadata lines are returned: they carry no key and no transcript text.
#[tauri::command]
#[specta::specta]
pub fn gemini_diagnostics(app: AppHandle) -> Result<String, String> {
    const MARKERS: [&str; 5] = [
        "gemini_diag",
        "gemini_live fallback",
        "gemini_live server_error",
        "gemini_live socket_closed",
        "gemini_live finalization_timeout",
    ];
    let dir = crate::portable::app_log_dir(&app).map_err(|_| "Cannot locate the log folder")?;
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|_| "Cannot read the log folder")?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "log")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("handy"))
        })
        .collect();
    files.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    let mut lines: Vec<String> = Vec::new();
    for file in files {
        if let Ok(bytes) = fs::read(&file) {
            lines.extend(
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .filter(|l| MARKERS.iter().any(|m| l.contains(m)))
                    .map(str::to_owned),
            );
        }
    }
    let skip = lines.len().saturating_sub(60);
    let body = if lines.is_empty() {
        "(no Google failures logged)".to_string()
    } else {
        lines.split_off(skip).join("\n")
    };
    Ok(format!(
        "Handy {} ({})\nLog folder: {}\n{}",
        app.package_info().version,
        std::env::consts::OS,
        dir.display(),
        body
    ))
}

#[derive(Serialize, specta::Type)]
pub struct Status {
    configured: bool,
    credential_storage: String,
    model: Option<String>,
    live_model: Option<String>,
    mode: String,
    chunk_seconds: u32,
    rpm: Option<u32>,
    rpd: Option<u32>,
    requests_today: u32,
    fallback: bool,
    fallback_model: String,
    pending: usize,
}
#[tauri::command]
#[specta::specta]
pub async fn gemini_status(app: AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = get_settings(&app);
        Ok(Status {
            configured: api_key(&app).is_ok(),
            credential_storage: credential_storage(&app).into(),
            model: s.gemini_model,
            live_model: s.gemini_live_model,
            mode: s.gemini_mode,
            chunk_seconds: s.gemini_chunk_seconds,
            rpm: s.gemini_rpm,
            rpd: s.gemini_rpd,
            requests_today: crate::gemini_stream::usage(&app),
            fallback: false,
            fallback_model: s.gemini_fallback_model,
            pending: pending(&app)?.len(),
        })
    })
    .await
    .map_err(|_| "Cannot inspect Gemini settings".to_string())?
}
#[tauri::command]
#[specta::specta]
pub async fn gemini_save_key(app: AppHandle, key: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = idle_guard()?;
        let key = key.trim();
        if key.is_empty() {
            return Err("Enter your Gemini API key".into());
        }
        let models = Gemini::new()
            .map_err(|e| e.to_string())?
            .models(key)
            .map_err(|e| e.to_string())?;
        let live_models = Gemini::new()
            .map_err(|e| e.to_string())?
            .live_models(key)
            .unwrap_or_default();
        store_key(&app, key)?;
        let mut s = get_settings(&app);
        s.gemini_model = Some(models[0].clone());
        s.gemini_live_model = live_models.first().cloned();
        write_settings(&app, s);
        Ok(models)
    })
    .await
    .map_err(|_| "Cannot save Gemini key".to_string())?
}
#[tauri::command]
#[specta::specta]
pub fn gemini_set_fallback(app: AppHandle, enabled: bool, model: String) -> Result<(), String> {
    let _guard = idle_guard()?;
    if enabled {
        return Err(
            "Google mode never runs local models. Select a local model to use offline dictation"
                .into(),
        );
    }
    let mut s = get_settings(&app);
    s.gemini_fallback = enabled;
    s.gemini_fallback_model = model;
    write_settings(&app, s);
    Ok(())
}
pub fn retry(
    app: &AppHandle,
    clip_id: Option<&str>,
    from_settings: bool,
) -> Result<String, String> {
    let _guard = idle_guard()?;
    if get_settings(app).selected_model != MODEL_ID {
        return Err("Choose Google Gemini before retrying".into());
    }
    let rm = app.state::<Arc<AudioRecordingManager>>();
    if rm.is_recording() {
        return Err("Finish recording before retrying".into());
    }
    let tm = app.state::<Arc<TranscriptionManager>>();
    tm.ensure_model_unlocked()?;
    let dir = pending_dir(app)?;
    let text = crate::saved_clips::recover(
        &dir,
        clip_id,
        |path| recover_text(app, path),
        |text| {
            let settings = get_settings(app);
            if settings.paste_method == crate::settings::PasteMethod::None
                && settings.clipboard_handling
                    != crate::settings::ClipboardHandling::CopyToClipboard
            {
                return Err("Text delivery is disabled. Enable a paste method or Copy to clipboard; the clip is still saved".into());
            }
            let window = app.get_webview_window("main");
            let hide = from_settings && settings.paste_method != crate::settings::PasteMethod::None;
            if hide {
                if let Some(window) = &window {
                    window.hide().map_err(|_| {
                        "Cannot return to the target application; clip is still saved"
                    })?;
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
            let result = crate::utils::paste(text.to_owned(), app.clone())
                .map_err(|e| format!("Text delivery failed: {e}. The clip is still saved"));
            if hide && result.is_err() {
                if let Some(window) = &window {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            result
        },
    );
    let _ = app.emit("gemini-clips-changed", ());
    text
}
fn recover_text(app: &AppHandle, path: &std::path::Path) -> Result<String, String> {
    // Enforce the same size limit on recovery, without loading an unbounded file.
    if fs::metadata(path)
        .map_err(|_| "Cannot read saved clip")?
        .len()
        > 44 + (16_000 * 3600 * 2) as u64
    {
        return Err(
            "Saved clip exceeds the one-hour recovery limit. Open the saved clips folder".into(),
        );
    }
    let wav = fs::read(path).map_err(|_| "Cannot read saved clip")?;
    let recovery = path.with_extension("json");
    let mut progress: crate::gemini_stream::Progress = if recovery.exists() {
        serde_json::from_slice(&fs::read(&recovery).map_err(|_| "Cannot read recovery progress")?)
            .map_err(|_| "Invalid recovery progress; clip is preserved")?
    } else {
        Default::default()
    };
    let reader =
        hound::WavReader::new(std::io::Cursor::new(&wav)).map_err(|_| "Cannot read saved WAV")?;
    let samples = reader
        .into_samples::<i16>()
        .map(|s| s.map(|n| n as f32 / 32767.0))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Cannot read saved WAV samples")?;
    if progress.offset > samples.len() {
        return Err("Recovery progress is outside the saved clip".into());
    }
    if progress.offset < samples.len() {
        let tail = &samples[progress.offset..];
        let mut cutter = handy_gemini::pieces::Cutter::new(get_settings(app).gemini_chunk_seconds);
        let mut cut = tail.len();
        for frame in tail.chunks_exact(handy_gemini::pieces::FRAME) {
            if let Some(n) = cutter.push(frame, handy_gemini::pieces::is_speech(frame)) {
                cut = n;
                break;
            }
        }
        let encoded = handy_gemini::encode_wav(&tail[..cut]).map_err(|e| e.to_string())?;
        let s = get_settings(app);
        let key = api_key(app)?;
        let model = s.gemini_model.ok_or("Verify your key first")?;
        crate::gemini_stream::reserve(app)?;
        let api = Gemini::new().map_err(|e| e.to_string())?;
        let previous = handy_gemini::pieces::last_sentence(&progress.text);
        let part = crate::gemini_stream::transcribe_retrying(
            app,
            None,
            "retry_clip",
            cut,
            "google_pieces",
            || api.transcribe_context(&key, &model, &encoded, &s.custom_words, &previous),
        )
        .map_err(|e| failed(app, e))?;
        handy_gemini::live::append_segment(&mut progress.text, &part);
        progress.offset += cut;
        // Recovery updates are atomic and private, just like the saved WAV.
        use std::io::Write;
        let next = recovery.with_extension("json.tmp");
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options
            .open(&next)
            .map_err(|_| "Cannot save recovery progress")?;
        f.write_all(&serde_json::to_vec(&progress).map_err(|_| "Cannot save recovery progress")?)
            .and_then(|_| f.sync_all())
            .map_err(|_| "Cannot save recovery progress")?;
        fs::rename(next, &recovery).map_err(|_| "Cannot save recovery progress")?;
        if progress.offset < samples.len() {
            return Err("One saved piece transcribed. The complete clip is still saved. Retry again when quota permits; nothing has been pasted yet".into());
        }
    }
    Ok(progress.text)
}
#[tauri::command]
#[specta::specta]
pub async fn gemini_retry_clip(app: AppHandle, clip_id: Option<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || retry(&app, clip_id.as_deref(), true))
        .await
        .map_err(|_| "Retry failed".to_string())?
}
#[tauri::command]
#[specta::specta]
pub fn gemini_open_clips(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(pending_dir(&app)?.to_string_lossy(), None::<String>)
        .map_err(|_| "Cannot open saved clips folder".into())
}

#[tauri::command]
#[specta::specta]
pub fn gemini_list_clips(app: AppHandle) -> Result<Vec<crate::saved_clips::SavedClip>, String> {
    pending(&app)
}
#[tauri::command]
#[specta::specta]
pub fn gemini_delete_clip(app: AppHandle, clip_id: String) -> Result<(), String> {
    let _guard = idle_guard()?;
    if app.state::<Arc<AudioRecordingManager>>().is_recording() {
        return Err("Finish recording before deleting a clip".into());
    }
    crate::saved_clips::delete(&pending_dir(&app)?, &clip_id)?;
    let _ = app.emit("gemini-clips-changed", ());
    Ok(())
}

// ---- Detailed Google log (SOP-10811) ------------------------------------------
// Off by default. The switch is a flag file in the app data folder so it survives
// restarts without touching the settings schema; trace lines live in the log folder.
#[derive(Serialize, specta::Type)]
pub struct TraceStatus {
    enabled: bool,
    folder: String,
    file: String,
}
fn trace_flag(app: &AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("google-trace-enabled"))
        .map_err(|_| "Cannot locate the app data folder".to_string())
}
fn trace_dir(app: &AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_log_dir(app).map_err(|_| "Cannot locate the log folder".to_string())
}
fn trace_status(app: &AppHandle) -> Result<TraceStatus, String> {
    let dir = trace_dir(app)?;
    let file = dir.join(handy_gemini::trace::file_name(
        &chrono::Local::now().format("%Y-%m-%d").to_string(),
    ));
    Ok(TraceStatus {
        enabled: handy_gemini::trace::enabled(),
        folder: dir.display().to_string(),
        file: file.display().to_string(),
    })
}
/// Called once at startup: resume tracing if the user left it on.
pub fn trace_init(app: &AppHandle) {
    let on = trace_flag(app).map(|f| f.exists()).unwrap_or(false);
    if on {
        if let Ok(dir) = trace_dir(app) {
            let _ = handy_gemini::trace::enable(&dir);
            handy_gemini::trace::log("trace", || {
                format!("Handy {} started", app.package_info().version)
            });
        }
    }
}
#[tauri::command]
#[specta::specta]
pub fn gemini_trace_status(app: AppHandle) -> Result<TraceStatus, String> {
    trace_status(&app)
}
#[tauri::command]
#[specta::specta]
pub fn gemini_trace_set(app: AppHandle, enabled: bool) -> Result<TraceStatus, String> {
    let flag = trace_flag(&app)?;
    if enabled {
        handy_gemini::trace::enable(&trace_dir(&app)?)
            .map_err(|_| "Cannot create the log folder".to_string())?;
        fs::write(&flag, b"on").map_err(|_| "Cannot save the log setting".to_string())?;
        handy_gemini::trace::log("trace", || {
            format!("Handy {} detailed log on", app.package_info().version)
        });
    } else {
        handy_gemini::trace::disable();
        let _ = fs::remove_file(&flag);
    }
    trace_status(&app)
}
/// Last part of today's trace, for the Copy debug log button.
#[tauri::command]
#[specta::specta]
pub fn gemini_trace_copy(_app: AppHandle) -> Result<String, String> {
    let text = handy_gemini::trace::tail(2 * 1024 * 1024);
    if text.is_empty() {
        Err("Nothing logged today. Turn the detailed log on, dictate, then copy it".into())
    } else {
        Ok(text)
    }
}
/// The overlay window reports what it shows (label, dot colour, error text).
#[tauri::command]
#[specta::specta]
pub fn gemini_trace_event(tag: String, message: String) {
    handy_gemini::trace::log(
        &format!("ui:{}", tag.chars().take(24).collect::<String>()),
        || message.chars().take(1000).collect(),
    );
}
#[tauri::command]
#[specta::specta]
pub fn gemini_trace_open(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = trace_dir(&app)?;
    fs::create_dir_all(&dir).map_err(|_| "Cannot create the log folder".to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<String>)
        .map_err(|_| "Cannot open the log folder".to_string())
}
