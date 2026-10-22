// No console window: this is a tray app. No-op off Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! Sua Vez ("your turn") lights up in the tray when a coding agent session
//! (Claude Code, Codex, Gemini CLI or Devin) finishes or is blocked waiting on
//! you, on both Windows and macOS.
//!
//! | module            | responsibility                                       |
//! |-------------------|------------------------------------------------------|
//! | `agent`           | the agents Sua Vez knows                             |
//! | `session`         | hook events → per-session state; what to announce    |
//! | `hooks`           | Sua Vez's entries in each agent's hooks config       |
//! | `config`          | the persisted preferences                            |
//! | `icon`            | draws the bubble                                     |
//! | `notify`          | toasts                                               |
//! | `app`             | the tray icon and its menu                           |
//! | `single_instance` | one copy at a time; relaunch and hook pokes          |
//! | `autostart`       | run at login                                         |
//! | `terminal`        | finds a session's terminal and brings it forward     |
//!
//! The same binary is the hook: the agent runs `suavez hook` with the event on
//! stdin, and that process updates a file and pokes the tray.

mod agent;
mod app;
mod autostart;
mod config;
mod hooks;
mod icon;
mod log;
mod notify;
mod session;
mod single_instance;
mod terminal;
mod theme;
mod util;

use agent::Agent;
use app::{App, Flow, UserEvent};
use log::{lerror, linfo, lwarn};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

const USAGE: &str = "\
Sua Vez: a tray icon that lights up when your coding agent finishes or needs you.
Works with Claude Code, Codex, Gemini CLI and Devin.

usage:
  suavez                    run in the tray (or summon the running copy)
  suavez install [agent]    connect to every installed agent, or just one
  suavez uninstall [agent]  disconnect from every agent, or just one
  suavez status             print connections and what every session is doing
  suavez hook [--agent a]   what the agents run; reads the event on stdin

agents: claude, codex, gemini (Devin uses Claude Code's hooks)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => tray(),
        Some("hook") => hook(&args[1..]),
        Some("--dump-icons") => {
            let dir = args.get(1).map(String::as_str).unwrap_or("target/icons");
            std::process::exit(match icon::dump(dir) {
                Ok(()) => 0,
                Err(_) => 1,
            });
        }
        Some(cmd) => {
            util::attach_console();
            std::process::exit(cli(cmd, args.get(1).map(String::as_str)));
        }
    }
}

/// Which agent is calling. `--agent` when the hook entry says so; otherwise
/// Claude Code's entry, which Devin also loads, so Devin is told apart by the
/// variable it sets for its hooks.
fn calling_agent(args: &[String]) -> Agent {
    let flag = args
        .iter()
        .position(|a| a == "--agent")
        .and_then(|i| args.get(i + 1))
        .and_then(|a| Agent::parse(a));
    flag.unwrap_or_else(|| {
        if std::env::var_os("DEVIN_PROJECT_DIR").is_some() {
            Agent::Devin
        } else {
            Agent::Claude
        }
    })
}

/// Runs inside the agent's turn, so: fast, exit 0 whatever happens, and quiet
/// on stdout except where the agent insists on JSON. A hook that fails must
/// never be the reason the agent stops.
fn hook(args: &[String]) {
    use std::io::Read;
    log::init(false);
    let agent = calling_agent(args);
    let mut input = String::new();
    // Hook payloads are a few hundred bytes; the cap is against a runaway pipe.
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    if let Err(e) = session::ingest(&input, agent, util::now_unix()) {
        lwarn!("hook ({}): {e}", agent.slug());
    }
    single_instance::poke_changed();
    if agent.wants_json_reply() {
        // An empty object: no decision, no context, carry on.
        println!("{{}}");
    }
}

/// The integrations a CLI command applies to: the one named, or with no name,
/// every installed agent (Claude Code if none is found, as before agents
/// were plural).
fn targets(
    which: Option<&str>,
    only_available: bool,
) -> Result<Vec<&'static hooks::Integration>, String> {
    match which {
        Some(name) => {
            let agent = Agent::parse(name).ok_or(format!(
                "unknown agent '{name}' (try claude, codex or gemini)"
            ))?;
            if agent == Agent::Devin {
                return Err("Devin uses Claude Code's hooks: run `suavez install claude`".into());
            }
            Ok(hooks::get(agent).into_iter().collect())
        }
        None if only_available => {
            let found: Vec<_> = hooks::INTEGRATIONS
                .iter()
                .filter(|i| i.available())
                .collect();
            Ok(if found.is_empty() {
                hooks::get(Agent::Claude).into_iter().collect()
            } else {
                found
            })
        }
        None => Ok(hooks::INTEGRATIONS.iter().collect()),
    }
}

fn cli(cmd: &str, arg: Option<&str>) -> i32 {
    let run =
        |only_available: bool, act: fn(&hooks::Integration) -> Result<(), String>, done: &str| {
            let list = match targets(arg, only_available) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("suavez: {e}");
                    return 2;
                }
            };
            let mut code = 0;
            for i in list {
                match act(i) {
                    Ok(()) => {
                        let path = i
                            .settings_path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        println!("{done} {}  ({path})", i.label());
                    }
                    Err(e) => {
                        eprintln!("suavez: {}: {e}", i.label());
                        code = 1;
                    }
                }
            }
            code
        };
    match cmd {
        "install" => {
            let code = run(true, hooks::Integration::install, "Connected to");
            if code == 0 {
                println!("Takes effect in new sessions.");
            }
            code
        }
        "uninstall" => run(false, hooks::Integration::uninstall, "Disconnected from"),
        "status" => {
            status();
            0
        }
        "--version" | "-V" => {
            println!("suavez {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            0
        }
        other => {
            eprintln!("suavez: unknown command '{other}'\n\n{USAGE}");
            2
        }
    }
}

fn status() {
    let now = util::now_unix();
    let cfg = config::Config::load();
    for i in &hooks::INTEGRATIONS {
        let state = if i.is_installed() {
            "connected"
        } else if i.available() {
            "not connected (run `suavez install`)"
        } else {
            "not installed"
        };
        println!("{:<24} {state}", i.label());
    }
    let sessions = session::load_all(now, cfg.stale_hours * 3600);
    println!();
    if sessions.is_empty() {
        println!("No sessions.");
        return;
    }
    let names = session::display_names(&sessions);
    for s in &sessions {
        let state = match s.state {
            session::State::NeedsYou => "needs you",
            session::State::Done => "your turn",
            session::State::Working => "working",
            session::State::Ready => "ready",
        };
        println!(
            "  {:<32} {:<10} {:>8}  {}",
            names[&s.id],
            state,
            util::ago(now.saturating_sub(s.since)),
            s.message
        );
    }
}

fn tray() {
    log::init(true);
    linfo!(
        "--- Sua Vez {} starting (pid {}) ---",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );

    // A second launch summons the running instance and exits.
    let Some(listener) = single_instance::acquire() else {
        return;
    };

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Menu-bar agent, no Dock icon. tao resets the activation policy as it
    // starts, overriding LSUIElement, so it has to be asked for here too.
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();

    let instance_proxy = proxy.clone();
    listener.watch(move |msg| {
        let _ = instance_proxy.send_event(UserEvent::Instance(msg));
    });

    // The tray is built once the loop is running: on macOS the status item
    // cannot be created before the application has finished launching.
    let mut app: Option<App> = None;

    event_loop.run(move |event, _target, control_flow| {
        match event {
            Event::NewEvents(StartCause::Init) => match App::new(proxy.clone()) {
                Ok(a) => app = Some(a),
                Err(e) => {
                    lerror!("{e}");
                    *control_flow = ControlFlow::Exit;
                    return;
                }
            },
            Event::UserEvent(e) => {
                if let Some(a) = app.as_mut() {
                    if a.handle(e) == Flow::Exit {
                        app = None;
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Some(a) = app.as_mut() {
                    a.tick();
                }
            }
            _ => {}
        }

        if let Some(a) = app.as_ref() {
            *control_flow = ControlFlow::WaitUntil(a.next_wake());
        }
    });
}
