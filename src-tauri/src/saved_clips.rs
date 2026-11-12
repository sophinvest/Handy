//! Saved recording identities are immutable timestamp filenames, never list indices.
//! All entry points use this newest-first ordering and the same delivery transaction.
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, specta::Type)]
pub struct SavedClip {
    pub id: String,
    pub saved_at_ms: f64,
    pub duration_seconds: Option<f64>,
}
fn timestamp(id: &str) -> Option<i64> {
    let stem = id.strip_suffix(".wav")?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse::<i64>().ok().filter(|n| *n > 0)
}
fn resolve(dir: &Path, id: &str) -> Result<PathBuf, String> {
    timestamp(id).ok_or("Invalid saved clip identity")?;
    let path = dir.join(id);
    let meta = fs::symlink_metadata(&path)
        .map_err(|_| "This clip is no longer available. Refresh the list")?;
    if !meta.file_type().is_file() {
        return Err("Saved clip is not a regular file".into());
    }
    Ok(path)
}
pub fn list(dir: &Path) -> Result<Vec<SavedClip>, String> {
    let mut clips = Vec::new();
    for entry in fs::read_dir(dir).map_err(|_| "Cannot read saved recordings")? {
        let entry = entry.map_err(|_| "Cannot read saved recording entry")?;
        let id = entry.file_name().to_string_lossy().into_owned();
        let Some(nanos) = timestamp(&id) else {
            continue;
        };
        if !entry
            .file_type()
            .map_err(|_| "Cannot inspect saved recording")?
            .is_file()
        {
            continue;
        }
        // A damaged WAV remains visible and deletable; retry reports the read error.
        let duration_seconds = hound::WavReader::open(entry.path()).ok().and_then(|r| {
            let rate = r.spec().sample_rate;
            (rate > 0).then(|| r.duration() as f64 / rate as f64)
        });
        clips.push(SavedClip {
            id,
            saved_at_ms: nanos as f64 / 1_000_000.0,
            duration_seconds,
        });
    }
    clips.sort_by(|a, b| {
        timestamp(&b.id)
            .cmp(&timestamp(&a.id))
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(clips)
}
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    let path = resolve(dir, id)?;
    fs::remove_file(&path).map_err(|_| "Cannot remove saved clip")?;
    // The audio is the identity/commit marker. A leftover sidecar is never listed.
    let _ = fs::remove_file(path.with_extension("json"));
    Ok(())
}
pub fn recover(
    dir: &Path,
    id: Option<&str>,
    transcribe: impl FnOnce(&Path) -> Result<String, String>,
    deliver: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    let id = match id {
        Some(id) => id.to_owned(),
        None => {
            list(dir)?
                .into_iter()
                .next()
                .ok_or("No saved failed clips")?
                .id
        }
    };
    let path = resolve(dir, &id)?;
    let text = transcribe(&path)?;
    if text.trim().is_empty() {
        return Err("No text was recognized. The clip is still saved".into());
    }
    deliver(&text)?;
    delete(dir, &id).map_err(|_| {
        "Text delivered, but clip could not be removed. Delete it to avoid duplicate text"
    })?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "handy-clips-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn clip(&self, id: &str) {
            let mut w = hound::WavWriter::create(
                self.0.join(id),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..32000 {
                w.write_sample(0i16).unwrap();
            }
            w.finalize().unwrap();
            fs::write(self.0.join(id).with_extension("json"), b"progress").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn newest_uses_numeric_identity_not_modified_time_or_lexical_order() {
        let f = Fixture::new();
        f.clip("99.wav");
        f.clip("100.wav");
        let clips = list(&f.0).unwrap();
        assert_eq!(clips[0].id, "100.wav");
        assert_eq!(clips[1].id, "99.wav");
        assert_eq!(clips[0].duration_seconds, Some(2.0));
        recover(
            &f.0,
            None,
            |p| {
                assert_eq!(p.file_name().unwrap(), "100.wav");
                Ok("newest".into())
            },
            |t| {
                assert_eq!(t, "newest");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(list(&f.0).unwrap()[0].id, "99.wav");
    }
    #[test]
    fn older_retry_and_delete_touch_only_requested_identity() {
        let f = Fixture::new();
        for id in ["100.wav", "200.wav", "300.wav"] {
            f.clip(id);
        }
        recover(&f.0, Some("100.wav"), |_| Ok("older".into()), |_| Ok(())).unwrap();
        assert!(!f.0.join("100.json").exists());
        delete(&f.0, "200.wav").unwrap();
        assert_eq!(
            list(&f.0)
                .unwrap()
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["300.wav"]
        );
    }
    #[test]
    fn recognition_delivery_and_empty_text_failures_preserve_audio_and_progress() {
        let f = Fixture::new();
        f.clip("100.wav");
        assert!(recover(
            &f.0,
            None,
            |_| Err("offline".into()),
            |_| panic!("no delivery")
        )
        .is_err());
        assert!(recover(
            &f.0,
            None,
            |_| Ok("text".into()),
            |_| Err("clipboard failed".into())
        )
        .is_err());
        assert!(recover(
            &f.0,
            None,
            |_| Ok("  ".into()),
            |_| panic!("no empty delivery")
        )
        .is_err());
        assert!(f.0.join("100.wav").exists());
        assert!(f.0.join("100.json").exists());
    }
    #[test]
    fn stale_identity_never_retargets_newest_and_empty_queue_is_explicit() {
        let f = Fixture::new();
        f.clip("200.wav");
        assert!(recover(
            &f.0,
            Some("100.wav"),
            |_| panic!("stale"),
            |_| panic!("stale")
        )
        .is_err());
        assert!(delete(&f.0, "../200.wav").is_err());
        assert!(delete(&f.0, "100.wav").is_err());
        assert_eq!(list(&f.0).unwrap().len(), 1);
        delete(&f.0, "200.wav").unwrap();
        assert!(recover(&f.0, None, |_| panic!("empty"), |_| panic!("empty")).is_err());
    }
}
