// No console window: this is a tray app. No-op off Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! Sua Vez ("your turn") lights up in the tray when a Claude Code session
//! finishes or is blocked waiting on you, on both Windows and macOS.
//!
//! | module            | responsibility                                       |
//! |-------------------|------------------------------------------------------|
//! | `session`         | hook events → per-session state; what to announce    |
//! | `hooks`           | Sua Vez's entries in Claude Code's settings.json     |
//! | `config`          | the persisted preferences                            |
//! | `icon`            | draws the bubble                                     |
//! | `notify`          | toasts                                               |
//! | `app`             | the tray icon and its menu                           |
//! | `single_instance` | one copy at a time; relaunch and hook pokes          |
//! | `autostart`       | run at login                                         |
//!
//! The same binary is the hook: Claude Code runs `suavez hook` with the event
//! on stdin, and that process updates a file and pokes the tray.

mod app;
mod autostart;
mod config;
mod hooks;
mod icon;
mod log;
mod notify;
mod session;
mod single_instance;
mod theme;
mod util;

use app::{App, Flow, UserEvent};
use log::{lerror, linfo, lwarn};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

const USAGE: &str = "\
Sua Vez: a tray icon that lights up when Claude Code finishes or needs you.

usage:
  suavez              run in the tray (or summon the running copy)
  suavez install      connect to Claude Code (adds hooks to settings.json)
  suavez uninstall    disconnect from Claude Code
  suavez status       print what every session is doing
  suavez hook         what Claude Code runs; reads the event on stdin";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => tray(),
        Some("hook") => hook(),
        Some("--dump-icons") => {
            let dir = args.get(1).map(String::as_str).unwrap_or("target/icons");
            std::process::exit(match icon::dump(dir) {
                Ok(()) => 0,
                Err(_) => 1,
            });
        }
        Some(cmd) => {
            util::attach_console();
            std::process::exit(cli(cmd));
        }
    }
}

/// Runs inside Claude Code's turn, so: fast, silent on stdout (Claude Code
/// reads some hooks' stdout as context), and exit 0 whatever happens. A hook
/// that fails must never be the reason Claude stops.
fn hook() {
    use std::io::Read;
    log::init(false);
    let mut input = String::new();
    // Hook payloads are a few hundred bytes; the cap is against a runaway pipe.
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    if let Err(e) = session::ingest(&input, util::now_unix()) {
        lwarn!("hook: {e}");
    }
    single_instance::poke_changed();
}

fn cli(cmd: &str) -> i32 {
    match cmd {
        "install" => match hooks::install() {
            Ok(()) => {
                println!("Connected to Claude Code.");
                if let Some(p) = hooks::settings_path() {
                    println!("  hooks added to {}", p.display());
                }
                println!("  Takes effect in new Claude Code sessions.");
                0
            }
            Err(e) => {
                eprintln!("suavez: {e}");
                1
            }
        },
        "uninstall" => match hooks::uninstall() {
            Ok(()) => {
                println!("Disconnected from Claude Code.");
                0
            }
            Err(e) => {
                eprintln!("suavez: {e}");
                1
            }
        },
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
    println!(
        "Claude Code hooks: {}",
        if hooks::is_installed() {
            "connected"
        } else {
            "not connected (run `suavez install`)"
        }
    );
    let sessions = session::load_all(now, cfg.stale_hours * 3600);
    if sessions.is_empty() {
        println!("No Claude Code sessions.");
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
            "  {:<28} {:<10} {:>8}  {}",
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
