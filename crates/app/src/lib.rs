//! What the tray app (`squ4t.exe`) and the command line (`squ4t-cli.exe`) share.

pub mod store;

use std::path::Path;

use squ4t_core::pyjson::{self, Value};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::Globalization::{GetLocaleInfoEx, GetNumberFormatEx, LOCALE_SGROUPING, LOCALE_STHOUSAND, NUMBERFMTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};

// ---------------------------------------------------------------- the running instance

pub const WINDOW_CLASS: &str = "squ4t.Tray";
/// Sent to the running instance, like the ShellIpc functions of Service.qml.
pub const MSG_OPEN: u32 = WM_APP + 10;
pub const MSG_REFRESH: u32 = WM_APP + 11;
pub const MSG_TEST_ALERT: u32 = WM_APP + 12;
pub const MSG_QUIT: u32 = WM_APP + 13;

pub fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// One instance per data folder: the hidden window's title names the folder.
pub fn instance_title(dir: &Path) -> String {
    let text = dir.to_string_lossy().to_lowercase();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("squ4t {hash:016x}")
}

pub fn find_instance(dir: &Path) -> Option<HWND> {
    let (class, title) = (wstr(WINDOW_CLASS), wstr(&instance_title(dir)));
    // SAFETY: NUL-terminated strings.
    let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    (!hwnd.is_null()).then_some(hwnd)
}

/// Posts a message to the running instance. False when none is running.
pub fn tell_instance(dir: &Path, message: u32) -> bool {
    match find_instance(dir) {
        // SAFETY: posting a private message to a window of this program.
        Some(hwnd) => unsafe { PostMessageW(hwnd, message, 0 as WPARAM, 0 as LPARAM) != 0 },
        None => false,
    }
}

// ---------------------------------------------------------------- language

/// The interface text: the plugin's English catalogue (i18n/en.json), with its plural forms.
pub struct Strings {
    catalog: Value,
}

impl Strings {
    pub fn english() -> Strings {
        Strings { catalog: pyjson::loads(include_bytes!("../i18n/en.json")).expect("en.json") }
    }
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.catalog.get(key)
    }
    fn text(v: Option<&Value>) -> String {
        match v {
            Some(Value::Str(s)) => String::from_utf8_lossy(s).into_owned(),
            _ => String::new(),
        }
    }
    pub fn t(&self, key: &str) -> String {
        Strings::text(self.lookup(key))
    }
    /// A plural form ("one" for exactly 1), with %1 replaced.
    pub fn tn(&self, key: &str, n: i128) -> String {
        let form = self.lookup(key).and_then(|v| v.get(if n == 1 { "one" } else { "other" }));
        Strings::text(form).replace("%1", &format_number(n))
    }
}

/// A whole number in the user's locale, with its digit grouping.
pub fn format_number(n: i128) -> String {
    let mut sep = [0u16; 8];
    let mut grouping = [0u16; 16];
    // SAFETY: buffers sized as passed; a null locale name is the user default.
    let (sep_len, group_len) =
        unsafe { (GetLocaleInfoEx(std::ptr::null(), LOCALE_STHOUSAND, sep.as_mut_ptr(), 8), GetLocaleInfoEx(std::ptr::null(), LOCALE_SGROUPING, grouping.as_mut_ptr(), 16)) };
    let groups = String::from_utf16_lossy(&grouping[..group_len.max(1) as usize - 1]);
    let grouping_code = match groups.as_str() {
        "3;2;0" => 32,
        "0" | "" => 0,
        _ => 3,
    };
    if sep_len == 0 {
        sep[0] = ',' as u16;
        sep[1] = 0;
    }
    let decimal = [b'.' as u16, 0];
    let format =
        NUMBERFMTW { NumDigits: 0, LeadingZero: 0, Grouping: grouping_code, lpDecimalSep: decimal.as_ptr() as *mut u16, lpThousandSep: sep.as_mut_ptr(), NegativeOrder: 1 };
    let value = wstr(&n.to_string());
    let mut out = [0u16; 64];
    // SAFETY: valid NUL-terminated input and an output buffer of the size passed.
    let len = unsafe { GetNumberFormatEx(std::ptr::null(), 0, value.as_ptr(), &format, out.as_mut_ptr(), 64) };
    if len <= 1 {
        return n.to_string();
    }
    String::from_utf16_lossy(&out[..len as usize - 1])
}

/// "mm:ss", the bar's countdown.
pub fn countdown(seconds: i128) -> String {
    let (m, s) = (seconds / 60, seconds % 60);
    let pad = |v: i128| if v < 10 { format!("{}{}", format_number(0), format_number(v)) } else { format_number(v) };
    format!("{}:{}", pad(m), pad(s))
}

/// Cuts text to at most `max` UTF-16 units, at a character boundary. Tooltips and balloons have fixed buffers.
pub fn fit_utf16(text: &str, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    for c in text.chars() {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        if out.len() + units.len() > max {
            break;
        }
        out.extend_from_slice(units);
    }
    out
}

// ---------------------------------------------------------------- start with Windows

/// The per-user `Run` key: the app starts at each logon of this user, as an enabled plugin
/// starts with the Omarchy shell.
pub mod autostart {
    use std::path::Path;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    use crate::wstr;

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "squ4t";

    /// The value the first builds registered, before the app was named squ4t.
    const LEGACY_VALUE: &str = "SquatBreak";

    pub fn get() -> Option<String> {
        read(VALUE)
    }

    /// Removes the first builds' entry; true when there was one.
    pub fn remove_legacy() -> bool {
        let had = read(LEGACY_VALUE).is_some();
        if had {
            let (key, value) = (wstr(KEY), wstr(LEGACY_VALUE));
            // SAFETY: NUL-terminated names.
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        }
        had
    }

    fn read(name: &str) -> Option<String> {
        let (key, value) = (wstr(KEY), wstr(name));
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: NUL-terminated names and a buffer of the size passed.
        let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    /// `Some(exe)` registers the program; `None` removes it.
    pub fn set(exe: Option<&Path>) -> std::io::Result<()> {
        let (key, value) = (wstr(KEY), wstr(VALUE));
        // SAFETY: NUL-terminated names; the data is a NUL-terminated wide string and its byte size.
        let status = unsafe {
            match exe {
                Some(exe) => {
                    let data = wstr(&format!("\"{}\"", exe.display()));
                    RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
                }
                None => match RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) {
                    2 => ERROR_SUCCESS, // ERROR_FILE_NOT_FOUND: already off
                    other => other,
                },
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(status as i32))
        }
    }
}
