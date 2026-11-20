//! The Windows side of the tray app: the hidden top-level window that receives the tray
//! icon's callbacks and system broadcasts, the notification icon itself, balloons, the
//! context menu, and the system appearance.
//!
//! The window is a normal (hidden) top-level window, not a message-only one: message-only
//! windows receive no broadcasts, and the app needs `TaskbarCreated`, `WM_POWERBROADCAST`,
//! `WM_TIMECHANGE` and `WM_SETTINGCHANGE`.

use std::cell::RefCell;

use squ4t::{fit_utf16, wstr, MSG_OPEN, MSG_QUIT, MSG_REFRESH, MSG_TEST_ALERT, WINDOW_CLASS};
use squ4t_core::look::{Rgba, SystemColors};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetMonitorInfoW, GetSysColor, MonitorFromPoint, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
    COLOR_WINDOW, COLOR_WINDOWTEXT, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_NONE, NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NIN_BALLOONUSERCLICK, NIN_SELECT, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, NOTIFYICON_VERSION_4,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, SystemParametersInfoW, TrackPopupMenu, HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR, MF_STRING, SM_CXSMICON,
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CONTEXTMENU, WM_DWMCOLORIZATIONCOLORCHANGED, WM_NULL,
    WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE, WM_TIMECHANGE, WNDCLASSW, WS_OVERLAPPED,
};

const CALLBACK: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;
const ICON_ID: u32 = 1;
/// NIN_SELECT | NINF_KEY: Enter or Space on the focused icon.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;

/// What the hidden window heard, handed to the app on its event loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Left click, or Enter/Space on the focused icon.
    Select,
    BalloonClick,
    /// The taskbar was recreated (Explorer restarted, or a DPI change): add the icon again.
    TaskbarCreated,
    Resumed,
    TimeChanged,
    AppearanceChanged,
    Menu(MenuCommand),
    Open,
    Refresh,
    TestAlert,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    Open = 1,
    PauseResume = 2,
    Snooze = 3,
    Startup = 4,
    Exit = 5,
}

/// The labels and states the context menu shows, kept current by the app.
#[derive(Clone, Default)]
pub struct MenuState {
    pub open: String,
    pub pause_resume: String,
    pub snooze: String,
    pub startup: String,
    pub startup_on: bool,
    pub exit: String,
}

type Sink = Box<dyn Fn(Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    static MENU: RefCell<MenuState> = RefCell::new(MenuState::default());
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub fn set_menu(state: MenuState) {
    MENU.with(|m| *m.borrow_mut() = state);
}

fn emit(event: Event) {
    // Deferred to the event loop: never re-enter Slint from inside a window procedure.
    slint::invoke_from_event_loop(move || {
        SINK.with(|s| {
            if let Some(f) = s.borrow().as_ref() {
                f(event)
            }
        })
    })
    .ok();
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        CALLBACK => {
            match (lparam & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => emit(Event::Select),
                NIN_BALLOONUSERCLICK => emit(Event::BalloonClick),
                WM_CONTEXTMENU => {
                    if let Some(command) = show_menu(hwnd) {
                        emit(Event::Menu(command));
                    }
                }
                _ => {}
            }
            0
        }
        WM_POWERBROADCAST => {
            if wparam == PBT_APMRESUMEAUTOMATIC {
                emit(Event::Resumed);
            }
            1
        }
        WM_TIMECHANGE => {
            emit(Event::TimeChanged);
            0
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            emit(Event::AppearanceChanged);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        MSG_OPEN => {
            emit(Event::Open);
            0
        }
        MSG_REFRESH => {
            emit(Event::Refresh);
            0
        }
        MSG_TEST_ALERT => {
            emit(Event::TestAlert);
            0
        }
        MSG_QUIT => {
            emit(Event::Quit);
            0
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.with(|t| t.get()) => {
            emit(Event::TaskbarCreated);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_menu(hwnd: HWND) -> Option<MenuCommand> {
    let state = MENU.with(|m| m.borrow().clone());
    let menu = CreatePopupMenu();
    let item = |id: MenuCommand, text: &str, flags: u32| {
        let t = wstr(text);
        AppendMenuW(menu, MF_STRING | flags, id as usize, t.as_ptr());
    };
    item(MenuCommand::Open, &state.open, 0);
    item(MenuCommand::PauseResume, &state.pause_resume, 0);
    item(MenuCommand::Snooze, &state.snooze, 0);
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    item(MenuCommand::Startup, &state.startup, if state.startup_on { MF_CHECKED } else { 0 });
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    item(MenuCommand::Exit, &state.exit, 0);
    // Right-click pauses or resumes in the plugin: that stays the default (bold) item.
    SetMenuDefaultItem(menu, MenuCommand::PauseResume as u32, 0);
    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    // The documented dance for tray menus: foreground first, WM_NULL after.
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, cursor.x, cursor.y, 0, hwnd, std::ptr::null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    [MenuCommand::Open, MenuCommand::PauseResume, MenuCommand::Snooze, MenuCommand::Startup, MenuCommand::Exit].into_iter().find(|c| *c as i32 == chosen)
}

/// The notification icon and its window.
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    tip: String,
}

impl Tray {
    /// Creates the hidden window on this (the event loop's) thread, named for one data folder.
    pub fn create(title: &str, sink: impl Fn(Event) + 'static) -> Tray {
        SINK.with(|s| *s.borrow_mut() = Some(Box::new(sink)));
        let class = wstr(WINDOW_CLASS);
        let title = wstr(title);
        // SAFETY: registering and creating a window of this module with a static procedure.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(wstr("TaskbarCreated").as_ptr())));
            CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null())
        };
        Tray { hwnd, icon: std::ptr::null_mut(), tip: String::new() }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        // SAFETY: plain data.
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = self.hwnd;
        d.uID = ICON_ID;
        d
    }

    fn fill_tip(&self, d: &mut NOTIFYICONDATAW) {
        let tip = fit_utf16(&self.tip, d.szTip.len() - 1);
        d.szTip[..tip.len()].copy_from_slice(&tip);
    }

    /// Adds the icon (again, after `TaskbarCreated`).
    pub fn add(&mut self) {
        let mut d = self.data();
        d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        d.uCallbackMessage = CALLBACK;
        d.hIcon = self.icon;
        self.fill_tip(&mut d);
        // SAFETY: a filled NOTIFYICONDATAW for this window.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &d);
            Shell_NotifyIconW(NIM_ADD, &d);
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &d);
        }
    }

    /// Replaces the icon picture and tooltip.
    pub fn update(&mut self, pixels: &[u8], size: u32, tip: &str) {
        let icon = make_icon(pixels, size);
        self.tip = tip.to_string();
        let mut d = self.data();
        d.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        d.hIcon = icon;
        self.fill_tip(&mut d);
        // SAFETY: as above; the previous icon is no longer shown once the shell has the new one.
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &d);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
        self.icon = icon;
    }

    /// A balloon, which Windows 10 and 11 show as a notification banner headed by the program's
    /// FileDescription and icon: squ4t and the ember ring. No second icon in the body, which
    /// already has the ring above it. (NIIF_USER with the ring as the balloon icon made Windows
    /// drop the notification.) Quiet time is respected; Do not disturb is applied by Windows.
    pub fn notify(&self, title: &str, body: &str) {
        let mut d = self.data();
        d.uFlags = NIF_INFO | NIF_SHOWTIP | NIF_TIP;
        self.fill_tip(&mut d);
        let t = fit_utf16(title, d.szInfoTitle.len() - 1);
        d.szInfoTitle[..t.len()].copy_from_slice(&t);
        let b = fit_utf16(body, d.szInfo.len() - 1);
        d.szInfo[..b.len()].copy_from_slice(&b);
        d.dwInfoFlags = NIIF_NONE | NIIF_RESPECT_QUIET_TIME;
        // SAFETY: as above.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }

    /// The icon's rectangle on screen, in physical pixels.
    pub fn icon_rect(&self) -> Option<RECT> {
        let id = NOTIFYICONIDENTIFIER { cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: self.hwnd, uID: ICON_ID, ..unsafe { std::mem::zeroed() } };
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: a valid identifier and out-parameter.
        (unsafe { Shell_NotifyIconGetRect(&id, &mut rect) } >= 0).then_some(rect)
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let d = self.data();
        // SAFETY: removing our own icon.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &d);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

fn make_icon(bgra: &[u8], size: u32) -> HICON {
    // SAFETY: a 32-bit top-down DIB of the given size, filled from `bgra`, and a 1-bit mask.
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len().min((size * size * 4) as usize));
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, std::ptr::null());
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// The small-icon size for the tray at the current DPI.
pub fn icon_size() -> u32 {
    // SAFETY: no arguments.
    (unsafe { GetSystemMetrics(SM_CXSMICON) }).max(16) as u32
}

// ---------------------------------------------------------------- appearance

fn dword(key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wstr(key), wstr(value));
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: a DWORD out-parameter of the size passed.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut size) };
    (status == 0).then_some(data)
}

const PERSONALIZE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub light_apps: bool,
    pub light_taskbar: bool,
    pub high_contrast: bool,
    pub animations: bool,
    pub system: SystemColors,
}

/// What Windows reports now. `SQU4T_THEME=light|dark` overrides the app theme.
pub fn appearance() -> Appearance {
    let theme = std::env::var("SQU4T_THEME").ok();
    let light_apps = match theme.as_deref() {
        Some("light") => true,
        Some("dark") => false,
        _ => dword(PERSONALIZE, "AppsUseLightTheme") == Some(1),
    };
    let light_taskbar = dword(PERSONALIZE, "SystemUsesLightTheme") == Some(1);
    let mut hc: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
    let mut animations = 1i32;
    // SAFETY: documented out-parameters.
    unsafe {
        SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&mut hc as *mut HIGHCONTRASTW).cast(), 0);
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut animations as *mut i32).cast(), 0);
    }
    let high_contrast = hc.dwFlags & HCF_HIGHCONTRASTON != 0;
    let sys = |index| {
        // SAFETY: a valid system colour index.
        let c = unsafe { GetSysColor(index) };
        Rgba::rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)
    };
    let reduced = std::env::var("SQU4T_REDUCED_MOTION").is_ok_and(|v| v == "1");
    Appearance {
        light_apps,
        light_taskbar,
        high_contrast,
        // Motion.Enabled: client-area animation on, and not high contrast.
        animations: animations != 0 && !high_contrast && !reduced,
        system: SystemColors { window: sys(COLOR_WINDOW), window_text: sys(COLOR_WINDOWTEXT), highlight: sys(COLOR_HIGHLIGHT), highlight_text: sys(COLOR_HIGHLIGHTTEXT) },
    }
}

/// Where the panel goes: next to the tray icon, inside the work area of its monitor.
/// Returns the top-left corner in physical pixels and the monitor's scale factor.
pub fn panel_position(anchor: Option<RECT>, width: f64, height: f64) -> (i32, i32, f64) {
    let point = match anchor {
        Some(r) => POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 },
        None => {
            let mut p = POINT { x: 0, y: 0 };
            // SAFETY: out-parameter.
            unsafe { GetCursorPos(&mut p) };
            p
        }
    };
    // SAFETY: plain queries with out-parameters.
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f64 / 96.0;
        let (w, h) = ((width * scale).round() as i32, (height * scale).round() as i32);
        let gap = (12.0 * scale).round() as i32;
        let work = info.rcWork;
        let mut x = point.x - w / 2;
        let mut y = if point.y >= work.bottom {
            work.bottom - h - gap
        } else if point.y < work.top {
            work.top + gap
        } else {
            point.y - h - gap
        };
        if point.x >= work.right {
            x = work.right - w - gap;
        } else if point.x < work.left {
            x = work.left + gap;
        }
        x = x.clamp(work.left + gap, (work.right - w - gap).max(work.left + gap));
        y = y.clamp(work.top + gap, (work.bottom - h - gap).max(work.top + gap));
        (x, y, scale)
    }
}

pub fn foreground(hwnd: HWND) {
    // SAFETY: plain call.
    unsafe { SetForegroundWindow(hwnd) };
}

/// Moves a window without resizing, activating or reordering it.
pub fn move_window(hwnd: HWND, x: i32, y: i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};
    // SAFETY: plain call on a window of this process.
    unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// The whole window's opacity, 0..1. A layered window only while it is see-through, so that
/// at rest it is an ordinary window that Windows 11 rounds. True when the window switched in or
/// out of layered mode: Windows then discards its pixels and it must be painted again in full.
pub fn window_alpha(hwnd: HWND, opacity: f64) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED};
    // SAFETY: plain calls on a window of this process.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let layered = style & WS_EX_LAYERED as isize != 0;
        if opacity >= 0.999 {
            if layered {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
            }
            return layered;
        }
        if !layered {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        }
        SetLayeredWindowAttributes(hwnd, 0, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8, LWA_ALPHA);
        !layered
    }
}
