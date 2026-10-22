//! The tray icon, its menu, and the notifications.
//!
//! Everything here runs on the event loop's thread. The state is the sessions
//! directory as of the last scan, plus the tracker's memory of what has been
//! announced and seen. A scan happens when a hook pokes us, when the menu is
//! used, and on a slow tick that ages out stale sessions and keeps the
//! "3m ago" labels honest.

use crate::agent::Agent;
use crate::config::Config;
use crate::icon::{self, Face};
use crate::log::{lerror, linfo, lwarn};
use crate::session::{self, Overall, Rules, Session, State, Tracker};
use crate::single_instance::Msg;
use crate::util::{ago, now_unix, span};
use crate::{autostart, hooks, notify, terminal, theme, wezterm};

use std::time::{Duration, Instant, SystemTime};
use tao::event_loop::EventLoopProxy;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const ID_SESSION: &str = "s:";
const ID_SEEN_ALL: &str = "seen-all";
const ID_NOTIFY_DONE: &str = "notify-done";
const ID_NOTIFY_NEEDS: &str = "notify-needs";
const ID_QUIET_FOCUSED: &str = "quiet-focused";
const ID_CONNECT: &str = "connect:";
const ID_FORGET: &str = "forget";
const ID_EDIT: &str = "edit-config";
const ID_AUTOSTART: &str = "autostart";
const ID_QUIT: &str = "quit";
/// Shared by every read-only row; nothing dispatches on it.
const ID_INERT: &str = "inert";

/// The slow tick. Hook pokes carry the news; this only ages out sessions,
/// refreshes minute labels, and is the fallback if a poke is ever lost.
const TICK: Duration = Duration::from_secs(15);

pub enum UserEvent {
    Menu(String),
    Instance(Msg),
}

#[derive(PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

pub struct App {
    cfg: Config,
    tray: TrayIcon,
    dark_taskbar: bool,
    sessions: Vec<Session>,
    tracker: Tracker,
    /// Agents whose hooks are in place, as of the last look.
    connected: Vec<Agent>,

    // What is on screen, so a scan only repaints what changed.
    face: Face,
    menu_sig: String,

    /// The config file's mtime as of our last read or write: anything newer is
    /// a hand edit.
    cfg_mtime: Option<SystemTime>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        forward_menu_events(&proxy);

        let dark_taskbar = theme::taskbar_is_dark();
        let tray = TrayIconBuilder::new()
            .with_tooltip("Sua Vez")
            .with_icon(tray_image(Face::Idle, dark_taskbar)?)
            // Either button opens the menu, on both platforms. The menu is the
            // whole interface: a left click that did anything else would look
            // like a click that did nothing.
            .with_menu_on_left_click(true)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .build()
            .map_err(|e| format!("could not create the tray icon: {e}"))?;

        let mut app = Self {
            cfg: Config::load(),
            tray,
            dark_taskbar,
            sessions: Vec::new(),
            tracker: Tracker::default(),
            connected: Vec::new(),
            face: Face::Idle,
            menu_sig: String::new(),
            cfg_mtime: Config::modified(),
        };
        app.connect_new_agents();
        for i in hooks::installed() {
            match i.repoint() {
                Ok(true) => linfo!("{} hooks brought up to date", i.agent.slug()),
                Ok(false) => {}
                Err(e) => lwarn!("could not check the {} hooks: {e}", i.agent.slug()),
            }
        }
        app.refresh_connections();
        app.scan();
        Ok(app)
    }

    /// Connect to each agent the first time it turns up installed: it is the
    /// one thing the app is for, and asking first would be a dialog nobody
    /// says no to. Once per agent, so unticking "Connect to …" sticks. Runs on
    /// the tick as well, so an agent installed later is picked up without a
    /// restart.
    fn connect_new_agents(&mut self) {
        for i in &hooks::INTEGRATIONS {
            if self.cfg.offered.contains(&i.agent) || !i.available() {
                continue;
            }
            self.cfg.offered.push(i.agent);
            self.save();
            if i.is_installed() {
                continue;
            }
            match i.install() {
                Ok(()) => {
                    linfo!("connected to {} on first sight", i.agent.slug());
                    notify::show(
                        &format!("Sua Vez is connected to {}", i.label()),
                        &format!(
                            "The bubble lights up when {} finishes or needs you.",
                            i.agent.name()
                        ),
                        Some("Takes effect in new sessions."),
                    );
                }
                Err(e) => {
                    lerror!("could not connect to {}: {e}", i.agent.slug());
                    notify::show(
                        &format!("Sua Vez could not connect to {}", i.label()),
                        &e,
                        Some("Try \"Connect to\" in the menu."),
                    );
                }
            }
            self.refresh_connections();
        }
    }

    fn refresh_connections(&mut self) {
        self.connected = hooks::installed().iter().map(|i| i.agent).collect();
    }

    pub fn handle(&mut self, event: UserEvent) -> Flow {
        match event {
            UserEvent::Instance(Msg::Changed) => self.scan(),
            UserEvent::Instance(Msg::Summon) => {
                self.scan();
                self.announce_summary();
            }
            UserEvent::Menu(id) => return self.menu(&id),
        }
        Flow::Continue
    }

    fn menu(&mut self, id: &str) -> Flow {
        match id {
            ID_QUIT => {
                linfo!("quit requested from the tray");
                return Flow::Exit;
            }
            ID_SEEN_ALL => self.mark_all_seen(),
            ID_NOTIFY_DONE => {
                self.cfg.notify_done = !self.cfg.notify_done;
                self.save();
            }
            ID_NOTIFY_NEEDS => {
                self.cfg.notify_needs_you = !self.cfg.notify_needs_you;
                self.save();
            }
            ID_QUIET_FOCUSED => {
                self.cfg.quiet_when_focused = !self.cfg.quiet_when_focused;
                self.save();
            }
            ID_FORGET => {
                linfo!("forgetting all sessions");
                session::remove_all();
                self.scan();
            }
            ID_EDIT => open_config(),
            ID_AUTOSTART => {
                let want = !autostart::is_enabled();
                match autostart::set(want) {
                    Ok(()) => linfo!("run at login {}", if want { "on" } else { "off" }),
                    Err(e) => lerror!("{e}"),
                }
            }
            other => {
                if let Some(agent) = other.strip_prefix(ID_CONNECT).and_then(Agent::parse) {
                    self.toggle_connection(agent);
                } else if let Some(sid) = other.strip_prefix(ID_SESSION) {
                    if let Some(s) = self.sessions.iter().find(|s| s.id == sid) {
                        self.tracker.mark_seen(s);
                        if let Some(t) = &s.terminal {
                            match terminal::focus(t) {
                                Ok(()) => linfo!("brought {} forward", t.name),
                                Err(e) => lwarn!("{e}"),
                            }
                        }
                    }
                }
            }
        }
        self.repaint(true);
        Flow::Continue
    }

    pub fn tick(&mut self) {
        self.reload_if_edited();
        self.connect_new_agents();
        let dark = theme::taskbar_is_dark();
        if dark != self.dark_taskbar {
            self.dark_taskbar = dark;
            self.paint_icon(true);
        }
        self.scan();
    }

    /// The slow tick, or sooner if a permission prompt's grace period ends
    /// first.
    pub fn next_wake(&self) -> Instant {
        let tick = Instant::now() + TICK;
        match self.tracker.next_due(&self.sessions, &self.rules()) {
            Some(due) => {
                let wait = due.saturating_sub(now_unix());
                tick.min(Instant::now() + Duration::from_secs(wait))
            }
            None => tick,
        }
    }

    fn rules(&self) -> Rules {
        Rules {
            notify_done: self.cfg.notify_done,
            notify_needs_you: self.cfg.notify_needs_you,
            min_turn_secs: self.cfg.min_turn_seconds,
            grace_secs: self.cfg.grace_seconds,
            quiet_when_focused: self.cfg.quiet_when_focused,
        }
    }

    fn scan(&mut self) {
        let now = now_unix();
        let before: Vec<(String, State)> = self
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.state))
            .collect();
        self.sessions = session::load_all(now, self.cfg.stale_hours * 3600);
        let after: Vec<(String, State)> = self
            .sessions
            .iter()
            .map(|s| (s.id.clone(), s.state))
            .collect();
        if before != after {
            linfo!("sessions: {after:?}");
        }
        let rules = self.rules();
        // Its terminal app has to be in front, and for WezTerm, its own pane:
        // a finish in another tab still notifies. If WezTerm can't be asked,
        // the app being in front is the best we know.
        let in_front = |s: &Session| {
            if !s.terminal.as_ref().is_some_and(terminal::in_front) {
                return false;
            }
            let front = match &s.pane {
                Some(p) => wezterm::is_focused(p).unwrap_or(true),
                None => true,
            };
            if front {
                linfo!("no notification for {}: you're in its terminal", s.id);
            }
            front
        };
        for t in self.tracker.update(&self.sessions, now, &rules, &in_front) {
            notify::show(&t.title, &t.body, None);
        }
        self.repaint(false);
    }

    fn mark_all_seen(&mut self) {
        for s in &self.sessions {
            self.tracker.mark_seen(s);
        }
        self.repaint(false);
    }

    fn toggle_connection(&mut self, agent: Agent) {
        let Some(i) = hooks::get(agent) else { return };
        let was = self.connected.contains(&agent);
        let result = if was { i.uninstall() } else { i.install() };
        self.refresh_connections();
        match result {
            Ok(()) => {
                linfo!(
                    "{} hooks {}",
                    agent.slug(),
                    if was { "removed" } else { "installed" }
                );
                notify::show(
                    &format!(
                        "{} {}",
                        if was {
                            "Disconnected from"
                        } else {
                            "Connected to"
                        },
                        i.label()
                    ),
                    "Takes effect in new sessions.",
                    None,
                );
            }
            Err(e) => {
                lerror!("{e}");
                notify::show(
                    &format!("Sua Vez could not change {}'s settings", agent.product()),
                    &e,
                    None,
                );
            }
        }
    }

    /// For a relaunch: say what is waiting, since that is presumably why
    /// someone went looking for the app.
    fn announce_summary(&self) {
        let names = session::display_names(&self.sessions);
        let waiting: Vec<&str> = self
            .sessions
            .iter()
            .filter(|s| matches!(s.state, State::NeedsYou | State::Done))
            .map(|s| names[&s.id].as_str())
            .collect();
        if waiting.is_empty() {
            notify::show("Sua Vez", &self.header(), None);
        } else {
            notify::show(
                &if waiting.len() == 1 {
                    "1 session is waiting on you".to_string()
                } else {
                    format!("{} sessions are waiting on you", waiting.len())
                },
                &waiting.join(", "),
                None,
            );
        }
    }

    fn save(&mut self) {
        match self.cfg.save() {
            Ok(()) => self.cfg_mtime = Config::modified(),
            Err(e) => lwarn!("could not save config: {e}"),
        }
    }

    fn reload_if_edited(&mut self) {
        let mtime = Config::modified();
        if mtime.is_none() || mtime == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = mtime;
        if let Some(fresh) = Config::reload() {
            if fresh != self.cfg {
                linfo!("config edited on disk");
                self.cfg = fresh;
                self.repaint(true);
            }
        }
    }

    fn repaint(&mut self, force_menu: bool) {
        self.paint_icon(false);
        self.rebuild_menu(force_menu);
    }

    fn paint_icon(&mut self, force: bool) {
        let face = match self.tracker.overall(&self.sessions) {
            Overall::NeedsYou => Face::NeedsYou,
            Overall::YourTurn => Face::YourTurn,
            Overall::Working => Face::Working,
            Overall::Idle => Face::Idle,
        };
        if face == self.face && !force {
            return;
        }
        self.face = face;
        match tray_image(face, self.dark_taskbar) {
            Ok(img) => {
                let _ = self.tray.set_icon(Some(img));
                #[cfg(target_os = "macos")]
                self.tray.set_icon_as_template(face.is_template());
            }
            Err(e) => lerror!("{e}"),
        }
    }

    fn header(&self) -> String {
        if self.connected.is_empty() {
            return "Not connected to any coding agent".into();
        }
        if self.sessions.is_empty() {
            return "No sessions".into();
        }
        let count = |state| self.sessions.iter().filter(|s| s.state == state).count();
        let parts: Vec<String> = [
            (count(State::NeedsYou), "needs you", "need you"),
            (count(State::Done), "your turn", "your turn"),
            (count(State::Working), "working", "working"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
        if parts.is_empty() {
            "All quiet".into()
        } else {
            parts.join(" \u{00b7} ")
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let now = now_unix();
        let names = session::display_names(&self.sessions);
        self.sessions
            .iter()
            .map(|s| {
                let name = menu_text(&names[&s.id]);
                let elapsed = now.saturating_sub(s.since);
                let when = ago(elapsed);
                let seen = self.tracker.is_seen(s);
                let (label, clickable) = match s.state {
                    State::NeedsYou => (
                        format!(
                            "{}  {} \u{00b7} {when}",
                            if seen { "\u{25cb}" } else { "\u{26a0}" },
                            menu_text(&blocked_line(&names[&s.id], &s.message))
                        ),
                        !seen,
                    ),
                    State::Done => (
                        format!(
                            "{}  {name}: your turn \u{00b7} {when}",
                            if seen { "\u{25cb}" } else { "\u{25cf}" }
                        ),
                        !seen,
                    ),
                    State::Working => {
                        let took = if elapsed < 60 {
                            "just started".to_string()
                        } else {
                            span(elapsed)
                        };
                        (
                            format!("\u{2026}  {name} is working \u{00b7} {took}"),
                            false,
                        )
                    }
                    State::Ready => (format!("     {name}: ready"), false),
                };
                // Any row whose terminal is known can bring it forward.
                let clickable = clickable || s.terminal.is_some();
                (format!("{ID_SESSION}{}", s.id), label, clickable)
            })
            .collect()
    }

    fn rebuild_menu(&mut self, force: bool) {
        let header = self.header();
        let rows = self.rows();
        let autostart_on = autostart::is_enabled();
        let sig = format!(
            "{header}|{rows:?}|{:?}{}{}{}{autostart_on}",
            self.connected,
            self.cfg.notify_done,
            self.cfg.notify_needs_you,
            self.cfg.quiet_when_focused
        );
        // Rebuilding closes an open menu on some platforms, so only when
        // something on it actually changed.
        if sig == self.menu_sig && !force {
            return;
        }
        self.menu_sig = sig;
        let _ = self.tray.set_tooltip(Some(format!("Sua Vez: {header}")));

        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id(ID_INERT, &header, false, None));
        if !rows.is_empty() {
            let _ = menu.append(&PredefinedMenuItem::separator());
            for (id, label, clickable) in &rows {
                let id = if *clickable { id.as_str() } else { ID_INERT };
                let _ = menu.append(&MenuItem::with_id(id, label, *clickable, None));
            }
            let any_unseen = self.tracker.overall(&self.sessions) == Overall::NeedsYou
                || self.tracker.overall(&self.sessions) == Overall::YourTurn;
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&MenuItem::with_id(
                ID_SEEN_ALL,
                "Mark all as seen",
                any_unseen,
                None,
            ));
            let _ = menu.append(&MenuItem::with_id(
                ID_FORGET,
                "Forget all sessions",
                true,
                None,
            ));
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_NOTIFY_DONE,
            "Notify when it's your turn",
            true,
            self.cfg.notify_done,
            None,
        ));
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_NOTIFY_NEEDS,
            "Notify when an agent needs you",
            true,
            self.cfg.notify_needs_you,
            None,
        ));
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_QUIET_FOCUSED,
            "Quiet while its terminal is in front",
            true,
            self.cfg.quiet_when_focused,
            None,
        ));
        let _ = menu.append(&MenuItem::with_id(
            ID_EDIT,
            "Edit settings\u{2026}",
            true,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
        // One toggle per agent that is installed, or that is connected even
        // though its folder has gone (so the hooks can still be taken out).
        for i in &hooks::INTEGRATIONS {
            let on = self.connected.contains(&i.agent);
            if !on && !i.available() {
                continue;
            }
            let _ = menu.append(&CheckMenuItem::with_id(
                format!("{ID_CONNECT}{}", i.agent.slug()),
                format!("Connect to {}", i.label()),
                true,
                on,
                None,
            ));
        }
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_AUTOSTART,
            "Run at login",
            true,
            autostart_on,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Quit Sua Vez", true, None));

        self.tray.set_menu(Some(Box::new(menu)));
    }
}

/// The menu row for a blocked session. "Codex needs your permission to use
/// Bash" reads better as "cota wants to use Bash", whichever agent said it.
fn blocked_line(name: &str, message: &str) -> String {
    const MIDDLE: &str = " needs your permission to use ";
    if let Some(i) = message.find(MIDDLE) {
        return format!("{name} wants to use {}", &message[i + MIDDLE.len()..]);
    }
    if message.is_empty() {
        return format!("{name} needs you");
    }
    let mut s: String = message.chars().take(48).collect();
    if s.len() < message.len() {
        s.push('\u{2026}');
    }
    format!("{name}: {s}")
}

/// Windows menus treat `&` as an accelerator marker; a folder called
/// "R&D" should not grow an underline.
fn menu_text(s: &str) -> String {
    if cfg!(windows) {
        s.replace('&', "&&")
    } else {
        s.to_string()
    }
}

/// Open config.json in a text editor, creating it first if need be.
fn open_config() {
    let Some(path) = Config::path() else { return };
    if !path.exists() {
        let _ = Config::load().save();
    }
    // Notepad / TextEdit rather than the .json association, which on many
    // machines is a browser or nothing at all.
    #[cfg(windows)]
    let spawned = std::process::Command::new("notepad.exe").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .spawn();
    if let Err(e) = spawned {
        lwarn!("could not open {}: {e}", path.display());
    }
}

fn tray_image(face: Face, dark: bool) -> Result<Icon, String> {
    Icon::from_rgba(icon::render(face, dark, icon::EDGE), icon::EDGE, icon::EDGE)
        .map_err(|e| format!("could not build the tray image: {e}"))
}

fn forward_menu_events(proxy: &EventLoopProxy<UserEvent>) {
    let proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = proxy.send_event(UserEvent::Menu(e.id.0));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_messages_read_naturally() {
        assert_eq!(
            blocked_line("cota", "Claude needs your permission to use Bash"),
            "cota wants to use Bash"
        );
        assert_eq!(blocked_line("cota", ""), "cota needs you");
        assert!(blocked_line("cota", &"x".repeat(80)).ends_with('\u{2026}'));
    }
}
