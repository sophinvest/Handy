//! Linux-only credential fallback, used only when Secret Service cannot save.
//! Values are never returned in errors or logs. The caller owns the directory.
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub fn load(path: &Path) -> Result<String, &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Cannot read private Gemini key file")?;
    if !meta.is_file() {
        return Err("Gemini key file must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("Gemini key file permissions must be 0600");
        }
    }
    let mut value = String::new();
    fs::File::open(path)
        .map_err(|_| "Cannot open private Gemini key file")?
        .take(4097)
        .read_to_string(&mut value)
        .map_err(|_| "Cannot read private Gemini key file")?;
    if value.is_empty() || value.len() > 4096 {
        return Err("Private Gemini key file is invalid");
    }
    Ok(value)
}

pub fn save(path: &Path, value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 4096 {
        return Err("Gemini key is invalid");
    }
    let parent = path
        .parent()
        .ok_or("Cannot locate private Gemini key directory")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create private Gemini key directory")?;
    let meta =
        fs::symlink_metadata(parent).map_err(|_| "Cannot inspect private Gemini key directory")?;
    if !meta.is_dir() {
        return Err("Gemini key directory must not be a symbolic link");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect private Gemini key directory")?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|_| "Cannot create private Gemini key file")?;
    let result = (|| {
        file.write_all(value.as_bytes())
            .map_err(|_| "Cannot save private Gemini key file")?;
        file.sync_all()
            .map_err(|_| "Cannot finish saving private Gemini key file")?;
        fs::rename(&tmp, path).map_err(|_| "Cannot replace private Gemini key file")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[test]
    fn private_file_roundtrip_and_atomic_replacement() {
        let dir =
            std::env::temp_dir().join(format!("handy-credential-test-{}", std::process::id()));
        let path = dir.join("private").join("api-key");
        save(&path, "fixture-key-one").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(load(&path).unwrap(), "fixture-key-one");
        save(&path, "fixture-key-two").unwrap();
        assert_eq!(load(&path).unwrap(), "fixture-key-two");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&path).is_err());
        let link = dir.join("link");
        symlink(&path, &link).unwrap();
        assert!(load(&link).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
