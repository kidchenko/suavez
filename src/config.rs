//! Persisted settings, as JSON in `%APPDATA%\SuaVez\config.json` on Windows and
//! `~/Library/Application Support/SuaVez/config.json` on macOS.
//!
//! The same directory holds `sessions/`, where hook processes leave one small
//! file per Claude Code session (see `session.rs`), plus the log and, on macOS,
//! the instance socket.

use crate::log::{lerror, linfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Notify when Claude finishes a turn.
    pub notify_done: bool,
    /// Notify when Claude is blocked on a permission prompt or a question.
    pub notify_needs_you: bool,
    /// A turn shorter than this does not notify when it ends. You were almost
    /// certainly still looking at it, and a toast per quick reply is noise.
    pub min_turn_seconds: u64,
    /// A permission prompt answered within this many seconds does not notify
    /// either, for the same reason.
    pub grace_seconds: u64,
    /// Sessions that have not reported anything for this long are dropped.
    /// Covers a terminal closed without Claude Code getting to say goodbye.
    pub stale_hours: u64,
    /// Whether the first launch has offered to connect to Claude Code. Set
    /// once, so disconnecting from the menu sticks across restarts.
    pub hooks_offered: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notify_done: true,
            notify_needs_you: true,
            min_turn_seconds: 30,
            grace_seconds: 8,
            stale_hours: 8,
            hooks_offered: false,
        }
    }
}

impl Config {
    #[cfg(windows)]
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SuaVez"))
    }

    #[cfg(target_os = "macos")]
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/SuaVez"))
    }

    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    pub fn modified() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path()?).ok()?.modified().ok()
    }

    /// `None` for missing or unparsable, so an editor caught mid-save changes
    /// nothing.
    pub fn reload() -> Option<Self> {
        Self::parse(&std::fs::read_to_string(Self::path()?).ok()?).ok()
    }

    fn parse(text: &str) -> Result<Self, serde_json::Error> {
        // Notepad and PowerShell both like to write a BOM; serde_json does not.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        serde_json::from_str::<Config>(text).map(Config::sanitised)
    }

    /// Never fails: a missing or corrupt file yields defaults.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            linfo!("no config at {}; using defaults", path.display());
            return Self::default();
        };
        match Self::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                lerror!(
                    "config at {} is unreadable ({e}); using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Temp file + rename, so an interrupted save never leaves half a file.
    pub fn save(&self) -> std::io::Result<()> {
        let (Some(dir), Some(path)) = (Self::dir(), Self::path()) else {
            return Err(std::io::Error::other("no config directory in environment"));
        };
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn sanitised(mut self) -> Self {
        self.min_turn_seconds = self.min_turn_seconds.min(3600);
        self.grace_seconds = self.grace_seconds.min(600);
        self.stale_hours = self.stale_hours.clamp(1, 7 * 24);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_bom_and_missing_fields() {
        let c = Config::parse("\u{feff}{\"notifyDone\": false}").unwrap();
        assert!(!c.notify_done);
        assert!(c.notify_needs_you);
        assert_eq!(c.min_turn_seconds, 30);
    }

    #[test]
    fn clamps_nonsense() {
        let c = Config::parse(r#"{"staleHours": 0, "graceSeconds": 99999}"#).unwrap();
        assert_eq!(c.stale_hours, 1);
        assert_eq!(c.grace_seconds, 600);
    }
}
