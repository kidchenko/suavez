//! Notifications, for the two moments Sua Vez exists for: an agent finished a
//! turn, or an agent is blocked waiting on you. Everything else is visible in the
//! bubble and its menu.
//!
//! Best-effort throughout: a notification that cannot be shown (Focus Assist,
//! Do Not Disturb, policy) is logged and otherwise ignored.

/// What a click on the notification does. Called on a system thread.
pub type OnClick = Box<dyn Fn() + Send + 'static>;

pub fn show(title: &str, line1: &str, line2: Option<&str>) {
    imp::show(title, line1, line2, None);
}

/// A notification that does `on_click` when clicked. On Windows that is
/// certain while its banner is on screen; a click from the notification centre
/// later goes through Windows, which may only relaunch the app. macOS shows it
/// all the same, but clicking opens Script Editor (see below), so the click does
/// nothing useful there yet.
pub fn show_clickable(title: &str, line1: &str, line2: Option<&str>, on_click: OnClick) {
    imp::show(title, line1, line2, Some(on_click));
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
//
// `Toast::POWERSHELL_APP_ID` is a borrowed identity: Windows will not show a
// toast for an application it has never heard of, and registering a real
// AppUserModelID means writing a Start Menu shortcut carrying it. That is the
// installer's job, not the app's, so an unpackaged run borrows PowerShell's
// registration and shows up under its name. Once the installer lands, the
// shortcut is detected and the toasts get the right name and icon.
#[cfg(windows)]
mod imp {
    use crate::log::{linfo, lwarn};
    use tauri_winrt_notification::{Duration, Toast};

    /// Matches the `AppUserModelID` the installer stamps on the Start Menu
    /// shortcut. The two must stay in step: Windows resolves a toast's name and
    /// icon by looking this string up against registered shortcuts, and an id
    /// with no shortcut behind it shows nothing at all.
    const APP_ID: &str = "kidchenko.SuaVez";

    /// The shortcut is the registration: Windows silently drops toasts for an
    /// AUMID with no shortcut behind it. A portable copy with no installer
    /// borrows PowerShell's identity instead: wrong name, but a toast.
    fn shortcut_installed() -> bool {
        const REL: &str = r"Microsoft\Windows\Start Menu\Programs\Sua Vez\Sua Vez.lnk";
        ["APPDATA", "PROGRAMDATA"].iter().any(|var| {
            std::env::var_os(var)
                .map(|base| std::path::PathBuf::from(base).join(REL).exists())
                .unwrap_or(false)
        })
    }

    fn app_id() -> String {
        if let Ok(explicit) = std::env::var("SUAVEZ_APP_ID") {
            return explicit;
        }
        if shortcut_installed() {
            APP_ID.to_string()
        } else {
            Toast::POWERSHELL_APP_ID.to_string()
        }
    }

    pub fn show(title: &str, line1: &str, line2: Option<&str>, on_click: Option<super::OnClick>) {
        let mut toast = Toast::new(&app_id())
            .title(title)
            .text1(line1)
            .duration(Duration::Short);
        if let Some(l2) = line2 {
            toast = toast.text2(l2);
        }
        if let Some(f) = on_click {
            toast = toast.on_activated(move |_| {
                f();
                Ok(())
            });
        }
        match toast.show() {
            Ok(()) => linfo!("toast shown: {title} / {line1}"),
            Err(e) => lwarn!("could not show toast: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
//
// `osascript`'s `display notification` is the analogue of borrowing PowerShell's
// identity on Windows: it shows under Script Editor's name rather than Sua Vez's,
// but it needs no bundle registration, no authorization prompt, and no
// UserNotifications plumbing.
#[cfg(target_os = "macos")]
mod imp {
    use crate::log::{linfo, lwarn};

    /// `display notification` cannot carry a click action: clicking one
    /// opens Script Editor, whose identity it borrows. A real click needs
    /// UserNotifications and a signed bundle, which is for later.
    pub fn show(title: &str, line1: &str, line2: Option<&str>, _on_click: Option<super::OnClick>) {
        // Notification anatomy on macOS is title / subtitle / body. We map the
        // Windows title to the title, the first line to the body, and the
        // optional detail to the subtitle.
        let mut script = format!(
            "display notification {} with title {}",
            applescript_string(line1),
            applescript_string(title),
        );
        if let Some(l2) = line2 {
            script.push_str(&format!(" subtitle {}", applescript_string(l2)));
        }

        match std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(&script)
            .spawn()
        {
            Ok(_) => linfo!("notification shown: {title} / {line1}"),
            Err(e) => lwarn!("could not show notification: {e}"),
        }
    }

    /// A double-quoted AppleScript string literal. Backslash and quote are
    /// escaped; newlines and tabs become their AppleScript escapes so a
    /// multi-line body cannot break out of the `-e` argument.
    fn applescript_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}
