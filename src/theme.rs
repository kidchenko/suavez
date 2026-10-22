//! Whether the Windows taskbar is dark, so the bubble can be drawn in a colour
//! that shows up against it. macOS needs none of this: the menu bar icon is a
//! template image and AppKit tints it.

pub use imp::taskbar_is_dark;

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use crate::util::{pcwstr, wide};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_VALUE_TYPE,
    };

    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    /// True when the *taskbar* is dark.
    ///
    /// Deliberately `SystemUsesLightTheme` and not `AppsUseLightTheme`: the two
    /// are set independently, and the only pixels we draw live in the
    /// notification area, and light-apps/dark-taskbar is a common combination.
    ///
    /// Defaults to dark when the value is missing: that is the Windows 11
    /// default and the safer guess.
    pub fn taskbar_is_dark() -> bool {
        read_dword(PERSONALIZE, "SystemUsesLightTheme")
            .map(|v| v == 0)
            .unwrap_or(true)
    }

    fn read_dword(subkey: &str, value: &str) -> Option<u32> {
        let sub = wide(subkey);
        let name = wide(value);
        let mut hkey = HKEY::default();

        unsafe {
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                pcwstr(&sub),
                None,
                KEY_QUERY_VALUE,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return None;
            }

            let mut data: u32 = 0;
            let mut size: u32 = std::mem::size_of::<u32>() as u32;
            let mut kind = REG_VALUE_TYPE::default();
            let rc = RegQueryValueExW(
                hkey,
                pcwstr(&name),
                None,
                Some(&mut kind),
                Some(&mut data as *mut u32 as *mut u8),
                Some(&mut size),
            );
            let _ = RegCloseKey(hkey);

            (rc == ERROR_SUCCESS).then_some(data)
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod imp {
    /// Unused in practice: the macOS icon is a template image, which AppKit
    /// tints for the menu bar itself.
    pub fn taskbar_is_dark() -> bool {
        false
    }
}
