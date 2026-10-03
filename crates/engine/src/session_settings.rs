//! Session side of Settings, keyboard shortcut presets, auto-save and crash recovery.

use std::path::Path;

use crate::autosave::{self, Sentinel};
use crate::prefs::{PREFS_FILE, Prefs};
use crate::shortcuts::{Keymaps, SHORTCUTS_FILE, ShortcutTable, UiCommand};
use crate::{EngineError, Result, Session};

impl Session {
    /// Load settings and shortcut presets from the config store and apply them.
    pub fn load_settings(&mut self) {
        if let Some(c) = &self.config {
            if let Some(t) = c.read(PREFS_FILE) {
                self.prefs = Prefs::from_json(&t);
            }
            if let Some(t) = c.read(SHORTCUTS_FILE) {
                self.keymaps = Keymaps::from_json(&t);
            }
        }
        self.prefs_changed();
        self.shortcuts_changed();
    }

    /// Settings changed: apply what the engine owns (cache budgets) and tell frontends.
    pub fn prefs_changed(&mut self) {
        self.prefs.normalize();
        self.apply_cache_budgets();
        self.configure_disk_cache();
        self.footage.set_conform_folder(self.conformed_audio_folder());
        // Switches Affect Nested Comps changes what precomp layers render: drop cached pixels.
        let nested = self.prefs.general.switches_affect_nested_comps;
        if self.applied_nested_switches.replace(nested).is_some_and(|was| was != nested) {
            self.layer_cache.clear();
            self.events.push(crate::Event::PurgeCaches);
        }
        // Fewer undo levels apply right away.
        let levels = self.prefs.general.undo_levels.max(1) as usize;
        if self.history.undo.len() > levels {
            let extra = self.history.undo.len() - levels;
            self.history.undo.drain(..extra);
        }
        self.prefs_revision += 1;
    }

    /// A Disk settings folder: the setting, or (for frontends with a settings store) `name` next
    /// to the platform disk cache folder. Headless sessions without the setting stay off disk.
    fn cache_folder(&self, setting: &str, name: &str) -> Option<std::path::PathBuf> {
        let s = setting.trim();
        if !s.is_empty() {
            return Some(std::path::PathBuf::from(s));
        }
        let base = effectcraft_render::disk_cache::default_folder();
        self.config.is_some().then(|| base.parent().map(|p| p.join(name)).unwrap_or_else(|| base.join(name)))
    }

    /// Settings ▸ Disk ▸ Database and Cache Folder (media cache: audio waveform summaries).
    pub fn media_cache_folder(&self) -> Option<std::path::PathBuf> {
        self.cache_folder(&self.prefs.disk.media_cache_folder, "Media Cache")
    }

    /// Settings ▸ Disk ▸ Conformed Audio Folder (decoded footage audio, see
    /// `MediaPool::set_conform_folder`).
    pub fn conformed_audio_folder(&self) -> Option<std::path::PathBuf> {
        self.cache_folder(&self.prefs.disk.conformed_media_folder, "Conformed Audio")
    }

    /// Apply the Memory & CPU cache budgets, limited by RAM Reserved for Other Applications and
    /// reduced while the system is low on memory (see [`crate::prefs::Prefs::cache_budgets`]).
    pub fn apply_cache_budgets(&mut self) {
        let b = self.prefs.cache_budgets(self.sys_memory);
        self.layer_cache.set_budget(b.layer);
        self.footage.set_cache_budget(b.media);
    }

    /// Re-read the system's memory (desktop frontends call this every few seconds) and re-apply
    /// the cache budgets when the reading changes them. Returns the effective budgets.
    pub fn memory_tick(&mut self) -> crate::prefs::CacheBudgets {
        let before = self.prefs.cache_budgets(self.sys_memory);
        self.sys_memory = crate::sysinfo::memory().or(self.sys_memory);
        let after = self.prefs.cache_budgets(self.sys_memory);
        if after != before {
            self.apply_cache_budgets();
            // Frontends re-apply their preview cache budget.
            self.prefs_revision += 1;
            if after.reduced && !before.reduced {
                self.toast("The system is low on memory: cache sizes were reduced");
            }
        }
        after
    }

    /// Apply Settings ▸ Media & Disk Cache ▸ Disk Cache: open (or close) the persistent cache
    /// and attach it to the layer cache. Without a folder setting the platform cache folder is
    /// used, but only by frontends with a settings store (headless sessions stay off disk).
    pub fn configure_disk_cache(&mut self) {
        use effectcraft_render::disk_cache::{DiskCache, default_folder, footage_salt};
        let d = &self.prefs.disk;
        let folder = if d.disk_cache_folder.trim().is_empty() {
            (self.config.is_some() && !cfg!(target_arch = "wasm32")).then(default_folder)
        } else {
            Some(std::path::PathBuf::from(d.disk_cache_folder.trim()))
        };
        let max = d.disk_cache_max_gb.max(1) as u64 * (1 << 30);
        // (no file system in the browser: the disk cache stays off there)
        let Some(folder) = folder.filter(|_| d.disk_cache_enabled && !cfg!(target_arch = "wasm32")) else {
            self.disk_cache = None;
            self.layer_cache.set_disk(None);
            return;
        };
        let dc = match &self.disk_cache {
            Some(c) if c.folder() == folder.as_path() => {
                c.set_max_bytes(max);
                c.clone()
            }
            _ => match DiskCache::open(&folder, max) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("disk cache at {}: {e}", folder.display());
                    self.disk_cache = None;
                    self.layer_cache.set_disk(None);
                    return;
                }
            },
        };
        self.layer_cache.set_disk(Some((dc.clone(), footage_salt(&self.project))));
        self.disk_cache = Some(dc);
    }

    /// Write settings to the config store.
    pub fn save_prefs(&self) {
        if let Some(c) = &self.config
            && let Err(e) = c.write(PREFS_FILE, &self.prefs.to_json())
        {
            log::warn!("saving settings: {e}");
        }
    }

    /// Shortcut presets changed: rebuild the table and save them.
    pub fn shortcuts_changed(&mut self) {
        self.shortcut_table = std::sync::OnceLock::new();
        if let Some(c) = &self.config
            && let Err(e) = c.write(SHORTCUTS_FILE, &self.keymaps.to_json())
        {
            log::warn!("saving keyboard shortcuts: {e}");
        }
    }

    /// The frontend's own bindable commands (tools, timeline navigation…).
    pub fn set_ui_commands(&mut self, cmds: Vec<UiCommand>) {
        self.ui_commands = cmds;
        self.shortcut_table = std::sync::OnceLock::new();
    }

    /// The active shortcut preset resolved against every bindable command.
    pub fn shortcuts(&self) -> &ShortcutTable {
        self.shortcut_table.get_or_init(|| ShortcutTable::build(&self.keymaps, &self.ui_commands))
    }

    /// Remember a project in File ▸ Open Recent and in the crash-recovery sentinel.
    pub(crate) fn note_project_path(&mut self, path: &str) {
        self.prefs.push_recent(path);
        self.save_prefs();
        self.autosave = autosave::AutoSaveState { last: self.autosave.last, sentinel: self.autosave.sentinel, ..Default::default() };
        self.update_sentinel();
    }

    /// Refresh the crash-recovery sentinel (when one is active).
    pub fn update_sentinel(&self) {
        if let Some(c) = &self.config
            && self.autosave.sentinel
        {
            let s = Sentinel { pid: autosave::pid(), project: self.path.clone(), autosave: self.autosave.last_path.clone(), dirty: self.is_dirty() };
            autosave::update(c.as_ref(), &s);
        }
    }

    /// Fallback folder for auto-saves of untitled projects (next to the settings).
    pub fn default_autosave_root(&self) -> Option<std::path::PathBuf> {
        self.config.as_ref().and_then(|c| c.dir())
    }

    /// Where auto-saves live: the config store's [`crate::config::FileOps`], else the file system.
    pub fn file_ops(&self) -> &dyn crate::config::FileOps {
        match &self.config {
            Some(c) => c.files(),
            None => &crate::config::StdFiles,
        }
    }

    /// Write an auto-save now (whether or not the project is dirty). Returns its path.
    pub fn autosave_now(&mut self) -> Result<String> {
        let json = self.project.to_json();
        let root = self.default_autosave_root();
        let (path, slot) = autosave::write_in(self.file_ops(), &self.prefs, self.path.as_deref(), root.as_deref(), self.autosave.last_slot, &json)
            .map_err(|e| EngineError::Other(format!("auto-save failed: {e}")))?;
        let p = path.to_string_lossy().to_string();
        self.autosave.last_slot = Some(slot);
        self.autosave.saved_revision = Some(self.revision);
        self.autosave.last_path = Some(p.clone());
        self.update_sentinel();
        Ok(p)
    }

    /// Call periodically with wall-clock seconds: auto-saves a dirty project once the interval
    /// has passed since the last auto-save (or since the first call). Returns the written path.
    pub fn autosave_tick(&mut self, now: f64) -> Option<Result<String>> {
        let a = &self.prefs.auto_save;
        if !a.enabled {
            self.autosave.last = Some(now);
            return None;
        }
        let interval = a.interval_minutes.max(1) as f64 * 60.0;
        let last = *self.autosave.last.get_or_insert(now);
        if now - last < interval {
            return None;
        }
        self.autosave.last = Some(now);
        if !self.is_dirty() || self.autosave.saved_revision == Some(self.revision) || self.render_job.is_some() {
            return None;
        }
        if self.path.is_none() && self.prefs.auto_save.location != "custom" && self.default_autosave_root().is_none() {
            return None;
        }
        let r = self.autosave_now();
        if let Ok(p) = &r {
            self.toast(format!("Auto-saved {}", Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        Some(r)
    }

    /// At launch: start crash recovery. Returns the previous run's state when it didn't exit
    /// cleanly (and the setting asks for it).
    pub fn begin_recovery(&mut self) -> Option<autosave::Recovery> {
        let c = self.config.clone()?;
        let r = autosave::begin(c.as_ref(), &self.prefs);
        self.autosave.sentinel = true;
        self.update_sentinel();
        r.filter(|_| self.prefs.startup.offer_crash_recovery)
    }

    /// Clean exit: remove the crash-recovery sentinel.
    pub fn end_recovery(&mut self) {
        if let Some(c) = &self.config
            && self.autosave.sentinel
        {
            autosave::end(c.as_ref());
            self.autosave.sentinel = false;
        }
    }
}
