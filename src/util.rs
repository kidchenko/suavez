//! Small helpers shared across modules.

/// Seconds since the Unix epoch, saturating at 0 on a pre-1970 clock.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "45s", "12m", "1h 05m": how long something took or has been going on.
pub fn span(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        format!("{mins}m")
    } else {
        format!("{}h {:02}m", mins / 60, mins % 60)
    }
}

/// "just now", "3m ago", "1h 05m ago": for menu rows, which are rebuilt at
/// most once a minute, so seconds would only ever be wrong.
pub fn ago(secs: u64) -> String {
    if secs < 60 {
        "just now".into()
    } else {
        format!("{} ago", span(secs))
    }
}

/// Whether this copy runs from somewhere that won't last: the mounted disk
/// image, or the random folder Gatekeeper runs a quarantined app from (App
/// Translocation). Hooks pointed here break the moment the image is ejected
/// or the app relaunched, so nothing should be connected from it.
pub fn running_from_temporary_place() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    std::env::current_exe()
        .map(|p| is_temporary_place(&p.to_string_lossy()))
        .unwrap_or(false)
}

fn is_temporary_place(exe: &str) -> bool {
    exe.starts_with("/Volumes/") || exe.contains("/AppTranslocation/")
}

/// Full path to the running executable, quoted for the Run key.
#[cfg(windows)]
pub fn exe_path_quoted() -> Option<String> {
    let p = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", p.display()))
}

/// Borrow the parent terminal's console, so `suavez status` and friends can
/// print. A `windows_subsystem = "windows"` binary starts without one. Harmless
/// when launched from Explorer, where there is no parent console to borrow.
#[cfg(windows)]
pub fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
pub fn attach_console() {}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use windows::core::PCWSTR;

    /// Null-terminated UTF-16. Keep the buffer alive while the PCWSTR is used.
    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn pcwstr(buf: &[u16]) -> PCWSTR {
        PCWSTR(buf.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans() {
        assert_eq!(span(5), "5s");
        assert_eq!(span(60), "1m");
        assert_eq!(span(119), "1m");
        assert_eq!(span(3600 + 5 * 60), "1h 05m");
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(600), "10m ago");
    }

    #[test]
    fn temporary_places() {
        assert!(is_temporary_place(
            "/Volumes/SuaVez/SuaVez.app/Contents/MacOS/suavez"
        ));
        assert!(is_temporary_place(
            "/private/var/folders/x/T/AppTranslocation/1234/d/SuaVez.app/Contents/MacOS/suavez"
        ));
        assert!(!is_temporary_place(
            "/Applications/SuaVez.app/Contents/MacOS/suavez"
        ));
    }
}
