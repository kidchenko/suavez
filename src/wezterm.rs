//! Pane-level precision for WezTerm.
//!
//! Every program WezTerm starts gets `WEZTERM_PANE` (which pane it is in) and
//! `WEZTERM_UNIX_SOCKET` (how to reach that WezTerm), and the hook inherits
//! both. With them, the tray can ask WezTerm two things through its CLI:
//!
//! - `list-clients`: which pane has focus. "Its terminal is in front" then
//!   means the session's own pane, so a finish in another tab still notifies.
//! - `activate-pane`: switch to the session's tab, so clicking a session or
//!   its notification lands on that session rather than whatever tab was
//!   showing.

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
    let out = cli(pane, &["list-clients", "--format", "json"])?;
    focused_in(&out, pane.id)
}

/// Switch WezTerm to `pane`'s tab.
pub fn activate(pane: &Pane) -> Result<(), String> {
    let id = pane.id.to_string();
    cli(pane, &["activate-pane", "--pane-id", &id])
        .map(|_| ())
        .ok_or_else(|| format!("WezTerm would not switch to pane {id}"))
}

/// Run `wezterm cli <args>` against the pane's WezTerm and return its stdout,
/// or `None` if it failed or took too long.
fn cli(pane: &Pane, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(&pane.cli);
    cmd.arg("cli")
        .args(args)
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
    Some(out)
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
