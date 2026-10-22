//! Connecting to coding agents: Sua Vez's entries in each agent's hooks config.
//!
//! | agent       | file                                                   |
//! |-------------|--------------------------------------------------------|
//! | Claude Code | `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`)    |
//! | Codex       | `~/.codex/hooks.json` (or `$CODEX_HOME`)               |
//! | Gemini CLI  | `~/.gemini/settings.json`                              |
//!
//! Devin has no entry of its own: it loads the hooks in Claude Code's settings
//! by default, so connecting Claude Code connects Devin too. The hook tells the
//! two apart by the `DEVIN_PROJECT_DIR` variable Devin sets.
//!
//! Every one of those files is the user's, so the edit is surgical: parse, add
//! or remove only the entries whose command is this app's `hook` subcommand,
//! and write it back with every other key where it was. A file that does not
//! parse is left alone rather than "repaired". Agents read hooks when a session
//! starts, so a change reaches new sessions, not ones already open.

use crate::agent::Agent;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub struct Integration {
    pub agent: Agent,
    /// The events to register, and the matcher each takes, if any.
    events: &'static [(&'static str, Option<&'static str>)],
    /// Hooks block the agent while they run. `suavez hook` takes milliseconds;
    /// this only bounds the damage if something goes badly wrong. In the
    /// agent's own unit.
    timeout: u64,
}

pub const INTEGRATIONS: [Integration; 3] = [
    Integration {
        agent: Agent::Claude,
        events: &[
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            ("PostToolUse", Some("*")),
            ("PermissionRequest", Some("*")),
            ("Notification", None),
            ("Stop", None),
            ("SessionEnd", None),
        ],
        timeout: 10,
    },
    Integration {
        // No matchers: an omitted one matches every tool, and Codex's are
        // regular expressions, where a bare "*" would not parse.
        agent: Agent::Codex,
        events: &[
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            ("PostToolUse", None),
            ("PermissionRequest", None),
            ("Stop", None),
            ("Interrupt", None),
            ("SessionEnd", None),
        ],
        // Seconds, and Codex caps SessionEnd and Interrupt at 3.
        timeout: 3,
    },
    Integration {
        agent: Agent::Gemini,
        events: &[
            ("SessionStart", None),
            ("BeforeAgent", None),
            ("AfterTool", Some("*")),
            ("Notification", None),
            ("AfterAgent", None),
            ("SessionEnd", None),
        ],
        // Gemini counts in milliseconds.
        timeout: 10_000,
    },
];

pub fn get(agent: Agent) -> Option<&'static Integration> {
    INTEGRATIONS.iter().find(|i| i.agent == agent)
}

fn home() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    Some(PathBuf::from(home))
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
}

/// Where Devin keeps its own config, which is how we know it is installed.
fn devin_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("APPDATA").map(|d| d.join("devin"))
    } else {
        home().map(|h| h.join(".config").join("devin"))
    }
}

impl Integration {
    /// The agent's config folder. Its existence is the sign the agent is
    /// installed.
    fn dir(&self) -> Option<PathBuf> {
        match self.agent {
            Agent::Claude => env_dir("CLAUDE_CONFIG_DIR").or_else(|| Some(home()?.join(".claude"))),
            Agent::Codex => env_dir("CODEX_HOME").or_else(|| Some(home()?.join(".codex"))),
            Agent::Gemini => Some(home()?.join(".gemini")),
            Agent::Devin => None,
        }
    }

    pub fn settings_path(&self) -> Option<PathBuf> {
        let file = match self.agent {
            Agent::Codex => "hooks.json",
            _ => "settings.json",
        };
        self.dir().map(|d| d.join(file))
    }

    /// Whether the agent looks installed. Claude Code's settings also serve
    /// Devin, so either one being there counts.
    pub fn available(&self) -> bool {
        let here = self.dir().map(|d| d.is_dir()).unwrap_or(false);
        here || (self.agent == Agent::Claude && devin_dir().map(|d| d.is_dir()).unwrap_or(false))
    }

    /// "Claude Code and Devin" where Devin rides along.
    pub fn label(&self) -> String {
        if self.agent == Agent::Claude && devin_dir().map(|d| d.is_dir()).unwrap_or(false) {
            "Claude Code and Devin".into()
        } else {
            self.agent.product().into()
        }
    }

    /// The command the agent runs. Forward slashes on Windows too: agents may
    /// run hooks through Git Bash, where backslashes are escapes, and Windows
    /// itself is happy with either. Claude Code's has no `--agent`, so Devin
    /// can share it and tell itself apart by environment.
    pub fn command(&self) -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let exe = exe.to_string_lossy().replace('\\', "/");
        Some(match self.agent {
            Agent::Claude => format!("\"{exe}\" hook"),
            other => format!("\"{exe}\" hook --agent {}", other.slug()),
        })
    }

    pub fn is_installed(&self) -> bool {
        let Some(settings) = self.read().ok().flatten() else {
            return false;
        };
        let event = self.events.last().map(|(e, _)| *e).unwrap_or("");
        settings
            .get("hooks")
            .and_then(|h| h.get(event))
            .and_then(Value::as_array)
            .map(|groups| groups.iter().any(group_has_ours))
            .unwrap_or(false)
    }

    /// If connected, but through a different copy of the binary (an upgrade to
    /// a new folder, a dev build swapped for the installed one) or with an
    /// older set of events, rewrite the entries for this one. Returns whether
    /// anything changed.
    pub fn repoint(&self) -> Result<bool, String> {
        if !self.is_installed() {
            return Ok(false);
        }
        let (Some(cmd), Some(settings)) = (self.command(), self.read()?) else {
            return Ok(false);
        };
        let mut wanted = settings.clone();
        with_ours(&mut wanted, Some(&cmd), self.events, self.timeout)?;
        if wanted == settings {
            return Ok(false);
        }
        self.write(&wanted).map(|()| true)
    }

    pub fn install(&self) -> Result<(), String> {
        let cmd = self
            .command()
            .ok_or("could not resolve the executable path")?;
        let mut settings = self.read()?.unwrap_or_else(|| json!({}));
        with_ours(&mut settings, Some(&cmd), self.events, self.timeout)?;
        self.write(&settings)
    }

    pub fn uninstall(&self) -> Result<(), String> {
        let Some(mut settings) = self.read()? else {
            return Ok(());
        };
        let before = settings.clone();
        with_ours(&mut settings, None, self.events, self.timeout)?;
        if settings == before {
            return Ok(());
        }
        self.write(&settings)
    }

    /// `Ok(None)` for no file yet, which is not an error: install creates it.
    fn read(&self) -> Result<Option<Value>, String> {
        let path = self
            .settings_path()
            .ok_or("no home directory in environment")?;
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

    fn write(&self, settings: &Value) -> Result<(), String> {
        let path = self
            .settings_path()
            .ok_or("no home directory in environment")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        text.push('\n');
        let tmp = path.with_extension("json.suavez-tmp");
        std::fs::write(&tmp, text)
            .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("could not replace {}: {e}", path.display())
        })
    }
}

/// Every integration that is connected.
pub fn installed() -> Vec<&'static Integration> {
    INTEGRATIONS.iter().filter(|i| i.is_installed()).collect()
}

/// Ours, wherever the binary lived when it was installed: a moved or upgraded
/// copy still recognises the old entry and replaces it.
fn is_ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .map(|c| {
            let c = c.to_ascii_lowercase();
            c.contains("suavez") && (c.trim_end().ends_with(" hook") || c.contains(" hook --agent"))
        })
        .unwrap_or(false)
}

fn group_has_ours(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hs| hs.iter().any(is_ours))
        .unwrap_or(false)
}

/// Strip every entry of ours from every event, then, given a command, add one
/// fresh entry per event. Pure, so it can be tested against real-looking
/// settings.
fn with_ours(
    settings: &mut Value,
    cmd: Option<&str>,
    events: &[(&str, Option<&str>)],
    timeout: u64,
) -> Result<(), String> {
    let root = settings
        .as_object_mut()
        .ok_or("the settings file is not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .ok_or("the settings file has a \"hooks\" that is not an object")?;

    // Every event, not just today's list: an event a newer version stopped
    // using still gets cleaned out.
    for groups in hooks.values_mut() {
        if let Value::Array(groups) = groups {
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
        for (event, matcher) in events {
            let mut group = Map::new();
            if let Some(m) = matcher {
                group.insert("matcher".into(), json!(m));
            }
            group.insert(
                "hooks".into(),
                json!([{ "type": "command", "command": cmd, "timeout": timeout }]),
            );
            let groups = hooks.entry(*event).or_insert_with(|| json!([]));
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
    fn claude() -> &'static Integration {
        get(Agent::Claude).unwrap()
    }
    fn ours(s: &mut Value, cmd: Option<&str>) -> Result<(), String> {
        let i = claude();
        with_ours(s, cmd, i.events, i.timeout)
    }

    #[test]
    fn install_keeps_everything_else_in_place() {
        let mut s: Value = serde_json::from_str(
            r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"model":"opus"}"#,
        )
        .unwrap();
        ours(&mut s, Some(CMD)).unwrap();

        let keys: Vec<&String> = s.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "hooks", "model"], "key order preserved");
        let stop = s["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "the user's own Stop hook survives");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        assert_eq!(s["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert_eq!(s["hooks"]["PermissionRequest"][0]["matcher"], "*");
        assert!(s["hooks"]["Stop"][1].get("matcher").is_none());
    }

    #[test]
    fn reinstall_replaces_rather_than_duplicates() {
        let mut s = json!({});
        ours(&mut s, Some("\"/old/place/suavez\" hook")).unwrap();
        ours(&mut s, Some(CMD)).unwrap();
        let stop = s["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], CMD);
    }

    #[test]
    fn events_dropped_by_a_newer_version_are_cleaned_too() {
        let mut s =
            json!({"hooks": {"OldEvent": [{"hooks": [{"type": "command", "command": CMD}]}]}});
        ours(&mut s, None).unwrap();
        assert_eq!(s, json!({}));
    }

    #[test]
    fn uninstall_leaves_no_trace_but_keeps_the_users_hooks() {
        let mut s = json!({"theme": "dark"});
        ours(&mut s, Some(CMD)).unwrap();
        ours(&mut s, None).unwrap();
        assert_eq!(s, json!({"theme": "dark"}));

        let mut s = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": "say done"},
            {"type": "command", "command": CMD}
        ]}]}});
        ours(&mut s, None).unwrap();
        assert_eq!(
            s,
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}})
        );
    }

    #[test]
    fn each_agent_gets_its_own_events_and_units() {
        let codex = get(Agent::Codex).unwrap();
        let mut s = json!({});
        with_ours(
            &mut s,
            Some("\"/x/suavez\" hook --agent codex"),
            codex.events,
            codex.timeout,
        )
        .unwrap();
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0]["timeout"], 3);
        assert!(s["hooks"]["PostToolUse"][0].get("matcher").is_none());

        let gemini = get(Agent::Gemini).unwrap();
        let mut s = json!({});
        with_ours(
            &mut s,
            Some("\"/x/suavez\" hook --agent gemini"),
            gemini.events,
            gemini.timeout,
        )
        .unwrap();
        assert_eq!(s["hooks"]["AfterAgent"][0]["hooks"][0]["timeout"], 10_000);
        assert!(s["hooks"].get("Stop").is_none());
        with_ours(&mut s, None, gemini.events, gemini.timeout).unwrap();
        assert_eq!(s, json!({}), "--agent commands are recognised as ours");
    }

    #[test]
    fn refuses_shapes_it_does_not_understand() {
        assert!(ours(&mut json!([]), Some(CMD)).is_err());
        assert!(ours(&mut json!({"hooks": 3}), Some(CMD)).is_err());
        assert!(ours(&mut json!({"hooks": {"Stop": {}}}), Some(CMD)).is_err());
    }
}
