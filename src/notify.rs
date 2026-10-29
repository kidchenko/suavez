//! Notifications, for the two moments Sua Vez exists for: an agent finished a
//! turn, or an agent is blocked waiting on you. Everything else is visible in the
//! bubble and its menu.
//!
//! Best-effort throughout: a notification that cannot be shown (Focus Assist,
//! Do Not Disturb, policy) is logged and otherwise ignored.

/// What a click on the notification does. Called on a system thread.
pub type OnClick = Box<dyn Fn() + Send + 'static>;

pub fn show(title: &str, line1: &str, line2: Option<&str>) {
    imp::show(title, line1, line2, None);
}

/// A notification that does `on_click` when clicked. On Windows that is
/// certain while its banner is on screen; a click from the notification centre
/// later goes through Windows, which may only relaunch the app. On macOS it
/// holds while Sua Vez is running, banner or Notification Center, as long as it
/// runs as SuaVez.app (see below).
pub fn show_clickable(title: &str, line1: &str, line2: Option<&str>, on_click: OnClick) {
    imp::show(title, line1, line2, Some(on_click));
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
//
// `Toast::POWERSHELL_APP_ID` is a borrowed identity: Windows will not show a
// toast for an application it has never heard of, and registering a real
// AppUserModelID means writing a Start Menu shortcut carrying it. That is the
// installer's job, not the app's, so an unpackaged run borrows PowerShell's
// registration and shows up under its name. Once the installer lands, the
// shortcut is detected and the toasts get the right name and icon.
#[cfg(windows)]
mod imp {
    use crate::log::{linfo, lwarn};
    use tauri_winrt_notification::{Duration, Toast};

    /// Matches the `AppUserModelID` the installer stamps on the Start Menu
    /// shortcut. The two must stay in step: Windows resolves a toast's name and
    /// icon by looking this string up against registered shortcuts, and an id
    /// with no shortcut behind it shows nothing at all.
    const APP_ID: &str = "kidchenko.SuaVez";

    /// The shortcut is the registration: Windows silently drops toasts for an
    /// AUMID with no shortcut behind it. A portable copy with no installer
    /// borrows PowerShell's identity instead: wrong name, but a toast.
    fn shortcut_installed() -> bool {
        const REL: &str = r"Microsoft\Windows\Start Menu\Programs\Sua Vez\Sua Vez.lnk";
        ["APPDATA", "PROGRAMDATA"].iter().any(|var| {
            std::env::var_os(var)
                .map(|base| std::path::PathBuf::from(base).join(REL).exists())
                .unwrap_or(false)
        })
    }

    fn app_id() -> String {
        if let Ok(explicit) = std::env::var("SUAVEZ_APP_ID") {
            return explicit;
        }
        if shortcut_installed() {
            APP_ID.to_string()
        } else {
            Toast::POWERSHELL_APP_ID.to_string()
        }
    }

    pub fn show(title: &str, line1: &str, line2: Option<&str>, on_click: Option<super::OnClick>) {
        let mut toast = Toast::new(&app_id())
            .title(title)
            .text1(line1)
            .duration(Duration::Short);
        if let Some(l2) = line2 {
            toast = toast.text2(l2);
        }
        if let Some(f) = on_click {
            toast = toast.on_activated(move |_| {
                f();
                Ok(())
            });
        }
        match toast.show() {
            Ok(()) => linfo!("toast shown: {title} / {line1}"),
            Err(e) => lwarn!("could not show toast: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
//
// UserNotifications, so a click comes back to us: the delegate hears it and
// runs the notification's `on_click`. It needs a bundle identifier, which only
// SuaVez.app has, and asks once for permission. A bare binary (`cargo run`)
// falls back to `osascript`'s `display notification`, the analogue of borrowing
// PowerShell's identity on Windows: it shows under Script Editor's name, and
// clicking it opens Script Editor.
#[cfg(target_os = "macos")]
mod imp {
    use super::OnClick;
    use crate::log::{linfo, lwarn};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, ClassType};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    /// Clicks still to come, by notification identifier. Only the latest few:
    /// one clicked after it has scrolled off this list does nothing.
    static PENDING: Mutex<Vec<(String, OnClick)>> = Mutex::new(Vec::new());
    const KEEP: usize = 32;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "SuaVezNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// Show banners even when Sua Vez counts as the active app, which
            /// it does for a moment after its menu is used.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                done.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            /// Without custom actions or a dismiss action, any response is a
            /// click on the notification itself.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                done: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                let found = {
                    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
                    pending
                        .iter()
                        .position(|(p, _)| *p == id)
                        .map(|i| pending.remove(i).1)
                };
                match found {
                    Some(f) => f(),
                    None => linfo!("notification {id} clicked; nothing to do"),
                }
                done.call(());
            }
        }
    );

    /// The notification centre with our delegate on it, or `None` outside a
    /// bundle, where asking for the centre at all would throw.
    fn center() -> Option<&'static Retained<UNUserNotificationCenter>> {
        struct Center(Option<Retained<UNUserNotificationCenter>>);
        // The centre is thread-safe; objc2 just can't say so.
        unsafe impl Send for Center {}
        unsafe impl Sync for Center {}
        static CENTER: OnceLock<Center> = OnceLock::new();
        CENTER.get_or_init(|| Center(make())).0.as_ref()
    }

    fn make() -> Option<Retained<UNUserNotificationCenter>> {
        NSBundle::mainBundle().bundleIdentifier()?;
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::class(), new] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The centre holds its delegate weakly; this one lives as long
        // as the process.
        std::mem::forget(delegate);
        Some(center)
    }

    pub fn show(title: &str, line1: &str, line2: Option<&str>, on_click: Option<OnClick>) {
        let Some(center) = center() else {
            return show_via_osascript(title, line1, line2);
        };

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!("suavez-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        if let Some(f) = on_click {
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            pending.push((id.clone(), f));
            let over = pending.len().saturating_sub(KEEP);
            pending.drain(..over);
        }

        // Same mapping as `osascript`: title / subtitle (the detail) / body.
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(line1));
        if let Some(l2) = line2 {
            content.setSubtitle(&NSString::from_str(l2));
        }
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&id),
            &content,
            None,
        );
        // Asking every time costs nothing once answered, and adding only once
        // permission is in hand means the first notification is not lost to
        // the prompt that asks for it.
        let (title, line1) = (title.to_string(), line1.to_string());
        let added = RcBlock::new(move |err: *mut NSError| match unsafe { err.as_ref() } {
            None => linfo!("notification shown: {title} / {line1}"),
            Some(e) => lwarn!("could not show notification: {}", e.localizedDescription()),
        });
        let center_ = center.clone();
        let asked = RcBlock::new(move |granted: Bool, err: *mut NSError| {
            if granted.as_bool() {
                center_.addNotificationRequest_withCompletionHandler(&request, Some(&added));
            } else if let Some(e) = unsafe { err.as_ref() } {
                lwarn!("notifications not allowed: {}", e.localizedDescription());
            } else {
                lwarn!("notifications are turned off for Sua Vez in System Settings");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &asked,
        );
    }

    fn show_via_osascript(title: &str, line1: &str, line2: Option<&str>) {
        // Notification anatomy on macOS is title / subtitle / body. We map the
        // Windows title to the title, the first line to the body, and the
        // optional detail to the subtitle.
        let mut script = format!(
            "display notification {} with title {}",
            applescript_string(line1),
            applescript_string(title),
        );
        if let Some(l2) = line2 {
            script.push_str(&format!(" subtitle {}", applescript_string(l2)));
        }

        match std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(&script)
            .spawn()
        {
            Ok(_) => linfo!("notification shown: {title} / {line1}"),
            Err(e) => lwarn!("could not show notification: {e}"),
        }
    }
    /// A double-quoted AppleScript string literal. Backslash and quote are
    /// escaped; newlines and tabs become their AppleScript escapes so a
    /// multi-line body cannot break out of the `-e` argument.
    fn applescript_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}
