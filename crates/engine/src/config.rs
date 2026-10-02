//! Persistent configuration storage (Settings, keyboard shortcut presets, the crash-recovery
//! sentinel). The engine only sees the [`ConfigStore`] trait; frontends choose where it lives:
//! the desktop app uses a [`DirConfig`] in the platform config directory, the web app can back it
//! with `localStorage`, tests and headless runs use [`MemoryConfig`] (or none at all).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Named text blobs (`prefs.json`, `shortcuts.json`, `session.lock`).
pub trait ConfigStore: Send + Sync {
    fn read(&self, name: &str) -> Option<String>;
    fn write(&self, name: &str, data: &str) -> std::io::Result<()>;
    fn remove(&self, name: &str) -> std::io::Result<()>;
    /// A directory on disk next to the configuration (default Auto-Save folder for untitled
    /// projects); `None` for stores that aren't on the file system.
    fn dir(&self) -> Option<PathBuf> {
        None
    }
}

/// In-memory store (tests, headless sessions).
#[derive(Default)]
pub struct MemoryConfig {
    pub files: Mutex<BTreeMap<String, String>>,
}

impl ConfigStore for MemoryConfig {
    fn read(&self, name: &str) -> Option<String> {
        self.files.lock().ok()?.get(name).cloned()
    }
    fn write(&self, name: &str, data: &str) -> std::io::Result<()> {
        if let Ok(mut m) = self.files.lock() {
            m.insert(name.to_string(), data.to_string());
        }
        Ok(())
    }
    fn remove(&self, name: &str) -> std::io::Result<()> {
        if let Ok(mut m) = self.files.lock() {
            m.remove(name);
        }
        Ok(())
    }
}

/// Files in one directory, written atomically.
pub struct DirConfig {
    pub dir: PathBuf,
}

impl DirConfig {
    pub fn new(dir: impl Into<PathBuf>) -> DirConfig {
        DirConfig { dir: dir.into() }
    }
}

impl ConfigStore for DirConfig {
    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }
    fn write(&self, name: &str, data: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(&self.dir.join(name), data.as_bytes())
    }
    fn remove(&self, name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
    fn dir(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}

/// The temporary file an atomic write of `path` goes through.
pub fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Write `data` to `path` atomically: write a temporary file next to it, flush it to disk, then
/// rename it over `path`. A crash at any point leaves either the old file or the new one, never a
/// torn file.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    atomic_write_with(path, data, |tmp, data| {
        use std::io::Write;
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(data)?;
        f.sync_all()
    })
}

/// [`atomic_write`] with an injectable temp-file writer (tests simulate a crash mid-write). A
/// failed write removes the temporary file and leaves `path` untouched.
pub fn atomic_write_with(path: &Path, data: &[u8], write: impl FnOnce(&Path, &[u8]) -> std::io::Result<()>) -> std::io::Result<()> {
    let tmp = temp_path(path);
    if let Err(e) = write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)
}
