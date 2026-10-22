//! Pane-level focus for WezTerm.
//!
//! Every program WezTerm starts gets `WEZTERM_PANE` (which pane it is in) and
//! `WEZTERM_UNIX_SOCKET` (how to reach that WezTerm), and the hook inherits
//! both. `wezterm cli list-clients` then reports which pane has focus. So for
//! WezTerm, "its terminal is in front" can mean the session's own pane, not
//! any WezTerm window: a finish in another tab still notifies.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Pane {
    pub id: u64,
    /// The `wezterm` CLI next to the GUI that owns the pane.
    pub cli: String,
    /// That GUI's socket, so the CLI asks the right instance.
    pub socket: String,
}

/// The pane this process runs in, from the environment the hook inherits.
pub fn current() -> Option<Pane> {
    let id = std::env::var("WEZTERM_PANE").ok()?.trim().parse().ok()?;
    let dir = std::env::var("WEZTERM_EXECUTABLE_DIR").ok()?;
    let exe = if cfg!(windows) {
        "wezterm.exe"
    } else {
        "wezterm"
    };
    let cli = std::path::Path::new(&dir).join(exe);
    Some(Pane {
        id,
        cli: cli.to_string_lossy().into_owned(),
        socket: std::env::var("WEZTERM_UNIX_SOCKET").unwrap_or_default(),
    })
}

/// How long the tray waits for the CLI. It answers in milliseconds; this is
/// for a WezTerm that has hung, which must not hang the tray with it.
const PATIENCE: Duration = Duration::from_millis(800);

/// Whether `pane` has focus. `None` when WezTerm could not be asked, so the
/// caller can fall back to "is WezTerm in front at all".
pub fn is_focused(pane: &Pane) -> Option<bool> {
    let mut cmd = std::process::Command::new(&pane.cli);
    cmd.args(["cli", "list-clients", "--format", "json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if !pane.socket.is_empty() {
        cmd.env("WEZTERM_UNIX_SOCKET", &pane.socket);
    }
    #[cfg(windows)]
    {
        // The CLI is a console program: without this, every check would flash
        // a console window.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > PATIENCE => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    focused_in(&out, pane.id)
}

/// Pure half of `is_focused`: does any client in `list-clients` JSON have
/// `id` as its focused pane?
fn focused_in(json: &str, id: u64) -> Option<bool> {
    let clients: serde_json::Value = serde_json::from_str(json).ok()?;
    let clients = clients.as_array()?;
    Some(
        clients
            .iter()
            .any(|c| c.get("focused_pane_id").and_then(serde_json::Value::as_u64) == Some(id)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[{"username":"jose","hostname":"x","pid":13168,
        "workspace":"default","focused_pane_id":2}]"#;

    /// Asks the WezTerm this runs in. By hand, from a WezTerm pane:
    /// `cargo test live_wezterm -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_wezterm() {
        let pane = current().expect("run this inside WezTerm");
        let started = Instant::now();
        let focused = is_focused(&pane);
        println!(
            "pane {} focused: {focused:?} in {:?}",
            pane.id,
            started.elapsed()
        );
        assert!(focused.is_some(), "the CLI did not answer");
    }

    #[test]
    fn reads_the_focused_pane() {
        assert_eq!(focused_in(SAMPLE, 2), Some(true));
        assert_eq!(focused_in(SAMPLE, 0), Some(false));
        assert_eq!(focused_in("[]", 2), Some(false));
        assert_eq!(focused_in("not json", 2), None);
    }
}
