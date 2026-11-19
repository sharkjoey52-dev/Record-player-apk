//! Saved app settings (settings.json in the app's data folder).

use serde::{Deserialize, Serialize};

use crate::platform;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Launch the app automatically when the tablet boots (Android only).
    pub open_on_startup: bool,
    /// Stop the screen from switching off while the app is open.
    pub keep_screen_on: bool,
    /// Interface zoom (1.0 = normal).
    pub ui_scale: f32,
    /// Use the keyboard drawn inside the app for the search box.
    pub builtin_keyboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            open_on_startup: false,
            keep_screen_on: true,
            ui_scale: 1.0,
            builtin_keyboard: false,
        }
    }
}

fn path() -> std::path::PathBuf {
    platform::data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), s);
        }
    }
}
