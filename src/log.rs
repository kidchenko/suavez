//! Minimal file logging to `suavez.log` next to the config.
//!
//! There is no console under `windows_subsystem = "windows"`, and "why didn't it
//! tell me?" is unanswerable without a record of which hook events arrived and
//! what the tray made of them. Volume is a line or two per turn, so it is always
//! on.
//!
//! Hand-rolled rather than `log` + a backend: it needs about fifty lines.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static SINK: Mutex<Option<File>> = Mutex::new(None);

/// Rotate at 1 MB. A tray app can run for months, and an unbounded log on
/// someone else's disk is not our call to make.
const MAX_BYTES: u64 = 1_000_000;

pub fn path() -> Option<PathBuf> {
    crate::config::Config::dir().map(|d| d.join("suavez.log"))
}

/// `rotate` is for the tray only. Hook processes come and go many times a
/// minute and several can run at once; letting them rename the file would race.
pub fn init(rotate: bool) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Keep one previous generation, so a crash loop cannot erase the run that
    // actually explains it.
    if rotate
        && std::fs::metadata(&p)
            .map(|m| m.len() > MAX_BYTES)
            .unwrap_or(false)
    {
        let _ = std::fs::rename(&p, p.with_extension("log.1"));
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(&p) {
        if let Ok(mut sink) = SINK.lock() {
            *sink = Some(f);
        }
    }
}

/// Local wall-clock time, to the millisecond. Local rather than UTC because the
/// log is read by a human next to a clock on the same wall.
#[cfg(windows)]
fn stamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(target_os = "macos")]
fn stamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as libc::time_t;
    let millis = dur.subsec_millis();
    // localtime_r turns the UTC time_t into the process's local calendar time,
    // honouring the current zone and DST without pulling in a date crate.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        millis
    )
}

pub fn write(level: &str, msg: &str) {
    // Formatted up front so the line reaches the file in a single write: the
    // listener thread logs too, and so do hook processes, appending to the same
    // file.
    let line = format!("{} {:<5} {}\n", stamp(), level, msg);
    let Ok(mut sink) = SINK.lock() else { return };
    if let Some(f) = sink.as_mut() {
        // A failed write is ignored on purpose: logging must never be the
        // reason the app misbehaves.
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

macro_rules! linfo  { ($($a:tt)*) => { $crate::log::write("INFO",  &format!($($a)*)) } }
macro_rules! lwarn  { ($($a:tt)*) => { $crate::log::write("WARN",  &format!($($a)*)) } }
macro_rules! lerror { ($($a:tt)*) => { $crate::log::write("ERROR", &format!($($a)*)) } }
pub(crate) use {lerror, linfo, lwarn};
