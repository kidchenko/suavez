//! Connecting to Claude Code: Sua Vez's entries in the `hooks` section of the
//! user's `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`).
//!
//! That file is the user's, so the edit is surgical: parse, add or remove only
//! the entries whose command is this app's `hook` subcommand, and write it back
//! with every other key where it was. A file that does not parse is left alone
//! rather than "repaired". Claude Code reads hooks when a session starts, so a
//! change reaches new sessions, not ones already open.

use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// The events Sua Vez listens to, and whether each takes a tool matcher.
const EVENTS: [(&str, bool); 6] = [
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("PostToolUse", true),
    ("Notification", false),
    ("Stop", false),
    ("SessionEnd", false),
];

/// Hooks block the turn while they run. `suavez hook` takes milliseconds; this
/// only bounds the damage of something going badly wrong.
const TIMEOUT_SECS: u64 = 10;

pub fn settings_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    Some(PathBuf::from(home).join(".claude").join("settings.json"))
}

/// The command Claude Code runs. Forward slashes on Windows too: Claude Code
/// may run hooks through Git Bash, where backslashes are escapes, and Windows
/// itself is happy with either.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.to_string_lossy().replace('\\', "/");
    Some(format!("\"{exe}\" hook"))
}

/// Ours, wherever the binary lived when it was installed: a moved or
/// upgraded copy still recognises the old entry and replaces it.
fn is_ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .map(|c| {
            let c = c.to_ascii_lowercase();
            c.contains("suavez") && c.trim_end().ends_with(" hook")
        })
        .unwrap_or(false)
}

pub fn is_installed() -> bool {
    let Some(settings) = read().ok().flatten() else {
        return false;
    };
    settings
        .get("hooks")
        .and_then(|h| h.get("Stop"))
        .and_then(Value::as_array)
        .map(|groups| groups.iter().any(group_has_ours))
        .unwrap_or(false)
}

fn group_has_ours(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hs| hs.iter().any(is_ours))
        .unwrap_or(false)
}

/// `Ok(None)` for no file yet, which is not an error: install creates it.
fn read() -> Result<Option<Value>, String> {
    let path = settings_path().ok_or("no home directory in environment")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(text)
        .map(Some)
        .map_err(|e| format!("{} is not valid JSON ({e}); left it alone", path.display()))
}

fn write(settings: &Value) -> Result<(), String> {
    let path = settings_path().ok_or("no home directory in environment")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    let tmp = path.with_extension("json.suavez-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {}: {e}", path.display())
    })
}

/// If connected, but through a different copy of the binary (an upgrade to a
/// new folder, a dev build swapped for the installed one), point the hooks at
/// this one. Returns whether anything changed.
pub fn repoint() -> Result<bool, String> {
    let Some(cmd) = command() else {
        return Ok(false);
    };
    let Some(settings) = read()? else {
        return Ok(false);
    };
    let hooks = settings.get("hooks");
    let current = |event: &str| {
        hooks
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .map(|groups| {
                groups.iter().any(|g| {
                    g.get("hooks")
                        .and_then(Value::as_array)
                        .map(|hs| hs.iter().any(|h| h.get("command") == Some(&json!(cmd))))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    };
    if !is_installed() || EVENTS.iter().all(|(e, _)| current(e)) {
        return Ok(false);
    }
    install().map(|()| true)
}

pub fn install() -> Result<(), String> {
    let cmd = command().ok_or("could not resolve the executable path")?;
    let mut settings = read()?.unwrap_or_else(|| json!({}));
    with_ours(&mut settings, Some(&cmd))?;
    write(&settings)
}

pub fn uninstall() -> Result<(), String> {
    let Some(mut settings) = read()? else {
        return Ok(());
    };
    with_ours(&mut settings, None)?;
    write(&settings)
}

/// Strip every entry of ours, then, given a command, add one fresh entry per
/// event. Pure, so it can be tested against real-looking settings.
fn with_ours(settings: &mut Value, cmd: Option<&str>) -> Result<(), String> {
    let root = settings
        .as_object_mut()
        .ok_or("settings.json is not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .ok_or("settings.json has a \"hooks\" that is not an object")?;

    for (event, _) in EVENTS {
        if let Some(Value::Array(groups)) = hooks.get_mut(event) {
            for group in groups.iter_mut() {
                if let Some(Value::Array(hs)) = group.get_mut("hooks") {
                    hs.retain(|h| !is_ours(h));
                }
            }
            groups.retain(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .map(|hs| !hs.is_empty())
                    .unwrap_or(true)
            });
        }
    }

    if let Some(cmd) = cmd {
        for (event, matcher) in EVENTS {
            let mut group = Map::new();
            if matcher {
                group.insert("matcher".into(), json!("*"));
            }
            group.insert(
                "hooks".into(),
                json!([{ "type": "command", "command": cmd, "timeout": TIMEOUT_SECS }]),
            );
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            match groups.as_array_mut() {
                Some(a) => a.push(Value::Object(group)),
                None => return Err(format!("hooks.{event} is not a list; left it alone")),
            }
        }
    }

    // Leave no empty husks behind on the way out.
    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "\"C:/Apps/SuaVez/suavez.exe\" hook";

    #[test]
    fn install_keeps_everything_else_in_place() {
        let mut s: Value = serde_json::from_str(
            r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"model":"opus"}"#,
        )
        .unwrap();
        with_ours(&mut s, Some(CMD)).unwrap();

        let keys: Vec<&String> = s.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "hooks", "model"], "key order preserved");
        let stop = s["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "the user's own Stop hook survives");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        assert_eq!(s["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert!(s["hooks"]["Stop"][1].get("matcher").is_none());
    }

    #[test]
    fn reinstall_replaces_rather_than_duplicates() {
        let mut s = json!({});
        with_ours(&mut s, Some("\"/old/place/suavez\" hook")).unwrap();
        with_ours(&mut s, Some(CMD)).unwrap();
        let stop = s["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], CMD);
    }

    #[test]
    fn uninstall_leaves_no_trace_but_keeps_the_users_hooks() {
        let mut s = json!({"theme": "dark"});
        with_ours(&mut s, Some(CMD)).unwrap();
        with_ours(&mut s, None).unwrap();
        assert_eq!(s, json!({"theme": "dark"}));

        let mut s = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "say done"},
            {"type": "command", "command": CMD}
        ]}]}});
        with_ours(&mut s, None).unwrap();
        assert_eq!(
            s,
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}})
        );
    }

    #[test]
    fn refuses_shapes_it_does_not_understand() {
        assert!(with_ours(&mut json!([]), Some(CMD)).is_err());
        assert!(with_ours(&mut json!({"hooks": 3}), Some(CMD)).is_err());
        assert!(with_ours(&mut json!({"hooks": {"Stop": {}}}), Some(CMD)).is_err());
    }
}
