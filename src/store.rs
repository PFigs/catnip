//! Saved settings: `Prefs` on native, local storage on the web. The web token, and the clock.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Search scopes that pick the PRs: `org:acme`, `user:ann` or `repo:acme/api`.
    pub scopes: Vec<String>,
    pub authors: Vec<String>,
    /// Saved groups of authors by name, such as `bots`. A group picks its authors.
    pub author_groups: BTreeMap<String, BTreeSet<String>>,
    /// The user closed the file tree.
    pub tree_closed: bool,
    /// The repositories whose group the drawer folds.
    pub folded: BTreeSet<String>,
    /// The width of the file tree, once the user changed it.
    pub tree_width: Option<f32>,
    /// The conversation lists the oldest comment first, not the newest.
    pub oldest_first: bool,
}

const SETTINGS: &str = "catnip.settings";

/// The settings store, opened once. It writes only when the settings change.
pub struct Store {
    #[cfg(not(target_arch = "wasm32"))]
    prefs: rune::prefs::Prefs,
    saved: Settings,
}

impl Store {
    pub fn settings(&self) -> &Settings {
        &self.saved
    }

    pub fn save(&mut self, settings: &Settings) {
        if *settings != self.saved && self.write(settings) {
            self.saved = settings.clone();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Store {
    pub fn open() -> Self {
        let prefs = rune::prefs::Prefs::open("catnip");
        let saved = prefs.get(SETTINGS).unwrap_or_default();
        Store { prefs, saved }
    }

    /// A store that keeps the settings in memory, for tests and scenes.
    pub fn memory() -> Self {
        Store {
            prefs: rune::prefs::Prefs::memory(),
            saved: Settings::default(),
        }
    }

    fn write(&mut self, settings: &Settings) -> bool {
        let written = self.prefs.set(SETTINGS, settings);
        if let Err(e) = &written {
            eprintln!("catnip: {e}");
        }
        written.is_ok()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(target_arch = "wasm32")]
const TOKEN: &str = "catnip.token";

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
impl Store {
    pub fn open() -> Self {
        let saved = storage()
            .and_then(|s| s.get_item(SETTINGS).ok()?)
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Store { saved }
    }

    fn write(&mut self, settings: &Settings) -> bool {
        let text = serde_json::to_string(settings).unwrap_or_default();
        storage().is_some_and(|s| s.set_item(SETTINGS, &text).is_ok())
    }
}

/// The token pasted in the web build.
#[cfg(target_arch = "wasm32")]
pub fn token() -> Option<String> {
    storage()?.get_item(TOKEN).ok()?.filter(|t| !t.is_empty())
}

#[cfg(target_arch = "wasm32")]
pub fn save_token(token: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(TOKEN, token);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn remove_token() {
    if let Some(s) = storage() {
        let _ = s.remove_item(TOKEN);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}
