//! What each coding agent session is doing, as told by its hooks.
//!
//! The agent (Claude Code, Codex, Gemini CLI or Devin) runs `suavez hook` on a
//! handful of events and pipes it a JSON description on stdin. That short-lived process folds the event into a small
//! file per session under `sessions/`, pokes the tray, and exits. The tray
//! reads the directory back. Files rather than a socket protocol because they
//! survive the tray not running yet, being restarted, or several hooks firing at
//! once, and because `type sessions\*.json` is the whole debugging story.
//!
//! | hook event                          | becomes                               |
//! |-------------------------------------|---------------------------------------|
//! | `SessionStart`                      | `Ready` (a resume keeps the state)    |
//! | `UserPromptSubmit`, `BeforeAgent`   | `Working`; the turn's clock starts    |
//! | `PostToolUse`, `AfterTool`          | `Working` (any prompt was answered)   |
//! | `PermissionRequest`, `Notification` | `NeedsYou`, for a permission prompt   |
//! | `Stop`, `AfterAgent`                | `Done`: your turn                     |
//! | `Interrupt`                         | `Ready`: you stopped it, so you know  |
//! | `SessionEnd`                        | the file is removed                   |
//!
//! The second name in a row is Gemini CLI's; the others share Claude Code's.
//!
//! `Tracker` is the tray's half: it turns successive snapshots into
//! notifications, and remembers which ones you have already seen.

use crate::agent::Agent;
use crate::log::lwarn;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Open, nothing asked yet.
    #[default]
    Ready,
    /// The agent is on it.
    Working,
    /// Blocked on a permission prompt or a question.
    NeedsYou,
    /// Finished the turn; the next move is yours.
    Done,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    pub id: String,
    pub agent: Agent,
    pub cwd: String,
    pub state: State,
    /// When the current state began.
    pub since: u64,
    /// When any hook last reported for this session.
    pub updated: u64,
    /// When the prompt that started the current turn was submitted.
    pub turn_started: u64,
    /// The agent's own words for `NeedsYou`, e.g. "Claude needs your
    /// permission to use Bash".
    pub message: String,
    /// The app the session runs in, for bringing it to the front.
    pub terminal: Option<crate::terminal::Terminal>,
    /// Whether the hook has looked for `terminal` yet. It looks once per
    /// session: a miss would only miss again, and the walk is not free.
    pub terminal_checked: bool,
}

impl Session {
    /// The folder name of the session's working directory: what you would call
    /// the project out loud.
    pub fn project(&self) -> &str {
        self.cwd
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(self.agent.product())
    }

    /// How long the turn that just ended took, for `Done`.
    pub fn turn_secs(&self) -> u64 {
        if self.turn_started == 0 {
            return 0;
        }
        self.since.saturating_sub(self.turn_started)
    }

    /// Identifies one stretch in a state, so "seen" and "announced" apply to
    /// this finish rather than every future one.
    fn episode(&self) -> (String, u64) {
        (self.id.clone(), self.since)
    }

    fn wants_you(&self) -> bool {
        matches!(self.state, State::NeedsYou | State::Done)
    }
}

/// The subset of a hook's stdin that matters here. Everything is optional:
/// agents add fields over time, older versions lack some, and not every agent
/// sends every field.
#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct HookInput {
    pub session_id: String,
    pub cwd: String,
    pub hook_event_name: String,
    pub message: String,
    pub notification_type: String,
    pub tool_name: String,
}

#[derive(PartialEq, Debug)]
pub enum Change {
    Write(Session),
    Remove,
}

/// Gemini CLI's names for the events the others share.
fn canonical(event: &str) -> &str {
    match event {
        "BeforeAgent" => "UserPromptSubmit",
        "BeforeTool" | "AfterTool" => "PostToolUse",
        "AfterAgent" => "Stop",
        other => other,
    }
}

/// Fold one hook event into the session's previous state. Pure.
pub fn fold(prev: Option<Session>, input: &HookInput, agent: Agent, now: u64) -> Change {
    let mut s = prev.clone().unwrap_or_else(|| Session {
        id: input.session_id.clone(),
        since: now,
        ..Session::default()
    });
    s.agent = agent;
    s.updated = now;
    if !input.cwd.is_empty() {
        s.cwd = input.cwd.clone();
    }

    let next = match canonical(&input.hook_event_name) {
        "SessionEnd" => return Change::Remove,
        // A resume, /clear or auto-compact also fires this; only a session we
        // have never heard of starts out Ready.
        "SessionStart" if prev.is_none() => Some(State::Ready),
        "UserPromptSubmit" => {
            s.turn_started = now;
            Some(State::Working)
        }
        "PreToolUse" | "PostToolUse" => {
            if s.turn_started == 0 {
                s.turn_started = now;
            }
            Some(State::Working)
        }
        "Notification" => match notification_kind(input) {
            Kind::Blocked => {
                s.message = input.message.clone();
                Some(State::NeedsYou)
            }
            // "Waiting for your input" a minute after a finish. If the Stop
            // got lost somewhere, this is the next best signal.
            Kind::Idle if s.state == State::Working || s.state == State::Ready => Some(State::Done),
            _ => None,
        },
        "PermissionRequest" => {
            s.message = if input.tool_name.is_empty() {
                format!("{} needs your permission", agent.name())
            } else {
                format!(
                    "{} needs your permission to use {}",
                    agent.name(),
                    input.tool_name
                )
            };
            Some(State::NeedsYou)
        }
        "Stop" => Some(State::Done),
        // You pressed Esc: you are looking at it, so nothing to announce.
        "Interrupt" => Some(State::Ready),
        _ => None,
    };

    if let Some(state) = next {
        if state != s.state {
            s.state = state;
            s.since = now;
        }
        if state != State::NeedsYou {
            s.message.clear();
        }
    }
    Change::Write(s)
}

enum Kind {
    Blocked,
    Idle,
    Other,
}

/// `notification_type` where Claude Code sends it; the message text where it
/// does not.
fn notification_kind(input: &HookInput) -> Kind {
    match input.notification_type.as_str() {
        // Claude Code's, then Gemini CLI's.
        "permission_prompt" | "elicitation_dialog" | "ToolPermission" => return Kind::Blocked,
        "idle_prompt" => return Kind::Idle,
        "" => {}
        _ => return Kind::Other,
    }
    let m = input.message.to_ascii_lowercase();
    if m.contains("permission") || m.contains("approval") {
        Kind::Blocked
    } else if m.contains("waiting for your input") {
        Kind::Idle
    } else {
        Kind::Other
    }
}

// ---------------------------------------------------------------------------
// The sessions directory
// ---------------------------------------------------------------------------

pub fn dir() -> Option<PathBuf> {
    crate::config::Config::dir().map(|d| d.join("sessions"))
}

/// Session ids are UUIDs today. Anything else is squeezed into a safe file
/// name rather than trusted as a path.
fn file_name(id: &str) -> Option<String> {
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(80)
        .collect();
    (!safe.is_empty()).then(|| format!("{safe}.json"))
}

fn read(path: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The `suavez hook` side: fold one event from stdin into its file.
pub fn ingest(stdin: &str, agent: Agent, now: u64) -> Result<(), String> {
    let mut input: HookInput =
        serde_json::from_str(stdin).map_err(|e| format!("hook input is not JSON: {e}"))?;
    // Devin's payload has no cwd; it sets the project folder in the
    // environment instead. Failing both, a hook runs in the project folder.
    if input.cwd.is_empty() {
        input.cwd = std::env::var("DEVIN_PROJECT_DIR")
            .ok()
            .filter(|d| !d.is_empty())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|d| d.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
    }
    let dir = dir().ok_or("no config directory in environment")?;
    let name = file_name(&input.session_id).ok_or("hook input has no session_id")?;
    let path = dir.join(name);

    match fold(read(&path), &input, agent, now) {
        Change::Remove => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("could not remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        },
        Change::Write(mut s) => {
            if !s.terminal_checked {
                s.terminal_checked = true;
                s.terminal = crate::terminal::detect();
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let json = serde_json::to_string(&s).map_err(|e| e.to_string())?;
            // A temp name per process: two hooks for one session can overlap
            // (parallel tool calls), and a shared temp file would interleave.
            let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
            std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("could not write {}: {e}", path.display())
            })
        }
    }
}

/// The tray side: every live session, most urgent first. Sessions quiet for
/// longer than `stale_secs` are deleted on the way: their terminal is gone.
pub fn load_all(now: u64, stale_secs: u64) -> Vec<Session> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read(&path) {
            Some(s) if now.saturating_sub(s.updated) <= stale_secs => out.push(s),
            Some(_) => {
                let _ = std::fs::remove_file(&path);
            }
            // Unreadable is usually a write in flight; the next scan gets it.
            None => {}
        }
    }
    sort(&mut out);
    out
}

pub fn sort(sessions: &mut [Session]) {
    fn rank(s: State) -> u8 {
        match s {
            State::NeedsYou => 0,
            State::Done => 1,
            State::Working => 2,
            State::Ready => 3,
        }
    }
    sessions.sort_by(|a, b| {
        rank(a.state)
            .cmp(&rank(b.state))
            .then(b.since.cmp(&a.since))
    });
}

/// "Forget all sessions": for when a crashed terminal left one stuck on
/// Working. Hooks recreate the live ones on their next event.
pub fn remove_all() {
    let Some(dir) = dir() else { return };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Err(err) = std::fs::remove_file(e.path()) {
                lwarn!("could not remove {}: {err}", e.path().display());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The tray's memory
// ---------------------------------------------------------------------------

/// What the tray shows overall, most urgent first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overall {
    NeedsYou,
    YourTurn,
    Working,
    Idle,
}

#[derive(PartialEq, Debug)]
pub struct Toast {
    pub title: String,
    pub body: String,
}

pub struct Rules {
    pub notify_done: bool,
    pub notify_needs_you: bool,
    pub min_turn_secs: u64,
    pub grace_secs: u64,
    /// No notification while the session's terminal is the app in front.
    pub quiet_when_focused: bool,
}

#[derive(Default)]
pub struct Tracker {
    /// Episodes already announced (or deliberately not), so a rescan never
    /// repeats a toast.
    announced: HashSet<(String, u64)>,
    /// Episodes the user has acknowledged; they no longer colour the icon.
    seen: HashSet<(String, u64)>,
    primed: bool,
}

impl Tracker {
    /// Take a fresh snapshot and return what deserves a notification.
    ///
    /// The first snapshot after launch announces nothing: those finishes
    /// happened while nobody was listening, and a burst of stale toasts at
    /// login helps no one. They still colour the icon until seen.
    ///
    /// `in_front` says whether a session's terminal is the app you are using.
    /// It is only asked at the moment a notification would go out. When it says
    /// yes, the notification is skipped but the bubble still turns: the app is
    /// in front, but the finish may be in a tab you are not looking at.
    pub fn update(
        &mut self,
        sessions: &[Session],
        now: u64,
        rules: &Rules,
        in_front: &dyn Fn(&Session) -> bool,
    ) -> Vec<Toast> {
        let watching = |s: &Session| rules.quiet_when_focused && in_front(s);
        let live: HashSet<(String, u64)> = sessions.iter().map(Session::episode).collect();
        self.announced.retain(|k| live.contains(k));
        self.seen.retain(|k| live.contains(k));

        let mut toasts = Vec::new();
        let primed = std::mem::replace(&mut self.primed, true);
        let names = display_names(sessions);
        for s in sessions.iter().filter(|s| s.wants_you()) {
            let key = s.episode();
            if self.announced.contains(&key) {
                continue;
            }
            if !primed {
                self.announced.insert(key);
                continue;
            }
            let name = &names[&s.id];
            match s.state {
                State::Done => {
                    if rules.notify_done && s.turn_secs() >= rules.min_turn_secs && !watching(s) {
                        toasts.push(Toast {
                            title: format!("Your turn in {name}"),
                            body: format!(
                                "{} finished after {}.",
                                s.agent.name(),
                                crate::util::span(s.turn_secs())
                            ),
                        });
                    }
                    self.announced.insert(key);
                }
                State::NeedsYou => {
                    // Wait out the grace period: answered by then, it was
                    // never news.
                    if now.saturating_sub(s.since) < rules.grace_secs {
                        continue;
                    }
                    if rules.notify_needs_you && !watching(s) {
                        toasts.push(Toast {
                            title: format!("{name} needs you"),
                            body: if s.message.is_empty() {
                                format!("{} is waiting on you.", s.agent.name())
                            } else {
                                s.message.clone()
                            },
                        });
                    }
                    self.announced.insert(key);
                }
                _ => {}
            }
        }
        toasts
    }

    /// When a pending permission prompt's grace period runs out, if any.
    pub fn next_due(&self, sessions: &[Session], rules: &Rules) -> Option<u64> {
        sessions
            .iter()
            .filter(|s| s.state == State::NeedsYou && !self.announced.contains(&s.episode()))
            .map(|s| s.since + rules.grace_secs)
            .min()
    }

    pub fn is_seen(&self, s: &Session) -> bool {
        self.seen.contains(&s.episode())
    }

    pub fn mark_seen(&mut self, s: &Session) {
        if s.wants_you() {
            self.seen.insert(s.episode());
        }
    }

    pub fn overall(&self, sessions: &[Session]) -> Overall {
        let unseen = |state| {
            sessions
                .iter()
                .any(|s| s.state == state && !self.is_seen(s))
        };
        if unseen(State::NeedsYou) {
            Overall::NeedsYou
        } else if unseen(State::Done) {
            Overall::YourTurn
        } else if sessions.iter().any(|s| s.state == State::Working) {
            Overall::Working
        } else {
            Overall::Idle
        }
    }
}

/// Project names, with the agent when it isn't Claude Code, and disambiguated
/// when two sessions of one agent share a folder.
pub fn display_names(sessions: &[Session]) -> HashMap<String, String> {
    let base = |s: &Session| match s.agent {
        Agent::Claude => s.project().to_string(),
        other => format!("{} ({})", s.project(), other.name()),
    };
    let mut count: HashMap<String, usize> = HashMap::new();
    for s in sessions {
        *count.entry(base(s)).or_default() += 1;
    }
    sessions
        .iter()
        .map(|s| {
            let b = base(s);
            let name = if count[&b] > 1 {
                let short: String = s.id.chars().take(4).collect();
                format!("{b} #{short}")
            } else {
                b
            };
            (s.id.clone(), name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str) -> HookInput {
        HookInput {
            session_id: "abc".into(),
            cwd: r"C:\code\cota".into(),
            hook_event_name: name.into(),
            ..HookInput::default()
        }
    }

    fn write(c: Change) -> Session {
        match c {
            Change::Write(s) => s,
            Change::Remove => panic!("expected a write"),
        }
    }

    const RULES: Rules = Rules {
        notify_done: true,
        notify_needs_you: true,
        min_turn_secs: 30,
        grace_secs: 8,
        quiet_when_focused: true,
    };

    #[test]
    fn a_turn_goes_working_then_done() {
        let s = write(fold(None, &ev("SessionStart"), Agent::Claude, 100));
        assert_eq!(s.state, State::Ready);
        let s = write(fold(Some(s), &ev("UserPromptSubmit"), Agent::Claude, 110));
        assert_eq!(
            (s.state, s.since, s.turn_started),
            (State::Working, 110, 110)
        );
        let s = write(fold(Some(s), &ev("PostToolUse"), Agent::Claude, 150));
        assert_eq!(s.since, 110, "staying in a state keeps its start");
        let s = write(fold(Some(s), &ev("Stop"), Agent::Claude, 200));
        assert_eq!((s.state, s.turn_secs()), (State::Done, 90));
        assert_eq!(s.project(), "cota");
    }

    #[test]
    fn permission_prompt_then_answer() {
        let s = write(fold(None, &ev("UserPromptSubmit"), Agent::Claude, 0));
        let mut n = ev("Notification");
        n.notification_type = "permission_prompt".into();
        n.message = "Claude needs your permission to use Bash".into();
        let s = write(fold(Some(s), &n, Agent::Claude, 5));
        assert_eq!(s.state, State::NeedsYou);
        assert_eq!(s.message, "Claude needs your permission to use Bash");
        let s = write(fold(Some(s), &ev("PostToolUse"), Agent::Claude, 9));
        assert_eq!(s.state, State::Working);
        assert!(s.message.is_empty());
    }

    #[test]
    fn old_versions_without_notification_type_are_read_from_the_message() {
        let mut n = ev("Notification");
        n.message = "Claude needs your permission to use Edit".into();
        assert_eq!(
            write(fold(None, &n, Agent::Claude, 0)).state,
            State::NeedsYou
        );
        n.message = "Claude is waiting for your input".into();
        assert_eq!(write(fold(None, &n, Agent::Claude, 0)).state, State::Done);
    }

    #[test]
    fn idle_reminder_does_not_restart_a_finish() {
        let s = write(fold(None, &ev("Stop"), Agent::Claude, 50));
        let mut n = ev("Notification");
        n.notification_type = "idle_prompt".into();
        let s = write(fold(Some(s), &n, Agent::Claude, 110));
        assert_eq!((s.state, s.since), (State::Done, 50));
    }

    #[test]
    fn compact_mid_turn_keeps_working() {
        let s = write(fold(None, &ev("UserPromptSubmit"), Agent::Claude, 0));
        let s = write(fold(Some(s), &ev("SessionStart"), Agent::Claude, 10));
        assert_eq!(s.state, State::Working);
    }

    #[test]
    fn session_end_removes() {
        assert_eq!(
            fold(None, &ev("SessionEnd"), Agent::Claude, 0),
            Change::Remove
        );
    }

    #[test]
    fn gemini_names_map_onto_the_same_states() {
        let g = Agent::Gemini;
        let s = write(fold(None, &ev("BeforeAgent"), g, 100));
        assert_eq!((s.state, s.agent), (State::Working, Agent::Gemini));
        let mut n = ev("Notification");
        n.notification_type = "ToolPermission".into();
        n.message = "Allow run_shell_command?".into();
        let s = write(fold(Some(s), &n, g, 105));
        assert_eq!(s.state, State::NeedsYou);
        let s = write(fold(Some(s), &ev("AfterTool"), g, 109));
        assert_eq!(s.state, State::Working);
        let s = write(fold(Some(s), &ev("AfterAgent"), g, 160));
        assert_eq!((s.state, s.turn_secs()), (State::Done, 60));
    }

    #[test]
    fn permission_requests_name_the_agent_and_tool() {
        let mut p = ev("PermissionRequest");
        p.tool_name = "apply_patch".into();
        let s = write(fold(None, &p, Agent::Codex, 0));
        assert_eq!(s.state, State::NeedsYou);
        assert_eq!(s.message, "Codex needs your permission to use apply_patch");
    }

    #[test]
    fn an_interrupt_is_not_news() {
        let s = write(fold(None, &ev("UserPromptSubmit"), Agent::Codex, 0));
        let s = write(fold(Some(s), &ev("Interrupt"), Agent::Codex, 5));
        assert_eq!(s.state, State::Ready);
    }

    #[test]
    fn other_agents_are_named_next_to_the_project() {
        let mut a = done("1111", 1, 9);
        a.cwd = "/x/cota".into();
        let mut b = a.clone();
        b.id = "2222".into();
        b.agent = Agent::Codex;
        let names = display_names(&[a, b]);
        assert_eq!(names["1111"], "cota");
        assert_eq!(names["2222"], "cota (Codex)");
    }

    #[test]
    fn file_names_are_tamed() {
        assert_eq!(file_name("../../etc/x").as_deref(), Some("etcx.json"));
        assert_eq!(file_name("///"), None);
    }

    fn done(id: &str, turn: u64, at: u64) -> Session {
        Session {
            id: id.into(),
            cwd: format!("/code/{id}"),
            state: State::Done,
            since: at,
            updated: at,
            turn_started: at - turn,
            ..Session::default()
        }
    }

    #[test]
    fn first_snapshot_is_quiet_then_finishes_toast_once() {
        let mut t = Tracker::default();
        assert!(t
            .update(&[done("old", 60, 100)], 100, &RULES, &|_| false)
            .is_empty());
        assert_eq!(t.overall(&[done("old", 60, 100)]), Overall::YourTurn);

        let now = [done("old", 60, 100), done("cafe", 120, 200)];
        let toasts = t.update(&now, 200, &RULES, &|_| false);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title, "Your turn in cafe");
        assert!(
            t.update(&now, 210, &RULES, &|_| false).is_empty(),
            "no repeats"
        );
    }

    #[test]
    fn no_toast_for_the_terminal_you_are_in_but_the_bubble_still_turns() {
        let mut t = Tracker::default();
        t.update(&[], 0, &RULES, &|_| false);
        let a = done("a", 60, 100);
        let toasts = t.update(std::slice::from_ref(&a), 100, &RULES, &|_| true);
        assert!(toasts.is_empty());
        assert_eq!(t.overall(&[a]), Overall::YourTurn);

        let loud = Rules {
            quiet_when_focused: false,
            ..RULES
        };
        let mut t = Tracker::default();
        t.update(&[], 0, &loud, &|_| false);
        assert_eq!(
            t.update(&[done("b", 60, 100)], 100, &loud, &|_| true).len(),
            1
        );
    }

    #[test]
    fn quick_turns_do_not_toast() {
        let mut t = Tracker::default();
        t.update(&[], 0, &RULES, &|_| false);
        assert!(t
            .update(&[done("a", 5, 100)], 100, &RULES, &|_| false)
            .is_empty());
    }

    #[test]
    fn permission_prompts_wait_out_the_grace_period() {
        let mut t = Tracker::default();
        t.update(&[], 0, &RULES, &|_| false);
        let s = Session {
            id: "a".into(),
            state: State::NeedsYou,
            since: 100,
            updated: 100,
            ..Session::default()
        };
        assert!(t
            .update(std::slice::from_ref(&s), 103, &RULES, &|_| false)
            .is_empty());
        assert_eq!(t.next_due(std::slice::from_ref(&s), &RULES), Some(108));
        assert_eq!(
            t.update(std::slice::from_ref(&s), 108, &RULES, &|_| false)
                .len(),
            1
        );
        assert_eq!(t.next_due(std::slice::from_ref(&s), &RULES), None);
    }

    #[test]
    fn seen_clears_the_colour_until_the_next_finish() {
        let mut t = Tracker::default();
        let a = done("a", 60, 100);
        t.update(std::slice::from_ref(&a), 100, &RULES, &|_| false);
        t.mark_seen(&a);
        assert_eq!(t.overall(std::slice::from_ref(&a)), Overall::Idle);
        let again = done("a", 60, 300);
        t.update(std::slice::from_ref(&again), 300, &RULES, &|_| false);
        assert_eq!(t.overall(&[again]), Overall::YourTurn);
    }

    #[test]
    fn duplicate_folders_get_told_apart() {
        let names = display_names(&[done("1111x", 1, 9), done("2222y", 1, 9)]);
        assert_eq!(names["1111x"], "1111x");
        let mut a = done("1111", 1, 9);
        let mut b = done("2222", 1, 9);
        a.cwd = "/x/cota".into();
        b.cwd = "/y/cota".into();
        let names = display_names(&[a, b]);
        assert_eq!(names["1111"], "cota #1111");
    }
}
