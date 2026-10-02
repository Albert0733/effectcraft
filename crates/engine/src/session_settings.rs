//! Session side of Settings.

use crate::Session;
use crate::prefs::{PREFS_FILE, Prefs};

impl Session {
    /// Load settings from the config store and apply them.
    pub fn load_settings(&mut self) {
        if let Some(c) = &self.config
            && let Some(t) = c.read(PREFS_FILE)
        {
            self.prefs = Prefs::from_json(&t);
        }
        self.prefs_changed();
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
}
