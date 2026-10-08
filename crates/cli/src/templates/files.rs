//! Bounded regular-file reads and private, atomic exports. A template never walks a workspace.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::format::MAX_BYTES;
use crate::app::App;

pub fn read(path: &Path, limit: usize) -> Result<String, String> {
    if fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "Choose a regular file, not a symbolic link: {}",
            path.display()
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Choose a regular template or Markdown file.".into());
    }
    if meta.len() > limit as u64 {
        return Err(format!(
            "{} exceeds the {limit} byte limit.",
            path.display()
        ));
    }
    let mut text = String::new();
    file.take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("Cannot read UTF-8 text: {e}"))?;
    if text.len() > limit {
        return Err(format!("The file exceeds the {limit} byte limit."));
    }
    Ok(text)
}

pub fn memory_dir(app: &App, bot_id: &str) -> Result<PathBuf, String> {
    if bot_id.is_empty() || bot_id == "." || bot_id == ".." || bot_id.contains(['/', '\\', '\0']) {
        return Err("The bot id is not a workspace directory name.".into());
    }
    let root = app.config.home.canonicalize().map_err(|e| e.to_string())?;
    let dir = root.join("workspaces").join(bot_id);
    regular_directories(&root, &dir)?;
    Ok(dir)
}

/// Refuses symlinks beneath the private runtime root, including a topic directory linked to
/// another folder. Missing directories are fine: a new bot may have no memory yet.
pub fn regular_directories(root: &Path, dir: &Path) -> Result<(), String> {
    let relative = dir
        .strip_prefix(root)
        .map_err(|_| "The folder is outside Lorca's data directory.")?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Unsafe memory directory.".into());
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "{} must be a directory, not a link or file.",
                    current.display()
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub fn export(app: &App, path: &Path, text: &str, overwrite: bool) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err("A template file is at most 1 MiB.".into());
    }
    if path.extension().and_then(|e| e.to_str()) != Some("lorca-template") {
        return Err("Save the private file with the .lorca-template extension.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|e| format!("Choose an existing folder: {e}"))?;
    let home = app.config.home.canonicalize().map_err(|e| e.to_string())?;
    if parent.starts_with(&home) {
        return Err("Save the template outside Lorca's data folder.".into());
    }
    let target = parent.join(path.file_name().ok_or("Choose a file name")?);
    match fs::symlink_metadata(&target) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err("The export destination must be a regular file.".into())
        }
        Ok(_) if !overwrite => {
            return Err("That file exists. Choose another name or explicitly replace it.".into())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let temporary = parent.join(format!(".lorca-template-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        if overwrite {
            fs::rename(&temporary, &target).map_err(|e| e.to_string())?;
        } else {
            // Atomic create, preserving a file that appeared after the panel/preview.
            fs::hard_link(&temporary, &target)
                .map_err(|e| format!("Cannot create the export file: {e}"))?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
