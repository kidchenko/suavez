//! One copy in the tray, and the two ways other processes talk to it.
//!
//! - **Summon**: launching Sua Vez while it is already running makes the
//!   running copy announce what is waiting on you, then steps aside.
//! - **Changed**: `suavez hook` has just rewritten a session file. The poke
//!   saves the tray from polling the directory; the files are the message.
//!
//! Windows carries each on a named auto-reset event (several pokes in a burst
//! collapse into one rescan, which is what we want); macOS on a Unix domain
//! socket in the config directory, one byte per message.

pub use imp::{acquire, poke_changed};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    Summon,
    Changed,
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use super::Msg;
    use crate::log::{linfo, lwarn};
    use crate::util::{pcwstr, wide};
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForMultipleObjects,
        EVENT_MODIFY_STATE, INFINITE,
    };

    // "Local\" scopes all three to the logon session, so two users signed in at
    // once each get their own tray and their own hooks.
    const MUTEX_NAME: &str = r"Local\SuaVez.SingleInstance";
    const SUMMON_NAME: &str = r"Local\SuaVez.Summon";
    const CHANGED_NAME: &str = r"Local\SuaVez.Changed";

    /// Held by the sole running instance: the summon and changed events.
    pub struct Listener(isize, isize);

    /// Returns `None` if another instance is already running, in which case it
    /// has been summoned and this process should exit.
    pub fn acquire() -> Option<Listener> {
        let mutex_name = wide(MUTEX_NAME);
        let existing = unsafe {
            match CreateMutexW(None, true, pcwstr(&mutex_name)) {
                Ok(handle) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        let _ = CloseHandle(handle);
                        true
                    } else {
                        // Never closed: it must outlive `main`, and Windows
                        // releases it on exit anyway.
                        false
                    }
                }
                // Cannot tell: assume we are alone rather than refuse to start.
                Err(_) => false,
            }
        };
        if existing {
            linfo!("another instance is running; summoning it");
            signal(SUMMON_NAME);
            return None;
        }

        let make = |name: &str| {
            let n = wide(name);
            match unsafe { CreateEventW(None, false, false, pcwstr(&n)) } {
                Ok(h) => h.0 as isize,
                Err(e) => {
                    // Not fatal: without it the tray still rescans on its tick.
                    lwarn!("could not create {name} ({e})");
                    0
                }
            }
        };
        Some(Listener(make(SUMMON_NAME), make(CHANGED_NAME)))
    }

    /// From a hook process. False when no tray is running to hear it.
    pub fn poke_changed() -> bool {
        signal(CHANGED_NAME)
    }

    fn signal(name: &str) -> bool {
        let n = wide(name);
        unsafe {
            match OpenEventW(EVENT_MODIFY_STATE, false, pcwstr(&n)) {
                Ok(h) => {
                    let ok = SetEvent(h).is_ok();
                    let _ = CloseHandle(h);
                    ok
                }
                Err(_) => false,
            }
        }
    }

    impl Listener {
        /// Run `on` for every message. The thread blocks for the life of the
        /// process, which tears it down on exit.
        pub fn watch<F>(self, on: F)
        where
            F: Fn(Msg) + Send + 'static,
        {
            let mut handles = Vec::new();
            let mut kinds = Vec::new();
            for (raw, kind) in [(self.0, Msg::Summon), (self.1, Msg::Changed)] {
                if raw != 0 {
                    handles.push(raw);
                    kinds.push(kind);
                }
            }
            if handles.is_empty() {
                lwarn!("no events; relaunch and hook pokes are disabled");
                return;
            }
            let _ = std::thread::Builder::new()
                .name("suavez-instance".into())
                .spawn(move || {
                    let hs: Vec<HANDLE> = handles
                        .iter()
                        .map(|&r| HANDLE(r as *mut std::ffi::c_void))
                        .collect();
                    loop {
                        let rc = unsafe { WaitForMultipleObjects(&hs, false, INFINITE) };
                        let i = rc.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                        match kinds.get(i) {
                            Some(&kind) => on(kind),
                            None => {
                                // A failed wait would spin at full tilt.
                                lwarn!("listener stopping (wait returned {})", rc.0);
                                break;
                            }
                        }
                    }
                });
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod imp {
    use super::Msg;
    use crate::log::{linfo, lwarn};
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    fn socket_path() -> Option<PathBuf> {
        // Alongside the config, well within the ~104-char sun_path limit.
        crate::config::Config::dir().map(|d| d.join("suavez.sock"))
    }

    pub struct Listener(Option<UnixListener>);

    /// Returns `None` if another instance is already running, in which case it
    /// has been summoned and this process should exit.
    pub fn acquire() -> Option<Listener> {
        let Some(path) = socket_path() else {
            lwarn!("no socket path; single-instance disabled");
            return Some(Listener(None));
        };
        if send(&path, b's') {
            linfo!("another instance is running; summoned it");
            return None;
        }
        // Nothing answered: a stale socket file, or none. Take it over.
        let _ = std::fs::remove_file(&path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match UnixListener::bind(&path) {
            Ok(l) => Some(Listener(Some(l))),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                // Lost a startup race; the winner is up now.
                let _ = send(&path, b's');
                None
            }
            Err(e) => {
                lwarn!("could not bind the instance socket ({e})");
                Some(Listener(None))
            }
        }
    }

    pub fn poke_changed() -> bool {
        socket_path().map(|p| send(&p, b'c')).unwrap_or(false)
    }

    fn send(path: &Path, byte: u8) -> bool {
        match UnixStream::connect(path) {
            Ok(mut s) => {
                let _ = s.write_all(&[byte]);
                true
            }
            Err(_) => false,
        }
    }

    impl Listener {
        pub fn watch<F>(self, on: F)
        where
            F: Fn(Msg) + Send + 'static,
        {
            let Some(listener) = self.0 else {
                lwarn!("no instance socket; relaunch and hook pokes are disabled");
                return;
            };
            let _ = std::thread::Builder::new()
                .name("suavez-instance".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        match stream {
                            Ok(mut s) => {
                                let mut buf = [0u8; 1];
                                let n = s.read(&mut buf).unwrap_or(0);
                                on(if n == 1 && buf[0] == b'c' {
                                    Msg::Changed
                                } else {
                                    Msg::Summon
                                });
                            }
                            Err(e) => {
                                lwarn!("instance accept failed ({e}); listener stopping");
                                break;
                            }
                        }
                    }
                });
        }
    }
}
