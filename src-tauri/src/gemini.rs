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
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager};

pub const MODEL_ID: &str = "google-gemini-cloud";
static REQUEST: Mutex<()> = Mutex::new(());
pub fn idle_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    REQUEST
        .try_lock()
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
fn api_key(app: &AppHandle) -> Result<String, String> {
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
        is_custom: false, supports_streaming: false, supports_language_detection: false,
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
fn pending(app: &AppHandle) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<_> = fs::read_dir(pending_dir(app)?)
        .map_err(|_| "Cannot read saved recordings")?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    Ok(files)
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
    Gemini::new()
        .map_err(|e| e.to_string())?
        .transcribe(&key, &model, wav, &s.custom_words)
        .map_err(|e| e.to_string())
}
/// Returns the saved path as well as the upload result so opt-in local fallback
/// can remove exactly this pending clip only after producing its transcript.
pub fn transcribe(
    app: &AppHandle,
    samples: &[f32],
) -> Result<
    (
        Option<std::sync::MutexGuard<'static, ()>>,
        PathBuf,
        Result<String, String>,
    ),
    String,
> {
    let guard = REQUEST.try_lock().ok();
    let path = save(app, samples)?;
    let result = (|| {
        if guard.is_none() {
            return Err("Another Gemini request is running".to_string());
        }
        let wav = handy_gemini::encode_wav(samples).map_err(|e| e.to_string())?;
        upload(app, &wav)
    })()
    .map_err(|e: String| failed(app, e));
    if result.is_ok() {
        let _ = fs::remove_file(&path);
        let _ = app.emit("gemini-clips-changed", ());
    }
    Ok((guard, path, result))
}

#[derive(Serialize, specta::Type)]
pub struct Status {
    configured: bool,
    credential_storage: String,
    model: Option<String>,
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
            fallback: s.gemini_fallback,
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
        let _guard = REQUEST
            .try_lock()
            .map_err(|_| "Finish the current Gemini request first")?;
        let key = key.trim();
        if key.is_empty() {
            return Err("Enter your Gemini API key".into());
        }
        let models = Gemini::new()
            .map_err(|e| e.to_string())?
            .models(key)
            .map_err(|e| e.to_string())?;
        store_key(&app, key)?;
        let mut s = get_settings(&app);
        s.gemini_model = Some(models[0].clone());
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
        let mm = app.state::<Arc<crate::managers::model::ModelManager>>();
        if model == MODEL_ID || !mm.get_model_info(&model).is_some_and(|m| m.is_downloaded) {
            return Err("Select a downloaded local model for fallback".into());
        }
    }
    let mut s = get_settings(&app);
    s.gemini_fallback = enabled;
    s.gemini_fallback_model = model;
    write_settings(&app, s);
    Ok(())
}
pub fn retry(app: &AppHandle, paste: bool) -> Result<String, String> {
    let _guard = REQUEST
        .try_lock()
        .map_err(|_| "Another Gemini request is running")?;
    if get_settings(app).selected_model != MODEL_ID {
        return Err("Choose Google Gemini before retrying".into());
    }
    let rm = app.state::<Arc<AudioRecordingManager>>();
    if rm.is_recording() {
        return Err("Finish recording before retrying".into());
    }
    let tm = app.state::<Arc<TranscriptionManager>>();
    tm.ensure_model_unlocked()?;
    let path = pending(app)?
        .into_iter()
        .next()
        .ok_or("No saved Gemini clips to retry")?;
    // Enforce the same size limit on recovery, without loading an unbounded file.
    if fs::metadata(&path)
        .map_err(|_| "Cannot read saved clip")?
        .len()
        > 44 + (handy_gemini::MAX_SAMPLES * 2) as u64
    {
        return Err("Saved clip exceeds five minutes. Open the saved clips folder and split it before retrying".into());
    }
    let wav = fs::read(&path).map_err(|_| "Cannot read saved clip")?;
    let text = upload(app, &wav).map_err(|e| failed(app, e))?;
    if paste {
        crate::utils::paste(text.clone(), app.clone())
            .map_err(|_| "Paste failed. The clip is still saved")?;
    }
    // The settings preview may be copied or pasted later, so keep its recording.
    if paste {
        fs::remove_file(path).map_err(|_| "Text pasted, but saved clip could not be removed")?;
    }
    Ok(text)
}
#[tauri::command]
#[specta::specta]
pub async fn gemini_retry_clip(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || retry(&app, false))
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
