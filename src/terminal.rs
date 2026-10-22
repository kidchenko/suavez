//! Bringing a session's terminal to the front.
//!
//! The hook process is a descendant of the terminal Claude Code runs in:
//! terminal → shell → claude → (shell) → `suavez hook`. So the first time a
//! session reports, the hook walks up its own ancestors to the first one that
//! is an app with a window (Windows Terminal, VS Code, iTerm, Terminal.app...)
//! and records it with the session. Clicking the session in the menu brings
//! that app forward.
//!
//! It finds the app, not the tab: with three Windows Terminal tabs, or two
//! VS Code windows, you get the front-most window of that app.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Terminal {
    pub pid: u32,
    /// The executable name on Windows (`WindowsTerminal.exe`), the `.app`
    /// bundle path on macOS. Also guards against a recycled pid.
    pub name: String,
}

pub use imp::{detect, focus};

/// Walk up from `start` through `parents` (pid → (parent pid, name)) and
/// return the first ancestor `is_app` accepts. Stops at `stop` names, which are
/// the desktop shell: past that point any window found is not the terminal.
fn walk(
    parents: &HashMap<u32, (u32, String)>,
    start: u32,
    stop: &[&str],
    is_app: impl Fn(u32, &str) -> bool,
) -> Option<(u32, String)> {
    let mut pid = start;
    // A cycle or a recycled pid can make the chain loop; real ones are short.
    for _ in 0..24 {
        let (parent, name) = parents.get(&pid)?;
        if stop.iter().any(|s| name.eq_ignore_ascii_case(s)) {
            return None;
        }
        if is_app(pid, name) {
            return Some((pid, name.clone()));
        }
        if *parent == pid || *parent == 0 {
            return None;
        }
        pid = *parent;
    }
    None
}

/// "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/
/// Contents/MacOS/Code Helper" → "/Applications/Visual Studio Code.app": the
/// outermost bundle is the app you see in the Dock.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn outer_bundle(path: &str) -> Option<&str> {
    path.find(".app/").map(|i| &path[..i + 4])
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use super::{walk, Terminal};
    use std::collections::{HashMap, HashSet};
    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
    };

    /// Where the chain has left the terminal behind.
    const STOP: &[&str] = &["explorer.exe", "svchost.exe", "services.exe", "wininit.exe"];

    /// pid → (parent pid, executable name).
    fn snapshot() -> HashMap<u32, (u32, String)> {
        let mut out = HashMap::new();
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = unsafe { Process32FirstW(snap, &mut entry) }.is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            out.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ),
            );
            ok = unsafe { Process32NextW(snap, &mut entry) }.is_ok();
        }
        unsafe {
            let _ = CloseHandle(snap);
        }
        out
    }

    /// Visible, titled, top-level windows (what Alt+Tab shows), front-most
    /// first, as (window, owning pid).
    fn app_windows() -> Vec<(HWND, u32)> {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let out = &mut *(lparam.0 as *mut Vec<(HWND, u32)>);
            let owned = GetWindow(hwnd, GW_OWNER)
                .map(|h| !h.is_invalid())
                .unwrap_or(false);
            if IsWindowVisible(hwnd).as_bool() && !owned && GetWindowTextLengthW(hwnd) > 0 {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                out.push((hwnd, pid));
            }
            BOOL(1)
        }
        let mut out: Vec<(HWND, u32)> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }

    pub fn detect() -> Option<Terminal> {
        let parents = snapshot();
        let windowed: HashSet<u32> = app_windows().into_iter().map(|(_, p)| p).collect();
        let me = std::process::id();
        let start = parents.get(&me)?.0;
        walk(&parents, start, STOP, |pid, _| windowed.contains(&pid))
            .map(|(pid, name)| Terminal { pid, name })
    }

    pub fn focus(t: &Terminal) -> Result<(), String> {
        let alive = snapshot()
            .get(&t.pid)
            .map(|(_, n)| n.eq_ignore_ascii_case(&t.name))
            .unwrap_or(false);
        if !alive {
            return Err(format!("{} (pid {}) is no longer running", t.name, t.pid));
        }
        let Some(&(hwnd, _)) = app_windows().iter().find(|(_, p)| *p == t.pid) else {
            return Err(format!("{} has no window to bring forward", t.name));
        };
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd).as_bool() {
                return Ok(());
            }
            // Windows only lets the process that last had input take the
            // foreground. A synthetic Alt press counts as input, which is the
            // usual way round it; the key itself does nothing on its own.
            let key = |flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            };
            let _ = SendInput(
                &[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)],
                std::mem::size_of::<INPUT>() as i32,
            );
            if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err(format!("Windows would not bring {} forward", t.name))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
//
// `ps` rather than libproc: this runs once per session, in the hook, and a
// few milliseconds of fork is cheaper than FFI nobody can test without a Mac.
// Activation is `open <bundle>`, which needs no Accessibility permission.
#[cfg(target_os = "macos")]
mod imp {
    use super::{outer_bundle, walk, Terminal};
    use std::collections::HashMap;

    const STOP: &[&str] = &["/sbin/launchd", "launchd"];

    fn snapshot() -> HashMap<u32, (u32, String)> {
        let mut out = HashMap::new();
        let Ok(o) = std::process::Command::new("/bin/ps")
            .args(["-A", "-o", "pid=,ppid=,comm="])
            .output()
        else {
            return out;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut rest = line.trim_start();
            let mut field = || {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let f = &rest[..end];
                rest = rest[end..].trim_start();
                f.parse::<u32>().ok()
            };
            if let (Some(pid), Some(ppid)) = (field(), field()) {
                out.insert(pid, (ppid, rest.to_string()));
            }
        }
        out
    }

    pub fn detect() -> Option<Terminal> {
        let parents = snapshot();
        let start = std::os::unix::process::parent_id();
        walk(&parents, start, STOP, |_, name| {
            outer_bundle(name).is_some()
        })
        .map(|(pid, name)| Terminal {
            pid,
            name: outer_bundle(&name).unwrap_or(&name).to_string(),
        })
    }

    pub fn focus(t: &Terminal) -> Result<(), String> {
        std::process::Command::new("/usr/bin/open")
            .arg(&t.name)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open {}: {e}", t.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> HashMap<u32, (u32, String)> {
        [
            (40, (30, "suavez.exe")),
            (30, (20, "bash.exe")),
            (20, (10, "claude.exe")),
            (10, (5, "pwsh.exe")),
            (5, (1, "WindowsTerminal.exe")),
            (1, (0, "explorer.exe")),
        ]
        .into_iter()
        .map(|(k, (p, n))| (k, (p, n.to_string())))
        .collect()
    }

    #[test]
    fn finds_the_first_ancestor_with_a_window() {
        let found = walk(&tree(), 30, &["explorer.exe"], |pid, _| {
            pid == 5 || pid == 1
        });
        assert_eq!(found, Some((5, "WindowsTerminal.exe".into())));
    }

    #[test]
    fn stops_at_the_desktop_shell() {
        let found = walk(&tree(), 30, &["explorer.exe"], |pid, _| pid == 1);
        assert_eq!(found, None);
    }

    #[test]
    fn survives_cycles() {
        let mut t = tree();
        t.insert(5, (30, "WindowsTerminal.exe".into()));
        assert_eq!(walk(&t, 30, &[], |_, _| false), None);
    }

    #[test]
    fn outermost_bundle_wins() {
        assert_eq!(
            outer_bundle("/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"),
            Some("/Applications/Visual Studio Code.app")
        );
        assert_eq!(outer_bundle("/bin/zsh"), None);
    }
}
