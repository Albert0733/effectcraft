//! Session side of Settings and keyboard shortcut presets.

use crate::Session;
use crate::prefs::{PREFS_FILE, Prefs};
use crate::shortcuts::{Keymaps, SHORTCUTS_FILE, ShortcutTable, UiCommand};

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
        self.layer_cache.set_budget(self.prefs.layer_cache_bytes());
        self.footage.set_cache_budget(self.prefs.media_cache_bytes());
        // Fewer undo levels apply right away.
        let levels = self.prefs.general.undo_levels.max(1) as usize;
        if self.history.undo.len() > levels {
            let extra = self.history.undo.len() - levels;
            self.history.undo.drain(..extra);
        }
        self.prefs_revision += 1;
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
}
